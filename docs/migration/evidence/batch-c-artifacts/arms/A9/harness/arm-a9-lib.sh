#!/opt/homebrew/bin/bash
# ARM GROUP A9 — quiet vs degraded. Rows: SC-519, SC-520, SC-405i.
#
# SC-405i is an ABSENCE claim, and three different states render the same emptiness:
#   meta ABSENT              A9/meta-absent
#   meta PRESENT, UNREADABLE  G3/meta-mode-000  (same bytes, mode 000)
#   the reader never looked   a property of the INSTRUMENT, not of any fixture
# So every case records the source state from the FILESYSTEM (meta-state.txt: exists,
# type, mode, size, hash if readable, and the rc/stderr of an actual read attempt), and
# the arm carries G1/healthy through the identical consumer set as a positive control —
# a reader that looks renders something there, so an empty rendering elsewhere is known
# not to be the instrument declining to look.
source "$(dirname "$0")/armlib.sh"
ARMG=A9
mkdir -p "$ADEST/$ARMG"
[[ -f "$ADEST/$ARMG/ledger.tsv" ]] || printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"

a9_case() { # <case-id> <rows> <group> <member> <mode> <live: yes|no>
    local cid="$1" rows="$2" grp="$3" mem="$4" mode="$5" live="$6"
    local suffix="$mode"; [[ "$live" == no ]] && suffix="$mode-noserver"
    local base="$AROOT/$ARMG/$cid-$suffix"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "$suffix"
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
        # Fixtures whose own meta is absent or unreadable BY DESIGN cannot supply the
        # roster; they name the member they were derived from instead, so every case in
        # the arm gets the SAME topology and only the fixture's files differ.
        local roster_override=""
        case "$grp/$mem" in
            A9/meta-absent|G3/meta-mode-000) roster_override="$TSTORE/G1/healthy/sessions/$sess/meta" ;;
        esac
        AE_ROSTER_META="$roster_override" build_live_topology "$aehome" "$sock" "$sess" \
            || { led CASE-ABORTED "reason=live topology"; return 1; }
    else
        led no-live-topology "reason=this subarm reads the same fixture with no tmux server at all"
    fi
    { echo "arm=$ARMG case=$cid"
      echo "rows=$rows"
      echo "template=$grp/$mem"
      echo "clone_mode=$suffix"
      echo "session=$sess"
      echo "live_topology=$live"
      echo "clone_fingerprint=$cf"
      echo "clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "frozen_sha=$FROZEN_SHA"
      echo "frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    [[ -f "$TSTORE/$grp/_meta/$mem.mutation.txt" ]] && cp "$TSTORE/$grp/_meta/$mem.mutation.txt" "$ACAP/member.mutation.txt"
    a9_source_state "$aehome" "$sess"
    cp "$aehome/sessions/$sess/meta" "$ACAP/meta.bytes.txt" 2>/dev/null || true
    cp "$aehome/sessions/$sess/events.jsonl" "$ACAP/events.bytes.jsonl" 2>/dev/null || true
    led fixture-bytes "meta_sha256=$(sha "$ACAP/meta.bytes.txt")" \
        "events_sha256=$(sha "$ACAP/events.bytes.jsonl")"
    case_env_record "$aehome" "$( [[ "$live" == yes ]] && echo "$sock" )"
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    [[ "$live" == yes ]] && { tmux_shim_equiv "$sock" "$sess" || { led HARNESS-ABORT "reason=tmux shim equivalence"; return 1; }; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    { echo "## sessions"; command tmux -S "$sock" list-sessions -F '#{session_name}' 2>&1
      echo "## panes"; command tmux -S "$sock" list-panes -a -F '#{session_name}|#{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}' 2>&1
    } >"$ACAP/tmux.before.txt"
    local B="$HARNESS_BASH" AE="$FROZEN_AE" S4="$( [[ "$live" == yes ]] && echo "$sock" )"
    run_consumer "list"          "$aehome" "$S4" -- "$B" "$AE" list
    run_consumer "list-json"     "$aehome" "$S4" -- "$B" "$AE" list --json
    run_consumer "list-all"      "$aehome" "$S4" -- "$B" "$AE" list --all
    run_consumer "list-all-json" "$aehome" "$S4" -- "$B" "$AE" list --all --json
    run_consumer "status"        "$aehome" "$S4" -- "$B" "$AE" status "$sess"
    run_consumer "next"          "$aehome" "$S4" -- "$B" "$AE" next
    [[ -x "$aehome/sessions/$sess/requests" ]] && run_consumer "requests-all" "$aehome" "$S4" -- "$aehome/sessions/$sess/requests" all
    [[ -x "$aehome/sessions/$sess/agents" ]]   && run_consumer "agents"       "$aehome" "$S4" -- "$aehome/sessions/$sess/agents"
    cp "$ACAP/tmux.before.txt" "$ACAP/tmux.after.txt"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')" >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    command tmux -S "$sock" kill-server >/dev/null 2>&1; pkill -x aefake >/dev/null 2>&1
    echo "  $cid ($suffix/live=$live) done"
}

# The SOURCE state, read from the filesystem rather than inferred from the rendering.
# "absent" and "present but unreadable" are different rows here even though a consumer
# that renders neither cannot tell them apart.
a9_source_state() { # <aehome> <sess>
    local aehome="$1" sess="$2" out="$ACAP/meta-state.txt"
    local d="$aehome/sessions/$sess"
    { echo "## source state of the files the rows are about, read from the FILESYSTEM"
      echo "session_dir=sessions/$sess"
      echo "session_dir_exists=$( [[ -d "$d" ]] && echo yes || echo no )"
      echo "session_dir_files=$(find "$d" -type f 2>/dev/null | wc -l | tr -d ' ')"
      for f in meta events.jsonl; do
          local p="$d/$f" rc out1
          echo "### $f"
          echo "  exists=$( [[ -e "$p" ]] && echo yes || echo no )"
          if [[ -e "$p" ]]; then
              echo "  type=$( [[ -L "$p" ]] && echo symlink || { [[ -d "$p" ]] && echo dir || echo file; } )"
              echo "  mode=$(stat -f %Lp "$p" 2>/dev/null || echo '-')"
              echo "  size=$(stat -f %z "$p" 2>/dev/null || echo '-')"
          else
              echo "  type=ABSENT"; echo "  mode=-"; echo "  size=-"
          fi
          out1="$(cat "$p" 2>&1 >/dev/null)"; rc=$?
          echo "  read_attempt_rc=$rc"
          echo "  read_attempt_stderr=${out1:-<none>}"
          if (( rc == 0 )) && [[ -e "$p" ]]; then
              echo "  sha256=$(shasum -a 256 "$p" | cut -d' ' -f1)"
          else
              echo "  sha256=- (not readable by the consumer's own uid)"
          fi
      done
    } >"$out"
    led source-state-captured "artifact_sha256=$(sha "$out")" \
        "note=absent vs present-but-unreadable separated at the source, not at the rendering"
}

C9() { # <case-id> <rows> <group> <member>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$ADEST/$ARMG/ledger.tsv"
    a9_case "$1" "$2" "$3" "$4" ro yes
    a9_case "$1" "$2" "$3" "$4" rw yes
    a9_case "$1" "$2" "$3" "$4" ro no
}
