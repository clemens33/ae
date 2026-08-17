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
