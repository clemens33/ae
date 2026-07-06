"""Phase-3 Slice 11a contract: command resolver (the routing security boundary).

Before any chat command is dispatched, its session + agent refs are resolved against
RUNNING sessions — this is where the confinement lives (a chat command must never
escape the named session into an arbitrary pane/session).

  resolve_session_ref (ae:3387-3417): RUNNING sessions only; EXACT name wins, else a
    UNIQUE session_id prefix; 0 matches -> none, >1 -> ambiguous.
  resolve_agent_in_session (ae:3428-3450): EXACT alias:name, else a UNIQUE bare name;
    a %pane-id / @session:agent / telegram:<id> / foreign-prefix ref simply does NOT
    match a real agent.* entry, so it resolves to `none` — the confinement (codex
    BLOCKER in ae). Returns the CANONICAL alias:name, never the raw input.

Pure resolution, no dispatch (that + the precedence chain is s11b). Pure stdlib.
"""

import unittest

from harness import AW


def _agent(ref, slot="main"):
    return AW.DiscoveredAgent(slot=slot, ref=ref, session_id="")


def _sess(name, session_id="", running=True, agents=()):
    return AW.DiscoveredSession(name=name, session_id=session_id, work_dir="",
                               tmux_server="", running=running, agents=list(agents))


class SessionRefTest(unittest.TestCase):
    def _sessions(self):
        return [
            _sess("work", session_id="abc12345"),
            _sess("steward", session_id="def67890"),
            _sess("dead", session_id="abc99999", running=False),
        ]

    def test_exact_name(self):
        r = AW.resolve_session_ref(self._sessions(), "work")
        self.assertTrue(r.ok)
        self.assertEqual(r.value.name, "work")

    def test_unique_session_id_prefix(self):
        r = AW.resolve_session_ref(self._sessions(), "def6")
        self.assertTrue(r.ok)
        self.assertEqual(r.value.name, "steward")

    def test_running_only_ignores_stopped_session(self):
        # "dead" exists but is not running -> not resolvable (ae:3395 daemon_session_running).
        self.assertEqual(AW.resolve_session_ref(self._sessions(), "dead").status, "none")

    def test_exact_name_beats_prefix(self):
        sessions = [_sess("abc", session_id="xyz"), _sess("other", session_id="abcdef")]
        r = AW.resolve_session_ref(sessions, "abc")
        self.assertTrue(r.ok)
        self.assertEqual(r.value.name, "abc", "an exact NAME wins over a session_id prefix")

    def test_no_match(self):
        self.assertEqual(AW.resolve_session_ref(self._sessions(), "ghost").status, "none")

    def test_ambiguous_prefix(self):
        sessions = [_sess("a", session_id="pre111"), _sess("b", session_id="pre222")]
        self.assertEqual(AW.resolve_session_ref(sessions, "pre").status, "ambiguous")

    def test_empty_or_whitespace_ref_is_none(self):
        # A reusable boundary: an empty ref must NOT prefix-match every session
        # (sid.startswith("") is True) — it resolves to nothing (codex).
        for ref in ("", "   ", "\t"):
            with self.subTest(ref=repr(ref)):
                self.assertEqual(AW.resolve_session_ref(self._sessions(), ref).status, "none")

    def test_empty_session_id_never_prefix_matches(self):
        # A running session with no session_id must not match even an empty ref (ae:3400).
        sessions = [_sess("s", session_id="")]
        self.assertEqual(AW.resolve_session_ref(sessions, "s").status, "ok")  # exact name still works
        self.assertEqual(AW.resolve_session_ref(sessions, "x").status, "none")


class AgentRefTest(unittest.TestCase):
    def _agents(self):
        return [_agent("codex:lead", "main"), _agent("optimal:cw", "worker.0")]

    def test_exact_alias_name(self):
        r = AW.resolve_agent_in_session(self._agents(), "codex:lead")
        self.assertTrue(r.ok)
        self.assertEqual(r.value, "codex:lead")

    def test_unique_bare_name_canonicalizes(self):
        r = AW.resolve_agent_in_session(self._agents(), "lead")
        self.assertTrue(r.ok)
        self.assertEqual(r.value, "codex:lead", "a bare name resolves to the canonical alias:name")

    def test_ambiguous_bare_name(self):
        agents = [_agent("codex:lead"), _agent("optimal:lead")]
        self.assertEqual(AW.resolve_agent_in_session(agents, "lead").status, "ambiguous")

    def test_no_such_agent(self):
        self.assertEqual(AW.resolve_agent_in_session(self._agents(), "ghost").status, "none")

    def test_rejects_pane_at_and_foreign_prefixes(self):
        # The confinement (codex BLOCKER): none of these can match a real agent.* entry,
        # so a chat command can never address a raw pane / another session / an external.
        for escape in ("%1", "@other:agent", "telegram:12345", "discord:9", "codex:lead:extra"):
            with self.subTest(escape=escape):
                self.assertEqual(AW.resolve_agent_in_session(self._agents(), escape).status, "none",
                                 f"{escape!r} must not resolve to any agent")

    def test_empty_or_whitespace_agent_is_none(self):
        for want in ("", "   ", "\n"):
            with self.subTest(want=repr(want)):
                self.assertEqual(AW.resolve_agent_in_session(self._agents(), want).status, "none")

    def test_explicit_guard_blocks_even_a_matching_escape_shape(self):
        # The confinement must not depend on agent VALUES: if a future/malicious meta
        # carried an agent whose name contained % or @, an escape-shaped `want` that
        # would otherwise bare-match must STILL be rejected by the explicit guard (codex).
        agents = [_agent("codex:@evil"), _agent("optimal:%pane")]
        self.assertEqual(AW.resolve_agent_in_session(agents, "@evil").status, "none")
        self.assertEqual(AW.resolve_agent_in_session(agents, "%pane").status, "none")


if __name__ == "__main__":
    unittest.main()
