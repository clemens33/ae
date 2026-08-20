#!/opt/homebrew/bin/bash
# ARM 1 — MANIPULATION: kill only the fake-agent child of the pane.
source "$(dirname "$0")/arm.sh" a1
echo "arm_manipulation=kill-only-the-fake-agent-child (pane shell returns to foreground)" >>"$RUNMAN"
cap_all t0-post-launch
cross_cycle base-c1 60
cross_cycle base-c2 60
cap_all pre-manipulation

PIDS="$(pgrep -x aefake | tr '\n' ' ')"
{ echo "pre_manipulation_aefake_pids=$PIDS"
  echo "pre_manipulation_pane_current_command=$(tm display-message -p -t "$AGENT_PANE" '#{pane_current_command}')"
  echo "pre_manipulation_pane_pid=$(tm display-message -p -t "$AGENT_PANE" '#{pane_pid}')"
  echo "manipulation_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >>"$RUNMAN"
for p in $PIDS; do kill -TERM "$p" 2>/dev/null; done
_t0="$(date -u +%s)"; _gone=0
while (( $(date -u +%s) - _t0 < 20 )); do
    pgrep -x aefake >/dev/null 2>&1 || { _gone=1; break; }
    sleep 1
done
{ echo "post_manipulation_aefake_gone=$_gone"
  echo "post_manipulation_pane_current_command=$(tm display-message -p -t "$AGENT_PANE" '#{pane_current_command}')"
} >>"$RUNMAN"
(( _gone == 1 )) || echo "OUTCOME=INCONCLUSIVE reason=fake-agent-child-still-present-after-20s" >>"$RUNMAN"
cap_all post-manipulation

for i in 1 2 3 4 5 6; do cross_cycle "obs-c$i" 60; done
cap_all final
echo "end_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"$RUNMAN"
cp "$META/meta" "$CAP/meta.final.txt" 2>/dev/null || true
twd_teardown
echo "ARM a1 DONE"
