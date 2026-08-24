#!/usr/bin/env python3
"""Prove the inherited human projection can accept changed membership."""
from __future__ import annotations

import sys
import types
from pathlib import Path


HERE = Path(__file__).resolve().parent
CANDIDATE = HERE / "inherited" / "human_project.py"


def main() -> int:
    module = types.ModuleType("inherited_human_project")
    # The preserved candidate derives REPO from __file__.  Keep its bytes unchanged,
    # but give the red-proof the historical, one-level-shallower location it expects.
    module.__file__ = str(HERE.parent / "human_project.py")
    sys.modules[module.__name__] = module
    exec(compile(CANDIDATE.read_bytes(), str(CANDIDATE), "exec"), module.__dict__)
    key = ("seed-case", "list-all")
    module.OBS[key] = {"SC-017l"}
    module.UNS.pop(key, None)
    baseline = module.project_baseline(b"kept\tstopped\n")
    successor = module.project_successor(b"replaced\tunknown\n")
    verdict, detail = module.compare(baseline, successor, *key)
    if verdict != "layout-open":
        print(f"INVALID: seed stopped distinguishing candidate output: {verdict} {detail}")
        return 1
    print("RED: candidate accepted changed session membership as layout-open")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
