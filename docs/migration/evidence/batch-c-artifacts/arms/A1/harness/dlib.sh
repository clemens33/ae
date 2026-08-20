#!/opt/homebrew/bin/bash
# D-record barrier driver. One shape for every hooked concurrency cut:
#   clone -> live topology -> standing checks -> before fingerprints -> hooked consumer
#   blocks at its hook -> AT-BARRIER capture -> CONTROLLER performs the named mutation ->
#   post-mutation capture -> release -> completion capture -> after fingerprints.
# Every barrier arm is paired with a CONTROLLER-ONLY TWIN: the same mutation performed
# alone on a fresh clone with no hooked reader, captured identically, so the controller's
# own effect can be subtracted from the barrier arm's.
source "$(dirname "${BASH_SOURCE[0]}")/armlib.sh"

FLOCK_SHIM_DIR=/tmp/aecx/shim-flock
TMUX_SHIM_DIR=/tmp/aecx/shim-tmux

d_env() { # <ae-home> <sock> <trace-prefix> [extra k=v ...]
    local aehome="$1" sock="$2" pfx="$3"; shift 3
    D_ENV=(env -i "HOME=$(dirname "$aehome")" "AE_HOME=$aehome"
        "PATH=${TMUX_SHIM_DIR}:${FLOCK_SHIM_DIR}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color"
        "TMUX_TMPDIR=${ARM_TMUXTMP}"
        "AE_REAL_TMUX=/opt/homebrew/bin/tmux" "AE_REAL_FLOCK=/opt/homebrew/bin/flock"
        "AE_TMUX_SHIM_LOG=$ACAP/out/${pfx}.tmuxtrace"
        "AE_FLOCK_SPY_LOG=$ACAP/out/${pfx}.flockspy")
    [[ -n "$sock" ]] && D_ENV+=("AE_TMUX_SERVER=$sock" "AE_TMUX_SERVER_KIND=socket")
    local kv; for kv in "$@"; do D_ENV+=("$kv"); done
    : >"$ACAP/out/${pfx}.tmuxtrace"; : >"$ACAP/out/${pfx}.flockspy"
}

d_snapshot() { # <label> <ae-home> <sock>
    local lbl="$1" aehome="$2" sock="$3"
    dir_manifest "$aehome" >"$ACAP/manifest.$lbl.tsv"
    { echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_windows}' 2>&1
      echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{pane_current_command}' 2>&1
    } >"$ACAP/tmux.$lbl.txt"
    led snapshot "label=$lbl" "manifest_sha256=$(sha "$ACAP/manifest.$lbl.tsv")" \
        "tmux_sha256=$(sha "$ACAP/tmux.$lbl.txt")"
}

# d_barrier_run <hook> <prefix> <ae-home> <sock> <mutation-fn> -- <consumer argv...>
d_barrier_run() {
    local hook="$1" pfx="$2" aehome="$3" sock="$4" mutfn="$5"; shift 5
    [[ "${1:-}" == "--" ]] && shift
    local hd="$ARM_TMUXTMP/hookdir.$pfx"; rm -rf "$hd"; mkdir -p "$hd"
    d_env "$aehome" "$sock" "$pfx" "AE_HOOK=$hook" "AE_HOOK_DIR=$hd"
    led barrier-ARMED "hook=$hook" "prefix=$pfx" "argv=$(printf '%q ' "$@")"
    ( "${D_ENV[@]}" "$@" </dev/null >"$ACAP/out/$pfx.stdout" 2>"$ACAP/out/$pfx.stderr"; echo $? >"$hd/rc" ) &
    local bpid=$! t0 reached=0
    t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 90 )); do
        [[ -e "$hd/$hook.reached" ]] && { reached=1; break; }
        kill -0 "$bpid" 2>/dev/null || break
        sleep 0.2
    done
    led barrier-REACHED "hook=$hook" "reached=$reached" "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
    if (( reached == 0 )); then
        led OUTCOME-INCONCLUSIVE "reason=hook $hook not reached within 90s"
        wait "$bpid" 2>/dev/null
        D_RC="$(cat "$hd/rc" 2>/dev/null || echo '?')"; D_REACHED=0
        return 0
    fi
    d_snapshot "at-barrier-before-mutation" "$aehome" "$sock"
    "$mutfn" "$aehome" "$sock"
    d_snapshot "at-barrier-after-mutation" "$aehome" "$sock"
    : >"$hd/$hook.release"
    led barrier-RELEASED "hook=$hook"
    wait "$bpid" 2>/dev/null
    D_RC="$(cat "$hd/rc" 2>/dev/null || echo '?')"; D_REACHED=1
    cp "$hd/hook.log" "$ACAP/hook.$pfx.log" 2>/dev/null
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$pfx" "$D_RC" \
        "$(sha "$ACAP/out/$pfx.stdout")" "$(stat -f %z "$ACAP/out/$pfx.stdout")" \
        "$(sha "$ACAP/out/$pfx.stderr")" "$(stat -f %z "$ACAP/out/$pfx.stderr")" \
        "$(sha "$ACAP/out/$pfx.tmuxtrace")" "$(wc -l <"$ACAP/out/$pfx.tmuxtrace" | tr -d ' ')" \
        "hooked:$hook" "$(printf '%q ' "$@")" >>"$ACAP/consumers.tsv"
    led barrier-consumer-COMPLETE "prefix=$pfx" "rc=$D_RC" \
        "stdout_sha256=$(sha "$ACAP/out/$pfx.stdout")" "stderr_sha256=$(sha "$ACAP/out/$pfx.stderr")" \
        "hook_log_sha256=$(sha "$ACAP/hook.$pfx.log")" \
        "flockspy_sha256=$(sha "$ACAP/out/$pfx.flockspy")"
    return 0
}

# d_plain_run <prefix> <ae-home> <sock> -- <argv...>  (unhooked, spied, recorded)
d_plain_run() {
    local pfx="$1" aehome="$2" sock="$3"; shift 3
    [[ "${1:-}" == "--" ]] && shift
    d_env "$aehome" "$sock" "$pfx"
    led plain-consumer-START "prefix=$pfx" "argv=$(printf '%q ' "$@")"
    local rc=0
    "${D_ENV[@]}" "$@" </dev/null >"$ACAP/out/$pfx.stdout" 2>"$ACAP/out/$pfx.stderr" || rc=$?
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$pfx" "$rc" \
        "$(sha "$ACAP/out/$pfx.stdout")" "$(stat -f %z "$ACAP/out/$pfx.stdout")" \
        "$(sha "$ACAP/out/$pfx.stderr")" "$(stat -f %z "$ACAP/out/$pfx.stderr")" \
        "$(sha "$ACAP/out/$pfx.tmuxtrace")" "$(wc -l <"$ACAP/out/$pfx.tmuxtrace" | tr -d ' ')" \
        "-" "$(printf '%q ' "$@")" >>"$ACAP/consumers.tsv"
    led plain-consumer-COMPLETE "prefix=$pfx" "rc=$rc" "stdout_sha256=$(sha "$ACAP/out/$pfx.stdout")" \
        "flockspy_sha256=$(sha "$ACAP/out/$pfx.flockspy")"
    return 0
}

# An xtrace TWIN: the same invocation run again with bash xtrace exported, so the call
# trace is a labelled SEPARATE run and the measured capture stays untraced.
d_trace_run() { # <prefix> <ae-home> <sock> -- <argv...>
    local pfx="$1" aehome="$2" sock="$3"; shift 3
    [[ "${1:-}" == "--" ]] && shift
    d_env "$aehome" "$sock" "$pfx.trace" "SHELLOPTS=xtrace"
    led trace-twin-START "prefix=$pfx" "note=separate run; the measured capture is untraced"
    "${D_ENV[@]}" "$@" </dev/null >"$ACAP/out/$pfx.trace.stdout" 2>"$ACAP/out/$pfx.trace.xtrace"
    led trace-twin-COMPLETE "prefix=$pfx" "xtrace_sha256=$(sha "$ACAP/out/$pfx.trace.xtrace")" \
        "xtrace_lines=$(wc -l <"$ACAP/out/$pfx.trace.xtrace" | tr -d ' ')"
}
