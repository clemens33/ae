# Batch H-HELPER — design

**STATUS: DRAFT v4 — worker-authored (opus5:cexec), REQUEST-CHANGES from gpt56sol:colead
addressed below. No arm runs until both seats approve.** v1-v3 are in git history;
this revision accepts every factual correction in that review, and each one is recorded
where it changed the design rather than silently applied.

**Boundary: twenty rows — the whole `crit-assign.md` H-HELPER set.** All twenty are still
CRITICAL in `ratification-critical.md`. `crit-assign.md` is the authority for membership.
The rows split into three MACHINERY classes, and that split drives execution order:

1. **Eighteen non-hook surface rows** — invoke a surface, capture streams and files.
2. **D14b** — mutating; A8's write witness beside the content manifest.
3. **SC-1301** — fault injection into three writers; hooks and barriers.

**Claim types are NOT uniform, and the earlier draft's "eighteen of twenty are refusal
rows" was false.** Only SC-211a-j and SC-211l are refusal/malformed-mode rows (eleven).
SC-211n is a LONG-LIVED QUERY, SC-211o an identity-registration row, SC-211p a GRAMMAR
row, SC-212c a signature row, SC-012b/013/014 surface-capture rows, D14b a regeneration
row, SC-1301 a fault row. The discriminator below is tailored per claim type, not applied
uniformly.

## Value-blindness, and the contamination disclosure

Every clause in **this document above the SEAT ANNEX** is manipulation + barrier +
capture. Expected relations — per-input outcome labels, which readings a fixture is built
to oppose, and the candidate implementations a differentiator separates — live in the
**SEAT ANNEX at the end**, which is never included in any worker brief. That is the seam
`batch-c-design.md` uses and this document now uses it consistently; v1-v3 did not, and
the reviewing seat was right to block on it.

**Disclosure:** the author of this design has read the relations now in the annex, having
written them. Two remedies are available and the choice is the seats':

- **(a) A fresh executor.** The annex-free region above is a complete brief; hand it to a
  worker who has not read the annex.
- **(b) A pre-registered, non-adaptive capture program.** Every arm script is committed
  BEFORE its first run, its sha256 recorded in the run manifest, and any post-hoc change
  requires an amendment record naming what changed and why. Adaptive capture — choosing
  what to record after seeing a reading — is then mechanically visible rather than
  promised.

The author's view, offered as input and not as a ruling: (b) is the stronger guarantee of
the two, because a fresh executor can adapt just as easily and leaves no trace when it
does, whereas a committed-before-the-run script makes adaptation an auditable diff. (a)
and (b) compose; if the seats want both, the brief is the region above the annex and the
scripts are committed first either way.

## Instrumentation admissibility

Governed by `cluster-plan.md`'s global rule; batch-local specifics, no exceptions:

- **The helpers under test are GENERATED artifacts.** Every helper byte exercised here
  comes from a real frozen `ae` launch, never a hand-written script.
- **The environment is an instrument** (A1 locale incident): the TAB round-trip is proven
  per arm before any capture.
- **The writing helpers make their arms MUTATING** (`state`, `goal`, `memo`, `say`,
  `spawn`, `retire`, `_register-sid`): A8's machinery applies — declared harness-touched
  set, content change record, inode/mtime/size write witness, and per-case instrument
  controls fired AFTER the measured invocation.
- **Every invocation is BOUNDED.** A timeout produces its own INCONCLUSIVE artifact naming
  the bound; it is never reported as a refusal or as a product rc.

## What an empty capture can mean — the state set, and how each is separated

An empty stdout with a non-zero rc is produced by at least five different states, and a
capture that cannot separate them is not evidence about a refusal:

| State | Separated by |
|---|---|
| the surface REFUSED after reaching the row's guard | leg 1 + the main-entry witness + the guard witness |
| PRE-MAIN-ABORT — shebang, `source _lib`, or an early guard died before `helper_*_main` | the main-entry witness is ABSENT; `surface-state.txt` carries the file facts |
| MAIN ENTERED but failed BEFORE the row's guard | main-entry witness present, guard witness absent |
| HUNG or BLOCKED — `flock`, a tty/stdin read, or a follow loop | the bound fires; its own artifact, never a refusal |
| the READER was blind — the capture path produced nothing whatever happened | the controller canaries |

**Leg 1 — raw streams.** stdout, stderr, rc captured as separate byte streams per
invocation, never merged. The helper's own exit status is recorded distinctly from the
shell's `126`/`127`, which are reports ABOUT the invocation rather than FROM the surface.

**Leg 2 — source state.** `surface-state.txt` per case: for the helper under test, the
resolved path, existence, type, mode, size, sha256, interpreter line, and the `_lib` it
sources, read from the filesystem as the invoking uid — A9's `meta-state.txt` pattern
applied to the helper itself.

**Leg 3 — main-entry and guard witnesses.** A controller-only xtrace twin, admissible only
under "The xtrace twin" below, records whether `helper_*_main` was entered and which guard
the invocation reached. Where the twin is inadmissible for a helper, that helper's case
says so by name and keeps the other legs.

**Leg 4 — controller canaries, not product controls.** The earlier draft used "an
invocation known to take the non-refusal path" as the capture-path control. That is
unusable twice over: it states a product expectation, and a successful helper may
legitimately emit nothing at all (`interrupt` without a message is one). The control is
therefore CONTROLLER-GENERATED: a canary process writes known bytes to stdout, known bytes
to stderr and exits with a known rc THROUGH THE EXACT capture wrapper the arm uses, in the
same case, and the captured artifacts must carry those bytes and that rc. It tests the
capture path and nothing else — the equipment, not the product.

**Leg 5 — the bound.** Every invocation runs under a named bound. Expiry writes its own
artifact (the bound, the elapsed time, what had been captured) and the case is
INCONCLUSIVE for that invocation.

## The xtrace twin — admissible only where it is proven inert

The twin is instrumentation, so cluster-plan's inactive-equivalence rule applies one layer
down, and the earlier construction was not admissible:

- **Traced and untraced runs go on SEPARATE FRESH CLONES of one sealed fingerprint**, not
  twice through one case. Most helpers here WRITE, so a re-run inside one case would let
  the primary change the input the twin then reads.
- **Equivalence is compared on stdout, stderr, rc AND the file/tmux deltas** — a twin that
  matches on streams while writing differently is not equivalent.
- **`BASH_XTRACEFD` alone does not enable tracing**; the twin must also enable `xtrace`,
  and the design states where: `bash -x <helper>` for the witness run only, which BYPASSES
  the shebang and the executable-mode path — so it is admissible only as a witness AFTER
  the primary has exercised those, never as the primary.
- **The fd is chosen against measurement and then asserted.** Two independent censuses of
  frozen `ae` (the seat's, wider; mine, narrower — mine misses the `200>"$lock"` form)
  agree that nothing at or above 250 is used. `BASH_XTRACEFD` is set at 250+ and the arm
  asserts the descriptor is closed before use. The hazard is concrete: fds 8/9 are `flock`
  descriptors and 200/201 the meta writers', so a twin landing on one would collide with
  the locking the helper performs.
- **`spawn` and `retire` `exec "$AE_PATH"` (ae:14722, ae:14746).** Wrapper xtrace ends at
  that exec, so the twin CANNOT witness which `_cmd_spawn`/`_cmd_retire` guard ran. Those
  two rows either get their own admissible witness on the delegated frozen `ae`, or their
  cases state that leg 3 is unavailable. No claim is made that the twin identifies those
  guards.
- A helper whose captures differ under trace has INADMISSIBLE trace evidence, recorded by
  name; its primary capture stands.

## Required pre-step — the argument census

Before any arm is written, each frozen helper's argument handling is enumerated into a
committed table: for every input class, whether it is ACCEPTED, REJECTED, IGNORED, or
HANGS, derived from the frozen function itself with line citations — not from prose. The
arms are then built from that table. This exists because the v3 matrix was freehand and
carried two direct row/source mismatches (below); a census makes the omission visible
instead of leaving it to the author's memory.

## Fixture groups

| Group | Fixture |
|---|---|
| H1 base | live session, two agent panes, the full generated helper set from a real launch |
| H2 dead-pane | H1 with one agent pane killed (named mutation; pane id retained in meta) |
| H3 shell-pane | H1 with one agent pane replaced by a plain shell (ae:14673's own precondition) |
| H4 no-server | H1's directory with no tmux server |
| H5 cross-session | two live sessions on one server: colliding bare names, colliding alias, one unique of each, session names chosen so no name is a prefix of another |
| H6 codex cohorts | two slots with distinct `launch_id.<slot>` / `launch_time.<slot>`, and producer-derived candidate Codex JSONL files whose token / mtime / CWD / first-line facts vary independently |
| H7 request-state | H1 plus a produced ask→reply pair and one unanswered ask |

Every member is producer-derived and mutated only by NAMED mutations with byte diffs
recorded beside it.

## Arms

**A-H1 top-level dispatch (SC-012b, SC-014).** `ae -h`, `ae --help`, `ae help`, `ae
version`, `ae --version`, `ae -V`, and one unknown flag — each invoked separately into its
own capture (ae:16841-16848).

**A-H2 steward (SC-013).** `ae steward --help`; `ae steward --detach` under a bound, its
process reaped in teardown, file manifest before/after.

**A-H3 helper argument surface (SC-211a-j, SC-211l, SC-212c).** One case per helper, each
invoked from a real pane, with the input classes drawn from the argument census. Known
corrections to the v3 matrix, all from the review and all verified against frozen source:

- **SC-211b `goal`:** `goal foo bar` is a VALID set (`*) text="$*"`, ae:14577-14590); the
  `$# -eq 1` guard at ae:14569 belongs to `--clear`. The case therefore exercises
  `--clear extra`, not "two positionals", and adds `-h`/`--help` as distinct paths.
- **SC-211i `spawn`:** the row is `spawn` NON-NAME argument errors. Name-grammar inputs
  belong to SC-1201 (F-IDENTITY, "the spawn boundary treats a peer name as hostile") and
  are routed there, not used to close 211i. 211i exercises wrapper and delegate errors: no
  args, missing/unknown alias, absent or malformed `meta`/`config`, and the pre-main
  `AE_PATH` guard (ae:14711-14716).
- **SC-211d `requests`, SC-211j `retire`, SC-211c `memo`, SC-211e `peek`, SC-211f
  `agents`, SC-211l `say`:** input classes come from the census, including no-arg
  defaults, extra-arg handling, ambiguous and out-of-session targets, negative and
  leading-plus numerics, `--all` variants, and a real-TTY invocation separated from a
  redirected empty stdin.

**A-H4 `_lib` name resolution grammar (SC-211p).** Observed on the generated `_lib`
DIRECTLY — the case sources the exact producer-derived `_lib`, calls `ae_resolve`, and
captures its rc together with `AE_RESOLVED_PANE/AGENT/SLOT/SESSION`. `focus` is NOT the
observation surface: it mutates client focus, emits an event, and its failure can come
from the downstream tmux operation rather than the grammar. In particular the raw `%*`
branch returns 0 unconditionally (ae:12885-12901), so a dead pane id is resolved by the
grammar and only the later operation fails — a `focus`-based pair would have measured
liveness and labelled it grammar. Inputs cover each branch and the malformed cross-session
forms `@session`, `@:agent`, `@session:`; session-exists-but-agent-missing is kept
distinct from session-missing. The `tmux has-session -t <name>` prefix behaviour is
recorded as a separate confounder rather than being allowed into the fixture.

**A-H5 codex identity registration (SC-211o).** `_register-sid` takes a SLOT
(ae:14750-14824): it reads `launch_id.<slot>` / `launch_time.<slot>` from meta, scans
today's and yesterday's Codex JSONL directories, selects by launch-id token then by CWD
fallback with an mtime preference, and writes the discovered UUID to `codex.<slot>.sid`.
The v3 arm modelled an API that does not exist (an id argument, a malformed id, pane
identity) and is discarded. The arm invokes `_register-sid <slot>` against H6's cohorts,
varying one fact at a time — matching vs wrong launch-id token, mtime before vs after
`launch_time`, two eligible files with different mtimes, a malformed or missing first-line
id, today vs yesterday, and the CWD-fallback path — and captures the resulting
`codex.<slot>.sid`, the meta bytes before/after, and the candidate files' facts. Whether a
caller may name another slot is captured as its own observation, not treated as pane
identity.

**A-H6 launch-artifact publication (D14b) — SPLIT, because the v3 arm grouped two writer
classes.** `write_launch_script` has exactly ONE call site, `send_agent_cmd` at ae:12602;
`doctor --refresh` calls `sync_session_assets` (ae:8610), whose body writes helpers and
shims and whose own comment records that `send_agent_cmd` writes `launch.<slot>.sh`
afterwards (ae:17631-17632). So the arm is two arms: **(i)** the launch-script and
`.started` marker publication, exercised on a real `send_agent_cmd` launch write; **(ii)**
`doctor --refresh` helper/shim publication, exercised on the refresh path. Both are
mutating arms with the write witness beside the content manifest, because a byte-identical
regeneration is invisible to a content hash (A8). A before/after pair showing no write is
not evidence of regeneration in either arm, and any zero-writer reading requires the
recorder to be demonstrated on a write that is known to occur — the demonstration is
performed on arm (i)'s real launch write.

**A-H7 meta-writer fault arm (SC-1301) — writer-shaped cuts, not one shared cut.** The
three writers do not share a boundary: `start_capture_session_id` (ae:2068-2075) and
`_cmd_spawn` (ae:11938-11945) APPEND directly to canonical `meta` under `flock 200`, and
only the typed writer publishes via temp+rename. A hook may block or emit; it may not turn
a direct append into an atomic publication, which is what a single temp/rename cut would
have done. So: for the atomic writer, a barrier at temp-complete/pre-rename; for each
direct-append writer, controller-applied partial canonical-byte states at a barrier, with
the writer named in every artifact. Each cut carries a can-fail control. One hook-only
patch over an exact 72c7293 copy, inactive-equivalence proven before any capture.

### SC-211l containment — the one row whose blast radius leaves the sandbox

`helper_say_main` (ae:14470-14486) makes no network call: it appends a `chat` event and
prints one line (ae:14485). The hop that leaves the machine belongs to a separate bridge
process that tails a session's events file, so containment addresses the WATCHER.

**Layer 1 — structural, and this is the load-bearing layer.** The bridge derives its root
from `AE_HOME` (`telegram-daemon:10-11`), and this batch's fixtures place `HOME` and
`AE_HOME` in the arm's own temp tree under a random name created after the census. A
watcher's reach is a property of its environment, and it is INHERITED ACROSS FORK: the
live daemon (measured) forks a short-lived child per poll cycle, so a child cannot reach
what its parent cannot. The census therefore enumerates long-lived ROOTS and their
`AE_HOME`-derived `SESSIONS_DIR`, which is what makes it sound despite the child
population changing between samples.

**Layer 2 — the census, corroborating.** Run before AND after the measured invocation,
because a new root can appear mid-arm. Two hazards are designed out rather than filtered:

- **A census whose own command line contains its search string counts itself.** The
  bracket trick protects `grep` alone, not the other processes in its pipeline. This
  census therefore does not match on argv at all: the pattern is read from a variable, the
  comparison happens inside the shell, and classification is by REACH (the process's own
  `AE_HOME`) rather than by name.
- **Harness processes carry the fixture's `AE_HOME` by construction**, so reach alone
  over-reports. The arm's own processes are identified by a token the arm exports into its
  environment — a property no foreign process can hold — and the exclusion must be
  DEMONSTRATED to fire (a token-carrying process is shown excluded) rather than assumed.
  The in-range control is spawned WITHOUT the token, so the census must still report it.
  Both directions are required: report the control, exclude the harness.

**Layer 3 — PATH-first `curl`/`wget` stubs that refuse and log.** Scope stated honestly:
these contain only processes the ARM spawns and their children. **An already-running
bridge does not inherit the arm's PATH and is NOT contained by them** — the v3 claim that
"nothing in the arm's environment can reach the network" overstated it. If a fixture
bridge is deliberately started, it is started with the stubs already in its environment
and its own process env/argv is captured to prove it.

**Rejected approach, kept with its reason:** `lsof` on the fixture's `events.jsonl` was
this design's first containment check and is wrong — the bridge POLLS rather than holding
the file open, so a single-instant probe returns nothing while a watcher is fully active.
It could only ever return the reassuring answer.

**What the surface says about delivery — captured, not classified.** `say` prints
`Sent to Telegram bridge (chat): …` (ae:14485) after appending the event. The arm captures
that line's bytes with NO in-range watcher, and with the controlled logging-only watcher
in range, beside the appended event bytes and the watcher's log.

## Ordering within the batch — dependency order, not consumption order

Batches are sequenced by which phase consumes them; rows inside a batch by what everything
else depends on. SC-211p (`_lib` name resolution) and SC-211o (identity registration) run
FIRST — the `send`/`ask` path stands on both — then the argument surface, then dispatch and
version, then D14b's two arms, then SC-1301.

## Lanes and environment

Bash lane only, on a frozen 72c7293 copy. The rust lane is HELD by seat ruling until the
Rust listing surface is wired to a real session source. Per arm: TZ=UTC, pinned UTF-8
locale, scrubbed env, single-threaded, frozen commit verified by hash. Artifacts under
`batch-h-artifacts/` with the four-check gate (citations, per-case schema + content-bound
case index, SHA256SUMS coverage and verification, committed-bytes fidelity) run as the
LAST act before any handoff.

---

## SEAT ANNEX — never included in any worker brief

Everything below states expected relations, candidate implementations, or per-input
outcome labels. It is excluded from every execution brief; the region above is complete
without it.

- **SC-211p opposed pairs.** Each branch is given one input it can resolve and one it
  cannot, so a first-match resolver and a require-unique resolver return different
  answers, and so that a dead `%pane-id`, an ambiguous bare name and a
  session-exists-agent-missing form do not collapse into one refusal. The `%*` branch's
  unconditional `return 0` is why liveness must not be read as grammar.
- **SC-211o.** The cohorts vary one selection fact at a time so that a token-first
  selector, an mtime-first selector and a CWD-fallback selector return different sids.
- **SC-1301.** The interesting window for the atomic writer is between temp-complete and
  rename; for the direct-append writers there is no such window, and the question is what
  a reader observes mid-append.
- **D14b.** The two arms exist because a byte-identical regeneration and no write at all
  are indistinguishable to a content hash.
- **SC-012b/SC-014.** The three help spellings and the three version spellings are
  captured separately so that any divergence between them is observable.
