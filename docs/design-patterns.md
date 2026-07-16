# Design patterns — coordination without a coordinator

Distilled 2026-07-07 by the phase-3 session lead (Fable 5); patterns 11–13 added
2026-07-16 from the input-region campaign retro. These are the patterns behind ae's
design rulings — how independent processes (bash watchdogs, a Python sidecar, tmux
sessions, revive hooks) and agents coordinate safely with no central broker. Each
earned its place by surviving adversarial review; several were carved by it.
Companion: `gatekeeping.md` (how to review against these).

## 1. Ownership is a durable fact, never process state

If process A must stand down because process B owns a role, the ownership claim must
be readable by *strangers*: a file with defined freshness, not an env var, flag, or
in-memory state. The revive path that resurrects A is typically another process
entirely (another session's supervisor) — it shares nothing with B's launcher.

*Form:* `owner-marker` file (atomic write: temp + rename; content = owner pid + stamp)
**plus** a freshness signal (heartbeat mtime ≤ N seconds). Marker without freshness is
an incomplete fact (gatekeeping B4): a crashed owner would squat forever.
*Exhibit:* `$AE_HOME/aewatch/bridge-owner` + heartbeat ≤ 90s (s19).

## 2. One guard at the chokepoint

When N code paths can revive/start a thing, do not gate N call sites — find (or make)
the single function they all flow through and put one ownership guard at its top.
N-site gating guarantees the N+1th future path ships ungated.

*Exhibit:* every bash-bridge revive path (launch hook, reattach hook, watchdog
`_supervise`) funnels through `_telegram_autostart_if_enabled`; the s19 guard is one
line at its top. The explicit human command (`ae telegram start`) deliberately
bypasses with a warning — human intent outranks the guard, but says so.

## 3. Fallback for free

When a new system takes over a role from an old one, don't disable the old system's
revive machinery — *aim it*. Let the takeover fact (pattern 1) suppress it; then when
the new system dies, the fact decays and the old machinery revives the old system
automatically. The degraded path costs zero new code and is exercised by the same
mechanisms that ran production yesterday.

*Exhibit:* aewatch dies → heartbeat stales → the next bash `_supervise` tick revives
the bash bridge → single sender again. The "enemy" revive paths became the safety net.

## 4. Fail-closed handoff ordering

Taking over a live role: **claim the fact → complete the fact → verify the fact →
stop the predecessor (all scopes) → only then act.** Any step fails → back out fully
and leave the predecessor running. A brief gap with *nobody* acting is acceptable;
a window with *two* actors is not — durable shared offsets (pattern 8) make the gap
lossless.

*Exhibit:* s19 handoff — write marker (fail-closed on write), refresh + verify
marker-plus-fresh-heartbeat, kill the bash bridge on *every* discovered tmux server
(not just the ambient one — gatekeeping B3), then first send. Implementation
caveat: the shipped code fail-closes before kill/send at every step, but a
heartbeat-write *exception* is contained by the daemon loop's crash-containment
and decay-stop path rather than a local marker rollback — the pattern above is
the ideal; the exhibit reaches its safety via two composed mechanisms.

## 5. Decouple liveness cadence from work cadence

A daemon loop maintains liveness facts (heartbeat) on one clock and runs components on
another. If one `sleep(interval)` drives both, every tuning of one concern silently
retunes the other: a long work interval lets the liveness fact decay (B6); capping the
wait to protect the fact over-drives the work (B7b).

*Form:* loop wakes at `min(interval, fact_decay_budget)` to maintain facts; each
component self-gates on its own due time. Re-verify the fact immediately before each
irreversible action, not just at loop entry (B7a).

## 6. Default-mode zero-diff

New subsystems arrive behind an explicit opt-in (`AE_WATCHDOG_IMPL=uv`); the unset
path stays byte-identical. This is a *provable* claim: structural unit guards, plus a
gate-time diff read, plus — where possible — an integration test of the default path
with the new component absent. It keeps `main` shippable through a long migration and
keeps rollback trivial (unset the flag).

## 7. Dual-run parity oracle

Porting a component: run old and new implementations against the same fixtures and
byte-diff their *ordered effects* (not their outputs — their effects on the world:
events appended, panes pasted, options set, messages sent). The oracle catches drift
in both directions — during the phase-2 parity port it found a real production bug
in the *old* bash side (`_agent_alert_reason` never emitted into the generated
watchdog), fixed before phase 3 began.

Know the oracle's blind dimensions and say them out loud: fixture ticks see no
wall-clock (missed B7b); a fake that ignores an argument dimension can't see bugs in
it (FakeTmux pre-s18a ignored `server=`). When a blind spot is found, widen the
oracle first, then fix the bug — the class dies, not the instance.

## 8. At-least-once with durable offsets beats exactly-once ambitions

For message forwarding across process boundaries and handoffs, persist consumption
offsets durably (byte offsets + inode guards), resume from the last durable position,
and accept occasional duplicates on crash-mid-send. Exactly-once needs a transaction
you don't have; silent loss is the failure mode users can't detect. Duplicates are
annoying; loss is invisible — pick annoying.

*Exhibit:* shared `$AE_HOME/telegram/{state.tsv,tg_offset,current_target}` across
bash↔aewatch handoffs (outbound position, inbound offset, *and* routing target);
truncation-reset guards for shrunk-same-inode files.

## 9. Routing key, address, spec, truth — never conflate

Four things about an agent (or any managed process) that want to be one string and
must not be:
- **Routing key** — what the machine routes and stores by. Maximally stable,
  boring, never derived from anything mutable: the session-qualified meta *slot*
  (`agent.main` / `agent.worker.N`), per the request-integrity ruling.
- **Address** — what humans type and read (`opus48:builder`). Short and
  meaningful, resolved to a routing key at use time; may drift across a rename
  without breaking stored requests, because nothing durable keys on it.
- **Spec** — what it's configured to be (model, effort, flags). Lives in config.
  Model-named aliases like `opus48` are spec-flavored *display labels* — fine in
  an address, never in a routing key.
- **Truth** — what it actually is right now. Only observable live (the TUI footer);
  the spec can silently lie the moment a harness falls back.

Encoding spec into identity means every respec is a rename; keying storage on the
*address* means every rename orphans in-flight state (the identity-churn incident:
requests orphaned by a config-driven rename). Surface truth next to the address in
status displays; alert on spec-vs-truth drift; route by the key, always.

## 10. Knowledge belongs in guards, not memories

Any invariant worth writing down is worth making a test fail over: emission-
completeness guards, contract-coverage guards, isolation tripwires, structural
`set -e` region checks. A mutation-proven guard outlives every contributor — human or
model. The corollary is the review rule: **a guard that cannot fail is decoration**
(delete the protected thing; if nothing goes red, the guard is theater).

Prose doctrine (like this file) is the fallback for what guards can't encode: name the
principle, cite the lived exhibit, and keep it one-sitting readable.

## 11. The delivery ladder — push beats pull; verify the effect

Inter-agent delivery has no ack in the substrate: a paste can stage unsent while the
sender's helper reports success. Rank the channels by what can eat them, and never
treat a send's own report as evidence:

1. **memo** — durable, cannot be eaten. First for anything that matters.
2. **short pointer via `send`** (<400 chars) — long sends chunk into
   `[Pasted Content N chars]` tokens and rot; the remainder leaks as literal text.
3. **`interrupt`** — clears staged junk *and* delivers. The reliable rung when the
   input already holds debris.
4. **`say`** — for the human on Telegram; pane output does not reach them.

Verify by `peek`: neither the success event nor the ledger's "pending" is evidence.

But memo-first is not memo-only. **Memos are PULL; sends are PUSH — a pull-only
handback is invisible.** A worker that memo'd its results, declared done, and never
sent a pointer left its lead waiting on work that already existed; the human had to
bridge. A handback is incomplete until a pointer-send is *delivered* — bounded retry,
else `state blocked: undelivered handback to <agent>`. Never `done` with an
unacknowledged handback. Observed same-day in two sessions and two model families:
a model-behavior pattern, not a one-off.

*Exhibit:* five silent-loss occurrences in the 2026-07 campaign, three inside the
very slice fixing them; the ladder's rung 3 recovered every one.

## 12. One sensor, shared by every caller

Pattern 2's read-side sibling. Pattern 2 puts one guard at the chokepoint N paths
flow through. When N callers each answer *the same question about the world*, they
must call one sensor — or they drift and disagree, and the newest caller is the one
that is wrong.

*Exhibit:* the send path used the structural input-region predicate; `_spawn` kept
its own `❯|bypass permissions|for shortcuts` marker grep. They disagreed on exactly
one real state — the trust dialog — and spawn pasted a worker's brief into a modal,
producing a launched, brief-less worker: the same silent-loss class the sensor
existed to kill. Fix: `_spawn_input_ready` routes modeled tools through the same
predicate, so spawn can no longer disagree with it.

*The nuance that makes it honest:* unmodeled tools keep their marker probe — the
shared sensor calls them safe unconditionally, so routing them through it would mean
"ready the instant the pane exists", i.e. the boot-gap paste the poll exists to
prevent. Share the sensor where it *models* the tool; do not share it into a lie.

## 13. A brief is a hypothesis; evidence outranks it

A brief that names a mechanism is the lead's *belief*, not an instruction to make
that mechanism true. The worker owes the lead evidence, not compliance — and
disproving a brief is a first-class deliverable.

*Exhibit:* a worker briefed "a no-change sweep skips the state-file write → mtime
stales → false alert" verified two ways — code (the write is unconditional) and
empirical (two byte-identical sweeps; mtime advanced) — and disproved it, making no
fix, because the briefed fix would have *masked real outages*: the "spurious" alert
was a true positive from the only working detector. Lead ruling: never execute a
lead's hypothesis against evidence.

*The residue is the point:* the proof lives in the two probes (the code read and the
empirical mtime check); five regression tests now pin the contract the disproof
rested on, so it stays true. **A disproof that leaves no guard behind will be
re-briefed.**

*Corollary — history carries the why.* When evidence kills a feature, ship the build
and the cut as two commits (`5a2e6a4` feat + `4f63c19` cut), so the history carries
*why* it died, not just that it is absent.
