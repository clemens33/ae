"""Phase-3 Slice 11c contract: command routing precedence (confine -> execute -> ROUTE).

The user-visible routing chain that composes on the resolver (s11a) + delivery (s11b).
Precedence — the ORDERING is behavior (ae:3485-3517 + the poll-level reply decision
ae:3792-3797):
    slash command  >  non-slash reply  >  @session:agent  >  /use sticky  >  steward default
A /help / /list / unknown-command / help text are the fallbacks.

  CurrentTarget (ae:3618 TG_TARGET_FILE): the sticky /use target "<session>\\t<agent>".
  find_steward (ae:3717-3746): running meta-agent for auto-default — 'steward' > 'hub' >
    any running meta-agent.
  route_message: decide the target by precedence, deliver via tg_dispatch (s11b), and
    send every user-facing reply through the injected `reply` seam (the outbound
    sendMessage, wired in s16). DispatchResult status -> a stable chat message.

Pure stdlib; delivery runs a temp send helper.
"""

import stat
import tempfile
import textwrap
import unittest
from pathlib import Path

from harness import AW

_HELPER = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, os, sys
    from pathlib import Path
    Path("__RECORD__").write_text(json.dumps({"argv": sys.argv[1:],
        "sender": os.environ.get("AE_SENDER_OVERRIDE", "")}))
''')


def _agent(ref, slot="main"):
    return AW.DiscoveredAgent(slot=slot, ref=ref, session_id="")


def _sess(name, running=True, agents=None):
    return AW.DiscoveredSession(name=name, session_id="", work_dir="", tmux_server="",
                               running=running, agents=list(agents or [_agent("codex:lead")]))


class Fixture:
    def __init__(self, root):
        self.ae_home = Path(root)
        self.record = self.ae_home / "call.json"
        self.replies = []
        self.current_target = AW.CurrentTarget(self.ae_home / "current_target")

    def reply(self, text):
        self.replies.append(text)

    def helper(self, session, verb="send"):
        d = self.ae_home / "sessions" / session
        d.mkdir(parents=True, exist_ok=True)
        h = d / verb
        h.write_text(_HELPER.replace("__RECORD__", str(self.record)))
        h.chmod(h.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    def meta(self, session, **kv):
        d = self.ae_home / "sessions" / session
        d.mkdir(parents=True, exist_ok=True)
        (d / "meta").write_text("".join(f"{k}={v}\n" for k, v in kv.items()))

    def call(self):
        import json
        return json.loads(self.record.read_text()) if self.record.exists() else None

    def route(self, text, *, sessions, reply_to=None):
        update = {"message": {"text": text}}
        if reply_to is not None:
            update["message"]["reply_to_message"] = {"text": reply_to}
        AW.route_message("55", text, update, sessions=sessions, ae_home=self.ae_home,
                         current_target=self.current_target, reply=self.reply)


class CurrentTargetTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.ct = AW.CurrentTarget(Path(self._tmp.name) / "ct")

    def test_load_empty_is_none(self):
        self.assertIsNone(self.ct.load())

    def test_save_load_clear(self):
        self.ct.save("work", "codex:lead")
        self.assertEqual(self.ct.load(), ("work", "codex:lead"))
        self.ct.clear()
        self.assertIsNone(self.ct.load())

    def test_corrupt_is_none(self):
        (Path(self._tmp.name) / "ct").write_text("no-tab-here")
        self.assertIsNone(self.ct.load())

    def test_save_is_owner_only(self):
        import stat as _stat
        self.ct.save("work", "codex:lead")
        mode = (Path(self._tmp.name) / "ct").stat().st_mode & 0o777
        self.assertEqual(mode, 0o600, f"the routing target must be owner-only, got {oct(mode)}")


class StewardTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fx = Fixture(self._tmp.name)

    def test_prefers_steward_then_hub_then_any(self):
        for name in ("hub", "steward", "other"):
            self.fx.meta(name, meta_agent="true")
        sessions = [_sess("hub"), _sess("steward"), _sess("other")]
        self.assertEqual(AW.find_steward(sessions, self.fx.ae_home)[0], "steward")

    def test_hub_when_no_steward(self):
        self.fx.meta("hub", meta_agent="true")
        self.fx.meta("plain")  # not a meta-agent
        self.assertEqual(AW.find_steward([_sess("hub"), _sess("plain")], self.fx.ae_home)[0], "hub")

    def test_ignores_non_meta_agent_and_stopped(self):
        self.fx.meta("work", meta_agent="true")
        # running but NOT meta_agent, and a stopped meta_agent -> no steward
        self.assertIsNone(AW.find_steward([_sess("plain")], self.fx.ae_home))
        self.fx.meta("dead", meta_agent="true")
        self.assertIsNone(AW.find_steward([_sess("dead", running=False)], self.fx.ae_home))


class PrecedenceTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fx = Fixture(self._tmp.name)
        self.fx.helper("work")
        self.fx.helper("steward")
        self.fx.meta("steward", meta_agent="true")
        self.sessions = [_sess("work"), _sess("steward")]

    def test_slash_command_beats_reply(self):
        # A /list sent AS a reply is still a command (slash > reply).
        self.fx.route("/list", sessions=self.sessions, reply_to="[work] nudge  codex:lead")
        self.assertIsNone(self.fx.call(), "a slash command must not deliver as a reply")
        self.assertTrue(any("session" in r.lower() for r in self.fx.replies))

    def test_non_slash_reply_routes_to_forwarded_agent(self):
        self.fx.route("my answer", sessions=self.sessions, reply_to="[work] nudge  codex:lead")
        self.assertEqual(self.fx.call()["argv"], ["codex:lead", "my answer"])

    def test_at_prefix_routes_directly(self):
        self.fx.route("@work:codex:lead do the thing", sessions=self.sessions)
        self.assertEqual(self.fx.call()["argv"], ["codex:lead", "do the thing"])

    def test_bare_uses_sticky_over_steward(self):
        self.fx.current_target.save("work", "codex:lead")
        self.fx.route("hello", sessions=self.sessions)
        call = self.fx.call()
        self.assertEqual(call["argv"], ["codex:lead", "hello"])
        self.assertEqual(call["sender"], "telegram:55")

    def test_bare_defaults_to_steward_when_no_sticky(self):
        self.fx.route("hello steward", sessions=self.sessions)
        # delivered to the steward session's main agent
        self.assertEqual(self.fx.call()["argv"], ["codex:lead", "hello steward"])

    def test_bare_no_sticky_no_steward_guides(self):
        self.fx.route("hello", sessions=[_sess("work")])  # no steward, no sticky
        self.assertIsNone(self.fx.call())
        self.assertTrue(self.fx.replies, "the user is guided, not silently dropped")


class SessionCommandTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fx = Fixture(self._tmp.name)
        self.fx.helper("work", "send")
        self.fx.helper("work", "ask")
        self.sessions = [_sess("work")]

    def test_session_send_delivers(self):
        self.fx.route("/session work send codex:lead hello there", sessions=self.sessions)
        self.assertEqual(self.fx.call()["argv"], ["codex:lead", "hello there"])

    def test_session_ask_uses_ask_helper(self):
        self.fx.route("/session work ask codex:lead a question?", sessions=self.sessions)
        self.assertEqual(self.fx.call()["argv"], ["codex:lead", "a question?"])
        self.assertTrue(any("delivered" in r for r in self.fx.replies))

    def test_session_rejected_verb_guides_and_never_dispatches(self):
        # caller-side verb validation: /session with retire/state must NOT execute even
        # though s11b would also block it at the boundary (codex).
        for verb in ("retire", "state", "review"):
            with self.subTest(verb=verb):
                self.fx.replies.clear()
                self.fx.route(f"/session work {verb} codex:lead x", sessions=self.sessions)
                self.assertIsNone(self.fx.call(), f"{verb!r} must never dispatch")
                self.assertTrue(any("verb" in r.lower() for r in self.fx.replies))

    def test_session_usage_on_missing_args(self):
        self.fx.route("/session work send", sessions=self.sessions)
        self.assertIsNone(self.fx.call())
        self.assertTrue(any("Usage" in r for r in self.fx.replies))


class ReplyFailSafeTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fx = Fixture(self._tmp.name)
        self.fx.helper("work", "send")
        self.fx.helper("steward", "send")
        self.fx.meta("steward", meta_agent="true")
        self.sessions = [_sess("work"), _sess("steward")]

    def test_malformed_reply_header_fails_safe_no_fallback(self):
        # A forged/broken reply (unparseable header) must GUIDE, not blind-dispatch AND
        # not fall back to a sticky / steward target (codex — security-relevant). Cover
        # BOTH parse guards: no bracket, and a bracketed-but-too-few-tokens / empty header.
        self.fx.current_target.save("work", "codex:lead")  # a sticky exists — must NOT be used
        for bad in ("this is not a [header]", "[work]", "[work] onlyaction", "[] send actor"):
            with self.subTest(header=bad):
                self.fx.replies.clear()
                self.fx.record.unlink(missing_ok=True)
                self.fx.route("my answer", sessions=self.sessions, reply_to=bad)
                self.assertIsNone(self.fx.call(), f"{bad!r} must NOT dispatch anywhere")
                self.assertTrue(any("reply" in r.lower() for r in self.fx.replies), self.fx.replies)


class PrecedenceContractAnchorTest(unittest.TestCase):
    def test_precedence_invariants_carry_source_anchors(self):
        # The precedence ORDERING is user-visible bridge behavior; the ae:NNNN obligation
        # is made EXECUTABLE here via the s7 validator (s15's driver replaces/extends this
        # with fixture-driven coverage). Every invariant must cite the bash daemon source.
        contract = {
            "id": "bridge.precedence", "expect": [
                {"rule": "slash beats reply", "source": ["ae:3485-3517", "ae:3792-3797"]},
                {"rule": "non-slash reply routes to the forwarded agent", "source": "ae:3591-3600"},
                {"rule": "@session:agent direct", "source": "ae:3628-3641"},
                {"rule": "/use sticky over steward default", "source": "ae:3648-3711"},
                {"rule": "steward auto-default", "source": "ae:3717-3746"},
            ],
        }
        self.assertEqual(AW.validate_bridge_fixture(contract), [], "the precedence contract must be fully anchored")

        # And the validator actually bites: drop one anchor -> failure.
        contract["expect"][0].pop("source")
        self.assertTrue(AW.validate_bridge_fixture(contract))


class UseCommandTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.fx = Fixture(self._tmp.name)
        self.sessions = [_sess("work", agents=[_agent("codex:lead")])]

    def test_use_sets_sticky(self):
        self.fx.route("/use work codex:lead", sessions=self.sessions)
        self.assertEqual(self.fx.current_target.load(), ("work", "codex:lead"))

    def test_use_clear(self):
        self.fx.current_target.save("work", "codex:lead")
        self.fx.route("/use clear", sessions=self.sessions)
        self.assertIsNone(self.fx.current_target.load())

    def test_use_rejects_unresolvable_target(self):
        self.fx.route("/use ghost codex:lead", sessions=self.sessions)
        self.assertIsNone(self.fx.current_target.load(), "an unresolved /use must not set a target")
        self.assertTrue(any("ghost" in r for r in self.fx.replies))

    def test_unknown_command_help(self):
        self.fx.route("/frobnicate", sessions=self.sessions)
        self.assertTrue(any("Unknown command" in r or "help" in r.lower() for r in self.fx.replies))


if __name__ == "__main__":
    unittest.main()
