#!/opt/homebrew/bin/bash
# ARM GROUP A8 — modes. Rows: SC-101, SC-102a, SC-102b, SC-018b.
#
# This is the first DELIBERATELY MUTATING group, so the manifest diff stops being a
# read-only proof and becomes the measurement. Every arm therefore declares, BEFORE the
# run, the paths the HARNESS itself touches, and change-record.py enumerates every changed
# path partitioned by who wrote it. A mutation nobody declared is visible as PRODUCT rather
# than absorbed into a count. The declaration is about the controller's own actions; it is
# not a claim about what the product should do.
source "$(dirname "$0")/armlib.sh"
ARMG=A8
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

a8_case() { # <case-id> <rows> <stage-fn> <mutate-fn> <harness-touched...>
    local cid="$1" rows="$2" stage="$3" mutate="$4"; shift 4
    local -a touched=("$@")
    # Two controls on the controller's OWN equipment, declared here so both
    # instruments have a positive control in EVERY case: one probe is rewritten
    # byte-identically (only the write witness can see that), one has its content
    # changed (only then does the content manifest have anything to report).
    touched+=(".a8-witness-probe-rewrite" ".a8-witness-probe-content")
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "mutating"
    led rows "rows=$rows" "template=none (live launch; modes are about the launch path)"
    t_sandbox "a8${cid//[^a-z0-9]/}" "fake:worker"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    t_launch "ta8${cid//[^a-z0-9]/}" || { led LAUNCH-FAILED; return 1; }
    "$stage"
    { echo "arm=$ARMG case=$cid"
      echo "rows=$rows"
      echo "clone_mode=mutating"
      echo "session=$TSESSION"
      echo "tmux_socket=$SOCK"
      echo "frozen_sha=$FROZEN_SHA"
      echo "frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "mutating_arm=yes — the manifest diff here is the MEASUREMENT, not a read-only proof"
      echo "harness_touched_declared=${touched[*]:-<none>}"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    led harness-touched-DECLARED "paths=${touched[*]:-<none>}" \
        "note=declared BEFORE the run; a statement about the controller's own actions"
    case_env_record "$AE_HOME" "$SOCK"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$SOCK" "$TSESSION" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }
    printf 'witness control probe (byte-identical rewrite) for %s\n' "$cid" >"$AE_HOME/.a8-witness-probe-rewrite"
    printf 'witness control probe (content) for %s seq=0\n' "$cid" >"$AE_HOME/.a8-witness-probe-content"
    led witness-control-PLANTED "paths=.a8-witness-probe-rewrite .a8-witness-probe-content" \
        "note=planted BEFORE the before-snapshot so both are pre-existing paths"
    dir_manifest "$AE_HOME" >"$ACAP/manifest.before.tsv"
    dir_witness "$AE_HOME" >"$ACAP/witness.before.tsv"
    { echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}|#{session_attached}' 2>&1
      echo "## panes"; tm list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}' 2>&1
      echo "## clients"; tm list-clients -F '#{client_name}|#{client_session}' 2>&1
    } >"$ACAP/tmux.before.txt"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")" \
        "witness_sha256=$(sha "$ACAP/witness.before.tsv")" "tmux_sha256=$(sha "$ACAP/tmux.before.txt")"
    "$mutate"
    # Fired AFTER the measured invocation, so a control cannot perturb what it checks.
    # The temp lives OUTSIDE the witnessed dir; the rewrite is temp+mv, the shape ae
    # itself publishes with.
    cat "$AE_HOME/.a8-witness-probe-rewrite" >"$ROOT/probe.tmp" \
        && mv "$ROOT/probe.tmp" "$AE_HOME/.a8-witness-probe-rewrite"
    printf 'witness control probe (content) for %s seq=1\n' "$cid" >"$AE_HOME/.a8-witness-probe-content"
    led witness-control-FIRED "rewrite=byte-identical temp+mv" "content=one line changed"
    dir_manifest "$AE_HOME" >"$ACAP/manifest.after.tsv"
    dir_witness "$AE_HOME" >"$ACAP/witness.after.tsv"
    { echo "## sessions"; tm list-sessions -F '#{session_name}|#{session_windows}|#{session_attached}' 2>&1
      echo "## panes"; tm list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}' 2>&1
      echo "## clients"; tm list-clients -F '#{client_name}|#{client_session}' 2>&1
    } >"$ACAP/tmux.after.txt"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")" \
        "witness_sha256=$(sha "$ACAP/witness.after.tsv")" "tmux_sha256=$(sha "$ACAP/tmux.after.txt")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    diff "$ACAP/tmux.before.txt" "$ACAP/tmux.after.txt" >"$ACAP/tmux.diff.txt" 2>&1
    python3 "$SCRATCH/harness/change-record.py" "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" \
        "$ACAP/change-record.txt" ${touched[@]+"${touched[@]}"} >"$ACAP/change-counts.txt"
    led change-record "$(cat "$ACAP/change-counts.txt")" "artifact_sha256=$(sha "$ACAP/change-record.txt")"
    # the WRITE witness: which paths were rewritten, byte-identically or not
    diff "$ACAP/witness.before.tsv" "$ACAP/witness.after.tsv" >"$ACAP/witness.diff.txt" 2>&1
    python3 "$SCRATCH/harness/write-witness.py" "$ACAP" ${touched[@]+"${touched[@]}"}
    led write-witness "$(grep '^rewritten_paths=' "$ACAP/write-witness.txt")" \
        "artifact_sha256=$(sha "$ACAP/write-witness.txt")"
    grep -q '^witness_control_rewrite_seen=yes$' "$ACAP/write-witness.txt" \
        || led HARNESS-ABORT "reason=the write witness did not see the controller's own byte-identical rewrite in this case"
    grep -q '^  \[harness\] ./.a8-witness-probe-content$' "$ACAP/change-record.txt" \
        || led HARNESS-ABORT "reason=the change record did not see the controller's own content change in this case"
    { echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
      echo "tmux_diff_lines=$(wc -l <"$ACAP/tmux.diff.txt" | tr -d ' ')"
      cat "$ACAP/change-counts.txt"
      grep -E '^rewritten_paths=|^added_paths=' "$ACAP/write-witness.txt"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    t_teardown
    echo "  $cid: $(cat "$ACAP/change-counts.txt")"
}

stage_running() { led stage "the session is left RUNNING after launch"; }
stage_stopped() {
    "$HARNESS_BASH" "$FROZEN_AE" stop "$TSESSION" </dev/null >"$ACAP/stage-stop.out" 2>"$ACAP/stage-stop.err"
    led stage "ae stop was run, so the session is STOPPED on disk before the measured invocation" "rc=$?"
}

# SC-101 — invoke `ae <name>` while the session is already RUNNING (the fast path)
mut_fastpath() {
    led measured-invocation "argv=ae $TSESSION" "context=session already running"
    run_consumer "ae-name-running" "$AE_HOME" "$SOCK" --bounded 20 -- "$HARNESS_BASH" "$FROZEN_AE" "$TSESSION"
}
# SC-102a — invoke `ae <name>` on a STOPPED session (resume, helpers regenerate)
mut_resume() {
    led measured-invocation "argv=ae $TSESSION" "context=session stopped on disk"
    run_consumer "ae-name-stopped" "$AE_HOME" "$SOCK" --bounded 40 -- "$HARNESS_BASH" "$FROZEN_AE" "$TSESSION"
}
# SC-102b — invoke `ae <name>` from INSIDE the session, on the live server
mut_inside() {
    # The agent panes run the fake tool, which CONSUMES stdin and never executes it — the
    # first run of this case send-keys'd the script into that pane and waited 40s for an rc
    # that could not arrive. `ae <name>` from inside a session needs a real shell pane, and
    # a faked $TMUX would not do: _ae_inside_tmux (ae@72c7293:257) asks the inner server
    # whether this tty is one of its panes, so the pane has to be real. The controller
    # therefore opens one window in the live session; that topology change is the
    # controller's own and is declared here.
    local runsh="$ROOT/inside.sh"
    cat >"$runsh" <<RUN
#!/opt/homebrew/bin/bash
export HOME="$ROOT/home" AE_HOME="$AE_HOME" TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
export PATH=/tmp/aecx/shim-tmux:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export AE_TMUX_SERVER="$SOCK" AE_TMUX_SERVER_KIND=socket AE_REAL_TMUX=/opt/homebrew/bin/tmux
export AE_TMUX_SHIM_LOG="$ACAP/out/inside.tmuxtrace"
env | LC_ALL=C sort | grep -E '^(TMUX|TMUX_PANE|AE_TMUX_SERVER|AE_TMUX_SERVER_KIND|PATH)=' >"$ACAP/out/inside.env"
"$HARNESS_BASH" "$FROZEN_AE" "$TSESSION" >"$ACAP/out/inside.stdout" 2>"$ACAP/out/inside.stderr"
echo \$? >"$ROOT/inside.rc"
sleep 600
RUN
    chmod +x "$runsh"; : >"$ACAP/out/inside.tmuxtrace"; rm -f "$ROOT/inside.rc"
    led controller-manipulation "action=new-window -d -n a8probe in the live session" \
        "reason=the consumer needs a REAL pane inside the session; the agent panes run the fake tool and consume stdin"
    # `-t <name>` is a target WINDOW, so it resolved to the session's current window and
    # tmux answered "index 0 in use". The trailing colon makes it a target SESSION and
    # the window lands on the next free index.
    tm new-window -d -t "$TSESSION:" -n a8probe "$HARNESS_BASH $runsh"
    local pane
    pane="$(tm list-panes -a -F '#{window_name}|#{pane_id}' 2>/dev/null | grep '^a8probe|' | head -1 | cut -d'|' -f2)"
    led measured-invocation "argv=ae $TSESSION" \
        "context=from a real shell pane INSIDE the session on the live server (window a8probe)" \
        "pane=${pane:-unknown}"
    local t0; t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 40 )); do [[ -e "$ROOT/inside.rc" ]] && break; sleep 0.5; done
    local rc; rc="$(cat "$ROOT/inside.rc" 2>/dev/null || echo '?')"
    [[ "$rc" == '?' ]] && led OUTCOME-INCONCLUSIVE "reason=the inside-session invocation did not finish within 40s"
    printf 'inside-session\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t-\t%s\n' "$rc" \
        "$(sha "$ACAP/out/inside.stdout")" "$(stat -f %z "$ACAP/out/inside.stdout" 2>/dev/null || echo 0)" \
        "$(sha "$ACAP/out/inside.stderr")" "$(stat -f %z "$ACAP/out/inside.stderr" 2>/dev/null || echo 0)" \
        "$(sha "$ACAP/out/inside.tmuxtrace")" "$(wc -l <"$ACAP/out/inside.tmuxtrace" | tr -d ' ')" \
        "ae $TSESSION from inside the session" >>"$ACAP/consumers.tsv"
    led inside-invocation-COMPLETE "rc=$rc" "stdout_sha256=$(sha "$ACAP/out/inside.stdout")" \
        "env_sha256=$(sha "$ACAP/out/inside.env")"
    tm capture-pane -p -J -S - -E - -t "${pane:-@0}" >"$ACAP/inside-pane.txt" 2>&1
    led inside-pane-capture "artifact_sha256=$(sha "$ACAP/inside-pane.txt")"
}
# SC-018b — invoke `ae --local <name>` against an EXISTING session dir
mut_use_existing() {
    led measured-invocation "argv=ae --local $TSESSION" "context=an existing session dir of that name is present"
    run_consumer "ae-local-existing" "$AE_HOME" "$SOCK" --bounded 30 -- "$HARNESS_BASH" "$FROZEN_AE" --local "$TSESSION"
}
reg() { printf '%s\t%s\tlive\tno-template (live launch)\n' "$1" "$2" >>"$ADEST/$ARMG/ledger.tsv"; }
reg a8-c01-fastpath-running "SC-101";  a8_case a8-c01-fastpath-running "SC-101"  stage_running mut_fastpath
reg a8-c02-resume-stopped   "SC-102a"; a8_case a8-c02-resume-stopped   "SC-102a" stage_stopped mut_resume
reg a8-c03-inside-session   "SC-102b"; a8_case a8-c03-inside-session   "SC-102b" stage_running mut_inside
reg a8-c04-use-existing     "SC-018b"; a8_case a8-c04-use-existing     "SC-018b" stage_running mut_use_existing
echo "A8 DONE"
