"""Phase-3 Slice 11b contract: agent delivery primitive (the command-execution boundary).

Where a chat command actually EXECUTES: after the resolver (s11a) confines the target,
delivery runs the session's OWN generated send/ask helper (ae:3572-3583) — never raw
tmux — so ask-tracking + event emission stay identical to a human using the helper. The
external-actor identity AE_SENDER_OVERRIDE=telegram:<from_id> is propagated; the message
is passed as a POSITIONAL arg (no eval), and only the CANONICAL alias:name reaches the
helper (a bare name is canonicalized first).

  tg_dispatch (ae:3545-3584): resolve the session ref (s11a) -> resolve the agent in it
    (s11a) -> run <ae_home>/sessions/<name>/<verb>. Returns a structured DispatchResult;
    the user-facing chat strings live in s11c (the precedence/UX slice).

Proven against a temp helper executable that records argv + AE_SENDER_OVERRIDE. Pure stdlib.
"""

import os
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
    Path("__RECORD__").write_text(json.dumps({
        "argv": sys.argv[1:],
        "sender": os.environ.get("AE_SENDER_OVERRIDE", "<unset>"),
    }))
    sys.exit(__EXIT__)
''')


def _agent(ref, slot="main"):
    return AW.DiscoveredAgent(slot=slot, ref=ref, session_id="")


def _sess(name, running=True, agents=()):
    return AW.DiscoveredSession(name=name, session_id="", work_dir="", tmux_server="",
                               running=running, agents=list(agents))


class DeliveryTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.ae_home = Path(self._tmp.name)
        self.record = self.ae_home / "call.json"

    def _helper(self, session, verb, exit_code=0):
        d = self.ae_home / "sessions" / session
        d.mkdir(parents=True, exist_ok=True)
        h = d / verb
        h.write_text(_HELPER.replace("__RECORD__", str(self.record)).replace("__EXIT__", str(exit_code)))
        h.chmod(h.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        return h

    def _call(self):
        import json
        return json.loads(self.record.read_text()) if self.record.exists() else None

    def _sessions(self, agents=(_agent("codex:lead"), _agent("optimal:cw"))):
        return [_sess("work", agents=agents)]

    def test_delivers_via_helper_under_sender_override(self):
        self._helper("work", "send")
        r = AW.tg_dispatch(self._sessions(), "work", "send", "codex:lead", "hello there",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "delivered")
        call = self._call()
        self.assertEqual(call["argv"], ["codex:lead", "hello there"])
        self.assertEqual(call["sender"], "telegram:55", "external-actor identity is propagated")

    def test_bare_agent_is_canonicalized_before_the_helper(self):
        self._helper("work", "send")
        AW.tg_dispatch(self._sessions(), "work", "send", "lead", "hi", from_id="55", ae_home=self.ae_home)
        self.assertEqual(self._call()["argv"][0], "codex:lead", "the helper gets the canonical ref, not 'lead'")

    def test_message_is_positional_no_eval(self):
        # A shell-metachar payload must reach the helper as ONE positional arg, verbatim.
        self._helper("work", "send")
        payload = "rm -rf / ; echo $(whoami) `id` && :"
        AW.tg_dispatch(self._sessions(), "work", "send", "codex:lead", payload,
                       from_id="55", ae_home=self.ae_home)
        self.assertEqual(self._call()["argv"], ["codex:lead", payload])

    def test_ask_verb_uses_the_ask_helper(self):
        self._helper("work", "ask")
        r = AW.tg_dispatch(self._sessions(), "work", "ask", "codex:lead", "q?",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "delivered")
        self.assertEqual(r.verb, "ask")

    def test_helper_nonzero_reports_failed(self):
        self._helper("work", "send", exit_code=3)
        r = AW.tg_dispatch(self._sessions(), "work", "send", "codex:lead", "x",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "failed")

    def test_missing_helper_reports_no_helper(self):
        # session resolves + agent resolves, but no `send` helper exists.
        r = AW.tg_dispatch(self._sessions(), "work", "send", "codex:lead", "x",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "no_helper")

    # ── resolver failures map to structured statuses (no dispatch) ───────
    def test_unknown_session_no_dispatch(self):
        r = AW.tg_dispatch(self._sessions(), "ghost", "send", "codex:lead", "x",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "no_session")
        self.assertIsNone(self._call())

    def test_ambiguous_session(self):
        sessions = [_sess("a"), _sess("b")]
        sessions[0].session_id = "pre1"; sessions[1].session_id = "pre2"
        r = AW.tg_dispatch(sessions, "pre", "send", "x:y", "m", from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "ambiguous_session")

    def test_no_agent_in_session(self):
        self._helper("work", "send")
        r = AW.tg_dispatch(self._sessions(), "work", "send", "ghost", "x",
                           from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "no_agent")
        self.assertIsNone(self._call(), "an unresolved agent never runs the helper")

    def test_ambiguous_agent(self):
        sessions = self._sessions(agents=(_agent("codex:lead"), _agent("optimal:lead")))
        r = AW.tg_dispatch(sessions, "work", "send", "lead", "x", from_id="55", ae_home=self.ae_home)
        self.assertEqual(r.status, "ambiguous_agent")

    def test_escape_agent_ref_never_dispatches(self):
        # The s11a confinement holds end-to-end: %pane / @other can't be delivered.
        self._helper("work", "send")
        for escape in ("%1", "@other:agent", "telegram:55"):
            with self.subTest(escape=escape):
                r = AW.tg_dispatch(self._sessions(), "work", "send", escape, "x",
                                   from_id="55", ae_home=self.ae_home)
                self.assertEqual(r.status, "no_agent")
                self.assertIsNone(self._call())

    def test_verb_whitelist_blocks_non_send_ask_even_if_the_file_exists(self):
        # Defense-in-depth (codex): the session dir has many helpers; a chat command must
        # ONLY run send/ask. Even a real state/retire/reply file must NOT execute, and a
        # path-fragment or empty verb must be rejected before any file is touched.
        for verb in ("state", "retire", "reply", "interrupt"):
            self._helper("work", verb)  # the file EXISTS + is executable
        self.record.unlink(missing_ok=True)
        for verb in ("state", "retire", "reply", "interrupt", "../work/state", "", "send ask"):
            with self.subTest(verb=verb):
                r = AW.tg_dispatch(self._sessions(), "work", verb, "codex:lead", "x",
                                   from_id="55", ae_home=self.ae_home)
                self.assertEqual(r.status, "bad_verb", f"{verb!r} must never execute")
                self.assertIsNone(self._call(), f"{verb!r} must not run any helper")

    def test_wedged_helper_times_out_to_failed(self):
        # A hung send/ask must not block the bridge poll forever (sidecar hardening).
        d = self.ae_home / "sessions" / "work"
        d.mkdir(parents=True, exist_ok=True)
        h = d / "send"
        h.write_text("#!/bin/sh\nsleep 10\n")
        h.chmod(h.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        r = AW.tg_dispatch(self._sessions(), "work", "send", "codex:lead", "x",
                           from_id="55", ae_home=self.ae_home, timeout=0.3)
        self.assertEqual(r.status, "failed")

    def test_result_preserves_verb_session_agent(self):
        self._helper("work", "send")
        r = AW.tg_dispatch(self._sessions(), "work", "send", "lead", "hi", from_id="55", ae_home=self.ae_home)
        self.assertEqual((r.session, r.agent, r.verb), ("work", "codex:lead", "send"))


if __name__ == "__main__":
    unittest.main()
