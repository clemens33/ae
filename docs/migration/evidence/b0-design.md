# Batch 0 designs — lead draft v2 (seat gate PENDING; nothing here is approved)

The four bespoke designs the cluster plan gates FIRST: SC-507b, SC-511c, D01–D04
concurrency, SC-1208. Sole-writer draft by fable5:lead from colead's positions
(PART 1/2 + addendum + gate v1 findings, 2026-08-20) and the verified b0-census.md.
Each design is INDIVIDUALLY seat-approved before any run. Execution ownership
(preflight ruling): the B0 worker executes SC-507b, SC-511c, SC-1208; the C worker
EXECUTES the approved D01–D04 designs and returns their evidence.

Instrumentation admissibility: cluster-plan.md's global rule governs (one hook-only
patch on an exact 72c7293 copy; per-fixture inactive equivalence; controller
performs mutations; hooks only BLOCK or EMIT a barrier/pass-ordinal — a hook never
reads, hashes, or computes over product state; product-visible-path equivalence with
harness trace bytes segregated).

**Value-blindness (gate v1 BLOCKER, binding):** every worker-facing arm below is
manipulation + barriers + captures ONLY. No arm tells the worker an expected
outcome, pass count, presence/absence, or direction. Everything the seats compare
lives in the SEAT CLASSIFICATION ANNEX at the end of this file, which is NEVER
included in any worker brief. Worker briefs are derived from the arm specs alone.

## Global artifact contract (every arm, every design)

Each arm records: frozen SHA; instrumented-copy hash + the patch itself; the
inactive-equivalence result for ITS fixture; environment/tool hashes; the exact
controller action; barrier timestamps/sequence; stdout/stderr/rc; a recursive file
manifest (path, type, mode, symlink target, content hash) of the cloned AE_HOME
before and after; tmux server/session/window/pane/client snapshots where a server
exists; the PATH-spy log (allowlisted variables only — see Design 8's env rule).
The worker emits NO verdict. Every active arm starts from a fresh fingerprinted
clone; no arm shares a sandbox with another.

**Controller-delta subtraction (binding for every D01–D04 arm):** every concurrency
arm has a controller mutation, so a raw before/after manifest cannot by itself
isolate the reader's own effects. Each arm is paired with a CONTROLLER-ONLY TWIN
from the same template fingerprint (identical mutation sequence, no reader
running); the captures deliver both deltas. Harness artifacts (barrier files, shim
logs) live OUTSIDE the cloned AE_HOME and are excluded from product-state
equivalence, separately hashed.

**Bounded waits (gate v1 IMPORTANT):** every observation window is a bounded poll
with its timeout recorded. On timeout the arm records an INCONCLUSIVE artifact
(pane capture + file manifests at expiry) and moves to cleanup — a timeout is never
converted into an absence observation by the worker.

## Design 1 — SC-507b: archive preview/digest stitch cut

One admissible patch, two hook sites, both barrier-only:

- `H507_AFTER_FACTS` — immediately after `_ar_facts_row` returns and before
  `_ar_build_meta` (ae:4956 region). Blocks until the controller releases it;
  emits nothing else.
- `H507_PASS` — after the outer `digest=$(_ar_preview_once …)` and the retry
  analogue (ae:5026-30 region). Emits ONLY a pass ordinal to the evidence fd; it
  never reads, hashes, or inspects the digest or any product state.

Named mutations (controller-performed at H507_AFTER_FACTS; each is an input-side
byte diff recorded per the producer-derivation rule):

- **meta**: writer-shaped temp+rename replacing session meta with a harvested
  variant whose ROSTER differs by exactly one agent entry (taken from a real
  spawn-produced meta of the same template lineage).
- **memo**: append one harvested memo row carrying a topic string not present in
  the baseline.
- **events**: append one harvested `ask` event (a request-opening row) not present
  in the baseline.

Arms (each from a fresh clone):

1. **Stable control** — no mutation; instrumented-inactive vs uninstrumented
   equivalence per the global rule; capture stdout/stderr/rc + pass ordinals.
2. **Transient meta** — mutation fires at the barrier on the first pass only.
3. **Transient memo** — same shape, memo mutation.
4. **Transient events** — same shape, events mutation.
5. **Persistent events** — the events mutation is applied at the barrier on EVERY
   pass (a fresh harvested line each time).

**LEAK-COMPARE capture pair (named proof material):** for each transient arm, the
worker also builds a POST-STATE CONTROL clone — same template, the arm's named
mutation applied cold (no run in progress) — and runs the frozen UNINSTRUMENTED
preview on it once, capturing stdout/stderr/rc. Each transient arm therefore
delivers: instrumented-run captures + pass ordinals + the post-state control
captures. The worker is not told what relation should hold between them.

## Design 2 — D01: list reader vs live writer (executed by C)

Hook `H_LIST_META_CAPTURED` immediately after meta_blob is read, RUNNING-session
site (ae:4161) — chosen explicitly: the record's concurrency question is a live
writer racing the reader, and a live writer exists only in the running arm; the
stopped-path read (ae:4265) is not instrumented in this design. At the barrier the
controller invokes the REAL frozen `goal` helper once — one logical writer
operation that rewrites goal in meta AND emits the goal event. Resume `list
--json`; capture stdout/rc, manifests, tmux snapshots, source/call trace,
delegating flock-spy log. Controller-only twin per the global subtraction rule.

M2 note (gate v1 IMPORTANT): protected clones ship a PREBUILT config so chmod
protection cannot manufacture a bootstrap failure — deliberately, so this arm
CANNOT close D01's M2-bootstrap current-effect field. That field cites the
already-accepted M2 census evidence (ownership.md mechanism M2); no new arm.

## Design 3 — D02: request-scan reader vs reply writer (executed by C)

Hook `H_REQUEST_SCAN_COMPLETE` after `_ar_request_states` finishes its reversed
scan and before row emission (ae:4530-35). Controller appends ONE
producer-harvested, identity-valid reply event at the barrier. Capture the current
invocation AND a clean rerun from the resulting clone. Controller-only twin.
Source/call trace + flock spy + manifests.

## Design 4 — D03: events-tail follow semantics (executed by C; events-tail ONLY
per the preflight ruling)

No fake tail — start the REAL generated events helper. The launch barrier is a
positive pane observation: the helper banner plus the final baseline record
rendered (bounded poll; timeout → INCONCLUSIVE per the global rule). Arms:

1. **Initial window** — 31 uniquely numbered producer events pre-seeded; capture
   the pane after the launch barrier.
2. **Complete append** — after the launch barrier, controller appends one real
   complete harvested event; bounded-poll the pane; capture.
3. **Line framing** — two-step: controller writes a partial producer-derived line
   (no newline), confirms the write via a file-size stat barrier, captures the
   pane; then writes the terminating newline, stat-confirms, bounded-polls,
   captures again. No sleep-based inference.
4. **Rotation** — controller holds a hardlink to the original inode, atomically
   replaces events.jsonl, stat-confirms both paths, then appends DISTINCT
   harvested sentinel events to the new path and to the old hardlink (each append
   stat-confirmed); bounded-polls the pane after each; captures after each step.

Cleanup is bounded and unconditional; a pane that shows neither sentinel at
timeout is recorded INCONCLUSIVE with its captures, never interpreted by the
worker. Controller-only twins apply to arms 2-4; arm 1 (initial window) is
read-only with no controller mutation and is explicitly EXEMPT from the twin
rule.

## Design 5 — D04a: status pane-set cut (executed by C)

Dedicated tmux server, ≥2 panes. A delegating tmux shim captures the REAL
`list-panes` result, signals `H_STATUS_PANESET`, and blocks before replay; the
controller kills one listed pane and creates one new pane, then releases. Capture
status stdout/stderr/rc and before/after tmux state. Per the global rule, the
shim's inactive equivalence is proven on the same stable topology before its
active barrier is admissible.

**Topology arms (both mandatory):** (a) exact-name only — the target session is
the sole session on the server; (b) prefix-sibling — a second session whose name
extends the target's name as a prefix exists on the same server; identical
manipulation and captures in both. Controller-only twin; trace + manifests.

## Design 6 — D04b: next selection/recheck cut (executed by C)

TWO hooks in the one patch:

- `H_NEXT_SELECTED` — after best-candidate resolution, BEFORE the exact recheck.
  (SC-020b's Batch C arm consumes THIS design — the cut and capture are named
  here, satisfying batch-c-design.md's reuse condition.)
- `H_NEXT_RECHECKED` — after the successful exact `list-sessions | grep -Fx`,
  before the final tmux focus call.

**Caller topology (gate v1 BLOCKER fix):** attach arms run `next --attach` from a
pane INSIDE an attached client on the dedicated isolated server — the harness
attaches a scripted client (pty-wrapped `tmux -S <sock> attach`) to a caller
session first; the intended final verb on this path is `switch-client`. The worker
snapshots `list-clients` (client → session mapping) before the run and after
completion, alongside rc and stdout/stderr.

Arms: (1) controller kills the exact target session between `H_NEXT_SELECTED` and
the recheck; (2) prefix-sibling topology (sibling pre-created), controller kills
only the exact target AFTER `H_NEXT_RECHECKED`; (3) companion arm identical to (2)
with no sibling present. Every controller kill is issued from a SEPARATE
controller client/socket connection — never from inside the client under test.
The default no-attach arm runs outside any client (the frozen outside-tmux verb
is blocking `attach-session`; this arm captures the non-attach outcome via file +
tmux manifests); attach arms capture the client mapping deltas. Controller-only
twins; source/call/flock traces.

## Design 7 — SC-511c: frozen-consumer schema-evolution fixtures (B0 worker
executes)

The consumer matrix is the b0-census.md §A product-reader enumeration — the run is
incomplete unless every family runs: bash list/next, watchdog, request+compact,
archive (facts/request-states/digest/staging), stop verification, telegram
formatter/filter, events-tail, contrib/aewatch (aemonitor is indirect via list).
The verbs here are SCHEMA-KEY mutations over specimen cohorts; the census's
lifecycle fixture matrix (spawn/rename/retire) is general context only and is NOT
this design's mutation set.

**Per-key cohort table (all ten documented stable keys — events.md:49-84).** A
cohort is EVERY specimen line in the fixture that carries the key (whole-cohort
mutation — a key removed from one optional occurrence merely impersonates
omission). Removal = delete the key/value pair from every cohort line; rename =
rewrite the key name to `<key>_x` on every cohort line; both as recorded byte
diffs. Consumers listed are where the census maps the key — the families the arm
must run; captures are those consumers' stdout/rc/files.

| Key | Producer-derived cohort | Discriminating consumers (census §A) |
|---|---|---|
| ts | goal events (real `goal` helper, clock hook) + state events + ask pair | `_goal_set_epoch`; `_ar_event_facts` first/last; `_session_attn_rollup` request aging; watchdog `_last_event_age` |
| actor | state events from two distinct agents + ask/reply pair | `_session_states`; `ae_latest_state_for`; watchdog `_agent_quiet_reason`; `_ar_request_states` sender identity |
| action | mixed cohort: state + goal + ask + reply lines | every reverse-scan selector: `_session_states`, `_goal_set_epoch`, `_ar_request_states`, telegram `event_action_allowed` |
| target | ask/review events + alert events (T-WD precursor bytes) incl. one `@session:agent` form | `_agents_alert_reasons` target routing; `ae_find_request` pairing |
| ref | ask/reply pair + state events (quiet refs) | request pairing (`ae_find_request`, `_ar_request_states`); `_session_states` state ref; watchdog quiet refs |
| summary | state + alert + stop-result lines (stop-result via real stop flow) | `_stop_result_ok` phrase check; alert reason text; archive digest render |
| actor_slot | routed ask/reply pair (helpers emit both routing keys) | `_ar_request_states` / `ae_find_request` identity validation; `_compact_reply_seen` |
| actor_session | same cohort as actor_slot | same consumers |
| target_slot | routed ask/review pair | request target validation |
| target_session | same cohort as target_slot | same consumers |

**Routing-key churn cases (whole-cohort; corrected per gate — `cmd_rename` is the
WRONG construction: it renames the SESSION and its paths, making `actor_session`
STALE rather than holding routing identity fixed):** use the frozen
integration-test shape at tests/integration@72c7293:1268-1285 — real ask/reply
producer bytes, then mutate BOTH panes' `@ae_agent` display names via
`tmux set-option -p` while `@ae_slot` and the session (hence all four routing
keys) stay untouched. Consumers run on pre-churn and post-churn states. (A
stale-`actor_session` cohort is SC-405j's negative case, owned by Batch C A7 —
not this design.)

**Additive arms:** one unknown optional key inserted at FIRST / MIDDLE / LAST
object position — three fresh-clone arms, each running every consumer family.

**body_file:** emitted by `ae_emit_event` but ABSENT from the documented stable
set — a separately-labelled EMPIRICAL EXTENSION lane: cohort = ask/review with a
long body (real helpers); the same removal/rename mutations; captures labelled
`empirical-extension`, never merged into the stable-key lanes.

Capture output/rc/files only; the worker NEVER labels compatible/breaking. Record
controls (unmutated clone per family) and exact byte diffs.

## Design 8 — SC-1208: transport-separation probe (B0 worker executes)

Per the precised row (type-boundary invariant): isolated HOME/AE_HOME + FAKE
claude/codex/gemini/grok/opencode binaries; the REAL frozen launch/injection path,
REAL `_cmd_spawn`, and REAL generated send helper. No live model, no network.

**Env rule (gate v1 BLOCKER fix):** launches run under `env -i` plus the
documented minimum; fake binaries log argv, cwd, stdin bytes, and ONLY an
ALLOWLISTED variable set (AE_*, OPENCODE_CONFIG, PATH, HOME, TERM, the tool's own
documented flags) — never an ambient environment dump, which risks committing real
secrets into evidence.

**Fake-TUI protocol (gate v1 BLOCKER fix):** for the tools whose send path is
TUI-modelled (claude, codex), the fake binary renders an idle input region
EXTRACTED from the real tool's captured idle screen (fixture files, harvested once
and hashed) and keeps reading stdin, logging every byte received. This lets the
real send helper's readiness/staged sensors reach VERIFIED SUBMIT — the probe
captures a delivery, not a defer. The other fakes render a plain prompt line and
read stdin identically.

**Ingress × tool matrix (gate v1 BLOCKER fix — all cells run):** ingress kinds:

1. **spawn-brief body** — the user_prompt argument of a real `_cmd_spawn`. NOTE
   the tool split at 72c7293:11964-69: fresh CODEX receives it as a POSITIONAL
   LAUNCH ARGV value; claude/gemini/grok/opencode receive it post-launch by paste.
   Both transports are captured; argv-borne user text is classified USER_INPUT by
   construction (channel = which artifact carried the byte, a structural fact).
2. **steady-state helper body** — a real `send` (and one `ask`) to the running
   fake, after launch.
3. **pane bytes** — hostile text pre-seeded into the fake's pane output before a
   send, so readiness sensors read over it.
4. **validated spawn name** — a hostile-looking but grammar-valid agent name used
   as the spawn name.

Payloads carry a unique sentinel per ingress cell plus: a nested fake ⟦ae:msg⟧
envelope, instruction prose, flag-looking strings (`--append-system-prompt`,
`-c developer_instructions=`), quotes/backslashes/newlines.

**Captured artifacts, classified by construction (which file/argv/paste carried
the byte — not by expected content):**

- INSTRUCTION — claude `--append-system-prompt` argv value; codex
  `developer_instructions` config value; gemini `-i` value; grok initial
  positional; opencode config + context markdown files (hashed before launch and
  after every delivery).
- USER_INPUT — tmux-pasted message bytes (including the helper envelope) AND the
  codex fresh-spawn positional argv user text.
- DATA — events.jsonl rows and body_file contents.

The worker delivers the artifacts, hashes, and logs per cell. It is NOT told which
channels a sentinel should or should not appear in — that comparison is the seats'
(annex).

**Limit (binding on the evidence and the row):** this proves ae's TRANSPORT
separation, not that any vendor model obeys instruction hierarchy; no artifact or
comment may claim a semantic model-compliance observation.

**Out-of-scope pointer:** the unsupported/other-command launch surface
(ae:1539,1558) is SC-707's code-observation row — not SC-1208 evidence.

## Sequencing

Seat gate per design → SC-507b/511c/1208 to the B0 worker (one worker, fixed
order, own sandboxes) → D01–D04 designs hand to Batch C's worker with the batch.
The T-WD producer precursor stays a separate subdesign (its own gate), related
only as G2's byte source.

---

## SEAT CLASSIFICATION ANNEX — never included in any worker brief

What the seats compare after captures return. This section is the only place
expected relations live.

- **Design 1**: pass-ordinal trace distinguishes one pass / two passes /
  exactly-two-then-stop against SC-507b's stitch contract. LEAK-COMPARE: a
  transient arm's final stdout should match its post-state control's stdout
  (facts and rendered detail from ONE consistent state B) — a mixed old/new
  render or a leaked first candidate is the failure the row forbids. The
  persistent arm's captures locate the bounded-retry fail path. Per-file
  discriminators: meta = roster/agent-count (facts old vs render new); memo =
  memo count/topic vs rendered memo row; events = event count/request row vs
  rendered request/state section.
- **Designs 2/3 (D01/D02)**: reader-effects = (reader+controller delta) minus
  (controller-only delta) — expected empty for a read-only reader; goal text vs
  goal_set_epoch (D01) and request-row set vs appended reply (D02) locate each
  reader's cut relative to the barrier.
- **Design 4 (D03)**: arm 1's surviving numbers locate the initial window (~30);
  arm 2 shows live follow; arm 3 discriminates line-buffered vs raw framing; arm
  4's sentinel provenance (old-inode vs new-path) locates descriptor-follow vs
  path-follow. INCONCLUSIVE artifacts are classified by the seats, never
  discarded.
- **Design 5 (D04a)**: frozen `has-session -t`/`list-panes -t` prefix-match may
  surface a neighbour session in arm (b); any exact/prefix divergence between the
  two topology arms becomes a row/defect, never buried in D prose.
- **Design 6 (D04b)**: arm 1 exercises the selection-vs-recheck window; arm 2 vs
  arm 3 separates prefix-fallback behavior from plain final-call failure on the
  acknowledged non-atomic final switch; client-mapping deltas name the actual
  target the client landed on.
- **Design 7**: a key's removal/rename is BREAKING for a consumer when the
  consumer's output/rc on the mutated clone loses the semantic it showed on the
  control clone; tolerated when output is unchanged or degrades per the
  documented additive rules. Seats label per key × consumer; bucket-3/DR
  reopenings on contradictions.
- **Design 8**: every hostile free-text sentinel (ingress 1-3) must be ABSENT
  from every INSTRUCTION artifact and present byte-exact, with outer provenance,
  in its USER_INPUT (or DATA) artifact; instruction-artifact hashes must be
  unchanged across deliveries; the grammar-valid hostile name (ingress 4) may
  appear in instruction material ONLY inside the fixed identity slot. Any other
  placement fails SC-1208.
