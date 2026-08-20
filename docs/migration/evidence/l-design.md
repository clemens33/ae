# Batch L designs — lifecycle destructive evidence (lead draft v1; SIX
# independently seat-gated sections, none approved yet)

Six batches over 94 assignments, rosters EXACTLY the crit-assign sets (verify
before any spawn: `grep '| L-<NAME> |' crit-assign.md` must equal each section's
roster — the checker's canonical invocation from evidence/ proves assignment
integrity). Sole-writer draft by fable5:lead. Each section gates and hands off
INDEPENDENTLY; a worker receives only gated sections.

Global rules, all binding, cited not restated: cluster-plan.md instrumentation
admissibility (one hook-only patch per instrumented copy, barrier/ordinal-only
hooks, per-fixture inactive equivalence, controller performs mutations,
product-visible-path equivalence, trace segregation); the b0-design.md global
artifact contract (per-arm records, fresh fingerprinted sandbox per arm, bounded
waits with INCONCLUSIVE-on-timeout, allowlisted env logging, no verdicts);
batch-c-design.md producer-derivation + date-shim contract. Fixtures build from
REAL frozen operations (ae launches, generated helpers, real compact/end/stop
runs); mutations are named byte diffs. Value-blindness: every arm below is
manipulation + barrier + capture; ALL expected relations live in the SEAT
CLASSIFICATION ANNEX at the end, never in a worker brief. No live models, no
network. Destructive arms run in DISPOSABLE sandboxes — nothing touches the live
checkout, real AE_HOME, or any running session.

Common captures per arm (in addition to design-specific ones): stdout/stderr/rc
of every ae invocation, recursive before/after AE_HOME manifests, tmux
server/session/pane/client snapshots per barrier, events.jsonl byte deltas,
archive-root recursive manifests, git log/status of the work dir where the arm
involves end's git phase.

## Section L-END (21) — end/archive transaction

Roster: SC-516 800 801 802 803 806a 806b 807 808 811a 811b 812 816 817 820a
821a 821b 830 831 838a 838b.

Fixture base: real `ae --local` sessions with fake agents (harness-slice fake
per cexec's step-zero pattern, composed-UI banner), a real git repo work dir
with an origin remote that is a LOCAL bare repo (file:// — no network), events
and memos produced by real helpers.

One instrumented copy carries barrier sites at end's phase boundaries (after
verified stop / after git outcome fixed / after capture staged / before
cleanup) — hooks block/emit only; the controller observes state at each barrier
(tmux liveness, git log of the bare remote, staging tree manifest, live dir
presence).

Arms (fresh sandbox each):

1. **Baseline end** — full run, barriers crossed in sequence; capture state at
   every barrier (SC-817 ordering evidence) + final archive manifest + events.
2. **Failing push** — the bare remote made unwritable (chmod) before end;
   capture where the run stops, rc, live dir state, whether any archive exists
   (SC-817's git-outcome-fixed leg).
3. **No-origin managed** — work dir with no origin remote; capture rc, archive
   presence, recorded push_outcome value, work-dir preservation (SC-817).
4. **Unwritable archive root** — archive root made unwritable after stop
   completes; capture rc (nonzero expected per crate contract — captured, not
   asserted by the worker), live session dir state (SC-516).
5. **Standing claim** — pre-create `.publishing.<uuid>` in the archive root;
   run end; capture the refusal text and that the claim dir is untouched
   (SC-800/803).
6. **Publication crash cuts** — controller kills the end process at the
   pre-rename barrier (staged, not yet renamed) and, in a sibling arm, after
   rename; capture archive-root manifests in both (SC-802).
7. **Staging privacy** — capture the staging tree's modes at the mid-staging
   barrier (SC-801) plus the final tree's modes.
8. **Archive identity** — end a session whose NAME was reused (end, recreate
   same name, end again); capture both archive dir names and their meta id
   fields (SC-806a). Legacy-uppercase arm: a fixture archive with an uppercase
   id key processed by a reader op (preview/--from); capture normalization
   (SC-806b).
9. **Compact-relaunch lock trace** — during a real compact (shared with
   L-COMPACT's fixture where practical, but run here on its own sandbox), the
   delegating flock spy records lock acquire/release around the relaunch
   boundary (SC-807); the child's parent-archive re-proof is exercised by a
   controller mutation of the parent archive between proof and publication —
   capture the rollback state (SC-808).
10. **launch.sh rerun matrix** — for an upfront-UUID fake tool: run
    launch.<slot>.sh, exit the tool, rerun the script; capture argv the fake
    logged both times + the `.started` marker's presence (SC-811a); have ae
    rewrite the script (resume path) and capture the marker state (SC-811b);
    capture `pane_current_command` during the relaunched run (SC-812).
11. **Unreachable server** — session meta records a tmux server whose socket
    dir is removed; run `end` (and the fleet arm in L-STOP mirrors this);
    capture whether it is carried, the per-target log text, rc (SC-816).
12. **end-all freeze/re-proof** — `end all` over three sessions; controller
    renames one target's tmux session between confirmation and the lock;
    capture the refusal text (both versions printed?) and the surviving
    sessions (SC-820a). Empty-confirmation arm: all sessions excluded at the
    prompt; capture the outcome record distinguishing ended-nothing from
    never-asked (SC-821a/b).
13. **History policy matrix** — CLI flag vs `[workspace] purge_agent_history`
    vs unset, one arm per precedence cell; capture which conversation files
    survive + the per-session decision lines of `end all` (SC-838a/b).
14. **Handover degradation pair** — a compact-driven end with `--digest-only`
    (capture what is withdrawn, SC-830) and a handover that times out under a
    shortened bound (capture that nothing stopped, nothing archived, the
    request's open state; bounded poll, INCONCLUSIVE discipline — SC-831).

Hostile tree (mandated): the END sandbox family includes an archive root that
is a SYMLINK to elsewhere, and a staged-tree mutation arm where the controller
plants one unexpected entry mid-staging — captures only.

## Section L-PURGE (14) — purge inversion + validator taxonomy

Roster: SC-804a 804b 804c 804d 804e 804f 805 810a 810b 818b 818c 818d 818e 819.

Fixture base: REAL archives produced by real ends (the only legitimate
specimens), then per-arm named mutations. Purge = `end --purge-history` and the
archive-delete path.

Arms:

1. **Purge writes no archive** — end --purge-history on a session with no
   existing archive; capture archive-root before/after (SC-810a).
2. **Purge deletes the existing archive** — session whose UUID already has a
   real archive; capture deletion + what a failed delete leaves (SC-810b).
3. **Validator taxonomy — six mutations, one per arm, each a named byte/mode
   diff on a REAL archive copy**: (a) one unexpected extra entry (SC-804a);
   (b) one symlink inside, and a FIFO sibling arm (SC-804b); (c) one directory
   mode 0755 (SC-804c); (d) one file mode 0644 (SC-804f); (e) exec bit set for
   USER-only, GROUP-only, OTHER-only — three subarms (SC-804d); (f) meta and
   digest.md disagreeing on id, then on counts (SC-804e). Each arm runs the
   purge-side validation AND a `--from` attempt against the mutated tree;
   capture refusal texts and that the tree survives (SC-818c ties here).
4. **Inert-data proof** — an archive member given exec bits + a shebang; the
   validation captures are the proof surface (SC-805).
5. **Symlinked archive root** — purge with the archive ROOT a symlink; capture
   refusal before any deletion (SC-818a).
6. **Claim collision** — plant `.publishing.<uuid>`; run purge; capture
   (SC-818b).
7. **Empty-identity archive** — real archive with the source-identity field
   emptied (named mutation); purge AND --from against it; capture both
   refusals (SC-818d).
8. **Lineage-protected parent** — child session created via real `--from`;
   purge the parent's archive; capture the refusal naming the lineage
   (SC-818e).
9. **Unidentifiable session** — live session dir with meta removed (named
   mutation, memory intact); run end with AND without --purge-history; capture
   that NOTHING is deleted and the refusal names the reason (SC-819).

Hostile tree (mandated): every validator arm runs on a COPY of a real archive
with exactly ONE named mutation — never a hand-built tree, so a refusal
implicates the mutation, not the fixture.

## Section L-STOP (20) — stop matrix + fleet identity

Roster: SC-515a 515b 515c 815a 815b 815c 815d 835a 835b 835c 835d 835e 835f
835g 835h 839a 839b 839c 839d 839e.

Fixture base: multi-session fleets on dedicated servers; fake agents; a
prefix-sibling session pair (the D04a lesson — stop targets must be exercised
against `tmux -t` prefix-match hazards).

Arms:

1. **Plain stop** — capture the exact tmux command trace (delegating shim:
   which server socket, which target form), post-verify state, that no file
   was deleted (SC-835a/b/d). Prefix-sibling present in the topology.
2. **Unverifiable kill** — recorded server socket removed; capture rc, output,
   absence of state change (SC-835c).
3. **Self-stop** — from inside the session (controller drives the pane); arms
   with and without `-y`; capture the confirmation text, the supervisor
   process (ps trace: out-of-pane), and the durable stop-result event
   (SC-835e/f/g/h).
4. **C1–C5 planted failures** — five arms, each making exactly one identity
   check fail (outside tmux; foreign server; wrong recorded server; pane in
   another session; foreign controlling terminal), plus a `--self` arm on the
   C5 cell and a malformed `--pane` token arm; capture each refusal text
   (SC-839a-e: which check is named, what enters shell strings — the trace
   shim records every shell-reaching argv).
5. **stop all fleet** — three sessions; controller starts a FOURTH during the
   confirmation window; capture the acted-on set (SC-815a). Name-handoff arm:
   one confirmed target is ended and recreated under the same name mid-op;
   capture the newcomer's survival + the recorded failure text (SC-815b).
   Concurrent-ops arm: two `stop all` runs with distinct op ids interleaved by
   controller barriers; capture each run's consumed results + the `[op <uuid>]`
   event forms (SC-815c/d).
6. **Exit folding** — a fleet where one target's per-target record is a
   planted failure; capture the folded exit (SC-515a). Result-wait timeout arm
   under a shortened bound; capture the report wording + rc (SC-515b).
   Unowned-ae-tagged arm: a tmux session carrying the ae tag but no session
   dir; capture that it is named, not stopped (SC-515c).

## Section L-COMPACT (21) — compact/handover + preview + exits

Roster: SC-500 501 502 503a 503b 504b 507a 507c 507d 508 512 517a 517b 517c
827 828 829a 829b 836 837 1305.

Fixture base: real sessions with fake agents accepting real sends; controller
drives the worker pane's real `reply` for handover completion (the b0exec
pattern: compact requires spawned agents retired first where the roster
demands — recorded controller actions).

Arms:

1. **Full compact baseline** — capture stdout BYTES exactly (SC-500/512: the
   four-line region), stderr separately (SC-501), the ordering of `Recovery:`
   relative to the relaunch (barrier: capture stdout state before the relaunch
   fires — SC-502), the frozen authorization tuple's one-time read (source
   trace: meta read count after the tuple freeze — SC-827).
2. **Interactive answers** — typed `n` arm vs EOF-as-answer arm (controller
   closes stdin); capture rc + what proceeded (SC-503a/b). `-f` arm
   (SC-837).
3. **SIGPIPE discipline** — compact piped into a consumer that exits early
   (head -1); capture the child's disposition + rc (SC-504b).
4. **Revalidation mismatches** — controller mutates the session (rename its
   tmux session / swap a roster field) at TWO barriers: after the human answer
   and after handover before teardown; capture each refusal and WHICH field it
   names (SC-828).
5. **Handover facts** — capture what completion polls (event log + memo.tsv,
   never pane bytes — source trace); the two-facts pair (reply AND
   handover-topic memo after the request): arms with only-reply and
   only-memo (controller withholds one); capture the wait state (SC-829a).
   Re-run arm: interrupt compact post-request, re-run; capture whether a
   second request is sent and whose baseline is used (SC-829b).
6. **Config refusal** — `purge_agent_history` set; compact with and without
   `--keep-history`; capture (SC-836).
7. **Exit identity** — compact whose relaunch succeeds to a terminal attach
   (pty-wrapped), exits on detach (SC-517a/b); a relaunch that fails
   (unlaunchable tool path); capture the report form + rc (SC-517c).
8. **archive preview** — stdout vs stderr byte separation (SC-507a/c);
   read-only proof by recursive manifest + zero events emitted (SC-507d).
9. **Residual exit codes** — the SC-508 capture-only sweep: every arm above
   already records rc; this arm adds the documented-vs-observed rc table as a
   CAPTURE (no classification).
10. **Mid-operation observability** — SC-1305: at each compact barrier, a
    CONCURRENT `ae list --json` + `requests` read from a separate invocation;
    capture what an observer sees at each cut (per the closure-map gate's
    deterministic-cut requirement; controller-only twins per the b0 rule where
    a reader-effects claim would otherwise be confounded).

## Section L-FROM (9) — lineage

Roster: SC-809 822 823 824a 824b 825a 825b 825c 826.

Arms (fixture: real parent archives from real ends):

1. **Name never infers** — an archive whose source session name matches the
   new session's name; launch WITHOUT --from; capture lineage fields (SC-809).
2. **Existing-session refusals** — --from onto (a) a running tmux session,
   (b) a stopped session dir, (c) a leftover worktree; capture each refusal +
   a full manifest proving nothing was created (SC-822/823).
3. **Proof-fact transport** — instrumented barrier after the parent proof;
   controller deletes the parent archive before the child publishes; capture
   what the child recorded vs re-read (SC-824a) — and the rollback if the
   re-proof path fires (SC-808 territory, captured here for the record).
4. **Mid-publication parent** — plant `.publishing.<uuid>` on the parent;
   --from; capture the refusal (SC-824b).
5. **Lineage durability** — successful --from child: capture
   parent_archive_id + counts in meta, across a stop/resume cycle (SC-825a);
   move AE_HOME wholesale and resume; capture the derived parent path
   behavior (SC-825b); delete the parent archive and resume; capture the
   warning + continuation (SC-825c).
6. **Minted-at-end** — a pre-id session (meta id field removed as a named
   mutation on a legacy-shaped fixture); end it; capture both sides' origin
   records (SC-826).

## Section L-RENTRANS (9) — rename/transfer residue

Roster: SC-814 832a 833a 1302 1303 1304a 1304b 1304c 1304d.

**TRANSPORT DECISION FLAGGED FOR THE GATE:** transfer's remote leg rides
rsync-over-SSH. No-network is the standing rule; the candidate substitutes are
(a) a loopback sshd inside the sandbox (localhost only, documented), or (b) if
frozen `ae transfer` accepts a local-path destination, the local form. The
designer has NOT verified (b) against the frozen source; the section does not
gate until the seats pick the transport. Everything else in this section is
transport-independent.

Arms:

1. **Endpoint validation order** — transfer invoked with a hostile target name;
   capture that refusal precedes any path construction/probe/mkdir/rsync (trace
   shim on ssh/rsync: zero invocations — SC-814).
2. **Rename effect set** — real rename on a running server; capture the four
   effects (tmux name, dir move, meta session=, workspace.md regeneration) +
   server survival (SC-832a).
3. **Rename mid-op observer** — barriers at the census-named cut points (dir
   moved / tmux renamed / meta updated); a concurrent `ae list --json` at each;
   capture the observer's view per cut (SC-1303).
4. **Serialization matrix** — two lifecycle ops on ONE name raced under
   controller barriers, once with flock present, once with flock removed from
   PATH (the documented optional-dep degrade); capture interleaving evidence
   (lock trace + outcome states — SC-1302's bucket-3 incumbent baseline).
5. **Transfer both directions** — stopped session pushed, then pulled back;
   capture full manifests both ends + conversation-file inclusion (SC-833a).
6. **Transfer crash cuts** — controller kills rsync mid-stream (byte-count
   barrier via the delegating shim; the controller, not the shim, kills):
   push-side after stop completes (SC-1304a: source state + destination
   absence), push-side mid-destination-write (SC-1304b), pull-side analogues
   (SC-1304c/d); capture both endpoints' manifests at each cut.

## Execution shape

Each section = its own worker handoff after ITS gate (one worker may take
several gated sections sequentially; sections never share sandboxes). END and
PURGE first (they produce the real-archive specimens PURGE and FROM consume —
a section handoff includes its specimen outputs). STOP independent. COMPACT
after END's fake-agent pattern is proven. RENTRANS last (transport decision).
Artifacts under `l-artifacts/<section>/` with per-section MANIFEST.md in the
b0-design global-contract shape.

---

## SEAT CLASSIFICATION ANNEX — never included in any worker brief

- L-END: barrier order must show stop → git outcome fixed → capture → cleanup
  (SC-817); failed push returns BEFORE capture; no-origin archives with
  push_outcome=no-origin and preserved work dir; failed archive = nonzero exit
  with the live session intact (SC-516, the mandatory-archive invariant);
  claim refusal names the claim and cleans nothing (SC-803); pre-rename kill
  leaves NO archive, post-rename a complete one, never a partial (SC-802);
  staging modes 0700/0600 under umask 077 (SC-801); same-name re-end yields
  two archives keyed by distinct UUIDs (SC-806a); uppercase key normalized
  lowercase (SC-806b); lock released before relaunch, child re-proof mismatch
  rolls back (SC-807/808); rerun resumes via marker, rewrite clears it, pane
  shows the tool not bash (SC-811a/b/812); unreachable server carried + loud
  per-target failure (SC-816); freeze/re-proof refusal prints both versions
  (SC-820a); empty confirmed list = ended nothing, distinct from never-asked
  (SC-821a/b); CLI > config > keep (SC-838a); per-session decision lines
  (SC-838b); digest-only withdraws outstanding (SC-830); timeout stops
  nothing, request stays open (SC-831).
- L-PURGE: each validator mutation FAILS validation (804a-f, exec bit for u/g/o
  all three); refusals delete nothing; purge without archive writes none
  (810a), with archive deletes it (810b); symlink root refused (818a); claim
  blocks (818b); unvalidatable tree refused — "remove it yourself" (818c);
  empty identity refused as malformed, --from will not inherit (818d);
  lineage-pointed parent refused (818e); unidentifiable session refused
  BEFORE stop, nothing deleted, regardless of history flag (819); the inert
  proof is the validator, not intent (805).
- L-STOP: recorded server + exact session id, never name (835a — the
  prefix-sibling must survive every arm); stopped only after verified gone
  (835b); unverifiable = loud, no change (835c); nothing deleted (835d);
  self-stop confirms with the recoverability wording, supervisor out-of-pane,
  durable stop-result (835e/g/h); -y skips (835f); --self waives C5 only
  (839a); malformed --pane refused on shape (839b); refusal names the check
  (839d); no tmux-expanded text in shell strings (839e); confirmed fleet only
  (815a); identity-carried entries leave the name-newcomer running with a
  recorded explanation (815b); own-results only, [op uuid] visible (815c/d);
  per-target results fold into exit (515a); results-timeout reports, is not a
  failure (515b); unowned ae-tagged named not stopped (515c).
- L-COMPACT: stdout is the four lines in order, empty unless boundary crossed,
  non-empty proves archive-exists + recovery-works and nothing more
  (500/512); stderr carries the rest (501); Recovery precedes relaunch (502);
  typed n answers, EOF does not (503a/b); no altered SIGPIPE disposition
  leaks (504b); tuple frozen once, meta never re-read downstream (827);
  mismatch names the moved field, first revalidation protects MESSAGING,
  second protects STOPPING (828); completion = reply AND fresh handover memo,
  polled from log+memo.tsv never panes (829a); re-run reuses the SAME request
  and its stored baseline (829b); purge-config refuses without --keep-history
  (836); -f skips confirmation (837); exit is the launch's; terminal case
  attaches and exits on detach; failure reports as plain ae <name> (517a-c);
  preview stdout = digest bytes exactly, diagnostics stderr, read-only
  (507a/c/d); SC-508 and SC-1305 are capture-only: seats classify the
  residual-rc table and the mid-op observer views at closure.
- L-FROM: lineage only via explicit --from (809); refusals leave no session,
  no state, no worktree (822/823); proof facts recorded-not-reread (824a);
  mid-publication refused outright (824b); lineage durable across resume
  (825a), path derived not stored (825b), deleted parent warns and continues
  (825c); minted-at-end recorded on BOTH sides (826).
- L-RENTRANS: both endpoint names validated before ANY side effect (814);
  rename's four effects with the server up (832a); transfer moves the stopped
  session both directions including conversation files (833a); serialization
  holds under flock, silently disables without it — the bucket-3 incumbent
  baseline, never a SHOULD (1302); the 1303/1304a-d mid-op views are
  capture-only code-observation rows: the seats classify residue at closure;
  1304a's gate precision — source PRESENT (not byte-intact), no destination
  write yet.
