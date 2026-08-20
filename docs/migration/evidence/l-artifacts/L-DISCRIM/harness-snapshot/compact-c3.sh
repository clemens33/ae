#!/opt/homebrew/bin/bash
# L-COMPACT chunk 3: handover-facts (SC-829a, SC-829b).
set -uo pipefail
source /tmp/aelx/lib/compact-lib.sh

# The controller needs a pane that the frozen reply helper accepts as the main
# agent. It gets one by PLANTING the main agent's pane options onto a fresh
# pane in the same window — a named controller manipulation, recorded here.
plant_agent_pane() { # <session>
    local sess="$1"
    local mainpane agent slot
    mainpane="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t "=$sess" -F '#{pane_id} #{@ae_agent} #{@ae_slot}' | awk '$2!=""{print;exit}')"
    read -r mainpane agent slot <<<"$mainpane"
    local newp
    newp="$(/opt/homebrew/bin/tmux -S "$SOCK" split-window -d -t "$mainpane" -c "$R/w" -P -F '#{pane_id}')"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_agent "$agent"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_slot "$slot"
    { printf 'plant.reason\tthe frozen reply helper proves the responder from the CURRENT PANE'"'"'s @ae_slot; a controller process needs a pane the helper accepts\n'
      printf 'plant.source_pane\t%s (@ae_agent=%s @ae_slot=%s)\n' "$mainpane" "$agent" "$slot"
      printf 'plant.new_pane\t%s\n' "$newp"
      printf 'plant.action\tsplit-window in the same window, then set-option -p @ae_agent and @ae_slot to the source pane'"'"'s values\n'
      printf 'plant.not_changed\tthe agent pane itself, its process, and every file under AE_HOME\n'
    } >>"$R/cap/planted-pane.txt"
    printf '%s' "$newp"
}

pane_run() { # <pane> <label> <command-string>
    local p="$1" l="$2" c="$3"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" -l -- "$c > $R/cap/$l.stdout 2> $R/cap/$l.stderr; echo \$? > $R/cap/$l.rc"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" Enter
    printf '%s\n' "$c" >"$R/cap/$l.invocation"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/$l.rc" ]] && return 0; sleep 0.1; i=$((i+1)); done
    return 1
}

# bounded POSITIVE barrier: the handover request row appears in the events file
wait_for_request() { # <session> <timeout-sec>
    local s="$1" t="$2" i=0
    local f="$R/h/.ae/sessions/$s/events.jsonl"
    while (( i < t*10 )); do
        [[ -f "$f" ]] && grep -q '"action":"ask"' "$f" && return 0
        sleep 0.1; i=$((i+1))
    done
    return 1
}
req_id_of() { # <session>
    grep '"action":"ask"' "$R/h/.ae/sessions/$1/events.jsonl" 2>/dev/null | tail -1 |
        sed -n 's/.*"ref":"\([^"]*\)".*/\1/p'
}

source_trace() {
    local out="$R/cap/source-trace.what-completion-polls.txt"
    { printf '%s\n' 'FROZEN SOURCE, extracted verbatim. A code observation, not a verdict.'
      printf 'frozen commit\t72c729343a0117af2968b66e1c43f89ad25fc0b2\n\n'
      local fn
      for fn in _compact_wait_handover _compact_memo_offset _compact_baseline_of _compact_find_outstanding _compact_handover_secs; do
        local ln; ln="$(grep -n "^${fn}()" /tmp/aelx/frozen/ae | head -1 | cut -d: -f1)"
        printf '===== %s  (ae:%s) =====\n' "$fn" "${ln:-?}"
        awk -v s="^${fn}\\\\(\\\\) \\\\{" 'BEGIN{p=0} $0 ~ s {p=1} p {print} p && /^\}$/ {exit}' /tmp/aelx/frozen/ae
        printf '\n'
      done
    } >"$out"
    printf 'source-trace.sha256\t%s\n' "$(l_sha "$out")" >"$R/cap/source-trace.sha256.txt"
    return 0
}

arm_withholding() { # <only-reply|only-memo|neither>
    local mode="$1"
    l_arm_begin L-COMPACT "handover-withholding-$mode" frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    source_trace
    csnap 1pre
    : >"$R/cap/planted-pane.txt"
    local BOUND=25
    HOOKS=""; BLOCK=""; l_arm_env "AE_COMPACT_HANDOVER_SECS=$BOUND"
    l_ae_bg 2op compact -f cp1
    if ! wait_for_request cp1 60; then
        printf 'INCONCLUSIVE: no handover request row observed within the 60s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    fi
    local RID; RID="$(req_id_of cp1)"
    printf 'request.ref\t%s\n' "${RID:-<none>}" >"$R/cap/request.txt"
    cp "$R/h/.ae/sessions/cp1/events.jsonl" "$R/cap/events.at-request.jsonl" 2>/dev/null
    l_manifest "$R/h/.ae/sessions/cp1/messages" "$R/cap/messages.at-request.tsv" 2>/dev/null
    { for b in "$R"/h/.ae/sessions/cp1/messages/*; do [[ -f "$b" ]] || continue
        printf '=== %s ===\n' "$b"; cat "$b"; printf '\n'; done; } >"$R/cap/request-bodies.txt" 2>&1
    grep -h 'AE-COMPACT-MEMO-BASELINE=' "$R"/h/.ae/sessions/cp1/messages/* 2>/dev/null >"$R/cap/baseline-bytes-used.txt" || printf '(no baseline line found)\n' >"$R/cap/baseline-bytes-used.txt"
    local AP=""
    case "$mode" in
      only-reply)
        AP="$(plant_agent_pane cp1)"; sleep 1
        pane_run "$AP" 3reply "$R/h/.ae/sessions/cp1/reply $RID handover done, nothing else outstanding" || \
          printf 'INCONCLUSIVE: the reply helper produced no exit status within the 30s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
        ;;
      only-memo)
        AP="$(plant_agent_pane cp1)"; sleep 1
        pane_run "$AP" 3memo "$R/h/.ae/sessions/cp1/memo add --topic handover state of play at the boundary" || \
          printf 'INCONCLUSIVE: the memo helper produced no exit status within the 30s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
        ;;
      neither) printf 'nothing supplied by the controller\n' >"$R/cap/withheld.txt" ;;
    esac
    # let the bounded wait reach its bound
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 2
    cbytes 2op
    { printf '# state AT THE BOUND\n'
      printf '## requests all\n'; ( cd "$R/w" && env -i "${AE_ENV[@]}" "$R/h/.ae/sessions/cp1/requests" all ) 2>&1
      printf '\n## memo tail\n'; ( cd "$R/w" && env -i "${AE_ENV[@]}" "$R/h/.ae/sessions/cp1/memo" tail 10 ) 2>&1
      printf '\n## events (ask/reply/memo rows)\n'; grep -E '"action":"(ask|reply|memo)"' "$R/h/.ae/sessions/cp1/events.jsonl" 2>/dev/null
    } >"$R/cap/state-at-bound.txt" 2>&1
    csnap 3post
    carmtxt "handover-withholding-$mode" SC-829a \
      "a real compact -f runs under a shortened handover bound (AE_COMPACT_HANDOVER_SECS=$BOUND); once the handover request row is observed the controller supplies $( case "$mode" in only-reply) echo 'ONLY a reply, through the real generated reply helper, and no memo' ;; only-memo) echo 'ONLY a handover memo, through the real generated memo helper, and no reply' ;; neither) echo 'NOTHING' ;; esac ), and the wait then runs to its bound" \
      "mode	$mode" "handover_bound_sec	$BOUND" \
      "request_ref	${RID:-<none>}" "op_rc	$(cat "$R/cap/2op.rc")" \
      "source_trace	source-trace.what-completion-polls.txt (frozen source of _compact_wait_handover and the facts it polls, hashed)" \
      "$( [[ -n "$AP" ]] && printf 'planted_pane\t%s (see planted-pane.txt)' "$AP" || printf 'planted_pane\tnone' )"
    l_arm_end
}

arm_rerun() {
    l_arm_begin L-COMPACT handover-rerun-after-interrupt frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    source_trace
    csnap 1pre
    local BOUND=25
    HOOKS=""; BLOCK=""; l_arm_env "AE_COMPACT_HANDOVER_SECS=$BOUND"
    l_ae_bg 2op compact -f cp1
    wait_for_request cp1 60 || printf 'INCONCLUSIVE: no handover request row observed within the 60s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    local RID1; RID1="$(req_id_of cp1)"
    cp "$R/h/.ae/sessions/cp1/events.jsonl" "$R/cap/events.after-first-request.jsonl" 2>/dev/null
    grep -h 'AE-COMPACT-MEMO-BASELINE=' "$R"/h/.ae/sessions/cp1/messages/* 2>/dev/null >"$R/cap/baseline.run1.txt" || printf '(none)\n' >"$R/cap/baseline.run1.txt"
    # INTERRUPT, post-request
    l_killtree "$AE_BG_PID"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 1
    { printf 'interrupt.method\tSIGKILL to the whole compact process tree, AFTER the handover request was published\n'
      printf 'interrupt.request_ref\t%s\n' "${RID1:-<none>}"; } >"$R/cap/interrupt.txt"
    csnap 2after-interrupt
    # RE-RUN
    l_ae_bg 3rerun compact -f cp1
    sleep 8
    cp "$R/h/.ae/sessions/cp1/events.jsonl" "$R/cap/events.after-rerun.jsonl" 2>/dev/null
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/3rerun.rc"
    sleep 2
    cbytes 3rerun
    local RID2; RID2="$(req_id_of cp1)"
    { printf 'run1.request_ref\t%s\n' "${RID1:-<none>}"
      printf 'run2.request_ref\t%s\n' "${RID2:-<none>}"
      printf 'ask.rows.count\t%s\n' "$(grep -c '"action":"ask"' "$R/h/.ae/sessions/cp1/events.jsonl" 2>/dev/null || echo 0)"
      printf 'reply.rows.count\t%s\n' "$(grep -c '"action":"reply"' "$R/h/.ae/sessions/cp1/events.jsonl" 2>/dev/null || echo 0)"
      printf '\n# every ask row, verbatim\n'; grep '"action":"ask"' "$R/h/.ae/sessions/cp1/events.jsonl" 2>/dev/null
      printf '\n# baseline lines in every request body\n'
      grep -h 'AE-COMPACT-MEMO-BASELINE=' "$R"/h/.ae/sessions/cp1/messages/* 2>/dev/null
    } >"$R/cap/request-events.txt" 2>&1
    grep -h 'AE-COMPACT-MEMO-BASELINE=' "$R"/h/.ae/sessions/cp1/messages/* 2>/dev/null >"$R/cap/baseline.run2.txt" || printf '(none)\n' >"$R/cap/baseline.run2.txt"
    diff -u "$R/cap/baseline.run1.txt" "$R/cap/baseline.run2.txt" >"$R/cap/baseline.diff" 2>&1
    l_manifest "$R/h/.ae/sessions/cp1/messages" "$R/cap/messages.after-rerun.tsv" 2>/dev/null
    csnap 3post
    carmtxt handover-rerun-after-interrupt SC-829b \
      "a real compact -f is interrupted (SIGKILL to the whole tree) AFTER its handover request is published, then compact is run again on the same session; the request events (count and refs) and the baseline bytes carried in each request body are captured across both runs" \
      "handover_bound_sec	$BOUND" \
      "run1_rc	$(cat "$R/cap/2op.rc")" "run2_rc	$(cat "$R/cap/3rerun.rc")" \
      "run1_request_ref	${RID1:-<none>}" "run2_request_ref	${RID2:-<none>}" \
      "source_trace	source-trace.what-completion-polls.txt"
    l_arm_end
}

case "${1:-all}" in
  wr)  arm_withholding only-reply ;;
  wm)  arm_withholding only-memo ;;
  wn)  arm_withholding neither ;;
  rr)  arm_rerun ;;
esac
echo DONE
