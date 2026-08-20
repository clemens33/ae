#!/opt/homebrew/bin/bash
# Batch L sandbox builder. Every arm gets a DISPOSABLE sandbox of its own.
# Nothing here touches the operator's HOME, AE_HOME, tmux server, or the live tree.
set -uo pipefail
source /tmp/aelx/lib/l_lib.sh

L_UTF8_LOCALE=en_US.UTF-8

# Build a fresh sandbox. Prints its root.
l_mksandbox() { # <section> <arm> [instrumented|frozen]
    local section="$1" arm="$2" which="${3:-frozen}"
    local root="/tmp/aelx/$section/$arm"
    rm -rf "$root" 2>/dev/null
    mkdir -p "$root"/{h,t,tp,b,cap,ctl}
    mkdir -p "$root/h/.ae"
    cp /tmp/aelx/lib/fake-tool.sh "$root/b/fake-tool.sh"
    # renamed-interpreter fakes (b0exec D8 measurement: a bash SCRIPT surfaces as
    # its interpreter in pane_current_command; a renamed COPY reports the name)
    local t
    for t in claude codex grok; do cp "$L_BASH" "$root/b/$t"; chmod 0755 "$root/b/$t"; done
    if [[ "$which" == instrumented ]]; then cp /tmp/aelx/instr/ae "$root/b/ae"
    else cp "$L_FROZEN/ae" "$root/b/ae"; fi
    chmod 0755 "$root/b/ae"
    printf '%s\n' "$root"
}

# The scrubbed, UTF-8-pinned environment for an arm. env -i plus the documented
# minimum. THE ENVIRONMENT IS AN INSTRUMENT (cluster-plan.md): the locale is
# pinned to UTF-8 and every LIVE arm proves the TAB round-trip before capturing.
l_env() { # <root> [extra VAR=VAL ...]
    local root="$1"; shift
    printf '%s\n' \
        "HOME=$root/h" \
        "AE_HOME=$root/h/.ae" \
        "TMPDIR=$root/tp" \
        "TMUX_TMPDIR=$root/t" \
        "PATH=$root/b:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
        "TERM=xterm-256color" \
        "LANG=$L_UTF8_LOCALE" \
        "LC_ALL=$L_UTF8_LOCALE" \
        "TZ=UTC" \
        "SHELL=$L_BASH" \
        "AE_FAKE_TOOL=fake" \
        "AE_FAKE_LOG_DIR=$root/cap/fake" \
        "$@"
}

# Run a command inside the arm's environment (non-pty).
l_run() { # <root> <capture-prefix> <cmd...>
    local root="$1" pfx="$2"; shift 2
    local -a e=(); mapfile -t e < <(l_env "$root")
    mkdir -p "$(dirname "$pfx")"
    env -i "${e[@]}" "$@" >"${pfx}.stdout" 2>"${pfx}.stderr"
    local rc=$?
    printf '%s\n' "$rc" >"${pfx}.rc"
    { printf 'argv:\n'; printf '  %s\n' "$@"; printf 'env:\n'; printf '  %s\n' "${e[@]}"; } >"${pfx}.invocation"
    return 0
}

# Run a command on a PTY (BSD script) inside the arm's environment.
# stdin is whatever the caller pipes in; the child sees a terminal.
l_run_pty() { # <root> <capture-prefix> <cmd...>
    local root="$1" pfx="$2"; shift 2
    local -a e=(); mapfile -t e < <(l_env "$root")
    mkdir -p "$(dirname "$pfx")"
    env -i "${e[@]}" /usr/bin/script -q "${pfx}.pty" "$@" >"${pfx}.stdout" 2>"${pfx}.stderr"
    local rc=$?
    printf '%s\n' "$rc" >"${pfx}.rc"
    { printf 'argv (pty):\n'; printf '  %s\n' "$@"; printf 'env:\n'; printf '  %s\n' "${e[@]}"; } >"${pfx}.invocation"
    return 0
}

l_sock() { printf '%s/t/tmux-%s/default\n' "$1" "$(id -u)"; }

# A minimal ae config for the arm.
l_config() { # <root> <main-alias> [extra lines...]
    local root="$1" main="$2"; shift 2
    { printf '[agents]\n'
      printf 'claude = "claude %s/b/fake-tool.sh"\n' "$root"
      printf 'codex = "codex %s/b/fake-tool.sh"\n' "$root"
      printf '\n[workspace]\n'
      printf 'main = %s\n' "$main"
      printf 'layout = vertical\n'
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$root/h/.ae/config"
}

# A git repo to launch --copy/--worktree from, plus a bare local origin.
l_mkrepo() { # <root>
    local root="$1"
    mkdir -p "$root/w"
    git -C "$root/w" init -q -b main 2>/dev/null
    git -C "$root/w" config user.email l@example.invalid
    git -C "$root/w" config user.name "batch l"
    printf 'seed\n' >"$root/w/README.md"
    git -C "$root/w" add -A && git -C "$root/w" commit -q -m seed
    git init -q --bare "$root/o.git"
    git -C "$root/w" remote add origin "file://$root/o.git"
    git -C "$root/w" push -q -u origin main 2>/dev/null
    git -C "$root/w" remote set-head origin -a 2>/dev/null
    return 0
}

l_teardown() { # <root>
    local root="$1" sock; sock="$(l_sock "$root")"
    "$L_TMUX" -S "$sock" kill-server 2>/dev/null
    return 0
}

# ---------------------------------------------------------------- tmux shim
# Delegate-and-log shim per the date-shim contract: EVERY invocation is
# delegated to the real binary; the shim substitutes nothing. Its log is a
# HARNESS artifact (outside AE_HOME) and is excluded from product-state
# equivalence. Supplies the "delegated command-tmux argv" half of rule (e).
l_install_tmux_shim() { # <root>
    local root="$1"
    cat >"$root/b/tmux" <<'SHIM'
#!/opt/homebrew/bin/bash
# delegate-and-log tmux shim (Batch L). Substitutes nothing.
_l="${AE_L_TMUX_LOG:-}"
if [[ -n "$_l" ]]; then
    { printf 'pid=%s ppid=%s AE_TMUX_SERVER=%s AE_TMUX_SERVER_KIND=%s argc=%s' \
        "$$" "$PPID" "${AE_TMUX_SERVER:-<unset>}" "${AE_TMUX_SERVER_KIND:-<unset>}" "$#"
      for _a in "$@"; do printf ' <%s>' "$_a"; done
      printf '\n'; } >>"$_l" 2>/dev/null || true
fi
exec /opt/homebrew/bin/tmux "$@"
SHIM
    chmod 0755 "$root/b/tmux"
    printf '%s\n' "$root/b/tmux"
}

l_remove_tmux_shim() { rm -f "$1/b/tmux"; }

# ------------------------------------------- arm-scoped environment preflight
# Runs the CONSUMER'S OWN tmux query INSIDE the arm's environment (env -i +
# the pinned UTF-8 locale). BLOCKING: a non-zero return means no capture may be
# taken through that environment.
l_preflight_arm() { # <root> <session> <out>
    local root="$1" sess="$2" out="$3"
    local -a e=(); mapfile -t e < <(l_env "$root")
    local sock; sock="$(l_sock "$root")"
    env -i "${e[@]}" "$L_BASH" -c '
        source /tmp/aelx/lib/l_lib.sh
        l_tab_preflight "$1" "$2" "$3"
    ' _ "$sock" "$sess" "$out"
    return $?
}

# ------------------------------------------- per-consumer in-process trace
# The in-process half of rule (e). The frozen shim-installer is extracted by
# awk range (the same technique ae's own unit harness uses), eval'd in a
# subshell under the arm's recorded AE_TMUX_SERVER/KIND, and `type`/`declare -F`
# recorded. Labelled a HARNESS-SIDE reconstruction; the authoritative delegated
# argv comes from the tmux shim log.
l_consumer_inproc() { # <root> <session> <out>
    local root="$1" sess="$2" out="$3"
    local meta="$root/h/.ae/sessions/$sess/meta"
    local srv kind
    srv="$(grep '^tmux_server=' "$meta" 2>/dev/null | head -1 | cut -d= -f2- || true)"
    kind="$(grep '^tmux_server_kind=' "$meta" 2>/dev/null | head -1 | cut -d= -f2- || true)"
    local ext="$root/cap/_ae_install_tmux_shim.extracted.sh"
    awk '/^_ae_install_tmux_shim\(\) \{/,/^\}$/' "$L_FROZEN/ae" >"$ext"
    {
        printf 'source\tfrozen 72c7293 ae, awk-extracted _ae_install_tmux_shim\n'
        printf 'extract.sha256\t%s\n' "$(l_sha "$ext")"
        printf 'meta.tmux_server\t%s\n' "${srv:-<empty>}"
        printf 'meta.tmux_server_kind\t%s\n' "${kind:-<empty>}"
        printf '%s\n' '--- reconstruction under those values ---'
        AE_TMUX_SERVER="$srv" AE_TMUX_SERVER_KIND="$kind" "$L_BASH" -c '
            source "$1"
            _ae_install_tmux_shim
            printf "declare -F tmux\t%s\n" "$(declare -F tmux || echo "<none>")"
            printf "type tmux\t%s\n" "$(type tmux 2>&1 | tr "\n" "|")"
            printf "AE_TMUX_SERVER\t%s\n" "${AE_TMUX_SERVER:-<unset>}"
            printf "AE_TMUX_SERVER_KIND\t%s\n" "${AE_TMUX_SERVER_KIND:-<unset>}"
            declare -f tmux 2>/dev/null || true
        ' _ "$ext"
    } >"$out" 2>&1
    return 0
}
