#!/opt/homebrew/bin/bash
# B0 Design 7 (SC-511c) product-layer family runners.
# Each runner drives a FROZEN product surface and captures stdout/stderr/rc/files.
# No runner interprets anything. Every invocation's exact argv is recorded.
set -uo pipefail
SB=/tmp/aeb0
D7="$SB/d7"
AE="$SB/frozen/ae"
S=b0d7
ARM_PATH="$D7/shim:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
CLOCK_CONSUMER=1787191200      # pinned "now" for every consumer run (fixture origin + 2h)

_now() { printf '%s|%s' "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" "${EPOCHREALTIME:-na}"; }

_timeout() { # <secs> <cmd...> -> 124 on expiry
    local secs="$1"; shift
    "$@" & local pid=$! i=0
    while (( i < secs * 10 )); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; i=$((i+1)); done
    if kill -0 "$pid" 2>/dev/null; then kill -TERM "$pid" 2>/dev/null; sleep 0.5; kill -KILL "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; return 124; fi
    wait "$pid"
}

# ── environment for one family run ────────────────────────────────────────────
_env() { # <clonedir> <srv> <shimlog>
    local home="$1" srv="$2" shimlog="$3"
    printf '%s\n' \
        "PATH=$ARM_PATH" "HOME=$home" "AE_HOME=$home/.ae" "TERM=dumb" \
        "TZ=UTC" "LANG=en_US.UTF-8" \
        "TMUX_TMPDIR=$home/../tmuxtmp" "AE_TMUX_SERVER=$srv" "AE_TMUX_SERVER_KIND=name" \
        "AE_DATE_REAL=/bin/date" "AE_DATE_SHIM_STATE=$home/../clock" \
        "AE_DATE_SHIM_LOG=$shimlog" "AE_DATE_SHIM_SUBSTITUTE=${AE_DATE_SHIM_SUBSTITUTE:-1}"
}

_run() { # <outdir> <label> <clonedir> <srv> -- <cmd...>
    local out="$1" label="$2" home="$3" srv="$4"; shift 5
    mkdir -p "$out"
    local shimlog="$out/date-shim.$label.log"; : >"$shimlog"
    local -a e; mapfile -t e < <(_env "$home" "$srv" "$shimlog")
    { printf 'label=%s\n' "$label"; printf 'started=%s\n' "$(_now)"
      printf 'argv:'; printf ' %q' "$@"; printf '\n'
      printf 'env (allowlisted):\n'; printf '  %s\n' "${e[@]}"; } > "$out/$label.invocation.txt"
    local rc=0
    _timeout "${RUN_TIMEOUT:-90}" env -i "${e[@]}" "$@" >"$out/$label.stdout.txt" 2>"$out/$label.stderr.txt" || rc=$?
    printf '%s\n' "$rc" > "$out/$label.rc.txt"
    printf 'finished=%s rc=%s\n' "$(_now)" "$rc" >> "$out/$label.invocation.txt"
    return 0
}

_prep() { # <armroot> -> fresh clone + tmuxtmp + pinned clock
    local root="$1"
    rm -rf "$root"; mkdir -p "$root/tmuxtmp" "$root/home"
    cp -a "$D7/template/.ae" "$root/home/.ae"
    printf '%s\n' "$CLOCK_CONSUMER" > "$root/clock"
    "$SB/bin/manifest.sh" "$root/home/.ae" > "$root/manifest.before.tsv"
    shasum -a 256 "$root/manifest.before.tsv" | awk '{print $1}' > "$root/clone-fingerprint.sha256"
}

_post() { # <armroot>
    local root="$1"
    "$SB/bin/manifest.sh" "$root/home/.ae" > "$root/manifest.after.tsv"
    diff -u "$root/manifest.before.tsv" "$root/manifest.after.tsv" > "$root/manifest.delta.diff" || true
    cp -a "$root/home/.ae/sessions/$S/events.jsonl" "$root/events.after.jsonl" 2>/dev/null || true
}

_tmuxsnap() { # <armroot> <srv> <label>
    local root="$1" srv="$2" label="$3"
    { echo "# tmux snapshot ($label) server -L $srv"
      TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" list-sessions 2>&1; echo "list-sessions rc=$?"
      TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" list-windows -a -F '#{window_id} #{session_name} #{window_name}' 2>&1; echo "list-windows rc=$?"
      TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" list-panes -a -F '#{pane_id} #{session_name} #{@ae_agent} #{@ae_slot} #{pane_current_command}' 2>&1; echo "list-panes rc=$?"
      TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" list-clients 2>&1; echo "list-clients rc=$?"
    } > "$root/tmux.$label.txt" 2>&1
}

_resume() { # <armroot> <srv> -> resume the fixture session; 0 ok, 1 timeout
    local root="$1" srv="$2"
    local -a e; mapfile -t e < <(_env "$root/home" "$srv" "$root/date-shim.resume.log")
    ( env -i "${e[@]}" bash -c "cd '$D7/repo' && exec '$AE' --local $S" >"$root/resume.out" 2>&1 & ) 
    local i=0
    while ! TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" has-session -t "=$S" 2>/dev/null; do
        sleep 0.5; i=$((i+1)); (( i < 60 )) || { echo "INCONCLUSIVE: resume timeout" > "$root/INCONCLUSIVE.resume.txt"; return 1; }
    done
    sleep 3
    return 0
}

_panes() { # <armroot> <srv> <agent> -> pane id
    TMUX_TMPDIR="$1/tmuxtmp" command tmux -L "$2" list-panes -s -t "=$S" -F '#{pane_id} #{@ae_agent}' 2>/dev/null | awk -v a="$3" '$2==a{print $1; exit}'
}

# ── the nine families ─────────────────────────────────────────────────────────
fam_list_next() { local root="$1" srv="$2"
    _resume "$root" "$srv" || return 0
    _tmuxsnap "$root" "$srv" before
    _run "$root" list       "$root/home" "$srv" -- "$AE" list
    _run "$root" list-json  "$root/home" "$srv" -- "$AE" list --json
    RUN_TIMEOUT=15 _run "$root" next "$root/home" "$srv" -- "$AE" next
    _tmuxsnap "$root" "$srv" after
}

fam_watchdog() { local root="$1" srv="$2"
    _resume "$root" "$srv" || return 0
    local sd="$root/home/.ae/sessions/$S"
    _tmuxsnap "$root" "$srv" before
    # The GENERATED watchdog script, run directly at its own `_run` verb so the
    # documented AE_WATCHDOG_* knobs actually reach the loop (a `watchdog start`
    # launches the loop into a tmux pane and the caller's env does not follow it —
    # measured: the pane banner reported the DEFAULTS). Not function-sourcing: the
    # generated script is executed as a program. Bounded by SIGTERM after the window,
    # then `watchdog stop`. Every knob value is recorded.
    { printf 'runner: %s _run  (the generated watchdog script, executed)\n' "$sd/watchdog"
      printf 'knobs: AE_WATCHDOG_INTERVAL_SEC=2 AE_WATCHDOG_STALE_MIN=1 AE_WATCHDOG_MAX_NUDGES=1 AE_WATCHDOG_THROTTLE_ALERT_CYCLES=2\n'
      printf 'window: SIGTERM after 15s (~7 cycles at INTERVAL=2)\n'; } > "$root/watchdog.knobs.txt"
    local shimlog="$root/date-shim.watchdog.log"; : >"$shimlog"
    local -a e; mapfile -t e < <(AE_DATE_SHIM_SUBSTITUTE=0 _env "$root/home" "$srv" "$shimlog")
    e+=("AE_WATCHDOG_INTERVAL_SEC=2" "AE_WATCHDOG_STALE_MIN=1" "AE_WATCHDOG_MAX_NUDGES=1" "AE_WATCHDOG_THROTTLE_ALERT_CYCLES=2")
    { printf 'argv:'; printf ' %q' "$sd/watchdog" _run; printf '\n'
      printf 'env (allowlisted):\n'; printf '  %s\n' "${e[@]}"; } > "$root/watchdog-run.invocation.txt"
    local rc=0
    _timeout 15 env -i "${e[@]}" "$sd/watchdog" _run >"$root/watchdog-run.stdout.txt" 2>"$root/watchdog-run.stderr.txt" || rc=$?
    printf '%s (124 = the bounded window closed the infinite loop, by construction)\n' "$rc" > "$root/watchdog-run.rc.txt"
    _tmuxsnap "$root" "$srv" running
    RUN_TIMEOUT=20 _run "$root" watchdog-stop "$root/home" "$srv" -- "$sd/watchdog" stop
    for f in watchdog.log loop.log; do [[ -f "$sd/$f" ]] && cp -a "$sd/$f" "$root/watchdog.$f"; done
    _tmuxsnap "$root" "$srv" after
}

fam_requests_state() { local root="$1" srv="$2"
    _resume "$root" "$srv" || return 0
    local sd="$root/home/.ae/sessions/$S"
    local main help
    main="$(_panes "$root" "$srv" dummy:dummy)"; help="$(_panes "$root" "$srv" dummy2:helper)"
    printf 'main_pane=%s helper_pane=%s\n' "$main" "$help" > "$root/panes.txt"
    _run "$root" requests-all "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/requests" all
    _run "$root" requests-mine "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/requests" mine
    _run "$root" requests-inbox "$root/home" "$srv" -- env TMUX_PANE="$help" "$sd/requests" inbox
    _run "$root" state-print "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/state"
    _run "$root" state-print-helper "$root/home" "$srv" -- env TMUX_PANE="$help" "$sd/state"
    # a real reply attempt against the fixture's OPEN review request, from the
    # WRONG pane (refusal path) and then from the target pane
    local openref
    openref="$(command grep '"action":"review"' "$sd/events.jsonl" | tail -1 | command grep -oE '"ref":"[^"]+"' | cut -d'"' -f4)"
    printf 'open_review_ref=%s\n' "$openref" >> "$root/panes.txt"
    _run "$root" reply-wrong-pane "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/reply" "$openref" "b0 probe reply from the wrong pane"
    _run "$root" reply-right-pane "$root/home" "$srv" -- env TMUX_PANE="$help" "$sd/reply" "$openref" "b0 probe reply from the target pane"
    _run "$root" requests-after "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/requests" all
    _tmuxsnap "$root" "$srv" after
}

fam_archive() { local root="$1" srv="$2"
    _run "$root" archive-preview "$root/home" "$srv" -- "$AE" archive preview "$S"
    _tmuxsnap "$root" "$srv" after
}

fam_events_tail() { local root="$1" srv="$2"
    # D03 shape: a POSITIVE launch barrier (banner + at least one rendered record),
    # bounded poll, then a bounded capture window. The helper is `tail -f` and never
    # exits on its own, so window closure is by TERM, by construction — that is not a
    # timeout. Failure to reach the launch barrier within the bound IS recorded
    # INCONCLUSIVE and never read as an absence.
    local sd="$root/home/.ae/sessions/$S"
    local out="$root/events-tail.stdout.txt" err="$root/events-tail.stderr.txt"
    local shimlog="$root/date-shim.events-tail.log"; : >"$shimlog"
    local -a e; mapfile -t e < <(_env "$root/home" "$srv" "$shimlog")
    { printf 'label=events-tail\n'; printf 'started=%s\n' "$(_now)"
      printf 'argv: %q\n' "$sd/events-tail"; printf 'env (allowlisted):\n'; printf '  %s\n' "${e[@]}"
      printf 'launch barrier: banner line + >=1 rendered record; bounded poll 25s\n'
      printf 'window closure: SIGTERM after the barrier (the helper is tail -f and never exits)\n'; } > "$root/events-tail.invocation.txt"
    : >"$out"; : >"$err"
    env -i "${e[@]}" "$sd/events-tail" >"$out" 2>"$err" &
    local pid=$! i=0 barrier=0
    while (( i < 250 )); do
        if command grep -q 'ae events' "$out" 2>/dev/null && (( $(command grep -c 'UTC' "$out" 2>/dev/null || echo 0) >= 1 )); then barrier=1; break; fi
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1; i=$((i+1))
    done
    printf 'launch_barrier_reached=%s after_polls=%s\n' "$barrier" "$i" >> "$root/events-tail.invocation.txt"
    if (( barrier == 1 )); then sleep 3; else printf 'INCONCLUSIVE: events-tail launch barrier not reached within the bounded poll\n' > "$root/INCONCLUSIVE.events-tail.txt"; fi
    kill -TERM "$pid" 2>/dev/null; sleep 0.5; kill -KILL "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
    printf 'closed_by=SIGTERM (by construction)\nfinished=%s\n' "$(_now)" >> "$root/events-tail.invocation.txt"
    printf 'n/a (window closed by TERM)\n' > "$root/events-tail.rc.txt"
    _tmuxsnap "$root" "$srv" after
}

fam_telegram() { local root="$1" srv="$2"
    local home="$root/home" sd="$root/home/.ae/sessions/$S"
    printf 'b0-fixture-not-a-real-token\n' > "$home/.ae/tg.token"; chmod 600 "$home/.ae/tg.token"
    cat >> "$home/.ae/config" <<CFG

[telegram]
enabled = true
token_file = $home/.ae/tg.token
chat_id = 1234567890
CFG
    mkdir -p "$root/curlshim"
    cat > "$root/curlshim/curl" <<'CURL'
#!/opt/homebrew/bin/bash
# delegate-log-FAIL curl shim: logs argv + stdin and NEVER reaches the network.
{ printf '=== curl invocation %s ===\n' "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'argv:'; printf ' %q' "$@"; printf '\n'
  if [[ ! -t 0 ]]; then
      cfg="$(cat)"
      printf 'stdin:\n'; printf '%s\n' "$cfg" | sed 's/^/  /'
      # the message TEXT rides a temp file (`data-urlencode = text@<path>`); log it,
      # because that body is the telegram formatter's actual output.
      while IFS= read -r l; do
          case "$l" in
              *text@*) f="${l#*text@}"; f="${f%% *}"
                  if [[ -f "$f" ]]; then printf 'text-file %s:\n' "$f"; sed 's/^/  | /' "$f"; else printf 'text-file %s: ABSENT\n' "$f"; fi ;;
          esac
      done <<< "$cfg"
  fi
} >> "${AE_CURL_SHIM_LOG:-/dev/null}" 2>&1
exit 7
CURL
    chmod 755 "$root/curlshim/curl"
    : > "$root/curl-shim.log"
    local -a e; mapfile -t e < <(_env "$home" "$srv" "$root/date-shim.telegram.log")
    e[0]="PATH=$root/curlshim:$ARM_PATH"
    e+=("AE_CURL_SHIM_LOG=$root/curl-shim.log")

    # (1) materialize + stop, so the bounded cycle below owns the singleton lock
    local rc=0
    _timeout 30 env -i "${e[@]}" "$AE" telegram start >"$root/telegram-start.stdout.txt" 2>"$root/telegram-start.stderr.txt" || rc=$?
    printf '%s\n' "$rc" > "$root/telegram-start.rc.txt"
    sleep 3
    rc=1; local si=0
    while (( si < 4 )); do
        rc=0
        _timeout 30 env -i "${e[@]}" "$AE" telegram stop >"$root/telegram-stop.stdout.txt" 2>"$root/telegram-stop.stderr.txt" || rc=$?
        (( rc == 0 )) && break
        sleep 3; si=$((si+1))
    done
    printf '%s (after %s attempt(s))\n' "$rc" "$((si+1))" > "$root/telegram-stop.rc.txt"
    sleep 1
    # If the product stop path did not clear it, the CONTROLLER stops the daemon
    # deterministically: kill its tmux session and clear the singleton lock. Recorded.
    if TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" has-session -t "=ae-telegram" 2>/dev/null; then
        TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" kill-session -t "=ae-telegram" 2>/dev/null
        printf 'controller killed tmux session ae-telegram (ae telegram stop rc=%s)\n' "$rc" > "$root/telegram-controller-stop.txt"
        sleep 1
    fi
    rm -f "$home/.ae/telegram/daemon.lock" "$home/.ae/telegram/control.lock"
    printf 'controller removed telegram/daemon.lock and control.lock before the bounded cycle\n' >> "$root/telegram-controller-stop.txt"

    # (2) seed the daemon's OWN persisted offset state so the bounded cycle reads the
    # fixture from byte 0. Without it an unseen session_id is initialised at EOF
    # (ae:10133-10138, the do-not-replay-history invariant) and no fixture line is
    # ever consumed. Controller action, recorded.
    local sid inode
    sid="$(command grep '^session_id=' "$sd/meta" | cut -d= -f2-)"
    inode="$(stat -f '%i' "$sd/events.jsonl")"
    mkdir -p "$home/.ae/telegram"
    { printf '# session_id\tinode\tbyte_offset\tlast_ts\n'
      printf '%s\t%s\t0\t-\n' "$sid" "$inode"; } > "$home/.ae/telegram/state.tsv"
    cp -a "$home/.ae/telegram/state.tsv" "$root/tg.state.seeded.tsv"
    { printf 'controller seeded %s/.ae/telegram/state.tsv with session_id=%s inode=%s offset=0\n' "$home" "$sid" "$inode"
      printf 'reason: ae:10133-10138 initialises an unseen session at EOF\n'; } > "$root/telegram-seed.txt"

    # (3) bounded direct daemon cycle
    if [[ -f "$home/.ae/telegram-daemon" ]]; then
        cp -a "$home/.ae/telegram-daemon" "$root/telegram-daemon.materialized"
        { printf 'argv: /opt/homebrew/bin/bash %s/.ae/telegram-daemon\n' "$home"
          printf 'bound: SIGTERM after 6s (the daemon loop is infinite by design)\n'
          printf 'env:\n'; printf '  %s\n' "${e[@]}"
          printf '  AE_TELEGRAM_DISCOVERY_INTERVAL=0\n  AE_TELEGRAM_POLL_INTERVAL=0\n  CONFIG_FILE=%s/.ae/config\n' "$home"; } > "$root/telegram-daemon.invocation.txt"
        rc=0
        _timeout 6 env -i "${e[@]}" AE_TELEGRAM_DISCOVERY_INTERVAL=0 AE_TELEGRAM_POLL_INTERVAL=0 \
            CONFIG_FILE="$home/.ae/config" /opt/homebrew/bin/bash "$home/.ae/telegram-daemon" \
            >"$root/telegram-daemon.stdout.txt" 2>"$root/telegram-daemon.stderr.txt" || rc=$?
        printf '%s\n' "$rc" > "$root/telegram-daemon.rc.txt"
    else
        printf 'telegram-daemon was not materialized by `ae telegram start`\n' > "$root/telegram-daemon.absent.txt"
    fi
    for f in "$home/.ae/telegram/"*; do [[ -f "$f" ]] && cp -a "$f" "$root/tg.$(basename "$f")"; done
    _tmuxsnap "$root" "$srv" after
}

fam_aewatch() { local root="$1" srv="$2"
    local home="$root/home"
    printf '\n[telegram]\nenabled = false\n' >> "$home/.ae/config"
    git -C /Users/ckriech/projects/clemens33/ae-rust show 72c7293:contrib/aewatch/aewatch > "$root/aewatch.frozen"
    chmod 755 "$root/aewatch.frozen"
    RUN_TIMEOUT=60 _run "$root" aewatch-once "$home" "$srv" -- /usr/bin/python3 "$root/aewatch.frozen" daemon --once --ae-home "$home/.ae"
    for f in "$home/.ae/aewatch/"*; do [[ -f "$f" ]] && cp -a "$f" "$root/aw.$(basename "$f")"; done
    _tmuxsnap "$root" "$srv" after
}

fam_compact() { local root="$1" srv="$2"
    _resume "$root" "$srv" || return 0
    local sd="$root/home/.ae/sessions/$S"
    local main; main="$(_panes "$root" "$srv" dummy:dummy)"
    printf 'main_pane=%s\n' "$main" > "$root/compact-panes.txt"
    local -a e; mapfile -t e < <(_env "$root/home" "$srv" "$root/date-shim.compact.log")
    e+=("AE_COMPACT_HANDOVER_SECS=45")
    { printf 'argv: %s compact %s --force\n' "$AE" "$S"; printf 'env:\n'; printf '  %s\n' "${e[@]}"
      printf 'handover: controller drives the REAL reply helper from pane %s\n' "$main"; } > "$root/compact.invocation.txt"
    # documented precondition: compact refuses while spawned agents are present
    # ("compact never retires someone else's worker"). Satisfy it with the REAL
    # retire helper, recorded as a precondition step.
    _run "$root" compact-precondition-retire "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/retire" dummy2:helper
    sleep 2
    local base; base="$(wc -l < "$sd/events.jsonl")"
    command grep -oE '"ref":"[^"]+"' "$sd/events.jsonl" 2>/dev/null | cut -d'"' -f4 | sort -u > "$root/refs.before-compact.txt"
    ( env -i "${e[@]}" "$AE" compact "$S" --force >"$root/compact.stdout.txt" 2>"$root/compact.stderr.txt"; echo $? > "$root/compact.rc.txt" ) &
    local cpid=$! i=0 ref=""
    while (( i < 400 )); do
        ref="$(command grep -E '"action":"(ask|review)"' "$sd/events.jsonl" 2>/dev/null | command grep -oE '"ref":"[^"]+"' | cut -d'"' -f4 | sort -u | comm -13 "$root/refs.before-compact.txt" - | tail -1)"
        [[ -n "$ref" ]] && break
        kill -0 "$cpid" 2>/dev/null || break
        sleep 0.25; i=$((i+1))
    done
    printf 'handover_ref=%s (found after %s polls)\n' "$ref" "$i" >> "$root/compact.invocation.txt"
    if [[ -n "$ref" ]] && kill -0 "$cpid" 2>/dev/null; then
        sleep 1
        _run "$root" compact-handover-reply "$root/home" "$srv" -- env TMUX_PANE="$main" "$sd/reply" "$ref" "b0 probe handover reply"
    fi
    i=0
    while kill -0 "$cpid" 2>/dev/null; do sleep 0.5; i=$((i+1)); (( i < 240 )) || { kill -TERM "$cpid" 2>/dev/null; : > "$root/INCONCLUSIVE.compact-timeout"; break; }; done
    wait "$cpid" 2>/dev/null || true
    [[ -f "$root/compact.rc.txt" ]] || printf '124\n' > "$root/compact.rc.txt"
    _tmuxsnap "$root" "$srv" after
}

fam_stop() { local root="$1" srv="$2"
    _resume "$root" "$srv" || return 0
    _tmuxsnap "$root" "$srv" before
    RUN_TIMEOUT=120 _run "$root" stop-all "$root/home" "$srv" -- "$AE" stop all -y
    sleep 2
    _tmuxsnap "$root" "$srv" after
}

_cleanup() { # <armroot> <srv>
    TMUX_TMPDIR="$1/tmuxtmp" command tmux -L "$2" kill-server 2>/dev/null || true
}

"$@"
