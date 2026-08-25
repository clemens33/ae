#!/usr/bin/env python3
"""RED-PROOF for verify-open-choice-reconciliation.py.

IT NEVER MUTATES THE TRACKED REGISTER, OCCURRENCES TABLE, OR CONTRACT. Table seeds
are written to an isolated temp directory. Contract seeds are immutable loose blobs,
passed explicitly to the verifier; neither seed writes a tracked path.

Criterion 8 names three failure directions and requires a seed of each:

  OMITTED              a product-output locus with no exact register row
  ORPHAN               a register row with no supporting occurrence
  DUPLICATE-CHOICE-ID  one id claimed by two register rows
  MALFORMED-PHRASE-HASH a phrase row is missing its normalized SHA-256
  BLOB-PROVENANCE      loose, non-blob, and scratch-ref objects are refused
  REFLOW-NEUTRAL       the same classified phrase moves across a Markdown wrap
  EXTRACT-UNCLASSIFIED a classified phrase changed under the same source/owner
  CLASS-WITHOUT-EXTRACT the previous classified phrase remains without an extract

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
CONTRACT_PATH = "docs/migration/semantic-contract.md"


def run(register=None, occ=None, allow_drift=False, contract_blob=None, repo=REPO):
    cmd = [sys.executable, VERIFY]
    if register:
        cmd += ["--register", register]
    if occ:
        cmd += ["--occurrences", occ]
    if contract_blob:
        cmd += ["--contract-blob", contract_blob]
    if allow_drift:
        cmd.append("--allow-register-drift")
    cmd += ["--repo", repo]
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=repo)
    ids = {ln.split()[1] for ln in r.stdout.splitlines() if ln.startswith("FAIL")}
    return r.returncode, ids, r.stdout, r.stderr


def head_contract() -> str:
    r = subprocess.run(
        ["git", "show", f"HEAD:{CONTRACT_PATH}"],
        capture_output=True, text=True, cwd=REPO,
    )
    if r.returncode != 0:
        raise RuntimeError(f"cannot read HEAD contract: {r.stderr.strip()}")
    return r.stdout


def write_contract_blob(text: str, cwd=REPO) -> str:
    r = subprocess.run(
        ["git", "hash-object", "-w", "--stdin"], input=text,
        capture_output=True, text=True, cwd=cwd,
    )
    if r.returncode != 0:
        raise RuntimeError(f"cannot write isolated contract blob: {r.stderr.strip()}")
    return r.stdout.strip()


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

        # MALFORMED-PHRASE-HASH: poison the first classified phrase hash. This is
        # deliberately a valid TSV edit so the hash guard, not row parsing, fires.
        occ_lines = orig_occ.splitlines()
        bad_hash_text = None
        for i, line in enumerate(occ_lines[1:], start=1):
            fields = line.split("\t")
            if len(fields) == 10 and fields[6] in {
                "internal", "product-output", "named-set", "review-rule"
            }:
                fields[9] = "not-a-sha256"
                bad_hash_text = "\n".join(occ_lines[:i] + ["\t".join(fields)] + occ_lines[i + 1:]) + "\n"
                break
        if bad_hash_text is None or bad_hash_text == orig_occ:
            print("MALFORMED-HASH   SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_occ, bad_hash_text)
            op = os.path.join(tmp, "occ-malformed-hash.tsv")
            open(op, "w", encoding="utf-8").write(bad_hash_text)
            rc2, ids2, out2, _ = run(occ=op)
            ok = rc2 != 0 and "MALFORMED-PHRASE-HASH" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (poisoned one phrase hash)"
                % (
                    "MALFORMED-HASH",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # BLOB-PROVENANCE: a loose object can be readable locally without being
        # reproducible from a clone.  Replace one SC provenance value with a
        # newly-written unrelated loose blob; the source/owner/hash remain
        # otherwise intact, so only the provenance guard can reject this seed.
        loose_blob = write_contract_blob("C8 unreachable provenance seed.\n")
        loose_provenance = None
        for i, line in enumerate(occ_lines[1:], start=1):
            fields = line.split("\t")
            if len(fields) == 10 and fields[1] == "SC":
                fields[2] = loose_blob
                loose_provenance = "\n".join(
                    occ_lines[:i] + ["\t".join(fields)] + occ_lines[i + 1:]
                ) + "\n"
                break
        landed = subprocess.run(
            ["git", "cat-file", "-t", loose_blob], capture_output=True,
            text=True, cwd=REPO, check=False,
        ).stdout.strip() == "blob"
        if loose_provenance is None or loose_provenance == orig_occ or not landed:
            print("BLOB-PROVENANCE SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_occ, loose_provenance)
            op = os.path.join(tmp, "occ-loose-provenance.tsv")
            open(op, "w", encoding="utf-8").write(loose_provenance)
            rc2, ids2, out2, _ = run(occ=op)
            ok = rc2 != 0 and "BLOB-PROVENANCE" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (readable loose SC blob)"
                % (
                    "BLOB-PROVENANCE",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # BLOB-PROVENANCE (reachable non-blob): commit and tree ids appear in
        # `rev-list --objects HEAD` too. A field named BLOB must reject the
        # reachable HEAD tree rather than accepting reachability alone.
        head_tree = subprocess.run(
            ["git", "rev-parse", "HEAD^{tree}"], capture_output=True,
            text=True, cwd=REPO, check=False,
        ).stdout.strip()
        tree_provenance = None
        for i, line in enumerate(occ_lines[1:], start=1):
            fields = line.split("\t")
            if len(fields) == 10 and fields[1] == "SC":
                fields[2] = head_tree
                tree_provenance = "\n".join(
                    occ_lines[:i] + ["\t".join(fields)] + occ_lines[i + 1:]
                ) + "\n"
                break
        head_objects = set(
            subprocess.run(
                ["git", "rev-list", "--objects", "HEAD"], capture_output=True,
                text=True, cwd=REPO, check=False,
            ).stdout.split()
        )
        tree_landed = (
            head_tree in head_objects
            and subprocess.run(
                ["git", "cat-file", "-t", head_tree], capture_output=True,
                text=True, cwd=REPO, check=False,
            ).stdout.strip() == "tree"
        )
        if tree_provenance is None or tree_provenance == orig_occ or not tree_landed:
            print("REACHABLE-NON-BLOB SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_occ, tree_provenance)
            op = os.path.join(tmp, "occ-tree-provenance.tsv")
            open(op, "w", encoding="utf-8").write(tree_provenance)
            rc2, ids2, out2, _ = run(occ=op)
            ok = rc2 != 0 and "BLOB-PROVENANCE" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (reachable HEAD tree)"
                % (
                    "REACHABLE-NON-BLOB",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # BLOB-PROVENANCE (local ref): use a throwaway clone so the red proof
        # never creates a scratch ref in the shared repository. The planted
        # object is a real blob and is visible to `--all` through the local ref,
        # but not to `rev-list --objects HEAD`; a fresh clone of HEAD would not
        # inherit that ref.
        clone = os.path.join(tmp, "local-ref-clone")
        cloned = subprocess.run(
            ["git", "clone", "--quiet", "--no-hardlinks", REPO, clone],
            capture_output=True, text=True, check=False,
        ).returncode == 0
        local_provenance = None
        local_landed = False
        if cloned:
            local_blob = write_contract_blob("C8 local-ref-only provenance seed.\n", clone)
            ref = "refs/c8-redproof/local-only"
            updated = subprocess.run(
                ["git", "update-ref", ref, local_blob], capture_output=True,
                text=True, cwd=clone, check=False,
            ).returncode == 0
            head_only = set(
                subprocess.run(
                    ["git", "rev-list", "--objects", "HEAD"], capture_output=True,
                    text=True, cwd=clone, check=False,
                ).stdout.split()
            )
            all_refs = set(
                subprocess.run(
                    ["git", "rev-list", "--objects", "--all"], capture_output=True,
                    text=True, cwd=clone, check=False,
                ).stdout.split()
            )
            local_landed = (
                updated
                and local_blob not in head_only
                and local_blob in all_refs
                and subprocess.run(
                    ["git", "cat-file", "-t", local_blob], capture_output=True,
                    text=True, cwd=clone, check=False,
                ).stdout.strip() == "blob"
            )
            for i, line in enumerate(occ_lines[1:], start=1):
                fields = line.split("\t")
                if len(fields) == 10 and fields[1] == "SC":
                    fields[2] = local_blob
                    local_provenance = "\n".join(
                        occ_lines[:i] + ["\t".join(fields)] + occ_lines[i + 1:]
                    ) + "\n"
                    break
        if local_provenance is None or local_provenance == orig_occ or not local_landed:
            print("LOCAL-REF-ONLY  SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            d = delta_lines(orig_occ, local_provenance)
            op = os.path.join(tmp, "occ-local-ref-provenance.tsv")
            open(op, "w", encoding="utf-8").write(local_provenance)
            rc2, ids2, out2, _ = run(occ=op, repo=clone)
            ok = rc2 != 0 and "BLOB-PROVENANCE" in ids2
            bad += 0 if ok else 1
            print(
                "%-16s delta=%-3d rc=%d ids=%-22s %s  (blob under local-only ref)"
                % (
                    "LOCAL-REF-ONLY",
                    d,
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught" if ok else "<-- MISSED",
                )
            )
            if not ok:
                print(out2)

        # LINE-INSERTION: source line numbers are provenance only. Insert a
        # non-phrase line immediately above SC-405g, verify the seed blob carries
        # it, then require the complete checker to stay neutral.
        original_contract = head_contract()
        anchor = "**SC-405g"
        marker = "C8 red-proof insertion: non-phrase provenance shift.\n\n"
        inserted_contract = original_contract.replace(anchor, marker + anchor, 1)
        if original_contract.count(anchor) != 1 or inserted_contract == original_contract:
            print("LINE-INSERTION   SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            blob = write_contract_blob(inserted_contract)
            landed = "C8 red-proof insertion" in "\n".join(
                subprocess.run(
                    ["git", "cat-file", "-p", blob], capture_output=True,
                    text=True, cwd=REPO, check=False,
                ).stdout.splitlines()
            )
            rc2, ids2, out2, _ = run(contract_blob=blob)
            ok = landed and rc2 == 0 and not ids2
            bad += 0 if ok else 1
            print(
                "%-16s rc=%d ids=%-22s %s  (inserted above SC-405g)"
                % (
                    "LINE-INSERTION",
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "neutral" if ok else "<-- REGRESSED",
                )
            )
            if not ok:
                print(out2)

        # REFLOW-NEUTRAL: move one classified phrase across a Markdown line boundary.
        # The words and owner remain exactly equal, so a physical-line fingerprint
        # would regress while the bounded token-context fingerprint stays neutral.
        old_phrase = "alive/dead/unknown words or glyphs are OPEN CHOICE"
        refolded_phrase = "alive/dead/unknown words or glyphs are OPEN\nCHOICE"
        refolded_contract = original_contract.replace(old_phrase, refolded_phrase, 1)
        if original_contract.count(old_phrase) != 1 or refolded_contract == original_contract:
            print("REFLOW-NEUTRAL   SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            blob = write_contract_blob(refolded_contract)
            landed = refolded_phrase in subprocess.run(
                ["git", "cat-file", "-p", blob], capture_output=True,
                text=True, cwd=REPO, check=False,
            ).stdout
            rc2, ids2, out2, _ = run(contract_blob=blob)
            ok = landed and rc2 == 0 and not ids2
            bad += 0 if ok else 1
            print(
                "%-16s rc=%d ids=%-22s %s  (moved SC-017r phrase across a wrap)"
                % (
                    "REFLOW-NEUTRAL",
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "neutral" if ok else "<-- REGRESSED",
                )
            )
            if not ok:
                print(out2)

        # NEARBY-WORD-DRIFT: change a semantic word inside that same bounded
        # context. The old occurrence must become extra and the new extraction
        # missing, demonstrating both Counter directions after the reflow control.
        new_phrase = "alive/dead/unknown tokens or glyphs are OPEN CHOICE"
        drift_contract = original_contract.replace(old_phrase, new_phrase, 1)
        if original_contract.count(old_phrase) != 1 or drift_contract == original_contract:
            print("NEARBY-WORD-DRIFT SEED-DID-NOT-LAND — invalid test, NOT a pass")
            bad += 1
        else:
            blob = write_contract_blob(drift_contract)
            landed = new_phrase in subprocess.run(
                ["git", "cat-file", "-p", blob], capture_output=True,
                text=True, cwd=REPO, check=False,
            ).stdout
            rc2, ids2, out2, _ = run(contract_blob=blob)
            want = {"EXTRACT-UNCLASSIFIED", "CLASS-WITHOUT-EXTRACT"}
            ok = landed and rc2 != 0 and want <= ids2
            bad += 0 if ok else 1
            print(
                "%-16s rc=%d ids=%-22s %s  (changed nearby SC-017r word)"
                % (
                    "NEARBY-WORD-DRIFT",
                    rc2,
                    ",".join(sorted(ids2))[:22] or "-",
                    "caught-both" if ok else "<-- MISSED",
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
        % ("ALL TEN DIRECTIONS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad)
    )
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
