"""Phase-3 Slice 5 contract: real ae/event boundaries (emit_event + recover_pending).

Phase 2 injected these seams from fixtures (env.append_event, a fixture-backed recover
lambda). The live daemon needs the REAL boundaries:

  make_emit_event(sessions_dir, recorder) -> emit(session, event): append the event
    COMPACTLY (one JSON object per line) to <sessions_dir>/<session>/events.jsonl —
    the file live ae processes also append to — under the SAME flock protocol as
    ae_log_append (ae:6192-6198), then record an event.append effect.

  make_recover_pending(sessions_dir, recorder) -> recover(session): shell
    `<ae_path> _recover-pending <session>` (ae_path from the session meta) and parse
    the TSV `<kind>\\t<slot>\\t<agent>\\t<tool>\\t<captured>` (ae:2683-2695; kinds
    ok/already/miss/skip, only ok carries a captured id) into row dicts. A malformed
    row (not five fields) warns through a REDACTED log.write and is skipped. No-op
    ([]) when ae_path is absent or non-executable.

The recover half is proven against a temp ae-path helper (a real executable). Pure
stdlib.
"""

import json
import os
import stat
import tempfile
import textwrap
import unittest
from pathlib import Path

from harness import AW

_FAKE_AE = textwrap.dedent('''\
    #!/usr/bin/env python3
    import sys
    # _recover-pending <session> -> print the fixed TSV rows baked below.
    if sys.argv[1:2] == ["_recover-pending"]:
        sys.stdout.write(__ROWS__)
        sys.exit(0)
    sys.exit(3)
''')


def _write_meta(session_dir, **kv):
    session_dir.mkdir(parents=True, exist_ok=True)
    (session_dir / "meta").write_text("".join(f"{k}={v}\n" for k, v in kv.items()), encoding="utf-8")


class EmitEventTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.sessions = Path(self._tmp.name) / "sessions"
        (self.sessions / "work").mkdir(parents=True)

    def test_appends_compact_one_object_per_line(self):
        rec = AW.EffectRecorder()
        emit = AW.make_emit_event(self.sessions, recorder=rec)
        emit("work", {"ts": "t", "actor": "human", "action": "alert", "summary": "a b"})
        emit("work", {"ts": "u", "actor": "watchdog", "action": "nudge"})
        lines = (self.sessions / "work" / "events.jsonl").read_text().splitlines()
        self.assertEqual(len(lines), 2)
        # Compact: no spaces after ':' or ',' — matches ae_emit_event / _dump_event.
        self.assertEqual(lines[0], '{"ts":"t","actor":"human","action":"alert","summary":"a b"}')
        self.assertEqual(json.loads(lines[1])["action"], "nudge")

    def test_records_event_append_effect(self):
        rec = AW.EffectRecorder()
        emit = AW.make_emit_event(self.sessions, recorder=rec)
        ev = {"ts": "t", "actor": "watchdog", "action": "nudge"}
        emit("work", ev)
        self.assertEqual(rec.as_list(), [{"kind": "event.append", "session": "work", "event": ev}])

    def test_append_is_flock_serialized_like_ae(self):
        # ae_log_append flocks <file>.lock (ae:6195); the daemon shares events.jsonl
        # with live ae, so it must lock the SAME file. Observable proxy: the lock file
        # is created next to events.jsonl.
        emit = AW.make_emit_event(self.sessions)
        emit("work", {"ts": "t", "actor": "watchdog", "action": "nudge"})
        self.assertTrue((self.sessions / "work" / "events.jsonl.lock").exists())

    def test_no_recorder_still_appends(self):
        emit = AW.make_emit_event(self.sessions)
        emit("work", {"ts": "t", "actor": "h", "action": "alert"})
        self.assertEqual(len((self.sessions / "work" / "events.jsonl").read_text().splitlines()), 1)

    def test_lock_timeout_warns_and_drops_without_appending(self):
        # Hold the same <events.jsonl>.lock the daemon takes; a tiny timeout then forces
        # the drop path: warn (redacted) + NO append + NO event.append effect (matches
        # ae_log_append's `flock -w 5 || exit 1` drop-and-continue, codex).
        import fcntl
        events = self.sessions / "work" / "events.jsonl"
        with open(str(events) + ".lock", "a") as held:
            fcntl.flock(held.fileno(), fcntl.LOCK_EX)
            rec = AW.EffectRecorder()
            emit = AW.make_emit_event(self.sessions, recorder=rec, timeout=0.2)
            self.assertIsNone(emit("work", {"ts": "t", "actor": "watchdog", "action": "nudge"}))
        self.assertFalse(events.exists(), "a lock timeout must not append")
        self.assertEqual([e["kind"] for e in rec.as_list()], ["log.write"], "one warning, no event.append")
        self.assertEqual(rec.as_list()[0]["level"], "WARNING")


class RecoverPendingTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.sessions = self.root / "sessions"

    def _fake_ae(self, rows_tsv):
        path = self.root / "fake-ae"
        path.write_text(_FAKE_AE.replace("__ROWS__", repr(rows_tsv)))
        path.chmod(path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
        return path

    def _session_with_ae(self, ae_path):
        _write_meta(self.sessions / "work", session="work", ae_path=str(ae_path))

    def test_parses_all_kinds(self):
        rows = (
            "ok\tmain\tcodex:lead\tcodex\tabcd1234ef\n"
            "already\tworker.0\topt:cw\tclaude\t\n"
            "miss\tworker.1\tfoo:bar\tgemini\t\n"
            "skip\tworker.2\tbaz:qux\t\t\n"
        )
        self._session_with_ae(self._fake_ae(rows))
        recover = AW.make_recover_pending(self.sessions)
        got = recover("work")
        self.assertEqual([r["kind"] for r in got], ["ok", "already", "miss", "skip"])
        self.assertEqual(got[0], {"kind": "ok", "slot": "main", "agent": "codex:lead",
                                  "tool": "codex", "captured": "abcd1234ef"})
        self.assertEqual(got[1]["captured"], "", "already carries no captured id")
        self.assertEqual(got[3], {"kind": "skip", "slot": "worker.2", "agent": "baz:qux",
                                  "tool": "", "captured": ""})

    def test_malformed_row_warns_redacted_and_is_skipped(self):
        rows = "ok\tmain\tcodex:lead\tcodex\tabcd\nnot a tsv row\n"
        self._session_with_ae(self._fake_ae(rows))
        rec = AW.EffectRecorder()
        recover = AW.make_recover_pending(self.sessions, recorder=rec)
        got = recover("work")
        self.assertEqual([r["kind"] for r in got], ["ok"], "the malformed row is skipped")
        warns = [e for e in rec.as_list() if e["kind"] == "log.write" and e["level"] == "WARNING"]
        self.assertEqual(len(warns), 1, "one redacted warning for the malformed row")

    def test_unknown_kind_warns_and_is_skipped(self):
        # A 5-field row whose kind is not ok/already/miss/skip signals ae contract
        # drift — warn+skip, do not keep it silently (codex).
        rows = "ok\tmain\tc:l\tcodex\tabcd\nbogus\tw.0\tx:y\ttool\tcap\n"
        self._session_with_ae(self._fake_ae(rows))
        rec = AW.EffectRecorder()
        recover = AW.make_recover_pending(self.sessions, recorder=rec)
        got = recover("work")
        self.assertEqual([r["kind"] for r in got], ["ok"], "the unknown-kind row is skipped")
        warns = [e for e in rec.as_list() if e["kind"] == "log.write" and e["level"] == "WARNING"]
        self.assertEqual(len(warns), 1)
        self.assertIn("unknown kind", warns[0]["message"])

    def test_noop_when_ae_path_absent(self):
        _write_meta(self.sessions / "work", session="work")  # NO ae_path
        recover = AW.make_recover_pending(self.sessions)
        self.assertEqual(recover("work"), [])

    def test_noop_when_ae_path_not_executable(self):
        plain = self.root / "plain"
        plain.write_text("#!/bin/sh\n")  # exists but NOT chmod +x
        self._session_with_ae(plain)
        recover = AW.make_recover_pending(self.sessions)
        self.assertEqual(recover("work"), [])

    def test_shell_failure_degrades_to_empty(self):
        # ae exits nonzero for a non-_recover-pending arg; a spawn/exec failure must
        # degrade to [] (bash `2>/dev/null || true`), never raise on a read boundary.
        missing = self.root / "does-not-exist"
        self._session_with_ae(missing)  # ae_path set but not executable -> [] already;
        _write_meta(self.sessions / "work", session="work", ae_path=str(missing))
        recover = AW.make_recover_pending(self.sessions)
        self.assertEqual(recover("work"), [])


if __name__ == "__main__":
    unittest.main()
