"""s19 aewatch-side: the Telegram bridge OWNERSHIP marker + the no-double-send handoff.

Per the lead's ruling, ownership is a durable $AE_HOME/aewatch/bridge-owner marker (the bash
guard stands down while it exists AND the heartbeat is fresh). The aewatch daemon's handoff
runs the STRICT order — (a) write the marker (suppresses every bash revive path), (b) stop the
live bash ae-telegram bridge, (c) only THEN send — so no fixture ever has two bridges sending.
Clean shutdown removes the marker so bash resumes instantly (a crash leaves it, but the
heartbeat stales -> bash revives = the free fallback).

Pure stdlib.
"""

import os
import tempfile
import time
import unittest
import unittest.mock as mock
from pathlib import Path

from harness import AW, FakeTmux


class _RecBridge:
    """A stand-in TelegramBridge whose tick just records a 'send' into a shared order log."""

    def __init__(self, order):
        self._order = order

    def tick(self, now=None):
        self._order.append("send")


def _write_enabled_config(home):
    tok = home / "tok"
    tok.write_text("111:secret-token")
    tok.chmod(0o600)
    (home / "config").write_text(
        "[telegram]\nenabled = true\ntoken_file = " + str(tok) + "\nchat_id = 42\nallowed_user_ids = 7\n")


class BridgeOwnerMarkerTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)

    def test_marker_write_and_clear(self):
        rt = AW.AewatchRuntime(self.home)
        self.assertFalse(rt.bridge_owner_path.exists())
        rt.write_bridge_owner()
        self.assertTrue(rt.bridge_owner_path.is_file(), "write_bridge_owner must create the marker")
        self.assertRegex(rt.bridge_owner_path.read_text(), r"^\d+ \d+")  # pid + ns stamp
        rt.clear_bridge_owner()
        self.assertFalse(rt.bridge_owner_path.exists(), "clear_bridge_owner must remove the marker")
        rt.clear_bridge_owner()  # idempotent — no error when already gone


class HandoffOrderTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)
        self.rt = AW.AewatchRuntime(self.home)

    def _instrumented(self):
        """A FakeTmux + runtime whose marker-write and kill are logged into one order list, so
        the strict handoff order (marker -> kill -> send) is directly observable."""
        order = []
        tmux = FakeTmux(AW.EffectRecorder())
        real_write = self.rt.write_bridge_owner
        real_kill = tmux.kill_session

        def rec_write():
            order.append("marker")
            return real_write()

        def rec_kill(session, server=None):
            order.append("kill:" + session)
            real_kill(session, server)

        self.rt.write_bridge_owner = rec_write
        tmux.kill_session = rec_kill
        return order, tmux

    def test_handoff_order_is_marker_then_stop_bash_then_send(self):
        _write_enabled_config(self.home)
        order, tmux = self._instrumented()
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)
        self.assertEqual(order, ["marker", "kill:ae-telegram", "send"],
                         "handoff must be marker -> stop bash -> first send (never a double-send window)")
        self.assertTrue(self.rt.bridge_owner_path.is_file(), "the marker persists after the handoff")

    def test_handoff_runs_once_then_ticks_plainly(self):
        _write_enabled_config(self.home)
        order, tmux = self._instrumented()
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)
        tick(2000)
        tick(3000)
        # marker + kill exactly once; the bash bridge is not killed on every tick.
        self.assertEqual(order, ["marker", "kill:ae-telegram", "send", "send", "send"])
        self.assertEqual(tmux.killed_sessions, ["ae-telegram"])

    def test_disabled_telegram_no_marker_no_kill(self):
        (self.home / "config").write_text("[telegram]\nenabled = false\n")
        order, tmux = self._instrumented()
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)
        self.assertEqual(tmux.killed_sessions, [], "disabled telegram must not stop the bash bridge")
        self.assertFalse(self.rt.bridge_owner_path.exists(), "disabled telegram must not claim ownership")

    def test_bridge_component_exposes_bridge(self):
        _write_enabled_config(self.home)
        bridge = _RecBridge([])
        tick = AW._make_bridge_component(bridge, self.rt, FakeTmux(AW.EffectRecorder()), lambda: [])
        self.assertIs(tick.bridge, bridge, "the component must expose .bridge for wiring/continuity tests")


    def test_full_ownership_fact_holds_before_first_send(self):
        # B4: on a FRESH daemon (no prior heartbeat), marker + FRESH heartbeat must both be
        # true BEFORE the first send, else a bash reviver sees only the marker and revives.
        _write_enabled_config(self.home)
        self.assertFalse(self.rt.heartbeat_path.exists())  # fresh start, no heartbeat yet
        rt = self.rt
        seen = {}

        class _ProbeBridge:
            def tick(self, now=None):
                seen["marker"] = rt.bridge_owner_path.exists()
                seen["fresh_hb"] = AW._bridge_ownership_fresh(rt)

        tick = AW._make_bridge_component(_ProbeBridge(), rt, FakeTmux(AW.EffectRecorder()), lambda: [])
        tick(1000)
        self.assertTrue(seen.get("marker"), "marker must exist at the first send")
        self.assertTrue(seen.get("fresh_hb"),
                        "marker + FRESH heartbeat (the full ownership fact) must hold BEFORE the first send")

    def test_handoff_fail_closed_when_heartbeat_not_fresh(self):
        # B4: if the heartbeat can't be made fresh, back out — clear our marker, no kill/send.
        _write_enabled_config(self.home)
        order = []
        tmux = FakeTmux(AW.EffectRecorder())
        self.rt.write_heartbeat = lambda: None  # heartbeat write does not land
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)
        self.assertEqual(tmux.killed_sessions, [], "no bash kill when the heartbeat isn't fresh")
        self.assertEqual(order, [], "no aewatch send when the heartbeat isn't fresh")
        self.assertFalse(self.rt.bridge_owner_path.exists(), "incomplete ownership -> our marker is cleared")

    def test_no_send_when_ownership_decays_within_iteration(self):
        # B7 (codex): after handoff, if time passes inside ONE iteration (a slow watchdog component)
        # and the ownership fact decays before the bridge tick, the bridge must NOT send — bash may
        # have revived. Per-send VERIFY, no refresh (refreshing would race the revived bash).
        _write_enabled_config(self.home)
        order = []
        tmux = FakeTmux(AW.EffectRecorder())
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)  # handoff: marker + fresh heartbeat + kill + first send
        self.assertEqual(order, ["send"])
        old = time.time() - 120  # a slow watchdog aged the heartbeat past the 90s decay
        os.utime(self.rt.heartbeat_path, (old, old))
        tick(2000)  # ownership decayed within the iteration -> must NOT send
        self.assertEqual(order, ["send"],
                         "the bridge must not send once the ownership fact decayed within the iteration")

    def test_stale_skip_reruns_handoff_and_rekills_bash(self):
        # B8: after a per-send stale skip the next fresh tick must re-run the FULL handoff (re-kill
        # any bash that revived during the stale window) — not just resume sending on a refreshed fact.
        _write_enabled_config(self.home)
        order = []
        tmux = FakeTmux(AW.EffectRecorder())
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)  # handoff #1: kill + send
        self.assertEqual(tmux.killed_sessions, ["ae-telegram"])
        old = time.time() - 120  # ownership decays within the iteration
        os.utime(self.rt.heartbeat_path, (old, old))
        tick(2000)  # stale -> skip + invalidate handoff (no send, marker cleared)
        self.assertEqual(order, ["send"])
        self.assertFalse(self.rt.bridge_owner_path.exists(), "stale skip clears our marker")
        tick(3000)  # next enabled tick re-runs the FULL handoff (fresh heartbeat) -> SECOND kill + send
        self.assertEqual(tmux.killed_sessions, ["ae-telegram", "ae-telegram"],
                         "the next send after a stale skip must be preceded by a fresh bash re-kill")
        self.assertEqual(order, ["send", "send"])


class CleanShutdownTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)
        self.rt = AW.AewatchRuntime(self.home)

    def test_clean_shutdown_removes_marker(self):
        marker = self.home / "aewatch" / "bridge-owner"
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text(str(os.getpid()) + " 456\n")  # OUR pid -> clean shutdown clears it
        # _run_daemon_loop_cli wraps run_daemon_loop in try/finally -> the marker is cleared on
        # a clean return so bash resumes instantly (not after heartbeat decay).
        with mock.patch.object(AW, "run_daemon_loop", lambda *a, **k: {"started": True}):
            AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        self.assertFalse(marker.exists(), "clean daemon shutdown must remove the bridge-owner marker")

    def test_handoff_fail_closed_when_marker_write_fails(self):
        # B1: if ownership can't be durably claimed, do NOT stop bash and do NOT send —
        # bash keeps the bridge (a missing marker means revive paths see no owner).
        _write_enabled_config(self.home)
        order = []
        tmux = FakeTmux(AW.EffectRecorder())
        self.rt.write_bridge_owner = lambda: False  # simulate a durable-write failure
        tick = AW._make_bridge_component(_RecBridge(order), self.rt, tmux, lambda: [])
        tick(1000)
        self.assertEqual(tmux.killed_sessions, [], "fail-closed: no bash kill when the marker claim fails")
        self.assertEqual(order, [], "fail-closed: no aewatch send when the marker claim fails")

    def test_handoff_kills_bash_on_every_discovered_server(self):
        # B3: a bash ae-telegram revived inside a named (-L) server pane survives an
        # ambient-only kill. The handoff must kill it on ambient AND every discovered server.
        _write_enabled_config(self.home)
        tmux = FakeTmux(AW.EffectRecorder())

        def discover():
            return [
                AW.DiscoveredSession(name="s1", session_id="", work_dir="", tmux_server="srvA", running=True, agents=[]),
                AW.DiscoveredSession(name="s2", session_id="", work_dir="", tmux_server="", running=True, agents=[]),
                AW.DiscoveredSession(name="s3", session_id="", work_dir="", tmux_server="srvA", running=True, agents=[]),
            ]

        tick = AW._make_bridge_component(_RecBridge([]), self.rt, tmux, discover)
        tick(1000)
        self.assertEqual(tmux.killed_sessions.count("ae-telegram"), 2, "kill on ambient + one unique named server")
        servers = {srv for method, srv in tmux.server_calls if method == "kill_session"}
        self.assertEqual(servers, {"", "srvA"}, "handoff must kill ae-telegram on ambient AND every discovered server")


class OwnerAwareClearTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)

    def test_clear_only_removes_our_own_marker(self):
        # B2: pid-aware clear — never remove a marker written by another (the real owner).
        rt = AW.AewatchRuntime(self.home)
        rt.bridge_owner_path.write_text("99999 123\n")  # another process's live marker
        rt.clear_bridge_owner()
        self.assertTrue(rt.bridge_owner_path.exists(), "must NOT clear a marker owned by another pid")
        rt.bridge_owner_path.write_text(str(os.getpid()) + " 123\n")  # our marker
        rt.clear_bridge_owner()
        self.assertFalse(rt.bridge_owner_path.exists(), "must clear our OWN marker")

    def test_second_daemon_invocation_keeps_active_marker(self):
        # B2: a second `daemon --loop` that loses the singleton race (started=False) must NOT
        # clear the live owner's marker (its heartbeat is fresh -> would let bash double-send).
        marker = self.home / "aewatch" / "bridge-owner"
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("99999 123\n")  # the REAL owner's marker (a different pid)
        with mock.patch.object(AW, "run_daemon_loop", lambda *a, **k: {"started": False, "reason": "already running"}):
            AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        self.assertTrue(marker.exists(), "a losing second invocation must not clear the active owner's marker")


class HeartbeatDecayStopTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)

    def test_persistent_heartbeat_failure_stops_before_ownership_decays(self):
        # B5: the ownership fact (marker + FRESH heartbeat) must not outlive our ability to
        # refresh it. If write_heartbeat fails for >= the decay-stop budget while the loop keeps
        # ticking (and the bridge keeps sending), the daemon must STOP so the CLI finally clears
        # the marker -> bash revives -> single sender.
        rt = AW.AewatchRuntime(self.home)
        rt.write_heartbeat = mock.Mock(side_effect=OSError("disk full"))
        clock_vals = iter([0, 30, 60, 90, 120])  # 30s per iteration; init consumes 0
        result = AW.run_daemon_loop(
            rt, [("noop", lambda now: None)],
            clock=lambda: next(clock_vals), interval=0,
            sleep=lambda _s: False,  # the interruptible wait never signals; only the B5 stop does
            install_signals=False,
        )
        self.assertEqual(result["reason"], "heartbeat-write-failed",
                         "persistent heartbeat-write failure must stop the daemon before the ownership fact decays")

    def test_single_heartbeat_blip_does_not_stop(self):
        # A single transient blip with a still-fresh heartbeat stays WARN-only (no shutdown).
        rt = AW.AewatchRuntime(self.home)
        n = {"i": 0}

        def hb():
            n["i"] += 1
            if n["i"] == 1:
                raise OSError("blip")  # fail once, then succeed

        rt.write_heartbeat = hb
        clock_vals = iter([0, 5, 10, 15, 20])
        stops = {"i": 0}

        def sleep(_s):
            stops["i"] += 1
            return stops["i"] >= 2  # a normal clean shutdown after two iterations

        result = AW.run_daemon_loop(
            rt, [("noop", lambda now: None)],
            clock=lambda: next(clock_vals), interval=0, sleep=sleep, install_signals=False,
        )
        self.assertNotEqual(result.get("reason"), "heartbeat-write-failed",
                            "a single transient heartbeat blip must not stop the daemon")

    def test_no_component_send_before_heartbeat_stop(self):
        # B6: the heartbeat is maintained/checked BEFORE components tick, so on a stale/
        # unmaintainable fact (long interval / >90s stall / persistent write failure) the daemon
        # STOPS before any component sends — the bridge never sends with a stale ownership fact.
        rt = AW.AewatchRuntime(self.home)
        rt.write_heartbeat = mock.Mock(side_effect=OSError("disk full"))
        sends = []
        clock_vals = iter([0, 120, 240])  # a >90s jump -> heartbeat unmaintainable on iter 1
        result = AW.run_daemon_loop(
            rt, [("bridge", lambda now: sends.append("send"))],
            clock=lambda: next(clock_vals), interval=0, sleep=lambda _s: False, install_signals=False,
        )
        self.assertEqual(result["reason"], "heartbeat-write-failed")
        self.assertEqual(sends, [], "no component may send once the heartbeat is unmaintainable (stale ownership)")


class DueGatedCadenceTest(unittest.TestCase):
    def setUp(self):
        self._tmp2 = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp2.cleanup)

    def test_watchdog_self_gates_on_configured_interval(self):
        # lead B7: the daemon loop wakes every <=60s (B6 heartbeat maintenance), but the WATCHDOG
        # must sweep ONLY at its configured cadence (e.g. 180) so cycle-counted logic (throttle /
        # nudge budgets, alert text) never skews 3x. The bridge, by contrast, ticks every wake.
        runs = []
        gated = AW._due_gated(lambda now: runs.append(now), 180)
        for now in (0, 60, 120, 180, 240, 300, 360):  # loop wakes every 60s
            gated(now)
        self.assertEqual(runs, [0, 180, 360],
                         "the watchdog runs only at its 180s cadence boundaries, not every wake")

    def test_first_tick_always_runs(self):
        runs = []
        AW._due_gated(lambda now: runs.append(now), 999)(0)
        self.assertEqual(runs, [0])

    def test_not_due_tick_returns_skip_sentinel(self):
        # B9: a not-due wake returns the skip sentinel so run_daemon_loop won't count it as a
        # clean tick (which would reset the crash streak).
        gated = AW._due_gated(lambda now: "ran", 180)
        self.assertEqual(gated(0), "ran")
        self.assertIs(gated(60), AW._TICK_SKIPPED)
        self.assertIs(gated(120), AW._TICK_SKIPPED)
        self.assertEqual(gated(180), "ran")

    def test_skipped_wakes_do_not_reset_crash_backoff(self):
        # B9: a due-gated watchdog that crashes on EVERY due run must still trip its crash budget —
        # the not-due skipped wakes between due runs must NOT reset the streak.
        rt = AW.AewatchRuntime(self._tmp2.name)

        def crasher(now):
            raise RuntimeError("watchdog boom")

        gated = AW._due_gated(crasher, 180)
        clock_vals = iter(range(0, 2000, 60))  # wakes every 60s; due (crash) at 60,240,420,600,780,960
        result = AW.run_daemon_loop(
            rt, [("watchdog", gated)],
            clock=lambda: next(clock_vals), interval=0, sleep=lambda _s: False, install_signals=False,
        )
        self.assertEqual(result["reason"], "watchdog-over-budget",
                         "skipped gated wakes must not reset the crash streak (backoff must still trip)")


if __name__ == "__main__":
    unittest.main()
