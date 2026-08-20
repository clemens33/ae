#!/opt/homebrew/bin/bash
# Template-build library: real ae launches at the frozen commit, real generated
# helpers as the producers. Value-blind: manipulation + capture only.
set -uo pipefail

FROZEN_SHA=72c729343a0117af2968b66e1c43f89ad25fc0b2
SCRATCH=/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad
FROZEN_AE="$SCRATCH/frozen/ae"
FAKE_BIN=/tmp/aecx/bin/aefake
HARNESS_BASH=/opt/homebrew/bin/bash
TROOT=/tmp/aecx/tpl          # template build sandboxes
TSTORE=/tmp/aecx/templates   # immutable, chmod-protected template groups

# <sandbox-id> <workers-line-or-empty>
t_sandbox() {
    TID="$1"; WORKERS="${2:-}"
    ROOT="$TROOT/$TID"
    rm -rf "$ROOT"; mkdir -p "$ROOT"
    export HOME="$ROOT/home"
    export AE_HOME="$ROOT/home/.ae"
    export XDG_CONFIG_HOME="$ROOT/home/.config"
    mkdir -p "$HOME" "$AE_HOME" "$XDG_CONFIG_HOME" "$ROOT/work" "$ROOT/cap" "$ROOT/ctl"
    export TMPDIR="$ROOT/tmp"; mkdir -p "$TMPDIR"
    export TMUX_TMPDIR="$ROOT/tmuxtmp"; mkdir -p "$TMUX_TMPDIR"
    export SOCK="$ROOT/s.sock"
    export AE_TMUX_SERVER="$SOCK"
    export AE_TMUX_SERVER_KIND=socket
    export AEFAKE_LOG="$ROOT/ctl/agent-stdin.log"
    export AEFAKE_BANNER="aefake ❯ ready"   # contains the composed-UI marker the frozen unknown-tool readiness predicate greps for
    unset AEFAKE_CTL 2>/dev/null || true
    export TZ=UTC; export LANG=C; export LC_ALL=C
    export SHELL=/bin/zsh
    export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    export AE_NO_AUTOSTART=1
    unset TMUX TMUX_PANE AE_WATCHDOG_IMPL 2>/dev/null || true
    : >"$AEFAKE_LOG"
    {
        echo "[agents]"
        echo "fake = \"$FAKE_BIN\""
        echo
        echo "[workspace]"
        echo "main = fake:lead"
        [[ -n "$WORKERS" ]] && echo "workers = $WORKERS"
        echo "layout = vertical"
        echo "watchdog = ${T_WATCHDOG:-false}"
    } >"$AE_HOME/config"
    ( cd "$ROOT/work" && git init -q . && git config user.email p@p && git config user.name p \
      && echo seed > seed.txt && git add -A && git commit -qm seed ) >/dev/null 2>&1
    CAP="$ROOT/cap"
    return 0
}

tm() { command tmux -S "$SOCK" "$@"; }

t_launch() { # <session-name>
    TSESSION="$1"
    cd "$ROOT/work" || return 1
    T_LAUNCH_RC=0
    "$HARNESS_BASH" "$FROZEN_AE" --local "$TSESSION" </dev/null \
        >"$CAP/ae-launch.out" 2>"$CAP/ae-launch.err" || T_LAUNCH_RC=$?
    META="$AE_HOME/sessions/$TSESSION"
    export META TSESSION
    [[ -f "$META/meta" ]] || return 1
    SRV_PID="$(tm display-message -p '#{pid}')"
    export SRV_PID
    return 0
}

# Run a generated helper AS a specific pane's agent: the real helper, with the
# real pane environment a live agent has (TMUX + TMUX_PANE), so actor identity and
# routing keys are produced by the product, not asserted by the harness.
as_agent() { # <pane-id> <helper> [args...]
    local pane="$1"; shift
    local h="$1"; shift
    env TMUX="${SOCK},${SRV_PID},0" TMUX_PANE="$pane" "$META/$h" "$@"
}

pane_of() { # <alias:name>
    tm list-panes -a -F '#{pane_id} #{@ae_agent}' | awk -v w="$1" '$2==w{print $1;exit}'
}

t_teardown() { tm kill-server >/dev/null 2>&1 || true; pkill -x aefake >/dev/null 2>&1 || true; }

# Recursive fingerprint of a directory: type, mode, symlink target, content hash.
dir_manifest() { # <dir>
    ( cd "$1" && find . -mindepth 1 -print0 | sort -z |
      while IFS= read -r -d '' p; do
          local typ mode lnk hash
          if [[ -L "$p" ]]; then typ=link; lnk="$(readlink "$p")"; hash="-"
          elif [[ -d "$p" ]]; then typ=dir; lnk="-"; hash="-"
          else typ=file; lnk="-"; hash="$(shasum -a 256 "$p" 2>/dev/null | cut -d' ' -f1)"; [[ -n "$hash" ]] || hash="UNREADABLE"; fi
          mode="$(stat -f %Lp "$p" 2>/dev/null || echo '-')"
          printf '%s\t%s\t%s\t%s\t%s\n' "$typ" "$mode" "$hash" "$lnk" "$p"
      done )
}
dir_fingerprint() { dir_manifest "$1" | shasum -a 256 | cut -d' ' -f1; }

# --- template store -------------------------------------------------------
# A template MEMBER is a snapshot of a whole AE_HOME (config + sessions/...), so a
# clone is a working AE_HOME. The pre-protection manifest records every path's
# ORIGINAL mode, so a writable clone can restore exactly what the producer wrote.
t_store() { # <group> <member> <provenance-text-file-or-->
    local grp="$1" mem="$2" prov="${3:--}"
    local dst="$TSTORE/$grp/$mem"
    rm -rf "$dst"; mkdir -p "$TSTORE/$grp/_meta"
    mkdir -p "$(dirname "$dst")"
    cp -R "$AE_HOME" "$dst"
    # original modes, recorded BEFORE protection
    dir_manifest "$dst" >"$TSTORE/$grp/_meta/$mem.modes.tsv"
    local fp; fp="$(dir_fingerprint "$dst")"
    {
        echo "group=$grp"
        echo "member=$mem"
        echo "fingerprint_pre_protection=$fp"
        echo "source_sandbox=$ROOT"
        echo "source_session=${TSESSION:-}"
        echo "frozen_sha=$FROZEN_SHA"
        echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
        [[ "$prov" != "-" && -f "$prov" ]] && { echo "--- provenance ---"; cat "$prov"; }
    } >"$TSTORE/$grp/_meta/$mem.txt"
    printf '%s' "$fp"
}

t_protect() { # <group> <member>
    local dst="$TSTORE/$1/$2"
    chmod -R a-w "$dst" 2>/dev/null || true
    local fp; fp="$(dir_fingerprint "$dst")"
    echo "fingerprint_protected=$fp" >>"$TSTORE/$1/_meta/$2.txt"
    printf '%s' "$fp"
}
