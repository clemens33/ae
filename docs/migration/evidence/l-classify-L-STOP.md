# Joint L classification — L-STOP worksheet (section 4 of 6)

Seats: fable5:lead (author) + gpt56sol:colead (independent read pending). Ordering set by
colead: L-STOP is independent of the END/COMPACT dependency chain and follows it.

Capture: `l-artifacts/L-STOP`, 18 arms, 838 files, 20 roster ids. Frozen `ae`
`b7b8aa9f…`; the four hooked arms use L-HOOKS-v2 (`4cc428e9…`), **reproduced by lead
from `git show 72c7293:ae` during this classification** — see the correction note below.

Grain requirements carried forward from L-COMPACT, honoured throughout:
1. **Raw pointers** to claim-bearing captures.
2. **Terminal-vs-downstream rc attribution kept separate.**
3. **Capture-only `code-observation` rows stay UNCLASSIFIED** pending seat ruling.
4. **A row's HEADLINE is checked against its own BODY** — added this section, after
   SC-835d (below) showed the two can disagree.

---

## The section's own correction, verified rather than accepted

L-STOP's manifest carries a `Correction` (added by lexec after L-RENTRANS exercised the
barrier) stating that `b_stop_one_pre_kill` was inserted in v2 INSIDE the fleet-only
`expect_set == true` branch, so it could never fire for the singular stop its name
describes, and that no arm of this section used it.

**Lead verified the load-bearing half independently**, because five committed sections
depend on it: `mkhooks2.py`/`mkhooks3.py`/`mkhooks4.py` run against
`git show 72c7293:ae` reproduce `4cc428e955a1e390…`, `b1b07709b01a66f7…` and
`c66fe2d897c5d3b3…` — byte-identical to what the manifests record. **No committed
capture is invalidated.** The correction stands as written.

---

## SC-515 — exit folding

**SC-515a — `stop all` folds per-target result records into its exit.** Bucket 2.
Arm: `exit-folding-planted-failure` (**rc=1**). Construction stated in the manifest: the
supervisor is held at its entry barrier and killed so it writes nothing, and the
controller supplies EVERY per-target record from producer-harvested lines with only the
op id and target name substituted (`planted.diff` shows each byte diff) — one FAILURE,
two SUCCESS.
IS: the caller's bounded wait folded them and exited **1**, `Error: stop failed for:
proj`. One failing record among three drove the exit.
**CONFIRMED.**

**SC-515b — result-wait timeout is not a failure.** Bucket 2.
Arm: `exit-folding-results-timeout` (**rc=0**). Same hold-and-kill, nothing planted, so
no per-target record is ever written and the wait reaches its bound.
IS — the sharpest confirmation in this family: **rc=0 with all three results missing**,
stderr `note: 3 result(s) still pending after 30s — the stop continues out of process`
plus a runnable readback recipe. A still-working supervisor is not called a failure, and
the operator is handed the command to finish the story themselves.
**Deviation recorded by the producer rather than hidden:** the arm waits out the REAL
30s bound. At 72c7293 the wait is called with a literal `30` and there is no environment
knob; shortening it would mean editing a constant, which exceeds the hook-only
instrumentation contract. Stated as a deviation from the design's wording, not as a
shortened bound.
**CONFIRMED.**

**SC-515c — an unowned ae-tagged session is named, not stopped.** Bucket 2.
Arm: `exit-folding-unowned-ae-tagged` (**rc=1**). A plain tmux session created directly
on the recorded server, given `AE_SESSION`, with no session directory.
IS: all four clauses of the row observed at once — **named** (`'ghost' is an ae-tagged
session on the current tmux server that ae's metadata does not own`), **not killed**
(`— NOT stopped`), **partial failure** (rc=1), and the message **names both ways out**
(`Adopt it ('ae doctor --refresh ghost') or stop it explicitly: ae stop ghost`).
**CONFIRMED.**

---

## SC-815 — fleet identity

**SC-815a — the confirmed fleet is the fleet acted on.** Bucket 1.
Arm: `fleet-fourth-session-in-confirmation-window` (**rc=0**). `ae stop all` on a real
terminal over three sessions; while the confirmation prompt is displayed the controller
launches a FOURTH real session, then answers `y`.
IS: `3post.tmux.txt` shows **exactly one session alive — `fourth`** — with its own
windows and panes intact. The three confirmed sessions are gone; the newcomer was never
enumerated and never touched. `stop all` did not re-enumerate after confirmation.
**CONFIRMED.**

**SC-815b — fleet entries carry session identity, not names.** Bucket 1.
Arm: `fleet-name-handoff-mid-op` (**rc=1**). Held at `b_stop_supervisor_entry` — op id
validated, nothing acted on yet — the controller ENDS one confirmed target and RELAUNCHES
it under the same name, then releases.
IS — all three clauses of the row in a single recorded record:
`[op 09b4af35…] FAILED (rc 1): Error: 'other' is not the session that was confirmed — its
recorded instance is now 624b543c…, was 1c4131b0….\n  The name was reused by a different
session after confirmation; that session was NOT stopped, and nothing was changed.`
The entry was keyed on the **instance uuid**; the newcomer is **left running**; the
failure is **recorded and explains that the name changed hands**.
**CONFIRMED** — the strongest row in the section.

**SC-815c — each fleet run has a unique operation identity and consumes ONLY its own
results.** Bucket 1.
Arm: `fleet-concurrent-ops` (**rc=0**). Run A held at `b_stop_supervisor_entry` while run
B starts; both supervisors' argv (and both op ids) read from the process table, then both
released. The op id is read from the **detached supervisor's own argv** — a system
observation by the controller, never a hook reading product state.
IS: session `other` carries **two** stop-result rows with **distinct op tags** —
`[op b277254f…] stopped: verified gone on its recorded server` and
`[op 06fff8e3…] FAILED (rc 1): Session 'other' is not running.` Each run recorded its own
outcome for the same target and neither consumed the other's. The second run's honest
failure is itself the evidence: it did not inherit A's success.
**CONFIRMED.**

**SC-815d — the visible representation is `[op <uuid>]` in the events.** Bucket 2.
IS: every stop-result summary in every fleet arm carries the literal `[op <uuid>]`
prefix. Directly observed, no inference.
**CONFIRMED.**

---

## SC-835 — singular stop and self-stop

**SC-835a — stop addresses the recorded server and the exact session id, never a name.**
Bucket 1. Arm: `plain-stop` (**rc=0**), three real sessions on one recorded server
**including the prefix-sibling pair `proj`/`projx`**, with a delegate-and-log tmux shim
tracing every argv.
IS — both halves of the row in one recorded argv line:
`<-S> </private/tmp/…/tmux-501/default> <kill-session> <-t> <$0>`
The **`-S`** names the recorded socket (not the ambient server) and the target is
**`$0` — a session ID, not the name**. The hazard the row exists to prevent is that
`tmux kill-session -t proj` prefix-matches `projx`; `3post.tmux.txt` shows **`projx`
still alive** after `proj` was stopped. The sibling was present, the id was used, and the
sibling survived.
**CONFIRMED** — and this is the section's sharpest, because the arm was built so it
*could* have killed the wrong session.

**SC-835b — stop reports stopped only after verifying the session is gone.** Bucket 1.
IS: every success record in the section reads `stopped: verified gone on its recorded
server`. The claim of verification is in the durable record, not only in the stdout line.
**CONFIRMED.**

**SC-835c — an unverifiable kill fails loudly and changes nothing.** Bucket 1.
Arm: `unverifiable-kill` (**rc=1**) — the directory holding the recorded tmux socket is
removed while the server process keeps running.
IS: `Error: cannot verify session 'proj' (its recorded tmux server is unreachable) —
nothing was stopped, state preserved.` Loud (rc=1, named cause), no success report, and
the no-change half asserted in the message itself.
**CONFIRMED.**

**SC-835d — stop never deletes anything.** Bucket 1.
Arm: `plain-stop`, `1pre.aehome.tsv` vs `3post.aehome.tsv`.
IS: the before/after difference is **exactly one path — `sessions/proj/.watchdog.pid`**,
which is removed. Everything else is preserved.
**CONFIRMED WITH A PRECISION REQUIRED — the row's HEADLINE contradicts its own BODY.**
The headline says "never deletes anything"; the body says "ae state, working tree, and
provider conversation files are preserved either way". The body is satisfied exactly: a
pid file is runtime bookkeeping for a process that no longer exists, not user state. The
headline is falsified by one file.
This is not a product divergence — it is a contract-text defect, and a reader checking the
headline literally would file a false divergence. **Proposed precision:** *"stop deletes
no user state — ae state, working tree and provider conversation files are preserved;
runtime bookkeeping for the stopped session (e.g. its watchdog pid file) is cleaned up."*
Colead's ruling requested; this is a b1 row so the rewrite is a seat act.
*(Generalised into grain requirement 4 at the top of this worksheet.)*

**SC-835e — self-stop confirms with the recoverability warning.** Bucket 1.
Arm: `self-stop-without-y`, `pty.at-prompt.txt`.
IS — the prompt states the guarantee in the row's own terms, verbatim:
```
Stop 'proj'? This kills the session you are working in.
  Agents may be mid-turn: active writes and partial turns can be interrupted.
  Your ae state, working tree and provider conversation files are PRESERVED —
  the guarantee is recoverability (resume from the provider's own checkpoint),
  not mid-write atomicity.
Continue? [y/N]
```
"recoverability from the provider checkpoint, not mid-write atomicity" is not paraphrased
— it is the shipped text.
**CONFIRMED.**

**SC-835f — `-y` skips the self-stop confirmation.** Bucket 2.
Arms: the `self-stop-without-y` / `self-stop-with-y` pair — identical construction, one
flag differing.
IS: `without-y` captures the prompt (`Continue? [y/N]`, 1 occurrence). `with-y` has **no
`pty.at-prompt.txt` at all** and its `pty.after-answer.txt` reads `(pane gone)`.
**CONFIRMED BY PAIRED CONTRAST, with the evidence weakness stated.** The paired control
is the right design and it carries the claim. But an **absent file is not a recorded
absence of prompt**: the harness did not capture "we looked and there was none", it
captured nothing. And the manifest names `pty.at-prompt.txt` as a key artifact for *both*
self-stop arms, so it names a file that does not exist for one of them.
Weak, not wrong — see the gate item below, which is the same class in the same arms.

**SC-835g — self-stop executes via a short-lived out-of-pane supervisor.** Bucket 1,
declared empirical basis **census-2** (not this arm).
**RULED: PARTIAL — GATE ITEM. The arm asserts an observation its own artifact does not
carry.** `ARM.txt` records `supervisor_observed  yes` and `supervisor_bound_sec  30` in
BOTH self-stop arms. The artifact named for it, `supervisor.ps-lineage.txt`, is
**byte-identical to `3post.ps.txt`** in both arms (verified by `diff`) — a post-hoc
snapshot containing three processes, none of them a supervisor: the tmux server and two
`projx` sidecars. `1pre.ps.txt` shows `proj`'s own `events-tail` and `watchdog` alive
before and gone after, which is good evidence the **session died** and no evidence at all
about **what killed it**.
So `supervisor_observed yes` is either an observation whose evidence was not kept, or an
INFERENCE ("the stop succeeded, so a supervisor must have run") recorded in a field that
reads as a measurement. Neither can be distinguished from the committed tree, which is
why it is a gate item rather than a mark.
**The row itself is not unsupported** — its declared empirical basis is census-2, and the
composite argument survives independently: the durable `stop-result` reading `verified
gone on its recorded server` was written **after** the session died, so the verifier
cannot have been living inside it. That is a real out-of-pane proof. It is just not the
proof the arm claims to have made.
**NEEDED to close:** a capture that catches the supervisor while it exists and shows its
parent is not the dying pane. Raised to lexec.

**SC-835h — the self-stop outcome is a durable `stop-result` event.** Bucket 1.
IS: `3post.events.proj.jsonl` carries
`{"ts":"2026-08-20T17:33:15Z","actor":"human","action":"stop-result","target":"proj",
"summary":"stopped: verified gone on its recorded server"}` — readable after the pane it
would otherwise have printed to is gone.
**CONFIRMED.**

---

## SC-839 — the identity gate

**SC-839a — `--self` waives exactly one check.** Bucket 1.
Arm: `identity-c5-self-flag` (**rc=0**) — the `run-shell` construction with `--self -y`.
IS (empirical): the waiver works — the same construction refuses with
`refusing: C5 — this process has no controlling terminal (pass --self if it genuinely is
the session, e.g. from run-shell)` without the flag, and succeeds with it.
**CONFIRMED AS COMPOSITE (frozen source + partial empirical), and the composite label is
load-bearing.** The arm demonstrates the waiver; **no arm combines `--self` with a C2, C3
or C4 violation**, so the "and NOTHING else" half is empirically untested here — the arm
cannot fail that half of the claim.
It is carried instead by frozen source, which closes it structurally rather than by
sampling: C1–C4 run unconditionally inside `_stop_self_target_from_pane` (ae:6869-…),
`--self` affects only the tty branch that follows (ae:6854-6855, *"the tty branch then
adds C5, and `--self` bypasses C5 alone"*), and ae:6550-6553 refuses `--self` combined
with an explicit target outright — a guard whose comment records the bug it exists to
prevent (`ae stop all -y --self` once classified every candidate as self, kept only the
last, and left the rest live with rc 0).
Any future citation must carry both halves; the arm alone overstates.

**SC-839b — `--pane` accepts only a shape-checked tmux pane id.** Bucket 1.
IS, both directions: `identity-malformed-pane-token` (`ae stop --pane=notapane`) → rc=1,
`Error: --pane expects a tmux pane id like %3 (use --pane=#{pane_id}).` — rejection names
the shape and the fix. Accept side: both `identity-c5-*` arms pass `--pane=#{pane_id}`,
tmux-expanded to a real `%N`, and proceed.
**CONFIRMED.**

**SC-839c — the stop identity checks are C1–C5.** Bucket 1.
IS (empirical): **C2, C3 and C5 each name themselves** in a refusal —
`refusing: C2 — our tmux server did not answer for itself ($TMUX is stale or forged)`,
`refusing: C3 — the session's recorded tmux server did not answer`,
`refusing: C5 — this process has no controlling terminal (…)`.
**C1 and C4 produce no `refusing:` line at all** in their arms — both print the generic
`Usage: ae stop …` block instead. See SC-839d for why; the cause is a call-site
behaviour, not a missing check.
**CONFIRMED AS COMPOSITE (frozen source + partial empirical).** All five checks exist and
are named in source: C1 at ae:6871 and ae:6877 (two sub-cases: not inside tmux, and
`$TMUX` malformed), C2/C3/C4 following, C5 in the tty branch, with the design block at
ae:6690-6719 naming each. Three of five are directly evidenced; two are source-only in
this section.

**SC-839d — a stop refusal names the failed check.** Bucket 1.
**RULED: DIVERGENCE, SCOPED — plus a STALE EXAMPLE in the row itself.** Two separate
defects, both found by reading the call site rather than the arms.

*1. The claim does not hold on the implicit no-name route.* At ae:6560-6568 the frozen
code reads:
```
target="$(_stop_self_target_from_pane 2>/dev/null)" || target=""
if [[ -z "$target" ]]; then
    target="$(detect_current_session 2>/dev/null)" || {
        echo "Usage: ae stop [session-name] [-y]" >&2
        …
```
`_stop_self_target_from_pane` sets `_STOP_UNPROVEN` to a precise, actionable message —
for C1, `C1 — not inside tmux (need $TMUX and a pane id; use --pane=#{pane_id} from
run-shell)` — and the call site discards it **twice over**: stderr to `/dev/null`, and the
status swallowed by `|| target=""`. When `detect_current_session` then also fails, the
user receives a generic usage block that never mentions the diagnosed cause, and in the
run-shell case never mentions `--pane=#{pane_id}`, which is the actual fix ae had already
computed.
The swallowing is deliberate — the comment at ae:6555-6559 says the pane derivation is
tried first and the ambient oracle is "only as fallback" — so the *fallthrough* is by
design. What is not by design is that when **both** fail, the better diagnosis is the one
thrown away. Observed in `identity-c1-outside-tmux` and `identity-c4-pane-in-other-session`,
both rc=1 with usage and no `refusing:` line.
Note the frozen code's own opinion of unattributed refusals: the refusal site at ae:6625
defaults to `<unattributed — this is a contract gap, please report>`. ae already treats a
refusal that cannot name its check as a defect.

*2. The row's canonical example is drawn from a design that was replaced.* SC-839d
illustrates itself with `refusing: C4 — pane %0 is in 'alpha', not 'beta'`. That string
appears at ae:6863 **inside the comment explaining why that approach was abandoned**: it
describes what happened when the name was resolved ambiently and then filtered with C4,
and the comment concludes *"Filtering an ambiently-resolved name can only ever refuse the
sanctioned recipe; deriving it from the pane is what C4 already knows."* On the implicit
route C4 is now **constructive** — the name is derived from the proven pane, so there is
no mismatch left to refuse. C4 keeps its filter role only on the `all` route (ae:6867-6868).
The contract quotes as its illustration the behaviour of the rejected design.

**Consequence for the row:** the normative claim is sound and should be kept — it is what
ae's own `<unattributed>` fallback asserts. It needs (a) scoping to the identity gate,
where it holds, (b) a decision on the implicit route's discarded diagnosis, and (c) a new
example taken from shipped behaviour. Both (a)/(c) are seat acts; (b) is a product
question — filed separately, low severity, diagnosability not safety: nothing wrong is
killed, the user is merely told less than ae knows.

**SC-839e — the no-name form keeps tmux-controlled text out of shell programs.** Bucket 1.
Arm: `legacy-migration-injection` (**rc=0**). A valid session is launched, then migrated
into the LEGACY direct-child shape under the name
`leg'"$(touch SENTINEL_TOUCHED)"'x` — quoting plus command substitution with an embedded
sentinel — and C1–C4 are re-proved from a shell pane inside it before the implicit
no-name stop runs there.
IS: `ARM.txt` records **`sentinel_before 0`** and **`sentinel_after 0`** — the
substitution never executed. The name is carried as DATA end to end: printed verbatim in
`Stopping 'leg'"$(touch SENTINEL_TOUCHED)"'x' out of pane…`, and used verbatim as a
**filename component** (`3post.events.leg'"$(touch SENTINEL_TOUCHED)"'x.jsonl`).
**CONFIRMED** — and this is a real zero, not an absent one: the sentinel was scanned for
before and after, so the harness could have registered it.

---

## Proposed dispositions

- **CONFIRMED / no change — 14**: SC-515a, SC-515b, SC-515c, SC-815a, SC-815b, SC-815c,
  SC-815d, SC-835a, SC-835b, SC-835c, SC-835e, SC-835h, SC-839b, SC-839e.
- **CONFIRMED BY PAIRED CONTRAST, evidence weakness stated — 1**: SC-835f.
- **CONFIRMED AS COMPOSITE (frozen source + partial empirical) — 2**: SC-839a, SC-839c.
  Neither arm can fail the half the source carries; the label must travel with the row.
- **CONFIRMED WITH A PRECISION REQUIRED — 1**: SC-835d. The headline ("never deletes
  anything") is falsified by one pid file; the body is satisfied exactly. Contract-text
  defect, not a product divergence.
- **PARTIAL, gate item — 1**: SC-835g. `supervisor_observed yes` is asserted where the
  named artifact is a post-hoc snapshot byte-identical to `3post.ps.txt`.
- **DIVERGENCE, scoped, + stale example — 1**: SC-839d.
- **No INCONCLUSIVE arms. No ARM-INVALID.**

**Section total: 20.**

## What this section changed about how we read rows

1. **A headline can contradict its own body** (SC-835d). Grain requirement 4 added.
2. **A stale example can outlive the design it described** (SC-839d). SC-839d's
   illustration is quoted in frozen source as the behaviour of the *rejected* approach —
   found only by reading the call site, never by reading the arms. Examples need the same
   provenance discipline as claims.
3. **A field named like a measurement can hold an inference** (SC-835g). `ARM.txt`'s
   `supervisor_observed yes` reads as an observation and cannot be told apart from a
   deduction using the committed tree.
4. **An absent artifact is not a recorded absence** (SC-835f). The paired control carries
   the claim; the missing file does not.
