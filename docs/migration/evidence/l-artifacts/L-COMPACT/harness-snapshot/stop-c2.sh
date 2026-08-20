#!/opt/homebrew/bin/bash
# L-STOP chunk 2: identity-checks (SC-839a-d) and legacy-migration-injection (SC-839e).
set -uo pipefail
source /tmp/aelx/lib/stop-lib.sh

TOPOLOGY="proj + projx (prefix-sibling pair), one recorded server"

cell() { # <cell-id> <ids> <construction> <runner-fn>
    local cid="$1" ids="$2" con="$3" fn="$4"
    l_arm_begin L-STOP "identity-$cid" frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    : >"$R/cap/tmux-argv.log"
    "$fn"
    sleep 1
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    sarmtxt "identity-$cid" "$ids" "$con" \
      "cell	$cid" "op_rc	$(cat "$R/cap/2op.rc" 2>/dev/null || echo '<in-pane, see pane capture>')"
    l_arm_end
}

# C1 — outside tmux entirely
run_c1() { HOOKS=""; BLOCK=""; l_arm_env; l_ae 2op stop; }

# C2 — $TMUX present but the server does not answer for itself (stale/forged)
run_c2() {
    HOOKS=""; BLOCK=""
    l_arm_env "TMUX=${SOCK},999999,0" "TMUX_PANE=%0"
    l_ae 2op stop
    printf 'planted.TMUX\t%s,999999,0\nplanted.TMUX_PANE\t%%0\nnote\tthe socket is real, the server pid in $TMUX is not\n' "$SOCK" >"$R/cap/planted-env.txt"
}

# C3 — our ambient server is not the target's RECORDED server
run_c3() {
    local m="$R/h/.ae/sessions/proj/meta"
    cp -p "$m" "$R/cap/meta.before.txt"
    local m0; m0="$(stat -f '%Lp' "$m")"
    l_rewrite_preserving_mode "$m" "s|^tmux_server=.*\$|tmux_server=${R}/t/tmux-501/OTHERSERVER|"
    cp -p "$m" "$R/cap/meta.after.txt"
    diff -u "$R/cap/meta.before.txt" "$R/cap/meta.after.txt" >"$R/cap/mutation.diff" 2>&1
    { printf 'mutation\tthe target session meta tmux_server value is repointed at a different socket path on the same server directory\n'
      printf 'mode.before\t%s\nmode.after\t%s\n' "$m0" "$(stat -f '%Lp' "$m")"; } >"$R/cap/mutation.txt"
    local PANE; PANE="$(open_shell_pane proj)"; sleep 1
    printf 'shell_pane\t%s\n' "$PANE" >"$R/cap/shell-pane.txt"
    pane_send "$PANE" "$R/b/ae stop > $R/cap/2op.stdout 2> $R/cap/2op.stderr; echo \$? > $R/cap/2op.rc"; pane_enter "$PANE"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/2op.rc" ]] && break; sleep 0.1; i=$((i+1)); done
    (( i >= 300 )) && printf 'INCONCLUSIVE: no exit status observed within the 30s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    pane_cap "$PANE" >"$R/cap/pane.after.txt" 2>&1
}

# C4 — our pane resolves to a session ae has no directory for
run_c4() {
    /opt/homebrew/bin/tmux -S "$SOCK" new-session -d -s plainsess -c "$R/w" 2>/dev/null
    sleep 1
    local PANE; PANE="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -t plainsess -F '#{pane_id}' | head -1)"
    { printf 'plain_tmux_session\tplainsess (created directly by the controller; ae has no session directory for it)\n'
      printf 'shell_pane\t%s\n' "$PANE"; } >"$R/cap/shell-pane.txt"
    pane_send "$PANE" "$R/b/ae stop > $R/cap/2op.stdout 2> $R/cap/2op.stderr; echo \$? > $R/cap/2op.rc"; pane_enter "$PANE"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/2op.rc" ]] && break; sleep 0.1; i=$((i+1)); done
    (( i >= 300 )) && printf 'INCONCLUSIVE: no exit status observed within the 30s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    pane_cap "$PANE" >"$R/cap/pane.after.txt" 2>&1
}

# C5 — a tmux run-shell child: no controlling terminal
run_c5() {
    local PANE; PANE="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t '=proj' -F '#{pane_id}' | head -1)"
    printf 'target_pane\t%s\n' "$PANE" >"$R/cap/shell-pane.txt"
    /opt/homebrew/bin/tmux -S "$SOCK" run-shell -t "$PANE" \
      "$R/b/ae stop --pane=#{pane_id} > $R/cap/2op.stdout 2> $R/cap/2op.stderr; echo \$? > $R/cap/2op.rc"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/2op.rc" ]] && break; sleep 0.1; i=$((i+1)); done
    (( i >= 300 )) && printf 'INCONCLUSIVE: no exit status observed within the 30s bound\n' >"$R/cap/INCONCLUSIVE.txt"
}

# C5 + --self (the ONE fact the flag bypasses)
run_c5_self() {
    local PANE; PANE="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t '=proj' -F '#{pane_id}' | head -1)"
    printf 'target_pane\t%s\n' "$PANE" >"$R/cap/shell-pane.txt"
    /opt/homebrew/bin/tmux -S "$SOCK" run-shell -t "$PANE" \
      "$R/b/ae stop --self -y --pane=#{pane_id} > $R/cap/2op.stdout 2> $R/cap/2op.stderr; echo \$? > $R/cap/2op.rc"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/2op.rc" ]] && break; sleep 0.1; i=$((i+1)); done
    (( i >= 300 )) && printf 'INCONCLUSIVE: no exit status observed within the 30s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    sleep 3
}

# malformed --pane token
run_badpane() { HOOKS=""; BLOCK=""; l_arm_env; l_ae 2op stop --pane=notapane; }

#cell c1-outside-tmux            "SC-839a SC-839b SC-839c SC-839d" "the implicit no-target stop runs from a plain process with no \$TMUX and no \$TMUX_PANE" run_c1
#cell c2-foreign-server          "SC-839a SC-839b SC-839c SC-839d" "the implicit no-target stop runs with a planted \$TMUX naming the real socket and a server pid that is not the running one" run_c2
#cell c3-wrong-recorded-server   "SC-839a SC-839b SC-839c SC-839d" "the target session meta's tmux_server is repointed at another socket path (mode preserved), then the implicit no-target stop runs from a genuine shell pane inside that session" run_c3
#cell c4-pane-in-other-session   "SC-839a SC-839b SC-839c SC-839d" "the implicit no-target stop runs from a pane of a plain tmux session the controller created directly, which ae has no session directory for" run_c4
cell c5-no-controlling-tty      "SC-839a SC-839b SC-839c SC-839d" "the stop runs as a tmux run-shell child of the target's own pane, passing --pane=#{pane_id}; a run-shell child has no controlling terminal" run_c5
cell c5-self-flag               "SC-839a SC-839b SC-839c SC-839d" "the same run-shell construction as c5, with --self -y added — the flag bypasses exactly one proof mechanism" run_c5_self
#cell malformed-pane-token       "SC-839a SC-839b SC-839c SC-839d" "ae stop --pane=notapane — a --pane token that does not match the tmux pane-id shape" run_badpane
echo DONE
