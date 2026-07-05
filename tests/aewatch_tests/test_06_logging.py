"""Slice 7 contract: daemon.log rotation + secret redaction.

Security-relevant: a daemon log must NEVER contain a raw Telegram bot token, a
bot<TOKEN> API URL, or configured token-file contents — in the live file OR any
rotated backup. Rotation is size-capped and bounded (daemon.log.1..N), and a
logging failure must never crash `daemon --once`.

Pure stdlib; isolated under temp roots.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from harness import AW

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"

# A realistic Telegram bot token (bot id : ~35-char secret). NEVER log this raw.
BOT_TOKEN = "987654321:AAHfakeTOKENsecret0123456789abcdefXYZ"
TOKEN_FILE_BLOB = "s3cr3t-blob-from-token-file-not-a-bot-token"


class LoggingTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.rt = AW.AewatchRuntime(self.root)

    def all_log_text(self):
        """Concatenate daemon.log and every rotated backup."""
        parts = []
        for p in sorted(self.rt.dir.glob("daemon.log*")):
            parts.append(p.read_text(encoding="utf-8"))
        return "\n".join(parts)

    def test_logs_to_daemon_log_path(self):
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        logger.log("INFO", "hello")
        self.assertEqual(self.rt.daemon_log_path, self.rt.dir / "daemon.log")
        self.assertTrue(self.rt.daemon_log_path.is_file())
        self.assertIn("hello", self.rt.daemon_log_path.read_text(encoding="utf-8"))

    def test_redacts_raw_bot_token_to_exact_form(self):
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        logger.log("INFO", f"connecting with token {BOT_TOKEN} now")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn(BOT_TOKEN, text)
        self.assertNotIn("AAHfakeTOKENsecret", text)  # the secret half must be gone
        self.assertNotIn("987654321:", text)          # codex: no partial (id:) leak
        self.assertIn("<REDACTED-TOKEN>", text)         # exact form

    def test_redacts_bot_api_url_to_exact_form(self):
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        url = f"https://api.telegram.org/bot{BOT_TOKEN}/sendMessage"
        logger.log("INFO", f"POST {url}")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn(BOT_TOKEN, text)
        self.assertNotIn("AAHfakeTOKENsecret", text)
        self.assertNotIn("bot987654321:", text)         # codex: no partial (bot<id>:) leak
        self.assertIn("bot<REDACTED-TOKEN>", text)       # exact form

    def test_redacts_nonstandard_bot_url_token_no_partial_leak(self):
        # codex IMPORTANT: a bot token whose secret contains '.' must not be
        # PARTIALLY redacted (leaving the tail after '.'), which would also slip
        # past the residual fail-closed detector. The URL token is consumed to '/'.
        weird = "bot123456:abcdefghijklmnop.qrstuvwxyz"
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        logger.log("INFO", f"POST https://api.telegram.org/{weird}/sendMessage")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn("qrstuvwxyz", text)      # the leaked tail must be gone
        self.assertNotIn("abcdefghijklmnop", text)
        self.assertIn("bot<REDACTED-TOKEN>", text)

    def test_redacts_nonstandard_standalone_token_no_partial_leak(self):
        # same bypass class for a bare (non-URL) token: a >=20-char prefix before a
        # '.' would partial-match the standard-charset regex, drop the 'digits:'
        # anchor, and slip the tail past the residual detector.
        weird = "123456:abcdefghijklmnopqrstuv.wxyz0123456789"
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        logger.log("INFO", f"token {weird} end")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn("wxyz0123456789", text)          # leaked tail must be gone
        self.assertNotIn("abcdefghijklmnopqrstuv", text)

    def test_redacts_secret_with_trailing_newline(self):
        # codex: token files end with '\n'; a caller passing secrets=[read_text()]
        # but logging the stripped value must still be redacted.
        logger = AW.AewatchLogger(self.rt.daemon_log_path, secrets=[TOKEN_FILE_BLOB + "\n"])
        logger.log("INFO", f"loaded {TOKEN_FILE_BLOB}")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn(TOKEN_FILE_BLOB, text)

    def test_empty_configured_secret_is_ignored(self):
        # codex NIT: str.replace("", ...) corrupts every line — empties must be dropped.
        logger = AW.AewatchLogger(self.rt.daemon_log_path, secrets=["", TOKEN_FILE_BLOB])
        logger.log("INFO", f"hello {TOKEN_FILE_BLOB} world")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertIn("hello", text)
        self.assertIn("world", text)
        self.assertNotIn(TOKEN_FILE_BLOB, text)

    def test_stderr_on_error_never_echoes_secret(self):
        # codex: CLI-mode write failure must warn about path/error only — never echo
        # the raw log line (which carries the token/secret).
        import contextlib
        import io

        self.rt.daemon_log_path.mkdir()  # force the write to fail
        logger = AW.AewatchLogger(self.rt.daemon_log_path, stderr_on_error=True)
        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            logger.log("INFO", f"bad {BOT_TOKEN} {TOKEN_FILE_BLOB}")
        err = buf.getvalue()
        self.assertNotIn(BOT_TOKEN, err)
        self.assertNotIn("AAHfakeTOKENsecret", err)
        self.assertNotIn(TOKEN_FILE_BLOB, err)

    def test_redacts_configured_secret_verbatim(self):
        # A token-file blob that does NOT match the bot-token pattern must still be
        # redacted when the logger is told about it.
        logger = AW.AewatchLogger(self.rt.daemon_log_path, secrets=[TOKEN_FILE_BLOB])
        logger.log("INFO", f"loaded token file: {TOKEN_FILE_BLOB}")
        text = self.rt.daemon_log_path.read_text(encoding="utf-8")
        self.assertNotIn(TOKEN_FILE_BLOB, text)
        self.assertIn("REDACTED", text)

    def test_rotation_is_bounded_and_redacted(self):
        # Small cap forces rotation; assert backups are bounded and NO backup leaks.
        logger = AW.AewatchLogger(self.rt.daemon_log_path, max_bytes=200, backups=3)
        for i in range(50):
            logger.log("INFO", f"line {i} token {BOT_TOKEN}")
        backups = sorted(p.name for p in self.rt.dir.glob("daemon.log.*"))
        self.assertLessEqual(len(backups), 3, f"too many backups: {backups}")
        self.assertNotIn("daemon.log.4", backups)
        # every rotated file stays under the cap-ish bound and none leaks the token
        self.assertNotIn(BOT_TOKEN, self.all_log_text())

    def test_rotation_deterministic_state(self):
        logger = AW.AewatchLogger(self.rt.daemon_log_path, max_bytes=64, backups=2)
        for i in range(20):
            logger.log("I", f"msg{i:03d}")
        present = {p.name for p in self.rt.dir.glob("daemon.log*")}
        self.assertIn("daemon.log", present)
        self.assertIn("daemon.log.1", present)
        self.assertIn("daemon.log.2", present)
        self.assertNotIn("daemon.log.3", present)  # backups=2 is the hard cap

    def test_log_failure_does_not_raise(self):
        # Point the log at an unwritable path (a directory) -> write fails, but the
        # logger must swallow it (best-effort) and never raise.
        (self.rt.dir / "daemon.log").mkdir()
        logger = AW.AewatchLogger(self.rt.daemon_log_path)
        logger.log("INFO", "this write will fail")  # must not raise

    def test_daemon_once_writes_redacted_daemon_log(self):
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root), "--once"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertTrue(self.rt.daemon_log_path.is_file(), "daemon --once should write daemon.log")

    def test_daemon_once_survives_log_failure(self):
        # daemon.log is a directory -> the tick's log write fails, but daemon --once
        # must still succeed (heartbeat written, rc0).
        self.rt.daemon_log_path.mkdir()
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root), "--once"],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertTrue(self.rt.heartbeat_path.is_file(), "heartbeat must survive a log failure")


if __name__ == "__main__":
    unittest.main()
