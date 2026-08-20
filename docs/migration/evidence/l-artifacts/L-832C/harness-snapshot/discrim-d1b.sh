#!/opt/homebrew/bin/bash
# D1b — the id half. Same construction as D1, but the replacement archive carries a
# DIFFERENT id as well as different counts.
set -uo pipefail
source /tmp/aelx/lib/discrim-lib.sh

PH=2; PP=1        # the parent's counts
RH=5; RP=3        # the replacement's counts
SWAPPED=""

d1b_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_from_proved.*)
        [[ -n "$SWAPPED" ]] && return 0
        SWAPPED=yes
        local ROOT="$R/h/.ae/archive"
        l_manifest "$ROOT" "$R/cap/$tag.archive.before-swap.tsv"
        rm -rf "$ROOT/$PARENT_UUID"
        cp -Rp "$R/replacement/$REPL_UUID" "$ROOT/$REPL_UUID"
        l_manifest "$ROOT" "$R/cap/$tag.archive.after-swap.tsv"
        diff -u "$R/cap/$tag.archive.before-swap.tsv" "$R/cap/$tag.archive.after-swap.tsv" >"$R/cap/$tag.archive.swap.diff" 2>&1
        { printf 'controller.barrier\t%s\n' "$k"
          printf 'controller.action\tthe parent archive at id %s is REMOVED and a second VALID archive is installed at a DIFFERENT id %s with counts %s/%s\n' "$PARENT_UUID" "$REPL_UUID" "$RH" "$RP"
          printf 'timing\tafter the launch has parsed its FIRST parent proof and before it publishes the child meta\n'
          printf 'note\tboth archives cannot coexist at one path; the frozen validator requires an archive meta archive_id to equal its directory name, so a different id means a different directory\n'
        } >"$R/cap/$tag.controller.txt"
        led swap barrier "$k"
        led swap removed.id "$PARENT_UUID"
        led swap installed.id "$REPL_UUID"
        ;;
    esac
    return 0
}

arm_d1b() {
    l_arm_begin L-DISCRIM D1b-parent-id-recorded-or-reread instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    : >"$R/cap/ledger.tsv"; led setup arm D1b
    d_make_counted_parent p1 "$PH" "$PP" || { printf 'ARM INVALID: the counted parent fixture could not be built\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    d_assert_counts_discriminating "$PARENT_H" "$PARENT_P" || {
        { printf 'ARM INVALID: the parent counts are not discriminating (handover=%s pending=%s).\n' "$PARENT_H" "$PARENT_P"; } >"$R/cap/ARM-INVALID.txt"
        l_arm_end; return 1; }

    # ── the REPLACEMENT: a DIFFERENT id AND different counts, still valid ─────
    REPL_UUID="$(/usr/bin/uuidgen | tr 'A-F' 'a-f')"
    mkdir -p "$R/replacement"
    cp -Rp "$R/h/.ae/archive/$PARENT_UUID" "$R/replacement/$REPL_UUID"
    d_set_counts "$R/replacement/$REPL_UUID" "$RH" "$RP" replacement
    # the id lives in TWO places the frozen validator checks: meta archive_id, and the
    # digest's "- Archive ID: <id>" line. Both are rewritten, mode preserved.
    cp -p "$R/replacement/$REPL_UUID/meta" "$R/cap/replacement-id.meta.before.txt"
    cp -p "$R/replacement/$REPL_UUID/digest.md" "$R/cap/replacement-id.digest.before.md"
    l_rewrite_preserving_mode "$R/replacement/$REPL_UUID/meta" "s/^archive_id=.*\$/archive_id=${REPL_UUID}/"
    l_rewrite_preserving_mode "$R/replacement/$REPL_UUID/digest.md" "s/^- Archive ID: .*\$/- Archive ID: ${REPL_UUID}/"
    cp -p "$R/replacement/$REPL_UUID/meta" "$R/cap/replacement-id.meta.after.txt"
    cp -p "$R/replacement/$REPL_UUID/digest.md" "$R/cap/replacement-id.digest.after.md"
    diff -u "$R/cap/replacement-id.meta.before.txt" "$R/cap/replacement-id.meta.after.txt" >"$R/cap/replacement-id.meta.diff" 2>&1
    diff -u "$R/cap/replacement-id.digest.before.md" "$R/cap/replacement-id.digest.after.md" >"$R/cap/replacement-id.digest.diff" 2>&1
    l_manifest "$R/replacement/$REPL_UUID" "$R/cap/replacement.manifest.tsv"
    led setup parent.uuid "$PARENT_UUID"
    led setup replacement.uuid "$REPL_UUID"

    # ── ABILITY-TO-FAIL CONTROL: a --from against the REPLACEMENT ALONE must
    #    record the replacement's id AND its counts, or the measurement is empty.
    local CTL="$R/h2"; mkdir -p "$CTL"
    cp -Rp "$R/h/.ae" "$CTL/.ae"
    rm -rf "$CTL/.ae/archive/$PARENT_UUID"
    cp -Rp "$R/replacement/$REPL_UUID" "$CTL/.ae/archive/$REPL_UUID"
    local -a CE=(); mapfile -t CE < <(l_env "$R" "AE_HOME=$CTL/.ae")
    ( cd "$R/w" && env -i "${CE[@]}" "$R/b/ae" --local ctlchild --from "$REPL_UUID" ) \
        >"$R/cap/1control.stdout" 2>"$R/cap/1control.stderr"; printf '%s\n' "$?" >"$R/cap/1control.rc"
    sleep 3
    local CM="$CTL/.ae/sessions/ctlchild/meta" CI CH CP2
    { if [[ -f "$CM" ]]; then
        printf '# the CONTROL child tuple, from a --from against the REPLACEMENT alone\n'
        grep -n '^parent_archive_id=\|^parent_archive_handover_count=\|^parent_archive_pending_count=' "$CM"
      else printf '(no control child meta)\n'; fi
    } >"$R/cap/control-child-tuple.txt" 2>&1
    CI="$(grep '^parent_archive_id=' "$CM" 2>/dev/null | cut -d= -f2-)"
    CH="$(grep '^parent_archive_handover_count=' "$CM" 2>/dev/null | cut -d= -f2-)"
    CP2="$(grep '^parent_archive_pending_count=' "$CM" 2>/dev/null | cut -d= -f2-)"
    led control replacement.readable.id "${CI:-<none>}"
    led control replacement.readable.counts "handover=${CH:-<none>} pending=${CP2:-<none>}"
    led control replacement.accepted "$( [[ "$CI" == "$REPL_UUID" && "$CH" == "$RH" && "$CP2" == "$RP" ]] && echo YES || echo NO )"
    /opt/homebrew/bin/tmux -S "$SOCK" kill-session -t ctlchild 2>/dev/null
    if [[ "$CI" != "$REPL_UUID" || "$CH" != "$RH" || "$CP2" != "$RP" ]]; then
        { printf 'ARM INVALID: the replacement archive is not readable as a parent under its own id.\n'
          printf 'A --from against it recorded id=%s handover=%s pending=%s; expected %s / %s / %s.\n' "${CI:-<none>}" "${CH:-<none>}" "${CP2:-<none>}" "$REPL_UUID" "$RH" "$RP"
          printf 'Without that, a measurement finding the FIRST id would prove nothing about re-reading.\n'
        } >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1
    fi

    # ── the frozen source, as a CODE OBSERVATION with line numbers, no verdict ──
    { printf '%s\n' 'FROZEN SOURCE, extracted verbatim with line numbers. A code observation, not a verdict.'
      printf 'frozen commit\t72c729343a0117af2968b66e1c43f89ad25fc0b2\n\n'
      printf '%s\n' '=== where the id in the proof tuple comes from (_ar_from_preflight) ==='
      LC_ALL=C awk '/^_ar_from_preflight\(\) \{/,/^\}$/{printf "%d:%s\n", NR, $0}' /tmp/aelx/frozen/ae
      printf '\n%s\n' '=== every line naming PARENT_ARCHIVE_ID or _AE_FROM_EXPECTED in the launch ==='
      LC_ALL=C awk '/PARENT_ARCHIVE_ID|_AE_FROM_EXPECTED/{printf "%d:%s\n", NR, $0}' /tmp/aelx/frozen/ae
    } >"$R/cap/source-trace.where-the-id-comes-from.txt"
    led setup source-trace.sha256 "$(l_sha "$R/cap/source-trace.where-the-id-comes-from.txt")"

    dsnap 1pre
    # ── the measurement ──────────────────────────────────────────────────────
    HOOKS=b_from_proved; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op --local child --from "$PARENT_UUID"
    l_barriers child 300 d1b_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    d_child_tuple child "$R/cap/child-tuple.txt"
    local KI KH KP
    KI="$(grep '^parent_archive_id=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    KH="$(grep '^parent_archive_handover_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    KP="$(grep '^parent_archive_pending_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    led measure child.exists "$( [[ -f "$R/h/.ae/sessions/child/meta" ]] && echo YES || echo NO )"
    led measure child.parent_archive_id "${KI:-<no child meta>}"
    led measure child.counts "handover=${KH:-<none>} pending=${KP:-<none>}"
    dsnap 3post
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions-full.tsv"

    # ── could this arm have produced the UNWANTED answer? Derived, not asserted ──
    local VERDICTABLE
    if [[ -z "$KI" ]]; then
        VERDICTABLE=NO
        { printf 'ARM-INVALID AS AN ID DISCRIMINATOR — and the reason is the finding.\n\n'
          printf 'No child meta exists after the swap, so there is no recorded id to read and the arm\n'
          printf 'cannot show WHICH id a surviving child would carry. The child rc was %s.\n\n' "$(cat "$R/cap/2op.rc")"
          printf 'WHY THE CONSTRUCTION CANNOT SEPARATE THE TWO BEHAVIOURS, from the frozen source in\n'
          printf 'source-trace.where-the-id-comes-from.txt:\n'
          printf '  - the id in a proof tuple is derived from the ARGUMENT (_ar_canonical_uuid of the\n'
          printf '    id passed in), never read out of the archive meta;\n'
          printf '  - the second proof is called with PARENT_ARCHIVE_ID, i.e. with the id the FIRST\n'
          printf '    proof already produced, so both proofs are asked about the SAME id by construction;\n'
          printf '  - the frozen validator requires an archive meta archive_id to equal its directory\n'
          printf '    name, so an archive carrying a different id must live at a different path — and\n'
          printf '    the path the second proof looks at is the one named by the first id.\n\n'
          printf 'A mutant that re-read the id would therefore re-read THE SAME STRING, and emits the\n'
          printf 'same artifacts as one that records it. The id half is not separable at runtime by this\n'
          printf 'construction, and no id observation is reported here.\n'
        } >"$R/cap/ARM-INVALID.txt"
    elif [[ "$KI" == "$REPL_UUID" ]]; then
        VERDICTABLE=YES
        led measure id.source "the child recorded the REPLACEMENT id"
    else
        VERDICTABLE=YES
        led measure id.source "the child recorded the FIRST id"
    fi
    led measure arm.could.produce.unwanted.answer "$VERDICTABLE"

    darmtxt D1b-parent-id-recorded-or-reread "SC-824a, id half" \
      "D1 holds the archive id CONSTANT by construction, so it discriminates on COUNTS only; a mutant re-reading the id while retaining proved counts emits identical artifacts" \
      "the same construction as D1, but at the barrier after the FIRST parent proof the archive at the parent id is REMOVED and a second VALID archive is installed at a DIFFERENT id with different counts; the child recorded tuple is then read BY EXACT KEY" \
      "parent_uuid	$PARENT_UUID" \
      "parent_counts	handover=$PARENT_H pending=$PARENT_P" \
      "replacement_uuid	$REPL_UUID" \
      "replacement_counts	handover=$RH pending=$RP" \
      "control_replacement_accepted	$( [[ "$CI" == "$REPL_UUID" ]] && echo YES || echo NO ) (a --from against the replacement alone records id=$CI handover=$CH pending=$CP2)" \
      "child_exists	$( [[ -n "$KI" ]] && echo YES || echo NO )" \
      "child_recorded_id	${KI:-<no child meta>}" \
      "child_recorded_counts	handover=${KH:-<none>} pending=${KP:-<none>}" \
      "op_rc	$(cat "$R/cap/2op.rc")" \
      "arm_could_produce_the_unwanted_answer	$VERDICTABLE" \
      "OBSERVATION	child-tuple.txt, control-child-tuple.txt, the swap diff, and source-trace.where-the-id-comes-from.txt. No verdict is stated here."
    l_arm_end
    return 0
}

arm_d1b
echo DONE
