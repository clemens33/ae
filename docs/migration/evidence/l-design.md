# Batch L designs — lifecycle destructive evidence (lead draft v2; six
# independently seat-gated sections, none approved yet)

Six batches over 94 assignments. Rosters are MACHINE-DECLARED below (`LROSTER:`
lines) and proven two-ways against crit-assign.md by
`./l-roster-check.sh` (five failure classes: missing / extra / dup /
unknown-batch / wrong-section; exits nonzero on any; red-proof demonstrated at
the gate). Sole-writer draft by fable5:lead. Each section gates and hands off
INDEPENDENTLY; a worker receives only gated sections.

LROSTER: L-END | SC-516 SC-800 SC-801 SC-802 SC-803 SC-806a SC-806b SC-807 SC-808 SC-811a SC-811b SC-812 SC-816 SC-817 SC-820a SC-821a SC-821b SC-830 SC-831 SC-838a SC-838b
LROSTER: L-PURGE | SC-804a SC-804b SC-804c SC-804d SC-804e SC-804f SC-805 SC-810a SC-810b SC-818b SC-818c SC-818d SC-818e SC-819
LROSTER: L-STOP | SC-515a SC-515b SC-515c SC-815a SC-815b SC-815c SC-815d SC-835a SC-835b SC-835c SC-835d SC-835e SC-835f SC-835g SC-835h SC-839a SC-839b SC-839c SC-839d SC-839e
LROSTER: L-COMPACT | SC-500 SC-501 SC-502 SC-503a SC-503b SC-504b SC-507a SC-507c SC-507d SC-508 SC-512 SC-517a SC-517b SC-517c SC-827 SC-828 SC-829a SC-829b SC-836 SC-837 SC-1305
LROSTER: L-FROM | SC-809 SC-822 SC-823 SC-824a SC-824b SC-825a SC-825b SC-825c SC-826
LROSTER: L-RENTRANS | SC-814 SC-832a SC-833a SC-1302 SC-1303 SC-1304a SC-1304b SC-1304c SC-1304d

Global rules, all binding, cited not restated: cluster-plan.md instrumentation
admissibility; the b0-design.md global artifact contract (per-arm records,
fresh fingerprinted sandbox per arm, bounded waits with
INCONCLUSIVE-on-timeout, allowlisted env logging, no verdicts); the
batch-c-design.md producer-derivation + date-shim contract. Fixtures build
from REAL frozen operations; mutations are named byte diffs.
**Value-blindness (gate v1 blocker, restored class-wide): worker-facing arms
below name CONSTRUCTION, MANIPULATION, BARRIERS, and RAW CAPTURES only — no
expected outputs, no survival/refusal/deletion language. Every expected
relation lives in the SEAT CLASSIFICATION ANNEX, never in a worker brief.**
No live models. Network: none, except L-RENTRANS's loopback-sshd construction
(below), which never leaves 127.0.0.1. Destructive arms run in DISPOSABLE
sandboxes.

**Failure-injection primitives (gate v1 important 1 — chmod alone is not
trusted):** where an arm needs an operation to be unable to proceed, it uses a
delegate-log-fail PATH shim (e.g. a `git` shim that delegates every subcommand
except `push`, which logs and exits nonzero) or an inability CANARY (before
relying on an unwritable path, the harness itself attempts the same class of
write and records the refusal; if the canary write SUCCEEDS the arm is INVALID,
recorded as such). chmod may still construct fixtures, but no arm's meaning
rests on chmod alone.

**Executable fakes:** any fake agent/tool whose pane identity matters is a
RENAMED COPY of a real interpreter executing a driver (the b0exec D8
measurement: an interpreted script surfaces as its interpreter in
`pane_current_command`; `exec -a` does not fix it).

Common captures per arm: stdout/stderr/rc of every ae invocation, recursive
before/after AE_HOME manifests, tmux snapshots per barrier, events.jsonl byte
deltas, archive-root recursive manifests, git state (local and the bare
remote) where the arm involves end's git phase.

## Section L-END (21) — end/archive transaction

Fixture modes are SPLIT (gate v1 blocker 3): a `--local` family for
claim/publication/identity/rerun arms, and a MANAGED family (real `--copy` /
`--worktree` launches with a local bare `file://` origin) for every arm whose
capture involves push outcomes, work-directory policy, or cleanup.

One instrumented copy carries barrier sites at end's phase boundaries (after
verified stop / after git outcome fixed / after capture staged / before
cleanup) — hooks block/emit only; the controller observes state at each
barrier.

Arms (fresh sandbox each):

1. **Baseline end (managed)** — full run; capture state at every barrier +
   final archive manifest + events + work-dir manifest.
2. **Push failure (managed)** — `git` delegate-log-fail shim failing only
   `push`; run end; capture rc, which barriers were reached, archive-root and
   live-dir manifests, the recorded push outcome field bytes.
3. **No-origin (managed)** — origin remote removed; same captures.
4. **Archive-write inability (managed + canary)** — archive root made
   unwritable AND the canary write recorded failing; run end; capture rc,
   live-dir manifest, archive-root manifest (SC-516 material).
5. **Standing claim** — pre-create `.publishing.<uuid>`; run end; capture
   output bytes and the claim dir's manifest before/after (SC-800/803).
6. **Publication crash cuts** — controller kills end at the pre-rename
   barrier; sibling arm after rename; capture archive-root manifests in both
   (SC-802).
7. **Staging modes** — capture the staging tree's recursive modes at the
   mid-staging barrier and the final tree's modes (SC-801).
8. **Identity pair** — end; recreate the SAME name; end again; capture both
   archive dir names + their meta id fields (SC-806a). **Writer-boundary
   uppercase arm (gate v1 blocker 3):** mutate a REAL LIVE session's meta
   UUID to uppercase (named byte diff), run end, capture the archive
   DIRECTORY name and the archive meta's id key bytes (SC-806b — the WRITER
   is the boundary under test, not a reader fed an uppercase archive).
9. **Compact-relaunch lock trace** — flock spy around the relaunch boundary
   (SC-807); controller mutates the parent archive between the child's proof
   and its publication; capture the resulting session/state manifests
   (SC-808).
10. **launch.sh rerun matrix** — the fake tool is a renamed-interpreter
    executable (rule above); run launch.<slot>.sh, exit, rerun; capture argv
    logged both times + `.started` marker state (SC-811a); ae rewrites the
    script (resume path); capture marker state (SC-811b); capture
    `pane_current_command` during the relaunched run (SC-812).
11. **Unreachable server** — recorded tmux socket dir removed; run end;
    capture per-target output, rc, state manifests (SC-816).
12. **end-all freeze/re-proof** — three sessions; controller renames one
    target's tmux session between confirmation and the lock; capture output
    bytes and post-state (SC-820a). **Empty-plan arm (gate v1 important 1):**
    all targets excluded at the prompt; capture the PROMPT TRANSCRIPT (the
    pty exchange proving the prompt ran) plus the frozen plan's recorded
    content and the outcome record (SC-821a/b).
13. **History policy matrix (managed)** — CLI flag x `[workspace]
    purge_agent_history` x unset, one arm per precedence cell; capture
    surviving conversation files + `end all`'s per-session decision lines
    (SC-838a/b).
14. **Handover pair** — a compact-driven end with `--digest-only` (capture
    the request states before/after — SC-830) and a handover under a
    shortened bound that expires (bounded poll, INCONCLUSIVE discipline;
    capture session/tmux/archive state + request state at expiry — SC-831).

Hostile constructions: an archive root that is a SYMLINK; a mid-staging
controller-planted unexpected entry. Captures only.

## Section L-PURGE (14) — purge inversion + validator taxonomy

Fixtures: REAL archives produced by real ends (L-END's specimen output feeds
this section), then per-arm single named mutations on COPIES.

Arms:

1. **No prior archive** — end --purge-history; capture archive-root
   before/after manifests (SC-810a).
2. **Existing archive** — session whose UUID has a real archive; end
   --purge-history; capture archive-root before/after (SC-810b).
3. **Validator taxonomy** — one named mutation per arm on a fresh copy:
   (a) one unexpected extra entry (SC-804a); (b) one symlink inside; FIFO
   sibling arm (SC-804b); (c) one directory mode 0755 (SC-804c); (d) one
   file mode 0644 (SC-804f); (e) exec bit USER-only / GROUP-only /
   OTHER-only, three subarms (SC-804d); (f) **two independent fresh clones**
   (gate v1 important 2): meta/digest id mismatch on one, count mismatch on
   the other (SC-804e). Each arm runs the purge-side validation AND a
   `--from` attempt; capture outputs + post-state manifests (SC-818c ties
   here).
4. **Execution-sentinel arm (gate v1 important 2)** — an archive member given
   exec bits + a shebang whose body writes a SENTINEL file outside the
   archive; run every archive-consuming operation in this section; capture
   the sentinel path's absence/presence + all outputs (SC-805).
5. **Symlinked root** — purge with the archive ROOT a symlink; capture output
   + both trees' manifests (SC-818a).
6. **Claim** — plant `.publishing.<uuid>`; purge; capture (SC-818b).
7. **Empty identity** — real archive, source-identity field emptied (named
   mutation); purge AND --from; capture both (SC-818d).
8. **Lineage-pointed parent** — child created via real --from; purge the
   parent's archive; capture (SC-818e).
9. **Unidentifiable session, TWO classes (gate v1 important 2)** — (a) meta
   removed entirely (memory intact); (b) meta present with an UNPARSEABLE
   session_id (named byte mutation) — distinct from the legacy MISSING-id
   mint path (SC-826, L-FROM's arm 6); run end with and without
   --purge-history on each; capture outputs + full manifests (SC-819).

## Section L-STOP (20) — stop matrix + fleet identity

Fixtures: multi-session fleets on dedicated servers; renamed-interpreter fake
agents; a prefix-sibling session pair present in every topology.

Arms:

1. **Plain stop** — delegating tmux shim traces every tmux argv (socket,
   target form); capture trace, post-state, file manifests (SC-835a/b/d).
2. **Unverifiable kill** — recorded server socket dir removed; capture rc,
   output, manifests (SC-835c).
3. **Self-stop** — controller drives the pane; arms with and without `-y`;
   capture the pty confirmation transcript, a ps-lineage trace of the
   supervisor process, events.jsonl deltas (SC-835e/f/g/h).
4. **C1–C5 planted singly** — five arms, one identity check failing each
   (outside tmux / foreign server / wrong recorded server / pane in another
   session / foreign controlling terminal); plus `--self` on the C5 cell and
   a malformed `--pane` token arm; capture each output + the shim's
   shell-reaching argv trace (SC-839a-d).
5. **Hostile-name injection arm (gate v1 important 3)** — sessions whose
   names carry quoting and command-substitution syntax with an embedded
   SENTINEL (e.g. a $(...) form whose execution would create a sentinel
   file), within the allowlisted-name limits ae accepts for existing
   sessions; run the no-name stop form; capture the shell-reaching argv
   trace AND the sentinel path's state (SC-839e).
6. **stop all fleet** — controller starts a FOURTH session during the
   confirmation window; capture the acted-on set + all outputs (SC-815a).
   Name-handoff arm: one confirmed target ended and recreated under the same
   name mid-op; capture final session states + per-target records (SC-815b).
   Concurrent-ops arm: two `stop all` runs, distinct op ids, controller
   barriers interleaving them; capture each run's consumed result set + event
   bytes (SC-815c/d).
7. **Exit folding** — one target's per-target record planted as a failure;
   capture the fold and rc (SC-515a). Results-timeout arm under a shortened
   bound; capture output + rc (SC-515b). Unowned-ae-tagged arm: a tmux
   session carrying the ae tag with no session dir; capture output +
   post-state (SC-515c).

## Section L-COMPACT (21) — compact/handover + preview + exits

Fixtures: real sessions with renamed-interpreter fake agents accepting real
sends; controller drives real `reply` for handover; roster preconditions
satisfied via the real retire helper (recorded actions).

Arms:

1. **Full compact baseline** — capture stdout BYTES, stderr BYTES separately
   (SC-500/501), stdout state AT the pre-relaunch barrier (SC-502).
   **Resolver traces named separately (gate v1 important 4):** the source
   trace distinguishes the RESOLVER ENTRY (the tuple-freeze site) from the
   two REVALIDATION SITES — three named trace channels, so tuple-resolution
   count and revalidation reads are independently visible (SC-827/828
   evidence).
2. **Recovery-command execution (gate v1 blocker 4)** — from the baseline's
   captured stdout, extract the printed `Recovery:` command; execute it
   VERBATIM on a fresh clone of the post-compact state; capture its full
   outcome (rc, resulting session state, tmux) — SC-512's specimen is the
   command RUNNING, not the archive existing.
3. **Interactive answers** — typed `n` arm; EOF arm (controller closes
   stdin); capture rc + post-state each (SC-503a/b). `-f` arm (SC-837).
4. **SIGPIPE supervision (gate v1 blocker 4)** — producer and early-closing
   consumer run as SEPARATELY SUPERVISED processes (no shell pipeline over
   the subject): the harness forks the consumer, connects an explicit pipe,
   closes the read end after one line; capture BOTH processes' statuses, the
   child's signal disposition record, and the relaunch state (SC-504b).
5. **Revalidation mismatches** — controller mutates the session at TWO
   barriers (after the human answer; after handover before teardown), one
   arm each; capture output bytes + which state changed (SC-828).
6. **Handover facts** — source trace of what completion polls; withholding
   arms: controller supplies only-reply, then only-memo; capture the wait
   state at the bound each time (SC-829a). Re-run arm: interrupt post-request,
   re-run; capture request events (count, refs) + the baseline bytes used
   (SC-829b).
7. **Config refusal** — `purge_agent_history` set; compact with and without
   `--keep-history`; capture (SC-836).
8. **Exit identity** — (a) relaunch reaching a terminal attach (pty-wrapped),
   then detach; capture rc propagation (SC-517a/b); (b) **attach-failure
   path (gate v1 blocker 4): the fresh session CREATES successfully but no
   terminal can attach** (launch driven with stdin/stdout not a tty —
   detached context); capture the report bytes + rc (SC-517c). An
   unlaunchable-binary arm is NOT this row's specimen.
9. **archive preview** — capture stdout bytes, stderr bytes, recursive
   manifest before/after, events delta (SC-507a/c/d). **Digest-pair
   construction (gate v1 blocker 4):** an IDENTICAL frozen twin of the
   session is ENDED (real end); capture its archived digest.md bytes
   alongside the preview stdout bytes — the seats compare; the worker only
   captures both.
10. **Residual exit codes** — the SC-508 capture-only rc table across every
    arm in this section.
11. **Mid-operation observability** — at each compact barrier, a concurrent
    `ae list --json` + `requests` invocation from a separate process; capture
    the observer outputs per cut (SC-1305, capture-only).

## Section L-FROM (9) — lineage

Fixtures: real parent archives from real ends (L-END specimen output).

Arms:

1. **Name never infers** — an archive whose source session name equals the
   new session's name; launch WITHOUT --from; capture meta lineage fields +
   workspace.md (SC-809).
2. **Existing-target refusals** — --from onto (a) a running tmux session,
   (b) a stopped session dir, (c) a leftover worktree; capture outputs + full
   before/after manifests (SC-822).
3. **Invalid-parent proof arm (gate v1 blocker 5)** — --from naming a
   nonexistent archive id, and a sibling arm naming a validation-failing
   archive (one named mutation); capture outputs + a FULL
   no-session/no-state/no-worktree manifest sweep after each (SC-823 — the
   proof-precedes-creation specimen; existing-target refusals are SC-822's,
   not this).
4. **Plain --from transport cut (gate v1 blocker 5, split)** — instrumented
   barrier after the parent proof; controller DELETES the parent archive;
   the launch continues; capture the child's recorded lineage fields + a
   source trace of parent-archive reads after the barrier (SC-824a). THIS ARM
   HAS NO ROLLBACK MACHINERY — it is the plain-launch path.
5. **Compact-child re-proof cut (gate v1 blocker 5, split)** — the SAME
   barrier shape inside a real COMPACT's child creation; controller mutates
   the parent archive between proof and publication; capture the child/session
   end-state manifests + outputs (SC-808's re-proof surface, captured here
   under L-FROM's fixture; artifacts kept separate from arm 4).
6. **Mid-publication parent** — plant `.publishing.<uuid>` on the parent;
   --from; capture (SC-824b).
7. **Lineage durability** — successful --from child; capture meta lineage
   fields across a stop/resume cycle (SC-825a); move AE_HOME wholesale,
   resume, capture behavior + field bytes (SC-825b); delete the parent
   archive, resume, capture output + workspace.md (SC-825c).
8. **Minted-at-end** — a legacy-shaped session with NO session_id key (named
   mutation, distinct from SC-819's unparseable class); end it; capture the
   live meta's origin field and the archive's origin field (SC-826).

## Section L-RENTRANS (9) — rename/transfer residue

**Hermetic loopback transport (gate v1 blocker 6 — the construction, then the
preflight):** a sandbox-local sshd bound to 127.0.0.1 on a random high port;
sandbox-HOME ssh config declaring a host ALIAS with that port; temporary host
and client keypairs + authorized_keys generated per sandbox; known_hosts
preseeded, StrictHostKeyChecking=yes; the sshd's ForceCommand wrapper sets a
separate remote HOME/PATH; the real user HOME is never referenced; no
interface but loopback. PRE-FLIGHT (blocking): drive the FROZEN `ae transfer`
end-to-end through this rig on a disposable session before any arm runs; if
the preflight fails, this SECTION is INCONCLUSIVE/BLOCKED and reported so — a
semantic ssh/rsync fake is never substituted.

**Deterministic mid-rsync cut (hook-only compliant):** the controller
pre-arranges the source with a deterministic file order and BOUNDED-POLLS THE
DESTINATION MANIFEST for the appearance of a named marker file, then kills the
rsync process (controller action; nothing in any shim counts or reads product
bytes). Cut position is a data property (which files precede the marker).

Arms:

1. **Endpoint validation, push AND pull subarms (gate v1 important 5)** —
   transfer invoked with a hostile target name in each direction; capture the
   ssh/rsync shim traces (invocation counts + argv) and both endpoints'
   manifests (SC-814).
2. **Rename effect set** — real rename on a running server; capture tmux
   state, dir manifests, meta bytes, workspace.md, server liveness (SC-832a).
3. **Rename mid-op observer** — barriers at the census-named cut points (dir
   moved / tmux renamed / meta updated); concurrent `ae list --json` at each;
   capture observer outputs per cut (SC-1303, capture-only).
4. **Same-name concurrency matrix (gate v1 important 5)** — the previously
   ruled stop/rename/transfer SAME-NAME matrix: each ordered pair of those
   three operations raced on ONE name under controller barriers, once with
   flock present, once with flock removed from PATH; capture lock traces +
   both operations' outputs + final state per cell (SC-1302 incumbent
   baseline).
5. **Transfer both directions** — stopped session pushed, then pulled back;
   capture full manifests on both endpoints + the conversation-file set
   (SC-833a).
6. **Transfer crash cuts** — the deterministic cut (above) applied: push-side
   after stop completes (SC-1304a), push-side mid-destination-write
   (SC-1304b), pull-side analogues (SC-1304c/d); capture both endpoints'
   manifests at each cut.

## Execution shape

END + PURGE first (END produces the real-archive specimens PURGE and FROM
consume; a section handoff includes its specimen outputs). STOP independent.
COMPACT after END's fake-agent pattern is proven. RENTRANS last (transport
preflight gates it). One worker may take several gated sections sequentially;
sections never share sandboxes. Artifacts under `l-artifacts/<section>/` with
per-section MANIFEST.md in the b0-design global-contract shape.

---

## SEAT CLASSIFICATION ANNEX — never included in any worker brief

- **L-END**: barrier order stop → git-outcome-fixed → capture → cleanup
  (SC-817); the push-shim arm returns BEFORE capture with no archive; the
  no-origin arm archives with push_outcome=no-origin and a preserved work
  dir; archive-write inability = nonzero exit with the live session intact
  (SC-516); claim refusal names the claim, claim dir untouched (SC-800/803);
  pre-rename kill leaves no archive, post-rename a complete one (SC-802);
  staging 0700/0600 under umask 077 (SC-801); same-name pair yields two
  UUID-keyed archives (SC-806a); the uppercase-mutated LIVE id is written
  CANONICAL LOWERCASE by the archive writer (SC-806b); lock released before
  relaunch; parent-mutation rolls the child back (SC-807/808); rerun resumes
  via marker, rewrite clears it, pane shows the tool (SC-811a/b/812);
  unreachable server carried + loud per-target failure (SC-816); freeze
  mismatch refusal prints both versions (SC-820a); the empty plan proves
  prompt-ran + ended-nothing, distinct from never-asked (SC-821a/b);
  CLI > config > keep (SC-838a); per-session decision lines (SC-838b);
  digest-only withdraws outstanding requests (SC-830); expired handover:
  nothing stopped, nothing archived, request still open (SC-831).
- **L-PURGE**: every taxonomy mutation FAILS validation and deletes nothing
  (804a-f; exec bit fails for u, g, AND o; id and count mismatches each fail
  on their own clone); purge writes no archive (810a) and deletes the
  existing one (810b); symlinked root refused (818a); claim blocks (818b);
  unvalidatable tree refused with the remove-it-yourself direction (818c);
  empty identity refused as malformed and --from will not inherit (818d);
  lineage-pointed parent refused (818e); BOTH unidentifiable classes refused
  BEFORE stop with nothing deleted, regardless of history flag (819); the
  execution sentinel must NEVER appear — no consumer executed archive content
  (805).
- **L-STOP**: exact recorded server + session id, never a name — the
  prefix-sibling survives every arm (835a); stopped only after verified gone
  (835b); unverifiable = loud, no state change (835c); nothing deleted
  (835d); self-stop confirms with recoverability wording, supervisor
  out-of-pane, durable stop-result event (835e/g/h); -y skips (835f);
  --self waives C5 only (839a); malformed --pane refused on shape (839b);
  refusals name the failed check (839d); the hostile-name sentinel must
  never appear and no tmux-expanded text reaches a shell string (839e);
  confirmed fleet only — the fourth session survives (815a); the name
  newcomer survives with a recorded name-changed-hands failure (815b);
  own-results only; [op <uuid>] visible (815c/d); per-target results fold
  into the exit (515a); results-timeout reports and is not a failure (515b);
  unowned ae-tagged sessions are named, not stopped (515c).
- **L-COMPACT**: stdout = the four lines in order, empty unless the boundary
  was crossed (500); stderr carries everything else (501); Recovery: present
  BEFORE the relaunch fires (502); the extracted Recovery command WORKS on
  the fresh clone — that execution is SC-512's proof; typed n answers, EOF
  does not (503a/b); both supervised statuses show no leaked SIGPIPE
  disposition (504b); the resolver-entry trace shows ONE tuple resolution
  while the two revalidation-site traces show their permitted reads — a raw
  read count is NOT the discriminator (827/828); completion = reply AND
  fresh handover memo, polled from log+memo.tsv never panes — each
  withholding arm must still be waiting at its bound (829a); the re-run
  reuses the SAME request and its stored baseline (829b); purge-config
  refuses without --keep-history (836); -f skips confirmation (837); exit is
  the launch's; terminal case attaches and exits on detach (517a/b); the
  created-but-unattachable case reports as plain `ae <name>` (517c);
  preview stdout equals the twin's end-produced digest bytes; diagnostics on
  stderr; read-only with zero events (507a/c/d); SC-508's rc table and
  SC-1305's observer views are classified at closure.
- **L-FROM**: lineage only via explicit --from (809); existing-target
  refusals leave prior state untouched (822); the invalid-parent arms prove
  proof-precedes-creation: no session, no state, no worktree after refusal
  (823); plain --from records proved facts and performs NO parent re-read
  after the barrier — the deleted parent does not disturb the child (824a);
  the compact child DOES re-prove and rolls back on mismatch (SC-808's
  surface, arm 5); mid-publication parent refused outright (824b); lineage
  durable across resume (825a); parent path derived, not stored — AE_HOME
  move does not rot it (825b); deleted parent warns and continues (825c);
  minted-at-end recorded on BOTH sides (826).
- **L-RENTRANS**: both endpoint names validated before ANY side effect in
  BOTH directions — zero ssh/rsync invocations on refusal (814); rename's
  four effects with the server up (832a); transfer moves the stopped session
  both directions including conversation files (833a); the same-name matrix
  serializes under flock and silently disables without it — the bucket-3
  incumbent baseline (1302); 1303/1304a-d observer/residue views are
  capture-only code-observation rows classified at closure; 1304a's gate
  precision — source PRESENT (not byte-intact), no destination write yet.
