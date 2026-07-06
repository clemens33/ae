"""Phase-3 Slice 7 contract: bridge harness — fake Telegram API + anchor validator.

The foundation s8-15 build on. Two deliverables, both proven here:

1. AW.TelegramTransport ABC (the one seam; FakeTelegramTransport here, the s9 urllib
   RealTelegramTransport later) + the fake's REAL-API fidelity: getUpdates ack-floor
   (at-most-once), sendMessage scripted responses with STRICT exhaustion, setMyCommands,
   and fail-loud on anything unmodeled.
2. AW.validate_bridge_fixture: the lead's binding rule made machine-checked — every
   bridge fixture expectation MUST carry an ae:NNNN source anchor to the bash daemon.

Anchors: getUpdates ae:3745-3799, sendMessage ae:3274-3304, setMyCommands ae:3310-3352.
Pure stdlib, no real token.
"""

import unittest

from harness import AW
from telegram_fake import FakeTelegramError, FakeTelegramTransport


def _msg(update_id, text="hi", from_id=42, chat_id=42, chat_type="private"):
    return {"update_id": update_id,
            "message": {"from": {"id": from_id}, "chat": {"id": chat_id, "type": chat_type}, "text": text}}


class TransportSeamTest(unittest.TestCase):
    def test_fake_implements_the_abc(self):
        self.assertIsInstance(FakeTelegramTransport(), AW.TelegramTransport)


class GetUpdatesFidelityTest(unittest.TestCase):
    def test_returns_updates_at_or_above_offset(self):
        t = FakeTelegramTransport(updates=[_msg(1), _msg(2), _msg(3)])
        self.assertEqual([u["update_id"] for u in t.get_updates(2)["result"]], [2, 3])

    def test_ack_floor_prevents_replay_of_confirmed_updates(self):
        # codex's at-most-once negative test: a LOWER offset after a higher one must NOT
        # resurrect a confirmed update. A naive stateless `update_id >= offset` filter
        # would return update 1 again on the third poll — the ack floor is what stops it.
        t = FakeTelegramTransport(updates=[_msg(1), _msg(2)])
        self.assertEqual([u["update_id"] for u in t.get_updates(1)["result"]], [1, 2], "offset 1 sees update 1")
        self.assertEqual([u["update_id"] for u in t.get_updates(2)["result"]], [2], "offset 2 confirms update 1")
        self.assertEqual([u["update_id"] for u in t.get_updates(1)["result"]], [2],
                         "a lower offset must NOT replay the confirmed update 1")

    def test_respects_limit(self):
        t = FakeTelegramTransport(updates=[_msg(i) for i in range(1, 6)])
        self.assertEqual(len(t.get_updates(1, limit=2)["result"]), 2)

    def test_records_timeout_without_blocking(self):
        # Fake models the long-poll timeout as a recorded PARAM only (no real sleep).
        t = FakeTelegramTransport(updates=[])
        t.get_updates(1, timeout=30)
        self.assertEqual(t.get_updates_calls[0]["timeout"], 30)

    def test_unmodeled_allowed_updates_fails_loud(self):
        t = FakeTelegramTransport()
        with self.assertRaises(FakeTelegramError):
            t.get_updates(1, allowed_updates=("callback_query",))

    def test_malformed_offset_fails_loud(self):
        t = FakeTelegramTransport()
        for bad in (-1, "1", 1.0, True):
            with self.subTest(offset=bad), self.assertRaises(FakeTelegramError):
                t.get_updates(bad)

    def test_bad_limit_fails_loud(self):
        # limit<=0 must fail loud, not silently return no updates (false-green).
        t = FakeTelegramTransport(updates=[_msg(1)])
        for bad in (0, -1, "10", 1.0, True):
            with self.subTest(limit=bad), self.assertRaises(FakeTelegramError):
                t.get_updates(1, limit=bad)

    def test_bad_timeout_fails_loud(self):
        t = FakeTelegramTransport()
        for bad in (-1, "0", 1.0, True):
            with self.subTest(timeout=bad), self.assertRaises(FakeTelegramError):
                t.get_updates(1, timeout=bad)

    def test_malformed_update_object_fails_loud(self):
        # A queued update without an int update_id must raise FakeTelegramError, not
        # surface incidentally as a KeyError/TypeError.
        for bad in ({"message": {}}, {"update_id": "5"}, {"update_id": True}, "notadict"):
            with self.subTest(update=bad), self.assertRaises(FakeTelegramError):
                FakeTelegramTransport(updates=[bad]).get_updates(1)


class SendMessageFidelityTest(unittest.TestCase):
    def test_returns_scripted_responses_in_order(self):
        script = [
            {"ok": False, "error_code": 429, "parameters": {"retry_after": 2}},
            {"ok": True, "result": {"message_id": 7}},
        ]
        t = FakeTelegramTransport(send_responses=list(script))
        self.assertEqual(t.send_message(42, "a"), script[0])
        self.assertEqual(t.send_message(42, "b"), script[1])
        self.assertEqual(t.sent, [{"chat_id": 42, "text": "a"}, {"chat_id": 42, "text": "b"}])

    def test_unscripted_send_fails_loud_by_default(self):
        # codex: a missing scripted 429/5xx must not let retry logic go falsely green.
        t = FakeTelegramTransport(send_responses=[{"ok": True}])
        t.send_message(42, "first")  # consumes the one scripted response
        with self.assertRaises(FakeTelegramError):
            t.send_message(42, "unscripted extra")

    def test_allow_default_ok_permits_unscripted_sends(self):
        t = FakeTelegramTransport(allow_default_ok=True)
        self.assertTrue(t.send_message(42, "x")["ok"])
        self.assertEqual(t.sent, [{"chat_id": 42, "text": "x"}])

    def test_malformed_target_or_text_fails_loud(self):
        t = FakeTelegramTransport(allow_default_ok=True)
        for chat_id, text in ((None, "hi"), ("", "hi"), (True, "hi"), (42, None), (42, ""), (42, 5)):
            with self.subTest(chat_id=chat_id, text=text), self.assertRaises(FakeTelegramError):
                t.send_message(chat_id, text)


class SetMyCommandsFidelityTest(unittest.TestCase):
    def test_returns_ok_and_records_commands(self):
        t = FakeTelegramTransport()
        cmds = [{"command": "list", "description": "Running sessions"}]
        self.assertEqual(t.set_my_commands(cmds), {"ok": True, "result": True})
        self.assertEqual(t.set_commands_calls, [cmds])

    def test_malformed_commands_payload_fails_loud(self):
        t = FakeTelegramTransport()
        for bad in ("bad", [], [{"command": "list"}], [{"command": 1, "description": "x"}], [{"description": "x"}]):
            with self.subTest(commands=bad), self.assertRaises(FakeTelegramError):
                t.set_my_commands(bad)


class BridgeFixtureAnchorTest(unittest.TestCase):
    def _valid(self):
        return {"id": "bridge.x", "expect": [
            {"kind": "send", "chat_id": 42, "text": "hi", "source": "ae:3274-3304"},
            {"kind": "offset_after", "value": 5, "source": ["ae:3745-3799", "ae:3368-3372"]},
        ]}

    def test_well_anchored_fixture_validates(self):
        self.assertEqual(AW.validate_bridge_fixture(self._valid()), [])

    def test_expectation_without_source_anchor_fails(self):
        fx = self._valid()
        del fx["expect"][0]["source"]
        errs = AW.validate_bridge_fixture(fx)
        self.assertTrue(any("source anchor" in e for e in errs), errs)

    def test_malformed_source_anchor_fails(self):
        # incl. impossible ranges (codex NIT): ae:0 (not 1-based), ae:99-12 (end<start).
        for bad in ("ae:foo", "src.py:5", "3745", "ae:", "", "ae:0", "ae:99-12"):
            with self.subTest(bad=bad):
                fx = self._valid()
                fx["expect"][0]["source"] = bad
                errs = AW.validate_bridge_fixture(fx)
                self.assertTrue(any("source anchor" in e for e in errs), f"{bad!r}: {errs}")

    def test_valid_ranges_accepted(self):
        for good in ("ae:1", "ae:3745", "ae:12-99", "ae:5-5"):
            with self.subTest(good=good):
                fx = self._valid()
                fx["expect"][0]["source"] = good
                self.assertEqual(AW.validate_bridge_fixture(fx), [], good)

    def test_missing_id_and_empty_expect_flagged(self):
        self.assertTrue(AW.validate_bridge_fixture({"expect": []}))
        self.assertTrue(any("id" in e for e in AW.validate_bridge_fixture({"expect": [{"source": "ae:1"}]})))


if __name__ == "__main__":
    unittest.main()
