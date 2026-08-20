#!/opt/homebrew/bin/bash
# T-WD precursor harness library. Value-blind: manipulation + barriers + capture only.
set -uo pipefail

FROZEN_SHA=72c729343a0117af2968b66e1c43f89ad25fc0b2
SCRATCH=/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad
FROZEN_AE="$SCRATCH/frozen/ae"
FAKE_BIN=/tmp/aecx/bin/aefake
HARNESS_BASH=/opt/homebrew/bin/bash

# --- sandbox construction ------------------------------------------------
# Separate export statements: single-statement export expansion binds the OLD
# HOME (AGENTS.md isolation footgun).
twd_sandbox() { # <arm-id>
    ARM="$1"
    ROOT="/tmp/aecx/twd/$ARM"
    rm -rf "$ROOT"
    mkdir -p "$ROOT"
    export HOME="$ROOT/home"
    export AE_HOME="$ROOT/home/.ae"
    export XDG_CONFIG_HOME="$ROOT/home/.config"
    mkdir -p "$HOME" "$AE_HOME" "$XDG_CONFIG_HOME" "$ROOT/work" "$ROOT/ctl" "$ROOT/cap"
    export TMPDIR="$ROOT/tmp"; mkdir -p "$TMPDIR"
    export TMUX_TMPDIR="$ROOT/tmuxtmp"; mkdir -p "$TMUX_TMPDIR"
    export SOCK="$ROOT/s.sock"
    export AE_TMUX_SERVER="$SOCK"
    export AE_TMUX_SERVER_KIND=socket
    export AEFAKE_LOG="$ROOT/ctl/agent-stdin.log"
    export AEFAKE_CTL="$ROOT/ctl/agent.ctl"
    export AEFAKE_BANNER="aefake-ready-fixed-prompt"
    export TZ=UTC
    export LANG=C
    export LC_ALL=C
    export SHELL=/bin/zsh
    export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    export AE_NO_AUTOSTART=1
    unset TMUX TMUX_PANE 2>/dev/null || true
    unset AE_WATCHDOG_IMPL 2>/dev/null || true
    mkfifo "$AEFAKE_CTL"
    : >"$AEFAKE_LOG"
    cat >"$AE_HOME/config" <<CFG
[agents]
fake = "$FAKE_BIN"

[workspace]
main = fake:probe
layout = vertical
watchdog = true
CFG
    ( cd "$ROOT/work" && git init -q . && git config user.email p@p && git config user.name p \
      && echo seed > seed.txt && git add -A && git commit -qm seed ) >/dev/null 2>&1
    CAP="$ROOT/cap"
    return 0
}

tm() { command tmux -S "$SOCK" "$@"; }

twd_teardown() {
    tm kill-server >/dev/null 2>&1 || true
    pkill -x aefake >/dev/null 2>&1 || true
}

# --- capture helpers (harness artifacts, segregated from product state) ---
cap_manifest() { # <label>
    local out="$CAP/manifest.$1.txt"
    ( cd "$AE_HOME" 2>/dev/null && find . -mindepth 1 -print0 2>/dev/null | sort -z |
      while IFS= read -r -d '' p; do
          local typ mode lnk hash
          if [[ -L "$p" ]]; then typ=link; lnk="$(readlink "$p")"; hash="-";
          elif [[ -d "$p" ]]; then typ=dir; lnk="-"; hash="-";
          else typ=file; lnk="-"; hash="$(shasum -a 256 "$p" 2>/dev/null | cut -d' ' -f1)"; [[ -n "$hash" ]] || hash="UNREADABLE"; fi
          mode="$(stat -f %Lp "$p" 2>/dev/null || echo '-')"
          printf '%s\t%s\t%s\t%s\t%s\n' "$typ" "$mode" "$hash" "$lnk" "$p"
      done ) >"$out" 2>/dev/null
    printf '%s' "$out"
}

cap_tmux() { # <label>
    local out="$CAP/tmux.$1.txt"
    {
        echo "## server-info"; tm display-message -p '#{version} pid=#{pid}' 2>&1
        echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}|#{session_attached}' 2>&1
        echo "## windows"; tm list-windows -a -F '#{session_name}|#{window_index}|#{window_name}|#{window_panes}' 2>&1
        echo "## panes"; tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}|#{pane_pid}|#{pane_dead}|#{pane_width}x#{pane_height}' 2>&1
        echo "## clients"; tm list-clients -F '#{client_name}|#{client_tty}|#{pane_id}|#{client_activity}' 2>&1
    } >"$out" 2>&1
    printf '%s' "$out"
}

cap_panes() { # <label>
    local out="$CAP/panes.$1.txt"
    : >"$out"
    while IFS='|' read -r pid ag; do
        { echo "## pane $pid agent=$ag  (capture-pane -p -J -S -40 -E -)";
          tm capture-pane -p -J -S -40 -E - -t "$pid" 2>&1; echo "## end $pid"; } >>"$out"
    done < <(tm list-panes -a -F '#{pane_id}|#{@ae_agent}' 2>/dev/null)
    printf '%s' "$out"
}

cap_events() { # <label>
    local out="$CAP/events.$1.jsonl"
    cp "$META/events.jsonl" "$out" 2>/dev/null || : >"$out"
    printf '%s' "$out"
}

cap_all() { # <label>
    cap_manifest "$1" >/dev/null; cap_tmux "$1" >/dev/null
    cap_panes "$1" >/dev/null; cap_events "$1" >/dev/null
    { echo "label=$1"; echo "epoch=$(date -u +%s)"; echo "utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)";
      echo "pgrep_aefake=$(pgrep -x aefake 2>/dev/null | tr '\n' ' ')";
      echo "events_lines=$(cat "$META/events.jsonl" 2>/dev/null | wc -l | tr -d ' ')";
      echo "events_bytes=$(stat -f %z "$META/events.jsonl" 2>/dev/null || echo 0)";
      echo "wd_log_bytes=$(stat -f %z "$CAP/watchdog.log" 2>/dev/null || echo 0)";
    } >"$CAP/stamp.$1.txt"
}

# Bounded wait for the events file to reach N lines. rc 0 reached, rc 3 timeout.
wait_events_lines() { # <n> <timeout-sec>
    local want="$1" tmo="$2" t0 now n
    t0="$(date -u +%s)"
    while :; do
        n="$(cat "$META/events.jsonl" 2>/dev/null | wc -l | tr -d ' ')"; n="${n:-0}"
        (( n >= want )) && return 0
        now="$(date -u +%s)"
        (( now - t0 >= tmo )) && return 3
        sleep 1
    done
}

# Bounded wait for N watchdog cycle-summary lines in the watchdog pane log.
# grep -c prints 0 AND exits 1 when there is no match, so `|| echo 0` appends a
# SECOND 0 and every arithmetic use of the result then errors. Digits only, once.
wd_cycles() {
    local n
    n="$(grep -c '── cycle ' "$CAP/watchdog.log" 2>/dev/null || true)"
    n="${n//[^0-9]/}"
    printf '%s' "${n:-0}"
}
wait_wd_cycles() { # <n> <timeout-sec>
    # RE-SNAPSHOTS EACH POLL. Polling a stale file is an instrument that can only
    # report timeout — it must read the live producer pane, not a frozen copy.
    local want="$1" tmo="$2" t0 now n
    t0="$(date -u +%s)"
    while :; do
        wd_snapshot
        n="$(wd_cycles)"
        (( n >= want )) && return 0
        now="$(date -u +%s)"
        (( now - t0 >= tmo )) && return 3
        sleep 1
    done
}

wd_snapshot() { tm capture-pane -p -J -S -2000 -E - -t "$WD_PANE" >"$CAP/watchdog.log" 2>/dev/null || true; }
