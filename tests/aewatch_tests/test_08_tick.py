"""Slice 9 contract: the composed phase-1 daemon tick (the last phase-1 slice).

`daemon --once` composes acquire-singleton -> parse config -> discover sessions
(read-only, per-meta tmux_server) -> refresh heartbeat -> write daemon.log ->
reset backoff, recording EVERY observable action as a normalized effect. Phase 1
is a smoke path, NOT a watchdog: it must emit NO event.append / tmux mutation /
telegram.send effects. Folds the two standing watch items:
  - the heartbeat write is an EFFECT_KINDS `file.write` record;
  - every `log.write` effect payload passes through redaction.

Fixture-driven, in-process, isolated temp roots; pure stdlib.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from harness import AW, build_fixture_env, load_fixture

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"
DISCOVERY_FIXTURE = "session.discovery.two-running"
BOT_TOKEN = "987654321:AAHfakeTOKENsecret0123456789abcdefXYZ"

FORBIDDEN = {"event.append", "tmux.paste", "tmux.set_option", "tmux.unset_option", "telegram.send"}


class TickTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_tick_composes_readonly_with_expected_effects(self):
        rec = AW.EffectRecorder()
        _home, tmux = build_fixture_env(load_fixture(DISCOVERY_FIXTURE), self.root, rec)
        runtime = AW.AewatchRuntime(self.root)
        result = AW.run_daemon_tick(runtime, tmux, rec, now=1000.0)

        self.assertTrue(result["acquired"], "the tick must acquire the singleton")
        self.assertIn("workspace", result["config"], "config must be parsed")
        self.assertEqual(len(result["sessions"]), 2, "both sessions discovered")
        self.assertTrue(runtime.heartbeat_path.is_file(), "heartbeat touched")

        kinds = [e["kind"] for e in rec.as_list()]
        self.assertIn("log.write", kinds)
        self.assertIn("file.write", kinds)
        # No watchdog/telegram side effects in phase 1.
        self.assertEqual(
            FORBIDDEN & set(kinds), set(), f"phase 1 emitted forbidden effects: {FORBIDDEN & set(kinds)}"
        )
        # codex: phase-1 effects are exactly the aewatch-owned surface.
        self.assertLessEqual(set(kinds), {"file.write", "log.write"})

    def test_runtime_writes_are_file_write_effects(self):
        # watch item #1 (+ codex): BOTH aewatch-owned runtime writes — heartbeat AND
        # the backoff reset — are recorded as file.write effects with a stable shape.
        rec = AW.EffectRecorder()
        _home, tmux = build_fixture_env(load_fixture(DISCOVERY_FIXTURE), self.root, rec)
        runtime = AW.AewatchRuntime(self.root)
        AW.run_daemon_tick(runtime, tmux, rec, now=1000.0)
        writes = {e["path"]: e for e in rec.as_list() if e["kind"] == "file.write"}
        self.assertIn(str(runtime.heartbeat_path), writes, "heartbeat must be a file.write effect")
        self.assertIn(str(runtime.backoff_path), writes, "backoff reset must be a file.write effect")
        self.assertIs(writes[str(runtime.heartbeat_path)]["redacted"], False)
        self.assertIs(writes[str(runtime.backoff_path)]["redacted"], False)

    def test_log_write_effect_payload_is_redacted(self):
        # watch item #2: a log.write EFFECT payload — not just the daemon.log file —
        # passes through redaction.
        rec = AW.EffectRecorder()
        runtime = AW.AewatchRuntime(self.root)
        logger = AW.AewatchLogger(runtime.daemon_log_path, recorder=rec)
        logger.log("INFO", f"connecting with {BOT_TOKEN}")
        effects = [e for e in rec.as_list() if e["kind"] == "log.write"]
        self.assertTrue(effects, "logger with a recorder must emit a log.write effect")
        self.assertNotIn(BOT_TOKEN, effects[0]["message"])
        self.assertIn("<REDACTED-TOKEN>", effects[0]["message"])

    def test_all_log_write_effects_are_redacted_not_just_the_logger(self):
        # codex IMPORTANT: recorder.log call sites (discovery/backoff warnings) must
        # also redact. A malformed session dir NAMED like a bot token would leak the
        # token into a discovery warning effect unless redaction is centralized.
        rec = AW.EffectRecorder()
        _home, tmux = build_fixture_env(load_fixture(DISCOVERY_FIXTURE), self.root, rec)
        runtime = AW.AewatchRuntime(self.root)
        bad = runtime.ae_home / "sessions" / BOT_TOKEN  # dir name IS a token
        bad.mkdir(parents=True)
        (bad / "meta").write_text("garbage no session key\n", encoding="utf-8")
        AW.run_daemon_tick(runtime, tmux, rec, now=1000.0)
        for e in rec.as_list():
            if e["kind"] == "log.write":
                self.assertNotIn(BOT_TOKEN, e["message"], f"log.write effect leaked a token: {e}")
                self.assertNotIn("AAHfakeTOKENsecret", e["message"])

    def test_tick_when_locked_does_no_work(self):
        runtime = AW.AewatchRuntime(self.root)
        held = runtime.singleton()
        self.assertTrue(held.acquire())
        try:
            rec = AW.EffectRecorder()
            _home, tmux = build_fixture_env(load_fixture(DISCOVERY_FIXTURE), self.root, rec)
            result = AW.run_daemon_tick(runtime, tmux, rec, now=1000.0)
            self.assertFalse(result["acquired"], "a locked-out tick must not run")
            # codex: assert NO physical side-effect files, not just no heartbeat —
            # a bad impl could write before acquiring the lock and still pass.
            self.assertFalse(runtime.heartbeat_path.exists(), "locked-out: no heartbeat")
            self.assertFalse(runtime.daemon_log_path.exists(), "locked-out: no daemon.log")
            self.assertFalse(runtime.backoff_path.exists(), "locked-out: no backoff.json")
            self.assertEqual(rec.as_list(), [], "locked-out tick emits no effects")
        finally:
            held.release()

    def test_cli_daemon_once_composes_discovery(self):
        # the subprocess CLI path composes the same tick: it discovers on-disk
        # sessions and records the count in daemon.log.
        (self.root / "sessions" / "s1").mkdir(parents=True)
        (self.root / "sessions" / "s1" / "meta").write_text(
            "session=s1\nsession_id=x\ntmux_server=\n", encoding="utf-8"
        )
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root), "--once"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        runtime = AW.AewatchRuntime(self.root)
        log_text = runtime.daemon_log_path.read_text(encoding="utf-8")
        self.assertIn("discovered 1 session", log_text)


if __name__ == "__main__":
    unittest.main()
