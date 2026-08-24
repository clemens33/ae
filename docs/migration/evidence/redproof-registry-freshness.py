#!/usr/bin/env python3
"""RED-PROOF for verify-registry-freshness.py — one stale seed PER PIN, plus the
shape checks.

Per pin, not one for the set: the whole point of direct provenance is that the
relations are independent, so a red-proof with a single stale seed would prove only
that SOME pin is read. Each registry gets its own, and each must fail alone.

Seeds are written to an isolated temp directory and the gate is pointed at it with
--dir. The tracked files are only ever READ.
"""
import os, re, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
VERIFY = os.path.join(HERE, "verify-registry-freshness.py")
DECLARING = ("closure-map.md", "crit-assign.md")


def run(d):
    r = subprocess.run([sys.executable, VERIFY, "--dir", d], capture_output=True, text=True)
    return r.returncode, {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}


def stale(name):
    return name, lambda s: re.sub(r"(SOURCE:\s+\S+\s+blob\s+)[0-9a-f]{40}",
                                  lambda m: m.group(1) + "b" * 40, s, count=1)


MUTATIONS = [
    ("STALE", "closure-map.md", "the contract moved under the closure map",
     stale("closure-map.md")[1]),
    ("STALE", "crit-assign.md", "the CLASSIFICATION moved under crit-assign — a pin to the "
     "contract would have called this fresh",
     stale("crit-assign.md")[1]),
    ("MALFORMED", "closure-map.md", "a pin present but not a blob id — different from absent",
     lambda s: re.sub(r"(SOURCE:\s+\S+\s+blob\s+)[0-9a-f]{40}",
                      lambda m: m.group(1) + "last-tuesday", s, count=1)),
    ("SOURCE", "crit-assign.md", "a declared source git cannot resolve at HEAD",
     lambda s: s.replace("SOURCE: docs/migration/evidence/ratification-critical.md",
                         "SOURCE: docs/migration/evidence/no-such-registry.md", 1)),
    ("MULTIPLE", "closure-map.md", "two declarations, so which one is the relation",
     lambda s: s.replace("SOURCE: ", "SOURCE: docs/migration/ownership.md blob "
                         + "c" * 40 + "\nSOURCE: ", 1)),
]


def main():
    with tempfile.TemporaryDirectory(prefix="rp-registry-") as base:
        clean = os.path.join(base, "clean")
        os.mkdir(clean)
        originals = {}
        for n in DECLARING:
            shutil.copy(os.path.join(HERE, n), os.path.join(clean, n))
            originals[n] = open(os.path.join(HERE, n), encoding="utf-8").read()
        rc, ids = run(clean)
        if rc != 0:
            print("ABORT: neutral is not clean — %s" % sorted(ids))
            return 1
        print("neutral        rc=0  clean  (tracked files only ever read)")
        bad = 0
        for want, target, why, fn in MUTATIONS:
            mutated = fn(originals[target])
            if mutated == originals[target]:
                print("%-10s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why))
                bad += 1
                continue
            d = tempfile.mkdtemp(prefix="seed-", dir=base)
            for n in DECLARING:
                open(os.path.join(d, n), "w", encoding="utf-8").write(
                    mutated if n == target else originals[n])
            rc2, ids2 = run(d)
            ok = rc2 != 0 and want in ids2
            bad += 0 if ok else 1
            print("%-10s %-16s rc=%d ids=%-20s %s  (%s)"
                  % (want, target, rc2, ",".join(sorted(ids2))[:20] or "-",
                     "caught" if ok else "<-- MISSED", why))
        rc3, _ = run(clean)
        print("restored       rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
        bad += 0 if rc3 == 0 else 1
    print("RED-PROOF: %s" % ("ALL PATHS PROVEN BY NAMED CHECK" if bad == 0
                             else "%d FAILURE(S)" % bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
