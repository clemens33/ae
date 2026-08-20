#!/opt/homebrew/bin/bash
# delegate-and-log `ssh` shim. Passes every argument through UNCHANGED and
# records it. Substitutes nothing, injects nothing.
_l="${AE_L_SSH_LOG:-}"
if [[ -n "$_l" ]]; then
    { printf 'pid=%s ppid=%s argc=%s' "$$" "$PPID" "$#"
      for _a in "$@"; do printf ' <%s>' "$_a"; done
      printf '\n'; } >>"$_l" 2>/dev/null || true
fi
exec /usr/bin/ssh "$@"
