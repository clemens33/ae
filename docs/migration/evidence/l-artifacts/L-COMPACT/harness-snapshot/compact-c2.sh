#!/opt/homebrew/bin/bash
# L-COMPACT chunk 2: recovery-exec (SC-512), sigpipe (SC-504b),
# exit-identity (SC-517a/b/c), revalidation (SC-828).
set -uo pipefail
source /tmp/aelx/lib/compact-lib.sh

# ───────────────────────────────────────────────── SC-512: the recovery command
CUTKILL=""
rec_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    cp_cb "$k" "$tag"
    case "$k" in
      b_cp_pre_relaunch.*)
        [[ "$CUTKILL" == yes ]] || return 0
        # THE SELECTED SPECIMEN: taken after archive publication and source removal
        # and BEFORE the relaunch. The state stays at its own path; nothing is moved.
        l_manifest "$R/h/.ae" "$R/cap/specimen.pre-relaunch.aehome.tsv"
        l_manifest "$R/h/.ae/sessions" "$R/cap/specimen.pre-relaunch.sessions.tsv"
        l_manifest "$R/h/.ae/archive" "$R/cap/specimen.pre-relaunch.archive.tsv"
        l_tmuxsnap "$SOCK" "$R/cap/specimen.pre-relaunch.tmux.txt"
        cp -Rp "$R/h/.ae" "$R/cap/specimen.pre-relaunch.aehome.copy" 2>/dev/null
        { printf 'specimen\tthe PRE-RELAUNCH clone: archive published, source session removed, relaunch not yet exec()d\n'
          printf 'taken.at\t%s\n' "$k"
          printf 'method\tthe controller SIGKILLs the whole compact process tree at this barrier, so the relaunch never runs and the live state IS the specimen at its own path\n'
        } >"$R/cap/specimen.txt"
        l_killtree "$AE_BG_PID"
        ;;
    esac
    return 0
}

arm_recovery_exec() { # <selected|contrast>
    local mode="$1"
    l_arm_begin L-COMPACT "recovery-exec-$mode" instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    CUTKILL=$( [[ "$mode" == selected ]] && echo yes || echo no )
    HOOKS=b_cp_pre_relaunch; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    CP_OBSERVE=""
    l_ae_bg 2op compact -f --digest-only cp1
    l_barriers cp1 300 rec_cb || printf 'NOTE: controller loop ended by subject death or bound\n' >>"$R/cap/barrier-order.tsv"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    cbytes 2op
    csnap 2after-compact
    # the printed Recovery: line, extracted verbatim
    local RECLINE; RECLINE="$(grep -m1 '^Recovery: ' "$R/cap/2op.stdout" || true)"
    printf '%s\n' "${RECLINE:-<no Recovery: line on stdout>}" >"$R/cap/recovery-line.txt"
    od -c "$R/cap/recovery-line.txt" >"$R/cap/recovery-line.od" 2>&1
    if [[ "$mode" == selected && -n "$RECLINE" ]]; then
        local CMD="${RECLINE#Recovery: }"
        printf '%s\n' "$CMD" >"$R/cap/recovery-command.verbatim.txt"
        ( env -i "${AE_ENV[@]}" "$L_BASH" -c "$CMD" ) >"$R/cap/3recovery.stdout" 2>"$R/cap/3recovery.stderr"
        printf '%s\n' "$?" >"$R/cap/3recovery.rc"
        { printf 'executed\tVERBATIM, as one shell command, in the arm environment\n'
          printf 'command\t%s\n' "$CMD"
          printf 'against\tthe PRE-RELAUNCH specimen at its own path (the relaunch was cut before it ran)\n'; } >"$R/cap/3recovery.invocation"
        sleep 3
        cbytes 3recovery
    fi
    csnap 3post
    carmtxt "recovery-exec-$mode" \
      "$( [[ "$mode" == selected ]] && echo 'SC-512' || echo '(none — the post-relaunch CONTRAST clone, captured for comparison; SC-822 territory, never the SC-512 specimen)' )" \
      "$( [[ "$mode" == selected ]] \
          && echo 'the compact is cut at the pre-relaunch barrier so the archive is published and the source removed but the relaunch never runs; the printed Recovery: command is then extracted and executed VERBATIM against that state' \
          || echo 'the same compact is allowed to complete, so the state captured afterwards ALREADY CONTAINS the replacement session — captured only for contrast and explicitly NOT the SC-512 specimen' )" \
      "mode	$mode" "op_rc	$(cat "$R/cap/2op.rc")" \
      "recovery_rc	$(cat "$R/cap/3recovery.rc" 2>/dev/null || echo '<not executed in this arm>')"
    l_arm_end
}

# ───────────────────────────────────────────────────────── SC-504b: SIGPIPE
arm_sigpipe() {
    l_arm_begin L-COMPACT sigpipe frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    HOOKS=""; BLOCK=""; l_arm_env
    local envblob="" v
    for v in "${AE_ENV[@]}"; do envblob+="${v}"$'\x1f'; done
    AE_L_SIGPIPE_ENV="$envblob" python3 /tmp/aelx/lib/sigpipe.py "$R/cap" \
        "$R/b/ae" compact -f --digest-only cp1 >"$R/cap/2op.stdout" 2>"$R/cap/2op.stderr"
    printf '%s\n' "$?" >"$R/cap/2op.rc"
    cp /tmp/aelx/lib/sigpipe.py "$R/cap/sigpipe-harness.py"
    sleep 4
    cbytes 2op
    csnap 3post
    carmtxt sigpipe SC-504b \
      "the producer (a real compact -f --digest-only) and an early-closing consumer are SEPARATELY SUPERVISED processes: the consumer creates an explicit pipe, hands the write end to the producer, reads exactly ONE line, closes the read end, then reaps the producer. No shell pipeline is placed over the subject" \
      "consumer_harness	sigpipe-harness.py (byte copy of what ran)" \
      "record	sigpipe-record.json — both statuses, the producer's signal disposition, and the one line the consumer read" \
      "relaunch_state	3post.tmux.txt and 3post.sessions.tsv" \
      "harness_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ─────────────────────────────────────── SC-517a/b: the relaunch reaches a terminal
arm_exit_identity_attach() {
    l_arm_begin L-COMPACT exit-identity-terminal-attach frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    HOOKS=""; BLOCK=""; l_arm_env
    l_pane_start 2op "$R/b/ae" compact -f --digest-only cp1
    # bounded POSITIVE barrier: the relaunched session's own pane content appears
    if l_pane_wait 90 "fake"; then
        l_pane_capture attached
        printf 'attach.observed\tthe relaunched session rendered inside the driver pane\n' >"$R/cap/attach.txt"
    else
        l_pane_capture attach-not-observed
        printf 'INCONCLUSIVE: the relaunched session was not observed inside the driver pane within the 90s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    fi
    # DETACH the inner client (tmux prefix, then d)
    l_ctl send-keys -t drv C-b
    l_ctl send-keys -t drv d
    l_pane_wait_rc 120 || printf 'INCONCLUSIVE: no exit status within the 120s bound after detach\n' >>"$R/cap/INCONCLUSIVE.txt"
    sleep 1
    l_pane_capture after-detach
    l_pane_stop
    cbytes 2op
    csnap 3post
    carmtxt exit-identity-terminal-attach "SC-517a SC-517b" \
      "compact runs on a REAL terminal (a driver pane on a dedicated control server), so the relaunch it execs into reaches a terminal attach; the controller then detaches the inner client with the tmux prefix and d, and the exit status is captured" \
      "op	ae compact -f --digest-only cp1 (pty)" "op_rc	$(cat "$R/cap/2op.rc" 2>/dev/null)" \
      "attach_bound_sec	90" "rc_bound_sec	120"
    l_arm_end
}

# ───────────────────────── SC-517c: the fresh session CREATES but nothing can attach
arm_exit_identity_no_terminal() {
    l_arm_begin L-COMPACT exit-identity-no-terminal frozen
    PATCHV="none (frozen, unmodified)"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    HOOKS=""; BLOCK=""; l_arm_env
    # -f so it does not exit at confirmation EOF, and neither stream is a tty
    ( cd "$R/w" && env -i "${AE_ENV[@]}" "$R/b/ae" compact -f --digest-only cp1 </dev/null ) \
        >"$R/cap/2op.stdout" 2>"$R/cap/2op.stderr"; printf '%s\n' "$?" >"$R/cap/2op.rc"
    { printf 'cwd: %s\n' "$R/w"; printf 'argv:\n'; printf '  %s\n' "$R/b/ae" compact -f --digest-only cp1
      printf 'stdin: /dev/null; stdout and stderr: regular files — no stream is a terminal\n'
      printf 'is_a_tty.stdin\tno\nis_a_tty.stdout\tno\nis_a_tty.stderr\tno\n'; } >"$R/cap/2op.invocation"
    sleep 3
    cbytes 2op
    csnap 3post
    carmtxt exit-identity-no-terminal SC-517c \
      "compact is invoked with -f (so it does not exit at confirmation EOF) and with no stream attached to a terminal; the fresh session is created but nothing can attach to it. The report bytes and the exit status are captured" \
      "op	ae compact -f --digest-only cp1 with stdin /dev/null and both outputs to files" \
      "op_rc	$(cat "$R/cap/2op.rc")" \
      "note	an unlaunchable-binary construction is deliberately NOT this row's specimen"
    l_arm_end
}

# ────────────────────────────────────────────────── SC-828: revalidation cuts
REVAL_AT=""
reval_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    cp_cb "$k" "$tag"
    case "$k" in
      "$REVAL_AT".*)
        local m="$R/h/.ae/sessions/cp1/meta"
        [[ -f "$m" ]] || return 0
        cp -p "$m" "$R/cap/$tag.meta.before.txt"
        local m0; m0="$(stat -f '%Lp' "$m")"
        local new; new="$(/usr/bin/uuidgen | tr 'A-F' 'a-f')"
        l_rewrite_preserving_mode "$m" "s/^session_id=.*\$/session_id=${new}/"
        cp -p "$m" "$R/cap/$tag.meta.after.txt"
        diff -u "$R/cap/$tag.meta.before.txt" "$R/cap/$tag.meta.after.txt" >"$R/cap/$tag.meta.diff" 2>&1
        { printf 'controller.barrier\t%s\n' "$k"
          printf 'controller.action\tthe live session meta session_id is replaced by a fresh uuid (temp + chmod-to-original-mode + rename)\n'
          printf 'controller.new_uuid\t%s\n' "$new"
          printf 'mode.before\t%s\nmode.after\t%s\n' "$m0" "$(stat -f '%Lp' "$m")"
          printf 'state.changed\tsessions/cp1/meta only; tmux, memo, events and messages untouched\n'
        } >"$R/cap/$tag.controller.txt"
        l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.after-mutation.tsv"
        ;;
    esac
    return 0
}

arm_revalidation() { # <after-answer|after-handover>
    local at="$1"
    local bar; bar=$( [[ "$at" == after-answer ]] && echo b_cp_after_answer || echo b_cp_after_handover )
    l_arm_begin L-COMPACT "revalidation-$at" instrumented
    l_use_v3; PATCHV="L-HOOKS-v3"
    cp_setup cp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    csnap 1pre
    REVAL_AT="$bar"; CP_OBSERVE=""
    HOOKS="$CPCHAN"; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2op compact -f --digest-only cp1
    l_barriers cp1 300 reval_cb || printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >"$R/cap/INCONCLUSIVE.txt"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2op.rc"
    sleep 3
    cbytes 2op
    csnap 3post
    carmtxt "revalidation-$at" SC-828 \
      "the controller mutates the live session's recorded identity at the $bar barrier — one arm per barrier — and the compact continues from there" \
      "barrier	$bar" "op_rc	$(cat "$R/cap/2op.rc")" "barrier_bound_sec	300"
    l_arm_end
}

case "${1:-all}" in
  rec)    arm_recovery_exec selected; arm_recovery_exec contrast ;;
  pipe)   arm_sigpipe ;;
  exitid) arm_exit_identity_attach; arm_exit_identity_no_terminal ;;
  reval)  arm_revalidation after-answer; arm_revalidation after-handover ;;
esac
echo DONE
