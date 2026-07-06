"""Phase-3 Slice 1 contract: per-session tick-input routing.

Phase 2 routed tick `events` / heartbeat / recover to the FIRST session only. Phase
3 has multiple live sessions per daemon tick, so tick inputs must be SESSION-KEYED
and the validator must REJECT the old first-session shortcuts:
  - events: a list of {session, event} (a flat event, or an entry missing `session`,
    is rejected);
  - heartbeats: {session: {age}} (the phase-2 singular `heartbeat_age` is rejected);
  - recover: {session: [rows]} (a flat list is rejected).

MultiTickEnv routes each to the named session; old captures-only phase-2 fixtures
(no tick inputs) are unaffected, and a two-session fixture proves routing. Pure
stdlib.
"""

import tempfile
import unittest
from pathlib import Path

from harness import AW, MultiTickEnv

_E = 1783234800


def _contracts_with_tick(tick_extra, sessions=("work", "steward")):
    """A minimal valid contracts object with one fixture (declaring `sessions`) whose
    single tick carries the given input keys (on top of a numeric epoch)."""
    return {
        "schema_version": 1,
        "fixtures": [{
            "id": "watchdog.tick-schema", "time": {}, "config": {},
            "sessions": [{"name": n} for n in sessions],
            "telegram": {}, "expect": {"effects": []},
            "ticks": [dict({"epoch": 1783234800}, **tick_extra)],
        }],
    }


class TickSchemaValidationTest(unittest.TestCase):
    def _tick_errs(self, tick_extra, sessions=("work", "steward")):
        # Only the tick-input errors — filter out the unrelated required-fixture-family
        # completeness errors from this minimal single-fixture object.
        return [e for e in AW.validate_contracts(_contracts_with_tick(tick_extra, sessions)) if "ticks[0]" in e]

    def test_flat_events_without_session_rejected(self):
        errs = self._tick_errs({"events": [{"actor": "claude:lead", "action": "send"}]})
        self.assertTrue(any("session" in e for e in errs),
                        f"a flat (first-session) event list must be rejected: {errs}")

    def test_event_entry_missing_session_rejected(self):
        errs = self._tick_errs({"events": [{"event": {"action": "send"}}]})
        self.assertTrue(any("session" in e for e in errs),
                        f"an event entry without a session key must be rejected: {errs}")

    def test_singular_heartbeat_age_rejected(self):
        errs = self._tick_errs({"heartbeat_age": 10})
        self.assertTrue(any("heartbeat" in e.lower() for e in errs),
                        f"the phase-2 singular heartbeat_age must be rejected: {errs}")

    def test_flat_recover_list_rejected(self):
        errs = self._tick_errs({"recover": [{"kind": "ok", "slot": "main", "agent": "codex:lead"}]})
        self.assertTrue(any("recover" in e.lower() for e in errs),
                        f"a flat recover list (not session-keyed) must be rejected: {errs}")

    def test_unknown_session_ref_rejected(self):
        # A tick input naming a session absent from fixture['sessions'] must be
        # rejected — across events, heartbeats, AND recover (codex).
        for label, extra in (
            ("event", {"events": [{"session": "ghost", "event": {"action": "send"}}]}),
            ("heartbeat", {"heartbeats": {"ghost": {"age": 5}}}),
            ("recover", {"recover": {"ghost": [{"kind": "ok"}]}}),
        ):
            with self.subTest(kind=label):
                errs = self._tick_errs(extra)
                self.assertTrue(any("ghost" in e for e in errs),
                                f"unknown {label} session must be rejected: {errs}")

    def test_empty_session_key_rejected(self):
        for label, extra in (
            ("heartbeat", {"heartbeats": {"": {"age": 5}}}),
            ("recover", {"recover": {"": [{"kind": "ok"}]}}),
        ):
            with self.subTest(kind=label):
                errs = self._tick_errs(extra)
                self.assertTrue(any("not a known session" in e for e in errs),
                                f"empty {label} session key must be rejected: {errs}")

    def test_well_formed_per_session_inputs_accepted(self):
        errs = self._tick_errs({
            "events": [{"session": "work", "event": {"actor": "claude:lead", "action": "send"}}],
            "heartbeats": {"steward": {"age": 10}},
            "recover": {"work": [{"kind": "ok", "slot": "main", "agent": "codex:lead", "tool": "codex", "captured": "x"}]},
        })
        self.assertEqual(errs, [], f"well-formed per-session tick inputs must validate: {errs}")


class TickRoutingTest(unittest.TestCase):
    def test_inputs_route_to_the_named_session(self):
        # Two sessions; a tick event targets beta and a heartbeat targets alpha —
        # each must land ONLY in its named session (no first-session shortcut).
        fixture = {
            "id": "daemon.route", "config": {"ini": ""},
            "sessions": [
                {"name": "alpha", "tmux_server": "", "meta": {"session": "alpha"}, "events": [], "panes": []},
                {"name": "beta", "tmux_server": "", "meta": {"session": "beta"}, "events": [], "panes": []},
            ],
            "ticks": [{
                "epoch": _E,
                "events": [{"session": "beta", "event": {"actor": "claude:lead", "action": "send"}}],
                "heartbeats": {"alpha": {"age": 5}},
            }],
        }
        with tempfile.TemporaryDirectory() as tmp:
            env = MultiTickEnv(fixture, tmp)
            env.start_tick(0)
            sessions = env.home.sessions
            self.assertIn('"action":"send"', (sessions / "beta" / "events.jsonl").read_text(),
                          "the event must route to beta")
            self.assertEqual((sessions / "alpha" / "events.jsonl").read_text().strip(), "",
                             "alpha must NOT receive beta's event")
            self.assertTrue((sessions / "alpha" / "meta-agent-state.json").exists(),
                            "alpha's heartbeat must be written")
            self.assertFalse((sessions / "beta" / "meta-agent-state.json").exists(),
                             "beta must NOT get alpha's heartbeat")


if __name__ == "__main__":
    unittest.main()
