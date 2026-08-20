#!/opt/homebrew/bin/bash
# T-100 harness: quiet-state vs pane-change, separated by who caused the change.
set -uo pipefail
source /tmp/aelx/lib/arm.sh
ARTROOT=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/t-artifacts

# ── the per-arm ledger, written BY the checks as they run ────────────────────
led() { # <checkpoint> <field> <value...>
    local cp="$1" f="$2"; shift 2
    printf '%s\t%s\t%s\t%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$cp" "$f" "$*" >>"$R/cap/ledger.tsv"
    return 0
}

# ── the frozen scrubber, taken from THIS session's own generated watchdog ────
# Never a reimplementation: the function body is extracted from the artifact that
# actually runs, and the session's own _lib is sourced for _ae_md5.
t_extract_scrubber() { # <session>
    local sess="$1"
    T_META="$R/h/.ae/sessions/$sess"
    T_SCRUB="$R/cap/extracted-watchdog-fns.sh"
    # LC_ALL=C for the EXTRACTION ONLY: the generated _lib carries a raw byte-class
    # regex that is not valid UTF-8, and awk aborts on it under a UTF-8 locale. This
    # is a byte-level read of a file, with no tmux and no product involved; the arm's
    # RUNTIME locale stays the pinned UTF-8 everywhere else.
    LC_ALL=C awk '/^_watchdog_quiet_hash \(\) $/,/^\}$/' "$T_META/watchdog" >"$T_SCRUB"
    LC_ALL=C awk '/^_watchdog_capture_pane \(\) $/,/^\}$/' "$T_META/watchdog" >>"$T_SCRUB"
    { printf 'source.generated_watchdog\t%s\n' "$T_META/watchdog"
      printf 'source.generated_lib\t%s\n' "$T_META/_lib"
      printf 'source.watchdog.sha256\t%s\n' "$(l_sha "$T_META/watchdog")"
      printf 'source.lib.sha256\t%s\n' "$(l_sha "$T_META/_lib")"
      printf 'extract.sha256\t%s\n' "$(l_sha "$T_SCRUB")"
      printf 'extract.functions\t%s\n' "$(LC_ALL=C grep -c '^[_a-z].* () $' "$T_SCRUB" || true)"
      printf 'md5.source\tthe session own generated _lib is SOURCED (not extracted), exactly as the watchdog prologue sources it, so _ae_md5 is the real one\n'
      printf 'note\tthe scrubber and the capture range are the bytes the RUNNING watchdog uses, extracted from the generated artifact — never a reimplementation\n'
      printf 'extraction.locale\tLC_ALL=C for the awk extraction only; the arm runtime locale is the pinned UTF-8\n'
    } >"$R/cap/scrubber-provenance.txt"
    LC_ALL=C grep -q '_watchdog_quiet_hash' "$T_SCRUB" || return 1
    return 0
}

# Hash a buffer with the extracted frozen scrubber, over the session's own _lib.
t_hash() { # <file-with-raw-bytes>
    local f="$1"
    "$L_BASH" -c '
        source "$1" >/dev/null 2>&1          # the REAL generated _lib: gives _ae_md5
        source "$2" >/dev/null 2>&1          # the two extracted watchdog functions
        _watchdog_quiet_hash "$(cat "$3")"
    ' _ "$T_META/_lib" "$T_SCRUB" "$f" 2>/dev/null
}

# Capture the pane with the watchdog's OWN range, raw bytes + od.
t_capture() { # <pane> <label>
    local pane="$1" lbl="$2"
    /opt/homebrew/bin/tmux -S "$SOCK" capture-pane -p -J -S -40 -E - -t "$pane" >"$R/cap/pane.$lbl.raw" 2>/dev/null
    od -c "$R/cap/pane.$lbl.raw" >"$R/cap/pane.$lbl.od"
    local h; h="$(t_hash "$R/cap/pane.$lbl.raw")"
    printf '%s\n' "$h" >"$R/cap/hash.$lbl.txt"
    led "$lbl" pane.raw.bytes "$(stat -f '%z' "$R/cap/pane.$lbl.raw" 2>/dev/null || echo -)"
    led "$lbl" pane.raw.sha256 "$(l_sha "$R/cap/pane.$lbl.raw")"
    led "$lbl" scrubbed.hash "$h"
    printf '%s' "$h"
    return 0
}

# events.jsonl timeline snapshot
t_events() { # <session> <label>
    local s="$1" lbl="$2"
    local f="$R/h/.ae/sessions/$s/events.jsonl"
    if [[ -f "$f" ]]; then cp "$f" "$R/cap/events.$lbl.jsonl"; else : >"$R/cap/events.$lbl.jsonl"; fi
    local _l _n
    _l="$(grep -c . "$R/cap/events.$lbl.jsonl" 2>/dev/null)"
    _n="$(grep -c '"action":"nudge"' "$R/cap/events.$lbl.jsonl" 2>/dev/null)"
    led "$lbl" events.lines "${_l:-0}"
    led "$lbl" events.nudges "${_n:-0}"
    return 0
}

# A pane whose @ae_agent/@ae_slot the real state helper accepts as the agent.
t_plant_agent_pane() { # <session>
    local sess="$1" row mainpane agent slot newp
    row="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t "=$sess" -F '#{pane_id} #{@ae_agent} #{@ae_slot}' | awk '$2!=""{print;exit}')"
    read -r mainpane agent slot <<<"$row"
    newp="$(/opt/homebrew/bin/tmux -S "$SOCK" new-window -d -t "$sess:" -c "$R/w" -P -F '#{pane_id}')"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_agent "$agent"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_slot "$slot"
    { printf 'plant.reason\tthe frozen state helper resolves the declaring agent from the CURRENT PANE; a controller process needs a pane it accepts\n'
      printf 'plant.source_pane\t%s (@ae_agent=%s @ae_slot=%s)\n' "$mainpane" "$agent" "$slot"
      printf 'plant.new_pane\t%s\n' "$newp"
      printf 'plant.window\ta SEPARATE window, so the AGENT pane under observation is never written to by the plant\n'
      printf 'plant.not_changed\tthe agent pane, its process, and every file under AE_HOME\n'
    } >"$R/cap/planted-pane.txt"
    T_AGENT_REF="$agent"; T_AGENT_PANE="$mainpane"
    printf '%s' "$newp"
}

t_pane_run() { # <pane> <label> <cmd-string>
    local p="$1" l="$2" c="$3"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" -l -- "$c > $R/cap/$l.stdout 2> $R/cap/$l.stderr; echo \$? > $R/cap/$l.rc"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" Enter
    printf '%s\n' "$c" >"$R/cap/$l.invocation"
    local i=0; while (( i < 300 )); do [[ -s "$R/cap/$l.rc" ]] && return 0; sleep 0.1; i=$((i+1)); done
    return 1
}

# A write that lands on the OBSERVED pane as pane OUTPUT, straight to its tty.
t_write_pane() { # <label> <text>
    local lbl="$1" text="$2"
    local tty; tty="$(cat "$R/cap/fake/"*.tty 2>/dev/null | head -1)"
    printf 'write.label\t%s\nwrite.tty\t%s\nwrite.text\t%s\n' "$lbl" "${tty:-<none>}" "$text" >>"$R/cap/writes.txt"
    [[ -n "$tty" && -w "$tty" ]] || { led "$lbl" write.FAILED "no writable tty for the observed pane"; return 1; }
    printf '%s\n' "$text" >"$tty"
    led "$lbl" write.tty "$tty"
    led "$lbl" write.text "$text"
    return 0
}
