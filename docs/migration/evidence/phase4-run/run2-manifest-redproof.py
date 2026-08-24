#!/usr/bin/env python3
"""C14 red proof: every prescribed scratch-state delta changes its manifest."""
from __future__ import annotations

import importlib.util
import os
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
RUNNER = HERE / "run2.py"


def load_runner():
    spec = importlib.util.spec_from_file_location("run2_runner", RUNNER)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load run2 runner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot_after(mutate) -> tuple[bytes, bytes]:
    runner = load_runner()
    with tempfile.TemporaryDirectory(prefix="ae-p4-run2-manifest-redproof-") as raw:
        root = Path(raw)
        (root / "nested").mkdir()
        (root / "nested" / "payload").write_bytes(b"baseline")
        os.symlink("nested/payload", root / "current")
        before = runner.state_manifest(root)
        mutate(root)
        return before, runner.state_manifest(root)


def require_red(label: str, mutate) -> None:
    before, after = snapshot_after(mutate)
    if before == after:
        raise RuntimeError(f"INVALID {label}: mutation did not land in manifest")
    print(f"RED {label}: manifest changed")


def main() -> int:
    require_red("path-add", lambda root: (root / "added").write_bytes(b"new"))
    require_red("path-delete", lambda root: (root / "nested" / "payload").unlink())
    require_red("content-change", lambda root: (root / "nested" / "payload").write_bytes(b"changed"))
    require_red("symlink-retarget", lambda root: ((root / "current").unlink(), os.symlink("nested", root / "current")))
    require_red("mode-change", lambda root: os.chmod(root / "nested" / "payload", 0o444))
    print("C14-MANIFEST-REDPROOF PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
