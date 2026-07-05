"""Slice 4 contract: per-AE_HOME singleton lock + heartbeat.

One supervisor per ae state root (NOT per machine): all runtime state lives under
$AE_HOME/aewatch/, so isolated roots (tests, e2e) never collide with a live one.
The singleton uses fcntl.flock on $AE_HOME/aewatch/aewatch.lock; `daemon --once`
is a skeleton tick that acquires the lock and touches the heartbeat, nothing else.

Pure stdlib; fully isolated under temp roots (never the real ~/.ae).
"""

import errno
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

from harness import AW

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"


def run_daemon_once(ae_home):
    return subprocess.run(
        [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(ae_home), "--once"],
        capture_output=True,
        text=True,
    )


class SingletonTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_runtime_paths_under_ae_home(self):
        rt = AW.AewatchRuntime(self.root)
        self.assertEqual(rt.lock_path, self.root / "aewatch" / "aewatch.lock")
        self.assertEqual(rt.heartbeat_path, self.root / "aewatch" / "heartbeat")
        for p in (rt.lock_path, rt.heartbeat_path):
            self.assertTrue(str(p).startswith(str(self.root / "aewatch")))

    def test_missing_ae_home_created_private(self):
        target = self.root / "fresh"  # does not exist yet
        rt = AW.AewatchRuntime(target)
        self.assertTrue(rt.dir.is_dir(), "aewatch dir must be created")
        mode = rt.dir.stat().st_mode & 0o777
        self.assertEqual(mode & 0o077, 0, f"aewatch dir not private: {oct(mode)}")

    def test_second_acquire_same_root_fails_without_touching_heartbeat(self):
        rt = AW.AewatchRuntime(self.root)
        first = rt.singleton()
        self.assertTrue(first.acquire(), "first acquire should succeed")
        try:
            second = rt.singleton()
            self.assertFalse(second.acquire(), "second acquire on same root must fail")
            self.assertFalse(
                rt.heartbeat_path.exists(), "contended acquire must not touch heartbeat"
            )
        finally:
            first.release()

    def test_different_roots_acquire_independently(self):
        a = AW.AewatchRuntime(self.root / "a")
        b = AW.AewatchRuntime(self.root / "b")
        la, lb = a.singleton(), b.singleton()
        try:
            self.assertTrue(la.acquire())
            self.assertTrue(lb.acquire(), "different AE_HOME roots must not contend")
        finally:
            la.release()
            lb.release()

    def test_acquire_reraises_non_contention_flock_errors(self):
        # codex IMPORTANT: only real contention is "already running". A genuine
        # flock failure (e.g. ENOLCK, no-locks-available) must NOT masquerade as
        # a held lock and silently suppress the heartbeat — it must propagate.
        rt = AW.AewatchRuntime(self.root)
        lock = rt.singleton()
        with mock.patch.object(
            AW.fcntl, "flock", side_effect=OSError(errno.ENOLCK, "no locks available")
        ):
            with self.assertRaises(OSError):
                lock.acquire()

    def test_release_allows_reacquire(self):
        rt = AW.AewatchRuntime(self.root)
        lock = rt.singleton()
        self.assertTrue(lock.acquire())
        lock.release()
        again = rt.singleton()
        self.assertTrue(again.acquire(), "lock must be re-acquirable after release")
        again.release()

    def test_daemon_once_touches_heartbeat_under_ae_home(self):
        rt = AW.AewatchRuntime(self.root)
        self.assertFalse(rt.heartbeat_path.exists())
        proc = run_daemon_once(self.root)
        self.assertEqual(proc.returncode, 0, f"daemon --once failed: {proc.stderr}")
        self.assertTrue(rt.heartbeat_path.is_file(), "daemon --once must touch heartbeat")
        self.assertTrue(str(rt.heartbeat_path).startswith(str(self.root)))

    def test_daemon_without_once_exits_2_and_leaves_heartbeat_untouched(self):
        # codex: pin the phase-1 "--once required" boundary (loop mode is later).
        rt = AW.AewatchRuntime(self.root)
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "daemon", "--ae-home", str(self.root)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 2, f"daemon without --once should be a usage error: {proc.stderr}")
        self.assertFalse(rt.heartbeat_path.exists(), "usage-error daemon must not touch heartbeat")

    def test_daemon_once_second_tick_refreshes_heartbeat(self):
        # codex: existence-only is too weak — a heartbeat must refresh each tick.
        rt = AW.AewatchRuntime(self.root)
        self.assertEqual(run_daemon_once(self.root).returncode, 0)
        first_payload = rt.heartbeat_path.read_text(encoding="utf-8")
        first_mtime = rt.heartbeat_path.stat().st_mtime_ns
        self.assertEqual(run_daemon_once(self.root).returncode, 0)
        second_payload = rt.heartbeat_path.read_text(encoding="utf-8")
        second_mtime = rt.heartbeat_path.stat().st_mtime_ns
        self.assertNotEqual(first_payload, second_payload, "heartbeat payload must refresh per tick")
        self.assertGreaterEqual(second_mtime, first_mtime)

    def test_daemon_once_when_locked_reports_already_running(self):
        rt = AW.AewatchRuntime(self.root)
        lock = rt.singleton()
        self.assertTrue(lock.acquire())
        try:
            proc = run_daemon_once(self.root)
            self.assertNotEqual(proc.returncode, 0, "locked daemon must not exit 0")
            self.assertIn("already running", (proc.stdout + proc.stderr).lower())
            self.assertFalse(
                rt.heartbeat_path.exists(), "a locked-out daemon must not touch heartbeat"
            )
        finally:
            lock.release()


if __name__ == "__main__":
    unittest.main()
