#!/opt/homebrew/bin/bash
# D01 / SC-1306a — list reader vs live writer. b0-design.md Design 2.
# Hook H_LIST_META_CAPTURED at the RUNNING-session site, immediately after meta_blob is
# read. At the barrier the controller invokes the REAL frozen `goal` helper ONCE — one
# logical writer operation that rewrites goal in meta AND emits the goal event.
source "$(dirname "$0")/dlib.sh"
ARMG=D
mkdir -p "$ADEST/$ARMG"
[[ -f "$ADEST/$ARMG/ledger.tsv" ]] || printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
printf 'd01-list-vs-goal-writer\tD01,SC-1306a\tG1\thealthy\n' >>"$ADEST/$ARMG/ledger.tsv"

GOAL_TEXT="D01 controller goal written at the barrier"
mutate_goal() { # <ae-home> <sock>  — THE CONTROLLER's named writer-shaped mutation
    local aehome="$1" sock="$2" sess
    sess="$(ls "$aehome/sessions" | head -1)"
    local srvpid; srvpid="$(command tmux -S "$sock" display-message -p '#{pid}' 2>/dev/null)"
    local lead; lead="$(command tmux -S "$sock" list-panes -a -F '#{pane_id} #{@ae_agent}' | awk 'NR==1{print $1}')"
    { echo "mutation=ONE invocation of the session's OWN real goal helper"
      echo "helper=$aehome/sessions/$sess/goal"
      echo "text=$GOAL_TEXT"
      echo "run_as_pane=$lead (the real pane environment a live agent has)"
      echo "utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
      echo "meta goal BEFORE: $(grep '^goal=' "$aehome/sessions/$sess/meta" || echo '(absent)')"
      echo "events lines BEFORE: $(wc -l <"$aehome/sessions/$sess/events.jsonl" | tr -d ' ')"
    } >"$ACAP/controller-mutation.txt"
    env TMUX="${sock},${srvpid},0" TMUX_PANE="$lead" \
        HOME="$(dirname "$aehome")" AE_HOME="$aehome" TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 \
        PATH="${TMUX_SHIM_DIR}:${FLOCK_SHIM_DIR}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
        AE_REAL_TMUX=/opt/homebrew/bin/tmux AE_REAL_FLOCK=/opt/homebrew/bin/flock \
        AE_FLOCK_SPY_LOG="$ACAP/out/controller.flockspy" \
        AE_TMUX_SHIM_LOG="$ACAP/out/controller.tmuxtrace" \
        "$aehome/sessions/$sess/goal" "$GOAL_TEXT" >"$ACAP/out/controller-goal.stdout" 2>"$ACAP/out/controller-goal.stderr"
    local rc=$?
    { echo "controller_goal_rc=$rc"
      echo "meta goal AFTER: $(grep '^goal=' "$aehome/sessions/$sess/meta" || echo '(absent)')"
      echo "events lines AFTER: $(wc -l <"$aehome/sessions/$sess/events.jsonl" | tr -d ' ')"
      echo "last event: $(tail -1 "$aehome/sessions/$sess/events.jsonl")"
    } >>"$ACAP/controller-mutation.txt"
    led controller-mutation "helper=goal" "rc=$rc" "artifact_sha256=$(sha "$ACAP/controller-mutation.txt")"
}
no_mutation() { led controller-mutation "none=this is the read-only control arm"; }

d01_case() { # <case-id> <arm-kind>   arm-kind: barrier | twin
    local cid="$1" kind="$2"
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$kind"
    led rows "rows=D01,SC-1306a" "template=G1/healthy" "design=b0-design.md Design 2"
    local aehome="$base/home/.ae"
    t_clone G1 healthy "$aehome" rw || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$aehome/sessions" | head -1)"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"; exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/G1/_meta/healthy.txt" | cut -d= -f2-)"
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    build_live_topology "$aehome" "$sock" "$sess"
    { echo "arm=$ARMG case=$cid arm_kind=$kind design=b0-design.md Design 2 (D01) rows=D01,SC-1306a"
      echo "template=G1/healthy clone_mode=rw session=$sess"
      echo "clone_fingerprint=$cf clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "hook=H_LIST_META_CAPTURED (running-session site, immediately after meta_blob is read)"
      echo "unmodified_ae_sha256=$(sha "$FROZEN_AE") hooked_ae_sha256=$(sha "$HOOKED_AE") patch_sha256=$(sha "$HOOK_PATCH")"
      echo "flock_spy=$FLOCK_SHIM_DIR/flock sha256=$(sha "$FLOCK_SHIM_DIR/flock") (pure delegate-and-log)"
      echo "tmux_socket=$sock"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }
    if [[ "$kind" == barrier ]]; then
        hook_inactive_equiv "$aehome" "$sock" "$sess" || { led HARNESS-ABORT "reason=inactive-hook equivalence"; return 1; }
        cp "$HOOK_PATCH" "$ACAP/hook.patch"; led hook-patch-published "artifact_sha256=$(sha "$ACAP/hook.patch")"
    fi
    d_snapshot before "$aehome" "$sock"
    d_plain_run "baseline-list-json" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
    case "$kind" in
      barrier)
        d_barrier_run H_LIST_META_CAPTURED "barrier-list-json" "$aehome" "$sock" mutate_goal \
            -- "$HARNESS_BASH" "$HOOKED_AE" list --json
        ;;
      twin)
        # CONTROLLER-ONLY TWIN: the same mutation, alone, no hooked reader.
        led twin-note "the identical controller mutation performed alone so its own effect can be subtracted"
        mutate_goal "$aehome" "$sock"
        ;;
    esac
    d_snapshot after "$aehome" "$sock"
    d_plain_run "rerun-list-json" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
    d_trace_run "trace-list-json" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    { echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid ($kind): manifest_diff_lines=$(grep '^manifest_diff_lines=' "$ACAP/case.txt" | cut -d= -f2-)"
}
d01_case d01-list-vs-goal-writer barrier
d01_case d01-controller-only-twin twin
printf 'd01-controller-only-twin\tD01,SC-1306a\tG1\thealthy\n' >>"$ADEST/$ARMG/ledger.tsv"
echo "D01 DONE"
