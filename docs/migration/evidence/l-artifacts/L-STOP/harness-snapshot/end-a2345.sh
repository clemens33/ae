#!/opt/homebrew/bin/bash
# L-END arms: archive-write-inability, claim, staging-modes,
# publication-crash-cuts. Each construction on its OWN sandbox.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

ALLB=b_confirm_answered:b_stop_local:b_stop_git:b_git_fixed:b_stage_mid:b_pre_rename:b_post_rename:b_pre_cleanup

# common: build a managed (--worktree) sandbox with a live session
setup() { # <arm> <sess> [binary]
    local arm="$1" sess="$2"
    local which="${3:-instrumented}"
    l_arm_begin L-END "$arm" "$which"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"
    l_ae 1launch --worktree "$sess"
    sleep 2
    WDIR="$R/h/.ae/worktrees/$sess"; [[ -d "$WDIR" ]] || WDIR="$R/w"
    l_arm_preflight "$sess" || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; return 1; }
    SESS_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/$sess/meta" | head -1 | cut -d= -f2-)"
    printf 'work\n' >"$WDIR/wip.txt"
    return 0
}

armtxt() { # <arm> <ids> <construction> <sess> [extra lines...]
    local arm="$1" ids="$2" con="$3" sess="$4"
    shift 4
    { printf 'arm\t%s\n' "$arm"; printf 'section\tL-END\n'; printf 'roster_ids\t%s\n' "$ids"
      printf 'fixture\tmanaged (--worktree, local bare file:// origin)\n'
      printf 'construction\t%s\n' "$con"
      printf 'session\t%s\n' "$sess"
      printf 'session_uuid\t%s\n' "${SESS_UUID:-<none>}"
      printf 'binary\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'hooks_active\t%s\n' "${HOOKS:-<none>}"
      printf 'launch_rc\t%s\n' "$(cat "$R/cap/1launch.rc" 2>/dev/null)"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

# ─────────────────────────────────────────── archive-write-inability (SC-516)
arm_inability() {
    setup archive-write-inability aw || { l_arm_end; return 1; }
    local ROOT="$R/h/.ae/archive"
    mkdir -p "$ROOT"
    l_manifest "$ROOT" "$R/cap/archive-root.pre-chmod.tsv"
    chmod 0500 "$ROOT"
    # INABILITY CANARY — recorded BEFORE the run
    l_canary_mkdir "$ROOT" "$R/cap/canary.txt"
    local crc=$?
    l_snap 0pre; l_events_snap aw 0pre
    if ((crc == 0)); then
        printf 'ARM INVALID: the inability canary SUCCEEDED; no observation taken.\n' >"$R/cap/ARM-INVALID.txt"
        chmod 0700 "$ROOT"; armtxt archive-write-inability SC-516 "archive root chmod 0500; canary SUCCEEDED" aw; l_arm_end; return 1
    fi
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2end end -f aw
    sleep 1
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.livedir.tsv"
    chmod 0700 "$ROOT"          # so the post manifest can be read
    l_snap 3post; l_events_snap aw 3post
    armtxt archive-write-inability SC-516 \
      "the archive root is made mode 0500 and the inability canary (mkdir under it) is recorded refusing, then end runs" aw \
      "end_rc	$(cat "$R/cap/2end.rc")" "canary_rc	$crc" \
      "note	the post-run archive-root manifest is taken after restoring mode 0700 so the tree is readable; the pre-restore mode is in canary.txt"
    l_arm_end
}

# ───────────────────────────────────────────────────── claim (SC-800, SC-803)
arm_claim() {
    setup claim cl || { l_arm_end; return 1; }
    local ROOT="$R/h/.ae/archive"
    mkdir -p "$ROOT"
    mkdir -m 0700 "$ROOT/.publishing.$SESS_UUID"
    l_manifest "$ROOT" "$R/cap/claimdir.before.tsv"
    l_snap 0pre; l_events_snap cl 0pre
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2end end -f cl
    sleep 1
    l_manifest "$ROOT" "$R/cap/claimdir.after.tsv"
    l_snap 3post; l_events_snap cl 3post
    od -c "$R/cap/2end.stdout" >"$R/cap/2end.stdout.od"
    od -c "$R/cap/2end.stderr" >"$R/cap/2end.stderr.od"
    armtxt claim "SC-800 SC-803" \
      "a .publishing.<uuid> directory for this session's own uuid is pre-created under the archive root (mode 0700), then end runs" cl \
      "end_rc	$(cat "$R/cap/2end.rc")" "planted_claim	$ROOT/.publishing.$SESS_UUID"
    l_arm_end
}

# ────────────────────────────────────────────────── staging-modes (SC-801)
STAGE_CB_MODE=""
stage_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_stage_mid.*)
        local p
        for p in "$R"/h/.ae/archive/.publishing.*/payload; do
            [[ -d "$p" ]] || continue
            l_manifest "$p" "$R/cap/$tag.staging-payload.tsv"
            l_manifest "$(dirname "$p")" "$R/cap/$tag.staging-claim.tsv"
            if [[ "$STAGE_CB_MODE" == plant ]]; then
                printf 'controller-planted unexpected entry\n' >"$p/UNEXPECTED.txt"
                mkdir -p "$p/unexpected.d"
                { printf 'planted.file\t%s\n' "$p/UNEXPECTED.txt"
                  printf 'planted.dir\t%s\n' "$p/unexpected.d"; } >"$R/cap/$tag.planted.txt"
                l_manifest "$p" "$R/cap/$tag.staging-payload.after-plant.tsv"
            fi
        done
        ;;
    esac
    return 0
}

arm_staging_modes() { # <mode: plain|plant>
    local mode="$1"
    local arm="staging-modes"
    [[ "$mode" == plant ]] && arm="staging-modes-planted-entry"
    setup "$arm" "sm${mode:0:2}" || { l_arm_end; return 1; }
    l_snap 0pre; l_events_snap "sm${mode:0:2}" 0pre
    STAGE_CB_MODE="$mode"
    HOOKS="$ALLB"; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2end end -f "sm${mode:0:2}"
    l_barriers "sm${mode:0:2}" 240 stage_cb
    local brc=$?
    wait "$AE_BG_PID"; printf '%s\n' "$?" >"$R/cap/2end.rc"
    (( brc != 0 )) && printf 'INCONCLUSIVE: barrier controller expired (bound 240s)\n' >>"$R/cap/barrier-order.tsv"
    sleep 1
    l_snap 3post; l_events_snap "sm${mode:0:2}" 3post
    # final published tree modes
    { for d in "$R"/h/.ae/archive/*/; do [[ -d "$d" ]] || continue; printf '=== %s ===\n' "$d"; done; } >"$R/cap/final-archive-dirs.txt"
    l_manifest "$R/h/.ae/archive" "$R/cap/final-archive.tsv"
    armtxt "$arm" SC-801 \
      "$( [[ "$mode" == plant ]] && echo 'the controller plants an unexpected file and directory into the staging payload at the mid-staging barrier' || echo 'no manipulation; the staging tree is captured at the mid-staging barrier and the published tree afterwards' )" \
      "sm${mode:0:2}" "end_rc	$(cat "$R/cap/2end.rc")" "barrier_bound_sec	240"
    l_arm_end
}

# ──────────────────────────────────────── publication-crash-cuts (SC-802)
CUT_AT=""
cut_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      "$CUT_AT".*)
        l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive-at-cut.tsv"
        l_manifest "$R/h/.ae" "$R/cap/$tag.aehome-at-cut.tsv"
        { printf 'cut.barrier\t%s\n' "$k"; printf 'cut.method\tSIGKILL to the whole end process tree\n'
          printf 'cut.pids\t%s\n' "$(ps -o pid=,ppid=,command= -ax | awk -v p="$AE_BG_PID" '$2==p||$1==p' | tr '\n' ';')"
        } >"$R/cap/$tag.cut.txt"
        l_killtree "$AE_BG_PID"
        ;;
    esac
    return 0
}

arm_crash_cut() { # <b_pre_rename|b_post_rename>
    local at="$1"
    local arm="publication-crash-cut-${at#b_}"
    setup "$arm" "pc${at:2:3}" || { l_arm_end; return 1; }
    l_snap 0pre; l_events_snap "pc${at:2:3}" 0pre
    CUT_AT="$at"
    HOOKS="$ALLB"; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2end end -f "pc${at:2:3}"
    l_barriers "pc${at:2:3}" 240 cut_cb
    local brc=$?
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2end.rc"
    (( brc != 0 )) && printf 'NOTE: controller loop ended by subject death or bound\n' >>"$R/cap/barrier-order.tsv"
    sleep 1
    l_snap 3post; l_events_snap "pc${at:2:3}" 3post
    l_manifest "$R/h/.ae/archive" "$R/cap/final-archive.tsv"
    armtxt "$arm" SC-802 \
      "the controller SIGKILLs the entire end process tree at the $at barrier" "pc${at:2:3}" \
      "end_rc	$(cat "$R/cap/2end.rc")" "cut_barrier	$at" "barrier_bound_sec	240"
    l_arm_end
    # hand the post-rename tree forward as the L-PURGE 'existing archive' specimen
    if [[ "$at" == b_post_rename ]]; then
        mkdir -p /tmp/aelx/specimens
        rm -rf /tmp/aelx/specimens/existing-archive
        cp -R "$R/h/.ae/archive" /tmp/aelx/specimens/existing-archive 2>/dev/null
        { printf 'specimen\texisting-archive\n'
          printf 'produced_by\tL-END publication-crash-cut-post_rename (product-produced, post-rename / pre-cleanup)\n'
          printf 'source_sandbox\t%s\n' "$R"
          printf 'session\tpc%s\n' "${at:2:3}"
          printf 'session_uuid\t%s\n' "$SESS_UUID"
        } >/tmp/aelx/specimens/existing-archive.provenance.txt
    fi
}

arm_staging_modes plain
arm_staging_modes plant
arm_crash_cut b_pre_rename
arm_crash_cut b_post_rename
echo DONE
