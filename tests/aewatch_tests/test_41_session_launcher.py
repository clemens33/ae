"""Phase-3 Slice 17 contract: dedicated ae-aewatch tmux session launcher.

Ensures the sidecar daemon runs in its OWN tmux session — per-AE_HOME, on the root's own
tmux server (the P1 s4 singleton scope), NOT a global session (ae:4067-4079 is the ae
port). Idempotent + heartbeat-aware:
  - not running               -> START (tmux new-session -d, status off, rename)
  - running + FRESH heartbeat  -> RUNNING (no-op — refuses a duplicate)
  - running + stale/no beat    -> RESTART (kill-session, recreate) — a wedged daemon
The daemon argv is shell-QUOTED safely; every tmux call carries -L <server> so it targets
the root's server. Proven against a fake tmux EXECUTABLE (no real tmux).
"""

import json
import os
import stat
import tempfile
import textwrap
import unittest
from pathlib import Path

from harness import AW

# Fake tmux: models session existence in a JSON file + records every argv. has-session
# exits 0/1; new-session/kill-session mutate the set; other subcommands just record.
_FAKE_TMUX = textwrap.dedent('''\
    #!/usr/bin/env python3
    import json, sys
    from pathlib import Path
    STATE = Path("__STATE__"); ARGV = Path("__ARGV__")
    argv = sys.argv[1:]
    with ARGV.open("a") as fh:
        fh.write(json.dumps(argv) + "\\n")
    if argv[:2] == ["-L", "__SRV__"] or (argv[:1] != ["-L"]):
        rest = argv[2:] if argv[:1] == ["-L"] else argv
    else:
        rest = argv
    sessions = set(json.loads(STATE.read_text())) if STATE.exists() else set()
    def opt(name):
        return rest[rest.index(name) + 1] if name in rest else None
    sub = rest[0] if rest else ""
    if sub == "has-session":
        sys.exit(0 if opt("-t") in sessions else 1)
    if sub == "new-session":
        if Path("__FAILNEW__").exists():
            sys.exit(1)  # injected create failure
        sessions.add(opt("-s")); STATE.write_text(json.dumps(sorted(sessions))); sys.exit(0)
    if sub == "kill-session":
        sessions.discard(opt("-t")); STATE.write_text(json.dumps(sorted(sessions))); sys.exit(0)
    sys.exit(0)
''')


class LauncherTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        self.runtime = AW.AewatchRuntime(self.root)
        self.state = self.root / "tmux-sessions.json"
        self.argv_log = self.root / "tmux-argv.log"
        self.tmux = self.root / "tmux"
        self.now = 100_000

    def _fake_tmux(self, server=""):
        self.fail_new = self.root / "fail-new"
        self.tmux.write_text(_FAKE_TMUX.replace("__STATE__", str(self.state))
                             .replace("__ARGV__", str(self.argv_log)).replace("__SRV__", server)
                             .replace("__FAILNEW__", str(self.fail_new)))
        self.tmux.chmod(self.tmux.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    def _existing(self, *names):
        self.state.write_text(json.dumps(sorted(names)))

    def _beat(self, age):
        self.runtime.dir.mkdir(parents=True, exist_ok=True)
        hb = self.runtime.heartbeat_path
        hb.write_text("x")
        os.utime(hb, (self.now - age, self.now - age))

    def _calls(self):
        return [json.loads(ln) for ln in self.argv_log.read_text().splitlines()] if self.argv_log.exists() else []

    def _ensure(self, *, server="", argv=("aewatch", "daemon"), **kw):
        self._fake_tmux(server)
        return AW.ensure_aewatch_session(self.runtime, argv=list(argv), tmux_server=server,
                                         tmux_bin=str(self.tmux), clock=lambda: self.now,
                                         heartbeat_max_age=90, **kw)

    def _subcommands(self):
        return [c[2] if c[:1] == ["-L"] else c[0] for c in self._calls()]

    def test_not_running_starts(self):
        self.assertEqual(self._ensure(), "started")
        self.assertIn("new-session", self._subcommands())

    def test_running_with_fresh_heartbeat_is_a_noop(self):
        self._existing("ae-aewatch")
        self._beat(10)  # fresh (< 90)
        self.assertEqual(self._ensure(), "running")
        self.assertNotIn("new-session", self._subcommands(), "a live daemon is not duplicated")
        self.assertNotIn("kill-session", self._subcommands())

    def test_running_with_stale_heartbeat_restarts(self):
        self._existing("ae-aewatch")
        self._beat(1000)  # stale (> 90)
        self.assertEqual(self._ensure(), "restarted")
        subs = self._subcommands()
        self.assertEqual(subs.count("kill-session"), 1)
        self.assertIn("new-session", subs)
        self.assertLess(subs.index("kill-session"), subs.index("new-session"), "kill before recreate")

    def test_running_with_no_heartbeat_restarts(self):
        self._existing("ae-aewatch")  # no heartbeat file at all
        self.assertEqual(self._ensure(), "restarted")
        self.assertIn("kill-session", self._subcommands())

    def test_server_propagated_to_every_tmux_call(self):
        result = self._ensure(server="ae-root")
        self.assertEqual(result, "started")
        for c in self._calls():
            self.assertEqual(c[:2], ["-L", "ae-root"], f"every tmux call targets the root server: {c}")

    def test_argv_is_shell_quoted(self):
        # a metachar-laden argv must be safely quoted into the new-session command string,
        # never interpreted.
        self._ensure(argv=["aewatch", "daemon", "--ae-home", "/tmp/a b; rm -rf /"])
        newsess = next(c for c in self._calls() if ("new-session" in c))
        cmd = newsess[-1]  # the command string is the last positional
        self.assertIn("'/tmp/a b; rm -rf /'", cmd, "the dangerous path is single-quoted, not bare")

    def test_command_carries_env_overlay(self):
        self._ensure(argv=["aewatch", "daemon"], env={"CONFIG_FILE": "/x/cfg"})
        newsess = next(c for c in self._calls() if "new-session" in c)
        self.assertIn("CONFIG_FILE=/x/cfg", newsess[-1])

    def test_ae_home_in_command_by_default(self):
        # per-AE_HOME scope: the daemon must read the same root even without explicit env.
        self._ensure(argv=["aewatch", "daemon"])
        newsess = next(c for c in self._calls() if "new-session" in c)
        self.assertIn(f"AE_HOME={self.root}", newsess[-1], "AE_HOME defaults to the runtime root")

    def test_new_session_failure_returns_failed_not_false_success(self):
        # best-effort must NOT lie: a create failure -> 'failed', logged, never 'started'.
        self._fake_tmux()
        (self.root / "fail-new").write_text("x")
        rec = AW.EffectRecorder()
        logger = AW.AewatchLogger(self.root / "d.log", secrets=[], recorder=rec)
        result = AW.ensure_aewatch_session(self.runtime, argv=["aewatch", "daemon"],
                                           tmux_bin=str(self.tmux), clock=lambda: self.now,
                                           heartbeat_max_age=90, logger=logger)
        self.assertEqual(result, "failed")
        self.assertTrue(any("create failed" in e.get("message", "") for e in rec.as_list()))


if __name__ == "__main__":
    unittest.main()
