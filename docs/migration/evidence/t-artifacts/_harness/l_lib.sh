#!/opt/homebrew/bin/bash
# Batch L harness library — capture-only. No verdicts, no expected-vs-actual.
# Every function here produces bytes; none interprets them.

if [[ -z "${BASH_VERSINFO[0]:-}" ]] || (( BASH_VERSINFO[0] < 4 )); then
    echo "l_lib: refusing to run under bash ${BASH_VERSION:-unknown} — use /opt/homebrew/bin/bash" >&2
    return 1 2>/dev/null || exit 1
fi
L_ROOT=/tmp/aelx
L_BASH=/opt/homebrew/bin/bash
L_TMUX=/opt/homebrew/bin/tmux
L_FROZEN=$L_ROOT/frozen

# ---------------------------------------------------------------- manifests
# Recursive manifest: rel-path <TAB> type <TAB> mode <TAB> nlink <TAB> size
#                     <TAB> symlink-target <TAB> sha256
# Deterministic order. Unreadable regular files record sha256=UNREADABLE.
l_manifest() { # <root> [out]
    local root="$1" out="${2:-/dev/stdout}"
    if [[ ! -e "$root" ]]; then printf 'ABSENT\t%s\n' "$root" >"$out"; return 0; fi
    {
        printf '# manifest-root\t%s\n' "$root"
        printf '#path\ttype\tmode\tnlink\tsize\tlink\tsha256\n'
        # -e on root itself too (mode of the root matters for the unwritable arms)
        find "$root" -mindepth 0 2>/dev/null | LC_ALL=C sort | while IFS= read -r p; do
            local rel="" type="" mode="" nlink="" size="" link="" hash=""
            rel="${p#"$root"}"; rel="${rel#/}"; [[ -z "$rel" ]] && rel="."
            if [[ -L "$p" ]]; then type=symlink
            elif [[ -d "$p" ]]; then type=dir
            elif [[ -p "$p" ]]; then type=fifo
            elif [[ -S "$p" ]]; then type=socket
            elif [[ -f "$p" ]]; then type=file
            else type=other; fi
            mode="$(stat -f '%Lp' "$p" 2>/dev/null || echo '?')"
            nlink="$(stat -f '%l' "$p" 2>/dev/null || echo '?')"
            size="$(stat -f '%z' "$p" 2>/dev/null || echo '?')"
            link='-'; [[ "$type" == symlink ]] && link="$(readlink "$p" 2>/dev/null || echo '?')"
            hash='-'
            if [[ "$type" == file ]]; then
                if [[ -r "$p" ]]; then hash="$(shasum -a 256 "$p" 2>/dev/null | awk '{print $1}')"
                else hash=UNREADABLE; fi
                [[ -n "$hash" ]] || hash=UNREADABLE
            fi
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$rel" "$type" "$mode" "$nlink" "$size" "$link" "$hash"
        done
    } >"$out"
    return 0
}

# ---------------------------------------------------------------- tmux snapshot
l_tmuxsnap() { # <socket> [out]
    local sock="$1" out="${2:-/dev/stdout}"
    {
        printf '## socket\t%s\n' "$sock"
        printf '## socket-exists\t%s\n' "$( [[ -S "$sock" ]] && echo yes || echo no )"
        printf '## sessions\n'
        "$L_TMUX" -S "$sock" list-sessions -F '#{session_id}|#{session_name}|#{session_windows}|#{session_attached}|#{session_created}' 2>&1 || printf '(rc=%s)\n' "$?"
        printf '## windows\n'
        "$L_TMUX" -S "$sock" list-windows -a -F '#{session_name}|#{window_id}|#{window_index}|#{window_name}|#{window_panes}' 2>&1 || printf '(rc=%s)\n' "$?"
        printf '## panes\n'
        "$L_TMUX" -S "$sock" list-panes -a -F '#{session_name}|#{window_id}|#{pane_id}|#{pane_pid}|#{pane_current_command}|#{@ae_agent}|#{@ae_slot}|#{pane_dead}' 2>&1 || printf '(rc=%s)\n' "$?"
        printf '## clients\n'
        "$L_TMUX" -S "$sock" list-clients -F '#{client_name}|#{client_session}|#{client_tty}' 2>&1 || printf '(rc=%s)\n' "$?"
    } >"$out" 2>&1
    return 0
}

# ------------------------------------------------- environment-as-instrument
# cluster-plan.md: pin a UTF-8 locale; prove the CONSUMER'S OWN tmux query
# round-trips a real TAB in THIS arm's environment BEFORE any capture.
# The consumer queries at 72c7293 that carry a literal TAB separator:
#   list-panes -s -t <s> -F '#{@ae_agent}\t#{pane_current_command}'   (ae:3631,4207)
#   list-panes -s -t <s> -F '#{pane_id}\t#{@ae_agent}'                (ae:6488,12151,12170,12297,12962)
#   list-sessions -F '#{session_id} #{session_name}'                  (space, not tab)
# Writes a PASS/FAIL byte record. Returns nonzero on failure (arm must not capture).
l_tab_preflight() { # <socket> <session> <out>
    local sock="$1" sess="$2" out="$3" rc=0
    local q1 q2 h1 h2
    q1="$("$L_TMUX" -S "$sock" list-panes -s -t "$sess" -F '#{@ae_agent}'$'\t''#{pane_current_command}' 2>&1)" || rc=1
    q2="$("$L_TMUX" -S "$sock" list-panes -s -t "$sess" -F '#{pane_id}'$'\t''#{@ae_agent}' 2>&1)" || rc=1
    h1="$(printf '%s' "$q1" | od -An -tx1 | tr -s ' ' | tr -d '\n')"
    h2="$(printf '%s' "$q2" | od -An -tx1 | tr -s ' ' | tr -d '\n')"
    {
        printf 'preflight\ttab-roundtrip\n'
        printf 'LANG\t%s\n' "${LANG:-<unset>}"
        printf 'LC_ALL\t%s\n' "${LC_ALL:-<unset>}"
        printf 'LC_CTYPE\t%s\n' "${LC_CTYPE:-<unset>}"
        printf 'query1\tlist-panes -s -t %s -F #{@ae_agent}<TAB>#{pane_current_command}\n' "$sess"
        printf 'query1.bytes\t%s\n' "$h1"
        printf 'query1.has_0x09\t%s\n' "$( [[ "$h1" == *" 09 "* || "$h1" == *" 09" ]] && echo yes || echo no )"
        printf 'query1.has_0x5f\t%s\n' "$( [[ "$h1" == *" 5f "* || "$h1" == *" 5f" ]] && echo yes || echo no )"
        printf 'query2\tlist-panes -s -t %s -F #{pane_id}<TAB>#{@ae_agent}\n' "$sess"
        printf 'query2.bytes\t%s\n' "$h2"
        printf 'query2.has_0x09\t%s\n' "$( [[ "$h2" == *" 09 "* || "$h2" == *" 09" ]] && echo yes || echo no )"
        printf 'query2.has_0x5f\t%s\n' "$( [[ "$h2" == *" 5f "* || "$h2" == *" 5f" ]] && echo yes || echo no )"
    } >"$out"
    # Gate: at least one 0x09 must survive in query2 (pane_id is always non-empty,
    # @ae_agent is set by a real launch, so the separator is observable).
    if [[ "$h2" == *" 09 "* || "$h2" == *" 09" ]]; then
        printf 'preflight.result\tTAB_0x09_OBSERVED\n' >>"$out"; return 0
    fi
    printf 'preflight.result\tTAB_0x09_NOT_OBSERVED\n' >>"$out"; return 1
}

# --------------------------------------------- per-consumer in-process trace
# Brief rule (e): effective AE_TMUX_SERVER / AE_TMUX_SERVER_KIND, type/declare -F
# tmux, and the delegated `command tmux` argv, observed FROM INSIDE the consumer.
# The delegated-argv half is supplied by the PATH tmux shim's log; this records
# the in-process half by sourcing the frozen ae's shim-install region in a
# subshell with the arm's environment.
l_consumer_trace() { # <out>
    local out="$1"
    {
        printf 'AE_TMUX_SERVER\t%s\n' "${AE_TMUX_SERVER:-<unset>}"
        printf 'AE_TMUX_SERVER_KIND\t%s\n' "${AE_TMUX_SERVER_KIND:-<unset>}"
        printf 'PATH\t%s\n' "$PATH"
        printf 'which-tmux\t%s\n' "$(command -v tmux || echo '<none>')"
    } >"$out"
    return 0
}

# ---------------------------------------------------------------- bounded poll
# Positive barrier with a recorded bound. On expiry the CALLER records
# INCONCLUSIVE — this returns 1 and never converts a timeout into an absence.
l_wait_for() { # <timeout-sec> <poll-sec> <predicate-cmd...>
    local t="$1" p="$2"; shift 2
    local waited=0
    while true; do
        if "$@" >/dev/null 2>&1; then return 0; fi
        # shellcheck disable=SC2016
        if (( $(printf '%.0f' "$(echo "$waited * 1000" | bc)") >= t*1000 )); then return 1; fi
        sleep "$p"
        waited="$(echo "$waited + $p" | bc)"
    done
}

l_wait_file() { # <timeout-sec> <path>
    local t="$1" f="$2" i=0
    while (( i < t*10 )); do [[ -e "$f" ]] && return 0; sleep 0.1; i=$((i+1)); done
    return 1
}

l_wait_grep() { # <timeout-sec> <file> <fixed-string>
    local t="$1" f="$2" s="$3" i=0
    while (( i < t*10 )); do
        [[ -f "$f" ]] && grep -Fq -- "$s" "$f" 2>/dev/null && return 0
        sleep 0.1; i=$((i+1))
    done
    return 1
}

l_sha() { shasum -a 256 "$1" 2>/dev/null | awk '{print $1}'; }
