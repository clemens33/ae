#!/opt/homebrew/bin/bash
# L-END arm: compact-relaunch-lock (SC-807, SC-808).
set -uo pipefail
source /tmp/aelx/lib/arm.sh

MUT=""
cp_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    cp "$R/cap/flock-spy.log" "$R/cap/$tag.flock-spy.snapshot.log" 2>/dev/null
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/$tag.ps.txt" 2>&1
    l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.tsv"
    case "$k" in
      b_from_proved.*)
        [[ "$MUT" != mutate ]] && return 0
        local m
        for m in "$R"/h/.ae/archive/*/meta; do
            [[ -e "$m" ]] || continue
            cp "$m" "$R/cap/$tag.parent-meta.before.txt"
            local h; h="$(grep '^handover_count=' "$m" | head -1 | cut -d= -f2-)"
            local n=$(( ${h:-0} + 7 ))
            sed "s/^handover_count=${h}\$/handover_count=${n}/" "$m" >"$m.tmp.$$" && mv "$m.tmp.$$" "$m"
            cp "$m" "$R/cap/$tag.parent-meta.after.txt"
            diff -u "$R/cap/$tag.parent-meta.before.txt" "$R/cap/$tag.parent-meta.after.txt" >"$R/cap/$tag.parent-meta.diff" 2>&1
            { printf 'controller.action\tparent archive meta handover_count %s -> %s (temp+rename)\n' "${h:-<absent>}" "$n"
              printf 'controller.barrier\t%s\n' "$k"
              printf 'controller.target\t%s\n' "$m"; } >"$R/cap/$tag.controller.txt"
        done
        ;;
    esac
    return 0
}

run() { # <arm> <control|mutate>
    local arm="$1" mode="$2"
    MUT="$mode"
    l_arm_begin L-END "$arm" instrumented
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    cp /tmp/aelx/lib/flockshim.sh "$R/b/flock"; chmod 0755 "$R/b/flock"
    HOOKS=""; BLOCK=""; l_arm_env "AE_L_FLOCK_LOG=$R/cap/flock-spy.log"
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 1launch --local cr1
    sleep 2
    l_arm_preflight cr1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    cp "$R/h/.ae/sessions/cr1/meta" "$R/cap/meta.parent-session.txt"
    PARENT_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/cr1/meta" | head -1 | cut -d= -f2-)"
    l_snap 0pre
    : >"$R/cap/flock-spy.log"
    HOOKS=b_cp_after_answer:b_cp_after_handover:b_cp_pre_relaunch:b_from_proved:b_pre_rename:b_post_rename:b_pre_cleanup
    BLOCK=1; BLOCK_MAX=1800
    l_arm_env "AE_L_FLOCK_LOG=$R/cap/flock-spy.log"
    l_ae_bg 2compact compact -f --digest-only cr1
    l_barriers cr1 300 cp_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >>"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2compact.rc"
    sleep 2
    l_snap 3post
    cp "$R/h/.ae/sessions/cr1/meta" "$R/cap/meta.child-session.txt" 2>/dev/null || printf '(no child session meta)\n' >"$R/cap/meta.child-session.txt"
    { printf '# lineage fields in the child meta, by exact key\n'
      grep -n '^session_id=\|^session_id_origin=\|^parent_archive_id=\|^parent_archive_handover\|^parent_archive_pending' "$R/cap/meta.child-session.txt" 2>&1
      printf '\n# archive dirs\n'; ls -1 "$R/h/.ae/archive" 2>&1
      printf '\n# session dirs\n'; ls -1 "$R/h/.ae/sessions" 2>&1
    } >"$R/cap/lineage.txt" 2>&1
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    { printf 'arm\t%s\nsection\tL-END\n' "$arm"
      printf 'roster_ids\t%s\n' "$( [[ "$mode" == mutate ]] && echo 'SC-808' || echo 'SC-807' )"
      printf 'fixture\t--local family, real compact with --digest-only -f\n'
      printf 'construction\t%s\n' "$( [[ "$mode" == mutate ]] && echo 'at the barrier after the child launch parses its FIRST parent-archive proof, the controller rewrites the parent archive meta handover_count (temp+rename) before releasing' || echo 'no mutation; a delegate-and-log flock spy records every lock invocation across compact and the relaunch it execs into' )"
      printf 'flock_spy\tPATH shim delegating every invocation to %s\n' "$(command -v flock)"
      printf 'parent_uuid\t%s\n' "${PARENT_UUID:-<none>}"
      printf 'compact_rc\t%s\n' "$(cat "$R/cap/2compact.rc")"
      printf 'barrier_bound_sec\t300\n'
    } >"$R/cap/ARM.txt"
    l_arm_end
}

run compact-relaunch-lock-control control
run compact-relaunch-lock-parent-mutated mutate
echo DONE
