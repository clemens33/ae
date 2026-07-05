"""Slice 8 contract: crash-loop backoff state ($AE_HOME/aewatch/backoff.json).

The supervisor (later) restarts a dead daemon, but must give up after too many
crashes within a window rather than hot-loop forever. This slice owns the STATE:
crash counting within a rolling window, reset on a healthy tick, an over-budget
"should stop" signal with one alert effect, atomic JSON writes, and graceful
recovery from a corrupt file. No supervisor loop or tmux session yet.

Time is injected (no real clock); pure stdlib; per-AE_HOME isolated temp roots.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from harness import AW

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"


class BackoffTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.rt = AW.AewatchRuntime(self.root)
        self.rec = AW.EffectRecorder()

    def make(self, *, window_secs=3600, budget=2):
        return AW.BackoffState(self.rt.backoff_path, window_secs=window_secs, budget=budget, recorder=self.rec)

    def test_backoff_path_under_ae_home(self):
        self.assertEqual(self.rt.backoff_path, self.rt.dir / "backoff.json")

    def test_crash_count_increments_within_window(self):
        b = self.make()
        b.record_crash(1000)
        b.record_crash(1010)
        self.assertEqual(b.count(1020), 2)

    def test_old_crashes_pruned_outside_window(self):
        b = self.make(window_secs=100)
        b.record_crash(1000)
        b.record_crash(1250)  # 250s later, first is now outside the 100s window
        self.assertEqual(b.count(1250), 1)

    def test_success_resets_count(self):
        b = self.make()
        b.record_crash(1000)
        b.record_crash(1010)
        b.record_success(1020)
        self.assertEqual(b.count(1030), 0)

    def test_over_budget_returns_should_stop_and_emits_one_alert(self):
        b = self.make(budget=2)
        self.assertFalse(b.record_crash(1000))  # 1 <= 2
        self.assertFalse(b.record_crash(1010))  # 2 <= 2
        self.assertTrue(b.record_crash(1020))   # 3 > 2 -> should stop
        alerts = [e for e in self.rec.as_list()
                  if e["kind"] == "log.write" and "backoff" in e.get("message", "").lower()]
        self.assertEqual(len(alerts), 1, f"exactly one over-budget alert expected: {alerts}")

    def test_under_budget_no_stop_no_alert(self):
        b = self.make(budget=5)
        for t in (1000, 1010, 1020):
            self.assertFalse(b.record_crash(t))
        alerts = [e for e in self.rec.as_list() if "backoff" in e.get("message", "").lower()]
        self.assertEqual(alerts, [])

    def test_corrupt_backoff_degrades_to_fresh_with_warning(self):
        self.rt.backoff_path.write_text("{ not valid json", encoding="utf-8")
        b = self.make()
        self.assertFalse(b.record_crash(1000))  # treated as fresh -> count becomes 1
        self.assertEqual(b.count(1000), 1)
        warnings = [e for e in self.rec.as_list()
                    if e["kind"] == "log.write" and "corrupt" in e.get("message", "").lower()]
        self.assertTrue(warnings, "corrupt backoff must emit a warning effect")

    def test_invalid_schema_degrades_to_fresh_with_warning(self):
        # codex: a syntactically valid but wrong-SHAPE file must not crash or poison
        # the counter — treat as corrupt (fresh + warning), not silently kept.
        for bad in ('{"crashes": "notalist"}', '{"crashes": [1, "bad", 3]}', '{"other": 1}'):
            self.rt.backoff_path.write_text(bad, encoding="utf-8")
            rec = AW.EffectRecorder()
            b = AW.BackoffState(self.rt.backoff_path, budget=2, recorder=rec)
            self.assertFalse(b.record_crash(1000), bad)
            self.assertEqual(b.count(1000), 1, bad)  # fresh -> just this crash
            warnings = [e for e in rec.as_list()
                        if e["kind"] == "log.write" and "corrupt" in e.get("message", "").lower()]
            self.assertTrue(warnings, f"wrong-shape state must warn: {bad}")

    def test_crash_exactly_at_window_boundary_stays_in_window(self):
        # codex NIT: a crash exactly window_secs old is IN-window (inclusive).
        b = self.make(window_secs=100)
        b.record_crash(1000)
        self.assertEqual(b.count(1100), 1)  # exactly 100s old -> kept
        self.assertEqual(b.count(1101), 0)  # 101s old -> pruned

    def test_daemon_once_survives_backoff_write_failure(self):
        # codex IMPORTANT: backoff.json is a directory -> the success write fails,
        # but daemon --once must still succeed (heartbeat survives, rc0).
        self.rt.backoff_path.mkdir(parents=True)
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root), "--once"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertTrue(self.rt.heartbeat_path.is_file(), "heartbeat must survive a backoff write failure")

    def test_write_is_atomic_valid_json(self):
        b = self.make()
        b.record_crash(1000)
        data = json.loads(self.rt.backoff_path.read_text(encoding="utf-8"))
        self.assertIn("crashes", data)
        self.assertIsInstance(data["crashes"], list)

    def test_state_is_per_ae_home(self):
        other = AW.AewatchRuntime(self.root / "other")
        a = self.make()
        b = AW.BackoffState(other.backoff_path, budget=2, recorder=AW.EffectRecorder())
        a.record_crash(1000)
        self.assertEqual(a.count(1000), 1)
        self.assertEqual(b.count(1000), 0)  # independent root

    def test_daemon_once_calls_success_path(self):
        # seed some crashes, then a successful tick must reset them.
        self.rt.backoff_path.parent.mkdir(parents=True, exist_ok=True)
        self.rt.backoff_path.write_text(json.dumps({"crashes": [1.0, 2.0, 3.0]}), encoding="utf-8")
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root), "--once"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        data = json.loads(self.rt.backoff_path.read_text(encoding="utf-8"))
        self.assertEqual(data["crashes"], [], "a successful --once tick must reset the crash count")


if __name__ == "__main__":
    unittest.main()
