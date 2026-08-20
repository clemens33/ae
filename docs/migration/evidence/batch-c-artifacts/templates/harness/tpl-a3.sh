#!/opt/homebrew/bin/bash
# A3 members: the amended SC-017g cohort, and the SC-524 source-discrimination pair.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
SHIM=/tmp/aecx/shim; REALDATE=/bin/date
banner() { echo; echo "############ $* ############"; }
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }

########## SC-017g amended — a SESSION-LEVEL unanswered request aged past the
########## default threshold, competing against an AGENT-OWNED reason.
banner "A3 017g unanswered-vs-agent-owned"
t_sandbox a3g "fake:bravo,fake:charlie"
export PATH="$SHIM:$PATH"; export AE_REAL_DATE="$REALDATE"
export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
t_launch ta3g || { echo FAILED; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; BRAVO="$(pane_of fake:bravo)"; CHAR="$(pane_of fake:charlie)"
P="$CAP/prov.txt"; : >"$P"
{ echo "construction: an AGENT-OWNED reason and a SESSION-LEVEL unanswered request compete."
  echo "arrival order T0<T1 is descending in the frozen _attn_rank ladder"
  echo "(ae@72c7293:3571-3581): the agent-owned declaration arrives FIRST, the aged"
  echo "unanswered ask LAST, so a last-wins reader and a rank-wins reader are"
  echo "distinguishable. The ask targets a THIRD agent, so the unanswered reason is owned"
  echo "by a different agent than the declaration."
  echo "clock hook active; real date=$REALDATE sha256=$(shasum -a 256 $REALDATE | cut -d' ' -f1)"
} >>"$P"
export AE_FAKE_NOW=1755000000
echo "  T0=$AE_FAKE_NOW bravo declares blocked (agent-owned)" >>"$P"
run "$BRAVO" state blocked "A3 017g agent-owned reason"
export AE_FAKE_NOW=1755000600
echo "  T1=$AE_FAKE_NOW lead->charlie ask, never replied (aged past the 1800s default)" >>"$P"
run "$LEAD" ask fake:charlie "A3 017g unanswered question (never replied)"
unset AE_FAKE_NOW
cat "$META/events.jsonl"
mkdir -p "$TSTORE/A3/_meta"
echo "A3/017g fingerprint(pre)=$(t_store A3 017g-unanswered-vs-agent-owned "$P")"
t_protect A3 017g-unanswered-vs-agent-owned >/dev/null
cp "$AE_DATE_SHIM_LOG" "$TSTORE/A3/_meta/017g.date-shim-invocations.log" 2>/dev/null || true
t_teardown
unset AE_DATE_SHIM_LOG AE_REAL_DATE

########## SC-524 source-discrimination pair — identical cloned inputs, one source
########## made anomalous in each half.
banner "A3 524 pair"
derive() {
    local grp="$1" mem="$2" sg="$3" sm="$4"
    mkdir -p "$TSTORE/$grp/_meta"
    DST="$TSTORE/$grp/$mem"
    [[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null; rm -rf "$DST"
    cp -Rp "$TSTORE/$sg/$sm" "$DST"; chmod -R u+w "$DST"
    DIFF="$TSTORE/$grp/_meta/$mem.mutation.txt"; : >"$DIFF"
    SESS="$(ls "$DST/sessions" | head -1)"
    { echo "group=$grp"; echo "member=$mem"; echo "derived_from=$sg/$sm (byte copy)"
      echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$sg/_meta/$sm.txt" | cut -d= -f2-)"
      echo "session=$SESS"; echo "frozen_sha=$FROZEN_SHA"
      echo "pair_note=both halves start from the SAME base member; they differ only in WHICH"
      echo "  of the two candidate activity sources is made anomalous."
      echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$TSTORE/$grp/_meta/$mem.txt"
}
seal() {
    dir_manifest "$TSTORE/$1/$2" >"$TSTORE/$1/_meta/$2.modes.tsv"
    echo "fingerprint_pre_protection=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    chmod -R a-w "$TSTORE/$1/$2" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$TSTORE/$1/$2")" >>"$TSTORE/$1/_meta/$2.txt"
    echo "$1/$2 sealed"
}
# (a) FUTURE event ts, ORDINARY mtime
derive A3 524a-future-ts-ordinary-mtime G1 healthy
LASTTS="$(tail -1 "$DST/sessions/$SESS/events.jsonl" | sed -n 's/.*"ts":"\([^"]*\)".*/\1/p')"
python3 "$MUT" "$DST/sessions/$SESS/events.jsonl" "$DIFF" \
    "last event ts -> a FUTURE value; the file's mtime is left ordinary" \
    repl 10 "\"ts\":\"$LASTTS\"" '"ts":"2099-01-01T00:00:00Z"'
touch -t 202601011200.00 "$DST/sessions/$SESS/events.jsonl"
{ echo "## manipulation: events.jsonl mtime pinned ORDINARY"
  echo "mtime=202601011200.00 epoch=$(stat -f %m "$DST/sessions/$SESS/events.jsonl")"; } >>"$DIFF"
seal A3 524a-future-ts-ordinary-mtime
# (b) ORDINARY event ts, FUTURE mtime
derive A3 524b-ordinary-ts-future-mtime G1 healthy
touch -t 209901010000.00 "$DST/sessions/$SESS/events.jsonl"
{ echo "## manipulation: events.jsonl bytes UNCHANGED; only the file mtime is set to a FUTURE value"
  echo "events_sha256=$(shasum -a 256 "$DST/sessions/$SESS/events.jsonl" | cut -d' ' -f1)"
  echo "base_events_sha256=$(shasum -a 256 "$TSTORE/G1/healthy/sessions/$SESS/events.jsonl" | cut -d' ' -f1)"
  echo "mtime=209901010000.00 epoch=$(stat -f %m "$DST/sessions/$SESS/events.jsonl")"; } >>"$DIFF"
seal A3 524b-ordinary-ts-future-mtime
echo "TPL-A3 DONE"
