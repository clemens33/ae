"""Fake Telegram transport for the bridge contract harness (phase-3 slice 7).

Models the REAL Bot API semantics the bridge depends on and FAILS LOUD on anything
unmodeled (the fakebin discipline) — a silent accept would forge API parity:

  get_updates  (ae:3745-3799): returns {"ok":true,"result":[update,...]} for update_id
    at or above an ACK FLOOR. An offset confirms every update below it (Telegram
    server-side ack), so a later LOWER offset can never replay a confirmed update —
    the at-most-once foundation s10 builds on. update_id below the floor is gone.
  send_message (ae:3274-3304): returns SCRIPTED responses in order (ok / 429 / failure).
    An unscripted send FAILS LOUD unless allow_default_ok=True — a missing scripted
    429/5xx must not let retry logic go falsely green (s13).
  set_my_commands (ae:3310-3352): returns a scripted (default ok) response.

REQUEST-SHAPE validation is strict (a bridge bug that sends limit<=0, a malformed
target/text, or a bad command menu must fail loud, not false-green); response BODIES
stay raw/scripted. The s9 RealTelegramTransport implements the SAME AW.TelegramTransport
ABC over urllib. No network, no real token. Pure stdlib.
"""

from harness import AW


class FakeTelegramError(AssertionError):
    """An unmodeled call or argument shape — surfaced loud, never silently accepted."""


def _is_int(v) -> bool:
    """A real int (bool is an int subclass in Python — exclude it)."""
    return isinstance(v, int) and not isinstance(v, bool)


class FakeTelegramTransport(AW.TelegramTransport):
    def __init__(self, *, updates=None, send_responses=None, set_commands_response=None,
                 allow_default_ok=False, default_send_response=None):
        self._updates = list(updates or [])
        self._send_script = list(send_responses or [])
        self._set_commands_response = set_commands_response or {"ok": True, "result": True}
        self._allow_default_ok = allow_default_ok
        self._default_send = default_send_response or {"ok": True, "result": {"message_id": 1}}
        self._confirmed_before = 0  # ack floor — highest offset ever seen
        # Observations for assertions.
        self.get_updates_calls = []
        self.sent = []
        self.set_commands_calls = []

    def get_updates(self, offset, *, timeout=0, limit=10, allowed_updates=("message",)):
        if tuple(allowed_updates) != ("message",):
            raise FakeTelegramError(f"unmodeled allowed_updates: {allowed_updates!r}")
        if not _is_int(offset) or offset < 0:
            raise FakeTelegramError(f"malformed offset (non-negative int): {offset!r}")
        if not _is_int(timeout) or timeout < 0:
            raise FakeTelegramError(f"malformed timeout (non-negative int): {timeout!r}")
        if not _is_int(limit) or limit < 1:
            raise FakeTelegramError(f"malformed limit (positive int): {limit!r}")
        # Ack floor: an offset confirms all updates below it; a later lower offset must
        # NOT resurrect them (real Telegram deletes confirmed updates server-side).
        self._confirmed_before = max(self._confirmed_before, offset)
        self.get_updates_calls.append({
            "offset": offset, "timeout": timeout, "limit": limit,
            "allowed_updates": tuple(allowed_updates),
        })
        floor = self._confirmed_before
        result = []
        for u in self._updates:
            if not isinstance(u, dict) or not _is_int(u.get("update_id")):
                raise FakeTelegramError(f"malformed update (needs int update_id): {u!r}")
            if u["update_id"] >= floor:
                result.append(u)
        return {"ok": True, "result": result[:limit]}

    def send_message(self, chat_id, text):
        if not (_is_int(chat_id) or (isinstance(chat_id, str) and chat_id)):
            raise FakeTelegramError(f"malformed chat_id (int or non-empty str): {chat_id!r}")
        if not isinstance(text, str) or not text:
            raise FakeTelegramError(f"malformed text (non-empty str): {text!r}")
        self.sent.append({"chat_id": chat_id, "text": text})
        if self._send_script:
            return self._send_script.pop(0)
        if self._allow_default_ok:
            return dict(self._default_send)
        raise FakeTelegramError(
            f"unscripted send #{len(self.sent)} (chat_id={chat_id!r}) — supply a scripted "
            "response or pass allow_default_ok=True"
        )

    def set_my_commands(self, commands):
        if not isinstance(commands, list) or not commands or not all(
            isinstance(c, dict) and isinstance(c.get("command"), str) and isinstance(c.get("description"), str)
            for c in commands
        ):
            raise FakeTelegramError(
                f"malformed commands (non-empty list of {{command,description}} str objects): {commands!r}"
            )
        self.set_commands_calls.append(commands)
        return dict(self._set_commands_response)
