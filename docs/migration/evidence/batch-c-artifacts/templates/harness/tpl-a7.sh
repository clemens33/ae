#!/opt/homebrew/bin/bash
# A7 fixtures: meta grammar.
#   405a  a producer-written meta value containing MULTIPLE '=' characters
#   405f  goal APPEND ORDER OPPOSED to timestamp order, plus the two controls an order
#         claim needs: an AGREEING pair (both candidate answers coincide) and a SINGLE
#         goal. Without them, an opposed pair alone cannot show the reader responds at all.
#   405g  tmux @ae_branch_name and the git branch given DIFFERENT values
#   405j  five identity cases sharing ONE display name
source "$(dirname "$0")/tlib.sh"
MUT="$SCRATCH/harness/mutate.py"
SHIM=/tmp/aecx/shim; REALDATE=/bin/date
banner() { echo; echo "######## $* ########"; }
run() { echo "  as=$1 helper='${*:2}'" >>"$P"; as_agent "$@"; echo "    rc=$?" >>"$P"; }

########## 405a — a meta value with multiple '=' ##########
banner "A7 meta-multi-equals (405a)"
t_sandbox a7a ""
t_launch ta7a || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; P="$CAP/prov.txt"; : >"$P"
{ echo "the goal helper WRITES the value into meta, so a goal text containing several '='"
  echo "characters produces a genuine meta line whose value contains them. A first-equals"
  echo "split and an any-equals split disagree on such a line."; } >>"$P"
run "$LEAD" goal 'alpha=beta=gamma delta=epsilon'
echo "  meta goal line: $(grep '^goal=' "$META/meta")" >>"$P"
grep '^goal=' "$META/meta"
mkdir -p "$TSTORE/A7/_meta"
echo "A7/meta-multi-equals fp=$(t_store A7 meta-multi-equals "$P")"; t_protect A7 meta-multi-equals >/dev/null
t_teardown

########## 405f — order arms: opposed, agreeing, single ##########
build_goal_order() { # <member> <first-now> <first-text> <second-now> <second-text> <note>
    local mem="$1" n1="$2" t1="$3" n2="$4" t2="$5" note="$6"
    banner "A7 $mem (405f)"
    t_sandbox "a7f${mem##*-}" ""
    export PATH="$SHIM:$PATH"; export AE_REAL_DATE="$REALDATE"
    export AE_DATE_SHIM_LOG="$ROOT/cap/date-shim.log"; : >"$AE_DATE_SHIM_LOG"
    t_launch "ta7${mem//[^a-z0-9]/}" || { echo FAILED; return 1; }
    LEAD="$(pane_of fake:lead)"; P="$CAP/prov.txt"; : >"$P"
    { echo "$note"; echo "clock hook active; real date=$REALDATE sha256=$(shasum -a 256 $REALDATE | cut -d' ' -f1)"; } >>"$P"
    export AE_FAKE_NOW="$n1"; echo "  append 1 at frozen now=$n1 text='$t1'" >>"$P"
    as_agent "$LEAD" goal "$t1" >/dev/null; echo "    rc=$?" >>"$P"
    if [[ -n "$n2" ]]; then
        export AE_FAKE_NOW="$n2"; echo "  append 2 at frozen now=$n2 text='$t2'" >>"$P"
        as_agent "$LEAD" goal "$t2" >/dev/null; echo "    rc=$?" >>"$P"
    fi
    unset AE_FAKE_NOW
    { echo "  events (append order, top to bottom):"; sed 's/^/    /' "$META/events.jsonl"
      echo "  meta goal line: $(grep '^goal=' "$META/meta")"; } >>"$P"
    cat "$META/events.jsonl"
    echo "A7/$mem fp=$(t_store A7 "$mem" "$P")"; t_protect A7 "$mem" >/dev/null
    cp "$AE_DATE_SHIM_LOG" "$TSTORE/A7/_meta/$mem.date-shim-invocations.log" 2>/dev/null || true
    t_teardown
    unset AE_DATE_SHIM_LOG AE_REAL_DATE
}
build_goal_order goal-order-opposed 1755003600 'GOAL-TEXT-WITH-NEWER-TS' 1755000000 'GOAL-TEXT-WITH-OLDER-TS' \
"APPEND ORDER OPPOSED TO TIMESTAMP ORDER. The first append carries the NEWER ts and the
second append carries the OLDER one, so a reader taking the LAST RECORD and a reader
taking the MAXIMUM TIMESTAMP return different goal texts. Without that opposition the two
implementations are indistinguishable."
build_goal_order goal-order-agreeing 1755000000 'GOAL-TEXT-FIRST' 1755003600 'GOAL-TEXT-SECOND' \
"CONTROL: append order AGREES with timestamp order, so both candidate readers return the
same text. This is what the arm needs to know the reader responds to a second goal at all —
an opposed pair alone cannot tell a discriminating reader from an inert one."
build_goal_order goal-order-single 1755000000 'GOAL-TEXT-ONLY' '' '' \
"CONTROL: a single goal. Establishes the baseline rendering with nothing to choose between."

########## 405g — branch name from two sources, given DIFFERENT values ##########
banner "A7 branch-two-sources (405g)"
t_sandbox a7g ""
t_launch ta7g || { echo FAILED; exit 1; }
P="$CAP/prov.txt"; : >"$P"
GITBRANCH="$(cd "$ROOT/work" && git rev-parse --abbrev-ref HEAD)"
tm set-option -t "$TSESSION" @ae_branch_name "TMUX-OPTION-BRANCH-VALUE"
{ echo "the two candidate sources are given DELIBERATELY DIFFERENT values, so which one the"
  echo "consumer reports is observable rather than a coincidence:"
  echo "  git branch in the work dir : $GITBRANCH"
  echo "  tmux @ae_branch_name       : TMUX-OPTION-BRANCH-VALUE"
  echo "The live subarm runs with the option set on a running server; the stopped subarm"
  echo "runs from the same fixture with no server at all, leaving only the git source."
} >>"$P"
echo "  tmux option now: $(tm show-options -v -t "$TSESSION" @ae_branch_name 2>&1)" >>"$P"
echo "A7/branch-two-sources fp=$(t_store A7 branch-two-sources "$P")"; t_protect A7 branch-two-sources >/dev/null
t_teardown

########## 405j — identity cases sharing ONE display name ##########
banner "A7 identity base (405j)"
t_sandbox a7j "fake:worker"
t_launch ta7j || { echo FAILED; exit 1; }
LEAD="$(pane_of fake:lead)"; P="$CAP/prov.txt"; : >"$P"
echo "a real ask, so all four routing members are present and fresh; the five 405j cases" >>"$P"
echo "are named byte mutations of THIS line, each keeping the same display name." >>"$P"
run "$LEAD" ask fake:worker "A7 405j identity question"
cat "$META/events.jsonl"
echo "A7/405j-full-fresh fp=$(t_store A7 405j-full-fresh "$P")"; t_protect A7 405j-full-fresh >/dev/null
t_teardown
echo "TPL-A7 PART 1 DONE"
