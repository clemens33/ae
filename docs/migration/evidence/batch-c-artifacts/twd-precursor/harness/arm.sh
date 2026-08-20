#!/opt/homebrew/bin/bash
# T-WD precursor arm runner. Value-blind: manipulation + barriers + capture only.
source "$(dirname "$0")/lib.sh"

ARM_ID="$1"
SESSION="twd${ARM_ID}"

twd_sandbox "$ARM_ID"

# --- pacing knobs (recorded verbatim in the run manifest) ---
export AE_WATCHDOG_INTERVAL_SEC=5
export AE_WATCHDOG_STALE_MIN=1
export AE_WATCHDOG_MAX_NUDGES=2
export AE_WATCHDOG_THROTTLE_ALERT_CYCLES=2
export AE_WATCHDOG_TG_SUPERVISE_SEC=0
export AE_SEND_DEFER_SEC=5
export AE_NO_AUTOSTART=1

RUNMAN="$CAP/run-manifest.txt"
{
  echo "arm_id=$ARM_ID"
  echo "session=$SESSION"
  echo "frozen_sha=$FROZEN_SHA"
  echo "frozen_ae_path=$FROZEN_AE"
  echo "frozen_ae_sha256=$(shasum -a 256 "$FROZEN_AE" | cut -d' ' -f1)"
  echo "producer=generated-watchdog-from-this-launch (<meta>/watchdog _run)"
  echo "interpreter=$HARNESS_BASH"
  echo "interpreter_version=$("$HARNESS_BASH" --version | head -1)"
  echo "interpreter_sha256=$(shasum -a 256 "$(readlink -f "$HARNESS_BASH" 2>/dev/null || echo "$HARNESS_BASH")" | cut -d' ' -f1)"
  echo "tmux_bin=$(command -v tmux)"
  echo "tmux_version=$(command tmux -V)"
  echo "fake_agent_bin=$FAKE_BIN"
  echo "fake_agent_sha256=$(shasum -a 256 "$FAKE_BIN" | cut -d' ' -f1)"
  echo "fake_agent_src_sha256=$(shasum -a 256 /tmp/aecx/src/aefake.c | cut -d' ' -f1)"
  echo "uname=$(uname -srm)"
  echo "clock_shim=none"
  echo "hook_patch=none (unmodified 72c7293 copy)"
  echo "knob.AE_WATCHDOG_INTERVAL_SEC=$AE_WATCHDOG_INTERVAL_SEC"
  echo "knob.AE_WATCHDOG_STALE_MIN=$AE_WATCHDOG_STALE_MIN"
  echo "knob.AE_WATCHDOG_MAX_NUDGES=$AE_WATCHDOG_MAX_NUDGES"
  echo "knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES=$AE_WATCHDOG_THROTTLE_ALERT_CYCLES"
  echo "knob.AE_WATCHDOG_TG_SUPERVISE_SEC=$AE_WATCHDOG_TG_SUPERVISE_SEC"
  echo "knob.AE_SEND_DEFER_SEC=$AE_SEND_DEFER_SEC"
  echo "env.TZ=$TZ"; echo "env.LANG=$LANG"; echo "env.LC_ALL=$LC_ALL"
  echo "env.SHELL=$SHELL"; echo "env.PATH=$PATH"
  echo "env.HOME=$HOME"; echo "env.AE_HOME=$AE_HOME"
  echo "env.AE_TMUX_SERVER=$AE_TMUX_SERVER"
  echo "start_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$RUNMAN"

cd "$ROOT/work" || exit 1
LAUNCH_RC=0
"$HARNESS_BASH" "$FROZEN_AE" --local "$SESSION" </dev/null >"$CAP/ae-launch.out" 2>"$CAP/ae-launch.err" || LAUNCH_RC=$?
echo "launch_rc=$LAUNCH_RC" >>"$RUNMAN"

META="$AE_HOME/sessions/$SESSION"
export META
[[ -f "$META/meta" ]] || { echo "HARNESS-ABORT: no meta at $META" >>"$RUNMAN"; exit 9; }
cp "$META/meta" "$CAP/meta.at-launch.txt"

WD_PANE="$(tm list-panes -a -F '#{pane_id} #{@ae_agent}' | awk '$2=="_watchdog"{print $1;exit}')"
AGENT_PANE="$(tm list-panes -a -F '#{pane_id} #{@ae_agent}' | awk '$2=="fake:probe"{print $1;exit}')"
export WD_PANE AGENT_PANE
{ echo "wd_pane=$WD_PANE"; echo "agent_pane=$AGENT_PANE"; } >>"$RUNMAN"

# INSTRUMENT SELF-CHECK (admissibility): the barrier must answer BOTH ways on this
# run before any capture is taken through it. Positive control = the next cycle is
# reached; negative control = an unreachable count times out as INCONCLUSIVE.
wd_snapshot
_ic_n="$(wd_cycles)"
_ic_pos=0; wait_wd_cycles $((_ic_n + 1)) 60 || _ic_pos=$?
_ic_neg=0; wait_wd_cycles 999999 8 || _ic_neg=$?
{ echo "instrument_selfcheck_positive_rc=$_ic_pos (harness positive control: barrier reached)"
  echo "instrument_selfcheck_negative_rc=$_ic_neg (harness negative control: bounded timeout)"
} >>"$RUNMAN"
if (( _ic_pos != 0 || _ic_neg != 3 )); then
    echo "HARNESS-ABORT: cycle-barrier instrument failed its self-check" >>"$RUNMAN"
    twd_teardown; exit 9
fi

# Cross one watchdog cycle barrier, then capture. rc 3 = INCONCLUSIVE (timeout).
cross_cycle() { # <label> <timeout-sec>
    local lbl="$1" tmo="${2:-60}" now target rc=0
    wd_snapshot
    now="$(wd_cycles)"
    target=$((now + 1))
    wait_wd_cycles "$target" "$tmo" || rc=$?
    wd_snapshot
    cp "$CAP/watchdog.log" "$CAP/watchdog.$lbl.log" 2>/dev/null || true
    cap_all "$lbl"
    if (( rc == 3 )); then
        echo "barrier=$lbl OUTCOME=INCONCLUSIVE reason=cycle-barrier-timeout after ${tmo}s (cycles seen=$(wd_cycles), wanted=$target)" >>"$RUNMAN"
    else
        echo "barrier=$lbl cycles=$(wd_cycles)" >>"$RUNMAN"
    fi
    return $rc
}
export -f cross_cycle 2>/dev/null || true
