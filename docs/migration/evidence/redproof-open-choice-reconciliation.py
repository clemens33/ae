#!/usr/bin/env python3
"""RED-PROOF for verify-open-choice-reconciliation.py.

IT NEVER MUTATES THE TRACKED REGISTER OR OCCURRENCES TABLE. Seeds are written
to an isolated temp directory and the verifier is pointed at them with
--register / --occurrences --allow-register-drift.

Criterion 8 names three failure directions and requires a seed of each:

  OMITTED              a product-output locus with no exact register row
  ORPHAN               a register row with no supporting occurrence
  DUPLICATE-CHOICE-ID  one id claimed by two register rows

The third is the quiet one. A dict keyed by CHOICE_ID keeps the LAST row, so
the survivor wears the dropped row's id, ORPHAN cannot fire because the id is
still present, and the only visible failures land on the HEALTHY occurrence
rows that cite the dropped surface — an investigation pointed at the wrong
artifact. The seed below exists because a guard whose red proof lives only in
a report regresses silently.

A seed that does not land is an INVALID TEST, not a pass.
"""
import difflib
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
VERIFY = os.path.join(HERE, "verify-open-choice-reconciliation.py")
REGISTER = os.path.join(REPO, "docs/migration/p1-phase4-open-choices.tsv")
OCC = os.path.join(HERE, "p1-phase4-open-choice-occurrences.tsv")


def run(register=None, occ=None, allow_drift=False):
    cmd = [sys.executable, VERIFY]
    if register:
        cmd += ["--register", register]
    if occ:
        cmd += ["--occurrences", occ]
    if allow_drift:
        cmd.append("--allow-register-drift")
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO)
    ids = {ln.split()[1] for ln in r.stdout.splitlines() if ln.startswith("FAIL")}
    return r.returncode, ids, r.stdout, r.stderr


def delta_lines(a: str, b: str) -> int:
    return sum(
        1
        for ln in difflib.unified_diff(a.split("\n"), b.split("\n"), n=0)
        if ln[:1] in "+-" and ln[:3] not in ("+++", "---")
    )


def main() -> int:
    rc, ids, out, err = run()
    if rc != 0:
        print("ABORT: neutral is not clean")
        print(out)
        print(err)
        return 1
    print("neutral            rc=0  clean  (tracked files untouched throughout)")

    orig_reg = open(REGISTER, encoding="utf-8").read()
    orig_occ = open(OCC, encoding="utf-8").read()
    bad = 0

    with tempfile.TemporaryDirectory(prefix="rp-open-choice-") as tmp:
        # OMITTED: drop the one register row whose only exclusive product
        # citation includes the C15 named-member for human layout. Deleting
        # OC-P3-HUMAN-LAYOUT leaves P3-C15-HUMAN-LAYOUT without a register row.
        omitted = "\n".join(
            ln for ln in orig_reg.splitlines()
            if not ln.startswith("OC-P3-HUMAN-LAYOUT\t")
        ) + "\n"
        if omitted == orig_reg:
            print("OMITTED          SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_reg, omitted)
            rp = os.path.join(tmp, "register-omitted.tsv")
            open(rp, "w", encoding="utf-8").write(omitted)
            shutil.copy(OCC, os.path.join(tmp, "occ.tsv"))
            rc2, ids2, out2, _ = run(rp, os.path.join(tmp, "occ.tsv"), True)
            ok = rc2 != 0 and "OMITTED" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (dropped OC-P3-HUMAN-LAYOUT)"
                % (
                    "OMITTED",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # ORPHAN: append a register row no occurrence cites.
        orphan_line = (
            "OC-FAKE-ORPHAN\tSC-9999\tJSON stdout\tdigest_all\t"
            "seeded orphan locus\tnothing remains required\n"
        )
        orphan = orig_reg if orig_reg.endswith("\n") else orig_reg + "\n"
        orphan = orphan + orphan_line
        if orphan == orig_reg:
            print("ORPHAN           SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_reg, orphan)
            rp = os.path.join(tmp, "register-orphan.tsv")
            open(rp, "w", encoding="utf-8").write(orphan)
            op = os.path.join(tmp, "occ-orphan.tsv")
            shutil.copy(OCC, op)
            rc2, ids2, out2, _ = run(rp, op, True)
            ok = rc2 != 0 and "ORPHAN" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (added OC-FAKE-ORPHAN)"
                % (
                    "ORPHAN",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # DUPLICATE-CHOICE-ID: give the SECOND data row the FIRST row's id, so
        # one id is claimed by two rows with different surfaces. Positional
        # rather than by name, so register growth cannot silently unland it.
        reg_lines = orig_reg.splitlines()
        dup_text = None
        if len(reg_lines) >= 3:
            first_id = reg_lines[1].split("\t")[0]
            second = reg_lines[2].split("\t")
            second[0] = first_id
            dup_lines = reg_lines[:2] + ["\t".join(second)] + reg_lines[3:]
            dup_text = "\n".join(dup_lines) + "\n"
            claims = sum(1 for ln in dup_lines[1:] if ln.split("\t")[0] == first_id)
        if dup_text is None or dup_text == orig_reg or claims != 2:
            print("DUPLICATE-CHOICE  SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_reg, dup_text)
            rp = os.path.join(tmp, "register-duplicate.tsv")
            open(rp, "w", encoding="utf-8").write(dup_text)
            op = os.path.join(tmp, "occ-duplicate.tsv")
            shutil.copy(OCC, op)
            rc2, ids2, out2, _ = run(rp, op, True)
            ok = rc2 != 0 and "DUPLICATE-CHOICE-ID" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (%s claimed twice)"
                % (
                    "DUPLICATE-CHOICE",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                    first_id,
                )
            )
            if not ok:
                print(out2)

    rc3, ids3, out3, err3 = run()
    print("restored           rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
    if rc3 != 0:
        bad += 1
        print(out3)
        print(err3)

    # tracked bytes must be identical
    if open(REGISTER, encoding="utf-8").read() != orig_reg:
        print("ABORT: tracked register mutated")
        bad += 1
    if open(OCC, encoding="utf-8").read() != orig_occ:
        print("ABORT: tracked occurrences table mutated")
        bad += 1

    print(
        "RED-PROOF: %s"
        % ("ALL THREE DIRECTIONS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad)
    )
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
