"""Phase-3 Slice 15 contract: bridge tick composition (the s8-15 capstone).

TelegramBridge.tick(now) composes the whole bridge in one pass: load [telegram] config
-> (if enabled) register the command menu ONCE -> poll inbound updates and route them
(s10/s11) -> drain each running session's outbound events (s12/s13) -> persist. Inbound
offset (tg_offset) and outbound offsets (state.tsv) are SEPARATE stores, so an inbound
dispatch and an outbound forward in the SAME tick cannot corrupt each other.

Done: a multi-session fixture proves inbound dispatch + outbound forwarding happen in one
tick without offset corruption. Pure stdlib.
"""

import json
import stat
import tempfile
import unittest
import unittest.mock
from pathlib import Path

from harness import AW
from telegram_fake import FakeTelegramTransport

_HELPER = "#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\n" \
          "Path('__REC__').write_text(json.dumps({'argv':sys.argv[1:],'sender':os.environ.get('AE_SENDER_OVERRIDE','')}))\n"


def _agent(ref):
    return AW.DiscoveredAgent(slot="main", ref=ref, session_id="")


def _line(**ev):
    return json.dumps(ev, separators=(",", ":")) + "\n"


class BridgeFixture:
    def __init__(self, root, *, updates=None):
        self.ae_home = Path(root)
        (self.ae_home / "sessions").mkdir(parents=True)
        self.steward_rec = self.ae_home / "steward-call.json"
        # token file (owner-only) + [telegram] config
        tok = self.ae_home / "tok"
        tok.write_text("123456:secret-token-value")
        tok.chmod(0o600)
        (self.ae_home / "config").write_text(
            "[telegram]\nenabled = true\ntoken_file = " + str(tok) + "\n"
            "chat_id = 42\nallowed_user_ids = 7\ninclude = send,ask,chat\n")
        self._session("steward", "stew0001", "codex:lead", meta_agent=True, helper=True)
        self._session("work", "work0002", "optimal:cw")
        self.transport = FakeTelegramTransport(updates=updates or [], allow_default_ok=True)
        self.recorder = AW.EffectRecorder()
        self.bridge = AW.TelegramBridge(
            self.ae_home, self.transport,
            discover=lambda: self._discover(),
            offset_store=AW.OffsetStore(self.ae_home / "tg_offset"),
            outbound_state=AW.OutboundState(self.ae_home / "state.tsv"),
            current_target=AW.CurrentTarget(self.ae_home / "current_target"),
            recorder=self.recorder,
        )

    def _session(self, name, sid, main_ref, *, meta_agent=False, helper=False):
        d = self.ae_home / "sessions" / name
        d.mkdir(parents=True, exist_ok=True)
        meta = f"session={name}\nsession_id={sid}\nagent.main={main_ref}:{sid}\n"
        if meta_agent:
            meta += "meta_agent=true\n"
        (d / "meta").write_text(meta)
        (d / "events.jsonl").write_text("")
        if helper:
            h = d / "send"
            h.write_text(_HELPER.replace("__REC__", str(self.steward_rec)))
            h.chmod(h.stat().st_mode | stat.S_IEXEC)

    def _discover(self):
        return [
            AW.DiscoveredSession(name="steward", session_id="stew0001", work_dir="",
                                 tmux_server="", running=True, agents=[_agent("codex:lead")]),
            AW.DiscoveredSession(name="work", session_id="work0002", work_dir="",
                                 tmux_server="", running=True, agents=[_agent("optimal:cw")]),
        ]

    def steward_call(self):
        return json.loads(self.steward_rec.read_text()) if self.steward_rec.exists() else None

    def append_work_event(self, **ev):
        with (self.ae_home / "sessions" / "work" / "events.jsonl").open("a") as fh:
            fh.write(_line(**ev))


def _update(uid, text, *, from_id=7, chat_id=42):
    return {"update_id": uid, "message": {"from": {"id": from_id},
            "chat": {"id": chat_id, "type": "private"}, "text": text}}


class BridgeTickTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def test_inbound_and_outbound_in_one_tick(self):
        # Tick 1 establishes outbound EOF (no history replay) + no inbound yet.
        fx = BridgeFixture(self._tmp.name)
        fx.bridge.tick()
        self.assertIsNone(fx.steward_call(), "no inbound dispatch on the init tick")
        base_sends = len(fx.transport.sent)

        # Now a NEW outbound event on `work` AND an inbound message arrive; ONE tick
        # must dispatch the inbound to the steward AND forward the outbound event.
        fx.append_work_event(action="send", actor="optimal:cw", summary="progress update")
        fx.transport._updates.append(_update(500, "hello steward"))
        fx.bridge.tick()

        # inbound: routed to the steward's main agent via its send helper.
        call = fx.steward_call()
        self.assertIsNotNone(call, "the inbound message reached the steward")
        self.assertEqual(call["argv"], ["codex:lead", "hello steward"])
        self.assertEqual(call["sender"], "telegram:7")

        # outbound: the new work event was forwarded (a telegram.send effect for it).
        sends = [e for e in fx.recorder.as_list() if e["kind"] == "telegram.send"]
        self.assertTrue(any("progress update" in e["text"] for e in sends), "outbound event forwarded")
        # and both happened in the SAME tick (transport.sent grew beyond the init baseline).
        self.assertGreater(len(fx.transport.sent), base_sends)

    def test_registers_command_menu_once_across_ticks(self):
        fx = BridgeFixture(self._tmp.name)
        fx.bridge.tick()
        fx.bridge.tick()
        self.assertEqual(len(fx.transport.set_commands_calls), 1, "menu registered exactly once")

    def test_register_retries_until_first_success(self):
        # Long-lived tick recovers from a transient setMyCommands failure (codex): mark
        # registered only AFTER success, then stop retrying.
        fx = BridgeFixture(self._tmp.name)
        calls = []
        real = fx.transport.set_my_commands

        def flaky(commands):
            calls.append(1)
            return {"ok": False} if len(calls) == 1 else real(commands)

        fx.transport.set_my_commands = flaky
        fx.bridge.tick()  # attempt 1 -> not ok
        fx.bridge.tick()  # attempt 2 -> ok, now registered
        fx.bridge.tick()  # already registered -> no further attempt
        self.assertEqual(len(calls), 2, "retries until the first success, then stops")

    def test_disabled_config_is_a_noop(self):
        fx = BridgeFixture(self._tmp.name)
        (fx.ae_home / "config").write_text("[telegram]\nenabled = false\n")
        fx.transport._updates.append(_update(1, "hi"))
        fx.bridge.tick()
        self.assertEqual(fx.transport.get_updates_calls, [], "a disabled bridge does not poll")
        self.assertEqual(fx.transport.set_commands_calls, [], "and registers nothing")
        # the enabled gate must skip the OUTBOUND drain too (no state.tsv is written).
        self.assertFalse((fx.ae_home / "state.tsv").exists(), "a disabled bridge does not drain/persist outbound")

    def test_outbound_drain_and_save_precede_inbound_poll(self):
        # ae:3988-4000 order: outbound offsets are persisted BEFORE inbound runs arbitrary
        # session-helper code (codex).
        fx = BridgeFixture(self._tmp.name)
        order = []
        real_save = fx.bridge._outbound_state.save
        real_get = fx.transport.get_updates
        fx.bridge._outbound_state.save = lambda st: (order.append("outbound-save"), real_save(st))[1]

        def get(offset, **kw):
            order.append("inbound-poll")
            return real_get(offset, **kw)

        fx.transport.get_updates = get
        fx.bridge.tick()
        self.assertEqual(order, ["outbound-save", "inbound-poll"])

    def test_one_bad_session_does_not_abort_the_tick(self):
        # A failing session's outbound drain must not starve the others (codex).
        fx = BridgeFixture(self._tmp.name)
        fx.bridge.tick()  # init EOF for both
        with (fx.ae_home / "sessions" / "steward" / "events.jsonl").open("a") as f:
            f.write(_line(action="send", actor="codex:lead", summary="steward event"))
        fx.append_work_event(action="send", actor="optimal:cw", summary="work event")

        real = AW.process_session_events

        def flaky(events_file, prev, name, *a, **kw):
            if name == "steward":  # the FIRST discovered session raises
                raise RuntimeError("boom in steward drain")
            return real(events_file, prev, name, *a, **kw)

        with unittest.mock.patch.object(AW, "process_session_events", flaky):
            fx.bridge.tick()  # must not abort — work still forwards + saves
        sends = [e for e in fx.recorder.as_list() if e["kind"] == "telegram.send"]
        self.assertTrue(any("work event" in e["text"] for e in sends),
                        "the healthy session forwards despite the earlier session raising")
        # the failed session keeps its previous (init-EOF) state; work's advanced.
        self.assertIn("work0002", AW.OutboundState(fx.ae_home / "state.tsv").load())

    def test_no_outbound_history_replay_on_first_tick(self):
        # Pre-existing events must NOT flood on the first tick (unseen -> EOF).
        fx = BridgeFixture(self._tmp.name)
        fx.append_work_event(action="send", actor="x:y", summary="ancient history")
        fx.bridge.tick()
        sends = [e for e in fx.recorder.as_list() if e["kind"] == "telegram.send"]
        self.assertFalse(any("ancient history" in e["text"] for e in sends), "history is not replayed")


if __name__ == "__main__":
    unittest.main()
