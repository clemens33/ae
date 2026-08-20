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
IS — **corrected; my first reading quoted the wrong file** (colead's finding, verified).
`name-never-infers/child-lineage.txt` shows the new session's meta carrying `origin`,
`session` and `session_id` and **no `parent_archive_id` KEY AT ALL**. The name matched an
existing archive exactly and no lineage was attached — the key is simply absent.
*My error:* I reported `parent_archive_id=-`, which came from `parent-archive-meta.txt` —
the PARENT ARCHIVE's own meta, describing the parent's (absent) parent. **Absence is the
correct incumbent observation; a sentinel value must not be invented for it.**
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
IS: each arm takes a FULL sweep of session dirs, worktrees, tmux sessions, the archive
root, `AE_HOME` and the work dir before and after, and diffs them: **`full-sweep.diff` is
0 bytes in BOTH arms** (pre and post captures byte-identical at 395 bytes / 28 lines in
the nonexistent arm).
**COMPOSITE (colead's finding, adopted) — and my temporal overclaim is DELETED.** A zero
diff proves the **POSTCONDITION** (no residue survives the refusal). It does **not** prove
the row's **TEMPORAL** headline "*proved BEFORE anything is created*", because a
**create-then-perfectly-roll-back mutant produces the same zero diff**. I wrote "proved
exhaustively… the whole observable surface was compared and nothing moved", which is true
about residue and silent about ordering — the strongest-sounding sentence in the worksheet
was measuring the wrong thing.
The ORDERING comes from frozen source, ae:16974-16998, where the parent proof precedes any
creation. The tree manifests remain strong direct evidence of the body's postcondition.
**CONFIRMED AS COMPOSITE (frozen source ordering + byte-identical tree manifests).**

## SC-824a — proof facts are recorded as proved, never re-read

Bucket 1. Arm: `transport-cut` — the PLAIN launch path
(`ae --local child --from <parent>`), with an instrumented barrier immediately after the
launch parses its FIRST parent proof, where the controller deletes the parent archive.
**RULED: PARTIAL — the arm cannot prove this row, and my evidence came from a DIFFERENT
ARM** (colead's BLOCKER, verified). `transport-cut/child-lineage.txt` reads literally
`(no meta at that path)` and `(no workspace.md)`: the run re-proves, rolls back and
**deletes the child**, so there is no recorded tuple to inspect. The barrier snapshot is
taken *before* meta construction, so it cannot show one either.
*My error:* the counts I quoted came from `lineage-durability-delete-parent`, which is
SC-825c's arm. I attributed cross-arm evidence to a row whose own arm holds none.
**The SEMANTIC is supported by frozen source** — the first tuple is assigned at
ae:16987-16988 and written at ae:17575-17580, and the second proof at ae:17613-17626 never
substitutes its own tuple — so the row may close as CODE-COMPOSITE. **Deletion cannot
prove it.**
**CLOSED AS COMPOSITE by L-DISCRIM D1** — with the scope colead required, because D1
does not close the whole row on its own.
D1 built exactly the discriminator specified: parent at handover=2/pending=1, replaced at
the barrier after the first proof with a VALID archive **at the same id** carrying
handover=5/pending=3. A control ran first — `--from` against the replacement ALONE records
5/3 — so the arm could have shown 5/3. The child recorded **2/1**.
**What D1 proves DIRECTLY: the COUNTS come from the first proof.**
**What it does NOT prove: the ID half.** The replacement holds the archive id CONSTANT by
construction, so a mutant that re-reads the id after the barrier while retaining the proved
counts emits byte-identical artifacts. The id half rides frozen source (tuple assigned
ae:16987-16988, written ae:17575-17580, second proof at ae:17613-17626 never substituting
its own) plus the earlier deletion construction.
**CONFIRMED AS COMPOSITE (D1 runtime for the counts + frozen source for the id).** Any
citation must carry both halves.
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

Bucket 1. Arm: `mid-publication` (rc=1). **`invalid-parent-validation-failing` is NOT a
second arm for this row** (colead's finding, verified): its mutation is
`handover_count=0` → `handover_count=99`, a digest/meta VALIDATION mismatch, not a
mid-publication or mid-purge state.
IS: `Error: archive 2814388f-602c-42e5-85da-9920f1e8e1aa is being published or purged
right now (…/.ae/archive/.publishing.2814388f-602c-42e5-85da-9920f1e8e1aa).` The refusal
names the specific lock file it saw, so the operator can confirm the claim independently
rather than trusting the diagnosis.
**COMPOSITE.** The planted `.publishing.<id>` claim is one exact runtime state **shared by
publish and purge**, so a single capture cannot separate the row's two named conditions;
frozen source maps BOTH operations to that same claim. The row closes on the refusal
capture **plus both claim-writer source paths**, not on two-arm direct evidence.

## SC-825a — the child records lineage durably

Bucket 2. Arm: `lineage-durability-stop-resume` (`stop_rc 0`; launch/resume rc are the
attach, per limit 1).
**CLOSED by L-DISCRIM D2** (seat-verified). D2 ran the cycle with counts asserted
NON-ZERO and DISTINCT (handover=2, pending=1) before proceeding, and both counts plus the
exact id survive `--from` → stop → resume. The fixture that could not discriminate is
replaced by one that can: its first attempt FAILED the non-zero assertion and wrote
ARM-INVALID rather than passing a zero through. **CONFIRMED.**
*The original finding, kept for the record:* **Every captured `parent_archive_handover_count` and
`parent_archive_pending_count` in this section is 0**, so an implementation that LOSES
both and defaults them to zero passes all three captures identically. The arm cannot fail
the count half of the claim.
To close it: a real parent with **non-zero** handover and pending counts, then the
stop/resume cycle.
The ID half is proven and stands:
`parent_archive_id=a3f54fee-fcf3-481c-a312-16648ad893f5` is present after `--from`,
after the stop, and after the resume. The arm captures `lineage.1after-from.txt`,
`lineage.2after-stop.txt`, `lineage.3after-resume.txt` and diffs the cycle: the
`parent_archive_id` **value is byte-identical**, and the arm additionally records an `od`
of that line, so byte-identity is measured rather than eyeballed. The only change across
the cycle is the line's position in the file.
**CONFIRMED.**

## SC-825b — the parent path is derived, never stored

Bucket 2. Arm: `lineage-durability-move-aehome`.
**CONFIRMED WITH A NORMATIVE PRECISION** (colead's finding, verified). An absolute parent
digest path **IS materialized** — `workspace.md` renders
`…/h/.ae/archive/9a325c6e-…/digest.md` before the move and
`…/h2/.ae/archive/9a325c6e-…/digest.md` after it. So "never stored" is too broad as
written; what is never persisted as lineage STATE is an absolute parent-path **KEY** in
meta (grepped: no `*path*=` key exists there).
**Adopted precision:** *"persistent lineage state stores the parent id and counts, never
an absolute parent path; rendered paths are derived from the current archive root plus the
id."*
The move arm then confirms exactly that, and confirms it well: the RENDERED path tracked
`h` → `h2` while the stored id stayed byte-identical. A stored path would have rotted; a
derived one followed the root. That contrast is the proof.

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

## Dispositions — POST-GATE

*Colead's independent read moved SEVEN of the nine, including two BLOCKERs, and in two
cases showed I had quoted evidence from the wrong FILE or the wrong ARM. This section
does **not** converge. My pre-gate worksheet claimed 9/9 and was wrong.*

*Updated after L-DISCRIM: both PARTIALs are closed by purpose-built discriminators, and
the section now converges. The route mattered — neither closed by re-reading the original
captures; each needed an arm that could produce the unwanted answer.*

- **CONFIRMED, direct — 5**: SC-809, SC-822, SC-825c, SC-826, **SC-825a** (closed by
  L-DISCRIM D2: counts asserted non-zero AND distinct before the arm proceeds, then
  surviving `--from` → stop → resume).
- **CONFIRMED AS COMPOSITE — 3**: SC-823 (frozen ordering + tree manifests), SC-824b
  (refusal capture + both claim-writer source paths), **SC-824a** (L-DISCRIM D1 runtime
  for the COUNTS + frozen source for the ID — D1 holds the archive id constant by
  construction, so it cannot discriminate that half).
- **CONFIRMED WITH A NORMATIVE PRECISION — 1**: SC-825b — persistent lineage state stores
  the parent id and counts, never an absolute parent path; rendered paths are derived from
  the current archive root plus the id.
- **PARTIAL — 0.**

**Section total: 9.**

---

## What this section changed about how we read rows

My pre-gate version of this heading claimed the section converged and explained why. It
did not converge, and two of the three reasons I gave were themselves wrong. Replaced with
what actually happened:

1. **A zero diff proves a postcondition, never an ordering** (SC-823). I called
   `full-sweep.diff` at 0 bytes "the strongest negative in batch L… the whole observable
   surface was compared and nothing moved". True — and silent about the row's actual
   temporal claim, because a **create-then-perfectly-roll-back mutant produces the same
   zero diff**. The most confident sentence in the worksheet was measuring the wrong thing.
2. **All-zero captures cannot discriminate** (SC-825a). Every handover and pending count
   in this section is 0, so "the counts were preserved" and "the counts were lost and
   defaulted to zero" are indistinguishable. A fixture whose values are all the type's
   default is a fixture that cannot fail.
3. **A recursive grep that strips filenames returns a value with no provenance.** Both of
   my factual errors came from `grep -rh … <arm>/`: SC-809's `parent_archive_id=-` was
   really from `parent-archive-meta.txt` (the PARENT's meta), and SC-824a's counts were
   really from `lineage-durability-delete-parent` (a DIFFERENT row's arm). I used `-h` to
   get clean output, and that flag suppressed exactly the field that would have caught
   both. **Third instrument error of the day, same shape as the checksum glob and the
   roster grep: a cheap probe that answers plausibly is the one that does not get
   re-checked.**
4. **Not every arm listed against a row is evidence for it** (SC-824b). The
   validation-failing mutation is `handover_count=0 → 99`, a digest/meta mismatch — not a
   mid-purge state. Two arms cited is not two arms proving.
5. **A producer's discipline still stands.** The rc-attribution limit declared in the
   manifest, and the `transport-cut` arm declining to bank SC-808 evidence that fell into
   its lap, were both real and both survived the gate. What did not survive was my reading
   of what the captures proved.
