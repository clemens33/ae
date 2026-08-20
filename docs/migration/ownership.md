# State/lock model — ownership and cutover table (rust-rewrite)

Deliverable 2 of #81, part of epic #79. Status: **DRAFT.**

**Grain rule (epic, binding):** ownership is per **logical mutation operation/domain**,
never per file. A logical operation (one command, one transaction) is owned wholly by bash
or wholly by Rust — never straddled. Per-file single-writer is insufficient: it still
permits split transactions across events/meta/requests/claims/tmux with divergent lock
order.

**Event-append rule (ruled 2026-08-20):** `events.jsonl` emission is a **mechanism**, not
an ownership domain. Every operation that appends an event owns that append as part of its
transaction; the shared append protocol (`ae_emit_event`) is documented once below and
cannot flip independently of its calling operations.

**Current vs planned (gate finding fe7cfc2e, blocker 3):** every record separates the
**current writer** (what writes today, at 72c7293) from the **planned owner/fate** (who
owns it after its flip, or that it dies/stays). "Owner: rust" is never written for code
that does not exist yet — unimplemented planned features record `current writer: none`.

**Grouped-cutover rule (gate finding b29dac92, blocker 3):** a record may group multiple
commands ONLY when evidence proves they cut over as one atomic unit — same store, same
lock protocol, compatible effects. Until that evidence is transcribed onto the record, the
grouping is provisional and marked; commands with visibly different effects or fates are
split immediately, no evidence needed.

**Rollback rule (epic):** every cutover is reversible until its phase gate passes; the
bash implementation of a domain is deleted only after the Rust owner survives a full phase
gate; the flip commit names the revert. Default gate for every record below: 811-test
integration suite green + phase acceptance + decision records for divergences; deviations
are stated on the record.

## Record fields (ruled, ae-20260820T075607Z-f0d46071 + fe7cfc2e)

- **effects** — every file AND tmux effect (meta, events.jsonl append, panes, windows, options)
- **current writer/call path** — which script/function actually writes today
- **locks (ordered)** — every lock taken, in acquisition order
- **atomicity boundary** — what is guaranteed atomic / what partial failure leaves behind
- **current owner** — `bash` | `python contrib` | `none` | mode-split (stated explicitly)
- **planned owner/fate** — `rust at P<n>` | `dies at P<n>` | `stays bash` | `stays contrib`, with gate + rollback if non-default

All `TBD` cells are filled from code at 72c7293 (empirical, observation only); where
observed protocol contradicts a doc contract, that becomes a semantic-contract conflict
row, never a silent table entry.

## Shared mechanisms (not domains — never flip alone)

### M1 — event append

`ae_emit_event` / `_event_json_str`: one lock file (`events.jsonl.lock`, fd8), but the
writer is **duplicated at 72c7293 with divergent failure semantics** (colead finding,
lead-verified): `_lib` `ae_log_append` = `flock -w 5 || exit 1` (hard exit, ae:13174);
`_spawn_emit_event` = `flock -w 5 && printf` with unconditional `return 0` (silent event
loss on timeout, ae:12113); an inline copy near retire (ae:12262). (Correction from
colead's citation audit: ae:6275/ae:17294 are NOT event writers — they are
`.lifecycle.<name>.lock` acquisitions that merely reuse fd number 8; attributed under
D17/D18/D22.) A #76-style duplicated-writer defect family + #75 flock dependency — per-writer
IS rows, never one global row. In Rust this becomes ONE library primitive with one typed
return contract (a `Result` — never process-exit, never silent success); what a caller
DOES with a failed append stays a per-operation ruling (authoritative-event ops,
audit-event-after-primary writes, and irreversible tmux delivery need different
partial-success handling — that is where the SHOULD/DR rows land). Never flips alone.

### M2 — pre-dispatch config bootstrap (frozen #61; gate finding b29dac92, blocker 2)

`ae` writes its default config **before any command runs** — a pre-dispatch mutation
every top-level dispatcher entry inherits on a config-less home (`list`, `status`,
`next`, `help`, `version`, `archive preview`, `doctor`, …), not a list-local effect.
Generated session helpers are explicitly OUTSIDE this path (measured contrast: `requests`
does not bootstrap). Atomicity (lead-verified, ae:344-352): `mkdir -p` + direct
`printf > "$CONFIG_FILE"` — UNLOCKED, no temp+rename; crash residue is an absent or
partially written config; the "Created default config" notice goes to stderr precisely
because stdout is a contract for some commands (the code comment names `archive
preview`). Every dispatcher record below carries this effect implicitly until the
intended P1 fix (reads never write; bootstrap moves to an explicit init path — contract
row, bucket 3, #61).

### M3 — executable-artifact publication chokepoint

`_publish_executable_artifact` (generate-to-temp, set mode, rename; generator as command,
never pipe) is a shared mechanism, not an ownership domain: measured call sites publish
`launch.<slot>.sh` (via `write_launch_script`, → D14b) and the machine-global
telegram-daemon script (→ D28) — it does NOT publish the generated session helpers. Each
consuming operation owns its own artifact publication and ports the chokepoint invariant
(normative contract row); the mechanism never flips independently.

---

## Read side (P1)

### D01 — `list [--json]`

- effects: read-only EXCEPT the inherited pre-dispatch bootstrap (M2, frozen #61)
- current writer/call path: `cmd_list` (bootstrap: M2, pre-dispatch)
- locks (ordered): OBSERVED (D01 @ba95a5e, seat-transcribed): `list` takes NO lock.
  M2's own lock chain stays census-owned (the D01 harvest does not speak to it).
- atomicity boundary: OBSERVED (D01 barrier + controller-only twin @ba95a5e,
  seat-transcribed): a reader past the meta-capture cut can emit the OLD goal value
  beside the NEW `goal_set_epoch`/`last_active` after the fd200+event-fd8 goal writer
  completes — a torn read across meta fields is a real observable state; a clean rerun
  sees the new goal. Contract rows: S14 (SC-1306a family).
- current owner: bash
- planned owner/fate: **rust at P1**; M2 removed per #61 intended fix

### D02 — `requests` query (generated helper)

- effects: none (helpers are outside the M2 bootstrap path — measured contrast; contract row)
- current writer/call path: `helper_requests_main`
- locks (ordered): OBSERVED (D02 @ba95a5e, seat-transcribed): the request-scan reader
  takes NO lock. The reply WRITER's lock chain is census-owned — classify it from the
  census, not from D02's directly-harvested controller append.
- atomicity boundary: OBSERVED (D02 barrier + controller-only twin @ba95a5e,
  seat-transcribed): the scan is snapshot-semantic — a reply appended after the scan
  completed leaves the CURRENT invocation's output `pending`; a clean rerun from the
  resulting state reports `replied`. Contract row: SC-1306d.
- current owner: bash
- planned owner/fate: **rust at P1**

### D03 — events queries (`events-tail` helper ONLY; narrowed by B0 preflight ruling
ae-20260820T132251Z-09009761)

- scope note: list/next/requests/archive readers OWN their event reads inside their
  operations (D01/D04/D02/L records), sharing the event-read mechanism per M1 — no
  separate flip; the former TBD-dispatcher-readers umbrella is dissolved
- effects: none
- current writer/call path: `helper_events_tail_main`
- locks (ordered): TBD
- atomicity boundary: TBD (tail vs concurrent append)
- current owner: bash
- planned owner/fate: **rust at P1**

### D04a — `status`

- effects: read-only (+ M2 pre-dispatch bootstrap)
- current writer/call path: `cmd_status`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P1**

### D04b — `next`

- effects: read-only state (+ M2) PLUS tmux focus switch — split from `status` because of
  that tmux effect (grouped-cutover rule)
- current writer/call path: `cmd_next`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P1** (Rust calls the tmux CLI directly — epic end state;
  no bash remainder)

## Write domains (P2)

Request tracking — SPLIT by joint marks ruling (2026-08-20,
ae-20260820T115339Z-9905b9ec): the frozen census DISPROVES the one-group premise
permanently against 72c7293 (withdraw is event-only; reply adds an unlocked lookup;
ask/review traverse target-lock → body → event) — the former D05 umbrella is replaced.
Shared facts: there is NO request table at 72c7293 — request authority is
`events.jsonl` + `messages/*` body artifacts.

### D05a — request initiation (`ask` / `review`)

- effects: deliver first, store body, THEN append the request event (the #66
  delivered-before-logged defect); pane delivery + body file + event
- current writer/call path: `helper_ask_main`/`helper_review_main`, `ae_tracked_send`
- locks (ordered): target fd9 (held for paste, released), then unlocked body write,
  then event fd8
- atomicity boundary: delivered-but-unlogged and body-without-event residues are real
  (#66 rows)
- current owner: bash
- planned owner/fate: **rust at P2**

### D05b — reply

- effects: unlocked request lookup, then the same deliver/body/event sequence
- current writer/call path: `helper_reply_main`, `ae_find_request` (reads events
  UNLOCKED — can race an append), `ae_tracked_send`
- locks (ordered): none for the lookup; then the D05a delivery sequence
- atomicity boundary: the lookup race + the D05a residues
- current owner: bash
- planned owner/fate: **rust at P2**

### D05c — withdraw

- effects: fd8 event append ONLY — `_compact_cancel_outstanding` (ae:5958-5969)
  delegates to send's external `ae:compact` path; an append timeout means the
  withdrawal was never recorded
- current writer/call path: `_compact_cancel_outstanding` → send external path
- locks (ordered): event fd8 only
- atomicity boundary: timeout = unrecorded withdrawal
- current owner: bash
- planned owner/fate: **rust at P2**

### D06 — plain `send` (untracked delivery)

- effects: pane delivery (defer on busy/human-typed, verify submit, fail loud),
  `messages/*` body file, event append (send DOES emit — census-verified)
- current writer/call path: `helper_send_main`
- locks (ordered): target fd9 held for paste → RELEASED → unlocked body write → event fd8
  (census ae:14235–14283)
- atomicity boundary: deliver-or-fail-loud promise (contract rows in S3); crash after
  release leaves delivered-pane + body-without-event residue
- current owner: bash
- planned owner/fate: **rust at P2** — one operation including its tmux calls (Rust calls
  tmux directly)

### D07 — agent state (`state` / `mark-done`)

- `mark-done` is an EXACT alias — it execs the `state` helper with `done` and adds no
  effect of its own; no grouped-cutover question exists (colead correction, verified)
- effects: events.jsonl appends ONLY — `helper_state_main` itself dual-emits on every
  `state done`: the `state` event, then the legacy `done` event. State writes NO meta:
  it is event-sourced (verified: only `ae_emit_event` calls in the function)
- current writer/call path: `helper_state_main` (mark-done = exec shim)
- locks (ordered): event lock fd8 (`events.jsonl.lock`) — acquired TWICE independently on
  `state done` (once per emit)
- atomicity boundary: the two appends are separate lock acquisitions — the `state` event
  can persist while the legacy `done` append times out (`flock -w 5 || exit 1`): torn
  outcome, contract row candidate. Also: the helper prints "Marked …" BEFORE its
  authoritative append (success surface precedes the proof — completion-without-delivery
  class; citation-audit finding)
- current owner: bash
- planned owner/fate: **rust at P2**

### D08 — goal (`goal [text|--clear]`)

- effects: meta `goal=` (locked write), `goal` event append
- current writer/call path: `helper_goal_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2**

### D09 — memo (`memo add`)

- effects: `memo.tsv` append **and memo event append** (gate finding fe7cfc2e, blocker 5)
- current writer/call path: `helper_memo_main`
- locks (ordered): TBD
- atomicity boundary: TBD — tsv row + event are two writes; torn state possible (contract row)
- current owner: bash
- planned owner/fate: **rust at P2**

### D10 — chat (`say`)

- effects: `chat` event append (bridge consumes)
- current writer/call path: `helper_say_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2**

### D11 — `interrupt`

- effects: pane key injection (cancel), optional follow-up delivery, `interrupt` event
- current writer/call path: `helper_interrupt_main`
- locks (ordered): **target lock fd9 → event lock fd8, fd9 HELD across the event append**
  — DIVERGES from D06 `send`, which releases fd9 before taking fd8 (colead finding,
  lead-verified 2026-08-20: `ae_lock_target` with no release before `ae_emit_event`).
  The Rust owner must pick ONE order for the pair — contract row candidate
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2** — one operation including its tmux calls

### D12 — `focus`

- effects: tmux focus switch PLUS a `focus` event append (ae:14649-14653, verified) —
  the "pure tmux glue" rationale was false
- current writer/call path: `helper_focus_main`
- locks (ordered): event fd8 only
- atomicity boundary: tmux selection can land with no event (append after selection)
- current owner: bash
- planned owner/fate: **rust at P2** (seat ruling 2026-08-20: it writes state, so it
  moves with the other event-writing helpers, one operation incl. its tmux calls)

### D13 — codex SID capture (staging + async reconciliation, one transaction)

- effects (two call paths — colead correction, lead-verified): (1) STAGING:
  `helper_register_sid_main` writes `codex.<slot>.sid` UNLOCKED, in-place
  (ae:14819) — it never writes meta, it only reads it; (2) ASYNC RECONCILIATION: a
  separate capture process consumes the staging file (ae:1829ff) and rewrites
  `agent.<slot>` under meta.lock; watchdog recovery may emit a `recover` event
- current writer/call path: `helper_register_sid_main` (staging) + the launch-side async
  capture process (reconciliation)
- locks (ordered): staging = none; reconciliation = meta.lock
- atomicity boundary: staging file can exist unconsumed (process died between);
  unlocked in-place staging write can be torn — contract row candidates. Full sequence
  (citation audit): capture child REMOVES the staging file, then rewrites `agent.<slot>`
  under meta.lock; the watchdog may later reconcile and emit a `recover` event
- current owner: bash (both paths — the domain is the full transaction; it flips whole)
- planned owner/fate: **rust at P2**

### Helper generation (artifact classes D14a/D14b — the umbrella carries NO record id;
joint marks ruling 2026-08-20: an umbrella with no independent field set is not a
record under the operation-grain rule)

**D14a — generated-logic helpers** (send/ask/state/… bodies via declare-f)

- effects: helper scripts under session meta (temp+chmod+mv)
- current writer/call path: SYNC_SESSION_ASSETS_BODY region, declare-f emission
- atomicity boundary: atomic per-artifact (temp+mv)
- current owner: bash
- planned owner/fate: **logic dies at P2** (#76): helpers become thin shims calling the
  binary. Shim TEXT emission stays with the emitting operations' owners — bash launch
  family + `doctor --refresh` through P2, absorbed by D17 op1 (rust) at P3

**D14b — `launch.<slot>.sh` + interactive shims (pane-side artifacts)**

- effects: launch scripts + `.started` markers in session meta, published via M3
- current writer/call path: launch family (`write_launch_script`) + `doctor --refresh`
- current owner: bash
- planned owner/fate: **artifact stays bash** (epic end state: bash keeps pane-side
  artifacts); the GENERATOR flips with its owning operations (D17 launch / doctor),
  porting the M3 invariant

*(former D14c removed — `_publish_executable_artifact` is mechanism M3, owned per
consuming operation, never an independent flip; gate finding b29dac92, blocker 4)*

## Roster + lifecycle (P3)

### D15 — `spawn`

- effects: meta entry, `workspace.md` manifest, tmux window/pane creation, launch script,
  brief delivery (readiness-gated), events
- current writer/call path: `_cmd_spawn` / `helper_spawn_main`
- locks (ordered): TBD (meta lock; event via `_spawn_emit_event`)
- atomicity boundary (citation-audit findings, verified): meta-lock timeout occurs AFTER
  `tmux new-window` — an unregistered live pane with no rollback; `_spawn_emit_event`
  swallows append failure (`return 0`, ae:12113-12114) — spawn can REPORT SUCCESS with no
  event recorded (completion-without-delivery class). Both are contract row candidates
- current owner: bash
- planned owner/fate: **rust at P3**

### D16 — `retire`

- effects: pane kill, meta removal, manifest update, events (inline writer, ae:12262)
- current writer/call path: `helper_retire_main`
- locks (ordered): TBD
- atomicity boundary: event append failure returns NONZERO — but only after pane, meta,
  and artifact mutations already landed (citation-audit finding): the operation fails
  loudly yet leaves its effects
- current owner: bash
- planned owner/fate: **rust at P3**

### D17 — session launch (`ae [name]`, modes, `--from <uuid>`)

- effects: session dir + meta + helpers + workspace.md, tmux session/panes, per-session
  ae-monitor window (`_monitor_ensure_events_pane`), worktree/copy creation, launch
  rollback (`rm -rf` of the validated name), archive inheritance. Launch delivery is a
  direct fire-and-forget paste — NO target lock, NO body, NO event; paste failure is
  IGNORED (ae:12608-12613). Contract-row candidates (census-2 audit): launch-script
  publication failure vs rollback; ignored paste failure = reported success with an
  unstarted agent (completion-without-delivery); deferred Codex-resume prompt failure
  leaves durable evidence (`undelivered.launch-*`, `launch-delivery-failed`) but the
  parent launch already reported success
- current writer/call path: launch family
- locks (ordered): `.lifecycle.<name>.lock` on fd 8 (NOT the event lock — fd number
  reuse), `flock -w 15`, and **degrades to UNLOCKED with a one-line note when flock is
  absent** (ae:17288-17298; #75 material) + TBD
- atomicity boundary: rollback contract on failed launch — TBD
- current owner: bash
- planned owner/fate: **rust at P3 — OPEN-DESIGN, due at P3 entry (narrowed per gate
  finding a1358882):** the launch transaction must split into whole operations at a
  defined interface (direction: Rust `prepare-session` owning state + rollback; a second
  operation owning tmux realization). What the design must cover before any ruling:
  op2's REAL effect set is not just attach — tmux session/window/pane creation, agent
  launch, async SID capture (D13), per-session ae-monitor window
  (`_monitor_ensure_events_pane`), watchdog/telegram/steward hooks; `tmux attach` runs
  after `new-session`, so an attach failure can leave a LIVE session (a "no state writes /
  stopped-but-launchable" claim is false without a rollback matrix); durable
  prepared-state marker + idempotent re-entry + full failure/rollback matrix required.
  Until ruled, no record may assert the boundary. Helpers/roster operations that touch
  tmux are unaffected: they own their tmux calls inside their single (Rust) operation.

### D18 — session end (`ae end` / `ae rm`, `--purge-history`)

- effects: git commit+push to `ae/<session>`, archive capture to `~/.ae/archive/<uuid>/`
  (MANDATORY on keep; ordering: after verified stop + git, before live-state removal;
  failed archive fails the end), session dir removal, tmux teardown, events
- current writer/call path: `cmd_end` → `end_session`
- locks (ordered): `.lifecycle.<name>.lock` on **fd 9** (`flock -w 15 9`, ae:2879-2894,
  verified — the fd 8 block at ae:17288 is the LAUNCH side of the same file);
  degrade-to-unlocked without flock; meta fd200 nested inside (lifecycle → meta,
  census-2); archive publication serializes via a `mkdir -m 0700` UUID claim, not flock
- atomicity boundary: archive-before-removal is the load-bearing promise (contract row).
  end appends NO event of its own; archive staging direct-copies
  `memo.tsv`/`events.jsonl`/`messages/*` while holding the lifecycle lock but NOT the
  writers' locks (ae:5356-5386) — snapshot consistency is an empirical race window,
  its own contract rows
- current owner: bash
- planned owner/fate: **rust at P3**

### D19a — external singular `stop`

- effects: tmux teardown without archive/removal; NO event (census-2 audit)
- current writer/call path: `cmd_stop` (resolves via session lookup, not raw paths — measured)
- locks (ordered): lifecycle lock per census-2; TBD detail
- current owner: bash
- planned owner/fate: **rust at P3**

### D19b — self-supervised stop

- effects: differ from D19a (census-2); split per the grouped-cutover rule
- current writer/call path / locks: census-2 stop section
- current owner: bash
- planned owner/fate: **rust at P3**

### D19c — fleet stop (`stop all`)

- effects: fleet identity mint (refuses without flock, ae:7420-7465), per-session stops,
  best-effort request/result reporting
- current writer/call path / locks: census-2 stop section
- current owner: bash
- planned owner/fate: **rust at P3**

### D20 — `rename`

- effects: meta, tmux session name, session dir move, `.lifecycle.<name>.lock` identity
- current writer/call path: `cmd_rename` (target name strict-validated)
- locks (ordered): BOTH names' lifecycle locks — and then rewrites meta WITHOUT
  `meta.lock` (ae:11597-11667): unserialized against helper meta writers (fd200),
  empirical race window, own contract row
- atomicity boundary: TBD (dir moved but tmux rename fails → ?)
- current owner: bash
- planned owner/fate: **rust at P3**

### D21 — `transfer`

- effects: rsync both directions, SSH probe, dest `mkdir`, name validation both ends
- current writer/call path: `cmd_transfer`
- locks (ordered): TBD
- atomicity boundary: TBD (partial rsync → ?); audit event is BEST-EFFORT — after
  stop+rsync succeed, event failure on either side is warned and transfer still reports
  success (ae:11530-11535): per-direction contract rows
- current owner: bash
- planned owner/fate: **rust at P3**

### D22 — compact / handover (self-continue under same name)

- effects (complete set — gate findings fe7cfc2e b5 + b29dac92 imp2): four-line stdout
  contract, frozen roster, archive capture (source handover memo is captured in the
  archive/digest and REFERENCED by the successor — never copied into the child's live
  memo), request-state disposition, events, predecessor teardown, successor launch
- current writer/call path: `cmd_compact`
- locks (ordered): `.lifecycle.<name>.lock` fd8 `flock -w 15` at the
  revalidate-after-confirmation boundary (ae:6272-6280) + TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3**

### D23 — `_recover-pending` (SID/meta reconciliation — NOT request recovery)

- corrected per census-2 audit: despite the name, this is the recovery half of the SID
  transaction family (D13), not a D05 request operation
- effects: rewrites `agent.<slot>` in meta; the watchdog appends a `recover` event only
  AFTERWARD (ae:8690-8872, ae:16528-16536)
- current writer/call path: standalone `_recover-pending` + the watchdog path
- locks (ordered): meta fd200 (`flock -w 5 || exit 1`, verified ae:8718-8733)
- atomicity boundary: meta rewrite and recover event are separate acquisitions
- current owner: bash
- planned owner/fate: **rust at P2, with D13** (seat ruling: same transaction family
  flips together)

### D24 — claims (#71 invariants)

- current effects / writer / owner: **none — unimplemented; no bash writer exists**
  (the absence is CLASSIFIED, not hidden — seat ruling with colead precision condition,
  2026-08-20)
- planned effects: the seat-owned #71 DURABLE HANDOFF RECORD design, **blocked on an
  explicit pre-build P3 design gate** that must define effects, locks + order,
  atomicity boundary, flip, rollback, and the test oracle before any code
- planned owner/fate: **rust-born at P3**; #71 invariants re-ratified against Rust
  semantics at that gate

### D31 — `doctor --refresh` (own mutation transaction; census-2 audit)

- effects: meta migrations/updates, helper-set publication AND removal, direct
  `workspace.md` rewrite, pane stamping/status, watchdog stop/start, SID recovery
- current writer/call path: doctor refresh family (ae:8536-8680)
- locks (ordered): **NO lifecycle lock spans it** — the D14 artifact classes do not own
  this transaction; per-write locks only (meta fd200 where used)
- atomicity boundary: per-artifact temp+mv; the transaction as a whole is unserialized
- current owner: bash
- planned owner/fate: **rust at P2** (it regenerates the helpers whose logic dies at P2;
  flips with D14a)

## Daemons + control surfaces (P4)

### D25 — watchdog daemon (mode-split; measured, colead 2026-08-20; census-3 audited)

- **default mode (bash):** nudges go through the generated `send` helper — per-target
  lock + busy/human/dead/verified-submit guards; tmux option writes otherwise carry no
  ae lock
- **`AE_WATCHDOG_IMPL=uv` mode (python contrib, aewatch):** holds `aewatch.lock` for
  loop/tick lifetime ONLY (`up`/start orchestration is OUTSIDE the singleton — audit I6:
  concurrent autostarts can race and kill/recreate each other); bounded
  `events.jsonl.lock` append (two failure directions — audit I2); reads meta/config/
  events UNLOCKED against bash writers (partial/mixed-generation reads possible — audit
  I5/I1); delivers via direct `RealTmuxClient.paste`, NO shared target lock, NO guards —
  **fix-known-defect(#45)**: Rust daemons use the one verified delivery primitive;
  heartbeat/backoff temp+replace (atomic visibility, NOT power-loss durability — audit
  I8); logger direct append with lock-free rotation
- planned owner/fate: **rust at P4**; both modes retire together. **Final row fill
  gate: DR-002's five binding conditions + the #83/#84/#88 intended behaviors ARE the written flip-gate requirement (stale B1–B3 blocker text removed 2026-08-20; B1–B3 resolved into #84/#85/DR-001).**

### D26a — watchdog start/stop (`ae watchdog start|stop`, session `watchdog` + `loop` helpers)

- effects: daemon lifecycle mutations (pid/marker files, tmux/options). NOT an M3
  consumer — the session watchdog/loop helpers are emitted via D14a; M3's census names
  launch scripts and the telegram daemon only (gate finding a1358882)
- current writer/call path: `cmd_watchdog`, `helper_watchdog_main`, `loop` shim
- locks / atomicity: PARTIAL SUCCESS CAN LIE (census-2 audit): start/stop mutate
  tmux/pid/options, call fd200 `_set_meta_kv`, IGNORE its failure, exit 0
  (ae:15060, ae:15102) — stale meta with reported success; contract row
- current owner: bash
- planned owner/fate: **rust at P4** (with D25)

### D26b — watchdog status

- effects: NOT read-only — can delete a stale pidfile (ae:14910-14931, ae:15064-15070;
  census-2 audit)
- current writer/call path: `cmd_watchdog` status arm
- current owner: bash
- planned owner/fate: **rust at P4**

### D27 — telegram bridge (runtime handoff; corrected per gate finding fe7cfc2e, blocker 6)

- **ownership is a runtime handoff, not a static split:** when the aewatch marker exists
  AND its heartbeat is fresh, **aewatch owns the bridge operation and the bash daemon
  stands down**; otherwise the bash daemon owns it.
- **known defects (both seats, 2026-08-20):** `ae telegram start` under a live aewatch
  WARNS AND PROCEEDS — double sender (ae:10639-10648): fix-known-defect(**#83**,
  intended: single-sender holds against explicit operator start). The takeover itself is
  **fail-open** — tolerant kill ignored (probe: rc=1 → still ticks), incomplete server
  scope, no control-lock serialization: fix-known-defect(**#84**, intended: prove every
  predecessor absent on the complete scope, serialize control before first send).
  Destructive tmux targets are raw prefix-matchable names: fix-known-defect(**#85**,
  intended: resolve exact session identity before any kill).
- mechanics (census-3 audited): marker/heartbeat atomic-replace visibility (no
  durability promise), freshness/clear unlocked, no shared control lock between the
  handoff and bash start/stop/supervise; `aewatch.lock` is distinct from bash's
  daemon/control locks; shared Telegram stores (`tg_offset`, `state.tsv`,
  `current_target`) have NO common lock — bash EXIT can regress offsets after a
  takeover (audit I3). **Gate: DR-002 conditions + DR-003 + #83-#87 intended behaviors are the written flip-gate requirement (stale blocker text removed 2026-08-20).**
- effects: chat event consumption, Telegram send/receive, reply routing to panes
- current writer/call path: bash telegram daemon; aewatch bridge (mode above)
- locks / atomicity: TBD per mode — the marker+heartbeat handoff itself is a contract
  surface (S10 rows)
- planned owner/fate: **rust at P4**

### D28a — telegram setup (`ae telegram setup`)

- effects: token/config writes ONLY (ae:10550-10607, verified — no publication, no tmux)
- current writer/call path: `cmd_telegram_setup`
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4**

### D28b — telegram start/stop (+ autostart)

- effects: the AUTHORITATIVE config mutation `telegram.enabled` persisted BEFORE
  spawn/kill (ae:10288-10310, ae:10628-10678 — the durable effect and residue; the
  control-lock file is mechanism), machine-global telegram-daemon script publication via
  M3 (ae:9305-9308, ae:10315-10346, ae:10628-10655 — moved here from setup per census-2
  audit), tmux creation, daemon lifecycle
- current writer/call path: `cmd_telegram_start` / `cmd_telegram_stop` / autostart path
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4**

### D28c — telegram status (read)

- effects: none
- current writer/call path: `cmd_telegram_status`
- current owner: bash
- planned owner/fate: **rust at P4**

### D29a — steward scaffolding (`ae steward --init`)

- effects: steward dir scaffolding (`AE_STEWARD_DIR`: steward.config, CHARTER.md — never
  overwrites)
- current writer/call path: `cmd_steward_init`
- current owner: **bash (product surface)**
- planned owner/fate: **rust at P4**

### D29b — steward session launch

- effects: detached steward session launch (isolated config), autostart hook
  (`AE_NO_AUTOSTART` gate). The per-session ae-monitor window is NOT here: it is created
  by `_monitor_ensure_events_pane` on every session launch — a D17 effect (gate finding
  a1358882)
- current writer/call path: `cmd_steward` family
- locks / atomicity: TBD
- current owner: **bash (product surface)**
- planned owner/fate: **rust at P4** — one operation including its tmux calls

### D30a — aesteward templates (static files)

- effects / call path: none at runtime — static templates CONSUMED by D29a (bash) at
  scaffold time; python owns only the source tree
- locks / atomicity: n/a
- current owner: source **python contrib**; runtime consumer bash (D29a)
- planned owner/fate: source **stays contrib**; disposition `stays-python-contrib` on #79

### D30b — python aemonitor (analytics)

- effects: writes its OWN state via fcntl `state.lock` + temp/`os.replace` (citation
  audit) — a distinct writer, but on aemonitor-owned files outside ae's state; excluded
  from the ae censuses by name, contrib census if ever needed
- current owner: **python contrib**
- planned owner/fate: **stays contrib indefinitely** (epic: optional analytics stay Python)

### D30c — aewatch internals

- effects / call path (census-3): daemon loop/once/`up` (up OUTSIDE the singleton),
  singleton/heartbeat/backoff/log/marker writers, event append (`_locked_append`) and
  unlocked event reads (stat/open race — audit I1), Telegram stores + config reads
  (only `$AE_HOME/config`, ignores `CONFIG_FILE`/`AE_LOCAL_CONFIG` — effective-config
  conflict candidate, audit I4), unlocked meta reads (partial reads of direct-append
  writers — audit I5), tmux takeover/mutations (#84/#85)
- current owner: **python contrib**
- planned owner/fate: split fates (gate finding a1358882) — **runtime ownership retires at
  P4** when D25/D27 flip to Rust; **source stays contrib as reference/incubator** per the
  epic (it is the measured spec for the daemon port). **Final row fill blocked until
  gate: DR-001/DR-002 conditions + #84-#88 intended behaviors are the written requirement (stale blocker text removed 2026-08-20).**
