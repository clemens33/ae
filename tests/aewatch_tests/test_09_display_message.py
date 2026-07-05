"""Phase-2 Slice 1 contract: the tmux.display_message effect kind.

The bash watchdog emits user-visible `tmux display-message -d 10000 "<text>"`
alerts for dead panes, persistent throttling, max nudges, missing panes, and the
meta-agent wedge. Phase 1's EFFECT_KINDS could not express that — mapping it to
log.write would hide a real tmux side effect. This slice adds the kind so the
dual-run oracle can compare it faithfully. Not an ae edit — a sidecar
contract/test correction.

Pure stdlib.
"""

import json
import unittest

from harness import AW, FakeTmux


class DisplayMessageEffectTest(unittest.TestCase):
    def test_display_message_is_a_known_effect_kind(self):
        self.assertIn("tmux.display_message", AW.EFFECT_KINDS)

    def test_make_effect_accepts_display_message(self):
        effect = AW.make_effect(
            "tmux.display_message",
            text="[ae watchdog] codex:w is DEAD — process dropped to shell",
            duration_ms=10000,
        )
        self.assertEqual(effect["kind"], "tmux.display_message")
        self.assertEqual(effect["text"], "[ae watchdog] codex:w is DEAD — process dropped to shell")
        self.assertEqual(effect["duration_ms"], 10000)
        self.assertEqual(json.loads(json.dumps(effect)), effect)  # serializable

    def test_faketmux_display_message_records_a_mutation_effect(self):
        rec = AW.EffectRecorder()
        tmux = FakeTmux(rec, {})
        # reads record nothing; the display-message is a user-visible MUTATION.
        tmux.list_sessions("")
        tmux.display_message("[ae watchdog] codex:w is DEAD", duration_ms=10000)
        effects = rec.as_list()
        self.assertEqual([e["kind"] for e in effects], ["tmux.display_message"])
        self.assertEqual(effects[0]["text"], "[ae watchdog] codex:w is DEAD")
        self.assertEqual(effects[0]["duration_ms"], 10000)

    def test_display_message_is_on_the_tmuxclient_interface(self):
        # the watchdog cycle drives display-message through the TmuxClient seam,
        # so both the fake and the future real subprocess client implement it.
        self.assertTrue(hasattr(AW.TmuxClient, "display_message"))

    def test_unknown_effect_kind_still_rejected(self):
        # the correction must NOT weaken unknown-kind rejection (control).
        with self.assertRaises(ValueError):
            AW.make_effect("tmux.teleport", target="%1")


if __name__ == "__main__":
    unittest.main()
