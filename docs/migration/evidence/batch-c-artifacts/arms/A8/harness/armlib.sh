#!/opt/homebrew/bin/bash
# Arm-execution library.
#
# Captures are written DIRECTLY into the committed artifact tree — there is no
# scratch-then-publish step for evidence, so no capture can exist without its
# admissibility proof beside it. Only the sandboxes (clones, tmux tmpdirs, sockets)
# live in /tmp.
#
# Every case keeps an append-only ADMISSIBILITY LEDGER: a monotonic seq + UTC + epoch
# record of each check and each consumer invocation, START and COMPLETE, with the
# artifact's own sha256 tied into the line. Ordering is therefore established by the
# original durable content itself, not by filesystem mtimes or by a later hash list.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/tlib.sh"

AROOT=/tmp/aecx/arms                     # sandboxes only
ADEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms

_led_seq=0
led() { # <event> [k=v ...]
    _led_seq=$((_led_seq + 1))
    printf 'seq=%03d\tutc=%s\tepoch=%s\tevent=%s\t%s\n' \
        "$_led_seq" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" "$(/bin/date -u +%s)" \
        "$1" "${*:2}" >>"$ACAP/admissibility-ledger.txt"
}
sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1; }

# --- the scrubbed consumer environment ------------------------------------
# UTF-8, not C: tmux picks its output encoding from LC_ALL/LC_CTYPE/LANG and sanitises
# the TAB in -F format output when none of them names UTF-8, which corrupts the frozen
# consumer's own tab-separated pane queries upstream of ae.
consumer_env() { # <ae-home> <sock-or-empty> <trace-file-or-empty>
    local aehome="$1" sock="$2" trace="$3"
    CONSUMER_ENV=(env -i
        "HOME=$(dirname "$aehome")" "AE_HOME=$aehome"
        "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=${ARM_LOCALE:-en_US.UTF-8}" "LC_ALL=${ARM_LOCALE:-en_US.UTF-8}"
        "TERM=xterm-256color" "TMUX_TMPDIR=${ARM_TMUXTMP}")
    [[ -n "$sock" ]] && CONSUMER_ENV+=("AE_TMUX_SERVER=$sock" "AE_TMUX_SERVER_KIND=socket")
    if [[ -n "$trace" ]]; then
        CONSUMER_ENV=("${CONSUMER_ENV[@]/#PATH=/PATH=/tmp/aecx/shim-tmux:}")
        CONSUMER_ENV+=("AE_TMUX_SHIM_LOG=$trace" "AE_REAL_TMUX=/opt/homebrew/bin/tmux")
        : >"$trace"
    fi
    # ARM_FAKE_NOW freezes the consumer's clock through the PATH-first date shim, for arms
    # whose row is about a TIME WINDOW and would otherwise be vacuous at whatever the wall
    # clock happens to be. The shim delegates every non-now-form.
    # ARM_EXTRA_ENV adds ONE variable to the scrubbed set, for arms whose row IS an
    # environment variable. It is added to the recorded env.txt like everything else, so the
    # capture says exactly what the consumer carried.
    [[ -n "${ARM_EXTRA_ENV:-}" ]] && CONSUMER_ENV+=("$ARM_EXTRA_ENV")
    if [[ -n "${ARM_FAKE_NOW:-}" ]]; then
        CONSUMER_ENV=("${CONSUMER_ENV[@]/#PATH=/PATH=/tmp/aecx/shim:}")
        CONSUMER_ENV+=("AE_FAKE_NOW=$ARM_FAKE_NOW" "AE_REAL_DATE=/bin/date")
        [[ -n "${ARM_DATE_SHIM_LOG:-}" ]] && CONSUMER_ENV+=("AE_DATE_SHIM_LOG=$ARM_DATE_SHIM_LOG")
    fi
}

# run_consumer <label> <ae-home> <sock-or-empty> [--bounded <secs>] -- <argv...>
run_consumer() {
    local lbl="$1" aehome="$2" sock="$3"; shift 3
    local secs=0
    if [[ "${1:-}" == "--bounded" ]]; then secs="$2"; shift 2; fi
    [[ "${1:-}" == "--" ]] && shift
    mkdir -p "$ACAP/out"
    local trace="$ACAP/out/$lbl.tmuxtrace"
    consumer_env "$aehome" "$sock" "$trace"
    local argvq; argvq="$(printf '%q ' "$@")"
    led consumer-START "label=$lbl" "bounded_secs=$secs" "argv=$argvq"
    local rc=0 stopped=no
    if ((secs > 0)); then
        "${CONSUMER_ENV[@]}" "$@" </dev/null >"$ACAP/out/$lbl.stdout" 2>"$ACAP/out/$lbl.stderr" &
        local pid=$! i=0
        while ((i < secs * 10)); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; i=$((i + 1)); done
        if kill -0 "$pid" 2>/dev/null; then pkill -P "$pid" 2>/dev/null; kill -TERM "$pid" 2>/dev/null; stopped=yes; fi
        wait "$pid" 2>/dev/null; rc=$?
    else
        "${CONSUMER_ENV[@]}" "$@" </dev/null >"$ACAP/out/$lbl.stdout" 2>"$ACAP/out/$lbl.stderr" || rc=$?
    fi
    [[ -s "$ACAP/out/$lbl.stderr" ]] || rm -f "$ACAP/out/$lbl.stderr"
    local so="$ACAP/out/$lbl.stdout" se="$ACAP/out/$lbl.stderr"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$lbl" "$rc" \
        "$(sha "$so")" "$(stat -f %z "$so" 2>/dev/null || echo 0)" \
        "$( [[ -f "$se" ]] && sha "$se" || echo '-')" "$( [[ -f "$se" ]] && stat -f %z "$se" || echo 0)" \
        "$(sha "$trace")" "$(wc -l <"$trace" 2>/dev/null | tr -d ' ')" \
        "$( ((secs>0)) && echo "${secs}s=$stopped" || echo '-')" "$argvq" >>"$ACAP/consumers.tsv"
    led consumer-COMPLETE "label=$lbl" "rc=$rc" "stdout_sha256=$(sha "$so")" \
        "stderr_sha256=$( [[ -f "$se" ]] && sha "$se" || echo '-')" "tmuxtrace_sha256=$(sha "$trace")" \
        "harness_stopped=$stopped"
    return 0
}

# ENVIRONMENT ADMISSIBILITY: the frozen consumer's own pane query is TAB-separated
# (ae@72c7293:3631, :4207). tmux sanitises that TAB when the locale names no UTF-8, which
# corrupts the consumer's parse before ae ever sees it. Every case proves, in ITS OWN
# scrubbed environment and BEFORE any consumer invocation, that a real TAB round-trips
# through that exact query. The C-locale probe runs on the same throwaway server and is
# published beside it as a paired RAW capture — both byte strings, no comparison verdict.
env_tab_selfcheck() {
    local out="$ACAP/env-tab-selfcheck.txt"
    led env-tab-selfcheck-START "artifact=env-tab-selfcheck.txt"
    local sk="${ARM_TMUXTMP}/tabcheck.sock"
    local -a pre=(env -i "HOME=${ARM_TMUXTMP}" "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color" "TMUX_TMPDIR=${ARM_TMUXTMP}")
    local -a prec=(env -i "HOME=${ARM_TMUXTMP}" "PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=C" "LC_ALL=C" "TERM=xterm-256color" "TMUX_TMPDIR=${ARM_TMUXTMP}")
    "${pre[@]}" tmux -S "$sk" kill-server >/dev/null 2>&1
    "${pre[@]}" tmux -S "$sk" new-session -d -s tabcheck 'sleep 30' >/dev/null 2>&1
    "${pre[@]}" tmux -S "$sk" set-option -p -t tabcheck @ae_agent 'x:y' >/dev/null 2>&1
    local fmt; fmt="$(printf '#{@ae_agent}\t#{pane_current_command}')"
    local raw rawc a b ca cb
    raw="$("${pre[@]}" tmux -S "$sk" list-panes -s -t tabcheck -F "$fmt" 2>&1)"
    IFS=$'\t' read -r a b <<<"$raw"
    rawc="$("${prec[@]}" tmux -S "$sk" list-panes -s -t tabcheck -F "$fmt" 2>&1)"
    IFS=$'\t' read -r ca cb <<<"$rawc"
    {
      echo "## environment admissibility — consumer TAB round-trip"
      echo "case=${ACASE:-?} clone_mode=${AMODE:-?}"
      echo "query=tmux list-panes -s -t <session> -F '#{@ae_agent}<TAB>#{pane_current_command}'"
      echo "locale=LANG=LC_ALL=en_US.UTF-8"
      printf 'raw_bytes='; printf '%s' "$raw" | od -c | head -2
      echo "split_field_1=[$a]"
      echo "split_field_2=[$b]"
      echo "tab_survived=$( [[ "$a" == "x:y" && -n "$b" ]] && echo yes || echo no )"
      echo "oracle=the TAB delimited TWO fields; the throwaway pane's own command is not part of the check"
      echo "## paired raw capture — identical probe, identical server, LANG=LC_ALL=C"
      printf 'c_locale_raw_bytes='; printf '%s' "$rawc" | od -c | head -2
      echo "c_locale_split_field_1=[$ca]"
      echo "c_locale_split_field_2=[$cb]"
    } >"$out"
    "${pre[@]}" tmux -S "$sk" kill-server >/dev/null 2>&1
    local ok=no; [[ "$a" == "x:y" && -n "$b" ]] && ok=yes
    led env-tab-selfcheck-COMPLETE "tab_survived=$ok" "artifact_sha256=$(sha "$out")"
    [[ "$ok" == yes ]]
}

# Delegate-and-log tmux shim: INACTIVE EQUIVALENCE on the arm's own stable topology.
tmux_shim_equiv() { # <socket> <session>
    local sock="$1" sess="$2" out="$ACAP/tmux-shim-equivalence.txt" ok=1
    led tmux-shim-equivalence-START "socket=$sock" "session=$sess"
    local -a queries=(
        "list-sessions -F #{session_name}"
        "list-panes -s -t $sess -F #{pane_id}|#{@ae_agent}|#{@ae_slot}|#{pane_current_command}"
        "list-windows -a -F #{session_name}|#{window_index}|#{window_name}"
        "display-message -p -t $sess #{session_name}"
    )
    { echo "## tmux delegate-and-log shim — inactive equivalence on this arm's topology"
      echo "case=${ACASE:-?} clone_mode=${AMODE:-?}"
      echo "shim=/tmp/aecx/shim-tmux/tmux sha256=$(sha /tmp/aecx/shim-tmux/tmux)"
      echo "real=/opt/homebrew/bin/tmux sha256=$(sha /opt/homebrew/bin/tmux)"
    } >"$out"
    local q a b ra rb
    for q in "${queries[@]}"; do
        # shellcheck disable=SC2086
        a="$(/tmp/aecx/shim-tmux/tmux -S "$sock" $q 2>&1)"; ra=$?
        # shellcheck disable=SC2086
        b="$(/opt/homebrew/bin/tmux -S "$sock" $q 2>&1)"; rb=$?
        if [[ "$a" == "$b" && "$ra" == "$rb" ]]; then
            printf 'query=%s\n  IDENTICAL stdout+stderr+rc (rc=%s, %s bytes)\n' "$q" "$ra" "${#a}" >>"$out"
        else
            ok=0
            printf 'query=%s\n  DIVERGED shim(rc=%s)=%q real(rc=%s)=%q\n' "$q" "$ra" "$a" "$rb" "$b" >>"$out"
        fi
    done
    echo "all_identical=$( ((ok)) && echo yes || echo no )" >>"$out"
    led tmux-shim-equivalence-COMPLETE "all_identical=$( ((ok)) && echo yes || echo no )" "artifact_sha256=$(sha "$out")"
    ((ok))
}

# case_open <arm> <case-id> <mode>  — creates the committed capture dir + ledger
case_open() {
    ARM="$1"; ACASE="$2"; AMODE="$3"
    ACAP="$ADEST/$ARM/${ACASE}-${AMODE}"
    rm -rf "$ACAP"; mkdir -p "$ACAP/out"
    _led_seq=0
    : >"$ACAP/admissibility-ledger.txt"
    printf 'consumer\trc\tstdout_sha256\tstdout_bytes\tstderr_sha256\tstderr_bytes\ttmuxtrace_sha256\ttmuxtrace_lines\tbounded\targv\n' >"$ACAP/consumers.tsv"
    led case-OPEN "arm=$ARM" "case=$ACASE" "clone_mode=$AMODE" "frozen_sha=$FROZEN_SHA" \
        "frozen_ae_sha256=$(sha "$FROZEN_AE")"
}

case_env_record() { # <ae-home> <sock>
    consumer_env "$1" "$2" ""
    printf '%s\n' "${CONSUMER_ENV[@]}" >"$ACAP/env.txt"
    led env-recorded "artifact_sha256=$(sha "$ACAP/env.txt")"
}

# Harness-built LIVE topology for a cloned AE_HOME: one tmux session per named session
# dir, one pane per roster entry from that session's OWN meta, running the fixture's
# controllable fake binary, stamped @ae_agent/@ae_slot and carrying the session
# environment ae itself writes at launch (ae@72c7293:17311-17318). Without AE_SESSION the
# frozen enumerator does not treat a tmux session as an ae session at all.
build_live_topology() { # <ae-home> <sock> <session>...
    local aehome="$1" sock="$2"; shift 2
    command tmux -S "$sock" kill-server >/dev/null 2>&1
    local s
    for s in "$@"; do
        local meta="$aehome/sessions/$s/meta" first=1 wd k v slot ref pane
        [[ -f "$meta" ]] || continue
        wd="$(grep '^work_dir=' "$meta" | cut -d= -f2-)"; [[ -d "$wd" ]] || wd="$aehome"
        while IFS='=' read -r k v; do
            [[ "$k" == agent.* ]] || continue
            slot="${k#agent.}"; ref="${v%:*}"
            if ((first)); then
                pane="$(command tmux -S "$sock" new-session -d -s "$s" -c "$wd" -P -F '#{pane_id}' "$FAKE_BIN")"; first=0
            else
                pane="$(command tmux -S "$sock" split-window -d -t "$s" -c "$wd" -P -F '#{pane_id}' "$FAKE_BIN")"
            fi
            command tmux -S "$sock" set-option -p -t "$pane" @ae_agent "$ref"
            command tmux -S "$sock" set-option -p -t "$pane" @ae_slot "$slot"
        done <"$meta"
        command tmux -S "$sock" set-environment -t "$s" AE_SESSION 1
        command tmux -S "$sock" set-environment -t "$s" AE_ORIGIN "$(grep '^origin=' "$meta" | cut -d= -f2-)"
        command tmux -S "$sock" set-environment -t "$s" AE_DIR "$wd"
        command tmux -S "$sock" set-environment -t "$s" AE_MODE "$(grep '^mode=' "$meta" | cut -d= -f2-)"
        command tmux -S "$sock" set-environment -t "$s" AE_HOME "$aehome"
    done
    sleep 1
    led live-topology-built "sessions=$*" "agent_binary=$FAKE_BIN" "socket=$sock" \
        "session_env=AE_SESSION/AE_ORIGIN/AE_DIR/AE_MODE/AE_HOME per ae@72c7293:17311-17318"
}

HOOKED_AE=/tmp/aecx/hooked/ae
HOOK_PATCH=/tmp/aecx/hooked/hook.patch

# INACTIVE-HOOK EQUIVALENCE, proven PER FIXTURE (cluster-plan global rule).
# The clock is frozen by the date shim for the whole pass so run-to-run volatility
# (generated_at, "active Ns ago") cannot masquerade as a binary difference; a
# CONTROL-CONTROL pass runs first and records the residual volatility floor, so the
# control-vs-hooked comparison is read against a measured baseline rather than an
# assumption. Any inactive divergence outside that floor INVALIDATES the run.
hook_inactive_equiv() { # <ae-home> <sock-or-empty> <session>
    local aehome="$1" sock="$2" sess="$3"
    local out="$ACAP/hook-inactive-equivalence.txt"
    local wd="$ARM_TMUXTMP/hookeq"; rm -rf "$wd"; mkdir -p "$wd"
    led hook-inactive-equivalence-START "unmodified_sha256=$(sha "$FROZEN_AE")" \
        "hooked_sha256=$(sha "$HOOKED_AE")" "patch_sha256=$(sha "$HOOK_PATCH")"
    local -a INV=(
        "list" "list|--json" "list|--all" "list|--all|--json" "next" "status|$sess"
    )
    local -a pre=(env -i "HOME=$(dirname "$aehome")" "AE_HOME=$aehome"
        "PATH=/tmp/aecx/shim:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color"
        "TMUX_TMPDIR=${ARM_TMUXTMP}" "AE_REAL_DATE=/bin/date" "AE_FAKE_NOW=1787000000")
    [[ -n "$sock" ]] && pre+=("AE_TMUX_SERVER=$sock" "AE_TMUX_SERVER_KIND=socket")
    local pass bin i inv
    for pass in ctlA ctlB hooked; do
        case "$pass" in ctlA|ctlB) bin="$FROZEN_AE" ;; hooked) bin="$HOOKED_AE" ;; esac
        for inv in "${INV[@]}"; do
            local IFSOLD="$IFS"; IFS='|'; local -a argv=($inv); IFS="$IFSOLD"
            local tag="${inv//|/_}"
            "${pre[@]}" "$HARNESS_BASH" "$bin" "${argv[@]}" </dev/null \
                >"$wd/$pass.$tag.out" 2>"$wd/$pass.$tag.err"; echo $? >"$wd/$pass.$tag.rc"
        done
    done
    local floor=0 diverge=0
    { echo "## inactive-hook equivalence, per fixture"
      echo "case=${ACASE:-?} clone_mode=${AMODE:-?} session=$sess"
      echo "unmodified=$FROZEN_AE sha256=$(sha "$FROZEN_AE")"
      echo "hooked=$HOOKED_AE sha256=$(sha "$HOOKED_AE")"
      echo "patch=$HOOK_PATCH sha256=$(sha "$HOOK_PATCH") (added lines only; see the patch file)"
      echo "AE_HOOK is UNSET for every invocation in this pass"
      echo "clock frozen at AE_FAKE_NOW=1787000000 via the PATH-first date shim, so run-to-run"
      echo "volatility cannot be mistaken for a binary difference"
      echo
      for inv in "${INV[@]}"; do
          local tag="${inv//|/_}"
          local ca cb ch
          ca="$(sha "$wd/ctlA.$tag.out")$(sha "$wd/ctlA.$tag.err")$(cat "$wd/ctlA.$tag.rc")"
          cb="$(sha "$wd/ctlB.$tag.out")$(sha "$wd/ctlB.$tag.err")$(cat "$wd/ctlB.$tag.rc")"
          ch="$(sha "$wd/hooked.$tag.out")$(sha "$wd/hooked.$tag.err")$(cat "$wd/hooked.$tag.rc")"
          printf 'invocation=ae %s\n' "${inv//|/ }"
          printf '  control-A vs control-B : %s\n' "$( [[ "$ca" == "$cb" ]] && echo IDENTICAL || { echo DIFFERS; })"
          printf '  control-A vs hooked    : %s\n' "$( [[ "$ca" == "$ch" ]] && echo IDENTICAL || echo DIFFERS)"
          printf '  rc control=%s hooked=%s   stdout_bytes control=%s hooked=%s\n' \
              "$(cat "$wd/ctlA.$tag.rc")" "$(cat "$wd/hooked.$tag.rc")" \
              "$(stat -f %z "$wd/ctlA.$tag.out")" "$(stat -f %z "$wd/hooked.$tag.out")"
          [[ "$ca" == "$cb" ]] || floor=$((floor+1))
          [[ "$ca" == "$ch" ]] || diverge=$((diverge+1))
          if [[ "$ca" != "$ch" ]]; then
              echo "  --- control-vs-hooked stdout diff ---"
              diff "$wd/ctlA.$tag.out" "$wd/hooked.$tag.out" | sed 's/^/    /' | head -40
          fi
      done
      echo
      echo "control_control_divergences=$floor (the measured run-to-run volatility floor)"
      echo "control_hooked_divergences=$diverge"
      echo "verdict_free_note=no interpretation is offered here; the byte comparisons are the record"
    } >"$out"
    led hook-inactive-equivalence-COMPLETE "control_control_divergences=$floor" \
        "control_hooked_divergences=$diverge" "artifact_sha256=$(sha "$out")"
    (( diverge == 0 && floor == 0 ))
}

# WRITE WITNESS. The content manifest hashes bytes, so a regeneration that rewrites a file
# with IDENTICAL bytes is invisible to it — which is exactly what `ae <name>` does to the
# generated helper set on resume. This records inode, mtime and size per path, so a
# byte-identical rewrite still shows as a changed inode or mtime. Used ALONGSIDE the content
# manifest, never instead of it: one answers "did the bytes change", the other "was it
# written".
dir_witness() { # <dir>
    ( cd "$1" && find . -mindepth 1 -print0 | sort -z |
      while IFS= read -r -d '' p; do
          printf '%s\t%s\t%s\t%s\n' \
            "$(stat -f %i "$p" 2>/dev/null || echo -)" \
            "$(stat -f %m "$p" 2>/dev/null || echo -)" \
            "$(stat -f %z "$p" 2>/dev/null || echo -)" \
            "$p"
      done )
}
