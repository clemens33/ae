#!/opt/homebrew/bin/bash
# Stage 4: G10 same-display/different-routing-key pair, produced live.
source "$(dirname "$0")/tlib.sh"
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }
t_sandbox g10 "fake:worker"
t_launch tg10 || { echo "FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
echo "construction: a SECOND agent carrying the SAME display name is added by the real spawn helper;" >>"$P"
echo "both then run the real ask helper, so one display name appears with two genuine routing keys." >>"$P"
run "$LEAD" spawn fake:lead "second holder of the display name"
echo "  meta after spawn:" >>"$P"; grep '^agent' "$META/meta" | sed 's/^/    /' >>"$P"
tm list-panes -a -F '#{pane_id}|#{@ae_agent}|#{@ae_slot}' | sed 's/^/    pane /' >>"$P"
SPAWNED="$(tm list-panes -a -F '#{pane_id} #{@ae_slot}' | awk '$2 ~ /^spawned\./{print $1;exit}')"
echo "  spawned_pane=$SPAWNED" >>"$P"
run "$LEAD" ask fake:worker "G10 question from the main-slot holder of the display name"
if [[ -n "$SPAWNED" ]]; then
    run "$SPAWNED" ask fake:worker "G10 question from the spawned-slot holder of the same display name"
else
    echo "  OUTCOME=INCONCLUSIVE reason=no spawned pane present" >>"$P"
fi
cat "$META/events.jsonl"
mkdir -p "$TSTORE/G10/_meta"
echo "G10/same-display-diff-routing fingerprint(pre)=$(t_store G10 same-display-diff-routing "$P")"
t_protect G10 same-display-diff-routing >/dev/null
t_teardown
echo "STAGE 4 DONE"
