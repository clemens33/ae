# Batch H-HELPER — design

**STATUS: DRAFT — worker-authored (opus5:cexec), awaiting seat approval. No arm runs until
this is approved.** Authored on the lead's instruction after the boundary ruling of
2026-08-20 (twenty rows, the whole crit-assign H-HELPER set).


**Boundary: twenty rows — the whole `crit-assign.md` H-HELPER set.** The lead's earlier
nineteen was a miscount whose cause is recorded in the batch log: a grep pattern requiring
an `SC-` prefix skipped `D14b`, which is a D-record. `crit-assign.md` is the authority for
batch membership; a grep finds what it is shaped to find. All twenty are still CRITICAL in
`ratification-critical.md`.

The rows split into three MACHINERY classes, and that split — not the boundary — drives
execution order:

1. **Eighteen uniform surface rows** — invoke a surface, capture stdout/stderr/rc.
2. **D14b** — mutating; reuses A8's write witness beside the content manifest.
3. **SC-1301** — fault injection into three writers; hooks and a barrier, machinery
   nothing else in the batch needs.

**Value-blindness.** Every clause below is manipulation + barrier + capture ONLY. No
expected output, no expected rc, no pass/fail label appears in this design or in any
artifact it produces. Seats classify. A capture is a candidate observation until seat
acceptance.

## Instrumentation admissibility

Governed by `cluster-plan.md`'s global rule "Instrumentation admissibility" — the normative
home. Batch-local specifics, none of them exceptions:

- **The helpers under test are GENERATED artifacts.** Producer-derivation therefore binds
  harder here than in Batch C: every helper byte exercised by this batch must come from a
  real frozen `ae` launch (or the frozen `doctor --refresh` path), never from a
  hand-written script. A hand-rolled `state` is not the `state` the contract is about.
- **The environment is an instrument** (A1 locale incident): every live arm proves the TAB
  round-trip in its own environment before any capture, per the global rule.
- **Several of these helpers WRITE** (`state`, `goal`, `memo`, `spawn`, `retire`, `say`,
  `_register-sid`). Those arms are MUTATING and carry A8's machinery: declared
  harness-touched set, content change record, inode/mtime/size write witness, and the two
  per-case instrument controls fired AFTER the measured invocation.
- **Refusal paths need a positive control in the same fixture.** A refusal capture and a
  broken fixture look identical from stdout, so every arm runs at least one invocation
  known to take the non-refusal path through the same helper in the same case. A blind
  instrument and a refusing product produce the same empty stdout.

## Refusal and absence rows — the three-way discriminator (normative for this batch)

Eighteen of these twenty rows are refusal or malformed-mode rows, so their claims are
ABSENCE claims, and Batch C's lesson applies in full: **three different states produce the
same empty stdout** —

1. the surface REFUSED,
2. the surface was NEVER REACHED (the helper was not found, not executable, ran with the
   wrong `cwd`, or died before its own argument handling),
3. the READER was blind (the capture harness produced nothing whatever the surface did).

A capture that cannot separate those three is not evidence about a refusal. Every case in
this batch therefore carries all three legs, and no arm is admissible without them:

**Leg 1 — the surface's own report.** stdout, stderr and rc captured as separate byte
streams per invocation, never merged. The rc is captured as the helper's own exit status,
distinguished from the shell's `126`/`127`, which are reports about the invocation rather
than from the surface.

**Leg 2 — the SOURCE state and an execution witness.** *(Twin admissibility is governed by
"The xtrace twin" below; a helper whose twin fails equivalence keeps leg 1 and leg 3 and
loses this leg, explicitly.)* Each case writes
`surface-state.txt`: for the helper under test, its resolved path, whether it exists, its
type, mode, size and sha256, its interpreter line, and the `_lib` it sources — read from
the filesystem, as the invoking uid, exactly as A9's `meta-state.txt` separated "absent"
from "present but unreadable". Beside it, each refusal invocation is repeated as a
CONTROLLER-ONLY xtrace twin (`BASH_XTRACEFD` to a separate file, never the primary
capture) whose trace bytes show which guard the invocation actually reached. The twin is
harness output and is hashed separately from product state, per the global admissibility
rule; the primary capture is untraced and is the evidence.

**Leg 3 — a positive control in the same case, through the same helper.** One invocation
known to take the non-refusal path, so a case that captures nothing everywhere is
distinguishable from a case whose reader never worked. A blind instrument and a refusing
product produce the same empty stdout — the control is what tells them apart, and a case
whose control produces nothing is INCONCLUSIVE, not a refusal finding.

Nothing in these three legs states which outcome is correct. They exist so that a seat
reading "stdout empty, rc non-zero" knows whether it is reading a refusal at all.

## The xtrace twin — admissible only where it is proven inert

The twin is the execution witness of leg 2, and it is INSTRUMENTATION, so cluster-plan's
inactive-equivalence rule applies one layer down: **the twin must be proven equivalent per
HELPER, not once for the batch.**

- **The fd is chosen against a measurement, and then asserted at run time.** Frozen `ae`
  uses low fds and two high ones for locking: the seat's census records 2, 7, 8, 9, 10,
  200 and 201, with fd 9 fourteen times, fd 200 thirteen and fd 8 nine; my own narrower
  census (exec/append/dup/flock-argument forms only) independently finds 8 and 9 and
  misses 200/201, which is a fact about my patterns, not a disagreement about the code.
  Both censuses agree on the only claim this design needs: **nothing at or above 250 is
  used by any measurement.** `BASH_XTRACEFD` is therefore set at 250+, and the arm
  ASSERTS the descriptor is closed in the invoking shell before use rather than trusting
  either census. The hazard is concrete: fds 8/9 are `flock` descriptors and 200/201 are
  the meta writers', so a twin that landed on one would collide with the locking the
  helper is performing and the trace would describe the twin rather than the primary.
- **Equivalence is proven, per helper, on the same case**: the twin must produce
  byte-identical stdout, byte-identical stderr and an identical rc to the untraced
  primary. The comparison is over the PRIMARY's own captures, not a re-run pair.
- **A helper whose output differs under trace has INADMISSIBLE trace evidence**, recorded
  as such by name. Its primary capture still stands; the case simply loses leg 2's witness
  and says so, rather than quietly reporting a trace that describes a different execution.
- The twin is controller-only output: hashed and manifested separately from product state,
  never merged into the primary capture, per the global admissibility rule.

## Fixture groups

Built once, sealed, fingerprinted, cloned per case exactly as Batch C's templates.

| Group | Fixture |
|---|---|
| H1 base | live session, two agent panes, the full generated helper set from a real launch |
| H2 dead-pane | H1 with one agent pane killed (named mutation; pane id retained in meta) |
| H3 shell-pane | H1 with one agent pane replaced by a plain shell — the `interrupt` refusal path's own precondition (ae:14673) |
| H4 no-server | H1's directory with no tmux server at all |
| H5 cross-session | two live sessions on one server, colliding bare names and one unique alias — `@session:agent`, alias-only, bare-name and ambiguous resolution in one fixture |
| H6 codex-identity | a session whose roster carries a codex-shaped slot, for `_register-sid` |
| H7 request-state | H1 plus a produced ask→reply pair and one unanswered ask, for `requests` mine/inbox/all |

Every member is producer-derived: launched by the frozen `ae`, mutated only by NAMED
mutations with the byte diff recorded beside the member.

## Arms

**A-H1 top-level dispatch (SC-012b, SC-014).** `ae -h`, `ae --help`, `ae help`, `ae
version`, `ae --version`, `ae -V`, each invoked SEPARATELY into its own capture, plus one
unknown-flag invocation as the negative control. One dispatch branch handles the help
trio and one the version trio (ae:16841-16848); the arm captures each spelling's own
stdout/stderr/rc bytes rather than asserting they coincide — seats compare.

**A-H2 steward (SC-013).** `ae steward --help` captured plainly; `ae steward --detach`
captured under a BOUNDED wait with an explicit INCONCLUSIVE outcome on timeout, its
process reaped in teardown, and the file manifest before/after recorded because a detach
starts something.

**A-H3 helper refusal/malformed matrix (SC-211a–j, l, n; SC-212c).** One case per helper,
each invoked as a real agent from a real pane (the `as_agent` path, so `$TMUX_PANE` and
the pane environment are genuine), with an argument matrix per helper:

| Row | Helper | Argument classes invoked, each its own capture |
|---|---|---|
| SC-211a | `state` | no args; each legal mode; `blocked` with and without a reason (ae:12855); an unknown mode; a mode with leading `-`; empty-string mode |
| SC-211b | `goal` | no args; set; `--clear`; two positionals (ae:14569); empty-string text |
| SC-211c | `memo` | `add` with and without text; `add --topic` with and without a topic and text (ae:14507); `read`/`read --topic` arity (ae:14528); `tail` with non-numeric and with excess args (ae:14537-14539); unknown subcommand |
| SC-211d | `requests` | `mine`, `inbox`, `all`, an unknown mode (ae:14412), and `mine` invoked where no agent identity is detectable (ae:14417) |
| SC-211e | `peek` | no args; unknown agent; `%pane-id`; `@session:agent`; non-numeric lines (ae:14607); zero; a line count far beyond the pane's history |
| SC-211f | `agents` | plain; `--all`; inside a session whose meta is unreadable; with an unknown argument |
| SC-211g | `focus` | no args (ae:14642); unknown agent; ambiguous bare name; `%pane-id`; `@session:agent` |
| SC-211h | `interrupt` | no args; unknown agent; a DEAD pane; a SHELL pane with a message (ae:14673 — H3 exists for this); with and without a message |
| SC-211i | `spawn` | no args (ae:14718); a name violating the agent-name grammar; a name containing `:` twice; a 64+ character name; an unknown alias |
| SC-211j | `retire` | no args (ae:14740); unknown agent; `%pane-id`; an agent that is not spawned |
| SC-211l | `say` | no args with an empty stdin; whitespace-only text (ae:14480); text via argv; text via stdin — **runs only under the containment of the SC-211l section** |
| SC-211n | `events-tail` | plain; against a session with no `events.jsonl` yet (ae:14897); bounded follow with a produced event; an unknown argument |
| SC-212c | `requests` | the three-mode signature captured as a set, from an agent that is both a sender and a target (H7) |

Every case captures per invocation: stdout, stderr, rc, the delegated tmux argv, the
before/after file manifest, and — for the writing helpers — the write witness and change
record. Each case also runs its own positive control invocation through the same helper.

### SC-211l containment — the one row whose blast radius leaves the sandbox

Every other write in this batch lands inside the fixture's `AE_HOME`. `say` does not stop
there, and A8's mutation machinery — harness-touched set, content record, write witness —
covers filesystem writes and says nothing about a network side effect.

**Measured mechanism first.** `helper_say_main` (ae:14470-14486) performs no network call:
it appends a `chat` event via `ae_emit_event` and prints one line. The hop that leaves the
machine belongs to a SEPARATE bridge process which tails a session's events file. The
blast radius is therefore INDIRECT — a bridge whose watch root contains the fixture would
forward what the arm writes — and containment has to address that path, not the helper.

**Three layers, and none of them is trusted without a demonstration.**

1. **Structural.** The bridge takes its root from `AE_HOME` (`telegram-daemon:10-11`:
   `AE_HOME="${AE_HOME:-$HOME/.ae}"`, `SESSIONS_DIR="$AE_HOME/sessions"`), and this batch's
   fixtures set `HOME` and `AE_HOME` into the arm's own temp tree. The arm CAPTURES, per
   run, every live bridge process with its own environment read from the process (not from
   this design's assumption), and the fixture path, so "outside the watch root" is a
   recorded comparison rather than a claim. Recorded before the measured invocation AND
   after it, because a bridge can start mid-arm.
2. **Behavioral.** The arm runs under PATH-first `curl`/`wget` stubs that REFUSE and log
   rather than delegate. Nothing in the arm's environment can reach the network even if a
   surface tried.
3. **Demonstrated.** A recorder nobody has seen fire is not evidence of silence — the
   SC-814 canary lesson. Before any zero is relied on: the `curl` stub is invoked
   deliberately in the same arm and its log line captured, and the bridge census is run
   against a deliberately IN-RANGE control process (a throwaway process whose `AE_HOME` is
   the fixture) and must report it. A census that cannot report an in-range watcher cannot
   report their absence, and the arm is INCONCLUSIVE rather than contained.

**Rejected approach, kept with its reason:** `lsof` on the fixture's `events.jsonl` was the
first containment check proposed here, and it is wrong. The bridge POLLS rather than
holding the file open, so a single-instant probe returns nothing WHILE A WATCHER IS FULLY
ACTIVE — a zero that means nothing, presented as containment. It could only ever return
the reassuring answer. The census-with-an-in-range-control replaces it.

**What the surface says about delivery — captured, not classified.** `say` prints
`Sent to Telegram bridge (chat): …` from its own `printf` (ae:14485) after appending the
event. The arm therefore captures that line's presence and bytes under an OPPOSED PAIR:
once with NO in-range watcher at all (the census reporting zero, itself demonstrated
against the control), and once with the controlled logging-only watcher of layer 3 in
range — a process that reads the fixture's events file and writes to a local log, never to
a network. The two captures sit beside the appended event bytes and the watcher's log.
Nothing here states what that line means or should mean; the pair exists so a seat can see
whether the surface's own report varies with whether anything is listening.

**A-H4 name resolution grammar (SC-211p).** H5's colliding fixture, invoked through a
helper that resolves and then refuses cheaply (`focus`).

SC-211p is a GRAMMAR row, and grammar evidence has to show the grammar DISCRIMINATING
rather than merely refusing: distinct causes that collapse into one refusal message would
be faithfully reproduced as a collapse by anyone reimplementing from the evidence. So each
resolution branch gets an OPPOSED PAIR — one input that the branch can resolve and one it
cannot, differing only in the property that branch is about:

| Branch | Resolves | Does not resolve |
|---|---|---|
| bare name | a bare name unique across the fixture | a bare name carried by two agents |
| `%pane-id` | a live pane id | a pane id whose pane is dead (retained in meta) |
| `@session:agent` | an agent that exists in the named session | a session that exists with NO such agent |
| `@session:agent`, session leg | the other live session | a session name that does not exist |
| alias-only | an alias unique in the fixture | an alias carried by two agents |
| exact `alias:name` | the exact pair | an exact pair whose name is not in that alias |

**Scope ruling on the lead's open question:** cross-session `@session:agent` and
alias-only-when-unique DO get their own pairs, for the same reason as the other three —
each is a distinct branch of the same grammar (ae:12878 onward) whose failure lands in the
same refusal, so dropping them would leave exactly the collapse this row exists to prevent.
They are not out of scope; they are two more rows in the table above, and H5 already
carries two live sessions, so they cost fixtures nothing.

An empty string and a name containing the `:` delimiter twice are captured as inputs that
belong to no branch.

**A-H5 codex identity (SC-211o).** H6. `_register-sid` invoked from the codex-shaped
pane with a well-formed id; from a pane whose slot differs from the one named; twice with
different ids; with a malformed id. Mutating arm: meta bytes captured before and after
each invocation, plus the witness and change record.

**A-H6 doctor --refresh, launch-artifact half (D14b).** Mutating arm. `launch.<slot>.sh` and the rest of the regenerated set
captured byte-for-byte before and after `ae doctor --refresh`, with the write witness
beside the content manifest — A8 proved a content manifest cannot see a byte-identical
regeneration, which is the most likely shape of this diff. The `launch.<slot>.started`
marker's presence is captured before and after.

**A-H7 meta-writer fault arm (SC-1301).** Hooked barrier arm under the global admissibility rule: ONE hook-only patch over an exact
72c7293 copy, with the inactive hook proven byte/rc/file/tmux-equivalent to the unmodified
control before any capture. A fault point sits between the temp write and the rename in
each of the three meta writers; the CONTROLLER performs the abort at the barrier; a reader
runs concurrently and its rendering is captured at the barrier and after. Captures: the
meta bytes at each barrier, the reader's stdout/rc, and the directory manifest — never a
statement about which of the two readings is correct.

## Per-row differentiators (the discriminating manipulation per row)

| Row | Discriminator |
|---|---|
| SC-012b | the three help spellings invoked SEPARATELY into separate captures — a single shared capture cannot show a divergence between them |
| SC-014 | version invoked through all three spellings, and the emitted string captured verbatim including its prefix |
| SC-211d | one invocation from a pane where identity IS detectable and one where it is not, same fixture (ae:14417) |
| SC-211e | a line count beyond the pane's real history, so a clamping reader and a failing reader differ |
| SC-211h | the SHELL-pane case with a message present, which is the only input that reaches ae:14673 |
| SC-211i | a name that is legal as a config key but ILLEGAL as an agent name (leading `_`), so the two grammars are distinguishable |
| SC-211p | a fixture containing BOTH an ambiguous bare name and a unique one, so first-match and require-unique resolvers disagree |
| SC-211o | a registration whose slot does NOT match the invoking pane, so a slot-bound writer and a slot-blind one disagree |
| SC-1301 | the abort placed BETWEEN temp write and rename, the only window where a torn read is possible |

Each refusal row's own "never reached" hazard is named in its case: for the helpers that
resolve first (`peek`, `focus`, `interrupt`, `retire`) a resolution failure and an argument
refusal both exit non-zero, so the xtrace twin is what separates them; for `spawn` and
`retire` the `AE_PATH` guard (ae:14711-14716) fires before argument handling, so each case
records whether that guard was reachable in its fixture; for `requests` the identity guard
(ae:14417) precedes mode validation, so the two are invoked as separate cases rather than
one.

## Lanes, ordering, environment

Bash lane only, on a frozen 72c7293 copy. The rust lane is HELD by the lead's ruling until
`listing.rs` (and the helper surface it implies) is wired to a real session source; a
paired lane before that captures one fact N times. Per arm: TZ=UTC, a pinned UTF-8 locale,
scrubbed env, single-threaded, frozen commit verified by hash. Artifacts under
`batch-h-artifacts/` with the same four-check gate (citation resolution, per-case schema +
content-bound case index, SHA256SUMS coverage and verification, committed-bytes fidelity)
run as the LAST act before any handoff.

## Ordering within the batch — dependency order, not consumption order

Batches are sequenced by which phase CONSUMES them. Rows inside a batch are sequenced by
what everything else DEPENDS ON, which is not the same thing and, inside a batch, beats it.

SC-211p (`_lib` name resolution) and SC-211o (codex identity registration) run FIRST: the
`send`/`ask` path stands on both, so if P1 does not reproduce them, nothing downstream of
them is testable at all and every later row's evidence would be read through an unproven
resolver. Then the refusal/malformed matrix, then dispatch and version, then D14b
(mutating machinery), then SC-1301 (hooks and barrier) last, because it needs machinery
nothing else in the batch needs.
