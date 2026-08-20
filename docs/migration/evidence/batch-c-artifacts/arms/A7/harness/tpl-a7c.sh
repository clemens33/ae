#!/opt/homebrew/bin/bash
# A7 405j, rebuilt on an ask->REPLY PAIR.
#
# The first attempt built the identity cases on a LONE ASK. All seven rendered identically,
# which was a property of the fixture, not of the product: with no reply to pair against,
# the routing members are inert and nothing can depend on them. The A6 SC-518 captures show
# the consumer responds sharply to the pairing inputs (control replied, four mutations
# pending), so the cases are rebuilt on the same pair base and the mutations land on the
# REPLY's routing members, where they can matter.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
derive() {
    local mem="$1" note="$2"
    mkdir -p "$TSTORE/A7/_meta"
    DST="$TSTORE/A7/$mem"
    [[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null; rm -rf "$DST"
    cp -Rp "$TSTORE/G5/m1-control" "$DST"; chmod -R u+w "$DST"
    DIFF="$TSTORE/A7/_meta/$mem.mutation.txt"; : >"$DIFF"
    SESS="$(ls "$DST/sessions" | head -1)"
    { echo "group=A7"; echo "member=$mem"; echo "derived_from=G5/m1-control (byte copy)"
      echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/G5/_meta/m1-control.txt" | cut -d= -f2-)"
      echo "session=$SESS"; echo "frozen_sha=$FROZEN_SHA"
      echo "base=a real ask (line 1) and its real identity-valid reply (line 2)"
      echo "mutations land on the REPLY's routing members, line 2"
      echo "note=$note"
      echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$TSTORE/A7/_meta/$mem.txt"
}
seal() {
    dir_manifest "$TSTORE/A7/$1" >"$TSTORE/A7/_meta/$1.modes.tsv"
    echo "fingerprint_pre_protection=$(dir_fingerprint "$TSTORE/A7/$1")" >>"$TSTORE/A7/_meta/$1.txt"
    echo "named_mutations=see _meta/$1.mutation.txt" >>"$TSTORE/A7/_meta/$1.txt"
    chmod -R a-w "$TSTORE/A7/$1" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$TSTORE/A7/$1")" >>"$TSTORE/A7/_meta/$1.txt"
    echo "  A7/$1 sealed"
}
EV() { echo "$DST/sessions/$SESS/events.jsonl"; }

derive pair-405j-full-fresh "case 1: the unmutated pair — all four routing members present and fresh (CONTROL)"
seal pair-405j-full-fresh

derive pair-405j-stale-keys "case 2: all four members PRESENT but naming a slot and session this is not"
python3 "$MUT" "$(EV)" "$DIFF" "reply actor_slot -> a slot no roster entry holds"  repl 2 '"actor_slot":"worker.0"' '"actor_slot":"worker.9"'
python3 "$MUT" "$(EV)" "$DIFF" "reply actor_session -> a session this is not"      repl 2 '"actor_session":"tg5"'  '"actor_session":"some-other-session"'
python3 "$MUT" "$(EV)" "$DIFF" "reply target_slot -> a slot no roster entry holds" repl 2 '"target_slot":"main"'    '"target_slot":"worker.8"'
python3 "$MUT" "$(EV)" "$DIFF" "reply target_session -> a session this is not"     repl 2 '"target_session":"tg5"'  '"target_session":"some-other-session"'
seal pair-405j-stale-keys

derive pair-405j-slot-only "case 3a: PARTIAL — slot members survive, both session members deleted"
python3 "$MUT" "$(EV)" "$DIFF" "delete reply actor_session"  delkey 2 actor_session
python3 "$MUT" "$(EV)" "$DIFF" "delete reply target_session" delkey 2 target_session
seal pair-405j-slot-only

derive pair-405j-session-only "case 3b: PARTIAL — session members survive, both slot members deleted"
python3 "$MUT" "$(EV)" "$DIFF" "delete reply actor_slot"  delkey 2 actor_slot
python3 "$MUT" "$(EV)" "$DIFF" "delete reply target_slot" delkey 2 target_slot
seal pair-405j-session-only

derive pair-405j-keyless "case 4: KEYLESS legacy reply — no routing members at all"
for k in actor_slot actor_session target_slot target_session; do
  python3 "$MUT" "$(EV)" "$DIFF" "delete reply $k" delkey 2 "$k"
done
seal pair-405j-keyless

derive pair-405j-one-empty "case 5a: ONE member present as the EMPTY STRING"
python3 "$MUT" "$(EV)" "$DIFF" "reply actor_slot -> present, empty string" repl 2 '"actor_slot":"worker.0"' '"actor_slot":""'
seal pair-405j-one-empty

derive pair-405j-all-empty "case 5b: ALL four members present as the EMPTY STRING"
python3 "$MUT" "$(EV)" "$DIFF" "reply actor_slot -> empty"     repl 2 '"actor_slot":"worker.0"'  '"actor_slot":""'
python3 "$MUT" "$(EV)" "$DIFF" "reply actor_session -> empty"  repl 2 '"actor_session":"tg5"'    '"actor_session":""'
python3 "$MUT" "$(EV)" "$DIFF" "reply target_slot -> empty"    repl 2 '"target_slot":"main"'     '"target_slot":""'
python3 "$MUT" "$(EV)" "$DIFF" "reply target_session -> empty" repl 2 '"target_session":"tg5"'   '"target_session":""'
seal pair-405j-all-empty
echo "TPL-A7C DONE"
