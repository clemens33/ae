# B0 Design 7 — SC-511c frozen-consumer schema-evolution fixtures: run manifest

Captures only. No verdict, classification, or expected-vs-actual claim appears in
this file or in any artifact it indexes.

## STANDING DISCLOSURE (recorded per seat ruling 2026-08-20)

While looking for the byte source named by this design's `target`/`summary` cohorts
("alert events (T-WD precursor bytes)"), this worker opened
`docs/migration/evidence/twd-precursor.md` and read the whole 96-line file in one
pass — including the `SEAT ANNEX — never included in the worker brief` at its end.
The worker's value-blind rule named b0-design.md's annex, semantic-contract.md and
ownership.md; twd-precursor.md was not on the input list and the annex was not
anticipated. What was thereby read: the T-WD manipulation->reason mapping and a
pointer to `_agents_alert_reasons`' summary-substring classes. Nothing about
SC-507b / SC-511c / SC-1208 rows, no contract values.

**Design 1 (SC-507b) was complete and delivered BEFORE this read.** The seat ruling
(2026-08-20) accepted the disclosure, kept this worker eligible for all Design 7/8
work, and closed the mitigation structurally: producer-derivation already forbids the
worker authoring summary bytes, and under the same ruling the worker produced NO alert
bytes at all — every alert-family specimen here comes from cexec's seat-gated T-WD
harvest, unaltered, with the set-equality proof below.

## Frozen source of truth and environment

| Item | Value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` sha256 | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| instrumentation | NONE. Design 7 uses no hook patch: every runner is a frozen product surface driven from outside. The only harness insertions are a PATH-first `date` shim and, for telegram, a PATH-first `curl` shim — both delegate-and-log, both recorded below. |
| `date` shim | `harness/shim/date` sha256 `eb227dc6d55e55d881d9f58752d214b67d7f94176f81df472737da41547f277d` — substitutes ONLY the four frozen now-forms (`date +%s`, `date -u +%FT%TZ`, `date -u +%Y/%m/%d`, `date -u +%Y%m%dT%H%M%SZ`), delegating those substitutions to the real binary via `-r <pinned>`; every other invocation is `exec`'d to the real `date`. Every call is logged with argv + disposition. |
| real `date` | `/bin/date` sha256 `0c7f77e19bc79013bc5bc4e67beea3b9d546d6d09a36799c2eed2693967af8c6` |
| pinned consumer clock | `1787191200` (2026-08-20T02:00:00Z) for every family run |
| locale | `TZ=UTC`, `LANG=en_US.UTF-8` — see the measured environment facts below for why NOT `LANG=C` |
| environment / tool hashes | `harness/env-record.txt` |

## Measured environment facts (captures, not interpretations)

1. **`LANG=C` breaks the generated `send`/`ask`/`review` helpers' agent resolution in
   this sandbox.** With `LANG=C` the helpers answer `Error: agent '<a>' not found in
   session '<s>'` for an agent that is present, while `tmux list-panes -s -t <session>
   -F '#{pane_id}|#{@ae_agent}'` under the SAME locale returns that agent's row. Bisected
   over the arm environment one variable at a time (`TZ`, `LANG`, `AE_TMUX_SERVER`,
   `AE_TMUX_SERVER_KIND`, `AE_DATE_SHIM_SUBSTITUTE`); only `LANG` reproduced it. Design 7
   therefore runs at `LANG=en_US.UTF-8`. Design 1 ran at `LANG=C` and never reaches agent
   resolution.
2. **tmux `-t` prefix-matches.** With a session `b0d7x` present, `ae --local b0d7` took the
   session-already-exists branch (`has-session -t b0d7` succeeded against `b0d7x`) and
   created nothing. The cross-session partner session is therefore named `xpartner`, out
   of prefix range of `b0d7`.
3. **`watchdog start` does not carry the caller's `AE_WATCHDOG_*` env into the loop.** It
   launches the loop into a tmux pane; the pane banner reported the DEFAULTS
   (`interval: 60s stale: 15m max nudges: 2`) while the caller had set 2s/1m/1. The
   watchdog family therefore executes the GENERATED watchdog script directly at its `_run`
   verb, where the knobs are honoured (banner: `interval: 2s stale: 1m max nudges: 1`).
   That is execution of the generated program, not function-sourcing.
4. **The telegram daemon initialises an UNSEEN session at EOF** (ae:10133-10138, the
   do-not-replay-history invariant), so a bounded cycle consumes nothing unless its own
   `telegram/state.tsv` is seeded. The controller seeds it at offset 0 with the fixture's
   real `session_id` and the current inode; the action is recorded per run in
   `telegram-seed.txt`, and the daemon's FINAL offset is itself part of the capture.
5. **`ae compact` refuses while spawned agents are present** ("compact never retires
   someone else's worker"). The compact runner satisfies that documented precondition
   with the REAL `retire` helper first, captured as `compact-precondition-retire.*`.
6. **A single external `ae stop <name>` emits no stop-request/stop-result**; the fleet path
   (`ae stop all -y`) does. Both the fixture's stop specimens and the stop-verification
   family runner therefore use the fleet form.

## Alert-family specimens — SET EQUALITY PROOF (binding seat guard)

Before ANY whole-cohort mutation ran, this worker proved set equality between its
alert-family specimen set and cexec's five-hash enumeration. Full proof:
`alert-specimens/SET-EQUALITY-PROOF.txt`. Summary:

```
## 1. Source file integrity (specimens/SHA256SUMS.txt)
  alert-specimens.a1.jsonl     recorded=b3659d6aee643cf494d0bc0549fede6bf0e405004550021dea7c49e99a7b0ace recomputed=b3659d6aee643cf494d0bc0549fede6bf0e405004550021dea7c49e99a7b0ace MATCH
  alert-specimens.a2.jsonl     recorded=8f4e8bc1a23be1aa58e4e2ee86bbbd4af5dc22db1eee9e8879569a31a07ae67d recomputed=8f4e8bc1a23be1aa58e4e2ee86bbbd4af5dc22db1eee9e8879569a31a07ae67d MATCH
  alert-specimens.a3.jsonl     recorded=31fc09b8241915a6b149aceb7628744497ed19c28d3d6922d93b368844021d67 recomputed=31fc09b8241915a6b149aceb7628744497ed19c28d3d6922d93b368844021d67 MATCH

## 2. Specimen records read from alert-specimens.*.jsonl: 5
## 3. Hashes tabulated in batch-c-artifacts/MANIFEST.md: 5

## 4. Per-specimen recomputation over the raw bytes B0 will use
  arm=a1 line_no=1 action=alert            recomputed_no_nl=eebbacd1a49fff567274f76ad68d644a06248168961cdc257c45339861c82435 recorded_no_nl=eebbacd1a49fff567274f76ad68d644a06248168961cdc257c45339861c82435 MATCH
      with_nl recomputed=95a330470df9c871d60ee25e540a2d55ccebdcd3b0bfd4cb2c439d09bce49b13 recorded=95a330470df9c871d60ee25e540a2d55ccebdcd3b0bfd4cb2c439d09bce49b13 MATCH  byte_len_no_nl=140(recomputed 140)
  arm=a2 line_no=3 action=alert            recomputed_no_nl=913a7a3ebed0a14ad34cee292e7bbfa706111ac9c45977849833e6f3dbdc3f9b recorded_no_nl=913a7a3ebed0a14ad34cee292e7bbfa706111ac9c45977849833e6f3dbdc3f9b MATCH
      with_nl recomputed=5a75a8bff2d7630402069cfb01b4aa5b1b781ed3182eb7aa3c5c5f2e47d89202 recorded=5a75a8bff2d7630402069cfb01b4aa5b1b781ed3182eb7aa3c5c5f2e47d89202 MATCH  byte_len_no_nl=155(recomputed 155)
  arm=a3 line_no=1 action=throttled        recomputed_no_nl=928a41591fce09a30408eb9a34dae36a90dfc3b4f74f21434a6be2d90f4a9ee9 recorded_no_nl=928a41591fce09a30408eb9a34dae36a90dfc3b4f74f21434a6be2d90f4a9ee9 MATCH
      with_nl recomputed=3ae9826242d1f49132e082f4d189be4bb9007be241c32b988f2779b9498af6a4 recorded=3ae9826242d1f49132e082f4d189be4bb9007be241c32b988f2779b9498af6a4 MATCH  byte_len_no_nl=152(recomputed 152)
  arm=a3 line_no=2 action=alert            recomputed_no_nl=3af565f24c621cbdf3dd1700d811dacc1338516f5ef35e88cd0f78ba9d016370 recorded_no_nl=3af565f24c621cbdf3dd1700d811dacc1338516f5ef35e88cd0f78ba9d016370 MATCH
      with_nl recomputed=7006737ee01edfcd55256b6206cfc73a6d0f4ec6af0216558f586a38a65dcf9d recorded=7006737ee01edfcd55256b6206cfc73a6d0f4ec6af0216558f586a38a65dcf9d MATCH  byte_len_no_nl=141(recomputed 141)
  arm=a3 line_no=3 action=throttle-cleared recomputed_no_nl=7505a850bb7e8b9766c9d4478cee48407827f38f590caf9a63a499af8b999299 recorded_no_nl=7505a850bb7e8b9766c9d4478cee48407827f38f590caf9a63a499af8b999299 MATCH
      with_nl recomputed=fca5bab4399940577530932c3bcf403be87f129713328bc606013d0546bd94a1 recorded=fca5bab4399940577530932c3bcf403be87f129713328bc606013d0546bd94a1 MATCH  byte_len_no_nl=145(recomputed 145)

## 5. SET EQUALITY — B0 specimen set vs cexec's enumeration
  |B0 set| = 5   |cexec enumeration| = 5
  in cexec but not in B0 (DROPPED): none
  in B0 but not in cexec (ADDED):   none
  SET EQUALITY: PROVEN (both sets have 5 members and are identical)
    3af565f24c621cbdf3dd1700d811dacc1338516f5ef35e88cd0f78ba9d016370  arm=a3 action=alert
    7505a850bb7e8b9766c9d4478cee48407827f38f590caf9a63a499af8b999299  arm=a3 action=throttle-cleared
    913a7a3ebed0a14ad34cee292e7bbfa706111ac9c45977849833e6f3dbdc3f9b  arm=a2 action=alert
    928a41591fce09a30408eb9a34dae36a90dfc3b4f74f21434a6be2d90f4a9ee9  arm=a3 action=throttled
    eebbacd1a49fff567274f76ad68d644a06248168961cdc257c45339861c82435  arm=a1 action=alert

## 6. Emitted /tmp/aeb0/d7/alerts/alert.lines.jsonl — 5 lines, ordered by (arm, line_no), unaltered
  file sha256 = f3da7beb8edfef78574b08d30ff039cc6e1b03f8900866c29a83cf0fe4b12cbe
  order: a1/1:alert, a2/3:alert, a3/1:throttled, a3/2:alert, a3/3:throttle-cleared

## 7. Provenance recorded by cexec and carried forward unchanged
  every specimen: actor=_watchdog, ref=ABSENT (NUL-prefixed sentinel, distinct from emitted-empty)
  the throttle-cleared specimen (a3/3) is PROCESS-MEMORY-DERIVED (one running
  watchdog, subarm B) and cannot be re-derived from a fresh clone; there is no
  re-harvest without a full cexec re-run.

```

Nothing under `docs/migration/evidence/batch-c-artifacts/` was written by this worker;
the specimen files were copied out and re-hashed. The five raw lines were appended to the
fixture unaltered, in cexec's `(arm, line_no)` order, as a recorded byte diff
(`fixture/events.extension.diff`).

## Fixture (producer-derived)

| Item | Value |
|---|---|
| template AE_HOME manifest | `fixture/manifest.tsv` |
| template fingerprint | `1d244522ac56be764c691cdc5b261880d6c390275c34c74dbd840ba32c5a442e` |
| session | `b0d7` (cross-session partner: `xpartner`) |
| events.jsonl | `fixture/events.jsonl` — sha256 `0c6bca17fe243ff616f2d50d148231f7a1a6e2b35d9fe7a4f8af7780a1c56dd2` |
| events.jsonl BEFORE the alert extension | `fixture/events.pre-extension.jsonl` sha256 `fdd70756475fe2e3ffbc888141ca9be5e61a9485289b4ae54774f4071d222881` |
| producers, in order | real `ae --local` launch; `spawn`; a bounded `send` settle probe; `goal` x2; `state` working/waiting-user/blocked/done across two agents; `memo add` x2; `say`; `ask`->`reply` (closed pair); `review` (left open); `ask` (left open); a real cross-session `ask @xpartner:dummy`; real `ae stop all -y` |

Fixture size: **25 lines**. Cohort sizes (a cohort is every specimen line carrying the key):

| Key | Cohort size |
|---|---|
| `action` | 25 |
| `actor` | 25 |
| `summary` | 25 |
| `ts` | 25 |
| `target` | 14 |
| `ref` | 12 |
| `body_file` | 6 |
| `actor_session` | 5 |
| `actor_slot` | 5 |
| `target_session` | 5 |
| `target_slot` | 5 |

Action classes present: `alert`x3, `ask`x3, `chat`x1, `done`x1, `goal`x2, `memo`x2, `reply`x1, `review`x1, `send`x1, `spawn-failed`x1, `state`x5, `stop-request`x1, `stop-result`x1, `throttle-cleared`x1, `throttled`x1.

## Runners (product-layer; exact argv recorded per run in `<label>.invocation.txt`)

| Family | Frozen runner as executed |
|---|---|
| list/next | `ae list`, `ae list --json`, `ae next` (session resumed, no attached client) |
| watchdog | the GENERATED `watchdog` script executed at its `_run` verb with the documented knobs, SIGTERM-bounded, then `watchdog stop` |
| requests/state | generated `requests all\|mine\|inbox`, `state`, plus a real `reply` from the WRONG pane (refusal path) and from the target pane |
| archive/digest | `ae archive preview b0d7` |
| compact | real `ae compact b0d7 --force`, handover answered by the controller driving the REAL `reply` helper from the main pane (`AE_COMPACT_HANDOVER_SECS=45`), after the documented `retire` precondition |
| stop verification | real `ae stop all -y` |
| events-tail | the generated `events-tail` helper: POSITIVE launch barrier (banner + >=1 rendered record, bounded 25s poll), then a capture window closed by SIGTERM (the helper is `tail -f` and never exits) |
| telegram | real `ae telegram start` -> `telegram stop` -> bounded direct daemon cycle, PATH-shimmed `curl` that logs argv, stdin AND the message temp-file body, then exits 7 (never network) |
| aewatch | frozen `contrib/aewatch` `daemon --once --ae-home <sandbox>` with `[telegram] enabled = false` |

## Arms

Every family-run gets its OWN fresh clone of the template AE_HOME, fingerprinted
before the mutation. The mutation is whole-cohort: every line carrying the key.
Removal deletes the key/value pair; rename rewrites the key name to `<key>_x`;
the additive arms insert one unknown optional key into EVERY line at the named
object position. Each is applied as a byte-level edit on the raw line (no
re-serialisation, so every unrelated byte survives) and gated by the design's
mutation-validity self-check, run per line:

* removal — `decoded(mutated)` must equal `decoded(control)` minus the named key;
* rename — `decoded(mutated)` must equal `decoded(control)` with the key renamed;
* insert — `decoded(mutated)` must equal `decoded(control)` plus exactly the inserted
  pair, and the inserted key must occupy the requested object position.

A line failing its self-check makes the arm INVALID (`ARM-INVALID.txt`) and no family
runs for it. Self-check reports: `<arm>/<family>/mutation.selfcheck.txt`; byte diffs:
`mutation.bytediff`.

| Arm | Row id | Mutation | Families | Cohort | Self-check |
|---|---|---|---|---|---|
| `additive-first` | SC-511c | whole-cohort INSERT `ae_unknown_optional_key`=`b0-additive-probe-value` at object position **first** | aewatch, archive, compact, events_tail, list_next, requests_state, stop, telegram, watchdog | 25 | PASS |
| `additive-last` | SC-511c | whole-cohort INSERT `ae_unknown_optional_key`=`b0-additive-probe-value` at object position **last** | aewatch, archive, compact, events_tail, list_next, requests_state, stop, telegram, watchdog | 25 | PASS |
| `additive-middle` | SC-511c | whole-cohort INSERT `ae_unknown_optional_key`=`b0-additive-probe-value` at object position **middle** | aewatch, archive, compact, events_tail, list_next, requests_state, stop, telegram, watchdog | 25 | PASS |
| `churn` | SC-511c | tmux `set-option -p @ae_agent` on BOTH panes (post clone only); `@ae_slot` and session untouched | post.archive, post.compact, post.requests_state, pre.archive, pre.compact, pre.requests_state | - | - |
| `control` | SC-511c | none (control) | aewatch, archive, compact, events_tail, list_next, requests_state, stop, telegram, watchdog | - | - |
| `ext-body_file-remove` | SC-511c (empirical-extension lane) | whole-cohort REMOVE of `body_file` | archive, compact | 6 | PASS |
| `ext-body_file-rename` | SC-511c (empirical-extension lane) | whole-cohort RENAME `body_file` -> `body_file_x` | archive, compact | 6 | PASS |
| `key-action-remove` | SC-511c | whole-cohort REMOVE of `action` | archive, list_next, requests_state, telegram | 25 | PASS |
| `key-action-rename` | SC-511c | whole-cohort RENAME `action` -> `action_x` | archive, list_next, requests_state, telegram | 25 | PASS |
| `key-actor-remove` | SC-511c | whole-cohort REMOVE of `actor` | archive, list_next, requests_state, watchdog | 25 | PASS |
| `key-actor-rename` | SC-511c | whole-cohort RENAME `actor` -> `actor_x` | archive, list_next, requests_state, watchdog | 25 | PASS |
| `key-actor_session-remove` | SC-511c | whole-cohort REMOVE of `actor_session` | archive, compact, requests_state | 5 | PASS |
| `key-actor_session-rename` | SC-511c | whole-cohort RENAME `actor_session` -> `actor_session_x` | archive, compact, requests_state | 5 | PASS |
| `key-actor_slot-remove` | SC-511c | whole-cohort REMOVE of `actor_slot` | archive, compact, requests_state | 5 | PASS |
| `key-actor_slot-rename` | SC-511c | whole-cohort RENAME `actor_slot` -> `actor_slot_x` | archive, compact, requests_state | 5 | PASS |
| `key-ref-remove` | SC-511c | whole-cohort REMOVE of `ref` | archive, list_next, requests_state, watchdog | 12 | PASS |
| `key-ref-rename` | SC-511c | whole-cohort RENAME `ref` -> `ref_x` | archive, list_next, requests_state, watchdog | 12 | PASS |
| `key-summary-remove` | SC-511c | whole-cohort REMOVE of `summary` | archive, list_next, stop | 25 | PASS |
| `key-summary-rename` | SC-511c | whole-cohort RENAME `summary` -> `summary_x` | archive, list_next, stop | 25 | PASS |
| `key-target-remove` | SC-511c | whole-cohort REMOVE of `target` | archive, list_next, requests_state | 14 | PASS |
| `key-target-rename` | SC-511c | whole-cohort RENAME `target` -> `target_x` | archive, list_next, requests_state | 14 | PASS |
| `key-target_session-remove` | SC-511c | whole-cohort REMOVE of `target_session` | archive, requests_state | 5 | PASS |
| `key-target_session-rename` | SC-511c | whole-cohort RENAME `target_session` -> `target_session_x` | archive, requests_state | 5 | PASS |
| `key-target_slot-remove` | SC-511c | whole-cohort REMOVE of `target_slot` | archive, requests_state | 5 | PASS |
| `key-target_slot-rename` | SC-511c | whole-cohort RENAME `target_slot` -> `target_slot_x` | archive, requests_state | 5 | PASS |
| `key-ts-remove` | SC-511c | whole-cohort REMOVE of `ts` | archive, list_next, watchdog | 25 | PASS |
| `key-ts-rename` | SC-511c | whole-cohort RENAME `ts` -> `ts_x` | archive, list_next, watchdog | 25 | PASS |

### Per-family-run artifacts

Each `<arm>/<family>/` holds: `clone-fingerprint.sha256`, `manifest.before.tsv`,
`manifest.after.tsv`, `manifest.delta.diff`, `events.control.jsonl`,
`events.mutated.jsonl`, `mutation.bytediff`, `mutation.bytes.txt`,
`mutation.report.txt`, `mutation.selfcheck.txt`, `events.after.jsonl`, the family's
`<label>.invocation.txt` / `.stdout.txt` / `.stderr.txt` / `.rc.txt`, its
`date-shim.*.log`, and the `tmux.*.txt` snapshots. Family-specific extras:
`panes.txt` (requests/state), `watchdog.knobs.txt` + `watchdog-run.*` (watchdog),
`curl-shim.log` + `tg.*` + `telegram-seed.txt` (telegram), `aw.*` (aewatch),
`compact.invocation.txt` + `refs.before-compact.txt` (compact),
`churn.controller.txt` + `churn.panes.after.txt` (churn post state).

**INVALID arms:** 0
**INCONCLUSIVE markers:** 0

## Scope notes

* The consumer matrix per key is b0-design.md's discriminating-consumer column mapped
  onto the design's runner table; the additive arms run every family; the churn arm runs
  the routing/identity families only (requests/state incl. the real reply attempt,
  archive/digest, compact) per the seat ruling of 2026-08-20.
* The churn arm mutates NO file: it is two clones, and on the `post` clone the controller
  rewrites BOTH panes' `@ae_agent` via `tmux set-option -p` while `@ae_slot` and the
  session name stay untouched (the frozen shape at tests/integration@72c7293:1268-1285).
* `body_file` is carried in a separately labelled EMPIRICAL EXTENSION lane
  (`ext-body_file-*`) and is never merged into the stable-key lanes.
* Duplicate-key mutations are deliberately ABSENT: SC-510e/f are Batch C's assignment
  (seat instruction 2026-08-20).
* `ae list --json` carries `last_active_epoch`, which is derived from events.jsonl MTIME
  rather than from any event field, so it varies between runs even under the pinned clock.
  Recorded, not normalised.
