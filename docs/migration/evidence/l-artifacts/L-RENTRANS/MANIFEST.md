# Batch L — section L-RENTRANS — MANIFEST — **TRANSPORT GATE BLOCKED; PARTIAL SECTION DELIVERED**

Worker: `opus5:lexec`. The design makes this section's transport preflight BLOCKING; the
preflight **did not pass on this host**, so every transport-dependent row is
INCONCLUSIVE/BLOCKED. **No semantic ssh or rsync substitute was used anywhere, and none
will be.**

On the lead's explicit ruling (and colead's amendment), the rows that touch neither ssh
nor rsync were then run and are delivered as a **PARTIAL SECTION UNDER A BLOCKED TRANSPORT
GATE**. Captures only: bytes, rc values, config files verbatim, server logs, manifests,
argv traces. No verdicts about any roster row.

## Result

| field | value |
|---|---|
| transport preflight | **BLOCKED** |
| `ae transfer` PUSH rc / PULL rc through the rig | `1` / `1` |
| roster ids CLOSED by transport-free arms | **SC-814, SC-832a, SC-1303** |
| roster id run but still PARTIAL | **SC-1302** — its stop×rename cells ran; its transfer cells did NOT |
| roster ids NOT RUN (transport-dependent) | **SC-833a, SC-1304a, SC-1304b, SC-1304c, SC-1304d** |
| arms in this section | 9 (1 preflight gate + 8 coverage arms) |

**SC-1302 stays PARTIAL and must not be read as closed.** Its transfer-dependent ordered
pairs — stop×transfer, rename×transfer and transfer×transfer, in both orders — did NOT
run, because they require the blocked transport. With those cells missing the
serialization row cannot flip to observed from this section, and this manifest says so
rather than letting a full-looking matrix imply closure.

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

---

# PARTIAL SECTION — the transport-free arms

Run on the lead's explicit ruling after the gate blocked, and on colead's amendment adding
SC-814. Each of these arms touches **neither ssh nor rsync**, and every `ARM.txt` in this
group carries `transport	NONE — this arm touches neither ssh nor rsync`.

## A fourth instrumented copy: L-HOOKS-v4

| field | value |
|---|---|
| instrumented `ae` sha256 | `c66fe2d897c5d3b354e1ee11663e17b63ba4077792ab2882c3317da8c082b308` |
| patch sha256 | `edeca4c4d04675048fd45d9d5ca3106a2d5459d0cc156956e52c099122b5758e` (`_harness/hooks-v4.patch`) |
| generator | `_harness/mkhooks4.py` |

v4 is v3's sites plus the four **census-named rename cut points** —
`b_rn_locked_entry` (inside the two-lock region, before any check the rename then
mutates), `b_rn_tmux_renamed` (after `tmux rename-session` and the main-window rename,
before the directory move), `b_rn_dir_moved` (after the state directory move, before the
meta rewrite), `b_rn_meta_updated` (after `session=` is rewritten, before `workspace.md`
is regenerated) — **and one placement CORRECTION**.

**The correction, stated.** `b_stop_one_pre_kill`, introduced in v2, had been inserted
INSIDE the fleet-only `expect_set == true` branch of `_stop_one_session`, so it could never
fire for a singular stop, which is what its name describes. This section found it the only
way such a thing is found: the first run of the two `stop`-first matrix cells recorded
`INCONCLUSIVE: the first operation did not reach b_stop_one_pre_kill within the 60s bound`
and the interleave never happened. In v4 the barrier sits before the branch, where both
paths reach it, and both cells were re-run. **v2 and v3 were deliberately NOT changed** —
no arm of theirs ever used that barrier, and leaving them alone keeps their committed
hashes byte-reproducible (verified: `mkhooks2.py` still yields `4cc428e9…` and
`mkhooks3.py` still yields `b1b07709…`). L-STOP's manifest carries the same correction.

Measured from the tree: **5 of the 8 coverage arms ran under v4** — `rename-observer` and
all four `samename-matrix-*` cells. `rename-effects` and both `endpoint-validation-*` arms
ran on the unmodified frozen binary.

### v4 and shim admissibility — proven BEFORE the captures that use them

| file | comparison | comparator verdict |
|---|---|---|
| `equiv-J-v4-inactive-rename.txt` | frozen vs v4 with `AE_L_HOOKS` unset, on the RENAME fixture | NO_DIFFERENCES |
| `equiv-J-known-difference.txt` | same fixture, a rename target outside the name grammar | DIFFERENCES_PRESENT |
| `equiv-K-ssh-rsync-shims.txt` | frozen with vs without the ssh and rsync delegate-and-log shims, on the TRANSFER fixture with a valid name | NO_DIFFERENCES |
| `equiv-K-known-difference.txt` | same fixture, a hostile session name instead of a valid one | DIFFERENCES_PRESENT |

The K fixture also demonstrates the ssh shim is REACHED on a valid name (its log gains
entries), which is what makes an empty log meaningful later.

## Arms

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `rename-effects` | **SC-832a** | a REAL `ae rename proj proj2` on a RUNNING server, over a topology carrying the prefix-sibling pair | `1pre.*`/`3post.*` (tmux state, session-dir manifests, per-session meta verbatim AND `od -c`, `workspace.md`, tmux server liveness), and `sessions/meta/workspace/server` before-after diffs, `tmux-argv.op.log` |
| `rename-observer` | **SC-1303** | the same rename held at EACH census-named cut point in turn; at every cut a concurrent `ae list --json` runs from a SEPARATE process | `cut-points.txt` (the four cuts in firing order plus a site-only legend), per-cut `*.observer.list.{stdout,stderr,rc}`, `*.sessions.tsv`, `*.tmux.txt`, `*.meta-session-key.txt` |
| `endpoint-validation-hostile-name-push` | **SC-814** | `ae transfer` with a HOSTILE SESSION NAME, push subarm | see the zero-invocation protocol below |
| `endpoint-validation-hostile-name-pull` | **SC-814** | the same, pull subarm | same |
| `samename-matrix-rename-first-flock-with` | SC-1302 (partial row) | the ordered pair rename→stop raced on ONE name under controller barriers, flock PRESENT through the delegate-and-log spy | `interleave.txt`, `at-first-barrier.*`, `both-in-flight.*`, `flock-spy.at-interleave.log`, `flock-spy.final.log`, `final-state.txt`, `flock-availability.txt` |
| `samename-matrix-rename-first-flock-without` | SC-1302 (partial row) | the same pair with flock REMOVED FROM PATH | same set; `flock-availability.txt` records the PATH and the `command -v flock` result |
| `samename-matrix-stop-first-flock-with` | SC-1302 (partial row) | the ordered pair stop→rename, flock PRESENT | same set |
| `samename-matrix-stop-first-flock-without` | SC-1302 (partial row) | the same pair with flock REMOVED FROM PATH | same set |

Measured across the partial group after the v4 correction: **0 `INCONCLUSIVE.txt`, 0
`ARM-INVALID.txt`.**

### How "flock removed from PATH" is constructed

`/opt/homebrew/bin` is dropped from `PATH` entirely and a sandbox bin directory holds
symlinks to `tmux`, `git` and `bash` ONLY, so `command -v flock` fails. Each such arm
records the exact `PATH` and the `command -v flock` result in `flock-availability.txt`, so
the construction is checkable rather than asserted. In the flock-PRESENT cells `flock` is
the delegate-and-log spy, which passes every argument through unchanged.

### SC-814: a POSITIVE zero-invocation capture, not an absence

Frozen `ae transfer` validates the session name and the path object at step 1 — before the
SSH probe at step 4 and before the rsync capability gate — so a hostile session name is
refused without touching the transport. Proving that requires showing the recorders were
LIVE, so each arm runs in three phases:

1. **live-shim canary** — a transfer with a VALID name runs FIRST and must record ssh
   invocations. Measured: `canary_ssh_invocations 1` in both arms. If it had recorded
   zero, the arm writes `ARM-INVALID.txt`, because an empty log afterwards would prove
   nothing about the product.
2. **reset** — both logs are removed (`shim-invocations.3before-measurement.txt` records
   the empty state).
3. **measurement** — the hostile name runs. Measured in both arms:
   `ssh_invocations_after_measure1 0`, `rsync_invocations_after_measure1 0`, and the same
   for a second hostile shape.

Two hostile shapes per arm, both recorded byte-exact (`hostile-name.raw` / `.od.txt` and
`hostile-name2.raw` / `.od.txt`): `../victim` — the path-traversal class the frozen
comment itself names — and a name carrying quoting and command substitution with an
embedded sentinel, whose sentinel path is scanned recursively across the whole sandbox
before and after (`sentinel-state.txt`). `endpoints.txt` states plainly that this host is
both endpoints here, because no transport was reached, and lists the sessions root and the
directory above it — the path a traversal name would reach.

## What the partial section does NOT contain

- **SC-833a** (`transfer-both`: a stopped session pushed, then pulled back) — NOT RUN.
  Requires the transport.
- **SC-1304a/b/c/d** (the POST-STOP and MID-WRITE crash cuts, push-side and pull-side) —
  NOT RUN. Every one of them is defined in terms of an rsync in flight.
- **SC-1302's transfer cells** — NOT RUN, as stated above; the row stays PARTIAL.

Nothing was substituted for any of them.
