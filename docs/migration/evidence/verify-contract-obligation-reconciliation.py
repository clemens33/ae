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
GATE_BLOB = "f31ece2ac40ed47077ab07f559ad8ab5ad97f6b0"
OBLIGATIONS_BLOB = "44e06c29cc078e6933298139d204413966419d81"
EXPECTED_IDS = {
    "SC-017l",
    "SC-017m",
    "SC-017o",
    "SC-017r",
    "SC-509b",
    "SC-509c",
    "SC-509d",
    "SC-509e",
}
EXPECTED_COUNTS = {
    "SC-017l": 134,
    "SC-017m": 150,
    "SC-017o": 802,
    "SC-017r": 78,
    "SC-509b": 14,
    "SC-509c": 222,
    "SC-509d": 401,
    "SC-509e": 42,
}
EXPECTED_RELATION_COUNT = 1843
EXPECTED_SUPPORT_COUNTS = {"OBSERVED": 1082, "UNSCORABLE": 761}
EXPECTED_CARRYING_ROWS = 581
EXPECTED_B_LOSS_INSTANCE_COUNT = 20
EXPECTED_UNPROVED_COUNT = 184
UNPROVED_HEADER = [
    "case",
    "consumer",
    "session",
    "agent_ref",
    "locus",
    "session_attention",
    "kind",
    "why",
]
INVENTORY_EXPECTATIONS = {
    ("SC-017a", "default running-scope selection"): ("retained corpus locus", "-", 859),
    ("SC-017b", "all-view status-group selection"): ("retained corpus locus", "-", 859),
    ("SC-017c", "stopped-only selection"): ("retained corpus locus", "-", 859),
    ("SC-017d", "attention filtering"): ("retained corpus locus", "-", 859),
    ("SC-017e", "activity filtering"): ("retained corpus locus", "-", 859),
    ("SC-017f", "JSON filter parity"): ("retained corpus locus", "-", 401),
    ("SC-017g", "attention marker selection"): ("retained corpus locus", "-", 859),
    ("SC-017h", "human per-agent health/state/attn presentation"): ("retained corpus locus", "-", 458),
    ("SC-017i", "explicit running alias selection"): ("retained corpus locus", "-", 859),
    ("SC-017j", "candidate membership and coalescence"): ("directional gap", "-", 0),
    ("SC-017k", "recorded-server exact liveness"): ("directional gap", "-", 0),
    ("SC-017l", "unknown session status"): ("directional corpus locus", "SC-017l", 134),
    ("SC-017m", "unknown session membership and rendering"): ("directional corpus locus", "SC-017m", 150),
    ("SC-017n", "C-byte group/name order"): ("directional gap", "-", 0),
    ("SC-017o", "inventory completeness boolean presence"): ("directional corpus locus", "SC-017o", 401),
    ("SC-017o", "inventory completeness VALUE"): ("underdetermined value locus", "SC-017o", 401),
    ("SC-017p", "positive per-agent liveness proof"): ("directional gap", "-", 0),
    ("SC-017q", "unknown per-agent liveness"): ("partial corpus locus", "SC-017r,SC-509e", 120),
    ("SC-017r", "human agent-health marker"): ("directional corpus locus", "SC-017r", 78),
    ("SC-017s", "pane live predicate"): ("directional gap", "-", 0),
    ("SC-021", "ls alias equivalence"): ("retained corpus locus", "-", 116),
    ("SC-400d", "canonical and legacy durable-root membership"): ("directional gap", "-", 0),
    ("SC-405l", "selector normalization"): ("input carrier", "SC-017l,SC-017m", 0),
    ("SC-506", "partial-failure JSON validity"): ("retained corpus locus", "-", 401),
    ("SC-509", "generated_at field presence/type"): ("retained corpus locus", "-", 401),
    ("SC-509", "generated_at VALUE"): ("underdetermined value locus", "-", 401),
    ("SC-509", "other retained version-1 object fields"): ("retained corpus locus", "-", 401),
    ("SC-509b", "degraded true after actual metadata read/parse loss"): ("directional corpus locus", "SC-509b", 14),
    ("SC-509c", "agents[].reason agent-owned active contribution"): ("directional corpus locus", "SC-509c", 222),
    ("SC-509d", "schema_version"): ("directional corpus locus", "SC-509d", 401),
    ("SC-509e", "agents[].alive nullable domain"): ("directional corpus locus", "SC-509e", 42),
    ("SC-518", "requests closure presentation"): ("retained corpus locus", "-", 168),
    ("SC-521c", "unknown attention/activity filtering"): ("directional gap", "-", 0),
    ("SC-1306a", "list snapshot cut"): ("retained corpus locus", "-", 743),
    ("SC-1306d", "requests snapshot cut"): ("retained corpus locus", "-", 168),
    ("SC-1306e", "events-tail snapshot cut"): ("retained corpus locus", "-", 38),
}
PANE_ASSOCIATION_GAPS = {
    ("SC-017p", "positive per-agent liveness proof"),
    ("SC-017s", "pane live predicate"),
}
DIRECTIONAL_GAP_SUCCESSORS = {
    ("SC-017j", "candidate membership and coalescence"): "p1-phase1-gate.md C5,C8,C9,C20; p1-phase2-gate.md C3-C12",
    ("SC-017k", "recorded-server exact liveness"): "p1-phase2-gate.md C3,C6,C8,C11,C12",
    ("SC-017n", "C-byte group/name order"): "p1-phase3-gate.md C9,C10,C11",
    ("SC-017p", "positive per-agent liveness proof"): "p1-phase4-gate.md C12",
    ("SC-017s", "pane live predicate"): "p1-phase4-gate.md C12",
    ("SC-400d", "canonical and legacy durable-root membership"): "p1-phase1-gate.md C2",
    ("SC-521c", "unknown attention/activity filtering"): "p1-phase3-gate.md C6,C7,C8",
}
ACTIVE_AGENT_REASONS = {"dead", "stale", "waiting-user", "blocked", "throttled"}
ALERT_REASONS = {
    "agent process dead — dropped to shell": "dead",
    "max nudges reached (no recent events), needs attention": "stale",
    "throttled for 10s — may need attention": "throttled",
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
OBLIGATION_HEADER = [
    "case", "consumer", "obligation_id", "stream", "locus", "from", "to",
    "predicate", "baseline_provenance", "support", "authority",
]
RECONCILIATION_KEY_COLUMNS = ("case", "consumer", "obligation_id", "stream", "locus")
PAYLOAD_COLUMNS = ("from", "to", "predicate", "baseline_provenance", "support", "authority")
SC017O_PAYLOADS = {
    "inventory_complete": (
        "ABSENT", "present", "present", "OBSERVED", "OBSERVED",
        "the field is mandated unconditionally; its VALUE is unscorable on this corpus and is recorded as such, not asserted",
    ),
    "inventory_complete (value)": (
        "ABSENT", "the enumeration's actual completeness", "undecidable", "OBSERVED", "UNSCORABLE",
        "no captured connect failure names a session's RECORDED server, so no independently entitled enumeration is shown to have finally failed; ambient entitlement turns on the AE_TMUX_SERVER selection SC-1410c leaves unclassified",
    ),
}
INVENTORY_IDS = {contract_id for contract_id, _ in INVENTORY_EXPECTATIONS}


def git_blob(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def read_tsv(path: Path) -> list[dict[str, str]]:
    with path.open(newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def read_comment_tsv(path: Path) -> tuple[list[str], list[dict[str, str]]]:
    with path.open(newline="") as handle:
        reader = csv.DictReader(
            (line for line in handle if not line.startswith("#")), delimiter="\t"
        )
        return reader.fieldnames or [], list(reader)


def read_obligation_rows(path: Path, errors: list[str]) -> list[dict[str, str]]:
    """Parse before pinning so malformed and drifted inputs stay distinct."""
    try:
        with path.open(newline="") as handle:
            reader = csv.DictReader(handle, delimiter="\t")
            if reader.fieldnames != OBLIGATION_HEADER:
                failure(errors, f"OBLIGATIONS.tsv header drift: {reader.fieldnames!r}")
                return []
            rows = list(reader)
    except (OSError, UnicodeError, csv.Error) as exc:
        failure(errors, f"OBLIGATIONS.tsv malformed: {exc}")
        return []
    for index, row in enumerate(rows, start=2):
        if None in row or any(row.get(column) is None for column in OBLIGATION_HEADER):
            failure(errors, f"OBLIGATIONS.tsv malformed row at line {index}")
            return []
    return rows


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


def template_identity(root: Path, invocation: dict[str, str]) -> tuple[str, str] | None:
    case = (case_dir(root, invocation) / "case.txt").read_text(errors="replace")
    template = next(
        (word.removeprefix("template=") for word in case.split() if word.startswith("template=")),
        None,
    )
    if template is None or "/" not in template:
        return None
    return tuple(template.split("/", 1))


def actual_meta_loss(
    root: Path, invocation: dict[str, str], session_name: str
) -> bool:
    """Prove actual metadata loss from fixture bytes, never from sparsity."""
    identity = template_identity(root, invocation)
    if identity is None:
        return False
    family, fixture_name = identity
    meta = (
        root
        / "docs/migration/evidence/batch-c-artifacts/templates"
        / family
        / "_meta"
    )
    modes = meta / f"{fixture_name}.modes.tsv"
    mutation = meta / f"{fixture_name}.mutation.txt"
    unreadable = False
    if modes.is_file():
        with modes.open(newline="") as handle:
            unreadable = any(
                len(row) >= 5
                and row[2] == "UNREADABLE"
                and row[4] == f"./sessions/{session_name}/meta"
                for row in csv.reader(handle, delimiter="\t")
            )
    mutation_text = mutation.read_text(errors="replace") if mutation.is_file() else ""
    absent = (
        f"file: sessions/{session_name}/meta" in mutation_text
        and "after:  FILE ABSENT" in mutation_text
    )
    return unreadable or absent


def fixture_has_meta_loss(root: Path, invocation: dict[str, str]) -> bool:
    """Count raw P1 loss-bearing JSON inputs independently of rendered grain."""
    identity = template_identity(root, invocation)
    if identity is None:
        return False
    family, fixture_name = identity
    meta = (
        root
        / "docs/migration/evidence/batch-c-artifacts/templates"
        / family
        / "_meta"
    )
    modes = meta / f"{fixture_name}.modes.tsv"
    mutation = meta / f"{fixture_name}.mutation.txt"
    unreadable = False
    if modes.is_file():
        with modes.open(newline="") as handle:
            unreadable = any(
                len(row) >= 5 and row[2] == "UNREADABLE" and row[4].endswith("/meta")
                for row in csv.reader(handle, delimiter="\t")
            )
    mutation_text = mutation.read_text(errors="replace") if mutation.is_file() else ""
    return unreadable or ("meta file" in mutation_text and "FILE ABSENT" in mutation_text)


def fixture_events(
    root: Path,
    family: str,
    fixture_name: str,
    session_name: str,
    cache: dict[tuple[str, str, str], list[dict[str, object]]],
) -> list[dict[str, object]]:
    """Read every usable producer event; malformed bytes establish no carrier."""
    key = (family, fixture_name, session_name)
    if key not in cache:
        path = (
            root
            / "docs/migration/evidence/batch-c-artifacts/templates"
            / family
            / "fixture-bytes"
            / fixture_name
            / "sessions"
            / session_name
            / "events.jsonl"
        )
        events: list[dict[str, object]] = []
        if path.is_file():
            for line in path.read_text(errors="replace").splitlines():
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(event, dict):
                    events.append(event)
        cache[key] = events
    return cache[key]


def agent_reason_carriers(
    events: list[dict[str, object]], agent_ref: str, output_state: object
) -> set[tuple[str, str]]:
    """Return independently proven `(carrier, reason)` pairs for one agent.

    A state event is self-declared.  An alert names its target; another later
    event from that target clears it, and the latest surviving alert wins.
    """
    carriers: set[tuple[str, str]] = set()
    if (
        isinstance(output_state, str)
        and output_state in ACTIVE_AGENT_REASONS
        and any(
            event.get("action") == "state"
            and event.get("actor") == agent_ref
            and event.get("ref") == output_state
            for event in events
        )
    ):
        carriers.add(("state", output_state))
    alerts = [
        (index, ALERT_REASONS[event["summary"]])
        for index, event in enumerate(events)
        if event.get("action") == "alert"
        and event.get("target") == agent_ref
        and event.get("summary") in ALERT_REASONS
        and not any(later.get("actor") == agent_ref for later in events[index + 1 :])
    ]
    if alerts:
        carriers.add(("alert", alerts[-1][1]))
    return carriers


def expected_agent_reason_loci(
    root: Path, p1: list[dict[str, str]]
) -> tuple[
    set[tuple[str, str, str, str, str, str, str, str]],
    set[tuple[str, str, str, str, str]],
    set[tuple[str, str, str, str, str]],
    dict[tuple[str, str, str, str, str], object],
]:
    """Select SC-509c at case/consumer/session/agent output grain."""
    result: set[tuple[str, str, str, str, str, str, str, str]] = set()
    state_addresses: set[tuple[str, str, str, str, str]] = set()
    alert_addresses: set[tuple[str, str, str, str, str]] = set()
    observed_addresses: dict[tuple[str, str, str, str, str], object] = {}
    cache: dict[tuple[str, str, str], list[dict[str, object]]] = {}
    for invocation in p1:
        if not is_listing(invocation) or not is_json(invocation):
            continue
        case = PurePosixPath(invocation["case"]).parent.as_posix()
        consumer = invocation["consumer"]
        document = json.loads(captured_stdout(root, invocation))
        identity = template_identity(root, invocation)
        for session in document.get("sessions", []):
            session_name = session.get("name")
            if not isinstance(session_name, str):
                continue
            for agent in session.get("agents", []):
                agent_ref = agent.get("ref")
                if (
                    not isinstance(agent_ref, str)
                    or agent.get("reason", object()) is not None
                ):
                    continue
                locus = f"sessions[{session_name}].agents[{agent_ref}].reason"
                address = (case, consumer, "SC-509c", "digest", locus)
                observed_addresses[address] = session.get("attention")
            if identity is None:
                continue
            family, fixture_name = identity
            events = fixture_events(root, family, fixture_name, session_name, cache)
            for agent in session.get("agents", []):
                agent_ref = agent.get("ref")
                if (
                    not isinstance(agent_ref, str)
                    or agent.get("reason", object()) is not None
                ):
                    continue
                locus = f"sessions[{session_name}].agents[{agent_ref}].reason"
                address = (case, consumer, "SC-509c", "digest", locus)
                for carrier, reason in agent_reason_carriers(
                    events, agent_ref, agent.get("state")
                ):
                    result.add((*address, "null", reason, "equals"))
                    if carrier == "state":
                        state_addresses.add(address)
                    else:
                        alert_addresses.add(address)
    return result, state_addresses, alert_addresses, observed_addresses


def expected_loci(
    root: Path, p1: list[dict[str, str]]
) -> tuple[set[tuple[str, str, str, str, str]], set[tuple[str, str]]]:
    """A fresh contract reading expressed as raw-corpus selection, not generator reuse."""
    result: set[tuple[str, str, str, str, str]] = set()
    loss_instances: set[tuple[str, str]] = set()
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

        # SC-509b: only fixture-proven metadata read/parse loss gains the
        # degraded field.  Multiple affected sessions in one digest coalesce
        # to its single `sessions[].degraded` output-field obligation.
        if json_output:
            document = json.loads(captured_stdout(root, invocation))
            if fixture_has_meta_loss(root, invocation):
                loss_instances.add((case, consumer))
            affected_sessions = {
                session["name"]
                for session in document.get("sessions", [])
                if isinstance(session.get("name"), str)
                and "degraded" not in session
                and actual_meta_loss(root, invocation, session["name"])
            }
            if affected_sessions:
                result.add((case, consumer, "SC-509b", "digest", "sessions[].degraded"))

        # SC-017o has two separate JSON loci.  Raw case bytes identify every
        # captured connection failure as ambient/no-server rather than an
        # entitled enumeration failure, so no human diagnostic is owed.
        if json_output:
            result.add((case, consumer, "SC-017o", "digest", "inventory_complete"))
            result.add((case, consumer, "SC-017o", "digest", "inventory_complete (value)"))

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
    return result, loss_instances


def verify_unproved_exclusions(
    path: Path,
    expected_agent_reasons: set[tuple[str, str, str, str, str, str, str, str]],
    observed_agent_addresses: dict[tuple[str, str, str, str, str], object],
    errors: list[str],
) -> None:
    header, rows = read_comment_tsv(path)
    if header != UNPROVED_HEADER:
        failure(errors, f"SC-509c exclusion header drift: {header!r}")
        return
    if len(rows) != EXPECTED_UNPROVED_COUNT:
        failure(
            errors,
            "SC-509c exclusion population drift: "
            f"expected {EXPECTED_UNPROVED_COUNT}, got {len(rows)}",
        )
    exclusions = {
        (row["case"], row["consumer"], "SC-509c", "digest", row["locus"])
        for row in rows
    }
    if len(exclusions) != len(rows):
        failure(errors, "SC-509c exclusions have duplicate ruled-grain addresses")
    for row in rows:
        address = (row["case"], row["consumer"], "SC-509c", "digest", row["locus"])
        expected_locus = f"sessions[{row['session']}].agents[{row['agent_ref']}].reason"
        if row["locus"] != expected_locus:
            failure(errors, f"SC-509c exclusion address/locus drift for {address}")
            continue
        if address not in observed_agent_addresses:
            failure(errors, f"SC-509c exclusion has no captured output address: {address}")
            continue
        if observed_agent_addresses[address] != row["session_attention"]:
            failure(errors, f"SC-509c exclusion attention drift for {address}")
    selected = {relation[:5] for relation in expected_agent_reasons}
    overlap = selected & exclusions
    if overlap:
        failure(
            errors,
            "SC-509c exclusion has independently proven raw carrier: "
            f"{sorted(overlap)[:3]!r} (total {len(overlap)})",
        )


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
    if set(keyed_rows) != set(INVENTORY_EXPECTATIONS):
        failure(
            errors,
            "inventory contract/locus rows differ: "
            f"missing {sorted(set(INVENTORY_EXPECTATIONS) - set(keyed_rows))}, "
            f"extra {sorted(set(keyed_rows) - set(INVENTORY_EXPECTATIONS))}",
        )
    for key, expected_fields in INVENTORY_EXPECTATIONS.items():
        row = keyed_rows.get(key)
        if row is None:
            failure(errors, f"inventory expected-fields row missing: {key}")
            continue
        try:
            actual_count = int(row["corpus_loci"])
        except ValueError:
            failure(
                errors,
                f"inventory corpus_loci is not an integer for {key}: {row['corpus_loci']!r}",
            )
            continue
        actual_fields = (
            row["p1_disposition"],
            row["obligation_ids"],
            actual_count,
        )
        if actual_fields != expected_fields:
            failure(
                errors,
                "inventory expected fields drift for "
                f"{key}: expected {expected_fields!r}, got {actual_fields!r}",
            )
    partial_rows = {
        (row["contract_id"], row["contract_locus"]): row
        for row in rows
        if row["p1_disposition"] == "partial corpus locus"
    }
    partial_key = ("SC-017q", "unknown per-agent liveness")
    partial_row = partial_rows.get(partial_key)
    if partial_row is None or "p1-phase4-gate.md C12" not in partial_row["pinned_successor"]:
        failure(errors, f"partial corpus locus lacks phase-4 C12 pin for {partial_key}")
    gap_rows = {
        (row["contract_id"], row["contract_locus"]): row
        for row in rows
        if row["p1_disposition"] == "directional gap"
    }
    for key in PANE_ASSOCIATION_GAPS:
        row = gap_rows.get(key)
        if row is None or "p1-phase4-gate.md C12" not in row["pinned_successor"]:
            failure(errors, f"pane-association gap lacks phase-4 C12 pin for {key}")
    for key, expected_successor in DIRECTIONAL_GAP_SUCCESSORS.items():
        row = gap_rows.get(key)
        actual_successor = None if row is None else row["pinned_successor"]
        if actual_successor != expected_successor:
            failure(
                errors,
                "directional-gap successor pin drift for "
                f"{key}: expected {expected_successor!r}, got {actual_successor!r}",
            )
    for contract_id in INVENTORY_IDS:
        if f"**{contract_id}" not in contract:
            failure(errors, f"contract no longer contains inventory row {contract_id}")
    for row in rows:
        if row["p1_disposition"] == "directional gap" and not row["pinned_successor"]:
            failure(errors, f"unnamed successor for zero-locus gap {row['contract_id']}")


def verify_payloads(rows: list[dict[str, str]], errors: list[str]) -> None:
    """Validate payload separately from identity; neither column set is implicit."""
    supports = Counter(row["support"] for row in rows)
    if supports != EXPECTED_SUPPORT_COUNTS:
        failure(errors, f"support population drift: expected {EXPECTED_SUPPORT_COUNTS}, got {dict(supports)}")
    carrying = {(row["case"], row["consumer"]) for row in rows}
    if len(carrying) != EXPECTED_CARRYING_ROWS:
        failure(errors, f"carrying-row population drift: expected {EXPECTED_CARRYING_ROWS}, got {len(carrying)}")
    sc017o_rows = [row for row in rows if row["obligation_id"] == "SC-017o"]
    if Counter(row["locus"] for row in sc017o_rows) != {
        "inventory_complete": 401,
        "inventory_complete (value)": 401,
    }:
        failure(errors, "SC-017o two-locus population drift")
    for row in sc017o_rows:
        expected = SC017O_PAYLOADS.get(row["locus"])
        actual = tuple(row[column] for column in PAYLOAD_COLUMNS)
        if expected is None or actual != expected:
            failure(errors, f"SC-017o payload shape drift at {tuple(row[column] for column in RECONCILIATION_KEY_COLUMNS)!r}")
    if any(row["obligation_id"] == "SC-017o" and row["stream"] != "digest" for row in rows):
        failure(errors, "SC-017o has a non-digest identity")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--allow-mutated-obligation-table", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    evidence = root / "docs/migration/evidence"
    corpus = evidence / "corpus"
    contract_path = root / "docs/migration/semantic-contract.md"
    gate_path = root / "docs/migration/p1-phase4-gate.md"
    inventory_path = evidence / "p1-phase4-contract-obligation-loci.tsv"
    obligations_path = corpus / "OBLIGATIONS.tsv"
    invocations_path = corpus / "INVOCATIONS.tsv"
    unproved_path = corpus / "SC-509C-UNPROVED.tsv"
    errors: list[str] = []

    required = [
        contract_path,
        gate_path,
        inventory_path,
        obligations_path,
        invocations_path,
        unproved_path,
    ]
    for path in required:
        if not path.is_file():
            failure(errors, f"missing required input: {path}")
    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1

    actual_rows = read_obligation_rows(obligations_path, errors)
    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1
    contract_bytes = contract_path.read_bytes()
    if git_blob(contract_bytes) != CONTRACT_BLOB:
        failure(errors, "contract blob drift: reconciliation must be re-derived")
    if git_blob(gate_path.read_bytes()) != GATE_BLOB:
        failure(errors, "phase-4 gate blob drift: reconciliation must be re-derived")
    if (
        not args.allow_mutated_obligation_table
        and git_blob(obligations_path.read_bytes()) != OBLIGATIONS_BLOB
    ):
        failure(errors, "accepted obligation-table blob drift: reconciliation must be re-derived")
    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1
    contract = contract_bytes.decode(errors="replace")
    verify_inventory(inventory_path, contract, errors)

    invocations = read_tsv(invocations_path)
    p1 = [row for row in invocations if row["phase"] == "P1"]
    if len(p1) != 1065:
        failure(errors, f"P1 population drift: expected 1065, got {len(p1)}")
    if len({(row["case"], row["consumer"]) for row in p1}) != len(p1):
        failure(errors, "P1 invocation keys are not unique")

    try:
        expected, loss_instances = expected_loci(root, p1)
        (
            expected_agent_reasons,
            state_addresses,
            alert_addresses,
            observed_agent_addresses,
        ) = expected_agent_reason_loci(root, p1)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        failure(errors, f"raw corpus evidence is unreadable or malformed: {exc}")
        expected = set()
        loss_instances = set()
        expected_agent_reasons = set()
        state_addresses = set()
        alert_addresses = set()
        observed_agent_addresses = {}
    verify_unproved_exclusions(
        unproved_path, expected_agent_reasons, observed_agent_addresses, errors
    )
    if state_addresses & alert_addresses:
        failure(
            errors,
            "SC-509c state and alert carriers overlap at session-and-agent grain: "
            f"{sorted(state_addresses & alert_addresses)[:3]!r}",
        )
    if not actual_rows:
        failure(errors, "OBLIGATIONS.tsv is empty")
    non_agent_rows = [row for row in actual_rows if row["obligation_id"] != "SC-509c"]
    agent_rows = [row for row in actual_rows if row["obligation_id"] == "SC-509c"]
    actual = {
        (row["case"], row["consumer"], row["obligation_id"], row["stream"], row["locus"])
        for row in non_agent_rows
    }
    actual_agent_reasons = {
        (
            row["case"],
            row["consumer"],
            row["obligation_id"],
            row["stream"],
            row["locus"],
            row["from"],
            row["to"],
            row["predicate"],
        )
        for row in agent_rows
    }
    if len(actual) != len(non_agent_rows):
        failure(errors, "OBLIGATIONS.tsv has duplicate non-SC-509c reconciliation keys")
    if len(actual_agent_reasons) != len(agent_rows):
        failure(errors, "OBLIGATIONS.tsv has duplicate SC-509c session-and-agent keys")

    actual_ids = {row["obligation_id"] for row in actual_rows}
    unexpected_ids = actual_ids - EXPECTED_IDS
    missing_ids = EXPECTED_IDS - actual_ids
    if unexpected_ids:
        failure(errors, f"orphan obligation ID(s): {sorted(unexpected_ids)}")
    if missing_ids:
        failure(errors, f"missing obligation ID(s): {sorted(missing_ids)}")
    verify_payloads(actual_rows, errors)
    if expected != actual:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        if missing:
            failure(errors, f"contract-selected locus missing from table: {missing[:3]!r} (total {len(missing)})")
        if extra:
            failure(errors, f"table locus has no independent contract/raw-corpus basis: {extra[:3]!r} (total {len(extra)})")
    if expected_agent_reasons != actual_agent_reasons:
        missing_agent_reasons = sorted(expected_agent_reasons - actual_agent_reasons)
        extra_agent_reasons = sorted(actual_agent_reasons - expected_agent_reasons)
        if missing_agent_reasons:
            failure(
                errors,
                "contract-selected SC-509c relation missing from table: "
                f"{missing_agent_reasons[:3]!r} (total {len(missing_agent_reasons)})",
            )
        if extra_agent_reasons:
            failure(
                errors,
                "table SC-509c relation has no independent contract/raw-corpus basis: "
                f"{extra_agent_reasons[:3]!r} (total {len(extra_agent_reasons)})",
            )

    counts = Counter(row["obligation_id"] for row in actual_rows)
    if counts != EXPECTED_COUNTS:
        failure(errors, f"obligation counts differ: expected {EXPECTED_COUNTS}, got {dict(sorted(counts.items()))}")
    relation_count = len(expected) + len(expected_agent_reasons)
    if relation_count != EXPECTED_RELATION_COUNT:
        failure(
            errors,
            "independent relation count drift: "
            f"expected {EXPECTED_RELATION_COUNT}, got {relation_count}",
        )
    if len(loss_instances) != EXPECTED_B_LOSS_INSTANCE_COUNT:
        failure(
            errors,
            "SC-509b raw loss-instance count drift: "
            f"expected {EXPECTED_B_LOSS_INSTANCE_COUNT}, got {len(loss_instances)}",
        )

    if errors:
        print("FAIL contract-obligation reconciliation")
        print("\n".join(errors))
        return 1
    suffix = " (accepted-table pin SKIPPED)" if args.allow_mutated_obligation_table else ""
    print(
        "PASS contract-obligation reconciliation: "
        "1843 relations / 1082 OBSERVED / 761 UNSCORABLE / contract 896d08ea"
        + suffix
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
