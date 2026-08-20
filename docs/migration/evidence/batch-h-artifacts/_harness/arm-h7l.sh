#!/opt/homebrew/bin/bash
# SC-211l — `say`, the one row whose blast radius can leave the machine.
#
# `helper_say_main` (ae:14470-14486) makes no network call: it appends a `chat` event and
# prints one line (ae:14485). The hop that leaves the machine belongs to a SEPARATE bridge
# process that tails a session's events file, so containment addresses the WATCHER.
#
# LAYER 1 — STRUCTURAL, and it is the load-bearing one. The bridge takes its root from
#   AE_HOME (telegram-daemon:10-11); this fixture's AE_HOME is a randomly named directory
#   created after the system census. Reach is INHERITED ACROSS FORK, so a child cannot
#   reach what its parent cannot, and the census enumerates long-lived ROOTS.
# LAYER 2 — the census, corroborating. It does NOT match on argv: a census whose own
#   command line contains its search string counts itself. It classifies by REACH — each
#   process's own AE_HOME — and excludes the arm's own processes by a TOKEN they carry,
#   which no foreign process can hold. Both directions are demonstrated.
# LAYER 3 — PATH-first curl/wget stubs that refuse and log. Scope stated honestly: they
#   contain only what the ARM spawns. An already-running bridge never inherited this PATH
#   and is NOT contained by them.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H7L
mkdir -p "$ADEST/$ARM"

ARM_TOKEN="AE_H_ARM_TOKEN_$$_$(/bin/date -u +%s)"
STUBDIR=/tmp/aecx/h/stubs
mkdir -p "$STUBDIR"
for tool in curl wget; do
    cat >"$STUBDIR/$tool" <<STUB
#!/opt/homebrew/bin/bash
# Refusing stub. It never delegates, so nothing that inherits this PATH can reach the
# network, and every attempt is recorded.
printf '%s\t%s\t%s\n' "\$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" "$tool" "\$*" >>"\${AE_H_STUB_LOG:-/tmp/aecx/h/stub.log}"
echo "$tool: refused by the arm's stub" >&2
exit 7
STUB
    chmod +x "$STUBDIR/$tool"
done

# THE CENSUS, AND WHAT IT CANNOT DO ON THIS PLATFORM.
#
# Classification is by REACH — a process's own AE_HOME — never by name, because a census
# whose command line contains its search string counts itself. But macOS exposes a
# process's environment to `ps e` only for a SUBSET of even one's own processes: measured
# here, 1 of 40 sampled. A process whose environment cannot be read CANNOT BE CLASSIFIED,
# so the census reports three classes and a count of the unclassifiable, and it does not
# claim zero in-range watchers — only zero AMONG THOSE IT CAN READ. Layer 1 carries the
# containment claim; this layer corroborates it and states its own blind spot.
census() { # <out-file> <fixture-ae-home>
    local out="$1" fixture="$2"
    { echo "## every process of this uid, classified by REACH (its own AE_HOME), not by name"
      echo "fixture_ae_home=$fixture"
      echo "arm_token=$ARM_TOKEN"
      echo "## macOS exposes an environment to ps e for only some processes; one it cannot"
      echo "## read is UNKNOWN-REACH, not out-of-range."
      printf 'pid\tppid\treach\ttoken\tae_home\n'
      while read -r pid ppid; do
          [[ "$pid" =~ ^[0-9]+$ ]] || continue
          local env_txt home tok reach
          env_txt="$(ps eww -p "$pid" 2>/dev/null | tail -1 | tr ' ' '\n')"
          if ! printf '%s\n' "$env_txt" | grep -qE '^[A-Za-z_][A-Za-z0-9_]*='; then
              printf '%s\t%s\t%s\t%s\t%s\n' "$pid" "$ppid" "UNKNOWN-REACH" "unknown" "<env unreadable>"
              continue
          fi
          home="$(printf '%s\n' "$env_txt" | grep '^AE_HOME=' | head -1 | cut -d= -f2-)"
          tok=no; printf '%s\n' "$env_txt" | grep -q "^AE_H_ARM_TOKEN=$ARM_TOKEN$" && tok=yes
          reach=out-of-range
          [[ -n "$home" && "$fixture" == "$home"* ]] && reach=IN-RANGE
          printf '%s\t%s\t%s\t%s\t%s\n' "$pid" "$ppid" "$reach" "$tok" "${home:-<none>}"
      done < <(ps -eo pid=,ppid=)
    } >"$out"
}

census_counts() { # <census-file>
    awk -F'\t' 'NR>5 {c[$3]++} END {printf "in_range=%d out_of_range=%d unknown_reach=%d",
        c["IN-RANGE"]+0, c["out-of-range"]+0, c["UNKNOWN-REACH"]+0}' "$1"
}

run_case() { # <case-id> <note> <invoker...>
    local cid="$1" note="$2"; shift 2
    case_open "$ARM" "$cid"
    led rows "rows=SC-211l" "surface=say" "containment=layer1 structural, layer2 census, layer3 refusing stubs"
    surface_state "$SAY_META/say" "$SAY_META/_lib"

    # LAYER 3, DEMONSTRATED: fire the stub deliberately and require its log to carry it.
    : >"$CASE_DIR/stub.log"
    AE_H_STUB_LOG="$CASE_DIR/stub.log" PATH="$STUBDIR:$PATH" curl https://example.invalid \
        >"$CASE_DIR/stub-probe.out" 2>"$CASE_DIR/stub-probe.err" || true
    led stub-demonstrated "log_lines=$(wc -l <"$CASE_DIR/stub.log" | tr -d ' ')" \
        "note=a recorder nobody has seen fire is not evidence of silence"
    [[ -s "$CASE_DIR/stub.log" ]] || { led HARNESS-ABORT "reason=the curl stub did not record its own invocation"; return 1; }

    # LAYER 2, BOTH DIRECTIONS: an in-range control the census MUST report, and the arm's
    # own token-carrying process it MUST exclude.
    # `bash -c 'sleep 25'` EXECS sleep and the environment stops being readable — the
    # control would then be unreportable for a reason that has nothing to do with the
    # census. The trailing `:` prevents the exec optimisation.
    env AE_HOME="$AE_HOME/control-in-range" /opt/homebrew/bin/bash -c 'sleep 25; :' &
    local ctl=$!
    env AE_H_ARM_TOKEN="$ARM_TOKEN" AE_HOME="$AE_HOME" /opt/homebrew/bin/bash -c 'sleep 25; :' &
    local mine=$!
    sleep 1
    census "$CASE_DIR/census.control.txt" "$AE_HOME"
    local saw_ctl saw_mine
    saw_ctl=$(grep -c "^$ctl	" "$CASE_DIR/census.control.txt" || true); saw_ctl=${saw_ctl//[^0-9]/}
    saw_mine=$(awk -F'\t' -v p="$mine" '$1==p && $4=="yes"{n++} END{print n+0}' "$CASE_DIR/census.control.txt")
    led census-control "in_range_control_pid=$ctl" "reported=$saw_ctl" \
        "harness_pid=$mine" "carries_token=$saw_mine" \
        "note=a census that cannot report a KNOWN in-range watcher cannot report their absence"
    kill "$ctl" "$mine" 2>/dev/null
    [[ "${saw_ctl:-0}" -ge 1 ]] || { led OUTCOME-INCONCLUSIVE "reason=the census did not report its own in-range control"; return 1; }

    census "$CASE_DIR/census.pre.txt" "$AE_HOME"
    led census-pre "artifact_sha256=$(sha "$CASE_DIR/census.pre.txt")" "$(census_counts "$CASE_DIR/census.pre.txt")" \
        "note=unknown_reach are processes whose environment this platform will not expose; they are NOT counted as out of range"
    cp "$SAY_META/events.jsonl" "$CASE_DIR/events.before.jsonl" 2>/dev/null || : >"$CASE_DIR/events.before.jsonl"
    led measured-input "note=$note" "argv=$*"
    run_in "$SAY_WORK" measured "$cid" "say" 20 -- "$@"
    cp "$SAY_META/events.jsonl" "$CASE_DIR/events.after.jsonl" 2>/dev/null || : >"$CASE_DIR/events.after.jsonl"
    diff "$CASE_DIR/events.before.jsonl" "$CASE_DIR/events.after.jsonl" >"$CASE_DIR/events.diff.txt" 2>&1
    census "$CASE_DIR/census.post.txt" "$AE_HOME"
    led census-post "artifact_sha256=$(sha "$CASE_DIR/census.post.txt")" "$(census_counts "$CASE_DIR/census.post.txt")"
    led stub-log-after "lines=$(wc -l <"$CASE_DIR/stub.log" | tr -d ' ')"
    led case-CLOSE "events_diff_lines=$(wc -l <"$CASE_DIR/events.diff.txt" | tr -d ' ')"
    echo "  $cid"
}

# fixture: a randomly named AE_HOME created AFTER the system-root census
census /tmp/aecx/h/system-roots.txt "/nonexistent-fixture-not-yet-created"
h_sandbox "l$(/bin/date -u +%s)" "cl:lead" "" || exit 1
h_launch th7l || { echo "launch failed"; exit 1; }
SAY_META="$HMETA"; SAY_WORK="$ROOT/work"; SAY_SOCK="$SOCK"; SAY_SRV="$HSRV_PID"
SAY_PANE="$(h_pane_of cl:lead)"
export PATH="$STUBDIR:$PATH"
SAYCMD=(env TMUX="${SAY_SOCK},${SAY_SRV},0" TMUX_PANE="$SAY_PANE" AE_H_STUB_LOG="/tmp/aecx/h/stub.log" "$SAY_META/say")

run_case h7l-c01-argv-text     "text via argv"                    "${SAYCMD[@]}" "a line via argv"
run_case h7l-c02-whitespace    "whitespace-only text"             "${SAYCMD[@]}" "   "
run_case h7l-c03-no-args-pipe  "no args with redirected empty stdin" \
    /opt/homebrew/bin/bash -c "'${SAY_META}/say' </dev/null"
run_case h7l-c04-stdin-text    "text via a pipe" \
    /opt/homebrew/bin/bash -c "printf 'a line via stdin\n' | '${SAY_META}/say'"
run_case h7l-c05-no-args-tty   "no args on a real TTY" \
    python3 "$HERE/pty-run.py" "${SAY_META}/say"
echo "A-H7L DONE"
