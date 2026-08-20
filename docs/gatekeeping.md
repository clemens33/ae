# Gatekeeping — the slice-gate craft

Distilled 2026-07-07 by the phase-3 session lead (Fable 5) from the s16–s20 gate record,
written so a lead on any capable model can run the same gates. The method is general;
the exhibits are ae's. Companion: `design-patterns.md` (what good designs look like),
`lead-handover.md` (where this codebase specifically bites).

The gate exists because two other strong agents — a builder and a cross-model reviewer —
have already passed the code green. Whatever the gate contributes must therefore be
something reviews structurally miss. In the phase-3 record, every gate-caught hole
(s18 default-mode violation, B5, B7b) shared one signature: **a global invariant
violated by a locally-correct diff**. That is the gate's job: read the diff against the
invariant, not against the code.

## The protocol

Run these steps in order. Mechanically. The checklist beats gestalt precisely on the
days you are tired, rushed, or running on a smaller model.

1. **State the binding invariant in one sentence before opening the diff.**
   ("No component runs twice in any mode; no double-send during handoff.") If you
   cannot, the slice was briefed wrong — stop and fix the brief, not the code.
2. **Enumerate the modes, default first.** For each: does the invariant hold? The
   default/unset path must be provably zero-diff unless the slice explicitly changes
   it. Tests rarely prove this — fakes don't run the real thing (the s18 lesson:
   suites were green while the default path double-ran live watchdogs).
3. **For every durable fact the diff introduces or touches** (marker, lock, pidfile,
   heartbeat, offset, state file), interrogate it:
   - Who writes it, who reads it, are those the same process? Same env? (If not, the
     fact must live in a file, not in env — strangers can't read your env.)
   - When does it decay, and who acts on decayed state?
   - Who cleans it up, and do they verify *ownership* before destroying it?
   - What happens when its maintainer dies mid-hold? Mid-write?
4. **For every loop**, ask: what drives its cadence? Does one timer now drive two
   concerns (B7b)? What happens when a tick outlives the interval? Is the
   maintain-the-fact step *before* or *after* the act-on-the-fact step (B6)?
5. **For every new branch**, name its failure direction: fail-open or fail-closed —
   and is that the *safe* direction for this branch? A bool return nobody checks is
   fail-open by accident (B1).
6. **Ask what the tests cannot see.** Every test harness has blind dimensions:
   fixture ticks vs wall-clock (hid B7b), fakes missing an argument dimension
   (FakeTmux ignoring `server=` hid B1-s18a), processes that never really run
   (hid the s18 violation), env assumed shared across processes (the s19 design
   BLOCKER). Name the blind spot out loud, then check that region by hand.

## The failure taxonomy

Confirmed holes from two campaigns — the s18/s19 slice family (phase 3) and the
2026-07 input-region/spawn campaign — generalized. Use as a diff-read checklist:
most map to a question in the protocol above, and each shipped past a builder *and*
a reviewer at least once before being caught.

| Class | The hole | The question that finds it |
|---|---|---|
| Fail-open claim (B1) | Claim/write reports failure; caller proceeds anyway | Is every claim's return value checked, and is the unchecked direction safe? |
| Owner-unaware cleanup (B2) | `finally` deletes shared state that may belong to a *newer* owner | Does cleanup verify ownership (pid/stamp) before destroying? |
| Partial-scope action (B3) | Fact spans N scopes (servers/sessions); action applied to one | Enumerate the scopes the fact covers; does the action cover all of them? |
| Incomplete fact (B4) | Fact claimed but not completed (marker without fresh heartbeat) before acting | Is the *full* fact — everything a stranger checks — true before the first act? |
| Decay-while-acting (B5) | Fact can rot while its holder keeps acting (heartbeat fails, sends continue) | If maintenance fails persistently, does the holder *stop* before others react to decay? |
| Maintain-after-act (B6) | Fact refreshed after the actions that depend on it | Within one iteration: maintain, verify, *then* act — is the order right? |
| Entry-only verify (B7a) | Fact verified at loop entry, not per irreversible action | Is the fact re-checked immediately before each send/delete/kill? |
| Coupled cadences (B7b) | One wait drives both liveness upkeep and component work; capping one over-drives the other | Which timer drives which concern? Does each component self-gate on its own due time? |
| Skip-path resets (B8/B9) | Skip/stale/resume paths bypass a safety step or reset a safety counter | Walk every early-return: what safety action does it skip, what state does it wrongly reset? |
| Default-mode violation (s18) | The unset/default path breaks the non-negotiable while all tests pass | Did *you* prove default zero-diff on a live system, not just in fixtures? |
| Completion without delivery | The success surface (stdout, event, ledger, report) asserts an outcome no branch proved; the failure branch printed and fell through | For each success line/event/return: which branch *proved* the effect landed? Is "pane created" being reported as "task assigned"? |
| Wrong-direction bound | A bound/threshold has two error directions; the code fails toward the silent, destructive one | Name **both** error directions for this bound. Which one is silent? Does the code fail toward the loud one? |
| Marker-grep as state | A substring is treated as evidence of a state; something else renders the same substring | What else can render that marker? Is the state decided by a structural predicate, or by a coincidence of text? |
| Unrepresentative fixture | The fixture's world does not match the live world; the code is correct about the fixture and wrong about reality | Has this predicate been run against a **real specimen** of what it senses — and does the real one look like the fixture? |
| Stale fixture | A fixture built for an older contract keeps passing after the contract moved; it now tests only itself | When a primitive's contract changed, which fixtures encode the **old** one? Does each still describe what the code now receives? |
| Ambient-derived identity | A recorded fact about *who acted* is inferred from ambient context that belongs to someone else | For every fact the code records about an actor, is it from a positively-owned signal, or from whatever the environment happened to hold? |
| Scope-shadowing override | Setting one index/key at a narrower scope replaces the whole container, silently dropping inherited entries | When you narrow scope to set one element, does the narrow scope **inherit** the container or **replace** it? |
| Display-form comparison | An identity is matched via its abbreviated or rendered form; distinct identities share that rendering, or the canonical spelling differs from what gets displayed | Is the comparison on the full canonical form — or on what a UI or emitter happens to print? |

Notes on the 2026-07 rows: *completion without delivery* is distinct from B1 —
B1 is a caller ignoring a returned failure; here the rc was captured and printed and
control still fell through to a positive report (helper `send`, watchdog sweep-nudge,
`_cmd_spawn` fixed `6fee8e4`, and worker memo-only handbacks — four independent
layers). *Wrong-direction bound* sharpens protocol step 5 for bounds where both
directions are failures: false IDLE clobbers a human's unsent draft (silent), false
OCCUPIED defers forever (loud) — three designs were rejected for failing silent.
*Unrepresentative fixture* is kin to the s18 row but asks whether the fixture's world
matches reality at all (the `⚙` sensor was green against a dummy-`sleep` fixture;
real agent subtrees hold persistent MCP services, so it could never go dark — feature
cut on the evidence, `4f63c19`).

Notes from the 2026-07-30 portability campaign: *display-form comparison* nearly
shipped three times in one afternoon — a session-id check that would have compared
UUIDv7 ids by their 8-char `ae list` rendering (same-second launches share the
prefix, so it would have false-passed a real cross-wiring and false-alarmed on the
fix), a shim-presence probe grepping `name()` where `declare -f` emits `name () `,
and two independently-written guards whose comment filter matched the `#!` line —
one of them reading a comment *describing* the absence of `source _lib` as evidence
of its presence (this row and marker-grep intersect there). *Ambient-derived
identity* gained its severity framing: both codex agents' registration fallback
captured the id of the human's own session running in the same cwd — a resume would
have replayed a private conversation into an agent context. Identity from ambient
signals is a **confidentiality** failure, not a bookkeeping bug. And for
detector/lint bounds, the loud direction is the false positive: a pinned, labelled
false positive is reviewable in the suite; an exemption list is fail-open (one
over-broad date exemption suppressed semantically-divergent fallbacks). Prefer
excluding forms *by construction* — a pattern that cannot match the BSD spelling
needs no allow-list at all.

Notes from the 2026-08 watchdog-footprint slice (#8, `d5275de`): the
*unrepresentative fixture* row has a general statement — **the specimen must come
from the layer the code reads**. Four instances in one session, same rule in a
different costume each time: a guard parsing *source* when the product is the
*generated* artifact (declare-f emission closure); a pane filter matching the bytes
ae *delivers* when the watchdog hashes what the TUI *renders* (it filtered nothing
at all in a live pane — every fixture passed); a filed root cause blaming a banner
that was merely *displayed* near the failure (#36 — refuted by a two-arm
experiment); and an identity check answered by a stub that was never asked about
the server in question (#39 C3). Capture the specimen with the same primitive the
code uses (`capture-pane` for pane readers, run-the-artifact for generated code) —
extracted, never retyped, so fixtures cannot drift from reality. And a process
corollary from the same slice, after a "be more specific" fix tightened the two
patterns the finding named while a third pattern in the same change stayed loose:
**a review finds instances; the builder owes the class**. When a finding's remedy
is a discipline, apply it to every sibling in the change, not only the named
exhibit.

Notes from the #30 resume-exec slice (`32719f5`), which closed a class across six
rounds: **a fact built upstream is transported, never re-parsed out of the
artifact it was baked into.** Four instances in one issue, all demonstrated on the
real pipeline: an id re-parsed from a context-injected command extracted the word
"by" from ae's own prose; a `codex*resume*` glob classified every fresh command as
a resume because the injected context contains the word "resume"; a marker search
for the injection boundary found the *user's* copy of the flag instead of ae's;
and a second, independent classifier in the delivery path answered the same
question from different evidence, double-delivering the initial prompt. The built
command is downstream data — hostile input, not a source of truth. When one fact
must drive two decisions (construction and delivery), one predicate answers both;
two classifiers over the same string will eventually disagree. And close a class
with a *guard*, not a sweep — the sweep missed the delivery-path member; the unit
guard that now forbids the glob spelling would have caught it. Two companion
rules from the same slice: **untouched is not unaffected** — a change that alters
which arm executes arms every latent defect in the newly-reachable arm, and the
diff shows nothing because the defect was already there; and **extraction
correctness is not probe correctness** — a probe whose fixture-extractor was
verified to pull the right lines can still be incapable of failing (the extracted
child aborted under `set -u` and process death impersonated descriptor closure);
the only property that makes a guard real is a demonstrated red.

Notes from #43 (`2601bc0`), which closed that class in the re-run path and spent
three of its four rounds on the guards rather than the code: **a refusal path is a
guard and owes the same proof of failure.** The arm that was supposed to answer
"cannot classify → no re-run form" was written as a `grep … | wc -l` assignment,
so under `pipefail` a no-match exited 1 in statement position and the function
*aborted* instead of refusing — turning an intended cosmetic degradation into a
failed launch. Its pin was green because `assert_eq "$(fn …)"` masks the abort,
which is the shape AGENTS.md already names as the only one that cannot see it.
Three companion rules earned the same way: **derive an enumeration, never recall
it** — the list of refusal arms missed a real one until a pin counted the
function's `return` sites and asserted the number, so the next arm cannot be added
silently; **feed every detector a hostile line before trusting it** — an exclusion
filter for definition lines was walked past by a trailing `# comment () {`, and a
`[^|&]*` exemption treated `pred | cat` as guarded although a lone pipeline aborts
under `pipefail` exactly like a bare call; and **a "count more things" fix needs a
false-positive control**, or it can pass by counting everything. State a textual
guard's honest limit in place (this one cannot see `eval`, wrappers, or indirect
invocation) rather than implying a completeness it does not have.

Notes from the #48 archive primitive (`f58f860`) — five review rounds, ten
blockers, every one on the **destructive** path and every one requiring a hostile
construction no passing suite produces. Four rules, each earned from a reproduced
defect:

- **A delete proves as much as a write.** Publication was given three proofs — a
  real archive root, an atomic claim, tree validation — and purge was given none.
  With the archive root symlinked, purge deleted a directory *outside* the root;
  with a publisher's claim standing, it deleted a target that publisher was about
  to recreate. A tree ae cannot validate is a tree ae cannot claim to own. Whenever
  a guard is written for the creative path, ask what the destructive one got.
- **A fact observed twice is two facts.** The confirmation prompt rendered one
  plan and the freeze captured a *second* observation of it, so flipping a
  session's config between the prompt and `y` let a human confirm KEEP and receive
  PURGE. Resolve once into fields; render, freeze and act on those same fields. If
  a value is worth confirming to a human, it is worth observing exactly once — and
  nothing in a freeze path may fork, because a subshell cannot return state.
- **Consolidating writers concentrates risk.** One definition for all callers is
  right, and that is precisely why the shared thing must be *more* defensive than
  the private ones it replaced: a fail-open meta writer reached every meta path at
  once, renaming a truncated file with a key silently dropped. A function whose
  callers invoke it under `!` or `if` cannot lean on errexit for any part of its
  job — the masking hazard documented for queries is far worse in a writer.
- **An empty set is not unset.** A confirmed-target list's *length* was used as
  the proxy for "a prompt ran", so an interactive `end all` with zero sessions
  confirmed an empty list, the count stayed zero, and the code re-enumerated —
  ending a session that appeared during the prompt and was never shown. Whether
  something happened is its own fact, never inferable from a count.

The same slice extended the specimen rule to **scaffolding**. Five fixtures and
probes of the author's own reported clean on broken code: a stub archive that
proved only that purge deletes whatever sits at the path; a leaked publication
claim that made unrelated assertions fail; a pin asserting the presence of the
very shape its label forbade; a probe capturing the subject through a **pipe**,
forking the write it existed to observe; and a stub whose behaviour depended on
invocation count. A probe verifying fork-sensitive state must not itself fork —
pipes, command substitution and process substitution all do. And the division of
labour worth remembering: the product defects were found by running the product;
the test defects were found by *someone else* running the tests.

One structural note on cadence: rounds three and four each fixed defects created
by the previous round's fix — the confirmation gap, then the freeze mechanics,
then the writer the freeze needed. That is not an argument against the fixes but
for the review cadence, and a reason to expect at least one more round after any
correction that introduces a new mechanism on a destructive path.

## The instrument taxonomy — how a probe lies to you

*Added after the P0 migration cluster, where the ratio of INSTRUMENT failures to PRODUCT
failures was not close. Five product defects were filed; the harnesses, probes and guards
built to find them failed well over twenty times. That is not alarming — the incumbent is
mature code whose author already paid for its bug classes, while a harness is new code
written fast — but it means **most of the gate's work is proving the measurement, not
reading it.** Budget accordingly.*

**The central fact: probe failures are not uniformly detectable, and the undetectable ones
are the common ones.**

> A probe that returns something plausible is more dangerous than one that returns nothing,
> because it does not announce itself.

Of one lead's seven probe failures in a single session, six returned a plausible number —
a checksum count of 134 (true: 140), a roster of 11 ids (true: 9), a field value read from
the wrong file, a batch of 19 rows (true: 20), a process count, a correlation drawn from
four rows that half of them refuted. Each looked like an answer and none announced itself.
The seventh emitted 7.5MB of a file and was caught in two seconds.

**Prefer probes whose failure mode is EMPTY over probes whose failure mode is PLAUSIBLE.**
Walking forward from a known anchor to the thing you want fails loudly when the anchor is
missing; grepping a fixed window around a line number returns whatever happens to be there.
That is a difference in instrument design, not in care.

### The recurring shapes, each observed more than once

- **A filter that matches nothing looks exactly like a subject that produced nothing.**
  A `grep '[OK]'` against output that prints `OK` unbracketed; a `sed` BRE alternation
  `\(a\|b\)` that is a GNU extension and matches nothing on BSD; a prefix-anchored glob
  that misses `*-SHA256SUMS.txt`. Make the helper fail loudly on zero matches.
- **A guard that runs in the same breath as the action cannot gate it.** A duplicate-id
  pre-check placed in the same shell invocation as the edit it guarded printed its warning
  and the edit ran anyway.
- **An enumeration will be beaten.** A field-name list, a method-name list, an
  outer-attribute prefix, a lint-group name, a leak-vocabulary list — five in one cluster,
  each fix correct, each closing less than it appeared to. **The way out is a mechanism
  that resolves meaning, not a longer list**: ask the compiler (`--force-warn`), drop the
  whole column rather than filtering words. And note which diagnoses: a lexical filter
  scrubs the symptom, a structural one fails and tells you *where* the problem actually is.
- **A recursive search that strips filenames returns a value with no provenance.**
  `grep -rh` over a directory produced two confident findings sourced from the wrong file
  and the wrong arm. The flag that cleaned the output suppressed the field that would have
  caught it.
- **A census whose own command line contains its search string counts itself.** The
  `[t]ool` bracket trick stops `grep` matching itself, not the other processes in the same
  pipeline carrying the literal.
- **An instrument that depends on what the fixture breaks.** A topology builder that read
  the fixture's own meta to learn its roster — for fixtures whose whole purpose was making
  that meta absent or unreadable. It built nothing, silently, while the case file still
  said it had.
- **An artifact header is a claim.** `"ancestor walk … taken WHILE IT WAS ALIVE"` above an
  empty walk; `supervisor_observed=yes` beside a post-hoc snapshot. Both written from what
  the harness INTENDED to capture. **Derive headers from the content, never from the plan:
  if a header cannot be computed from what was captured, it does not get written.**
- **A claim classified one level up from where you were being careful.** A worker
  disciplined about never classifying an OUTCOME stated a correlation ACROSS outcomes and
  called it an observation. The framing said observation; the content said explanation.
- **Rebuilding the thing a claim is about is exactly when the claim stops being true.**
  Regenerating a harness snapshot overwrote the preserved pre-fix copy that a correction
  notice asserted still existed.
- **Verifying the tool RAN is not verifying the edit LANDED.** A patch script threw on a
  bad anchor at its last edit and wrote nothing; the commit went out with an accurate
  description of *intent* and the wrong *content*. The operator had checked the tool's
  report, not the tool's effect. Diff what you committed against what you meant, or make
  the tool's exit status depend on the edit applying.
- **Fake precision is worse than an obvious error.** Line offsets read inside a
  window-relative grep were written down as ABSOLUTE line numbers and then described as
  exact frozen verification. The output was not merely wrong — it was wrong while wearing
  the costume of a checked result. Structural fix: resolve every citation against the
  frozen source and emit **the cited line's own text** beside it. A line number is a claim;
  the line is the evidence.
- **A one-shot process check cannot see a polling watcher**, and a before/after pair cannot
  see a child that lived only between the samples. Census the long-lived ROOTS instead —
  reach is inherited across fork, so a child cannot reach what its parent cannot.

### The vacuity regress — every layer can be blind, independently

The hardest lesson of the cluster, because each fix looks like the end of the problem.

A guard proves a property. A **red-proof** proves the guard can fail. But the red-proof is
itself code, and it can be vacuous in its own right — and **fixing layer N does not validate
layer N+1.** Three instances, each found after the previous had been fixed:

1. A capability guard that enumerated forbidden spellings — beaten four times in sequence,
   each fix correct and narrower than it looked.
2. A red-proof that computed `ok` from its red arms, PRINTED it, and exited on failures
   only — so a blind arm returned `0` and the whole red-proof reported success.
3. Six red arms comparing a 2-tuple against a 3-tuple: `eq = pairs(v) == census_pairs(c)`,
   where one returns `(set, duplicates)` and the other `(set, duplicates, scope_errors)`.
   A 2-tuple never equals a 3-tuple, so `not eq` was always true, so **`caught` was
   unconditionally `YES` for every arm** — six red-proofs that could not fail, reporting
   perfect coverage.

The third is the purest form: a check whose entire job is proving something CAN fail, which
itself could not.

**Why the third one survived eleven rounds: overlapping checks mask a dead check.**
Every list mutation also broke the diff-clean leg, so *no arm could distinguish "equality
works" from "equality is dead."* Redundancy that looks like defence in depth is also
camouflage: when two legs fire on the same input, a leg that has stopped working is
invisible. **Calibrate each leg against a mutation only IT catches**, or the redundancy is
hiding rather than helping.

**And the asymmetry that let all of it happen** — stated by the worker whose tools they
were, and it is the structural lesson of the cluster:

> The evidence layer had been gated hard for eleven rounds. The gates themselves were not
> gated until a reviewer started reading them. **A reviewer who only reads the artifact
> never sees it.**

Every one of the three regressions lived in the CHECKING layer, not the evidence, and all
three had the same signature: **green output that could not have been anything else.** Read
the gate, not only what the gate passed. Budget seat time for it explicitly — it is not
free, and nothing in the artifact will prompt you to spend it.

**The regress does not terminate by adding another layer.** It terminates with one cheap,
concrete assertion:

> **Every red arm must be shown to report NO at least once.**

Invert one injection — run the arm against an UNMODIFIED input — and require `caught=NO`.
A red-proof set in which no arm has ever reported `NO` is indistinguishable from one that
is structurally incapable of it. That single control is what separates "these six arms all
pass" from "these six arms are wired to a constant."

The same shape appears wherever a checker writes what it reads: a `--check` mode that
**regenerates its output before comparing against it** compares the file to itself. An empty
stale artifact returned `rc 0` and was silently overwritten by the very run meant to detect
it. **A verifier must not be able to repair the thing it is verifying.**

And when an injection harness reports a pass, confirm the injection actually LANDED: one
column-drop arm anchored on a pre-change row, corrupted the original, and passed anyway
because the generator's `rc 2` was never inspected. A red arm that passes because the
generator crashed is a red arm that tested nothing.

### A tool that makes a check CHEAP is not a tool that PERFORMS it

The most easily forgotten limit, recorded because it is invisible once a tool reports
green. A citation-pinning tool resolves every `ae:NNNN` against the frozen source and
prints the cited line's own text. What it proves is that the citation **resolves to a real
line**. What it does **not** prove is that the line is the RIGHT one for the claim beside
it — a citation pointing confidently at a wrong-but-plausible line passes clean.

The demonstration is exact: that tool would NOT have caught the `+8`-offset steward
citations that motivated building it. `ae:16748` is a real line with real text. What caught
them was a seat reading the source.

**The tool's value is that it makes that reading CHEAP — the line's text sits beside the
claim — not that it performs it.** Conflating the two is how a green check becomes a
substitute for judgment. State this limit wherever the tool's output is cited, because the
first reader who has not built it will assume the stronger thing.

The same distinction applies to a count: **a count is a fact about an invocation, and a
count quoted in prose has lost its invocation.** Two honest runs of the same tool over
different document sets returned 234 and 257; neither was wrong and only one matched the
claim. Generate the number with its inputs recorded; never retype it into a sentence.

**A count is also a fact about a PREDICATE — and when it characterises a defect, the
predicate must be the defect's own.** Two seats counted the same anchor string in the same
file and got 10 and 12: one matched `^- no args$` exactly, the other tested substring
membership. Both correct. But the defective code under discussion used **substring
membership**, so 12 was the relevant baseline and the exact-match count *understated the
defect's reach by two strings that could satisfy the broken check.* Measuring a bug with a
predicate other than the bug's own does not merely produce a different number — it
misdescribes how much surface the bug has.

### A remedy that depends on someone staying uninformed is not a remedy

A lead leaked one row's disposition to an executor, then routed that row's
symmetry certification to the OTHER seat — **because that seat was uninformed.** Within the
hour the other seat, doing its job, sent the executor a ruling containing dispositions for
six rows. The remedy evaporated, and there was no uncontaminated party left anywhere in the
session.

**In a system whose participants exchange rulings by design, "an uninformed party" is a
temporary state, not a resource.** Any control that makes ignorance load-bearing is
consumed the first time the people involved do their jobs.

**Replace the uninformed judge with a recorded result.** The fix that works under universal
contamination is a **reachability proof**: two pre-registered synthetic fixtures, committed
before the real capture, one built to drive the arm to each candidate outcome. The arm must
report BOTH; if it can only ever report one, it is ARM-INVALID and no real capture is taken
through it; the runner refuses until both outcomes have been observed.

Why that survives when judgment does not: **a contaminated reviewer cannot be trusted to
JUDGE whether an arm is symmetric, but can still VERIFY that two committed fixtures produced
two different outcomes.** That is a fact rather than a judgment, and facts do not care who
reads them. It is the vacuity control one level up — every red arm must report NO at least
once; every contaminated arm must reach both outcomes at least once.

**The gap it does not close, which must be closed separately:** a reachability proof shows
the arm's PREDICATE has both directions. It does not show the real FIXTURE is unbiased — a
contaminated builder could pass both synthetics and still construct a real fixture that only
lands one way. So the real fixture is pre-registered too, hash-pinned and committed
alongside the synthetics, and inspected before the run. **Inspecting a committed artifact is
not judgment and is not compromised by contamination.**

Corollary on channels: normative dispositions, buckets, conflict fields and issue numbers go
seat-to-seat only; workers receive neutral surface lines. A channel with no rule will leak
through people behaving correctly.

### Bounding your own result

A guard that becomes able to fail and immediately finds something proves exactly one thing:
**it was not guarding anything before.** It does not license any claim about how much the
other guards are missing. The worker who built such a guard, having just watched it catch a
real duplicate the moment it could fail, wrote the narrow statement and refused the broad
one — while the lead reviewing it had already inflated it into "the strongest argument for
the discipline." **Overclaiming is the reviewer's failure mode as much as the builder's**,
and it arrives dressed as enthusiasm rather than as error.

### What to demand instead

1. **The arm must be able to produce the unwanted answer.** All-zero counts cannot
   distinguish "preserved" from "lost then defaulted to zero"; seven cases that all agree
   is a question about the fixture, not a finding. `ARM-INVALID` with the reason beats a
   clean result — and an arm proven unable to discriminate *forecloses* the gap rather
   than leaving it open.
2. **A zero is a measurement only if THE RECORDER THAT REPORTS IT was demonstrated live in
   the same arm** — not merely *a* recorder. One canary proved an `ssh` recorder and
   inherited an unproven `rsync` one; the resulting zero meant nothing.
3. **"All readings agree" is a question about the fixture, and the cheapest answer is
   usually elsewhere in your own evidence.** One worker caught a bad fixture because a flat
   result contradicted a corpus they had already captured.
4. **A 0-byte diff means what its CONTENT means.** Over a pid and socket existence it
   proves liveness; over a tree manifest it proves residue and is silent about the ordering
   the row claims. A create-then-perfectly-roll-back mutant produces the same zero diff.
5. **The product's own prose is not evidence for the product's behaviour.** `verified gone`
   and `nothing was stopped, state preserved` are the product asserting properties, exactly
   as strong as any other string it prints. The anti-oracle rule applies to messages, not
   only to exit codes.
6. **A citation is not a reading.** Line numbers pointing at a function's opening brace are
   not the exit code inside it.
7. **A check you learn to ignore is worse than no check** — a permanently-red guard trains
   people to skip it and then looks like coverage. Fix the check or delete it.
8. **Generate, then paste; derive, never hand-copy.** And the clause that completes it:
   **MEASURE, READ THE OUTPUT, THEN ASSERT — never measure and assert in the same action.**
   A worker ran a readiness check and sent the claim in the same breath, so the claim could
   not have been derived from the output; it was written before the output existed. The
   message contained real, correct hashes sitting beside a sentence one of them
   contradicted. **A generated number pasted next to an unchecked claim is worse than no
   number, because the number lends the claim credibility it has not earned.** Measuring
   and asserting simultaneously defeats the generate-then-paste rule while appearing to
   follow it.
9. **Verifying a conclusion independently does not validate the reasoning that reached it.**
   In the same incident a seat re-derived the conclusion from scratch, found it correct,
   and reported the claim verified — because the underlying FACT was right. The defect was
   in the claimant's evidence chain (they had searched the wrong tree), which no
   independent check of the conclusion can surface. **Only the claimant can catch a wrong
   path to a right answer**, which is why self-correction still matters after review
   passes, and why a review that agrees is not a review that checked the same thing. A table you generate and a reviewer
   reads is still your word. A table the reviewer can regenerate is not. The same applies
   one layer down to any document derived from another.

## Verification mechanics

- **Rerun the gate legs yourself, on committed main, with unmasked exit codes.**
  `cmd; echo EXIT=$?` — never `cmd | tail` (the pipe's exit status is `tail`'s;
  a red suite reads as green — this happened, phase-3 review, and was caught only
  by re-running). Background long legs; hold the verdict until they land.
- **Artifact-map completeness claims.** "20/20 done" is verified by naming, for each
  Done-criterion, a file/test on main that satisfies it — not by trusting the count.
- **Mutation-proof every guard.** A guard or coverage check that cannot fail is
  decoration. Delete the thing it protects; the gate must go red — against *every
  branch* of the guarded artifact: one behavioural guard stayed green under
  shim-deletion because its fixture only reached the branch that never called the
  shim (today's partition vs yesterday's).
- **A gate never observed to fail is not a gate — and gate tooling is code too.**
  One audit script shipped four independent fail-open defects (a stale line-window
  anchor, classification from an export list without checking the artifact sources
  it, a comment matched as evidence, an extraction regex that read every function
  body as empty) — each one reported CLEAN, and together they would have passed the
  exact bug the script was adopted to catch. Standing procedure: break the guarded
  thing and require red *immediately before* trusting any green; if the self-test's
  mutation pattern no longer matches the code, the gate reports INCONCLUSIVE and
  fails — it never silently skips.
- **A flake-tainted gate run doesn't count.** Rerun the failed leg alone on a quiet
  machine; "it was probably contention" is a hypothesis, and hypotheses get tested.
- **Source anchors decay.** `file:NNNN` citations in fixtures/plans are shape-checked
  by machine, semantically checked by nobody. Re-verify anchors by hand after any
  edit to the anchored file.
- **A green suite is not evidence a probe would pass.** Four times in one campaign a
  test was green while testing the wrong thing: a fixture built for a superseded
  contract (`c661b48`), fixtures whose bytes were never real (an encoder had turned
  spaces into NBSP), source-text asserts that pinned an error string while the
  function still returned success, and a `set -e` abort masked by every call shape
  the suite used. When a test asserts *about* the code (source text, a fixture's own
  content) rather than *driving* it, name that out loud — it pins spelling, not
  behavior.

## Verdict discipline

- Invariant hole or default-mode violation → **fail or conditional-pass** with a
  precise finding: the mechanism, the realistic trigger, the fix shape (prefer one
  that reuses an existing mechanism), and the regression test that must exist.
  Precision is what makes a conditional gate fast — vague findings cost a round-trip.
- A fix can open a new hole in the same class-family (B5's fix created B6; B6's fix
  created B7). **Re-read every fold at full depth**; "it's just the fix" is how
  shadows ship. When a builder flags that a fold grew, that flag is always worth
  the re-read.
- Style, naming, comments → notes, never gate blockers. Keep the gate's authority
  spent on invariants.

## Running this without a frontier lead

When the lead model is not the strongest available, trade depth for structure:

1. Run the protocol and taxonomy **mechanically, in writing** — a filled checklist,
   not an impression. The taxonomy encodes the deep reads already made.
2. On invariant-critical slices, give the cross-model reviewer **two rounds with
   different stances**: round one find-bugs, round two *refute the claim that the
   invariant holds* (skeptic stance). Independent stances catch what one pass ratifies.
3. Keep slices small enough that the invariant fits in one sentence. If it doesn't,
   split the slice — that is a design smell, not a review burden.
4. Mutation-proof and default-mode zero-diff checks are **not optional** at any
   model tier; they are the floor that holds when judgment thins.
