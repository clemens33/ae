#!/opt/homebrew/bin/bash
# A1-specific template members: SC-510b (empty vs omitted), SC-510c (recover-ref),
# SC-510e/f (duplicate key cohorts, both member orders), SC-511a omitted-routing.
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
banner() { echo; echo "############ $* ############"; }
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }

########## SC-510b — empty-at-input vs absent-at-input ##########
banner "A1 510b empty-vs-omitted"
t_sandbox a1510b "fake:worker"
t_launch ta1b || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; P="$CAP/prov.txt"; : >"$P"
E() { env TMUX="${SOCK},${SRV_PID},0" TMUX_PANE="$LEAD" "$@"; }
echo "cohort: the SAME real send helper, with its documented event overrides set to a" >>"$P"
echo "genuinely EMPTY STRING versus left UNSET. Producer input is recorded per case." >>"$P"
echo "  case b1: _AE_EVENT_SUMMARY unset, _AE_EVENT_REF unset (both absent at input)" >>"$P"
E "$META/send" fake:worker "b1 body: both overrides absent"; echo "    rc=$?" >>"$P"
echo "  case b2: _AE_EVENT_SUMMARY='' (present, empty), _AE_EVENT_REF unset" >>"$P"
E _AE_EVENT_SUMMARY='' "$META/send" fake:worker "b2 body: empty summary override"; echo "    rc=$?" >>"$P"
echo "  case b3: _AE_EVENT_REF='' (present, empty), _AE_EVENT_SUMMARY set" >>"$P"
E _AE_EVENT_REF='' _AE_EVENT_SUMMARY='b3 summary present' "$META/send" fake:worker "b3 body"; echo "    rc=$?" >>"$P"
echo "  case b4: _AE_EVENT_REF='b4-ref-present' _AE_EVENT_SUMMARY='' " >>"$P"
E _AE_EVENT_REF='b4-ref-present' _AE_EVENT_SUMMARY='' "$META/send" fake:worker "b4 body"; echo "    rc=$?" >>"$P"
echo "  case b5: state helper with an EMPTY reason argument (empty at input)" >>"$P"
E "$META/state" working ""; echo "    rc=$?" >>"$P"
echo "  case b6: state helper with NO reason argument (absent at input)" >>"$P"
E "$META/state" working; echo "    rc=$?" >>"$P"
echo "  case b7: memo with an EMPTY topic (empty at input) vs b8 no topic (absent)" >>"$P"
E "$META/memo" add --topic "" "b7 memo body"; echo "    rc=$?" >>"$P"
E "$META/memo" add "b8 memo body"; echo "    rc=$?" >>"$P"
cat "$META/events.jsonl"
mkdir -p "$TSTORE/A1/_meta"
echo "A1/510b fingerprint(pre)=$(t_store A1 510b-empty-vs-omitted "$P")"; t_protect A1 510b-empty-vs-omitted >/dev/null
t_teardown

########## SC-511a — a producer cohort with genuinely OMITTED routing ##########
banner "A1 511a omitted-routing"
t_sandbox a1511a "fake:worker"
t_launch ta1r || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"; P="$CAP/prov.txt"; : >"$P"
echo "cohort: only helpers whose emitter never sets the routing members (state/goal/memo/say/send)." >>"$P"
run "$LEAD" state working "omitted-routing cohort"
run "$LEAD" goal "omitted-routing cohort goal"
run "$LEAD" memo add --topic omit "omitted-routing memo"
run "$LEAD" say "omitted-routing chat"
run "$LEAD" send fake:worker "omitted-routing send"
run "$WORK" state blocked "omitted-routing worker state"
cat "$META/events.jsonl"
echo "A1/511a-omitted fingerprint(pre)=$(t_store A1 511a-omitted-routing "$P")"; t_protect A1 511a-omitted-routing >/dev/null
t_teardown

########## SC-510c — recover-ref bytes from the real producer ##########
banner "A1 510c recover-ref"
export T_WATCHDOG=true
t_sandbox a1510c ""
cat >"$AE_HOME/config" <<CFG
[agents]
fake = "/tmp/aecx/bin/codex"

[workspace]
main = fake:lead
layout = vertical
watchdog = true
CFG
export AE_WATCHDOG_INTERVAL_SEC=5
export AE_WATCHDOG_TG_SUPERVISE_SEC=0
P="$ROOT/cap/prov.txt"; : >"$P"
DAY="$(/bin/date -u +%Y/%m/%d)"
mkdir -p "$HOME/.codex/sessions/$DAY"
PLANT="$HOME/.codex/sessions/$DAY/rollout-planted.jsonl"
printf '{"id":"11111111-2222-3333-4444-555555555555","timestamp":"2026-08-20T00:00:00Z","cwd":"%s"}\n' "$ROOT/work" >"$PLANT"
printf '{"type":"message","role":"user","content":"planted producer input"}\n' >>"$PLANT"
{
  echo "planted PRODUCER INPUT (an external codex-shaped session log, not an ae fixture byte):"
  echo "  path=$PLANT"
  echo "  sha256=$(shasum -a 256 "$PLANT" | cut -d' ' -f1) bytes=$(stat -f %z "$PLANT")"
  echo "  content:"; sed 's/^/    /' "$PLANT"
  echo "agent binary is the controllable fake copied to /tmp/aecx/bin/codex so the frozen"
  echo "tool classifier reports tool_kind=codex and the real recover path runs. No live model."
  echo "  codex_fake_sha256=$(shasum -a 256 /tmp/aecx/bin/codex | cut -d' ' -f1)"
} >>"$P"
t_launch ta1c || { echo FAILED; cat "$CAP/ae-launch.err"; exit 1; }
echo "  meta agent line: $(grep '^agent.main=' "$META/meta")" >>"$P"
_t0=$(/bin/date -u +%s); GOT=0
while (( $(/bin/date -u +%s) - _t0 < 120 )); do
    grep -q '"action":"recover"' "$META/events.jsonl" 2>/dev/null && { GOT=1; break; }
    sleep 3
done
echo "  recover_event_present=$GOT" >>"$P"
(( GOT == 1 )) || echo "  OUTCOME=INCONCLUSIVE reason=no recover byte within 120s" >>"$P"
"$META/watchdog" stop >/dev/null 2>&1
cp "$PLANT" "$AE_HOME/_a1-510c-planted-producer-input.jsonl"
cat "$META/events.jsonl"
echo "A1/510c fingerprint(pre)=$(t_store A1 510c-recover-ref "$P")"; t_protect A1 510c-recover-ref >/dev/null
t_teardown
unset T_WATCHDOG AE_WATCHDOG_INTERVAL_SEC AE_WATCHDOG_TG_SUPERVISE_SEC
echo "TPL-A1 PART 1 DONE"
