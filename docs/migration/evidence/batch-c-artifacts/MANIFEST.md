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
- **RUST lane**: NOT started. Gated on both (1) the lead's message that reviewer3 has
  passed the rust slice and (2) the frozen reviewed rust tree HASH, which will be
  recorded here with the exact invocation before any rust-lane capture. Rust lanes
  clone FRESH from the same pre-lane template fingerprints the bash lane cloned from.
  Per-lane captures are reported raw; no bash-vs-rust comparison or divergence verdict
  is produced here — divergences are paired raw artifacts.

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
| Template groups G1–G11 | not started |
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
