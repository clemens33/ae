"""Phase-2 Slice 6 contract: the stale nudge path.

First slice where the watchdog EMITS side effects into an agent: a stale agent
(pane static past the stale window, no recent event) gets nudged — a tmux.paste
of the nudge text into its pane PLUS an event.append(action=nudge). The nudge
text is user-visible, so it is byte-exact incl. the Session-goal prefix.

Parity trap: the nudge text embeds ${META_DIR}/state (an absolute path that
differs between the two oracle runs), so the paste text is compared after
normalizing that path to a placeholder — the only runtime-variable part.

Multi-tick: tick 1 baselines the pane (active via first hash); tick 2 is past the
stale window with no recent event -> stale -> ONE nudge (count 0 < MAX_NUDGES).

Non-git work_dir. tool "other" (non-shell "node", not codex/claude) -> the default
paste path. Pure stdlib, no ae edits.
"""

import datetime
import re
import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture

_E = 1783234800
_AGENT = "optimal:cw"
_PANE = "%1"
_GOAL = "ship aewatch"


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _idle_once_fixture(work_dir):
    """Stale agent, session goal set. Pane static, ticks 1000s apart (> STALE_SECS
    900), no events -> tick 2 nudges exactly once ('no recent events')."""
    return {
        "id": "watchdog.nudge.idle-once",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {
                "name": "work",
                "tmux_server": "",
                "meta": {
                    "session": "work", "session_id": "s1", "tmux_server": "",
                    "work_dir": work_dir, "goal": _GOAL, "agent.main": f"{_AGENT}:s1",
                },
                "events": [],
                "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node", "capture": ""}],
            }
        ],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle-frame"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "idle-frame"}},
        ],
    }


def _normalize(effects):
    """Replace the run-specific META_DIR in nudge paste text with a placeholder so
    the two oracle runs (different temp AE_HOMEs) compare byte-for-byte."""
    out = []
    for e in effects:
        e = dict(e)
        if e.get("kind") == "tmux.paste" and isinstance(e.get("text"), str):
            # Any session name segment (…/sessions/<name>/state), so this is reusable
            # across multi-session fixtures (daemon composition), not just 'work'.
            e["text"] = re.sub(r"\S+/sessions/[^/\s]+/state", "<META_DIR>/state", e["text"])
        out.append(e)
    return out


class NudgeParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def test_idle_once_nudge_parity(self):
        fixture = _idle_once_fixture(str(self.nogit))
        bash = _normalize(run_bash_watchdog_fixture(fixture, self.root / "bash"))
        python = _normalize(run_python_watchdog_fixture(fixture))
        self.assertEqual(python, bash, "python nudge stream diverged from bash")

        # The bash side must ALSO record exactly one paste (codex) — proves the
        # fake-tmux paste seam, not a python-only paste hidden by normalized equality.
        self.assertEqual(len([e for e in bash if e["kind"] == "tmux.paste"]), 1,
                         "bash oracle must record exactly one nudge paste (fake-tmux seam)")

        # Exactly one nudge event with the ae summary, and one paste carrying the
        # goal-quoted nudge text.
        nudges = [e for e in python if e["kind"] == "event.append" and e["event"].get("action") == "nudge"]
        self.assertEqual(len(nudges), 1, "expected exactly one nudge event")
        self.assertEqual(nudges[0]["event"].get("summary"), "no recent events, no recent ae activity")
        self.assertEqual(nudges[0]["event"].get("actor"), "watchdog")

        pastes = [e for e in python if e["kind"] == "tmux.paste"]
        self.assertEqual(len(pastes), 1, "expected exactly one nudge paste")
        text = pastes[0]["text"]
        self.assertTrue(text.startswith(f"Session goal: {_GOAL}. Status check:"),
                        f"nudge paste missing goal prefix: {text!r}")
        self.assertIn("<META_DIR>/state <waiting-user|blocked|done>", text)

        # Stale glyph on the nudging cycle.
        health = [e["value"] for e in python
                  if e["kind"] == "tmux.set_option" and e["option"] == "@ae_watchdog_status"]
        self.assertTrue(any("⚠ 0/1" in v for v in health), f"no [watch ⚠ 0/1] health: {health}")


if __name__ == "__main__":
    unittest.main()
