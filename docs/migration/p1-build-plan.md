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

**Already landed.** Type-level work ahead of phase 1, because it blocked nothing:
`Status::Unknown` with the ratified semantics; `Scope::order()` holding the SC-017m composition
and SC-017n group order at one site; `Status::ALL` plus an exhaustive-match guard and a coverage
test closing the array-literal hole.

**Phase 1 is landed and its gate PASSES** — see `p1-phase1-gate.md` for the 24
pre-registered criteria. History, because the file's own `Status` header is stale and
describes only its original run: all 24 criteria were re-run against gate blob `8e3c9ec0`
at `fab19eb4` and reported 24/24 MET. Phase 1 then **reopened** when phase-2 rework
changed inventory and session-snapshot code, and was independently re-gated against that
same `8e3c9ec0` at `00196e32`, returning PASS. The test-only phase-2 repair that followed
moved no phase-1 or product bytes, so that PASS is preserved by object identity rather
than by assumption. Phase 2 therefore did not proceed on an unresolved phase-1 verdict.
`src/inventory.rs` holds the candidate collection with its invariants **structural rather than
defended**: `Roots` cannot
spell an archive path, `durable_records()` opens nothing inside a candidate directory, and
`candidates()` starts as every durable record and only pushes. The live half is a called port,
`trait Discovery`, with **no name parameter anywhere** — so SC-017k's forbidden per-candidate
existence check is not expressible in the type.

No product code constructs `Unknown` yet and `SCHEMA_VERSION` is still 1 — both deliberate, and
both change in phase 2. The phase-2 gate is pre-registered at `p1-phase2-gate.md`; its
pre-implementation status was **NOT RUN**, when phase 2 did not yet exist, and it has not been
re-run since, for the same reason the phase-1 one was.


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

**The refusal is removed HERE, and nowhere earlier.** `src/lib.rs`'s `NO_SESSION_SOURCE` stands
until inventory, liveness and rendering all exist, because an empty or partial listing on a
machine that HAS sessions is a wrong answer wearing the shape of a right one — which is the
reasoning the const's own doc comment carries. Deleting it before this phase would trade an
honest refusal for a confident falsehood.

*Done when:* order is reproducible across platforms without consulting tmux, every scope's
membership is asserted by a test that fails if a variant is added, and `ae list` answers instead
of refusing.

### Phase 4 — parity, which is NOT "match the corpus"

Uniform byte-parity against the corpus is wrong, and measurably so. `SC-509d` moves the schema
for **every** successor digest, so divergence is not a carve-out for defect rows — it is the
majority of the machine surface, driven by one enum value:

| rows | disposition |
|---:|---|
| 76 | diverge on **both** status and schema |
| 152 | diverge on status |
| 325 | diverge on schema alone |
| 306 | status-bearing, expected identical |
| 206 | carry neither field (`requests`, `events-tail`) — expected identical |

**553 of 1065 rows (52%) would go red precisely when the implementation is correct.**

The keying property is **what the output carries**, not which scenario produced the row: 859
rows carry a status field, 401 of those carry the machine digest, 206 carry neither
(verified, not assumed).

So every P1 row carries a **pre-registered verdict** — `EXPECTED-MATCH` or
`EXPECTED-DIVERGENCE` — derived from which rows govern its output shape. **An
expected-divergence row is not skipped; it asserts the mandated divergence.** A row that must
move `stopped` → `unknown` fails if it still says `stopped` *and* fails if it says anything
else. That makes the 553 the strongest part of the suite rather than a hole in it: those are
the rows proving the fix, not the port.

Normalisation is already proven two-sided — same invocation across hosts converges, different
invocations do not collide — and the corpus is consumed through `verify-corpus.py`, which has
no write path by construction.

### Known corpus gaps — successor tests cover these, not the corpus

The corpus exercises an **unreachable recorded server** (228 rows, verified end to end: a
recorded socket that is absent, `error connecting`, and bash printing `stopped` in both
surfaces where `SC-017l` mandates `unknown`). It does **not** exercise:

- a live **prefix sibling** in the firing orientation (co-occurring pairs exist; all oriented
  the wrong way — the defect needs a non-live candidate prefixing a live one)
- a **non-ambient** server, as distinct from no server
- an **ambiguous** recorded server
- `unknown` and `degraded` **together**, which `SC-017l` asserts are orthogonal
- a **missing ownership marker** — and this one is *unobservable*, not absent: ownership is
  proved via the tmux session environment and the capture recorded pane options, so the corpus
  cannot answer in either direction

Measured after `SC-400d` and `SC-405l` landed, the corpus is thinner still, and two of these
can **never** be corroborated by parity:

- the **worktree-nested layout** is absent — zero paths across 177 case manifests and the
  template fixture bytes, measured both ways. One case holds a `./worktrees` directory and it is
  empty, which `SC-400d` disposes of by name. **0 of 1065 rows.**
- the **anti-deduplication clause** is equally unexercised: no two candidates anywhere share a
  leaf across roots, so no corpus row can distinguish a correct implementation from one keyed on
  the leaf alone.
- `positive(socket)` is universal (130 occurrences, every kind in the corpus); `positive(name)`
  is zero, including the legacy kind-absent form the row specifically ratifies; all six
  `ambiguous` sub-states are zero.

For those, **successor tests are not the primary evidence — they are the only evidence this
corpus will ever provide.** Phase-1 gate criterion 2 already requires the layout fixture, so the
gate is not blocked; nothing downstream will corroborate it. These gaps are *absent* rather than
*unobservable*, which is the cheaper kind: a fixture closes each one.

The five original gaps are **not** captured before building, deliberately. They would capture *bash behaving
defectively*, which is already source-proven and, for the prefix primitive, observed. What P1
needs is proof the **successor** behaves correctly — and that is successor tests constructing
these scenarios directly, which do not depend on the corpus at all. The captures upgrade an
empirical label; they do not gate an implementation, and frozen source means they are exactly
as available later.

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
