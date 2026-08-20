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
IS — one recorded argv line: `<-S> </private/tmp/…/tmux-501/default> <kill-session>
<-t> <$0>`, the target being **`$0` — a session ID, not the name**.
**COMPOSITE (colead's finding, adopted — my "both halves in one line" was wrong).** The
argv proves EXPLICIT TARGETING; it cannot prove WHERE that socket was selected from,
because in `plain-stop` the recorded and ambient servers COINCIDE. An
ambient-selector implementation would emit a byte-identical line. The **exact-id /
prefix-sibling half is directly confirmed**; the **recorded-not-ambient half rests on
the frozen `_end_target_server` call path**, or would need a two-server discriminator
arm to be shown at runtime. The hazard the row exists to prevent is that
`tmux kill-session -t proj` prefix-matches `projx`; `3post.tmux.txt` shows **`projx`
still alive** after `proj` was stopped. The sibling was present, the id was used, and the
sibling survived.
**CONFIRMED** — and this is the section's sharpest, because the arm was built so it
*could* have killed the wrong session.

**SC-835b — stop reports stopped only after verifying the session is gone.** Bucket 1.
**COMPOSITE (colead's finding, adopted).** Every success record reads `stopped:
verified gone on its recorded server` — but that is **ae's own prose asserting the
property, not evidence of it**. A message claiming verification is exactly as strong as
a message claiming anything else. The anti-oracle rule applies to the product's strings,
not only to its exit codes.
The ordering is proved by **sequence in frozen source**, ae:7948-7950:
```
kill_heartbeat "$name"
_lifecycle_kill_verified "$name" stop "$sid" || return 1
echo "Stopped $name"
```
The success line is unreachable unless `_lifecycle_kill_verified` returned 0. That, plus
the post-op tmux state, is the confirmation.
**CONFIRMED AS COMPOSITE (frozen source ordering + state snapshot).**

**SC-835c — an unverifiable kill fails loudly and changes nothing.** Bucket 1.
Arm: `unverifiable-kill` (**rc=1**) — the directory holding the recorded tmux socket is
removed while the server process keeps running.
IS: `Error: cannot verify session 'proj' (its recorded tmux server is unreachable) —
nothing was stopped, state preserved.` rc=1 with a named cause and no success report —
the **loud half is directly confirmed**.
**COMPOSITE for the no-change half (colead's finding, adopted).** "nothing was stopped,
state preserved" is an ASSERTION IN THE MESSAGE, not a proof of no change — the same
error I made on SC-835b. And the arm's `AE_HOME` diff cannot stand in for it: it
contains **concurrent event-file changes**, so it is not a clean no-change witness.
The no-change half rests on the frozen early-return path, which exits before the kill.
**CONFIRMED AS COMPOSITE.**

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
headline literally would file a false divergence. **My proposed precision was itself self-contradictory and is WITHDRAWN** (colead's
BLOCKER, adopted). `.watchdog.pid` lives **under the documented ae-state directory**, so
"ae state is preserved" while that file is removed is false — I fixed the headline by
writing a body that repeats the same error one level down. It also **froze cleanup as a
REQUIREMENT** inferred from one observed mechanism.
**Adopted rewrite, at outcome grain:** *"stop preserves all state required for full
resume — durable session metadata, working tree, and provider conversation files. It MAY
remove ephemeral runtime bookkeeping for the processes it stops."* `May`, not `does`:
permitted, not required — the same discipline as SC-1305's permitted no-session interval.
**COMPOSITE:** the `AE_HOME` manifest proves the pid-file exception; the manifest does
**not** capture the working tree or provider conversation domains at all, so those rest
on frozen source showing the stop path never touches them.
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
**NOT confirmed by paired contrast — RELABELLED COMPOSITE** (colead's finding, adopted).
My paired-contrast reading fails to a specific mutant: **a build that briefly prompts and
then proceeds passes the `with-y` side**, because that side never records a
prompt-ABSENCE observation. An absent file is not a recorded absence, and the manifest
names `pty.at-prompt.txt` as a key artifact for an arm that has none.
The `without-y` transcript is a sound CONTROL, and the claim closes structurally on
frozen source at **ae:7022**: the entire prompt branch sits inside
`if [[ "${_AE_STOP_YES:-}" != true ]]; then`, so `-y` bypasses it by shape and cannot
prompt-then-proceed.
**CONFIRMED AS COMPOSITE.** No rerun needed unless direct runtime evidence is required.

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
**My proposed rescue does NOT rehabilitate the field** (colead's finding, adopted). I
argued the durable event proves an out-of-pane writer. It does not: **an event cannot
identify its own writer or that writer's parentage**. I reasoned to a conclusion the
artifact does not carry — the exact failure I had flagged in others three rows earlier.
`supervisor_observed  yes` has no retained lineage evidence and is **WITHDRAWN as an
observation**; the arm defect is preserved below as an evidence-integrity note.
**The ROW closes as COMPOSITE on frozen source ordering**, which is sound because the
ordering is stated rather than assumed:
- **ae:7052** launches the detached one-shot worker —
  `nohup "$_ae_self" "$0" _stop-supervisor "$name" </dev/null >/dev/null 2>&1 &` followed
  by `disown`, with the comment *"the supervisor must own no end of the dying pane's
  tty"* and *"The detached worker. It — not the dying caller — owns the lock, the
  identity…"*.
- **ae:7380-7388** emits the result only AFTER `_stop_one_session` returns and only on
  its rc — `out="$(_stop_one_session …)" || rc=$?` then `if ((rc == 0))` → the
  `stop-result` event.
- The post-kill event proves that path survived the dying session.
**CONFIRMED AS COMPOSITE — and the runtime lineage has since been obtained** by
L-DISCRIM **D5b**, which holds the singular self-stop supervisor at an L-HOOKS-v5 barrier
at its entry (pid 48880), captures the live process table there, walks its ancestry while
alive, and diffs that live table against the post-hoc snapshot this section's artifact was.
The row now rests on source ordering AND a deterministic live capture.
**D5a's LINEAGE FILE is empty, but D5a is not lineage-free — my first statement of this
was too strong and lexec corrected it.** Its `supervisor-samples.txt` rows are
`ps -o pid=,ppid=,command=` output, so all three real-timing sightings read
`pid=20573 ppid=1 … ae _stop-supervisor s1`. **`ppid=1` is captured, while alive, three
times.** That is the load-bearing out-of-pane fact — a process reparented to PID 1 is
precisely what `nohup … & disown` produces.
What D5a cannot support is a multi-level ANCESTOR WALK, and that is what its lineage file
claims. So the correct scope is: D5a contributes existence under real timing **and the
parent**; D5b contributes the deterministic live table and the two-level walk. Its `supervisor-lineage.txt`
header claims an ancestor walk "taken WHILE IT WAS ALIVE" and contains `0<TAB>` — the
harness `wait`s for the 40s sampler before calling the lineage step, by which time the
supervisor is dead. **The arm built to close this defect reproduced it**, in its own
manifest, one artifact over. That is the sharpest possible demonstration that the class is
not about carelessness.
**Evidence-integrity note, retained deliberately:** `supervisor.ps-lineage.txt` is
byte-identical to `3post.ps.txt` in both arms while `ARM.txt` asserts
`supervisor_observed yes`. The row closes on other evidence; the arm's claim does not,
and the defect stays on the record so the artifact is never cited for it.

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
sampling. **My source account was inaccurate and is corrected here** (colead's finding,
verified by lead against the frozen tree): `_stop_self_target_from_pane` does **not**
carry C1–C4 — it handles C1/C2 plus constructive name resolution. The full C1–C4 proof
lives in **`_stop_current_target_proven` (ae:6904-6960)**, which produces C1 ×2, C2 ×1,
C3 ×2 and C4 ×3 named refusals. `--self` is then consulted at **ae:6964-6968**, and the
shape is the proof:
```
_stop_current_target_proven "$name" || return 1
# C5, and only C5, is what --self may bypass.
[[ "${_AE_STOP_SELF_FLAG:-}" == true ]] && return 0
```
C1–C4 have already run and returned before the flag is read, so `--self` structurally
cannot reach them. ae:6550-6553 additionally refuses `--self` combined with an explicit
target outright — a guard whose comment records the bug it exists to
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
**RULED: BUCKET 3 — fix-known-defect (#101)** (colead's BLOCKER, adopted). This resolves
through the contract enum, not as a free-form DIVERGENCE: the **SHOULD is KEPT** and
scoped to identity-gate refusals, the observed generic-only C1/C4 path is recorded, and
the row points at #101's intended behaviour (retain the precise C diagnosis alongside
usage).

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

*2. The example is RARELY REACHABLE, not stale — my finding was wrong and is corrected.*
I claimed the row's example describes a deleted design. **The C4 diagnostic producer
still exists in frozen source at ae:6956-6957**, in exactly the row's shape:
`_STOP_UNPROVEN="C4 — pane ${_pane} is in '${_pane_sess#* }', not '${name}'"`.
Colead's formulation is the correct one: **public reachability is stale or rare; source
existence is not.** So the example illustrates live code, and the defect is only that the
implicit route cannot surface it — which is #101, already filed.
**Consequence: the example is NOT rewritten.** Replacing it with a live C2/C5 example is
an optional clarity edit, not a second conflict, and rewriting a normative example merely
because the incumbent usually cannot surface it would let measurement edit the contract.
*What I originally wrote, kept for the record:* SC-839d
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
question — filed as **#101**, low severity, diagnosability not safety: nothing wrong is
killed, the user is merely told less than ae knows. The stale-example half is recorded
there too, flagged as a doc refresh rather than a code defect.

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

## Dispositions — POST-GATE

*Colead's independent read moved EIGHT of the twenty, and every move was away from a mark
I proposed. Two of the eight corrected my reading of the frozen source itself. Recorded
that way deliberately: a gate whose findings are folded silently into the totals leaves no
evidence it ran.*

- **CONFIRMED, direct — 11**: SC-515a, SC-515b, SC-515c, SC-815a, SC-815b, SC-815c,
  SC-815d, SC-835e, SC-835h, SC-839b, SC-839e.
- **CONFIRMED AS COMPOSITE — 8**: SC-835a, SC-835b, SC-835c, SC-835d, SC-835f, SC-835g,
  SC-839a, SC-839c. Every one of these has a half no arm in this section can fail; the
  label must travel with the row, and a future citation that drops it overstates.
- **BUCKET 3, fix-known-defect (#101) — 1**: SC-839d. SHOULD kept and scoped to
  identity-gate refusals; example NOT rewritten.
- **No PARTIAL, no free-form DIVERGENCE, no reopened conflicts, no INCONCLUSIVE arms.**

**Section total: 20.**

**Arithmetic flagged for colead:** your summary says "13 direct confirmed"; I count **11**
(20 − 8 composite − 1 bucket-3). The difference is that your list moves SC-835d and
SC-835g out of "direct" in the prose but the count appears to retain them. Raising it
rather than silently adopting either number.

---

## What this section changed about how we read rows

1. **A headline can contradict its own body** (SC-835d) — and **a proposed fix can repeat
   the same error one level down**. My precision said "ae state is preserved" while the
   deleted `.watchdog.pid` lives *under* the ae-state directory. The fix was outcome
   grain plus `MAY`, not `DOES`: permitted, not required, so one observed mechanism does
   not become a requirement.
2. **The product's own prose is not evidence for the product's behaviour** (SC-835b,
   SC-835c). `stopped: verified gone` and `nothing was stopped, state preserved` are ae
   asserting properties, exactly as strong as any other string it prints. Ordering came
   from ae:7948-7950; no-change came from the early-return path. **The anti-oracle rule
   applies to messages, not only to exit codes** — this is the sharpest lesson of the
   section and I had to be shown it twice in adjacent rows.
3. **An argv line proves targeting, not selection** (SC-835a). `-S <socket>` cannot
   distinguish "the recorded server" from "the ambient server" when the arm makes them
   the same socket. A discriminator needs two servers that differ.
4. **"Rarely reachable" is not "removed"** (SC-839d). I called the row's C4 example stale;
   the producer is live at ae:6956-6957. Public reachability is stale or rare; source
   existence is not — and a normative example must not be rewritten because the incumbent
   usually cannot surface it.
5. **A field named like a measurement can hold an inference** (SC-835g) — and so can a
   seat's rescue of it. My argument that a durable event proves an out-of-pane writer was
   the same over-reach I had flagged in the arm three rows earlier: an event cannot
   identify its own writer. The row closed on stated source ordering instead.
6. **An absent artifact is not a recorded absence** (SC-835f). The failing mutant is
   concrete: a build that briefly prompts and then proceeds passes an arm that never
   observed the prompt's absence.
