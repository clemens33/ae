# T-100 fake agent CLI (run through a RENAMED copy of bash so
# pane_current_command reports the intended tool name).
#
# It renders an idle prompt line and then PRINTS EVERY COMPLETE LINE IT READS
# VERBATIM. That is what an UNMODELED pane does with delivered text — it is not
# an invented TUI rendering, and nothing about the bytes is reshaped.
set -u
TOOL="${AE_FAKE_TOOL:?}"
LOGD="${AE_FAKE_LOG_DIR:?}"
mkdir -p "$LOGD"
INST="${TOOL}.$$"
printf '%s\tpid=%s\ttool=%s\tstarted=%s\n' "$INST" "$$" "$TOOL" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$LOGD/index.txt"
{
  printf '=== fake %s invoked %s ===\n' "$TOOL" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'argc=%s\n' "$#"
  i=0; for a in "$@"; do printf 'argv[%s]=%s\n' "$i" "$a"; i=$((i+1)); done
  printf 'cwd=%s\ntty=%s\n' "$PWD" "$(tty 2>/dev/null || echo '<none>')"
} > "$LOGD/$INST.launch.txt" 2>&1
printf '%s' "$(tty 2>/dev/null)" > "$LOGD/$INST.tty"
printf '%s\0' "$@" > "$LOGD/$INST.argv.nul"

stty -echo -icanon min 1 time 0 2>/dev/null || true
printf 'fake %s ready\n' "$TOOL"
: > "$LOGD/$INST.stdin.raw"
: > "$LOGD/$INST.stdin.lines"
line=""
while IFS= read -r -N 1 ch; do
    printf '%s' "$ch" >> "$LOGD/$INST.stdin.raw"
    case "$ch" in
        $'\r' | $'\n')
            printf '%s\n' "$line" >> "$LOGD/$INST.stdin.lines"
            # VERBATIM render of the complete line, exactly as read.
            printf '%s\n' "$line"
            line=""
            ;;
        *) line+="$ch" ;;
    esac
done
