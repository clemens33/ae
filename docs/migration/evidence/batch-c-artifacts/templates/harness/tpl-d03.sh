#!/opt/homebrew/bin/bash
# D03 fixture: 31 UNIQUELY NUMBERED producer events, plus two harvested complete event
# lines kept beside the member as the controller's append payloads (one for the complete-
# append arm, two distinct sentinels for the rotation arm).
source "$(dirname "$0")/tlib.sh"
t_sandbox d03 "fake:worker"
t_launch td03 || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"
P="$CAP/prov.txt"; : >"$P"
{ echo "construction: 31 real memo invocations, each carrying a unique numbered marker, so"
  echo "the follow window can be read off the pane by number rather than by counting lines."
} >>"$P"
for i in $(seq -f '%02g' 1 31); do
    as_agent "$LEAD" memo add --topic d03 "D03-SEED-EVENT-$i" >/dev/null
done
echo "  seeded_events=$(wc -l <"$META/events.jsonl" | tr -d ' ')" >>"$P"
# harvest three MORE real events, then remove them: they become the controller payloads
for tag in APPEND-SENTINEL ROTATE-NEWPATH ROTATE-OLDINODE; do
    as_agent "$LEAD" memo add --topic d03 "D03-$tag" >/dev/null
done
tail -3 "$META/events.jsonl" > "$CAP/payloads.jsonl"
# BSD head has no negative line count (GNU-only). Drop the last three lines portably.
awk -v n="$(wc -l <"$META/events.jsonl" | tr -d ' ')" 'NR <= n-3' "$META/events.jsonl" >"$CAP/trimmed.jsonl" \
    && mv "$CAP/trimmed.jsonl" "$META/events.jsonl"
mkdir -p "$AE_HOME/_d03-payloads"
awk 'NR==1{print > "'"$AE_HOME"'/_d03-payloads/append-sentinel.jsonl"} NR==2{print > "'"$AE_HOME"'/_d03-payloads/rotate-newpath.jsonl"} NR==3{print > "'"$AE_HOME"'/_d03-payloads/rotate-oldinode.jsonl"}' "$CAP/payloads.jsonl"
{ echo "  controller payloads (each a REAL harvested complete event line, carried inside the member):"
  for f in append-sentinel rotate-newpath rotate-oldinode; do
      echo "    $f.jsonl sha256=$(shasum -a 256 "$AE_HOME/_d03-payloads/$f.jsonl" | cut -d' ' -f1) bytes=$(stat -f %z "$AE_HOME/_d03-payloads/$f.jsonl")"
  done
  echo "  events after removing the three payload lines: $(wc -l <"$META/events.jsonl" | tr -d ' ')"
} >>"$P"
mkdir -p "$TSTORE/D/_meta"
echo "D/d03 fingerprint(pre)=$(t_store D d03-31-numbered-events "$P")"
t_protect D d03-31-numbered-events >/dev/null
t_teardown
echo "TPL-D03 DONE"
