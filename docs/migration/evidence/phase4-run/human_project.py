#!/usr/bin/env python3
"""Human stdout projection for phase 4.

Implements the frozen comparison-projection byte-role partition
(header / layout / semantic-field / residual: total, disjoint, minimal)
and the semantic row layer. OC-P3-HUMAN-LAYOUT removes only header+layout.
OC-P3-AGENT-HEALTH-TOKEN removes only the recovered health-cell literal.
Does not write the corpus. Does not amend C8.
"""
from __future__ import annotations

import json
import os
import re
import sys
import tempfile
from collections import Counter
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass, field
from io import StringIO
from pathlib import Path

from artifact_tuple import (
    GAP_NAME,
    ArtifactTupleError,
    make_redproof_fixture,
    parse_tsv_bytes,
    read_generated_tuple,
)

REPO = Path(__file__).resolve().parents[4]
CORPUS = REPO / "docs/migration/evidence/batch-c-artifacts"
INV = REPO / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
RUN = Path(__file__).resolve().parent
# Run 2 materialises captures outside the repository.  The scorer defaults to
# its local evidence directory only for the pre-run diagnostic view.
CAPS = Path(os.environ.get("AE_P4_RUN2_OUTPUT", str(RUN))) / "captures"
SUCCESSOR_TIMEOUT_SECS = 20
HUMAN_CLOCK_BRACKET_MAX_SECS = SUCCESSOR_TIMEOUT_SECS + 1

HEALTH_MAP = {"alive": "alive", "dead": "dead", "unknown": "unknown"}
EMPTY_MSGS = (
    b"No running ae sessions. (try: ae list --all)\n",
    b"No recently active sessions.\n",
    b"No running sessions need your attention.\n",
)
AGENT_REF = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_-]*:[A-Za-z0-9_][A-Za-z0-9_-]*$")
AGENT_OWNER_LOCUS = re.compile(
    r"^agents\[(?P<session>[^:\]]+):"
    r"(?P<ref>[A-Za-z0-9_][A-Za-z0-9_-]*:[A-Za-z0-9_][A-Za-z0-9_-]*):"
    r"(?P<session_id>[^:\]]+)\](?P<class>\(class\))?\."
    r"(?P<member>health|state)$"
)
COUNTER_VALUE = re.compile(r"^(?P<value>.+) x(?P<count>[1-9][0-9]*)$")


def normalise_rendered_session_id(value: object) -> str | None:
    """Accept exactly the frozen rendered short-session-id grammar.

    The product truncates by Unicode character under the frozen corpus's UTF-8
    locale premise, so Python's code-point len is the matching evidence grammar:
    dash for the empty/pending arm, otherwise the whole non-pending identifier
    when it is one through eight characters.
    Literal ``pending`` is producer-normalized-away: ``display_session_id``
    must render dash, so accepting its seven characters would launder a
    producer failure into a presented identity.
    """
    if not isinstance(value, str) or "\t" in value or "\n" in value:
        return None
    if value == "-":
        return value
    if value and value != "pending" and len(value) <= 8:
        return value
    return None


@dataclass
class AgentSem:
    ref: str
    session_id: str | None
    health_token: str | None
    health_sem: str | None
    state: str | None


@dataclass
class SessionSem:
    name: str
    status: str | None
    attn: str | None
    agents: list[AgentSem] = field(default_factory=list)


@dataclass(frozen=True)
class TemporalSpan:
    session: str
    kind: str
    rendered: bytes


@dataclass(frozen=True)
class EpochFacts:
    goal_set_epoch: object | None
    last_active_epoch: object | None


@dataclass(frozen=True)
class ClockBracket:
    before_epoch: int
    after_epoch: int


class TemporalFixtureError(RuntimeError):
    """The runner did not bind enough fixed evidence to score a human clock span."""


@dataclass(frozen=True)
class RoleSpan:
    """One contiguous raw-byte interval assigned to exactly one closed role."""

    start: int
    end: int
    role: str
    data: bytes


class RoleRecorder:
    """Assign raw bytes once, in order, while preserving role aggregates."""

    ROLES = frozenset({"header", "layout", "semantic", "residual"})

    def __init__(self, raw: bytes):
        self.raw = raw
        self.cursor = 0
        self.parts = {role: bytearray() for role in self.ROLES}
        self.spans: list[RoleSpan] = []
        self.error: str | None = None

    def put(self, role: str, data: bytes) -> None:
        if not data:
            return
        start = self.cursor
        end = start + len(data)
        if role not in self.ROLES:
            self.error = self.error or f"unknown projection role {role!r}"
        if end > len(self.raw) or self.raw[start:end] != data:
            self.error = self.error or f"projection byte assignment diverged at [{start},{end})"
        self.parts.get(role, bytearray()).extend(data)
        self.spans.append(RoleSpan(start, end, role, data))
        self.cursor = end

    def apply(self, projection: "Projection") -> None:
        projection.header = bytes(self.parts["header"])
        projection.layout = bytes(self.parts["layout"])
        projection.semantic = bytes(self.parts["semantic"])
        projection.residual = bytes(self.parts["residual"])
        projection.spans = self.spans
        projection.parse_ok = self.error is None and _roles_ok(projection)
        projection.note = (
            "ok"
            if projection.parse_ok
            else self.error or f"invalid role-span tile at cursor {self.cursor}/{len(self.raw)}"
        )


@dataclass
class Projection:
    sessions: list[SessionSem]
    temporal: list[TemporalSpan] = field(default_factory=list)
    header: bytes = b""
    layout: bytes = b""
    semantic: bytes = b""
    residual: bytes = b""
    raw: bytes = b""
    spans: list[RoleSpan] = field(default_factory=list)
    parse_ok: bool = True
    note: str = ""

    def covered(self) -> int:
        return len(self.header) + len(self.layout) + len(self.semantic) + len(self.residual)


def _roles_ok(p: Projection) -> bool:
    cursor = 0
    aggregates = {role: bytearray() for role in RoleRecorder.ROLES}
    for span in p.spans:
        if (
            span.role not in RoleRecorder.ROLES
            or span.start != cursor
            or span.end <= span.start
            or span.end - span.start != len(span.data)
            or span.end > len(p.raw)
            or p.raw[span.start:span.end] != span.data
        ):
            return False
        aggregates[span.role].extend(span.data)
        cursor = span.end
    return (
        cursor == len(p.raw)
        and p.header == bytes(aggregates["header"])
        and p.layout == bytes(aggregates["layout"])
        and p.semantic == bytes(aggregates["semantic"])
        and p.residual == bytes(aggregates["residual"])
    )


def _split_nl(raw: bytes) -> list[tuple[bytes, bytes]]:
    """[(line_without_nl, terminator)] covering raw exactly."""
    out = []
    i = 0
    while i < len(raw):
        j = raw.find(b"\n", i)
        if j < 0:
            out.append((raw[i:], b""))
            break
        out.append((raw[i:j], b"\n"))
        i = j + 1
    return out


def _put_temporal_subline(
    line_b: bytes,
    nl: bytes,
    session: str | None,
    roles: RoleRecorder,
    temporal: list[TemporalSpan],
) -> tuple[bool, str | None]:
    """Project only formatter values from a frozen goal/git/version/active line.

    Its static text is residual and remains exact.  The pair returned tells the
    caller whether this was a subline and, independently, whether it carried
    the tabular session's `attn:` field.
    """
    if session is None:
        return False, None
    active_marker = b" \xc2\xb7 active "
    active_marker_at = line_b.rfind(active_marker)
    if active_marker_at < 0:
        return False, None
    active_start = active_marker_at + len(active_marker)
    active_end = line_b.find(b" \xc2\xb7 ", active_start)
    if active_end < 0:
        active_end = len(line_b)
    if active_start == active_end:
        return False, None

    atoms: list[tuple[int, int, str, bytes]] = [
        (active_start, active_end, "active", line_b[active_start:active_end])
    ]
    goal_prefix = b"  goal ("
    if line_b.startswith(goal_prefix):
        goal_start = len(goal_prefix)
        goal_end = line_b.find(b"):", goal_start)
        if goal_end < 0 or goal_end == goal_start or goal_end >= active_marker_at:
            return False, None
        atoms.append((goal_start, goal_end, "goal", line_b[goal_start:goal_end]))
    attn_match = re.search(rb"attn:([A-Za-z0-9_-]+)", line_b)
    attn = attn_match.group(1).decode("ascii") if attn_match else None
    if attn_match:
        atoms.append((attn_match.start(), attn_match.end(), "attn", attn_match.group(0)))
    atoms.sort()
    cursor = 0
    for start, end, kind, value in atoms:
        if start < cursor:
            return False, None
        roles.put("residual", line_b[cursor:start])
        roles.put("semantic", value)
        if kind != "attn":
            temporal.append(TemporalSpan(session, kind, value))
        cursor = end
    roles.put("residual", line_b[cursor:] + nl)
    return True, attn


def project_successor(raw: bytes) -> Projection:
    """Successor table: name\\tstatus[\\tattn:reason]\\n  ref\\tshort-id\\thealth\\tstate\\n."""
    p = Projection(sessions=[], raw=raw)
    if raw in EMPTY_MSGS or raw in (b"", b"\n"):
        roles = RoleRecorder(raw)
        roles.put("residual", raw)
        roles.apply(p)
        p.note = "empty-or-msg" if p.parse_ok else p.note
        return p
    roles = RoleRecorder(raw)
    sessions: list[SessionSem] = []
    temporal: list[TemporalSpan] = []
    semantic_problem: str | None = None

    def put(kind: str, chunk: bytes) -> None:
        roles.put(kind, chunk)

    for line_b, nl in _split_nl(raw):
        line = line_b.decode("utf-8", "replace")
        if line.startswith("  "):
            parts = line[2:].split("\t")
            if len(parts) != 4:
                recognized, attn = _put_temporal_subline(
                    line_b, nl, sessions[-1].name if sessions else None, roles, temporal
                )
                if recognized:
                    if attn is not None:
                        sessions[-1].attn = attn
                    continue
                if parts and AGENT_REF.fullmatch(parts[0]):
                    semantic_problem = f"malformed successor agent row {line!r}"
                put("residual", line_b + nl)
                continue
            ref, session_id, health, state = parts
            normalised_session_id = normalise_rendered_session_id(session_id)
            put("layout", b"  ")
            put("semantic", ref.encode())
            put("layout", b"\t")
            put("semantic", session_id.encode())
            put("layout", b"\t")
            put("semantic", health.encode())
            put("layout", b"\t")
            put("semantic", state.encode())
            put("layout", nl)
            if not AGENT_REF.fullmatch(ref):
                semantic_problem = f"unmapped successor agent ref {ref!r}"
            elif normalised_session_id is None:
                semantic_problem = f"malformed successor short session-id {session_id!r}"
            elif health not in HEALTH_MAP:
                semantic_problem = f"unmapped successor agent-health token {health!r}"
            if sessions:
                sessions[-1].agents.append(
                    AgentSem(
                        ref,
                        normalised_session_id,
                        health,
                        HEALTH_MAP.get(health),
                        state,
                    )
                )
            continue
        parts = line.split("\t")
        if len(parts) < 2:
            put("residual", line_b + nl)
            continue
        name, status = parts[0], parts[1]
        attn = None
        put("semantic", name.encode())
        put("layout", b"\t")
        put("semantic", status.encode())
        if len(parts) >= 3 and parts[2].startswith("attn:"):
            attn = parts[2][5:]
            put("layout", b"\t")
            put("semantic", parts[2].encode())
        put("layout", nl)
        sessions.append(SessionSem(name, status, attn, []))
    p.sessions = sessions
    p.temporal = temporal
    roles.apply(p)
    if semantic_problem is not None:
        p.parse_ok = False
        p.note = semantic_problem
    return p


def _put_line(line_b: bytes, nl: bytes, name: str, status: str, roles: RoleRecorder) -> None:
    idx_name = line_b.find(name.encode())
    if idx_name < 0:
        roles.put("residual", line_b + nl)
        return
    roles.put("layout", line_b[:idx_name])
    roles.put("semantic", name.encode())
    rest = line_b[idx_name + len(name.encode()) :]
    st = status.encode()
    j = rest.find(st)
    if j < 0:
        roles.put("residual", rest + nl)
        return
    roles.put("layout", rest[:j])
    roles.put("semantic", st)
    # MODE and ORIGIN have no paired successor fields or directional rows. They
    # are residual, not layout: layout is only spacing around recovered fields.
    # Preserve the surrounding spaces/newline as layout, so an otherwise
    # name/status-only row does not manufacture a residual newline mismatch.
    tail = rest[j + len(st) :]
    if not tail.strip(b" \t"):
        roles.put("layout", tail + nl)
        return
    leading = len(tail) - len(tail.lstrip(b" \t"))
    core_end = len(tail.rstrip(b" \t"))
    roles.put("layout", tail[:leading])
    roles.put("residual", tail[leading:core_end])
    roles.put("layout", tail[core_end:] + nl)


def project_baseline(raw: bytes) -> Projection:
    """Frozen bash tabular / empty-state listing."""
    p = Projection(sessions=[], raw=raw)
    if raw in EMPTY_MSGS or raw in (b"", b"\n"):
        roles = RoleRecorder(raw)
        roles.put("residual", raw)
        roles.apply(p)
        p.note = "empty-or-msg" if p.parse_ok else p.note
        return p
    roles = RoleRecorder(raw)
    sessions: list[SessionSem] = []
    temporal: list[TemporalSpan] = []
    semantic_problem: str | None = None

    def put(kind: str, chunk: bytes) -> None:
        roles.put(kind, chunk)

    first = True
    for line_b, nl in _split_nl(raw):
        line = line_b.decode("utf-8", "replace")
        if first and line.startswith("SESSION"):
            put("header", line_b + nl)
            first = False
            continue
        first = False
        if line.startswith("No "):
            put("residual", line_b + nl)
            continue
        if line.startswith("  "):
            # `git:master ... active ...` is a temporal subline, even though
            # its first token happens to match the agent-ref grammar.  Decide
            # that grammar first so a session's metadata never manufactures an
            # agent-row semantic fact. The retired agent-first temporal census
            # is a parser-defect measurement, not a second population: classify
            # this line by grammar, never by an aggregate-count backdoor.
            recognized, attn = _put_temporal_subline(
                line_b, nl, sessions[-1].name if sessions else None, roles, temporal
            )
            if recognized:
                if attn is not None:
                    sessions[-1].attn = attn
                continue
            stripped = line.lstrip(" ")
            first_tok = stripped.split()[0] if stripped.split() else ""
            if AGENT_REF.match(first_tok):
                indent_n = len(line) - len(line.lstrip(" "))
                cols = re.split(r"[ \t]{2,}", stripped.strip())
                ref = cols[0] if cols else first_tok
                # Frozen bash rows are selected by their enclosing session, not
                # by a column-count guess: running is REF / SID / state /
                # optional marker; stopped is REF / SID only. A stopped row's
                # formatter has no trailing cell or trailing space, which is
                # the fixed stopped-space discriminator.
                session_status = sessions[-1].status if sessions else None
                session_id = normalise_rendered_session_id(cols[1]) if len(cols) > 1 else None
                state: str | None = None
                health_token: str | None = None
                health_sem: str | None = None
                if session_id is None:
                    semantic_problem = f"frozen agent row lacks short session-id for {ref!r}"
                elif session_status == "stopped":
                    if len(cols) != 2 or stripped.rstrip(" \t") != stripped:
                        semantic_problem = f"malformed frozen stopped agent row for {ref!r}"
                elif session_status == "running":
                    if len(cols) not in (3, 4):
                        semantic_problem = f"malformed frozen running agent row for {ref!r}"
                    else:
                        state = cols[2]
                        health_token = cols[3] if len(cols) == 4 else ""
                        if health_token not in ("", "!"):
                            semantic_problem = (
                                "malformed frozen agent health marker "
                                f"for {ref!r}: {health_token!r}"
                            )
                        elif health_token == "":
                            # Parsing retains the blank carrier; its separate
                            # semantic mapping is alive.
                            health_sem = "alive"
                        else:
                            # Parsing retains literal bang; its separate semantic
                            # mapping is dead rather than the spelling itself.
                            health_sem = "dead"
                else:
                    semantic_problem = f"agent row has no running/stopped session carrier for {ref!r}"
                put("layout", line_b[:indent_n])
                mid = line_b[indent_n:]
                put("semantic", ref.encode())
                mid = mid[len(ref.encode()) :]
                if session_id:
                    i = mid.find(session_id.encode())
                    if i >= 0:
                        put("layout", mid[:i])
                        put("semantic", session_id.encode())
                        mid = mid[i + len(session_id.encode()) :]
                if state:
                    i = mid.find(state.encode())
                    if i >= 0:
                        put("layout", mid[:i])
                        put("semantic", state.encode())
                        mid = mid[i + len(state.encode()) :]
                if health_token:
                    i = mid.find(health_token.encode())
                    if i >= 0:
                        put("layout", mid[:i])
                        put("semantic", health_token.encode())
                        mid = mid[i + len(health_token.encode()) :]
                put("layout", mid + nl)
                if sessions:
                    sessions[-1].agents.append(
                        AgentSem(ref, session_id, health_token, health_sem, state)
                    )
                continue
            attn_m = re.search(r"attn:([A-Za-z0-9_-]+)", line)
            if attn_m and sessions:
                sessions[-1].attn = attn_m.group(1)
                put("residual", line_b[: attn_m.start()])
                put("semantic", attn_m.group(0).encode())
                put("residual", line_b[attn_m.end() :] + nl)
            else:
                put("residual", line_b + nl)
            continue
        cols = re.split(r"[ \t]{2,}", line.strip())
        if len(cols) < 2:
            put("residual", line_b + nl)
            continue
        name, status = cols[0], cols[1]
        _put_line(line_b, nl, name, status, roles)
        sessions.append(SessionSem(name, status, None, []))

    p.sessions = sessions
    p.temporal = temporal
    roles.apply(p)
    if semantic_problem is not None:
        p.parse_ok = False
        p.note = semantic_problem
    return p


def load_tsv(path: Path):
    rows = [ln.split("\t") for ln in path.read_text(encoding="utf-8").splitlines() if ln]
    return rows[0], [dict(zip(rows[0], r)) for r in rows[1:]]


OBS_SHAPES: dict[tuple[str, str], list[dict]] = {}


def load_observed_shapes():
    """Read one verified tuple lazily; import and redproof stay runnable if it is bad."""
    snapshot = read_generated_tuple(REPO)
    _, rows = parse_tsv_bytes(snapshot.obligations, "saved OBLIGATIONS snapshot")
    shapes: dict[tuple[str, str], list[dict]] = {}
    for row in rows:
        if row["support"] == "OBSERVED":
            shapes.setdefault((row["case"], row["consumer"]), []).append(row)
    return snapshot, shapes


_, _INV_ROWS = load_tsv(INV)
_EPOCH_FACTS_CACHE: dict[tuple[str, str], EpochFacts] = {}


def _parse_json_object(raw: bytes, source: Path) -> dict:
    def unique(pairs: list[tuple[str, object]]) -> dict:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise TemporalFixtureError(f"duplicate JSON key in epoch source {source}: {key!r}")
            result[key] = value
        return result

    try:
        document = json.loads(raw.decode("utf-8"), object_pairs_hook=unique)
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise TemporalFixtureError(f"malformed paired JSON epoch source {source}: {error}") from error
    if not isinstance(document, dict) or not isinstance(document.get("sessions"), list):
        raise TemporalFixtureError(f"paired JSON epoch source lacks sessions[]: {source}")
    return document


def _typed_value(value: object) -> tuple[type, object]:
    """Keep 1 and true distinct while checking cross-consumer source consistency."""
    return type(value), value


def epoch_facts(case_dir: str, session: str) -> EpochFacts:
    """Read the fixed session source from every same-case frozen JSON capture.

    This deliberately does not infer a JSON consumer from the human consumer.
    The source is all paired JSON captures in the exact case; the facts must be
    present and type-identical wherever that session appears.
    """
    key = (case_dir, session)
    if key in _EPOCH_FACTS_CACHE:
        return _EPOCH_FACTS_CACHE[key]
    paths = []
    for row in _INV_ROWS:
        if str(Path(row["case"]).parent) != case_dir:
            continue
        if "--json" not in row["normalised_argv"].split():
            continue
        path = CORPUS / case_dir / "out" / f"{row['consumer']}.stdout"
        if not path.is_file():
            raise TemporalFixtureError(f"paired JSON capture absent: {path}")
        paths.append(path)
    if not paths:
        raise TemporalFixtureError(f"no paired JSON captures for {case_dir}/{session}")

    facts: list[tuple[bool, object, bool, object]] = []
    for path in sorted(set(paths)):
        document = _parse_json_object(path.read_bytes(), path)
        matches = [entry for entry in document["sessions"] if isinstance(entry, dict) and entry.get("name") == session]
        if len(matches) > 1:
            raise TemporalFixtureError(f"paired JSON has duplicate session {session!r}: {path}")
        if not matches:
            continue
        record = matches[0]
        facts.append((
            "goal_set_epoch" in record,
            record.get("goal_set_epoch"),
            "last_active_epoch" in record,
            record.get("last_active_epoch"),
        ))
    if not facts:
        raise TemporalFixtureError(f"paired JSON never names human session {session!r} in {case_dir}")
    first = facts[0]
    if any(
        (goal_present, _typed_value(goal), active_present, _typed_value(active))
        != (first[0], _typed_value(first[1]), first[2], _typed_value(first[3]))
        for goal_present, goal, active_present, active in facts[1:]
    ):
        raise TemporalFixtureError(f"inconsistent paired JSON epochs for {case_dir}/{session}")
    goal_present, goal, active_present, active = first
    result = EpochFacts(goal if goal_present else None, active if active_present else None)
    _EPOCH_FACTS_CACHE[key] = result
    return result


def frozen_relative_time(now: int, epoch: int | None) -> bytes:
    """Exact listing.rs formatter, supplied time only: no ambient-clock fallback."""
    if epoch is None or epoch <= 0:
        return b"-"
    delta = now - epoch
    if delta < 0:
        return b"just now"
    if delta < 60:
        return f"{delta}s ago".encode("ascii")
    if delta < 3_600:
        return f"{delta // 60}m ago".encode("ascii")
    if delta < 86_400:
        return f"{delta // 3_600}h ago".encode("ascii")
    if delta < 604_800:
        return f"{delta // 86_400}d ago".encode("ascii")
    return b">7d"


def census_epoch_sources() -> tuple[int, int]:
    """Verify the fixed same-case JSON source for every frozen temporal row."""
    human_rows = 0
    temporal_spans = 0
    for row in _INV_ROWS:
        if row["phase"] != "P1" or row["surface"] not in ("ae list", "ae ls"):
            continue
        if "--json" in row["normalised_argv"].split():
            continue
        case_dir = str(Path(row["case"]).parent)
        baseline = CORPUS / case_dir / "out" / f"{row['consumer']}.stdout"
        projection = project_baseline(baseline.read_bytes())
        if not projection.temporal:
            continue
        human_rows += 1
        temporal_spans += len(projection.temporal)
        for span in projection.temporal:
            facts = epoch_facts(case_dir, span.session)
            value = facts.goal_set_epoch if span.kind == "goal" else facts.last_active_epoch
            if span.kind == "goal" and (type(value) is not int or value <= 0):
                raise TemporalFixtureError(f"goal_set_epoch unavailable/non-integer for {case_dir}/{span.session}")
            if span.kind == "active" and value is not None and type(value) is not int:
                raise TemporalFixtureError(f"last_active_epoch is not integer/null for {case_dir}/{span.session}")
    return human_rows, temporal_spans


def _parse_int(fields: dict[str, str], key: str) -> int:
    value = fields.get(key)
    if value is None or not re.fullmatch(r"0|[1-9][0-9]*", value):
        raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-MALFORMED {key}")
    return int(value)


def load_clock_bracket(capture: Path, index: int, row: dict[str, str]) -> ClockBracket:
    """Read one exact runner binding; an absent or drifting record is a fixture abort."""
    path = capture / "human-clock-bracket.tsv"
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-MISSING {path}: {error}") from error
    fields: dict[str, str] = {}
    for line in lines:
        if line.count("\t") != 1:
            raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-MALFORMED row {line!r}")
        key, value = line.split("\t", 1)
        if not key or key in fields:
            raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-MALFORMED duplicate/blank key {key!r}")
        fields[key] = value
    expected_tokens = row["normalised_argv"].split()
    if expected_tokens and expected_tokens[0] == "ae":
        expected_tokens = expected_tokens[1:]
    expected = {
        "schema": "1",
        "index": str(index),
        "case": row["case"],
        "consumer": row["consumer"],
        "surface": row["surface"],
        "normalised_argv": row["normalised_argv"],
        "successor_argv": " ".join(expected_tokens),
        "max_width_seconds": str(HUMAN_CLOCK_BRACKET_MAX_SECS),
    }
    if set(fields) != {
        *expected,
        "before_epoch",
        "after_epoch",
        "before_monotonic_ns",
        "after_monotonic_ns",
        "monotonic_elapsed_ns",
        "width_seconds",
    }:
        raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-MALFORMED key set {sorted(fields)!r}")
    for key, value in expected.items():
        if fields[key] != value:
            raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-BINDING {key} {fields[key]!r} != {value!r}")
    before = _parse_int(fields, "before_epoch")
    after = _parse_int(fields, "after_epoch")
    before_monotonic = _parse_int(fields, "before_monotonic_ns")
    after_monotonic = _parse_int(fields, "after_monotonic_ns")
    width = _parse_int(fields, "width_seconds")
    monotonic_elapsed = _parse_int(fields, "monotonic_elapsed_ns")
    if after < before:
        raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-BACKWARD before={before} after={after}")
    if after - before != width:
        raise TemporalFixtureError("HUMAN-CLOCK-BRACKET-MALFORMED UTC width")
    if width > HUMAN_CLOCK_BRACKET_MAX_SECS:
        raise TemporalFixtureError(f"HUMAN-CLOCK-BRACKET-UNBOUNDED width={width}")
    if after_monotonic < before_monotonic or after_monotonic - before_monotonic != monotonic_elapsed:
        raise TemporalFixtureError("CLOCK-BRACKET-DISCONTINUITY monotonic endpoints")
    if abs(width - monotonic_elapsed / 1_000_000_000) > 1:
        raise TemporalFixtureError(
            "CLOCK-BRACKET-DISCONTINUITY "
            f"utc_width={width} monotonic_elapsed_ns={monotonic_elapsed}"
        )
    return ClockBracket(before, after)


def temporal_verdict(
    base: Projection,
    suc: Projection,
    case_dir: str,
    bracket: ClockBracket | None,
    facts_for_session=epoch_facts,
) -> tuple[str, str] | None:
    """Require one UTC witness that validates every temporal span in this stdout."""
    if not base.temporal and not suc.temporal:
        return None
    if bracket is None:
        return "fixture-abort", "HUMAN-CLOCK-BRACKET-MISSING for temporal human output"
    base_shape = [(span.session, span.kind) for span in base.temporal]
    successor_shape = [(span.session, span.kind) for span in suc.temporal]
    if base_shape != successor_shape:
        return "semantic-fail", f"temporal span shape {base_shape!r} vs {successor_shape!r}"
    universe = set(range(bracket.before_epoch, bracket.after_epoch + 1))
    witnesses = set(universe)
    for base_span, successor_span in zip(base.temporal, suc.temporal):
        facts = facts_for_session(case_dir, base_span.session)
        if base_span.kind == "goal":
            epoch = facts.goal_set_epoch
            if type(epoch) is not int or epoch <= 0:
                raise TemporalFixtureError(f"goal_set_epoch unavailable/non-integer for {case_dir}/{base_span.session}")
        elif base_span.kind == "active":
            epoch = facts.last_active_epoch
            if epoch is not None and type(epoch) is not int:
                raise TemporalFixtureError(f"last_active_epoch is not integer/null for {case_dir}/{base_span.session}")
        else:
            raise TemporalFixtureError(f"unknown temporal span kind {base_span.kind!r}")
        accepted = {
            now
            for now in universe
            if frozen_relative_time(now, epoch) == successor_span.rendered
        }
        if not accepted:
            expected = sorted({frozen_relative_time(now, epoch) for now in universe})
            return (
                "semantic-fail",
                f"temporal {base_span.session}/{base_span.kind} {successor_span.rendered!r} not in {expected!r}",
            )
        witnesses &= accepted
    if not witnesses:
        return "semantic-fail", "temporal spans have no common UTC witness"
    return None


def _observed_shapes(case_dir: str, consumer: str, obligation_id: str) -> list[dict]:
    return [
        row
        for row in OBS_SHAPES.get((case_dir, consumer), [])
        if row["obligation_id"] == obligation_id
    ]


def _obligation_value(value: str | None) -> str:
    return "null" if value is None else value


def _owns_exact_transition(
    shapes: list[dict], before: str | None, after: str | None, locus_word: str
) -> bool:
    """Only an exact observed tuple for this field can relax it."""
    return any(
        locus_word in row["locus"].lower()
        and row["from"] == _obligation_value(before)
        and row["to"] == _obligation_value(after)
        for row in shapes
    )


def _source_health_carrier(agent: AgentSem) -> str:
    """Translate parsed carriers into table spellings without losing their source class.

    Parser state retains literal bang in ``health_token`` and separately maps it
    to semantic dead in ``health_sem``. The table-facing source spelling remains
    literal ``!``; blank and ABSENT stay distinct carriers.
    """
    if agent.health_token is None:
        return "ABSENT"
    if agent.health_token == "":
        return "blank"
    if agent.health_token == "!":
        return "!"
    if agent.health_sem is not None:
        return agent.health_sem
    return f"INVALID({agent.health_token!r})"


def _target_health_value(agent: AgentSem) -> str:
    if agent.health_sem == "unknown":
        return "unambiguous unknown"
    if agent.health_sem is not None:
        return agent.health_sem
    return "ABSENT"


def _source_state_carrier(agent: AgentSem) -> str:
    return "ABSENT" if agent.state is None else agent.state


def _target_state_value(agent: AgentSem) -> str:
    return "ABSENT" if agent.state is None else agent.state


def _counter_value(value: str) -> Counter[str]:
    """Expand the final table's exact class-multiset value spelling."""
    match = COUNTER_VALUE.fullmatch(value)
    if match is None:
        return Counter({value: 1})
    return Counter({match.group("value"): int(match.group("count"))})


def _agent_identity(session: str, agent: AgentSem) -> tuple[str, str, str] | None:
    session_id = normalise_rendered_session_id(agent.session_id)
    if session_id is None:
        return None
    return session, agent.ref, session_id


def _agent_groups(session: str, agents: list[AgentSem]) -> dict[tuple[str, str, str], list[AgentSem]] | None:
    groups: dict[tuple[str, str, str], list[AgentSem]] = {}
    for agent in agents:
        identity = _agent_identity(session, agent)
        if identity is None:
            return None
        groups.setdefault(identity, []).append(agent)
    return groups


def _agent_identity_sequence(
    session: str, agents: list[AgentSem]
) -> list[tuple[str, str, str]] | None:
    """Keep retained semantic agent-row order outside explicit collision classes."""
    sequence: list[tuple[str, str, str]] = []
    for agent in agents:
        identity = _agent_identity(session, agent)
        if identity is None:
            return None
        sequence.append(identity)
    return sequence


def _owner_relation_holds(
    shapes: list[dict],
    member: str,
    identity: tuple[str, str, str],
    source: Counter[str],
    target: Counter[str],
) -> tuple[bool, str]:
    """Apply h/r only to one exact rendered identity and its full multiplicity.

    A same-display collision has no stable row order. Its class rows therefore
    owe complete source-carrier and target-value Counters; a single owner can
    never multiply across two rendered agents.
    """
    relevant: list[tuple[dict, re.Match[str]]] = []
    for row in shapes:
        match = AGENT_OWNER_LOCUS.fullmatch(row["locus"])
        if match is None or match.group("member") != member:
            continue
        owner_identity = (
            match.group("session"),
            match.group("ref"),
            match.group("session_id"),
        )
        if owner_identity == identity:
            relevant.append((row, match))
    if not relevant:
        return source == target, "default parity"

    collision = sum(source.values()) > 1
    if any(bool(match.group("class")) != collision for _row, match in relevant):
        return False, "owner locus class grain disagrees with rendered identity multiplicity"

    owed_source: Counter[str] = Counter()
    owed_target: Counter[str] = Counter()
    for row, _match in relevant:
        owed_source.update(_counter_value(row["from"]))
        owed_target.update(_counter_value(row["to"]))
    if source != owed_source or target != owed_target:
        return (
            False,
            f"source={dict(source)!r} target={dict(target)!r} "
            f"owner_source={dict(owed_source)!r} owner_target={dict(owed_target)!r}",
        )
    return True, "exact owner counter"


def _semantic_projection(
    base: Projection, suc: Projection, case_dir: str, consumer: str
) -> tuple[str, str] | None:
    """Compare all recovered values before applying byte-role exclusions.

    An UNSCORABLE row is evidence of a gap, not permission to accept a change.
    OBSERVED directional rows authorize only their own exact from/to relation.
    """
    l_shapes = _observed_shapes(case_dir, consumer, "SC-017l")
    h_shapes = _observed_shapes(case_dir, consumer, "SC-017h")
    r_shapes = _observed_shapes(case_dir, consumer, "SC-017r")
    bnames = [session.name for session in base.sessions]
    snames = [session.name for session in suc.sessions]
    if len(set(bnames)) != len(bnames) or len(set(snames)) != len(snames):
        return "semantic-fail", "duplicate session identity"
    bmap = {session.name: session for session in base.sessions}
    smap = {session.name: session for session in suc.sessions}

    # Compare the intersection first. A membership delta cannot hide a changed
    # attn marker, health token, state, or roster on another shared session.
    bshared = [name for name in bnames if name in smap]
    sshared = [name for name in snames if name in bmap]
    if bshared != sshared:
        return "semantic-fail", f"shared-order {bshared} vs {sshared}"
    for name in bshared:
        b, s = bmap[name], smap[name]
        if b.status != s.status:
            allowed = (
                b.status == "stopped"
                and s.status == "unknown"
                and _owns_exact_transition(l_shapes, "stopped", "unknown", "status")
            )
            if not allowed:
                return "semantic-fail", f"status {name} {b.status!r}->{s.status!r}"
        if b.attn != s.attn and not _owns_exact_transition(h_shapes, b.attn, s.attn, "attn"):
            return "semantic-fail", f"attn {name} {b.attn!r}->{s.attn!r}"
        b_identity_sequence = _agent_identity_sequence(name, b.agents)
        s_identity_sequence = _agent_identity_sequence(name, s.agents)
        if b_identity_sequence is None or s_identity_sequence is None:
            return "semantic-fail", f"agent {name} lacks a normalized rendered short session-id"
        # OC-P3-HUMAN-LAYOUT removes layout only: retained semantic rows stay
        # ordered. An exchange is neutral only inside one complete collision
        # class, where every swapped entry has the same full projection key.
        if b_identity_sequence != s_identity_sequence:
            return (
                "semantic-fail",
                f"agent identity order {b_identity_sequence!r} vs {s_identity_sequence!r}",
            )
        b_agents = _agent_groups(name, b.agents)
        s_agents = _agent_groups(name, s.agents)
        if b_agents is None or s_agents is None:
            return "semantic-fail", f"agent {name} lacks a normalized rendered short session-id"
        if set(b_agents) != set(s_agents):
            return (
                "semantic-fail",
                f"agent identities {sorted(b_agents)} vs {sorted(s_agents)}",
            )
        for identity in sorted(b_agents):
            baseline_agents = b_agents[identity]
            successor_agents = s_agents[identity]
            if len(baseline_agents) != len(successor_agents):
                return (
                    "semantic-fail",
                    f"agent identity {identity!r} multiplicity "
                    f"{len(baseline_agents)}->{len(successor_agents)}",
                )
            health_ok, health_detail = _owner_relation_holds(
                r_shapes,
                "health",
                identity,
                Counter(_source_health_carrier(agent) for agent in baseline_agents),
                Counter(_target_health_value(agent) for agent in successor_agents),
            )
            if not health_ok:
                return "semantic-fail", f"health {identity!r} {health_detail}"
            state_ok, state_detail = _owner_relation_holds(
                h_shapes,
                "state",
                identity,
                Counter(_source_state_carrier(agent) for agent in baseline_agents),
                Counter(_target_state_value(agent) for agent in successor_agents),
            )
            if not state_ok:
                return "semantic-fail", f"state {identity!r} {state_detail}"

    missing = [name for name in bnames if name not in smap]
    if missing:
        # SC-017l owns an in-place stopped -> unknown status transition.  It
        # never authorizes removal: SC-017m's row-set presence requirement is
        # the independent guard that keeps the unknown session visible.
        return "semantic-fail", f"missing retained session {missing}"
    extra = [name for name in snames if name not in bmap]
    if extra:
        # The final SC-017m owner is an exact per-view candidate-identity /
        # status set, not this draft's superseded generic row-set tuple.  Do
        # not admit any membership addition until the C1-pinned final grammar
        # is implemented and its OBSERVED owner can be compared exactly.
        return "semantic-fail", f"unowned extra {extra} (SC-017m final view-set owner not pinned)"
    return None


def compare(
    base: Projection,
    suc: Projection,
    case_dir: str,
    consumer: str,
    bracket: ClockBracket | None = None,
    facts_for_session=epoch_facts,
) -> tuple[str, str]:
    """Return a fixed human-projection verdict; no bracket is a fixture error."""
    if not base.parse_ok or not _roles_ok(base):
        return "parse-fail", f"baseline {base.note}"
    if not suc.parse_ok or not _roles_ok(suc):
        return "parse-fail", f"successor {suc.note}"
    semantic = _semantic_projection(base, suc, case_dir, consumer)
    if semantic is not None:
        return semantic
    temporal = temporal_verdict(base, suc, case_dir, bracket, facts_for_session)
    if temporal is not None:
        return temporal
    if base.raw == suc.raw:
        return "exact", "bytes"

    # residual: layout-open may not drop residual. Empty-state msg vs empty
    # with zero semantic on both sides is residual-fail.
    if not base.sessions and not suc.sessions:
        br = base.residual
        sr = suc.residual
        if br != sr:
            if br in EMPTY_MSGS and sr in (b"", b"\n"):
                return "residual-fail", "empty-state-copy vs empty"
            if br != sr:
                return "residual-fail", f"residual {br[:40]!r} vs {sr[:40]!r}"
        return "layout-open", "no-semantic-rows"

    # OC-P3-HUMAN-LAYOUT removes header/layout only. Goal-age and active-age
    # values have already passed the one-witness temporal check above; every
    # other subline byte (goal text, git/version, punctuation) remains residual
    # exact. Missing residual is never reclassified as layout.
    if base.residual != suc.residual:
        return "residual-fail", f"DIVERGENCE residual {base.residual[:40]!r} vs {suc.residual[:40]!r}"
    return "layout-open", "semantic-held; residual exact"


def redproof() -> None:
    """Projection and semantic false-pass red proofs against direct mutations."""
    # This is deliberately an end-to-end call of main(), not a mocked reader:
    # a corrupt generated member must leave human projection runnable as a
    # redproof command while main refuses before opening an output.
    global REPO, RUN
    original_repo, original_run = REPO, RUN
    try:
        with tempfile.TemporaryDirectory(prefix="ae-human-tuple-") as temp:
            root = Path(temp)
            fixture_root = root / "tuple"
            paths, _obligations = make_redproof_fixture(fixture_root)
            paths[GAP_NAME].write_bytes(paths[GAP_NAME].read_bytes() + b"new\tview\tsession\n")
            REPO = fixture_root
            RUN = root / "out"
            refusal_stdout, refusal_stderr = StringIO(), StringIO()
            with redirect_stdout(refusal_stdout), redirect_stderr(refusal_stderr):
                refusal_rc = main()
            if (
                refusal_rc != 2
                or (RUN / "human-projection.tsv").exists()
                or refusal_stdout.getvalue()
                or "ARTIFACT-TUPLE" not in refusal_stderr.getvalue()
            ):
                raise RuntimeError("REDPROOF corrupt GAP tuple did not refuse cleanly")
    finally:
        REPO, RUN = original_repo, original_run
    print("RED human corrupt GAP tuple: named stderr refusal, no output")

    case_dir, consumer = "arms/redproof", "human"
    matching_bytes = b"static retained residual\n"
    base = project_baseline(matching_bytes)
    matching = project_successor(matching_bytes)
    absent = project_successor(b"")
    if compare(base, matching, case_dir, consumer)[0] != "exact":
        raise RuntimeError("REDPROOF matching residual was not exact")
    absent_verdict, absent_detail = compare(base, absent, case_dir, consumer)
    if absent_verdict != "residual-fail" or "residual" not in absent_detail:
        raise RuntimeError("REDPROOF absent residual was not a divergence")
    temporal_base = project_baseline(
        b"tg1  stopped\n"
        b"  goal (24m ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 24m ago\n"
    )
    temporal_match = project_successor(
        b"tg1\tstopped\n"
        b"  goal (24m ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 24m ago\n"
    )
    temporal_facts = lambda _case, _session: EpochFacts(1_000, 1_000)
    stable_bracket = ClockBracket(2_440, 2_460)
    if compare(temporal_base, temporal_match, case_dir, consumer, stable_bracket, temporal_facts)[0] != "layout-open":
        raise RuntimeError("REDPROOF valid shared temporal witness was not accepted")
    if compare(temporal_base, temporal_match, case_dir, consumer)[0] != "fixture-abort":
        raise RuntimeError("REDPROOF unbracketed temporal invoke was not refused")
    clock_changed = project_successor(
        b"tg1\tstopped\n"
        b"  goal (25m ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 25m ago\n"
    )
    if compare(temporal_base, clock_changed, case_dir, consumer, stable_bracket, temporal_facts)[0] != "semantic-fail":
        raise RuntimeError("REDPROOF goal/active clock drift was not a temporal divergence")

    wrong_epoch = lambda _case, _session: EpochFacts(1_061, 1_061)
    if compare(temporal_base, temporal_match, case_dir, consumer, stable_bracket, wrong_epoch)[0] != "semantic-fail":
        raise RuntimeError("REDPROOF wrong temporal epoch was accepted")
    wrong_unit = project_successor(
        b"tg1\tstopped\n"
        b"  goal (1440s ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 1440s ago\n"
    )
    if compare(temporal_base, wrong_unit, case_dir, consumer, stable_bracket, temporal_facts)[0] != "semantic-fail":
        raise RuntimeError("REDPROOF wrong temporal unit/rounding was accepted")

    crossing_base = project_baseline(
        b"tg1  stopped\n"
        b"  goal (59s ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 59s ago\n"
    )
    crossing_facts = lambda _case, _session: EpochFacts(1_000, 1_000)
    crossing_bracket = ClockBracket(1_059, 1_060)
    crossing_59 = project_successor(
        b"tg1\tstopped\n"
        b"  goal (59s ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 59s ago\n"
    )
    crossing_1m = project_successor(
        b"tg1\tstopped\n"
        b"  goal (1m ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 1m ago\n"
    )
    for name, candidate in (("threshold-59s", crossing_59), ("threshold-1m", crossing_1m)):
        if compare(crossing_base, candidate, case_dir, consumer, crossing_bracket, crossing_facts)[0] != "layout-open":
            raise RuntimeError(f"REDPROOF {name} was not admitted by its bracket witness")
    crossing_58 = project_successor(
        b"tg1\tstopped\n"
        b"  goal (58s ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 58s ago\n"
    )
    if compare(crossing_base, crossing_58, case_dir, consumer, crossing_bracket, crossing_facts)[0] != "semantic-fail":
        raise RuntimeError("REDPROOF threshold crossing accepted a spelling outside its finite set")
    split_witness = project_successor(
        b"tg1\tstopped\n"
        b"  goal (59s ago): retained goal \xc2\xb7 git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active 1m ago\n"
    )
    split_verdict, split_detail = compare(
        crossing_base, split_witness, case_dir, consumer, crossing_bracket, crossing_facts
    )
    if split_verdict != "semantic-fail" or "no common UTC witness" not in split_detail:
        raise RuntimeError("REDPROOF independently-valid temporal spans were accepted without one witness")

    # Parse the runner-side record as a binding, not as an advisory timestamp.
    # This exercises absent/wrong-binding/clock-step failures independently of
    # temporal spelling acceptance.
    clock_row = {
        "case": "arms/redproof/case",
        "consumer": "list-all",
        "surface": "ae list",
        "normalised_argv": "ae list --all",
    }

    def clock_text(index: int = 7, after: int = 1_021, after_mono: int = 21_000_000_000) -> str:
        return (
            "schema\t1\n"
            f"index\t{index}\n"
            "case\tarms/redproof/case\n"
            "consumer\tlist-all\n"
            "surface\tae list\n"
            "normalised_argv\tae list --all\n"
            "successor_argv\tlist --all\n"
            "before_epoch\t1000\n"
            f"after_epoch\t{after}\n"
            "before_monotonic_ns\t0\n"
            f"after_monotonic_ns\t{after_mono}\n"
            f"monotonic_elapsed_ns\t{after_mono}\n"
            f"width_seconds\t{after - 1000}\n"
            f"max_width_seconds\t{HUMAN_CLOCK_BRACKET_MAX_SECS}\n"
        )

    with tempfile.TemporaryDirectory(prefix="ae-human-clock-redproof-") as raw:
        capture = Path(raw)
        (capture / "human-clock-bracket.tsv").write_text(clock_text(), encoding="utf-8")
        if load_clock_bracket(capture, 7, clock_row) != ClockBracket(1_000, 1_021):
            raise RuntimeError("REDPROOF valid human clock binding did not round-trip")
        for name, contents, expected in (
            ("wrong-binding", clock_text(index=8), "BINDING index"),
            ("clock-step", clock_text(after=1_010, after_mono=100_000_000), "DISCONTINUITY"),
        ):
            (capture / "human-clock-bracket.tsv").write_text(contents, encoding="utf-8")
            try:
                load_clock_bracket(capture, 7, clock_row)
            except TemporalFixtureError as error:
                if expected not in str(error):
                    raise
            else:
                raise RuntimeError(f"REDPROOF {name} bracket was accepted")
        (capture / "human-clock-bracket.tsv").unlink()
        try:
            load_clock_bracket(capture, 7, clock_row)
        except TemporalFixtureError as error:
            if "MISSING" not in str(error):
                raise
        else:
            raise RuntimeError("REDPROOF absent clock bracket was accepted")

    # Header recovery/calibration is a parser concern. Feed compare two otherwise
    # identical, total role partitions to prove that a header-only difference stays
    # inside OC-P3-HUMAN-LAYOUT rather than becoming a residual divergence.
    row = [SessionSem("tg1", "stopped", None, [])]
    def fixture_projection(
        sessions: list[SessionSem], chunks: list[tuple[str, bytes]]
    ) -> Projection:
        raw = b"".join(chunk for _role, chunk in chunks)
        projection = Projection(sessions=sessions, raw=raw)
        roles = RoleRecorder(raw)
        for role, chunk in chunks:
            roles.put(role, chunk)
        roles.apply(projection)
        if not projection.parse_ok:
            raise RuntimeError(f"REDPROOF fixture role tile invalid: {projection.note}")
        return projection

    base_header = fixture_projection(
        row,
        [
            ("header", b"SESSION STATUS\n"),
            ("semantic", b"tg1"),
            ("layout", b"\t"),
            ("semantic", b"stopped"),
            ("layout", b"\n"),
        ],
    )
    successor_header = fixture_projection(
        row,
        [
            ("header", b"NAME STATE\n"),
            ("semantic", b"tg1"),
            ("layout", b"\t"),
            ("semantic", b"stopped"),
            ("layout", b"\n"),
        ],
    )
    if not _roles_ok(base_header) or not _roles_ok(successor_header):
        raise RuntimeError("REDPROOF header fixtures are not total role partitions")
    if compare(base_header, successor_header, case_dir, consumer)[0] != "layout-open":
        raise RuntimeError("REDPROOF header-only difference was not layout-open")

    def direct(tag: bytes, sessions: list[SessionSem]) -> Projection:
        # A compact, total role partition is enough here: compare() receives
        # fully parsed rows and must reject these named semantic mutations before
        # byte-role handling can classify them as layout-open.
        return fixture_projection(sessions, [("semantic", tag)])

    # Frozen bash grammar is not the successor tab grammar. A running agent row
    # is REF / short-session-id / declared-state / optional health marker; a
    # stopped row is REF / short-session-id only. The temporal git/active
    # subline also begins with a token matching AGENT_REF, so it must be
    # classified before an adjacent agent row is considered. The fixed-source
    # audit consequently corrects the old mixed-grain 294-file/1,104-row
    # reading to 286 outputs/816 actual agent rows: the 288 difference is
    # metadata sublines, not agent evidence. This proof enforces grammar and
    # total byte-role tiling, never those contingent aggregate counts.
    frozen_agent_grammar = (
        b"SESSION                   STATUS    MODE        ORIGIN\n"
        b"trun                      running   local       /tmp/frozen/run\n"
        b"  git:master \xc2\xb7 ae 0.2.1 \xc2\xb7 active >7d\n"
        b"  fake:lead               -         working       \n"
        b"  fake:bang               -         blocked       !\n"
        b"tstop                     stopped   local       /tmp/frozen/stop\n"
        b"  fake:worker             11111111\n"
    )
    frozen_agent_projection = project_baseline(frozen_agent_grammar)
    if not frozen_agent_projection.parse_ok or not _roles_ok(frozen_agent_projection):
        raise RuntimeError(
            f"REDPROOF frozen baseline agent grammar lost exact role tiling: {frozen_agent_projection.note}"
        )
    expected_agent_rows = [
        (
            "trun",
            [
                ("fake:lead", "-", "", "alive", "working"),
                ("fake:bang", "-", "!", "dead", "blocked"),
            ],
        ),
        ("tstop", [("fake:worker", "11111111", None, None, None)]),
    ]
    observed_agent_rows = [
        (
            session.name,
            [
                (
                    agent.ref,
                    getattr(agent, "session_id", None),
                    agent.health_token,
                    agent.health_sem,
                    agent.state,
                )
                for agent in session.agents
            ],
        )
        for session in frozen_agent_projection.sessions
    ]
    if observed_agent_rows != expected_agent_rows:
        raise RuntimeError(
            "REDPROOF frozen baseline agent grammar misparsed: "
            f"expected={expected_agent_rows!r} got={observed_agent_rows!r}"
        )
    if b"!" in frozen_agent_projection.layout or b"!" not in frozen_agent_projection.semantic:
        raise RuntimeError("REDPROOF frozen bang marker was not retained as a semantic field")
    stopped_trailing_cell = project_baseline(
        b"SESSION  STATUS  MODE  ORIGIN\n"
        b"tstop  stopped  local  /tmp/frozen/stop\n"
        b"  fake:worker  11111111  \n"
    )
    if stopped_trailing_cell.parse_ok or "malformed frozen stopped agent row" not in stopped_trailing_cell.note:
        raise RuntimeError("REDPROOF stopped-space discriminator accepted a trailing running cell")
    running_two_fields = project_baseline(
        b"SESSION  STATUS  MODE  ORIGIN\n"
        b"trun  running  local  /tmp/frozen/run\n"
        b"  fake:lead  -\n"
    )
    if running_two_fields.parse_ok or "malformed frozen running agent row" not in running_two_fields.note:
        raise RuntimeError("REDPROOF running grammar was inferred from stopped column arity")
    missing_source_id = project_baseline(
        b"SESSION                   STATUS    MODE        ORIGIN\n"
        b"tbad                      stopped   local       /tmp/frozen/bad\n"
        b"  fake:missing\n"
    )
    if missing_source_id.parse_ok or "lacks short session-id" not in missing_source_id.note:
        raise RuntimeError("REDPROOF baseline agent without short session-id was accepted")

    successor_agent_grammar = project_successor(
        b"tsuccess\trunning\n"
        b"  fake:lead\t-\talive\tworking\n"
        b"  fake:worker\t11111111\tdead\tdone\n"
    )
    successor_rows = [
        (agent.ref, agent.session_id, agent.health_token, agent.health_sem, agent.state)
        for agent in successor_agent_grammar.sessions[0].agents
    ]
    if not successor_agent_grammar.parse_ok or successor_rows != [
        ("fake:lead", "-", "alive", "alive", "working"),
        ("fake:worker", "11111111", "dead", "dead", "done"),
    ]:
        raise RuntimeError(f"REDPROOF successor agent identity grammar misparsed: {successor_rows!r}")
    missing_successor_id = project_successor(
        b"tsuccess\trunning\n  fake:lead\talive\tworking\n"
    )
    if missing_successor_id.parse_ok or "malformed successor agent row" not in missing_successor_id.note:
        raise RuntimeError("REDPROOF successor agent without short session-id was accepted")

    source_shaped_ids: dict[str, tuple[AgentSem, AgentSem]] = {}
    # Five Greek alpha Unicode code points occupy ten UTF-8 bytes. With no
    # combining characters or normalization ambiguity, this proves character
    # count rather than byte count and must reach the exact h/r owner relation.
    for rendered_id in ("abc", "éø", "ααααα"):
        baseline = project_baseline(
            (
                "SESSION  STATUS  MODE  ORIGIN\n"
                "tshort   stopped  local  /tmp/frozen/short\n"
                f"  fake:lead  {rendered_id}\n"
            ).encode("utf-8")
        )
        successor = project_successor(
            f"tshort\tstopped\n  fake:lead\t{rendered_id}\tdead\tworking\n".encode("utf-8")
        )
        if (
            not baseline.parse_ok
            or not successor.parse_ok
            or baseline.sessions[0].agents[0].session_id != rendered_id
            or successor.sessions[0].agents[0].session_id != rendered_id
        ):
            raise RuntimeError(f"REDPROOF short rendered session-id grammar lost {rendered_id!r}")
        source_shaped_ids[rendered_id] = (
            baseline.sessions[0].agents[0],
            successor.sessions[0].agents[0],
        )
    # ``pending`` is producer-normalized-away: it is seven characters but the
    # producer must render dash, so accepting it would launder producer failure.
    for malformed_id in ("", "pending", "abcdefghi"):
        malformed = project_successor(
            f"tshort\tstopped\n  fake:lead\t{malformed_id}\tdead\tworking\n".encode("utf-8")
        )
        if malformed.parse_ok:
            raise RuntimeError(f"REDPROOF invalid rendered session-id was accepted: {malformed_id!r}")

    # Offset spans, not aggregate role lengths, are the projection invariant.
    # A double-assigned byte plus a skipped one must reach parse-fail rather
    # than silently deleting residual bytes under the layout open choice.
    malformed_roles = direct(b"role-tile", [SessionSem("tg1", "running", None, [])])
    malformed_roles.spans = [
        RoleSpan(0, 5, "semantic", b"role-"),
        RoleSpan(4, 9, "layout", b"-tile"),
    ]
    if compare(malformed_roles, direct(b"role-tile", [SessionSem("tg1", "running", None, [])]), case_dir, consumer)[0] != "parse-fail":
        raise RuntimeError("REDPROOF overlapping role spans were accepted")

    def agent(
        health: str | None, state: str | None, session_id: str | None = "deadbeef"
    ) -> AgentSem:
        return AgentSem(
            "fixture:agent",
            session_id,
            health,
            HEALTH_MAP.get(health) if health is not None else None,
            state,
        )

    if agent(None, None).health_sem is not None:
        raise RuntimeError("REDPROOF helper manufactured health from an absent stopped locus")

    frozen_agents = {
        agent.ref: agent
        for session in frozen_agent_projection.sessions
        for agent in session.agents
    }
    blank_agent = frozen_agents["fake:lead"]
    bang_agent = frozen_agents["fake:bang"]
    stopped_agent = frozen_agents["fake:worker"]

    def agent_compare_projection(tag: bytes, session: str, status: str, row: AgentSem) -> Projection:
        return direct(tag, [SessionSem(session, status, None, [row])])

    def successor_agent(
        ref: str, health: str, state: str | None, session_id: str | None = "deadbeef"
    ) -> AgentSem:
        return AgentSem(ref, session_id, health, HEALTH_MAP[health], state)

    # Short IDs are source-shaped parser outputs, then exact h/r owner keys.
    # This covers both a shorter ASCII id and a shorter multibyte id without
    # substituting byte length for the frozen shell's character truncation.
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        for rendered_id, (before_agent, after_agent) in source_shaped_ids.items():
            OBS_SHAPES[(case_dir, consumer)] = [
                {
                    "obligation_id": "SC-017h",
                    "locus": f"agents[tshort:fake:lead:{rendered_id}].state",
                    "from": "ABSENT",
                    "to": "working",
                },
                {
                    "obligation_id": "SC-017r",
                    "locus": f"agents[tshort:fake:lead:{rendered_id}].health",
                    "from": "ABSENT",
                    "to": "dead",
                },
            ]
            before = agent_compare_projection(b"short-id-owner", "tshort", "stopped", before_agent)
            after = agent_compare_projection(b"short-id-owner", "tshort", "stopped", after_agent)
            if compare(before, after, case_dir, consumer)[0] != "exact":
                raise RuntimeError(f"REDPROOF short rendered id owner did not hold: {rendered_id!r}")
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    def expect_session_id_fail(name: str, before: Projection, after: Projection) -> None:
        verdict, detail = compare(before, after, case_dir, consumer)
        if verdict != "semantic-fail" or not (
            "short session-id" in detail
            or detail.startswith("agent identities")
            or detail.startswith("agent identity order")
        ):
            raise RuntimeError(f"REDPROOF {name} session-id divergence survived: {verdict} {detail!r}")

    # These are comparator proofs, not merely parser-field checks. The frozen
    # carrier distinguishes a blank running cell from an absent stopped locus:
    # known source health holds directly, while SC-017r may relax only its exact
    # blank or ABSENT transition. The source formatter's short session-id is a
    # separate required semantic field, so successor omission/change fails first.
    expect_session_id_fail(
        "running session-id omission",
        agent_compare_projection(b"run-sid-omit", "trun", "running", blank_agent),
        agent_compare_projection(
            b"run-sid-omit", "trun", "running", successor_agent("fake:lead", "alive", "working", None)
        ),
    )
    expect_session_id_fail(
        "running session-id change",
        agent_compare_projection(b"run-sid-change", "trun", "running", blank_agent),
        agent_compare_projection(
            b"run-sid-change", "trun", "running", successor_agent("fake:lead", "alive", "working", "other")
        ),
    )
    expect_session_id_fail(
        "stopped session-id omission",
        agent_compare_projection(b"stop-sid-omit", "tstop", "stopped", stopped_agent),
        agent_compare_projection(
            b"stop-sid-omit", "tstop", "stopped", successor_agent("fake:worker", "unknown", None, None)
        ),
    )
    expect_session_id_fail(
        "stopped session-id change",
        agent_compare_projection(b"stop-sid-change", "tstop", "stopped", stopped_agent),
        agent_compare_projection(
            b"stop-sid-change", "tstop", "stopped", successor_agent("fake:worker", "unknown", None, "other")
        ),
    )
    spacing_a = project_baseline(
        b"SESSION                   STATUS    MODE        ORIGIN\n"
        b"tspace                    running   local       /tmp/frozen/space\n"
        b"  fake:lead               -         working       \n"
    )
    spacing_b = project_baseline(
        b"SESSION                   STATUS    MODE        ORIGIN\n"
        b"tspace                    running   local       /tmp/frozen/space\n"
        b"  fake:lead  -  working  \n"
    )
    # The second fixture differs only in frozen layout bytes; give its parsed
    # row the successor's literal health carrier so this is not accidentally a
    # blank-to-alive SC-017r transition test.
    spacing_b.sessions[0].agents[0].health_token = "alive"
    spacing_b.sessions[0].agents[0].health_sem = "alive"
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tspace:fake:lead:-].health",
                "from": "blank",
                "to": "alive",
            }
        ]
        spacing_verdict, spacing_detail = compare(spacing_a, spacing_b, case_dir, consumer)
        if spacing_verdict != "layout-open":
            raise RuntimeError(
                f"REDPROOF agent-cell padding stopped being layout-only: {spacing_verdict} {spacing_detail!r}"
            )
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes
    blank_to_alive_before = agent_compare_projection(b"blank-alive", "trun", "running", blank_agent)
    blank_to_alive_after = agent_compare_projection(
        b"blank-alive", "trun", "running", successor_agent(
            "fake:lead", "alive", "working", blank_agent.session_id
        )
    )
    if compare(blank_to_alive_before, blank_to_alive_after, case_dir, consumer)[0] != "semantic-fail":
        raise RuntimeError("REDPROOF blank source carrier silently accepted literal alive")
    bang_to_dead_before = agent_compare_projection(b"bang-dead", "trun", "running", bang_agent)
    bang_to_dead_after = agent_compare_projection(
        b"bang-dead", "trun", "running", successor_agent(
            "fake:bang", "dead", "blocked", bang_agent.session_id
        )
    )
    if compare(
        bang_to_dead_before,
        bang_to_dead_after,
        case_dir,
        consumer,
    )[0] != "semantic-fail":
        raise RuntimeError("REDPROOF literal bang silently accepted semantic dead without an owner")
    if Counter(_source_health_carrier(agent) for agent in bang_to_dead_before.sessions[0].agents) != Counter({"!": 1}):
        raise RuntimeError("REDPROOF frozen bang did not retain literal table-facing source Counter")

    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[trun:fake:lead:-].health",
                "from": "blank",
                "to": "alive",
            }
        ]
        if compare(blank_to_alive_before, blank_to_alive_after, case_dir, consumer)[0] != "exact":
            raise RuntimeError("REDPROOF exact blank-to-alive SC-017r owner did not hold")
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[trun:fake:bang:-].health",
                "from": "!",
                "to": "dead",
            }
        ]
        if compare(bang_to_dead_before, bang_to_dead_after, case_dir, consumer)[0] != "exact":
            raise RuntimeError("REDPROOF exact bang-to-dead SC-017r owner did not hold")
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[trun:fake:lead:-].health",
                "from": "blank",
                "to": "dead",
            }
        ]
        blank_to_dead = compare(
            agent_compare_projection(b"blank-dead", "trun", "running", blank_agent),
            agent_compare_projection(
                b"blank-dead", "trun", "running", successor_agent(
                    "fake:lead", "dead", "working", blank_agent.session_id
                )
            ),
            case_dir,
            consumer,
        )
        if blank_to_dead[0] != "exact":
            raise RuntimeError(f"REDPROOF exact blank-to-dead owner did not hold: {blank_to_dead!r}")
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[trun:fake:lead:-].health",
                "from": "blank",
                "to": "unambiguous unknown",
            }
        ]
        blank_to_unknown = compare(
            agent_compare_projection(b"blank-unknown", "trun", "running", blank_agent),
            agent_compare_projection(
                b"blank-unknown", "trun", "running", successor_agent(
                    "fake:lead", "unknown", "working", blank_agent.session_id
                )
            ),
            case_dir,
            consumer,
        )
        if blank_to_unknown[0] != "exact":
            raise RuntimeError(f"REDPROOF blank SC-017r owner did not hold: {blank_to_unknown!r}")
        expect_bang_unknown = compare(
            agent_compare_projection(b"bang-unknown", "trun", "running", bang_agent),
            agent_compare_projection(
                b"bang-unknown", "trun", "running", successor_agent(
                    "fake:bang", "unknown", "blocked", bang_agent.session_id
                )
            ),
            case_dir,
            consumer,
        )
        if expect_bang_unknown[0] != "semantic-fail":
            raise RuntimeError(f"REDPROOF blank owner accepted frozen bang: {expect_bang_unknown!r}")
        expect_stopped_unknown = compare(
            agent_compare_projection(b"stopped-unknown", "tstop", "stopped", stopped_agent),
            agent_compare_projection(
                b"stopped-unknown", "tstop", "stopped", successor_agent(
                    "fake:worker", "unknown", None, stopped_agent.session_id
                )
            ),
            case_dir,
            consumer,
        )
        if expect_stopped_unknown[0] != "semantic-fail":
            raise RuntimeError(f"REDPROOF blank owner accepted stopped absence: {expect_stopped_unknown!r}")
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tstop:fake:worker:11111111].health",
                "from": "ABSENT",
                "to": "unambiguous unknown",
            }
        ]
        absent_to_unknown = compare(
            agent_compare_projection(b"absent-unknown", "tstop", "stopped", stopped_agent),
            agent_compare_projection(
                b"absent-unknown", "tstop", "stopped", successor_agent(
                    "fake:worker", "unknown", None, stopped_agent.session_id
                )
            ),
            case_dir,
            consumer,
        )
        if absent_to_unknown[0] != "exact":
            raise RuntimeError(f"REDPROOF ABSENT SC-017r owner did not hold: {absent_to_unknown!r}")
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    def expect_semantic_fail(
        name: str, before: Projection, after: Projection, detail_fragment: str
    ) -> None:
        verdict, detail = compare(before, after, case_dir, consumer)
        if verdict != "semantic-fail" or detail_fragment not in detail:
            raise RuntimeError(
                f"REDPROOF {name} survived: {verdict} {detail!r}; expected {detail_fragment!r}"
            )

    def projected_agents(tag: bytes, session: str, agents: list[AgentSem]) -> Projection:
        return direct(tag, [SessionSem(session, "running", None, agents)])

    # G10 is deliberately not a collision: two lead entries share a full key,
    # while worker has a distinct key between them. Group/set comparison loses
    # this retained semantic order; an exchange inside one full-key collision
    # remains covered by the collision proof below.
    g10_order_before = projected_agents(
        b"g10-order-before",
        "tg-g10",
        [
            AgentSem("g10:lead", "-", "!", "dead", "working"),
            AgentSem("g10:worker", "-", "!", "dead", "working"),
            AgentSem("g10:lead", "-", "!", "dead", "working"),
        ],
    )
    g10_order_after = projected_agents(
        b"g10-order-after",
        "tg-g10",
        [
            AgentSem("g10:lead", "-", "dead", "dead", "working"),
            AgentSem("g10:lead", "-", "dead", "dead", "working"),
            AgentSem("g10:worker", "-", "dead", "dead", "working"),
        ],
    )
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tg-g10:g10:lead:-](class).health",
                "from": "! x2",
                "to": "dead x2",
            },
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tg-g10:g10:worker:-].health",
                "from": "!",
                "to": "dead",
            },
        ]
        expect_semantic_fail(
            "G10 distinct agent identity permutation",
            g10_order_before,
            g10_order_after,
            "agent identity order",
        )
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    # Exact h/r admission precedes value comparison. A row for one short SID
    # cannot authorize its same-ref sibling, and an unchanged h cell remains
    # covered by default parity even when no h row exists for that invocation.
    same_ref_base = projected_agents(
        b"same-ref-sid",
        "tg-sid",
        [
            AgentSem("same:agent", "11111111", "!", "dead", None),
            AgentSem("same:agent", "22222222", "!", "dead", None),
        ],
    )
    same_ref_after = projected_agents(
        b"same-ref-sid",
        "tg-sid",
        [
            AgentSem("same:agent", "11111111", "dead", "dead", "working"),
            AgentSem("same:agent", "22222222", "dead", "dead", "working"),
        ],
    )
    matching_h_before = projected_agents(
        b"matching-h",
        "tg-match",
        [AgentSem("same:agent", "33333333", "!", "dead", "working")],
    )
    matching_h_after = projected_agents(
        b"matching-h",
        "tg-match",
        [AgentSem("same:agent", "33333333", "dead", "dead", "working")],
    )
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017h",
                "locus": "agents[tg-sid:same:agent:22222222].state",
                "from": "ABSENT",
                "to": "working",
            },
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tg-sid:same:agent:11111111].health",
                "from": "!",
                "to": "dead",
            },
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tg-sid:same:agent:22222222].health",
                "from": "!",
                "to": "dead",
            },
        ]
        expect_semantic_fail(
            "same-ref distinct-SID owner isolation",
            same_ref_base,
            same_ref_after,
            "state ('tg-sid', 'same:agent', '11111111')",
        )
        OBS_SHAPES[(case_dir, consumer)] = [
            {
                "obligation_id": "SC-017r",
                "locus": "agents[tg-match:same:agent:33333333].health",
                "from": "!",
                "to": "dead",
            }
        ]
        if compare(matching_h_before, matching_h_after, case_dir, consumer)[0] != "exact":
            raise RuntimeError("REDPROOF matching SC-017h cell lost default parity coverage")
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    # Same session/ref/SID is the one deliberately order-free collision class.
    # Owners aggregate exact source and target Counters, so a swapped output is
    # neutral while a dropped member, wrong target multiplicity, or one-row
    # multiplication fails.
    collision_before = projected_agents(
        b"collision",
        "tg-collision",
        [
            AgentSem("same:agent", "-", "!", "dead", None),
            AgentSem("same:agent", "-", "!", "dead", None),
        ],
    )
    collision_after_swapped = projected_agents(
        b"collision",
        "tg-collision",
        [
            AgentSem("same:agent", "-", "dead", "dead", "done"),
            AgentSem("same:agent", "-", "dead", "dead", "working"),
        ],
    )
    collision_after_wrong_values = projected_agents(
        b"collision-wrong-values",
        "tg-collision",
        [
            AgentSem("same:agent", "-", "dead", "dead", "working"),
            AgentSem("same:agent", "-", "dead", "dead", "working"),
        ],
    )
    collision_after_dropped = projected_agents(
        b"collision-dropped",
        "tg-collision",
        [AgentSem("same:agent", "-", "dead", "dead", "working")],
    )
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        collision_locus = "agents[tg-collision:same:agent:-](class).state"
        OBS_SHAPES[(case_dir, consumer)] = [
            {"obligation_id": "SC-017h", "locus": collision_locus, "from": "ABSENT", "to": "working"},
            {"obligation_id": "SC-017h", "locus": collision_locus, "from": "ABSENT", "to": "done"},
            {"obligation_id": "SC-017r", "locus": "agents[tg-collision:same:agent:-](class).health", "from": "! x2", "to": "dead x2"},
        ]
        if compare(collision_before, collision_after_swapped, case_dir, consumer)[0] != "exact":
            raise RuntimeError("REDPROOF collision order swap was not neutral")
        expect_semantic_fail(
            "collision heterogeneous target counter",
            collision_before,
            collision_after_wrong_values,
            "owner_target",
        )
        expect_semantic_fail(
            "collision dropped member",
            collision_before,
            collision_after_dropped,
            "agent identity order",
        )
        OBS_SHAPES[(case_dir, consumer)] = [
            {"obligation_id": "SC-017h", "locus": collision_locus, "from": "ABSENT", "to": "working"},
            {"obligation_id": "SC-017r", "locus": "agents[tg-collision:same:agent:-](class).health", "from": "! x2", "to": "dead x2"},
        ]
        expect_semantic_fail(
            "collision owner does not multiply",
            collision_before,
            collision_after_wrong_values,
            "owner_source",
        )
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    # Four independent false-pass channels measured in compare() before this
    # repair.  Each seed changes a real recovered fact, not merely a verdict
    # label or byte prefix.
    expect_semantic_fail(
        "health alive-to-dead",
        direct(b"health-before", [SessionSem("tg1", "running", None, [agent("alive", "working")])]),
        direct(b"health-after", [SessionSem("tg1", "running", None, [agent("dead", "working")])]),
        "health ('tg1', 'fixture:agent', 'deadbeef')",
    )
    expect_semantic_fail(
        "delete-only-running",
        direct(b"running-before", [SessionSem("tg1", "running", None, [])]),
        direct(b"running-after", []),
        "missing retained session ['tg1']",
    )
    expect_semantic_fail(
        "delete-stopped-plus-shared-attn",
        direct(
            b"shared-before",
            [SessionSem("shared", "running", "blocked", []), SessionSem("gone", "stopped", None, [])],
        ),
        direct(b"shared-after", [SessionSem("shared", "running", None, [])]),
        "attn shared 'blocked'->None",
    )
    # SC-017l is an in-place status relation, never a deletion allowance.
    # The same named tuple that permits stopped -> unknown must still reject
    # an absent stopped row.
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {"obligation_id": "SC-017l", "locus": "status cell", "from": "stopped", "to": "unknown"},
        ]
        expect_semantic_fail(
            "observed-status-does-not-authorize-deletion",
            direct(b"stopped-before", [SessionSem("tg1", "stopped", None, [])]),
            direct(b"stopped-after", []),
            "missing retained session ['tg1']",
        )
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes
    # The legacy SC-017m generic row-set tuple is superseded by the pending
    # exact per-view candidate/status-set grammar.  It must not remain an
    # acceptance backdoor while the final owner is unpinned.
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {"obligation_id": "SC-017m", "locus": "(row set)", "from": "empty", "to": "unknown rows present"},
        ]
        expect_semantic_fail(
            "legacy-row-set-owner-is-rejected",
            direct(b"extra-before", [SessionSem("tg1", "running", None, [])]),
            direct(
                b"extra-after",
                [SessionSem("tg1", "running", None, []), SessionSem("tg2", "unknown", None, [])],
            ),
            "SC-017m final view-set owner not pinned",
        )
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes
    expect_semantic_fail(
        "state-dash-to-working",
        direct(b"state-before", [SessionSem("tg1", "running", None, [agent("alive", "-")])]),
        direct(b"state-after", [SessionSem("tg1", "running", None, [agent("alive", "working")])]),
        "state ('tg1', 'fixture:agent', 'deadbeef')",
    )

    # A real frozen tabular row proves MODE and ORIGIN are residual rather than
    # layout, then the successor's absent fields must be a divergence.
    mode_origin_baseline = project_baseline(
        b"SESSION                   STATUS    MODE        ORIGIN\n"
        b"tg1                       running   local       /tmp/frozen-origin\n"
    )
    mode_origin_successor = project_successor(b"tg1\trunning\n")
    if b"local" in mode_origin_baseline.layout or b"/tmp/frozen-origin" in mode_origin_baseline.layout:
        raise RuntimeError("REDPROOF mode/origin leaked into layout")
    if b"local" not in mode_origin_baseline.residual or b"/tmp/frozen-origin" not in mode_origin_baseline.residual:
        raise RuntimeError("REDPROOF mode/origin were not residual")
    mode_verdict, mode_detail = compare(
        mode_origin_baseline, mode_origin_successor, case_dir, consumer
    )
    if mode_verdict != "residual-fail" or not mode_detail.startswith("DIVERGENCE residual"):
        raise RuntimeError(f"REDPROOF mode/origin omission survived: {mode_verdict} {mode_detail!r}")

    # Positive controls prove that implemented relaxations are not merely
    # deleted. Each source-valid carrier isolates stopped status, running declared
    # state, running blank health, and stopped absent
    # health. SC-017m stays deliberately fail-closed until its final view-set
    # grammar is pinned.
    saved_shapes = OBS_SHAPES.get((case_dir, consumer))
    try:
        OBS_SHAPES[(case_dir, consumer)] = [
            {"obligation_id": "SC-017l", "locus": "status cell", "from": "stopped", "to": "unknown"},
            {"obligation_id": "SC-017h", "locus": "agents[tg-state:fixture:agent:deadbeef].state", "from": "-", "to": "unknown"},
            {"obligation_id": "SC-017r", "locus": "agents[tg-state:fixture:agent:deadbeef].health", "from": "!", "to": "dead"},
            {"obligation_id": "SC-017r", "locus": "agents[tg-blank:fixture:agent:deadbeef].health", "from": "blank", "to": "unambiguous unknown"},
            {"obligation_id": "SC-017r", "locus": "agents[tg-absent:fixture:agent:deadbeef].health", "from": "ABSENT", "to": "unambiguous unknown"},
        ]
        positive_controls = (
            (
                "SC-017l stopped status",
                direct(b"status-before", [SessionSem("tg-status", "stopped", None, [])]),
                direct(b"status-after", [SessionSem("tg-status", "unknown", None, [])]),
            ),
            (
                "SC-017h running state",
                direct(
                    b"state-before",
                    [SessionSem("tg-state", "running", None, [AgentSem("fixture:agent", "deadbeef", "!", "dead", "-")])],
                ),
                direct(
                    b"state-after",
                    [SessionSem("tg-state", "running", None, [agent("dead", "unknown")])],
                ),
            ),
            (
                "SC-017r running blank",
                direct(
                    b"blank-before",
                    [SessionSem("tg-blank", "running", None, [AgentSem("fixture:agent", "deadbeef", "", "alive", "working")])],
                ),
                direct(
                    b"blank-after",
                    [SessionSem("tg-blank", "running", None, [agent("unknown", "working")])],
                ),
            ),
            (
                "SC-017r stopped absent",
                direct(
                    b"absent-before",
                    [SessionSem("tg-absent", "stopped", None, [AgentSem("fixture:agent", "deadbeef", None, None, None)])],
                ),
                direct(
                    b"absent-after",
                    [SessionSem("tg-absent", "stopped", None, [agent("unknown", None)])],
                ),
            ),
        )
        for name, before, after in positive_controls:
            verdict, detail = compare(before, after, case_dir, consumer)
            if verdict != "layout-open":
                raise RuntimeError(f"REDPROOF {name} owner did not hold: {verdict} {detail!r}")
    finally:
        if saved_shapes is None:
            OBS_SHAPES.pop((case_dir, consumer), None)
        else:
            OBS_SHAPES[(case_dir, consumer)] = saved_shapes

    print(
        "HUMAN-PROJECTION-REDPROOF PASS: residual/layout roles; matching residual/exact; "
        "absent residual/divergence; header-only/layout-open; six semantic false-pass/ownership seeds; "
        "baseline/successor agent-row grammar; short ASCII/Unicode IDs; exact h/r session-ref-short-id owners; "
        "ordered agent keys; collision Counters/order/multiplicity; role-span tiling; "
        "clock binding/epoch/rounding/crossing/common-witness"
    )


def main() -> int:
    # Load a single immutable tuple before opening output.  `snapshot.obligations`
    # is the exact member whose FRESHNESS hash was validated; parsing a path here
    # would re-open the TOCTOU seam this boundary closes.
    try:
        snapshot, shapes = load_observed_shapes()
    except ArtifactTupleError as error:
        print(error, file=sys.stderr)
        return 2
    global OBS_SHAPES
    OBS_SHAPES = shapes
    _, inv = load_tsv(INV)
    p1 = [r for r in inv if r["phase"] == "P1"]
    epoch_rows, epoch_spans = census_epoch_sources()
    outp = RUN / "human-projection.tsv"
    counts = {}
    n_cover_fail = 0
    with outp.open("w", encoding="utf-8") as fh:
        fh.write(f"# generated_tuple\t{snapshot.identity}\n")
        fh.write(
            "idx\tcase\tconsumer\tverdict\tdetail\t"
            "b_sessions\ts_sessions\tb_cover\ts_cover\n"
        )
        for i, row in enumerate(p1, 1):
            if row["surface"] not in ("ae list", "ae ls"):
                continue
            if "--json" in row["normalised_argv"].split():
                continue
            consumer = row["consumer"]
            case_dir = str(Path(row["case"]).parent)
            basep = CORPUS / Path(row["case"]).parent / "out" / f"{consumer}.stdout"
            capture = CAPS / f"{i:04d}-{consumer}"
            sucp = capture / "stdout"
            if not sucp.exists():
                fh.write(f"{i}\t{row['case']}\t{consumer}\tfixture-abort\tno-capture\t\t\t\t\n")
                counts["fixture-abort"] = counts.get("fixture-abort", 0) + 1
                continue
            if not basep.exists():
                fh.write(f"{i}\t{row['case']}\t{consumer}\tfixture-abort\tno-baseline\t\t\t\t\n")
                counts["fixture-abort"] = counts.get("fixture-abort", 0) + 1
                continue
            try:
                rc_text = (capture / "rc").read_text(encoding="utf-8").strip()
                if not re.fullmatch(r"-?[0-9]+", rc_text):
                    raise TemporalFixtureError(f"malformed successor rc {rc_text!r}")
                rc = int(rc_text)
                if rc != 0:
                    verd, det = "not-evaluated", f"successor human invocation rc={rc}; temporal comparison unavailable"
                    counts[verd] = counts.get(verd, 0) + 1
                    fh.write(f"{i}\t{row['case']}\t{consumer}\t{verd}\t{det}\t\t\t\t\n")
                    continue
                bracket = load_clock_bracket(capture, i, row)
            except (OSError, TemporalFixtureError) as error:
                verd, det = "fixture-abort", str(error)
                counts[verd] = counts.get(verd, 0) + 1
                fh.write(f"{i}\t{row['case']}\t{consumer}\t{verd}\t{det}\t\t\t\t\n")
                continue
            braw, sraw = basep.read_bytes(), sucp.read_bytes()
            bp, sp = project_baseline(braw), project_successor(sraw)
            if not bp.parse_ok:
                n_cover_fail += 1
            if not sp.parse_ok:
                n_cover_fail += 1
            try:
                verd, det = compare(bp, sp, case_dir, consumer, bracket)
            except TemporalFixtureError as error:
                verd, det = "fixture-abort", str(error)
            counts[verd] = counts.get(verd, 0) + 1
            det = det.replace("\t", " ").replace("\n", " ")
            fh.write(
                f"{i}\t{row['case']}\t{consumer}\t{verd}\t{det}\t"
                f"{len(bp.sessions)}\t{len(sp.sessions)}\t"
                f"{int(bp.parse_ok)}\t{int(sp.parse_ok)}\n"
            )
    print(
        "human epoch census",
        {"rows": epoch_rows, "spans": epoch_spans, "missing_or_inconsistent": 0},
    )
    print("human projection", counts, "cover_fail", n_cover_fail)
    print("wrote", outp)
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["redproof"]:
        redproof()
    else:
        raise SystemExit(main())
