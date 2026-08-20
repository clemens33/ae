# Batch C artifacts — MANIFEST

Worker: `opus5:cexec`. This file maps **assignment → row ids → artifact paths →
template fingerprints → mutation diffs** for the Batch C read-side evidence run.

Everything here is a CAPTURE: bytes, hashes, manifests, byte diffs, rc values, logs,
and the manipulation/barrier that produced them. No verdicts, no expected-vs-actual
statements, no classification. Seats classify.

## Run-wide provenance

| field | value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` blob | `50e4f575baf3aa584b98b3eaaeec5264e4916161` |
| frozen `ae` sha256 (extracted) | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| frozen tree source | `git archive 72c7293` into worker scratch — the live checkout's `ae` and product files are never touched |
| host | `Darwin 25.6.0 arm64` |
| interpreter | `/opt/homebrew/bin/bash` — GNU bash 5.3.15(1) — sha256 `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| tmux | `/opt/homebrew/bin/tmux` — `tmux 3.7b` |
| per-arm environment | `TZ=UTC`, `LANG=C`, `LC_ALL=C`, scrubbed `PATH`, fresh `HOME`/`AE_HOME`, own `TMPDIR`/`TMUX_TMPDIR`, dedicated tmux socket, cleaned per arm |
| live models / network | none used |

## Lane status

- **BASH lane**: running, per brief.
- **RUST lane**: UNLOCKED, not yet run. Both gate conditions were delivered by the
  lead: reviewer3 passed the rust slice (round-4 delta verdict, no findings), and the
  frozen reviewed rust tree is commit `2cfbf80` on `rust-rewrite` —
  full id `2cfbf805fa442e2e2d712c81e883fbc7036f0fb2`, resolved locally at record time. The exact
  rust-lane invocation is recorded per arm alongside that commit id before any rust
  capture. Rust lanes clone FRESH from the same pre-lane template fingerprints the
  bash lane cloned from, never from bash-mutated state. Per-lane captures are reported
  raw; no bash-vs-rust comparison or divergence verdict is produced here — divergences
  are paired raw artifacts. Sequencing: the bash lane completes per arm group first.

## Boundary as executed

64 assignments (batch-c-design.md v3 at `8f21a48`, slice-1d): A1 gains SC-510e/f;
A7's SC-405j gains case 5 (empty-member subcases). D01–D04 concurrency cuts execute
the approved b0-design.md Designs 2–6; SC-1306a–e ride those designs per the mapping
note (1306a→D01/Design 2, 1306b→D04a/Design 5, 1306c→D04b/Design 6, 1306d→D02/Design 3,
1306e→D03/Design 4). Designs 1/7/8 are not this worker's.

## Contents

| section | status |
|---|---|
| Step 0 — T-WD producer precursor (feeds G2) | COMPLETE (below) |
| Template groups G1–G11 (+G2b) | COMPLETE — 12 groups, 29 members, all fingerprinted and chmod-protected (below) |
| Arm groups A1–A9 (64 assignments) | not started |

---

## Step 0 — T-WD producer precursor (fixture harvest for G2)

Design executed: `docs/migration/evidence/twd-precursor.md` v2 (approved 4af7be0).
Producer: the REAL generated watchdog of a real `ae` launch at the frozen commit
(`<meta>/watchdog _run`). No hook patch, no clock shim; pacing rides the documented
`AE_WATCHDOG_*` knobs, recorded per arm. Arms are named by MANIPULATION.

### Arm `a1` — manipulation: kill only the fake-agent child of the pane (pane shell returns to foreground)

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T14:59:57Z` |
| `end_utc` | `2026-08-20T15:00:46Z` |
| `pre_manipulation_aefake_pids` | `59585 ` |
| `pre_manipulation_pane_current_command` | `aefake` |
| `pre_manipulation_pane_pid` | `59360` |
| `manipulation_utc` | `2026-08-20T15:00:16Z` |
| `post_manipulation_aefake_gone` | `1` |
| `post_manipulation_pane_current_command` | `zsh` |
| `barriers_crossed` | `8` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a1/run-manifest.txt` — knobs, hashes, barrier ledger
- `a1/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a1/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a1/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a1/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a1/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a1/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a1/meta.at-launch.txt`, `a1/meta.final.txt` — session meta bytes
- `a1/ae-launch.out`, `a1/ae-launch.err` — launch stdout/stderr
- `a1/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a1/cap/events.final.jsonl` |
| `source_events_sha256` | `95a330470df9c871d60ee25e540a2d55ccebdcd3b0bfd4cb2c439d09bce49b13` |
| `source_events_bytes` | `141` |
| `total_specimens` | `1` |
| `alert_family_specimens` | `1` |
| `all_actions` | `['alert']` |
| `alert_family_actions` | `['alert']` |

### Arm `a2` — manipulation: none after launch — fake agent alive, pane left static; shortened stale threshold and nudge cap

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T15:00:48Z` |
| `end_utc` | `2026-08-20T15:06:31Z` |
| `observation_window_cycles` | `64` |
| `agent_stdin_log_sha256` | `bb838301c4c0671d61d3dcf3c4805fb533596e5b770593088069ec0ad2a69f11` |
| `agent_stdin_log_bytes` | `476` |
| `final_aefake_pids` | `70841 ` |
| `barriers_crossed` | `64` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a2/run-manifest.txt` — knobs, hashes, barrier ledger
- `a2/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a2/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a2/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a2/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a2/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a2/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a2/meta.at-launch.txt`, `a2/meta.final.txt` — session meta bytes
- `a2/ae-launch.out`, `a2/ae-launch.err` — launch stdout/stderr
- `a2/agent-stdin.log` — the bytes the pane RECEIVED (fake agent's stdin, no echo)
- `a2/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a2/cap/events.final.jsonl` |
| `source_events_sha256` | `0553a5be84ee5c4b33b4f816447e8207dc7455cd87024ecf872123c9f6cd407c` |
| `source_events_bytes` | `640` |
| `total_specimens` | `3` |
| `alert_family_specimens` | `1` |
| `all_actions` | `['alert', 'nudge']` |
| `alert_family_actions` | `['alert']` |

### Arm `a3` — manipulation: two-phase pane content in ONE sandbox / ONE running watchdog: phase A prints a documented generic phrase; phase B prints nonmatching lines that displace it

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T15:06:33Z` |
| `end_utc` | `2026-08-20T15:07:39Z` |
| `phaseA_phrase_literal` | `429 Too Many Requests` |
| `phaseA_phrase_sha256` | `9bfee574ec1da5f37ce1af2779f320181faeed87153d2a04799b3cb6e315f0c0` |
| `producer_pane_view_cmd` | `capture-pane -p -J -S -40 -E -` |
| `phaseA_inject_utc` | `2026-08-20T15:06:52Z` |
| `phaseA_producer_view_phrase_occurrences` | `1` |
| `phaseB_fill_lines` | `200` |
| `phaseB_inject_utc` | `2026-08-20T15:07:18Z` |
| `phaseB_producer_view_phrase_occurrences` | `0` |
| `phaseB_producer_view_sha256` | `24344cf608581711a1febc10a80f646fd58d38d1401122fb27436e334443fc6c` |
| `final_aefake_pids` | `54970 ` |
| `barriers_crossed` | `11` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a3/run-manifest.txt` — knobs, hashes, barrier ledger
- `a3/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a3/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a3/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a3/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a3/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a3/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a3/meta.at-launch.txt`, `a3/meta.final.txt` — session meta bytes
- `a3/ae-launch.out`, `a3/ae-launch.err` — launch stdout/stderr
- `a3/agent-stdin.log` — the bytes the pane RECEIVED (fake agent's stdin, no echo)
- `a3/producer-view.phaseA-injected.txt` — positive capture of the producer's pane view at that point
- `a3/producer-view.phaseB-displaced.txt` — positive capture of the producer's pane view at that point
- `a3/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a3/cap/events.final.jsonl` |
| `source_events_sha256` | `785ce345e5eacb2688bc0ddd0cff5526120e05b3c486a84a7c04cd9aa2c08cab` |
| `source_events_bytes` | `441` |
| `total_specimens` | `3` |
| `alert_family_specimens` | `3` |
| `all_actions` | `['alert', 'throttle-cleared', 'throttled']` |
| `alert_family_actions` | `['alert', 'throttle-cleared', 'throttled']` |

### Per-specimen enumeration (every harvested event line, individually hashed)

Machine-readable: `twd-precursor/specimens/specimens.<arm>.jsonl` (ALL lines) and
`twd-precursor/specimens/alert-specimens.<arm>.jsonl` (the alert-family subset:
`alert`, `alert-cleared`, `throttled`, `throttle-cleared`). Each record carries
`arm`, `line_no`, `byte_offset`, `byte_len_with_nl`, `byte_len_no_nl`,
`sha256_line_with_nl`, `sha256_line_no_nl`, the captured `action`/`actor`/`ref`/
`summary`/`ts` byte values, `first_seen_capture_label`, and `raw_line_no_nl`.
A field absent from the producer's bytes is recorded as the sentinel `\u0000ABSENT`,
distinct from an emitted empty string.

Alert-family specimens, by hash:

| arm | line | byte offset | len (+nl) | sha256 (line, no trailing newline) | action | actor | target/ref | first seen at |
|---|---|---|---|---|---|---|---|---|
| `a1` | 1 | 0 | 141 | `eebbacd1a49fff567274f76ad68d644a06248168961cdc257c45339861c82435` | `alert` | `_watchdog` | `ABSENT` | `obs-c1` |
| `a2` | 3 | 484 | 156 | `913a7a3ebed0a14ad34cee292e7bbfa706111ac9c45977849833e6f3dbdc3f9b` | `alert` | `_watchdog` | `ABSENT` | `obs-c13` |
| `a3` | 1 | 0 | 153 | `928a41591fce09a30408eb9a34dae36a90dfc3b4f74f21434a6be2d90f4a9ee9` | `throttled` | `_watchdog` | `ABSENT` | `phaseA-c1` |
| `a3` | 2 | 153 | 142 | `3af565f24c621cbdf3dd1700d811dacc1338516f5ef35e88cd0f78ba9d016370` | `alert` | `_watchdog` | `ABSENT` | `phaseA-c2` |
| `a3` | 3 | 295 | 146 | `7505a850bb7e8b9766c9d4478cee48407827f38f590caf9a63a499af8b999299` | `throttle-cleared` | `_watchdog` | `ABSENT` | `phaseB-c1` |

Alert-family specimen count across all arms: **5**.
Set equality against this enumeration is provable from
`twd-precursor/specimens/alert-specimens.*.jsonl` + `specimens/SHA256SUMS.txt`.


### Harness, provenance, and admissibility records (step 0)

- `twd-precursor/harness/` — the exact harness that produced the arms:
  `aefake.c` (the controllable fake agent), `lib.sh` (sandbox construction, capture
  helpers, bounded waits), `arm.sh` (common runner + run-manifest emission),
  `arm1.sh` / `arm2.sh` / `arm3.sh`, `runall.sh`, `publish.sh`, `enumerate.py`,
  `SHA256SUMS.txt`, and `NOTE-arm.sh-revision.txt` (records the only post-run edit:
  two harness label strings, no behavioral change).
- **Fake agent**: a compiled non-shell executable with its own
  `pane_current_command` identity (`aefake`), a fixed startup banner, terminal echo
  disabled so the pane is stable, real `send` deliveries accepted and written to
  `agent-stdin.log` (never echoed to the pane), and controller-driven lines printed on
  demand through a FIFO. Never a live model, never a plain shell.
- **Producer capture form**: the producer reads its pane with
  `tmux capture-pane -p -J -S -40 -E -` (frozen `ae` `_watchdog_capture_pane`); every
  `panes.<label>.txt` and `producer-view.*.txt` in this archive uses that same form.
- **Instrumentation admissibility** (cluster-plan.md global rule): no hook patch and
  no clock shim were used — the frozen `ae` copy is byte-identical to `72c7293` and
  the arms ride the documented `AE_WATCHDOG_*` knobs, so there is no inactive-hook
  equivalence obligation for this step. Harness artifacts (capture files, the fake
  agent's stdin log, the FIFO) live outside the product state that
  `fs-manifests/manifest.<label>.txt` hashes.
- **Barrier instrument self-check**: each arm proves its cycle barrier answers BOTH
  ways on that run before any capture is taken through it — a positive control (the
  next producer cycle is reached, rc 0) and a negative control (an unreachable cycle
  count times out, rc 3). Recorded per arm in `run-manifest.txt`. Two earlier harness
  revisions were discarded and re-run rather than reported: one whose barrier polled a
  frozen snapshot (could only ever time out) and one whose `grep -c` fallback emitted
  a doubled count. No capture in this archive comes from either.
- **Bounded windows**: every barrier has a fixed timeout and every expiry is recorded
  as `OUTCOME=INCONCLUSIVE` in `run-manifest.txt`. Across a1/a2/a3 the recorded
  INCONCLUSIVE count is 0.
- **Published-copy equality**: `twd-precursor/<arm>/events/events.final.jsonl` is
  byte-identical to the `source_events_file` named in
  `specimens/summary.<arm>.json` (verified by sha256 at publish time); the `/tmp`
  paths in those summaries are the scratch originals.
## Template groups G1-G11 (+G2b)

A template MEMBER is a snapshot of a whole `AE_HOME` (its `config` plus
`sessions/<session>/...`), so an arm clone is a working `AE_HOME` and the M2 bootstrap
never has to run against a protected tree. Every member is stored chmod-protected
(`a-w`) and fingerprinted twice: once before protection and once after, so a clone can be
checked against either. `_meta/<member>.modes.tsv` is the recursive manifest the
fingerprint is taken over — type, mode, content hash, symlink target, path for every
file — and it records each path's ORIGINAL mode, so a WRITABLE clone (the A8 arms) can
restore exactly what the producer wrote rather than inheriting the protection.

**Producer-derivation**: every fixture byte comes from a real frozen producer — a real
`ae` launch at 72c7293 for meta and the helper set, the real generated helpers
(`state`/`goal`/`memo`/`say`/`send`/`ask`/`review`/`reply`/`spawn`) for event bytes, the
real generated watchdog for alert bytes, and the T-WD precursor archive for the
dead/stale/throttled families. Nothing is hand-authored. Members marked *byte copy* start
as an exact copy of another member and differ only by the NAMED mutation recorded in
`_meta/<member>.mutation.txt`, which carries the before/after sha256, byte counts, and the
removed/inserted byte spans.

**Fake agent**: sessions are launched with the same compiled non-shell fake agent used in
step 0, with its banner carrying the composed-UI marker the frozen unknown-tool readiness
predicate greps for, so `spawn` and launch delivery complete rather than timing out.
No live models, no network.

| group | member | session | files | fingerprint (pre-protection) | fingerprint (protected) | construction |
|---|---|---|---|---|---|---|
| `G1` | `healthy` | `tg1` | 42 | `bbfdf6957bb62063e2c5c94fc36844bede3eeab190027799b5a11164ddeab5dd` | `07def20c11115339748598d834b4240ac0ea2c817cf713c68fc4cd09d8c38152` | 2-agent session, full launch-written meta, harvested event history with no attention-family bytes: state/goal/memo/send/ask/reply/state-done, ask answered |
| `G10` | `display-only-legacy` | `tg1` | 42 | `8d30b9a41b173a80a55376c5f840f390e50bd88bdfb5ec4dde08dfea632fa372` | `c96d9564b6d4ad60383e66c82b502eab20f249f3ace64e07d2a9070b9cd473c5` | G1/healthy byte copy; named mutation: all four routing members deleted from BOTH sides of the ask/reply pair (keyless legacy pair) |
| `G10` | `same-display-diff-routing` | `tg10` | 39 | `ed4e8f498da25df8aebefec61ef278b4afcf2fbbbce5c2a9567eae7c28496bac` | `c0e588ee2d386d1b5e55f50374cfa0bc865a7ef7562cbb5587c6ca6d80c16800` | produced LIVE — the real `spawn` helper adds a second agent with the SAME display name, then both run the real `ask`, so one display name carries two genuine routing keys (main vs spawned.0) |
| `G11` | `escapes` | `tg11` | 49 | `aac57e86f0e04b92777d0bd5364470148958dabc79e83cd386a4e5bece0597f3` | `84b2df23b1fdaadc629f79332a3d956e294a0e88a09cfd8a9e0b362cfffb6d5a` | five escape classes (quote, backslash, newline, tab, CR) each fed to the real say/memo/send helpers; the producer INPUT bytes are stored beside the emitted bytes |
| `G2` | `blocked` | `tg2bl` | 33 | `9935089d5c95d32398ba6d7d5ff1a03be2b0be3a0fc35b07215c4d368e248090` | `cde5c018d6ee1ff88a9a39f4c3ecb3943d8160d1b83f048c3280ce8556d7376e` | fresh 2-agent launch; the worker runs the real `state blocked` helper |
| `G2` | `dead` | `twda1` | 32 | `cd8a3a64cd530850f5ad8641ab8209344d0afa4ad50b6414cec489255792cb53` | `eff60da558789b3fbac3ae2794105894d058faeeace5bd8399348d31d3b359f5` | AE_HOME of T-WD arm a1 as the producer left it (watchdog alert bytes) |
| `G2` | `stale` | `twda2` | 37 | `c6085b8190566df8e823fc1189356879e7c183ed925ea5b9329c9a9c33df665a` | `c533c47b12dfb72506a60cf5361b55c0fc0adb1cab8a954fb82ed83f91669eb9` | AE_HOME of T-WD arm a2 as the producer left it (nudge + alert bytes) |
| `G2` | `throttled` | `twda3` | 32 | `5d3bae39c512c4415200e3729347171f62e8a6a9d95efbcd498313d5f8718fff` | `23c0a792899cf4bb169a391cdbf1ed1adf7fb2e8df5a2232f37e17c9152a3dc2` | AE_HOME of T-WD arm a3 with events.jsonl set to the producer state captured at barrier post-phaseA (named manipulation, byte diff recorded) |
| `G2` | `unanswered` | `tg2un` | 37 | `50ffdd45792750ecb7ed0c6f682aceaa6c25a7203cd41946c271024045a5c7f1` | `489b489e33e9253ee07d6e35dfe34aa66f3324a0e8bcafdf29689e31885e7ac5` | fresh 2-agent launch; a real `ask` produced under the clock hook (aged) and never replied to |
| `G2` | `waiting-user` | `tg2wu` | 37 | `24c2cbcec497f267cafc3b312fb027b99a0515a605194a21f21ffcb4bd5fd439` | `386df958c8ee6df1b30373825e7015865e78448b41f06cf29f927391de69d720` | fresh 2-agent launch; the worker runs the real `state waiting-user` helper |
| `G2b` | `competing` | `tg2b` | 38 | `58311be71fd4a5bc2db5cd2668ffe90cfba530f353dfe4c85b3729e59cbeea66` | `8636f234a0c5dddbcf5944f9c18d215c041d4b8f07343c4e19298437b9f8b391` | 3-agent session, one reason per agent, produced in arrival order T0<T1<T2 that is DESCENDING in the frozen `_attn_rank` ladder (ae@72c7293:3571-3581, comment at :3586): dead first (real watchdog alert after the fake child under bravo's pane was killed), waiting-user second, unanswered ask last; clock hook throughout |
| `G3` | `malformed-complete-line` | `tg1` | 42 | `567dc0a279c86307fd4225dc4981a55b6412db59db3cf876fe21727ba876c964` | `b7d64f72ede6cbd61dc1652eedace4128610868395befd2da67895b3fdcd9705` | G1/healthy byte copy; named mutation: one closing quote deleted inside a COMPLETE, newline-terminated event line |
| `G3` | `meta-mode-000` | `tg1` | 42 | `aedf591ba17afa050ae38b2b488c8b98d8dafcce753ae7d38f4940233858aa3c` | `4b127dd1a084858f39feae630282595ccf965dae34a6ec7544e026346b02001b` | G1/healthy byte copy; named mutation: session meta mode -> 000, content bytes untouched |
| `G4` | `no-events` | `tg1` | 41 | `5e18fb79539713c2ac885c5323f0e79b4b64f3bea11917297c3525372caca000` | `80cdb9235faac60b7aff2baa44b7366c3f5fcb858f84e47b45f8c5f7c2775a0e` | G1/healthy byte copy; named mutation: events.jsonl removed |
| `G4` | `zero-byte-events` | `tg1` | 42 | `dcb21ed0c682902efc637a29a27b8b2db165d7be874b5c261aa450c02e6780eb` | `80dfb22168682bf0de0b9be06084460f5275b33557055ec1f6977660d7379956` | G1/healthy byte copy; named mutation: events.jsonl truncated to zero bytes |
| `G5` | `m1-control` | `tg5` | 42 | `b8c3259102203f74a460ec56d466a06d068455a692c2490cfda6aee4c581de53` | `e729e933f1465f1d31332090a4eb4e583c7bfa337cb4ed6c83f1d307c4dae385` | harvested ask->reply mirror pair (lead->worker, worker->lead) plus a third-agent ask and a review, harvested so their genuine routing-key and ref bytes exist as donors for the mutations |
| `G5` | `m2-wrong-ref` | `tg5` | 42 | `daf9b7375f4c8fa7f011b24f1e7a60dac8c86dcb8e9a6ae3371fbc95b16aca6b` | `6ba86f7acb332e598e234b5337cafe73193f767eaf3d983d63375e2fed704a27` | m1 byte copy; reply `ref` -> the review's real ref (wrong ref) |
| `G5` | `m3-wrong-actor` | `tg5` | 42 | `707e1807d8750479f87847ad361f160bbe2a4386b3fdaad3bfffc554abbad135` | `7538479765c5af9959ca0230cce2222c10ede7a2e41983faf961abab97901294` | m1 byte copy; reply actor display AND actor_slot moved to the third agent together (same ref, wrong actor) |
| `G5` | `m4-wrong-target` | `tg5` | 42 | `d3dca7169df2847903530850c22960c2a26cedabe2403c27c4c1752daed54d1b` | `b3b7dbaa5fe5c51661e690ea0a0e0bef0bebffee123999c89cb693ba402300d8` | m1 byte copy; reply target display AND target_slot moved to the third agent together (same ref, correct actor, wrong target) |
| `G5` | `m5-routed-vs-routed-mismatch` | `tg5` | 42 | `88f8396d3ae64e9ed4b28c5bd1c3eecff525242bd4a64e4c479a003256aeded9` | `5a32de26102d8a47e6985b4a8498694106f74924b0e88b43021c8292f5078a85` | m1 byte copy; the ask's target_slot changed while the reply's actor_slot is left as produced — both sides routed, keys disagree |
| `G5` | `m6-mixed-routed-display` | `tg5` | 42 | `c2855c324a8ed5620d618761e0d6e1973aeb84b648eeafffd39c666872d19849` | `d9955f175f9633b6a18eaff67fcff33bc294b80ab020cfded97af08f8d5b699e` | m1 byte copy; all four routing members deleted from the reply only — one side routed, one display-only |
| `G6` | `stopped-attention` | `tg6b` | 33 | `efe1955bfc91cd0218f0245267896eef75d037554d75d47ad4a2c2892fe07ee6` | `5c55b704019a8dc261d17d467e804584c70deaf8a451ae5a8b9dd361a184d1ec` | 2-agent session whose worker declared `blocked`, then the real `ae stop` (attention-shaped history) |
| `G6` | `stopped-plain` | `tg6a` | 37 | `8120c7cb5b5c457166bc8e61a38006fd1eafb3729bffa1648657c7c9ea44b17c` | `d62e16b4d40fbc0e243064bbc918ac4f48569381fc42fd1baf2687149d1ac9a7` | 2-agent session with ordinary traffic, then the real `ae stop` |
| `G7` | `events-unknown-action` | `tg7a` | 39 | `fd9ea1661827ec84bd518462beced7e3e0b6bbd2b4b7050f29986753317ffc99` | `ae83c220053fb12e54dbaef0349905a374b0f8054d80b692b6522d966ab13a39` | produced LIVE — three unknown action values emitted by the real `send` helper through its documented `_AE_EVENT_ACTION` override (no mutation) |
| `G7` | `events-unknown-keys` | `tg1` | 42 | `2652b58f18f883e6be5facff84f3d156811b2f77fa1730ed4609c58c5e89b926` | `f7487f66c7a17ea89e7ccb5d6b08546696ea17114f3dccc4c6b729a69b0f620b` | G1/healthy byte copy; named additions: an unknown member inserted into a state event and another appended to an ask event |
| `G7` | `meta-unknown-keys` | `tg1` | 42 | `693b3ddf0e39655ea3865f3cc4c5d7e982c7d38e0189a207c67c762673fef885` | `dac4925b3cb97cf0155340bc21e9eb5df19b1034cfc7ab0a349a3740d530a511` | G1/healthy byte copy; named addition: two unknown keys appended to meta |
| `G8` | `no-trailing-newline` | `tg1` | 42 | `7981f19b685c649571d0dd6ce67a67700314083671d780a41e758d95b019448d` | `1b6467a3534677ce5a766935eaf6e1a0b5a031304bf6953f195b11f8cad36666` | G1/healthy byte copy; named mutation: the file's single trailing newline dropped |
| `G8` | `partial-trailing-record` | `tg1` | 42 | `837c6bb0547a90bf59dbda97ce6dd6367f8662811478f91bc409a23a5468d53e` | `2dd43440979581c3a6de5796a715f33460625a20024861f7421b40a4ec877575` | G1/healthy byte copy; named mutation: 40 trailing bytes truncated, leaving a partial unterminated final record |
| `G9` | `goals-distinct-ts` | `tg9` | 32 | `f8a991667313065e7c2db99cffacd14d0d2a51858b055fb85b08f7e535dc4d90` | `ffd1463af143f2191ff5fa003b830f14f18cd1d40741d1f94ceed890a1ee66d9` | four real `goal` invocations under the clock hook at 1755000000/+600/+1200/+1800, giving four distinct deterministic event timestamps |

Artifact paths (under `docs/migration/evidence/batch-c-artifacts/templates/`):

- `FINGERPRINTS.tsv` — the table above, machine-readable
- `<group>/_meta/<member>.txt` — provenance: source sandbox and session, the exact helper
  invocations and their rc, clock-hook values where used, both fingerprints
- `<group>/_meta/<member>.modes.tsv` — the recursive manifest the fingerprint is taken over
- `<group>/_meta/<member>.mutation.txt` — the named byte diff for every derived member
- `<group>/_meta/shim-inactive-equivalence.*.txt` — the per-fixture date-shim proof (below)
- `<group>/_meta/date-shim-invocations.log`, `*.date-shim-invocations.log` — every `date`
  call the fixture's producers made, as recorded by the delegate-and-log shim
- `<group>/fixture-bytes/<member>/` — the load-bearing bytes verbatim: `config`,
  `sessions/<s>/meta`, `sessions/<s>/events.jsonl`, `sessions/<s>/messages/*`, plus G11's
  producer INPUT files. The remaining files in a member are the frozen-`ae`-generated
  helper set; they are not duplicated here because `modes.tsv` hashes every one of them.
- `SHA256SUMS.txt` — hash of every published file above

### Clock hook (date-shim contract)

Used by `G9/goals-distinct-ts`, `G2/unanswered` and `G2b/competing`, and by nothing else.
The shim is PATH-first and **delegates every invocation to the real binary** except four
exact now-forms — `-u +%FT%TZ`, `-u +%Y-%m-%dT%H:%M:%SZ`, `-u +%Y%m%dT%H%M%SZ`, `+%s` —
which it substitutes from `AE_FAKE_NOW`. The substituted values are themselves produced
BY THE REAL BINARY from that epoch (`date -u -r <epoch> +FMT`), so no parsing or
formatting behaviour is ever supplied by the shim. With `AE_FAKE_NOW` unset the shim
reaches `exec $REAL "$@"` before any case analysis.

Real binary: `/bin/date`, sha256 `0c7f77e19bc79013bc5bc4e67beea3b9d546d6d09a36799c2eed2693967af8c6`.
Shim source: `templates/date-shim/date`. Build scripts: `templates/harness/`.

**Per-fixture inactive equivalence** (cluster-plan.md's rule: proven per shim fixture, not
by one token control): for each fixture the shim's delegate-and-log records every `date`
argv its producers actually invoked; each DISTINCT argv is then replayed through the shim
with `AE_FAKE_NOW` unset and through the real binary, comparing stdout, stderr and rc byte
for byte. Deterministic argv are compared once; now-forms, whose output legitimately
advances with the clock, are compared over 20 back-to-back paired trials and the match
count is reported. Results: `<group>/_meta/shim-inactive-equivalence.*.txt`. The shim's
own log files are harness artifacts and are excluded from the product-state manifests.

