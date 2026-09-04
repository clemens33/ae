#!/usr/bin/env python3
"""Function-name census: did a structural edit remove something it was not aimed at?

Six times in this file an unbounded text slice between two anchors swallowed a
function it was never meant to touch — the failure is silent, because the next
reference to the lost name is usually far from the edit and the file still parses.
A promise to bound slices better lives in a habit, and an expectation that lives in
a habit gets dropped exactly like one that lives in a message. This is the guard.

It is FAMILY-SET one level down: instead of asserting that every declared check
RAN, it asserts that every module-level name that EXISTED still exists.

    python3 defcensus.py            # compare the worktree against HEAD
    python3 defcensus.py --save     # record the current names as the baseline
    python3 defcensus.py --redproof # prove the census can go red

Additions are reported and never fail — writing a new function is the normal case.
REMOVALS fail, because a removal is either deliberate (re-run --save) or the defect
this exists to catch.
"""
from __future__ import annotations

import ast
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
WATCHED = ("obligations.py", "verify-obligations.py", "redproof-obligations.py")
BASELINE = HERE / ".defcensus.tsv"


def names(source: str) -> set[str]:
    """Module-level def and class names, plus module-level assignments."""
    tree = ast.parse(source)
    out: set[str] = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            out.add(node.name)
        elif isinstance(node, ast.Assign):
            out.update(t.id for t in node.targets if isinstance(t, ast.Name))
    return out


def at_head(rel: str) -> str | None:
    r = subprocess.run(["git", "show", f"HEAD:docs/migration/evidence/corpus/{rel}"],
                       capture_output=True, text=True, cwd=HERE.parents[3])
    return r.stdout if r.returncode == 0 else None


def baseline() -> dict[str, set[str]]:
    if not BASELINE.exists():
        return {}
    out: dict[str, set[str]] = {}
    for line in BASELINE.read_text(encoding="utf-8").splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        rel, _, rest = line.partition("\t")
        out[rel] = set(rest.split(","))
    return out


def compare(quiet: bool = False) -> int:
    base = baseline()
    bad = 0
    for rel in WATCHED:
        path = HERE / rel
        if not path.exists():
            continue
        now = names(path.read_text(encoding="utf-8"))
        # THE SAVED BASELINE WINS. HEAD cannot see the loss of a name HEAD never
        # had, so in-flight work compared against HEAD reports "none lost" while a
        # function written this hour vanishes — measured, that is exactly what
        # happened on the first live test.
        was = base.get(rel)
        if was is None:
            head = at_head(rel)
            was = names(head) if head else None
        if was is None:
            if not quiet:
                print(f"{rel}: no baseline and untracked — recording is the only option")
            continue
        lost, gained = sorted(was - now), sorted(now - was)
        if lost:
            bad += 1
            print(f"FAIL  DEF-CENSUS  {rel}: {len(lost)} name(s) REMOVED — {lost}")
        elif not quiet:
            print(f"ok    {rel}: {len(now)} names, {len(gained)} added, none lost")
    return 1 if bad else 0


def redproof() -> int:
    """Prove the census goes red on a REAL excision, not on set arithmetic.

    The first version computed `now - (now - {victim})`, which is {victim} by
    algebra and could never fail — a control that cannot go red, in the guard built
    to stop repeating that exact mistake. It also hid a real gap: comparing against
    HEAD cannot see the loss of a name HEAD never had, so in-flight work needs a
    SAVED baseline. Both were found by excising a function for real and watching the
    tool say "none lost".
    """
    import shutil
    import tempfile
    rel = WATCHED[0]
    path = HERE / rel
    source = path.read_text(encoding="utf-8")
    before = names(source)
    victim = "owed_loss_members" if "owed_loss_members" in before else sorted(before)[-1]
    marker = f"def {victim}("
    if marker not in source:
        print(f"REDPROOF  cannot locate {victim!r} to excise — INVALID TEST")
        return 1
    start = source.index(marker)
    rest = source[start + len(marker):]
    nxt = rest.find("\ndef ")
    cut = source[:start] + (rest[nxt + 1:] if nxt >= 0 else "")
    with tempfile.TemporaryDirectory(prefix="defcensus-") as td:
        backup = Path(td) / "backup.py"
        shutil.copy(path, backup)
        try:
            path.write_text(cut, encoding="utf-8")
            after = names(path.read_text(encoding="utf-8"))
            lost = sorted(before - after)
            ok = victim in lost
            print(f"REDPROOF  excised {victim!r} for real: census reports lost={lost} -> "
                  f"{'caught' if ok else 'MISSED'}")
        finally:
            shutil.copy(backup, path)
    return 0 if ok else 1


def main() -> int:
    if "--save" in sys.argv:
        rows = []
        for rel in WATCHED:
            p = HERE / rel
            if p.exists():
                rows.append(f"{rel}\t{','.join(sorted(names(p.read_text(encoding='utf-8'))))}")
        BASELINE.write_text("# module-level names, the defcensus baseline\n"
                            + "\n".join(rows) + "\n", encoding="utf-8")
        print(f"baseline recorded: {len(rows)} file(s)")
        return 0
    if "--redproof" in sys.argv:
        return redproof()
    return compare()


if __name__ == "__main__":
    raise SystemExit(main())
