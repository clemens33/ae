#!/opt/homebrew/bin/bash
# A1 / SC-405k — LIVE tmux topology: one EXTRA runtime pane absent from the roster
# AND one roster slot whose pane is absent. Capture the rendered agents[] plus the
# tmux snapshot at each stage.
source "$(dirname "$0")/armlib.sh"
ARM=A1; CID=c20-405k-live
BASE="$AROOT/$ARM/$CID"
[[ -e "$BASE" ]] && chmod -R u+w "$BASE" 2>/dev/null; rm -rf "$BASE"; mkdir -p "$BASE"
export ARM_TMUXTMP="$BASE/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
export ARM_TMUX_TRACE=1
OUT="$BASE/cap"; mkdir -p "$OUT"
env_tab_selfcheck "$OUT/env-tab-selfcheck.txt" || { echo "HARNESS-ABORT: environment tab self-check failed"; exit 9; }

t_sandbox a1405k "fake:worker"
t_launch ta1k || { echo FAILED; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
{
  echo "arm=$ARM case=$CID rows=SC-405k"
  echo "live session=$TSESSION socket=$SOCK"
  echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(shasum -a 256 "$FROZEN_AE" | cut -d' ' -f1)"
  echo "lead_pane=$LEAD worker_pane=$WORK"
  echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$OUT/case.txt"

snap() { # <stage>
    local st="$1"
    { echo "## panes"; tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}|#{pane_pid}|#{window_index}|#{window_name}'
      echo "## windows"; tm list-windows -a -F '#{session_name}|#{window_index}|#{window_name}|#{window_panes}'
      echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}|#{session_attached}'
      echo "## clients"; tm list-clients -F '#{client_name}|#{client_tty}|#{pane_id}'
    } >"$OUT/tmux.$st.txt" 2>&1
    grep '^agent' "$META/meta" >"$OUT/roster.$st.txt" 2>&1
    # RAW PROBE under the SAME scrubbed environment the consumers run in: the exact
    # tmux query the frozen consumer makes, so the consumer's tmux view is captured
    # beside the consumer's output rather than inferred from it.
    env -i HOME="$ROOT/home" AE_HOME="$AE_HOME"         PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8         TERM=xterm-256color TMUX_TMPDIR="$ARM_TMUXTMP"         AE_TMUX_SERVER="$SOCK" AE_TMUX_SERVER_KIND=socket         /opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t "$TSESSION"         -F '#{@ae_agent}	#{pane_current_command}	#{pane_id}	#{@ae_slot}'         >"$OUT/rawprobe.$st.txt" 2>&1
    dir_manifest "$AE_HOME" >"$OUT/manifest.$st.tsv"
    consumer_battery "$OUT/consumers.$st" "$AE_HOME" "$TSESSION" "$SOCK"
}

tmux_shim_equiv "$SOCK" "$TSESSION" "$OUT/tmux-shim-equivalence.txt" \
    || { echo "HARNESS-ABORT: tmux delegate-and-log shim failed its equivalence check"; t_teardown; exit 9; }

snap s0-baseline

# PAIRED RAW CAPTURE (no comparison verdict): the identical consumer battery, identical
# live topology, run once more with the locale pinned to C instead of UTF-8. It exists
# because the frozen script has seven TAB-separated tmux format sites — two pane/alive
# walks (:3631, :4207) and five pane-id/agent resolution sites (:6488, :12151, :12170,
# :12297, :12962) — and tmux's output encoding follows the locale. Both halves are
# published raw; which (if either) is the product's intended behaviour is not decided here.
ARM_LOCALE=C consumer_battery "$OUT/consumers.s0-baseline-clocale" "$AE_HOME" "$TSESSION" "$SOCK"
{ echo "## paired-locale capture"
  echo "consumers.s0-baseline        = LANG=LC_ALL=en_US.UTF-8"
  echo "consumers.s0-baseline-clocale = LANG=LC_ALL=C"
  echo "identical AE_HOME, identical socket, identical live topology, identical argv"
} >>"$OUT/case.txt"

# manipulation 1: an EXTRA runtime pane, agent-shaped, absent from the roster
GH="$(tm split-window -d -t "$LEAD" -c "$ROOT/work" -P -F '#{pane_id}')"
tm set-option -p -t "$GH" @ae_agent "fake:ghost"
tm set-option -p -t "$GH" @ae_slot "ghost.0"
{ echo "manipulation_1=extra runtime pane $GH stamped @ae_agent=fake:ghost @ae_slot=ghost.0, NOT present in meta"
  echo "meta_has_ghost=$(grep -c 'fake:ghost' "$META/meta" || true)"; } >>"$OUT/case.txt"
snap s1-extra-pane

# manipulation 2: a roster slot whose pane is absent
tm kill-pane -t "$WORK"
{ echo "manipulation_2=killed the pane of roster slot worker.0 ($WORK); the meta entry remains"
  echo "meta_worker_line=$(grep '^agent.worker.0=' "$META/meta")"; } >>"$OUT/case.txt"
snap s2-extra-pane-and-missing-roster-pane

echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$OUT/case.txt"
for st in s0-baseline s1-extra-pane s2-extra-pane-and-missing-roster-pane; do
    echo "-- $st agents[] --"; sed -n 's/.*\("agents":\[[^]]*\]\).*/\1/p' "$OUT/consumers.$st/list-json.stdout" | head -1
done
t_teardown
echo "A1 405k DONE"
