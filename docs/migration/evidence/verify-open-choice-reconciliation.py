#!/usr/bin/env python3
"""Reconcile the phase-4 open-choice register both directions.

Reads the exact accepted phase-1/2/3 gate blobs and the current contract blob,
extracts every ratified OPEN CHOICE phrase occurrence, and checks the committed
classification table against the closed register. Independent of any phase-4
runner. Does not import obligations.py.

FAIL ids this check can emit:

  STALE-BLOB            a pinned input is no longer the named object
  HEADER                register or occurrences table has the wrong header
  EXTRACT-UNCLASSIFIED  a blob occurrence has no classification row
  CLASS-WITHOUT-EXTRACT a gate/SC phrase row cites a line the extractor did not hit
  OMITTED               a product-output locus has no exact register row
  ORPHAN                a register row is cited by no occurrence/SC-arm
  COMPLETENESS          P2 C13 or P3 C10/C12/C13/C15 is missing
  C10-REGISTERED        P3 C10 is classified as a product-output locus
  HEALTH-IN-GATE        OC-P3-AGENT-HEALTH-TOKEN is carried by a P1/P2/P3 gate hit
  HEALTH-WITHOUT-SC     OC-P3-AGENT-HEALTH-TOKEN is not carried by SC-017r

An unlanded seed is INVALID, never a pass — the red-proof diffs first.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))

P1_BLOB = "8e3c9ec0b031f4947260d4e0327bad562a10fdcd"
P2_BLOB = "29db943aa85319534301332052105ba16df03b4d"
P3_BLOB = "8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c"
REGISTER_BLOB = "2da4fb86933a6b8edee15fd61596d6f53fa6c550"
REGISTER_PATH = "docs/migration/p1-phase4-open-choices.tsv"
OCC_PATH_DEFAULT = os.path.join(HERE, "p1-phase4-open-choice-occurrences.tsv")
CONTRACT_PATH = "docs/migration/semantic-contract.md"
C3_BLOB = "0126d765d57da2f8cbe86e93660362121f96d2f8"
C3_PATH = "docs/migration/evidence/p1-phase4-contract-obligation-reconciliation.md"

PHRASE = re.compile(r"open[\s\-]*choice", re.I)
CRIT = re.compile(r"^(\d+)\.\s+")
H2 = re.compile(r"^##\s+")
SC_HEAD = re.compile(r"^\*\*(SC-\S+)\s")

REGISTER_HEADER = (
    "CHOICE_ID\tAUTHORITY\tSURFACE\tSCOPE_KEY\t"
    "EXCLUDED_COMPARISON_LOCUS\tSTILL_REQUIRED"
)
OCC_HEADER = (
    "OCC_ID\tSOURCE\tBLOB\tLINE\tSPAN\tOWNER\tCLASS\tCHOICE_ID\tLOCUS"
)

GATE_BLOBS = {
    "P1": (P1_BLOB, "docs/migration/p1-phase1-gate.md"),
    "P2": (P2_BLOB, "docs/migration/p1-phase2-gate.md"),
    "P3": (P3_BLOB, "docs/migration/p1-phase3-gate.md"),
}

PRODUCT_CLASSES = {"product-output", "named-member"}
PHRASE_CLASSES = {"internal", "product-output", "named-set", "review-rule"}


def git_blob(path: str, cwd: str = REPO) -> str:
    r = subprocess.run(
        ["git", "rev-parse", f"HEAD:{path}"],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        return ""
    return r.stdout.strip()


def cat_blob(blob: str, cwd: str = REPO) -> list[str]:
    r = subprocess.run(
        ["git", "cat-file", "-p", blob],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        raise RuntimeError(f"git cat-file -p {blob} failed: {r.stderr.strip()}")
    return r.stdout.splitlines()


def extract_gate(source: str, blob: str, cwd: str = REPO) -> list[dict]:
    lines = cat_blob(blob, cwd)
    owner = "PREAMBLE"
    hits: list[dict] = []
    i = 0
    while i < len(lines):
        if H2.match(lines[i]) and lines[i][3:].strip().lower().startswith(
            "phase 3 handoff"
        ):
            owner = "HANDOFF"
        m = CRIT.match(lines[i])
        if m:
            owner = f"C{int(m.group(1))}"
        this = lines[i]
        nxt = lines[i + 1] if i + 1 < len(lines) else ""
        hit_here = bool(PHRASE.search(this))
        hit_split = (
            not hit_here
            and bool(PHRASE.search(this + " " + nxt))
            and not PHRASE.search(nxt)
        )
        if hit_here or hit_split:
            text = this.strip() if hit_here else (this + " " + nxt).strip()
            hits.append(
                {
                    "source": source,
                    "blob": blob,
                    "line": i + 1,
                    "span": 1 if hit_here else 2,
                    "owner": owner,
                    "text": " ".join(text.split()),
                }
            )
            if hit_split:
                i += 1
        i += 1
    return hits


def extract_sc(blob: str, cwd: str = REPO) -> list[dict]:
    lines = cat_blob(blob, cwd)
    sc = None
    hits: list[dict] = []
    for i, this in enumerate(lines):
        m = SC_HEAD.match(this)
        if m:
            sc = m.group(1)
        nxt = lines[i + 1] if i + 1 < len(lines) else ""
        hit_here = bool(PHRASE.search(this))
        hit_split = (
            not hit_here
            and bool(PHRASE.search(this + " " + nxt))
            and not PHRASE.search(nxt)
        )
        if hit_here or hit_split:
            text = this.strip() if hit_here else (this + " " + nxt).strip()
            hits.append(
                {
                    "source": "SC",
                    "blob": blob,
                    "line": i + 1,
                    "span": 1 if hit_here else 2,
                    "owner": sc or "UNKNOWN",
                    "text": " ".join(text.split()),
                }
            )
    return hits


def read_tsv(path: str) -> tuple[str, list[list[str]]]:
    text = open(path, encoding="utf-8").read()
    if not text.endswith("\n"):
        text += "\n"
    rows = [ln.split("\t") for ln in text.splitlines() if ln != ""]
    if not rows:
        return "", []
    return rows[0][0] + ("\t" + "\t".join(rows[0][1:]) if len(rows[0]) > 1 else ""), rows[1:]


def parse_register(path: str) -> tuple[str, dict[str, list[str]]]:
    header, rows = read_tsv(path)
    by_id = {}
    for row in rows:
        if not row or row[0].startswith("#"):
            continue
        by_id[row[0]] = row
    return header, by_id


def parse_occurrences(path: str) -> tuple[str, list[dict]]:
    header, rows = read_tsv(path)
    out = []
    for row in rows:
        if not row or row[0].startswith("#"):
            continue
        # pad to 9
        while len(row) < 9:
            row.append("")
        out.append(
            {
                "occ_id": row[0],
                "source": row[1],
                "blob": row[2],
                "line": int(row[3]),
                "span": int(row[4]),
                "owner": row[5],
                "class": row[6],
                "choice_id": row[7],
                "locus": row[8],
            }
        )
    return header, out


def fail(ids: set[str], code: str, msg: str) -> None:
    ids.add(code)
    print(f"FAIL {code} {msg}")


def verify(
    register_path: str,
    occ_path: str,
    cwd: str = REPO,
    check_register_blob: bool = True,
    contract_blob: str | None = None,
) -> int:
    ids: set[str] = set()

    if check_register_blob:
        got = git_blob(REGISTER_PATH, cwd)
        if got != REGISTER_BLOB:
            fail(
                ids, "STALE-BLOB",
                f"register HEAD blob {got or '(missing)'} != pin {REGISTER_BLOB}",
            )
    got_c3 = git_blob(C3_PATH, cwd)
    if got_c3 != C3_BLOB:
        fail(
            ids, "STALE-BLOB",
            f"C3 recon HEAD blob {got_c3 or '(missing)'} != pin {C3_BLOB}",
        )
    for src, (blob, path) in GATE_BLOBS.items():
        got = git_blob(path, cwd)
        if got != blob:
            fail(
                ids, "STALE-BLOB",
                f"{src} HEAD blob {got or '(missing)'} != pin {blob}",
            )

    if contract_blob is None:
        contract_blob = git_blob(CONTRACT_PATH, cwd)
    if not contract_blob:
        fail(ids, "STALE-BLOB", "contract blob missing")

    header, register = parse_register(register_path)
    if header != REGISTER_HEADER:
        fail(ids, "HEADER", f"register header mismatch: {header!r}")

    occ_header, occs = parse_occurrences(occ_path)
    if occ_header != OCC_HEADER:
        fail(ids, "HEADER", f"occurrences header mismatch: {occ_header!r}")

    extracts: list[dict] = []
    for src, (blob, _path) in GATE_BLOBS.items():
        extracts.extend(extract_gate(src, blob, cwd))
    extracts.extend(extract_sc(contract_blob, cwd))

    extract_keys = {(h["source"], h["line"]) for h in extracts}
    extract_owner = {(h["source"], h["line"]): h["owner"] for h in extracts}

    phrase_rows = [o for o in occs if o["class"] in PHRASE_CLASSES]
    phrase_keys = {(o["source"], o["line"]) for o in phrase_rows}

    for h in extracts:
        if (h["source"], h["line"]) not in phrase_keys:
            fail(
                ids, "EXTRACT-UNCLASSIFIED",
                f"{h['source']} L{h['line']} {h['owner']}: {h['text'][:80]}",
            )

    for o in phrase_rows:
        key = (o["source"], o["line"])
        if key not in extract_keys:
            fail(
                ids, "CLASS-WITHOUT-EXTRACT",
                f"{o['occ_id']} {o['source']} L{o['line']} has no blob occurrence",
            )
        else:
            want = extract_owner[key]
            if o["owner"] != want:
                fail(
                    ids, "CLASS-WITHOUT-EXTRACT",
                    f"{o['occ_id']} owner {o['owner']} != extracted {want}",
                )

    cited: set[str] = set()
    for o in occs:
        cid = o["choice_id"]
        if cid in ("", "-"):
            if o["class"] in PRODUCT_CLASSES:
                fail(
                    ids, "OMITTED",
                    f"{o['occ_id']} class {o['class']} has empty CHOICE_ID",
                )
            continue
        cited.add(cid)
        if o["class"] in PRODUCT_CLASSES or o["class"] == "sc-arm":
            if cid not in register:
                fail(
                    ids, "OMITTED",
                    f"{o['occ_id']} product locus {cid} is not a register row",
                )

    for cid in register:
        if cid not in cited:
            fail(ids, "ORPHAN", f"register row {cid} is cited by no occurrence")

    owners = {(o["source"], o["owner"]) for o in occs}
    for need in (("P2", "C13"), ("P3", "C10"), ("P3", "C12"), ("P3", "C13"), ("P3", "C15")):
        if need not in owners:
            fail(ids, "COMPLETENESS", f"missing {need[0]} {need[1]} in completeness set")

    c10 = [o for o in occs if o["source"] == "P3" and o["owner"] == "C10"]
    if not c10:
        fail(ids, "COMPLETENESS", "P3 C10 missing")
    else:
        for o in c10:
            if o["class"] in PRODUCT_CLASSES or o["choice_id"] not in ("", "-"):
                fail(
                    ids, "C10-REGISTERED",
                    f"{o['occ_id']} P3 C10 must stay unregistered internal",
                )

    health = [o for o in occs if o["choice_id"] == "OC-P3-AGENT-HEALTH-TOKEN"]
    if not any(o["source"] == "SC" and o["owner"] == "SC-017r" for o in health):
        fail(
            ids, "HEALTH-WITHOUT-SC",
            "OC-P3-AGENT-HEALTH-TOKEN is not carried by SC-017r",
        )
    if any(o["source"] in ("P1", "P2", "P3") for o in health):
        fail(
            ids, "HEALTH-IN-GATE",
            "OC-P3-AGENT-HEALTH-TOKEN is carried by a P1/P2/P3 gate occurrence",
        )

    if not ids:
        print(
            f"PASS extracts={len(extracts)} occ_rows={len(occs)} "
            f"register={len(register)} contract={contract_blob[:12]}"
        )
        return 0
    print("ids=%s" % ",".join(sorted(ids)))
    return 1


def dump(cwd: str = REPO) -> int:
    contract_blob = git_blob(CONTRACT_PATH, cwd)
    extracts: list[dict] = []
    for src, (blob, _path) in GATE_BLOBS.items():
        extracts.extend(extract_gate(src, blob, cwd))
    extracts.extend(extract_sc(contract_blob, cwd))
    print(OCC_HEADER)
    for h in extracts:
        print(
            f"{h['source']}-L{h['line']:03d}\t{h['source']}\t{h['blob'][:12]}\t"
            f"{h['line']}\t{h['span']}\t{h['owner']}\t?\t-\t{h['text']}"
        )
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--register", default=os.path.join(REPO, REGISTER_PATH))
    p.add_argument("--occurrences", default=OCC_PATH_DEFAULT)
    p.add_argument("--repo", default=REPO)
    p.add_argument("--contract-blob", default=None)
    p.add_argument(
        "--allow-register-drift",
        action="store_true",
        help="skip the HEAD==pin check on the register (isolated seeds)",
    )
    p.add_argument("--dump", action="store_true")
    args = p.parse_args(argv)
    if args.dump:
        return dump(args.repo)
    return verify(
        args.register,
        args.occurrences,
        cwd=args.repo,
        check_register_blob=not args.allow_register_drift,
        contract_blob=args.contract_blob,
    )


if __name__ == "__main__":
    sys.exit(main())
