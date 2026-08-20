#!/opt/homebrew/bin/bash
# D1 — parent tuple RECORDED, not re-read.
# D2 — counts survive a stop/resume cycle.
set -uo pipefail
source /tmp/aelx/lib/discrim-lib.sh

PH=2; PP=1        # the parent's counts: non-zero and distinct
RH=5; RP=3        # the replacement's counts: non-zero, distinct, and different again

SWAPPED=""
d1_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_from_proved.*)
        [[ -n "$SWAPPED" ]] && return 0
        SWAPPED=yes
        local AD="$R/h/.ae/archive/$PARENT_UUID"
        l_manifest "$AD" "$R/cap/$tag.archive.before-swap.tsv"
        cp -p "$AD/meta" "$R/cap/$tag.parent-meta.before-swap.txt"
        # REPLACE the parent with the prepared same-id archive carrying different counts.
        rm -rf "$AD"
        cp -Rp "$R/replacement/$PARENT_UUID" "$AD"
        l_manifest "$AD" "$R/cap/$tag.archive.after-swap.tsv"
        cp -p "$AD/meta" "$R/cap/$tag.parent-meta.after-swap.txt"
        diff -u "$R/cap/$tag.parent-meta.before-swap.txt" "$R/cap/$tag.parent-meta.after-swap.txt" >"$R/cap/$tag.parent-meta.swap.diff" 2>&1
        { printf 'controller.barrier\t%s\n' "$k"
          printf 'controller.action\tthe parent archive directory is REPLACED, at the same id, by a second VALID archive whose counts are %s/%s instead of %s/%s\n' "$RH" "$RP" "$PH" "$PP"
          printf 'controller.replacement.source\t%s\n' "$R/replacement/$PARENT_UUID"
          printf 'timing\tafter the launch has parsed its FIRST parent proof and before it publishes the child meta\n'
        } >"$R/cap/$tag.controller.txt"
        led swap barrier "$k"
        led swap replacement.counts "handover=$RH pending=$RP"
        ;;
    esac
    return 0
}

arm_d1() {
    l_arm_begin L-DISCRIM D1-parent-tuple-recorded-not-reread instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    : >"$R/cap/ledger.tsv"
    led setup arm D1
    d_make_counted_parent p1 "$PH" "$PP" || { printf 'ARM INVALID: the counted parent fixture could not be built\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    if ! d_assert_counts_discriminating "$PARENT_H" "$PARENT_P"; then
        { printf 'ARM INVALID: the parent counts are not discriminating (handover=%s pending=%s).\n' "$PARENT_H" "$PARENT_P"
          printf 'Both must be NON-ZERO and DISTINCT, or a lost value is indistinguishable from a default.\n'
        } >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1
    fi
    # ── build the REPLACEMENT: same id, valid tree, different non-zero counts ──
    mkdir -p "$R/replacement"
    cp -Rp "$R/h/.ae/archive/$PARENT_UUID" "$R/replacement/$PARENT_UUID"
    d_set_counts "$R/replacement/$PARENT_UUID" "$RH" "$RP" replacement
    l_manifest "$R/replacement/$PARENT_UUID" "$R/cap/replacement.manifest.tsv"

    # ── ABILITY-TO-FAIL CONTROL, run FIRST, in its own throwaway AE_HOME clone.
    #    If a --from against the REPLACEMENT does NOT record the replacement's counts,
    #    then the swap below could never have shown them either, and a measurement
    #    that finds the first tuple would prove nothing about re-reading.
    local CTL="$R/h2"
    mkdir -p "$CTL"
    cp -Rp "$R/h/.ae" "$CTL/.ae"
    rm -rf "$CTL/.ae/archive/$PARENT_UUID"
    cp -Rp "$R/replacement/$PARENT_UUID" "$CTL/.ae/archive/$PARENT_UUID"
    local -a CE=(); mapfile -t CE < <(l_env "$R" "AE_HOME=$CTL/.ae")
    ( cd "$R/w" && env -i "${CE[@]}" "$R/b/ae" --local ctlchild --from "$PARENT_UUID" ) \
        >"$R/cap/1control.stdout" 2>"$R/cap/1control.stderr"; printf '%s\n' "$?" >"$R/cap/1control.rc"
    sleep 3
    { local m="$CTL/.ae/sessions/ctlchild/meta"
      if [[ -f "$m" ]]; then
        printf '# the CONTROL child tuple, from a --from against the REPLACEMENT archive\n'
        grep -n '^parent_archive_id=\|^parent_archive_handover_count=\|^parent_archive_pending_count=' "$m"
      else printf '(no control child meta)\n'; fi
    } >"$R/cap/control-child-tuple.txt" 2>&1
    local CH CP2
    CH="$(grep '^parent_archive_handover_count=' "$CTL/.ae/sessions/ctlchild/meta" 2>/dev/null | cut -d= -f2-)"
    CP2="$(grep '^parent_archive_pending_count=' "$CTL/.ae/sessions/ctlchild/meta" 2>/dev/null | cut -d= -f2-)"
    led control replacement.readable.handover "${CH:-<none>}"
    led control replacement.readable.pending "${CP2:-<none>}"
    led control replacement.accepted "$( [[ "$CH" == "$RH" && "$CP2" == "$RP" ]] && echo YES || echo NO )"
    /opt/homebrew/bin/tmux -S "$SOCK" kill-session -t ctlchild 2>/dev/null
    if [[ "$CH" != "$RH" || "$CP2" != "$RP" ]]; then
        { printf 'ARM INVALID: the replacement archive is not readable as a parent.\n'
          printf 'A --from against it recorded handover=%s pending=%s, expected %s/%s.\n' "${CH:-<none>}" "${CP2:-<none>}" "$RH" "$RP"
          printf 'Without that, a swap that leaves the first tuple in place would prove nothing:\n'
          printf 'the second tuple could simply be unreadable. The arm cannot produce its unwanted answer.\n'
        } >"$R/cap/ARM-INVALID.txt"
        l_arm_end; return 1
    fi

    dsnap 1pre
    # ── the measurement: swap the parent at the barrier after the first proof ──
    HOOKS=b_from_proved; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op --local child --from "$PARENT_UUID"
    l_barriers child 300 d1_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    d_child_tuple child "$R/cap/child-tuple.txt"
    local KH KP
    KH="$(grep '^parent_archive_handover_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    KP="$(grep '^parent_archive_pending_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    led measure child.handover_count "${KH:-<no child meta>}"
    led measure child.pending_count "${KP:-<no child meta>}"
    dsnap 3post
    darmtxt D1-parent-tuple-recorded-not-reread "SC-824a discriminator" \
      "the existing arm deletes the parent archive at the barrier, so the launch has no tuple to record and the arm cannot show WHICH tuple a surviving child would carry" \
      "a parent archive is built with handover=$PH pending=$PP (non-zero and distinct); at the barrier after the launch parses its FIRST parent proof the controller REPLACES that archive, at the same id, with a second VALID archive carrying handover=$RH pending=$RP; the launch is released and the child recorded tuple is read BY EXACT KEY" \
      "parent_uuid	$PARENT_UUID" \
      "parent_counts	handover=$PARENT_H pending=$PARENT_P" \
      "replacement_counts	handover=$RH pending=$RP" \
      "control_replacement_accepted	$( [[ "$CH" == "$RH" && "$CP2" == "$RP" ]] && echo YES || echo NO ) (a --from against the replacement alone records handover=$CH pending=$CP2)" \
      "child_recorded_handover	${KH:-<no child meta>}" \
      "child_recorded_pending	${KP:-<no child meta>}" \
      "op_rc	$(cat "$R/cap/2op.rc")" \
      "OBSERVATION	the child recorded tuple is in child-tuple.txt; the control tuple in control-child-tuple.txt; the swap diff in the b*-controller and swap files. No verdict is stated here."
    l_arm_end
    return 0
}

arm_d2() {
    l_arm_begin L-DISCRIM D2-counts-survive-a-cycle frozen
    PATCHV="none (frozen, unmodified)"
    : >"$R/cap/ledger.tsv"
    led setup arm D2
    d_make_counted_parent p1 "$PH" "$PP" || { printf 'ARM INVALID: the counted parent fixture could not be built\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    if ! d_assert_counts_discriminating "$PARENT_H" "$PARENT_P"; then
        { printf 'ARM INVALID: the parent counts are not discriminating (handover=%s pending=%s).\n' "$PARENT_H" "$PARENT_P"
          printf 'Both must be NON-ZERO and DISTINCT: with a 0, losing the value and defaulting to zero are the same observation.\n'
        } >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1
    fi
    HOOKS=""; BLOCK=""; l_arm_env
    dsnap 1pre
    l_ae 1child --local child --from "$PARENT_UUID"
    sleep 3
    [[ -f "$R/h/.ae/sessions/child/meta" ]] || { printf 'ARM INVALID: the --from child was not created\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    d_child_tuple child "$R/cap/tuple.1after-from.txt"
    local H1 P1; H1="$(grep '^parent_archive_handover_count=' "$R/h/.ae/sessions/child/meta" | cut -d= -f2-)"
    P1="$(grep '^parent_archive_pending_count=' "$R/h/.ae/sessions/child/meta" | cut -d= -f2-)"
    led cycle after-from.handover "$H1"; led cycle after-from.pending "$P1"
    l_ae 2stop stop -y child
    sleep 3
    d_child_tuple child "$R/cap/tuple.2after-stop.txt"
    l_ae 3resume --local child
    sleep 4
    d_child_tuple child "$R/cap/tuple.3after-resume.txt"
    local H3 P3; H3="$(grep '^parent_archive_handover_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    P3="$(grep '^parent_archive_pending_count=' "$R/h/.ae/sessions/child/meta" 2>/dev/null | cut -d= -f2-)"
    led cycle after-resume.handover "${H3:-<none>}"; led cycle after-resume.pending "${P3:-<none>}"
    diff -u "$R/cap/tuple.1after-from.txt" "$R/cap/tuple.3after-resume.txt" >"$R/cap/tuple.across-cycle.diff" 2>&1
    dsnap 3post
    darmtxt D2-counts-survive-a-cycle "SC-825a discriminator" \
      "every count in the existing lineage-durability arms is 0, so losing the value and defaulting to zero produce the same reading; only NON-ZERO and DISTINCT counts can fail" \
      "a parent archive is built with handover=$PH pending=$PP; a real --from child is launched, its tuple read, the child stopped and resumed, and its tuple read again BY EXACT KEY" \
      "parent_uuid	$PARENT_UUID" \
      "parent_counts	handover=$PARENT_H pending=$PARENT_P" \
      "after_from	handover=$H1 pending=$P1" \
      "after_resume	handover=${H3:-<none>} pending=${P3:-<none>}" \
      "child_launch_rc	$(cat "$R/cap/1child.rc")" "stop_rc	$(cat "$R/cap/2stop.rc")" "resume_rc	$(cat "$R/cap/3resume.rc")" \
      "OBSERVATION	the three tuple captures and their diff are in tuple.*.txt and tuple.across-cycle.diff. No verdict is stated here."
    l_arm_end
    return 0
}

case "${1:-}" in
  d1) arm_d1 ;;
  d2) arm_d2 ;;
esac
echo DONE
