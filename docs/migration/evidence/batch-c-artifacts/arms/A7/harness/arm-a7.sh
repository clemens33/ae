#!/opt/homebrew/bin/bash
# ARM GROUP A7 — meta grammar. Rows: SC-405a..g, SC-405j.
source "$(dirname "$0")/armlib.sh"
ARMG=A7
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

a7_case() { # <case-id> <rows> <group> <member> <mode> <live: yes|no>
    local cid="$1" rows="$2" grp="$3" mem="$4" mode="$5" live="$6"
    local base="$AROOT/$ARMG/$cid-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$mode"
    led rows "rows=$rows" "template=$grp/$mem" "live_topology=$live"
    local aehome="$base/home/.ae"
    t_clone "$grp" "$mem" "$aehome" "$mode" || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$aehome/sessions" | head -1)"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    if [[ "$live" == yes ]]; then
        build_live_topology "$aehome" "$sock" "$sess"
        if [[ "$mem" == branch-two-sources ]]; then
            command tmux -S "$sock" set-option -t "$sess" @ae_branch_name "TMUX-OPTION-BRANCH-VALUE"
            led branch-source-set "tmux_@ae_branch_name=TMUX-OPTION-BRANCH-VALUE" \
                "git_branch=$(cd "$(grep '^work_dir=' "$aehome/sessions/$sess/meta" | cut -d= -f2-)" 2>/dev/null && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '<unreadable>')"
        fi
    else
        led no-live-topology "reason=this subarm reads the same fixture with no tmux server at all"
    fi
    { echo "arm=$ARMG case=$cid"
      echo "rows=$rows"
      echo "template=$grp/$mem"
      echo "clone_mode=$mode"
      echo "session=$sess"
      echo "live_topology=$live"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "frozen_sha=$FROZEN_SHA"
      echo "frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    [[ -f "$TSTORE/$grp/_meta/$mem.mutation.txt" ]] && cp "$TSTORE/$grp/_meta/$mem.mutation.txt" "$ACAP/member.mutation.txt"
    # the meta bytes the consumer will read, verbatim
    cp "$aehome/sessions/$sess/meta" "$ACAP/meta.bytes.txt"
    cp "$aehome/sessions/$sess/events.jsonl" "$ACAP/events.bytes.jsonl" 2>/dev/null || true
    led fixture-bytes "meta_sha256=$(sha "$ACAP/meta.bytes.txt")" \
        "events_sha256=$(sha "$ACAP/events.bytes.jsonl")"
    case_env_record "$aehome" "$( [[ "$live" == yes ]] && echo "$sock" )"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    [[ "$live" == yes ]] && { tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    { echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}' 2>&1
      echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}' 2>&1
      echo "## @ae_branch_name"; command tmux -S "$sock" show-options -v -t "$sess" @ae_branch_name 2>&1
    } >"$ACAP/tmux.before.txt"
    local B="$HARNESS_BASH" AE="$FROZEN_AE" S4="$( [[ "$live" == yes ]] && echo "$sock" )"
    run_consumer "list"          "$aehome" "$S4" -- "$B" "$AE" list
    run_consumer "list-json"     "$aehome" "$S4" -- "$B" "$AE" list --json
    run_consumer "list-all"      "$aehome" "$S4" -- "$B" "$AE" list --all
    run_consumer "list-all-json" "$aehome" "$S4" -- "$B" "$AE" list --all --json
    run_consumer "status"        "$aehome" "$S4" -- "$B" "$AE" status "$sess"
    run_consumer "next"          "$aehome" "$S4" -- "$B" "$AE" next
    [[ -x "$aehome/sessions/$sess/requests" ]] && run_consumer "requests-all" "$aehome" "$S4" -- "$aehome/sessions/$sess/requests" all
    cp "$ACAP/tmux.before.txt" "$ACAP/tmux.after.txt"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')" >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid ($mode/live=$live) done"
}
C() { # <case-id> <rows> <group> <member> <live>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"
    a7_case "$1" "$2" "$3" "$4" ro "$5"
    a7_case "$1" "$2" "$3" "$4" rw "$5"
}
C a7-c01-meta-multi-equals   "SC-405a"          A7 meta-multi-equals        yes
C a7-c02-meta-duplicate-key  "SC-405a,SC-405d"  A7 meta-duplicate-key       yes
C a7-c03-meta-unknown-keys   "SC-405b,SC-405c"  G7 meta-unknown-keys        yes
C a7-c04-healthy-baseline    "SC-405b,SC-405c"  G1 healthy                  yes
C a7-c05-meta-mode-000       "SC-405e"          G3 meta-mode-000            yes
C a7-c06-goal-order-opposed  "SC-405f"          A7 goal-order-opposed       yes
C a7-c07-goal-order-agreeing "SC-405f"          A7 goal-order-agreeing      yes
C a7-c08-goal-order-single   "SC-405f"          A7 goal-order-single        yes
C a7-c09-goals-distinct-ts   "SC-405f"          G9 goals-distinct-ts        yes
C a7-c10-branch-live         "SC-405g"          A7 branch-two-sources       yes
C a7-c11-branch-stopped      "SC-405g"          A7 branch-two-sources       no
C a7-c12-405j-full-fresh     "SC-405j"          A7 405j-full-fresh          yes
C a7-c13-405j-stale-keys     "SC-405j"          A7 405j-stale-mismatched-keys yes
C a7-c14-405j-slot-only      "SC-405j"          A7 405j-slot-only           yes
C a7-c15-405j-session-only   "SC-405j"          A7 405j-session-only        yes
C a7-c16-405j-keyless        "SC-405j"          A7 405j-keyless-legacy      yes
C a7-c17-405j-one-empty      "SC-405j"          A7 405j-one-empty-member    yes
C a7-c18-405j-all-empty      "SC-405j"          A7 405j-all-empty-members   yes
echo "A7 DONE"
