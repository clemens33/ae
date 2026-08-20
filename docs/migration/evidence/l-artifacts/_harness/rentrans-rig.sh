#!/opt/homebrew/bin/bash
# L-RENTRANS: the hermetic loopback transport rig.
# A sandbox-local sshd bound to 127.0.0.1 on a random high port; per-sandbox
# host and client keypairs; authorized_keys; a preseeded known_hosts with
# StrictHostKeyChecking=yes; a ForceCommand wrapper that sets a separate remote
# HOME and PATH. No real user HOME, no interface but loopback.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

rig_build() { # <root>
    local root="$1"
    RIG="$root/rig"
    rm -rf "$RIG"; mkdir -p "$RIG"/{etc,bin,log,remote-home}
    mkdir -p "$root/h/.ssh"
    chmod 700 "$root/h/.ssh"
    # random high port
    RIG_PORT=$(( 20000 + (RANDOM % 20000) ))
    # per-sandbox host + client keypairs
    /usr/bin/ssh-keygen -q -t ed25519 -N '' -f "$RIG/etc/ssh_host_ed25519_key" -C "batch-l-rig-host" </dev/null
    /usr/bin/ssh-keygen -q -t ed25519 -N '' -f "$root/h/.ssh/id_ed25519" -C "batch-l-rig-client" </dev/null
    cp "$root/h/.ssh/id_ed25519.pub" "$RIG/etc/authorized_keys"
    chmod 600 "$RIG/etc/authorized_keys" "$RIG/etc/ssh_host_ed25519_key" "$root/h/.ssh/id_ed25519"
    # the ForceCommand wrapper: a SEPARATE remote HOME and PATH
    cat >"$RIG/bin/forced.sh" <<WRAP
#!/opt/homebrew/bin/bash
# ForceCommand wrapper. Sets a separate remote HOME and PATH, then runs the
# client's own command verbatim. It substitutes nothing and fakes nothing.
export HOME=$(printf '%q' "$RIG/remote-home")
export PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin
export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 TZ=UTC
printf 'forced %s\n' "\${SSH_ORIGINAL_COMMAND:-<login shell>}" >> $(printf '%q' "$RIG/log/forced.log") 2>/dev/null || true
if [[ -n "\${SSH_ORIGINAL_COMMAND:-}" ]]; then
    exec /bin/sh -c "\$SSH_ORIGINAL_COMMAND"
fi
exec /bin/sh -l
WRAP
    chmod 0755 "$RIG/bin/forced.sh"
    cat >"$RIG/etc/sshd_config" <<CFG
Port ${RIG_PORT}
ListenAddress 127.0.0.1
HostKey ${RIG}/etc/ssh_host_ed25519_key
PidFile ${RIG}/sshd.pid
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AuthorizedKeysFile ${RIG}/etc/authorized_keys
PermitUserEnvironment no
X11Forwarding no
AllowTcpForwarding no
PrintMotd no
LogLevel VERBOSE
ForceCommand ${RIG}/bin/forced.sh
Subsystem sftp internal-sftp
CFG
    # preseed known_hosts for the alias + StrictHostKeyChecking=yes
    {
        printf '[127.0.0.1]:%s ' "$RIG_PORT"; cut -d' ' -f1,2 "$RIG/etc/ssh_host_ed25519_key.pub"
        printf 'aepeer ';                      cut -d' ' -f1,2 "$RIG/etc/ssh_host_ed25519_key.pub"
    } >"$root/h/.ssh/known_hosts"
    chmod 600 "$root/h/.ssh/known_hosts"
    cat >"$root/h/.ssh/config" <<SCFG
Host aepeer
  HostName 127.0.0.1
  Port ${RIG_PORT}
  User $(id -un)
  IdentityFile ${root}/h/.ssh/id_ed25519
  IdentitiesOnly yes
  StrictHostKeyChecking yes
  UserKnownHostsFile ${root}/h/.ssh/known_hosts
  BatchMode yes
  ConnectTimeout 5
SCFG
    chmod 600 "$root/h/.ssh/config"
    /usr/sbin/sshd -f "$RIG/etc/sshd_config" -E "$RIG/log/sshd.log" 2>"$RIG/log/sshd.start.err"
    RIG_SSHD_RC=$?
    sleep 1
    RIG_SSHD_PID="$(cat "$RIG/sshd.pid" 2>/dev/null || true)"
    return 0
}

rig_stop() {
    [[ -n "${RIG_SSHD_PID:-}" ]] && kill "$RIG_SSHD_PID" 2>/dev/null
    return 0
}
