#!/opt/homebrew/bin/bash
# L-FROM: name-never-infers (SC-809), existing-target (SC-822),
# invalid-parent (SC-823), mid-publication (SC-824b), minted-at-end (SC-826).
set -uo pipefail
source /tmp/aelx/lib/from-lib.sh

# ── SC-809: the archive's source session name EQUALS the new session's name ──
arm_name_never_infers() {
    l_arm_begin L-FROM name-never-infers frozen
    PATCHV="none (frozen, unmodified)"
    make_parent same || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    fsnap 1pre; full_sweep 1pre
    # launch a NEW session under the SAME name, WITHOUT --from
    l_ae 2op --local same
    sleep 3
    lineage_fields same "$R/cap/child-lineage.txt"
    cp -p "$R/h/.ae/sessions/same/workspace.md" "$R/cap/child-workspace.md" 2>/dev/null
    fsnap 3post; full_sweep 3post
    farmtxt name-never-infers SC-809 \
      "a real parent archive is produced by a real end of a session named 'same'; a NEW session is then launched under that SAME name WITHOUT --from" \
      "parent_uuid	$PARENT_UUID" "op	ae --local same (no --from)" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-822: --from onto a target that already exists, three shapes ───────────
arm_existing_target() { # <running|stopped|worktree>
    local shape="$1"
    l_arm_begin L-FROM "existing-target-$shape" frozen
    PATCHV="none (frozen, unmodified)"
    make_parent par || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    case "$shape" in
      running)
        l_ae 1target --local tgt; sleep 3
        printf 'target.shape\ta RUNNING tmux session named tgt, launched for real\n' >"$R/cap/target.txt" ;;
      stopped)
        l_ae 1target --local tgt; sleep 3
        l_ae 1bstop stop -y tgt; sleep 2
        printf 'target.shape\ta STOPPED session: its state directory is on disk, its tmux session is gone\n' >"$R/cap/target.txt" ;;
      worktree)
        l_ae 1target --worktree tgt; sleep 3
        l_ae 1bstop stop -y tgt; sleep 2
        rm -rf "$R/h/.ae/sessions/tgt"
        printf 'target.shape\ta LEFTOVER WORKTREE: a real --worktree launch, stopped, then its session state directory removed by the controller so only the worktree remains\n' >"$R/cap/target.txt" ;;
    esac
    fsnap 1pre; full_sweep 1pre
    l_ae 2op --local tgt --from "$PARENT_UUID"
    sleep 3
    lineage_fields tgt "$R/cap/target-lineage.txt"
    fsnap 3post; full_sweep 3post
    diff -u "$R/cap/1pre.aehome.tsv" "$R/cap/3post.aehome.tsv" >"$R/cap/aehome.before-after.diff" 2>&1
    farmtxt "existing-target-$shape" SC-822 \
      "--from a real parent archive onto a name that already exists as $shape; full before/after manifests and a full sweep are captured on both sides" \
      "shape	$shape" "parent_uuid	$PARENT_UUID" \
      "op	ae --local tgt --from $PARENT_UUID" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-823: an invalid parent, two shapes ────────────────────────────────────
arm_invalid_parent() { # <nonexistent|validation-failing>
    local shape="$1"
    l_arm_begin L-FROM "invalid-parent-$shape" frozen
    PATCHV="none (frozen, unmodified)"
    make_parent par || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local TARGET_ID
    if [[ "$shape" == nonexistent ]]; then
        TARGET_ID="00000000-0000-4000-8000-000000000000"
        printf 'construction\ta well-formed archive uuid that names no archive in the root\n' >"$R/cap/mutation.txt"
    else
        TARGET_ID="$PARENT_UUID"
        local ARCH="$R/h/.ae/archive/$PARENT_UUID"
        cp -p "$ARCH/meta" "$R/cap/mutation.before.txt"
        l_rewrite_preserving_mode "$ARCH/meta" 's/^handover_count=.*$/handover_count=99/'
        cp -p "$ARCH/meta" "$R/cap/mutation.after.txt"
        diff -u "$R/cap/mutation.before.txt" "$R/cap/mutation.after.txt" >"$R/cap/mutation.diff" 2>&1
        { printf 'construction\tONE named mutation makes the real parent archive fail validation: the archive meta handover_count is set to 99 so meta and digest disagree, temp + chmod-to-original-mode + rename\n'
          printf 'mode.preserved\tyes\n'; } >"$R/cap/mutation.txt"
    fi
    fsnap 1pre; full_sweep 1pre
    l_ae 2op --local child --from "$TARGET_ID"
    sleep 3
    fsnap 3post; full_sweep 3post
    diff -u "$R/cap/1pre.full-sweep.txt" "$R/cap/3post.full-sweep.txt" >"$R/cap/full-sweep.diff" 2>&1
    lineage_fields child "$R/cap/child-lineage.txt"
    farmtxt "invalid-parent-$shape" SC-823 \
      "--from names $( [[ "$shape" == nonexistent ]] && echo 'a nonexistent archive id' || echo 'a real archive made to fail validation by ONE named mutation' ); after it, a FULL sweep of session dirs, worktrees, tmux sessions, the archive root, AE_HOME and the work dir is taken and diffed against the same sweep taken before" \
      "shape	$shape" "from_id	$TARGET_ID" \
      "op	ae --local child --from $TARGET_ID" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-824b: a claim standing on the parent ──────────────────────────────────
arm_mid_publication() {
    l_arm_begin L-FROM mid-publication frozen
    PATCHV="none (frozen, unmodified)"
    make_parent par || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local ROOT="$R/h/.ae/archive"
    l_manifest "$ROOT" "$R/cap/archive.before-plant.tsv"
    mkdir -m 0700 "$ROOT/.publishing.$PARENT_UUID"
    l_manifest "$ROOT" "$R/cap/archive.after-plant.tsv"
    diff -u "$R/cap/archive.before-plant.tsv" "$R/cap/archive.after-plant.tsv" >"$R/cap/plant.diff" 2>&1
    printf 'construction\ta .publishing.<parent-uuid> claim directory (mode 0700) is planted on the PARENT under the archive root, then --from runs against that parent\n' >"$R/cap/mutation.txt"
    fsnap 1pre; full_sweep 1pre
    l_ae 2op --local child --from "$PARENT_UUID"
    sleep 3
    fsnap 3post; full_sweep 3post
    lineage_fields child "$R/cap/child-lineage.txt"
    farmtxt mid-publication SC-824b \
      "a claim for the parent archive's own uuid is planted under the archive root, then --from runs against that parent" \
      "parent_uuid	$PARENT_UUID" "op	ae --local child --from $PARENT_UUID" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-826: a legacy-shaped session with NO session_id key ───────────────────
MINT_META_AT_BARRIER=""
mint_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_pre_cleanup.*)
        # the LIVE meta, captured before cleanup removes the directory
        cp -p "$R/h/.ae/sessions/leg/meta" "$R/cap/live-meta.at-pre-cleanup.txt" 2>/dev/null
        { printf '# the LIVE session meta at the pre-cleanup barrier, id keys BY EXACT KEY\n'
          grep -n '^session_id=\|^session_id_origin=' "$R/h/.ae/sessions/leg/meta" 2>&1
          printf '\n# every key containing the string origin, to show which is which\n'
          grep -n 'origin' "$R/h/.ae/sessions/leg/meta" 2>&1
        } >"$R/cap/live-meta-id-keys.txt" 2>&1
        l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.tsv"
        ;;
    esac
    return 0
}

arm_minted_at_end() {
    l_arm_begin L-FROM minted-at-end instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local leg
    sleep 3
    l_arm_preflight leg || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    local META="$R/h/.ae/sessions/leg/meta"
    cp -p "$META" "$R/cap/meta.before-mutation.txt"
    local m0; m0="$(stat -f '%Lp' "$META")"
    l_rewrite_preserving_mode "$META" '/^session_id=/d'
    cp -p "$META" "$R/cap/meta.after-mutation.txt"
    diff -u "$R/cap/meta.before-mutation.txt" "$R/cap/meta.after-mutation.txt" >"$R/cap/mutation.diff" 2>&1
    { printf 'mutation\tthe session_id KEY is REMOVED entirely from a real live session meta (legacy shape), temp + chmod-to-original-mode + rename\n'
      printf 'distinct_from\tSC-819 unparseable class, where the key is PRESENT and its value is unparseable\n'
      printf 'mode.before\t%s\nmode.after\t%s\n' "$m0" "$(stat -f '%Lp' "$META")"; } >"$R/cap/mutation.txt"
    fsnap 1pre
    HOOKS=b_pre_cleanup; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op end -f leg
    l_barriers leg 300 mint_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 2
    { printf '# the ARCHIVE meta id keys BY EXACT KEY\n'
      for m in "$R"/h/.ae/archive/*/meta; do [[ -e "$m" ]] || continue
        printf '=== %s ===\n' "$m"
        grep -n '^archive_id=\|^archive_id_origin=\|^source_session_id=\|^source_session=' "$m" 2>&1
        printf '\n# every key containing the string origin in this archive meta\n'
        grep -n 'origin' "$m" 2>&1
        printf '\n# NOTE the repository key origin= is a DIFFERENT key and is shown above only so the two are visibly distinct\n'
      done; } >"$R/cap/archive-meta-id-keys.txt" 2>&1
    fsnap 3post
    farmtxt minted-at-end SC-826 \
      "a real live session has its session_id KEY removed entirely (the legacy shape) by one named mutation, then end runs; the LIVE meta is captured at the pre-cleanup barrier and the ARCHIVE meta afterwards, both read BY EXACT KEY" \
      "op	ae end -f leg" "op_rc	$(cat "$R/cap/2op.rc")" \
      "live_meta_at_barrier	live-meta.at-pre-cleanup.txt and live-meta-id-keys.txt (session_id_origin)" \
      "archive_meta	archive-meta-id-keys.txt (archive_id_origin) — the repository key origin= is listed separately so the two are never confused"
    l_arm_end
}

case "${1:-all}" in
  a) arm_name_never_infers ;;
  b) arm_existing_target running; arm_existing_target stopped; arm_existing_target worktree ;;
  c) arm_invalid_parent nonexistent; arm_invalid_parent validation-failing ;;
  d) arm_mid_publication ;;
  e) arm_minted_at_end ;;
esac
echo DONE
