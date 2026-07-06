"""Phase-3 Slice 10 contract: inbound offset + auth.

Makes inbound updates DURABLE and EXACT-AUTH checked:
  OffsetStore (ae:3356-3372): load (validate ^[0-9]+$ else 0) + atomic save (tmp+rename).
  update_authorized (ae:3210-3230): ALL must hold — numeric from_id, private chat, EXACT
    chat_id == config.chat_id, and from_id in allowed_user_ids. Confines commands to the
    1:1 control channel; a group / accidental-add cannot drive ae.
  inbound_enabled (ae:3203-3205): at least one allow-listed id, else outbound-only.
  poll_inbound (ae:3745-3799): getUpdates(stored+1), then PER update persist the offset
    BEFORE dispatch (at-most-once, ae:3770-3775), drop non-text (ae:3780) + unauthorized
    (ae:3781-3784) updates, and hand authorized text to the injected dispatch seam (s11).

Done: a crash/restart fixture proves at-most-once (offset acked before the crash is not
replayed); unauthorized updates neither dispatch nor leak secrets. Pure stdlib.
"""

import tempfile
import unittest
from pathlib import Path

from harness import AW
from telegram_fake import FakeTelegramTransport

_CFG = AW.TelegramConfig(enabled=True, token="123456:tok", chat_id="42", allowed_user_ids="7, 8")


def _upd(update_id, *, text="hi", from_id=7, chat_id=42, chat_type="private"):
    return {"update_id": update_id,
            "message": {"from": {"id": from_id}, "chat": {"id": chat_id, "type": chat_type}, "text": text}}


class OffsetStoreTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.path = Path(self._tmp.name) / "tg_offset"

    def test_load_default_zero(self):
        self.assertEqual(AW.OffsetStore(self.path).load(), 0)

    def test_save_then_load_roundtrip(self):
        s = AW.OffsetStore(self.path)
        self.assertTrue(s.save(1234))
        self.assertEqual(AW.OffsetStore(self.path).load(), 1234)

    def test_non_numeric_loads_zero(self):
        self.path.write_text("garbage")
        self.assertEqual(AW.OffsetStore(self.path).load(), 0)

    def test_save_is_atomic_no_temp_left(self):
        AW.OffsetStore(self.path).save(9)
        leftovers = [p.name for p in self.path.parent.iterdir() if p.name != "tg_offset"]
        self.assertEqual(leftovers, [], f"atomic save must leave no temp file: {leftovers}")


class AuthTest(unittest.TestCase):
    def test_authorized_when_all_hold(self):
        self.assertTrue(AW.update_authorized("7", "42", "private", _CFG))

    def test_rejects_non_numeric_from_id(self):
        self.assertFalse(AW.update_authorized("seven", "42", "private", _CFG))

    def test_rejects_non_private_chat(self):
        self.assertFalse(AW.update_authorized("7", "42", "group", _CFG))

    def test_rejects_wrong_chat_id(self):
        self.assertFalse(AW.update_authorized("7", "999", "private", _CFG))

    def test_rejects_unlisted_sender(self):
        self.assertFalse(AW.update_authorized("9", "42", "private", _CFG))

    def test_exact_match_not_prefix(self):
        # allow-list "7" must not authorize "70" (exact match, ae:3218-3219).
        self.assertFalse(AW.update_authorized("70", "42", "private", _CFG))

    def test_inbound_enabled(self):
        self.assertTrue(AW.inbound_enabled(_CFG))
        self.assertFalse(AW.inbound_enabled(AW.TelegramConfig(enabled=True, allowed_user_ids="")))
        self.assertFalse(AW.inbound_enabled(AW.TelegramConfig(enabled=True, allowed_user_ids=" , ")))


class PollInboundTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.store = AW.OffsetStore(Path(self._tmp.name) / "tg_offset")

    def _poll(self, transport, dispatch, **kw):
        return AW.poll_inbound(transport, self.store, _CFG, dispatch=dispatch, **kw)

    def test_authorized_text_dispatched(self):
        got = []
        self._poll(FakeTelegramTransport(updates=[_upd(100)]),
                   lambda from_id, text, upd: got.append((from_id, text)))
        self.assertEqual(got, [("7", "hi")])

    def test_polls_with_stored_offset_plus_one(self):
        self.store.save(100)
        t = FakeTelegramTransport(updates=[_upd(101)])
        self._poll(t, lambda *a: None)
        self.assertEqual(t.get_updates_calls[0]["offset"], 101, "getUpdates uses stored+1 (ae:3749)")

    def test_non_text_update_dropped(self):
        got = []
        self._poll(FakeTelegramTransport(updates=[_upd(100, text="")]),
                   lambda *a: got.append(a))
        self.assertEqual(got, [], "a non-text update is dropped (ae:3780)")

    def test_unauthorized_dropped_without_secret_leak(self):
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(Path(self._tmp.name) / "d.log", secrets=[_CFG.token], recorder=rec)
        got = []
        self._poll(FakeTelegramTransport(updates=[_upd(100, from_id=999)]),
                   lambda *a: got.append(a), logger=logger)
        self.assertEqual(got, [], "an unlisted sender never dispatches")
        for e in rec.as_list():
            self.assertNotIn(_CFG.token, e.get("message", ""))

    def test_offset_persisted_before_dispatch(self):
        # The at-most-once foundation: when dispatch runs for update N, the offset N is
        # ALREADY on disk (ae:3770-3775). A process crash during dispatch therefore
        # cannot replay N. Design-agnostic w.r.t. dispatch error handling.
        seen = []

        def dispatch(from_id, text, upd):
            seen.append((upd["update_id"], self.store.load()))

        self._poll(FakeTelegramTransport(updates=[_upd(100), _upd(101)]), dispatch)
        self.assertEqual(seen, [(100, 100), (101, 101)],
                         "the offset is persisted to the update's id BEFORE its dispatch")

    def test_restart_does_not_replay_acked_updates(self):
        # After processing 100/101 the offset is 101; a fresh transport (Telegram acked
        # them) offers only >= 102 on restart, so nothing replays.
        self._poll(FakeTelegramTransport(updates=[_upd(100), _upd(101)]), lambda *a: None)
        self.assertEqual(self.store.load(), 101)
        got = []
        self._poll(FakeTelegramTransport(updates=[_upd(100), _upd(101)]),
                   lambda from_id, text, upd: got.append(upd["update_id"]))
        self.assertEqual(got, [], "acked updates are not replayed after restart")

    def test_dispatch_error_does_not_starve_the_batch(self):
        # codex: update 100's dispatch raises (with the token in its message); the batch
        # must still process 101, the offset must reach 101, and the error log must be
        # redacted. Proves one bad command can't starve the rest + at-most-once holds.
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(Path(self._tmp.name) / "d.log", secrets=[_CFG.token], recorder=rec)
        done = []

        def dispatch(from_id, text, upd):
            if upd["update_id"] == 100:
                raise RuntimeError(f"boom leaking {_CFG.token}")
            done.append(upd["update_id"])

        self._poll(FakeTelegramTransport(updates=[_upd(100), _upd(101)]), dispatch, logger=logger)
        self.assertEqual(done, [101], "101 is still dispatched after 100's handler raised")
        self.assertEqual(self.store.load(), 101, "offset advances past the failed command")
        warns = [e for e in rec.as_list() if e["kind"] == "log.write"]
        self.assertTrue(any("dispatch error" in e["message"] for e in warns), warns)
        for e in warns:
            self.assertNotIn(_CFG.token, e["message"], "a token in the exception must be redacted")

    def test_malformed_result_is_dropped_not_iterated(self):
        # A raw transport dict whose `result` is missing / a str / a dict must be dropped,
        # not iterated as characters/keys (codex NIT).
        got = []
        for bad in ({"ok": True, "result": "oops"}, {"ok": True, "result": {"k": "v"}}, {"ok": True}):
            with self.subTest(resp=bad):
                class _Bad(FakeTelegramTransport):
                    def get_updates(self, offset, **kw):
                        return bad
                self._poll(_Bad(), lambda *a: got.append(a))
        self.assertEqual(got, [])

    def test_get_updates_not_ok_no_dispatch(self):
        class _NotOk(FakeTelegramTransport):
            def get_updates(self, offset, **kw):
                return {"ok": False, "error_code": 409, "description": "conflict"}
        got = []
        self._poll(_NotOk(), lambda *a: got.append(a))
        self.assertEqual(got, [])

    def test_disabled_inbound_does_not_poll(self):
        cfg = AW.TelegramConfig(enabled=True, chat_id="42", allowed_user_ids="")
        t = FakeTelegramTransport(updates=[_upd(100)])
        AW.poll_inbound(t, self.store, cfg, dispatch=lambda *a: None)
        self.assertEqual(t.get_updates_calls, [], "no allow-list -> outbound-only, no getUpdates")


if __name__ == "__main__":
    unittest.main()
