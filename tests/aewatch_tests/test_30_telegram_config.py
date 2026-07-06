"""Phase-3 Slice 8 contract: Telegram token config, validation, redaction.

Loads the [telegram] block (ae:3154-3198 read_config + load_runtime_config): enabled
truthiness (1/true/yes/on, ae:3001-3006), token_file with ~ expansion (ae:3186), token
read + CR/LF strip (ae:3188-3190), chat_id required (ae:3192-3193), allowed_user_ids
(ae:3196). Beyond ae, the sidecar HARDENS the token file: it must be owned by the
running user and mode 400 or 600 (ae only checks readable, ae:3187) — a secret must not
be group/world readable.

Done: any failure (missing/empty token, bad owner/mode/readability, missing chat_id)
DISABLES the bridge without leaking the token or the token-file CONTENTS into the
reason. The loaded token wires into AewatchLogger(secrets=...) so it is redacted in
logs even when it doesn't match the generic bot-token pattern. Pure stdlib, dummy token.
"""

import os
import stat
import tempfile
import unittest
import unittest.mock
from pathlib import Path

from harness import AW

_DUMMY = "notarealtoken-plain"  # deliberately NOT a <id>:<secret> shape, to prove the
#                                 LITERAL redaction path (redact_generic would miss it).


def _cfg(**telegram):
    return {"telegram": telegram}


class TelegramConfigTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self._n = 0

    def _token_file(self, content=_DUMMY, mode=0o600, name=None):
        # Unique per call — a prior chmod 0o400 would otherwise block re-writing "tok".
        if name is None:
            self._n += 1
            name = f"tok{self._n}"
        p = self.root / name
        p.write_text(content, encoding="utf-8")
        p.chmod(mode)
        return p

    def _load(self, config, **kw):
        return AW.load_telegram_config(config, **kw)

    # ── happy path ──────────────────────────────────────────────────────
    def test_valid_config_loads_token_chat_allowed(self):
        tf = self._token_file(f"{_DUMMY}\n")  # trailing newline stripped
        cfg = self._load(_cfg(enabled="true", token_file=str(tf), chat_id="99",
                              allowed_user_ids="1, 2 3"))
        self.assertTrue(cfg.enabled)
        self.assertEqual(cfg.token, _DUMMY)
        self.assertEqual(cfg.chat_id, "99")
        self.assertEqual(cfg.allowed_user_ids, "1, 2 3")

    def test_enabled_truthiness(self):
        tf = self._token_file()
        for v in ("1", "true", "yes", "on", "TRUE", "On"):
            with self.subTest(v=v):
                self.assertTrue(self._load(_cfg(enabled=v, token_file=str(tf), chat_id="9")).enabled)
        for v in ("false", "0", "no", "", "bogus"):
            with self.subTest(v=v):
                self.assertFalse(self._load(_cfg(enabled=v, token_file=str(tf), chat_id="9")).enabled)

    def test_expands_tilde_in_token_file(self):
        self._token_file(name="tok")  # ~/tok -> <root>/tok under the patched HOME
        with unittest.mock.patch.dict(os.environ, {"HOME": str(self.root)}):
            cfg = self._load(_cfg(enabled="true", token_file="~/tok", chat_id="9"))
        self.assertTrue(cfg.enabled)
        self.assertEqual(cfg.token, _DUMMY)

    # ── security: owner + mode ──────────────────────────────────────────
    def test_owner_mismatch_disables_without_leak(self):
        tf = self._token_file()
        cfg = self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9"),
                         geteuid=lambda: os.getuid() + 1)
        self.assertFalse(cfg.enabled)
        self.assertNotIn(_DUMMY, cfg.disabled_reason, "owner failure must not read/leak the token")

    def test_bad_mode_disables(self):
        for mode in (0o644, 0o640, 0o660, 0o604):
            with self.subTest(mode=oct(mode)):
                tf = self._token_file(mode=mode)
                cfg = self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9"))
                self.assertFalse(cfg.enabled, f"mode {oct(mode)} must disable")
                self.assertNotIn(_DUMMY, cfg.disabled_reason)

    def test_secure_modes_enable(self):
        for mode in (0o400, 0o600):
            with self.subTest(mode=oct(mode)):
                tf = self._token_file(mode=mode)
                self.assertTrue(self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9")).enabled)

    def test_missing_or_unreadable_token_file_disables(self):
        cfg = self._load(_cfg(enabled="true", token_file=str(self.root / "nope"), chat_id="9"))
        self.assertFalse(cfg.enabled)

    def test_empty_token_disables(self):
        tf = self._token_file(content="\n")
        self.assertFalse(self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9")).enabled)

    def test_missing_chat_id_disables_without_touching_token(self):
        # codex: non-secret config is validated BEFORE the token file is opened, so a
        # missing-chat_id config never reads or retains the token. Patch read_text to
        # blow up if touched — the loader must still return a clean disabled config.
        tf = self._token_file()
        with unittest.mock.patch.object(Path, "read_text",
                                        side_effect=AssertionError("token file must not be read")):
            cfg = self._load(_cfg(enabled="true", token_file=str(tf)))
        self.assertFalse(cfg.enabled)
        self.assertEqual(cfg.token, "")
        self.assertEqual(cfg.secrets, [], "a disabled config carries no secret")
        self.assertNotIn(_DUMMY, cfg.disabled_reason)

    def test_token_newline_cr_strip_mirrors_ae(self):
        # Deliberate ae port (ae:3189-3190): strip ONE trailing newline, then remove all
        # CRs. Normal cases pinned; a token never keeps a trailing \n or \r.
        for content, expected in ((f"{_DUMMY}\n", _DUMMY), (f"{_DUMMY}\r\n", _DUMMY), (_DUMMY, _DUMMY)):
            with self.subTest(content=repr(content)):
                tf = self._token_file(content=content)
                self.assertEqual(self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9")).token, expected)

    def test_not_enabled_disables(self):
        tf = self._token_file()
        self.assertFalse(self._load(_cfg(enabled="false", token_file=str(tf), chat_id="9")).enabled)

    def test_missing_token_file_key_disables(self):
        self.assertFalse(self._load(_cfg(enabled="true", chat_id="9")).enabled)

    # ── redaction wiring ────────────────────────────────────────────────
    def test_loaded_token_redacted_through_logger(self):
        # The literal token (which redact_generic can't match) must be scrubbed once
        # wired into AewatchLogger via cfg.secrets — the s16 daemon composition path.
        tf = self._token_file()
        cfg = self._load(_cfg(enabled="true", token_file=str(tf), chat_id="9"))
        self.assertEqual(cfg.secrets, [_DUMMY])
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(self.root / "daemon.log", secrets=cfg.secrets, recorder=rec)
        logger.log("INFO", f"calling api with {_DUMMY} now")
        (effect,) = rec.as_list()
        self.assertNotIn(_DUMMY, effect["message"], "the token must be redacted in the log.write effect")

    def test_disabled_config_has_no_secrets(self):
        self.assertEqual(self._load(_cfg(enabled="false")).secrets, [])


if __name__ == "__main__":
    unittest.main()
