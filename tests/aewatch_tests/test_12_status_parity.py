"""Phase-2 Slice 4 contract: the FIRST true dual-run parity slice.

The bash oracle (test_11) drives the REAL generated watchdog and records its
ordered effect stream. This slice builds the PYTHON side of the watchdog cycle
(`WatchdogConfig`, `WatchdogState`, `run_watchdog_cycle`, the branch/dirty helper,
and the @ae_watchdog_status / @ae_branch_status / @ae_branch_name writes) and
proves it produces the BYTE-IDENTICAL ordered stream for the status-only fixture.

DONE (per the slice brief): bash-vs-python effects match, ordered, for
`watchdog.status.branch-side-channel`. Both sides run real git on the SAME
non-git work_dir, so the branch side channel resolves identically.

Coarse import red is intentional: the Python cycle + its fixture driver ARE the
deliverable. No real ~/.ae, no ae edits. Pure stdlib.
"""

import shutil
import tempfile
import unittest
from pathlib import Path

from harness import run_bash_watchdog_fixture, run_python_watchdog_fixture
from test_11_bash_oracle import _status_only_fixture


@unittest.skipUnless(shutil.which("git") or True, "parity needs git for the branch side channel")
class StatusParityTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.nogit = self.root / "nogit"  # isolated, deterministically NON-git
        self.nogit.mkdir()

    # Belt-and-suspenders anchor (codex): pin the exact stream against ae ONCE, so
    # two oracle bugs cannot silently agree. Bootstrap 3 effects + one cycle's 3.
    EXPECTED = [
        {"kind": "tmux.set_option", "target": "work", "option": "@ae_watchdog_status", "value": "[watch ◌ starting]"},
        {"kind": "tmux.unset_option", "target": "work", "option": "@ae_branch_name"},
        {"kind": "tmux.set_option", "target": "work", "option": "@ae_branch_status", "value": ""},
        {"kind": "tmux.set_option", "target": "work", "option": "@ae_watchdog_status", "value": "[watch ● 0/0]"},
        {"kind": "tmux.unset_option", "target": "work", "option": "@ae_branch_name"},
        {"kind": "tmux.set_option", "target": "work", "option": "@ae_branch_status", "value": ""},
    ]

    def test_status_fixture_bash_equals_python(self):
        fixture = _status_only_fixture(str(self.nogit))
        bash = run_bash_watchdog_fixture(fixture, self.root / "bash")
        python = run_python_watchdog_fixture(fixture)
        # Independent anchor FIRST: python matches the hand-verified ae stream.
        self.assertEqual(python, self.EXPECTED,
                         "python cycle diverged from the ae-anchored status stream")
        # Then the real dual-run gate: python == the REAL watchdog's stream, ordered.
        self.assertEqual(
            python, bash,
            "python watchdog cycle must reproduce the bash effect stream exactly (ordered)",
        )
        # Guard the shape so a mutual-empty pass can never look green.
        self.assertTrue(bash, "bash oracle produced no effects — fixture broken")
        self.assertEqual(len(python), 6, f"expected the 6-effect status stream, got {len(python)}")


if __name__ == "__main__":
    unittest.main()
