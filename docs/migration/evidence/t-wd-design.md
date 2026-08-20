# T-WD design — watchdog cluster — worker draft **v4** (NOTHING APPROVED, NOTHING RUN)

Drafted by `opus5:lexec` for seat gate by `fable5:lead` and `gpt56sol:colead`. Sole-writer
draft; this file is mine and nothing else in the evidence tree is touched by it.

**v4 answers colead's v3 gate: BLOCKER 1–5, IMPORTANT 1–3**, and the lead's confirmation that
**M12 binds all six leaked rows, each with its own concrete A/B/C matrix at execution grain**.
Section 6 is the change log against each finding.

**v3's clearance does not travel to this file.** The lead gated `a5245a5` specifically. This is
a different blob and re-enters gate 1 from the beginning, both seats.

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

## 1. The method — thirteen enforced mechanisms

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
what was and was not done. **The figure becomes checkable at gate 2**, when the linter is
committed and a seat runs it.

### M2 — pre-registration of the full executable dependency closure, in a stopped prepare phase
Script-hash identity is insufficient: an unchanged script can source a changed harness, hook
patch, shim, fixture builder or generated helper. **`PREREGISTRATION.tsv` registers the whole
closure** — arm script; every library it sources; the instrumented binary and its hook patch;
every shim; every fixture builder; every Gate-A synthetic; the frozen `ae` blob hash **and the
frozen tree hash**; **and the generated-helper hashes**.

**Phasing (colead IMPORTANT 2).** Generated-helper hashes captured at run time cannot belong to
a pre-run closure, so fixture preparation is a **separate, stopped phase**:

| phase | what happens | what it produces |
|---|---|---|
| **P-PREPARE** | every sandbox is built, every session launched, every generated helper emitted — **then everything is stopped**. No arm runs. | the fixture trees, at rest |
| **P-REGISTER** | the closure is hashed over the stopped trees, `PREREGISTRATION.tsv` written and **committed** | the registered closure |
| **P-GATE2** | seats review the committed scripts and closure | clearance |
| **P-CAPTURE** | arms run against the already-prepared fixtures | the evidence |

**The capture runner refuses to prepare.** In `P-CAPTURE` it recomputes the closure, and
**fails `ARM-INVALID` if any generated helper differs from its registered hash, or if any
fixture-builder or asset-regenerating entry point is reached at all.** Regeneration during
capture is an invalid run, not a silent re-baseline. Changes require an `AMENDMENTS.md` entry
and a new registered closure.

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

### M13 — equipment canaries are CONTROLLER-generated, never product outcomes *(colead IMPORTANT 3)*
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

**Thirty arms** — 29 RED, 1 CAPTURE-ONLY. Every arm gets its own disposable sandbox and its own
fixture clone; no arm shares a sandbox. Fifteen arms (rows SC-920/921/926/927/928/929) carry
M12 and take their A/B/C matrix from §4.

| # | arm id | row | class | M12 |
|---|---|---|---|---|
| 1 | `WD-D25-serve-at-start` | D25 | RED | — |
| 2 | `WD-D25-serve-after-flip` | D25 | RED | — |
| 3 | `WD-834A-pending-at-start` | SC-834a | RED | — |
| 4 | `WD-834A-pending-midrun` | SC-834a | RED | — |
| 5 | `WD-900-resume-overlimit` | SC-900 | RED | — |
| 6 | `WD-900-run-overlimit` | SC-900 | RED | — |
| 7 | `WD-901-two-sessions` | SC-901 | RED | — |
| 8 | `WD-901-second-start` | SC-901 | RED | — |
| 9 | `WD-913-lock-contention` | SC-913 | RED | — |
| 10 | `WD-913-occupied-input` | SC-913 | RED | — |
| 11 | `WD-913-dead-pane` | SC-913 | RED | — |
| 12 | `WD-913-submit-unverified` | SC-913 | RED | — |
| 13 | `WD-913-durable-failure` | SC-913 | RED | — |
| 14 | `WD-913-delivery-count` | SC-913 | RED | — |
| 15 | `WD-920-origin-matrix` | SC-920 | RED | §4.1 |
| 16 | `WD-921-monitor-in-roster` | SC-921 | RED | §4.2 |
| 17 | `WD-921-monitor-stamped` | SC-921 | RED | §4.2 |
| 18 | `WD-926-start-cut-pre-intent` | SC-926 | RED | §4.3 |
| 19 | `WD-926-start-cut-post-intent` | SC-926 | RED | §4.3 |
| 20 | `WD-926-stop-cut-pre-intent` | SC-926 | RED | §4.3 |
| 21 | `WD-926-stop-cut-post-intent` | SC-926 | RED | §4.3 |
| 22 | `WD-927-residue-dead-pid` | SC-927 | RED | §4.4 |
| 23 | `WD-927-residue-empty-pidfile` | SC-927 | RED | §4.4 |
| 24 | `WD-927-residue-recycled-pid` | SC-927 | RED | §4.4 |
| 25 | `WD-928-writer-fault` | SC-928 | RED | §4.5 |
| 26 | `WD-928-lock-fault` | SC-928 | RED | §4.5 |
| 27 | `WD-929-refresh-ordering` | SC-929 | RED | §4.6 |
| 28 | `WD-929-refresh-fails` | SC-929 | RED | §4.6 |
| 29 | `WD-929-restart-state` | SC-929 | RED | §4.6 |
| 30 | `WD-980-alert-bytes` | SC-980 | **CAPTURE-ONLY** | — |

**Uniform invalid / inconclusive conditions**, enforced by the runner for every arm:
`ARM-INVALID` when — the closure hash check fails or a regenerating entry point is reached
(M2); the M13 canary does not come back through the capture primitive; the landing check fails
or the generator rc is non-zero (M5); the neutral leg does not report `NO` (M3); a leg's
mutation fires another leg (M8); an M12 arm has not shown both outcomes (M12 Gate A) or cannot
construct its matrix (Gate B). `INCONCLUSIVE` when — a bounded wait expires; the bound and the
state at expiry are recorded and no absence is inferred.

**Execution order.** (i) M11's projection check; (ii) `P-PREPARE` → `P-REGISTER` → `P-GATE2`;
(iii) per-arm M13 canaries; (iv) M12 Gate A + Gate B reachability for the fifteen M12 arms;
(v) arms 1–30 in id order; (vi) generated tables. Any failure at (i)–(iv) stops the arms it
gates.

**Named cut sites and barriers used below**, each a pinned citation (M9) against
`git show 72c7293:ae`. A cut is a controller-driven signal at a hook-emitted barrier: the hook
blocks and announces, the CONTROLLER acts, per the cluster-plan admissibility rule.

| id | site | frozen anchor |
|---|---|---|
| `CUT-926-START-RUNTIME` | after the watchdog pane split returns, before the durable intent write | `_watchdog_start`, ae:15089–15101 |
| `CUT-926-START-INTENT` | after `_set_meta_watchdog "true"` returns, before the success line | `_watchdog_start`, ae:15102–15103 |
| `CUT-926-STOP-RUNTIME` | after the pid kill / pidfile removal / pane kill, before the durable intent write | `_watchdog_stop`, ae:15044–15051 |
| `CUT-926-STOP-INTENT` | after `_set_meta_watchdog "false"` returns, before `exit 0` | `_watchdog_stop`, ae:15060 |
| `CUT-928-APPEND` | with the flock already held, immediately before the append writer | `ae_log_append`, ae:13175 |
| `CUT-928-LOCK` | at the lock acquisition itself, before the writer is reached | `ae_log_append`, ae:13174 |
| `BAR-929-PUB` | the rename that publishes a regenerated session helper | `_publish_executable_artifact`, ae:833 |
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
- **CANDIDATE SPACE** — **A:** the mode switch selects the serving implementation at launch, so
  exactly one implementation's process exists and the other's does not. **B:** the switch
  selects only which implementation is *started*, leaving the other's artifacts (pidfile, pane,
  meta keys) present from a prior state. Distinguishable because the arm captures the process
  census **and** both implementations' artifact sets as full sets (M4).
- **Fixture facts** — fresh `AE_HOME`; one session launched with `workspace.watchdog` enabled;
  `AE_WATCHDOG_IMPL` set to `uv` in one leg and unset in the other; the `contrib/aewatch`
  script and a `uv` runtime present in both, so availability is not what differs.
- **Named manipulation** — `AE_WATCHDOG_IMPL=uv` is exported (or not) in the launching
  environment, once, before launch.
- **Barriers** — none.
- **Raw captures** — full process census with argv; `.watchdog.pid` bytes; the `ae-aewatch`
  tmux session list; `@ae_agent` pane stamps in the session; `watchdog=` meta key; the aewatch
  heartbeat file's existence and mtime.
- **Calibration** — neutral: census taken with no session launched at all (`caught=NO`).
  mutated: a controller-planted decoy process under a watchdog-shaped argv that the census must
  report (`caught=YES`).
- **ARM-INVALID** — if `uv` or `contrib/aewatch` is absent in either leg, since the switch would
  then not be the only difference; the shortfall is named and no capture is taken.

#### 2. `WD-D25-serve-after-flip` — RED
- **CANDIDATE SPACE** — **A:** the mode switch is read once at launch, so flipping it mid-run
  changes nothing until a restart. **B:** it is re-read per cycle or per control invocation, so
  the serving process changes without a restart. Distinguishable because the census is taken at
  three points: before the flip, after a full cycle, and after an explicit restart.
- **Fixture facts** — as arm 1, launched in the non-`uv` leg; `AE_WATCHDOG_INTERVAL_SEC` pinned
  low so a cycle boundary is reached inside the bound.
- **Named manipulation** — `AE_WATCHDOG_IMPL` is flipped in the environment of a subsequent
  control invocation, once, while a daemon is running.
- **Barriers** — none.
- **Raw captures** — census + argv at all three points; both implementations' artifact sets;
  serving pid and its start time at each point.
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
- **Fixture facts** — one launched session; one pending item planted **before** launch, derived
  from a real producer (a genuine tracked `ask` whose reply never arrives) and mutated only by a
  named manipulation with a recorded byte diff.
- **Named manipulation** — the request row's state field is set to the pending value by the
  producer-derivation rule, with the byte diff recorded.
- **Barriers** — a delegate-and-log shim on the `_recover-pending` path recording pid, ppid,
  argv and stamp, delegating unchanged.
- **Raw captures** — invocation trace (may be empty; emptiness is recorded as a fact, never as
  an absence verdict); events delta as a full set; daemon log bytes; the request row before and
  after.
- **Calibration** — neutral: same fixture with the pending item absent (`caught=NO`). mutated:
  the controller invokes the shim's delegate target directly under a nonce argv, which the trace
  must record (`caught=YES`) — an M13 canary, not a product outcome.
- **ARM-INVALID** — if the bound expires before N cycles complete, `INCONCLUSIVE`.

#### 4. `WD-834A-pending-midrun` — RED
- **CANDIDATE SPACE** — **A:** pending items are discovered per cycle, so one appearing mid-run
  is picked up without a restart. **B:** the pending set is read once at daemon start.
  Distinguishable because the item is planted **after** the daemon has completed at least one
  clean cycle, and the trace is captured across N further cycles.
- **Fixture facts** — as arm 3, but the daemon is running and has completed one cycle before
  the plant; cycle boundaries observed at `BAR-QS-ARM` or from the daemon log.
- **Named manipulation** — the same producer-derived pending item is planted once, mid-run.
- **Barriers** — as arm 3.
- **Raw captures** — as arm 3, plus the cycle index at plant time and at each trace entry.
- **Calibration** — as arm 3.
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
  the container at both points.
- **Calibration** — neutral: stop/resume with the knob left at its default and the log below the
  bound (`caught=NO`). mutated: the controller appends a nonce line to a **scratch** file and the
  same inode/byte primitive must report the change (`caught=YES`).
- **ARM-INVALID** — if the product did not itself grow the log past the bound.

#### 6. `WD-900-run-overlimit` — RED
- **CANDIDATE SPACE** — as arm 5, with the distinguishing axis being **when**: **A:** the bound
  is enforced on any crossing, so an ordinary run behaves as a resume does. **B:** enforcement is
  bound to the resume path specifically, so an ordinary run crossing the same bound differs.
- **Fixture facts** — as arm 5, but the bound is crossed **during an ordinary run with no
  stop and no resume**; identical knob value, identical growth mechanism.
- **Named manipulation** — the same knob lowered once, before the growth, with no resume.
- **Barriers** — none.
- **Raw captures** — as arm 5, sampled at the same three cycle offsets.
- **Calibration** — as arm 5.
- **ARM-INVALID** — if a resume occurs at any point, since the arm's whole axis is its absence.

### SC-901 — daemon count per `AE_HOME` and what each owns

#### 7. `WD-901-two-sessions` — RED
- **CANDIDATE SPACE** — **A:** daemons are per-session, so two sessions in one `AE_HOME` yield
  two processes with disjoint ownership. **B:** they are per-`AE_HOME`, so one process serves
  both. Distinguishable because the arm captures the census and each daemon's ownership records
  as full sets.
- **Fixture facts** — one `AE_HOME`; two real launches with distinct session names, neither a
  prefix of the other (the `#102` topology lesson); both with the watchdog enabled.
- **Named manipulation** — the second session is launched; nothing else changes.
- **Barriers** — none.
- **Raw captures** — census with argv and ppid; per-session `.watchdog.pid` bytes; each pidfile's
  pid mapped to its argv; `@ae_agent` stamps per session; both `watchdog=` meta keys.
- **Calibration** — neutral: census with one session launched (`caught=NO`). mutated: an M13
  controller `sleep` under a nonce argv the census must report (`caught=YES`).
- **ARM-INVALID** — if either launch fails or the two sessions share a name prefix.

#### 8. `WD-901-second-start` — RED
- **CANDIDATE SPACE** — **A:** a second start for a session already served is refused or
  collapses to the incumbent, leaving one process. **B:** it produces a second process.
  Distinguishable because the census is a full set before and after and the pidfile bytes are
  captured at both points.
- **Fixture facts** — as arm 7 but with a single session already served by a running daemon.
- **Named manipulation** — the product's own `watchdog start` is invoked a second time.
- **Barriers** — none.
- **Raw captures** — census before/after; pidfile bytes before/after; rc and stdout of the second
  start; any reap trace; the `_watchdog` pane set.
- **Calibration** — neutral: the census taken twice with no second start (`caught=NO`). mutated:
  as arm 7 (`caught=YES`).
- **ARM-INVALID** — if the incumbent daemon is not confirmed running before the second start.

### SC-913 — the nudge delivery mechanism and what is verified about the path

v3 had two cells (a baseline delivery and a shell pane). The neutral surface names **six
independent dimensions**, and a two-cell arm cannot fail four of them, so each dimension gets a
cell that can independently produce the unwanted answer. The six share the **fixture builder**
and nothing else: each runs in its own sandbox and takes its own captures, so a defect in one
construction cannot propagate into another's reading.

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
  fails loudly with a non-zero rc, leaving the occupying text intact. **B:** delivery proceeds
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
  never a change to the product); target otherwise identical to arm 9's.
- **Named manipulation** — the fake is started in non-echo mode, once, at fixture build.
- **Barriers** — none.
- **Raw captures** — rc, stdout, stderr; pane bytes with `od`; paste-buffer state; event rows as
  a full set; the stored message body's presence as a raw fact.
- **Calibration** — neutral: the same send against the fake in ordinary echo mode (`caught=NO`).
  mutated: the controller drives the same verification primitive against its own scratch pane
  with a known-absent marker (`caught=YES`, M13).
- **ARM-INVALID** — if the fake's non-echo mode cannot be confirmed by a controller probe before
  the send.

#### 13. `WD-913-durable-failure` — RED
- **CANDIDATE SPACE** — **A:** a delivery that did not land leaves no durable delivery record,
  so the event set and the body store are unchanged across the attempt. **B:** it leaves a record
  indistinguishable from a landed one. Distinguishable because the arm captures the event set and
  the body-store set as **full sets before and after**, with the symmetric difference emitted.
- **Fixture facts** — its **own sandbox**, running its own copies of the three failure
  constructions from arms 10, 11 and 12 in sequence. It shares the fixture builder with them and
  reads none of their artifacts, so a defect in one arm's construction cannot enter this arm's
  reading.
- **Named manipulation** — one per sub-construction, each firing that sub-construction only (M8).
- **Barriers** — none.
- **Raw captures** — event set and body-store set before and after each attempt, with symmetric
  differences; rc and stderr per attempt; the daemon log if the daemon was the sender.
- **Calibration** — neutral: an attempt that is allowed to land normally (`caught=NO`).
  mutated: the controller appends a nonce row to a scratch event file and the set-difference
  primitive must report it (`caught=YES`, M13).
- **ARM-INVALID** — if any sub-construction fails to reach its intended failure class, which is
  named rather than substituted.

#### 14. `WD-913-delivery-count` — RED
- **CANDIDATE SPACE** — **A:** the daemon's counter counts **deliveries**, so attempts that did
  not land do not advance it. **B:** it counts **attempts**, so they do. Distinguishable because
  the arm runs a known number of non-landing attempts followed by a landing one and captures the
  counter's rendered value and the alert set at every cycle.
- **Fixture facts** — a real daemon; a target constructed to refuse (arm 11's class);
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

#### 15. `WD-920-origin-matrix` — RED, **M12 §4.1**
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

#### 16. `WD-921-monitor-in-roster` — RED, **M12 §4.2**
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

#### 17. `WD-921-monitor-stamped` — RED, **M12 §4.2**
- **CANDIDATE SPACE** — **A:** participation is decided by the pane's `@ae_agent` stamp alone, so
  adding or removing a stamp moves a pane in or out of the roster. **B:** it is decided by
  something else (pane provenance, window, or process), so the stamp does not move it.
  Distinguishable because the stamp is the single thing manipulated and the roster is a full set
  before and after.
- **Fixture facts** — as arm 16, with the monitor panes present and the daemon running.
- **Named manipulation** — one monitor pane's `@ae_agent` stamp is added or removed, once, by the
  controller, with the byte diff of the pane-option set recorded.
- **Barriers** — `BAR-QS-ARM`.
- **Raw captures** — as arm 16, before and after the stamp change, with the symmetric difference
  of the roster sets emitted.
- **Calibration** — neutral: the same before/after capture with no stamp change (`caught=NO`).
  mutated: as arm 16 (`caught=YES`, M13).
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

#### 18. `WD-926-start-cut-pre-intent` — RED, **M12 §4.3**
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

#### 19. `WD-926-start-cut-post-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — as arm 18, at the opposite side of the same boundary: **A:** the success
  report is emitted only after the durable intent write, so a cut after it leaves intent written
  and no report. **B:** the report precedes the durable write. Distinguishable because rc, the
  emitted bytes and the durable key are three separate captures.
- **Fixture facts** — as arm 18; instrumented copy carrying only the `CUT-926-START-INTENT` hook.
- **Named manipulation** — the controller signals at `CUT-926-START-INTENT`.
- **Barriers** — `CUT-926-START-INTENT`.
- **Raw captures** — as arm 18.
- **Calibration** — as arm 18.
- **ARM-INVALID** — as arm 18.

#### 20. `WD-926-stop-cut-pre-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — **A:** stopping writes durable intent and mutates runtime as one
  effective unit, so a cut before the intent write leaves both untouched or both done. **B:** the
  runtime mutation (kill, pidfile removal, pane kill) completes and the durable key does not
  follow. Distinguishable because runtime facts and the durable key are captured separately.
- **Fixture facts** — a prepared session with the watchdog **running**, confirmed by census and
  pidfile before the arm; instrumented copy carrying only `CUT-926-STOP-RUNTIME`.
- **Named manipulation** — the controller signals at `CUT-926-STOP-RUNTIME`.
- **Barriers** — `CUT-926-STOP-RUNTIME`.
- **Raw captures** — as arm 18, plus: the killed pid's liveness, the `_watchdog` pane's presence,
  and the tmux user options the stop path clears, each as its own fact.
- **Calibration** — as arm 18.
- **ARM-INVALID** — as arm 18, plus: if the daemon is not confirmed running before the cut.

#### 21. `WD-926-stop-cut-post-intent` — RED, **M12 §4.3**
- **CANDIDATE SPACE** — as arm 20 at the opposite side: **A:** the durable key and the reported
  outcome agree once the intent write has returned. **B:** they can disagree at that point.
- **Fixture facts** — as arm 20; instrumented copy carrying only `CUT-926-STOP-INTENT`.
- **Named manipulation** — the controller signals at `CUT-926-STOP-INTENT`.
- **Barriers** — `CUT-926-STOP-INTENT`.
- **Raw captures** — as arm 20.
- **Calibration** — as arm 20.
- **ARM-INVALID** — as arm 20.

### SC-927 — the status surface's read/write behaviour and where cleanup is performed

The three pid-residue fixtures live here, per colead. Each is its own arm with its own sandbox.

#### 22. `WD-927-residue-dead-pid` — RED, **M12 §4.4**
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

#### 23. `WD-927-residue-empty-pidfile` — RED, **M12 §4.4**
- **CANDIDATE SPACE** — **A:** cleanup is keyed on the recorded pid's liveness, so an **empty**
  pidfile — which names no pid at all — is a distinct case from a dead pid and the meta file set
  is unchanged by the read. **B:** cleanup is keyed on the pidfile being unusable for any reason,
  so emptiness and a dead pid are handled alike. Distinguishable because this arm and arm 22
  differ in exactly this one property and in nothing else; they are separate arms because one arm
  covering both classes could not tell the two mechanisms apart.
- **Fixture facts** — as arm 22 but `.watchdog.pid` is zero bytes, confirmed by size and hash.
- **Named manipulation** — the pidfile is truncated to zero bytes, once, byte diff recorded.
- **Barriers** — none.
- **Raw captures** — as arm 22.
- **Calibration** — as arm 22.
- **ARM-INVALID** — as arm 22.

#### 24. `WD-927-residue-recycled-pid` — RED, **M12 §4.4**
- **CANDIDATE SPACE** — **A:** liveness of the recorded pid is sufficient, so a pidfile naming
  **any** live process leaves the meta file set unchanged by the read. **B:** liveness is not
  sufficient and the daemon's own stamped pane is also required, so a live but unrelated pid is
  treated as residue. Distinguishable because the named process is alive and demonstrably not
  this session's daemon, and no `_watchdog`-stamped pane exists in the session.
- **Fixture facts** — as arm 22, but `.watchdog.pid` names a controller-owned `sleep` under a
  nonce argv, confirmed alive immediately before the read, and **no `_watchdog`-stamped pane
  exists** in the session.
- **Named manipulation** — the pidfile is written with the live non-daemon pid, once, byte diff
  recorded.
- **Barriers** — none.
- **Raw captures** — as arm 22, plus the nonce process's liveness before and after the read as
  its own fact.
- **Calibration** — as arm 22.
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

#### 25. `WD-928-writer-fault` — RED, **M12 §4.5**
- **CANDIDATE SPACE** — **A:** an append error is contained at the emit site, so the emitting
  process survives and later cycles continue. **B:** it propagates, so the emitting process
  terminates. `F1`–`F4` are captured separately precisely so both readings stay reachable.
- **Fixture facts** — a running bash watchdog; the instrumented copy carrying only the
  `CUT-928-APPEND` hook, which blocks **with the flock already held** so the fault cannot be
  attributed to lock acquisition; the lock file itself remains writable throughout.
- **Named manipulation** — at the barrier, the controller makes the **target events file alone**
  unappendable, once, leaving `events.jsonl.lock` and the containing directory writable; the
  byte/mode diff is recorded and reverted after the capture.
- **Barriers** — `CUT-928-APPEND`.
- **Raw captures** — `F1`, `F2`, `F3`, `F4` as four separately-headed artifacts; the events file
  bytes and hash before and after; the daemon log across at least two cycle indices; the pane
  bytes of the `_watchdog` pane.
- **Calibration** — neutral: the same barrier reached with the manipulation not applied
  (`caught=NO`). mutated: the controller makes a **scratch** file unappendable and the same
  write primitive must report the failure (`caught=YES`, M13).
- **ARM-INVALID** — if the lock file or the directory is not writable at the moment of the fault
  (which would unbind the fault from the writer); if the inactive-equivalence proof for this
  hook/fixture pair has not passed with a working known-difference control; if `F2` cannot be
  evaluated because no further cycle boundary is reached inside the bound, in which case `F2`
  alone is `INCONCLUSIVE` and is recorded as such rather than read as "no cycle occurred".

#### 26. `WD-928-lock-fault` — RED, **M12 §4.5**
- **CANDIDATE SPACE** — **A:** a failure to acquire the append lock is handled the same way as a
  failure to write, so `F1`–`F4` match arm 25's. **B:** the two are handled differently.
  Distinguishable because the two arms differ **only** in which of the two named cuts is used.
- **Fixture facts** — as arm 25; the instrumented copy carrying only the `CUT-928-LOCK` hook.
- **Named manipulation** — at the barrier the controller holds the lock from a separate,
  controller-owned process for longer than the wait bound, once; the holder's pid and argv are
  recorded, and it is released after the capture.
- **Barriers** — `CUT-928-LOCK`.
- **Raw captures** — as arm 25, plus the lock holder's identity and hold interval.
- **Calibration** — neutral: the same barrier with no competing holder (`caught=NO`). mutated:
  the controller holds a **scratch** lock and the same acquisition primitive must report the
  timeout (`caught=YES`, M13).
- **ARM-INVALID** — as arm 25.

### SC-929 — state after restart, and `doctor --refresh` serving-version ordering

**Four named barriers** (colead IMPORTANT 1), because pre/post snapshots cannot attribute state
to before a command returned:
`BAR-929-PUB` (the rename that publishes a regenerated helper),
`BAR-929-RESTART` (the product's own stop/start pair, when the arm invokes it),
`BAR-929-SERVE` (the first daemon cycle boundary after publication), and
`BAR-929-PRERETURN` (immediately before `doctor --refresh` returns to its caller).
"Which version serves" is derived from **raw facts only** — the serving process's pid and start
time versus the published helper's inode, mtime and hash — never from a marker written into the
product.

#### 27. `WD-929-refresh-ordering` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** publication replaces the on-disk artifact only, so the process
  serving at `BAR-929-PRERETURN` is the one that predates `BAR-929-PUB`. **B:** publication is
  accompanied by a restart, so it does not. Distinguishable because process identity and helper
  identity are captured independently at all four barriers.
- **Fixture facts** — a running bash watchdog; the instrumented copy carrying the four
  `BAR-929-*` hooks; `AE_WATCHDOG_INTERVAL_SEC` pinned low so `BAR-929-SERVE` is reachable
  inside the bound.
- **Named manipulation** — `ae doctor --refresh <session>` is invoked once.
- **Barriers** — all four `BAR-929-*`.
- **Raw captures** — at each barrier: serving pid, ppid, argv and start time; the on-disk helper's
  inode, mtime, size and sha256; the `_watchdog` pane set; `watchdog=` meta bytes; the refresh
  invocation's rc and stdout.
- **Calibration** — neutral: the same four barriers reached with **no** refresh invoked
  (`caught=NO`). mutated: the controller republishes a **scratch** artifact through the same
  temp-and-rename shape and the inode/hash primitive must report the change (`caught=YES`, M13).
- **ARM-INVALID** — if `BAR-929-SERVE` is not reached inside the bound (`INCONCLUSIVE` for that
  barrier, recorded, never read as an absence); if inactive equivalence has not passed per
  fixture.

#### 28. `WD-929-refresh-fails` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** a refresh that fails partway leaves the previous artifact whole,
  so the helper's hash at `BAR-929-PRERETURN` equals its hash before the invocation. **B:** it
  leaves a partial or absent artifact. Distinguishable because the helper's bytes and hash are
  captured at every barrier.
- **Fixture facts** — as arm 27. **The failure injection is named**: at `BAR-929-PUB`, with the
  temp artifact already generated and before the rename, the controller makes the **destination
  directory** non-renameable-into for that one publication, once, and restores it after the
  capture. The generator itself is untouched — the product is not made to produce bad bytes.
- **Named manipulation** — that one directory-permission change at that one barrier.
- **Barriers** — all four `BAR-929-*`.
- **Raw captures** — as arm 27, plus: the temp artifact's presence and bytes at each barrier;
  the refresh's rc, stdout and stderr as bytes; the full meta manifest before and after with
  symmetric difference.
- **Calibration** — neutral: the same barriers with the permission change not applied
  (`caught=NO`). mutated: the controller performs the same rename-blocked publication against a
  **scratch** directory and the same primitive must report the failure (`caught=YES`, M13).
- **ARM-INVALID** — as arm 27, plus: if the destination permission change cannot be confirmed
  landed and reverted (M5).

#### 29. `WD-929-restart-state` — RED, **M12 §4.6**
- **CANDIDATE SPACE** — **A:** the state observable after a restart is reconstructed from durable
  facts, so it matches the pre-restart state on every captured key. **B:** some of it lives only
  in the process, so it does not survive. Distinguishable because the arm captures the same key
  set before and after as a full set with the symmetric difference emitted.
- **Fixture facts** — a running bash watchdog that has completed at least two cycles and has a
  non-initial nudge/streak state, reached **by the product** through arm 14's refusing-target
  class rather than by planting counters.
- **Named manipulation** — the product's own `watchdog stop` followed by `watchdog start`, once.
- **Barriers** — `BAR-929-RESTART`, `BAR-929-SERVE`.
- **Raw captures** — before and after: `watchdog=` meta bytes; `.watchdog.pid` bytes; serving pid
  and start time; nudge counter, undelivered streak and alert set as full sets; events set with
  symmetric difference; daemon log bytes with cycle indices.
- **Calibration** — neutral: the same two captures with no restart between them (`caught=NO`).
  mutated: the controller advances a nonce counter in a scratch file and the same set-difference
  primitive must report it (`caught=YES`, M13).
- **ARM-INVALID** — if the pre-restart state is not confirmed non-initial by the product's own
  path; if either control invocation returns non-zero, which is recorded rather than retried.

### SC-980 — the incumbent alert's action and summary bytes

#### 30. `WD-980-alert-bytes` — **CAPTURE-ONLY**
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

#### Gate C — pre-registration
The real fixture builder, both synthetics, all four specimen recipes, every library, the
instrumented binary and its `BAR-920-SEND` patch, every shim, the frozen blob and tree hashes:
registered in `P-REGISTER`, recomputed by the runner in `P-CAPTURE`.

#### What the raw capture must leave reachable
Raw pane bytes with `od` at each stabilization observation, the initiating-path register, the
settled hash at each point, and the frozen scrub applied identically to all four specimens —
with **no relation stated between them**. Both a conforming and a divergent seat reading must
remain reachable from the artifact.

**`ARM-INVALID`** — byte-identity of S1/S2 not achieved; any specimen not constructible; the
initiating-path register missing an entry for any specimen; inactive equivalence not proven for
the `BAR-920-SEND` patch on this fixture with a working known-difference control.

### 4.2 SC-921 — arms 16 and 17

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

### 4.3 SC-926 — arms 18–21

- **Gate A** — two committed synthetic fact-tuples `(rc, stdout bytes, watchdog= bytes, census)`,
  one internally agreeing and one internally disagreeing, driving the arm's predicate to each
  label. Labelled insufficient.
- **Gate B** — **B1:** the control invocation **uncut**, an ordinary product-valid invocation.
  **B2:** the same invocation cut at the named pre-intent boundary — product-valid because a
  process interrupted between two of its own steps is a state the product can legitimately be
  brought to, the hook only blocks and announces, and the controller performs the signal.
  **Agreeing control:** the same invocation cut at the **post-intent** boundary, where the two
  candidates coincide — which is why arms 19 and 21 exist as arms rather than as prose.
- **Gate C** — the four instrumented copies (one hook each), their patches and hash triples, the
  fixture builders, both synthetics, frozen blob and tree hashes.
- **`ARM-INVALID`** — inactive equivalence unproven for any of the four hook/fixture pairs; the
  cut not confirmed landed at the named site; for the stop arms, the daemon not confirmed running
  beforehand.

### 4.4 SC-927 — arms 22–24

- **Gate A** — two committed synthetic before/after meta manifests, one differing by a removed
  pidfile and one identical, driving the mutation predicate to each label. Labelled insufficient.
- **Gate B** — **B1:** the status surface invoked against a session with a **live, confirmed**
  daemon and no residue. **B2:** the status surface invoked against one of the three residue
  fixtures. Both are ordinary product-valid invocations of a documented command; neither plants
  an outcome. **Agreeing control:** a **different read surface** (`agents`) invoked against the
  same residue fixture, which separates "this surface mutates" from "any read mutates" — a
  distinction the arm could not otherwise make.
- **Gate C** — the three residue fixture builders with their recorded byte diffs, both synthetics,
  the manifest primitive, libraries, frozen hashes.
- **`ARM-INVALID`** — residue class not confirmed immediately before the read (dead pid still
  dead, pidfile still empty, nonce process still alive); a real daemon running in the sandbox.

### 4.5 SC-928 — arms 25 and 26

- **Gate A** — two committed synthetic `F1`–`F4` tuples, one showing an emitting process alive
  with a subsequent cycle and one showing it gone with none, driving the predicate to each label.
  Labelled insufficient.
- **Gate B** — **B1:** the emit path reached at the named cut with the fault **not** applied — an
  ordinary product-valid emission. **B2:** the same path at the same cut with the writer-bound
  fault applied. Both are states the product can be brought to; the product is never asked to
  emit a particular outcome, and `F1`–`F4` are captured separately so either reading stays
  reachable. **Agreeing control:** the fault applied to a **different session's** events file
  that this daemon does not write, where the two candidates coincide.
- **Gate C** — both instrumented copies and their patches, the fault appliers, both synthetics,
  the four fact primitives, libraries, frozen hashes.
- **`ARM-INVALID`** — the lock file or directory not writable at fault time (fault unbound from
  the writer); inactive equivalence unproven; `F2` unreachable inside the bound, which is
  `INCONCLUSIVE` for `F2` and recorded as such.

### 4.6 SC-929 — arms 27–29

- **Gate A** — two committed synthetic `(serving pid, serving start time, helper inode, helper
  mtime, helper hash)` tuples, one where the serving process predates publication and one where
  it does not, driving the predicate to each label. Labelled insufficient.
- **Gate B** — **B1:** `doctor --refresh` invoked with **no** restart. **B2:** the same refresh
  followed by the product's own `watchdog stop` + `start`. Both are documented product
  invocations. **Agreeing control:** the four barriers reached with **no** refresh invoked at all,
  where the two candidates coincide.
- **Gate C** — the instrumented copy carrying all four `BAR-929-*` hooks, its patch and hash
  triple, the failure-injection applier, both synthetics, libraries, frozen hashes.
- **`ARM-INVALID`** — `BAR-929-SERVE` unreached inside the bound (`INCONCLUSIVE` for that
  barrier); inactive equivalence unproven per fixture; for arm 28, the destination permission
  change not confirmed landed **and** reverted.

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

**New barrier sites at v4** — `CUT-926-START-RUNTIME`, `CUT-926-START-INTENT`,
`CUT-926-STOP-RUNTIME`, `CUT-926-STOP-INTENT`, `CUT-928-APPEND`, `CUT-928-LOCK`, `BAR-929-PUB`,
`BAR-929-SERVE`, `BAR-929-RESTART`, `BAR-929-PRERETURN`, `BAR-920-SEND`. Each needs its own
hook-patch version, its own hash triple, and a **per-fixture** inactive-equivalence proof with a
**working known-difference control** before a single hooked capture. A control that cannot fail
is not a control.

---

## 6. Change log against colead's v3 gate

| finding | disposition |
|---|---|
| **BLOCKER 1** no RED arm carries a real CANDIDATE SPACE; roster shorthand is not execution grain | **Accepted in full.** §3A gives every one of the 29 RED arms its own block with candidate A and candidate B named, exact fixture facts, ONE named manipulation, its barriers, its raw captures, its neutral and mutated calibration legs, and its arm-specific invalid condition. The roster table is now an index, not the specification. |
| **BLOCKER 2** M12 instantiated once and asserted for the rest; same-shape-class is not constant shape; S3 knob unnamed; origin inferred from the tty writer | **Accepted in full**, and the lead confirms M12 binds all six leaked rows. §4 carries six concrete matrices. The shape-class fallback is **withdrawn** — byte-identity of S1/S2 or `ARM-INVALID`. S3's knob is named: the documented `goal` helper, whose value the daemon interpolates into its own nudge (ae:16477–16480). Origin is now **initiating-path provenance** recorded at each call site, because every pane write travels through the tmux server and the tty writer cannot distinguish them. |
| **BLOCKER 3** SC-926 varied pidfiles — residue, not the durable-intent boundary | **Accepted.** The pid-residue fixtures move to SC-927 (arms 22–24). SC-926 is rebuilt on the boundary the surface names: four arms over {start, stop} × {cut before the durable intent write, cut after it}, with runtime facts, the durable key, rc and output captured separately. |
| **BLOCKER 4** SC-928 named no implementation, no deterministic append boundary, and could not separate the dimensions | **Accepted.** The implementation is named (the bash per-session watchdog, `AE_WATCHDOG_IMPL` unset, aewatch explicitly out of scope). The fault is **writer-bound** at `CUT-928-APPEND` with the flock already held and the lock file left writable, plus a second arm at `CUT-928-LOCK`. `F1`–`F4` — process identity/liveness, subsequent unrelated cycle, operation state, nudge state — are four separately-headed raw facts. |
| **BLOCKER 5** SC-913's two cells cannot fail four of its six dimensions | **Accepted.** Six arms, one per dimension (target-lock, occupied/human-input, dead, submit-verification, durable-failure, delivery-count), each independently fail-capable, each in its own sandbox, sharing the fixture builder and nothing else. |
| **IMPORTANT 1** SC-929 lacked ordering barriers and named no failure injection | **Accepted.** Four named barriers — `BAR-929-PUB`, `BAR-929-RESTART`, `BAR-929-SERVE`, `BAR-929-PRERETURN` — and the injection is named: at `BAR-929-PUB`, after the temp artifact exists and before the rename, the destination is made non-renameable-into for that one publication, then restored. A third arm covers the post-restart half of the surface. |
| **IMPORTANT 2** run-time helper hashes cannot belong to a pre-run closure | **Accepted.** M2 now phases the work `P-PREPARE` → `P-REGISTER` → `P-GATE2` → `P-CAPTURE`, with preparation **stopped** before hashing, and the capture runner **refuses to regenerate**: any changed generated helper, or any fixture-builder entry point reached during capture, is `ARM-INVALID`. |
| **IMPORTANT 3** recorder controls depended on product outcomes | **Accepted.** New **M13**: every canary is controller-generated and pushed through the exact capture primitive with no product involvement, recorded separately, `ARM-INVALID` on failure. The five primitives and their canaries are tabulated. A canary that needs the product to act is a defect in the canary. |
| lint figure | v3's `rows_examined=65` is **withdrawn as stale**, not restated: the arm blocks are rewritten, so the number no longer describes this document. No figure is quoted until the linter is committed and a seat can run it at gate 2. |

## 7. What this design does not contain

- No statement of what the frozen implementation does, anywhere. If any sentence reads as one,
  it is a defect and I want it flagged rather than excused.
- No SEAT CLASSIFICATION ANNEX; this worker does not write one and will not read one.
- No leaked content, only the leak register in §0.
- No reading of `crit-assign.md` or the referenced defect issues.
- No claim to close the six `Authority: UNRESOLVED` rows.
- No scripts yet, and no run: two gates stand between this file and execution.
- No measured lint figure, deliberately — see §1 M1.
