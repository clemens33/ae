#!/opt/homebrew/bin/bash
# SC-510c — recover-ref bytes from the real producer (watchdog + _recover-pending).
source "$(dirname "$0")/tlib.sh"
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
t_launch ta1c || { echo FAILED; cat "$CAP/ae-launch.err"; exit 1; }
LID="$(grep '^launch_id.main=' "$META/meta" | cut -d= -f2-)"
DAY="$(/bin/date -u +%Y/%m/%d)"
mkdir -p "$HOME/.codex/sessions/$DAY"
PLANT="$HOME/.codex/sessions/$DAY/rollout-planted.jsonl"
printf '{"id":"11111111-2222-3333-4444-555555555555","timestamp":"2026-08-20T00:00:00Z","cwd":"%s"}\n' "$ROOT/work" >"$PLANT"
printf '{"type":"message","role":"user","content":"AE_CODEX_LAUNCH_ID=%s"}\n' "$LID" >>"$PLANT"
{
  echo "planted PRODUCER INPUT (an external codex-shaped session log; not an ae fixture byte),"
  echo "written AFTER launch so its mtime is newer than launch_time.main, and carrying the"
  echo "launch marker ae itself injected, so the frozen matcher's own filters are satisfied:"
  echo "  path=$PLANT"
  echo "  sha256=$(shasum -a 256 "$PLANT" | cut -d' ' -f1) bytes=$(stat -f %z "$PLANT")"
  echo "  mtime=$(stat -f %m "$PLANT")  launch_time.main=$(grep '^launch_time.main=' "$META/meta" | cut -d= -f2-)"
  echo "  launch_id.main=$LID"
  echo "  content:"; sed 's/^/    /' "$PLANT"
  echo "agent binary: the controllable fake copied to /tmp/aecx/bin/codex so the frozen tool"
  echo "classifier reports tool_kind=codex and the real recover path runs. No live model."
  echo "  codex_fake_sha256=$(shasum -a 256 /tmp/aecx/bin/codex | cut -d' ' -f1)"
  echo "  meta agent line before recovery: $(grep '^agent.main=' "$META/meta")"
  echo "  NOTE: no harness probe of _recover-pending is run before the producer — it CLAIMS the"
  echo "  pending slot (it writes meta), which would consume the recovery the watchdog must emit."
} >>"$P"
_t0=$(/bin/date -u +%s); GOT=0
while (( $(/bin/date -u +%s) - _t0 < 120 )); do
    grep -q '"action":"recover"' "$META/events.jsonl" 2>/dev/null && { GOT=1; break; }
    sleep 3
done
echo "  recover_event_present=$GOT" >>"$P"
(( GOT == 1 )) || echo "  OUTCOME=INCONCLUSIVE reason=no recover byte within 120s" >>"$P"
echo "  meta agent line after: $(grep '^agent.main=' "$META/meta")" >>"$P"
tm capture-pane -p -J -S -200 -E - -t "$(tm list-panes -a -F '#{pane_id} #{@ae_agent}' | awk '$2=="_watchdog"{print $1;exit}')" >"$CAP/watchdog.log" 2>/dev/null
"$META/watchdog" stop >/dev/null 2>&1
cp "$PLANT" "$AE_HOME/_a1-510c-planted-producer-input.jsonl"
cat "$META/events.jsonl" 2>/dev/null
mkdir -p "$TSTORE/A1/_meta"
echo "A1/510c fingerprint(pre)=$(t_store A1 510c-recover-ref "$P")"; t_protect A1 510c-recover-ref >/dev/null
cp "$CAP/watchdog.log" "$TSTORE/A1/_meta/510c-recover-ref.watchdog.log" 2>/dev/null
t_teardown
