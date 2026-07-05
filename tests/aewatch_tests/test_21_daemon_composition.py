"""Phase-2 Slice 13 contract: daemon tick composition.

run_daemon_tick discovers sessions and does the phase-1 smoke work (heartbeat /
daemon.log / backoff). This slice lets it also RUN a watchdog cycle per discovered
session, under injection: a WatchdogRunner carrying per-session WatchdogState across
ticks + the fixture boundaries (tmux, git, emit_event, has_descendant,
recover_pending, supervise_bridge). The default CLI stays SAFE — with no runner (or
a NullTmuxClient) the tick mutates nothing and only emits file.write / log.write.

Parity: a daemon tick processes ALL sessions once, so over N ticks the effects
INTERLEAVE per tick (alpha-c1, beta-c1, alpha-c2, beta-c2) — a flat concat of the
direct per-session cycles would NOT match. So each session's SUB-STREAM (filtered
in order) must equal its direct run_python_watchdog_fixture cycle (itself
bash-verified in earlier slices).

Ticks here are CAPTURES-ONLY: MultiTickEnv routes per-tick events/heartbeat to the
FIRST session only, so multi-session fixtures needing recency/quiet/meta input wait
for the {session,event} extension (deferred). Non-git work_dirs. No ae edits.
"""

import datetime
import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_daemon_tick_fixture, run_daemon_tick_smoke, run_python_watchdog_fixture
from test_14_nudge_parity import _normalize  # META_DIR normalization (now session-name-general)

_PANE_SESSION = {"%1": "alpha", "%2": "beta"}


def _session_of(effect):
    """Which session an effect belongs to (this fixture: distinct panes per session)."""
    kind = effect["kind"]
    if kind == "event.append":
        return effect["session"]
    if kind in ("tmux.set_option", "tmux.unset_option"):
        return effect["target"]  # target is the session name
    if kind == "tmux.paste":
        return _PANE_SESSION.get(effect["target"])  # target is a pane_id
    return None

_E = 1783234800


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _session(name, work_dir, capture):
    return {
        "name": name, "tmux_server": "",
        "meta": {"session": name, "session_id": "s1", "tmux_server": "", "work_dir": work_dir,
                 "agent.main": "optimal:cw:s1"},
        "events": [],
        "panes": [{"pane_id": "%1", "agent": "optimal:cw", "current_command": "node",
                   "pane_pid": 999999, "capture": ""}],
        "_capture": capture,  # convenience for the single-session projection below
    }


def _multi_fixture(a_dir, b_dir):
    """Two sessions with distinct activity: 'alpha' active (hash changes), 'beta'
    stale->nudge. Proves the composed tick runs the full branch surface per session."""
    return {
        "id": "daemon.multi",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {"name": "alpha", "tmux_server": "",
             "meta": {"session": "alpha", "session_id": "s1", "tmux_server": "", "work_dir": a_dir,
                      "agent.main": "optimal:cw:s1"},
             "events": [], "panes": [{"pane_id": "%1", "agent": "optimal:cw", "current_command": "node",
                                      "pane_pid": 999999, "capture": ""}]},
            {"name": "beta", "tmux_server": "",
             "meta": {"session": "beta", "session_id": "s2", "tmux_server": "", "work_dir": b_dir,
                      "agent.main": "worker:w0:s2"},
             "events": [], "panes": [{"pane_id": "%2", "agent": "worker:w0", "current_command": "node",
                                      "pane_pid": 999998, "capture": ""}]},
        ],
        "ticks": [
            {"epoch": _E, "now": _iso(_E), "captures": {"%1": "f1", "%2": "idle"}},
            {"epoch": _E + 1000, "now": _iso(_E + 1000), "captures": {"%1": "f2", "%2": "idle"}},
        ],
    }


def _single(fixture, name):
    """Project one session out of a multi-session fixture into a single-session
    fixture (same ticks/captures) for the direct-cycle expectation."""
    sess = next(s for s in fixture["sessions"] if s["name"] == name)
    return {"id": f"single.{name}", "config": fixture["config"], "sessions": [sess], "ticks": fixture["ticks"]}


class DaemonCompositionTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.a = self.root / "a"; self.a.mkdir()
        self.b = self.root / "b"; self.b.mkdir()

    def test_composed_tick_runs_every_session_cycle(self):
        fx = _multi_fixture(str(self.a), str(self.b))
        composed = run_daemon_tick_fixture(fx)  # watchdog effects only (daemon file/log filtered)
        # Both sessions ran (their status side channel is present).
        self.assertEqual({_session_of(e) for e in composed} - {None}, {"alpha", "beta"})
        # Each session's sub-stream (in order) matches its direct, bash-verified cycle
        # (normalize the nudge paste META_DIR — composed + direct use different temps).
        for name in ("alpha", "beta"):
            got = _normalize([e for e in composed if _session_of(e) == name])
            expected = _normalize(run_python_watchdog_fixture(_single(fx, name)))
            self.assertEqual(got, expected, f"[{name}] composed sub-stream must match its direct cycle")

    def test_stopped_session_gets_no_cycle(self):
        # discover_sessions returns stopped session dirs too; the runner must skip
        # them (running=False) — no bootstrap/mutation for stale metadata (codex).
        fx = _multi_fixture(str(self.a), str(self.b))
        fx["sessions"].append({
            "name": "gamma", "tmux_server": "", "running": False,  # dir present, not in tmux
            "meta": {"session": "gamma", "session_id": "s3", "tmux_server": "", "work_dir": str(self.a),
                     "agent.main": "optimal:cw:s3"},
            "events": [], "panes": [{"pane_id": "%3", "agent": "optimal:cw", "current_command": "node",
                                     "pane_pid": 999997, "capture": ""}],
        })
        for t in fx["ticks"]:
            t["captures"]["%3"] = "idle"
        composed = run_daemon_tick_fixture(fx)
        seen = {_session_of(e) for e in composed} - {None}
        self.assertEqual(seen, {"alpha", "beta"}, "stopped 'gamma' must get no watchdog cycle")

    def test_default_cli_is_smoke_safe(self):
        # No runner + NullTmuxClient -> only the daemon's own file.write/log.write;
        # never a watchdog mutation (tmux/event/telegram).
        effects = run_daemon_tick_smoke(_multi_fixture(str(self.a), str(self.b)))
        forbidden = {"event.append", "tmux.set_option", "tmux.unset_option", "tmux.paste",
                     "tmux.display_message", "telegram.send", "telegram.supervise"}
        self.assertEqual({e["kind"] for e in effects} & forbidden, set(),
                         "default CLI smoke must not mutate sessions")
        self.assertTrue({e["kind"] for e in effects} <= {"file.write", "log.write"})


if __name__ == "__main__":
    unittest.main()
