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

**Empirical-status amendment (binding split, both seats, 2026-08-20,
ae-20260820T112501Z-d684b294):** every row's empirical field carries
`empirical_status = observed(pointer)` — accepted evidence that can fail against the
row — or `pending(owner + phase + probe-batch + protected gate)`. A pending row is NOT
empirical closure and is never described as an IS observation. RATIFICATION-CRITICAL
rows (IS-dependent seat rulings; bucket-3/4 baselines lacking accepted evidence; P1
corpus/query surfaces; destructive/data-loss/security/identity/format/byte/exit
boundaries) must reach `observed` BEFORE the #81 ratification comments — no scoped-gap
escape. The deferrable lane exists only for bucket-1/2 rows with strong normative
authority, no conflict, and no P1 fixture dependency. Ratification language states:
normative/classification/conflict contract ratified; empirical baseline PARTIAL with
exact observed/pending counts and the id manifest. Pending rows lend SHOULD as named
authority only; a newly measured contradiction reopens the row for seat-only
bucket-3/DR resolution — measurement never rewrites SHOULD.

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

Launch/attach/resume decision (`ae [name]`, `--local/--copy/--worktree`, `--from <uuid>`,
`use <alias>`), `list [--json]` (alias `ls`), `status`, `next` (alias `jump`),
`end`/`rm` (`--purge-history`), `stop`, `stop all`, `rename`, `transfer`, `compact`,
`archive preview` (two words), `_recover-pending` (internal, helper-invoked — not
public surface), `doctor [--refresh]`, `watchdog …` (helper alias `loop`),
`telegram setup|start|stop|status`, `steward --init|--attach|--help|--detach` (flags,
not subcommands; deprecated alias `hub`), `help`/`version`, exit codes. (Census:
`cmd_*` functions + dispatcher arms at 72c7293.)

<!-- rows: SC-0xx -->

### S2 — Session modes and states

`--local/--copy/--worktree`; fresh / resume / stopped / inside-session / `--all` /
default-name resolution (`default_session_name` guarantees the grammar).

<!-- rows: SC-1xx -->

**SC-100 — a derived default session name GUARANTEES the grammar.** Bucket 1 —
`default_session_name` produces a valid name for ANY working directory rather than
being checked against the grammar afterwards. Authority: AGENTS.md session-name bullet
(ruling). Empirical: unit pins @72c7293. Conflict: none.

**SC-101 — the running-session fast path's mutation exclusion.**
  Empirical: census-2 launch section. Conflict: pending seat closure (UNCLASSIFIED).
`authority=code-observation` (gate correction: "pure attach, autostarts excepted" was
self-contradictory, and architecture.md says RESUME regenerates assets/monitor/
watchdog — the attach-vs-resume boundary needs an exact phase-bounded mutation
exclusion). Census-2 records the fast path taking no lifecycle lock; the precise
mutation set is a seat ruling after the probe. UNCLASSIFIED pending closure.

**SC-102a — resume of a stopped session.** `authority=code-observation` — what resume
  Empirical: census-2 launch section. Conflict: pending seat closure (UNCLASSIFIED).
regenerates (assets/monitor/watchdog per architecture.md) vs preserves; probe + seat
ruling. UNCLASSIFIED pending closure.

**SC-102b — invocation from inside a session.** `authority=code-observation` — the
  Empirical: census-2 launch section. Conflict: pending seat closure (UNCLASSIFIED).
inside-session decision surface; probe + seat ruling. UNCLASSIFIED pending closure.

**SC-011 — `rm` is an alias of `end`.** Bucket 2 — same operation, both spellings.
Authority: commands.md end section. Empirical: pending. Conflict: none.

**SC-012 — `help` prints usage.** Bucket 2 — `ae help` (and the bare-invocation help
path) prints the command surface; inherits the M2 bootstrap caveat like every
dispatcher entry. Authority: commands.md. Empirical: pending. Conflict: none.

**classified_by (S1 preflight MARK, ae-20260820T115449Z-1b7ef041): SC-016a, SC-016b,
SC-016c, SC-016d, SC-017a..i, SC-020a, SC-020b, SC-020c — fable5:lead +
gpt56sol:colead, 2026-08-20. Normative/conflict lane only (SC-020b is bucket 1 — the
existence re-check safety invariant; the rest of this exact set bucket 2;
conflict=none throughout). Empirical remains pending C-cluster observation; this MARK
does not ratify IS.**

**SC-013 — `steward --help`/`--detach` flag surface.** `authority=code-observation`.
Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).

**SC-016a — `ae status [name]` signature.** Bucket 2. Authority: commands.md:134.
Empirical: pending. Conflict: none.

**SC-016c — status defaults to the current session when run inside one.** Bucket 2.
Authority: commands.md:42. Empirical: pending. Conflict: none.

**SC-016d — status never attaches.** Bucket 2 — read-only inspection. Authority:
commands.md:134-136. Empirical: pending. Conflict: none.

**SC-016b — status prints ~80 labeled lines per agent.** Bucket 2 — last ~80 pane
lines per agent, each marked with binary name and pane id. Authority:
commands.md:134-136. Empirical: pending. Conflict: none.

**SC-017a — `list` shows running sessions only by default.** Bucket 2 — stopped
history is opt-in noise. Authority: commands.md:78-81. Empirical: pending.
Conflict: none.

**SC-017b — `--all` shows running sessions, then stopped.** Bucket 2. Authority:
commands.md:82-84. Empirical: pending. Conflict: none.

**SC-017c — `--stopped` shows stopped sessions only.** Bucket 2. Authority:
commands.md:85. Empirical: pending. Conflict: none.

**SC-017d — `--needs-attn` filters to attention sessions; aliases accepted.** Bucket 2
— `--needs-me`/`--needs`/`--attn`. Authority: commands.md:86. Empirical: pending.
Conflict: none.

**SC-017e — `--active` filters on recent activity.** Bucket 2 — an ae event within
~5min, `AE_LIST_ACTIVE_SECS` tunes, `--busy` alias. Authority: commands.md:87.
Empirical: pending. Conflict: none.

**SC-017f — `--json` honours the active filters.** Bucket 2. Authority:
commands.md:88. Empirical: pending. Conflict: none.

**SC-017g — the attention marker is the single most-actionable reason.** Bucket 2 —
dead > stale > waiting-user > blocked > throttled > unanswered, derived as the MAX
across agent reasons PLUS session-level unresolved-request facts (amended slice-1b:
unanswered is a PAIR fact with no owning agent — cross-session ask/review makes
target ownership non-local; agents[].reason never reads unanswered). Authority:
commands.md:60-76 + slice-1b joint ruling. Empirical: pending. Conflict: none.
**classified_by: RE-MARKED after amendment — both seats, 2026-08-20.**

**SC-017h — the tabular view shows per-agent health, declared state, and the session
attn marker.** Bucket 2. Authority: commands.md:56-59. Empirical: pending.
Conflict: none.

**SC-018 — `ae [name] use <alias>` starts the session with that agent as main.**
Bucket 2. Authority: commands.md:5. Empirical: pending. Conflict: none.

**SC-018b — `use` interaction with resume.** `authority=code-observation` — what
`use` does against an existing/resumable session is undocumented; probe + seat ruling.
Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).

**SC-019 — `jump` is an alias of `next`.** Bucket 2. Authority: commands.md:10-11.
Empirical: pending. Conflict: none.

**SC-017i — `--running` is the explicit spelling of the default filter.** Bucket 2.
Authority: commands.md:83. Empirical: pending. Conflict: none.

**SC-020a — `next --attach` switches inside tmux, attaches outside.** Bucket 2 —
`tmux switch-client` when already in tmux, `tmux attach-session` otherwise; `--switch`
is its alias. Authority: commands.md:155-157. Empirical: pending. Conflict: none.

**SC-020b — `--attach` re-checks the session still exists first.** Bucket 1.
Authority: commands.md:157-158. Empirical: pending. Conflict: none.

**SC-020c — `--attach` no-ops with a message when already current.** Bucket 2.
Authority: commands.md:158. Empirical: pending. Conflict: none.

**SC-014 — `version` output surface.** `authority=code-observation` — prints the
CalVer `AE_VERSION`; exact format unpinned by docs. Empirical: pending probe.
Conflict: pending seat closure (UNCLASSIFIED).

Machine-readable S1 surface→row declaration table (checker-validated: every inventory
item declared, every target id must exist):

```
S1MAP: launch -> SC-100 SC-101 SC-102a SC-102b SC-813
S1MAP: --local/--copy/--worktree -> SC-306
S1MAP: --from -> SC-822 SC-823 SC-824a SC-824b SC-825a
S1MAP: list -> SC-017i SC-017a SC-017b SC-017c SC-017d SC-017e SC-017f SC-017g SC-017h SC-509 SC-506 SC-1306a
S1MAP: ls -> SC-017i SC-017a SC-017b SC-017c SC-017d SC-017e SC-017f SC-017g SC-017h SC-509 SC-506 SC-1306a
S1MAP: status -> SC-016a SC-016b SC-016c SC-016d SC-1306b
S1MAP: next -> SC-513a SC-513b SC-513c SC-020a SC-020b SC-020c SC-1306c
S1MAP: jump -> SC-019
S1MAP: use -> SC-018 SC-018b
S1MAP: end -> SC-516 SC-817 SC-819 SC-820a SC-820b SC-821a SC-821b SC-826 SC-838a SC-838b
S1MAP: rm -> SC-011
S1MAP: --purge-history -> SC-810a SC-810b SC-818a SC-818b SC-818c SC-818d SC-818e
S1MAP: stop -> SC-835a SC-835b SC-835c SC-835d SC-835e SC-835f SC-835g SC-835h SC-839a SC-839b SC-839c SC-839d SC-839e SC-1302
S1MAP: stop all -> SC-515a SC-515b SC-515c SC-815a SC-815b SC-815c SC-815d SC-816
S1MAP: rename -> SC-832a SC-832b SC-832c SC-1303 SC-813
S1MAP: transfer -> SC-833a SC-833b SC-833c SC-833d SC-814 SC-1304a SC-1304b SC-1304c SC-1304d
S1MAP: compact -> SC-500 SC-501 SC-502 SC-503a SC-503b SC-504a SC-504b SC-512 SC-517a SC-517b SC-517c SC-827 SC-828 SC-829a SC-829b SC-830 SC-831 SC-836 SC-837 SC-1305
S1MAP: archive preview -> SC-507a SC-507b SC-507c SC-507d
S1MAP: _recover-pending -> SC-834a SC-834b SC-834c
S1MAP: doctor -> SC-514 SC-1002
S1MAP: doctor --refresh -> SC-1001 SC-929
S1MAP: watchdog -> SC-902 SC-904 SC-926 SC-927
S1MAP: telegram setup -> SC-969 SC-970
S1MAP: telegram start -> SC-953 SC-956 SC-963 SC-971
S1MAP: telegram stop -> SC-954 SC-957 SC-971
S1MAP: telegram status -> SC-955
S1MAP: steward --init -> SC-932
S1MAP: steward --attach -> SC-931
S1MAP: steward --help -> SC-013
S1MAP: steward --detach -> SC-013
S1MAP: hub -> SC-939f
S1MAP: help -> SC-012
S1MAP: version -> SC-014
S1MAP: loop -> SC-902 SC-904 SC-926 SC-927
S1MAP: -h/--help -> SC-012
S1MAP: -V/--version -> SC-014
S1MAP: exit codes -> SC-513a SC-514 SC-515a SC-516 SC-517a SC-508
```

Alias rule — JOINT NORMATIVE RULING (both seats, 2026-08-20, revised per confirmation
gate): TWO alias models, consistently applied. (a) PASS-THROUGH aliases with an own
identity row (`rm`→SC-011, `jump`→SC-019, `hub`→SC-939f): the row asserts operation
identity and equivalence is TRANSITIVE to the canonical surface's full set — the map
lists the identity row only. (b) SPELLING aliases without own rows (`ls`, `loop`,
`-h/--help`, `-V/--version`): the map lists the FULL canonical target set; a partial
spelling-alias mapping is a finding. `use` is NOT an alias (own rows SC-018/018b).
The aggregate "refusal contracts" pseudo-item is DELETED — per-surface rows own their
refusals (gate ruling).

### S3 — Generated helper CLIs (every one — census: `helper_*_main` at 72c7293)

`send`, `ask`, `review`, `reply`, `requests`, `state`, `mark-done`, `say`, `memo`, `goal`,
`peek`/`peak`, `agents [--all]`, `focus`, `interrupt`, `spawn`, `retire`, `loop`,
`watchdog`, `events-tail`, `_register-sid`, `_lib` name resolution (`alias:name`,
alias-only, bare name, `%pane-id`, `@session:agent`), delivery semantics (defer on
busy/human-typed, verify submit, fail loud).

<!-- rows: SC-2xx -->

**SC-200 — delivery-model evolution.** Bucket 4 — **DR-004** (ratified): the durable
  Empirical: n/a (successor design under DR-004).
per-agent inbox with coalesced notification replaces the paste-delivery model at P2;
the paste rows stand until that cutover. Authority: DR-004 (both seats).
Conflict: DR-004.

**SC-201 — text is never pasted into a shell.** Bucket 1 — notification-safety
invariant that SURVIVES DR-004: a target pane fallen to a shell is refused with the
named reason; nothing is injected (a stray Enter would execute the message). Under the
P2 inbox the same invariant governs the notification line. Authority: helpers.md "How
send delivers" 1. Empirical: pending. Conflict: none.

**SC-202 — a human's unsent input is never clobbered.** Bucket 1 — survives DR-004:
injection into a modelled TUI defers fail-closed while the input box is non-empty,
mid-generation, or unreadable, and abandons loudly rather than clobbering. Authority:
helpers.md 2. Empirical: pending. Conflict: none.

**SC-203 — delivery uncertainty is typed, never silent.** Bucket 1 — survives DR-004:
submit is verified after injection (bounded nudges) and unconfirmable delivery is a
LOUD typed outcome; after P2 the uncertainty applies to the NOTIFICATION only, while
the stored body remains readable regardless. Authority: helpers.md 3 + DR-004
outcomes. Empirical: pending. Conflict: none.

**SC-204 — no durable outbox (until DR-004).** Bucket 4 — DR-004: at 72c7293 a loud
  Empirical: helpers.md 4 is the frozen IS ("not a queue").
failure is the re-send signal ("ae is not a queue"); the P2 inbox makes the store the
transport and this promise retires. Authority: helpers.md 4 + DR-004.
Conflict: DR-004.

**SC-205 — one transport primitive delivers messages.** Bucket 1 — semantic (gate
correction: the literal one-helper-touches-tmux wording is false at 72c7293 —
interrupt issues its own cancel/Escape and focus selects panes): every MESSAGE
delivery (send/ask/review/reply, and interrupt's optional message) flows through the
one guarded primitive; pane actions that are not message delivery are their own
operations. Under DR-004 the primitive becomes the store-commit + notification path.
Authority: helpers.md composition + gate ruling. Empirical: census-1. Conflict: none.

**SC-206 — one path mints request ids.** Bucket 1 — `ae_tracked_send` is the single
mint point. Authority: helpers.md composition. Empirical: pending. Conflict: none.

**SC-207 — one validator pairs replies.** Bucket 1 — reply pairing is verified in one
place before delegation to send. Authority: helpers.md composition.
Empirical: pending. Conflict: none.

**SC-208 — every interaction crosses one emit point.** Bucket 1 — the surface is
auditable in events.jsonl because all messaging passes the same emit call. Authority:
helpers.md composition. Empirical: pending. Conflict: none.

**SC-209a — requests and replies are addressed by slot + session.** Bucket 1.
Authority: helpers.md "Slot identity". Empirical: pending. Conflict: none.

**SC-209b — reply verifies the sender's live slot against the stored slot.** Bucket 1
— before delivering. Authority: helpers.md. Empirical: pending. Conflict: none.

**SC-209c — the display name is never trusted for routing.** Bucket 1 — `--as` sets
display only and cannot bypass slot verification. Authority: helpers.md.
Empirical: pending. Conflict: none.

**SC-209d — routing survives display-name churn.** Bucket 1 — a reply reaches the
right agent after its display name changes. Authority: helpers.md. Empirical: pending.
Conflict: none.

**SC-210 — the unprotected-delivery degradation retires.** Bucket 4 — **DR-004**
(gate consistency rule: a b2 row must survive with conflict none, and this one
retires): at 72c7293 unmodelled tools receive without busy protection (only
claude/codex expose an input-state read — empirical boundary); under the P2
notification path the degradation ceases to exist. Authority: helpers.md closing note
+ DR-004. Empirical: matrix. Conflict: DR-004.

Documented helper SIGNATURES — one head per helper, bucket 2, Authority: the frozen
helpers/AGENTS.md helper table (gate correction: these signatures ARE documented;
code-observation was wrong for them); Empirical: pending; Conflict: none:

**SC-212a — `goal [text|--clear]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212b — `memo add [--topic t] <text>` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212c — `requests [mine|inbox|all]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212d — `peek <agent> [lines]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212e — `agents [--all]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212f — `focus <agent>` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212g — `interrupt <agent> [message]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212h — `spawn <alias:name> [prompt]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212i — `retire <agent|pane-id>` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212j — `say` accepts args or piped stdin.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212k — `memo read [--topic t]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212l — `memo tail [n]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212m — `peak` is an alias of `peek`.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212n — peek default is 80 lines.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212o — peek maximum is 2000 lines.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212p — `interrupt` with no message cancels only.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212q — `retire` acts on spawned agents only.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212r — `say` emits a chat event.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212s — `state <working|waiting-user|blocked|done> [reason]` signature** (gate
  Bucket 2.
correction: documented at AGENTS.md@72c7293:96 — not code-observation).

Code-observed refusal/malformed modes — one head per helper
(`authority=code-observation`; Empirical: pending probe; Conflict: pending seat
closure; UNCLASSIFIED):

**SC-211a — `state` refusal/malformed modes** (the signature is SC-212s; only the
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
residue is code-observed).
**SC-211b — `goal` refusal/malformed modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211c — `memo` refusal/malformed modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211d — `requests` refusal/malformed modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211e — `peek` out-of-bounds and refusal modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211f — `agents` failure modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211g — `focus` refusal modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211h — `interrupt` refusal modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211i — `spawn` non-name argument errors** (name validation is SC-1201).
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-211j — `retire` refusal modes.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).

**SC-211k — `mark-done` is an exact alias surface.** Bucket 2 — exec shim over
`state done`. Authority: the frozen helper table (documents mark-done as the state-done
shim). Empirical: ownership D07 verification + census-1. Conflict: none.

**SC-211l — `say` refusal/failure modes.** `authority=code-observation`; Empirical:
census-1 D10; Conflict: pending seat closure. UNCLASSIFIED. (The documented signature
is SC-212j.)

**SC-211m — session `watchdog` + `loop` helper surface.** Bucket 2 — cross-reference:
behavior is SC-904/SC-926/SC-927 + D26a. Authority: those rows. Empirical: census-2.
Conflict: none.

**SC-211n — `events-tail` query surface.** `authority=code-observation` — snapshot cut
is SC-1306e. Empirical: census-1 D03. Conflict: pending seat closure. UNCLASSIFIED.

**SC-211o — Codex session identity is registered positively and slot-bound.** Bucket 2
— outcome-level (gate correction: commands.md calls `_register-sid` INTERNAL, so the
executable name is mechanism, not promise): the Codex UUID is registered/captured from
a positively owned, slot-bound signal. The `_register-sid` shim itself is empirical
mechanism whose retirement rides DR-006. Authority: commands.md:711-723 (outcome) +
DR-005/SC-704b (ownership rule). Empirical: census-1 D13. Conflict: none.

**SC-211p — `_lib` name resolution grammar.** Bucket 2 — exact `alias:name`,
alias-only when unique, bare name, `%pane-id`, and `@session:agent` all resolve; the
grammar is the documented helper contract. Authority: AGENTS.md session-helpers
section. Empirical: pending. Conflict: none.

### S4 — Config INI grammar

Dialect, key grammar, defaults, precedence (incl. `AE_LOCAL_CONFIG`), malformed-line
behavior.

<!-- rows: SC-3xx -->

**SC-300a — config syntax is the simple regex-parsed INI.** Bucket 2 — with the
AGENTS.md doctrine rule: don't extend the format (no TOML/YAML/JSON parsing).
Authority: AGENTS.md config section (ruling). Empirical: pending. Conflict: none.

**SC-300b — the recognized sections are `[agents]`/`[workspace]`/`[prompt]`/
`[telegram]`.** Bucket 2 — recognized-and-consumed set. Authority: config.md +
telegram.md. Empirical: pending. Conflict: none.

**SC-300c — unknown sections and unconsumed keys are ignored, not errors.** 
`authority=code-observation` — gate evidence: `parse_config` accepts arbitrary section
names and ignores unconsumed keys; seat ruling (preserve/fix) at the sweep.
UNCLASSIFIED pending closure.

**SC-307 — malformed-line behavior.** `authority=code-observation` — undocumented;
probe + seat ruling at the sweep. UNCLASSIFIED pending closure.

**SC-301 — an `[agents]` alias value is the launch command, doctor-verified.** Bucket
2 — the executable name is extracted and verified on PATH by doctor. Authority:
config.md `[agents]`. Empirical: pending. Conflict: none.

**SC-302 — env-prefixed alias commands are legal identity aliases.** Bucket 2 — one
binary, several logins via inline env prefix; each alias is its own identity.
Authority: config.md multiple-identities section. Empirical: pending. Conflict: none.

**SC-303 — env-prefixed commands get full tool handling.** Bucket 2 — cross-reference
to SC-705. Authority: SC-705's joint seat ruling (the classify-the-actual-executable
rule) + the #32 closure ruling in the frozen tree (7ab6457, ancestor of 72c7293).
Empirical: frozen unit pins tests/unit:1029-1053 (classification, context injection,
UUID mint, exact resume). config.md's limitation paragraph is STALE PROSE — docs fix
queued; stale docs never manufacture a defect. Conflict: none.

**SC-304 — per-project `.ae/config` shadows the global config key-by-key.** Bucket 2 —
full key-level shadowing (gate correction: config.md:3 — not `[prompt]`-only).
Authority: config.md:3. Empirical: pending. Conflict: none.

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

**SC-400a — the bash-era session layout remains READABLE.** Bucket 2 — legacy-read
compatibility survives every flip: a pre-flip session dir (`meta`, `events.jsonl`,
`memo.tsv`, `messages/`, locks, `workspace.md`, helpers, `launch.<slot>.sh` +
`.started`) is consumable by the successor. Authority: architecture.md + AGENTS.md.
Empirical: census-1/2. Conflict: none.

**SC-400b — the event store's written layout changes under DR-001.** Bucket 4 —
**DR-001**: generations replace the single `events.jsonl`, with legacy-read/migration/
write ownership stated at the flip commit. Authority: DR-001. Empirical: n/a
(successor design). Conflict: DR-001.

**SC-400c — generated-logic helpers retire from the written layout at P2.** Bucket 4 —
**DR-006** (gate ruling: every b4 carries a DR; the epic is authority, not a waiver).
Authority: DR-006 + epic #79 P2 + #76. Empirical: n/a (successor design).
Conflict: DR-006.

**SC-401a — the archive payload is the five-part set.** Bucket 2 — generated `meta`,
rendered `digest.md`, `memo.tsv`, `events.jsonl`, `messages/*` bodies (#48 format;
inertness proofs are SC-804/805). Survives all flips. Authority:
architecture.md:77-83. Empirical: census-2 end section. Conflict: none.

**SC-401b — an archive materializes ONE canonical event stream.** Bucket 4 —
**DR-001**, BOTH SEATS (lead ruling + colead binding precision, 2026-08-20):
publication takes a frozen cut — after writers quiesce, or under the same generation
protocol — and concatenates every RETAINED generation exactly once in total order.
"Complete" means the complete RETAINED history plus explicit retention/loss
provenance, never an impossible lifetime claim after retention. Generation container
files/boundaries are not exported; stable event/dedupe identities
(session_id+generation+offset) SURVIVE materialization. Archive consumers (digest,
`--from` preflight, inheritance) read one stream. Authority: DR-001 + this joint
ruling. Empirical: n/a (successor design). Conflict: DR-001.

**SC-402 — working directories stay clean.** Bucket 1 — ae writes its coordination
state under `~/.ae`, never into the project tree (`.ae/config` is the one deliberate
project-side file). Authority: AGENTS.md rules. Empirical: pending. Conflict: none.

**SC-403 — record framing round-trips every field faithfully.** Bucket 1 — semantic
(gate correction — the `\x1f` choice is the bash mechanism, empirical): an empty
field, free text with separators, and embedded-newline handling all round-trip without
field shift or phantom rows; typed Rust satisfies this by construction. Authority:
AGENTS.md TSV-framing ruling (the invariant behind it). Empirical: unit pins @72c7293
(the `\x1f` implementation). Conflict: none.

Meta key grammar (slice-1 Q1 seat rulings, source ownership corrected — meta supplies
what meta OWNS; derived facts cite their true sources):

**SC-405a — meta parse grammar.** Bucket 2 — `key=value` split on the FIRST equals;
single-line values. Authority: slice-1 joint seat ruling 2026-08-20 (the exact parser
semantics) + architecture.md:61-70 (layout authority — it says only INI-style).
Empirical: census-1/2 meta writers. Conflict: none. **classified_by: both seats,
2026-08-20.**

**SC-405b — the session-context keys.** Bucket 2 — `mode`, `origin`, `work_dir`,
`goal` are meta keys (goal per AGENTS.md@72c7293:102 `goal=` locked write). Authority:
architecture.md:61-70 + AGENTS.md:102. Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-405c — the roster keys.** Bucket 2 — `agent.<slot>` carries `alias:name:
provider-session-id` (SC-1207b) and `agent_bin.<slot>` the recorded binary. Authority:
architecture.md roster + #46/#50 rulings (recorded agent_bin). Empirical: census.
Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-405d — unknown meta keys are tolerated and never degrade.** Bucket 2 —
(slice-1b closure, both seats): the digest consumes only SC-405b/c; every other key is
tolerated silently — unknown keys are the normal state of real metas (the builder's
30-meta name census records MIGRATION-COMPATIBILITY pressure, not frozen IS; the
C-cluster still captures incumbent behavior). Malformed and duplicate keys DO degrade
interim per SC-509b's actual-loss test; SC-405e's probe still owes the exact malformed
shapes. There is NO enumerating row for the writer-key population (SC-405h REJECTED —
a live census never becomes contract; per-family S5 rows only when successor writers
need a SHOULD; the inventory stays evidence). Authority: slice-1b joint ruling.
Empirical: builder name-census + C-cluster pending. Conflict: none. **classified_by:
both seats, 2026-08-20.**

**SC-405i — a present session dir with MISSING meta is degraded.** Bucket 2 —
(slice-1b Q8): identity beyond the directory name and the entire roster are lost at
once — actual loss by SC-509b's own test; distinct from missing/empty EVENT logs,
which SC-519 makes quiet. Authority: slice-1b joint ruling + SC-509b. Empirical:
pending (C-cluster). Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-405j — an event carrying ANY routing key that does not fully and freshly match
stays UNASSOCIATED.** Bucket 2 — (slice-1b Q10, colead dissent adopted; PRECISED after
the builder's premise correction — the pre-existing code already refused stale
full-key events, and the actual change was the PARTIAL-key case): stale full keys,
mismatched keys, and partial keys (slot without session or session without slot) all
identify nobody; display-name matching exists ONLY for events with NO routing keys at
all (pre-SC-511a records depend on that surviving). Display fallback for keyed events
would create FALSE ATTRIBUTION against the SC-518/SC-511b loud direction; rename loss
is the KNOWN LIMITATION until SC-977's P2 stable identity. One shared invariant —
a total association decision function — so one row remains valid grain; builder tests
are candidate successor evidence, never frozen IS. **classified_by: REOPENED by the
precision and RE-MARKED on this exact text — fable5:lead + gpt56sol:colead,
2026-08-20.** Authority: slice-1b
joint ruling + SC-518/511b direction. Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-405k — agents[] membership is roster-defined.** Bucket 2 — (slice-1b Q11):
runtime-only panes/slots never invent agents; SC-509's agents[] fields are roster
fields; a missing roster/meta routes through SC-405i. Authority: slice-1b joint
ruling. Empirical: pending. Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-405e — malformed/duplicate key handling.** `authority=code-observation` — probe +
seat closure; never guessed. UNCLASSIFIED pending closure.

**SC-405f — `goal_set_epoch` is DERIVED from the latest goal event.** Bucket 2 — not a
meta key; the digest derives it from the event stream. Authority: slice-1 Q1 ruling +
commands.md (goal_set_epoch semantics). Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-405g — `branch` is the live tmux branch with a git fallback.** Bucket 2 — not a
meta key; per commands.md:124-129 (the watchdog's status segment, git fallback).
Authority: commands.md:124-129. Empirical: pending. Conflict: none. **classified_by:
both seats, 2026-08-20.**

**SC-404 — state roots derive from `AE_HOME` (default `~/.ae`) with explicit override
exceptions.** Bucket 2 — the DEFAULT derivation covers config, `sessions/`,
`archive/`, worktrees, daemon dirs; the exceptions are explicit overrides only:
`CONFIG_FILE` may point outside `AE_HOME`, project `.ae/config` shadows key-by-key
(SC-304), and `AE_STEWARD_DIR`/`AE_HUB_DIR` may relocate their dirs (frozen ae:56-64,
ae:10365-10370 — gate correction: not ALL roots). Authority: AGENTS.md +
architecture.md + config.md. Empirical: frozen source cites. Conflict: none.

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

**SC-509b — read/parse loss is visible in the digest.** Bucket 2 — additive schema
(slice-1 Q5 seat ruling, both seats): a session entry whose data suffered ACTUAL
read/parse loss carries `degraded: true` (additive key; normal entries may omit it);
identity (name + status) always survives; unreadable optional facts are omitted, never
fabricated, never null; `agents` remains an array. Damage is never rendered
identically to legitimate sparsity — a machine digest that hides loss lies by
omission. Authority: slice-1 joint Q5 ruling + SC-509's `schema_version` consumer-
gating design (the events evolution rule SC-511c is a different schema and is NOT the
authority here). Empirical: pending (builder implementation + C-cluster).
Conflict: none.
**classified_by: fable5:lead + gpt56sol:colead, 2026-08-20.**

**SC-518 — request closure requires the full mirror match.** Bucket 1 — (slice-1 Q6
seat ruling, reversing the lead's scope confirmation): an unanswered request closes
only on same ref AND reply.actor = request.target AND reply.target = request.actor;
routed identities (slot+session) compare when both sides carry them, display
identities when neither does, and MIXED identity matches nothing — the loud
false-pending direction is safer than silent false-closure by a reply sent to someone
else. Authority: events.md:108-117 (normative dependency of SC-017g) + joint ruling.
Empirical: pending (builder P1 implementation + C-cluster). Conflict: none.
**classified_by: fable5:lead + gpt56sol:colead, 2026-08-20.**

**SC-519 — absent and zero-byte event logs are quiet, not degraded.** Bucket 2 —
(slice-1 Q7b seat ruling): a fresh session may have no events file until first write
and readers tolerate ENOENT (bridge-protocol.md:90-92); missing and empty are both
quiet empty streams. An EXISTING-but-unreadable file, malformed complete records, or
other I/O failure degrades (SC-509b). Authority: bridge-protocol.md:90-92 + joint
ruling. Empirical: pending. Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-520 — a skipped malformed record is observable.** Bucket 1 — (slice-1 Q7a): skip
the malformed COMPLETE line, continue, retain generation+offset+reason internally, and
mark the session degraded in the public JSON (a buffered unterminated tail is not
malformed — SC-975b). Authority: joint ruling + SC-975b. Empirical: pending.
Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-521a — cross-dimension filter combinations intersect literally.** Bucket 2 —
(slice-1 Q7c; split from SC-521 at slice-1c for row grain): `--stopped --needs-attn`
and `--stopped --active` select nothing (each attention/activity row reads "running
sessions only" literally); `--all` with either keeps only matching running sessions;
no invented usage error. Authority: commands.md filter rows + joint ruling.
Empirical: pending (Batch C A2). Conflict: none. **classified_by: both seats,
2026-08-20.**

**SC-521b — same-dimension scope flags are alternatives: last distinct selector
wins.** Bucket 2 — (slice-1c, seat ruling on reviewer3 I7):
`--running`/`--stopped`/`--all` are ALTERNATIVE modes per commands.md:81-87, not
independent predicates; the last distinct selector wins and a repeated flag is
idempotent. The lead's set-intersection alternative was rejected as inventing
semantics the docs do not state and failing silently on `--stopped --running`.
Authority: commands.md:81-87 + joint ruling. Empirical: observed(ae@72c7293:4077-4089
— case loop reassigns show_running/show_stopped per selector). Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-522 — the unanswered threshold is strictly past.** Bucket 2 — (slice-1 Q7e): age
must EXCEED the threshold; equality is not past it. Authority: commands.md:60-76
("past the threshold") + joint ruling. Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-523a — the unanswered threshold default is 1800s.** Bucket 2 — (slice-1 Q7f):
`AE_ATTN_REQUEST_SECS` defaults to 30 minutes, a NORMATIVE value; SC-1410j owns its
unset/override/malformed ENV behavior separately; implementations may take it as a
caller parameter. Authority: commands.md:71. Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-523b — the activity window default is 300s.** Bucket 2 — (slice-1 Q7f):
`AE_LIST_ACTIVE_SECS` defaults to ~5 minutes, a NORMATIVE value; SC-1410k owns its ENV
behavior separately; caller-parameter transport permitted. Authority: commands.md:87.
Empirical: pending. Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-524 — a future timestamp counts as active.** Bucket 1 — (slice-1 Q7f seat
ruling): clock skew fails toward the loud false-positive (a session shown active)
rather than silently hiding a live session. Authority: joint ruling (loud-direction
doctrine). Empirical: pending. Conflict: none. **classified_by: both seats,
2026-08-20.**

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

**SC-510c — `ref` polysemy follows the COMPLETE action table.** Bucket 2 — request id
for ask/review/reply, topic for memo, captured session id for recover, DECLARED STATE
for `state` events, and for other actions USUALLY absent — never categorically absent
(amended per slice-1 Q3: a contract transcription defect found during implementation —
the original row dropped the doc's own hedge and the state entry; conflict=none, no
bucket-3 fiction). Authority: events.md:62-68 + 86-106. Empirical: pending.
Conflict: none. **classified_by: REOPENED by the amendment and RE-MARKED — fable5:lead
+ gpt56sol:colead, 2026-08-20.**

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

**SC-603 — layout application semantics.** `authority=code-observation` — how
`layout =` maps to pane arrangement, and its failure mode; probe + seat ruling at the
sweep. UNCLASSIFIED pending closure.

**SC-604 — window naming semantics.** `authority=code-observation` — session window
titles and spawned-agent window naming; probe + seat ruling. UNCLASSIFIED pending
closure.

(Monitor-window and status-bar behavior rows live in S10: SC-922/923/924 and M-03.)

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
  Authority: S8 joint adapter ruling (SC-704 frame).
— context rides an append/positional/config surface per tool; a replace-style flag is
forbidden (the grok `--system-prompt-override` trap). Empirical: matrix rows.
Conflict: none.

**SC-704b — capture binds only a positively-owned signal.** Bucket 3 — SHOULD: an
  Authority: S8 joint adapter ruling + DR-005 ownership rule.
identity is captured only from a signal this agent slot positively owns, never ambient
context (the confidentiality rule). IS at 72c7293: opencode capture is cwd/time
heuristic — two agents in one dir are indistinguishable, max-updated wins.
Conflict: fix-known-defect(#56, intended per DR-005). Empirical: matrix + capture
exhibits.

**SC-704c — resume requires exact ownership.** Bucket 4 — **DR-005**: a resume targets
  Authority: DR-005 (both seats).
only an exactly-owned identity; with none stored, the command REFUSES with recovery
guidance — it never guesses and never silently starts fresh over the only stored
provider UUID (#50). Empirical: matrix resume rows. Conflict: DR-005.

**SC-704d — heuristic fallbacks retire.** Bucket 4 — **DR-005**: `--continue` (CWD
  Authority: DR-005 (both seats).
guess) and `--resume latest` (recency guess) are cross-wire risks and do not survive;
fresh launch remains an explicit, distinct operation that never claims to be a resume.
Empirical: matrix fallback rows. Conflict: DR-005.

**SC-704e — rerun truth is explicit.** Bucket 1 — re-running a launch script either
  Authority: S8 joint adapter ruling (SC-704 frame) + SC-811 pins context.
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

**SC-707 — an unsupported launch command receives no context injection.**
`authority=code-observation` — inject_ae_context and initial_prompt_for_cmd leave
unrecognized agent commands as passthrough (ae@72c7293:1539,1558): no system/
developer/context material and no initial prompt are delivered; the agent starts
bare with only the on-disk workspace. Surfaced by the B0 transport census
(b0-census.md §B gap list), cut OUT of SC-1208's scope by seat ruling — SC-1208
guarantees the five modeled tools only. Bucket + intended Rust behavior (refuse?
warn? document?) are a seat ruling. Empirical: observed(b0-census.md §B;
ae:1539,1558 rechecked). Conflict: pending seat closure (UNCLASSIFIED).

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

**SC-835a — stop addresses the recorded server and the exact session id.** Bucket 1 —
never the ambient server, never a name (`tmux kill-session -t` prefix-matches: a
name-based stop could kill a sibling). Authority: commands.md:297-305. Empirical:
census-2 stop section. Conflict: none.

**SC-835b — stop reports stopped only after verifying the session is gone.** Bucket 1.
Authority: commands.md:297-300. Empirical: pending. Conflict: none.

**SC-835c — an unverifiable kill fails loudly and changes nothing.** Bucket 1 — the
recorded server unreachable means no success report and no state change. Authority:
commands.md:297-302. Empirical: pending. Conflict: none.

**SC-835d — stop never deletes anything.** Bucket 1 — ae state, working tree, and
provider conversation files are preserved either way. Authority: commands.md:301-302.
Empirical: pending. Conflict: none.

**SC-835e — self-stop confirms with the recoverability warning.** Bucket 1 — the
confirmation states the guarantee honestly: recoverability from the provider
checkpoint, not mid-write atomicity. Authority: commands.md:311-321. Empirical:
pending. Conflict: none.

**SC-835g — self-stop executes via a short-lived out-of-pane supervisor.** Bucket 1 —
the process inside the session cannot kill it and still verify/record. Authority:
commands.md:311-325. Empirical: census-2. Conflict: none.

**SC-835h — the self-stop outcome is a durable `stop-result` event.** Bucket 1 — the
pane dies with the session, so the result is written where it survives. Authority:
commands.md:325-330. Empirical: census-2. Conflict: none.

**SC-835f — `-y` skips the self-stop confirmation.** Bucket 2 — required when no
terminal can ask. Authority: commands.md:333-334. Empirical: pending. Conflict: none.

**SC-838a — end history policy precedence is CLI > session config > keep.** Bucket 2 —
`--purge-history`/`--keep-history` force this run; `[workspace] purge_agent_history`
is the session default; unset means KEEP. Authority: commands.md:459-465. Empirical:
pending. Conflict: none.

**SC-838b — `end all` resolves and lists both decisions per session.** Bucket 2 — one
line each: which archive path (or none, and which archive is deleted) and whether
conversation files are kept — the purge default is per-session config, so no single
sentence covers all. Authority: commands.md:453-458. Empirical: pending.
Conflict: none.

**SC-839a — `--self` waives exactly one check.** Bucket 1 — the controlling-terminal
proof (C5) and NOTHING else; server and pane identity are still proven. Authority:
commands.md:417-421. Empirical: pending. Conflict: none.

**SC-839b — `--pane` accepts only a shape-checked tmux pane id.** Bucket 1 — `%N`
form, tmux-generated; nothing attacker-influenced enters the command ($TMUX_PANE in a
run-shell child names some other pane — measured). Authority: commands.md:423-430.
Empirical: measured exhibit in the doc. Conflict: none.

**SC-839c — the stop identity checks are C1–C5.** Bucket 1 — inside tmux with a pane
id; the server answers for itself; it is the session's recorded server; the pane is in
that session; the controlling terminal is that pane's (the one C5 `--self` waives).
Authority: commands.md:431-434. Empirical: pending. Conflict: none.

**SC-839d — a stop refusal names the failed check.** Bucket 1 — e.g. `refusing: C4 —
pane %0 is in 'alpha', not 'beta'`; the named fact says what to fix. Authority:
commands.md:430-434. Empirical: pending. Conflict: none.

**SC-839e — the no-name form keeps tmux-controlled text out of shell programs.** Bucket
1 — ae resolves the target itself; no tmux-expanded text (session names with quotes or
`$(…)`) enters a shell string. Authority: commands.md:408-414. Empirical: pending.
Conflict: none.

**SC-836 — `purge_agent_history` refuses compact unless `--keep-history`.** Bucket 1 —
the config contradicts an operation whose purpose is keeping the record; the override
is explicit. Authority: commands.md:651-652. Empirical: pending. Conflict: none.

**SC-837 — `compact -f` proceeds without asking.** Bucket 2 — the explicit
skip-confirmation surface (distinct from end's `-f` freeze semantics, SC-820b).
Authority: commands.md:698. Empirical: pending. Conflict: none.

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
  Empirical: census-2 watchdog/telegram sections + census-3 (the measured topologies).
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
  Authority: watchdog.md:5-10 + commands.md:177-185. Empirical: pending-probe(watchdog default/disable parsing). Conflict: none.
- **SC-903** b2 — per-session enable state persists across resume.
  Authority: watchdog.md:7-10. Empirical: docs/migration/evidence/locks-census-2.md § Watchdog controls and daemon loop — Write sequence/Crash residue. Conflict: none.
- **SC-904** b2 — start is idempotent, confirms enabled state; start/stop/status + loop
  Authority: commands.md:177-185 + watchdog.md:7-10. Empirical: docs/migration/evidence/locks-census-2.md § Watchdog controls and daemon loop — Write sequence. Conflict: none.
  surface survive DR-002.
- **SC-905** b1 — per-pane cycle is first-match-wins; no later branch after a
  Authority: watchdog.md:48-70. Empirical: pending-probe(per-pane first-match classification). Conflict: none.
  classification.
- **SC-906** b1 — dead = shell foreground with no agent descendant; alert once, then
  Authority: watchdog.md:70-73. Empirical: pending-probe(dead-pane definition and alert suppression). Conflict: none.
  ignore until state changes.
- **SC-907** b1 — a quiet declaration applies only while it is the LATEST relevant
  Authority: watchdog.md:73,85-88. Empirical: pending-probe(latest relevant quiet-state event). Conflict: none.
  event; any newer relevant event invalidates it.
- **SC-908** b1 — `done` is event-only: pane churn never revives it; resumption needs a
  Authority: watchdog.md:73,87-90. Empirical: pending-probe(event-only done resumption). Conflict: none.
  newer ae event.
- **SC-909** b1 — waiting-user/blocked arm a post-echo pane baseline, hold while
  Authority: watchdog.md:91-98. Empirical: pending-probe(waiting-user/blocked post-echo baseline). Conflict: none.
  unchanged, yield on later pane change.
- **SC-910** b1 — active pane change suppresses the nudge and resets the count.
  Authority: watchdog.md:74-75. Empirical: pending-probe(active pane-change nudge reset). Conflict: none.
- **SC-911** b1 — recently-visible pane change suppresses within the stale window.
  Authority: watchdog.md:75-76. Empirical: pending-probe(recent visible pane-change stale window). Conflict: none.
- **SC-912** b1 — recent ae activity suppresses within the stale window.
  Authority: watchdog.md:76-77. Empirical: pending-probe(recent ae-activity stale window). Conflict: none.
- **SC-913** b3 fix-known-defect(#45) — every daemon nudge uses the ONE verified
  Authority: watchdog.md:77-78 + joint S10 ruling grounded in closed #44 semantics. Empirical: docs/migration/evidence/locks-census-3-aewatch.md § I7 — delivery-guard asymmetry (#45). Conflict: fix-known-defect(#45).
  delivery primitive (target lock, busy/human/dead guards, durable failure evidence,
  verified submit); only rc0 delivery spends MAX_NUDGES. IS: aewatch pastes ungated
  (census-3 I7).
- **SC-914** b1 — after MAX_NUDGES confirmed deliveries: one alert + visible banner,
  Authority: watchdog.md:77-78,136. Empirical: pending-probe(MAX_NUDGES alert/banner and state-change reset). Conflict: none.
  then silent waiting until state changes.
- **SC-915** b1 — first throttle cycle suppresses the nudge and resets the stale budget.
  Authority: watchdog.md:104-121. Empirical: pending-probe(first throttle-cycle suppression/reset). Conflict: none.
- **SC-916** b1 — first cycle of a throttle streak emits exactly one `throttled` event.
  Authority: watchdog.md:116-121. Empirical: pending-probe(first throttle-streak event). Conflict: none.
- **SC-917** b1 — continuous throttle alerts once at the threshold, never re-alerts in
  Authority: watchdog.md:116-136. Empirical: pending-probe(throttle threshold single alert). Conflict: none.
  the same streak.
- **SC-918** b1 — throttle disappearance emits `throttle-cleared` and resets the streak.
  Authority: watchdog.md:116-136. Empirical: pending-probe(throttle-cleared transition). Conflict: none.
- **SC-919** b1 — a registered missing pane alerts once per disappearance.
  Authority: watchdog.md:80-83. Empirical: pending-probe(missing-pane disappearance alert). Conflict: none.
- **SC-920** b3 fix-known-defect(#51) — human-origin evidence inside quiet stabilization
  Authority: UNRESOLVED(memo s10-watchdog gives no normative authority citation). Empirical: pending-probe(human-origin evidence versus agent churn). Conflict: fix-known-defect(#51).
  must yield; a submitted human reply is never absorbed as agent churn. IS: equal pane
  hashes cannot distinguish them.
- **SC-921** b3 fix-known-defect(#73) — monitor panes are never agents and never enter
  Authority: UNRESOLVED(memo s10-watchdog gives no normative authority citation). Empirical: pending-probe(internal monitor panes versus user-agent roster). Conflict: fix-known-defect(#73).
  the roster. IS: regenerate_manifest lists `_watchdog`/`_events`.
- **SC-922** b2 — every session keeps `ae-monitor` with an always-present `_events`
  Authority: monitor.md:1-40. Empirical: docs/migration/evidence/locks-census-2.md § Watchdog controls and daemon loop — Write sequence. Conflict: none.
  view, independent of watchdog enablement, across resume.
- **SC-923** b1 — monitor panes are read-only/input-disabled.
  Authority: monitor.md:5-12. Empirical: pending-probe(monitor pane input-disabled/read-only behavior). Conflict: none.
- **SC-924** b2 — watchdog stop never removes the `_events` inspection surface (DR-002
  Authority: monitor.md:34-40,100-109; DR-002 retires only the _watchdog pane. Empirical: docs/migration/evidence/locks-census-2.md § Watchdog controls and daemon loop — stop Write sequence. Conflict: none.
  retires only the `_watchdog` pane).
- **SC-925** b1 — a dead agent is never auto-restarted by the watchdog.
  Authority: watchdog.md:138-143. Empirical: pending-probe(dead-agent no-auto-restart). Conflict: none.
- **SC-926** b3 fix-known-defect(#88-A) — control success only when durable intent and
  Authority: UNRESOLVED(memo supplies ownership D26a/census-2 evidence but no normative authority citation). Empirical: docs/migration/evidence/locks-census-2.md § Watchdog control partial success. Conflict: fix-known-defect(#88-A).
  runtime converge; typed partial failure otherwise. IS: meta-write failure ignored
  after tmux mutation.
- **SC-927** b3 fix-known-defect(#88-B) — status is read-only; cleanup belongs to an
  Authority: UNRESOLVED(memo supplies ownership D26b/census-2 evidence but no normative authority citation). Empirical: docs/migration/evidence/locks-census-2.md § Watchdog control partial success. Conflict: fix-known-defect(#88-B).
  explicit reconcile path. IS: status deletes stale pidfiles.
- **SC-928** b3 fix-known-defect(#88-C) — an event-append error is surfaced and
  Authority: UNRESOLVED(memo supplies census-3 I2 evidence but no normative authority citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § I2 — _locked_append failure directions are two, not one. Conflict: fix-known-defect(#88-C).
  contained to its operation; it never stops the combined daemon or spends nudge state.
  IS: census-3 I2 crash/backoff.
- **SC-929** b4 DR-002 — the restart outcome (gate ruling, testable): after a
  Authority: UNRESOLVED(no SC-929 authority citation in requested S10 memos). Empirical: pending-probe(doctor --refresh serving-version ordering). Conflict: DR-002.
  successful `doctor --refresh` the serving daemon runs the INSTALLED version before
  the command returns; a failed refresh returns nonzero and leaves the previous daemon
  serving; the restart emits durable before/after/failure facts. Implementation may
  re-exec. (The bash keeps-loaded-body behavior retires with the topology.)

Steward (authority: commands.md/telegram.md @72c7293, cited per memo):
- **SC-930** b2 — bare `ae steward` ensures the detached steward, never attaches.
  Authority: commands.md:219-249. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Locks and acquisition order. Conflict: none.
- **SC-931** b2 — `--attach` is the explicit attach/switch surface.
  Authority: commands.md:233-249. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Locks and acquisition order. Conflict: none.
- **SC-932** b1 — `--init` scaffolds and NEVER overwrites operator files.
  Authority: commands.md:233-238,251-256. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Write sequence/Crash residue. Conflict: none.
- **SC-933** b1 — steward launch isolates its config and neutralizes project-local
  Authority: commands.md:240-246. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Locks and acquisition order. Conflict: none.
  config.
- **SC-934** b1 — steward authority is monitor/relay/suggest ONLY: never ends, stops,
  Authority: commands.md:221-231. Empirical: pending-probe(steward authority boundary). Conflict: none.
  edits, or dispatches into another session without human authorization.
- **SC-935** b1 — only the steward main agent gets sweep cadence, no stale escalation;
  Authority: commands.md:187-195. Empirical: pending-probe(steward-only sweep cadence and worker watchdog). Conflict: none.
  its workers keep normal watchdog behavior.
- **SC-936** b1 — a sweep nudge is delivery-checked; refusal is logged, never counted
  Authority: commands.md:197-200. Empirical: pending-probe(sweep delivery refusal accounting). Conflict: none.
  delivered.
- **SC-937** b1 — undelivered sweeps retry on the short cadence.
  Authority: commands.md:197-203. Empirical: pending-probe(short retry cadence after undelivered sweep). Conflict: none.
- **SC-938** b1 — after retry max: normal cadence + one unreachable alert, cleared on a
  Authority: commands.md:201-203. Empirical: pending-probe(retry-max fallback and alert clear). Conflict: none.
  landed delivery.
- **SC-939a** b1 — sweep delivery is at-least-once: event-write failure after paste may
  Authority: commands.md:203-206. Empirical: pending-probe(at-least-once sweep event write). Conflict: none.
  duplicate, never silently drop.
- **SC-939b** b1 — steward liveness = dead-pane checks AND a live-but-not-sweeping
  Authority: commands.md:208-215. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Write sequence (aemonitor/heartbeat read). Conflict: none.
  heartbeat (~2x cadence); stale alerts once, recovery clears.
- **SC-939c** b2 — sweep nudges are outside the default Telegram include.
  Authority: commands.md:215-217 + telegram.md:138-140. Empirical: pending-probe(steward sweep Telegram include exclusion). Conflict: none.
- **SC-939d** b2 — plain Telegram text defaults to the running steward absent a sticky
  Authority: telegram.md:69-77,110-116. Empirical: pending-probe(plain Telegram text steward default routing). Conflict: none.
  override; no steward yields start guidance.
- **SC-939e** b2 — `/use` overrides that default; `/use clear` restores steward routing.
  Authority: telegram.md:73-77,110-116. Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Audited addenda — I3 shared Telegram store caller semantics. Conflict: none.
- **SC-939f** b2 — deprecated `hub` stays accepted; canonical name is steward (#52
  Authority: commands.md:264-272 + #52 policy ruling. Empirical: docs/migration/evidence/locks-census-2.md § Steward — Locks and acquisition order (steward/hub trampoline). Conflict: none.
  policy ruling).

Telegram (authority: telegram.md @72c7293, cited per memo):
- **SC-940** b1 — jq/curl absence refuses ONLY the bridge; core commands unimpaired.
  Authority: telegram.md@72c7293:19-24. Empirical: pending-probe(feature-only jq/curl dependency refusal). Conflict: none.
- **SC-941** b2 — outbound include allow-list default; exclude applies after include.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 47-53). Empirical: pending-probe(outbound include/exclude precedence). Conflict: none.
- **SC-942** b2 — `chat` action gives the two-way loop; include-without-chat disables
  Authority: UNRESOLVED(memo citation is only the unqualified line range 9-15). Empirical: pending-probe(chat action and include-without-chat status). Conflict: none.
  it and status warns.
- **SC-943** b1 — inbound exists only with nonempty `allowed_user_ids`; empty =
  Authority: UNRESOLVED(memo citation is only the unqualified line range 51,55-57). Empirical: pending-probe(allowed_user_ids empty outbound-only mode). Conflict: none.
  outbound-only.
- **SC-944a** b1 — inbound trust predicate: numeric allowlisted `from.id`; failure
  Authority: UNRESOLVED(memo citation is only the unqualified line range 59-65). Empirical: pending-probe(numeric allowlisted from.id trust predicate). Conflict: none.
  silently drops.
- **SC-944b** b1 — inbound trust predicate: exact configured `chat.id`; failure
  Authority: UNRESOLVED(memo citation is only the unqualified line range 59-65). Empirical: pending-probe(exact configured chat.id trust predicate). Conflict: none.
  silently drops.
- **SC-944c** b1 — inbound trust predicate: private chat only; failure silently drops.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 59-65). Empirical: pending-probe(private-chat trust predicate). Conflict: none.
- **SC-945** b2 — routing precedence: command > reply > compact > override/steward.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 67-77). Empirical: pending-probe(inbound routing precedence). Conflict: none.
- **SC-946** b1 — every inbound route passes the same session/agent revalidation.
  Authority: UNRESOLVED(memo citation is only the unqualified line references 69,77). Empirical: pending-probe(shared session/agent revalidation). Conflict: none.
- **SC-947** b1 — only running sessions are addressable.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 91). Empirical: pending-probe(running-session addressability). Conflict: none.
- **SC-948** b2 — session resolves by exact name or unique session_id prefix.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 91). Empirical: pending-probe(exact-name/unique-session-prefix resolution). Conflict: none.
- **SC-949** b1 — agents resolve only within that session; pane-id, cross-session, and
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 92). Empirical: pending-probe(session-local canonical agent resolution). Conflict: none.
  external-actor escapes rejected.
- **SC-950** b2 — sender identity is `telegram:<id>`; replies route back outbound.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 93). Empirical: pending-probe(Telegram sender identity and reply route). Conflict: none.
- **SC-951** b1 — inbound update offset persists BEFORE dispatch: at-most-once side
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 97). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Inbound polling. Conflict: none.
  effects.
- **SC-952** b2 — command-menu registration is best-effort (log and ignore).
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 95). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — tg_set_commands. Conflict: none.
- **SC-953** b2 — start is idempotent.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 155). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Locks and acquisition order. Conflict: none.
- **SC-954** b2 — stop succeeds when already stopped.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 155). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Locks and acquisition order. Conflict: none.
- **SC-955** b2 — status reports persisted intent, runtime, deps, token validity.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 148-155). Empirical: pending-probe(status persisted intent/runtime/deps/token validity). Conflict: none.
- **SC-956** b1 — autostart failure warns one line and never blocks session launch.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 161-167). Empirical: pending-probe(autostart failure warning and launch non-blocking). Conflict: none.
- **SC-957** b1 — supervision honors durable disabled state; can never revive after an
  Authority: UNRESOLVED(memo citation is only the unqualified line range 163-171). Empirical: pending-probe(disabled-state supervision). Conflict: none.
  explicit stop (DR-002 changes topology, not this).
- **SC-958** b4 DR-003 — outbound delivery is at-least-once: cursor persistence is part
  Authority: UNRESOLVED(memo gives line ranges 9-12,167-169,181-185 and census3 I8 but no normative source citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § I3 — shared Telegram store caller semantics. Conflict: DR-003.
  of committed progress, save failure is LOUD and retry-safe, duplicates possible only
  in the crash window, event id rides the outbound text/log for dedupe. (IS: save
  failure silently ignored — #86-D evidence.)
- **SC-959** b2 — a first-seen session starts at EOF; no history flood.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 167-169). Empirical: pending-probe(first-seen session EOF initialization). Conflict: none.
- **SC-960** b1 — the persisted getUpdates offset prevents inbound redispatch on
  Authority: UNRESOLVED(memo gives unqualified line references 97,169). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Inbound polling. Conflict: none.
  restart.
- **SC-961** b1 — token file is owner-only 0600; wrong perms refuse start with a
  Authority: UNRESOLVED(memo gives only unqualified line references 35,210,216-220). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Setup write sequence. Conflict: none.
  corrective diagnostic.
- **SC-962** b1 — the token never enters argv; logs redact it.
  Authority: UNRESOLVED(memo citation is only the unqualified line reference 212). Empirical: pending-probe(token argv/log redaction). Conflict: none.
- **SC-963** b3 fix-known-defect(#83) — explicit start preserves exactly-one-sender:
  Authority: UNRESOLVED(memo gives issue-evidence line range 181-198 without a frozen normative source citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Audited addenda — #83 explicit-start bypass. Conflict: fix-known-defect(#83).
  refuse or complete a verified takeover, never warn-and-double-send.
- **SC-964** b3 fix-known-defect(#84) — takeover is serialized and proves every
  Authority: UNRESOLVED(memo gives issue-evidence range 181-187 and DR-002 condition without a frozen normative source citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Telegram and aewatch tmux sessions / Audited addenda. Conflict: fix-known-defect(#84).
  predecessor absent across the COMPLETE scope before the first send.
- **SC-965** b3 fix-known-defect(#85) — destructive tmux targets resolve exact
  Authority: UNRESOLVED(memo names issue #85 without a frozen normative source citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Telegram and aewatch tmux sessions — destructive tmux targets. Conflict: fix-known-defect(#85).
  identity, never prefix.
- **SC-966** b3 fix-known-defect(#86-E) — `/use clear` succeeds only after durable
  Authority: UNRESOLVED(memo s10-telegram gives no normative authority citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § I3 — shared Telegram store caller semantics. Conflict: fix-known-defect(#86-E).
  removal.
- **SC-967** b3 fix-known-defect(#87) — one effective-config authority for every
  Authority: UNRESOLVED(memo s10-telegram gives no normative authority citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § config and token file. Conflict: fix-known-defect(#87).
  daemon mode, `CONFIG_FILE`/`AE_LOCAL_CONFIG` included.
- **SC-968** b3 fix-known-defect(#88-G) — lifecycle ownership acquired before any
  Authority: UNRESOLVED(memo s10-telegram gives no normative authority citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § I6 — up is outside the daemon singleton. Conflict: fix-known-defect(#88-G).
  probe/kill/create mutation.
- **SC-969** b3 fix-known-defect(#87-H) — setup publishes token/config with atomic
  Authority: UNRESOLVED(memo s10-telegram gives no normative authority citation). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § config and token file; docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Setup write sequence. Conflict: fix-known-defect(#87-H).
  visibility; no reader observes empty/partial canonical state.
- **SC-970** b2 — setup persists enabled, token_file, chat_id, seeded allowlist (byte
  Authority: UNRESOLVED(memo citation is only the unqualified line range 27-53). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Setup write sequence. Conflict: none.
  formats: S5/S15).
- **SC-971** b2 — start persists `enabled=true`; stop persists `enabled=false`.
  Authority: UNRESOLVED(memo citation is only the unqualified line range 148-165). Empirical: docs/migration/evidence/locks-census-2.md § Telegram setup/start/stop and daemon loop — Start/stop write sequence. Conflict: none.

- **SC-980 — successor alert events carry a typed reason.** Bucket 1 — (slice-1 Q2
  seat ruling): alert events gain an ADDITIVE typed reason key sufficient to
  discriminate dead | stale | throttled; free-text `summary` is never a discriminator.
  Additive keys are legal per the events evolution rule. The INCUMBENT action/summary
  byte shapes are T-WD probe material for the legacy adapter — empirical only, never
  SHOULD. Authority: commands.md:60-76 + joint seat ruling 2026-08-20. Empirical: T-WD
  pending. Conflict: none. **classified_by: both seats, 2026-08-20.**

Bridge protocol (authority: bridge-protocol.md @72c7293; lead splits per gate):
- **SC-972** b2 — external actors are `<platform>:<id>`, opaque past the allowlisted
  Authority: UNRESOLVED(no SC-972 authority citation in requested S10 memos). Empirical: pending-probe(external-actor target grammar). Conflict: none.
  prefix.
- **SC-973a** b1 — event-only sinks (`telegram:`/`discord:`/`ae:compact:`) emit without
  Authority: UNRESOLVED(no SC-973a authority citation in requested S10 memos). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Shared event log. Conflict: none.
  tmux resolution and preserve the literal target.
- **SC-973b** b1 — an unknown non-allowlisted external target fails LOUDLY.
  Authority: UNRESOLVED(no SC-973b authority citation in requested S10 memos). Empirical: pending-probe(unknown external target failure). Conflict: none.
- **SC-974a** b2 — `AE_SENDER_OVERRIDE` sets the actor for send/ask/review.
  Authority: UNRESOLVED(no SC-974a authority citation in requested S10 memos). Empirical: pending-probe(AE_SENDER_OVERRIDE actor selection). Conflict: none.
- **SC-974b** b2 — reply caller identity comes from `--as`, not the override.
  Authority: UNRESOLVED(no SC-974b authority citation in requested S10 memos). Empirical: pending-probe(reply --as identity precedence). Conflict: none.
- **SC-975a** b1 — bridge readers tolerate a missing event file.
  Authority: UNRESOLVED(no SC-975a authority citation in requested S10 memos). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Shared event log — Reads and writes. Conflict: none.
- **SC-975b** b1 — malformed/unterminated trailing data is buffered until a complete
  Authority: UNRESOLVED(no SC-975b authority citation in requested S10 memos). Empirical: pending-probe(trailing event record buffering). Conflict: none.
  newline record exists.
- **SC-976a** b4 DR-001 — the reader cursor is generation-aware (generation + offset
  Authority: UNRESOLVED(no SC-976a authority citation in requested S10 memos). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § B3 — append-only contract vs resume trim; § I1 — event reader stat/open generation race. Conflict: DR-001.
  replaces the (session_id,inode) key).
- **SC-976b** b2 — event logs are tailed/back-scanned bounded, never whole-loaded.
  Authority: UNRESOLVED(no SC-976b authority citation in requested S10 memos). Empirical: docs/migration/evidence/locks-census-3-aewatch.md § Shared event log — Reads and writes. Conflict: none.
- **SC-977** b1 — bridges bind the stable session_id across resume/rename/transfer.
  Authority: UNRESOLVED(no SC-977 authority citation in requested S10 memos). Empirical: pending-probe(stable session_id across resume/rename/transfer). Conflict: none.
- **SC-978a** b2 — bridges ignore unknown fields/actions.
  Authority: UNRESOLVED(no SC-978a authority citation in requested S10 memos). Empirical: pending-probe(unknown bridge fields/actions). Conflict: none.
- **SC-978b** b2 — renames/removals/semantic changes of existing fields are BREAKING.
  Authority: UNRESOLVED(no SC-978b authority citation in requested S10 memos). Empirical: pending-probe(bridge field rename/removal compatibility). Conflict: none.
- **SC-979a** b1 — telegram sends use plain-text paths (no parse-mode injection).
  Authority: UNRESOLVED(no SC-979a authority citation in requested S10 memos). Empirical: pending-probe(plain-text Telegram send path). Conflict: none.
- **SC-979b** b1 — jq programs stay fixed strings; user data enters via stdin only.
  Authority: UNRESOLVED(no SC-979b authority citation in requested S10 memos). Empirical: pending-probe(fixed jq programs and stdin-only user data). Conflict: none.

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

**SC-1000 — installation is one command from either entry path.** Bucket 2 —
outcome-level (gate: the clone+symlink mechanism is the bash era's, empirical): the
one-liner and the local-clone path both yield a working `ae` on PATH; the P5 binary
install preserves the same outcome with its mechanism ruled via #57/SC-1006.
Authority: install.md. Empirical: pending. Conflict: none.

**SC-1001 — an upgrade preserves existing sessions and refreshes/migrates phase-owned
assets.** Bucket 2 — outcome-level (gate correction: helper REGENERATION is the
bash-era asset mechanism and retires at P2 — it is not frozen as a P5 promise):
sessions keep working across an upgrade, and whatever assets the current phase owns
are refreshed or migrated on next start/resume (`doctor --refresh [name]` forces it).
Authority: install.md upgrading (outcome). Empirical: pending. Conflict: none.

**SC-1002 — doctor reports environment health as a fixed OK/WARN/FAIL checklist.**
Bucket 2 — dependency presence, config, registered agent executables, sessions dir;
the bash-version item is scoped to the surviving glue (SC-1105); exit contract is
SC-514. Authority: install.md verify + commands.md:168. Empirical: pending.
Conflict: none.

**SC-1005 — installer failure modes.** `authority=code-observation` — partial clone,
missing PATH dir, re-run over an existing install; probe + seat ruling. UNCLASSIFIED
pending closure.

**SC-1006 — the installed artifact is versioned and atomic.** Bucket 3 — SHOULD: what
  Authority: install.md (outcome) + the #57 finding record.
runs as `ae` is a deliberately installed version, atomically switched. IS at 72c7293:
the installed `ae` is a symlink into the live dev checkout — work sessions run
whatever the working tree holds (#57). Conflict: fix-known-defect(#57, intended: the
P5 installer flip ships an atomic, versioned binary install). Empirical: #57 +
install.md.

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
  Authority: AGENTS.md core-dependency rule (bash+tmux+git only; optional features degrade).
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
  Authority: S12 seat ruling 2026-08-20 (semantic limit-or-loud).
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
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
`^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`, because a name reaches a privileged sink: it is
interpolated into that agent's own system prompt (the identity sentence).
Empirical: unit pins @72c7293. Conflict: none.

**SC-1201 — the spawn boundary treats a peer name as hostile.** Bucket 1 — a name
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
arriving via `spawn` is validated fatally: violation refuses the spawn (the #59 exploit
was a legal-looking name carrying prose into the identity sentence).
Empirical: pending. Conflict: none.

**SC-1202 — the operator roster boundary fails the launch before product mutation.**
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
Bucket 3 (gate finding: known IS/SHOULD conflict) — SHOULD: a roster violation is
rejected before any tmux, session-state, or config mutation (diagnostics permitted).
IS at 72c7293: M2 writes the default config BEFORE dispatcher/roster validation, so an
invalid fresh roster has a filesystem side effect before refusal.
Conflict: fix-known-defect(#61, intended: read/validation paths never bootstrap).
Empirical: M2 census + ae:344-352.

**SC-1203 — enforcement follows provenance, not the variable.** Bucket 1 — FRESH input
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
(config, CLI, spawn) is fatal on violation; RESTORED input (saved meta, compact's
frozen roster) is left to the interpolation guard — refusing restored input would make
a pre-grammar session unresumable and kill a compact child whose source is already
archived. Empirical: pending. Conflict: none.

**SC-1204 — the interpolation boundary re-validates and fails quiet.** Bucket 1 —
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
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
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
`workers = _foo` (alias as its own name) fails the launch with the grammar; internal
`_`-prefixed helpers stay out of the agent namespace. Empirical: pending.
Conflict: none.

**SC-1207a — prompt identity facets are unambiguous.** Bucket 1 — neither the alias nor
  Authority: #59 ruling.
the name may contain the facet separator; identity parses one way only. Empirical:
pending. Conflict: none.

**SC-1207b — meta serializes agents as `alias:name:provider-session-id`.** Bucket 2 —
  Authority: #59 ruling + meta format (S5).
exact on-disk form (cross-link: S5 formats family). Empirical: pending. Conflict: none.

**SC-1208 — untrusted pane bytes and peer message-body prose are never spliced into
instruction material.** Bucket 1 — (precised, B0-census reopening 2026-08-20):
transport delivers peer text through the model's USER-INPUT surface; ae never places
pane content or peer MESSAGE-BODY prose into system/developer instruction material;
delivered text retains peer provenance (authority/envelope semantics are SC-1209's
row — cross-link, not this row's claim: B0 evidence closes only the structural
boundary and could never fail against a model-compliance clause). The invariant
holds at the TYPE boundary, not the actor boundary:
schema-typed, allowlist-validated identity facts MAY enter instruction material — a
peer-supplied spawn name, after `_validate_agent_name`, is intentionally interpolated
into the fixed identity slot (its own AGENTS authority, #59). The B0 probe carries
both controls: a hostile free-text sentinel absent from every instruction channel,
AND a hostile-looking-but-grammar-valid spawn name present ONLY in the fixed identity
slot. Probe evidence proves ae's transport separation only — never a claim that a
vendor model obeys instruction hierarchy. Authority: AGENTS.md interpreted-sinks row
+ agent-name allowlist section (ruling). Empirical: pending (B0). Conflict: none.

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

**SC-1300 — concurrent event appends yield complete, non-interleaved, ordered
records.** Bucket 1 — promise-level (gate: the adjacent lock file is the bash
mechanism, empirical; DR-001's one-generation protocol supersedes the mechanism, not
the promise). Failure SEMANTICS are per-operation rows (M1). Authority: events.md +
bridge-protocol.md. Empirical: census-1 M1. Conflict: none (DR-001 affected —
mechanism only).

**SC-1301 — session meta is written through one fail-closed writer.** Bucket 3 —
  Authority: architecture.md:158-166 (the one-writer doc contract).
SHOULD (architecture.md:158-166): one function, every step checked, missing meta
refused, temp removed on error, rename only after complete content. IS at 72c7293:
two additional DIRECT-APPEND writers exist under the same lock (`launch_time.*`
capture ae:2068-2075; `_cmd_spawn` rows ae:11923-11945 — census-3 audit I5), so
unlocked readers can observe partial canonical meta. Conflict: fix-known-defect(#88-I,
intended: every canonical meta update goes through ONE typed fail-closed atomic
transaction; direct append is unrepresentable; a reader sees a complete old or new
generation, never partial canonical bytes — both seats, transcribed from the issue
comment). Empirical: census-3 I5.

**SC-1302 — a session name's lifecycle operations serialize on one lock.** Bucket 3 —
SHOULD: start/resume/end/stop/rename/transfer/compact for one name serialize on
`.lifecycle.<name>.lock`. IS at 72c7293: with flock ABSENT the serialization silently
disables (census-2 matrix) — the #75 conflict is LOCAL to this row, not only
SC-1101a's. Conflict: fix-known-defect(#75, intended: native locking, never optional).
Authority: architecture.md lifecycle-lock contract. Empirical: census-2 (fd8 launch /
fd9 end, same lock file; degrade-to-unlocked matrix).

Externally visible atomicity, one head per surface (each:
`authority=code-observation`; Empirical: census-2 + deterministic probes per the
closure-map gate designs; Conflict: pending seat closure; UNCLASSIFIED):

**SC-1303 — rename: what a concurrent observer may see mid-operation.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1304a — transfer push: after stop completes, the source remains present and no
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
destination write has yet occurred; stop effects may be visible on the source**
(gate precision: present, not byte-intact — census-2:339-342).
**SC-1304b — transfer push: the remote destination may hold partial/mixed state
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
mid-operation.**
**SC-1304c — transfer pull: after stop completes, the remote source remains present
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
and no destination write has yet occurred; stop effects may be visible.**
**SC-1304d — transfer pull: the local destination may hold partial/mixed state
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
mid-operation** (frozen ae:11443-11457 + census-2:329-345 — gate correction: the
data-residue surface is the destination, per direction).
**SC-1305 — compact: mid-operation observability.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1306a — `list` snapshot cut under concurrent writes.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1306b — `status` snapshot cut.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1306c — `next` snapshot cut.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1306d — `requests` snapshot cut.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1306e — `events-tail` snapshot cut vs concurrent append/trim.**
  Authority: code-observation. Empirical: census-2 + deterministic probes per closure-map gate. Conflict: pending seat closure (UNCLASSIFIED).

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

One head per claim, fields per head (gate: no range-level association; A = Authority
`config.md watchdog table + watchdog.md:35-44`, E = Empirical pending, C = Conflict
none — spelled per row):

**SC-1400** — `AE_WATCHDOG_INTERVAL_SEC` defaults to 60. Bucket 2. Authority: config.md table
+ watchdog.md:35. Empirical: pending. Conflict: none.
**SC-1401** — `AE_WATCHDOG_STALE_MIN` defaults to 15. Bucket 2. Authority: config.md table +
watchdog.md:36. Empirical: pending. Conflict: none.
**SC-1402** — `AE_WATCHDOG_MAX_NUDGES` defaults to 2. Bucket 2. Authority: config.md table +
watchdog.md:37. Empirical: pending. Conflict: none.
**SC-1403** — `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` defaults to 5. Bucket 2. A:
  Authority: config.md table + watchdog.md:38. Empirical: pending. Conflict: none.
config.md table + watchdog.md:38. E: pending. C: none.
**SC-1404a** — `AE_WATCHDOG_TG_SUPERVISE_SEC` defaults to 120. Bucket 2. Authority: config.md
table + watchdog.md:41. Empirical: pending. Conflict: none.
**SC-1404b** — tg-supervise `0` disables supervision. Bucket 2. Authority: config.md table +
watchdog.md:41. Empirical: pending. Conflict: none.
**SC-1405a** — `AE_WATCHDOG_SWEEP_SEC` defaults to 300. Bucket 2. Authority: config.md table +
watchdog.md:42. Empirical: pending. Conflict: none.
**SC-1405b** — sweep `0` falls back to normal watchdog behavior. Bucket 2. A:
  Authority: config.md table + watchdog.md:42. Empirical: pending. Conflict: none.
config.md table + watchdog.md:42. E: pending. C: none.
**SC-1406a** — `AE_WATCHDOG_SWEEP_RETRY_SEC` defaults to 30. Bucket 2. Authority: config.md
table + watchdog.md:43. Empirical: pending. Conflict: none.
**SC-1406b** — sweep-retry is clamped to the sweep cadence (floor: next poll). Bucket
2. Authority: config.md table + watchdog.md:43. Empirical: pending. Conflict: none.
**SC-1407a** — `AE_WATCHDOG_SWEEP_RETRY_MAX` defaults to 6. Bucket 2. Authority: config.md
table + watchdog.md:44. Empirical: pending. Conflict: none.
**SC-1407b** — exhausting retry-max escalates exactly as SC-938 rules (cross-reference,
  Authority: config.md table + SC-938 cross-reference. Empirical: pending. Conflict: none.
not a duplicate behavior row). Bucket 2. A: config.md table + SC-938. E: pending.
C: none.

**SC-1408a — an explicit `AE_WATCHDOG_*` value wins over its `AE_LOOP_*` legacy name.**
Bucket 2. Authority: config.md. Empirical: pending. Conflict: none.

**SC-1408b — each documented tunable honours its `AE_LOOP_*` name when the primary is
unset.** Bucket 2 — per-mapping verification is one probe matrix. Authority:
config.md. Empirical: pending. Conflict: none.

Malformed-value classes, one head each (`authority=code-observation`; Empirical:
pending probe; Conflict: pending seat closure; UNCLASSIFIED):

**SC-1409a — non-numeric values in numeric watchdog/loop tunables.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1409b — malformed telegram include/exclude lists.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1409c — malformed `allowed_user_ids`.**

Remaining `AE_*` variables, one head each (`authority=code-observation`; Empirical:
pending probe; Conflict: pending seat closure; UNCLASSIFIED — except where noted):

**SC-1410a — `AE_HOME`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410b — `CONFIG_FILE`/`AE_LOCAL_CONFIG` precedence** (#87 names the intended
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
authority).
**SC-1410c — `AE_TMUX_SERVER`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410d — `AE_NO_AUTOSTART`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410e — `AE_END_SERVER`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410f — `AE_HUB_DIR`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410g — `AE_STEWARD_DIR`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410h — `AE_EVENTS_KEEP`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410i — `AE_SEND_DEFER_SEC`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410j — `AE_ATTN_REQUEST_SECS`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410k — `AE_LIST_ACTIVE_SECS`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1410l — `AE_COMPACT_HANDOVER_SECS`.**
Launch-token/slot vars, split by shared invariant per provider (gate grain — each:
`authority=code-observation`; Empirical: pending probe; Conflict: pending; UNCLASSIFIED):
**SC-1411a — `AE_CODEX_LAUNCH_ID`/`AE_CODEX_SLOT`** (one invariant: the token pair
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
identifies the launch for post-launch capture).
**SC-1411b — `AE_GEMINI_LAUNCH_ID`/`AE_GEMINI_SLOT`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1411c — `AE_OPENCODE_LAUNCH_ID`** (inert since the config route — census note).
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).

Resolution/context exports, split by shared invariant (same field defaults):
**SC-1412a — `AE_RESOLVED_*`** (one invariant: the resolution result set a helper
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
exports for its target).
**SC-1412b — `AE_SESSION`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1412c — `AE_META`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1412d — `AE_DIR`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1412e — `AE_MODE`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1412f — `AE_ORIGIN`.**
  Authority: code-observation. Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).
**SC-1412g — `AE_PATH`/`AE_PATH_BIN`** (one invariant: ae's own path identity).
  Empirical: pending probe.
(`AE_SENDER_OVERRIDE` is SC-974a, not duplicated.)

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
| DR-006 | Helpers become versioned Rust + thin shims | RATIFIED (both seats, 2026-08-20) |

```
DR-006 Helpers become versioned Rust + thin shims
- affected SC ids: SC-400c; the S3 helper-surface rows (their CLI signatures are the
  compatibility contract the shims preserve); SC-211o's mechanism note; interacts with
  SC-1001 (phase-owned asset migration) and SC-1004 (atomic publication survives as
  the shim-publication rule).
- context / current IS: helper LOGIC is generated bash (declare-f emission) per
  session; #76 proved the drift class (a session driven through its own stale
  helpers); the epic retires generated logic at P2.
- decision / binding outcomes: helper business logic moves into the versioned Rust
  binary; sessions keep thin generated SHIMS that exec it. (1) SHIM COMPATIBILITY —
  every documented helper CLI signature (S3 rows) is preserved by the shim surface;
  (2) ATOMIC REFRESH/MIGRATION — shim publication stays atomic per artifact
  (SC-1004), and a phase upgrade migrates sessions without a broken intermediate
  state; (3) BINARY-PATH/VERSION OWNERSHIP — the shim addresses an installed,
  versioned binary at a known path; version skew between a session's shims and the
  binary is DETECTED and reported, never silently divergent (#76's class, closed
  structurally); (4) ROLLBACK BOUNDARY — reversible until the P2 phase gate passes,
  per the epic rollback rule; the flip commit names the revert.
- trade-offs accepted: a binary dependency for every helper call (ms startup measured
  acceptable per epic); the generated-logic auditability moves from emitted bash to
  the binary's tests.
- authority: epic #79 (strangler strategy, P2 phase), #76 (the drift-class finding),
  gate ruling family-gate-3 (the b4-requires-DR rule).
- seats + date: fable5:lead (record) + gpt56sol:colead (required the record,
  content per their gate), 2026-08-20.
```

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
- affected SC ids: SC-200; SC-204 (no-outbox promise retires); SC-201/202/203 (recast
  as notification-safety invariants that survive — the stored body is never lost or
  undone by notification failure); SC-205 (one transport primitive becomes
  store-commit + notification); SC-210 (unprotected-delivery degradation retires with
  the notification path); S5 mailbox persistence + S6 `msg` CLI output rows (drafted
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
  SC-976a (generation-aware cursor); SC-400 (live layout re-cut at the flip, with
  legacy-read/migration/write ownership stated); SC-401 (one-canonical-stream vs
  preserved-generations archive decision at P2); SC-1300 (mechanism superseded,
  promise kept); S10/S14 reader rows (census-3 audit I1 stat/open race); B3 (census-3
  addenda). SC-511c (additive event-OBJECT keys) is NOT affected — the conflict is
  with the no-rotation CONTAINER promise, and SC-511c stays bucket 2 conflict=none
  (consistency hold resolved 2026-08-20).
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
