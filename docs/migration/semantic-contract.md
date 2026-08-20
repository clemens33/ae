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
`classified_by` fills at the joint pass — these are lead drafts.

**SC-500 — compact stdout is exactly four lines or empty.**
- normative: bucket 2 (compat promise) — `Archived`, `Archive:`, `Digest:`, `Recovery:`,
  that order, nothing else, and EMPTY unless the boundary was crossed; non-empty stdout
  means the session really was archived and replaced. Authority:
  docs/internals/architecture.md ("Its stdout is a contract"), normative doc contract.
- empirical: pending (census-2 compact section + integration pins).
- conflict: none (draft).

**SC-501 — compact stderr carries everything else.**
- normative: bucket 2 — end's progress, compact's frozen facts, the confirmation body,
  the question, and `Aborted.` all go to stderr, so a caller can pipe stdout and parse
  it. Authority: architecture.md, normative.
- empirical: pending.
- conflict: none (draft).

**SC-502 — `Recovery:` prints BEFORE the relaunch.**
- normative: bucket 1 (invariant) — past the relaunch the archive is published, the
  source is gone, and the process may exec and never return; a recovery command emitted
  from a failure handler does not exist when needed. Authority: architecture.md,
  normative.
- empirical: pending.
- conflict: none (draft).

**SC-503 — compact confirmation treats EOF as no.**
- normative: bucket 1 — a bare read returning 1 at end-of-input must refuse, not abort
  between the question and any word about what happened. Rust shape: closed stdin =
  refusal, never a crash or a yes. Authority: architecture.md, normative.
- empirical: pending.
- conflict: none (draft).

**SC-504 — the boundary report survives a departed consumer.**
- normative: bucket 1 — SIGPIPE is ignored while the boundary report writes and restored
  before exec; a consumer exiting early must not kill the process between archive and
  launch, and the ignored disposition must not leak into children. Rust shape: report
  write errors handled, never fatal at the boundary, no signal-disposition leak.
  Authority: architecture.md, normative.
- empirical: pending.
- conflict: none (draft).

**SC-505 — validation errors echo the grammar verbatim.**
- normative: bucket 2 — `_validate_session_name` / `_validate_agent_name` error messages
  state the allowlist grammar exactly (`^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$` /
  `{0,63}$`), one definition each. Authority: #59 ruling + AGENTS.md allowlist bullets,
  normative.
- empirical: unit-test pins at 72c7293 (verification, not authority).
- conflict: none (draft).

**SC-506 — `list --json` never truncates mid-document.**
- normative: bucket 1 — one bad session degrades its entry; the JSON array always closes.
  Authority: AGENTS.md set-e rule ("Long emitters must not abort mid-output", guarded by
  a structural unit test — ruling bullet, normative). Shipped exhibit
  `_agent_alert_reason` is the defect this rule answers.
- empirical: structural unit guard at 72c7293 (verification).
- conflict: none (draft).

**SC-507 — `archive preview` stdout is the digest bytes and nothing else.**
- normative: PENDING SOURCE CHECK — stated today only in a code comment at the M2
  bootstrap site (weakest tier). If no doc-contract backing exists, flag
  `authority=code-observation` and close via seat ruling (expected: preserve, bucket 2 —
  the M2 stderr routing exists precisely to protect this).
- empirical: M2 census citation (ae:344-352 comment).
- conflict: pending normative closure.

**SC-508 — bash `ae` exit codes.**
- normative: `authority=code-observation` — no normative source names a dispatcher-wide
  exit-code contract. Seats classify after evidence (probe: exit codes per refusal
  family). Note: the Rust binary already pins 0 success / 2 usage (its own tests); the
  seam between the two is a P1 decision row.
- empirical: pending probe.
- conflict: pending normative closure.

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

### S9 — Lifecycle transactions

git commit/push on end, archive capture ordering (archive-before-removal, failed archive
fails the end), `--purge-history`, transfer (both directions), rename, compact/handover,
`--from <uuid>` lineage, recover-pending.

<!-- rows: SC-8xx -->

### S10 — Daemons/sidecars + contrib boundary

Watchdog (nudge rules, quiet-state honoring, footprint exclusion; bash impl vs
`AE_WATCHDOG_IMPL=uv` aewatch), telegram bridge (chat events, reply routing, markdown/jq
injection boundaries; bash daemon vs aewatch runtime handoff via marker + fresh
heartbeat), `ae steward` + ae-monitor window (bash product surfaces) vs contrib
templates/sidecars.

<!-- rows: SC-9xx -->

### S11 — Installer / doctor / refresh

Install contract (symlink or curl|bash), `doctor --refresh` regeneration boundary
(running watchdog keeps loaded body), `_publish_executable_artifact` chokepoint.

<!-- rows: SC-10xx -->

### S12 — Platform/dependency degradation

GNU/BSD shims, optional `flock`/`timeout` degradation, `sun_path` limits, uuid case,
bash >= 4.0 floor for the remaining glue.

<!-- rows: SC-11xx -->

### S13 — Identity/provenance security surface

System-prompt interpolation (#59): agent/session name allowlists at every creation
boundary, fresh=fatal vs restored=fail-quiet provenance rule, derived names as grammar
fixed points, message-envelope authority (human = no envelope).

<!-- rows: SC-12xx -->

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
| — | (none yet) | |

## Open-issue migration dispositions

Recorded on the epic (#79), not here — an explicit **pre-ratification gate**. Quadrant
(ratified ruling, ae-20260820T075423Z-7c2ce445): `rust-requirement` | `migration-enabler`
(owner, phase-by-which-needed, gate protected, gate-impact: `gate-integrity` |
`gate-cost`) | `wontfix-by-policy` | `stays-python-contrib`.
