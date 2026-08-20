#!/opt/homebrew/bin/bash
# ARM GROUP A1 — schema/document. Bash lane. Evidence is written directly into the
# committed artifact tree, with a per-case admissibility ledger establishing that both
# standing checks completed BEFORE the first consumer invocation.
source "$(dirname "$0")/armlib.sh"
ARMG=A1
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

doc_case() { # <case-id> <rows> <group> <member> <mode>
    local cid="$1" rows="$2" grp="$3" mem="$4" mode="$5"
    local base="$AROOT/$ARMG/$cid-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$mode"
    led rows "rows=$rows" "template=$grp/$mem"
    local aehome="$base/home/.ae"
    t_clone "$grp" "$mem" "$aehome" "$mode" || { led CLONE-FAILED "template=$grp/$mem"; return 1; }
    local sess; sess="$(ls "$aehome/sessions" 2>/dev/null | head -1)"
    local sock="$base/none.sock"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" \
        "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    { echo "arm=$ARMG case=$cid rows=$rows template=$grp/$mem clone_mode=$mode session=$sess"
      echo "template_fingerprint_pre_protection=$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      echo "template_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "tmux_socket=$sock (no server started for this case)"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$ACAP/case.txt"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=environment tab self-check failed; no capture taken"
        echo "  HARNESS-ABORT $cid-$mode: tab self-check"; return 1; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    command tmux -S "$sock" list-sessions >"$ACAP/tmux.before.txt" 2>&1
    local B="$HARNESS_BASH" AE="$FROZEN_AE"
    run_consumer "list"          "$aehome" "$sock" -- "$B" "$AE" list
    run_consumer "list-json"     "$aehome" "$sock" -- "$B" "$AE" list --json
    run_consumer "list-all"      "$aehome" "$sock" -- "$B" "$AE" list --all
    run_consumer "list-all-json" "$aehome" "$sock" -- "$B" "$AE" list --all --json
    run_consumer "ls-alias"      "$aehome" "$sock" -- "$B" "$AE" ls
    run_consumer "ls-alias-all"  "$aehome" "$sock" -- "$B" "$AE" ls --all
    run_consumer "status"        "$aehome" "$sock" -- "$B" "$AE" status "$sess"
    run_consumer "next"          "$aehome" "$sock" -- "$B" "$AE" next
    if [[ -x "$aehome/sessions/$sess/requests" ]]; then
        run_consumer "requests-all" "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" all
        run_consumer "agents"       "$aehome" "$sock" -- "$aehome/sessions/$sess/agents"
        run_consumer "events-tail"  "$aehome" "$sock" --bounded 4 -- "$aehome/sessions/$sess/events-tail"
    fi
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    command tmux -S "$sock" list-sessions >"$ACAP/tmux.after.txt" 2>&1
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "manifest_before_sha256=$(sha "$ACAP/manifest.before.tsv")"
      echo "manifest_after_sha256=$(sha "$ACAP/manifest.after.tsv")"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$ACAP/tmux.before.txt")" == "$(cat "$ACAP/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led manifest-diff "lines=$dl"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    echo "  case $cid ($grp/$mem, $mode): manifest_diff_lines=$dl"
}

C() { # <case-id> <rows> <group> <member>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"
    echo "== case $1 ($2) <- $3/$4"
    doc_case "$1" "$2" "$3" "$4" ro
    doc_case "$1" "$2" "$3" "$4" rw
}
C c01-healthy              "SC-509,SC-509b,SC-510a"   G1  healthy
C c02-meta-mode-000        "SC-506"                   G3  meta-mode-000
C c03-malformed-line       "SC-506,SC-509b"           G3  malformed-complete-line
C c04-empty-vs-omitted     "SC-510b"                  A1  510b-empty-vs-omitted
C c05-recover-ref          "SC-510c"                  A1  510c-recover-ref
C c06-escapes              "SC-510d"                  G11 escapes
C c07-dupkey-known         "SC-510e"                  A1  510e-dupkey-known
C c08-dupkey-known-rev     "SC-510e"                  A1  510e-dupkey-known-reversed
C c09-dupkey-unknown       "SC-510f"                  A1  510f-dupkey-unknown
C c10-dupkey-unknown-rev   "SC-510f"                  A1  510f-dupkey-unknown-reversed
C c11-routing-known        "SC-511a,SC-511b"          G5  m1-control
C c12-routing-omitted      "SC-511a"                  A1  511a-omitted-routing
C c13-same-display-routing "SC-511b"                  G10 same-display-diff-routing
C c14-display-only-legacy  "SC-511b"                  G10 display-only-legacy
C c15-meta-unknown-keys    "SC-509,SC-509b"           G7  meta-unknown-keys
C c16-events-unknown-keys  "SC-509b,SC-510a"          G7  events-unknown-keys
C c17-unknown-action       "SC-509b,SC-510a"          G7  events-unknown-action
C c18-no-trailing-newline  "SC-509b"                  G8  no-trailing-newline
C c19-partial-tail         "SC-509b"                  G8  partial-trailing-record
echo "A1 DOCUMENT CASES DONE"
