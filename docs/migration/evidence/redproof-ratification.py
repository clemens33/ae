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

def run(crit_path=None, wt_path=None, subject_path=None):
    cmd = [sys.executable, VERIFY]
    if crit_path: cmd += ["--file", crit_path]
    if wt_path: cmd += ["--worktree-contract", wt_path]
    if subject_path: cmd += ["--contract-subject", subject_path]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return r.returncode, {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}

# (want, why, crit-mutator, worktree-mutator, contract-SUBJECT-mutator) — all optional
MUTATIONS = [
    ("COVERAGE", "a contract row stripped of its classification entry",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("- SC-017p — ")), None, None),
    ("ORPHAN", "a class assigned to an id that is not a contract row",
     lambda s: s.rstrip("\n") + "\n- SC-9999 — CRITICAL(A)\n", None, None),
    ("DUPLICATE", "one SC row classified twice",
     lambda s: s.replace("- SC-016a — ", "- SC-016a — CRITICAL(C)\n- SC-016a — ", 1), None, None),
    ("DUPLICATE", "one D record classified twice — dict() used to hide this",
     lambda s: s.replace("- D03 — ", "- D03 — CRITICAL(A,C)\n- D03 — ", 1), None, None),
    ("CLASS", "a class outside the closed set",
     lambda s: s.replace("- SC-016b — CRITICAL", "- SC-016b — PROBABLY", 1), None, None),
    ("COUNTS", "a class count that no longer matches the entries",
     lambda s: s.replace("DEFERRABLE=103", "DEFERRABLE=104", 1), None, None),
    # DERIVED, not hard-coded. This seed used to spell the counts literally and
    # silently stopped landing the moment a row was reclassified (found stale at
    # HEAD: it wanted CRITICAL=341 while the registry had moved to 342). The
    # harness reported SEED-DID-NOT-LAND rather than a false pass, which is the
    # design working — but a seed that decays on every re-derivation is a lane
    # that goes quiet exactly when the registry is busiest.
    ("COUNTS", "a count key named TWICE while another vanishes (colead's seed)",
     lambda s: re.sub(
         r"^class_counts: (CRITICAL=\d+) (DEFERRABLE=\d+) (OBSERVED=\d+)$",
         lambda m: "class_counts: %s %s %s" % (m.group(1), m.group(1), m.group(2)),
         s, count=1, flags=re.M), None, None),
    ("COUNTS", "a count key simply absent",
     lambda s: s.replace(" OBSERVED=43", "", 1), None, None),
    ("COUNTS", "a letter count that no longer matches the entries",
     lambda s: s.replace("letter_counts: A=123", "letter_counts: A=124", 1), None, None),
    ("COUNTS", "the whole letter record removed",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("letter_counts:")), None, None),
    ("STALE", "the contract having moved since this classification was derived",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}", "contract_blob: " + "a" * 40, s, count=1), None, None),
    ("MALFORMED", "a contract_blob present but not a blob id — DIFFERENT from absent",
     lambda s: re.sub(r"contract_blob: [0-9a-f]{40}", "contract_blob: yesterday", s, count=1), None, None),
    ("FRESHNESS", "the freshness relation removed entirely",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith("contract_blob:")), None, None),
    ("LETTERS", "a CRITICAL row carrying an invented criterion letter (colead's control)",
     lambda s: s.replace("- SC-400d — CRITICAL(C,D)", "- SC-400d — CRITICAL(C,E)", 1), None, None),
    ("LETTERS", "a CRITICAL row with no letter set at all (colead's control)",
     lambda s: s.replace("- SC-400d — CRITICAL(C,D)", "- SC-400d — CRITICAL", 1), None, None),
    ("LETTERS", "a CRITICAL row repeating a criterion letter",
     lambda s: s.replace("- SC-400d — CRITICAL(C,D)", "- SC-400d — CRITICAL(C,C,D)", 1), None, None),
    ("COUNTS", "a SECOND, contradictory class_counts record (colead's control)",
     lambda s: s.rstrip("\n") + "\nclass_counts: CRITICAL=1 DEFERRABLE=1 OBSERVED=1\n", None, None),
    ("COUNTS", "a SECOND, contradictory letter_counts record",
     lambda s: s.rstrip("\n") + "\nletter_counts: A=1 B=1 C=1 D=1\n", None, None),
    ("WORKTREE-DRIFT", "non-heading contract prose changed while HEAD and the pin stay fixed "
                       "(colead's seed: the subject is not the pinned object)",
     None, lambda s: s.replace("Bucket 2 —", "Bucket 2  —", 1), None),
    ("DUPLICATE-ROW", "a BOLDED EMPHASIS LABEL beginning with an existing row id — a second "
                      "heading for that id, while the set-sized row total stays put",
     None, None, lambda s: s.replace(
         "**SC-017h — the tabular view shows per-agent health",
         "**SC-509b is emphasised here and therefore becomes a heading.** Prose.\n\n"
         "**SC-017h — the tabular view shows per-agent health", 1)),
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
        for want, why, cf, wf, sf in MUTATIONS:
            cp = wp = sp = None
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
            if sf:
                m = sf(wt0)
                if m == wt0:
                    print("%-15s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why)); bad += 1; continue
                delta = sum(1 for l in difflib.unified_diff(wt0.split("\n"), m.split("\n"), n=0)
                            if l[:1] in "+-" and l[:3] not in ("+++", "---"))
                sp = os.path.join(tmp, "subject.md"); open(sp, "w", encoding="utf-8").write(m)
            rc2, ids2 = run(cp, wp, sp)
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
