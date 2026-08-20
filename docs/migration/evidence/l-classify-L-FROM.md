# Joint L classification — L-FROM worksheet (section 5 of 6)

Seats: fable5:lead (author) + gpt56sol:colead (independent read pending).

Capture: `l-artifacts/L-FROM`, 12 arms, 646 files, **9** roster ids. Frozen `ae`
`b7b8aa9fb77afc07…`, unmodified — **no hook patch in any arm of this section**.

Grain requirements carried forward, honoured throughout:
1. **Raw pointers** to claim-bearing captures.
2. **Terminal-vs-downstream rc attribution kept separate** — and in this section the
   producer applied it first, unprompted (see "the section's own limits" below).
3. **Capture-only `code-observation` rows stay UNCLASSIFIED** pending seat ruling.
4. **A row's HEADLINE is checked against its own BODY.**

---

## A lead methodology error, recorded because it nearly set the roster wrong

I derived this section's roster by grepping `MANIFEST.md` for `SC-\d+` and got **11**
ids — SC-808 and SC-819 among them. Both are wrong. The manifest's
**"Roster coverage as executed"** statement names **9**: SC-809, SC-822, SC-823, SC-824a,
SC-824b, SC-825a, SC-825b, SC-825c, SC-826.

- **SC-808** appears only in a boundary statement: *"SC-808's re-proof surface is L-END's
  `compact-relaunch-lock-parent-mutated` arm; it is REFERENCED here and NOT re-executed."*
- **SC-819** appears only as a contrast: the `invalid-parent-validation-failing` mutation
  is *"deliberately DISTINCT from SC-819's unparseable class"*. SC-819 belongs to L-PURGE,
  where it was classified.

A grep finds mentions; only the coverage statement records assignments. This is the second
instance today of a lead probe returning a wrong count with no sign of being wrong — after
the checksum glob that undercounted 140 as 134. **The pattern: a cheap probe that answers
plausibly is the one that does not get re-checked.** Recorded so the next seat reads the
coverage statement first.

---

## The section's own limits, stated by the producer before review

Three, all recorded in `MANIFEST.md` rather than found by a reader:

1. **Every non-pty launch ends with the frozen attach failing** (there is no terminal), so
   those rc values reflect the ATTACH, not session creation. Each arm records the created
   state separately in its manifests and full sweeps **rather than resting on the rc**.
   This is grain requirement 2, applied by the producer without being asked — it is why
   `child_launch_rc 1` and `resume_rc 1` appear throughout the lineage arms without
   contradicting a single confirmation below.
2. **`existing-target-worktree`** reaches its shape through a controller removal of the
   session state directory after a real stopped `--worktree` launch; the removal is the
   named construction and is stated in `target.txt`.
3. **The watchdog is left ENABLED** (the launch default) in every arm.

---

## SC-809 — lineage is never inferred from a name

Bucket 1. Arm: `name-never-infers`.
Construction: a real parent archive is produced by a real end of a session named `same`
(`parent_uuid c83db248-e951-426a-885e-0ed8989285ff`); a NEW session is then launched under
**that same name WITHOUT `--from`**.
IS: the new session's meta records **`parent_archive_id=-`**. The name matched an existing
archive exactly and no lineage was attached.
*rc note:* the arm's rc=1 is the frozen attach (`open terminal failed: not a terminal`)
while stdout shows the session did start (`Watchdog started in hidden ae-monitor window`).
Per grain requirement 2 the rc belongs to the attach, and the claim rests on the recorded
meta, not on the rc.
**CONFIRMED.**

## SC-822 — `--from` is valid only for a session that does not exist in any form

Bucket 1. Arms: `existing-target-running`, `existing-target-stopped`,
`existing-target-worktree` — **all three rc=1**.
IS: all three produce the same refusal, and the message states the row's own reasoning
rather than merely refusing:
```
Error: --from is only valid for a NEW session, and 'tgt' already exists.
       Inheriting an archive into a running or resumable session would mean two
       different things (replace its history? merge it?) with no safe default.
```
The three-arm design is what carries "in any form": a live tmux session, a stopped
session with state, and a worktree are three different kinds of existence, and each is
refused identically.
**CONFIRMED.**

## SC-823 — the parent is proved before anything is created

Bucket 1. Arms: `invalid-parent-nonexistent` (rc=1,
`Error: no archive 00000000-0000-4000-8000-000000000000 in …/.ae/archive.`) and
`invalid-parent-validation-failing` (rc=1, `archive: digest says ## Handover (0) but meta
says 99.` → `Error: archive 35ae42c1… did not validate — refusing to inherit from it.`).
IS — **the strongest negative in batch L, because it is proved exhaustively rather than by
spot check.** Each arm takes a FULL sweep of session dirs, worktrees, tmux sessions, the
archive root, `AE_HOME` and the work dir before and after, and diffs them:
**`full-sweep.diff` is 0 bytes in BOTH arms** (pre and post captures identical at 395
bytes / 28 lines each in the nonexistent arm).
"a refusal leaves no tmux session, no session state, no worktree" is not sampled here —
the whole observable surface was compared and nothing moved.
**CONFIRMED.**

## SC-824a — proof facts are recorded as proved, never re-read

Bucket 1. Arm: `transport-cut` — the PLAIN launch path
(`ae --local child --from <parent>`), with an instrumented barrier immediately after the
launch parses its FIRST parent proof, where the controller deletes the parent archive.
IS: the child's meta carries the proof's facts as recorded values —
`parent_archive_id=2a3a6787-…`, `parent_archive_handover_count=0`,
`parent_archive_pending_count=0` — and they are still there after the parent archive is
gone. Nothing was re-read from a file another process was deleting, because the facts had
already been recorded from the one proof.
**Boundary stated by the producer and respected here:** this arm exercises the plain
launch path only and *"carries no re-proof expectation and no rollback machinery of its
own"*. Its `source-trace.*` file is a frozen-source extract with line numbers — every line
after the first proof naming `PARENT_ARCHIVE_ID`, `FROM_ARCHIVE`, `_ar_from_preflight`,
`_AE_FROM_EXPECTED` or the archive root — and is explicitly *"a code observation, not a
runtime trace, and it carries no verdict"*.
**CONFIRMED.**

**Seat note — evidence deliberately NOT claimed.** This arm's stderr also records a
product re-proof and rollback:
```
note: parent archive 22685cdf… is no longer on this machine — the lineage is recorded,
      but its digest cannot be read.
Error: parent archive 22685cdf… stopped validating while this session was being created.
ae: launch failed — rolling back 'child'.
```
That is SC-808's claim almost verbatim — re-prove immediately before publication, roll the
launch back on mismatch rather than creating a child with no lineage. The manifest
**declines to claim it**, because SC-808's designated surface is L-END's
`compact-relaunch-lock-parent-mutated` arm. Recording it as **CORROBORATING, not primary**:
a producer refusing to bank evidence that fell into its lap is the discipline that makes
the rest of the roster trustworthy, and the corroboration is worth having on the record
without moving the row's home.

## SC-824b — an archive mid-publication or mid-purge is refused outright

Bucket 1. Arms: `mid-publication` (rc=1) and `invalid-parent-validation-failing` (rc=1).
IS: `Error: archive 2814388f-602c-42e5-85da-9920f1e8e1aa is being published or purged
right now (…/.ae/archive/.publishing.2814388f-602c-42e5-85da-9920f1e8e1aa).` The refusal
names the specific lock file it saw, so the operator can confirm the claim independently
rather than trusting the diagnosis. `full-sweep.diff` for the validation-failing arm is
likewise 0 bytes — refused outright, nothing created.
**CONFIRMED.**

## SC-825a — the child records lineage durably

Bucket 2. Arm: `lineage-durability-stop-resume` (`stop_rc 0`; launch/resume rc are the
attach, per limit 1).
IS: `parent_archive_id=a3f54fee-fcf3-481c-a312-16648ad893f5` is present after `--from`,
after the stop, and after the resume. The arm captures `lineage.1after-from.txt`,
`lineage.2after-stop.txt`, `lineage.3after-resume.txt` and diffs the cycle: the
`parent_archive_id` **value is byte-identical**, and the arm additionally records an `od`
of that line, so byte-identity is measured rather than eyeballed. The only change across
the cycle is the line's position in the file.
**CONFIRMED.**

## SC-825b — the parent path is derived, never stored

Bucket 2. Arm: `lineage-durability-move-aehome`.
IS — the arm's construction is the proof: after the move, the lineage is read from
**`h2/.ae/sessions/child/meta`** (a different `AE_HOME` root than the `h/` it was written
under) and `parent_archive_id=9a325c6e-8f5c-4cfa-9591-69372cd9cab6` is intact and
byte-identical. Had the parent PATH been stored rather than derived from archive root + id,
moving `AE_HOME` would have rotted it. It did not.
**CONFIRMED.**

## SC-825c — a deleted parent warns and continues on resume

Bucket 2. Arm: `lineage-durability-delete-parent`.
IS — all three clauses of the row, each separately captured:
- **warns**: `note: parent archive 2a3a6787-… is no longer on this machine — the lineage
  is recorded, but its digest cannot be read.`
- **workspace.md says the digest is gone**: line 17 of the regenerated workspace carries
  `- NOTE: that digest is no longer on this machine — the archive was removed after this
  session was created.`
- **the lineage fact is still true, and it continues**: `parent_archive_id=2a3a6787-…`
  survives the parent's deletion across the resume, byte-identical.
Note the wording ae chose: *"the lineage is recorded, but its digest cannot be read"* —
it distinguishes the fact from the artifact, which is exactly the distinction the row
makes.
**CONFIRMED.**

## SC-826 — a pre-id session gets one minted at end, recorded on both sides

Bucket 2. Arm: `minted-at-end` (**rc=0** — the one arm in the section that is not
attach-limited).
IS: stdout `Minted a session id for 'leg' (it predates them):
0c2a9ea4-987b-45d4-982e-3ec7ee769dba` / `Archived 0c2a9ea4-…`, and **both** origin
records are present in the tree — `session_id_origin=minted-at-end` in the live meta and
`archive_id_origin=minted-at-end` in the archive. The row's "recorded on both sides" is
observed on both sides, not inferred from one.
**CONFIRMED.**

---

## Proposed dispositions

- **CONFIRMED / no change — 9**: SC-809, SC-822, SC-823, SC-824a, SC-824b, SC-825a,
  SC-825b, SC-825c, SC-826.
- **No PARTIAL, no DIVERGENCE, no reopened conflicts, no INCONCLUSIVE arms, no
  ARM-INVALID.**

**Section total: 9.** The second fully-converging section after L-PURGE (14/14).

## Why this section converged when L-STOP did not

Worth stating, because the difference is method and not luck:

1. **The producer applied the rc-attribution rule to itself first.** Limit 1 declares that
   every non-pty rc is the attach's, and every arm records created state separately. In
   L-STOP the same class of question had to be resolved row by row at classification time.
2. **The negatives are proved by exhaustive diff, not by inspection.** `full-sweep.diff`
   at 0 bytes says nothing anywhere changed; "we looked and saw nothing" says only that
   someone looked. Compare SC-835f in L-STOP, where an absent file had to stand in for a
   recorded absence.
3. **Evidence that fell into the section's lap was declined.** The `transport-cut` arm
   captured a product rollback matching SC-808 and did not bank it, because SC-808's arm
   lives elsewhere. A section that will not over-claim in its own favour is one whose
   confirmations mean something.
