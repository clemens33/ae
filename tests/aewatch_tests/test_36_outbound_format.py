"""Phase-3 Slice 12 contract: outbound formatter + include/exclude filters.

The outbound half of the bridge — turning an ae event into the chat line the human sees,
and deciding which events forward at all.

  format_event (ae:3261-3275): "[<session>] <action>  <actor>[ → <target>]" then, if a
    summary, a newline + UTF-8-safe truncation. This header is EXACTLY what s11c's reply
    parser reads back, so the round-trip stays consistent.
  event_action_allowed (ae:3232-3247): forward only actions in `include` and not in
    `exclude` (comma/space lists; default include ae:3135).
  truncate_text (ae:3250-3257): char-based (codepoint) truncation -> never splits a
    multi-byte char, appends "…(truncated)".
  forward_event: filter -> format -> send via the injected seam + record a telegram.send
    effect.

Pure stdlib.
"""

import unittest

from harness import AW

_DEFAULT_INCLUDE = "send,ask,review,reply,done,alert,throttled,chat"  # ae:3135


def _cfg(include=None, exclude="", chat_id="42"):
    kw = {"enabled": True, "chat_id": chat_id, "exclude": exclude}
    if include is not None:
        kw["include"] = include
    return AW.TelegramConfig(**kw)


class FormatEventTest(unittest.TestCase):
    def test_header_with_target_and_summary(self):
        ev = {"action": "send", "actor": "claude:lead", "target": "codex:cw", "summary": "do the thing"}
        self.assertEqual(AW.format_event("work", ev), "[work] send  claude:lead → codex:cw\ndo the thing")

    def test_header_without_target(self):
        ev = {"action": "alert", "actor": "watchdog", "summary": "agent is dead"}
        self.assertEqual(AW.format_event("work", ev), "[work] alert  watchdog\nagent is dead")

    def test_header_without_summary(self):
        ev = {"action": "done", "actor": "optimal:cw"}
        self.assertEqual(AW.format_event("work", ev), "[work] done  optimal:cw")

    def test_header_matches_the_reply_parser(self):
        # The forwarded header must round-trip through s11c's _parse_reply_target.
        ev = {"action": "nudge", "actor": "codex:lead", "summary": "x"}
        line = AW.format_event("work", ev)
        self.assertTrue(line.startswith("[work] nudge  codex:lead"))


class TruncateTest(unittest.TestCase):
    def test_under_limit_unchanged(self):
        self.assertEqual(AW.truncate_text("hello", 3500), "hello")

    def test_over_limit_truncated(self):
        self.assertEqual(AW.truncate_text("a" * 10, 4), "aaaa…(truncated)")

    def test_utf8_safe_never_splits_a_multibyte_char(self):
        # 5 emoji (each 1 codepoint / 4 UTF-8 bytes); truncating to 3 codepoints must keep
        # 3 WHOLE emoji, never a half-byte sequence.
        text = "🙂" * 5
        out = AW.truncate_text(text, 3)
        self.assertEqual(out, "🙂🙂🙂…(truncated)")
        out.encode("utf-8")  # must be valid UTF-8 (no exception)


class ActionFilterTest(unittest.TestCase):
    def test_default_include_forwards_common_actions(self):
        cfg = _cfg()  # default include
        for action in ("send", "ask", "chat", "alert"):
            self.assertTrue(AW.event_action_allowed(action, cfg.include, cfg.exclude), action)

    def test_action_not_in_include_is_filtered(self):
        self.assertFalse(AW.event_action_allowed("recover", "send,ask", ""))

    def test_exclude_overrides_include(self):
        self.assertFalse(AW.event_action_allowed("alert", _DEFAULT_INCLUDE, "alert"))

    def test_config_defaults_include_to_ae_default(self):
        self.assertEqual(_cfg().include, _DEFAULT_INCLUDE)

    def test_chat_excluded_when_not_in_include(self):
        # Done: an explicit include without `chat` does not forward chat (ae docs).
        self.assertFalse(AW.event_action_allowed("chat", "send,ask", ""))


class ForwardEventTest(unittest.TestCase):
    def _forward(self, name, event, cfg, recorder=None):
        self.sent = []
        return AW.forward_event(name, event, cfg, send=lambda cid, text: self.sent.append((cid, text)),
                                recorder=recorder)

    def test_allowed_event_sends_and_records_effect(self):
        rec = AW.EffectRecorder()
        ev = {"action": "send", "actor": "a:b", "summary": "hi"}
        self._forward("work", ev, _cfg(), recorder=rec)
        self.assertEqual(self.sent, [("42", "[work] send  a:b\nhi")])
        kinds = [e["kind"] for e in rec.as_list()]
        self.assertIn("telegram.send", kinds)

    def test_filtered_event_does_not_send_or_record(self):
        rec = AW.EffectRecorder()
        ev = {"action": "recover", "actor": "human", "summary": "x"}
        self._forward("work", ev, _cfg(include="send,ask"), recorder=rec)
        self.assertEqual(self.sent, [], "a filtered action must not forward")
        self.assertEqual(rec.as_list(), [], "and records no telegram.send effect")

    def test_chat_event_forwards_when_included(self):
        ev = {"action": "chat", "actor": "human", "summary": "hello from telegram"}
        self._forward("work", ev, _cfg())
        self.assertEqual(len(self.sent), 1)
        self.assertIn("chat", self.sent[0][1])

    def test_returns_send_result_and_records_one_effect(self):
        # s12/s13 split (codex): forward once, record the ATTEMPT, return the raw send
        # result (incl. a 429 with retry_after) — retry belongs to s13.
        rec = AW.EffectRecorder()
        fail = {"ok": False, "error_code": 429, "parameters": {"retry_after": 5}}
        ev = {"action": "send", "actor": "a:b", "summary": "hi"}
        result = AW.forward_event("work", ev, _cfg(), send=lambda cid, text: fail, recorder=rec)
        self.assertEqual(result, fail, "the raw send result is returned for s13 to inspect")
        self.assertEqual([e["kind"] for e in rec.as_list()], ["telegram.send"], "exactly one attempt effect")

    def test_raising_send_propagates_with_no_phantom_effect(self):
        # A raising seam is a broken transport, not a delivery outcome (codex): propagate,
        # and record NO effect (the record happens only after send returns).
        rec = AW.EffectRecorder()
        ev = {"action": "send", "actor": "a:b", "summary": "hi"}

        def boom(cid, text):
            raise RuntimeError("seam broken")

        with self.assertRaises(RuntimeError):
            AW.forward_event("work", ev, _cfg(), send=boom, recorder=rec)
        self.assertEqual(rec.as_list(), [], "no phantom telegram.send when the seam explodes")


class FilterTokenizationTest(unittest.TestCase):
    def test_mixed_comma_and_space_separators(self):
        # Bash IFS=', ' splits on BOTH comma and space (ae:3235/3242).
        include, exclude = "send, ask  chat", "alert throttled"
        self.assertTrue(AW.event_action_allowed("ask", include, exclude))
        self.assertTrue(AW.event_action_allowed("chat", include, exclude))
        self.assertFalse(AW.event_action_allowed("alert", include, exclude))
        self.assertFalse(AW.event_action_allowed("throttled", include, exclude))
        self.assertFalse(AW.event_action_allowed("done", include, exclude))


if __name__ == "__main__":
    unittest.main()
