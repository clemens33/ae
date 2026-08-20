# Lock/atomicity census for `ae`

- Source commit: `72c729343a0117af2968b66e1c43f89ad25fc0b2`
- Source commit date: 2026-08-20T08:49:06+02:00
- Census date: 2026-08-20
- Agent: `gpt56luna:evidence`

Line references below are to `ae` at the source commit. “Crash residue” describes files and
directories present when execution stops at the indicated point; an open `flock` descriptor is
closed by process exit, while the lock-file path created by redirection remains on disk.

## Helper `send` (`helper_send_main` + `ae_tracked_send`)

### Locks and acquisition order

- `ae_tracked_send` does not acquire a lock. It resolves the target and `exec`s the session
  `send` helper with event environment variables at `ae:13444-13488`.
- A normal pane target in `helper_send_main` acquires the per-target lock first: it calls
  `ae_lock_target` at `ae:14235`; that function creates `${_AE_SESSIONS_DIR}/.locks`, opens
  `send-lock-${pane}` on fd 9, and blocks on `flock 9` at `ae:12993-13000`.
- The target lock is released at `ae:14275`. The body-file write then runs without a lock,
  followed by the event append lock.
- The event append acquires fd 8 on `${_AE_META_DIR}/events.jsonl.lock` at
  `ae:13171-13176`; `helper_send_main` reaches it at `ae:14282-14283`.
- An external target (`telegram:*`, `discord:*`, or `ae:compact:*`) skips the target lock and
  calls `ae_emit_event` directly at `ae:14180-14194`; its only lock is the event append lock.

### Write sequence

- For a normal target, pane delivery occurs while fd 9 is held (`ae:14235-14275`). No file is
  written by the tmux paste path.
- `ae_store_message_body` creates `${_AE_META_DIR}/messages` with `mkdir -p` at `ae:13322-13325`.
  It creates a unique temporary file with `mktemp` at `ae:13342-13345`, renames that empty file
  to `${tmp}.txt` at `ae:13347-13351` (or keeps the temporary name if that rename fails), writes
  the body in place with `printf >` at `ae:13352`, and then runs `chmod 600` at `ae:13357`.
- `ae_emit_event` builds JSON in memory (`ae:13208-13255`) and `ae_log_append` appends one line
  directly to `${_AE_META_DIR}/events.jsonl` under its lock at `ae:13171-13176`.
- For an external target, only `events.jsonl` is appended; no message body file is created by
  this path (`ae:14192-14194`).

### Crash residue

- Before `ae_lock_target` completes, the `.locks` directory and the target lock path may have
  been created; no body file or event append has run (`ae:12996-13000`, `ae:14235-14275`).
- During body storage, `messages/` may exist with an empty or partially written `${tmp}.txt`;
  if the rename has not completed, `${tmp}` may remain instead (`ae:13342-13353`). The event
  append has not started yet.
- After body storage and before `ae_emit_event`, the completed body file can remain without a
  corresponding event (`ae:14282-14283`).
- During event append, `events.jsonl.lock` remains and `events.jsonl` can end with a partial JSON
  line because the write is a direct `printf >>` (`ae:13171-13176`).
- For external targets, the same event-lock/partial-line residue applies, without a body file
  (`ae:14192-14194`).

## `ask`, `review`, and `reply` (`helper_*_main` + `ae_find_request`)

### Locks and acquisition order

- `helper_ask_main` calls `ae_tracked_send` at `ae:14287-14296`; `helper_review_main` calls it
  at `ae:14298-14312`. Their direct paths acquire no lock.
- `ae_find_request` scans `${_AE_META_DIR}/events.jsonl` newest-first using `_ae_tac` at
  `ae:13490-13538`; it takes no lock and writes no file.
- `helper_reply_main` calls `ae_find_request` at `ae:14343`, performs slot/name checks, and
  `exec`s the session `send` helper at `ae:14400-14403`. It takes no lock before that `exec`.
- The subsequent normal-pane `send` path has the same order as the `send` section: target fd 9
  (`ae:12993-13000`, called at `ae:14235`), release at `ae:14275`, then event fd 8
  (`ae:13171-13176`, called at `ae:14282-14283`). An event-only target takes only fd 8.

### Write sequence

- `ask`/`review` build the request id and message in memory (`ae:13344-13488`) and then use the
  `send` sequence above. The message body is stored as `messages/*` before the event append
  (`ae:13322-13359`, `ae:14270-14283`).
- `reply` does not update a request table. After `ae_find_request`, it sends a message with
  `_AE_EVENT_ACTION=reply` and `_AE_EVENT_REF=<request-id>` (`ae:14400-14403`); the `send` helper
  writes a body artifact and appends the reply event as above.

### Crash residue

- A crash during `ae_find_request` leaves no write from that function (`ae:13493-13538`).
- Once `send` begins, body-file and event residues are the same as in the `send` section: a
  possible `messages/*` temp/empty/partial file before the event, then a possible partial
  `events.jsonl` line under a persistent `.lock` path (`ae:13342-13357`, `ae:13171-13176`).
- A crash before `reply` reaches its `exec` leaves no reply body or reply event (`ae:14343-14403`).

## `state` (`helper_state_main`)

### Locks and acquisition order

- Query mode (`state` with no arguments) calls `ae_latest_state_for` at `ae:12836-12845`; the
  reader scans `events.jsonl` directly at `ae:13263-13295` and takes no lock.
- Set mode calls `ae_emit_event` once at `ae:12863-12864`; `done` calls it a second time at
  `ae:12870-12871`. Each call independently acquires fd 8 on
  `${_AE_META_DIR}/events.jsonl.lock` through `ae_log_append` (`ae:13171-13176`). Acquisition
  order for `done` is the `state` event lock, release, then the legacy `done` event lock.

### Write sequence

- No state file is written. Each event is appended in place to `events.jsonl`; there is no temp
  file or rename (`ae:13171-13176`, `ae:13208-13257`).

### Crash residue

- Query mode has no write residue (`ae:12836-12845`, `ae:13263-13295`).
- During either event append, `events.jsonl.lock` remains and the appended line may be partial
  (`ae:13171-13176`).
- For `state done`, a completed first `state` event may be present while the second `done` event
  has not started; a crash during the first append can instead leave only its partial line
  (`ae:12863-12871`).

## `goal` (`helper_goal_main` + `ae_meta_set`)

### Locks and acquisition order

- Read mode calls `ae_meta_get` at `ae:14558-14567`; `ae_meta_get` is a direct `grep` pipeline
  and takes no lock (`ae:14123-14125`).
- `--clear` calls `ae_meta_unset` at `ae:14568-14572`. It acquires fd 200 on
  `${_AE_META_DIR}/meta.lock`, then rewrites `meta`, and only after releasing that lock calls
  the event append (`ae:14145-14153`, `ae:14570-14571`, `ae:13171-13176`).
- Set mode calls `ae_meta_set` at `ae:14579-14589`. It acquires fd 200 on `meta.lock`, then
  releases it before acquiring fd 8 on `events.jsonl.lock` (`ae:14127-14142`, `ae:14585-14589`,
  `ae:13171-13176`).

### Write sequence

- `ae_meta_set` writes `${meta}.tmp.$$` with `awk` and renames it over `meta` with `mv` while
  fd 200 is held (`ae:14127-14142`). `ae_meta_unset` uses the same temp+`mv` sequence with a
  filtering `awk` program (`ae:14145-14153`).
- The subsequent goal event is appended directly to `events.jsonl` (`ae:14570-14571` or
  `ae:14585-14589`, `ae:13171-13176`).
- If `meta` does not exist, `ae_meta_set` returns before taking its lock (`ae:14129-14130`),
  while `ae_meta_unset` returns success before taking its lock (`ae:14147-14148`); the caller's
  event call remains at `ae:14570-14571` or is skipped by the set error arm at `ae:14585-14589`.

### Crash residue

- During the meta rewrite, `${meta}.tmp.$$` can be absent or partially written; until `mv`
  completes the prior `meta` remains at its original path (`ae:14135-14142`, `ae:14151-14153`).
  `meta.lock` remains as a path after the process exits.
- Between the meta `mv` and the goal event, the new/updated `meta` can remain without the goal
  event (`ae:14585-14589` or `ae:14570-14571`).
- During the event append, `events.jsonl` can contain a partial line and its `.lock` path remains
  (`ae:13171-13176`).
- Read mode has no write residue (`ae:14561-14567`).

## `memo add` (`helper_memo_main`)

### Locks and acquisition order

- The add branch calls `ae_log_append` for `memo.tsv` at `ae:14503-14523`. It acquires fd 8 on
  `${MEMO_FILE}.lock` (`${_AE_META_DIR}/memo.tsv.lock`) and appends under that lock at
  `ae:13171-13176`.
- After that lock scope ends, it calls `ae_emit_event` at `ae:14522-14523`, which acquires fd 8
  on `${_AE_META_DIR}/events.jsonl.lock` (`ae:13171-13176`). Order is memo lock, then event lock.

### Write sequence

- The memo record is appended directly to `memo.tsv` with a trailing newline; no temp file or
  rename is used (`ae:13171-13176`, called at `ae:14522`).
- The event is appended directly to `events.jsonl` under its separate lock (`ae:14523`,
  `ae:13208-13257`, `ae:13171-13176`).

### Crash residue

- During the memo append, `memo.tsv.lock` remains and the final memo line can be partial; the
  event append has not started (`ae:13171-13176`).
- After the memo append and before the event append, a complete memo record can remain without
  its event (`ae:14522-14523`).
- During the event append, `events.jsonl.lock` remains and its line can be partial
  (`ae:13171-13176`).

## `say` (`helper_say_main`)

### Locks and acquisition order

- `helper_say_main` calls only `ae_emit_event` for its persistent operation at `ae:14470-14485`.
  The only lock is fd 8 on `${_AE_META_DIR}/events.jsonl.lock`, acquired by `ae_log_append` at
  `ae:13171-13176`.

### Write sequence

- The chat event is appended directly to `events.jsonl`; no temp file or rename is used
  (`ae:14484`, `ae:13208-13257`, `ae:13171-13176`).

### Crash residue

- A crash during append leaves the event lock path and can leave a partial JSON line in
  `events.jsonl` (`ae:13171-13176`). No other file is written by `helper_say_main`.

## Event append (`ae_emit_event`)

### Locks and acquisition order

- `ae_emit_event` itself takes no lock before JSON construction (`ae:13208-13255`). It calls
  `ae_log_append` at `ae:13256`; that function opens `${file}.lock` on fd 8 and acquires
  `flock -w 5 8` before the append (`ae:13171-13176`). For this function, the only lock is
  `${_AE_META_DIR}/events.jsonl.lock`.

### Write sequence

- JSON is assembled in shell variables (`ae:13208-13255`), then one line is written in place
  with `printf '%s\n' >>"$file"` (`ae:13171-13176`). There is no temp file and no `mv`.

### Crash residue

- The lock path remains after exit. A stop during the direct append can leave an incomplete final
  JSON line in `events.jsonl` (`ae:13171-13176`).

## `spawn` (`_cmd_spawn`)

### Locks and acquisition order

- `_cmd_spawn` reads `meta` without a lock, creates the tmux window, then acquires fd 200 on
  `${meta_dir}/meta.lock` at `ae:11923-11945`. This is the first direct lock.
- If the selected tool supports launch IDs, `start_capture_session_id` subsequently acquires
  the same `meta.lock` on fd 200 and appends `launch_time.<slot>` at `ae:2068-2075`. It then
  starts an asynchronous capture process (`ae:2075`).
- A successful or delivery-failed spawn later calls `_spawn_emit_event` at `ae:12071-12072` or
  `ae:12084-12085`; that function acquires fd 8 on `${meta_dir}/events.jsonl.lock` at
  `ae:12089-12114`.
- On `send_agent_cmd` failure, `_spawn_rollback` acquires `meta.lock` on fd 200 at
  `ae:12554-12578`; it does not acquire an event lock. The caller returns at `ae:11974-11977`
  without calling `_spawn_emit_event`.
- The asynchronous capture process can later acquire `meta.lock` on fd 200 while updating the
  roster: Codex at `ae:1860-1864`, OpenCode at `ae:2021-2025`, or Gemini at `ae:2050-2054`.
  Those acquisitions occur in a child after `_cmd_spawn` has returned and can interleave with
  later operations.

### Write sequence

- `tmux new-window` creates the pane/window before any spawn roster write (`ae:11916-11921`).
- The initial meta registration appends `agent.<slot>`, `agent_bin.<slot>`, and, for supported
  tools, `launch_id.<slot>` directly to `meta` under fd 200; no temp file or rename is used
  (`ae:11927-11945`).
- `regenerate_manifest` writes `${meta_dir}/workspace.md` directly with `cat >` at
  `ae:11954-11955` and `ae:12339-12353`; it has no lock and no temp+`mv`.
- `send_agent_cmd` writes `launch.<slot>.sh` through `write_launch_script` at `ae:12602-12605`.
  `_publish_executable_artifact` generates `${dest}.tmp.$$`, applies `chmod`, and renames it to
  the destination at `ae:833-850`; `write_launch_script` then removes the prior
  `launch.<slot>.started` marker at `ae:900-906`. The generated script can later create that
  marker in place with `: >` at `ae:864-872`.
- `start_capture_session_id` appends `launch_time.<slot>` directly to `meta` at `ae:2071-2074`.
  Its asynchronous capture first reads and removes `codex.<slot>.sid` at `ae:1835-1844` when
  present, then updates the `agent.<slot>` line under `meta.lock`. `_ae_sed_inplace` copies to
  `${meta}.sedtmp.$$`, writes the transformed copy, and renames it over `meta` at
  `ae:156-171`; the Codex/OpenCode/Gemini callers are `ae:1860-1864`, `ae:2021-2025`, and
  `ae:2050-2054`.
- On brief-delivery failure, `_cmd_spawn` writes `undelivered.<alias>-<name>.txt` directly with
  `printf >` and applies `chmod 600` at `ae:12052-12059`, then appends a `spawn-failed` event
  (`ae:12068-12072`, `ae:12089-12114`). On success it appends a `spawn` event at
  `ae:12076-12085`, using the same direct event append.
- On launch failure, `_spawn_rollback` rewrites `meta` through
  `${meta_dir}/meta.rollback.$$` plus `mv` under `meta.lock` at `ae:12558-12578`, removes the
  launch script and marker at `ae:12579`, kills the pane at `ae:12580`, and rewrites
  `workspace.md` directly at `ae:12581`.

### Crash residue

- After `tmux new-window` and before initial meta registration, the pane/window exists while no
  `agent.<slot>` registration has been appended (`ae:11916-11921`, `ae:11923-11945`).
- During initial or `launch_time` meta append, `meta.lock` remains and the last direct append can
  be partial (`ae:11927-11945`, `ae:2071-2074`).
- During `workspace.md` regeneration, the file can be truncated or partially written because
  the write is direct `cat >` (`ae:12339-12353`).
- During launch-script publication, `launch.<slot>.sh.tmp.$$` can remain partially generated;
  until `mv` succeeds, the previous destination remains at its path (`ae:833-850`). The
  `launch.<slot>.started` marker can remain absent or can be created as an empty file by the
  generated script (`ae:864-872`, `ae:900-906`).
- During asynchronous roster capture, `codex.<slot>.sid` can be present, absent after the
  `rm`, or its `meta.sedtmp.$$` can be partial; the previous `meta` remains until the rename
  (`ae:1839-1843`, `ae:156-171`).
- During undelivered-brief persistence, `undelivered.*.txt` can be partial and the failure event
  can be absent or partial (`ae:12057-12072`, `ae:12089-12114`).
- During rollback, `meta.rollback.$$` can remain partial, launch artifacts can remain if the
  `rm` has not run, the pane can remain if `kill-pane` has not run, and `workspace.md` can be
  partial if regeneration has started (`ae:12558-12581`).

## `retire` (`helper_retire_main` + `_cmd_retire`)

### Locks and acquisition order

- `helper_retire_main` only loads metadata and `exec`s `_retire` at `ae:14725-14746`; it takes
  no lock itself.
- `_cmd_retire` kills the target pane before acquiring any file lock (`ae:12213-12214`). It
  then acquires fd 200 on `${meta_dir}/meta.lock` for the meta rewrite at `ae:12216-12229`.
- After the meta lock scope ends, it regenerates the manifest and then appends the retire event;
  the event append acquires fd 8 on `${meta_dir}/events.jsonl.lock` at `ae:12245-12262`.
  Lock order is meta lock, then event lock.

### Write sequence

- The meta rewrite writes `${meta_dir}/meta.tmp.$$` using `grep -v`, then renames it over
  `meta` under fd 200 (`ae:12218-12229`).
- It removes `launch.<slot>.sh` and `launch.<slot>.started` directly with `rm -f` at
  `ae:12230-12232`.
- `regenerate_manifest` rewrites `workspace.md` directly with `cat >` (`ae:12236-12242`,
  `ae:12339-12353`).
- The retire event is serialized to JSON in memory and appended directly to `events.jsonl`
  under fd 8 (`ae:12245-12262`).

### Crash residue

- A crash after `kill-pane` and before meta rewrite leaves the pane gone while its meta entry
  remains (`ae:12213-12229`).
- During meta rewrite, `meta.tmp.$$` can be partial and the prior `meta` remains until `mv`
  completes; `meta.lock` remains as a path (`ae:12218-12229`).
- A crash during launch-artifact removal can leave one or both launch files (`ae:12230-12232`).
- During manifest regeneration, `workspace.md` can be truncated or partial (`ae:12339-12353`).
- During event append, `events.jsonl` can contain a partial JSON line and its `.lock` path remains
  (`ae:12245-12262`, `ae:13171-13176`).

## `_register-sid` (`helper_register_sid_main`)

### Locks and acquisition order

- `helper_register_sid_main` reads `${META_DIR}/meta` and scans Codex session files; all reads
  are direct at `ae:14755-14817`.
- It takes no flock or mkdir lock. The only operation write is the sid-file redirect at
  `ae:14818-14820`.

### Write sequence

- When a matching session is found, `echo "$best_id" >"$META_DIR/codex.${SLOT}.sid"` truncates or
  creates that file in place (`ae:14818-14820`). There is no temp file and no rename.
- When no matching session is found, the function exits through `ae:14821-14824` without a
  write.

### Crash residue

- During the redirect, `codex.${SLOT}.sid` can be absent, empty, or partially written; after a
  completed redirect it contains the echoed ID and newline (`ae:14818-14820`).
- No lock-file path is created by this helper. The asynchronous capture reader may later read and
  remove the sid file at `ae:1839-1843`.
