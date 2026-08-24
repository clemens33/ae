#!/usr/bin/env python3
"""RED-PROOF for verify-ratification.py — every check, by its own named id.

IT NEVER MUTATES THE TRACKED FILE. Earlier it copied, overwrote and restored
`ratification-critical.md` in the shared checkout: even a clean run exposed seeded
bytes to any concurrent reader or committer, and this session has already shipped a
mutation out of exactly that race. Seeds are written to an isolated temp directory
and the verifier is pointed at them with --file / --worktree-contract. Never mutate
the subject to test its checker.

A seed that does not land is an INVALID TEST, not a pass: mutating an absent phrase
produces silence indistinguishable from a working check.
"""
import difflib, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CRIT = os.path.join(HERE, "ratification-critical.md")
CONTRACT = os.path.normpath(os.path.join(HERE, "..", "semantic-contract.md"))
VERIFY = os.path.join(HERE, "verify-ratification.py")

def run(crit_path=None, wt_path=None):
    cmd = [sys.executable, VERIFY]
    if crit_path: cmd += ["--file", crit_path]
    if wt_path: cmd += ["--worktree-contract", wt_path]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}

# (want, why, crit-mutator or None, worktree-mutator or None)
MUTATIONS = [
    ("COVERAGE", "a contract row stripped of its classification entry",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("- SC-017p — ")), None),
    ("ORPHAN", "a class assigned to an id that is not a contract row",
     lambda s: s.rstrip("\n") + "\n- SC-9999 — CRITICAL(A)\n", None),
    ("DUPLICATE", "one SC row classified twice",
     lambda s: s.replace("- SC-016a — ", "- SC-016a — CRITICAL(C)\n- SC-016a — ", 1), None),
    ("DUPLICATE", "one D record classified twice — dict() used to hide this",
     lambda s: s.replace("- D03 — ", "- D03 — CRITICAL(A,C)\n- D03 — ", 1), None),
    ("CLASS", "a class outside the closed set",
     lambda s: s.replace("- SC-016b — CRITICAL", "- SC-016b — PROBABLY", 1), None),
    ("COUNTS", "a class count that no longer matches the entries",
     lambda s: s.replace("DEFERRABLE=103", "DEFERRABLE=104", 1), None),
    ("COUNTS", "a count key named TWICE while another vanishes (colead's seed)",
     lambda s: s.replace("class_counts: CRITICAL=341 DEFERRABLE=103 OBSERVED=43",
                         "class_counts: CRITICAL=341 CRITICAL=341 DEFERRABLE=103", 1), None),
    ("COUNTS", "a count key simply absent",
     lambda s: s.replace(" OBSERVED=43", "", 1), None),
    ("COUNTS", "a letter count that no longer matches the entries",
     lambda s: s.replace("letter_counts: A=123", "letter_counts: A=124", 1), None),
    ("COUNTS", "the whole letter record removed",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("letter_counts:")), None),
    ("STALE", "the contract having moved since this classification was derived",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}", "contract_blob: " + "a" * 40, s, count=1), None),
    ("MALFORMED", "a contract_blob present but not a blob id — DIFFERENT from absent",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}", "contract_blob: yesterday", s, count=1), None),
    ("FRESHNESS", "the freshness relation removed entirely",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("contract_blob:")), None),
    ("WORKTREE-DRIFT", "non-heading contract prose changed while HEAD and the pin stay fixed "
                       "(colead's seed: the subject is not the pinned object)",
     None, lambda s: s.replace("Bucket 2 —", "Bucket 2  —", 1)),
]

def main():
    rc, ids = run()
    if rc != 0:
        print("ABORT: neutral is not clean — %s" % sorted(ids)); return 1
    print("neutral            rc=0  clean  (tracked file untouched throughout)")
    bad = 0
    crit0 = open(CRIT, encoding="utf-8").read()
    wt0 = open(CONTRACT, encoding="utf-8").read()
    with tempfile.TemporaryDirectory(prefix="rp-ratification-") as tmp:
        for want, why, cf, wf in MUTATIONS:
            cp = wp = None
            delta = 0
            if cf:
                m = cf(crit0)
                if m == crit0:
                    print("%-15s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why)); bad += 1; continue
                delta = sum(1 for l in difflib.unified_diff(crit0.split("\n"), m.split("\n"), n=0)
                            if l[:1] in "+-" and l[:3] not in ("+++", "---"))
                cp = os.path.join(tmp, "crit.md"); open(cp, "w", encoding="utf-8").write(m)
            if wf:
                m = wf(wt0)
                if m == wt0:
                    print("%-15s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why)); bad += 1; continue
                delta = sum(1 for l in difflib.unified_diff(wt0.split("\n"), m.split("\n"), n=0)
                            if l[:1] in "+-" and l[:3] not in ("+++", "---"))
                wp = os.path.join(tmp, "contract.md"); open(wp, "w", encoding="utf-8").write(m)
            rc2, ids2 = run(cp, wp)
            ok = rc2 != 0 and want in ids2
            bad += 0 if ok else 1
            print("%-15s delta=%-3d rc=%d ids=%-22s %s  (%s)"
                  % (want, delta, rc2, ",".join(sorted(ids2))[:22] or "-",
                     "caught" if ok else "<-- MISSED", why))
    rc3, _ = run()
    print("restored           rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
    if rc3 != 0: bad += 1
    print("RED-PROOF: %s" % ("ALL PATHS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
