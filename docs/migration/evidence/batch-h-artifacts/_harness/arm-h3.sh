#!/opt/homebrew/bin/bash
# A-H3 — the argument surface: SC-211a-j and SC-212c.
#
# One case per input class from the executor brief. Each case runs ONE invocation of ONE
# generated helper, bracketed by capture-path canaries, and records the session directory's
# own bytes before and after — several of these helpers WRITE, and a refusal that wrote
# something is a different reading from a refusal that did not.
#
# SC-211l (`say`) is NOT here: it runs under its own containment section.
# Helper groups each get their own launched sandbox, so one group's mutations cannot
# become another group's precondition — the A-H4 lesson.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H3
mkdir -p "$ADEST/$ARM"

dir_bytes() { # <dir> -> path<TAB>sha256 per file, sorted
    ( cd "$1" 2>/dev/null && find . -type f -print0 | sort -z | while IFS= read -r -d '' f; do
        printf '%s\t%s\n' "$f" "$(shasum -a 256 "$f" 2>/dev/null | cut -d' ' -f1)"
      done )
}

G_META=""; G_SOCK=""; G_SRV=""; G_PANE=""
group() { # <group-id> <main> <workers>
    pkill -x aefake >/dev/null 2>&1
    h_sandbox "h3$1" "$2" "${3:-}" || return 1
    h_launch "th3$1" || return 1
    G_META="$HMETA"; G_SOCK="$SOCK"; G_SRV="$HSRV_PID"; G_WORK="$ROOT/work"
    G_PANE="$(h_pane_of "$2")"
    echo "== group $1 launched: $HSESSION pane $G_PANE"
}

hcase() { # <case-id> <helper> [args...]
    local cid="$1" helper="$2"; shift 2
    case_open "$ARM" "$cid"
    led rows "rows=SC-211x" "helper=$helper" "group_session=${TSESSION:-$HSESSION}"
    surface_state "$G_META/$helper" "$G_META/_lib"
    dir_bytes "$G_META" >"$CASE_DIR/session-bytes.before.tsv"
    led measured-input "helper=$helper" "argv=$*" "invocation_cwd=$G_WORK"
    run_in "$G_WORK" measured "$cid" "$helper" 25 -- env TMUX="${G_SOCK},${G_SRV},0" \
        TMUX_PANE="$G_PANE" "$G_META/$helper" "$@"
    dir_bytes "$G_META" >"$CASE_DIR/session-bytes.after.tsv"
    diff "$CASE_DIR/session-bytes.before.tsv" "$CASE_DIR/session-bytes.after.tsv" \
        >"$CASE_DIR/session-bytes.diff.txt" 2>&1
    led write-record "changed_lines=$(wc -l <"$CASE_DIR/session-bytes.diff.txt" | tr -d ' ')" \
        "before_sha256=$(sha "$CASE_DIR/session-bytes.before.tsv")" \
        "after_sha256=$(sha "$CASE_DIR/session-bytes.after.tsv")"
    led case-CLOSE "invocations_sha256=$(sha "$CASE_DIR/invocations.tsv")"
    echo "  $cid"
}

########## SC-211a — state ##########
group a "cl:lead" "" || exit 1
hcase h3-a01-no-args            state
hcase h3-a02-working            state working
hcase h3-a03-waiting-user       state waiting-user
hcase h3-a04-done               state done
hcase h3-a05-blocked-reason     state blocked "a reason"
hcase h3-a06-blocked-no-reason  state blocked
hcase h3-a07-unknown-mode       state nosuchmode
hcase h3-a08-empty-mode         state ""
hcase h3-a09-leading-dash       state -working
hcase h3-a10-extra-words        state working extra words here

########## SC-211b — goal ##########
group b "cl:lead" "" || exit 1
hcase h3-b01-no-args            goal
hcase h3-b02-one-word           goal alpha
hcase h3-b03-several-words      goal alpha beta gamma
hcase h3-b04-clear              goal --clear
hcase h3-b05-clear-extra        goal --clear extra
hcase h3-b06-help-long          goal --help
hcase h3-b07-help-short         goal -h
hcase h3-b08-controls-only      goal "$(printf '\t\n')"

########## SC-211c — memo ##########
group c "cl:lead" "" || exit 1
hcase h3-c01-no-args            memo
hcase h3-c02-add-text           memo add "a memo line"
hcase h3-c03-add-no-text        memo add
hcase h3-c04-add-topic-text     memo add --topic arch "a topical line"
hcase h3-c05-add-topic-bare     memo add --topic
hcase h3-c06-add-topic-no-text  memo add --topic arch
hcase h3-c07-read               memo read
hcase h3-c08-read-topic         memo read --topic arch
hcase h3-c09-read-topic-bare    memo read --topic
hcase h3-c10-read-extra         memo read extra
hcase h3-c11-tail               memo tail
hcase h3-c12-tail-n             memo tail 3
hcase h3-c13-tail-nonnumeric    memo tail abc
hcase h3-c14-tail-extra         memo tail 3 extra
hcase h3-c15-unknown-subcmd     memo nosuchsub

########## SC-211d / SC-212c — requests ##########
group d "cl:lead" "cx:worker" || exit 1
hcase h3-d01-no-args            requests
hcase h3-d02-mine               requests mine
hcase h3-d03-inbox              requests inbox
hcase h3-d04-all                requests all
hcase h3-d05-unknown-mode       requests nosuchmode
hcase h3-d06-extra-args         requests all extra

########## SC-211e — peek ##########
group e "cl:lead" "cx:worker" || exit 1
hcase h3-e01-no-args            peek
hcase h3-e02-absent-target      peek cl:nosuch
hcase h3-e03-present-no-count   peek cx:worker
hcase h3-e04-nonnumeric-count   peek cx:worker abc
hcase h3-e05-negative-count     peek cx:worker -5
hcase h3-e06-leading-plus       peek cx:worker +5
hcase h3-e07-zero               peek cx:worker 0
hcase h3-e08-above-cap          peek cx:worker 99999
hcase h3-e09-extra-args         peek cx:worker 10 extra

########## SC-211f — agents ##########
group f "cl:lead" "cx:worker" || exit 1
hcase h3-f01-no-args            agents
hcase h3-f02-all                agents --all
hcase h3-f03-other-arg          agents nosucharg
chmod 000 "$G_META/meta"
hcase h3-f04-all-unreadable-meta agents --all
chmod 644 "$G_META/meta"

########## SC-211g — focus ##########
group g "cl:lead" "cx:worker" || exit 1
hcase h3-g01-no-args            focus
hcase h3-g02-absent-target      focus cl:nosuch
hcase h3-g03-present            focus cx:worker
hcase h3-g04-extra-args         focus cx:worker extra

########## SC-211h — interrupt ##########
group h "cl:lead" "cx:worker" || exit 1
hcase h3-h01-no-args            interrupt
hcase h3-h02-absent-target      interrupt cl:nosuch
hcase h3-h03-present-no-message interrupt cx:worker
hcase h3-h04-present-message    interrupt cx:worker "a message"
SHELLPANE="$(h_pane_of cx:worker)"
command tmux -S "$G_SOCK" respawn-pane -k -t "$SHELLPANE" "/bin/sh" >/dev/null 2>&1
sleep 1
hcase h3-h05-shell-pane-message interrupt "$SHELLPANE" "a message into a shell"

########## SC-211i — spawn ##########
group i "cl:lead" "" || exit 1
hcase h3-i01-no-args            spawn
hcase h3-i02-unknown-alias      spawn nosuchalias:name
cp "$G_META/meta" "$G_META/meta.bak"; : >"$G_META/meta"
hcase h3-i03-empty-meta         spawn cx:probe
cp "$G_META/meta.bak" "$G_META/meta"
printf 'not a config at all\n' >"$AE_HOME/config"
hcase h3-i04-malformed-config   spawn cx:probe
( cd "$ROOT/work" && "$HARNESS_BASH" "$FROZEN_AE" --local th3i2 >/dev/null 2>&1 ) || true

########## SC-211j — retire ##########
group j "cl:lead" "cx:worker" || exit 1
env TMUX="${G_SOCK},${G_SRV},0" TMUX_PANE="$G_PANE" "$G_META/spawn" cx:helper "fixture" \
    >"$ROOT/cap/spawn.out" 2>"$ROOT/cap/spawn.err"
hcase h3-j01-no-args            retire
hcase h3-j02-absent-name        retire cl:nosuch
hcase h3-j03-main-agent         retire cl:lead
hcase h3-j04-configured-worker  retire cx:worker
hcase h3-j05-pane-outside       retire %999
hcase h3-j06-extra-args         retire cx:helper extra
hcase h3-j07-valid-spawned      retire cx:helper
echo "A-H3 DONE"
