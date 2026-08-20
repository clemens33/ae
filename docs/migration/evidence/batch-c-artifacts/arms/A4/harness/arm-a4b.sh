#!/opt/homebrew/bin/bash
# ARM GROUP A4, part 2 — next rows (SC-019, SC-020a-c) including 020b's named barrier
# on D04b's approved hook (b0-design.md Design 6: H_NEXT_SELECTED).
source "$(dirname "$0")/armlib.sh"
ARMG=A4
lg() { printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"; }
clients_snap() {
    { echo "## clients"; command tmux -S "$1" list-clients -F '#{client_name}|#{client_tty}|#{client_session}|#{pane_id}|#{client_activity}' 2>&1
      echo "## sessions"; command tmux -S "$1" list-sessions -F '#{session_name}|#{session_attached}' 2>&1
    } >"$ACAP/clients.$2.txt"
    led clients-snapshot "label=$2" "artifact_sha256=$(sha "$ACAP/clients.$2.txt")"
}

# ONE attention candidate by construction, so the session `next` resolves to is known
# without asking the product first: only tg2wu is given a live tmux session.
NEXT_SESSIONS=(tg2wu)

setup_next_case() { # <case-id> <rows> <mode>
    local cid="$1" rows="$2" mode="$3"
    NBASE="$AROOT/$ARMG/$cid-$mode"
    [[ -e "$NBASE" ]] && chmod -R u+w "$NBASE" 2>/dev/null; rm -rf "$NBASE"; mkdir -p "$NBASE/home"
    export ARM_TMUXTMP="$NBASE/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$mode"
    led rows "rows=$rows" "template=A2/composite"
    NAEHOME="$NBASE/home/.ae"
    t_clone A2 composite "$NAEHOME" "$mode" || { led CLONE-FAILED; return 1; }
    local cf exp
    cf="$(dir_fingerprint "$NAEHOME")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/A2/_meta/composite.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    NSOCK="$NBASE/live.sock"
    build_live_topology "$NAEHOME" "$NSOCK" "${NEXT_SESSIONS[@]}"
    { echo "arm=$ARMG case=$cid rows=$rows template=A2/composite clone_mode=$mode"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "attention_candidates_by_construction=${NEXT_SESSIONS[*]} (the only session given a live tmux session, so next's resolution is known without asking the product first)"
      echo "tmux_socket=$NSOCK"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$NAEHOME" "$NSOCK"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$NSOCK" "${NEXT_SESSIONS[0]}" || { led HARNESS-ABORT "reason=shim equivalence"; return 1; }
    dir_manifest "$NAEHOME" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    { echo "## panes"; command tmux -S "$NSOCK" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$NSOCK" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.before.txt" 2>&1
    clients_snap "$NSOCK" before
    return 0
}
close_next_case() {
    dir_manifest "$NAEHOME" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    { echo "## panes"; command tmux -S "$NSOCK" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$NSOCK" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.after.txt" 2>&1
    clients_snap "$NSOCK" after
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$ACAP/tmux.before.txt")" == "$(cat "$ACAP/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "client_snapshot_identical=$( [[ "$(cat "$ACAP/clients.before.txt")" == "$(cat "$ACAP/clients.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led manifest-diff "lines=$dl"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$NSOCK" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $ACASE-$AMODE: manifest_diff_lines=$dl"
}

########## c03 — next, no attach ##########
lg c03-next-noattach "SC-019,SC-020a,SC-020c" A2 composite
for m in ro rw; do
    setup_next_case c03-next-noattach "SC-019,SC-020a,SC-020c" "$m" || continue
    run_consumer "next"        "$NAEHOME" "$NSOCK" -- "$HARNESS_BASH" "$FROZEN_AE" next
    run_consumer "next-again"  "$NAEHOME" "$NSOCK" -- "$HARNESS_BASH" "$FROZEN_AE" next
    run_consumer "list-needsattn-json" "$NAEHOME" "$NSOCK" -- "$HARNESS_BASH" "$FROZEN_AE" list --needs-attn --json
    close_next_case
done

########## c04 — next --attach from OUTSIDE any client ##########
lg c04-next-attach-outside "SC-020a,SC-020c" A2 composite
for m in ro rw; do
    setup_next_case c04-next-attach-outside "SC-020a,SC-020c" "$m" || continue
    echo "note=the frozen outside-tmux verb is a BLOCKING attach-session, so this invocation is harness-bounded and the bound is recorded" >>"$ACAP/case.txt"
    run_consumer "next-attach" "$NAEHOME" "$NSOCK" --bounded 8 -- "$HARNESS_BASH" "$FROZEN_AE" next --attach
    close_next_case
done

########## c05 — SC-020b named barrier on D04b's approved hook ##########
lg c05-020b-barrier "SC-020b" A2 composite
for m in rw; do
    setup_next_case c05-020b-barrier "SC-020b" "$m" || continue
    { echo "hook_design=b0-design.md Design 6 (D04b), H_NEXT_SELECTED — after best-candidate"
      echo "  resolution, BEFORE the exact recheck. The design self-declares that SC-020b's"
      echo "  Batch C arm consumes it, which is batch-c-design.md's reuse condition."
      echo "unmodified_ae_sha256=$(sha "$FROZEN_AE")"
      echo "hooked_ae_sha256=$(sha "$HOOKED_AE")"
      echo "hook_patch_sha256=$(sha "$HOOK_PATCH") (22 added lines, 0 removed, 0 modified)"
    } >>"$ACAP/case.txt"
    cp "$HOOK_PATCH" "$ACAP/hook.patch"
    led hook-patch-published "artifact_sha256=$(sha "$ACAP/hook.patch")"
    hook_inactive_equiv "$NAEHOME" "$NSOCK" "${NEXT_SESSIONS[0]}" || {
        led HARNESS-ABORT "reason=inactive-hook equivalence failed; no hooked capture taken"
        echo "  HARNESS-ABORT c05: inactive-hook equivalence"; close_next_case; continue; }
    HD="$NBASE/hookdir"; rm -rf "$HD"; mkdir -p "$HD"
    led barrier-ARMED "hook=H_NEXT_SELECTED" "hook_dir=$HD"
    ( env -i "HOME=$NBASE/home" "AE_HOME=$NAEHOME" \
        "PATH=/tmp/aecx/shim-tmux:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
        "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color" \
        "TMUX_TMPDIR=$ARM_TMUXTMP" "AE_TMUX_SERVER=$NSOCK" "AE_TMUX_SERVER_KIND=socket" \
        "AE_TMUX_SHIM_LOG=$ACAP/out/barrier-next.tmuxtrace" "AE_REAL_TMUX=/opt/homebrew/bin/tmux" \
        "AE_HOOK=H_NEXT_SELECTED" "AE_HOOK_DIR=$HD" \
        "$HARNESS_BASH" "$HOOKED_AE" next </dev/null \
        >"$ACAP/out/barrier-next.stdout" 2>"$ACAP/out/barrier-next.stderr" ; echo $? >"$HD/rc" ) &
    BPID=$!
    _t0=$(/bin/date -u +%s); REACHED=0
    while (( $(/bin/date -u +%s) - _t0 < 60 )); do
        [[ -e "$HD/H_NEXT_SELECTED.reached" ]] && { REACHED=1; break; }
        sleep 0.2
    done
    led barrier-REACHED "reached=$REACHED" "waited_s=$(( $(/bin/date -u +%s) - _t0 ))"
    if (( REACHED == 0 )); then
        echo "OUTCOME=INCONCLUSIVE reason=hook barrier not reached within 60s" >>"$ACAP/case.txt"
        kill "$BPID" 2>/dev/null; wait "$BPID" 2>/dev/null
        close_next_case; continue
    fi
    { echo "## tmux at the barrier, before the controller mutation"
      command tmux -S "$NSOCK" list-sessions -F '#{session_name}|#{session_windows}' 2>&1
    } >"$ACAP/tmux.at-barrier-before.txt"
    # THE CONTROLLER performs the named mutation — the hook only blocks.
    command tmux -S "$NSOCK" kill-session -t "${NEXT_SESSIONS[0]}" 2>&1 | sed 's/^/killrc: /' >"$ACAP/controller-mutation.txt"
    { echo "mutation=kill the EXACT session next had already resolved to (${NEXT_SESSIONS[0]})"
      echo "issued_from=a separate controller connection, never from inside the process under test"
      echo "utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
      echo "## tmux immediately after the mutation"
      command tmux -S "$NSOCK" list-sessions -F '#{session_name}|#{session_windows}' 2>&1
    } >>"$ACAP/controller-mutation.txt"
    led controller-mutation "target=${NEXT_SESSIONS[0]}" "artifact_sha256=$(sha "$ACAP/controller-mutation.txt")"
    : >"$HD/H_NEXT_SELECTED.release"
    led barrier-RELEASED
    wait "$BPID" 2>/dev/null
    RC="$(cat "$HD/rc" 2>/dev/null || echo '?')"
    cp "$HD/hook.log" "$ACAP/hook.log" 2>/dev/null
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "barrier-next" "$RC" \
        "$(sha "$ACAP/out/barrier-next.stdout")" "$(stat -f %z "$ACAP/out/barrier-next.stdout")" \
        "$(sha "$ACAP/out/barrier-next.stderr")" "$(stat -f %z "$ACAP/out/barrier-next.stderr")" \
        "$(sha "$ACAP/out/barrier-next.tmuxtrace")" "$(wc -l <"$ACAP/out/barrier-next.tmuxtrace" | tr -d ' ')" \
        "hooked" "AE_HOOK=H_NEXT_SELECTED $HOOKED_AE next" >>"$ACAP/consumers.tsv"
    led barrier-consumer-COMPLETE "label=barrier-next" "rc=$RC" \
        "stdout_sha256=$(sha "$ACAP/out/barrier-next.stdout")" \
        "stderr_sha256=$(sha "$ACAP/out/barrier-next.stderr")" \
        "hook_log_sha256=$(sha "$ACAP/hook.log")"
    close_next_case
done
echo "A4 PART 2 DONE"
