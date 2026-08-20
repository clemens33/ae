#!/opt/homebrew/bin/bash
# L-STOP chunk 1: plain-stop (SC-835a/b/d), unverifiable-kill (SC-835c),
# self-stop (SC-835e/f/g/h).
set -uo pipefail
source /tmp/aelx/lib/stop-lib.sh

TOPOLOGY="proj + projx (prefix-sibling pair) + other, one recorded server"

arm_plain_stop() {
    l_arm_begin L-STOP plain-stop frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    : >"$R/cap/tmux-argv.log"     # zero the trace so it covers the op only
    l_ae 2op stop -y proj
    sleep 2
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    sarmtxt plain-stop "SC-835a SC-835b SC-835d" \
      "three real --local sessions on one recorded server, including the prefix-sibling pair proj/projx; ae stop -y proj runs with the delegate-and-log tmux shim tracing every tmux argv" \
      "op	ae stop -y proj" "op_rc	$(cat "$R/cap/2op.rc")" \
      "trace	tmux-argv.op.log holds every delegated command-tmux invocation made during the op, zeroed immediately before it"
    l_arm_end
}

arm_unverifiable_kill() {
    l_arm_begin L-STOP unverifiable-kill frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local SRVPID; SRVPID="$(/opt/homebrew/bin/tmux -S "$SOCK" display-message -p '#{pid}' 2>/dev/null)"
    ssnap 1pre
    l_manifest "$(dirname "$SOCK")" "$R/cap/socketdir.before.tsv"
    { printf 'manipulation\tthe directory holding the recorded tmux socket is removed; the server process is left running\n'
      printf 'recorded_socket\t%s\n' "$(grep '^tmux_server=' "$R/h/.ae/sessions/proj/meta" | cut -d= -f2-)"
      printf 'server_pid\t%s\n' "${SRVPID:-<none>}"; } >"$R/cap/manipulation.txt"
    rm -rf "$(dirname "$SOCK")"
    l_manifest "$(dirname "$SOCK")" "$R/cap/socketdir.after.tsv"
    : >"$R/cap/tmux-argv.log"
    l_ae 2op stop -y proj
    l_ae 3opall stop all -y
    sleep 2
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    printf 'server_alive_after\t%s\n' "$(kill -0 "${SRVPID:-0}" 2>/dev/null && echo yes || echo no)" >>"$R/cap/manipulation.txt"
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    ps -ax -o pid=,ppid=,tty=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/3post.ps.txt" 2>&1
    sarmtxt unverifiable-kill SC-835c \
      "the directory holding the recorded tmux socket is removed while the server process keeps running, then a singular stop and a fleet stop each run against it" \
      "op1	ae stop -y proj" "op1_rc	$(cat "$R/cap/2op.rc")" \
      "op2	ae stop all -y" "op2_rc	$(cat "$R/cap/3opall.rc")" \
      "server_pid	${SRVPID:-<none>}"
    [[ -n "${SRVPID:-}" ]] && kill -9 "$SRVPID" 2>/dev/null
    l_arm_end
}

arm_self_stop() { # <with-y|without-y>
    local mode="$1"
    l_arm_begin L-STOP "self-stop-$mode" frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local PANE; PANE="$(open_shell_pane proj)"
    [[ -n "$PANE" ]] || { printf 'FIXTURE-INVALID: no shell pane\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    sleep 1
    { printf 'shell_pane\t%s\n' "$PANE"
      /opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{session_name}|#{pane_id}|#{pane_tty}|#{pane_current_command}'; } >"$R/cap/shell-pane.txt" 2>&1
    ssnap 1pre
    : >"$R/cap/tmux-argv.log"
    local cmd="$R/b/ae stop"
    [[ "$mode" == with-y ]] && cmd="$R/b/ae stop -y"
    pane_send "$PANE" "$cmd"; pane_enter "$PANE"
    printf 'typed\t%s\n' "$cmd" >"$R/cap/typed.txt"
    if [[ "$mode" == without-y ]]; then
        if pane_wait "$PANE" 30 "Continue?"; then
            pane_cap "$PANE" >"$R/cap/pty.at-prompt.txt"
            pane_send "$PANE" "y"; pane_enter "$PANE"
        else
            pane_cap "$PANE" >"$R/cap/pty.prompt-not-observed.txt"
            printf 'INCONCLUSIVE: the confirmation prompt was not observed within the 30s bound\n' >"$R/cap/INCONCLUSIVE.txt"
        fi
    fi
    # bounded positive barrier: the detached supervisor appears in the process table
    local i=0 seen=0
    while (( i < 300 )); do
        if ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -q '_stop-supervisor'; then seen=1; break; fi
        sleep 0.1; i=$((i+1))
    done
    ps -ax -o pid=,ppid=,tty=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/supervisor.ps-lineage.txt" 2>&1
    (( seen == 0 )) && printf 'INCONCLUSIVE: no _stop-supervisor process observed within the 30s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    pane_cap "$PANE" >"$R/cap/pty.after-answer.txt" 2>&1 || printf '(pane gone)\n' >"$R/cap/pty.after-answer.txt"
    sleep 5
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt "self-stop-$mode" "SC-835e SC-835f SC-835g SC-835h" \
      "a SHELL pane is opened inside the live target session and the controller types the implicit no-target stop into it ($cmd); the pty transcript, a ps lineage of the detached supervisor and the events deltas are captured" \
      "mode	$mode" "typed	$cmd" "shell_pane	$PANE" \
      "supervisor_observed	$( ((seen)) && echo yes || echo no )" \
      "prompt_bound_sec	30" "supervisor_bound_sec	30"
    l_arm_end
}

arm_plain_stop
arm_unverifiable_kill
arm_self_stop without-y
arm_self_stop with-y
echo DONE
