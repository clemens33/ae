# P1 phase 2 — pre-registered falsification gate

Rows: SC-017k, SC-017l, SC-509d. Inputs: the complete SC-017j phase-1 candidate
collection, including SC-405l's typed selector and phase-1 provenance/read-loss facts.
Output: the same semantic candidate set with liveness classified, plus the successor
JSON digest at schema version 2.

**Authored by gpt56sol:colead, 2026-08-21, WITHOUT reading `src/` and before phase 2
exists.** Acceptance criteria derived after seeing an implementation are shaped by it;
this file fixes the failure conditions while they can still be independent.

**PASS requires every structural check and every required test below.** Review prose is
not a substitute for an observable. Each criterion labels the kind of obligation it
tests: `STATE`, `PAIR`, `SET`, or `SEQUENCE`. A state snapshot cannot close a sequence;
one member of a pair cannot close the relation.

## Status

**NOT RUN — phase 2 does not exist.** The lead is holding this gate unread by the
builder until implementation handback.

## Falsification criteria

1. **[SEQUENCE] THE COMPLETE INVENTORY PRECEDES CLASSIFICATION.** Record named
   `inventory complete` and `classify enter` events. At `inventory complete`, capture
   the semantic candidate set and its phase-1 facts; at the first `classify enter`,
   capture the classifier input. Observable: `inventory complete` occurs first and the
   two semantic sets are equal. FAIL if classification starts early, the input differs,
   or any candidate is materialized only inside classification. Raw entitled-server
   discovery may have run before or after durable discovery; order *within* inventory
   remains an **OPEN CHOICE**.

2. **[SET] CLASSIFICATION IS TOTAL AND CARDINALITY-PRESERVING.** Feed candidates that
   exercise every status and both degradation values. Observable: every phase-1
   identity appears exactly once after classification; no identity is added, dropped,
   merged, split, or rewritten, and record-read/degradation semantics survive. FAIL if
   `unknown` becomes absence, one candidate's backend failure aborts the whole set, or
   classification performs another reconciliation. Retaining source provenance and the
   typed selector *after* their liveness role is complete is an **OPEN CHOICE**; phase 2
   may not mutate or fabricate those facts in its input. Collection type and iteration
   order are also **OPEN CHOICES**.

3. **[PAIR] RECORDED SERVER BEATS AMBIENT EVIDENCE IN BOTH DIRECTIONS.** Use a durable
   candidate named `mdk` recorded on server B while server A is ambient. Run three
   opposed cells: (a) B returns exact `mdk` with positive ownership while A reports it
   absent; (b) B successfully proves exact `mdk` absent while A returns it positively
   owned; (c) B's query fails while A returns it positively owned. Observable for the
   durable candidate: `running`, `stopped`, `unknown`, respectively, with backend trace
   attributing its answer to B. FAIL on ambient fallback or cross-server evidence. Run
   the relation once with a positive named selector and once with a positive socket
   selector; query order/count is not asserted.

4. **[PAIR] EXACT NAME AND PREFIX SIBLING ARE OPPOSED.** On the candidate's recorded
   server, a successful response containing only positively owned `mdk-app` classifies
   durable `mdk` as `stopped`; adding exact positively owned `mdk` changes it to
   `running`. Observable: the two statuses and exact backend names. FAIL if prefix
   presence produces `running` or `unknown`, if `has-session -t mdk`-shaped prefix
   semantics appear on the classification path, or if the exact control does not change
   the result.

5. **[STATE TABLE] OWNERSHIP IS PART OF THE POSITIVE PROOF.** Hold recorded server,
   exact name, and successful query fixed across four cells: exact name with positive
   ae-ownership -> `running`; exact name with ownership missing -> `unknown`; exact name
   with ownership mismatched -> `unknown`; exact name absent -> `stopped`. Use the
   accepted phase-1 ownership predicate and opposed marker fixtures rather than
   inventing new marker bytes here. FAIL if existence alone means running, missing or
   mismatched ownership means stopped/absence, or renderer/caller provenance changes a
   cell.

6. **[PAIR MATRIX] SUCCESS AND FAILURE OVERRIDE IDENTICAL PAYLOAD BYTES.** Push two
   payloads through the exact backend capture primitive: empty session output and output
   containing the exact positively owned candidate. For each payload, run one
   demonstrated successful query and one demonstrated transport/backend failure with
   identical bytes. Observable: empty-success -> `stopped`, exact-owned-success ->
   `running`, and both failed queries -> `unknown`. FAIL if payload bytes alone decide
   the status, partial output from a failed query supplies positive or negative proof,
   either arm is uncalibrated, or failure yields running/stopped/absence.

7. **[STATE TABLE] EVERY NON-PROOF ROUTES TO UNKNOWN WITHOUT DELETION.** Construct
   separate candidates for: SC-405l `missing` from a readable record; SC-405l `missing`
   from an absent record; SC-405l `missing` from an unreadable record; SC-405l
   `ambiguous`; positive selector whose server is unreachable; positive selector whose
   enumeration query returns a non-transport failure; exact live name with missing
   ownership; exact live name with mismatched ownership; and exact live name whose
   ownership query fails. Observable: all candidates remain and all classify `unknown`.
   Missing and ambiguous selectors cause no backend target derived from their raw bytes;
   positive unreachable/failed selectors show an attempted query of their typed target.
   The three `missing` cells retain distinct phase-1 record-read facts even though their
   selector state agrees. FAIL on `stopped`, omission, guessed entitlement, fabricated
   selector knowledge, or whole-set error. Retry and diagnostic policy are **OPEN
   CHOICES**.

8. **[SET] QUERY FAILURE IS LOCAL TO ITS SERVER GROUP.** Put at least two candidates on
   failing server A and candidates with opposed exact-name facts on successful server B.
   Observable: every A candidate is `unknown`; B candidates independently become
   `running` or `stopped`; the output identity set equals the input set. FAIL if A's
   failure suppresses B, one status is broadcast to both groups, or the classifier
   returns no set.

9. **[SEQUENCE/STATE] A TMUX-ONLY CANDIDATE REUSES ITS DISCOVERY FACT.** Phase 1 supplies
   a candidate sourced solely from a successful, positively owned live discovery on
   server A. After `inventory complete`, make A unavailable before classification.
   Observable: the candidate remains `running` from that snapshot's successful discovery
   fact. Its durable-source provenance remains absent, no durable selector/record is
   fabricated in the phase-1 input or filesystem, and a before/after filesystem capture
   is unchanged. The classified output need not retain provenance once consumed. FAIL if a
   re-query downgrades it, if it becomes unknown/absent, or if transient
   discovery evidence is persisted as durable state. How the snapshot fact is carried
   in memory is an **OPEN CHOICE**.

10. **[PAIR/SEQUENCE] DUAL PROVENANCE PRESERVES THE MATCHED SNAPSHOT PROOF.** Use the
    exact phase-1 criterion-20 coalescence: one durable candidate with a positive server
    selector plus an exact, positively owned live sighting from a successful query of
    that recorded server. After `inventory complete`, make the server unavailable before
    classification. Observable: the one coalesced candidate remains `running`; the
    matched discovery fact is the successful own-server proof for this snapshot, and no
    second candidate appears. Opposed control: a durable-only candidate with the same
    selector/name but no matched live sighting becomes `unknown` when its query fails.
    FAIL if dual provenance is discarded, if a later re-query replaces the accepted
    snapshot proof, or if merely having a durable source prevents reuse of that proof.
    A redundant query and the in-memory proof carrier are **OPEN CHOICES**; neither may
    change this snapshot's classification.

11. **[GROUP RELATION] GROUPING BY RECORDED SERVER MAY SHARE WORK, NEVER ANSWERS.** On
    one recorded server, classify three durable candidates from one successful response:
    exact positively owned `alpha`, absent `beta`, and exact but unowned `gamma`.
    Observable: `running`, `stopped`, `unknown`, respectively, each attributed to its
    exact name on that server. FAIL if grouping broadcasts one answer, loses ownership
    per name, or consults another server. One query, several queries, caching, and
    retries are all accepted; assert the semantic server set, not exact call count.

12. **[GROUP RELATION] SAME NAME ON DIFFERENT SERVERS NEVER SHARES LIVENESS.** Three
    durable candidates have the same inventory name but positive selectors for A, B and
    C. A returns exact positive ownership, B successfully proves absence, C fails.
    Observable: `running`, `stopped`, `unknown` on the three distinct identities. FAIL
    if grouping keys on name alone, selector spellings are treated as proof of server
    equivalence, or any result crosses server identity. Positively proven equivalent
    server selectors MAY be grouped per SC-017k; raw spelling inference may not.

13. **[STATE MATRIX] UNKNOWN AND DEGRADED ARE ORTHOGONAL THROUGH THE EMITTED DIGEST.**
    Construct the required four cells while keeping the non-varied axis fixed:
    (`running`, not degraded),
    (`running`, degraded), (`unknown`, not degraded), (`unknown`, degraded). Use a
    separate phase-1 read-loss fact for degradation; do not manufacture it by breaking
    the liveness query. For the unknown pair, hold selector state at SC-405l `missing`:
    use a readable record with no selector for not-degraded and an absent or genuinely
    unreadable record for degraded. Observe every pair twice: at classifier output and in
    the emitted schema-version-2 digest. Flip degradation alone and require identical
    query/status at both boundaries; flip liveness proof alone and require identical
    degradation at both. Add `stopped` with both degradation values as the SC-509b
    preservation control. FAIL if unknown sets degraded, degraded forces unknown, the
    serializer drops/forces degradation specifically for unknown, the two missing-
    selector cells collapse their distinct read-loss provenance before emission, false
    is confused with omitted true, or any cell is unconstructible. SC-509b permits
    omission of semantic false; exact JSON presence for false is an **OPEN CHOICE**.

14. **[STATE/BOUNDARY] PHASE 2 CONSUMES PHASE-1 FACTS; IT DOES NOT REDISCOVER THEM.**
    Instrument durable-root/meta access after `inventory complete`. Observable: no
    phase-2 filesystem discovery or meta reread; classification consumes the typed
    selector, provenance, discovery fact, and read-loss fact already supplied. FAIL if
    phase 2 rescans roots, repairs a missing phase-1 fact by rereading disk, or derives
    degradation from a new read. Backend liveness queries remain allowed.

15. **[PAIR] RENDER/FILTER PATH CANNOT CHANGE KNOWLEDGE.** Classify one fixed candidate
    set once, then hand the result to any available caller/view controls separately.
    Observable: the classified statuses are byte/semantic-identical before rendering.
    Diff check: the classifier accepts no renderer-block or list-filter provenance as a
    liveness fact. FAIL if default/`--running`/`--stopped`/`--all` selection, human vs
    JSON route, or the code path that requested classification changes a status.
    Rendering, filtering, and final ordering belong to SC-017m/n and are outside this
    phase; this criterion does not prescribe their outputs.

16. **[STATE MATRIX] EVERY SUCCESSOR DIGEST IS SCHEMA VERSION 2 WITH THE CLOSED STATUS
    DOMAIN.** Emit successor digests for: empty inventory; running only; stopped only;
    unknown only; mixed statuses; and the degradation matrix. Observable: numeric
    `schema_version: 2` exactly once in every document, and every `sessions[].status` is
    exactly one of lowercase `running | unknown | stopped`. FAIL on a conditional bump
    only when an unknown exists, version 1 on any successor path, null/free-text/alias
    status, invalid JSON, or a status outside the closed domain. JSON field order is an
    **OPEN CHOICE**.

17. **[PAIR] SCHEMA VERSION 2 PRESERVES THE REST OF SC-509/SC-509b.** For a fixture
    whose liveness remains running/stopped across the flip, compare version-1 baseline
    semantics with the successor digest. Observable: apart from schema version and the
    expanded status domain, all still-applicable SC-509 fields and SC-509b degradation
    semantics survive. FAIL if the version bump silently drops/renames unrelated fields,
    changes omission/null rules, or turns degradation into liveness. Exact generated
    timestamps are normalized before comparison; byte identity is not required.

18. **[SEQUENCE/PAIR] THE VERSION BUMP AND FIRST PRODUCTION UNKNOWN ARE ONE CHANGE.**
    Compare the accepted phase-1 baseline with the complete phase-2 integration change
    set. Baseline observable: schema version 1 and no production classification path can
    construct/emit unknown. Phase-2 observable: the first such path exists and every
    successor digest is version 2. FAIL if either side can enter a shippable/gated state
    without the other, including an earlier schema bump that versions an unchanged
    emitted domain or a later bump after unknown became reachable. Local/WIP commit
    topology is an **OPEN CHOICE**; this criterion binds the integrated change, not one
    commit boundary. A pre-existing but unreachable `Status::Unknown` enum variant is
    explicitly allowed; constructibility/emission, not declaration, is the boundary.

19. **[PAIR] VERSION 1 NEVER EMITS UNKNOWN.** Retain a frozen/version-1 digest control
    that exercises its full status domain and contains no unknown. Search every emitted
    successor digest for the forbidden pair `schema_version: 1` plus status `unknown`.
    Observable: zero such documents. FAIL on any pair. If no version-selectable successor
    serializer exists, do not invent one for the test; parent/frozen output plus the
    all-path successor-v2 matrix closes the implication.

20. **[PAIR/CONTROL] THE QUERY RECORDER CAN DISTINGUISH SUCCESS, ABSENCE, OWNERSHIP AND
    FAILURE.** Before relying on liveness results, demonstrate the exact capture/backend
    seam with calibrated responses for successful-empty, successful-exact-owned,
    successful-exact-unowned, and explicit failure. Record target server, exact returned
    names, ownership result, transport success/failure, and invocation result separately.
    FAIL the gate if a control does not land, if success/failure collapse to the same
    record, or if the recorder's own mutation changes product input. Fake backend versus
    isolated tmux is an **OPEN CHOICE**, but at least one isolated two-server adapter test
    must prove typed Name/Socket routing reaches the intended real server rather than only
    proving a mock received an argument.

21. **[SCOPE GUARD] DO NOT TURN THIS GATE INTO PHASE 3.** Classifier representation,
    query grouping/count/order, caching, retry count, error/diagnostic channel, and
    in-memory proof carrier are **OPEN CHOICES** where criteria above do not constrain
    them. Do not assert human rendering order, filter membership, attention behavior, or
    product-owned C sorting here; SC-017m/n and SC-521c own those. FAIL this gate itself
    if a test rejects an otherwise correct classifier for one of those unratified choices.

22. **[SET/BEHAVIOR] PHASE 2 NEVER EXPANDS PHASE-1 ENTITLEMENT.** Use the phase-1
    entitled-server set as the authorization upper bound, then place valid but
    unentitled tmux sockets/server names in plausible scan locations. Instrument both
    filesystem enumeration and every backend target. Observable: every server phase 2
    contacts belongs to the phase-1 entitled set, and phase 2 performs no arbitrary
    socket-path or server-name sweep. FAIL on any extra target or broad socket/server
    enumeration even when its result is discarded and every final status is otherwise
    correct. How phase 2 carries or recomputes an allowed subset from the phase-1 typed
    facts is an **OPEN CHOICE**; it may not rediscover entitlement by sweeping. Querying
    any subset of entitled servers, grouping equivalent selectors, query order, caching,
    and retry count also remain **OPEN CHOICES**. This criterion bounds authorization,
    not provenance or the amount of permitted work.

23. **[PAIR/STATE] INVENTORY COMPLETENESS SURVIVES CLASSIFICATION AND EMISSION.** Feed
    the same candidate set, selectors, read-loss facts, and backend answers twice. The
    only difference is a phase-1 completeness fact: complete with zero enumeration
    losses versus incomplete with one named logical-source loss. Observable: classified
    identities, statuses, and degradation facts are identical; completeness and loss
    facts cross the classifier boundary unchanged; emitted schema-version-2 JSON carries
    `inventory_complete: true` versus `false`. Repeat with an empty candidate set so an
    incomplete-empty snapshot cannot masquerade as authoritative empty. FAIL if the
    classifier clears or recomputes the loss, maps incompleteness to `unknown` or
    `degraded`, fabricates a candidate, drops healthy candidates, or emits the same
    boolean for both arms. Reuse phase-1 criterion 24's simultaneous two-source failure
    and require both distinguishable loss facts to cross classification; retaining only
    the first is a failure even though JSON still needs only the boolean. Internal loss
    representation, detailed JSON loss exposure, and human warning policy are outside
    this phase.

## Phase 3 handoff — pre-registered consequences, not phase-2 PASS conditions

- Run every human list filter/view over one fixed incomplete snapshot. For each filter,
  the emitted rows must equal that filter applied to the candidates the incomplete
  inventory actually found; do not compare against identities visible only when the
  failed source becomes readable. Every incomplete invocation emits an explicit stderr
  diagnostic containing at least the logical-source loss count. Reuse phase-1 criterion
  24's simultaneous two-source failure and require the reported count to be `2`, so a
  boolean or constant-one diagnostic fails. FAIL if only `--all` warns, a filter hides
  the warning, found rows change, or a synthetic session/status represents the loss.
  Exact wording, optional paths/targets, and exit status remain **OPEN CHOICES**.
- Run human complete/incomplete controls and JSON complete/incomplete controls, including
  both empty inventories. Complete human output emits no incompleteness warning. Every
  successor JSON document carries the top-level boolean with the correct value regardless
  of filter or emptiness; version 1 remains unchanged. Detailed machine loss records are
  **OPEN CHOICE** because SC-017o requires only the boolean.
