#!/opt/homebrew/bin/bash
# T-100: six arms — three change-sources x two quiet states.
# CAPTURE ONLY. Nothing here states which outcome is correct.
set -uo pipefail
source /tmp/aelx/lib/t100-lib.sh

INTERVAL=5; STALE_MIN=1; SETTLE_BEAT=1; SETTLE_TRIES=4; STAB_PER_CYCLE=4; MAXN=2
OBS_CYCLES=30            # observation cycles after the declaration (x INTERVAL sec)
                         # 150s, deliberately longer than STALE_MIN so the stale branch is REACHABLE
NUDGE_WAIT=45            # bounded wait for a nudge event, seconds

CANNED_LINE='CANNED-AGENT-LINE: continuing to wait; nothing new to report.'
HUMAN_LINE='HUMAN-TYPED-REPLY: yes, go ahead with option two.'

wd_pane() { /opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{pane_id} #{@ae_agent}' 2>/dev/null | awk '$2=="_watchdog"{print $1; exit}'; }
agent_pane() { /opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{pane_id} #{@ae_agent}' 2>/dev/null | awk '$2!="" && $2!~/^_/{print $1; exit}'; }

t_wdlog() { # <label>
    local lbl="$1" wp; wp="$(wd_pane)"
    if [[ -n "$wp" ]]; then
        /opt/homebrew/bin/tmux -S "$SOCK" capture-pane -p -J -S -400 -E - -t "$wp" >"$R/cap/wdlog.$lbl.txt" 2>/dev/null
    else
        printf '(no _watchdog pane)\n' >"$R/cap/wdlog.$lbl.txt"
    fi
    local _n; _n="$(grep -c . "$R/cap/wdlog.$lbl.txt" 2>/dev/null)"
    led "$lbl" wdlog.lines "${_n:-0}"
    return 0
}

# grep -c exits 1 on a ZERO count, so `grep -c ... || echo 0` prints "0" TWICE and
# every later $(( )) on it is a syntax error. Take the count, default it, print once.
nudge_count_now() {
    local n; n="$(grep -c '"action":"nudge"' "$R/h/.ae/sessions/t1/events.jsonl" 2>/dev/null)"
    printf '%s' "${n:-0}"
}

obs() { # <label> <pane>
    local lbl="$1" p="$2"
    t_capture "$p" "$lbl" >/dev/null
    t_events t1 "$lbl"
    t_wdlog "$lbl"
    return 0
}

run_arm() { # <state: waiting-user|blocked> <source: A|B|C>
    local st="$1" src="$2"
    local arm="t100-${src}-${st}"
    l_arm_begin T-100 "$arm" frozen
    cp /tmp/aelx/lib/t100-fake.sh "$R/b/fake-tool.sh"
    # THE TOOL IS DELIBERATELY AN UNMODELLED ONE. ae's send path gates delivery on a
    # readiness/staged-paste sensor for the TUI-modelled tools, and a fake that does
    # not present their idle screen makes every nudge undeliverable — which would make
    # these arms about delivery rather than about quiet states. grok is documented as
    # having no readiness detection, so delivery is ungated, and the fake's verbatim
    # line rendering is the same unmodelled shape the scrubber's raw branch handles.
    { printf '[agents]\n'
      printf 'grok = "grok %s/b/fake-tool.sh"\n' "$R"
      printf '\n[workspace]\nmain = grok\nlayout = vertical\n'
    } >"$R/h/.ae/config"
    { l_mkrepo "$R"; } >/dev/null 2>&1
    : >"$R/cap/ledger.tsv"; : >"$R/cap/writes.txt"
    led setup arm "$arm"
    led setup knobs "INTERVAL=$INTERVAL STALE_MIN=$STALE_MIN QUIET_SETTLE_BEAT=$SETTLE_BEAT QUIET_SETTLE=$SETTLE_TRIES QUIET_STABILIZE_PER_CYCLE=$STAB_PER_CYCLE MAX_NUDGES=$MAXN"
    HOOKS=""; BLOCK=""
    l_arm_env "AE_WATCHDOG_INTERVAL_SEC=$INTERVAL" "AE_WATCHDOG_STALE_MIN=$STALE_MIN" \
              "AE_WATCHDOG_QUIET_SETTLE_BEAT=$SETTLE_BEAT" "AE_WATCHDOG_QUIET_SETTLE=$SETTLE_TRIES" \
              "AE_WATCHDOG_QUIET_STABILIZE_PER_CYCLE=$STAB_PER_CYCLE" "AE_WATCHDOG_MAX_NUDGES=$MAXN"
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local t1
    sleep 4
    led setup launch.rc "$(cat "$R/cap/0launch.rc")"
    l_arm_preflight t1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local P; P="$(agent_pane)"
    # Read the agent ref back from tmux: t_plant_agent_pane sets it in a $( ) subshell,
    # which forks the assignment away — the frozen code carries a note about exactly
    # this hazard, and the harness is not exempt from it.
    AGENT_REF="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{pane_id} #{@ae_agent}' 2>/dev/null | awk '$2!="" && $2!~/^_/{print $2; exit}')"
    led setup agent.pane "${P:-<none>}"
    led setup agent.ref "${AGENT_REF:-<none>}"
    [[ -n "$P" ]] || { printf 'ARM INVALID: no agent pane\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    /opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{session_name}|#{window_id}|#{pane_id}|#{pane_current_command}|#{@ae_agent}|#{pane_tty}|#{pane_dead}' >"$R/cap/panes.txt" 2>&1
    t_extract_scrubber t1 || { printf 'ARM INVALID: could not extract the frozen scrubber\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    led setup scrubber.sha256 "$(l_sha "$R/cap/extracted-watchdog-fns.sh")"

    # ── INSTRUMENT LIVE CONTROL, written by the checks as they run ───────────
    # (1) the recorder must REGISTER a plain pane write, or nothing below means
    #     anything; (2) it must NOT register a nudge-shaped block, which is the
    #     discriminator the product itself uses.
    local c0 c1 cn
    c0="$(t_capture "$P" ctl0)"
    t_write_pane ctl "INSTRUMENT-CONTROL-MARKER-$$"
    sleep 1
    c1="$(t_capture "$P" ctl1)"
    led control plain-write.hash.before "$c0"
    led control plain-write.hash.after "$c1"
    led control plain-write.registered "$( [[ "$c0" != "$c1" ]] && echo YES || echo NO )"
    # BOTH SIDES OF THE NUDGE CONTROL MUST BE COMPARED ON THE SAME FOOTING.
    # The frozen scrubber receives its buffer through a command substitution, which
    # strips TRAILING NEWLINES — so a pane's empty bottom rows survive when text
    # follows them and vanish when it does not. Appending the block to the raw
    # capture therefore changes the buffer in a second, unrelated way. The base is
    # taken through the same stripping first, and the block is appended to that.
    printf '%s' "$(cat "$R/cap/pane.ctl1.raw")" >"$R/cap/pane.ctlbase.raw"
    local cb; cb="$(t_hash "$R/cap/pane.ctlbase.raw")"
    printf '%s\n' "$cb" >"$R/cap/hash.ctlbase.txt"
    led control base.stripped.hash "$cb"
    { cat "$R/cap/pane.ctlbase.raw"
      printf '\n'
      printf '⟦ae:msg from watchdog⟧\n'
      printf 'Status check: if you have more work, continue. Otherwise declare your state so I stop nudging: %s/state <waiting-user|blocked|done> "<reason>"\n' "$R/h/.ae/sessions/t1"
    } >"$R/cap/pane.ctlnudge.raw"
    od -c "$R/cap/pane.ctlnudge.raw" >"$R/cap/pane.ctlnudge.od"
    cn="$(t_hash "$R/cap/pane.ctlnudge.raw")"
    printf '%s\n' "$cn" >"$R/cap/hash.ctlnudge.txt"
    led control nudge-shaped.hash "$cn"
    led control nudge-shaped.registered "$( [[ "$cb" != "$cn" ]] && echo YES || echo NO )"
    if [[ "$c0" == "$c1" || "$cb" != "$cn" ]]; then
        { printf 'ARM INVALID — the instrument did not demonstrate the product discriminator.\n'
          printf 'plain pane write registered: %s (must be YES)\n' "$( [[ "$c0" != "$c1" ]] && echo YES || echo NO )"
          printf 'nudge-shaped block registered: %s (must be NO)\n' "$( [[ "$cb" != "$cn" ]] && echo YES || echo NO )"
          printf 'Without both, a later unchanged hash could not be sourced to the product rather than to the recorder.\n'
        } >"$R/cap/ARM-INVALID.txt"
        l_arm_end; return 1
    fi

    # ── FORCED INITIAL YIELD, then SETTLE, before the state is declared ──────
    t_write_pane yield "FORCED-INITIAL-YIELD-MARKER-$$"
    led yield write.done "a marker is written to the observed pane so it demonstrably CHANGES before the declaration"
    local prev="" cur="" i=0 settled=""
    while (( i < 40 )); do
        cur="$(t_capture "$P" "settle$i")"
        if [[ -n "$prev" && "$cur" == "$prev" ]]; then settled="$cur"; break; fi
        prev="$cur"; sleep 1; i=$((i+1))
    done
    led yield settle.samples "$i"
    led yield settle.hash "${settled:-<never settled>}"
    if [[ -z "$settled" ]]; then
        printf 'ARM INVALID: the pane never settled within the 40-sample bound, so no stable baseline existed before the declaration\n' >"$R/cap/ARM-INVALID.txt"
        l_arm_end; return 1
    fi

    obs t0-pre-declaration "$P"

    # ── DECLARE the quiet state through the REAL state helper ────────────────
    local AP; AP="$(t_plant_agent_pane t1)"
    sleep 1
    led declare planted.pane "$AP"
    t_pane_run "$AP" 1declare "$R/h/.ae/sessions/t1/state $st T-100 arm $src" \
        || led declare state.helper "NO EXIT STATUS WITHIN THE 30s BOUND"
    led declare state.rc "$(cat "$R/cap/1declare.rc" 2>/dev/null || echo '<none>')"
    led declare state.stdout "$(head -1 "$R/cap/1declare.stdout" 2>/dev/null)"
    # RETIRE THE PLANTED PANE AT ONCE. It carries a COPY of @ae_agent/@ae_slot, so
    # while it exists the watchdog sees a SECOND pane for this agent whose process is
    # a plain shell — which it classifies dead, and a dead-classified agent is never
    # asked the question this arm is about.
    /opt/homebrew/bin/tmux -S "$SOCK" kill-window -t "$AP" 2>/dev/null
    led declare planted.pane.retired "the planted window is killed immediately after the helper returns"
    sleep 1
    # THE HELPER'S OWN STDOUT BYTES, RELOCATED TO THE OBSERVED PANE. In a real session
    # the state helper runs in the agent's own pane and its echo lands there — which is
    # exactly the line the frozen scrubber is written to remove. Nothing is reshaped:
    # these are the helper's own bytes, written verbatim.
    if [[ -s "$R/cap/1declare.stdout" ]]; then
        local _echo; _echo="$(head -1 "$R/cap/1declare.stdout")"
        t_write_pane declaration-echo "$_echo"
        led declare echo.relocated "$_echo"
    else
        led declare echo.relocated "NONE — the state helper produced no stdout"
    fi
    sleep 2
    obs t1-post-declaration "$P"
    local N0; N0="$(nudge_count_now)"
    led declare nudges.at.declaration "$N0"

    # ── the arm's own change-source ──────────────────────────────────────────
    local WROTE_AT_CYCLE=-1 NUDGE_SEEN=no
    case "$src" in
      A)  led source plan "NUDGE-ONLY: no further write of any kind is made to the observed pane" ;;
      B)  led source plan "INDUCED RESPONSE: bounded wait for a nudge event, then ONE fixed canned line written to the observed pane; the state is NOT redeclared" ;;
      C)  led source plan "UNOWNED CHANGE (control): one human-like line written to the observed pane, early, with no agent involvement; zero nudges must have fired first" ;;
    esac

    if [[ "$src" == C ]]; then
        sleep 3
        local NC; NC="$(nudge_count_now)"
        led source nudges.before.write "$NC"
        if (( NC > N0 )); then
            printf 'ARM INVALID: a nudge fired before the unowned write, so the pane change cannot be attributed to the controller alone (nudges %s -> %s)\n' "$N0" "$NC" >"$R/cap/ARM-INVALID.txt"
            l_arm_end; return 1
        fi
        t_write_pane unowned "$HUMAN_LINE"
        WROTE_AT_CYCLE=0
        obs t2-after-unowned-write "$P"
    fi

    # ── observation loop ─────────────────────────────────────────────────────
    local c
    for ((c = 1; c <= OBS_CYCLES; c++)); do
        sleep "$INTERVAL"
        obs "c$(printf '%02d' "$c")" "$P"
        local NN; NN="$(nudge_count_now)"
        led "c$(printf '%02d' "$c")" nudges.total "$NN"
        if [[ "$src" == B && "$WROTE_AT_CYCLE" -lt 0 ]]; then
            if (( NN > N0 )); then
                NUDGE_SEEN=yes
                led source nudge.observed "at cycle $c, nudge count $N0 -> $NN"
                t_write_pane induced "$CANNED_LINE"
                WROTE_AT_CYCLE=$c
                sleep 2
                obs t2-after-induced-write "$P"
            elif (( c * INTERVAL >= NUDGE_WAIT )); then
                led source nudge.not.observed "no nudge event within the ${NUDGE_WAIT}s bound; the canned line is written anyway and this is recorded"
                t_write_pane induced "$CANNED_LINE"
                WROTE_AT_CYCLE=$c
                sleep 2
                obs t2-after-induced-write "$P"
            fi
        fi
    done

    obs t3-final "$P"
    local NF; NF="$(nudge_count_now)"
    # A dead-classified agent is never asked the question this arm is about, so a DEAD
    # line anywhere in the watchdog's own log invalidates the arm rather than becoming
    # an observation about quiet states.
    local _dead; _dead="$(grep -c 'DEAD\|dead (no further checks)' "$R/cap/wdlog.t3-final.txt" 2>/dev/null)"
    led final wdlog.dead.lines "${_dead:-0}"
    if [[ "${_dead:-0}" != 0 ]]; then
        { printf 'ARM INVALID: the watchdog classified the observed agent DEAD during the run (%s log lines).\n' "${_dead:-0}"
          printf 'A dead-classified agent is skipped before the quiet-state question is reached, so nothing here would be an observation about quiet states.\n'
          grep -n 'DEAD\|dead (no further checks)' "$R/cap/wdlog.t3-final.txt" | head -10
        } >"$R/cap/ARM-INVALID.txt"
    fi

    # ── the watchdog's own per-cycle verdict lines for this agent ────────────
    { printf '# the watchdog own log for the observed agent, from its _watchdog pane\n'
      grep -F "$AGENT_REF" "$R/cap/wdlog.t3-final.txt" 2>/dev/null || printf '(no matching line)\n'
    } >"$R/cap/watchdog-verdict-lines.txt"
    { printf '# every nudge and state event, in order\n'
      grep -E '"action":"(nudge|state)"' "$R/cap/events.t3-final.jsonl" 2>/dev/null || printf '(none)\n'
    } >"$R/cap/event-timeline.txt"
    { printf '# the scrubbed hash at every observation point, in order\n'
      for f in "$R"/cap/hash.*.txt; do printf '%s\t%s\n' "$(basename "$f" .txt | sed 's/^hash\.//')" "$(cat "$f")"; done
    } >"$R/cap/hash-timeline.txt"

    led final nudges.total "$NF"
    led final nudges.after.declaration "$((NF - N0))"

    { printf 'arm\t%s\nsection\tT-100\n' "$arm"
      printf 'issue\t#100 capture arm\n'
      printf 'quiet_state\t%s\n' "$st"
      printf 'change_source\t%s\n' "$src"
      printf 'construction\t%s\n' "$( case "$src" in
          A) echo 'the state is declared and NOTHING further is written to the observed pane' ;;
          B) echo 'the state is declared; after a bounded wait for a nudge event, ONE fixed canned line is written to the observed pane and the state is NOT redeclared' ;;
          C) echo 'the state is declared and the observed pane is changed by a human-like typed write, with no agent involvement and with zero nudges having fired first' ;;
        esac )"
      printf 'binary\tfrozen 72c7293, unmodified (sha256 %s)\n' "$(l_sha "$R/b/ae")"
      printf 'tool\tgrok — an UNMODELLED tool, chosen so nudge delivery is ungated; the fake is a renamed copy of bash rendering every received line verbatim\n'
      printf 'tool.binary.sha256\t%s\n' "$(l_sha "$R/b/grok")"
      printf 'watchdog\tthe REAL generated watchdog auto-started by the real launch; pacing rides the documented AE_WATCHDOG_* knobs only\n'
      printf 'knobs\tINTERVAL=%s STALE_MIN=%s QUIET_SETTLE_BEAT=%s QUIET_SETTLE=%s QUIET_STABILIZE_PER_CYCLE=%s MAX_NUDGES=%s\n' "$INTERVAL" "$STALE_MIN" "$SETTLE_BEAT" "$SETTLE_TRIES" "$STAB_PER_CYCLE" "$MAXN"
      printf 'observation_cycles\t%s x %ss\n' "$OBS_CYCLES" "$INTERVAL"
      printf 'agent_pane\t%s\nagent_ref\t%s\n' "$P" "${AGENT_REF:-<none>}"
      printf 'state_helper_rc\t%s\n' "$(cat "$R/cap/1declare.rc" 2>/dev/null || echo '<none>')"
      printf 'nudges_at_declaration\t%s\n' "$N0"
      printf 'nudges_total_at_end\t%s\n' "$NF"
      printf 'nudges_after_declaration\t%s\n' "$((NF - N0))"
      printf 'nudge_observed_before_write\t%s\n' "$NUDGE_SEEN"
      printf 'wrote_at_cycle\t%s\n' "$WROTE_AT_CYCLE"
      printf 'OBSERVATION\tnudges after the declaration: %s. The watchdog own per-cycle lines for this agent are in watchdog-verdict-lines.txt; the scrubbed-hash timeline is in hash-timeline.txt; the event timeline is in event-timeline.txt. No verdict is stated here.\n' "$((NF - N0))"
    } >"$R/cap/ARM.txt"
    l_arm_end
    return 0
}

case "${1:-}" in
  a1) run_arm waiting-user A ;;
  a2) run_arm blocked A ;;
  b1) run_arm waiting-user B ;;
  b2) run_arm blocked B ;;
  c1) run_arm waiting-user C ;;
  c2) run_arm blocked C ;;
esac
echo DONE
