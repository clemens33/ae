#!/usr/bin/env python3
"""Reconcile the phase-4 open-choice register both directions.

Reads the exact accepted phase-1/2/3 gate blobs and the current contract blob,
extracts every ratified OPEN CHOICE phrase occurrence, and checks the committed
classification table against the closed register. Phrase identity is a normalized,
line-independent bounded token-context SHA-256 plus source and owner; line/blob/span
are provenance only. A Counter preserves multiplicity where a normalized phrase
repeats. Independent of any phase-4 runner. Does not import obligations.py.

FAIL ids this check can emit:

  STALE-BLOB            a pinned input is no longer the named object
  BLOB-PROVENANCE       an occurrence's source object is not a HEAD-reachable blob
  HEADER                register or occurrences table has the wrong header
  MALFORMED-PHRASE-HASH a phrase hash is missing/malformed, or a non-phrase carries one
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
import collections
import hashlib
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))

P1_BLOB = "8e3c9ec0b031f4947260d4e0327bad562a10fdcd"
P2_BLOB = "29db943aa85319534301332052105ba16df03b4d"
P3_BLOB = "8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c"
REGISTER_BLOB = "931773e99e30bea49d0303550cd08d68122a5054"
REGISTER_PATH = "docs/migration/p1-phase4-open-choices.tsv"
OCC_PATH_DEFAULT = os.path.join(HERE, "p1-phase4-open-choice-occurrences.tsv")
CONTRACT_PATH = "docs/migration/semantic-contract.md"
C3_BLOB = "6bf2e7f86c82ba15eb8479cff3b139ce708f15bd"
C3_PATH = "docs/migration/evidence/p1-phase4-contract-obligation-reconciliation.md"

PHRASE = re.compile(r"open[\s\-]*choice", re.I)
TOKEN = re.compile(r"\S+")
CRIT = re.compile(r"^(\d+)\.\s+")
H2 = re.compile(r"^##\s+")
# Must match verify-ratification.py's row definition. In particular, a bolded
# label beginning ``SC-017m-...`` is a second SC-017m heading, not an invented
# distinct owner. The contract avoids that ambiguous label shape altogether.
SC_HEAD = re.compile(r"^\s*(?:- )?\*\*(SC-[0-9]+[a-z]*)\b")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
FINGERPRINT_RADIUS = 12

REGISTER_HEADER = (
    "CHOICE_ID\tAUTHORITY\tSURFACE\tSCOPE_KEY\t"
    "EXCLUDED_COMPARISON_LOCUS\tSTILL_REQUIRED"
)
OCC_HEADER = (
    "OCC_ID\tSOURCE\tBLOB\tLINE\tSPAN\tOWNER\tCLASS\tCHOICE_ID\tLOCUS"
    "\tPHRASE_SHA256"
)

GATE_BLOBS = {
    "P1": (P1_BLOB, "docs/migration/p1-phase1-gate.md"),
    "P2": (P2_BLOB, "docs/migration/p1-phase2-gate.md"),
    "P3": (P3_BLOB, "docs/migration/p1-phase3-gate.md"),
}

# A product-link is a second product locus ratified by an already-counted physical
# phrase (P3-L202b). It remains a citation, but must not counterfeit a second phrase
# occurrence merely because one sentence names two choices.
PRODUCT_CLASSES = {"product-output", "named-member", "product-link"}
PHRASE_CLASSES = {"internal", "product-output", "named-set", "review-rule"}
NON_PHRASE_CLASSES = {"internal-member", "named-member", "product-link", "sc-arm"}
KNOWN_CLASSES = PHRASE_CLASSES | NON_PHRASE_CLASSES


def phrase_sha256(text: str) -> str:
    """Hash one normalized, line-independent phrase-context fingerprint."""
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def git_blob(path: str, cwd: str = REPO) -> str:
    r = subprocess.run(
        ["git", "rev-parse", f"HEAD:{path}"],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        return ""
    return r.stdout.strip()


def head_reachable_objects(cwd: str = REPO) -> set[str]:
    """Return object ids in HEAD ancestry, never merely local scratch refs."""
    r = subprocess.run(
        ["git", "rev-list", "--objects", "HEAD"],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        return set()
    return {line.split(" ", 1)[0] for line in r.stdout.splitlines() if line}


def object_type(object_id: str, cwd: str = REPO) -> str:
    """Return the exact Git object type, or empty when no object can be read."""
    r = subprocess.run(
        ["git", "cat-file", "-t", object_id],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    return r.stdout.strip() if r.returncode == 0 else ""


def cat_blob(blob: str, cwd: str = REPO) -> list[str]:
    r = subprocess.run(
        ["git", "cat-file", "-p", blob],
        cwd=cwd, capture_output=True, text=True, check=False,
    )
    if r.returncode != 0:
        raise RuntimeError(f"git cat-file -p {blob} failed: {r.stderr.strip()}")
    return r.stdout.splitlines()


def normalized_block(rows: list[tuple[int, str]]) -> tuple[str, list[tuple[int, int]]]:
    """Join one owner block exactly as the old extractor normalized whitespace."""
    parts: list[str] = []
    starts: list[tuple[int, int]] = []
    length = 0
    for line, raw in rows:
        part = " ".join(raw.split())
        if not part:
            continue
        if parts:
            length += 1
        starts.append((length, line))
        parts.append(part)
        length += len(part)
    return " ".join(parts), starts


def provenance_line(starts: list[tuple[int, int]], offset: int) -> int:
    """Return a line only as informational provenance, never as identity."""
    line = starts[0][1]
    for start, candidate in starts:
        if start > offset:
            break
        line = candidate
    return line


def extract_block(
    source: str, blob: str, owner: str, rows: list[tuple[int, str]]
) -> list[dict]:
    """Extract line-independent bounded token fingerprints within one owner."""
    text, starts = normalized_block(rows)
    if not text:
        return []
    tokens = list(TOKEN.finditer(text))
    hits = []
    for match in PHRASE.finditer(text):
        matched = [
            n
            for n, token in enumerate(tokens)
            if token.start() < match.end() and token.end() > match.start()
        ]
        if not matched:
            raise RuntimeError("open-choice match did not overlap a normalized token")
        left = max(0, matched[0] - FINGERPRINT_RADIUS)
        right = min(len(tokens), matched[-1] + FINGERPRINT_RADIUS + 1)
        fingerprint = " ".join(token.group(0) for token in tokens[left:right])
        line = provenance_line(starts, match.start())
        end_line = provenance_line(starts, max(match.start(), match.end() - 1))
        hits.append(
            {
                "source": source,
                "blob": blob,
                "line": line,
                "span": end_line - line + 1,
                "owner": owner,
                "text": fingerprint,
                "phrase_sha256": phrase_sha256(fingerprint),
            }
        )
    return hits


def extract_gate(source: str, blob: str, cwd: str = REPO) -> list[dict]:
    owner = "PREAMBLE"
    active: list[tuple[int, str]] = []
    hits: list[dict] = []

    def flush() -> None:
        if active:
            hits.extend(extract_block(source, blob, owner, active))

    for line, raw in enumerate(cat_blob(blob, cwd), start=1):
        next_owner = owner
        if H2.match(raw) and raw[3:].strip().lower().startswith("phase 3 handoff"):
            next_owner = "HANDOFF"
        if m := CRIT.match(raw):
            next_owner = f"C{int(m.group(1))}"
        if next_owner != owner:
            flush()
            active = []
            owner = next_owner
        active.append((line, raw))
    flush()
    return hits


def extract_sc(blob: str, cwd: str = REPO) -> list[dict]:
    owner = "UNKNOWN"
    active: list[tuple[int, str]] = []
    hits: list[dict] = []

    def flush() -> None:
        if active:
            hits.extend(extract_block("SC", blob, owner, active))

    for line, raw in enumerate(cat_blob(blob, cwd), start=1):
        next_owner = owner
        if m := SC_HEAD.match(raw):
            next_owner = m.group(1)
        if next_owner != owner:
            flush()
            active = []
            owner = next_owner
        active.append((line, raw))
    flush()
    return hits


def read_tsv(path: str) -> tuple[str, list[list[str]]]:
    text = open(path, encoding="utf-8").read()
    if not text.endswith("\n"):
        text += "\n"
    rows = [ln.split("\t") for ln in text.splitlines() if ln != ""]
    if not rows:
        return "", []
    return rows[0][0] + ("\t" + "\t".join(rows[0][1:]) if len(rows[0]) > 1 else ""), rows[1:]


def parse_register(path: str) -> tuple[str, dict[str, list[str]], list[tuple[list[str], list[str]]]]:
    """Parse the register, and report ids claimed more than once.

    A dict keyed by CHOICE_ID silently keeps the LAST row for a repeated id, so
    a duplicate does not merely lose a row: the surviving row wears the dropped
    row's id, ORPHAN cannot fire because the id is still present, and the only
    visible failures land on the healthy occurrence rows that cite the dropped
    surface.  Returning the collisions lets the caller name the real defect.
    """
    header, rows = read_tsv(path)
    by_id: dict[str, list[str]] = {}
    duplicates: list[tuple[list[str], list[str]]] = []
    for row in rows:
        if not row or row[0].startswith("#"):
            continue
        if row[0] in by_id:
            duplicates.append((by_id[row[0]], row))
        by_id[row[0]] = row
    return header, by_id, duplicates


def parse_occurrences(path: str) -> tuple[str, list[dict], list[str]]:
    header, rows = read_tsv(path)
    out = []
    malformed = []
    for n, row in enumerate(rows, start=2):
        if not row or row[0].startswith("#"):
            continue
        if len(row) != 10:
            malformed.append(f"line {n} has {len(row)} column(s), expected 10")
            continue
        try:
            line = int(row[3])
            span = int(row[4])
        except ValueError:
            malformed.append(f"line {n} has non-integer LINE or SPAN")
            continue
        out.append(
            {
                "occ_id": row[0],
                "source": row[1],
                "blob": row[2],
                "line": line,
                "span": span,
                "owner": row[5],
                "class": row[6],
                "choice_id": row[7],
                "locus": row[8],
                "phrase_sha256": row[9],
            }
        )
    return header, out, malformed


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

    header, register, register_duplicates = parse_register(register_path)
    for first, later in register_duplicates:
        fail(
            ids, "DUPLICATE-CHOICE-ID",
            f"register id {first[0]} is claimed twice: "
            f"{first[2] if len(first) > 2 else '?'}/{first[3] if len(first) > 3 else '?'} "
            f"then {later[2] if len(later) > 2 else '?'}/{later[3] if len(later) > 3 else '?'}; "
            "the later row REPLACED the earlier one under that id, so any "
            "OMITTED or ORPHAN line for it is a consequence of this defect, "
            "not a defect in the occurrences table",
        )
    if header != REGISTER_HEADER:
        fail(ids, "HEADER", f"register header mismatch: {header!r}")

    occ_header, occs, malformed_rows = parse_occurrences(occ_path)
    if occ_header != OCC_HEADER:
        fail(ids, "HEADER", f"occurrences header mismatch: {occ_header!r}")
    for msg in malformed_rows:
        fail(ids, "MALFORMED-PHRASE-HASH", f"occurrence {msg}")

    # BLOB, LINE and SPAN are provenance, not phrase identity, but provenance
    # still must be reproducible. Gate rows name their exact accepted source;
    # SC rows may cite a historical contract blob because identity
    # is source/owner/phrase hash, not a mutable line pointer. Every such SC
    # object must be a blob reachable from HEAD ancestry. `--all` is too broad:
    # a local scratch ref is not guaranteed in a clone of the pushed branch;
    # reachability alone is too broad because commits and trees are not blobs.
    expected_gate_blobs = {src: blob for src, (blob, _path) in GATE_BLOBS.items()}
    head_reachable = head_reachable_objects(cwd)
    for o in occs:
        source = o["source"]
        blob = o["blob"]
        kind = object_type(blob, cwd)
        if kind != "blob":
            fail(
                ids, "BLOB-PROVENANCE",
                f"{o['occ_id']} BLOB {blob[:12]} has type {kind or '(unreadable)'}, expected blob",
            )
            continue
        if source in expected_gate_blobs:
            if blob != expected_gate_blobs[source]:
                fail(
                    ids, "BLOB-PROVENANCE",
                    f"{o['occ_id']} {source} blob {blob[:12]} != accepted "
                    f"{expected_gate_blobs[source][:12]}",
                )
        elif source == "SC":
            if blob not in head_reachable:
                fail(
                    ids, "BLOB-PROVENANCE",
                    f"{o['occ_id']} SC blob {blob[:12]} is not reachable from HEAD ancestry",
                )
        else:
            fail(ids, "BLOB-PROVENANCE", f"{o['occ_id']} has unknown source {source!r}")

    extracts: list[dict] = []
    for src, (blob, _path) in GATE_BLOBS.items():
        extracts.extend(extract_gate(src, blob, cwd))
    extracts.extend(extract_sc(contract_blob, cwd))

    phrase_rows = []
    for o in occs:
        if o["class"] not in KNOWN_CLASSES:
            fail(
                ids,
                "MALFORMED-PHRASE-HASH",
                f"{o['occ_id']} has unknown occurrence class {o['class']!r}",
            )
            continue
        is_phrase = o["class"] in PHRASE_CLASSES
        actual = o["phrase_sha256"]
        if is_phrase:
            if not SHA256.fullmatch(actual):
                fail(
                    ids, "MALFORMED-PHRASE-HASH",
                    f"{o['occ_id']} phrase row needs lowercase 64-hex SHA-256, got {actual!r}",
                )
                continue
            phrase_rows.append(o)
        elif actual != "-":
            fail(
                ids, "MALFORMED-PHRASE-HASH",
                f"{o['occ_id']} non-phrase row must carry '-', got {actual!r}",
            )

    extract_counter = collections.Counter(
        (h["source"], h["owner"], h["phrase_sha256"]) for h in extracts
    )
    phrase_counter = collections.Counter(
        (o["source"], o["owner"], o["phrase_sha256"]) for o in phrase_rows
    )
    extract_by_key = {
        (h["source"], h["owner"], h["phrase_sha256"]): h for h in extracts
    }
    rows_by_key = collections.defaultdict(list)
    for o in phrase_rows:
        rows_by_key[(o["source"], o["owner"], o["phrase_sha256"])].append(o)

    for key, n in sorted((extract_counter - phrase_counter).items()):
        h = extract_by_key[key]
        fail(
            ids, "EXTRACT-UNCLASSIFIED",
            f"{key[0]} {key[1]} sha256={key[2]} missing={n}: {h['text'][:80]}",
        )

    for key, n in sorted((phrase_counter - extract_counter).items()):
        o = rows_by_key[key][0]
        fail(
            ids, "CLASS-WITHOUT-EXTRACT",
            f"{o['occ_id']} {key[0]} {key[1]} sha256={key[2]} extra={n}; "
            f"provenance=L{o['line']}/{o['blob'][:12]}",
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
            f"{h['line']}\t{h['span']}\t{h['owner']}\t?\t-\t{h['text']}\t"
            f"{h['phrase_sha256']}"
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
