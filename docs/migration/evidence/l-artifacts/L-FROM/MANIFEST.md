# Batch L — section L-FROM artifacts — MANIFEST

Worker: `opus5:lexec`. Maps **arm → row ids → artifact paths → fixtures → mutation diffs**
for the L-FROM (lineage) evidence run. Captures only: bytes, hashes, recursive manifests,
byte diffs, rc values, full state sweeps, frozen-source extracts. No verdicts, no
expected-vs-actual statements. Seats classify.

## Run-wide provenance

Frozen commit `72c729343a0117af2968b66e1c43f89ad25fc0b2`, frozen `ae` sha256
`b7b8aa9f…`; tool hashes in `_harness/env-record.txt`; `LANG=LC_ALL=en_US.UTF-8`,
`TZ=UTC`; no live models, no network. Each arm in its own disposable sandbox under
`/tmp/aelx/L-FROM/<arm>/`.

**Fixtures are real parent archives from real ends.** `_harness/from-lib.sh :: make_parent`
launches a real `--local` session, ends it for real, and refuses to proceed unless the
archive directory exists afterwards; the parent's archive meta is copied into every arm as
`parent-archive-meta.txt` and its name and uuid into `parent.txt`. No archive is
hand-built and none is copied between sandboxes.

## Instrumented copies used

Measured from the tree: **2 of the 12 arms ran under L-HOOKS-v3** — `transport-cut` and
`minted-at-end` — and the other 10 ran on the unmodified frozen binary. `ARM.txt` in every
arm carries `hook_patch_version` and the binary's own sha256.

| field | value |
|---|---|
| L-HOOKS-v3 instrumented `ae` sha256 | `b1b07709b01a66f7467333a151e641d2139b6950e58ca4cd53b47aa020afdfdd` |
| L-HOOKS-v3 patch sha256 | `9243b21168b2efe1c031856c36651c8f5c230af269fa4e8029acff1563c7ad6c` |

The two barriers this section uses are `b_from_proved` (present since v1) and
`b_pre_cleanup` (present since v1); v3 is used simply because it is the superset copy
whose admissibility is already proven. Its proofs are `_admissibility/equiv-I-v3-inactive-compact.txt`
(NO_DIFFERENCES) with its control `equiv-I-known-difference.txt` (DIFFERENCES_PRESENT),
alongside the v1 and v2 proofs.

## Common capture set (every arm)

`ARM.txt` · `<step>.stdout` / `.stderr` / `.rc` / `.invocation` ·
`1pre.*` / `3post.*` recursive manifests of `AE_HOME`, `sessions/`, `archive/` and
`worktrees/`, plus a tmux snapshot · `<label>.full-sweep.txt` — a FULL sweep listing
session dirs, worktree dirs, archive dirs, tmux sessions, the `AE_HOME` top level and the
work dir, whether or not each exists · `preflight-tab.txt` (blocking
environment-as-instrument proof, in the arm's own `env -i`) · `consumer-inproc.txt` +
`tmux-argv.log` (rule (e)) · `SHA256SUMS.txt`. Lineage arms add `lineage_fields` captures:
the child's meta read **BY EXACT KEY** (`session_id`, `session_id_origin`,
`parent_archive_id`, `parent_archive_handover_count`, `parent_archive_pending_count`,
`session`, `origin`), the whole meta verbatim, and the `workspace.md` lineage lines.

Measured across this section: **0 preflight failures, 0 `ARM-INVALID.txt`, 0
`INCONCLUSIVE.txt`.**

## Arms

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `name-never-infers` | SC-809 | a real parent archive is produced by a real end of a session named `same`; a NEW session is then launched under that SAME name **without** `--from` | `parent.txt`, `child-lineage.txt` (meta by exact key + the whole meta), `child-workspace.md`, `1pre/3post.full-sweep.txt` |
| `existing-target-running` | SC-822 | `--from` onto a name that is a RUNNING tmux session launched for real | `target.txt`, `2op.*`, full before/after manifests, `aehome.before-after.diff`, `target-lineage.txt` |
| `existing-target-stopped` | SC-822 | `--from` onto a name whose STOPPED session directory is on disk | same set |
| `existing-target-worktree` | SC-822 | `--from` onto a name that is a LEFTOVER WORKTREE: a real `--worktree` launch, stopped, then its session state directory removed by the controller so only the worktree remains | same set |
| `invalid-parent-nonexistent` | SC-823 | `--from` names a well-formed archive uuid that names no archive in the root | `mutation.txt`, `1pre/3post.full-sweep.txt`, `full-sweep.diff`, `child-lineage.txt` |
| `invalid-parent-validation-failing` | SC-823 | ONE named mutation makes the real parent archive fail validation — the archive meta's `handover_count` is set to `99` so meta and digest disagree, mode preserved | `mutation.before/after.txt`, `mutation.diff`, `mutation.txt`, `full-sweep.diff` |
| `transport-cut` | SC-824a | the PLAIN launch path (`ae --local child --from <parent>`); an instrumented barrier sits immediately after the launch parses its FIRST parent proof, the controller deletes the parent archive there and releases, and the launch resumes | `<tag>.controller.txt`, `<tag>.archive.before-delete.tsv` / `.after-delete.tsv` / `.delete.diff`, `child-lineage.txt`, `source-trace.parent-archive-reads-after-the-barrier.txt` (+ its sha256) |
| `mid-publication` | SC-824b | a `.publishing.<parent-uuid>` claim directory (mode 0700) is planted on the PARENT under the archive root, then `--from` runs against that parent | `archive.before-plant.tsv`, `archive.after-plant.tsv`, `plant.diff`, `mutation.txt` |
| `lineage-durability-stop-resume` | SC-825a | a successful `--from` child is stopped and resumed with no manipulation in between | `lineage.1after-from.txt`, `lineage.2after-stop.txt`, `lineage.3after-resume.txt`, `lineage.across-cycle.diff`, `workspace.1after-from.md`, `workspace.3after-resume.md` |
| `lineage-durability-move-aehome` | SC-825b | the WHOLE `AE_HOME` is moved — a mode-preserving copy at a new absolute path — and the resume runs with `AE_HOME` pointing there | same set plus `manipulation.txt`, `moved-aehome.tsv`; the post-resume lineage read includes an `od -c` of the `parent_archive_id` line |
| `lineage-durability-delete-parent` | SC-825c | the PARENT archive is removed while the child is stopped, then the child is resumed | same set plus `archive.before-delete.tsv`, `archive.after-delete.tsv`, `archive.delete.diff` |
| `minted-at-end` | SC-826 | a real live session has its `session_id` KEY REMOVED ENTIRELY (the legacy shape) by one named mode-preserving mutation, then `end` runs; the LIVE meta is captured at the `b_pre_cleanup` barrier — before cleanup removes the directory — and the ARCHIVE meta afterwards | `mutation.txt`, `mutation.diff`, `live-meta.at-pre-cleanup.txt`, `live-meta-id-keys.txt`, `archive-meta-id-keys.txt` |

### SC-826 read by exact key, and the key that is NOT it

`live-meta-id-keys.txt` reads `session_id_origin` from the LIVE meta by exact key;
`archive-meta-id-keys.txt` reads `archive_id_origin` from the ARCHIVE meta by exact key.
Both files also list **every** key whose text contains `origin`, and say in the file that
the repository key `origin=` is a different key shown only so the two can never be
confused. The mutation is deliberately DISTINCT from SC-819's unparseable class: here the
key is ABSENT, there the key is present with an unparseable value.

### SC-824a's boundary, stated

This arm exercises the PLAIN launch path only. It carries no re-proof expectation and no
rollback machinery of its own. **SC-808's re-proof surface is L-END's
`compact-relaunch-lock-parent-mutated` arm; it is REFERENCED here and NOT re-executed.**
The `source-trace.*` file is a frozen-source extract with line numbers — every line after
the first proof that names `PARENT_ARCHIVE_ID`, `FROM_ARCHIVE`, `_ar_from_preflight`,
`_AE_FROM_EXPECTED` or the archive root, plus `_ar_from_preflight` in full. It is a code
observation, not a runtime trace, and it carries no verdict.

## Roster coverage as executed

All 9 L-FROM roster ids have an arm above: SC-809, SC-822, SC-823, SC-824a, SC-824b,
SC-825a, SC-825b, SC-825c, SC-826.

## Known limits of this section, stated

- Every non-pty launch in this section ends with the frozen attach failing (there is no
  terminal), so those invocations' rc reflects the attach and not the session creation.
  Each arm records the created state separately in its manifests and full sweeps rather
  than resting on the rc.
- `existing-target-worktree` reaches its shape through a controller removal of the session
  state directory after a real stopped `--worktree` launch; that removal is the named
  construction and is stated in `target.txt`.
- The watchdog is left ENABLED (the launch default) in every arm.

## Harness stability at the section boundary

`L-FROM/harness-snapshot/` is a byte copy of the shared `_harness/` exactly as this
section ran, hashed by `L-FROM/HARNESS-SHA256SUMS.txt`;
`L-FROM/ADMISSIBILITY-SHA256SUMS.txt` hashes the admissibility proofs it rests on.
Nothing under `L-FROM/` changes after this point.
