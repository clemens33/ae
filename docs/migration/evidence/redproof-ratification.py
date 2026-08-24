#!/usr/bin/env python3
"""RED-PROOF for verify-ratification.py — every check, by its own named id.

Neutral must pass; each seeded mutation must be CAUGHT BY THE CHECK IT TARGETS,
with the seed diffed first. A seed that does not land is an INVALID TEST, not a
pass: mutating an absent phrase produces silence indistinguishable from a working
check. A gate that cannot fail proves nothing about the file it reads.
"""
import difflib, os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRIT = os.path.join(HERE, "ratification-critical.md")

def run():
    r = subprocess.run([sys.executable, os.path.join(HERE, "verify-ratification.py")],
                       capture_output=True, text=True)
    return r.returncode, {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}

MUTATIONS = [
    ("COVERAGE", "a contract row stripped of its classification entry",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("- SC-017p — "))),
    ("ORPHAN", "a class assigned to an id that is not a contract row",
     lambda s: s.rstrip("\n") + "\n- SC-9999 — CRITICAL(A)\n"),
    ("DUPLICATE", "one row classified twice",
     lambda s: s.replace("- SC-016a — ", "- SC-016a — CRITICAL(C)\n- SC-016a — ", 1)),
    ("CLASS", "a class outside the closed set",
     lambda s: s.replace("- SC-016b — CRITICAL", "- SC-016b — PROBABLY", 1)),
    ("COUNT", "a header count that no longer matches the entries",
     lambda s: s.replace("DEFERRABLE=103", "DEFERRABLE=104", 1)),
    ("STALE", "the contract having moved since this classification was derived",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}",
                      "contract_blob: " + "a" * 40, s, count=1)),
    ("MALFORMED", "a contract_blob present but not a blob id — a DIFFERENT defect from absent",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}", "contract_blob: yesterday", s, count=1)),
    ("FRESHNESS", "the freshness relation removed entirely",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("contract_blob:"))),
]

def main():
    rc, ids = run()
    if rc != 0:
        print("ABORT: neutral is not clean — %s" % sorted(ids)); return 1
    print("neutral        rc=0  clean")
    bad = 0
    for want, why, fn in MUTATIONS:
        orig = open(CRIT, encoding="utf-8").read()
        mutated = fn(orig)
        if mutated == orig:
            print("%-10s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why)); bad += 1; continue
        delta = sum(1 for l in difflib.unified_diff(orig.split("\n"), mutated.split("\n"), n=0)
                    if l[:1] in "+-" and l[:3] not in ("+++", "---"))
        shutil.copy(CRIT, CRIT + ".bak")
        open(CRIT, "w", encoding="utf-8").write(mutated)
        rc2, ids2 = run()
        shutil.move(CRIT + ".bak", CRIT)
        ok = rc2 != 0 and want in ids2
        bad += 0 if ok else 1
        print("%-10s delta=%-3d rc=%d ids=%-24s %s  (%s)"
              % (want, delta, rc2, ",".join(sorted(ids2))[:24] or "-",
                 "caught" if ok else "<-- MISSED", why))
    rc3, _ = run()
    print("restored       rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
    if rc3 != 0: bad += 1
    print("RED-PROOF: %s" % ("ALL PATHS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
