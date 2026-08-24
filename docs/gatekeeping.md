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

## Which entry do I need?

| SYMPTOM | ENTRY |
|---|---|
| a grep, census, or probe returned a plausible number that later proved wrong | The recurring shapes, each observed more than once |
| the red-proof or checker reports green and you cannot make any arm report NO | The vacuity regress — every layer can be blind, independently |
| someone said gate PASS and you cannot run the same command on the committed tree | A green gate a reader cannot reproduce is a claim, not evidence |
| you are briefing a worker and need to keep product conclusions off that channel | The seat boundary, in its finished form |
| you and another person both confirmed a document contains X without re-reading the stated clause | Two people agreeing from memory about a document is not a reading of the document |
| the commit that introduced a rule also violates it in the same text | Why rules get broken inside their own statement |
| you filtered a tool's output to the fields you expected to change | Your probe's SCOPE decides the finding, and you chose that scope from expectation |
| your criteria all pass but the obligation was about ordering, causality, or a matched pair | A check can only see obligations SHAPED like it — temporal ones survive a static gate |
| a case with more information available is about to be handled by a different branch | Adding evidence must never SUBTRACT knowledge — check the richer case against the poorer one |
| a rule says two facts are independent and you are choosing test cases | Orthogonality is proven on the OFF-DIAGONAL; the diagonal is where a derivation hides |
| your control patch produced no failures and you are concluding the guard is weak | An instrument must be present when the fixture is BUILT, not only when it is READ |
| you are checking whether a test is thorough enough, and have not asked the other question | A test that is too STRONG is a defect, and nobody is looking for it |
| you are accepting a delivery and have checked that it builds and behaves | Reviewing OUTCOMES is not reviewing INSTRUMENTS, and the outcomes are what you are shown |
| a test count moved and you are about to report the total | The DELTA carries information the total does not — a count that drops is a contradiction |
| you are accepting a report that says something was added, removed, or replaced | Verify ADDITIONS by presence and REMOVALS by absence — they are different checks |
| a test constructs the marker, fixture, or list that its own conclusion rests on | If the TEST authors the instrument, it observes what the test believes |
| you are about to trust a number, or you just found one that disagrees with another | The wrong-set sequence: grain, predicate, proxy, gate, spelling |
| you are escalating a question and have written down what you think the answer is | An escalation that carries its expected answer gets the frame confirmed, not examined |
| you are about to report a count, or you have just been given one | A count can be honestly MEASURED over a population you narrowed without saying so |
| your independent count disagrees with the figure you were asked to verify | Check that the field you COUNT BY has the same granularity as the thing you are counting |
| something was derived from a spec, and the spec has changed since | A DERIVED artifact goes stale the moment its source moves, and nothing re-runs to say so |
| a test arm's input never actually differs between iterations | An obligation can be discharged by the TYPE, and then its test is a restatement |
| you scoped someone's search, and they came back reporting that something is missing | A verification confirms WITHIN its scope and cannot see its scope |
| a document explains that your concern is covered by some other rule, and you are satisfied | A DELEGATION is a claim about scope, and being talked out of a concern is not resolving it |
| your fix satisfied a structural criterion and you have not asked what information it discarded | A structural criterion measures a PROXY, and satisfying it can defeat the goal |
| a guard says NO FILESYSTEM ACCESS or NO SPAWNING and its body lists identifiers | A guard that enumerates NAMES enforces those names, whatever its title claims |
| a rule says EVERY / ALWAYS and one criterion demonstrates it on the case it happened to build | A UNIVERSAL obligation checked on ONE fixture is checked nowhere |
| you found two rules that contradict and routed the question upward | Settling an ambiguity is not the same as finding what happened while it was ambiguous |
| your fixture plants a precondition while other parts of the same fixture break its source | A fixture can succeed at building a state the product could never produce |
| a rule lists cases and you just found one it does not cover | Adding the missing member to an enumerated rule PRESERVES the defect that produced it |
| a rule was just clarified, refined, or split into two orthogonal facts | A clarification that SPLITS a concept creates a coverage gap in evidence captured before it |
| you verified every flagged item and found nothing wrong | Forward verification cannot see a FALSE NEGATIVE — you have to check the converse |
| you are claiming a body of evidence covers some condition | A condition that was never CAPTURED cannot later be shown to have been ABSENT |
| a coverage count came back healthy and nobody has re-derived it | A measurement error that produces COMFORT gets used; one that produces alarm gets checked |
| you are about to write acceptance criteria now that the diff has arrived | If a judgement must be INDEPENDENT of a thing, it has to be made BEFORE that thing exists |
| your independent cross-check disagrees and you are about to adjust the cross-check | Fixing an instrument and TUNING it produce identical diffs |
| you added a variant to a closed set, fixed everything the compiler flagged, and it went green | A type-system guarantee has a precise scope; the confidence it creates does not |
| a number or pointer kept in two places has silently disagreed | Hand-maintained redundancy, of which stale pointers are one instance |
| your self-check grep reported CLEAN after you retyped the terms it was meant to enforce | A transcribed checklist can only under-report |
| a prose "as arm N" reference now points at a different row after a split or rebuild | A back-reference by position is a pointer into a list that changes |
| every test case reports the same result because an earlier guard refused before the subject ran | Ask what gates precede the fact under test |
| you opened a blocked test and it still cannot tell MATCH from MISS, or a case you did not name changed after the fix | Reachable is not discriminating, and an amendment's blast radius is not what it names |
| every citation now has a line number and one of them still cites the wrong function | Closing layer 1 makes layer 2 HARDER to see |
| people are following the no-leak rule and the leak is still happening | A rule that is being followed and still failing is the wrong instrument |
| the census cannot classify some processes and you were about to write a caveat sentence | Put a blind spot's SIZE in the data, not in prose |
| the new types only have the two states the original printed, and those two already mixed meanings | A rewrite inherits the original's CONFLATIONS through its type definitions, before any logic exists |
| running the checker changed a file that was already committed | A gate that GENERATES its input validates its own output, not the commit |
| you just spent many rounds hardening a protocol and the next task is a source-reading problem | A protocol that was just hard-won is the one most likely to be OVER-APPLIED |
| you are capturing evidence against frozen source for a later phase while the current build is blocked | When the source is FROZEN, evidence has no expiry — so capture order must follow BUILD order |
| a design document asserts a product outcome that an evidence arm is supposed to establish | Value-blindness is a DESIGN forcing function, not only anti-contamination hygiene |
| the citation pin resolves to a real line that is a different subsystem than the claim | A citation that RESOLVES is the most dangerous kind of wrong |
| you pointed a hooked binary at a fixture that was built by the unhooked one | An instrument must be present when the fixture is BUILT, not only when it is READ |
| the instrumented copy must differ from the frozen one and you were about to assert byte-identity | When the instrument necessarily perturbs, PROVE the difference set — do not assert it |
| every finding is treated as a missing check and the tool keeps growing with no ceiling | An overstated claim is the defect — and it has TWO repairs |
| you tightened a check to stop a false positive and a real violation then got through | A narrowing needs its own red-proof for what it now lets through |
| a pinning or count tool reported green and you treated that as having read the source | A tool that makes a check CHEAP is not a tool that PERFORMS it |
| the control only works while one party has not seen a ruling, and they just did their job | A remedy that depends on someone staying uninformed is not a remedy |
| a guard that just became able to fail caught one thing and you are about to claim that proves how much the rest miss | Bounding your own result |
| you are writing or reviewing an evidence arm and need the demand list rather than another failure story | What to demand instead |

This table is HAND-MAINTAINED and may lag the entries below it.

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

## The second protocol — reviewing a CLAIM, a CRITERION, or a body of EVIDENCE

The protocol above reads a diff against an invariant. This one is for the other half of the
job: gating captured evidence, reviewing acceptance criteria, and judging whether a green
result means anything. Same rule — run it mechanically, because every step below exists
because someone competent skipped it.

1. **Fix the subject before reading it.** Get an immutable identity — a blob hash, a commit —
   and verify it yourself. Reviewing bytes that can move is a conversation, not a gate. A pin
   also makes the *author's own summary* falsifiable: describe an artifact and cite its hash,
   and the recipient can check your prose in one command.
2. **Count the evidence base before measuring inside it.** Ask *what would have to be true for
   this to be all of it?* A verification confirms within its scope and cannot see its scope, so
   rigour inside a wrong boundary produces confident error. Absence findings are the most
   scope-sensitive of all.
3. **For every universal claim — every, always, even when empty — find where the artifacts are
   ENUMERATED** and check the obligation is asserted over the set that enumerator produces.
   Grepping the claim's own noun is a fast way in, but **a lone occurrence beside a different
   generator is a LEAD, not the finding**: aliases, generated field names and typed joins can
   satisfy a universal without ever repeating the noun. Locate the actual enumerator and read
   it. A count is where you start looking, never what you conclude.
4. **Before constructing any cell or arm, trace provenance rather than preconditions.** By what
   path would the *product* reach this state? A fixture can succeed at building something the
   product cannot emit, and green then looks exactly like proof. Do this **first**: a sequence
   that builds the matrix and checks reachability afterwards manufactures the defect before
   looking for it.
5. **For every claimed-independent pair, build PRODUCT-VALID off-diagonals**, and prefer a flip
   test — vary one axis, require the other unchanged — over four sampled cells. The diagonal is
   where a derivation hides.
6. **For every differential, name the axes it MOVES.** Anything constant in all arms is
   invisible, and an absent input is the most constant thing there is. Then ask the question
   that decides which arm carries the proof: **is a failing re-observation distinguishable from
   no re-observation at all?** Where it is not, a *readable opposed value* carries the proof and
   a removal arm cannot. Where deletion or failure is itself the required observable —
   cleanup, revocation, incompleteness, loud-error paths — removal is the strongest arm you
   have. There is no general ranking; there is only that question.
7. **For every control, prove it applied — then hold it to the standard for its KIND.** A
   *sensitivity* control must change the detector's verdict; a *neutral* or
   inactive-equivalence control must leave the subject unchanged, and demanding a change from
   it would reject the correct control. Every neutral leg needs a separately calibrated red leg
   proving the instrument can move at all. Applied-but-inert, never-applied, and
   truncated-by-fail-fast all read identically to "your tests are weak."
8. **For every "covered elsewhere", go read the elsewhere and ask what it was written to
   catch.** A delegation is a claim about scope; obligations written before a change do not
   extend to what the change introduced.
9. **Ask which CORRECT implementations this would reject.** Over-strong checks are defects
   nobody hunts, and the answer is only checkable if the open choices are enumerated.
10. **Read the whole output once before filtering it.** Your probe's scope decides the finding,
    and expectation chose that scope. Compare invocations before comparing results.
11. **Audit the REASSURING numbers.** An error that flatters coverage is systematically less
    likely to be caught. When a measurement reports coverage, open one covered case and confirm
    it exhibits the condition — but know what that buys: it calibrates that the counter can tell
    a match from a miss, and **it does not validate the count**. If the number is load-bearing,
    independently re-derive the inventory.
12. **Ask what the repair DESTROYED, not only what it removed.** Consolidation and
    normalisation often narrow a *representation*, and a narrowed representation is where a
    distinction goes to die — `.ok()` on a fallible read being the canonical case. Then ask
    whether the structural criterion in play **is the property or a proxy for it**: a type that
    cannot express the forbidden state is direct enforcement and the strongest thing here; a
    count of call sites is a proxy, and satisfying a proxy can relocate the violation somewhere
    it does not look.

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

**A red-proof's coverage must match the tool's, and reporting otherwise is its own defect.**
A seat red-proofed a newly committed checker with **one** seeded mutation on **one** of its
six check paths, watched it go red, and reported the instrument verified in both
directions — both directions of one check. The other seat then seeded seven mutations
across every path: *forbidden vocabulary, missing candidate field, uppercase ordinal,
duplicate id, unknown class, unused typed barrier, deleted roster.* **All seven returned
`rc=0 OK`.** The tool caught exactly the one spelling the first seat happened to try.

Worse, the vocabulary check was dead on the supported host: the pattern used GNU `\<…\>`
word boundaries, which **macOS awk does not implement** — it silently matches nothing
(probe: boundary `0`, plain `1`). The checker printed the forbidden term in its own derived
term list and reported clean while two live instances sat in the document. **The term-list
defect one layer later: the derivation was fixed, and the predicate never matched.**

The rule: **red-proof every named check, on the host that will run it, with a
neutral/mutated pair each.** One green mutation proves one path and licenses no statement
about the tool.

**And the deeper limit, stated by the author of the tool it indicts:**

> My self-test seeded the exact literal my regex matched, and a count in one of the five
> phrasings my pattern listed. **The red-proof proved the predicate matches what the
> predicate matches.** A red-proof written by the same hand as the predicate inherits its
> blind spot — and mine did so completely, while producing fourteen green lines that looked
> like coverage.

**A self-test cannot establish coverage.** It proves the predicate fires on inputs its
author imagined, and its author's imagination is the same one that bounded the predicate.
This is why a reviewer re-running someone's self-test is **not** performing an adversarial
check — they are re-executing the author's assumptions with the author's fixtures, and will
get the author's answer. A seat did exactly that here, twice, and cleared a checker that was
blind to four classes.

**Only a mutation the author did not anticipate tests anything.** The demonstration, from
the round that followed: the rebuilt tool forbade *numerals* beside derived countables and
its 26-mutation suite passed clean — then an outside probe wrote the count **in words**
(*"There are forty-four executed units"*) and the checker returned `rc=0 OK`. A hand-typed
word-count drifts exactly as a numeral does. The escape from filters to contracts had kept
one representation as its subject, which is the enumeration treadmill wearing a contract's
clothes. **A contract over one representation of a thing is still a filter;** the contract
has to be over the *thing* — counts may appear only as generated markers, so any prose count
is invalid whatever its spelling.

**And note where that defect landed.** This repo's own hazard list documents the GNU-vs-BSD
divergence class at length — `tac`, `stat -c`, `date -d`, `sed -i`, `grep -oP` — with the
warning that every row *"shipped as a macOS bug"* and that they **fail silently**: the
command does not error, a fallback lands, and the feature reads as *nothing found* rather
than *broken*. The dead word-boundary is a **sixth row of that table, inside the instrument
built to police defects of that shape**, and it was found by a reviewer rather than its
author.

That is the cognitive-acts mechanism applied to a *documented hazard* rather than a
self-authored rule: the list is a **category** — GNU-only constructs fail silently on BSD —
and `\<…\>` arriving inside a regex is an **instance that does not announce its
membership**. It explains why writing hazards down does not prevent them, and why the
repair was the right one: the tool was **rewritten in a language not exposed to the class**,
rather than having the one boundary patched. Patching the instance leaves the family.

**Point the same rule at the INSTRUMENT, not only the product.** A seed that does not land
is indistinguishable from a check that does not fire, and it errs in **both** directions
depending on how the harness is coded. A seat red-proving someone else's checker seeded a
pattern matching zero lines; nothing was inserted, the tool correctly reported clean, and
the reading available at that instant was *"the checker missed it"* — **a false alarm about
a working tool.** The column-drop arm above is the same non-landing seed producing the
opposite error, a **false pass**. One discipline covers both: **seed, DIFF to confirm the
seed is present, then run.**

### A green gate a reader cannot reproduce is a claim, not evidence

The longest-running defect of the cluster, and the one that hid best: a worker ended every
handover for **eleven rounds** with *"gate PASS, tree clean."* The result was correct every
time. It was also **unverifiable by anyone but them** — the gate they ran lived in a scratch
harness and was never published. The only committed copies were per-arm snapshots, each
deliberately frozen with the captures it audited.

Freezing a tool beside its evidence is **right for provenance** — an arm's snapshot records
the version that arm actually ran under, and it must not drift. It is **insufficient for
reproducibility**: with only snapshots committed, the newest published copy is always older
than the working one, and a reader who runs it audits a previous era. The seat who tried
got 14 schema violations on a clean tree (kinds the old gate had never heard of) and
`present_at_HEAD=0` on a fully committed tree (the prefix hardcoded to the other batch).

**You need both: the frozen per-arm snapshot AND a canonical current copy**, with a README
naming which is which and why they disagree. Publishing the tool is part of publishing the
result.

The seat's handling is the reusable half. Three eliminations — copies all hash identically,
checkout is current, no gate exists under the audited tree — none of which could find the
cause, because *the version that mattered was never in the tree at all.* After three failed
probes they **asked rather than filed**: exact command, exact output, three named hypotheses,
and an explicit statement that a false blocker against freshly-gated work costs more than a
question. The answer was hypothesis (b), and both parties were right — it *was* a reader
running the wrong thing, and the reason was that the right thing had never been published.

**Corollary, general:** after N failed probes, asking the person who built the instrument is
cheaper and more reliable than probe N+1. Guessing feels like progress and is the same
motion that produced the first N.

### The seat boundary, in its finished form

Reached after two leaks and three refinements, and worth stating whole because each clause
was paid for.

**Why a rule was not enough.** Normative content crossed to a worker twice while a channel
rule forbidding it was in force, both times from a seat behaving correctly — once inside a
message *arguing for the exclusion*, once inside a legitimate scope correction. A rule
being followed and still failing is the wrong instrument.

**The boundary is a FILTER, not a rule.** Outcome-bearing gate analysis routes seat-to-seat;
the worker receives a **neutral delta** produced by a seat. That costs a hop and puts
filtering work on the seats, which is the price of a boundary that does not depend on
everyone remembering it.

**Filter by SUBJECT MATTER, not severity and not source.** Findings about the harness, the
document, or the platform travel **verbatim** — a subshell breaking a sequence counter, an
arm deferring constructibility to its own artifact, a positional reference that retargeted.
Only findings whose *reasoning states what the product does* get converted. Applied bluntly
the rule over-applies: every gate routes through a seat, and since a delta is always lossier
than the original, filtering what needs no filtering costs precision for nothing. In one
nine-blocker gate, five travelled whole and four needed conversion.

**A source anchor may travel; the relation it proves may not.** `cut at ae:15089` locates a
site and is necessary for the worker to build anything. *"ae:15089 shows the daemon is
stopped before X"* states the relation the arm exists to test. Without this precision the
rule either blocks every citation — making arms unbuildable — or admits citations together
with the prose that explains them, which is the leak.

**The worker derives its own product facts.** Told what to change and where, a worker reads
the frozen source themselves. That is strictly better than being handed a conclusion: it
keeps the anti-oracle rule pointed at seats too, and a seat that has been right six times is
still not an authority about the product.

### Two people agreeing from memory about a document is not a reading of the document

The most dangerous verification failure found, because it is **undetectable from either
side** and it looks exactly like independent confirmation.

A seat told a worker three conditions were binding and added *"you have all three
already"* — having read the file. The worker checked instead of confirming, and found only
one and a half: an execution-order line naming spec ids where executed units were meant,
lane visibility present as incidental phrasing in one row rather than as a rule, and
nothing anywhere forbidding the cross-lane aggregate the condition existed to prevent.

The seat had checked whether the **concept** was present. The condition was about whether
the **rule** was stated. The worker knew the concept was there because they had written it.
**Both parties confirmed the same wrong thing from different directions**, and confirming
would have cost nothing and left no trace, because the seat had already reached the
conclusion independently. Neither reading was careless and **neither was wrong on its own
terms** — the seat's answer about the concept was correct, the worker's knowledge that the
concept was present was correct. **The agreement was the artefact: two correct answers to
two different questions, presented as one confirmation.**

> Two independent wrong readings look exactly like verification.

This is distinct from *verifying a conclusion does not validate the reasoning* — that is
about one party's path to an answer. This is two parties' **agreement** substituting for a
check neither performed at the right grain. The defence is the worker's: when told "you
already have X," **read for X as stated, not for X as remembered** — and treat a seat's
confirmation as a claim about the document, checkable like any other.

*Where the defect sat, measured:* not near the rule — **in the same sentence as it**, one
semicolon away:

> …derived from the typed rows by the checker, **never from a prose count**, which is how
> v5 said fifteen while its own roster said eighteen; (v) **arms 1–37 in id order**;

The clause forbidding prose counts and the prose count share a sentence, written in one
pass, by someone who had just been shown that defect. "A few lines below" would let a
reader imagine attention lapsing across a gap. **There was no gap.**

### Why rules get broken inside their own statement

Four times in one cluster a rule was violated by the commit that introduced it: a mechanism
banning positional references shipped with seven; a lint vocabulary omitted one of its own
terms; a cleanup removed the required delimiter along with the stray; an enumeration
forbade prose counts mid-sentence and then used one. Calling this irony explains nothing.
The worker who did the last one supplied the mechanism:

> **Stating a rule and applying it are different cognitive acts, and doing the first does
> not prime the second.** I was composing an enumeration and reached for the number I had.
> The rule I had just written was about a *category* — prose counts — and the thing in my
> hand did not present itself as a member of that category.

**A rule is stored as a category; a violation arrives as an instance that does not announce
its membership.**

**The same gap separates KNOWING a fact from APPLYING it.** One design derived a product
fact, cited it correctly in its own change log two drafts earlier, and then contradicted it
in an arm body. Three further contradictions in that document had the corrected text and
the uncorrected text **both by the same author, in the same section, in the same commit** —
a preamble demoting a path to out-of-scope while an arm below named it as its sink; a
preamble naming one subject directly above arms testing another. **A correction lands where
it is written and does not propagate**, and neither version looks wrong on its own: only
holding both at once shows the contradiction, which is exactly what an author revising in
place cannot do. This is why an internal-contradiction check is worth more than a careful
re-read — the check holds both simultaneously and the author cannot. That is the whole reason every one of these has needed a *mechanism*
rather than a resolution: a mechanism operates on instances — it sees the actual text and
does not need the author to recognise the category — while a resolution operates on
categories and requires exactly the recognition that fails. It also explains why *being
told twice is not a mechanism either*.

### Hand-maintained redundancy, of which stale pointers are one instance

Sixty-two positional back-references retargeted silently when a roster renumbered; the
proposed alternative to a parameterised arm was sixteen near-identical blocks differing
only in a selector. The worker's reframing is the general form:

> I had been treating that failure as being about POINTERS. It is about HAND-MAINTAINED
> REDUNDANCY, and pointers were one instance.

Any fact stated in two places that a human must keep in agreement will drift, and the drift
is silent because both copies still parse. **Where a number must be maintained by hand, the
right move is usually to stop having the number** — a heading naming a range rather than a
count, a checker deriving a set from typed rows rather than asserting its size.

### A transcribed checklist can only under-report

The sharpest instrument bias found in the cluster, because its error mode has **one
direction**.

A worker's self-check grep omitted one term from the vocabulary it was meant to enforce —
a list they had retyped from the rule rather than derived from it. It reported CLEAN for
several rounds while five violations stood.

**A shorter list always reports cleaner.** You cannot accidentally *add* a term, because a
spurious term produces false positives you notice immediately; you can only accidentally
*drop* one, and a dropped term is silent. So transcription error in a checklist is
**biased toward nothing-to-fix** — the reassuring answer, arriving through a mechanism
that looks like diligence.

The fix is structural and generalises to any enforcement vocabulary: **the checker READS
its terms from the declaration it enforces, rather than carrying its own copy.** A term
list that is transcribed can only under-report; one that is derived cannot drift from the
rule at all.

Found by re-deriving the pattern from the rule's own sentence instead of retyping it —
which is the citation-from-recall defect wearing different clothes.

### A back-reference by position is a pointer into a list that changes

Sixty-two prose back-references of the form *"as arm 16, with…"* **silently retargeted**
when splitting one arm and rebuilding two rows renumbered the roster. A reference inside
one row's arm came to point at a different row's arm entirely, and nothing anywhere flagged
it — the text still parsed, still read sensibly, and now meant something else.

**Positional references are unstable under any reordering and fail silently when they
move.** Replace them with named, keyed blocks that a checker expands into each referencing
site, so a rename breaks loudly and a renumber cannot happen at all.

Note the recursion, which is the usual shape: the arms written to *introduce* this rule
initially carried the same prose back-references the rule forbids.

### Ask what gates precede the fact under test

Three times in one cluster an arm could not exercise its own subject because an **upstream
guard fired first**, and each was found only after the capture:

| arm intent | upstream gate that foreclosed it |
|---|---|
| does the cwd fallback select? | the token path selects first, and the fallback runs only when it finds nothing |
| does refresh restart a running daemon? | a liveness gate decides whether the running-daemon branch is entered at all |
| do these selector spellings differ? | a config-presence check refuses before any selector is consulted |

In each case the cases reported **identically**, which reads as "no difference here" and is
actually "the question was never asked." The pattern is not carelessness — a fixture author
reasons forward from the manipulation to the expected observation and does not naturally
enumerate what stands between them.

**And the complement, which the guard enumeration does NOT cover: did the manipulation
actually produce the input it claims?** A worker enumerated every downstream guard before
building — banner, file-existence wait, replay cut, a formatter that returns early on a
malformed line — wrote the enumeration into the run manifest, correctly identified which
guard would have cost a run, and **still measured nothing**. Their planting helper emitted
JSON *without a trailing newline*, so a 29-event cohort was one unterminated line: `wc -l`
returned 0, the cut had nothing to cut, and the formatter rendered whatever it found first.

**Guards sit downstream of the fixture; a construction failure is upstream of all of them.**
No amount of call-path reading reaches it.

The fix generalises and is cheap: **every planted input gets an independent count of what
actually landed, recorded beside what was intended, so the two can disagree visibly** —
`planted_lines` next to `planted_events`. That completes a family of three, all asking *did
the thing you believe happened actually happen* at successive stages: verify the seed landed
(red-proof injection), verify the manipulation produced its claimed input (fixture
construction), enumerate the guards between manipulation and observation (below).

**So enumerate it deliberately, before building: what guards precede this fact, and does
the fixture pass them?** It costs one read of the call path and it is the difference between
an arm that answers its question and an arm that answers a question nobody asked.

And when it happens anyway, the disposition is the one that preserves both facts: **keep the
cases as evidence for what they DO establish** — the precedence, the gate's own behaviour —
**and build a separate pair that reaches the original question.** Rebuilding in place throws
away a true observation to test a different one.

### Reachable is not discriminating, and an amendment's blast radius is not what it names

Two rules from one rebuild, both earned twice over.

**A fix that makes a check REACHABLE is not a fix that makes it DISCRIMINATE.** A pair of
cases meant to oppose on one fact reported identically; a seat found the guard that made
the fact unreachable, and the fix opened it. The pair then reported identically **again, in
the other direction** — because the compared value was taken from the invoking process's
own `$PWD`, and the arm ran from wherever the controller happened to stand, so the MATCH
case could not match *by construction*. **The arm's own invocation environment was a hidden
input to the test.** Getting past a guard says nothing about whether the fact behind it is
consulted meaningfully; re-check the pair discriminates after making it reachable, as a
separate step.

**An amendment moves readings in cases it does not name.** After the second fix, one case
no amendment mentioned changed from `rc 1` to `rc 0` with nothing about its own fixture
altered — its reading was a joint property of its fixture *and* the shared invocation
directory, and it moved when that moved. So: **re-read every case after an amendment, not
only the amended ones.** The worker's own framing is why it worked —

> Twelve reproduced identically across the amendment, which is what made the one that did
> not stand out.

The unchanged majority is simultaneously the detector and the control. Re-reading only the
named cases would have shipped a silently moved reading with a clean amendment record
beside it.

### Closing layer 1 makes layer 2 HARDER to see

Citation failure has three layers, all three hit in one cluster, each invisible from the
one below:

| layer | failure | caught by |
|---|---|---|
| 1 | an **unpinned** citation that reads exactly like a pinned one — recall wearing the costume of transcription | the author, resolving them |
| 2 | a **pinned** citation that resolves to a real line which is the **wrong line for its claim** | a seat reading source |
| 3 | the **pin tool**, which makes the check cheap and does not perform it | the tool's own author |

The trap is that **fixing layer 1 camouflages layer 2.** A worker wrote the rule that a pin
proves resolution and not aptness, corrected eight unpinned citations to earn their pins,
and shipped a pinned-and-wrong one **in the same document** — a real function that really
does the described operation and is simply not the code path the claim is about. Their own
observation:

> Layer 1 is cheap to close and closing it does not touch layer 2 at all — if anything it
> makes layer 2 harder to see, because everything that survives now carries a verified line
> number.

Uniform verification hides the cases it does not cover. Once every citation is pinned they
all *look* equally checked, and the ones that resolve-but-mislead are indistinguishable
from the ones that are right. **Never let a pinning pass be reported as a correctness
pass** — say which layer was checked.

### A rule that is being followed and still failing is the wrong instrument

Normative content crossed to a worker **twice while a channel rule forbidding it was in
force**, both times from a seat behaving correctly: once inside a message *arguing* for the
exclusion, once inside a legitimate scope correction on a gate. Neither was carelessness.

> The rule is being followed and the channel is still leaking, which says the fix is the
> typed projection and not more care.

And the projection alone is **necessary but insufficient**, because free-form review prose
is a leak surface no schema covers. The structural fix is to make the seat boundary a
**filter rather than a rule**: outcome-bearing gate analysis routes seat-to-seat, and the
worker receives a **neutral delta** produced by a seat. That costs a hop and puts filtering
work on the seats, which is the price of a boundary that does not depend on everyone
remembering it.

### Put a blind spot's SIZE in the data, not in prose

A containment census classified processes by reading their environment. It could not report
its own deliberately in-range control, and the arm refused to run until it said why. Two
measured causes, and the second is a platform fact worth knowing:

**macOS exposes a process environment to `ps e` for only a subset of even one's own
processes — 1 of 40 sampled** (independently reproduced: 40 sampled, 1 readable). So **a
process whose environment cannot be read cannot be classified by reach at all.**

The wrong response is a caveat sentence — *"the census may not see every process"* — which
is unfalsifiable and shrinks in the reader's mind to nothing. The right one is a **third
class carried as a number in every record**: `in_range 3–4`, `out_of_range ~435`,
**`UNKNOWN-REACH 865–916`**. That last figure is the honest size of what the instrument
cannot see, and because it sits in the ledger rather than in prose, a reader can weigh it
and a checker can watch it move.

**Never let an unreadable subject be counted as a negative.** An unclassifiable process
counted as out-of-range converts ignorance into a clean result — the same shape as an
absent file standing in for a recorded absence.

*The first cause is its own lesson:* the control was `bash -c sleep 25`, which **execs**
`sleep`, and the exec'd process exposes no environment. The control failed **for its own
reasons**, and a control that fails for reasons unrelated to its subject reads exactly like
the subject failing.

### Your probe's SCOPE decides the finding, and you chose that scope from expectation

Two instances the same evening, by the same person, in the same role — which is what makes it
a class rather than an accident.

**One.** A tool's summary line reported twenty counters. The check grepped it down to the
handful expected to move. A real regression landed in one of the omitted counters and was
invisible for two commits — not hidden, *filtered*. The output had said it plainly.

**Two.** The same fact was probed twice ten seconds apart, once case-insensitively and once
not. The two answers disagreed, and the disagreement was briefly read as a finding about the
document rather than about the probes.

Both share a shape: **the instrument's scope was set by what the operator expected to matter,
and then the result was read as though the scope were the subject's.** A filtered view of a
green report looks exactly like a green report. A narrowed grep returns a smaller number that
looks exactly like a smaller number.

This bites hardest on the tools you trust most, because a trusted tool is the one you stop
reading in full. And note the asymmetry with the comfort rule above: **filtering is how a
comfortable reading gets manufactured without anyone intending it.** Nobody decided to ignore
that counter; they decided which counters were interesting, which is the same act with better
manners.

Practically: read a summary line **whole** the first time, and only then filter. When
comparing two probe results, compare the **invocations** before the outputs — if the flags
differ, there is no finding yet. And when a tool emits counters you do not recognise, that is
the moment to look them up, not the moment to drop them.

### A check can only see obligations SHAPED like it — temporal ones survive a static gate

A rule was named *"candidates exist **before** liveness is classified"* — the ordering was the
whole point, the fix for a defect born of collapsing two phases into one. Nineteen acceptance
criteria were written for it. Two were aimed at exactly this: one diffed for any status being
assigned during discovery, another forbade discovery being *gated* on a query succeeding.

Neither forbids **building the candidate set out of the liveness map**. Enumerate the servers
first, construct candidates from the live names you got back, append the durable directories
afterwards: no status is assigned, nothing is dropped, both criteria pass, and the ordering the
rule exists to guarantee is exactly inverted. **A snapshot of a result cannot show the order in
which it was built**, and every criterion observed a result.

The general failure: an obligation about **sequence, causality, or provenance** checked by an
instrument that only sees **final state**. The check is not wrong, it is the wrong *shape* — and
it reports green with total sincerity. Temporal obligations need a temporal observable: a call
trace, a construction-order assertion, a type that cannot be built out of the later phase's
output, or a seam that makes the dependency direction structural rather than conventional.

**The same mismatch has a second common form: a paired obligation with only one side checked.**
The same gate guarded hard against candidates being wrongly *collapsed* — by basename, by
prefix, by name alone — and never once asserted the collection was a **union**. An
implementation emitting one identity twice, from two sources, passed every criterion. Guarding
a failure direction intensely is what makes its opposite invisible: you are looking so hard at
over-merging that under-merging never occurs to you.

So before trusting a criteria list, ask of each obligation: **is this about a state, a sequence,
or a pair?** and does anything in the list observe that kind of thing at all.

### Adding evidence must never SUBTRACT knowledge — check the richer case against the poorer one

A rule said a candidate discovered only as a live sighting is proven live by that sighting.
A separate rule said a candidate with a durable record is proven live by querying its recorded
server. Both sensible. Nobody had written down what happens to a candidate that is **both** —
which is what an ordinary running instance looks like once the two sources are joined.

The default reading made it the *durable* case: query the server again. And that reading is
incoherent, for a reason that has nothing to do with either rule's merits. The **same accepted
sighting** keeps a live-only candidate proven; if adding a durable record meant a redundant
failed query could turn that identical candidate *unknown*, then **learning more would reduce
what is known.**

That is the check worth generalising. Wherever a system classifies by *how much* it knows about
something — provenance, source count, record richness — take a case, add a fact that is
strictly additional, and confirm the conclusion does not get weaker. A pipeline that treats
"richer" as a different branch rather than a superset will silently violate this, because the
two branches are written by different people at different times against different rules.

The corroborating detail here is worth noticing too: the join conditions and the proof
conditions turned out to be **the same conditions** — same recorded server, exact name, positive
ownership, exactly one match. That is not a coincidence, and when it happens it usually means
one of the two steps has already established what the other is about to re-derive. Look for it
before adding a second query, a second read, or a second verification pass.

### Orthogonality is proven on the OFF-DIAGONAL; the diagonal is where a derivation hides

A contract declared two facts independent — either, both, or neither may hold. The gate for it
constructed the obvious cases: both present, and both absent. It read as thorough. It proves
nothing about independence.

Consider the defect it is meant to exclude: an implementation that **computes X from Y**. On
the diagonal, the derivation and the truth **agree** — when Y holds, the derived X holds, and
the correct X happens to hold too. Both-present passes. Both-absent passes. A criteria list
built from those cells is fully satisfied by exactly the implementation the orthogonality claim
exists to forbid.

**Only the off-diagonal cells discriminate**: X without Y, and Y without X. Those are the cells
that read as odd combinations — a record we could read whose server we could not reach; a
session we can see that we cannot prove is ours — and they are precisely the ones a fixture
author skips, because they feel like contrived corners rather than the point.

So for every claimed-independent pair, **build the two off-diagonal cells first** and treat the
diagonal as the cheap confirmation it is.

**And cells alone are still the weak form, because four points can agree with independence by
coincidence.** The strong form tests the claim itself: **flip one axis and require the other to
be unchanged.** Vary the record's readability and demand the identical liveness answer; vary
the liveness proof and demand the identical degradation. That is an invariance assertion rather
than a sample, and it fails against a derivation no matter which cells someone chose to build.
Prefer it wherever the two facts can be varied independently in a fixture.

Two further habits fall out:

- **Name the derivation you are excluding, concretely**, before designing the cells. "X iff Y"
  is a hypothesis you can test; "they are orthogonal" is a hope you cannot.
- **Test both directions.** X-from-Y and Y-from-X are different implementations with different
  off-diagonals. The reverse is the one that gets forgotten — the interesting failure is often
  not *liveness derived from provenance* but *provenance quietly discarded once liveness is
  known*.

The same shape governs any claim of the form "these vary independently": ordering versus
membership, identity versus reachability, presence versus permission. If the evidence only
holds cases where both move together, the claim is untested no matter how many of them there
are.

### A test that is too STRONG is a defect, and nobody is looking for it

Every instinct in review points one way: is this check thorough enough, could something slip
past, what did we fail to assert. The opposite defect gets almost no attention — **a check that
rejects a correct implementation** — and it is not harmless. It costs rework, and worse, it
pressures whoever is building to change *correct* code until an unratified constraint is
satisfied. The contract loses, quietly, to a test nobody ratified.

Two exhibits from one project, in both directions.

An implementer wrote two tests asserting a query count of exactly zero. The governing criterion
explicitly made a redundant query an **open choice** that merely must not change the answer.
Their own tests would therefore have failed a correct classifier for making a permitted choice —
and **they found it, said so, and replaced it with the real requirement**: that the contradicting
answer does not move the status. Reporting your own tests as *too strong* is much rarer than
reporting them as too weak.

Separately, a reviewer noticed a gap in a criteria list, found a sentence delegating it
elsewhere, and accepted — the delegation's scope did not actually cover the case. Being talked
out of a concern and over-constraining an implementation are the same failure seen from
opposite ends: **in both, the document is trusted about its own coverage.**

The mechanism that makes the over-strong direction findable is a **scope guard that fails the
gate itself**: a criterion stating in terms that the gate FAILS if a test rejects an otherwise
correct implementation for a choice no rule constrains, with the open choices enumerated beside
it. That turns "do not over-reach" from an intention into something a reviewer can check, and it
gives an implementer standing to push back with a citation rather than an opinion.

Ask it explicitly, every pass: **which correct implementations would this reject?** If the
answer is "none I can think of," the enumerated open choices are what make that checkable rather
than hopeful.

### Reviewing OUTCOMES is not reviewing INSTRUMENTS, and the outcomes are what you are shown

A delivery arrived reporting fifteen of fifteen criteria met. The reviewer checked the suite was
green, checked the refusal had been removed, checked the sort order in real output, and found a
genuine defect the report had flagged. Every one of those checks was a check on an **outcome**.

The gate then failed it on three blockers, and two of them were **instruments that measure
nothing**. A test built a scratch directory, wrote opposed files into it, and never passed it to
the invocation — which received an in-memory value instead — so the "opposed external worlds"
were one value and a directory nothing reads. Another asserted an emptiness the criterion
explicitly left open, rejecting a correct implementation.

Neither is visible from an outcome. **A test that measures nothing passes, and a test that
over-constrains passes, and both look exactly like a test that works.** The only way to see
either is to read what the test *does* against what the criterion *asks* — which is a different
activity from confirming the result.

The pull is structural, not lazy. A report hands you outcomes; outcomes are checkable in
seconds; and each one you confirm feels like review happening. **The instrument review has no
prompt.** Nothing in a green run says *by the way, look at whether the fixture is connected.*

So when accepting work against criteria, budget the pass in two parts and do the second one
explicitly:

- **Outcomes:** does it build, do the numbers reconcile, does the reported behaviour appear in
  real output.
- **Instruments:** for each criterion, what does its test actually observe, and could that
  observation differ if the product were wrong? Trace one fixture end to end — from where the
  world is set up to where the assertion reads it — and confirm the two are connected at all.

The reviewer here had written a checklist containing both of those steps **two hours earlier**,
and skipped them on the next delivery. Knowing the hazard is not the mechanism; a checklist you
do not run is a document about someone else.

### The DELTA carries information the total does not — a count that drops is a contradiction

A change was described as *adding* a structural test. The suite went from **451 to 448**. Both
the author and the reviewer read "448 passed" as green and moved on.

Minus four deleted plus one added is exactly minus three. **A test count that falls after an
additive change is a contradiction visible in the report itself**, and it was the only signal
either of them had. A total answers *did everything pass*; a delta answers *did the change do
what it said*. Only the second is a claim about the work.

The cause beneath it is worth separating from the arithmetic. **A report written from what you
did describes what you did, not what happened.** The author had genuinely built the helper the
report described — earlier in the same session, in the working tree — and a later edit of their
own destroyed it. The report was written from memory of the work rather than from the diff, so
it was an accurate account of an artifact that no longer existed. It had never been committed at
all.

And the destroying edit is a mechanism worth naming: a programmatic rewrite sliced the file
**from a found index to the end**, with no terminating marker. Anything appended after that
point — including tests added earlier by the same person — was inside the replaced region. **An
unbounded slice from a found index consumes everything later than it**, and "later in the file"
usually means "added more recently", which is exactly the work least likely to be missed by a
quick reread and most likely to be missed by memory.

Three checks fall out, all cheap and all checkable by someone else:

- **Report the delta, not the total.** *451 to 448* invites the question; *448 passed* does not.
- **Explain any decrease explicitly, or do not send the report.** A drop after an additive
  change is either a deletion you did not intend or a claim you cannot support.
- **Write the report from `git diff --stat`, never from recollection of the session.** The diff
  is the artifact; your memory is of the intention.

### Verify ADDITIONS by presence and REMOVALS by absence — they are different checks

A report described a repair: a helper was added, three tests were rewritten to read a value
from the product, two structures had a field taken away. The reviewer checked it and passed it.

They had checked **only the removals.** *No path type on this structure* and *this is now a
production type* are both claims about something being **gone** or **changed in place**, and a
grep confirms them in seconds. That the new helper **existed** was never checked — and it did
not. The tests it was supposed to support had been deleted rather than replaced, so the behaviour
they guarded was unprotected entirely, and two one-line mutations to the product passed the
whole suite.

**The asymmetry is easy to miss because both feel like "checking the report."** An absence claim
is falsified by finding the thing; a presence claim is falsified by *not* finding it — and the
second requires you to know what to look for, from a report you are simultaneously trying to
verify. The reviewer confirmed everything the report said had been taken away and nothing it
said had been put in.

So when accepting work against a report, split the claims:

- **Removals and constraints** — grep for the thing that should be gone.
- **Additions** — grep for the thing that should be there, **by the name the report gave it**.
  If the report names a helper, a test, or a field, that name is a checkable assertion.
- **Replacements are both.** *X was replaced by Y* is two claims, and confirming X is gone says
  nothing about whether Y arrived.

**And when a report and the tree disagree, that is a distinct category of failure.** A weak test
is a design problem; a report describing an artifact that does not exist is a *reporting*
problem, and it invalidates the basis on which everything else in that report was accepted.
Ask what happened before asking for a rebuild — a lost edit, a stale working tree, a commit that
missed files, and a summary written from intention rather than from the diff are four different
causes with four different remedies, and only one of them is fixed by writing the code again.

### If the TEST authors the instrument, it observes what the test believes

Three separate criteria in one project failed the same way, with three different repairs and one
diagnosis:

- A test proving the renderer never reads the filesystem built a scratch directory, wrote
  opposed files into it, and **never passed it to the invocation** — which received an in-memory
  value. The opposed worlds were one value and a directory nothing read.
- A test proving discovery completes before presentation called the first presentation
  operation, **then appended a "presentation enter" marker to a test-local list.** Everything
  that operation did happened before the marker and was invisible; a sort inserted inside it
  passed.
- A guard claiming a capability boundary enumerated eleven method names, and **the file itself
  said it enumerated entry points** — while the criterion it served demanded a boundary. An
  unlisted safe-standard-library call at the same entry point passed.

In each, the observable was **constructed by the test out of what its author already believed**,
rather than **emitted by the product at the boundary being claimed**. A test-authored marker
records the test's model of the sequence. A test-held fixture records the test's model of the
world. A hand-listed set of names records the test's model of the capability. All three are
green when the model is right and green when the model is wrong.

The repair that worked here is worth naming because it changed the CLAIM rather than
strengthening the guard: presentation was given **no address** — no root, no record path,
nothing to point at — so a re-derivation of the thing being presented became inexpressible,
while a gratuitous syscall stayed possible and was **explicitly no longer claimed against.** A
narrower claim that is structurally true beats a broad one held up by a list.

**The check: for each observable an argument rests on, ask who emits it.** If the answer is *the
test does, at the point it thinks is correct*, the criterion is testing the author's
understanding rather than the program's behaviour. A production-emitted trace, a type the
product cannot construct incorrectly, a compiler-resolved rule — those are observations. A
`Vec` the test appends to is narration.

Two corollaries earned the hard way:

- **A later log cannot establish an earlier sequence.** If the marker is written after the
  operation returns, the operation is outside it, no matter what the marker says.
- **A premise is not a boundary just because it is true.** Two real, checkable facts — no
  dependencies, no unsafe — were cited as closing an enumeration gap. They close *third-party*
  and *libc* routes. The gap was *unlisted safe-standard-library* routes, which neither touches.
  Check what a premise covers, not whether it holds.
- **An honest limitation stated beside an over-claim makes the over-claim HARDER to see.** That
  guard's own file said, in plain terms, that it enumerated entry points — and then called the
  enumeration a boundary. Its author wrote both sentences in one sitting and did not notice they
  contradicted each other; the reviewer read the admission as candour, credited it, and stopped.
  **Candour reads as sufficiency.** When a document discloses its own limit, that is the moment
  to ask whether the rest of the document respects it, not the moment to relax.

### The wrong-set sequence: grain, predicate, proxy, gate, spelling

The same error appeared four times in one project, at four different depths, and every instance
produced **arithmetically perfect** numbers. It is worth naming as a sequence, because
recognising the second instance did not prevent the third.

1. **Wrong GRAIN.** A count keyed on a field stamped at *case* level while the obligation was
   defined *by surface*. Real field, correctly populated, in the obvious place — 86 rows
   inherited a property they did not have. The verifier nearly filed a disagreement.
2. **Wrong POPULATION.** A figure reported as *45 of 91* over a population narrowed twice
   without saying so: an unstated exclusion dropping 86 of 177, and a class assigned by a
   path-shape proxy confirmed on two specimens. The measurement was honest; the denominator was
   undeclared.
3. **Wrong DECIDING FIELD.** The queried server was *inferred from a path shape* when another
   captured file recorded it directly. Every number built on it was measured over a proxy for
   the thing that decides — and the class most confidently labelled "established by a positive
   marker" turned out to be established by a marker **about a different server**.
4. **GATE — inside the generator.** A per-candidate obligation gated on a case-level condition
   the governing rules never required. This one emitted *nothing* rather than something wrong,
   so no count moved and no gate fired. It surfaced only because two people counted a related
   thing and disagreed.
5. **SPELLING.** A reviewer matched a phase label as a **substring**, so `P1` also caught
   `P1-ADJACENT` — 1414 rows where the population was 1065. Committed by the person who had
   written up instance 4 minutes earlier.

**The common shape: a fact about a container being read as a fact about its contents.** A case
is not its rows. A corpus is not its P1 subset. A path prefix is not a recorded pointer. And a
case-level failure is not a per-candidate one.

Three things this sequence teaches that the individual instances do not:

- **Recognising the class does not confer immunity.** Each instance was found *after* the
  previous had been written up, by the people who wrote it up. The reviewer who documented the
  granularity trap then matched `P1` as a substring and caught `P1-ADJACENT`.
- **The deeper it goes, the quieter it gets.** A wrong grain produces a wrong number someone can
  argue with. A wrong conditional in a generator produces *silence* — no record, no count, no
  red — and silence has no reviewer.
- **Ask which of the five you are at.** Before trusting any figure: is the field the deciding
  one, is it at the right grain, is the population stated, does the code that produced it gate
  on something the rules do not require, and is the match exact?
- **Re-checking the arithmetic catches none of them.** Every instance summed correctly, because
  a wrong set produces a right-*looking* answer. **A second, differently-derived number is the
  only detector** — which is also why deferring to the more confident source destroys the signal.
- **The failure is not about care.** Instance 5 was committed by the author of instance 4, in
  the same hour, having just written the warning. Treat it as a standing hazard of counting
  rather than as a lapse to resolve not to repeat.

### An escalation that carries its expected answer gets the frame confirmed, not examined

A question was escalated with the reasoning attached: *here is the gap, here is what the rows
appear to say about it by construction, and here is why the consequence is too large for me to
rule alone.* That reads like diligence. Everything in it was offered as reasoning to be checked.

The ruling came back saying **no gap existed.** The rows already settled the branch; the
escalator had read a gap into them and then, having read it in, sized it — producing a
carefully-measured estimate of something that was not there.

And the same escalation carried a second inherited claim, relayed with endorsement: that a
derived table had been *safely confined* to cases the evidence supported. It had not. Two
hundred and forty of its records asserted an outcome nothing had captured. **A scoping claim
accepted without testing, and then passed on as reassurance, is worse than one merely believed —
it arrives at the next reader with a second signature on it.**

The mechanism is ordinary and hard to feel: a question shaped as *is X true?* invites a check of
X. A question shaped as *X appears true by construction — confirm?* invites agreement, because
the frame now carries the escalator's authority as well as their evidence. The more senior the
escalator, the stronger the pull, and the more carefully the reasoning is laid out, the more it
reads as settled.

**So escalate the observation and withhold the conclusion.** State what was measured, state what
is undetermined, and stop — offering the expected answer only if asked for it, and marked as a
guess rather than a reading. Where you must offer one, say explicitly which finding survives if
it is wrong: here the shape (*a recorded pointer can name something never observed*) held
perfectly while the interpretation (*therefore those cases must diverge*) inverted.

**And re-test every inherited claim you are about to repeat.** Relaying is not neutral. The
claim that was refuted here had been examined by nobody, endorsed by two people, and would have
reached a third with the weight of both.

### A count can be honestly MEASURED over a population you narrowed without saying so

A figure was reported as *45 of 91 cases*. Both numbers were measured. Neither was wrong. And
the population had been narrowed twice, silently, before the measuring began:

- **An unstated exclusion.** The predicate required a particular field, and 86 of 177 cases
  lacked it. They were dropped without appearing anywhere in the report — so *91* looked like
  the corpus and was in fact a subset less than half its size.
- **An unverified proxy.** One class was assigned by a *shape* — a path prefix standing in for
  the relation actually of interest — confirmed by hand on two specimens and then generalised
  across the class, never compared field to field.

Nothing about the number looked wrong, because **the arithmetic was never the weak part.** The
author found it themselves while pulling one specimen per class for someone else to reproduce:
the excluded specimen turned out to record the same fact under a different spelling.

**A count is only as honest as its denominator, and the denominator is where the undeclared
work hides.** So report a population the way you report a measurement: the predicate that
selected it, what that predicate excluded, and how many. *45 of 91* and *45 of 91, having
dropped 86 of 177 for lacking a field and assigned one class by a proxy verified on two
specimens* are the same measurement and different claims.

**And when the finding is about a SHAPE, lead with the shape.** Here the substance was that a
recorded pointer can name something the evidence never observed — true whether the count is 45,
43 or 48. Leading with a number invites everyone, including its author, to argue about the
number, and puts the weight on the part most likely to move. State the mechanism first and
offer the count as an indication of scale.

The pattern beneath both defects: **the same author had earlier keyed a count on a field of the
wrong granularity.** That was one level down — a field whose grain did not match the obligation.
This is one level up — a population defined by predicates that were never stated. Both produce
a number that is arithmetically perfect and answers a question nobody asked.

### Check that the field you COUNT BY has the same granularity as the thing you are counting

A figure needed independent verification: how many rows carry a newly-added obligation. The
obvious key was right there — a column recording whether that case's server was unreachable.
Counting it gave **659**. The figure under test was **573**.

An 86-row disagreement, from a real field, correctly populated, in the obvious place.

The field was **case-level**, stamped on every row of an affected case. The obligation was
defined **by surface** — *every successor JSON digest*, *human list/ls output*. So 86 rows sat
in unreachable cases while being neither digests nor list/ls output, gained neither obligation,
and correctly carried no divergence. The count was answering a subtly different question than
the one asked, **and both questions were reasonable ones about the same data.**

The reviewer nearly filed it as a disagreement, and would have been wrong while holding a
number derived by an independent method — which is exactly the outcome independent verification
is supposed to prevent.

**So before trusting any count, state the granularity of the obligation and the granularity of
the field you are keying on, and check they match.** A coarser field over-counts by inheritance;
a finer one under-counts by splitting. The error is confident in both directions because
nothing about the field looks wrong — it is real, populated, and named after the thing you care
about.

The tell is a *disagreement* rather than an absence: two competent derivations of the same
figure differing by a clean, explainable margin usually means one of them counted a different
population, not that either is broken. **Reconcile the populations before arguing about the
number.**

**And do not assume the reconciliation will settle it.** In one case both counts reproduced —
six was *selector-missing AND failed-case-query*, ten was *selector-missing* — and neither was
wrong. The disagreement pointed at a **third thing**: the generator gated a per-candidate
obligation on a case-level condition the governing rules never required, so sixteen rows carried
no directional obligation at all. Reconciling found a defect in the machinery rather than an
error in either tally.

**So a disagreement between two competent derivations is not only a population question** — it
can be a defect in whatever produced one of them, and the person best placed to find it is often
the one about to defer. Here the author of the lower-confidence count offered to stand down,
having been wrong twice before on populations, and **accepting that deference would have buried
the finding**, because the mismatch was the only reason anyone looked. Reconcile rather than
defer, even when your own track record argues for deferring.

### A DERIVED artifact goes stale the moment its source moves, and nothing re-runs to say so

A machine-readable column was derived from a contract: for each row of evidence, whether the
successor must match the frozen output or diverge from it, and **which contract rule mandates
that divergence**. Carefully built, independently verified, red-proofed both ways.

Then the contract grew a rule. The column could not name it — the rule did not exist when the
column was derived — so rows already marked *diverge* now diverged for the **old** reason while
the new rule's obligations went unrepresented. A checker asking only *does this differ?* passes
them. **The row diverges for the reason recorded and violates the reason that is not.**

Nothing in the system reports this. The column is present, well-formed, provenance-stamped, and
consistent with the contract *as it stood*. Its lineage is accurate and its currency is not,
and **lineage is what provenance stamps record.**

So for anything derived from a moving source:

- **Give it a freshness relation, not just a provenance stamp.** Pin the source revision *and*
  make staleness detectable — a check that the derivation still reproduces, or a recorded
  source identity a later gate can compare against. "Derived from X" is a fact about the past;
  "still agrees with X" is the property you actually rely on.
- **When the source grows, ask what was derived from it.** That question is nobody's job by
  default, which is why the answer is usually *nothing was re-run*. A contract amendment should
  carry a list of its dependents the way a schema change carries its consumers.
- **The check must be AUTOMATIC, not periodic.** Since amendment size predicts nothing, *"was
  that a big change?"* is not a usable trigger — a human deciding when to re-verify will
  reliably skip the one-line rule that reached every artifact. Gate on a stored source hash so
  staleness is *reported* rather than noticed.
- **Do not size the invalidation by the size of the amendment.** In this instance a single new
  rule made the reason-projection stale on **every** divergent row in the column, not on the
  class it obviously touched — because the obligation it added reaches a field that every
  emitted document carries. The first reading of the finding named one identifiable subset, was
  the comfortable size, and was wrong by a factor of two. **A small change to a source can
  invalidate a derived artifact entirely, and the diff is no guide to the blast radius.**
- **Re-derive by a DIFFERENT method than the one that built it.** Running the original
  generator again inherits its blind spots exactly — and a generator that has already been
  caught mis-deriving one class has a demonstrated one.

The sharper version of the defect: **"expect divergence" degrades into "expect anything but
this."** A verdict that records only *that* something must differ, not *how*, cannot fail an
implementation that differs wrongly — which is the whole population it exists to judge.

### An obligation can be discharged by the TYPE, and then its test is a restatement

A criterion required that a failed query never supply proof — specifically, that *partial output
from a failure* could not be read as evidence either way. A test existed for it, running the
same payload bytes through a successful arm and a failed arm.

The failed arm could not carry those bytes at all. The result type was
`Result<Vec<Session>, QueryFailed>` with `struct QueryFailed;` — a failure with **no room for a
payload**. So the fixture's own comment described something it could not express: both
iterations of its loop produced the identical empty error, byte-identical to itself, and the
payload never reached the arm it was meant to contaminate.

**The obligation was already discharged, by the type, more completely than any test could.**
That is a good outcome — but it means the test is a *restatement* of a guarantee, not evidence
for it, and treating it as coverage points attention at the wrong layer.

**The risk relocates to wherever the type is constructed.** Something decides success from
failure, and *that* decision can absolutely be fooled by output — an adapter that inspects
stdout, a wrapper that treats an empty result as an error, a parser that succeeds on garbage.
In this codebase that seam had its own direct test, looping empty, plausible-looking, and
error-shaped payloads against a failing run. That is where the criterion actually lives.

Two things to take from it:

- **When a criterion is satisfied structurally, say so in the criterion**, and point at the
  boundary that now carries the risk. Otherwise the next reader sees a green test and believes
  the wrong layer is guarded — and the day someone widens the error type to carry a message,
  the restatement test still passes.
- **The detection method generalises: ask why a differential arm never varies.** An arm whose
  input cannot differ is either enforcing something structurally or testing nothing, and both
  answers are worth having. It is the same question that exposes an unvaried axis, turned on a
  single arm.

### A verification confirms WITHIN its scope and cannot see its scope

A reviewer was asked to find which axes a body of tests varies. They were told: *do not read
`src/` beyond what the tests import — the tests are the subject.* Sensible on its face, and
meant to keep their read independent of the implementation.

The unit tests lived in `src/`.

So they measured over one integration file, found twelve criteria with no test and two facts
never varied, and reported it. **Every count was correct.** And every conclusion was wrong,
because a second file held named tests for all twelve.

**The instructive part is what they did next, which was right and made it worse.** They asked
the correct verifying question — *is this genuine absence, or just a naming difference?* — and
probed the vocabulary each missing criterion would need: prefix, grouping, mismatch, same name,
two servers. All zero. A well-chosen check, correctly executed, **which confirmed a false
conclusion, because it ran inside the same wrong boundary as the count it was checking.**

**Rigour inside a bad scope produces confident error rather than caught error, and more rigour
produces more confidence.** No verification reaches outside its own frame; that is what makes a
frame a frame. The reviewer could not have caught it, because the boundary came from someone
else and honouring it was correct.

So scope is the reviewer's responsibility to state and the *requester's* to get right:

- **Name the boundary in terms of what the artifact IS, not where it lives.** "Do not read
  product code" and "do not read `src/`" are different instructions in any language that
  colocates tests with implementation, and only one of them was meant.
- **Before accepting an absence result, ask what would exist outside the frame if the finding
  were false.** One question — *where else could a test for this live?* — costs nothing and is
  the only check that looks at the boundary rather than through it.
- **An absence is the finding most sensitive to scope.** A positive finding survives a
  too-narrow frame; a negative one is manufactured by it.

The reviewer's own account of why they could not see it is the useful part: **the boundary was
invisible because it was plausible.** *Tests live in `tests/`* is true in most repositories and
false in Rust, so the probe was well-formed for a question one directory too narrow. Their
prescription is better than a better probe: **count the evidence base before measuring inside
it, and ask what would have to be true for this to be all of it.** That question interrogates
the frame; every question asked after it interrogates only the contents.

### A DELEGATION is a claim about scope, and being talked out of a concern is not resolving it

A reviewer noticed a gap: an alias command was untested for most filters. They looked for it
deliberately, found it, and then read a sentence in the same document explaining that alias
parity was covered by an older obligation elsewhere. They accepted it and wrote *"delegated
deliberately and said so"* into the review as a point in the document's favour.

The delegation was real and its scope was wrong. The older obligation covered the
**pre-existing** behaviour; the filters this phase changed were **new**, so a defect in the
alias under the new semantics was outside what that obligation was ever written to catch. The
document's author found it later by self-reading.

This is not *missing* something. **It is finding it and being argued out of it by a sentence** —
and that is more dangerous, because the noticing already happened and got spent. Locating the
explanation *feels* like discharging the concern: you did the work, the document answered, the
item closes. Nothing about that sequence prompts the one remaining question.

So treat every "covered elsewhere" as **an unverified claim with two halves**: that the other
rule exists, and that **its scope contains this case**. The first is a lookup and is nearly
always true — which is exactly what makes it satisfying. The second is the actual check, and it
is skipped most reliably when the delegation is *accurate about the thing it names*.

The concrete habit: when a document points you elsewhere, **go read the elsewhere and ask what
it was written to catch**, not merely whether it is about the same subject. Obligations written
before a change do not automatically extend to what the change introduced — and a phase that
alters semantics has, by definition, produced cases its predecessors were not written against.

### A structural criterion measures a PROXY, and satisfying it can defeat the goal

A rule forbade observing mutable state twice: the digest was reading session files again, after
liveness had been decided, so a record repaired between the two reads could erase the fact that
it had ever been damaged. The criterion expressed this as **one read**.

The repair consolidated all I/O into a single function. Read count: one. Criterion satisfied.
**And it created a fresh defect of exactly the kind the rule existed to prevent** — because that
one read kept only the *success* and discarded the error, so the distinction between *no file
here* and *a file I could not open* was destroyed at the moment of reading. Something downstream
then had to **look again** to reconstruct it, using a different question (*does this path
exist?*) that answers wrongly when a directory cannot be searched at all.

So the second observation was not removed. It was **relocated, and made worse**, while the
metric the criterion counted improved.

**One read** was a proxy for *one consistent observation*. Reading once and throwing away what
you learned forces a second look, and no count of call sites can see that. The general form:
**a structural criterion is a proxy for a property, and a repair aimed at the proxy can move the
violation somewhere the proxy does not look.**

Two habits follow:

- When a criterion counts something, **name the property it stands in for**, in the criterion.
  *One read* should read *one observation, whose full outcome is preserved and never re-derived*
  — then a fix that discards the outcome fails on its face instead of passing on the count.
- When reviewing a repair, ask **what information the repair destroyed**, not only what it
  removed. Consolidation, normalisation, and simplification all narrow types, and a narrowed
  type is where a distinction goes to die. `.ok()` on a fallible read is the canonical instance:
  it converts *why it failed* into *whether it worked*, and every consumer that needed the why
  must now guess.

### A guard that enumerates NAMES enforces those names, whatever its title claims

A defect was found: a whole-phase obligation verified against one function and one file. The
repair replaced it with what everyone — including the reviewer who passed it — called a
crate-wide guard. Its stated claim was *exactly one filesystem call outside the tests*. Its
implementation counted three identifiers.

A different spelling of filesystem access, under none of those three names, sat live inside the
guarded region. So the guard reported structural closure over a codebase that was still making
the observation it existed to forbid — **and the claim in its own title is what stopped anyone
checking.**

The reviewer's error is the instructive half: they verified the guard covered the three names it
listed, and never asked **whether those three names were all the ways to do the thing**. That
is the same universal-on-a-particular shape the guard was written to fix, occurring inside the
guard, and it survived a review by someone who had documented that shape hours earlier.

So when a guard's title quantifies over a **capability** — no filesystem access, no process
spawn, no network — and its body enumerates **identifiers**, the gap between them is unbounded
and invisible. Two consequences:

- **Do not repair it by lengthening the list.** A longer blacklist is the same defect with more
  entries, and each addition makes the claim feel better supported while the gap stays open.
  Enforce the capability where the language or toolchain can enforce it — a lint that bans a
  *type* rather than a call, a module boundary, a signature that cannot express the operation —
  or **narrow the title to name exactly what is counted.**
- **Review a guard against its claim, not its body.** Reading the body tells you what it does;
  the finding lives in the difference between that and what it says. Ask: *what else would
  satisfy this sentence and not trip this code?*

The strongest version of this project's guards are the ones with nothing to enumerate — a type
that cannot spell an archive path, a port with no name parameter, a function whose signature has
no failure mode. Those cannot drift, because there is no list to fall behind.

**The repair that worked here did both halves, and neither alone would have been enough.** The
tripwire was narrowed to *claim only what it checks* — it now states that it scans three names
and **names what it cannot see**: `Path::exists`, `metadata`, `File::open`, any wrapper. Then
the capability was closed **behaviourally**, where a spelling has no purchase: render the
output, grow the filesystem underneath, render again, delete the tree, render again, and require
all three byte-identical from the same captured input. A second observation under any name then
sees a different world and fails — **for the facts the fixture actually varies.**

**That last qualifier was missing when this was first written, and the counterexample landed
within the hour, against this very sentence.** The test grew and changed the *meta*. Every
fixture in it had an **absent event log**, and an absent log maps to the same quiet stream
before growth, after growth, and after deletion — so a reread of the event half, through any
helper under any spelling, observes nothing different in any arm and passes all three byte
comparisons. The guard was sound for one half of the record and blind to the other, while its
title claimed the class: the same claim-exceeds-enforcement defect this entry exists to describe,
committed into the entry itself.

**A differential test discriminates exactly over the axes it MOVES.** Anything held constant in
every arm is invisible to it, and an *absent* input is the most constant thing there is — which
makes "we deleted it and nothing changed" the weakest available evidence about a source that was
never there.

**And the deletion arm is weak a second way, which the repair's red-proof exposed.** A planted
second read reddened the **growth** arm only. The removal arm stayed green — because a reread
that *fails* on a deleted tree simply keeps the value already carried, producing the same output
a correct implementation produces. So the arm that feels most decisive, the one where you tear
the world away, is the least discriminating of the set: blind to a source that was absent, and
forgiving of a re-observation that errors.

**Growth is what discriminates.** A second read only betrays itself when it *succeeds and sees
something different* — so the fixture must make the later world both **readable and opposed**,
not merely gone. Prefer arms that change a value over arms that remove one, and when a
differential has both, know which one is carrying the proof. So enumerate what the fixture varies, and either plant a live observable fact on
every axis the claim covers, or narrow the claim to the axes actually planted. **An unused
observation cannot be closed behaviourally at all**; only a real capability boundary closes that.

And the control is what makes it evidence rather than design: a second read was planted using
the spelling that had defeated the old guard — **invisible to the tripwire by construction** —
and the behavioural test went red. When you replace a guard that missed something, **prove the
replacement catches the specific escape that beat its predecessor.** A new guard asserted to be
broader is a claim; a new guard demonstrated against the old failure is a measurement.

### A UNIVERSAL obligation checked on ONE fixture is checked nowhere

Three separate findings in one review turned out to be the same defect wearing different
subjects, and the pattern is worth more than any of them:

> **The criterion that PRODUCES the artifacts is not the criterion that EXAMINES them.**

A rule said *every* emitted document carries a certain field, *even when empty* — plainly
universal. One criterion emitted six document shapes and inspected them for two other
properties, never that field. A different criterion mentioned the field, on the two documents it
built for its own purpose. So a serializer that emits the field **only on the paths the second
criterion happens to construct** satisfies both, plus a third that fails only on fields *dropped
or renamed* — and a field that was never there to drop is neither.

The same shape had already appeared as an orthogonality proven in the classifier and unchecked
in the serializer, and again as a shared fixture consumed with full rigour by two phases and
loosely by a third.

**The fix is placement, not strength.** A universal obligation has to be asserted **where the
artifacts are enumerated** — inside the criterion that already loops over every shape — rather
than demonstrated once wherever it was most convenient to construct one. Adding a stronger
assertion in the convenient place does not help; it is still one fixture.

Two diagnostics that find these cheaply:

- **Grep the obligation's own noun through the gate.** If a required field, flag, or invariant
  appears in exactly one criterion while some other criterion is the one generating instances,
  that is the defect, and the count is the tell — here the field appeared **once**.
- **Read the quantifier and then ask where the loop is.** *Every*, *always*, *in all cases*,
  *even when empty* — each names a set, and the check belongs wherever that set is produced. If
  the producing criterion and the checking criterion are different criteria, the obligation is
  only as universal as one author's fixture list.

### A fixture can succeed at building a state the product could never produce

A test needed a healthy source alongside two deliberately broken ones. The criterion said to
use "an entitled server". Reasonable — until someone traced where entitlement comes from: it is
derived from the ambient server plus selectors read out of durable records. The fixture had made
**both durable roots unreadable**. So there were no records, hence no selectors, hence no
entitlement except ambient.

A generic "entitled server" in that construction is therefore a server the harness **planted**
and the product could never have reached. The fixture builds fine. The code under test runs.
Every assertion passes. And the run says **nothing about reachable behaviour**, because the
precondition it depends on cannot occur.

This is worse than a fixture that fails to create its condition — that one at least breaks. Here
the setup **succeeds**, so nothing signals that the state is unreachable, and the green result
looks exactly like proof.

The check is to trace **provenance, not preconditions**. A criterion reads as a list of things
that must be true; that framing is what invites planting them. Ask of each precondition: *by
what path would the product itself arrive here?* If the answer requires a fact the rest of the
fixture has just destroyed, the case is unreachable and the assertion is vacuous.

It bites hardest exactly where fixtures are most aggressive — when several things are broken at
once to test resilience, the breakages interact, and one of them quietly invalidates the
premise another one needs. **The more thorough the setup, the more likely some part of it is
unreachable.**

A second exhibit, and note where it landed. A test existing to prove two facts **independent**
hand-built its cells by pairing a positive selector with an absent record — a combination the
constructor cannot emit, because it assigns that selector only on the branch where the record
parsed. So the criterion whose entire purpose was proving independence demonstrated only that
manually contradictory structs survive processing, and said nothing about whether the product
reaches those cells at all. **The off-diagonal has to be reconstructed through the real
constructor**, with the second axis varied by an independent, product-valid cause. Both exhibits
were caught by the *other* seat, each in work the first had already reviewed and passed —
unreachability is close to invisible to whoever built the fixture, because they know what they
meant it to represent.

### Settling an ambiguity is not the same as finding what happened while it was ambiguous

Two rules contradicted each other. The reviewer who found it routed the question upward — which
reading governs? — and treated that as the finding. The ruling came back, and the ruling was
the easy half.

The other seat answered the reading **and then went to look at what the implementation had
already done under it.** The product was violating the rule on either reading: a second read of
mutable state, in a place neither reading permitted. That would have shipped behind a resolved
ambiguity, because everyone's attention had moved to the wording.

An unresolved rule never stops code from being written. It just means **nobody checked which
side the code landed on** — and the interval where a rule is ambiguous is precisely the interval
where an implementation guesses, unreviewed, with the honest excuse that the rule did not say.
So an ambiguity is a *lead*, not a finding. The finding is downstream of it.

Routing an ambiguity feels like discharging it: you noticed, you escalated, someone with the
authority answers. But the answer changes nothing about code already written against the gap.
**Every ambiguity report should carry the survey with it** — here is the question, and here is
what the current implementation does on each reading. Sometimes the survey makes the question
moot, because one reading is already violated and the answer cannot save it.



### Adding the missing member to an enumerated rule PRESERVES the defect that produced it

A rule named three places an enumeration could fail. A fourth turned up. The obvious repair is
to add it — and that repair is wrong, for a reason worth stating precisely:

> A fourth named class would repair today's inventory while **preserving the defect's cause**:
> treating the leaves of the *current* traversal graph as the normative domain.

That is a different claim from "the list was incomplete". Incompleteness is an accident you
patch. **The list was never the right kind of thing** — it defined membership by pointing at
today's instances, so every future instance needs another edit, and each edit is another chance
to miss one. The fix is to state the **property that makes something a member**, and demote the
current names to explicitly non-exhaustive examples.

The tell that you are looking at this failure: the missing member was *obvious once seen*, and
the argument for adding it is that it "clearly belongs". Clearly belongs **to what**? Answer
that, and you have written the principle.

**A generalisation needs its bounds in the same edit, or it becomes a widening.** Three clauses
did that work here and each is reusable:

- **Keep the existing exclusions verbatim.** They bound the principle to what was actually
  required, and stop "any required traversal that failed" collapsing into "any I/O error".
- **A failed step records its OWN loss only** — never fabricated losses or identities for
  children it could not discover. **You cannot count what you could not see**, so a loss count
  is of *operations attempted and known to have failed*, never of the unknown quantity hiding
  behind them. The other reading makes the count unbounded and unverifiable.
- **Fix the word that caused the gap, in the same edit.** The exclusion here came from
  "terminal", which reads as both *leaf of the graph* and *final after retries*. The wrong
  reading is what let the intermediate node escape. Disambiguating it is what stops the same
  gap reappearing under a new name.

### A clarification that SPLITS a concept creates a coverage gap in evidence captured before it

A rule said a certain field was `missing` when its keys were absent. It was ambiguous about a
neighbouring case, so it was clarified: `missing` now means *no fact is available to the
reader*, which covers both an unreadable record and a readable one whose keys are absent — and
the two are declared **orthogonal** to a second, separately tracked fact about whether the
record could be read at all.

The clarification is correct and it immediately created a hole nobody put there. The old,
undivided concept had one shape and the corpus exercised it. The new one has **two** shapes:

- unreadable record → `missing` **with** the read-loss fact — 68 corpus rows
- readable record, no keys → `missing` **without** it — **zero** corpus rows

Every instance the evidence holds sits on the **same side of the new distinction**. So an
implementation that derives one fact from the other — collapsing exactly the axes the rule now
declares independent — is wrong in a way that body of evidence **cannot detect**, and it became
undetectable at the moment the rule got more precise.

The general shape: **evidence is only ever as discriminating as the distinctions that existed
when it was captured.** Sharpening a definition does not sharpen the corpus; it silently
promotes a previously-adequate body of evidence into an inadequate one, and nothing re-runs to
tell you. The gap is created by an edit to a *document*, so no capture, test, or gate reports
it.

So whenever a rule is split, refined, or made orthogonal to something, **immediately ask which
side of the new line the existing evidence sits on.** If it is all on one side, the distinction
is unverified by construction, and only new fixtures can close it — which is worth knowing on
the day of the edit rather than at the gate that fails to catch it.

### Forward verification cannot see a FALSE NEGATIVE — you have to check the converse

A classification pass labelled 1065 rows as expected-to-differ or expected-to-match, and was
then verified: open the rows marked *differ*, confirm they really do. Every one checked out.

The error was in the other set. Rows whose captured output carried **no status field at all**
— an empty listing, `"sessions":[]` — had been scored *match*, because the derivation keyed on
whether a status label was present to change. But the governing rule changed the view's
**membership**, not just its labels: entries that become `unknown` appear where the old
product showed nothing. **The output that diverges most visibly was the one the derivation
excused**, and 268 rows carried it.

No amount of forward checking finds this. Confirming that flagged items deserve their flag is
structurally blind to items that were never flagged — the check ranges over the positives, and
the defect lives in the negatives. This is the same asymmetry that makes a green test suite
weak evidence: it tells you what you asserted, not what you failed to assert.

**So verify both directions, and state the converse as its own assertion.** Not "each divergence
row really diverges" but also "no row carrying a machine digest is scored a match" and "no
unreachable-server listing is scored a match" — claims that a wrong *exclusion* violates. Then
red-proof the converse arm by flipping one correctly-flagged row into the excluded set and
confirming it is caught.

A rule of thumb for where to look: **the cheap half of a verification is the half that ranges
over the things you already found.** The expensive half ranges over everything you didn't, and
only a converse property makes that set checkable at all.

### A condition that was never CAPTURED cannot later be shown to have been ABSENT

A corpus was audited for whether it exercised four failure conditions. Three answers were
possible, and only two were expected.

One condition — a session missing its ownership marker — turned out to be **unobservable**.
The marker is proved by querying the tmux *session environment*; the capture recorded pane
options. So the corpus cannot say the condition was present, and it cannot say it was absent.
It is silent, **and silence in a corpus reads exactly like coverage**: the rows are there, the
captures are complete by their own manifest, nothing is missing or malformed. Only asking
*what field would have recorded this?* separates "did not happen" from "was never looked at".

This is the absence-of-evidence trap with a specific, checkable remedy. For any condition a
body of evidence is claimed to cover, name **the field that would carry it** and confirm that
field exists in the capture. If no field could have recorded it, the corpus is not weak
evidence about that condition — **it is not evidence about it at all**, and any coverage claim
that includes it is unfounded rather than optimistic.

Report it as a third answer — present / absent / **unobservable** — because folding it into
"absent" is how it becomes coverage.

**And size it as a fraction of the evidence base, not as a list of cells.** In this project the
unobservable answer came back three times, and the third instance covered **45 of 91 cases** —
half the corpus — where a session's recorded server differed from the one the capture observed,
and that server's state was never written down anywhere. The first two instances were a single
missing field and a single unvaried axis, which is what made the third one surprising: the
same answer at a completely different scale.
So the useful question is not *does any case exercise this?* but **what fraction of the evidence
base can answer on this axis at all** — because a corpus that is 50% silent about something
will still contain plenty of cases that look like they cover it.

**And keep the third answer separate afterwards, because the two gaps cost different things to
close.** *Absent* means the evidence could have recorded the condition and no case happened to
exhibit it — one new fixture closes it. *Unobservable* means no field in the capture could have
carried it, so nothing short of re-capturing the whole body of evidence will ever answer, and
until then every claim over that condition is unfounded rather than merely unproven. Absence is
the cheap gap; unobservability is the one that quietly caps what the evidence can ever be asked.

### A measurement error that produces COMFORT gets used; one that produces alarm gets checked

The same audit first reported **146** rows exercising a prefix-collision defect. The real
number was **zero**.

The error: the count tested whether a prefix *pair co-occurred* in a fixture. The defect fires
only when a **non-live** candidate's name is a strict prefix of a **live** one — an asymmetric
condition. Co-occurrence and firing differ by exactly the asymmetry the defect depends on, and
in every co-occurring case the pair was oriented the wrong way round.

What makes this worth a rule is not the mistake but **its direction**. 146 says *that path is
well covered* — a result that ends the inquiry, gets quoted downstream, and never gets a second
look. Zero says *we have a gap* — a result that gets re-derived, argued with, and checked.
**Errors that flatter coverage are systematically less likely to be caught than errors that
threaten it**, because scrutiny is spent on findings that cost something.

So audit the reassuring numbers, not the alarming ones — the alarming ones defend themselves.
And when a count stands in for a *condition*, state the condition's firing shape explicitly
before counting: every simplification that drops an asymmetry, an ordering, or a negation
produces a number that is too large, and too large is the comfortable direction.

**The check, because "be suspicious" is not one:** when a measurement reports coverage, **go
read one of the cases it says is covered** before believing the count. Not a sample, not a
review of the query — open a single covered case and confirm it exhibits the condition. That
is how the 146 was actually caught: its author went to quote a concrete example and the
example did not behave the way the number implied. Reaching for an example is the cheapest
operation that can falsify a count, and it is the one a comfortable number never prompts.

Its converse is the strongest argument for the check: a parity suite that verified only the
512 rows expected to match **would pass a binary that faithfully reproduced every defect it
was built to catch.** Coverage counted over the cases that agree is not coverage.

### If a judgement must be INDEPENDENT of a thing, it has to be made BEFORE that thing exists

Two decisions in one session ran into the same trap, from opposite directions.

**First:** a per-row expected-outcome column was deferred until the implementation existed,
on the reasoning that a divergence assertion needs something to assert against. But the
column was meant to be **pre-registered**, and a verdict computed after the code exists is a
verdict that had the opportunity to be shaped by what the code turned out to do. Waiting
would have destroyed the exact property the column existed to provide.

**Second:** acceptance criteria for a build were about to be written when the diff arrived —
the natural moment, because that is when the work is concrete. It is also the moment they
stop being independent. You read the implementation, the implementation suggests what to
check, and the checks pass because they were derived from the thing under test.

The general shape: **the intuitive schedule and the required schedule run opposite.** Work
that must be independent of an artifact feels like it needs that artifact to be concrete, and
every day you wait makes it easier to write and less worth having. The deferral never
announces itself as a loss of independence; it announces itself as *being better informed*.

So: **name the thing each judgement must be independent of, and schedule it before that thing
exists.** Expected values before the run, acceptance criteria before the diff, the falsifying
condition before the measurement.

The practical tell that you have waited too long: you can no longer state the criterion
without referring to how the implementation works. At that point the criterion is a
description, and a description cannot fail its subject.

### Fixing an instrument and TUNING it produce identical diffs

An independent checker was built to cross-examine a normaliser: two methods, computed
separately, expected to agree. They disagreed twice, and the two repairs looked the same
from outside.

**The first was a real bug in the instrument.** It reduced each path to its basename, so two
genuinely different files collapsed to one — and it then reported the normaliser as wrong for
having *correctly* distinguished them. **An instrument that discards a distinction cannot
judge whether that distinction was preserved**; it will always report the more careful party
as the defective one.

**The second was the trap.** Two remaining disagreements were the same invocation recorded
with and without an interpreter prefix. The available move was to keep adjusting the
independent method until it agreed — and each adjustment would have been individually
defensible, produced a smaller diff than the first fix, and turned the report green.

That move **destroys the independence the check exists to provide.** An instrument tuned
until it matches its subject is no longer evidence about the subject; it is a second copy of
it. The honest repair was to stop editing the instrument and **declare the equivalences as
rules** — an interpreter token carries no meaning; a binary path's meaning is its basename —
so the claim is stated and reviewable rather than absorbed into code that now silently agrees.

**Both edits touch the same file, shrink the same disagreement count, and end green.** Nothing
in the diff, the commit, or the test output distinguishes them. The only thing that does is
whether the change was derived from the instrument's own definition or from *the answer it
was supposed to produce* — and that lives in the author's head unless they say so.

So: when a cross-check disagrees, record *why* each repair was made before making it, and be
suspicious of any instrument edit whose justification is that it removes a disagreement. The
author who flags this about their own work — as happened here, unprompted, at the cost of a
clean two-arm pass — is giving you the only signal that exists.

### A type-system guarantee has a precise scope; the confidence it creates does not

An enum gained a variant. The compiler pointed at one site — a `match` — it was fixed, the
build went green, the suite passed, and the change looked complete.

Elsewhere the same enum was enumerated in an **array literal**: a per-scope list of which
variants that scope displays. Rust's exhaustiveness checking covers `match` and nothing else,
so the array compiled unchanged and the new variant was silently absent from every scope. A
session in the new state would have been dropped from all output — by a file the compiler had
just implicitly certified by saying nothing about it.

The trap is not the array. It is that **"the compiler found the sites" is a claim the
compiler never made.** It reports the sites *it checks*; the reader hears *the sites*. Every
other enumeration of the variants — const lists, map initializers, match arms behind a
wildcard `_`, a `to_string` round-trip, serialization tables, test fixtures asserting
"all" — is invisible to it, and each looks exactly like code that would have failed if it
were wrong.

So when a closed set gains a member, **grep for the set's members by name** and treat the
compiler's list as a lower bound. Better, close it structurally: a test asserting the
enumerated collection covers every variant fails when the next one is added, which is
precisely the day the compiler will not.

The generalisation past Rust: every automated guarantee has a documented scope, and the
confidence it produces is scope-free. A tool that verifies X reliably will be heard as
verifying "this is correct" — so the useful question about any green check is never *did it
pass* but **what, exactly, did it look at, and what sits just outside that?**

### A rewrite inherits the original's CONFLATIONS through its type definitions, before any logic exists

A rewrite froze its predecessor and set out to re-derive every behaviour from ratified
evidence rather than copy it. It was careful: the new binary literally **refuses** to perform
its main command, on the ground that the surfaces involved are not ratified yet.

Meanwhile its `Status` type had two variants, `Running` and `Stopped`.

The predecessor could not distinguish "this session is stopped" from "I could not reach the
server to ask" — it printed `stopped` for both. That is a real defect, filed and confirmed.
And the rewrite had **already inherited it**, not through ported logic (there was none) but
through a type that offered nowhere to put the third answer. Whoever eventually writes the
enumeration will pick one of the two variants because those are the two that exist, and the
defect will be reproduced **by omission rather than by decision**.

The mechanism is that **data shape gets ported before behaviour**, usually early, usually by
someone modelling "what the output looks like" rather than "what we can know". Output shape
is exactly where an implicit conflation is invisible: the original's two spellings look like
the complete set precisely *because* the original never had a third.

**So a re-derivation discipline that governs logic and not types is not a re-derivation
discipline.** Every enum ported from a system under replacement deserves the question: *what
states did the original fail to distinguish, and does this type let us distinguish them?*
Ask it when the type is written, because after that the answer is load-bearing and the change
is breaking.

The three dispositions, and only the third is a defect: make the missing state
first-class; **or** keep the collapse deliberately, in a written row that says so in those
words; **or** leave the type as it is because nobody looked — which reads identically to the
second from inside the code, and identically to the first from inside the intent.

### A gate that GENERATES its input validates its own output, not the commit

A tree gate emitted a derived index into the tree it audited, then reported over that index.
Running it changed a **committed** file: two counts moved, because the last arm's artifacts
had landed after the index was last generated. The commit had shipped a stale index and the
gate had passed anyway — not by oversight but **structurally**, since the stale bytes are
overwritten before anything reads them.

The consequence is narrow and worth stating precisely: the numbers such a gate reports are
true of the **working tree**, and are not evidence about the **commit**. Where the underlying
artifacts carry their own independent verification (checksums, presence-at-HEAD), the
conclusion survives; what was weaker than advertised is the mechanism.

The separation to insist on: **a gate reads; a generator writes; they are different programs
run at different times.** If one command must do both, it has to compare the regenerated
artifact against the committed one and fail on difference — otherwise "regenerate" silently
means "repair", and a repair inside a check is indistinguishable from a pass.

### A protocol that was just hard-won is the one most likely to be OVER-APPLIED

Six gate rounds hardened an evidence protocol — pre-registration, blindness, typed barriers,
red-proofs. It was the correct instrument for contested claims where an arm could pre-judge
itself, and it cost a great deal to get right.

The next task was a source-reading problem: read a frozen script, cite it accurately, mark
what source alone cannot settle. Its author's own account of what they would have done
absent instruction: *"I would have carried the heavy shape over by default … after six rounds
the protocol had stopped feeling like a cost and started feeling like the standard."*

That is the mechanism, and it is not laziness — it is the opposite. **Effort spent hardening
a protocol converts into belief that the protocol is the baseline**, and the belief is
strongest immediately after the effort. Nobody over-applies a process they got cheaply.

So proportionate rigor has to be decided **per task, out loud, by someone who is not holding
the hard-won instrument** — and the answer "less rigor here" needs stating as a judgment
about fit, never as a discount. The tell that it is fit rather than fatigue: you can name
what the heavy protocol protects against, and say why that failure cannot occur here.

### When the source is FROZEN, evidence has no expiry — so capture order must follow BUILD order

A rewrite froze its predecessor at a known commit and began capturing behavioural evidence
against it. Six consecutive gate rounds went into one evidence batch — rigorously, with real
defects found every round — for a subsystem belonging to **phase four**, while the product
sat in **phase one** unable to perform its only implemented command, blocked on two surfaces
no ratified row defined.

The fact that settles the priority is the freeze itself. **Evidence captured against a frozen
source cannot decay**: the phase-four batch would be exactly as capturable, byte for byte, at
phase four as it was that night, because the thing it measures cannot move. Deferring it cost
nothing. Doing it first cost the phase actually in progress.

So a frozen oracle removes the only honest reason to capture early. What remains is:
**does the next item unblock the next build?** Anything else is queue position chosen by
interest, and interest reliably selects the subsystem that is most intricate rather than the
one that is most in the way.

The failure is hard to see from inside because **every local signal is green** — the work is
careful, the gates find real defects, each round is a genuine improvement. Nothing in a
well-run evidence loop reports that it is three phases ahead of the build. Only the question
*what is currently blocked, and does this unblock it?* surfaces it, and that question has to
be asked deliberately, on a schedule, by someone whose job is the queue and not the batch.

**Corollary for reviewers:** a team capturing in interest order will always feel productive
and will never unblock anything. Sustained high-quality output against the wrong queue
position is the most expensive failure mode available to a competent team, precisely because
none of the usual quality signals fire.

### Value-blindness is a DESIGN forcing function, not only anti-contamination hygiene

A gate found that a design document cited the wrong source for a product claim. The correct
source was deliberately **withheld** from the document's author, under the rule that a source
anchor may travel but the relation it proves may not — the expectation being that they would
re-derive the right anchor from their own reading.

They did something better. Unable to look the answer up, they asked why the *document* was
asserting it at all — and **withdrew the claim** instead of re-anchoring it. Where the
behaviour lives is a product outcome; the arm exists to capture it; so the design should
assert nothing about it. The author's own note: *"that is where it should have been from the
start, and is also why I did not go looking for the real location after you withheld it."*

The general form: **a design document that asserts a product outcome has pre-judged the
experiment meant to establish it.** That defect is invisible while the author can look the
outcome up, because a correct-looking assertion with a resolving citation reads as diligence.
Take the lookup away and the only honest move left is to stop asserting.

So the blindness rule earns its cost twice: it stops a known answer from contaminating a
result, **and** it exposes the places where a document was quietly doing the arm's job.

### A citation that RESOLVES is the most dangerous kind of wrong

The bad anchor above was not a broken link. It pointed at a real file, a real line, in the
right repository, at the pinned revision — and the line was the **Telegram outbound drain**,
a different subsystem entirely from the watchdog claim it was cited for.

Every mechanical check in the chain reported green, because every mechanical check asks
*does this pin resolve*, and none asks *does the thing at the other end say what the sentence
claims it says*. A dangling reference announces itself on the first run. **A resolving
reference to the wrong thing survives every automated pass and is only caught by someone
reading both ends** — which is why anchor-resolution tooling must never be described as
citation verification.

### An instrument must be present when the fixture is BUILT, not only when it is READ

A hooked copy of the product was used to observe a mid-write boundary. The hook was
invoked correctly, against a real fixture, and the run produced a clean-looking capture.
It was measuring nothing.

**ae's generated helpers are written BY the launching binary.** The fixture had been built
by the *unhooked* copy, so the `_lib` inside it carried no hook — and no amount of invoking
the hooked binary afterwards could put one there. The barrier reported `mark_seen=no`, and
the case went on to record a reader observing the **completed** write: a state at no
boundary at all. **The previous run had published exactly that.**

This is the plausible failure mode in its most expensive form: you get a reading, the
reading has the shape of a barrier observation, and it is a completed write wearing a
barrier's label. Nothing about it looks empty.

So for any system that **generates its own surfaces**, ask where the instrumented artifact
comes from, not just where the instrument is pointed. An instrument that must exist at
construction time cannot be added at observation time.

**The same hazard applies to the STIMULUS, not just the instrument — and there it is worse.**
A control run patches the code to break something, expecting the guard to go red. If the patch
silently fails to apply — an anchor reflowed by a formatter, a moved line, a renamed symbol —
the suite stays green, and **a control that never applied is indistinguishable from a control
that applied and was not caught.** The conclusion it pushes you toward is that your guard is
weak, so you go strengthen a guard that was already fine: wasted work you would never discover
was wasted, because the "evidence" for it was a green run that measured nothing.

So **assert the perturbation landed before interpreting the response.** Confirm the anchor
matched, the mutant compiled, the fixture wrote the bytes — then read the result.

**Landing is still not enough: a patch can apply cleanly and change nothing.** A control meant
to simulate a fallback edited only the *read* side of a lookup — and the value it read was never
in the answer set, so the mutation was **semantically inert**. It applied, it compiled, the
anchor assertion passed, and the suite stayed green, which again reads as a weak test. An anchor
check proves the bytes changed; it says nothing about whether the *behaviour* did. So a control
owes a demonstration that it **alters an observable** — the patched build produces a different
result on some input — before its failure to redden anything is treated as a finding about your
tests. Noticing that
output looks "suspiciously empty" is luck; the assertion is not.

**And your own success message is not evidence of success.** A reviewer invoked a delivery
helper and printed a confirmation line after it. The helper died on a shell quoting error — an
apostrophe inside the quoted argument — and delivered nothing; the confirmation printed anyway,
because it was a separate statement that never depended on the outcome. Only reading the exit
code showed the review had not been sent. **A status line you write yourself reports that you
reached the line, never that the thing worked.** Chain it (`cmd && echo ok`) or check the
status; an unconditional confirmation is a claim, not an observation.

**And a control run must be COMPLETE, not first-hit.** The same project hit this immediately
after: a test runner's default fail-fast cancelled the remaining tests after two failures, so
the control everyone cared about **never executed** — and "did not appear in the failures" reads
exactly like "was not reddened by the break". An early-exit default silently converts *not run*
into *not affected*. Disable it for any run whose purpose is to enumerate what a change breaks.

And the structural remedy is the one that generalises: **an unarmed barrier is not a null
result — it invalidates everything captured after it.** The case now records INCONCLUSIVE
and *stops*, because a run that continues past an unarmed barrier does not produce less
evidence, it produces confident wrong evidence.

### When the instrument necessarily perturbs, PROVE the difference set — do not assert it

The same arm needed to show its hooked copy was otherwise equivalent to the frozen one, and
reached for byte-identity. Byte-identity was the **wrong bar**: the hook writes itself into
every generated `_lib`, so `_lib` and the two files quoting it *must* differ. A bar the
instrument cannot clear is not a strict bar, it is one that gets waived.

The repair is not a looser assertion — it is a **proof obligation**. Equivalence became the
enumerated form (identical except files whose only difference is the hook's own bytes), and
the arm **diffs each differing file and fails on any non-hook line**, rather than asserting
the difference set is what it expects. It also proved the comparator can report red, against
a copy with an altered version string.

**A known, bounded perturbation is admissible; an unexamined one is not — and the boundary
between them is whether the run measures the difference or merely expects it.**

### An overstated claim is the defect — and it has TWO repairs

The bound a verification project needs, and the reason one can otherwise grow forever.

A checker was rebuilt eight times across successive gates. Every round found real blindness,
every round added checks, and the tool's scope had no ceiling — because every finding was
being read as **"a check is missing"**, which has exactly one repair.

It is not the only repair. The defect is not the missing check; **the defect is the gap
between what the document CLAIMS and what the tool ENFORCES**, and a gap closes from either
side:

1. **build the check**, or
2. **narrow the claim** to what is actually enforced.

**A document that says exactly what it enforces is correct even if it enforces little. One
that says more than it enforces is the thing that keeps getting found.** Reaching only for
repair (1) guarantees unbounded growth, and the growth is invisible because each individual
addition is justified.

The operating rule that follows: **build a check when its class has shipped twice; otherwise
narrow the claim.** Twice, because one occurrence is an incident and two is a pattern, and
because a reviewer offering *"or narrow the claim"* as an alternative — as good reviewers do
— is telling you which repair they consider proportionate.

### A narrowing needs its own red-proof for what it now lets through

Precision and recall move in opposite directions and **nothing in an edit records which way
it went.** A checker's author added a lookbehind to stop a false positive; that narrowing is
exactly what later let a real violation through — same predicate, same file, twenty minutes
apart, with nothing connecting the two edits. **A fix for a false positive became a false
negative, untracked.**

So when a check is tightened to stop noise, the obligation is not only to show the noise
stopped: **prove what it still catches.** A narrowing without a red-proof of its surviving
coverage is a silent widening of the hole.

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
8. **Pre-registration freezes by PROVENANCE, not by path.** Hash every
   harness-supplied executable and input — scripts, libs, hooks, shims, fixture builders,
   generators, the frozen subject, the harness tree. Do **not** hash what the product
   writes during the run: those are the named subject effects the arm exists to observe.
   Getting this wrong is possible in both directions, and both were found in successive
   review rounds of one design — **script-only** hashing is too narrow (an unchanged script
   can source a changed harness), **whole-closure** hashing is too broad (it refuses an arm
   for the product doing the thing being measured). A **dual-provenance** path — a helper
   the harness planted and the product then rewrote — is **two artifacts**: registered
   planted bytes and captured product-produced bytes. **Never a silently updated baseline**,
   which is the shape that makes the whole registration vacuous.
9. **Generate, then paste; derive, never hand-copy.** And the clause that completes it:
   **MEASURE, READ THE OUTPUT, THEN ASSERT — never measure and assert in the same action.**
   A worker ran a readiness check and sent the claim in the same breath, so the claim could
   not have been derived from the output; it was written before the output existed. The
   message contained real, correct hashes sitting beside a sentence one of them
   contradicted. **A generated number pasted next to an unchecked claim is worse than no
   number, because the number lends the claim credibility it has not earned.** Measuring
   and asserting simultaneously defeats the generate-then-paste rule while appearing to
   follow it.
10. **Verifying a conclusion independently does not validate the reasoning that reached it.**
   In the same incident a seat re-derived the conclusion from scratch, found it correct,
   and reported the claim verified — because the underlying FACT was right. The defect was
   in the claimant's evidence chain (they had searched the wrong tree), which no
   independent check of the conclusion can surface. **Only the claimant can catch a wrong
   path to a right answer**, which is why self-correction still matters after review
   passes, and why a review that agrees is not a review that checked the same thing. A table you generate and a reviewer
   reads is still your word. A table the reviewer can regenerate is not. The same applies
   one layer down to any document derived from another.

### A closed register is only as closed as its most open cell

An exclusion register — the list of differences a parity gate is allowed to forgive —
is worth exactly what its weakest row is worth, and the guards protecting it are
structurally unable to say so.

The exhibit: a phase-4 gate carried a six-row register with closure enforced from seven
directions. Only the register may exclude; a runner may not declare a choice; compare
exactly and subtract only the exact named loci; a choice never exempts a row or a whole
stream; the row's still-required facts stay asserted even where bytes are excluded;
**fail on an exclusion wider than its registered locus**; and no unregistered phrase
expands the set. Five rows were airtight. The sixth excluded
`Headers, columns, widths, colour, whitespace, and other layout bytes`.

**The guard measures width against the registered text, so an unbounded registration is
never exceeded — it is inhabited.** A runner who calls a newly-differing byte "layout"
has not gone wider than the locus; they are inside it. The one clause written to catch
this catches every row but the one that needs it.

The global default does not reach it either. "Any other difference fails" operates on
the residue *after* subtraction, and an open subtraction lets the runner decide what the
residue contains. Nor does the positive counterpart: the row's still-required list is an
enumeration, so anything in neither list — a footer, a count, a trailing marker — falls
through both sides at once. A wrong count in a summary line is not a semantic row, not a
field value, and plausibly "layout bytes."

**The sharp part is why it survived.** The same author, in the same artifact, in the same
pass, had already found and removed two escapes of this exact class — `except
runtime-variable fields` and `any other open choice`. Both *read* as exceptions, and an
exception announces that it is one. `and other layout bytes` reads as specificity: four
concrete nouns stand in front of it and lend it their precision. **An example list
survives review that an exception clause would not**, and it is camouflaged by the
concrete items preceding it rather than despite them.

The test that finds it: for every exclusion, ask **who decides whether a new byte falls
inside this locus.** If the answer is the runner rather than the text, the cell is open
however specific its prefix looks. Trailing `and other …`, `such as`, `etc.`, and a
bare `…` are the spellings; the property is the delegated decision, not the phrase.

Repair by mechanism, not by taxonomy: delete the tail, keep the named categories, and
let the global default own the residue. The row then closes the same way its five
siblings do — because nothing outside the enumeration was ever subtracted — rather than
by trusting whoever runs it to classify honestly.

Corollary for self-review: the two escapes the author caught were exception-shaped and
the one they missed was example-shaped. Self-review reliably catches the form that
declares itself. Budget a second seat for the form that does not.

**And the half that review missed: a register with no open cell can still be incomplete.**
The reviewer who found the open cell audited all twelve policy cells for over-width and
reported the register sound. A second seat then found two ratified open choices that were
never registered at all. Both readings were of the same artifact; only one of the two
questions had been asked.

A set has two independent failure modes and checking one says nothing about the other:
a member too permissive, and a member absent. **Auditing the cells is not auditing the
set.** The absent member is the harder one because there is nothing on the page to read
critically — closure review is driven by what is present, so it is structurally blind to
what is not. The check that finds it does not live in the register at all: enumerate the
choices the upstream criteria RATIFIED, then confirm each appears. Completeness is only
provable against the source that authorised the entries, never against the entries.

Same shape as verifying additions by presence and removals by absence: the direction you
do not check is where the defect sits.

### An exclusion whose SCOPE the subject controls is one the subject grants itself

A parity register's rows each carry a scope — the condition under which the exclusion
applies. Two of them read, in effect, *an invocation whose `inventory_complete` fact is
false*.

`inventory_complete` is **the subject's own output**. So a successor that emits `false`
has selected those exclusions into applicability for itself, and the gate forgives
exactly the invocations the product nominated. Worse at the margin: where that value is
UNSCORABLE, the exclusion still applies, on a premise nothing established.

**This is "a gate that generates its input" relocated from a FIXTURE to a PREDICATE** —
and that relocation is why the rule already being written down did not prevent it. The
reviewer who missed it had authored the fixture-shaped version of the rule in this very
document. A defect that changes part of speech stops matching the pattern you memorised,
because you memorised the noun it usually wears. The mechanism is identical; only the
grammatical slot moved.

The check: for every conditional anywhere in a gate — exclusion scopes, skip conditions,
applicability guards, any *only when* clause — ask **whose output decides this
condition.** If the answer is the subject's, the subject is grading itself, however
rigorous the rest of the row is. A scope is not metadata; it is an assertion, and it
needs the same provenance discipline as an obligation.

Repair: establish the predicate from **harness-side facts** — what was planted, what the
manifest says was there — never from what the product printed. The condition and the
thing being judged must not share an author.

### A self-diagnosed failure mode becomes the thing you check, so the next miss comes from elsewhere

A lead had missed the same way twice: verified a type's path *vocabulary* and reported an
address boundary; substring-matched a phase key and counted the wrong population. Both
spelling-shaped. Diagnosing that honestly, they asked a second seat whether the findings
they had just missed were that class again, planning to write it up as a standing personal
failure mode.

**None of the three was.** They were set-completeness, predicate provenance, and a
case-versus-invocation grain mismatch — not one vocabulary error among them.

The mechanism is worth more than the diagnosis: **naming a failure mode converts it into
an explicit check, and an explicit check is the one place you are now strong.** The
residual risk does not stay put; it migrates to whatever you are not currently watching.
A personal error log is genuinely useful for *repairing* a class and actively misleading
for *predicting* the next one. Treat a self-diagnosis as a fix, never as a forecast — and
never let it narrow the search, because a hypothesis about your own blindness is still a
hypothesis about where to look.

**The other half of that result: of the three findings, two were reachable from the
artifact under review and one required reading a different SOURCE entirely** — the corpus
specimen rather than the gate prose. Two seats given identical inputs would have missed
it however carefully they read. Independence has to cover **inputs**, not only judgment:
schedule the second seat to read from a different starting point, or it is a second pass
rather than a second perspective.

### Forbid the OPERATION, not the POSSESSION — and beware that only one of them is enforceable

A layer was supposed not to reach back for state it had already been handed. The rule
written was **"presentation carries no address."** It was verified by scanning the input
type for path-typed fields, found clean, and reported as a structural boundary.

It was false. `work_dir` is a payload field contractually and an address operationally, and
the forbidden address composes from `work_dir` + `.ae` + the session name — no new field,
no path type, no spelling any scan enumerates. **Possession is compositional**: a
capability assembled from individually legitimate parts is a capability held, and an
inventory of what a type *holds* is structurally unable to see it.

The rule that replaced it names the operation: **"presentation must not recompute source
identities from paths."** That is better on three counts. It is checkable at the site
where the harm would occur rather than at a declaration site. It needs no definition of
what counts as an address, which is the enumeration that sank the first version. And it
survives new payload fields — the next field that also happens to compose an address does
not reopen it, because nothing turned on which fields exist.

**The general move:** when you are about to forbid *holding* X, ask what you are afraid
will be *done* with X, and forbid that. The possession rule is always a proxy for a use
you fear; write down the use. This is the same distinction as a capability boundary versus
a name list — a name list enumerates spellings of the thing held, a capability boundary
constrains what can be done with it.

**And the trap, which is the part worth remembering.** Possession rules are seductive
*because the type system can enforce them.* You can make a field unspellable; you usually
cannot make an operation unspellable. So the enforceable rule and the correct rule come
apart, and the enforceable one arrives wearing a structural proof — which is exactly the
evidence that stops further questioning. **The confidence came from the enforcement
mechanism, not from the rule being right.** When a boundary is provable by construction,
check separately that the thing proved is the thing you needed; those are two claims and
the proof only speaks to one.

Corollary for the weaker rule: a use-shaped rule usually cannot be made structural, so it
needs a real test and an opposed control rather than a type. That is a genuine cost. Pay
it — a defended rule that names the concern beats a structural rule that names a proxy.

### A scoped verdict copied without its scope is a STRONGER claim than the source made

A gate file records its outcome like this:

```
## Status against `f6c8f0a` (phase 1 as landed)

**DOES NOT PASS.** …
```

A seat reconciling a stale plan against that file lifted the verdict and left the heading
behind. `DOES NOT PASS` became the plan's statement about the present, when the source
said it about one commit. The companion case was worse: a gate held unread before its
implementation existed carried `NOT RUN — phase 2 does not exist`, written truthfully at
the time; copied forward, the plan asserted phase 2 did not exist while eighteen phase-2
tests, a bumped schema constant and four source files constructing the new variant sat in
the tree.

**Scope lives in headings; verdicts live in bodies; copying is body-shaped.** That is why
the scope is structurally the part that falls off. Nobody decides to drop it.

**The laundering is the real damage.** The derived document does not merely repeat a stale
claim — it repeats it *with a citation to a real line*. That makes it more credible than
the vague sentence it replaced and much harder to challenge, because checking the citation
CONFIRMS it. A reader who verifies the reference finds exactly the quoted words and stops.
Propagating staleness with a citation is worse than propagating it without one; see also
a citation that resolves being the most dangerous kind of wrong.

The rule when lifting a verdict: **look UP for a scoping heading before you copy.** If the
scope cannot travel with the verdict, do not lift the verdict — cite the location and let
the reader go to the only place the status can be current.

**And the dispatch half, which is the reusable part: authority is per-QUESTION, not
per-artifact.** The instruction was "derive every status and count from the gate files."
Those files are genuinely authoritative for criteria counts — a count is scope-free, you
can number the list. They are *not* authoritative for current status, because every status
in them is bound to the commit it was measured against. One file, two questions, two
different answers about whether it may be trusted. A worker told to treat an artifact as
authoritative cannot make that split if the person dispatching them did not. Name the
question the source is authoritative FOR, or expect the scope-free answers and the
scope-bound ones to come back with equal confidence.

### Wiring an INERT SEAM makes every fixture that depended on its inertness contingent

ae had no tmux transport: `Discovery` failed every query by construction. A test fixture
planted a session recording `tmux_server=ae-test` and asserted the row rendered `unknown`.
That fixture's correctness depends on `ae-test` **not existing** — and while the seam was
inert, that premise was FREE. No query could run, so whether any such server existed on
the machine could not matter.

Wiring the transport revoked the grant. The same fixture, unchanged, now issues a real
query. A developer with `tmux -L ae-test` running anywhere on the box turns the failed
query into a **successful** one that legitimately reports the session absent — `stopped`,
a red test, and a failure with nothing to do with the product. **Nothing in the fixture
changed; the cost of its precondition did.**

**The general shape: an inert seam grants a free precondition to every fixture downstream
of it, and wiring the seam revokes that grant silently, everywhere, at once.** The grant
is invisible while it holds, because a precondition that cannot fail is indistinguishable
from one that is not there.

**Detection does not follow the diff.** The change shows you the seam; it does not show
you what was leaning on it, and most of the exposed fixtures are files the change never
opens. Grep instead for **what the seam used to make free** — here, every fixture naming a
tmux server or socket, then filter to those whose premise is that the named thing is
ABSENT. A fixture depending on a server it *creates* is safe; a fixture depending on a
server nobody created is the exposed shape.

Note the discovery mode, because it is the common one: this instance was found by
**proximity** — while fixing a comment three lines away that the same change had made
false — not by looking. Proximity finds the first one. It never finds the set, and the
feeling of having found it is the same either way.

**The load-bearing move was declaring the boundary.** The author wrote: *a gap in my
landing, not a completed sweep — I only swept the file I was already in.* That sentence is
worth more than the fix. **An unswept file you have NAMED is a task; an unswept file you
have not named is a defect that will later be attributed to something else** — a flaky
test, a bad machine, an unrelated commit. Only the author knows the scope of what they
actually checked, so a check reported without its scope inherits the same defect as a
verdict reported without its scope: it reads stronger than it is.

### Every gate property fails in TWO directions; review instinct is trained on one

Three findings in one night of gate review, each the same shape, each found only because
someone happened to look the unusual way:

**Forgiving what must be compared / being unable to forgive what was ratified.** A fixed
comparison projection was reviewed on the explicit question "does it invent or over-exempt
semantics?" It did neither. It UNDER-admitted: one ratified register row — excluding a
timestamp value that cannot match a replay — had no expressible mechanism, because the
projection enumerated removal of member *order* and *presence* and never member *value*.
All 401 machine-output rows would have failed, and the projection itself barred the runner
from improvising. The question named one direction; the defect was in the other.

**A member too permissive / a member absent.** An exclusion register was audited cell by
cell for over-wide loci, one was found and repaired, and the register was reported sound.
A second seat then found two ratified choices never registered at all. Auditing the cells
is not auditing the set.

**Forgiving a wrong implementation / rejecting a correct one.** A reviewer (me) found an
ambiguous role definition and proposed tightening it. The tightening would have converted a
ratified open choice into a required byte spelling — rejecting a correct implementation for
exercising a choice the contract grants. The gate's own closing constraint forbade exactly
that. **The fix was prohibited by the artifact it was filed against**, and it looked safe
because tightening always looks safe.

**The pattern: reviewers are trained to hunt leniency.** Every instinct — be adversarial,
assume the author is wrong, look for the hole — points at the forgiving direction. The
strict direction produces failures that *look like the gate working*: a red run, a refused
row, an implementation told no. Nobody investigates a gate for being too harsh, because
harshness wears the costume of rigor.

Practical form: for each property a gate asserts, write both failure sentences before
reviewing it. *This is wrong if it lets X through* and *this is wrong if it rejects Y.*
If the second sentence is hard to write, that is the direction you are not checking. When a
brief names one direction — as "does it over-exempt?" did — treat that as a description of
where the author already looked, not as the scope of the review.

### VERIFY-THEN-COMMIT is not atomic when another agent shares the tree

A reviewer was told to run the author's mutation attacks — deliberately breaking the code
to confirm the suite goes red. Nobody said WHERE. They worked in the live checkout.

The author, in that same checkout, ran the gate green, checked `git status`, ran `git add`,
and committed. Between the green run and the `git add`, the reviewer applied a mutation.
The commit shipped `Err(QueryFailed)` — the mutant — as the real body of the transport, and
HEAD did not build. Both agents did exactly what they were asked.

**Two mechanisms, and the first one is the trap:**

- **`git status` is a NAME oracle, not a CONTENT oracle.** `?? src/transport.rs` reports
  that a file is untracked. It reports nothing whatever about its bytes. Reading a status
  listing as verification looks precisely like diligence, which is why it survives.
- **Any verification establishes a fact about the tree at time T; the commit captures the
  tree at T+n.** Single-writer, `n` is harmless and the habit is safe. Add a concurrent
  writer and it is an unlocked race that git will never flag.

Narrowing the window is not the fix — the window is the defect. The repair that holds is
**fail-closed and in one command**: snapshot hashes of every path in the same command that
runs the gate, chain the commit behind a `shasum -c` of that snapshot, and then — the step
whose absence caused the incident — **extract the paths back out of HEAD and re-hash them
against the same snapshot.** The first two check what was on disk when you typed. Only the
third checks what actually landed.

Dispatch half, which is the durable lesson: **never place a mutating agent and a committing
agent in one tree.** Mutation work belongs on a copy or in a dedicated worktree, and an
instruction to "run the mutations" is incomplete until it says where. The condition was
created by the dispatcher, not by either agent in it.

Related: "the tree is frozen" is never one agent's promise to keep — an agent controls only
its own edits. Treat a freeze as a coordination request, and re-measure anything measured
across one, because a stale green is indistinguishable from a fresh one.

### Disjoint findings come from disjoint POSITIONS — and pooling early destroys them

One slice, three reviewing seats, three findings, and **no set contained another**:

- the seat that had **written the fixture** found that its seven session names contained no
  prefix of another, so the fixture population was structurally incapable of exhibiting the
  bug being guarded against;
- the seat attacking **cold from the code**, with no knowledge of which cases the author had
  already considered, found that a signalled child read as success left the entire detector
  suite green;
- the seat that had **built the implementation** reached the ownership seam, because it knew
  where it had made a judgement call rather than a derivation.

**The disjointness is structural, not lucky.** Each seat found what its position let it see:
authorship of the fixture reveals what the fixture excludes; cold attack reveals what
familiarity has stopped questioning; authorship of the implementation reveals which lines
are decisions. None of those vantage points is reachable from the others, however carefully
the occupant reads.

That is an argument for **diverse seats**, not for *more* review. A second pass from the
same position mostly re-derives the first.

**And the caveat that comes with the citation.** The implementer's attack list — the most
useful artifact any of them produced — was also what nearly destroyed the result, because
handing it to the cold seat first would have converted an independent pass into an audit of
that list. The author cannot assess their own list's *completeness*; that is precisely the
property another position exists to supply. So: **let each position be USED before its
knowledge is pooled.** Share the coverage *boundary* freely — a fact about what someone did
— and withhold the *conclusions* until the other seat has reached its own. Cold first,
calibrate second.

### A decision's PREMISES can be invalidated by a later ruling that never mentions the decision

A ratification-priority document deferred one contract row. The recorded rationale included
`NO-P1-FIXTURE-DEPENDENCY` — nothing in phase 1 needs it, so it can wait.

Later, on the same day, a new P1 row was ratified that **cites the deferred row
normatively.** The deferral's stated justification became false at that moment. Nothing
announced it. The ratification document was never edited, because from its author's seat
nothing about it had changed.

**Neither document is wrong.** Each is internally consistent and would survive any careful
reading of itself. The contradiction exists only in the *relation between them*, which is
exactly why it survived both being read closely — and why it was found by someone doing
neither, reading the new row in order to implement it.

**The general mechanism: a decision records a conclusion and the premises that justified
it, and then nothing ever watches the premises.** Conclusions are load-bearing and get
cited; premises are prose in a rationale column. When a later change falsifies one, the
conclusion persists — and it reads as *settled*, because a decision with a written
rationale is the most convincing kind of artifact there is.

This is the same shape as a scoped verdict copied without its scope, one level up: there,
a claim outlived its scope; here, a decision outlived its reason.

**The mechanism that catches it is a freshness relation, not more careful reading.** An
obligation table that stores its contract blob hash and refuses to run when the contract
moves is the working example. A deferral list, an exemption list, or a known-gaps register
needs the same thing pointed at whatever its rationales depend on: *these deferrals were
justified against ratification state X; if ratification has moved, re-derive.* Without it,
every rationale is a claim about a world that is no longer being checked.

Corollary for the reviewer: when you find one, **sweep for the class before fixing the
instance.** One invalidated premise is a bug; several mean the list itself is missing its
freshness relation, and repairing the instances leaves the generator running.

**The same family, running the other direction: a deferral whose CONDITION was satisfied.**
Two accepted criteria carried arms deferred in terms — *this is not an arm until the real
transport supplies a product-valid observation route.* The transport then landed. Nothing in
either criterion changed, nothing announced it, and both gates still read as accepted;
but two arms that had been legitimately absent became legitimately required, and an
acceptance that was complete on Monday was incomplete on Tuesday without being edited.

So a conditional obligation fails in both directions — its premise can be falsified, and its
condition can be met — and neither event touches the document that records it. **A deferral
is a promise about the future written in the present tense.** The repair is the same
relation: recheck conditional predecessor evidence against the pinned *successor commit*
rather than freezing it at the gate's authoring date. Note the cost is real and worth
paying: firing those arms means a phase that looked finished is not. The alternative was an
acceptance that had quietly stopped meaning what it said.

### A mechanism with ONE reachable output cannot be tested — and it matters which one

A slice was scoped to add pane-to-agent association. Reading the contract before building
it, the implementer found that the only verdict the mechanism could ever reach was `dead`:
the row granting `alive` depended on a predicate that occurs exactly once in the whole
contract — inside the row that depends on it — and is defined nowhere.

So the feature could produce one value. **No test could show the association CORRECTLY
FINDS an agent's pane; only that it correctly fails to find one.** With a single reachable
output, *behaving correctly* and *returning a constant* are observationally identical, and
no amount of test-writing separates them.

The reviewer's sharpening is the part worth keeping: it is not merely untestable in the
useful direction, **it is unfalsifiable in the harmful one.** Every wrong answer such a
mechanism can produce is a false `dead` — the precise failure the whole effort exists to
prevent — and nothing available to it could ever contradict that. It is a monotonic source
of exactly one error, with no instrument that can see the error.

**The asymmetry decides the ruling.** A mechanism that can only answer `unknown` is useless
but harmless: its single value is the safe one. A mechanism that can only answer `dead` is
useless *and* dangerous, because the single value is the damaging one. Same structural
defect, opposite disposition — the first can ship and wait, the second must not.

Before building, ask **what values can this reach?** If the answer is one, no test you write
afterwards will help, and the fix is never more tests — it is giving the mechanism a second
reachable value. Here that meant stopping the slice and ratifying a live predicate first,
so the association could be proven to succeed and not merely to fail.

Corollary: this is the one-sided-arm problem scaled from an assertion to a whole feature.
An arm with one direction is a weak test; a *feature* with one direction is not a testable
thing at all.

### Correctness that lives in the RELATION between two functions is invisible to both

A roster parser validated that an agent's alias and name were non-empty and never checked
the **slot**, so a hand-edited meta line yields a roster entry whose slot is the empty
string. Separately, a pane reader normalised an empty marker to `None`, so no observed pane
ever carries `Some("")`. Composed, the empty slot matches nothing — which is the correct
answer.

**Neither function is wrong. Neither holds the invariant. Nothing asserts it.** The safety
is real and it is entirely emergent.

The realistic attack is not malice, it is tidying. Deleting the reader's
`.filter(|slot| !slot.is_empty())` looks like removing defensive noise — and it does, right
up until you remember the measured platform fact underneath it: **tmux reports "unset" and
"set to the empty string" identically.** Forget that and the filter is obviously redundant.
Remove it and every unmarked pane carries `Some("")`, an empty roster slot matches *every*
unmarked pane, and an agent's health is read off somebody else's pane.

Two rules fall out:

- **A guard whose justification is a measured fact must carry that fact at the guard.**
  Otherwise the guard's own apparent obviousness is the argument for deleting it.
- **Name the test for the FACT, not the mechanism.** *An empty roster slot matches no pane*
  survives the deletion it guards against; *the empty-marker filter works* is read as a test
  **of** the filter and gets removed in the same commit by someone who believes they are
  cleaning up. The test has to outlive the implementation that currently makes it pass.

Detection is the hard part, because review reads functions one at a time and this class is
structurally invisible to that. The question that surfaces it: **what makes this safe, and
does that thing live in this function?** If the answer is "something else normalises it
first," the invariant is unowned.

### A migration's defect list is a REVIEW INSTRUMENT, not just a record

The finding above was not discovered by reading the new code. It was found by taking a
defect that had *just been filed against the frozen system* — a shell-set predicate missing
its empty-string case, so an unreadable value read as a positive result — and asking the
**successor's** reader the same question. It had a live one.

That inversion is the whole technique. A defect found in the thing you are replacing
arrives looking like a fact **about the incumbent**: evidence for the migration, a line in
an issue tracker, a reason the rewrite is justified. It is also, and more usefully, **a
question you have not yet asked of the replacement.**

The generalisation step is small and mechanical: strip the instance down to its class —
*missing empty case*, *failure treated as absence*, *identity keyed on a display field* —
then grep the successor for where that class could live. Rewrites are especially exposed
here, because the new implementation inherits the old one's *problem shape* even when it
shares none of its code, and the author is primed to see each incumbent defect as something
they have already escaped by construction.

So: every issue filed against the frozen system should end with a question, not a
conclusion. Not *bash got this wrong* — **does ours?**

### A test can NAME a fact without exercising it — two shapes, one cause

Both of these shipped in the same file, written by a careful author, in the area they
understood best.

**Shape one: the tautology.** A test called *a marker set to empty is not a usable identity*
asserted `interpret(payload) == interpret(payload)` — a function against itself. The fact
was real and independently confirmed by measurement, but the assertion pins nothing and
cannot fail for any implementation. It consumes a name and a line in the count while
providing no coverage.

**Shape two: the fixture that builds its own conclusion.** A test guarding a normalization
constructed the **post-normalization value by hand** — `pane(None)` — rather than feeding
raw bytes through the normalizer. Deleting the very filter the test existed to protect left
it **green**. The corrected version enters through the transformation
(`interpret_panes(true, "\nmain\n")`) and reddens as intended. Measured both ways under the
mutant, not argued.

**The rule for shape two: a test guarding a transformation must ENTER THROUGH the
transformation.** A fixture that builds the conclusion cannot observe the step that produces
it — the same defect as a gate that generates its own input, wearing test clothes.

**The shared cause is the useful part, and it is not carelessness.** The author's own
diagnosis: *knowing a fact well is the condition under which you stop checking whether you
asserted it.* Every other test in that file was written by asking **what mutation would
redden this.** The two defective ones were written by asking **what do I know.** Having just
measured the platform behaviour with `od`, a test that *mentioned* the fact read as a test
that *checked* it.

So expertise is the risk factor, not ignorance — which inverts the usual intuition about
where to look. The area an author understands best is where their tests are most likely to
restate knowledge instead of exercising code, because that is the only place they have
enough knowledge to restate.

Two practices fall out. Write every test from the mutation question rather than from the
fact, and when review rejects a draft, **keep the rejected version as a comment beside the
fix** — the wrong version looked right, and the next author will reach for it again unless
they can see why it fails.

### A STATED precondition is a hole; a MEASURED one is a closed bound

A locator had to identify a field inside a rendered row. Two ways to fool it were found, both
reachable in principle: a tab inside an upstream field shifts the offset, and a session row
can be made to imitate an agent row because the only discriminator is a two-space prefix.

The natural repair is to write the preconditions down — *assumes no tab in that field;
assumes no name begins with two spaces* — and ship. That is honest, and it is still a hole:
a commitment carrying unenforced preconditions hands every downstream consumer a defect they
did not agree to, and the gate that consumes it inherits a mis-comparison rather than a
failure.

The better repair costs almost nothing: **measure whether the precondition is violable in the
actual population.** The corpus is frozen; both questions are greppable. If no row anywhere
contains the offending shape, the same sentence stops being an assumption and becomes a
*measured bound for this run* — and if one does, you have found a real block instead of
documenting it.

**Same words, different epistemic status, decided by one grep.** When a finding's severity
turns on reachability, measuring reachability is almost always cheaper than the argument
about it — and unlike the argument, it terminates. Prefer it over both the confident dismissal
and the cautious caveat.

Two riders. Prefer a locator **robust by construction** over one **correct by precondition**
even when both measure clean — here, indexing from the END of the row meant extra fields
appear upstream of the target and the offset still lands, which kills the mechanism rather
than its current instances. And file the defect anyway: unreachable in *this* population is
not the same as fixed, and the next population is not frozen.

### A COMMITMENT is not a RATIFICATION — and the artifact must say which it is

The module being described declared, in its own comments, that its layout was **provisional
and unratified**: those bytes were a seat decision to be informed by parity evidence. The
parity gate *is* that evidence. So pinning the layout in a pre-registered manifest, before the
gate runs, pins the very thing the gate was supposed to inform.

The resolution is a distinction, not a compromise: the manifest states **what the successor
currently emits** (a commitment), not **what it must emit** (a ratification). A commitment
about present behaviour cannot pre-empt a later decision about required behaviour, so the
circularity dissolves.

**But the artifact's FORM argues against its status.** A pinned, hash-identified byte map
*looks* ratified — that is what pinned byte maps usually are. A reader six months out finds it,
reads it as settled layout, and the gate has quietly decided the question it existed to
inform. The distinction lived in a message between two agents and nowhere in the file.

So when a pre-registration artifact records current behaviour rather than required behaviour,
**say so inside the artifact**, in those terms. The rule generalises past manifests: any
document whose form implies more authority than its content claims must disclaim the
difference where it will be read, not where it was decided.

### A reservation covers MECHANISMS that presuppose an answer, not just the code

A question was held open: does a criterion's flat mention of *exit status* mean rc is open on
every surface, or only where the register leaves it open? Three test assertions turned on it.
The implementer was told explicitly not to touch them pending a ruling.

They removed two — the conservative direction, and probably correct — and then **added a
self-scan that forbids the shape.** That is the part that needed undoing.

**Removing a test and adding a guard are not the same act.** A deleted assertion is
recoverable in one line if the ruling goes the other way. A guard that forbids the correct
shape will *fight* the fix: it fires on the right answer, and it fires persuasively, because a
mechanism forbidding something reads as settled policy to whoever meets it. Nobody re-derives
a guard's justification; they route around it or assume it encodes a decision someone made.

So a reservation is not only about the lines the answer would change. It covers **anything
that presupposes an answer** — a guard, a lint, a schema, a naming convention, a helper that
makes one branch easy and the other awkward. Those are decisions in mechanism form, and they
are harder to reverse than the decision itself, because they stop looking like decisions.

Two riders. If a guard must land before its question resolves, make it say so **at the guard**
— name the open question, cite both readings, state which one it implements and that it is
provisional. And when correcting this, be proportionate: the deletions here were defensible on
the merits and only the mechanism needed changing. *Acting inside a reservation* and *reaching
the wrong answer* are different failures and warrant different responses; conflating them
teaches people to defend their reasoning instead of respecting the hold.

### A discovery that changes a ratified row must GRADUATE into an assertion — or the row rests on a note

A probe observing real tmux panes noticed something its author had not gone looking for: an
exited pane still reports a plausible foreground command. That discovery was strong enough to
change a **ratified contract row** — the live predicate gained a `pane_dead` conjunct.

Then the probe shipped with the discovery as a `note()` line. Nothing asserted it. The
predicate under test read only the command field, so a `pane_dead=1` row read as alive, the
probe passed, and the README **cited that pass as proof of the very conjunct nothing was
checking.** The strongest finding in the run was the one assertion never written — and its
absence was invisible because everything around it was green.

The mechanism: **discovery and assertion are different artifacts, and the pipeline from one
to the other has no natural forcing step.** A discovery lands in prose (a note, a comment, a
README sentence, a contract amendment) because that is where observations go. An assertion
has to be *written*, and by the time the row is amended the author's attention has moved to
the row — the probe already "found" the fact, so re-instrumenting it feels redundant. It is
the expertise trap one door down: the fact is so established in the author's head that a line
*mentioning* it reads as a line *checking* it.

The rule: **when an observation changes a contract, the observation's instrument changes in
the same commit** — the noticed fact becomes an asserted precondition or a verdict arm, and
the red-proof includes the pre-discovery predicate failing against it. A row amended on
evidence whose instrument still cannot see that evidence is a row resting on a memory.

Companion repair from the same incident, worth copying: the probe now exits **2 for FIXTURE
ABORT** and **1 for PRODUCT FAIL**, because three consecutive versions were broken in the
fixture rather than the subject, each presenting as a confident verdict about the subject. An
instrument that cannot say "I am broken" says "the subject is" instead — give the fixture its
own exit code, assert every precondition before reading any verdict, and have the red-proof
harness refuse to run (SEED DID NOT LAND) when the defect it injects is already what is live.

### ABSENT and MALFORMED are different defects — a gate that collapses them misdirects the reader

A freshness gate red-proofed its own STALE detection: the seed prepended text to the pinned
blob hash. The gate reported **FRESHNESS — "no contract_blob is recorded"** — because the
extracting regex demanded exactly 40 hex digits, so a corrupted pin matched nothing and read
as no pin at all.

"The file asserts no relation" and "the file asserts a relation you cannot parse" send the
investigator to **different places** — one to the file's author, one to whatever mangled it.
A gate that reports the wrong one is not weaker, it is *misdirecting*: the reader debugs the
absence that isn't there. Extraction failure needs its own outcome (MALFORMED), its own seed,
and its own red-proof, because the collapse is invisible until the day the wrong report costs
the diagnosis.

The general rule: **wherever a parser's "no match" feeds a verdict, ask which distinct
real-world states all map to that no-match** — and give each state the gate can be wrong
about its own name. This is the same family as exit-status-decides (empty output from a
failed query vs a truly empty answer) and rc=2 FIXTURE ABORT vs rc=1 FAIL: instruments must
be able to say *I am broken* and *the input is broken* separately from *the subject failed*.

Same sweep, same author, second instance of an adjacent class: a field parser read
`Bucket (\d)` on one line, and a row that wrapped "Bucket" and "2" across two lines reported
the bucket **absent**. A wrapped field read as a missing field is the empty-string-is-a-shell
defect one layer up — the parser's blind spot maps a present-but-differently-shaped input
onto the absence branch. Two instances in one week: when a value can legally arrive in more
than one shape, the parser needs a seed in every shape, or its "absent" arm silently
accumulates everything it never learned to read.

### Two name-enumerating validators, one day apart — and only the FAIL-CLOSED one told anyone

Two validators in the same evidence chain enumerated accepted names where they should have
validated a grammar, and both were exposed by the first new member of their set. A letter
gate matched `CRITICAL\(([A-D,]+)\)` — a seeded `C,E` fell OUT of its denominator and the
artifact **passed**. A batch checker matched a hardcoded alternation of capture-cluster
tags — the first genuinely new tag since the list was written was reported malformed and
the summary **failed**.

Same defect class. **Opposite failure directions, and the direction decides everything about
discovery.** The fail-open one is silent: a name outside the enumeration simply stops being
counted, and the check stays green until somebody thinks to seed the case — it was found by
an adversarial control, days after shipping. The fail-closed one is self-reporting: it
announced itself within minutes of the first legitimate new value appearing, loudly, with a
count of exactly the new lines. Nobody had to suspect it.

The known hierarchy stands — validate a grammar, not a list; adding the missing member
preserves the generator. But when a grammar is genuinely unavailable and an enumeration is
what you can have, **the direction is a choice, and fail-closed is the defensible one**: an
enumeration that REJECTS the unknown converts its own staleness into a visible finding at
the first new member, while one that EXCLUDES the unknown converts its staleness into a
silently narrowed population. You will not remember to re-audit the list; arrange for the
list to complain.

Corollary for reading counters: a fail-closed enumeration's false alarm (`MALFORMED=15` on a
ruled-legitimate taxonomy) is the *checker* behind the *taxonomy* — repair the checker,
never rename the legitimate values back inside the stale list to make the counter quiet.
The durable repair is the same in both cases: the name set stops being a list inside a
checker and becomes a derivation from the source that owns it.

### A key that is not a key — and the agreement it manufactures

Two seats independently derived the same population and diverged: 128 versus 88. The
adjudicator was warned, in advance, not to accept a cardinality match as a set match.
Then the trap turned out to be live **inside one of the artifacts**: the handover's key was
`(case, consumer, locus)` with the locus spelled `agents[].reason` — session-blind — so the
128 obligations collapsed to **88 distinct keys**. Had the join run on that address, 128
would have *presented* as 88 and **matched the other seat's 88 by cardinality while being a
different set**. The two derivations would have agreed for the wrong reason, and agreement
is where investigation stops.

The repair attempt then hit the same defect one field over: keying on `agent_ref` was also
insufficient — 64 triples named an agent appearing under more than one session in the same
digest, silently overwriting eight rows per sub-population. Both failures are one mechanism:
**an address that cannot tell two things apart is not a key, and everything joined on it
inherits the collapse silently** — the same class as a total over a parse that drops rows,
wearing different clothes.

Three rules fall out:

- **An address is a key only when its distinct-count equals the population it addresses.**
  Measure it (here: 128 obligations, 128 keys after correcting to
  `sessions[name].agents[ref].reason`); never assume it from the schema's shape.
- **A comparison exists only at one declared key.** Joining two artifacts keyed at different
  grains is not a comparison, however identical the columns look — verify BOTH sides
  disambiguate before diffing.
- **Matching cardinalities between independent derivations is the most dangerous form of
  agreement**, because it is exactly what a key collapse produces and exactly what ends the
  investigation. Two counts agreeing says almost nothing; two KEY SETS agreeing says
  everything the counts pretended to.

Note where it was caught: not by the warning — the warning was about the adjudication and
the defect was in the artifact — but by **building the handover**, which forced the key to
be materialized and its distinct-count measured. Preparing evidence for someone else's
scrutiny is itself an instrument; the act of making a thing handable is what made the
defect visible.

### A negative claim inherits the scope of the search that produced it — and quietly sheds it in the reporting

A derivation excluded 94 loci with the phrase **"unguessable by construction"** — the
owner-naming evidence, it said, could not exist. The evidence existed. It sat in the
producer template's event bytes, one join away, reachable from every case through the
`template=` line each case's own file records. The author had searched the case's CAPTURED
events, and only one action kind — and reported absence-in-the-place-searched as
absence-in-principle.

**An exhaustive-sounding negative is the most expensive thing to get wrong**, because it
closes the question for everyone downstream. "No carrier exists" ends the search; "I found
no carrier in the captured case events" invites exactly the second look that broke this one
open. The two claims cost the same to write, and only the author knows which one the search
actually supports.

The mechanism is scope-shedding at the report boundary: the SEARCH has a precise scope (one
file class, one action kind); the SENTENCE describing its result defaults to the unscoped
form, because that is how conclusions are naturally worded. Same family as a check reported
without its coverage boundary and a verdict copied without its scoping heading — the scope
is structurally the part that falls off, and with negatives it falls furthest, because a
positive claim carries its own witness while a negative claim carries only the author's
search.

Three notes from the incident worth keeping with the rule:

- **It was the third evidence-source defect in one day from a seat whose logic was sound
  every time.** A wrong source with correct logic is undetectable from inside the
  derivation — internally consistent, confidently wrong — and only a DIFFERENT source
  contradicts it. Budget for source-level review, not just logic-level.
- **The catch came from an adjudicator reading the EXCLUSION file**, not the result. The
  excluded set is where an over-scoped negative hides, because nobody audits what a
  derivation declined to claim.
- The repair is the honest smaller claim, written into the artifact: the re-derived
  exclusion file says *no carrier FOUND*, names what was searched, and thereby stays an
  open invitation instead of a closed door. Reserve *by construction* for impossibility
  you can actually derive — an argument from the shape of the data, not from the
  emptiness of one search.

### A checker that iterates the subject gets QUIETER as the defect grows — and a false claim about your own instrument nearly became doctrine

An editing accident deleted an entire generator block: 236 obligations across two ids,
silently gone. The author caught it within seconds — the generator prints the id list it
emitted, and six ids sat where eight belonged — restored, and reported the incident with a
conclusion attached: *the gate would NOT have caught this, because a missing generator block
produces a table with no rows to check, and the MISSING-seeds fire on rows that carry the
trigger, which the deleted code no longer visited.*

That claim was internally plausible, carried a mechanism, arrived beside a genuinely good
catch — and was **false**. Tested later by actually stripping every such row and running the
gate: it fires immediately, across the population. **The converse loop iterates
INVOCATIONS.tsv — an independent fixed input — not the obligation table.** It visits every
row that carries the trigger and asks whether the obligation exists, and that question is
answered the same way whether one obligation is missing or all of them. The absence has
nowhere to hide because the denominator does not come from the thing being checked.

**The inverted lesson is the durable one:**

- A converse check over an **independent population** is precisely the guard that sees
  wholesale absence.
- A check that **iterates the artifact under test** goes quieter as the defect grows —
  deleting rows deletes the questions, and the check is silent exactly when it is needed
  most. That is the failure mode to hunt for in any verifier: *where does this loop's
  denominator come from?* If from the subject, absence is invisible; if from an independent
  input, absence is loud.
- The cheap fast detector (the generator printing its emitted id census) is a real first
  line — it fired seconds before any gate could — but it is a first line, not the guard.

**And the meta-incident matters as much as the mechanics.** The false claim was a confident
negative about the author's OWN instrument, produced by *reasoning about* the gate instead
of running it — "I had a story for it, and nothing in my own head could have contradicted
it" — inside a report to a seat that was about to write it into this document, adjacent to a
self-caught error that lent the whole report credibility. The lead had already endorsed the
claim in a routed message; only the doc commit had not happened. Two rules from that:
**reasoning about an instrument is not a test of it — instruments answer questions about
themselves only when run**, on a seeded copy, exactly like any other subject; and **a lead
relaying a worker's claim into doctrine owes it the same verification as any other claim** —
endorsement is publication, and the doctrine file is the least recoverable place to publish
a plausible falsehood, because doctrine is what nobody re-derives.

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
