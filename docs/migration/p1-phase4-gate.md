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

The obligation table's current committed projection contains 1,843 obligations over
581 carrying invocation rows: 1,082 `OBSERVED` and 761 `UNSCORABLE`. These numbers are a
reconciliation control for the current contract, not hand-maintained authority. The
table's contract-blob freshness relation is authoritative: if the contract moves, phase
4 stops for re-derivation even when the old counts still happen to agree.

The only product-output exclusions are the CLOSED register
`p1-phase4-open-choices.tsv`, blob
`2da4fb86933a6b8edee15fd61596d6f53fa6c550`. Every entry names its authority,
surface, scope, exact excluded comparison locus, and the facts that remain required.
An implementation or runner cannot declare another open choice. Changing the register
changes a fixed input and requires a new gate identity and review.

The comparison relation is the fixed `p1-phase4-comparison-projection.md`, blob
`c15087aa57a4f24e4ca773df6cafb60097492454`. It defines `rc`, JSON stdout,
human list/ls stdout, opaque stdout, stderr, and no-mutation separately. The runner may
choose a parser implementation; it may not choose another projection. Changing that
file changes a fixed input and requires a new gate identity and review.

The successor also commits an **agent-health presentation manifest** before the first
phase-4 comparison. It contains one machine-decidable structural locator for SC-017r's
health cell within a roster-derived agent row and an exact nonempty token-byte mapping for
`alive`, `dead`, and `unknown`. The locator is independent of token value and neighboring
declared-state/reason text; the three token values are injective. The parity runner does
not derive or amend this manifest from captured successor output. Criterion 1 pins its
blob, and criterion 8 calibrates it before any health-token exclusion applies.

`SCOPE_KEY` is a closed, fixed-input projection:

- `digest_all`, `human_list_all`, and `json_objects_all` come only from the exact P1
  invocation/surface partition;
- `human_incomplete_observed` additionally requires an `OBSERVED` SC-017o human-diagnostic
  obligation for that invocation — successor output cannot select it;
- `human_agent_health_cells` is the field selected by the pinned agent-health presentation
  manifest within a roster-derived agent row under the fixed human-list comparison
  projection;
- `equal_name_ties` comes from fixed candidate identities before presentation.

No other scope key is valid. `digest_all` on the machine-loss row fixes where the choice
may occur; its concrete successor-only member subtrees still owe the independent
loss-projection calibration required by criterion 8.

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
- A **comparison locus** is one of exactly four facts projected by the fixed comparison
  artifact outside the obligation table — rc, stdout, stderr, or no-mutation — addressed separately so a
  match-only failure is never reported only as an undifferentiated row failure. Retained
  semantic subfields are assertions within the appropriate stdout/stderr projection, not
  runner-generated terminal loci.
- **Parity is not match.** A match obligation requires equality; a directional
  obligation requires its exact mandated result. Any other difference fails.

## Falsification criteria

1. **[STATE/BOUNDARY] ALL REFERENCES ARE FIXED BEFORE COMPARISON.** Before invoking the
   successor, run the read-only corpus, invocation, and obligation verifiers against
   committed bytes. Record the corpus root digest, contract blob, invocation-table blob,
   obligation-table blob, accepted phase-1/2/3 gate blobs, open-choice-register blob,
   comparison-projection blob, agent-health-presentation-manifest blob,
   published-projection-fingerprint artifact and its verifier/red-proof blobs specified
   by criterion 14,
   independently produced contract-to-obligation-reconciliation blob specified by
   criterion 3,
   independently produced open-choice-reconciliation blob specified by criterion 8, this
   gate blob, and successor commit in the run manifest.
   All must
   remain unchanged through the run. FAIL on corpus drift, a stale contract relation,
   an uncommitted input, a verifier that rewrites what it checks, or a result whose fixed
   identities cannot be reproduced. The obligation freshness check is HEAD-relative by
   design; an uncommitted contract edit therefore invalidates the run rather than being
   silently included. After the last successor run, re-run the read-only verifiers and
   re-assert the corpus root digest; the post-run identities are required output, not an
   assumption that the pre-run pins stayed true.

2. **[SET] THE P1 POPULATION IS EXACTLY THE RATIFIED 1,065 ROWS.** Derive the set from
   `INVOCATIONS.tsv` by requiring the phase column EQUALS `P1`, then assert both
   directions: every P1 row runs exactly once and no
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
   orphan id separately; each must make this check red. A seat that did not author the
   obligation table or its generator performs or independently repeats this
   reconciliation from its own contract reading. Before reading either seed's rc, verify
   that the mutation landed; a seed that did not land is an INVALID TEST, never a pass.

4. **[PAIR/CONTROL] THE OBLIGATION TABLE PROVES ITS OWN FRAMING.** For every obligation,
   re-read `from` from the captured stream, validate the closed stream/predicate/support
   domains, and verify its `(case, consumer)` is a P1 row. Derive the P1 universe by exact
   phase equality and prove its 1,065 keys are distinct. Derive the 581 distinct carrying
   keys from obligations, prove they are a subset of that universe, and derive the 484
   match-only keys as the complement. `INVOCATIONS.tsv` names a consumer file while the
   obligation table names its case directory, so the join uses the consumer path's
   dirname rather than comparing the two spellings directly. No stored verdict column is
   an input. The verifier has no write path. Red-prove at least: wrong `from`, wrong
   locus, wrong stream, unknown predicate, unknown support, orphan row, duplicate P1 key,
   carrying key outside P1, and a deleted obligation. Verify each seed landed before
   reading its rc; an unlanded seed is INVALID.

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
   `OBSERVED` remains 14. These figures are controls for the contract blob pinned by
   criterion 1, not authority: a fresh contract-driven re-derivation changes them rather
   than failing merely because old prose still names the prior counts.

7. **[PAIR] EVERY OBSERVED OBLIGATION MUST MOVE IN THE EXACT MANDATED DIRECTION.** For
   each `OBSERVED` locus, assert the table's `to` value with its declared predicate.
   Retaining `from` fails; producing any third value fails; merely producing different
   bytes fails. A carrying row with multiple observed obligations passes only when every
   one passes independently. Use opposed seeds for unchanged, wrong-direction, missing,
   and duplicated target values. The runner must name the failed obligation id and locus,
   not only the invocation. Verify every opposed seed landed before reading its rc; an
   unlanded seed is an INVALID TEST, never a pass.

8. **[PAIR/SET] MATCHED FACTS STAY MATCHED AND OPEN CHOICES STAY OPEN.** Every invocation
   starts with comparison loci for captured rc, stdout, stderr, and no-mutation evidence.
   Project them through the fixed comparison-projection blob and compare exactly, then
   subtract only the exact
   field/surface/scope loci named by the fixed open-choice register; an open choice never
   exempts a row or a stream merely by naming it. A registered semantic locus may account
   for every byte of a warning-only stderr, and the scalar rc may be wholly open, but all
   unregistered residue on stdout/stderr remains compared.
   On a directional row, also compare every retained contract field and semantic row not
   owned by an obligation. An expected change is not permission for unrelated drift.
   The register's `STILL_REQUIRED` facts remain asserted even where exact bytes are
   excluded. Before use, validate its exact header, unique ids, closed surfaces, closed
   machine-decidable `SCOPE_KEY` domain, and authority-plus-criterion fields resolvable
   against the exact accepted gate blobs pinned by criterion 1;
   wildcard scopes or loci are invalid. Scope applicability is decided only from fixed
   pre-successor inputs: invocation metadata, accepted contract/obligation facts, and
   predeclared controlled-arm inputs. It is never selected by successor output. In
   addition, an exclusion whose fixed-input applicability condition — its `SCOPE_KEY` or
   a selector fact inside the excluded locus — is `UNSCORABLE` or otherwise undecidable
   from those fixed inputs does not apply to that row; the underlying comparison
   projection remains exact. `STILL_REQUIRED` facts are assertions about successor output,
   not applicability conditions: a failed assertion fails the row or cross-arm control
   rather than lapsing the exclusion. For
   `OC-P2-SEMANTIC-FALSE-PRESENCE`, the frozen corpus has no `degraded` member: absence on
   both sides is a match, while successor presence is excludable only where a fixed
   pre-successor fact independently supplies semantic false. Unsupported presence fails,
   and a fixed true fact still requires `degraded: true`. In particular,
   `human_incomplete_observed` applies only where the fixed obligation table
   carries an `OBSERVED` human-diagnostic locus; an `UNSCORABLE` completeness locus buys no
   human diagnostic or rc exclusion. JSON warning policy applies to the fixed `digest_all`
   population without using the successor's completeness value as a selector. Any stderr
   span excluded under `OC-P3-JSON-WARNING` must be calibrated by a product-valid opposed
   completeness flip that holds the JSON document, process rc, and every other input fact
   fixed and proves that exact warning projection occurs only in the incomplete arm. A
   digest row whose fixed completeness locus is `OBSERVED` true must have empty stderr,
   as must the fixed-complete arm of each calibration. An implementation choosing no JSON
   warning has no span to exclude; it does not fail merely for that choice. Red-prove this
   boundary by planting unrelated stderr in a fixed-complete digest, verifying the seed
   landed, and requiring it to remain residual and fail. For every
   machine-loss subtree excluded under `OC-P3-MACHINE-LOSS-RECORDS`, enumerate its concrete
   JSON Pointer before comparison and calibrate an opposed loss-fact flip proving the
   subtree projects only those facts. Evaluate `OC-P3-GENERATED-AT`'s non-effect clause
   through every digest row's retained-fact assertions plus criterion 18's paired/reversed
   replay. Before applying `OC-P3-AGENT-HEALTH-TOKEN`, feed one otherwise-identical
   roster-derived agent row with each of the three already-supplied semantic health values
   through the real presentation boundary. Require the pinned locator to select exactly
   one minimal cell, the three captured token spans to equal the manifest's injective map,
   and every neighboring state/reason/row fact to remain identical. Red-seed a locator
   swap to the adjacent declared-state cell; verify the seed landed and require failure.
   This presentation-only calibration proves the representation and parser relation; it
   does not make any SC-017p/q liveness input product-reachable, score an SC-017r corpus
   obligation, or close the positive/negative/ambiguous liveness matrix in criteria 11/12.
   The JSON-warning and machine-loss cross-arm controls are the opposed flips specified
   above.

   A seat that did not author the register reconciles it item by item, both directions,
   against every ratified `OPEN CHOICE` occurrence in the exact accepted phase-1/2/3 gate
   blobs and every P1-applicable SC row identified by criterion 3's independent contract
   reconciliation. Each occurrence is classified as either a product-output locus with
   an exact register row or an internal/test/topology/retry choice that subtracts no
   product-output locus. This full set — including
   phase-2 criterion 13 and phase-3 criteria 10, 12, 13, and 15 — is the open-choice
   completeness set. Phase-2 criterion 13's semantic-false presence choice maps to
   `OC-P2-SEMANTIC-FALSE-PRESENCE`; its exact member-presence exclusion is applied before
   `OC-P3-JSON-FIELD-ORDER` checks the remaining member set. Criterion
   10's sort implementation, stable-sort algorithm, and filter-versus-sort sequence are
   deliberately unregistered internal choices: they subtract no product-output locus.
   Their only output-visible underdetermination is equal-name tie order, owned by
   `OC-P3-EQUAL-NAME-TIE`. The committed reconciliation
   records that `OC-P3-AGENT-HEALTH-TOKEN` is carried by the P1-applicable-SC arm through
   SC-017r, not by an occurrence in the three earlier gate blobs; that resolved SC row and
   its obligation loci make the entry supported rather than orphaned. The reconciliation
   must account for every ratified open choice and every register row. Seed one omitted
   register entry and one orphan entry separately; verify each seed landed and require
   red. An unlanded seed is INVALID, never a pass.
   FAIL on a runner-declared choice, an exclusion wider than its registered
   locus, an unregistered semantic difference, blanket byte-difference acceptance, or
   rejection of a correct implementation for a registered choice. Comparator
   implementation and how it projects a changed human row remain implementation choices;
   they subtract no product-output locus.

9. **[PAIR/STATE] SUCCESSOR-ONLY DIGEST FIELDS ARE TESTED BEFORE RETAINED CONTENT.** For
   every digest invocation, independently assert exactly one numeric `schema_version: 2`
   and exactly one boolean `inventory_complete`. Assert the completeness VALUE only when
   its locus is `OBSERVED` or a criterion-11 controlled successor arm supplies the missing
   fact; field presence and type do not make an `UNSCORABLE` value scorable. Then compare
   every retained version-1 session field. Over the complementary 664 non-digest P1
   invocations, require zero semantic top-level `schema_version` and
   `inventory_complete` fields whenever successor stdout parses as JSON. If it does not
   parse as JSON, criterion 8's projected exact stdout comparison governs; this remains a
   semantic field rule, not a substring ban. Under the fixed JSON projection, `2`, `2.0`,
   and `2e0` are the same JSON number value; criterion 9 deliberately does not invent a
   lexical integer subtype that the ratified schema never names.
   FAIL if schema version or completeness alone
   causes the whole document to be accepted as different, if either field is absent,
   duplicated, or wrongly typed, or if filtering drops or rewrites a retained field.
   Red-proof with a document carrying correct new fields and a wrong status; the comparator
   must fail on status rather than pass on schema divergence.

10. **[PAIR/SET] MULTIPLE OBLIGATIONS ON ONE INVOCATION REMAIN MULTIPLE.** Exercise an
    observed incomplete human row and its paired digest so status/membership, diagnostic,
    inventory completeness, schema version, and any supported agent-health obligation
    are checked at their own loci. Remove or corrupt each obligation one at a time while
    holding the others correct; each mutation must fail by its own id. Verify each
    mutation landed before reading its rc; an unlanded mutation is INVALID, never a pass.
    FAIL on one boolean
    `different`, first-obligation-only handling, or a row verdict that hides which of its
    obligations passed, failed, or remained unscorable.

11. **[PROVENANCE/PAIR] AN UNSCORABLE CORPUS LOCUS IS CLOSED ONLY BY A SEPARATE,
    PRODUCT-VALID SUCCESSOR ARM.** Phase 4 may make a missing server fact testable by
    injecting and recording a typed result through the same discovery/transport boundary
    the product uses. The arm records selector type/value, contacted server, transport
    success/failure, exact session name, ownership result, and the resulting locus; its
    opposed control must change the answer. Before any live capture, the arm proves in its
    own predeclared UTF-8 environment that an actual tmux query against a throwaway pane on
    a throwaway server round-trips a real TAB byte; the server under test is not the
    instrument. Every live phase-4 arm uses its own short scratch `tmux -S` socket and
    proves no other arm can contact it; the pre-existing phase-2 `-L ae-p2-<pid>` fixture
    lives in tmux's shared label namespace and does not establish this isolation. Its
    residual collision direction can only create a false failure, so it is recorded as a
    deferred fixture nit rather than evidence for or against parity. A failed or unlanded
    self-check invalidates the arm. Hand-built `World`, `Snapshot`, classified status,
    digest, or expected output does not close the corpus gap. The original corpus
    obligation remains labelled `UNSCORABLE`; the controlled arm is separate evidence,
    never retroactive observation. If no product-valid arm exists, retain the locus as
    unscorable and link the exact prior phase-gate criterion, or phase 4 does not close it.

12. **[SET/PROVENANCE] CORPUS ABSENCE IS REPORTED, NOT LAUNDERED INTO COVERAGE.** Re-run
    the sufficiency census against the current contract and preserve its full output, not
    only a hand-picked floor. A seat that authored neither the prior sufficiency census nor
    the obligation table performs or independently repeats this contract-to-corpus census
    from raw accepted artifacts. It must include at minimum the absent worktree-nested
    layout, cross-root same-leaf anti-dedup case, positive name selector, readable record
    with no selector and no record-loss fact, ambiguous selector forms, live prefix
    sibling in the firing orientation, non-ambient live server, and the unobservable
    ownership-marker cell. It also names the
    pending positive/negative/ambiguous per-agent pane matrix until a product-valid pane
    observation route closes it. The landed session transport does not by itself supply
    pane-to-agent association. It also records that the 458 human list/ls rows contain zero
    header-only views with no semantic rows: 302 have semantic rows; 86 are the single residual line
    `No running ae sessions. (try: ae list --all)`; 52 are `No recently active sessions.`;
    14 are `No running sessions need your attention.`; and four have empty stdout.
    Separately, 294 of the 458 human rows carry 1,104 agent rows, while all 78 SC-017r
    obligation loci are `UNSCORABLE`. Agent-row presence is not agent-health coverage.
    The projection's zero-row header
    calibration therefore has no corpus specimen and must not be reported as exercised.
    Each gap points to its only evidence: a pinned successor test, a controlled phase-4
    arm, or an explicit unresolved blocker. FAIL if the parity summary implies those rows
    were exercised, if zero examples are reported as a pass, or if an incidental new
    corpus specimen is ignored rather than reclassified.

13. **[BOUNDARY/CONTROL] THE FROZEN PRODUCT IS NEVER RE-RUN TO ANSWER A NEW QUESTION.**
    The baseline side consists only of accepted files named by the corpus manifest. Start
    child-execution and tmux/server instrumentation BEFORE the first pin, verifier, or
    manifest-construction step and keep it active through post-run verification; no frozen
    binary, generated frozen helper, original capture hook, or original server is invoked
    to fill an evidence gap. Calibrate the recorder with a harmless child and tmux query
    that must appear. FAIL on an uncalibrated zero, a fallback rerun after a parse failure,
    or any comparison rule that requires an artifact the frozen capture never recorded.
    A new successor-only controlled arm is permitted under criterion 11 and is never a
    frozen-side answer.

14. **[SEQUENCE/BOUNDARY] EACH SUCCESSOR RUN STARTS FROM A VERIFIED SCRATCH FIXTURE.**
    Verify corpus and case hashes first. Every invocation row must name the exact recorded
    state manifest and effective environment under which its capture ran; bind and
    fingerprint both before materialising fixture bytes outside the corpus. A multi-state
    case is bound per invocation, never to one case-level default; an unbound row fails.
    Begin each environment binding with `TZ`, `LANG`, `LC_ALL`, `TERM`, and
    `AE_TMUX_SERVER` from the case-level `env.txt`. Override only fields explicitly
    recorded by fixed state-specific evidence: for the paired C-locale specimen,
    `env-tab-selfcheck.txt` records `LANG=LC_ALL=C`, so bind both locale fields to `C`
    while retaining `TZ`, `TERM`, and `AE_TMUX_SERVER` from `env.txt`. Do not infer any
    value from a state-directory name such as `s0-baseline-clocale`. A future override
    requires a fixed artifact that names the field and value. Criterion 1 pins a
    committed published-projection-fingerprint artifact derived only from the frozen
    committed corpus. It contains exactly one row for each of the 70 published members
    under batch-c-artifacts/templates, binds each row to the corpus root and committed
    tree-ish, and records two differently purposed identities. git_tree_id is the exact
    Git tree object at that commit-ish and member path, derived without the index or
    working-tree stat; it is sensitive to the Git executable bit. canonical_sha256 is
    computed from the artifact documented byte-exact framing of every normalized relative
    tracked path, kind as file or symlink, file-content SHA-256 or exact symlink-target
    bytes; it excludes all mode bits. The artifact states this algorithm in prose.
    Missing, duplicate, unresolvable, dirty-index-selected, or unreproducible rows fail
    before materialisation. Its isolated red proof requires a non-executable chmod to
    move neither identity, an executable-bit flip to move only git_tree_id, and a content
    change, path addition/removal, or symlink retarget to move both. Before invoking the
    successor, recompute canonical_sha256 from the materialised scratch member and
    require the recorded value; independently require its tracked path/kind/content set
    and executable-bit projection to equal the committed Git tree. Record effective
    normalized argv and environment remapping; compare `TZ`, `LANG`, `LC_ALL`, `TERM`,
    and `AE_TMUX_SERVER` to that per-invocation binding, naming and justifying every
    deliberate remap without changing the recorded locale behavior.

    Before the successor capture, run the corpus's TAB oracle in that effective locale
    against a throwaway pane on a throwaway tmux server, never the case server. Require the
    raw/split result recorded for that invocation's state: UTF-8 states preserve the TAB;
    the deliberate C-locale state reproduces the recorded underscore/no-second-field
    result. A dead case server therefore cannot make the instrument check vacuous or
    impossible. After identity verification, apply the declared portable
    scratch-permission policy without changing tracked contents, paths, symlink targets,
    or executable bits, then recompute BOTH canonical_sha256 and the scratch
    executable-bit projection: canonical must match the recorded value and the exec
    projection must match the committed Git tree, because canonical_sha256 excludes every
    mode bit and cannot see a permission step that drifts an executable bit on its own.
    Enforce the source corpus as
    read-only by filesystem permission or read-only mount, never by path-prefix matching.
    Before relying on either protection, prove that a write through the successor
    execution identity and an alternate path spelling fails. Then invoke. Afterward
    compare the scratch state manifest against
    that invocation's recorded no-mutation expectation. FAIL if invocation precedes
    either published-member identity check, either post-permission identity recheck, or
    either write-protection proof, a row is bound to the wrong
    state in a multi-state case, a stale scratch tree is reused, the product sees corpus
    paths directly, one case can affect the next, the environment differs silently, the
    TAB oracle differs from the per-invocation record, or a read-side invocation mutates
    state. Scratch location and copy
    mechanism are implementation choices and subtract no comparison locus.

15. **[PAIR/CONTROL] INVOCATION NORMALISATION IS THE ACCEPTED TWO-SIDED RELATION.** Run
    the committed `verify-invocations.py` and pin its blob. Require convergence for the
    same semantic invocation under different host/run prefixes and no collision for
    different flags, flag order, session identity, or surface. Record the raw and
    normalized argv used for every successor row. Red-prove by erasing `--json` or another
    semantic token and by changing only a host prefix. Verify both seeds landed before
    reading their rc; an unlanded seed is INVALID, never a pass. FAIL if normalisation decides P1
    scope, normalises captured output, conflates `list` and `ls` provenance, or is trusted
    from a count without an independently inspected collision specimen.

16. **[SET/STATE] EVERY P1 ROW AND LOCUS GETS ONE TERMINAL ACCOUNTING STATE.** Report
    contract-obligation locus counts for PASS, FAIL, and UNSCORABLE; separately report
    comparison-locus PASS/FAIL counts. The fixed population creates exactly four
    comparison loci per row — rc, stdout, stderr, and no-mutation — hence exactly 4,260
    comparison loci over 1,065 rows. Retained semantic subfields are named in a failing
    stdout/stderr projection but do not create runner-chosen terminal loci. Reconcile the
    exact `(case, consumer, locus-kind)` set against the P1 population before deriving row
    counts for PASS, FAIL, and PARTIAL. A row is PASS only when
    all its obligations/comparable facts are scorable and
    pass; FAIL if any scorable fact fails; PARTIAL when all scorable facts pass and at
    least one obligation remains unscorable. The three row counts sum to 1,065; the
    contract-locus counts reconcile to the independently verified obligation inventory,
    and the comparison-locus counts reconcile separately to the fixed 4,260-key set. Empty,
    crashed, skipped, unsupported, or unimplemented invocations remain named rows rather
    than falling out of the denominator. The phase summary says **scorable parity passed**
    only with zero FAIL; it may say **P1 parity closed** only when every PARTIAL locus also
    has the separate product-valid evidence criterion 11 requires.

17. **[PAIR/CONTROL] THE COMPARATOR IS RED-PROVEN AGAINST BOTH FAILURE DIRECTIONS.** In
    disposable copies, verify each seed landed, then require nonzero for: one changed byte
    on a match-only row; a required divergence left unchanged; a required locus changed to
    the wrong third value; a correct schema-only change masking wrong retained content;
    one obligation deleted from a multi-obligation row; an `UNSCORABLE` locus reported as
    PASS; an extra semantic divergence outside all obligations and registered open-choice
    loci; swapped case
    outputs; a corpus write; and a normalized-argv collision. Each mutation has a named,
    local target and bounded delta. A self-test proves non-regression for imagined inputs;
    it does not establish adequacy, so an independent reviewer must add at least one
    mutation the comparator author did not design.

18. **[PAIR/STATE] REPLAY IS REPRODUCIBLE WITHOUT CROSS-CASE STATE.** Run at least one
    match-only row, one observed directional row, one mixed-support row, one controlled
    successor arm, and one digest row twice from independently materialised scratch state.
    One row may satisfy more than one category; this does not require five distinct rows.
    The digest row is the `OC-P3-GENERATED-AT` invariance control: its timestamp locus may
    differ while every retained semantic fact and every other comparison locus stays
    identical. Require
    identical captured successor bytes and identical locus results within each pair,
    except the exact loci in the fixed open-choice register. Reverse the case order
    and require the same per-case answers. FAIL on shared tmux state, clock leakage,
    scratch reuse, order-dependent obligation evaluation, or a normaliser that makes two
    distinct cases share an output.

19. **[SCOPE GUARD] PHASE 4 COMPARES; IT DOES NOT REOPEN PHASES 1–3.** Candidate
    discovery, liveness knowledge, schema domains, rendering/filtering, and product order
    are consumed from their accepted gates. Conditional predecessor evidence is rechecked
    against the pinned successor commit rather than frozen at the gate's authoring date.
    Real tmux transport landed at `fb5c6450`, invalidating the historical no-emission
    explanation but not adding a presentation obligation: the executed product consumes
    transport while building the carried snapshot before `Presentation::enter`, and
    presentation has no post-entry transport observation route. The accepted phase-3 gate
    records that boundary; an opposed live arm is optional strengthening, not a phase-4
    prerequisite. This does not promote the separate per-agent pane association route,
    which remains absent. SC-017p/q/r and SC-509e post-date those gates
    and are not silently promoted to gate-accepted facts: phase 4 consumes their exact
    contract/obligation loci, while the corpus-absent positive/negative/ambiguous agent
    liveness matrix remains pending the separate successor evidence named by criterion 12.
    Passing observed agent-health loci cannot close that pending matrix. Phase 4 may expose
    a defect and fail; it may not repair expectations to fit output, weaken an obligation,
    infer a missing product fact, or turn an earlier open choice into a required byte
    spelling. The gate itself
    FAILS if it rejects a correct implementation for a locus in the fixed open-choice
    register, or if it treats internal comparator design or scratch path as a product
    output. No unregistered phrase such as `any other open choice` expands that set.

## Required handback

The phase-4 handback pins: successor commit; this gate blob; accepted phase-1/2/3 gate
blobs; corpus root; contract, invocation, obligation, open-choice-register,
comparison-projection, agent-health-presentation-manifest, published-projection-fingerprint
artifact and its verification/red-proof reports, open-choice-reconciliation,
and verifier blobs; the independent contract reconciliation;
the per-invocation state-and-environment
bindings; effective-environment and TAB
self-check records; the agent-health presentation calibration and its swapped-locator
red proof; pre-run and post-run corpus root verification; raw terminal row/locus
accounting; every controlled-arm manifest; and the red-proof report. It states
separately:

1. scorable parity result;
2. remaining partial/unscorable loci;
3. which separate successor evidence closes each corpus gap; and
4. which contract rows remain unimplemented or empirically pending.

No single PASS token may collapse those four answers.
