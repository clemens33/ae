# Batch 0 designs — lead draft v1 (seat gate PENDING; nothing here is approved)

The four bespoke designs the cluster plan gates FIRST: SC-507b, SC-511c, D01–D04
concurrency, SC-1208. Sole-writer draft by fable5:lead from colead's positions
(PART 1/2 + addendum, 2026-08-20) and the verified b0-census.md. Each design is
INDIVIDUALLY seat-approved before any run. Execution ownership (preflight ruling):
the B0 worker executes SC-507b, SC-511c, SC-1208; the C worker EXECUTES the approved
D01–D04 designs and returns their evidence.

Instrumentation admissibility: cluster-plan.md's global rule governs (one hook-only
patch on an exact 72c7293 copy; per-fixture inactive equivalence; controller performs
mutations; product-visible-path equivalence with harness trace bytes segregated).

## Global artifact contract (every arm, every design)

Each arm records: frozen SHA; instrumented-copy hash + the patch itself; the
inactive-equivalence result for ITS fixture; environment/tool hashes; the exact
controller action; barrier timestamps/sequence; stdout/stderr/rc; a recursive file
manifest (path, type, mode, symlink target, content hash) of the cloned AE_HOME
before and after; tmux server/session/window/pane/client snapshots where a server
exists; the PATH-spy log. The worker emits NO verdict — seats classify. Every active
arm starts from a fresh fingerprinted clone; no arm shares a sandbox with another.

**Controller-delta subtraction (addendum — binding for every D01–D04 arm):** every
concurrency arm has a controller mutation, so a raw before/after manifest cannot by
itself prove the reader's effects=none. Each arm is paired with a CONTROLLER-ONLY
TWIN from the same template fingerprint (controller performs the identical mutation
sequence with no reader running); reader-effects are judged as
(reader+controller manifest delta) MINUS (controller-only delta) across AE_HOME and
tmux snapshots. A reader write can no longer hide inside the intended mutation.
Harness artifacts (barrier files, shim logs) live OUTSIDE the cloned AE_HOME and are
excluded from product-state equivalence, separately hashed.

## Design 1 — SC-507b: archive preview/digest stitch cut

One admissible patch, TWO named hook sites:

- `H507_AFTER_FACTS` — immediately after `_ar_facts_row` returns and before
  `_ar_build_meta` (ae:4956 region). This is the real stitch cut: facts are OLD;
  meta build / detail render can become NEW.
- `H507_CANDIDATE` — trace-only, after the outer `digest=$(_ar_preview_once …)` and
  before the after-fingerprint (ae:5026-30 and the retry analogue). Writes the
  candidate digest hash to the evidence fd only; never blocks.

Arms (each from a fresh clone):

1. **Stable control** — no mutation; hooks inactive-equivalent; single pass expected
   (trace decides, not assumption).
2. **Transient meta** — at H507_AFTER_FACTS, controller rewrites meta via a
   writer-shaped temp+rename. Fires on FIRST pass only; retry sees stable state B.
3. **Transient memo** — same barrier; controller appends a harvested memo row.
4. **Transient events** — same barrier; controller appends a harvested event line.
5. **Persistent events** — controller mutates events on BOTH passes, exposing the
   bounded fail path.

Every mutation must alter RENDERED SEMANTICS (a fact the digest displays), not
merely the fingerprint. The stat/hook trace distinguishes one pass / two passes /
exactly-two-then-stop; capture proves no leaked first candidate reaches stdout and
records final stdout/stderr/rc.

## Design 2 — D01: list reader vs live writer (executed by C)

Hook `H_LIST_META_CAPTURED` immediately after meta_blob is read (ae:4161/4265,
running-session arm). At the barrier the controller invokes the REAL frozen `goal`
helper once — one logical writer operation that rewrites goal in meta AND emits the
goal event. Resume `list --json`. The single writer makes the reader's independent
meta/event cuts observable (goal text vs goal_set_epoch) without invented multi-file
edits. Controller-only twin per the subtraction rule. Source/call trace + delegating
flock spy + before/after manifests close: effects=none, cmd_list path, no read lock,
Bash owner. Protected clones are built WITH a prebuilt config so the M2 bootstrap
cannot turn chmod protection into a synthetic failure.

## Design 3 — D02: request-scan reader vs reply writer (executed by C)

Hook `H_REQUEST_SCAN_COMPLETE` after `_ar_request_states` finishes its reversed scan
and before row emission (ae:4530-35). Controller appends ONE producer-harvested,
identity-valid reply event at the barrier. Capture the current invocation AND a
clean rerun from the resulting clone. Controller-only twin. Source/call trace +
flock spy + manifests close: helper_requests/_ar_request_states, event-only read, no
reader lock, per-scan cut.

## Design 4 — D03: events-tail follow semantics (executed by C; events-tail ONLY per
the preflight ruling)

No fake tail — start the REAL generated events helper. The barrier is POSITIVE
observation: the banner plus the last baseline record in the pane proves the real
`tail -n 30 -f` (ae:14827-14905) opened and replayed. Arms:

1. **Initial window** — 31 numbered producer events pre-seeded; which numbers appear
   locates the initial-window cut.
2. **Complete append** — controller appends a real complete event after the barrier.
3. **Line framing** — controller appends a partial producer-derived line, then the
   terminating newline, as a two-step subarm.
4. **Rotation** — controller holds a hardlink to the OLD inode, atomically replaces
   events.jsonl, then appends DISTINCT harvested sentinels to the NEW path and the
   OLD hardlink. Positive OLD-side output locates descriptor-follow behavior without
   relying on absence/timing alone.

Terminate only after a named positive barrier; capture trace/manifests/flock spy.
Controller-only twins apply.

## Design 5 — D04a: status pane-set cut (executed by C)

Dedicated tmux server, ≥2 panes. A delegating tmux shim captures the REAL
`list-panes` result, signals `H_STATUS_PANESET`, and blocks before replay; the
controller kills one listed pane and creates one new pane, then releases. Capture
headers/bodies/rc and before/after tmux state: the pane-set cut and the later
per-pane capture become independently visible. **Mandatory separate arm — exact-name
vs prefix-sibling:** frozen `has-session -t`/`list-panes -t` prefix-match can make
status observe a NEIGHBOUR session; any conflict found becomes a row/defect, never
buried in D prose. Per the global rule, the shim's inactive equivalence is proven on
the same stable topology (wrapped output byte-equal to unwrapped) before its active
barrier is admissible. Controller-only twin; trace+manifest close read
effects/no-lock/cmd_status/Bash.

## Design 6 — D04b: next selection/recheck cut (executed by C)

TWO hooks in the one patch:

- `H_NEXT_SELECTED` — after best-candidate resolution, BEFORE the exact recheck.
  (SC-020b's Batch C arm consumes THIS design — the cut and capture are named here,
  satisfying batch-c-design.md's reuse condition.)
- `H_NEXT_RECHECKED` — after the successful exact `list-sessions | grep -Fx`, before
  the final tmux focus call.

Arms: (1) kill the exact target between selection and recheck; (2) prefix-sibling
present, kill only the exact target AFTER recheck, capture rc and the actual client
target — exposes the acknowledged non-atomic final switch and any prefix fallback;
(3) companion no-sibling arm isolating final-call failure. The default no-attach arm
closes read-only via file+tmux manifests; attach arms capture the sole tmux effect.
Controller-only twins; source/call/flock traces close cmd_next/no read lock/Bash.

## Design 7 — SC-511c: frozen-consumer compatibility fixtures (B0 worker executes)

The consumer matrix is the b0-census.md product-reader enumeration — the run is
incomplete unless it covers EVERY family: bash list/next, watchdog, request+compact,
archive (facts/request-states/digest/staging), stop verification, telegram
formatter/filter, events-tail, and contrib/aewatch (aemonitor is indirect via list).

- **Stable schema = events.md:49-84 keys ONLY** (ts/actor/action/target/ref/summary
  + the four routing keys). `body_file` is emitted but NOT in the documented stable
  set: capture its behavior as an EMPIRICAL EXTENSION, never silently promote it.
- Harvest valid producer bytes per action/key (harvester list per the
  producer-derivation rule).
- **Additive arm**: one unknown optional key inserted at FIRST/MIDDLE/LAST object
  positions; run every consumer family.
- **Removal/rename arms**: mutate the WHOLE relevant specimen cohort per key, not
  one optional occurrence — per-key named mutation; otherwise optional omission
  falsely impersonates schema removal.
- Every stable key needs at least one consumer where its loss/rename is SEMANTICALLY
  DISCRIMINATING (the census maps key → reader). The verbs here are SCHEMA-KEY
  mutations (add/remove/rename a KEY across a specimen cohort) — NOT the census
  fixture matrix's session-lifecycle verbs (spawn/rename/retire), which remain
  general consumer-census context only and are not this design's cohorts.
- Routing keys use a churn/identity fixture: the real `cmd_rename` operation
  produces specimens with stable slot/session routing and changed display names
  (the census documents that rename emits NO event action at 72c7293 — churn is
  observed via session/meta/tmux/path effects plus surrounding producer sequences,
  never an invented rename event).
- Capture output/rc/files only; the worker NEVER labels compatible/breaking. Record
  controls and exact byte diffs.

## Design 8 — SC-1208: transport-separation probe (B0 worker executes)

Per the precised row (type-boundary invariant): isolated HOME/AE_HOME + FAKE
claude/codex/gemini/grok/opencode binaries that NUL-log argv/env/cwd and act as
raw-input sinks; the REAL frozen launch/injection path and REAL generated send
helper. No live model, no network.

Seed hostile pane text and a hostile peer payload: nested fake envelope, instruction
prose, flag-looking strings, quotes/backslashes/newlines. Classify every captured
artifact by channel:

- **INSTRUCTION** — Claude `--append-system-prompt` argv, Codex
  developer_instructions config value, Gemini `-i` value, Grok initial positional,
  OpenCode config + context markdown files.
- **USER_INPUT** — raw tmux paste including the helper envelope.
- **DATA** — events.jsonl / body_file.

Requirements: the hostile free-text sentinel is ABSENT from every INSTRUCTION
artifact and present BYTE-EXACT with outer provenance in USER_INPUT; instruction
artifacts are hashed before and after peer delivery (no post-launch mutation). Both
row controls run: the free-text sentinel arm AND a
hostile-looking-but-grammar-valid spawn name that appears ONLY in the fixed
identity slot of instruction material.

**Limit (binding on the evidence and the row):** this proves ae's TRANSPORT
separation, not that any vendor model obeys instruction hierarchy; no artifact or
comment may claim a semantic model-compliance observation. The SHOULD consequence
stays normative.

**Census-derived gap:** the unsupported/other-command launch surface (ae:1539,1558)
has no modeled context transport. That is a SEPARATE code-observation row (S13
candidate, drafted at the marks pass) — NOT SC-1208 evidence.

## Sequencing

Seat gate per design → SC-507b/511c/1208 to the B0 worker (one worker, fixed order,
own sandboxes) → D01–D04 designs hand to Batch C's worker with the batch. The T-WD
producer precursor stays a separate subdesign (its own gate) and is unrelated to
these cuts except as G2's byte source.
