#!/opt/homebrew/bin/bash
# A7 derived members: the 405j identity cases (all sharing ONE display name) and a
# duplicate meta KEY. Each is a byte copy of a producer-derived member changed only by the
# named mutation recorded beside it.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
derive() {
    local grp="$1" mem="$2" sg="$3" sm="$4" note="$5"
    mkdir -p "$TSTORE/$grp/_meta"
    DST="$TSTORE/$grp/$mem"
    [[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null; rm -rf "$DST"
    cp -Rp "$TSTORE/$sg/$sm" "$DST"; chmod -R u+w "$DST"
    DIFF="$TSTORE/$grp/_meta/$mem.mutation.txt"; : >"$DIFF"
    SESS="$(ls "$DST/sessions" | head -1)"
    { echo "group=$grp"; echo "member=$mem"; echo "derived_from=$sg/$sm (byte copy)"
      echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$sg/_meta/$sm.txt" | cut -d= -f2-)"
      echo "session=$SESS"; echo "frozen_sha=$FROZEN_SHA"
      echo "note=$note"
      echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$TSTORE/$grp/_meta/$mem.txt"
}
seal() {
    dir_manifest "$TSTORE/$1/$2" >"$TSTORE/$1/_meta/$2.modes.tsv"
    echo "fingerprint_pre_protection=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "named_mutations=see _meta/$2.mutation.txt" >>"$TSTORE/$1/_meta/$2.txt"
    chmod -R a-w "$TSTORE/$1/$2" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "  $1/$2 sealed"
}
EV() { echo "$DST/sessions/$SESS/events.jsonl"; }

derive A7 405j-stale-mismatched-keys A7 405j-full-fresh \
  "case 2: all four routing members PRESENT but naming a slot and a session that are not this session's"
python3 "$MUT" "$(EV)" "$DIFF" "actor_slot -> a slot no roster entry holds"   repl 1 '"actor_slot":"main"'        '"actor_slot":"worker.9"'
python3 "$MUT" "$(EV)" "$DIFF" "actor_session -> a session this is not"        repl 1 '"actor_session":"ta7j"'     '"actor_session":"some-other-session"'
python3 "$MUT" "$(EV)" "$DIFF" "target_slot -> a slot no roster entry holds"   repl 1 '"target_slot":"worker.0"'   '"target_slot":"worker.8"'
python3 "$MUT" "$(EV)" "$DIFF" "target_session -> a session this is not"       repl 1 '"target_session":"ta7j"'    '"target_session":"some-other-session"'
seal A7 405j-stale-mismatched-keys

derive A7 405j-slot-only A7 405j-full-fresh \
  "case 3a: PARTIAL keys — the slot members survive, both session members are deleted"
python3 "$MUT" "$(EV)" "$DIFF" "delete actor_session"  delkey 1 actor_session
python3 "$MUT" "$(EV)" "$DIFF" "delete target_session" delkey 1 target_session
seal A7 405j-slot-only

derive A7 405j-session-only A7 405j-full-fresh \
  "case 3b: PARTIAL keys — the session members survive, both slot members are deleted"
python3 "$MUT" "$(EV)" "$DIFF" "delete actor_slot"  delkey 1 actor_slot
python3 "$MUT" "$(EV)" "$DIFF" "delete target_slot" delkey 1 target_slot
seal A7 405j-session-only

derive A7 405j-keyless-legacy A7 405j-full-fresh \
  "case 4: KEYLESS legacy — no routing members at all, the shape a pre-routing event has"
for k in actor_slot actor_session target_slot target_session; do
  python3 "$MUT" "$(EV)" "$DIFF" "delete $k" delkey 1 "$k"
done
seal A7 405j-keyless-legacy

derive A7 405j-one-empty-member A7 405j-full-fresh \
  "case 5a: ONE routing member PRESENT AS THE EMPTY STRING — distinct from absent and from set"
python3 "$MUT" "$(EV)" "$DIFF" "actor_slot -> present, empty string" repl 1 '"actor_slot":"main"' '"actor_slot":""'
seal A7 405j-one-empty-member

derive A7 405j-all-empty-members A7 405j-full-fresh \
  "case 5b: ALL routing members present as the EMPTY STRING"
python3 "$MUT" "$(EV)" "$DIFF" "actor_slot -> empty"     repl 1 '"actor_slot":"main"'      '"actor_slot":""'
python3 "$MUT" "$(EV)" "$DIFF" "actor_session -> empty"  repl 1 '"actor_session":"ta7j"'   '"actor_session":""'
python3 "$MUT" "$(EV)" "$DIFF" "target_slot -> empty"    repl 1 '"target_slot":"worker.0"' '"target_slot":""'
python3 "$MUT" "$(EV)" "$DIFF" "target_session -> empty" repl 1 '"target_session":"ta7j"'  '"target_session":""'
seal A7 405j-all-empty-members

derive A7 meta-duplicate-key A7 meta-multi-equals \
  "a DUPLICATE meta key: the same key appears twice with different values, in the file the launch wrote"
M="$DST/sessions/$SESS/meta"
BEF="$(shasum -a 256 "$M" | cut -d' ' -f1)"; BEFB="$(stat -f %z "$M")"
printf 'goal=SECOND-GOAL-LINE-APPENDED-AFTER\n' >>"$M"
{ echo "## mutation: append a SECOND goal= line after the producer-written one"
  echo "file: sessions/$SESS/meta"
  echo "before: sha256=$BEF bytes=$BEFB"
  echo "after:  sha256=$(shasum -a 256 "$M" | cut -d' ' -f1) bytes=$(stat -f %z "$M")"
  echo "the two values differ, so a first-wins reader and a last-wins reader disagree"
  echo "first goal line: $(grep -m1 '^goal=' "$M")"
  echo "last  goal line: $(grep '^goal=' "$M" | tail -1)"; } >>"$DIFF"
seal A7 meta-duplicate-key
echo "TPL-A7 PART 2 DONE"
