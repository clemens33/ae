# T-WD design — watchdog cluster — worker draft **v6** (NOTHING APPROVED, NOTHING RUN)

Drafted by `opus5:lexec` for seat gate by `fable5:lead` and `gpt56sol:colead`. Sole-writer
draft; this file is mine and nothing else in the evidence tree is touched by it.

**v6 answers the v5 gate: BLOCKER 1–9, IMPORTANT 1–2**, delivered as a neutral delta by the lead
after colead's outcome-bearing analysis went seat-to-seat. Section 6 is the change log against
each finding.

**v5's clearance does not travel to this file.** Each version re-enters gate 1 from the beginning,
both seats, on its own fixed hash. **v5 was gated FAIL** — the lead's half-clearance of v5 is void
and is not carried forward here.

**Where a rebuild needed a product fact, it was derived from `72c7293` (or the recorded aewatch
blob) by this worker rather than requested back from a seat.** The B4 sink correction and the
B2 constructibility answer are both derivations of that kind, and both are cited.

**The sharpest finding is B1, and it is mine twice over.** M14 was introduced in v5 to abolish
positional back-references, and v5 shipped seven of them — four already stale, pointing at arms
that had moved when the roster renumbered, still parsing and still reading sensibly. A rule
asserted without a mechanism is a rule that will be broken by the commit that asserts it; gate 2
now has to prove the linter can go red on this, because being told twice is not a mechanism
either.

**Two gates, not one.** Approving the arms below is not approval to run them. Per colead:
**pre-registered scripts take a separate pre-run seat gate**, after the scripts exist and their
dependency closure is registered. Nothing executes before that second gate.

**Value-blindness.** Every RED arm names a CANDIDATE SPACE — a fixture in which two
implementations would differ — and never what the frozen implementation does. Frozen source is
read to name *sites, knobs and boundaries*; the reading is recorded as a citation (M9) and never
as an outcome. Expected relations belong in a SEAT CLASSIFICATION ANNEX which this worker does
not write and will not read.

---

## 0. Gate answers, disclosures, and the constraints they impose

**Q1 — TWELVE rows**, `D25` included. The "11" came from a row-set grep requiring an `SC-`
prefix, which silently dropped the D-record. Carried into this design as **M11**.

**Q2 — neutral surface lines supplied by the lead**, one per row, reproduced in §3.

**Q3 — `crit-assign.md` IS NOT NEUTRAL and is NOT read for T-WD.** Seven of the twelve lines
carry a bucket, a defect number and a partial SHOULD. **Standing constraint: this worker does
not open `crit-assign.md`, does not open the referenced defect issues, and does not seek the
lines it has not seen.** Colead has ruled crit-assign seat-only pending a typed projection.

**Channel discipline, now stated and binding (lead ruling).** Normative dispositions, buckets,
conflict fields and issue numbers travel **seat to seat only**; workers receive neutral surface
lines and nothing else. Until the typed projection lands mechanically, this is a channel rule.
This design records it so it is not rediscovered.

### Leak register — events, not information

Recorded as a register because more than one has now occurred. **No leaked content is
reproduced in this design, in any arm spec, or in any artifact, and none is consulted while
building or running.**

| # | when | from | rows affected | what reached this worker |
|---|---|---|---|---|
| L1 | assigning brief | `fable5:lead` | SC-920 | a disposition label and a defect pointer |
| L2 | seat ruling, item 3 | `gpt56sol:colead` | SC-920, SC-921, SC-926, SC-927, SC-928, SC-929 | row authority dispositions stating what each mechanism is held to do |
| L3 | SC-920 authority correction | `gpt56sol:colead` | SC-920 | a withdrawal and re-characterisation of L1/L2's anchor for that row |
| L4 | v4 gate, BLOCKER 3 | `gpt56sol:colead` | SC-928 | a behavioural statement about the row's subject, alongside a legitimate scope correction |

**L4, disclosed by this worker and owned by colead.** The scope half — that SC-928's assigned
empirical mechanism is aewatch's `_locked_append`, and that a bash-only arm cannot close that
surface — is a legitimate worker-facing finding and is acted on in §3A and §4.5. The behavioural
half was answer content. Colead has responded by **typing the review transport**: detailed gate
findings now go seat-to-seat, and the worker receives a neutral delta naming row, wrong
surface/reachability class, required evidence dimensions and source anchors — never an observed
relation or disposition. Recorded because the pattern matters more than the instance: **the
channel rule was in force and being followed, and the channel leaked anyway**, which is what
distinguishes a mechanism problem from a lapse.

**A limit this exposes, stated rather than papered over.** SC-928's cuts cannot be sited without
reading `_locked_append`, and reading it exposes its behaviour. For that row a value-blind
executor was never available — not because of L4, but because siting a cut requires reading the
code the cut goes in. **M12's three gates carry that row, not the executor's ignorance**, and
this is the same lesson as the unleaked seat: ignorance is a temporary state, not a mechanism.

**Consequence, stated plainly: no unleaked seat remains among the three participants.** The
Q3a remedy routed SC-920's symmetry certification to colead because they had not been leaked
to; L2 came from that seat, colead has since read issue 51 for the authority audit, and the
lead authored L1. Symmetry can therefore no longer be certified by anyone's judgment here —
which is why M12 replaces judgment with three gates checkable by a leaked reviewer.

**Recorded as doctrine, since it is the general lesson:** *a remedy that depends on someone
staying uninformed is not a remedy.* In a session whose participants exchange rulings by design,
an unleaked seat is a temporary state, not a resource.

### M12 — a three-part requirement, binding for **all six** L2 rows

Ruled by the lead, restated after v3 instantiated it once: **SC-920, SC-921, SC-926, SC-927,
SC-928 and SC-929 each carry their own concrete A/B/C matrix at execution grain** (§4). A
reference to the general mechanism is not an instantiation of it. The three parts are
**separate gates**; passing one is not passing another.

**Gate A — the synthetic pair. Exercises the CLASSIFIER. Necessary and INSUFFICIENT.**
Two synthetic fixtures, committed before any real capture, each built to drive the arm's
predicate to one of the two candidate labels. The predicate must report **both** across them.
**Labelled insufficient in the artifact itself**, because a self-fulfilling synthetic layer can
manufacture both labels while the arm remains one-sided against the product — nothing in Gate A
touches the product.

**Gate B — two PRODUCT-VALID opposed constructions plus an agreeing control. Exercises the
PRODUCT.** Both constructions must be things the product can legitimately be brought to; **the
product is never forced to manufacture a result**. This does not require the product to emit
both outcomes: it requires that the **raw capture leave both a conforming and a divergent
reading reachable to a seat**. If the fixture cannot construct the matrix, the arm is
`ARM-INVALID` rather than a one-sided capture, and the shortfall is named.

**Gate C — the REAL fixture is pre-registered too**, hash-pinned beside the two synthetics,
inside the full executable dependency closure plus the frozen tree hash (M2). Inspecting a
committed artifact is a fact-check, not a judgment.

**Why this survives universal contamination.** A leaked reviewer cannot be trusted to JUDGE
whether an arm is symmetric, but can verify that two committed fixtures produced two different
outcomes, that two product-valid constructions exist, and that a hash matches. Those are facts,
and facts do not care who reads them.

**Amendments reopen the affected arm.** Any change to a registered closure invalidates that
arm's prior gates; A, B and C are re-run before it may capture again.

**Authority note.** Six rows carry `Authority: UNRESOLVED`; per colead, IS capture proceeds and
**closes no normative field**. No artifact here is to be mistaken for closing one, and no probe
or defect report may become authority.

---

## 1. The method — the enforced mechanisms (M1–M15)

### M1 — candidate space stated, outcome absent; the lint covers the surface table
Every RED arm carries a mandatory `CANDIDATE SPACE:` field **naming candidate A and candidate B
explicitly** (§3A). A committed linter rejects the design if any RED arm lacks either candidate,
or if any arm text or **surface line** matches
`expected | should | correctly | known to succeed | resolves | does not resolve | passes |
fails | verifies that`. It reads declared fields and skips backticked spans.

**No exemption.** All surface rows are linted; `exempt_rows` must be `0`, and a non-zero value
is a lint failure. A zero-row run is `ARM-INVALID` for the lint, not a pass.

The linter is a belt. **The structural guarantee is the seat-owned typed projection**, not
vocabulary matching, and the linter's header says so — it cannot tell a mention from a use.

**Lint status at v4: NO LINTER FIGURE IS QUOTED, AND NONE IS INDEPENDENTLY CHECKABLE YET.** The
linter is not in the tree at gate 1 — it lands with the scripts. v3 quoted a figure this worker
had measured; with the arm blocks rewritten that figure is stale, and a stale number beside a
fresh document is exactly the failure this programme keeps closing, so it is withdrawn rather
than restated. **What HAS been run is a worker-side `grep` self-check** over this document,
which found and fixed two arms whose candidate space was a back-reference rather than a named
pair, and reworded two sentences that tripped the vocabulary list as mentions rather than uses.
**A grep by the author is not the linter and is not evidence** — it is stated so the record shows
what was and was not done.

**And the self-check instrument was itself defective, which is the point of not calling it
evidence.** The `grep` this worker ran against v5 omitted `fails` from the term list for several
rounds — a term M1 itself declares. It reported clean while five instances stood, and a shorter
list always reports cleaner, so the bias ran toward "nothing to fix". The omission was found by
re-deriving the pattern from M1's own sentence instead of retyping it from memory, which is the
same defect class as a citation written from recall. **A checker whose term list is transcribed
rather than derived can only under-report**, so the committed linter reads its terms from the M1
declaration rather than carrying its own copy. **The figure becomes checkable at gate 2**, when the linter is
committed and a seat runs it.

### M2 — pre-registration of the full executable dependency closure, in a stopped prepare phase
Script-hash identity is insufficient: an unchanged script can source a changed harness, hook
patch, shim, fixture builder or generated helper. **`PREREGISTRATION.tsv` registers the whole
closure** — arm script; every library it sources; the instrumented binary and its hook patch;
every shim; every fixture builder; every Gate-A synthetic; the frozen `ae` blob hash **and the
frozen tree hash**; **and the generated-helper hashes**.

**Scope is PROVENANCE-BOUNDED, not total (lead ruling, v4 gate BLOCKER 1).** v3's closure was
too narrow (script identity only); v4's was too broad (everything hashed and frozen), which
would have failed arms for the product doing the very thing they exist to observe — arm 5 resumes
a session, and a resume rewrites session assets; arms 27–29 invoke `doctor --refresh`, whose
defining action is `sync_session_assets` (ae:8610). Neither error is visible from inside the
other. The boundary the rule was missing is provenance:

| provenance | treatment | rationale |
|---|---|---|
| the **harness** supplied it — arm scripts, libraries, hook patches, shims, fixture builders, generators, the frozen `ae` blob, the instrumented copies, the harness tree hash | **PINNED**, recomputed, `ARM-INVALID` on mismatch | this is the instrument's identity |
| the **product** wrote or regenerated it during the arm | **CAPTURED** as an observation | a helper that changed because `doctor --refresh` regenerated it **is the measurement**, not drift |
| **both** — the harness planted it and the product then rewrote it | **two-artifact record**: planted hash at prepare time, rewritten hash as a result, both recorded | a hash change here is a finding, not a failure |

`PREREGISTRATION.tsv` carries a `provenance` column (`harness` \| `product` \| `both`) and the
runner dispatches on it. **Every product write an arm expects is registered by exact path with
its pre-state before the run**; anything outside that registered set that the product writes is
captured and flagged as unregistered, and anything harness-owned that changes is `ARM-INVALID`.
Freezing is the default; the allowed product writes are the enumerated exception.

**Phasing (colead v3 IMPORTANT 2).** Generated-helper hashes captured at run time cannot belong
to a pre-run closure, so fixture preparation is a **separate, stopped phase**:

| phase | what happens | what it produces |
|---|---|---|
| **P-PREPARE** | every sandbox is built, every session launched, every generated helper emitted — **then everything is stopped**. No arm runs. | the fixture trees, at rest |
| **P-REGISTER** | the closure is hashed over the stopped trees, `PREREGISTRATION.tsv` written and **committed** | the registered closure |
| **P-GATE2** | seats review the committed scripts and closure | clearance |
| **P-CAPTURE** | arms run against the already-prepared fixtures | the evidence |

**The capture runner refuses to prepare — but only for harness-owned paths.** In `P-CAPTURE` it
recomputes the closure and fails `ARM-INVALID` if any **harness-provenance** entry differs from
its registered hash, or if any **harness** fixture-builder entry point is reached. A
**product-provenance** path that changes is recorded as a result. A product write to a path that
was never registered for that arm is captured **and** flagged, because an unexpected product
write is itself evidence. Changes to harness entries require an `AMENDMENTS.md` entry and a new
registered closure.

### M3 — a neutral/mutated pair PER ARM, one line each
Every RED arm ships a **neutral** leg (must report `caught=NO`) and a **mutated** leg
(`caught=YES`), one row each in the generated calibration table. An arm whose neutral leg has
never reported `NO` is `ARM-INVALID`.

### M4 — set comparison, never containment
Full sets, sorted and complete, with the symmetric difference emitted. A lint rejects
`grep -q`, substring tests and `grep -c` used as assertions inside arm scripts.

### M5 — the injection is verified to have landed, and the generator's rc inspected
Every mutation is followed by a landing check that re-reads the artifact and compares as a set
(M4), plus explicit capture of the generator's rc and stderr. An arm whose tool crashed reports
`ARM-INVALID`.

### M6 — the verifier never repairs
No regenerate-then-compare mode. A lint rejects any write to a path the same script later reads
as evidence.

### M7 — derivation, never transcription
Anything shown is generated from its source document by dropping fields structurally, with the
generator committed.

### M8 — each leg calibrated separately
Every leg gets its own mutation firing that leg and no other; the calibration table records
per-leg `caught`. Overlapping mutations are forbidden.

### M9 — every citation pinned, and its limit stated
Every `ae:NNNN` resolved against `git show 72c7293:ae` by a committed pinner emitting **the line
text beside the claim**. `CITATIONS.tsv` carries (claim, file, line, line text, sha256).
**A pin proves resolution, not aptness** — only a seat reading the source catches a confident
citation on a wrong-but-plausible line. Every site named in §3A and §4 is a pinned citation.

### M10 — typed fields, validated headers
Every artifact a table reads is a TSV with a declared header the generator validates first.

### M11 — the row set is checked against an authoritative opposite side
v2's generator read ids out of **this design** and asserted a hand count — which agrees
perfectly with the wrong set and has no opposite side. **Corrected: exact id AND batch sets are
compared BOTH DIRECTIONS against the seat-owned value-blind projection filtered to T-WD, with
the symmetric difference emitted. The count is output, never the oracle.** The projection does
not exist yet, so **M11 is `PENDING-DEPENDENCY` and no arm runs until it lands.**

### M12 — symmetry as a mechanism
As §0, instantiated per row in §4, for all six L2 rows.

*(M13 canaries, M14 named blocks and M15 the lane-unit contract follow below. The heading names
the range rather than a spelled-out total: a hand-maintained count is a transcription, and this
document has already shipped one prose count that disagreed with its own roster.)*

### M14 — shared blocks are machine-addressable, never prose inheritance *(colead v4 BLOCKER 6)*
v4 claimed every RED arm was a standalone execution-grain spec and it was not: nine arms
delegated fixture, barriers, captures or calibration to a sibling by prose ("as arm 22"), which
gate 2 cannot expand and cannot prove closure over. **Corrected: every shared element is a named
block in `SHARED-BLOCKS.tsv`** (`block_id`, `kind`, `owner_arm`, `body`), and an arm references
it by id in a typed field. **The checker expands every referenced block into every referencing
arm and validates the expanded arm**, so per-arm closure and calibration are proven rather than
inherited. An arm carrying a bare prose back-reference is a lint failure.

**References are by NAME, never by ORDINAL.** v5 claimed this and reproduced the defect at seven
sites, four of which had already gone **stale** when the roster renumbered — still parsing, still
reading sensibly, pointing at the wrong arm. **Every cross-reference is a stable arm id (`WD-…`)
or a block id**, and the linter rejects the token pattern `arms? <digits>` anywhere in an arm
field or in §4. Ordinals survive only in the roster table, which is generated from the headings
(M7), and in the execution order, which is about sequence rather than identity.

**Gate 2 must RED-PROVE the rejection.** The linter is run against a deliberately seeded
positional reference and must report it. A run that cannot go red proves nothing, and this rule
has now been asserted twice with no mechanism behind it.

**One manipulation per arm is enforced, not merely stated.** v4's arm 13 carried three failure
sub-constructions and three manipulations against its own one-manipulation rule, with a single
generic calibration covering all three. It is split into three arms (§3A). The linter counts
declared manipulations per arm and rejects any arm declaring more than one.

### M15 — the lane-unit contract *(lead ruling on the v6 judgment call, binding)*
Where an arm runs under a pinned selector that is itself the controlled variable, **the unit of
closure is `(arm_id, lane)`**, and three conditions bind — stated here as a general rule because
v6 carried them only as incidental phrasing inside two row sections, which is not the same as
having them:

1. **The checker expands, validates and COUNTS per pair.** No number anywhere in this design, in
   any generated table, or in any artifact header may state a spec count where a unit count is
   meant. v6 had exactly that defect in its execution order, which named 37 in a context that
   means 47.
2. **The lane is visible per unit and NO AGGREGATE READING IS AVAILABLE.** Every artifact
   directory, every captured TSV row, every calibration row and every canary record carries its
   `lane` field. **The generators emit no cross-lane total, union or merged set** — a number that
   spans lanes is a lint failure, because an aggregate silently re-creates the confound the
   pinning exists to remove.
3. **Calibration legs and M13 canaries are per unit and NEVER shared.** A canary registered in one
   lane says nothing about the other; a neutral leg that reported `NO` in one lane does not
   satisfy M3 for the other.

**Why this is a matrix rather than the inheritance shape twice rejected** (the lead's test, kept
because it settles the question rather than the instance): the distinguishing test is
**attribution** — can a result be attributed to exactly one manipulation? A lane pair is one
manipulation under a pinned, recorded selector with nothing shared to confound it, so yes. v4's
arm 13 bundled three manipulations, so no. **The test is the rule; the count of ids is not.** And
sixteen hand-maintained near-copies would be worse: duplication invites drift, someone edits one
and not the other, and nothing flags it — the same disease as the 62 retargeted references, with
more surface.

### M13 — equipment canaries are CONTROLLER-generated, never product outcomes *(colead v3 IMPORTANT 3)*
v3's recorder-liveness controls were phrased as "a known daemon start", "a known delivery", "a
known failure", "a known version change" — every one of them a **product outcome**, so a
recorder that only ever registered when the product acted could not be distinguished from a
recorder that works. **Replaced: each recorder is proven by a datum the CONTROLLER generates
and pushes through the EXACT capture primitive the arm will use, with no product involvement,
immediately before the measurement and separately recorded.**

| recorder | canary the controller generates | primitive it must come back through |
|---|---|---|
| process census | a `sleep` the controller starts under a distinctive argv nonce it owns | the same census command the arm uses |
| pane-bytes | a nonce the controller writes to a **scratch pane it owns**, never a fixture pane | the same capture-pane call, same scrub, same range |
| rc/stdout/stderr | `sh -c 'printf NONCE; printf ERRNONCE >&2; exit 7'` | the same capture wrapper, asserting rc 7 and both streams |
| file bytes / inode | a scratch file the controller creates and appends to | the same stat/hash/inode primitive |
| invocation trace | a controller-issued call to the shim's delegate target under a nonce argv | the same shim log |

The canary is `ARM-INVALID` on failure, is captured to its own artifact, and **is never
mixed into the product measurement**. A canary that requires the product to do anything is a
defect in the canary.

---

## 2. Arm classes, typed

| class | must carry | must not carry |
|---|---|---|
| **RED** | `CANDIDATE SPACE` with A and B named, neutral + mutated legs (M3), per-leg mutations (M8), landing check (M5), M13 canary | — |
| **CAPTURE-ONLY** | provenance of the invocation, M13 canary, exact bytes with `od`, sha256, and an explicit in-artifact statement that it makes no comparison | no candidate space, no legs, no assertion of any kind |

The linter and the calibration generator both dispatch on `class`, so a CAPTURE-ONLY arm is
excluded structurally and a RED arm cannot silently become one.

---

## 3. Rows, neutral surfaces, and the arm roster

Surface lines are the lead's verbatim except SC-980, rewritten on colead's instruction.

| row | neutral surface line | family |
|---|---|---|
| D25 | the watchdog daemon process itself, and which implementation serves under the mode split | F9 |
| SC-834a | the `_recover-pending` internal helper, and the watchdog's invocation of it | F10 |
| SC-900 | the `events.jsonl` container's lifecycle — growth, rotation, and behaviour across resume | F11 |
| SC-901 | how many daemon processes exist per `AE_HOME` and what each one owns | F12 |
| SC-913 | the mechanism by which a daemon nudge is delivered, and what is verified about the path | F6 |
| SC-920 | how quiet stabilization treats pane evidence according to its ORIGIN | F4 |
| SC-921 | whether internal monitor panes participate in agent roster and branch logic | F7 |
| SC-926 | the control surface's success reporting relative to durable intent and runtime state | F1 |
| SC-927 | the status surface's read/write behaviour, and where cleanup is performed | F1 |
| SC-928 | what becomes of an event-append error | F11 |
| SC-929 | the state observable after a restart, and `doctor --refresh` serving-version ordering | F9 |
| SC-980 | the incumbent alert's action and summary byte surface *(adapted per colead)* | F13 |

**37 arm specs** — 35 RED, 1 CAPTURE-ONLY, 1 declared gap. 10 run in **two pinned lanes**
(bash and uv), so the executed unit count is 47. Every arm gets its own disposable sandbox and its own
fixture clone; no arm shares a sandbox. 18 arms (rows SC-920/921/926/927/928/929) carry M12 and take their A/B/C matrix
from §4; the two bash baselines under SC-928 close nothing and say so in their own headers.

| # | arm id | row | class | lanes | M12 |
|---|---|---|---|---|---|
| 1 | `WD-D25-serve-at-start` | D25 | RED | — | — |
| 2 | `WD-D25-serve-after-flip` | D25 | RED | — | — |
| 3 | `WD-834A-pending-at-start` | SC-834a | RED | — | — |
| 4 | `WD-834A-pending-midrun` | SC-834a | RED | — | — |
| 5 | `WD-900-resume-overlimit` | SC-900 | RED | — | — |
| 6 | `WD-900-run-overlimit` | SC-900 | RED | — | — |
| 7 | `WD-901-two-sessions` | SC-901 | RED | bash+uv | — |
| 8 | `WD-901-second-start` | SC-901 | RED | bash+uv | — |
| 9 | `WD-913-lock-contention` | SC-913 | RED | bash+uv | — |
| 10 | `WD-913-occupied-input` | SC-913 | RED | bash+uv | — |
| 11 | `WD-913-dead-pane` | SC-913 | RED | bash+uv | — |
| 12 | `WD-913-submit-unverified` | SC-913 | RED | bash+uv | — |
| 13 | `WD-913-durable-occupied` | SC-913 | RED | bash+uv | — |
| 14 | `WD-913-durable-dead` | SC-913 | RED | bash+uv | — |
| 15 | `WD-913-durable-unverified` | SC-913 | RED | bash+uv | — |
| 16 | `WD-913-delivery-count` | SC-913 | RED | bash+uv | — |
| 17 | `WD-920-origin-matrix` | SC-920 | RED | — | §4.1 |
| 18 | `WD-921-monitor-in-roster` | SC-921 | RED | — | §4.2 |
| 19 | `WD-921-monitor-stamped` | SC-921 | RED | — | §4.2 |
| 20 | `WD-926-start-cut-pre-intent` | SC-926 | RED | — | §4.3 |
| 21 | `WD-926-start-cut-post-intent` | SC-926 | RED | — | §4.3 |
| 22 | `WD-926-stop-cut-pre-intent` | SC-926 | RED | — | §4.3 |
| 23 | `WD-926-stop-cut-post-intent` | SC-926 | RED | — | §4.3 |
| 24 | `WD-927-residue-dead-pid` | SC-927 | RED | — | §4.4 |
| 25 | `WD-927-residue-empty-pidfile` | SC-927 | RED | — | §4.4 |
| 26 | `WD-927-residue-recycled-pid` | SC-927 | RED | — | §4.4 |
| 27 | `WD-928A-lock-timeout` | SC-928 | RED | — | §4.5 |
| 28 | `WD-928A-open-fault` | SC-928 | RED | — | §4.5 |
| 29 | `WD-928A-write-fault` | SC-928 | RED | — | §4.5 |
| 30 | `WD-928A-unlock-fault` | SC-928 | **GAP — does not run** | — | — |
| 31 | `WD-928B-bash-writer-baseline` | SC-928 | RED | — | baseline |
| 32 | `WD-928B-bash-lock-baseline` | SC-928 | RED | — | baseline |
| 33 | `WD-929-refresh-running` | SC-929 | RED | — | §4.6 |
| 34 | `WD-929-refresh-not-running` | SC-929 | RED | — | §4.6 |
| 35 | `WD-929-refresh-fails` | SC-929 | RED | — | §4.6 |
| 36 | `WD-929-restart-state` | SC-929 | RED | — | §4.6 |
| 37 | `WD-980-alert-bytes` | SC-980 | **CAPTURE-ONLY** | — | — |

**Uniform invalid / inconclusive conditions**, enforced by the runner for every arm:
`ARM-INVALID` when — a **harness-provenance** closure entry mismatches, or a **harness-owned**
regenerating entry point is reached (M2 — a *product* regeneration is the measurement, not a
failure; v5's uniform wording here reinstated the collision the lead had already ruled on); the M13 canary does not come back through the capture primitive; the landing check does not confirm
or the generator rc is non-zero (M5); the neutral leg does not report `NO` (M3); a leg's
mutation fires another leg (M8); an M12 arm has not shown both outcomes (M12 Gate A) or cannot
construct its matrix (Gate B). `INCONCLUSIVE` when — a bounded wait expires; the bound and the
state at expiry are recorded and no absence is inferred.

**Execution order.** (i) M11's projection check; (ii) `P-PREPARE` → `P-REGISTER` → `P-GATE2`;
(iii) per-arm M13 canaries; (iv) M12 Gate A + Gate B reachability for **every arm whose typed
`m12` field is set** — derived from the typed rows by the checker, never from a prose count, which
is how v5 said fifteen while its own roster said eighteen;
(v) **all 47 executed units** — every `(arm_id, lane)` pair — in id order, lane-minor;
(vi) generated tables. Any failure at (i)–(iv) stops the units it
gates.

**Named cut sites and barriers used below**, each a pinned citation (M9) against
`git show 72c7293:ae` (or against the recorded aewatch blob for the `CUT-928A-*` family). A cut is
a controller-driven signal at a hook-emitted barrier: the hook blocks and announces, the
CONTROLLER acts, per the cluster-plan admissibility rule.

**Every cut declares its exact action (colead v4 IMPORTANT 3).** "The controller signals the
invocation" is under-specified — `SIGTERM`, `SIGKILL` and simply releasing the barrier produce
three different worlds, and an arm that does not say which is not reproducible. Each cut therefore
carries a typed `cut_action` field: the **signal or action** (`SIGKILL` \| `SIGTERM` \| `release`
\| a named filesystem or permission change), the **exact target** (pid and argv of the signalled
process, or the path acted on), and **whether the barrier is subsequently released**. The
controller records the tuple it actually performed, and the arm captures the target's identity at
the moment of the action — so the artifact says what was done to what, not that something was
done. The `CUT-926-*` family uses `SIGKILL` on the pid of the control invocation itself, barrier
not released, because a cut that lets the process continue is not a cut; deviations are recorded
per arm.

| id | site | frozen anchor |
|---|---|---|
| `CUT-926-START-RUNTIME` | after the watchdog pane split returns, before the durable intent write | `_watchdog_start`, ae:15089–15101 |
| `CUT-926-START-INTENT` | after `_set_meta_watchdog "true"` returns, before the success line | `_watchdog_start`, ae:15102–15103 |
| `CUT-926-STOP-RUNTIME` | after the pid kill / pidfile removal / pane kill, before the durable intent write | `_watchdog_stop`, ae:15044–15051 |
| `CUT-926-STOP-INTENT` | after `_set_meta_watchdog "false"` returns, before `exit 0` | `_watchdog_stop`, ae:15060 |
| `CUT-928-APPEND` | with the flock already held, immediately before the append writer | `ae_log_append`, ae:13175 |
| `CUT-928-LOCK` | at the lock acquisition itself, before the writer is reached | `ae_log_append`, ae:13174 |
| `CUT-928A-LOCK` | the bounded lock-acquisition loop, at its timeout return | aewatch `_locked_append`, aewatch:2372–2376 |
| `CUT-928A-OPEN` | opening the target file for append, lock already held | aewatch `_locked_append`, aewatch:2379 |
| `CUT-928A-WRITE` | the write on the already-open descriptor | aewatch `_locked_append`, aewatch:2380 |
| `CUT-928A-UNLOCK` | the unlock in the call's cleanup path | aewatch `_locked_append`, aewatch:2382 |
| `BAR-929-PUB` | the rename that publishes the regenerated **watchdog helper** — `watchdog.tmp.$$` + `chmod 0700` + `mv`. **NOT `_publish_executable_artifact` (ae:833)**: session helpers are exempt from that chokepoint by shape, so a hook there never fires for this artifact | ae:18007–18009 |
| `BAR-929-SERVE` | the first daemon cycle boundary after `BAR-929-PUB` | watchdog `_run` main loop, ae:16019 |
| `BAR-929-RESTART` | the product's own stop/start pair, when the arm invokes it | `_watchdog_stop`/`_watchdog_start`, ae:15031/15073 |
| `BAR-929-PRERETURN` | immediately before `doctor --refresh` returns to its caller | `doctor_refresh_sessions`, ae:8913–8917 |
| `BAR-920-SEND` | the daemon's nudge-send invocation, delegating unchanged | watchdog `_run`, ae:16477–16480 |
| `BAR-QS-ARM` | quiet-stabilization entry for a pane | `_quiet_stabilize`, ae:15451 |

Each barrier that is new at v4 (`CUT-926-*`, `CUT-928-*`, `BAR-929-*`, `BAR-920-SEND`) needs its
own hook-patch version, hash triple, and per-fixture inactive-equivalence proof with a working
known-difference control **before a single hooked capture** (§5).

---

## 3A. Per-arm execution blocks

Every RED arm below states candidate A and candidate B, the exact fixture facts, **one** named
manipulation, its barriers, its raw captures, its neutral/mutated calibration legs, and its
arm-specific invalid condition. The uniform conditions above apply to all of them and are not
repeated.

### D25 — which implementation serves under the mode split

#### 1. `WD-D25-serve-at-start` — RED
- **CANDIDATE SPACE** — **A:** the selector decides which implementation serves **and** the
  non-selected implementation's prior state is reaped or ignored, so exactly one
  implementation's process exists afterwards. **B:** the selector decides only which
  implementation is *started*, so a prior opposite-implementation state survives alongside it. Distinguishable because the arm captures the process
  census **and** both implementations' artifact sets as full sets (M4).
- **Fixture facts** — block `FX-D25-PRIOR`. **The prior opposite state is PRODUCT-CREATED, not
  planted** (colead v4 BLOCKER 5): the `AE_HOME` is first driven through a complete launch under
  the *other* implementation and stopped by the product's own path, leaving whatever artifacts
  that implementation genuinely leaves. Only then is the selector set for the leg under test and
  a launch performed. `contrib/aewatch` and a `uv` runtime are present in both legs, so
  availability is not what differs. **A fresh `AE_HOME` cannot reach candidate B at all** — v4's
  fixture made this arm unable to produce the unwanted answer, which is `ARM-INVALID` by this
  design's own rule.
- **Named manipulation** — `AE_WATCHDOG_IMPL=uv` is exported (or not) in the launching
  environment, once, before the second launch.
- **Barriers** — none.
- **Raw captures** — full process census with argv; `.watchdog.pid` bytes; the `ae-aewatch` tmux
  session list; `@ae_agent` pane stamps; `watchdog=` meta key; the aewatch heartbeat file's
  existence and mtime — **and, because presence alone cannot separate "a process survives" from
  "that process serves THIS session" (colead v5 BLOCKER 7), per-implementation CYCLE AND DECISION
  evidence KEYED TO THE NEW SESSION**: for each implementation, log lines or emitted rows naming
  this session's id and showing a cycle executing against its roster, plus each per-agent decision
  that cycle produced. Pid, heartbeat and pane existence are explicitly **not** sufficient and are
  captured as context, never as the discriminator.
- **Calibration** — neutral: census taken with no session launched at all (`caught=NO`).
  mutated: a controller-planted decoy process under a watchdog-shaped argv that the census must
  report (`caught=YES`).
- **ARM-INVALID** — if `uv` or `contrib/aewatch` is absent in either leg, since the switch would
  then not be the only difference; the shortfall is named and no capture is taken.

#### 2. `WD-D25-serve-after-flip` — RED
- **CANDIDATE SPACE** — **A:** the selector is consulted only on the launch/resume path, so a
  flip followed by a *control* invocation changes nothing while a flip followed by a *resume*
  does. **B:** it is consulted more widely. Distinguishable because the arm exercises **both**
  call shapes against the same flip and captures the census after each.
- **Fixture facts** — block `FX-D25-PRIOR`, launched in the non-`uv` leg with a running daemon;
  `AE_WATCHDOG_INTERVAL_SEC` pinned low so a cycle boundary is reached inside the bound.
  **The exact product call that re-enters the selector is named** (colead v4 BLOCKER 5): it is
  read in `_start_session_watchdog` (ae:10457–10466), which runs on the launch/resume path
  (ae:18224). `cmd_watchdog` executes the generated bash helper and never consults it, so v4's
  flip-then-`watchdog start` construction could not have reached the selector at all.
- **Named manipulation** — `AE_WATCHDOG_IMPL` is flipped once. **How the flipped value reaches
  the resume is stated as a mechanism, not as "shared environment"** (colead v5 BLOCKER 7): the
  controller writes an env-file recording the exact variable set, and each subsequent invocation
  is launched by `env -i` from that file, whose bytes are captured and hashed per invocation. The
  artifact shows the environment each call actually received rather than asserting that two calls
  shared one.
- **Barriers** — none.
- **Raw captures** — census + argv at four points: before the flip; after a control invocation
  (`watchdog stop` then `start`); after a cycle boundary; after a **resume**. Both
  implementations' artifact sets; serving pid and start time at each point; which call shape
  preceded each.
- **Calibration** — neutral: three censuses with no flip (`caught=NO`). mutated: a controller
  `sleep` started between points 2 and 3 under a nonce argv, which the census must newly report
  (`caught=YES`).
- **ARM-INVALID** — if no cycle boundary is observed within the bound, the arm is `INCONCLUSIVE`
  with the bound and state recorded, never an absence.

### SC-834a — `_recover-pending` and the watchdog's invocation of it

#### 3. `WD-834A-pending-at-start` — RED
- **CANDIDATE SPACE** — **A:** the helper is invoked by the daemon's own cycle, so the
  invocation trace shows the daemon as the caller. **B:** it is invoked only by a control-plane
  path (`doctor --refresh` calls `doctor_recover_pending_sessions`, ae:8820, called at ae:8887 / 8899 / 8913), so a running
  daemon alone produces no invocation. Distinguishable because the arm runs the daemon **without
  any doctor invocation** and captures caller pid/ppid/argv.
- **Fixture facts** — block `FX-834A-PENDING`. **The pending object is an agent session-id slot,
  not a tracked request** (colead v4 BLOCKER 2): `walk_pending_session_ids` iterates `agent.<slot>`
  keys in session meta and skips every entry whose stored id is not literally `pending`
  (ae:8745–8790); the watchdog invokes the helper at ae:16528–16536. **A tracked `ask` whose reply
  never arrives cannot make this surface run at all**, so v4's `WD-834A-*` arms would have captured an
  empty trace that read like a finding. The fixture is a **producer-valid** pending slot: a
  session launched with a post-launch-capture tool (codex / gemini / opencode) whose capture
  genuinely did not complete, leaving `agent.<slot>=alias:name:pending` as the product wrote it,
  **plus** a matching tool-session candidate file in that tool's own session directory.
- **Named manipulation** — the tool-session candidate is placed so the slot becomes resolvable,
  once. **Any byte the controller plants rather than the producer writing it is declared as
  planted in the artifact**, with its diff, rather than presented as producer output.
- **Barriers** — a delegate-and-log shim on the `_recover-pending` path recording pid, ppid,
  argv and stamp, delegating unchanged.
- **Raw captures** — invocation trace (may be empty; emptiness is recorded as a fact, never as an
  absence verdict); events delta as a full set; daemon log bytes; **and the objects this row
  actually turns on** (colead v5 BLOCKER 6): the exact `agent.<slot>` meta row before and after,
  byte for byte; the resolved `stored_alias`, `tool_kind` and the `agents.<alias>` config command
  the walk reads (ae:8757–8776); the `launch_id` and `launch_time` inputs as the product holds
  them; the tool-session candidate bytes **with provenance** — produced-by-tool or
  planted-by-controller, declared per byte range; and the post-recovery row. v5 captured a tracked
  request row, which is not an object this surface reads.
- **Calibration** — neutral: same fixture with the pending item absent (`caught=NO`). mutated:
  the controller invokes the shim's delegate target directly under a nonce argv, which the trace
  must record (`caught=YES`) — an M13 canary, not a product outcome.
- **ARM-INVALID** — if the bound expires before N cycles complete, `INCONCLUSIVE`.

#### 4. `WD-834A-pending-midrun` — RED
- **CANDIDATE SPACE** — **A:** pending items are discovered per cycle, so one appearing mid-run
  is picked up without a restart. **B:** the pending set is read once at daemon start.
  Distinguishable because the item is planted **after** the daemon has completed at least one
  clean cycle, and the trace is captured across N further cycles.
- **Fixture facts** — block `FX-834A-PENDING`, but the daemon is running and has completed at
  least one clean cycle before the slot reaches its pending state; cycle boundaries observed at
  `BAR-QS-ARM` or from the daemon log.
- **Named manipulation** — the matching tool-session candidate appears once, mid-run, with the
  planted-versus-produced declaration `INV-834A` requires.
- **Precondition versus manipulation, separated (colead v5 BLOCKER 6).** The pending slot must
  already exist **before** the clean cycle — it is part of `FX-834A-PENDING`, established during
  `P-PREPARE` — and this arm's single manipulation is the candidate's mid-run appearance and
  nothing else. v5 stated a precondition its sole manipulation did not create, which cannot
  satisfy one-manipulation attribution: either the slot pre-exists (this arm), or creating it is a
  second named product operation and therefore a different arm.
- **Barriers** — block `BAR-834A`.
- **Raw captures** — block `CAP-834A`, plus the cycle index at plant time and at each trace entry.
- **Calibration** — block `CAL-834A`.
- **ARM-INVALID** — if no clean cycle precedes the plant, the arm is invalid rather than
  re-timed.

### SC-900 — the `events.jsonl` container across growth and resume

#### 5. `WD-900-resume-overlimit` — RED
- **CANDIDATE SPACE** — **A:** the container is trimmed in place, so the inode is preserved and
  the byte count falls. **B:** it is replaced, so the inode changes. A third reading — no
  change at all — is left reachable by capturing both facts rather than asserting either.
- **Fixture facts** — one session; the log grown past the retention bound **by the product**
  (real emitted events, never hand-written lines); the retention knob lowered by its documented
  environment variable; the session then stopped and resumed.
- **Named manipulation** — the retention knob is lowered once, before the resume.
- **Barriers** — none.
- **Raw captures** — inode, byte count, line count, first and last line bytes, and the full
  container set in the meta dir, each taken before the resume and after it; open file handles on
  the container at both points; **and a REAL READER held across the boundary** (colead v4
  IMPORTANT 2): a controller-owned process opens the container **before** the resume, holds the
  descriptor across it, and **reads afterwards** — its post-resume bytes, cursor offset and the
  `st_dev`/`st_ino` of the still-open descriptor are captured as their own facts. A handle
  snapshot alone cannot separate a reader that follows a replacement from one stranded on the old
  generation, which is the distinction this arm's candidate space turns on.
- **Calibration** — neutral: stop/resume with the knob left at its default and the log below the
  bound (`caught=NO`). mutated: the controller appends a nonce line to a **scratch** file and the
  same inode/byte primitive must report the change (`caught=YES`).
- **ARM-INVALID** — if the product did not itself grow the log past the bound.

#### 6. `WD-900-run-overlimit` — RED
- **CANDIDATE SPACE** — block `CS-900`, with the distinguishing axis being **when**: **A:** the bound
  is enforced on any crossing, so an ordinary run behaves as a resume does. **B:** enforcement is
  bound to the resume path specifically, so an ordinary run crossing the same bound differs.
- **Fixture facts** — block `FX-900`, but the bound is crossed **during an ordinary run with no
  stop and no resume**; identical knob value, identical growth mechanism.
- **Named manipulation** — the same knob lowered once, before the growth, with no resume.
- **Barriers** — none.
- **Raw captures** — block `CAP-900`, sampled at the same three cycle offsets.
- **Calibration** — block `CAL-900`.
- **ARM-INVALID** — if a resume occurs at any point, since the arm's whole axis is its absence.

### SC-901 — daemon count per `AE_HOME` and what each owns

#### 7. `WD-901-two-sessions` — RED
- **CANDIDATE SPACE** — **A:** daemons are per-session, so two sessions in one `AE_HOME` yield
  two processes with disjoint ownership. **B:** they are per-`AE_HOME`, so one process serves
  both. Distinguishable because the arm captures the census and each daemon's ownership records
  as full sets.
- **Fixture facts** — one `AE_HOME`; two real launches with distinct session names, neither a
  prefix of the other (the `#102` topology lesson); both with the watchdog enabled. **The
  implementation is PINNED and the lanes are separate** (colead v5 BLOCKER 5): `AE_WATCHDOG_IMPL`
  is set explicitly — unset for the bash lane, `uv` for the aewatch lane — and each lane runs in
  its **own sandbox**. An unpinned selector means one capture cannot attribute a topology to an
  implementation, since either could have produced it. **Every process record and every ownership
  record is tagged with the pinned lane** in the captured TSV, so attribution is a field rather
  than an inference. **The unit of closure here is `(arm_id, lane)` as well**, with its own
  sandbox and calibration per pair.
- **Named manipulation** — the second session is launched; nothing else changes.
- **Barriers** — none.
- **Raw captures** — census with argv and ppid; per-session `.watchdog.pid` bytes; each pidfile's
  pid mapped to its argv; `@ae_agent` stamps per session; both `watchdog=` meta keys.
- **Calibration** — neutral: census with one session launched (`caught=NO`). mutated: an M13
  controller `sleep` under a nonce argv the census must report (`caught=YES`).
- **ARM-INVALID** — if either launch returns non-zero; if the two sessions share a name prefix;
  if `AE_WATCHDOG_IMPL` is not explicitly pinned for the lane; if the `uv` lane cannot be brought
  up (missing runtime or `contrib/aewatch`), which is named as a shortfall rather than silently
  falling back to bash.

#### 8. `WD-901-second-start` — RED
- **CANDIDATE SPACE** — **A:** a second start for a session already served is refused or
  collapses to the incumbent, leaving one process. **B:** it produces a second process.
  Distinguishable because the census is a full set before and after and the pidfile bytes are
  captured at both points.
- **Fixture facts** — block `FX-901` but with a single session already served by a running daemon.
- **Named manipulation** — the product's own `watchdog start` is invoked a second time.
- **Barriers** — none.
- **Raw captures** — census before/after; pidfile bytes before/after; rc and stdout of the second
  start; any reap trace; the `_watchdog` pane set.
- **Calibration** — block `CAL-901` (`caught=NO` / `caught=YES`).
- **ARM-INVALID** — if the incumbent daemon is not confirmed running before the second start.

### SC-913 — the nudge delivery mechanism and what is verified about the path

The neutral surface names **six independent dimensions**, and a two-cell arm cannot fail four of
them, so each dimension gets a cell that can independently produce the unwanted answer. The cells
share the **fixture builder** and nothing else: each runs in its own sandbox and takes its own
captures, so a defect in one construction cannot propagate into another's reading.

**BOTH delivery lanes are exercised (colead v5 BLOCKER 4).** v5's cells drove only the generated
`send` helper and the bash daemon path. Every dimension below is therefore run **twice** — once in
the bash lane (`AE_WATCHDOG_IMPL` unset) and once in the **aewatch lane** (`AE_WATCHDOG_IMPL=uv`)
— in separate sandboxes, with the aewatch lane additionally capturing its **invocation trace**:
what the daemon invoked, with which argv, from which pid. **The unit of closure is `(arm_id, lane)`, not `arm_id`** — each pair gets its own sandbox, its
own calibration legs, its own M13 canaries and its own artifact directory, and the checker
expands, validates and counts per pair. Stated explicitly rather than left as one spec silently
covering two executions, which is the defect shape this gate has rejected twice. **Flagged for the
seats as a judgment call:** the alternative is sixteen arm ids for eight dimensions. Per-pair
closure is equivalent and avoids sixteen near-identical blocks differing only in a selector value
— if a seat prefers distinct ids, say so and they will be split.

**THE DURABLE SINK NAMED IN v5 IS NOT REACHABLE FROM THE DAEMON NUDGE PATH.**
`undelivered.launch-<slot>.txt` is written on the **launch** delivery path (ae:12689–12691); the
nudge path invokes the session's `send` helper, whose refusal, abandonment and unconfirmed
branches all exit before any store is written. Capturing that file therefore cannot evidence
durable nudge failure. **Corrected: the arms capture the full CANDIDATE DURABLE-STORE SET
reachable from each nudge path**, as a set, with each member's presence, bytes and mtime:
`events.jsonl` (the daemon's own alert emission at ae:16495 sits on the failure side, unlike the
send helper's); the message-body store `${META_DIR}/messages/` (`ae_store_message_body`, one
artifact per delivery); the watchdog's own daemon log; the tmux `display-message` surface
(ae:16496), captured as the transient it is; and in the uv lane, aewatch's own recorder log,
heartbeat and backoff files. **`undelivered.launch-<slot>.txt` is retained as an explicitly
OUT-OF-SCOPE control** — captured, labelled not-reachable-from-this-path in its own artifact
header, and excluded from every set comparison.

#### 9. `WD-913-lock-contention` — RED
- **CANDIDATE SPACE** — **A:** the target lock (`ae_lock_target`, ae:14235) excludes concurrent
  writers to one pane, so two deliveries are serialised and their pasted bytes are whole. **B:**
  exclusion is absent or advisory, so the two interleave. Distinguishable because both deliveries
  carry distinct nonce payloads and the pane bytes are captured as a full sequence.
- **Fixture facts** — one launched session; target is the unmodelled `grok`-shaped fake pane
  rendering every received line verbatim; two senders, each issuing a nonce payload through the
  session's own generated `send` helper.
- **Named manipulation** — the two sends are issued concurrently at a controller-held barrier
  released once.
- **Barriers** — a controller release barrier only; no hook.
- **Raw captures** — pane bytes with `od`; per-sender rc, stdout, stderr; lock file state and
  acquisition timestamps from each sender's own trace; event rows as a full set.
- **Calibration** — neutral: the two sends issued sequentially, second after the first returns
  (`caught=NO`). mutated: the controller writes an interleaving nonce to its **own scratch pane**
  and the capture primitive must show the interleave (`caught=YES`, M13).
- **ARM-INVALID** — if the two sends do not overlap in wall-clock, the arm reports
  `INCONCLUSIVE` with both intervals recorded.

#### 10. `WD-913-occupied-input` — RED
- **CANDIDATE SPACE** — **A:** an occupied input region defers delivery within a bound and then
  aborts loudly with a non-zero rc, leaving the occupying text intact. **B:** delivery proceeds
  and the occupying text is altered. Distinguishable because the occupying text is a nonce and is
  captured byte-for-byte before and after.
- **Fixture facts** — the fake pane holds staged, unsubmitted nonce text; `AE_SEND_DEFER_SEC`
  pinned to a low documented value so the bound is reached inside the arm's own bound.
- **Named manipulation** — the nonce text is staged in the target's input region once.
- **Barriers** — none.
- **Raw captures** — rc, stdout, stderr of the send; pane bytes with `od` before and after; the
  staged text's bytes at both points; elapsed time against the configured bound; event rows as a
  full set.
- **Calibration** — neutral: the same send against the same pane with nothing staged
  (`caught=NO`). mutated: the controller stages a nonce in its own scratch pane and the capture
  primitive must report it (`caught=YES`, M13).
- **ARM-INVALID** — if the staged text cannot be confirmed present immediately before the send.

#### 11. `WD-913-dead-pane` — RED
- **CANDIDATE SPACE** — **A:** a pane whose agent process is gone is refused before anything is
  pasted, so the pane's byte sequence is unchanged and no shell command is executed. **B:** the
  paste proceeds into the shell. Distinguishable because the payload is a nonce that would leave
  an observable trace if executed as a command, and the arm captures both the pane bytes and the
  filesystem effect that trace would produce.
- **Fixture facts** — the fake agent is exited so its pane drops to a shell, confirmed via
  `pane_current_command` before the send; the payload is a nonce whose execution would create a
  uniquely-named file in a controller-owned scratch directory.
- **Named manipulation** — the fake agent process is exited once.
- **Barriers** — none.
- **Raw captures** — rc, stdout, stderr; pane bytes with `od`; `pane_current_command` before and
  after; presence or absence of the scratch file as a raw fact; event rows as a full set.
- **Calibration** — neutral: the same send against the same pane with the fake still running
  (`caught=NO`). mutated: the controller executes the nonce payload itself in its own scratch
  shell, and the scratch-file primitive must report the effect (`caught=YES`, M13).
- **ARM-INVALID** — if the pane is not confirmed a shell before the send.

#### 12. `WD-913-submit-unverified` — RED
- **CANDIDATE SPACE** — **A:** submission is verified, so a paste that never submits is reported
  as unconfirmed with a non-zero rc. **B:** submission is assumed, so the same state is reported
  as delivered. Distinguishable because the fake is run in a mode where it renders received text
  but never echoes a submitted line.
- **Fixture facts** — the fake's own documented non-echo mode (a property of the harness fake,
  never a change to the product); target otherwise identical to `WD-913-lock-contention`'s.
- **Named manipulation** — the fake is started in non-echo mode, once, at fixture build.
- **Barriers** — none.
- **Raw captures** — rc, stdout, stderr; pane bytes with `od`; paste-buffer state; event rows as
  a full set; the stored message body's presence as a raw fact.
- **Calibration** — neutral: the same send against the fake in ordinary echo mode (`caught=NO`).
  mutated: the controller drives the same verification primitive against its own scratch pane
  with a known-absent marker (`caught=YES`, M13).
- **ARM-INVALID** — if the fake's non-echo mode cannot be confirmed by a controller probe before
  the send.

#### 13. `WD-913-durable-occupied` — RED
- **CANDIDATE SPACE** — **A:** a delivery abandoned on an occupied target leaves **no**
  success-shaped record and **does** leave a durable error record at a named sink. **B:** it
  leaves one, both, or neither. **The two are separate facts and are captured separately**
  (colead v4 IMPORTANT 1): `D1` no-success-record — the event set and body-store set as full sets
  before and after, with symmetric difference; `D2` durable-error-record — the presence, bytes and
  mode of the named sink.
- **The durable sink is NAMED** — `${META_DIR}/undelivered.launch-<slot>.txt`, written mode 600
  at ae:12689–12691 with a `launch-delivery-failed` event at ae:12692, and the sibling
  `spawn-failed` event at ae:12071–12072. Whether the *nudge/send* path writes to any sink at all
  is precisely what candidate A and B differ on, so the arm captures the sink's state rather than
  assuming its use.
- **Fixture facts** — block `FX-913-OCCUPIED`, in its **own sandbox**. Shares the fixture builder
  with `WD-913-occupied-input` and reads none of its artifacts.
- **Named manipulation** — nonce text is staged in the target's input region, once (exactly one).
- **Barriers** — none.
- **Raw captures** — `D1` and `D2` as separately-headed artifacts; rc and stderr; **persistence
  after caller and pane loss**: the sink and event set are re-read after the calling process is
  terminated and after the target pane is killed, each as its own fact with the loss event
  timestamped.
- **Calibration** — neutral: the same attempt allowed to land normally (`caught=NO`). mutated:
  the controller appends a nonce row to a **scratch** event file and writes a nonce sink file in a
  scratch directory; the set-difference and sink primitives must report both (`caught=YES`, M13).
- **ARM-INVALID** — if the occupied class is not reached; if the sink path is not registered in
  `PREREGISTRATION.tsv` with `provenance=product` and its pre-state recorded (M2).

#### 14. `WD-913-durable-dead` — RED
- **CANDIDATE SPACE** — block `CS-913`'s `D1`/`D2` pair, over the **refused-on-dead-pane** class. The
  classes are separate arms because a sink written on one failure class and not another is the
  distinction the surface names, and one arm covering three classes could not attribute a sink to
  a class.
- **Fixture facts** — block `FX-913-DEAD`, own sandbox.
- **Named manipulation** — the fake agent process is exited, once (exactly one).
- **Barriers** — none.
- **Raw captures** — block `CAP-913`, including persistence after caller and pane loss.
- **Calibration** — block `CAL-913`.
- **ARM-INVALID** — block `INV-913`, with the dead class substituted.

#### 15. `WD-913-durable-unverified` — RED
- **CANDIDATE SPACE** — block `CS-913`'s `D1`/`D2` pair, over the **submit-unverified** class.
- **Fixture facts** — block `FX-913-NOECHO`, own sandbox.
- **Named manipulation** — the fake is started in non-echo mode, once (exactly one).
- **Barriers** — none.
- **Raw captures** — block `CAP-913`, including persistence after caller and pane loss.
- **Calibration** — block `CAL-913`.
- **ARM-INVALID** — block `INV-913`, with the unverified class substituted.

#### 16. `WD-913-delivery-count` — RED
- **CANDIDATE SPACE** — **A:** the daemon's counter counts **deliveries**, so attempts that did
  not land do not advance it. **B:** it counts **attempts**, so they do. Distinguishable because
  the arm runs a known number of non-landing attempts followed by a landing one and captures the
  counter's rendered value and the alert set at every cycle.
- **Fixture facts** — a real daemon; a target constructed to refuse (`WD-913-dead-pane`'s class);
  `AE_WATCHDOG_MAX_NUDGES`, `AE_WATCHDOG_UNDELIVERED_MAX`, `AE_WATCHDOG_STALE_MIN` and
  `AE_WATCHDOG_INTERVAL_SEC` pinned to documented low values so the bounds are reached inside
  the arm's bound.
- **Named manipulation** — the target is put into the refusing class once.
- **Barriers** — none.
- **Raw captures** — daemon log bytes per cycle; the rendered counter and streak at each cycle;
  event set as a full set; the alert set with each alert's action and target; cycle indices.
- **Calibration** — neutral: the same daemon against a target that accepts every nudge
  (`caught=NO`). mutated: an M13 canary — the controller emits a nonce line through the same log
  primitive, which the per-cycle capture must report (`caught=YES`).
- **ARM-INVALID** — if the configured bounds are not reached within the arm's wait, which is
  `INCONCLUSIVE` with the bound and the state at expiry recorded.

### SC-920 — quiet stabilization and the ORIGIN of pane evidence

#### 17. `WD-920-origin-matrix` — RED, **M12 §4.1**
- **CANDIDATE SPACE** — **A:** quiet stabilization reads pane evidence without regard to which
  process produced it, so two byte-identical pane states of different origin are treated alike.
  **B:** origin participates, so they are not. Distinguishable **only** if two specimens are
  byte-identical at differing origin — which is why §4.1 makes byte-identity a hard admission
  requirement rather than a preference.
- **Fixture facts, barriers, captures and the full matrix** — §4.1.
- **Calibration** — neutral: the matrix run with S1 and S2 at the **same** origin (`caught=NO`).
  mutated: the controller writes a nonce to its own scratch pane and the same capture-pane
  primitive, scrub and range must return it (`caught=YES`, M13).
- **ARM-INVALID** — as §4.1, including the byte-identity requirement.

### SC-921 — monitor panes in roster and branch logic

#### 18. `WD-921-monitor-in-roster` — RED, **M12 §4.2**
- **CANDIDATE SPACE** — **A:** internal monitor panes are excluded from the agent roster, so the
  roster set and health denominator count only real agent panes. **B:** they participate, so the
  denominator includes them. Distinguishable because the roster set, the per-branch verdict map
  and the denominator are captured as **full sets** (M4), not as counts.
- **Fixture facts** — a launched session whose only non-agent panes are the product-created
  `_events` and `_watchdog` panes; one real spawned agent pane in the opposed construction
  (§4.2).
- **Named manipulation** — one real agent is spawned through the product's own `spawn` helper.
- **Barriers** — `BAR-QS-ARM` to bound the cycle in which the capture is taken.
- **Raw captures** — roster set with each member's pane id, `@ae_agent` stamp and
  `pane_current_command`; per-branch verdict map; health denominator; daemon log bytes for the
  cycle.
- **Calibration** — neutral: the same capture with the watchdog stopped and the monitor window
  absent (`caught=NO`). mutated: the controller creates a scratch pane carrying a nonce
  `@ae_agent` stamp **in a session no arm measures**, which the roster primitive must report
  (`caught=YES`, M13).
- **ARM-INVALID** — as §4.2. Note the L-DISCRIM lesson: a planted pane inside a measured session
  pollutes the roster it is meant to observe, so the canary pane lives outside it and any planted
  window is killed before the measurement cycle.

#### 19. `WD-921-monitor-stamped` — RED, **M12 §4.2**
- **CANDIDATE SPACE** — **A:** participation is decided by the pane's `@ae_agent` stamp alone, so
  adding or removing a stamp moves a pane in or out of the roster. **B:** it is decided by
  something else (pane provenance, window, or process), so the stamp does not move it.
  Distinguishable because the stamp is the single thing manipulated and the roster is a full set
  before and after.
- **Fixture facts** — block `FX-921`, with the monitor panes present and the daemon running.
- **Named manipulation** — one monitor pane's `@ae_agent` stamp is added or removed, once, by the
  controller, with the byte diff of the pane-option set recorded.
- **Barriers** — `BAR-QS-ARM`.
- **Raw captures** — block `CAP-921`, before and after the stamp change, with the symmetric difference
  of the roster sets emitted.
- **Calibration** — neutral: the same before/after capture with no stamp change (`caught=NO`).
  mutated: the mutated leg of block `CAL-921` (`caught=YES`, M13).
- **ARM-INVALID** — as §4.2, plus: if the stamp change cannot be confirmed landed by re-reading
  the pane-option set (M5).

### SC-926 — the control surface's success reporting vs durable intent and runtime state

**v3's SC-926 pair varied stale / orphaned / recycled pidfiles** — which exercises residue and
liveness, not the boundary the neutral surface names. Those fixtures move to SC-927 (arms
22–24), and SC-926 is rebuilt on the **durable-intent write boundary**: in both `_watchdog_start`
(ae:15073–15103) and `_watchdog_stop` (ae:15031–15061) the runtime mutation and the durable
intent write (`_set_meta_watchdog`, ae:14961) are **separate steps in a fixed order**, so a cut
between them and a cut after them are different states of the world. Four arms: {start, stop} ×
{pre-intent cut, post-intent cut}.

#### 20. `WD-926-start-cut-pre-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — **A:** the durable intent and the runtime state are written as one
  effective unit, so a cut before the intent write leaves neither. **B:** they are independent,
  so a cut there leaves a running runtime with no durable intent. Distinguishable because the
  arm captures the runtime facts and the durable key **separately** and states no relation.
- **Fixture facts** — a prepared session with the watchdog **not** running and `watchdog=` at its
  pre-start value; the instrumented copy carrying only the `CUT-926-START-RUNTIME` hook.
- **Named manipulation** — the controller signals the control invocation at
  `CUT-926-START-RUNTIME`; the hook only blocks and announces.
- **Barriers** — `CUT-926-START-RUNTIME`.
- **Raw captures** — rc and stdout/stderr of the control invocation (may be truncated by the cut;
  captured as bytes either way); `watchdog=` meta bytes; `.watchdog.pid` bytes; process census
  with argv; the `_watchdog` pane set; meta file mtime and mode.
- **Calibration** — neutral: the same invocation with the hook **inactive** (`caught=NO`, and the
  inactive-equivalence proof for this fixture is a precondition). mutated: the controller writes
  a nonce to a scratch meta file and the same key-read primitive must report it (`caught=YES`,
  M13).
- **ARM-INVALID** — if the inactive-equivalence proof for this hook/fixture pair has not passed
  with a working known-difference control; if the cut cannot be confirmed to have landed at the
  named site.

#### 21. `WD-926-start-cut-post-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — block `CS-926`, at the opposite side of the same boundary: **A:** the success
  report is emitted only after the durable intent write, so a cut after it leaves intent written
  and no report. **B:** the report precedes the durable write. Distinguishable because rc, the
  emitted bytes and the durable key are three separate captures.
- **Fixture facts** — block `FX-926`; instrumented copy carrying only the `CUT-926-START-INTENT` hook.
- **Named manipulation** — the controller signals at `CUT-926-START-INTENT`.
- **Barriers** — `CUT-926-START-INTENT`.
- **Raw captures** — block `CAP-926`.
- **Calibration** — block `CAL-926`.
- **ARM-INVALID** — block `INV-926`.

#### 22. `WD-926-stop-cut-pre-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — **A:** stopping writes durable intent and mutates runtime as one
  effective unit, so a cut before the intent write leaves both untouched or both done. **B:** the
  runtime mutation (kill, pidfile removal, pane kill) completes and the durable key does not
  follow. Distinguishable because runtime facts and the durable key are captured separately.
- **Fixture facts** — a prepared session with the watchdog **running**, confirmed by census and
  pidfile before the arm; instrumented copy carrying only `CUT-926-STOP-RUNTIME`.
- **Named manipulation** — the controller signals at `CUT-926-STOP-RUNTIME`.
- **Barriers** — `CUT-926-STOP-RUNTIME`.
- **Raw captures** — block `CAP-926`, plus: the killed pid's liveness, the `_watchdog` pane's presence,
  and the tmux user options the stop path clears, each as its own fact.
- **Calibration** — block `CAL-926`.
- **ARM-INVALID** — block `INV-926`, plus: if the daemon is not confirmed running before the cut.

#### 23. `WD-926-stop-cut-post-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — block `CS-926` at the opposite side: **A:** the durable key and the reported
  outcome agree once the intent write has returned. **B:** they can disagree at that point.
- **Fixture facts** — block `FX-926`; instrumented copy carrying only `CUT-926-STOP-INTENT`.
- **Named manipulation** — the controller signals at `CUT-926-STOP-INTENT`.
- **Barriers** — `CUT-926-STOP-INTENT`.
- **Raw captures** — block `CAP-926`.
- **Calibration** — block `CAL-926`.
- **ARM-INVALID** — block `INV-926`.

### SC-927 — the status surface's read/write behaviour and where cleanup is performed

The three pid-residue fixtures live here, per colead. Each is its own arm with its own sandbox.

#### 24. `WD-927-residue-dead-pid` — RED, **M12 §4.4**
- **CANDIDATE SPACE** — **A:** the status surface is a pure read, so the meta directory's file
  set and bytes are identical before and after. **B:** it performs cleanup, so the residue is
  removed by the read. Distinguishable because the arm captures a full manifest (path, size,
  inode, mode, sha256) before and after and emits the symmetric difference.
- **Fixture facts** — a prepared session with no daemon running; `.watchdog.pid` containing a pid
  that is confirmed dead at fixture-build time and re-confirmed immediately before the read.
- **Named manipulation** — the pidfile is written with the dead pid, once, and the byte diff
  recorded.
- **Barriers** — none.
- **Raw captures** — full meta manifest before and after with symmetric difference; rc and stdout
  of the status invocation; process census; the `_watchdog` pane set.
- **Calibration** — neutral: the same manifest taken twice with no status invocation between
  (`caught=NO`). mutated: the controller removes a nonce file from a scratch directory and the
  same manifest primitive must report the difference (`caught=YES`, M13).
- **ARM-INVALID** — if the pid is not confirmed dead immediately before the read; if a real
  daemon is running in the sandbox.

#### 25. `WD-927-residue-empty-pidfile` — RED, **M12 §4.4**
- **CANDIDATE SPACE** — **A:** cleanup is keyed on the recorded pid's liveness, so an **empty**
  pidfile — which names no pid at all — is a distinct case from a dead pid and the meta file set
  is unchanged by the read. **B:** cleanup is keyed on the pidfile being unusable for any reason,
  so emptiness and a dead pid are handled alike. Distinguishable because this arm and
  `WD-927-residue-dead-pid` differ in exactly this one property and in nothing else; they are separate arms because one arm
  covering both classes could not tell the two mechanisms apart.
- **Fixture facts** — block `FX-927` but `.watchdog.pid` is zero bytes, confirmed by size and hash.
- **Named manipulation** — the pidfile is truncated to zero bytes, once, byte diff recorded.
- **Barriers** — none.
- **Raw captures** — block `CAP-927`.
- **Calibration** — block `CAL-927`.
- **ARM-INVALID** — block `INV-927`.

#### 26. `WD-927-residue-recycled-pid` — RED, **M12 §4.4**
- **CANDIDATE SPACE** — **A:** liveness of the recorded pid is sufficient, so a pidfile naming
  **any** live process leaves the meta file set unchanged by the read. **B:** liveness is not
  sufficient and the daemon's own stamped pane is also required, so a live but unrelated pid is
  treated as residue. Distinguishable because the named process is alive and demonstrably not
  this session's daemon, and no `_watchdog`-stamped pane exists in the session.
- **Fixture facts** — block `FX-927`, but `.watchdog.pid` names a controller-owned `sleep` under a
  nonce argv, confirmed alive immediately before the read, and **no `_watchdog`-stamped pane
  exists** in the session.
- **Named manipulation** — the pidfile is written with the live non-daemon pid, once, byte diff
  recorded.
- **Barriers** — none.
- **Raw captures** — block `CAP-927`, plus the nonce process's liveness before and after the read as
  its own fact.
- **Calibration** — block `CAL-927`.
- **ARM-INVALID** — if the nonce process is not confirmed alive immediately before the read, or
  if a `_watchdog`-stamped pane exists in the session.

### SC-928 — what becomes of an event-append error

v3 made the whole events **container** unwritable and relied on one refusal canary. That cannot
separate the dimensions the surface names, and an unwritable container also breaks the lock file
alongside the writer, so the fault was not bound to the append at all. **Rebuilt: the fault is
writer-bound, at a named cut, against a named implementation.**

**The implementation under test is named: the bash per-session watchdog** (`_watchdog _run`,
emitted into `${META_DIR}/watchdog`), with `AE_WATCHDOG_IMPL` unset so the bash implementation
is the one serving (ae:10462). The aewatch sidecar is out of scope for these two arms and the
exclusion is recorded rather than left implicit.

**Four raw facts are captured separately** in both arms, never merged into one verdict:
`F1` the emitting process's identity and liveness (pid, ppid, argv, start time, alive/gone);
`F2` whether a **subsequent unrelated cycle** occurs (a cycle whose work is untouched by the
fault, identified by cycle index in the daemon log);
`F3` the state of the **operation** that was emitting (its rc, its own output, and its durable
effect if it has one);
`F4` the **nudge state** (counter, undelivered streak, alert set) before and after.

#### 27. `WD-928A-lock-timeout` — RED, **M12 §4.5**
- **CANDIDATE SPACE** — **A:** a bounded lock-acquisition timeout is a contained outcome at the
  emit boundary, so the emitting daemon is unaffected past that call. **B:** it is not contained.
  `F1`–`F4` are captured separately so both readings stay reachable.
- **Fixture facts** — block `FX-928A-DAEMON`: an aewatch daemon serving under
  `AE_WATCHDOG_IMPL=uv`, with the instrumented aewatch copy carrying **only** the
  `CUT-928A-LOCK` hook.
- **Named manipulation** — a controller-owned process holds `events.jsonl.lock` for longer than
  the call's bound, once; the holder's pid, argv and hold interval are recorded and it is
  released after the capture.
- **Barriers** — `CUT-928A-LOCK`.
- **Raw captures** — `F1`–`F4` as four separately-headed artifacts; the events file bytes and
  hash; the daemon log across at least two cycle indices; the aewatch heartbeat and backoff state
  files as raw bytes.
- **Calibration** — neutral: the same barrier with no competing holder (`caught=NO`). mutated:
  the controller holds a **scratch** lock and the same acquisition primitive must report the
  timeout (`caught=YES`, M13).
- **ARM-INVALID** — inactive equivalence unproven for this hook/fixture pair; `F2` unreachable
  inside the bound, which is `INCONCLUSIVE` for `F2` alone.

#### 28. `WD-928A-open-fault` — RED, **M12 §4.5**
- **CANDIDATE SPACE** — block `CS-928A`, over the **open-for-append** error class rather than the lock
  class. Separate arms because the two classes sit on opposite sides of the lock acquisition and
  a single arm could not attribute an outcome to one of them.
- **Fixture facts** — block `FX-928A-DAEMON`; instrumented copy carrying only `CUT-928A-OPEN`.
- **Named manipulation** — with the lock **already held by the daemon**, the controller makes the
  target events file alone un-openable for append, once, leaving the lock file and the containing
  directory intact; mode diff recorded and reverted after the capture.
- **Barriers** — `CUT-928A-OPEN`.
- **Raw captures** — block `CAP-928A`.
- **Calibration** — block `CAL-928A`, against a scratch file.
- **ARM-INVALID** — block `INV-928A`, plus: if the lock file or directory is not intact at fault time,
  which would unbind the fault from the open.

#### 29. `WD-928A-write-fault` — RED, **M12 §4.5**
- **CANDIDATE SPACE** — block `CS-928A`, over the **write** error class, with the descriptor already
  open. Separate from `WD-928A-open-fault` because open-time and write-time errors are different
  classes.
- **Fixture facts** — block `FX-928A-DAEMON`; instrumented copy carrying only `CUT-928A-WRITE`.
- **Named manipulation** — **ENOSPC on a dedicated filesystem.** The session's events directory
  sits on its own small filesystem the controller creates during `P-PREPARE`; at the barrier the
  controller fills it to capacity with one named filler file, once, removing it after the capture.
  This is a controller-performed, product-valid state (a full filesystem is a real condition), it
  needs no hook beyond blocking and announcing, and it requires neither closing a descriptor nor
  manufacturing an `errno` — **neither of which a hook is permitted to do**, and both of which are
  why `chmod` and rename cannot serve here: once a process holds an open descriptor, permission
  and name changes cannot make *that descriptor's* write err.
- **Constructibility caveats, at gate 1 rather than deferred to the artifact.** (i) The writer is
  a buffered Python text handle, so `ENOSPC` may surface at the implicit flush/close inside the
  same block rather than at the `write` call; the arm captures **where it surfaced** as a fact and
  does not assume the call site. (ii) The lock file is on the same filesystem but consumes no
  bytes at acquisition, so the fault stays bound to the append; the arm asserts the lock was held
  before the filler landed. (iii) If a dedicated filesystem cannot be created on the host, the arm
  is `ARM-INVALID` with the shortfall named — **not** substituted with a whole-container
  permission change, which is the unbound fault v4 already had.
- **Barriers** — `CUT-928A-WRITE`.
- **Raw captures** — block `CAP-928A`.
- **Calibration** — block `CAL-928A`.
- **ARM-INVALID** — block `INV-928A`, plus: if the descriptor is not confirmed open at fault time.

#### 30. `WD-928A-unlock-fault` — **ARM-INVALID AT GATE 1 — DECLARED GAP, NOT A CAPTURE**
- **This arm does not run, and the reason is recorded rather than worked around.** The unlock
  class would be the cleanup-path counterpart to the write class, but **no controller-performed,
  product-valid mechanism exists to make `LOCK_UN` on a live, valid descriptor err** on either
  supported platform. Every alternative is forbidden: a hook may block or announce, it may not
  close descriptors or manufacture an `errno`, and nothing another process can do to the
  filesystem reaches a held lock's release.
- **Recorded as a declared gap** in `ARM-GAPS.tsv` (`arm_id`, `row`, `class`, `reason`,
  `what_would_be_needed`), so the row's coverage shortfall is a visible fact at ratification
  rather than an absence nobody notices. **A declared gap is evidence; a quietly dropped one is
  not.**
- **What would lift it** — a hook contract permitting fault injection *inside* the subject
  process, which this cluster's admissibility rule deliberately forbids. Lifting it is a seat
  decision about that rule, not a fixture problem.

#### 31. `WD-928B-bash-writer-baseline` — RED, **BASELINE — closes nothing**
- **CANDIDATE SPACE** — **A:** the bash appender's writer-error handling is contained at the emit
  site. **B:** it is not. **This arm is a BASELINE and closes no part of SC-928** (colead v4
  BLOCKER 3): the row's assigned empirical mechanism is aewatch's, so bash evidence is a
  comparison point and is labelled as such in its own artifact header.
- **Fixture facts** — block `FX-928B-BASH`: the bash per-session watchdog, `AE_WATCHDOG_IMPL`
  unset; instrumented `ae` copy carrying only `CUT-928-APPEND`, which blocks **with the flock
  already held** so the fault cannot be attributed to lock acquisition.
- **Named manipulation** — the controller makes the target events file alone unappendable, once,
  leaving `events.jsonl.lock` and the directory writable; mode diff recorded and reverted.
- **Barriers** — `CUT-928-APPEND` (frozen `ae_log_append`, ae:13175).
- **Raw captures** — `F1`–`F4`; events file bytes and hash; daemon log across two cycle indices.
- **Calibration** — neutral: the barrier reached with the manipulation not applied (`caught=NO`).
  mutated: a scratch file made unappendable, reported by the same write primitive (`caught=YES`).
- **ARM-INVALID** — block `INV-928B`; **plus** the artifact header must state that this arm closes no
  part of the row, and a missing statement is an invalid artifact.

#### 32. `WD-928B-bash-lock-baseline` — RED, **BASELINE — closes nothing**
- **CANDIDATE SPACE** — block `CS-928B`, over the bash lock-acquisition class (`CUT-928-LOCK`, frozen
  `ae_log_append`, ae:13174). Baseline; closes nothing.
- **Fixture facts** — block `FX-928B-BASH`; instrumented copy carrying only `CUT-928-LOCK`.
- **Named manipulation** — a controller-owned process holds the lock past the wait bound, once.
- **Barriers** — `CUT-928-LOCK`.
- **Raw captures** — block `CAP-928B`, plus the holder's identity and hold interval.
- **Calibration** — block `CAL-928B`.
- **ARM-INVALID** — block `INV-928B`.

**Barriers are declared per arm as REQUIRED or EXCLUDED-BY-CONSTRUCTION** (colead v5 BLOCKER 3).
v5 required all four on every `WD-929-*` arm; two cannot occur under the stopped-watchdog
construction, and the failure construction can prevent more. **An arm may not require a barrier
its own construction excludes.** A non-occurrence is captured as a bounded fact — barrier not
observed within N seconds, state at expiry recorded — never as a required barrier and never read
as an absence. The typed fields are `barriers_required` and `barriers_excluded`, and **gate 2
rejects any arm whose sets overlap or whose required set names a barrier its construction
excludes.**

| arm | required | excluded by construction | bounded non-occurrence |
|---|---|---|---|
| `WD-929-refresh-running` | all four | — | — |
| `WD-929-refresh-not-running` | `BAR-929-PUB`, `BAR-929-PRERETURN` | `BAR-929-RESTART`, `BAR-929-SERVE` — no running daemon to restart or to serve a next cycle | both, with bound and state at expiry |
| `WD-929-refresh-fails` | `BAR-929-PUB`, `BAR-929-PRERETURN` | — | `BAR-929-RESTART`, `BAR-929-SERVE`, whose occurrence depends on how far the interrupted refresh proceeds |
| `WD-929-restart-state` | `BAR-929-RESTART`, `BAR-929-SERVE` | `BAR-929-PUB` — no publication is invoked | one, with its bound |

#### 33. `WD-929-refresh-running` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** the process serving after a refresh of a **running** watchdog is a
  different process from the one serving before it. **B:** it is the same process. Distinguishable
  because process identity and helper identity are captured independently at all four barriers.
- **Four facts are kept SEPARATE** (colead v4 BLOCKER 4), never merged into one verdict:
  `G1` refresh success/failure (rc and the report lines as bytes); `G2` serving version (the
  serving process's pid and start time versus the published helper's inode, mtime and sha256);
  `G3` whether a restart occurred (pid change, `_watchdog` pane id change, pidfile bytes);
  `G4` durable state (`watchdog=` meta bytes).
- **Fixture facts** — block `FX-929-RUNNING`: a running bash watchdog whose liveness satisfies the
  frozen gate at ae:8627–8649 — a pidfile, a `kill -0`-live pid, **and** a live
  `@ae_agent=_watchdog` pane — since that gate is what selects the restart path.
  `AE_WATCHDOG_INTERVAL_SEC` pinned low so `BAR-929-SERVE` is reachable inside the bound.
- **Named manipulation** — `ae doctor --refresh <session>` is invoked once.
- **Barriers** — `barriers_required` = all four `BAR-929-*`; `barriers_excluded` = none.
- **Raw captures** — `G1`–`G4` at each barrier, separately headed.
- **Calibration** — neutral: the four barriers reached with **no** refresh invoked (`caught=NO`).
  mutated: the controller republishes a **scratch** artifact through the same temp+chmod+mv shape
  and the inode/hash primitive must report the change (`caught=YES`, M13).
- **ARM-INVALID** — if the liveness gate's three conditions are not all confirmed before the
  invocation; if `BAR-929-SERVE` is unreached inside the bound (`INCONCLUSIVE` for that barrier);
  if inactive equivalence is unproven for this fixture.

#### 34. `WD-929-refresh-not-running` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — block `CS-929`, against a session whose watchdog is **not** running, so the
  frozen liveness gate selects the other path. This is the **product-valid opposed construction**:
  v4 paired "refresh with restart" against "refresh with no restart" **on a running watchdog**,
  which the frozen product forecloses — a running watchdog is stopped and started as part of the
  refresh (ae:8621–8679). A no-restart refresh is reachable only when there is nothing to restart.
- **Fixture facts** — block `FX-929-STOPPED`: the same session with the watchdog stopped by the
  product's own path, confirmed absent by census, pidfile and pane set.
- **Named manipulation** — `ae doctor --refresh <session>` is invoked once.
- **Barriers** — `barriers_required` = `BAR-929-PUB`, `BAR-929-PRERETURN`; `barriers_excluded` =
  `BAR-929-RESTART`, `BAR-929-SERVE`, each captured as a bounded non-occurrence.
- **Raw captures** — block `CAP-929`.
- **Calibration** — block `CAL-929`.
- **ARM-INVALID** — if any of the three liveness conditions is still satisfied at invocation time.

#### 35. `WD-929-refresh-fails` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** a refresh interrupted partway leaves the previous artifact whole, so
  the helper's hash at `BAR-929-PRERETURN` equals its hash before the invocation. **B:** it leaves
  a partial or absent artifact. Distinguishable because the helper's bytes and hash are captured
  at every barrier, and `G1`–`G4` stay separate.
- **Fixture facts** — block `FX-929-RUNNING`. **The failure injection is named and sited at the
  real publication**: the watchdog helper is **not** published through
  `_publish_executable_artifact` (ae:833) — session helpers are exempt from that chokepoint by
  shape — but by `>"${AE_META}/watchdog.tmp.$$"` then `chmod 0700` then `mv` at ae:18007–18009.
  v4 hooked ae:833 and the hook would never have fired.
- **Named manipulation** — at `BAR-929-PUB`, with the temp artifact already written and before the
  rename, the controller makes the destination directory non-renameable-into for that one
  publication, once, then restores it. The generator is untouched — the product is not made to
  produce bad bytes.
- **Barriers** — `barriers_required` = `BAR-929-PUB`, `BAR-929-PRERETURN`; `BAR-929-RESTART` and
  `BAR-929-SERVE` are recorded if observed and bounded if not, never required.
- **Raw captures** — `G1`–`G4`; the temp artifact's presence and bytes at each barrier; the full
  meta manifest before and after with symmetric difference.
- **Calibration** — neutral: the same barriers with the permission change not applied
  (`caught=NO`). mutated: the same rename-blocked publication against a **scratch** directory,
  reported by the same primitive (`caught=YES`, M13).
- **ARM-INVALID** — block `INV-929`, plus: if the destination permission change is not confirmed landed
  **and** reverted (M5).

#### 36. `WD-929-restart-state` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** the state observable after a restart is reconstructed from durable
  facts, so it matches the pre-restart state on every captured key. **B:** some of it lives only in
  the process, so it does not survive. Distinguishable because the arm captures the same key set
  before and after as a full set with the symmetric difference emitted.
- **Fixture facts** — block `FX-929-RUNNING`, additionally driven to a non-initial nudge/streak
  state **by the product** through the refusing-target class, never by planting counters.
- **Named manipulation** — the product's own `watchdog stop` followed by `watchdog start`, once.
- **Barriers** — `barriers_required` = `BAR-929-RESTART`, `BAR-929-SERVE`; `barriers_excluded` =
  `BAR-929-PUB`, since no publication is invoked.
- **Raw captures** — before and after: `watchdog=` meta bytes; `.watchdog.pid` bytes; serving pid
  and start time; nudge counter, undelivered streak and alert set as full sets; events set with
  symmetric difference; daemon log bytes with cycle indices.
- **Calibration** — neutral: the same two captures with no restart between them (`caught=NO`).
  mutated: the controller advances a nonce counter in a scratch file, reported by the same
  set-difference primitive (`caught=YES`, M13).
- **ARM-INVALID** — if the pre-restart state is not confirmed non-initial by the product's own
  path; if either control invocation returns non-zero, which is recorded rather than retried.

#### 37. `WD-980-alert-bytes` — **CAPTURE-ONLY**
- **No candidate space, no legs, no assertion.** This arm records bytes and provenance and makes
  no comparison; the artifact says so in its own header.
- **Fixture facts** — a real daemon reaching an alert through its own cadence, with the
  `AE_WATCHDOG_*` pacing knobs at documented low values; no planted alert.
- **Barriers** — none.
- **Raw captures** — the emitted event's `action` and `summary` bytes with `od -c` and sha256;
  the full event line; the invocation provenance (which process emitted, pid, ppid, argv, cycle
  index); the daemon log around the emission.
- **M13 canary** — the controller emits a nonce line through the same event-read primitive from
  a scratch file, which the capture must return byte-for-byte.
- **ARM-INVALID** — if the alert did not arise from the product's own cadence; if the canary does
  not come back.

---

## 4. M12 instantiated — one concrete A/B/C matrix per leaked row

Six matrices, one per L2 row, at execution grain. A reference to the general mechanism is not an
instantiation of it, so each is written out.

### 4.1 SC-920 — `WD-920-origin-matrix`

**Origin is proven by INITIATING-PATH PROVENANCE, never inferred from the writer at the tty.**
Every write to a pane travels through the tmux server, so the process observed touching the tty is
the same for a daemon delivery and a controller write and cannot distinguish them. Instead each
specimen's origin is recorded **at its initiating call site**: the daemon's delivery through
`BAR-920-SEND` (hook-only, delegating the call unchanged, recording pid, ppid, argv and stamp),
and the controller's write recorded by the controller at its own call site in the same tuple
shape. Both registers are harness artifacts, segregated from product state and hashed separately.

#### Gate B — two product-valid opposed constructions plus an agreeing control

| specimen | origin (initiating path) | rendered bytes | product-valid because |
|---|---|---|---|
| S1 | daemon | the daemon's own delivered bytes | a real daemon delivery through the real send path |
| S2 | controller | **byte-identical to S1 in the captured, scrubbed range — no fallback** | writing to a pane's tty is what a human or another writer legitimately does |
| S3 | daemon | the daemon's own delivered bytes **carrying a distinct session goal** | driven through the daemon's own path by the documented `goal` helper, whose value the daemon interpolates into its nudge text (ae:16477–16480). Never hand-planted |
| S4 | controller | ordinary prose of neither shape | **the agreeing control** |

**Byte-identity is a hard admission requirement (colead BLOCKER 2).** v3 permitted a "same shape
class" fallback when byte-identity was inconvenient. That concession is **withdrawn**: if S1 and
S2 differ by even one byte in the captured range, differing bytes are an available explanation
for any difference in treatment, and the arm can no longer isolate origin. **If byte-identity
cannot be achieved, the arm is `ARM-INVALID` and the shortfall is named — no capture is taken.**
Achieving it is a fixture problem (S2 is replayed from S1's own captured bytes, and the capture
range and scrub come from that session's own generated artifacts), not a licence to relax.

Opposition in both axes: S1/S2 differ **in origin at byte-constant shape**; S1/S3 differ **in
shape at constant origin**. Neither forces the product to manufacture a result.

#### Gate A — the synthetic pair
Two committed synthetic pane buffers, built to drive the arm's predicate to each of the two
labels, recorded with their hashes. **Insufficient on its own, and labelled so in the artifact**:
nothing in Gate A touches the product.

#### Gate C — pre-registration, and where S2's bytes come from
The real fixture builder, both synthetics, all four specimen recipes, every library, the
instrumented binary and its `BAR-920-SEND` patch, every shim, the frozen blob and tree hashes:
registered in `P-REGISTER`, recomputed by the runner in `P-CAPTURE`.

**S2 replays S1's bytes, so Gate C must say WHICH of two regimes produces them** (colead v5
IMPORTANT 1) — the design picks the first and the runner enforces it:

1. **HARVEST-AT-PREPARE (chosen).** S1's bytes are harvested during `P-PREPARE`, written to a
   registered specimen file with `provenance=both`, and hash-pinned **before** gate 2. S2 replays
   that registered file, so the bytes a seat reviews are the bytes the arm uses.
2. *Generate-at-capture by a hash-pinned recipe* — permitted only if the recipe, not the output,
   is the registered object, and the arm records the produced hash as a result.

**A baseline is never silently updated to whatever the source emitted.** If S1's live bytes differ
from the registered specimen at capture time, the arm is `ARM-INVALID` and the difference is
reported — never absorbed by re-pinning, which would make registration vacuous and let the
byte-identity requirement pass by construction.

#### What the raw capture must leave reachable
Raw pane bytes with `od` at each stabilization observation, the initiating-path register, the
settled hash at each point, and the frozen scrub applied identically to all four specimens —
with **no relation stated between them**. Both a conforming and a divergent seat reading must
remain reachable from the artifact.

**`ARM-INVALID`** — byte-identity of S1/S2 not achieved; any specimen not constructible; the
initiating-path register missing an entry for any specimen; inactive equivalence not proven for
the `BAR-920-SEND` patch on this fixture with a working known-difference control.

### 4.2 SC-921 — the `WD-921-*` arms

- **Gate A** — two committed synthetic roster listings, one containing a monitor-stamped pane and
  one not, driving the roster predicate to each label. Hashes recorded; labelled insufficient.
- **Gate B** — **B1:** a session whose only non-agent panes are the product-created `_events` and
  `_watchdog` panes, with no agent spawned. **B2:** the same session after one real agent is
  spawned through the product's own `spawn` helper. The two differ by a known member, so both
  readings — monitor panes counted, and monitor panes not counted — remain reachable from the
  captured sets. **Agreeing control:** the same session with the watchdog stopped and the monitor
  window absent and no agent spawned, where the two candidates coincide.
- **Gate C** — fixture builder, both synthetics, the spawn recipe, libraries, frozen hashes.
- **`ARM-INVALID`** — if the spawned agent's pane cannot be confirmed present and stamped; if any
  canary pane is created inside a measured session (the L-DISCRIM roster-pollution lesson).

### 4.3 SC-926 — the `WD-926-*` arms

- **Gate A** — two committed synthetic fact-tuples `(rc, stdout bytes, watchdog= bytes, census)`,
  one internally agreeing and one internally disagreeing, driving the arm's predicate to each
  label. Labelled insufficient.
- **Gate B** — **B1:** the control invocation **uncut**, an ordinary product-valid invocation.
  **B2:** the same invocation cut at the named pre-intent boundary — product-valid because a
  process interrupted between two of its own steps is a state the product can legitimately be
  brought to, the hook only blocks and announces, and the controller performs the signal.
  **Agreeing control: the ORDINARY UNCUT INVOCATION** (colead v5 BLOCKER 8). v5 named the
  post-intent cut as the agreeing control while `WD-926-start-cut-post-intent` and
  `WD-926-stop-cut-post-intent` use those same cuts to **discriminate** — a cut cannot be both the
  discriminator and the agreement case. The uncut invocation is where the two candidates coincide;
  the post-intent cuts remain discriminators and are listed as such.
- **Gate C** — the four instrumented copies (one hook each), their patches and hash triples, the
  fixture builders, both synthetics, frozen blob and tree hashes.
- **`ARM-INVALID`** — inactive equivalence unproven for any of the four hook/fixture pairs; the
  cut not confirmed landed at the named site; for the stop arms, the daemon not confirmed running
  beforehand.

### 4.4 SC-927 — the `WD-927-*` arms

- **Gate A** — two committed synthetic before/after meta manifests, one differing by a removed
  pidfile and one identical, driving the mutation predicate to each label. Labelled insufficient.
- **Gate B** — **B1:** the status surface invoked against a session with a **live, confirmed**
  daemon and no residue. **B2:** the status surface invoked against one of the three residue
  fixtures. Both are ordinary product-valid invocations of a documented command; neither plants
  an outcome. **Agreeing control: the construction where the two candidates coincide** — the status surface
  invoked against a session with a live, confirmed daemon and **no residue at all**, where a pure
  read and a cleanup-performing read leave identical state (colead v5 BLOCKER 8). v5 named a
  different read surface (`agents`) as the control; that is **source discrimination**, not
  agreement, and it is retained as a separate useful capture under its own label rather than as
  this matrix's control.
- **Gate C** — the three residue fixture builders with their recorded byte diffs, both synthetics,
  the manifest primitive, libraries, frozen hashes.
- **`ARM-INVALID`** — residue class not confirmed immediately before the read (dead pid still
  dead, pidfile still empty, nonce process still alive); a real daemon running in the sandbox.

### 4.5 SC-928 — the aewatch arms (and the two bash baselines, which close nothing)

**Subject corrected (colead v4 BLOCKER 3).** The row's assigned empirical mechanism is aewatch's
`_locked_append`; a bash-only arm cannot close it. v4 excluded aewatch explicitly and faulted the
bash appender, which was the wrong subject for the claim. The aewatch arms carry the row; the two
bash arms are a **labelled baseline that closes no part of it**, and their artifact headers must
say so or the artifact is invalid.

- **Gate A** — two committed synthetic `F1`–`F4` tuples, one showing the emitting daemon alive
  with a subsequent unrelated cycle and one showing it gone with none, driving the predicate to
  each label. Hashes recorded; labelled insufficient.
- **Gate B** — **B1:** the emit path reached at the named cut with the fault **not** applied, an
  ordinary product-valid emission. **B2:** the same path at the same cut with the class-specific
  fault applied. Both are states the product can be brought to; the product is never asked to emit
  a particular outcome, and `F1`–`F4` are captured separately so either reading stays reachable.
  **Agreeing control:** the fault applied to a **different session's** events file that this
  daemon does not write, where the two candidates coincide.
- **Gate C** — the instrumented aewatch copies (one hook each) and their patches, the frozen
  aewatch blob hash, the fault appliers, both synthetics, the four fact primitives, libraries and
  frozen hashes — registered in `P-REGISTER`, recomputed in `P-CAPTURE`.
- **The four cuts are separate arms** because the error classes sit at different points of the
  same call — lock acquisition, open, write, unlock — and one arm covering them could not
  attribute an outcome to a class.
- **`ARM-INVALID`** — the lock file or directory not intact when the fault is bound to open/write;
  inactive equivalence unproven for the aewatch hook on this fixture with a working
  known-difference control; `F2` unreachable inside the bound, which is `INCONCLUSIVE` for `F2`
  alone and recorded as such.
- **Executor contamination, stated rather than hidden.** Siting these cuts requires reading
  `_locked_append`, and reading it exposes its behaviour, so a value-blind executor was never
  available for this row — independently of L4. M12's three gates carry the row; the executor's
  ignorance does not, and never could have.

### 4.6 SC-929 — the `WD-929-*` arms

- **Gate A** — two committed synthetic `(serving pid, serving start time, helper inode, helper
  mtime, helper hash)` tuples, one where the serving process predates publication and one where it
  does not, driving the predicate to each label. Labelled insufficient.
- **Gate B, rebuilt on what the frozen product can actually reach (colead v4 BLOCKER 4).** v4
  paired "refresh with restart" against "refresh with **no** restart" on a running watchdog. The
  frozen product forecloses that pair: a refresh whose liveness gate is satisfied — pidfile, `kill -0`,
  **and** a live `@ae_agent=_watchdog` pane (ae:8627–8649) — stops and starts the watchdog as part
  of the refresh (ae:8621–8679). The reachable opposed pair is therefore **B1:** refresh against a
  **running** watchdog, where that gate selects the restart path; **B2:** refresh against a
  **stopped** watchdog, where it does not, because there is nothing to restart. **Agreeing
  control:** the four barriers reached with no refresh invoked at all.
- **Barriers per arm** — the required/excluded table in §3A governs; this matrix asserts no
  uniform four-barrier requirement.
- **Gate C** — the instrumented copy carrying all four `BAR-929-*` hooks (with `BAR-929-PUB` at
  the real helper publication, ae:18007–18009), its patch and hash triple, the failure-injection
  applier, both synthetics, libraries, frozen hashes.
- **`G1`–`G4` stay separate** — refresh success/failure, serving version, whether a restart
  occurred, and durable state are four facts, never one verdict.
- **`ARM-INVALID`** — `BAR-929-SERVE` unreached inside the bound (`INCONCLUSIVE` for that
  barrier); inactive equivalence unproven per fixture; the liveness gate's three conditions not
  all confirmed (`WD-929-refresh-running`) or not all absent (`WD-929-refresh-not-running`); for
  `WD-929-refresh-fails`, the destination permission change
  not confirmed landed **and** reverted.

---

## 5. Fixture

Reused from T-100 where it already works: the unmodelled `grok`-shaped fake rendering received
lines verbatim (`t100-fake.sh`, `t-artifacts/_harness/`, sha256 `4473878e99e84d35…`); the real
generated daemon started by a real launch; pacing on documented `AE_WATCHDOG_*` knobs only; the
frozen scrubber and capture range extracted from **that session's own generated artifacts** with
its own generated `_lib` sourced, never reimplemented; the pinned UTF-8 locale with the blocking
TAB round-trip proof per live arm; the ledger written BY the checks as they run; per-arm
`SHA256SUMS.txt` that excludes itself and lists from the correct root; harness snapshot.

**The closure spans two trees.** `t100-lib.sh`, `t100-run.sh` and `t100-mktables.sh` live only in
`t-artifacts/_harness/`; only `t100-fake.sh` was ever copied into `l-artifacts/_harness/`
(byte-identical), because L-DISCRIM and L-832C reused the fake. A closure registration built from
one tree alone is short by three files, so `PREREGISTRATION.tsv` enumerates by path from both
roots and the runner recomputes both.

**Shared blocks are files, not prose (M14).** `SHARED-BLOCKS.tsv` carries every block an arm
references, keyed `<KIND>-<ROW>`: `CS-*` candidate space, `FX-*` fixture, `BAR-*` barriers,
`CAP-*` captures, `CAL-*` calibration, `INV-*` invalid conditions. The checker expands each
referenced block into each referencing arm and validates the **expanded** arm, so gate 2 proves
per-arm closure and calibration rather than trusting prose inheritance.

**Why this is not cosmetic, demonstrated on this document.** v4's arms carried 62 prose
back-references of the form "as arm 16". Splitting one arm into three and rebuilding two rows
renumbered the roster — and **every one of those references silently retargeted**: "as arm 16" in
an SC-921 arm came to point at an SC-913 arm, with nothing to flag it. A back-reference by
position is a pointer into a list that changes; a block id is a name that does not. The
`FX-D25-PRIOR`, `FX-834A-PENDING`, `FX-913-OCCUPIED`, `FX-913-DEAD`, `FX-913-NOECHO`,
`FX-928A-DAEMON`, `FX-928B-BASH`, `FX-929-RUNNING` and `FX-929-STOPPED` fixtures are the blocks
that carry real construction detail; the rest are shared field bodies.

**A second product source enters the closure.** The `CUT-928A-*` family instruments
`contrib/aewatch/aewatch`, so its blob hash joins the frozen `ae` blob and tree hashes in
`PREREGISTRATION.tsv`, with its own instrumented copies (one hook each) and their own
inactive-equivalence proofs. The aewatch arms additionally require a `uv` runtime, whose absence
is `ARM-INVALID` with the shortfall named rather than a silent bash fallback.

**New barrier sites at v5** — `CUT-926-START-RUNTIME`, `CUT-926-START-INTENT`,
`CUT-926-STOP-RUNTIME`, `CUT-926-STOP-INTENT`, `CUT-928-APPEND`, `CUT-928-LOCK`, `BAR-929-PUB`,
`BAR-929-SERVE`, `BAR-929-RESTART`, `BAR-929-PRERETURN`, `BAR-920-SEND`, and the four
`CUT-928A-*` cuts in aewatch. Each needs its own
hook-patch version, its own hash triple, and a **per-fixture** inactive-equivalence proof with a
**working known-difference control** before a single hooked capture. A control that cannot fail
is not a control.

---

## 6. Change log against colead's v5 gate

Delivered as a neutral delta by the lead. Where a rebuild needed a product fact, it was derived
from `72c7293` (or the recorded aewatch blob) by this worker, not requested back.

| finding | disposition |
|---|---|
| **B1** M14 still false; seven live positional references, several stale | **Accepted; confirmed worse than stated.** Four had already gone stale when the roster renumbered — still parsing, still reading sensibly, pointing at the wrong arm — and two were unresolved placeholders my own conversion script left behind. All replaced with stable arm ids or block ids. **M14 now bans ordinals outright** in arm fields and §4, and **gate 2 must RED-PROVE the rejection** against a deliberately seeded reference, because this rule has now been asserted twice with no mechanism behind it. |
| **B2** `WD-928A-write` / `-unlock` are not execution grain | **Accepted, and the constructibility is answered at gate 1, not deferred.** Write becomes **ENOSPC on a dedicated filesystem** the controller creates at prepare and fills at the barrier — no descriptor closing, no manufactured `errno`, both forbidden to a hook — with the buffering caveat stated (the error may surface at flush/close, so the arm captures *where* it surfaced). **Unlock is declared `ARM-INVALID` at gate 1 and does not run**: no controller-performed, product-valid mechanism can make `LOCK_UN` on a live descriptor err. It is recorded in `ARM-GAPS.tsv` with what would lift it. A declared gap is evidence; a quietly dropped one is not. |
| **B3** SC-929 per-arm barrier sets are impossible | **Accepted.** Barriers are typed `barriers_required` / `barriers_excluded` per arm, with a table showing which are excluded **by construction** and which are captured as bounded non-occurrences. Gate 2 rejects an arm requiring a barrier its own construction excludes. §4.6 no longer asserts a uniform four-barrier requirement. |
| **B4** SC-913 exercises the wrong surface; the named sink is unreachable | **Accepted.** Both delivery lanes are exercised, the uv lane additionally capturing its invocation trace, with `(arm_id, lane)` as the unit of closure. The sink correction is derived and confirmed: `undelivered.launch-<slot>.txt` is written on the **launch** path (ae:12689–12691), while the nudge path's refusal, abandonment and unconfirmed branches all exit before any store is written. The arms now capture the **full candidate durable-store set reachable from each nudge path**, and the launch sink is retained only as an explicitly out-of-scope control excluded from every set comparison. |
| **B5** SC-901 never pins the selector | **Accepted.** Explicit bash and uv lanes in separate sandboxes, `AE_WATCHDOG_IMPL` pinned per lane, every process and ownership record tagged with the pinned lane so attribution is a field rather than an inference. |
| **B6** SC-834a captures the wrong object; arm 4's precondition | **Accepted.** Captures become the `agent.<slot>` row byte-for-byte before and after, the resolved alias / tool kind / config command the walk reads (ae:8757–8776), the `launch_id` and `launch_time` inputs, the candidate bytes **with per-range provenance**, and the post-recovery row. The pending slot is moved into the fixture, established at prepare, so the arm's single manipulation is the candidate's mid-run appearance and nothing else. |
| **B7** D25 captures presence, which collapses two readings | **Accepted.** Per-implementation **cycle and decision evidence keyed to the new session's id** is now required; pid, heartbeat and pane existence are demoted to context and explicitly named insufficient. The flipped selector reaches each invocation through a recorded env-file replayed by `env -i`, captured and hashed per invocation — "shared environment" is not an artifact. |
| **B8** two M12 agreeing controls are misidentified | **Accepted.** SC-926's agreement case becomes the **ordinary uncut invocation**; the post-intent cuts stay discriminators, since a cut cannot be both. SC-927's becomes the **no-residue live-daemon construction** where both candidates coincide; the different-surface capture is kept under its own label as source discrimination, not as the control. |
| **B9** the old M2 scope bug survives in the uniform runner rule | **Accepted.** The uniform `ARM-INVALID` line is scoped to **harness-provenance** mismatches and **harness-owned** regenerating entry points, restoring the provenance boundary the lead ruled. Separately, the M12 set is now **derived from the typed `m12` field** rather than a prose count — which is how v5 said fifteen while its own roster said eighteen. |
| **I1** SC-920's byte-identical replay is dynamically sourced | **Accepted.** Gate C names the regime: **harvest-at-prepare**, S1's bytes registered `provenance=both` and hash-pinned before gate 2, S2 replaying that registered file. If S1's live bytes differ at capture time the arm is `ARM-INVALID` and the difference is reported — **never absorbed by re-pinning**, which would make registration vacuous. |
| **I2** the no-implementation-statement claim is false | **Accepted.** Narrowed to **no expected outcome in any worker-facing arm field**, with the reason stated: reachability cannot be established without reading the frozen source, and an arm that cannot say what the product forecloses is the arm that names an unreachable candidate. The gate-2 linter's scope matches the narrowed claim — arm fields and surface lines, not change-log rationale. |

**37 arm specs; 47 executed units** (10 specs run in two pinned lanes). 35 RED, 1 CAPTURE-ONLY,
1 declared gap.

## 7. What this design does not contain

- **No expected outcome in any worker-facing arm field** — candidate spaces, manipulations,
  barriers, captures, calibration, invalid conditions. That is the claim, and it is narrower than
  v5's, which asserted no statement of what the frozen implementation does *anywhere* and was
  therefore false: §6 and several arm preambles carry source-derived readings of the frozen code,
  because **reachability cannot be established without them** — an arm that cannot say which
  states the product forecloses is the arm that names an unreachable candidate. Those readings are
  legitimate; the absolute claim was not. **The gate-2 linter's scope matches the narrowed
  claim**: it lints arm fields and surface lines, not change-log rationale.
- No SEAT CLASSIFICATION ANNEX; this worker does not write one and will not read one.
- No leaked content, only the leak register in §0.
- No reading of `crit-assign.md` or the referenced defect issues.
- No claim to close the six `Authority: UNRESOLVED` rows.
- No scripts yet, and no run: two gates stand between this file and execution.
- No measured lint figure, deliberately — see §1 M1.
- No claim that a pinning check is a correctness check. Every citation here was checked against
  the frozen file, which is **layer 1** — that the line number lands. **Layer 2** is aptness, that
  the line fits the claim it supports, and v4 shipped a citation that cleared layer 1 and did not
  clear layer 2. Uniform pinning makes layer 2 *harder* to see, because everything that survives
  carries a verified line number and therefore looks equally checked.
