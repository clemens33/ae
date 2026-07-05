"""Slice 6 contract: session discovery with per-meta tmux_server.

Discovers $AE_HOME/sessions/*/meta, parses the core fields + agents, and checks
liveness through EACH session's OWN tmux_server (never a global server). One bad
meta cannot hide good sessions — malformed dirs are skipped with a recorded
warning, not stderr spam. Output is JSON-serializable for later oracle snapshots.

Pure stdlib; fixture-driven via the committed CONTRACTS.md + the fake harness.
"""

import json
import tempfile
import unittest
from pathlib import Path

from harness import AW, build_fixture_env, load_fixture

DISCOVERY_FIXTURE = "session.discovery.two-running"


class DiscoveryTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.rec = AW.EffectRecorder()
        self.home, self.tmux = build_fixture_env(load_fixture(DISCOVERY_FIXTURE), self.root, self.rec)

    def discover(self):
        return {s.name: s for s in AW.discover_sessions(self.home.home, self.tmux, self.rec)}

    def test_output_is_sorted_by_session_name(self):
        # codex: pin deterministic order so phase-2 oracle snapshots aren't flaky.
        names = [s.name for s in AW.discover_sessions(self.home.home, self.tmux, self.rec)]
        self.assertEqual(names, ["docs", "work"])

    def test_discovers_core_fields_from_meta(self):
        found = self.discover()
        self.assertEqual(set(found), {"work", "docs"})
        self.assertEqual(found["work"].session_id, "sess-1")
        self.assertEqual(found["work"].work_dir, "/repo")
        self.assertEqual(found["work"].tmux_server, "")
        self.assertEqual(found["docs"].tmux_server, "ae-alt")

    def test_parses_agents(self):
        work = self.discover()["work"]
        refs = {(a.slot, a.ref, a.session_id) for a in work.agents}
        self.assertIn(("main", "codex:lead", "uuid"), refs)

    def test_liveness_uses_per_meta_tmux_server(self):
        # docs lives on server 'ae-alt'; a global default-server query would report
        # it stopped. Its running=True proves discovery routed to the meta's server.
        # A ghost session (on ae-alt but absent from that server) must be stopped.
        self.home.session("ghost", meta={"session": "ghost", "session_id": "g1", "tmux_server": "ae-alt"})
        found = self.discover()
        self.assertTrue(found["work"].running)
        self.assertTrue(found["docs"].running)
        self.assertFalse(found["ghost"].running)

    def test_malformed_meta_skipped_and_warned_without_aborting(self):
        (self.home.sessions / "broken").mkdir()
        (self.home.sessions / "broken" / "meta").write_text("garbage no equals no session\n", encoding="utf-8")
        found = self.discover()
        # good sessions survive the bad one...
        self.assertEqual(set(found), {"work", "docs"})
        # ...and the bad dir produced a recorded warning naming it (not stderr spam).
        warnings = [e for e in self.rec.as_list() if e["kind"] == "log.write" and "broken" in e.get("message", "")]
        self.assertTrue(warnings, "malformed meta must record a warning log effect")

    def test_dir_without_meta_is_ignored(self):
        (self.home.sessions / "nometa").mkdir()
        found = self.discover()
        self.assertNotIn("nometa", found)
        self.assertEqual(set(found), {"work", "docs"})

    def test_output_is_json_serializable(self):
        sessions = AW.discover_sessions(self.home.home, self.tmux, self.rec)
        dumped = json.dumps([s.to_dict() for s in sessions])
        self.assertIn("\"work\"", dumped)
        # round-trips cleanly
        self.assertEqual(len(json.loads(dumped)), 2)

    def test_clean_discovery_records_no_effects(self):
        AW.discover_sessions(self.home.home, self.tmux, self.rec)
        self.assertEqual(self.rec.as_list(), [], "clean discovery is read-only (no effects)")


if __name__ == "__main__":
    unittest.main()
