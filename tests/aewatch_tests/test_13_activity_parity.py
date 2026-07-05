"""Phase-2 Slice 5 contract: event recency + pane activity parity.

Extends the dual-run oracle from the status side channel to the AGENT ACTIVITY
classification: an agent counts toward cycle_active when (5) its pane hash changed
since last cycle, (6) it changed within the stale window ("recently visible"), or
(7) it emitted a recent ae event ("recently alive"). None of these paths emit a
nudge or alert — they only move the health count in `[watch ● <active>/<total>]`.

So slice-5 parity = the active count in the status string matches bash across
ticks, and NO nudge/alert/event effects appear (those are slices 6-9). Each
fixture is multi-tick, since activity is a per-cycle diff against carried state
(prev_hash / last_hash_change in WatchdogState).

Both sides run over the SAME fixture. Non-git work_dir (deterministic branch).
Pure stdlib, no ae edits.
"""

import datetime
import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture

_E = 1783234800  # tick-1 epoch == 2026-07-05T07:00:00Z
_AGENT = "optimal:cw"
_PANE = "%1"


def _iso(epoch: int) -> str:
    """ae's event ts format: `date -u +%FT%TZ` -> 2026-07-05T07:00:00Z."""
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _base(work_dir, ticks, events=None):
    """One session, non-git work_dir, one NON-shell agent pane (so the dead check
    is skipped and the agent reaches the activity branches)."""
    return {
        "id": "watchdog.activity",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {
                "name": "work",
                "tmux_server": "",
                "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir},
                "events": events or [],
                "panes": [
                    {"pane_id": _PANE, "agent": _AGENT, "current_command": "node", "capture": ""},
                ],
            }
        ],
        "ticks": ticks,
    }


def _active_hash_fixture(work_dir):
    """Pane content changes between ticks -> active via hash diff (branch 5) both
    cycles."""
    return _base(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "frame-1"}},
        {"epoch": _E + 60, "now": _iso(_E + 60), "captures": {_PANE: "frame-2"}},
    ])


def _recently_visible_fixture(work_dir):
    """Pane static, but the last change is within STALE_SECS (900) -> recently
    visible (branch 6) on tick 2."""
    return _base(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "static"}},
        {"epoch": _E + 60, "now": _iso(_E + 60), "captures": {_PANE: "static"}},
    ])


def _recent_event_fixture(work_dir):
    """Pane static and last change is OLDER than STALE_SECS, but a recent ae event
    from the agent keeps it alive (branch 7) on tick 2."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "static"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "static"}},
        ],
        events=[{"ts": _iso(_E + 900), "actor": _AGENT, "action": "working", "summary": "still going"}],
    )


class ActivityParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _assert_parity(self, fixture, label):
        bash = run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}")
        python = run_python_watchdog_fixture(fixture)
        self.assertEqual(python, bash, f"[{label}] python cycle diverged from the bash watchdog stream")
        self.assertTrue(bash, f"[{label}] bash oracle produced no effects")
        # Activity paths never nudge/alert; only the status count moves.
        kinds = {e["kind"] for e in python}
        for forbidden in ("event.append", "tmux.paste", "tmux.display_message", "telegram.send"):
            self.assertNotIn(forbidden, kinds, f"[{label}] activity fixture must emit no {forbidden}")
        # Every end-of-cycle health count is 1/1 (the one agent is active).
        health = [e["value"] for e in python
                  if e["kind"] == "tmux.set_option" and e["option"] == "@ae_watchdog_status"]
        self.assertTrue(any("1/1" in v for v in health), f"[{label}] no [watch ● 1/1] health: {health}")

    def test_active_hash_change(self):
        self._assert_parity(_active_hash_fixture(str(self.nogit)), "active")

    def test_recently_visible(self):
        self._assert_parity(_recently_visible_fixture(str(self.nogit)), "recent-visible")

    def test_recent_event(self):
        self._assert_parity(_recent_event_fixture(str(self.nogit)), "recent-event")


if __name__ == "__main__":
    unittest.main()
