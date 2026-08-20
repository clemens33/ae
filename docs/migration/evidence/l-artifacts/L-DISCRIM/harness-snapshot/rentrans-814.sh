#!/opt/homebrew/bin/bash
# L-RENTRANS PARTIAL: endpoint-validation (SC-814) — hostile session name,
# PUSH and PULL subarms, with a POSITIVE zero-invocation capture for ssh and
# rsync. Transport-free by construction: the name is refused before the probe.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

TOPOLOGY="one stopped --local session (tf1) plus a hostile name that never becomes a session"

logs_state() { # <label>
    { printf 'label\t%s\n' "$1"
      printf 'ssh.log.exists\t%s\n'   "$( [[ -e "$R/cap/ssh.log" ]] && echo yes || echo no )"
      printf 'ssh.log.lines\t%s\n'    "$(grep -c . "$R/cap/ssh.log" 2>/dev/null || echo 0)"
      printf 'rsync.log.exists\t%s\n' "$( [[ -e "$R/cap/rsync.log" ]] && echo yes || echo no )"
      printf 'rsync.log.lines\t%s\n'  "$(grep -c . "$R/cap/rsync.log" 2>/dev/null || echo 0)"
      printf '--- ssh.log contents ---\n'; cat "$R/cap/ssh.log" 2>/dev/null
      printf '--- rsync.log contents ---\n'; cat "$R/cap/rsync.log" 2>/dev/null
    } >"$R/cap/shim-invocations.$1.txt" 2>&1
    return 0
}

arm_814() { # <push|pull>
    local dir="$1"
    local arm="endpoint-validation-hostile-name-$dir"
    l_arm_begin L-RENTRANS "$arm" frozen
    PATCHV="none (frozen, unmodified)"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    # delegate-and-log ssh and rsync shims: argv passed through UNCHANGED
    cp /tmp/aelx/lib/sshshim.sh "$R/b/ssh"; cp /tmp/aelx/lib/rsyncshim.sh "$R/b/rsync"
    chmod 0755 "$R/b/ssh" "$R/b/rsync"
    HOOKS=""; BLOCK=""; l_arm_env "AE_L_SSH_LOG=$R/cap/ssh.log" "AE_L_RSYNC_LOG=$R/cap/rsync.log"
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local tf1
    sleep 3
    l_arm_preflight tf1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    l_ae 0stop stop -y tf1
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/1pre.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/1pre.tmux.txt"

    # ── LIVE-SHIM CANARY, first, so a later empty log is a MEASUREMENT and not
    #    an absence: a VALID name reaches the SSH probe and the shim must record it.
    local -a fl=(); [[ "$dir" == pull ]] && fl=(--pull)
    l_ae 2canary transfer tf1 nosuchpeer.invalid ${fl[@]+"${fl[@]}"} -y
    logs_state 2after-canary
    local CANARY_SSH; CANARY_SSH="$(grep -c . "$R/cap/ssh.log" 2>/dev/null || echo 0)"
    if [[ "$CANARY_SSH" == 0 ]]; then
        printf 'ARM INVALID: the live-shim canary recorded ZERO ssh invocations, so an empty log in the measurement below would prove nothing about the product.\n' >"$R/cap/ARM-INVALID.txt"
    fi

    # ── reset the logs, then the MEASUREMENT: a hostile session name ──────────
    rm -f "$R/cap/ssh.log" "$R/cap/rsync.log"
    logs_state 3before-measurement
    local HOSTILE='../victim'
    printf '%s' "$HOSTILE" >"$R/cap/hostile-name.raw"
    printf '%s' "$HOSTILE" | od -c >"$R/cap/hostile-name.od.txt"
    l_ae 4measure transfer "$HOSTILE" nosuchpeer.invalid ${fl[@]+"${fl[@]}"} -y
    logs_state 4after-measurement

    # a second hostile shape: quoting and command substitution in the name
    rm -f "$R/cap/ssh.log" "$R/cap/rsync.log"
    local HOSTILE2='x'"'"'"$(touch SENTINEL_TOUCHED)"'"'"'y'
    printf '%s' "$HOSTILE2" >"$R/cap/hostile-name2.raw"
    printf '%s' "$HOSTILE2" | od -c >"$R/cap/hostile-name2.od.txt"
    find "$R" -name SENTINEL_TOUCHED >"$R/cap/sentinel.before.txt" 2>&1
    l_ae 5measure2 transfer "$HOSTILE2" nosuchpeer.invalid ${fl[@]+"${fl[@]}"} -y
    logs_state 5after-measurement2
    find "$R" -name SENTINEL_TOUCHED >"$R/cap/sentinel.after.txt" 2>&1
    { printf 'sentinel.name\tSENTINEL_TOUCHED\nscan.root\t%s (recursive)\n' "$R"
      printf 'count.before\t%s\n' "$(grep -c . "$R/cap/sentinel.before.txt")"
      printf 'count.after\t%s\n'  "$(grep -c . "$R/cap/sentinel.after.txt")"; } >"$R/cap/sentinel-state.txt"

    l_manifest "$R/h/.ae" "$R/cap/6post.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/6post.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/6post.tmux.txt"
    diff -u "$R/cap/1pre.aehome.tsv" "$R/cap/6post.aehome.tsv" >"$R/cap/aehome.before-after.diff" 2>&1
    { printf '# BOTH ENDPOINTS. This host is both endpoints in this section: there is no\n'
      printf '# remote side, because no transport was reached. The local side is captured\n'
      printf '# above; the would-be remote side is the same filesystem and is captured by\n'
      printf '# the same manifests. Recorded so the single-endpoint scope is explicit.\n'
      printf '\n# sessions root, full listing\n'; ls -1a "$R/h/.ae/sessions" 2>&1
      printf '\n# one level ABOVE the sessions root (the path a traversal name would reach)\n'; ls -1a "$R/h/.ae" 2>&1
    } >"$R/cap/endpoints.txt" 2>&1

    { printf 'arm\t%s\nsection\tL-RENTRANS (PARTIAL — transport-free subset under a BLOCKED transport gate)\n' "$arm"
      printf 'roster_ids\tSC-814\n'
      printf 'direction\t%s\n' "$dir"
      printf 'construction\tae transfer runs with a HOSTILE SESSION NAME, %s subarm. Frozen ae validates the session name and the path object at step 1, before the SSH probe at step 4 and before the rsync capability gate — so this arm is transport-free by construction\n' "$dir"
      printf 'hostile_names\t../victim  (path traversal, the class the frozen comment names) and a second carrying quoting and command substitution with an embedded sentinel\n'
      printf 'shims\tdelegate-and-log ssh and rsync, argv passed through UNCHANGED\n'
      printf 'live_shim_canary\ta VALID-name transfer runs FIRST and must record ssh invocations; the logs are then RESET before the hostile-name measurement, so an empty log afterwards is a measurement and not an absence\n'
      printf 'canary_ssh_invocations\t%s\n' "$CANARY_SSH"
      printf 'canary_rc\t%s\n' "$(cat "$R/cap/2canary.rc")"
      printf 'measure1_rc\t%s\n' "$(cat "$R/cap/4measure.rc")"
      printf 'measure2_rc\t%s\n' "$(cat "$R/cap/5measure2.rc")"
      printf 'ssh_invocations_after_measure1\t%s\n' "$(grep '^ssh.log.lines' "$R/cap/shim-invocations.4after-measurement.txt" | cut -f2)"
      printf 'rsync_invocations_after_measure1\t%s\n' "$(grep '^rsync.log.lines' "$R/cap/shim-invocations.4after-measurement.txt" | cut -f2)"
      printf 'ssh_invocations_after_measure2\t%s\n' "$(grep '^ssh.log.lines' "$R/cap/shim-invocations.5after-measurement2.txt" | cut -f2)"
      printf 'rsync_invocations_after_measure2\t%s\n' "$(grep '^rsync.log.lines' "$R/cap/shim-invocations.5after-measurement2.txt" | cut -f2)"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'topology\t%s\n' "$TOPOLOGY"
    } >"$R/cap/ARM.txt"
    l_arm_end
}

case "${1:-all}" in
  push) arm_814 push ;;
  pull) arm_814 pull ;;
esac
echo DONE
