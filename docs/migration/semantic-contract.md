# Semantic contract (rust-rewrite)

Deliverable 1 of #81, part of epic #79. Status: **DRAFT — not ratified.**

**Ratification gates (all explicit, all hard):** zero unclassified rows; zero unresolved
conflicts; every open bash-era issue carries a migration disposition on #79; ratification
recorded by BOTH seats as comments on #81.

## Row schema (ratified ruling, ae-20260820T075300Z-49abbb8b)

No flat bucket column. Every row carries orthogonal fields:

| Field | Meaning | Who decides | Who writes this file |
|---|---|---|---|
| `id` | stable behavior id (`SC-NNN`) + surface | seats | lead (file owner) |
| `normative` | exactly one bucket (below) + authority citation with `source_role=normative` | seats only | lead |
| `empirical` | observed behavior + evidence pointer (probe run / format dump / test pin) | builders collect evidence; seats accept it | lead transcribes — builders never write this file |
| `conflict` | `none` \| `fix-known-defect(#issue, intended Rust behavior)` \| `DR-NNN` | seats only | lead |
| `classified_by` | both seats (`fable5:lead` + `gpt56sol:colead`) | seats | lead |

One-writer rule: builders deliver evidence as probe outputs/files with commit +
environment + exit-code provenance; the file owner transcribes and cites them. No builder
edits this document.

Consistency constraints enforced at ratification:

- Bucket 3 requires `conflict=fix-known-defect` with issue link AND intended behavior.
- Bucket 4 requires a `DR-NNN`.
- Buckets 1–2 require `conflict=none`; a conflict there must resolve to bucket 3 or a DR before ratification.
- Unclassified = missing `normative`, missing authority, or unresolved `conflict`. Zero at ratification.

**Row grain (ratified ruling, ae-20260820T075852Z-b9c95acb):** one row = one
independently testable behavior/expectation — never one command or surface. A surface owns
many rows (e.g. `ask`: signature promise, payload promise, and each #66 defect are
separate rows). Each row carries exactly one SHOULD claim, one IS observation, one
resolution.

## Source roles — role is the lane, rank is not (gate finding fe7cfc2e, blocker 1)

Every source in #81's authority index carries a `source_role`. **Only
`source_role=normative` sources may freeze SHOULD.** Measured facts, tests, and code
establish IS — regardless of how high their tier ranks for reliability.

| Source | source_role | Notes |
|---|---|---|
| Issue rulings (#59 closing comment + 72c7293 commit message; #71 "DURABLE HANDOFF RECORD"; rulings recorded on #79/#80/#81) | normative | claim-level rulings |
| `docs/internals/architecture.md` (+ other doc contracts: events.md, bridge-protocol.md, watchdog.md, monitor.md) | normative | doc contracts |
| `docs/gatekeeping.md`, `docs/design-patterns.md` | normative | doctrine (constrains HOW, rarely a behavior claim) |
| AGENTS.md@72c7293 ruling bullets (name allowlists, provenance model) | normative | indexed as rulings, cited per bullet |
| AGENTS.md@72c7293 measured sections (tool matrix, TUI timings, hazards exhibits) | **empirical** | high-reliability IS, never SHOULD |
| `tests/unit` (2150) + `tests/integration` (811) @72c7293 | **empirical** | always empirical — a test never becomes normative. When a test traces to a ruling/doc contract, THAT traced source is the row's normative authority; the test remains its empirical verification, cited on the row |
| Code @72c7293 | **empirical** | observation only |

**Lane discipline (ratified, ae-20260820T075607Z-f0d46071):** seats freeze SHOULD from
normative-role sources **before** reading probe evidence. Builder probe briefs carry **no
expected values** and must report commit + environment + exit-code provenance. Behaviors
with no normative-role source at all are flagged `authority=code-observation`, classified
jointly after evidence, with heightened defect/DR scrutiny — no pretend-frozen SHOULD.
**Normative closure (gate finding b29dac92):** such a row ratifies only after the seats
issue an explicit normative **seat ruling** — preserve / fix / diverge — recorded on the
row (as a DR when diverging); the seat ruling becomes the row's normative authority, and
`code-observation` remains only its empirical pointer. No row ratifies with an
empirical-only lane.

**Authority freeze:** the empirical baseline and all @-references above are pinned at
commit **72c7293** (the bash freeze), never branch HEAD.

## Buckets

1. **Normative invariant** — must hold in Rust.
2. **Compatibility promise** — CLI surface, exit codes, stdout contracts, formats, layout.
3. **Observed behavior / known defect** — parity must NOT freeze these; each carries its issue link and intended Rust behavior.
4. **Deliberate divergence** — numbered decision record (DR register below).

## Surface inventory

Families verified against the dispatcher/helper/env symbols in `ae` at 72c7293 (grep
census 2026-08-20); completeness map contributed by gpt56sol:colead (memo
2026-08-20T07:57:13Z). Rows are drafted here and classified jointly; `UNCLASSIFIED`
markers are legal only while drafting.

**Per-family completeness rule:** each family's rows must make **default/unset**,
**malformed-input**, and **partial-failure** modes explicit — a family with only its
happy path inventoried is incomplete.

### S1 — Dispatcher CLI + start grammar

Launch/attach/resume decision (`ae [name]`, `--local/--copy/--worktree`, `--from <uuid>`),
`list [--json]`, `status`, `next`, `end`/`rm` (`--purge-history`), `stop`, `rename`,
`transfer`, `compact`, `archive preview` (two words), `_recover-pending` (internal,
helper-invoked — not public surface), `doctor [--refresh]`, `watchdog …`,
`telegram setup|start|stop|status`, `steward --init|--attach|--help|--detach` (flags, not
subcommands; deprecated alias `hub`), `help`/`version`, exit codes, refusal/error
contracts. (Census: `cmd_*` functions + dispatcher arms at 72c7293.)

<!-- rows: SC-0xx -->

### S2 — Session modes and states

`--local/--copy/--worktree`; fresh / resume / stopped / inside-session / `--all` /
default-name resolution (`default_session_name` guarantees the grammar).

<!-- rows: SC-1xx -->

**SC-100 — a derived default session name GUARANTEES the grammar.** Bucket 1 —
`default_session_name` produces a valid name for ANY working directory rather than
being checked against the grammar afterwards. Authority: AGENTS.md session-name bullet
(ruling). Empirical: unit pins @72c7293. Conflict: none.

**SC-101 — `ae <name>` on a running session is a pure attach.** Bucket 2 — the
fast path attaches without taking the lifecycle lock or mutating session state
(autostarts excepted, censused). Authority: architecture.md start-vs-resume +
census-2 launch section. Empirical: census-2. Conflict: none.

**SC-102 — remaining mode/state transitions.** `authority=code-observation` — the
fresh/resume/stopped decision matrix beyond SC-101 and the S9 rows needs probe + seat
ruling at the sweep. UNCLASSIFIED pending closure.

(S1's dispatcher exit/refusal rows live in S6 SC-513..517; its transaction rows in S9;
the public command surface itself is the censused `cmd_*` set — S1 header.)

### S3 — Generated helper CLIs (every one — census: `helper_*_main` at 72c7293)

`send`, `ask`, `review`, `reply`, `requests`, `state`, `mark-done`, `say`, `memo`, `goal`,
`peek`/`peak`, `agents [--all]`, `focus`, `interrupt`, `spawn`, `retire`, `loop`,
`watchdog`, `events-tail`, `_register-sid`, `_lib` name resolution (`alias:name`,
alias-only, bare name, `%pane-id`, `@session:agent`), delivery semantics (defer on
busy/human-typed, verify submit, fail loud).

<!-- rows: SC-2xx -->

**SC-200 — delivery-model evolution.** Bucket 4 — **DR-004** (ratified): the durable
per-agent inbox with coalesced notification replaces the paste-delivery model at P2;
the paste rows stand until that cutover. Authority: DR-004 (both seats).
Conflict: DR-004.

**SC-201 — dead-pane refusal.** Bucket 1 — a target pane fallen to a shell is refused
with the named reason; nothing is pasted (a stray Enter would execute the message).
Authority: helpers.md "How send delivers" 1. Empirical: pending. Conflict: none.

**SC-202 — busy/human-input defer is fail-closed.** Bucket 1 — for a modelled TUI,
send waits while the input box is non-empty, mid-generation, or unreadable, and
ABANDONS loudly rather than clobbering a half-typed human question. Authority:
helpers.md 2. Empirical: pending. Conflict: none.

**SC-203 — submit is verified.** Bucket 1 — after pasting, send confirms the text left
the input box (bounded Enter nudges); unconfirmable delivery fails loudly as
UNCONFIRMED. Authority: helpers.md 3. Empirical: pending. Conflict: none.

**SC-204 — no durable outbox (until DR-004).** Bucket 4 — DR-004: at 72c7293 a loud
failure is the re-send signal ("ae is not a queue"); the P2 inbox makes the store the
transport and this promise retires. Authority: helpers.md 4 + DR-004.
Conflict: DR-004.

**SC-205 — one helper touches tmux.** Bucket 1 — only `send` pastes; ask/review/reply/
interrupt deliver through it and inherit every guard. Authority: helpers.md
composition. Empirical: pending. Conflict: none.

**SC-206 — one path mints request ids.** Bucket 1 — `ae_tracked_send` is the single
mint point. Authority: helpers.md composition. Empirical: pending. Conflict: none.

**SC-207 — one validator pairs replies.** Bucket 1 — reply pairing is verified in one
place before delegation to send. Authority: helpers.md composition.
Empirical: pending. Conflict: none.

**SC-208 — every interaction crosses one emit point.** Bucket 1 — the surface is
auditable in events.jsonl because all messaging passes the same emit call. Authority:
helpers.md composition. Empirical: pending. Conflict: none.

**SC-209 — the slot is the routing key.** Bucket 1 — requests and replies are
addressed and VERIFIED by slot + session; the display name is never trusted for
routing (`--as` is display only); slot survives display-name churn. Authority:
helpers.md "Slot identity". Empirical: pending. Conflict: none.

**SC-210 — unmodelled tools receive without busy protection.** Bucket 2 — documented
degradation (only claude/codex expose a reliable input-state read at 72c7293); the
per-tool boundary is empirical. Authority: helpers.md closing note. Empirical: matrix.
Conflict: none.

### S4 — Config INI grammar

Dialect, key grammar, defaults, precedence (incl. `AE_LOCAL_CONFIG`), malformed-line
behavior.

<!-- rows: SC-3xx -->

**SC-300 — the config format is the closed four-section INI dialect.** Bucket 2 —
`[agents]`/`[workspace]`/`[prompt]` (+ `[telegram]` written by setup), simple regex
parse, and the AGENTS.md rule: don't extend the format (no TOML/YAML/JSON). Authority:
AGENTS.md config section (ruling) + config.md. Empirical: pending. Conflict: none.

**SC-301 — an `[agents]` alias value is the launch command, doctor-verified.** Bucket
2 — the executable name is extracted and verified on PATH by doctor. Authority:
config.md `[agents]`. Empirical: pending. Conflict: none.

**SC-302 — env-prefixed alias commands are legal identity aliases.** Bucket 2 — one
binary, several logins via inline env prefix; each alias is its own identity.
Authority: config.md multiple-identities section. Empirical: pending. Conflict: none.

**SC-303 — env-prefixed commands get full tool handling.** Bucket 3 — SHOULD:
classification follows the actual executable (SC-705), so identity aliases keep
session ids and exact resume. IS at 72c7293: raw prefix match degrades them to
generic-tool handling — documented in config.md itself with its issue.
Conflict: fix-known-defect(#32, intended per SC-705/DR-005). Empirical: config.md
limitation note + matrix.

**SC-304 — per-project `.ae/config` overrides the global for `[prompt]`.** Bucket 2.
Authority: config.md `[prompt]`. Empirical: pending. Conflict: none.

**SC-305 — a renamed/wrapper binary is deliberately an unknown tool.** Bucket 2 — ae
keys on the exact executable name; wrappers get no session machinery. Authority:
config.md trap note. Empirical: pending. Conflict: none.

**SC-306 — the three copy modes are local (default) / full / worktree.** Bucket 2.
Authority: config.md copy modes. Empirical: pending. Conflict: none.

### S5 — On-disk formats (live + archive + claims)

`~/.ae/sessions/<name>/` layout (`meta`, `events.jsonl`, `memo.tsv`, request records,
locks, `workspace.md`, generated helpers), `~/.ae/archive/<uuid>/` (inert; #48 format +
digests), claims (#71 — first Rust-native), `launch.<slot>.sh` + `.started` marker,
hub/steward dirs (`AE_HUB_DIR`, `AE_STEWARD_DIR`).

<!-- rows: SC-4xx -->

**SC-400 — the live session layout is the documented file set.** Bucket 2 — session
state lives under `~/.ae/sessions/<name>/`: `meta`, `events.jsonl`, `memo.tsv`,
`messages/`, lock files, `workspace.md`, generated helpers, `launch.<slot>.sh` (+
`.started`). Authority: architecture.md per-session state + AGENTS.md. Empirical:
census-1/2. Conflict: none.

**SC-401 — the archive payload is the five-part set.** Bucket 2 — generated `meta`,
rendered `digest.md`, `memo.tsv`, `events.jsonl`, `messages/*` bodies (#48 format;
inertness proofs are SC-804/805). Authority: architecture.md:77-83. Empirical:
census-2 end section. Conflict: none.

**SC-402 — working directories stay clean.** Bucket 1 — ae writes its coordination
state under `~/.ae`, never into the project tree (`.ae/config` is the one deliberate
project-side file). Authority: AGENTS.md rules. Empirical: pending. Conflict: none.

**SC-403 — request-record framing uses non-whitespace separators with free text last.**
Bucket 1 — the `\x1f` framing rule after #48: an empty TSV field must not shift its
row. Authority: AGENTS.md TSV-framing ruling. Empirical: unit pins @72c7293.
Conflict: none.

**SC-404 — state roots are `~/.ae/config`, `~/.ae/sessions/`, `~/.ae/archive/`.**
Bucket 2. Authority: AGENTS.md + architecture.md. Empirical: pending. Conflict: none.

### S6 — Stdout/stderr/exit/refusal contracts

`list --json` schema, compact's four-line stdout, event JSON shape (`_event_json_str`),
validation-error grammar echoes (`_validate_session_name`, `_validate_agent_name`).

<!-- rows: SC-5xx -->

Rows below: SHOULD frozen from normative sources BEFORE census consultation (lane
discipline); `empirical` cites audited census/test evidence or is `pending`.
Revised per gate e3516d34 (sources extended to docs/reference/commands.md +
docs/internals/events.md — a lead lane miss the gate caught); split per its fold
verdict.

**classified_by: SC-500..517 and all their letter-splits EXCEPT SC-508 —
fable5:lead + gpt56sol:colead, 2026-08-20. SC-508 is explicitly UNCLASSIFIED until its
evidence probe + joint closure.**

**SC-500 — compact stdout byte format.** Bucket 2 — `Archived`, `Archive:`, `Digest:`,
`Recovery:`: four lines, that order, nothing else, and EMPTY unless the boundary was
crossed. Authority: architecture.md + commands.md:673-676. Empirical: pending.
Conflict: none.

**SC-512 — compact stdout truth claim.** Bucket 2 — non-empty stdout proves exactly:
the archive EXISTS and the printed recovery command WORKS. It deliberately does NOT
claim the fresh session started (the relaunch can still refuse; a stdout line asserting
a launch that then failed would be worse than no line). Authority: commands.md:673-676 +
architecture.md. Empirical: pending. Conflict: none.

**SC-501 — compact stderr carries everything else.** Bucket 2 — frozen facts,
confirmation + question, end's progress, handover chatter, `Aborted.`, the relaunch
announcement, and a SECOND copy of the `Recovery:` line (a broken stdout cannot destroy
the only route back). Authority: commands.md:678-683. Empirical: pending. Conflict: none.

**SC-502 — `Recovery:` prints BEFORE the relaunch.** Bucket 1 — past the relaunch the
archive is published, the source is gone, and the process may exec and never return.
Authority: architecture.md. Empirical: pending. Conflict: none.

**SC-503a — a typed `n` is an answer.** Bucket 1 — prints `Aborted.` and exits **0**.
Authority: commands.md:692-697. Empirical: pending. Conflict: none.

**SC-503b — end-of-input is not an answer.** Bucket 1 — with no stdin, compact reports
it could not obtain confirmation and exits **non-zero**; stdout is empty in both cases,
so exit status is the caller's only way to tell "operator said no" from "the question
never reached anyone". Authority: commands.md:692-697. Empirical: pending. Conflict: none.

**SC-504a — a reporting failure never suppresses the relaunch.** Bucket 1 — a consumer
that exits early (closed/broken stdout) must not kill the operation between archive and
launch. Authority: commands.md:685-686 + architecture.md. Empirical: pending.
Conflict: none.

**SC-504b — no altered SIGPIPE disposition leaks into the child.** Bucket 1 — semantic
SHOULD, narrowed per fold guard: the child sees normal/unmodified SIGPIPE behavior; the
authority guarantees restoration of SIGPIPE specifically, not that every disposition is
default. The parent's mechanism (ignore/restore) is implementation, not contract.
Authority: architecture.md. Empirical: pending. Conflict: none.

**SC-505a — session-name validation error echoes its grammar verbatim.** Bucket 2 —
the message states `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$` exactly. Authority: #59 ruling +
AGENTS.md session-name bullet. (The bash one-definition structure is mechanism, not Rust
SHOULD.) Empirical: unit pins @72c7293 (verification). Conflict: none.

**SC-505b — agent-name validation error echoes its grammar verbatim.** Bucket 2 —
`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`. Authority: #59 ruling + AGENTS.md agent-name bullet.
Empirical: unit pins @72c7293. Conflict: none.

**SC-506 — `list --json` partial-failure validity.** Bucket 1 — one bad session degrades
its own entry; the document always closes, never truncates. Authority: AGENTS.md ruling
bullet (long emitters must not abort mid-output; structural guard). Empirical: structural
unit guard @72c7293. Conflict: none.

**SC-509 — `list --json` versioned object schema.** Bucket 2 — a single JSON object:
`schema_version` (1), `generated_at`, `sessions[]` with the documented session fields
(name/status/mode/origin/work_dir/goal/goal_set_epoch/branch/last_active_epoch/
needs_attention/attention/attention_rank) and `agents[]` fields (ref/alias/name/
session_id/alive/state/reason); `schema_version` lets consumers gate on shape.
Authority: commands.md:97-132. Empirical: pending. Conflict: none.

**SC-510a — event required keys.** Bucket 2 — every event carries `ts` (ISO 8601 UTC,
second precision), `actor`, `action`. Authority: events.md:47-60. Empirical: pending.
Conflict: none.

**SC-510b — optional keys are omitted when empty.** Bucket 2 — `target`/`ref`/`summary`
never appear as empty strings. Authority: events.md:49. Empirical: pending.
Conflict: none.

**SC-510c — `ref` polysemy follows the action table.** Bucket 2 — request id for
ask/review/reply, topic for memo, captured session id for recover, absent otherwise.
Authority: events.md:62-68. Empirical: pending. Conflict: none.

**SC-510d — string values are JSON-escaped.** Bucket 2 — the escape set is `\"` `\\`
`\n` `\t` `\r`. Authority: events.md:70. Empirical: pending. Conflict: none.

**SC-511a — messaging events carry optional routing-key fields.** Bucket 2 —
`actor_slot`/`actor_session`/`target_slot`/`target_session` on send/ask/review/reply
when known, omitted when empty. Authority: events.md:71-84. Empirical: pending.
Conflict: none.

**SC-511b — readers prefer slot+session over display name.** Bucket 2 — pairing and
delivery use the churn-proof routing key where present; unknown keys are ignored.
Authority: events.md:84. Empirical: pending. Conflict: none.

**SC-511c — schema evolution is additive-only.** Bucket 2 — adding optional keys is
fine; renaming/removing is BREAKING and requires a migration story (#21 lands at the
first such change). Authority: events.md:142-144. Empirical: pending. Conflict: none.

**SC-507a — `archive preview` stdout is exactly the digest.** Bucket 2 — redirectable
digest bytes, nothing else. Authority: commands.md:553-554 (normative — the gate
corrected an earlier code-observation flag; quick-start.md corroborates).
Empirical: M2 census note. Conflict: none.

**SC-507c — `archive preview` diagnostics go to stderr.** Bucket 2 — canonical archive
id, source session, file counts and bytes. Authority: commands.md:554-556.
Empirical: pending. Conflict: none.

**SC-507d — `archive preview` is read-only by construction.** Bucket 2 — writes
nothing, emits no event, creates no archive, never enters the lifecycle. Authority:
commands.md:546-548. Empirical: pending. Conflict: none.

**SC-507b — a live preview is never stitched from two moments.** Bucket 2 — the three
moving files are fingerprinted before and after the render with one clean retry; if
still moving, it says so instead. Authority: commands.md:556-560. Empirical: pending.
Conflict: none.

**SC-513a — `next` exits non-zero when nothing needs attention.** Bucket 2 — with a
message; composes in scripts. Authority: commands.md:150-152. Empirical: pending.
Conflict: none.

**SC-513b — `next` exits non-zero on an unknown argument.** Bucket 2. Authority:
commands.md:158-159. Empirical: pending. Conflict: none.

**SC-513c — `next` is read-only by default.** Bucket 2 — no tmux focus change without
`--attach`. Authority: commands.md:150. Empirical: pending. Conflict: none.

**SC-514 — `doctor` exit contract.** Bucket 2 — non-zero if any checklist item FAILed.
Authority: commands.md:168. Empirical: pending. Conflict: none.

**SC-515a — `stop all` folds per-target result records into its exit.** Bucket 2 —
bounded (~30s) wait on the per-session stop-result events. Authority:
commands.md:365-373. Empirical: pending. Conflict: none.

**SC-515b — result-wait timeout is not a failure.** Bucket 2 — reports `results
pending` and keeps the handoff status rather than calling a still-working supervisor a
failure. Authority: commands.md:370-372. Empirical: pending. Conflict: none.

**SC-515c — an unowned ae-tagged session is named, not stopped.** Bucket 2 — visible on
the server but absent from meta: not killed, run becomes a partial failure (non-zero),
message names both ways out. Authority: commands.md:392-395. Empirical: pending.
Conflict: none.

**SC-516 — `end` fails non-zero when the archive cannot be written.** Bucket 1 —
capture-then-delete: publication happens after verified stop and git, before any live
state is removed; a failed archive fails the end with the whole session still on disk.
Authority: commands.md:499-501 + architecture.md publication protocol.
Empirical: pending (census-2 end section). Conflict: none.

**SC-517a — compact's exit status is the launch's.** Bucket 2 — compact execs into the
launch; there is no separate compact exit. Authority: commands.md:687-688.
Empirical: pending. Conflict: none.

**SC-517b — terminal case: attach, exit on detach.** Bucket 2. Authority:
commands.md:688-689. Empirical: pending. Conflict: none.

**SC-517c — non-terminal case: launch failure reports as plain `ae <name>`.** Bucket 2
— with archive and fresh session already in place, `Recovery:` naming the route.
Authority: commands.md:689-691. Empirical: pending. Conflict: none.

**SC-508 — residual undocumented exit codes.** `authority=code-observation` — only the
cases NOT covered by SC-513..517 remain; probes then seat closure (preserve/fix/
diverge). The Rust binary's 0/2 convention seam is a P1 decision row.
Empirical: pending probe. Conflict: pending normative closure.

### S7 — tmux effects

Options/format-string literalization (`_ae_tmux_format_literal`, `@ae_*` user options),
layout, pane/window naming, status surfaces, ae-monitor window.

<!-- rows: SC-6xx -->

**SC-600 — user text reaching a tmux format string is literalized or option-routed.**
Bucket 1 — `#` and `%` are interpreted (`#()` RUNS SHELL); text is escaped or carried
via `@ae_*` user options, which interpolate literally. Authority: AGENTS.md
interpreted-sinks table (ruling). Empirical: pending. Conflict: none.

**SC-601 — send-keys never receives user text as key names.** Bucket 1 — literal mode
or paste-buffer only; the generated helpers are the boundary, raw send-keys is
forbidden. Authority: AGENTS.md interpreted-sinks (ruling). Empirical: pending.
Conflict: none.

**SC-602 — `@ae_slot` carries identity; `@ae_agent` is display.** Bucket 2 — the slot
option is the stable routing stamp (SC-209); pre-slot sessions are back-filled on
refresh/resume. Authority: helpers.md slot identity. Empirical: pending.
Conflict: none.

(Monitor-window and status-bar rows live in S10: SC-922/923/924 and M-03.)

### S8 — Tool adapters (five tools)

Launch/resume/capture/readiness per tool: claude, codex, gemini, grok, opencode —
capability matrix at AGENTS.md@72c7293 (**empirical**). Readiness gating
(`_spawn_input_ready`, `_tool_initializing`, negative markers), `pane_current_command`
quirks (`opencode.exe`, `env` prefix), re-run semantics of `launch.<slot>.sh`.

<!-- rows: SC-7xx -->

The per-tool capability matrix is NOT expanded into per-cell rows: it is classified
WHOLESALE as empirical pins — the fixture source for the adapter test tables (SC-704).
The rows below are the normative rules that govern every adapter.

**SC-700 — delivery gates on readiness, initialization checked first.** Bucket 1 — both
launch-delivery moments (spawn task, launch/resume prompt) gate on input-readiness, and
a tool that is provably still initializing is not ready however idle its input box
looks. Authority: AGENTS.md readiness ruling ("an idle input box is not an initialized
application"). Empirical: measured codex timings @72c7293. Conflict: none.

**SC-701 — readiness markers are negative evidence only.** Bucket 1 — a marker's
absence is never proof of readiness; a predicate that demands a positive banner breaks
the day a tool stops printing one. Authority: AGENTS.md readiness ruling. Empirical:
codex `model: loading` / MCP progress measurements. Conflict: none.

**SC-702 — a readiness timeout fails loud and durable.** Bucket 1 — the pane text is
preserved next to the session and an event is emitted, because launch delivery runs
detached where stderr reaches nobody. Authority: AGENTS.md readiness ruling.
Empirical: pending. Conflict: none.

**SC-703 — an unmodelled TUI is an accepted risk, never a pretend gate.** Bucket 2 —
a tool without readiness/busy modelling (grok at 72c7293) delivers ungated, and that
status is DOCUMENTED, not silently faked. Authority: AGENTS.md grok row (ruling
framing). Empirical: matrix. Conflict: none.

**SC-704 — adapter expectations are seat-ruled, never measurement-promoted.** Bucket 1
— the capability matrix stays IS; measurements never become expected outputs without an
explicit seat ruling (the anti-oracle rule). The generic product outcomes are frozen
NOW as SC-704a-e (gate ruling — classification is not deferred to P1); exact flags,
markers, and per-tool capabilities remain empirical/adaptable. Upstream drift is
detected (canary class, #22), never silently absorbed. Authority: #81 source-lane rule
+ epic #79. Empirical: the matrix. Conflict: none.

**SC-704a — injected ae context never replaces a vendor's own agent prompt.** Bucket 1
— context rides an append/positional/config surface per tool; a replace-style flag is
forbidden (the grok `--system-prompt-override` trap). Empirical: matrix rows.
Conflict: none.

**SC-704b — capture binds only a positively-owned signal.** Bucket 3 — SHOULD: an
identity is captured only from a signal this agent slot positively owns, never ambient
context (the confidentiality rule). IS at 72c7293: opencode capture is cwd/time
heuristic — two agents in one dir are indistinguishable, max-updated wins.
Conflict: fix-known-defect(#56, intended per DR-005). Empirical: matrix + capture
exhibits.

**SC-704c — resume requires exact ownership.** Bucket 4 — **DR-005**: a resume targets
only an exactly-owned identity; with none stored, the command REFUSES with recovery
guidance — it never guesses and never silently starts fresh over the only stored
provider UUID (#50). Empirical: matrix resume rows. Conflict: DR-005.

**SC-704d — heuristic fallbacks retire.** Bucket 4 — **DR-005**: `--continue` (CWD
guess) and `--resume latest` (recency guess) are cross-wire risks and do not survive;
fresh launch remains an explicit, distinct operation that never claims to be a resume.
Empirical: matrix fallback rows. Conflict: DR-005.

**SC-704e — rerun truth is explicit.** Bucket 1 — re-running a launch script either
resumes the same conversation or honestly starts fresh; never a collision error,
never a silent identity swap. Empirical: matrix rerun row + SC-811 pins.
Conflict: none.

**SC-705 — tool detection identifies the actual executable without interpreting
injected prose.** Bucket 1 — semantic SHOULD under joint ruling: classification derives
from the real binary being launched, never from the kilobytes of injected context,
wrapper prefixes, or launcher spellings. The concrete prefix/suffix exhibits
(`env`/`VAR=val` stripping, `opencode.exe`) are empirical. Authority: S8 joint seat
ruling (2026-08-20) grounded in the #46/#30 transported-fact rulings. Empirical:
measured exhibits @72c7293. Conflict: none.

**SC-706 — a fact built upstream is transported, never re-parsed.** Bucket 1 — resume
ids, injection boundaries, and tool kinds ride explicit parameters; the built command
is downstream data — hostile input, not a source of truth. Authority: #30-family ruling
(commit 32719f5) + AGENTS.md. Empirical: shipped exhibits. Conflict: none.

### S9 — Lifecycle transactions

git commit/push on end, archive capture ordering (archive-before-removal, failed archive
fails the end), `--purge-history`, transfer (both directions), rename, compact/handover,
`--from <uuid>` lineage, recover-pending.

<!-- rows: SC-8xx -->

SHOULD frozen from architecture.md + commands.md + AGENTS.md ruling bullets; one
testable SHOULD per row.

**classified_by: all S9 normative rows (SC-800..831 including letter-splits) —
fable5:lead + gpt56sol:colead, 2026-08-20 (gate 7398f6de + mechanical batch applied).
UNCLASSIFIED pending seat closure: SC-832b/c, SC-833b/c/d, SC-834b/c.**

**SC-800 — archive publication claims by `mkdir`.** Bucket 1 — the atomic claim is
`mkdir .publishing.<uuid>`; mkdir failing IS the mutual exclusion (no flock required —
flock is optional on the platform). Authority: architecture.md:85-88.
Empirical: census-2 end section (audited). Conflict: none.

**SC-801 — staging is private by construction.** Bucket 1 — payload populated under
umask 077 with every mode set explicitly. Authority: architecture.md:89.
Empirical: pending. Conflict: none.

**SC-802 — the final archive appears complete or not at all.** Bucket 1 — validate the
staged tree, re-check the target absent, then one same-filesystem `rename`.
Authority: architecture.md:90-93. Empirical: census-2 (audited). Conflict: none.

**SC-803 — a standing claim is refused and named, never cleaned.** Bucket 1 — from the
outside a stale claim and a live publisher are indistinguishable; the next run refuses
with the claim's name. Authority: architecture.md:95-97. Empirical: pending.
Conflict: none.

**SC-804a — validator: exact path whitelist.** Bucket 1 — an entry ae does not
recognise FAILS validation rather than being ignored. Authority: architecture.md:99-104.
Empirical: pending. Conflict: none.

**SC-804b — validator: no symlink or special file.** Bucket 1. Authority:
architecture.md:100. Empirical: pending. Conflict: none.

**SC-804c — validator: directories 0700.** Bucket 1. Authority: architecture.md:100-101.
Empirical: pending. Conflict: none.

**SC-804f — validator: files 0600.** Bucket 1. Authority: architecture.md:100-101.
Empirical: pending. Conflict: none.

**SC-804d — validator: no executable bit for user, group, OR other.** Bucket 1 — `-x`
answers only for the current user; a group-executable file is still a program.
Authority: architecture.md:101-103. Empirical: pending. Conflict: none.

**SC-804e — validator: `meta` and `digest.md` must agree.** Bucket 1 — on the archive
id and the counts they report. Authority: architecture.md:103-104. Empirical: pending.
Conflict: none.

**SC-805 — an archive is inert data.** Bucket 1 — never an executable file; the
validator is the proof, not intent. Authority: AGENTS.md rules bullet + architecture.md.
Empirical: pending. Conflict: none.

**SC-806a — archive identity is the session UUID, never the mutable name.** Bucket 1 —
addressable independently of a name that is neither unique over time nor stable.
Authority: architecture.md:81-83. Empirical: pending. Conflict: none.

**SC-806b — canonical lowercase key; legacy uppercase normalized.** Bucket 2.
Authority: architecture.md:81-82. Empirical: pending. Conflict: none.

**SC-807 — the lifecycle lock is released before the relaunch.** Bucket 1 — the child
takes the same lock under the same name; holding across both would deadlock ae against
itself. Authority: architecture.md:230-232. Empirical: pending. Conflict: none.

**SC-808 — the child re-proves the exact parent archive before publishing its state.**
Bucket 1 — semantic invariant: re-prove immediately before publication, roll the launch
back on mismatch rather than creating a child with no lineage. (The bash transport
variable is mechanism — ownership/evidence, not SHOULD.) Authority:
architecture.md:232-234. Empirical: pending. Conflict: none.

**SC-809 — lineage is never inferred from a name.** Bucket 1 — a session continues an
archive only via explicit `--from <uuid>`. Authority: AGENTS.md "How it works" (ruling
text) + architecture.md. Empirical: pending. Conflict: none.

**SC-810a — `--purge-history` writes no archive.** Bucket 2. Authority: AGENTS.md "How
it works" + architecture.md:131-133. Empirical: pending. Conflict: none.

**SC-810b — `--purge-history` deletes any existing archive for the source UUID.**
Bucket 2 — a purge that left memo and request payloads would only have looked like
privacy. Delete PROOFS are SC-818a-e (bucket 1). Authority: architecture.md:131-133.
Empirical: pending. Conflict: none.

**SC-811a — `launch.<slot>.sh` re-run: first run creates, later runs resume.** Bucket 2
— the `.started` marker decides. Authority: AGENTS.md launch-rerun bullet (ruling).
Empirical: pending. Conflict: none.

**SC-811b — ae clears the marker whenever it rewrites the script.** Bucket 2 — a fresh
launch always creates. Authority: AGENTS.md launch-rerun bullet. Empirical: pending.
Conflict: none.

**SC-812 — the resume decision happens BEFORE exec.** Bucket 1 — a `cmd || fallback`
chain leaves bash as the pane process and `pane_current_command` reports `bash`,
silently disabling the send path's TUI modelling. Authority: AGENTS.md launch-rerun
bullet + #30-family ruling (commit 32719f5). Empirical: pending. Conflict: none.

**SC-813 — session names are allowlisted at every creation/import boundary.** Bucket 1
— `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$` at launch entry, `default_session_name`,
`transfer` both directions, `rename` target; consumers of existing sessions may accept
legacy direct-child names as a migration path, never traversal. Authority: AGENTS.md
session-name bullet (ruling). Empirical: unit pins @72c7293 (verification).
Conflict: none.

**SC-814 — transfer validates both endpoint names before any side effect.** Bucket 1 —
before any path construction, SSH probe, mkdir, or rsync. Authority: AGENTS.md
session-name bullet. Empirical: pending. Conflict: none.

**SC-815a — the confirmed fleet is the fleet acted on.** Bucket 1 — `stop all` hands
over the exact confirmed list and does not re-enumerate; a session started during
confirmation is left alone. Authority: commands.md:382-386. Empirical: pending.
Conflict: none.

**SC-815b — fleet entries carry session identity, not names.** Bucket 1 — ending a
session and starting a new one under the same name mid-operation leaves the newcomer
running, with a recorded failure explaining the name changed hands. Authority:
commands.md:386-389. Empirical: pending. Conflict: none.

**SC-815c — each fleet run has a unique operation identity and consumes ONLY its own
results.** Bucket 1 — cross-run result folding is never permitted; the label alone is
not the mechanism. Authority: commands.md:389-390. Empirical: pending. Conflict: none.

**SC-815d — the visible representation is `[op <uuid>]` in the events.** Bucket 2.
Authority: commands.md:389-390. Empirical: pending. Conflict: none.

**SC-816 — an unverifiable session is still a target.** Bucket 1 — if its recorded tmux
server is unreachable, ae does not know it is stopped: it is carried into the fleet and
fails loudly in its own log rather than being silently counted as gone. Authority:
commands.md:378-381. Empirical: pending. Conflict: none.

**SC-817 — end's transaction order: stop, git-outcome-fixed, capture, cleanup.** Bucket
1 (rewritten per gate 7398f6de) — verified stop precedes git; the ACTUAL commit/push
outcome is fixed before capture and RECORDED in the archive (`push_outcome` — managed
mode with no origin still archives, records `no-origin`, and preserves the work
directory; a FAILED push returns before capture); cleanup follows only a successful
archive. No promise that the final commit exists remotely except in the
pushed/already-reachable outcomes. Authority: commands.md end section +
architecture.md. Empirical: census-2 end section (audited). Conflict: none.

**SC-818a — purge requires ae's REAL archive root, never a symlink.** Bucket 1.
Authority: commands.md:534-535 + architecture.md:134-137. Empirical: pending.
Conflict: none.

**SC-818b — purge acquires the same `.publishing.<uuid>` claim.** Bucket 1 — a delete
cannot race a publisher's rename. Authority: commands.md:534-536. Empirical: pending.
Conflict: none.

**SC-818c — purge validates the tree as an ae archive before deleting.** Bucket 1 — a
tree ae cannot validate is a tree ae cannot claim to own; a hand-edited archive is
refused (remove it yourself). Authority: commands.md:536-542. Empirical: pending.
Conflict: none.

**SC-818d — purge requires a NONEMPTY exact source-identity match.** Bucket 1 — an
archive naming no session is absence of proof, not a wildcard; refused as malformed
(and `--from` will not inherit from it either). Authority: commands.md:537-540 +
architecture.md:134-137. Empirical: pending. Conflict: none.

**SC-818e — purge refuses to delete a parent a live `--from` lineage points at.**
Bucket 1. Authority: architecture.md:137-138. Empirical: pending. Conflict: none.

**SC-819 — an unidentifiable session is refused BEFORE anything is stopped.** Bucket 1
— meta gone with memory remaining, or `session_id` unparseable: refused with the
reason, nothing deleted, regardless of history flag ("delete it" is not an answer to
"which session is this"). Authority: commands.md:513-518 + architecture.md:139-143.
Empirical: pending. Conflict: none.

**SC-820a — end freezes the confirmed plan and re-proves it under the lock.** Bucket 1
— each target resolved exactly ONCE; the prompt renders from those fields and the
freeze captures the same observation (a fork cannot carry a freeze back); re-proof
under the lifecycle lock refuses on mismatch and prints both versions. Authority:
commands.md:526-532 + architecture.md:146-149,158-166. Empirical: pending.
Conflict: none.

**SC-820b — `-f` freezes nothing.** Bucket 2 — nothing was promised, so nothing is
frozen or re-proved. Authority: commands.md:526-532. Empirical: pending. Conflict: none.

**SC-821a — `end all` acts on the confirmed target set only.** Bucket 1 — the set can
never grow between question and answer. Authority: architecture.md:150-155.
Empirical: pending. Conflict: none.

**SC-821b — "a prompt ran" is its own fact, never a count.** Bucket 1 — an empty
confirmed list means end NOTHING, which a count cannot distinguish from
nobody-was-asked. Authority: architecture.md:150-155. Empirical: pending.
Conflict: none.

**SC-822 — `--from` is valid only for a session that does not exist in any form.**
Bucket 1 — no tmux session, no session state, no worktree; onto an existing session it
refuses ("resume this AND inherit that" has two meanings and no safe default).
Authority: commands.md:580-584. Empirical: pending. Conflict: none.

**SC-823 — the parent is proved before anything is created.** Bucket 1 — a refusal
leaves no tmux session, no session state, no worktree. Authority: commands.md:586-592.
Empirical: pending. Conflict: none.

**SC-824a — proof facts are recorded as proved, never re-read.** Bucket 1 — id and
handover/pending counts come back from the one proof, not from a file another process
may be deleting. Authority: commands.md:589-592. Empirical: pending. Conflict: none.

**SC-824b — an archive mid-publication or mid-purge is refused outright.** Bucket 1.
Authority: commands.md:589-592. Empirical: pending. Conflict: none.

**SC-825a — the child records lineage durably.** Bucket 2 — `parent_archive_id` +
parent handover/pending counts, preserved across resumes. Authority:
commands.md:594-598. Empirical: pending. Conflict: none.

**SC-825b — the parent path is derived, never stored.** Bucket 2 — from archive root +
id, so moving `AE_HOME` cannot rot it. Authority: commands.md:594-598.
Empirical: pending. Conflict: none.

**SC-825c — a deleted parent warns and continues on resume.** Bucket 2 — the lineage
fact is still true; workspace.md says the digest is gone. Authority:
commands.md:594-598. Empirical: pending. Conflict: none.

**SC-826 — a pre-id session gets one minted at end, recorded on both sides.** Bucket 2
— `session_id_origin=minted-at-end` in live meta AND `archive_id_origin=minted-at-end`
in the archive; the live record keeps a retry after failed publication honest.
Authority: commands.md:520-524. Empirical: pending. Conflict: none.

**SC-827 — compact freezes ONE authorization tuple.** Bucket 1 — the session resolves
once into the eight-field tuple; everything downstream reads the tuple, never meta
again; end takes compact's frozen plan as its confirmed authority instead of resolving
a second one. Authority: architecture.md:177-184 + commands.md:626-630. Empirical:
census-2 compact section (audited). Conflict: none.

**SC-828 — two revalidations, positioned by what they protect.** Bucket 1 — first
immediately after the human's answer (a replaced session is never MESSAGED); second
under the lifecycle lock immediately before teardown (a replacement is never STOPPED);
a mismatch names the field that moved. Authority: architecture.md:186-191 +
commands.md:650-655. Empirical: pending. Conflict: none.

**SC-829a — handover completion is two facts.** Bucket 1 — a reply to the request AND a
new `handover`-topic memo written after the request went out, polled from the event log
and `memo.tsv`, never pane output. Authority: architecture.md:193-199. Empirical:
pending. Conflict: none.

**SC-829b — a re-run reuses the outstanding request and its baseline.** Bucket 1 — the
memo baseline travels in the request's own stored body, so re-running waits on the SAME
request instead of sending a second, and the fact survives into the archive.
Authority: architecture.md:198-201. Empirical: pending. Conflict: none.

**SC-830 — `--digest-only` is the one explicit degradation.** Bucket 2 — withdraws
anything outstanding and treats the digest as the handover. Authority:
commands.md:634-638 + architecture.md:201-203. Empirical: pending. Conflict: none.

**SC-831 — a timed-out handover stops nothing.** Bucket 1 — nothing stopped, nothing
archived, the request stays open so a re-run waits on the SAME request rather than
sending a second. Authority: commands.md:656-658. Empirical: pending. Conflict: none.

**SC-832a — rename's effect set.** Bucket 2 — renames the tmux session, moves the
session directory, updates `session=` in meta, regenerates `workspace.md`; a running
tmux server stays up. Authority: commands.md:287-290 (normative — the gate corrected an
earlier no-source claim). Empirical: census-2 rename section. Conflict: none.

**SC-832b — rename vs concurrent meta writers.** `authority=code-observation` — the
meta rewrite runs without `meta.lock` (census-2, ae:11597-11667); race semantics need
seat closure. UNCLASSIFIED pending closure.

**SC-832c — rename crash cuts.** `authority=code-observation` — residue at each cut
point (dir moved / tmux renamed / meta updated) per census-2; seat closure pending.
UNCLASSIFIED pending closure.

**SC-833a — transfer moves a stopped session both directions.** Bucket 2 — including
Claude/Codex conversation files; `--pull` is the reverse direction. Authority:
commands.md:24. Empirical: census-2 transfer section. Conflict: none.

**SC-833b — transfer requires the stopped state first.** `authority=code-observation` —
stop-before-rsync ordering; seat closure pending. UNCLASSIFIED.

**SC-833c — per-direction partial-rsync residue.** `authority=code-observation` —
census-2 evidence; seat closure pending. UNCLASSIFIED.

**SC-833d — transfer's audit event is best-effort.** `authority=code-observation` —
warned, success still reported (census-2 addenda, ae:11530-11535); seat closure
pending. UNCLASSIFIED.

**SC-834a — `_recover-pending` re-attempts post-launch session-id capture.** Bucket 2 —
internal helper, called by the watchdog. Authority: commands.md:713-715 (normative
purpose/caller — gate correction). Empirical: census evidence. Conflict: none.

**SC-834b — recovery meta reconciliation.** `authority=code-observation` — fd200
check-then-set rewriting `agent.<slot>` (census, ae:8717-8732); seat closure pending.
UNCLASSIFIED.

**SC-834c — the `recover` event follows success separately.** `authority=code-observation`
— separate append after the meta write (census); seat closure pending. UNCLASSIFIED.

### S10 — Daemons/sidecars + contrib boundary

Watchdog (nudge rules, quiet-state honoring, footprint exclusion; bash impl vs
`AE_WATCHDOG_IMPL=uv` aewatch), telegram bridge (chat events, reply routing, markdown/jq
injection boundaries; bash daemon vs aewatch runtime handoff via marker + fresh
heartbeat), `ae steward` + ae-monitor window (bash product surfaces) vs contrib
templates/sidecars.

<!-- rows: SC-9xx — claims collected by gpt56luna:s10source (colead's evidence worker,
2026-08-20, frozen-doc citations); buckets proposed by lead; colead confirm pending -->

**SC-900 — event-log container lifecycle.** Bucket 4 — **DR-001**: the container is NOT
append-only forever; explicit generations with rotation under DR-001's binding
conditions replace both the frozen resume-trim behavior and the docs' no-rotation
promise. Authority: DR-001 (both seats). Empirical: ae:18046-18075 (trim) + census-3
audit I1 (reader race). Conflict: DR-001.

**SC-901 — daemon topology.** Bucket 4 — **DR-002**: one Rust daemon per `AE_HOME`
owns watchdog + telegram at P4; the `AE_WATCHDOG_IMPL` selector and the per-session
`_watchdog` process/pane retire. Per-session enable/persistence/start-stop-status
semantics survive; `ae-monitor`/`_events` stay inspectable; daemon decisions are
durable events/log, never pane-peeking. Authority: DR-002 (both seats).
Conflict: DR-002.

Rows SC-902+ per the ratified schema (gate: the earlier flat table violated it).
Sources: colead's fine-grain proposals (memo topics s10-watchdog/-steward/-telegram/
-defects, frozen citations therein) + lead bridge/cross-cutting splits. Numeric
tunables/defaults/malformed values live in S15; helper-publication atomicity in S11;
core-dependency floor deduped to SC-940 + S12. **classified_by: pending colead's
one-pass confirm (held at ratification per gate).**

Watchdog + monitor (authority: watchdog.md / monitor.md @72c7293, cited per memo):
- **SC-902** b2 — watchdog enabled by default; only explicit false/no/off/0 disables.
- **SC-903** b2 — per-session enable state persists across resume.
- **SC-904** b2 — start is idempotent, confirms enabled state; start/stop/status + loop
  surface survive DR-002.
- **SC-905** b1 — per-pane cycle is first-match-wins; no later branch after a
  classification.
- **SC-906** b1 — dead = shell foreground with no agent descendant; alert once, then
  ignore until state changes.
- **SC-907** b1 — a quiet declaration applies only while it is the LATEST relevant
  event; any newer relevant event invalidates it.
- **SC-908** b1 — `done` is event-only: pane churn never revives it; resumption needs a
  newer ae event.
- **SC-909** b1 — waiting-user/blocked arm a post-echo pane baseline, hold while
  unchanged, yield on later pane change.
- **SC-910** b1 — active pane change suppresses the nudge and resets the count.
- **SC-911** b1 — recently-visible pane change suppresses within the stale window.
- **SC-912** b1 — recent ae activity suppresses within the stale window.
- **SC-913** b3 fix-known-defect(#45) — every daemon nudge uses the ONE verified
  delivery primitive (target lock, busy/human/dead guards, durable failure evidence,
  verified submit); only rc0 delivery spends MAX_NUDGES. IS: aewatch pastes ungated
  (census-3 I7).
- **SC-914** b1 — after MAX_NUDGES confirmed deliveries: one alert + visible banner,
  then silent waiting until state changes.
- **SC-915** b1 — first throttle cycle suppresses the nudge and resets the stale budget.
- **SC-916** b1 — first cycle of a throttle streak emits exactly one `throttled` event.
- **SC-917** b1 — continuous throttle alerts once at the threshold, never re-alerts in
  the same streak.
- **SC-918** b1 — throttle disappearance emits `throttle-cleared` and resets the streak.
- **SC-919** b1 — a registered missing pane alerts once per disappearance.
- **SC-920** b3 fix-known-defect(#51) — human-origin evidence inside quiet stabilization
  must yield; a submitted human reply is never absorbed as agent churn. IS: equal pane
  hashes cannot distinguish them.
- **SC-921** b3 fix-known-defect(#73) — monitor panes are never agents and never enter
  the roster. IS: regenerate_manifest lists `_watchdog`/`_events`.
- **SC-922** b2 — every session keeps `ae-monitor` with an always-present `_events`
  view, independent of watchdog enablement, across resume.
- **SC-923** b1 — monitor panes are read-only/input-disabled.
- **SC-924** b2 — watchdog stop never removes the `_events` inspection surface (DR-002
  retires only the `_watchdog` pane).
- **SC-925** b1 — a dead agent is never auto-restarted by the watchdog.
- **SC-926** b3 fix-known-defect(#88-A) — control success only when durable intent and
  runtime converge; typed partial failure otherwise. IS: meta-write failure ignored
  after tmux mutation.
- **SC-927** b3 fix-known-defect(#88-B) — status is read-only; cleanup belongs to an
  explicit reconcile path. IS: status deletes stale pidfiles.
- **SC-928** b3 fix-known-defect(#88-C) — an event-append error is surfaced and
  contained to its operation; it never stops the combined daemon or spends nudge state.
  IS: census-3 I2 crash/backoff.
- **SC-929** b4 DR-002 — the restart outcome (gate ruling, testable): after a
  successful `doctor --refresh` the serving daemon runs the INSTALLED version before
  the command returns; a failed refresh returns nonzero and leaves the previous daemon
  serving; the restart emits durable before/after/failure facts. Implementation may
  re-exec. (The bash keeps-loaded-body behavior retires with the topology.)

Steward (authority: commands.md/telegram.md @72c7293, cited per memo):
- **SC-930** b2 — bare `ae steward` ensures the detached steward, never attaches.
- **SC-931** b2 — `--attach` is the explicit attach/switch surface.
- **SC-932** b1 — `--init` scaffolds and NEVER overwrites operator files.
- **SC-933** b1 — steward launch isolates its config and neutralizes project-local
  config.
- **SC-934** b1 — steward authority is monitor/relay/suggest ONLY: never ends, stops,
  edits, or dispatches into another session without human authorization.
- **SC-935** b1 — only the steward main agent gets sweep cadence, no stale escalation;
  its workers keep normal watchdog behavior.
- **SC-936** b1 — a sweep nudge is delivery-checked; refusal is logged, never counted
  delivered.
- **SC-937** b1 — undelivered sweeps retry on the short cadence.
- **SC-938** b1 — after retry max: normal cadence + one unreachable alert, cleared on a
  landed delivery.
- **SC-939a** b1 — sweep delivery is at-least-once: event-write failure after paste may
  duplicate, never silently drop.
- **SC-939b** b1 — steward liveness = dead-pane checks AND a live-but-not-sweeping
  heartbeat (~2x cadence); stale alerts once, recovery clears.
- **SC-939c** b2 — sweep nudges are outside the default Telegram include.
- **SC-939d** b2 — plain Telegram text defaults to the running steward absent a sticky
  override; no steward yields start guidance.
- **SC-939e** b2 — `/use` overrides that default; `/use clear` restores steward routing.
- **SC-939f** b2 — deprecated `hub` stays accepted; canonical name is steward (#52
  policy ruling).

Telegram (authority: telegram.md @72c7293, cited per memo):
- **SC-940** b1 — jq/curl absence refuses ONLY the bridge; core commands unimpaired.
- **SC-941** b2 — outbound include allow-list default; exclude applies after include.
- **SC-942** b2 — `chat` action gives the two-way loop; include-without-chat disables
  it and status warns.
- **SC-943** b1 — inbound exists only with nonempty `allowed_user_ids`; empty =
  outbound-only.
- **SC-944a/b/c** b1 — three separate trust predicates: numeric allowlisted `from.id`;
  exact configured `chat.id`; private chat — ANY failure silently drops.
- **SC-945** b2 — routing precedence: command > reply > compact > override/steward.
- **SC-946** b1 — every inbound route passes the same session/agent revalidation.
- **SC-947** b1 — only running sessions are addressable.
- **SC-948** b2 — session resolves by exact name or unique session_id prefix.
- **SC-949** b1 — agents resolve only within that session; pane-id, cross-session, and
  external-actor escapes rejected.
- **SC-950** b2 — sender identity is `telegram:<id>`; replies route back outbound.
- **SC-951** b1 — inbound update offset persists BEFORE dispatch: at-most-once side
  effects.
- **SC-952** b2 — command-menu registration is best-effort (log and ignore).
- **SC-953/954** b2 — start is idempotent; stop-when-stopped succeeds.
- **SC-955** b2 — status reports persisted intent, runtime, deps, token validity.
- **SC-956** b1 — autostart failure warns one line and never blocks session launch.
- **SC-957** b1 — supervision honors durable disabled state; can never revive after an
  explicit stop (DR-002 changes topology, not this).
- **SC-958** b4 DR-003 — outbound delivery is at-least-once: cursor persistence is part
  of committed progress, save failure is LOUD and retry-safe, duplicates possible only
  in the crash window, event id rides the outbound text/log for dedupe. (IS: save
  failure silently ignored — #86-D evidence.)
- **SC-959** b2 — a first-seen session starts at EOF; no history flood.
- **SC-960** b1 — the persisted getUpdates offset prevents inbound redispatch on
  restart.
- **SC-961** b1 — token file is owner-only 0600; wrong perms refuse start with a
  corrective diagnostic.
- **SC-962** b1 — the token never enters argv; logs redact it.
- **SC-963** b3 fix-known-defect(#83) — explicit start preserves exactly-one-sender:
  refuse or complete a verified takeover, never warn-and-double-send.
- **SC-964** b3 fix-known-defect(#84) — takeover is serialized and proves every
  predecessor absent across the COMPLETE scope before the first send.
- **SC-965** b3 fix-known-defect(#85) — destructive tmux targets resolve exact
  identity, never prefix.
- **SC-966** b3 fix-known-defect(#86-E) — `/use clear` succeeds only after durable
  removal.
- **SC-967** b3 fix-known-defect(#87) — one effective-config authority for every
  daemon mode, `CONFIG_FILE`/`AE_LOCAL_CONFIG` included.
- **SC-968** b3 fix-known-defect(#88-G) — lifecycle ownership acquired before any
  probe/kill/create mutation.
- **SC-969** b3 fix-known-defect(#87-H) — setup publishes token/config with atomic
  visibility; no reader observes empty/partial canonical state.
- **SC-970** b2 — setup persists enabled, token_file, chat_id, seeded allowlist (byte
  formats: S5/S15).
- **SC-971** b2 — start persists `enabled=true`; stop persists `enabled=false`.

Bridge protocol (authority: bridge-protocol.md @72c7293; lead splits per gate):
- **SC-972** b2 — external actors are `<platform>:<id>`, opaque past the allowlisted
  prefix.
- **SC-973a** b1 — event-only sinks (`telegram:`/`discord:`/`ae:compact:`) emit without
  tmux resolution and preserve the literal target.
- **SC-973b** b1 — an unknown non-allowlisted external target fails LOUDLY.
- **SC-974a** b2 — `AE_SENDER_OVERRIDE` sets the actor for send/ask/review.
- **SC-974b** b2 — reply caller identity comes from `--as`, not the override.
- **SC-975a** b1 — bridge readers tolerate a missing event file.
- **SC-975b** b1 — malformed/unterminated trailing data is buffered until a complete
  newline record exists.
- **SC-976a** b4 DR-001 — the reader cursor is generation-aware (generation + offset
  replaces the (session_id,inode) key).
- **SC-976b** b2 — event logs are tailed/back-scanned bounded, never whole-loaded.
- **SC-977** b1 — bridges bind the stable session_id across resume/rename/transfer.
- **SC-978a** b2 — bridges ignore unknown fields/actions.
- **SC-978b** b2 — renames/removals/semantic changes of existing fields are BREAKING.
- **SC-979a** b1 — telegram sends use plain-text paths (no parse-mode injection).
- **SC-979b** b1 — jq programs stay fixed strings; user data enters via stdin only.

(A-02 is the historical revisit-trigger doctrine — executed by #79, not a contract row.
A-05 = SC-1101, not duplicated. Batch conflicts 1–3 are owned by SC-900/#84-85/#45
rows; conflict 6 is the I8 precision note; conflict 7 — autostart race outside the
singleton — is absorbed by DR-002 condition 1 (race-free self-healing), no separate
issue; conflict 8 rides M1's typed-Result contract with per-operation policy rows.)

**S10 gaps (recorded, not closed):** malformed/non-numeric handling for `AE_WATCHDOG_*`,
`AE_TELEGRAM_*`, include/exclude and `allowed_user_ids` is UNDOCUMENTED — probe + seat
decisions before those rows can complete (S15 carries them per-variable); outbound
send-failure and offset-save-failure semantics undocumented (#86 fixes forward);
crash/power durability of markers is explicitly NOT promised (I8).

### S11 — Installer / doctor / refresh

Install contract (symlink or curl|bash), `doctor --refresh` regeneration boundary
(running watchdog keeps loaded body), `_publish_executable_artifact` chokepoint.

<!-- rows: SC-10xx -->

**SC-1000 — install is clone + symlink, both entry paths handled.** Bucket 2 — the
one-liner clones to `~/.local/share/ae` and symlinks into `~/.local/bin`; running
`install` from a clone just symlinks it. Authority: install.md. Empirical: pending.
Conflict: none. (#57's installed-symlink-tracks-dev-checkout finding is its own B3
row at the P5 flip.)

**SC-1001 — upgrade is git pull; helpers self-heal.** Bucket 2 — existing sessions
auto-regenerate helpers on next start/resume; `doctor --refresh [name]` forces it
without reattaching. Authority: install.md upgrading. Empirical: pending.
Conflict: none.

**SC-1002 — doctor walks a fixed OK/WARN/FAIL checklist.** Bucket 2 — bash version,
tmux/git, config, registered agent executables, sessions dir; exit contract is SC-514.
Authority: install.md verify + commands.md:168. Empirical: pending. Conflict: none.

**SC-1003 — published executables cross one atomic chokepoint.** Bucket 1 — every
generated executable artifact outside a session's helper set is generated to temp,
mode-set there, and renamed — with the generator run as a COMMAND, never piped (a
producer dying mid-pipe must not publish a prefix). Authority: AGENTS.md M3 ruling.
Empirical: unit guard @72c7293. Conflict: none.

**SC-1004 — session helpers publish temp+chmod+mv, atomically per artifact.** Bucket 1
— a generator failure can never truncate a live helper. Authority: AGENTS.md declare-f
section (ruling; relocated A-03). Empirical: unit guards @72c7293. Conflict: none.

### S12 — Platform/dependency degradation

GNU/BSD shims, optional `flock`/`timeout` degradation, `sun_path` limits, uuid case,
bash >= 4.0 floor for the remaining glue.

<!-- rows: SC-11xx -->

**SC-1100 — the enumerated divergence class behaves identically on GNU and BSD.**
Bucket 1 — narrowed per gate: for the operations in the divergence table (reverse-cat,
stat fields, date parsing, in-place sed, JSON extraction, wc padding, uuid case, sed
alternation), observable command/format semantics are platform-identical — via shims in
bash, by construction in Rust. NOT a promise that every product behavior is identical.
Authority: AGENTS.md shim rule (use the shim, never the raw tool). Empirical: shipped
macOS bugs @72c7293. Conflict: none.

**SC-1101a — `flock` is an optional dependency.** Bucket 3 — SHOULD: absence degrades
loudly, never hard-fails core commands. IS at 72c7293: `ae_meta_set`/`_lib` paths die
`command not found` (#75); per-path outcomes diverge (census-2 matrix).
Conflict: fix-known-defect(#75, intended: the Rust core needs no external flock —
native locking; surviving glue degrades loudly). Empirical: census-2 missing-flock
matrix.

**SC-1101b — `timeout` is an optional dependency.** Bucket 2 — guard with `command -v`
and degrade. Authority: AGENTS.md. Empirical: pending. Conflict: none.

**SC-1102 — session/archive UUIDs are canonical lowercase.** Bucket 2 — `gen_uuid`
normalizes (macOS `uuidgen` is uppercase); validators and filenames are
lowercase-only. Authority: AGENTS.md bullet (ruling). Empirical: pending.
Conflict: none.

**SC-1103 — a socket path stays within the active platform's limit or fails loud
before creation.** Bucket 1 — seat-ruled semantic SHOULD (2026-08-20): the limit is
respected or the failure is explicit pre-creation; the exact byte limits (104 macOS /
108 Linux) are measured platform facts, empirical only. Empirical: measured mktemp
overflow exhibit @72c7293. Conflict: none.

**SC-1104 — process introspection works without `/proc`.** Bucket 1 — parent walking
and process facts come from portable interfaces (bash: `ps -o ppid=`; Rust: proper
APIs), never `/proc` paths. Authority: AGENTS.md bullet. Empirical: pending.
Conflict: none.

**SC-1105 — the bash floor is 4.0, scoped to the surviving glue.** Bucket 2 — the
requirement applies to what remains bash after each flip; the binary imposes no bash
requirement at all. Authority: AGENTS.md rules + epic end state. Empirical: pending.
Conflict: none.

### S13 — Identity/provenance security surface

System-prompt interpolation (#59): agent/session name allowlists at every creation
boundary, fresh=fatal vs restored=fail-quiet provenance rule, derived names as grammar
fixed points, message-envelope authority (human = no envelope).

<!-- rows: SC-12xx -->

Authority for all rows is the #59 ruling (closing comment + 72c7293 commit message +
AGENTS.md allowlist bullets) unless noted — normative role. Session-name boundary rows
live in S9 (SC-813/814).

**classified_by: SC-1200..1209 including all splits — fable5:lead + gpt56sol:colead,
2026-08-20 (confirmed on 07e2770), including SC-1202 bucket 3/#61 and the SC-1209 joint
ruling.**

**SC-1200 — agent names are allowlisted, not screened.** Bucket 1 —
`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`, because a name reaches a privileged sink: it is
interpolated into that agent's own system prompt (the identity sentence).
Empirical: unit pins @72c7293. Conflict: none.

**SC-1201 — the spawn boundary treats a peer name as hostile.** Bucket 1 — a name
arriving via `spawn` is validated fatally: violation refuses the spawn (the #59 exploit
was a legal-looking name carrying prose into the identity sentence).
Empirical: pending. Conflict: none.

**SC-1202 — the operator roster boundary fails the launch before product mutation.**
Bucket 3 (gate finding: known IS/SHOULD conflict) — SHOULD: a roster violation is
rejected before any tmux, session-state, or config mutation (diagnostics permitted).
IS at 72c7293: M2 writes the default config BEFORE dispatcher/roster validation, so an
invalid fresh roster has a filesystem side effect before refusal.
Conflict: fix-known-defect(#61, intended: read/validation paths never bootstrap).
Empirical: M2 census + ae:344-352.

**SC-1203 — enforcement follows provenance, not the variable.** Bucket 1 — FRESH input
(config, CLI, spawn) is fatal on violation; RESTORED input (saved meta, compact's
frozen roster) is left to the interpolation guard — refusing restored input would make
a pre-grammar session unresumable and kill a compact child whose source is already
archived. Empirical: pending. Conflict: none.

**SC-1204 — the interpolation boundary re-validates and fails quiet.** Bucket 1 —
semantic SHOULD: at the system-prompt interpolation boundary, alias and name are EACH
revalidated under their respective allowlists; an invalid restored identity omits ONLY
the identity sentence and the launch continues. (The bash function/call path is
empirical/ownership material.) Empirical: pending. Conflict: none.

**SC-1205a — every derived name is grammar-valid and unique after derivation.** Bucket
1 — the FINAL value is validated, not the base it was derived from. Authority: #59
durable ruling. Empirical: unit pins @72c7293. Conflict: none.

**SC-1205b — dedup shape: truncate to fit, suffix from `-2`.** Bucket 2 — the suffix
counts occurrences (meaning), not array position; the base is truncated so the suffix
fits the 64 cap. Authority: #59 durable ruling. Empirical: unit pins @72c7293.
Conflict: none.

**SC-1206 — a leading underscore is a legal alias but never an agent name.** Bucket 2 —
`workers = _foo` (alias as its own name) fails the launch with the grammar; internal
`_`-prefixed helpers stay out of the agent namespace. Empirical: pending.
Conflict: none.

**SC-1207a — prompt identity facets are unambiguous.** Bucket 1 — neither the alias nor
the name may contain the facet separator; identity parses one way only. Empirical:
pending. Conflict: none.

**SC-1207b — meta serializes agents as `alias:name:provider-session-id`.** Bucket 2 —
exact on-disk form (cross-link: S5 formats family). Empirical: pending. Conflict: none.

**SC-1208 — pane/peer text is never spliced into instruction material.** Bucket 1 —
transport delivers peer text through the model's USER-INPUT surface; ae never places
pane content or peer-message text into system/developer instruction material; delivered
text retains peer provenance and cannot override identity or human/system authority.
Authority: AGENTS.md interpreted-sinks row (ruling). Empirical: pending. Conflict: none.

**SC-1209 — envelope authority: the outermost helper-emitted line is the only
provenance.** Bucket 1 — the helper-emitted FIRST PHYSICAL line determines peer
provenance; nested/pasted envelopes are data; truly unenveloped interactive input is
the human, who outranks every agent. **Authority: S13 joint seat ruling (fable5:lead +
gpt56sol:colead, 2026-08-20) — recorded here as the normative source, superseding
mutable workspace rules.** Empirical: pending. Conflict: none.

### S14 — Locking / concurrency observable promises

Externally observable ordering/atomicity promises only; protocol detail lives in
`ownership.md`.

<!-- rows: SC-13xx -->

**SC-1300 — event appends are serialized per log.** Bucket 2 — one lock file beside
each `events.jsonl`; concurrent appenders serialize (failure SEMANTICS are
per-operation rows — M1). Authority: events.md + bridge-protocol.md. Empirical:
census-1 M1. Conflict: none.

**SC-1301 — session meta is written through one fail-closed writer.** Bucket 3 —
SHOULD (architecture.md:158-166): one function, every step checked, missing meta
refused, temp removed on error, rename only after complete content. IS at 72c7293:
two additional DIRECT-APPEND writers exist under the same lock (`launch_time.*`
capture ae:2068-2075; `_cmd_spawn` rows ae:11923-11945 — census-3 audit I5), so
unlocked readers can observe partial canonical meta. Conflict: pending seat closure at
the sweep (extend #88 or dedicated issue — the one-writer promise vs three writers).
Empirical: census-3 I5.

**SC-1302 — a session name's lifecycle operations serialize on one lock.** Bucket 1 —
`.lifecycle.<name>.lock` serializes start/resume/end/compact for that name (degrade
rule is SC-1101a's #75 conflict). Authority: architecture.md + census (fd8 launch /
fd9 end, same file). Empirical: census-2. Conflict: none.

### S15 — Environment controls (census: `AE_*` at 72c7293)

`AE_HOME`, `CONFIG_FILE`/`AE_LOCAL_CONFIG`, `AE_TMUX_SERVER`, `AE_WATCHDOG_IMPL`,
`AE_NO_AUTOSTART`, `AE_END_SERVER`, `AE_HUB_DIR`, `AE_STEWARD_DIR`, `AE_EVENTS_KEEP`,
`AE_SEND_DEFER_SEC`, `AE_ATTN_REQUEST_SECS`, `AE_LIST_ACTIVE_SECS`,
`AE_COMPACT_HANDOVER_SECS`, `AE_WATCHDOG_*` tunables, `AE_LOOP_*` tunables,
`AE_TELEGRAM_*` tunables, `AE_SENDER_OVERRIDE`, launch-token/slot vars
(`AE_CODEX_*`, `AE_GEMINI_*`, `AE_OPENCODE_*`), resolution vars exported to helpers
(`AE_RESOLVED_*`, `AE_SESSION`, `AE_META`, `AE_DIR`, `AE_MODE`, `AE_ORIGIN`, `AE_PATH*`).
Each row: default when unset, malformed-value behavior, failure mode.

<!-- rows: SC-14xx — the documented numeric defaults relocated from S10 (W-09..12) -->

**SC-1400** b2 — `AE_WATCHDOG_INTERVAL_SEC` default 60. **SC-1401** b2 —
`AE_WATCHDOG_STALE_MIN` default 15. **SC-1402** b2 — `AE_WATCHDOG_MAX_NUDGES` default
2. **SC-1403** b2 — `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` default 5. **SC-1404** b2 —
`AE_WATCHDOG_TG_SUPERVISE_SEC` default 120, `0` disables. **SC-1405** b2 —
`AE_WATCHDOG_SWEEP_SEC` default 300, `0` falls back to normal watchdog. **SC-1406**
b2 — `AE_WATCHDOG_SWEEP_RETRY_SEC` default 30, clamped to sweep cadence. **SC-1407**
b2 — `AE_WATCHDOG_SWEEP_RETRY_MAX` default 6, then one unreachable alert.
(Authority for 1400-1407: config.md watchdog-defaults table + watchdog.md:35-44.
Empirical: pending. Conflict: none.)

**SC-1408 — legacy `AE_LOOP_*` names are honoured as fallbacks.** Bucket 2 — per
tunable. Authority: config.md. Empirical: pending. Conflict: none.

**SC-1409 — malformed/non-numeric tunable values.** `authority=code-observation` —
UNDOCUMENTED everywhere (S10 gap); probe + seat ruling per variable class at the
sweep. UNCLASSIFIED pending closure.

**SC-1410 — the remaining `AE_*` surface.** `authority=code-observation` — `AE_HOME`,
`CONFIG_FILE`/`AE_LOCAL_CONFIG` (#87 names the intended authority), `AE_TMUX_SERVER`,
`AE_NO_AUTOSTART`, `AE_END_SERVER`, hub/steward dirs, `AE_EVENTS_KEEP`, send/attn
tunables, launch-token and resolution exports: semantics per variable need probe +
seat ruling; rows split per variable AT the sweep from probe results. UNCLASSIFIED
pending closure.

## Known-defect register (bucket 3)

**One row per finding** — an issue with N distinct findings contributes N rows (#66
contributes at least two). Minimum set per #81: #40, #49, #61 (pre-dispatch config
bootstrap — mechanism M2 in ownership.md, inherited by EVERY dispatcher entry, not
list-local), #63, #66 (≥2 rows), #75. Colead-proposed additions (accepted, joint pass
confirms): #45, #57, #71 (concrete observed defects with intended Rust fixes).
Candidates from gate-audit: #69 (if its finding implicates product behavior, not only
the test rig).

## DR register (bucket 4)

A DR is a full decision record, not a table line — mandatory fields (gate finding
fe7cfc2e, important 2):

```
DR-NNN <title>
- affected SC ids:
- context / current IS (with evidence pointers):
- options considered:
- decision / intended Rust behavior:
- trade-offs accepted:
- authority (normative sources consulted):
- seats + date:
```

Ambitious-divergence latitude — **ruled by Clemens 2026-08-20, this paragraph is the
durable record**: where the seats see a genuine overall-architecture improvement, bucket-4
divergences may be ambitious rather than conservative-parity-only. The latitude raises the
bar for DR completeness: the wider the divergence, the fuller the record.

| DR | Title | Status |
|---|---|---|
| DR-001 | Event-log generations | RATIFIED (both seats, 2026-08-20) |
| DR-002 | One daemon per AE_HOME | RATIFIED (both seats, 2026-08-20) |
| DR-003 | At-least-once outbound Telegram delivery | RATIFIED (both seats, 2026-08-20) |
| DR-004 | Durable inbox, coalesced notification | RATIFIED (both seats, 2026-08-20) |
| DR-005 | Exact identity or loud refusal | RATIFIED (both seats, 2026-08-20) |

```
DR-005 Exact identity or loud refusal
- affected SC ids: SC-704c, SC-704d (bucket 4 under this DR); SC-704b (#56's intended
  behavior is defined by this DR); interacts with #50 (resume preflight) and the
  matrix's per-tool resume/fallback rows (heuristic cells become historical IS).
- context / current IS: claude/grok fall back to --continue (CWD heuristic), gemini to
  --resume latest (recency), opencode capture is cwd/time-heuristic (#56); the codex
  registration exhibit captured the HUMAN's own session in the same cwd — gatekeeping
  classifies ambient-derived identity as a CONFIDENTIALITY failure, not bookkeeping.
- options considered: (a) weaken never-cross-wires and accept documented
  confidentiality risk; (b) exact identity or loud refusal.
- decision / intended Rust behavior: capture records a provider session id only from a
  positively slot-owned signal; resume acts on an exactly-owned id only. If the id is
  absent, pending, ambiguous, or the current profile store does not match, the command
  fails BEFORE any tmux/meta mutation, with a diagnostic and an explicit reset/
  new-session remedy — never `--continue`, never `latest`, never silently fresh, never
  overwriting stored identity (#50: fresh-on-mismatch destroys reachability). The
  distinction that keeps SC-704e coherent: a launch-script rerun with NO established
  conversation may honestly start fresh; a command acting on SAVED resume state may
  not. Heuristic resume/capture is removed across ALL adapters, not patched per tool.
- trade-offs accepted: a session whose id was never captured requires explicit operator
  action instead of a convenient guess; convenience lost, cross-wire class gone.
- authority: gatekeeping ambient-derived-identity doctrine + #56/#50 evidence + the
  matrix (empirical).
- seats + date: gpt56sol:colead (proposed) + fable5:lead (concurred), 2026-08-20.
```

```
DR-004 Durable inbox, coalesced notification
- affected SC ids: SC-200; the S3 paste-delivery rows (stand until the P2 messaging
  cutover implements this); S5 mailbox persistence + S6 `msg` CLI output rows (drafted
  at P2 under this DR); #82 disposition RR(P2)+DR-004.
- context / current IS: delivery conflates notification with payload in a pane a human
  reads; worker fan-in floods the human lane (#82 problem statement).
- options considered (per #82): topology restrictions (rejected — breaks tracked
  replies, compact handover, dissent lane; makes the challenger the channel); pull
  model (rejected — reintroduces polling); presentation fix (chosen).
- decision / binding outcomes (from #82 acceptance, refinable in implementation only):
  every message body persists to the ONE existing ledger/body store; pane notification
  is edge-triggered and coalesced — at most ONE input-safe line per unread epoch,
  carrying trusted metadata only (origin, type, ref, count), NEVER sender-authored text
  (the #59 injection-sink class); bodies render only via msg read inside the origin
  envelope; the body is stored BEFORE any notification (the store is the transport —
  pane failure loses no body); N queued messages yield N ordered authenticated bodies
  with ≤1 pane line before ack, an exact unread count, and exactly one NEW edge after
  ack; reply routing, origin envelope, and request tracking unchanged; per-agent
  delivery policy with full-paste compatibility retained — any DEFAULT change requires
  its own ruled row after the coalesced gate; NO agent-to-lead write prohibition, ever.
- trade-offs accepted: a second read step (msg read/ack) for coalesced agents; extra
  ack state.
- authority: #82 (acceptance section = the binding outcomes), both-seat design record
  therein.
- seats + date: fable5:lead + gpt56sol:colead, 2026-08-20 (written at P0 per gate — P2
  refines implementation, never these outcomes).
```

```
DR-003 At-least-once outbound Telegram delivery
- affected SC ids: SC-958 (bucket 4 under this DR); #86's outbound half (scope refined
  by issue comment); inbound is NOT affected — SC-951/SC-960 stay at-most-once.
- context / current IS: telegram.md:9-12,167-169 promise saved offsets prevent restart
  replay; census-3 I3 shows sends succeed while the durable offset save fails silently.
  After a remote send succeeds, a crash before local cursor commit makes exactly-once
  unachievable without a remote idempotency primitive; persist-before-send trades
  duplicates for LOST notifications.
- options considered: (a) fix-known-defect preserving no-replay via at-most-once —
  explicitly accepts silent alert loss; (b) at-least-once with explicit policy.
- decision / intended Rust behavior: at-least-once, with the crash-window bound made
  MECHANICAL (gate ruling): after remote success with a failed cursor commit, the
  pending cursor is retained and the COMMIT ALONE is retried — the message is never
  re-sent while the process lives; only a restart can duplicate. Cursor-commit failure
  is LOUD. The dedupe id is stable and sourced: `session_id + generation + offset`
  (the DR-001 cursor triple), carried in the outbound text/log.
- trade-offs accepted: occasional visible duplicate over silent alert loss — for a
  notification bridge, a lost alert is the worse failure; doc no-replay promise amended
  for outbound.
- authority: telegram.md/bridge-protocol.md (current promise), census-3 I3 (evidence),
  ambitious-divergence latitude.
- seats + date: gpt56sol:colead (proposed) + fable5:lead (concurred), 2026-08-20.
```

```
DR-002 One daemon per AE_HOME
- affected SC ids: SC-901 (topology), SC-929 (restart contract), SC-924 (the
  `_watchdog` pane retires, `_events` survives); interacts with SC-963/964 (#83/#84 —
  the handoff protocol they afflict ceases to exist under one owner) and SC-957
  (supervision honors explicit stop — preserved).
- context / current IS: two daemon runtimes (bash per-session _watchdog process/pane +
  machine bridge; python aewatch machine singleton under AE_WATCHDOG_IMPL=uv) with a
  marker/heartbeat handoff protocol between them — probe-proven fail-open (#84),
  operator bypass (#83), store regression (#86), config divergence (#87).
- options considered: (a) preserve both topologies in Rust (duplicates ownership and
  process supervision that #79 already retires); (b) one Rust daemon per AE_HOME
  (the aewatch ownership topology).
- decision / intended Rust behavior: ONE Rust daemon per AE_HOME owns watchdog +
  telegram at P4. AE_WATCHDOG_IMPL and the per-session _watchdog process/pane retire.
  Kept: per-session enable/persistence/start|stop|status semantics; ae-monitor/_events
  inspectability; daemon decisions exposed via durable events/log, never pane-peeking.
- binding conditions (fable5:lead): (1) a dead daemon is VISIBLE (ae list/status) and
  self-heals via race-free ensure-at-launch — never silent global no-nudges; (2)
  complete-scope discovery is a design requirement (#84's class unrepresentable);
  (3) per-session state stays session-scoped — one process, never one blob; (4) an
  explicit restart contract (upgrade/doctor-refresh behavior stated, restarts loud in
  events); (5) durable decision events fully replace peek _watchdog (#19 ae explain is
  the consumer).
- trade-offs accepted: single supervised process replaces N independent ones (SPOF
  accepted against condition 1); observable topology changes (pane and selector gone).
- authority: epic #79 P4 (both runtimes retire); census-3 (measured aewatch topology =
  the incumbent design); Clemens' ambitious-divergence latitude.
- seats + date: gpt56sol:colead (proposed) + fable5:lead (concurred, five conditions),
  2026-08-20.
```

```
DR-001 Event-log generations
- affected SC ids: SC-900 (S10 event-log container lifecycle, bucket 4 under this DR);
  S10/S14 reader rows (census-3 audit I1 stat/open race); B3 (census-3 addenda).
  SC-511c (additive event-OBJECT keys) is NOT affected — the conflict is with the
  no-rotation CONTAINER promise, and SC-511c stays bucket 2 conflict=none (consistency
  hold resolved 2026-08-20).
- context / current IS: bridge-protocol.md:90-95 + events.md:142-148 promise append-only,
  no rotation, lifetime growth; frozen ae:18046-18075 REPLACES events.jsonl with the
  newest N lines on resume (probe-verified reader losses, census-3 audit I1).
- options considered: (a) fix-known-defect — restore append-only forever: resurrects
  unbounded growth, the trim exists because retention is a real need; (b) DR —
  explicit generations.
- decision / intended Rust behavior: explicit event-log GENERATIONS. Conditions
  (binding, gpt56sol:colead): append-only WITHIN a generation; atomic writer/reader
  generation handoff under ONE shared protocol; a reader drains a stable opened
  generation before advancing; the cursor persists generation + offset; retention and
  lagging-reader data-loss policy are explicit; bash/Rust coexistence either speaks the
  same protocol or rotation stays DISABLED until one owner. May resolve the stat/open
  race only after those guarantees are written and tested.
- trade-offs accepted: doc promise amended (no-rotation clause retired); reader
  implementations must become generation-aware; #21 (schema versioning) becomes the
  implementation vehicle — this is the first format evolution.
- authority: bridge-protocol.md + events.md (current promise), census-3 audit I1/B3
  (evidence), Clemens' ambitious-divergence latitude (recorded above).
- seats + date: fable5:lead (proposed) + gpt56sol:colead (concurred, conditions),
  2026-08-20.
```

## Open-issue migration dispositions

Recorded on the epic (#79), not here — an explicit **pre-ratification gate**. Quadrant
(ratified ruling, ae-20260820T075423Z-7c2ce445): `rust-requirement` | `migration-enabler`
(owner, phase-by-which-needed, gate protected, gate-impact: `gate-integrity` |
`gate-cost`) | `wontfix-by-policy` | `stays-python-contrib`.
