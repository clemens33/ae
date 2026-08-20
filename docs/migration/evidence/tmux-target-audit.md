# Tmux Target Addressing Audit — Frozen Bash `ae`

Audit date: 2026-08-20.
Source audited: `72c729343a0117af2968b66e1c43f89ad25fc0b2:ae` (`/tmp/aefrozen.sh`, 18,241 lines).
Scope: Every tmux invocation taking a target in frozen bash `ae`, classified by target type, operation class, prefix-matchability, and server pinning.

## Census Summary

- **Total target invocations audited:** 171
- **Class breakdown:**
  - `ALREADY-EXACT-ID`: 102
  - `DESTRUCTIVE-MUTATION`: 36
  - `EXISTENCE-GATE`: 12
  - `READ`: 21
  - `UNKNOWN`: 0
- **Target Type breakdown:**
  - `pane-id`: 79
  - `session-NAME`: 60
  - `session-id`: 21
  - `compound`: 8
  - `window-id`: 2
  - `other`: 1 (line 13148: `${pane:-$_AE_SESSION}` dynamically resolves to pane-id or session-NAME)
- **Total rows in RISK section:** 48 (session-NAME or compound targets with DESTRUCTIVE-MUTATION or EXISTENCE-GATE class)

## Target Classification Register

| Line | Subcommand | Target Expression | Target Type | Class | Prefix-Matchable? | Names Server (-S/-L)? | Context / Function |
|---|---|---|---|---|---|---|---|
| 425 | `display-message` | `"$1"` | pane-id | ALREADY-EXACT-ID | no | no | `pane_current_command` |
| 941 | `send-keys` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `reset_shell_launch_input` |
| 943 | `send-keys` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `reset_shell_launch_input` |
| 1262 | `has-session` | `"$session"` | session-NAME | EXISTENCE-GATE | yes | yes | `_ae_apply_status_bar` |
| 1275 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1276 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1285 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1286 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1287 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1293 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1302 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1308 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1319 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1320 | `set-option` | `"$_st"` | session-id | ALREADY-EXACT-ID | no | yes | `_ae_apply_status_bar` |
| 1856 | `capture-pane` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `capture_codex_session_id` |
| 2324 | `switch-client` | `"$target"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `tmux_attach` |
| 2326 | `attach-session` | `"$target"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `tmux_attach` |
| 2688 | `show-environment` | `"$name"` | session-NAME | READ | yes | no | `list_ae_sessions` |
| 2706 | `has-session` | `"$name"` | session-NAME | EXISTENCE-GATE | yes | no | `iter_stopped_sessions` |
| 2857 | `show-environment` | `"$_sid"` | session-id | ALREADY-EXACT-ID | no | yes | `resolve_session_workdir` |
| 2977 | `show-environment` | `"$_es_sid"` | session-id | ALREADY-EXACT-ID | no | yes | `_end_session_locked` |
| 2978 | `show-environment` | `"$_es_sid"` | session-id | ALREADY-EXACT-ID | no | yes | `_end_session_locked` |
| 3193 | `show-environment` | `"$name"` | session-NAME | READ | yes | no | `_session_env_map` |
| 3227 | `show-options` | `"$name"` | session-NAME | READ | yes | no | `_session_branch` |
| 3631 | `list-panes` | `"$name"` | session-NAME | READ | yes | no | `_session_attn_rollup` |
| 3841 | `$(_next_focus_verb)` | `"$best_name"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `cmd_next` |
| 4207 | `list-panes` | `"$name"` | session-NAME | READ | yes | no | `cmd_list` |
| 6470 | `has-session` | `"$target"` | session-NAME | EXISTENCE-GATE | yes | no | `cmd_status` |
| 6486 | `capture-pane` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `cmd_status` |
| 6488 | `list-panes` | `"$target"` | session-NAME | READ | yes | no | `cmd_status` |
| 6890 | `display-message` | `"$_pane"` | pane-id | ALREADY-EXACT-ID | no | yes | `_stop_self_target_from_pane` |
| 6946 | `display-message` | `"$_pane"` | pane-id | ALREADY-EXACT-ID | no | yes | `_stop_current_target_proven` |
| 6977 | `display-message` | `"${_AE_STOP_PANE:-${TMUX_PANE:-}}"` | pane-id | ALREADY-EXACT-ID | no | yes | `_stop_is_self` |
| 7893 | `kill-session` | `"$sid"` | session-id | ALREADY-EXACT-ID | no | yes | `_lifecycle_kill_verified` |
| 8263 | `show-environment` | `"$_sid"` | session-id | ALREADY-EXACT-ID | no | yes | `_end_will_push` |
| 8649 | `list-panes` | `"$session_name"` | session-NAME | READ | yes | yes | `sync_existing_session_assets` |
| 9029 | `has-session` | `"=${_oname}"` | compound | EXISTENCE-GATE | no | yes | `cmd_doctor` |
| 10344 | `set-option` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_telegram_spawn_daemon` |
| 10345 | `rename-window` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_telegram_spawn_daemon` |
| 10673 | `kill-session` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `cmd_telegram_stop` |
| 11617 | `has-session` | `"$old_name"` | session-NAME | EXISTENCE-GATE | yes | no | `_cmd_rename_locked` |
| 11622 | `has-session` | `"$new_name"` | session-NAME | EXISTENCE-GATE | yes | no | `_cmd_rename_locked` |
| 11635 | `rename-session` | `"$old_name"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_cmd_rename_locked` |
| 11645 | `rename-window` | `"${new_name}:0"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_cmd_rename_locked` |
| 11771 | `capture-pane` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_tool_initializing` |
| 11823 | `capture-pane` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_spawn_input_ready` |
| 11867 | `display-message` | `"$caller_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 11898 | `has-session` | `"$session"` | session-NAME | EXISTENCE-GATE | yes | no | `_cmd_spawn` |
| 11921 | `new-window` | `"${session}:"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_cmd_spawn` |
| 11949 | `select-pane` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 11950 | `set-option` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 11951 | `set-option` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 11952 | `rename-window` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 12038 | `capture-pane` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 12040 | `send-keys` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_spawn` |
| 12151 | `list-panes` | `"$session"` | session-NAME | READ | yes | no | `_cmd_retire` |
| 12170 | `list-panes` | `"$session"` | session-NAME | READ | yes | no | `_cmd_retire` |
| 12214 | `kill-pane` | `"$resolved"` | pane-id | ALREADY-EXACT-ID | no | no | `_cmd_retire` |
| 12297 | `list-panes` | `"$sess"` | session-NAME | READ | yes | no | `regenerate_manifest` |
| 12580 | `kill-pane` | `"$pane_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_spawn_rollback` |
| 12889 | `display-message` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_resolve` |
| 12890 | `display-message` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_resolve` |
| 12891 | `display-message` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_resolve` |
| 12919 | `has-session` | `"$search_session"` | session-NAME | EXISTENCE-GATE | yes | no | `ae_resolve` |
| 12962 | `list-panes` | `"$search_session"` | session-NAME | READ | yes | no | `ae_resolve` |
| 12987 | `display-message` | `"$resolved"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_resolve` |
| 12989 | `display-message` | `"$resolved"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_resolve` |
| 13023 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_current_agent` |
| 13033 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_current_slot` |
| 13060 | `list-panes` | `"$search"` | session-NAME | READ | yes | no | `ae_slot_resolver` |
| 13138 | `set-option` | `"$_pid"` | pane-id | ALREADY-EXACT-ID | no | yes | `_stamp_live_slots` |
| 13139 | `list-panes` | `"$sess"` | session-NAME | READ | yes | yes | `_stamp_live_slots` |
| 13148 | `display-message` | `"${pane:-$_AE_SESSION}"` | other | READ | yes | no | `ae_current_agent_ref` |
| 13419 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | yes | `ae_provenance_sender` |
| 13422 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | yes | `ae_provenance_sender` |
| 13425 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | yes | `ae_provenance_sender` |
| 13568 | `capture-pane` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_capture_input_region` |
| 14004 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_agent_bin_for_pane` |
| 14009 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_agent_bin_for_pane` |
| 14035 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_pane_agent_is_dead` |
| 14045 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_pane_agent_is_dead` |
| 14080 | `paste-buffer` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_submit_pasted_message` |
| 14085 | `paste-buffer` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_submit_pasted_message` |
| 14099 | `send-keys` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_submit_pasted_message` |
| 14107 | `send-keys` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `ae_submit_pasted_message` |
| 14268 | `send-keys` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_send_main` |
| 14341 | `display-message` | `"$(ae_current_pane)"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_reply_main` |
| 14430 | `display-message` | `"$(ae_current_pane)"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_requests_main` |
| 14614 | `capture-pane` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_peek_main` |
| 14625 | `has-session` | `"$sess"` | session-NAME | EXISTENCE-GATE | yes | no | `helper_agents_main` |
| 14629 | `list-panes` | `"$sess"` | session-NAME | READ | yes | no | `helper_agents_main` |
| 14636 | `list-panes` | `"$_AE_SESSION"` | session-NAME | READ | yes | no | `helper_agents_main` |
| 14651 | `select-window` | `"$AE_RESOLVED_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_focus_main` |
| 14652 | `select-pane` | `"$AE_RESOLVED_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_focus_main` |
| 14682 | `send-keys` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_interrupt_main` |
| 14683 | `send-keys` | `"$target"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_interrupt_main` |
| 14965 | `list-panes` | `"$_AE_SESSION"` | session-NAME | READ | yes | no | `_monitor_find_pane` |
| 14978 | `new-window` | `"${_AE_SESSION}:99"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_monitor_ensure_events_pane` |
| 14980 | `new-window` | `"$_AE_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_monitor_ensure_events_pane` |
| 14985 | `set-option` | `"$MONITOR_EVENTS_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `_monitor_ensure_events_pane` |
| 14986 | `select-pane` | `"$MONITOR_EVENTS_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `_monitor_ensure_events_pane` |
| 14987 | `set-window-option` | `"${_AE_SESSION}:ae-monitor"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_monitor_ensure_events_pane` |
| 14988 | `select-pane` | `"$MONITOR_EVENTS_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `_monitor_ensure_events_pane` |
| 15021 | `kill-pane` | `"$legacy_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_reap_legacy_one` |
| 15051 | `kill-pane` | `"$watchdog_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_stop` |
| 15089 | `split-window` | `"$MONITOR_EVENTS_PANE"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_start` |
| 15094 | `set-option` | `"$watchdog_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_start` |
| 15095 | `select-pane` | `"$watchdog_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_start` |
| 15097 | `select-pane` | `"$watchdog_pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_start` |
| 15376 | `capture-pane` | `"$1"` | pane-id | ALREADY-EXACT-ID | no | no | `_watchdog_capture_pane` |
| 15536 | `set-option` | `"$_bs_sid"` | session-id | ALREADY-EXACT-ID | no | no | `_watchdog_branch_segment` |
| 15545 | `set-option` | `"$_bs_sid"` | session-id | ALREADY-EXACT-ID | no | no | `_watchdog_branch_segment` |
| 15629 | `set-option` | `"$_sid"` | session-id | ALREADY-EXACT-ID | no | no | `_watchdog_set_status` |
| 15660 | `set-option` | `"$_sid"` | session-id | ALREADY-EXACT-ID | no | no | `_watchdog_clear_bar_options` |
| 15668 | `set-option` | `"$_wi"` | window-id | ALREADY-EXACT-ID | no | no | `_watchdog_clear_bar_options` |
| 15669 | `list-windows` | `"$_sid"` | session-id | ALREADY-EXACT-ID | no | no | `_watchdog_clear_bar_options` |
| 15886 | `capture-pane` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `_pane_shows_throttle` |
| 16021 | `has-session` | `"$_AE_SESSION"` | session-NAME | EXISTENCE-GATE | yes | no | `helper_watchdog_main` |
| 16073 | `display-message` | `"$pane"` | pane-id | ALREADY-EXACT-ID | no | no | `helper_watchdog_main` |
| 16511 | `list-panes` | `"$_AE_SESSION"` | session-NAME | READ | yes | no | `helper_watchdog_main` |
| 16617 | `set-option` | `"$_roster_sid"` | session-id | ALREADY-EXACT-ID | no | no | `helper_watchdog_main` |
| 16633 | `set-option` | `"$_w_id"` | window-id | ALREADY-EXACT-ID | no | no | `helper_watchdog_main` |
| 16651 | `list-panes` | `"$_AE_SESSION"` | session-NAME | READ | yes | no | `helper_watchdog_main` |
| 16864 | `kill-session` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_rollback` |
| 16989 | `has-session` | `"$SESSION"` | session-NAME | EXISTENCE-GATE | yes | no | `_launch_parse_flags` |
| 17053 | `has-session` | `"$SESSION"` | session-NAME | EXISTENCE-GATE | yes | no | `_launch_parse_flags` |
| 17054 | `show-environment` | `"$SESSION"` | session-NAME | READ | yes | no | `_launch_parse_flags` |
| 17307 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17308 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17311 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17312 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17313 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17314 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17318 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17321 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17322 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17323 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17324 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17325 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17327 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17328 | `rename-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17330 | `display-message` | `"$SESSION"` | session-NAME | READ | yes | no | `_launch_parse_flags` |
| 17338 | `split-window` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17340 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17343 | `split-window` | `"${PANE_IDS[$_prev]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17348 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17351 | `split-window` | `"${PANE_IDS[$_prev]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17354 | `split-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17356 | `split-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17365 | `select-layout` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17367 | `select-layout` | `"${PANE_IDS[2]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17372 | `select-layout` | `"${PANE_IDS[1]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17375 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17377 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17384 | `rename-window` | `"${PANE_IDS[1]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17389 | `rename-window` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17391 | `rename-window` | `"${PANE_IDS[2]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17396 | `select-pane` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17397 | `set-option` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17398 | `set-option` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17401 | `select-pane` | `"${PANE_IDS[$idx]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17402 | `set-option` | `"${PANE_IDS[$idx]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17403 | `set-option` | `"${PANE_IDS[$idx]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17462 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17463 | `rename-window` | `"$local_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17465 | `select-pane` | `"$local_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17466 | `set-option` | `"$local_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17467 | `set-option` | `"$local_id"` | pane-id | ALREADY-EXACT-ID | no | no | `_launch_parse_flags` |
| 17475 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 17477 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | yes | no | `_launch_parse_flags` |
| 18236 | `select-pane` | `"${PANE_IDS[0]}"` | pane-id | ALREADY-EXACT-ID | no | no | `_send_or_rollback` |

## Risk Section (Session Name / Compound Targets with Mutation or Existence Gate)

The following 48 invocations address tmux using a session NAME or compound target (`name:window` / `name:`) and perform either a DESTRUCTIVE-MUTATION (writing, killing, renaming, splitting, layout changes) or an EXISTENCE-GATE (`has-session`). Because tmux prefix-matches session names by default, an invocation targeting an absent session can silently match, mutate, kill, or falsely validate a running prefix-sibling session (e.g. targeting `foo` when `foobar` exists, as exposed in issue #102).

| Line | Subcommand | Target Expression | Target Type | Class | Consequence of Prefix Matching |
|---|---|---|---|---|---|
| 1262 | `has-session` | `"$session"` | session-NAME | EXISTENCE-GATE | If `$session` is absent but a prefix sibling is live, `has-session` falsely succeeds instead of returning 0, proceeding to apply status bar options on the prefix sibling or querying its state. |
| 2324 | `switch-client` | `"$target"` | session-NAME | DESTRUCTIVE-MUTATION | If `$target` is absent or matches a prefix of a live session, `switch-client` switches the client to the prefix sibling session instead of the requested session. |
| 2326 | `attach-session` | `"$target"` | session-NAME | DESTRUCTIVE-MUTATION | If `$target` is absent or matches a prefix of a live session, `attach-session` attaches the terminal to the prefix sibling session instead of the requested session. |
| 2706 | `has-session` | `"$name"` | session-NAME | EXISTENCE-GATE | If stopped session `$name` is a prefix of an active session, `has-session` falsely succeeds on the active sibling, wrongly hiding the stopped session from the stopped-sessions listing. |
| 3841 | `$(_next_focus_verb)` | `"$best_name"` | session-NAME | DESTRUCTIVE-MUTATION | If `$best_name` is a prefix of another session, `$(_next_focus_verb)` (`switch-client` or `attach-session`) focuses the prefix sibling session rather than `$best_name`. |
| 6470 | `has-session` | `"$target"` | session-NAME | EXISTENCE-GATE | If `$target` is absent but a prefix sibling is live, `has-session` falsely succeeds and `cmd_status` prints pane status for the wrong prefix sibling session. |
| 9029 | `has-session` | `"=${_oname}"` | compound | EXISTENCE-GATE | If `=${_oname}` were evaluated without exact-match protection, `has-session` would falsely detect a live session and refuse doctor migration/cleanup. |
| 10344 | `set-option` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$_TELEGRAM_TMUX_SESSION` (e.g. `ae-telegram`) is absent or a prefix of another session, mutates the status line option on the wrong sibling session. |
| 10345 | `rename-window` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$_TELEGRAM_TMUX_SESSION` is a prefix of another session, renames window 0 of the wrong sibling session to 'ae-telegram'. |
| 10673 | `kill-session` | `"$_TELEGRAM_TMUX_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$_TELEGRAM_TMUX_SESSION` is absent but a prefix sibling exists (e.g. `ae-telegram-bot`), kills the prefix sibling session instead. |
| 11617 | `has-session` | `"$old_name"` | session-NAME | EXISTENCE-GATE | If `$old_name` is absent but a prefix sibling is live, `has-session` falsely reports the session exists, allowing the rename command to proceed against the sibling. |
| 11622 | `has-session` | `"$new_name"` | session-NAME | EXISTENCE-GATE | If `$new_name` is absent but a prefix sibling is live, `has-session` falsely reports the target name is already in use and wrongly aborts a valid rename. |
| 11635 | `rename-session` | `"$old_name"` | session-NAME | DESTRUCTIVE-MUTATION | If `$old_name` is absent but a prefix sibling is live, `rename-session` renames the prefix sibling session to `$new_name` (the exact issue #102 defect). |
| 11645 | `rename-window` | `"${new_name}:0"` | compound | DESTRUCTIVE-MUTATION | If `$new_name` prefix-matches another session, `rename-window` renames window 0 of the wrong sibling session. |
| 11898 | `has-session` | `"$session"` | session-NAME | EXISTENCE-GATE | If `$session` is absent but a prefix sibling is live, `has-session` falsely succeeds and allows worker spawn to proceed into the wrong session. |
| 11921 | `new-window` | `"${session}:"` | compound | DESTRUCTIVE-MUTATION | If `$session` prefix-matches another live session, `new-window` creates the new worker window/pane inside the sibling session instead of the target session. |
| 12919 | `has-session` | `"$search_session"` | session-NAME | EXISTENCE-GATE | If `$search_session` is absent but a prefix sibling is live, `has-session` falsely reports the session is running and searches for agents in the sibling session. |
| 14625 | `has-session` | `"$sess"` | session-NAME | EXISTENCE-GATE | If an archived or stopped session name `$sess` prefix-matches a live session, `has-session` falsely treats it as running and queries panes from the sibling. |
| 14978 | `new-window` | `"${_AE_SESSION}:99"` | compound | DESTRUCTIVE-MUTATION | If `$_AE_SESSION` prefix-matches another live session, `new-window` creates the `ae-monitor` events window in the wrong sibling session. |
| 14980 | `new-window` | `"$_AE_SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$_AE_SESSION` prefix-matches another live session, `new-window` creates the `ae-monitor` window in the wrong sibling session. |
| 14987 | `set-window-option` | `"${_AE_SESSION}:ae-monitor"` | compound | DESTRUCTIVE-MUTATION | If `$_AE_SESSION` prefix-matches another live session, `set-window-option` mutates window options on the wrong sibling session. |
| 16021 | `has-session` | `"$_AE_SESSION"` | session-NAME | EXISTENCE-GATE | If `$_AE_SESSION` dies while a prefix sibling session is running, `has-session` falsely succeeds, preventing the watchdog from detecting session death. |
| 16864 | `kill-session` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If launch fails before creation on this server and a prefix sibling exists, `kill-session` kills the running prefix sibling session during rollback. |
| 16989 | `has-session` | `"$SESSION"` | session-NAME | EXISTENCE-GATE | If `$SESSION` does not exist but a prefix sibling is running, `has-session` falsely treats the session as live and enters the resume/attach branch. |
| 17053 | `has-session` | `"$SESSION"` | session-NAME | EXISTENCE-GATE | If `$SESSION` does not exist but a prefix sibling is running, `has-session` falsely treats the session as live and attempts to inspect and resume the sibling session. |
| 17307 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` unsets `CLAUDECODE` in the prefix sibling session. |
| 17308 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` unsets `CLAUDE_CODE_SESSION` in the prefix sibling session. |
| 17311 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` sets `AE_SESSION=1` in the prefix sibling session. |
| 17312 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` overwrites `AE_ORIGIN` in the prefix sibling session. |
| 17313 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` overwrites `AE_DIR` in the prefix sibling session. |
| 17314 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` overwrites `AE_MODE` in the prefix sibling session. |
| 17318 | `set-environment` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-environment` overwrites `AE_HOME` in the prefix sibling session. |
| 17321 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` enables `mouse` in the prefix sibling session. |
| 17322 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` enables `focus-events` in the prefix sibling session. |
| 17323 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` sets `history-limit` in the prefix sibling session. |
| 17324 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` sets `pane-border-status` in the prefix sibling session. |
| 17325 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` sets `pane-border-format` in the prefix sibling session. |
| 17327 | `set-option` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `set-option` disables `automatic-rename` in the prefix sibling session. |
| 17328 | `rename-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `rename-window` renames window 0 of the prefix sibling session. |
| 17340 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `new-window` creates an unintended worker window inside the prefix sibling session. |
| 17348 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `new-window` creates an unintended worker window inside the prefix sibling session. |
| 17354 | `split-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `split-window` splits a pane in the prefix sibling session. |
| 17356 | `split-window` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `split-window` splits a pane in the prefix sibling session. |
| 17375 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `select-layout` mutates window layout in the prefix sibling session. |
| 17377 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `select-layout` mutates window layout in the prefix sibling session. |
| 17462 | `new-window` | `"${SESSION}:"` | compound | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `new-window` creates an unintended spawned worker window in the prefix sibling session. |
| 17475 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `select-layout` mutates window layout in the prefix sibling session. |
| 17477 | `select-layout` | `"$SESSION"` | session-NAME | DESTRUCTIVE-MUTATION | If `$SESSION` creation collided or failed and a prefix sibling exists, `select-layout` mutates window layout in the prefix sibling session. |

## Target Expression & Variable Reference Notes

1. **Exact IDs (`ALREADY-EXACT-ID`):**
   - **Pane IDs (`pane-id`):** `%...` (e.g. `%0`, `%3`), passed via `$1`, `$pane`, `$pane_id`, `$caller_pane`, `$resolved`, `$local_id`, `$legacy_pane`, `$watchdog_pane`, `$MONITOR_EVENTS_PANE`, `$AE_RESOLVED_PANE`, `$_pid`, `$_pane`, `${PANE_IDS[...]}`. Tmux pane IDs are exact numeric handles prefixed with `%` and do not prefix-match.
   - **Window IDs (`window-id`):** `@...` (e.g. `@0`, `@1`), passed via `$_wi`, `$_w_id` (obtained from `list-windows -F '#{window_id}'`). Tmux window IDs are exact numeric handles prefixed with `@` and do not prefix-match.
   - **Session IDs (`session-id`):** `$...` (e.g. `$0`, `$1`), passed via `$_st`, `$_sid`, `$_es_sid`, `$sid`, `$_bs_sid`, `$_roster_sid` (resolved via `_roster_session_id` exact-name filtering against `list-sessions -F '#{session_id} #{session_name}'` or `_end_live_id`). Tmux session IDs are exact handles prefixed with `$` and do not prefix-match.

2. **Session Names (`session-NAME`):**
   - Unqualified string names passed via `$session`, `$name`, `$target`, `$best_name`, `$session_name`, `$_TELEGRAM_TMUX_SESSION`, `$old_name`, `$new_name`, `$sess`, `$search_session`, `$search`, `$_AE_SESSION`, `$SESSION`. Tmux targets taking bare session names prefix-match by default.

3. **Compound Targets (`compound`):**
   - `${session}:` or `${SESSION}:`: session name with trailing colon to specify a session target for window creation. Prefix-matches on the session component.
   - `${SESSION}:99` or `${new_name}:0`: `session:window_index`. Prefix-matches on the session component.
   - `${_AE_SESSION}:ae-monitor`: `session:window_name`. Prefix-matches on the session component.
   - `=${_oname}`: exact-match prefix (`=`) on session name. Prefix-matching is disabled in tmux `has-session` when the leading `=` prefix is present.

4. **Dynamic / Other (`other`):**
   - Line 13148: `${pane:-$_AE_SESSION}` in `ae_current_agent_ref`. If `$pane` is passed, evaluates to a `pane-id` (`%...`); if omitted, falls back to `$_AE_SESSION` (`session-NAME`). Classified as `READ` (`display-message`).
