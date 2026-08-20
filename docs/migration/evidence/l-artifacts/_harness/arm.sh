#!/opt/homebrew/bin/bash
# Batch L per-arm framework. Capture-only.
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh

ARTROOT=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/l-artifacts

# --- arm scaffolding -------------------------------------------------------
# l_arm_begin <section> <arm> <frozen|instrumented>
# Sets: R (sandbox root), CAP (artifact dir), SOCK, and the env array AE_ENV.
l_arm_begin() {
    SECTION="$1"; ARM="$2"; WHICH="${3:-frozen}"
    R="/tmp/aelx/$SECTION/$ARM"
    CAP="$ARTROOT/$SECTION/arms/$ARM"
    rm -rf "$CAP"; mkdir -p "$CAP"
    { l_mksandbox "$SECTION" "$ARM" "$WHICH"; } >/dev/null 2>&1
    mkdir -p "$R/ctl" "$R/cap"
    l_install_tmux_shim "$R" >/dev/null
    SOCK="$(l_sock "$R")"
    HOOKS=""; BLOCK=""
    return 0
}

# Build the arm env array into AE_ENV (call after any HOOKS/BLOCK assignment).
l_arm_env() {
    local -a extra=("AE_L_TMUX_LOG=$R/cap/tmux-argv.log")
    [[ -n "${HOOKS:-}" ]] && extra+=("AE_L_HOOKS=$HOOKS" "AE_L_TRACE=$R/cap/hook-trace.tsv")
    [[ -n "${BLOCK:-}" ]] && extra+=("AE_L_BLOCK=$R/ctl" "AE_L_BLOCK_MAX=${BLOCK_MAX:-1200}")
    AE_ENV=(); mapfile -t AE_ENV < <(l_env "$R" "$@" ${extra[@]+"${extra[@]}"})
    return 0
}

# l_ae <capture-prefix> <args...>   — run frozen/instrumented ae in the arm env
l_ae() {
    local pfx="$1"; shift
    ( cd "${AE_CWD:-$R/w}" && env -i "${AE_ENV[@]}" "$R/b/ae" "$@" ) \
        >"$R/cap/$pfx.stdout" 2>"$R/cap/$pfx.stderr"
    printf '%s\n' "$?" >"$R/cap/$pfx.rc"
    { printf 'cwd: %s\n' "${AE_CWD:-$R/w}"; printf 'argv:\n'; printf '  %s\n' "$R/b/ae" "$@"
      printf 'env:\n'; printf '  %s\n' "${AE_ENV[@]}"; } >"$R/cap/$pfx.invocation"
    return 0
}

# Background variant; sets AE_BG_PID.
l_ae_bg() {
    local pfx="$1"; shift
    ( cd "${AE_CWD:-$R/w}" && env -i "${AE_ENV[@]}" "$R/b/ae" "$@" ) \
        >"$R/cap/$pfx.stdout" 2>"$R/cap/$pfx.stderr" &
    AE_BG_PID=$!
    { printf 'cwd: %s\n' "${AE_CWD:-$R/w}"; printf 'argv (bg):\n'; printf '  %s\n' "$R/b/ae" "$@"
      printf 'env:\n'; printf '  %s\n' "${AE_ENV[@]}"; } >"$R/cap/$pfx.invocation"
    return 0
}

# --- barrier controller ----------------------------------------------------
# VALUE-BLIND: it presumes no barrier set and no order. It polls for any
# `*.reached` marker, captures state under that marker's own name, then
# releases it. Bounded: on expiry the caller records INCONCLUSIVE.
# l_barriers <session> <deadline-sec> [pre-release-callback]
l_barriers() {
    local sess="$1" deadline="$2" cb="${3:-}"
    local t0=$SECONDS seq=0 f k
    while :; do
        if ! kill -0 "$AE_BG_PID" 2>/dev/null; then
            # subject exited; drain any final markers then stop
            for f in "$R"/ctl/*.reached; do [[ -e "$f" ]] || continue; done
            break
        fi
        for f in "$R"/ctl/*.reached; do
            [[ -e "$f" ]] || continue
            k="$(basename "$f" .reached)"
            [[ -e "$R/ctl/$k.release" ]] && continue
            seq=$((seq + 1))
            local tag; tag="$(printf 'b%02d-%s' "$seq" "$k")"
            printf '%s\t%s\n' "$seq" "$k" >>"$R/cap/barrier-order.tsv"
            l_manifest "$R/h/.ae" "$R/cap/$tag.aehome.tsv"
            l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive.tsv"
            l_tmuxsnap "$SOCK" "$R/cap/$tag.tmux.txt"
            [[ -d "${WDIR:-$R/w}" ]] && { git -C "${WDIR:-$R/w}" status --porcelain=v1 -b 2>&1; echo "--- log ---"; git -C "${WDIR:-$R/w}" log --oneline -5 2>&1; echo "--- branches -r ---"; git -C "${WDIR:-$R/w}" branch -r 2>&1; } >"$R/cap/$tag.git.txt"
            if [[ -n "$cb" ]]; then "$cb" "$k" "$tag"; fi
            : >"$R/ctl/$k.release"
        done
        (( SECONDS - t0 > deadline )) && { printf 'BARRIER-CONTROLLER-EXPIRED after %ss\n' "$deadline" >>"$R/cap/barrier-order.tsv"; return 1; }
        sleep 0.1
    done
    return 0
}

# --- capture helpers -------------------------------------------------------
l_snap() { # <label>
    local lbl="$1"
    l_manifest "$R/h/.ae" "$R/cap/$lbl.aehome.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/$lbl.archive.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$lbl.tmux.txt"
    [[ -d "${WDIR:-$R/w}" ]] && { git -C "${WDIR:-$R/w}" status --porcelain=v1 -b 2>&1; echo "--- log ---"; git -C "${WDIR:-$R/w}" log --oneline -8 2>&1; echo "--- remote refs ---"; git -C "${WDIR:-$R/w}" branch -r 2>&1; } >"$R/cap/$lbl.git.txt"
    l_manifest "${WDIR:-$R/w}" "$R/cap/$lbl.workdir.tsv"
    [[ -d "$R/o.git" ]] && git -C "$R/o.git" show-ref >"$R/cap/$lbl.origin-refs.txt" 2>&1
    return 0
}

l_events_snap() { # <session> <label>
    local s="$1" lbl="$2"
    local f="$R/h/.ae/sessions/$s/events.jsonl"
    if [[ -f "$f" ]]; then cp "$f" "$R/cap/$lbl.events.jsonl"
    else printf '' >"$R/cap/$lbl.events.jsonl"; fi
    return 0
}

# Blocking environment preflight + rule (e) trace. Returns nonzero => no capture.
l_arm_preflight() { # <session>
    local sess="$1"
    l_preflight_arm "$R" "$sess" "$R/cap/preflight-tab.txt" || return 1
    l_consumer_inproc "$R" "$sess" "$R/cap/consumer-inproc.txt"
    return 0
}

l_arm_end() { # collect everything into the artifact dir
    l_ctl kill-server 2>/dev/null
    l_teardown "$R"
    l_sweep_sandbox "$R"
    cp -R "$R/cap/." "$CAP/" 2>/dev/null
    ( cd "$CAP" && find . -type f | LC_ALL=C sort | xargs shasum -a 256 ) >"$CAP/SHA256SUMS.txt" 2>/dev/null
    return 0
}

# Kill a whole process tree (crash-cut primitive). Deepest-first.
l_killtree() { # <pid>
    local pid="$1" kids k
    kids="$(ps -o pid=,ppid= -ax 2>/dev/null | awk -v p="$pid" '$2==p {print $1}')"
    for k in $kids; do l_killtree "$k"; done
    kill -9 "$pid" 2>/dev/null
    return 0
}

# Inability canary: attempt the SAME write class the product would attempt.
# A canary write that SUCCEEDS marks the arm INVALID.
l_canary_mkdir() { # <dir> <out>
    local d="$1" out="$2"
    local p="$d/.l-canary.$$"
    local rc=0 err
    err="$(mkdir "$p" 2>&1)" || rc=$?
    { printf 'canary.class\tmkdir under the archive root (the write _ar_publish makes for its claim)\n'
      printf 'canary.path\t%s\n' "$p"
      printf 'canary.rc\t%s\n' "$rc"
      printf 'canary.stderr\t%s\n' "$err"
      printf 'canary.euid\t%s\n' "$(id -u)"
      printf 'canary.dir.mode\t%s\n' "$(stat -f '%Lp' "$d" 2>/dev/null || echo '?')"
      if ((rc == 0)); then
          printf 'canary.result\tSUCCEEDED — ARM INVALID (the inability was not established)\n'
          rmdir "$p" 2>/dev/null
      else
          printf 'canary.result\tREFUSED\n'
      fi
    } >"$out"
    return $rc
}

# ---------------------------------------------------------------- pty driver
# Runs a command on a REAL terminal in a tmux pane on a DEDICATED control
# server whose socket lives outside the sandbox's own tmux socket directory
# (so ae's enumerable-server sweep never sees it). The subject runs under
# `env -i` + the arm environment, so it inherits no $TMUX from the pane.
# Captures: the pane transcript (pty), separated stdout/stderr bytes, rc.
L_CTLSOCK=""
l_ctl() { "$L_TMUX" -S "$R/ctl/ctlsock" "$@"; }

l_pane_start() { # <pfx> <cmd...>
    local pfx="$1"; shift
    mkdir -p "$R/cap"
    L_PANE_PFX="$pfx"
    local envq="" v
    for v in "${AE_ENV[@]}"; do envq+=" $(printf '%q' "$v")"; done
    local argq="" a
    for a in "$@"; do argq+=" $(printf '%q' "$a")"; done
    local inner
    inner="cd $(printf '%q' "${AE_CWD:-$R/w}"); env -i${envq}${argq} > >(tee $(printf '%q' "$R/cap/$pfx.stdout")) 2> >(tee $(printf '%q' "$R/cap/$pfx.stderr") >&2); echo \$? > $(printf '%q' "$R/cap/$pfx.rc"); sleep 3600"
    l_ctl kill-server 2>/dev/null
    l_ctl new-session -d -s drv -x 200 -y 50 "$L_BASH" -c "$inner"
    { printf 'pty: tmux pane on a dedicated control server\n'; printf 'cwd: %s\n' "${AE_CWD:-$R/w}"
      printf 'argv:\n'; printf '  %s\n' "$@"; printf 'env:\n'; printf '  %s\n' "${AE_ENV[@]}"; } >"$R/cap/$pfx.invocation"
    return 0
}

l_pane_capture() { # <label>
    l_ctl capture-pane -p -t drv >"$R/cap/${L_PANE_PFX}.pane.$1.txt" 2>&1
    return 0
}

# Bounded positive barrier on pane content. Returns 1 on expiry (INCONCLUSIVE).
l_pane_wait() { # <timeout-sec> <fixed-string>
    local t="$1" s="$2" i=0
    while (( i < t*10 )); do
        l_ctl capture-pane -p -t drv 2>/dev/null | grep -Fq -- "$s" && return 0
        sleep 0.1; i=$((i+1))
    done
    return 1
}

l_pane_send() { l_ctl send-keys -t drv -l -- "$1"; }
l_pane_enter() { l_ctl send-keys -t drv Enter; }

l_pane_wait_rc() { # <timeout-sec>
    local t="$1" i=0
    while (( i < t*10 )); do [[ -s "$R/cap/${L_PANE_PFX}.rc" ]] && return 0; sleep 0.1; i=$((i+1)); done
    return 1
}

l_pane_stop() { l_ctl kill-server 2>/dev/null; return 0; }

# Sandbox-wide process sweep: a sandbox whose socket was removed cannot be torn
# down through tmux, so the arm sweeps anything still holding its root path.
l_sweep_sandbox() { # <root>
    local root="$1" p pids
    pids="$(ps -ax -o pid=,command= | grep -F "$root" | grep -v '[g]rep' | awk '{print $1}')"
    for p in $pids; do kill -9 "$p" 2>/dev/null; done
    return 0
}

# Barrier controller for a subject running on the pty driver: same value-blind
# polling, but termination is detected by the rc file rather than a pid.
l_barriers_pane() { # <deadline-sec> <rcfile> [callback]
    local deadline="$1" rcf="$2" cb="${3:-}"
    local t0=$SECONDS seq=0 f k tag
    while :; do
        for f in "$R"/ctl/*.reached; do
            [[ -e "$f" ]] || continue
            k="$(basename "$f" .reached)"
            [[ -e "$R/ctl/$k.release" ]] && continue
            seq=$((seq + 1))
            tag="$(printf 'b%02d-%s' "$seq" "$k")"
            printf '%s\t%s\n' "$seq" "$k" >>"$R/cap/barrier-order.tsv"
            l_manifest "$R/h/.ae" "$R/cap/$tag.aehome.tsv"
            l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive.tsv"
            l_tmuxsnap "$SOCK" "$R/cap/$tag.tmux.txt"
            [[ -n "$cb" ]] && "$cb" "$k" "$tag"
            : >"$R/ctl/$k.release"
        done
        [[ -s "$rcf" ]] && return 0
        (( SECONDS - t0 > deadline )) && { printf 'BARRIER-CONTROLLER-EXPIRED after %ss\n' "$deadline" >>"$R/cap/barrier-order.tsv"; return 1; }
        sleep 0.1
    done
}

# Rewrite a file in the writer's shape (temp + rename) while preserving its
# ORIGINAL mode, so a named byte diff carries no second, unnamed change. The
# plain `> tmp && mv` idiom lands the umask's mode instead, which on a mode-600
# archive member is an extra mutation the content diff cannot show.
l_rewrite_preserving_mode() { # <file> <sed-expr>
    local f="$1" expr="$2"
    local mode; mode="$(stat -f '%Lp' "$f" 2>/dev/null)" || return 1
    sed "$expr" "$f" >"$f.tmp.$$" || return 1
    chmod "0$mode" "$f.tmp.$$" || return 1
    mv "$f.tmp.$$" "$f" || return 1
    return 0
}
