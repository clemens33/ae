# Batch L — section L-STOP artifacts — MANIFEST

Worker: `opus5:lexec`. Maps **arm → row ids → artifact paths → fixtures → mutation diffs**
for the L-STOP (stop matrix + fleet identity) evidence run. Captures only: bytes, hashes,
recursive manifests, byte diffs, rc values, tmux argv traces, pty transcripts, process
tables, event rows. No verdicts, no expected-vs-actual statements. Seats classify.

## Run-wide provenance

Frozen commit `72c729343a0117af2968b66e1c43f89ad25fc0b2`, frozen `ae` sha256
`b7b8aa9f…`; host, interpreter, tmux, git, flock and date hashes in
`_harness/env-record.txt`; `LANG=LC_ALL=en_US.UTF-8`, `TZ=UTC`; no live models, no
network. Each arm's own disposable sandbox under `/tmp/aelx/L-STOP/<arm>/`.

## TWO instrumented copies — every arm names which one it ran under

| patch | instrumented `ae` sha256 | patch sha256 | used by |
|---|---|---|---|
| none (frozen, unmodified) | `b7b8aa9f…` (the frozen binary itself) | — | `plain-stop`, `unverifiable-kill`, `self-stop-*`, all `identity-*`, `legacy-migration-injection`, `fleet-fourth-session-in-confirmation-window`, `exit-folding-unowned-ae-tagged` |
| **L-HOOKS-v2** | `4cc428e955a1e390bdb7afb6f71e4ce86847b4b5cf98e7f8a45599401378f845` | `a0e6f75ee02a6db99a003e85291bdd8534d2aac5e26d551c8e479b59349c125f` (`_harness/hooks-v2.patch`) | `fleet-name-handoff-mid-op`, `fleet-concurrent-ops`, `exit-folding-planted-failure`, `exit-folding-results-timeout` |

`ARM.txt` in every arm carries a `hook_patch_version` field and the binary's own sha256.

**L-HOOKS-v2 is L-HOOKS-v1 plus three stop-path barriers, nothing else**
(`_harness/mkhooks2.py`): `b_stop_supervisor_entry` (the detached fleet supervisor, after
its op-id validation and before it acts), `b_stop_before_await` (the caller, immediately
before it waits on the durable per-target records), `b_stop_one_pre_kill` (a singular
stop, under the lifecycle lock, before the kill). `_H` is unchanged: it returns 0
immediately when `AE_L_HOOKS` is unset, and when active it only appends a barrier ordinal
to a harness file and optionally blocks on a release file. It never reads, hashes or
computes over product state.

### v2 admissibility — proven BEFORE any v2-hooked capture (`_admissibility/`)

| file | comparison | comparator verdict |
|---|---|---|
| `equiv-G-v2-inactive-stop.txt` | frozen vs v2 with `AE_L_HOOKS` unset, on the STOP fixture (two launches, a singular stop, a fleet stop) | NO_DIFFERENCES |
| `equiv-G-known-difference.txt` | same STOP fixture, `-y` dropped so there is no terminal to confirm on | DIFFERENCES_PRESENT |
| `equiv-H-v2-inactive-end.txt` | frozen vs v2 with `AE_L_HOOKS` unset, on the END fixture | NO_DIFFERENCES |
| `equiv-H-known-difference.txt` | same END fixture, `--purge-history` added | DIFFERENCES_PRESENT |

v1 and its four proofs (`equiv-A`, `-B`, `-E`, `-F`) are untouched. The G-control was
vacuous on its first run — an empty third argument fell through a `${3:--y}` default and
both sides ran with `-y`, so the comparator was reporting "no differences" about two
identical commands. It was fixed and re-run before either v2 reading was recorded. The
normalizer also gained one rule at this point: `.watchdog.pid`'s SIZE is masked, because a
five-digit pid and a six-digit pid are a volatile difference and not a hook effect. That
rule can only mask differences, so it cannot have flipped an earlier NO_DIFFERENCES
reading; the earlier sections keep the normalizer they ran under, hashed in their own
harness snapshots.

## Fixtures

Multi-session fleets on one dedicated recorded server per sandbox, renamed-interpreter
fake tools (a copy of `bash` named `claude`), and **a prefix-sibling pair in every
topology** (`proj` / `projx`, or `leg` / `legx`). `ARM.txt` records the topology.

**Producer-derived stop-result fixtures** (`_harness/fixtures/`, copied into the arm that
uses them): both base lines were EMITTED BY THE FROZEN PRODUCT in this worker's own
sandboxes, never hand-authored — a success line from a real `ae stop all -y`, and a
FAILURE line from a real `ae stop all -y` whose recorded socket directory had been
removed so the kill genuinely could not be verified. `PROVENANCE.txt` records both
producers.

## Common capture set (every arm)

`ARM.txt` (arm, roster ids, construction, hook patch version, binary sha256, topology,
rc values, bounds) · `<step>.stdout` / `.stderr` / `.rc` / `.invocation` ·
`1pre.*` and `3post.*` (`AE_HOME` manifest, sessions manifest, tmux snapshot, process
table filtered to the sandbox, a copy of every session's `events.jsonl`) ·
`tmux-argv.op.log` — the delegated `command tmux` argv trace, **zeroed immediately before
the operation** so it covers the op and nothing else · `preflight-tab.txt` (the blocking
environment-as-instrument proof, in the arm's own `env -i`) · `consumer-inproc.txt` +
`_ae_install_tmux_shim.extracted.sh` (rule (e)) · `stop-results.txt` (every
`action: stop-result` row per session) · `SHA256SUMS.txt`.

## Arms

### The stop matrix

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `plain-stop` | SC-835a, SC-835b, SC-835d | three real `--local` sessions on one recorded server including the prefix-sibling pair; `ae stop -y proj` with the delegate-and-log tmux shim tracing every argv | `tmux-argv.op.log`, `1pre.*`/`3post.*`, `stop-results.txt` |
| `unverifiable-kill` | SC-835c | the directory holding the recorded tmux socket is removed while the server process keeps running; a singular stop and then a fleet stop each run against it | `manipulation.txt`, `socketdir.before.tsv`, `socketdir.after.tsv`, `2op.*`, `3opall.*` |
| `self-stop-without-y` | SC-835e, SC-835f, SC-835g, SC-835h | a SHELL pane is opened inside the live target session and the controller types the implicit no-target `ae stop` into it; the prompt is awaited as a POSITIVE barrier and answered `y` | `pty.at-prompt.txt`, `pty.after-answer.txt`, `supervisor.ps-lineage.txt`, `typed.txt`, `shell-pane.txt`, events deltas |
| `self-stop-with-y` | same | the same construction with `-y`, so no prompt is involved | same set |

### The identity gate — C1..C5 planted singly

Every cell runs the IMPLICIT no-target route and captures its own output plus the tmux
argv trace. `ARM.txt` names the cell; `2op.stdout` / `2op.stderr` / `2op.rc` hold what the
invocation said.

| arm | planted condition |
|---|---|
| `identity-c1-outside-tmux` | a plain process with no `$TMUX` and no `$TMUX_PANE` |
| `identity-c2-foreign-server` | a planted `$TMUX` naming the real socket with a server pid that is not the running one (`planted-env.txt`) |
| `identity-c3-wrong-recorded-server` | the target meta's `tmux_server` repointed at another socket path, mode preserved (`mutation.txt`, `mutation.diff`), then the route runs from a genuine shell pane inside that session |
| `identity-c4-pane-in-other-session` | the route runs from a pane of a plain tmux session the controller created directly, which ae has no session directory for |
| `identity-c5-no-controlling-tty` | the route runs as a tmux `run-shell` child of the target's own pane, passing `--pane=#{pane_id}` |
| `identity-c5-self-flag` | the same `run-shell` construction with `--self -y` added |
| `identity-malformed-pane-token` | `ae stop --pane=notapane` |

All seven are labelled SC-839a–d: the design plants the five conditions singly and reads
them as one cell family, so no single cell is claimed as the primary of one id.

**Harness note, recorded because it bit this harness and not the product.** The first run
of the two `c5` cells selected their pane with `tmux list-panes -s -t proj`, and that
resolved to `projx` — the prefix sibling. Both cells were re-run with tmux's exact-match
target syntax (`-t '=proj'`) and now record `target_pane %0` in session `proj`. Only those
two cells were re-run.

### Legacy migration injection

| arm | roster ids | construction |
|---|---|---|
| `legacy-migration-injection` | SC-839e | a VALID real session is launched, then a named controller mutation moves it into the LEGACY physical direct-child shape under a name carrying quoting and command-substitution syntax with an embedded sentinel: `tmux rename-session`, a matching state-directory move, and a meta `session=` rewrite (mode preserved). C1–C4 are re-proved from a shell pane inside the migrated session, then the implicit no-name stop route runs there |

Artifacts: `hostile-name.raw` (the name byte-exact) and `hostile-name.od.txt` (`od -c`),
`migration.txt` (the three steps and the argv-safety statement), `mutation.diff` (the meta
byte diff plus the mode before/after), `2migrated.sessions.tsv`, `2migrated.tmux.txt`,
`reproof-C1-C4.txt`, `reproof-pane.txt`, `tmux-argv.op.log` (the shell-reaching argv
trace), `sentinel.1before.txt` / `sentinel.3after.txt` / `sentinel-state.txt` (a recursive
scan of the whole sandbox for the sentinel filename, before and after).

The sentinel mechanism: the name embeds `$(touch SENTINEL_TOUCHED)`, so a shell that
EVALUATES the name creates that file in its working directory — and the scan covers the
entire sandbox rather than one guessed directory. Every controller invocation passes the
name as ONE argv word and never through a shell string; the arm is identified as the
legacy-migration arm and is never an allowlisted launch.

### Fleet

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `fleet-fourth-session-in-confirmation-window` | SC-815a | `ae stop all` on a real terminal over three sessions; while the confirmation prompt is displayed the controller launches a FOURTH real session, then answers `y` | `2op.pane.at-prompt.txt`, `2during-window.tmux.txt`, `2during-window.sessions.tsv`, `stop-results.txt`, `3post.tmux.txt` |
| `fleet-name-handoff-mid-op` | SC-815b | at `b_stop_supervisor_entry` — the supervisor has validated its op id and not yet acted — the controller ENDS one confirmed target and RELAUNCHES it under the same name, then releases | `barrier.txt` (barrier key + the op id read from the process table), `at-barrier.ps.txt`, `at-barrier.tmux.txt`, `controller.txt`, `after-handoff.*`, `stop-results.txt` |
| `fleet-concurrent-ops` | SC-815c, SC-815d | run A is held at `b_stop_supervisor_entry` while run B is started; both supervisors' argv (and therefore both op ids) are read from the process table, then both are released | `opids.txt` (both supervisor argv lines verbatim), `at-barrierA.ps.txt`, `at-barrierB.ps.txt`, `barriers-pending.txt`, `2opA.*`, `2opB.*`, `stop-results.txt` |

The op id is read from the DETACHED SUPERVISOR'S OWN ARGV in the process table — a
system observation by the controller, never a hook reading product state.

### Exit folding

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `exit-folding-planted-failure` | SC-515a | the supervisor is held at its entry barrier and then killed so it writes nothing, and the controller supplies EVERY per-target record itself from the producer-harvested lines with ONLY the op id and the target name substituted — one target's record is the harvested FAILURE line, the other two the harvested SUCCESS line. The caller's bounded wait then folds them | `opid.txt`, `after-supervisor-kill.ps.txt`, `planted.diff` (the byte diff per planted line), the two harvested fixtures + `PROVENANCE.txt`, `stop-results.txt` |
| `exit-folding-results-timeout` | SC-515b | the same hold-and-kill, but nothing is planted, so no per-target record is ever written and the caller's bounded wait reaches its bound | `opid.txt`, `after-supervisor-kill.ps.txt`, `2op.stderr`, `stop-results.txt` |
| `exit-folding-unowned-ae-tagged` | SC-515c | a plain tmux session is created directly on the recorded server and given `AE_SESSION` in its environment, with no session directory; `ae stop all -y` then runs | `manipulation.txt`, `1pre.*`/`3post.*`, `2op.*` |

**Two things stated rather than left implicit.**

1. `exit-folding-results-timeout` runs the REAL bound, not a shortened one. The design's
   wording asks for a shortened bound, but at 72c7293 the results wait is called with a
   literal `30` (`_stop_fleet_await "$_opid" "$sessions" 30`) and there is no environment
   knob for it. Shortening it would require editing a constant, which is more than the
   hook-only instrumentation contract allows, so the arm waits out the frozen bound
   instead. Recorded as a deviation from the design's wording, not as a shortened bound.
2. In `planted.diff`, the failure line's PROSE still names `projx` inside the harvested
   summary text, because the named byte diff substitutes exactly two things — the op id
   and the `"target"` field — and nothing else. That residual is the harvested producer's
   own bytes, not a second mutation; the diff shows both sides in full.

## Roster coverage as executed

All 20 L-STOP roster ids have an arm above: SC-515a, SC-515b, SC-515c, SC-815a, SC-815b,
SC-815c, SC-815d, SC-835a, SC-835b, SC-835c, SC-835d, SC-835e, SC-835f, SC-835g, SC-835h,
SC-839a, SC-839b, SC-839c, SC-839d, SC-839e.

## Known limits of this section, stated

- `identity-c1-outside-tmux` and `identity-c4-pane-in-other-session` reach the usage
  message rather than a C-numbered refusal; that is what the frozen code says for those
  constructions and it is captured verbatim. No arm was reshaped to produce a
  C-numbered string.
- The watchdog is left ENABLED (the launch default) in every arm.
- The `self-stop-*` arms drive a real pane by `send-keys`; the typed text is recorded
  verbatim in `typed.txt` and contains no hostile bytes — the hostile bytes in this
  section reach ae only through the session NAME in `legacy-migration-injection`.

## Harness stability at the section boundary

`L-STOP/harness-snapshot/` is a byte copy of the shared `_harness/` (including
`fixtures/`) exactly as this section ran, hashed by `L-STOP/HARNESS-SHA256SUMS.txt`;
`L-STOP/ADMISSIBILITY-SHA256SUMS.txt` hashes the admissibility proofs it rests on.
Nothing under `L-STOP/` changes after this point.

Checksum note: this section's checksum lists are generated with `find -print0` /
`xargs -0`, because `legacy-migration-injection` deliberately produces artifact
filenames containing quotes and `$( )`. The whitespace-split form used by the earlier
sections cannot read those names; those sections have no such filenames.
