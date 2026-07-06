"""Phase-3 Slice 14 contract: command menu registration (setMyCommands).

At startup, when inbound is enabled, register the bot's slash-command menu so the
commands surface in Telegram's "/" UI (ae:3306-3352). Best-effort and non-fatal: only
when inbound is enabled (no point advertising commands that would be rejected), and a
failure logs REDACTED and NEVER fails the bridge tick.

The menu (ae:3313-3316): list / use / session / help with their descriptions.
Pure stdlib.
"""

import unittest
from pathlib import Path

from harness import AW
from telegram_fake import FakeTelegramTransport

_ON = AW.TelegramConfig(enabled=True, chat_id="42", allowed_user_ids="7")
_OFF = AW.TelegramConfig(enabled=True, chat_id="42", allowed_user_ids="")  # no allow-list -> outbound-only


class CommandMenuTest(unittest.TestCase):
    def test_registers_the_four_command_menu_when_inbound_enabled(self):
        t = FakeTelegramTransport()
        self.assertTrue(AW.register_commands(t, _ON))
        self.assertEqual(len(t.set_commands_calls), 1)
        menu = t.set_commands_calls[0]
        self.assertEqual([c["command"] for c in menu], ["list", "use", "session", "help"])
        for c in menu:
            self.assertTrue(c["description"], f"{c['command']} needs a description")

    def test_does_not_register_when_inbound_disabled(self):
        t = FakeTelegramTransport()
        self.assertFalse(AW.register_commands(t, _OFF))
        self.assertEqual(t.set_commands_calls, [], "no inbound -> no menu advertised")

    def test_no_menu_when_allowlist_is_whitespace_or_comma_only(self):
        # Guard the menu gate directly (codex NIT): a whitespace/comma-only allow-list is
        # still outbound-only, so the menu must not be advertised.
        t = FakeTelegramTransport()
        for allow in (" , ", "   ", ",", ", ,"):
            with self.subTest(allow=repr(allow)):
                cfg = AW.TelegramConfig(enabled=True, chat_id="42", allowed_user_ids=allow)
                self.assertFalse(AW.register_commands(t, cfg))
        self.assertEqual(t.set_commands_calls, [])

    def test_not_ok_response_logs_and_returns_false_without_raising(self):
        class _NotOk(FakeTelegramTransport):
            def set_my_commands(self, commands):
                return {"ok": False, "error_code": 400, "description": "bad"}
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(Path("/nonexistent/d.log"), secrets=["123:tok"], recorder=rec)
        self.assertFalse(AW.register_commands(_NotOk(), _ON, logger=logger))
        self.assertTrue(any(e["kind"] == "log.write" for e in rec.as_list()))

    def test_raising_transport_never_fails_the_tick(self):
        class _Boom(FakeTelegramTransport):
            def set_my_commands(self, commands):
                raise RuntimeError("transport broken 123:tok")
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(Path("/nonexistent/d.log"), secrets=["123:tok"], recorder=rec)
        # must NOT raise — setMyCommands can never fail the bridge tick.
        self.assertFalse(AW.register_commands(_Boom(), _ON, logger=logger))
        for e in rec.as_list():
            self.assertNotIn("123:tok", e.get("message", ""), "a token in the error must be redacted")


if __name__ == "__main__":
    unittest.main()
