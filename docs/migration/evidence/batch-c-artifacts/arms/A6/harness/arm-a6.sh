#!/opt/homebrew/bin/bash
# ARM GROUP A6 — requests / pairing. Rows: SC-518, SC-522, SC-523a, SC-523b.
#
# SC-518: the six G5 request-pair members, one consumer run each.
# SC-522 / SC-523a-b: the unanswered threshold, on ONE fixture whose ask ts is a known
# epoch (1755000000), read at SEVERAL frozen nows. Equality and strictly-past are separate
# inputs (age exactly 1800, and 1799 / 1801 either side), and the arm first establishes
# that the sensor RESPONDS AT ALL on this fixture — a far-below and a far-above reading
# that agree would mean the fixture cannot discriminate anything and the boundary triple
# would be meaningless. That control is recorded, not assumed.
source "$(dirname "$0")/armlib.sh"
ARMG=A6
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
ASK_EPOCH=1755000000

a6_pair_case() { # <case-id> <member> <mode>
    local cid="$1" mem="$2" mode="$3"
    local base="$AROOT/$ARMG/$cid-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$mode"
    led rows "rows=SC-518" "template=G5/$mem"
    local aehome="$base/home/.ae"
    t_clone G5 "$mem" "$aehome" "$mode" || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$aehome/sessions" | head -1)"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/G5/_meta/$mem.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/G5/_meta/$mem.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    build_live_topology "$aehome" "$sock" "$sess"
    { echo "arm=$ARMG case=$cid rows=SC-518 template=G5/$mem clone_mode=$mode session=$sess"
      echo "clone_fingerprint=$cf clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "mutation_recorded_at=templates/G5/_meta/$mem.mutation.txt"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    cp "$TSTORE/G5/_meta/$mem.mutation.txt" "$ACAP/member.mutation.txt" 2>/dev/null
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    { echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}' 2>&1
      echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}' 2>&1
    } >"$ACAP/tmux.before.txt"
    run_consumer "requests-all"   "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" all
    run_consumer "requests-mine"  "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" mine
    run_consumer "requests-inbox" "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" inbox
    run_consumer "list-json"      "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
    cp "$ACAP/tmux.before.txt" "$ACAP/tmux.after.txt"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')" >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid ($mode): manifest_diff_lines=$(grep '^manifest_diff_lines=' "$ACAP/case.txt" | cut -d= -f2-)"
}

a6_threshold_case() {
    local cid=a6-threshold
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "fixed-clock"
    led rows "rows=SC-522,SC-523a,SC-523b" "template=G2/unanswered"
    local aehome="$base/home/.ae"
    t_clone G2 unanswered "$aehome" rw || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$aehome/sessions" | head -1)"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"; exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/G2/_meta/unanswered.txt" | cut -d= -f2-)"
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    build_live_topology "$aehome" "$sock" "$sess"
    { echo "arm=$ARMG case=$cid rows=SC-522,SC-523a,SC-523b template=G2/unanswered clone_mode=rw session=$sess"
      echo "clone_fingerprint=$cf clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "ask_ts_epoch=$ASK_EPOCH ($(/bin/date -u -r $ASK_EPOCH +%Y-%m-%dT%H:%M:%SZ)), never replied"
      echo "documented_default_threshold=1800s (AE_ATTN_REQUEST_SECS)"
      echo "clock frozen per reading by the PATH-first date shim; every non-now-form still delegates"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    { echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}' 2>&1
      echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}' 2>&1
    } >"$ACAP/tmux.before.txt"
    export ARM_DATE_SHIM_LOG="$ACAP/date-shim.log"; : >"$ARM_DATE_SHIM_LOG"
    # readings: two responsiveness controls, then the boundary triple
    local age now lbl
    for age in 10 1799 1800 1801 100000; do
        now=$((ASK_EPOCH + age))
        lbl="age${age}"
        led threshold-reading "age_s=$age" "frozen_now=$now"
        ARM_FAKE_NOW=$now run_consumer "${lbl}_list_json"      "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
        ARM_FAKE_NOW=$now run_consumer "${lbl}_list_needsattn" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --needs-attn
        ARM_FAKE_NOW=$now run_consumer "${lbl}_next"           "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" next
    done
    # scrubbed-env default vs explicit override vs malformed override, all at the SAME age
    now=$((ASK_EPOCH + 1000))
    for v in unset 500 900x; do
        lbl="env_${v}"
        led threshold-env-reading "AE_ATTN_REQUEST_SECS=$v" "age_s=1000" "frozen_now=$now"
        if [[ "$v" == unset ]]; then
            ARM_FAKE_NOW=$now run_consumer "${lbl}_list_json" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
        else
            ARM_FAKE_NOW=$now ARM_EXTRA_ENV="AE_ATTN_REQUEST_SECS=$v" \
                run_consumer "${lbl}_list_json" "$aehome" "$sock" -- "$HARNESS_BASH" "$FROZEN_AE" list --json
        fi
    done
    unset ARM_DATE_SHIM_LOG
    # The discrimination record is built by a TESTED helper, not an ad-hoc pattern. The
    # first version used sed BRE alternation, a GNU extension that matches NOTHING on BSD,
    # so every reading came back empty and the record claimed the sensor was unresponsive.
    python3 "$SCRATCH/harness/derive-discrimination.py" "$ACAP" "$ASK_EPOCH" >/dev/null
    local responsive; responsive="$(grep '^responsive=' "$ACAP/discrimination.txt" | cut -d= -f2)"
    led discrimination-record "responsive=$responsive" "artifact_sha256=$(sha "$ACAP/discrimination.txt")"
    [[ "$responsive" == yes ]] || led OUTCOME-INCONCLUSIVE "reason=the sensor does not respond to age on this fixture; the boundary triple cannot discriminate"
    cp "$ACAP/tmux.before.txt" "$ACAP/tmux.after.txt"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')" >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid: responsive=$( [[ "$a" != "$b" ]] && echo yes || echo no )"
}
reg() { printf '%s\t%s\tG5\t%s\n' "$1" "$2" "$3" >>"$ADEST/$ARMG/ledger.tsv"; }
i=0
for m in m1-control m2-wrong-ref m3-wrong-actor m4-wrong-target m5-routed-vs-routed-mismatch m6-mixed-routed-display; do
    i=$((i+1)); cid="$(printf 'a6-c%02d-%s' "$i" "$m")"
    reg "$cid" "SC-518" "$m"
    a6_pair_case "$cid" "$m" ro
    a6_pair_case "$cid" "$m" rw
done
printf 'a6-threshold\tSC-522,SC-523a,SC-523b\tG2\tunanswered\n' >>"$ADEST/$ARMG/ledger.tsv"
a6_threshold_case
echo "A6 DONE"
