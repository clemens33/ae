#!/opt/homebrew/bin/bash
# B0 Design 7 (SC-511c) fixture builder. Producer-derived bytes only.
# Alert-family specimens are deliberately ABSENT (seat ruling 2026-08-20: the
# T-WD precursor harvest is the only legitimate alert-byte source).
set -euo pipefail
SB=/tmp/aeb0
D7="$SB/d7"
AE="$SB/frozen/ae"
BASE=1787184000        # 2026-08-20T00:00:00Z — the pinned fixture clock origin
STEP=60

# ── real-config tripwire ──
REAL_HOME="$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory 2>/dev/null | awk '{print $2}')"
REAL_CFG="${REAL_HOME}/.ae/config"
_fpr() { shasum -a 256 2>/dev/null | awk '{print $1}'; }
if [[ -f "$REAL_CFG" ]]; then REAL_FPR="$(_fpr <"$REAL_CFG")"; else REAL_FPR=absent; fi
trap 'now=absent; [[ -f "$REAL_CFG" ]] && now="$(_fpr <"$REAL_CFG")"; \
      if [[ "$now" != "$REAL_FPR" ]]; then echo "FATAL: real ~/.ae/config changed" >&2; exit 97; fi' EXIT

export HOME="$D7/build/home"
export AE_HOME="$D7/build/home/.ae"
export TMUX_TMPDIR="$D7/build/sock"
export AE_TMUX_SERVER="aeb0d7b"
export AE_TMUX_SERVER_KIND="name"
# LANG: fixed for the whole design. NOT LANG=C — measured 2026-08-20 in this
# sandbox: with LANG=C the generated send/ask/review helpers fail agent resolution
# ("agent '<a>' not found in session '<s>'") while `tmux list-panes` under the same
# locale returns the roster correctly. Recorded as an environment fact; not
# interpreted here.
export TZ=UTC
export LANG=en_US.UTF-8
unset TMUX TMUX_PANE   # do not inherit the operator's live tmux client
export AE_DATE_REAL=/bin/date
export AE_DATE_SHIM_STATE="$D7/build/clock"
export AE_DATE_SHIM_LOG="$D7/build/date-shim.log"
export AE_DATE_SHIM_SUBSTITUTE=1
export PATH="$D7/shim:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"

rm -rf "$D7/build"; mkdir -p "$AE_HOME" "$TMUX_TMPDIR"
: > "$AE_DATE_SHIM_LOG"
TICK=0
clock() { TICK=$((TICK + 1)); printf '%s\n' "$((BASE + TICK * STEP))" > "$AE_DATE_SHIM_STATE"; }
clock
T() { command tmux -L "$AE_TMUX_SERVER" "$@"; }
LOG="$D7/build/producers.log"
: > "$LOG"
run() { printf '\n$ %s\n' "$*" >>"$LOG"; "$@" >>"$LOG" 2>&1 || printf '(rc=%s)\n' "$?" >>"$LOG"; }

cat > "$AE_HOME/config" <<'CFG'
[agents]
dummy = "bash"
dummy2 = "bash"
dummy3 = "bash"

[workspace]
main = dummy
layout = vertical
watchdog = false
CFG

REPO="$D7/repo"; mkdir -p "$REPO"
git -C "$REPO" init -q
git -C "$REPO" config user.email "b0@probe"; git -C "$REPO" config user.name "b0probe"
git -C "$REPO" commit -q --allow-empty -m init

launch() { # <name>
    (cd "$REPO" && "$AE" --local "$1" >"$D7/build/launch.$1.log" 2>&1 &)
    local i=0
    while ! T has-session -t "$1" 2>/dev/null; do sleep 0.5; i=$((i+1)); ((i<40)) || { echo "TIMEOUT session $1" >&2; exit 1; }; done
    i=0
    while [[ ! -f "$AE_HOME/sessions/$1/meta" ]]; do sleep 0.4; i=$((i+1)); ((i<40)) || { echo "TIMEOUT meta $1" >&2; echo "--- launch.$1.log ---" >&2; cat "$D7/build/launch.$1.log" >&2; exit 1; }; done
    sleep 1
}

S=b0d7; X=xpartner   # NOT a prefix sibling of $S: tmux -t prefix-matches, and a
                     # prefix sibling makes `has-session -t b0d7` match `b0d7x`.
# The cross-session target session is launched FIRST and left to settle, so the
# cross-session ask below resolves against a fully-established session.
launch "$X"
sleep 2
launch "$S"
SD="$AE_HOME/sessions/$S"
clock; run "$SD/spawn" dummy2:helper "fixture worker"
sleep 1
as() { local p="$1"; shift; clock; printf '\n$ TMUX_PANE=%s %s\n' "$p" "$*" >>"$LOG"; TMUX_PANE="$p" "$@" >>"$LOG" 2>&1 || printf '(rc=%s)\n' "$?" >>"$LOG"; }
# The delivery helpers (ask/review/send) resolve a LIVE pane and can transiently
# refuse while a freshly spawned pane is still settling. Bounded retry, logged.
as_retry() { local p="$1"; shift; local i=0 rc=0
    while ((i < 12)); do
        clock; printf '\n$ TMUX_PANE=%s %s   (attempt %s)\n' "$p" "$*" "$((i+1))" >>"$LOG"
        rc=0; TMUX_PANE="$p" "$@" >>"$LOG" 2>&1 || rc=$?
        ((rc == 0)) && return 0
        printf '(rc=%s, retrying)\n' "$rc" >>"$LOG"; sleep 2; i=$((i+1))
    done
    { echo "--- diagnostics at give-up ---"
      T list-panes -a -F '#{pane_id}|#{session_name}|#{@ae_agent}|#{@ae_slot}' 2>&1
      echo "--- agents helper ---"; "$SD/agents" 2>&1
      echo "--- meta roster ---"; grep '^agent' "$SD/meta" 2>&1
    } >>"$LOG" 2>&1
    printf '(GAVE UP after %s attempts)\n' "$i" >>"$LOG"; return 1
}
# settle: the spawned pane must carry its identity options before any resolution
# The delivery path (send, and ask/review which exec it) stays unresolvable for a
# while after a spawn whose brief delivery reported INCOMPLETE. Poll it with a real
# `send` until it succeeds; the successful probe send is itself a producer specimen
# and is recorded as such. Bounded; every attempt logged.
settle_delivery() { local from="$1" to="$2" i=0 rc=0
    while ((i < 20)); do
        clock; rc=0
        printf '\n$ [settle probe %s] TMUX_PANE=%s send %s\n' "$((i+1))" "$from" "$to" >>"$LOG"
        TMUX_PANE="$from" "$SD/send" "$to" "fixture settle probe" >>"$LOG" 2>&1 || rc=$?
        ((rc == 0)) && { printf '(settle probe succeeded on attempt %s)\n' "$((i+1))" >>"$LOG"; return 0; }
        printf '(rc=%s)\n' "$rc" >>"$LOG"; sleep 3; i=$((i+1))
    done
    echo "TIMEOUT settling delivery $from -> $to" >&2; return 1
}
settle_pane() { local pane="$1" i=0
    while ((i < 20)); do
        [[ -n "$(T display-message -p -t "$pane" '#{@ae_slot}' 2>/dev/null)" ]] && return 0
        sleep 0.5; i=$((i+1))
    done
    echo "TIMEOUT settling pane $pane" >&2; return 1
}

MAIN="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy:dummy"{print $1; exit}')"
HELP="$(T list-panes -s -t "$S" -F '#{pane_id} #{@ae_agent}' | awk '$2=="dummy2:helper"{print $1; exit}')"
[[ -n "$MAIN" && -n "$HELP" ]] || { echo "MISSING PANES" >&2; exit 1; }
echo "MAIN=$MAIN HELP=$HELP"
settle_pane "$HELP"
settle_delivery "$MAIN" dummy2:helper

as "$MAIN" "$SD/goal" "fixture goal one"
as "$MAIN" "$SD/goal" "fixture goal two"
as "$MAIN" "$SD/state" working "main working"
as "$HELP" "$SD/state" working "helper working"
as "$HELP" "$SD/state" waiting-user "helper waiting on the human"
as "$MAIN" "$SD/memo" add --topic design "fixture memo row"
as "$HELP" "$SD/memo" add --topic handover "fixture handover row"
as "$MAIN" "$SD/say" "fixture chat line"
as_retry "$MAIN" "$SD/ask" dummy2:helper "fixture question, closed by a reply"
REQ="$(command grep -E '"action":"ask"' "$SD/events.jsonl" | tail -1 | command grep -oE '"ref":"[^"]+"' | cut -d'"' -f4)"
as_retry "$HELP" "$SD/reply" "$REQ" "fixture answer"
as_retry "$MAIN" "$SD/review" dummy2:helper "fixture review request, left open"
as_retry "$MAIN" "$SD/ask" dummy2:helper "fixture second ask, left open"

# cross-session target specimen (@session:agent form)
as_retry "$MAIN" "$SD/ask" "@${X}:dummy" "fixture cross-session ask"

as "$MAIN" "$SD/state" blocked "main blocked on a fixture dependency"
as "$HELP" "$SD/state" done "helper finished the fixture task"

# stop-request/stop-result specimens via the real fleet stop flow (the singular
# external `ae stop <name>` path emits neither; measured).
clock; printf '\n$ ae stop all -y\n' >>"$LOG"
"$AE" stop all -y >>"$LOG" 2>&1 || printf '(rc=%s)\n' "$?" >>"$LOG"
sleep 3

echo "--- events ---"; wc -l "$SD/events.jsonl"
command grep -oE '"action":"[^"]+"' "$SD/events.jsonl" | sort | uniq -c
echo "--- keys present ---"
python3 - "$SD/events.jsonl" <<'PY'
import json,sys,collections
c=collections.Counter()
for l in open(sys.argv[1]):
    l=l.strip()
    if not l: continue
    for k in json.loads(l): c[k]+=1
for k,v in c.most_common(): print(" %-16s %d" % (k,v))
PY
