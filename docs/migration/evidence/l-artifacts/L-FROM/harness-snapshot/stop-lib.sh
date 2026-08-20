#!/opt/homebrew/bin/bash
# L-STOP shared helpers.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

# Use the v2 instrumented copy where stop barriers are needed.
l_use_v2() { cp /tmp/aelx/instr2/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

# Build a fleet on ONE sandbox server. Every topology carries a PREFIX-SIBLING
# PAIR (proj / projx) so a name-prefix match is observable.
stop_fleet() { # <session...>
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    local s
    for s in "$@"; do l_ae "0launch-$s" --local "$s"; sleep 2; done
    return 0
}

sarmtxt() { # <arm> <ids> <construction> [extra...]
    local arm="$1" ids="$2" con="$3"; shift 3
    { printf 'arm\t%s\nsection\tL-STOP\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'construction\t%s\n' "$con"
      printf 'hook_patch_version\t%s\n' "${PATCHV:-none (frozen, unmodified)}"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'topology\t%s\n' "${TOPOLOGY:-<none>}"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

ssnap() { # <label>
    local l="$1"
    l_manifest "$R/h/.ae" "$R/cap/$l.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$l.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$l.tmux.txt"
    ps -ax -o pid=,ppid=,tty=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/$l.ps.txt" 2>&1
    local s
    for s in "$R"/h/.ae/sessions/*/events.jsonl; do
        [[ -e "$s" ]] || continue
        cp "$s" "$R/cap/$l.events.$(basename "$(dirname "$s")").jsonl"
    done
    return 0
}

# Open a SHELL pane inside a live ae session (the identity route's natural home).
open_shell_pane() { # <session>
    local sess="$1"
    /opt/homebrew/bin/tmux -S "$SOCK" new-window -d -t "$sess:" -c "$R/w" -P -F '#{pane_id}' 2>/dev/null
}

pane_send() { /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$1" -l -- "$2"; }
pane_enter() { /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$1" Enter; }
pane_cap() { /opt/homebrew/bin/tmux -S "$SOCK" capture-pane -p -t "$1" 2>&1; }

# Bounded positive barrier on a pane's rendered text.
pane_wait() { # <pane> <timeout-sec> <fixed-string>
    local p="$1" t="$2" s="$3" i=0
    while (( i < t*10 )); do
        pane_cap "$p" 2>/dev/null | grep -Fq -- "$s" && return 0
        sleep 0.1; i=$((i+1))
    done
    return 1
}
