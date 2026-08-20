#!/opt/homebrew/bin/bash
# delegate-and-log `flock` spy. Delegates EVERY invocation to the real binary;
# substitutes nothing. Inherited fds pass through the exec unchanged.
_l="${AE_L_FLOCK_LOG:-}"
if [[ -n "$_l" ]]; then
    { printf 'pid=%s ppid=%s t=%s argc=%s' "$$" "$PPID" "${EPOCHREALTIME:-0}" "$#"
      for _a in "$@"; do printf ' <%s>' "$_a"; done
      printf '\n'; } >>"$_l" 2>/dev/null || true
fi
exec /opt/homebrew/bin/flock "$@"
