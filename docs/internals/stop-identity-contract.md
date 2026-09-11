# The self/target identity contract of `stop`

Moved verbatim from the bash glue's stop arm when the whole stop moved into the core
(`src/lifecycle.rs`, `_stop`; glue cut 3, 2026-09-03). The reasoning, the C/D facts and the
review-findings ledger are historical evidence for the core's behaviour: the glue's stop arm
is now a pane passthrough, and the core resolves the pane to a session itself.

```text
══════════════════════════════════════════════════════════════════════════
THE SELF/TARGET IDENTITY CONTRACT

THREAT MODEL. Everything below defends against ACCIDENTAL MISUSE by this tool's
single human and the agents acting on their behalf — a stale payload, a name
reused between two operations, a lifecycle op landing mid-freeze, a pane that is
not the session it appears to be. It does NOT defend against adversarial
processes on the same machine: anything that can forge argv, write ae's metadata
or hold its locks has already won by easier routes. Findings outside this model
are filed as issues, not fixed inline — the contract is here to keep ordinary use
from destroying work, not to survive an attacker who already owns the box.

The question every stop must answer is "may THIS process stop THAT session".
It decomposes differently per route:

  NAMED ROUTE  (`ae stop <name>`) — the human named a target. Self-ness is
    irrelevant; stopping a session from anywhere is legitimate. It needs only
    the destruction facts D1..D5. It must merely never be MISCLASSIFIED as
    self, which would route it through the supervisor and tell the user their
    pane is closing when it is not.

  IDENTITY ROUTE (`ae stop` with no name, or a target that IS us) — the kill
    would destroy the caller mid-operation, so C1..C5 must ALL hold before the
    work may be handed to the supervisor.

This block exists because four review rounds each found a different INCOMPLETE
FACET of the same proof (tty-but-not-server, server-but-not-target,
target-but-not-enumeration) — a design being derived by adversarial iteration
instead of stated. Every fact below names what PROVES it and what happens when
it cannot be proven. A fact with no proof mechanism is a REFUSAL, never a gap
to be filled later.

── CURRENT-TARGET FACTS (identity route; ALL required) ───────────────────
  C1  We are inside tmux at all.
      PROVEN BY: $TMUX and $TMUX_PANE both set.
      UNPROVABLE -> not self. Env is a PRECONDITION, never evidence: every one
      of these variables is inherited and can be handed to us.
  C2  Our ambient server answers for ITSELF.
      PROVEN BY: socket_path + server pid, round-tripped from that socket,
      equal to the fields we were handed in $TMUX.
      UNPROVABLE -> not self. Catches a stale $TMUX (server restarted) and a
      forged one.
  C3  Our ambient server IS the target's RECORDED server.
      PROVEN BY: the same socket_path + pid pair asked of the target's recorded
      server via _end_tmux — compared as an ANSWER, not as strings, so a -L
      name and an -S path naming one server still match.
      UNPROVABLE -> not self. Missing this killed a recorded session on server
      A from an identically-named interactive pane on server B.
  C4  Our PANE resolves, on that server, to the target session itself.
      PROVEN BY: our pane id -> '#{session_id} #{session_name}' on that socket,
      both equal to the target's exact live id and its name.
      WHICH PANE ID: $TMUX_PANE for an interactive shell. NOT for a run-shell
      child — MEASURED 2026-08-11: `run-shell -t probe` hands the child
      TMUX_PANE=%60 (inherited from the server's own environment) while the
      target pane is %0. The variable is not merely absent there, it is WRONG,
      and trusting it is exactly why `run-shell -t alpha` resolved and killed
      BETA. A run-shell child therefore learns its pane the only way tmux
      offers: a FORMAT the server expands for the target, passed as --pane
      #{pane_id} and validated as ^%[0-9]+$ before use. That token is
      tmux-generated and shape-checked, unlike #{session_name}, which is
      attacker-influenced text and stays out of the recipe.
      UNPROVABLE -> not self.
  C5  Our CONTROLLING TTY is that pane's tty.
      PROVEN BY: ps -o tty= of this process == '#{pane_tty}'.
      UNPROVABLE -> not self, EXCEPT under --self, which bypasses THIS FACT AND
      ONLY THIS FACT, because a tmux run-shell child legitimately has no
      controlling terminal. The flag buys the bypass of a proof MECHANISM; it
      never supplies the CONCLUSION, and C1..C4 still have to hold.

── DESTRUCTION FACTS (both routes; ALL required) ─────────────────────────
  D1  Name usable AND on-disk path a physical direct child, checked at EVERY
      entrypoint (public dispatch + both internal). A boundary is entered,
      never inherited. UNPROVABLE -> refuse, nothing touched.
  D2  A POSITIVE recorded server. UNPROVABLE -> refuse; ae never guesses a
      server, and since stop destroys nothing there is nothing to acknowledge
      one's way past.
  D3  Addressed by EXACT session id on that server, never by name (`-t <name>`
      prefix-matches).
  D4  The kill is VERIFIED gone, tri-state: alive and unverifiable are BOTH
      failures and neither may print "Stopped".
  D5  The lifecycle lock is held across identity -> kill -> verify.

── FLEET FACTS (`ae stop all`) ───────────────────────────────────────────
  F1  The fleet is what ae's METADATA owns; enumerate the RECORDED servers.
      Ambient enumeration is a redirectable heuristic — AE_TMUX_SERVER redirects
      it, and using it stopped server A's session while reporting on B's.
  F2  An AE_SESSION-tagged session seen ambiently but ABSENT from metadata is
      reported LOUDLY as unmanaged: never silently stopped, never silently
      ignored.
  F3  Any mismatch or per-target failure is a loud PARTIAL FAILURE, never
      counted as success.

── INTERACTION FACTS (the caller IS one of the fleet targets) ────────────
C1..C5, D1..D5 and F1..F3 each hold in isolation. Both round-6 blockers lived
in what none of them said: what happens when the session being enumerated is
the session doing the enumerating. Stating the interaction is the fix.
  F4  THE FLEET LOOP NEVER RUNS IN A PROCESS THE FLEET LOOP CAN KILL.
      Rounds 4-6 tried to IDENTIFY the caller among the targets and stop it
      last. That rests on a false premise: "cannot prove a caller" is not
      "there is no caller". A child with $TMUX/$TMUX_PANE sanitised away is
      still PHYSICALLY IN THE PANE that dies — measured: C1 unprovable, so the
      caller was classed as no-one, killed inline first, and the remaining
      target was abandoned. And --pane is an ARGV SELECTOR, not evidence of
      where the caller lives: any process can pass any valid id, so without C5
      the wrong target gets deferred while the real caller stays inline-killable.
      No-tty caller attestation is impossible IN GENERAL, so ae stops attempting
      it: `stop all` ALWAYS hands the loop to a DETACHED supervisor.
      PROVEN BY: construction, not inference — the supervisor is nohup'd with
      all stdio detached and holds no lifecycle fd, so no session it kills can
      be running it. Ordering is then a nicety, never a safety property, and no
      flag or pane id is consulted for `all` at all.
      (C5/--pane machinery stays for SINGULAR self-stop, where the caller IS
      the named target by construction and the tty proof genuinely holds.)
  F5  A TARGET AE DOES NOT OWN IS A FAILURE, NOT A NOTE. An unmanaged
      ae-tagged session found and left running means the fleet operation did
      NOT do what `stop all` says. It contributes a non-zero rc, and the
      success-shaped summary ("No running ae sessions.") is suppressed whenever
      anything was found. This check KILLS NOTHING, so it stays with the caller
      where its rc is directly useful; F4 governs the loop that kills.
  F6  OWNERSHIP AND LIVENESS ARE SEPARATE QUESTIONS, and liveness is TRI-STATE.
      Enumerating the fleet by "has a resolvable live id" silently DROPPED every
      target whose recorded server was UNREACHABLE, conflating it with verified-
      stopped: `stop all` answered rc 0 "No running ae sessions." for a session
      that singular `stop` correctly refused with "cannot verify". Enumerate
      OWNERSHIP from metadata; then skip a target only when it is VERIFIED GONE.
      UNKNOWN is carried into the fleet and folded as FAILURE, never dropped.
  F7  HANDING OFF THE LOOP MUST NOT COST THE CALLER ITS EXIT CODE. F4 moved the
      kills out of process; an rc that then covered only the HANDOFF would starve
      every OUTSIDE caller — a script running `ae stop all` deserves a real exit
      status, which is F5's rc-honesty one level up. So the caller WAITS ON THE
      RECORD instead of on the loop: bounded (~30s) polling of each target's
      durable stop-result, then the outcomes fold into the rc alongside F5.
      PROVEN BY: the caller's SURVIVAL, which is never asked as a question — F4's
      abolished identity proof does not come back in disguise. A caller that WAS
      a target dies mid-wait having already reported all it could honestly know;
      a caller that was not stays and reports. Timeout is not a verdict on the
      sessions: the handoff rc stands, "results pending" is said out loud, and
      the read-back one-liner is named.
  F8  AN OPERATION AWAITS ITS OWN RESULTS, NOT RESULTS IN GENERAL. F7 asked "is
      there a stop-result newer than my baseline" — a COUNT DELTA, and a count
      delta carries no provenance. Measured: two concurrent `ae stop all -y` over
      one session, and the FIRST child's single result satisfied BOTH callers
      (rc 0 twice) while the second child found an empty fleet and emitted
      nothing. PROVEN BY: an OPERATION ID minted per invocation, stamped into
      BOTH stop-request and stop-result, with the poll filtering on it. One
      function owns the stamp so writer and reader cannot drift.
  F9  WHAT WAS CONFIRMED IS WHAT GETS ACTED ON — THE SET IS FROZEN AT
      CONFIRMATION. The supervisor re-enumerated metadata after the handoff, so
      the fleet it KILLED was not the fleet the human APPROVED: a session created
      in that gap died unconfirmed, and unawaited (measured — both died, caller
      rc 0). A destructive scope must never be re-derived downstream of its own
      confirmation. PROVEN BY: the caller freezes the list and passes it as argv;
      the supervisor enumerates NOTHING and may only shrink that set, never grow
      it. F8's id and F9's list are one payload: they are minted together, travel
      together, and answer the same question — WHICH operation, over WHICH set.
  F10 A FROZEN NAME AUTHORIZES KILLING THE INSTANCE THAT CARRIED IT AT
      CONFIRMATION, NEVER WHATEVER CARRIES THE NAME AT KILL TIME. F9 froze the
      SET but the set was made of NAMES, and a name is a slot, not a thing —
      classic ABA. Measured: create a session, `ae end` it completely, launch a
      FRESH one under the same name, then act on the frozen payload; the
      REPLACEMENT died and success was recorded. The window is exactly as long as
      a human stands at the confirmation prompt, and it falsified the promise the
      docs make — that a session started while you were deciding is left alone.
      PROVEN BY: freezing (name, session_id) pairs — the instance token ae already
      records, minted per launch and preserved across stop/resume, rename and
      transfer — and comparing the frozen value to what meta records RIGHT NOW,
      INSIDE the per-name lifecycle lock, immediately before the server is
      resolved and the kill issued. Inside is the whole fix: a check taken before
      the lock is acquired leaves the identical race, since a launch can complete
      between the check and the kill. A mismatch is a STAMPED FAILURE naming both
      instances, and the replacement is left running.
      "-" IS NEVER COMPARED AS IDENTITY. Treating it as a value that must match
      was still wrong, because it is a SHARED sentinel and equality is not
      identity when the value is shared: two pre-feature sessions both froze "-",
      so ending one and `ae rename`-ing another into its name (rename preserves a
      missing id) satisfied the comparison exactly, and the replacement died with
      rc 0. Two layers, and they compose:
        (a) MINT AT FREEZE. A session entering a fleet confirmation without an
            identity is GIVEN one there — under its own lifecycle lock, written
            durably (migration-on-touch, as with the doctor migrations and the
            legacy-name arm). From its first fleet contact it has a real token:
            stop/resume preserves it, and rename then moves something that
            actually denotes a session.
        (b) REFUSE A FROZEN "-". After (a) a dash can only be a stale or forged
            payload, so it is refused outright and stamped, naming this fact.
            Nothing is ever authorized by a sentinel two sessions can share.
  F11 A PAYLOAD IS ACCEPTED WHOLE, OR NOT AT ALL. The first shape check validated
      the odd tail AFTER the loop, so `opid oddone <id> dangling` KILLED oddone
      and recorded it verified-gone before returning 1 — a refusal issued after
      the destruction it existed to prevent. The committed seed missed it because
      one bare name is ZERO complete pairs: the specimen never reached the
      property. PROVEN BY: a preflight over the entire payload — count, every
      name, every token — that must pass before the first pair is acted on. This
      is the round-3 validate-before-act law, moved from the single target to the
      batch.
══════════════════════════════════════════════════════════════════════════
```

## The absence proof, and who is allowed to weaken it (2026-09-11)

The contract above is about `stop`. The same question — *is that session gone?* — is asked
by a resume, by the fleet listing and by `compact`, and the answer is `src/tmux.rs`'s
`StopProbe`. It has one strict reading and one widened one, and which one a caller gets is
the whole design.

**The strict reading** is `interpret_stopped`. A session is `Absent` only because the server
said so: it listed its sessions without the name, or it reported the clean-exit
`no server running on …` diagnostic. Every failure is `Unknown`, including
`error connecting to … (No such file or directory)` — because a server that is still
running answers exactly that once something unlinks its socket. `stop`, `end` and `compact`
cross this one and nothing else. They are irreversible; being wrong costs work.

**The widened reading** is `classify_absence`, and only a RESUME and the fleet LISTING may
ask for it. It adds one fact the strict reading does not have: the host's boot time
(`doors::boot_time` — `/proc/stat` `btime` on Linux, `sysctl -n kern.boottime` on macOS,
through `transport::run_sysctl`). On ENOENT, and on ENOENT alone, a session is `Absent` when
its own last sign of life predates the boot. No process survives a reboot, so nothing started
since can be holding it.

**What counts as a sign of life** is `inventory::last_live`, and the rule is narrow on
purpose:

* `.launch-attempt` (`store::LAUNCH_ATTEMPT`) — the launch-attempt stamp, written by every
  launch, resume and spawn, for every tool, under the lifecycle lock and BEFORE the tmux
  create it describes. It is checked, not best-effort: a launch that cannot write it is
  refused and creates no tmux session. It exists because every other fact is published
  AFTER the create, so an attempt that died in between would have left a session ae could
  then prove "gone".
* `started`, `launch_time.<slot>`, `capture_floor.<slot>` — meta rows, for the sessions that
  predate the stamp. `launch_time` and `capture_floor` are capture-tool-only and
  best-effort; they are read, never relied on.
* the watchdog pidfile's mtime.

**What does NOT count**, and each for a reason that was found the hard way:

* the meta file's own mtime — an upgrade migration, `ae doctor --refresh` and a rename all
  rewrite the meta of a session that is not running.
* `events.jsonl` — the event ledger is a ledger, not a heartbeat. `memo add`, `goal` and the
  audit records are appended from OUTSIDE a session by a human with no tmux anywhere, so a
  memo left on a stopped session would date it to now and make every later reboot
  unprovable.

**Every source answers in three states, and the third is why this is not an `Option`.** A
source that is NOT THERE says nothing, which is exactly what every session written before
that source existed looks like. A source that IS there and cannot be read — wrong mode, a
directory in its place, contents that are not a moment, a claim longer than any epoch — is
DAMAGE, and one damaged source makes the whole answer damaged. Folding damage into silence
would let an unreadable newer claim leave an older readable row proving the session gone,
which is the single direction this proof may never fail in. `tmux::Evidence` carries the
three states, `Evidence::folded` is the rule, and the per-source readings live with the
files they read: `store::SessionStore::launch_attempt`, `inventory::launch_epochs`,
`watchdog_glue::pidfile_modified`, `run::newest_start_marker`.

**A moment is a strictly positive epoch, and the grammar is PER SOURCE.** A row that NAMES
a launch moment and spells nothing after it — no `=` at all — is a damaged claim, not an
absent row, and is recognised by its name before the value is looked for. There is exactly
one legal non-positive claim: `capture_floor.<slot>=0`, which a retained exact resume
publishes when the legacy conversation has no known origin (`session_launch`). It means "no
floor", says nothing about a launch, and stays SILENT for liveness, because reading it as
damage would strand every such session after a reboot. Nothing else may borrow that
sentinel: the stamp is mandatory and has none, a negative floor is not it, and `started` and
`launch_time.<slot>` are launch facts that only ever spell a real moment.
`Evidence::floor_claim` is the one exception's owner and `Evidence::claim` the strict rule.

**The stamp is hostile persisted state**, so it is read under a byte cap
(`store::LAUNCH_ATTEMPT_CAP`) checked before the open and again on the read, and it is
WRITTEN through an exclusive temporary. The temp name is predictable and sits in session
state a human edits, so a plain create would follow a link planted there and truncate a file
outside the session entirely; `create_new` refuses a taken name whatever is behind it, and
only a temp this process made is ever cleaned up. A refused stamp is a refused launch, which
is the behaviour that was wanted anyway. The two pure reducers — `store::parse_launch_attempt`
and `inventory::launch_epochs` — carry cargo-fuzz targets, as AGENTS.md requires of any
parser of hostile persisted state.

**Fail closed** is every other branch: no boot time, no recorded activity, damaged activity,
activity at or after the boot, a boot time in the future (the clock moved), or any failure
that is not ENOENT. The refusal then says which one, and `ae doctor` prints the two numbers.

**The residual, documented rather than defended**: a foreign tmux server started after the
boot, on the same socket path, holding a same-named session ae never launched, whose socket
was then unlinked. The proof is about THIS SESSION being unreachable, never about that
server being empty — and a resume lands on the configured server regardless.

`tests/it/doors.rs::the_boot_time_proof_is_reachable_from_exactly_two_operations` is what
keeps the destructive gates out of the widened reading.
