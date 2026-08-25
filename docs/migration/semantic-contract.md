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
being checked against the grammar afterwards. Authority: AGENTS.md@72c7293:164
session-names bullet ("`default_session_name` (which *guarantees* the grammar for any
PWD rather than being checked against it)") (ruling). Empirical: unit pins @72c7293.
Conflict: none.

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
Authority: commands.md@72c7293:33 usage synopsis ("ae end|rm [-f]
[--purge-history|--keep-history] [name]") — anchor corrected: the frozen doc has no
dedicated end section; the synopsis line is the claim-bearing sentence.
Empirical: pending. Conflict: none.

**SC-012 — `ae help` prints the short command surface.** Bucket 2 — narrowed to the
documented outcome only (seat ruling ae-20260820T165746Z-fb9c4fb6): `ae help` prints
the short command surface; inherits the M2 bootstrap caveat like every dispatcher
entry. This row does NOT cover bare invocation — commands.md:4 owns bare `ae [name]`
as launch/reattach, not help (residue: SC-012b). Authority: commands.md@72c7293:39
("ae help — Show short help"). Empirical: pending. Conflict: none.

**SC-012b — top-level `-h`/`--help` as aliases of `ae help`.**
`authority=code-observation` — the frozen dispatcher has exactly ONE help branch,
`help | -h | --help` at ae:16841-16843 (`cmd_help` has no other caller); the two flag
spellings are undocumented in the frozen docs. Narrowed from a broader
"dispatcher-fallback" phrasing by seat second-gate correction (there is no such
surface: unknown OPTION is SC-022's, non-option falls into launch per commands.md:4).
Probe + seat ruling. Empirical: pending probe. Conflict: pending seat closure
(UNCLASSIFIED).

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
~5min, `AE_LIST_ACTIVE_SECS` tunes, `--busy` alias.
**RELATIVE-TIME SPANS ARE SEMANTICALLY SCORED, NOT EXEMPTED** (Option A as ruled,
colead 2026-08-25). This row hosts the mechanism for BOTH human relative spans —
this row's active age and SC-405f's goal age — and SC-405f cross-references it, so
the rule has one home.
**The scored requirement is a SINGLE WITNESS EPOCH, not per-span membership.** One
human invocation samples ONE `World.now`, so the comparison must find one epoch `t`
inside the runner's recorded before/after wall-clock bracket for that invocation
which explains ALL rendered relative spans JOINTLY — each against its own fixed
epoch, through the frozen formatter.
**Per-span independent scoring is the FORBIDDEN WEAKER FORM**, named here so it is
refused rather than merely unmentioned: checking each span for membership in its own
allowed set accepts a document whose spans are valid at DIFFERENT moments. The
mandatory red seed is exactly that — two spans individually valid at opposite
bracket endpoints with NO common `t` MUST FAIL. A document whose relative texts
imply two different reading moments describes no moment at all, which is the rule
the digest already carries for its stamp and the attention beside it.
**Fail-closed inputs.** A missing bracket, a backward clock, or an unbounded
interval FAILS; none of them may widen the allowed set. A paired digest
`generated_at` may CORROBORATE its own bracket and may never substitute as the human
anchor. Unit formatter tests with a supplied `World.now` prove bytes and rounding
only — they are not evidence about any invocation's epoch.
**Why a mechanism rather than an open choice.** An open choice would exempt these
bytes; this SCORES them, so a wrong relation FAILS rather than passing unexamined.
It adds no harness seam to the product: the bracket is the runner's own recording,
and nothing in the product is asked to expose a clock for it.
The projection doc's BYTE-ROLE class for these spans is colead's authority and
changes with their signature; this row governs how they are SCORED once classified,
not which role they carry.
Authority: commands.md:87 + colead Option-A ruling 2026-08-25. Empirical: pending.
Conflict: none.

**SC-017f — `--json` honours the active filters.** Bucket 2. Authority:
commands.md:88. Empirical: pending. Conflict: none.

**SC-017g — the attention marker is the single most-actionable reason, and a quiet
entry RENDERS its triad rather than omitting it.** Bucket 2 —
dead > stale > waiting-user > blocked > throttled > unanswered, derived as the MAX
across agent reasons PLUS session-level unresolved-request facts (amended slice-1b:
unanswered is a PAIR fact with no owning agent — cross-session ask/review makes
target ownership non-local; agents[].reason never reads unanswered).
**REOPENED AND PRECISED** (colead ruling, 2026-08-24) after a successor serializer
omitted `attention` and `attention_rank` on entries needing no attention, and a
boundary test asserted that absence as correct.
**The precision:** every session entry that was READ carries all three attention
members — `needs_attention`, `attention`, `attention_rank`. An entry needing no
attention renders them `false`, `null` and `0`. Absence of a member is NOT a
spelling of "no attention"; a reader may treat a missing member only as the loss
SC-509b describes.
**This is not IS becoming SHOULD.** The frozen authority already fixes it and this
row inherits it: SC-509 lists `needs_attention/attention/attention_rank` among "the
documented session fields"; SC-509d carries "All other SC-509 fields and SC-509b
degradation semantics carry forward unless another row changes them", and no row
changes these; and no open-choice row authorises movement between presence and
absence for any SC-509 member.
**The row this would otherwise collide with rules the other way.** SC-509b's
omission clause is scoped to "unreadable optional facts", and its closing
sentence is the whole argument — "Damage is never rendered identically to legitimate
sparsity". An omitted `attention` on a fully-read quiet entry is exactly that
collapse: loss and legitimate-none become one byte pattern, the condition SC-509b
exists to forbid. Reusing loss's spelling for a legitimate answer does not extend
SC-509b; it destroys it.
**JOINT INTERPRETATION — required in-row (colead, 2026-08-24).** The values below
describe an entry whose inputs were READ. When they were not, SC-509b governs:
`attention` and `attention_rank` are OMITTED rather than nulled UNLESS the answer
remains exact under SC-509b's maximum/upper-bound rule, and `needs_attention` always
renders as an ALWAYS-PRESENT PARTIAL-EVIDENCE INDICATOR: `true` iff >=1 contribution
remains established after reducing the READABLE facts; `false` iff none remains
established in those readable facts. Later readable records may add, clear, or
supersede a contribution, so more input may change `true` to `false` as well as
`false` to `true`. When the loss could affect the ATTENTION INPUTS, neither
boolean value alone proves the exact final attention: missing facts may add,
clear, or supersede a contribution. When SC-509b's exactness rule establishes the
maximum despite unrelated loss, the full triad remains exact — aggregate
`degraded` does not make a per-member answer uncertain. Either way,
`degraded: true` is the mandatory incompleteness qualifier. `null` here
means read-and-quiet and nothing else, which is exactly why it may not stand in for
"not established".
**SCOPE GUARD — required in-row.** This row owns the attention triad's VALUES —
`false` / `null` / `0` when quiet. It does NOT own presence as a class: SC-509 does,
and its presence rule reaches every documented member on the same evidence. The
first draft of this guard claimed the eight neighbouring members "need their own
evidence read", which the census in this very row had already falsified; corrected
on colead's ruling, 2026-08-24. `degraded` is untouched — SC-509b states its
omission on a normal entry positively. `generated_at`'s exact bytes remain open.
`agents[].reason`'s value is SC-509c's.
Authority: commands.md:60-76 + slice-1b joint ruling + SC-509 field list + SC-509d
carry-forward + SC-509b scope and its identical-rendering prohibition + colead
ruling 2026-08-24. Empirical: **observed** — within SC-509's fixed 401-capture
census, all 431 frozen v1 session entries carry all three members, and all 193 quiet
entries among them render exactly `false` / `null` / `0`; zero frozen entries carry
`degraded`, so omission of these members is unattested anywhere in the frozen record
while their presence is universal. Conflict: none — SC-509b is read, not overridden.
**classified_by: RE-MARKED after the 2026-08-24 presence precision — gpt56sol:colead
ruling, drafted opus5:reason2.**

**SC-017h — the tabular view shows per-agent health, declared state, and the session
attn marker.** Bucket 2. **IN-ROW JOINT RULING (colead, 2026-08-24):** declared
state has three distinct renderings: exact `Some(state)` renders that state; exact
no declaration renders `-`; an inexact or unreadable event-derived state renders
`unknown`. Loss must not publish a stale partial state and must not collapse into
legitimate absence — SC-509b's rule is that damage is never rendered identically to
legitimate sparsity. This row owns declared-state rendering only; agent health and
liveness remain separate under SC-017p/q/r. Authority: commands.md:56-59 + SC-509b
+ colead ruling 2026-08-24.
Empirical: **observed** — the measured human state-cell census is 34 cells whose
frozen rendering moves under this row: 24 in A1 c03/c07/c08 rendering `-`, 8 in
A9 c04 rendering `working` or `done` four each, and 2 in A9 c04's `ro-noserver`
variant rendering `-`; each moves to `unknown`. Derived independently three times —
colead, `sc017hscan`, and a third reading taken while drafting this amendment, which
reproduced all three splits and their frozen VALUES exactly. Recorded as EVIDENCE
for the ruling above and never as a normative count: the scorable rows are lexec's
derivation, and a cardinality that must be UPDATED to stay true does not belong in a
row beside facts that are CHECKED. Conflict: none.

**SC-018 — `ae [name] use <alias>` starts the session with that agent as main.**
Bucket 2. Authority: commands.md@72c7293:5 ("ae [name] use <alias> — Start session
with a specific agent as main"). Empirical: pending. Conflict: none.

**SC-018b — `use` interaction with resume.** `authority=code-observation` — what
`use` does against an existing/resumable session is undocumented; probe + seat ruling.
Empirical: pending probe. Conflict: pending seat closure (UNCLASSIFIED).

**SC-019 — `jump` is an alias of `next`.** Bucket 2. Authority: commands.md@72c7293:10-11
("alias: ae jump" in the next synopsis) + section head :138 ("ae next (alias
ae jump)"). Empirical: pending. Conflict: none.

**classified_by (S1/S2 MARK batch 3, ae-20260820T165746Z-fb9c4fb6): SC-100, SC-011,
SC-012, SC-018, SC-019 — fable5:lead + gpt56sol:colead, 2026-08-20. Exact
enumeration; later rows never inherit this mark. SC-100 bucket 1, the rest bucket 2;
conflict=none throughout. Marked with the countersign condition applied first:
SC-012 narrowed to the documented `ae help` outcome, with bare-invocation help-path
reachability split out as the NEW code-observation residue SC-012b (NOT marked;
UNCLASSIFIED, Q3, CRIT-ASSIGN H-HELPER in-class with SC-013/SC-014).
Normative/conflict lane only; Empirical remains pending where so marked.**

**SC-017i — `--running` is the explicit spelling of the default filter.** Bucket 2.
Authority: commands.md:83. Empirical: pending. Conflict: none.

**SC-017j — inventory candidates exist before liveness is classified.** Bucket 3 —
fix-known-defect(#105). The list inventory is the union of (a) durable current-session
state under SC-400d's canonical and legacy-readable layouts
and (b) positively identified ae-owned live tmux sessions on a server the product is
already entitled to query. The entitled server set is finite and pointer-derived: (1)
the single ambient server selected by this invocation's ordinary tmux transport, and
(2) every distinct positive, unambiguous server selector read from a durable inventory
candidate under that candidate's ratified current or legacy format. A
missing/ambiguous selector confers no entitlement; it leaves that durable candidate in
inventory with liveness unknown. Inventory attempts enumeration of every distinct
entitled server; positively proven equivalent selectors MAY share one query, and every
successful enumeration contributes every positively ae-owned live session it found.
ae does not gain entitlement by sweeping arbitrary tmux socket paths or server names. A
live session with no durable candidate on a server outside this set is absent from
inventory by epistemic limit — not classified stopped or unknown — and may become
visible later when an ambient selection or durable record supplies a pointer. SC-1410c
separately owns whether/how AE_TMUX_SERVER selects the ambient server; this row consumes
the selected ambient server and does not ratify that environment control. Archives are
inert and never enter this inventory. Every
durable candidate survives into classification: a failed liveness query, a prefix-only
name match, or a live exact-name session whose ownership marker is missing cannot delete
the candidate. A positively live tmux-only candidate remains visible; loss of its
durable record is the separate SC-509b `degraded` fact. This row does NOT authorize
basename-only deduplication of distinct identities. Discovery completes before
reconciliation. A positively ae-owned live sighting MAY coalesce with a durable
candidate only when the durable selector is positive, the sighting came from a
successful query of that candidate's recorded server, its tmux name exactly equals the
SC-400d inventory name, and **exactly one** durable candidate matches that server/name
tuple. With zero durable matches the live-only candidate remains; with more than one,
NONE merges and every durable candidate plus the live sighting remains. Missing or
ambiguous selectors, name-only equality, prefix equality, ambient membership, and
unproved equivalence between selector spellings never authorize a merge. Server plus
exact name is a one-to-one **join witness**, not stable identity. IS at 72c7293:
VIOLATED —
`list_ae_sessions` discovers ambient tmux sessions and requires `AE_SESSION` at
ae:2682-2693, while `iter_stopped_sessions` separately discovers disk directories and
then removes them through `tmux has-session -t "$name"` at ae:2697-2708. A missing
marker or a prefix sibling can therefore remove the same durable candidate from both
blocks. The prefix behavior itself was observed on isolated tmux 3.7b/Darwin
(exact control 0, absent negative control 1); the resulting product disappearance is
source-proven, not yet captured end-to-end. Empirical: primitive prefix behavior
observed; product disappearance source-proven, end-to-end capture pending. Authority:
architecture.md@72c7293:61-83 +
SC-400a + SC-400d + SC-509b + joint P1 ruling. Issue #105 records IS/conflict only; it is not
normative authority. Conflict: fix-known-defect(#105).

**SC-017k — list liveness is a positive, exact fact from the session's own server.**
Bucket 3 — fix-known-defect(#105). A durable candidate is `running` only when a
successful query of its recorded tmux server returns the exact session name with
positive ae-ownership evidence; it is `stopped` only when a successful query of that
same server proves the exact name absent. For a candidate sourced solely from a live
tmux discovery, that successful discovery query is the positive server fact for this
snapshot; it does not fabricate a durable server record. A positively owned exact-name live
sighting that SC-017j coalesces with exactly one durable candidate came from a successful
query of that candidate's recorded server. It remains the positive server fact for that
snapshot after coalescence, exactly as for a tmux-only candidate: the dual-provenance
candidate is `running` without requiring another liveness query. A later or redundant
query may not retroactively replace that accepted snapshot fact; changed liveness is
observed by a fresh inventory/classification snapshot. A dual candidate without that
matched sighting follows the ordinary durable-candidate query rule. Ambient-server membership,
prefix success, and the renderer block that happened to print the row are not liveness
facts. Implementations MAY group candidates by recorded server and query each server
once; every candidate's answer must still come from its own server and exact name. IS at
72c7293: VIOLATED — there is no per-session liveness predicate: the running block writes
the literal `running` (ae:4244), the stopped block writes `stopped` (ae:4290), and the
only stopped-side test is ambient, prefix-matching `tmux has-session -t "$name"`
(ae:2706). The prefix behavior is observed at the tmux primitive; ambient-server and
block-provenance violations are source-proven. Empirical: primitive prefix behavior
observed; remaining product relations source-proven, end-to-end capture pending.
Authority: SC-835a's recorded-server /
exact-identity rule + the SC-832d/e exact-name hazard + joint P1 ruling. Issue #105 is
IS/conflict only. Conflict: fix-known-defect(#105).

**SC-017l — unprovable liveness is first-class `unknown`.** Bucket 3 —
fix-known-defect(#105). An unreachable, missing, or ambiguous recorded server; a failed
server query; or an exact live name with missing/mismatched ownership evidence yields
`unknown` — never `stopped`, never absence.
**Absence is owned HERE and ALSO by SC-017m, at DIFFERENT GRAINS** (ruling, colead
2026-08-25, retracting an earlier exclusive split): an omitted durable candidate owes
BOTH — an obligation under this row that its liveness fact is `unknown` rather than
absent, which is the PER-CANDIDATE grain, and SC-017m's PER-VIEW membership
contribution that the view's exact candidate set includes it. One omission violates
two rows. That is not double counting, because the grains differ — the same shape as
`needs_attention` and `degraded`, two obligations on one occurrence rather than one
obligation stated twice.
**The enforceable invariant is CANDIDATE-GRAINED.** For every omitted durable
candidate, its SC-017m view-membership contribution must PAIR with an SC-017l
absent-to-unknown obligation AT THE SAME CANDIDATE IDENTITY. A table carrying one
without the other is undercovered at that identity, and the identity is where the
test lives.
**Aggregate counts and set-disjointness between the two families are DIAGNOSTIC
EVIDENCE ONLY, never the test.** They are neither necessary nor sufficient for
undercoverage; they compare different grains, since one family counts invocations
while the obligation counts candidates; and they go stale the moment the table is
repaired. A symptom that disappears when the defect is fixed cannot be the rule that
detects it. `unknown` is a fact about liveness knowledge.
It is orthogonal to SC-509b's `degraded`, which is a fact about record read/parse loss:
either, both, or neither may hold. IS at 72c7293: VIOLATED — a failed ambient
`list-sessions` produces zero running rows (ae:2692), and each stopped-side
`has-session` then fails (ae:2706), so every durable directory is printed `stopped` with
no diagnostic; a non-ambient live session is misclassified the same way, while a live
session missing `AE_SESSION` disappears. One violated outcome is observed end to end in the
accepted Batch C corpus: 38 no-server cases (228 list/ls consumer rows) retain durable
candidates while the tmux query is unavailable, and frozen bash renders those candidates
`stopped` or omits them from the active view in human and JSON output instead of `unknown`.
The corpus predates and did not target SC-017l; that makes the capture incidental, not
inadmissible. It closes the failed-query/unreachable-server baseline only. Non-ambient and
ambiguous recorded servers, missing/mismatched ownership evidence, and unknown x degraded
orthogonality remain uncaptured; the remaining product relations are source-proven.
Empirical: observed(Batch C A1 no-server corpus, indexed by
docs/migration/evidence/corpus/P1-SUFFICIENCY.md@71e8a83); scoped matrix gaps as named. Authority: SC-816 + SC-835c's established
rule that inability to verify is not absence + joint P1 ruling. Issue #105 is
IS/conflict only. Conflict: fix-known-defect(#105).

**SC-017m — list renders and filters `unknown` without hiding it.** Bucket 3 —
fix-known-defect(#105). Human and JSON surfaces spell the status `unknown` explicitly.
The default / `--running` view is the active inventory, not stopped-history noise: it
shows `running`, then `unknown`. `--stopped` shows only `stopped`; `--all` shows
`running`, then `unknown`, then `stopped`. Attention/activity filters remain
positive-running predicates and never relabel an unknown session. `unknown` alone does
not set `degraded`; a damaged record may independently carry both. This precisely
narrows SC-017a/b/c/i: an unknown session is not stopped history, and hiding it from the
active view would recreate #105. SC-017f requires the same selection in human and JSON
renderings. IS at 72c7293: VIOLATED — status is the literal of the block that printed the
row, so an unverified candidate is either silently absent or rendered `stopped`.
Empirical: source-proven, end-to-end capture pending. Authority:
commands.md@72c7293:78-88 + SC-509b + joint P1 ruling. SC-509d owns the
machine-schema version change; issue #105 is IS/conflict only. Conflict:
fix-known-defect(#105).

**SC-017n — list owns a portable deterministic session order.** Bucket 2 — within
each status group, session names sort by raw byte / `LC_ALL=C` order; group order is
`running`, `unknown`, `stopped` (with filters retaining only their selected groups).
The session-name grammar is ASCII, so byte order is locale-independent and unambiguous.
The product sorts itself: tmux emission, filesystem glob, root traversal, locale, and
creation/id order never become output contracts by accident. Authority: SC-017b's
existing inter-group order + joint P1 reproducibility ruling. Empirical: isolated tmux
3.7b/Darwin emitted byte-identically to `LC_ALL=C sort` across opposed creation/id and
case/numeric-looking names, but ae applies no sort; that one implementation's observed
order is IS only, never authority or a cross-version guarantee. Conflict: none.

**classified_by:** SC-017j SC-017k SC-017l SC-017m SC-017n — P1
inventory/liveness joint ruling, 2026-08-21; fable5:lead + gpt56sol:colead. Exact enumeration;
SC-017j..m are bucket 3 fix-known-defect(#105), SC-017n is bucket 2 conflict=none.
Normative/conflict classification and the explicitly scoped source/probe empirical lanes
only; no primitive probe is promoted to an end-to-end product capture.**

**SC-017o — incomplete inventory is explicit snapshot state, never a synthetic session.**
Bucket 3 — fix-known-defect(#105). Every inventory snapshot records whether every
enumeration operation required to form SC-017j's candidate union completed, and retains
one loss fact for each logical operation whose final outcome was failure. ANY required
traversal whose failure hides candidates the snapshot would otherwise have been entitled
to see makes the snapshot INCOMPLETE, whether that traversal enumerates candidates
directly or discovers further enumeration sources. Current non-exhaustive instances are
the canonical sessions root, the worktrees root, a discovered worktree `.ae` subtree, and
an entitled tmux server. A failed intermediate traversal records its own loss only; it
never fabricates losses or identities for child sources it could not discover. Here
"final outcome" means the operation still failed after whatever retry policy the
implementation chose; it does not mean a leaf node in the discovery graph. Discovery
continues wherever independent sources remain usable, and every candidate found from them
survives. The loss COUNT is of failed logical operations actually attempted and known —
never of hidden candidates or hypothetical child operations: one failed worktrees-root
enumeration contributes one loss however many unknown subtrees it may contain. No candidate, name, status, `unknown` value, or `degraded` value is fabricated
for identities the failed source may contain. A missing durable root or absent worktree
`.ae` subtree is an AUTHORITATIVE EMPTY SOURCE, not a loss; archives and servers outside
the entitled set were never required and do not make the snapshot incomplete. Once a
candidate directory was discovered, missing/unreadable `meta` remains that candidate's
separate SC-405i/SC-509b record-loss fact and does not become enumeration incompleteness.

User visibility is MANDATORY. Human `list`/`ls` output keeps its partial table and emits
an explicit stderr diagnostic containing at least the NUMBER of failed logical sources;
exact wording, whether paths/targets are also named, and exit status are OPEN CHOICES.
Every successor JSON digest emits top-level boolean `inventory_complete`; `true` means
zero required enumeration losses, `false` means one or more. It is present even for an
empty inventory. Internal loss representation and ordering remain open; they may not leak
guessed session identities. SC-509d remains schema version 2 — version 2 has not shipped,
and this row is the "unless another row changes them" addition SC-509d already permits.
Version 1 remains unchanged.

The useful fact is not WHICH sessions were lost; it is that ABSENCE IN THIS SNAPSHOT IS
NOT PROOF. A listing that silently omits an unknowable number of sessions asserts a
completeness it did not establish, which is the confident-empty shape #105 exists to
remove; having no identity to report is why the signal belongs at the snapshot level, not
a reason to withhold it. IS at 72c7293: VIOLATED — `iter_stopped_sessions`
enumerates durable roots through an unguarded shell glob and only directory/continuation
guards (ae:2697-2708); neither it nor `cmd_list` carries an enumeration-loss fact or
diagnostic to any surface (ae:4153-4304). The absence of a reporting path is
source-proven; the exact unreadable-root runtime cell is not captured. Empirical: source-proven; successor implementation
pending. Authority: SC-017j + SC-509b + SC-509d + joint P1 ruling. Issue #105 is
IS/conflict only. Conflict: fix-known-defect(#105).
**classified_by:** SC-017o — incomplete-inventory snapshot ruling, 2026-08-21;
fable5:lead + gpt56sol:colead. Bucket 3 fix-known-defect(#105); grain is an identityless
whole-snapshot property that neither SC-017j membership nor SC-017m per-session rendering
can absorb without conflating subjects.**

**SC-017p — per-agent liveness is a positive, exact fact from the session's own
server.** Bucket 3 — fix-known-defect(#105). `agents[]` membership remains the durable
SC-405k roster; liveness classifies each roster member without inventing agents. An
agent is `alive` only when a successful observation of the candidate's positively
recorded server and exact session establishes an exact association to that agent's
pane/slot and that pane satisfies the ratified live predicate of SC-017s. **Amended
2026-08-21**: this clause previously read "positively recognizes its agent process as
live" — a PROCESS claim whose phrase occurred exactly once in this contract, inside this
row, and which no row defined. It is amended to a pane-observation claim in the same
ruling that ratifies SC-017s, called out here rather than folded into that row's landing,
because a ratified row is being altered. An agent is `dead` only
when an equally successful observation positively proves that exact roster agent has
no live pane/slot, or that its exactly associated pane satisfies the ratified dead
predicate of SC-906. A successful complete pane enumeration that returns no pane
associated with the roster agent proves `dead` only when every observed pane carries a usable,
unambiguous association to some other identity (or no panes exist); an unassociated or
ambiguously associated pane leaves the missing roster agent `unknown` under SC-017q.
A successful exact-session absence proof proves every roster agent dead for that
snapshot; successful exact-session presence alone proves no individual agent health.
Implementations MAY share one server/session/pane enumeration across agents,
but every answer must still belong to that server, exact session, and exact roster
agent. Ambient-server membership, prefix success, which renderer emitted the row, and
another agent's pane are not agent-liveness facts. Frozen IS at 72c7293: VIOLATED —
`cmd_list` invokes ambient `tmux list-panes -s -t "$name"` and discards every error
(ae:4200-4207), then treats absence from the resulting map as not alive
(ae:4053-4058). Successor IS at 92a20ee9: listing runtime is constructed with no agents
(`src/listing.rs:139-151`) and a missing runtime member maps to false
(`src/session.rs:519-533`). Empirical: frozen and successor relations source-proven;
successor end-to-end agent-liveness matrix pending. Authority:
commands.md@72c7293:56-59 + SC-017k's own-server/exact-identity rule + SC-017s
(the alive route) + SC-906 (the dead route, and see that row's status) + joint
P1 ruling.
Issue #105 names the session-level instance; this ruling extends the same epistemic
defect family to per-agent health and does not take normative authority from the issue.
Conflict: fix-known-defect(#105).

**SC-017q — unprovable agent liveness is first-class `unknown`.** Bucket 3 —
fix-known-defect(#105). A session whose liveness is `unknown`; a missing or ambiguous
recorded server; a failed exact-session or pane query; or an unusable, missing,
duplicate, conflicting, or otherwise ambiguous pane/slot marker that prevents
SC-017p's positive or negative proof yields agent health `unknown` — never `dead`,
never removal of the roster agent. A successful complete enumeration returning no
matching pane is not ambiguous merely because other panes carry usable markers for
other identities: that is SC-017p's negative proof. A successfully observed exact live
session with an unowned or ambiguously owned pane does not turn that pane into this
agent and does not prove the roster agent absent.

Session and agent liveness are separate grains but not a free Cartesian product in one
snapshot. `stopped` from successful exact-session absence implies every roster agent
`dead`; session `unknown` implies agent `unknown`; `running` permits agent `alive`,
`dead`, or `unknown` according to the pane observation. For a roster agent whose
identity survives discovery, SC-509b record degradation, the agent's declared state,
and its independently established attention reason remain orthogonal to agent
liveness: none supplies a missing liveness fact, and each retains only its
independently established value. Until the real transport exists, `unknown`
is the honest output. Frozen IS at 72c7293: VIOLATED — a failed `list-panes` query
is indistinguishable from an empty successful result at ae:4200-4207, so JSON emits
`alive:false` at ae:4057-4065. Successor IS at 92a20ee9: the absent runtime member
described in SC-017p repeats the collapse. Empirical: frozen and successor
relations source-proven; successor positive/negative/unknown matrix pending.
Authority: SC-816 + SC-835c's rule that inability to verify is not absence + SC-017l
+ joint P1 ruling. Issue #105 is the defect-family label, never normative authority.
Conflict: fix-known-defect(#105).

**SC-017r — human list renders agent liveness without collapsing `unknown`.** Bucket 3
— fix-known-defect(#105). Every selected roster agent's human row has three
distinguishable, non-silent health renderings carrying SC-017p/q unchanged.
**The owed POPULATION is every selected roster agent, and an UNATTEMPTED observation
changes the VALUE rather than the membership** (ruling 2026-08-25, fable5:lead;
colead no-contest). SC-017p/q's unattempted state is what the unknown rendering
RENDERS, so a row is owed for an agent whose observation was never attempted exactly
as for one whose observation failed — excluding unattempted would exclude the case
this row's own violation narrative is about.
**This includes rows the successor ADDS under SC-017m's membership obligations that
the predecessor omitted.** Membership is SC-017m's grain and member VALUE is this
row's, so a newly present row owes per-agent health correctness here. Stated
explicitly because the contrary carve-out was tentatively held and then WITHDRAWN,
and would otherwise be reinvented by a reader who assumes an added row is SC-017m's
business alone.
**The identity this row addresses is a FIXED PRE-SUCCESSOR HUMAN PROJECTION, and VALUE
bytes are no part of it** (ruling 2026-08-25, gpt56sol:colead + fable5:lead, on a
two-derivation comparison that a name-free obligation could not have surfaced). The
projection is the SESSION identity plus the agent display and identity fields the human
projection RETAINS. It EXCLUDES health, and every independently mutable state, reason and
attention cell — those are values this row and its neighbours may legitimately change, so
keying identity on them would let a value edit silently re-partition the population. The
ruled minimal form is per-session rendered NAME plus the health MULTISET.
**The class is fixed BEFORE any health value is read.** Partitioning uses the projection
and nothing else, so a health difference can never move an agent between classes — the
population is settled first, and only then is the owed fact for each class determined.
**Differing values do not manufacture roster identity.** Two agents rendering under one
display name may carry DIFFERENT health and remain UNBOUND: nothing in the human bytes
associates either value with either roster slot. A class of cardinality ONE is
identity-addressed only where the projection actually establishes the roster association;
where it does, health is owed at that identity exactly as before. Where the projection
leaves two or more agents in one class, the owed fact for that class is an ORDER-FREE
COUNT of semantic health values at EXACT multiplicity.
**Order-freedom is owned HERE, by the evidence, and borrowed from nothing.** The human
bytes carry no occurrence identity for such a class — the subline simply repeats the
display name — so an obligation cannot be keyed on a distinction the evidence does not
carry, and this row declines to invent one. No registered open choice is cited or relied
on: the registered equal-name tie is session-candidate order and does not reach agent
rows, and widening it by citation is exactly what this paragraph refuses. A display name
was never an identity.
**What that makes fail on THIS ROW'S SURFACE, and what it does not.** On the HUMAN
surface: DROPPING one of the indistinguishable entries FAILS, and rendering the class at
the WRONG MULTIPLICITY of health values FAILS. EXCHANGING two entries inside the class is
NOT OBSERVED and therefore neutral — no obligation is keyed on a binding the evidence does
not carry. That is a consequence of the evidence, not a tolerance granted, and not a
licence for agent order anywhere else. A derivation may choose neither a LIST, which
invents an order the bytes do not carry, nor a SET, which drops a real agent; both make
their totals agree with something.
**This row is HUMAN-ONLY and this grain does NOT reach the digest.** Digest agent
multiplicity and health stay owned by the existing JSON rows and by default parity, never
by SC-017r. The frozen digest is invoked here for ONE purpose only — to CORROBORATE that
no cross-surface escape recovers the lost identity, its entries for such a class being
byte-identical down to `session_id` — and corroborating evidence is not a scored surface.
**The collision stays a frozen DEFECT and this grain is not its licence.** Naming the
count is how the obligation remains checkable while indistinguishable agents exist; it
ratifies nothing about rendering them that way.
**EMPIRICAL COVERAGE GAP — the rosters of rows SC-017m ADDS are not observable in this
corpus.** The paragraph above owes their agents' health and that duty stands UNCHANGED;
what is absent is evidence, not obligation. Each session's meta is carried as a HASH and
the captured agents output is scoped to its own capturing session, so no added row's
roster is recoverable here. The gap is named PER KNOWN ADDED SESSION rather than as one
blanket hole, so it stays enumerable in the obligation table and cannot quietly absorb a
session nobody noticed. This is a stated limit on what the corpus can EXERCISE — never a
normative exclusion, and never a claim that the exercised population is complete.
Occurrence counts live in the obligation table, never here.
**The two are DIFFERENT KINDS and must not be merged.** An indistinguishable collision is
ADDRESSABLE, as an order-free count, and is owed. An added row's roster is not observable
at all, and is declared. Collapsing them would either mint obligations for agents no
evidence can name, or excuse obligations that are perfectly checkable. The exact
alive/dead/unknown words or glyphs are OPEN CHOICE, but the unknown rendering must be
unambiguously recognizable as unknown rather than absence or blank output. Agent
`unknown` never silently renders as alive or dead, never disappears, never overwrites
the agent's separately known declared state or reason, and does not by itself
manufacture the session-level `attn:dead` marker. Session selection and ordering remain
the SC-017m/n facts; this row changes presentation of an agent fact, not session status
or filter membership. `status` is out of scope: it renders pane content/existence, not
the list/ls per-agent health field, and its failed-query-versus-empty-session behavior
requires a separately classified row before migration.

Frozen IS at 72c7293: VIOLATED — dead is `!`, alive is the empty marker, and the human
row also defaults a missing map entry to the empty marker (ae:4200-4207, 4247-4255).
A failed pane query therefore renders every agent exactly like a healthy agent, while
JSON on the same failure emits `alive:false` under SC-017q: the two surfaces collapse
the same unknown in opposite directions. Successor IS at 92a20ee9: every absent runtime
member prints as `dead` (`src/listing.rs:240-267`). Empirical:
frozen and successor relations source-proven; successor end-to-end rendering pending.
Authority: commands.md@72c7293:56-59 + SC-017h + SC-017p/q + joint P1 ruling. Issue
#105 is the defect-family label, never normative authority. Conflict:
fix-known-defect(#105).
**classified_by:** SC-017p SC-017q SC-017r — per-agent liveness P1 ruling,
2026-08-21; fable5:lead + gpt56sol:colead. Exact enumeration; all three are bucket 3
fix-known-defect(#105). Knowledge, epistemic state, and human rendering are separate
grains; normative/conflict lane only, with empirical scope stated per row.**

**SC-017s — a pane-observed live predicate: the only ratified route to `alive`.**
Bucket 3 — fix-known-defect(#105). SC-017p grants `alive` only on an observation that
recognizes the agent as live, and no row defined one. This row supplies it, IN ONE
DIRECTION ONLY, from tmux format fields.

OBSERVATION — no new query and no new observation type: the pane enumeration SC-017p
already requires, one `tmux list-panes -s -t <exact session>` against the candidate's
POSITIVELY RECORDED server (SC-017k), reading `#{pane_dead}` and
`#{pane_current_command}` beside the pane's identity marker.

ASSOCIATION: the pane must carry a usable, unambiguous association to that exact roster
agent under SC-017p. Missing, unusable, duplicate, conflicting or ambiguous markers never
reach this predicate — they are SC-017q `unknown`.

PREDICATE: the pane proves `alive` iff `#{pane_dead}` is `0` AND
`#{pane_current_command}` is NOT a member of the closed shell set `bash`, `zsh`, `fish`,
`sh`, `dash`, AND THE EMPTY STRING. An empty or absent command reading is NOT alive: an
unreadable field is the absence of evidence, and reading absence as positive proof is
#105's own defect. The `pane_dead` conjunct is measured rather than theoretical — a pane
retained by `remain-on-exit` keeps reporting the exited process's command (measured: a
pane that ran `true` reports `pane_dead=1` with `pane_current_command=true`), so the
command field alone would prove a DEAD agent alive. Neither ae@72c7293 nor the successor
sets `remain-on-exit`, so the hazard is operator-configurable rather than default; the
guard costs one more field in a query already being made.

DIRECTION: this row grants `alive` ONLY. A shell foreground command proves nothing — not
`dead` — and leaves the agent `unknown` under SC-017q. The watchdog's dead test is a
CONJUNCTION of shell-foreground and no-agent-descendant; negating one conjunct is sound in
one direction only, so the alive half stands alone while the dead half does not. A
symmetric row would re-import the unratified SC-906 and, with it, a process-ancestry
observation this row does not make.

CAPABILITY: this row observes TMUX FORMAT FIELDS and asserts nothing about processes,
process ancestry, or descendants. It neither requires nor grants any process-inspection
capability.

KNOWN FALSE NEGATIVE, accepted and recorded so a measurement of it is not reported as a
violation: under SC-812 a `cmd || fallback` resume chain leaves bash as the pane process,
so a genuinely live agent reports `bash` and lands in `unknown`.

WHY NOT-IN-SHELL-SET RATHER THAN RECOGNIZE-THE-TOOL: SC-705 measures that a real Claude
pane reports its VERSION STRING as `#{pane_current_command}`, and opencode reports
`opencode.exe`. A predicate that never has to recognize a tool cannot be broken by a tool
changing what it reports.

Frozen IS at 72c7293: VIOLATED — the shape exists in the list path (ae:4201-4206) but its
set OMITS the empty string that `command_is_shell` (ae:428-434) includes, so a failed or
absent read falls to the non-shell arm and yields a positive alive; there is no
`pane_dead` guard; and the map is keyed on `#{@ae_agent}` (ae:4207), the field SC-602
designates DISPLAY-only, when identity is `@ae_slot`. That map is also NAMED `_alive`
while storing its marker for SHELL panes — anyone citing it as precedent must read the
case arms, not the identifier. Successor IS at 92a20ee9: ABSENT — listing runtime is
constructed with no agents (`src/listing.rs:139-151`), so there is no alive route to
violate. Empirical: observed(docs/migration/evidence/sc-017s/, 2026-08-21 — throwaway
`tmux -S` server, marker-uniqueness instrument check, then non-shell foreground -> alive,
shell foreground -> unknown, empty reading -> unknown; the ae:4201-4206 set reproduced
turning an empty reading into a positive alive). Authority: SC-017k's
own-server/exact-identity rule + SC-017p's exact-association rule + SC-705 and SC-812 (the
contract's existing recognition of `#{pane_current_command}`) + ae@72c7293:428-434
(`command_is_shell`) as the predicate + ae@72c7293:4201-4206 as the ENUMERATION precedent
ONLY + joint P1 ruling 2026-08-21. Deliberately NOT watchdog.md: that is SC-906's
authority, and citing it would recreate the dependency this row exists to remove.
Conflict: fix-known-defect(#105).

**SC-020a — `next --attach` switches inside tmux, attaches outside.** Bucket 2 —
`tmux switch-client` when already in tmux, `tmux attach-session` otherwise; `--switch`
is its alias. Authority: commands.md:155-157. Empirical: pending. Conflict: none.

**SC-021 — `ls` is an alias of `list`.** Bucket 2 — same operation, both spellings
(SC-011/SC-019 precedent). `ls` appears NOWHERE in commands.md — a doc gap worth
closing bash-side or noting at P5. Authority: the joint S1 spelling-alias ruling
+ the ratification-day S1MAP `ls ->` inventory line (colead countersign
precision — never inventory alone). Empirical: observed(dispatcher `list | ls)`
ae@72c7293:16663). Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-022 — the usage-error surface: unknown OPTIONS are refused loudly on stderr.**
Bucket 2 — an unknown top-level OPTION (a `-`/`--` token the dispatcher does not
define) and an unknown `list`/`ls` TAIL token are usage errors: diagnostic on
STDERR, stdout EMPTY, exit 2 (the crate exit contract keeps 2 distinct from 1).
SCOPE, ruled precisely (colead veto of the lead's broader draft): a top-level
NON-option token is a session-name/launch candidate under the S1 start grammar —
it is NEVER an unknown-subcommand error, and no such phrase becomes contract.
Trailing tokens after `help`/`version` are explicitly UNRULED — out of any
acceptance claim until their own row exists; silently ignoring them is not a
pinnable behavior. Authority: joint seat ruling (s2 Q3 + colead refinement,
2026-08-20) + the crate exit-code contract; SC-508 (residual exit codes) stays
its own code-observation row at final grain. Empirical: pending (rust-side
acceptance; bash incumbent measurable later). Conflict: none. **classified_by:
both seats, 2026-08-20.**

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
S1MAP: list -> SC-017i SC-017a SC-017b SC-017c SC-017d SC-017e SC-017f SC-017g SC-017h SC-017j SC-017k SC-017l SC-017m SC-017n SC-509 SC-509d SC-506 SC-1306a SC-521c SC-017o SC-017p SC-017q SC-017r SC-017s SC-509e
S1MAP: ls -> SC-021 SC-017i SC-017a SC-017b SC-017c SC-017d SC-017e SC-017f SC-017g SC-017h SC-017j SC-017k SC-017l SC-017m SC-017n SC-509 SC-509d SC-506 SC-1306a SC-521c SC-017o SC-017p SC-017q SC-017r SC-017s SC-509e
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
send delivers" §1 (dead-pane refusal: a shell pane is refused with the named reason;
nothing is pasted). Empirical: pending. Conflict: none.

**SC-202 — a human's unsent input is never clobbered.** Bucket 1 — survives DR-004:
injection into a modelled TUI defers fail-closed while the input box is non-empty,
mid-generation, or unreadable, and abandons loudly rather than clobbering. Authority:
helpers.md "How send delivers" §2 (busy/human-input defer, fail-closed; abandons
rather than clobbering). Empirical: pending. Conflict: none.

**SC-203 — delivery uncertainty is typed, never silent.** Bucket 1 — survives DR-004:
submit is verified after injection (bounded nudges) and unconfirmable delivery is a
LOUD typed outcome; after P2 the uncertainty applies to the NOTIFICATION only, while
the stored body remains readable regardless. Authority: helpers.md "How send
delivers" §3 (submit verification, bounded nudges, loud UNCONFIRMED) + DR-004
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
Authority: helpers.md "How they compose" (send is the only helper that pastes into a
pane) + gate ruling. Empirical: census-1. Conflict: none.

**SC-206 — request ids have exactly one minting authority.** Bucket 1 — every request
id is minted by a single authority; no second mint path exists. (Frozen source, IS
explanation only — not the SHOULD: at 72c7293 that authority is the shared
`ae_tracked_send` in `_lib`.) Authority: helpers.md "How they compose" (only one path
mints request ids). Empirical: pending. Conflict: none.

**SC-207 — one validator pairs replies.** Bucket 1 — reply pairing is verified in one
place before delegation to send. Authority: helpers.md "How they compose" (reply
looks up the original request via one validator before delegating to send).
Empirical: pending. Conflict: none.

**SC-208 — every accepted delivery crosses one emit point.** Bucket 1 — every
ACCEPTED message delivery is auditable in events.jsonl through the shared
event-emission contract. Refused, abandoned, and unconfirmed attempts are loud typed
errors (SC-201/202/203), not events: the frozen emitter fires only after verified
submit, so failed attempts are deliberately absent from the log. Authority:
helpers.md "How they compose" (same emit point), narrowed to accepted deliveries by
seat ruling ae-20260820T163523Z-e935697d. Empirical: code-read ae:14283
(`ae_emit_event` strictly after the `ae_submit_pasted_message` guard; UNCONFIRMED
exits before emit); probe pending. Conflict: none.

**SC-209a — requests and replies are addressed by slot + session.** Bucket 1.
Authority: helpers.md "Slot identity" (the slot is the routing key: requests and
replies are addressed and verified by slot + session). Empirical: pending.
Conflict: none.

**SC-209b — reply verifies the sender's live slot against the stored slot.** Bucket 1
— before delivering. Authority: helpers.md "Slot identity" (reply checks the sender's
live slot against the request's stored slot before delivering). Empirical: pending.
Conflict: none.

**SC-209c — the display name is never trusted for routing.** Bucket 1 — `--as` sets
display only and cannot bypass slot verification. Authority: helpers.md "Slot
identity" (the name is display only; a name passed with `--as` is shown but never
trusted for routing). Empirical: pending. Conflict: none.

**SC-209d — routing survives display-name churn.** Bucket 1 — a reply reaches the
right agent after its display name changes. Authority: helpers.md "Slot identity" (a
reply reaches the right agent even after its display name changes).
Empirical: pending. Conflict: none.

**classified_by (S3 delivery/routing MARK batch 1, ae-20260820T163523Z-e935697d):
SC-201, SC-202, SC-203, SC-205, SC-206, SC-207, SC-208, SC-209a, SC-209b, SC-209c,
SC-209d — fable5:lead + gpt56sol:colead, 2026-08-20. Exact enumeration; later rows
never inherit this mark. All bucket 1, conflict=none. Marked with the colead
conditions applied first: SC-206 rewritten outcome-level (one minting authority; the
bash symbol demoted to frozen-source explanation) and SC-208 narrowed to accepted
deliveries. Normative/conflict lane only; Empirical remains pending where so
marked.**

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
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table row :105 + prose :111 ("`agents --all` lists agents across all running ae sessions") — one optional-flag signature, jointly supported (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212f — `focus <agent>` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212g — `interrupt <agent> [message]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212h — `spawn <alias:name> [prompt]` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212i — `retire <agent|pane-id>` signature.**
  Bucket 2. Authority: AGENTS.md@72c7293 table row :109 + name-resolution prose :111 (`%pane-id`) — one CLI-signature claim: every helper taking an agent argument resolves `%pane-id`; the spawned-only semantic is SC-212q's, not this row's (frozen helper docs). Empirical: pending. Conflict: none.
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
  Bucket 2. Authority: helpers.md@72c7293:77 Inspection table (peek: "default 80, max 2000") — re-anchored: the AGENTS.md table documents only the default; the max appears nowhere in AGENTS.md (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212p — `interrupt` with no message cancels only.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212q — `retire` acts on spawned agents only.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212r — `say` emits a chat event.**
  Bucket 2. Authority: AGENTS.md@72c7293 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.
**SC-212s — `state <working|waiting-user|blocked|done> [reason]` signature** (gate
correction: documented — not code-observation).
  Bucket 2. Authority: AGENTS.md@72c7293:96 session-helpers table (frozen helper docs). Empirical: pending. Conflict: none.

**classified_by (S3 helper-signature MARK batch 2A, ae-20260820T164044Z-f958a368):
SC-212a, SC-212b, SC-212c, SC-212d, SC-212e, SC-212f, SC-212g, SC-212h, SC-212i,
SC-212j, SC-212k, SC-212l, SC-212m, SC-212n, SC-212o, SC-212p, SC-212q, SC-212r,
SC-212s — fable5:lead + gpt56sol:colead, 2026-08-20. Exact enumeration; later rows
never inherit this mark; SC-211a..p (Batch 2B) are EXCLUDED — they remain
code-observation rows awaiting accepted IS plus a preserve/fix/diverge seat ruling.
All bucket 2, conflict=none. Marked with the countersign conditions applied first:
SC-212o re-anchored to helpers.md@72c7293:77, SC-212e/SC-212i composite anchors
(table row + :111 prose), SC-212s normalized into one contiguous row.
Normative/conflict lane only; Empirical remains pending throughout.**

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
`.started`) is consumable by the successor. Authority: two explicit lanes —
architecture.md@72c7293:61-71 (the documented frozen layout) + joint family-gate
ruling f869b66/#79/#81 (legacy readability across flips). The exact full artifact
inventory (notably messages/locks/`.started`) remains empirical census scope, never
promoted from measurement. Empirical: census-1/2. Conflict: none.

**SC-400b — the event store's written layout changes under DR-001.** Bucket 4 —
**DR-001**: generations replace the single `events.jsonl`, with legacy-read/migration/
write ownership stated at the flip commit. Authority: DR-001. Empirical: n/a
(successor design). Conflict: DR-001.

**SC-400c — generated-logic helpers retire from the written layout at P2.** Bucket 4 —
**DR-006** (gate ruling: every b4 carries a DR; the epic is authority, not a waiver).
Authority: DR-006 + epic #79 P2 + #76. Empirical: n/a (successor design).
Conflict: DR-006.

**SC-400d — durable current-session inventory has two readable state layouts.** Bucket
2 — a durable candidate is a state directory at either
`<AE_HOME>/sessions/<session-name>/` (canonical) or
`<AE_HOME>/worktrees/<worktree-name>/.ae/<session-name>/` (legacy worktree-nested).
The `<session-name>` leaf is the candidate's inventory name; the root-qualified state
directory is its discovery identity/provenance, so equal leaves across paths never
deduplicate candidates. Presence of the state directory is sufficient for discovery:
missing or unreadable `meta` loses/degrades facts under SC-405i/SC-509b but cannot remove
the candidate. A bare worktree directory without the nested state directory is not a
session candidate. Archives remain inert and excluded by SC-017j. Authority: SC-400a's
legacy-read promise + joint P1 ruling. Empirical: source-proven legacy shape — frozen ae
resolves `${WORKTREES_DIR}/${name}/.ae/${name}` at ae:3200-3209 and independently scans
`${WORKTREES_DIR}/*/.ae/*/meta` at ae:8893, taking the inner directory leaf as the
session name; successor end-to-end inventory pending. Conflict: none.
**classified_by:** SC-400d — two-root P1 inventory ruling, 2026-08-21;
fable5:lead + gpt56sol:colead. Bucket 2, conflict none.

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
project-side file). Authority: AGENTS.md@72c7293:19-21 Rules bullet ("Working
directories stay clean"). Empirical: pending. Conflict: none.

**SC-403 — record framing round-trips every field faithfully.** Bucket 1 — semantic
(gate correction — the `\x1f` choice is the bash mechanism, empirical): an empty
field, free text with separators, and embedded-newline handling all round-trip without
field shift or phantom rows; typed Rust satisfies this by construction. Authority:
AGENTS.md@72c7293:188-199 TSV-framing section (the invariant bullets behind the
`\x1f` mechanism). Empirical: unit pins @72c7293
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

**SC-405l — a durable tmux-server selector normalizes to a typed knowledge fact.**
Bucket 2 — every durable candidate exposes exactly one of
`positive(name:<nonempty>)`, `positive(socket:<absolute-path>)`, `missing`, or
`ambiguous`. Only `positive` confers SC-017j server entitlement or can support SC-017k
liveness; `missing` and `ambiguous` leave the candidate inventoried and route liveness
through SC-017l `unknown`.

The well-formed bash-era two-key read mapping is exact: one
`tmux_server_kind=name` plus nonempty `tmux_server` is `positive(name)`;
`kind=socket` plus a nonempty absolute value is `positive(socket)`;
`kind=ambiguous` is `ambiguous`; and an **absent** kind plus a nonempty value is the
legacy `positive(name)` form. A selector with no value and no nonempty kind is
`missing`. Every other selector-level combination — including an unknown kind, typed
empty value, non-absolute socket, present-empty kind beside a nonempty value, or
duplicate/conflicting selector keys — is `ambiguous` and MUST NOT confer entitlement.
Whether malformed selector bytes also mark the record `degraded` remains
SC-405e/SC-509b's separate question.

`missing` means that no selector fact is available to the reader; it is not a claim that
readable bytes positively omitted the selector keys. An absent or unreadable `meta` record
therefore normalizes the selector to `missing` AND independently carries the phase-1
record-read-loss fact governed by SC-405i/SC-509b. A readable record with no selector also
normalizes to `missing` without that loss fact. `ambiguous` remains reserved for selector
bytes that were readable but do not admit one positive mapping. Selector knowledge and
record-read/degradation knowledge are orthogonal; neither substitutes for the other.

This row defines READ normalization only. A successor writer owes a separately
ratified, round-tripping encoding before it emits this fact. It supersedes SC-405d's
catch-all treatment only for the exact `tmux_server` / `tmux_server_kind` family in the
P1 successor; every other unknown key remains tolerated and uninterpreted, and
SC-405b/c remain true rather than exhaustive. Authority: SC-400a legacy readability +
SC-017j/k/l + joint P1 ruling. Empirical: frozen ae's released reader treats a
kind-absent nonempty value as `-L <name>` and explicit `ambiguous` as no target at
ae:7594-7599; the typed writer emits the two keys at ae:17558-17559. Those bytes prove
the legacy mapping, not the successor write format. Successor read-path capture pending.
Conflict: none.
**classified_by:** SC-405l — durable-server normalization ruling,
2026-08-21; fable5:lead + gpt56sol:colead. Bucket 2, conflict none.

**SC-405i — a present session dir with MISSING meta is degraded.** Bucket 2 —
(slice-1b Q8): identity beyond the directory name and the entire roster are lost at
once — actual loss by SC-509b's own test; distinct from missing/empty EVENT logs,
which SC-519 makes quiet. Authority: slice-1b joint ruling + SC-509b. Empirical:
pending (C-cluster). Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-405j — an event carrying ANY routing key that does not fully and freshly match
stays UNASSOCIATED.** Bucket 2 — (slice-1b Q10, colead dissent adopted; PRECISED after
the builder's premise correction; REOPENED AND PRECISED AGAIN by the empty-member
ruling, slice-1d): PRESENCE is decided BEFORE any empty-string normalization — a
routing key member that appears in the record's JSON is PRESENT even when its value
is the empty string. Structurally ABSENT routing keys (no member at all) permit the
legacy display fallback (pre-SC-511a records depend on that surviving); ANY present
routing member that does not fully and freshly match — stale full keys, mismatched
keys, partial keys (slot without session or session without slot), and any present
EMPTY member — makes the keyed identity invalid and UNASSOCIATED. Readers never
erase a present ROUTING member into structural absence — this prohibition is
scoped to the four routing keys; the SC-510b trio's reader-side empty-as-omission
stands unchanged, and SC-510b's/SC-511a's producer rules are authority for their
own surfaces only (scoping clarification after the builder flagged the unscoped
sentence would contradict the same ruling's trio preservation). Evidence and
tests carry the
three-way discriminator: keys absent / one present-empty member / all-present-empty.
Display fallback for keyed events would create FALSE ATTRIBUTION against the
SC-518/SC-511b loud direction; rename loss is the KNOWN LIMITATION until SC-977's
P2 stable identity. One shared invariant — a total association decision function —
so one row remains valid grain; builder tests are candidate successor evidence,
never frozen IS. Authority: slice-1b + slice-1d joint rulings + SC-518/511b
direction. Empirical: pending. Conflict: none. **classified_by: REOPENED by the
slice-1d precision and RE-MARKED on this exact text — fable5:lead + gpt56sol:colead,
2026-08-20.**

**SC-405k — agents[] membership is roster-defined.** Bucket 2 — (slice-1b Q11):
runtime-only panes/slots never invent agents; SC-509's agents[] fields are roster
fields; a missing roster/meta routes through SC-405i. Authority: slice-1b joint
ruling. Empirical: pending. Conflict: none. **classified_by: both seats, 2026-08-20.**

**SC-405e — malformed/duplicate key handling.** `authority=code-observation` — probe +
seat closure; never guessed. UNCLASSIFIED pending closure.

**SC-405f — `goal_set_epoch` is the `ts` of the LAST ACCEPTED goal event in canonical
logical event-stream order.** Bucket 2 — not a meta key; the digest derives it from the
event stream. **REOPENED AND PRECISED** (both seats, 2026-08-20) after A7's opposed-order
arm exposed that "latest" was undecidable.
**The precision:** the epoch is the `ts` of the last accepted goal event in **canonical
logical event-stream order** (generation + offset under DR-001) — **not** the numerically
greatest `ts` among goal events.
**This is not IS becoming SHOULD.** The frozen authority already establishes the ordering
model, and this row merely inherits it: `events.md:3` — *"Append-only, one JSON object per
line"*; `events.md:7` — *"single writer … the append-only structure plus flock-serialized
writes mean readers can scan safely"*; and decisively `events.md:110-128`, which **defines
latest by BACKWARD STREAM SCAN** (*"walks `events.jsonl` backward via `tac`"*, *"The latest
`ask` / `review` event per `ref`"*, *"Scan is newest-first via `tac`"*). `commands.md:125-126`
defines the field as *"when the goal was last set"*. A max-timestamp fold would invent
clock-order semantics absent from every one of those authorities and would let clock skew
reorder committed state. SC-1300 + DR-001 preserve one logical total order across
generations. A7 then resolves the ambiguity empirically: the incumbent IS last-appended.
**Rationale, with the lead's first wording CORRECTED by colead:** append order is the
system's **canonical committed serialization order**, and its virtue is narrow and exact —
*it does not depend on clock correctness*. It is **not** "causal order" (concurrent
producers have no causal relation established by stream position) and it **cannot** be
called unforgeable (a writer or operator can alter ledger position). The lead's original
"causal order … cannot be forged" overclaimed on both counts.
Concurrency makes this real rather than hypothetical: the writer stamps `ts` at **ae:13214**
but the flock is taken only inside `ae_log_append` at **ae:13174**, with the append at
**ae:13256** — so two concurrent emitters can stamp in one order and append in the other.
**The RENDERED goal age is semantically scored, per SC-017e.** Once this row fixes
which event supplies the epoch, the human table's relative rendering of that epoch
is scored under SC-017e's single-witness-epoch mechanism — jointly with the active
age, against one `t` in the recorded bracket, with per-span independent scoring
explicitly forbidden there. This row supplies the epoch; SC-017e governs how its
rendering is compared.
**SCOPE GUARD — required in-row.** This chooses **which EVENT supplies the epoch**. It does
NOT bless goal/meta versus event-store tearing and does NOT claim both fields come from one
snapshot; meta and events take **separate locks**, and that atomicity/coherence question is
owned by **D08 + SC-1306a**. Without this guard the precision would accidentally ratify a
distinct hole.
Authority: `events.md:3,7` + `events.md:110-128` + `commands.md:125-126` + slice-1 Q1
ruling + SC-1300/DR-001. Empirical: **observed** (A7 opposed-order arm, plus its AGREEING
control and single-goal baseline — the opposed fixture makes the two candidate answers
different strings by construction, so a last-record reader and a max-timestamp reader
cannot both be right). Conflict: none.
**classified_by: REOPENED by the A7 order finding and RE-MARKED on this exact text —
fable5:lead + gpt56sol:colead, 2026-08-20.**

**SC-405g — `branch` is the live tmux branch with a git fallback.** Bucket 2 — not a
meta key; per commands.md:124-129 (the watchdog's status segment, git fallback).
**TEMPORARY EXCEPTION to SC-509b's per-member presence rule** (colead ruling,
2026-08-25, option (a)). UNTIL the source-acquisition slice for this row lands,
`sessions[].branch` ALONE retains its predecessor projection when NO branch
observation exists: `null` on a non-degraded entry, ABSENT on a degraded entry. An
OBSERVED branch renders regardless of `degraded`.
**Why the exception rather than simply conforming.** The acquisition source named
by this row — the watchdog status segment, with the git fallback — is not yet
wired, so a `None` here does not distinguish "the source was read and reported no
branch" from "the source was never read at all". Rendering `branch: null` on a
degraded entry would therefore masquerade an UNAVAILABLE source as a legitimate
empty: it removes the aggregate-erasure symptom by making a different false claim,
which is the trade SC-509b exists to refuse. Retaining the predecessor's two shapes
is the honest holding position until the source exists to be asked.
**Scope, stated so it cannot be borrowed.** This is temporary byte compatibility. It
is NOT evidence that `degraded` identifies `branch` loss — `degraded` remains
aggregate visibility and identifies nothing per member. It is NOT precedent for any
other member: every other optional member's presence is decided by its own source's
provenance, with no appeal to this paragraph.
**While the exception stands, `branch` VALUE is UNSCORED across the P1 DIGEST
comparison under `OC-P4-BRANCH-VALUE` and across the P1 HUMAN comparison under
`OC-P4-HUMAN-BRANCH-VALUE`.** Prose alone
would not carry it: the closed phase-4 open-choices register is the only
product-output exclusion the gates honour, so an exemption asserted here and
unregistered there is residual divergence. The human surface joined by amendment
(colead measurement, 2026-08-25) on the authority this clause promised, not by
assumption. TWO CHOICES AND TWO REGISTER ROWS, one per surface, under two DISTINCT
ids — two populations, two predicates, two names, because a folded count cannot say
which surface moved. The ids are distinct rather than one id spanning two rows
because a register id is unique by gate criterion 8: two rows sharing one id is a
schema the register cannot hold, and the digest id is preserved unchanged so
existing digest obligations do not churn.
**The partition is ENUMERATED BEFORE THE SUCCESSOR RUNS, at the session-subline
occurrence grain.** Among the fixed P1 human occurrences whose FROZEN subline
carries a `git:` atom, the contract-derived presence-loss set MUST omit that atom,
and its COMPLEMENT is the VALUE set, which MUST render exactly one syntactically
valid, NONEMPTY `git:<value>` atom. An occurrence whose frozen subline carries NO
`git:` atom is in NEITHER set and MUST remain atom-free under default parity, which
this row does not govern. Membership is derived from the frozen corpus plus this
row's temporary projection rule — both fixed before any successor output exists.
**ACTUAL OUTPUT NEVER SELECTS ITS CLASS.** A rule reading the class off what the
successor rendered would let a defective successor choose which obligation it owes:
omit the atom and be scored a presence move, emit one and be scored an unscored
value. The class is an INPUT to scoring, never an output of it. The occurrence sets
live in the register, never here.
**ATOM SHAPE is not the value, and only the VALUE is open.** Which bytes the
required atom carries is the open choice; that it is present, single and nonempty
across the VALUE set is not. Mandating a particular placeholder here would
contradict declaring the value open in the same breath — the successor's current
placeholder is implementation EVIDENCE, not a mandated byte, and it becomes one
only if the open choice is deliberately narrowed by its own amendment.
**What the register must STILL assert.** The exclusion is the value bytes and
nothing else, so BOTH choices' `STILL_REQUIRED` facts carry the atom-shape rule
above on the human side and this row's presence discipline on the digest side. An
implementation that dropped the atom from a non-loss row, emitted an empty one, or
emitted more than one fails the register with the value still open. The successor renders `null` on a healthy entry
because no observation exists to render — production constructs its runtime with
`branch: None` unconditionally, and only the acquisition slice changes that. The
predecessor renders the branch its live acquisition recorded. Neither is scored
against the other for the value.
**Branch PRESENCE is policed independently of this exception**, by the DERIVED
presence-only obligations across the loss population — the ones carrying the
predecessor's recorded value or `null` to ABSENT. That policing is not granted by
this paragraph and this paragraph is not its authority; the exception governs one
thing only, whether the VALUE bytes are compared. The occurrence COUNT lives in the
obligation table and the register, never here: a cardinality written into a row goes
stale on the next derivation and then reads as a rule.
**Why the values are unscored rather than reconciled either way.** An implementation
gap is not a ruling, so it may not mint a MANDATED divergence — scoring the
successor's `null` as correct would ratify the absence of a source this row
requires. Nor may the reverse be written down: a clause asserting `null` forever
would gate against this contract's own end state on the day acquisition wires, and
the retirement trigger below would then have to fight the row that survives it.
Unscored is the only reading that is true now and still true afterwards.
**RETIREMENT TRIGGER — the commit that wires this row's watchdog-status and
git-fallback acquisition.** From that identity onward, `branch` presence is governed
solely by branch-source provenance and `degraded` MUST NOT select it. This paragraph
AND BOTH register rows — `OC-P4-BRANCH-VALUE` and `OC-P4-HUMAN-BRANCH-VALUE` —
retire together with that commit, as ONE unit, and none of the three is to be
re-derived. Retiring any one alone leaves the others asserting a state that no
longer exists.
Authority: commands.md:124-129 + colead ruling 2026-08-25. Empirical: pending.
Conflict: none. **classified_by: both seats, 2026-08-20; temporary exception
RE-MARKED 2026-08-25 — gpt56sol:colead ruling, drafted opus5:reason2.**

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

**classified_by (S6 gate — EXACT FROZEN SET, range form retired): SC-500, SC-501,
SC-502, SC-503a, SC-503b, SC-504a, SC-504b, SC-505a, SC-505b, SC-506, SC-507a,
SC-507b, SC-507c, SC-507d, SC-509, SC-510a, SC-510b, SC-510c, SC-510d, SC-511a,
SC-511b, SC-511c, SC-512, SC-513a, SC-513b, SC-513c, SC-514, SC-515a, SC-515b,
SC-515c, SC-516, SC-517a, SC-517b, SC-517c — fable5:lead + gpt56sol:colead,
2026-08-20. SC-508 is explicitly UNCLASSIFIED until its evidence probe + joint
closure. PROCESS RULE (colead, declaration-normalization ruling): a range or
family mark is FROZEN to the exact row set existing at its ruling; rows added
inside the range later NEVER inherit the mark — each carries its own
classified_by or none. Historical-membership audit (colead): SC-510c WAS present
at the S6 gate (76722eb fold, f4e93ef ratification) and is enumerated for
exactness — its later slice-1 Q3 amendment is governed by its own per-row
re-mark, which this historical mark does not authorize; SC-509b post-dates the
gate (slice-1 Q5) and is excluded — its own per-row mark governs; SC-510e/f
(slice-1d) likewise carry their own marks.**

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
fabricated, never null; `agents` remains an array. **The omission clause is
LOSS-ONLY** (scope stated 2026-08-24, no change of meaning): it licenses omission
for a fact that could not be READ, and never for a fact that was read and is
legitimately empty — see the SC-017g presence precision, which turns on this row's
own closing sentence.
**PRECISED 2026-08-24 (colead) — `degraded` IS AGGREGATE VISIBILITY, NEVER
PER-MEMBER EVIDENCE.** The flag says this entry lost something; it does not say
WHICH member was lost, and it may not be used to select any member's omission. A
digest that omits every optional member because one source failed erases facts it
read perfectly from the OTHER sources. Presence is decided PER MEMBER by the
provenance of the member's own source: a known value renders, a known legitimate
empty renders its empty (SC-509's presence rule), and only an UNREADABLE source
omits. Unrelated loss cannot erase a known fact.
**ONE NAMED EXCEPTION EXISTS, and it is temporary.** `sessions[].branch` is carved
out at SC-405g until that row's source-acquisition slice lands, because its
acquisition source is not yet wired and a `null` there would assert a legitimate
empty about a source never read. The carve-out is byte compatibility with the
predecessor, is NOT evidence that `degraded` identifies branch loss, and is
precedent for nothing else. This cross-reference exists so the prohibition above is
never read as unconditional while a conforming implementation contradicts it — and
so that retiring the exception at SC-405g restores it to unconditional here.
**The attention members are exact-or-omitted, and `needs_attention` is not one of
them.** `attention` and `attention_rank` — and `agents[].reason` — render only when
the answer is EXACT given the inputs actually read; when a lost input could change
the reason or the rank, those members are OMITTED. `null` is unavailable here
precisely because SC-017g defines `null` as read-and-quiet, so writing it under
uncertainty would assert the very thing that was not established.
**`needs_attention` STAYS, as an ALWAYS-PRESENT PARTIAL-EVIDENCE INDICATOR.** It
renders always: `true` iff >=1 contribution remains established after reducing the
READABLE facts; `false` iff none remains established in those readable facts. Later
readable records may add, clear, or supersede a contribution, so more input may
change `true` to `false` as well as `false` to `true`. When the loss could affect the ATTENTION INPUTS, neither
boolean value alone proves the exact final attention: missing facts may add,
clear, or supersede a contribution. When SC-509b's exactness rule establishes the
maximum despite unrelated loss, the full triad remains exact — aggregate
`degraded` does not make a per-member answer uncertain. Either way,
`degraded: true` is the mandatory incompleteness qualifier.
**WHAT EXACT MEANS, and it is not "no loss anywhere".** SC-017g's marker is a MAX
across the session's agents, so exactness is a question about that maximum, not
about whether every byte was read. The rule: **the answer is EXACT iff every
relevant source is complete, OR the established values come from complete or
independently established sources and known UPPER BOUNDS on the missing sources
force one maximum.** UPPER BOUNDS constrain only contributions that missing
sources may ADD; they do not prove that a value from a partial source cannot be
cleared or superseded by a later readable record. There is no blanket
any-loss-implies-omit.
Three consequences, each a required discriminator (colead, 2026-08-24):
a READABLE EMPTY roster is FULLY ENUMERATED and supports exact quiet — only a
roster absent or unreadable THROUGH LOSS is unenumerable, because readable events
cannot prove that no unenumerated agent contributed;
an established **`dead`** stands despite other-source loss only when its OWN source
is complete or independently established; a `dead` derived from a partial source
can be cleared or superseded by a later readable record, so it is not exact merely
because it ranks first; a partial-ledger `dead` therefore omits exact
`attention`/`attention_rank` and `agents[].reason` when a later readable record could
clear or supersede it. A complete or independently established runtime hand-in
`dead` observation is the mandatory top-class canary for this rule;
and loss in a source that could not have changed the maximum leaves the triad
exact and present when the source establishing that maximum is complete or
independently established; a partial source never earns exactness from rank alone.
`agents[].reason` follows the same principle rather than a weaker one: an
`AgentEntry` proves roster MEMBERSHIP (SC-405k) and nothing about the completeness
of its own reason inputs, so it renders when its exact own contribution is
established from a complete or independently established source despite unrelated
loss. If its source is partial, a later readable record may clear or supersede the
earlier contribution (including `dead` or `blocked`), so `agents[].reason` omits
when a relevant missing input could change it.
**What frozen v1 does and does not witness.** Its `needs_attention: false` on a
loss entry is NOT defect evidence — under this row that boolean is the partial-
evidence proposition "no contribution remains established after reducing the
readable facts", which is exactly what the incumbent had grounds to say. When the loss could affect the ATTENTION INPUTS, neither
boolean value alone proves the exact final attention: missing facts may add,
clear, or supersede a contribution. When SC-509b's exactness rule establishes the
maximum despite unrelated loss, the full triad remains exact — aggregate
`degraded` does not make a per-member answer uncertain. Either way,
`degraded: true` is the mandatory incompleteness qualifier. Its `attention: null` and `attention_rank: 0`
ARE defect evidence wherever exact quiet or an exact maximum was not established,
because those spellings assert read-and-quiet (SC-017g) about inputs that were not
read. The incumbent could not have done otherwise — its emitter is one `printf`
with every member positional, so it has no way to omit and no way to distinguish —
which is why this row is the later explicit rule and takes precedence. Damage is never rendered
identically to legitimate sparsity — a machine digest that hides loss lies by
omission. Authority: slice-1 joint Q5 ruling + SC-509's `schema_version` consumer-
gating design (the events evolution rule SC-511c is a different schema and is NOT the
authority here). Empirical: pending (builder implementation + C-cluster).
Conflict: none.
**classified_by: fable5:lead + gpt56sol:colead, 2026-08-20.**

**SC-509c — `agents[].reason` carries the agent's own attention contribution.**
Bucket 3 — fix-known-defect(#97) (final-grain row added by seat read of ba95a5e):
SHOULD: agent-owned active contributions (dead, stale, waiting-user, blocked,
throttled) populate THAT agent's `reason`; `reason: null` means no agent-owned
contribution exists; session-only reasons (the aged-unanswered-request rank of
SC-017g) remain session-level and never fabricate a per-agent reason. IS at 72c7293:
every agent-owned reason renders null while session `attention` is non-null —
ae:3714's `[[ -n "${_areason+x}" ]]` is false for a declared-but-empty associative
array, so the FIRST contribution is never written (reproduced under
/opt/homebrew/bin/bash). Documented contract broken: commands.md:124-125 (each
agent's reason is its own contribution). Authority: commands.md:124-125 + seat read
ruling. Empirical: observed (A3/A3b attention-fields @ba95a5e — null per-agent reason
beside non-null session attention across all five reason classes; root cause
code-read ae:3714). Conflict: fix-known-defect(#97, intended: populate per-agent
reason; null = no contribution). SC-017g is NOT contaminated — its total-order IS is
closed by A3b. **classified_by: both seats, 2026-08-20 (seat-read finding adopted).**

**SC-518 — request closure requires the full mirror match.** Bucket 1 — (slice-1 Q6
seat ruling, reversing the lead's scope confirmation): an unanswered request closes
only on same ref AND reply.actor = request.target AND reply.target = request.actor;
routed identities (slot+session) compare when both sides carry them, display
identities when neither does, and MIXED identity matches nothing — the loud
false-pending direction is safer than silent false-closure by a reply sent to someone
else. Authority: events.md:108-117 (normative dependency of SC-017g) + joint ruling.
Empirical: OBSERVED (C-cluster A7 c12-c18 + A6 m6 + A1 c14, ro and rw).
Conflict: frozen bash closes five of the six mixed shapes.
**classified_by: fable5:lead + gpt56sol:colead, 2026-08-20.**

**Amended 2026-08-24 — Empirical is now OBSERVED and Conflict is no longer none. The
normative sentence above is UNCHANGED. MEASUREMENT below is exactly the C-cluster
inventory, the frozen readings, the two capture counts, and the frozen branch predicate.
The rest is NOT: "present-but-empty is not absent" and the named gap are RULINGS, labelled
as such where they appear. A heading cannot call a ruling a measurement, and it must not
identify its own scope by counting paragraphs either.**

The C-cluster holds one ask carrying all four routing members and varies only the REPLY's, across `ro`
and `rw` twins whose captures are byte-identical: A7 c12 all four matching, c13 all four
naming a different slot and session, c14 slot members only, c15 session members only, c16
none at all, c17 one member present as the EMPTY STRING, c18 all four present as empty
strings; A6 m6 pairs a routed ask with a keyless reply; A1 c14-display-only-legacy is the
corpus's only display-to-display pair.

MEASURED — and every case id below names its ARM, because A7 c14 and A1 c14 are different
cases that behave oppositely: frozen ae closes A7 c12, A7 c15, A7 c16, A7 c17, A7 c18,
A6 m6 and A1 c14, and pends only A7 c13 and A7 c14. SIX of those shapes are MIXED under
this row — a routed ask against a reply that is not fully routed — and none of them may
close. Frozen pends exactly one of the six (A7 c14) and CLOSES FIVE: A7 c15, A7 c16,
A7 c17, A7 c18 and A6 m6. Those five are the conflict; the row's strict reading stands
over them, and the two that behave (A7 c12 routed-to-routed, A1 c14 display-to-display)
are the only shapes entitled to close.

TWO NUMBERS, AND NEITHER IS AN OBLIGATION COUNT. The identity matrix is TEN SHAPES, each
with a `ro` and an `rw` capture, so TWENTY CAPTURED INVOCATION ROWS. SIX shapes move, so
TWELVE CAPTURED ROWS change bytes — A7 c15/c16/c17/c18, A6 m6 and A6 m2, in both twins.
Both numbers count CAPTURES. Neither may be used as a table count: the status token and the
displayed summary are separate obligations derived and addressed separately, so the
obligation-locus count over these captures EXCEEDS twelve and is whatever the derivation
emits. Counting captures and counting loci are different questions and the same arithmetic
answers neither.

ONE PREDICATE EXPLAINS ALL SEVEN READINGS, which is why this is one source defect and not
seven accidents. ae@72c7293:4551 selects the routed comparison iff `request.target_slot` is
NONEMPTY **and** `reply.actor_slot` is NONEMPTY — those two fields alone select the branch;
the other two routed members are compared inside it and never decide entry. So A7 c12 enters
and matches, A7 c13 enters and mismatches, A7 c14 enters and breaks on its absent session
half, while A7 c15, c16, c17, c18 and A6 m6 carry no NONEMPTY `reply.actor_slot` — c17 and c18
carry it as the EMPTY STRING, which `-n` rejects exactly as it rejects an absent one — never
enter, and fall back to the display comparison, which succeeds. The two cases that look backwards
(slot-only pends, session-only closes) are one selector seen from two sides.

PRESENT-BUT-EMPTY IS NOT ABSENT. c17 and c18 carry empty-string members and are MIXED, not
display; c16 omits them and is display. All three fail here only because the request side is
routed in all three. Against a DISPLAY-ONLY request they would diverge — keyless closing,
all-empty not — and the corpus has no such specimen.

THE NAMED GAP, and the reason "in BOTH directions" is a ruling rather than a reading: every
mixed specimen in this corpus mixes in ONE direction, a routed ask against an under-routed
reply. The INVERSE — an under-routed ask against a fully routed reply — has ZERO specimens.
The symmetry is ruled and pinned by successor test, never latitude: a successor test that
exercises only the direction the corpus owns would pass an implementation that is
directional.

**SC-518a — closure ordering is its own dimension.** Bucket 1 — (2026-08-24 joint
ruling): a `reply` or `cancel` terminates ONLY THE NEWEST PRECEDING `ask`/`review` carrying
that ref, where PRECEDING means an EARLIER COMPLETE RECORD IN APPEND (LEDGER) ORDER. A
TIMESTAMP NEVER ORDERS A LIFECYCLE: `ts` is a writer's clock, and skew must not be able to
carry a terminal across an opening. The ledger is the order — events.md:110 has the reader
walking `events.jsonl` backward — so "newest preceding" is a position in the file, not a
comparison of clocks. A terminal that precedes its opening closes nothing; causality is not
a matching condition that ref equality can satisfy. A later re-`ask` on the same ref opens a NEW
lifecycle, and an earlier terminal cannot reach forward to close it. This row governs ORDER ONLY, and
AUTHORIZATION IS NOT SYMMETRIC BETWEEN THE TWO TERMINALS. A `reply` must ADDITIONALLY satisfy
SC-518, whose unchanged sentence is a two-ended mirror over `reply.actor`/`reply.target`. A
`cancel` has no target end and cannot satisfy that sentence, and NO ROW IN THIS CONTRACT
DEFINES CANCEL AUTHORIZATION — SC-830 withdraws outstanding work under `--digest-only` and
says nothing about which cancel event a reader accepts. So: SC-518a CONSTRAINS ORDER ONLY AND
DOES NOT AUTHORIZE A CANCEL; cancel authorization requires a separate row or ruling, which is
deliberately NOT added here. What this row does say about `cancel` is one-directional and
safe: any otherwise-authorized cancel still cannot close an opening that FOLLOWS it. A
successor test over the cancel gaps can therefore prove CAUSALITY CONDITIONAL ON
AUTHORIZATION and can never prove authorization itself. SC-206's single minting authority
makes ref reuse rare but does not authorize a pre-opening terminal; scarcity of collisions is
not a causality argument. Authority: events.md:108-117 + joint ruling. Empirical: OBSERVED (A6 m2,
ro and rw). Conflict: frozen bash matches on ref alone with no ordering test.
**classified_by: fable5:lead + gpt56sol:colead, 2026-08-24.**

MEASURED, one specimen, STATED IN LEDGER POSITIONS: in A6 m2's fixed `events.jsonl` the
`reply` carrying ref `review-20260820T161305Z-dc302d09` is at LEDGER LINE 2 and the `review`
that OPENS that ref is at LEDGER LINE 4. The terminal precedes its own opening by two
records. (Their `ts` values, 16:13:04Z and 16:13:06Z, agree with that order here and are
DESCRIPTIVE ONLY — had they disagreed, the ledger would still decide.) Frozen renders that review `replied`
and displays the REPLY's summary. Under this row it is `pending` and displays the review's
own summary. Note what frozen already does correctly, because it bounds the blast radius:
the row's FROM/TO columns show the OPENING's participants even while its summary shows the
terminal's, so a status moving back to pending carries the summary with it and moves nothing
else.

THIS ROW OWNS EXACTLY TWO RATIFIED ZERO-SPECIMEN GAPS, and the count is stated that way
because an earlier "three" invited them to be read as three ORDERING gaps and a downstream
assignment duly miscounted them. The two are: (1) RE-ASK AFTER TERMINAL — the re-ask is
pending and the earlier lifecycle stays closed, and a successor test must separate this from
the identity rule or it cannot tell which one it exercised; (2) CANCEL CAUSALITY, and only
CONDITIONAL ON AUTHORIZATION — a cancel before its opening closes nothing, while whether any
given cancel is authorized at all is unruled here.

TWO MORE GAPS ARE NAMED NEARBY AND NEITHER BELONGS TO THIS ROW. INVERSE-MIXED IDENTITY is an
SC-518 IDENTITY gap, not an ordering one, and is owned there. And note what is NOT a gap: A6
m2's pre-opening terminal is OBSERVED, the specimen this row is built on. The last is an
ordering question but is UNRATIFIED, so nothing may pin a winner on it: a `cancel` AND a `reply` both carrying one ref, both AFTER the
opening. The newest-preceding rule decides which OPENING a terminal attaches to, not which of
two terminals wins on one opening, so that case is UNDECIDED by this row. It cannot be closed by
evidence either: the 6862-file corpus contains ZERO `cancel` events (measured), so no capture
will ever speak to it — which is why what follows is read from the frozen SOURCE and not from
any capture, and is recorded as an IS rather than an OUGHT.

FROZEN'S CANCEL BEHAVIOUR, MEASURED AT ae@72c7293 AND RATIFIED BY NOTHING. Authorization
(4567-4589): only the request's own SENDER may withdraw it, checked as the ACTOR HALF of the
question the reply asks — routed when `request.actor_slot` and `cancel.actor_slot` are both
nonempty (comparing actor_slot + actor_session), display otherwise. Precedence (4591-4599):
recency decides WITHIN a kind, and BETWEEN kinds a valid cancellation wins, on the stated
reasoning that a straggler reply must not reopen a request nobody is waiting on.

BOTH READINGS ARE MEASURED IS, NEVER RATIFICATION, and neither may be asserted as SC-518a
behaviour. The authorization rule is recorded precisely because NO ROW DEFINES IT: the
implementation has a policy the contract does not, which is the concrete form of the gap
named above and the reason a cancel-authorization ruling is owed rather than optional. The
precedence is recorded so that "undecided" plus a running implementation cannot make whichever
arm someone writes the answer invisibly.

WHAT A SUCCESSOR TEST MAY DO WITH THEM, and the distinction is ENFORCEMENT, not labelling: a
GATING test MUST NOT assert either unratified outcome — authorization or precedence — because
a gating test that fails under the other policy has ratified one choice BY ENFORCEMENT
whatever its comment says, and "recorded, not normative" is not a property a merge gate can
honour. A gating test MAY exercise the shared attachment and causality mechanics in an
OUTCOME-NEUTRAL way, since those are ruled. A clearly NON-GATING diagnostic may record the
current IS. A seat ruling remains the only thing that makes either outcome normative.

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

**SC-521a — cross-dimension filter combinations intersect literally.** Bucket 3 —
fix-known-defect(#96) (reclassified from bucket 2/conflict=none by seat ruling, colead
phase read 2026-08-20, on A2 evidence): SHOULD (unchanged): `--stopped --needs-attn`
and `--stopped --active` select nothing (each attention/activity row reads "running
sessions only" literally); `--all` with either keeps only matching running sessions;
no invented usage error — and no silent selector override. IS at 72c7293: `list
--needs-attn --stopped` emits RUNNING attention sessions — ae:4122-4127 force-resets
the scope to running after selector parsing whenever an attention/activity filter is
present, silently overriding `--stopped`. Authority: commands.md filter rows + joint
ruling. Empirical: observed (Batch C A2 @36a107b, inter_needsattnstopped arm — ONE
captured order; order independence is source-proven, not captured: the ae:4122-4127
post-parse force ignores selector order). Conflict: fix-known-defect(#96, intended:
literal intersection).
**classified_by: RE-MARKED after reclassification — both seats, 2026-08-20.**

**SC-521b — same-dimension scope flags are alternatives: last distinct selector
wins.** Bucket 2 — (slice-1c, seat ruling on reviewer3 I7):
`--running`/`--stopped`/`--all` are ALTERNATIVE modes per commands.md:81-87, not
independent predicates; the last distinct selector wins and a repeated flag is
idempotent. The lead's set-intersection alternative was rejected as inventing
semantics the docs do not state and failing silently on `--stopped --running`.
Authority: commands.md:81-87 + joint ruling. Empirical: observed(ae@72c7293:4077-4089
— case loop reassigns show_running/show_stopped per selector). Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-521c — schema-v2 attention/activity filters apply to every session not known
stopped.** Bucket 2 — under SC-509d, `--needs-attn` and `--active` test positive
attention/activity facts on status `running` OR `unknown`; status remains unchanged. A
stopped session never satisfies either live-scope predicate. Therefore default/`--running`
with either filter keeps matching running and unknown sessions; `--all` with either keeps
matching running and unknown sessions; `--stopped` with either remains empty. This
supersedes SC-521a's "matching running sessions" domain and SC-017m's
"positive-running predicates" sentence ONLY for schema version 2; their
selector-intersection, no-relabel, and frozen two-valued conclusions stand. Liveness
uncertainty never erases an independently established record fact. Authority: SC-017d/e +
SC-017l/m + SC-509d + joint P1 ruling. Empirical: successor implementation pending; frozen
schema v1 has no unknown state. Conflict: none.
**classified_by:** SC-521c — attention/liveness independence ruling, 2026-08-21;
fable5:lead + gpt56sol:colead. Bucket 2, conflict none. Successor semantics, not a
frozen-Bash defect: #105 motivates first-class unknown but is not this row's authority.**

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
**PRESENCE IS PART OF THE SCHEMA** (colead ruling, 2026-08-24): every documented
member above whose SOURCE WAS READ is PRESENT, carrying its legitimate empty value
— `null` for an absent string or reason, `0` for `attention_rank`, `false` for
`needs_attention`. Omission requires SC-509b loss or another explicit row, and
nothing else. This rules PRESENCE as a class and invents no VALUES: what each
member's legitimate empty value IS stays with the row that owns it (SC-017g for the
attention triad, SC-509c for `agents[].reason`), and `generated_at`'s exact bytes
remain open.
**Why a class rule rather than a member-by-member one.** The argument does not vary
across members: each is a documented SC-509 member, SC-509d carries it forward, no
row authorises omitting it, frozen v1 always renders it, and SC-509b reserves
omission for facts that could not be READ. Fixed-selector provenance:
`INVOCATIONS.tsv` blob `035c5fab48cf04229daa9285457922d90563fabe`; select rows with
`rc=0`, `phase=P1`, surface in `{ae list,ae ls}`, and `normalised_argv` containing
`--json` as an exact token. For each `(case, consumer)`, resolve committed stdout
exactly as `CORPUS / dirname(case) / out / <consumer>.stdout`, where `CORPUS` is
`docs/migration/evidence/batch-c-artifacts`; accept only one RFC 8259 JSON value
followed only by JSON whitespace, with `schema_version=1` as the guard. This yields
401 captures: 400 `instrument=frozen` and one `instrument=hooked`, the hooked row
being `arms/D/d01-list-vs-goal-writer-barrier/consumers.tsv` /
`barrier-list-json`. Its fixed hook-inactive equivalence is recorded in
`docs/migration/evidence/batch-c-artifacts/arms/D/d01-list-vs-goal-writer-barrier/hook-inactive-equivalence.txt`:
`AE_HOOK` is UNSET, the committed `hook.patch` is added-lines-only, control-vs-hooked
`ae list --json` is IDENTICAL (`rc=0`, `stdout_bytes=598`), and
`control_hooked_divergences=0`; the hook's first guard returns before side effects
unless `AE_HOOK` names it. The hooked capture is therefore inert unless `AE_HOOK`
is set and remains reconstructible as frozen-v1-era evidence. This supersedes the earlier shallow-glob 399/429/840 census, which IS reproducible (glob `batch-c-artifacts/arms/*/*/out/*json*.stdout` plus the `schema_version=1` guard; selected-path sha256 `f3544a89d58caf48521806d7168a6367576219e1cd6fc1e8b6a436c8c6a46534`) but is the wrong population: that artifact-path selector admits 2 D01 `trace-list-json.trace.stdout` non-invocation copies and excludes 4 valid frozen P1 A1/c20 captures nested under `out/<run>/` solely through path depth — minus the 2 traces, plus the 4 nested captures, yields exactly this census. The pinned INVOCATIONS P1 selector is the governed population. The 401 captures contain 431 session
entries and 844 agents; all 431 frozen v1 session entries carry every session member
listed above and all 844 agent entries carry every agent member — including the ones
a successor is tempted to omit, `goal` (null in 326 of 431), `goal_set_epoch`
(null in 336), `branch` (null in 6), `attention` (null in 193) and
`agents[].reason` (null in all 844). The quiet attention triad is `false` / `null`
/ `0` in all 193 quiet entries. Ruling these one at a time would have spent eight
rounds re-deriving one argument.
**The distinction this protects is SC-509b's own.** Its closing sentence — "Damage
is never rendered identically to legitimate sparsity" — holds only if the two have
different spellings. Presence-with-empty-value is the legitimate answer; omission
is the loss. Spend omission on a legitimate empty and SC-509b has nothing left to
say.
Authority: commands.md:97-132 + SC-509d carry-forward + SC-509b scope + colead
ruling 2026-08-24. Empirical: **observed** — 431/431 frozen v1 session entries and
844/844 agent entries render every documented member. Conflict: none.

**SC-509d — the P1 successor schema is version 2 because status gains `unknown`.**
Bucket 3 — fix-known-defect(#105). Once the P1 read-side flip implements SC-017l/m,
every successor digest emits `schema_version: 2`, and `sessions[].status` has the closed
domain `running | unknown | stopped`; it never emits `unknown` under version 1. All
other SC-509 fields and SC-509b degradation semantics carry forward unless another row
changes them. SC-509 remains the true frozen-Bash version-1 contract rather than being
rewritten after the fact. A new enum value is a consumer-visible contract change even
when field name, JSON type, and position stay unchanged; versioning is the gating
mechanism. Authority: SC-509's consumer-gating design + SC-017l/m + joint P1 ruling.
IS at 72c7293: version 1 has only `running`/`stopped`; successor implementation pending.
Empirical: frozen baseline source-proven; successor implementation pending.
Issue #105 records the defect requiring this change, never its normative authority.
Conflict: fix-known-defect(#105).
**classified_by:** P1 schema-v2 joint ruling, 2026-08-21 — SC-509d; fable5:lead +
gpt56sol:colead; bucket 3 fix-known-defect(#105), normative/conflict lane.**

**SC-509e — schema version 2 carries three-valued agent liveness.** Bucket 3 —
fix-known-defect(#105). Every successor version-2 digest retains the SC-509
`agents[].alive` field with the closed JSON domain `true | false | null`: `true` and
`false` carry SC-017p's positively established alive/dead facts, and `null` carries
SC-017q `unknown`. The field is present even when null. Version 1 remains the true
frozen-Bash contract and emits only boolean agent liveness; it never emits null. All
other SC-509 agent fields, SC-509b degradation semantics, and SC-509d session-status
domain carry forward unchanged. Agent liveness null never nulls or relabels an
independently known `state` or `reason`. Schema version 2 has not shipped, so this row
extends its not-yet-released contract rather than creating version 3. Until the real
transport exists, null is the honest output. Frozen IS at 72c7293: VIOLATED — schema
version 1 initializes each agent's `alive` to false and changes it only on a positive map hit
(ae:4053-4065), so unavailable evidence is encoded as a negative fact. Successor IS at
92a20ee9: `AgentEntry.alive` is still `bool` (`src/digest.rs:93-105`) and emits
false for absent runtime evidence. Empirical: frozen and successor relations
source-proven; successor schema-v2 capture pending. Authority: SC-509's consumer-gating
design + SC-017p/q + SC-509d + joint P1 ruling. Issue #105 is the defect-family label,
never normative authority. Conflict: fix-known-defect(#105).
**classified_by:** SC-509e — per-agent liveness schema-v2 ruling, 2026-08-21;
fable5:lead + gpt56sol:colead; bucket 3 fix-known-defect(#105). A boolean-to-nullable
domain change is consumer-visible even though the field name is unchanged; normative/
conflict lane only, with successor capture pending.**

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

**SC-510e — a duplicate KNOWN event key makes the whole record malformed.** Bucket 1
— (slice-1d joint ruling): no row defines duplicate-member precedence, RFC 8259 makes
duplicate-name resolution non-interoperable, and known-key first/last winner
selection is forbidden FABRICATION; a record carrying two members of any documented
stable key is skipped and counted, degrading the session via SC-520's path.
Evidence and tests carry the ordering discriminator: the same duplicate pair with
member order REVERSED must produce the same outcome. Authority: slice-1d joint
ruling (this row is the normative source — SC-405e is meta grammar and remains
UNCLASSIFIED; it is not authority here). Empirical: pending. Conflict: none.
**classified_by: both seats, 2026-08-20.**

**SC-510f — duplicate UNKNOWN event keys stay inert.** Bucket 2 — (slice-1d joint
ruling): additive-schema semantics ignore unknown members whether they appear once
or many times; duplication of an unknown key is not an anomaly and never degrades.
Evidence and tests carry the reversed-order discriminator alongside SC-510e's.
Authority: slice-1d joint ruling + SC-511b additive direction. Empirical: pending.
Conflict: none. **classified_by: both seats, 2026-08-20.**

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
via `@ae_*` user options, which interpolate literally. Authority:
AGENTS.md@72c7293:177 interpreted-sinks row (verbatim). Empirical: pending.
Conflict: none.

**SC-601 — send-keys never receives user text as key names.** Bucket 1 — literal mode
or paste-buffer only; the generated helpers are the boundary, raw send-keys is
forbidden. Authority: AGENTS.md@72c7293:178 interpreted-sinks row (verbatim).
Empirical: pending. Conflict: none.

**SC-602 — `@ae_slot` carries identity; `@ae_agent` is display.** Bucket 2 — the slot
option is the stable routing stamp (SC-209); pre-slot sessions are back-filled on
refresh/resume. Authority: helpers.md@72c7293:57-59 "Slot identity". Empirical:
pending. Conflict: none.

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
looks. Authority: AGENTS.md@72c7293:141 readiness bullet ("An idle input box is not
an initialized application"; both delivery moments gated, `_tool_initializing` asked
FIRST). Empirical: measured codex timings @72c7293. Conflict: none.

**SC-701 — readiness markers are negative evidence only.** Bucket 1 — a marker's
absence is never proof of readiness; a predicate that demands a positive banner breaks
the day a tool stops printing one. Authority: AGENTS.md@72c7293:149-150 ("The
markers are NEGATIVE: their absence is not proof of readiness"). Empirical:
codex `model: loading` / MCP progress measurements. Conflict: none.

**SC-702 — a readiness timeout fails loud and durable.** Bucket 1 — the pane text is
preserved next to the session and an event is emitted, because launch delivery runs
detached where stderr reaches nobody. Authority: AGENTS.md@72c7293:151-152 ("Timeout
is a LOUD, DURABLE failure"). Empirical: pending. Conflict: none.

**SC-703 — an unmodelled TUI is an accepted risk, never a pretend gate.** Bucket 2 —
a tool without readiness/busy modelling (grok at 72c7293) delivers ungated, and that
status is DOCUMENTED, not silently faked. Authority: AGENTS.md@72c7293:137 grok
column ("EXEMPT — no readiness detection … Accepted risk, not a pretend gate").
Empirical: matrix. Conflict: none.

**SC-704 — adapter expectations are seat-ruled, never measurement-promoted.** Bucket 1
— the capability matrix stays IS; measurements never become expected outputs without an
explicit seat ruling (the anti-oracle rule). The generic product outcomes are frozen
NOW as SC-704a-e (gate ruling — classification is not deferred to P1); exact flags,
markers, and per-tool capabilities remain empirical/adaptable. Upstream drift is
detected (canary class, #22), never silently absorbed. Authority: adapter-outcome
ruling commit 661f8f6 + the #81 source-lane rule + epic #79. Empirical: the matrix.
Conflict: none.

**SC-704a — injected ae context never replaces a vendor's own agent prompt.** Bucket
1 — context rides an append/positional/config surface per tool; a replace-style flag
is forbidden (the grok `--system-prompt-override` trap). Authority: adapter-outcome
ruling commit 661f8f6 (SC-704 frame) + AGENTS.md@72c7293 grok system-prompt row
("`--system-prompt-override` … REPLACES grok's own agent prompt — never use either").
Empirical: matrix rows. Conflict: none.

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
resumes the same conversation or honestly starts fresh; never a collision error,
never a silent identity swap. Authority: adapter-outcome ruling commit 661f8f6
(SC-704 frame) + AGENTS.md@72c7293 launch-rerun bullet and table row (upfront-UUID
tools resume the same UUID; post-launch-capture tools start FRESH — "the conversation
is simply lost", never a collision) + SC-811 pins context. Empirical: matrix rerun
row + SC-811 pins. Conflict: none.

**classified_by (S8 adapter-frame MARK batch 5, ae-20260820T174651Z-400d70f4):
SC-700, SC-701, SC-702, SC-703, SC-704, SC-704a, SC-704e — fable5:lead +
gpt56sol:colead, 2026-08-20. Exact enumeration; later rows never inherit; SC-704b,
SC-704c, SC-704d explicitly do NOT inherit this mark (bucket-3/DR-005 rows, Q2).
SC-700/701/702/704/704a/704e bucket 1, SC-703 bucket 2; conflict=none throughout.
Marked with the countersign conditions applied first: SC-704a/704e normalized into
contiguous rows, the joint-ruling authority made durable (adapter-outcome ruling
commit 661f8f6 + the SC-704/#81 frame), anchors line-precise. Normative/conflict
lane only; Empirical columns untouched.**

**SC-705 — tool detection identifies the actual executable without interpreting
injected prose.** Bucket 3 — semantic SHOULD under joint ruling: classification derives
from the real binary being launched, never from the kilobytes of injected context,
wrapper prefixes, or launcher spellings. The concrete prefix/suffix exhibits
(`env`/`VAR=val` stripping, `opencode.exe`) are empirical. IS at 72c7293 (B0 Design 8
live observation, both-seats classified): a real Claude pane reports its VERSION
STRING as `pane_current_command`, and `tmux_paste_submit` re-derives the tool from it
(ae:412-417), so the spawn-brief and launch-prompt deliveries (ae:12024, 12678) run
the unmodeled-tool arms of `_paste_input_busy`/`_paste_still_staged`
(ae:13976-94, 13935-44) — Claude's paste protection silently bypassed; helper sends
unaffected (they transport ae_target_tool); `wait_for_agent_start` tolerates via its
non-shell fallback (ae:923-932). Authority: S8 joint seat ruling (2026-08-20)
grounded in the #46/#30 transported-fact rulings. Empirical: measured exhibits
@72c7293 + b0-artifacts/design8 fake-vs-real divergence record. Conflict:
fix-known-defect(#94, intended: the canonical tool kind is transported from
config/meta into every delivery call; a process title is liveness evidence only,
never a classifier). **classified_by: REOPENED by the #94 observation and RE-MARKED —
fable5:lead + gpt56sol:colead, 2026-08-20.**


**Empirical colour (joint L classification, both seats, 2026-08-20):** the affected
population is not hypothetical. Frozen AGENTS.md@72c7293:165 states as a MEASURED fact
that the `claude --resume … || --continue` resume path reads `bash` in
`pane_current_command`; since delivery re-derives tool kind from that same field,
Claude panes launched through the bash-era fallback-chain resume path lose the TUI
protection this row governs. Scoped precisely (colead): NOT every resumed session —
marker-based `launch.<slot>.sh` reruns are also resumes and take the explicit-branch
path. Sourced to AGENTS.md:165 only; no new issue — #94 owns the fix.

**SC-706 — a fact built upstream is transported, never re-parsed.** Bucket 3 — resume
ids, injection boundaries, and tool kinds ride explicit parameters; the built command
is downstream data — hostile input, not a source of truth. IS at 72c7293:
`tmux_paste_submit` violates this for the delivery tool kind (see SC-705's #94
chain — the one shipped exception to the #30-family rule found by the evidence
program). Authority: #30-family ruling (commit 32719f5) + AGENTS.md. Empirical:
shipped exhibits + b0-artifacts/design8. Conflict: fix-known-defect(#94, intended as
in SC-705). **classified_by: REOPENED by the #94 observation and RE-MARKED —
fable5:lead + gpt56sol:colead, 2026-08-20.**

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

**SC-818e — purge never deletes the parent archive referenced by a live `--from`
lineage.** Bucket 1 — precised to OUTCOME grain by joint L-PURGE classification (both
seats, 2026-08-20): the promise is the outcome, not the guard. NORMAL lineage
satisfies it by ADDRESS SEPARATION, not by refusal — a real `--from` child receives a
fresh session UUID, so `end --purge-history <child>` addresses the child's own archive
and never the parent's; the operation SUCCEEDS on the child and the parent is
untouched. The defensive guard (`_ar_purge_archive` 72c7293:5404-5408, which fires
only when the purged aid equals the session's own `parent_archive_id`) is reachable
only from CORRUPTED meta where those ids are equal, and there it refuses by name.
Guard reachability is implementation evidence, not the normative claim.
Authority: architecture.md:137-138. Empirical: observed (L-PURGE @c7f291b —
`lineage-parent-literal` rc=0: real child purges its own archive, parent intact
[address separation]; `lineage-parent-mutated` rc=1 with the named refusal "refusing
to purge <aid> — it is the parent archive this session was launched from"
[corrupted-meta case]). Conflict: none.
**classified_by: both seats, 2026-08-20 (joint L-PURGE classification).**

**SC-819 — an unidentifiable session is refused BEFORE anything is stopped.** Bucket 1
— meta gone with memory remaining, or `session_id` unparseable: refused with the
reason, nothing deleted, regardless of history flag ("delete it" is not an answer to
"which session is this"). Authority: commands.md:513-518 + architecture.md:139-143.
Empirical: pending. Conflict: none.

**SC-820a — end freezes the confirmed plan and re-proves it under the lock.** Bucket 3
— fix-known-defect(#98) (reclassified from bucket 1/conflict=none by joint L-END
classification, both seats 2026-08-20). SHOULD (unchanged): each target resolved
exactly ONCE; the prompt renders from those fields and the freeze captures the same
observation (a fork cannot carry a freeze back); re-proof under the lifecycle lock
REFUSES on mismatch, prints both versions, and takes no action on that target. IS at
72c7293: with a confirmed target's tmux session renamed between the accepted answer
and the per-target lifecycle lock (on-disk state untouched), `end all` proceeds
normally — stdout prints the ordinary archive/cleanup/ended lines, stderr is EMPTY,
no both-versions diagnostic appears, and the session is archived and cleaned on disk
while its renamed tmux session stays ALIVE: the torn state the freeze contract exists
to prevent, produced silently. Authority: commands.md:526-532 +
architecture.md:146-149,158-166. Empirical: observed (L-END
`endall-rename-between-confirm-and-lock` @7aab1b4 — `2endall.stdout.od` decoded,
`2endall.stderr.od` empty, `post-state.txt` showing `ef2-renamed` alive; see
l-classify-L-END.md). Conflict: fix-known-defect(#98, intended: refuse the mismatched
target, print both versions, act on nothing for it).
**classified_by: both seats, 2026-08-20 (joint L-END classification).**

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

**SC-832c — an interrupted rename never leaves a mixed generation ACCEPTED as
committed.** Bucket 3 — fix-known-defect(#103). **NORMATIVE CONCUR / EMPIRICAL HOLD**
(joint seat closure, 2026-08-20).
SHOULD, one invariant: after interruption at ANY rename cut, **no product reader or
operation accepts a mixed generation as committed**. Before any further effects, ae must —
under the rename identity protocol — either complete or roll back to exactly ONE coherent
old/new generation, or **refuse loudly without further identity mutation**.
**The recovery STRATEGY is deliberately not frozen; the ACCEPTANCE of mixed state is
what is forbidden.** Completing forward, rolling back, and refusing are all conforming;
proceeding as though a half-renamed session were whole is not.
**Boundary with SC-1303:** SC-1303 owns **live-reader linearizability** during the
operation; this row and D20 own **fresh-process recovery and rollback** after it dies.
The concurrent live meta-writer serialization question stays with SC-832b, which is not
crash recovery. *(Prose kept unbolded at line start — the sweep guard reads a line-initial
bolded `SC-` as a row head; this is the second time it caught me.)*
**EMPIRICAL STATUS: OBSERVED — the hold is LIFTED** (L-832C, 13 arms, 2026-08-20). The
gate's conditions were met exactly: deterministic **SIGKILL** of the rename's whole process
tree at post-tmux, post-dir and post-meta with the entry cut as control (`rename_rc 137`
in all thirteen), descriptors released **by death** and proven so per arm (`kill -0` fails,
every `.lifecycle.*.lock` freshly acquired and released, `precondition.met YES`), and a
**fresh process** as the only thing touching the crashed state.
**IS: VIOLATED, and now observed rather than argued.** The identity sources DISAGREE in
**7 of 13 arms** — every tmux-cut and every dir-cut — and agree in the entry control and
the meta cut, following the frozen phase order exactly (tmux ae:11635 → dir `mv` ae:11650
→ meta ae:11655). The window is two cuts wide.
*The ability to fail is satisfied POSITIVELY, not by control:* whether the sources agree is
DERIVED from the captured tuple, and the recorder demonstrably registers a mixed generation
where one exists, so the six agreeing arms report against an instrument shown capable of
the opposite.
**A supported product reader ACCEPTS the mixed generation.** At the tmux cut a fresh
`ae status proj` returns **rc 0** reporting *"Session 'proj' is stopped"*, while
`ae status proj2` returns **rc 0** rendering the LIVE PANE — running out of
`sessions/proj/`. Both names answer successfully with contradictory answers about the same
session and neither signals inconsistency. That is the row's prohibition violated at a
consumer, not merely residue on disk.
Authority: joint seat ruling. Conflict: fix-known-defect(#103).
**classified_by: both seats, 2026-08-20 (normative concur, empirical hold).**

**SC-832d — rename addresses its SOURCE by recorded server and exact live session id.**
Bucket 3 — fix-known-defect(#102). SHOULD: rename resolves the target on the session's
**recorded** tmux server and addresses the **exact live session id**; when no exact
instance exists it refuses **loudly, with zero effects**, and never touches a prefix
sibling. IS at 72c7293: **VIOLATED** — ae:11635 is
`tmux rename-session -t "$old_name" "$new_name"`, by NAME and on the AMBIENT server, and
`tmux -t <name>` prefix-matches. Observed: with `proj` already stopped and `projx` live,
`ae rename proj proj2` renamed **`projx`** and returned 0.
**Serialization is innocent** — the cell had `flock` present with both lifecycle locks
acquired; *it serialized the wrong identity decision.* Do not file this against the
lifecycle-lock work.
*Evidence lane:* source-name prefix corruption is **observed** (L-RENTRANS
`samename-matrix-stop-first-flock-with`); the **ambient-server** half is **source-proven
only** (ae:11617/11635) because that arm ran on a single server, and needs a multi-server
arm before its empirical field says observed.
Authority: joint seat ruling grounded in the SC-835a hazard, which `stop` already
addresses with `-S` plus an exact id. Conflict: fix-known-defect(#102).
**classified_by: both seats, 2026-08-20.**

**SC-832e — rename's DESTINATION occupancy check is an exact-name check.**
Bucket 3 — fix-known-defect(#102). SHOULD: the destination-taken gate tests the **exact
name on that same recorded server** — a prefix-only sibling does **not** block the rename,
an exact match does. IS at 72c7293: **VIOLATED** — ae:11622 uses
`tmux has-session -t "$new_name"`, which prefix-matches, so renaming to `proj` while
`projx` exists falsely reports `session 'proj' already exists`. The mirror of SC-832d:
one defect refuses what it should allow, the other mutates what it should refuse.
*Id note:* these are **d/e**, not b/c — SC-832b (rename vs concurrent meta writers) and
SC-832c (rename crash cuts) already existed as code-observation placeholders. The crash-cut
placeholder *overlaps #103*: it asks what residue each cut leaves, which is the crash-side
view of the same window SC-1303 now governs from the reader side. Flagged for seat closure
together, NOT closed here. *(Prose kept unbolded at line start deliberately — the sweep
guard reads a line-initial bolded `SC-` as a row head, and this note tripped it once.)*
*Evidence lane:* **source-proven only** — no arm exercised a destination whose prefix
sibling exists. Needs a destination-prefix arm.
*Consequence recorded with the row:* post-rename window and status calls should continue
from **captured exact ids**, not from `<new_name>:0`.
Authority: joint seat ruling. Conflict: fix-known-defect(#102).
**classified_by: both seats, 2026-08-20.**

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
Authority: install.md@72c7293:12-27 (One-line install + From-a-local-clone).
Empirical: pending. Conflict: none.

**SC-1001 — an upgrade preserves existing sessions and refreshes/migrates phase-owned
assets.** Bucket 2 — outcome-level (gate correction: helper REGENERATION is the
bash-era asset mechanism and retires at P2 — it is not frozen as a P5 promise):
sessions keep working across an upgrade, and whatever assets the current phase owns
are refreshed or migrated on next start/resume (`doctor --refresh [name]` forces it).
Authority: install.md@72c7293:37-50 Upgrading (:46 — frozen BASELINE: it proves only
the bash helper mechanism) + joint gate ruling f869b66 and the epic frame for the
phase-owned refresh/migrate successor abstraction. Empirical: pending.
Conflict: none.

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
producer dying mid-pipe must not publish a prefix). Authority: AGENTS.md@72c7293:119
(`_publish_executable_artifact` chokepoint paragraph, verbatim; M3 ruling).
Empirical: unit guard @72c7293. Conflict: none.

**SC-1004 — session helpers publish temp+chmod+mv, atomically per artifact.** Bucket 1
— a generator failure can never truncate a live helper. Authority:
AGENTS.md@72c7293:117 declare-f paragraph ("written atomically (temp + chmod + mv) so
a generator failure can never truncate a live session's helper" — verbatim; relocated
A-03). Empirical: unit guards @72c7293. Conflict: none.

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

**SC-1106 — tmux-format reads are locale-independent.** Bucket 3 — SHOULD: either
the invocation environment guarantees an encoding under which the chosen separator
survives, or the separator is one tmux never rewrites; a valid user locale never
converts live panes into dead/absent. IS at 72c7293 (Batch C A1 incident, four-way
locale matrix): tmux 3.7b sanitises TAB to underscore in `-F` output under a
non-UTF-8 locale, and ae parses TAB-framed tmux formats at seven sites (ae:3631,
4207, 6488, 12151, 12170, 12297, 12962) without forcing a locale — under LC_ALL=C
live agents render alive:false, the rollup reports attn:dead, status reports no
panes. Distinct from the harness lesson (baselines pin UTF-8); the C-locale failure
is the product's own (colead dissent, adopted). Authority: AGENTS.md TSV-framing +
interpreted-sinks direction (ruling). Empirical: observed(committed artifacts @36a107b
— the 605cbb6 citation was dangling: its claimed per-case self-checks were never
persisted, and the A1 arm was RERUN in full under chronology-bearing admissibility
ledgers rather than backfilled. Now durable: batch-c-artifacts/MANIFEST.md
"Correction" section (the four-way isolation matrix); per-case
arms/A1/*/env-tab-selfcheck.txt UTF-8/C paired probes with their
admissibility-ledger.txt ordering records; the 405k paired consumer battery
out/s0-baseline vs out/s0-baseline-clocale on one unchanged topology. The dedicated
C-vs-UTF-8 negative arm still rides F-PLATFORM).
Conflict: fix-known-defect(#95, intended: locale-independent tmux-format reads —
guaranteed encoding or an unsanitisable separator). **classified_by: both seats,
2026-08-20 (dissent adopted as the ruling).**

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
closing ruling + AGENTS.md@72c7293:163 (a name ae DERIVES must be a fixed point of
the grammar) + the 72c7293 commit body. Empirical: unit pins @72c7293.
Conflict: none.

**SC-1205b — dedup shape: truncate to fit, suffix from `-2`.** Bucket 2 — the suffix
counts occurrences (meaning), not array position; the base is truncated so the suffix
fits the 64 cap. Authority: #59 closing ruling + AGENTS.md@72c7293:163 (dedup
truncates the base, suffixes from -2, validates the FINAL value) + the 72c7293 commit
body. Empirical: unit pins @72c7293. Conflict: none.

**SC-1206 — a leading underscore is a legal alias but never an agent name.** Bucket 2 —
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
`workers = _foo` (alias as its own name) fails the launch with the grammar; internal
`_`-prefixed helpers stay out of the agent namespace. Empirical: pending.
Conflict: none.

**SC-1207a — prompt identity facets are unambiguous.** Bucket 1 — neither the alias
nor the name may contain the facet separator; identity parses one way only.
Authority: #59 ruling + AGENTS.md@72c7293:162-163. Empirical: pending.
Conflict: none.

**SC-1207b — meta serializes agents as `alias:name:provider-session-id`.** Bucket 2
— exact on-disk form (cross-link: S5 formats family). Authority: #59 ruling +
AGENTS.md@72c7293:162 + architecture.md@72c7293:65. Empirical: pending.
Conflict: none.

**SC-1208 — untrusted pane bytes and peer message-body prose are never spliced into
instruction material.** Bucket 1 — (precised, B0-census reopening 2026-08-20):
transport delivers peer text through the model's USER-INPUT surface; ae never places
pane content or peer MESSAGE-BODY prose into AE-CONSTRUCTED CONTEXT MATERIAL —
build_ae_context output on whatever vendor surface carries it: a system/developer
flag for claude/codex/opencode, the initial user-turn slot (-i / positional) for
gemini/grok; the vendor lane is an annotation, never an upgrade of the guarantee;
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

**SC-1300 — concurrent event appends yield complete, non-interleaved records in one
stable append order.** Bucket 1 — promise-level ("ordered" precised by seat ruling
ae-20260820T175119Z-faaf0b3f: one stable append order, not any cross-generation
claim — DR-001 governs successor generation ordering; the adjacent lock file is the
bash mechanism, empirical; DR-001's one-generation protocol supersedes the mechanism,
not the promise). Failure SEMANTICS are per-operation rows (M1). Authority:
events.md@72c7293:7 + bridge-protocol.md@72c7293:90. Empirical: census-1 M1.
Conflict: none (DR-001 affected — mechanism only).

**classified_by (mixed-tail MARK batch 6, ae-20260820T175119Z-faaf0b3f): SC-400a,
SC-401a, SC-402, SC-403, SC-600, SC-601, SC-602, SC-1000, SC-1001, SC-1002, SC-1003,
SC-1004, SC-1205a, SC-1205b, SC-1207a, SC-1207b, SC-1300 — fable5:lead +
gpt56sol:colead, 2026-08-20. Exact enumeration; later rows never inherit; SC-1301
was normalized in the same pass but gains NO mark (bucket-3 Q2 row). Marked with the
countersign conditions applied first: SC-400a's two authority lanes explicit
(frozen layout vs f869b66/#79/#81 readability frame, artifact inventory left
empirical), SC-1001's successor abstraction anchored to f869b66 + epic frame with
install.md as frozen baseline, the four #59 anchors made precise, SC-1300's
"ordered" precised to one stable append order with DR-001 owning cross-generation
ordering, SC-1207a/b normalized. Normative/conflict lane only.**

**SC-1301 — session meta is written through one fail-closed writer.** Bucket 3 —
SHOULD (architecture.md:158-166, the one-writer doc contract): one function, every
step checked, missing meta
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

**SC-1303 — rename is LINEARIZABLE: every external reader sees ONE coherent identity
generation.** Bucket 3 — fix-known-defect(#103).
**SHOULD, scoped to be testable:** every **product reader or operation** — anything ae
ships that resolves a session (`list`, `stop`, `send`, the helpers, a concurrent `ae`
process) — takes ONE COHERENT LOGICAL SNAPSHOT of the identity generation: the old one or
the new one, **never mixed**. A generation is the tuple (tmux session name, state
directory name, meta `session=` key, `workspace.md`).
**Scope precision (colead, adopted):** the claim is deliberately NOT "every external
reader". tmux plus four filesystem facts live in different stores and cannot be made
atomic against an arbitrary lock-free reader that does not participate in the protocol;
a SHOULD that requires the impossible is untestable and would be quietly ignored. The
requirement binds **supported readers**, which must participate.
**Boundary with the crash case:** this row governs **LIVE READER LINEARIZABILITY** only.
**Crash recovery and rollback semantics — what residue an interrupted rename leaves and
how it is repaired — belong to SC-832c and D20**, not here.
IS at 72c7293: **VIOLATED** — the four are mutated in sequence with no barrier (tmux
ae:11635, dir `mv` ae:11650, meta ae:11655, manifest/status ae:11665-67), and at the
post-`mv` cut the directory is `sessions/<new>/` while its own meta still reads
`session=<old>`.
*Seat history, recorded because the first attempt was wrong:* lead proposed closing this
at bucket 1 with the disagreement PERMITTED ("the directory name and meta key MAY briefly
disagree"). Colead dissented and carried it — that would have promoted an observed bash
mechanism into a SHOULD and **blessed D20's atomicity hole**, leaving the P3 owner no
requirement to do better. Permitting a hole is still writing the hole into the contract.
No DR is needed because mixed generations are ruled a DEFECT, not an acceptable outcome.
*Note what the evidence says — CORRECTED (colead, 2026-08-20; lead's first reading was
wrong).* `ae list` was only **NAME-CARDINALITY coherent**, never **generation-coherent**,
and the supported product reader **did accept mixed generations**:
- at `b_rn_tmux_renamed` it returned rc0 for `proj2` with **`agents=[]` and
  `last_active_epoch=0`**, while the complete state still lived under `sessions/proj`;
- at `b_rn_dir_moved` it returned rc0 reading `sessions/proj2` whose own meta still said
  `session=proj`.
So the violation is **user-visible through the product's own reader**, not confined to a
direct filesystem inspection — an `ae list` during a rename can show a live session as
having zero agents and no activity. This **strengthens** the bucket-3 conflict rather than
softening it.
Authority: joint seat ruling (lead ruling on colead's dissent, 2026-08-20). Empirical:
observed (L-RENTRANS `rename-observer`, four cuts). Conflict: fix-known-defect(#103).
**classified_by: both seats, 2026-08-20 (dissent ruled).**
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
**SC-1305 — concurrent readers see ONE coherent lifecycle phase during compact.**
Bucket 1 — seat closure of a placeholder head (joint L-COMPACT classification, both
seats 2026-08-20; the previous text was "mid-operation observability", which stated no
SHOULD and could not be ratified as written). A reader observing compact mid-operation
sees one coherent phase and NEVER mixed predecessor/successor state; a no-session
interval between predecessor removal and successor publication is PERMITTED (permitted,
not required — a successor that publishes without a visible gap also satisfies this).
The requests-helper absence observed at the pre-relaunch cut is empirical MECHANISM,
not part of the claim. Authority: joint seat ruling (L-COMPACT closure) grounded in
architecture.md's compact phase order. Empirical: observed (L-COMPACT @abaeb4f —
five pre-teardown cuts each showing one coherent running predecessor; the pre-relaunch
cut showing no session). Conflict: none.
**classified_by: both seats, 2026-08-20 (joint L-COMPACT closure).**
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

**SC-1400** — `AE_WATCHDOG_INTERVAL_SEC` defaults to 60. Bucket 2. Authority:
config.md:120 + watchdog.md:37 (anchor corrected: the frozen tunables table header
occupies :35-36; rows run :37-44). Empirical: pending. Conflict: none.
**SC-1401** — `AE_WATCHDOG_STALE_MIN` defaults to 15. Bucket 2. Authority:
config.md:121 + watchdog.md:38 (anchor corrected, same offset). Empirical: pending.
Conflict: none.
**SC-1402** — `AE_WATCHDOG_MAX_NUDGES` defaults to 2. Bucket 2. Authority:
config.md:122 + watchdog.md:39 (anchor corrected, same offset). Empirical: pending.
Conflict: none.
**SC-1403** — `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` defaults to 5. Bucket 2. Authority:
config.md:123 + watchdog.md:40 (anchor corrected, same offset; row normalized from
interleaved fragments). Empirical: pending. Conflict: none.
**SC-1404a** — `AE_WATCHDOG_TG_SUPERVISE_SEC` defaults to 120. Bucket 2. Authority:
config.md:124 + watchdog.md:41. Empirical: pending. Conflict: none.
**SC-1404b** — tg-supervise `0` disables supervision. Bucket 2. Authority:
config.md:124 + watchdog.md:41 ("`0` disables"). Empirical: pending. Conflict: none.
**SC-1405a** — `AE_WATCHDOG_SWEEP_SEC` defaults to 300. Bucket 2. Authority:
config.md:125 + watchdog.md:42. Empirical: pending. Conflict: none.
**SC-1405b** — sweep `0` falls back to normal watchdog behavior. Bucket 2. Authority:
config.md:125 + watchdog.md:42 ("`0` falls back to the normal watchdog"; row
normalized from interleaved fragments). Empirical: pending. Conflict: none.
**SC-1406a** — `AE_WATCHDOG_SWEEP_RETRY_SEC` defaults to 30. Bucket 2. Authority:
config.md:126 + watchdog.md:43. Empirical: pending. Conflict: none.
**SC-1406b** — sweep-retry is clamped to the sweep cadence (floor: next poll). Bucket
2. Authority: config.md:126 + watchdog.md:43 ("clamped to it; floor — lands on the
next poll"). Empirical: pending. Conflict: none.
**SC-1407a** — `AE_WATCHDOG_SWEEP_RETRY_MAX` defaults to 6. Bucket 2. Authority:
config.md:127 + watchdog.md:44. Empirical: pending. Conflict: none.
**SC-1407b** — reaching the configured retry maximum ends fast retry, returns to
normal cadence, and raises one `meta-agent unreachable` alert. Bucket 2 — rewritten
self-contained by seat grain condition (ae-20260820T173732Z-95db692d): the former
"exactly as SC-938" wording would have imported the still-unmarked SC-938 contract
through a marked row. Clearing the alert on landed delivery remains owned ONLY by
SC-938 and is not claimed here; SC-938 gains no inherited mark. Authority:
config.md:127 + watchdog.md:44. Empirical: pending. Conflict: none.

**SC-1408a — an explicit `AE_WATCHDOG_*` value wins over its `AE_LOOP_*` legacy name.**
Bucket 2. Authority: config.md:129 ("the legacy `AE_LOOP_*` names are still honoured
as fallbacks for each tunable" — fallback semantics directly make an explicit primary
value authoritative; seat-confirmed reading). Empirical: pending. Conflict: none.

**SC-1408b — each documented tunable honours its `AE_LOOP_*` name when the primary is
unset.** Bucket 2 — per-mapping verification is one probe matrix. Authority:
config.md:129 (verbatim sentence). Empirical: pending. Conflict: none.

**classified_by (S15 env/config MARK batch 4, ae-20260820T173732Z-95db692d):
SC-1400, SC-1401, SC-1402, SC-1403, SC-1404a, SC-1404b, SC-1405a, SC-1405b,
SC-1406a, SC-1406b, SC-1407a, SC-1407b, SC-1408a, SC-1408b — fable5:lead +
gpt56sol:colead, 2026-08-20. Exact enumeration; later rows never inherit; SC-938
explicitly gains NO inherited mark from SC-1407b's rewrite. All bucket 2,
conflict=none. Marked with the countersign conditions applied first: four watchdog.md
anchors corrected for the +2 table-header offset, config.md anchors made exact
(:120-127, :129), three malformed rows (SC-1403/1405b/1407b) normalized, SC-1407b
rewritten self-contained. Normative/conflict lane only; Empirical remains pending
(H-batch env matrix).**

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
