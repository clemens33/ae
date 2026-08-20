#!/opt/homebrew/bin/bash
# Stage 3: mutation-derived members (G3, G4, G5 m2-m6, G7 key cohorts, G8, G10).
# Every member starts as a byte copy of a PRODUCER-DERIVED template member and is
# changed only by the NAMED mutation recorded beside it.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
banner() { echo; echo "############ $* ############"; }

# derive <group> <member> <src-group> <src-member>  -> sets DST, DIFF, SESS
derive() {
    local grp="$1" mem="$2" sg="$3" sm="$4"
    mkdir -p "$TSTORE/$grp/_meta"
    DST="$TSTORE/$grp/$mem"
    [[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null
    rm -rf "$DST"; cp -R "$TSTORE/$sg/$sm" "$DST"; chmod -R u+w "$DST"
    DIFF="$TSTORE/$grp/_meta/$mem.mutation.txt"; : >"$DIFF"
    SESS="$(ls "$DST/sessions" | head -1)"
    {
      echo "group=$grp"; echo "member=$mem"
      echo "derived_from=$sg/$sm (byte copy)"
      echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$sg/_meta/$sm.txt" | cut -d= -f2-)"
      echo "session=$SESS"
      echo "frozen_sha=$FROZEN_SHA"; echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$TSTORE/$grp/_meta/$mem.txt"
}
seal() { # <group> <member>
    dir_manifest "$TSTORE/$1/$2" >"$TSTORE/$1/_meta/$2.modes.tsv"
    echo "fingerprint_pre_protection=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "named_mutations=see _meta/$2.mutation.txt" >>"$TSTORE/$1/_meta/$2.txt"
    chmod -R a-w "$TSTORE/$1/$2" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "$1/$2 sealed: $(grep '^fingerprint_protected=' "$TSTORE/$1/_meta/$2.txt" | cut -d= -f2-)"
}

########## G3 ##########
banner G3
derive G3 meta-mode-000 G1 healthy
BEFORE_MODE="$(stat -f %Lp "$DST/sessions/$SESS/meta")"
chmod 000 "$DST/sessions/$SESS/meta"
{ echo "## mutation: meta file mode -> 000"
  echo "file: sessions/$SESS/meta"
  echo "detail: mode $BEFORE_MODE -> 000; CONTENT BYTES UNCHANGED"
  echo "content_sha256=$(sudo=; shasum -a 256 "$TSTORE/G1/healthy/sessions/$SESS/meta" | cut -d' ' -f1)"
} >>"$DIFF"
seal G3 meta-mode-000

derive G3 malformed-complete-line G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "malformed-complete-line" \
    repl 3 '"action":"memo"' '"action:"memo"'
seal G3 malformed-complete-line

########## G4 ##########
banner G4
derive G4 no-events G1 healthy
B="$DST/sessions/$SESS/events.jsonl"
{ echo "## mutation: remove events.jsonl entirely"
  echo "file: sessions/$SESS/events.jsonl"
  echo "before: sha256=$(shasum -a 256 "$B" | cut -d' ' -f1) bytes=$(stat -f %z "$B")"
  echo "after:  FILE ABSENT"; } >>"$DIFF"
rm -f "$B"
seal G4 no-events

derive G4 zero-byte-events G1 healthy
B="$DST/sessions/$SESS/events.jsonl"
{ echo "## mutation: truncate events.jsonl to zero bytes"
  echo "file: sessions/$SESS/events.jsonl"
  echo "before: sha256=$(shasum -a 256 "$B" | cut -d' ' -f1) bytes=$(stat -f %z "$B")"; } >>"$DIFF"
: >"$B"
{ echo "after:  sha256=$(shasum -a 256 "$B" | cut -d' ' -f1) bytes=0"; } >>"$DIFF"
seal G4 zero-byte-events

########## G5 m2-m6 ##########
banner G5-mutations
# The donor byte values are READ OUT of the rebuilt control member, never hardcoded:
# every rebuild mints fresh request ids, and a hardcoded id would silently stop matching.
G5EV="$TSTORE/G5/m1-control/sessions/$(ls "$TSTORE/G5/m1-control/sessions" | head -1)/events.jsonl"
MIRROR_REF="$(sed -n '1p' "$G5EV" | sed -n 's/.*"ref":"\([^"]*\)".*/\1/p')"
REVIEW_REF="$(sed -n '4p' "$G5EV" | sed -n 's/.*"ref":"\([^"]*\)".*/\1/p')"
echo "donors: mirror_ref=$MIRROR_REF review_ref=$REVIEW_REF"
[[ -n "$MIRROR_REF" && -n "$REVIEW_REF" ]] || { echo "HARNESS-ABORT: could not read G5 donor refs"; exit 9; }
derive G5 m2-wrong-ref G5 m1-control
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply ref -> a different well-formed producer-derived request id (line 4's review ref)" \
    repl 2 "\"ref\":\"$MIRROR_REF\"" "\"ref\":\"$REVIEW_REF\""
seal G5 m2-wrong-ref

derive G5 m3-wrong-actor G5 m1-control
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply actor display -> fake:third" repl 2 '"actor":"fake:worker"' '"actor":"fake:third"'
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply actor_slot -> worker.1 (fake:third's real slot bytes, line 3)" repl 2 '"actor_slot":"worker.0"' '"actor_slot":"worker.1"'
seal G5 m3-wrong-actor

derive G5 m4-wrong-target G5 m1-control
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply target display -> fake:third" repl 2 '"target":"fake:lead"' '"target":"fake:third"'
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply target_slot -> worker.1 (fake:third's real slot bytes)" repl 2 '"target_slot":"main"' '"target_slot":"worker.1"'
seal G5 m4-wrong-target

derive G5 m5-routed-vs-routed-mismatch G5 m1-control
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "ask target_slot -> worker.1 while the reply's actor_slot stays worker.0: both sides routed, keys disagree" \
    repl 1 '"target_slot":"worker.0"' '"target_slot":"worker.1"'
seal G5 m5-routed-vs-routed-mismatch

derive G5 m6-mixed-routed-display G5 m1-control
for k in actor_slot actor_session target_slot target_session; do
    python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "reply: delete routing member $k (reply becomes display-only, ask stays routed)" delkey 2 "$k"
done
seal G5 m6-mixed-routed-display

########## G7 key cohorts ##########
banner G7-keys
derive G7 meta-unknown-keys G1 healthy
M="$DST/sessions/$SESS/meta"
BEF="$(shasum -a 256 "$M" | cut -d' ' -f1)"; BEFB="$(stat -f %z "$M")"
printf 'zzz_unknown_key=some value\nanother.unknown.key=42\n' >>"$M"
{ echo "## mutation: append two unknown keys to meta"
  echo "file: sessions/$SESS/meta"
  echo "before: sha256=$BEF bytes=$BEFB"
  echo "after:  sha256=$(shasum -a 256 "$M" | cut -d' ' -f1) bytes=$(stat -f %z "$M")"
  echo "inserted: 'zzz_unknown_key=some value\\nanother.unknown.key=42\\n' appended at EOF"; } >>"$DIFF"
seal G7 meta-unknown-keys

derive G7 events-unknown-keys G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "insert an unknown member into the state event (line 1)" \
    repl 1 ',"summary":"building the healthy fixture"}' ',"zzz_unknown":"unknown member value","summary":"building the healthy fixture"}'
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "insert an unknown member at end of the ask event (line 6)" \
    repl 6 '.txt"}' '.txt","another_unknown":123}'
seal G7 events-unknown-keys

########## G8 tail ##########
banner G8
derive G8 no-trailing-newline G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "drop the file's single trailing newline" droptail 1
seal G8 no-trailing-newline

derive G8 partial-trailing-record G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "truncate 40 bytes: the final record is left partial and unterminated" droptail 40
seal G8 partial-trailing-record

########## G10 display-only legacy pair ##########
banner G10-legacy
derive G10 display-only-legacy G1 healthy
for L in 6 7; do
  for k in actor_slot actor_session target_slot target_session; do
    python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "line $L: delete routing member $k (keyless legacy pair)" delkey "$L" "$k"
  done
done
seal G10 display-only-legacy
echo; echo "STAGE 3 DONE"
