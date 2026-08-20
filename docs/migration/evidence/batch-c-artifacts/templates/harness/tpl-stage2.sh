#!/opt/homebrew/bin/bash
# Stage 2: G2 (six attention members), G2b (competing), G6 (stopped).
source "$(dirname "$0")/tlib.sh"
SHIM=/tmp/aecx/shim
REALDATE=/bin/date
banner() { echo; echo "############ $* ############"; }
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }

########## G2 members 1-3: the T-WD precursor AE_HOMEs, as produced ##########
banner "G2 dead/stale/throttled (from the T-WD producer archive)"
mkdir -p "$TSTORE/G2/_meta"
for pair in "a1:dead" "a2:stale" "a3:throttled"; do
    arm="${pair%%:*}"; mem="${pair##*:}"
    src="/tmp/aecx/twd/$arm/home/.ae"
    dst="$TSTORE/G2/$mem"
    rm -rf "$dst"; cp -R "$src" "$dst"
    sess="twd$arm"
    if [[ "$mem" == "throttled" ]]; then
        # NAMED manipulation: restore this session's events.jsonl to the producer
        # state captured at barrier phaseA-c5 (before the phase-B displacement).
        before="$dst/sessions/$sess/events.jsonl"
        cp "$before" "$TSTORE/G2/_meta/throttled.events.before.jsonl"
        cp "/tmp/aecx/twd/a3/cap/events.post-phaseA.jsonl" "$before"
        {
          echo "## manipulation: G2/throttled events.jsonl <- producer state at barrier post-phaseA"
          echo "before: sha256=$(shasum -a 256 "$TSTORE/G2/_meta/throttled.events.before.jsonl" | cut -d' ' -f1) bytes=$(stat -f %z "$TSTORE/G2/_meta/throttled.events.before.jsonl")"
          echo "after:  sha256=$(shasum -a 256 "$before" | cut -d' ' -f1) bytes=$(stat -f %z "$before")"
          echo "byte-diff (removed suffix):"
          diff <(cat "$TSTORE/G2/_meta/throttled.events.before.jsonl") <(cat "$before") | sed 's/^/  /'
        } >"$TSTORE/G2/_meta/throttled.mutation.txt"
    fi
    dir_manifest "$dst" >"$TSTORE/G2/_meta/$mem.modes.tsv"
    fp="$(dir_fingerprint "$dst")"
    {
      echo "group=G2"; echo "member=$mem"; echo "fingerprint_pre_protection=$fp"
      echo "source=T-WD producer precursor arm $arm (AE_HOME snapshot), session $sess"
      echo "producer=the REAL generated watchdog of that launch"
      echo "frozen_sha=$FROZEN_SHA"; echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
      [[ "$mem" == "throttled" ]] && echo "named_manipulation=see _meta/throttled.mutation.txt"
    } >"$TSTORE/G2/_meta/$mem.txt"
    chmod -R a-w "$dst" 2>/dev/null || true
    echo "fingerprint_protected=$(dir_fingerprint "$dst")" >>"$TSTORE/G2/_meta/$mem.txt"
    echo "G2/$mem fingerprint(pre)=$fp"
done

########## G2 member 4: waiting-user ##########
banner "G2 waiting-user"
t_sandbox g2wu "fake:worker"
t_launch tg2wu || { echo "FAILED"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
run "$LEAD" state working "before the declaration"
run "$LEAD" send fake:worker "ordinary traffic"
run "$WORK" state waiting-user "asked the human a question"
cat "$META/events.jsonl"
echo "G2/waiting-user fingerprint(pre)=$(t_store G2 waiting-user "$P")"; t_protect G2 waiting-user >/dev/null
t_teardown

########## G2 member 5: blocked ##########
banner "G2 blocked"
t_sandbox g2bl "fake:worker"
t_launch tg2bl || { echo "FAILED"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
run "$LEAD" state working "before the declaration"
run "$WORK" state blocked "waiting on an external dependency"
cat "$META/events.jsonl"
echo "G2/blocked fingerprint(pre)=$(t_store G2 blocked "$P")"; t_protect G2 blocked >/dev/null
t_teardown

########## G2 member 6: unanswered (ask-pair aging via the clock hook) ##########
banner "G2 unanswered"
t_sandbox g2un "fake:worker"
export PATH="$SHIM:$PATH"; export AE_REAL_DATE="$REALDATE"
export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
t_launch tg2un || { echo "FAILED"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
echo "clock hook: PATH-first date shim; real date=$REALDATE sha256=$(shasum -a 256 $REALDATE | cut -d' ' -f1)" >>"$P"
export AE_FAKE_NOW=1755000000
echo "  AE_FAKE_NOW=$AE_FAKE_NOW for the ask (aged; no reply is ever produced)" >>"$P"
run "$LEAD" ask fake:worker "G2 unanswered question (never replied)"
unset AE_FAKE_NOW
cat "$META/events.jsonl"
cp "$AE_DATE_SHIM_LOG" "$TSTORE/G2/_meta/unanswered.date-shim-invocations.log" 2>/dev/null || true
echo "G2/unanswered fingerprint(pre)=$(t_store G2 unanswered "$P")"; t_protect G2 unanswered >/dev/null
t_teardown
unset AE_DATE_SHIM_LOG AE_REAL_DATE

########## G2b: competing reasons, arrival order descending in _attn_rank ##########
banner "G2b competing"
export T_WATCHDOG=true
t_sandbox g2b "fake:bravo,fake:charlie"
export PATH="$SHIM:$PATH"; export AE_REAL_DATE="$REALDATE"
export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
export AE_WATCHDOG_INTERVAL_SEC=5
export AE_FAKE_NOW=1755000000
t_launch tg2b || { echo "FAILED"; cat "$CAP/ae-launch.err"; exit 1; }
LEAD="$(pane_of fake:lead)"; BRAVO="$(pane_of fake:bravo)"; CHAR="$(pane_of fake:charlie)"
P="$CAP/prov.txt"; : >"$P"
{
  echo "construction: three agents, one reason each, produced in ARRIVAL order T0<T1<T2."
  echo "arrival order is DESCENDING in the frozen _attn_rank ladder (ae@72c7293:3571-3581,"
  echo "comment at :3586): dead=6 arrives first, waiting-user=4 second, unanswered=1 last."
  echo "clock hook active throughout; real date=$REALDATE sha256=$(shasum -a 256 $REALDATE | cut -d' ' -f1)"
  echo "panes: lead=$LEAD bravo=$BRAVO charlie=$CHAR"
} >>"$P"
BPID="$(tm display-message -p -t "$BRAVO" '#{pane_pid}')"
echo "  T0=$AE_FAKE_NOW manipulation: kill the fake-agent child under bravo's pane ($BPID); the REAL watchdog is the producer" >>"$P"
for p in $(pgrep -x aefake); do
    anc="$p"
    while [[ -n "$anc" && "$anc" != "1" && "$anc" != "0" ]]; do
        [[ "$anc" == "$BPID" ]] && { kill -TERM "$p" 2>/dev/null; echo "    killed aefake pid $p under $BPID" >>"$P"; break; }
        anc="$(ps -o ppid= -p "$anc" 2>/dev/null | tr -d '[:space:]')"
    done
done
_t0=$(/bin/date -u +%s); DEAD_OK=0
while (( $(/bin/date -u +%s) - _t0 < 90 )); do
    grep -q '"action":"alert"' "$META/events.jsonl" 2>/dev/null && { DEAD_OK=1; break; }
    sleep 2
done
echo "  dead_alert_present=$DEAD_OK" >>"$P"
(( DEAD_OK == 1 )) || echo "  OUTCOME=INCONCLUSIVE reason=no alert byte within 90s" >>"$P"
"$META/watchdog" stop >/dev/null 2>&1; echo "  watchdog stopped (no further producer cycles)" >>"$P"
export AE_FAKE_NOW=1755000600
echo "  T1=$AE_FAKE_NOW: lead declares waiting-user" >>"$P"
run "$LEAD" state waiting-user "lead asked the human"
export AE_FAKE_NOW=1755001200
echo "  T2=$AE_FAKE_NOW: bravo->charlie ask, never replied (aged)" >>"$P"
run "$BRAVO" ask fake:charlie "G2b unanswered question (never replied)"
unset AE_FAKE_NOW
cat "$META/events.jsonl"
cp "$AE_DATE_SHIM_LOG" "$CAP/date-shim.log.copy"
mkdir -p "$TSTORE/G2b/_meta"
echo "G2b fingerprint(pre)=$(t_store G2b competing "$P")"; t_protect G2b competing >/dev/null
cp "$AE_DATE_SHIM_LOG" "$TSTORE/G2b/_meta/competing.date-shim-invocations.log" 2>/dev/null || true
t_teardown
unset AE_DATE_SHIM_LOG AE_REAL_DATE T_WATCHDOG AE_WATCHDOG_INTERVAL_SEC

########## G6: stopped session dirs ##########
banner "G6 stopped-plain"
t_sandbox g6a "fake:worker"
t_launch tg6a || { echo "FAILED"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
run "$LEAD" state working "work before the stop"
run "$LEAD" send fake:worker "ordinary traffic before the stop"
echo "  ae stop tg6a" >>"$P"
"$HARNESS_BASH" "$FROZEN_AE" stop tg6a </dev/null >"$CAP/stop.out" 2>"$CAP/stop.err"; echo "    rc=$?" >>"$P"
cat "$CAP/stop.out" "$CAP/stop.err"
echo "G6/stopped-plain fingerprint(pre)=$(t_store G6 stopped-plain "$P")"; t_protect G6 stopped-plain >/dev/null
t_teardown

banner "G6 stopped-attention"
t_sandbox g6b "fake:worker"
t_launch tg6b || { echo "FAILED"; exit 1; }
LEAD="$(pane_of fake:lead)"; WORK="$(pane_of fake:worker)"
P="$CAP/prov.txt"; : >"$P"
run "$LEAD" state working "work before the stop"
run "$WORK" state blocked "blocked before the stop"
echo "  ae stop tg6b" >>"$P"
"$HARNESS_BASH" "$FROZEN_AE" stop tg6b </dev/null >"$CAP/stop.out" 2>"$CAP/stop.err"; echo "    rc=$?" >>"$P"
cat "$CAP/stop.out" "$CAP/stop.err"
echo "G6/stopped-attention fingerprint(pre)=$(t_store G6 stopped-attention "$P")"; t_protect G6 stopped-attention >/dev/null
t_teardown
echo; echo "STAGE 2 DONE"
