#!/opt/homebrew/bin/bash
# delegate-log-fail `git` shim (Batch L failure-injection primitive).
# Delegates EVERY invocation to the real binary except the named subcommand,
# which is LOGGED and exits non-zero. chmod alone is not trusted; this is.
_real=/usr/bin/git
_fail="${AE_L_GIT_FAIL:-}"
_log="${AE_L_GIT_LOG:-}"
# first non-option word, skipping -C <dir> / -c <kv> values
_sub=""; _skip=0
for _a in "$@"; do
    if (( _skip )); then _skip=0; continue; fi
    case "$_a" in
        -C|-c|--git-dir|--work-tree|--namespace|--exec-path) _skip=1 ;;
        -*) : ;;
        *) _sub="$_a"; break ;;
    esac
done
if [[ -n "$_log" ]]; then
    { printf 'pid=%s sub=%s argc=%s' "$$" "${_sub:-<none>}" "$#"
      for _a in "$@"; do printf ' <%s>' "$_a"; done; printf '\n'; } >>"$_log" 2>/dev/null || true
fi
if [[ -n "$_fail" && "$_sub" == "$_fail" ]]; then
    [[ -n "$_log" ]] && printf 'INJECTED-FAILURE sub=%s rc=128\n' "$_sub" >>"$_log" 2>/dev/null
    printf 'fatal: batch-l delegate-log-fail shim: refusing %s\n' "$_sub" >&2
    exit 128
fi
exec "$_real" "$@"
