#!/usr/bin/env python3
"""RED-PROOF — every check path in verify-obligations.py, both directions.

Neutral must pass; each seeded mutation must be CAUGHT BY ITS OWN NAMED CHECK, with
the seed diffed first. A seed that does not land is an INVALID TEST, not a pass — a
mutation of an absent phrase produces silence indistinguishable from a working check.
"""
import difflib, os, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")

def run():
    r = subprocess.run([sys.executable, os.path.join(HERE, "verify-obligations.py")],
                       capture_output=True, text=True)
    ids = {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}
    return r.returncode, ids

MUTATIONS = [
    ("FROM", OBL, "a captured value the table misreports",
     lambda s: s.replace("\tschema_version\t1\t2\t", "\tschema_version\t3\t2\t", 1)),
    ("PREDICATE", OBL, "a predicate outside the closed set",
     lambda s: s.replace("\tat-least\t", "\tvibes\t", 1)),
    ("STREAM", OBL, "a stream outside the closed set",
     lambda s: s.replace("\tstderr\t", "\ttelepathy\t", 1)),
    ("WRONG-KIND", OBL, "a membership obligation where the capture shows a label move",
     lambda s: s.replace("\tSC-017l\tdigest\tsessions[].status\tstopped\tunknown\tall-of\t",
                         "\tSC-017m\tdigest\t(row set)\tempty\tunknown rows present\tpresent\t", 1)),
    ("MISSING-509d", OBL, "a digest row stripped of its schema obligation",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro\tlist-json\tSC-509d")))),
    ("VERDICT", OBL, "every obligation removed from a divergent row",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not l.startswith("arms/A1/c01-healthy-ro\tlist-all-json\t"))),
    ("MISSING-509e", OBL, "an unreachable digest stripped of its agent-liveness move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro") and "\tSC-509e\t" in l))),
    ("STALE", FRESH, "the contract having moved since derivation",
     lambda s: s.replace("contract_blob\t", "contract_blob\tdeadbeef", 1)),
]

def main():
    rc, ids = run()
    if rc != 0:
        print("ABORT: neutral is not clean — %s" % sorted(ids)); return 1
    print("neutral            rc=0  clean")
    bad = 0
    for want, target, why, fn in MUTATIONS:
        orig = open(target, encoding="utf-8").read()
        mutated = fn(orig)
        if mutated == orig:
            print("%-14s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why)); bad += 1; continue
        delta = sum(1 for l in difflib.unified_diff(orig.split("\n"), mutated.split("\n"), n=0)
                    if l[:1] in "+-" and l[:3] not in ("+++", "---"))
        shutil.copy(target, target + ".bak")
        open(target, "w", encoding="utf-8").write(mutated)
        rc2, ids2 = run()
        shutil.move(target + ".bak", target)
        ok = rc2 != 0 and want in ids2
        if not ok: bad += 1
        print("%-14s delta=%-3d rc=%d ids=%-28s %s  (%s)"
              % (want, delta, rc2, ",".join(sorted(ids2))[:28] or "-",
                 "caught" if ok else "<-- MISSED", why))
    rc3, _ = run()
    print("restored           rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
    if rc3 != 0: bad += 1
    print("RED-PROOF: %s" % ("ALL PATHS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
