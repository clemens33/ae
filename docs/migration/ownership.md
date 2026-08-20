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

`ae_emit_event` / `_event_json_str`: TBD (lock target, serialization, JSON escaping,
control-byte strip). Ported as a library primitive inside the Rust owner of each calling
operation; never flips alone.

### M2 — pre-dispatch config bootstrap (frozen #61; gate finding b29dac92, blocker 2)

`ae` writes its default config **before any command runs** — a pre-dispatch mutation
every top-level dispatcher entry inherits on a config-less home (`list`, `status`,
`next`, `help`, `version`, `archive preview`, `doctor`, …), not a list-local effect.
Generated session helpers are explicitly OUTSIDE this path (measured contrast: `requests`
does not bootstrap). Every dispatcher record below carries this effect implicitly until
the intended P1 fix (reads never write; bootstrap moves to an explicit init path —
contract row, bucket 3, #61).

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
- locks (ordered): TBD (incl. what torn states readers may observe)
- atomicity boundary: snapshot semantics mid-write — TBD, becomes contract rows (S14)
- current owner: bash
- planned owner/fate: **rust at P1**; M2 removed per #61 intended fix

### D02 — `requests` query (generated helper)

- effects: none (helpers are outside the M2 bootstrap path — measured contrast; contract row)
- current writer/call path: `helper_requests_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P1**

### D03 — events queries (`events-tail` helper, dispatcher event reads)

- effects: none
- current writer/call path: `helper_events_tail_main` + TBD dispatcher readers
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

### D05 — request tracking (`ask` / `review` / `reply` / withdraw)

- **grouping provisional** (grouped-cutover rule): stays one record ONLY if evidence
  proves one shared store + one lock protocol + one atomic cutover; else splits
- effects: request records, events.jsonl appends, pane delivery
- current writer/call path: `helper_ask_main`/`helper_review_main`/`helper_reply_main`,
  `ae_tracked_send`, `ae_find_request`, slot verification
- locks (ordered): TBD (flock serialization on send + request store)
- atomicity boundary: TBD (request logged vs delivered vs replied; each #66 finding lands
  as its own contract row)
- current owner: bash
- planned owner/fate: **rust at P2**

### D06 — plain `send` (untracked delivery)

- effects: pane delivery (defer on busy/human-typed, verify submit, fail loud), events —
  TBD whether send emits
- current writer/call path: `helper_send_main`
- locks (ordered): TBD (flock on pane delivery)
- atomicity boundary: deliver-or-fail-loud promise (contract rows in S3)
- current owner: bash
- planned owner/fate: **rust at P2** — one operation including its tmux calls (Rust calls
  tmux directly)

### D07 — agent state (`state` / `mark-done`)

- **ruled exception to the grouped-cutover rule:** `mark-done` is a shim over `state done`
  whose one extra effect is the legacy `done` event append — same store, same call path,
  one operation family; it cuts over with `state` or not at all
- effects: meta, events.jsonl append (incl. legacy `done` event via mark-done shim)
- current writer/call path: `helper_state_main` + mark-done shim
- locks (ordered): TBD
- atomicity boundary: TBD
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

- effects: pane key injection (cancel), optional follow-up delivery; events TBD
- current writer/call path: `helper_interrupt_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2** — one operation including its tmux calls

### D12 — `focus`

- effects: tmux focus switch only (no state write — TBD verify)
- current writer/call path: `helper_focus_main`
- locks (ordered): none expected — TBD
- atomicity boundary: n/a
- current owner: bash
- planned owner/fate: TBD — candidate **stays bash** (pure tmux glue)

### D13 — `_register-sid` (codex post-launch sid capture)

- effects: meta write (session id), events TBD
- current writer/call path: `helper_register_sid_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2**

### D14 — helper generation (split by artifact class; gate finding fe7cfc2e, blocker 3)

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
- locks (ordered): TBD
- atomicity boundary: TBD (pane created but meta write fails → ?)
- current owner: bash
- planned owner/fate: **rust at P3**

### D16 — `retire`

- effects: pane kill, meta removal, manifest update, events
- current writer/call path: `helper_retire_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3**

### D17 — session launch (`ae [name]`, modes, `--from <uuid>`)

- effects: session dir + meta + helpers + workspace.md, tmux session/panes, per-session
  ae-monitor window (`_monitor_ensure_events_pane`), worktree/copy creation, launch
  rollback (`rm -rf` of the validated name), archive inheritance
- current writer/call path: launch family
- locks (ordered): `.lifecycle.<name>.lock` + TBD
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
- current writer/call path: `cmd_end`
- locks (ordered): `.lifecycle.<name>.lock` + TBD
- atomicity boundary: archive-before-removal is the load-bearing promise (contract row)
- current owner: bash
- planned owner/fate: **rust at P3**

### D19 — `stop`

- effects: tmux teardown without archive/removal — TBD exact meta/state writes
- current writer/call path: `cmd_stop` (resolves via session lookup, not raw paths — measured)
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3**

### D20 — `rename`

- effects: meta, tmux session name, session dir move, `.lifecycle.<name>.lock` identity
- current writer/call path: `cmd_rename` (target name strict-validated)
- locks (ordered): TBD
- atomicity boundary: TBD (dir moved but tmux rename fails → ?)
- current owner: bash
- planned owner/fate: **rust at P3**

### D21 — `transfer`

- effects: rsync both directions, SSH probe, dest `mkdir`, name validation both ends
- current writer/call path: `cmd_transfer`
- locks (ordered): TBD
- atomicity boundary: TBD (partial rsync → ?)
- current owner: bash
- planned owner/fate: **rust at P3**

### D22 — compact / handover (self-continue under same name)

- effects (complete set — gate findings fe7cfc2e b5 + b29dac92 imp2): four-line stdout
  contract, frozen roster, archive capture (source handover memo is captured in the
  archive/digest and REFERENCED by the successor — never copied into the child's live
  memo), request-state disposition, events, predecessor teardown, successor launch
- current writer/call path: `cmd_compact`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3**

### D23 — `recover-pending`

- effects: TBD (request-recovery writes)
- current writer/call path: `cmd_recover_pending`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3** (grouped with D05's domain if evidence shows shared
  store + locks — grouped-cutover justification required otherwise)

### D24 — claims (#71 invariants)

- effects: TBD (design per #71 DURABLE HANDOFF RECORD, re-ratified against Rust semantics)
- current writer/call path: **none — unimplemented; no bash writer exists**
- current owner: **none**
- planned owner/fate: **rust-born at P3**; brief re-ratified before build

## Daemons + control surfaces (P4)

### D25 — watchdog daemon (mode-split; measured, colead 2026-08-20)

- **default mode:** current owner = **bash** (watchdog loop in ae)
- **`AE_WATCHDOG_IMPL=uv` mode:** current owner = **python contrib (aewatch)**
- effects: nudge delivery to panes, nudge counters, footprint exclusion, quiet-state honoring
- locks / atomicity: TBD per mode
- planned owner/fate: **rust at P4**; both modes retire together

### D26a — watchdog start/stop (`ae watchdog start|stop`, session `watchdog` + `loop` helpers)

- effects: daemon lifecycle mutations (pid/marker files). NOT an M3 consumer — the
  session watchdog/loop helpers are emitted via D14a; M3's census (lines above) names
  launch scripts and the telegram daemon only (gate finding a1358882)
- current writer/call path: `cmd_watchdog`, `helper_watchdog_main`, `loop` shim
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4** (with D25)

### D26b — watchdog status (read)

- effects: none
- current writer/call path: `cmd_watchdog` status arm
- current owner: bash
- planned owner/fate: **rust at P4**

### D27 — telegram bridge (runtime handoff; corrected per gate finding fe7cfc2e, blocker 6)

- **ownership is a runtime handoff, not a static split:** when the aewatch marker exists
  AND its heartbeat is fresh, **aewatch owns the bridge operation and the bash daemon
  stands down**; otherwise the bash daemon owns it.
- effects: chat event consumption, Telegram send/receive, reply routing to panes
- current writer/call path: bash telegram daemon; aewatch bridge (mode above)
- locks / atomicity: TBD per mode — the marker+heartbeat handoff itself is a contract
  surface (S10 rows)
- planned owner/fate: **rust at P4**

### D28a — telegram setup (`ae telegram setup`)

- effects: credential/config writes; machine-global telegram-daemon script publication (M3)
- current writer/call path: `cmd_telegram_setup`
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4**

### D28b — telegram start/stop

- effects: daemon lifecycle (pid/marker files)
- current writer/call path: `cmd_telegram_start` / `cmd_telegram_stop`
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

- effects / call path / locks / atomicity: TBD (optional sidecar, own process)
- current owner: **python contrib**
- planned owner/fate: **stays contrib indefinitely** (epic: optional analytics stay Python)

### D30c — aewatch internals

- effects / call path / locks / atomicity: TBD per mode (see D25/D27)
- current owner: **python contrib**
- planned owner/fate: split fates (gate finding a1358882) — **runtime ownership retires at
  P4** when D25/D27 flip to Rust; **source stays contrib as reference/incubator** per the
  epic (it is the measured spec for the daemon port)
