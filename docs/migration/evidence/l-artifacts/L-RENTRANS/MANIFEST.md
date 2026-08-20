# Batch L — section L-RENTRANS — MANIFEST — **INCONCLUSIVE / BLOCKED**

Worker: `opus5:lexec`. **No coverage arm ran.** The design makes this section's transport
preflight BLOCKING, the preflight did not pass on this host, and the section is therefore
reported INCONCLUSIVE/BLOCKED. **No semantic ssh or rsync substitute was used anywhere,
and none will be.**

What this file contains is the preflight's own evidence: bytes, rc values, config files
verbatim, server logs, and manifests. No verdicts about any roster row.

## Result

| field | value |
|---|---|
| preflight result | **BLOCKED** |
| `ae transfer` PUSH rc | `1` |
| `ae transfer` PULL rc | `1` |
| coverage arms run | **0 of 9 roster ids** |
| roster ids NOT covered | SC-814, SC-832a, SC-833a, SC-1302, SC-1303, SC-1304a, SC-1304b, SC-1304c, SC-1304d |

Artifacts: `arms/_preflight/` (40 files), summarised by `arms/_preflight/PREFLIGHT-RESULT.txt`.

## The rig — built, and PROVEN WORKING

`_harness/rentrans-rig.sh` builds exactly what the design specifies, per sandbox:
a local `sshd` bound to `127.0.0.1` on a random high port; per-sandbox host and client
ed25519 keypairs; an `authorized_keys` holding only that client key; a preseeded
`known_hosts` with `StrictHostKeyChecking=yes`; a `ForceCommand` wrapper that exports a
separate remote `HOME` and `PATH`; no real user HOME and no interface but loopback.

Stage 1 (`stage1.rig.txt`) records the sshd start rc `0`, its pid, and `lsof` showing
`TCP 127.0.0.1:<port> (LISTEN)` and nothing else. The full `sshd_config` and ssh client
config are copied verbatim as `sshd_config.txt` and `ssh_client_config.txt`, and the
wrapper as `forced-command-wrapper.sh`.

**Stage 2a (`stage2a.rig-proof.txt`) proves the rig end to end** with the real OpenSSH
client pointed at the sandbox config by an explicit `-F` — labelled in the file as a
HARNESS-SIDE proof and explicitly NOT the product's call shape:

- `ssh … aepeer 'id -un; echo HOME=$HOME; …'` → rc `0`, and the answer shows the
  ForceCommand wrapper in force: `HOME=<rig>/remote-home`, the rig's `PATH`, `Darwin`.
- the frozen probe SHAPE (`-o BatchMode=yes -o ConnectTimeout=5 … true`) → rc `0`.
- **plain `rsync -aH` over that channel → rc `0`, and the file landed on the remote side**
  (`ls` of the remote directory is in the same file). `sshd.log` records the accepted
  publickey; `forced-command.log` records the wrapper receiving
  `rsync --server -g -l -o -p -D -r -t -H --dirs …`.

So the loopback transport itself works, carries rsync, and is not the problem.

## The two blockers, both measured

### BLOCKER A — the host's only rsync cannot run the frozen invocation (decisive)

`stage0.host-prerequisites.txt` enumerates every rsync binary on this machine. There is
exactly one, `/usr/bin/rsync`, and it reports `openrsync: protocol version 29` /
`rsync version 2.6.9 compatible`. Frozen `ae transfer` rsyncs with `rsync -aHA --protect-args`
and gates on `rsync --protect-args --version` (the gate function is quoted verbatim in
that file with its line number).

Measured over the PROVEN channel, in `stage2a.rig-proof.txt`:

| invocation | result |
|---|---|
| `rsync -aHA --protect-args …` (the frozen flags) | `rsync: invalid option -- A`, rc `1` |
| `rsync -aH --protect-args …` (`-A` dropped, to isolate) | `rsync: unrecognized option '--protect-args'`, rc `1` |
| `rsync -aH …` (both dropped) | rc `0`, file transferred |
| `rsync --protect-args --version` (the frozen gate itself) | NOT SUPPORTED |

Both flags the frozen code requires are absent from this rsync. No rig can supply them.
The frozen command refuses at its own local gate before any rsync runs.

### BLOCKER B — the frozen `ssh <target>` call cannot reach a sandbox-only config

`stage2b.frozen-call-shape.txt` runs the plain call shape the product uses — `ssh <target>`,
no `-F`, no shim — and captures the client's own `-v` trace. OpenSSH 10.3p1 reads
`/Users/ckriech/.ssh/config` (the real user's home, from the passwd entry) and never reads
`$HOME/.ssh/config`, so the sandbox Host alias is invisible: `Could not resolve hostname
aepeer`, rc `255`. That is exactly what `stage3.transfer.txt` shows the product hitting —
`Probing SSH connectivity to aepeer...` then `Error: SSH probe to 'aepeer' failed`.

Removing this blocker would require one of:
1. writing a Host alias into the operator's real `~/.ssh/config` — **forbidden**: this
   worker never touches the operator's real HOME;
2. binding the rig's sshd on port 22 so no alias is needed — requires root;
3. an `ssh` PATH shim injecting `-F <sandbox config>` — an instrument that alters the
   subject's own argv rather than delegating it, which this worker will not adopt without
   an explicit ruling.

**Blocker A stands regardless of B.** Even with the alias reachable, the frozen rsync
invocation cannot execute here.

## Stage 3 — the frozen product, driven end to end anyway

`stage3.transfer.txt` records both directions run against the rig with the unmodified
frozen binary and no shim of any kind:

```
push.cmd  ae transfer xfer aepeer -y            push.rc  1
pull.cmd  ae transfer xfer aepeer --pull -y     pull.rc  1
```

with each stream captured verbatim, plus `AE_HOME` and remote-HOME recursive manifests
before, after push and after pull (`stage3.*.tsv`). The session it operated on was a real
`--local` launch, stopped first, exactly as the command's preconditions require.

## What would unblock this section

1. **A real rsync 3.0+ on the host** (both ends — they are the same machine here), so that
   `-A` and `--protect-args` exist. This is a machine change and needs network, so it is
   the operator's call, not this worker's.
2. **A ruling on reaching the sandbox ssh config** — see the three options under
   BLOCKER B. Option 3 is the only one available without touching the operator's machine.

Both are required. (1) alone is not enough on this host, and (2) alone changes nothing.

## A separable observation for the lead, not acted on

Two of this section's rows — SC-832a (`rename-effects`) and SC-1303 (`rename-observer`) —
describe `ae rename` on a running server and concurrent observers at rename cut points.
Neither names ssh or rsync. Part of SC-1302's matrix (the stop×rename ordered pairs) is
likewise transport-free; its transfer pairs are not. The design nonetheless makes the
transport preflight gate **this SECTION**, so this worker ran nothing. Whether the
transport-free subset should run under the blocked gate is a design question and is asked
of the lead rather than decided here.

## Security note

No private key material was written into this repository. The rig's host and client
private keys live only under `/tmp/aelx/…` in the disposable sandbox. A recursive grep of
this section's artifacts for PEM private-key headers returns nothing; the copied configs
reference key PATHS only, and `sshd.log` contains public-key FINGERPRINTS as the server
logged them. The rig's sshd was stopped at the end of the preflight and is not running.

## Harness

`_harness/rentrans-rig.sh` (the rig builder) and `_harness/rentrans-preflight.sh` (the
blocking preflight) are byte-copied into `L-RENTRANS/harness-snapshot/` and hashed by
`L-RENTRANS/HARNESS-SHA256SUMS.txt`.
