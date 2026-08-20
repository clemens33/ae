#!/opt/homebrew/bin/bash
# D03 / SC-1306e — events-tail follow semantics. b0-design.md Design 4.
# No hook: the REAL generated events helper is started in a pane and the LAUNCH BARRIER is
# a POSITIVE pane observation — the helper banner plus the final baseline record rendered,
# bounded poll, timeout recorded INCONCLUSIVE. Every controller write is confirmed by a
# file-size stat barrier, never by a sleep.
source "$(dirname "$0")/dlib.sh"
ARMG=D
mkdir -p "$ADEST/$ARMG"
[[ -f "$ADEST/$ARMG/ledger.tsv" ]] || printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

EV=""; SOCK=""; PANE=""; AEH=""
size_of() { stat -f %z "$1" 2>/dev/null || echo -1; }
inode_of() { stat -f %i "$1" 2>/dev/null || echo -1; }

stat_barrier() { # <file> <expected-min-size> <label>
    local f="$1" want="$2" lbl="$3" t0; t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < 30 )); do
        (( $(size_of "$f") >= want )) && { led stat-barrier "label=$lbl" "file_size=$(size_of "$f")" "reached=1"; return 0; }
        sleep 0.2
    done
    led stat-barrier "label=$lbl" "file_size=$(size_of "$f")" "reached=0" "OUTCOME=INCONCLUSIVE"
    return 3
}
pane_capture() { # <label>
    command tmux -S "$SOCK" capture-pane -p -J -S - -E - -t "$PANE" >"$ACAP/pane.$1.txt" 2>&1
    led pane-capture "label=$1" "sha256=$(sha "$ACAP/pane.$1.txt")" \
        "lines=$(wc -l <"$ACAP/pane.$1.txt" | tr -d ' ')"
}
pane_poll() { # <marker> <label> <timeout>
    local m="$1" lbl="$2" tmo="$3" t0 found=0; t0=$(/bin/date -u +%s)
    while (( $(/bin/date -u +%s) - t0 < tmo )); do
        command tmux -S "$SOCK" capture-pane -p -J -S - -E - -t "$PANE" 2>/dev/null | grep -q -- "$m" && { found=1; break; }
        sleep 0.5
    done
    led pane-poll "label=$lbl" "marker=$m" "found=$found" "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
    (( found == 1 )) || led OUTCOME-INCONCLUSIVE "reason=marker '$m' not on the pane within ${tmo}s ($lbl)"
    pane_capture "$lbl"
    return 0
}

d03_case() { # <case-id> <arm-kind: follow|twin> <arm-fn>
    local cid="$1" kind="$2" fn="$3"
    # PANE is per-case. It leaked across cases on the first run: a twin inherited the
    # previous follow case's pane id, polled a server that was already killed, and wrote
    # spurious INCONCLUSIVE lines into a case that has no pane by design.
    PANE=""; SOCK=""; EV=""; AEH=""
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$kind"
    led rows "rows=D03,SC-1306e" "template=D/d03-31-numbered-events" "design=b0-design.md Design 4"
    AEH="$base/home/.ae"
    t_clone D d03-31-numbered-events "$AEH" rw || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$AEH/sessions" | head -1)"
    EV="$AEH/sessions/$sess/events.jsonl"
    local cf exp
    cf="$(dir_fingerprint "$AEH")"; exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/D/_meta/d03-31-numbered-events.txt" | cut -d= -f2-)"
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    SOCK="$base/live.sock"
    { echo "arm=$ARMG case=$cid arm_kind=$kind design=b0-design.md Design 4 (D03) rows=D03,SC-1306e"
      echo "template=D/d03-31-numbered-events clone_mode=rw session=$sess"
      echo "clone_fingerprint=$cf clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "consumer=the session's OWN generated events-tail helper, started in a pane"
      echo "launch_barrier=positive pane observation (banner + the final baseline record), bounded poll"
      echo "write_confirmation=file-size stat barrier on every controller write; no sleep-based inference"
      echo "seeded_events=$(wc -l <"$EV" | tr -d ' ') last=$(tail -1 "$EV" | sed -n 's/.*"summary":"\([^"]*\)".*/\1/p')"
      echo "events_inode_at_start=$(inode_of "$EV") size=$(size_of "$EV")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$AEH" "$SOCK"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    d_snapshot before "$AEH" "$SOCK"
    if [[ "$kind" == follow ]]; then
        command tmux -S "$SOCK" kill-server >/dev/null 2>&1
        PANE="$(command tmux -S "$SOCK" new-session -d -s d03 -P -F '#{pane_id}' \
            "env HOME=$base/home AE_HOME=$AEH TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 $AEH/sessions/$sess/events-tail")"
        led events-tail-STARTED "pane=$PANE" "socket=$SOCK"
        local t0 ok=0; t0=$(/bin/date -u +%s)
        while (( $(/bin/date -u +%s) - t0 < 60 )); do
            command tmux -S "$SOCK" capture-pane -p -J -S - -E - -t "$PANE" 2>/dev/null | grep -q 'D03-SEED-EVENT-31' && { ok=1; break; }
            sleep 0.5
        done
        led launch-barrier "final_baseline_record_rendered=$ok" "waited_s=$(( $(/bin/date -u +%s) - t0 ))"
        (( ok == 1 )) || led OUTCOME-INCONCLUSIVE "reason=launch barrier not observed within 60s"
        pane_capture "00-launch-barrier"
    else
        led twin-note "controller-only twin: the identical mutations with NO events-tail running"
    fi
    "$fn"
    d_snapshot after "$AEH" "$SOCK"
    { echo "events_inode_at_end=$(inode_of "$EV") size=$(size_of "$EV")"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')" >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$SOCK" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid ($kind) done"
}

a_initial_window() { :; }   # arm 1 is the launch capture itself; no controller mutation
a_complete_append() {
    local pay="$AEH/_d03-payloads/append-sentinel.jsonl" before after
    before="$(size_of "$EV")"
    { echo "mutation=append ONE real harvested complete event"
      echo "payload=_d03-payloads/append-sentinel.jsonl sha256=$(sha "$pay") bytes=$(stat -f %z "$pay")"
      echo "events size before=$before"; } >"$ACAP/controller-mutation.txt"
    cat "$pay" >>"$EV"
    after="$(size_of "$EV")"
    echo "events size after=$after" >>"$ACAP/controller-mutation.txt"
    led controller-mutation "append=complete harvested event" "size_before=$before" "size_after=$after"
    stat_barrier "$EV" "$after" "complete-append"
    [[ -n "$PANE" ]] && pane_poll 'D03-APPEND-SENTINEL' "01-after-complete-append" 30
}
a_line_framing() {
    local pay="$AEH/_d03-payloads/append-sentinel.jsonl" full partial before mid final
    full="$(cat "$pay")"
    partial="${full:0:$(( ${#full} - 25 ))}"
    before="$(size_of "$EV")"
    { echo "mutation step 1=write a PARTIAL producer-derived line with NO terminating newline"
      echo "full_line_bytes=${#full} partial_bytes=${#partial} (the last 25 bytes withheld)"
      echo "events size before=$before"; } >"$ACAP/controller-mutation.txt"
    printf '%s' "$partial" >>"$EV"
    mid="$(size_of "$EV")"
    echo "events size after the partial write=$mid" >>"$ACAP/controller-mutation.txt"
    led controller-mutation "step=partial-line-no-newline" "size_before=$before" "size_after=$mid"
    stat_barrier "$EV" "$mid" "partial-line"
    [[ -n "$PANE" ]] && { sleep 3; pane_capture "01-after-partial-line"; }
    { echo "mutation step 2=write the withheld remainder AND the terminating newline"; } >>"$ACAP/controller-mutation.txt"
    printf '%s\n' "${full: -25}" >>"$EV"
    final="$(size_of "$EV")"
    echo "events size after the terminating write=$final" >>"$ACAP/controller-mutation.txt"
    led controller-mutation "step=terminating-newline" "size_after=$final"
    stat_barrier "$EV" "$final" "terminated-line"
    [[ -n "$PANE" ]] && pane_poll 'D03-APPEND-SENTINEL' "02-after-terminating-newline" 30
}
a_rotation() {
    local link="$AEH/sessions/$(ls "$AEH/sessions"|head -1)/events.jsonl.oldinode"
    local newp="$AEH/_d03-payloads/rotate-newpath.jsonl" oldp="$AEH/_d03-payloads/rotate-oldinode.jsonl"
    local i0 i1 s0
    i0="$(inode_of "$EV")"; s0="$(size_of "$EV")"
    ln "$EV" "$link"
    { echo "mutation=rotation"
      echo "original_inode=$i0 size=$s0"
      echo "hardlink_held_at=events.jsonl.oldinode inode=$(inode_of "$link")"; } >"$ACAP/controller-mutation.txt"
    cp "$EV" "$EV.new" && mv "$EV.new" "$EV"
    i1="$(inode_of "$EV")"
    { echo "after atomic replace: events.jsonl inode=$i1 size=$(size_of "$EV")"
      echo "old inode still reachable through the hardlink: inode=$(inode_of "$link") size=$(size_of "$link")"
      echo "inode_changed=$( [[ "$i0" != "$i1" ]] && echo yes || echo no )"; } >>"$ACAP/controller-mutation.txt"
    led controller-mutation "step=atomic-replace" "old_inode=$i0" "new_inode=$i1" \
        "inode_changed=$( [[ "$i0" != "$i1" ]] && echo yes || echo no )"
    cat "$newp" >>"$EV"; stat_barrier "$EV" "$(size_of "$EV")" "append-to-new-path"
    echo "appended ROTATE-NEWPATH to the NEW path; size=$(size_of "$EV")" >>"$ACAP/controller-mutation.txt"
    [[ -n "$PANE" ]] && pane_poll 'D03-ROTATE-NEWPATH' "01-after-append-to-new-path" 30
    cat "$oldp" >>"$link"; stat_barrier "$link" "$(size_of "$link")" "append-to-old-inode"
    echo "appended ROTATE-OLDINODE through the hardlink to the ORIGINAL inode; size=$(size_of "$link")" >>"$ACAP/controller-mutation.txt"
    [[ -n "$PANE" ]] && pane_poll 'D03-ROTATE-OLDINODE' "02-after-append-to-old-inode" 30
}
reg() { printf '%s\t%s\t%s\t%s\n' "$1" "D03,SC-1306e" "D" "d03-31-numbered-events" >>"$ADEST/$ARMG/ledger.tsv"; }
reg d03-a1-initial-window;      d03_case d03-a1-initial-window      follow a_initial_window
reg d03-a2-complete-append;     d03_case d03-a2-complete-append     follow a_complete_append
reg d03-a2-twin;                d03_case d03-a2-twin                twin   a_complete_append
reg d03-a3-line-framing;        d03_case d03-a3-line-framing        follow a_line_framing
reg d03-a3-twin;                d03_case d03-a3-twin                twin   a_line_framing
reg d03-a4-rotation;            d03_case d03-a4-rotation            follow a_rotation
reg d03-a4-twin;                d03_case d03-a4-twin                twin   a_rotation
echo "D03 DONE"
