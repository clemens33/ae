#!/opt/homebrew/bin/bash
# SC-832c — POST-CRASH RE-ENTRY after a rename interruption.
# The writer is KILLED at each cut, so the lifecycle descriptors are released BY
# DEATH and never by a resumed process; a FRESH PROCESS is then the only reader.
set -uo pipefail
source /tmp/aelx/lib/arm.sh
ARTROOT=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/l-artifacts

l_use_v4() { cp /tmp/aelx/instr4/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

led() { local cp="$1" f="$2"; shift 2
    printf '%s\t%s\t%s\t%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$cp" "$f" "$*" >>"$R/cap/ledger.tsv"; return 0; }

c_config() {
    { printf '[agents]\n'; printf 'grok = "grok %s/b/fake-tool.sh"\n' "$R"
      printf '\n[workspace]\nmain = grok\nlayout = vertical\n'; } >"$R/h/.ae/config"
    cp /tmp/aelx/lib/t100-fake.sh "$R/b/fake-tool.sh"; return 0
}

# THE IDENTITY TUPLE, read from every place identity lives, at one instant.
# Nothing here interprets whether the parts agree; it records what each says.
c_tuple() { # <label> [ae_home]
    local lbl="$1" H="${2:-$R/h/.ae}"
    local out="$R/cap/tuple.$lbl.txt"
    { printf '# identity tuple at %s (AE_HOME=%s)\n' "$lbl" "$H"
      printf '\n## tmux session names on the recorded server\n'
      /opt/homebrew/bin/tmux -S "$SOCK" list-sessions -F '#{session_id}|#{session_name}' 2>&1
      printf '\n## state directory names under sessions/\n'
      ls -1 "$H/sessions" 2>&1 | grep -v '^\.lifecycle' || true
      printf '\n## per state directory: meta session= , session_id= , and workspace.md\n'
      local d n
      for d in "$H"/sessions/*/; do
        [[ -d "$d" ]] || continue
        n="$(basename "$d")"
        printf -- '--- dir: %s\n' "$n"
        printf 'meta.session=      %s\n' "$(grep -m1 '^session=' "$d/meta" 2>/dev/null | cut -d= -f2- || echo '<no meta>')"
        printf 'meta.session_id=   %s\n' "$(grep -m1 '^session_id=' "$d/meta" 2>/dev/null | cut -d= -f2- || echo '<none>')"
        printf 'meta.tmux_server=  %s\n' "$(grep -m1 '^tmux_server=' "$d/meta" 2>/dev/null | cut -d= -f2- || echo '<none>')"
        printf 'workspace.md       %s\n' "$( [[ -f "$d/workspace.md" ]] && echo present || echo ABSENT )"
        printf 'workspace.session  %s\n' "$(grep -m1 '^Session:' "$d/workspace.md" 2>/dev/null || echo '<no Session: line>')"
      done
      printf '\n## lifecycle lock files present\n'
      ls -1 "$H/sessions"/.lifecycle.*.lock 2>/dev/null || printf '(none)\n'
    } >"$out" 2>&1
    # a compact machine-readable row for the generated table
    { printf 'label\t%s\n' "$lbl"
      printf 'tmux.names\t%s\n' "$(/opt/homebrew/bin/tmux -S "$SOCK" list-sessions -F '#{session_name}' 2>/dev/null | sort | paste -sd, -)"
      printf 'dir.names\t%s\n' "$(ls -1 "$H/sessions" 2>/dev/null | grep -v '^\.lifecycle' | sort | paste -sd, -)"
      printf 'meta.sessions\t%s\n' "$(for d in "$H"/sessions/*/; do [[ -d "$d" ]] && grep -m1 '^session=' "$d/meta" 2>/dev/null | cut -d= -f2-; done | sort | paste -sd, -)"
      printf 'session_ids\t%s\n' "$(for d in "$H"/sessions/*/; do [[ -d "$d" ]] && grep -m1 '^session_id=' "$d/meta" 2>/dev/null | cut -d= -f2- | cut -c1-8; done | sort | paste -sd, -)"
    } >"$R/cap/tuple.$lbl.row"
    return 0
}

# DEATH AND DESCRIPTOR PROOF. Both are preconditions for the fresh process, and
# both are written by the check itself.
c_prove_dead_and_free() { # <pid>
    local pid="$1" out="$R/cap/death-and-descriptors.txt" ok=1
    local alive; kill -0 "$pid" 2>/dev/null && alive=yes || alive=no
    { printf 'subject.pid\t%s\n' "$pid"
      printf 'subject.alive\t%s\n' "$alive"
      printf 'any.remaining.sandbox.process\n'
      ps -ax -o pid=,ppid=,command= 2>/dev/null | grep -F "$R/b/ae" | grep -v '[g]rep' || printf '  (none)\n'
    } >"$out"
    [[ "$alive" == no ]] || ok=0
    local lk
    for lk in "$R"/h/.ae/sessions/.lifecycle.*.lock; do
        [[ -e "$lk" ]] || continue
        local rc=0
        ( flock -w 2 9 || exit 1 ) 9>"$lk" || rc=1
        printf 'lock.reacquirable\t%s\t%s\n' "$(basename "$lk")" "$( ((rc == 0)) && echo YES || echo NO )" >>"$out"
        (( rc == 0 )) || ok=0
    done
    printf 'precondition.met\t%s\n' "$( ((ok)) && echo YES || echo NO )" >>"$out"
    led precondition death.and.descriptors "$( ((ok)) && echo MET || echo NOT_MET )"
    (( ok ))
}

# An INDEPENDENT post-crash clone of AE_HOME for one consumer.
c_clone() { # <name> -> prints the clone's AE_HOME
    local n="$1"
    local c="$R/clones/$n"
    mkdir -p "$R/clones"; rm -rf "$c"; mkdir -p "$c"
    cp -Rp "$R/h/.ae" "$c/.ae"
    printf '%s' "$c/.ae"
}

c_run_fresh() { # <ae_home> <label> <args...>
    local H="$1" lbl="$2"; shift 2
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_HOME=$H")
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" "$@" ) >"$R/cap/$lbl.stdout" 2>"$R/cap/$lbl.stderr"
    printf '%s\n' "$?" >"$R/cap/$lbl.rc"
    { printf 'FRESH PROCESS. cwd: %s\nAE_HOME: %s\nargv:\n' "$R/w" "$H"; printf '  %s\n' "$R/b/ae" "$@"; } >"$R/cap/$lbl.invocation"
    led consumer "$lbl.rc" "$(cat "$R/cap/$lbl.rc")"
    return 0
}
