# Phase-4 open-choice reconciliation

**By `grok46:txreview`, 2026-08-24.** Seat that did not author
`docs/migration/p1-phase4-open-choices.tsv` (colead) and did not author any of
the three accepted phase gates. Spec is phase-4 gate criterion 8.

Identities: first (`697e8507`, blob `b4d0f34d`) used independent SC reading
against gate `3a63f741`; second (`9294dbc5`, blob `6a155272`) pinned C3
`0126d765` and HEAD gate `4612208d`. This identity rebinds the SC arm from
`0126d765` to C3 `343fcd80` (`0194c465`). Occurrence row set unchanged;
identity moves because the SC arm is a new named object.

This file is the independently produced open-choice-reconciliation blob
criterion 1 pins. It does not modify the register or any gate. A mismatch is a
FINDING, not a repair.

Machine census: `p1-phase4-open-choice-occurrences.tsv`.
Verifier: `verify-open-choice-reconciliation.py`.
Isolated red-proof: `redproof-open-choice-reconciliation.py`.

## Pins

| Input | Blob |
|---|---|
| this gate (HEAD, C1 names C3) | `4612208d411f352cd6a049e24278e472f7c58e66` |
| C8 spec as first authored | `3a63f7416ccda870a503ac5e11fb2f53ccbea2a1` |
| open-choice register | `2da4fb86933a6b8edee15fd61596d6f53fa6c550` |
| accepted phase-1 gate | `8e3c9ec0b031f4947260d4e0327bad562a10fdcd` |
| accepted phase-2 gate | `29db943aa85319534301332052105ba16df03b4d` |
| accepted phase-3 gate | `8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c` |
| C3 contract-to-obligation recon | `343fcd80916cdffc4a3d7a25e865056e0fb8d336` |
| prior C3 identity (replaced) | `0126d765d57da2f8cbe86e93660362121f96d2f8` |
| contract (C3's pin, HEAD) | `896d08ea3ac753095c04af17dfba92cd9d15fb38` |

C3 blob verified from HEAD before this pin: `git rev-parse
HEAD:docs/migration/evidence/p1-phase4-contract-obligation-reconciliation.md`
=`343fcd80916cdffc4a3d7a25e865056e0fb8d336`; sha256
`f32a9242260df637c764b8e4b0be605bb5b3c006a0f401a6878ab76affd39848`.
Commit `0194c465`, transport-approved and post-commit extended.

Extractor reads `git cat-file -p` of the three accepted gate blobs, never the
worktree file. Phrase: `open[\s\-]*choice` (covers `OPEN CHOICE`,
`OPEN CHOICES`, `OPEN-CHOICE`, and the split-line form `**OPEN` /
`CHOICE**`). Criterion owner walks back to the last `N.` heading; the phase-2
"Phase 3 handoff" section is `HANDOFF`, not C23.

The SC arm is **the P1-applicable rows in C3 blob `343fcd80`**, not this
seat's independent contract reading and not the superseded `0126d765` census.
Per-row agreement or divergence against that census is recorded below.
Divergence is a FINDING, not a silent merge.

## Method

1. Extract every ratified OPEN CHOICE phrase occurrence from the three accepted
   gate blobs (34 hits).
2. Bind C3 blob `343fcd80` as the P1-applicable SC census. Compare every C3
   inventory row to this seat's first-identity SC reading and to the previous
   C3 pin `0126d765`, agree or diverge per row.
3. Classify each phrase occurrence as a product-output locus with an exact
   register `CHOICE_ID`, or as an internal/test/topology/retry choice that
   subtracts no product-output locus.
4. Expand phase-3 criterion 15's named set into per-member rows so C15 stays
   per-surface and is not a flat union.
5. Account every register row in the other direction.
6. Seed omitted and orphan on isolated copies; verify each seed landed; require
   red. Tracked files are never mutated.

## Direction A — every occurrence, classified

### Phase 1 (`8e3c9ec0`) — 9 occurrences, all internal

| Occ | Owner | Locus | Class |
|---|---|---|---|
| P1-L047 | C1 | test-only observation seam | internal |
| P1-L070 | C4 | unreadable-record carrier | internal |
| P1-L089 | C7 | diagnostic/error side channel | internal |
| P1-L109 | C10 | grouping, caching, retries, query count | internal |
| P1-L148 | C16 | candidate representation | internal |
| P1-L158 | C18 | collection/query order, multi-provenance carrier | internal |
| P1-L166 | C19 | `AE_TMUX_SERVER` selection, outside this phase | internal |
| P1-L189 | C21 | exact enum/field representation | internal |
| P1-L242 | C24 | loss carrier, error detail, retry, ordering | internal |

No phase-1 occurrence subtracts a product-output locus. Phase-1 loss *carrier*
is not the later JSON/human diagnostic; those are phase-3 surfaces.

### Phase 2 (`29db943a`) — 15 occurrences

| Occ | Owner | Locus | Class | Register |
|---|---|---|---|---|
| P2-L031 | C1 | discovery order within inventory | internal | — |
| P2-L039 | C2 | provenance retained after liveness | internal | — |
| P2-L041 | C2 | collection type / iteration order | internal | — |
| P2-L090 | C7 | retry and diagnostic policy | internal | — |
| P2-L109 | C9 | in-memory snapshot-proof carrier | internal | — |
| P2-L121 | C10 | redundant query / proof carrier | internal | — |
| P2-L155 | C13 | JSON presence vs omission of semantic-false `degraded` | product-output | `OC-P2-SEMANTIC-FALSE-PRESENCE` |
| P2-L183 | C16 | JSON object member order | product-output | `OC-P3-JSON-FIELD-ORDER` |
| P2-L201 | C18 | local/WIP commit topology | internal | — |
| P2-L219 | C20 | fake backend vs isolated tmux | internal | — |
| P2-L225 | C21 | classifier representation, grouping, cache, retry, channel | internal | — |
| P2-L238 | C22 | carrying the entitled-server subset | internal | — |
| P2-L240 | C22 | subset/group/order/cache/retry | internal | — |
| P2-L279 | HANDOFF | incomplete-human wording, paths, rc | product-output | `OC-P3-HUMAN-DIAGNOSTIC` |
| P2-L284 | HANDOFF | detailed machine loss records | product-output | `OC-P3-MACHINE-LOSS-RECORDS` |

P2 C13 is the completeness-set member criterion 8 names. It maps to
`OC-P2-SEMANTIC-FALSE-PRESENCE`; that exclusion is applied before
`OC-P3-JSON-FIELD-ORDER` checks the remaining member set.

P2 C17 says "Exact generated timestamps are normalized before comparison; byte
identity is not required." That sentence is **not** an OPEN CHOICE phrase
occurrence. It is supporting text for `OC-P3-GENERATED-AT`, recorded under
direction B.

The two HANDOFF hits live in "Phase 3 handoff — pre-registered consequences,
not phase-2 PASS conditions." They are still ratified occurrences in the
accepted P2 blob. Phase 3 C12/C13 absorb them; they are the same loci, not
extra register rows.

### Phase 3 (`8cccbe44`) — 8 phrase hits, C15 expanded per-surface

| Occ | Owner | Locus | Class | Register |
|---|---|---|---|---|
| P3-L056 | C1 | refusal-removal site / internal wiring | internal | — |
| P3-L069 | C2 | move/borrow/copy/stream; filter-versus-sort; buffered vs iterator | internal | — |
| P3-L145 | C9 | equal-name tie order (SC-017n supplies no tie-breaker) | product-output | `OC-P3-EQUAL-NAME-TIE` |
| P3-L158 | C10 | sort implementation, stable-sort algorithm, filter-versus-sort | internal | — |
| P3-L189 | C12 | incomplete-human wording, paths, rc | product-output | `OC-P3-HUMAN-DIAGNOSTIC` |
| P3-L202a | C13 | detailed machine loss records and their order | product-output | `OC-P3-MACHINE-LOSS-RECORDS` |
| P3-L202b | C13 | JSON stderr warning policy; JSON process rc retained | product-output | `OC-P3-JSON-WARNING` |
| P3-L215 | C15 | the finite named set (container) | named-set | — |
| P3-L229 | C15 | review rule: do not fail a correct implementation solely for a named open choice | review-rule | — |

C15 named members, each keeping the surface scope of its owning criterion:

| Member | Surface scope | Class | Register |
|---|---|---|---|
| human table layout, colors, headers, whitespace | human stdout | product-output | `OC-P3-HUMAN-LAYOUT` |
| incomplete-human diagnostic wording, path detail, exit status from C12 | human stderr and rc | product-output | `OC-P3-HUMAN-DIAGNOSTIC` |
| JSON stderr warning policy from C13; JSON process rc remains retained | JSON stderr | product-output | `OC-P3-JSON-WARNING` |
| JSON object field order | JSON stdout members | product-output | `OC-P3-JSON-FIELD-ORDER` |
| detailed machine loss records and their order | JSON stdout subtrees | product-output | `OC-P3-MACHINE-LOSS-RECORDS` |
| internal collection or sort implementation | none | internal-member | — |
| equal-name tie-breaking | human and JSON order inside that tie | product-output | `OC-P3-EQUAL-NAME-TIE` |

C15 is per-surface, not a flat union: incomplete-human **rc is open**; JSON
**process rc is retained**. The register matches that split:
`OC-P3-HUMAN-DIAGNOSTIC` excludes human stderr and rc;
`OC-P3-JSON-WARNING` excludes only the calibrated stderr span and still
requires the JSON document and process rc.

P3 C10 is the completeness-set member criterion 8 names as deliberately
unregistered: sort implementation, stable-sort algorithm, and
filter-versus-sort sequence subtract no product-output locus. Their only
output-visible underdetermination is equal-name tie order, owned by
`OC-P3-EQUAL-NAME-TIE` (P3 C9 + C15 member).

### Contract phrase occurrences

| Occ | Owner | Locus | Class | Register |
|---|---|---|---|---|
| SC-L399 | SC-017o | human diagnostic wording, paths/targets, exit status | product-output | `OC-P3-HUMAN-DIAGNOSTIC` |
| SC-L490 | SC-017r | exact alive/dead/unknown words or glyphs | product-output | `OC-P3-AGENT-HEALTH-TOKEN` |

`OC-P3-AGENT-HEALTH-TOKEN` is carried by the P1-applicable-SC arm through
SC-017r, not by an occurrence in the three earlier gate blobs. The three
token values remaining distinct and nonempty stay required; only the literal
spelling is excluded.

## Direction B — every register row

| CHOICE_ID | Supporting occurrences | Orphan? |
|---|---|---|
| `OC-P2-SEMANTIC-FALSE-PRESENCE` | P2-L155 (C13). C3's SC-509b row is the distinct `degraded: true` after actual loss (14), not this OC | no |
| `OC-P3-GENERATED-AT` | C3 `SC-509` / `generated_at VALUE` (underdetermined value locus, 401); P3 C3 opposed-clock residual; P2 C17 timestamp normalization (no phrase) | no — C3 pin |
| `OC-P3-HUMAN-LAYOUT` | P3-C15-HUMAN-LAYOUT | no |
| `OC-P3-HUMAN-DIAGNOSTIC` | P2-L279, P3-L189, P3-C15-HUMAN-DIAGNOSTIC, SC-L399 | no |
| `OC-P3-JSON-WARNING` | P3-L202b, P3-C15-JSON-WARNING | no |
| `OC-P3-AGENT-HEALTH-TOKEN` | SC-L490 only (not in P1/P2/P3 gates) | no |
| `OC-P3-EQUAL-NAME-TIE` | P3-L145, P3-C15-EQUAL-NAME | no |
| `OC-P3-JSON-FIELD-ORDER` | P2-L183, P3-C15-JSON-FIELD-ORDER | no |
| `OC-P3-MACHINE-LOSS-RECORDS` | P2-L284, P3-L202a, P3-C15-MACHINE-LOSS, SC-L402 sc-arm | no |

Nine register rows, nine accounted. Zero omitted product-output phrase
occurrences. Completeness set present: P2 C13, P3 C10, P3 C12, P3 C13, P3 C15.

## C3-bound SC arm

C3 blob `343fcd80` (`0194c465`): 1,614 relations, eight obligation IDs
(including SC-509b 14 and SC-509c 222), contract `896d08ea`. Companion
inventory `p1-phase4-contract-obligation-loci.tsv` is the per-row census.
Compared to first identity (`b4d0f34d`) and to the superseded C3 pin
`0126d765`. **AGREE** = same P1-applicable grain and same open-choice
consequence. **DIVERGE** = a FINDING, not silently merged.

### SC-509b (the hard row)

C3 now inventories it as a **directional corpus locus, 14 relations, ID
`SC-509b`**. Raw basis: 20 P1 JSON inputs whose fixture metadata marks that
session's `meta` `UNREADABLE` or `FILE ABSENT`; fourteen of those coalesce to
captured `sessions[].degraded` output loci. Successor pins: P2 C13 and P4
C7/C9. C3's own prose: this row is **distinct from phase-2 criterion 13's
open choice about emitting `false`**; C13 does not name this locus; the
contract's fixed `degraded: true` requirement is selected from raw actual
loss only.

First identity claimed SC-509b as "permits omission of semantic false" and
mapped that to `OC-P2-SEMANTIC-FALSE-PRESENCE`. That was the **false-presence**
open choice, which lives at P2 C13, not the **true-after-loss** directional
obligation. C3's re-derivation from raw inputs matches the register's
`STILL_REQUIRED` clause (`degraded true is present and true`) and does not
absorb the excluded locus (presence vs omission of semantic false).

**AGREE.** FINDING 3 is discharged: the row exists, is re-derived not
zero-stamped, and does not orphan or swallow `OC-P2-SEMANTIC-FALSE-PRESENCE`.
No new product-output open choice.

### Rows this seat independently named as comparison-underdetermined

| Row / locus | C3 `343fcd80` | vs first identity / vs `0126d765` | Verdict |
|---|---|---|---|
| SC-017o completeness JSON + human diagnostic | directional, 573, ID `SC-017o` | unchanged | **AGREE** — SC-L399 → `OC-P3-HUMAN-DIAGNOSTIC`; machine-loss → `OC-P3-MACHINE-LOSS-RECORDS` |
| SC-017r human agent-health marker | directional, 78, ID `SC-017r` | unchanged | **AGREE** — SC-L490 → `OC-P3-AGENT-HEALTH-TOKEN`; not in P1/P2/P3 gates |
| SC-509 `generated_at` field presence/type | retained, 401 | unchanged | **AGREE** |
| SC-509 `generated_at` VALUE | underdetermined value locus, 401; P2 C17, P3 C3, P4 C8/C18 | unchanged | **AGREE** — ruled SC-509 carrier |
| SC-017h human per-agent presentation | retained, 458; non-SC-017r facts | unchanged | **AGREE**. Layout OC is P3 C15 (`OC-P3-HUMAN-LAYOUT`), not a C3-declared underdetermination |
| SC-017n C-byte group/name order | directional gap, 0; P3 C9/C10/C11 | unchanged | **AGREE**. Product-output OC is equal-name ties at P3 C9 (`OC-P3-EQUAL-NAME-TIE`) |

### C3 inventory rows: unchanged grain, no new open choice

| Row / locus | C3 `343fcd80` | Open-choice consequence |
|---|---|---|
| SC-017a default running-scope | retained, 859 | none |
| SC-017b all-view status-group | retained, 859 | none |
| SC-017c stopped-only | retained, 859 | none |
| SC-017d attention filtering | retained, 859 | none |
| SC-017e activity filtering | retained, 859 | none |
| SC-017f JSON filter parity | retained, 401 | none |
| SC-017g attention marker | retained, 859 | none |
| SC-017i `--running` alias | retained, 859 | none |
| SC-017j candidate membership | directional gap, 0 | none |
| SC-017k recorded-server liveness | directional gap, 0 | none |
| SC-017l unknown session status | directional, 134 | none |
| SC-017m unknown membership/render | directional, 150 | none |
| SC-021 ls alias | retained, 116 | none |
| SC-400d durable-root membership | directional gap, 0 | none |
| SC-506 JSON validity | retained, 401 | none |
| SC-405l selector normalization | input carrier, 0 | none |
| SC-509d schema_version | directional, 401 | none |
| SC-509e agents[].alive nullable | directional, 42 | none |
| SC-518 requests closure | retained, 168 | none |
| SC-521c unknown attn/activity filter | directional gap, 0 | none |
| SC-1306a list snapshot cut | retained, 743 | none |
| SC-1306d requests snapshot cut | retained, 168 | none |
| SC-1306e events-tail snapshot cut | retained, 38 | none |

### C3 inventory rows that moved vs `0126d765`

| Row / locus | `0126d765` | `343fcd80` | Open-choice consequence | Verdict |
|---|---|---|---|---|
| SC-509b degraded true after actual loss | absent (FINDING 3) | directional, 14, ID `SC-509b`; raw 20→14 | not the false-presence OC; P2 C13 still owns that | **AGREE** — FINDING 3 discharged; see hard-row section |
| SC-509c agents[].reason contribution | absent | directional, 222, ID `SC-509c`; 128 state + 94 alert | none — mandated field value, not an OPEN CHOICE phrase | **AGREE** — P1-applicable; no product-output OC |
| SC-509 other retained v1 object fields | retained exact except 509d/e and 017o | retained exact except 509b/d/e, 509c, and 017o | none | **AGREE** — the previous overclaim on remaining v1 fields is gone |
| SC-017p positive agent liveness | input carrier, 0 | directional gap, 0; successor P4 C12 | none | **AGREE** — named-gap-plus-pinned-criterion; not fully-carried |
| SC-017q unknown agent liveness | input carrier, 0 | partial, 120 carried by SC-017r/SC-509e; matrix still P4 C12 | none beyond SC-017r's already-registered token OC | **AGREE** |
| SC-017s pane live predicate | input carrier, 0 | directional gap, 0; successor P4 C12 | none | **AGREE** |

C3 still does not list SC-508 (unclassified). **AGREE.** SC-510a is still not
a P1 output obligation. **AGREE** — `generated_at` VALUE stays on SC-509.
Helpers `requests` / `events-tail` remain SC-518 / SC-1306d / SC-1306e
retained exact. **AGREE.**

## Findings (not repairs)

1. **Discharged.** C3 recon is committed at blob `343fcd80`, sha256
   `f32a9242260df637c764b8e4b0be605bb5b3c006a0f401a6878ab76affd39848`,
   commit `0194c465`. This identity pins it.
2. **Pinned.** `OC-P3-GENERATED-AT` has no OPEN CHOICE phrase in the three
   accepted gates. C3 still records it as SC-509 `generated_at VALUE` on 401
   digest rows.
3. **Discharged.** SC-509b is now a directional 14-locus row re-derived from
   raw `UNREADABLE` / `FILE ABSENT` metadata, distinct from P2 C13's
   false-presence open choice. `OC-P2-SEMANTIC-FALSE-PRESENCE` remains
   supported by P2 C13.
4. **No omitted product-output phrase occurrence. No orphan register row.
   No live divergence against `343fcd80`.** Occurrence row set unchanged from
   `b4d0f34d`. Register and gates were not edited.

## Red-proof

Tracked register and occurrences table are never mutated. Seeds go to an
isolated temp directory. Each seed is diffed first; an unlanded seed is
INVALID.

Recorded run against this commit's files:

```
neutral            rc=0  clean  (tracked files untouched throughout)
OMITTED          delta=1   rc=1 ids=OMITTED                caught  (dropped OC-P3-HUMAN-LAYOUT)
ORPHAN           delta=1   rc=1 ids=ORPHAN                 caught  (added OC-FAKE-ORPHAN)
restored           rc=0  clean
RED-PROOF: BOTH DIRECTIONS PROVEN BY NAMED CHECK
```

OMITTED seed: delete `OC-P3-HUMAN-LAYOUT` from an isolated register copy.
C15's named human-layout member then has no exact register row.

ORPHAN seed: append `OC-FAKE-ORPHAN` to an isolated register copy. No
occurrence cites it.

## Verdict

Accounting holds both directions against the pinned gate blobs, register
`2da4fb86`, and C3 blob `343fcd80`. Completeness set present. Health-token
entry is SC-017r, not a prior gate. C15 is per-surface. P3 C10 stays
unregistered internal. `generated_at` VALUE is SC-509-carried. SC-509b is
the directional true-after-loss grain; P2 C13 remains the false-presence
open choice.

Occurrence row set unchanged from first identity `b4d0f34d`. This identity
exists because the SC arm moved `0126d765` → `343fcd80`.

No live FINDING. FINDING 3 is discharged by C3's raw-derived SC-509b row.
