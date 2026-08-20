#!/opt/homebrew/bin/bash
# SC-510e / SC-510f — duplicate-key cohorts, each in both member orders.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
derive() {
    local grp="$1" mem="$2" sg="$3" sm="$4"
    mkdir -p "$TSTORE/$grp/_meta"
    DST="$TSTORE/$grp/$mem"
    [[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null
    rm -rf "$DST"; cp -R "$TSTORE/$sg/$sm" "$DST"; chmod -R u+w "$DST"
    DIFF="$TSTORE/$grp/_meta/$mem.mutation.txt"; : >"$DIFF"
    SESS="$(ls "$DST/sessions" | head -1)"
    { echo "group=$grp"; echo "member=$mem"; echo "derived_from=$sg/$sm (byte copy)"
      echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$sg/_meta/$sm.txt" | cut -d= -f2-)"
      echo "session=$SESS"; echo "frozen_sha=$FROZEN_SHA"
      echo "both duplicate values are PRODUCER-DERIVED byte values taken verbatim from real"
      echo "summaries already present in this same events.jsonl (lines 1 and 10)."
      echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$TSTORE/$grp/_meta/$mem.txt"
}
seal() {
    dir_manifest "$TSTORE/$1/$2" >"$TSTORE/$1/_meta/$2.modes.tsv"
    echo "fingerprint_pre_protection=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "named_mutations=see _meta/$2.mutation.txt" >>"$TSTORE/$1/_meta/$2.txt"
    chmod -R a-w "$TSTORE/$1/$2" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "$1/$2 sealed"
}
A='building the healthy fixture'; B='lead continues'

derive A1 510e-dupkey-known G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "line 1: KNOWN key 'summary' appears twice, conflicting values, produced-order A then B" \
  repl 1 ",\"summary\":\"$A\"}" ",\"summary\":\"$A\",\"summary\":\"$B\"}"
seal A1 510e-dupkey-known

derive A1 510e-dupkey-known-reversed G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "line 1: KNOWN key 'summary' appears twice, conflicting values, MEMBER ORDER REVERSED (B then A)" \
  repl 1 ",\"summary\":\"$A\"}" ",\"summary\":\"$B\",\"summary\":\"$A\"}"
seal A1 510e-dupkey-known-reversed

derive A1 510f-dupkey-unknown G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "line 1: UNKNOWN key 'zzz_unknown' appears twice, conflicting values, order A then B" \
  repl 1 ",\"summary\":\"$A\"}" ",\"zzz_unknown\":\"$A\",\"zzz_unknown\":\"$B\",\"summary\":\"$A\"}"
seal A1 510f-dupkey-unknown

derive A1 510f-dupkey-unknown-reversed G1 healthy
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" "line 1: UNKNOWN key 'zzz_unknown' appears twice, conflicting values, MEMBER ORDER REVERSED (B then A)" \
  repl 1 ",\"summary\":\"$A\"}" ",\"zzz_unknown\":\"$B\",\"zzz_unknown\":\"$A\",\"summary\":\"$A\"}"
seal A1 510f-dupkey-unknown-reversed
echo DONE
