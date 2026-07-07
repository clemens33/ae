"""Phase-2 Slice 2 contract: the multi-tick fixture harness.

The watchdog is a per-cycle state machine (nudge counts, quiet baselines, throttle
streaks, sweep latches), so a fixture must be able to model N cycles in one run —
and effects recorded in one tick must MUTATE the fake state so the next tick reads
them (an event appended by tick 1 is visible to tick 2). Without that, two
implementations could emit matching effects while reading divergent state.

This slice builds that harness scaffolding (a fake tick clock, per-tick pane
state, effect-applying event append) plus optional-field validation for `ticks`
and `expect.final_*`. The watchdog cycle itself lands in later slices.

Pure stdlib.
"""

import copy
import unittest
import tempfile
from pathlib import Path

from harness import AW, MultiTickEnv, load_ticks


def _two_tick_fixture():
    return {
        "id": "harness.multitick.smoke",
        "description": "two-tick smoke: an event appended in tick 1 is visible in tick 2",
        "tags": ["harness"],
        "time": {"now": "2026-07-05T07:00:00Z", "epoch": 1783234800},
        "config": {"ae_home": "$TMP/ae", "ini": ""},
        "sessions": [
            {
                "name": "work",
                "tmux_server": "",
                "meta": {"session": "work", "session_id": "s1", "tmux_server": "", "agent.main": "codex:lead:uuid"},
                "events": [],
                "panes": [{"pane_id": "%1", "agent": "codex:lead", "current_command": "codex", "capture": ""}],
                "tmux_options": {"%1": {"@ae_agent": "codex:lead"}},
            }
        ],
        "telegram": {"enabled": False, "offset": 0, "state_tsv": ""},
        "source": ["aewatch:1"],
        "ticks": [
            {"epoch": 1000, "now": "2026-07-05T07:00:00Z", "captures": {"%1": "tick-1 pane text"}},
            {"epoch": 1060, "now": "2026-07-05T07:01:00Z", "captures": {"%1": "tick-2 pane text"}},
        ],
        "expect": {"effects": [], "final_events": {}, "final_tmux_options": {}, "final_files": {}},
    }


class MultiTickHarnessTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_two_ticks_event_effects_feed_forward(self):
        env = MultiTickEnv(_two_tick_fixture(), self.root)

        # ── tick 1 ──
        env.start_tick(0)
        self.assertEqual(env.clock.epoch(), 1000)
        self.assertEqual(env.tmux.capture_pane("%1"), "tick-1 pane text")
        self.assertEqual(env.read_events("work"), [])
        env.append_event("work", {"ts": "2026-07-05T07:00:30Z", "actor": "codex:lead", "action": "state", "summary": "working"})

        # ── tick 2 ── the appended event MUST be visible now (effects feed forward)
        env.start_tick(1)
        self.assertEqual(env.clock.epoch(), 1060)
        self.assertEqual(env.tmux.capture_pane("%1"), "tick-2 pane text")
        events = env.read_events("work")
        self.assertEqual([e["summary"] for e in events], ["working"])

        # the event.append effect was recorded (ordered oracle)
        self.assertEqual([e["kind"] for e in env.recorder.as_list()], ["event.append"])

    def test_ticks_absent_derives_single_tick_from_time(self):
        fx = _two_tick_fixture()
        del fx["ticks"]
        ticks = load_ticks(fx)
        self.assertEqual(len(ticks), 1)
        self.assertEqual(ticks[0]["epoch"], 1783234800)
        self.assertEqual(ticks[0]["now"], "2026-07-05T07:00:00Z")

    def test_explicit_empty_ticks_is_not_silently_replaced(self):
        # codex: only an ABSENT `ticks` key derives a tick from `time`. An explicit
        # `ticks: []` must be honored as-is (empty), not masked by a synthetic tick
        # — masking would hide a bad contract from the oracle.
        self.assertEqual(load_ticks({"ticks": [], "time": {"epoch": 1, "now": "x"}}), [])
        self.assertEqual(len(load_ticks({"time": {"epoch": 1, "now": "x"}})), 1)

    def test_per_tick_pane_capture_advances(self):
        env = MultiTickEnv(_two_tick_fixture(), self.root)
        env.start_tick(0)
        self.assertEqual(env.tmux.capture_pane("%1"), "tick-1 pane text")
        env.start_tick(1)
        self.assertEqual(env.tmux.capture_pane("%1"), "tick-2 pane text")

    def test_append_event_copies_the_event(self):
        # codex: the recorded effect and events.jsonl must snapshot the event, so a
        # caller mutating the original dict later cannot retroactively change them.
        env = MultiTickEnv(_two_tick_fixture(), self.root)
        env.start_tick(0)
        ev = {"ts": "2026-07-05T07:00:30Z", "actor": "codex:lead", "action": "state", "summary": "working"}
        env.append_event("work", ev)
        ev["summary"] = "MUTATED-AFTER-APPEND"
        self.assertEqual(env.recorder.as_list()[0]["event"]["summary"], "working")
        env.start_tick(1)
        self.assertEqual(env.read_events("work")[0]["summary"], "working")

    def test_validator_tick_shape_checks(self):
        # If a fixture carries `ticks`, the validator checks their shape so a
        # syntactically-valid-but-wrong fixture cannot crash the oracle later:
        # ticks must be a list, each entry an object, and epoch numeric (codex).
        def tick_errs(ticks):
            fx = copy.deepcopy(_two_tick_fixture())
            fx["id"] = "session.discovery.multitick-probe"
            fx["ticks"] = ticks
            return [e for e in AW.validate_contracts({"schema_version": 1, "fixtures": [fx]})
                    if "tick" in e.lower() or "epoch" in e]

        self.assertEqual(tick_errs([{"epoch": 1000, "now": "x"}]), [])          # well-formed
        self.assertTrue(tick_errs({"epoch": 1000}))                            # not a list
        self.assertTrue(tick_errs([]))                                         # empty ticks list (codex)
        self.assertTrue(tick_errs(["not an object"]))                          # non-object entry
        self.assertTrue(tick_errs([{"now": "x"}]))                             # missing epoch
        self.assertTrue(tick_errs([{"epoch": "1000"}]))                        # non-numeric epoch
        self.assertTrue(tick_errs([{"epoch": True}]))                          # bool epoch (int subclass)


if __name__ == "__main__":
    unittest.main()
