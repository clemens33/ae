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
for i in 1 2 3 4; do
  TMUX_PANE="$MAIN_PANE" "$SD/ask" dummy2:helper "harvested mutation ask number $i" >>"$SB/logs/producers.out" 2>&1 || true
  sleep 0.6
  grep -E '"action":"ask"' "$SD/events.jsonl" | tail -1 > "$SB/payloads/events.ask.$i"
done
for i in 1 2 3 4; do echo "--- ask $i ---"; cut -c1-260 < "$SB/payloads/events.ask.$i"; done
