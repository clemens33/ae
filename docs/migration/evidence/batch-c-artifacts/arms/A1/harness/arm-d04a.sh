#!/opt/homebrew/bin/bash
# D04a / SC-1306b — status pane-set cut. b0-design.md Design 5.
# A DELEGATING tmux shim captures the REAL list-panes result, signals H_STATUS_PANESET and
# BLOCKS before replay; the controller then kills one listed pane and creates one new pane,
# and only then releases. Both mandatory topology arms: (a) exact-name only, (b) a
# prefix-sibling session on the same server. Controller-only twins. The shim's inactive
# equivalence is proven on the same stable topology before its active barrier is used.
source "$(dirname "$0")/dlib.sh"
ARMG=D
mkdir -p "$ADEST/$ARMG"
[[ -f "$ADEST/$ARMG/ledger.tsv" ]] || printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

d04a_case() { # <case-id> <arm-kind: barrier|twin> <topology: exact|prefix>
    local cid="$1" kind="$2" topo="$3"
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$kind"
    led rows "rows=D04a,SC-1306b" "design=b0-design.md Design 5" "topology=$topo"
    t_sandbox "d04a${topo}${kind}" "fake:worker"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    t_launch "td04a${topo:0:3}${kind:0:3}" || { led LAUNCH-FAILED; return 1; }
    local sib=""
    if [[ "$topo" == prefix ]]; then
        sib="${TSESSION}extra"
        tm new-session -d -s "$sib" "$FAKE_BIN"
        led prefix-sibling-created "sibling=$sib" \
            "note=its name EXTENDS the target's name, so a prefix match would capture it"
    fi
    { echo "arm=$ARMG case=$cid arm_kind=$kind design=b0-design.md Design 5 (D04a) rows=D04a,SC-1306b"
      echo "topology=$topo target_session=$TSESSION sibling=${sib:-<none>}"
      echo "tmux_socket=$SOCK"
      echo "tmux_shim=/tmp/aecx/shim-tmux/tmux sha256=$(sha /tmp/aecx/shim-tmux/tmux)"
      echo "shim_default_mode=pure delegate-and-log; the active barrier exists only when AE_TMUX_BARRIER_DIR is set"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$AE_HOME" "$SOCK"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$SOCK" "$TSESSION" || { led HARNESS-ABORT "reason=tmux shim inactive equivalence"; return 1; }
    d_snapshot before "$AE_HOME" "$SOCK"
    { echo "## pane set before anything"; tm list-panes -s -t "$TSESSION" -F '#{pane_id}|#{@ae_agent}|#{pane_current_command}'
      echo "## all panes on the server"; tm list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{pane_current_command}'
    } >"$ACAP/paneset.before.txt" 2>&1
    led paneset-before "artifact_sha256=$(sha "$ACAP/paneset.before.txt")"

    mutate_paneset() {
        local victim newpane
        victim="$(tm list-panes -s -t "$TSESSION" -F '#{pane_id} #{@ae_agent}' | awk '$2!=""{print $1; exit}')"
        { echo "mutation=kill ONE listed pane and create ONE new pane, while the reader is blocked"
          echo "killed_pane=$victim"; } >"$ACAP/controller-mutation.txt"
        tm kill-pane -t "$victim" 2>&1 | sed 's/^/killrc: /' >>"$ACAP/controller-mutation.txt"
        newpane="$(tm new-window -d -t "${TSESSION}:" -P -F '#{pane_id}' "$FAKE_BIN" 2>/dev/null)"
        tm set-option -p -t "$newpane" @ae_agent "fake:added-after-the-cut" 2>/dev/null
        { echo "created_pane=$newpane stamped @ae_agent=fake:added-after-the-cut"
          echo "utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
          echo "## pane set immediately after the mutation"
          tm list-panes -s -t "$TSESSION" -F '#{pane_id}|#{@ae_agent}|#{pane_current_command}'
        } >>"$ACAP/controller-mutation.txt" 2>&1
        led controller-mutation "killed=$victim" "created=$newpane" "artifact_sha256=$(sha "$ACAP/controller-mutation.txt")"
    }

    if [[ "$kind" == barrier ]]; then
        local bd="$ARM_TMUXTMP/barrierdir"; rm -rf "$bd"; mkdir -p "$bd"
        led barrier-ARMED "hook=H_STATUS_PANESET (tmux shim, list-panes)" "barrier_dir=$bd"
        : >"$ACAP/out/barrier-status.tmuxtrace"; : >"$ACAP/out/barrier-status.flockspy"
        ( env -i "HOME=$ROOT/home" "AE_HOME=$AE_HOME" \
            "PATH=/tmp/aecx/shim-tmux:/tmp/aecx/shim-flock:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
            "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color" \
            "TMUX_TMPDIR=$ARM_TMUXTMP" "AE_TMUX_SERVER=$SOCK" "AE_TMUX_SERVER_KIND=socket" \
            "AE_REAL_TMUX=/opt/homebrew/bin/tmux" "AE_REAL_FLOCK=/opt/homebrew/bin/flock" \
            "AE_TMUX_SHIM_LOG=$ACAP/out/barrier-status.tmuxtrace" \
            "AE_FLOCK_SPY_LOG=$ACAP/out/barrier-status.flockspy" \
            "AE_TMUX_BARRIER_DIR=$bd" "AE_TMUX_BARRIER_CMD=list-panes" \
            "$HARNESS_BASH" "$FROZEN_AE" status "$TSESSION" </dev/null \
            >"$ACAP/out/barrier-status.stdout" 2>"$ACAP/out/barrier-status.stderr"; echo $? >"$bd/rc" ) &
        local bp=$! t0 reached=0; t0=$(/bin/date -u +%s)
        while (( $(/bin/date -u +%s) - t0 < 60 )); do
            [[ -e "$bd/paneset.reached" ]] && { reached=1; break; }
            kill -0 "$bp" 2>/dev/null || break
            sleep 0.2
        done
        led barrier-REACHED "reached=$reached" "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
        if (( reached == 0 )); then
            led OUTCOME-INCONCLUSIVE "reason=list-panes barrier not reached within 60s"
            wait "$bp" 2>/dev/null
        else
            cp "$bd/captured.stdout" "$ACAP/barrier-captured-paneset.stdout" 2>/dev/null
            led barrier-captured-paneset "sha256=$(sha "$ACAP/barrier-captured-paneset.stdout")" \
                "lines=$(wc -l <"$ACAP/barrier-captured-paneset.stdout" 2>/dev/null | tr -d ' ')"
            mutate_paneset
            : >"$bd/paneset.release"
            led barrier-RELEASED
            wait "$bp" 2>/dev/null
            cp "$bd/barrier.log" "$ACAP/barrier.log" 2>/dev/null
            local rc; rc="$(cat "$bd/rc" 2>/dev/null || echo '?')"
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "barrier-status" "$rc" \
                "$(sha "$ACAP/out/barrier-status.stdout")" "$(stat -f %z "$ACAP/out/barrier-status.stdout")" \
                "$(sha "$ACAP/out/barrier-status.stderr")" "$(stat -f %z "$ACAP/out/barrier-status.stderr")" \
                "$(sha "$ACAP/out/barrier-status.tmuxtrace")" "$(wc -l <"$ACAP/out/barrier-status.tmuxtrace" | tr -d ' ')" \
                "barrier:H_STATUS_PANESET" "ae status $TSESSION" >>"$ACAP/consumers.tsv"
            led barrier-consumer-COMPLETE "rc=$rc" "stdout_sha256=$(sha "$ACAP/out/barrier-status.stdout")"
        fi
    else
        led twin-note "controller-only twin: the identical pane-set mutation with no reader blocked in it"
        mutate_paneset
    fi
    { echo "## pane set after"; tm list-panes -s -t "$TSESSION" -F '#{pane_id}|#{@ae_agent}|#{pane_current_command}'
      echo "## all panes on the server"; tm list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{pane_current_command}'
    } >"$ACAP/paneset.after.txt" 2>&1
    led paneset-after "artifact_sha256=$(sha "$ACAP/paneset.after.txt")"
    d_plain_run "clean-rerun-status" "$AE_HOME" "$SOCK" -- "$HARNESS_BASH" "$FROZEN_AE" status "$TSESSION"
    d_snapshot after "$AE_HOME" "$SOCK"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    { echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    t_teardown
    echo "  $cid ($kind/$topo) done"
}
reg() { printf '%s\tD04a,SC-1306b\tlive\tno-template (live launch + pane-set cut)\n' "$1" >>"$ADEST/$ARMG/ledger.tsv"; }
reg d04a-exact-barrier;  d04a_case d04a-exact-barrier  barrier exact
reg d04a-exact-twin;     d04a_case d04a-exact-twin     twin    exact
reg d04a-prefix-barrier; d04a_case d04a-prefix-barrier barrier prefix
reg d04a-prefix-twin;    d04a_case d04a-prefix-twin    twin    prefix
echo "D04a DONE"
