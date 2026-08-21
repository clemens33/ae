# P1 phase 4 — pre-registered parity falsification gate

Input: the accepted phase-3 product, the frozen Batch C corpus, the P1 invocation
partition, and the contract-derived obligation table. Output: one accounted result for
every P1 invocation and every obligation locus.

**Authored by gpt56sol:colead, 2026-08-21, before any phase-4 implementation
exists.** This authoring pass did not inspect `src/` or an implementation of the parity
runner. The criteria come from the contract, the frozen corpus, and the already accepted
phase gates. Phase-4 code must not be used to choose what phase 4 checks.

**PASS requires every structural check and required test below.** Each criterion labels
the kind of obligation it observes: `STATE`, `PAIR`, `SET`, `SEQUENCE`, `BOUNDARY`,
`PROVENANCE`, or `CONTROL`. A green comparison is not enough: the runner must show which
fact it compared, which evidence supports that fact, and which facts the corpus cannot
score.

## Status

**NOT RUN — phase 4 does not exist.** The fixed gate is the implementation's acceptance
standard, not a description of whatever is later built.

## Fixed inputs and vocabulary

The corpus is the named, hash-pinned view recorded in
`docs/migration/evidence/corpus/FREEZE.txt`: 6,862 files at root digest
`802c882bca64453e33efce5351e43b5954ddecc3daed6c2b0b6c8833487b4e12`.
`INVOCATIONS.tsv` contains 1,065 P1 rows: 743 `ae list`, 116 `ae ls`, 168
`helper:requests`, and 38 `helper:events-tail` rows. P1-adjacent and P2 rows remain
frozen but do not enter this gate.

The obligation table's current committed projection contains 1,378 obligations over
581 carrying invocation rows: 713 `OBSERVED` and 665 `UNSCORABLE`. These numbers are a
reconciliation control for the current contract, not hand-maintained authority. The
table's contract-blob freshness relation is authoritative: if the contract moves, phase
4 stops for re-derivation even when the old counts still happen to agree.

Terms:

- An **invocation row** is one `(case, consumer)` from the P1 partition.
- An **obligation locus** is the addressable field, row-set fact, or stream property one
  contract row requires to change.
- `OBSERVED` means the accepted artifacts contain the decisive input/frozen fact needed
  to score that locus.
- `UNSCORABLE` means the obligation still holds, but this corpus lacks a decisive fact.
  It is not match, divergence, pass, failure, or permission to guess.
- A row may be **partial**: every scorable locus can pass while another locus on the same
  row remains unscorable. Support belongs to a locus, never to the whole row.
- **Parity is not match.** A match obligation requires equality; a directional
  obligation requires its exact mandated result. Any other difference fails.

## Falsification criteria

1. **[STATE/BOUNDARY] ALL REFERENCES ARE FIXED BEFORE COMPARISON.** Before invoking the
   successor, run the read-only corpus, invocation, and obligation verifiers against
   committed bytes. Record the corpus root digest, contract blob, invocation-table blob,
   obligation-table blob, gate blob, and successor commit in the run manifest. All must
   remain unchanged through the run. FAIL on corpus drift, a stale contract relation,
   an uncommitted input, a verifier that rewrites what it checks, or a result whose fixed
   identities cannot be reproduced. The obligation freshness check is HEAD-relative by
   design; an uncommitted contract edit therefore invalidates the run rather than being
   silently included.

2. **[SET] THE P1 POPULATION IS EXACTLY THE RATIFIED 1,065 ROWS.** Derive the set from
   `INVOCATIONS.tsv`, then assert both directions: every P1 row runs exactly once and no
   P1-adjacent, P2, unresolved, provenance-only, or out-of-scope row runs. Reconcile the
   four surface counts against 743/116/168/38 before measuring inside the population.
   FAIL on case-directory census, consumer-shape inference, a filtered subset, duplicate
   execution, or a total inferred from successful runs. A failed or unscorable row stays
   in the denominator.

3. **[PROVENANCE/SET] CONTRACT-TO-OBLIGATION RECONCILIATION IS INDEPENDENT OF THE
   GENERATOR.** Before parity, produce a committed reconciliation against the current
   contract that accounts for every P1-applicable output obligation and every obligation
   id in the table, both directions. The method must not invoke `obligations.py`, reuse
   its predicates, or accept its authority prose as proof; it reads the contract and raw
   artifacts independently. A contract row may map to zero corpus loci only with a named
   coverage gap and the pinned successor criterion that carries it. FAIL on an orphan
   obligation, an applicable contract rule with no obligation/gap, or a reconciliation
   derived from the same predicate as the table. Seed one omitted obligation and one
   orphan id separately; each must make this check red.

4. **[PAIR/CONTROL] THE OBLIGATION TABLE PROVES ITS OWN FRAMING.** For every obligation,
   re-read `from` from the captured stream, validate the closed stream/predicate/support
   domains, and verify its `(case, consumer)` is a P1 row. Derive carrying-row verdicts
   from obligations; never use the legacy `VERDICTS.tsv` verdict as authority. A stored
   divergence with no obligation fails as unexplained; a stored match that gains a new
   obligation is reported as legacy drift, never used to suppress the obligation. The
   verifier has no write path. Red-prove at least: wrong `from`, wrong locus, wrong
   stream, unknown predicate, unknown support, orphan row, deleted obligation, and an
   unexplained divergence. Verify each seed landed before reading its rc.

5. **[SET/PAIR] SUPPORT IS LOCUS-SCOPED, NEVER ROW-SCOPED.** Evaluate every `OBSERVED`
   locus against its exact predicate. Preserve every `UNSCORABLE` locus in the result and
   refuse to score it. On a mixed row, report the scorable locus result and the separate
   unscorable locus; do not promote or discard either because of the other. FAIL if one
   observable field makes the whole row pass, one unavailable fact makes observable
   fields disappear, `UNSCORABLE` becomes expected-match/expected-divergence, or a row is
   called PASS while one of its obligations is unscorable. This is the governing rule
   for partial evidence.

6. **[PAIR/CONTROL] SELECTOR-MISSING IS INDEPENDENT OF CASE-SERVER FAILURE.** Reproduce
   the current discriminator from raw manifests and the P1 partition: ten selector-
   missing cases total; six also carry the case-level failed-query condition; the four
   live-socket cases are `a9-c03-meta-mode-000-ro`, `a9-c03-meta-mode-000-rw`,
   `a9-c05-meta-absent-ro`, and `a9-c05-meta-absent-rw`. Their four list invocations each
   must carry an `OBSERVED` SC-017m locus requiring an unknown row to be present. FAIL if
   any of these sixteen obligations is gated on case-level unreachability, labelled
   SC-017l, widened into a prediction of the whole row set, or omitted. A live sighting
   may add a running candidate; it cannot remove the durable selector-missing unknown
   candidate. Current reconciliation control: SC-017m `OBSERVED` is 30 and SC-017l
   `OBSERVED` remains 14.

7. **[PAIR] EVERY OBSERVED OBLIGATION MUST MOVE IN THE EXACT MANDATED DIRECTION.** For
   each `OBSERVED` locus, assert the table's `to` value with its declared predicate.
   Retaining `from` fails; producing any third value fails; merely producing different
   bytes fails. A carrying row with multiple observed obligations passes only when every
   one passes independently. Use opposed seeds for unchanged, wrong-direction, missing,
   and duplicated target values. The runner must name the failed obligation id and locus,
   not only the invocation.

8. **[PAIR/SET] MATCHED FACTS STAY MATCHED AND OPEN CHOICES STAY OPEN.** A P1 row with no
   obligation and no cited open-choice locus must reproduce captured rc, stdout, stderr,
   and no-mutation evidence exactly. On a directional row, compare every retained
   contract field and semantic row not owned by an obligation; an expected change is not
   permission for unrelated drift. Separately enumerate open choices already ratified by
   the earlier gates, including diagnostic wording/path detail/exit status, JSON warning
   policy, human layout, and literal agent-health glyphs. Do not compare those as frozen
   obligations. FAIL on an unregistered semantic difference, blanket byte-difference
   acceptance, or rejection of a correct implementation for an open choice. Comparator
   implementation and how it projects a changed human row are **OPEN CHOICES**; the
   retained facts it must compare are not.

9. **[PAIR/STATE] SUCCESSOR-ONLY DIGEST FIELDS ARE TESTED BEFORE RETAINED CONTENT.** For
   every digest invocation, independently assert exactly one numeric `schema_version: 2`
   and exactly one boolean `inventory_complete`. Assert the completeness VALUE only when
   its locus is `OBSERVED` or a criterion-11 controlled successor arm supplies the missing
   fact; field presence and type do not make an `UNSCORABLE` value scorable. Then compare
   every retained version-1 session field. FAIL if schema version or completeness alone
   causes the whole document to be accepted as different, if either field is absent,
   duplicated, or wrongly typed, or if filtering drops or rewrites a retained field.
   Red-proof with a document carrying correct new fields and a wrong status; the comparator
   must fail on status rather than pass on schema divergence.

10. **[PAIR/SET] MULTIPLE OBLIGATIONS ON ONE INVOCATION REMAIN MULTIPLE.** Exercise an
    observed incomplete human row and its paired digest so status/membership, diagnostic,
    inventory completeness, schema version, and any supported agent-health obligation
    are checked at their own loci. Remove or corrupt each obligation one at a time while
    holding the others correct; each mutation must fail by its own id. FAIL on one boolean
    `different`, first-obligation-only handling, or a row verdict that hides which of its
    obligations passed, failed, or remained unscorable.

11. **[PROVENANCE/PAIR] AN UNSCORABLE CORPUS LOCUS IS CLOSED ONLY BY A SEPARATE,
    PRODUCT-VALID SUCCESSOR ARM.** Phase 4 may make a missing server fact testable by
    injecting and recording a typed result through the same discovery/transport boundary
    the product uses. The arm records selector type/value, contacted server, transport
    success/failure, exact session name, ownership result, and the resulting locus; its
    opposed control must change the answer. Hand-built `World`, `Snapshot`, classified
    status, digest, or expected output does not close the corpus gap. The original corpus
    obligation remains labelled `UNSCORABLE`; the controlled arm is separate evidence,
    never retroactive observation. If no product-valid arm exists, retain the locus as
    unscorable and link the exact prior phase-gate criterion, or phase 4 does not close it.

12. **[SET/PROVENANCE] CORPUS ABSENCE IS REPORTED, NOT LAUNDERED INTO COVERAGE.** Re-run
    the sufficiency census against the current contract. At minimum it must name the
    absent worktree-nested layout, cross-root same-leaf anti-dedup case, positive name
    selector, ambiguous selector forms, live prefix sibling in the firing orientation,
    non-ambient live server, and the unobservable ownership-marker cell. It also names the
    pending positive/negative/ambiguous per-agent pane matrix until real transport closes
    it. Each gap points to its only evidence: a pinned successor test, a controlled phase-4
    arm, or an explicit unresolved blocker. FAIL if the parity summary implies those rows
    were exercised, if zero examples are reported as a pass, or if an incidental new
    corpus specimen is ignored rather than reclassified.

13. **[BOUNDARY/CONTROL] THE FROZEN PRODUCT IS NEVER RE-RUN TO ANSWER A NEW QUESTION.**
    The baseline side consists only of accepted files named by the corpus manifest. From
    parity entry onward, instrument child execution and tmux/server access; no frozen
    binary, generated frozen helper, original capture hook, or original server is invoked
    to fill an evidence gap. Calibrate the recorder with a harmless child and tmux query
    that must appear. FAIL on an uncalibrated zero, a fallback rerun after a parse failure,
    or any comparison rule that requires an artifact the frozen capture never recorded.
    A new successor-only controlled arm is permitted under criterion 11 and is never a
    frozen-side answer.

14. **[SEQUENCE/BOUNDARY] EACH SUCCESSOR RUN STARTS FROM A VERIFIED SCRATCH FIXTURE.**
    Verify corpus and case hashes first; materialise fixture bytes outside the corpus;
    prove the clone fingerprint equals the case's recorded fingerprint; record effective
    normalized argv and environment remapping; then invoke. Afterward compare the scratch
    state manifest against the recorded no-mutation expectation. Instrument corpus paths
    as read-only and require zero writes. FAIL if invocation precedes clone verification,
    a stale scratch tree is reused, the product sees corpus paths directly, one case can
    affect the next, or a read-side invocation mutates state. Scratch location and copy
    mechanism are **OPEN CHOICES**.

15. **[PAIR/CONTROL] INVOCATION NORMALISATION IS THE ACCEPTED TWO-SIDED RELATION.** Run
    the committed `verify-invocations.py` and pin its blob. Require convergence for the
    same semantic invocation under different host/run prefixes and no collision for
    different flags, flag order, session identity, or surface. Record the raw and
    normalized argv used for every successor row. Red-prove by erasing `--json` or another
    semantic token and by changing only a host prefix. FAIL if normalisation decides P1
    scope, normalises captured output, conflates `list` and `ls` provenance, or is trusted
    from a count without an independently inspected collision specimen.

16. **[SET/STATE] EVERY P1 ROW AND LOCUS GETS ONE TERMINAL ACCOUNTING STATE.** Report
    locus counts for PASS, FAIL, and UNSCORABLE, then row counts for PASS, FAIL, and
    PARTIAL. A row is PASS only when all its obligations/comparable facts are scorable and
    pass; FAIL if any scorable fact fails; PARTIAL when all scorable facts pass and at
    least one obligation remains unscorable. The three row counts sum to 1,065, and the
    locus counts reconcile to the independently verified obligation inventory. Empty,
    crashed, skipped, unsupported, or unimplemented invocations remain named rows rather
    than falling out of the denominator. The phase summary says **scorable parity passed**
    only with zero FAIL; it may say **P1 parity closed** only when every PARTIAL locus also
    has the separate product-valid evidence criterion 11 requires.

17. **[PAIR/CONTROL] THE COMPARATOR IS RED-PROVEN AGAINST BOTH FAILURE DIRECTIONS.** In
    disposable copies, verify each seed landed, then require nonzero for: one changed byte
    on a match-only row; a required divergence left unchanged; a required locus changed to
    the wrong third value; a correct schema-only change masking wrong retained content;
    one obligation deleted from a multi-obligation row; an `UNSCORABLE` locus reported as
    PASS; an extra semantic divergence outside all obligations/open choices; swapped case
    outputs; a corpus write; and a normalized-argv collision. Each mutation has a named,
    local target and bounded delta. A self-test proves non-regression for imagined inputs;
    it does not establish adequacy, so an independent reviewer must add at least one
    mutation the comparator author did not design.

18. **[PAIR/STATE] REPLAY IS REPRODUCIBLE WITHOUT CROSS-CASE STATE.** Run at least one
    match-only row, one observed directional row, one mixed-support row, and one
    controlled successor arm twice from independently materialised scratch state. Require
    identical captured successor bytes and identical locus results within each pair,
    except fields the contract explicitly makes runtime-variable. Reverse the case order
    and require the same per-case answers. FAIL on shared tmux state, clock leakage,
    scratch reuse, order-dependent obligation evaluation, or a normaliser that makes two
    distinct cases share an output.

19. **[SCOPE GUARD] PHASE 4 COMPARES; IT DOES NOT REOPEN PHASES 1–3.** Candidate
    discovery, liveness knowledge, schema domains, rendering/filtering, and product order
    are consumed from their accepted gates. Phase 4 may expose a defect and fail; it may
    not repair expectations to fit output, weaken an obligation, infer a missing product
    fact, or turn an earlier open choice into a required byte spelling. The gate itself
    FAILS if it rejects a correct implementation for corpus-absent behavior, detailed
    loss-record schema, diagnostic prose, agent-health token choice, internal comparator
    design, scratch path, or any other choice the cited rows leave open.

## Required handback

The phase-4 handback pins: successor commit; this gate blob; corpus root; contract,
invocation, obligation, and verifier blobs; the independent reconciliation; raw terminal
row/locus accounting; every controlled-arm manifest; and the red-proof report. It states
separately:

1. scorable parity result;
2. remaining partial/unscorable loci;
3. which separate successor evidence closes each corpus gap; and
4. which contract rows remain unimplemented or empirically pending.

No single PASS token may collapse those four answers.
