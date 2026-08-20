# B0 Design 8 fake agent CLI (run through a RENAMED copy of bash so
# pane_current_command reports the intended tool name — a bash SCRIPT surfaces as
# "bash", measured, which is exactly the fake-recognition hazard the design names).
#
# It renders an idle input region, reads stdin FOREVER logging every byte, and
# re-renders the idle region after each submitted line so the frozen send path's
# staged-paste sensor can reach VERIFIED SUBMIT. It never queries a model and never
# opens a socket.
set -u
TOOL="${AE_FAKE_TOOL:?}"
LOGD="${AE_FAKE_LOG_DIR:?}"
FIX="${AE_FAKE_IDLE_FIXTURE:-}"
mkdir -p "$LOGD"
# One fake can be invoked several times per sandbox (launch, then each spawn), so
# every invocation gets its own artifact set keyed by pid; index.txt maps them.
INST="${TOOL}.$$"
printf '%s\tpid=%s\ttool=%s\tstarted=%s\n' "$INST" "$$" "$TOOL" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOGD/index.txt"

# ── argv / cwd / stdin / ALLOWLISTED env only (never an ambient dump) ──
{
  printf '=== fake %s invoked %s ===\n' "$TOOL" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'argc=%s\n' "$#"
  i=0; for a in "$@"; do printf 'argv[%s]=%s\n' "$i" "$a"; i=$((i+1)); done
  printf 'cwd=%s\n' "$PWD"
  printf 'env (ALLOWLIST ONLY: AE_*, OPENCODE_CONFIG, PATH, HOME, TERM):\n'
  while IFS= read -r kv; do
      case "${kv%%=*}" in
          AE_*|OPENCODE_CONFIG|PATH|HOME|TERM) printf '  %s\n' "$kv" ;;
      esac
  done < <(env)
} > "$LOGD/$INST.launch.txt" 2>&1

# argv verbatim, one file, NUL-separated, so a flag value containing newlines or
# quotes is recoverable byte-for-byte.
printf '%s\0' "$@" > "$LOGD/$INST.argv.nul"

render() {
    printf '\033[H\033[2J'
    if [[ -n "$FIX" && -f "$FIX" ]]; then cat "$FIX"; else
        printf '\n  fake %s — no model, no network\n\n❯ \n' "$TOOL"
    fi
}

stty -echo -icanon min 1 time 0 2>/dev/null || true
render
: > "$LOGD/$INST.stdin.raw"
: > "$LOGD/$INST.stdin.lines"
line=""
while IFS= read -r -N 1 ch; do
    printf '%s' "$ch" >> "$LOGD/$INST.stdin.raw"
    case "$ch" in
        $'\r' | $'\n')
            printf '%s\n' "$line" >> "$LOGD/$INST.stdin.lines"
            line=""
            render
            ;;
        *) line+="$ch" ;;
    esac
done
