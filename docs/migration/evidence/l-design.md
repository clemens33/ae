# Batch L designs — lifecycle destructive evidence (lead draft v3; six
# independently seat-gated sections, none approved yet)

Six batches over 94 assignments. Coverage is MACHINE-DECLARED: `LROSTER:` lines
assert each section's roster, `LARM:` lines bind each arm to the critical ids it
PRIMARY-owns (`| ref:` ids are typed non-roster safety controls, never counted
as coverage). `./l-roster-check.sh` proves, from those declarations against
crit-assign.md: roster == assignment set both ways, every roster id has exactly
one primary arm, no arm owns an id outside its roster, no id is primary twice,
and every primary id actually appears in the design BODY (the body-erasure
catch — a declaration table alone gates nothing). Six failure classes, each
demonstrated RED at the gate. Sole-writer draft by fable5:lead; each section
gates and hands off INDEPENDENTLY.

LROSTER: L-END | SC-516 SC-800 SC-801 SC-802 SC-803 SC-806a SC-806b SC-807 SC-808 SC-811a SC-811b SC-812 SC-816 SC-817 SC-820a SC-821a SC-821b SC-830 SC-831 SC-838a SC-838b
LROSTER: L-PURGE | SC-804a SC-804b SC-804c SC-804d SC-804e SC-804f SC-805 SC-810a SC-810b SC-818b SC-818c SC-818d SC-818e SC-819
LROSTER: L-STOP | SC-515a SC-515b SC-515c SC-815a SC-815b SC-815c SC-815d SC-835a SC-835b SC-835c SC-835d SC-835e SC-835f SC-835g SC-835h SC-839a SC-839b SC-839c SC-839d SC-839e
LROSTER: L-COMPACT | SC-500 SC-501 SC-502 SC-503a SC-503b SC-504b SC-507a SC-507c SC-507d SC-508 SC-512 SC-517a SC-517b SC-517c SC-827 SC-828 SC-829a SC-829b SC-836 SC-837 SC-1305
LROSTER: L-FROM | SC-809 SC-822 SC-823 SC-824a SC-824b SC-825a SC-825b SC-825c SC-826
LROSTER: L-RENTRANS | SC-814 SC-832a SC-833a SC-1302 SC-1303 SC-1304a SC-1304b SC-1304c SC-1304d

Global rules, binding, cited not restated: cluster-plan.md instrumentation
admissibility; b0-design.md global artifact contract; batch-c-design.md
producer-derivation + date-shim contract. Fixtures build from REAL frozen
operations; mutations are named byte diffs. **Value-blindness (gate v1/v2
blocker — de-oracled twice now): worker arms name CONSTRUCTION, MANIPULATION,
BARRIERS, RAW CAPTURES only. No relation words (refusal, survival, deletion,
continues, precedes). Every expected relation is in the SEAT CLASSIFICATION
ANNEX.** No live models. Network: none except L-RENTRANS's loopback sshd (never
leaves 127.0.0.1). Destructive arms run in DISPOSABLE sandboxes.

Failure-injection primitives (chmod alone is not trusted): a delegate-log-fail
PATH shim (a `git` shim delegating all but `push`, which logs+exits nonzero) or
an inability CANARY (the harness first attempts the same write class and records
its refusal; a canary write that SUCCEEDS marks the arm INVALID). Executable
fakes are RENAMED COPIES of a real interpreter (the b0exec D8 measurement: an
interpreted script surfaces as its interpreter in `pane_current_command`).
Per-consumer separate clones: where an arm feeds a specimen to more than one
destructive consumer, EACH consumer gets its own fresh clone, so a first
consumer that destroys the tree cannot make the second test something else.

Common captures per arm: stdout/stderr/rc of every ae invocation, recursive
before/after AE_HOME manifests, tmux snapshots per barrier, events.jsonl byte
deltas, archive-root recursive manifests, git state where end's git phase runs.

## Section L-END (21) — end/archive transaction

LARM: L-END | transaction-order | SC-817
LARM: L-END | archive-write-inability | SC-516
LARM: L-END | claim | SC-800 SC-803
LARM: L-END | staging-modes | SC-801
LARM: L-END | publication-crash-cuts | SC-802
LARM: L-END | identity | SC-806a SC-806b
LARM: L-END | compact-relaunch-lock | SC-807 SC-808
LARM: L-END | launch-rerun | SC-811a SC-811b SC-812
LARM: L-END | unreachable-server | SC-816
LARM: L-END | endall-freeze | SC-820a SC-821a SC-821b
LARM: L-END | history-policy | SC-838a SC-838b
LARM: L-END | handover | SC-830 SC-831

Fixture modes SPLIT: a `--local` family for claim/publication/identity/rerun
arms; a MANAGED family (real `--copy`/`--worktree` launches, local bare
`file://` origin) for arms whose captures involve push, work-dir policy, or
cleanup. One instrumented copy carries barriers at end's phase boundaries
(after verified stop / after git outcome fixed / after capture staged / before
cleanup) — hooks block/emit only; the controller observes state at each.

- **transaction-order (managed, SC-817)** — three constructed inputs on their
  own sandboxes: (a) full run; (b) `git` delegate-log-fail shim failing only
  `push`; (c) origin remote removed. Capture barrier-by-barrier state, the
  recorded push-outcome field bytes, archive-root + work-dir manifests, rc.
- **archive-write-inability (managed + canary, SC-516)** — archive root made
  unwritable with the canary recorded refusing; run end; capture rc, live-dir
  and archive-root manifests.
- **claim (SC-800, SC-803)** — pre-create `.publishing.<uuid>`; run end;
  capture output bytes and the claim dir manifest before/after.
- **staging-modes (SC-801)** — capture the staging tree's recursive modes at
  the mid-staging barrier and the final tree's modes.
- **publication-crash-cuts (SC-802)** — controller kills end at the pre-rename
  barrier; sibling arm after rename; archive-root manifests in both.
- **identity (SC-806a, SC-806b)** — end; recreate the SAME name; end again;
  capture both archive dir names + meta id fields (SC-806a). Writer-boundary
  arm: mutate a REAL LIVE session's meta UUID to uppercase (named byte diff),
  run end, capture the archive DIRECTORY name and the archive meta id key
  bytes (SC-806b — the WRITER is under test).
- **compact-relaunch-lock (SC-807, SC-808)** — flock spy around the relaunch
  boundary (SC-807); controller mutates the parent archive between the child's
  proof and publication; capture the session/state manifests (SC-808). This is
  SC-808's PRIMARY arm; L-FROM references it, never re-runs it.
- **launch-rerun (SC-811a, SC-811b, SC-812)** — renamed-interpreter fake tool;
  run launch.<slot>.sh, exit, rerun; capture argv both times + `.started`
  marker (SC-811a); ae rewrites the script; capture marker (SC-811b); capture
  `pane_current_command` during the relaunched run (SC-812).
- **unreachable-server (SC-816)** — recorded tmux socket dir removed; run end;
  capture per-target output, rc, manifests.
- **endall-freeze (SC-820a, SC-821a, SC-821b)** — three sessions; controller
  renames one target's tmux session between confirmation and the lock; capture
  output bytes + post-state (SC-820a). Empty-plan arm: all targets excluded at
  the prompt; capture the pty PROMPT TRANSCRIPT + the frozen plan's recorded
  content + the outcome record (SC-821a/b).
- **history-policy (managed, SC-838a, SC-838b)** — CLI flag x `[workspace]
  purge_agent_history` x unset, one arm per cell (each cell its own clone);
  capture surviving conversation files + `end all` per-session decision lines.
- **handover (SC-830, SC-831)** — a compact-driven end with `--digest-only`
  (capture request states before/after — SC-830); a handover under a shortened
  bound (bounded poll, INCONCLUSIVE discipline; capture session/tmux/archive +
  request state at expiry — SC-831).

Hostile constructions: symlinked archive root; a mid-staging controller-planted
unexpected entry. Captures only.

## Section L-PURGE (14) — purge inversion + validator taxonomy

LARM: L-PURGE | no-prior-archive | SC-810a
LARM: L-PURGE | existing-archive | SC-810b
LARM: L-PURGE | validator-taxonomy | SC-804a SC-804b SC-804c SC-804d SC-804e SC-804f SC-818c
LARM: L-PURGE | execution-sentinel | SC-805
LARM: L-PURGE | claim | SC-818b
LARM: L-PURGE | empty-identity | SC-818d
LARM: L-PURGE | lineage-parent | SC-818e
LARM: L-PURGE | unidentifiable | SC-819 | ref: SC-818a

Fixtures: REAL archives. The "existing archive" specimen is NAMED as L-END's
publication-crash-cut output (post-rename/pre-cleanup) — product-produced, not
hand-fabricated. Per-consumer separate clones are MANDATORY here: every arm
that runs BOTH the purge path and a `--from` attempt uses TWO fresh clones of
the same mutation (a broken purge that deletes the tree must not turn the
`--from` test into "missing archive").

- **no-prior-archive (SC-810a)** — end --purge-history, no existing archive;
  capture archive-root before/after.
- **existing-archive (SC-810b)** — session whose UUID has the L-END-produced
  archive; end --purge-history; capture before/after.
- **validator-taxonomy (SC-804a-f, SC-818c)** — one named mutation per arm on
  fresh copies: (a) an unexpected extra entry (SC-804a); (b) a symlink inside,
  FIFO sibling (SC-804b); (c) a directory mode 0755 (SC-804c); (d) a file mode
  0644 (SC-804f); (e) exec bit USER/GROUP/OTHER, three subarms (SC-804d);
  (f) id-mismatch on one clone AND count-mismatch on a second independent clone
  (SC-804e). Each mutation drives the purge-side validation on ONE clone and a
  `--from` attempt on a SEPARATE clone; capture outputs + post-state manifests
  (SC-818c).
- **execution-sentinel (SC-805)** — an archive member given exec bits + a
  shebang whose body would write a SENTINEL outside the archive; run EACH
  archive-consuming operation on ITS OWN clone; capture the sentinel path state
  + outputs.
- **claim (SC-818b)** — plant `.publishing.<uuid>`; purge; capture.
- **empty-identity (SC-818d)** — real archive, source-identity emptied (named
  mutation); purge on one clone, `--from` on another; capture both.
- **lineage-parent (SC-818e)** — child via real --from; purge the parent's
  archive; capture.
- **unidentifiable (SC-819; ref SC-818a)** — two classes: (a) meta removed
  (memory intact); (b) meta present with an UNPARSEABLE session_id (named byte
  mutation), distinct from the legacy MISSING-id mint path (SC-826); end with
  and without --purge-history on each; capture outputs + full manifests. The
  symlinked-archive-ROOT construction (SC-818a, ALREADY-OBSERVED — a non-roster
  safety control, ref only) runs here as a control: purge with the root a
  symlink; capture — it is NOT a coverage arm.

## Section L-STOP (20) — stop matrix + fleet identity

LARM: L-STOP | plain-stop | SC-835a SC-835b SC-835d
LARM: L-STOP | unverifiable-kill | SC-835c
LARM: L-STOP | self-stop | SC-835e SC-835f SC-835g SC-835h
LARM: L-STOP | identity-checks | SC-839a SC-839b SC-839c SC-839d
LARM: L-STOP | legacy-migration-injection | SC-839e
LARM: L-STOP | fleet | SC-815a SC-815b SC-815c SC-815d
LARM: L-STOP | exit-folding | SC-515a SC-515b SC-515c

Fixtures: multi-session fleets on dedicated servers; renamed-interpreter fakes;
a prefix-sibling pair in every topology.

- **plain-stop (SC-835a/b/d)** — delegating tmux shim traces every tmux argv (SC-835a, SC-835b, SC-835d);
  capture trace, post-state, file manifests.
- **unverifiable-kill (SC-835c)** — recorded server socket dir removed; capture
  rc, output, manifests.
- **self-stop (SC-835e, SC-835f, SC-835g, SC-835h)** — controller drives the
  pane; arms with/without `-y`; capture the pty confirmation transcript, a
  ps-lineage trace of the supervisor, events deltas.
- **identity-checks (SC-839a-d)** — C1-C5 planted singly (outside tmux /
  foreign server / wrong recorded server / pane in another session / foreign
  controlling terminal); plus `--self` on the C5 cell and a malformed `--pane` (SC-839b, SC-839c, SC-839d)
  token arm; capture each output + the shim's shell-reaching argv trace.
- **legacy-migration-injection (SC-839e)** — quotes and `$()` are OUTSIDE the
  name allowlist, so there is no allowlisted spelling: start a VALID real
  session, then a named controller mutation into the LEGACY physical
  direct-child shape (tmux rename + matching real state-dir/meta move, argv-safe
  throughout) whose name carries quoting/command-substitution syntax with an
  embedded SENTINEL; re-prove C1-C4; run the implicit no-name stop route;
  capture the shell-reaching argv trace + the sentinel path state. Identified as
  the legacy-migration arm, never an allowlisted launch.
- **fleet (SC-815a-d)** — controller starts a FOURTH session during the
  confirmation window; capture the acted-on set (SC-815a). Name-handoff arm: one
  confirmed target ended and recreated under the same name mid-op; capture final
  states + per-target records (SC-815b). Concurrent-ops arm: two `stop all`
  runs, distinct op ids, controller barriers interleaving; capture each run's
  consumed results + event bytes (SC-815c, SC-815d).
- **exit-folding (SC-515a-c)** — one target's per-target record planted as a
  failure; capture the fold + rc (SC-515a). Results-timeout arm, shortened
  bound; capture output + rc (SC-515b). Unowned-ae-tagged arm: a tmux session
  carrying the ae tag with no session dir; capture output + post-state
  (SC-515c).

## Section L-COMPACT (21) — compact/handover + preview + exits

LARM: L-COMPACT | baseline | SC-500 SC-501 SC-502 SC-827
LARM: L-COMPACT | recovery-exec | SC-512
LARM: L-COMPACT | interactive | SC-503a SC-503b SC-837
LARM: L-COMPACT | sigpipe | SC-504b
LARM: L-COMPACT | revalidation | SC-828
LARM: L-COMPACT | handover-facts | SC-829a SC-829b
LARM: L-COMPACT | config-refusal | SC-836
LARM: L-COMPACT | exit-identity | SC-517a SC-517b SC-517c
LARM: L-COMPACT | preview | SC-507a SC-507c SC-507d
LARM: L-COMPACT | residual-rc | SC-508
LARM: L-COMPACT | mid-op | SC-1305

Fixtures: real sessions with renamed-interpreter fakes accepting real sends;
controller drives real `reply`; roster preconditions via the real retire
helper.

- **baseline (SC-500, SC-501, SC-502, SC-827)** — capture stdout BYTES, stderr
  BYTES separately (SC-500/501), stdout state AT the pre-relaunch barrier
  (SC-502). Three NAMED trace channels: the RESOLVER ENTRY (tuple-freeze site)
  distinct from the two REVALIDATION SITES, so one authoritative resolution and
  the permitted revalidation reads are separable — a raw meta-read count is not
  the discriminator (SC-827).
- **recovery-exec (SC-512)** — capture a clone taken AFTER archive publication
  and source removal but BEFORE the relaunch (a post-relaunch clone already
  holds the replacement session, so the printed command would correctly refuse
  under SC-822 and prove nothing); extract the printed `Recovery:` command;
  execute it VERBATIM on that pre-relaunch clone; capture its full outcome.
- **interactive (SC-503a, SC-503b, SC-837)** — typed `n` arm; EOF arm
  (controller closes stdin); capture rc + post-state (SC-503a/b). `-f` arm
  (SC-837).
- **sigpipe (SC-504b)** — producer and early-closing consumer as SEPARATELY
  SUPERVISED processes (no shell pipeline over the subject): fork the consumer,
  explicit pipe, close the read end after one line; capture BOTH statuses, the
  child's signal disposition, the relaunch state.
- **revalidation (SC-828)** — controller mutates the session at TWO barriers
  (after the human answer; after handover before teardown), one arm each;
  capture output bytes + which state changed.
- **handover-facts (SC-829a, SC-829b)** — source trace of what completion
  polls; withholding arms: controller supplies only-reply, then only-memo;
  capture the wait state at the bound (SC-829a). Re-run arm: interrupt
  post-request, re-run; capture request events (count, refs) + baseline bytes
  used (SC-829b).
- **config-refusal (SC-836)** — `purge_agent_history` set; compact with and
  without `--keep-history`; capture.
- **exit-identity (SC-517a, SC-517b, SC-517c)** — (a) relaunch reaching a
  terminal attach (pty-wrapped), then detach; capture rc propagation
  (SC-517a/b); (b) the fresh session CREATES but no terminal can attach —
  invoked with `-f` (so it does not exit at confirmation EOF, SC-503b) and
  stdin/stdout not a tty; capture the report bytes + rc (SC-517c). An
  unlaunchable-binary arm is NOT this row's specimen.
- **preview (SC-507a, SC-507c, SC-507d)** — capture stdout bytes, stderr bytes,
  recursive manifest before/after, events delta.
- **residual-rc (SC-508)** — the capture-only rc table across every arm.
- **mid-op (SC-1305)** — at each compact barrier, a concurrent `ae list --json`
  + `requests` from a separate process; capture observer outputs per cut.

## Section L-FROM (9) — lineage

LARM: L-FROM | name-never-infers | SC-809
LARM: L-FROM | existing-target | SC-822
LARM: L-FROM | invalid-parent | SC-823
LARM: L-FROM | transport-cut | SC-824a | ref: SC-808
LARM: L-FROM | mid-publication | SC-824b
LARM: L-FROM | lineage-durability | SC-825a SC-825b SC-825c
LARM: L-FROM | minted-at-end | SC-826

Fixtures: real parent archives from real ends.

- **name-never-infers (SC-809)** — an archive whose source session name equals
  the new session's name; launch WITHOUT --from; capture meta lineage fields +
  workspace.md.
- **existing-target (SC-822)** — --from onto (a) a running tmux session, (b) a
  stopped session dir, (c) a leftover worktree; capture outputs + full
  before/after manifests.
- **invalid-parent (SC-823)** — --from naming a nonexistent archive id, and a
  sibling naming a validation-failing archive (one named mutation); capture
  outputs + a FULL no-session/no-state/no-worktree manifest sweep after each.
- **transport-cut (SC-824a; ref SC-808)** — instrumented barrier after the
  parent proof; controller deletes the parent archive; resume the launch;
  capture the child's recorded lineage fields + a source trace of
  parent-archive reads after the barrier. This is the PLAIN launch path and has
  NO re-proof/rollback machinery; SC-808's re-proof surface is L-END's
  compact-relaunch-lock arm (referenced, not re-executed here).
- **mid-publication (SC-824b)** — plant `.publishing.<uuid>` on the parent;
  --from; capture.
- **lineage-durability (SC-825a, SC-825b, SC-825c)** — successful --from child;
  capture meta lineage fields across a stop/resume cycle (SC-825a); move AE_HOME
  wholesale, resume, capture behavior + field bytes (SC-825b); delete the parent
  archive, resume, capture output + workspace.md (SC-825c).
- **minted-at-end (SC-826)** — a legacy-shaped session with NO session_id key
  (named mutation, distinct from SC-819's unparseable class); end it; capture
  the LIVE meta's `session_id_origin` value and the ARCHIVE's `archive_id_origin`
  value by EXACT KEY (never the repository `origin=`).

## Section L-RENTRANS (9) — rename/transfer residue

LARM: L-RENTRANS | endpoint-validation | SC-814
LARM: L-RENTRANS | rename-effects | SC-832a
LARM: L-RENTRANS | rename-observer | SC-1303
LARM: L-RENTRANS | samename-matrix | SC-1302
LARM: L-RENTRANS | transfer-both | SC-833a
LARM: L-RENTRANS | crash-cut-poststop | SC-1304a SC-1304c
LARM: L-RENTRANS | crash-cut-midwrite | SC-1304b SC-1304d

**Hermetic loopback transport + BLOCKING preflight (both directions):** a
sandbox-local sshd bound to 127.0.0.1 on a random high port; sandbox-HOME ssh
config with a host ALIAS on that port; per-sandbox host+client keypairs +
authorized_keys; known_hosts preseeded, StrictHostKeyChecking=yes; the sshd
ForceCommand wrapper sets a separate remote HOME/PATH; no real user HOME, no
interface but loopback. The preflight drives FROZEN `ae transfer` end-to-end
through this rig, PUSH and PULL, on disposable sessions before any arm; failure
= this SECTION is INCONCLUSIVE/BLOCKED, never a semantic ssh/rsync fake.

**Two crash-cut constructions (B6):** (i) POST-STOP barrier for SC-1304a/c — a
hook fires immediately after stop completion and BEFORE any destination
mkdir/rsync; the state is: stop complete, source present, no destination write.
(ii) MID-WRITE cut for SC-1304b/d — the controller bounded-polls the
destination manifest for a named marker and kills the rsync process, VALID only
while rsync is demonstrably still alive and the destination demonstrably
incomplete (marker appearance alone can race a finished rsync); nothing in any
shim counts or reads product bytes.

- **endpoint-validation (SC-814)** — transfer with a hostile target name, PUSH
  and PULL subarms (path construction/mkdir occur on opposite endpoints);
  capture the ssh/rsync shim traces (invocation counts + argv) and both
  endpoints' manifests.
- **rename-effects (SC-832a)** — real rename on a running server; capture tmux
  state, dir manifests, meta bytes, workspace.md, server liveness.
- **rename-observer (SC-1303)** — barriers at the census-named cut points (dir
  moved / tmux renamed / meta updated); concurrent `ae list --json` at each;
  capture observer outputs per cut.
- **samename-matrix (SC-1302)** — the previously ruled stop/rename/transfer
  SAME-NAME concurrency matrix: each ordered pair of those three operations
  raced on ONE name under controller barriers, once with flock present, once
  with flock removed from PATH; capture lock traces + both operations' outputs +
  final state per cell.
- **transfer-both (SC-833a)** — stopped session pushed, then pulled back;
  capture full manifests on both endpoints + the conversation-file set.
- **crash-cut-poststop (SC-1304a, SC-1304c)** — the POST-STOP barrier applied
  push-side (SC-1304a) and pull-side (SC-1304c); capture both endpoints'
  manifests at the cut.
- **crash-cut-midwrite (SC-1304b, SC-1304d)** — the MID-WRITE cut applied
  push-side (SC-1304b) and pull-side (SC-1304d); capture both endpoints'
  manifests at the cut.

## Execution shape

END + PURGE first (END produces the real-archive specimens PURGE and FROM
consume; a section handoff includes its specimen outputs). STOP independent.
COMPACT after END's fake-agent pattern is proven. RENTRANS last (transport
preflight gates it). One worker may take several gated sections sequentially;
sections never share sandboxes. Artifacts under `l-artifacts/<section>/`.

---

## SEAT CLASSIFICATION ANNEX — never included in any worker brief

- **L-END**: barrier order stop → git-outcome-fixed → capture → cleanup
  (SC-817); the push-shim input returns BEFORE capture with no archive; the
  no-origin input archives with push_outcome=no-origin and a preserved work
  dir; archive-write inability = nonzero exit, live session intact (SC-516);
  claim refusal names the claim, claim dir untouched (SC-800/803); pre-rename
  kill leaves no archive, post-rename a complete one (SC-802); staging
  0700/0600 under umask 077 (SC-801); same-name pair yields two UUID-keyed
  archives (SC-806a); the uppercase-mutated LIVE id is written CANONICAL
  LOWERCASE by the archive writer (SC-806b); lock released before relaunch;
  parent-mutation rolls the child back (SC-807/808); rerun resumes via marker,
  rewrite clears it, pane shows the tool (SC-811a/b/812); unreachable server
  carried + loud per-target failure (SC-816); freeze mismatch refusal prints
  both versions (SC-820a); the empty plan proves prompt-ran + ended-nothing,
  distinct from never-asked (SC-821a/b); CLI > config > keep (SC-838a);
  per-session decision lines (SC-838b); digest-only withdraws outstanding
  requests (SC-830); expired handover: nothing stopped, nothing archived,
  request still open (SC-831).
- **L-PURGE**: every taxonomy mutation FAILS validation and deletes nothing
  (804a-f; exec bit fails for u, g, AND o; id and count mismatches each fail on
  their own clone); on each shared mutation the purge clone and the --from
  clone fail INDEPENDENTLY (neither result depends on the other's tree); purge
  writes no archive (810a), deletes the existing one (810b); symlinked root
  refused (818a, control); claim blocks (818b); unvalidatable tree refused with
  the remove-it-yourself direction (818c); empty identity refused as malformed,
  --from will not inherit (818d); lineage-pointed parent refused (818e); BOTH
  unidentifiable classes refused BEFORE stop, nothing deleted, regardless of
  history flag (819); the execution sentinel never appears for any consumer
  (805).
- **L-STOP**: exact recorded server + session id, never a name — the
  prefix-sibling survives every arm (835a); stopped only after verified gone
  (835b); unverifiable = loud, no change (835c); nothing deleted (835d);
  self-stop confirms with recoverability wording, supervisor out-of-pane,
  durable stop-result (835e/g/h); -y skips (835f); --self waives C5 only
  (839a); malformed --pane refused on shape (839b); refusals name the failed
  check (839d); in the legacy-migration arm the hostile sentinel never appears
  and no tmux-expanded text reaches a shell string (839e); confirmed fleet
  only — the fourth session survives (815a); the name newcomer survives with a
  recorded name-changed-hands failure (815b); own-results only, [op <uuid>]
  visible (815c/d); per-target results fold into the exit (515a);
  results-timeout reports, is not a failure (515b); unowned ae-tagged named,
  not stopped (515c).
- **L-COMPACT**: stdout = the four lines in order, empty unless the boundary
  was crossed (500); stderr carries the rest (501); Recovery: present BEFORE
  the relaunch (502); the extracted Recovery command run on the pre-relaunch
  clone SUCCEEDS — that execution is SC-512's proof (a post-relaunch clone
  would refuse under SC-822 and prove nothing); typed n answers, EOF does not
  (503a/b); both supervised statuses show no leaked SIGPIPE disposition (504b);
  the resolver-entry trace shows ONE tuple resolution while the two
  revalidation traces show their permitted reads — a raw read count is NOT the
  discriminator (827/828); completion = reply AND fresh handover memo, polled
  from log+memo.tsv never panes — each withholding arm still waiting at its
  bound (829a); the re-run reuses the SAME request and its stored baseline
  (829b); purge-config refuses without --keep-history (836); -f skips (837);
  exit is the launch's; terminal attaches and exits on detach (517a/b); the
  created-but-unattachable `-f` case reports as plain `ae <name>` (517c);
  preview stdout equals the twin's end-produced digest ONLY after normalizing
  the two authority-named volatile lines — `Archived at: pending` and `Push
  outcome: preview-not-run` (ae:4947-50, commands.md:562-563) — which end
  writes with real values; the RAW preview values of those two lines are
  asserted SEPARATELY as the preview's own contract; diagnostics on stderr;
  read-only with zero events (507a/c/d); SC-508's rc table and SC-1305's
  observer views classified at closure.
- **L-FROM**: lineage only via explicit --from (809); existing-target refusals
  leave prior state untouched (822); the invalid-parent arms prove
  proof-precedes-creation: no session, no state, no worktree after refusal
  (823); plain --from records proved facts and performs NO parent re-read after
  the barrier — the deleted parent does not disturb the child (824a); SC-808's
  re-prove-and-rollback is L-END's arm, referenced here for the contrast;
  mid-publication parent refused outright (824b); lineage durable across resume
  (825a); parent path derived, not stored — AE_HOME move does not rot it
  (825b); deleted parent warns and continues (825c); minted-at-end recorded on
  BOTH sides via the exact keys session_id_origin (live) and archive_id_origin
  (archived) (826).
- **L-RENTRANS**: both endpoint names validated before ANY side effect in BOTH
  directions — zero ssh/rsync invocations on refusal (814); rename's four
  effects with the server up (832a); transfer moves the stopped session both
  directions including conversation files (833a); the same-name matrix
  serializes under flock and silently disables without it — the bucket-3
  incumbent baseline (1302); 1303/1304a-d observer/residue views are
  capture-only code-observation rows classified at closure; SC-1304a/c (the
  post-stop cut): source PRESENT (not byte-intact), no destination write yet;
  SC-1304b/d (the mid-write cut): partial/mixed destination while rsync was
  still alive.
