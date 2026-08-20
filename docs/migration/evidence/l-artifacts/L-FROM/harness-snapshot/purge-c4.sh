#!/opt/homebrew/bin/bash
# L-PURGE: execution-sentinel (SC-805), lineage-parent (SC-818e),
# unidentifiable (SC-819).
set -uo pipefail
source /tmp/aelx/lib/purge-lib.sh

snap_pre()  { l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"; l_manifest "$R/h/.ae/archive" "$R/cap/1pre.archive.tsv"; l_manifest "$R/h/.ae/sessions" "$R/cap/1pre.sessions.tsv"; }
snap_post() { l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"; l_manifest "$R/h/.ae/archive" "$R/cap/3post.archive.tsv"; l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"; l_tmuxsnap "$SOCK" "$R/cap/3post.tmux.txt"; }

# ─────────────────────────────────────────── SC-805: execution sentinel
sentinel_state() { # <label>
    { printf 'sentinel.path\t%s\n' "$R/SENTINEL-WAS-EXECUTED"
      printf 'sentinel.exists\t%s\n' "$( [[ -e "$R/SENTINEL-WAS-EXECUTED" ]] && echo yes || echo no )"
      printf 'sentinel.bytes\t%s\n' "$(stat -f '%z' "$R/SENTINEL-WAS-EXECUTED" 2>/dev/null || echo '-')"
      [[ -e "$R/SENTINEL-WAS-EXECUTED" ]] && { printf 'sentinel.content\n'; cat "$R/SENTINEL-WAS-EXECUTED"; }
    } >"$R/cap/sentinel.$1.txt"
    return 0
}

arm_sentinel() { # <op: purge|from|compact> <mode: exec|noexec>
    local op="$1" mode="$2"
    local arm="execution-sentinel-${op}"
    [[ "$mode" == noexec ]] && arm="control-sentinel-no-exec-bit-${op}"
    l_arm_begin L-PURGE "$arm" instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    local ARCH="$R/h/.ae/archive/$PG_UUID"
    local TGT="$ARCH/memo.tsv"
    local before_mode; before_mode="$(stat -f '%Lp' "$TGT")"
    cp -p "$TGT" "$R/cap/member.before.txt"
    { printf '#!/bin/sh\n'
      printf 'printf %%s "batch-l sentinel: this archive member was EXECUTED" > %s\n' "$R/SENTINEL-WAS-EXECUTED"
    } >"$TGT"
    if [[ "$mode" == exec ]]; then chmod 0700 "$TGT"; else chmod "0$before_mode" "$TGT"; fi
    cp -p "$TGT" "$R/cap/member.after.txt"
    { printf 'mutation.target\t%s\n' "$TGT"
      printf 'mutation.description\tthe archive member memo.tsv is replaced by a shebang script whose body would write a SENTINEL file OUTSIDE the archive\n'
      printf 'mutation.mode.before\t%s\n' "$before_mode"
      printf 'mutation.mode.after\t%s\n' "$(stat -f '%Lp' "$TGT")"
      printf 'mutation.exec_bits\t%s\n' "$( [[ "$mode" == exec ]] && echo 'GRANTED (0700)' || echo 'NOT granted — original mode kept (control)' )"
      printf 'sentinel.path\t%s\n' "$R/SENTINEL-WAS-EXECUTED"
    } >"$R/cap/mutation.txt"
    diff -u "$R/cap/member.before.txt" "$R/cap/member.after.txt" >"$R/cap/mutation.diff" 2>&1
    l_manifest "$ARCH" "$R/cap/archive.post-mutation.tsv"
    sentinel_state 1pre
    snap_pre
    HOOKS=""; BLOCK=""; l_arm_env
    case "$op" in
      purge)   l_ae 2op end -f --purge-history pg ;;
      from)    l_ae 2op --local pgchild --from "$PG_UUID" ;;
      compact) l_ae 2op compact -f --digest-only pg ;;
    esac
    sleep 2
    sentinel_state 3post
    snap_post
    parmtxt "$arm" \
      "$( [[ "$mode" == exec ]] && echo 'SC-805' || echo '(none — control, captures only)' )" \
      "an archive member is replaced by a shebang script whose body would write a sentinel OUTSIDE the archive$( [[ "$mode" == exec ]] && echo ', and is given executable bits' || echo ', with its ORIGINAL mode kept (control: no executable bit)' ); ONE archive-consuming operation runs on this clone" \
      "archive_consuming_op	$op" "op_rc	$(cat "$R/cap/2op.rc")" \
      "sentinel_before	$(grep '^sentinel.exists' "$R/cap/sentinel.1pre.txt" | cut -f2)" \
      "sentinel_after	$(grep '^sentinel.exists' "$R/cap/sentinel.3post.txt" | cut -f2)"
    l_arm_end
}

# ───────────────────────────────────── SC-818e: lineage parent
arm_lineage_parent() { # <mutated|literal>
    local reading="$1"
    local arm="lineage-parent-$reading"
    l_arm_begin L-PURGE "$arm" frozen
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch-parent --local pp
    sleep 2
    l_arm_preflight pp || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    PARENT_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/pp/meta" | head -1 | cut -d= -f2-)"
    l_ae 0end-parent end -f pp
    sleep 1
    cp -p "$R/h/.ae/archive/$PARENT_UUID/meta" "$R/cap/parent-archive-meta.txt" 2>/dev/null
    # a REAL --from child
    l_ae 1launch-child --local ch --from "$PARENT_UUID"
    sleep 3
    local CMETA="$R/h/.ae/sessions/ch/meta"
    [[ -f "$CMETA" ]] || { printf 'FIXTURE-INVALID: the --from child was not created\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    cp -p "$CMETA" "$R/cap/child-meta.before.txt"
    PG_UUID="$PARENT_UUID"
    if [[ "$reading" == mutated ]]; then
        local m0; m0="$(stat -f '%Lp' "$CMETA")"
        l_rewrite_preserving_mode "$CMETA" "s/^session_id=.*\$/session_id=${PARENT_UUID}/"
        cp -p "$CMETA" "$R/cap/child-meta.after.txt"
        diff -u "$R/cap/child-meta.before.txt" "$R/cap/child-meta.after.txt" >"$R/cap/mutation.diff" 2>&1
        { printf 'mutation.target\t%s\n' "$CMETA"
          printf 'mutation.description\tthe real --from child'"'"'s meta session_id is set EQUAL to its parent_archive_id, temp + chmod-to-original-mode + rename\n'
          printf 'mutation.mode.before\t%s\n' "$m0"
          printf 'mutation.mode.after\t%s\n' "$(stat -f '%Lp' "$CMETA")"
          printf 'reachability.note\tframed as a CODE OBSERVATION for the seats, not a verdict: at 72c7293 _ar_purge_archive:5404-5408 the refusal fires only when the aid being purged equals the session'"'"'s own parent_archive_id, i.e. when meta session_id == meta parent_archive_id. A real --from child receives a FRESH session_id (launch path), so no sequence of real operations produces that equality. This clone reaches it by the single named mutation above.\n'
        } >"$R/cap/mutation.txt"
    else
        cp -p "$CMETA" "$R/cap/child-meta.after.txt"
        printf 'mutation\tNONE — the real --from child is left exactly as the product created it\n' >"$R/cap/mutation.txt"
    fi
    snap_pre
    l_ae 2op end -f --purge-history ch
    sleep 1
    snap_post
    { printf '# parent archive present after\t%s\n' "$( [[ -d "$R/h/.ae/archive/$PARENT_UUID" ]] && echo yes || echo no )"
      printf '# archive dirs after\n'; ls -1 "$R/h/.ae/archive" 2>&1
      printf '# session dirs after\n'; ls -1 "$R/h/.ae/sessions" 2>&1
      printf '# child meta lineage keys at the time of the op\n'
      grep -n '^session_id=\|^parent_archive_id=\|^session_id_origin=' "$R/cap/child-meta.after.txt" 2>&1
    } >"$R/cap/lineage.txt" 2>&1
    parmtxt "$arm" SC-818e \
      "a REAL --from child of a REAL parent archive$( [[ "$reading" == mutated ]] && echo ', with ONE named mutation setting the child meta session_id equal to its parent_archive_id' || echo ', left unmutated' ); end --purge-history then runs on the child" \
      "reading	$( [[ "$reading" == mutated ]] && echo '(a) the construction that reaches the named guard' || echo '(b) the literal reading — the real child, unmutated' )" \
      "parent_uuid	$PARENT_UUID" \
      "parent_end_rc	$(cat "$R/cap/0end-parent.rc")" \
      "child_launch_rc	$(cat "$R/cap/1launch-child.rc")" \
      "op	ae end -f --purge-history ch" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ─────────────────────────────────── SC-819: unidentifiable session
arm_unidentifiable() { # <missing-meta|unparseable-id> <keep|purge> [assume-stopped]
    local cls="$1" pol="$2"
    local ack="${3:-}"
    local arm="unidentifiable-${cls}-${pol}"
    [[ -n "$ack" ]] && arm="unidentifiable-${cls}-${pol}-assume-stopped"
    l_arm_begin L-PURGE "$arm" frozen
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local un
    sleep 2
    l_arm_preflight un || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    PG_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/un/meta" | head -1 | cut -d= -f2-)"
    l_ae 0stop stop -y un
    sleep 1
    local META="$R/h/.ae/sessions/un/meta"
    cp -p "$META" "$R/cap/session-meta.before.txt"
    l_manifest "$R/h/.ae/sessions/un" "$R/cap/sessiondir.before.tsv"
    if [[ "$cls" == missing-meta ]]; then
        rm -f "$META"
        printf 'mutation\tthe session meta file is REMOVED; the rest of the session directory (memo, events, helpers, messages) is left intact\n' >"$R/cap/mutation.txt"
        printf '(the meta file no longer exists)\n' >"$R/cap/session-meta.after.txt"
    else
        local m0; m0="$(stat -f '%Lp' "$META")"
        l_rewrite_preserving_mode "$META" 's/^session_id=.*$/session_id=not-a-uuid--0000/'
        cp -p "$META" "$R/cap/session-meta.after.txt"
        { printf 'mutation\tthe session meta session_id value is replaced by an UNPARSEABLE token (not-a-uuid--0000), temp + chmod-to-original-mode + rename\n'
          printf 'mutation.mode.before\t%s\n' "$m0"
          printf 'mutation.mode.after\t%s\n' "$(stat -f '%Lp' "$META")"
          printf 'distinct_from\tthe legacy MISSING-session_id mint path (SC-826) — the key is PRESENT here, its value is unparseable\n'; } >"$R/cap/mutation.txt"
    fi
    diff -u "$R/cap/session-meta.before.txt" "$R/cap/session-meta.after.txt" >"$R/cap/mutation.diff" 2>&1
    l_manifest "$R/h/.ae/sessions/un" "$R/cap/sessiondir.after.tsv"
    snap_pre
    if [[ -n "$ack" ]]; then
        if [[ "$pol" == purge ]]; then l_ae 2op end -f --assume-stopped --purge-history un
        else l_ae 2op end -f --assume-stopped un; fi
    else
        if [[ "$pol" == purge ]]; then l_ae 2op end -f --purge-history un
        else l_ae 2op end -f un; fi
    fi
    sleep 1
    snap_post
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions-full.tsv"
    parmtxt "$arm" "SC-819" \
      "a real --local session is stopped, then ONE named mutation is applied to its state ($cls), then exactly ONE end invocation runs on this clone ($pol)" \
      "class	$cls" "policy	$pol" \
      "subclass	$( [[ -n "$ack" ]] && echo 'flag-bearing: --assume-stopped is passed, a real frozen per-target flag' || echo 'front-door: no acknowledgement flag' )" \
      "op	ae end -f $( [[ -n "$ack" ]] && printf -- '--assume-stopped ' )$( [[ "$pol" == purge ]] && printf -- '--purge-history ' )un" \
      "op_rc	$(cat "$R/cap/2op.rc")" "stop_rc	$(cat "$R/cap/0stop.rc")"
    l_arm_end
}

case "${1:-all}" in
  sentinel)
    arm_sentinel purge exec; arm_sentinel from exec; arm_sentinel compact exec
    arm_sentinel purge noexec ;;
  lineage) arm_lineage_parent mutated; arm_lineage_parent literal ;;
  unident)
    arm_unidentifiable missing-meta keep; arm_unidentifiable missing-meta purge
    arm_unidentifiable unparseable-id keep; arm_unidentifiable unparseable-id purge ;;
  unident-ack)
    arm_unidentifiable missing-meta keep assume-stopped
    arm_unidentifiable missing-meta purge assume-stopped ;;
esac
echo DONE
