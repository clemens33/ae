"""Phase-3 Slice 9 contract: RealTelegramTransport (Bot API over urllib).

The second implementation of the s7 AW.TelegramTransport ABC (the fake is the first).
Performs real Bot API calls through the stdlib — no curl, no jq:
  getUpdates    (ae:3752-3756): offset/limit/timeout/allowed_updates=["message"].
  sendMessage   (ae:3292-3294): chat_id + (possibly multi-line) text.
  setMyCommands (ae:3335-3336): commands as a JSON string.

Driven against an injected fake `urlopen` (fake HTTP, no network). A 4xx/5xx carries a
JSON body (ok=false + error_code + parameters.retry_after) which is returned so s13 can
retry; a transport failure (URLError/timeout) or invalid JSON degrades to a synthetic
{"ok": false} and logs a REDACTED error — the URL embeds bot<token>, which must never
reach a log. Pure stdlib, dummy token.
"""

import io
import json
import unittest
import urllib.error
import urllib.parse
from pathlib import Path

from harness import AW

_TOKEN = "123456:DUMMY-secret-token-value"
_BASE = "https://api.telegram.org"


class _Resp:
    """Minimal urlopen return value: a context manager exposing .read()."""

    def __init__(self, body):
        self._body = body.encode("utf-8") if isinstance(body, str) else body

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class _Opener:
    """A fake urlopen: records each request, then returns/raises a scripted result."""

    def __init__(self, result):
        self._result = result
        self.calls = []

    def __call__(self, req, timeout=None):
        self.calls.append({
            "url": req.full_url,
            "data": req.data.decode("utf-8") if req.data else "",
            "method": req.get_method(),
            "timeout": timeout,
        })
        if isinstance(self._result, Exception):
            raise self._result
        return self._result


def _http_error(code, body):
    return urllib.error.HTTPError(f"{_BASE}/x", code, "err", {}, io.BytesIO(body.encode("utf-8")))


def _api(result, **kw):
    opener = _Opener(result)
    tr = AW.RealTelegramTransport(_TOKEN, urlopen=opener, base_url=_BASE, **kw)
    return tr, opener


class GetUpdatesTest(unittest.TestCase):
    def test_url_params_and_ok_result(self):
        tr, op = _api(_Resp(json.dumps({"ok": True, "result": [{"update_id": 5}]})))
        out = tr.get_updates(5, timeout=30, limit=10)
        self.assertEqual(out, {"ok": True, "result": [{"update_id": 5}]})
        call = op.calls[0]
        self.assertEqual(call["url"], f"{_BASE}/bot{_TOKEN}/getUpdates")
        self.assertIn("offset=5", call["data"])
        self.assertIn("limit=10", call["data"])
        self.assertIn("timeout=30", call["data"])
        # allowed_updates is a JSON string param (ae:3756 allowed_updates=["message"]).
        self.assertIn("allowed_updates=%5B%22message%22%5D", call["data"])

    def test_ok_false_passthrough(self):
        tr, _ = _api(_Resp(json.dumps({"ok": False, "error_code": 400, "description": "bad"})))
        self.assertEqual(tr.get_updates(1)["error_code"], 400)


class SendMessageTest(unittest.TestCase):
    def test_url_params_and_multiline_text(self):
        tr, op = _api(_Resp(json.dumps({"ok": True, "result": {"message_id": 7}})))
        out = tr.send_message(42, "line one\nline two")
        self.assertTrue(out["ok"])
        call = op.calls[0]
        self.assertEqual(call["url"], f"{_BASE}/bot{_TOKEN}/sendMessage")
        self.assertIn("chat_id=42", call["data"])
        # multi-line text must survive urlencoding intact (%0A = newline).
        self.assertIn("text=line+one%0Aline+two", call["data"])

    def test_429_body_returned_for_retry(self):
        # 4xx is an HTTPError but Telegram's body carries retry_after — s13 needs it.
        body = json.dumps({"ok": False, "error_code": 429, "parameters": {"retry_after": 5}})
        tr, _ = _api(_http_error(429, body))
        out = tr.send_message(42, "hi")
        self.assertEqual(out["error_code"], 429)
        self.assertEqual(out["parameters"]["retry_after"], 5)

    def test_5xx_nonjson_body_synthesizes_ok_false(self):
        tr, _ = _api(_http_error(500, "<html>Bad Gateway</html>"))
        out = tr.send_message(42, "hi")
        self.assertFalse(out["ok"])
        self.assertEqual(out["error_code"], 500)


class SetMyCommandsTest(unittest.TestCase):
    def test_url_and_commands_json(self):
        tr, op = _api(_Resp(json.dumps({"ok": True, "result": True})))
        cmds = [{"command": "list", "description": "Running sessions"}]
        self.assertTrue(tr.set_my_commands(cmds)["ok"])
        call = op.calls[0]
        self.assertEqual(call["url"], f"{_BASE}/bot{_TOKEN}/setMyCommands")
        self.assertIn("commands=", call["data"])
        # the commands value is a JSON-encoded string (ae:3335-3336).
        decoded = urllib.parse.unquote_plus(call["data"])
        self.assertIn(json.dumps(cmds, separators=(",", ":")), decoded)


class TransportFailureTest(unittest.TestCase):
    def _logger(self):
        self.rec = AW.EffectRecorder()
        return AW.AewatchLogger(Path("/nonexistent/daemon.log"), secrets=[_TOKEN], recorder=self.rec)

    def test_timeout_degrades_and_redacts(self):
        tr, _ = _api(urllib.error.URLError("timed out"), logger=self._logger())
        out = tr.get_updates(1)
        self.assertFalse(out["ok"])
        for e in self.rec.as_list():
            self.assertNotIn(_TOKEN, e.get("message", ""), "the bot token must never reach a log")

    def test_invalid_json_degrades_and_redacts(self):
        tr, _ = _api(_Resp("not json at all"), logger=self._logger())
        out = tr.send_message(42, "hi")
        self.assertFalse(out["ok"])
        self.assertTrue(any(e["kind"] == "log.write" for e in self.rec.as_list()))
        for e in self.rec.as_list():
            self.assertNotIn(_TOKEN, e.get("message", ""))

    def test_url_with_token_never_logged_verbatim(self):
        # Even the request URL (which embeds bot<token>) must be redacted if logged.
        tr, _ = _api(_http_error(500, "boom"), logger=self._logger())
        tr.send_message(42, "hi")
        for e in self.rec.as_list():
            self.assertNotIn(_TOKEN, e.get("message", ""))
            self.assertNotIn(f"bot{_TOKEN}", e.get("message", ""))


class SocketTimeoutTest(unittest.TestCase):
    # codex's rule: ABC `timeout` is the Telegram long-poll param; the urllib SOCKET
    # timeout = max(request_timeout, api_timeout + long_poll_margin) so a long poll never
    # self-aborts before Telegram returns.
    def test_quick_calls_use_request_timeout(self):
        tr, op = _api(_Resp('{"ok": true, "result": true}'))
        tr.send_message(42, "hi")
        tr.set_my_commands([{"command": "list", "description": "x"}])
        self.assertEqual(op.calls[0]["timeout"], 30)
        self.assertEqual(op.calls[1]["timeout"], 30)

    def test_get_updates_socket_outlasts_long_poll(self):
        tr, op = _api(_Resp('{"ok": true, "result": []}'))
        tr.get_updates(1, timeout=0)     # max(30, 0+10) = 30
        tr.get_updates(1, timeout=30)    # max(30, 30+10) = 40
        self.assertEqual(op.calls[0]["timeout"], 30)
        self.assertEqual(op.calls[1]["timeout"], 40)

    def test_custom_timeout_and_margin(self):
        tr, op = _api(_Resp('{"ok": true, "result": []}'), timeout=5, long_poll_margin=3)
        tr.get_updates(1, timeout=10)    # max(5, 13) = 13
        tr.send_message(42, "hi")        # 5
        self.assertEqual(op.calls[0]["timeout"], 13)
        self.assertEqual(op.calls[1]["timeout"], 5)


class MalformedBodyTest(unittest.TestCase):
    def test_non_object_json_is_malformed(self):
        # A 200 body that parses to a non-object ([]/true/"str") must NOT reach bridge
        # code that calls .get — synthesize ok=false (codex NIT).
        for body in ("[]", "true", '"a string"', "42"):
            with self.subTest(body=body):
                tr, _ = _api(_Resp(body))
                self.assertEqual(tr.get_updates(1)["ok"], False)


class TransportSeamTest(unittest.TestCase):
    def test_implements_the_abc(self):
        self.assertIsInstance(AW.RealTelegramTransport(_TOKEN), AW.TelegramTransport)


if __name__ == "__main__":
    unittest.main()
