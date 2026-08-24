#!/usr/bin/env python3
"""Isolated RED proofs for the independent contract-obligation reconciliation."""

from __future__ import annotations

import csv
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
RELATIVE = [
    Path("docs/migration/semantic-contract.md"),
    Path("docs/migration/evidence/corpus/INVOCATIONS.tsv"),
    Path("docs/migration/evidence/corpus/OBLIGATIONS.tsv"),
    Path("docs/migration/evidence/batch-c-artifacts"),
    Path("docs/migration/evidence/p1-phase4-contract-obligation-loci.tsv"),
]
VERIFY = HERE / "verify-contract-obligation-reconciliation.py"


def copy_inputs(destination: Path) -> None:
    for relative in RELATIVE:
        source = ROOT / relative
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_dir():
            shutil.copytree(source, target)
        else:
            shutil.copy2(source, target)


def read_rows(path: Path) -> tuple[list[str], list[list[str]]]:
    with path.open(newline="") as handle:
        rows = list(csv.reader(handle, delimiter="\t"))
    return rows[0], rows[1:]


def write_rows(path: Path, header: list[str], rows: list[list[str]]) -> None:
    with path.open("w", newline="") as handle:
        writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def require_red(temp_root: Path, label: str, expected_error: str) -> None:
    completed = subprocess.run(
        [sys.executable, str(VERIFY), "--root", str(temp_root)],
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode == 0:
        raise RuntimeError(f"{label}: verifier unexpectedly passed\n{completed.stdout}{completed.stderr}")
    output = completed.stdout + completed.stderr
    if expected_error not in output:
        raise RuntimeError(
            f"{label}: verifier failed for the wrong reason; expected {expected_error!r}\n{output}"
        )
    print(f"PASS red proof {label}: rc={completed.returncode}")


def omitted_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-omitted-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        original = len(rows)
        target = next(row for row in rows if row[2] == "SC-017m")
        rows.remove(target)
        write_rows(table, header, rows)
        # LANDING CHECK precedes the verifier rc: this seed is invalid if absent.
        _, landed = read_rows(table)
        if len(landed) != original - 1 or target in landed:
            raise RuntimeError("omitted seed did not land in isolated table")
        require_red(temp_root, "omitted SC-017m locus", "contract-selected locus missing from table")


def orphan_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-orphan-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        original = len(rows)
        orphan = list(rows[0])
        orphan[2] = "SC-FAKE-ORPHAN"
        rows.append(orphan)
        write_rows(table, header, rows)
        # LANDING CHECK precedes the verifier rc: this seed is invalid if absent.
        _, landed = read_rows(table)
        if len(landed) != original + 1 or orphan not in landed:
            raise RuntimeError("orphan seed did not land in isolated table")
        require_red(temp_root, "orphan obligation ID", "orphan obligation ID(s)")


def inventory_rows(temp_root: Path) -> tuple[Path, list[str], list[list[str]]]:
    inventory = temp_root / "docs/migration/evidence/p1-phase4-contract-obligation-loci.tsv"
    header, rows = read_rows(inventory)
    return inventory, header, rows


def valid_id_extra_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-valid-extra-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        extra = next(row for row in rows if row[2] == "SC-017m")
        extra = list(extra)
        extra[4] = "invented locus"
        rows.append(extra)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if extra not in landed:
            raise RuntimeError("valid-ID extra seed did not land in isolated table")
        require_red(temp_root, "extra locus with valid ID", "table locus has no independent contract/raw-corpus basis")


def generated_at_seed(column: int, replacement: str, label: str, expected_error: str) -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-generated-at-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-509" and row[1] == "generated_at VALUE")
        if column < 0:
            rows.remove(target)
        else:
            target[column] = replacement
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if column < 0 and any(row[0] == "SC-509" and row[1] == "generated_at VALUE" for row in landed):
            raise RuntimeError(f"{label}: deleted generated_at seed did not land")
        if column >= 0 and target not in landed:
            raise RuntimeError(f"{label}: generated_at seed did not land")
        require_red(temp_root, label, expected_error)


def gap_pin_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-gap-pin-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017j")
        target[6] = ""
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed or target[6] != "":
            raise RuntimeError("directional-gap pin seed did not land")
        require_red(temp_root, "blank directional-gap successor", "unnamed successor for zero-locus gap SC-017j")


def inventory_count_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-inventory-count-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017l")
        target[4] = "999"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("inventory count seed did not land")
        require_red(temp_root, "inventory locus count", "inventory corpus_loci drift")


def input_carrier_mapping_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-input-carrier-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017q")
        target[3] = "-"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("input-carrier mapping seed did not land")
        require_red(
            temp_root,
            "input-carrier obligation mapping",
            "input-carrier obligation mapping drift",
        )


def contract_blob_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-contract-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        contract = temp_root / "docs/migration/semantic-contract.md"
        contract.write_bytes(contract.read_bytes() + b"\nseed contract drift\n")
        if not contract.read_bytes().endswith(b"seed contract drift\n"):
            raise RuntimeError("contract-drift seed did not land")
        require_red(temp_root, "contract blob drift", "contract blob drift")


def inventory_header_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-inventory-header-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        header[0] = "wrong_contract_id"
        write_rows(inventory, header, rows)
        landed_header, _ = read_rows(inventory)
        if landed_header[0] != "wrong_contract_id":
            raise RuntimeError("inventory-header seed did not land")
        require_red(temp_root, "inventory header", "inventory header drift")


def p1_population_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-population-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        invocations = temp_root / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
        header, rows = read_rows(invocations)
        phase = header.index("phase")
        target = next(row for row in rows if row[phase] == "P1")
        rows.remove(target)
        write_rows(invocations, header, rows)
        _, landed = read_rows(invocations)
        if target in landed:
            raise RuntimeError("P1-population seed did not land")
        require_red(temp_root, "P1 population", "P1 population drift")


def duplicate_key_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-duplicate-key-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        duplicate = list(rows[0])
        rows.append(duplicate)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if landed.count(duplicate) != 2:
            raise RuntimeError("duplicate-key seed did not land")
        require_red(temp_root, "duplicate reconciliation key", "OBLIGATIONS.tsv has duplicate reconciliation keys")


def main() -> int:
    omitted_seed()
    orphan_seed()
    valid_id_extra_seed()
    generated_at_seed(-1, "", "missing generated_at VALUE record", "underdetermination record is missing or altered")
    generated_at_seed(2, "directional corpus locus", "generated_at misclassified directional", "underdetermination record is missing or altered")
    generated_at_seed(6, "p1-phase2-gate.md C99", "generated_at C17 pin", "lacks phase-2 C17 pin")
    gap_pin_seed()
    inventory_count_seed()
    input_carrier_mapping_seed()
    contract_blob_seed()
    inventory_header_seed()
    p1_population_seed()
    duplicate_key_seed()
    return 0


if __name__ == "__main__":
    sys.exit(main())
