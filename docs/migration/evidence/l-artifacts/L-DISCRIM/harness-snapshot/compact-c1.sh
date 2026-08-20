#!/opt/homebrew/bin/bash
# L-COMPACT chunk 1: baseline (SC-500/501/502/827), mid-op observers (SC-1305),
# preview (SC-507a/c/d), interactive (SC-503a/b, SC-837), config-keephistory (SC-836).
set -uo pipefail
source /tmp/aelx/lib/compact-lib.sh

# ── observers for SC-1305: a concurrent reader at EVERY compact cut ───────────
observe_mid_op() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    local -a e=(); mapfile -t e < <(l_env "$R")
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" list --json ) >"$R/cap/$tag.observer.list.stdout" 2>"$R/cap/$tag.observer.list.stderr"
    printf '%s\n' "$?" >"$R/cap/$tag.observer.list.rc"
    local rq="$R/h/.ae/sessions/cp1/requests"
    if [[ -x "$rq" ]]; then
        ( cd "$R/w" && env -i "${e[@]}" "$rq" all ) >"$R/cap/$tag.observer.requests.stdout" 2>"$R/cap/$tag.observer.requests.stderr"
        printf '%s\n' "$?" >"$R/cap/$tag.observer.requests.rc"
    else
        printf '(the generated requests helper is not present at this cut: %s)\n' "$rq" >"$R/cap/$tag.observer.requests.stdout"
        printf '%s\n' "-" >"$R/cap/$tag.observer.requests.rc"
    fi
    return 0
}

arm_baseline() {
    l_arm_begin L-COMPACT baseline instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    HOOKS="$CPCHAN"; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    CP_OBSERVE=observe_mid_op
    l_ae_bg 2op compact -f --digest-only cp1
    l_barriers cp1 300 cp_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    cbytes 2op
    csnap 3post
    { printf '# NAMED TRACE CHANNELS, in the order they fired (name, pid, monotonic clock)\n'
      cat "$R/cap/hook-trace.tsv" 2>/dev/null
      printf '\n# channel legend (site, not meaning)\n'
      printf 'b_cp_resolver_entry\t_compact_freeze_source entry — the tuple-freeze site\n'
      printf 'b_cp_reval_after_confirmation\tthe FIRST revalidation site (message: after confirmation)\n'
      printf 'b_cp_reval_after_wait\tthe SECOND revalidation site (message: after the handover wait)\n'
      printf 'b_cp_after_answer\tafter the human answer is accepted\n'
      printf 'b_cp_after_handover\tafter the handover completed, before phase (c)\n'
      printf 'b_cp_pre_relaunch\timmediately before the exec into the relaunch\n'
    } >"$R/cap/trace-channels.txt"
    carmtxt baseline "SC-500 SC-501 SC-502 SC-827 SC-1305" \
      "a real compact -f --digest-only under the v3 trace channels, blocking at every named compact cut; both output streams are captured separately and byte-exactly, snapshotted AGAIN at each cut, and a concurrent ae list --json plus the generated requests helper run from a separate process at every cut" \
      "op	ae compact -f --digest-only cp1" "op_rc	$(cat "$R/cap/2op.rc")" \
      "barrier_bound_sec	300" \
      "note	the three trace channels are SITES, named separately so one authoritative resolution and the permitted revalidation reads are separable; this worker records which channel fired and when, nothing about what that means"
    l_arm_end
}

arm_preview() {
    l_arm_begin L-COMPACT preview frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    # freeze the subject: stop it, so nothing writes while the twin is built
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 1stop stop -y cp1
    sleep 2
    csnap 1pre
    # ── the TWIN: a byte copy of the frozen session with exactly TWO named diffs
    local SD="$R/h/.ae/sessions"
    cp -Rp "$SD/cp1" "$SD/twin"
    cp -p "$SD/twin/meta" "$R/cap/twin-meta.before.txt"
    local TWIN_UUID; TWIN_UUID="$(/usr/bin/uuidgen | tr 'A-F' 'a-f')"
    local m0; m0="$(stat -f '%Lp' "$SD/twin/meta")"
    l_rewrite_preserving_mode "$SD/twin/meta" "s/^session=cp1\$/session=twin/"
    l_rewrite_preserving_mode "$SD/twin/meta" "s/^session_id=.*\$/session_id=${TWIN_UUID}/"
    cp -p "$SD/twin/meta" "$R/cap/twin-meta.after.txt"
    diff -u "$R/cap/twin-meta.before.txt" "$R/cap/twin-meta.after.txt" >"$R/cap/twin-meta.diff" 2>&1
    { printf 'twin.construction\ta byte copy (cp -Rp) of the FROZEN (stopped) session directory, with exactly TWO named byte diffs\n'
      printf 'twin.diff.1\tsession=cp1 -> session=twin\n'
      printf 'twin.diff.2\tsession_id -> a fresh uuid (two coexisting sessions cannot share one id)\n'
      printf 'twin.uuid\t%s\n' "$TWIN_UUID"
      printf 'twin.meta.mode.before\t%s\ntwin.meta.mode.after\t%s\n' "$m0" "$(stat -f '%Lp' "$SD/twin/meta")"
      printf 'twin.memory.identical\tmemo.tsv, events.jsonl and messages/ are the byte copy; see twin-vs-source.manifest.diff\n'
    } >"$R/cap/twin.txt"
    l_manifest "$SD/cp1" "$R/cap/source.sessiondir.tsv"
    l_manifest "$SD/twin" "$R/cap/twin.sessiondir.tsv"
    diff -u <(sed 's|/twin/|/SESS/|' "$R/cap/source.sessiondir.tsv") <(sed 's|/twin/|/SESS/|' "$R/cap/twin.sessiondir.tsv") >"$R/cap/twin-vs-source.manifest.diff" 2>&1
    # ── the PREVIEW on the source
    l_ae 2op archive preview cp1
    cbytes 2op
    # ── a REAL end on the TWIN
    l_ae 3twinend end -f twin
    sleep 1
    cbytes 3twinend
    local TD="$R/h/.ae/archive/$TWIN_UUID/digest.md"
    if [[ -f "$TD" ]]; then
        cp -p "$TD" "$R/cap/twin.archived-digest.md"
        od -c "$TD" >"$R/cap/twin.archived-digest.md.od"
    else
        printf '(no archived digest at %s)\n' "$TD" >"$R/cap/twin.archived-digest.md"
    fi
    csnap 3post
    diff -u "$R/cap/1pre.aehome.tsv" "$R/cap/3post.aehome.tsv" >"$R/cap/aehome.before-after.diff" 2>&1
    carmtxt preview "SC-507a SC-507c SC-507d" \
      "ae archive preview runs on a FROZEN (stopped) session; a twin of that same frozen session — a byte copy with exactly two named diffs — is then ended for real, and the twin's ARCHIVED digest.md bytes are captured alongside the preview's stdout bytes. The worker captures both and compares nothing" \
      "op	ae archive preview cp1" "op_rc	$(cat "$R/cap/2op.rc")" \
      "twin_end	ae end -f twin" "twin_end_rc	$(cat "$R/cap/3twinend.rc")" \
      "twin_uuid	$TWIN_UUID"
    l_arm_end
}

arm_interactive() { # <typed-n|eof|force>
    local mode="$1"
    l_arm_begin L-COMPACT "interactive-$mode" frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    HOOKS=""; BLOCK=""; l_arm_env "AE_COMPACT_HANDOVER_SECS=5"
    case "$mode" in
      typed-n)
        l_pane_start 2op "$R/b/ae" compact --digest-only cp1
        if l_pane_wait 60 "Continue?"; then l_pane_capture at-prompt; l_pane_send n; l_pane_enter
        else l_pane_capture prompt-not-observed; printf 'INCONCLUSIVE: prompt not observed in 60s\n' >"$R/cap/INCONCLUSIVE.txt"; fi
        l_pane_wait_rc 120 || printf 'INCONCLUSIVE: no exit status within 120s\n' >>"$R/cap/INCONCLUSIVE.txt"
        sleep 1; l_pane_capture final; l_pane_stop ;;
      eof)
        # the controller closes stdin: no terminal, no answer
        ( cd "$R/w" && env -i "${AE_ENV[@]}" "$R/b/ae" compact --digest-only cp1 </dev/null ) \
            >"$R/cap/2op.stdout" 2>"$R/cap/2op.stderr"; printf '%s\n' "$?" >"$R/cap/2op.rc"
        { printf 'cwd: %s\nargv:\n' "$R/w"; printf '  %s\n' "$R/b/ae" compact --digest-only cp1
          printf 'stdin: /dev/null (closed by the controller — end of input, no terminal)\n'; } >"$R/cap/2op.invocation" ;;
      force)
        l_ae 2op compact -f --digest-only cp1 ;;
    esac
    sleep 3
    cbytes 2op
    csnap 3post
    carmtxt "interactive-$mode" \
      "$( case "$mode" in typed-n) echo 'SC-503a' ;; eof) echo 'SC-503b' ;; force) echo 'SC-837' ;; esac )" \
      "$( case "$mode" in
            typed-n) echo 'ae compact --digest-only runs on a real terminal and the controller types n at the confirmation' ;;
            eof)     echo 'ae compact --digest-only runs with stdin closed by the controller (/dev/null) and no terminal' ;;
            force)   echo 'ae compact -f --digest-only runs, so no confirmation is asked at all' ;;
          esac )" \
      "mode	$mode" "op_rc	$(cat "$R/cap/2op.rc" 2>/dev/null)"
    l_arm_end
}

arm_config_keephistory() { # <with-keep|without-keep>
    local mode="$1"
    l_arm_begin L-COMPACT "config-keephistory-$mode" frozen
    PATCHV="none (frozen, unmodified)"
    l_config "$R" claude "purge_agent_history = true"
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local cp1
    sleep 3
    l_arm_preflight cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    CP_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/cp1/meta" | head -1 | cut -d= -f2-)"
    cp -p "$R/h/.ae/config" "$R/cap/config.txt"
    # controller-planted conversation markers at the exact path the frozen locator globs
    local projd="$R/h/.claude/projects/-tmp-aelx-w"; mkdir -p "$projd"
    { printf 'planted conversation markers (path-only; the frozen locator never reads their content)\n'
      while IFS= read -r line; do
        case "$line" in agent.*) u="${line##*:}"; [[ "$u" == pending ]] && continue
          printf '{"planted":"batch-l"}\n' >"$projd/$u.jsonl"; printf '  %s\t%s\n' "$u" "$projd/$u.jsonl" ;;
        esac
      done <"$R/h/.ae/sessions/cp1/meta"; } >"$R/cap/planted-conversations.txt"
    l_manifest "$R/h/.claude" "$R/cap/conversations.before.tsv"
    csnap 1pre
    if [[ "$mode" == with-keep ]]; then l_ae 2op compact -f --keep-history --digest-only cp1
    else l_ae 2op compact -f --digest-only cp1; fi
    sleep 3
    cbytes 2op
    l_manifest "$R/h/.claude" "$R/cap/conversations.after.tsv"
    diff -u "$R/cap/conversations.before.tsv" "$R/cap/conversations.after.tsv" >"$R/cap/conversations.diff" 2>&1
    csnap 3post
    carmtxt "config-keephistory-$mode" SC-836 \
      "the session's own config sets [workspace] purge_agent_history = true; compact then runs $( [[ "$mode" == with-keep ]] && echo 'WITH --keep-history' || echo 'WITHOUT --keep-history' )" \
      "mode	$mode" "op_rc	$(cat "$R/cap/2op.rc")" \
      "note	the conversation files are controller-planted markers at the exact path the frozen locator globs; that locator matches on PATH only and never reads their content"
    l_arm_end
}

case "${1:-all}" in
  base)    arm_baseline ;;
  preview) arm_preview ;;
  inter)   arm_interactive typed-n; arm_interactive eof; arm_interactive force ;;
  keep)    arm_config_keephistory with-keep; arm_config_keephistory without-keep ;;
esac
echo DONE
