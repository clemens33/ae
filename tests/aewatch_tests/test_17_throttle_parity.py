"""Phase-2 Slice 9 contract: throttle detection.

When an upstream rate-limit / overload phrase appears in an agent's pane, the
watchdog PAUSES nudges (the agent is not stuck, it is throttled), emits a
`throttled` event once, escalates to an `alert` + display-message after
THROTTLE_ALERT_CYCLES consecutive throttled cycles, and emits `throttle-cleared`
when the pane recovers. All three events are DIRECT emits -> actor 'human'.

Pattern-catalog fidelity is the risk (ae:7517): the per-tool catalogs
(claude/codex/gemini, opencode = their union) + the generic 429/503 patterns are
ported VERBATIM. Every catalog string is exercised through both oracles: a fixture
runs one pattern per tick, so a DROPPED string fails to throttle on its tick and
surfaces as a divergent `throttle-cleared` (which the byte-exact diff catches).

INTERVAL: the alert figure is THROTTLE_ALERT_CYCLES * INTERVAL. run_python_watchdog_
fixture runs the SAME interval the bash oracle forces (via bash_oracle._ORACLE_*),
so the summary is byte-exact and the formula itself is under test — no normalization
(codex). Non-git work_dir. No ae edits.
"""

import datetime
import shutil
import tempfile
import unittest
from pathlib import Path

import bash_oracle
from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture

_E = 1783234800
_AGENT = "optimal:cw"
_PANE = "%1"
_THROTTLE_ALERT_CYCLES = 5  # ae/aewatch default; both oracles agree

# VERBATIM catalogs (ae:7517) — exercised string-by-string.
_CATALOGS = {
    "claude": ("Server is temporarily limiting requests", "API Error: Overloaded", "Anthropic API error"),
    "codex": ("Rate limit exceeded", "RateLimitError", "ratelimit_exceeded"),
    "gemini": ("RESOURCE_EXHAUSTED", "Quota exceeded"),
}
# opencode = the FULL union of all three catalogs (codex: prove every opencode
# phrase, not a sample — a port reduced to a subset would otherwise pass).
_OPENCODE_UNION = _CATALOGS["claude"] + _CATALOGS["codex"] + _CATALOGS["gemini"]

_EXPECTED_ALERT = f"throttled for {_THROTTLE_ALERT_CYCLES * bash_oracle._ORACLE_INTERVAL}s — may need attention"


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _fixture(work_dir, ticks, agent_bin=None):
    meta = {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
            "agent.main": f"{_AGENT}:s1"}
    if agent_bin:
        meta["agent_bin.main"] = agent_bin
    return {
        "id": "watchdog.throttle",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{"name": "work", "tmux_server": "", "meta": meta, "events": [],
                      "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node",
                                 "pane_pid": 999999, "capture": ""}]}],
        "ticks": ticks,
    }


def _pause_fixture(work_dir, pattern="429 Too Many Requests", agent_bin=None):
    """Pane shows a throttle phrase, static past the stale window -> without throttle
    detection this nudges; with it, nudges are PAUSED and a throttled event fires."""
    buf = f"working... {pattern}"
    return _fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: buf}},
        {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: buf}},
    ], agent_bin)


def _alert_recover_fixture(work_dir):
    """5 consecutive throttled cycles -> alert on the 5th, then a recovery cycle ->
    throttle-cleared."""
    buf = "boom 503 Service Unavailable"
    ticks = [{"epoch": _E + i * 60, "now": _iso(_E + i * 60), "captures": {_PANE: buf}} for i in range(5)]
    ticks.append({"epoch": _E + 5 * 60, "now": _iso(_E + 5 * 60), "captures": {_PANE: "recovered and working"}})
    return _fixture(work_dir, ticks)


def _catalog_fixture(work_dir, tool, patterns):
    """One tick per catalog string. If EVERY string throttles, exactly one throttled
    event (tick 0) and ZERO throttle-cleared; a dropped string breaks throttle on its
    tick -> a throttle-cleared appears (and the byte-exact diff diverges)."""
    ticks = [{"epoch": _E + i * 60, "now": _iso(_E + i * 60), "captures": {_PANE: f"error: {p}"}}
             for i, p in enumerate(patterns)]
    return _fixture(work_dir, ticks, agent_bin=tool)


class ThrottleParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        bash = run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}")
        python = run_python_watchdog_fixture(fixture)
        self.assertEqual(python, bash, f"[{label}] python throttle stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def _actions(self, effects, action):
        return [e for e in effects if e["kind"] == "event.append" and e["event"].get("action") == action]

    def test_throttle_pauses_nudges(self):
        python = self._parity(_pause_fixture(str(self.nogit)), "pause")
        throttled = self._actions(python, "throttled")
        self.assertEqual(len(throttled), 1, "one throttled event")
        self.assertEqual(throttled[0]["event"].get("actor"), "human")
        self.assertEqual(throttled[0]["event"].get("summary"), "upstream throttling detected — pausing nudges")
        self.assertEqual(len(self._actions(python, "nudge")), 0, "throttle must pause nudges")

    def test_throttle_alert_and_recover(self):
        python = self._parity(_alert_recover_fixture(str(self.nogit)), "alert-recover")
        self.assertEqual(len(self._actions(python, "throttled")), 1)
        alerts = self._actions(python, "alert")
        self.assertEqual(len(alerts), 1, "one throttle alert at the streak")
        self.assertEqual(alerts[0]["event"].get("actor"), "human")
        self.assertEqual(alerts[0]["event"].get("summary"), _EXPECTED_ALERT)  # byte-exact formula
        cleared = self._actions(python, "throttle-cleared")
        self.assertEqual(len(cleared), 1)
        self.assertEqual(cleared[0]["event"].get("actor"), "human")
        self.assertEqual(cleared[0]["event"].get("summary"), "throttling cleared after 5 cycles")
        self.assertEqual(len([e for e in python if e["kind"] == "tmux.display_message"]), 1)

    def test_per_tool_catalogs_verbatim(self):
        cases = dict(_CATALOGS)
        cases["opencode"] = _OPENCODE_UNION
        for tool, patterns in cases.items():
            with self.subTest(tool=tool):
                python = self._parity(_catalog_fixture(str(self.nogit), tool, patterns), f"cat-{tool}")
                self.assertEqual(len(self._actions(python, "throttled")), 1,
                                 f"{tool}: one throttled event across its catalog")
                self.assertEqual(len(self._actions(python, "throttle-cleared")), 0,
                                 f"{tool}: every catalog string must throttle (a drop -> a clear)")


if __name__ == "__main__":
    unittest.main()
