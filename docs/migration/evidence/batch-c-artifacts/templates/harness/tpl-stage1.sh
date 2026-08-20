#!/opt/homebrew/bin/bash
# Stage 1 template groups: G1 healthy, G5 control pair, G9 goals, G11 escapes,
# G7 unknown-action. Every byte is produced by a real frozen helper.
source "$(dirname "$0")/tlib.sh"
SHIM=/tmp/aecx/shim
REALDATE=/bin/date

banner() { echo; echo "############ $* ############"; }

########## G1 — healthy 2-agent session, no attention reasons ##########
banner G1
t_sandbox g1 "fake:worker"
t_launch tg1 || { echo "G1 LAUNCH FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
echo "producers (all real generated helpers of session tg1, run with the pane environment of the named agent):" >>"$P"
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }
run "$LEAD" state working "building the healthy fixture"
run "$LEAD" goal "healthy fixture session goal"
run "$LEAD" memo add --topic arch "healthy fixture memo body"
run "$LEAD" send fake:worker "healthy fixture send body"
run "$WORK" state working "worker acknowledging"
run "$LEAD" ask fake:worker "healthy fixture question"
RID="$(as_agent "$LEAD" requests all | awk '/pending/{print $3;exit}')"
echo "  harvested_request_id=$RID" >>"$P"
run "$WORK" reply "$RID" "healthy fixture answer"
run "$WORK" state done "worker finished"
run "$LEAD" state working "lead continues"
echo "G1 events:"; cat "$META/events.jsonl"
FP="$(t_store G1 healthy "$P")"; echo "G1/healthy fingerprint(pre)=$FP"
t_protect G1 healthy >/dev/null
t_teardown

########## G5 — harvested valid ask/reply mirror pair (control member) ##########
banner G5-control
t_sandbox g5 "fake:worker,fake:third"
t_launch tg5 || { echo "G5 LAUNCH FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"; THIRD="$(pane_of fake:third)"
P="$CAP/prov.txt"; : >"$P"
echo "producers: real ask (lead->worker) + real reply (worker->lead) = the mirror pair;" >>"$P"
echo "plus a real ask from the THIRD agent, harvested only so its genuine routing-key" >>"$P"
echo "bytes exist to be used verbatim by the G5 mutations." >>"$P"
run "$LEAD" ask fake:worker "G5 mirror question"
RID="$(as_agent "$LEAD" requests all | awk '/pending/{print $3;exit}')"
echo "  mirror_request_id=$RID" >>"$P"
run "$WORK" reply "$RID" "G5 mirror answer"
run "$THIRD" ask fake:lead "G5 third-agent question (routing-key donor)"
RID3="$(as_agent "$THIRD" requests all | awk '/pending/{print $3;exit}')"
echo "  donor_request_id=$RID3" >>"$P"
run "$LEAD" review fake:worker "G5 review request (second well-formed ref donor)"
echo "G5 events:"; cat "$META/events.jsonl"
FP="$(t_store G5 m1-control "$P")"; echo "G5/m1-control fingerprint(pre)=$FP"
t_protect G5 m1-control >/dev/null
t_teardown

########## G9 — goal events with DISTINCT deterministic timestamps ##########
banner G9
t_sandbox g9 ""
export PATH="$SHIM:$PATH"
export AE_REAL_DATE="$REALDATE"
export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
t_launch tg9 || { echo "G9 LAUNCH FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"
P="$CAP/prov.txt"; : >"$P"
echo "clock hook: PATH-first date shim at $SHIM/date; real date=$REALDATE sha256=$($REALDATE -u +%s >/dev/null; shasum -a 256 $REALDATE | cut -d' ' -f1)" >>"$P"
echo "substituted now-forms: '-u +%FT%TZ', '-u +%Y-%m-%dT%H:%M:%SZ', '-u +%Y%m%dT%H%M%SZ', '+%s'; every other invocation delegates" >>"$P"
i=0
for T in 1755000000 1755000600 1755001200 1755001800; do
    i=$((i+1))
    export AE_FAKE_NOW="$T"
    echo "  AE_FAKE_NOW=$T -> goal 'G9 goal revision $i'" >>"$P"
    as_agent "$LEAD" goal "G9 goal revision $i"; echo "    rc=$?" >>"$P"
done
unset AE_FAKE_NOW
cp "$AE_DATE_SHIM_LOG" "$CAP/date-shim.log.copy"
echo "G9 events:"; cat "$META/events.jsonl"
FP="$(t_store G9 goals-distinct-ts "$P")"; echo "G9 fingerprint(pre)=$FP"
t_protect G9 goals-distinct-ts >/dev/null
cp "$AE_DATE_SHIM_LOG" "$TSTORE/G9/_meta/date-shim-invocations.log"
t_teardown
unset AE_DATE_SHIM_LOG AE_REAL_DATE

########## G11 — escape classes, producer INPUT and emitted bytes ##########
banner G11
t_sandbox g11 "fake:worker"
t_launch tg11 || { echo "G11 LAUNCH FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
mkdir -p "$CAP/inputs"
emit() { # <class> <payload-file>
    local cls="$1" f="$2"
    local body; body="$(cat "$f")"
    echo "  class=$cls input_file=inputs/$cls.bin input_sha256=$(shasum -a 256 "$f" | cut -d' ' -f1) input_bytes=$(stat -f %z "$f")" >>"$P"
    as_agent "$LEAD" say "$body"; echo "    say rc=$?" >>"$P"
    as_agent "$LEAD" memo add --topic "$cls" "$body"; echo "    memo rc=$?" >>"$P"
    as_agent "$LEAD" send fake:worker "$body"; echo "    send rc=$?" >>"$P"
}
printf 'quote class: he said "hello" and '"'"'bye'"'"'' >"$CAP/inputs/quote.bin"
printf 'backslash class: a\\b and c\\\\d and \\n literal' >"$CAP/inputs/backslash.bin"
printf 'newline class: line one\nline two\nline three' >"$CAP/inputs/newline.bin"
printf 'tab class: col1\tcol2\tcol3' >"$CAP/inputs/tab.bin"
printf 'cr class: before\rafter' >"$CAP/inputs/cr.bin"
for c in quote backslash newline tab cr; do emit "$c" "$CAP/inputs/$c.bin"; done
mkdir -p "$AE_HOME/_g11-producer-inputs"
cp "$CAP/inputs"/*.bin "$AE_HOME/_g11-producer-inputs/"
echo "G11 events:"; cat "$META/events.jsonl"
FP="$(t_store G11 escapes "$P")"; echo "G11 fingerprint(pre)=$FP"
t_protect G11 escapes >/dev/null
t_teardown

########## G7c — events with an UNKNOWN ACTION, produced live ##########
banner G7-unknown-action
t_sandbox g7a "fake:worker"
t_launch tg7a || { echo "G7a LAUNCH FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
echo "unknown action produced by the REAL send helper via its documented _AE_EVENT_ACTION override" >>"$P"
run "$LEAD" state working "baseline before the unknown action"
for act in zzz-unknown-action another.unknown/action UPPER-Unknown; do
  echo "  _AE_EVENT_ACTION=$act send" >>"$P"
  env TMUX="${SOCK},${SRV_PID},0" TMUX_PANE="$LEAD" _AE_EVENT_ACTION="$act" "$META/send" fake:worker "unknown action body for $act"
  echo "    rc=$?" >>"$P"
done
echo "G7a events:"; cat "$META/events.jsonl"
FP="$(t_store G7 events-unknown-action "$P")"; echo "G7/events-unknown-action fingerprint(pre)=$FP"
t_protect G7 events-unknown-action >/dev/null
t_teardown
echo; echo "STAGE 1 DONE"
