"""Phase-2 Slice 8 contract: alerts + missing/dead panes.

display_message finally speaks. Three alert families, each an ae_emit_event(alert)
emitted DIRECTLY by the watchdog (not via send) — so actor is 'human', not
'watchdog' — plus a `tmux display-message -d 10000` user-visible alert:
  - dead pane (ae:7703-7717): the pane's foreground is a shell and the agent is
    not a running descendant -> dead. Latched via dead_agents.
  - missing pane (ae:7954-7963): a registered meta agent whose pane no longer
    exists. Latched via alerted_missing.
  - max-nudge alert (ae:7942-7947): a stale agent nudged MAX_NUDGES times then
    alerted once; further stale cycles are silent.

Dead detection uses `pgrep -x <agent_bin>` on the bash side; the fixtures set
agent_bin.<slot> to a nonexistent binary so pgrep is empty on any machine, and the
Python side injects has_descendant=False to match. Non-git work_dir. No ae edits.
"""

import datetime
import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture
from test_14_nudge_parity import _normalize

_E = 1783234800
_AGENT = "optimal:cw"
_PANE = "%1"
_NOPROC = "zzdeadbin"  # never a real process -> pgrep -x empty -> dead on any host


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _dead_pane_fixture(work_dir):
    """Pane foreground is a shell + no live descendant -> dead alert, then latched."""
    return {
        "id": "watchdog.alert.dead-pane",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{
            "name": "work", "tmux_server": "",
            "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                     "agent.main": f"{_AGENT}:s1", "agent_bin.main": _NOPROC},
            "events": [],
            "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "bash", "pane_pid": 999999, "capture": ""}],
        }],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "$ "}},
            {"epoch": _E + 60, "now": _iso(_E + 60), "captures": {_PANE: "$ "}},
        ],
    }


def _missing_pane_fixture(work_dir):
    """A registered agent with NO pane -> missing alert, then latched."""
    return {
        "id": "watchdog.alert.missing-pane",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{
            "name": "work", "tmux_server": "",
            "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                     "agent.main": f"{_AGENT}:s1"},
            "events": [],
            "panes": [],
        }],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {}},
            {"epoch": _E + 60, "now": _iso(_E + 60), "captures": {}},
        ],
    }


def _max_nudge_fixture(work_dir):
    """Stale agent nudged MAX_NUDGES (2) times then alerted once on the 3rd stale
    cycle; the 4th is silent."""
    return {
        "id": "watchdog.alert.max-nudge",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{
            "name": "work", "tmux_server": "",
            "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                     "agent.main": f"{_AGENT}:s1"},
            "events": [],
            "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node", "pane_pid": 999999, "capture": ""}],
        }],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "idle"}},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "idle"}},
            {"epoch": _E + 3000, "now": _iso(_E + 3000), "captures": {_PANE: "idle"}},
        ],
    }


class AlertParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        # Normalize the max-nudge fixture's pre-alert nudge paste META_DIR paths
        # (different temp AE_HOMEs) so a diff reflects ALERT behavior, not temp-path
        # noise (codex).
        bash = _normalize(run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}"))
        python = _normalize(run_python_watchdog_fixture(fixture))
        self.assertEqual(python, bash, f"[{label}] python alert stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def _alerts(self, effects):
        return [e for e in effects if e["kind"] == "event.append" and e["event"].get("action") == "alert"]

    def _displays(self, effects):
        return [e for e in effects if e["kind"] == "tmux.display_message"]

    def test_dead_pane_alert_and_latch(self):
        python = self._parity(_dead_pane_fixture(str(self.nogit)), "dead")
        self.assertEqual(len(self._alerts(python)), 1, "dead alert fires once (latched)")
        self.assertEqual(self._alerts(python)[0]["event"].get("actor"), "human")
        self.assertEqual(self._alerts(python)[0]["event"].get("summary"), "agent process dead — dropped to shell")
        self.assertEqual(len(self._displays(python)), 1)
        self.assertEqual(self._displays(python)[0]["duration_ms"], 10000)

    def test_missing_pane_alert_and_latch(self):
        python = self._parity(_missing_pane_fixture(str(self.nogit)), "missing")
        self.assertEqual(len(self._alerts(python)), 1, "missing alert fires once (latched)")
        self.assertEqual(self._alerts(python)[0]["event"].get("actor"), "human")
        self.assertEqual(self._alerts(python)[0]["event"].get("summary"),
                         "pane missing — agent no longer visible in session")
        self.assertEqual(len(self._displays(python)), 1)

    def test_max_nudge_alert(self):
        python = self._parity(_max_nudge_fixture(str(self.nogit)), "max-nudge")
        nudges = [e for e in python if e["kind"] == "event.append" and e["event"].get("action") == "nudge"]
        self.assertEqual(len(nudges), 2, "exactly MAX_NUDGES nudges before the alert")
        self.assertEqual(len(self._alerts(python)), 1, "one max-nudge alert")
        self.assertEqual(self._alerts(python)[0]["event"].get("actor"), "human")
        self.assertIn("max nudges reached", self._alerts(python)[0]["event"].get("summary", ""))
        self.assertEqual(len(self._displays(python)), 1)


if __name__ == "__main__":
    unittest.main()
