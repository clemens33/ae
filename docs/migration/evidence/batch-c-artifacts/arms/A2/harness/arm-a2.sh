#!/opt/homebrew/bin/bash
# ARM GROUP A2 — list filters. Rows: SC-017a-f, SC-017i, SC-021 (ls alias), SC-521a.
# One invocation per flag AND per documented alias, plain and --json, on `list` and `ls`;
# plus the intersection arms in both orders. Evidence written directly into the committed
# tree with a per-case admissibility ledger.
source "$(dirname "$0")/armlib.sh"
ARMG=A2
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
printf 'c01-filters\tSC-017a,SC-017b,SC-017c,SC-017d,SC-017e,SC-017f,SC-017i,SC-021,SC-521a\tA2\tcomposite\n' >>"$ADEST/$ARMG/ledger.tsv"
RUNNING_SESSIONS=(tg1 twda1 tg2wu tg2b)

a2_case() { # <mode>
    local mode="$1"
    local base="$AROOT/$ARMG/c01-filters-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" c01-filters "$mode"
    led rows "rows=SC-017a-f,SC-017i,SC-021,SC-521a" "template=A2/composite"
    local aehome="$base/home/.ae"
    t_clone A2 composite "$aehome" "$mode" || { led CLONE-FAILED; return 1; }
    local cf exp
    cf="$(dir_fingerprint "$aehome")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    command tmux -S "$sock" kill-server >/dev/null 2>&1
    local s
    for s in "${RUNNING_SESSIONS[@]}"; do
        local meta="$aehome/sessions/$s/meta" first=1 wd
        [[ -f "$meta" ]] || continue
        wd="$(grep '^work_dir=' "$meta" | cut -d= -f2-)"; [[ -d "$wd" ]] || wd="$base"
        while IFS='=' read -r k v; do
            [[ "$k" == agent.* ]] || continue
            local slot="${k#agent.}" ref="${v%:*}" pane
            if ((first)); then
                pane="$(command tmux -S "$sock" new-session -d -s "$s" -c "$wd" -P -F '#{pane_id}' "$FAKE_BIN")"; first=0
            else
                pane="$(command tmux -S "$sock" split-window -d -t "$s" -c "$wd" -P -F '#{pane_id}' "$FAKE_BIN")"
            fi
            command tmux -S "$sock" set-option -p -t "$pane" @ae_agent "$ref"
            command tmux -S "$sock" set-option -p -t "$pane" @ae_slot "$slot"
        done <"$meta"
        # the session environment ae itself stamps at launch (ae@72c7293:17311-17318)
        command tmux -S "$sock" set-environment -t "$s" AE_SESSION 1
        command tmux -S "$sock" set-environment -t "$s" AE_ORIGIN "$(grep '^origin=' "$meta" | cut -d= -f2-)"
        command tmux -S "$sock" set-environment -t "$s" AE_DIR "$wd"
        command tmux -S "$sock" set-environment -t "$s" AE_MODE "$(grep '^mode=' "$meta" | cut -d= -f2-)"
        command tmux -S "$sock" set-environment -t "$s" AE_HOME "$aehome"
    done
    sleep 1
    led live-topology-built "running_sessions=${RUNNING_SESSIONS[*]}" "agent_binary=$FAKE_BIN" \
        "session_env=AE_SESSION/AE_ORIGIN/AE_DIR/AE_MODE/AE_HOME per ae@72c7293:17311-17318" "socket=$sock"
    { echo "arm=$ARMG case=c01-filters clone_mode=$mode template=A2/composite"
      echo "template_fingerprint_pre_protection=$(grep '^fingerprint_pre_protection=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"
      echo "template_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "live_topology_sessions=${RUNNING_SESSIONS[*]}"
      echo "stopped_sessions=twda2 twda3 tg2bl tg2un tg6a tg6b (no tmux session created)"
      echo "live_topology_agent_binary=$FAKE_BIN (the fixture's own controllable fake; never a live model)"
      echo "live_topology_session_env=AE_SESSION/AE_ORIGIN/AE_DIR/AE_MODE/AE_HOME, stamped exactly as ae@72c7293:17311-17318 stamps them"
      echo "clone_preserves_mtime=yes (harness change: events.jsonl MTIME is the frozen reader's activity clock, so a clone must carry the fixture's chosen mtime, not the clone's own)"
      echo "tmux_socket=$sock"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$ACAP/case.txt"
    { echo "## events.jsonl mtimes as the consumer sees them"
      for s in $(ls "$aehome/sessions"); do
          local f="$aehome/sessions/$s/events.jsonl"
          if [[ -f "$f" ]]; then printf '%s\tmtime_epoch=%s\tutc=%s\n' "$s" "$(stat -f %m "$f")" "$(/bin/date -u -r "$(stat -f %m "$f")" +%Y-%m-%dT%H:%M:%SZ)"
          else printf '%s\tevents.jsonl ABSENT\n' "$s"; fi
      done; } >"$ACAP/fixture-mtimes.txt"
    led fixture-mtimes "artifact_sha256=$(sha "$ACAP/fixture-mtimes.txt")"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=environment tab self-check failed"; return 1; }
    tmux_shim_equiv "$sock" "${RUNNING_SESSIONS[0]}" || { led HARNESS-ABORT "reason=tmux shim equivalence failed"; return 1; }
    { echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.before.txt" 2>&1
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"

    local B="$HARNESS_BASH" AE="$FROZEN_AE" f lbl i
    local -a FLAGS=("" "--running" "--all" "--stopped" "--needs-attn" "--needs-me" "--needs" "--attn" "--active" "--busy")
    for f in "${FLAGS[@]}"; do
        lbl="list${f:+_${f//-/}}"
        run_consumer "$lbl"        "$aehome" "$sock" -- "$B" "$AE" list ${f:+$f}
        run_consumer "${lbl}_json" "$aehome" "$sock" -- "$B" "$AE" list ${f:+$f} --json
        lbl="ls${f:+_${f//-/}}"
        run_consumer "$lbl"        "$aehome" "$sock" -- "$B" "$AE" ls ${f:+$f}
        run_consumer "${lbl}_json" "$aehome" "$sock" -- "$B" "$AE" ls ${f:+$f} --json
    done
    local -a INTER=("--needs-attn --all" "--all --needs-attn" "--active --all" "--all --active" "--needs-attn --stopped" "--active --stopped")
    for i in "${INTER[@]}"; do
        lbl="inter_${i//[- ]/}"
        # shellcheck disable=SC2086
        run_consumer "$lbl"        "$aehome" "$sock" -- "$B" "$AE" list $i
        # shellcheck disable=SC2086
        run_consumer "${lbl}_json" "$aehome" "$sock" -- "$B" "$AE" list $i --json
    done
    run_consumer "list_badflag" "$aehome" "$sock" -- "$B" "$AE" list --no-such-flag
    run_consumer "list_help"    "$aehome" "$sock" -- "$B" "$AE" list --help

    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    { echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.after.txt" 2>&1
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$ACAP/tmux.before.txt")" == "$(cat "$ACAP/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led manifest-diff "lines=$dl"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1
    pkill -x aefake >/dev/null 2>&1
    echo "  A2 case ($mode): manifest_diff_lines=$dl"
}
a2_case ro
a2_case rw
echo "A2 DONE"
