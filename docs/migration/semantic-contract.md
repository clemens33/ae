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

### S3 — Generated helper CLIs (every one — census: `helper_*_main` at 72c7293)

`send`, `ask`, `review`, `reply`, `requests`, `state`, `mark-done`, `say`, `memo`, `goal`,
`peek`/`peak`, `agents [--all]`, `focus`, `interrupt`, `spawn`, `retire`, `loop`,
`watchdog`, `events-tail`, `_register-sid`, `_lib` name resolution (`alias:name`,
alias-only, bare name, `%pane-id`, `@session:agent`), delivery semantics (defer on
busy/human-typed, verify submit, fail loud).

<!-- rows: SC-2xx -->

### S4 — Config INI grammar

Dialect, key grammar, defaults, precedence (incl. `AE_LOCAL_CONFIG`), malformed-line
behavior.

<!-- rows: SC-3xx -->

### S5 — On-disk formats (live + archive + claims)

`~/.ae/sessions/<name>/` layout (`meta`, `events.jsonl`, `memo.tsv`, request records,
locks, `workspace.md`, generated helpers), `~/.ae/archive/<uuid>/` (inert; #48 format +
digests), claims (#71 — first Rust-native), `launch.<slot>.sh` + `.started` marker,
hub/steward dirs (`AE_HUB_DIR`, `AE_STEWARD_DIR`).

<!-- rows: SC-4xx -->

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

**SC-704 — adapter behavior is pinned by table-driven tests from measured facts.**
Bucket 1 — the capability matrix ports as test tables; upstream tool drift is detected
(canary class, #22), never silently absorbed. Authority: epic #79 (folded
requirements). Empirical: the matrix itself. Conflict: none.

**SC-705 — tool detection never parses the injected tail.** Bucket 1 — classification
looks only at the first word after stripping `env`/`-u`/`-i`/`VAR=val` prefixes and
tolerates launcher suffixes (`opencode.exe`); the kilobytes of injected prose are data.
Authority: AGENTS.md ruling bullets (tool_kind_from_cmd + pane_current_command).
Empirical: measured exhibits @72c7293. Conflict: none.

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

The S10 claim table (source: s10source batch, one testable SHOULD per row; frozen-doc
citations in the batch, memo-linked). Bucket column is the lead proposal; rows marked
**DR-002** have their bash-era SHOULD retired by the topology decision and are kept as
compatibility semantics only where DR-002 preserves them:

| Row | Claim (abbrev.) | Bucket | Conflict |
|---|---|---|---|
| W-01 | watchdog on by default; only false/no/off/0 disables | 2 | none |
| W-02 | enablement persists in meta across resume | 2 | none |
| W-03 | watchdog self-terminates when session/meta disappears | 1 | none |
| W-04 | bash default impl; aewatch via AE_WATCHDOG_IMPL=uv | 4/DR-002 | selector retires |
| W-05 | uv selects exclusively; else bash; never duplicates | 4/DR-002 | — |
| W-06 | reuse running aewatch only on fresh heartbeat | 4/DR-002 | — |
| W-07 | bridge ownership = marker AND heartbeat ≤90s | 4/DR-002 | — |
| W-08 | stale ownership → bash revives | 4/DR-002 | — |
| W-09 | interval/stale/nudge/throttle defaults 60s/15m/2/5 | 2 | none |
| W-10 | tg supervision default 120s; 0 disables | 2 | none |
| W-11 | steward sweep default 300s; 0 → normal behavior | 2 | none |
| W-12 | sweep retry 30s clamped, 6 fast, one unreachable alert | 2 | none |
| W-13 | branch order first-match-wins | 2 | none |
| W-14 | agentless shell pane: alert once, mark dead, ignore | 2 | none |
| W-15 | quiet declarations suppress; done event-only; wu/blocked yield to pane | 1 | none |
| W-16 | throttle streak: suppress, one event, alert at N, cleared on recovery | 2 | none |
| W-17 | at most MAX_NUDGES; limit cycle alerts then silence | 2 | none |
| W-18 | missing panes alert once; pending captures retried | 2 | none |
| W-19 | state done dual-emits; either recognized by older watchdog | 2 | none (torn emit = D07/S14 row) |
| M-01 | ae-monitor window always; _events always; _watchdog only-while-running | 2 (+4/DR-002 for the _watchdog pane half) | — |
| M-02 | watchdog stop/restart leaves events pane usable | 2 | none |
| M-03 | baked status bar owned by ae; stop clears live segments only | 2 | none |
| M-04 | doctor --refresh replaces disk code, never the running watchdog | 2 | none |
| B-01 | external actor = platform:id, opaque past allowlist | 2 | none |
| B-02 | event-only sinks: no tmux resolution, literal target; unknown fails loud | 1 | none |
| B-03 | AE_SENDER_OVERRIDE for send/ask/review; reply identity via --as | 2 | none |
| B-04 | readers tolerate missing files, buffer partial trailing records | 1 | none |
| B-05 | offsets keyed (session_id,inode); tail never whole-load | 2 | DR-001 amends (generation cursor) |
| B-06 | bridges bind stable session_id across resume/rename/transfer | 1 | none |
| B-07 | claim, stop the other, THEN send; fresh marker = stand down | 1 | fix-known-defect(#84; #83 operator bypass) |
| B-08 | bridges ignore unknown fields; renames/removals breaking | 2 | none |
| T-01 | missing jq/curl refuses only the bridge, never core | 1 | none |
| T-02 | default include set; per-session offsets prevent replay | 2 | fix-known-defect(#86: offsets can regress) |
| T-03 | empty allowed_user_ids = outbound-only | 2 | none |
| T-04 | inbound: numeric allowlisted from.id + exact chat.id + private, else silent drop | 1 | none |
| T-05 | routing precedence chain with common revalidation | 2 | none |
| T-06 | targets resolve to running exact/unique-prefix + real agents; escapes rejected | 1 | none |
| T-07 | setMyCommands failure logged and ignored | 2 | none |
| T-08 | offset advances BEFORE dispatch — restart cannot replay a side-effecting command | 1 | none |
| T-09 | token file 0600 owner-only; wrong perms refuse start with diagnostic | 1 | none |
| T-10 | start idempotent; stop-when-stopped succeeds; status = intent + runtime | 2 | none |
| T-11 | autostart never blocks launch; failure = one-line warning | 1 | none |
| T-12 | supervision ~120s, 0 disables, never undoes explicit stop, best-effort | 2 | none |
| T-13 | tracked sessions resume from saved offset; new sessions start at EOF | 2 | DR-001 amends (cursor) |
| T-14 | both backends: same filter/routing/menu over same state | 3 | fix-known-defect(#45 delivery, #86 stores, #87 config) |
| T-15 | token never in argv; redacted in logs | 1 | none |
| C-01 | steward session swept every 300s, no stale escalation; workers normal | 2 | none |
| C-02 | sweep nudges delivery-checked, retried, at-least-once | 2 | none |
| C-03 | steward liveness = pane checks + state-file mtime ~2x cadence | 2 | none |
| C-04 | steward never ends/stops/edits another session or dispatches without human say-so | 1 | none |
| C-05 | steward isolates config, neutralizes local, --init never overwrites | 1 | none |
| C-06 | legacy hub supported; steward name reserved | 2 | none |
| A-01 | core deps = bash>=4 + tmux + git; jq/curl never core | 1 | none |
| A-03 | helper publication atomic temp+chmod+rename; running watchdog keeps body | 1 | none (M3 mechanism) |
| A-04 | telegram plain-text paths; jq programs fixed, data via stdin | 1 | none |

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

### S12 — Platform/dependency degradation

GNU/BSD shims, optional `flock`/`timeout` degradation, `sun_path` limits, uuid case,
bash >= 4.0 floor for the remaining glue.

<!-- rows: SC-11xx -->

**SC-1100 — behavior is identical on GNU and BSD userlands.** Bucket 1 — the bash era
enforces this via the shim table (never call the raw divergent tool); the Rust core
gets it by construction (std lib), and the class of silent `|| fallback` platform bugs
dies. Authority: AGENTS.md GNU/BSD section (ruling: use the shim) — the per-tool rows
are empirical exhibits. Empirical: shipped macOS bugs @72c7293. Conflict: none.

**SC-1101 — `flock` and `timeout` are optional dependencies.** Bucket 3 — SHOULD:
guard with `command -v` and DEGRADE, never hard-require (core ae runs on a bare
bash+tmux+git machine). IS at 72c7293: `ae_meta_set` and `_lib` paths hard-require
flock and die `command not found` (#75); the missing-flock matrix (census-2 addenda)
shows divergent per-path outcomes. Conflict: fix-known-defect(#75, intended: the Rust
core needs NO external flock — native locking; remaining glue degrades loudly).
Empirical: census-2 missing-flock matrix.

**SC-1102 — session/archive UUIDs are canonical lowercase.** Bucket 2 — `gen_uuid`
normalizes (macOS `uuidgen` is uppercase); validators and filenames are
lowercase-only. Authority: AGENTS.md bullet (ruling). Empirical: pending.
Conflict: none.

**SC-1103 — unix socket paths fit `sun_path` on both platforms.** Bucket 1 — 104 bytes
macOS / 108 Linux; anything creating sockets accounts for the tighter bound.
Authority: AGENTS.md bullet. Empirical: measured mktemp overflow exhibit.
Conflict: none.

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

### S15 — Environment controls (census: `AE_*` at 72c7293)

`AE_HOME`, `CONFIG_FILE`/`AE_LOCAL_CONFIG`, `AE_TMUX_SERVER`, `AE_WATCHDOG_IMPL`,
`AE_NO_AUTOSTART`, `AE_END_SERVER`, `AE_HUB_DIR`, `AE_STEWARD_DIR`, `AE_EVENTS_KEEP`,
`AE_SEND_DEFER_SEC`, `AE_ATTN_REQUEST_SECS`, `AE_LIST_ACTIVE_SECS`,
`AE_COMPACT_HANDOVER_SECS`, `AE_WATCHDOG_*` tunables, `AE_LOOP_*` tunables,
`AE_TELEGRAM_*` tunables, `AE_SENDER_OVERRIDE`, launch-token/slot vars
(`AE_CODEX_*`, `AE_GEMINI_*`, `AE_OPENCODE_*`), resolution vars exported to helpers
(`AE_RESOLVED_*`, `AE_SESSION`, `AE_META`, `AE_DIR`, `AE_MODE`, `AE_ORIGIN`, `AE_PATH*`).
Each row: default when unset, malformed-value behavior, failure mode.

<!-- rows: SC-14xx -->

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

```
DR-002 One daemon per AE_HOME
- affected SC ids: SC-901; W-04..08 and M-01's _watchdog-pane half (bash-era SHOULDs
  retired at P4); interacts with #83/#84 (the handoff protocol they afflict ceases to
  exist under one owner).
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
