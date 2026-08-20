#!/opt/homebrew/bin/bash
# A-H8 — SC-211n, the long-lived query (`events-tail`).
#
# GUARD ENUMERATION, done BEFORE building, per the preventive rule. Between an invocation
# and the fact each case is about there sit:
#   1. an unconditional banner (ae:14889-14894) — no guard, but it is the first bytes any
#      capture sees and a barrier must not mistake it for content;
#   2. `while [[ ! -f "$EVENTS_FILE" ]]; do sleep 1; done` (ae:14897-14899) — the file
#      must exist before anything else happens;
#   3. `tail -n 30 -f` (ae:14902) — the replay cut this arm is built to observe;
#   4. `helper_events_tail_format_event` (ae:14862-14884), which RETURNS EARLY on an empty
#      line and on a line not starting with `{`. A planted line that fails the formatter
#      produces no output, and that would look exactly like the replay cut dropping it.
#      Every planted event is therefore well-formed JSON with the fields the formatter
#      reads, except the case that is deliberately about a partial line.
#
# Termination is a CONTROLLER ACTION after a named barrier. This surface never exits on
# its own, so a bound expiring here is an instrument artifact and is recorded as one —
# never as a product rc.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H8
mkdir -p "$ADEST/$ARM"

# NOTE THE TRAILING NEWLINE. Without it every "event" concatenates onto the previous one
# and the file holds ONE unterminated line however many events were written — `tail -n 30`
# then has nothing to cut and the formatter renders the first field it finds. The replay
# cohorts measured nothing on their first run for exactly that reason: the fact under test
# was foreclosed by the INPUT CONSTRUCTION rather than by a product guard, which is the same
# shape as the upstream-guard pattern and is just as invisible in the output.
ev_line() { # <n> <marker>
    printf '{"ts":"2026-08-21T00:00:%02dZ","actor":"fake:lead","action":"memo","target":"","summary":"%s"}\n' \
        "$(( $1 % 60 ))" "$2"
}

# A follower does not exit. It is started, watched for a NAMED barrier, and then terminated
# by the controller — the termination is recorded as a controller action and the barrier is
# what closes the capture.
follow_run() { # <label> <barrier-regex> <max-seconds> -- <argv...>
    local label="$1" barrier="$2" maxs="$3"; shift 4
    local o="$CASE_DIR/out/$label.stdout" e="$CASE_DIR/out/$label.stderr"
    "$@" >"$o" 2>"$e" &
    local pid=$! waited=0 seen=no
    while (( waited < maxs * 2 )); do
        if grep -qE "$barrier" "$o" 2>/dev/null; then seen=yes; break; fi
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.5; waited=$((waited + 1))
    done
    led follow-barrier "label=$label" "barrier=$barrier" "seen=$seen" "waited_s=$((waited / 2))"
    if [[ "$seen" == no ]]; then
        { echo "## BARRIER NOT SEEN within ${maxs}s — an artifact of the bound, not a product rc"
          echo "label=$label"; echo "barrier=$barrier"; echo "outcome=INCONCLUSIVE for this invocation"
        } >"$CASE_DIR/out/$label.BARRIER-MISSED.txt"
        led OUTCOME-INCONCLUSIVE "reason=named barrier not observed" "label=$label"
    fi
    kill -TERM "$pid" 2>/dev/null; sleep 0.3; kill -KILL "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
    led controller-termination "label=$label" "note=this surface never exits on its own; termination is the controller's act, never a product rc"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$label" "controller-terminated" \
        "$(sha "$o")" "$(stat -f %z "$o" 2>/dev/null || echo 0)" \
        "$(sha "$e")" "$(stat -f %z "$e" 2>/dev/null || echo 0)" \
        "$maxs" "$seen" "$*" >>"$CASE_DIR/invocations.tsv"
}

open_case() { # <case-id> <note>
    case_open "$ARM" "$1"
    led rows "rows=SC-211n" "surface=events-tail" "claim_type=long-lived query, not a refusal row"
    led guards-enumerated "g1=banner unconditional ae:14889" "g2=file-existence wait ae:14897" \
        "g3=replay cut tail -n 30 ae:14902" "g4=formatter drops empty and non-{ lines ae:14864-14865"
    surface_state "$Q_META/events-tail" "$Q_META/_lib"
    led measured-input "note=$2"
}

h_sandbox h8 "cl:lead" "" || exit 1
h_launch th8 || { echo "launch failed"; exit 1; }
Q_META="$HMETA"; Q_WORK="$ROOT/work"
EV="$Q_META/events.jsonl"

########## c01 — the file does not exist, then the controller creates it ##########
open_case h8-c01-file-absent-then-created "invoked with no events file; the controller creates it as a named transition"
mv "$EV" "$EV.held" 2>/dev/null
canary pre h8c01
led PRODUCT-START "label=tail"
( "$Q_META/events-tail" >"$CASE_DIR/out/tail.stdout" 2>"$CASE_DIR/out/tail.stderr" & echo $! >"$CASE_DIR/pid" ) 
sleep 2
cp "$CASE_DIR/out/tail.stdout" "$CASE_DIR/out/before-file-exists.stdout"
led transition "action=controller creates the events file" "note=the wait at ae:14897 is what this crosses"
mv "$EV.held" "$EV"
ev_line 1 "AFTER-FILE-CREATED" >>"$EV"
W=0; while (( W < 20 )); do grep -q 'AFTER-FILE-CREATED' "$CASE_DIR/out/tail.stdout" && break; sleep 0.5; W=$((W+1)); done
led follow-barrier "label=tail" "barrier=AFTER-FILE-CREATED" "seen=$(grep -qc 'AFTER-FILE-CREATED' "$CASE_DIR/out/tail.stdout" 2>/dev/null && echo yes || echo no)" "waited_s=$((W/2))"
kill -TERM "$(cat "$CASE_DIR/pid")" 2>/dev/null; sleep 0.3; kill -KILL "$(cat "$CASE_DIR/pid")" 2>/dev/null
led controller-termination "label=tail"
printf 'tail\tcontroller-terminated\t%s\t%s\t%s\t%s\t20\t-\tevents-tail\n' \
    "$(sha "$CASE_DIR/out/tail.stdout")" "$(stat -f %z "$CASE_DIR/out/tail.stdout")" \
    "$(sha "$CASE_DIR/out/tail.stderr")" "$(stat -f %z "$CASE_DIR/out/tail.stderr")" >>"$CASE_DIR/invocations.tsv"
led PRODUCT-COMPLETE "label=tail"
canary post h8c01
led case-CLOSE
echo "  h8-c01"

########## c02-c04 — replay cohorts of 29, 30 and 31 events ##########
for n in 29 30 31; do
    open_case "h8-c0$((n - 27))-replay-$n" "a file holding exactly $n events before any follow begins"
    : >"$EV"
    for i in $(seq 1 "$n"); do ev_line "$i" "REPLAY-$(printf '%03d' "$i")-OF-$n" >>"$EV"; done
    cp "$EV" "$CASE_DIR/events.planted.jsonl"
    led planted "events=$n" "artifact_sha256=$(sha "$CASE_DIR/events.planted.jsonl")"
    canary pre "h8r$n"
    led PRODUCT-START "label=tail"
    follow_run "tail" "REPLAY-$(printf '%03d' "$n")-OF-$n" 20 -- "$Q_META/events-tail"
    led PRODUCT-COMPLETE "label=tail"
    canary post "h8r$n"
    { echo "planted_events=$n"
      echo "planted_lines=$(wc -l <"$CASE_DIR/events.planted.jsonl" | tr -d ' ')  # must equal planted_events"
      echo "rendered_lines=$(grep -c 'REPLAY-' "$CASE_DIR/out/tail.stdout" 2>/dev/null | tr -d ' ')"
      echo "first_rendered=$(grep -o 'REPLAY-[0-9]*-OF-[0-9]*' "$CASE_DIR/out/tail.stdout" 2>/dev/null | head -1)"
      echo "last_rendered=$(grep -o 'REPLAY-[0-9]*-OF-[0-9]*' "$CASE_DIR/out/tail.stdout" 2>/dev/null | tail -1)"
    } >"$CASE_DIR/replay-record.txt"
    led replay-record "artifact_sha256=$(sha "$CASE_DIR/replay-record.txt")" \
        "$(grep '^rendered_lines=' "$CASE_DIR/replay-record.txt")"
    led case-CLOSE
    echo "  replay-$n"
done

########## c05 — follow across a second named event ##########
open_case h8-c05-follow-second-event "a second event emitted while the follower runs"
: >"$EV"; ev_line 1 "FOLLOW-FIRST" >>"$EV"
canary pre h8c05
led PRODUCT-START "label=tail"
"$Q_META/events-tail" >"$CASE_DIR/out/tail.stdout" 2>"$CASE_DIR/out/tail.stderr" &
TP=$!
W=0; while (( W < 20 )); do grep -q 'FOLLOW-FIRST' "$CASE_DIR/out/tail.stdout" && break; sleep 0.5; W=$((W+1)); done
led follow-barrier "label=first" "barrier=FOLLOW-FIRST" "seen=$(grep -q 'FOLLOW-FIRST' "$CASE_DIR/out/tail.stdout" && echo yes || echo no)"
ev_line 2 "FOLLOW-SECOND" >>"$EV"
W=0; while (( W < 20 )); do grep -q 'FOLLOW-SECOND' "$CASE_DIR/out/tail.stdout" && break; sleep 0.5; W=$((W+1)); done
led follow-barrier "label=second" "barrier=FOLLOW-SECOND" "seen=$(grep -q 'FOLLOW-SECOND' "$CASE_DIR/out/tail.stdout" && echo yes || echo no)"
kill -TERM "$TP" 2>/dev/null; sleep 0.3; kill -KILL "$TP" 2>/dev/null
led controller-termination "label=tail"
printf 'tail\tcontroller-terminated\t%s\t%s\t%s\t%s\t20\t-\tevents-tail\n' \
    "$(sha "$CASE_DIR/out/tail.stdout")" "$(stat -f %z "$CASE_DIR/out/tail.stdout")" \
    "$(sha "$CASE_DIR/out/tail.stderr")" "$(stat -f %z "$CASE_DIR/out/tail.stderr")" >>"$CASE_DIR/invocations.tsv"
led PRODUCT-COMPLETE "label=tail"; canary post h8c05; led case-CLOSE
echo "  h8-c05"

########## c06 — a partial final line, completed in a second step ##########
open_case h8-c06-partial-final-line "a final line written in two steps with a barrier between them"
: >"$EV"; ev_line 1 "PARTIAL-BASE" >>"$EV"
canary pre h8c06
led PRODUCT-START "label=tail"
"$Q_META/events-tail" >"$CASE_DIR/out/tail.stdout" 2>"$CASE_DIR/out/tail.stderr" &
TP=$!
W=0; while (( W < 20 )); do grep -q 'PARTIAL-BASE' "$CASE_DIR/out/tail.stdout" && break; sleep 0.5; W=$((W+1)); done
printf '{"ts":"2026-08-21T00:00:05Z","actor":"fake:lead","action":"memo","target":"","summary":"PARTIAL-HALF' >>"$EV"
sleep 2
cp "$CASE_DIR/out/tail.stdout" "$CASE_DIR/out/after-partial.stdout"
led partial-step-1 "note=the line is written WITHOUT its newline; the capture at this barrier is kept separately" \
    "artifact_sha256=$(sha "$CASE_DIR/out/after-partial.stdout")"
printf 'WRITTEN"}\n' >>"$EV"
W=0; while (( W < 20 )); do grep -q 'PARTIAL-HALFWRITTEN' "$CASE_DIR/out/tail.stdout" && break; sleep 0.5; W=$((W+1)); done
led partial-step-2 "barrier=PARTIAL-HALFWRITTEN" "seen=$(grep -q 'PARTIAL-HALFWRITTEN' "$CASE_DIR/out/tail.stdout" && echo yes || echo no)"
kill -TERM "$TP" 2>/dev/null; sleep 0.3; kill -KILL "$TP" 2>/dev/null
led controller-termination "label=tail"
printf 'tail\tcontroller-terminated\t%s\t%s\t%s\t%s\t20\t-\tevents-tail\n' \
    "$(sha "$CASE_DIR/out/tail.stdout")" "$(stat -f %z "$CASE_DIR/out/tail.stdout")" \
    "$(sha "$CASE_DIR/out/tail.stderr")" "$(stat -f %z "$CASE_DIR/out/tail.stderr")" >>"$CASE_DIR/invocations.tsv"
led PRODUCT-COMPLETE "label=tail"; canary post h8c06; led case-CLOSE
echo "  h8-c06"

########## c07 — unknown argv, which this surface never reads ##########
open_case h8-c07-unknown-argv "an argument the surface never reads"
: >"$EV"; ev_line 1 "ARGV-IGNORED" >>"$EV"
canary pre h8c07
led PRODUCT-START "label=tail"
follow_run "tail" "ARGV-IGNORED" 20 -- "$Q_META/events-tail" --nosuchflag extra
led PRODUCT-COMPLETE "label=tail"; canary post h8c07; led case-CLOSE
echo "  h8-c07"
echo "A-H8 DONE"
