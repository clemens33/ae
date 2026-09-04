# Commands

```text
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list [--all|--stopped|--needs-attn]
                       List sessions (running by default; --all adds stopped
                       history, --needs-attn only those needing attention)
ae upgrade             Install the latest tagged immutable release; no extra arguments
ae next [--attach]     Name the top running session needing attention (read-only;
                       alias: ae jump). --attach jumps to it. Non-zero when none.
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Regenerate helper scripts and workspace.md in existing sessions
ae rename [old] <new>  Rename a running session
ae watchdog <start|stop|status> [name]
                       Toggle the stale-agent watchdog (per-session, persists across resume)
ae telegram <setup|start|stop|status>
                       Machine-global Telegram bridge — see Telegram bridge reference
ae stop [name]         Pause session, keep ae + agent conversation state for resume
ae archive preview [name]
                       Print the digest an end would archive. Read-only: writes nothing,
                       emits no event, does not stop the session
ae [name] --from <archive-uuid>
                       Start a NEW session that explicitly continues an archived one
ae compact [-f] [--digest-only] [--keep-history] [name]
                       Archive the session and start a fresh one under the SAME name,
                       continuing from that archive. Local mode only in v1.
ae end|rm [-f] [--purge-history|--keep-history] [name]
                       End session: commit, push to ae/<name>, ARCHIVE the session's memory
                       to ~/.ae/archive/<session-uuid>/, then remove ae state. KEEPS the
                       per-session claude/codex conversation files by default (token history);
                       --purge-history deletes them AND writes no archive.
ae version             Show version
ae help                Show short help
```

When run inside an ae session, `stop`, `end`, `watchdog`, `rename` and `doctor --refresh`
detect the current session automatically.

### Retired words

Three commands were cut rather than ported to the Rust core. Two keep a **refusing arm** in
the core instead of being deleted, because anything the core does not match falls through to
a launch and a launch takes the last positional as a session name — a bare `ae status` would
otherwise create a session called `status`.

| Word | What now |
|---|---|
| `ae status [name]` | Refuses (exit 2). `ae list` answers the same question from one implementation, and its per-session sub-line already carries the state, goal and attention rollup `status` printed. Inside a session, the `peek` helper shows one agent's recent output |
| `ae orchestrator` / `ae hub` | Refuses (exit 2), and prints the replacement recipe. The orchestrator is an ordinary ae session against its own config — see [the orchestrator](#the-orchestrator-companion) below |
| `ae transfer <name> <ssh-target>` | Gone, no arm. Cross-machine session sync was ruled cut rather than ported |

Any other `_`-prefixed word nobody serves also fails closed with exit 2, for the same
fall-through reason.

## Modes

ae creates sessions in one of three working-directory modes. Pick one with a flag at start time.

```bash
ae --local my-feature       # default — agents work in the current dir
ae --copy my-feature        # full cp -a; isolated copy
ae --worktree my-feature    # git worktree; lightweight branch isolation
```

See [Configuration → copy modes](../getting-started/config.md#copy-modes) for the trade-offs.

## `ae list`

Tabular view of ae sessions with per-agent health, declared state, and a
session-level `attn:<reason>` marker when a session needs attention.

The marker is a derived rollup — the single most-actionable reason across the
session's agents, by severity:

| Reason | Meaning |
|--------|---------|
| `attn:dead` | an agent's pane vanished (or the watchdog flagged it missing) |
| `attn:stale` | the watchdog gave up nudging an idle agent (max nudges) |
| `attn:waiting-user` | an agent declared it's waiting on you |
| `attn:blocked` | an agent declared it's blocked on an external dep |
| `attn:throttled` | an agent is being rate-limited upstream |
| `attn:unanswered` | an inter-agent `ask`/`review` went unanswered past the fixed 1800-second (30-minute) threshold |

(`dead`/`stale`/`throttled` reuse the watchdog's own alert events;
`waiting-user`/`blocked` are self-declared; `unanswered` flags an `ask`/`review`
whose target never replied within 1800 seconds (30 minutes) — the lowest-severity reason.)

By default it shows **running sessions only** — stopped sessions are usually the
bulk of the list and just noise for monitoring. Flags:

| Flag | Shows |
|------|-------|
| *(none)* / `--running` | running sessions only |
| `--all` | running sessions, then stopped ones |
| `--stopped` | stopped sessions only |
| `--needs-attn` | only running sessions with an `attn:` reason; aliases: `--needs-me`, `--needs`, `--attn` |
| `--active` | only running sessions with recent activity (an ae event within the fixed 300 seconds / 5 minutes); alias: `--busy` |
| `--json` | machine-readable digest (honours the filters above) |

`AE_LIST_ACTIVE_SECS` and `AE_ATTN_REQUEST_SECS` are not honoured: the core owns
`list` with fixed 300-second activity and 1800-second unanswered defaults. Restoring
operator overrides is a recorded follow-up.

For a live dashboard, wrap it with `watch`:

```bash
watch -n 10 'ae list'            # live view of running sessions
watch -n 10 'ae list --needs-attn' # only what needs your attention
```

### `--json` digest

`ae list --json` emits a single JSON object — Rust-core output for a monitoring
script or agent; no `jq` is required to produce it. The filters
(`--running`/`--all`/`--stopped`/`--needs-attn`) decide which sessions appear.

```json
{
  "schema_version": 1,
  "generated_at": "2026-05-29T14:00:00Z",
  "sessions": [
    {
      "name": "my-feature", "status": "running",
      "mode": "local", "origin": "/…", "work_dir": "/…",
      "goal": "ship the login flow", "goal_set_epoch": 1779990000,
      "branch": "feature/login", "last_active_epoch": 1780000000,
      "needs_attention": true, "attention": "blocked", "attention_rank": 3,
      "agents": [
        {"ref": "claude:lead", "alias": "claude", "name": "lead",
         "session_id": "e795c9e9", "alive": true, "state": "blocked",
         "reason": "blocked"}
      ]
    }
  ]
}
```

`attention` is the session's single most-actionable reason (see the reason
table above); each agent's `reason` is its own contribution. `goal_set_epoch`
is when the goal was last set (age it for staleness); `branch` is the
session's live git branch (from the watchdog's status segment, with a git
fallback) — together with `name`, `origin` and `mode` they give a consumer
(e.g. the orchestrator) the session's context without any manual bookkeeping.
`schema_version` lets consumers gate on shape. `attention_rank` is the numeric
severity (`dead` 6 → `unanswered` 1); richer per-agent timing fields are a
planned addition.

## `ae upgrade`

`ae upgrade` has no arguments: any extra argument is a usage error (exit 2).
To request a specific release, set a CalVer pin in the environment:

```bash
AE_VERSION=2026.8.2 ae upgrade
```

The public wrapper dispatches this repair path before its wrapper/core
pair gate, so a broken installed generation can still repair itself. Its
immutable sibling installer downloads the selected release, verifies its
checksum before extraction, and atomically publishes the matched immutable
version and current selectors.

Stopped session directories are untouched and consume the current version on
their next resume. Running sessions are reported by name as deferred until
stop and resume; upgrade never hot-rewrites loaded helpers or daemon bodies.

## `ae next` (alias `ae jump`)

The attention navigator — the action half of `ae list`. Names the single
**top-ranked running session needing attention**, using the *same* rollup and
severity ranking as `ae list` (`dead > stale > waiting-user > blocked >
throttled > unanswered`):

```text
$ ae next
my-feature  attn:blocked  rank:3  codex:coworker
```

Read-only by default (it does not change tmux focus). Exits **non-zero** with a
message when nothing needs attention, so it composes in scripts and is a clean
primitive for a future monitoring agent. Tie-break across equally-severe
sessions: most-recent activity, then session name ascending (deterministic).

With **`--attach`** (alias `--switch`) it jumps straight to that session —
`tmux switch-client` when you're already inside tmux, `tmux attach-session`
otherwise. It re-checks the session still exists first, and no-ops with a
message if you're already in it. `-h`/`--help` prints usage; an unknown argument
exits non-zero.

```text
$ ae next --attach
# → switches your tmux client to my-feature (the blocked session)
```

## `ae doctor`

Pre-flight + post-upgrade self-test. Walks a fixed checklist of `OK / WARN / FAIL` items and
returns non-zero if anything failed: the two hard dependencies (`tmux`, `git`), whether the
config parses and names a startup roster whose profiles resolve to real executables, whether
the state root's sessions are coherent, and whether each session's recorded core agrees with
the binary answering right now.

The report is the core's. The wrapper hands it the one fact the core cannot see — which bash
is running the wrapper, passed as `--bash-major` — because `ae` re-execs itself under a
modern bash when its shebang lands on macOS's 3.2, and a core probing `bash --version` would
report whatever is first on `PATH` instead.

Three rows the frozen bash `doctor` printed are **dropped rather than reported as
permanently OK**: `flock` and `timeout` are no longer ae's dependencies (the core locks with
its own `flock(2)` and times out in its own code), and there is no portability-shim layer
left to name in a `userland` row.

An upgrade needs no helper refresh: stop/resume is the generation-migration
boundary, while running watchdogs retain their loaded body until stopped and
restarted. `doctor --refresh` is an explicit repair/development mutation; do
not run it unscoped while sessions are running. After `git pull`, run `just
install` (checkout mode), or use tagged `ae upgrade`:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## `ae watchdog`

```bash
ae watchdog start my-feature
ae watchdog stop my-feature
ae watchdog status my-feature
```

`start`, `stop` and `status` are core operations, and so is the daemon they manage: the
session's `watchdog` helper is a shim that execs the core's `_watchdog-run`, which is the
whole command of the monitor pane. `ae loop` is the deprecated spelling, kept as an alias.

The [watchdog](../internals/watchdog.md) is on by default — only an explicit `false` / `no` / `off` / `0` in config or session meta keeps it off. `watchdog start` is idempotent; running it again just confirms the meta flag.

### Meta-agent (orchestrator) sweep cadence

A session marked as the fleet orchestrator with `[workspace] orchestrator = true` (or its
legacy aliases `hub = true` / `meta = true`; persisted to
its meta as `meta_agent=true`) gets a different watchdog behaviour for its **main
agent**: instead of the stale-nudge watchdog, the watchdog sends a *"run your sweep
now"* nudge every `AE_WATCHDOG_SWEEP_SEC` seconds (default 300) and never escalates
the orchestrator to a stale `attn:` alert (idle between sweeps is normal for a monitor).
Workers/spawned agents in the same session keep the normal watchdog.

Sweep nudges are **delivery-checked**. A nudge can fail to land — the target's shell
is dead (refused), or it stayed busy / a human was typing in it (abandoned after
`AE_SEND_DEFER_SEC`). A failed nudge is logged as `sweep nudge FAILED` with the
reason, and is retried after `AE_WATCHDOG_SWEEP_RETRY_SEC` (default 30) rather than
waiting a full sweep window. After `AE_WATCHDOG_SWEEP_RETRY_MAX` (default 6) fast
retries the watchdog falls back to the normal cadence and raises one
`meta-agent unreachable` alert, cleared when a nudge next lands. Delivery is
**at-least-once**: a nudge that lands but fails to write its event is retried, so the
orchestrator may occasionally sweep twice — a redundant sweep is cheap, a silently dropped
one is not.

Liveness is still guarded two ways: the dead/missing-pane checks catch a crashed
orchestrator, and a **heartbeat** check catches a *live-but-not-sweeping* orchestrator (model
stall, upstream throttle, wedge) — the orchestrator's sweep helper rewrites
`~/.ae/sessions/<orchestrator>/meta-agent-state.json` on each real sweep, and if that mtime
stops advancing past ~`2×AE_WATCHDOG_SWEEP_SEC` the watchdog raises one alert (cleared on
recovery). This is the file [`contrib/aemonitor`](../../contrib/aemonitor/) writes
by default; if you override its `--state` path for the orchestrator, point it at this same
file or the watchdog heartbeat will false-alarm. The sweep nudges use `action=nudge`,
which is **not in the default telegram include set**, so routine sweeps don't
reach your phone (a custom `include` containing `nudge` would forward them).

## The orchestrator companion

The **orchestrator** — your fleet's chief of staff: a single ae session that monitors
all your *other* ae sessions and is your one point of contact to them (it relays
your instructions to the other sessions and reports what needs you, via the
Telegram `say` channel). Once you set an objective (`objective: …` over Telegram) it also holds it, parks
your ideas, and answers `what next` — and may proactively nudge you when you drift,
but only through hard gates (concrete signal held two sweeps, a rate budget, quiet
hours, suggest-only; ignore a couple and it self-mutes for the day). See
[`contrib/aeorchestrator`](../../contrib/aeorchestrator/). It
is a monitor + relay + focus aide: per its charter it never ends/stops/edits
another session on its own, and only suggests — it dispatches nothing without
your say-so.

**`ae orchestrator` is no longer a command.** It was a trampoline, not an operation: it
scaffolded a config and a charter on `--init`, then rewrote the config path and the working
directory and fell through to the generic launch, so the orchestrator ran as an ordinary
session that happened to be named `orchestrator`. Everything it did has an owner now, and it
is not bash. Run it as what it always was:

```bash
cd ~/.ae/orchestrator && CONFIG_FILE=$PWD/orchestrator.config ae --local orchestrator
```

That is exactly what the retired command prints when you reach for it.

**Setting it up.** There is no `--init` any more. Copy the two templates from
[`contrib/aeorchestrator`](../../contrib/aeorchestrator/) into `~/.ae/orchestrator/` yourself
— `orchestrator.config` and `CHARTER.md` — and replace the charter path placeholder in the
config with the real path. The charter wires the deterministic sweep to
[`aemonitor`](../../contrib/aemonitor/), defines the objective-armed focus aide, and tells the
agent its only channel to you is `say`. The config marks the session with `[workspace]
orchestrator = true`, which is what gives its main agent the sweep cadence described above.

Config isolation is still the point of running it this way. `CONFIG_FILE` names the
orchestrator's own config and `--local` keeps it in that directory, so the global config's
`workers` never leak into the single-agent orchestrator regardless of where you started from.

**Autostart.** A launch starts the companion for you: if `~/.ae/orchestrator/orchestrator.config`
exists (or the legacy `~/.ae/meta-hub/hub.config`, which keeps running under the name `hub`
so its baked charter paths stay consistent), the core brings the orchestrator up in the
background beside the session you asked for. It is guarded three ways — the session it is
launching from is never the orchestrator itself, a verified-present orchestrator is left
alone, and a tmux server that cannot answer counts as *unknown* and refuses to start one
rather than risk a duplicate. `AE_NO_AUTOSTART=1` starts neither companion.

To talk to the orchestrator from your phone, run the [Telegram bridge](telegram.md):
plain messages route to the running orchestrator automatically (no `/use` setup), and
`/use <session> <agent>` redirects to another session when you want (`/use clear`
returns to the orchestrator) — see
[Orchestrator-centric routing](telegram.md#orchestrator-centric-routing-talk-to-the-meta-agent-not-ten-sessions).

## `ae telegram`

```bash
ae telegram setup       # interactive: writes [telegram] config + token file
ae telegram start       # spawn daemon now, persist enabled=true
ae telegram stop        # kill daemon, persist enabled=false
ae telegram status      # report intent + runtime + core + token validation
```

Machine-global daemon that bridges every ae session on this host to one Telegram chat. Single instance per machine (one `ae-telegram` tmux session). Outbound forwards filtered events to chat. Inbound (when `allowed_user_ids` is set) offers three ways to reach an agent: **reply** to a forwarded event (routes to that agent), the compact **`@session:agent <msg>`** prefix, and a sticky **`/use <session> <agent>`** default for plain messages — plus the explicit `/list` and `/session <name|id-prefix> send|ask <agent> <msg>`. All paths share the same session/agent revalidation. Inbound is from the configured private chat only — auth requires matching `from.id` + `chat.id` + a private chat.

`setup`, `start`, `stop` and `status` are core operations, and the daemon is the ae core
binary running `_telegram-run` — no `jq`, no `curl`, no extra CLI dependency. The wrapper's
preamble passes only what the core will not read for itself: which config to honour, which
home to keep state under, and which tmux server the daemon's session belongs on. See the [Telegram
bridge](telegram.md) page for setup, config schema, inbound trust boundary, and lifecycle.

## `ae rename [old] <new>`

Rename a session: the tmux session, the session directory, `session=` in meta, the
regenerated `workspace.md`, and the status bar, all under the session's lifecycle lock as one
core operation. The running tmux server stays up. `[old]` is optional — run it inside the
session you mean and the core resolves it. The new name must satisfy the session-name
grammar, and the error echoes it verbatim when it does not.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves everything on disk: ae state at `~/.ae/sessions/<name>/` plus the per-agent conversation files at `~/.claude/projects/.../<uuid>.jsonl` and `~/.codex/sessions/.../<uuid>.jsonl`. The next `ae <name>` resumes with the full conversation history.

Use this when you're done for the day or switching contexts.

**What "stopped" means.** `ae stop` resolves the session on the tmux server its own
meta records — never whichever server happens to be ambient — addresses it by exact
session id rather than by name, and verifies it is gone before saying so. If the kill
cannot be verified (the recorded server is unreachable), it fails loudly and changes
nothing rather than reporting success. `ae stop` never deletes anything: state, working
tree and agent conversation files are all preserved either way.

Addressing by exact id is not pedantry — `tmux kill-session -t proj` prefix-matches, so
a name-based stop for a session that does not exist could kill `project` instead.

### Stopping the session you are inside

`ae stop` with no name, or naming the session you are currently in, cannot be done by
the process inside it — killing the session would kill the caller mid-operation, before
it verified anything or recorded the outcome. So ae confirms, then hands the work to a
short-lived supervisor outside the pane:

```console
$ ae stop            # from inside the session
Stop 'myproject'? This kills the session you are working in.
  Agents may be mid-turn: active writes and partial turns can be interrupted.
  Your ae state, working tree and provider conversation files are PRESERVED —
  the guarantee is recoverability (resume from the provider's own checkpoint),
  not mid-write atomicity.
Continue? [y/N] y
Stopping 'myproject' out of pane; this pane will close.
  The outcome is recorded durably in ~/.ae/sessions/myproject/events.jsonl (action: stop-result).
```

Your pane disappears with the session, so the outcome is written to the session's event
log rather than to a terminal you can no longer see. After reattaching elsewhere:

```bash
grep '"action":"stop-result"' ~/.ae/sessions/myproject/events.jsonl | tail -1
```

Add `-y` to skip the confirmation (required when there is no terminal to ask on, e.g.
from a script running inside the session).

### Stopping every session (`ae stop all`)

`ae stop all` stops every session **ae's own metadata owns**, using each
session's recorded tmux server metadata. The public wrapper ignores ambient
`AE_TMUX_SERVER` for operational commands.

The loop always runs *outside* the calling process, whether or not the caller is one of
the targets:

```console
$ ae stop all
Stop ALL 3 ae session(s)?
  Agents may be mid-turn: active writes and partial turns can be interrupted.
  ae state, working trees and provider conversation files are PRESERVED.
Continue? [y/N] y
Stopping 3 session(s) out of process; this pane may be one of them.
  Each outcome is recorded durably in its own
  ~/.ae/sessions/<name>/events.jsonl (action: stop-result).
```

There is no flag to make it run in-process, and it never asks whether *you* are one of
the targets. That question cannot be answered honestly: a caller whose `$TMUX` and
`$TMUX_PANE` have been sanitised away is still physically in the pane that dies, and
`--pane=…` merely *selects* a pane — any process can pass any valid id, so it is not
evidence of where the caller lives. Instead of inferring the answer, ae puts the loop
somewhere nothing it kills can be running it. (`--self` and `--pane` stay meaningful for
the singular self-stop above, where the caller *is* the named target by construction.)

Two consequences worth knowing:

- **You still get a real exit status.** Every outcome is written to its own session's event
  log, and after the handoff the caller waits for those records — bounded, about 30 seconds —
  then folds them into its exit code, so a script driving `ae stop all` can branch on the
  result. If the caller was itself one of the targets it simply disappears mid-wait, having
  already printed everything it could honestly know; nothing is lost, because the records
  outlive it. If the wait times out, ae says `results pending` and keeps the handoff status
  rather than reporting a still-working supervisor as a failure. Read the records directly
  any time with:

  ```bash
  for f in ~/.ae/sessions/*/events.jsonl; do grep '"action":"stop-result"' "$f" | tail -1; done
  ```

- **A session ae cannot verify is still a target.** If a session's recorded tmux server is
  unreachable, ae does not know whether it is stopped — so it is carried into the fleet and
  its stop fails loudly in its own log, rather than being silently counted as already gone.

The set you confirm is the set that gets stopped. ae works out the fleet, shows you the
count, and then hands that exact list over — it does not look again afterwards, so a session
started while you were deciding is left alone rather than swept up in an operation nobody
approved it for. That promise is about *sessions*, not names: each entry carries the identity
of the session it named at the moment you confirmed, so ending a session and starting a new
one under the same name in the meantime leaves the newcomer running, with a recorded failure
explaining that the name changed hands. Each run also carries its own operation id, which
appears in the events it writes (`[op <uuid>]`), so two `ae stop all` runs happening at once
can each tell its own results apart from the other's.

An ae-tagged session that is visible on the current tmux server but absent from ae's
metadata is named and **not** stopped — ae will not kill something it has no record of
owning. That makes the run a partial failure (non-zero exit), and the message gives you
both ways out: adopt it with `ae doctor --refresh <name>`, or stop it explicitly by name.

### Recipe: a confirm-before stop key in tmux

ae deliberately ships no keybinding — the trigger belongs in *your* tmux config, so it
never fights your prefix or your muscle memory. ae owns the semantics; you own the key.

```tmux
# ~/.tmux.conf — prefix + S: stop the current ae session, with tmux's own confirmation.
bind-key S confirm-before -p "stop this ae session? (y/n)" \
  "run-shell 'ae stop -y --self --pane=#{pane_id}'"
```

Note what the command does **not** contain: a session name. `#{session_name}` is a
tmux format expanded by tmux and pasted into a shell string, and the binding is global —
so a session named with a quote or a `$(…)` would reach the shell, from any session, ae
or not. The no-name form sidesteps that entirely: ae resolves the target itself, and no
tmux-controlled text ever enters a shell program.

`confirm-before` does the asking, which is why the inner command passes `-y`.

`--self` is required because a `run-shell` child has no controlling terminal, so ae
cannot use its usual proof that you are in the pane. The flag waives **that one check**
and nothing else — ae still proves your server is the session's recorded server and that
the pane is that session.

`--pane=#{pane_id}` is required because `$TMUX_PANE` lies here: a `run-shell` child
inherits it from the tmux server's own environment, so it names some other pane
entirely (measured — a child targeted at one pane received the id of another). Only a
format the server expands for the target is trustworthy. Unlike `#{session_name}`, a
pane id is tmux-generated and shape-checked (`%3`), so nothing attacker-influenced
enters the command. The stop itself still runs out of pane, so it completes and records its
result even though `run-shell`'s own child would not survive the session it kills.

If a stop refuses, it names the check that failed rather than only saying no — e.g.
`refusing: C4 — pane %0 is in 'alpha', not 'beta'`. The identity checks are: you are
inside tmux with a pane id (C1), your tmux server answers for itself (C2), it is the
session's recorded server (C3), your pane is in that session (C4), and your controlling
terminal is that pane's (C5, the one `--self` waives). The named fact tells you which
one to fix.

End a session for good. Removes ae's own state; **keeps the agent conversation
history by default**. If you want to resume later, use `ae stop` instead.

Wraps up:

1. Commits any pending changes in the working tree (or worktree).
2. Pushes to a branch named `ae/<session-name>` on the remote.
3. Kills the tmux session.
4. **Archives the session's memory** to `~/.ae/archive/<session-uuid>/` (see
   [Session archives](#session-archives) below).
5. Removes ae state at `~/.ae/sessions/<name>/`.
5. **Keeps the per-session Claude / Codex conversation files** (jsonl + rollout) by
   default — they are the only local record of that session's token usage, retained
   for later usage/cost reporting. Purge them with `ae end --purge-history` (or set
   `[workspace] purge_agent_history = true` as the default). Tool detection uses
   `agent_bin.<slot>` from meta; Gemini and OpenCode files are always left in place.

### Controlling conversation-file cleanup

`ae end all` resolves both decisions **per session** and lists them, one line each:
which archive path that session gets (or that it gets none, and which existing archive is
deleted), and whether its conversation files are kept or deleted. The purge default comes
from each session's own config, so a single sentence about "all sessions" would have been
true of none of them.

| Precedence | Source | Effect |
|---|---|---|
| 1 (highest) | `ae end --purge-history` / `--keep-history` | Force purge / keep for this run |
| 2 | `[workspace] purge_agent_history = true\|false` | Default policy |
| 3 (default) | *(unset)* | **Keep** |

Pass `-f` to force without confirmation. `ae end all` ends every session.

## Session archives

`ae end` deletes a session's state. Everything the session *knew* — its goal, its memo,
its event log, the request payloads agents exchanged — lived only in
`~/.ae/sessions/<name>/`, so ending it used to be the moment all of that stopped
existing. An archive is that memory, kept: an inert, immutable, UUID-keyed snapshot.

```text
~/.ae/archive/<session-uuid>/     0700
  meta                            0600   sanitized session facts, GENERATED not copied
  digest.md                       0600   the human-readable summary
  memo.tsv                        0600   durable shared memory, verbatim
  events.jsonl                    0600   the raw event log, verbatim evidence
  messages/                       0700
    <ae-generated>.txt            0600   the request payload bodies the digest links
```

Four properties are worth knowing, because they are what the archive *is*:

- **Inert by validator, not by intent.** Before anything is published, ae proves the
  staged tree against an exact path whitelist: every entry is a regular file or the one
  expected directory, nothing is a symlink or a special file, no file carries an
  executable bit for *anyone*, and the meta and digest agree about what they describe.
  Helpers, `launch.*.sh`, provider session-id scratch files, locks and the generated
  `workspace.md` are all left behind — an archive is data, and it must not be possible to
  run one.
- **The meta is generated, never copied.** Live meta carries runtime coordinates that are
  meaningless or harmful in a snapshot — panes, sockets, watchdog state, launch ids — and
  in `harness_session.<slot>` it carries the *provider conversation UUID*, the one field
  that could re-open somebody's real transcript. The archive records `seat.<slot>=<name>`
  and drops the rest (a pre-v2 source instead keeps the legacy `agent.<slot>=alias:name`).
- **Capture, then delete.** The archive is published after the session is verifiably
  stopped and after git has had its say, and *before* any live state is removed. If it
  cannot be written, `ae end` fails non-zero and the whole session is still there.
- **Immutable.** An existing archive is never merged into, appended to or overwritten.
  Publication takes an atomic `mkdir` claim (`.publishing.<uuid>`), stages a payload,
  validates it, then renames it into place — so two publishers of the same id serialize
  without needing `flock`. A crash leaves the claim standing on purpose: ae refuses and
  names it rather than guess-cleaning something another publisher may still hold.

`ae end --purge-history` writes **no** archive and deletes any existing one for that
session's UUID. That is deliberate: purge means the session's traces go, and deleting the
provider transcripts while leaving the memo and every stored request payload on disk
would only have looked like privacy.

A session ae cannot **identify** — one whose `meta` is gone while its memo, events or
request payloads remain, or one whose `session_id` is present but unparseable — is
refused *before* anything is stopped, with the reason, and nothing is deleted. That
refusal does not depend on which history flag you passed: `--purge-history` on an
unidentifiable session refuses too, because "delete it" is not an answer to "which
session is this".

A session that predates session ids has nothing to lose, so ae mints one and records the
mint **in the live meta** (`session_id_origin=minted-at-end`) as well as in the archive
(`archive_id_origin=minted-at-end`, with `source_session_id` rendered `-`). The live
record is what makes a retry after a failed publication still tell the truth: by then the
id is simply present, and its presence alone cannot say who put it there.

**What you confirm is what happens.** The plan is resolved from configuration, and
configuration can change while the prompt waits — so ae resolves each target once,
freezes exactly what it showed you, and re-proves it under the lifecycle lock. `ae end
all` ends exactly the sessions it listed: one that appears after the prompt is not part
of what you agreed to. If it no longer matches,
the end refuses and prints both versions rather than carrying out an action you never
agreed to. (`-f` freezes nothing, because nothing was promised.)

A purge makes every proof a publish makes, because it is the more dangerous of the two:
the archive root must be ae's real directory (never a symlink), it acquires the same
`.publishing.<uuid>` claim so a delete cannot race a publisher's rename, the tree must
**validate** as an ae archive, and its meta must name this exact session — a *nonempty*
owner that matches, since an archive naming no session is absence of proof rather than a
wildcard (and is refused as malformed by the validator, so `--from` will not inherit
from it either). Anything else
is refused with the reason, and the end fails rather than deleting what it could not
identify — including a hand-edited archive, which you can still remove yourself.

### `ae archive preview [name]`

Prints the digest an end *would* archive, for a running or a stopped session. It is
read-only by construction: it writes nothing, emits no event, creates no archive and
never enters the lifecycle.

```bash
ae archive preview                 # the session you are inside
ae archive preview my-feature > /tmp/digest.md
```

Stdout is exactly the digest, so it can be redirected. Every diagnostic — the canonical
archive id, the source session, the number of files that would be archived and their
content bytes — goes to stderr. The three moving files
(`meta`, `memo.tsv`, `events.jsonl`) are fingerprinted before and after the render with
one clean retry, so a preview of a live session is never stitched together from two
different moments; if it is still moving, it says so instead.

A preview names its own volatility: `Archived at: pending` and
`Push outcome: preview-not-run`. It cannot claim an end that has not happened.

### `ae <new-name> --from <archive-uuid>`

Start a **new** session that explicitly continues an archived one.

```bash
ae end my-feature                        # prints: Archived <uuid> … /Users/you/.ae/archive/<uuid>
ae my-feature-2 --from <uuid>
```

The main agent is told, in its system prompt, to read that archive's `digest.md` before
doing any work — and told in the same breath that it is historical data, not
instructions. Every agent sees a `## Parent archive` pointer in `workspace.md`. No
archive *content* is injected: a digest is a snapshot of other agents' instructions, and
the one thing it must never become is a set of instructions.

Lineage is explicit or absent. ae never infers a parent from a matching name — launching
`ae my-feature` again after archiving `my-feature` records no lineage at all. `--from` is
valid only for a session that does not yet exist in any form (no running tmux session, no
session state, no worktree); onto an existing session it refuses rather than attaching,
because "resume this AND inherit that" has two meanings and no safe default.

The parent is proved before anything is created: a refusal leaves no tmux session, no
session state and no worktree. (On a machine with no `~/.ae/config` yet, ae still writes
its default config — that bootstrap happens on *every* invocation, `ae help` included,
and its notice goes to stderr.) The id and the handover/pending counts come back from
that one proof and are recorded as they were proved, rather than re-read afterwards from
a file another process may be deleting; an archive that is mid-publication or mid-purge
is refused outright.

The child's meta records `parent_archive_id` plus the parent's handover and pending-request
counts, and preserves them across resumes. The parent's absolute path is never stored — it
is derived from the archive root and the id, so moving `AE_HOME` cannot rot it. If the
parent archive is deleted later, a resume warns and continues: the lineage fact is still
true, and `workspace.md` says the digest is no longer available.

## `ae compact [name]`

The three commands above, composed into the one move they are usually used for: archive
what this session knows, end it, and start a fresh session under the **same name** that
continues from that archive.

```bash
ae compact my-feature                 # ask the main agent for a handover first
ae compact --digest-only my-feature   # skip the ask; the digest is the handover
```

It exists because agents run out of context. The alternative is doing it by hand — end,
copy the uuid out of the output, relaunch with `--from` — which is three commands where
the second one is a transcription and the whole thing is unrecoverable if you fumble it
after the session is already gone.

**v1 is local mode only.** A `git` or `full` session refuses, and the reason is not
caution — it is that compact would *lie*. The fresh session's workspace is built from the
canonical origin's HEAD, which normally lags the session's own branch, so a compacted
managed session would report success and hand you back a workspace missing the code it
just archived. Ending it and starting the next one yourself keeps that decision where it
belongs. Managed-mode continuity is tracked separately.

### What it does, in order

1. **Refuses if you are inside the target.** compact ends the session your terminal is
   attached to and starts another; one command cannot honestly hand your terminal over.
   Run it from outside, or detach first.
2. **Freezes the session's identity** — name, uuid, mode, origin, config, history policy,
   archive path — into a single tuple. Everything after this point acts on that tuple, and
   nothing is re-resolved.
3. **Confirms**, naming the archive path, the roster the *child* will start (read from the
   recorded config, not from what the source happens to be running), and what does not
   survive: panes, spawned agents, provider conversations, launch scratch.
4. **Asks the main agent for a handover** and waits for *two* facts: a reply to the request
   **and** a new `handover` memo written after the request went out. A reply alone is an
   agent saying "done" with nothing written down; a memo alone is something written with
   nobody claiming the work stopped. `--digest-only` skips this step explicitly — the
   digest is then the whole handover.
5. **Ends the session** through `ae end`'s own locked implementation — the same ordering,
   the same archive publication, the same git behaviour. Not a second process and not a
   copy of end's logic.
6. **Starts the fresh session** with `--from <uuid>`, from the recorded origin, with the
   recorded config.

### What it refuses

- Running **from inside** the target session.
- A **`git` or `full`** session (v1).
- A session with **spawned agents**. compact never retires someone else's worker — retire
  them yourself, then re-run. `--digest-only` does not weaken this.
- A session whose config enables **`purge_agent_history`**, which contradicts an operation
  whose whole purpose is keeping the record. Pass `--keep-history` to proceed.
- A session that **changed under the prompt**. The frozen tuple is re-proved twice: once
  after your answer, so a replacement session is never *messaged*, and once again under the
  lifecycle lock, so a replacement is never *stopped*. A mismatch names the field that
  moved.
- A **timed-out handover**. Nothing is stopped and nothing is archived; the request stays
  open, so re-running keeps waiting on the same one rather than sending a second.

### Its output is a contract

**stdout is empty unless the boundary was crossed.** A refusal, a decline, and a prompt
answered `n` all write nothing to it. When the compact does happen, stdout is exactly four
lines, in this order:

```text
Archived <uuid>
Archive: /Users/you/.ae/archive/<uuid>
Digest: /Users/you/.ae/archive/<uuid>/digest.md
Recovery: cd <origin> && ae --local <name> --from <uuid>
```

**What that guarantees is precise**: the archive exists, and the printed recovery command
will work. It deliberately does *not* claim the fresh session started — the relaunch can
still refuse (the name is claimable in the window between teardown and launch), and a
line on stdout asserting a launch that then failed would be worse than no line at all.
The relaunch announcement is progress, and goes to stderr with everything else.

Everything else goes to stderr: the frozen facts, the confirmation and its question,
end's own progress, the handover chatter, `Aborted.`, the relaunch announcement — and a
second copy of the `Recovery:` line, so that a broken or closed stdout cannot destroy the
only route back. Anything printed after the contract belongs to the fresh session: compact
`exec`s into the launch, so from there on you are reading the child.

Piping compact is supported, including to a consumer that exits early. A reporting failure
never suppresses the relaunch.

Because of that `exec`, compact's exit status is the launch's: in a terminal it attaches
you to the new session and exits when you detach. With no terminal to attach to, the
launch reports failure the same way a plain `ae <name>` does — the archive and the fresh
session are already there, and the `Recovery:` line names how to reach it.

`ae compact` distinguishes **declining** from **not being asked**. A typed `n` is an
answer: it prints `Aborted.` and exits 0. End-of-input is not an answer — with no stdin
(a script, cron, `< /dev/null`) compact reports that it could not obtain confirmation and
exits **non-zero**, because stdout is empty in both cases and the exit status is a
caller's only way to tell "the operator said no" from "the question never reached anyone".
Pass `-f` if you mean to proceed without being asked.

The **Recovery** line is printed *before* the relaunch is invoked, not from a failure
handler. Past that line the archive is published and the source session is gone, and the
process may `exec` into the launch and never return: a recovery command emitted from an
error path is one that does not exist at the moment it is needed. If the relaunch fails,
the line is already on your screen.

`ae compact` never deletes an archive. Not the one it just published, not an older one —
its cleanup is live session state only.

## Hidden subcommands

Everything ae does is one core operation reached through a `_`-prefixed entry: `_launch`,
`_stop`, `_end`, `_compact`, `_spawn`, `_retire`, `_send`, `_ask`, `_review`, `_reply`,
`_requests`, `_state`, `_goal`, `_memo`, `_say`, `_peek`, `_agents`, `_focus`, `_interrupt`,
`_watchdog`, `_telegram`, and the two daemon bodies `_watchdog-run` and `_telegram-run`.
The public words above and the session helpers are thin routes to them.

Don't call them directly — the core refuses any `_`-prefixed word it does not serve, with
exit 2 and before any side effect, so a typo cannot quietly become a session name.

Two entries retired with the glue cuts and are listed so an old note does not mislead:
`_recover-pending` re-attempted post-launch session-id capture by shelling back into bash;
the core now recovers in-process on every watchdog cycle. `_stop-supervisor` and
`_stop-fleet-supervisor` were the detached workers behind `ae stop` and `ae stop all`; the
core forks its own supervisor.
