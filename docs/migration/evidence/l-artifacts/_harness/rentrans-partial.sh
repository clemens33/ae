#!/opt/homebrew/bin/bash
# L-RENTRANS PARTIAL (transport-free subset, run under a BLOCKED transport gate
# on the lead's explicit ruling): rename-effects (SC-832a), rename-observer
# (SC-1303) and the transport-free cells of SC-1302's same-name matrix.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

l_use_v4() { cp /tmp/aelx/instr4/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

TOPOLOGY="proj + projx (prefix-sibling pair), one recorded server"

rsnap() { # <label>
    local l="$1"
    l_manifest "$R/h/.ae" "$R/cap/$l.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$l.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$l.tmux.txt"
    { printf '# tmux server liveness\n'
      printf 'server.pid\t%s\n' "$(/opt/homebrew/bin/tmux -S "$SOCK" display-message -p '#{pid}' 2>&1)"
      printf 'socket.exists\t%s\n' "$( [[ -S "$SOCK" ]] && echo yes || echo no )"
    } >"$R/cap/$l.server.txt" 2>&1
    local s
    for s in "$R"/h/.ae/sessions/*/; do
        [[ -d "$s" ]] || continue
        local n; n="$(basename "$s")"
        cp -p "$s/meta" "$R/cap/$l.meta.$n.txt" 2>/dev/null
        od -c "$s/meta" >"$R/cap/$l.meta.$n.od" 2>/dev/null
        cp -p "$s/workspace.md" "$R/cap/$l.workspace.$n.md" 2>/dev/null
    done
    return 0
}

rarmtxt() { # <arm> <ids> <construction> [extra...]
    local arm="$1" ids="$2" con="$3"; shift 3
    { printf 'arm\t%s\nsection\tL-RENTRANS (PARTIAL — transport-free subset under a BLOCKED transport gate)\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'construction\t%s\n' "$con"
      printf 'transport\tNONE — this arm touches neither ssh nor rsync\n'
      printf 'hook_patch_version\t%s\n' "${PATCHV:-none (frozen, unmodified)}"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'topology\t%s\n' "$TOPOLOGY"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

setup_fleet() { # <session...>
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    local s
    for s in "$@"; do l_ae "0launch-$s" --local "$s"; sleep 2; done
    return 0
}

# ───────────────────────────────────────────── SC-832a: rename-effects
arm_rename_effects() {
    l_arm_begin L-RENTRANS rename-effects frozen
    PATCHV="none (frozen, unmodified)"
    setup_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    rsnap 1pre
    : >"$R/cap/tmux-argv.log"
    l_ae 2op rename proj proj2
    sleep 2
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    rsnap 3post
    diff -u "$R/cap/1pre.sessions.tsv" "$R/cap/3post.sessions.tsv" >"$R/cap/sessions.before-after.diff" 2>&1
    diff -u "$R/cap/1pre.meta.proj.txt" "$R/cap/3post.meta.proj2.txt" >"$R/cap/meta.before-after.diff" 2>&1
    diff -u "$R/cap/1pre.workspace.proj.md" "$R/cap/3post.workspace.proj2.md" >"$R/cap/workspace.before-after.diff" 2>&1
    diff -u "$R/cap/1pre.server.txt" "$R/cap/3post.server.txt" >"$R/cap/server.before-after.diff" 2>&1
    rarmtxt rename-effects SC-832a \
      "a REAL ae rename on a RUNNING server, over a topology carrying the prefix-sibling pair proj/projx: ae rename proj proj2" \
      "op	ae rename proj proj2" "op_rc	$(cat "$R/cap/2op.rc")" \
      "captures	tmux state, session-dir manifests, meta bytes (verbatim + od) for every session, workspace.md, tmux server liveness — each before and after, each diffed"
    l_arm_end
}

# ─────────────────────────────────────────── SC-1303: rename-observer
observe_rename() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    local -a e=(); mapfile -t e < <(l_env "$R")
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" list --json ) >"$R/cap/$tag.observer.list.stdout" 2>"$R/cap/$tag.observer.list.stderr"
    printf '%s\n' "$?" >"$R/cap/$tag.observer.list.rc"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$tag.tmux.txt"
    { for s in "$R"/h/.ae/sessions/*/meta; do [[ -e "$s" ]] || continue
        printf '=== %s ===\n' "$s"; grep -n '^session=' "$s"; done; } >"$R/cap/$tag.meta-session-key.txt" 2>&1
    return 0
}

arm_rename_observer() {
    l_arm_begin L-RENTRANS rename-observer instrumented
    l_use_v4; PATCHV="L-HOOKS-v4"
    setup_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    rsnap 1pre
    HOOKS=b_rn_locked_entry:b_rn_tmux_renamed:b_rn_dir_moved:b_rn_meta_updated
    BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op rename proj proj2
    l_barriers proj 300 observe_rename || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 2
    rsnap 3post
    { printf '# the census-named cut points, in the order they fired\n'
      cat "$R/cap/barrier-order.tsv" 2>/dev/null
      printf '\n# legend (site, not meaning)\n'
      printf 'b_rn_locked_entry\tinside the two-lock region, before any check the rename then mutates\n'
      printf 'b_rn_tmux_renamed\tafter tmux rename-session (and the main-window rename), before the directory move\n'
      printf 'b_rn_dir_moved\tafter the state directory move, before the meta rewrite\n'
      printf 'b_rn_meta_updated\tafter session= is rewritten, before workspace.md is regenerated\n'
    } >"$R/cap/cut-points.txt"
    rarmtxt rename-observer SC-1303 \
      "a REAL ae rename held at each census-named cut point in turn; at every cut a concurrent ae list --json runs from a SEPARATE process, and the sessions manifest, the tmux snapshot and every meta session= key are captured" \
      "op	ae rename proj proj2" "op_rc	$(cat "$R/cap/2op.rc")" \
      "cut_points	b_rn_locked_entry, b_rn_tmux_renamed, b_rn_dir_moved, b_rn_meta_updated" \
      "barrier_bound_sec	300"
    l_arm_end
}

# ───────────────────── SC-1302: the transport-free cells of the same-name matrix
arm_matrix_cell() { # <first: stop|rename> <flock: with|without>
    local first="$1" fl="$2"
    local arm="samename-matrix-${first}-first-flock-${fl}"
    l_arm_begin L-RENTRANS "$arm" instrumented
    l_use_v4; PATCHV="L-HOOKS-v4"
    setup_fleet proj projx
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    cp /tmp/aelx/lib/flockshim.sh "$R/b/flock"; chmod 0755 "$R/b/flock"
    # "flock removed from PATH": a bin dir with symlinks to the tools ae needs and
    # NO flock at all, and /opt/homebrew/bin dropped from PATH entirely.
    mkdir -p "$R/nb"
    local t
    for t in tmux git bash; do ln -sf "/opt/homebrew/bin/$t" "$R/nb/$t" 2>/dev/null; done
    local -a extra=("AE_L_FLOCK_LOG=$R/cap/flock-spy.log")
    if [[ "$fl" == without ]]; then
        rm -f "$R/b/flock"
        AE_ENV=(); mapfile -t AE_ENV < <(printf '%s\n' \
            "HOME=$R/h" "AE_HOME=$R/h/.ae" "TMPDIR=$R/tp" "TMUX_TMPDIR=$R/t" \
            "PATH=$R/b:$R/nb:/usr/bin:/bin:/usr/sbin:/sbin" \
            "TERM=xterm-256color" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TZ=UTC" \
            "SHELL=/opt/homebrew/bin/bash" "AE_FAKE_TOOL=fake" "AE_FAKE_LOG_DIR=$R/cap/fake" \
            "AE_L_TMUX_LOG=$R/cap/tmux-argv.log" \
            "AE_L_HOOKS=b_rn_locked_entry:b_stop_one_pre_kill" "AE_L_TRACE=$R/cap/hook-trace.tsv" \
            "AE_L_BLOCK=$R/ctl" "AE_L_BLOCK_MAX=1800")
        { printf 'flock.availability\tREMOVED from PATH\n'
          printf 'method\t/opt/homebrew/bin dropped from PATH entirely; a sandbox bin dir holds symlinks to tmux, git and bash ONLY, so command -v flock fails\n'
          printf 'PATH\t%s\n' "$R/b:$R/nb:/usr/bin:/bin:/usr/sbin:/sbin"
          printf 'command -v flock under that PATH\t%s\n' "$(PATH="$R/b:$R/nb:/usr/bin:/bin:/usr/sbin:/sbin" command -v flock || echo '<not found>')"
        } >"$R/cap/flock-availability.txt"
    else
        HOOKS=b_rn_locked_entry:b_stop_one_pre_kill; BLOCK=1; BLOCK_MAX=1800
        l_arm_env ${extra[@]+"${extra[@]}"}
        { printf 'flock.availability\tPRESENT, through the delegate-and-log spy\n'
          printf 'spy\t%s (delegates every invocation to %s, substitutes nothing)\n' "$R/b/flock" "$(command -v flock)"
          printf 'command -v flock\t%s\n' "$R/b/flock"; } >"$R/cap/flock-availability.txt"
    fi
    rsnap 1pre
    : >"$R/cap/tmux-argv.log"; : >"$R/cap/flock-spy.log"
    local FIRSTBAR SECONDCMD
    if [[ "$first" == rename ]]; then
        FIRSTBAR=b_rn_locked_entry
        l_ae_bg 2first rename proj proj2
    else
        FIRSTBAR=b_stop_one_pre_kill
        l_ae_bg 2first stop -y proj
    fi
    local FP=$AE_BG_PID
    # bounded POSITIVE barrier on the first operation
    local i=0 k=""
    while (( i < 600 )); do
        for f in "$R"/ctl/"$FIRSTBAR".*.reached; do [[ -e "$f" ]] && { k="$(basename "$f" .reached)"; break; }; done
        [[ -n "$k" ]] && break
        sleep 0.1; i=$((i+1))
    done
    [[ -z "$k" ]] && printf 'INCONCLUSIVE: the first operation did not reach %s within the 60s bound\n' "$FIRSTBAR" >"$R/cap/INCONCLUSIVE.txt"
    printf 'first.barrier\t%s\n' "${k:-<none>}" >"$R/cap/interleave.txt"
    l_tmuxsnap "$SOCK" "$R/cap/at-first-barrier.tmux.txt"
    l_manifest "$R/h/.ae/sessions" "$R/cap/at-first-barrier.sessions.tsv"
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/at-first-barrier.ps.txt" 2>&1
    # the SECOND operation on the SAME name, while the first is held
    if [[ "$first" == rename ]]; then
        printf 'second.op\tae stop -y proj (the SAME name, while the rename is held at its barrier)\n' >>"$R/cap/interleave.txt"
        l_ae_bg 3second stop -y proj
    else
        printf 'second.op\tae rename proj proj2 (the SAME name, while the stop is held at its barrier)\n' >>"$R/cap/interleave.txt"
        l_ae_bg 3second rename proj proj2
    fi
    local SP=$AE_BG_PID
    sleep 8
    l_tmuxsnap "$SOCK" "$R/cap/both-in-flight.tmux.txt"
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/both-in-flight.ps.txt" 2>&1
    cp "$R/cap/flock-spy.log" "$R/cap/flock-spy.at-interleave.log" 2>/dev/null
    # release everything
    local f2; for f2 in "$R"/ctl/*.reached; do [[ -e "$f2" ]] || continue; : >"$R/ctl/$(basename "$f2" .reached).release"; done
    sleep 1
    for f2 in "$R"/ctl/*.reached; do [[ -e "$f2" ]] || continue; : >"$R/ctl/$(basename "$f2" .reached).release"; done
    wait "$FP" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2first.rc"
    wait "$SP" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/3second.rc"
    sleep 4
    cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"
    cp "$R/cap/flock-spy.log" "$R/cap/flock-spy.final.log" 2>/dev/null
    rsnap 3post
    { printf '# final state\n'; printf '## tmux sessions\n'
      /opt/homebrew/bin/tmux -S "$SOCK" list-sessions -F '#{session_id}|#{session_name}' 2>&1
      printf '\n## session dirs\n'; ls -1 "$R/h/.ae/sessions" 2>&1
      printf '\n## meta session= per dir\n'
      for s in "$R"/h/.ae/sessions/*/meta; do [[ -e "$s" ]] || continue; printf '%s: ' "$s"; grep '^session=' "$s"; done
    } >"$R/cap/final-state.txt" 2>&1
    rarmtxt "$arm" SC-1302 \
      "the SAME-NAME concurrency matrix, transport-free cell: the ordered pair ($first first, then the other) raced on ONE name under controller barriers, with flock $fl" \
      "ordered_pair	$first -> $( [[ "$first" == rename ]] && echo stop || echo rename )" \
      "flock	$fl" "first_rc	$(cat "$R/cap/2first.rc")" "second_rc	$(cat "$R/cap/3second.rc")" \
      "first_barrier	$FIRSTBAR" "barrier_bound_sec	60" \
      "lock_trace	$( [[ "$fl" == with ]] && echo 'flock-spy.at-interleave.log and flock-spy.final.log' || echo 'none — flock is absent from PATH in this cell; flock-availability.txt records the PATH and the command -v result' )"
    l_arm_end
}

case "${1:-all}" in
  eff) arm_rename_effects ;;
  obs) arm_rename_observer ;;
  m1)  arm_matrix_cell rename with ;;
  m2)  arm_matrix_cell rename without ;;
  m3)  arm_matrix_cell stop with ;;
  m4)  arm_matrix_cell stop without ;;
esac
echo DONE
