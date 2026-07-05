"""Slice 3 contract: the fake-fs / fake-tmux / effect-recorder harness.

This is the LOAD-BEARING slice. In phase 2 the same harness becomes the bash-vs-
aewatch parity oracle, so the Effect schema must normalize EVERYTHING observable
(session/event writes, tmux mutations, log lines, runtime-file writes) and NOTHING
else — reads emit no effects, and unknown effect kinds are rejected.

Everything here is pure stdlib and fully isolated: FakeAeHome owns a temp dir and
never reads or writes the real ~/.ae.
"""

import json
import tempfile
import unittest
from pathlib import Path

from harness import (
    AW,
    FakeAeHome,
    FakeTmux,
    Pane,
    build_fixture_env,
    canonical,
    load_fixture,
)

DISCOVERY_FIXTURE = "session.discovery.two-running"


class HarnessTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    # ── FakeAeHome: realistic ae paths, all under the temp root ─────────
    def test_fakeaehome_paths_isolated_under_temp_root(self):
        fh = FakeAeHome(self.root)
        for p in (fh.home, fh.sessions, fh.aewatch, fh.config):
            self.assertTrue(
                str(p).startswith(str(self.root)), f"{p} escapes temp root {self.root}"
            )
        # Never the real ~/.ae.
        self.assertNotEqual(fh.home.resolve(), (Path.home() / ".ae").resolve())
        self.assertEqual(fh.config, fh.home / "config")
        self.assertEqual(fh.sessions, fh.home / "sessions")
        self.assertEqual(fh.aewatch, fh.home / "aewatch")

    def test_fakeaehome_config_and_session_roundtrip(self):
        fh = FakeAeHome(self.root)
        fh.write_config("[telegram]\nenabled = true\n")
        self.assertTrue(fh.config.is_file())
        sess = fh.session(
            "work",
            meta={"session": "work", "session_id": "s1", "watchdog": "true"},
            events=[{"ts": "2026-07-05T06:20:00Z", "actor": "codex:lead", "action": "state", "summary": "working"}],
        )
        self.assertTrue(str(sess).startswith(str(self.root)))
        self.assertEqual(fh.read_meta("work")["session_id"], "s1")
        self.assertEqual(fh.read_jsonl("work")[0]["action"], "state")
        # runtime files live under $AE_HOME/aewatch/
        self.assertEqual(fh.runtime_file("heartbeat"), fh.aewatch / "heartbeat")

    # ── FakeTmux: reads emit NO effects; mutations emit normalized ones ──
    def test_fixture_env_reads_emit_no_effects(self):
        fixture = load_fixture(DISCOVERY_FIXTURE)
        rec = AW.EffectRecorder()
        fh, tmux = build_fixture_env(fixture, self.root, rec)
        self.assertEqual(sorted(tmux.list_sessions("")), ["work"])  # server "" hosts "work"
        self.assertEqual(tmux.list_sessions("ae-alt"), ["docs"])    # per-meta tmux_server
        # codex IMPORTANT: None (real client's ambient/default server) and ""
        # (fixture meta's default) must resolve to the SAME default server.
        self.assertEqual(tmux.list_sessions(None), ["work"])
        panes = tmux.list_panes("work", "")
        self.assertIsInstance(panes[0], Pane)
        self.assertEqual(panes[0].pane_id, "%1")
        tmux.capture_pane("%1")
        tmux.display_option("%1", "@ae_agent")
        self.assertEqual(rec.as_list(), [], "reads must not record any effect")

    def test_tmux_mutations_record_normalized_effects(self):
        rec = AW.EffectRecorder()
        tmux = FakeTmux(rec, {})
        tmux.set_option("%1", "@ae_watchdog_status", "[watch ok 0/2]")
        tmux.unset_option("%1", "@ae_branch_status")
        tmux.paste("%2", "Status check: ...", submit=True)
        effects = rec.as_list()
        self.assertEqual([e["kind"] for e in effects],
                         ["tmux.set_option", "tmux.unset_option", "tmux.paste"])
        self.assertEqual(effects[0], {"kind": "tmux.set_option", "target": "%1",
                                      "option": "@ae_watchdog_status", "value": "[watch ok 0/2]"})
        self.assertEqual(effects[2], {"kind": "tmux.paste", "target": "%2",
                                      "text": "Status check: ...", "submit": True})

    def test_tmux_target_is_a_generic_target_string(self):
        # codex IMPORTANT: `target` carries the exact tmux target string — a
        # session-level option AND a pane-level option must both round-trip
        # through the same schema (no pane-only semantics baked in).
        rec = AW.EffectRecorder()
        tmux = FakeTmux(rec, {})
        tmux.set_option("work", "status-left", "[ae work]")   # session target
        tmux.set_option("%1", "@ae_agent", "codex:lead")      # pane target
        effects = rec.as_list()
        self.assertEqual(effects[0], {"kind": "tmux.set_option", "target": "work",
                                      "option": "status-left", "value": "[ae work]"})
        self.assertEqual(effects[1], {"kind": "tmux.set_option", "target": "%1",
                                      "option": "@ae_agent", "value": "codex:lead"})

    # ── Effect schema: enumerate observable kinds, reject anything else ──
    def test_make_effect_rejects_unknown_kind(self):
        with self.assertRaises(ValueError):
            AW.make_effect("tmux.teleport", target="%1")

    def test_all_observable_kinds_build_and_serialize(self):
        samples = [
            AW.make_effect("event.append", session="s", event={"action": "alert", "target": "codex:w"}),
            AW.make_effect("tmux.paste", target="%2", text="hi", submit=True),
            AW.make_effect("tmux.set_option", target="%1", option="@ae_x", value="v"),
            AW.make_effect("tmux.unset_option", target="%1", option="@ae_x"),
            AW.make_effect("tmux.display_message", text="[ae watchdog] x is DEAD", duration_ms=10000),
            AW.make_effect("telegram.send", text="[s] chat ..."),
            AW.make_effect("telegram.supervise", tmux_server="ae-alt"),
            AW.make_effect("file.write", path="$AE_HOME/aewatch/heartbeat", redacted=False),
            AW.make_effect("log.write", level="INFO", message="discovered 2 sessions"),
        ]
        # JSON round-trip proves serializability; kinds cover the observable surface.
        self.assertEqual(json.loads(json.dumps(samples)), samples)
        self.assertEqual({e["kind"] for e in samples}, set(AW.EFFECT_KINDS))

    def test_canonical_is_deterministic_presentation_sort_preserving_duplicates(self):
        # canonical() is a deterministic PRESENTATION helper (order-independent),
        # NOT the primary oracle — and it must never collapse duplicates (codex
        # IMPORTANT: phase-2 multiset/sequence comparisons depend on that).
        a = [
            AW.make_effect("log.write", level="INFO", message="b"),
            AW.make_effect("log.write", level="INFO", message="a"),
            AW.make_effect("tmux.paste", target="%1", text="x", submit=False),
        ]
        b = list(reversed(a))
        self.assertEqual(canonical(a), canonical(b))  # deterministic, order-independent
        self.assertEqual(canonical(a), canonical(a))  # stable
        dup = AW.make_effect("log.write", level="INFO", message="dup")
        self.assertEqual(len(canonical([dup, dup])), 2, "canonical must NOT dedup")

    def test_recorded_sequence_is_the_first_class_ordered_oracle(self):
        # The ordered sequence from the recorder — not the sorted canonical — is
        # the primary oracle: it preserves order AND duplicate records (codex
        # IMPORTANT: paste/submit/nudge ordering must survive).
        rec = AW.EffectRecorder()
        rec.record("log.write", level="INFO", message="first")
        rec.record("tmux.paste", target="%1", text="x", submit=True)
        rec.record("log.write", level="INFO", message="first")  # identical to #0
        seq = rec.as_list()
        self.assertEqual([s["kind"] for s in seq], ["log.write", "tmux.paste", "log.write"])
        self.assertEqual(len(seq), 3)
        self.assertEqual(seq[0], seq[2], "duplicate records are kept as separate entries")

    # ── the no-op collect-inputs helper: read-only, records only log ────
    def test_collect_inputs_is_readonly(self):
        fixture = load_fixture(DISCOVERY_FIXTURE)
        rec = AW.EffectRecorder()
        fh, tmux = build_fixture_env(fixture, self.root, rec)
        result = AW.collect_inputs(fh.home, tmux, rec)
        effects = rec.as_list()
        # Only read-only-class effects (log.write) — NO session/tmux mutation.
        self.assertTrue(effects, "collect_inputs should record at least a log line")
        self.assertEqual({e["kind"] for e in effects}, {"log.write"})
        for bad in ("tmux.set_option", "tmux.unset_option", "tmux.paste",
                    "event.append", "file.write", "telegram.send"):
            self.assertNotIn(bad, {e["kind"] for e in effects})
        # It saw both seeded sessions and is JSON-serializable.
        self.assertEqual(sorted(result), ["docs", "work"])
        json.dumps(effects)


if __name__ == "__main__":
    unittest.main()
