#!/usr/bin/env python3
"""Independent P1 contract-to-obligation reconciliation gate.

This program deliberately reads only the contract, raw corpus artifacts, this
reconciliation inventory, and OBLIGATIONS.tsv.  It neither imports nor invokes
the obligation-table generator.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path, PurePosixPath


CONTRACT_BLOB = "896d08ea3ac753095c04af17dfba92cd9d15fb38"
EXPECTED_IDS = {
    "SC-017l",
    "SC-017m",
    "SC-017o",
    "SC-017r",
    "SC-509d",
    "SC-509e",
}
EXPECTED_COUNTS = {
    "SC-017l": 134,
    "SC-017m": 150,
    "SC-017o": 573,
    "SC-017r": 78,
    "SC-509d": 401,
    "SC-509e": 42,
}
INVENTORY_LOCUS_COUNTS = {
    ("SC-017l", "unknown session status"): 134,
    ("SC-017m", "unknown session membership and rendering"): 150,
    ("SC-017o", "inventory completeness JSON field and human diagnostic"): 573,
    ("SC-017r", "human agent-health marker"): 78,
    ("SC-509", "generated_at field presence/type"): 401,
    ("SC-509", "generated_at VALUE"): 401,
    ("SC-509", "other retained version-1 object fields"): 401,
    ("SC-509d", "schema_version"): 401,
    ("SC-509e", "agents[].alive nullable domain"): 42,
}
INPUT_CARRIER_IDS = {
    ("SC-017p", "positive per-agent liveness proof"): "SC-017r,SC-509e",
    ("SC-017q", "unknown per-agent liveness"): "SC-017r,SC-509e",
    ("SC-017s", "pane live predicate"): "SC-017r,SC-509e",
    ("SC-405l", "selector normalization"): "SC-017l,SC-017m",
}

INVENTORY_HEADER = [
    "contract_id",
    "contract_locus",
    "p1_disposition",
    "obligation_ids",
    "corpus_loci",
    "independent_raw_basis",
    "pinned_successor",
]
INVENTORY_IDS = {
    "SC-017a",
    "SC-017b",
    "SC-017c",
    "SC-017d",
    "SC-017e",
    "SC-017f",
    "SC-017g",
    "SC-017h",
    "SC-017i",
    "SC-017j",
    "SC-017k",
    "SC-017l",
    "SC-017m",
    "SC-017n",
    "SC-017o",
    "SC-017p",
    "SC-017q",
    "SC-017r",
    "SC-017s",
    "SC-021",
    "SC-400d",
    "SC-506",
    "SC-405l",
    "SC-509",
    "SC-509d",
    "SC-509e",
    "SC-518",
    "SC-521c",
    "SC-1306a",
    "SC-1306d",
    "SC-1306e",
}


def git_blob(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def read_tsv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def failure(errors: list[str], message: str) -> None:
    errors.append(message)


def case_dir(root: Path, invocation: dict[str, str]) -> Path:
    return root / "docs/migration/evidence/batch-c-artifacts" / PurePosixPath(
        invocation["case"]
    ).parent


def is_listing(invocation: dict[str, str]) -> bool:
    return invocation["surface"] in {"ae list", "ae ls"}


def argv_words(invocation: dict[str, str]) -> set[str]:
    return set(invocation["normalised_argv"].split())


def is_json(invocation: dict[str, str]) -> bool:
    return "--json" in argv_words(invocation)


def is_all(invocation: dict[str, str]) -> bool:
    return "--all" in argv_words(invocation)


def has_captured_transport_failure(root: Path, invocation: dict[str, str]) -> bool:
    """True only for recorded failure evidence; false never claims reachability."""
    transcript = case_dir(root, invocation) / "tmux.before.txt"
    return transcript.exists() and "error connecting" in transcript.read_text(
        errors="replace"
    )


def live_selector_missing(root: Path, invocation: dict[str, str]) -> bool:
    """Raw, narrow selector-missing arm; separate from failed server transport."""
    case = (case_dir(root, invocation) / "case.txt").read_text(errors="replace")
    template = next(
        (word.removeprefix("template=") for word in case.split() if word.startswith("template=")),
        None,
    )
    if template is None or "/" not in template:
        return False
    family, fixture_name = template.split("/", 1)
    template_meta = root / "docs/migration/evidence/batch-c-artifacts/templates" / family / "_meta"
    modes = template_meta / f"{fixture_name}.modes.tsv"
    mutation = template_meta / f"{fixture_name}.mutation.txt"
    modes_text = modes.read_text(errors="replace") if modes.is_file() else ""
    mutation_text = mutation.read_text(errors="replace") if mutation.is_file() else ""
    unreadable = "UNREADABLE" in modes_text and "/meta" in modes_text
    absent = "meta file" in mutation_text and "FILE ABSENT" in mutation_text
    return (
        "live_topology=yes" in case
        and (case_dir(root, invocation) / "tmux.before.txt").is_file()
        and not has_captured_transport_failure(root, invocation)
        and (unreadable or absent)
    )


def captured_stdout(root: Path, invocation: dict[str, str]) -> str:
    path = case_dir(root, invocation) / "out" / f"{invocation['consumer']}.stdout"
    return path.read_text(errors="replace")


def has_rendered_agent(root: Path, invocation: dict[str, str]) -> bool:
    """Read the frozen human product row, not meta or the obligation table.

    This is deliberately bounded to this corpus's renderer: agent rows begin
    with two spaces and their first token is the ``alias:name`` identity.
    """
    for line in captured_stdout(root, invocation).splitlines():
        first = line.lstrip().split(maxsplit=1)
        if line.startswith("  ") and first and ":" in first[0]:
            return True
    return False


def has_json_agent(root: Path, invocation: dict[str, str]) -> bool:
    document = json.loads(captured_stdout(root, invocation))
    return any(session.get("agents") for session in document.get("sessions", []))


def expected_loci(root: Path, p1: list[dict[str, str]]) -> set[tuple[str, str, str, str, str]]:
    """A fresh contract reading expressed as raw-corpus selection, not generator reuse."""
    result: set[tuple[str, str, str, str, str]] = set()
    for invocation in p1:
        if not is_listing(invocation):
            continue

        case = PurePosixPath(invocation["case"]).parent.as_posix()
        consumer = invocation["consumer"]
        json_output = is_json(invocation)
        stream = "digest" if json_output else "stdout"
        failed_server = has_captured_transport_failure(root, invocation)

        # SC-509d: every P1 JSON list/ls digest moves to schema version 2.
        if json_output:
            result.add((case, consumer, "SC-509d", "digest", "schema_version"))

        # SC-017o: every JSON digest gains completeness; a failed enumeration
        # makes every P1 human list/ls output carry the diagnostic.
        if json_output:
            result.add((case, consumer, "SC-017o", "digest", "inventory_complete"))
        elif failed_server:
            result.add((case, consumer, "SC-017o", "stderr", "(whole stream)"))

        # SC-017l: an unavailable server changes the all-view status in either
        # rendering.  It intentionally excludes the selector-missing live arm.
        if failed_server and is_all(invocation):
            locus = "sessions[].status" if json_output else "status cell"
            result.add((case, consumer, "SC-017l", stream, locus))

        # SC-017m has two independent product arms: failed server default view,
        # and selected live topology with missing durable selector.
        words = argv_words(invocation)
        if failed_server and not is_all(invocation) and "--stopped" not in words:
            result.add((case, consumer, "SC-017m", stream, "(row set)"))
        if live_selector_missing(root, invocation):
            result.add((case, consumer, "SC-017m", stream, "unknown row present"))

        # SC-017r/e project an unknown agent only when the frozen product shows
        # an agent grain.  Human and JSON evidence are independently parsed.
        if failed_server and is_all(invocation) and not json_output and has_rendered_agent(
            root, invocation
        ):
            result.add((case, consumer, "SC-017r", "stdout", "agent health marker"))
        if failed_server and is_all(invocation) and json_output and has_json_agent(
            root, invocation
        ):
            result.add((case, consumer, "SC-509e", "digest", "agents[].alive"))
    return result


def verify_inventory(path: Path, contract: str, errors: list[str]) -> None:
    with path.open(newline="") as handle:
        reader = csv.DictReader(handle, delimiter="\t")
        if reader.fieldnames != INVENTORY_HEADER:
            failure(errors, f"inventory header drift: {reader.fieldnames!r}")
            return
        rows = list(reader)

    seen = {row["contract_id"] for row in rows}
    if seen != INVENTORY_IDS:
        failure(errors, f"inventory contract IDs differ: expected {sorted(INVENTORY_IDS)}, got {sorted(seen)}")
    generated = [row for row in rows if row["contract_id"] == "SC-509" and row["contract_locus"] == "generated_at VALUE"]
    if len(generated) != 1 or generated[0]["p1_disposition"] != "underdetermined value locus":
        failure(errors, "SC-509 generated_at VALUE underdetermination record is missing or altered")
    if generated and "C17" not in generated[0]["pinned_successor"]:
        failure(errors, "SC-509 generated_at VALUE record lacks phase-2 C17 pin")
    if generated and "C3" not in generated[0]["pinned_successor"]:
        failure(errors, "SC-509 generated_at VALUE record lacks phase-3 C3 pin")
    keyed_rows = {(row["contract_id"], row["contract_locus"]): row for row in rows}
    if len(keyed_rows) != len(rows):
        failure(errors, "inventory has duplicate contract/locus rows")
    for key, expected_count in INVENTORY_LOCUS_COUNTS.items():
        row = keyed_rows.get(key)
        if row is None:
            failure(errors, f"inventory count row missing: {key}")
            continue
        try:
            actual_count = int(row["corpus_loci"])
        except ValueError:
            failure(errors, f"inventory corpus_loci is not an integer for {key}: {row['corpus_loci']!r}")
            continue
        if actual_count != expected_count:
            failure(errors, f"inventory corpus_loci drift for {key}: expected {expected_count}, got {actual_count}")
    for key, expected_ids in INPUT_CARRIER_IDS.items():
        row = keyed_rows.get(key)
        if row is None:
            failure(errors, f"input-carrier mapping row missing: {key}")
        elif row["obligation_ids"] != expected_ids:
            failure(errors, f"input-carrier obligation mapping drift for {key}")
    for contract_id in INVENTORY_IDS:
        if f"**{contract_id}" not in contract:
            failure(errors, f"contract no longer contains inventory row {contract_id}")
    for row in rows:
        if row["p1_disposition"] == "directional gap" and not row["pinned_successor"]:
            failure(errors, f"unnamed successor for zero-locus gap {row['contract_id']}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    root = args.root.resolve()
    evidence = root / "docs/migration/evidence"
    corpus = evidence / "corpus"
    contract_path = root / "docs/migration/semantic-contract.md"
    inventory_path = evidence / "p1-phase4-contract-obligation-loci.tsv"
    obligations_path = corpus / "OBLIGATIONS.tsv"
    invocations_path = corpus / "INVOCATIONS.tsv"
    errors: list[str] = []

    required = [contract_path, inventory_path, obligations_path, invocations_path]
    for path in required:
        if not path.is_file():
            failure(errors, f"missing required input: {path}")
    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1

    contract_bytes = contract_path.read_bytes()
    if git_blob(contract_bytes) != CONTRACT_BLOB:
        failure(errors, "contract blob drift: reconciliation must be re-derived")
    contract = contract_bytes.decode(errors="replace")
    verify_inventory(inventory_path, contract, errors)

    invocations = read_tsv(invocations_path)
    p1 = [row for row in invocations if row["phase"] == "P1"]
    if len(p1) != 1065:
        failure(errors, f"P1 population drift: expected 1065, got {len(p1)}")
    if len({(row["case"], row["consumer"]) for row in p1}) != len(p1):
        failure(errors, "P1 invocation keys are not unique")

    try:
        expected = expected_loci(root, p1)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        failure(errors, f"raw corpus evidence is unreadable or malformed: {exc}")
        expected = set()
    actual_rows = read_tsv(obligations_path)
    required_header = {
        "case",
        "consumer",
        "obligation_id",
        "stream",
        "locus",
    }
    if actual_rows and not required_header.issubset(actual_rows[0]):
        failure(errors, "OBLIGATIONS.tsv lacks reconciliation key columns")
    actual = {
        (row["case"], row["consumer"], row["obligation_id"], row["stream"], row["locus"])
        for row in actual_rows
    }
    if len(actual) != len(actual_rows):
        failure(errors, "OBLIGATIONS.tsv has duplicate reconciliation keys")

    unexpected_ids = {row[2] for row in actual} - EXPECTED_IDS
    missing_ids = EXPECTED_IDS - {row[2] for row in actual}
    if unexpected_ids:
        failure(errors, f"orphan obligation ID(s): {sorted(unexpected_ids)}")
    if missing_ids:
        failure(errors, f"missing obligation ID(s): {sorted(missing_ids)}")
    if expected != actual:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        if missing:
            failure(errors, f"contract-selected locus missing from table: {missing[:3]!r} (total {len(missing)})")
        if extra:
            failure(errors, f"table locus has no independent contract/raw-corpus basis: {extra[:3]!r} (total {len(extra)})")

    counts = Counter(row[2] for row in actual)
    if counts != EXPECTED_COUNTS:
        failure(errors, f"obligation counts differ: expected {EXPECTED_COUNTS}, got {dict(sorted(counts.items()))}")
    if len(expected) != 1378:
        failure(errors, f"independent locus count drift: expected 1378, got {len(expected)}")

    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1
    print("PASS contract-obligation reconciliation: 1378 loci / 6 IDs / contract 896d08ea")
    return 0


if __name__ == "__main__":
    sys.exit(main())
