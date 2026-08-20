#!/opt/homebrew/bin/bash
# L-END arm: transaction-order (managed). Three constructed inputs, each on its
# OWN sandbox. Capture-only.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

ALLB=b_confirm_answered:b_stop_local:b_stop_git:b_git_fixed:b_stage_mid:b_pre_rename:b_post_rename:b_pre_cleanup

run_case() { # <case> <construction>
    local c="$1" construction="$2"
    l_arm_begin L-END "transaction-order-$c" instrumented
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    # construction (c): the origin remote is removed before the launch
    if [[ "$c" == "c-no-origin" ]]; then
        git -C "$R/w" remote remove origin >/dev/null 2>&1
    fi
    HOOKS=""; BLOCK=""
    l_arm_env
    AE_CWD="$R/w"
    l_ae 1launch --worktree "tx$c"
    sleep 2
    WDIR="$R/h/.ae/worktrees/tx$c"
    [[ -d "$WDIR" ]] || WDIR="$R/w"

    # BLOCKING environment preflight (cluster-plan.md environment-as-instrument)
    if ! l_arm_preflight "tx$c"; then
        printf 'PREFLIGHT-FAILED — no capture taken through this environment\n' >"$R/cap/ARM-INVALID.txt"
        l_arm_end; return 1
    fi
    # a dirty file so end's commit phase runs
    printf 'work in progress %s\n' "$c" >"$WDIR/wip.txt"
    l_snap 0pre
    l_events_snap "tx$c" 0pre

    # construction (b): git delegate-log-fail shim, failing ONLY push
    local -a envextra=()
    if [[ "$c" == "b-push-fails" ]]; then
        cp /tmp/aelx/lib/gitshim.sh "$R/b/git"; chmod 0755 "$R/b/git"
        envextra=("AE_L_GIT_FAIL=push" "AE_L_GIT_LOG=$R/cap/git-shim.log")
    fi
    HOOKS="$ALLB"; BLOCK=1; BLOCK_MAX=1800
    l_arm_env ${envextra[@]+"${envextra[@]}"}
    l_ae_bg 2end end -f "tx$c"
    l_barriers "tx$c" 240
    local brc=$?
    wait "$AE_BG_PID"; printf '%s\n' "$?" >"$R/cap/2end.rc"
    (( brc != 0 )) && printf 'INCONCLUSIVE: barrier controller expired (bound 240s)\n' >>"$R/cap/barrier-order.tsv"
    sleep 1
    l_snap 3post
    l_events_snap "tx$c" 3post
    # the recorded push-outcome field bytes, by exact key, from the published archive meta
    { for m in "$R"/h/.ae/archive/*/meta; do
        [[ -e "$m" ]] || continue
        printf '=== %s ===\n' "$m"
        grep -n '^git_\|^push\|^archive_id\|^source_session\|^session_id\|^preserved' "$m" 2>/dev/null
        printf '%s\n' '--- full meta bytes (od) ---'; od -c "$m" | head -60
      done; } >"$R/cap/archive-meta-fields.txt" 2>&1
    { printf 'arm\ttransaction-order-%s\n' "$c"
      printf 'section\tL-END\n'
      printf 'roster_ids\tSC-817\n'
      printf 'fixture\tmanaged (--worktree, local bare file:// origin)\n'
      printf 'construction\t%s\n' "$construction"
      printf 'binary\tinstrumented (hooks-only patch %s)\n' "$(l_sha /tmp/aelx/instr/ae)"
      printf 'hooks_active\t%s\n' "$ALLB"
      printf 'barrier_bound_sec\t240\n'
      printf 'launch_rc\t%s\n' "$(cat "$R/cap/1launch.rc")"
      printf 'end_rc\t%s\n' "$(cat "$R/cap/2end.rc")"
      printf 'workdir\t%s\n' "$WDIR"
    } >"$R/cap/ARM.txt"
    l_arm_end
    return 0
}

run_case a-full-run       "no manipulation; complete end on a managed session with a dirty tree"
run_case b-push-fails     "PATH git shim delegates every subcommand except push, which is logged and exits 128"
run_case c-no-origin      "the origin remote is removed from the repo before launch"
echo DONE
