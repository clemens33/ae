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
    Path("docs/migration/p1-phase4-gate.md"),
    Path("docs/migration/evidence/corpus/INVOCATIONS.tsv"),
    Path("docs/migration/evidence/corpus/OBLIGATIONS.tsv"),
    Path("docs/migration/evidence/corpus/SC-509C-UNPROVED.tsv"),
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


def require_red(
    temp_root: Path, label: str, expected_error: str, *, allow_mutated_table: bool = True
) -> None:
    command = [sys.executable, str(VERIFY), "--root", str(temp_root)]
    if allow_mutated_table:
        command.append("--allow-mutated-obligation-table")
    completed = subprocess.run(
        command,
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


def directional_gap_successor_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-gap-successor-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017j")
        target[6] = "p1-phase99-gate.md C100"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("directional-gap successor seed did not land")
        require_red(
            temp_root,
            "directional-gap arbitrary successor",
            "directional-gap successor pin drift",
        )


def pane_association_gap_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-pane-gap-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017p")
        target[6] = "p1-phase4-gate.md C11"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("pane-association gap seed did not land")
        require_red(
            temp_root,
            "pane-association gap C12 pin",
            "pane-association gap lacks phase-4 C12 pin",
        )


def partial_locus_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-partial-locus-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017q")
        target[6] = "p1-phase4-gate.md C11"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("partial corpus-locus seed did not land")
        require_red(
            temp_root,
            "partial corpus-locus C12 pin",
            "partial corpus locus lacks phase-4 C12 pin",
        )


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
        require_red(temp_root, "inventory locus count", "inventory expected fields drift")


def retained_locus_count_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-retained-count-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017a")
        target[4] = "1"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("retained locus-count seed did not land")
        require_red(
            temp_root,
            "retained corpus locus count",
            "inventory expected fields drift",
        )


def directional_gap_locus_count_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-gap-count-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017p")
        target[4] = "5"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("directional-gap locus-count seed did not land")
        require_red(
            temp_root,
            "directional-gap nonzero locus count",
            "inventory expected fields drift",
        )


def unpinned_gap_reclassification_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-unpinned-gap-reclass-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017j")
        target[2] = "retained corpus locus"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("unpinned directional-gap reclassification seed did not land")
        require_red(
            temp_root,
            "unpinned directional-gap reclassification",
            "inventory expected fields drift",
        )


def input_carrier_mapping_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-input-carrier-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-405l")
        target[3] = "-"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("input-carrier mapping seed did not land")
        require_red(
            temp_root,
            "input-carrier obligation mapping",
            "inventory expected fields drift",
        )


def carrier_reclassification_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-carrier-reclass-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-017j")
        target[2] = "input carrier"
        target[3] = "SC-TOTALLY-BOGUS"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("carrier reclassification seed did not land")
        require_red(
            temp_root,
            "carrier reclassification",
            "inventory expected fields drift",
        )


def extra_carrier_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-extra-carrier-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        extra = [
            "SC-017p",
            "laundered carrier locus",
            "input carrier",
            "SC-TOTALLY-BOGUS",
            "0",
            "seed only",
            "",
        ]
        rows.append(extra)
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if extra not in landed:
            raise RuntimeError("extra carrier seed did not land")
        require_red(
            temp_root,
            "extra input-carrier row",
            "inventory contract/locus rows differ",
        )


def carrier_locus_count_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-carrier-count-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-405l")
        target[4] = "777"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("carrier locus-count seed did not land")
        require_red(
            temp_root,
            "input-carrier nonzero locus count",
            "inventory expected fields drift",
        )


def obligation_ids_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-obligation-ids-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        inventory, header, rows = inventory_rows(temp_root)
        target = next(row for row in rows if row[0] == "SC-509d")
        target[3] = "SC-BOGUS"
        write_rows(inventory, header, rows)
        _, landed = read_rows(inventory)
        if target not in landed:
            raise RuntimeError("obligation-ids seed did not land")
        require_red(
            temp_root,
            "directional obligation IDs",
            "inventory expected fields drift",
        )


def omitted_sc509b_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509b-omitted-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        target = next(row for row in rows if row[2] == "SC-509b")
        rows.remove(target)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if target in landed:
            raise RuntimeError("SC-509b omitted seed did not land")
        require_red(temp_root, "omitted SC-509b relation", "contract-selected locus missing from table")


def omitted_sc509c_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509c-omitted-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        target = next(row for row in rows if row[2] == "SC-509c")
        rows.remove(target)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if target in landed:
            raise RuntimeError("SC-509c omitted seed did not land")
        require_red(
            temp_root,
            "omitted SC-509c relation",
            "contract-selected SC-509c relation missing from table",
        )


def sc017o_value_omission_seed() -> None:
    """Detect a checker that sees presence but forgets the VALUE owed set."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-017o-value-omitted-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        target = next(row for row in rows if row[2] == "SC-017o" and row[4] == "inventory_complete (value)")
        rows.remove(target)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if target in landed:
            raise RuntimeError("SC-017o VALUE omission seed did not land")
        require_red(temp_root, "omitted SC-017o VALUE locus", "contract-selected locus missing from table")


def accepted_table_blob_seed() -> None:
    """Detect a verifier that reads a changed accepted table without rejecting its pin."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-table-blob-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        original = table.read_bytes()
        mutated = original.replace(b"the field is mandated unconditionally", b"the field is mandated conditionally", 1)
        table.write_bytes(mutated)
        if b"the field is mandated conditionally" not in table.read_bytes():
            raise RuntimeError("accepted-table-blob seed did not land")
        require_red(
            temp_root,
            "accepted obligation-table blob drift",
            "accepted obligation-table blob drift",
            allow_mutated_table=False,
        )


def sc017o_payload_shape_seed() -> None:
    """Detect a checker that accepts an UNSCORABLE VALUE as an observed fact."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-017o-payload-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        target = next(row for row in rows if row[2] == "SC-017o" and row[4] == "inventory_complete (value)")
        target[9] = "OBSERVED"
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if target not in landed or target[9] != "OBSERVED":
            raise RuntimeError("SC-017o payload-shape seed did not land")
        require_red(temp_root, "SC-017o VALUE support", "SC-017o payload shape drift")


def sc017o_payload_map_unknown_key_seed() -> None:
    """Detect a payload map that silently defaults an unknown locus."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-017o-map-key-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        target = next(row for row in rows if row[2] == "SC-017o" and row[4] == "inventory_complete (value)")
        target[4] = "inventory_complete (unknown map key)"
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if target not in landed or target[4] != "inventory_complete (unknown map key)":
            raise RuntimeError("SC-017o unknown-map-key seed did not land")
        require_red(temp_root, "SC-017o payload unknown map key", "SC-017o payload shape drift")


def sc509b_raw_loss_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509b-raw-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        modes = (
            temp_root
            / "docs/migration/evidence/batch-c-artifacts/templates/G3/_meta/meta-mode-000.modes.tsv"
        )
        mutated = modes.read_text().replace("UNREADABLE", "READABLE", 1)
        modes.write_text(mutated)
        if "READABLE" not in modes.read_text() or "UNREADABLE" in modes.read_text():
            raise RuntimeError("SC-509b raw-loss seed did not land")
        require_red(
            temp_root,
            "SC-509b raw metadata loss",
            "table locus has no independent contract/raw-corpus basis",
        )


def sc509c_alert_currency_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509c-currency-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        events = (
            temp_root
            / "docs/migration/evidence/batch-c-artifacts/templates/A2/fixture-bytes/composite/sessions/twda1/events.jsonl"
        )
        events.chmod(events.stat().st_mode | 0o200)
        events.write_text(
            events.read_text()
            + '{"ts":"2026-08-20T15:00:20Z","actor":"fake:probe","action":"state","ref":"working"}\n'
        )
        if not events.read_text().endswith('"ref":"working"}\n'):
            raise RuntimeError("SC-509c alert-currency seed did not land")
        require_red(
            temp_root,
            "SC-509c alert currency",
            "table SC-509c relation has no independent contract/raw-corpus basis",
        )


def sc509c_duplicate_key_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509c-duplicate-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        duplicate = list(next(row for row in rows if row[2] == "SC-509c"))
        rows.append(duplicate)
        write_rows(table, header, rows)
        _, landed = read_rows(table)
        if landed.count(duplicate) != 2:
            raise RuntimeError("SC-509c duplicate-key seed did not land")
        require_red(
            temp_root,
            "duplicate SC-509c session-and-agent key",
            "OBLIGATIONS.tsv has duplicate SC-509c session-and-agent keys",
        )


def sc509c_carrier_overlap_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509c-overlap-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        events = (
            temp_root
            / "docs/migration/evidence/batch-c-artifacts/templates/A2/fixture-bytes/composite/sessions/twda1/events.jsonl"
        )
        events.chmod(events.stat().st_mode | 0o200)
        events.write_text(
            '{"ts":"2026-08-20T15:00:18Z","actor":"fake:probe","action":"state","ref":"dead"}\n'
            + events.read_text()
        )
        output = temp_root / "docs/migration/evidence/batch-c-artifacts/arms/A2/c01-filters-ro/out/list_json.stdout"
        output.chmod(output.stat().st_mode | 0o200)
        original = output.read_text()
        mutated = original.replace('"ref":"fake:probe","alias":"fake","name":"probe","session_id":"-","alive":true,"state":null,"reason":null', '"ref":"fake:probe","alias":"fake","name":"probe","session_id":"-","alive":true,"state":"dead","reason":null')
        output.write_text(mutated)
        if not events.read_text().startswith('{"ts":"2026-08-20T15:00:18Z"') or '"state":"dead","reason":null' not in output.read_text():
            raise RuntimeError("SC-509c carrier-overlap seed did not land")
        require_red(
            temp_root,
            "SC-509c state/alert carrier overlap",
            "SC-509c state and alert carriers overlap at session-and-agent grain",
        )


def sc509c_exclusion_carrier_seed() -> None:
    with tempfile.TemporaryDirectory(prefix="ae-c3-509c-exclusion-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        exclusions = temp_root / "docs/migration/evidence/corpus/SC-509C-UNPROVED.tsv"
        lines = exclusions.read_text().splitlines()
        header_index = next(index for index, line in enumerate(lines) if line.startswith("case\tconsumer\t"))
        header = lines[header_index].split("\t")
        rows = list(csv.reader(lines[header_index + 1 :], delimiter="\t"))
        target = next(
            row
            for row in rows
            if row[0] == "arms/A2/c01-filters-ro"
            and row[1] == "list_json"
            and row[2] == "tg2wu"
            and row[3] == "fake:lead"
        )
        target[2] = "tg2b"
        target[4] = "sessions[tg2b].agents[fake:lead].reason"
        exclusions.chmod(exclusions.stat().st_mode | 0o200)
        with exclusions.open("w", newline="") as handle:
            handle.write("\n".join(lines[:header_index]) + "\n")
            writer = csv.writer(handle, delimiter="\t", lineterminator="\n")
            writer.writerow(header)
            writer.writerows(rows)
        landed_lines = exclusions.read_text().splitlines()
        if "\ttg2b\tfake:lead\tsessions[tg2b].agents[fake:lead].reason\t" not in "\n".join(landed_lines):
            raise RuntimeError("SC-509c exclusion-carrier seed did not land")
        require_red(
            temp_root,
            "SC-509c exclusion claiming selected carrier",
            "SC-509c exclusion has independently proven raw carrier",
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


def gate_blob_seed() -> None:
    """Detect a rerun that retains values after criterion 3's gate input moved."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-gate-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        gate = temp_root / "docs/migration/p1-phase4-gate.md"
        gate.write_bytes(gate.read_bytes() + b"\nseed gate drift\n")
        if not gate.read_bytes().endswith(b"seed gate drift\n"):
            raise RuntimeError("gate-drift seed did not land")
        require_red(temp_root, "phase-4 gate blob drift", "phase-4 gate blob drift")


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


def obligation_header_seed() -> None:
    """Detect a parser that accepts a reordered or renamed identity column."""
    with tempfile.TemporaryDirectory(prefix="ae-c3-obligation-header-") as temp:
        temp_root = Path(temp)
        copy_inputs(temp_root)
        table = temp_root / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
        header, rows = read_rows(table)
        header[0] = "wrong_case"
        write_rows(table, header, rows)
        landed_header, _ = read_rows(table)
        if landed_header[0] != "wrong_case":
            raise RuntimeError("obligation-header seed did not land")
        require_red(
            temp_root,
            "obligation header",
            "OBLIGATIONS.tsv header drift",
            allow_mutated_table=False,
        )


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
        require_red(
            temp_root,
            "duplicate reconciliation key",
            "OBLIGATIONS.tsv has duplicate non-SC-509c reconciliation keys",
        )


def main() -> int:
    omitted_seed()
    orphan_seed()
    valid_id_extra_seed()
    generated_at_seed(-1, "", "missing generated_at VALUE record", "underdetermination record is missing or altered")
    generated_at_seed(2, "directional corpus locus", "generated_at misclassified directional", "underdetermination record is missing or altered")
    generated_at_seed(6, "p1-phase2-gate.md C99", "generated_at C17 pin", "lacks phase-2 C17 pin")
    gap_pin_seed()
    directional_gap_successor_seed()
    pane_association_gap_seed()
    partial_locus_seed()
    inventory_count_seed()
    retained_locus_count_seed()
    directional_gap_locus_count_seed()
    unpinned_gap_reclassification_seed()
    input_carrier_mapping_seed()
    carrier_reclassification_seed()
    extra_carrier_seed()
    carrier_locus_count_seed()
    obligation_ids_seed()
    omitted_sc509b_seed()
    omitted_sc509c_seed()
    sc017o_value_omission_seed()
    accepted_table_blob_seed()
    sc017o_payload_shape_seed()
    sc017o_payload_map_unknown_key_seed()
    sc509b_raw_loss_seed()
    sc509c_alert_currency_seed()
    sc509c_duplicate_key_seed()
    sc509c_carrier_overlap_seed()
    sc509c_exclusion_carrier_seed()
    contract_blob_seed()
    gate_blob_seed()
    inventory_header_seed()
    obligation_header_seed()
    p1_population_seed()
    duplicate_key_seed()
    return 0


if __name__ == "__main__":
    sys.exit(main())
