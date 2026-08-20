#!/opt/homebrew/bin/bash
# Arm-execution library: clone a template member, run the read-side consumer
# families under a scrubbed environment, capture everything. No verdicts.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/tlib.sh"

AROOT=/tmp/aecx/arms
ADEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms

# The documented minimum environment for a scrubbed consumer run.
scrub_env() { # <ae-home> <tmux-socket-or-->  -> prints an env -i prefix as words
    local aehome="$1" sock="$2"
    printf '%s\0' env -i \
        "HOME=$(dirname "$aehome")" \
        "AE_HOME=$aehome" \
        "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
        "TZ=UTC" "LANG=${ARM_LOCALE:-en_US.UTF-8}" "LC_ALL=${ARM_LOCALE:-en_US.UTF-8}" "TERM=xterm-256color" \
        "TMUX_TMPDIR=${ARM_TMUXTMP}" \
        ${sock:+"AE_TMUX_SERVER=$sock"} ${sock:+"AE_TMUX_SERVER_KIND=socket"}
}

# run_consumer <outdir> <label> <ae-home> <sock-or-empty> -- <argv...>
run_consumer() {
    local out="$1" lbl="$2" aehome="$3" sock="$4"; shift 4; [[ "${1:-}" == "--" ]] && shift
    mkdir -p "$out"
    local -a pre=(env -i
        "HOME=$(dirname "$aehome")" "AE_HOME=$aehome"
        "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=${ARM_LOCALE:-en_US.UTF-8}" "LC_ALL=${ARM_LOCALE:-en_US.UTF-8}" "TERM=xterm-256color"
        "TMUX_TMPDIR=${ARM_TMUXTMP}")
    [[ -n "$sock" ]] && pre+=("AE_TMUX_SERVER=$sock" "AE_TMUX_SERVER_KIND=socket")
    if [[ "${ARM_TMUX_TRACE:-0}" == 1 ]]; then
        # delegate-and-log tmux shim FIRST on PATH, one trace file per invocation
        pre=("${pre[@]/#PATH=/PATH=/tmp/aecx/shim-tmux:}")
        pre+=("AE_TMUX_SHIM_LOG=$out/$lbl.tmuxtrace" "AE_REAL_TMUX=/opt/homebrew/bin/tmux")
        : >"$out/$lbl.tmuxtrace"
    fi
    printf '%s\n' "${pre[@]}" >"$out/$lbl.env.txt"
    printf 'argv:\n' >>"$out/$lbl.env.txt"
    printf '  %q\n' "$@" >>"$out/$lbl.env.txt"
    local rc=0
    "${pre[@]}" "$@" </dev/null >"$out/$lbl.stdout" 2>"$out/$lbl.stderr" || rc=$?
    printf '%s\n' "$rc" >"$out/$lbl.rc"
    { printf 'stdout_sha256=%s bytes=%s\n' "$(shasum -a 256 "$out/$lbl.stdout" | cut -d' ' -f1)" "$(stat -f %z "$out/$lbl.stdout")"
      printf 'stderr_sha256=%s bytes=%s\n' "$(shasum -a 256 "$out/$lbl.stderr" | cut -d' ' -f1)" "$(stat -f %z "$out/$lbl.stderr")"
      printf 'rc=%s\n' "$rc"; } >"$out/$lbl.hash.txt"
    return 0
}

# Delegate-and-log tmux shim: INACTIVE-EQUIVALENCE on the arm's own stable topology.
# The shim has no active mode — it logs and execs — so equivalence is proven by running
# the read-only queries the consumers make through the shim and through the real binary
# on the SAME unchanged topology and comparing stdout, stderr and rc byte for byte.
tmux_shim_equiv() { # <socket> <session> <outfile>
    local sock="$1" sess="$2" out="$3" ok=1
    local -a queries=(
        "list-sessions -F #{session_name}"
        "list-panes -s -t $sess -F #{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}"
        "list-windows -a -F #{session_name}|#{window_index}|#{window_name}"
        "display-message -p -t $sess #{session_name}"
    )
    { echo "## tmux delegate-and-log shim — inactive equivalence on this arm's topology"
      echo "shim=/tmp/aecx/shim-tmux/tmux sha256=$(shasum -a 256 /tmp/aecx/shim-tmux/tmux | cut -d' ' -f1)"
      echo "real=/opt/homebrew/bin/tmux sha256=$(shasum -a 256 /opt/homebrew/bin/tmux | cut -d' ' -f1)"
    } >"$out"
    local q
    for q in "${queries[@]}"; do
        # shellcheck disable=SC2086
        local a b ra rb
        a="$(/tmp/aecx/shim-tmux/tmux -S "$sock" $q 2>&1)"; ra=$?
        b="$(/opt/homebrew/bin/tmux -S "$sock" $q 2>&1)"; rb=$?
        if [[ "$a" == "$b" && "$ra" == "$rb" ]]; then
            printf 'query=%s\n  IDENTICAL stdout+stderr+rc (rc=%s, %s bytes)\n' "$q" "$ra" "${#a}" >>"$out"
        else
            ok=0
            printf 'query=%s\n  DIVERGED shim(rc=%s)=%q real(rc=%s)=%q\n' "$q" "$ra" "$a" "$rb" "$b" >>"$out"
        fi
    done
    echo "all_identical=$( ((ok)) && echo yes || echo no )" >>"$out"
    ((ok))
}

# ENVIRONMENT ADMISSIBILITY: the frozen consumer's own pane query is TAB-separated
# (ae@72c7293:3631 and :4207). tmux picks its output encoding from LC_ALL/LC_CTYPE/LANG
# and SANITISES that TAB to '_' when none of them names UTF-8 — which silently corrupts
# the consumer's parse upstream of ae and renders every agent not-alive. So every arm
# proves, in ITS OWN scrubbed environment and before any capture, that a real TAB
# round-trips through that exact query. Writes the evidence; returns non-zero if not.
env_tab_selfcheck() { # <outfile>
    local out="$1"
    local sk="${ARM_TMUXTMP}/tabcheck.sock"
    local -a pre=(env -i
        "HOME=${ARM_TMUXTMP}" "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=${ARM_LOCALE:-en_US.UTF-8}" "LC_ALL=${ARM_LOCALE:-en_US.UTF-8}" "TERM=xterm-256color"
        "TMUX_TMPDIR=${ARM_TMUXTMP}")
    "${pre[@]}" tmux -S "$sk" kill-server >/dev/null 2>&1
    "${pre[@]}" tmux -S "$sk" new-session -d -s tabcheck 'sleep 30' >/dev/null 2>&1
    "${pre[@]}" tmux -S "$sk" set-option -p -t tabcheck @ae_agent 'x:y' >/dev/null 2>&1
    local fmt; fmt="$(printf '#{@ae_agent}\t#{pane_current_command}')"
    local raw; raw="$("${pre[@]}" tmux -S "$sk" list-panes -s -t tabcheck -F "$fmt" 2>&1)"
    local a b; IFS=$'\t' read -r a b <<<"$raw"
    # Same probe, same server, C locale — recorded as a PAIRED RAW capture beside the
    # UTF-8 one. No comparison verdict is drawn here; both byte strings are published.
    local -a prec=(env -i
        "HOME=${ARM_TMUXTMP}" "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=C" "LC_ALL=C" "TERM=xterm-256color" "TMUX_TMPDIR=${ARM_TMUXTMP}")
    local rawc; rawc="$("${prec[@]}" tmux -S "$sk" list-panes -s -t tabcheck -F "$fmt" 2>&1)"
    local ca cb; IFS=$'\t' read -r ca cb <<<"$rawc"
    {
      echo "## environment admissibility — consumer tab round-trip"
      echo "query=tmux list-panes -s -t <session> -F '#{@ae_agent}<TAB>#{pane_current_command}'"
      echo "locale=LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 (UTF-8 required: tmux sanitises the TAB otherwise)"
      printf 'raw_bytes='; printf '%s' "$raw" | od -c | head -2
      echo "split_field_1=[$a]"
      echo "split_field_2=[$b]"
      echo "tab_survived=$( [[ "$a" == "x:y" && -n "$b" ]] && echo yes || echo no )"
      echo "note=the oracle is that the TAB delimited TWO fields; the pane command itself is whatever tmux runs the throwaway command under and is not part of the check"
      echo "## paired raw capture — identical probe, identical server, LANG=C LC_ALL=C"
      printf 'c_locale_raw_bytes='; printf '%s' "$rawc" | od -c | head -2
      echo "c_locale_split_field_1=[$ca]"
      echo "c_locale_split_field_2=[$cb]"
    } >"$out"
    "${pre[@]}" tmux -S "$sk" kill-server >/dev/null 2>&1
    [[ "$a" == "x:y" && -n "$b" ]]
}

# run_consumer_bounded <seconds> <outdir> <label> <ae-home> <sock> -- <argv...>
run_consumer_bounded() {
    local secs="$1"; shift
    local out="$1" lbl="$2" aehome="$3" sock="$4"; shift 4; [[ "${1:-}" == "--" ]] && shift
    mkdir -p "$out"
    local -a pre=(env -i
        "HOME=$(dirname "$aehome")" "AE_HOME=$aehome"
        "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=${ARM_LOCALE:-en_US.UTF-8}" "LC_ALL=${ARM_LOCALE:-en_US.UTF-8}" "TERM=xterm-256color"
        "TMUX_TMPDIR=${ARM_TMUXTMP}")
    [[ -n "$sock" ]] && pre+=("AE_TMUX_SERVER=$sock" "AE_TMUX_SERVER_KIND=socket")
    if [[ "${ARM_TMUX_TRACE:-0}" == 1 ]]; then
        # delegate-and-log tmux shim FIRST on PATH, one trace file per invocation
        pre=("${pre[@]/#PATH=/PATH=/tmp/aecx/shim-tmux:}")
        pre+=("AE_TMUX_SHIM_LOG=$out/$lbl.tmuxtrace" "AE_REAL_TMUX=/opt/homebrew/bin/tmux")
        : >"$out/$lbl.tmuxtrace"
    fi
    printf '%s\n' "${pre[@]}" >"$out/$lbl.env.txt"
    printf 'argv:\n' >>"$out/$lbl.env.txt"
    printf '  %q\n' "$@" >>"$out/$lbl.env.txt"
    printf 'bounded_by_harness_seconds=%s\n' "$secs" >>"$out/$lbl.env.txt"
    "${pre[@]}" "$@" </dev/null >"$out/$lbl.stdout" 2>"$out/$lbl.stderr" &
    local pid=$!
    local i=0
    while ((i < secs * 10)); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; i=$((i+1)); done
    local rc stopped=no
    if kill -0 "$pid" 2>/dev/null; then
        pkill -P "$pid" 2>/dev/null; kill -TERM "$pid" 2>/dev/null; stopped=yes
    fi
    wait "$pid" 2>/dev/null; rc=$?
    printf '%s\n' "$rc" >"$out/$lbl.rc"
    { printf 'stdout_sha256=%s bytes=%s\n' "$(shasum -a 256 "$out/$lbl.stdout" | cut -d' ' -f1)" "$(stat -f %z "$out/$lbl.stdout")"
      printf 'stderr_sha256=%s bytes=%s\n' "$(shasum -a 256 "$out/$lbl.stderr" | cut -d' ' -f1)" "$(stat -f %z "$out/$lbl.stderr")"
      printf 'rc=%s\n' "$rc"
      printf 'stopped_by_harness_after=%ss=%s\n' "$secs" "$stopped"; } >"$out/$lbl.hash.txt"
    return 0
}

# consumer_battery <outdir> <ae-home> <session> <sock-or-empty>
consumer_battery() {
    local out="$1" aehome="$2" sess="$3" sock="$4"
    local AE="$FROZEN_AE" B="$HARNESS_BASH"
    run_consumer "$out" "list"          "$aehome" "$sock" -- "$B" "$AE" list
    run_consumer "$out" "list-json"     "$aehome" "$sock" -- "$B" "$AE" list --json
    run_consumer "$out" "list-all"      "$aehome" "$sock" -- "$B" "$AE" list --all
    run_consumer "$out" "list-all-json" "$aehome" "$sock" -- "$B" "$AE" list --all --json
    run_consumer "$out" "ls-alias"      "$aehome" "$sock" -- "$B" "$AE" ls
    run_consumer "$out" "ls-alias-all"  "$aehome" "$sock" -- "$B" "$AE" ls --all
    run_consumer "$out" "status"      "$aehome" "$sock" -- "$B" "$AE" status "$sess"
    run_consumer "$out" "next"        "$aehome" "$sock" -- "$B" "$AE" next
    if [[ -x "$aehome/sessions/$sess/requests" ]]; then
        run_consumer "$out" "requests-all" "$aehome" "$sock" -- "$aehome/sessions/$sess/requests" all
        run_consumer "$out" "agents"       "$aehome" "$sock" -- "$aehome/sessions/$sess/agents"
        # events-tail is a STREAMING consumer with no one-shot mode: it is run bounded
        # and stopped by the harness, and that fact is recorded beside its bytes.
        run_consumer_bounded 4 "$out" "events-tail" "$aehome" "$sock" -- "$aehome/sessions/$sess/events-tail"
    fi
}

# arm_case <arm> <case-id> <group> <member> <ro|rw>  — clone, manifest, run, manifest, diff
arm_case() {
    local arm="$1" cid="$2" grp="$3" mem="$4" mode="$5"
    local base="$AROOT/$arm/$cid-$mode"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null
    rm -rf "$base"; mkdir -p "$base"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    export ARM_TMUX_TRACE=1
    local aehome="$base/home/.ae"
    mkdir -p "$base/home"
    t_clone "$grp" "$mem" "$aehome" "$mode" || { echo "CLONE FAILED $grp/$mem"; return 1; }
    local sess; sess="$(ls "$aehome/sessions" 2>/dev/null | head -1)"
    local out="$base/cap"; mkdir -p "$out"
    if ! env_tab_selfcheck "$out/env-tab-selfcheck.txt"; then
        echo "  HARNESS-ABORT case $cid: consumer tab round-trip failed its environment self-check"
        echo "HARNESS-ABORT=environment tab self-check failed; no capture taken" >>"$out/env-tab-selfcheck.txt"
        return 1
    fi
    local sock="$base/none.sock"   # a socket path with no server behind it
    {
      echo "arm=$arm case=$cid template=$grp/$mem clone_mode=$mode session=$sess"
      echo "template_fingerprint_pre_protection=$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      echo "template_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      local _cf _exp
      _cf="$(dir_fingerprint "$aehome")"
      if [[ "$mode" == "ro" ]]; then _exp="$(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
      else _exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"; fi
      echo "clone_fingerprint=$_cf"
      echo "clone_fingerprint_matches_template=$( [[ "$_cf" == "$_exp" ]] && echo yes || echo no )"
      echo "tmux_socket=$sock (no server started for this case)"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(shasum -a 256 "$FROZEN_AE" | cut -d' ' -f1)"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >"$out/case.txt"
    dir_manifest "$aehome" >"$out/manifest.before.tsv"
    command tmux -S "$sock" list-sessions >"$out/tmux.before.txt" 2>&1
    consumer_battery "$out" "$aehome" "$sess" "$sock"
    dir_manifest "$aehome" >"$out/manifest.after.tsv"
    command tmux -S "$sock" list-sessions >"$out/tmux.after.txt" 2>&1
    diff "$out/manifest.before.tsv" "$out/manifest.after.tsv" >"$out/manifest.diff.txt" 2>&1
    local dl; dl="$(wc -l <"$out/manifest.diff.txt" | tr -d ' ')"
    { echo "manifest_diff_lines=$dl"
      echo "manifest_before_sha256=$(shasum -a 256 "$out/manifest.before.tsv" | cut -d' ' -f1)"
      echo "manifest_after_sha256=$(shasum -a 256 "$out/manifest.after.tsv" | cut -d' ' -f1)"
      echo "tmux_snapshot_identical=$( [[ "$(cat "$out/tmux.before.txt")" == "$(cat "$out/tmux.after.txt")" ]] && echo yes || echo no)"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$out/case.txt"
    echo "  case $cid ($grp/$mem, $mode): manifest_diff_lines=$dl"
    return 0
}
