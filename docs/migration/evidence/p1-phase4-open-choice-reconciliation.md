# Phase-4 open-choice reconciliation

**By `grok46:txreview`, 2026-08-24.** Seat that did not author
`docs/migration/p1-phase4-open-choices.tsv` (colead) and did not author any of
the three accepted phase gates. Spec is phase-4 gate criterion 8. First
identity (`697e8507`, blob `b4d0f34d`) used gate `3a63f741`. This identity
rebinds the SC arm to the landed criterion-3 blob and cites HEAD gate
`4612208d` (C1's two-line insertion naming that C3 blob; criterion 8 body
unchanged). Colead's ruling: the rebind moves identity even if the occurrence
row set is unchanged, because the rebinding establishes freshness, not content.

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
| C3 contract-to-obligation recon | `0126d765d57da2f8cbe86e93660362121f96d2f8` |
| contract (C3's pin, HEAD) | `896d08ea3ac753095c04af17dfba92cd9d15fb38` |

C3 blob verified from HEAD before this pin: `git rev-parse
HEAD:docs/migration/evidence/p1-phase4-contract-obligation-reconciliation.md`
=`0126d765d57da2f8cbe86e93660362121f96d2f8`; sha256
`7a3fe7f7a75bc40f8b9dc624b8b3c72b128687c0d1cc8749615e551975a87f0a`.

Extractor reads `git cat-file -p` of the three accepted gate blobs, never the
worktree file. Phrase: `open[\s\-]*choice` (covers `OPEN CHOICE`,
`OPEN CHOICES`, `OPEN-CHOICE`, and the split-line form `**OPEN` /
`CHOICE**`). Criterion owner walks back to the last `N.` heading; the phase-2
"Phase 3 handoff" section is `HANDOFF`, not C23.

The SC arm is **the P1-applicable rows in C3 blob `0126d765`**, not this
seat's independent contract reading. Per-row agreement or divergence against
that census is recorded below. Divergence is a FINDING, not a silent merge.

## Method

1. Extract every ratified OPEN CHOICE phrase occurrence from the three accepted
   gate blobs (34 hits).
2. Bind C3 blob `0126d765` as the P1-applicable SC census. Compare every C3
   inventory row to this seat's first-identity SC reading, agree or diverge per
   row.
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
| `OC-P2-SEMANTIC-FALSE-PRESENCE` | P2-L155 (C13) | no |
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

C3 blob `0126d765` (`c6c65f2a`): 1,378 loci, six obligation IDs, contract
`896d08ea`. Companion inventory
`p1-phase4-contract-obligation-loci.tsv` is the per-row census this rebind
reads. First identity (`b4d0f34d`) listed six independently read rows as the
stand-in. Every C3 inventory row is compared to that reading below.
**AGREE** = same P1-applicable grain and same open-choice consequence.
**DIVERGE** = a FINDING, not silently merged.

### Rows this seat independently named as comparison-underdetermined

| Row / locus | C3 disposition | First-identity reading | Verdict |
|---|---|---|---|
| SC-017o completeness JSON + human diagnostic | directional corpus locus, 573, ID `SC-017o` | OPEN CHOICE: human wording/paths/rc; JSON owes the boolean; internal loss representation remain open | **AGREE** — phrase SC-L399 maps to `OC-P3-HUMAN-DIAGNOSTIC`; machine-loss remains `OC-P3-MACHINE-LOSS-RECORDS` |
| SC-017r human agent-health marker | directional corpus locus, 78, ID `SC-017r` | OPEN CHOICE: alive/dead/unknown words or glyphs | **AGREE** — phrase SC-L490 maps to `OC-P3-AGENT-HEALTH-TOKEN`; not in P1/P2/P3 gates |
| SC-509 `generated_at` field presence/type | retained corpus locus, 401 | field required | **AGREE** |
| SC-509 `generated_at` VALUE | underdetermined value locus, 401; successor P2 C17, P3 C3, P4 C8/C18 | exact timestamp bytes unspecified; register `OC-P3-GENERATED-AT` | **AGREE** — first-identity FINDING 2's prose note now has this referent. Ruled: SC-509 is the carrier |
| SC-017h human per-agent health/state/attn | retained corpus locus, 458; retain non-SC-017r presentation facts | layout/colour/headers unspecified; OC via P3 C15 named member, not an SC phrase | **AGREE** as P1-applicable retained. Layout exclusion is a gate named-set member (`OC-P3-HUMAN-LAYOUT`), not a C3-declared underdetermination |
| SC-017n C-byte group/name order | directional gap, 0; successor P3 C9/C10/C11 | no equal-name tie-breaker; OC via P3 C9 phrase | **AGREE** as P1-applicable. C3's gap is corpus-unscorable C-byte vs incidental order; the product-output OC is equal-name ties at P3 C9 (`OC-P3-EQUAL-NAME-TIE`) |

### C3 inventory rows this seat did not independently name

None of these use an OPEN CHOICE phrase. Classification: no new product-output
open-choice locus. **AGREE** they are P1-applicable as C3 dispositioned them.

| Row / locus | C3 disposition | Open-choice consequence |
|---|---|---|
| SC-017a default running-scope | retained, 859 | none — exact residue |
| SC-017b all-view status-group | retained, 859 | none |
| SC-017c stopped-only | retained, 859 | none |
| SC-017d attention filtering | retained, 859 | none; unknown-filter gap is SC-521c |
| SC-017e activity filtering | retained, 859 | none; unknown-filter gap is SC-521c |
| SC-017f JSON filter parity | retained, 401 | none |
| SC-017g attention marker | retained, 859 | none |
| SC-017i `--running` alias | retained, 859 | none |
| SC-017j candidate membership | directional gap, 0 | none — not a product-output OC |
| SC-017k recorded-server liveness | directional gap, 0 | none |
| SC-017l unknown session status | directional, 134 | none — mandated unknown, not open |
| SC-017m unknown membership/render | directional, 150 | none |
| SC-017p positive agent liveness | input carrier, 0 | none — carried by SC-017r / SC-509e |
| SC-017q unknown agent liveness | input carrier, 0 | none — carried by SC-017r / SC-509e |
| SC-017s pane live predicate | input carrier, 0 | none |
| SC-021 ls alias | retained, 116 | none |
| SC-400d durable-root membership | directional gap, 0 | none |
| SC-506 JSON validity | retained, 401 | none |
| SC-405l selector normalization | input carrier, 0 | none |
| SC-509 other retained v1 object fields | retained, 401 | see FINDING 3 on SC-509b |
| SC-509d schema_version | directional, 401 | none |
| SC-509e agents[].alive nullable | directional, 42 | none |
| SC-518 requests closure | retained, 168 | none |
| SC-521c unknown attn/activity filter | directional gap, 0 | none |
| SC-1306a list snapshot cut | retained, 743 | none |
| SC-1306d requests snapshot cut | retained, 168 | none |
| SC-1306e events-tail snapshot cut | retained, 38 | none |

C3 explicitly does not list SC-508 (unclassified code-observation). **AGREE** —
not a ratified P1 output obligation, so not an open-choice carrier.

Helpers `requests` / `events-tail` appear as SC-518 / SC-1306d / SC-1306e
retained exact. **AGREE** they carry none of the registered product-output
open-choice loci.

### First-identity rows C3 does not inventory

| Row | First-identity claim | C3 | Verdict |
|---|---|---|---|
| SC-509b semantic-false omission | permits omission of semantic false; phrase occurrence is P2 C13 (`OC-P2-SEMANTIC-FALSE-PRESENCE`) | no inventory row | **DIVERGE** — FINDING 3 |
| SC-510a event timestamp grammar | register cites it as `generated_at` format authority | no inventory row | **AGREE it is not a P1 output obligation.** C3 carries the value locus on SC-509 only. SC-510a is event keys, not list/ls output |

## Findings (not repairs)

1. **Discharged.** C3 recon is committed: blob `0126d765`, sha256
   `7a3fe7f7a75bc40f8b9dc624b8b3c72b128687c0d1cc8749615e551975a87f0a`,
   commit `c6c65f2a`. This identity pins it.
2. **Pinned, not a defect.** `OC-P3-GENERATED-AT` has no OPEN CHOICE phrase in
   the three accepted gates. C3 records it as SC-509 `generated_at VALUE`,
   an underdetermined value locus on 401 digest rows. That is the carrier the
   ruling named. The entry is supported, not orphaned.
3. **C3 inventory has no SC-509b row.** Register authority for
   `OC-P2-SEMANTIC-FALSE-PRESENCE` is `SC-509b + p1-phase2-gate.md criterion 13`.
   The phrase occurrence remains P2 C13, so the register row is not orphaned by
   this absence. C3's `SC-509` / `other retained version-1 object fields`
   presents remaining v1 fields as retained exact except SC-509d/e and SC-017o
   and does not split semantic-false `degraded` presence as underdetermined.
   That grain is a FINDING to the lead, not a silent merge and not a register
   edit.
4. **No omitted product-output phrase occurrence. No orphan register row
   under the accounting above.** Occurrence row set unchanged from `b4d0f34d`.
   Register and gates were not edited.

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
`2da4fb86`, and C3 blob `0126d765`. Completeness set present. Health-token
entry is SC-017r, not a prior gate. C15 is per-surface. P3 C10 stays
unregistered internal. `generated_at` VALUE is SC-509-carried, as C3 records
and as ruled.

Occurrence row set unchanged from first identity `b4d0f34d`. This identity
exists because the SC arm is now the C3 blob, not an independent reading.

FINDING 3 (C3 has no SC-509b row) is the only live FINDING. It does not
orphan `OC-P2-SEMANTIC-FALSE-PRESENCE` (P2 C13 still carries the phrase).
