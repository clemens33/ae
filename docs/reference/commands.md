# Commands

```text
ae                     Attach to the fleet server's most recently used session;
                       inside that server, list the fleet instead
ae <name> [--local|--copy|--worktree] [--dir <path>] [--no-attach]
                       Start or reattach a session. --dir selects its origin;
                       --no-attach prints the exact attach command and exits
ae <name> use <alias>  Start session with a specific agent as main
ae <name> --seat <agent>=<profile> [...]
                       Replace selected launch-seat profiles; --lead and --colead
                       are shortcuts for those agent names
ae list [--all|--stopped|--needs-attn]
                       List sessions (running by default; --all adds stopped
                       history, --needs-attn only those needing attention)
ae upgrade             Install the latest tagged immutable release; no extra arguments
ae next [--attach]     Name the top running session needing attention (read-only;
                       alias: ae jump). --attach jumps to it. Non-zero when none.
ae brief [name] [--all] [--since <dur>]
                       Card a session: goal, the latest note per memo topic, each agent's
                       declared state, and who is waiting on you. Read-only
ae orchestrator        Start or reattach the orchestrator seat: a local session named
                       orchestrator, pinned first in the fleet strip
ae orchestrator --popup
                       Pick a session, then one of its agents, in a tmux menu; the
                       chosen agent's pane gets the client. Needs tmux >= 3.4
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
ae <name> --from <archive-uuid>
                       Start a NEW session that explicitly continues an archived one
ae compact [-f] [--digest-only] [--keep-history] [name]
                       Archive the session and start a fresh one under the SAME name,
                       continuing from that archive. Local mode only in v1.
ae end|rm [-f] [--purge-history|--keep-history] [name]
                       End session: commit, push to ae/<name>, ARCHIVE the session's memory
                       to ~/.ae/archive/<session-uuid>/, then remove ae state. KEEPS the
                       per-session claude/codex conversation files by default (token history);
                       --purge-history deletes them AND writes no archive.
ae version             Show version, tmux floor, auto-upgrade policy, and last check
ae help                Show short help
```

Every lead's injected role includes `NEVER BUILD`: spawned workers make every product, test and docs tree change so leads preserve context for judgment and gates.

When run inside an ae session, `stop`, `end`, `watchdog`, `rename` and `doctor --refresh`
detect the current session automatically.

When a launch name resolves to the session the caller's pane already belongs to, `ae`
prints `you are in '<name>'` and exits 0. Naming another running session still switches
the current tmux client to it.

Entering a session by any path — fleet-strip click, prefix-s, `ae next`, the orchestrator
picker, or plain `tmux switch-client`/`attach` — lands on that session's lead pane.

### Retired words

Three commands were cut rather than ported to the Rust core. Two keep a **refusing arm** in
the core instead of being deleted, because anything the core does not match falls through to
a launch and a launch takes the last positional as a session name — a bare `ae status` would
otherwise create a session called `status`.

| Word | What now |
|---|---|
| `ae status [name]` | Refuses (exit 2). `ae list` answers the same question from one implementation, and its per-session sub-line already carries the state, goal and attention rollup `status` printed. Inside a session, the `peek` helper shows one agent's recent output |
| `ae hub` | Refuses (exit 2). The orchestrator seat is [`ae orchestrator`](#ae-orchestrator); the fleet picker is [`ae orchestrator --popup`](#ae-orchestrator---popup) |
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

Use `--dir <path>` when the origin is not the shell's current directory. ae requires the path to
exist, canonicalises it, reads its `.ae/config`, and uses it as the local/copy/worktree source:

```bash
ae my-feature --worktree --dir /path/to/project --no-attach
# Session 'my-feature' started. Attach with: tmux -L ae attach -t "=my-feature"
```

`--no-attach` works for new and running sessions. It leaves the session running, prints the exact
server-aware attach command, and exits successfully. When `--dir` names an existing session, its
canonical directory must match the recorded origin; ae refuses rather than attaching to a session
owned by another directory. Without `--dir`, named-session reattach behavior stays unchanged.
A launch always needs an explicit name; `ae --dir <path>` without one is a usage error.

Use `--seat <agent>=<profile>` more than once to select profiles for this launch. The agent
must be one of its configured main/workers and the profile must exist under `[profiles]`.
`--lead <profile>` and `--colead <profile>` are shortcuts for those two agent names. ae records
the choices in session metadata, so a later stop/resume keeps them. A stopped seat can be
re-paired to another profile of the same tool kind while keeping its conversation; changing tool
kind is refused. A running session must be stopped before any seat profile changes.

See [Configuration → copy modes](../getting-started/config.md#copy-modes) for the trade-offs.

## tmux server

With no checkout override, every new session starts on ae's isolated named server:

```bash
tmux -L ae attach -t "=my-feature"
```

Outside tmux, bare `ae` attaches to this server without a session target, so tmux chooses its most
recently used session. Inside the same server, it behaves as `ae list`. From another tmux server it
prints this line; a declared socket uses the corresponding `tmux -S <path> attach` form:

```text
ae: the ae fleet is on another tmux server. Attach with: tmux -L ae attach
```

The exact `=name` target prevents tmux from falling through to prefix or pattern matching. Every
launch records its typed server pair in session metadata. A cold server is recorded as `name=ae`;
once running, tmux's absolute socket answer is recorded instead. Both spellings address the
same server, and `ae list` de-duplicates them only after proving that equality from the servers'
own socket answers. The server reads your normal `~/.tmux.conf`; ae supplies no private `-f`
configuration.

A resume first checks the recorded server. A running session stays there and keeps its pair.
A session proved absent there is rebuilt on the current launch destination, and the new pair is
published only after the replacement panes exist; a failed build leaves the old pair intact.
Metadata from before server pairs existed is checked on tmux's historical `default` server and
backfilled only when the session's ownership marker, state root, and recorded main pane all prove
it is this ae session. An unreachable server or an unowned namesake is refused rather than guessed.

Attaching follows the servers, not just the session name. Outside tmux, ae attaches on the recorded
destination. From a client on that same proven server it switches clients. From another tmux server
it prints the exact `tmux … attach -t "=name"` command and exits successfully without nesting or
trying a cross-server `switch-client`.

Checkout runs may override the destination with the typed `AE_TMUX_SERVER_KIND` /
`AE_TMUX_SERVER` pair used by `ae-dev` and the test rigs. Installed ae ignores that override.

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
`waiting-user`/`blocked` are self-declared and require a reason. Use a one-line decision
question for `waiting-user` (`<what>: <A> | <B> (recommend A because …)`); `blocked`
names blocker and unblock owner. `unanswered` flags an `ask`/`review`
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

## `ae brief`

The reading half of the session's own memory. `ae list` names sessions and marks the ones
needing attention; a brief says **why**, without opening a single pane.

```bash
ae brief              # the session this pane is in, else the whole fleet
ae brief aedev        # one session by name, running or not
ae brief --all        # every running session, most actionable first
ae brief --all --since 4h    # ... dropping topic records older than four hours
```

One card per session, plain text, no colour:

```text
aedev · running · attn:waiting-user · ae 2026.9.5 · s1-brief* · ~/projects/clemens33/ae
  goal: ship S1 of #113
  topics:
    decision    12m   lead          gate once per merge, release after both land
    parking     2h    brief         resume here: the renderer is half written
  agents:
    lead          waiting-user  12m   "which layout do you want"
    brief         blocked       8m    "awaiting lead direction"
    colead        blocked       4m    "gate needs a second provider"
  needs you:
    lead          waiting-user  12m   which layout do you want
    colead        blocked       4m    gate needs a second provider
```

| Section | What it holds |
|---|---|
| header | name, liveness, the `attn:` rollup [`ae list`](#ae-list) shows, the session's ae version, its branch (`*` when the work tree has tracked changes) and its work dir |
| `goal:` | the session's [`goal`](helpers.md), in full, or `none` |
| `topics:` | the **latest** record per `memo` topic, newest topic first — see the topic convention below |
| `agents:` | one line per roster agent: its declared state, how long ago it declared, and the reason it gave |
| `needs you:` | explicit `waiting-user`/`blocked` declarations from the session's main agent or named `colead`. Worker declarations and unanswered asks/reviews are intra-session traffic and stay out. Nothing here is inferred, so an empty section reads `none recorded` |

### The topic convention

A brief shows one line per topic, so topics are **stable and reused**, never invented per
message. The four that pay for themselves:

| Topic | What goes in it |
|---|---|
| `goal` | what this session is for, when it is longer than the one-line `goal` helper holds |
| `decision` | a ruling and the reason behind it |
| `parking` | where to resume — the record starts `resume here:` and names the next concrete action |
| `<feature>` | one topic per feature or slice an agent owns |

Write **checkpoints, not turns**: each record supersedes the last one on that topic, because
that is the only one a brief will show. The same convention is injected into every agent's
system prompt as rule 6.

### Flags and exit codes

`--since <dur>` takes `90` (seconds), `45s`, `30m`, `2h`, `3d` or `1w`, and drops topic
records older than that; a record whose timestamp does not parse is kept rather than
silently dropped. `--all` and a session name are mutually exclusive.

Exit **2** for a usage error, **1** for a name no session carries. An empty fleet is a fact
rather than a failure: `ae brief --all` says so on stderr and exits **0**.

`ae brief` writes nothing — no tmux option, no event, no meta — so it is safe to run against
a session you do not own.

## `ae upgrade`

`ae upgrade` has no arguments: any extra argument is a usage error (exit 2).
To request a specific release, set a CalVer pin in the environment:

```bash
AE_VERSION=2026.8.2 ae upgrade
```

`ae upgrade` runs ahead of the version-directory gate, so a broken installed generation can
still repair itself. It downloads the selected release, verifies its checksum before
extraction, and hands publication to that release's own core. Publication creates the
immutable `~/.ae/versions/<V>/`, migrates every session, repoints its recorded core and helper
links, restarts companion daemons, then atomically moves `~/.local/bin/ae` to the new core.
Existing agent harnesses stay running. The `install` script beside `ae-core` is only the
bootstrap for a machine with no ae yet.

Installed ae also checks quietly during validated launch/reattach, list, brief and
orchestrator use, and after watchdog verdict cycles. `[workspace] auto_upgrade = off` in the
global config disables this machine-wide; absence means `on`, project-local config cannot
override it, and `AE_NO_AUTOSTART=1` suppresses scheduling. Automatic publication accepts only
a strictly newer candidate. It uses the same verified download, downloaded-core publication,
migration and recovery path as manual upgrade. `ae version` and `ae doctor` inspect policy and
the bounded last-check record without triggering a check.

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

With **`--attach`** (alias `--switch`) it jumps straight to that session on the server recorded in
the chosen session's metadata. It uses the same attach rule as `ae <name>`: attach from outside
tmux, switch a client proved to be on that server, or print the exact attach command from a foreign
server. It re-checks the session still exists first, and no-ops with a message if you're already in
it. A legacy pre-pair session is refused here until its first ordinary resume verifies ownership
and backfills the pair. `-h`/`--help` prints usage; an unknown argument exits non-zero.

```text
$ ae next --attach
# → switches your tmux client to my-feature (the blocked session)
```

## `ae orchestrator`

Bare `ae orchestrator` starts or reattaches the local session named
`orchestrator`: the orchestrator seat. Its local overlay is always
`~/.ae/orchestrator.config`, independent of the current directory's
`.ae/config`. Set its profile globally with `[roster] orchestrator = <profile>`;
ae refuses before writing the seat when that row is missing. Ae seeds the seat
file from the embedded template on first run; the template carries only
workspace and prompt settings. Existing seat files that still carry
`[profiles]`/`[roster]` are ignored for identity; `[workspace]` and `[prompt]`
still overlay. Add `--no-attach` to build or reattach without attaching; ae prints the exact
attach command and exits successfully. The seat keeps its fixed launch shape, so `--dir` remains
an ordinary-session flag. The seat is pinned first in the status
bar's fleet strip, marked `◆`. The `--popup` form is the picker, next. From another
ae session on the same tmux server, click `ae <version>` at the bottom-right of
the status bar to jump to the orchestrator.

## `ae orchestrator --popup`

The fleet picker, drawn by tmux itself. No daemon, no polling, no dependency: one
`display-menu` built from the same [`ae list`](#ae-list) digest, thrown away when you
choose.

```text
$ ae orchestrator --popup
┌─ ae fleet — 3 running ──────────────────────────────────────────────────┐
│ gamma              dead         tmux -L ae-dev2 attach -t "=gamma"     │
│ alpha              -             2ag ship the S0 picker                 │ (1)
│ beta               stale         3ag port the watchdog                  │ (2)
└─────────────────────────────────────────────────────────────────────────┘
```

Sessions come in **attention order** — `dead > stale > waiting-user > blocked >
throttled > unanswered`, then the quiet ones — and ties break on the name, ascending, so
the list is the same on every invocation. Each row carries the session name, the attention
word (`-` when nothing wants a human, `?` when the evidence behind the marker was
incomplete), how many agents the roster holds, and the goal, cut to fit. At most 30 rows;
a note names how many were left out.

Choosing a session opens its agents, each with its declared state, its own attention
reason and its pane id:

```text
┌─ alpha — attn:- — active 13m ───────────────────────────┐
│ lead                   working      -            %0 (1) │
│ helper                 blocked      blocked      %1 (2) │
└─────────────────────────────────────────────────────────┘
```

Choosing an agent runs `switch-client`, then `select-window`, then `select-pane` on the
recorded ids — the window first, because a worker lives in its own and `select-pane` alone
does not change which window is viewed. tmux resolves all three when you choose, so a pane
that died in the meantime fails the jump instead of landing you somewhere else. The core's
own monitor panes (`_watchdog`, `_events`) are stamped outside the agent grammar and are
not listed.

**Coming back is tmux's own.** `switch-client -l` (prefix + `L`) returns the client to the
session it came from; within one session, `last-pane` (prefix + `;`) is the equivalent. ae
remembers nothing about where you were — a second answer to a question tmux already answers
is a second chance to disagree with it.

**One server only, and proven.** `switch-client` cannot cross tmux servers, and it targets a
session by NAME on the server it is given — so a session ae recorded elsewhere, with a
same-named stranger here, would otherwise take your jump. The picker compares the socket
path each server reports for itself, and a session it cannot prove is on this one becomes a
row you cannot choose, showing the `tmux … attach -t "=<name>"` command that reaches it — with
its attention word intact, because a session ae cannot reach can still be the one that needs
you.

### Bind it

```tmux
bind o run-shell "ae orchestrator --popup"
```

Measured on tmux 3.7b: `run-shell` needs neither `-c` nor `-t`. The command inherits the
pane it was bound from, and `display-menu` with no target draws on that pane's client. The
plain form above is the working one.

### The tmux floor

ae needs **tmux 3.4 or newer**, fleet-wide. A LAUNCH is gated before it writes anything —
it asks the running server (`display-message -p '#{version}'`) when one answers, and the
`tmux` binary on `PATH` (`tmux -V`) when none does, because that is the binary the launch
would start a server with. A long-lived server keeps running the binary that started it,
and the two disagree exactly when an upgrade has happened. The picker asks the server
alone: a menu needs a client.

Below the floor both refuse at exit 1, naming what was found, what is required, which
server was asked and how to get it. Neither ever starts or restarts a server for you.

`ae list`, `ae version`, `ae doctor` and `ae upgrade` keep working below the floor —
`version` prints the tmux reading on its second line and automatic-upgrade status on its third
and fourth, `doctor` carries the corresponding `tmux-floor`, `auto-upgrade` and `upgrade-check`
rows, and a publish warns without refusing. That is how a machine below the floor sees the
problem and leaves it behind.

Ubuntu 24.04 ships tmux 3.4, which clears the floor exactly, so `apt install tmux` is
enough there; Homebrew carries current. An older distro needs its backport or a build.

```text
$ ae orchestrator --popup
ae orchestrator: the tmux server it would use is older than ae runs on.
  found:    3.3a
  required: 3.4 or newer
  server:   the current server ($TMUX)
Install a newer tmux, then start a NEW server with it — ae never restarts a
running server for you.
  macOS:  brew install tmux
  Linux:  apt install tmux   (Ubuntu 24.04 ships 3.4, which clears the floor;
          an older distro needs its backport, a newer package, or a source build).
```

## `ae doctor`

Pre-flight + post-upgrade self-test. Walks a fixed checklist of `OK / WARN / FAIL` items and
returns non-zero if anything failed: the two hard dependencies (`tmux`, `git`), whether the
config parses and names a startup roster whose profiles resolve to real executables, whether
the state root's sessions are coherent, and whether each session's recorded core agrees with
the binary answering right now. Its read-only `auto-upgrade` and `upgrade-check` rows report
global policy plus missing, stale, failed or malformed check state; they never schedule work.

The report is the core's. Its bash-version row is fed through a `--bash-major` flag rather
than the core probing `bash --version` itself, which would report whatever is first on
`PATH` instead of what actually invoked it — a distinction that mattered when a Bash
wrapper re-exec'd itself under a modern bash on macOS's 3.2. That wrapper is gone as of
Z3; what now supplies `--bash-major` to a directly-run `ae-core` is being reworked with it.

Three rows the frozen bash `doctor` printed are **dropped rather than reported as
permanently OK**: `flock` and `timeout` are no longer ae's dependencies (the core locks with
its own `flock(2)` and times out in its own code), and there is no portability-shim layer
left to name in a `userland` row.

An upgrade needs no helper refresh: publication migrates and relinks every session and restarts
its companion daemons before moving the public command pointer. Existing agent harnesses retain
their loaded process. `doctor --refresh` is an explicit repair/development mutation; do not run
it unscoped while sessions are running. After `git pull`, run `just install` (checkout mode), or
use tagged `ae upgrade`:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## Session helpers

Session delivery helpers stay inside their own ae session by default:

```text
send [--cross-session] <agent> <message>
ask [--cross-session] <agent> <question>
review [--cross-session] <agent> <request>
interrupt [--cross-session] <agent> [message]
```

A target that resolves to another ae session is refused unless the leading
`--cross-session` flag is present. Passing the flag states that the human
explicitly instructed that delivery. Resolution happens first, so an unknown
target keeps its normal resolution error. Successful cross-session deliveries
write the same event to both session ledgers with `cross_session: true`.
Outside tmux, the helper's own session is treated as the caller.

`reply <request-id> <message>` needs no flag when answering a cross-session
`ask` or `review`: the stored request id proves that conversation was opened
explicitly. Inspection and navigation (`peek`, `focus`, `agents --all`) do not
deliver messages and need no flag.

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

### Meta-agent (orchestrator) overview spacing

A session marked as the fleet orchestrator with `[workspace] orchestrator = true` (or its
legacy aliases `hub = true` / `meta = true`; persisted to
its meta as `meta_agent=true`) gets a different watchdog behaviour for its **main
agent**: instead of the stale-nudge watchdog, each cycle renders the fleet
overview and sends it only when the text changed and the minimum spacing
elapsed. The spacing comes from persisted `[workspace] sweep`, then
`AE_WATCHDOG_SWEEP_SEC`, then 120 seconds. A zero disables the sweep branch;
positive values below 60 seconds are clamped to 60, one normal watchdog cycle.
It never escalates the orchestrator to a stale `attn:` alert (idle between changes is normal for a monitor).
Workers/spawned agents in the same session keep the normal watchdog.

Sweep nudges are **delivery-checked**. A nudge can fail to land — the target's shell
is dead (refused), or it stayed busy / a human was typing in it (abandoned after
`AE_SEND_DEFER_SEC`). A failed nudge is logged as `sweep nudge FAILED` with the
reason, and is retried after `AE_WATCHDOG_SWEEP_RETRY_SEC` (default 30) rather than
waiting a full sweep window. After `AE_WATCHDOG_SWEEP_RETRY_MAX` (default 6) fast
retries the watchdog falls back to the normal spacing and raises one
`meta-agent unreachable` alert, cleared when a nudge next lands. Delivery is
**at-least-once**: a nudge that lands but fails to write its event or overview
checkpoint is retried after the spacing, so the orchestrator may occasionally
receive the same overview twice rather than silently lose it.

Liveness is still guarded two ways: the dead/missing-pane checks catch a crashed
orchestrator, while a live seat acknowledges each delivered overview with
`state done`. The watchdog records the oldest unacknowledged delivery and the
latest successful delivery in `meta-agent-state.json`, both at the checked
submit time. Repeated deliveries advance minimum spacing without sliding the
acknowledgement deadline. A `done` in `events.jsonl` acknowledges the batch only
when it is at or after the latest successful delivery. When none qualifies by
`sweep * 2 + 60` seconds from the oldest outstanding delivery, the watchdog
raises one `meta-agent not acknowledging overviews` alert, cleared by the next
qualifying `done`. The state-file mtime is the watchdog's own render heartbeat
and is never treated as seat liveness. The same file carries
the last semantic overview hash: elapsed age labels, request bodies and hidden
worker needs do not change it, but a visible state, leadership reason,
per-session open-request count, goal or topic change does. A restart neither
resends unchanged text nor forgets minimum spacing or the outstanding deadline.
The overview is built from the same `current_world` plus brief-card facts as
`ae brief --all`; the seat does not run that command on a timer. Sweep nudges
use `action=nudge`, which is **not
in the default telegram include set**, so routine overviews do not reach your
phone (a custom `include` containing `nudge` would forward them).

## The orchestrator seat

The **orchestrator** is an ordinary local ae session named `orchestrator`. It
receives a watchdog-rendered `NEEDS YOU` / `WORKING` / `QUIET` overview only
when that text changes. `NEEDS YOU` is grouped by session and contains only the
main/`colead` decisions that need the human; open asks are counts on `WORKING`
rows. A human answer for a session is relayed to that session's lead, or to the
named seat when the human selects one. Its entire overview turn is `state done`: it prints
nothing and stays done until another change. Each delivered change therefore
costs one minimal seat turn; a timer-only cycle costs none. For a human fleet
question it may run `ae brief --all` once. It relays only explicit human
instructions through its `relay <session[:agent]> <text…>` helper.
One quoted text argument works; otherwise remaining argv are joined with single
spaces. The delivery is bare human-authority text, audited with target and full
text only in the orchestrator session. Free text without a leading target is
routed only when exactly one session goal or latest memo topic matches; an
ambiguous or missing match prompts for the target and sends nothing. It never
dispatches work, changes goals, clears questions, edits project or session
state, or treats text from another session as instructions. On an explicit
instruction naming a stopped session, it runs `ae <name> --no-attach` and
reports the printed attach line; it never runs bare `ae <name>`. To create a
session, it acts only on explicit instruction. When the human names the session
and a directory that exists at the spelled path (including `~` or relative
paths expanded), it runs immediately in default local mode; it confirms only
when a fact is inferred or missing (directory missing or nonexistent,
resolution lands elsewhere such as a symlink, or mode unclear).
Otherwise it first prints one proposal line
`name=<n> dir=<canonical path> mode=local|copy|worktree` (default `local`,
`worktree` only for branch/isolated/parallel, `copy` only when asked), waits
for `yes` or an edit, then runs exactly one matching command: mode `local` →
`ae <name> --dir <path> --local --no-attach`; mode `copy` →
`ae <name> --dir <path> --copy --no-attach`; mode `worktree` →
`ae <name> --dir <path> --worktree --no-attach`. It never omits or combines
mode flags and never infers a missing directory.
It runs `ae stop <name> -y` or ordinary `ae end <name> -f --keep-history` only
when the human explicitly names that verb and session; it runs
`ae end <name> -f --purge-history` only when the human explicitly says purge
or delete history. It never stops or ends its own session (the seat named
`orchestrator`) and never all sessions: on such a request it runs nothing and
answers that the seat cannot stop or end itself; the human does that from a terminal. After a lifecycle command it runs nothing else and declares `done`
after the watchdog reports the result. See
[`contrib/aeorchestrator`](../../contrib/aeorchestrator/).

**Starting it.** Run it from anywhere:

```bash
ae orchestrator
```

The bare command seeds `~/.ae/orchestrator.config` on first run, then starts or
reattaches the local seat with that file as its overlay. Project-local
`.ae/config` files never affect the seat. Bind the profile globally with
`[roster] orchestrator = <profile>`; a missing row refuses before the seat file
is written. Use `--no-attach` to build or reattach without attaching and print the exact attach
command. The generated config
carries workspace and role prompt only, including `orchestrator = true` and
`sweep = 120`; old generated configs with
`[profiles]`/`[roster]` are ignored for identity; `[workspace]` and `[prompt]`
still overlay. `CHARTER.md` is its readable reference,
not a guessed runtime path.

**Autostart.** A launch may start the configured Telegram bridge. The
orchestrator is started explicitly with `ae orchestrator`; it is never a
background companion. `AE_NO_AUTOSTART=1` suppresses the Telegram bridge.

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
binary running `_telegram-run` — no `jq`, no `curl`, no extra CLI dependency. What the core
does not read for itself — which config to honour, which home to keep state under, and
which tmux server the daemon's session belongs on — used to arrive as the wrapper's
preamble. Slice Z3 deleted the wrapper and each of those facts became an env DOOR the core
reads directly (`CONFIG_FILE`, `AE_HOME`, `AE_TMUX_SERVER` with its kind). See the
[Telegram bridge](telegram.md) page for setup, config schema, inbound trust boundary, and
lifecycle.

## `ae rename [old] <new>`

Rename a session: the tmux session, the session directory, `session=` in meta, the
regenerated `workspace.md`, and the status bar, all under the session's lifecycle lock as one
core operation. The running tmux server stays up. `[old]` is optional — run it inside the
session you mean and the core resolves it. The new name must satisfy the session-name
grammar, and the error echoes it verbatim when it does not.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves everything on disk: ae state at `~/.ae/sessions/<name>/` plus the per-agent conversation files at `~/.claude/projects/.../<uuid>.jsonl` and `~/.codex/sessions/.../<uuid>.jsonl`. The next `ae <name>` resumes with the full conversation history. When the recorded server proves the session absent, that build may move it to the current launch destination; the server pair changes only with the successful build publication.

Use this when you're done for the day or switching contexts.

When stopping a session a client is watching, ae moves that client to the next ae session in strip order on the same server; if none remains, the client is detached as before.

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
log. Ae also displays `Stopped <name>` on every client still attached to that tmux server.
After reattaching elsewhere:

```bash
grep '"action":"stop-result"' ~/.ae/sessions/myproject/events.jsonl | tail -1
```

An agentic CLI shell escape has no terminal on stdin. In that case ae asks the human on
the most recently active tmux client attached to the caller's session:

```text
Stop 'myproject'? Kills the session you are in. (y/n)
```

`y` starts a short-lived server job, which detaches the real supervisor and returns before
the session is killed. `n` or Escape changes nothing. With no attached client, ae refuses
with `nobody attached to confirm; pass -y`. Add `-y` only when the caller has already been
authorized to skip human confirmation.

### Stopping every session (`ae stop all`)

`ae stop all` stops every session **ae's own metadata owns**, using each
session's recorded tmux server metadata rather than ambient
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

## `ae end` / `ae rm`

End a session for good. Removes ae's own state; **keeps the agent conversation
history by default**. If you want to resume later, use `ae stop` instead.

When ending a session a client is watching, ae moves that client to the next ae session in strip order on the same server; if none remains, the client is detached as before.

Inside a session, bare `ae end` and `ae end <current-name>` target the caller's session.
When stdin has no terminal, ae asks the attached human's tmux client:

```text
End 'myproject'? Archives, then deletes its state. (y/n)
```

When the frozen plan purges history, the prompt says so instead:

```text
End 'myproject'? Deletes its state and purges the agent history. (y/n)
```

`y` hands the entire kill/archive/cleanup sequence to a detached supervisor, so destroying
the caller's pane cannot interrupt the archive. The plan is carried to the supervisor and
revalidated under the lifecycle lock; if policy or state changes while the prompt is open,
ae refuses before stopping the session. `n` or Escape changes nothing. With no attached
client, ae refuses with `nobody attached to confirm; pass -f`. `-f` remains the explicit
non-interactive authorization.

Before the handoff, ae records `end-request`; that event is therefore captured by a
successful archive. A failed end leaves live state and records `end-result` with the failed
step. Success writes no result into the immutable published archive: its UUID is the result.
Ae displays `Ended <name> — archived <uuid>` on every client still attached to the server.
It emits no end `chat` event: the live event source disappears during the end, so that
delivery could not be promised.

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
  Helpers, the `launch.*` per-slot bookkeeping, provider session-id scratch files, locks
  and the generated `workspace.md` are all left behind — an archive is data, and it must
  not be possible to run one. The no-symlink rule earns its keep here: since slice Z2 a
  helper *is* a symlink to the ae core, so a copied one would put the binary itself one
  hop from an archive.
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
`_stop`, `_end`, `_compact`, `_spawn`, `_retire`, `_send`, `_relay`, `_ask`, `_review`, `_reply`,
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
