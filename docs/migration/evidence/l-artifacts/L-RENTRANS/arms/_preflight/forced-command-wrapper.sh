#!/opt/homebrew/bin/bash
# ForceCommand wrapper. Sets a separate remote HOME and PATH, then runs the
# client's own command verbatim. It substitutes nothing and fakes nothing.
export HOME=/tmp/aelx/L-RENTRANS/_preflight/rig/remote-home
export PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 TZ=UTC
printf 'forced %s\n' "${SSH_ORIGINAL_COMMAND:-<login shell>}" >> /tmp/aelx/L-RENTRANS/_preflight/rig/log/forced.log 2>/dev/null || true
if [[ -n "${SSH_ORIGINAL_COMMAND:-}" ]]; then
    exec /bin/sh -c "$SSH_ORIGINAL_COMMAND"
fi
exec /bin/sh -l
