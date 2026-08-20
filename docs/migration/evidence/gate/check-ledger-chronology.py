#!/usr/bin/env python3
"""Ledger chronology: sequence identities must be UNIQUE and MONOTONIC.

The ledgers exist to establish ORDER. Nothing else in the gate looks at order — presence
and hash both pass on a file that repeats an identity or moves backwards, which is a
chronologically impossible record with a correct checksum.

Found the expensive way: an arm ran its measured invocation inside `( cd dir && ... )`, the
subshell advanced the sequence counter, the parent resumed at its own stale value, and
every ledger in the arm repeated two identities. Gate: clean.

A ledger cannot be repaired after capture — renumbering manufactures the ordering the file
exists to attest — so a failure here means re-running the arm, not editing the artifact.

usage: check-ledger-chronology.py <tree> [--redproof]
"""
import glob, os, re, sys, tempfile

def read_seqs(path):
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        m = re.match(r"seq=(\d+)\t", line)
        if m:
            out.append(int(m.group(1)))
    return out

def problems(path):
    seqs = read_seqs(path)
    bad = []
    if not seqs:
        return [("EMPTY", "no seq= lines at all")]
    seen = set()
    for i, n in enumerate(seqs):
        if n in seen:
            bad.append(("DUPLICATE", f"seq={n:03d} appears more than once (position {i + 1})"))
        seen.add(n)
        if i and n < seqs[i - 1]:
            bad.append(("BACKWARD", f"seq={n:03d} follows seq={seqs[i - 1]:03d}"))
    return bad

def scan(tree):
    fails = []
    for led in sorted(glob.glob(os.path.join(tree, "**", "admissibility-ledger.txt"), recursive=True)):
        for kind, detail in problems(led):
            fails.append((os.path.relpath(led, tree), kind, detail))
    return fails

if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        sys.stderr.write("usage: check-ledger-chronology.py <tree> [--redproof]\n")
        sys.exit(2)
    tree = args[0]
    fails = scan(tree)
    if "--redproof" in args:
        print("## red-proof — each arm reports both states")
        d = tempfile.mkdtemp()
        def arm(lines, label, expect):
            p = os.path.join(d, "admissibility-ledger.txt")
            open(p, "w").write("".join(f"seq={n:03d}\tutc=x\tevent=e\n" for n in lines))
            caught = bool(problems(p))
            print(f"  {label:34s} caught={'YES' if caught else 'NO'}")
            if caught != expect:
                fails.append((label, "RED-PROOF", f"expected caught={expect}"))
        arm([1, 2, 3, 4], "clean ledger [neutral]", False)
        arm([1, 2, 3, 2], "duplicate identity [mutated]", True)
        arm([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 5, 6], "the subshell shape [mutated]", True)
        arm([1, 2, 5, 4], "backward step [mutated]", True)
        arm([], "empty ledger [mutated]", True)
    nled = len(glob.glob(os.path.join(tree, "**", "admissibility-ledger.txt"), recursive=True))
    print(f"ledgers_checked={nled} chronology_failures={len(fails)}")
    for f in fails[:20]:
        print(f"  LEDGER {f[0]} :: {f[1]} — {f[2]}")
    sys.exit(1 if fails else 0)
