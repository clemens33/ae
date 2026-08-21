# P1 build plan — make `ae list` answer

**Status:** drafted 2026-08-21 by fable5:lead. Execution is GATED — see Entry conditions.

The binary currently refuses `list` (`src/lib.rs` `NO_SESSION_SOURCE`) because enumeration and
liveness were unratified surfaces. They are ratified now. This plan turns that refusal into an
answer, in phases that each end green-or-stop.

## What P1 is accountable for

VISION:93 names the read side as `list --json`, requests, events queries. The corpus partition
holds **1065 P1 rows** across `list` (743), `ls` (116), `requests` (168), `events-tail` (38).

`status` (140), `next` (140), `agents` (62) and `doctor` (7) are **P1-adjacent**: captured,
frozen, kept, and *not* parity-gating for P1 entry. A phase whose scope widens by
reasonable-sounding increments never closes. Revisiting is a column re-run, not a rebuild.

## Entry conditions (all must hold before phase 1)

1. `SC-017j`–`SC-017n` written into `semantic-contract.md` by the classifying seat.
2. `SC-509d` written — the schema row that moves the machine surface to **version 2**.
3. The `Status::Unknown` variant merged, constructed nowhere, schema still at 1.

Nothing in phases 1–4 may begin against a summary of a row. Gate against the written text.

## The invariant, stated once

> **A candidate never disappears because a liveness query failed, prefix-matched, or found a
> live session without its marker.**

Discovery and liveness classification are **separate phases**. Every phase below is checked
against this sentence before its diff is read. Collapsing them is what produced
[#105](https://github.com/clemens33/ae/issues/105); the incumbent's two-disjoint-enumerator
shape is the thing being replaced, not ported.

## Phases

**Already landed** (ahead of phase 1, because it was type-level and blocking nothing):
`Status::Unknown` exists with the ratified semantics; `Scope::order()` holds the SC-017m
composition and SC-017n group order at one site; `Status::ALL` plus an exhaustive-match guard
and a coverage test close the array-literal hole. No product code constructs `Unknown` yet and
`SCHEMA_VERSION` is still 1 — both deliberate, and both change in phase 2.


Each ends with `just rust-check` green, or stops. No phase begins while the previous is red.

### Phase 1 — candidate inventory (`SC-017j`)

The union of durable session state under the canonical root plus `SC-400a`'s legacy-readable
worktree layout, **and** positively identified ae-owned live sessions. Archives are inert and
never inventory. Duplicate source paths do not remove a candidate.

`SC-017j` explicitly **does not authorize basename-only deduplication of distinct
identities** — two different paths whose last component matches are two candidates.

*Done when:* a candidate with no readable meta still appears; a live session with no durable
record still appears; the archive contributes nothing; two same-basename identities survive
as two.

### Phase 2 — liveness knowledge, and the schema moves with it (`SC-017k`, `SC-017l`, `SC-509d`)

`RUNNING` requires a successful query of the session's **recorded** server plus exact-name,
ae-owned evidence. `STOPPED` requires that same query proving exact-name absence. Anything
else — unreachable server, failed query, missing ownership evidence — is `UNKNOWN`.

Ambient-server membership, name-prefix success, and renderer-block provenance are **not**
liveness facts. A candidate sourced solely from live tmux discovery has no durable recorded
server; **that successful discovery query is its server fact for the snapshot**, and it does
not fabricate a durable record.

On cost: N sessions on N recorded servers is N queries where the incumbent made one ambient
call. `SC-017k` permits **grouping candidates by recorded server and querying each once** —
every candidate's answer must still come from its own server and its exact name. That is the
sanctioned optimisation; the ambient shortcut is the defect.

**`SC-509d` lands here, not earlier.** This phase contains the first code able to construct
`Unknown`, and version 1 must never emit it — so the machine surface moves to version 2 in
this same change. A new value in an existing field is a consumer-visible contract change even
though the field name, JSON type and position are unchanged; versioning is the gate. An
earlier bump would version a domain nothing can produce, and a later one ships the break.

*Done when:* prefix siblings cannot mask each other, a downed server yields `UNKNOWN` rather
than `stopped`, no code path infers liveness from which block is rendering, and every emitted
digest carries `schema_version: 2`.

### Phase 3 — rendering and order in the output itself (`SC-017m`, `SC-017n`)

Human and JSON surfaces render `unknown` explicitly. Default and `--running` mean **active
inventory**: `RUNNING` then `UNKNOWN`. `--stopped` is `STOPPED` only. `--all` is `RUNNING`,
`UNKNOWN`, `STOPPED`. `UNKNOWN` never sets `degraded` by itself.

ae owns ordering: C byte order by session name within a status group; group order `RUNNING`,
`UNKNOWN`, `STOPPED`. No tmux version, locale, glob, or traversal order reaches output.

*Done when:* order is reproducible across platforms without consulting tmux, and every scope's
membership is asserted by a test that fails if a variant is added.

### Phase 4 — parity

Snapshot parity for the 1065 P1 rows, consuming the frozen corpus through `verify-corpus.py`
(which has no write path, deliberately). Normalisation is proven two-sided already: same
invocation across hosts converges, different invocations do not collide.

## Non-goals

Write domains (`send`, `state`, `goal`, `memo`, request *tracking*) are P2. Lifecycle is P3.
Daemons are P4. `ae next --attach` and session launch stay frozen as P2 parity inputs.

## Standing hazards for this build

- **Exhaustiveness covers `match` and nothing else.** Array literals, const lists and map
  initializers enumerating `Status` compile silently when a variant is added. Grep the
  variants by name; the compiler's list is a lower bound.
- **A gate that generates its input** validates its own output, not the commit.
- **Bash is evidence, never a normative oracle** — including its comments, which are claims
  inside the artifact under test.
