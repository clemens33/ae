"""Phase-2 Slice 10 contract: meta-agent (steward) sweep cadence + wedge.

The main agent of a meta session (meta_agent=true) is a long-running SERVICE, so
"idle between sweeps" is NORMAL, not stale. Instead of the stale-nudge path, the
watchdog (ae:7719-7798): (a) prompts a sweep every SWEEP_SECS (a nudge with a fixed
'Run your sweep now: …' paste + summary 'sweep cadence', actor 'watchdog'), and
(b) guards liveness via the steward's OWN heartbeat file (meta-agent-state.json) —
if it stops advancing after we've prompted long enough, raise ONE wedge alert
(actor 'human') + display, cleared on recovery.

Covers sweep cadence, wedge from an absent heartbeat, the meta gate (non-meta and
meta-non-main workers use the normal watchdog), latch-based recovery via a fresh
heartbeat, and the post-restart reconcile. The oracle originally exposed that
reconcile as DEAD CODE in ae (the generated watchdog helper omitted
_agent_alert_reason); slice 10b FIXED it (ae now emits _agent_alert_reason into
_lib), so the fixture pins exactly one durable-log alert-cleared. Recovery uses a
heartbeat-mtime seam (per-tick, integer epoch - age on both sides). Non-git
work_dir.
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
_SWEEP_TEXT = ("Run your sweep now: ae list --json, diff your state file, and report ONLY "
               "new/changed attention to Clemens via say (stay silent if nothing changed). "
               "Stay in 'working'.")


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _meta_fixture(work_dir, ticks, *, meta_agent=True, agent="optimal:cw"):
    meta = {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
            "agent.main": f"{agent}:s1"}
    if meta_agent:
        meta["meta_agent"] = "true"
    return {
        "id": "watchdog.meta.sweep",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{"name": "work", "tmux_server": "", "meta": meta, "events": [],
                      "panes": [{"pane_id": _PANE, "agent": agent, "current_command": "node",
                                 "pane_pid": 999999, "capture": ""}]}],
        "ticks": ticks,
    }


def _sweep_cadence_fixture(work_dir):
    """tick1 prompts a sweep; tick2 (<SWEEP_SECS later) does not re-prompt and, being
    within the wedge window, raises no alert."""
    return _meta_fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}},
        {"epoch": _E + 100, "now": _iso(_E + 100), "captures": {_PANE: "idle"}},
    ])


def _wedge_no_heartbeat_fixture(work_dir):
    """No heartbeat ever. tick1 prompts; tick2 (past the wedge window with no
    heartbeat) raises one wedge alert AND prompts again."""
    return _meta_fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}},
        {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "idle"}},
    ])


def _non_meta_fixture(work_dir):
    """A NON-meta session's main agent gets the NORMAL watchdog (stale nudge), never
    a sweep — proves the meta_agent gate."""
    return _meta_fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}},
        {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "idle"}},
    ], meta_agent=False)


def _meta_non_main_fixture(work_dir):
    """A meta session with a main + a worker. Only the MAIN is swept; the worker
    (not META_MAIN_AGENT) follows the normal watchdog — proves the agent==main half
    of the gate (codex)."""
    return {
        "id": "watchdog.meta.non-main",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [{"name": "work", "tmux_server": "",
            "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                     "agent.main": "optimal:cw:s1", "agent.worker": "worker:w1:s2", "meta_agent": "true"},
            "events": [],
            "panes": [
                {"pane_id": "%1", "agent": "optimal:cw", "current_command": "node", "pane_pid": 999999, "capture": ""},
                {"pane_id": "%2", "agent": "worker:w1", "current_command": "node", "pane_pid": 999998, "capture": ""},
            ]}],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {"%1": "idle", "%2": "idle"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {"%1": "idle", "%2": "idle"}},
        ]}


def _wedge_recover_fixture(work_dir):
    """No heartbeat -> wedge on tick2; a FRESH heartbeat on tick3 -> alert-cleared."""
    return _meta_fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}},
        {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {_PANE: "idle"}},
        {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "idle"}, "heartbeats": {"work": {"age": 10}}},
    ])


def _post_restart_reconcile_fixture(work_dir):
    """A prior watchdog's wedge alert is in the durable log; a fresh watchdog (fresh
    WatchdogState) sees a fresh heartbeat and reconciles it -> one alert-cleared.
    The oracle originally found this path DEAD (ae omitted _agent_alert_reason from
    the watchdog helper); slice 10b emits it via _lib, so the reconcile now fires on
    both sides."""
    fx = _meta_fixture(work_dir, [
        {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "idle"}, "heartbeats": {"work": {"age": 10}}},
    ])
    fx["sessions"][0]["events"] = [
        {"ts": _iso(_E - 100), "actor": "human", "action": "alert", "target": _AGENT,
         "summary": "meta-agent not sweeping — never wrote a heartbeat in 20m of sweep prompts (may be stuck)"}
    ]
    return fx


class SweepParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        # Normalize the non-meta guard's stale-nudge META_DIR path (the sweep paste
        # itself is a fixed string with no path).
        bash = _normalize(run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}"))
        python = _normalize(run_python_watchdog_fixture(fixture))
        self.assertEqual(python, bash, f"[{label}] python sweep stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def _nudges(self, effects):
        return [e for e in effects if e["kind"] == "event.append" and e["event"].get("action") == "nudge"]

    def _alerts(self, effects):
        return [e for e in effects if e["kind"] == "event.append" and e["event"].get("action") == "alert"]

    def test_sweep_cadence(self):
        python = self._parity(_sweep_cadence_fixture(str(self.nogit)), "cadence")
        nudges = self._nudges(python)
        self.assertEqual(len(nudges), 1, "exactly one sweep prompt on the cadence")
        self.assertEqual(nudges[0]["event"].get("summary"), "sweep cadence")
        self.assertEqual(nudges[0]["event"].get("actor"), "watchdog")
        pastes = [e for e in python if e["kind"] == "tmux.paste"]
        self.assertEqual(len(pastes), 1)
        self.assertEqual(pastes[0]["text"], _SWEEP_TEXT)
        self.assertEqual(len(self._alerts(python)), 0, "no wedge alert within the window")

    def test_wedge_from_absent_heartbeat(self):
        python = self._parity(_wedge_no_heartbeat_fixture(str(self.nogit)), "wedge")
        alerts = self._alerts(python)
        self.assertEqual(len(alerts), 1, "one wedge alert")
        self.assertEqual(alerts[0]["event"].get("actor"), "human")
        self.assertIn("not sweeping", alerts[0]["event"].get("summary", ""))
        self.assertIn("never wrote a heartbeat", alerts[0]["event"].get("summary", ""))
        self.assertEqual(len([e for e in python if e["kind"] == "tmux.display_message"]), 1)

    def test_non_meta_agent_uses_normal_watchdog(self):
        python = self._parity(_non_meta_fixture(str(self.nogit)), "non-meta")
        # A normal (non-meta) idle agent goes stale and is NUDGED, not swept.
        self.assertTrue(len(self._nudges(python)) >= 1, "non-meta idle agent is nudged")
        self.assertNotIn("sweep cadence",
                         [n["event"].get("summary") for n in self._nudges(python)],
                         "non-meta agent must not get a sweep prompt")

    def test_meta_worker_uses_normal_watchdog(self):
        python = self._parity(_meta_non_main_fixture(str(self.nogit)), "non-main")
        sweeps = [n for n in self._nudges(python) if n["event"].get("summary") == "sweep cadence"]
        others = [n for n in self._nudges(python) if n["event"].get("summary") != "sweep cadence"]
        self.assertTrue(sweeps, "the main agent must be swept")
        self.assertTrue(all(n["event"].get("target") == "optimal:cw" for n in sweeps),
                        "only the main is swept")
        self.assertTrue(any(n["event"].get("target") == "worker:w1" for n in others),
                        "the worker must get a NORMAL (non-sweep) nudge")

    def test_wedge_then_recover(self):
        python = self._parity(_wedge_recover_fixture(str(self.nogit)), "wedge-recover")
        self.assertEqual(len(self._alerts(python)), 1, "one wedge alert")
        cleared = [e for e in python if e["kind"] == "event.append"
                   and e["event"].get("action") == "alert-cleared"]
        self.assertEqual(len(cleared), 1, "one alert-cleared on recovery")
        self.assertEqual(cleared[0]["event"].get("actor"), "human")
        self.assertEqual(cleared[0]["event"].get("summary"), "meta-agent sweeping again (heartbeat resumed)")

    def test_post_restart_reconcile(self):
        # Slice 10b fixes the ae bug (emits _agent_alert_reason into the watchdog
        # helper): a fresh watchdog seeing a fresh heartbeat + a durable wedge alert
        # reconciles it -> exactly one alert-cleared, on BOTH sides.
        python = self._parity(_post_restart_reconcile_fixture(str(self.nogit)), "reconcile")
        cleared = [e for e in python if e["kind"] == "event.append"
                   and e["event"].get("action") == "alert-cleared"]
        self.assertEqual(len(cleared), 1, "a fresh watchdog reconciles the durable wedge alert")
        self.assertEqual(cleared[0]["event"].get("actor"), "human")


if __name__ == "__main__":
    unittest.main()
