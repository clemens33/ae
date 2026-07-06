"""Phase-3 Slice 4 contract: RealTmuxClient write path (subprocess-backed mutations).

Slice 3 gave RealTmuxClient its reads; this adds the four MUTATIONS. Each runs the
real tmux write AND records exactly ONE normalized effect whose fields match what
FakeTmux emits, so the daemon's effect stream is schema-identical to the dual-run's.

Bash argv reproduced (against a fake tmux EXECUTABLE that records exact argv):
  set_option     : tmux set-option -t <target> <option> <value>         (ae:7483)
  unset_option   : tmux set-option -t <target> -u <option>              (ae:7224-7226)
  display_message: tmux display-message [-d <ms>] <text>                (ae:7712 etc)
  paste+submit   : tmux_paste_submit (ae:120-151) — set-buffer -b <buf> -- <text> ;
                   paste-buffer -d [-p] -b <buf> -t <pane>  then  send-keys -t <pane>
                   Enter, with a codex bracketed-paste (-p) branch and a claude
                   `[Pasted text #N +M lines]` readiness retry (one bounded extra Enter).

Paste branches on the pane's current command (display-message -p '#{pane_current_command}',
ae:157-159). A WRITE that exits nonzero fails LOUD (a silently-swallowed mutation would
record a phantom effect that never happened); the retry's capture is a read and swallows.
Sleeps are injected so tests never actually wait. Pure stdlib.
"""

import json
import os
import stat
import tempfile
import textwrap
import unittest
from pathlib import Path

from harness import AW, FakeTmux

# Fake `tmux` for the write path: records each PROCESS invocation's argv (one JSON
# line — the set-buffer;paste-buffer pair is one process, so its `;` stays inline),
# serves the command-lookup + capture reads from FIXTURE, and can inject a nonzero
# exit for a named subcommand to exercise loud-failure.
_FAKE_TMUX = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, sys
    from pathlib import Path
    ARGV_LOG = "__ARGV_LOG__"
    FIXTURE = json.loads(Path("__FIXTURE__").read_text())
    argv = sys.argv[1:]
    with open(ARGV_LOG, "a") as fh:
        fh.write(json.dumps(argv) + "\\n")
    if argv[:1] == ["-L"]:
        argv = argv[2:]
    # Split ';'-separated commands (paste stages two in one process).
    cmds, cur = [], []
    for tok in argv:
        if tok == ";":
            cmds.append(cur); cur = []
        else:
            cur.append(tok)
    cmds.append(cur)
    def opt(a, name):
        return a[a.index(name) + 1] if name in a else None
    fail = set(FIXTURE.get("fail", []))
    for c in cmds:
        if c and c[0] in fail:
            sys.stderr.write("fake-tmux: injected failure\\n"); sys.exit(1)
    # READS (single-command): serve, exit 0.
    if len(cmds) == 1:
        c = cmds[0]; sub = c[0] if c else ""
        if sub == "display-message" and "-p" in c:
            target = opt(c, "-t")
            sys.stdout.write(FIXTURE.get("commands", {}).get(target, "") + "\\n"); sys.exit(0)
        if sub == "capture-pane":
            sys.stdout.write(FIXTURE.get("captures", {}).get(opt(c, "-t"), "")); sys.exit(0)
    sys.exit(0)
''')

_PASTED = "[Pasted text #3 +12 lines]"


class RealTmuxWriteTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.fixture_path = self.root / "fixture.json"
        self.argv_log = self.root / "argv.log"
        self.tmux_bin = self.root / "tmux"
        self.tmux_bin.write_text(
            _FAKE_TMUX.replace("__ARGV_LOG__", str(self.argv_log)).replace("__FIXTURE__", str(self.fixture_path))
        )
        self.tmux_bin.chmod(self.tmux_bin.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    def _client(self, fixture=None):
        self.fixture_path.write_text(json.dumps(fixture or {}))
        if self.argv_log.exists():
            self.argv_log.unlink()
        self.recorder = AW.EffectRecorder()
        # sleep injected as a no-op so the readiness pauses never actually wait.
        return AW.RealTmuxClient(self.recorder, tmux_bin=str(self.tmux_bin), sleep=lambda *_: None)

    def _argv_calls(self):
        if not self.argv_log.exists():
            return []
        return [json.loads(ln) for ln in self.argv_log.read_text().splitlines() if ln.strip()]

    def _buf(self, target):
        # RealTmuxClient runs in THIS process, so the pid in the buffer name matches.
        return f"ae-ps-{os.getpid()}-{target}"

    # ── set / unset option ──────────────────────────────────────────────
    def test_set_option_argv_and_effect(self):
        client = self._client()
        client.set_option("work", "@ae_watchdog_status", "idle 3m")
        self.assertEqual(self._argv_calls(),
                         [["set-option", "-t", "work", "@ae_watchdog_status", "idle 3m"]])
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.set_option", "target": "work", "option": "@ae_watchdog_status", "value": "idle 3m"}])

    def test_unset_option_argv_and_effect(self):
        client = self._client()
        client.unset_option("work", "@ae_watchdog_status")
        self.assertEqual(self._argv_calls(),
                         [["set-option", "-t", "work", "-u", "@ae_watchdog_status"]])
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.unset_option", "target": "work", "option": "@ae_watchdog_status"}])

    def test_option_writes_named_server_uses_dash_L(self):
        client = self._client()
        client.set_option("work", "@x", "y", server="srv")
        self.assertEqual(self._argv_calls()[0][:3], ["-L", "srv", "set-option"])

    # ── display-message ─────────────────────────────────────────────────
    def test_display_message_with_duration(self):
        client = self._client()
        client.display_message("[ae watchdog] agent is DEAD", duration_ms=10000)
        self.assertEqual(self._argv_calls(),
                         [["display-message", "-d", "10000", "[ae watchdog] agent is DEAD"]])
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.display_message", "text": "[ae watchdog] agent is DEAD", "duration_ms": 10000}])

    def test_display_message_without_duration(self):
        client = self._client()
        client.display_message("hi")
        self.assertEqual(self._argv_calls(), [["display-message", "hi"]])
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.display_message", "text": "hi", "duration_ms": None}])

    # ── paste + submit ──────────────────────────────────────────────────
    def test_paste_default_command_submits_once(self):
        client = self._client({"commands": {"%1": "node"}})
        client.paste("%1", "nudge text", submit=True)
        calls = self._argv_calls()
        self.assertEqual(calls[0], ["display-message", "-p", "-t", "%1", "#{pane_current_command}"])
        self.assertEqual(calls[1], ["set-buffer", "-b", self._buf("%1"), "--", "nudge text",
                                    ";", "paste-buffer", "-d", "-b", self._buf("%1"), "-t", "%1"])
        self.assertEqual(calls[2], ["send-keys", "-t", "%1", "Enter"])
        self.assertEqual(len(calls), 3, "a non-claude, non-codex pane submits with a single Enter, no retry")
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.paste", "target": "%1", "text": "nudge text", "submit": True}])

    def test_paste_codex_uses_bracketed_paste(self):
        client = self._client({"commands": {"%1": "codex"}})
        client.paste("%1", "msg", submit=True)
        calls = self._argv_calls()
        # codex gets `-p` on paste-buffer (bracketed paste) and a single Enter.
        self.assertIn("-p", calls[1])
        self.assertEqual(calls[1].index("-p"), calls[1].index("paste-buffer") + 2,
                         "-p follows `paste-buffer -d`")
        self.assertEqual(calls[2], ["send-keys", "-t", "%1", "Enter"])
        self.assertEqual(len(calls), 3, "codex does not run the claude readiness retry")

    def test_paste_claude_retries_when_token_present(self):
        client = self._client({"commands": {"%1": "claude"}, "captures": {"%1": f"blah\n{_PASTED}\n"}})
        client.paste("%1", "msg", submit=True)
        calls = self._argv_calls()
        self.assertNotIn("-p", calls[1], "claude uses plain (non-bracketed) paste")
        self.assertEqual(calls[2], ["send-keys", "-t", "%1", "Enter"])
        self.assertEqual(calls[3][:2], ["capture-pane", "-p"], "claude checks readiness via capture")
        self.assertEqual(calls[4], ["send-keys", "-t", "%1", "Enter"], "the staged-paste token needs a second Enter")
        self.assertEqual(len(calls), 5)

    def test_paste_claude_no_retry_when_token_absent(self):
        client = self._client({"commands": {"%1": "claude"}, "captures": {"%1": "ordinary output\n"}})
        client.paste("%1", "msg", submit=True)
        calls = self._argv_calls()
        # command lookup, stage, one Enter, one readiness capture — but NO second Enter.
        self.assertEqual([c[0] if c[0] != "set-buffer" else "stage" for c in calls],
                         ["display-message", "stage", "send-keys", "capture-pane"])

    def test_paste_claude_ignores_stale_token_beyond_last_15_lines(self):
        # Bash checks `tail -n 15 | grep`: a [Pasted text ...] token 20 lines back must
        # NOT trigger the extra Enter. Guards the false-positive extra-submit class.
        stale = _PASTED + "\n" + "\n".join(f"line{i}" for i in range(20)) + "\n"
        client = self._client({"commands": {"%1": "claude"}, "captures": {"%1": stale}})
        client.paste("%1", "msg", submit=True)
        sends = [c for c in self._argv_calls() if c and c[0] == "send-keys"]
        self.assertEqual(len(sends), 1, "a token outside the last 15 lines must NOT cause a second Enter")

    def test_paste_named_server_threads_dash_L_through_every_process(self):
        # The singleton daemon pastes into a pane on its per-meta server: EVERY internal
        # tmux process (command lookup, stage, send-keys, claude capture, retry) must
        # carry `-L srv`. Use a claude pane with an in-window token so the retry runs too.
        client = self._client({"commands": {"%1": "claude"}, "captures": {"%1": f"{_PASTED}\n"}})
        client.paste("%1", "msg", submit=True, server="srv")
        calls = self._argv_calls()
        for c in calls:
            self.assertEqual(c[:2], ["-L", "srv"], f"process missing -L srv: {c}")
        self.assertEqual([c[2] for c in calls],
                         ["display-message", "set-buffer", "send-keys", "capture-pane", "send-keys"])

    def test_paste_without_submit_skips_send_keys(self):
        client = self._client({"commands": {"%1": "claude"}})
        client.paste("%1", "msg", submit=False)
        subs = [c[0] for c in self._argv_calls()]
        self.assertNotIn("send-keys", subs, "submit=False stages the paste but sends no Enter")
        self.assertEqual(self.recorder.as_list(),
                         [{"kind": "tmux.paste", "target": "%1", "text": "msg", "submit": False}])

    def test_paste_buffer_names_match_between_set_and_paste(self):
        client = self._client({"commands": {"%1": "node"}})
        client.paste("%1", "msg", submit=True)
        stage = next(c for c in self._argv_calls() if c and c[0] == "set-buffer")
        set_buf = stage[stage.index("-b") + 1]
        paste_buf = stage[stage.index("-b", stage.index("paste-buffer")) + 1]
        self.assertEqual(set_buf, paste_buf)
        self.assertTrue(set_buf.startswith("ae-ps-"))

    # ── parity + loud failure ───────────────────────────────────────────
    def test_effects_match_faketmux(self):
        real = self._client({"commands": {"%1": "node"}})
        fake = FakeTmux(AW.EffectRecorder())
        for client in (real, fake):
            client.set_option("work", "@s", "v")
            client.unset_option("work", "@s")
            client.display_message("alert", duration_ms=10000)
            client.paste("%1", "text", submit=True)
        self.assertEqual(self.recorder.as_list(), fake._rec.as_list(),
                         "RealTmuxClient effect stream must match FakeTmux field-for-field")

    def test_write_nonzero_fails_loud_and_records_no_effect(self):
        client = self._client({"fail": ["set-option"]})
        with self.assertRaises(RuntimeError):
            client.set_option("work", "@x", "y")
        self.assertEqual(self.recorder.as_list(), [], "a failed mutation must not record a phantom effect")


if __name__ == "__main__":
    unittest.main()
