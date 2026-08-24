#!/usr/bin/env python3
"""RED-PROOF — every check path in verify-obligations.py, both directions.

IT NEVER MUTATES THE TRACKED EVIDENCE FILES. Seeds are written to an isolated
temp directory and the verifier is pointed at them; the shared checkout is only
ever READ. A red-proof that copies, overwrites and restores the subject exposes
seeded bytes to every concurrent reader for the length of the run, and a restore
on the happy path does not close that window.

Neutral must pass; each seeded mutation must be CAUGHT BY ITS OWN NAMED CHECK, with
the seed diffed first. A seed that does not land is an INVALID TEST, not a pass — a
mutation of an absent phrase produces silence indistinguishable from a working check.
"""
import difflib, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")

def run(obl=None, fresh=None, inv=None):
    cmd = [sys.executable, os.path.join(HERE, "verify-obligations.py")]
    for flag, val in (("--obl", obl), ("--fresh", fresh), ("--inv", inv)):
        if val:
            cmd += [flag, val]
    r = subprocess.run(cmd, capture_output=True, text=True)
    ids = {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}
    return r.returncode, ids

MUTATIONS = [
    ("FROM", OBL, "a captured value the table misreports",
     lambda s: s.replace("\tschema_version\t1\t2\t", "\tschema_version\t3\t2\t", 1)),
    # Re-aimed: the SC-017o human diagnostic was the table's only `at-least` and its
    # only `stderr` row, and the entitlement re-derivation removed it. The domain checks
    # for those two values are therefore no longer exercised BY DATA — stated rather
    # than papered over by picking a value that happens to exist.
    ("PREDICATE", OBL, "a predicate outside the closed set",
     lambda s: s.replace("\tpresent\t", "\tvibes\t", 1)),
    ("STREAM", OBL, "a stream outside the closed set",
     lambda s: s.replace("\tdigest\t", "\ttelepathy\t", 1)),
    ("WRONG-KIND", OBL, "a membership obligation where the capture shows a label move",
     lambda s: s.replace("\tSC-017l\tdigest\tsessions[].status\tstopped\tunknown\tall-of\t",
                         "\tSC-017m\tdigest\t(row set)\tempty\tunknown rows present\tpresent\t", 1)),
    ("MISSING-509d", OBL, "a digest row stripped of its schema obligation",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro\tlist-json\tSC-509d")))),
    ("POPULATION", OBL, "an obligation for a row outside the P1 universe it claims to cover",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/ZZ/not-a-case", "list", "SC-509d", "digest", "schema_version", "1", "2",
          "equals", "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    # The label check, red-proved by the ONE mutation a substring matcher cannot fail:
    # relabel a carrying row P1 -> P1-ADJACENT. Exact matching drops it from the universe
    # and its obligations become stray; substring matching still admits it and stays green.
    ("POPULATION", INV, "a carrying row relabelled out of P1 (substring matching would not notice)",
     lambda s: s.replace("arms/A1/c01-healthy-ro/consumers.tsv\tlist-json\t0\tP1\t",
                         "arms/A1/c01-healthy-ro/consumers.tsv\tlist-json\t0\tP1-ADJACENT\t", 1)),
    ("SUPPORT", OBL, "an obligation with no support verdict",
     lambda s: s.replace("\tUNSCORABLE\t", "\tmaybe\t", 1)),
    ("MISSING-509e", OBL, "an unreachable digest stripped of its agent-liveness move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro") and "\tSC-509e\t" in l))),
    ("MISSING-509b", OBL, "a read-loss digest stripped of its degraded move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c02-meta-mode-000-ro\tlist-all-json")
                                 and "\tSC-509b\t" in l))),
    ("MISSING-509c", OBL, "a digest stripped of the reason move its own agent state proves",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A3/c07-competing-rw\t")
                                 and "\tSC-509c\t" in l))),
    ("MISSING-509c", OBL, "an ALERT-derived reason move stripped (evidence class 2)",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not ("\tSC-509c\t" in l and "sessions[twda1].agents[fake:probe]" in l))),
    ("SURFACE", OBL, "a JSON-only obligation parked on a human row",
     lambda s: s.replace("arms/A1/c02-meta-mode-000-ro\tlist-all-json\tSC-509b\t",
                         "arms/A1/c02-meta-mode-000-ro\tlist\tSC-509b\t", 1)),
    # colead's B1: an UNRELATED id adopting the new predicate. A closed-set member is
    # open until something binds who may use it.
    ("UNDECIDABLE", OBL, "an unrelated obligation adopting `undecidable` to launder itself",
     lambda s: re.sub(r"^(arms/A1/c01-healthy-ro\tlist-json\tSC-509d\t[^\n]*?)"
                      r"\tequals\tSOURCE\tOBSERVED\t",
                      r"\1\tundecidable\tSOURCE\tUNSCORABLE\t", s, count=1, flags=re.M)),
    # colead's B2b: the value row's target drifting while its predicate stays.
    ("UNDECIDABLE", OBL, "the completeness-value row with a drifted `to` target",
     lambda s: s.replace("the enumeration's actual completeness", "GARBAGE", 1)),
    ("VALUE-SHAPE", OBL, "the completeness-value locus carrying a scorable predicate",
     lambda s: s.replace("\tinventory_complete (value)\tABSENT\tthe enumeration's actual "
                         "completeness\tundecidable\tOBSERVED\tUNSCORABLE\t",
                         "\tinventory_complete (value)\tABSENT\tthe enumeration's actual "
                         "completeness\tequals\tOBSERVED\tOBSERVED\t", 1)),
    # colead's B2a: set membership where exact arity is owed.
    ("DUPLICATE-017o", OBL, "a digest carrying two completeness-value loci",
     lambda s: re.sub(r"^([^\n]*inventory_complete \(value\)[^\n]*)$", r"\1\n\1",
                      s, count=1, flags=re.M)),
    ("DUPLICATE-ADDRESS", OBL, "any obligation address appearing twice",
     lambda s: re.sub(r"^([^\n]*\tSC-509d\t[^\n]*)$", r"\1\n\1", s, count=1, flags=re.M)),
    ("MISSING-017o-VALUE", OBL, "a digest stripped of its completeness VALUE locus",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro\tlist-json\tSC-017o")
                                 and "inventory_complete (value)" in l))),
    ("STALE", FRESH, "the contract having moved since derivation",
     lambda s: s.replace("contract_blob\t", "contract_blob\tdeadbeef", 1)),
]

def main():
    rc, ids = run()
    if rc != 0:
        print("ABORT: neutral is not clean — %s" % sorted(ids)); return 1
    print("neutral            rc=0  clean")
    bad = 0
    # Every mutation target, read ONCE from the shared checkout and never written.
    originals = {t: open(t, encoding="utf-8").read()
                 for t in {m[1] for m in MUTATIONS}}
    with tempfile.TemporaryDirectory(prefix="rp-obligations-") as tmp:
        for want, target, why, fn in MUTATIONS:
            orig = originals[target]
            mutated = fn(orig)
            if mutated == orig:
                print("%-14s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why))
                bad += 1
                continue
            delta = sum(1 for l in difflib.unified_diff(orig.split("\n"), mutated.split("\n"), n=0)
                        if l[:1] in "+-" and l[:3] not in ("+++", "---"))
            seeded = os.path.join(tmp, os.path.basename(target))
            open(seeded, "w", encoding="utf-8").write(mutated)
            kw = {OBL: "obl", FRESH: "fresh", INV: "inv"}[target]
            kw = {kw: seeded}
            rc2, ids2 = run(**kw)
            ok = rc2 != 0 and want in ids2
            if not ok:
                bad += 1
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
