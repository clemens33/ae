# Lock/atomicity census — lifecycle, daemon, and remaining operations (P2 scope)

> Scope (lead note, 2026-08-20): this census covers the operations named for census-2:
> interrupt, focus, launch (including `--from`), end/rm and archive publication, stop,
> rename, transfer, compact, doctor `--refresh`, watchdog controls and daemon loop,
> Telegram setup/start/stop and daemon loop, steward, `_recover-pending`, request WITHDRAW,
> and the pre-dispatch config bootstrap. The helper/write-domain operations in
> `locks-census.md` are a separate census. Python `aemonitor` state writes are excluded.

- Source commit: `72c729343a0117af2968b66e1c43f89ad25fc0b2`
- Source commit date: 2026-08-20T08:49:06+02:00
- Census date: 2026-08-20
- Agent: `gpt56luna:census2`

Line references below are to `ae` at the source commit. “Crash residue” describes files and
directories present when execution stops at the indicated point; an open `flock` descriptor is
closed by process exit, while the lock-file path created by redirection remains on disk.

## `interrupt` (`helper_interrupt_main`)

### Locks and acquisition order

- It resolves the pane, then acquires the per-target lock with `ae_lock_target` at
  `ae:14664-14678`. `ae_lock_target` creates `${_AE_SESSIONS_DIR}/.locks`, opens
  `send-lock-${pane}` on fd 9, and uses blocking `flock 9` with no timeout at `ae:12993-13000`.
- The target fd 9 remains held across `tmux send-keys -X cancel`, Escape, and the optional
  paste at `ae:14680-14696`.
- `ae_emit_event` is called before `ae_unlock_target` at `ae:14699-14700`. It acquires the
  events lock on fd 8 with `flock -w 5` and appends through `ae_log_append` at
  `ae:13171-13176`. Order is target fd 9, then events fd 8 while target fd 9 is held.
- A failed optional paste explicitly unlocks fd 9 and exits before event emission at
  `ae:14689-14695`.

### Write sequence

- `tmux send-keys` cancellation and Escape are issued at `ae:14682-14683`; an optional
  `ae_submit_pasted_message` performs tmux buffer/paste/Enter operations at `ae:14685-14696`.
- The event JSON is built in memory and appended directly to `events.jsonl` by `ae_emit_event`
  → `ae_log_append`; this is the `ae_log_append` writer, with `-w 5`, not `_spawn_emit_event`
  (`ae:13208-13256`, `ae:13171-13176`). No body file is written by `interrupt`.

### Crash residue

- Interruption after `ae_lock_target` creates `.locks/send-lock-*`; the descriptor closes on
  process exit. Before the event call, tmux cancellation, Escape, and any completed paste remain
  while no interrupt event exists (`ae:14678-14699`).
- During event append, `events.jsonl.lock` remains and a direct `printf >>` can leave an
  unterminated event line (`ae:13171-13176`).
- A failed paste leaves the tmux cancellation/Escape side effects and no interrupt event
  (`ae:14692-14695`).

## `focus` (`helper_focus_main`)

### Locks and acquisition order

- No target, lifecycle, or metadata lock is acquired. Target resolution is at `ae:14648`.
- `tmux select-window` and `tmux select-pane` run at `ae:14649-14652`; then `ae_emit_event`
  acquires the events fd 8 lock with `flock -w 5` through `ae_log_append` at
  `ae:14653`, `ae:13171-13176`.

### Write sequence

- tmux focus side effects occur first (`ae:14651-14652`).
- The `focus` event is an in-place append to `events.jsonl` through `ae_log_append`; no body,
  metadata, or helper file is written (`ae:14653`, `ae:13208-13256`).

### Crash residue

- A stop after either tmux selection leaves the selected window/pane and no focus event
  (`ae:14651-14653`).
- During the event append, the event lock path remains and `events.jsonl` can contain a partial
  line (`ae:13171-13176`).

## Session launch (`ae [name]`, including `--from`)

### Locks and acquisition order

- Flag parsing, session-name validation, and `--from` archive preflight run before the launch
  lifecycle lock. The parser assignments are at `ae:16941-16945`; name validation is at
  `ae:16949-16972`; the preflight is at `ae:16974-16998` and `_ar_from_preflight` reads the
  archive root/tree and emits the frozen counts without a lock (`ae:5470-5521`).
- The existing-running-session fast path at `ae:17052-17070` takes no lifecycle lock. Its
  Telegram autostart can take the machine-global control lock (`ae:10486-10524`); steward
  autostart takes no lock (`ae:10357-10400`); `tmux_attach` is then a tmux side effect with no
  ae lock (`ae:17052-17066`).
- Work-directory preparation precedes the lifecycle lock: local resume or `mkdir`, git
  worktree creation, or full copy occurs at `ae:17215-17271`.
- The launch opens `${SESSIONS_DIR}/.lifecycle.${SESSION}.lock` on fd 8 and uses
  `flock -w 15 8` at `ae:17282-17300`. If `flock` is unavailable, no lifecycle lock is
  acquired. This fd remains held through tmux creation, metadata, helper/manifest generation,
  launch scripts, and launch-time sends; it is released at `ae:18168-18179`.
- The first tmux client call closes fd 8 in its child at `ae:17301-17305`; subsequent tmux
  session/environment/window/pane operations run while the parent lifecycle fd 8 is held
  (`ae:17306-17404`).
- Initial metadata updates use the lifecycle fd 8 first. `sync_session_assets` calls
  `set_session_meta_value`, which acquires `${meta}.lock` on fd 200 with `flock -w 5` and
  temp+mv at `ae:2359-2436`; these acquisitions occur while the lifecycle fd 8 is held
  (`ae:17628-17640`, `ae:18039`).
- The initial canonical meta is built without `meta.lock`: stale `meta.tmp.*` is removed, the
  file is written/appended, and then moved while lifecycle fd 8 is held (`ae:17539-17628`).
- `sync_session_assets` removes listed orphan helper files with direct `rm -f` before emitting
  replacements (`ae:17675-17683`); the launch lifecycle fd 8 remains held around this call.
- On resume, events trimming acquires `events.jsonl.lock` on fd 9 with `flock -w 5`, while
  lifecycle fd 8 remains held; it reads/counts and tail+mv's the log at `ae:18046-18075`.
  Order for this path is lifecycle fd 8, then events fd 9.
- Helper and manifest generators run while lifecycle fd 8 is held and publish temp+chmod+mv
  artifacts; representative generated helper sites are `ae:17798-17810` and
  `ae:17909-18032`, and the manifest write is `ae:12267-12339`.
- `send_agent_cmd` launch delivery runs while lifecycle fd 8 is held through
  `_send_or_rollback` at `ae:18114-18166`. For normal pane delivery its helper target lock is
  acquired under that lifecycle lock, then released before the body-file write and event
  append (`ae:14235-14283`). Launch failure reporting uses `_spawn_emit_event` with an events
  fd 8 `flock -w 5` at `ae:12689-12693`; the Codex deferred-delivery child closes its inherited
  lifecycle fd before this call (`ae:12622-12635`).
- After fd 8 is released, background session-id capture may acquire `${meta}.lock` fd 200
  with `flock -w 5` and perform `_ae_sed_inplace`'s copy/sed-to-temp then mv rewrite
  (`ae:152-171`, `ae:2068-2076`, `ae:1860-1865`, `ae:2020-2025`, `ae:2049-2054`).

### Write sequence

- Before lifecycle acquisition, partial worktree/copy content can remain; the generic
  `--from` preflight itself has no write (`ae:17215-17271`, `ae:5470-5521`).
- While lifecycle fd 8 is held, tmux creates the session and panes and writes session/window/
  pane options (`ae:17305-17404`). The session metadata directory is created and chmod'd 0700
  at `ae:17406-17415`.
- A fresh initial `meta` is assembled in `${AE_META}/meta.tmp.$$` by heredoc and appends at
  `ae:17539-17606`, then a second `--from` proof can remove the temp and invoke rollback at
  `ae:17607-17627`; the canonical `meta` appears by one `mv` at `ae:17628`.
- `sync_session_assets` updates selected metadata keys with `set_session_meta_value` (fd 200,
  awk to temp, mv) and emits helper artifacts via temp+chmod+mv (`ae:17640-18037`).
  `regenerate_manifest` writes `workspace.md` directly with `cat >` (`ae:12267-12339`), not
  temp+mv. It also stamps missing live-pane `@ae_slot` options through tmux without a file lock
  (`ae:18025-18032`, `ae:13088-13140`).
- Resume event retention uses `tail -n ... > events.jsonl.trim.$$ && mv` under the events lock
  (`ae:18046-18075`).
- Launch scripts are generated and launch prompts are sent before release (`ae:18114-18166`).
  A normal launch send uses tmux side effects, then `ae_store_message_body` (unique mktemp,
  rename to `.txt`, in-place `printf`, chmod) and `ae_emit_event` after target-lock release
  (`ae:13322-13359`, `ae:14235-14283`). A failed launch delivery writes
  `undelivered.launch-<slot>.txt` directly and emits via `_spawn_emit_event`
  (`ae:12689-12693`).
- Post-release capture appends `launch_time.<slot>` directly under meta fd 200 at
  `ae:2068-2076`; capture rewrites `meta` with `_ae_sed_inplace` temp+mv under the same lock
  (`ae:152-171`, `ae:1860-1865`, `ae:2020-2025`, `ae:2049-2054`). Watchdog/Telegram/steward autostart calls then perform
  their own operation-specific writes and tmux side effects (`ae:18180-18233`).

### Crash residue

- Before lifecycle acquisition, partial worktree/copy content can remain; the generic
  `--from` preflight itself has no write (`ae:17215-17271`, `ae:5470-5521`).
- During tmux setup, a tmux session, windows, panes, options, or environment can remain before
  `meta` exists (`ae:17305-17404`). The lifecycle lock path remains.
- During initial metadata assembly, `${AE_META}/meta.tmp.$$` can be partial; an existing
  canonical `meta` is unchanged until the final mv (`ae:17546-17628`). On a fresh directory,
  the directory can contain no canonical meta.
- During helper generation, individual `*.tmp.$$` files can remain; already-published helpers
  remain and not-yet-published helpers are absent or retain their prior versions. The direct
  `workspace.md` write can be partial (`ae:17798-17810`, `ae:12267-12339`).
- Orphan helper files selected by the cleanup list can already be removed before a later helper
  publish is interrupted; live-pane slot stamping can stop after only some tmux panes have
  received `@ae_slot` (`ae:17675-17683`, `ae:13088-13140`).
- During resume trim, `events.jsonl.trim.$$` can remain; after mv the log inode is the trimmed
  one. The lifecycle and events lock paths remain (`ae:18060-18075`).
- After canonical meta but before helper completion, tmux and meta can coexist with an
  incomplete helper set. `_launch_rollback` kills tmux and removes only a directory created by
  this attempt at `ae:16861-16875`; process interruption does not invoke that function.
- After lifecycle release, a capture child can leave `launch_time.*`, `.sid`, or a
  `${meta}.sedtmp.$$`; the prior `meta` remains until the capture mv, and the meta lock path
  remains (`ae:152-171`, `ae:1860-1865`, `ae:2020-2025`, `ae:2049-2054`).
- On the existing-running-session path, Telegram control-file/daemon residues and steward
  child-launch residues follow the delegated operations; `tmux_attach` itself has no file
  residue (`ae:17052-17070`).

## End/rm (`cmd_end`/`end_session`, including archive publication)

### Locks and acquisition order

- `cmd_end` handles `end` and `rm` and resolves/plans targets before each `end_session` call
  (`ae:8356-8495`). For `all`, each target is locked independently; there is no fleet-wide
  outer lock.
- `end_session` opens `.lifecycle.<name>.lock` on fd 9 and uses `flock -w 15 9` at
  `ae:2879-2894`. It calls `_end_session_locked` while fd 9 remains held.
- `_end_session_locked` holds lifecycle fd 9 across target proof, tmux kill/verification,
  git commit/fetch/push, archive publication or purge, and cleanup (`ae:2896-3137`).
- If archive planning mints a UUID, `set_session_meta_values` acquires `${meta}.lock` fd 200
  with `flock -w 5` while lifecycle fd 9 remains held at `ae:8142-8161`. Order is lifecycle
  fd 9, then meta fd 200.
- `_ar_publish` and `_ar_purge_archive` use a UUID claim directory created by `mkdir -m 0700`
  rather than an `flock` (`ae:5285-5303`, `ae:5404-5425`). No additional flock is acquired
  by these archive functions.

### Write sequence

- Local mode verifies/kills tmux, calls `_end_archive_step`, then `cleanup_session`
  (`ae:3014-3027`). Git/full mode verifies/kills tmux, performs git add/commit/fetch/push,
  then calls `_end_archive_step` and cleanup (`ae:3031-3137`). These tmux/git operations occur
  under lifecycle fd 9.
- `_end_archive_step` plans under the held lifecycle lock, optionally writes `session_id` and
  `session_id_origin` using meta fd 200 temp+mv (`ae:8126-8164`), then selects purge or publish.
- Publication creates `${archive-root}/.publishing.<uuid>`, stages `payload/meta`,
  `payload/memo.tsv`, `payload/events.jsonl`, `payload/messages/*`, and `payload/digest.md`
  using direct `cat`/redirection and chmod, validates, then renames `payload` to the final
  UUID directory (`ae:5285-5340`, `ae:5346-5393`). The final `mv` is the only payload-to-target
  publication step.
- `_ar_stage_payload` copies `memo.tsv` and `events.jsonl` with direct `cat` while lifecycle
  fd 9 is held; it does not acquire the source `events.jsonl.lock` (`ae:5356-5365`).
- Purge creates the same claim directory, validates the existing archive and source identity,
  then runs `rm -rf` on the target (`ae:5404-5463`).
- `cleanup_session` removes conversation files if configured, heartbeat, session metadata, and
  worktree/copy state after archive/purge success (`ae:2765-2836`). It acquires no additional
  lock and runs while lifecycle fd 9 is held.
- End/rm itself does not append a new event. Existing `events.jsonl` is copied verbatim into
  the archive by `_ar_stage_payload` (`ae:5356-5365`); no `ae_log_append`, `_spawn_emit_event`,
  or inline `-w 15` event writer is used.

### Crash residue

- Before lifecycle acquisition, target planning and prompts have no write from end/rm. The
  lifecycle lock path is created at `ae:2881-2893`.
- After verified tmux kill and before archive publication, the tmux session is gone while
  metadata/worktree state remains; a git commit/push may already be present (`ae:3014-3134`).
- During publication, `.publishing.<uuid>/payload` can contain partial staged files; the final
  archive target does not appear until `mv payload target` (`ae:5291-5328`). The claim path
  remains if execution stops before `rmdir`.
- After final archive mv and before cleanup, the complete archive and the source session state
  coexist (`ae:5323-5328`, `ae:8200-8209`).
- During cleanup, archive files remain while session metadata, heartbeat, conversation files,
  or worktree/copy can be partially removed (`ae:2765-2836`).
- During purge, the claim path can remain before target removal; after `rm -rf target`, the
  archive is absent and source cleanup has not yet run (`ae:5422-5462`).

## `stop` (`cmd_stop` and supervisors)

### Locks and acquisition order

- Singular stop dispatches to `_stop_one_session`; it acquires the per-session lifecycle lock
  `.lifecycle.<name>.lock` on fd 9 with `flock -w 15 9` at `ae:7534-7569`.
- `_stop_session_locked` performs identity checks and verified tmux kill while lifecycle fd 9
  is held; no metadata lock is taken by this path (`ae:7914-7952`).
- The self/supervised stop path starts a detached supervisor without holding a lifecycle lock
  in the caller (`ae:7015-7056`). `_cmd_stop_supervisor` first emits `stop-request` with
  `_spawn_emit_event`, whose events lock is fd 8 with `flock -w 5`, then calls `_stop_one_session`,
  then emits `stop-result` after lifecycle release (`ae:7343-7390`, `ae:12092-12115`). Order is
  events fd 8 → lifecycle fd 9 → events fd 8; no lock is held across the entire sequence.
- Fleet freeze first reads each meta without a lock; for an unassigned ID it acquires the
  lifecycle lock on fd 7 with `flock -w 5`, rereads, and may call `set_session_meta_value`,
  which acquires meta fd 200 while lifecycle fd 7 remains held (`ae:7420-7465`). Order is
  lifecycle fd 7 → meta fd 200.
- Fleet supervisor/refusal paths use `_spawn_emit_event` independently for request/result rows
  (`ae:7254-7284`, `ae:7319-7339`); no lifecycle lock is held by refusal events.

### Write sequence

- A stop request/result event is built and appended directly by `_spawn_emit_event` with
  `flock -w 5` on `events.jsonl.lock` (`ae:12092-12115`). This is `_spawn_emit_event`, not
  `ae_log_append` and not the inline `-w 15` variant.
- The stop operation has tmux side effects: `_lifecycle_kill_verified` kills and verifies the
  exact tmux session; `_stop_session_locked` also removes the watchdog heartbeat before kill
  (`ae:7891-7904`, `ae:7914-7952`). No session metadata, worktree, or archive is removed.
- Fleet freeze may write `session_id` through `set_session_meta_value` using temp+mv
  (`ae:7450-7459`).

### Crash residue

- A supervisor stopping after `stop-request` leaves that event and no result if it exits before
  the result append (`ae:7372-7388`). The events lock path remains.
- A stop during the lifecycle critical section leaves `.lifecycle.<name>.lock`; tmux can be
  absent after a completed kill, or still present after a failed/unverified kill. The session
  metadata/worktree remains (`ae:7891-7952`).
- Heartbeat removal precedes the verified kill (`ae:7948-7949`); interruption after removal
  leaves no heartbeat with the remaining tmux/session state.
- Fleet freeze can leave `meta.lock`, `meta.tmp.$$`, or a completed metadata rewrite when ID
  minting is interrupted (`ae:7450-7459`, `ae:2359-2436`).

## `rename` (`cmd_rename`)

### Locks and acquisition order

- It validates the new name and resolves safe old/new paths before locking at `ae:11547-11596`.
- It opens two lifecycle lock files, sorted lexically, on fd 9 and fd 10, and acquires each
  with `flock -w 15` at `ae:11597-11608`. No meta lock is acquired.
- `_cmd_rename_locked` runs while both lifecycle descriptors are held; no later lock is taken
  (`ae:11614-11671`).

### Write sequence

- tmux session rename and optional window rename occur first (`ae:11634-11645`).
- The session metadata directory is moved with `mv old_dir new_dir` at `ae:11648-11651`.
- The `session=` key in the new metadata is rewritten by `_ae_sed_inplace` using its temp copy,
  sed output, and mv sequence at `ae:152-171`, called at `ae:11653-11656`.
- `regenerate_manifest` then rewrites `workspace.md` directly with `cat >` at
  `ae:11658-11667`, `ae:12267-12339`; the status bar is applied after that.
- Rename appends no event and uses no `ae_log_append`, `_spawn_emit_event`, or inline event
  writer.

### Crash residue

- After tmux rename and before metadata-directory mv, tmux has the new name while the old
  metadata directory remains (`ae:11634-11651`).
- After metadata mv and before `_ae_sed_inplace`'s temp+mv rewrite, the new directory can contain metadata
  whose `session=` value is still the old name (`ae:11648-11656`).
- During the sed rewrite, `${meta}.sedtmp.$$` can remain; the prior `meta` remains until the
  temp+mv completes. The two lifecycle lock paths remain (`ae:11653-11656`, `ae:152-171`,
  `ae:11597-11608`).
- During manifest generation, `workspace.md` can be partial because `regenerate_manifest` uses
  direct `cat >`; tmux and metadata changes remain (`ae:11658-11667`, `ae:12339`).

## `transfer` (`cmd_transfer`)

### Locks and acquisition order

- Argument/name/path validation, SSH probes, remote checks, collision checks, and UUID
  discovery run without a transfer-wide lock (`ae:11116-11431`).
- Local stop delegates to `cmd_stop` and therefore takes the stop lifecycle lock only for the
  stop operation (`ae:10832-10846`, `ae:7534-7569`). Remote stop invokes remote `ae stop`,
  taking the corresponding remote lifecycle lock (`ae:10848-10862`). No lock remains held
  across rsync.
- Destination event append is separate from stop and rsync. Pull uses a local inline writer:
  fd 9 on `${events_file}.lock`, `flock -w 5`, direct append (`ae:11097-11114`). Push uses
  the remote inline writer with fd 9 and `flock -w 5` (`ae:11003-11023`).

### Write sequence

- Stop calls complete before state transfer (`ae:11432-11442`).
- Pull creates the local session directory and rsyncs remote state with `--delete`, excluding
  lock/pid/status files (`ae:11443-11457`). Push creates the remote session directory and
  rsyncs local state with the same options (`ae:11459-11473`).
- Conversation files are copied with per-file `mkdir` and rsync, with no temp+mv publication
  (`ae:11475-11515`).
- The destination transfer event is a direct JSON line append under the inline `flock -w 5`
  writer (`ae:11516-11537`). It is not `ae_log_append` or `_spawn_emit_event`.

### Crash residue

- Before stop, no transfer destination write has run. A completed local/remote stop can leave
  stop-request/result events and stopped session state before transfer starts (`ae:11432-11442`).
- During state rsync, the destination directory can contain a partial or mixed file set because
  rsync writes directly; source state remains on the source side (`ae:11443-11473`).
- During conversation-file rsync, a destination conversation file or parent directory can be
  partial (`ae:11475-11515`).
- During event append, the destination events lock path remains and the direct append can leave
  a partial line (`ae:11003-11023`, `ae:11097-11114`).

## `compact` (`cmd_compact`)

### Locks and acquisition order

- Freeze, archive preflight, and confirmation occur without a lifecycle lock. If the source has
  no UUID, `_compact_freeze_source` calls `set_session_meta_values`, acquiring meta fd 200
  with `flock -w 5` and temp+mv, without lifecycle fd (`ae:5647-5745`, `ae:2359-2436`).
- Handover publication opens `.lifecycle.<name>.lock` on fd 8 and uses `flock -w 15 8` at
  `ae:6272-6313`. While it is held, `_compact_send_handover` delegates to the generated
  `ask`/`send` path. The normal pane path acquires target fd 9, releases it before body/event,
  then acquires events fd 8 with `flock -w 5`; lifecycle fd 8 remains held by compact
  (`ae:5912-5921`, `ae:14235-14283`, `ae:13171-13176`).
- Request withdrawal uses the same compact lifecycle fd 8 and delegates to `send` with the
  external `ae:compact:` target. The external send path skips target fd 9 and takes only the
  events fd 8 `flock -w 5` through `ae_log_append` (`ae:5961-5969`, `ae:14180-14194`,
  `ae:13171-13176`).
- Compact releases the handover lifecycle lock before the bounded wait (`ae:6313-6319`).
- End phase opens the same lifecycle lock on fd 9 and uses `flock -w 15 9` at `ae:6352-6390`.
  It calls `_end_session_locked` while fd 9 is held; archive UUID minting can then take meta
  fd 200 (`ae:8142-8161`). Order is compact lifecycle fd 9 → meta fd 200.
- After end/archive and lifecycle release, `--from` preflight is read-only and the command
  `exec`s a fresh generic launch, which takes the launch lifecycle fd 8 as described above
  (`ae:6395-6459`, `ae:17282-17300`).

### Write sequence

- UUID migration in the freeze phase rewrites meta using awk to `${meta}.tmp.$$` and mv
  (`ae:5710-5716`, `ae:2359-2436`).
- Handover `ask` writes the full request body through `ae_store_message_body`: unique mktemp,
  rename of the empty temp to `.txt`, in-place body `printf`, chmod (`ae:13322-13359`), then
  appends an `ask` event through `ae_log_append` (`ae:14270-14283`, `ae:13171-13176`).
- Request withdrawal delegates to `send` with `_AE_EVENT_ACTION=cancel`; for the external
  `ae:compact:` target it emits only an event through `ae_emit_event` and does not write a body
  file or perform tmux operations (`ae:5961-5965`, `ae:14180-14194`).
- After the wait, compact calls `_end_session_locked`, whose verified tmux stop, git steps,
  archive publication/purge, and cleanup write sequence is the end sequence above
  (`ae:6354-6390`, `ae:8126-8210`).
- The boundary prints recovery information, then launches `ae ... --from <uuid>` by `exec`
  (`ae:6395-6459`). The child launch performs the second archive proof before moving its
  canonical meta (`ae:17539-17628`).

### Crash residue

- Freeze-time UUID migration can leave `${meta}.tmp.$$` or the prior meta file; `meta.lock`
  remains (`ae:5710-5716`, `ae:2359-2436`).
- During handover request body storage, `messages/` and an empty/partial final body file can
  remain without an event; during event append, `events.jsonl` can have a partial line and its
  lock path remains (`ae:13322-13359`, `ae:13171-13176`).
- A handover wait runs with no lifecycle lock. A stop during this interval leaves the request/
  memo observations and the live session state at their respective write points (`ae:6313-6347`).
- During the end phase, archive claim/payload and cleanup residues are those in the end section.
  After archive publication and source cleanup, the source is gone; interruption before the
  relaunch can leave only the archive and no child session (`ae:6395-6459`).

## `doctor --refresh`

### Locks and acquisition order

- `cmd_doctor --refresh` calls `doctor_refresh_sessions` for each session without a
  lifecycle-wide lock (`ae:8919-8934`, `ae:8874-8917`).
- Legacy tmux-server migration in `sync_existing_session_assets` acquires `${meta}.lock` on
  fd 201 with `flock -w 5`, then writes `${meta}.tmp.$$` and mv's it at `ae:8550-8587`.
- `sync_session_assets` then calls `set_session_meta_value` for individual keys; each call
  acquires `${meta}.lock` fd 200 with `flock -w 5` and temp+mv (`ae:8536-8610`,
  `ae:2359-2436`). No lifecycle lock is held across these calls.
- If a running watchdog is detected, refresh invokes its helper `stop` and `start` without an
  outer lock (`ae:8621-8680`). Those helpers take meta fd 200 only when recording the final
  `watchdog=false/true`; their tmux operations occur before that lock (`ae:15031-15105`,
  `ae:14934-14948`).
- Pending session-ID recovery called by refresh takes meta fd 200 with `flock -w 5` and rewrites
  the matching meta line through `_ae_sed_inplace` temp+mv (`ae:152-171`, `ae:8690-8743`).

### Write sequence

- Refresh scans/reads metadata and may migrate `tmux_server` by awk-to-temp+mv (`ae:8550-8587`).
- It updates selected metadata keys through `set_session_meta_value` (temp+mv), emits helper
  scripts with temp+chmod+mv, removes the old helper names listed by `sync_session_assets`,
  writes `workspace.md` directly with `cat >`, stamps live pane slots with tmux options, and
  applies the status bar (`ae:8536-8620`, `ae:12267-12339`, `ae:13088-13140`).
- A live watchdog is stopped (pid kill, pidfile removal, tmux pane/status options), then
  restarted (monitor/pane creation), then metadata watchdog state is recorded through the
  meta-lock writer (`ae:8621-8680`, `ae:15031-15105`).
- Recovery updates pending `agent.<slot>` with `_ae_sed_inplace` temp+mv under meta fd 200
  (`ae:152-171`, `ae:8717-8732`).
- Refresh itself emits doctor reports only; it does not append an ae event. The standalone
  recovery reporter prints rows; no `ae_log_append` or `_spawn_emit_event` is used by that
  reporter (`ae:8801-8837`).

### Crash residue

- Migration or metadata updates can leave `meta.tmp.$$` with the old `meta` still present;
  after mv the new metadata inode is visible (`ae:8550-8587`, `ae:2359-2436`).
- Helper generation can leave individual helper temp files; completed artifact renames remain,
  while direct `workspace.md` generation can be partial (`ae:17798-17810`, `ae:12267-12339`).
- The refresh can remove orphan helper paths before a later artifact fails, and can leave only a
  subset of live panes stamped with `@ae_slot` (`ae:17675-17683`, `ae:13088-13140`).
- Watchdog stop can leave the pidfile absent, status options cleared, and the watchdog pane
  absent before start completes; start can leave a monitor/watchdog pane and no metadata state
  if interrupted before `_set_meta_watchdog` (`ae:15031-15105`).
- Recovery can leave `meta.lock` and `${meta}.sedtmp.$$`; `meta` is replaced only by the final
  mv (`ae:152-171`, `ae:8721-8732`).

## `_recover-pending` (standalone command and watchdog path)

### Locks and acquisition order

- Standalone `cmd_recover_pending` hydrates config/origin, then walks pending slots without an
  outer lock at `ae:8853-8872`.
- For each matching slot, `doctor_try_capture_session_id` acquires `${meta_dir}/meta.lock`
  on fd 200 with `flock -w 5` at `ae:8721-8732`. It reads the pending value while holding
  that lock, then rewrites the matching line. No lifecycle or events lock is acquired.
- The watchdog invokes the standalone `ae _recover-pending` subprocess at `ae:16528-16536`.
  The subprocess takes only meta fd 200 for each successful capture. After it exits, the
  watchdog emits a `recover` event through `ae_emit_event` → `ae_log_append`, acquiring the
  events fd 8 with `flock -w 5` (`ae:16532-16536`, `ae:13171-13176`). Order in the watchdog
  process is meta fd 200 in the child, then events fd 8 in the parent; no lock spans both.

### Write sequence

- Matching agent-session files are read; only a `pending` `agent.<slot>` row is rewritten with
  `_ae_sed_inplace` in `meta` under fd 200 (`ae:8690-8743`).
- Standalone doctor reporting prints `OK/already/miss/skip` rows and writes no event
  (`ae:8801-8837`).
- Watchdog success rows cause a `recover` event append via `ae_emit_event`/`ae_log_append`
  (`ae:16532-16536`, `ae:13171-13176`). This is not `_spawn_emit_event`.

### Crash residue

- A successful meta rewrite leaves the captured ID in `meta`; an interruption during
  `_ae_sed_inplace` can leave `${meta}.sedtmp.$$` while the prior metadata remains, and the
  meta lock path remains (`ae:152-171`, `ae:8721-8732`).
- Standalone interruption has no event-file residue from the reporter.
- Watchdog interruption after meta capture and before event append leaves updated meta without a
  `recover` event; interruption during append can leave a partial event line and its lock path
  (`ae:16532-16536`, `ae:13171-13176`).

## Request WITHDRAW (`_compact_cancel_outstanding`)

### Locks and acquisition order

- Compact calls `_compact_cancel_outstanding` while its handover lifecycle fd 8 is held
  (`ae:6272-6313`, `ae:6290-6295`).
- `_compact_cancel_outstanding` delegates to the session `send` helper with target
  `ae:compact:<uuid>` and `_AE_EVENT_ACTION=cancel` (`ae:5961-5969`). The external-target branch
  skips the target lock and acquires only the events lock on fd 8 with `flock -w 5` through
  `ae_log_append` (`ae:14180-14194`, `ae:13171-13176`). The compact lifecycle lock remains held
  by the caller while this event lock is acquired in the child.

### Write sequence

- No tmux paste and no body artifact occurs for the `ae:compact:` target. `send` calls
  `ae_emit_event` directly with action/ref/summary from the environment (`ae:5963-5965`,
  `ae:14180-14194`). The JSON is appended directly to `events.jsonl` by `ae_log_append`
  (`ae:13208-13256`, `ae:13171-13176`).

### Crash residue

- The compact lifecycle lock path remains on interruption. During event append,
  `events.jsonl.lock` remains and the direct append can leave a partial cancellation event.
- If interruption occurs before the delegated send, the outstanding request event remains and no
  cancellation event is added (`ae:6290-6295`, `ae:5961-5969`).

## Watchdog controls and daemon loop

### Locks and acquisition order

- `cmd_watchdog` resolves the session/helper and invokes `watchdog start|stop|status` with no
  outer lock at `ae:9145-9182`.
- `watchdog start`, `stop`, and `status` use no lifecycle or events lock. `start`/`stop` take
  metadata fd 200 with `flock -w 5` only in `_set_meta_kv` when writing `watchdog=true/false`
  (`ae:14934-14948`, `ae:15031-15105`). Their pidfile, tmux pane, and tmux option operations
  precede that metadata lock. `status` may remove a stale pidfile without a lock
  (`ae:14910-14931`, `ae:15064-15070`).
- The watchdog daemon itself takes no long-lived flock. Each `ae_emit_event` call acquires
  `${META_DIR}/events.jsonl.lock` fd 8 with `flock -w 5` through `ae_log_append`; calls occur
  without another daemon lock held (`ae:16019-16659`, `ae:13171-13176`).
- Stale-agent and steward sweep nudges invoke the generated `send` helper at
  `ae:16195-16215` and `ae:16467-16480`. Each nudge acquires the target lock on fd 9 with
  blocking `flock 9` and no timeout (`ae:12993-13000`), releases it after tmux submission,
  then stores the body and takes the events fd 8 `flock -w 5` through `ae_log_append`
  (`ae:14235-14283`). The target lock is not held across the body/event writes.
- The daemon invokes `_recover-pending` as a child (meta fd 200) and Telegram `_supervise`,
  whose control lock is separate (`ae:16528-16557`, `ae:10486-10524`). No watchdog lifecycle
  lock spans those calls.

### Write sequence

- `start` creates/ensures the monitor window and watchdog pane, writes `.watchdog.pid` in place
  from `_run`, sets tmux status options, and records `watchdog=true` through meta temp+mv
  (`ae:14969-14995`, `ae:15085-15103`, `ae:15981-15986`, `ae:14934-14948`).
- `start` and `stop` first reap legacy shepherd/loop pid/status files and panes with direct
  `rm`/tmux operations and no lock (`ae:15006-15029`); status can also remove a stale current
  pidfile without a lock (`ae:14910-14931`).
- `stop` kills the watchdog process, removes `.watchdog.pid`, clears tmux bar options, kills
  only the watchdog pane, and records `watchdog=false` through meta temp+mv
  (`ae:15031-15061`, `ae:14934-14948`).
- `status` reads/removes pidfile as described above and writes no event (`ae:15064-15070`).
- Each daemon cycle scans panes/meta, may write tmux status options, emits alerts/nudges/recovery
  events using `ae_emit_event`, and sleeps; the loop and cleanup are at `ae:15981-16659`.
  Event writes are `ae_log_append`, not `_spawn_emit_event` or inline `-w 15`.
- A delivered nudge also has the generated-send sequence: tmux paste/Enter while target fd 9 is
  held, then `messages/*` body storage and an `ae_log_append` nudge event after fd 9 release
  (`ae:16195-16215`, `ae:16467-16480`, `ae:14235-14283`).
- On exit the watchdog removes `.watchdog.pid` and clears bar options through its EXIT cleanup
  (`ae:16658`, `ae:15642-15670`).

### Crash residue

- During start, monitor/watchdog panes or `.watchdog.pid` can exist before metadata records
  `watchdog=true`; during stop, pidfile/options/pane can be removed before metadata records
  `watchdog=false` (`ae:15031-15105`).
- `status` can remove a stale pidfile while leaving the tmux/session state unchanged
  (`ae:14910-14931`).
- During watchdog runtime, `.watchdog.pid` remains until EXIT cleanup; an interrupted cycle can
  leave it, tmux status options, and any already-appended/partial event line
  (`ae:15981-16659`, `ae:13171-13176`).
- An interrupted nudge can additionally leave a target `.locks/send-lock-*`, completed/partial
  tmux input, a body artifact without its event, or a partial nudge event (`ae:12993-13000`,
  `ae:13322-13359`, `ae:14235-14283`).

## Telegram setup/start/stop and daemon loop

### Locks and acquisition order

- `telegram setup` acquires no lock. It creates the token/config directories and writes directly
  (`ae:10550-10607`).
- `telegram start` and `telegram stop` obtain the machine-global control lock
  `${CONFIG_DIR}/telegram/control.lock` on fd 9 with `flock -w 5`, at `ae:10628-10655` and
  `ae:10658-10678`. Start holds it across intent persistence, daemon artifact publication,
  tmux spawn, and post-check. Stop holds it across intent persistence and tmux kill. No other
  ae lock is acquired by these callers.
- `_telegram_autostart_if_enabled` (used by launch/reattach/watchdog supervise) uses the same
  control lock on fd 9 with non-blocking `flock -n` at `ae:10486-10524`; it holds the lock
  across intent/running rechecks and daemon spawn.
- The generated Telegram daemon acquires its own `${CONFIG_DIR}/telegram/daemon.lock` on fd 9
  with non-blocking `flock -n` at `ae:10177-10185`, held for the process lifetime after
  `main` sets `LOCK_HELD=1` (`ae:10225-10232`). It is not held by the outer start/stop control
  lock after tmux spawn.

### Write sequence

- Setup writes the token file with `umask 077`, direct `printf >`, then chmod 600, and appends
  the `[telegram]` block directly with `cat >>` (`ae:10575-10602`). Existing sections are left
  unchanged.
- Start/stop persist `enabled` through `_telegram_persist_intent`: missing/no-section config
  uses direct `printf >`/`>>`; an existing section uses awk to `${cfg}.tmp.$$` then mv
  (`ae:10280-10310`). Start then publishes `telegram-daemon` via `_publish_executable_artifact`
  temp+chmod+mv and creates/renames the `ae-telegram` tmux session (`ae:9305-9308`,
  `ae:10315-10346`). Stop kills the tmux session (`ae:10671-10677`).
- The daemon rotates `daemon.log` with mv, creates/appends it directly, then redirects all output
  to it (`ae:9355-9367`). It loads config/state, runs `tg_set_commands` when enabled, and loops
  discovery, event forwarding, state save, update polling, and sleep (`ae:10225-10256`).
- `tg_set_commands` writes a temporary command-menu payload directly, sends it, and removes it;
  it takes no additional lock beyond the daemon-wide lock (`ae:9557-9598`, `ae:10177-10185`).
- `save_state` writes `state.tsv.tmp.$$` then mv without a separate flock; the daemon-wide
  `daemon.lock` is held by the process (`ae:10086-10099`, `ae:10177-10185`). `tg_save_offset`
  similarly writes `tg_offset.tmp.$$` then mv (`ae:9611-9620`).
- Inbound polling persists each update offset before dispatch (`ae:9990-10045`). `/use` writes
  the target file directly with `printf >` and `rm -f` clears it (`ae:9884-9935`). Outbound
  Telegram payloads use `/tmp` mktemp, direct printf, curl, and rm (`ae:9521-9551`); these are
  daemon-local temporary files, not ae session lock files.
- The daemon emits no ae event for its own state writes. Agent-facing Telegram sends route via
  generated helpers and therefore use their normal target/body/`ae_log_append` event path.

### Crash residue

- Setup can leave a partial token file or partial config block; the token path and config have
  no setup lock (`ae:10575-10602`).
- Start/stop can leave `control.lock`, a persisted enabled value before tmux spawn/kill, a
  published daemon artifact, or an `ae-telegram` tmux session at the corresponding stage
  (`ae:10628-10678`, `ae:10280-10310`, `ae:10315-10346`).
- During daemon startup/rotation, `daemon.log.1` or a direct log file can remain; during
  state/offset writes, `.tmp.$$` files can remain while the prior canonical file remains
  (`ae:9355-9367`, `ae:10086-10099`, `ae:9615-9619`).
- A daemon interruption after an outbound Telegram send and before the next `save_state` can
  leave the sent message with the old offset/state file; the write order is visible at
  `ae:10249-10254`, `ae:10086-10099`.
- An inbound interruption after offset mv and before dispatch leaves the persisted offset with
  no dispatch for that update (`ae:10017-10022`, `ae:9615-9619`).

## Steward (`cmd_steward*`, autostart, and runtime)

### Locks and acquisition order

- `ae steward --init` takes no lock. It creates the scaffold directory and writes each template
  directly (`ae:12746-12786`, `ae:12705-12741`).
- `ae steward`/`hub` trampoline sets the standalone config and falls through to generic launch
  (`ae:16722-16820`). It therefore takes the generic launch lifecycle fd 8 with `flock -w 15`
  after workdir setup, and keeps it through tmux/meta/helper/manifest/send operations
  (`ae:17282-18179`).
- `cmd_steward_help` prints a here-document and takes no lock or write path (`ae:12788-12802`).
  `--attach` only changes the generic launch attach flag before that same launch/reattach
  path (`ae:16744-16769`).
- `_steward_autostart_if_scaffolded` takes no lock; its background child enters generic launch
  and, when it creates a new steward session, acquires the lifecycle lock there
  (`ae:10357-10400`, `ae:17052-17300`).
- Steward runtime uses the generic watchdog path. Its events acquire the events fd 8
  `flock -w 5` through `ae_log_append`; no separate steward lock is acquired
  (`ae:15981-16659`, `ae:13171-13176`).

### Write sequence

- Init creates `steward_dir`, writes `CHARTER.md` directly with `printf >`, then writes
  `steward.config` directly with `printf >`; files are not temp+mv (`ae:12705-12741`,
  `ae:12767-12783`).
- Help writes only stdout from its here-document; `--attach` adds no file write before the
  generic launch path (`ae:12788-12802`, `ae:16744-16769`).
- Generic launch writes `meta_agent=true` into `meta.tmp.$$` and publishes canonical `meta`
  by mv (`ae:17582-17586`, `ae:17628`), then emits helpers/manifest and launches panes as in
  the launch section.
- Autostart backgrounds `ae steward` with stdout/stderr redirected to `/dev/null` at
  `ae:10393-10399`; the child performs the generic launch writes.
- The steward watchdog reads aemonitor/heartbeat data and emits runtime alert/nudge/recovery
  events through `ae_emit_event` → `ae_log_append`; ae itself does not write aemonitor's state
  files (`ae:16110-16536`, `ae:13171-13176`).

### Crash residue

- Init can leave a partial `CHARTER.md` or `steward.config`; because each destination is
  skipped if it exists, a later init can preserve the partial first file and create the second
  (`ae:12708-12740`).
- Steward generic launch has the launch residues listed above: tmux panes, `meta.tmp.$$`,
  partial direct manifest, helper temps, and lifecycle lock path (`ae:17282-18179`).
- Autostart can leave no visible child diagnostic because its child output is redirected to
  `/dev/null`; any child-created tmux/meta/helper residue is at the generic launch points
  (`ae:10393-10399`, `ae:16861-16875`).
- Runtime interruption leaves watchdog pid/status artifacts and any completed/partial event
  append at the watchdog points (`ae:15981-16659`, `ae:13171-13176`).

## Pre-dispatch config bootstrap

### Locks and acquisition order

- Before function definitions and dispatcher execution, the only write in the inspected
  bootstrap region is the missing-config branch at `ae:344-352`. It acquires no lock.

### Write sequence

- If `$CONFIG_FILE` is absent, it runs `mkdir -p "$CONFIG_DIR"` and writes the full
  `$DEFAULT_CONFIG` with direct `printf >"$CONFIG_FILE"` at `ae:344-347`, then writes only a
  stderr notice at `ae:351`.
- No event writer (`ae_log_append`, `_spawn_emit_event`, or inline event append) is used. No
  other pre-dispatch command is executed as a file write in the surrounding bootstrap. The
  `_ae_sed_inplace` body at `ae:152-171` contains write commands but is only a function
  definition at this point; the following `parse_config` definition starts at `ae:354`.

### Crash residue

- Interruption after directory creation and before `printf` can leave `$CONFIG_DIR` without
  `$CONFIG_FILE` (`ae:344-347`).
- Interruption during direct `printf >"$CONFIG_FILE"` can leave a partial default config. The
  next invocation enters the branch only if the file is absent, so a partial existing file is
  not replaced by this bootstrap (`ae:345-347`).
