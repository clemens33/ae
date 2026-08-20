# Joint L classification — L-END worksheet (section 1 of 6)

Seats: fable5:lead (author) + gpt56sol:colead (independent read). Request
ae-20260820T175445Z-d607f1c2; preflight conditions adopted: every pointer reaches the
claim-bearing RAW capture(s); empirical acceptance and normative `classified_by` are
ORTHOGONAL — no row marks merely because its capture matches; any mismatch routes
through fix-known-defect or a DR, never measurement-over-contract.

**STATUS: CONVERGED 2026-08-20** — colead's independent per-id read is folded in
below; every correction they made is applied in place and attributed. All 21 ids
accounted for: 19 CONFIRMED, 1 REOPENED (SC-820a → #98, filed), 1 PARTIAL (SC-821a,
colead dissent sustained).

Evidence base: L-END committed at 7aab1b4, corrected at d89f0d8 (SC-808 arm re-run —
see its entry). All artifact paths below are under
`docs/migration/evidence/l-artifacts/L-END/arms/`.

`verified` column: **bytes** = lead read the named raw captures directly; **record** =
lead's IS statement rests on the arm's recorded artifacts + manifest facts (colead's
independent read is the second eye either way). Section-wide: zero INCONCLUSIVE, zero
ARM-INVALID, zero preflight failures (MANIFEST "Known limits" states the five
recorded limits; none is an INCONCLUSIVE).

---

**SC-516** — end fails non-zero when the archive cannot be written; capture-then-delete
(b1, conflict none). Artifacts: `archive-write-inability/2end.rc` (=1), `2end.stderr`,
`canary.txt` (root 0500, canary mkdir REFUSED, euid 501), `3post.livedir.tsv`.
IS: rc=1; stderr verbatim: "archiving session 'aw' failed — the session is STOPPED and
NOTHING was deleted. Fix the cause reported above, then re-run: ae end aw"; live dir
intact in post manifest. Verified: bytes. **Proposed: CONFIRMED / no change.**

**SC-800** — publication claims by `mkdir`; mkdir failure IS the mutual exclusion (b1,
none). Artifacts: `claim/claimdir.before.tsv`/`.after.tsv`, `2end.rc` (=1),
`2end.stderr.od`. IS: with a pre-created `.publishing.<uuid>` for the session's own
uuid, end fails rc=1; the claim dir is untouched after. Verified: bytes (rc+stderr
decode; claim-dir manifests record). **Proposed: CONFIRMED / no change.**

**SC-801** — staging is private by construction (b1, none). Artifacts:
`staging-modes/b04-b_stage_mid.*.staging-payload.tsv` + `.staging-claim.tsv`,
`final-archive.tsv` (JOINT DECODE, colead: claim/payload dirs 0700, payload files
0600, final archive same); hostile companion `staging-modes-planted-entry/*.planted.txt` +
payload manifests before/after plant + `2end.stderr`. IS: mid-staging manifests record
the modes of every staged entry; the planted-entry arm records what a foreign file in
staging does to the run. Verified: record + colead joint decode. **CONFIRMED / no change (both seats).**

**SC-802** — the final archive appears complete or not at all (b1, none). Artifacts:
`publication-crash-cut-pre_rename/b05-*.archive-at-cut.tsv` + `final-archive.tsv` +
`2end.rc` (=137); `publication-crash-cut-post_rename/b06-*` same set (rc=137).
IS: SIGKILL at pre-rename leaves NO archive at the target (staging only); at
post-rename the complete archive exists (with the standing claim — that state is the
L-PURGE specimen). No partial target-path tree in either. Verified: record.
**Proposed: CONFIRMED / no change.**

**SC-803** — a standing claim is refused and NAMED, never cleaned (b1, none).
Artifacts: `claim/2end.stderr.od` (decoded), `claimdir.after.tsv`. IS verbatim from
the decode: "archive: another publisher holds <root>/.publishing.8f4a3aae-… (or a
previous run crashed holding it). ae will not guess-clean it. Inspect it, then remove
it by hand if it is stale." — claim named, not cleaned, end fails loud. Verified:
bytes. **Proposed: CONFIRMED / no change.**

**SC-806a** — archive identity is the session UUID, never the name (b1, none).
Artifacts: `identity-same-name-twice/archive-identity.txt`, `meta.run1.txt`,
`meta.run2.txt`, `archive.after-end1.tsv`/`.after-end2.tsv`. IS: the same name ended
twice yields two distinct UUID-keyed archives, both addressable. Verified: record.
**Proposed: CONFIRMED / no change.**

**SC-806b** — canonical lowercase key; legacy uppercase normalized (b2, none).
Artifacts: `identity-uppercase-meta-uuid/mutation.txt` (+`meta.mutation.diff`, A-F
fold, mode 644→644 recorded post-correction), `archive-identity.txt` (od bytes).
IS: with the live session meta's `session_id` case-folded to uppercase, the published
archive directory name is all-lowercase (`ae326a1d-…`, od-verified) and the meta id
keys are recorded as exact bytes beside it. Verified: bytes (directory name),
record (meta id line). Colead adds: `archive_id`/path are lowercase while
`source_session_id` PRESERVES the uppercase input — normalization is on the key, not
a rewrite of the recorded source. **CONFIRMED / no change (both seats).**

**SC-807** — the lifecycle lock is released before the relaunch (b1, none).
Artifacts: `compact-relaunch-lock-control/flock-spy.log`, per-barrier
`*.flock-spy.snapshot.log`, `*.ps.txt`, `lineage.txt`, `barrier-order.tsv`.
IS — EVIDENCE REWRITTEN per colead's raw read (my first statement over-claimed):
`flock-spy.log` records fd ACQUISITIONS but NOT lock paths, so the log alone does NOT
prove "the same lock name". The admissible proof is COMPOSITE: frozen source
ae:6352-6390 (the parent's lock block ends) + ae:6458 (exec into the child) +
ae:17292-17295 (the child opens and acquires the same name), together with the
RUNTIME fact that the child reached `b_from_proved` (it could not have, holding a
still-held lock). Retained limit: nothing here timestamps the release or names its
mechanism (explicit unlock vs fd close at exec/exit). Verified: record + colead
source read. **CONFIRMED / no change (both seats)**, with this composite statement —
not the log alone — carried into the Empirical column.

**SC-808** — the child re-proves the exact parent archive before publishing (b1,
none). Artifacts (CORRECTED ARM — provenance is part of this entry):
`compact-relaunch-lock-parent-mutated/b07-*.parent-meta.{before,after}.txt`, `.diff`
(single named line `handover_count 0→7`), `b07-*.parent-meta.mode.txt`
(mode.before=600 mode.after=600), `b07-*.controller.txt`, `lineage.txt`,
`3post.sessions.tsv`. Correction provenance: the 7aab1b4 captures of THIS ARM are
INADMISSIBLE (unnamed umask-644 mode side effect on an archive meta the frozen
validator asserts 600 on; L-END/MANIFEST.md "Correction (post-commit 7aab1b4)");
the arm was re-run with the mode-preserving mutator and the d89f0d8 captures are the
evidence. IS: with the parent archive meta mutated at the barrier AFTER the child's
first parent proof, the outcome recorded is the parent archive failing validation
while the child was being created (rc/lineage per arm files). Verified: record.
**Proposed: CONFIRMED / no change** — the re-prove-before-publish invariant is what
the arm exercises; joint read of `lineage.txt` + rc at convergence.

**SC-811a** — launch script: first run creates, later runs resume; `.started` decides
(b2, none). Artifacts: `launch-rerun/marker-timeline.txt` (script sha, marker
existence/size/mtime, invocation count at seven labels),
`fake-argv-both-runs.txt` (every execution's argv, NUL-separated), execution ledger
mapping (execution 1 = ae's own, frozen ae:12606; controller runs are 2 and 3).
IS: the marker timeline + per-execution argv show creation vs resume argv per
execution. Verified: record. **Proposed: CONFIRMED / no change.**

**SC-811b** — ae clears the marker whenever it rewrites the script (b2, none).
Artifacts: `launch-rerun/launch-script.rewrite.diff`, `marker-timeline.txt` labels
around the stop+resume rewrite (execution 4). IS: the rewrite is captured with the
marker state before/after in the timeline. Verified: record. **Proposed: CONFIRMED /
no change.**

**SC-812** — the resume decision happens BEFORE exec (b1, none). **My original IS
statement here was FALSE and is withdrawn** (colead caught it; lead re-read the bytes
and confirms): both `launch-rerun/1first-run.pane_current_command.txt` and
`3second-run.pane_current_command.txt` read `%0|<pid>|bash|0` — `bash`, NOT the tool
name. The fixture tool is a renamed bash copy, so these pane files are
FIDELITY-LIMITED and prove nothing either way about fallback retention; they must not
be cited for this row. (Note also: L-END's section report claims
`pane_current_command` positively records the tool name per arm — that claim does not
hold for this arm's captures; relayed to lexec.) Admissible empirical instead
(colead): the generated `launch.main.sh.0after-launch` BYTES showing the branch
decision followed by an explicit `exec` on BOTH paths, plus the frozen source.
Verified: bytes (the falsification), record (the replacement pointer).
**CONFIRMED / no change (both seats)** on the normative row; empirical rests on the
script bytes, with the pane limitation recorded.

**SC-816** — an unverifiable session is still a target (b1, none). Artifacts:
`unreachable-server/manipulation.txt`, `socketdir.before/after.tsv`, `2end.stderr`
(single, rc=1), `3endall.stderr` (all, rc=1), `3post.aehome.tsv`. IS: with the
recorded socket's directory removed, single end fails loud rc=1; `end all` carries
the session and fails loud rather than counting it gone; on-disk state survives per
post manifest. Verified: record. **Proposed: CONFIRMED / no change.**

**SC-817** — transaction order: verified stop → git outcome FIXED and RECORDED →
capture → cleanup (b1, none). Artifacts: `transaction-order-a-full-run/
barrier-order.tsv` + `archive-meta-fields.txt`; `-b-push-fails/git-shim.log`
(INJECTED-FAILURE sub=push rc=128) + `2end.stderr` (rc=1);
`-c-no-origin/archive-meta-fields.txt` + `3post.workdir.tsv`.
IS (bytes): barrier order is confirm → b_stop_git → b_git_fixed → b_stage_mid →
b_pre_rename → b_post_rename → b_pre_cleanup; arm a records
`git_push_outcome=pushed` + `git_final_commit=<sha>`; arm c (no origin) STILL
archives, records `git_push_outcome=no-origin` + a real `git_final_commit`, rc=0,
work preserved; arm b (push refused) rc=1. Verified: bytes.
**Proposed: CONFIRMED / no change.**

**SC-820a** — end freezes the confirmed plan and re-proves it under the lock;
re-proof REFUSES on mismatch and prints both versions (b1, none). Artifacts:
`endall-rename-between-confirm-and-lock/b01-*.tmux.before-rename.txt` /
`.after-rename.txt`, `b01-*.controller.txt`, `2endall.stdout.od`, `2endall.stderr.od`
(EMPTY), `post-state.txt`. IS (bytes, od decoded): one confirmed target's tmux
session was renamed `ef2`→`ef2-renamed` after the answer and before the lock;
`end all` then printed "- ef2: archive -> …/04bdce1f-… · conversation files KEPT",
"Cleaned up local session ef2", "Ended local session ef2" — NO refusal, NO
both-versions print, stderr EMPTY — while `post-state.txt` shows tmux session
`ef2-renamed` STILL ALIVE after the run. The on-disk session was archived and
cleaned while its live (renamed) tmux session survived as an orphan.
Verified: bytes. **REOPENED CONFLICT — BOTH SEATS CONCUR; issue #98 FILED; SC-820a
reclassified bucket 3 fix-known-defect(#98), SHOULD unchanged, intended = refuse the
mismatched target + print both versions + act on nothing for it.** The capture
contradicts the
ratified SHOULD (documented at commands.md:526-532 + architecture.md:146-149):
the frozen re-proof did not refuse, did not print versions, and produced exactly the
torn ended-on-disk / alive-in-tmux state the freeze contract exists to prevent,
silently. Proposed route per schema: bucket 3 fix-known-defect(new issue — sixth
live defect — now #98). SHOULD unchanged.

**SC-821a** — `end all` acts on the confirmed target set only; the set can never grow
(b1, none). Artifacts: `endall-empty-plan/frozen-plan-as-rendered.txt`,
`2endall.pane.at-prompt.txt`, `3post.aehome.tsv`. IS: the rendered frozen plan and
the prompt are captured; with an empty enumeration nothing is acted on.
Verified: record. **PARTIAL — colead DISSENT SUSTAINED, lead concurs.** The
never-GROW half is NOT proven by either available arm: the empty plan adds nothing
after the answer, and SC-820a's arm renames a tmux session while fleet targets are
enumerated from session STATE DIRECTORIES — neither construction can fail a product
that re-enumerates and picks up a newly created session. What IS evidenced: the
confirmed set was not exceeded in these runs. Empirical stays PENDING/partial until a
dedicated arm creates a real session/state-dir AFTER the confirmation and captures
whether it enters the acted-on set. Queued for L-FROM/L-RENTRANS-era scheduling or a
targeted L-END addendum arm.

**SC-821b** — "a prompt ran" is its own fact, never a count (b1, none). Artifacts:
`endall-empty-plan/2endall.pane.at-prompt.txt`, `2endall.stdout.od`, `0pre.dirs.txt`.
IS: against zero sessions, the captured pane/stdout show how the frozen product
distinguishes end-NOTHING from nobody-was-asked (joint read of the decoded stdout at
convergence). Verified: record. **Proposed: CONFIRMED / no change**, pending the
joint decode.

**SC-830** — `--digest-only` is the one explicit degradation; withdraws outstanding,
digest is the handover (b2, none). Artifacts: `handover/requests.2before-digest-only
.txt` vs `.3after-digest-only.txt`, `events.<label>.jsonl`, `post-archive.txt`
. IS — CORRECTED per colead's raw read: `--digest-only` WITHDREW the same request,
emitted the cancel, archived the digest as the handover, and CROSSED the
archive/source-removal boundary — the degradation itself completed. The command's
rc=1 comes from the SEPARATE relaunch afterwards failing with `open terminal failed:
not a terminal` (a harness-environment fact), and must NEVER be attributed to the
degradation. My earlier "the compact completes" phrasing is deleted as imprecise.
Verified: record + colead decode. **CONFIRMED / no change (both seats), no conflict.**

**SC-831** — a timed-out handover stops nothing; the request stays OPEN so a re-run
waits on the SAME request (b1, none). Artifacts: `handover/requests.0pre.txt`,
`requests.1at-expiry.txt`, `1at-expiry.sessiondir.tsv`, `events.*.jsonl`; bounded
wait AE_COMPACT_HANDOVER_SECS=8, expiry by construction (no reply, no memo; MANIFEST
Known limits: the product's own reported outcome is what is recorded, no absence
inferred). IS: at expiry the session dir is intact (nothing stopped/archived per
manifest) and the request state at expiry is captured; same-request persistence is
readable from requests.1 vs requests.2 ids. Verified: record. **Proposed: CONFIRMED
/ no change.**

**SC-838a** — history policy precedence CLI > session config > keep (b2, none).
Artifacts: nine cells `history-policy-c{1..9}-*/conversations.{before,after}.tsv` +
`.diff`, `planted-conversations.txt`, `2endall.stdout` (decision lines),
`2endall.pane.at-prompt.txt`; all rc=0. Conversation files are controller-planted
path markers (frozen locator matches PATH only, ae:10769 — no content fabricated).
IS (bytes for c1): cell c1 (no CLI flag, config unset) prints "conversation files
KEPT" per session and the planted files survive in `conversations.after.tsv` — the
KEEP default; the 3×3 cross's per-cell diffs record which cells delete.
Verified: bytes (c1), record (c2-c9). **Proposed: CONFIRMED / no change**, with the
full 3×3 outcome table read jointly at convergence.

**SC-838b** — `end all` lists both decisions per session, one line each (b2, none).
Artifacts: same cells, `2endall.stdout` + `2endall.pane.at-prompt.txt`. IS (bytes,
c1): per-session line "  - hp1: archive -> <path> · conversation files KEPT" — the
archive decision and the conversation decision on one line per session.
Verified: bytes (c1). **Proposed: CONFIRMED / no change.**

---

## Non-roster hostile constructions (captures only; no row proposed)

- `hostile-symlinked-archive-root` (rc=1): archive root replaced by an out-of-tree
  symlink; `2end.stderr` + link-target manifests before/after. Feeds S9 residue
  rulings / SC-818a context in L-PURGE; no L-END row claims it.
- `staging-modes-planted-entry`: foreign file+dir planted mid-staging; `2end.stderr`
  + payload manifests. Read jointly under SC-801/SC-802 at convergence if wanted.

## Converged dispositions (both seats, 2026-08-20)

- **CONFIRMED / no change — 19:** SC-516, SC-800, SC-801, SC-802, SC-803, SC-806a,
  SC-806b, SC-807 (composite proof), SC-808 (d89f0d8 supersession), SC-811a, SC-811b,
  SC-812 (empirical re-pointed; pane files fidelity-limited), SC-816, SC-817,
  SC-821b, SC-830 (IS corrected), SC-831, SC-838a, SC-838b.
- **REOPENED CONFLICT — 1:** SC-820a → bucket 3 fix-known-defect(#98), FILED; SHOULD
  unchanged; row reclassified and marked by both seats.
- **PARTIAL — 1:** SC-821a — the set-can-never-GROW half needs a dedicated
  post-confirmation session-creation arm; Empirical stays pending/partial.
- Fidelity notes carried: SC-808 (d89f0d8 supersedes 7aab1b4 for that arm), SC-807
  (composite proof; release timing/mechanism unprovable here), SC-812 (pane
  fidelity limit recorded).
- No INCONCLUSIVE arms in section.

### Lead corrections accepted from colead's independent read
Three of my IS statements were wrong or over-claimed and are fixed in place:
SC-807 (flock log has no lock paths — composite source proof required), SC-812 (pane
files say `bash`, not the tool — statement withdrawn), SC-830 (rc=1 belongs to the
relaunch, not the degradation). Recording them rather than quietly editing: the
worksheet is the durable record of the joint session, including who caught what.

Marks: NONE proposed here. Empirical acceptance per id lands as Empirical-column
updates citing this worksheet + the raw pointers after per-id convergence; any S9
normative marks remain separate countersigned batches per the standing process.
