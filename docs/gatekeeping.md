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

Nine confirmed holes from one slice family (s18/s19), generalized. Use as a diff-read
checklist — each maps to a question above, and each shipped past a builder *and* a
reviewer at least once before being caught.

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

## Verification mechanics

- **Rerun the gate legs yourself, on committed main, with unmasked exit codes.**
  `cmd; echo EXIT=$?` — never `cmd | tail` (the pipe's exit status is `tail`'s;
  a red suite reads as green — this happened, phase-3 review, and was caught only
  by re-running). Background long legs; hold the verdict until they land.
- **Artifact-map completeness claims.** "20/20 done" is verified by naming, for each
  Done-criterion, a file/test on main that satisfies it — not by trusting the count.
- **Mutation-proof every guard.** A guard or coverage check that cannot fail is
  decoration. Delete the thing it protects; the gate must go red.
- **A flake-tainted gate run doesn't count.** Rerun the failed leg alone on a quiet
  machine; "it was probably contention" is a hypothesis, and hypotheses get tested.
- **Source anchors decay.** `file:NNNN` citations in fixtures/plans are shape-checked
  by machine, semantically checked by nobody. Re-verify anchors by hand after any
  edit to the anchored file.

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
