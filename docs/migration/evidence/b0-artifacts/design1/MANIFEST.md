# B0 Design 1 — SC-507b archive preview/digest stitch cut: run manifest

Captures only. This file records what was run, what was captured, and where the
bytes are. It contains no verdict, classification, or expected-vs-actual claim.

## Frozen source of truth

| Item | Value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` sha256 | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| instrumented `ae` sha256 | `7cf6ec8d664753b01fc79f4ccb9e3c26d589005cec401e90d86070ac6241026f` |
| hook patch | `harness/h507.patch` |
| hook patch sha256 | `fa12aa4aa975b19296c84435bc8a28d085b1685790477895ccb2af1c03929646` |
| environment / tool hashes | `harness/env-record.txt` |
| runner scripts | `harness/run-arm.sh`, `harness/manifest.sh`, `harness/build-template.sh`, `harness/harvest.sh`, `harness/harvest2.sh`, `harness/envrec.sh`, `harness/assemble.sh` |

## Hook sites (one patch, two sites, both in the patch above)

| Hook | Site (frozen line numbers) | Emission |
|---|---|---|
| `H507_AFTER_FACTS` | inserted after `ae:4956` — after the `_ar_facts_row` read in `_ar_compose_meta`, before `_ar_build_meta` (`ae:4960`) | writes an arrival ordinal + timestamp to the barrier log outside the cloned AE_HOME, then blocks on `release.<n>`; bounded by `AE_H507_MAXPOLL` (600 polls x 0.1s), on expiry writes a `timeout` line and returns |
| `H507_PASS` | inserted after `ae:5029` (outer `digest="$(_ar_preview_once …)"` block) and after `ae:5038` (retry analogue) | appends a pass ordinal + timestamp to `pass.log`; reads no product state |

Both hooks are no-ops when `AE_H507_DIR` is unset. All hook and controller
artifacts live outside the cloned AE_HOME (`arms/<arm>/h507/`), so they are not
part of any product-state manifest.

## Template fixture (producer-derived)

| Item | Value |
|---|---|
| template AE_HOME manifest | `template/manifest.tsv` |
| template fingerprint (sha256 of that manifest) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| session name | `b0tmpl` |
| session_id | `aee85067-eebd-4ef4-83f9-e7f358759d73` |
| meta / memo.tsv / events.jsonl / messages | `template/session/` |
| launch config | `template/config` |

Construction (all bytes producer-derived, per the batch-c-design.md
producer-derivation rule):

1. isolated `HOME` + `AE_HOME` (separate assignments), dedicated tmux server
   (`-L aeb0tmpl`, `TMUX_TMPDIR` under the harness temp root), real-`~/.ae/config`
   fingerprint tripwire armed for the whole build;
2. real frozen `ae --local b0tmpl` launch in a fresh git repo, agent commands
   `bash` (config at `template/config`);
3. real generated helpers, in order: `spawn dummy2:helper`, `goal`, `state`
   (both panes), `memo add` (two topics), `ask` -> `reply` (closed pair), `ask`
   (left open);
4. the generated `watchdog` was stopped and the tree left to settle before the
   snapshot; the snapshot is the template.

The template's `events.jsonl` also contains two events emitted by ae itself
during the build (`_watchdog` `alert`, `dummy:dummy` `spawn-failed`) — recorded
here because they are part of the fixture bytes.

## Mutation payloads (harvested AFTER the template snapshot, same session lineage)

| Payload | Path | sha256 | Producer |
|---|---|---|---|
| meta variant | `payloads/meta.variant` | `e5e7f4a988363fe348eee887effdccf3eb18040a69cbfa5a6cf5af993df14a31` | real `spawn dummy2:helper2` on the template session |
| meta variant diff vs template meta | `payloads/meta.variant.diff` | `eac0818d0046c030c6a6d9d0bfe26af1eb27513d4fd202b71adbad0053480936` | `diff -u` |
| memo row | `payloads/memo.row` | `eb07c2a0fcdf4c9108f6338a40a33e314a7d6550bd471ed51296353c5c15ed2a` | real `memo add --topic mutationtopic` |
| ask event 1 | `payloads/events.ask.1` | `e4104e835a21aa3f4a791e761b9c0f30171826e72d085e857f21d96ca60cbca7` | real `ask dummy2:helper` |
| ask event 2 | `payloads/events.ask.2` | `527f609236e4a844f776a99627ab5a25ea7559735b86f241dad30cadaed2e1ea` | real `ask dummy2:helper` |
| ask event 3 | `payloads/events.ask.3` | `516f545a7a5e57ba66f7531398da7084be6716b4bbadf0b124bcb57d577bf0c1` | real `ask dummy2:helper` |
| ask event 4 | `payloads/events.ask.4` | `4c001d841f4f796f4544f4f628b6edeb143ad15603f186382ebfcafeba656a2a` | real `ask dummy2:helper` |

Recorded properties of the payloads (checked before the arms ran):

- the roster in `payloads/meta.variant` differs from the template meta by the
  two lines `agent.spawned.1=dummy2:helper2:pending` and
  `agent_bin.spawned.1=bash` (one agent entry plus its bin line) — full diff in
  `payloads/meta.variant.diff`;
- the memo row carries topic `mutationtopic`; occurrences of that string in the
  template `memo.tsv`: 0;
- each `events.ask.<i>` carries a distinct `ref`; occurrences of each ref in the
  template `events.jsonl`: 0;
- the `body_file` value inside each harvested ask event names an absolute path
  under the TEMPLATE session directory; the file it names is not present in an
  arm clone's `sessions/b0tmpl/messages/` directory (the arm clones carry only
  the template's own `messages/` files, listed in `template/manifest.tsv`).

## Arms

Every arm starts from a fresh clone of the template AE_HOME
(`cp -a`), with its own clone fingerprint recorded. Every arm ran under
`env -i` plus the allowlisted set recorded in each run's
`env.allowlisted.txt` (`PATH`, `HOME`, `AE_HOME`, `TERM`, `TZ`, `LANG`,
`TMUX_TMPDIR`, `AE_TMUX_SERVER`, `AE_TMUX_SERVER_KIND`, and for active runs
`AE_H507_DIR`, `AE_H507_MAXPOLL`). Outer bounded wait per invocation: 90s
(none expired). Barrier wait in the controller: 60s (none expired).

Every arm carries a per-fixture inactive-equivalence result
(`equiv/RESULT.txt`): the instrumented binary with `AE_H507_DIR` UNSET vs the
uninstrumented frozen binary, each on its own fresh clone of the same template,
compared on `stdout`, `stderr`, `rc`, the recursive after-manifest, and the tmux
probe (the probe's own socket path and server name are harness-per-run values
and are normalised before that one comparison; both raw files are kept).

| Arm | Row id | Mutation (controller action at `H507_AFTER_FACTS`) | Artifacts |
|---|---|---|---|
| `arm1-stable-control` | SC-507b | none | `arms/arm1-stable-control/` |
| `arm2-transient-meta` | SC-507b | pass 1 only: writer-shaped temp+rename of sessions/b0tmpl/meta from payloads/meta.variant | `arms/arm2-transient-meta/` |
| `arm3-transient-memo` | SC-507b | pass 1 only: append payloads/memo.row to sessions/b0tmpl/memo.tsv | `arms/arm3-transient-memo/` |
| `arm4-transient-events` | SC-507b | pass 1 only: append payloads/events.ask.1 to sessions/b0tmpl/events.jsonl | `arms/arm4-transient-events/` |
| `arm5-persistent-events` | SC-507b | EVERY pass: append payloads/events.ask.<pass> to sessions/b0tmpl/events.jsonl (a distinct harvested line per pass) | `arms/arm5-persistent-events/` |

### Per-arm captures

#### `arm1-stable-control`

| Item | Value |
|---|---|
| clone fingerprint (sha256 of `manifest.before.tsv`) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| template fingerprint it was cloned from | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| inactive-equivalence result | `arms/arm1-stable-control/equiv/RESULT.txt` — 5 EQUAL, 0 DIFFER |
| instrumented-inactive stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| uninstrumented frozen stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| ACTIVE run rc | `0` (`arms/arm1-stable-control/active/rc.txt`) |
| ACTIVE run stdout | `arms/arm1-stable-control/active/stdout.txt` — 1964 bytes, sha256 `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` |
| ACTIVE run stderr | `arms/arm1-stable-control/active/stderr.txt` — 222 bytes, sha256 `3b1058782a49a4f6976d50ef0e76823704eb7e38290b9e3f5619270e8864eab8` |
| pass ordinals emitted | 1 (`arms/arm1-stable-control/h507/pass.log`) |
| barrier arrivals | 1 (`arms/arm1-stable-control/h507/barrier.log`) |
| controller action log | `arms/arm1-stable-control/h507/controller.log` |
| controller stderr surface | `arms/arm1-stable-control/h507/controller.stdouterr` — 0 bytes |
| INCONCLUSIVE / abort markers | 0 |
| before-manifest | `arms/arm1-stable-control/manifest.before.tsv` |
| after-manifest | `arms/arm1-stable-control/manifest.after.tsv` |
| before/after manifest delta | `arms/arm1-stable-control/manifest.delta.diff` — 0 changed manifest lines |
| tmux snapshot probe | `arms/arm1-stable-control/active/tmux-probe.txt` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |
| mutation byte diffs | none (no mutation in this arm) |
| LEAK-COMPARE post-state control | not part of this arm's spec |

```
--- arms/arm1-stable-control/h507/barrier.log ---
arrive	after_facts	1	2026-08-20T14:42:30Z|1787236950.107357
release	after_facts	1	2026-08-20T14:42:30Z|1787236950.219947
--- arms/arm1-stable-control/h507/pass.log ---
1	2026-08-20T14:42:30Z|1787236950.378246
--- arms/arm1-stable-control/h507/controller.log ---
arm=arm1-stable-control mutation=none start 2026-08-20T14:42:30Z|1787236950.010569
controller start	2026-08-20T14:42:30Z|1787236950.021485	mutation=none
controller saw arrival	1	2026-08-20T14:42:30Z|1787236950.135944
no mutation at pass 1	2026-08-20T14:42:30Z|1787236950.139129
RELEASE pass=1	2026-08-20T14:42:30Z|1787236950.145243
controller exit (ae finished before arrival 2)	2026-08-20T14:42:30Z|1787236950.478244
--- arms/arm1-stable-control/active/stderr.txt ---
PREVIEW ONLY — nothing was written and nothing was stopped.
archive id: aee85067-eebd-4ef4-83f9-e7f358759d73
source session: b0tmpl (aee85067-eebd-4ef4-83f9-e7f358759d73)
selected files: 7, estimated content bytes: 5433
```

#### `arm2-transient-meta`

| Item | Value |
|---|---|
| clone fingerprint (sha256 of `manifest.before.tsv`) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| template fingerprint it was cloned from | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| inactive-equivalence result | `arms/arm2-transient-meta/equiv/RESULT.txt` — 5 EQUAL, 0 DIFFER |
| instrumented-inactive stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| uninstrumented frozen stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| ACTIVE run rc | `0` (`arms/arm2-transient-meta/active/rc.txt`) |
| ACTIVE run stdout | `arms/arm2-transient-meta/active/stdout.txt` — 2053 bytes, sha256 `33f4e5c62b1402116733e36d3fbaef8db6bf5102acc334392fa3b68cb70137df` |
| ACTIVE run stderr | `arms/arm2-transient-meta/active/stderr.txt` — 222 bytes, sha256 `ca2a5744bf464e205bfce2261fc1885081ae3de93373f5e69fd2105fc85d751b` |
| pass ordinals emitted | 2 (`arms/arm2-transient-meta/h507/pass.log`) |
| barrier arrivals | 2 (`arms/arm2-transient-meta/h507/barrier.log`) |
| controller action log | `arms/arm2-transient-meta/h507/controller.log` |
| controller stderr surface | `arms/arm2-transient-meta/h507/controller.stdouterr` — 0 bytes |
| INCONCLUSIVE / abort markers | 0 |
| before-manifest | `arms/arm2-transient-meta/manifest.before.tsv` |
| after-manifest | `arms/arm2-transient-meta/manifest.after.tsv` |
| before/after manifest delta | `arms/arm2-transient-meta/manifest.delta.diff` — 2 changed manifest lines |
| tmux snapshot probe | `arms/arm2-transient-meta/active/tmux-probe.txt` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |
| mutation byte diffs | `arms/arm2-transient-meta/mutations/pass1.meta.diff`  |
| mutation byte facts (sha256/size/inode pre+post) | `arms/arm2-transient-meta/mutations/pass1.meta.bytes.txt`  |
| LEAK-COMPARE post-state control | `arms/arm2-transient-meta/poststate/` — same template, the arm's named mutation applied cold, frozen UNINSTRUMENTED `ae archive preview` run once |
| post-state control rc | `0` |
| post-state control stdout | `arms/arm2-transient-meta/poststate/run/stdout.txt` — 2053 bytes, sha256 `33f4e5c62b1402116733e36d3fbaef8db6bf5102acc334392fa3b68cb70137df` |
| post-state control stderr | `arms/arm2-transient-meta/poststate/run/stderr.txt` — sha256 `ca2a5744bf464e205bfce2261fc1885081ae3de93373f5e69fd2105fc85d751b` |
| post-state control after-manifest | `arms/arm2-transient-meta/poststate/manifest.after.tsv` |

```
--- arms/arm2-transient-meta/h507/barrier.log ---
arrive	after_facts	1	2026-08-20T14:42:34Z|1787236954.396888
release	after_facts	1	2026-08-20T14:42:34Z|1787236954.509558
arrive	after_facts	2	2026-08-20T14:42:34Z|1787236954.767806
release	after_facts	2	2026-08-20T14:42:34Z|1787236954.878301
--- arms/arm2-transient-meta/h507/pass.log ---
1	2026-08-20T14:42:34Z|1787236954.692910
2	2026-08-20T14:42:35Z|1787236955.042723
--- arms/arm2-transient-meta/h507/controller.log ---
arm=arm2-transient-meta mutation=meta-t1 start 2026-08-20T14:42:34Z|1787236954.302712
controller start	2026-08-20T14:42:34Z|1787236954.313530	mutation=meta-t1
controller saw arrival	1	2026-08-20T14:42:34Z|1787236954.430371
MUTATE pass=1 file=meta	2026-08-20T14:42:34Z|1787236954.437386
action: writer-shaped temp+rename of meta from payloads/meta.variant
RELEASE pass=1	2026-08-20T14:42:34Z|1787236954.485806
controller saw arrival	2	2026-08-20T14:42:34Z|1787236954.808266
no mutation at pass 2	2026-08-20T14:42:34Z|1787236954.811097
RELEASE pass=2	2026-08-20T14:42:34Z|1787236954.816220
controller exit (ae finished before arrival 3)	2026-08-20T14:42:35Z|1787236955.257096
--- arms/arm2-transient-meta/active/stderr.txt ---
PREVIEW ONLY — nothing was written and nothing was stopped.
archive id: aee85067-eebd-4ef4-83f9-e7f358759d73
source session: b0tmpl (aee85067-eebd-4ef4-83f9-e7f358759d73)
selected files: 7, estimated content bytes: 5586
```

#### `arm3-transient-memo`

| Item | Value |
|---|---|
| clone fingerprint (sha256 of `manifest.before.tsv`) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| template fingerprint it was cloned from | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| inactive-equivalence result | `arms/arm3-transient-memo/equiv/RESULT.txt` — 5 EQUAL, 0 DIFFER |
| instrumented-inactive stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| uninstrumented frozen stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| ACTIVE run rc | `0` (`arms/arm3-transient-memo/active/rc.txt`) |
| ACTIVE run stdout | `arms/arm3-transient-memo/active/stdout.txt` — 2021 bytes, sha256 `4ce12b4832352ad54dbcda5bcd23cb285996f63781be6f5b68e2d7e96379035a` |
| ACTIVE run stderr | `arms/arm3-transient-memo/active/stderr.txt` — 222 bytes, sha256 `c8303f24ea8dbed1b84c41fd05532e4a8a7bb6743f351e016d048de3fbdc5e3c` |
| pass ordinals emitted | 2 (`arms/arm3-transient-memo/h507/pass.log`) |
| barrier arrivals | 2 (`arms/arm3-transient-memo/h507/barrier.log`) |
| controller action log | `arms/arm3-transient-memo/h507/controller.log` |
| controller stderr surface | `arms/arm3-transient-memo/h507/controller.stdouterr` — 0 bytes |
| INCONCLUSIVE / abort markers | 0 |
| before-manifest | `arms/arm3-transient-memo/manifest.before.tsv` |
| after-manifest | `arms/arm3-transient-memo/manifest.after.tsv` |
| before/after manifest delta | `arms/arm3-transient-memo/manifest.delta.diff` — 2 changed manifest lines |
| tmux snapshot probe | `arms/arm3-transient-memo/active/tmux-probe.txt` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |
| mutation byte diffs | `arms/arm3-transient-memo/mutations/pass1.memo.tsv.diff`  |
| mutation byte facts (sha256/size/inode pre+post) | `arms/arm3-transient-memo/mutations/pass1.memo.tsv.bytes.txt`  |
| LEAK-COMPARE post-state control | `arms/arm3-transient-memo/poststate/` — same template, the arm's named mutation applied cold, frozen UNINSTRUMENTED `ae archive preview` run once |
| post-state control rc | `0` |
| post-state control stdout | `arms/arm3-transient-memo/poststate/run/stdout.txt` — 2021 bytes, sha256 `4ce12b4832352ad54dbcda5bcd23cb285996f63781be6f5b68e2d7e96379035a` |
| post-state control stderr | `arms/arm3-transient-memo/poststate/run/stderr.txt` — sha256 `c8303f24ea8dbed1b84c41fd05532e4a8a7bb6743f351e016d048de3fbdc5e3c` |
| post-state control after-manifest | `arms/arm3-transient-memo/poststate/manifest.after.tsv` |

```
--- arms/arm3-transient-memo/h507/barrier.log ---
arrive	after_facts	1	2026-08-20T14:42:41Z|1787236961.291615
release	after_facts	1	2026-08-20T14:42:41Z|1787236961.405398
arrive	after_facts	2	2026-08-20T14:42:41Z|1787236961.655351
release	after_facts	2	2026-08-20T14:42:41Z|1787236961.769471
--- arms/arm3-transient-memo/h507/pass.log ---
1	2026-08-20T14:42:41Z|1787236961.572975
2	2026-08-20T14:42:41Z|1787236961.930951
--- arms/arm3-transient-memo/h507/controller.log ---
arm=arm3-transient-memo mutation=memo-t1 start 2026-08-20T14:42:41Z|1787236961.201470
controller start	2026-08-20T14:42:41Z|1787236961.212299	mutation=memo-t1
controller saw arrival	1	2026-08-20T14:42:41Z|1787236961.322914
MUTATE pass=1 file=memo.tsv	2026-08-20T14:42:41Z|1787236961.329267
action: append payloads/memo.row to memo.tsv
RELEASE pass=1	2026-08-20T14:42:41Z|1787236961.374003
controller saw arrival	2	2026-08-20T14:42:41Z|1787236961.700325
no mutation at pass 2	2026-08-20T14:42:41Z|1787236961.703336
RELEASE pass=2	2026-08-20T14:42:41Z|1787236961.708096
controller exit (ae finished before arrival 3)	2026-08-20T14:42:42Z|1787236962.143398
--- arms/arm3-transient-memo/active/stderr.txt ---
PREVIEW ONLY — nothing was written and nothing was stopped.
archive id: aee85067-eebd-4ef4-83f9-e7f358759d73
source session: b0tmpl (aee85067-eebd-4ef4-83f9-e7f358759d73)
selected files: 7, estimated content bytes: 5573
```

#### `arm4-transient-events`

| Item | Value |
|---|---|
| clone fingerprint (sha256 of `manifest.before.tsv`) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| template fingerprint it was cloned from | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| inactive-equivalence result | `arms/arm4-transient-events/equiv/RESULT.txt` — 5 EQUAL, 0 DIFFER |
| instrumented-inactive stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| uninstrumented frozen stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| ACTIVE run rc | `0` (`arms/arm4-transient-events/active/rc.txt`) |
| ACTIVE run stdout | `arms/arm4-transient-events/active/stdout.txt` — 2148 bytes, sha256 `29fc60e9c627edc9e9407abaec040278699fb283c57989013d35cec1a9759095` |
| ACTIVE run stderr | `arms/arm4-transient-events/active/stderr.txt` — 222 bytes, sha256 `f13707c4bfdd9ae850dc48d6c0a70a09a90c068acf963d411bcde1636ef4a5c0` |
| pass ordinals emitted | 2 (`arms/arm4-transient-events/h507/pass.log`) |
| barrier arrivals | 2 (`arms/arm4-transient-events/h507/barrier.log`) |
| controller action log | `arms/arm4-transient-events/h507/controller.log` |
| controller stderr surface | `arms/arm4-transient-events/h507/controller.stdouterr` — 0 bytes |
| INCONCLUSIVE / abort markers | 0 |
| before-manifest | `arms/arm4-transient-events/manifest.before.tsv` |
| after-manifest | `arms/arm4-transient-events/manifest.after.tsv` |
| before/after manifest delta | `arms/arm4-transient-events/manifest.delta.diff` — 2 changed manifest lines |
| tmux snapshot probe | `arms/arm4-transient-events/active/tmux-probe.txt` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |
| mutation byte diffs | `arms/arm4-transient-events/mutations/pass1.events.jsonl.diff`  |
| mutation byte facts (sha256/size/inode pre+post) | `arms/arm4-transient-events/mutations/pass1.events.jsonl.bytes.txt`  |
| LEAK-COMPARE post-state control | `arms/arm4-transient-events/poststate/` — same template, the arm's named mutation applied cold, frozen UNINSTRUMENTED `ae archive preview` run once |
| post-state control rc | `0` |
| post-state control stdout | `arms/arm4-transient-events/poststate/run/stdout.txt` — 2148 bytes, sha256 `29fc60e9c627edc9e9407abaec040278699fb283c57989013d35cec1a9759095` |
| post-state control stderr | `arms/arm4-transient-events/poststate/run/stderr.txt` — sha256 `f13707c4bfdd9ae850dc48d6c0a70a09a90c068acf963d411bcde1636ef4a5c0` |
| post-state control after-manifest | `arms/arm4-transient-events/poststate/manifest.after.tsv` |

```
--- arms/arm4-transient-events/h507/barrier.log ---
arrive	after_facts	1	2026-08-20T14:42:48Z|1787236968.172138
release	after_facts	1	2026-08-20T14:42:48Z|1787236968.283226
arrive	after_facts	2	2026-08-20T14:42:48Z|1787236968.546366
release	after_facts	2	2026-08-20T14:42:48Z|1787236968.659954
--- arms/arm4-transient-events/h507/pass.log ---
1	2026-08-20T14:42:48Z|1787236968.458041
2	2026-08-20T14:42:48Z|1787236968.836135
--- arms/arm4-transient-events/h507/controller.log ---
arm=arm4-transient-events mutation=events-t1 start 2026-08-20T14:42:48Z|1787236968.074268
controller start	2026-08-20T14:42:48Z|1787236968.085453	mutation=events-t1
controller saw arrival	1	2026-08-20T14:42:48Z|1787236968.199756
MUTATE pass=1 file=events.jsonl	2026-08-20T14:42:48Z|1787236968.207494
action: append payloads/events.ask.1 to events.jsonl
RELEASE pass=1	2026-08-20T14:42:48Z|1787236968.255723
controller saw arrival	2	2026-08-20T14:42:48Z|1787236968.571904
no mutation at pass 2	2026-08-20T14:42:48Z|1787236968.576856
RELEASE pass=2	2026-08-20T14:42:48Z|1787236968.580393
controller exit (ae finished before arrival 3)	2026-08-20T14:42:49Z|1787236969.026836
--- arms/arm4-transient-events/active/stderr.txt ---
PREVIEW ONLY — nothing was written and nothing was stopped.
archive id: aee85067-eebd-4ef4-83f9-e7f358759d73
source session: b0tmpl (aee85067-eebd-4ef4-83f9-e7f358759d73)
selected files: 7, estimated content bytes: 5994
```

#### `arm5-persistent-events`

| Item | Value |
|---|---|
| clone fingerprint (sha256 of `manifest.before.tsv`) | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| template fingerprint it was cloned from | `27a287d7559c56d09a18c112366db8b1a0b43bd54a8bb9d53ddbb3ed00d719da` |
| inactive-equivalence result | `arms/arm5-persistent-events/equiv/RESULT.txt` — 5 EQUAL, 0 DIFFER |
| instrumented-inactive stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| uninstrumented frozen stdout sha256 | `66d22b66b58fe2f0b277d3200213948730b90cbbb4e056a6dff9f33c40714459` (rc 0) |
| ACTIVE run rc | `1` (`arms/arm5-persistent-events/active/rc.txt`) |
| ACTIVE run stdout | `arms/arm5-persistent-events/active/stdout.txt` — 0 bytes, sha256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| ACTIVE run stderr | `arms/arm5-persistent-events/active/stderr.txt` — 54 bytes, sha256 `e34550b0e4ff81ee9e2e8a7c67aeaebded575f1db47dabaee4f82066b53a5dbf` |
| pass ordinals emitted | 2 (`arms/arm5-persistent-events/h507/pass.log`) |
| barrier arrivals | 2 (`arms/arm5-persistent-events/h507/barrier.log`) |
| controller action log | `arms/arm5-persistent-events/h507/controller.log` |
| controller stderr surface | `arms/arm5-persistent-events/h507/controller.stdouterr` — 0 bytes |
| INCONCLUSIVE / abort markers | 0 |
| before-manifest | `arms/arm5-persistent-events/manifest.before.tsv` |
| after-manifest | `arms/arm5-persistent-events/manifest.after.tsv` |
| before/after manifest delta | `arms/arm5-persistent-events/manifest.delta.diff` — 2 changed manifest lines |
| tmux snapshot probe | `arms/arm5-persistent-events/active/tmux-probe.txt` (list-sessions / list-panes / list-clients with their rc; no tmux server is started by this arm) |
| mutation byte diffs | `arms/arm5-persistent-events/mutations/pass1.events.jsonl.diff` `arms/arm5-persistent-events/mutations/pass2.events.jsonl.diff`  |
| mutation byte facts (sha256/size/inode pre+post) | `arms/arm5-persistent-events/mutations/pass1.events.jsonl.bytes.txt` `arms/arm5-persistent-events/mutations/pass2.events.jsonl.bytes.txt`  |
| LEAK-COMPARE post-state control | not part of this arm's spec |

```
--- arms/arm5-persistent-events/h507/barrier.log ---
arrive	after_facts	1	2026-08-20T14:42:55Z|1787236975.278320
release	after_facts	1	2026-08-20T14:42:55Z|1787236975.392995
arrive	after_facts	2	2026-08-20T14:42:55Z|1787236975.616921
release	after_facts	2	2026-08-20T14:42:55Z|1787236975.829357
--- arms/arm5-persistent-events/h507/pass.log ---
1	2026-08-20T14:42:55Z|1787236975.543201
2	2026-08-20T14:42:55Z|1787236975.987922
--- arms/arm5-persistent-events/h507/controller.log ---
arm=arm5-persistent-events mutation=events-all start 2026-08-20T14:42:55Z|1787236975.183372
controller start	2026-08-20T14:42:55Z|1787236975.193967	mutation=events-all
controller saw arrival	1	2026-08-20T14:42:55Z|1787236975.303302
MUTATE pass=1 file=events.jsonl	2026-08-20T14:42:55Z|1787236975.308595
action: append payloads/events.ask.1 to events.jsonl
RELEASE pass=1	2026-08-20T14:42:55Z|1787236975.354760
controller saw arrival	2	2026-08-20T14:42:55Z|1787236975.681588
MUTATE pass=2 file=events.jsonl	2026-08-20T14:42:55Z|1787236975.687933
action: append payloads/events.ask.2 to events.jsonl
RELEASE pass=2	2026-08-20T14:42:55Z|1787236975.751115
controller exit (ae finished before arrival 3)	2026-08-20T14:42:56Z|1787236976.088965
--- arms/arm5-persistent-events/active/stderr.txt ---
ae: session 'b0tmpl' changed while previewing; retry.
```

## Recorded construction facts and limits

- No tmux server exists during any arm; `ae archive preview` was invoked
  directly against a cloned session directory. Each run's `tmux-probe.txt`
  records `list-sessions` / `list-panes` / `list-clients` against that run's own
  socket, with their rc.
- Mutations are performed by the controller only, from a separate process, while
  the instrumented `ae` is blocked at `H507_AFTER_FACTS`. The hooks never read,
  hash, or compute over product state.
- Arm clone paths differ from the template build path, so absolute paths
  recorded inside the fixture (`origin`, `work_dir`, `config`, `ae_path`,
  `tmux_server`, event `body_file`) name the template build locations.
- `ae` was invoked as `<binary> archive preview b0tmpl`; the exact argv of every
  run is recorded at the end of each run's `env.allowlisted.txt`.
- The harness scripts that produced every artifact here are copied into
  `harness/`.
