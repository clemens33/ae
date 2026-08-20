# P0 closure evidence map

- commit: `72c7293` (`72c729343a0117af2968b66e1c43f89ad25fc0b2`)
- date: `2026-08-20`
- agent: `gpt56luna:closuremap`
- counts: mapped `109`; probe-needed `23`; total `132`

## Semantic-contract placeholders

SC-500 (@semantic-contract.md:154) -> tests/integration:compact-out: line 1 names the archive id; line 2 the archive path; line 3 the digest path; line 4 the recovery command, BEFORE the relaunch
SC-512 (@semantic-contract.md:159) -> tests/integration:compact-out: it does NOT claim a relaunch that can still refuse
SC-501 (@semantic-contract.md:165) -> tests/integration:compact-out: the relaunch announcement is progress, so it is on stderr; and the recovery command reaches stderr too
SC-502 (@semantic-contract.md:170) -> tests/integration:compact-out: line 4 the recovery command, BEFORE the relaunch
SC-503a (@semantic-contract.md:174) -> tests/integration:compact: while a typed 'n' is an answer, and answers exit 0
SC-503b (@semantic-contract.md:177) -> tests/integration:compact: EOF exits NONZERO — ae could not ask
SC-504a (@semantic-contract.md:182) -> tests/integration:compact-pipe: AND THE CHILD STILL STARTED — a reporting failure must never suppress the relaunch
SC-504b (@semantic-contract.md:187) -> tests/unit:compact-boundary: SIGPIPE is ignored across the report and RESTORED before the exec
SC-509 (@semantic-contract.md:207) -> tests/unit:list: --json emits a versioned digest wrapper
SC-510a (@semantic-contract.md:214) -> tests/unit:events: send emits event with action and summary override support
SC-510b (@semantic-contract.md:218) -> NO-EVIDENCE(probe needed: emit events with empty target, ref, and summary and inspect omitted JSON keys)
SC-510c (@semantic-contract.md:222) -> tests/integration:request-integrity: ask records target_slot; reply verifies by slot after a name churn
SC-510d (@semantic-contract.md:226) -> tests/unit:events: json_escape combined
SC-511a (@semantic-contract.md:229) -> tests/integration:request-integrity: ask records target_slot
SC-511b (@semantic-contract.md:234) -> tests/integration:request-integrity: reply ROUTES to the sender CURRENT name via slot (not stale)
SC-511c (@semantic-contract.md:238) -> NO-EVIDENCE(probe needed: compare an event consumer before and after adding, removing, and renaming optional event keys)
SC-507c (@semantic-contract.md:247) -> tests/integration:archive-preview: stderr reports what would be archived; stderr names the canonical archive id
SC-507d (@semantic-contract.md:251) -> tests/integration:archive-preview: it wrote nothing into the live session; it published no archive; it took no publisher claim
SC-507b (@semantic-contract.md:255) -> NO-EVIDENCE(probe needed: mutate each moving preview file during render and inspect the clean-retry or moving diagnostic)
SC-513a (@semantic-contract.md:260) -> tests/integration:next: exits non-zero when nothing needs attention
SC-513b (@semantic-contract.md:264) -> tests/integration:next: unknown flag exits non-zero
SC-513c (@semantic-contract.md:267) -> tests/integration:next: output does not advertise attach (read-only)
SC-514 (@semantic-contract.md:270) -> tests/integration:doctor: prints summary
SC-515a (@semantic-contract.md:273) -> tests/integration:outside caller (F7): it waited for the outcomes rather than only announcing
SC-515b (@semantic-contract.md:277) -> tests/unit:F7: the wait is bounded and a timeout keeps the handoff rc
SC-515c (@semantic-contract.md:281) -> tests/integration:stop all: an unmanaged ae-tagged session makes the fleet op FAIL
SC-516 (@semantic-contract.md:286) -> tests/integration:archive-claim: a standing claim FAILS the end rather than being cleaned up
SC-517a (@semantic-contract.md:292) -> NO-EVIDENCE(probe needed: make the compacted child exit with a distinct status and compare compact's returned status)
SC-517b (@semantic-contract.md:296) -> NO-EVIDENCE(probe needed: run compact from a terminal, detach from the successor, and record the terminal-path status)
SC-517c (@semantic-contract.md:299) -> NO-EVIDENCE(probe needed: force successor launch failure and capture the non-terminal compact diagnostic and recovery command)
SC-508 (@semantic-contract.md:303) -> NO-EVIDENCE(probe needed: enumerate exit-status paths not covered by SC-513 through SC-517 and run one probe per path)
SC-702 (@semantic-contract.md:339) -> tests/unit:#36: an undelivered launch prompt is preserved and recorded, never silent
SC-801 (@semantic-contract.md:391) -> tests/unit:archive-publish: dirs 0700, files 0600, no executable bit anywhere
SC-803 (@semantic-contract.md:399) -> tests/integration:archive-claim: a standing claim FAILS the end rather than being cleaned up; and names the claim path
SC-804a (@semantic-contract.md:404) -> tests/unit:archive-publish: exactly the whitelisted tree, nothing else
SC-804b (@semantic-contract.md:408) -> tests/unit:validator: a symlink out of the archive FAILS
SC-804c (@semantic-contract.md:411) -> tests/unit:validator: a loosened directory mode FAILS
SC-804f (@semantic-contract.md:414) -> tests/unit:validator: digest.md chmod 0700 FAILS
SC-804d (@semantic-contract.md:417) -> tests/unit:validator: a GROUP-executable file FAILS (a -x test would have missed it)
SC-804e (@semantic-contract.md:421) -> tests/unit:validator: meta and digest disagreeing about counts FAILS
SC-805 (@semantic-contract.md:425) -> tests/integration:archive-end: dirs 0700, files 0600 — nothing in an archive is executable
SC-806a (@semantic-contract.md:429) -> tests/integration:archive-end: outcome names the canonical UUID; meta records source session name
SC-806b (@semantic-contract.md:433) -> tests/unit:archive-uuid: uppercase canonicalizes to lowercase
SC-807 (@semantic-contract.md:436) -> tests/unit:lifecycle-lock (12th I1): released BEFORE the async SID-capture children inherit fd 8
SC-808 (@semantic-contract.md:440) -> tests/integration:compact-rollback: a parent that no longer matches refuses the launch; rollback ran
SC-809 (@semantic-contract.md:446) -> tests/integration:lineage: the same name without --from records NO parent
SC-810a (@semantic-contract.md:450) -> tests/integration:end-all: the purge target was NOT archived
SC-810b (@semantic-contract.md:453) -> tests/integration:end-all: the purge target's pre-existing archive was deleted
SC-811a (@semantic-contract.md:458) -> tests/integration:#27: re-running the script RESUMES the ORIGINAL id
SC-811b (@semantic-contract.md:462) -> tests/integration:#27: the pane's own first launch CREATED that session; re-running the script RESUMES the ORIGINAL id
SC-812 (@semantic-contract.md:466) -> tests/unit:launch: claude resume decides before exec, no fallback chain
SC-814 (@semantic-contract.md:478) -> tests/integration:transfer: no ssh and no rsync ever ran for an invalid name
SC-815a (@semantic-contract.md:482) -> tests/unit:F9: the detached supervisor never re-enumerates the fleet
SC-815b (@semantic-contract.md:487) -> tests/integration:ABA (F10): the refusal is recorded, stamped, and names both instances
SC-815c (@semantic-contract.md:492) -> tests/unit:F8: the op stamp has exactly one definition, used by writer AND reader
SC-815d (@semantic-contract.md:496) -> tests/unit:F8: stop-request AND stop-result are both stamped with the operation
SC-816 (@semantic-contract.md:499) -> tests/unit:F6: skip requires VERIFIED GONE — unknown is carried, not dropped
SC-818a (@semantic-contract.md:513) -> tests/unit:archive-root: a SYMLINKED archive root is refused, not followed
SC-818b (@semantic-contract.md:517) -> tests/unit:archive-purge: a standing claim REFUSES the purge
SC-818c (@semantic-contract.md:521) -> tests/unit:archive-purge: a directory that does not validate as an archive is NOT deleted
SC-818d (@semantic-contract.md:526) -> tests/unit:archive-purge: an archive naming no session is NOT purged by a session that shares its UUID
SC-818e (@semantic-contract.md:531) -> tests/unit:archive-purge: the parent archive named by --from is REFUSED
SC-819 (@semantic-contract.md:534) -> tests/integration:archive-nometa: refused BEFORE anything was stopped; archive-badid: an unparseable session_id FAILS the end
SC-820a (@semantic-contract.md:540) -> tests/integration:end-freeze: a confirmed KEEP is not carried out as a PURGE; caught BEFORE the stop
SC-820b (@semantic-contract.md:547) -> tests/unit:end-freeze: -f freezes nothing, because nothing was promised
SC-821a (@semantic-contract.md:550) -> tests/integration:end-all: a session that appeared AFTER the prompt was NOT ended
SC-821b (@semantic-contract.md:554) -> tests/unit:end-all: branch keys off whether a prompt RAN, never off the list's length
SC-822 (@semantic-contract.md:559) -> tests/integration:lineage: --from onto an EXISTING session refuses instead of attaching
SC-823 (@semantic-contract.md:564) -> tests/integration:lineage: invalid --from creates no session state, no worktree, and no tmux session
SC-824a (@semantic-contract.md:568) -> tests/unit:from-preflight: id and counts come back as ONE frozen observation
SC-824b (@semantic-contract.md:572) -> tests/unit:from-preflight: an archive being published or purged right now is refused
SC-825a (@semantic-contract.md:575) -> tests/integration:lineage: child records parent archive id, handover count, and pending count
SC-825b (@semantic-contract.md:579) -> tests/unit:from-prompt: digest path follows AE_HOME (derived, not stored)
SC-825c (@semantic-contract.md:583) -> tests/integration:lineage: a vanished parent leaves lineage recorded; workspace.md says digest is no longer there
SC-826 (@semantic-contract.md:587) -> tests/integration:archive-legacy: a pre-session-id session is minted one; archive marks the id as minted at archive time
SC-828 (@semantic-contract.md:598) -> tests/integration:compact-gate2: an identity change during the unlocked wait is refused; the replacement was never STOPPED
SC-829a (@semantic-contract.md:604) -> tests/integration:compact-wait: reply AND memo together complete the handover
SC-829b (@semantic-contract.md:609) -> tests/integration:compact-wait: a retry reuses the outstanding handover request
SC-830 (@semantic-contract.md:614) -> tests/integration:compact-withdraw: --digest-only withdraws it explicitly; archive carries ask and cancellation
SC-831 (@semantic-contract.md:618) -> tests/integration:compact-wait: with neither fact, it times out; exactly one request was sent
SC-832b (@semantic-contract.md:627) -> docs/migration/evidence/locks-census-2.md:## `rename` (`cmd_rename`), lines 279-310
SC-832c (@semantic-contract.md:631) -> docs/migration/evidence/locks-census-2.md:## `rename` (`cmd_rename`), lines 279-310
SC-833b (@semantic-contract.md:639) -> docs/migration/evidence/locks-census-2.md:## `transfer` (`cmd_transfer`), lines 312-346
SC-833c (@semantic-contract.md:642) -> docs/migration/evidence/locks-census-2.md:## `transfer` (`cmd_transfer`), lines 312-346
SC-833d (@semantic-contract.md:645) -> docs/migration/evidence/locks-census-2.md:### Transfer audit event is best-effort, lines 735-739
SC-834b (@semantic-contract.md:653) -> docs/migration/evidence/locks-census-2.md:## `_recover-pending` (standalone command and watchdog path), lines 450-482
SC-834c (@semantic-contract.md:657) -> docs/migration/evidence/locks-census-2.md:## `_recover-pending` (standalone command and watchdog path), lines 450-482
SC-1101b (@semantic-contract.md:883) -> NO-EVIDENCE(probe needed: run the core command set with timeout and gtimeout absent and record each fallback or refusal)
SC-1102 (@semantic-contract.md:886) -> tests/unit:archive-uuid: uppercase canonicalizes to lowercase
SC-1104 (@semantic-contract.md:897) -> NO-EVIDENCE(probe needed: run process-tree introspection on a host without /proc and record the parent-walk result)
SC-1105 (@semantic-contract.md:902) -> tests/integration:helpers: every shebang names a bash >= 4 that can parse the helper
SC-1201 (@semantic-contract.md:928) -> tests/unit:#59 R1: _cmd_spawn REFUSES the injection name
SC-1203 (@semantic-contract.md:941) -> tests/integration:#59 C3-2: a RESTORED roster name is fail-quiet, a FRESH one is fatal
SC-1204 (@semantic-contract.md:947) -> tests/unit:#59 R1: a hostile name ALREADY in meta yields NO identity line
SC-1206 (@semantic-contract.md:962) -> tests/unit:#59 R1: a leading '_' is refused
SC-1207a (@semantic-contract.md:967) -> NO-EVIDENCE(probe needed: supply facet-separator characters in alias and name and inspect prompt identity parsing)
SC-1207b (@semantic-contract.md:971) -> tests/integration:spawn: recorded in meta
SC-1208 (@semantic-contract.md:974) -> NO-EVIDENCE(probe needed: send peer text containing instruction material and inspect system/developer context versus user-input delivery)
SC-1209 (@semantic-contract.md:980) -> tests/integration:envelope (#39): a send FROM an agent pane carries that agent as sender; it leads the message, ahead of the body

## Ownership TBD cells

D01.locks (@ownership.md:95) -> NO-EVIDENCE(probe needed: run list concurrently with metadata and event writers while tracing lock acquisition)
D01.atomicity (@ownership.md:96) -> NO-EVIDENCE(probe needed: mutate each list input during rendering and capture whether the result is a coherent snapshot)
D02.locks (@ownership.md:104) -> NO-EVIDENCE(probe needed: run requests query during concurrent request-event append and trace lock acquisition)
D02.atomicity (@ownership.md:105) -> NO-EVIDENCE(probe needed: query requests while appending an event and capture the observed record boundary)
D03.writer-dispatcher-readers (@ownership.md:112) -> NO-EVIDENCE(probe needed: enumerate dispatcher call sites that read events-tail data)
D03.locks (@ownership.md:113) -> NO-EVIDENCE(probe needed: run events-tail during concurrent append and trace lock acquisition)
D03.atomicity (@ownership.md:114) -> NO-EVIDENCE(probe needed: tail events.jsonl during concurrent append and capture partial-line behavior)
D04a.locks (@ownership.md:122) -> NO-EVIDENCE(probe needed: run status during concurrent metadata and event writes while tracing lock acquisition)
D04a.atomicity (@ownership.md:123) -> NO-EVIDENCE(probe needed: capture status output while each input file is rewritten)
D04b.locks (@ownership.md:132) -> NO-EVIDENCE(probe needed: run next during concurrent metadata and event writes while tracing lock acquisition)
D04b.atomicity (@ownership.md:133) -> NO-EVIDENCE(probe needed: capture next output while its event and metadata inputs are rewritten)
D08.locks (@ownership.md:196) -> docs/migration/evidence/locks-census.md:## `goal` (`helper_goal_main` + `ae_meta_set`), lines 120-153
D08.atomicity (@ownership.md:197) -> docs/migration/evidence/locks-census.md:## `goal` (`helper_goal_main` + `ae_meta_set`), lines 120-153
D09.locks (@ownership.md:205) -> docs/migration/evidence/locks-census.md:## `memo add` (`helper_memo_main`), lines 155-179
D09.atomicity (@ownership.md:206) -> docs/migration/evidence/locks-census.md:## `memo add` (`helper_memo_main`), lines 155-179
D10.locks (@ownership.md:214) -> docs/migration/evidence/locks-census.md:## `say` (`helper_say_main`), lines 181-198
D10.atomicity (@ownership.md:215) -> docs/migration/evidence/locks-census.md:## `say` (`helper_say_main`), lines 181-198
D11.atomicity (@ownership.md:227) -> docs/migration/evidence/locks-census-2.md:## `interrupt` (`helper_interrupt_main`), lines 19-50
D15.locks (@ownership.md:290) -> docs/migration/evidence/locks-census.md:## `spawn` (`_cmd_spawn`), lines 218-286
D16.locks (@ownership.md:302) -> docs/migration/evidence/locks-census.md:## `retire` (`helper_retire_main` + `_cmd_retire`), lines 287-320
D17.locks-extra (@ownership.md:323) -> docs/migration/evidence/locks-census-2.md:## Session launch (`ae [name]`, including `--from`), lines 74-176
D17.atomicity (@ownership.md:324) -> docs/migration/evidence/locks-census-2.md:## Session launch (`ae [name]`, including `--from`), lines 74-176
D19a.locks-detail (@ownership.md:361) -> docs/migration/evidence/locks-census-2.md:## `stop` (`cmd_stop` and supervisors), lines 236-277
D20.atomicity (@ownership.md:387) -> docs/migration/evidence/locks-census-2.md:## `rename` (`cmd_rename`), lines 279-310
D21.locks (@ownership.md:395) -> docs/migration/evidence/locks-census-2.md:## `transfer` (`cmd_transfer`), lines 312-346
D21.atomicity (@ownership.md:396) -> docs/migration/evidence/locks-census-2.md:## `transfer` (`cmd_transfer`), lines 312-346
D22.locks-extra (@ownership.md:410) -> docs/migration/evidence/locks-census-2.md:## `compact` (`cmd_compact`), lines 348-400
D22.atomicity (@ownership.md:411) -> docs/migration/evidence/locks-census-2.md:## `compact` (`cmd_compact`), lines 348-400
D24.effects (@ownership.md:430) -> NO-EVIDENCE(probe needed: inspect #71 durable handoff record implementation and identify its effect writers)
D27.locks-atomicity (@ownership.md:507) -> docs/migration/evidence/locks-census-3-aewatch.md:## Bridge ownership handoff (`marker -> fresh heartbeat -> kill Bash -> send`), lines 141-190
D28a.locks-atomicity (@ownership.md:515) -> docs/migration/evidence/locks-census-2.md:## Telegram setup/start/stop and daemon loop, lines 568-626
D28b.locks-atomicity (@ownership.md:527) -> docs/migration/evidence/locks-census-2.md:## Telegram setup/start/stop and daemon loop, lines 568-626
D29b.locks-atomicity (@ownership.md:553) -> docs/migration/evidence/locks-census-2.md:## Steward (`cmd_steward*`, autostart, and runtime), lines 628-675
