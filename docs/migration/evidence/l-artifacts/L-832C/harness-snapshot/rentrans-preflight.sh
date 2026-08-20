#!/opt/homebrew/bin/bash
# L-RENTRANS BLOCKING transport preflight.
# Drives FROZEN `ae transfer` end-to-end through the hermetic loopback rig,
# PUSH and PULL, on disposable sessions, BEFORE any arm runs.
set -uo pipefail
source /tmp/aelx/lib/rentrans-rig.sh

l_arm_begin L-RENTRANS _preflight frozen
PATCHV="none (frozen, unmodified)"
CAPD="$R/cap"

# ── stage 0: the host's own transport prerequisites, measured ────────────────
{
  printf 'stage\t0 — host transport prerequisites, measured on this machine\n'
  printf 'ssh.path\t%s\n' "$(command -v ssh)"
  printf 'ssh.version\t%s\n' "$(ssh -V 2>&1)"
  printf 'sshd.path\t/usr/sbin/sshd\n'
  printf 'sshd.version\t%s\n' "$(/usr/sbin/sshd -V 2>&1 | head -1)"
  printf 'rsync.path\t%s\n' "$(command -v rsync)"
  printf 'rsync.version\t%s\n' "$(rsync --version 2>&1 | head -1)"
  printf 'rsync.version.line2\t%s\n' "$(rsync --version 2>&1 | sed -n 2p)"
  printf '\n# the FROZEN gate, verbatim from 72c7293 (ae:%s):\n' "$(grep -n '^_transfer_local_rsync_supports_protect_args' /tmp/aelx/frozen/ae | cut -d: -f1)"
  awk '/^_transfer_local_rsync_supports_protect_args\(\) \{/,/^\}$/' /tmp/aelx/frozen/ae
  printf '\n# running that exact probe here:\n'
  if rsync --protect-args --version >/dev/null 2>&1; then printf 'probe.result\tSUPPORTED (rc 0)\n'
  else printf 'probe.result\tNOT SUPPORTED (rc %s)\n' "$?"; fi
  printf '\n# every rsync binary present on this host:\n'
  for p in /usr/bin/rsync /usr/local/bin/rsync /opt/homebrew/bin/rsync /opt/local/bin/rsync /bin/rsync; do
    if [[ -x "$p" ]]; then printf '  %s\t%s\n' "$p" "$("$p" --version 2>&1 | head -1)"; else printf '  %s\t(absent)\n' "$p"; fi
  done
} >"$CAPD/stage0.host-prerequisites.txt" 2>&1

# ── stage 1: build the rig ───────────────────────────────────────────────────
rig_build "$R"
{
  printf 'stage\t1 — the hermetic loopback rig\n'
  printf 'sshd.start.rc\t%s\n' "${RIG_SSHD_RC:-?}"
  printf 'sshd.pid\t%s\n' "${RIG_SSHD_PID:-<none>}"
  printf 'port\t%s (random high port)\n' "${RIG_PORT:-?}"
  printf 'listen\t127.0.0.1 only\n'
  printf 'host_keypair\t%s (per-sandbox, generated here)\n' "$RIG/etc/ssh_host_ed25519_key"
  printf 'client_keypair\t%s (per-sandbox, generated here)\n' "$R/h/.ssh/id_ed25519"
  printf 'authorized_keys\t%s\n' "$RIG/etc/authorized_keys"
  printf 'known_hosts\tpreseeded, StrictHostKeyChecking=yes\n'
  printf 'forced_command\t%s — sets a separate remote HOME=%s and PATH\n' "$RIG/bin/forced.sh" "$RIG/remote-home"
  printf 'real_user_home\tNOT used: HOME is the sandbox home for every client invocation\n'
  printf '\n# sshd_config, verbatim\n'; cat "$RIG/etc/sshd_config"
  printf '\n# ssh client config, verbatim\n'; cat "$R/h/.ssh/config"
  printf '\n# sshd start stderr\n'; cat "$RIG/log/sshd.start.err" 2>/dev/null
  printf '\n# listening sockets on that port\n'; /usr/sbin/lsof -nP -iTCP:"${RIG_PORT}" -sTCP:LISTEN 2>/dev/null || printf '(lsof produced nothing)\n'
} >"$CAPD/stage1.rig.txt" 2>&1
cp -p "$RIG/etc/sshd_config" "$CAPD/sshd_config.txt"
cp -p "$R/h/.ssh/config" "$CAPD/ssh_client_config.txt"
cp -p "$RIG/bin/forced.sh" "$CAPD/forced-command-wrapper.sh"

HOOKS=""; BLOCK=""; l_arm_env
AE_CWD="$R/w"; WDIR="$R/w"

# ── stage 2: the loopback channel itself, through the real client ────────────
# TWO halves, kept apart on purpose:
#   2a  HARNESS-SIDE proof that the RIG works — the real ssh client is pointed at
#       the sandbox config with an explicit -F. This proves the sshd, the keys,
#       StrictHostKeyChecking and the ForceCommand wrapper, and NOTHING about
#       what the product can reach.
#   2b  the FROZEN call shape — plain `ssh <target>`, exactly as ae transfer
#       invokes it, with no -F and no shim.
{
  printf 'stage\t2a — RIG PROOF (harness-side, explicit -F; this is NOT the product call shape)\n'
  printf '\n# ssh -F <sandbox config> aepeer, through the ForceCommand wrapper\n'
  env -i "${AE_ENV[@]}" ssh -F "$R/h/.ssh/config" aepeer 'id -un; echo HOME=$HOME; echo PATH=$PATH; uname -s' 2>&1
  printf 'ssh.rc\t%s\n' "$?"
  printf '\n# the frozen probe SHAPE, but with -F so it can reach the rig\n'
  env -i "${AE_ENV[@]}" ssh -F "$R/h/.ssh/config" -o BatchMode=yes -o ConnectTimeout=5 aepeer true 2>&1
  printf 'probe.rc\t%s\n' "$?"
  printf '\n# rsync over that PROVEN channel with the EXACT flags the frozen code uses\n'
  mkdir -p "$R/rsyncsrc" "$RIG/remote-home/rsyncdst"
  printf 'loopback rsync probe\n' >"$R/rsyncsrc/probe.txt"
  env -i "${AE_ENV[@]}" rsync -aHA --protect-args -e "ssh -F $R/h/.ssh/config" "$R/rsyncsrc/" "aepeer:$RIG/remote-home/rsyncdst/" 2>&1
  printf 'rsync.frozen-flags.rc\t%s\n' "$?"
  printf '\n# the same rsync with -A dropped, to isolate which flag fails first\n'
  env -i "${AE_ENV[@]}" rsync -aH --protect-args -e "ssh -F $R/h/.ssh/config" "$R/rsyncsrc/" "aepeer:$RIG/remote-home/rsyncdst/" 2>&1
  printf 'rsync.no-A.rc\t%s\n' "$?"
  printf '\n# and with both -A and --protect-args dropped: does the CHANNEL carry rsync at all?\n'
  env -i "${AE_ENV[@]}" rsync -aH -e "ssh -F $R/h/.ssh/config" "$R/rsyncsrc/" "aepeer:$RIG/remote-home/rsyncdst/" 2>&1
  printf 'rsync.plain.rc\t%s\n' "$?"
  printf '\n# what landed on the remote side\n'; ls -la "$RIG/remote-home/rsyncdst" 2>&1
} >"$CAPD/stage2a.rig-proof.txt" 2>&1

{
  printf 'stage\t2b — the FROZEN call shape: plain ssh <target>, no -F, no shim\n'
  printf '\n# which config files the real client actually reads under this environment\n'
  env -i "${AE_ENV[@]}" ssh -v -o BatchMode=yes -o ConnectTimeout=5 aepeer true 2>&1 | head -20
  printf 'plain.probe.rc\t%s\n' "${PIPESTATUS[0]}"
  printf '\n# HOME as the client saw it\t%s\n' "$R/h"
  printf '# the sandbox config the client did NOT read\t%s\n' "$R/h/.ssh/config"
} >"$CAPD/stage2b.frozen-call-shape.txt" 2>&1

# ── stage 3: FROZEN ae transfer end-to-end, PUSH then PULL ──────────────────
l_config "$R" claude
{ l_mkrepo "$R"; } >/dev/null 2>&1
l_arm_env
l_ae 3launch --local xfer
sleep 3
l_arm_preflight xfer || printf 'NOTE: the TAB preflight did not pass in this sandbox\n' >>"$CAPD/stage3.note.txt"
l_ae 3stop stop -y xfer
sleep 2
l_manifest "$R/h/.ae" "$CAPD/stage3.local-aehome.before.tsv"
l_manifest "$RIG/remote-home" "$CAPD/stage3.remote-home.before.tsv"
l_ae 4push transfer xfer aepeer -y
sleep 1
l_manifest "$R/h/.ae" "$CAPD/stage3.local-aehome.after-push.tsv"
l_manifest "$RIG/remote-home" "$CAPD/stage3.remote-home.after-push.tsv"
l_ae 5pull transfer xfer aepeer --pull -y
sleep 1
l_manifest "$R/h/.ae" "$CAPD/stage3.local-aehome.after-pull.tsv"
l_manifest "$RIG/remote-home" "$CAPD/stage3.remote-home.after-pull.tsv"

PUSH_RC="$(cat "$CAPD/4push.rc")"
PULL_RC="$(cat "$CAPD/5pull.rc")"
{
  printf 'stage\t3 — FROZEN ae transfer, end to end, through the rig\n'
  printf 'push.cmd\tae transfer xfer aepeer -y\npush.rc\t%s\n' "$PUSH_RC"
  printf 'pull.cmd\tae transfer xfer aepeer --pull -y\npull.rc\t%s\n' "$PULL_RC"
  printf '\n# push stdout\n'; cat "$CAPD/4push.stdout"
  printf '\n# push stderr\n'; cat "$CAPD/4push.stderr"
  printf '\n# pull stdout\n'; cat "$CAPD/5pull.stdout"
  printf '\n# pull stderr\n'; cat "$CAPD/5pull.stderr"
} >"$CAPD/stage3.transfer.txt" 2>&1

cp -p "$RIG/log/sshd.log" "$CAPD/sshd.log" 2>/dev/null
cp -p "$RIG/log/forced.log" "$CAPD/forced-command.log" 2>/dev/null

VERDICT=BLOCKED
[[ "$PUSH_RC" == 0 && "$PULL_RC" == 0 ]] && VERDICT=PASSED
{ printf 'preflight.result\t%s\n' "$VERDICT"
  printf 'push.rc\t%s\npull.rc\t%s\n' "$PUSH_RC" "$PULL_RC"
  printf 'rule\tthe design makes this preflight BLOCKING: unless BOTH directions complete, no L-RENTRANS arm may run and the section is reported INCONCLUSIVE/BLOCKED. No semantic ssh or rsync fake is substituted, ever.\n'
} >"$CAPD/PREFLIGHT-RESULT.txt"

rig_stop
{ printf 'arm\t_preflight\nsection\tL-RENTRANS\n'
  printf 'roster_ids\t(none — this is the section BLOCKING gate, not a coverage arm)\n'
  printf 'construction\ta hermetic loopback rig (sandbox-local sshd on 127.0.0.1, random high port, per-sandbox host and client keypairs, preseeded known_hosts with StrictHostKeyChecking=yes, a ForceCommand wrapper setting a separate remote HOME and PATH) driving FROZEN ae transfer end to end, PUSH then PULL, on a disposable session\n'
  printf 'preflight.result\t%s\n' "$VERDICT"
  printf 'push_rc\t%s\npull_rc\t%s\n' "$PUSH_RC" "$PULL_RC"
  printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
} >"$CAPD/ARM.txt"
l_arm_end
echo "PREFLIGHT RESULT: $VERDICT (push rc=$PUSH_RC pull rc=$PULL_RC)"
