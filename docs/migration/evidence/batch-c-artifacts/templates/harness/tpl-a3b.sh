#!/opt/homebrew/bin/bash
# A3b members: the five UNDISCRIMINATED adjacent rank pairs, plus a competing set whose
# lower-rank event is issued by an UNINVOLVED agent so no reason-owner's own later
# activity can clear its own alert.
#
# Rank ladder read out of the frozen source (ae@72c7293:3571-3581, comment at :3586):
#   dead 6 > stale 5 > waiting-user 4 > blocked 3 > throttled 2 > unanswered 1
# Each pair arm puts the HIGHER-rank reason FIRST in arrival order and the adjacent
# LOWER-rank reason last, so a last-wins reader and a rank-wins reader disagree.
source "$(dirname "$0")/tlib.sh"
SHIM=/tmp/aecx/shim; REALDATE=/bin/date
PHRASE='429 Too Many Requests'

wait_event() { # <events-file> <grep-pattern> <timeout-s> -> 0 found / 3 timeout
    local f="$1" pat="$2" tmo="$3" t0; t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < tmo )); do
        grep -q "$pat" "$f" 2>/dev/null && return 0
        sleep 2
    done
    return 3
}
kill_fake_under() { # <pane-pid>
    local target="$1" p anc
    for p in $(pgrep -x aefake); do
        anc="$p"
        while [[ -n "$anc" && "$anc" != "1" && "$anc" != "0" ]]; do
            [[ "$anc" == "$target" ]] && { kill -TERM "$p" 2>/dev/null; return 0; }
            anc="$(ps -o ppid= -p "$anc" 2>/dev/null | tr -d '[:space:]')"
        done
    done
    return 1
}
say_phrase() { # <pane-id>
    local fifo="$ROOT/ctl/${1}.ctl" t0; t0=$(/bin/date -u +%s)
    while [[ ! -p "$fifo" ]] && (( $(/bin/date -u +%s) - t0 < 20 )); do sleep 0.5; done
    [[ -p "$fifo" ]] || return 1
    printf '%s\n' "$PHRASE" >"$fifo"
}

# build_pair <member> <rowlabel> <recipe-fn> <workers>
build_pair() {
    local mem="$1" rows="$2" recipe="$3" workers="$4"
    echo; echo "######## $mem ($rows) ########"
    export T_WATCHDOG=true
    export T_MAIN=fake:high
    t_sandbox "p_$mem" "$workers"
    export AE_WATCHDOG_INTERVAL_SEC=5
    export AE_WATCHDOG_STALE_MIN=1
    export AE_WATCHDOG_MAX_NUDGES=2
    export AE_WATCHDOG_THROTTLE_ALERT_CYCLES=2
    export AE_WATCHDOG_TG_SUPERVISE_SEC=0
    export AE_SEND_DEFER_SEC=5
    export PATH="$SHIM:$PATH"; export AE_REAL_DATE="$REALDATE"
    export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
    t_launch "t${mem//[^a-z0-9]/}" || { echo "LAUNCH FAILED"; return 1; }
    P="$CAP/prov.txt"; : >"$P"
    { echo "member=$mem rows=$rows"
      echo "rank ladder cited from the frozen source: ae@72c7293:3571-3581 (comment at :3586)"
      echo "construction: the HIGHER-rank reason arrives FIRST, the adjacent LOWER-rank reason LAST"
      echo "watchdog knobs: INTERVAL=5 STALE_MIN=1 MAX_NUDGES=2 THROTTLE_ALERT_CYCLES=2"
      echo "no reason-owner is the actor of any later event, so no own-activity clear can occur"
    } >>"$P"
    "$recipe"
    echo "events:"; cat "$META/events.jsonl"
    "$META/watchdog" stop >/dev/null 2>&1
    mkdir -p "$TSTORE/A3b/_meta"
    echo "$mem fingerprint(pre)=$(t_store A3b "$mem" "$P")"
    t_protect A3b "$mem" >/dev/null
    cp "$AE_DATE_SHIM_LOG" "$TSTORE/A3b/_meta/$mem.date-shim-invocations.log" 2>/dev/null || true
    t_teardown
    unset T_WATCHDOG AE_WATCHDOG_INTERVAL_SEC AE_WATCHDOG_STALE_MIN AE_WATCHDOG_MAX_NUDGES \
          AE_WATCHDOG_THROTTLE_ALERT_CYCLES AE_WATCHDOG_TG_SUPERVISE_SEC AE_SEND_DEFER_SEC \
          AE_DATE_SHIM_LOG AE_REAL_DATE
}

r_dead_stale() {
    local H W
    H="$(pane_of fake:high)"; W="$(pane_of fake:low)"
    echo "  T0: kill the fake child under fake:high's pane -> the real watchdog raises the DEAD alert" >>"$P"
    kill_fake_under "$(tm display-message -p -t "$H" '#{pane_pid}')"
    wait_event "$META/events.jsonl" 'dropped to shell' 120 || echo "  OUTCOME=INCONCLUSIVE reason=no dead alert in 120s" >>"$P"
    echo "  T1: fake:low left static past the shortened stale window -> the real watchdog nudges then alerts STALE" >>"$P"
    wait_event "$META/events.jsonl" 'max nudges reached' 300 || echo "  OUTCOME=INCONCLUSIVE reason=no stale alert in 300s" >>"$P"
}
r_stale_waitinguser() {
    local L H t0; L="$(pane_of fake:low)"; H="$(pane_of fake:high)"
    echo "  T0: fake:high left static past the shortened stale window -> STALE alert." >>"$P"
    echo "      fake:low is kept demonstrably ACTIVE meanwhile (a line into its own pane every" >>"$P"
    echo "      15s) so it cannot accumulate a stale alert of its own — the pair must isolate" >>"$P"
    echo "      ONE reason per agent, not stack two on the lower one." >>"$P"
    t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 300 )); do
        grep -q "max nudges reached" "$META/events.jsonl" 2>/dev/null && break
        printf 'low-keepalive-%s
' "$(/bin/date -u +%s)" >"$ROOT/ctl/${L}.ctl" 2>/dev/null || true
        sleep 15
    done
    grep -q "max nudges reached" "$META/events.jsonl" 2>/dev/null ||         echo "  OUTCOME=INCONCLUSIVE reason=no stale alert in 300s" >>"$P"
    echo "  low stale alerts present: $(grep -c '"target":"fake:low".*max nudges' "$META/events.jsonl" || true)" >>"$P"
    echo "  T1: fake:low declares waiting-user with its own real state helper" >>"$P"
    as_agent "$L" state waiting-user "A3b lower-rank declaration"; echo "    rc=$?" >>"$P"
}
r_waitinguser_blocked() {
    local H L; H="$(pane_of fake:high)"; L="$(pane_of fake:low)"
    echo "  T0: fake:high declares waiting-user" >>"$P"
    as_agent "$H" state waiting-user "A3b higher-rank declaration"; echo "    rc=$?" >>"$P"
    sleep 2
    echo "  T1: fake:low declares blocked" >>"$P"
    as_agent "$L" state blocked "A3b lower-rank declaration"; echo "    rc=$?" >>"$P"
}
r_blocked_throttled() {
    local H L; H="$(pane_of fake:high)"; L="$(pane_of fake:low)"
    echo "  T0: fake:high declares blocked" >>"$P"
    as_agent "$H" state blocked "A3b higher-rank declaration"; echo "    rc=$?" >>"$P"
    sleep 2
    echo "  T1: fake:low prints the documented generic throttle phrase into its pane tail" >>"$P"
    say_phrase "$L" || echo "    OUTCOME=INCONCLUSIVE reason=no control fifo for $L" >>"$P"
    wait_event "$META/events.jsonl" '"action":"throttled"' 120 || echo "  OUTCOME=INCONCLUSIVE reason=no throttled event in 120s" >>"$P"
}
r_throttled_unanswered() {
    local H X; H="$(pane_of fake:high)"; X="$(pane_of fake:asker)"
    echo "  T0: fake:high prints the documented generic throttle phrase into its pane tail" >>"$P"
    say_phrase "$H" || echo "    OUTCOME=INCONCLUSIVE reason=no control fifo for $H" >>"$P"
    wait_event "$META/events.jsonl" '"action":"throttled"' 120 || echo "  OUTCOME=INCONCLUSIVE reason=no throttled event in 120s" >>"$P"
    echo "  T1: fake:asker (an UNINVOLVED agent) asks fake:low under the clock hook, aged past" >>"$P"
    echo "      the 1800s default and never replied. The asker owns no reason, so nothing clears." >>"$P"
    export AE_FAKE_NOW=1755000000
    as_agent "$X" ask fake:low "A3b unanswered question (never replied)"; echo "    rc=$?" >>"$P"
    unset AE_FAKE_NOW
}
r_competing_noclear() {
    local H B C X
    H="$(pane_of fake:high)"; B="$(pane_of fake:low)"; X="$(pane_of fake:asker)"
    echo "  T0: kill the fake child under fake:high's pane -> DEAD alert (rank 6)" >>"$P"
    kill_fake_under "$(tm display-message -p -t "$H" '#{pane_pid}')"
    wait_event "$META/events.jsonl" 'dropped to shell' 120 || echo "  OUTCOME=INCONCLUSIVE reason=no dead alert in 120s" >>"$P"
    echo "  T1: fake:low declares waiting-user (rank 4)" >>"$P"
    as_agent "$B" state waiting-user "A3b competing middle-rank declaration"; echo "    rc=$?" >>"$P"
    sleep 2
    echo "  T2: fake:asker — an agent owning NO reason — asks fake:third, aged, never replied (rank 1)." >>"$P"
    echo "      This is the fix for the own-activity clear: in the earlier G2b the asker was the" >>"$P"
    echo "      dead agent itself, so its own later event cleared its own alert." >>"$P"
    export AE_FAKE_NOW=1755000000
    as_agent "$X" ask fake:third "A3b competing unanswered question (never replied)"; echo "    rc=$?" >>"$P"
    unset AE_FAKE_NOW
}
#build_pair pair-dead-over-stale disabled for the targeted rebuild
build_pair pair-stale-over-waitinguser "SC-017g adjacent pair stale>waiting-user" r_stale_waitinguser "fake:low"

echo "TPL-A3b DONE"
