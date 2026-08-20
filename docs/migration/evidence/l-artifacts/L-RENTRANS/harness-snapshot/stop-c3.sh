#!/opt/homebrew/bin/bash
# L-STOP: legacy-migration-injection (SC-839e).
set -uo pipefail
source /tmp/aelx/lib/stop-lib.sh

TOPOLOGY="leg + legx (prefix-sibling pair), one recorded server"
HOSTILE='leg'"'"'"$(touch SENTINEL_TOUCHED)"'"'"'x'

l_arm_begin L-STOP legacy-migration-injection frozen
PATCHV="none (frozen, unmodified)"
stop_fleet leg legx
l_arm_preflight leg || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; exit 1; }
ssnap 1pre

SD="$R/h/.ae/sessions"
printf '%s' "$HOSTILE" >"$R/cap/hostile-name.raw"
{ printf 'hostile.name.bytes (od -c)\n'; printf '%s' "$HOSTILE" | od -c; } >"$R/cap/hostile-name.od.txt"

# ── the named controller mutation into the LEGACY physical direct-child shape ──
{ printf 'migration.step.1\ttmux rename-session -t "=leg" -> the hostile name\n'
  printf 'migration.step.2\tmv %s/leg -> %s/<hostile>\n' "$SD" "$SD"
  printf 'migration.step.3\tthe session meta key session= is rewritten to the hostile name (mode preserved)\n'
  printf 'migration.note\targv-safe throughout: every controller invocation passes the name as ONE argv word, never through a shell string\n'
  printf 'sentinel.mechanism\tthe name embeds $(touch SENTINEL_TOUCHED); a shell that EVALUATES it creates that file in its cwd, and the whole sandbox is scanned for it\n'
} >"$R/cap/migration.txt"

/opt/homebrew/bin/tmux -S "$SOCK" rename-session -t '=leg' "$HOSTILE" >"$R/cap/rename.out" 2>&1
printf 'rename.rc\t%s\n' "$?" >>"$R/cap/rename.out"
mv "$SD/leg" "$SD/$HOSTILE"
META="$SD/$HOSTILE/meta"
cp -p "$META" "$R/cap/meta.before.txt"
MODE0="$(stat -f '%Lp' "$META")"
python3 - "$META" "$HOSTILE" <<'PY'
import sys
p, name = sys.argv[1], sys.argv[2]
lines = open(p, 'r', encoding='utf-8', errors='surrogateescape').read().splitlines(True)
out = []
for ln in lines:
    if ln.startswith('session='):
        out.append('session=' + name + '\n')
    else:
        out.append(ln)
open(p + '.tmp', 'w', encoding='utf-8', errors='surrogateescape').write(''.join(out))
PY
chmod "0$MODE0" "$META.tmp"; mv "$META.tmp" "$META"
cp -p "$META" "$R/cap/meta.after.txt"
diff -u "$R/cap/meta.before.txt" "$R/cap/meta.after.txt" >"$R/cap/mutation.diff" 2>&1
printf 'meta.mode.before\t%s\nmeta.mode.after\t%s\n' "$MODE0" "$(stat -f '%Lp' "$META")" >>"$R/cap/migration.txt"

sleep 1
l_manifest "$SD" "$R/cap/2migrated.sessions.tsv"
l_tmuxsnap "$SOCK" "$R/cap/2migrated.tmux.txt"

# ── re-prove C1..C4 from a shell pane inside the migrated session ──────────────
PANE="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -s -t "=$HOSTILE" -F '#{pane_id}' | head -1)"
WIN="$(/opt/homebrew/bin/tmux -S "$SOCK" new-window -d -t "$HOSTILE:" -c "$R/w" -P -F '#{pane_id}')"
sleep 1
{ printf 'reproof.C1\t$TMUX and $TMUX_PANE in the new shell pane\n'
  /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$WIN" -l -- 'printf "TMUX=%s TMUX_PANE=%s\n" "$TMUX" "$TMUX_PANE"'
  /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$WIN" Enter
  sleep 1
  printf 'reproof.C2/C3\tserver socket_path + pid, round-tripped from this socket\n'
  /opt/homebrew/bin/tmux -S "$SOCK" display-message -p '#{socket_path} #{pid}'
  printf 'reproof.C4\tthe new pane resolves to session id + name:\n'
  /opt/homebrew/bin/tmux -S "$SOCK" display-message -p -t "$WIN" '#{session_id} #{session_name}'
  printf 'reproof.C5\tthe pane tty:\n'
  /opt/homebrew/bin/tmux -S "$SOCK" display-message -p -t "$WIN" '#{pane_tty}'
  printf 'first_agent_pane\t%s\nshell_pane\t%s\n' "$PANE" "$WIN"
} >"$R/cap/reproof-C1-C4.txt" 2>&1
pane_cap "$WIN" >"$R/cap/reproof-pane.txt" 2>&1

# sentinel BEFORE
find "$R" -name SENTINEL_TOUCHED >"$R/cap/sentinel.1before.txt" 2>&1
printf 'sentinel.count.before\t%s\n' "$(grep -c . "$R/cap/sentinel.1before.txt")" >>"$R/cap/sentinel.1before.txt"

# ── the implicit no-name stop route, typed into that pane ──────────────────────
: >"$R/cap/tmux-argv.log"
/opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$WIN" -l -- "$R/b/ae stop -y > $R/cap/2op.stdout 2> $R/cap/2op.stderr; echo \$? > $R/cap/2op.rc"
/opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$WIN" Enter
i=0; while (( i < 400 )); do [[ -s "$R/cap/2op.rc" ]] && break; sleep 0.1; i=$((i+1)); done
(( i >= 400 )) && printf 'INCONCLUSIVE: no exit status observed within the 40s bound\n' >"$R/cap/INCONCLUSIVE.txt"
sleep 6
cp "$R/cap/tmux-argv.log" "$R/cap/tmux-argv.op.log"

# sentinel AFTER — the whole sandbox, plus every plausible cwd
find "$R" -name SENTINEL_TOUCHED >"$R/cap/sentinel.3after.txt" 2>&1
printf 'sentinel.count.after\t%s\n' "$(grep -c . "$R/cap/sentinel.3after.txt")" >>"$R/cap/sentinel.3after.txt"
{ printf 'scan.roots\t%s (recursive)\n' "$R"
  printf 'sentinel.name\tSENTINEL_TOUCHED\n'
  printf 'sentinel.paths.before\n'; cat "$R/cap/sentinel.1before.txt"
  printf 'sentinel.paths.after\n'; cat "$R/cap/sentinel.3after.txt"; } >"$R/cap/sentinel-state.txt"

ssnap 3post
l_manifest "$SD" "$R/cap/3post.sessions-full.tsv"
{ printf '# stop-result rows\n'
  for f in "$SD"/*/events.jsonl; do [[ -e "$f" ]] || continue; printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done; } >"$R/cap/stop-results.txt" 2>&1

sarmtxt legacy-migration-injection SC-839e \
  "a VALID real session is launched, then a named controller mutation moves it into the LEGACY physical direct-child shape under a name carrying quoting and command-substitution syntax with an embedded sentinel: tmux rename-session, a matching state-directory move, and a meta session= rewrite (mode preserved). C1-C4 are re-proved from a shell pane inside the migrated session, then the implicit no-name stop route runs there. Identified as the legacy-migration arm; this is never an allowlisted launch" \
  "hostile_name_file	hostile-name.raw (byte-exact) and hostile-name.od.txt" \
  "op	ae stop -y typed into a shell pane of the migrated session (implicit no-name route)" \
  "op_rc	$(cat "$R/cap/2op.rc" 2>/dev/null || echo '<none>')" \
  "sentinel_before	$(grep '^sentinel.count.before' "$R/cap/sentinel.1before.txt" | cut -f2)" \
  "sentinel_after	$(grep '^sentinel.count.after' "$R/cap/sentinel.3after.txt" | cut -f2)" \
  "bound_sec	40"
l_arm_end
echo DONE
