# P1 phase 1 — pre-registered falsification gate (SC-017j)

**Authored by gpt56sol:colead, 2026-08-21, WITHOUT reading the implementation.** Recorded
verbatim by fable5:lead. Transcription can only under-report, so where this disagrees with the
author's message, the message is the original.

**Amended before any implementation repair, still without reading the implementation.** An
independent completeness critique by opus5:lexec found that the forward list admitted a missing
exactly-one union, collapsed record-loss provenance, and an unproved phase boundary; it also
found the query-trace/set wording conflict and the gate's unstated exhaustive-discovery policy.
The joint SC-400d/SC-405l/SC-017j ruling resolved the normative gaps. Criteria 2–4, 10 and 18
were narrowed or strengthened; criteria 20–23 are the durable additions. The temporal repair
requires union-before-classification but deliberately does not invent an order within discovery.

Written before the diff deliberately: acceptance criteria derived *from* an implementation are
shaped by it — you read the code, the code suggests what to check, and the checks pass because
they came from the thing under test. See `docs/gatekeeping.md`, *"If a judgement must be
INDEPENDENT of a thing, it has to be made BEFORE that thing exists"*.

**PASS requires every structural check plus every required test below.** A missing test is a
gate failure where the behaviour cannot be established from the diff alone; review prose is not
a substitute.

## Status against `f6c8f0a` (phase 1 as landed)

**DOES NOT PASS.** The original failures exposed contract gaps; SC-400d, SC-405l and
the SC-017j reconciliation precision now resolve them normatively. Against the landed
phase-1 report (not an implementation read by this gate's author), the one-root build
still fails the two-layout obligation, and the selector/reconciliation cases remain
unimplemented or unproved. The criteria were amended before any implementation repair.

| blocked by | criteria |
|---|---|
| SC-405l now defines the durable **server selector** read domain; landed phase 1 has no reported reader/tests for it | 10, 11, 12, 20, 23, and part of 14 |
| SC-400d now requires **two durable layouts**; landed phase 1 has one root | 2, 6, 20 |
| SC-017j now defines one-to-one source reconciliation; no reported exactly-one or ambiguous-cardinality proof exists | 20 |

The gate independently re-derived both gaps from the normative side, while the builder hit them
from the implementation side. That convergence is what establishes they are real rather than a
misreading.

## The criteria

1. **PHASE OBSERVATION SEAM.** A unit or integration test must observe the candidate collection
   at the inventory/classification boundary. FAIL if the only observable is a rendered
   `SessionEntry` after liveness, filtering, or schema conversion; then phase separation cannot
   be attributed. *OPEN CHOICE:* private function, internal type, or test-only seam.

2. **BOTH DURABLE ROOT CLASSES.** Test must construct one valid durable candidate under the
   canonical sessions root and one under the ratified legacy-readable worktree layout, with
   distinct stable identities. The legacy fixture's outer worktree name MUST differ from its
   inner session-name leaf; add a bare worktree-directory control with no nested state directory.
   Observable: both state-directory identities present in the phase-1 candidate collection,
   the legacy candidate named by its inner leaf, and the bare control absent. FAIL if either
   candidate is absent, the outer name becomes the inventory name, the bare worktree appears,
   or the legacy root is treated as archive/history.

3. **ARCHIVES ARE ZERO INPUT.** Take a baseline candidate collection, then add (a) an
   archive-only identity, (b) an archive entry whose basename collides with a live durable
   candidate, (c) an archive meta record naming an otherwise unentitled server. Observable:
   semantic candidate collection and **set of queried servers** identical to baseline. FAIL if an
   archive-only identity appears, a durable candidate changes, or archive bytes confer
   entitlement.

4. **UNREADABLE META DOES NOT DELETE THE DIRECTORY CANDIDATE.** Create a real durable directory
   in EACH SC-400d layout whose meta read demonstrably fails under the invoking uid; **chmod
   alone is insufficient if the runner can still read it.** Observable: both directory
   identities/paths remain candidates, while no server entitlement is derived from unread bytes.
   FAIL on omission, whole-inventory error, fabricated meta facts, or a server query sourced from
   either unreadable record. The carrier is an *OPEN CHOICE*; preserving a separately observable
   durable-source/read-failure fact is not (criterion 21).

5. **ENTITLED LIVE-ONLY DISCOVERY, WITH OWNERSHIP CONTROL.** Two tmux-only sessions on the
   already-resolved ambient server, neither with durable state: one with positive `AE_SESSION`
   ownership evidence, one without. Observable: the marked identity appears; the unmarked
   control does not. FAIL if the marked session is absent or an unmarked tmux-only session
   enters inventory.

6. **DISTINCT IDENTITIES WITH ONE BASENAME.** Two durable candidates sharing a basename but with
   different positive stable identities, preferably one per durable root. Observable: two
   independently addressable candidates survive. FAIL if the count becomes one, one overwrites
   the other, or facts from both are merged under a basename key. No output-order assertion.

7. **SERVER-QUERY FAILURE CANNOT REMOVE DURABLE INPUT.** Construct durable candidates, then make
   enumeration of one entitled server fail as a demonstrated backend/transport failure.
   Observable: every durable candidate remains in phase-1 output; candidates from other
   successfully queried entitled servers still appear. FAIL if the function returns no
   inventory, drops a durable candidate, or converts the failure into any status. A
   diagnostic/error side channel is an *OPEN CHOICE*.

8. **PREFIX ORIENTATION RED TEST.** Durable candidate `mdk`, no live `mdk`, and a separately
   live positively marked `mdk-app` on an entitled server. Observable: `mdk` remains, and
   `mdk-app` may enter as its own live-only candidate. FAIL if `mdk` vanishes, is merged into
   `mdk-app`, or its identity changes. **This exact short-dead/long-live orientation is
   required; mere co-occurrence is an invalid test.**

9. **EXACT LIVE NAME WITHOUT OWNERSHIP MARKER.** Test A: durable `mdk` plus exact live tmux
   `mdk` with `AE_SESSION` absent or mismatched — observable: durable `mdk` remains. Test B
   control: the same unmarked live session with no durable directory — observable: no candidate.
   FAIL if A disappears or B appears. This separates durable survival from positive live-only
   ownership.

10. **FINITE POINTER-DERIVED ENTITLEMENT.** Three isolated servers: A the already-resolved
    ambient server; B named by a valid, unambiguous selector in a durable candidate; C live and
    discoverable to the harness but named by no durable candidate. A positively marked tmux-only
    session on each. Observable: A and B live-only identities appear, C does not, and the
    backend query trace names only A and B. FAIL if C is queried or appears, or if B is not
    queried. This is exhaustive over the finite entitled set because SC-017j now requires an
    enumeration attempt for every distinct entitled server. *OPEN CHOICE:* positively proven
    equivalent-server grouping, caching, retries, and exact query count.

11. **MISSING AND AMBIGUOUS SELECTORS CONFER NOTHING BUT DELETE NOTHING.** One durable candidate
    with no selector, one with a demonstrably ambiguous/invalid selector, plus marked live-only
    sessions on the raw servers those bytes might tempt an implementation to use. Observable:
    both durable candidates remain; neither raw server enters the query trace; those live-only
    sessions remain absent unless independently entitled. FAIL on candidate loss, guessed
    selector, or accidental entitlement.

12. **OUTSIDE-SET LIVE SESSION IS ABSENT, NOT CLASSIFIED.** In the A/B/C test, observable for C
    is **no phase-1 candidate at all and no status artifact**. FAIL if C appears as
    running/stopped/unknown, or if any placeholder candidate is fabricated for it. This is the
    epistemic-limit outcome, not a liveness result.

13. **NO SOCKET SWEEP.** Needs an instrumented integration test, not diff judgment alone. Plant
    unentitled, valid tmux sockets/server names in plausible scan locations beside entitled
    fixtures; record filesystem enumeration and tmux backend targets. Observable: access limited
    to canonical durable roots plus the resolved A/B server pointers; no arbitrary socket
    directory enumerated and no unentitled server contacted. FAIL on any broad
    socket-path/server-name sweep **even if its sessions are later filtered out — a
    candidate-absence assertion alone is insufficient because a sweeper can query then discard.**

14. **DURABLE SUBSET INVARIANCE ACROSS TMUX WORLDS.** Run the same durable fixture under four
    tmux conditions: server failure; short-dead/long-live prefix sibling; exact-live-without-
    marker; no live server. Observable: projection onto the planted durable stable identities is
    identical in all four runs. FAIL on any missing or rewritten durable identity. Extra
    positively owned live-only candidates are compared separately, not treated as a set
    mismatch.

15. **DISCOVERY CALL BOUNDARY.** Diff plus query-trace check: phase 1 may enumerate each entitled
    server and read `AE_SESSION` ownership for names returned by that enumeration. FAIL if it
    performs per-durable-candidate `has-session`/existence checks, uses any tmux result to assign
    liveness, or gates durable inclusion on query success. Ownership filtering is allowed only
    for tmux-only discovery; it never filters durable discovery.

16. **NO STATUS OR UNKNOWN CONSTRUCTION.** Diff check at the phase-1 boundary: candidate
    construction must assign no `Running`, `Stopped`, or `Unknown` value, and must not construct
    a rendered `SessionEntry` merely to carry a candidate. FAIL on any status assignment,
    status-derived branch, or classifier invocation inside inventory. *OPEN CHOICE:* candidate
    representation and which raw facts/provenance it carries for phase 2.

17. **SCHEMA AND RENDERING UNTOUCHED BY THIS PHASE.** Compare the phase-1 diff to its base. FAIL
    if it changes `SCHEMA_VERSION`/`schema_version`, the JSON status domain, human rendering,
    status filters, or any consumer-facing output. A `Status::Unknown` variant already present
    from a separately owned prerequisite is not itself a failure; a new phase-1 construction or
    emission of it is.

18. **NO UNMANDATED ORDER GATE.** Candidate collection order, map/set type, server query order,
    and the carrier used for a correctly coalesced candidate's multiple provenances are *OPEN
    CHOICES* here. Semantic cardinality is fixed by criterion 20. Tests compare semantic
    identities/facts, never iteration bytes. `SC-017n` owns final rendered order; this gate must
    not reject a correct inventory for choosing a different internal order.

19. **NO AMBIENT-SELECTION POLICY GATE.** Tests inject or establish the ordinary ambient server
    before inventory runs. FAIL only if inventory ignores that resolved input or expands beyond
    it. How `AE_TMUX_SERVER` selects the ambient server belongs to `SC-1410c` and is an
    *OPEN CHOICE* outside this phase.

20. **THE UNION COALESCES EXACTLY ONE MATCH AND REFUSES AMBIGUOUS CARDINALITY.** Test A constructs
    one durable candidate with a positive server selector plus one positively owned live sighting
    from that recorded server under the exact SC-400d inventory name. Observable: exactly ONE
    semantic candidate carrying both durable and live provenance/facts. FAIL if two candidates
    remain, either provenance is lost, or equality by name alone is sufficient. Control: the same
    name on a different entitled server remains distinct. Test B constructs TWO root-distinct
    durable candidates with the same positive server and exact inventory name plus one positively
    owned live sighting. Observable: all three remain and the sighting is attached to neither
    durable record. FAIL if either durable record absorbs it or candidate cardinality drops below
    three. Representation is open; one-to-one join semantics are not.

21. **SOURCE MEMBERSHIP AND RECORD-READ LOSS ARE INDEPENDENT FACTS.** Construct (a) a tmux-only
    candidate with no durable state and (b) a durable-directory candidate whose meta read
    demonstrably fails. Observe the phase-1 carrier before classification. It must distinguish
    durable-source presence, live-source presence, and durable-record read outcome so SC-509b can
    later derive degradation without re-running discovery. FAIL if both fixtures collapse to one
    undifferentiated `no readable meta` state, if source provenance is lost, or if read failure is
    inferred from absence of live evidence. Exact enum/field representation is an *OPEN CHOICE*.

22. **THE COMPLETE UNION CROSSES THE BOUNDARY BEFORE CLASSIFICATION.** Phase 1 must expose a
    standalone inventory operation over raw durable/live discovery facts; it must not accept a
    preclassified liveness/status map, call a classifier, or materialize candidates only inside a
    classifier. A test with both durable and live-only inputs observes their complete semantic
    union at that boundary. FAIL now on reversed dependency or no standalone union seam. When
    phase 2 wires the caller, its integration test must additionally record `inventory complete`
    before first `classify enter`, with the classifier input equal to that union. Raw entitled-
    server discovery may run before or after durable discovery — SC-017j does not mandate order
    within inventory, and this criterion must not invent one.

23. **SC-405l SELECTOR NORMALIZATION IS A DISCRIMINATING MATRIX.** Unit-test the normalized
    server fact with opposed inputs: `name` + nonempty value -> positive name; `socket` +
    nonempty absolute path -> positive socket; explicit `ambiguous` -> ambiguous; kind ABSENT +
    nonempty value -> legacy positive name; both selector fields absent/empty -> missing. Test
    invalid combinations separately: unknown kind, typed empty value, relative socket,
    present-but-empty kind beside a nonempty value, duplicate equal keys, and duplicate
    conflicting keys must all be ambiguous. The absent-kind and present-empty-kind fixtures must
    differ by construction. FAIL if any candidate disappears, any invalid form becomes positive,
    name/socket payloads lose their type, or a missing/ambiguous form enters the queried-server
    set. Whether malformed forms also set `degraded` is outside this criterion (SC-405e/SC-509b).
