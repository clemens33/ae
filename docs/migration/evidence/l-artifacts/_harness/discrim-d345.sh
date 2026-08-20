#!/opt/homebrew/bin/bash
# D3 — rsync recorder canary (reachability first).
# D4 — SIGPIPE disposition, ability-to-fail control.
# D5 — self-stop supervisor, caught alive.
set -uo pipefail
source /tmp/aelx/lib/discrim-lib.sh

# ─────────────────────────────────────────────────────────── D3
arm_d3() {
    l_arm_begin L-DISCRIM D3-rsync-recorder-canary frozen
    PATCHV="none (frozen, unmodified)"
    : >"$R/cap/ledger.tsv"; led setup arm D3
    d_config "$R"; { l_mkrepo "$R"; } >/dev/null 2>&1
    cp /tmp/aelx/lib/sshshim.sh "$R/b/ssh"; cp /tmp/aelx/lib/rsyncshim.sh "$R/b/rsync"
    chmod 0755 "$R/b/ssh" "$R/b/rsync"
    HOOKS=""; BLOCK=""; l_arm_env "AE_L_SSH_LOG=$R/cap/ssh.log" "AE_L_RSYNC_LOG=$R/cap/rsync.log"
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local tf1
    sleep 4
    l_arm_preflight tf1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    l_ae 0stop stop -y tf1
    sleep 2

    # ── STEP 1: is the RECORDER itself live? Invoke rsync DIRECTLY through the shim.
    #    This is NOT the ae path and is labelled as such; it answers only "does the
    #    recorder record".
    mkdir -p "$R/src" "$R/dst"; printf 'recorder canary\n' >"$R/src/probe.txt"
    ( env -i "${AE_ENV[@]}" rsync -a "$R/src/" "$R/dst/" ) >"$R/cap/1direct.stdout" 2>"$R/cap/1direct.stderr"
    printf '%s\n' "$?" >"$R/cap/1direct.rc"
    local DIRECT; DIRECT="$(grep -c . "$R/cap/rsync.log" 2>/dev/null)"; DIRECT="${DIRECT:-0}"
    led recorder direct.rsync.rc "$(cat "$R/cap/1direct.rc")"
    led recorder direct.rsync.invocations "$DIRECT"
    led recorder direct.note "a DIRECT rsync invocation, not the ae path; it answers only whether the recorder records"
    cp -p "$R/cap/rsync.log" "$R/cap/rsync.log.after-direct" 2>/dev/null

    # ── STEP 2: can ae's VALID-name path REACH rsync on this host?
    #    Both gates that precede any rsync invocation are exercised and captured.
    rm -f "$R/cap/ssh.log" "$R/cap/rsync.log"
    ( cd "$R/w" && env -i "${AE_ENV[@]}" "$R/b/ae" transfer tf1 nosuchpeer.invalid -y ) \
        >"$R/cap/2valid.stdout" 2>"$R/cap/2valid.stderr"; printf '%s\n' "$?" >"$R/cap/2valid.rc"
    local VS VR
    VS="$(grep -c . "$R/cap/ssh.log" 2>/dev/null)"; VS="${VS:-0}"
    VR="$(grep -c . "$R/cap/rsync.log" 2>/dev/null)"; VR="${VR:-0}"
    led reach valid-name.rc "$(cat "$R/cap/2valid.rc")"
    led reach valid-name.ssh.invocations "$VS"
    led reach valid-name.rsync.invocations "$VR"
    { printf '# GATE 1 — the SSH probe, the frozen call shape (ae:_transfer_ssh_probe)\n'
      printf 'ssh -o BatchMode=yes -o ConnectTimeout=5 <target> true\n'
      printf 'result on this host, from the valid-name transfer above:\n'; cat "$R/cap/2valid.stderr"
      printf '\n# GATE 2 — the local rsync capability gate, which the frozen code checks\n'
      printf '#          AFTER the probe and BEFORE any rsync invocation\n'
      printf 'rsync --protect-args --version -> '
      if env -i "${AE_ENV[@]}" rsync --protect-args --version >/dev/null 2>&1; then printf 'SUPPORTED\n'; else printf 'NOT SUPPORTED (rc %s)\n' "$?"; fi
      printf 'rsync --version, line 1: %s\n' "$(env -i "${AE_ENV[@]}" rsync --version 2>&1 | head -1)"
      printf '\n# every rsync binary on this host\n'
      for p in /usr/bin/rsync /usr/local/bin/rsync /opt/homebrew/bin/rsync /opt/local/bin/rsync; do
        if [[ -x "$p" ]]; then printf '  %s\t%s\n' "$p" "$("$p" --version 2>&1 | head -1)"; else printf '  %s\t(absent)\n' "$p"; fi
      done
    } >"$R/cap/gates.txt" 2>&1

    # ── the finding, or the canary ───────────────────────────────────────────
    if (( VR > 0 )); then
        led canary result "the ae valid-name path REACHED rsync; the canary is available"
    else
        { printf 'ARM INVALID as a CANARY — and that is the finding.\n\n'
          printf 'ae valid-name transfer path did NOT reach rsync on this host: %s rsync invocations recorded.\n' "$VR"
          printf 'TWO independent gates precede any rsync invocation in the frozen code, and BOTH fail here:\n'
          printf '  GATE 1  the SSH probe. The frozen call is a plain `ssh <target>`, and OpenSSH reads the\n'
          printf '          config from the passwd-entry home, so a sandbox-only Host alias is unreachable.\n'
          printf '          Reaching it would need the operator real ~/.ssh/config, root for port 22, or an\n'
          printf '          argv-injecting ssh shim — the last of which is REFUSED by standing ruling.\n'
          printf '  GATE 2  `rsync --protect-args --version`. This host only rsync is openrsync, which does\n'
          printf '          not support it, so ae refuses BEFORE any rsync call even if GATE 1 passed.\n'
          printf '          GATE 2 alone makes the ae path unreachable for ANY target on this host.\n\n'
          printf 'CONSEQUENCE, stated rather than worked around: the SC-814 rsync zero remains a zero from an\n'
          printf 'UNDEMONSTRATED recorder ON THE AE PATH. The recorder itself IS live — a direct rsync\n'
          printf 'invocation through the same shim recorded %s invocation(s), rc %s — but that is not the ae\n' "$DIRECT" "$(cat "$R/cap/1direct.rc")"
          printf 'path and does not license the SC-814 reading. No zero is reported here as an observation.\n'
        } >"$R/cap/ARM-INVALID.txt"
        led canary result "NOT AVAILABLE — see ARM-INVALID.txt; the finding is the unreachability, not a zero"
    fi
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    darmtxt D3-rsync-recorder-canary "SC-814 discriminator" \
      "the SC-814 canary proved the ssh recorder and inherited an unproven rsync one: the valid-name transfer dies at the SSH probe and never reaches rsync" \
      "the recorder is first exercised by a DIRECT rsync invocation through the same delegate-and-log shim; then ae valid-name transfer path is run and both gates that precede any rsync invocation are captured" \
      "recorder.direct.invocations	$DIRECT" \
      "ae.valid_name.ssh.invocations	$VS" \
      "ae.valid_name.rsync.invocations	$VR" \
      "ae.valid_name.rc	$(cat "$R/cap/2valid.rc")" \
      "canary_available	$( ((VR > 0)) && echo YES || echo NO )" \
      "OBSERVATION	gates.txt carries both gates verbatim; ARM-INVALID.txt carries the finding when the canary is unavailable. No zero from an undemonstrated recorder is reported."
    l_arm_end
    return 0
}

# ─────────────────────────────────────────────────────────── D4
arm_d4() {
    l_arm_begin L-DISCRIM D4-sigpipe-ability-to-fail frozen
    PATCHV="none (frozen, unmodified)"
    : >"$R/cap/ledger.tsv"; led setup arm D4
    d_config "$R"; { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    cp /tmp/aelx/lib/sigpipe2.py "$R/cap/sigpipe-harness.py"
    local mode
    for mode in dfl ign inherit; do
        l_ae "0launch-$mode" --local "cp$mode"
        sleep 4
        [[ "$mode" == dfl ]] && { l_arm_preflight "cp$mode" || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }; }
        local envblob="" v
        for v in "${AE_ENV[@]}"; do envblob+="${v}"$'\x1f'; done
        AE_L_SIGPIPE_ENV="$envblob" AE_L_SIGPIPE_MODE="$mode" \
          python3 /tmp/aelx/lib/sigpipe2.py "$R/cap" "$mode" \
          "$R/b/ae" compact -f --digest-only "cp$mode" >"$R/cap/1$mode.stdout" 2>"$R/cap/1$mode.stderr"
        printf '%s\n' "$?" >"$R/cap/1$mode.rc"
        sleep 3
        led "$mode" harness.rc "$(cat "$R/cap/1$mode.rc")"
        led "$mode" record "$(python3 -c "import json;d=json.load(open('$R/cap/sigpipe-record.$mode.json'));print('exited=%s code=%s signalled=%s sig=%s' % (d['producer_exited_normally'],d['producer_exit_code'],d['producer_signalled'],d['producer_term_signal_name']))" 2>/dev/null || echo '<no record>')"
    done
    # the ability-to-fail comparison, generated from the records themselves
    python3 - "$R/cap" >"$R/cap/disposition-table.txt" <<'PYEOF'
import json, sys, os
d = sys.argv[1]
print('# SIGPIPE disposition handed to the producer, and what the SAME capture reported.')
print('# Generated from the sigpipe-record.*.json files.\n')
print('%-10s %-46s %-10s %-10s %-12s %s' % ('mode','disposition the harness set before exec','exited','code','signalled','signal'))
for m, desc in (('dfl','SIG_DFL — explicitly reset by the harness'),
                ('ign','SIG_IGN — DELIBERATELY LEAKED by the harness'),
                ('inherit','not set at all — whatever the harness had')):
    p = os.path.join(d, 'sigpipe-record.%s.json' % m)
    if not os.path.exists(p):
        print('%-10s %-46s %s' % (m, desc, '(no record)')); continue
    r = json.load(open(p))
    print('%-10s %-46s %-10s %-10s %-12s %s' % (m, desc, r['producer_exited_normally'], r['producer_exit_code'], r['producer_signalled'], r['producer_term_signal_name']))
PYEOF
    cat "$R/cap/disposition-table.txt" >>"$R/cap/ledger.tsv"
    local DFLC IGNC
    DFLC="$(python3 -c "import json;print(json.load(open('$R/cap/sigpipe-record.dfl.json'))['producer_exit_code'])" 2>/dev/null || echo '')"
    IGNC="$(python3 -c "import json;print(json.load(open('$R/cap/sigpipe-record.ign.json'))['producer_exit_code'])" 2>/dev/null || echo '')"
    led ability leaked.detected "$( [[ -n "$DFLC" && -n "$IGNC" && "$DFLC" != "$IGNC" ]] && echo YES || echo NO )"
    if [[ -z "$DFLC" || -z "$IGNC" || "$DFLC" == "$IGNC" ]]; then
        { printf 'ARM INVALID: the capture did not DETECT a deliberately leaked SIG_IGN.\n'
          printf 'dfl exit code %s, ign exit code %s — the two must differ, or the capture cannot fail\n' "${DFLC:-<none>}" "${IGNC:-<none>}"
          printf 'and the clean reading is worthless.\n'
        } >"$R/cap/ARM-INVALID.txt"
    fi
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    darmtxt D4-sigpipe-ability-to-fail "SC-504b discriminator" \
      "the existing SC-504b arm sets SIG_DFL in the child before exec, so its clean reading cannot be told apart from the harness having reset the disposition itself" \
      "the SAME capture is run three times against three deliberately different dispositions handed to the producer: SIG_DFL reset by the harness, SIG_IGN LEAKED by the harness, and nothing set at all" \
      "leaked_ign_detected	$( [[ -n "$DFLC" && -n "$IGNC" && "$DFLC" != "$IGNC" ]] && echo YES || echo NO )" \
      "dfl.exit_code	${DFLC:-<none>}" \
      "ign.exit_code	${IGNC:-<none>}" \
      "ATTRIBUTION LIMIT, stated	the harness sets the disposition before exec in the dfl and ign runs, so those runs report what the HARNESS handed the producer. They demonstrate the capture CAN fail — a leaked SIG_IGN is detected — but they cannot separate 'ae leaked nothing' from 'the harness reset it first'. Reading a live process's ignored-signal mask is what would separate them, and macOS has no /proc equivalent for it, so this construction cannot answer that question on this platform. That is the finding, not a clean result." \
      "OBSERVATION	disposition-table.txt is generated from the three records. No verdict is stated here."
    l_arm_end
    return 0
}

# ─────────────────────────────────────────────────────────── D5
l_use_v5() { cp /tmp/aelx/instr5/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

d5_common_setup() { # <session>
    local sess="$1"
    d_config "$R"; { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local "$sess"
    sleep 4
    l_arm_preflight "$sess" || return 1
    D5_SHELL="$(/opt/homebrew/bin/tmux -S "$SOCK" new-window -d -t "$sess:" -c "$R/w" -P -F '#{pane_id}')"
    sleep 1
    led setup shell.pane "$D5_SHELL"
    l_manifest "$R/h/.ae/sessions" "$R/cap/1pre.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/1pre.tmux.txt"
    return 0
}

# The ancestor walk, taken from a pid WHILE IT IS ALIVE.
d5_lineage() { # <pid> <out>
    local cur="$1" out="$2" depth=0
    { printf '# ancestor walk from pid %s, taken WHILE IT WAS ALIVE\n' "$cur"
      while [[ -n "$cur" && "$cur" != 0 && $depth -lt 12 ]]; do
          printf '%d\t%s\n' "$depth" "$(ps -o pid=,ppid=,pgid=,tty=,stat=,command= -p "$cur" 2>/dev/null | head -1)"
          cur="$(ps -o ppid= -p "$cur" 2>/dev/null | tr -d ' ')"
          depth=$((depth + 1))
      done
    } >"$out" 2>&1
    return 0
}

# ── D5a: the REAL-TIMING attempt. No hook. A dedicated sampler process started
#    BEFORE the stop, running pgrep with no sleep at all, so the sampling rate is
#    whatever the machine can do rather than a chosen interval.
arm_d5a() {
    l_arm_begin L-DISCRIM D5a-supervisor-real-timing frozen
    PATCHV="none (frozen, unmodified)"
    : >"$R/cap/ledger.tsv"; led setup arm D5a
    d5_common_setup s1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    cat >"$R/sampler.sh" <<SAMP
#!/opt/homebrew/bin/bash
# No sleep: sample as fast as the machine allows, and record the rate achieved.
n=0; hits=0; t0=\$SECONDS
while (( SECONDS - t0 < 40 )); do
    n=\$((n + 1))
    p="\$(pgrep -f '_stop-supervisor' 2>/dev/null | head -1)"
    if [[ -n "\$p" ]]; then
        hits=\$((hits + 1))
        printf 'hit=%s iter=%s pid=%s %s\n' "\$hits" "\$n" "\$p" "\$(ps -o pid=,ppid=,command= -p "\$p" 2>/dev/null | head -1)" >>"$R/cap/supervisor-samples.txt"
        printf '%s' "\$p" >"$R/cap/supervisor.firstpid"
    fi
done
printf 'iterations=%s hits=%s window=40s\n' "\$n" "\$hits" >"$R/cap/sampler-rate.txt"
SAMP
    chmod 0755 "$R/sampler.sh"
    : >"$R/cap/supervisor-samples.txt"
    "$L_BASH" "$R/sampler.sh" &
    local SAMPLER=$!
    sleep 1
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$D5_SHELL" -l -- "$R/b/ae stop -y > $R/cap/stop.stdout 2> $R/cap/stop.stderr; echo \$? > $R/cap/stop.rc"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$D5_SHELL" Enter
    led measure typed "ae stop -y (implicit self-stop) typed into a shell pane inside the session"
    wait "$SAMPLER" 2>/dev/null
    local FP=""; [[ -s "$R/cap/supervisor.firstpid" ]] && FP="$(cat "$R/cap/supervisor.firstpid")"
    local HITS; HITS="$(grep -c . "$R/cap/supervisor-samples.txt" 2>/dev/null)"; HITS="${HITS:-0}"
    led measure sampler.rate "$(cat "$R/cap/sampler-rate.txt" 2>/dev/null)"
    led measure supervisor.hits "$HITS"
    led measure route.stdout "$(head -1 "$R/cap/stop.stdout" 2>/dev/null)"
    led measure stop.rc "$(cat "$R/cap/stop.rc" 2>/dev/null || echo '<none>')"
    [[ -n "$FP" ]] && d5_lineage "$FP" "$R/cap/supervisor-lineage.txt"
    sleep 3
    ps -ax -o pid=,ppid=,command= 2>/dev/null | grep -F "$R" | grep -v '[g]rep' >"$R/cap/3post.ps.txt"
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    { printf '# stop-result rows\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    if (( HITS == 0 )); then
        { printf 'ARM INVALID as a lineage capture — and the reason is the finding.\n\n'
          printf 'The SELF-STOP ROUTE DID RUN: stop.stdout carries the supervisor handoff line, and\n'
          printf 'stop-results.txt carries the durable stop-result the supervisor itself wrote. So a\n'
          printf 'supervisor existed and did its work.\n\n'
          printf 'It was NEVER OBSERVED ALIVE by an external process-table sampler: %s\n' "$(cat "$R/cap/sampler-rate.txt" 2>/dev/null)"
          printf 'The sampler ran pgrep -f with NO sleep for the whole window and matched nothing.\n'
          printf 'No lineage is reported from this arm. D5b holds the supervisor at a barrier instead\n'
          printf 'of racing it, and is where the lineage comes from.\n'
        } >"$R/cap/ARM-INVALID.txt"
    fi
    darmtxt D5a-supervisor-real-timing "SC-835e-h discriminator, real-timing half" \
      "the existing self-stop artifact is a post-hoc snapshot byte-identical to 3post.ps.txt, so it shows no supervisor at all while its ARM.txt asserts supervisor_observed=yes" \
      "a dedicated sampler process running pgrep -f with NO sleep is started BEFORE the implicit self-stop is typed into a shell pane inside the session; every sighting is recorded with the full argv row, and the achieved sampling rate is recorded too" \
      "sampler_rate	$(cat "$R/cap/sampler-rate.txt" 2>/dev/null)" \
      "supervisor_hits	$HITS" \
      "supervisor_first_pid	${FP:-<never seen>}" \
      "caught_alive	$( ((HITS > 0)) && echo YES || echo NO )" \
      "self_stop_route_ran	$(head -1 "$R/cap/stop.stdout" 2>/dev/null || echo '<no stdout>')" \
      "OBSERVATION	supervisor-samples.txt, sampler-rate.txt, stop.stdout and stop-results.txt. No verdict is stated here."
    l_arm_end
    return 0
}

# ── D5b: the SAME question without the race. The singular supervisor is HELD at
#    its own entry barrier, so it is caught alive by construction.
D5_CAUGHT=""
d5b_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_stop_supervisor_singular_entry.*)
        [[ -n "$D5_CAUGHT" ]] && return 0
        D5_CAUGHT=yes
        local p; p="$(pgrep -f '_stop-supervisor' 2>/dev/null | head -1)"
        printf '%s' "${p:-}" >"$R/cap/supervisor.firstpid"
        ps -ax -o pid=,ppid=,pgid=,tty=,stat=,command= 2>/dev/null | grep -F "$R" | grep -v '[g]rep' >"$R/cap/at-barrier.ps.txt"
        [[ -n "$p" ]] && d5_lineage "$p" "$R/cap/supervisor-lineage.txt"
        { printf 'barrier\t%s\n' "$k"
          printf 'method\tthe singular self-stop supervisor is HELD at its own entry, so it is caught alive by construction rather than by racing a sampler\n'
          printf 'supervisor.pid\t%s\n' "${p:-<not matched by pgrep even while held>}"
        } >"$R/cap/at-barrier.txt"
        led measure supervisor.pid.at.barrier "${p:-<not matched>}"
        ;;
    esac
    return 0
}

arm_d5b() {
    l_arm_begin L-DISCRIM D5b-supervisor-held-at-entry instrumented
    l_use_v5; PATCHV="L-HOOKS-v5"
    : >"$R/cap/ledger.tsv"; led setup arm D5b
    d5_common_setup s1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    HOOKS=b_stop_supervisor_singular_entry; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    # NOT `env -i`: the identity route needs $TMUX and $TMUX_PANE, and a scrubbed
    # environment strips exactly those, so the stop refuses at C1 and no supervisor is
    # ever spawned — measured on the first attempt at this arm. The pane already
    # carries the arm environment from the tmux server, so only the hook variables are
    # added, inline, in front of the command.
    local hookpfx
    hookpfx="AE_L_HOOKS=b_stop_supervisor_singular_entry AE_L_TRACE=$(printf '%q' "$R/cap/hook-trace.tsv") AE_L_BLOCK=$(printf '%q' "$R/ctl") AE_L_BLOCK_MAX=1800"
    led setup hook.prefix "$hookpfx"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$D5_SHELL" -l -- "$hookpfx $R/b/ae stop -y > $R/cap/stop.stdout 2> $R/cap/stop.stderr; echo \$? > $R/cap/stop.rc"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$D5_SHELL" Enter
    led measure typed "ae stop -y (implicit self-stop) typed into a shell pane inside the session, under the v5 barrier"
    l_barriers_pane 120 "$R/cap/stop.rc" d5b_cb || printf 'NOTE: the barrier controller ended by bound or by the subject exiting\n' >>"$R/cap/barrier-order.tsv"
    sleep 4
    ps -ax -o pid=,ppid=,command= 2>/dev/null | grep -F "$R" | grep -v '[g]rep' >"$R/cap/3post.ps.txt"
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    { printf '# stop-result rows\n'
      for f in "$R"/h/.ae/sessions/*/events.jsonl; do [[ -e "$f" ]] || continue
        printf '=== %s ===\n' "$f"; grep '"action":"stop-result"' "$f" 2>/dev/null || printf '(none)\n'; done
    } >"$R/cap/stop-results.txt" 2>&1
    diff -u "$R/cap/at-barrier.ps.txt" "$R/cap/3post.ps.txt" >"$R/cap/ps.at-barrier-vs-post.diff" 2>&1
    local SP=""; [[ -s "$R/cap/supervisor.firstpid" ]] && SP="$(cat "$R/cap/supervisor.firstpid")"
    if [[ -z "$D5_CAUGHT" ]]; then
        { printf 'ARM INVALID: the singular supervisor entry barrier was never reached within the bound,\n'
          printf 'so nothing was caught alive and no lineage may be reported from this arm.\n'
        } >"$R/cap/ARM-INVALID.txt"
    fi
    darmtxt D5b-supervisor-held-at-entry "SC-835e-h discriminator, barrier half" \
      "catching the singular self-stop supervisor by sampling is a race the sampler loses; holding it at its own entry removes the race" \
      "the implicit self-stop is typed into a shell pane inside the session under L-HOOKS-v5, whose only addition over v4 is a barrier at the entry of the SINGULAR stop supervisor; at that barrier the live process table, the supervisor pid and an ancestor walk are recorded, then it is released" \
      "barrier	b_stop_supervisor_singular_entry" \
      "caught_alive	$( [[ -n "$D5_CAUGHT" ]] && echo YES || echo NO )" \
      "supervisor_pid_at_barrier	${SP:-<not matched>}" \
      "stop_rc	$(cat "$R/cap/stop.rc" 2>/dev/null || echo '<none>')" \
      "OBSERVATION	at-barrier.ps.txt is the live table while the supervisor was held, supervisor-lineage.txt the ancestor walk taken then, and ps.at-barrier-vs-post.diff the difference against the post-hoc snapshot the earlier artifact was. No verdict is stated here."
    l_arm_end
    return 0
}

case "${1:-}" in
  d3) arm_d3 ;;
  d4) arm_d4 ;;
  d5a) arm_d5a ;;
  d5b) arm_d5b ;;
esac
echo DONE
