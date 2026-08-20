#!/opt/homebrew/bin/bash
# ARM 2 — MANIPULATION: none applied to the pane after launch; the fake agent
# stays alive and its pane stays static. Shortened stale threshold + nudge cap.
source "$(dirname "$0")/arm.sh" a2
echo "arm_manipulation=none-after-launch (fake agent alive, pane left static)" >>"$RUNMAN"
cap_all t0-post-launch
OBS_CYCLES=64
echo "observation_window_cycles=$OBS_CYCLES" >>"$RUNMAN"
for ((i=1;i<=OBS_CYCLES;i++)); do
    cross_cycle "$(printf 'obs-c%02d' "$i")" 60 || {
        echo "OUTCOME=INCONCLUSIVE reason=observation-window-barrier-timeout at cycle $i" >>"$RUNMAN"; break; }
done
cap_all final
{ echo "agent_stdin_log_sha256=$(shasum -a 256 "$AEFAKE_LOG" | cut -d' ' -f1)"
  echo "agent_stdin_log_bytes=$(stat -f %z "$AEFAKE_LOG")"
  echo "final_aefake_pids=$(pgrep -x aefake | tr '\n' ' ')"
  echo "end_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$RUNMAN"
cp "$AEFAKE_LOG" "$CAP/agent-stdin.log"
cp "$META/meta" "$CAP/meta.final.txt" 2>/dev/null || true
twd_teardown
echo "ARM a2 DONE"
