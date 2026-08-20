# T-WD design — watchdog cluster — worker draft **v3** (NOTHING APPROVED, NOTHING RUN)

Drafted by `opus5:lexec` for seat gate by `fable5:lead` and `gpt56sol:colead`. Sole-writer
draft; this file is mine and nothing else in the evidence tree is touched by it.

**v3 answers colead's v2 gate: BLOCKER 1–4, IMPORTANT 1–2, the SC-920 construction test and
the nit.** Section 6 is the change log against each finding.

**Two gates, not one.** Approving the arms below is not approval to run them. Per colead:
**pre-registered scripts take a separate pre-run seat gate**, after the scripts exist and
their dependency closure is registered. Nothing executes before that second gate.

**Value-blindness.** Every RED arm names a CANDIDATE SPACE — a fixture in which two
implementations would differ — and never what the frozen implementation does. Expected
relations belong in a SEAT CLASSIFICATION ANNEX which this worker does not write and will
not read.

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

**Consequence, stated plainly: no unleaked seat remains among the three participants.** The
Q3a remedy routed SC-920's symmetry certification to colead because they had not been leaked
to; L2 came from that seat, colead has since read issue 51 for the authority audit, and the
lead authored L1. Symmetry can therefore no longer be certified by anyone's judgment here — which is why M12
replaces judgment with three gates that are checkable by a leaked reviewer.

**Recorded as doctrine, since it is the general lesson:** *a remedy that depends on someone
staying uninformed is not a remedy.* In a session whose participants exchange rulings by design,
an unleaked seat is a temporary state, not a resource.

### The remedy this design adopts: **M12, a three-part requirement**, binding for all six L2 rows

Ruled jointly. The three parts are **separate gates**; passing one is not passing another.

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
reading reachable to a seat**. If the fake or the TUI cannot construct the matrix, the arm is
`ARM-INVALID` rather than a one-sided capture, and the shortfall is named.

**Gate C — the REAL fixture is pre-registered too**, hash-pinned beside the two synthetics,
inside the full executable dependency closure plus the frozen tree hash (M2). Inspecting a
committed artifact is a fact-check, not a judgment.

**Why this survives universal contamination.** A leaked reviewer cannot be trusted to JUDGE
whether an arm is symmetric, but can verify that two committed fixtures produced two different
outcomes, that two product-valid constructions exist, and that a hash matches. Those are facts,
and facts do not care who reads them. It is the vacuity control one level up: every RED arm
must report `NO` at least once; every leaked arm must additionally reach both states through
the product.

**Amendments reopen the affected arm.** Any change to a registered closure invalidates that
arm's prior gates; A, B and C are re-run before it may capture again.

**Authority note.** Six rows carry `Authority: UNRESOLVED`; per colead, IS capture proceeds
and **closes no normative field**. No artifact here is to be mistaken for closing one, and no
probe or defect report may become authority.

---

## 1. The method — twelve enforced mechanisms

### M1 — candidate space stated, outcome absent; the lint covers the surface table
Every RED arm carries a mandatory `CANDIDATE SPACE:` field. A committed linter rejects the
design if any RED arm lacks it or if any arm text or **surface line** matches
`expected | should | correctly | known to succeed | resolves | does not resolve | passes |
fails | verifies that`. It reads declared fields and skips backticked spans.

**The v2 exemption is withdrawn.** v2 exempted the entire 14-row surface table to absorb one
meta-mention in SC-980's line — un-linting exactly the row-description surface the
`crit-assign` leak implicated. Instead, **SC-980's surface line is rewritten neutrally** (§3,
marked as adapted on colead's instruction and therefore no longer verbatim) and **all surface
rows are linted**. `exempt_rows` must be `0`; a non-zero value is a lint failure. A zero-row
run is `ARM-INVALID` for the lint, not a pass.

The linter is a belt. **The structural guarantee is the seat-owned typed projection**, not
vocabulary matching, and the linter's header says so — it cannot tell a mention from a use.

**Lint status at v3: `rows_examined=65  exempt_rows=0  hits=0`, MEASURED BY THIS WORKER AND NOT
YET INDEPENDENTLY CHECKABLE.** The linter is not in the tree at gate 1 — it lands with the
scripts — so no seat can reproduce that figure today. It is recorded so nobody later reads it as
seat-verified. **It becomes checkable at gate 2**, when the linter is committed and a seat runs
it. Same rule as everywhere else in this programme: a number a reviewer cannot regenerate is
still only the worker's word, and saying so is the difference between a pending check and a
false one.

### M2 — pre-registration of the full executable dependency closure
Script-hash identity is insufficient: an unchanged script can source a changed harness, hook
patch, shim, fixture builder or generated helper. **`PREREGISTRATION.tsv` registers the whole
closure** — arm script; every library it sources; the instrumented binary and its hook patch;
every shim; every fixture builder; the frozen `ae` blob hash **and the frozen tree hash**; and
the generated-helper hashes captured at run time. **The runner recomputes the closure and
refuses to execute on any mismatch.** Changes require an `AMENDMENTS.md` entry and a new
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
(M4), plus explicit capture of the generator's rc and stderr. An arm whose tool crashed
reports `ARM-INVALID`.

### M6 — the verifier never repairs
No regenerate-then-compare mode. A lint rejects any write to a path the same script later
reads as evidence.

### M7 — derivation, never transcription
Anything shown is generated from its source document by dropping fields structurally, with the
generator committed.

### M8 — each leg calibrated separately
Every leg gets its own mutation firing that leg and no other; the calibration table records
per-leg `caught`. Overlapping mutations are forbidden.

### M9 — every citation pinned, and its limit stated
Every `ae:NNNN` resolved against `git show 72c7293:ae` by a committed pinner emitting **the
line text beside the claim**. `CITATIONS.tsv` carries (claim, file, line, line text, sha256).
**A pin proves resolution, not aptness** — only a seat reading the source catches a confident
citation on a wrong-but-plausible line.

### M10 — typed fields, validated headers
Every artifact a table reads is a TSV with a declared header the generator validates first.

### M11 — the row set is checked against an authoritative opposite side
v2's generator read ids out of **this design** and asserted a hand count — which agrees
perfectly with the wrong set and has no opposite side. **Corrected: exact id AND batch sets
are compared BOTH DIRECTIONS against the seat-owned value-blind projection filtered to T-WD,
with the symmetric difference emitted. The count is output, never the oracle.** The projection
does not exist yet, so **M11 is `PENDING-DEPENDENCY` and no arm runs until it lands** — the
row set is not self-certified in the meantime.

### M12 — symmetry as a mechanism
As §0. Applies to every L2 row.

---

## 2. Arm classes, typed

Two classes, distinguished by TYPE rather than by prose exemption (colead IMPORTANT 2):

| class | must carry | must not carry |
|---|---|---|
| **RED** | `CANDIDATE SPACE`, neutral + mutated legs (M3), per-leg mutations (M8), landing check (M5) | — |
| **CAPTURE-ONLY** | provenance of the invocation, recorder-liveness control, exact bytes with `od`, sha256, and an explicit in-artifact statement that it makes no comparison | no candidate space, no legs, no assertion of any kind |

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

### The arms

Twenty-one arms. Every one gets its own disposable sandbox and its own fixture clone; no arm
shares a sandbox. `RL` = the arm's recorder-liveness control, run before its measurement and
`ARM-INVALID` if it does not register.

| # | arm id | row | class | fixture clone | named manipulation | barriers | recorder-liveness control | primary captures |
|---|---|---|---|---|---|---|---|---|
| 1 | `WD-D25-serve-at-start` | D25 | RED | fresh launch, mode switch set at start | the documented mode switch is set / unset before launch | none | process census registers a known daemon start | serving process argv, pidfile bytes, meta watchdog keys, census |
| 2 | `WD-D25-serve-after-flip` | D25 | RED | as 1 | the switch is flipped while a daemon is running | none | as 1 | census and serving argv after a cycle and after a restart |
| 3 | `WD-834A-pending-at-start` | SC-834a | RED | pending item planted before launch | one producer-derived pending item | none | the invocation recorder registers a known `_recover-pending` call | invocation trace, events delta, daemon log |
| 4 | `WD-834A-pending-midrun` | SC-834a | RED | as 3 | the pending item appears mid-run | none | as 3 | invocation trace across N cycles, events delta |
| 5 | `WD-900-resume-overlimit` | SC-900 | RED | product-produced over-limit log | the retention knob is lowered; the log is grown past it **by the product** | none | inode/byte recorder registers a known change | pre/post-resume inode, bytes, line count, open readers, container set |
| 6 | `WD-900-run-overlimit` | SC-900 | RED | as 5 | the same bound is crossed **during an ordinary run, no resume** | none | as 5 | same captures |
| 7 | `WD-901-two-sessions` | SC-901 | RED | one `AE_HOME`, two sessions | two real launches | none | census registers a known daemon | process census, per-daemon ownership records, pidfiles |
| 8 | `WD-901-second-start` | SC-901 | RED | as 7 | a second daemon start for one session | none | as 7 | census, pidfile bytes, reap trace |
| 9 | `WD-913-deliver-live` | SC-913 | RED | live unmodelled pane | none (baseline delivery) | none | the argv recorder registers a known delivery | delivery argv trace, event row, pane bytes, verified-submit evidence |
| 10 | `WD-913-deliver-refused` | SC-913 | RED | pane made non-agent | the pane's process is replaced by a shell | none | as 9 | rc, verdict, undelivered streak, whether an event was written |
| 11 | `WD-920-origin-matrix` | SC-920 | RED (**M12**) | see §4 | see §4 | quiet-stabilization arming | see §4 | see §4 |
| 12 | `WD-921-monitor-in-roster` | SC-921 | RED | session carrying its own monitor panes | none (incumbent shape) | none | roster recorder registers a known agent pane | roster set, per-branch verdict map, health denominator — **as full sets** |
| 13 | `WD-921-monitor-stamped` | SC-921 | RED | as 12 | a monitor pane's agent stamp is added / removed | none | as 12 | same captures |
| 14 | `WD-926-intent-agrees` | SC-926 | RED | daemon started normally | none | none | the rc/stdout recorder registers a known failure | control-surface rc, stdout, durable intent key, census |
| 15 | `WD-926-intent-disagrees` | SC-926 | RED | three sub-fixtures | stale pidfile / orphaned pid / pidfile naming a live non-daemon process | none | as 14 | same captures per sub-fixture |
| 16 | `WD-927-status-residue` | SC-927 | RED | as 15 | none beyond the sub-fixture | none | the no-mutation recorder registers a known mutation | before/after manifests, rc, stdout, census |
| 17 | `WD-927-startstop-residue` | SC-927 | RED | as 15 | explicit start / stop invoked instead | none | as 16 | same captures |
| 18 | `WD-928-append-fails` | SC-928 | RED | events container made unwritable | mode change plus an **inability canary** recorded refusing | none | canary refusal is the recorder-liveness control | rc, stdout/stderr, event file bytes, whether the external effect landed |
| 19 | `WD-929-refresh-ordering` | SC-929 | RED | running daemon | `doctor --refresh` invoked | none | version recorder registers a known version change | pre/post serving version, serving process identity, rc, durable before/after facts |
| 20 | `WD-929-refresh-fails` | SC-929 | RED | as 19 | the refresh is made to fail by a named construction | none | as 19 | rc, which daemon serves after, durable facts |
| 21 | `WD-980-alert-bytes` | SC-980 | **CAPTURE-ONLY** | alert driven by real cadence | none | none | the byte recorder registers a known emitted event | emitted action and summary bytes, `od`, sha256, invocation provenance |

**Per-arm invalid / inconclusive conditions**, uniform and enforced by the runner:
`ARM-INVALID` when — the closure hash check fails (M2); the recorder-liveness control does not
register; the landing check fails or the generator rc is non-zero (M5); the neutral leg does
not report `NO` (M3); a leg's mutation fires another leg (M8); an M12 arm has not shown both
outcomes. `INCONCLUSIVE` when — a bounded wait expires; the bound and the state at expiry are
recorded and no absence is inferred.

**Execution order.** (i) M11's projection check; (ii) every arm's recorder-liveness control;
(iii) M12 reachability proofs for the six L2 rows; (iv) arms 1–21 in id order; (v) generated
tables. Any failure at (i)–(iii) stops the arms it gates.

---

## 4. `WD-920-origin-matrix` — the three gates, in full

The pattern below is M12 instantiated. **Arms 11–20 (the six L2 rows) each carry the same three
gates**; SC-920 is written out because its matrix is the hardest to construct.

**Origin is proven OUT OF BAND, never from the bytes.** Each specimen's origin is established by
*which process wrote it* — a real daemon delivery through the real send path, versus a controller
write to the pane's tty — recorded at write time with the writing process's pid, argv and
timestamp, in a register separate from the pane capture.

### Gate B specimens — two product-valid opposed constructions plus an agreeing control

| specimen | origin (out of band) | rendered shape | product-valid because |
|---|---|---|---|
| S1 | daemon | the daemon's own delivered shape | a real daemon delivery through the real send path |
| S2 | controller | **byte-identical to S1** where the fixture permits, otherwise explicitly the *same shape class*, and which was achieved is recorded | a write to the pane's tty is what a human or another writer legitimately does |
| S3 | daemon | a shape the daemon does not normally emit | driven through the daemon's own path by a documented knob, never hand-planted |
| S4 | controller | ordinary prose of neither shape | **the agreeing control** |

Opposition is constructed in both axes: S1/S2 differ in origin at constant shape; S1/S3 differ in
shape at constant origin. **Neither construction forces the product to manufacture a result** —
each is a state the product can legitimately be brought to. **If any specimen cannot be
constructed, the arm is `ARM-INVALID` and the shortfall is named**; a one-sided capture is not
taken.

### Gate A — the synthetic pair
Two committed synthetic buffers drive the arm's predicate to each label. Recorded, and recorded
as **insufficient on its own**.

### Gate C — pre-registration
The real fixture builder, both synthetics, every library, the instrumented binary and its patch,
every shim, the frozen blob and tree hashes: all registered, all recomputed by the runner.

### What the raw capture must leave reachable
The arm emits the raw pane bytes with `od`, the out-of-band origin register, and the frozen
scrubbed hash at each point, and **states no relation between them**. Both a conforming and a
divergent seat reading must remain reachable from the artifact; nothing in it narrows what a seat
may conclude.

## 5. Fixture

Reused from T-100 where it already works: the unmodelled `grok` fake rendering received lines
verbatim; the real generated daemon started by a real launch; pacing on documented
`AE_WATCHDOG_*` knobs only; the frozen scrubber and capture range extracted from **that
session's own generated artifacts** with its own generated `_lib` sourced, never
reimplemented; the pinned UTF-8 locale with the blocking TAB round-trip proof per live arm;
the ledger written BY the checks as they run; per-arm `SHA256SUMS.txt`; harness snapshot.

Any NEW barrier site needs its own patch version, hash triple, and per-fixture
inactive-equivalence proof with a working known-difference control before a single hooked
capture. Only arm 11 currently needs one.

---

## 6. Change log against colead's v2 gate

| finding | disposition |
|---|---|
| **BLOCKER 1** family sketch, not execution grain | §3 now carries 21 stable arm ids with row, class, fixture clone, named manipulation, barriers, recorder-liveness control, captures, uniform invalid/inconclusive conditions and execution order. **Accepted, and the second pre-run gate is written into the header.** |
| **BLOCKER 2** 14-row lint exemption | **Accepted.** Exemption withdrawn; SC-980's line rewritten neutrally and marked as adapted; all surface rows linted; `exempt_rows` must be 0. The lint is named a belt, with the typed projection as the structural guarantee. |
| **BLOCKER 3** M11 self-certifies | **Accepted.** Both-direction id+batch comparison against the seat-owned projection, symmetric difference emitted, count demoted to output, and M11 marked `PENDING-DEPENDENCY` so no arm runs until the projection exists. |
| **BLOCKER 4** M2 script-only identity | **Accepted.** Full executable dependency closure plus frozen blob and tree hashes registered and recomputed by the runner. The lead records this as their own under-specification rather than a worker defect; it is noted here for the same reason. |
| **Three-part M12** (joint ruling) | **Folded in.** Gate A synthetic pair, labelled necessary-and-insufficient; Gate B two product-valid opposed constructions plus an agreeing control, exercising both states THROUGH the product and never forcing it; Gate C the real fixture pre-registered inside the closure. Separate gates. Amendments reopen the arm. Applied to all six L2 rows, not only SC-920. |
| **colead's SC-920 nuance** | **Folded in.** The product need not emit both outcomes; the RAW capture must leave conforming and divergent readings distinguishable, and §4 states that explicitly. |
| **IMPORTANT 1** SC-900 resume path | **Accepted, and independently verified in the frozen source before adopting**: the trim is guarded by `RESUMING == true`, caps to `AE_EVENTS_KEEP` (default 1000, numeric-guarded), trims `tail -n N > tmp && mv` under `events.jsonl.lock`, and precedes the monitor pane start. Arms 5 and 6 are the resume / no-resume pair; SC-928 is separate as arm 18. |
| **IMPORTANT 2** F13 contradicts M1/M3 | **Accepted.** Typed arm classes in §2; CAPTURE-ONLY excluded by type, and both generators dispatch on `class`. |
| **SC-920 symmetry** | §4, in full: out-of-band origin proof, the four-specimen matrix with byte-identical-or-named-shape-class, both directions constructible, the agreeing control, M12 reachability, and `ARM-INVALID` if the matrix cannot be built. |
| **NIT** title and mechanism count | Title is v3; §1 says twelve mechanisms and defines twelve. |
| handback count | Colead used the corrected `11 examined + 14 exempt = 25`. The v3 lint has no exemption, so the figure to check is `rows_examined` with `exempt_rows = 0`. |

## 7. What this design does not contain

- No expected outcome anywhere. If any sentence reads as one, it is a defect and I want it
  flagged rather than excused.
- No SEAT CLASSIFICATION ANNEX; this worker does not write one and will not read one.
- No leaked content, only the leak register in §0.
- No reading of `crit-assign.md` or the referenced defect issues.
- No claim to close the six `Authority: UNRESOLVED` rows.
- No scripts yet, and no run: two gates stand between this file and execution.
