#!/opt/homebrew/bin/bash
# A-H4 — SC-211p, the `_lib` name-resolution grammar.
#
# Observed on the generated `_lib` DIRECTLY: the case sources the exact producer-derived
# `_lib`, calls ae_resolve, and captures its rc together with AE_RESOLVED_{PANE,AGENT,SLOT,
# SESSION}. `focus` is not the observation surface — it mutates client focus, emits an
# event, and a failure it reports can originate downstream of the grammar, so grammar and
# liveness would be confounded in one rc.
#
# Inputs come from batch-h-input-list.md (the brief). Nothing here states what any input
# should produce.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H4
mkdir -p "$ADEST/$ARM"

########## fixture H5 — two live sessions on one server, built by a real launch ##########
h_sandbox h5 "cl:lead" "cx:lead,cx:solo,zz:only" || exit 1
h_launch th5a || { echo "launch A failed"; exit 1; }
A_META="$HMETA"; A_SESSION="$HSESSION"; A_SRV="$HSRV_PID"; A_HOME="$AE_HOME"; A_SOCK="$SOCK"
( cd "$ROOT/work" && "$HARNESS_BASH" "$FROZEN_AE" --local th5b </dev/null \
    >"$ROOT/cap/launch.th5b.out" 2>"$ROOT/cap/launch.th5b.err" )
B_META="$AE_HOME/sessions/th5b"
LIVE_PANE="$(h_pane_of cl:lead)"
DEAD_PANE="$(h_pane_of zz:only)"
command tmux -S "$A_SOCK" kill-pane -t "$DEAD_PANE" 2>/dev/null

########## the probe: source the generated _lib, call ae_resolve, print the outputs ##########
PROBE="$ROOT/probe.sh"
cat >"$PROBE" <<'PRB'
#!/opt/homebrew/bin/bash
# Sources the EXACT generated _lib and calls ae_resolve. Prints the resolver's own output
# contract (ae@72c7293:12983-12989) and its rc. No interpretation.
set -uo pipefail
META_DIR="$1"; TARGET="$2"
cd "$META_DIR" || exit 90
source "$META_DIR/_lib" || exit 91
ae_resolve "$TARGET"; rc=$?
printf 'rc=%s\n' "$rc"
printf 'AE_RESOLVED_PANE=%s\n'    "${AE_RESOLVED_PANE:-}"
printf 'AE_RESOLVED_AGENT=%s\n'   "${AE_RESOLVED_AGENT:-}"
printf 'AE_RESOLVED_SLOT=%s\n'    "${AE_RESOLVED_SLOT:-}"
printf 'AE_RESOLVED_SESSION=%s\n' "${AE_RESOLVED_SESSION:-}"
exit "$rc"
PRB
chmod +x "$PROBE"

# ENVIRONMENT-EQUIVALENCE CONTROL: the controller shell that sources _lib records its
# effective resolution domain beside that of a REAL generated-helper invocation from the
# same fixture, so a correct function observed in a different domain is visible as such.
ENVPROBE="$ROOT/envprobe.sh"
cat >"$ENVPROBE" <<'ENV'
#!/opt/homebrew/bin/bash
set -uo pipefail
META_DIR="$1"; cd "$META_DIR" || exit 90
source "$META_DIR/_lib" || exit 91
printf '_AE_SESSION=%s\n'      "${_AE_SESSION:-}"
printf '_AE_SESSIONS_DIR=%s\n' "${_AE_SESSIONS_DIR:-}"
printf 'AE_TMUX_SERVER=%s\n'   "${AE_TMUX_SERVER:-}"
printf 'AE_TMUX_SERVER_KIND=%s\n' "${AE_TMUX_SERVER_KIND:-}"
printf 'cwd=%s\n'              "$PWD"
printf 'TMUX=%s\n'             "${TMUX:-}"
printf 'TMUX_PANE=%s\n'        "${TMUX_PANE:-}"
ENV
chmod +x "$ENVPROBE"

########## cases — one per input class from the brief ##########
run_case() { # <case-id> <input-literal-or-EMPTY> <note>
    local cid="$1" input="$2" note="$3"
    case_open "$ARM" "$cid"
    led rows "rows=SC-211p" "surface=_lib ae_resolve" "fixture=H5 (two live sessions, one server)"
    led fixture "session_a=$A_SESSION" "session_b=th5b" "live_pane=$LIVE_PANE" "dead_pane=$DEAD_PANE"
    { h_roster; } >"$CASE_DIR/roster.txt"
    surface_state "$A_META/_lib" "$A_META/meta"
    # the environment-equivalence control, both domains, before the measured invocation
    env TMUX="${A_SOCK},${A_SRV},0" TMUX_PANE="$LIVE_PANE" "$ENVPROBE" "$A_META" \
        >"$CASE_DIR/env.helper-domain.txt" 2>&1
    "$ENVPROBE" "$A_META" >"$CASE_DIR/env.controller-domain.txt" 2>&1
    diff "$CASE_DIR/env.helper-domain.txt" "$CASE_DIR/env.controller-domain.txt" \
        >"$CASE_DIR/env.domain-diff.txt" 2>&1
    led env-equivalence "helper_sha256=$(sha "$CASE_DIR/env.helper-domain.txt")" \
        "controller_sha256=$(sha "$CASE_DIR/env.controller-domain.txt")" \
        "diff_lines=$(wc -l <"$CASE_DIR/env.domain-diff.txt" | tr -d ' ')"
    led measured-input "input=${input}" "note=$note"
    measured "$cid" "resolve" 20 -- env TMUX="${A_SOCK},${A_SRV},0" TMUX_PANE="$LIVE_PANE" \
        "$PROBE" "$A_META" "$input"
    led case-CLOSE "invocations_sha256=$(sha "$CASE_DIR/invocations.tsv")"
    echo "  $cid done"
}

run_case h4-c01-pane-live       "$LIVE_PANE"     "a live pane id"
run_case h4-c02-pane-dead       "$DEAD_PANE"     "a pane id whose pane was killed"
run_case h4-c03-xsession-ok     "@th5b:cl:remote" "cross-session, session exists"
run_case h4-c04-xsession-noagent "@th5b:cl:nosuch" "cross-session, session exists, agent absent"
run_case h4-c05-xsession-nosess "@th5zz:cl:lead" "cross-session, session absent"
run_case h4-c06-at-nocolon      "@th5b"          "@session with no colon"
run_case h4-c07-at-empty-sess   "@:lead"         "@:agent"
run_case h4-c08-at-empty-agent  "@th5b:"         "@session:"
run_case h4-c09-bare-unique     "solo"           "a bare name carried by one agent"
run_case h4-c10-bare-ambiguous  "lead"           "a bare name carried by two agents"
run_case h4-c11-alias-unique    "zz"             "an alias carried by one agent"
run_case h4-c12-alias-ambiguous "cx"             "an alias carried by two agents"
run_case h4-c13-exact-present   "cx:solo"        "an exact alias:name present"
run_case h4-c14-name-absent     "cl:nosuch"      "a name absent from the session"
run_case h4-c15-empty-string    ""               "the empty string"
echo "A-H4 DONE"
