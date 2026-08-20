#!/opt/homebrew/bin/bash
# ARM GROUP A4 — status / next. Rows: SC-016a-d, SC-513a-c, SC-019, SC-020a-c.
# Live tmux on dedicated servers; never-attaches proven by client-list snapshots.
source "$(dirname "$0")/armlib.sh"
ARMG=A4
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
lg() { printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"; }

clients_snap() { # <sock> <label>
    { echo "## clients"; command tmux -S "$1" list-clients -F '#{client_name}|#{client_tty}|#{client_session}|#{pane_id}|#{client_activity}' 2>&1
      echo "## sessions"; command tmux -S "$1" list-sessions -F '#{session_name}|#{session_attached}' 2>&1
    } >"$ACAP/clients.$2.txt"
    led clients-snapshot "label=$2" "artifact_sha256=$(sha "$ACAP/clients.$2.txt")"
}

########################################################################
# c01/c02 — live launched session (status rows)
########################################################################
live_status_case() { # <case-id> <rows> <fill-lines>
    local cid="$1" rows="$2" fill="$3"
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "live"
    led rows "rows=$rows" "template=none (live 2-agent launch)"
    t_sandbox "a4$cid" "fake:worker"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    t_launch "ta4${cid//-/}" || { led LAUNCH-FAILED; return 1; }
    local LEAD WORK
    LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
    { echo "arm=$ARMG case=$cid rows=$rows clone_mode=live session=$TSESSION socket=$SOCK"
      echo "lead_pane=$LEAD worker_pane=$WORK"
      echo "fake_agent_sha256=$(sha "$FAKE_BIN")"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$AE_HOME" "$SOCK"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; t_teardown; return 1; }
    tmux_shim_equiv "$SOCK" "$TSESSION" || { led HARNESS-ABORT "reason=shim equivalence"; t_teardown; return 1; }
    if (( fill > 0 )); then
        # SC-016b: each of >=2 panes filled with >80 UNIQUELY NUMBERED lines, via each
        # pane's own control FIFO so the two streams cannot be confused for one another.
        local p n
        for p in "$LEAD" "$WORK"; do
            local fifo="$ROOT/ctl/${p}.ctl"
            local _t0; _t0=$(/bin/date -u +%s)
            while [[ ! -p "$fifo" ]] && (( $(/bin/date -u +%s) - _t0 < 20 )); do sleep 0.5; done
            [[ -p "$fifo" ]] || { led FILL-FIFO-MISSING "pane=$p" "fifo=$fifo"; continue; }
            for ((n=1; n<=fill; n++)); do printf 'PANE%s-LINE-%04d\n' "${p#%}" "$n" >"$fifo"; done
            led pane-filled "pane=$p" "lines=$fill" "marker=PANE${p#%}-LINE-NNNN"
        done
        sleep 2
        local q
        for q in "$LEAD" "$WORK"; do
            tm capture-pane -p -J -S - -E - -t "$q" >"$ACAP/panefull.${q#%}.txt" 2>&1
            { echo "pane=$q"
              echo "captured_lines=$(wc -l <"$ACAP/panefull.${q#%}.txt" | tr -d ' ')"
              echo "unique_markers=$(grep -c "PANE${q#%}-LINE-" "$ACAP/panefull.${q#%}.txt" || true)"
              echo "first_marker=$(grep -o "PANE${q#%}-LINE-[0-9]*" "$ACAP/panefull.${q#%}.txt" | head -1)"
              echo "last_marker=$(grep -o "PANE${q#%}-LINE-[0-9]*" "$ACAP/panefull.${q#%}.txt" | tail -1)"
            } >>"$ACAP/pane-fill-summary.txt"
        done
        led pane-fill-summary "artifact_sha256=$(sha "$ACAP/pane-fill-summary.txt")"
    fi
    { echo "## panes"; tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}|#{pane_pid}|#{pane_height}x#{pane_width}'
      echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.before.txt" 2>&1
    dir_manifest "$AE_HOME" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    clients_snap "$SOCK" before
    local B="$HARNESS_BASH" AE="$FROZEN_AE"
    run_consumer "status"             "$AE_HOME" "$SOCK" -- "$B" "$AE" status "$TSESSION"
    run_consumer "status-noarg"       "$AE_HOME" "$SOCK" -- "$B" "$AE" status
    run_consumer "status-missing"     "$AE_HOME" "$SOCK" -- "$B" "$AE" status no-such-session
    run_consumer "list"               "$AE_HOME" "$SOCK" -- "$B" "$AE" list
    run_consumer "list-json"          "$AE_HOME" "$SOCK" -- "$B" "$AE" list --json
    run_consumer "next"               "$AE_HOME" "$SOCK" -- "$B" "$AE" next
    run_consumer "agents"             "$AE_HOME" "$SOCK" -- "$META/agents"
    clients_snap "$SOCK" after
    dir_manifest "$AE_HOME" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    { echo "## panes"; tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}|#{pane_pid}|#{pane_height}x#{pane_width}'
      echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.after.txt" 2>&1
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$ACAP/tmux.before.txt")" == "$(cat "$ACAP/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "client_snapshot_identical=$( [[ "$(cat "$ACAP/clients.before.txt")" == "$(cat "$ACAP/clients.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led manifest-diff "lines=$dl"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    t_teardown
    echo "  $cid: manifest_diff_lines=$dl"
}
lg c01-status-live "SC-016a,SC-016c,SC-016d,SC-019,SC-513a,SC-513b,SC-513c" live "none (live 2-agent launch)"
live_status_case c01-status-live "SC-016a,SC-016c,SC-016d,SC-019,SC-513a,SC-513b,SC-513c" 0
lg c02-status-016b "SC-016b" live "none (live 2-agent launch, 150 uniquely numbered lines per pane)"
live_status_case c02-status-016b "SC-016b" 150
echo "A4 PART 1 DONE"
