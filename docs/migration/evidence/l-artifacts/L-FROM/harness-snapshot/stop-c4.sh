#!/opt/homebrew/bin/bash
# L-STOP chunk 4: fleet (SC-815a-d) and exit-folding (SC-515a-c).
# Runs under the v2 instrumented copy where a stop barrier is needed.
set -uo pipefail
source /tmp/aelx/lib/stop-lib.sh

FIX=/tmp/aelx/fixtures
TOPOLOGY="proj + projx (prefix-sibling pair) + other, one recorded server"

opid_from_ps() {
    ps -ax -o command= | grep -F "$R" | grep -o '_stop-fleet-supervisor [0-9a-f-]\{36\}' | awk '{print $2}' | head -1
}
supervisor_pids() {
    ps -ax -o pid=,command= | grep -F "$R" | grep '_stop-fleet-supervisor' | grep -v '[g]rep' | awk '{print $1}'
}
wait_barrier() { # <name-prefix> <timeout-sec>
    local n="$1" t="$2" i=0 f
    while (( i < t*10 )); do
        for f in "$R"/ctl/"$n".*.reached; do [[ -e "$f" ]] && { printf '%s' "$(basename "$f" .reached)"; return 0; }; done
        sleep 0.1; i=$((i+1))
    done
    return 1
}
release_all() { local f; for f in "$R"/ctl/*.reached; do [[ -e "$f" ]] || continue; : >"$R/ctl/$(basename "$f" .reached).release"; done; }

# Derive a stop-result line from a PRODUCER-HARVESTED one: substitute only the
# op id and the target name; record the byte diff.
plant_result() { # <session> <opid> <success|failure> <difflog>
    local sess="$1" opid="$2" kind="$3" dl="$4"
    local base="$FIX/stop-result.${kind}.harvested"
    local f="$R/h/.ae/sessions/$sess/events.jsonl"
    local before after
    before="$(cat "$base")"
    after="$(python3 - "$base" "$opid" "$sess" <<'PY'
import sys, re
line = open(sys.argv[1]).read().rstrip('\n')
line = re.sub(r'\[op [0-9a-f-]{36}\]', '[op %s]' % sys.argv[2], line)
line = re.sub(r'"target":"[^"]*"', '"target":"%s"' % sys.argv[3], line)
print(line)
PY
)"
    { printf '=== %s (%s) ===\n' "$sess" "$kind"
      printf -- '- %s\n' "$before"
      printf -- '+ %s\n' "$after"; } >>"$dl"
    local mode; mode="$(stat -f '%Lp' "$f" 2>/dev/null || echo 644)"
    printf '%s\n' "$after" >>"$f"
    chmod "0$mode" "$f"
    return 0
}

# ─────────────────────────────── SC-815a: a FOURTH session in the confirmation window
arm_fourth_in_window() {
    l_arm_begin L-STOP fleet-fourth-session-in-confirmation-window frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    HOOKS=""; BLOCK=""; l_arm_env
    : >"$R/cap/tmux-argv.log"
    l_pane_start 2op "$R/b/ae" stop all
    if l_pane_wait 60 "Continue?"; then
        l_pane_capture at-prompt
        # the confirmation window is open RIGHT NOW
        l_ae 1dfourth --local fourth
        sleep 2
        l_tmuxsnap "$SOCK" "$R/cap/2during-window.tmux.txt"
        l_manifest "$R/h/.ae/sessions" "$R/cap/2during-window.sessions.tsv"
        l_pane_send y; l_pane_enter
    else
        l_pane_capture prompt-not-observed
        printf 'INCONCLUSIVE: the confirmation prompt was not observed within the 60s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    fi
    l_pane_wait_rc 120 || printf 'INCONCLUSIVE: no exit status within the 120s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    sleep 6
    l_pane_capture final
    l_pane_stop
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt fleet-fourth-session-in-confirmation-window SC-815a \
      "ae stop all runs on a real terminal over three sessions; while the confirmation prompt is displayed the controller launches a FOURTH real session, then answers y" \
      "fourth_launch_rc	$(cat "$R/cap/1dfourth.rc" 2>/dev/null)" \
      "op_rc	$(cat "$R/cap/2op.rc" 2>/dev/null)" "prompt_bound_sec	60" "rc_bound_sec	120"
    l_arm_end
}

# ─────────────────────── SC-815b: a confirmed target ended and recreated mid-op
arm_name_handoff() {
    l_arm_begin L-STOP fleet-name-handoff-mid-op instrumented
    l_use_v2; PATCHV="L-HOOKS-v2"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    HOOKS=b_stop_supervisor_entry; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    : >"$R/cap/tmux-argv.log"
    l_ae_bg 2op stop all -y
    local k; k="$(wait_barrier b_stop_supervisor_entry 60)" || {
        printf 'INCONCLUSIVE: the supervisor-entry barrier was not reached within the 60s bound\n' >"$R/cap/INCONCLUSIVE.txt"; }
    printf 'barrier\t%s\n' "${k:-<none>}" >"$R/cap/barrier.txt"
    printf 'opid.from.ps\t%s\n' "$(opid_from_ps)" >>"$R/cap/barrier.txt"
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/at-barrier.ps.txt" 2>&1
    l_tmuxsnap "$SOCK" "$R/cap/at-barrier.tmux.txt"
    # the controller ends ONE confirmed target and recreates it under the SAME name
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2bend end -f other
    sleep 1
    l_ae 2crecreate --local other
    sleep 3
    l_tmuxsnap "$SOCK" "$R/cap/after-handoff.tmux.txt"
    l_manifest "$R/h/.ae/sessions" "$R/cap/after-handoff.sessions.tsv"
    { printf 'controller.action.1\tae end -f other  (rc %s)\n' "$(cat "$R/cap/2bend.rc")"
      printf 'controller.action.2\tae --local other (rc %s) — the SAME name, a different session id\n' "$(cat "$R/cap/2crecreate.rc")"
    } >"$R/cap/controller.txt"
    release_all
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 8
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt fleet-name-handoff-mid-op SC-815b \
      "ae stop all -y runs over three sessions; at the barrier where the detached fleet supervisor has validated its op id and not yet acted, the controller ENDS one confirmed target and RELAUNCHES it under the same name, then releases the supervisor" \
      "barrier	b_stop_supervisor_entry" "op_rc	$(cat "$R/cap/2op.rc")" "barrier_bound_sec	60"
    l_arm_end
}

# ─────────────────── SC-815c/d: two concurrent stop all runs, distinct op ids
arm_concurrent_ops() {
    l_arm_begin L-STOP fleet-concurrent-ops instrumented
    l_use_v2; PATCHV="L-HOOKS-v2"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    HOOKS=b_stop_supervisor_entry; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    : >"$R/cap/tmux-argv.log"
    l_ae_bg 2opA stop all -y
    local PA=$AE_BG_PID
    wait_barrier b_stop_supervisor_entry 60 >/dev/null || printf 'INCONCLUSIVE: run A supervisor barrier not reached in 60s\n' >>"$R/cap/INCONCLUSIVE.txt"
    printf 'runA.opid\t%s\n' "$(opid_from_ps)" >"$R/cap/opids.txt"
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/at-barrierA.ps.txt" 2>&1
    l_ae_bg 2opB stop all -y
    local PB=$AE_BG_PID
    sleep 6
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/at-barrierB.ps.txt" 2>&1
    { printf 'all supervisor argv seen while both runs were in flight\n'
      ps -ax -o pid=,command= | grep -F "$R" | grep '_stop-fleet-supervisor' | grep -v '[g]rep'; } >>"$R/cap/opids.txt" 2>&1
    ls -1 "$R"/ctl/*.reached >"$R/cap/barriers-pending.txt" 2>&1
    release_all
    sleep 1
    release_all
    wait "$PA" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2opA.rc"
    wait "$PB" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2opB.rc"
    sleep 8
    release_all
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    { printf '# every stop-result row, with its op tag, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt fleet-concurrent-ops "SC-815c SC-815d" \
      "two ae stop all -y runs over the same three sessions; run A is held at the supervisor-entry barrier while run B is started, both op ids are read from the process table, then both are released" \
      "barrier	b_stop_supervisor_entry (both runs)" \
      "opA_rc	$(cat "$R/cap/2opA.rc")" "opB_rc	$(cat "$R/cap/2opB.rc")" "barrier_bound_sec	60"
    l_arm_end
}

# ───────────────────────── SC-515a: a per-target record planted as a failure
arm_planted_failure() {
    l_arm_begin L-STOP exit-folding-planted-failure instrumented
    l_use_v2; PATCHV="L-HOOKS-v2"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    cp "$FIX"/stop-result.*.harvested "$FIX/PROVENANCE.txt" "$R/cap/"
    HOOKS=b_stop_supervisor_entry; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op stop all -y
    wait_barrier b_stop_supervisor_entry 60 >/dev/null || printf 'INCONCLUSIVE: supervisor barrier not reached in 60s\n' >"$R/cap/INCONCLUSIVE.txt"
    local OPID; OPID="$(opid_from_ps)"
    { printf 'opid.from.ps\t%s\n' "${OPID:-<none>}"
      printf 'source\tthe detached supervisor'"'"'s own argv in the process table — a system observation, not a hook read\n'
      ps -ax -o pid=,command= | grep -F "$R" | grep '_stop-fleet-supervisor' | grep -v '[g]rep'; } >"$R/cap/opid.txt" 2>&1
    # the supervisor is killed so it writes nothing; the controller supplies every record
    local p; for p in $(supervisor_pids); do l_killtree "$p"; done
    sleep 1
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/after-supervisor-kill.ps.txt" 2>&1
    : >"$R/cap/planted.diff"
    if [[ -n "$OPID" ]]; then
        plant_result proj  "$OPID" failure "$R/cap/planted.diff"
        plant_result projx "$OPID" success "$R/cap/planted.diff"
        plant_result other "$OPID" success "$R/cap/planted.diff"
    else
        printf 'INCONCLUSIVE: no op id observable in the process table; nothing planted\n' >>"$R/cap/INCONCLUSIVE.txt"
    fi
    release_all
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 2
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt exit-folding-planted-failure SC-515a \
      "ae stop all -y runs over three sessions; the detached supervisor is held at its entry barrier and then killed so it writes nothing, and the controller supplies EVERY per-target record itself from PRODUCER-HARVESTED stop-result lines with only the op id and the target name substituted — one target's record is the harvested FAILURE line, the other two the harvested SUCCESS line. The caller's bounded wait then folds them" \
      "planted_failure_target	proj" "planted_success_targets	projx, other" \
      "opid	${OPID:-<none>}" "op_rc	$(cat "$R/cap/2op.rc")" \
      "fixtures	stop-result.success.harvested, stop-result.failure.harvested (+ PROVENANCE.txt), byte diffs in planted.diff"
    l_arm_end
}

# ───────────────────────── SC-515b: the results wait reaches its bound
arm_results_timeout() {
    l_arm_begin L-STOP exit-folding-results-timeout instrumented
    l_use_v2; PATCHV="L-HOOKS-v2"
    stop_fleet proj projx other
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    ssnap 1pre
    HOOKS=b_stop_supervisor_entry; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op stop all -y
    wait_barrier b_stop_supervisor_entry 60 >/dev/null || printf 'INCONCLUSIVE: supervisor barrier not reached in 60s\n' >"$R/cap/INCONCLUSIVE.txt"
    printf 'opid.from.ps\t%s\n' "$(opid_from_ps)" >"$R/cap/opid.txt"
    local p; for p in $(supervisor_pids); do l_killtree "$p"; done
    sleep 1
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/after-supervisor-kill.ps.txt" 2>&1
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt exit-folding-results-timeout SC-515b \
      "ae stop all -y runs over three sessions; the detached supervisor is held at its entry barrier and killed so no per-target record is ever written, and nothing is planted. The caller's bounded wait therefore reaches its bound" \
      "op_rc	$(cat "$R/cap/2op.rc")" \
      "bound	the frozen bound is the literal 30 passed at 72c7293 ae:6612 (_stop_fleet_await ... 30); there is no environment knob for it, so this arm runs the REAL bound rather than a shortened one — recorded as a deviation from the design's wording, not as a shortened bound"
    l_arm_end
}

# ───────────────────────── SC-515c: an ae-tagged session ae does not own
arm_unowned_tagged() {
    l_arm_begin L-STOP exit-folding-unowned-ae-tagged frozen
    PATCHV="none (frozen, unmodified)"
    stop_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    /opt/homebrew/bin/tmux -S "$SOCK" new-session -d -s ghost -c "$R/w" 2>/dev/null
    /opt/homebrew/bin/tmux -S "$SOCK" set-environment -t ghost AE_SESSION 1
    sleep 1
    { printf 'construction\ta plain tmux session "ghost" is created directly on the recorded server and given AE_SESSION in its environment; ae has NO session directory for it\n'
      /opt/homebrew/bin/tmux -S "$SOCK" show-environment -t ghost | grep '^AE_'
      printf 'session_dirs\n'; ls -1 "$R/h/.ae/sessions"; } >"$R/cap/manipulation.txt" 2>&1
    ssnap 1pre
    : >"$R/cap/tmux-argv.log"
    l_ae 2op stop all -y
    sleep 6
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    ssnap 3post
    { printf '# stop-result rows, per session\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    sarmtxt exit-folding-unowned-ae-tagged SC-515c \
      "a tmux session carrying the ae tag but with no session directory is created directly on the recorded server, then ae stop all -y runs" \
      "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

case "${1:-all}" in
  a) arm_fourth_in_window ;;
  b) arm_name_handoff ;;
  c) arm_concurrent_ops ;;
  d) arm_planted_failure ;;
  e) arm_results_timeout ;;
  f) arm_unowned_tagged ;;
esac
echo DONE
