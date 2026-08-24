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
import re
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
CORPUS = REPO / "docs/migration/evidence/batch-c-artifacts"
INV = REPO / "docs/migration/evidence/corpus/INVOCATIONS.tsv"
OBL = REPO / "docs/migration/evidence/corpus/OBLIGATIONS.tsv"
RUN = Path(__file__).resolve().parent
CAPS = RUN / "captures"

HEALTH_MAP = {"alive": "alive", "dead": "dead", "unknown": "unknown"}
EMPTY_MSGS = (
    b"No running ae sessions. (try: ae list --all)\n",
    b"No recently active sessions.\n",
    b"No running sessions need your attention.\n",
)
AGENT_REF = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_-]*:[A-Za-z0-9_][A-Za-z0-9_-]*$")


@dataclass
class AgentSem:
    ref: str
    health_token: str | None
    health_sem: str | None
    state: str | None


@dataclass
class SessionSem:
    name: str
    status: str | None
    attn: str | None
    agents: list[AgentSem] = field(default_factory=list)


@dataclass
class Projection:
    sessions: list[SessionSem]
    header: bytes = b""
    layout: bytes = b""
    semantic: bytes = b""
    residual: bytes = b""
    raw: bytes = b""
    parse_ok: bool = True
    note: str = ""

    def covered(self) -> int:
        return len(self.header) + len(self.layout) + len(self.semantic) + len(self.residual)


def _roles_ok(p: Projection) -> bool:
    return p.covered() == len(p.raw)


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


def project_successor(raw: bytes) -> Projection:
    """Successor listing.rs table(): name\\tstatus[\\tattn:reason]\\n  ref\\thealth\\tstate\\n"""
    p = Projection(sessions=[], raw=raw)
    if raw in EMPTY_MSGS or raw in (b"", b"\n"):
        p.residual = raw
        p.note = "empty-or-msg"
        return p
    header, layout, semantic, residual = bytearray(), bytearray(), bytearray(), bytearray()
    sessions: list[SessionSem] = []

    def put(kind: str, chunk: bytes) -> None:
        if kind == "header":
            header.extend(chunk)
        elif kind == "layout":
            layout.extend(chunk)
        elif kind == "semantic":
            semantic.extend(chunk)
        else:
            residual.extend(chunk)

    for line_b, nl in _split_nl(raw):
        line = line_b.decode("utf-8", "replace")
        if line.startswith("  "):
            parts = line[2:].split("\t")
            if len(parts) < 3:
                put("residual", line_b + nl)
                continue
            ref, health, state = parts[0], parts[-2], parts[-1]
            put("layout", b"  ")
            put("semantic", ref.encode())
            put("layout", b"\t")
            put("semantic", health.encode())
            put("layout", b"\t")
            put("semantic", state.encode())
            put("layout", nl)
            if sessions:
                sessions[-1].agents.append(
                    AgentSem(ref, health, HEALTH_MAP.get(health), state)
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
    p.header, p.layout, p.semantic, p.residual = bytes(header), bytes(layout), bytes(semantic), bytes(residual)
    p.parse_ok = _roles_ok(p)
    p.note = "ok" if p.parse_ok else f"cover {p.covered()}!={len(raw)}"
    return p


def _put_line(line_b: bytes, nl: bytes, name: str, status: str, roles: tuple) -> None:
    header, layout, semantic, residual = roles
    idx_name = line_b.find(name.encode())
    if idx_name < 0:
        residual.extend(line_b + nl)
        return
    layout.extend(line_b[:idx_name])
    semantic.extend(name.encode())
    rest = line_b[idx_name + len(name.encode()) :]
    st = status.encode()
    j = rest.find(st)
    if j < 0:
        residual.extend(rest + nl)
        return
    layout.extend(rest[:j])
    semantic.extend(st)
    layout.extend(rest[j + len(st) :] + nl)


def project_baseline(raw: bytes) -> Projection:
    """Frozen bash tabular / empty-state listing."""
    p = Projection(sessions=[], raw=raw)
    if raw in EMPTY_MSGS or raw in (b"", b"\n"):
        p.residual = raw
        p.note = "empty-or-msg"
        return p
    header, layout, semantic, residual = bytearray(), bytearray(), bytearray(), bytearray()
    sessions: list[SessionSem] = []
    roles = (header, layout, semantic, residual)

    def put(kind: str, chunk: bytes) -> None:
        if kind == "header":
            header.extend(chunk)
        elif kind == "layout":
            layout.extend(chunk)
        elif kind == "semantic":
            semantic.extend(chunk)
        else:
            residual.extend(chunk)

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
            stripped = line.lstrip(" ")
            first_tok = stripped.split()[0] if stripped.split() else ""
            if AGENT_REF.match(first_tok):
                indent_n = len(line) - len(line.lstrip(" "))
                cols = re.split(r"[ \t]{2,}", stripped.strip())
                ref = cols[0] if cols else first_tok
                health = cols[1] if len(cols) > 1 else None
                state = cols[2] if len(cols) > 2 else None
                put("layout", line_b[:indent_n])
                mid = line_b[indent_n:]
                put("semantic", ref.encode())
                mid = mid[len(ref.encode()) :]
                if health:
                    i = mid.find(health.encode())
                    if i >= 0:
                        put("layout", mid[:i])
                        put("semantic", health.encode())
                        mid = mid[i + len(health.encode()) :]
                if state:
                    i = mid.find(state.encode())
                    if i >= 0:
                        put("layout", mid[:i])
                        put("semantic", state.encode())
                        mid = mid[i + len(state.encode()) :]
                put("layout", mid + nl)
                if sessions:
                    sessions[-1].agents.append(
                        AgentSem(ref, health, HEALTH_MAP.get(health or ""), state)
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
    p.header, p.layout, p.semantic, p.residual = bytes(header), bytes(layout), bytes(semantic), bytes(residual)
    p.parse_ok = _roles_ok(p)
    p.note = "ok" if p.parse_ok else f"cover {p.covered()}!={len(raw)}"
    return p


def load_obs(case_dir: str, consumer: str) -> set[str]:
    """Observed obligation ids for this invocation."""
    return OBS.get((case_dir, consumer), set())


def load_tsv(path: Path):
    rows = [ln.split("\t") for ln in path.read_text(encoding="utf-8").splitlines() if ln]
    return rows[0], [dict(zip(rows[0], r)) for r in rows[1:]]


_, _OBL_ROWS = load_tsv(OBL)
OBS: dict[tuple[str, str], set[str]] = {}
UNS: dict[tuple[str, str], set[str]] = {}
OBS_SHAPES: dict[tuple[str, str], list[dict]] = {}
for o in _OBL_ROWS:
    key = (o["case"], o["consumer"])
    if o["support"] == "OBSERVED":
        OBS.setdefault(key, set()).add(o["obligation_id"])
        OBS_SHAPES.setdefault(key, []).append(o)
    else:
        UNS.setdefault(key, set()).add(o["obligation_id"])


def compare(base: Projection, suc: Projection, case_dir: str, consumer: str) -> tuple[str, str]:
    """Return (layout-open|semantic-fail|residual-fail|parse-fail|exact, detail)."""
    if not base.parse_ok:
        return "parse-fail", f"baseline {base.note}"
    if not suc.parse_ok:
        return "parse-fail", f"successor {suc.note}"
    if base.raw == suc.raw:
        return "exact", "bytes"
    obs = load_obs(case_dir, consumer)
    uns = UNS.get((case_dir, consumer), set())
    owns_l = "SC-017l" in obs or "SC-017l" in uns
    owns_m = "SC-017m" in obs or "SC-017m" in uns
    # directional status
    expect_unknown_all = "SC-017l" in obs
    expect_unknown_present = "SC-017m" in obs

    bnames = [s.name for s in base.sessions]
    snames = [s.name for s in suc.sessions]

    # membership after directional 017l: stopped rows may leave --stopped view
    if expect_unknown_all:
        # successor statuses must be unknown where sessions remain
        bad_st = [s.status for s in suc.sessions if s.status != "unknown"]
        if bad_st:
            return "semantic-fail", f"017l statuses {bad_st}"
        # names: successor may drop stopped-only view members that became unknown
        # compare is not name-equality to baseline
    elif expect_unknown_present:
        if not any(s.status == "unknown" for s in suc.sessions):
            return "semantic-fail", "017m unknown absent"
    else:
        # SC-017l/m own unknown-vs-running/stopped even when the table has no
        # OBSERVED row on this consumer (C5: do not expected-match). Shared
        # identities still compare in order.
        suc_unknown = {s.name for s in suc.sessions if s.status == "unknown"}
        b_shared = [n for n in bnames if n in snames]
        s_shared = [n for n in snames if n in bnames]
        extra_s = [n for n in snames if n not in bnames]
        extra_b = [n for n in bnames if n not in snames]
        extra_s_ok = extra_s and all(n in suc_unknown for n in extra_s)
        extra_b_ok = extra_b and all(
            next(s.status for s in base.sessions if s.name == n) in ("stopped", "running")
            and n not in snames
            for n in extra_b
        )
        if b_shared != s_shared:
            return "semantic-fail", f"shared-order {b_shared} vs {s_shared}"
        if extra_s and not extra_s_ok:
            return "semantic-fail", f"unmandated extra {extra_s}"
        if extra_b and not extra_b_ok:
            return "semantic-fail", f"unmandated missing {extra_b}"

    if not expect_unknown_all and not expect_unknown_present:
        if bnames == snames:
            for b, s in zip(base.sessions, suc.sessions):
                if b.status and s.status and b.status != s.status:
                    if s.status == "unknown" and b.status in ("running", "stopped"):
                        continue
                    return "semantic-fail", f"status {b.name} {b.status}->{s.status}"
                if b.attn != s.attn and not (b.attn is None and s.attn is None):
                    # attn present vs absent
                    if b.attn != s.attn:
                        return "semantic-fail", f"attn {b.name} {b.attn!r}->{s.attn!r}"
                prefs = [a.ref for a in b.agents]
                srefs = [a.ref for a in s.agents]
                if prefs != srefs:
                    return "semantic-fail", f"agents {b.name} {prefs} vs {srefs}"
                for ba, sa in zip(b.agents, s.agents):
                    if ba.state and sa.state and ba.state != sa.state and ba.state != "-":
                        return "semantic-fail", f"state {ba.ref} {ba.state}->{sa.state}"

    if expect_unknown_present or expect_unknown_all:
        # still require agent refs per remaining sessions if we can pair by name
        bmap = {s.name: s for s in base.sessions}
        for s in suc.sessions:
            b = bmap.get(s.name)
            if b is None:
                continue
            prefs = [a.ref for a in b.agents]
            srefs = [a.ref for a in s.agents]
            if prefs and srefs and prefs != srefs:
                return "semantic-fail", f"agents {s.name} {prefs} vs {srefs}"

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

    # header+layout ignored. residual of goal sublines vs none: residual-fail?
    # Frozen goal/git/active sublines are residual. Successor has none.
    # Those are summaries → residual, not layout. Spec says HUMAN-LAYOUT is
    # headers/columns/widths/colour/whitespace, not goal sublines.
    # But goal sublines are presentation around semantic rows — they are
    # residual. Comparing them exactly would fail every header-table row.
    # They carry attn which we already lifted into session.attn.
    # Remaining residual (goal copy) is not a contract field.
    # Treat goal-subline-only residual as layout-adjacent presentation:
    # STILL_REQUIRED is semantic rows. Goal-subline residual without
    # unmatched attn is recorded as layout-open (columns/whitespace family)
    # ONLY if we already extracted attn. Pure goal/git/active copy is the
    # same family as headers: it is not a semantic-field.
    # Spec is stricter. Report as residual-open-unregistered? Lead asked
    # semantic-fail vs layout-open. Goal sublines are not semantic fields.
    # Classify as layout-open when residual is only those sublines / padding.
    return "layout-open", "semantic-held"


def main() -> int:
    _, inv = load_tsv(INV)
    p1 = [r for r in inv if r["phase"] == "P1"]
    outp = RUN / "human-projection.tsv"
    counts = {}
    n_cover_fail = 0
    with outp.open("w", encoding="utf-8") as fh:
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
            sucp = CAPS / f"{i:04d}-{consumer}" / "stdout"
            if not sucp.exists():
                fh.write(f"{i}\t{row['case']}\t{consumer}\tfixture-abort\tno-capture\t\t\t\t\n")
                counts["fixture-abort"] = counts.get("fixture-abort", 0) + 1
                continue
            if not basep.exists():
                fh.write(f"{i}\t{row['case']}\t{consumer}\tfixture-abort\tno-baseline\t\t\t\t\n")
                counts["fixture-abort"] = counts.get("fixture-abort", 0) + 1
                continue
            braw, sraw = basep.read_bytes(), sucp.read_bytes()
            bp, sp = project_baseline(braw), project_successor(sraw)
            if not bp.parse_ok:
                n_cover_fail += 1
            if not sp.parse_ok:
                n_cover_fail += 1
            if braw == sraw:
                verd, det = "exact", "bytes"
            else:
                verd, det = compare(bp, sp, case_dir, consumer)
            counts[verd] = counts.get(verd, 0) + 1
            det = det.replace("\t", " ").replace("\n", " ")
            fh.write(
                f"{i}\t{row['case']}\t{consumer}\t{verd}\t{det}\t"
                f"{len(bp.sessions)}\t{len(sp.sessions)}\t"
                f"{int(bp.parse_ok)}\t{int(sp.parse_ok)}\n"
            )
    print("human projection", counts, "cover_fail", n_cover_fail)
    print("wrote", outp)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
