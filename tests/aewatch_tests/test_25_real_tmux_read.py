"""Phase-3 Slice 3 contract: RealTmuxClient read path (subprocess-backed reads).

Phase 1/2 ran the watchdog against FakeTmux; the real daemon (slice 16-17) needs a
subprocess client that discovers sessions/panes/options off a live `tmux`. This slice
implements the READ half and pins it against a FAKE tmux EXECUTABLE that records exact
argv, so the argv construction (the thing most likely to drift from bash) is proven
end-to-end.

Bash argv the real client must reproduce:
  list_sessions : tmux [-L <srv>] list-sessions -F '#{session_name}'        (ae:1174/3032)
  list_panes    : tmux [-L <srv>] list-panes -s -t <s>
                        -F '#{pane_id}|#{@ae_agent}|#{pane_current_command}|#{pane_pid}' (ae:7952)
  capture_pane  : tmux capture-pane -p -J -S -40 -E - -t <pane>             (ae:7805)
  display_option: tmux show-options -t <target> -qv <option>               (ae:1493)

Server normalization: None (ambient) and "" (a meta's default tmux_server) resolve to
the SAME default server — NO `-L`. Reads swallow a nonzero tmux (stopped/absent
session -> [] / "" / None, mirroring bash `2>/dev/null || true`) and record NO effect.

Pure stdlib. Uses a real executable (not an injected runner) so the subprocess path is
exercised for real; the fake bakes its fixture + argv-log paths as literals (no env
threading, so RealTmuxClient stays env-agnostic).
"""

import json
import os
import stat
import tempfile
import textwrap
import unittest
from pathlib import Path

from harness import AW

# A fake `tmux` executable: records each invocation's argv (JSON line) to ARGV_LOG,
# then answers reads from FIXTURE. `-L <server>` is parsed off the front; a missing
# server bucket / absent session exits 1 (as real tmux does with no server/target).
_FAKE_TMUX = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, sys
    from pathlib import Path
    ARGV_LOG = "__ARGV_LOG__"
    FIXTURE = json.loads(Path("__FIXTURE__").read_text())
    argv = sys.argv[1:]
    with open(ARGV_LOG, "a") as fh:
        fh.write(json.dumps(argv) + "\\n")
    server = ""
    if argv[:1] == ["-L"]:
        server = argv[1]; argv = argv[2:]
    sub = argv[0] if argv else ""
    def opt(name):
        return argv[argv.index(name) + 1] if name in argv else None
    if sub == "list-sessions":
        bucket = FIXTURE.get("sessions_by_server", {}).get(server)
        if bucket is None:
            sys.exit(1)  # no server running
        sys.stdout.write("".join(n + "\\n" for n in bucket)); sys.exit(0)
    if sub == "list-panes":
        session = opt("-t")
        rows = FIXTURE.get("panes", {}).get(server, {}).get(session)
        if rows is None:
            sys.exit(1)  # session absent on this server
        sys.stdout.write("".join(r + "\\n" for r in rows)); sys.exit(0)
    if sub == "capture-pane":
        pane = opt("-t")
        sys.stdout.write(FIXTURE.get("captures", {}).get(pane, "")); sys.exit(0)
    if sub == "show-options":
        target = opt("-t"); option = opt("-qv")
        val = FIXTURE.get("options", {}).get(target, {}).get(option)
        if val is None:
            sys.exit(0)  # -q: unset option prints nothing, exits 0
        sys.stdout.write(val + "\\n"); sys.exit(0)
    sys.stderr.write("fake-tmux: unmodeled " + " ".join(sys.argv[1:]) + "\\n"); sys.exit(2)
''')


class RealTmuxReadTest(unittest.TestCase):
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

    def _client(self, fixture):
        self.fixture_path.write_text(json.dumps(fixture))
        if self.argv_log.exists():
            self.argv_log.unlink()
        self.recorder = AW.EffectRecorder()
        return AW.RealTmuxClient(self.recorder, tmux_bin=str(self.tmux_bin))

    def _argv_calls(self):
        if not self.argv_log.exists():
            return []
        return [json.loads(ln) for ln in self.argv_log.read_text().splitlines() if ln.strip()]

    # ── list_sessions ───────────────────────────────────────────────────
    def test_default_server_omits_dash_L_and_None_equals_empty(self):
        fx = {"sessions_by_server": {"": ["work", "orchestrator"]}}
        for server in (None, ""):
            client = self._client(fx)
            self.assertEqual(client.list_sessions(server), ["work", "orchestrator"])
            self.assertEqual(self._argv_calls(), [["list-sessions", "-F", "#{session_name}"]],
                             f"server={server!r} must produce identical no--L argv")

    def test_named_server_uses_dash_L(self):
        fx = {"sessions_by_server": {"srv": ["alpha"]}}
        client = self._client(fx)
        self.assertEqual(client.list_sessions("srv"), ["alpha"])
        self.assertEqual(self._argv_calls(),
                         [["-L", "srv", "list-sessions", "-F", "#{session_name}"]])

    def test_stopped_server_returns_empty_not_crash(self):
        # tmux exits 1 when no server is running -> [] (bash `2>/dev/null || true`).
        client = self._client({"sessions_by_server": {}})
        self.assertEqual(client.list_sessions(""), [])

    # ── list_panes ──────────────────────────────────────────────────────
    def test_list_panes_argv_and_pipe_parsing(self):
        rows = ["%1|codex:lead|node|12345", "%2|optimal:cw|python|678"]
        client = self._client({"panes": {"": {"work": rows}}})
        panes = client.list_panes("work", "")
        self.assertEqual(self._argv_calls(), [[
            "list-panes", "-s", "-t", "work", "-F",
            "#{pane_id}|#{@ae_agent}|#{pane_current_command}|#{pane_pid}",
        ]])
        self.assertEqual([(p.pane_id, p.agent, p.current_command, p.pane_pid) for p in panes],
                         [("%1", "codex:lead", "node", "12345"), ("%2", "optimal:cw", "python", "678")])

    def test_list_panes_empty_agent_field_preserved(self):
        # A pane with no @ae_agent prints an empty middle field; the row is KEPT with
        # agent="" (the cycle, not the client, drops non-agent panes).
        client = self._client({"panes": {"": {"work": ["%9||bash|42"]}}})
        (pane,) = client.list_panes("work", "")
        self.assertEqual((pane.pane_id, pane.agent, pane.current_command, pane.pane_pid),
                         ("%9", "", "bash", "42"))

    def test_list_panes_command_with_pipe_absorbed_by_last_split(self):
        # Mirrors bash `IFS='|' read -r a b c d` — the LAST field absorbs extra pipes,
        # so pane_pid keeps trailing content rather than corrupting earlier fields.
        client = self._client({"panes": {"": {"work": ["%1|a:b|weird|cmd|999"]}}})
        (pane,) = client.list_panes("work", "")
        self.assertEqual((pane.pane_id, pane.agent, pane.current_command, pane.pane_pid),
                         ("%1", "a:b", "weird", "cmd|999"))

    def test_list_panes_blank_lines_skipped(self):
        client = self._client({"panes": {"": {"work": ["%1|x:y|node|1", "", "  "]}}})
        panes = client.list_panes("work", "")
        self.assertEqual([p.pane_id for p in panes], ["%1"])

    def test_list_panes_absent_session_returns_empty(self):
        client = self._client({"panes": {"": {"work": ["%1|x:y|node|1"]}}})
        self.assertEqual(client.list_panes("ghost", ""), [])

    def test_list_panes_named_server_uses_dash_L(self):
        client = self._client({"panes": {"srv": {"work": ["%1|x:y|node|1"]}}})
        client.list_panes("work", "srv")
        self.assertEqual(self._argv_calls()[0][:3], ["-L", "srv", "list-panes"])

    # ── capture_pane ────────────────────────────────────────────────────
    def test_capture_pane_argv_and_output(self):
        client = self._client({"captures": {"%1": "line-a\nline-b\n"}})
        self.assertEqual(client.capture_pane("%1"), "line-a\nline-b\n")
        self.assertEqual(self._argv_calls(),
                         [["capture-pane", "-p", "-J", "-S", "-40", "-E", "-", "-t", "%1"]])

    def test_capture_pane_named_server_uses_dash_L(self):
        # The singleton daemon reaches a session on its per-meta tmux_server (codex):
        # a named server must prefix `-L <server>`; None/"" stay ambient (above).
        client = self._client({"captures": {"%1": "hi"}})
        self.assertEqual(client.capture_pane("%1", server="srv"), "hi")
        self.assertEqual(self._argv_calls()[0][:3], ["-L", "srv", "capture-pane"])

    # ── display_option ──────────────────────────────────────────────────
    def test_display_option_present_and_absent(self):
        client = self._client({"options": {"work": {"@ae_branch_name": "ae/feature"}}})
        self.assertEqual(client.display_option("work", "@ae_branch_name"), "ae/feature")
        self.assertEqual(self._argv_calls()[0],
                         ["show-options", "-t", "work", "-qv", "@ae_branch_name"])
        client = self._client({"options": {}})
        self.assertIsNone(client.display_option("work", "@ae_branch_name"))

    def test_display_option_named_server_uses_dash_L(self):
        client = self._client({"options": {"work": {"@ae_branch_name": "ae/feature"}}})
        self.assertEqual(client.display_option("work", "@ae_branch_name", server="srv"), "ae/feature")
        self.assertEqual(self._argv_calls()[0][:3], ["-L", "srv", "show-options"])

    # ── invariants ──────────────────────────────────────────────────────
    def test_reads_record_no_effect(self):
        client = self._client({
            "sessions_by_server": {"": ["work"]},
            "panes": {"": {"work": ["%1|x:y|node|1"]}},
            "captures": {"%1": "hi"},
            "options": {"work": {"@ae_branch_name": "b"}},
        })
        client.list_sessions("")
        client.list_panes("work", "")
        client.capture_pane("%1")
        client.display_option("work", "@ae_branch_name")
        self.assertEqual(self.recorder.as_list(), [], "reads must record no effect")

    def test_spawn_failure_returns_empty_not_raise(self):
        # A missing tmux binary (OSError) must degrade to []/"" /None, never propagate.
        rec = AW.EffectRecorder()
        client = AW.RealTmuxClient(rec, tmux_bin=str(self.root / "does-not-exist"))
        self.assertEqual(client.list_sessions(""), [])
        self.assertEqual(client.list_panes("work", ""), [])
        self.assertEqual(client.capture_pane("%1"), "")
        self.assertIsNone(client.display_option("work", "@x"))

    def test_pane_type_is_compatible_with_harness_pane(self):
        # AW.Pane must carry the fields the cycle reads (pane_id/agent/current_command/
        # pane_pid) so the real client's panes drive run_watchdog_cycle unchanged.
        for field in ("pane_id", "agent", "current_command", "pane_pid"):
            self.assertIn(field, {f.name for f in AW.dataclasses.fields(AW.Pane)})


if __name__ == "__main__":
    unittest.main()
