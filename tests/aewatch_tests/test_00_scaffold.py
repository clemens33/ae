"""Slice 1 scaffold contract for the aewatch sidecar.

Proves the single-file PEP 723 skeleton exists and is a runnable, stdlib-only
Python >= 3.11 script before any behavior is implemented. Pure stdlib unittest;
no third-party deps, no network, no real ~/.ae touched.
"""

import os
import py_compile
import re
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
AEWATCH = REPO_ROOT / "contrib" / "aewatch" / "aewatch"


class ScaffoldTest(unittest.TestCase):
    def test_script_exists_and_executable(self):
        self.assertTrue(AEWATCH.is_file(), f"missing sidecar script: {AEWATCH}")
        self.assertTrue(
            os.access(AEWATCH, os.X_OK), f"sidecar not executable: {AEWATCH}"
        )

    def test_py_compiles(self):
        # Runner gates the interpreter to >= 3.11; here we prove it byte-compiles.
        py_compile.compile(str(AEWATCH), doraise=True)

    def test_pep723_block_present_and_dependency_free(self):
        text = AEWATCH.read_text(encoding="utf-8")
        # PEP 723 inline script metadata: opened by `# /// script`, closed by the
        # next exact `# ///`. Extract the block body and assert the metadata lives
        # INSIDE it — a matching comment elsewhere in the file must not satisfy
        # the contract (codex NIT).
        block = re.search(
            r"^# /// script$\n(?P<body>.*?)^# ///$",
            text,
            re.MULTILINE | re.DOTALL,
        )
        self.assertIsNotNone(
            block, "PEP 723 script metadata block (# /// script ... # ///) not found"
        )
        body = block.group("body")
        self.assertRegex(body, r'(?m)^#\s*requires-python\s*=\s*">=3\.11"\s*$')
        # Dependencies block present AND empty (any dep needs a written why later).
        self.assertRegex(body, r'(?m)^#\s*dependencies\s*=\s*\[\s*\]\s*$')

    def test_version_prints_stable_line(self):
        proc = subprocess.run(
            [sys.executable, str(AEWATCH), "--version"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 0, f"--version exited {proc.returncode}: {proc.stderr}")
        line = proc.stdout.strip()
        self.assertRegex(line, r"^aewatch \d+\.\d+\.\d+$")


if __name__ == "__main__":
    unittest.main()
