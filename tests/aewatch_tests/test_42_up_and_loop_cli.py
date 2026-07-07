"""Phase-3 Slice 18 (aewatch side): the `up` + `daemon --loop` CLI the ae autostart hook
invokes. `up` ensures the ae-aewatch session (s17, idempotent + heartbeat-aware), preserving
the chosen runner (uv vs python3) into the spawned loop argv (codex: no bare sys.executable
PEP723 footgun). `daemon --loop` wires the production supervisor (s16) with a watchdog
component (RealTmuxClient + WatchdogRunner); s18 wires the watchdog ONLY (the bridge lands in s19).

The heavy boundaries (ensure_aewatch_session / run_daemon_loop) are patched so this pins the
CLI CONTRACT + WIRING, not the full live integration. Pure stdlib.
"""

import os
import sys
import tempfile
import unittest
import unittest.mock as mock
from pathlib import Path

from harness import AW


class UpCliTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)

    def _run_up(self, extra, ensure_result="started"):
        captured = {}

        def fake_ensure(runtime, *, argv, tmux_server, clock, logger=None, **kw):
            captured["argv"] = argv
            captured["server"] = tmux_server
            return ensure_result

        with mock.patch.object(AW, "ensure_aewatch_session", fake_ensure):
            rc = AW.main(["up", "--ae-home", str(self.home)] + extra)
        return rc, captured

    def test_uv_runner_builds_uv_loop_argv(self):
        rc, cap = self._run_up(["--runner", "uv"])
        self.assertEqual(rc, 0)
        self.assertEqual(cap["argv"][:2], ["uv", "run"], "uv re-invocation preserves `uv run`")
        self.assertIn("daemon", cap["argv"])
        self.assertIn("--loop", cap["argv"])
        self.assertIn(str(self.home), cap["argv"], "the loop inherits the same --ae-home")

    def test_python_runner_builds_python_loop_argv(self):
        rc, cap = self._run_up(["--runner", "python3"])
        self.assertEqual(cap["argv"][0], sys.executable, "python3 re-invokes the running interpreter")
        self.assertIn("--loop", cap["argv"])

    def test_server_from_flag(self):
        _, cap = self._run_up(["--runner", "uv", "--tmux-server", "ae-root"])
        self.assertEqual(cap["server"], "ae-root")

    def test_server_defaults_to_ae_tmux_server_env(self):
        with mock.patch.dict(os.environ, {"AE_TMUX_SERVER": "envsrv"}):
            _, cap = self._run_up(["--runner", "uv"])
        self.assertEqual(cap["server"], "envsrv")

    def test_up_returns_nonzero_on_failed(self):
        rc, _ = self._run_up(["--runner", "uv"], ensure_result="failed")
        self.assertEqual(rc, 1, "a failed launch is a nonzero exit so the caller can tell")


class DaemonLoopWiringTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.home = Path(self._tmp.name)

    def test_daemon_loop_wires_watchdog_then_bridge(self):
        # s19 wires BOTH the watchdog and the co-located Telegram bridge.
        captured = {}

        def fake_loop(runtime, components, **kw):
            captured["names"] = [name for name, _ in components]
            captured["components"] = components
            return {"started": True}

        with mock.patch.object(AW, "run_daemon_loop", fake_loop):
            rc = AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        self.assertEqual(rc, 0)
        self.assertEqual(captured["names"], ["watchdog", "bridge"],
                         "s19 daemon loop runs the watchdog THEN the bridge")

    def test_bridge_stores_share_the_bash_telegram_dir(self):
        # Offset continuity: the aewatch bridge must read/write bash's SHARED state files
        # ($AE_HOME/telegram/{tg_offset,state.tsv,current_target}) — NOT $AE_HOME/aewatch/ —
        # so it resumes from the bash bridge's last durable offset across the =uv handoff.
        captured = {}

        def fake_loop(runtime, components, **kw):
            captured["bridge"] = dict(components)["bridge"].bridge
            return {"started": True}

        with mock.patch.object(AW, "run_daemon_loop", fake_loop):
            AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        bridge = captured["bridge"]
        tg = self.home / "telegram"
        self.assertEqual(bridge._offset_store._path, tg / "tg_offset")
        self.assertEqual(bridge._outbound_state._path, tg / "state.tsv")
        self.assertEqual(bridge._current_target._path, tg / "current_target")

    def test_telegram_dir_is_created_before_wiring(self):
        # The stores don't create parents; a fresh AE_HOME (no prior bash bridge) must still
        # get $AE_HOME/telegram so inbound-offset/current-target/state saves don't fail.
        with mock.patch.object(AW, "run_daemon_loop", lambda *a, **k: {"started": True}):
            AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        self.assertTrue((self.home / "telegram").is_dir(),
                        "daemon --loop must create $AE_HOME/telegram before wiring the bridge stores")

    def test_bridge_resumes_from_bash_durable_offset(self):
        # Continuity (explicit): a state.tsv written by the BASH bridge in its exact ae-format
        # is read by the WIRED aewatch bridge's OutboundState -> the aewatch bridge resumes
        # from bash's last durable offset across the =uv handoff (no replay of <=offset).
        tg = self.home / "telegram"
        tg.mkdir(parents=True)
        (tg / "state.tsv").write_text("# session_id\tinode\tbyte_offset\tlast_ts\nses-x\t42\t2048\t\n")
        captured = {}

        def fake_loop(runtime, components, **kw):
            captured["bridge"] = dict(components)["bridge"].bridge
            return {"started": True}

        with mock.patch.object(AW, "run_daemon_loop", fake_loop):
            AW.main(["daemon", "--loop", "--ae-home", str(self.home)])
        self.assertEqual(captured["bridge"]._outbound_state.load(), {"ses-x": (42, 2048)},
                         "wired aewatch bridge must resume from the bash bridge's durable state.tsv offset")

    def test_daemon_once_still_smoke_path(self):
        # --loop is additive; --once keeps the existing single-tick behavior.
        rc = AW.main(["daemon", "--once", "--ae-home", str(self.home)])
        self.assertIn(rc, (0, 3))  # 0 acquired, 3 already-locked


if __name__ == "__main__":
    unittest.main()
