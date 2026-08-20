#!/opt/homebrew/bin/bash
# L-FROM: transport-cut (SC-824a) and lineage-durability (SC-825a/b/c).
set -uo pipefail
source /tmp/aelx/lib/from-lib.sh

parent_read_source_trace() {
    local out="$R/cap/source-trace.parent-archive-reads-after-the-barrier.txt"
    local first
    first="$(grep -n '_from_proof="\$(_ar_from_preflight "\$FROM_ARCHIVE")"' /tmp/aelx/frozen/ae | head -1 | cut -d: -f1)"
    { printf '%s\n' 'FROZEN SOURCE, extracted verbatim with line numbers. A code observation, not a verdict.'
      printf 'frozen commit\t72c729343a0117af2968b66e1c43f89ad25fc0b2\n'
      printf 'first parent proof at ae:%s — the barrier b_from_proved sits immediately after it parses\n\n' "${first:-?}"
      printf '%s\n' '=== every line AFTER that point that names PARENT_ARCHIVE_ID, FROM_ARCHIVE, _ar_from_preflight, _AE_FROM_EXPECTED or the archive root ==='
      awk -v s="${first:-0}" 'NR>s && (/PARENT_ARCHIVE_ID/ || /FROM_ARCHIVE/ || /_ar_from_preflight/ || /_AE_FROM_EXPECTED/ || /_ar_root/ || /_ar_require_real_root/) {printf "%d:%s\n", NR, $0}' /tmp/aelx/frozen/ae
      printf '\n%s\n' '=== _ar_from_preflight in full (the read it performs) ==='
      awk '/^_ar_from_preflight\(\) \{/,/^\}$/' /tmp/aelx/frozen/ae
    } >"$out"
    printf 'source-trace.sha256\t%s\n' "$(l_sha "$out")" >"$R/cap/source-trace.sha256.txt"
    return 0
}

CUT_DONE=""
cut_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_from_proved.*)
        [[ -n "$CUT_DONE" ]] && return 0
        CUT_DONE=yes
        l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive.before-delete.tsv"
        rm -rf "$R/h/.ae/archive/$PARENT_UUID"
        l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive.after-delete.tsv"
        diff -u "$R/cap/$tag.archive.before-delete.tsv" "$R/cap/$tag.archive.after-delete.tsv" >"$R/cap/$tag.archive.delete.diff" 2>&1
        { printf 'controller.barrier\t%s\n' "$k"
          printf 'controller.action\trm -rf the PARENT archive directory, after the launch has parsed its first parent proof and before it publishes the child meta\n'
          printf 'controller.target\t%s\n' "$R/h/.ae/archive/$PARENT_UUID"
          printf 'path.is.plain.launch\tthis is ae <new> --local <name> --from <uuid>; no compact ran, so no _AE_FROM_EXPECTED is carried in\n'
        } >"$R/cap/$tag.controller.txt"
        l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.tsv"
        ;;
    esac
    return 0
}

arm_transport_cut() {
    l_arm_begin L-FROM transport-cut instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    make_parent par || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    parent_read_source_trace
    fsnap 1pre; full_sweep 1pre
    HOOKS=b_from_proved; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op --local child --from "$PARENT_UUID"
    l_barriers child 300 cut_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    lineage_fields child "$R/cap/child-lineage.txt"
    fsnap 3post; full_sweep 3post
    farmtxt transport-cut "SC-824a" \
      "the PLAIN launch path: ae --local child --from <parent>. An instrumented barrier sits immediately after the launch parses its FIRST parent proof; the controller deletes the parent archive there and releases, and the launch resumes" \
      "barrier	b_from_proved" "parent_uuid	$PARENT_UUID" \
      "op	ae --local child --from $PARENT_UUID" "op_rc	$(cat "$R/cap/2op.rc")" \
      "source_trace	source-trace.parent-archive-reads-after-the-barrier.txt (every frozen line after the first proof that names the parent archive, with line numbers, plus _ar_from_preflight in full)" \
      "ref_only	SC-808 re-proof surface is L-END compact-relaunch-lock-parent-mutated; it is REFERENCED here and NOT re-executed. This path carries no re-proof expectation and no rollback machinery of its own" \
      "barrier_bound_sec	300"
    l_arm_end
}

arm_lineage_durability() { # <stop-resume|move-aehome|delete-parent>
    local mode="$1"
    l_arm_begin L-FROM "lineage-durability-$mode" frozen
    PATCHV="none (frozen, unmodified)"
    make_parent par || { printf 'FIXTURE-INVALID\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    l_ae 1child --local child --from "$PARENT_UUID"
    sleep 3
    [[ -f "$R/h/.ae/sessions/child/meta" ]] || { printf 'FIXTURE-INVALID: the --from child was not created\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    lineage_fields child "$R/cap/lineage.1after-from.txt"
    cp -p "$R/h/.ae/sessions/child/meta" "$R/cap/meta.1after-from.txt"
    cp -p "$R/h/.ae/sessions/child/workspace.md" "$R/cap/workspace.1after-from.md" 2>/dev/null
    fsnap 1pre
    l_ae 2stop stop -y child
    sleep 2
    lineage_fields child "$R/cap/lineage.2after-stop.txt"
    case "$mode" in
      move-aehome)
        local NEW="$R/h2/.ae"
        mkdir -p "$R/h2"
        cp -Rp "$R/h/.ae" "$NEW"
        { printf 'manipulation\tthe WHOLE AE_HOME is moved: a mode-preserving copy at a new absolute path, and the resume runs with AE_HOME pointing there\n'
          printf 'from\t%s\nto\t%s\n' "$R/h/.ae" "$NEW"; } >"$R/cap/manipulation.txt"
        l_manifest "$NEW" "$R/cap/moved-aehome.tsv"
        HOOKS=""; BLOCK=""
        AE_ENV=(); mapfile -t AE_ENV < <(l_env "$R" "AE_HOME=$NEW" "AE_L_TMUX_LOG=$R/cap/tmux-argv.log")
        ;;
      delete-parent)
        l_manifest "$R/h/.ae/archive" "$R/cap/archive.before-delete.tsv"
        rm -rf "$R/h/.ae/archive/$PARENT_UUID"
        l_manifest "$R/h/.ae/archive" "$R/cap/archive.after-delete.tsv"
        diff -u "$R/cap/archive.before-delete.tsv" "$R/cap/archive.after-delete.tsv" >"$R/cap/archive.delete.diff" 2>&1
        printf 'manipulation\tthe PARENT archive directory is removed while the child is stopped, then the child is resumed\n' >"$R/cap/manipulation.txt"
        HOOKS=""; BLOCK=""; l_arm_env
        ;;
      *) printf 'manipulation\tnone: a plain stop then resume cycle\n' >"$R/cap/manipulation.txt"
         HOOKS=""; BLOCK=""; l_arm_env ;;
    esac
    l_ae 3resume --local child
    sleep 3
    local SD="$R/h/.ae/sessions"
    [[ "$mode" == move-aehome ]] && SD="$R/h2/.ae/sessions"
    { printf '# lineage fields BY EXACT KEY after the resume, from %s/child/meta\n' "$SD"
      if [[ -f "$SD/child/meta" ]]; then
        grep -n '^session_id=\|^session_id_origin=\|^parent_archive_id=\|^parent_archive_handover_count=\|^parent_archive_pending_count=\|^session=\|^origin=' "$SD/child/meta" 2>&1
        printf '\n# the whole meta, verbatim\n'; cat "$SD/child/meta"
        printf '\n# od of the parent_archive_id line\n'; grep '^parent_archive_id=' "$SD/child/meta" | od -c
      else printf '(no meta at that path)\n'; fi
      printf '\n# workspace.md lineage lines\n'
      if [[ -f "$SD/child/workspace.md" ]]; then grep -n -i 'archive\|lineage\|--from\|continues\|parent' "$SD/child/workspace.md" 2>&1 || printf '(no matching line)\n'
      else printf '(no workspace.md)\n'; fi
    } >"$R/cap/lineage.3after-resume.txt" 2>&1
    cp -p "$SD/child/workspace.md" "$R/cap/workspace.3after-resume.md" 2>/dev/null
    diff -u "$R/cap/lineage.1after-from.txt" "$R/cap/lineage.3after-resume.txt" >"$R/cap/lineage.across-cycle.diff" 2>&1
    fsnap 3post; full_sweep 3post
    farmtxt "lineage-durability-$mode" \
      "$( case "$mode" in stop-resume) echo 'SC-825a' ;; move-aehome) echo 'SC-825b' ;; delete-parent) echo 'SC-825c' ;; esac )" \
      "a successful --from child of a real parent archive is stopped and then resumed$( case "$mode" in
          move-aehome) echo ', with the WHOLE AE_HOME moved to a new absolute path in between and the resume run against that path' ;;
          delete-parent) echo ', with the PARENT archive removed in between' ;;
          *) echo ' with no manipulation in between' ;; esac )" \
      "mode	$mode" "parent_uuid	$PARENT_UUID" \
      "child_launch_rc	$(cat "$R/cap/1child.rc")" "stop_rc	$(cat "$R/cap/2stop.rc")" \
      "resume_rc	$(cat "$R/cap/3resume.rc")" \
      "lineage_captures	lineage.1after-from.txt, lineage.2after-stop.txt, lineage.3after-resume.txt, lineage.across-cycle.diff"
    l_arm_end
}

case "${1:-all}" in
  cut) arm_transport_cut ;;
  d1)  arm_lineage_durability stop-resume ;;
  d2)  arm_lineage_durability move-aehome ;;
  d3)  arm_lineage_durability delete-parent ;;
esac
echo DONE
