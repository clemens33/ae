#!/opt/homebrew/bin/bash
# ARM GROUP A3 — rollup / severity. Rows: SC-017g, SC-017h, SC-524.
# Every case runs on a LIVE topology (the rollup reads panes as well as events).
source "$(dirname "$0")/armlib.sh"
ARMG=A3
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

a3_case() { # <case-id> <rows> <group> <member> <mode>
    local cid="$1" rows="$2" grp="$3" mem="$4" mode="$5"
    local base="$AROOT/$ARMG/$cid-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$mode"
    led rows "rows=$rows" "template=$grp/$mem"
    local aehome="$base/home/.ae"
    t_clone "$grp" "$mem" "$aehome" "$mode" || { led CLONE-FAILED; return 1; }
    local sess; sess="$(ls "$aehome/sessions" | head -1)"
    local cf exp
    cf="$(dir_fingerprint "$aehome")"
    if [[ "$mode" == ro ]]; then exp="$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
    else exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"; fi
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    local sock="$base/live.sock"
    build_live_topology "$aehome" "$sock" "$sess"
    { echo "arm=$ARMG case=$cid rows=$rows template=$grp/$mem clone_mode=$mode session=$sess"
      echo "template_fingerprint_pre_protection=$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      echo "template_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "live_topology=one pane per roster entry, the fixture's controllable fake binary, never a live model"
      echo "tmux_socket=$sock"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$ACAP/case.txt"
    { echo "## the two candidate activity sources, as the consumer sees them"
      local f="$aehome/sessions/$sess/events.jsonl"
      if [[ -f "$f" ]]; then
        printf 'events.jsonl mtime_epoch=%s utc=%s\n' "$(stat -f %m "$f")" "$(/bin/date -u -r "$(stat -f %m "$f")" +%Y-%m-%dT%H:%M:%SZ)"
        printf 'last_event_ts=%s\n' "$(tail -1 "$f" | sed -n 's/.*"ts":"\([^"]*\)".*/\1/p')"
        printf 'events_sha256=%s bytes=%s lines=%s\n' "$(sha "$f")" "$(stat -f %z "$f")" "$(wc -l <"$f" | tr -d ' ')"
      else echo "events.jsonl ABSENT"; fi
      printf 'harness_now_epoch=%s utc=%s\n' "$(/bin/date -u +%s)" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$ACAP/activity-sources.txt"
    led activity-sources "artifact_sha256=$(sha "$ACAP/activity-sources.txt")"
    case_env_record "$aehome" "$sock"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=environment tab self-check failed"; return 1; }
    tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence failed"; return 1; }
    { echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.before.txt" 2>&1
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    local B="$HARNESS_BASH" AE="$FROZEN_AE"
    run_consumer "list"           "$aehome" "$sock" -- "$B" "$AE" list
    run_consumer "list-json"      "$aehome" "$sock" -- "$B" "$AE" list --json
    run_consumer "list-all-json"  "$aehome" "$sock" -- "$B" "$AE" list --all --json
    run_consumer "list-needsattn" "$aehome" "$sock" -- "$B" "$AE" list --needs-attn
    run_consumer "list-needsattn-json" "$aehome" "$sock" -- "$B" "$AE" list --needs-attn --json
    run_consumer "list-active"    "$aehome" "$sock" -- "$B" "$AE" list --active
    run_consumer "list-active-json" "$aehome" "$sock" -- "$B" "$AE" list --active --json
    run_consumer "next"           "$aehome" "$sock" -- "$B" "$AE" next
    run_consumer "status"         "$aehome" "$sock" -- "$B" "$AE" status "$sess"
    run_consumer "requests-all"   "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" all
    # the two fields the row asks for, lifted out of the captured JSON bytes verbatim
    { echo "## attention fields, extracted from out/list-json.stdout"
      sed -n 's/.*\("needs_attention":[^,]*\).*/\1/p' "$ACAP/out/list-json.stdout"
      sed -n 's/.*\("attention":"[^"]*"\).*/\1/p' "$ACAP/out/list-json.stdout"
      sed -n 's/.*\("attention_rank":[0-9]*\).*/\1/p' "$ACAP/out/list-json.stdout"
      echo "## per-agent reasons"
      tr ',' '\n' <"$ACAP/out/list-json.stdout" | grep -E '"ref"|"reason"|"state"|"alive"' || true
    } >"$ACAP/attention-fields.txt"
    led attention-fields "artifact_sha256=$(sha "$ACAP/attention-fields.txt")"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    { echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}'
      echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}|#{session_windows}'
    } >"$ACAP/tmux.after.txt" 2>&1
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$ACAP/tmux.before.txt")" == "$(cat "$ACAP/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led manifest-diff "lines=$dl"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  case $cid ($grp/$mem, $mode): manifest_diff_lines=$dl"
}
C() {
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"
    echo "== case $1 ($2) <- $3/$4"
    a3_case "$1" "$2" "$3" "$4" ro
    a3_case "$1" "$2" "$3" "$4" rw
}
C c01-dead          "SC-017g,SC-017h" G2  dead
C c02-stale         "SC-017g,SC-017h" G2  stale
C c03-throttled     "SC-017g,SC-017h" G2  throttled
C c04-waiting-user  "SC-017g,SC-017h" G2  waiting-user
C c05-blocked       "SC-017g,SC-017h" G2  blocked
C c06-unanswered    "SC-017g,SC-017h" G2  unanswered
C c07-competing     "SC-017g,SC-017h" G2b competing
C c08-unanswered-vs-agent-owned "SC-017g" A3 017g-unanswered-vs-agent-owned
C c09-524a-future-ts-ordinary-mtime "SC-524" A3 524a-future-ts-ordinary-mtime
C c10-524b-ordinary-ts-future-mtime "SC-524" A3 524b-ordinary-ts-future-mtime
echo "A3 DONE"
