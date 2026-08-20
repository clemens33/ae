#!/opt/homebrew/bin/bash
# SC-1301 — the meta-writer fault arm. Three writer-shaped cuts, THREE DIFFERENT evidence
# claims, because the three writers do not share a boundary.
#
#   cut 1  ae_meta_set (atomic): barrier between the temp write and the rename. Claim: what
#          a concurrent reader observes across that boundary.
#   cut 2  _cmd_spawn: it performs SEVERAL of its own appends, so a barrier between two of
#          them plus a controller SIGKILL yields a partial logical generation THE FROZEN
#          WRITER PRODUCED. Claim: an observed partial-generation state.
#   cut 3  start_capture_session_id: ONE append, so no mid-write window exists. A
#          controller-created partial line is admissible only as a READER-FAULT RESPONSE
#          probe, labelled that way in every artifact, and the untouched source writer is
#          captured separately in the same case.
#
# Admissibility: ONE hook-only patch over an exact 72c7293 copy; the INACTIVE hook proven
# equivalent to the unmodified control BEFORE any hooked capture, with a known-difference
# control proving the comparator can report red.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H1301
mkdir -p "$ADEST/$ARM"
HOOKED=/tmp/aecx/h/hook/ae
PATCHF=/tmp/aecx/h/hook/hook.patch

snapshot() { # <ae-home> <sock> -> a comparable state document
    local aehome="$1" sock="$2"
    ( cd "$aehome" 2>/dev/null && find . -type f -print0 | sort -z | while IFS= read -r -d '' f; do
        printf '%s\t%s\n' "$f" "$(shasum -a 256 "$f" 2>/dev/null | cut -d' ' -f1)"; done )
    echo "## tmux"
    command tmux -S "$sock" list-panes -a -F '#{session_name}|#{@ae_agent}|#{pane_current_command}' 2>&1 | sort
}

########## inactive-hook equivalence, before any hooked capture ##########
case_open "$ARM" "h1301-c00-inactive-equivalence"
led rows "rows=SC-1301" "phase=admissibility"
led hash-triple "frozen_sha256=$(sha "$FROZEN_AE")" "hooked_sha256=$(sha "$HOOKED")" \
    "patch_sha256=$(sha "$PATCHF")" "lines_added=$(grep -c '^+[^+]' "$PATCHF" | tr -d ' ')" \
    "lines_removed=$(grep -c '^-[^-]' "$PATCHF" | tr -d ' ')"
cp "$PATCHF" "$CASE_DIR/hook.patch"
surface_state "$HOOKED" "$FROZEN_AE"

equiv_run() { # <tag> <binary>
    # SAME session name in every run. The first attempt named the session after the tag, so
    # every path differed and the comparison reported the tag rather than the binary — a
    # comparator answering about its own labelling.
    h_sandbox "eq$1" "cl:lead" "cx:worker" || return 1
    ( cd "$ROOT/work" && "$HARNESS_BASH" "$2" --local "teq" </dev/null \
        >"$CASE_DIR/out/$1.launch.stdout" 2>"$CASE_DIR/out/$1.launch.stderr" )
    echo "rc=$?" >>"$CASE_DIR/out/$1.launch.rc"
    snapshot "$AE_HOME" "$SOCK" | sed "s#/tmp/aecx/h/eq$1#<ROOT>#g" >"$CASE_DIR/state.$1.txt"
    command tmux -S "$SOCK" kill-server >/dev/null 2>&1
}
equiv_run control "$FROZEN_AE"; ROOT_CONTROL="$AE_HOME"
equiv_run hooked  "$HOOKED";   ROOT_HOOKED="$AE_HOME"
# THE KNOWN-DIFFERENCE CONTROL: a binary that differs in a way the comparator must catch.
sed 's/^AE_VERSION=.*/AE_VERSION="0.0.0-known-difference"/' "$FROZEN_AE" >/tmp/aecx/h/hook/ae.different
equiv_run different /tmp/aecx/h/hook/ae.different
diff "$CASE_DIR/state.control.txt" "$CASE_DIR/state.hooked.txt" >"$CASE_DIR/equiv.control-vs-hooked.diff" 2>&1
EQ=$?
diff "$CASE_DIR/state.control.txt" "$CASE_DIR/state.different.txt" >"$CASE_DIR/equiv.control-vs-different.diff" 2>&1
DF=$?
diff "$CASE_DIR/out/control.launch.stdout" "$CASE_DIR/out/hooked.launch.stdout" >"$CASE_DIR/equiv.stdout.diff" 2>&1
# The hooked copy emits `_ae_hook` into every generated `_lib`, so a byte-identical `_lib`
# is impossible with this patch. Equivalence is therefore the ENUMERATED form: identical
# everywhere, except files whose only difference is the hook's own bytes — and the arm
# proves the difference set is exactly that rather than asserting it.
: >"$CASE_DIR/equiv.differing-files.txt"
: >"$CASE_DIR/equiv.non-hook-differences.txt"
while IFS=$'\t' read -r f h1; do
    h2="$(awk -F'\t' -v k="$f" '$1==k{print $2}' "$CASE_DIR/state.hooked.txt")"
    [[ -n "$h2" && "$h1" != "$h2" ]] || continue
    echo "$f" >>"$CASE_DIR/equiv.differing-files.txt"
    a="$ROOT_CONTROL/${f#./}"; b="$ROOT_HOOKED/${f#./}"
    if [[ -f "$a" && -f "$b" ]]; then
        if diff "$a" "$b" | grep -E '^[<>]' | grep -qv '_ae_hook'; then
            { echo "## $f — differs in bytes that are NOT the hook's"; diff "$a" "$b" | head -20; } \
                >>"$CASE_DIR/equiv.non-hook-differences.txt"
        fi
    fi
done < <(grep -v '^##' "$CASE_DIR/state.control.txt")
NONHOOK=$(wc -l <"$CASE_DIR/equiv.non-hook-differences.txt" | tr -d ' ')
led inactive-equivalence \
    "differing_files=$(wc -l <"$CASE_DIR/equiv.differing-files.txt" | tr -d ' ')" \
    "files_differing_in_NON_hook_bytes=$NONHOOK" \
    "control_vs_known_difference_differs=$( ((DF==0)) && echo no || echo yes )" \
    "stdout_identical=$( [[ -s "$CASE_DIR/equiv.stdout.diff" ]] && echo no || echo yes )" \
    "note=the hooked copy emits _ae_hook into every generated _lib, so equivalence is the enumerated form; the third column is the comparator's own can-fail control"
led case-CLOSE
if (( NONHOOK != 0 )); then
    led HARNESS-ABORT "reason=the inactive hook changed bytes that are not the hook's own"
    echo "INACTIVE EQUIVALENCE FAILED"; exit 1
fi
if ((DF == 0)); then
    led HARNESS-ABORT "reason=the comparator did not report a KNOWN difference — it cannot report red"
    echo "COMPARATOR CANNOT FAIL"; exit 1
fi
echo "  inactive equivalence proven, comparator proven able to fail"

########## cut 1 — the atomic writer's temp/rename window ##########
case_open "$ARM" "h1301-c01-atomic-temp-rename"
led rows "rows=SC-1301" "writer=ae_meta_set (atomic, temp+rename)" \
    "claim=what a concurrent reader observes across the boundary"
H_AE="$HOOKED" h_sandbox c1 "cl:lead" "" || exit 1
H_AE="$HOOKED" h_launch tc1301a || exit 1
M="$HMETA"
surface_state "$M/goal" "$M/meta"
cp "$M/meta" "$CASE_DIR/meta.before.txt"
MARK="$CASE_DIR/hook.mark"; WAIT="$CASE_DIR/hook.release"
canary pre c1
led PRODUCT-START "label=goal-set"
env AE_HOOK=AH_META_TEMP_COMPLETE AE_HOOK_MARK="$MARK" AE_HOOK_WAIT="$WAIT" \
    TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" \
    "$M/goal" "GOAL-UNDER-BARRIER" >"$CASE_DIR/out/goal-set.stdout" 2>"$CASE_DIR/out/goal-set.stderr" &
GP=$!
W=0; while (( W < 40 )); do [[ -s "$MARK" ]] && break; sleep 0.5; W=$((W+1)); done
led barrier-ARMED "point=AH_META_TEMP_COMPLETE" "mark_seen=$( [[ -s "$MARK" ]] && echo yes || echo no )" "waited_s=$((W/2))"
if [[ ! -s "$MARK" ]]; then
    led OUTCOME-INCONCLUSIVE "reason=the barrier never armed; anything captured after this is a completed write, not a state at a boundary"
    : >"$WAIT"; wait "$GP" 2>/dev/null; led case-CLOSE; command tmux -S "$SOCK" kill-server >/dev/null 2>&1
    echo "  cut 1 INCONCLUSIVE"; exit 1
fi
cp "$M/meta" "$CASE_DIR/meta.at-barrier.txt"
ls "$M" | grep 'meta.tmp' >"$CASE_DIR/temp-files-at-barrier.txt" 2>&1 || echo "(none)" >"$CASE_DIR/temp-files-at-barrier.txt"
env TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" "$M/goal" \
    >"$CASE_DIR/out/reader-at-barrier.stdout" 2>"$CASE_DIR/out/reader-at-barrier.stderr"
led reader-at-barrier "stdout_sha256=$(sha "$CASE_DIR/out/reader-at-barrier.stdout")" \
    "note=a concurrent read taken while the temp file exists and the rename has not happened"
: >"$WAIT"
wait "$GP" 2>/dev/null
cp "$M/meta" "$CASE_DIR/meta.after.txt"
led PRODUCT-COMPLETE "label=goal-set"
canary post c1
diff "$CASE_DIR/meta.before.txt" "$CASE_DIR/meta.at-barrier.txt" >"$CASE_DIR/meta.before-vs-barrier.diff" 2>&1
diff "$CASE_DIR/meta.at-barrier.txt" "$CASE_DIR/meta.after.txt" >"$CASE_DIR/meta.barrier-vs-after.diff" 2>&1
led case-CLOSE "before_vs_barrier_lines=$(wc -l <"$CASE_DIR/meta.before-vs-barrier.diff" | tr -d ' ')" \
    "barrier_vs_after_lines=$(wc -l <"$CASE_DIR/meta.barrier-vs-after.diff" | tr -d ' ')"
command tmux -S "$SOCK" kill-server >/dev/null 2>&1
echo "  cut 1 done"

########## cut 2 — a partial logical generation the FROZEN WRITER produced ##########
case_open "$ARM" "h1301-c02-spawn-partial-generation"
led rows "rows=SC-1301" "writer=_cmd_spawn (several direct appends)" \
    "claim=an observed partial-generation state, attributed to the product's own writes"
H_AE="$HOOKED" h_sandbox c2 "cl:lead" "" || exit 1
H_AE="$HOOKED" h_launch tc1301b || exit 1
M="$HMETA"
surface_state "$M/spawn" "$M/meta"
cp "$M/meta" "$CASE_DIR/meta.before.txt"
MARK="$CASE_DIR/hook.mark"; WAIT="$CASE_DIR/hook.release"
canary pre c2
led PRODUCT-START "label=spawn"
env AE_HOOK=AH_SPAWN_BETWEEN_APPENDS AE_HOOK_MARK="$MARK" AE_HOOK_WAIT="$WAIT" \
    AE_PATH="$HOOKED" TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" \
    "$M/spawn" cx:probe "fixture" >"$CASE_DIR/out/spawn.stdout" 2>"$CASE_DIR/out/spawn.stderr" &
SP=$!
W=0; while (( W < 60 )); do [[ -s "$MARK" ]] && break; sleep 0.5; W=$((W+1)); done
led barrier-ARMED "point=AH_SPAWN_BETWEEN_APPENDS" "mark_seen=$( [[ -s "$MARK" ]] && echo yes || echo no )" "waited_s=$((W/2))"
if [[ ! -s "$MARK" ]]; then
    led OUTCOME-INCONCLUSIVE "reason=the barrier never armed; a kill after a completed generation is not a partial-generation observation"
    kill -KILL "$SP" 2>/dev/null; led case-CLOSE; command tmux -S "$SOCK" kill-server >/dev/null 2>&1
    echo "  cut 2 INCONCLUSIVE"; exit 1
fi
cp "$M/meta" "$CASE_DIR/meta.at-barrier.txt"
led controller-action "action=SIGKILL the spawning process at the barrier" \
    "note=the bytes already in meta are the FROZEN WRITER's own; the controller wrote none of them"
pkill -KILL -P "$SP" 2>/dev/null; kill -KILL "$SP" 2>/dev/null; wait "$SP" 2>/dev/null
cp "$M/meta" "$CASE_DIR/meta.after-kill.txt"
env TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" "$M/agents" \
    >"$CASE_DIR/out/reader-after-kill.stdout" 2>"$CASE_DIR/out/reader-after-kill.stderr"
led PRODUCT-COMPLETE "label=spawn"
canary post c2
diff "$CASE_DIR/meta.before.txt" "$CASE_DIR/meta.after-kill.txt" >"$CASE_DIR/meta.before-vs-after.diff" 2>&1
{ echo "## the agent.* lines present after the kill, which the frozen writer appended"
  grep '^agent' "$CASE_DIR/meta.after-kill.txt" || echo "(none)"
  echo "## the keys a COMPLETE spawn generation writes, per ae:11939-11943"
  echo "agent.<slot>, agent_bin.<slot>, and launch_id.<slot> for a launch-id tool"; } \
  >"$CASE_DIR/partial-generation.txt"
led case-CLOSE "diff_lines=$(wc -l <"$CASE_DIR/meta.before-vs-after.diff" | tr -d ' ')"
command tmux -S "$SOCK" kill-server >/dev/null 2>&1
echo "  cut 2 done"

########## cut 3 — READER-FAULT RESPONSE, never an observed writer tear ##########
case_open "$ARM" "h1301-c03-reader-fault-response"
led rows "rows=SC-1301" "writer=start_capture_session_id (ONE append; no mid-write window)" \
    "claim=READER-FAULT RESPONSE — this is a controller-created state, NOT an observed writer tear"
H_AE="$HOOKED" h_sandbox c3 "cl:lead" "" || exit 1
H_AE="$HOOKED" h_launch tc1301c || exit 1
M="$HMETA"
surface_state "$M/_lib" "$M/meta"
cp "$M/meta" "$CASE_DIR/meta.before.txt"
# the untouched source writer, captured separately in the same case
canary pre c3a
led PRODUCT-START "label=untouched-writer"
env AE_HOOK=AH_CAPTURE_APPEND_DONE AE_HOOK_MARK="$CASE_DIR/hook.mark" \
    TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" \
    "$M/state" working "untouched writer" >"$CASE_DIR/out/untouched.stdout" 2>"$CASE_DIR/out/untouched.stderr"
led PRODUCT-COMPLETE "label=untouched-writer"
canary post c3a
cp "$M/meta" "$CASE_DIR/meta.untouched.txt"
# now the controller writes a partial line ITSELF, and says so in the artifact
PARTIAL='launch_time.main=17872'
printf '%s' "$PARTIAL" >>"$M/meta"
{ echo "## CONTROLLER-CREATED partial line — this is NOT something a writer was observed doing"
  echo "writer_shaped_bytes=$PARTIAL"
  echo "newline_present=no"
  echo "intended_complete_row=launch_time.main=<epoch>"
  echo "why_this_is_not_a_writer_tear=start_capture_session_id performs ONE append (ae:2073),"
  echo "  so there is no window in which the frozen writer can leave a line half-written."
  echo "what_this_probes=how a READER responds to a meta file whose last line lacks a newline"; } \
  >"$CASE_DIR/controller-mutation.txt"
cp "$M/meta" "$CASE_DIR/meta.with-partial.txt"
led controller-mutation "artifact_sha256=$(sha "$CASE_DIR/controller-mutation.txt")" \
    "bytes=$PARTIAL" "label=READER-FAULT RESPONSE"
canary pre c3b
led PRODUCT-START "label=reader"
env TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" "$M/agents" \
    >"$CASE_DIR/out/reader.stdout" 2>"$CASE_DIR/out/reader.stderr"
env TMUX="${SOCK},${HSRV_PID},0" TMUX_PANE="$(h_pane_of cl:lead)" "$M/goal" \
    >"$CASE_DIR/out/reader-goal.stdout" 2>"$CASE_DIR/out/reader-goal.stderr"
led PRODUCT-COMPLETE "label=reader"
canary post c3b
led case-CLOSE
command tmux -S "$SOCK" kill-server >/dev/null 2>&1
echo "  cut 3 done"
echo "A-H1301 DONE"
