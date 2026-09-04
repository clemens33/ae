#!/usr/bin/env python3
"""Read the generated obligation tuple once and keep its verified bytes.

``FRESHNESS.tsv`` is published after its three data members.  A reader must
therefore read OBLIGATIONS, the added-roster declaration and SC-509C-UNPROVED
once, read FRESHNESS last, validate those exact bytes, and parse the saved
OBLIGATIONS bytes.  Running a verifier and reopening OBLIGATIONS afterwards
would cross the publication boundary a rename is meant to protect.
"""
from __future__ import annotations

import hashlib
import re
import subprocess
import tempfile
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


CONTRACT_PATH = "docs/migration/semantic-contract.md"
CORPUS_REL = Path("docs/migration/evidence/corpus")
OBLIGATIONS_NAME = "OBLIGATIONS.tsv"
GAP_NAME = "UNOBSERVABLE-ADDED-ROSTER.tsv"
UNPROVED_NAME = "SC-509C-UNPROVED.tsv"
FRESHNESS_NAME = "FRESHNESS.tsv"
FRESH_FIELDS = (
    "contract_path",
    "contract_blob",
    "p1_rows",
    "obligation_rows",
    "obligations_sha256",
    "added_roster_gap_sha256",
    "sc509c_unproved_sha256",
)
HASH_FIELDS = (
    ("obligations_sha256", OBLIGATIONS_NAME),
    ("added_roster_gap_sha256", GAP_NAME),
    ("sc509c_unproved_sha256", UNPROVED_NAME),
)
SHA256 = re.compile(r"^[0-9a-f]{64}$")
BLOB = re.compile(r"^[0-9a-f]{40}$")


class ArtifactTupleError(RuntimeError):
    """A phase-4 reader has no coherent generated tuple to score."""


@dataclass(frozen=True)
class ArtifactTuple:
    """One validated four-file generation, held in memory for a whole score pass."""

    obligations: bytes
    added_roster_gap: bytes
    sc509c_unproved: bytes
    freshness: bytes
    fields: dict[str, str]

    @property
    def identity(self) -> str:
        """A compact, complete attribution record for derived scorer output."""
        return " ".join(
            f"{field}={self.fields[field]}"
            for field in ("contract_blob", *(field for field, _ in HASH_FIELDS))
        )


def _paths(repo: Path) -> dict[str, Path]:
    corpus = repo / CORPUS_REL
    return {
        OBLIGATIONS_NAME: corpus / OBLIGATIONS_NAME,
        GAP_NAME: corpus / GAP_NAME,
        UNPROVED_NAME: corpus / UNPROVED_NAME,
        FRESHNESS_NAME: corpus / FRESHNESS_NAME,
    }


def _read(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-MISSING {path}: {error}") from error


def _freshness_fields(raw: bytes, source: Path) -> dict[str, str]:
    if b"\r" in raw:
        raise ArtifactTupleError(
            f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: CR bytes are outside the LF TSV grammar"
        )
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: non-UTF-8") from error
    lines = [line for line in text.splitlines() if line and not line.startswith("#")]
    if not lines or lines[0] != "field\tvalue":
        raise ArtifactTupleError(
            f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: expected exact field<TAB>value header"
        )
    fields: dict[str, str] = {}
    counts: Counter[str] = Counter()
    for number, line in enumerate(lines[1:], 2):
        cells = line.split("\t")
        if len(cells) != 2:
            raise ArtifactTupleError(
                f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: data row {number} has {len(cells)} fields"
            )
        key, value = cells
        counts[key] += 1
        fields[key] = value
    if tuple(fields) != FRESH_FIELDS or any(counts[key] != 1 for key in FRESH_FIELDS):
        missing = sorted(set(FRESH_FIELDS) - set(fields))
        unknown = sorted(set(fields) - set(FRESH_FIELDS))
        repeated = sorted(key for key, count in counts.items() if count != 1)
        raise ArtifactTupleError(
            "ARTIFACT-TUPLE-FRESHNESS-SCHEMA "
            f"{source}: missing={missing} unknown={unknown} repeated={repeated} or non-canonical order"
        )
    if fields["contract_path"] != CONTRACT_PATH:
        raise ArtifactTupleError(
            "ARTIFACT-TUPLE-CONTRACT "
            f"{source}: contract_path={fields['contract_path']!r}, expected {CONTRACT_PATH!r}"
        )
    if not BLOB.fullmatch(fields["contract_blob"]):
        raise ArtifactTupleError(
            f"ARTIFACT-TUPLE-CONTRACT {source}: contract_blob is not a lowercase Git blob id"
        )
    for field, _name in HASH_FIELDS:
        if not SHA256.fullmatch(fields[field]):
            raise ArtifactTupleError(
                f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: {field} is not a lowercase SHA-256"
            )
    for field in ("p1_rows", "obligation_rows"):
        if not re.fullmatch(r"0|[1-9][0-9]*", fields[field]):
            raise ArtifactTupleError(
                f"ARTIFACT-TUPLE-FRESHNESS-SCHEMA {source}: {field} is not a canonical non-negative count"
            )
    return fields


def _head_contract_blob(repo: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", f"HEAD:{CONTRACT_PATH}"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", "replace").strip() or "no diagnostic"
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-CONTRACT cannot resolve HEAD contract blob: {detail}")
    return result.stdout.decode("ascii", "strict").strip()


def read_generated_tuple(
    repo: Path,
    *,
    read_bytes: Callable[[Path], bytes] | None = None,
) -> ArtifactTuple:
    """Return the one valid in-memory tuple; FRESHNESS is deliberately read last."""
    reader = read_bytes or _read
    paths = _paths(repo)
    # This order is the reader side of the publisher's FRESHNESS-last protocol.
    obligations = reader(paths[OBLIGATIONS_NAME])
    gap = reader(paths[GAP_NAME])
    unproved = reader(paths[UNPROVED_NAME])
    freshness = reader(paths[FRESHNESS_NAME])
    fields = _freshness_fields(freshness, paths[FRESHNESS_NAME])
    contents = {
        OBLIGATIONS_NAME: obligations,
        GAP_NAME: gap,
        UNPROVED_NAME: unproved,
    }
    for field, name in HASH_FIELDS:
        actual = hashlib.sha256(contents[name]).hexdigest()
        if fields[field] != actual:
            raise ArtifactTupleError(
                "ARTIFACT-TUPLE "
                f"{field}={fields[field]} but {name} snapshot hashes to {actual}"
            )
    expected_contract = _head_contract_blob(repo)
    if fields["contract_blob"] != expected_contract:
        raise ArtifactTupleError(
            "ARTIFACT-TUPLE-CONTRACT "
            f"FRESHNESS binds {fields['contract_blob']}, HEAD binds {expected_contract}"
        )
    return ArtifactTuple(obligations, gap, unproved, freshness, fields)


def parse_tsv_bytes(raw: bytes, source: str) -> tuple[list[str], list[dict[str, str]]]:
    """Parse saved OBLIGATIONS-shaped TSV bytes; comments are not this grammar."""
    try:
        lines = raw.decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-OBLIGATIONS {source}: non-UTF-8") from error
    if not lines:
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-OBLIGATIONS {source}: empty TSV")
    header = lines[0].split("\t")
    if not header or any(not field for field in header) or len(set(header)) != len(header):
        raise ArtifactTupleError(f"ARTIFACT-TUPLE-OBLIGATIONS {source}: malformed or duplicate header")
    rows: list[dict[str, str]] = []
    for number, line in enumerate(lines[1:], 2):
        if not line:
            continue
        fields = line.split("\t")
        if len(fields) != len(header):
            raise ArtifactTupleError(
                f"ARTIFACT-TUPLE-OBLIGATIONS {source}: row {number} has {len(fields)} fields, expected {len(header)}"
            )
        rows.append(dict(zip(header, fields)))
    return header, rows


def _git(repo: Path, *args: str) -> str:
    result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=True)
    return result.stdout.strip()


def make_redproof_fixture(root: Path) -> tuple[dict[str, Path], bytes]:
    """Create an isolated Git-backed tuple fixture for phase-4 redproofs only."""
    corpus = root / CORPUS_REL
    corpus.mkdir(parents=True, exist_ok=True)
    contract = root / CONTRACT_PATH
    contract.parent.mkdir(parents=True, exist_ok=True)
    contract.write_text("# contract\n", encoding="utf-8")
    _git(root, "init", "-q")
    _git(root, "config", "user.email", "tuple@example.invalid")
    _git(root, "config", "user.name", "tuple redproof")
    _git(root, "add", CONTRACT_PATH)
    _git(root, "commit", "-qm", "fixture")
    paths = _paths(root)
    obligations = b"case\tconsumer\nold\tview\n"
    gap = b"case\tconsumer\tadded_session\n"
    unproved = b"case\tconsumer\tsession\tagent_ref\tlocus\n"
    paths[OBLIGATIONS_NAME].write_bytes(obligations)
    paths[GAP_NAME].write_bytes(gap)
    paths[UNPROVED_NAME].write_bytes(unproved)
    fields = {
        "contract_path": CONTRACT_PATH,
        "contract_blob": _git(root, "rev-parse", f"HEAD:{CONTRACT_PATH}"),
        "p1_rows": "1",
        "obligation_rows": "1",
        "obligations_sha256": hashlib.sha256(obligations).hexdigest(),
        "added_roster_gap_sha256": hashlib.sha256(gap).hexdigest(),
        "sc509c_unproved_sha256": hashlib.sha256(unproved).hexdigest(),
    }
    paths[FRESHNESS_NAME].write_text(
        "# fixture tuple\nfield\tvalue\n" + "".join(f"{key}\t{fields[key]}\n" for key in FRESH_FIELDS),
        encoding="utf-8",
    )
    return paths, obligations


def _must_refuse(action: Callable[[], object], label: str) -> None:
    try:
        action()
    except ArtifactTupleError as error:
        print(f"RED {label}: {error}")
        return
    raise RuntimeError(f"REDPROOF {label} unexpectedly accepted")


def redproof() -> None:
    """Prove the publication-boundary reader refuses mixed or malformed tuples."""
    with tempfile.TemporaryDirectory(prefix="ae-artifact-tuple-") as temp:
        root = Path(temp)
        fixture_root = root / "valid"
        paths, old_obligations = make_redproof_fixture(fixture_root)
        read_order: list[str] = []

        def recording_reader(path: Path) -> bytes:
            read_order.append(path.name)
            return _read(path)

        snapshot = read_generated_tuple(fixture_root, read_bytes=recording_reader)
        if read_order != [OBLIGATIONS_NAME, GAP_NAME, UNPROVED_NAME, FRESHNESS_NAME]:
            raise RuntimeError(f"REDPROOF FRESHNESS-last read order changed: {read_order}")
        _header, rows = parse_tsv_bytes(snapshot.obligations, "saved OBLIGATIONS snapshot")
        if rows != [{"case": "old", "consumer": "view"}]:
            raise RuntimeError(f"REDPROOF valid snapshot parsed wrong rows: {rows!r}")
        print("GREEN valid FRESHNESS-last tuple")

        # A rename after capture cannot change the saved bytes; reopening would see
        # this new row, while parsing snapshot.obligations remains old by construction.
        paths[OBLIGATIONS_NAME].write_bytes(b"case\tconsumer\nnew\tview\n")
        _header, rows = parse_tsv_bytes(snapshot.obligations, "saved OBLIGATIONS snapshot")
        if snapshot.obligations != old_obligations or rows[0]["case"] != "old":
            raise RuntimeError("REDPROOF TOCTOU snapshot was not retained")
        _must_refuse(lambda: read_generated_tuple(fixture_root), "TOCTOU OBLIGATIONS replacement")

        fixture_root = root / "schema"
        paths, _old_obligations = make_redproof_fixture(fixture_root)
        fresh = paths[FRESHNESS_NAME].read_text(encoding="utf-8")
        paths[FRESHNESS_NAME].write_text(fresh + "unknown\tvalue\n", encoding="utf-8")
        _must_refuse(lambda: read_generated_tuple(fixture_root), "FRESHNESS exact schema")

        fixture_root = root / "crlf"
        paths, _old_obligations = make_redproof_fixture(fixture_root)
        paths[FRESHNESS_NAME].write_bytes(
            paths[FRESHNESS_NAME].read_bytes().replace(b"\n", b"\r\n")
        )
        _must_refuse(lambda: read_generated_tuple(fixture_root), "FRESHNESS LF grammar")

        fixture_root = root / "gap"
        paths, _old_obligations = make_redproof_fixture(fixture_root)
        paths[GAP_NAME].write_bytes(paths[GAP_NAME].read_bytes() + b"new\tview\tsession\n")
        _must_refuse(lambda: read_generated_tuple(fixture_root), "GAP hash binding")

        fixture_root = root / "unproved"
        paths, _old_obligations = make_redproof_fixture(fixture_root)
        paths[UNPROVED_NAME].write_bytes(paths[UNPROVED_NAME].read_bytes() + b"new\tview\ts\ta\tl\n")
        _must_refuse(lambda: read_generated_tuple(fixture_root), "UNPROVED hash binding")

        fixture_root = root / "contract"
        paths, _old_obligations = make_redproof_fixture(fixture_root)
        fresh = paths[FRESHNESS_NAME].read_text(encoding="utf-8")
        paths[FRESHNESS_NAME].write_text(
            re.sub(r"(?m)^contract_blob\t[0-9a-f]{40}$", "contract_blob\t" + "0" * 40, fresh),
            encoding="utf-8",
        )
        _must_refuse(lambda: read_generated_tuple(fixture_root), "contract blob binding")
    print("ARTIFACT-TUPLE-REDPROOF PASS: FRESHNESS-last same-read snapshot")


if __name__ == "__main__":
    if __import__("sys").argv[1:] == ["redproof"]:
        redproof()
    else:
        raise SystemExit("usage: artifact_tuple.py redproof")
