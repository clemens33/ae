"""Phase-2 Slice 7 contract: quiet states (done / waiting-user / blocked).

The subtlest ported semantics. A quiet declaration tells the watchdog to stop
nudging:
  - done is EVENT-ONLY quiet: pane churn never invalidates it (ae:7839-7843).
  - waiting-user / blocked yield to pane activity via a per-declaration baseline
    (ae:7845-7864): ARM the baseline the first cycle a declaration is seen, HOLD
    while the pane hash matches it, YIELD (fall through to active/recent/stale)
    once the pane changes — the human usually replies in the pane, not via events.
A quiet state is invalidated only by a NEWER ae event whose actor != agent, or by
a genuinely new declaration (which re-arms the baseline).

These fixtures are multi-tick and DISTINGUISHING: without the quiet branch a
static pane past the stale window would nudge; with it, done/hold suppress the
nudge. wu-yield guards the other direction: quiet must NOT over-suppress once the
pane changes and the agent then goes idle.

Non-git work_dir. Pure stdlib, no ae edits. Re-arm (a second declaration) lands
in green with the per-tick event seam.
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


def _iso(epoch: int) -> str:
    return datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _decl(epoch, ref, summary):
    """An agent self-declaration event (`state <ref>`)."""
    return {"ts": _iso(epoch), "actor": _AGENT, "action": "state", "ref": ref, "summary": summary}


def _legacy_done(epoch, summary):
    """The legacy `action=done` event ae's state helper dual-emits AFTER the state
    event (ae:6016-6017) — no ref/target. _latest_relevant_event sees this later
    one, so a real port must map action==done -> quiet, not only state ref=done."""
    return {"ts": _iso(epoch), "actor": _AGENT, "action": "done", "summary": summary}


def _base(work_dir, ticks, events):
    return {
        "id": "watchdog.quiet",
        "config": {"ini": "[workspace]\nwatchdog = true\n"},
        "sessions": [
            {
                "name": "work",
                "tmux_server": "",
                "meta": {
                    "session": "work", "session_id": "s1", "tmux_server": "",
                    "work_dir": work_dir, "agent.main": f"{_AGENT}:s1",
                },
                "events": events,
                "panes": [{"pane_id": _PANE, "agent": _AGENT, "current_command": "node", "capture": ""}],
            }
        ],
        "ticks": ticks,
    }


def _done_holds_fixture(work_dir):
    """done declared via the REAL dual-emit (state ref=done THEN legacy action=done,
    ae:6012-6017); pane static past the stale window -> without quiet this would
    nudge; done suppresses it on both cycles (event-only quiet)."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "static"}},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "static"}},
        ],
        [_decl(_E, "done", "shipped it"), _legacy_done(_E, "shipped it")],
    )


def _inbound_invalidates_fixture(work_dir):
    """waiting-user declared (arm), then a NEWER inbound event (actor != agent,
    target == agent) becomes the latest relevant event -> quiet is invalidated ->
    the agent falls through to normal handling and, once idle past the window, is
    nudged. Pins _latest_relevant_event + the actor gate (guards against a port
    that only scans the agent's own declarations and over-holds)."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "echo"}},
            {"epoch": _E + 100, "now": _iso(_E + 100), "captures": {_PANE: "inbound"},
             "events": [{"ts": _iso(_E + 100), "actor": "claude:lead", "action": "send",
                         "target": _AGENT, "summary": "ping"}]},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "inbound"}},
        ],
        [_decl(_E, "waiting-user", "waiting for review")],
    )


def _rearm_fixture(work_dir):
    """waiting-user (arm), pane changes (yield -> active), then a NEW `state blocked`
    declaration at a later tick re-arms a FRESH baseline; the pane then stays static
    past the window -> hold -> no nudge. Without re-arm the stale pane nudges."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "echo1"}},
            {"epoch": _E + 100, "now": _iso(_E + 100), "captures": {_PANE: "changed"}},
            {"epoch": _E + 200, "now": _iso(_E + 200), "captures": {_PANE: "echo2"},
             "events": [_decl(_E + 200, "blocked", "blocked on infra")]},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "echo2"}},
        ],
        [_decl(_E, "waiting-user", "waiting for review")],
    )


def _waiting_user_hold_fixture(work_dir):
    """waiting-user declared; pane static past the stale window -> arm then hold ->
    no nudge (the human hasn't replied in the pane yet)."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "echo"}},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "echo"}},
        ],
        [_decl(_E, "waiting-user", "waiting for review")],
    )


def _waiting_user_yield_fixture(work_dir):
    """waiting-user declared; pane CHANGES (human replied) -> yield -> normal
    active/recent/stale. Then the pane goes idle past the window -> nudged. Guards
    that quiet does not over-suppress a post-reply hang."""
    return _base(
        work_dir,
        [
            {"epoch": _E, "now": _iso(_E), "captures": {_PANE: "echo"}},
            {"epoch": _E + 100, "now": _iso(_E + 100), "captures": {_PANE: "human-reply"}},
            {"epoch": _E + 2000, "now": _iso(_E + 2000), "captures": {_PANE: "human-reply"}},
        ],
        [_decl(_E, "waiting-user", "waiting for review")],
    )


class QuietParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"
        self.nogit.mkdir()

    def _parity(self, fixture, label):
        bash = _normalize(run_bash_watchdog_fixture(fixture, self.root / f"bash-{label}"))
        python = _normalize(run_python_watchdog_fixture(fixture))
        self.assertEqual(python, bash, f"[{label}] python quiet stream diverged from bash")
        self.assertTrue(bash, f"[{label}] bash produced no effects")
        return python

    def test_done_holds_through_stale_window(self):
        python = self._parity(_done_holds_fixture(str(self.nogit)), "done")
        self.assertNotIn("event.append", {e["kind"] for e in python}, "done must suppress the nudge")
        self.assertNotIn("tmux.paste", {e["kind"] for e in python})

    def test_waiting_user_holds(self):
        python = self._parity(_waiting_user_hold_fixture(str(self.nogit)), "wu-hold")
        self.assertNotIn("event.append", {e["kind"] for e in python}, "waiting-user hold must suppress the nudge")

    def test_waiting_user_yields_then_nudges(self):
        python = self._parity(_waiting_user_yield_fixture(str(self.nogit)), "wu-yield")
        nudges = [e for e in python if e["kind"] == "event.append" and e["event"].get("action") == "nudge"]
        self.assertEqual(len(nudges), 1, "post-reply idle must nudge exactly once (quiet yielded)")

    def test_inbound_event_invalidates_quiet(self):
        python = self._parity(_inbound_invalidates_fixture(str(self.nogit)), "inbound")
        nudges = [e for e in python if e["kind"] == "event.append" and e["event"].get("action") == "nudge"]
        self.assertEqual(len(nudges), 1, "inbound event must invalidate quiet -> one nudge")

    def test_new_declaration_rearms(self):
        python = self._parity(_rearm_fixture(str(self.nogit)), "rearm")
        self.assertNotIn("event.append", {e["kind"] for e in python},
                         "a fresh declaration must re-arm the baseline -> no nudge")


if __name__ == "__main__":
    unittest.main()
