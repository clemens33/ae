"""Phase-3 Slice 16 contract: daemon long loop + crash backoff.

The supervisor that runs forever: tick-driven with an INJECTABLE clock/sleep (no real
time), per-component CRASH CONTAINMENT via per-component BackoffState, a heartbeat EVERY
iteration, signal-aware CLEAN shutdown (SIGTERM/SIGHUP/SIGINT), and redacted crash
tracebacks. It holds the singleton for its whole lifetime.

The loop is generic over COMPONENTS = [(name, tick(now)), ...]; the daemon wires
("watchdog", _run_daemon_tick_body) + ("bridge", TelegramBridge.tick). Per component:
its own backoff-<name>.json; a crash records + logs a redacted traceback and, over
budget, ALERTS and stops the whole daemon; a clean tick resets only THAT component's
streak so one component's health can't mask another's crash cadence.

Pure stdlib; no real time/signals in the deterministic tests.
"""

import os
import signal
import tempfile
import time
import unittest
from pathlib import Path

from harness import AW


class Clock:
    def __init__(self):
        self.t = 1000

    def __call__(self):
        self.t += 1
        return self.t


class StopAfter:
    """A fake sleep: records the interval, returns True (stop the loop) after n calls."""

    def __init__(self, n):
        self.n = n
        self.intervals = []

    def __call__(self, interval):
        self.intervals.append(interval)
        return len(self.intervals) >= self.n


def _runtime(root):
    return AW.AewatchRuntime(root)


class DaemonLoopTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.rt = _runtime(self._tmp.name)
        self.rec = AW.EffectRecorder()

    def _loop(self, components, *, n=3, sleep=None, secrets=(), install_signals=False):
        return AW.run_daemon_loop(
            self.rt, components, clock=Clock(), interval=5,
            sleep=sleep or StopAfter(n), recorder=self.rec,
            secrets_provider=lambda: list(secrets), install_signals=install_signals,
        )

    def test_runs_each_component_each_iteration(self):
        calls = {"a": [], "b": []}
        self._loop([("a", lambda now: calls["a"].append(now)),
                    ("b", lambda now: calls["b"].append(now))], n=3)
        self.assertEqual(len(calls["a"]), 3)
        self.assertEqual(calls["a"], calls["b"], "both components tick each iteration with the same now")

    def test_component_crash_is_contained(self):
        other = []
        self._loop([("bad", self._raise("boom")), ("good", lambda now: other.append(now))], n=3)
        self.assertEqual(len(other), 3, "a crashing component does not stop the others' ticks")

    def test_crash_over_budget_stops_the_daemon(self):
        # a component that always crashes trips its budget and stops the loop early.
        result = self._loop([("bad", self._raise("always"))], n=100)  # would run 100x if not stopped
        crashes = AW.BackoffState(self.rt.dir / "backoff-bad.json").count(time.time() + 1)
        self.assertLessEqual(crashes, AW._BACKOFF_BUDGET + 1)
        self.assertTrue(any("over crash budget" in e.get("message", "") for e in self.rec.as_list()),
                        "an over-budget alert is logged")
        self.assertIn("bad", str(result.get("reason", "")))

    def test_per_component_backoff_isolation(self):
        # bridge crashes every iteration, watchdog is clean: the watchdog's success must
        # NOT reset the bridge streak, and the watchdog backoff stays empty.
        wd = []
        self._loop([("watchdog", lambda now: wd.append(now)), ("bridge", self._raise("br"))], n=100)
        self.assertEqual(AW.BackoffState(self.rt.dir / "backoff-watchdog.json").count(time.time() + 1), 0,
                         "a clean watchdog accrues no crashes")
        self.assertTrue(wd, "the healthy watchdog kept ticking until the bridge tripped")
        # per-component: the watchdog's clean ticks must NOT reset the bridge streak, so
        # the bridge trips and stops the daemon early (a shared streak would never trip).
        self.assertLess(len(wd), AW._BACKOFF_BUDGET + 3,
                        "the bridge crash-loop tripped its own budget and stopped the daemon")

    def test_clean_tick_resets_that_components_streak(self):
        # crash once, then succeed forever -> the single crash never accumulates to budget.
        state = {"i": 0}

        def flaky(now):
            state["i"] += 1
            if state["i"] == 1:
                raise RuntimeError("first only")

        self._loop([("c", flaky)], n=10)
        self.assertEqual(AW.BackoffState(self.rt.dir / "backoff-c.json").count(time.time() + 1), 0,
                         "a clean tick after a crash resets the streak")

    def test_heartbeat_every_iteration_including_crash(self):
        beats = []
        real = self.rt.write_heartbeat
        self.rt.write_heartbeat = lambda: beats.append(1) or real()
        self._loop([("bad", self._raise("x"))], n=3)
        self.assertEqual(len(beats), 3, "heartbeat stamps every iteration even when a tick crashes")

    def test_crash_traceback_is_redacted(self):
        token = "123456:super-secret-token-value"
        self._loop([("bad", self._raise(f"leaking {token}"))], n=2, secrets=[token])
        for e in self.rec.as_list():
            self.assertNotIn(token, e.get("message", ""), "a token in a crash traceback must be redacted")

    def test_backoff_persist_failure_on_crash_is_contained(self):
        # A broken backoff file (here: a DIRECTORY) makes record_crash's _save raise
        # OSError. That must NOT escape the loop — heartbeat still runs, a WARNING logs,
        # and the daemon does not crash (codex).
        (self.rt.dir / "backoff-bad.json").mkdir(parents=True)
        beats = []
        real = self.rt.write_heartbeat
        self.rt.write_heartbeat = lambda: beats.append(1) or real()
        self._loop([("bad", self._raise("x"))], n=3)
        self.assertEqual(len(beats), 3, "heartbeat continues despite the backoff-write failure")
        self.assertTrue(any("backoff persist failed" in e.get("message", "") for e in self.rec.as_list()))

    def test_backoff_persist_failure_on_clean_is_contained(self):
        (self.rt.dir / "backoff-clean.json").mkdir(parents=True)
        self._loop([("clean", lambda now: None)], n=2)
        self.assertTrue(any("backoff reset failed" in e.get("message", "") for e in self.rec.as_list()))

    def test_duplicate_component_names_rejected(self):
        with self.assertRaises(ValueError):
            AW.run_daemon_loop(self.rt, [("dup", lambda now: None), ("dup", lambda now: None)],
                               clock=Clock(), interval=5, sleep=StopAfter(1), recorder=self.rec,
                               secrets_provider=lambda: [], install_signals=False)

    def test_singleton_already_held_does_not_start(self):
        lock = self.rt.singleton()
        self.assertTrue(lock.acquire())
        try:
            result = self._loop([("a", lambda now: None)], n=3)
            self.assertFalse(result.get("started"), "a second daemon must not start")
        finally:
            lock.release()

    def test_interval_zero_yields_once_not_tight_loop(self):
        sleep = StopAfter(1)
        AW.run_daemon_loop(self.rt, [("a", lambda now: None)], clock=Clock(), interval=0,
                           sleep=sleep, recorder=self.rec, secrets_provider=lambda: [],
                           install_signals=False)
        self.assertEqual(sleep.intervals, [0], "interval=0 still goes through the injected sleep once")

    # ── signals ─────────────────────────────────────────────────────────
    def test_signal_triggers_clean_shutdown_no_crash(self):
        for signame in ("SIGTERM", "SIGHUP", "SIGINT"):
            with self.subTest(signal=signame):
                rt = _runtime(tempfile.mkdtemp())
                rec = AW.EffectRecorder()
                fired = {"done": False}

                def sleep(interval):
                    # deliver the signal mid-loop, then let the loop notice shutdown.
                    if not fired["done"]:
                        fired["done"] = True
                        os.kill(os.getpid(), getattr(signal, signame))
                    return False  # never stop via sleep — the signal must

                result = AW.run_daemon_loop(rt, [("a", lambda now: None)], clock=Clock(),
                                            interval=5, sleep=sleep, recorder=rec,
                                            secrets_provider=lambda: [], install_signals=True)
                self.assertEqual(AW.BackoffState(rt.dir / "backoff-a.json").count(time.time() + 1), 0,
                                 f"{signame} is a clean shutdown, NOT a crash")
                self.assertTrue(rt.singleton().acquire(), "the singleton is released on shutdown")
                rt.singleton().release()

    def _raise(self, msg):
        def tick(now):
            raise RuntimeError(msg)
        return tick


if __name__ == "__main__":
    unittest.main()
