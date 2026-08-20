#!/opt/homebrew/bin/bash
# SC-832c arms: one sandbox per (cut, consumer) pair, so every consumer sees an
# INDEPENDENT post-crash state — including the tmux side, which cannot be cloned.
set -uo pipefail
source /tmp/aelx/lib/c832-lib.sh

declare -A BARRIER=(
  [entry]=b_rn_locked_entry
  [tmux]=b_rn_tmux_renamed
  [dir]=b_rn_dir_moved
  [meta]=b_rn_meta_updated
)
declare -A CUTDESC=(
  [entry]="inside the two-lock region, before any check the rename then mutates (the CONTROL cut)"
  [tmux]="after tmux rename-session and the main-window rename, before the state directory move"
  [dir]="after the state directory move, before the meta rewrite"
  [meta]="after meta session= is rewritten, before workspace.md is regenerated"
)

KILLED=""
cut_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      "$CUTBAR".*)
        [[ -n "$KILLED" ]] && return 0
        KILLED=yes
        c_tuple at-cut
        led cut barrier "$k"
        # DESCRIPTORS ARE RELEASED BY DEATH, never by a resumed process.
        l_killtree "$AE_BG_PID"
        led cut method "SIGKILL to the whole rename process tree at the barrier"
        ;;
    esac
    return 0
}

run_arm() { # <cut> <consumer>
    local cut="$1" cons="$2"
    local arm="cut-${cut}-consumer-${cons}"
    l_arm_begin L-832C "$arm" instrumented
    l_use_v4; PATCHV="L-HOOKS-v4"
    : >"$R/cap/ledger.tsv"
    led setup arm "$arm"
    led setup cut "$cut — ${CUTDESC[$cut]}"
    led setup barrier "${BARRIER[$cut]}"
    c_config "$R"; { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch-proj  --local proj;  sleep 3
    l_ae 0launch-projx --local projx; sleep 3
    l_arm_preflight proj || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    led setup topology "proj + projx (prefix-sibling pair) on one recorded server"
    c_tuple 1pre
    l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"

    # ── the rename, cut by DEATH ─────────────────────────────────────────────
    CUTBAR="${BARRIER[$cut]}"; KILLED=""
    HOOKS="$CUTBAR"; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 2rename rename proj proj2
    local SUBJECT=$AE_BG_PID
    led cut subject.pid "$SUBJECT"
    l_barriers proj 240 cut_cb || printf 'NOTE: the controller loop ended by subject death or bound\n' >>"$R/cap/barrier-order.tsv"
    wait "$SUBJECT" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/2rename.rc"
    led cut rename.rc "$(cat "$R/cap/2rename.rc")"
    sleep 2
    if [[ -z "$KILLED" ]]; then
        { printf 'ARM INVALID: the cut barrier %s was never reached, so the writer was never killed\n' "$CUTBAR"
          printf 'at the intended point and nothing here is a post-crash observation.\n'
        } >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1
    fi

    # ── preconditions for a fresh process ────────────────────────────────────
    if ! c_prove_dead_and_free "$SUBJECT"; then
        { printf 'ARM INVALID: the death-and-descriptor precondition was not met.\n'
          printf 'Either the subject was still alive or a lifecycle lock could not be reacquired, so a\n'
          printf 'fresh process would not have been re-entering a crashed state. See death-and-descriptors.txt.\n'
        } >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1
    fi
    c_tuple 3post-crash
    l_manifest "$R/h/.ae" "$R/cap/3post-crash.aehome.tsv"

    # ── the ABILITY-TO-FAIL reading: do the identity sources DISAGREE here? ──
    # Derived from the captured row, never asserted: an arm that cannot observe a
    # mixed generation cannot support a claim that none survives.
    local T="$R/cap/tuple.3post-crash.row"
    local TN DN MN
    TN="$(grep -m1 '^tmux.names' "$T" | cut -f2)"
    DN="$(grep -m1 '^dir.names' "$T" | cut -f2)"
    MN="$(grep -m1 '^meta.sessions' "$T" | cut -f2)"
    local MIXED=no
    [[ "$TN" == "$DN" && "$DN" == "$MN" ]] || MIXED=yes
    led mixed tmux.names "$TN"
    led mixed dir.names "$DN"
    led mixed meta.sessions "$MN"
    led mixed sources.disagree "$MIXED"

    # ── ONE consumer, a FRESH PROCESS, on its OWN post-crash clone ───────────
    local H; H="$(c_clone "$cons")"
    led consumer clone "$H"
    l_manifest "$H" "$R/cap/4before.clone.tsv"
    case "$cons" in
      list)
        c_run_fresh "$H" 4list list --json
        c_run_fresh "$H" 4list-all list --all ;;
      status)
        c_run_fresh "$H" 4status-proj  status proj
        c_run_fresh "$H" 4status-proj2 status proj2 ;;
      stop|stop-other)
        local target=proj2
        [[ "$cons" == stop-other ]] && target=proj
        led consumer stop.target "$target"
        c_run_fresh "$H" 4stop stop -y "$target" ;;
    esac
    sleep 2
    l_manifest "$H" "$R/cap/5after.clone.tsv"
    diff -u "$R/cap/4before.clone.tsv" "$R/cap/5after.clone.tsv" >"$R/cap/no-mutation.clone.diff" 2>&1
    local MUT; MUT="$(grep -c '^[+-][^+-]' "$R/cap/no-mutation.clone.diff" 2>/dev/null)"; MUT="${MUT:-0}"
    led consumer clone.manifest.changed.lines "$MUT"
    c_tuple 6after-consumer "$H"
    l_manifest "$R/h/.ae" "$R/cap/6after.origin.aehome.tsv"
    diff -u "$R/cap/3post-crash.aehome.tsv" "$R/cap/6after.origin.aehome.tsv" >"$R/cap/no-mutation.origin.diff" 2>&1
    local MUTO; MUTO="$(grep -c '^[+-][^+-]' "$R/cap/no-mutation.origin.diff" 2>/dev/null)"; MUTO="${MUTO:-0}"
    led consumer origin.manifest.changed.lines "$MUTO"

    { printf 'arm\t%s\nsection\tL-832C\n' "$arm"
      printf 'roster_ids\tSC-832c (and the D20 gate that carries the same hold)\n'
      printf 'cut\t%s\n' "$cut"
      printf 'cut_site\t%s\n' "${CUTDESC[$cut]}"
      printf 'barrier\t%s\n' "$CUTBAR"
      printf 'consumer\t%s\n' "$cons"
      printf 'construction\tthe rename is interrupted by SIGKILL to its whole process tree at the named cut, so the lifecycle descriptors are released BY DEATH and never by a resumed process; a FRESH PROCESS then runs one consumer against its OWN independent post-crash clone\n'
      printf 'independence\tone sandbox per (cut, consumer) pair — the tmux server cannot be cloned, so each consumer gets a whole separate run rather than a shared server\n'
      printf 'hook_patch_version\t%s\n' "$PATCHV"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'subject_pid\t%s\n' "$SUBJECT"
      printf 'rename_rc\t%s\n' "$(cat "$R/cap/2rename.rc")"
      printf 'precondition_met\t%s\n' "$(grep -m1 '^precondition.met' "$R/cap/death-and-descriptors.txt" | cut -f2)"
      printf 'postcrash.tmux_names\t%s\n' "$TN"
      printf 'postcrash.dir_names\t%s\n' "$DN"
      printf 'postcrash.meta_sessions\t%s\n' "$MN"
      printf 'postcrash.sources_disagree\t%s\n' "$MIXED"
      printf 'clone_manifest_changed_lines\t%s\n' "$MUT"
      printf 'origin_manifest_changed_lines\t%s\n' "$MUTO"
      printf 'consumer_rcs\t%s\n' "$(for f in "$R"/cap/4*.rc; do [[ -e "$f" ]] && printf '%s=%s ' "$(basename "$f" .rc)" "$(cat "$f")"; done)"
      printf 'OBSERVATION\ttuple.*.txt at 1pre, at-cut, 3post-crash and 6after-consumer; death-and-descriptors.txt; the consumer stdout/stderr/rc; and the two no-mutation diffs. No verdict is stated here.\n'
    } >"$R/cap/ARM.txt"
    l_arm_end
    return 0
}

for spec in "$@"; do
    run_arm "${spec%%:*}" "${spec##*:}"
done
echo DONE
