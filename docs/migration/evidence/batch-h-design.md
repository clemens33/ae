# Batch H-HELPER — design

**STATUS: DRAFT v9 — worker-authored (opus5:cexec). Two REQUEST-CHANGES rounds addressed. No arm runs until both seats approve.** v1-v3 are in git history;
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
- **(b) A pre-registered, non-adaptive capture program — RULED LOAD-BEARING.** Every arm
  script is committed and hash-registered BEFORE its first run, its sha256 in the run
  manifest; any amendment is durable and REOPENS the affected arms. Adaptive capture is
  then a diff rather than a promise. A fresh executor is additive where cheap.

**Exposure record (required).** The run manifest records that this design's author saw
answer-labelled source outcomes in v1-v3 — `interrupt`'s silent non-refusal path, `goal`'s
arity guard, the `%*` branch's return, `_register-sid`'s selection order, and `say`'s exact
stdout — before they were moved to the annex, so a seat reading the captures knows what the
author knew.

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
  controls under the BEFORE-AND-AFTER chronology below. (An earlier draft said "fired
  AFTER" here and "before and after" below; an instrument contract stated twice with two
  values is not a contract.)
- **Every invocation is BOUNDED.** A timeout produces its own INCONCLUSIVE artifact naming
  the bound; it is never reported as a refusal or as a product rc.
- **Controls fire BEFORE and AFTER every measured invocation, not after only.** A post-run
  canary cannot show the capture and witness paths were live WHILE the product ran — the
  chronology failure already seen in A1. The pre-canary COMPLETES before PRODUCT-START and
  the post-canary begins after PRODUCT-COMPLETE, both content-bound to that case in the
  append-only ledger, and BOTH must pass for the observation to be admissible. This
  supersedes A8's post-only construction, which proved the instrument responsive at the
  after-snapshot rather than across the run.

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
unusable twice over: it states a product expectation, and a non-refusal path may
legitimately emit nothing at all, which makes "the control printed nothing" uninterpretable
as a canary. The control is
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

## Citations are pinned, not asserted

Every `ae:NNNN` in this design, the census and the generated list is resolved against
`git show 72c7293:ae` by `batch-h-tools/check-citations.py`, which emits
`batch-h-citation-pins.md` carrying the cited line's own TEXT — so a seat reads source
rather than trusting arithmetic. `--check` fails on a stale committed pin file and
`--redproof` proves the tool can report red. The count is whatever the tool reports; it is
not restated here, because a count in prose has lost the invocation that produced it.

It exists because the steward citations here were wrong by a constant 8: transcribed from a
windowed grep, offsets written down as absolute lines, under a claim of exact verification.

**Two limits, stated before anyone cites it.** It pins a range's ENDPOINTS, not the lines
between them — the artifact is named endpoint pins for that reason. And a pin proves a
citation RESOLVES, never that it is the RIGHT line for the claim beside it: the +8 steward
citations would have passed it clean, because those were real lines with real text. What
caught them was a seat reading the source. The tool makes that reading cheap; it does not
perform it.

## Required pre-step — the argument census

Before any arm is written, the argument handling of each frozen helper, the delegated
`_cmd_spawn`/`_cmd_retire` paths, the top-level dispatcher and the `steward` flag surface is
enumerated into a committed table, returned for a seat gate with an explicit row ->
input-class mapping. The executor receives the seat-approved INPUT LIST, which is GENERATED
from the census by a committed script that drops every non-input COLUMN — a columnar drop,
not a vocabulary filter, so no outcome label reaches the brief whatever it is called — and
is gated by a committed checker (diff-clean, set equality both directions, scope-column
validation, duplicate and conflicting-ownership detection performed BEFORE the scope filter,
a novel-label injection proving the drop, and a lexical belt), with red arms on both the
generated list AND the census source — including in->OOB, OOB->in, duplicate in-scope,
duplicate OOB, conflicting ownership, an invalid scope value and a missing Scope column. Any
blind arm fails the run. Scope is an explicit validated COLUMN, not a substring found in
prose. The table records: for every input class, whether it is ACCEPTED, REJECTED, IGNORED, or
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

**A-H1 top-level dispatch (SC-012b, SC-014).** `ae -h`, `ae --help`, `ae help` (SC-012b)
and `ae version`, `ae --version`, `ae -V` (SC-014), each invoked separately into its own
capture and recorded as a spelling FAMILY. The unknown-LONG-OPTION class belongs to SC-022
and a non-option word enters the launch path; both are marked OUT-OF-BATCH in the census,
neither can close SC-012b, and the generated brief excludes them. They stay in the census
because an earlier version collapsed them into "an unknown first word", erasing a
distinction the contract draws.

**A-H2 steward (SC-013).** SC-013 owns the HELP and DETACH spellings only: `-h`,
`--help`, `help`, `--detach`, `--no-attach`, help with trailing arguments, and the
selector-order and repeated-selector classes that the iterative parser makes reachable
(ae:16730-16758). Detach invocations run under a bound with the process reaped in teardown
and the file manifest captured before and after. `--init`, `--attach`/`--switch`, a bare
`steward`, the `hub` spelling and a positional argument are OTHER ROWS' (SC-932, SC-931,
SC-930, SC-939f); they appear in the census as out-of-batch cross-references and NO arm
here closes them. `contrib/aesteward` at 72c7293 carries no executable, so what these
flags reach is captured rather than assumed.

**A-H3 helper argument surface (SC-211a-j, SC-211l, SC-212c).** One case per helper, each
invoked from a real pane, with the input classes drawn from the argument census. Known
corrections to the v3 matrix, all from the review and all verified against frozen source:

- **SC-211b `goal`:** input classes include multi-word text, `--clear` alone, `--clear`
  with a trailing argument, empty text, and `-h`/`--help` as their own inputs. (v3's
  "two positionals" class rested on a source misreading; the census supersedes it.)
- **SC-211i `spawn`:** the row is `spawn` NON-NAME argument errors, so name-grammar inputs
  are routed to SC-1201 (F-IDENTITY) rather than used here. Input classes come from the census's WRAPPER rows (no args; an environment in which the
pre-main `AE_PATH` guard at ae:14711-14716 is reachable) AND its DELEGATED `_cmd_spawn`
sub-census (an alias not defined in the config, a session absent from meta, a session named
but not running, meta present but unlockable, a legal alias with no `:name`). `retire`
likewise draws on the `_cmd_retire` sub-census — main-agent reference, configured worker,
reference absent from `agent.spawned.*`, ambiguous bare name, `%pane-id` outside the
session, extra arguments.
- **SC-211d `requests`, SC-211j `retire`, SC-211c `memo`, SC-211e `peek`, SC-211f
  `agents`, SC-211l `say`:** input classes come from the census, including no-arg
  defaults, extra-arg handling, ambiguous and out-of-session targets, negative and
  leading-plus numerics, `--all` variants, and a real-TTY invocation separated from a
  redirected empty stdin.

**A-H4 `_lib` name resolution grammar (SC-211p).** Observed on the generated `_lib`
DIRECTLY — the case sources the exact producer-derived `_lib`, calls `ae_resolve`, and
captures its rc together with `AE_RESOLVED_PANE/AGENT/SLOT/SESSION`. `focus` is NOT the
observation surface: it mutates client focus, emits an event, and a failure it reports can
originate downstream of the grammar rather than in it, so grammar and liveness would be
confounded in a single rc. Inputs cover every branch of the resolver plus the malformed
cross-session forms `@session`, `@:agent`, `@session:`; session-exists-but-agent-missing is
kept distinct from session-missing. **Environment-equivalence control:** the controller
shell that sources the generated `_lib` records its effective `_AE_SESSION`,
`_AE_SESSIONS_DIR`, tmux selector, cwd and exported globals beside those of a real
generated-helper invocation from the same sealed fixture, so a correct function observed in
a different resolution domain is visible as such. The `tmux has-session -t <name>` prefix behaviour is
recorded as a separate confounder rather than being allowed into the fixture.

**A-H5 codex identity registration (SC-211o).** `_register-sid` is invoked as
`_register-sid <slot>` (ae:14750-14824) against H6's cohorts. The fixture varies ONE fact at a time across the candidate Codex JSONL files and the
slot's meta: launch-id token absent / matching / mismatched, file mtime before vs after
`launch_time.<slot>`, two eligible files with different mtimes, two with EQUAL mtimes, a
recorded cwd matching or differing from the invoking one, first-line id malformed or
missing, today vs yesterday directory, the default slot, and an explicitly named slot other
than the invoking pane's. The slot argument is scoped as TRUSTED INTERNAL input in this
batch — `_register-sid` is launched by ae, not by a peer — and hostile slot values belong to
the identity boundary rows if that scope is ever wrong. Captures: the
`codex.<slot>.sid` artifact, meta bytes before and after, and every candidate file's own
facts (path, mtime, first line). Invoking with a slot other than the invoking pane's is its
own input class. The v3 arm's id-argument inputs are discarded: they do not exist in this
surface.

**A-H6 launch-artifact publication (D14b) — HELD: the RECORD needs correcting before this
arm can gate anything.** The v3 arm grouped artifact classes with different writers,
effects and phase owners, and splitting the ARM does not split the RECORD. Four distinct
write events are in play — launch-script generation, marker CLEAR, marker CREATE (by the
generated script's own execution), and helper/shim publication — and they differ in writer,
atomicity and phase owner. Until the ownership record and its manifest/map/assignment are
corrected by the seat that owns them, this design carries them as separately-attributed
mutating probes with NO shared claim, and runs none of them. Each, when unblocked, uses the
write witness beside the content manifest; a before/after pair showing no write is not
evidence of regeneration, and any zero-write reading requires the recorder demonstrated on
a write known to occur.

**A-H7 meta-writer fault arm (SC-1301) — three writer-shaped cuts, three DIFFERENT evidence
claims.** The writers do not share a boundary, so neither the cut nor the claim can be
shared:

- **The atomic writer:** barrier at temp-complete / pre-rename, controller-performed abort.
  Claim: what a concurrent reader observes across that boundary.
- **`_cmd_spawn` (ae:11938-11945):** it performs SEVERAL of its own appends, so a hook
  BETWEEN two of them plus a controller SIGKILL yields a partial logical generation the
  FROZEN WRITER produced. Claim: an observed partial-generation state, attributed to the
  product's own writes.
- **`start_capture_session_id` (ae:2068-2075):** one append, so no such window exists. A
  controller-created partial line is admissible only as a READER-FAULT RESPONSE probe,
  labelled that way in every artifact, and is never reported as an observed writer tear.
  The untouched source writer is captured separately in the same case.

Every controller mutation names the exact writer-shaped bytes it wrote (the intended row's
prefix; newline present or absent) and carries a can-fail control. One hook-only patch over
an exact 72c7293 copy; a hook may block or emit and may not convert an append into an
atomic publication. Inactive equivalence proven before any capture.

**Cross-row family records — a per-row capture is not always legible alone.** Some
properties of this surface are only visible ACROSS rows, so the batch record places the
relevant captures side by side rather than leaving each in its own case directory. The
first is the USAGE-EXIT FAMILY: every surface in the generated helper set that has a usage
path is invoked into it, and its rc recorded beside the others in one generated table.
SC-211b's and SC-211l's captures are two rows of that table, not two isolated numbers. The
record states no relation between them; it exists so a seat reads the family. A second
family record covers the help/version spellings of A-H1 for the same reason.

**A-H8 long-lived query (SC-211n).** `events-tail` is not a refusal row: it is a long-lived
query, so refusal semantics do not apply and a generic timeout must never be read as a
product rc. The arm uses NAMED barriers and controller termination: (1) invoked against a session directory whose events file does not yet exist, with the
controller CREATING it as a named transition that the capture brackets; (2) producer-derived
replay cohorts of 29, 30 and 31 events with per-line provenance,
each read before any follow begins, so the frozen replay cut is exercised on both sides and
at the boundary rather than assumed; (3) a replay capture closed by a named event
barrier — the controller emits a known event and the capture closes when it appears; (4)
a follow capture across a second named event; (5) a partial final line written in TWO
steps with a barrier between them, so what the follower emits at each step is captured
separately; (6) an unknown-argv invocation captured beside the plain one. Termination is
performed by the controller AFTER the named capture barrier and recorded as a controller
action — never as an rc, never as a timeout. SC-1306e's snapshot claim is cross-referenced,
not classified here.

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

**Layer 2 — the census, corroborating.** The chronology is fixed and recorded in the
ledger: system-root census -> creation of the randomly-named isolated `AE_HOME` and the
producer launch -> immediate pre-invocation root census -> the measured invocation ->
immediate post-invocation census. A fixture bridge, if one is started at all, inherits the
refusing stubs before it starts and appears in the pre-invocation census with its own
environment captured. A root that both appears and exits between the two immediate censuses
remains a stated LIMIT, bounded only by the parent-root inheritance argument, which is
proven for each bridge mode this batch uses. Two hazards are designed out rather than filtered:

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

**What the surface says about delivery — captured, not classified.** The arm captures
`say`'s own stdout bytes (ae:14485) under an opposed pair — with NO in-range watcher, and
with the controlled logging-only watcher in range — beside the appended event bytes and the
watcher's log.

## Ordering within the batch — dependency order, not consumption order

Batches are sequenced by which phase consumes them; rows inside a batch by what everything
else depends on. SC-211p (`_lib` name resolution) runs FIRST — the `send`/`ask` delivery path stands on it.
SC-211o is NOT a prerequisite for delivery: `_register-sid` owns Codex resume identity, and
the earlier draft's ordering repeated the API conflation in prose. It runs with the
argument surface. Then dispatch and version, then D14b's probes once its record is
corrected, then SC-1301.

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
- **Leg 4.** Which helpers have a silent non-refusal path is an answer label; the brief
  needs only that the canary is controller-generated.
- **SC-012b/SC-014.** The three help spellings and the three version spellings are
  captured separately so that any divergence between them is observable.
- **SC-211b / SC-211i.** That `goal <two words>` reaches the set branch and that the arity
  guard belongs to `--clear` are answer labels; the brief carries input classes only.
- **SC-211p.** The `%*` branch's unconditional `return 0` (ae:12885-12901) is the answer
  label that motivates observing `_lib` directly.
- **SC-211o.** The selection ORDER and what the helper writes are answer labels; the brief
  varies the facts and captures the artifact.
- **D14b.** The writer attributions — `write_launch_script`'s single call site (ae:12602),
  the clear-vs-create split, the helper/shim path (ae:8610, ae:17631-17632) — are
  source-derived ownership findings, reported to the record's owner.
- **SC-1301.** Which writer appends and which publishes atomically is a source-derived
  ownership reading; the brief carries the cut shapes and the evidence labels.
- **SC-211l.** The exact text `say` prints, and that it prints it before any forwarding has
  occurred, are the answer labels.
