# Batch L — section L-END artifacts — MANIFEST

Worker: `opus5:lexec`. This file maps **arm → row ids → artifact paths → fixtures →
mutation diffs** for the L-END (end/archive transaction) evidence run.

Everything here is a CAPTURE: bytes, hashes, recursive manifests, byte diffs, rc values,
barrier orderings, argv traces and logs, together with the manipulation and the barrier
that produced them. There are no verdicts, no expected-vs-actual statements and no
classification anywhere in this section. Seats classify.

## Run-wide provenance

| field | value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` blob | `50e4f575baf3aa584b98b3eaaeec5264e4916161` |
| frozen `ae` sha256 | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| frozen tree source | `git archive 72c7293` into `/tmp/aelx/frozen` — the live checkout's `ae` and product files are never touched, and no operator `AE_HOME`, tmux server or running session is involved |
| instrumented `ae` sha256 | `84f68ac8f14bd9686d34a9153eaafbce575e6a4066e0d44c258423c9a9300f18` |
| hook patch | `_harness/hooks.patch` — sha256 `00b06eaad573db298bd2d5c608a297e1aa6099f863fc3a819b05346c3ceeedde` |
| host / interpreter / tmux / git | `_harness/env-record.txt` (all hashed) |
| pinned locale | `LANG=LC_ALL=en_US.UTF-8`, `TZ=UTC` |
| live models / network | none used |

Every arm ran in its own **disposable sandbox** under `/tmp/aelx/L-END/<arm>/` with a
private `HOME`, `AE_HOME`, `TMPDIR`, `TMUX_TMPDIR` and tmux socket, built fresh and torn
down (tmux kill-server plus a process sweep on the sandbox path). No arm shares a
sandbox with another.

## Instrumentation, and the proofs it is admissible

Per cluster-plan.md's global rule: an exact 72c7293 copy plus **ONE hook-only patch**,
recorded and hashed. `_H` returns 0 immediately when `AE_L_HOOKS` is unset; when active
it appends a barrier ordinal to a harness file and optionally blocks on a release file.
It never reads, hashes or computes over product state, and it writes only to harness
paths (`AE_L_TRACE`, `AE_L_BLOCK`) outside every cloned `AE_HOME`.

Hook sites, by barrier name (all in `_harness/hooks.patch`):

| barrier | site |
|---|---|
| `b_confirm_answered` | `cmd_end`, after the confirmation phase — whether or not a prompt ran — and before target dispatch |
| `b_stop_local` | `_end_session_locked`, local branch, after the verified kill |
| `b_stop_git` | `_end_session_locked`, git branch, after the verified kill |
| `b_git_fixed` | before each `_end_archive_step` whose push outcome is already fixed |
| `b_stage_mid` | `_ar_stage_payload`, after the payload copies and before the digest render |
| `b_pre_rename` | `_ar_publish`, after validation and the target recheck, immediately before `mv` |
| `b_post_rename` | `_ar_publish`, immediately after `mv` succeeded |
| `b_pre_cleanup` | before each `cleanup_session` in `_end_session_locked` |
| `b_from_proved` | launch, after the FIRST parent-archive proof is parsed |
| `b_cp_after_answer` | `cmd_compact`, after the human answer is accepted |
| `b_cp_after_handover` | `cmd_compact`, after the handover completed, before phase (c) |
| `b_cp_pre_relaunch` | `cmd_compact`, immediately before the `exec` into the relaunch |

Three PATH shims are used, all **delegate-and-log** per the date-shim contract (they
substitute nothing) except where a named failure is injected:

| shim | role |
|---|---|
| `_harness/gitshim.sh` | `git` — delegates every subcommand; with `AE_L_GIT_FAIL=push` it logs and exits 128 for `push` only |
| `_harness/flockshim.sh` | `flock` spy — delegates every invocation, logs argv + timestamp; inherited fds pass through the exec |
| tmux shim (in `_harness/sandbox.sh`) | delegates every `tmux` invocation, logs argv and the effective `AE_TMUX_SERVER`/`AE_TMUX_SERVER_KIND` |

### Admissibility proofs (`_admissibility/`)

Each compares two runs of one identical scripted fixture on equal-length sandbox paths,
over stdout, stderr, rc, structural `AE_HOME` manifests and tmux snapshots, through the
recorded normalizer `_harness/norm.sed`.

| file | comparison | comparator verdict |
|---|---|---|
| `equiv-A-inactive-hook.txt` | frozen vs instrumented with `AE_L_HOOKS` unset (end fixture) | NO_DIFFERENCES |
| `equiv-B-tmux-shim.txt` | frozen without vs with the tmux delegate-and-log shim | NO_DIFFERENCES |
| `equiv-D-known-difference.txt` | same binary, `end -f` vs `end -f --purge-history` | DIFFERENCES_PRESENT |
| `equiv-E-flock-spy.txt` | frozen without vs with the flock spy (compact fixture) | NO_DIFFERENCES |
| `equiv-E-known-difference.txt` | same fixture, no `--digest-only` and a short handover bound | DIFFERENCES_PRESENT |
| `equiv-F-git-shim.txt` | frozen without vs with the git shim in DELEGATING mode (managed end fixture) | NO_DIFFERENCES |
| `equiv-F-known-difference.txt` | same fixture, `end -f` vs `end -f --purge-history` | DIFFERENCES_PRESENT |

The `known-difference` rows exist because a comparator that cannot report a difference
proves nothing about the ones that report none. A first version of this comparator
reported "no differences" for every pair because a fixture helper's stdout had corrupted
the sandbox paths it was handed; the assertion that every capture file exists and is
non-empty, plus these controls, is what replaced it.

## The environment is an instrument (blocking, per LIVE arm)

Every live arm runs `_harness/l_lib.sh :: l_tab_preflight` **inside that arm's own
`env -i` environment**, driving the consumer's own TAB-separated tmux queries at
72c7293 (`list-panes -s -t <s> -F '#{@ae_agent}<TAB>#{pane_current_command}'` and
`'#{pane_id}<TAB>#{@ae_agent}'`) and recording the raw bytes in hex. The arm proceeds
only when byte `0x09` is observed in the consumer's own answer; otherwise the arm writes
`ARM-INVALID.txt` and takes no capture. Per-arm file: `preflight-tab.txt`.

## Per-consumer in-process trace (brief rule (e), every live arm)

| file | content |
|---|---|
| `consumer-inproc.txt` | the arm's recorded `tmux_server` / `tmux_server_kind`, and `declare -F tmux` + `type tmux` + the resulting function body under those values. The frozen `_ae_install_tmux_shim` is awk-extracted from 72c7293 and hashed; the file is labelled a harness-side reconstruction |
| `tmux-argv.log` | the authoritative delegated `command tmux` argv, one line per invocation, with the effective `AE_TMUX_SERVER`/`AE_TMUX_SERVER_KIND` as the delegated call saw them |
| `_ae_install_tmux_shim.extracted.sh` | the extracted frozen source the reconstruction ran |

## Common capture set (every arm)

`ARM.txt` (arm, roster ids, fixture, construction, rc values, bounds) ·
`<step>.stdout` / `.stderr` / `.rc` / `.invocation` (argv + full env) ·
`0pre.*` and `3post.*` recursive `AE_HOME` manifests (path, type, mode, nlink, size,
symlink target, sha256), archive-root manifests, work-dir manifests, tmux snapshots
(server/sessions/windows/panes/clients), git state and origin refs where end's git phase
runs · `*.events.jsonl` copies for byte deltas · `barrier-order.tsv` plus a per-barrier
`AE_HOME` manifest, archive manifest, tmux snapshot and git state ·
`hook-trace.tsv` (barrier ordinals with pid and monotonic timestamp) · `SHA256SUMS.txt`.

Barrier controllers are **value-blind**: they presume no barrier set and no order. They
poll for any `*.reached` marker, capture under that marker's own name, run the arm's
controller action if it matches, then release. Every wait is bounded and the bound is
recorded in `ARM.txt`; on expiry the arm writes `INCONCLUSIVE.txt` or an
`INCONCLUSIVE:` line into `barrier-order.tsv` and no absence is inferred.

## Arms

| arm | roster ids | fixture | construction | key artifacts |
|---|---|---|---|---|
| `transaction-order-a-full-run` | SC-817 | managed (`--worktree`, local bare `file://` origin) | no manipulation; a complete end over a dirty tree, all eight barriers blocking | `barrier-order.tsv`, `b0*-*.{aehome,archive,tmux,git}.txt`, `archive-meta-fields.txt`, `3post.workdir.tsv` |
| `transaction-order-b-push-fails` | SC-817 | same | `git` PATH shim delegates every subcommand except `push`, which logs and exits 128 | `git-shim.log` (carries the `INJECTED-FAILURE sub=push rc=128` line), `2end.stderr`, `barrier-order.tsv` |
| `transaction-order-c-no-origin` | SC-817 | same | the `origin` remote is removed from the repo before the launch | `archive-meta-fields.txt`, `3post.git.txt`, `3post.workdir.tsv` |
| `archive-write-inability` | SC-516 | managed + inability canary | the archive root is made mode `0500`; the canary attempts the same write class (`mkdir` under the root, the write `_ar_publish` makes for its claim) and is recorded refusing; then end runs | `canary.txt`, `3post.livedir.tsv`, `2end.{stdout,stderr,rc}` |
| `claim` | SC-800, SC-803 | managed | a `.publishing.<uuid>` directory for this session's OWN uuid is pre-created mode `0700` under the archive root | `claimdir.before.tsv`, `claimdir.after.tsv`, `2end.stdout.od`, `2end.stderr.od` |
| `staging-modes` | SC-801 | managed, barriers blocking | no manipulation; the staging tree is captured at the mid-staging barrier and the published tree afterwards | `b04-b_stage_mid.*.staging-payload.tsv`, `*.staging-claim.tsv`, `final-archive.tsv` |
| `staging-modes-planted-entry` | SC-801 | managed, barriers blocking | hostile construction: the controller plants an unexpected file and directory into the staging payload at the mid-staging barrier | `*.planted.txt`, `*.staging-payload.tsv` and `*.staging-payload.after-plant.tsv`, `2end.stderr` |
| `publication-crash-cut-pre_rename` | SC-802 | managed, barriers blocking | the controller SIGKILLs the entire end process tree at `b_pre_rename` | `b05-*.cut.txt`, `b05-*.archive-at-cut.tsv`, `final-archive.tsv`, `2end.rc` |
| `publication-crash-cut-post_rename` | SC-802 | managed, barriers blocking | the controller SIGKILLs the entire end process tree at `b_post_rename` | `b06-*.cut.txt`, `b06-*.archive-at-cut.tsv`, `final-archive.tsv`; **produces the `existing-archive` specimen** |
| `identity-same-name-twice` | SC-806a | `--local` | a session is ended, the SAME name is launched again, and that one is ended too — no mutation | `archive-identity.txt`, `meta.run1.txt`, `meta.run2.txt`, `archive.after-end1.tsv`, `archive.after-end2.tsv` |
| `identity-uppercase-meta-uuid` | SC-806b | `--local`, REAL LIVE session | writer boundary: the live session meta's `session_id` value is case-folded `a-f` → `A-F` by a temp+rename write, then end runs | `mutation.txt`, `meta.mutation.diff`, `archive-identity.txt` (archive directory name and the `archive_id` line, both as `od` bytes), `final-archive.tsv` |
| `compact-relaunch-lock-control` | SC-807 | `--local`, real `compact -f --digest-only` | no mutation; a delegate-and-log `flock` spy records every lock invocation across compact and the relaunch it `exec`s into | `flock-spy.log`, per-barrier `*.flock-spy.snapshot.log`, `*.ps.txt`, `lineage.txt`, `barrier-order.tsv` |
| `compact-relaunch-lock-parent-mutated` | SC-808 | same | at `b_from_proved` — the barrier after the child launch parses its FIRST parent-archive proof — the controller rewrites the parent archive meta's `handover_count` (temp+rename) before releasing | `b07-*.parent-meta.{before,after}.txt` and `.diff`, `b07-*.controller.txt`, `lineage.txt`, `3post.sessions.tsv` |
| `launch-rerun` | SC-811a, SC-811b, SC-812 | `--local`, renamed-interpreter fake tool | ae's OWN launch executes the generated `launch.main.sh` (frozen ae:12606) as execution 1; the controller then executes the SAME script directly in a control pane (execution 2), kills that pane, executes it again (execution 3); ae is then made to rewrite the script by stop + resume (execution 4) | `marker-timeline.txt` (script sha256, marker existence/size/mtime and `fake.invocations.so_far` at seven labels), `fake-argv-both-runs.txt` (every execution's argv verbatim, NUL-separated), `launch.main.sh.<label>` copies, `launch-script.rewrite.diff`, `*.pane_current_command.txt`, `*.script.stderr` |
| `unreachable-server` | SC-816 | `--local`, two sessions on one recorded server | the directory holding the recorded tmux socket is removed (the server process is left running), then end runs for one target and then for `all` | `manipulation.txt`, `socketdir.before.tsv`, `socketdir.after.tsv`, `2end.stderr`, `3endall.stderr`, `3post.aehome.tsv` |
| `endall-rename-between-confirm-and-lock` | SC-820a | `--local`, THREE sessions on one server, real terminal | `ae end all` runs on a pty; after the answer is accepted and before the per-target lifecycle lock, the controller renames ONE target tmux session (`ef2` → `ef2-renamed`) and leaves its on-disk state untouched | `b01-*.tmux.before-rename.txt` / `.after-rename.txt`, `b01-*.controller.txt`, `2endall.stdout.od`, `2endall.stderr.od`, `2endall.pane.*.txt`, `post-state.txt` |
| `endall-empty-plan` | SC-821a, SC-821b | an `AE_HOME` with a config and no sessions at all, real terminal | `ae end all` runs against a state whose target enumeration yields nothing | `2endall.pane.at-prompt.txt`, `frozen-plan-as-rendered.txt`, `2endall.stdout.od`, `2endall.stderr.od`, `0pre.dirs.txt`, `3post.aehome.tsv` |
| `history-policy-c1..c9` | SC-838a, SC-838b | nine independent clones, two `--local` sessions each, real terminal | the full cross of the CLI history flag {none, `--purge-history`, `--keep-history`} with `[workspace] purge_agent_history` {unset, `true`, `false`}; `end all` is answered `y` at a real prompt | per cell: `planted-conversations.txt`, `conversations.before.tsv`, `conversations.after.tsv`, `conversations.diff`, `2endall.pane.at-prompt.txt` (the per-target plan lines), `2endall.stdout` (the per-session decision lines) |
| `handover` | SC-830, SC-831 | `--local`, renamed-interpreter fake tool that accepts real sends | `compact -f` runs first under a SHORTENED handover bound (`AE_COMPACT_HANDOVER_SECS=8`) with no reply and no handover memo supplied; the same session is then compacted with `--digest-only` | `requests.0pre.txt`, `requests.1at-expiry.txt`, `requests.2before-digest-only.txt`, `requests.3after-digest-only.txt`, `events.<label>.jsonl`, `1at-expiry.sessiondir.tsv`, `post-archive.txt` |
| `hostile-symlinked-archive-root` | none — hostile construction, captures only | `--local` | the archive root is replaced by a symlink to a directory outside `AE_HOME` | `manipulation.txt`, `0pre.linktarget.tsv`, `3post.linktarget.tsv`, `2end.stderr` |

### Roster coverage as executed

All 21 L-END roster ids have a primary arm above: SC-516, SC-800, SC-801, SC-802,
SC-803, SC-806a, SC-806b, SC-807, SC-808, SC-811a, SC-811b, SC-812, SC-816, SC-817,
SC-820a, SC-821a, SC-821b, SC-830, SC-831, SC-838a, SC-838b. `hostile-symlinked-archive-root`
and `staging-modes-planted-entry` are the design's two hostile constructions and carry no
roster id.

## Named mutations and byte diffs

| arm | mutation | recorded diff |
|---|---|---|
| `identity-uppercase-meta-uuid` | live session meta `session_id`, `a-f` → `A-F`, temp+rename | `mutation.txt`, `meta.mutation.diff` |
| `compact-relaunch-lock-parent-mutated` | parent archive meta `handover_count` `n` → `n+7`, temp+rename | `b07-*.parent-meta.diff`, `b07-*.controller.txt` |
| `staging-modes-planted-entry` | one unexpected file and one unexpected directory planted into the staging payload | `*.planted.txt`, payload manifests before and after the plant |
| `claim` | a `.publishing.<uuid>` directory created for the session's own uuid | `claimdir.before.tsv` vs `claimdir.after.tsv` |
| `archive-write-inability` | archive root mode `0700` → `0500` | `canary.txt` (records the mode and the refusal) |
| `unreachable-server` | the recorded socket's directory removed | `socketdir.before.tsv` vs `socketdir.after.tsv` |
| `hostile-symlinked-archive-root` | archive root replaced by a symlink | `manipulation.txt`, `0pre.aehome.tsv` vs `3post.aehome.tsv` |
| `endall-rename-between-confirm-and-lock` | one target's tmux session renamed at the post-confirmation barrier, on-disk state untouched | `b01-*.tmux.before-rename.txt` vs `.after-rename.txt` |
| `transaction-order-b-push-fails` | `push` refused by the delegate-log-fail shim | `git-shim.log` |
| `transaction-order-c-no-origin` | the `origin` remote removed before the launch | `0pre.git.txt` vs the arm's `1launch.invocation` |

Conversation-file note for `history-policy-c1..c9`: the files are controller-planted
markers at the exact path the frozen locator globs
(`$HOME/.claude/projects/*/<uuid>.jsonl`). The frozen locator
(`_transfer_find_claude_session_file`, 72c7293:10769) matches on PATH only and never
reads their content, so no content is fabricated. Each planted path is listed in
`planted-conversations.txt`.

## Specimens handed forward (`specimens/`)

| specimen | provenance | for |
|---|---|---|
| `existing-archive/` | the product-produced output of `publication-crash-cut-post_rename`, captured post-rename and pre-cleanup — it therefore still carries the `.publishing.<uuid>` claim directory | L-PURGE's `existing-archive` arm (SC-810b), which the design names as this arm's output |
| `clean-archive/` | the published archive of `transaction-order-a-full-run`, a complete managed end | any consumer needing a clean real archive |

Both are copied mode-preserving; `*.manifest.tsv` records the recursive manifest of each
and `SHA256SUMS.txt` hashes every file. Consumers take their OWN fresh clone per
consumer, per the design's per-consumer separate-clone rule.

## Known limits of this section, stated

- The barrier named `b_confirm_answered` sits after the confirmation PHASE and fires
  whether or not a prompt ran (with `-f` there is no prompt); the name describes the
  window, not a guarantee that a human answered.
- `launch-rerun`'s execution 1 is ae's own, not the controller's: the frozen launch
  executes `launch.<slot>.sh` itself. The execution ledger in `marker-timeline.txt`
  states which execution produced which argv record rather than leaving it implicit.
- `archive-write-inability` restores the archive root to `0700` before its post-run
  manifest so the tree is readable; the mode in force during the run is recorded in
  `canary.txt`.
- The watchdog is left ENABLED (the launch default) in every arm, so events written by a
  running watchdog appear in the events deltas. It is recorded rather than suppressed.
- `handover`'s bounded wait expired by construction (no reply and no handover memo were
  supplied); what is recorded is the product's own reported outcome plus the full state
  at expiry. No absence is inferred from it.

## Harness stability at the section boundary

`L-END/harness-snapshot/` is a byte copy of the shared `_harness/` exactly as this
section ran, and `L-END/HARNESS-SHA256SUMS.txt` / `L-END/ADMISSIBILITY-SHA256SUMS.txt`
hash that snapshot and the admissibility proofs it rests on. Later sections may extend
the shared `_harness/` libraries additively; this section does not depend on that
top-level copy staying byte-identical, because its own snapshot is here. Nothing under
`L-END/`, `L-END/specimens/` or `L-END/harness-snapshot/` changes after this point.

## Correction (post-commit `7aab1b4`)

**What was wrong.** The controller's mutation idiom was `sed … > f.tmp && mv f.tmp f`.
That is writer-shaped, but the temp file is created under the default umask, so the
rename lands mode `644`. On a SESSION meta that is inert — a session meta is `644`
already. On an ARCHIVE meta it is not: archive members are mode `600` and the frozen
validator asserts exactly that (`_ar_validate_tree`, 72c7293:5218-5224). The mutation
therefore carried a SECOND, unnamed change that a content diff is structurally blind to.

**Which arm.** `compact-relaunch-lock-parent-mutated` (SC-808), and only that one. Its
parent archive meta ended at `644` where the control arm's is `600`, so the captured
outcome — the launch reporting that the parent archive stopped validating — is
attributable to EITHER the named `handover_count` diff OR the unnamed mode change. As
first captured it is **INADMISSIBLE** and must not be classified.

**Which arms are unaffected.** Every other arm in this section. The only other controller
mutation of a product file was `identity-uppercase-meta-uuid`, on a session meta
(`644` → `644`, measured against an unmutated session meta in a sibling arm), so its
observation never depended on the defect. Arms that plant new entries
(`staging-modes-planted-entry`, the `history-policy` conversation markers) create rather
than rewrite; their modes are umask-derived by construction and are recorded in the
recursive manifests rather than asserted.

**The fix.** `_harness/arm.sh :: l_rewrite_preserving_mode` captures the target's mode,
writes the temp, chmods the temp back to that mode, then renames — the same
temp + chmod-to-target-mode + rename shape ae's own
`_publish_executable_artifact` chokepoint uses. It is now used at EVERY site where the
controller rewrites a product file, not only the two where the defect was observed.

**What was re-run.** `compact-relaunch-lock-parent-mutated` (the affected arm) and
`identity-uppercase-meta-uuid` (unaffected, re-run so the harness is uniform and the mode
is explicitly recorded). Both now carry an explicit `mode.before` / `mode.after` record:
`b*-b_from_proved.*.parent-meta.mode.txt` (`600` → `600`) and `mutation.txt`
(`644` → `644`) respectively. The arm directories in this working tree hold the
REPLACEMENT captures; the contaminated captures remain in commit `7aab1b4` and are
superseded by these.

**How it surfaced.** An L-PURGE arm found it, not a review of L-END: SC-818d's `--from`
subarm reported `'meta' has mode 644, expected 600` instead of anything about the field
it had emptied. A content diff could not have shown it, which is why it survived review
here.
