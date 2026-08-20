#!/opt/homebrew/bin/bash
# D-record fixture members.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }

########## D02 — a PENDING ask plus the identity-valid reply harvested from the
########## real reply helper, kept beside the member as the controller's payload.
t_sandbox d02 "fake:worker"
t_launch td02 || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
{ echo "construction: a real ask, then a real reply produced by the real reply helper from"
  echo "the responder's own pane (so it is identity-valid in every routing member). The reply"
  echo "LINE is then lifted out of events.jsonl and kept beside the member as the controller's"
  echo "append payload, leaving the member itself holding a genuinely PENDING request."
} >>"$P"
run "$LEAD" ask fake:worker "D02 question"
RID="$(as_agent "$LEAD" requests all | awk '/pending/{print $3;exit}')"
echo "  request_id=$RID" >>"$P"
run "$WORK" reply "$RID" "D02 identity-valid answer"
cp "$META/events.jsonl" "$CAP/events.with-reply.jsonl"
REPLY_LINE="$(grep '"action":"reply"' "$META/events.jsonl" | tail -1)"
printf '%s\n' "$REPLY_LINE" >"$CAP/controller-payload.reply.jsonl"
mkdir -p "$TSTORE/D/_meta"
DIFF="$TSTORE/D/_meta/d02-pending-with-harvested-reply.mutation.txt"; : >"$DIFF"
python3 "$MUT" "$META/events.jsonl" "$DIFF" \
  "remove the harvested reply line, leaving the request PENDING; the removed bytes become the controller's append payload" \
  dropline "$(grep -n '"action":"reply"' "$META/events.jsonl" | tail -1 | cut -d: -f1)"
cp "$CAP/controller-payload.reply.jsonl" "$AE_HOME/_d02-controller-payload.reply.jsonl"
{ echo "  controller_payload_sha256=$(shasum -a 256 "$CAP/controller-payload.reply.jsonl" | cut -d' ' -f1)"
  echo "  controller_payload_bytes=$(stat -f %z "$CAP/controller-payload.reply.jsonl")"
  echo "  payload is stored INSIDE the member as _d02-controller-payload.reply.jsonl so a clone"
  echo "  carries its own payload and no arm has to reach outside its sandbox for it"
} >>"$P"
echo "D/d02 events after mutation:"; cat "$META/events.jsonl"
echo "D/d02 fingerprint(pre)=$(t_store D d02-pending-with-harvested-reply "$P")"
t_protect D d02-pending-with-harvested-reply >/dev/null
t_teardown
echo "TPL-D DONE"
