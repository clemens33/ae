#!/usr/bin/env python3
"""Batch L hook patch generator — HOOK-ONLY.

Produces an instrumented copy of the frozen 72c7293 `ae`.  The hook function
_H() is a no-op returning 0 when AE_L_HOOKS is unset; when active it appends a
trace line to a harness file OUTSIDE any product path and optionally blocks on
a release file.  It never reads, hashes or computes over product state.

Each insertion is anchored on a UNIQUE literal; a non-unique or missing anchor
aborts the build.
"""
import sys, hashlib

SRC, DST = sys.argv[1], sys.argv[2]
src = open(SRC, 'r', encoding='utf-8').read()

HOOK_FN = r'''
# ─── BATCH-L INSTRUMENTATION (hook-only; inert unless AE_L_HOOKS is set) ───────
# Blocks or emits a barrier ordinal. Never reads, hashes or computes over
# product state. Writes only to harness paths (AE_L_TRACE / AE_L_BLOCK).
_H() {
    [[ -n "${AE_L_HOOKS:-}" ]] || return 0
    case ":${AE_L_HOOKS}:" in
        *":$1:"*) ;;
        *) return 0 ;;
    esac
    _AE_L_N=$((${_AE_L_N:-0} + 1))
    if [[ -n "${AE_L_TRACE:-}" ]]; then
        printf '%s\t%s\t%s\t%s\n' "$_AE_L_N" "$1" "$$" "${EPOCHREALTIME:-0}" >>"$AE_L_TRACE" 2>/dev/null || true
    fi
    if [[ -n "${AE_L_BLOCK:-}" ]]; then
        local _k="$1.${BASHPID:-$$}.${_AE_L_N}"
        : >"${AE_L_BLOCK}/${_k}.reached" 2>/dev/null || true
        local _i=0
        while [[ ! -e "${AE_L_BLOCK}/${_k}.release" ]]; do
            sleep 0.1
            _i=$((_i + 1))
            ((_i > ${AE_L_BLOCK_MAX:-1200})) && break
        done
    fi
    return 0
}
# ─── END BATCH-L INSTRUMENTATION ──────────────────────────────────────────────
'''

def insert_after(text, anchor, payload, label):
    n = text.count(anchor)
    if n != 1:
        sys.exit("ANCHOR %s occurs %d times (need exactly 1):\n%r" % (label, n, anchor))
    i = text.index(anchor) + len(anchor)
    return text[:i] + payload + text[i:]

def insert_before(text, anchor, payload, label):
    n = text.count(anchor)
    if n != 1:
        sys.exit("ANCHOR %s occurs %d times (need exactly 1):\n%r" % (label, n, anchor))
    i = text.index(anchor)
    return text[:i] + payload + text[i:]

# 1. the hook function itself, right after the AE_HOME resolution block
out = insert_after(src,
    'AE_HOME="${AE_HOME:-${HOME}/.ae}"\n',
    HOOK_FN, 'HOOK_FN')

# 2. cmd_end: after the confirmation answer is accepted (SC-820a window)
out = insert_after(out,
    '''        if [[ ! "$reply" =~ ^[Yy]$ ]]; then
            echo "Aborted."
            return 0
        fi
    fi
''', '    _H b_confirm_answered\n', 'b_confirm_answered')

# 3. _end_session_locked, local branch: after the verified kill
out = insert_after(out,
    '''            _lifecycle_kill_verified "$session_name" end "$_es_sid" || return 1
        fi
''', '        _H b_stop_local\n', 'b_stop_local')

# 4. _end_session_locked, git branch: after the verified kill
out = insert_after(out,
    '''        _lifecycle_kill_verified "$session_name" end "$_es_sid" || return 1
        _es_sid=""
    fi
''', '    _H b_stop_git\n', 'b_stop_git')

# 5/6. before each archive step whose git outcome is now fixed
out = insert_before(out,
    '        _end_archive_step "$session_name" "$_es_push_out" "$_es_push_ref" "$wdir" "$wdir" || return 1\n',
    '        _H b_git_fixed\n', 'b_git_fixed_preserve')
out = insert_before(out,
    '    _end_archive_step "$session_name" "$_es_push_out" "$_es_push_ref" "-" "$wdir" || return 1\n',
    '    _H b_git_fixed\n', 'b_git_fixed_normal')

# 7/8/9. before each cleanup_session in _end_session_locked
out = insert_before(out,
    '        cleanup_session "$session_name" "$mode" "$origin"\n        echo "Ended local session $session_name"\n',
    '        _H b_pre_cleanup\n', 'b_pre_cleanup_local')
out = insert_before(out,
    '        cleanup_session "$session_name" "$mode" "$origin" preserve\n',
    '        _H b_pre_cleanup\n', 'b_pre_cleanup_preserve')
out = insert_before(out,
    '    cleanup_session "$session_name" "$mode" "$origin"\n    echo "Ended $session_name"\n',
    '    _H b_pre_cleanup\n', 'b_pre_cleanup_normal')

# 10. _ar_stage_payload: mid-staging, after payload copies, before the digest render
out = insert_before(out,
    '    _ar_render_digest "${payload}/meta" "${payload}/memo.tsv" "${payload}/events.jsonl" \\\n',
    '    _H b_stage_mid\n', 'b_stage_mid')

# 11. _ar_publish: after validation and the target recheck, immediately before the rename
out = insert_before(out,
    '    if ! mv "$payload" "$target"; then\n',
    '    _H b_pre_rename\n', 'b_pre_rename')

# 12. _ar_publish: immediately after the rename succeeded
out = insert_after(out,
    '''    if ! mv "$payload" "$target"; then
        rm -rf "$claim"
        echo "archive: could not publish ${target}." >&2
        return 1
    fi
''', '    _H b_post_rename\n', 'b_post_rename')

# 13. launch: after the FIRST parent-archive proof is parsed
out = insert_after(out,
    '    IFS=$\'\\t\' read -r PARENT_ARCHIVE_ID PARENT_ARCHIVE_HANDOVER PARENT_ARCHIVE_PENDING <<<"$_from_proof"\n',
    '    _H b_from_proved\n', 'b_from_proved')

# 14. cmd_compact: after the human answer is accepted
out = insert_after(out,
    '''        if [[ ! "$reply" =~ ^[Yy]$ ]]; then
            echo "Aborted." >&2
            return 0
        fi
    fi
''', '    _H b_cp_after_answer\n', 'b_cp_after_answer')

# 15. cmd_compact: after the handover completed, before phase (c) teardown
out = insert_after(out,
    '        echo "compact: handover complete (reply ${ref} and a new handover memo)." >&2\n    fi\n',
    '    _H b_cp_after_handover\n', 'b_cp_after_handover')

# 16. cmd_compact: immediately before the exec into the relaunch
out = insert_before(out,
    '    exec env "${launch_env[@]}" "$(_ae_own_path)" "$mode_flag" "$c_name" --from "$c_uuid"\n',
    '    _H b_cp_pre_relaunch\n', 'b_cp_pre_relaunch')

# ─── v2 additions: stop-path barriers ────────────────────────────────────────
# 17. the detached fleet supervisor, after its opid validation and before it acts
out = insert_after(out,
    '    _stop_opid_valid "$opid" || return 1\n',
    '    _H b_stop_supervisor_entry\n', 'b_stop_supervisor_entry')

# 18. the caller, immediately before it waits on the durable per-target records
out = insert_before(out,
    '                _stop_fleet_await "$_opid" "$sessions" 30 || _all_rc=1\n',
    '                _H b_stop_before_await\n', 'b_stop_before_await')

# 19. a singular stop, under the lifecycle lock, before the kill
out = insert_before(out,
    '        if [[ "$expect_set" == true ]]; then\n            _stop_session_locked "$name" "$expect_sid"\n',
    '        _H b_stop_one_pre_kill\n', 'b_stop_one_pre_kill')

# ─── v3 additions: compact trace channels (NAMED, one per site) ──────────────
# 20. the RESOLVER ENTRY — the tuple-freeze site, distinct from any revalidation
out = insert_after(out,
    '_compact_freeze_source() { # <name> <keep-history:true|false>\n',
    '    _H b_cp_resolver_entry\n', 'b_cp_resolver_entry')

# 21/22. the TWO revalidation sites, separately named
out = insert_before(out,
    '        _compact_revalidate "$c_name" "$tuple" "after confirmation" || return 1\n',
    '        _H b_cp_reval_after_confirmation\n', 'b_cp_reval_after_confirmation')
out = insert_before(out,
    '            if _compact_revalidate "$c_name" "$tuple" "after the handover wait"; then\n',
    '            _H b_cp_reval_after_wait\n', 'b_cp_reval_after_wait')

# ─── v4 additions: the rename cut points (census-named) ──────────────────────
# 23. inside the two-lock region, before any check the rename then mutates
out = insert_after(out,
    '_cmd_rename_locked() {\n    local old_name="$1" new_name="$2"\n',
    '    _H b_rn_locked_entry\n', 'b_rn_locked_entry')

# 24. TMUX RENAMED — after the session (and main window) rename, before the move
out = insert_before(out,
    '    # 2. Move session directory\n',
    '    _H b_rn_tmux_renamed\n', 'b_rn_tmux_renamed')

# 25. DIR MOVED — after the state directory move, before the meta rewrite
out = insert_before(out,
    '    # 3. Update session= in meta file\n',
    '    _H b_rn_dir_moved\n', 'b_rn_dir_moved')

# 26. META UPDATED — after session= is rewritten, before workspace.md is regenerated
out = insert_before(out,
    '    # 4. Regenerate workspace.md (contains session name)\n',
    '    _H b_rn_meta_updated\n', 'b_rn_meta_updated')

open(DST, 'w', encoding='utf-8').write(out)
print("v4 instrumented sha256:", hashlib.sha256(out.encode()).hexdigest())
