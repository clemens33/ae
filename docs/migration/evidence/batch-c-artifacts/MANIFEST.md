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
| per-arm environment | `TZ=UTC`, `LANG=LC_ALL=en_US.UTF-8` (UTF-8 is required — see the correction section), scrubbed `PATH`, fresh `HOME`/`AE_HOME`, own `TMPDIR`/`TMUX_TMPDIR`, dedicated tmux socket, cleaned per arm |
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

65 assignments (batch-c-design.md v3 at `8f21a48`, slice-1d, plus SC-021 — the `ls` alias arm — added to A2): A1 gains SC-510e/f;
A7's SC-405j gains case 5 (empty-member subcases). D01–D04 concurrency cuts execute
the approved b0-design.md Designs 2–6; SC-1306a–e ride those designs per the mapping
note (1306a→D01/Design 2, 1306b→D04a/Design 5, 1306c→D04b/Design 6, 1306d→D02/Design 3,
1306e→D03/Design 4). Designs 1/7/8 are not this worker's.

## Contents

| section | status |
|---|---|
| Step 0 — T-WD producer precursor (feeds G2) | COMPLETE (below) |
| Template groups G1–G11 (+G2b) + the A1 members | COMPLETE — 36 members, all fingerprinted and chmod-protected; REBUILT under the corrected locale (below) |
| Arm group A1 (SC-509, 509b, 506, 510a–f, 511a–b, 405k) | COMPLETE, bash lane — 39 case runs, RE-RUN under the corrected locale |
| Arm groups A2–A9 | not started |

---

## Correction — the C-locale tmux defect, and what it forced

**What happened.** The scrubbed environment for consumer runs and for the template-build
sandboxes pinned `LANG=C` / `LC_ALL=C`. tmux decides its output encoding by looking for
the substring `UTF-8` in `LC_ALL` / `LC_CTYPE` / `LANG`; in a non-UTF-8 locale it
SANITISES the TAB in `-F` format output to `_`. The frozen consumer's own pane query is
TAB-separated — `tmux list-panes -s -t "$name" -F '#{@ae_agent}<TAB>#{pane_current_command}'`
at ae@72c7293:4207, and the same query in the attention rollup at :3631 — so
`IFS=$'\t' read -r ae_agent pane_cmd` received ONE field, the `_alive` map was keyed on
`fake:lead_aefake` instead of `fake:lead`, every agent rendered `alive:false`, and the
rollup independently concluded the panes had vanished (`attn:dead`). One cause, two
symptoms, and the cause was the harness environment.

**How it was isolated** (same server, same socket, same format; only the locale varies):

| environment | output |
|---|---|
| `env -i` + `LC_ALL=C LANG=C` | `x:y_sleep` — TAB mangled |
| `env -i` + `LANG=C` | `x:y_sleep` — TAB mangled |
| `env -i` + `LC_ALL=C LANG=C LC_CTYPE=UTF-8` | `x:y<TAB>sleep` — TAB preserved |
| `env -i` + `LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8` | `x:y<TAB>sleep` — TAB preserved |

**Why the first raw probe did not catch it.** The 405k raw probe established that the
panes existed, but it wrote its own output to a file instead of round-tripping through
the consumer's parse, so it proved the topology while missing that the consumer's read of
the same query was being corrupted upstream of `ae`. Necessary, not sufficient.

**What changed.** The documented minimum scrubbed environment now pins a UTF-8 locale
(`LANG=LC_ALL=en_US.UTF-8`) instead of C, in both the arm consumer environment and the
sandbox builder. `ae` is a UTF-8 program — its own output carries the origin envelope,
box drawing and separators — and tmux changes its output encoding with the locale, so C
was the wrong scrub: it silently altered the bytes the product reads. Determinism is
preserved; the locale is still pinned, just pinned to UTF-8.

**Two standing admissibility checks were added, both proven to answer in both directions
before anything is captured through them:**

1. **Consumer TAB round-trip.** Every case proves, in ITS OWN scrubbed environment and
   before any capture, that a real TAB survives the exact query the frozen consumer
   makes. Published per case as `env-tab-selfcheck.txt`, together with a PAIRED RAW
   capture of the identical probe under `LANG=LC_ALL=C` on the same throwaway server —
   both byte strings, no comparison verdict. A failed check is a HARNESS-ABORT: the case
   takes no capture at all.
2. **In-process tmux trace.** A PATH-first `tmux` shim (pure delegate-and-log — it has no
   active mode) records, per consumer invocation, the effective `AE_TMUX_SERVER` /
   `AE_TMUX_SERVER_KIND` the consumer carried, the effective locale, and the DELEGATED
   argv. A leading `-S <socket>` in that argv is the frozen ambient shim
   (ae@72c7293:80-89) having been installed; its absence would mean the call went to the
   default server. Published per consumer as `out/<label>.tmuxtrace`. Its
   inactive-equivalence is proven on each live arm's own stable topology by running the
   read-only queries through the shim and through the real binary and comparing stdout,
   stderr and rc byte for byte (`tmux-shim-equivalence.txt`).

**Blast radius and what was redone.** All template groups were rebuilt and A1 was re-run
in full under the corrected locale; both are republished here and their fingerprints
changed. The superseded fingerprint table is kept verbatim as
`templates/FINGERPRINTS.superseded-pre-locale-fix.tsv`, and the superseded captures remain
in git history rather than being reverted. **Step 0 (T-WD) was NOT rebuilt**, by ruling:
the watchdog's own pane query is `|`-separated (ae@72c7293:16511), its dead/throttle
detection is `capture-pane` based, the generated `_lib` contains no TAB-separated tmux
format at all, and the harvested bytes are hash-verified — so the reasoning is recorded
rather than the rebuild being done silently. `G2/dead`, `G2/stale` and `G2/throttled` are
copies of that archive and their fingerprints are correspondingly unchanged.

**Related observation, recorded not chased.** The b0exec `LANG=C` agent-resolution
observation is plausibly the same tmux behaviour: the frozen script has seven
TAB-separated tmux format sites — the two pane/alive walks above plus five pane-id/agent
resolution sites at :6488, :12151, :12170, :12297 and :12962. No special arm was built
for it. Instead the paired capture falls out of work already being done: every case's
`env-tab-selfcheck.txt` carries the UTF-8 and C probes side by side, and the 405k live
arm runs the ENTIRE consumer battery twice on one unchanged topology —
`consumers.s0-baseline` under UTF-8 and `consumers.s0-baseline-clocale` under C, identical
AE_HOME, socket, topology and argv. Both halves are published raw; which, if either, is
the product's intended behaviour is not decided here.

**Post-fix state.** With the locale corrected, the 405k rendering is coherent with the
topology at every stage: both agents `alive:true` at baseline and after the extra pane
appears, and `alive:false` for exactly the roster slot whose pane was killed, with
`attn:dead` appearing only at that stage. No list/status-versus-topology divergence
remains.

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
## Template groups G1-G11 (+G2b) and the A1 members

A template MEMBER is a snapshot of a whole `AE_HOME` (its `config` plus
`sessions/<session>/...`), so an arm clone is a working `AE_HOME` and the M2 bootstrap
never runs against a protected tree. Every member is stored chmod-protected (`a-w`) and
fingerprinted twice — before protection and after — over `_meta/<member>.modes.tsv`, the
recursive manifest (type, mode, content hash, symlink target, path for every file). That
manifest also records each path's ORIGINAL producer-written mode, so a WRITABLE clone
restores exactly what the producer wrote instead of inheriting the store's protection.

**Producer-derivation**: every fixture byte comes from a real frozen producer — a real
`ae` launch at 72c7293 for meta and the helper set, the real generated helpers
(`state`/`goal`/`memo`/`say`/`send`/`ask`/`review`/`reply`/`spawn`) for event bytes, the
real generated watchdog for alert and recover bytes, and the T-WD precursor archive for
the dead/stale/throttled families. Nothing is hand-authored. Members marked *byte copy*
differ from their source only by the NAMED mutation in `_meta/<member>.mutation.txt`,
which carries before/after sha256, byte counts, and the removed/inserted byte spans.

**Rebuilt under the corrected locale.** These fingerprints supersede the ones committed at
`305b16e`; the superseded table is kept verbatim as
`templates/FINGERPRINTS.superseded-pre-locale-fix.tsv` and the forcing defect is described
in the correction section above. `G2/dead`, `G2/stale` and `G2/throttled` are the only
members whose fingerprints are UNCHANGED: they are copies of the step-0 archive, which was
ruled not to be rebuilt.

| group | member | session | files | fingerprint (pre-protection) | fingerprint (protected) | superseded pre-protection fp |
|---|---|---|---|---|---|---|
| `A1` | `510b-empty-vs-omitted` | `ta1b` | 42 | `e16decdda8210de95018c5014443cd4a634969320fa3fdbba1a8bd51ff4b1163` | `a6740d4fc379cdb0b65f737fec64aeeebd9b5b8ad020d1e1f1d46cdea8ddaa04` | `(new member)` |
| `A1` | `510c-recover-ref` | `ta1c` | 33 | `616f38cb9e59690b033ee07f6e8c95dacda1b70a72c9ea05f9a8570f2e92b10f` | `68721ca91ce8dc7c8b71859582be718fc5a657668181081992694d4e4dabdbf6` | `(new member)` |
| `A1` | `510e-dupkey-known-reversed` | `tg1` | 42 | `78d1edc6a255d058aa1ed83f46cb7956a9ba14779f16f871fcbdc571a995365b` | `0d586ef284c2ea5502216f81c0310eefc345dfe389de39f3caf730d3fe15dd85` | `(new member)` |
| `A1` | `510e-dupkey-known` | `tg1` | 42 | `bb8d8aeaefeae66e260f0950cca159ccb891b6c858ce7cd138a09e0e1e0135b4` | `588c8d7d688e6bcf90045320b63caf955423750525c9f7ffde870742c2223b10` | `(new member)` |
| `A1` | `510f-dupkey-unknown-reversed` | `tg1` | 42 | `3fc90b2e6ace970f570a3c52f3c0c7bf5482745be24b311a99ab105455dd8f00` | `3a71a1286311ed968b85651dbfe2b6014f4e52e235cf900ea6f5dadb7889632a` | `(new member)` |
| `A1` | `510f-dupkey-unknown` | `tg1` | 42 | `077199567900528338bcfd0cdb0f295ddfd29709bfdea0e5f2366adce18d15d9` | `470da1778b62611b4c09fa61cc19c6d2e71af2e45aba7331c89edc22440ed804` | `(new member)` |
| `A1` | `511a-omitted-routing` | `ta1r` | 39 | `7d3465fc653cabc05248dd804ef354cf535780acca5eceb43a025903e2e03f27` | `d4852d1b4b5f683c55ebc400c4d3e9b316ae9f69d007c660a385d6a50ac13da8` | `(new member)` |
| `G1` | `healthy` | `tg1` | 42 | `075d5a2c2b065c8511193a8b16ee9d7785ae91ce8d9ba3020d7bc40a171ff667` | `c940ecaed0bba78bb6b78f7a286ed3c65f601aa18f5a0babda6d2910aa062f18` | `bbfdf6957bb62063e2c5c94fc36844bede3eeab190027799b5a11164ddeab5dd` |
| `G10` | `display-only-legacy` | `tg1` | 42 | `6de723ca980e505cb9833c1931ce95474c17192b8aea051cbc906b7fdb78de98` | `6d2ecd219feb5a0e2f77a46f9bee6b2ea5bf01e6841a438e6e096b3a9fe05d52` | `8d30b9a41b173a80a55376c5f840f390e50bd88bdfb5ec4dde08dfea632fa372` |
| `G10` | `same-display-diff-routing` | `tg10` | 39 | `285cd5137c196a605e32088c6ad4657096e6da8375fda6afc2fdf8bd88ed9785` | `296b9224a4903e3b783484f69eb46c286716ce6f1ecd7c88cd479ac2b75cfb06` | `ed4e8f498da25df8aebefec61ef278b4afcf2fbbbce5c2a9567eae7c28496bac` |
| `G11` | `escapes` | `tg11` | 49 | `9d5b086599c350ac99003dbe4baa0b326af0874c421a54e28c58ab8f042f77b0` | `b72f31d61edfe9bcec5aad8dfe1e53c69122edc2cd63c74332366d9d75ca534d` | `aac57e86f0e04b92777d0bd5364470148958dabc79e83cd386a4e5bece0597f3` |
| `G2` | `blocked` | `tg2bl` | 33 | `e8fd157ab8249d2f8cf0edbf33301acfd3d2dc53e11a3ddecdf7171965767087` | `66b8da9282e05e7cb61c70d354d3db1b4abf2d9c2d9e7fd18f4d5b5785d52a51` | `9935089d5c95d32398ba6d7d5ff1a03be2b0be3a0fc35b07215c4d368e248090` |
| `G2` | `dead` | `twda1` | 32 | `cd8a3a64cd530850f5ad8641ab8209344d0afa4ad50b6414cec489255792cb53` | `eff60da558789b3fbac3ae2794105894d058faeeace5bd8399348d31d3b359f5` | `cd8a3a64cd530850f5ad8641ab8209344d0afa4ad50b6414cec489255792cb53` (unchanged) |
| `G2` | `stale` | `twda2` | 37 | `c6085b8190566df8e823fc1189356879e7c183ed925ea5b9329c9a9c33df665a` | `c533c47b12dfb72506a60cf5361b55c0fc0adb1cab8a954fb82ed83f91669eb9` | `c6085b8190566df8e823fc1189356879e7c183ed925ea5b9329c9a9c33df665a` (unchanged) |
| `G2` | `throttled` | `twda3` | 32 | `5d3bae39c512c4415200e3729347171f62e8a6a9d95efbcd498313d5f8718fff` | `23c0a792899cf4bb169a391cdbf1ed1adf7fb2e8df5a2232f37e17c9152a3dc2` | `5d3bae39c512c4415200e3729347171f62e8a6a9d95efbcd498313d5f8718fff` (unchanged) |
| `G2` | `unanswered` | `tg2un` | 37 | `e3b001a06f56b2635329ae424bbd55ce13e796a091ff11df9099ba5be8388981` | `82949f5e4b21565eef74b5e446c9d10723eabe3148c14b19fa15e9b739e262ab` | `50ffdd45792750ecb7ed0c6f682aceaa6c25a7203cd41946c271024045a5c7f1` |
| `G2` | `waiting-user` | `tg2wu` | 37 | `c32946b6a4cb67d93fcbc31a82124ee24873a8c59119b253dc655394532f06c7` | `bc93e334397500ba409957e952ae7d502ca3ca0cdb66f6467293fb54d4815570` | `24c2cbcec497f267cafc3b312fb027b99a0515a605194a21f21ffcb4bd5fd439` |
| `G2b` | `competing` | `tg2b` | 38 | `0d7df5c73481c9aa92d568ac64990a0563e1f59dcce651b5a7237bd6d5c6debe` | `57f784caf0acc08884823490f6242c82d9f002fa3547f4e5e8a06397f89f9a86` | `58311be71fd4a5bc2db5cd2668ffe90cfba530f353dfe4c85b3729e59cbeea66` |
| `G3` | `malformed-complete-line` | `tg1` | 42 | `d10d3366168f38c4cbd6004ccef66ccecd3f7b244c84757f1fa8b7c5436b48fc` | `0be46cd1df0d89453f7ac919c28afc96373c24d4a67ad8aaf41461ecfc20aaa4` | `567dc0a279c86307fd4225dc4981a55b6412db59db3cf876fe21727ba876c964` |
| `G3` | `meta-mode-000` | `tg1` | 42 | `03fc645781ec224d20f5f6d11b0c87280197126af028f6047893d7dc8354a8f6` | `167845758dd6755ea404ebbb53bef4abc3ecdc8e0511d90ed566dd9c0169180f` | `aedf591ba17afa050ae38b2b488c8b98d8dafcce753ae7d38f4940233858aa3c` |
| `G4` | `no-events` | `tg1` | 41 | `a248386e82ffad7f33cb3ef015c058d4098cdc9b8e409e14cc5f70a982f0efed` | `dfce9e3f5e9a4d7b1fa5c26e4080109521702b9eb6f3c0cfdc5722485638e4dc` | `5e18fb79539713c2ac885c5323f0e79b4b64f3bea11917297c3525372caca000` |
| `G4` | `zero-byte-events` | `tg1` | 42 | `98d0044246b078b8bbcadb4459b2ebdfd8a47b0e7ff8fe40c625528523cc9463` | `17ebdc098212cbd2459353d53f67b7dbab1284b852a1b14cc37685b5a4c0cdba` | `dcb21ed0c682902efc637a29a27b8b2db165d7be874b5c261aa450c02e6780eb` |
| `G5` | `m1-control` | `tg5` | 42 | `28dc10300e4bb7b260b1e9273cf6bddf2189910ff4e783f4d8eb346436b4cc04` | `fad76e71451d28616d5fee1fa330a6ab23bf5a5545b0ea12a3a748f04ad72205` | `b8c3259102203f74a460ec56d466a06d068455a692c2490cfda6aee4c581de53` |
| `G5` | `m2-wrong-ref` | `tg5` | 42 | `886e4cca0949bf281e58e287359141b652ab8456c5c92bff9556a9b1b3c0cab1` | `779e7deb29e2138092219022404412d7dab0582d8f826ed41d93697fc2efa394` | `daf9b7375f4c8fa7f011b24f1e7a60dac8c86dcb8e9a6ae3371fbc95b16aca6b` |
| `G5` | `m3-wrong-actor` | `tg5` | 42 | `6757b54db877496486ca4a89d86b0ed084bbad6183bb0bf8d89baf44cd65d257` | `8b6e96f784a004283d32a4dfa03ba9e306af305c0616e333460b461f22e2a8e3` | `707e1807d8750479f87847ad361f160bbe2a4386b3fdaad3bfffc554abbad135` |
| `G5` | `m4-wrong-target` | `tg5` | 42 | `b39dd916d0937584e0bc0e88afb8f206c0ba8dceded78ae69fb83c3f6c8b76ae` | `7910a388cbfd7619f5fdce05d44707214a8d83adf4fa1dbfc9e04b983860eaaf` | `d3dca7169df2847903530850c22960c2a26cedabe2403c27c4c1752daed54d1b` |
| `G5` | `m5-routed-vs-routed-mismatch` | `tg5` | 42 | `99b236f7fa50620ae90d18c88252257a32db3478d74f7567496751f83c5a36af` | `3c8cf713fd7ca99b4ad30c3c99ab08259dbd7d0f6c9a47cfc44cb88d63526b95` | `88f8396d3ae64e9ed4b28c5bd1c3eecff525242bd4a64e4c479a003256aeded9` |
| `G5` | `m6-mixed-routed-display` | `tg5` | 42 | `dc659e622fd3217bf5276223336db604bc1f14cfd704dd64c38abc3fb4f05078` | `3683b2d418628bc0a004d576a7e3de39752aa4350002aae24a61991358277edc` | `c2855c324a8ed5620d618761e0d6e1973aeb84b648eeafffd39c666872d19849` |
| `G6` | `stopped-attention` | `tg6b` | 33 | `568271e35faf9872d49ea35517bc3b0e6f21193fa1f9bdfd0daf293457cb6ce3` | `77273b71fc99506fe4eb2b535ee890470753d2973c96f15bd50d6046e1dfc4f5` | `efe1955bfc91cd0218f0245267896eef75d037554d75d47ad4a2c2892fe07ee6` |
| `G6` | `stopped-plain` | `tg6a` | 37 | `1632a52265ea330162416eb35df2529c72bd59fb1d02cdc353265b08bb7e6b83` | `a9b599015269fb53c8773dc201fa672e3635fe7eab3496b6786b38d222446dd1` | `8120c7cb5b5c457166bc8e61a38006fd1eafb3729bffa1648657c7c9ea44b17c` |
| `G7` | `events-unknown-action` | `tg7a` | 39 | `e1f8c849e17e3ea93939c281aaab642c365b7392ac53fdc6d8bb84719b1d4e52` | `9d39d69a507af1f96ebec0443ecc78cb6c6c19e7ee784fb291b23099807dcb0f` | `fd9ea1661827ec84bd518462beced7e3e0b6bbd2b4b7050f29986753317ffc99` |
| `G7` | `events-unknown-keys` | `tg1` | 42 | `b6bf1820d37210e25d00a91fbc70d8b8117a6430cbde58ce5bac9f12851a318d` | `27f3dcb2d61df9482caab93330f5d4373842007e7d223ad1a8a114627f2c4495` | `2652b58f18f883e6be5facff84f3d156811b2f77fa1730ed4609c58c5e89b926` |
| `G7` | `meta-unknown-keys` | `tg1` | 42 | `2e6d148627736058c398bdeb40169c564a61e784ab51cadb22eb2f62547ffa50` | `e8f8e99e3976a7de9e597dc9bd76de02766171f21908b147d514c13821b97c20` | `693b3ddf0e39655ea3865f3cc4c5d7e982c7d38e0189a207c67c762673fef885` |
| `G8` | `no-trailing-newline` | `tg1` | 42 | `5ad4e8108ee97ba2d72176a1053be7f06d44f7967a47e400881d991b47e3091e` | `d644c1fc58dbfb2769cc2f06ddbf1aaab2d004a8300e8ead920ea1b0688d7b16` | `7981f19b685c649571d0dd6ce67a67700314083671d780a41e758d95b019448d` |
| `G8` | `partial-trailing-record` | `tg1` | 42 | `46ff5fa69ee89099188b7c4e72c59f460618f70c92eaf95aaa2910f7c2638b74` | `f75a4ebe46759eae10217f00cbe1a93b321ceb46f02bab401a791d98accb5b3d` | `837c6bb0547a90bf59dbda97ce6dd6367f8662811478f91bc409a23a5468d53e` |
| `G9` | `goals-distinct-ts` | `tg9` | 32 | `d7e477f10f217dcd10e9ccd418aa9af79e80222cb0e718165c1a208ebd9e1b72` | `48f844ec7dd5cde59a576e706375875462e3a56833bec4f29df0d31b8fbcb808` | `f8a991667313065e7c2db99cffacd14d0d2a51858b055fb85b08f7e535dc4d90` |

### How each member was constructed

| group | member | construction |
|---|---|---|
| `G1` | `healthy` | 2-agent session, full launch-written meta, harvested event history with no attention-family bytes: state/goal/memo/send/ask/reply/state-done, ask answered |
| `G2` | `dead` | AE_HOME of T-WD arm a1 as the producer left it (watchdog alert bytes) |
| `G2` | `stale` | AE_HOME of T-WD arm a2 as the producer left it (nudge + alert bytes) |
| `G2` | `throttled` | AE_HOME of T-WD arm a3 with events.jsonl set to the producer state captured at barrier post-phaseA (named manipulation, byte diff recorded) |
| `G2` | `waiting-user` | fresh 2-agent launch; the worker runs the real `state waiting-user` helper |
| `G2` | `blocked` | fresh 2-agent launch; the worker runs the real `state blocked` helper |
| `G2` | `unanswered` | fresh 2-agent launch; a real `ask` produced under the clock hook (aged past the 30-minute default) and never replied to |
| `G2b` | `competing` | 3-agent session, one reason per agent, produced in arrival order T0<T1<T2 that is DESCENDING in the frozen `_attn_rank` ladder (ae@72c7293:3571-3581, comment at :3586): dead first (a real watchdog alert after the fake child under bravo's pane was killed), waiting-user second, unanswered ask last; clock hook throughout |
| `G3` | `meta-mode-000` | G1/healthy byte copy; named mutation: session meta mode -> 000, content bytes untouched |
| `G3` | `malformed-complete-line` | G1/healthy byte copy; named mutation: one closing quote deleted inside a COMPLETE, newline-terminated event line |
| `G4` | `no-events` | G1/healthy byte copy; named mutation: events.jsonl removed |
| `G4` | `zero-byte-events` | G1/healthy byte copy; named mutation: events.jsonl truncated to zero bytes |
| `G5` | `m1-control` | harvested ask->reply mirror pair plus a third-agent ask and a review, harvested so their genuine routing-key and ref bytes exist as donors. The mutations READ those donor values out of the rebuilt control rather than hardcoding them |
| `G5` | `m2-wrong-ref` | m1 byte copy; reply `ref` -> the review's real ref |
| `G5` | `m3-wrong-actor` | m1 byte copy; reply actor display AND actor_slot moved to the third agent together |
| `G5` | `m4-wrong-target` | m1 byte copy; reply target display AND target_slot moved to the third agent together |
| `G5` | `m5-routed-vs-routed-mismatch` | m1 byte copy; the ask's target_slot changed while the reply's actor_slot is left as produced — both sides routed, keys disagree |
| `G5` | `m6-mixed-routed-display` | m1 byte copy; all four routing members deleted from the reply only |
| `G6` | `stopped-plain` | 2-agent session with ordinary traffic, then the real `ae stop` |
| `G6` | `stopped-attention` | 2-agent session whose worker declared `blocked`, then the real `ae stop` |
| `G7` | `meta-unknown-keys` | G1/healthy byte copy; named addition: two unknown keys appended to meta |
| `G7` | `events-unknown-keys` | G1/healthy byte copy; named additions: an unknown member inserted into a state event and another appended to an ask event |
| `G7` | `events-unknown-action` | produced LIVE — three unknown action values emitted by the real `send` helper through its documented `_AE_EVENT_ACTION` override (no mutation) |
| `G8` | `no-trailing-newline` | G1/healthy byte copy; named mutation: the file's single trailing newline dropped |
| `G8` | `partial-trailing-record` | G1/healthy byte copy; named mutation: 40 trailing bytes truncated, leaving a partial unterminated final record |
| `G9` | `goals-distinct-ts` | four real `goal` invocations under the clock hook at 1755000000/+600/+1200/+1800 |
| `G10` | `same-display-diff-routing` | produced LIVE — the real `spawn` helper adds a second agent with the SAME display name, then both run the real `ask`, so one display name carries two genuine routing keys (main vs spawned.0) |
| `G10` | `display-only-legacy` | G1/healthy byte copy; named mutation: all four routing members deleted from BOTH sides of the ask/reply pair |
| `G11` | `escapes` | five escape classes (quote, backslash, newline, tab, CR) each fed to the real say/memo/send helpers; the producer INPUT bytes are stored beside the emitted bytes |
| `A1` | `510b-empty-vs-omitted` | the same real `send` helper with its documented event overrides set to a genuinely EMPTY STRING versus left UNSET, plus `state` with an empty vs absent reason and `memo` with an empty vs absent topic |
| `A1` | `510c-recover-ref` | recover-ref bytes from the REAL producer (the generated watchdog running `ae _recover-pending`). The agent binary is the controllable fake copied to `codex` so the frozen classifier reports tool_kind=codex; a codex-shaped session log is planted as external PRODUCER INPUT, hashed and reproduced in the provenance file, written after launch so its mtime beats launch_time and carrying the launch marker ae itself injected. No harness probe runs first — it would CLAIM the pending slot and consume the recovery |
| `A1` | `510e-dupkey-known` | a KNOWN key (`summary`) appearing twice with conflicting values on one event line; both values are producer-derived byte values from that same file |
| `A1` | `510e-dupkey-known-reversed` | the same pair with MEMBER ORDER REVERSED |
| `A1` | `510f-dupkey-unknown` | the identical construction with an UNKNOWN key (`zzz_unknown`) |
| `A1` | `510f-dupkey-unknown-reversed` | the same pair with MEMBER ORDER REVERSED |
| `A1` | `511a-omitted-routing` | a cohort produced only by helpers whose emitter never sets the routing members, so routing is genuinely OMITTED at the producer. The KNOWN-routing half of the pair is G5/m1-control |

Artifact paths (under `docs/migration/evidence/batch-c-artifacts/templates/`):

- `FINGERPRINTS.tsv` and `FINGERPRINTS.superseded-pre-locale-fix.tsv`
- `<group>/_meta/<member>.txt` — provenance: source sandbox and session, the exact helper
  invocations and their rc, clock-hook values where used, both fingerprints
- `<group>/_meta/<member>.modes.tsv` — the recursive manifest the fingerprint is taken over
- `<group>/_meta/<member>.mutation.txt` — the named byte diff for every derived member
- `<group>/_meta/shim-inactive-equivalence.*.txt` and `*date-shim-invocations.log`
- `<group>/fixture-bytes/<member>/` — `config`, `sessions/<s>/meta`,
  `sessions/<s>/events.jsonl`, `sessions/<s>/messages/*`, plus G11's producer INPUT files
  and A1/510c's planted producer input. The remaining files in a member are the
  frozen-`ae`-generated helper set, hashed in `modes.tsv` rather than duplicated here
- `date-shim/date`, `harness/` — the clock hook and every build script
- `SHA256SUMS.txt`

### Clock hook (date-shim contract)

Used by `G9/goals-distinct-ts`, `G2/unanswered` and `G2b/competing`, and nothing else.
PATH-first; **delegates every invocation to the real binary** except four exact now-forms
— `-u +%FT%TZ`, `-u +%Y-%m-%dT%H:%M:%SZ`, `-u +%Y%m%dT%H%M%SZ`, `+%s` — which it
substitutes from `AE_FAKE_NOW`. The substituted values are produced BY THE REAL BINARY
from that epoch (`date -u -r <epoch> +FMT`), so the shim never supplies parsing or
formatting behaviour. With `AE_FAKE_NOW` unset it reaches `exec $REAL "$@"` before any
case analysis. Real binary `/bin/date`, sha256
`0c7f77e19bc79013bc5bc4e67beea3b9d546d6d09a36799c2eed2693967af8c6`.

**Per-fixture inactive equivalence**: the shim's delegate-and-log records every `date`
argv each fixture's producers actually invoked; every DISTINCT argv is replayed through
the shim with `AE_FAKE_NOW` unset and through the real binary, comparing stdout, stderr
and rc byte for byte. Deterministic argv are compared once; now-forms, whose output
legitimately advances with the clock, are compared over 20 back-to-back paired trials and
the match count is reported. Results per group in `_meta/shim-inactive-equivalence.*.txt`.
Shim log files are harness artifacts and are excluded from product-state manifests.

## Arm group A1 — schema/document (bash lane)

Rows executed: SC-509, SC-509b, SC-506, SC-510a, SC-510b, SC-510c, SC-510d, SC-510e,
SC-510f, SC-511a, SC-511b, SC-405k. (SC-511c is B0's and is not run here.)

Each document case is run TWICE from the same template member: once on a **protected**
clone (the design's read-only vehicle) and once on a **writable** clone whose modes are
restored to what the producer wrote. Both are published. The pair exists because a
protected clone can turn a write into a refusal rather than revealing it, while the
writable clone lets the manifest diff be the proof the design asks for; reporting only
one of the two would hide which of those two things happened.

Consumer families per case, each captured as stdout + stderr + rc + byte counts +
sha256: `ae list`, `ae list --json`, `ae list --all`, `ae list --all --json`, `ae ls`,
`ae ls --all`, `ae status <session>`, `ae next`, the session's `requests all`, its
`agents`, and its `events-tail`. `events-tail` is a streaming consumer with no one-shot
mode, so it is bounded by the harness and the stop is recorded beside its bytes.
Every consumer runs under `env -i` plus the documented minimum
(`HOME`, `AE_HOME`, `PATH`, `TZ=UTC`, `LANG=LC_ALL=en_US.UTF-8`, `TERM`, `TMUX_TMPDIR`,
`AE_TMUX_SERVER`+kind) — never inherited shell state. The exact env is published per
case as `env.txt` and the exact argv per consumer in `consumers.tsv`.

| case | clone | rows | template | clone fp = template fp | manifest diff (lines) | tmux snapshot identical | consumers |
|---|---|---|---|---|---|---|---|
| `c01-healthy-ro` | ro | SC-509,SC-509b,SC-510a | `G1/healthy` | yes | 0 | yes | 11 |
| `c01-healthy-rw` | rw | SC-509,SC-509b,SC-510a | `G1/healthy` | yes | 0 | yes | 11 |
| `c02-meta-mode-000-ro` | ro | SC-506 | `G3/meta-mode-000` | yes | 0 | yes | 11 |
| `c02-meta-mode-000-rw` | rw | SC-506 | `G3/meta-mode-000` | yes | 0 | yes | 11 |
| `c03-malformed-line-ro` | ro | SC-506,SC-509b | `G3/malformed-complete-line` | yes | 0 | yes | 11 |
| `c03-malformed-line-rw` | rw | SC-506,SC-509b | `G3/malformed-complete-line` | yes | 0 | yes | 11 |
| `c04-empty-vs-omitted-ro` | ro | SC-510b | `A1/510b-empty-vs-omitted` | yes | 0 | yes | 11 |
| `c04-empty-vs-omitted-rw` | rw | SC-510b | `A1/510b-empty-vs-omitted` | yes | 0 | yes | 11 |
| `c05-recover-ref-ro` | ro | SC-510c | `A1/510c-recover-ref` | yes | 0 | yes | 11 |
| `c05-recover-ref-rw` | rw | SC-510c | `A1/510c-recover-ref` | yes | 0 | yes | 11 |
| `c06-escapes-ro` | ro | SC-510d | `G11/escapes` | yes | 0 | yes | 11 |
| `c06-escapes-rw` | rw | SC-510d | `G11/escapes` | yes | 0 | yes | 11 |
| `c07-dupkey-known-ro` | ro | SC-510e | `A1/510e-dupkey-known` | yes | 0 | yes | 11 |
| `c07-dupkey-known-rw` | rw | SC-510e | `A1/510e-dupkey-known` | yes | 0 | yes | 11 |
| `c08-dupkey-known-rev-ro` | ro | SC-510e | `A1/510e-dupkey-known-reversed` | yes | 0 | yes | 11 |
| `c08-dupkey-known-rev-rw` | rw | SC-510e | `A1/510e-dupkey-known-reversed` | yes | 0 | yes | 11 |
| `c09-dupkey-unknown-ro` | ro | SC-510f | `A1/510f-dupkey-unknown` | yes | 0 | yes | 11 |
| `c09-dupkey-unknown-rw` | rw | SC-510f | `A1/510f-dupkey-unknown` | yes | 0 | yes | 11 |
| `c10-dupkey-unknown-rev-ro` | ro | SC-510f | `A1/510f-dupkey-unknown-reversed` | yes | 0 | yes | 11 |
| `c10-dupkey-unknown-rev-rw` | rw | SC-510f | `A1/510f-dupkey-unknown-reversed` | yes | 0 | yes | 11 |
| `c11-routing-known-ro` | ro | SC-511a,SC-511b | `G5/m1-control` | yes | 0 | yes | 11 |
| `c11-routing-known-rw` | rw | SC-511a,SC-511b | `G5/m1-control` | yes | 0 | yes | 11 |
| `c12-routing-omitted-ro` | ro | SC-511a | `A1/511a-omitted-routing` | yes | 0 | yes | 11 |
| `c12-routing-omitted-rw` | rw | SC-511a | `A1/511a-omitted-routing` | yes | 0 | yes | 11 |
| `c13-same-display-routing-ro` | ro | SC-511b | `G10/same-display-diff-routing` | yes | 0 | yes | 11 |
| `c13-same-display-routing-rw` | rw | SC-511b | `G10/same-display-diff-routing` | yes | 0 | yes | 11 |
| `c14-display-only-legacy-ro` | ro | SC-511b | `G10/display-only-legacy` | yes | 0 | yes | 11 |
| `c14-display-only-legacy-rw` | rw | SC-511b | `G10/display-only-legacy` | yes | 0 | yes | 11 |
| `c15-meta-unknown-keys-ro` | ro | SC-509,SC-509b | `G7/meta-unknown-keys` | yes | 0 | yes | 11 |
| `c15-meta-unknown-keys-rw` | rw | SC-509,SC-509b | `G7/meta-unknown-keys` | yes | 0 | yes | 11 |
| `c16-events-unknown-keys-ro` | ro | SC-509b,SC-510a | `G7/events-unknown-keys` | yes | 0 | yes | 11 |
| `c16-events-unknown-keys-rw` | rw | SC-509b,SC-510a | `G7/events-unknown-keys` | yes | 0 | yes | 11 |
| `c17-unknown-action-ro` | ro | SC-509b,SC-510a | `G7/events-unknown-action` | yes | 0 | yes | 11 |
| `c17-unknown-action-rw` | rw | SC-509b,SC-510a | `G7/events-unknown-action` | yes | 0 | yes | 11 |
| `c18-no-trailing-newline-ro` | ro | SC-509b | `G8/no-trailing-newline` | yes | 0 | yes | 11 |
| `c18-no-trailing-newline-rw` | rw | SC-509b | `G8/no-trailing-newline` | yes | 0 | yes | 11 |
| `c19-partial-tail-ro` | ro | SC-509b | `G8/partial-trailing-record` | yes | 0 | yes | 11 |
| `c19-partial-tail-rw` | rw | SC-509b | `G8/partial-trailing-record` | yes | 0 | yes | 11 |
| `c20-405k-live` | live | SC-405k | `live/no-template (live 2-agent launch + two named topology manipulations)` | - | - | - | 44 |

Artifact paths: `docs/migration/evidence/batch-c-artifacts/arms/A1/<case>/` —
`case.txt` (template + both fingerprints + clone fingerprint + verification result +
manifest-diff line count + timestamps), `env.txt`, `consumers.tsv` (label, rc, stdout
and stderr sha256 + byte counts, bounded flag, exact argv), `out/<label>.stdout` and
`out/<label>.stderr` verbatim, `manifest.before.tsv` / `manifest.after.tsv` /
`manifest.diff.txt` (recursive: type, mode, content hash, symlink target, path across
the cloned AE_HOME), `tmux.before.txt` / `tmux.after.txt`, and `A1/ledger.tsv`
mapping every case to its row ids. `SHA256SUMS.txt` hashes every published file.

### A1 additional template members

See the template construction table above for `A1/510b-empty-vs-omitted`,
`A1/510c-recover-ref`, `A1/510e-dupkey-known(-reversed)`,
`A1/510f-dupkey-unknown(-reversed)` and `A1/511a-omitted-routing`. Their fingerprints and
provenance sit in `templates/A1/_meta/` and in `templates/FINGERPRINTS.tsv`.

### SC-405k live sub-arm

`c20-405k-live` is not a template clone: a live 2-agent launch on its own tmux server with
two named topology manipulations, captured at three stages — `s0-baseline`,
`s1-extra-pane` (an EXTRA runtime pane stamped `@ae_agent=fake:ghost`, `@ae_slot=ghost.0`,
absent from meta), and `s2-extra-pane-and-missing-roster-pane` (the pane of roster slot
`worker.0` killed, its meta entry left in place). Each stage carries the full consumer
battery, the tmux snapshot, the roster lines from meta, a recursive AE_HOME manifest, and
a raw probe running the exact tmux query the frozen consumer makes from the same scrubbed
environment and socket.

Additional artifacts for this case: `env-tab-selfcheck.txt` (the TAB round-trip plus the
paired C-locale probe), `tmux-shim-equivalence.txt` (the delegate-and-log shim proven
byte-identical to the real binary on this arm's own topology), per-consumer
`out/<stage>/<label>.tmuxtrace` (effective server, kind, locale and delegated argv), and
`consumers.s0-baseline-clocale/` — the entire battery run a second time on the same
unchanged topology with the locale pinned to C, published as a paired raw capture with no
comparison verdict.
