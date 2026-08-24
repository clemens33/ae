#!/usr/bin/env bash
# SC-017s probe — the pane-observed live predicate.
#
# THREE SEPARATE OUTCOMES, and they are not the same thing:
#   rc=0  the predicate answered correctly on measured bytes
#   rc=1  PRODUCT FAIL — the predicate is wrong
#   rc=2  FIXTURE ABORT — the world this probe built is not the world it meant to
#         build, so nothing below it is evidence about the predicate
# The distinction exists because this probe's first two versions each failed in a
# way that presented as the other: pane-index targeting produced a confident
# product FAIL that was entirely the fixture's, and launching `sleep 300` through
# the default shell made pane_current_command report the SHELL on a fish host —
# a fixture defect that reads as the predicate being wrong.
#
# MEASURE, THEN ASSERT THE PRECONDITIONS, THEN JUDGE. Every precondition this
# probe needs is measured and checked before any verdict is read. A probe that
# cannot confirm its fixture landed is measuring its own assumption.
#
# Short socket path on purpose: macOS sun_path is 104 bytes.
set -uo pipefail

SOCK="/tmp/ae017s.$$"
SESS="p017s"
fail=0
note() { printf '%s\n' "$*"; }
abort() { note ""; note "FIXTURE ABORT: $*"; note "Nothing above is evidence about the predicate."; exit 2; }
cleanup() { tmux -S "$SOCK" kill-server >/dev/null 2>&1 || true; rm -f "$SOCK"; }
trap cleanup EXIT
command -v tmux >/dev/null || { note "SKIP: no tmux"; exit 77; }

# ---- the predicates -------------------------------------------------------
# The closed shell set is command_is_shell's (ae@72c7293:428-434), INCLUDING "".
is_shell() { case "${1-}" in bash | zsh | fish | sh | dash | "") return 0 ;; *) return 1 ;; esac; }
# SC-017s as ratified: alive iff the pane is not dead AND the command is not a shell.
predicate() { if [[ "${1-}" != "0" ]]; then printf 'unknown'; elif is_shell "${2-}"; then printf 'unknown'; else printf 'alive'; fi; }
# The DEFECT this probe red-proves: the command field alone, ignoring pane_dead.
predicate_cmd_only() { if is_shell "${1-}"; then printf 'unknown'; else printf 'alive'; fi; }

# ---- world: one server, three panes, addressed by the id tmux returns ------
# `exec` so the agent process REPLACES the shell: without it tmux runs the
# command through $SHELL and pane_current_command reports the shell instead
# (measured on a fish host by gpt56sol:colead).
tmux -S "$SOCK" new-session -d -s "$SESS" -n w0 'exec sleep 30' || abort "cannot start a tmux server on $SOCK"
p_nonshell="$(tmux -S "$SOCK" list-panes -t "$SESS:w0" -F '#{pane_id}' | head -1)"
p_shell="$(tmux -S "$SOCK" split-window -t "$SESS:w0" -d -P -F '#{pane_id}' "${SHELL:-/bin/sh}")"
p_exited="$(tmux -S "$SOCK" split-window -t "$SESS:w0" -d -P -F '#{pane_id}' 'exec true')"
tmux -S "$SOCK" set-option -p -t "$p_exited" remain-on-exit on >/dev/null 2>&1
tmux -S "$SOCK" set-option -p -t "$p_nonshell" @ae_agent 'alias:nonshell' >/dev/null
tmux -S "$SOCK" set-option -p -t "$p_shell"    @ae_agent 'alias:shellpane' >/dev/null
tmux -S "$SOCK" set-option -p -t "$p_exited"   @ae_agent 'alias:exited' >/dev/null
sleep 1

note "=== FIXTURE (pane ids resolved, never guessed) ==="
printf '  nonshell=%s shell=%s exited=%s\n' "$p_nonshell" "$p_shell" "$p_exited"

note ""
note "=== RAW: the enumeration SC-017p already needs, one call ==="
enum="$(tmux -S "$SOCK" list-panes -s -t "$SESS" -F '#{@ae_agent}	#{pane_current_command}	#{pane_dead}	#{pane_id}' 2>/dev/null)"
printf '%s\n' "$enum" | cat -v | sed 's/^/  /'

field() { printf '%s\n' "$enum" | awk -F'\t' -v k="$1" -v f="$2" '$1==k{print $f}'; }
cmd_nonshell="$(field 'alias:nonshell' 2)"; dead_nonshell="$(field 'alias:nonshell' 3)"
cmd_shell="$(field 'alias:shellpane' 2)";   dead_shell="$(field 'alias:shellpane' 3)"
cmd_exited="$(field 'alias:exited' 2)";     dead_exited="$(field 'alias:exited' 3)"

note ""
note "=== PRECONDITIONS (measured, asserted BEFORE any verdict) ==="
for m in alias:nonshell alias:shellpane alias:exited; do
    n="$(printf '%s\n' "$enum" | awk -F'\t' -v k="$m" '$1==k{c++} END{print c+0}')"
    [[ "$n" == "1" ]] || abort "marker $m appears $n times, want exactly 1"
done
note "  ok  all three markers present exactly once"
is_shell "$cmd_nonshell" && abort "the non-shell pane reports [$cmd_nonshell], which IS in the closed shell set — the exec fixture did not land on this host"
note "  ok  non-shell pane reports [$cmd_nonshell], outside the closed shell set"
is_shell "$cmd_shell" || abort "the shell pane reports [$cmd_shell], which is NOT in the closed shell set"
note "  ok  shell pane reports [$cmd_shell], inside the closed shell set"
[[ "$dead_nonshell" == "0" ]] || abort "the non-shell pane reports pane_dead=$dead_nonshell, want 0"
[[ "$dead_exited" == "1" ]] || abort "the exited pane reports pane_dead=$dead_exited, want 1 — remain-on-exit did not hold it"
is_shell "$cmd_exited" && abort "the exited pane reports [$cmd_exited], a shell — this arm needs a NON-shell command to discriminate"
note "  ok  exited pane reports pane_dead=1 with command [$cmd_exited], outside the shell set"
note "      (that combination is the whole point: the command field alone would say alive)"

note ""
note "=== VERDICTS (SC-017s as ratified) ==="
check() { local got; got="$(predicate "$2" "$3")"
    if [[ "$got" == "$4" ]]; then printf '  ok   %-28s %-8s (dead=%s cmd=[%s])\n' "$1" "$got" "$2" "$3"
    else printf '  FAIL %-28s got=%-8s want=%-8s (dead=%s cmd=[%s])\n' "$1" "$got" "$4" "$2" "$3"; fail=1; fi; }
check 'non-shell, live pane'   "$dead_nonshell" "$cmd_nonshell" alive
check 'shell foreground'       "$dead_shell"    "$cmd_shell"    unknown
check 'exited pane'            "$dead_exited"   "$cmd_exited"   unknown
if [[ "$(predicate 0 '')" == "unknown" ]]; then
    printf '  ok   %-28s %-8s (unit: live tmux produced no empty reading here)\n' 'empty command reading' unknown
else printf '  FAIL empty command reading must not be alive\n'; fail=1; fi

note ""
note "=== RED-PROOF 1: the pane_dead conjunct, on the SAME measured bytes ==="
seed="$(predicate_cmd_only "$cmd_exited")"; fixed="$(predicate "$dead_exited" "$cmd_exited")"
printf '  command-only predicate  -> %s\n' "$seed"
printf '  SC-017s predicate       -> %s\n' "$fixed"
if [[ "$seed" == "alive" && "$fixed" == "unknown" ]]; then
    note "  CAUGHT: the defect yields ALIVE on a pane_dead=1 pane; the conjunct yields unknown."
else
    note "  SEED DID NOT LAND — the two predicates did not diverge on these bytes."
    note "  That is an INVALID TEST, not a pass: it proves nothing about the conjunct."
    fail=1
fi

note ""
note "=== RED-PROOF 2: the empty reading, frozen list-path set vs command_is_shell ==="
frozen_is_shell() { case "${1-}" in bash | zsh | fish | sh | dash) return 0 ;; *) return 1 ;; esac; }
fv="$(if frozen_is_shell ''; then printf unknown; else printf alive; fi)"
printf '  ae:4201-4206 set on an empty reading -> %s\n' "$fv"
printf '  ae:428-434  set on an empty reading -> %s\n' "$(if is_shell ''; then printf unknown; else printf alive; fi)"
if [[ "$fv" == "alive" ]]; then note "  CAUGHT: the list-path set turns an empty/failed read into a POSITIVE alive."
else note "  SEED DID NOT LAND — control did not reproduce; the ae:4201-4206 claim is unproven here."; fail=1; fi

note ""
if [[ $fail -eq 0 ]]; then note "SC-017s PROBE: PASS"; else note "SC-017s PROBE: PRODUCT FAIL"; fi
exit $fail
