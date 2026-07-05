"""Phase-2 Slice 14 contract: phase-gate hardening + fast-subset lane.

Two jobs:
  1. FAST-SUBSET LANE — the dual-run bash-oracle tests are the slow part (each is a
     `ae doctor --refresh` + `watchdog _run` subprocess). AEWATCH_FAST=1 makes
     run_bash_watchdog_fixture raise SkipTest, so the commit-gate can run the
     pure-Python surface fast; the FULL suite (no marker) stays the phase/nightly
     gate that actually exercises bash-vs-python.
  2. COVERAGE GUARDS — every effect kind the bash oracle can emit is in
     EFFECT_KINDS (an unknown kind would silently forge parity), and the ae script
     stays UNTOUCHED (the zero-ae-edit invariant is part of the gate unless a
     reviewed seam slice says otherwise).

Pure stdlib.
"""

import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

from harness import AW, run_bash_watchdog_fixture

_ROOT = Path(__file__).resolve().parents[2]
_HERE = Path(__file__).resolve().parent
_FAKEBIN = _HERE / "fakebin"


class PhaseGateTest(unittest.TestCase):
    def test_fast_marker_skips_the_bash_oracle(self):
        # With AEWATCH_FAST set, the bash oracle is skipped (not run) so the fast
        # lane never pays the subprocess cost.
        old = os.environ.get("AEWATCH_FAST")
        os.environ["AEWATCH_FAST"] = "1"
        try:
            with self.assertRaises(unittest.SkipTest):
                run_bash_watchdog_fixture({"sessions": [{"name": "x", "meta": {"session": "x"}}]}, self._t())
        finally:
            if old is None:
                os.environ.pop("AEWATCH_FAST", None)
            else:
                os.environ["AEWATCH_FAST"] = old

    def test_fast_marker_requires_exactly_1(self):
        # Only "1" enables the fast lane — AEWATCH_FAST=0 must NOT skip (else a stray
        # CI/user env would silently downgrade the phase gate). An empty-sessions
        # fixture fails fast (ValueError) AFTER the marker check, proving no skip.
        old = os.environ.get("AEWATCH_FAST")
        os.environ["AEWATCH_FAST"] = "0"
        try:
            with self.assertRaises(ValueError):
                run_bash_watchdog_fixture({"sessions": []}, self._t())
        finally:
            if old is None:
                os.environ.pop("AEWATCH_FAST", None)
            else:
                os.environ["AEWATCH_FAST"] = old

    def test_every_bash_effect_kind_is_known(self):
        # EVERY source that writes an effect record on the bash-oracle side must only
        # use kinds in EFFECT_KINDS, or the oracle could emit a kind the Python side
        # can never produce -> a silent parity hole (codex: cover all sources).
        kinds = set()
        # Dict-literal "kind":"..." — the fakebin shims (tmux/ae).
        for path in (_FAKEBIN / "tmux", _FAKEBIN / "ae"):
            kinds |= set(re.findall(r'"kind"\s*:\s*"([^"]+)"', path.read_text(encoding="utf-8")))
        # bash_oracle._lib wrapper writes ESCAPED JSON inside a Python string
        # (\"kind\":\"event.append\") — the unescaped regex finds NOTHING there, so
        # match the escaped form too (codex: a typo there must not evade the guard).
        bo = (_HERE / "bash_oracle.py").read_text(encoding="utf-8")
        kinds |= set(re.findall(r'"kind"\s*:\s*"([^"]+)"', bo))
        kinds |= set(re.findall(r'\\"kind\\"\s*:\s*\\"([^\\"]+)\\"', bo))
        # .record("<kind>", ...) — the harness FakeTmux mutations, append_event, and
        # the supervise recorder bindings.
        kinds |= set(re.findall(r'\.record\(\s*"([^"]+)"', (_HERE / "harness.py").read_text(encoding="utf-8")))
        self.assertIn("event.append", kinds, "the bash_oracle _lib wrapper's kind must be covered")
        self.assertTrue(kinds, "expected to find recorded effect kinds")
        self.assertLessEqual(kinds, set(AW.EFFECT_KINDS),
                             f"a bash-side source records unknown effect kind(s): {kinds - set(AW.EFFECT_KINDS)}")

    def test_fast_lane_actually_skips_a_dual_run_test(self):
        # Beyond the helper raising in isolation: run a REAL dual-run module under the
        # marker and prove unittest reports it SKIPPED (discovery/skip behavior works).
        env = dict(os.environ, AEWATCH_FAST="1")
        r = subprocess.run([sys.executable, "-m", "unittest", "-v", "test_12_status_parity"],
                           cwd=str(_HERE), env=env, capture_output=True, text=True)
        self.assertIn("skipped", r.stderr.lower(),
                      f"fast lane must SKIP the bash-oracle dual-run:\n{r.stderr[-400:]}")

    def test_ae_script_is_untouched(self):
        # Zero-ae-edit invariant: the phase-2 port must not have dirtied ae. (A
        # reviewed seam/bugfix slice commits ae separately; the working tree is clean.)
        r = subprocess.run(["git", "-C", str(_ROOT), "diff", "--exit-code", "--", "ae"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, f"ae has uncommitted changes:\n{r.stdout}")

    def _t(self):
        import tempfile
        d = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(d, ignore_errors=True))
        return d


if __name__ == "__main__":
    unittest.main()
