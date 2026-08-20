#!/opt/homebrew/bin/bash
# ARM 3 — MANIPULATION: two-phase pane-content manipulation in ONE sandbox with
# ONE running watchdog. Phase A prints a documented generic phrase into the live
# fake agent's pane tail; Phase B displaces it with nonmatching lines.
source "$(dirname "$0")/arm.sh" a3
echo "arm_manipulation=two-phase pane-content (A: print phrase; B: displace phrase)" >>"$RUNMAN"
cap_all t0-post-launch
cross_cycle base-c1 60
cross_cycle base-c2 60
cap_all pre-phaseA

PHRASE='429 Too Many Requests'
{ echo "phaseA_phrase_literal=$PHRASE"
  echo "phaseA_phrase_sha256=$(printf '%s' "$PHRASE" | shasum -a 256 | cut -d' ' -f1)"
  echo "producer_pane_view_cmd=capture-pane -p -J -S -40 -E -"
  echo "phaseA_inject_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$RUNMAN"
printf '%s\n' "$PHRASE" > "$AEFAKE_CTL"
sleep 1
tm capture-pane -p -J -S -40 -E - -t "$AGENT_PANE" >"$CAP/producer-view.phaseA-injected.txt" 2>/dev/null
echo "phaseA_producer_view_phrase_occurrences=$(grep -c -F "$PHRASE" "$CAP/producer-view.phaseA-injected.txt" 2>/dev/null || echo 0)" >>"$RUNMAN"
for i in 1 2 3 4 5; do cross_cycle "phaseA-c$i" 60; done
cap_all post-phaseA

# Phase B — displace the phrase from the producer's captured tail.
FILL=200
{ echo "phaseB_fill_lines=$FILL"; echo "phaseB_inject_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$RUNMAN"
for ((i=1;i<=FILL;i++)); do printf 'filler-line-%03d\n' "$i" > "$AEFAKE_CTL"; done
sleep 2
tm capture-pane -p -J -S -40 -E - -t "$AGENT_PANE" >"$CAP/producer-view.phaseB-displaced.txt" 2>/dev/null
echo "phaseB_producer_view_phrase_occurrences=$(grep -c -F "$PHRASE" "$CAP/producer-view.phaseB-displaced.txt" 2>/dev/null || echo 0)" >>"$RUNMAN"
echo "phaseB_producer_view_sha256=$(shasum -a 256 "$CAP/producer-view.phaseB-displaced.txt" | cut -d' ' -f1)" >>"$RUNMAN"
for i in 1 2 3 4; do cross_cycle "phaseB-c$i" 60; done
cap_all final
{ echo "final_aefake_pids=$(pgrep -x aefake | tr '\n' ' ')"
  echo "end_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$RUNMAN"
cp "$AEFAKE_LOG" "$CAP/agent-stdin.log"
cp "$META/meta" "$CAP/meta.final.txt" 2>/dev/null || true
twd_teardown
echo "ARM a3 DONE"
