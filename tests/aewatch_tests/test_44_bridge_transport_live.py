"""s18 / B2 regression: the Telegram bridge must resolve its transport from the
CURRENT config every tick, not freeze it at daemon start.

B2 (codex, s18 green review): _run_daemon_loop_cli built RealTelegramTransport
ONCE at daemon start (transport=None when disabled / no token) while
TelegramBridge.tick reloads [telegram] config every tick. A bridge that started
disabled and was enabled later kept a frozen None transport — outbound silently
dropped, inbound crashed the tick — and a rotated token kept sending on the OLD
token.

Fix: TelegramBridge takes a transport_provider(config) resolved per tick; the
daemon's provider builds RealTelegramTransport(cfg.token) only when the CURRENT
config carries a token. These two tests pin the live behavior — enable-after-start
and token rotation. (The red is interface-shaped: the provider seam is the
deliverable, exactly like the earlier coarse-interface reds.)

Pure stdlib.
"""

import json
import tempfile
import unittest
from pathlib import Path

from harness import AW
from telegram_fake import FakeTelegramTransport


def _write_config(ae_home, *, enabled, token_file):
    (ae_home / "config").write_text(
        "[telegram]\n"
        f"enabled = {'true' if enabled else 'false'}\n"
        f"token_file = {token_file}\n"
        "chat_id = 42\nallowed_user_ids = 7\ninclude = send,ask,chat\n"
    )


def _token_file(ae_home, name, value):
    p = ae_home / name
    p.write_text(value)
    p.chmod(0o600)
    return p


def _line(**ev):
    return json.dumps(ev, separators=(",", ":")) + "\n"


class BridgeTransportLiveTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.ae_home = Path(self._tmp.name)
        work = self.ae_home / "sessions" / "work"
        work.mkdir(parents=True)
        (work / "meta").write_text("session=work\nsession_id=work0002\nagent.main=optimal:cw:work0002\n")
        (work / "events.jsonl").write_text("")

    def _discover(self):
        return [AW.DiscoveredSession(
            name="work", session_id="work0002", work_dir="", tmux_server="", running=True,
            agents=[AW.DiscoveredAgent(slot="main", ref="optimal:cw", session_id="")])]

    def _bridge(self, provider):
        return AW.TelegramBridge(
            self.ae_home, transport_provider=provider,
            discover=self._discover,
            offset_store=AW.OffsetStore(self.ae_home / "tg_offset"),
            outbound_state=AW.OutboundState(self.ae_home / "state.tsv"),
            current_target=AW.CurrentTarget(self.ae_home / "current_target"),
            recorder=AW.EffectRecorder(),
        )

    def _emit(self, **ev):
        with (self.ae_home / "sessions" / "work" / "events.jsonl").open("a") as fh:
            fh.write(_line(**ev))

    def test_disabled_at_start_then_enabled_forwards_outbound(self):
        tok = _token_file(self.ae_home, "tok1", "111:aaa")
        transport = FakeTelegramTransport(updates=[], allow_default_ok=True)
        seen = []

        def provider(config):
            seen.append(config.token)
            return transport if config.token else None

        _write_config(self.ae_home, enabled=False, token_file=str(tok))
        bridge = self._bridge(provider)

        bridge.tick()  # disabled -> early return; the provider must NOT be consulted
        self.assertEqual(seen, [], "provider resolved while disabled — tick must return before touching transport")

        _write_config(self.ae_home, enabled=True, token_file=str(tok))
        bridge.tick()  # now enabled -> establish outbound EOF (no replay)
        self._emit(action="send", actor="optimal:cw", summary="after enable")
        bridge.tick()  # forward the new event on the freshly-resolved transport

        self.assertTrue(seen and all(t == "111:aaa" for t in seen),
                        "transport must be resolved from the enabled config's token each tick")
        self.assertTrue(any("after enable" in s["text"] for s in transport.sent),
                        "outbound after enabling was never sent — the transport stayed frozen from the disabled start")

    def test_token_rotation_sends_on_the_new_transport(self):
        tok1 = _token_file(self.ae_home, "tok1", "111:aaa")
        tok2 = _token_file(self.ae_home, "tok2", "222:bbb")
        t1 = FakeTelegramTransport(updates=[], allow_default_ok=True)
        t2 = FakeTelegramTransport(updates=[], allow_default_ok=True)
        by_token = {"111:aaa": t1, "222:bbb": t2}
        provider = lambda config: by_token.get(config.token)

        _write_config(self.ae_home, enabled=True, token_file=str(tok1))
        bridge = self._bridge(provider)
        bridge.tick()  # EOF on token1
        self._emit(action="send", actor="optimal:cw", summary="event-A")
        bridge.tick()  # forward A on t1

        _write_config(self.ae_home, enabled=True, token_file=str(tok2))  # ROTATE
        self._emit(action="send", actor="optimal:cw", summary="event-B")
        bridge.tick()  # forward B on the rotated transport t2

        self.assertTrue(any("event-A" in s["text"] for s in t1.sent), "A was not sent on the token1 transport")
        self.assertFalse(any("event-B" in s["text"] for s in t1.sent), "B leaked to the OLD token1 transport after rotation")
        self.assertTrue(any("event-B" in s["text"] for s in t2.sent), "B was not sent on the rotated token2 transport")

    def test_token_rotation_reregisters_command_menu(self):
        # codex: setMyCommands is a per-BOT side channel — a rotated token is a new bot
        # that must get its OWN menu, even though the bridge "registered once" already.
        tok1 = _token_file(self.ae_home, "tok1", "111:aaa")
        tok2 = _token_file(self.ae_home, "tok2", "222:bbb")
        t1 = FakeTelegramTransport(updates=[], allow_default_ok=True)
        t2 = FakeTelegramTransport(updates=[], allow_default_ok=True)
        provider = lambda config: {"111:aaa": t1, "222:bbb": t2}.get(config.token)

        _write_config(self.ae_home, enabled=True, token_file=str(tok1))
        bridge = self._bridge(provider)
        bridge.tick()
        bridge.tick()  # registered once on token1; a stable token must NOT re-register
        self.assertEqual(len(t1.set_commands_calls), 1, "token1 menu registered exactly once")

        _write_config(self.ae_home, enabled=True, token_file=str(tok2))  # ROTATE
        bridge.tick()
        self.assertEqual(len(t2.set_commands_calls), 1, "rotated token2 bot never got its setMyCommands")

    def test_enabled_but_no_transport_is_a_clean_noop(self):
        # codex: enabled config whose provider yields no transport (e.g. token
        # unreadable) must be a clean no-op — never a crash on a None transport.
        tok = _token_file(self.ae_home, "tok1", "111:aaa")
        _write_config(self.ae_home, enabled=True, token_file=str(tok))
        bridge = self._bridge(lambda config: None)   # enabled, but no usable transport
        self._emit(action="send", actor="optimal:cw", summary="dropped")
        try:
            bridge.tick()
            bridge.tick()
        except Exception as exc:  # noqa: BLE001 — the whole point is that it must NOT raise
            self.fail(f"enabled + None transport must be a no-op, but tick raised: {exc!r}")


if __name__ == "__main__":
    unittest.main()
