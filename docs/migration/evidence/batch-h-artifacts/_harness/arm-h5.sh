#!/opt/homebrew/bin/bash
# A-H5 — SC-211o, codex identity registration through `_register-sid`.
#
# The surface takes a SLOT (ae@72c7293:14752), reads `launch_id.<slot>` and
# `launch_time.<slot>` from meta, scans today's and yesterday's Codex session directories,
# and writes what it selects to `codex.<slot>.sid`. Each case varies ONE fixture fact and
# captures the artifact; nothing here says which candidate should win.
#
# CONSTRUCTED INPUTS, declared: the candidate `.jsonl` files and the `launch_id`/
# `launch_time` meta lines are written by the CONTROLLER. There is no offline producer for
# a Codex session file, and this batch runs with no live models and no network. They are
# INPUT DATA the surface reads, not helper bytes — every helper byte still comes from a real
# frozen launch — and each case records the exact bytes it planted.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
ARM=A-H5
mkdir -p "$ADEST/$ARM"

h_sandbox h6 "cl:lead" "" || exit 1
h_launch th6 || { echo "launch failed"; exit 1; }
H_META="$HMETA"; H_SOCK="$SOCK"; H_SRV="$HSRV_PID"; H_WORK="$ROOT/work"
TODAY="$(/bin/date -u +%Y/%m/%d)"
YESTERDAY="$(/bin/date -u -v-1d +%Y/%m/%d)"
CODEX="$HOME/.codex/sessions"

plant() { # <day> <name> <mtime-epoch> <first-line>
    mkdir -p "$CODEX/$1"
    printf '%s\n' "$4" >"$CODEX/$1/$2.jsonl"
    /usr/bin/touch -t "$(/bin/date -u -r "$3" +%Y%m%d%H%M.%S)" "$CODEX/$1/$2.jsonl"
    printf '  planted %s/%s.jsonl mtime=%s sha256=%s\n' "$1" "$2" "$3" \
        "$(sha "$CODEX/$1/$2.jsonl")" >>"$CASE_DIR/planted-inputs.txt"
    printf '    first_line=%s\n' "$4" >>"$CASE_DIR/planted-inputs.txt"
}

meta_set() { # <key> <value>   — a NAMED controller mutation of the session's meta
    printf '%s=%s\n' "$1" "$2" >>"$H_META/meta"
    printf '  meta line appended: %s=%s\n' "$1" "$2" >>"$CASE_DIR/planted-inputs.txt"
}

reset_fixture() {
    rm -rf "$CODEX"; mkdir -p "$CODEX"
    grep -v '^launch_id\.\|^launch_time\.\|^codex_launch_id\.\|^codex_launch_time\.' \
        "$H_META/meta" >"$H_META/meta.tmp" && mv "$H_META/meta.tmp" "$H_META/meta"
    rm -f "$H_META"/codex.*.sid
}

run_case() { # <case-id> <slot-argv-or-EMPTY> <setup-fn> <note>
    local cid="$1" slot="$2" setup="$3" note="$4"
    case_open "$ARM" "$cid"
    led rows "rows=SC-211o" "surface=_register-sid" "fixture=H6 cohorts"
    : >"$CASE_DIR/planted-inputs.txt"
    reset_fixture
    "$setup"
    cp "$H_META/meta" "$CASE_DIR/meta.before.txt"
    surface_state "$H_META/_register-sid" "$H_META/meta"
    led planted "artifact_sha256=$(sha "$CASE_DIR/planted-inputs.txt")" \
        "note=candidate files and meta keys are CONTROLLER-constructed input data, listed with their bytes"
    led measured-input "slot_argv=${slot:-<none>}" "note=$note"
    if [[ -n "$slot" ]]; then
        measured "$cid" "register" 25 -- env TMUX="${H_SOCK},${H_SRV},0" \
            TMUX_PANE="$(h_pane_of cl:lead)" "$H_META/_register-sid" "$slot"
    else
        measured "$cid" "register" 25 -- env TMUX="${H_SOCK},${H_SRV},0" \
            TMUX_PANE="$(h_pane_of cl:lead)" "$H_META/_register-sid"
    fi
    cp "$H_META/meta" "$CASE_DIR/meta.after.txt"
    diff "$CASE_DIR/meta.before.txt" "$CASE_DIR/meta.after.txt" >"$CASE_DIR/meta.diff.txt" 2>&1
    { for f in "$H_META"/codex.*.sid; do
          [[ -e "$f" ]] || { echo "no codex.<slot>.sid written"; break; }
          printf '%s sha256=%s bytes=%s\n' "$(basename "$f")" "$(sha "$f")" "$(stat -f %z "$f")"
          printf '  content: %s\n' "$(cat "$f")"
      done; } >"$CASE_DIR/sid-artifact.txt"
    led sid-artifact "artifact_sha256=$(sha "$CASE_DIR/sid-artifact.txt")" \
        "meta_diff_lines=$(wc -l <"$CASE_DIR/meta.diff.txt" | tr -d ' ')"
    led case-CLOSE "invocations_sha256=$(sha "$CASE_DIR/invocations.tsv")"
    echo "  $cid done"
}

NOW=$(/bin/date -u +%s)
ID_A=aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa
ID_B=bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb
line() { printf '{"id":"%s","cwd":"%s","marker":"%s"}' "$1" "$2" "$3"; }

s_no_meta()      { plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" "$H_WORK" plain)"; }
s_token_match()  { meta_set "launch_id.main" "TOK-MATCH"; meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "{\"id\":\"$ID_A\",\"cwd\":\"$H_WORK\",\"x\":\"AE_CODEX_LAUNCH_ID=TOK-MATCH\"}"; }
s_token_wrong()  { meta_set "launch_id.main" "TOK-MATCH"; meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "{\"id\":\"$ID_A\",\"cwd\":\"$H_WORK\",\"x\":\"AE_CODEX_LAUNCH_ID=TOK-OTHER\"}"; }
s_older_than()   { meta_set "launch_time.main" "$NOW"
                   plant "$TODAY" cand-a $((NOW - 3600)) "$(line "$ID_A" "$H_WORK" old)"; }
s_nonnumeric()   { meta_set "launch_time.main" "not-a-number"
                   plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" "$H_WORK" plain)"; }
s_equal_mtime()  { meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" "$H_WORK" equal-a)"
                   plant "$TODAY" cand-b $((NOW - 60)) "$(line "$ID_B" "$H_WORK" equal-b)"; }
s_diff_mtime()   { meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 120)) "$(line "$ID_A" "$H_WORK" older)"
                   plant "$TODAY" cand-b $((NOW - 60))  "$(line "$ID_B" "$H_WORK" newer)"; }
# The cwd fallback (ae:14794-14812) runs ONLY when the token pass found nothing
# (ae:14793 `if [ -z "$best_id" ]`). Built without forcing that, both cwd cases had their
# fact never consulted and read identically — same output, different reason. A launch-id
# token that NO candidate carries makes the token pass select nothing, so the fallback is
# reached and cwd is what decides.
s_cwd_match()    { meta_set "launch_id.main" "TOK-NOBODY-CARRIES-THIS"
                   meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" "$H_WORK" cwd-match)"; }
s_cwd_differs()  { meta_set "launch_id.main" "TOK-NOBODY-CARRIES-THIS"
                   meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" /tmp cwd-differs)"; }
s_malformed_id() { meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) '{"id":"NOT-A-UUID!!","cwd":"'"$H_WORK"'"}'; }
s_empty_first()  { meta_set "launch_time.main" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) ""; }
s_yesterday()    { meta_set "launch_time.main" "$((NOW - 172800))"
                   plant "$YESTERDAY" cand-y $((NOW - 90000)) "$(line "$ID_B" "$H_WORK" yesterday)"; }
s_other_slot()   { meta_set "launch_time.worker.0" "$((NOW - 600))"
                   plant "$TODAY" cand-a $((NOW - 60)) "$(line "$ID_A" "$H_WORK" other-slot)"; }

run_case h5-c01-no-slot-arg        ""         s_no_meta      "no slot argument"
run_case h5-c02-slot-no-launchtime "main"     s_no_meta      "a slot with no launch_time"
run_case h5-c03-token-match        "main"     s_token_match  "a candidate carrying the launch-id token"
run_case h5-c04-token-mismatch     "main"     s_token_wrong  "a candidate carrying a different token"
run_case h5-c05-older-than-launch  "main"     s_older_than   "a candidate older than the launch time"
run_case h5-c06-nonnumeric-time    "main"     s_nonnumeric   "a non-numeric launch time"
run_case h5-c07-equal-mtimes       "main"     s_equal_mtime  "two candidates with equal mtimes"
run_case h5-c08-different-mtimes   "main"     s_diff_mtime   "two candidates with different mtimes"
run_case h5-c09-cwd-match          "main"     s_cwd_match    "a candidate whose cwd matches the invoking cwd"
run_case h5-c10-cwd-differs        "main"     s_cwd_differs  "a candidate whose cwd differs"
run_case h5-c11-malformed-id       "main"     s_malformed_id "a malformed first-line id"
run_case h5-c12-empty-first-line   "main"     s_empty_first  "an empty first line"
run_case h5-c13-yesterday          "main"     s_yesterday    "a candidate in yesterday's directory"
run_case h5-c14-other-slot         "worker.0" s_other_slot   "a slot other than the invoking pane's"
echo "A-H5 DONE"
