#!/opt/homebrew/bin/bash
# Batch H fixture builder. Pre-registered with the arms that use it.
#
# Producer-derivation: every helper byte the arms exercise is written by a REAL frozen `ae`
# launch into an isolated AE_HOME. Nothing here hand-writes a helper.
set -uo pipefail
HROOT=/tmp/aecx/h

h_sandbox() { # <id> <main> <workers-csv-or-empty>
    HID="$1"; local main="$2" workers="${3:-}"
    ROOT="$HROOT/$HID"; rm -rf "$ROOT"; mkdir -p "$ROOT"
    export HOME="$ROOT/home" AE_HOME="$ROOT/home/.ae" XDG_CONFIG_HOME="$ROOT/home/.config"
    mkdir -p "$HOME" "$AE_HOME" "$XDG_CONFIG_HOME" "$ROOT/work" "$ROOT/cap" "$ROOT/ctl"
    export TMPDIR="$ROOT/tmp"; mkdir -p "$TMPDIR"
    export TMUX_TMPDIR="$ROOT/tmuxtmp"; mkdir -p "$TMUX_TMPDIR"
    export SOCK="$ROOT/s.sock" AE_TMUX_SERVER="$ROOT/s.sock" AE_TMUX_SERVER_KIND=socket
    export AEFAKE_LOG="$ROOT/ctl/agent-stdin.log" AEFAKE_BANNER="aefake ❯ ready"
    export AEFAKE_CTL_DIR="$ROOT/ctl"
    # UTF-8 deliberately: a non-UTF-8 locale makes tmux sanitise the TAB in -F output and
    # silently corrupts the tab-separated pane queries the product itself runs (A1).
    export TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 SHELL=/bin/zsh
    export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    export AE_NO_AUTOSTART=1
    unset TMUX TMUX_PANE 2>/dev/null || true
    : >"$AEFAKE_LOG"
    { echo "[agents]"
      echo "cl = \"$FAKE_BIN\""
      echo "cx = \"$FAKE_BIN\""
      echo "zz = \"$FAKE_BIN\""
      echo
      echo "[workspace]"
      echo "main = $main"
      [[ -n "$workers" ]] && echo "workers = $workers"
      echo "layout = vertical"
      echo "watchdog = false"; } >"$AE_HOME/config"
    ( cd "$ROOT/work" && git init -q . && git config user.email p@p && git config user.name p \
      && echo seed >seed.txt && git add -A && git commit -qm seed ) >/dev/null 2>&1
    return 0
}

h_launch() { # <session-name>
    HSESSION="$1"
    ( cd "$ROOT/work" && "$HARNESS_BASH" "$FROZEN_AE" --local "$HSESSION" </dev/null \
        >"$ROOT/cap/launch.$HSESSION.out" 2>"$ROOT/cap/launch.$HSESSION.err" )
    HMETA="$AE_HOME/sessions/$HSESSION"
    [[ -f "$HMETA/meta" ]] || return 1
    HSRV_PID="$(command tmux -S "$SOCK" display-message -p '#{pid}' 2>/dev/null)"
    return 0
}

h_pane_of() { # <agent-ref>
    command tmux -S "$SOCK" list-panes -a -F '#{@ae_agent}|#{pane_id}' 2>/dev/null \
      | awk -F'|' -v a="$1" '$1==a{print $2; exit}'
}

h_roster() { grep '^agent\.' "$HMETA/meta"; }
