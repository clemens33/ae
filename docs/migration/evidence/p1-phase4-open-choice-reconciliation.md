# Phase-4 open-choice reconciliation

**By `grok46:txreview`, 2026-08-24.** Seat that did not author
`docs/migration/p1-phase4-open-choices.tsv` (colead) and did not author any of
the three accepted phase gates. Spec is phase-4 gate blob `3a63f741`, criterion
8, not a lead paraphrase.

This file is the independently produced open-choice-reconciliation blob
criterion 1 pins. It does not modify the register or any gate. A mismatch is a
FINDING, not a repair.

Machine census: `p1-phase4-open-choice-occurrences.tsv`.
Verifier: `verify-open-choice-reconciliation.py`.
Isolated red-proof: `redproof-open-choice-reconciliation.py`.

## Pins

| Input | Blob |
|---|---|
| this gate (criterion 8) | `3a63f7416ccda870a503ac5e11fb2f53ccbea2a1` |
| open-choice register | `2da4fb86933a6b8edee15fd61596d6f53fa6c550` |
| accepted phase-1 gate | `8e3c9ec0b031f4947260d4e0327bad562a10fdcd` |
| accepted phase-2 gate | `29db943aa85319534301332052105ba16df03b4d` |
| accepted phase-3 gate | `8cccbe44787d4ea6007ad9cf9d1cc83a3d03936c` |
| contract (HEAD at authoring) | `896d08ea3ac753095c04af17dfba92cd9d15fb38` |

Extractor reads `git cat-file -p` of those blobs, never the worktree file, for
the three gates. The contract is HEAD-relative because criterion 8's SC arm is
the P1-applicable rows identified against the current contract. Phrase:
`open[\s\-]*choice` (covers `OPEN CHOICE`, `OPEN CHOICES`, `OPEN-CHOICE`, and
the split-line form `**OPEN` / `CHOICE**`). Criterion owner walks back to the
last `N.` heading; the phase-2 "Phase 3 handoff" section is `HANDOFF`, not C23.

## Method

1. Extract every ratified OPEN CHOICE phrase occurrence from the three accepted
   gate blobs (34 hits).
2. Extract every OPEN CHOICE phrase occurrence from the current contract (2
   hits: SC-017o L399, SC-017r L490). Independently note SC-017o L402 "remain
   open" (not the phrase) and SC-509 `generated_at` (required field, exact
   timestamp bytes unspecified).
3. Classify each phrase occurrence as a product-output locus with an exact
   register `CHOICE_ID`, or as an internal/test/topology/retry choice that
   subtracts no product-output locus.
4. Expand phase-3 criterion 15's named set into per-member rows so C15 stays
   per-surface and is not a flat union.
5. Account every register row in the other direction.
6. Seed omitted and orphan on isolated copies; verify each seed landed; require
   red. Tracked files are never mutated.

Criterion 3's independent contract-to-obligation reconciliation is **not a
committed artifact at HEAD**. That is FINDING 1. The SC arm below is this
seat's independent contract reading, not a reading of a C3 blob. If that blob
later identifies a P1-applicable OPEN CHOICE row this census missed, this
reconciliation needs a new identity.

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
| `OC-P2-SEMANTIC-FALSE-PRESENCE` | P2-L155 (C13) | no |
| `OC-P3-GENERATED-AT` | SC-509 L1367 sc-arm; P3 C3 opposed-clock residual; P2 C17 timestamp normalization (no phrase) | no — see FINDING 2 |
| `OC-P3-HUMAN-LAYOUT` | P3-C15-HUMAN-LAYOUT | no |
| `OC-P3-HUMAN-DIAGNOSTIC` | P2-L279, P3-L189, P3-C15-HUMAN-DIAGNOSTIC, SC-L399 | no |
| `OC-P3-JSON-WARNING` | P3-L202b, P3-C15-JSON-WARNING | no |
| `OC-P3-AGENT-HEALTH-TOKEN` | SC-L490 only (not in P1/P2/P3 gates) | no |
| `OC-P3-EQUAL-NAME-TIE` | P3-L145, P3-C15-EQUAL-NAME | no |
| `OC-P3-JSON-FIELD-ORDER` | P2-L183, P3-C15-JSON-FIELD-ORDER | no |
| `OC-P3-MACHINE-LOSS-RECORDS` | P2-L284, P3-L202a, P3-C15-MACHINE-LOSS, SC-L402 sc-arm | no |

Nine register rows, nine accounted. Zero omitted product-output phrase
occurrences. Completeness set present: P2 C13, P3 C10, P3 C12, P3 C13, P3 C15.

## Independent SC reading (stand-in for criterion 3)

P1-applicable list/ls output rows that underdetermine a comparison locus:

- **SC-017o** — OPEN CHOICES: human wording, paths/targets, exit status.
  Internal loss representation and ordering remain open (L402, not the phrase).
  JSON owes the boolean, not a loss-record schema.
- **SC-017r** — OPEN CHOICE: alive/dead/unknown words or glyphs.
- **SC-509** — `generated_at` is a required digest field; the exact timestamp
  value is not specified. Authority cited by the register together with SC-510a
  (timestamp grammar) and phase-3 criterion 3 (opposed clock must not move
  planted snapshot facts).
- **SC-509b** — permits omission of semantic false; the phrase occurrence is
  P2 C13, not this row.
- **SC-017h** — tabular health/state/attn view; layout/colour/headers are not
  specified here. Phrase occurrence is P3 C15's named member.
- **SC-017n** — C-byte order within status groups; no equal-name tie-breaker.
  Phrase occurrence is P3 C9.

No other contract row uses the OPEN CHOICE phrase. Helpers
(`requests`, `events-tail`) are P1 invocations but carry none of these
surface-open loci.

## Findings (not repairs)

1. **Criterion 3's independent contract-to-obligation reconciliation is not
   committed at HEAD.** Criterion 8 tells this seat to use the P1-applicable SC
   rows that blob identifies. The blob does not exist, so the SC arm is this
   seat's independent contract reading. That is a phase-4 input gap, not a
   register defect.
2. **`OC-P3-GENERATED-AT` has no OPEN CHOICE phrase in the three accepted
   gates.** Analogous to the health-token recording criterion 8 already
   requires for SC-017r: the entry is supported by SC-509 (P1-applicable digest
   field, exact value unspecified) plus phase-3 criterion 3's opposed-clock
   residual. It is not an orphan under that SC-arm reading. If a later C3 blob
   refuses SC-509 as an open-choice carrier, the row becomes orphan and this
   file is stale.
3. **No omitted product-output phrase occurrence. No orphan register row
   under the accounting above.** Register and gates were not edited.

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

Accounting holds both directions against the pinned blobs and this seat's
independent SC reading. Completeness set present. Health-token entry is
SC-017r, not a prior gate. C15 is per-surface. P3 C10 stays unregistered
internal.

FINDING 1 (missing C3 blob) is the unresolved input. This reconciliation does
not close it.
