#!/opt/homebrew/bin/bash
# A1 / SC-405k — LIVE tmux topology. Evidence written directly into the committed tree.
source "$(dirname "$0")/armlib.sh"
ARMG=A1
BASE="$AROOT/$ARMG/c20-405k-live"
[[ -e "$BASE" ]] && chmod -R u+w "$BASE" 2>/dev/null; rm -rf "$BASE"; mkdir -p "$BASE"
export ARM_TMUXTMP="$BASE/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
case_open "$ARMG" c20-405k "live"
led rows "rows=SC-405k" "template=none (live 2-agent launch + two named topology manipulations)"

t_sandbox a1405k "fake:worker"
export ARM_TMUXTMP="$BASE/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
t_launch ta1k || { led LAUNCH-FAILED; echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
{ echo "arm=$ARMG case=c20-405k rows=SC-405k clone_mode=live"
  echo "live session=$TSESSION socket=$SOCK"
  echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
  echo "lead_pane=$LEAD worker_pane=$WORK"
  echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$ACAP/case.txt"
case_env_record "$AE_HOME" "$SOCK"
env_tab_selfcheck || { led HARNESS-ABORT "reason=environment tab self-check failed"; t_teardown; exit 9; }
tmux_shim_equiv "$SOCK" "$TSESSION" || { led HARNESS-ABORT "reason=tmux shim equivalence failed"; t_teardown; exit 9; }

snap() { # <stage>
    local st="$1"
    { echo "## panes"; tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}|#{pane_pid}|#{window_index}|#{window_name}'
      echo "## windows"; tm list-windows -a -F '#{session_name}|#{window_index}|#{window_name}|#{window_panes}'
      echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}|#{session_attached}'
      echo "## clients"; tm list-clients -F '#{client_name}|#{client_tty}|#{pane_id}'
    } >"$ACAP/tmux.$st.txt" 2>&1
    grep '^agent' "$META/meta" >"$ACAP/roster.$st.txt" 2>&1
    env -i HOME="$ROOT/home" AE_HOME="$AE_HOME" \
        PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 \
        TERM=xterm-256color TMUX_TMPDIR="$ARM_TMUXTMP" \
        AE_TMUX_SERVER="$SOCK" AE_TMUX_SERVER_KIND=socket \
        /opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t "$TSESSION" \
        -F '#{@ae_agent}	#{pane_current_command}	#{pane_id}	#{@ae_slot}' \
        >"$ACAP/rawprobe.$st.txt" 2>&1
    dir_manifest "$AE_HOME" >"$ACAP/manifest.$st.tsv"
    led stage-snapshot "stage=$st" "tmux_sha256=$(sha "$ACAP/tmux.$st.txt")" \
        "rawprobe_sha256=$(sha "$ACAP/rawprobe.$st.txt")" "manifest_sha256=$(sha "$ACAP/manifest.$st.tsv")"
    local AE="$FROZEN_AE" B="$HARNESS_BASH"
    run_consumer "$st/list"          "$AE_HOME" "$SOCK" -- "$B" "$AE" list
    run_consumer "$st/list-json"     "$AE_HOME" "$SOCK" -- "$B" "$AE" list --json
    run_consumer "$st/list-all"      "$AE_HOME" "$SOCK" -- "$B" "$AE" list --all
    run_consumer "$st/status"        "$AE_HOME" "$SOCK" -- "$B" "$AE" status "$TSESSION"
    run_consumer "$st/next"          "$AE_HOME" "$SOCK" -- "$B" "$AE" next
    run_consumer "$st/agents"        "$AE_HOME" "$SOCK" -- "$META/agents"
    run_consumer "$st/requests-all"  "$AE_HOME" "$SOCK" -- "$META/requests" all
}
mkdir -p "$ACAP/out/s0-baseline" "$ACAP/out/s1-extra-pane" "$ACAP/out/s2-extra-pane-and-missing-roster-pane" "$ACAP/out/s0-baseline-clocale"
snap s0-baseline

# PAIRED RAW CAPTURE (no comparison verdict): identical battery, identical topology,
# locale pinned to C instead of UTF-8. The frozen script has seven TAB-separated tmux
# format sites — two pane/alive walks (:3631, :4207) and five pane-id/agent resolution
# sites (:6488, :12151, :12170, :12297, :12962) — and tmux's output encoding follows the
# locale. Both halves are published raw.
led paired-locale-capture-START "stage=s0-baseline-clocale" "locale=C"
ARM_LOCALE=C run_consumer "s0-baseline-clocale/list"       "$AE_HOME" "$SOCK" -- "$HARNESS_BASH" "$FROZEN_AE" list
ARM_LOCALE=C run_consumer "s0-baseline-clocale/list-json"  "$AE_HOME" "$SOCK" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
ARM_LOCALE=C run_consumer "s0-baseline-clocale/status"     "$AE_HOME" "$SOCK" -- "$HARNESS_BASH" "$FROZEN_AE" status "$TSESSION"
ARM_LOCALE=C run_consumer "s0-baseline-clocale/next"       "$AE_HOME" "$SOCK" -- "$HARNESS_BASH" "$FROZEN_AE" next
ARM_LOCALE=C run_consumer "s0-baseline-clocale/agents"     "$AE_HOME" "$SOCK" -- "$META/agents"
led paired-locale-capture-COMPLETE "stage=s0-baseline-clocale"

GH="$(tm split-window -d -t "$LEAD" -c "$ROOT/work" -P -F '#{pane_id}')"
tm set-option -p -t "$GH" @ae_agent "fake:ghost"
tm set-option -p -t "$GH" @ae_slot "ghost.0"
led manipulation-1 "extra runtime pane $GH stamped @ae_agent=fake:ghost @ae_slot=ghost.0, absent from meta" \
    "meta_mentions_ghost=$(grep -c 'fake:ghost' "$META/meta" || true)"
echo "manipulation_1=extra runtime pane $GH stamped fake:ghost/ghost.0, NOT in meta" >>"$ACAP/case.txt"
snap s1-extra-pane

tm kill-pane -t "$WORK"
led manipulation-2 "killed the pane of roster slot worker.0 ($WORK); meta entry left in place" \
    "meta_worker_line=$(grep '^agent.worker.0=' "$META/meta")"
echo "manipulation_2=killed the pane of roster slot worker.0 ($WORK); meta entry remains" >>"$ACAP/case.txt"
snap s2-extra-pane-and-missing-roster-pane

{ echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "admissibility_note=The raw probe runs the exact tmux query the frozen consumer makes,"
  echo "  from the SAME scrubbed environment and socket the consumers are given, and the session's"
  echo "  own agents helper is in the battery. The per-consumer tmuxtrace records the effective"
  echo "  AE_TMUX_SERVER/kind, the effective locale, and the DELEGATED argv for every invocation."
  echo "  No claim is made about what any rendering should be."
} >>"$ACAP/case.txt"
led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
printf 'c20-405k\tSC-405k\tlive\tno-template (live 2-agent launch + two named topology manipulations)\n' >>"$ADEST/$ARMG/ledger.tsv"
t_teardown
echo "A1 405k DONE"
