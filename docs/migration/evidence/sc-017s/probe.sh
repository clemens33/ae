#!/usr/bin/env bash
# SC-017s probe — a list-level live predicate over #{pane_current_command}.
#
# MEASURE FIRST, ASSERT SECOND. Every tmux reading is captured and printed
# before any predicate is applied, so a fixture defect shows up as a strange
# READING rather than as a product verdict.
#
# The predicate under test is the frozen `command_is_shell` set
# (ae@72c7293:428-434), INCLUDING the empty string.
# Short socket path on purpose: macOS sun_path is 104 bytes.
set -uo pipefail

SOCK="/tmp/ae017s.$$"
SESS="p017s"
fail=0
note() { printf '%s\n' "$*"; }
cleanup() { tmux -S "$SOCK" kill-server >/dev/null 2>&1 || true; rm -f "$SOCK"; }
trap cleanup EXIT
command -v tmux >/dev/null || { note "SKIP: no tmux"; exit 77; }

is_shell() { case "${1-}" in bash | zsh | fish | sh | dash | "") return 0 ;; *) return 1 ;; esac; }
verdict() { if is_shell "${1-}"; then printf 'unknown'; else printf 'alive'; fi; }

# --- world: one server; panes addressed by the ID tmux returns, never by index.
tmux -S "$SOCK" new-session -d -s "$SESS" -n w0 'sleep 300' || { note "FAIL: no tmux server"; exit 1; }
p_nonshell="$(tmux -S "$SOCK" list-panes -t "$SESS:w0" -F '#{pane_id}' | head -1)"
p_shell="$(tmux -S "$SOCK" split-window -t "$SESS:w0" -d -P -F '#{pane_id}' "${SHELL:-/bin/sh}")"
tmux -S "$SOCK" set-option -p -t "$SESS:w0" remain-on-exit on >/dev/null 2>&1
p_exited="$(tmux -S "$SOCK" split-window -t "$SESS:w0" -d -P -F '#{pane_id}' 'true')"
tmux -S "$SOCK" set-option -p -t "$p_exited" remain-on-exit on >/dev/null 2>&1
tmux -S "$SOCK" set-option -p -t "$p_nonshell" @ae_agent 'alias:nonshell' >/dev/null
tmux -S "$SOCK" set-option -p -t "$p_shell"    @ae_agent 'alias:shellpane' >/dev/null
tmux -S "$SOCK" set-option -p -t "$p_exited"   @ae_agent 'alias:exited' >/dev/null
sleep 1

note "=== FIXTURE (pane ids resolved, not guessed) ==="
printf '  nonshell=%s shell=%s exited=%s\n' "$p_nonshell" "$p_shell" "$p_exited"

note ""
note "=== RAW: the enumeration SC-017p already needs, one call ==="
enum="$(tmux -S "$SOCK" list-panes -s -t "$SESS" -F '#{@ae_agent}	#{pane_current_command}	#{pane_id}	#{pane_dead}' 2>/dev/null)"
printf '%s\n' "$enum" | cat -v | sed 's/^/  /'

# INSTRUMENT CHECK: every marker must be present exactly once, or nothing below is evidence.
for m in alias:nonshell alias:shellpane alias:exited; do
    n="$(printf '%s\n' "$enum" | awk -F'\t' -v k="$m" '$1==k{c++} END{print c+0}')"
    [[ "$n" == "1" ]] || { note "INSTRUMENT BROKEN: marker $m appears $n times — no verdict is evidence."; exit 1; }
done
note "  instrument ok: all three markers present exactly once"

get() { printf '%s\n' "$enum" | awk -F'\t' -v k="$1" '$1==k{print $2}'; }
c_nonshell="$(get 'alias:nonshell')"
c_shell="$(get 'alias:shellpane')"
c_exited="$(get 'alias:exited')"

note ""
note "=== READINGS (measured, before any assertion) ==="
printf '  %-30s [%s]\n' 'non-shell foreground'     "$c_nonshell"
printf '  %-30s [%s]\n' 'shell foreground'         "$c_shell"
printf '  %-30s [%s]\n' 'pane whose process exited' "$c_exited"

note ""
note "=== PREDICATE (applied after reading) ==="
check() { local got; got="$(verdict "$3")"
    if [[ "$got" == "$2" ]]; then printf '  ok   %-30s %-8s (cmd=[%s])\n' "$1" "$got" "$3"
    else printf '  FAIL %-30s got=%-8s want=%-8s (cmd=[%s])\n' "$1" "$got" "$2" "$3"; fail=1; fi; }
check 'non-shell foreground'     alive   "$c_nonshell"
check 'shell foreground'         unknown "$c_shell"
# The exited pane is measured, not predicted: whatever tmux reports, the row must
# not read it as alive unless a real non-shell process is in the foreground.
note "  -- exited pane: tmux reports [$c_exited]; predicate says $(verdict "$c_exited")"

note ""
note "=== UNIT: the empty reading, which live tmux may never produce ==="
printf '  empty string -> %s (want unknown)\n' "$(verdict '')"
[[ "$(verdict '')" == "unknown" ]] || { note "  FAIL: empty must not be alive"; fail=1; }

note ""
note "=== CONTROL: the frozen LIST-PATH set (ae:4201-4206) omits the empty string ==="
frozen_is_shell() { case "${1-}" in bash | zsh | fish | sh | dash) return 0 ;; *) return 1 ;; esac; }
fv="$(if frozen_is_shell ''; then printf unknown; else printf alive; fi)"
printf '  frozen list-path set on an empty reading -> %s\n' "$fv"
if [[ "$fv" == "alive" ]]; then
    note "  CONFIRMED: ae:4201-4206 turns an empty/failed read into a POSITIVE alive;"
    note "  ae:428-434 (command_is_shell, which includes \"\") does not."
else
    note "  NOT CONFIRMED — control did not reproduce; the ae:4201-4206 claim is unproven."; fail=1
fi

note ""
if [[ $fail -eq 0 ]]; then note "SC-017s PROBE: PASS"; else note "SC-017s PROBE: FAIL"; fi
exit $fail
