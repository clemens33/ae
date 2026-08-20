# Lock/atomicity census — `contrib/aewatch/aewatch` (census #3)

- Source commit: `72c729343a0117af2968b66e1c43f89ad25fc0b2`
- Source commit date: `2026-08-20T08:49:06+02:00`
- Census date: `2026-08-20`
- Agent: `gpt56luna:census3`
- Scope: the `aewatch` sidecar at the source commit; event append/drop, runtime singleton and runtime files, daemon loop, Telegram stores, bridge ownership handoff, metadata/event/tmux contracts shared with Bash `ae`, and the stale phase-1 header.
- Working-tree check: `git diff --quiet 72c7293 -- contrib/aewatch` returned exit `0`.

Line references below are to `contrib/aewatch/aewatch` at the source commit unless prefixed
`ae:` (the Bash script at the same commit). “Crash residue” describes paths and state left
when execution stops at the indicated point. A `flock` descriptor is released by process
exit; the lock-file path created by opening it remains on disk.

## Header versus wired runtime

The module docstring still says this is a “runnable, stdlib-only skeleton,” performs “NO
runtime side effects,” and has only a read-only `daemon --once` tick whose writes land under
`$AE_HOME/aewatch/` (`aewatch:6-18`). The source below that header contains the production
runtime: `daemon --once` calls `run_daemon_tick` (`aewatch:3423-3435`), which acquires the
singleton and writes heartbeat/backoff state (`aewatch:3085-3115`), while `daemon --loop`
calls the long-running supervisor and co-located bridge (`aewatch:3354-3420`). The parser
description also still says “phase 1” (`aewatch:3438-3442`).

## Shared event log (`events.jsonl`)

### Lock and acquisition

- `_locked_append` targets `<events.jsonl>.lock` (`aewatch:2357-2368`). It opens the lock
  path in append mode, then repeatedly attempts `fcntl.flock(fd, LOCK_EX|LOCK_NB)` every
  `0.02` seconds until a monotonic deadline (`timeout`, default `5`) (`aewatch:2369-2377`).
  This is non-blocking `fcntl` plus polling, not a blocking flock call.
- After acquiring the lock, it opens the target event file with `open(path, "a")`, writes
  one line plus `\n`, and unlocks in `finally` (`aewatch:2378-2383`). The lock is held only
  for that open/write/close scope.
- `make_emit_event` applies this protocol to
  `<sessions_dir>/<session>/events.jsonl`; a timeout logs a warning and drops the event,
  while the recorder effect is recorded only after the append succeeds (`aewatch:2386-2406`).
- Bash uses the same `<file>.lock` path and a blocking `flock -w 5` before its direct append
  (`ae:13171-13177`). Bash also has a shared-lock reader (`ae:13179-13185`), while the
  aewatch event readers below do not acquire the lock.

### Reads and writes

- Watchdog activity reads the session event file directly with `read_text().splitlines()`:
  `_last_event_age` (`aewatch:2029-2060`), `_latest_relevant_event`
  (`aewatch:2063-2098`), and `_agent_alert_reason` (`aewatch:2267-2314`). These reads have
  no `fcntl` lock and can observe the file while Bash or aewatch appends, or while Bash
  replaces it during resume retention (`ae:18046-18075`).
- `process_session_events` stats the file, opens it in binary mode, seeks to the saved byte
  offset, and reads to the stat'ed size (`aewatch:1463-1508`). It advances only complete
  newline-terminated records and returns the observed inode/offset; it has no event lock.
- Aewatch watchdog events are emitted through the injected `emit_event` boundary from the
  watchdog cycle (`aewatch:2499-2524`, `aewatch:2688-2735`), which is bound to
  `make_emit_event` in the production CLI (`aewatch:3367-3374`).

### Write sequence and residue

- Event writes are in-place appends. There is no temp file, rename, `fsync`, or length check
  in `_locked_append` (`aewatch:2378-2383`). A stop during the write can leave an incomplete
  final JSON line; the `.lock` path remains after descriptor close.
- A lock timeout leaves no event line and no recorder `event.append` effect, but the warning
  is sent through the logger (`aewatch:2394-2404`).
- The lock does not cover preceding event construction or subsequent effects. For the
  watchdog path, a tmux mutation can occur before the event append; the append can then
  time out or fail independently (`aewatch:2688-2735`).

## Runtime singleton, heartbeat, log, and backoff

### Singleton lock

- `SingletonLock.acquire` creates `$AE_HOME/aewatch` as needed, opens
  `aewatch.lock` with `O_CREAT|O_RDWR` and mode `0600`, and tries
  `fcntl.flock(fd, LOCK_EX|LOCK_NB)` (`aewatch:2798-2814`). There is no timeout or retry.
- `EAGAIN`, `EWOULDBLOCK`, and `EACCES` close the fd and return `False`; other flock errors
  are raised (`aewatch:2815-2824`). A successful fd remains held until `release`, which
  unlocks and closes it (`aewatch:2826-2833`).
- `AewatchRuntime` resolves `aewatch.lock`, `heartbeat`, `daemon.log`, `backoff.json`, and
  `bridge-owner` below `$AE_HOME/aewatch`, creates the directory, and attempts `chmod 0700`
  (`aewatch:2835-2855`). Directory creation/chmod has no separate lock.
- `run_daemon_tick` acquires and releases one singleton per tick
  (`aewatch:3085-3115`). `run_daemon_loop` acquires once before installing the loop and holds
  the fd through all component ticks, sleep, shutdown logging, and final release
  (`aewatch:3150-3232`).

The lock path is a persistent inode, but ownership is the open flock, not path existence.
After a process exits, the descriptor is gone and a later process can acquire the same path.
There is no stale-lock deletion or timeout recovery in `SingletonLock` (`aewatch:2798-2833`).

### Heartbeat and bridge-owner files

- `write_bridge_owner` writes `<aewatch>/.bridge-owner.<pid>.tmp` in place, then publishes
  `bridge-owner` with `os.replace` (`aewatch:2859-2876`). The body is the writer PID plus a
  wall-clock nanosecond stamp. It returns `False` after an `OSError` and attempts to unlink
  its temp path.
- `clear_bridge_owner` reads the marker in place, compares its first field with the current
  PID, and unlinks only on a match (`aewatch:2878-2891`). It takes no lock; the read,
  comparison, and unlink are separate operations.
- `write_heartbeat` uses `<aewatch>/.heartbeat.<pid>.tmp`, writes the nanosecond stamp, and
  `os.replace`s it over `heartbeat` (`aewatch:2893-2911`). On failure it attempts to unlink
  the temp and re-raises. It takes no file lock beyond any caller-held singleton.
- `_heartbeat_fresh` and `_bridge_ownership_fresh` read heartbeat mtime and marker
  existence directly, without a lock (`aewatch:1626-1631`, `aewatch:3263-3273`).

The atomic replace leaves the previous canonical file until the rename. A crash before the
rename can leave a PID-specific temp; a crash after `write_bridge_owner` can leave
`bridge-owner` in place. A loop's clean `finally` clears the marker
(`aewatch:3412-3420`); an unhandled process termination before that `finally` leaves the
marker for the Bash freshness check. The heartbeat's last canonical mtime remains until a
successful replacement; no directory fsync is performed.

### `daemon.log`

- `AewatchLogger` rotates without a lock: it unlinks the oldest backup, renames backups
  upward, renames the live file to `.1`, then opens the live path in append-binary mode
  (`aewatch:2924-2983`).
- Redaction happens before rotation-size calculation and before the append
  (`aewatch:2952-2977`). The append is direct/in-place; there is no temp+replace for a log
  line. Rotation and append failures are caught and optionally reported to stderr
  (`aewatch:2978-2988`).
- A stop can leave a partially rotated set (some backup renames completed, later ones not),
  a missing live path before the next append, or a partial final log write. This file is
  sidecar-owned; Bash's Telegram daemon uses the distinct `$AE_HOME/telegram/daemon.log`
  (`ae:9327-9333`).

### Backoff state

- `BackoffState._load` reads JSON from its path without a lock and treats missing, unreadable,
  malformed, or wrong-shaped data as an empty crash list (`aewatch:3000-3031`).
- `_save` writes `.backoff.<pid>.tmp` and publishes with `os.replace`; a failed save attempts
  to remove the temp and re-raises (`aewatch:3037-3048`). `record_crash` and
  `record_success` load/prune then replace the whole file (`aewatch:3050-3069`).
- The loop creates one file per component (`backoff-watchdog.json`,
  `backoff-bridge.json`) and saves it after that component's tick or crash handling
  (`aewatch:3196-3221`). The one-shot path uses the shared `backoff.json`
  (`aewatch:3108-3112`). No Bash path reads these files.
- A crash before replacement can leave a temp and the prior canonical state; a crash during
  a replacement leaves the canonical path at either the old or new complete file. No
  backoff file lock is taken.

## Bridge ownership handoff (`marker -> fresh heartbeat -> kill Bash -> send`)

### Lock and ordering

- `_make_bridge_component` has no marker-specific lock. In the production loop it executes
  while `run_daemon_loop` holds the lifetime singleton (`aewatch:3150-3175`).
- On the first enabled tick it calls `write_bridge_owner`, then `write_heartbeat`, checks
  marker plus heartbeat freshness, kills the Bash bridge on every discovered/default tmux
  server, marks the handoff complete, and calls `bridge.tick` (`aewatch:3316-3348`).
- `write_bridge_owner`, `write_heartbeat`, and the freshness checks take no additional
  `fcntl` lock (`aewatch:2859-2911`, `aewatch:3263-3273`). The tmux kill path is a tolerant
  subprocess mutation with no ae file lock (`aewatch:3276-3288`, `aewatch:672-676`).
- A later enabled tick checks freshness before calling `bridge.tick`; if stale, it resets
  the in-memory `handed_off` flag, clears its own marker, and returns without ticking
  (`aewatch:3338-3348`).
- The bridge's first and subsequent sends use `TelegramBridge.tick`, which drains outbound
  `events.jsonl` before polling inbound updates (`aewatch:1571-1595`). The Telegram stores
  are not locked by this wrapper.
- The separate `BridgeSupervisor` boundary, when injected into a watchdog cycle, shells
  `ae telegram _supervise` with a five-second timeout and records only an attempt before
  the shell (`aewatch:2453-2496`). The production co-located CLI passes
  `supervise_bridge=None` (`aewatch:3367-3375`), so its bridge handoff uses the component
  above rather than this alternate Bash-revive boundary.

### Bash side and ownership residue

- Bash tests the same durable marker and heartbeat mtime in `_aewatch_owns_bridge`, with no
  flock (`ae:10469-10484`). Every Bash autostart path returns while that fact is true
  (`ae:10486-10493`). Bash's separate Telegram start/supervise critical section uses the
  non-blocking `telegram/control.lock` (`ae:10508-10524`); it does not lock the marker or
  heartbeat.
- A clean loop exit runs `runtime.clear_bridge_owner` (`aewatch:3412-3420`), so Bash can
  resume immediately. A stop after marker publication but before a fresh heartbeat leaves
  the marker until the owner clears it or the freshness condition fails. A stop after the
  fresh heartbeat but before/inside `kill-session` leaves marker+fresh heartbeat while the
  Bash guard stands down; the tmux kill may or may not have completed because the kill call
  is tolerant (`aewatch:3276-3288`). **FAIL-OPEN direction (audit, probe-verified exit 0):
  a kill failure (rc=1/timeout) is IGNORED — `handed_off` is still set and `bridge.tick`
  still sends, so an existing bash daemon and aewatch BOTH send (probe: forced kill rc=1
  → bridge_ticks=1, marker_fresh=true). The marker suppresses future revive, not the
  live predecessor. Kill scope is ambient + discovered-meta servers only (an unrepresented
  named server is missed), and the handoff takes no `telegram/control.lock`, so bash
  start/stop/supervise can interleave. Defect: #84 (intended: prove every predecessor
  absent on the complete scope, serialize control before first send). Destructive tmux
  targets here are raw prefix-matchable names — defect #85. #83 remains the separate
  explicit-start bypass.** A process termination that bypasses cleanup leaves the
  marker and the last heartbeat; after the heartbeat ages beyond 90 seconds, Bash's guard
  no longer returns early (`ae:10479-10484`).
- `clear_bridge_owner` only unlinks a marker whose PID field equals the clearing process
  (`aewatch:2878-2891`). A marker replaced by a different writer is left by an older process.

## Shared Telegram stores

The production bridge wires all three stores to `$AE_HOME/telegram`
(`aewatch:3389-3409`). None has a store-specific file lock. The aewatch singleton and Bash Telegram
daemon lock are different paths: aewatch uses `aewatch.lock` (`aewatch:2844-2850`), while
Bash uses `$AE_HOME/telegram/daemon.lock` (`ae:9327-9333`, `ae:10177-10185`).

### `tg_offset`

- `OffsetStore.load` reads the file directly, returning `0` for missing, unreadable, or
  non-numeric content (`aewatch:893-906`). `save` writes `tg_offset.tmp.<pid>` in place and
  atomically replaces `tg_offset`; failed saves try to remove the temp and return `False`
  (`aewatch:908-921`).
- `poll_inbound` saves each accepted `update_id` before authorization/dispatch and stops
  the batch when the save fails (`aewatch:948-993`).
- Bash uses the same path (`ae:9327-9333`, `ae:9380-9382`), reads it without a lock
  (`ae:9603-9609`), and writes `<tg_offset>.tmp.$$` followed by `mv` (`ae:9611-9620`). Its
  poll loop also persists before dispatch (`ae:9990-10022`).
- Both implementations publish complete replacement files, but no common lock orders
  reads/writes. A crash before replacement can leave a temp and preserve the old offset;
  concurrent complete replacements leave whichever replacement is last observed at the
  canonical path.

### `state.tsv`

- `OutboundState.load` reads and parses the file without a lock; it accepts rows with at
  least three tab fields and numeric inode/offset, and ignores malformed rows
  (`aewatch:1411-1436`). `save` serializes the complete map to a `0600` temp and
  `os.replace`s it over the path (`aewatch:1438-1456`). The compatibility fourth column is
  emitted empty (`aewatch:1443-1449`).
- `TelegramBridge._drain_outbound` loads once, updates per-session inode/byte offsets while
  consuming event files, then saves the whole map (`aewatch:1608-1620`).
- Bash uses `$AE_HOME/telegram/state.tsv` (`ae:9327-9333`), reads it directly into
  associative arrays (`ae:10073-10084`), and writes a complete temp+rename snapshot
  (`ae:10086-10099`). The Bash daemon loads it after taking `daemon.lock`
  (`ae:10177-10185`, `ae:10225-10232`), updates it each loop, and saves from the EXIT trap
  (`ae:10214-10223`, `ae:10241-10254`).
- There is no `state.tsv.lock` and no lock shared with aewatch. Each side can replace the
  complete canonical file while the other has an older in-memory map; the later replacement
  can omit offsets written by the earlier one. A temp can remain after a crash, while the
  old canonical file remains until a successful replace.

### `current_target`

- `CurrentTarget.load` reads the target directly with no lock and requires one tab plus
  non-empty session/agent fields (`aewatch:1110-1126`). `save` writes a `0600` temp then
  `os.replace`s it (`aewatch:1128-1143`); `clear` unlinks the canonical path directly and
  ignores `OSError` (`aewatch:1145-1149`).
- Inbound routing reads and writes this store through `_route_use` and `route_message`
  (`aewatch:1269-1306`, `aewatch:1324-1356`).
- Bash uses `$AE_HOME/telegram/current_target` (`ae:9856-9859`), reads it directly for
  `/use` and sticky routing (`ae:9881-9898`, `ae:9965-9973`), unlinks it for clear
  (`ae:9900-9905`), and writes it in place with `printf >` under `umask 077`
  (`ae:9929-9935`).
- No lock is shared. Bash's redirect can leave an empty/partial canonical target if stopped
  during the write; aewatch's temp+replace preserves the previous canonical file until
  replace. Aewatch `clear` and Bash read/write are independent path operations.

## Shared session metadata and recovery

### `meta` reads and Bash writes

Aewatch reads the flat session metadata without `meta.lock` in all of these paths:

- Discovery reads `$AE_HOME/sessions/*/meta` and extracts session, IDs, agents, work dir,
  and `tmux_server` (`aewatch:1806-1856`).
- Telegram/session routing and watchdog helpers reread agent bins, refs, main agent, and
  arbitrary keys (`aewatch:2147-2195`, `aewatch:2245-2250`, `aewatch:2317-2331`).
- `find_steward` resolves `meta_agent=true` and the main-agent ref from the same metadata
  (`aewatch:1159-1179`).
- `make_recover_pending` reads `ae_path` from metadata before invoking Bash
  (`aewatch:2409-2427`).

Bash's normal metadata writer takes `<meta>.lock`, reads the whole file, writes a temp, and
  renames it while fd 200 is held (`ae:2348-2436`). **CORRECTED (audit I5): not all
  writers are temp+rename — `start_capture_session_id` appends `launch_time.*` DIRECTLY
  under meta.lock (`ae:2068-2075`), and `_cmd_spawn` directly appends agent/bin/launch_id
  rows under meta.lock (`ae:11923-11945`).** Aewatch's direct reads take no lock, so they
  can observe meta before, during, or after a replacement — including PARTIAL canonical
  meta from the direct-append writers. Aewatch has no direct `meta` writer of its own.

The recovery boundary is cross-language: aewatch shells the executable recorded in `ae_path`
as `ae _recover-pending <session>` and parses its TSV output (`aewatch:2409-2450`). Bash's
recovery check-then-set takes `meta.lock` and rewrites the pending `agent.<slot>` line with
temp+rename (`ae:8717-8732`); aewatch can read that metadata concurrently. Aewatch's
Telegram delivery similarly invokes the session's Bash `send`/`ask` helper rather than raw
tmux (`aewatch:1074-1098`), so the helper owns its normal body/event writes.

### `meta-agent-state.json`

Aewatch reads only the mtime of the steward state file and returns `0` when absent or
unreadable (`aewatch:2253-2264`). Bash's steward watchdog reads the same path and mtime
(`ae:16110-16132`); the Bash comments identify `contrib/aemonitor` as the writer for each
real sweep (`ae:16122-16129`). Neither the aewatch read nor the Bash read takes a file lock.

### `config` and token file

Aewatch reloads `$AE_HOME/config` each bridge tick via the ae-compatible INI parser
(`aewatch:1700-1757`, `aewatch:1571-1580`) and reads the configured Telegram token file
(`aewatch:731-773`). **CORRECTED (audit I4):** `telegram setup` directly OVERWRITES the
token file and directly appends config; `_telegram_persist_intent` writes config
DIRECTLY when the file/section is missing and uses temp+mv only for an existing section
— the token is NOT read-only. There is no shared config lock; aewatch can observe a
partial setup write. Aewatch's `load_config` reads ONLY `$AE_HOME/config` — it catches
missing-file but not read errors (an OSError crashes a component path) and deliberately
ignores bash's `CONFIG_FILE`/`AE_LOCAL_CONFIG`, so **the two backends do not always read
the same config** (S10/S15 conflict candidate: interchangeable backends need one
explicit effective-config authority — fix or DR).

## Shared event/tmux state and tmux mutation paths

### Event file overlap recap

The event file is read by aewatch without a lock (`aewatch:2029-2098`, `aewatch:2267-2314`,
`aewatch:1463-1508`) and appended by aewatch with `<events>.lock`
(`aewatch:2357-2406`). Bash appends under the same lock (`ae:13171-13177`), reads some
paths under a shared lock (`ae:13179-13185`), performs many line-oriented reads directly,
and atomically trims/replaces the file under that lock (`ae:18046-18075`).

### Tmux user options

- `RealTmuxClient` reads sessions, panes, pane capture, pane command, and user options by
  invoking tmux without an ae file lock (`aewatch:553-636`, `aewatch:648-655`). Its mutation
  methods run `set-option`, `send-keys`, `paste-buffer`, `display-message`, and
  `kill-session`, then record effects (`aewatch:638-700`).
- The watchdog writes `@ae_watchdog_status`, `@ae_branch_name` (set/unset), and
  `@ae_branch_status` in that order (`aewatch:1973-2006`). Bash reads `@ae_branch_name` for
  `ae list` (`ae:3217-3233`), renders `@ae_branch_status` and `@ae_watchdog_status`
  (`ae:1283-1285`), writes the same branch/status options in its watchdog
  (`ae:15524-15557`), and clears them on watchdog exit (`ae:15654-15669`). There is no
  ae-level lock around these tmux option reads/writes; tmux server operations provide the
  only substrate-level serialization.
- Aewatch watchdog `paste` does not acquire Bash's per-pane `send-lock-*` before sending
  (`aewatch:678-699`). A Bash helper send path does acquire its target flock before tmux
  delivery (`ae:12993-13000`, `ae:14235-14275`).

### Telegram and aewatch tmux sessions

- `ensure_aewatch_session` probes, kills stale, creates, and configures the dedicated
  `ae-aewatch` tmux session; it uses tmux subprocesses and the heartbeat freshness check,
  not an ae file lock (`aewatch:1634-1677`). Bash checks the same session and heartbeat and
  starts `aewatch up` from its autostart path (`ae:10402-10449`).
- The handoff kills the Bash `ae-telegram` session on the ambient server and every
  discovered named server (`aewatch:3276-3288`). Bash names/checks the same session
  (`ae:9184-9203`, `ae:9247-9252`) and starts/stops/supervises it through its separate
  control lock (`ae:10508-10524`).

These tmux operations have no common ae lock with Bash. They can therefore interleave at the
tmux server/session/pane level even when the file-backed singleton or Telegram daemon lock is
held by one implementation.

## Residue summary for the shared surface

- `events.jsonl.lock`, `meta.lock`, `telegram/control.lock`, `telegram/daemon.lock`, and
  `aewatch.lock` are lock-path files whose flock ownership ends with the owning descriptor;
  aewatch does not open Bash's `meta.lock`, `control.lock`, or `daemon.lock`.
- Aewatch event appends are direct and can leave a partial final line; its event readers do
  not lock. Bash event appends use the same lock, while Bash retention can replace the inode.
- Aewatch's `meta` and `meta-agent-state.json` accesses are reads; Bash/aemonitor are the
  writers for the corresponding shared paths. Aewatch's recovery and Telegram delivery
  boundaries can invoke Bash code that performs the writes.
- `tg_offset`, `state.tsv`, and `current_target` have no lock common to the two bridges.
  Aewatch uses temp+replace for all three writes (and direct unlink for current-target clear);
  Bash uses temp+rename for the two offset snapshots and direct redirect for current-target.
- Marker and heartbeat publication is temp+replace, but ownership checks and clear are
  unlocked path operations. Clean shutdown clears the marker; termination that bypasses
  cleanup leaves marker/heartbeat residue until Bash's 90-second freshness test stops
  treating the sidecar as owner.

## Audited addenda (colead batch, memo topic census3-audit, 2026-08-20; lead-transcribed)

Defect issues opened from this audit: **#84** (fail-open takeover), **#85**
(prefix-matchable destructive tmux targets). **#83** remains the separate explicit-start
bypass. B3 (below) awaits a joint fix-vs-DR choice.

### B3 — append-only contract vs resume trim (normative conflict, NOT conflict=none)

bridge-protocol.md:90-95 and events.md:142-148 promise past lines never change, no
rotation, lifetime growth. Frozen `ae:18046-18075` REPLACES `events.jsonl` with the
newest N lines on resume. Joint resolution pending: fix-known-defect (preserve
append-only) or a DR for explicit generations/rotation + reader-cursor migration; #21 is
a candidate implementation vehicle but does not itself resolve the conflict.

### I1 — event reader stat/open generation race (probe-verified, exit 0)

aewatch:1470-1508 and bash:10123-10174 stat the path, later open it, and return the stat
inode without the event lock. A resume-trim replacement between stat and open caused
`new-first` to send with the OLD inode retained; the next tick saw the new inode, jumped
to EOF, and silently skipped `new-second`. Even an un-raced inode change jumps to current
EOF — events appended after trim and before the first poll are lost. Resolution rides B3.

### I2 — `_locked_append` failure directions are two, not one

Returns false only after repeated flock OSError until deadline (contention
indistinguishable from other flock errors); lock-path open, event open, write, close,
and unlock errors ESCAPE the false-return path, abort the watchdog component tick, enter
crash/backoff, and can eventually stop the combined daemon. `make_emit_event` drops only
the false return.

### I3 — shared Telegram store caller semantics (transition triggers, not generic LWW)

`_drain_outbound` ignores `OutboundState.save(False)` — sends can succeed while the
durable offset stays old: replay/duplicates on next tick or restart (at-least-once needs
its own row). `CurrentTarget.clear` ignores unlink failure and `/use clear` replies
success with the sticky target still active (known-defect candidate: success only after
durable clear). During takeover, bash EXIT can save an old in-memory `state.tsv` AFTER
the kill while aewatch has begun — no common store lock, so later replacement can
regress offsets.

### I6 — `up` is outside the daemon singleton

`ensure_aewatch_session` probes/kills/creates BEFORE any `aewatch.lock`; concurrent
autostarts can race and kill/recreate each other. The lifetime singleton covers
loop/tick only, never up/start orchestration.

### I7 — delivery-guard asymmetry (#45)

Bash watchdog nudges go through generated `send` (per-pane lock, busy/human/dead/
verified-submit guards); aewatch calls `RealTmuxClient.paste` directly with no shared
target lock and no guards. Classified fix-known-defect(#45): Rust daemons use the one
verified delivery primitive.

### I8 — marker/heartbeat precision

Temp+replace gives atomic VISIBILITY, not power-loss durability (no fsync anywhere);
"durable fact" here means stranger-readable process state — do not promise crash/power
persistence. A first-handoff `write_heartbeat` exception lands after marker publication
and before local clear; loop containment catches it, so the marker stands until retry,
loop-exit cleanup, or freshness decay. Clean SIGTERM/HUP/INT runs the CLI `finally`;
only crash/SIGKILL/bypass leaves permanent residue.

### Citation nits (accepted)

`record_success` does only `_save([])` (not load/prune) — census line 132.
`kill_session` records no effect (lines 297-300). tmux status: ae:15524-15557 is the
branch segment; status writes are ae:15985-15986 and 16570-16571. Autostart uses
NONBLOCKING flock; explicit start uses `flock -w 5` (line 169). The logger records
`log.write` BEFORE mkdir/rotate/write — a caught write failure can leave a phantom
oracle effect.
