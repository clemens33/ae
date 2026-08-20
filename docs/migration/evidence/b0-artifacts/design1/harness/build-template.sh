#!/opt/homebrew/bin/bash
# B0/Design 1 — template session builder. Producer-derived bytes only.
set -euo pipefail
SB=/tmp/aeb0
AE="$SB/frozen/ae"

# ── real-config tripwire (separate statements; AGENTS.md isolation footgun) ──
REAL_HOME="$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
REAL_CFG="${REAL_HOME}/.ae/config"
_fpr() { shasum -a 256 2>/dev/null | awk '{print $1}'; }
if [[ -f "$REAL_CFG" ]]; then REAL_FPR="$(_fpr <"$REAL_CFG")"; else REAL_FPR=absent; fi
trap 'now=absent; [[ -f "$REAL_CFG" ]] && now="$(_fpr <"$REAL_CFG")"; \
      if [[ "$now" != "$REAL_FPR" ]]; then echo "FATAL: real ~/.ae/config changed" >&2; exit 97; fi' EXIT

export HOME="$SB/tmpl/home"
export AE_HOME="$SB/tmpl/home/.ae"
export TMUX_TMPDIR="$SB/sock"
export AE_TMUX_SERVER="aeb0tmpl"
export AE_TMUX_SERVER_KIND="name"
export TZ=UTC
export LANG=C
mkdir -p "$AE_HOME" "$TMUX_TMPDIR"
T() { command tmux -L "$AE_TMUX_SERVER" "$@"; }

cat > "$AE_HOME/config" <<'CFG'
[agents]
dummy = "bash"
dummy2 = "bash"

[workspace]
main = dummy
layout = vertical
CFG

REPO="$SB/tmpl/repo"
mkdir -p "$REPO"
git -C "$REPO" init -q
git -C "$REPO" config user.email "b0@probe"
git -C "$REPO" config user.name "b0probe"
git -C "$REPO" commit -q --allow-empty -m init

S=b0tmpl
SD="$AE_HOME/sessions/$S"
(cd "$REPO" && "$AE" --local "$S" >"$SB/logs/launch.out" 2>"$SB/logs/launch.err" &)
i=0; while ! T has-session -t "$S" 2>/dev/null; do sleep 0.5; i=$((i+1)); ((i<40)) || { echo "TIMEOUT session" >&2; exit 1; }; done
i=0; while [[ ! -f "$SD/meta" ]]; do sleep 0.4; i=$((i+1)); ((i<40)) || { echo "TIMEOUT meta" >&2; exit 1; }; done
sleep 1

# ── producer ops (baseline) ──
"$SD/spawn" dummy2:helper "template worker" >"$SB/logs/spawn-helper.out" 2>&1 || true
sleep 1
MAIN_PANE="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy:dummy"{print $1; exit}')"
WORK_PANE="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy2:helper"{print $1; exit}')"
echo "MAIN_PANE=$MAIN_PANE WORK_PANE=$WORK_PANE"
[[ -n "$MAIN_PANE" && -n "$WORK_PANE" ]] || { echo "MISSING PANES" >&2; T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' >&2; exit 1; }

TMUX_PANE="$MAIN_PANE" "$SD/goal" "b0 template baseline goal" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$MAIN_PANE" "$SD/state" working "template baseline" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$WORK_PANE" "$SD/state" working "worker baseline" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$MAIN_PANE" "$SD/memo" add --topic build "baseline memo row one" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$WORK_PANE" "$SD/memo" add --topic handover "baseline handover row" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$MAIN_PANE" "$SD/ask" dummy2 "baseline question one" >>"$SB/logs/producers.out" 2>&1 || true
REQ1="$(grep -E '"action":"(ask|review)"' "$SD/events.jsonl" | tail -1 | grep -oE '"ref":"[^"]+"' | cut -d'"' -f4)"
TMUX_PANE="$WORK_PANE" "$SD/reply" "$REQ1" "baseline answer one" >>"$SB/logs/producers.out" 2>&1 || true
TMUX_PANE="$MAIN_PANE" "$SD/ask" dummy2 "baseline question two (left open)" >>"$SB/logs/producers.out" 2>&1 || true
sleep 0.5
echo "--- baseline events ---"; wc -l "$SD/events.jsonl" "$SD/memo.tsv"; ls "$SD"
