# Changelog

All notable changes to this project will be documented in this file.
## [v2026.9.1] - 2026-09-04

### Bug Fixes

- **gates**: The bash lint and format lanes go green, and the linter is pinned
- **glue**: Every pane kill is guarded by a fail-closed ownership check; suites stop inheriting the pane's CONFIG_FILE
- **watchdog**: The pidfile is released by Drop, so every exit after publish releases it
- **deliver**: A paste that fails after the load deletes what it staged
- **spawn**: A profile selected at spawn passes the one-simple-command lexer before any effect
- **launch**: Record the glue as ae_path, render the window glyphs, restore the saved roster on resume
- **core**: Close the four glue-cut-2 gaps in launch, stop and compact
- **stop**: The fleet form confirms from every caller, --self derives its name, every stop is recorded
- **watchdog,doctor**: Start runs the shim without a stray word; doctor reports the core path and the glue's bash
- **core**: The three dead links the docs pass found
- **core,glue**: The three dead links, six review findings, and the glue path leaves meta
- **wrapper**: A set-empty server half is a declared server, not an absent one
- **run**: Take the start marker back when the exec did not happen
- **launch**: Every rollback announces itself, through one helper
- **run**: A recorded id is the resume target for every tool
- **run**: One environment prefix, one durable marker, one grammar
- **run**: Quoting decides assignment-shape, and both lexers read it
- **z3**: Launch-plan usage errors exit 2, and one env is peeled, not a run of them
- **z3**: The shape is the executable's position, and a foreign HOME is refused
- **z3**: Every effectful invocation passes the install gate
- **z3**: Every execution boundary names the resolved core

### Documentation

- Current surfaces read as ae, not as a migration project
- Slim the README intro and drop restated architecture
- Record the #79 ruling — destination B, the ae-dev namespace, the roll-forward cycle
- Helpers and the wrapper contract describe the core-required glue (post A.1)
- Name the parallel and domain-scoped test lanes
- **development**: Show the fast test lanes beside the serial commands
- The JSON sink is core-owned; _json_escape no longer exists
- The agent-name grammar lives in the core
- **internals**: Preserve the stop self/target identity contract as the glue's stop arm becomes a passthrough
- **watchdog**: The daemon's module doc no longer claims the recovery stays in bash
- The glue cuts, recorded where the docs claimed bash
- **glue**: The server-kind comments state the refusal the core makes, not a fallback it no longer has
- **core**: The pane runs a command, not a script — and one flagged conflict
- **z3**: The suite and the docs describe the symlink install
- **readme**: The helpers are links to the binary, not bash scripts

### Features

- **roles**: Lead and colead are equal leadership peers under lead-pair
- **identity**: Core reads the alias-free v2 roster (additive, read-only)
- **identity**: P2 primitives — v2 config reader, one-simple-command lexer, v2 roster render/migrate
- **identity**: Alias-free agent identity v2 — core-owned roster, bare names, spawn/retire cutover
- **core**: List computes liveness, branch and attention itself (slice A.2a, Rust side)
- **core**: Next/jump — pick the session that needs you, in the core (A.2a follow-up)
- **core**: The watchdog pane and the workspace renders are core-owned (A.3 and A.2c, Rust side)
- **core**: The core delivers to the pane itself; `_interrupt` joins it (B move 1, Rust side)
- **core**: Spawn and retire are whole core operations (B move 2, Rust side)
- **core**: Launch, resume, end, stop and compact are whole core operations (B moves 3 and 4, Rust side)
- **core**: The capture entry answers for every tool, and spawn drives it
- **capture**: The core captures session ids for codex, opencode and gemini; spawn forks its own capture
- **core**: The watchdog and the telegram bridge are the core's to start and stop
- **daemons**: The core owns the watchdog and telegram lifecycle and starts both companions at launch
- **core**: Doctor, rename and the dependency gate are the core's
- **doctor**: The core owns doctor, doctor --refresh, the dependency check, shim rendering and rename
- **watchdog**: The core recovers pending tool session ids itself
- **watchdog**: The core recovers pending tool session ids in-process
- **core**: The core is the entry — the preamble, the launch fall-through, the refusals
- **run**: A resuming run says so before it becomes its tool
- **z3**: The core IS the public ae — shape, doors, upgrade
- **z3**: The public ae is the core, and the install layout says so

### Miscellaneous

- Drop a measured-timings artifact that is not this slice's file

### Other

- Merge branch 'z1-core'
- Merge branch 'z1-wrapper'
- Merge branch 'z1-serverpair'
- Merge branch 'z2-core'
- Merge branch 'z2-suites'
- Merge branch 'main' into z2-resume
- Merge branch 'z2-resume'
- Merge branch 'z2-fix'
- Merge branch 'z3-core'
- Merge branch 'z3-usage'
- Merge branch 'z3-install'
- Merge branch 'z3-fix'

### Refactoring

- **glue**: The core is required — every no-core bash fallback is deleted (slice A.1)
- **glue**: `list` is the core's; doctor names unbound sessions; a refresh never clears a pin (A.2a glue)
- **glue**: Next, the workspace renders and the watchdog run body are core execs (A.2c, A.3, next glue)
- **glue**: Send delivery, interrupt and the spawn/retire bodies are the core's (B glue cut 1)
- **glue**: Cut 2 — launch, end, stop and compact route to the core; helper templates and transfer deleted
- **glue**: Final cut — every arm with a core entry is a core call, every callerless body is gone
- **glue**: Pass 3 — status and the orchestrator scaffold are cut, both arms refuse
- **glue**: Pass 4 — the recovery arm and the last session-state readers are gone
- **wrapper**: Delete ae-glue; ae-entry is the whole of ae's Bash
- **core**: Session helpers become links to the core; the pane runs `_run`
- **run**: Choose the resume arm before injecting it, not after

### Testing

- Suites refuse bash 3.2 and clear every pane-exported ae variable (slice A.0)
- **it**: Sc_017p waits for the pane to exec its command before asking the world
- Parallel sharded integration runner, domain selection, fast unit default
- **z2**: Re-aim the bash suites and docs at links and `_run`
- **z2**: A fixture must not write through a helper link
- **z2**: #27 tests the probed resume, and both of its answers
- **z2**: #27 re-runs from the pane's own directory, which is what the probe reads
- **z2**: Refresh itest section timings from a green full pass
- **z3**: Bind the integration suite to the core and fix the shape's positional test
- **z3**: The Rust-owned sections bind the core directly, not a sibling of it
- **z3**: Refresh itest section timings from a green full pass
## [v2026.8.2] - 2026-09-01

### Bug Fixes

- Fixes
- Fix config cache path security: use XDG_RUNTIME_DIR over /tmp
- Fix Claude staged-paste delivery, remove heartbeat helper

Extract ae_submit_pasted_message() shared helper for send/interrupt and
add Claude staged-paste detection ([Pasted text #N +M lines] token)
with an extra Enter keystroke. Apply the same fix to tmux_paste_submit
used during agent launch. Remove the heartbeat helper entirely — it was
opt-in, underused, and its nudge logic added maintenance surface without
clear value. Update AGENTS.md and README.md to match.
- Fix silent exit on session resume when meta has no loop= line

The PRESERVED_LOOP grep pipeline used set -e + pipefail, so when the
meta file existed but had no loop= entry, grep returned 1, pipefail
propagated, and the script exited silently after printing "Resuming
session..." — never reaching tmux_attach.

Add `|| true` to swallow grep's no-match exit code. The empty result
was already handled correctly downstream.
- Fix codex/gemini/opencode pending session id recovery

Two fixes for the case where post-launch session ID capture fails on
the initial launch and the slot stays "pending" forever:

1. On resume, re-run capture for any slot whose stored session id is
   still "pending". Previously capture only ran on fresh start, so a
   missed initial capture meant the agent could never be resumed.

2. ae doctor --refresh now also recovers pending session IDs offline
   by scanning the agent's local session files (no live pane needed):
   codex via launch-token + CWD scan, gemini via local chat history,
   opencode via session DB. Updates meta atomically under flock so
   the next launch generates a proper resume command.

The next ae <session> after recovery picks up the captured ID from
meta and produces a real resume command instead of a fresh start.
- Fix extract_binary_from_cmd launcher flags + events-tail JSON parsing

extract_binary_from_cmd now tracks per-launcher option-argument flags
so command lines like "sudo -u alice codex --yolo" correctly resolve
to "codex" instead of "alice". Previously the function only skipped
the launcher word itself ("sudo") but treated "-u" as a generic flag
and "alice" as the binary. Each known launcher (env, sudo, nice,
ionice, time) now declares which of its flags take an option argument,
and the walker skips both the flag and its arg when matched.

events-tail now uses a character-by-character JSON string parser
(_extract_json_str) instead of a sed regex with [^"]*. The sed
approach broke on summaries containing escaped quotes (\") because
sed terminates at the first literal " regardless of preceding
backslash. The new parser recognizes ae_emit_event's exact escape
set (\\, \", \n, \t, \r) and unescapes inline, so summaries like
'he said \"hi\" and path C:\\tmp' now display correctly.

Both fixes were verified against the failing inputs codex reproduced
during round 3 review. 135 tests pass.
- **list**: Never truncate ae list output when a per-session probe fails
- **transfer**: Steward guard both directions; preserve unresolved workers
- **watchdog**: Pane-gate _watchdog_is_running — stale recycled pids never report running
- **watchdog**: Emit _agent_alert_reason via _lib — post-restart reconcile was dead code
- **aewatch**: Migrate test_20 recover to session-keyed form
- **aewatch**: Watchdog threads the per-session -L server to every tmux call
- **ae**: Comms-guard hardening — NBSP idle normalize + dead-shell descendant walk
- **ae**: Idempotent watchdog start — dedup the status-right health indicator
- **ae**: Content-keyed config cache — kill same-second-rewrite poisoning
- **ae**: Focus-free sends + uncached config — colead review folds
- **ae**: Copy global status-format[0] to session scope — array-shadowing blank bar
- **ae**: Steward-hardening — delivery-checked sweep nudge
- **ae**: Cut ⚙ subprocess-activity from the roster — the sensor cannot mean it
- **ae**: Input-region — parse SGR state, capture to cursor, strip anchor only
- **ae**: Input-region — structural prompt selection, ESC advance, restore tests
- **ae**: Input-region — structural claude selection against real 2.1.209 bytes
- **ae**: Input-region — bound the input by the box's chrome, not the cursor
- **ae**: Input-region — identify the border positively; route spawn through the sensor
- **ae**: Spawn reports failure when the brief is not delivered
- **ae**: Route tmux option writes by ID — stop cross-session clobber
- **ae**: Grok resume UUID capture + Grok-complete flag normalizer
- **ae**: Strip Grok attached short session flags (-sUUID/-rUUID)
- **ae**: Resolve `ae end` target before the destructive confirm
- **ae**: Guard _end_target_class against an empty target
- **ae**: Fold cross-model review — exact-id end targets, dot guard, truthful flags
- **ae**: Sweep the end path for the raw-name/wrong-server class (delta review)
- **ae**: Fold 2nd delta review — target-owned tmux server, truthful no-remote prompt
- **ae**: Fold 3rd review — pin the default socket, verify kills, preserve no-remote work
- **ae**: Fold 4th review — record the real launch server, tri-state kill verify
- **ae**: Fold 5th review — socket-path identity, legacy-empty fail-closed + migration
- **ae**: Fold 6th review — typed socket selectors, unconditional legacy fail-closed
- **ae**: Fold 7th review — stored kind key, pid-verified sockets, no guessed teardown
- **ae**: Safety hotfix — isolate assume-stopped fixture, fix _lib kind-source typo
- **ae**: Fold 9th review — tri-state sweep, clause (c), central ambiguous refusal
- **ae**: Fold 10th review — bare sweep, anchored clean-dead, kind-first shim
- **ae**: Fold 11th review — ENOENT is not death, lifecycle lock spans proof to cleanup
- **ae**: Lifecycle lock release + flock-optional degradation (12th review fold)
- **ae**: Migration adds missing tmux_server line; launch meta write fully atomic
- **ae**: MacOS/BSD userland is first-class — portability shims + the silent-failure sweep
- **ae**: Generated helpers name their interpreter — /bin/bash 3.2 could not parse them
- **ae**: Stale re-exec marker recovers via attempt counter; portability lint learns real flag grammar
- **ae**: Summary caps are character-safe in every locale; event writers sanitize UTF-8 at the boundary
- **ae**: Lifecycle lock covers the whole launch — end can no longer delete a session mid-creation
- **ae**: $TMUX is verified, not trusted — an inherited copy no longer hijacks attach
- **ae**: Launch scripts re-run, artifacts publish atomically, and failures take their debris with them
- **ae**: DEFAULT_CONFIG mirrors config.sample — completes f07ec5f
- **ae**: Stop resolves, verifies and destroys under one identity contract
- **ae**: End stops the session before it snapshots the work
- **ae**: Task delivery waits for a tool that can act, not one that has drawn a box
- **ae**: A message that cannot be delivered whole is refused, not reported sent
- **ae**: Spawn delivers its brief without taking focus
- **ae**: The watchdog stops counting its own footprints as activity
- **ae**: A resumed pane reports the tool again — the resume decision moves before exec
- **ae**: The re-run form is built from transported facts, not recovered from the command
- **ae**: A declared quiet state survives the agent's own last message, and nudges are counted only when delivered
- **ae**: The whole launch family classifies through one definition, so an env-prefixed agent is a first-class agent
- **ci**: Break the push-cancel livelock — cancel only PR runs, filter to rust paths
- **gate**: Just check is green on a clean tree and the lint cannot wedge or be fooled
- **evidence**: .gitattributes -text so recorded hashes survive clone
- **list**: Read the session once, at discovery — criterion 14 binds the whole phase
- **list**: Preserve the read outcome, and close the one-read class behaviourally
- **list**: Presentation holds no address, and the boundary is a production type
- **list**: Restore the deleted agent-liveness tests; enter presentation on the real route
- **509c**: An agent's attention reason is the newest thing the ledger says about it
- **509c**: Alert currency follows the ledger's order, not a writer's clock
- **510c**: A declared state is the last one appended, not the best-stamped one
- **405f**: The goal epoch is the last appended goal, and the boundary I documented was wrong
- **518**: The matcher takes the ruling — strict identity, and causality is its own dimension
- **rust**: Ship usable session listing
- **requests**: A slotless cancel withdraws the request it names, as compact's does
- **watchdog**: Serialize start so concurrent starts spawn exactly one daemon
- **send**: Bracketed paste for Claude panes — no more head-truncated deliveries
- **send**: Publish the body before the paste — one immutable file per delivery
- **attach**: Honour the named tmux server on attach — exec bypassed the shim
- **tmux**: A separator tmux 3.4 does not escape — a present pane is never "hard dead"
- **send**: An OSC sequence is not text — idle Claude read as busy
- **coexistence**: Eight pre-flip hardenings — the advertised install works, the version pair is gated everywhere

### Documentation

- Scaffold MkDocs Material site under docs/
- Address codex NITs from review-...-900d1be0
- Add diagrams for lifecycle, call graph, event flow, throttle state
- Bridge protocol — substrate contract for chat bridges
- **telegram**: Correct recovery note — inbound IS queued by Telegram
- **telegram**: Hub-centric routing — talk to the meta-agent, not N sessions
- **hub**: Cross-link ae hub ↔ telegram hub-centric routing
- **index**: Surface the ae hub + Telegram fleet-routing pattern
- **agents**: Add bash-hazards checklist (interpreted sinks + set -e footguns)
- Declare-f pattern, revisit triggers, and test-section rewrite
- **readme**: Reflect the new reality — tiers, steward, companions, honesty
- Currency sweep — slot identity, send delivery guards, aewatch backends, framing cleanup
- Doctrine distillation — gatekeeping craft, design patterns, lead handover + steward charter tuning
- **ae**: Correct why the set -e probe exists — masking is $(), not missing errexit
- **ae**: Bash hazards — only a bare call proves set -e safety
- **ae**: Fold retro v1 into doctrine — taxonomy rows, patterns 11-13, trust map
- **ae**: Currency pass — status-bar feature bullet, exact-resume wording, model bump
- **ae**: README + config currency — modern sample, self-documenting default, repo-visible config.sample
- **ae**: DEFAULT_CONFIG banner — narrow the resume-applies claim to what actually reloads
- **readme**: Structural gut — front door, not the reference manual
- **ae**: Gate rulings inline — clause (c) ratified, server-generation residual accepted
- **ae**: Gatekeeping folds from the portability campaign
- **ae**: Revisit-note learns the 2026 multiplexer field — herdr is a watchlist item, not a migration
- **ae**: Grok --system-prompt compat alias carries the same override hazard
- **ae**: Multiple identities of one CLI — the CLAUDE_CONFIG_DIR pattern under [agents]
- **ae**: Promoted tiers — chores run luna at FULL effort, dev on opus5 xhigh, review on gpt56sol xhigh
- **gatekeeping**: The specimen must come from the layer the code reads
- **gatekeeping**: A fact built upstream is transported, never re-parsed
- **gatekeeping**: A refusal path is a guard and owes the same proof of failure
- **gatekeeping**: A delete proves as much as a write
- **migration**: P0 semantic-contract and ownership drafts, plus issue-disposition proposal
- **migration**: Lock/atomicity census of ae at the bash freeze
- **migration**: Event append is a duplicated-writer family, state is event-sourced, interrupt diverges on lock order
- **migration**: Request domain has no table, SID capture is a two-process transaction
- **migration**: Citation-audit batch — lifecycle locks are not event writers, spawn/retire/state failure semantics recorded
- Rewrite direction in README, VISION.md, and the Rust-era AGENTS.md overlay
- **migration**: Lock/atomicity census 2 — lifecycle, daemons, controls, bootstrap
- **migration**: Census-2 audited batch — launch delivery is unguarded, control surfaces can lie, twelve grain corrections
- **migration**: Second-gate corrections — #83 cited on D27, telegram enabled-intent effect, false-diagnosis lock row
- CI wording upgraded against green run 32350969851; census race-precision fix
- **migration**: Census 3 — aewatch sidecar locks and the cross-language contention surface
- **migration**: S6 row batch — stdout/stderr/exit contracts, SHOULD frozen from doc contracts
- **migration**: S6 rewritten per gate — 20 rows, commands.md/events.md as the missed normative sources
- **migration**: S6 ratified — 31 rows classified by both seats, SC-508 held unclassified
- **migration**: S9 row batch — lifecycle transactions, 24 rows at final grain
- **migration**: S13 row batch — identity and provenance, ten rows from the #59 ruling
- **migration**: Census-3 audited batch — fail-open takeover (#84), prefix-match kills (#85), append-only conflict surfaced
- **migration**: S9 + S13 gate folds and DR-001 ratified
- **migration**: S9 ratified after mechanical batch; DR-001 consistency resolved via SC-900
- **migration**: S8 + S12 row batches — adapter rules and platform degradation, #75 anchored
- **migration**: S10 transcribed from the source batch — 60 claims bucketed, DR-002 in the register, #86/#87 opened
- **migration**: S10 mechanically expanded to schema — 78 rows, DR-003 ratified, #88 opened
- **migration**: S8/S12 anti-oracle fold, dispositions completed to full coverage, DR-004 reserved
- **migration**: Gate delta folded — DR-004 ratified now, adapter outcomes frozen, restart and dedupe made mechanical
- **migration**: DR-005 ratified — exact identity or loud refusal; the S8 contradiction resolves
- **migration**: Closure evidence map — 132 placeholders, 109 mapped, 23 probe specs
- **migration**: D24 absence classified, not hidden — pre-build P3 design gate replaces effects TBD
- **migration**: Final family drafts — helpers, config, formats, tmux, installer, locking, env, modes
- **migration**: Final-family gate folded — grain completed, stale-docs defect retracted, DR fan-outs recorded
- **migration**: Second family gate folded — every id owns a head, conflict states consistent
- **migration**: Sweep-check — the canonical-set checker, proven able to fail on all seven arms
- **migration**: Sweepgen-gate contract fixes — documented signatures split from code-observed refusals, groups atomized, SC-401b joint text
- **migration**: Family-gate-3 folded — DR-006 recorded, signatures at behavior grain, register-sid outcome-level, transfer four-way
- **migration**: S10 fields transcribed per-row, checker delta landed, surface gaps closed
- **migration**: S1MAP declaration table — every dispatcher surface names its covering rows
- **migration**: S1MAP gate folded — singular stop gets its real contract, list/status/use/jump surfaced
- **migration**: S1MAP confirmation fold — status and self-stop at behavior grain, full-inheritance alias ruling, anchors re-verified
- **migration**: Final pre-regeneration batch — stop safety boundary, attach behavior, history precedence, two-model alias ruling
- **migration**: Sweep-check delta 6 — fence state enforced, empty declarations fail
- **migration**: #89 dispositioned — dead-code coverage arrives by construction
- **migration**: Closure map regenerated against the canonical 440 — set equality exact
- **migration**: Map header amended — planned routes, not proven evidence
- **migration**: #90 dispositioned — doctor-owned socket sweep at the rust doctor phase
- **migration**: Pin audit — 24 PROVES, 24 PARTIAL, 54 FALSE of 102 real pins; 338 routed to probe design
- **migration**: Empirical-status amendment — the binding ratification split
- **migration**: Ratification-critical manifest — 302 critical, 114 deferrable, 24 observed
- **migration**: Manifest amended — the D-record type error corrected, 314 critical
- **migration**: Census audit — 14 records close on existing evidence, D05 split map ready
- **migration**: Census-audit gate structural batch — D05 split executed, D14 de-recorded, honest arithmetic
- **migration**: S1 preflight MARK — status/list/next rows carry both seats
- **migration**: Remaining-ID manifest regenerated and the probe cluster plan drafted
- **migration**: D-aware checker, repaired map, and the exact 299-line assignment table
- **migration**: Fifteen assignment reroutes from the table gate
- **migration**: Assignment gate armed by default — distinct input, red-not-dormant, strict fields
- **migration**: Slice-1 seat verdict transcribed — 18 new both-seats rows from building real code
- **migration**: Table verdict fixes — nine delivery/identity reroutes, guard closed, slice-1 rows integrated
- **migration**: Fold re-gate corrections — nesting untangled, arithmetic mechanical, grain and authority honest
- **migration**: Section-label nit — 17 rows after the split
- **migration**: Batch C design — nine producer-harvested fixtures, 59 arms in nine groups
- **migration**: Batch C design v2 — six gate blockers folded
- **migration**: Batch C v3 — batch boundary exact, six discriminators, shim contract
- **migration**: Slice-1b verdict executed and Batch C boundary reopened to 62
- **migration**: SC-405j precised after the builder's premise correction
- **migration**: SC-405j re-marked on the precised text; its C arm becomes the four-case set
- **migration**: B0 preflight rulings executed; #92 dispositioned
- **migration**: B0 design draft, census evidence, SC-521 split, SC-1208 precision
- **migration**: B0 design v2 — value-blind split, ingress matrix, SC-707
- **migration**: B0 v2.1 — churn construction corrected, topology explicitness
- **migration**: B0 v2.2 — Design 7/8 deltas green all eight; T-WD precursor draft
- **migration**: T-WD precursor v2 — fake agent, phrase-driven throttle, two-phase recovery
- **migration**: T-WD precursor citation corrections, marked approved
- **migration**: Batch C spawn folds — seat annex, SC-521a, 1306 mapping, prereqs satisfied
- **migration**: Slice-1d — SC-405j presence rule, SC-510e/f duplicate-key heads
- **migration**: SC-405j scoping clarification — reader-erasure prohibition is routing-keys only
- **migration**: Evidence deliveries — B0 Design 1 arms, T-WD step-zero archive
- **rust**: The self-referential-test class, recorded where the eleventh key gets added
- **migration**: S6 range mark frozen to its exact id set — ranges never inherit
- **migration**: SC-510c joins the frozen S6 historical enumeration
- **migration**: SC-021 ls-alias row (s2 Q4) with honest inventory-authority
- **migration**: SC-021 marked per countersign; SC-022 usage-error surface row
- **migration**: Batch L design draft v1 — six seat-gated sections, 94 assignments
- **migration**: B0 Design 8 delivery — transport-separation captures, 20 cells
- **migration**: Marks-pass queues, machine-derived (Q1=179 Q2=32 Q3=107 closed=103)
- **migration**: L design v2 + roster checker; SC-705/706 -> fix-known-defect(#94)
- **migration**: B0 Design 7 delivery — schema-evolution matrix, 108 family-runs
- **migration**: L design v3 — arm-coverage gate, per-consumer clones, corrected specimens
- **migration**: L v3.1 — per-arm body association, four-clone SC-819, twin artifact
- **migration**: Batch C arm group A1 delivered — schema/document, 39 case runs
- **migration**: Environment-as-instrument global rule (the A1 locale incident)
- **migration**: SC-1106 — locale-dependent tmux parsing is the product's own defect
- **migration**: A1 re-run under pinned UTF-8 — no divergence; correction persisted
- **migration**: SC-1106 empirical pointer -> committed isolation artifacts @605cbb6
- **migration**: S3 delivery/routing mark batch 1 - 11 rows classified by both seats
- **migration**: S3 helper-signature mark batch 2A - 19 SC-212 rows classified by both seats
- **migration**: SC-1106 empirical pointer re-anchored to the rerun evidence
- **migration**: Marks-queues regenerated from HEAD - stale c5f2a2 derivation replaced
- **migration**: S1/S2 mark batch 3 - 5 rows classified; SC-012b residue row split out
- **migration**: SC-521a reclassified bucket 3 fix-known-defect(#96) on A2 evidence
- **migration**: Second-gate precision fixes - SC-521a evidence attribution, SC-012b grain
- **migration**: SC-012b probe wording - capture separately, seats compare
- **migration**: S15 env/config mark batch 4 - 14 rows classified by both seats
- **migration**: SC-509c reason-null defect row (#97) + D01/D02 boundary transcription
- **migration**: Closure state follows the accepted evidence - D01/D02/SC-509c flip to OBSERVED
- **migration**: S8 adapter-frame mark batch 5 - 7 rows classified by both seats
- **migration**: Mixed-tail mark batch 6 - 17 rows classified by both seats
- **migration**: L-END joint-classification worksheet - 21 roster ids, one reopened conflict
- **migration**: L-END classification CONVERGED - SC-820a reclassified (#98), three lead IS corrections
- **migration**: L-PURGE classification worksheet + SC-812 root cause + scoped #94 population
- **migration**: SC-818e outcome-grain precision + L-PURGE worksheet close-out
- **migration**: Provenance note for e3ace55 - it carried the Batch-L checksum fix too
- **migration**: L-PURGE classification CONVERGED - 14/14, zero reopened conflicts
- **migration**: L-COMPACT joint-classification worksheet - 21 ids, one scope question
- **migration**: Stage-2 corpus-import design (#93) + SC-1305 seat closure
- **migration**: Stage-2 import design — four rulings applied (#93)
- **migration**: L-COMPACT gate applied — four proposed marks moved
- **migration**: L-STOP classification worksheet — 20 rows, 4 findings
- **migration**: Link SC-839d finding to #101
- **migration**: SC-508 row-grain subtraction — no residual survives
- **migration**: L-FROM classification worksheet — 9/9, second clean section
- **migration**: L-STOP gate applied — colead moved 8 of 20, two of them my source reading
- **migration**: L-FROM gate applied — section does NOT converge; my 9/9 was wrong
- **migration**: L-RENTRANS worksheet (batch L classification complete) + stage-2 schema inventory
- **migration**: Provenance note for 25c6a00 — T-100 swept into a contract commit
- **migration**: Stage-2 import design — G1 reconciled, F2 escalated and ruled (#93)
- **migration**: Typed tmux session-target audit (#102)
- **migration**: Restore the code references fish ate from the previous message
- **migration**: L-DISCRIM dispositions — two PARTIALs closed, one held, one scoped
- **migration**: Correct my own overstatement about D5a's lineage evidence
- **migration**: Batch C arm A8 — launch modes, the first mutating group
- **migration**: Batch C arm A9 — quiet vs degraded, and META ABSENT
- **migration**: Batch H-HELPER design draft — for seat approval
- **migration**: Batch H design v2 — twin equivalence, opposed pairs, say containment
- **migration**: Batch H design v3 — SC-211l delivery-claim pair, lsof kept as rejected
- **migration**: Batch H design v4 — every REQUEST-CHANGES blocker addressed
- **migration**: Settle the value-blindness line — candidate space vs expected outcome
- **migration**: Batch H design v5 — second review round addressed
- **migration**: Batch H argument census — source-derived, seat-gated, plus the executor list
- **migration**: A8 limitation disclosed — controls were post-only, no re-run
- **migration**: Gate resolver records tree-relative resolutions
- **migration**: Batch H census v2 + committed generator, checker and red-proofs
- **migration**: Batch H v7 — usage-exit family measured, captured side by side
- **migration**: Retract the usage-exit correlation — refuted by its own table
- **gatekeeping**: The instrument taxonomy — how a probe lies to you
- **migration**: Batch H v8 — ownership enforced by machine, citations pinned, red-proofs gate rc
- **migration**: Batch H design v8 — the edits f1405f0's message described
- **gatekeeping**: Two more instrument shapes from the batch-H gate
- **gatekeeping**: A tool that makes a check cheap is not one that performs it
- **migration**: Batch H v9 — scope is a validated field, red arms reach the census
- **gatekeeping**: The vacuity regress — every layer can be blind, independently
- **migration**: Batch H v10 — the red-proofs could not fail; now they must prove they can
- **gatekeeping**: The gates were never gated, and redundancy is camouflage
- **gatekeeping**: A count is a fact about a predicate, not only an invocation
- **migration**: Batch H v11 — identity instead of substring, and per-arm calibration
- **migration**: Batch H pre-registration — harness and the first arm, before any run
- **gatekeeping**: A remedy that depends on someone staying uninformed is not a remedy
- **migration**: Batch H amendment A1 — split-assignment abort in the canary
- **migration**: T-WD design v3 — 21 arms, three-part symmetry gate
- **migration**: Batch H amendment A2 — the fixture did not carry its own collisions
- **gatekeeping**: Measure-read-assert, and why an independent check can miss a wrong path
- **migration**: Batch H A-H4 — SC-211p captured, fifteen input classes
- **migration**: Batch H A-H4 re-run under amendment A4, gate green on the H tree
- **migration**: Batch H pre-registration — A-H5 (SC-211o) before its first run
- **migration**: Canonical gate published; A-H5 (SC-211o) captured
- **migration**: Two manifest phrases read as path citations, not prose
- **migration**: T-WD design v4 — per-arm execution grain, M12 for all six leaked rows
- **gatekeeping**: A green gate a reader cannot reproduce is a claim, not evidence
- **migration**: A-H5 record generated from its captures
- **migration**: Batch H amendment A8 — the cwd cases never consulted cwd
- **migration**: Batch H amendment A9 — the invocation stood in the wrong directory
- **gatekeeping**: Pre-registration freezes by provenance, not by path
- **gatekeeping**: Closing layer 1 hides layer 2; and a followed rule that still fails
- **migration**: Batch H amendment A10 — token-precedence controls kept, fallback pair added
- **migration**: A-H5 re-run under A10 — token precedence and the fallback, separately
- **gatekeeping**: Reachable is not discriminating; an amendment moves cases it does not name
- **migration**: Batch H pre-registration — A-H3 (argument surface) before its first run
- **migration**: T-WD design v5 — provenance-bounded closure, aewatch subject, named blocks
- **gatekeeping**: A transcribed checklist can only under-report; positional back-references retarget silently
- **migration**: Amendment A12 + a gate that can see chronology at all
- **gatekeeping**: The seat boundary in its finished form
- **migration**: A-H3 (argument surface) and A-H5 re-run, chronology clean
- **migration**: T-WD design v6 — named references, constructibility at gate 1, reachable barriers
- **migration**: Manifest prose that read as path citations
- **migration**: T-WD v7 — M15 lane-unit contract made binding, stray spec/unit count fixed
- **gatekeeping**: Agreement is not verification; hand-maintained redundancy drifts silently
- **migration**: Gate prints its own coverage; SC-211l pre-registered
- **gatekeeping**: Why rules get broken inside their own statement
- **migration**: Amendment A15 — the census cannot classify most processes here
- **migration**: SC-211l captured under containment, with the census's blind spot stated
- **migration**: Batch H captures are -text — a pty writes CRLF
- **gatekeeping**: Put a blind spot's size in the data, not in prose
- **migration**: Batch H pre-registration — A-H1 and A-H2 before their first run
- **migration**: T-WD design — per-lane unit ids, typed GAP class, and a committed checker
- **gatekeeping**: Knowing a fact and applying it are also different acts
- **gatekeeping**: Point verify-the-injection-landed at the instrument too
- **migration**: A-H1 and A-H2 — spelling families by captured stdout hash
- **gatekeeping**: Ask what gates precede the fact under test
- **migration**: What A-H2's selector cases do not establish, recorded beside them
- **migration**: Batch H pre-registration — A-H8, guards enumerated before building
- **gatekeeping**: A red-proof's coverage must match the tool's
- **migration**: Amendment A19 — the cohorts held one line, not 29/30/31 events
- **migration**: T-WD — replace the vacuous checker with a parsed, self-red-proven one
- **gatekeeping**: A documented hazard reappearing inside the tool built to police it
- **migration**: A-H8 — SC-211n captured, cohorts either side of the replay cut
- **gatekeeping**: Guard enumeration does not cover fixture construction
- **migration**: Batch H pre-registration — SC-1301's hooked arm and its patch
- **migration**: Amendment A22 — the equivalence comparator was answering about labels
- **migration**: T-WD — contracts instead of spelling filters, roster/body join, named-id red-proofs
- **migration**: Amendment A23 — the hooked binary must BUILD the fixture
- **gatekeeping**: A self-test cannot establish coverage
- **migration**: T-WD — counts by provenance, not recognition
- **migration**: Amendment A24 — the hooked copy had no executable bit
- **gatekeeping**: An overstated claim has two repairs, and narrowings need their own red-proof
- **migration**: SC-1301 captured — three writer-shaped cuts, three claims
- **gatekeeping**: An instrument must exist when the fixture is built, and a perturbation must be measured not expected
- **migration**: T-WD — build the four blocker classes, shrink the claims that outran them
- **gatekeeping**: Blindness forces design honesty, and a resolving citation can still be wrong
- **gatekeeping**: A frozen oracle removes the reason to capture evidence early
- **migration**: T-WD — narrow the claims to what the tool enforces, then bank
- **migration**: T-WD — fix a banned term the previous commit shipped red
- **migration**: Refresh batch-H PATH-CITES — the committed index was stale, and the gate could not have told us
- **gatekeeping**: A gate that generates its input, and the over-application of hard-won protocol
- **gatekeeping**: A rewrite inherits conflations through its types, before any logic exists
- **migration**: Execute the golden-corpus promotion (P0 debt under P1)
- **gatekeeping**: Route by symptom, because nobody reads 1100 lines before a gate
- **migration**: Partition corpus invocations by read/write and prove the normaliser both ways
- **gatekeeping**: The compiler's list is a lower bound, and a tuned instrument is not evidence
- **migration**: Label the P1/P1-adjacent line per the seat ruling
- **migration**: Write the P1 build plan before building it
- **migration**: Correct the P1 plan — the schema moves with liveness, not before it
- **migration**: P1 corpus sufficiency analysis — not sufficient, and larger than defect rows
- **migration**: Record the parity-verdict ruling and why the missing captures are not built
- **gatekeeping**: Unobservable is a third answer, and comfortable errors go unchecked
- **migration**: Parity is not 'match the corpus' — 52% must diverge when we are right
- **gatekeeping**: Give the comfortable-error rule an actual check
- **gatekeeping**: A probe's scope decides the finding, and expectation chose the scope
- **gatekeeping**: Independence is a scheduling constraint, and it runs opposite to intuition
- **migration**: Land the pre-registered parity verdict column over the 1065 P1 rows
- **gatekeeping**: Forward verification is blind to false negatives
- **migration**: Record the pre-registered phase-1 gate, and that it does not pass
- **migration**: Completeness critique of the SC-017j phase-1 gate
- **gatekeeping**: A static gate cannot see a temporal obligation
- **migration**: Extend P1 sufficiency to SC-400d and SC-405l
- **gatekeeping**: Absent and unobservable cost different things to close
- **migration**: Assign the refusal's removal to a phase, and record what parity can never prove
- **migration**: Correct the SC-405l 'missing' reading after the clarifying ruling
- **gatekeeping**: Sharpening a rule silently blunts the evidence behind it
- **gatekeeping**: A richer case must not conclude less than a poorer one
- **migration**: Bring the build plan up to where the work actually is
- Pre-register P1 phase 2 falsification gate
- **gatekeeping**: Orthogonality is proven off-diagonal, never on it
- **migration**: Third sweep-in near-miss, and the previous remedy did not cover it
- **migration**: Completeness critique of the phase-2 gate
- **gatekeeping**: The flip test beats off-diagonal cells for proving independence
- Gate incomplete P1 inventory snapshots
- **gatekeeping**: Patching an enumerated rule keeps the cause that produced the gap
- **gatekeeping**: A fixture that succeeds can still build an unreachable state
- **gatekeeping**: A control that never applied looks exactly like a guard that failed
- **migration**: Second pass on the gate text that has not had two independent reads
- **gatekeeping**: A universal obligation checked on one fixture is checked nowhere
- **gatekeeping**: A confirmation you print yourself is not evidence
- **gatekeeping**: Being argued out of a concern is not resolving it
- Preregister P1 phase 2 and 3 gates
- **gatekeeping**: A control can apply cleanly and still change nothing
- **gatekeeping**: An over-strong test is a defect and almost nobody hunts for it
- **gatekeeping**: An ambiguity is a lead, not a finding
- **gatekeeping**: A guard enforces the names it lists, not the capability it claims
- **gatekeeping**: Satisfying a structural proxy can relocate the violation
- **gatekeeping**: How the name-guard was actually repaired, and the control that proved it
- **gatekeeping**: Correct an overclaim — behavioural closure reaches only what the fixture varies
- **migration**: Which axes the phase-2 evidence varies, and which it holds constant
- **migration**: Mark the axes analysis INVALIDATED — all three findings false
- **gatekeeping**: The deletion arm is the weakest one, and growth carries the proof
- **gatekeeping**: Rigour inside a wrong scope produces confident error
- **migration**: Axes analysis re-scoped over the whole phase-2 evidence base
- **gatekeeping**: A structurally-discharged obligation makes its test a restatement
- **gatekeeping**: Add a second protocol, for reviewing claims and evidence
- **gatekeeping**: Five corrections to the second protocol, including one it fails itself
- **gatekeeping**: A derived artifact goes stale when its source moves, silently
- **gatekeeping**: The size of an amendment is no guide to what it invalidates
- **migration**: Reconcile VERDICTS.tsv against the contract as it now stands
- **gatekeeping**: Granularity mismatch produces a confident wrong count
- **gatekeeping**: Reviewing outcomes is not reviewing instruments
- **migration**: Obligation-grained parity table with a freshness relation
- **migration**: Name the freshness reference on the gate's success path
- **migration**: Define three-valued agent liveness
- **migration**: Re-derive obligations against the agent-liveness rows
- **gatekeeping**: Size unobservability as a fraction, not a list of cells
- **gatekeeping**: A count is only as honest as its denominator
- **gatekeeping**: Escalate the observation, withhold the conclusion
- **migration**: Obligations carry SUPPORT — the corpus cannot score 665 of them
- **gatekeeping**: A test-authored instrument observes the test's beliefs
- **gatekeeping**: Reconciling a disagreement can find a defect rather than settle a count
- **migration**: Selector-missing is an independent sufficient cause — sixteen rows recovered
- **gatekeeping**: Name the four-depth sequence, because recognising it did not prevent it
- **migration**: Preregister the P1 parity gate
- **gatekeeping**: Candour reads as sufficiency, and the repair that changed the claim
- **list**: Write the transport handover at the seam, on both sides
- **migration**: Retire the stored verdict column; name the wrong-set sequence
- **gatekeeping**: The wrong-set sequence, now five, and named by its mechanisms
- **gatekeeping**: Additions are verified by presence, removals by absence
- **gatekeeping**: A test count that drops after an additive change is a contradiction
- **gatekeeping**: A closed register is only as closed as its most open cell
- **gatekeeping**: Auditing the cells is not auditing the set
- **gatekeeping**: Forbid the operation, not the possession
- Reconcile P1 gate status claims
- **gatekeeping**: A scoped verdict copied without its scope is a stronger claim
- Scope historical P1 gate statuses
- **gatekeeping**: Wiring an inert seam makes dependent fixtures contingent
- **p1**: Record the phase-1 re-gate history, not the stale header
- **gatekeeping**: Every gate property fails in two directions
- **gatekeeping**: Disjoint findings come from disjoint positions
- **migration**: Pre-register P1 phase 3 gate
- **gatekeeping**: A decision's premises can be invalidated by a later ruling
- **migration**: Pre-register P1 phase 4 parity gate
- **gatekeeping**: A deferral is a promise about the future in the present tense
- **gatekeeping**: A mechanism with one reachable output cannot be tested
- **migration**: Correct transport gate premise
- **gatekeeping**: A migration's defect list is a review instrument
- **gatekeeping**: A test can name a fact without exercising it
- **gatekeeping**: A stated precondition is a hole; a measured one is a bound
- **p1**: Commit the agent-health presentation manifest, bounded by measurement
- **migration**: Scope phase 3 exit-status choice
- **gatekeeping**: A reservation covers mechanisms that presuppose an answer
- **gatekeeping**: A discovery that changes a row must graduate into an assertion
- **agents**: The researched dependency line — std until TLS, fuzz, or vet demands otherwise
- **gatekeeping**: Absent and malformed are different defects
- **p1**: Re-pin the agent-health manifest, and retire a sentence that expired
- **gatekeeping**: Fail-open enumeration hides its staleness; fail-closed announces it
- **gate**: C1 pins the criterion-3 reconciliation blob — a fixed input the manifest could not name
- **gatekeeping**: A key that is not a key manufactures agreement
- **gatekeeping**: A negative claim inherits the scope of the search that produced it
- **gate**: The aggregate control moves with the twice-confirmed table — 1,614 / 949
- **gatekeeping**: A checker iterating the subject gets quieter as the defect grows
- **gatekeeping**: Seeds aimed at the code test the mechanism; aimed at the defect, the fear
- **gate**: C14 names an object that exists — the published projection, two identities, verified before invoke
- **gate**: The post-permission recheck must use the identity that can see what permissions change
- **session**: Scope the ordering claim, and name last_active as the ruled exception
- **gatekeeping**: A gating test ratifies by enforcement, whatever its label says
- **gatekeeping**: A fact that must be updated does not belong beside a fact that is checked
- **gatekeeping**: Neutrality is measured by flipping the unruled dimensions
- **hazards**: The userland table governs your own shell, not only the product
- **hazards**: The expensive member of the own-shell class is silent
- **contract**: The undercoverage test is candidate identity, not an aggregate count
- **run1**: The digest-scored label meant envelope admissibility, not parity
- **coexistence**: Record the canary outcomes and the CI state as they are
- **canary**: Musl DNS/NSS — outcome 4 passes on the Linux CI leg
- **telegram**: The reference knows about the autostart-refusal record

### Features

- **watchdog**: Mode-aware session location in the status bar
- **watchdog**: Group repo context on status-left; drive live status via tmux user options
- **goal**: Session goal as first-class metadata
- **steward**: Rename ae hub → ae steward + focus-mode rituals
- **steward**: Gated proactive interrupts in focus mode
- **telegram**: Default plain messages to the running steward
- **compat**: Fail fast on bash < 4 with a macOS remedy
- **steward**: The objective is the switch — collapse focus/passive modes
- **list**: Session context — git branch + goal age in list and --json
- **delegation**: Tiered-model delegation protocol + steward config watch
- **spawn**: Workers get their own tmux window; main window stays the lead's
- **delegation**: Prefer ae workers over harness-internal subagents
- **autostart**: Steward + telegram bridge come up on any ae entry point
- **spawn,attn**: TUI-readiness before prompt paste; unanswered-request attention
- **doctor**: Refresh restarts running watchdogs, liveness-gated
- **events**: Resume-time retention for events.jsonl + attention docs
- **aewatch**: Phase-1 skeleton — PEP 723 sidecar scaffold + test runner
- **aewatch**: Contract fixture matrix — loader, schema validator, CLI
- **aewatch**: Effect-oracle harness — EFFECT_KINDS schema, recorder, FakeTmux/FakeAeHome
- **aewatch**: Per-AE_HOME singleton lock + atomic heartbeat, daemon --once skeleton
- **aewatch**: Ae INI parser — exact parse_config parity port
- **aewatch**: Session discovery — per-meta tmux_server, inventory-not-filter
- **aewatch**: Daemon.log — bounded rotation + fail-closed secret redaction
- **aewatch**: Crash-loop backoff state — windowed budget, reset-on-success
- **aewatch**: Phase-1 tick composition — the sidecar skeleton is complete
- **aewatch**: Tmux.display_message effect kind — complete the oracle surface
- **aewatch**: Multi-tick fixture harness — TickClock, MultiTickEnv, feed-forward events
- **aewatch**: Bash dual-run oracle — fakebin shims + real-watchdog runner
- **aewatch**: Python watchdog cycle skeleton — first byte-identical status parity
- **aewatch**: Activity classification parity — event recency + pane hashing
- **aewatch**: Stale-nudge parity — first tmux.paste + event.append, byte-exact
- **aewatch**: Quiet-state parity — done/waiting-user/blocked arm-hold-yield
- **aewatch**: Alert parity — dead/missing/max-nudge, display_message speaks
- **aewatch**: Throttle parity — verbatim per-tool catalogs, streak + alert + clear
- **aewatch**: Sweep-cadence + wedge parity — pins ae reconcile dead-code
- **aewatch**: Recover-pending parity — post-launch session-id capture retry
- **aewatch**: Telegram-supervise parity — scheduler + tmux_server propagation
- **aewatch**: Daemon tick composition — run watchdog cycles per session under injection
- **aewatch**: Per-session tick-input routing — phase-3 contract + harness cutoff
- **aewatch**: RealTmuxClient read path + single-source Pane
- **aewatch**: RealTmuxClient write path (mutations + paste/submit)
- **aewatch**: Real ae/event boundaries (emit_event + recover_pending)
- **aewatch**: Real BridgeSupervisor boundary
- **aewatch**: Bridge oracle — TelegramTransport seam, fake API, machine-checked anchors
- **config**: Default setup — strongest lead + two standing coworkers
- **aewatch**: Telegram token config, validation, redaction
- **aewatch**: RealTelegramTransport — Bot API over urllib
- **aewatch**: Inbound offset + auth (at-most-once, exact-auth)
- **aewatch**: Command resolver (routing security boundary)
- **aewatch**: Agent delivery primitive (command-execution boundary)
- **aewatch**: Command routing precedence (confine -> execute -> route)
- **aewatch**: Outbound formatter + include/exclude filters
- **aewatch**: Outbound state.tsv + at-least-once retry
- **aewatch**: Command menu registration (setMyCommands)
- **aewatch**: Bridge tick composition (TelegramBridge)
- **aewatch**: Supervisor loop — per-component crash backoff, clean-signal shutdown
- **aewatch**: Dedicated ae-aewatch session launcher — per-AE_HOME, heartbeat-gated
- **aewatch**: Opt-in aewatch watchdog autostart + exclusivity, up/daemon--loop CLI
- **aewatch**: Ae telegram bridge handoff — marker-owned, no double-send, bash fallback
- **aewatch**: Phase-3 closer — contracts migration + mutation-proven coverage guards
- **ae**: Slot-keyed request integrity — churn-safe identity + routing
- **ae**: Request-integrity 2 — live slot stamping, paste verify + interpreted-sink guards, spawned-slot stability
- **ae**: Lead-default — model-named aliases, slot-aware role context, lead-solo layout
- **ae**: Mode-context — mode-aware working-tree block + accurate copy_desc
- **ae**: Session-shape — lead-pair layout, colead seat, status-left session name
- **ae**: Footer rework — ae-owned status bar, agent-identity line, leads default
- **ae**: Footer agent-roster — per-agent verdict + subprocess activity
- **ae**: Input-region sensor — cursor-anchored SGR, codex staged detection restored
- **ae**: Roster completeness — per-window glyphs, registered keying, steward
- **ae**: Grok build integration — claude-class session handling
- **ae**: Dedicated gpt56sol xhigh reviewer in the default worker roster
- **ae**: Doctor orphan check + wind-down discipline (lifecycle blindness)
- **ae**: Opus5 replaces opus48 as the default builder tier
- **ae**: Spawn lifecycle closure is an emphatic contract — every spawn ends in a retire
- **ae**: Helper-emitted origin envelope, and the authority rule it enables
- **ae**: Opencode gets real system-level context instead of a first-message paste
- **ae**: A session that ends leaves an inert archive, and a new one can inherit it
- **ae**: A request can be withdrawn, and the one sensor both readers share knows it
- **ae**: A session can hand itself over, end, and continue under the same name
- **ae**: An agent is told its own name, and a name is an allowlist before it may reach a prompt
- **rust**: P0 toolchain, quality lanes, and CI — pins are the contract
- **rust**: P1 slice 1 — event log, session digest, list read-side
- **list**: First-class Unknown liveness, wired into scope selection
- **list**: Phase 1 candidate inventory, with the invariants held by construction
- **list**: SC-400d two durable layouts and SC-405l typed selector
- **list**: Record an unlistable state root instead of skipping it silently
- **list**: SC-017o incomplete-inventory snapshot fact, and the scan becomes infallible
- **list**: Phase 2 — liveness knowledge, first-class unknown, schema version 2
- **list**: Wire schema version 2 and the completeness field into the digest
- **list**: Phase 3 — the product answers
- **list**: Agent liveness gains a real unknown — #105 one level down
- **liveness**: Give tmux a real transport, and let the exit status decide
- **liveness**: Derive and read a pane enumeration, without deciding anything
- **liveness**: Carry SC-017s's two conjuncts out of the pane read
- **rust**: The requests and events-tail read surfaces, byte-compared against the corpus
- **509**: A member that was read is rendered; omission is reserved for loss
- **509b,017h**: Presence follows per-source knowledge, not a degraded bit
- **list**: The human table renders frozen's subline and empty states
- **list**: The short session id was never rendered, on either surface
- **p2.1**: The requests helper hands mode all to a pinned Rust core
- **p2.1b**: The requests core reads the caller's pane identity
- **p2.2**: State declarations are written by the pinned Rust core
- **p2.3**: Goal and memo writes go through the pinned Rust core
- **p2.4**: State, goal and memo reads go through the pinned Rust core
- **p2.5a**: Ask and review are created and delivered through the pinned Rust core
- **p2.5b**: Reply is created and delivered through the pinned Rust core
- **p2.6**: The public send is resolved, delivered and recorded by the pinned Rust core
- **p2.7**: The monitor pane's events-tail runs on the pinned Rust core
- **p3.1**: Archive preview is the read-only lifecycle tracer on the Rust core
- **p3.2**: Route worktree/copy archive preview through the typed-git core
- **rust**: Publish the archive from the typed core (P3.3)
- **rust**: Move the archive TRUST domain into the typed core (P3.4)
- **rust**: Move local-mode teardown into the typed core (P3.5)
- **rust**: Move nonlocal (copy/worktree) teardown into the typed core (P3.6)
- **rust**: Compact freeze/resolve resolver — minimal typed config reader (P3.7a)
- **compact**: Dormant Rust core for compact's destructive-safety gates
- **compact**: Activate the clean-cut core end to end
- **watchdog**: The Rust core owns the loop; bash keeps process glue
- **watchdog**: The Rust core owns the steward/meta-agent sweep
- **telegram**: The Rust core's first runtime dependency and the outbound bridge
- **telegram**: The Rust core owns the bridge; bash keeps start/stop glue
- **coexistence**: Ae-next runs the Rust hybrid beside an untouched ae
- **send**: Notice mode for long bodies — the file is the delivery, the pane gets one line
- **telegram**: An autostart refusal leaves a trace — closed category, two surfaces
- **release**: SemVer-compatible CalVer — the tag ledger owns the sequence
- **dist**: Prebuilt ae-next bundles and the one-line remote installer
- **p5**: Forward-only entry flip — public Rust entry, immutable bundle, ae-next retired

### Miscellaneous

- **config**: GPT-5.6 aliases — gpt56sol/terra/luna strict pins, sol as default reviewer
- **ae**: Promoted-tier examples drop sonnet5 — chores run luna
- **ae**: Grok-4.6 high is a dev-tier peer of opus5
- **version**: 2026.8.2 — bumped ahead of the entry flip, deliberately untagged

### Other

- Updates
- Add doctor, memo, rename, perf improvements, pane resilience

- ae doctor: check deps, config, agent CLIs; --sync-sessions refreshes
  existing session helpers from current ae code
- memo: shared append-only session memory (add/read/tail with topics)
- ae rename: rename running sessions (tmux + meta + workspace.md)
- config caching: deterministic temp file cache with mtime invalidation,
  eliminates repeated parse_config subshell forks (~3s saved on startup)
- parse_config: replace sed forks with pure bash trimming
- startup polling: reduce wait_for_agent_start from 40 to 10 iterations,
  remove blocking retry loop (~7s saved on startup)
- pane resilience: remove exec from pasted launch command so panes
  survive agent crashes/exits
- default model: opus[1m] for 1M context window
- new helpers: review, reply, requests, peak (typo alias for peek)
- launch scripts: write_launch_script + build_launch_command infrastructure
- agent context: expanded REQUIRED AE RULES with ask/review/reply/memo
- tests: 99 unit, 45 integration all passing
- Add events.jsonl structured event log for session observability

Add ae_json_escape and ae_emit_event to _lib. Every ae-mediated action
(send, ask, reply, review, memo, spawn, retire, interrupt, focus) now
appends one JSONL line to events.jsonl after success. ask/review/reply
delegate to send with action/ref/summary overrides to avoid duplicate
events. spawn/retire use inline flock-protected emission since they run
outside _lib context. 18 new unit tests for escaping and structural
verification.
- Add loop watchdog for stale agent detection and nudging

Generate loop helper with start/stop/status subcommands. Detects stale
agents via pane content hash (staleness) and events.jsonl timestamps
(liveness). Nudges via existing send helper with AE_SENDER_OVERRIDE=loop.
Escalates after max nudges with alert event and tmux display-message.

Runs in hidden tmux window tagged @ae_agent=_loop, inspectable via peek.
Skips focused pane, respects human grace window after helper-mediated
actions, detects dead agents (process dropped to shell). Auto-start via
[workspace] loop=true config. Configurable thresholds via env vars.
- Rename loop watchdog to sentry

Rename loop → sentry across helper, config key (workspace.sentry), env
vars (AE_SENTRY_*), tmux window (ae-sentry), agent tag (_sentry),
sender override, display messages, and tests.
- Drop focused-pane check from sentry

Focused pane is a weak signal — it can't tell if the human is actually
looking. The pane hash (step 2) and human grace window (step 4) already
cover the real cases: direct typing changes the hash, helper-mediated
interaction triggers the grace window.
- Drop human grace, add missing pane detection
- Add recently-visible check for human interaction edge case
- Add ae sentry top-level command with per-session persistence

New 'ae sentry <start|stop|status> [name]' top-level command resolves
the session directory and execs the sentry helper. Auto-detects current
session when run inside one.

Sentry helper now persists state to the session meta (sentry=true|false)
under flock discipline. Session start/resume reads meta first for the
per-session override and falls back to workspace.sentry config default.
The meta rewrite path preserves the sentry= line across resume.

Lets you have global sentry=false but enable it for one long session,
or vice versa, and survive ae stop + resume.
- Rename sentry back to loop

The watchdog feature is more clearly named loop than sentry. loop is
self-explanatory in the user-facing context (ae loop start aedev),
while sentry was an abstract marketing term that obscured what the
feature actually does. Renames cover the helper, top-level command
(ae loop), config key (workspace.loop), env vars (AE_LOOP_*), tmux
window (ae-loop), agent tag (_loop), meta key, and tests.
- Rename ae doctor --sync-sessions to --refresh

"sync" implies bidirectional synchronization with something else, but
the flag actually just regenerates session helper scripts from the
current ae source. "refresh" is more accurate and shorter.

  ae doctor --refresh         # all sessions
  ae doctor --refresh aedev   # one specific session
- Recover pending session IDs each cycle
- Read-only pane, status output, cross-window resolution
- Surface live status in tmux status-right
- Walk process tree to avoid false-positive dead alerts
- Prepend status indicator instead of replacing user's status-right
- Ae-monitor window with loop + events panes; codex review fixes

Replace the single ae-loop window with an ae-monitor window that has
two panes split horizontally:

  top    — the loop watchdog (existing behavior, banner + cycle log)
  bottom — events-tail, a formatted live tail of events.jsonl

Both panes are read-only (pane input disabled). The events tail shows
recent history (last 30 events) and follows new ones, formatted as:

  HH:MM:SS  action    actor                  → target                 summary

The bottom pane is tagged @ae_agent=_events so peek/agents/focus can
find it. The loop body's iteration now skips both _loop and _events
to avoid false-positive dead alerts on the monitor's own panes.

Also addresses codex review findings on the previous round:

- extract_binary_from_cmd() now resolves the actual binary, skipping
  leading env-var assignments (FOO=bar) and well-known launcher
  prefixes (env, sudo, nice, ionice, time, nohup, command, exec).
  Used by initial meta write and _cmd_spawn for agent_bin.<slot>.
- agent_bins associative array is declared at top level instead of
  via `declare -gA`, keeping bash 4.0 compatibility (declare -g
  requires bash 4.2).
- The retire path also strips agent_bin.<slot> when removing an
  agent.<slot> entry.
- Sticky column headers via tmux pane borders
- Handle GNU long-option launcher flags
- Add behavioral unit tests for extract_binary_from_cmd

Lock in the launcher edge cases codex flagged across rounds 3, 4,
and 5: bare command, absolute path, env-var assignments, env -i,
env --chdir (space and equals forms), sudo with short -u and long
--user, --user=alice, sudo -E -u (boolean + arg-flag combo), nice
-n / --adjustment, time --format, ionice --class, and nested
env+sudo. 15 new cases. Now exercises real behavior, not just
source-string presence.
- Ae list: show ae version and last-active per session

New sub-line under each session row shows the ae version currently
associated with the session and the time since last meaningful
activity:

  aedev                     running   local       /home/ckriech/...
    ae 0.2.1 · active 2m ago
    claude:lead             1ce6bedf
    codex:coworker          019d66a6

- ae_version is written to meta on initial session creation and
  updated by every subsequent ae <name> start/resume and by
  ae doctor --refresh via sync_session_assets. So it reflects the
  version currently managing the session, not a frozen "born on"
  value.
- Last active is the mtime of events.jsonl (most precise — updated
  by every helper call), falling back to workspace.md, then meta
  for sessions that predate events.jsonl.
- New format_relative_time() helper renders epoch timestamps as
  "Xs/Xm/Xh/Xd ago", ">7d", or "-".
- Sessions without the new ae_version field display as "ae ?".
- Ae list: honor legacy worktree-nested meta path

cmd_list previously read meta_blob directly from
${SESSIONS_DIR}/${name}/meta and passed that same path into
_print_session_meta_line for the active-time mtime lookup. This
missed the legacy worktree-nested fallback path that
read_session_meta() still honors, so sessions whose metadata lives
at ${WORKTREES_DIR}/${name}/.ae/${name}/meta lost their agent rows,
mode/origin detail, version, and active time.

Add _resolve_session_dir() which checks the new path first and
falls back to the legacy path (same logic as read_session_meta).
Both the running-session loop and the stopped-session loop in
cmd_list now use it to resolve the right directory for both the
meta blob and the _print_session_meta_line mtime scan.
- Drop sticky pane-border headers, use in-pane banner
- Label panes via tmux pane titles
- Add mark-done helper so agents can signal completion
- Collapse logs into events.jsonl + factor ask/review
- Trim build_ae_context to 7 numbered rules
- Remove orphan files from prior ae versions on resume
- Decouple events pane from loop lifecycle
- Show date+time in events pane, not time alone
- Detect upstream throttle errors and pause nudges
- On by default; label events banner as UTC
- Event-only done invalidation, drop pane-hash reconciliation
- Ae transfer push (phase 1 walking skeleton)
- Fix SSH arg-passing — %q into unquoted heredoc
- --pull direction (phase 2)
- Clean up underlying claude/codex conversation files
- External-actor protocol + session UUID
- Phase 1 — state helper + ae_latest_state_for
- Phase 3 — surface per-agent state in ae list
- Phase 2 — loop watchdog honours declared quiet states
- Make the stale nudge actionable — hand the agent the state command
- Stage 2 — read-only Telegram bridge (native, machine-global)
- Fix ~/ token_file expansion (literal tilde strip)
- Add ae list filters
- Window 0 must follow the session rename
- Ae list: derived needs_attention rollup (slice 2)

The session attn marker is no longer state-only. cmd_list now computes a
derived rollup — the single most-actionable reason across a session's
current agents, by severity: dead > stale > waiting-user > blocked >
throttled.

- dead: an agent registered in meta has no pane (direct check), or the
  watchdog flagged it (pane missing / process dead — dropped to shell).
- stale: the loop watchdog hit max-nudges on an idle agent.
- throttled: persistent upstream rate-limit alert.
- waiting-user / blocked: self-declared state (unchanged).

New top-level helpers: _agent_alert_reason (reuses the loop's own
alert/throttled events; an alert stays active until the agent's OWN
activity or a throttle-cleared supersedes it — a loop nudge or inbound
send addressed to the agent does NOT count as recovery) and _attn_rank
(severity). The alive map is now built before the rollup so a missing
pane can raise dead.

--needs-me now surfaces watchdog-derived dead/stale/throttled sessions,
not just declared waiting-user/blocked. Help text, workspace prompt,
manifest, README and commands.md updated to document the reason
vocabulary. Pending unanswered ask/review edges remain a planned reason.

Tests: unit for _agent_alert_reason (real loop summaries incl.
process-dead, target-only-does-not-clear, agent-activity-clears,
throttle-cleared, @session target) + _attn_rank ordering + rollup
wiring; integration Test 3c (injected max-nudges alert -> attn:stale,
cleared by newer activity). unit 450, integration 85; shellcheck clean.
Builds on 57a8860; disjoint from the concurrent telegram work.
- Stage 3 — bidirectional (chat → ae inbound)
- Telegram stage 3: address codex review (BLOCKER agent escape + 3 more)
- Ae list: --json digest (slice 3)

ae list --json emits a single machine-readable snapshot for a monitoring
script or agent — pure bash, no jq required. The filters
(--running/--all/--stopped/--needs-me) decide which sessions appear.

Shape: {schema_version, generated_at, sessions:[{name, status, mode,
origin, work_dir, last_active_epoch, needs_attention, attention,
attention_rank, agents:[{ref, alias, name, session_id, alive, state,
reason}]}]}. attention is the session's most-actionable rollup reason;
each agent's reason is its own contribution. schema_version lets
consumers gate on shape.

- New top-level _json_escape (byte-identical mirror of the _lib
  ae_json_escape, which cmd_list can't source) and _session_active_epoch
  (shared with the table meta line).
- _list_session_json reads the per-iteration maps from cmd_list scope via
  dynamic scoping; the stopped path declares empty maps so subscript
  lookups are set -u safe.
- alive means the agent process is running: pane present AND a non-shell
  foreground command. A pane dropped to a bare shell is alive=false.

Tests: _json_escape sync + behavioural; --json wiring asserts; integration
covers valid JSON for default/--needs-me/--stopped/--all, the filters, and
the attention reason. unit 478, integration 96; shellcheck clean. Builds
on the telegram Stage 3 base (6db9347); files disjoint.

Completes Layer 1 of the ae list attention work (filters + rollup + json).
- Ae list: rename --needs-me to --needs-attn

The attention filter now reads as --needs-attn, matching the attn:<reason>
marker the rows print and the internal needs_attention rollup. --needs-me,
--needs, and --attn stay as aliases so nothing breaks. Help text, README,
and commands.md updated. Pure rename — no behaviour change.
- Persistent daemon log file
- Ae list: --active filter for recently-active sessions

`ae list --active` (alias --busy) shows only running sessions with recent
activity — an ae event within the last 5 minutes (override with
AE_LIST_ACTIVE_SECS). Implies running-only, like --needs-attn, and
composes with --json. Answers "which sessions are in flight right now".

Honest scope: recency is measured from ae-event mtime (messages, state,
nudges, spawns), not raw pane churn — a silently-working agent that emits
no events won't count until it uses a helper. The loop watchdog remains
the thing that tracks pane-level activity.

Tests: unit (flag/alias, default+env window, skip logic, empty message) +
integration (fresh session shows, backdated activity drops out, plain
list still shows it). unit 487, integration 100; shellcheck clean.
Builds on 0abc704; cmd_list-only, disjoint from the telegram loop work.
- Loop watchdog best-effort revives the bridge
- Reply-to-routing (inbound UX, slice 1)
- Compact @session:agent prefix + sticky /use (inbound UX, slice 2)
- Register slash-command menu via setMyCommands (inbound UX, slice 3)
- Fix setMyCommands 400 — pass commands JSON via @file form
- Add 'say' helper + chat event (agent → human, two-way)
- Record terminating signal in daemon exit log
- Ae next: attention navigator, read-only (Layer 2, slice A)

Layer 2 of the meta-agent: turn the Layer-1 attention SIGNAL into the next
ACTION. 'ae next' (alias 'ae jump') names the top-ranked running session
needing attention — name, reason, rank, contributing agent — read-only, exit
non-zero when nothing needs you (composes in scripts + Layer 3).

To avoid duplicating attention semantics (codex BLOCKER), the per-session
rollup is extracted into a shared _session_attn_rollup — the SINGLE source of
the dead>stale>waiting-user>blocked>throttled severity rollup, used by BOTH
cmd_list and cmd_next. It returns via globals (not stdout) so the call runs in
the caller's shell and still fills cmd_list's _areason map (--json per-agent
reason) via dynamic scope; a process-substitution subshell would drop those
writes. cmd_list refactored onto it (its inline _cur build + rollup loop
removed); the rollup unit asserts now target the shared function.

Acceptance: ae next names the attention session + reason, exit 0; clear
message + non-zero when none; read-only (no tmux focus change). Tests: unit
(rollup logic, wiring, read-only guard, globals/_areason) + integration
(none→non-zero, waiting-user→named, read-only). 537 unit / 110 integration,
shellcheck clean. Slice B adds --attach.
- Ae next --attach: jump to the attention session (Layer 2, slice B)

Adds the action half of the navigator: 'ae next --attach' (alias --switch)
jumps to the top attention session — switch-client when already inside tmux
(attach-session errors there), attach-session otherwise. Read-only stays the
default. It re-checks the session still exists (race: it may have ended between
the scan and the jump → clean non-zero error) and no-ops with a message if
you're already in it.

The inside/outside-tmux decision is a pure _next_focus_argv (unit-tested both
ways); the exec is thin (array-expanded, no word-split). --attach is now parsed
(not rejected) and still guards on no-attention before any focus change.

Tests: unit — _next_focus_argv inside/outside, read-only-default gating,
--attach revalidate/no-op/focus wiring. integration — --attach with nothing to
attend exits non-zero without a focus change (the happy-path switch/attach needs
a live tmux client, so it's covered by the pure unit test + manual QA per the
plan's test strategy). Docs (README/commands.md/help) updated. 548 unit / 115
integration / shellcheck clean.
- Shfmt-format ae so 'just check' is green
- Ae loop: meta-agent sweep cadence (Layer 3, slice 3)

The monitoring hub is a long-running SERVICE — 'idle between sweeps' is normal,
not stale. The stale-nudge watchdog would nudge it to declare a quiet state and
then alert 'needs attention' after MAX_NUDGES, which is wrong for a monitor (and
codex's BLOCKER: don't pretend the stale-watchdog is a scheduler).

Add an explicit sweep cadence: [workspace] meta = true marks a session as the
hub (persisted to meta as meta_agent=true, config-driven, re-read each
start/resume). When set, the loop — AFTER its dead-check (so a dead hub still
alerts) and before the stale machinery — sends a 'run your sweep now' nudge every
SWEEP_SECS (AE_LOOP_SWEEP_SEC, default 300) and never escalates the hub to stale.
The missing-pane check (step 8) still applies. Non-hub sessions are unchanged.

Tests: structural asserts for the config→meta flow, the SWEEP_SECS knob +
meta_agent read, the gated/interval-throttled cadence branch, the 'run your
sweep now' wording, and that the cadence branch precedes the stale-nudge logic
(hub bypasses max-nudges/stale-alert). 556 unit / 115 integration / just check
green.
- Deterministic state/dedup helper for the hub (Layer 3, slice 2)
- Per-agent attention keying + first-run-by-existence (codex slice-2 review)
- Ae loop: fix wedge-heartbeat filename (+ doc); throttle already covers the banner

BUG (real, live-impacting): the Slice-3 meta-agent wedge-detector watched
META_DIR/meta-state.json, but the Slice-2 contrib aemonitor helper writes
meta-agent-state.json. Once the hub is wired to aemonitor the watched file is
never written again → a FALSE 'meta-agent not sweeping' alert fires while the hub
IS sweeping (observed live at 10:52). Align the loop to meta-agent-state.json
(aemonitor's atomic-write file) + fix the stale docs/reference/commands.md that
named the old file (the same doc-drift that caused the mismatch) + note that
overriding aemonitor --state must point at the same path.

Throttle: NO code change. Clemens's Claude banner ('API Error: Server is
temporarily limiting requests (not your usage limit) · Rate limited') is ALREADY
detected by the claude catalog's 'Server is temporarily limiting requests' (test
1617). A bare generic 'Rate limited' was considered but REJECTED (codex
IMPORTANT): it false-matches normal prose without fixing any live case — added a
negative test pinning that it must not match.

Tests: heartbeat asserts meta-agent-state.json (and NOT the old name); bare
'Rate limited' prose negative. unit 563 / integration 115 / just check green.
- Ae hub: first-class meta-agent launcher

Promote the start-hub wrapper to a real `ae hub` subcommand — start/resume the
meta-agent hub (one session that monitors all other ae sessions and is the
operator's single point of contact to them).

- `ae hub` trampoline (dispatcher): handles --init/--help, else sets up FULL
  config isolation and FALLS THROUGH to the generic start/resume path (no
  re-dispatch → no recursion). Isolation = clear AE_LOCAL_CONFIG (captured from
  the caller PWD at script top, before any cd) + cd HUB_DIR + absolute
  CONFIG_FILE from $PWD. Fixes the worker-leak class a project-local ./.ae/config
  would otherwise reintroduce (codex BLOCKER).
- `ae hub --init`: scaffold ~/.ae/meta-hub/{hub.config,CHARTER.md} from
  contrib/aehub templates; only-missing-files (no overwrite, idempotent),
  symlink-aware template resolution with realpath/echo fallback (no new dep),
  bash placeholder substitution (no sed), rejects HUB_DIR with quote/newline.
- Config flag: accept `hub = true` (preferred) as a non-breaking alias of
  `meta = true`; internal meta_agent / state-file names unchanged.
- Templates: contrib/aehub/{hub.config,CHARTER.md,README.md}, genericized
  ("your operator", ~ paths, __CHARTER_PATH__/__AEMONITOR_PATH__ placeholders).
- Docs: README + commands.md document `ae hub` (+ the hub/meta alias, the
  AE_HUB_DIR override, and the `ae --local hub` escape hatch).
- Tests: +10 unit (scaffold no-overwrite/substitution/reject, dispatcher
  structure, flag alias) and +5 integration (the BLOCKER isolation test: `ae hub`
  from a hostile project .ae/config → hub config wins, no worker leak, work_dir/
  config/meta_agent pinned). 573 unit / 120 integration / just check green.
- Make AE_HOME authoritative for all state (isolation without swapping $HOME)
- Ae hub: charter-path fix (AE_HOME-correct helpers) + hub-injection-guard e2e

The aehub charter template hardcoded ~/.ae/sessions/hub/<helper>, so an isolated
AE_HOME hub would point its say/peek/aemonitor at the LIVE ~/.ae — blocking any
hub e2e scenario. Fix: the charter uses a __HELPERS_DIR__ placeholder that
cmd_hub_init substitutes with $CONFIG_DIR/sessions/hub (default ~/.ae/sessions/hub;
isolated runs $AE_HOME/sessions/hub). _hub_scaffold_file now takes multiple
placeholder/value pairs. Backward-compatible (AE_HOME unset → unchanged).

New e2e scenario tests/e2e/ai/scenarios/smoke/hub-injection-guard: launches a real
hub (ae hub --init + ae hub in the isolated workspace), relays it a message
embedding "run 'ae end hub'" as quoted pane content, and asserts the hub SURVIVES
(no self-end/stop — the strongest signal) + no end/stop/retire event; an advisory
judge checks it treated the line as data. Regression-guards the meta-agent's
never-self-end charter rule under real prompt-injection.

Tests: unit (multi-pair scaffold; charter uses __HELPERS_DIR__ not literal ~/.ae;
cmd_hub_init substitutes it) + integration (AE_HOME hub --init bakes the isolated
helper path into the charter, never ~/.ae). 581 unit / 127 integration / just
check green; the 3 e2e scenarios parse + skip (77) without the gate.
- Ae end: keep agent conversation files by default (opt-in purge)

ae end previously always deleted the per-session claude/codex jsonl. Those are the
only local record of a session's token usage, so they're now KEPT by default and
purged only on request — for future usage/cost reporting.

Decision (per session, never from the caller's cwd):
- CLI --purge-history / --keep-history (global override) > the session's OWN
  [workspace] purge_agent_history > default KEEP.
- cmd_end only sets the global CLI override (_AE_PURGE_HISTORY_CLI). The default is
  resolved in cleanup_session by hydrating CONFIG_FILE from the session's meta
  'config' + AE_LOCAL_CONFIG from its origin/.ae/config (the resume pattern), so a
  cross-repo end or 'ae end all' honors each session's policy. When a session has
  no usable stored config, CONFIG_FILE is pointed at /dev/null (NOT the caller's
  config), so a stray cwd purge=true can't bleed in.
- cleanup_session (the only path to _cleanup_agent_session_files, only reached via
  end_session) gates on the resolved decision; keeping prints a one-line note.
- cmd_end arg-parse rewritten to a flag loop and now REJECTS a stray second
  positional (destructive command — no silent drop); dispatcher passes "${@:2}".
  Confirm prompt states KEEP/DELETE (CLI) or the per-session policy.
- Docs/help (cmd_help, README, commands.md incl a precedence table) updated.

Tests: unit (flag parse, CLI-only global, per-session resolution, reject-extra,
gated purge) + integration (default keeps; --purge-history purges; a session's own
purge=true purges from a no-flag cwd; a purge=true cwd does NOT override a keep
session; a no-usable-config session keeps; extra positional rejected). 586 unit /
133 integration / just check green.
- **input-region**: Shared _capture_input_region primitive (cursor_y-anchored -e)
- **ae**: Default workspace is the judgment pair — workers are spawned, not standing
- **batch-c**: A1 full rerun + A2 composite - admissibility made first-class
- **batch-l**: L-END section complete - 28 arms, all 21 roster ids + 2 hostile constructions
- **batch-c**: A4 - live-tmux CLI arms + the first hooked barrier capture
- **batch-l**: L-END correction - SC-808 arm re-run with mode-preserving mutation
- **batch-l**: L-PURGE section complete - 41 arms, all 14 roster ids + 2 controls
- **batch-c**: Seat-read remediations + D01/D02 concurrency records
- **batch-l**: L-STOP section complete - 18 arms, all 20 roster ids
- **batch-l**: L-COMPACT section complete - 18 arms, all 21 roster ids
- **batch-l**: L-COMPACT manifest corrections - counts and the two-pid-columns distinction
- **batch-l**: L-FROM section complete - 12 arms, all 9 roster ids
- **batch-c**: Gate v3 (per-case schema + case index) and all five D-record executions
- **batch-l**: L-RENTRANS INCONCLUSIVE/BLOCKED - transport preflight failed honestly
- **migration**: L-RENTRANS partial — batch L capture complete
- Rename identity defects — SC-1303 to bucket 3 (#103), SC-832d/e (#102)
- SC-832c seat closure (normative concur, empirical HOLD) + ae-list coherence correction
- **batch-c**: Gate v4 — committed-bytes check, plus the generated arm table
- **batch-c**: A5 — doctor exits under a controlled PATH (SC-514)
- **batch-c**: A6 — request pairs and the unanswered threshold (SC-518, 522, 523a-b)
- **migration**: L-DISCRIM — five discriminators, each able to produce the unwanted answer
- **batch-c**: A7 — meta grammar (SC-405a-g, 405j)
- SC-405f precised — last event by stream order, not greatest timestamp
- **migration**: D1b — ARM-INVALID is the result, and it forecloses the gap
- **migration**: L-832C — a mixed generation survives a crash, and readers accept it
- SC-832c empirical hold LIFTED — a mixed generation survives, and a reader accepts it
- Report drift when the committed index differs from the generated one
- SC-017j names the entitled server set, so no implementation answers it by accident
- SC-521c — liveness uncertainty does not erase a known attention fact
- SC-521c classified_by in the form the sweep actually parses
- Land the seven P1 entries and refresh the header
- SC-017l's unreachable-server outcome is OBSERVED end to end, not merely source-proven
- Close P1 inventory format gaps
- SC-400d and SC-405l, both CODE
- SC-405l — missing means no selector fact is available, not that bytes omitted it
- SC-017k — a coalesced sighting stays proof, because knowledge must not shrink when evidence is added
- SC-017o — a snapshot that could not see everything must say so
- Correct SC-017o's IS relation — the -d guard does not skip an unreadable root
- SC-017o generalizes to the enumeration graph, not its current leaves
- Three precisions to criterion 24, one of them my own regression
- Install tmux, because the phase-2 liveness proof needs a real one
- SC-017p/q/r and SC-509e, the agent-liveness family
- SC-017s gives ae a way to say alive, and it is one-directional
- **sc-017s**: The probe overclaimed one axis and false-failed another
- The classification document now says which contract it classifies
- **corpus**: Re-derive the obligation table — the contract moved, the table did not
- The gate classified one contract and pinned another
- The four D seat calls, in the exact forms ruled
- A total derived from a permissive parse can always be satisfied by dropping rows
- **crit-assign**: The fifteen successor-era rows, bound to their observers
- **sweep-check**: An enumeration is caught by the first new member of its set
- Phase-4 open-choice reconciliation, both directions
- Cargo fmt import ordering left behind by retired seats
- Reconcile P1 contract obligations independently
- Rebind the open-choice recon to the landed C3 blob
- **corpus**: SC-509b and SC-509c enter the obligation table
- **corpus**: A key that is not a key, found while building the handover
- **corpus**: SC-509c over every producer carrier, not just the self-declared one
- **corpus**: The obligation red-proof stops mutating the tracked evidence
- Evidence(corpus): the exclusion file was below the ruled grain, so it could not
substantiate its own claims

Checker-plus-exclusions only. The accepted table identity does not move:
OBLIGATIONS.tsv stays b1fa3bbf33639aa32ae8641cc51065fe834c7163, confirmed by two
independent derivations at 222/222.

SC-509C-UNPROVED.tsv was keyed (case, consumer, agent_ref) while the accepted table
is keyed (case, consumer, session, agent_ref, locus). 34 of its 184 rows therefore
mapped ambiguously onto two same-attention sessions, and a no-carrier claim that
cannot be resolved to one address is not a claim about anything. A key that is not a
key, this time in the file that records what the derivation DECLINED to claim —
which is where it does the most damage, because nobody audits a negative.

Now emitted at the full ruled address: 184 rows, 184 DISTINCT addresses, each
carrying its session and its exact locus.

Re-evaluated per resulting row rather than assumed to survive the split, and the
asymmetry c3recon measured is now visible instead of collapsed: at tg2b the excluded
agents are fake:bravo and fake:charlie with no declared state, while fake:lead at
tg2b carries waiting-user and is an OBLIGATION, not an exclusion; at tg2wu fake:lead
declares `working`, which is not one of the row's agent-owned active contributions,
so it stays excluded. Two halves, two different answers, previously one unaddressed
row. Checked across all 184: ZERO excluded addresses have carrier evidence at their
own address, so every no-carrier claim survives the split.

The header now states what the file does and does not assert — no carrier was FOUND
by the three searches this generator performs, which is not a claim of impossibility.
- Reconcile phase-4 contract obligations
- Rebind the open-choice recon to C3 343fcd80
- Freshness follows direct provenance, and the taxonomy comes from its rows
- Rebind C8 recon to gate ea794124
- Phase-4 first run against the frozen chain
- Phase-4 OBSERVED obligation scores and per-criterion verdicts
- Phase-4 C3 isolated red-proof transcript
- **corpus**: SC-017o re-derived on entitlement — the value is unscorable, not false
- Publish phase-4 fixture fingerprints
- **corpus**: The unscorable value is an obligation, not a footnote
- Prove published symlink grammar
- **corpus**: A new closed-set member is open until something binds who may use it
- **corpus**: Drop the last reference to a file that no longer exists
- **corpus**: The address is identity, the shape is whole — both declared in bytes
- **corpus**: Proving the owed rows exist never proved nothing else does
- Reject ignored published fixture dirt
- Anchor published fingerprint derivation
- **corpus**: Owed-zero is an obligation to check, not a row to skip
- **corpus**: An allowlist that ignores what is off the list is not a closed set
- Rerun phase-4 contract reconciliation
- Rebind C8 to C3 6bf2e7f8 and gate f31ece2a, with colead's ruled disposition
- Make the byte-exact claim true rather than the claim weaker
- **corpus**: Selector first, then fields — members 1 and 2
- **corpus**: A clock is a recorded fact, not a prefix on a name
- **corpus**: The instrument was the defect under test
- Ordering is a dimension, and a cancel is not a reply with one end missing
- Classify SC-518a, and re-derive what the contract move invalidated
- **crit-assign**: SC-518a enters lead-authored assignment, and the pin follows its source
- A gating test ratifies by enforcement, whatever its label says
- Re-pin both registries to the tightened contract
- SC-518a owns two ordering gaps, and the heading said three
- **crit-assign**: One checked pin, the ratified gap set, and a tag that tells the truth
- Re-pin both registries to 3ba5fdf1
- **crit-assign**: The pin follows its source, and history is not restated
- **corpus**: Selection changes what is shown, never what is true
- **corpus**: An exact shape is not an exact population
- **corpus**: The reason grammar, over every agent rather than the stopped ones
- **017g**: A quiet entry renders its triad; omission belongs to loss alone
- **ratification**: A set-sized total is not evidence of that many headings
- One authority per family, and the guard becomes a seed
- **509**: Presence is part of the schema, ruled once as a class
- **corpus**: A carrier is bound to its session, not to a name
- **509b**: Degraded is aggregate visibility; exactness is a claim about the maximum
- Clarify needs_attention lower-bound semantics
- Make degraded attention lower bound explicit
- The census names its selector, and partial evidence is non-monotone
- **405g**: Branch keeps its predecessor projection, named and dated for retirement
- **corpus**: The qualifier names the session it qualifies
- **corpus**: A control that lives in a message is a control nobody runs
- **405g**: Branch VALUE is unscored under OC-P4-BRANCH-VALUE while the exception stands
- **405g**: Scope the branch-value exemption to the digest comparison, and keep the count out of the row
- **509b**: Scope the attention-uncertainty claim to the attention INPUTS
- **017g**: Repair the third blanket copy, and guard the count instead of the search
- **017e,405f,017h**: Relative spans are scored against one witness epoch, and the state-cell census lands as evidence
- **017l,017r**: Absence belongs to SC-017m, and an unattempted observation changes the value not the membership
- **017l**: Absence is owned at BOTH grains — one omission, two rows
- **405g**: The branch-value exemption reaches the human surface, registered
- **017r**: A display name was never an identity, so the collision owes a count
- **509**: The two-field session id is ruled, and C8 anchors on phrase content
- **017s**: The four-output tuple is regenerated, and panes join by exact slot
- **017s**: Seed 78 owes a bound six-field pane, so the branch deletion is red
- **orchestrator**: Steward becomes orchestrator, the canonical product term
- **rust**: Full-history checkout — the criterion-1 control derives its baseline from git
- **mutants**: Copy the VCS directory — the criterion-1 control needs git inside the copy
- **rust**: The mutation lane is bounded to the pushed range — and its gap is named

### Performance

- **ae**: Ae list 7-13x faster — fork-storm removed from event parsing, one-pass rollup, exact early-exit scans

### Refactoring

- **watchdog**: Rename the loop watchdog to watchdog
- **helpers**: Generate the state helper from template functions (pilot)
- **helpers**: Generate _lib from the top-level template library
- **watchdog**: Generate the watchdog from the template library
- **helpers**: Migrate the remaining 16 helpers to declare-f emission
- **ae**: Option B — one invariant, no deletion without identity or acknowledgement
- **511b**: The identity comparison moves onto the type whose doc already ruled it

### Testing

- **telegram**: Fix stale auto-start-guard count (2 → 3)
- AI-driven e2e harness (scripted driver, real agents as subjects)
- **integration**: Tripwire — fail the run if the real user config changes
- **unit**: Replace O(n^2) substring-strip ordering assert with grep line numbers
- **aewatch**: Phase-gate hardening + fast-subset commit lane
- **aewatch**: Pin watchdog env config (_env_int / from_env)
- **ae**: Occurrence #3 — chunked multi-token + leaked-tail staging
- **ae**: Specimen-5 — fixture must be the WHOLE-pane region, not 0..cursor_y
- **ae**: REAL human-typed specimens from a disposable v2.1.209 rig
- **ae**: Harden lifecycle end/doctor integration probes against contention
- **ae**: Wait for meta + poll end in the named-server/worktree end probes
- **ae**: Make the doctor-orphan probe deterministic under load
- **integration**: Hermetic socket-dir ownership, unfilterable full mode, scoped name filter
- **parity**: Close the capability boundary by mechanism, not by a fifth list
- **list**: Run the pre-registered phase-1 gate, and let criterion 13 change the design
- **list**: Phase 1 passes its pre-registered gate, 24 of 24
- **list**: Plant the event axis, because deleting a source that was never there proves nothing
- **list**: Phase-3 rework — a capability boundary instead of a disconnected differential
- **list**: Give criterion 1 its opposed control
- **list**: A comment names what the test injects, not what the build lacks
- **list**: Retarget phase-3 criterion 3 onto the live gate
- **list**: Stop pinning open JSON field order and incomplete-human rc
- **522**: One clock stamps the document and decides the attention in it
- **522**: Scope the guarantee to what the document can contradict
- **518**: The held-out stderr rows assert the successor side rather than skipping it
## [v0.2.1] - 2026-03-06

### Bug Fixes

- Fix integration assertions for agent identity

### Other

- Add opencode resume support
## [v0.2.0] - 2026-03-02

### Bug Fixes

- Fix release: make gh release creation best-effort

Git tag push via SSH works regardless of gh auth. The GitHub release
creation is now optional — logs a warning instead of failing the pipeline.
- Fix agent launch: add delay between paste and C-m submit

The default agent launch path (Claude Code) used paste-buffer
followed immediately by send-keys C-m. On large commands (long
--append-system-prompt payloads), the paste hasn't finished
rendering before C-m fires, causing the agent to never start.
Add 0.3s delay matching the send helper pattern.
- Fix heartbeat: select-pane before paste for codex TUI compat

Codex TUI requires pane focus to process Enter after paste-buffer.
Add select-pane with focus restore to hb_send, matching the send
helper pattern.
- Fix send reliability across codex and claude

### Other

- Improve agent prompt: add concurrent collaboration awareness

Agents now know other agents are editing files simultaneously.
Unexpected modifications trigger verification (send) before
reverting, not blind acceptance. Clarify peek is for inspecting
work state, not polling for replies.
- Add heartbeat: background daemon detects stale/dead agents

Polls panes every 60s, checks alive via pane_current_command and
output freshness via capture-pane checksum. Dead agents trigger
tmux alerts; stale workers get nudged (max 2), then human alert.
Background-safe send (no focus switch), self-terminates when
session disappears. Configurable via AE_HEARTBEAT_INTERVAL_SEC
and AE_HEARTBEAT_STALE_MIN env vars.

### Refactoring

- Atomic tmux paste-and-submit, eliminate race condition
## [v0.1.1] - 2026-02-25

### Bug Fixes

- Fix agent send-keys instructions in workspace manifest

Use Enter instead of C-m and add explicit wrong/right examples
so agents keep the Enter key outside the quoted message string.
- Fix send helper: use literal text (-l) and C-m for reliable submit

The previous helper sent text with `Enter` key name which is unreliable
in TUI apps. Now uses `-l` flag for literal text injection and a separate
`C-m` (carriage return) for submit. Also fixes argument handling to
capture full multi-word messages and updates manifest to direct agents
to always use the helper instead of raw tmux send-keys.
- Fix codex resume: don't pass prompt as argument

codex resume --last doesn't accept inline prompts — the prompt was being
interpreted as a session ID. Now launches codex resume first, then sends
the prompt as user input after a delay. Guards against resume failure by
checking codex is still running before sending.
- Fix review findings: quoting bug, stale docs, test robustness

- send_agent_cmd: use buffer-paste with escaped single quotes to
  prevent prompt quoting breakage (Codex review IMPORTANT #1)
- AGENTS.md: fix stale "worktree default" → local is the default
- README: local sessions now survive reboots, clarify agent resume
- test: remove head-200 brittleness, match "func() {" to skip
  heredoc copies, add sanitize_branch_name and default_session_name
  tests (43 total)
- Move regenerate_manifest above dispatcher so spawn works
- Fix claude nesting detection: use env -u instead of bash-only unset

unset is bash syntax — breaks in fish shell tmux panes. env -u is
POSIX and shell-independent.
- Fix send/spawn Enter delivery: use C-m, increase paste delays

Enter key name can be remapped by tmux; C-m is the raw carriage
return that always works. Increased pre-submit delay to 0.3s for
TUI ingestion, added post-submit delay in send to keep focus while
target processes input.
- Fix spawn: wait for new pane shell init before sending launch command

split-window returns immediately but the shell in the new pane may
not be ready to accept input yet, causing paste-buffer to miss the
target pane.
- Fix send: serialize concurrent sends with flock

Concurrent sends to the same target pane could interleave paste and
C-m steps, causing messages to appear pasted but not submitted.
Add per-target flock serialization keyed by pane ID. Replace EXIT
trap with explicit focus restore to avoid racing with C-m delivery.
- Fix helpers: honor AE_TMUX_SERVER, filter non-agent panes

All session helpers (send, peek, agents, focus) now read tmux_server
from meta and wrap tmux with -L flag when set. Spawn exports
AE_TMUX_SERVER for the child ae process. Agents helper uses pipe
delimiter to correctly skip panes without @ae_agent set.
- Fix retire: validate pane-id belongs to session, prevent cross-session kills

Pane-ID targets now resolve through session pane list instead of
direct tmux access, preventing accidental kills of panes from other
sessions. Also use grep -Fv for fixed-string meta removal and update
manifest docs to show pane-id support.
- Fix integration tests: use ae end -f to skip confirmation prompt

All ae end calls in integration tests now pass -f flag to bypass
the interactive confirmation prompt that was causing 4 test failures.
22/22 integration tests passing.
- Fix resume: restore config, mode, and CWD from session meta

Claude Code's --resume is CWD-scoped — sessions are stored under
~/.claude/projects/<encoded-CWD>/. When ae resumed from a different
directory, both --resume UUID and --continue failed silently, starting
agents fresh instead of resuming conversations.

- Restore CONFIG_FILE and AE_LOCAL_CONFIG from meta before agent
  alias resolution (prevents "agent not defined" on cross-dir resume)
- Restore COPY_MODE from meta when no CLI flag override (prevents
  mode drift between original start and resume)
- Restore WORK_DIR from meta in local mode so tmux panes get the
  correct CWD (the primary fix for Claude Code session lookup)
- Restore ORIGIN_DIR from meta in all modes (worktree cleanup and
  env vars depend on it)
- Fix env -u CLAUDECODE prefix bug in resume fallback chain: was
  using $cmd instead of $launch_cmd, losing the nesting guard
- Fix lint and format: shfmt auto-format, shellcheck clean, add Developer section to README

Apply shfmt canonical formatting (redirect spacing, arithmetic, case alignment).
Fix all shellcheck warnings: suppress false positives (SC2015, SC2001, SC2034),
remove dead code (unused kind/MAIN_TOOL_KIND vars), fix real issues (SC2059 printf
format, SC2004 array index). Add Developer section to README with dev tooling info.

### Documentation

- Add session helpers to README and AGENTS.md

### Other

- Initial commit
- Initial release

tmux-based multi-agent workspace launcher with shared awareness.
Agents discover each other via .ae/workspace.md manifest and
communicate through tmux send-keys/capture-pane.
- Add project docs and improve installer

- AGENTS.md with structure, design decisions, and rules
- CLAUDE.md referencing AGENTS.md
- install script handles ./install, curl|bash, and missing parent dirs
- README install section updated with curl one-liner
- Add badges to README
- Add named sessions, session tagging, and ae list improvements

- ae <name> creates/reattaches named sessions (not just auto-generated)
- Tag sessions with AE_SESSION/AE_DIR env vars for reliable listing
- ae list shows directory column
- ae kill all uses env var tags instead of prefix matching
- Rewrite AGENTS.md to emphasize simplicity philosophy
- Guard against hijacking non-ae tmux sessions

- ae <name> refuses to attach if existing tmux session lacks AE_SESSION tag
- add .gitignore for .ae/ and .local/
- Revise README title and description

Updated project title and description in README.md.
- Isolate workspace manifests per session

Write to .ae/<session>/workspace.md instead of .ae/workspace.md so
multiple sessions from the same directory don't overwrite each other.
Single-quote the initial prompt to prevent shell expansion of session names.
- Add hardlink worktree isolation for all sessions

Every ae session now works on a hardlink copy at ~/.ae/worktrees/<session>/.
Agents work on the copy, push to remote, merge from there.

- Worktrees stored in ~/.ae/worktrees/ (invisible to user)
- Config validated before creating worktree (no orphaned dirs)
- Stale worktrees auto-cleaned on session start
- ae kill removes worktree on cleanup
- ae list shows origin directory
- Set tmux window name to session name
- Add send helper script to fix agents not pressing Enter
- Use absolute path for send helper in manifest
- Add session resume across reboots

Worktrees persist on disk at ~/.ae/worktrees/. Running ae <name> again
after reboot detects the existing worktree and resumes agents with their
previous conversation context (claude --continue, codex resume --last).

- ae list shows running and stopped (resumable) sessions
- ae kill handles stopped sessions (worktree-only cleanup)
- ae kill all cleans both running sessions and stopped worktrees
- Sanitize kill target to prevent path traversal
- Replace hardlink copy with git worktree default and full copy opt-in

cp -al shared inodes so agent edits could corrupt originals. Replace
with git worktree (detached HEAD) as default and cp -a as opt-in via
--full flag. Add session metadata for mode-aware cleanup, MODE column
in ae list, copy mode validation, and improved send helper that
focuses pane before paste+Enter for reliable TUI input.
- Add local mode and rename flags to --worktree/--copy/--local

New --local flag runs agents directly in the current directory
without any copy or worktree. Rename --git to --worktree and
--full to --copy for consistency across three modes. Store
AE_MODE in tmux env so ae list/kill work for local sessions
which have no on-disk worktree directory.
- Single-agent default with on-demand spawn helper

Start with just the main agent, spawn more on demand via
.ae/<session>/spawn <alias> [prompt]. Workers config still
works for fixed layouts but is no longer in the default config.

- rename default aliases to full names (claude/codex/opencode)
- add spawn helper with safe meta parsing and buffer-paste prompt
- regenerate workspace.md from live tmux panes (ae: prefix filter)
- extend meta file with session/work_dir/layout/config/main_pane
- always refresh dynamic meta fields on resume (pane IDs change)
- include spawn instructions in workspace.md and initial prompt
- Replace kill with end (commit+push+cleanup) and discard

ae end: auto-commits dirty state, pushes to ae/<session> branch,
then removes the tmux session and worktree/copy. Preserves session
on commit or push failure. Local mode just kills tmux.

ae discard: destroys session without saving (old kill behavior).
ae kill: deprecated alias to discard with warning.
- Move session state to ~/.ae/sessions/, keep working dirs clean

Session metadata, helpers (send/spawn), and workspace.md now live
in ~/.ae/sessions/<session>/ instead of <workdir>/.ae/<session>/.
Working directories stay clean — no .ae/ pollution, no gitignore
needed. Agents use fully-expanded absolute paths from the manifest.

Backward-compat: read_session_meta falls back to old worktree-nested
path for existing sessions. cleanup_session removes legacy paths.
- Switch default mode from git worktree to local
- Session-scoped agent resume across reboots

Thread a unique UUID per agent pane through the full session lifecycle:
generate on first start, persist in meta, inject into agent CLI flags,
and restore on resume. Claude Code uses --session-id/--resume, Codex
gets post-launch capture with flock-serialized meta writes, unknown
agents fall back to fresh start.

Also: local mode now detects existing sessions for resume, flag
stripping uses whole-token matching, and gen_uuid has no python
fallback (bash/tmux/git only per project rules).
- Add test suite for pure functions

34 assertions covering strip_session_flags, resume_cmd_from_cmd,
inject_session_id, tool_kind_from_cmd, tool_name_from_cmd, and
gen_uuid. Pure bash, no test framework dependency. Extracts functions
from ae via awk and tests them in isolation.
- Harden ae: health check, spawn self-invoke, spawn resume, integration tests

1. ae list shows agent health (alive/total, ! for crashed agents)
2. spawn refactored from declare-f heredoc to ae _spawn self-invocation,
   eliminating function inlining drift risk
3. spawned agents persist in meta and survive reboot with session-scoped
   IDs, flock-serialized writes, and codex capture support
4. 18 integration tests using isolated tmux server (AE_TMUX_SERVER),
   covering lifecycle, resume, health check, spawn persistence, and
   end-session workflows

Also moved resolve_agent_session_id and capture_codex_session_id to
top-level function block for availability across all code paths.
- Sharpen docs: emphasize simplicity, fix stale spawn info

AGENTS.md: add "What ae is NOT" section, line count cap (~1500),
strengthen philosophy ("simplicity is the feature"). README: rewrite
opening to lead with the value prop (one command, everything resumes),
fix stale note about spawned agents being ephemeral (they now persist).
- Add status/end-without-name/project-config, system prompt injection

- ae status [name]: show recent agent output without attaching
- ae end/discard/status auto-detect current session from $TMUX
- per-project config: .ae/config in project dir shadows global
- inject ae workspace context into system prompt (Claude Code
  via --append-system-prompt, Codex via -c developer_instructions)
  so agents retain ae awareness through context compaction
- slim initial prompt (system prompt carries all instructions now)
- move tests to tests/unit and tests/integration
- Add ae stop: pause session for later resume

Kills tmux session but preserves all meta — next ae <name>
resumes with all agents (main, workers, spawned) restored.
- Drop initial prompt on fresh start, system prompt is sufficient

Agents with system prompt injection (claude, codex) start
interactive — no busywork reading workspace.md on first turn.
Resume still sends a short nudge about changed pane IDs.
- Drop resume initial prompt too, system prompt is sufficient
- Name agent panes ae:<alias>:<name> for clearer identification

Main pane: ae:claude:main, workers: ae:codex:worker-0, spawned:
ae:claude:reviewer (user-named) or ae:claude:spawned-0 (auto).
Spawn syntax: spawn <alias>[:<name>] [prompt]. Manifest, meta,
and resume all parse the new format with backward compat for old
sessions. Spawn index scan + auto-naming moved inside flock to
prevent races.
- Agents address each other by name instead of raw pane IDs

Send helper resolves agent names (claude:main, codex:worker-0) to
pane IDs by scanning titles. System prompt and workspace.md tell
agents to use names. Pane border strips ae: prefix for cleaner
tmux display. ae status shows clean names too.
- Config-driven agent names, @ae_agent pane option, encourage creative naming

Config supports alias:name (e.g. main=claude:lead, workers=codex:reviewer).
Default name is the alias itself. Duplicate names auto-deduplicated.

Agent identity stored in tmux pane option @ae_agent — immune to title
overrides by tools like Claude Code. All scanning (manifest, health,
status, send) uses @ae_agent. Border display uses it too.

System prompt and workspace.md encourage descriptive names when spawning
(codex:reviewer, claude:pair-programmer). Auto-fallback: helper-N.
Role labels: lead/agent instead of main/worker/spawned.
- Rewrite README: streamlined, focused on real workflow

Drop verbose sections (modes table, session management list, workspace.md
internals). Lead with why-ae bullet points, show 4 real use cases, keep
config and commands compact. Reflects current state: named agents,
system prompt injection, reboot persistence, clean repos.
- Natural language for collaboration, document copy modes
- Explain how inter-agent communication works under the hood
- Configurable [prompt] instructions injected into agent system prompts
- Opencode support: inject workspace context as emphasized initial message
- Gemini cli support: context injection via -i, resume via --resume latest

Gemini gets workspace context through -i (prompt-interactive) flag.
Resume uses --resume latest (index-based, no UUID scoping).
Gemini-specific strip_gemini_prompt_flags() avoids breaking -i on
non-gemini commands.
- Ae list: show TARGET column for copy/worktree working directories
- Unified agent meta format, per-agent ae list, resilient resume, codex self-registration

- unified meta: agent.SLOT=alias:name:session_id replaces separate spawned.N + agent_session.N entries
- ae list: per-agent rows with truncated session IDs and idle markers, columnar layout with indented target
- resilient resume: claude --resume UUID || --continue fallback, codex resume || fresh start
- codex self-registration: register-sid helper script with slot-scoped sid files (prevents race conditions)
- preserve config flags (e.g. --yolo) through codex resume path
- colon validation: reject agent names containing ':' in main, worker, and spawn paths
- collapse discard/kill into end (ae end|rm is the only exit command)
- AGENTS.md: agent tool capabilities table documenting session/resume/prompt differences
- fix: shell-quote injection in codex developer_instructions when meta_dir contains single quotes
- fix: send_agent_cmd defined before dispatcher so _cmd_spawn can call it
- Ae <name> use <alias> CLI override, drop discard command, update docs

- ae <name> use <alias>: override main agent from CLI without editing config
- remove discard/kill commands — ae end|rm is the only exit path
- update README: document use syntax, replace discard references, fix ae list format
- integration test for use override (pane title + meta assertion)
- Config parser: allow hyphens, fix resume/codex/gemini, ae end safeguard

- parse_config: allow hyphens in section names and keys (gemini-flash etc)
- resume: read agent.main from session meta to preserve 'use' override
- codex: send initial "Go" prompt to trigger developer_instructions
- gemini: add "wait for task" instruction to -i context injection
- ae end: interactive y/N confirmation (single keypress), -f to bypass
- Reply-back communication pattern, fix claude nesting detection

- build_ae_context: teach agents to reply via send instead of polling capture-pane
- workspace.md: document reply-back pattern as primary communication flow
- spawn: resolve caller agent name, include reply-back instruction in spawn prompt
- send_agent_cmd: unset CLAUDECODE env var so claude launches from inside ae sessions
- Resolve bare agent names (e.g. send "lead" instead of "claude:lead")
- Add peek helper, fix local-outside-function bug in send

Add peek session helper: thin wrapper around tmux capture-pane with
agent name resolution. Supports bare names, numeric line count with
clamping (default 80, max 2000). Documented in workspace manifest.

Fix send helper: remove `local` keyword used outside a function in
the name resolution loop (caused errors in bash strict mode).
- Add agents and focus session helpers

agents: list all agents in session with pane ID and process name.
focus: switch to another agent's pane by name, with same name
resolution as send/peek (exact alias:name + bare name fallback).
Both documented in workspace manifest.
- Add retire helper: clean removal of spawned agents

retire kills the pane, removes the agent.spawned entry from meta
(flock-protected), rebalances layout, and regenerates the manifest.
Guards against retiring main or worker agents. Implemented as
ae _retire internal command with thin helper script, matching the
spawn pattern.
- Add interrupt helper: cancel agent generation with optional redirect

Single Escape to interrupt (safe across all TUIs — double-Escape
triggers edit/rewind on Claude, Codex, Gemini). Shares per-target
flock with send to prevent interleaving. Optional message delivered
inline after 0.5s delay. Documented in manifest, README, AGENTS.md.
- Add ask helper, expand agent system prompt with all helpers

The injected system prompt (build_ae_context) only mentioned send and
spawn. Agents didn't know about peek, agents, focus, interrupt, or
retire — limiting their ability to collaborate effectively.

- Expand build_ae_context to list all 8 session helpers with brief
  descriptions
- Add `ask` helper: thin wrapper around send that auto-detects
  caller identity via @ae_agent and embeds reply-to metadata in the
  message, making request-response between agents reliable
- Fall back to plain send if caller identity can't be detected
- Use alias:name (not bare name) in reply-to for unambiguous routing
- Update OpenCode initial prompt to reference helpers generically
- Document ask in AGENTS.md helper table
- Add justfile pipeline, version support, ask helper, expanded agent prompt

- Add justfile with check/lint/test/release pipeline (SemVer, git-cliff
  changelog, shellcheck, shfmt, GitHub releases via gh)
- Add AE_VERSION constant and ae version/--version command
- Add cliff.toml for git-cliff with SemVer tag pattern
- Add ask helper: structured send with reply-to metadata so agents
  reliably respond back to the asking agent
- Expand build_ae_context to list all 8 session helpers (was only
  send + spawn, agents didn't know about peek/agents/focus/interrupt/retire)
- Add version badge to README, document ask helper
- Update AGENTS.md structure section

### Refactoring

- Extract subcommands into named functions
- Refactor helpers into shared _lib, add cross-session communication

Extract duplicated resolver/lock logic from all helpers into _lib shared
library. Add @session:agent syntax for cross-session send/peek/focus/interrupt.
Add agents --all for cross-session discovery. Lock files now use shared
~/.ae/sessions/.locks/ dir for correct cross-session serialization.

Net -42 lines despite new features.
