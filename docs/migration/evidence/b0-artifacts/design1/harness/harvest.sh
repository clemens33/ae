#!/opt/homebrew/bin/bash
set -euo pipefail
SB=/tmp/aeb0
export HOME="$SB/tmpl/home"
export AE_HOME="$SB/tmpl/home/.ae"
export TMUX_TMPDIR="$SB/sock"
export AE_TMUX_SERVER="aeb0tmpl"
export AE_TMUX_SERVER_KIND="name"
export TZ=UTC; export LANG=C
S=b0tmpl; SD="$AE_HOME/sessions/$S"
T() { command tmux -L "$AE_TMUX_SERVER" "$@"; }
MAIN_PANE="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy:dummy"{print $1; exit}')"
WORK_PANE="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy2:helper"{print $1; exit}')"

# quiesce: stop the watchdog so the template stops moving
"$SD/watchdog" stop >"$SB/logs/watchdog-stop.out" 2>&1 || true
sleep 2

# ── BASELINE SNAPSHOT ──
rm -rf "$SB/template"; mkdir -p "$SB/template"
cp -a "$AE_HOME" "$SB/template/.ae"
echo "baseline snapshot taken"

# ── harvest 1: meta variant (roster +1, from a REAL spawn of the same lineage) ──
"$SD/spawn" dummy2:helper2 "second template worker" >"$SB/logs/spawn-helper2.out" 2>&1 || true
sleep 1
cp -a "$SD/meta" "$SB/payloads/meta.variant"
diff -u "$SB/template/.ae/sessions/$S/meta" "$SB/payloads/meta.variant" > "$SB/payloads/meta.variant.diff" || true

# ── harvest 2: memo row with a topic absent from baseline ──
before_memo=$(wc -l < "$SD/memo.tsv")
TMUX_PANE="$WORK_PANE" "$SD/memo" add --topic mutationtopic "harvested memo row for the b0 arm" >>"$SB/logs/producers.out" 2>&1 || true
sleep 0.3
tail -n +$((before_memo+1)) "$SD/memo.tsv" > "$SB/payloads/memo.row"

# ── harvest 3..N: ask (request-opening) events not present in baseline ──
for i in 1 2 3 4; do
  TMUX_PANE="$MAIN_PANE" "$SD/ask" dummy2 "harvested mutation ask number $i" >>"$SB/logs/producers.out" 2>&1 || true
  sleep 0.4
  grep -E '"action":"ask"' "$SD/events.jsonl" | tail -1 > "$SB/payloads/events.ask.$i"
done
sleep 0.3
wc -l "$SB/payloads/"* 2>/dev/null || true
echo "--- meta diff ---"; cat "$SB/payloads/meta.variant.diff"
echo "--- memo row ---"; cat "$SB/payloads/memo.row"
echo "--- ask 1 ---"; cat "$SB/payloads/events.ask.1"
