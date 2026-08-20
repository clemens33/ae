#!/opt/homebrew/bin/bash
# B0 Design 1 arm runner. Captures only; emits no verdict.
set -euo pipefail
SB=/tmp/aeb0
S=b0tmpl
ARM_PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
AE_TIMEOUT=${AE_TIMEOUT:-90}
BARRIER_TIMEOUT=${BARRIER_TIMEOUT:-60}

_now() { printf '%s|%s' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${EPOCHREALTIME:-na}"; }

_timeout() { # <secs> <cmd...>  -> 124 on expiry
    local secs="$1"; shift
    "$@" & local pid=$! i=0
    while (( i < secs * 10 )); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; i=$((i+1)); done
    if kill -0 "$pid" 2>/dev/null; then kill -TERM "$pid" 2>/dev/null; wait "$pid" 2>/dev/null || true; return 124; fi
    wait "$pid"
}

_clone() { # <destdir>  -> fresh clone of the template AE_HOME + fingerprint
    local d="$1"
    rm -rf "$d"; mkdir -p "$d"
    cp -a "$SB/template/.ae" "$d/.ae"
    "$SB/bin/manifest.sh" "$d/.ae" > "${d}.manifest.before.tsv"
    shasum -a 256 "${d}.manifest.before.tsv" | awk '{print $1}' > "${d}.fingerprint.sha256"
}

_runae() { # <ae-binary> <home> <outdir> [H507DIR]
    local aebin="$1" home="$2" out="$3" hdir="${4:-}"
    mkdir -p "$out"
    local -a e=(env -i
        PATH="$ARM_PATH" HOME="$home" AE_HOME="$home/.ae"
        TERM=dumb TZ=UTC LANG=C
        TMUX_TMPDIR="$out/tmuxtmp" AE_TMUX_SERVER="aeb0-$(basename "$out")" AE_TMUX_SERVER_KIND=name)
    mkdir -p "$out/tmuxtmp"
    [[ -n "$hdir" ]] && e+=(AE_H507_DIR="$hdir" AE_H507_MAXPOLL=600)
    printf '%s\n' "${e[@]:1}" > "$out/env.allowlisted.txt"
    printf 'argv: %s archive preview %s\n' "$aebin" "$S" >> "$out/env.allowlisted.txt"
    local rc=0
    _timeout "$AE_TIMEOUT" "${e[@]}" "$aebin" archive preview "$S" >"$out/stdout.txt" 2>"$out/stderr.txt" || rc=$?
    printf '%s\n' "$rc" > "$out/rc.txt"
    # tmux probe (server may or may not exist)
    { echo "# tmux probe (TMUX_TMPDIR=$out/tmuxtmp, -L aeb0-$(basename "$out"))"
      TMUX_TMPDIR="$out/tmuxtmp" command tmux -L "aeb0-$(basename "$out")" list-sessions 2>&1
      echo "list-sessions rc=$?"
      TMUX_TMPDIR="$out/tmuxtmp" command tmux -L "aeb0-$(basename "$out")" list-panes -a -F '#{pane_id} #{session_name} #{window_id} #{pane_current_command}' 2>&1
      echo "list-panes rc=$?"
      TMUX_TMPDIR="$out/tmuxtmp" command tmux -L "aeb0-$(basename "$out")" list-clients 2>&1
      echo "list-clients rc=$?"
    } > "$out/tmux-probe.txt" 2>&1 || true
    return 0
}

# ── controller: performs the named mutation at the barrier, then releases ──
_controller() { # <armdir> <hdir> <sd> <mutation>
    local arm="$1" hdir="$2" sd="$3" mut="$4"
    # A backgrounded helper that dies under set -e is invisible AND misattributing
    # (AGENTS.md). Give it its own error surface.
    trap 'printf "CONTROLLER ABORTED at line %s (status %s): %s\n" "$LINENO" "$?" "$BASH_COMMAND" >&2; : > "'"$2"'/CONTROLLER_ABORTED"' ERR
    local n=1 waited=0 k=0
    mkdir -p "$arm/mutations"
    printf 'controller start\t%s\tmutation=%s\n' "$(_now)" "$mut" >> "$hdir/controller.log"
    while (( n <= 4 )); do
        waited=0
        while [[ ! -e "$hdir/arrived.$n" ]]; do
            [[ -e "$hdir/done" ]] && { printf 'controller exit (ae finished before arrival %s)\t%s\n' "$n" "$(_now)" >> "$hdir/controller.log"; return 0; }
            sleep 0.1; waited=$((waited+1))
            if (( waited > BARRIER_TIMEOUT * 10 )); then
                printf 'controller TIMEOUT waiting for arrival %s\t%s\n' "$n" "$(_now)" >> "$hdir/controller.log"
                : > "$hdir/INCONCLUSIVE.timeout.arrival.$n"
                return 0
            fi
        done
        printf 'controller saw arrival\t%s\t%s\n' "$n" "$(_now)" >> "$hdir/controller.log"
        local do_mut=0
        case "$mut" in
            none) do_mut=0 ;;
            meta-t1|memo-t1|events-t1) (( n == 1 )) && do_mut=1 ;;
            events-all) do_mut=1 ;;
        esac
        if (( do_mut == 1 )); then
            k=$((k+1))
            local f target
            case "$mut" in
                meta-t1)   f=meta ;;
                memo-t1)   f=memo.tsv ;;
                events-t1|events-all) f=events.jsonl ;;
            esac
            cp -a "$sd/$f" "$arm/mutations/pass${n}.$f.pre"
            printf 'MUTATE pass=%s file=%s\t%s\n' "$n" "$f" "$(_now)" >> "$hdir/controller.log"
            case "$mut" in
                meta-t1)
                    printf 'action: writer-shaped temp+rename of meta from payloads/meta.variant\n' >> "$hdir/controller.log"
                    cp "$SB/payloads/meta.variant" "$sd/meta.tmp.$$"
                    mv "$sd/meta.tmp.$$" "$sd/meta"
                    ;;
                memo-t1)
                    printf 'action: append payloads/memo.row to memo.tsv\n' >> "$hdir/controller.log"
                    cat "$SB/payloads/memo.row" >> "$sd/memo.tsv"
                    ;;
                events-t1)
                    printf 'action: append payloads/events.ask.1 to events.jsonl\n' >> "$hdir/controller.log"
                    cat "$SB/payloads/events.ask.1" >> "$sd/events.jsonl"
                    ;;
                events-all)
                    printf 'action: append payloads/events.ask.%s to events.jsonl\n' "$k" >> "$hdir/controller.log"
                    cat "$SB/payloads/events.ask.$k" >> "$sd/events.jsonl"
                    ;;
            esac
            cp -a "$sd/$f" "$arm/mutations/pass${n}.$f.post"
            diff -u "$arm/mutations/pass${n}.$f.pre" "$arm/mutations/pass${n}.$f.post" > "$arm/mutations/pass${n}.$f.diff" || true
            { printf 'pre  sha256=%s size=%s inode=%s\n' \
                "$(shasum -a 256 "$arm/mutations/pass${n}.$f.pre" | awk '{print $1}')" \
                "$(stat -f '%z' "$arm/mutations/pass${n}.$f.pre")" \
                "$(stat -f '%i' "$arm/mutations/pass${n}.$f.pre")"
              printf 'post sha256=%s size=%s inode=%s\n' \
                "$(shasum -a 256 "$arm/mutations/pass${n}.$f.post" | awk '{print $1}')" \
                "$(stat -f '%z' "$arm/mutations/pass${n}.$f.post")" \
                "$(stat -f '%i' "$sd/$f")"
            } > "$arm/mutations/pass${n}.$f.bytes.txt"
        else
            printf 'no mutation at pass %s\t%s\n' "$n" "$(_now)" >> "$hdir/controller.log"
        fi
        printf 'RELEASE pass=%s\t%s\n' "$n" "$(_now)" >> "$hdir/controller.log"
        : > "$hdir/release.$n"
        n=$((n+1))
    done
    return 0
}

run_equiv() { # <armid> : per-fixture inactive equivalence (instrumented-inactive vs uninstrumented)
    local id="$1"
    local arm="$SB/arms/$id"
    mkdir -p "$arm/equiv"
    _clone "$arm/equiv/inactive-home"
    _clone "$arm/equiv/uninstr-home"
    _runae "$SB/instr/ae"  "$arm/equiv/inactive-home" "$arm/equiv/inactive"
    _runae "$SB/frozen/ae" "$arm/equiv/uninstr-home"  "$arm/equiv/uninstr"
    "$SB/bin/manifest.sh" "$arm/equiv/inactive-home/.ae" > "$arm/equiv/inactive.manifest.after.tsv"
    "$SB/bin/manifest.sh" "$arm/equiv/uninstr-home/.ae"  > "$arm/equiv/uninstr.manifest.after.tsv"
    { echo "# per-fixture inactive equivalence (instrumented, AE_H507_DIR unset) vs uninstrumented frozen"
      for f in stdout.txt stderr.txt rc.txt; do
        if cmp -s "$arm/equiv/inactive/$f" "$arm/equiv/uninstr/$f"; then echo "EQUAL $f"; else echo "DIFFER $f"; diff -u "$arm/equiv/uninstr/$f" "$arm/equiv/inactive/$f" || true; fi
      done
      if cmp -s "$arm/equiv/inactive.manifest.after.tsv" "$arm/equiv/uninstr.manifest.after.tsv"; then echo "EQUAL manifest.after"; else echo "DIFFER manifest.after"; diff -u "$arm/equiv/uninstr.manifest.after.tsv" "$arm/equiv/inactive.manifest.after.tsv" || true; fi
      # tmux probe: the harness's own socket dir + server name differ by construction
      # (one run dir each). Normalise those HARNESS tokens only; nothing product-visible.
      _nrm() { sed -E 's#/(private/)?tmp/aeb0/arms/[^ )]*#<HARNESSPATH>#g; s#aeb0-[A-Za-z0-9_-]+#<HARNESSSRV>#g' "$1"; }
      if cmp -s <(_nrm "$arm/equiv/inactive/tmux-probe.txt") <(_nrm "$arm/equiv/uninstr/tmux-probe.txt"); then
        echo "EQUAL tmux-probe.txt (harness socket path + server name normalised)"
      else
        echo "DIFFER tmux-probe.txt"; diff -u <(_nrm "$arm/equiv/uninstr/tmux-probe.txt") <(_nrm "$arm/equiv/inactive/tmux-probe.txt") || true
      fi
      echo "inactive_rc=$(cat "$arm/equiv/inactive/rc.txt")  uninstr_rc=$(cat "$arm/equiv/uninstr/rc.txt")"
      echo "inactive_stdout_sha256=$(shasum -a 256 "$arm/equiv/inactive/stdout.txt" | awk '{print $1}')"
      echo "uninstr_stdout_sha256=$(shasum -a 256 "$arm/equiv/uninstr/stdout.txt" | awk '{print $1}')"
    } > "$arm/equiv/RESULT.txt" 2>&1
}

run_active() { # <armid> <mutation>
    local id="$1" mut="$2"
    local arm="$SB/arms/$id"
    local hdir="$arm/h507"
    mkdir -p "$hdir"
    _clone "$arm/home"
    local sd="$arm/home/.ae/sessions/$S"
    printf 'arm=%s mutation=%s start %s\n' "$id" "$mut" "$(_now)" > "$hdir/controller.log"
    "$SB/bin/run-arm.sh" _controller "$arm" "$hdir" "$sd" "$mut" >"$hdir/controller.stdouterr" 2>&1 &
    local cpid=$!
    _runae "$SB/instr/ae" "$arm/home" "$arm/active" "$hdir"
    : > "$hdir/done"
    wait "$cpid" 2>/dev/null || true
    "$SB/bin/manifest.sh" "$arm/home/.ae" > "$arm/manifest.after.tsv"
    diff -u "$arm/home.manifest.before.tsv" "$arm/manifest.after.tsv" > "$arm/manifest.delta.diff" || true
    cp "$arm/home.manifest.before.tsv" "$arm/manifest.before.tsv"
    cp "$arm/home.fingerprint.sha256" "$arm/clone-fingerprint.sha256"
}

run_poststate() { # <armid> <mutation>  : LEAK-COMPARE post-state control, FROZEN uninstrumented
    local id="$1" mut="$2"
    local arm="$SB/arms/$id"
    mkdir -p "$arm/poststate"
    _clone "$arm/poststate-home"
    local sd="$arm/poststate-home/.ae/sessions/$S"
    { printf 'poststate control: mutation=%s applied COLD (no run in progress) %s\n' "$mut" "$(_now)"
      case "$mut" in
        meta-t1)   cp "$SB/payloads/meta.variant" "$sd/meta.tmp.$$"; mv "$sd/meta.tmp.$$" "$sd/meta"; echo "applied: writer-shaped temp+rename meta<-payloads/meta.variant" ;;
        memo-t1)   cat "$SB/payloads/memo.row" >> "$sd/memo.tsv"; echo "applied: append payloads/memo.row" ;;
        events-t1) cat "$SB/payloads/events.ask.1" >> "$sd/events.jsonl"; echo "applied: append payloads/events.ask.1" ;;
      esac
    } > "$arm/poststate/controller.log"
    "$SB/bin/manifest.sh" "$arm/poststate-home/.ae" > "$arm/poststate/manifest.premutation-plus.tsv"
    _runae "$SB/frozen/ae" "$arm/poststate-home" "$arm/poststate/run"
    "$SB/bin/manifest.sh" "$arm/poststate-home/.ae" > "$arm/poststate/manifest.after.tsv"
}

"$@"
