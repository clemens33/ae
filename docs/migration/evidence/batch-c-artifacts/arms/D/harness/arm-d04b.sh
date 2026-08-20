#!/opt/homebrew/bin/bash
# D04b / SC-1306c — next selection/recheck cut. b0-design.md Design 6.
# Two hooks in the one patch: H_NEXT_SELECTED (after best-candidate resolution, before the
# exact recheck) and H_NEXT_RECHECKED (after the successful exact list-sessions|grep -Fx,
# before the final focus call).
# Attach arms run `next --attach` from a pane INSIDE an attached client on the dedicated
# isolated server: the harness attaches a scripted, pty-wrapped client to a CALLER session
# first, so the intended final verb on this path is switch-client. Every controller kill is
# issued from a SEPARATE controller connection, never from inside the client under test.
source "$(dirname "$0")/dlib.sh"
ARMG=D
mkdir -p "$ADEST/$ARMG"
[[ -f "$ADEST/$ARMG/ledger.tsv" ]] || printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
NEXT_SESSIONS=(tg2wu)

clients_snap() { # <sock> <label>
    { echo "## clients (client -> session mapping)"
      command tmux -S "$1" list-clients -F '#{client_name}|#{client_tty}|#{client_session}|#{pane_id}|#{client_activity}' 2>&1
      echo "## sessions"; command tmux -S "$1" list-sessions -F '#{session_name}|#{session_attached}' 2>&1
    } >"$ACAP/clients.$2.txt"
    led clients-snapshot "label=$2" "artifact_sha256=$(sha "$ACAP/clients.$2.txt")"
}

d04b_case() { # <case-id> <hook> <kill-when: at-hook|after-recheck|none> <sibling: yes|no>
    local cid="$1" hook="$2" killwhen="$3" sibling="$4"
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "attach"
    led rows "rows=D04b,SC-1306c" "design=b0-design.md Design 6" "hook=$hook" "kill_when=$killwhen" "prefix_sibling=$sibling"
    local aehome="$base/home/.ae"
    t_clone A2 composite "$aehome" rw || { led CLONE-FAILED; return 1; }
    local cf exp
    cf="$(dir_fingerprint "$aehome")"; exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    build_live_topology "$aehome" "$sock" "${NEXT_SESSIONS[@]}"
    local target="${NEXT_SESSIONS[0]}" sib=""
    if [[ "$sibling" == yes ]]; then
        sib="${target}extra"
        command tmux -S "$sock" new-session -d -s "$sib" "$FAKE_BIN"
        led prefix-sibling-created "sibling=$sib" "note=name EXTENDS the target's, so a prefix match would capture it"
    fi
    # the CALLER session the scripted client attaches to — never the target
    command tmux -S "$sock" new-session -d -s caller
    local cpane=""
    local _t; _t=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - _t < 20 )); do
        cpane="$(command tmux -S "$sock" list-panes -t caller -F '#{pane_id}' 2>/dev/null | head -1)"
        [[ -n "$cpane" ]] && break
        sleep 0.5
    done
    [[ -n "$cpane" ]] || { led HARNESS-ABORT "reason=caller pane never appeared"; t_teardown; return 1; }
    { echo "arm=$ARMG case=$cid design=b0-design.md Design 6 (D04b) rows=D04b,SC-1306c"
      echo "hook=$hook kill_when=$killwhen prefix_sibling=$sibling"
      echo "target_session=$target sibling=${sib:-<none>} caller_session=caller caller_pane=$cpane"
      echo "attach_client=pty-wrapped script(1) running tmux -S <sock> attach -t caller"
      echo "controller_connection=a SEPARATE tmux -S <sock> invocation, never from inside the client under test"
      echo "unmodified_ae_sha256=$(sha "$FROZEN_AE") hooked_ae_sha256=$(sha "$HOOKED_AE") patch_sha256=$(sha "$HOOK_PATCH")"
      echo "tmux_socket=$sock"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    cp "$HOOK_PATCH" "$ACAP/hook.patch"; led hook-patch-published "artifact_sha256=$(sha "$ACAP/hook.patch")"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$sock" "$target" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }
    hook_inactive_equiv "$aehome" "$sock" "$target" || { led HARNESS-ABORT "reason=inactive-hook equivalence"; return 1; }
    # attach a scripted client to the CALLER session on its own pty
    # script(1) cannot be used: it calls tcgetattr on its own stdin and this harness has no
    # controlling terminal ("Operation not supported on socket"). pty-attach.py forks a real
    # pty, gives the child the pty as its controlling terminal, sizes the window so tmux will
    # attach, and drains the master side to a log.
    python3 "$SCRATCH/harness/pty-attach.py" "$ACAP/scripted-client.log" \
        /opt/homebrew/bin/tmux -S "$sock" attach -t caller &
    local clpid=$! t0=0
    t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 20 )); do
        [[ -n "$(command tmux -S "$sock" list-clients -F '#{client_name}' 2>/dev/null)" ]] && break
        sleep 0.5
    done
    led scripted-client-attached "clients=$(command tmux -S "$sock" list-clients -F '#{client_name}' 2>/dev/null | tr '\n' ' ')" \
        "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
    clients_snap "$sock" before
    d_snapshot before "$aehome" "$sock"
    local hd="$ARM_TMUXTMP/hookdir"; rm -rf "$hd"; mkdir -p "$hd"
    local runsh="$base/run-next.sh"
    cat >"$runsh" <<RUN
#!/opt/homebrew/bin/bash
export HOME="$base/home" AE_HOME="$aehome" TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
export PATH=/tmp/aecx/shim-tmux:/tmp/aecx/shim-flock:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export AE_TMUX_SERVER="$sock" AE_TMUX_SERVER_KIND=socket
export AE_REAL_TMUX=/opt/homebrew/bin/tmux AE_REAL_FLOCK=/opt/homebrew/bin/flock
export AE_TMUX_SHIM_LOG="$ACAP/out/attach-next.tmuxtrace" AE_FLOCK_SPY_LOG="$ACAP/out/attach-next.flockspy"
export AE_HOOK="$hook" AE_HOOK_DIR="$hd"
"$HARNESS_BASH" "$HOOKED_AE" next --attach >"$ACAP/out/attach-next.stdout" 2>"$ACAP/out/attach-next.stderr"
echo \$? >"$hd/rc"
RUN
    chmod +x "$runsh"
    : >"$ACAP/out/attach-next.tmuxtrace"; : >"$ACAP/out/attach-next.flockspy"
    led barrier-ARMED "hook=$hook" "invoked_from=caller pane $cpane inside the attached client"
    command tmux -S "$sock" send-keys -t "$cpane" "$runsh" Enter
    led send-keys-issued "pane=$cpane" "rc=$?" "script=$runsh"
    local reached=0; t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 90 )); do
        [[ -e "$hd/$hook.reached" ]] && { reached=1; break; }
        [[ -e "$hd/rc" ]] && break
        sleep 0.2
    done
    led barrier-REACHED "hook=$hook" "reached=$reached" "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
    if (( reached == 1 )); then
        { echo "## sessions at the barrier, before any controller action"
          command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_attached}' 2>&1; } >"$ACAP/tmux.at-barrier-before.txt"
        if [[ "$killwhen" != none ]]; then
            { echo "mutation=kill the EXACT target session ($target) from a SEPARATE controller connection"
              echo "kill_when=$killwhen (the reader is blocked at $hook)"
              echo "sibling_present=${sib:-<none>}"
              echo "utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/controller-mutation.txt"
            command tmux -S "$sock" kill-session -t "$target" 2>&1 | sed 's/^/killrc: /' >>"$ACAP/controller-mutation.txt"
            { echo "## sessions immediately after the kill"
              command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_attached}' 2>&1; } >>"$ACAP/controller-mutation.txt"
            led controller-mutation "target=$target" "when=$killwhen" "artifact_sha256=$(sha "$ACAP/controller-mutation.txt")"
        else
            # A barrier case must record what the controller did even when the answer is
            # NOTHING — an absent record and a record of no action are different claims.
            { echo "mutation=NONE — this arm releases the barrier without killing anything"
              echo "purpose=the no-kill companion: the same hook, the same caller topology,"
              echo "  the same release, with the controller deliberately inert"
              echo "utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
              echo "## sessions at release (unchanged by the controller)"
              command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_attached}' 2>&1
            } >"$ACAP/controller-mutation.txt"
            led controller-mutation "none=this arm releases without killing anything" \
                "artifact_sha256=$(sha "$ACAP/controller-mutation.txt")"
        fi
        : >"$hd/$hook.release"
        led barrier-RELEASED "hook=$hook"
    else
        led OUTCOME-INCONCLUSIVE "reason=hook $hook not reached within 90s"
    fi
    t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 60 )); do [[ -e "$hd/rc" ]] && break; sleep 0.5; done
    local rc; rc="$(cat "$hd/rc" 2>/dev/null || echo '?')"
    cp "$hd/hook.log" "$ACAP/hook.log" 2>/dev/null
    command tmux -S "$sock" capture-pane -p -J -S - -E - -t "$cpane" >"$ACAP/caller-pane.after.txt" 2>&1
    clients_snap "$sock" after
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "attach-next" "$rc" \
        "$(sha "$ACAP/out/attach-next.stdout")" "$(stat -f %z "$ACAP/out/attach-next.stdout" 2>/dev/null || echo 0)" \
        "$(sha "$ACAP/out/attach-next.stderr")" "$(stat -f %z "$ACAP/out/attach-next.stderr" 2>/dev/null || echo 0)" \
        "$(sha "$ACAP/out/attach-next.tmuxtrace")" "$(wc -l <"$ACAP/out/attach-next.tmuxtrace" | tr -d ' ')" \
        "hooked:$hook" "ae next --attach (from the caller pane inside the attached client)" >>"$ACAP/consumers.tsv"
    led attach-consumer-COMPLETE "rc=$rc" "stdout_sha256=$(sha "$ACAP/out/attach-next.stdout")" \
        "hook_log_sha256=$(sha "$ACAP/hook.log")" \
        "client_mapping_changed=$( [[ "$(cat "$ACAP/clients.before.txt")" == "$(cat "$ACAP/clients.after.txt")" ]] && echo no || echo yes )"
    d_snapshot after "$aehome" "$sock"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    { echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
      echo "client_mapping_changed=$( [[ "$(cat "$ACAP/clients.before.txt")" == "$(cat "$ACAP/clients.after.txt")" ]] && echo no || echo yes )"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    kill "$clpid" 2>/dev/null; command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid done (rc=$rc)"
}
reg() { printf '%s\tD04b,SC-1306c\tA2\tcomposite\n' "$1" >>"$ADEST/$ARMG/ledger.tsv"; }
reg d04b-arm1-kill-at-selected;        d04b_case d04b-arm1-kill-at-selected        H_NEXT_SELECTED  at-hook        no
reg d04b-arm2-prefix-kill-after-recheck; d04b_case d04b-arm2-prefix-kill-after-recheck H_NEXT_RECHECKED after-recheck yes
reg d04b-arm3-nosibling-kill-after-recheck; d04b_case d04b-arm3-nosibling-kill-after-recheck H_NEXT_RECHECKED after-recheck no
reg d04b-twin-no-kill;                 d04b_case d04b-twin-no-kill                 H_NEXT_SELECTED  none           no
echo "D04b DONE"
