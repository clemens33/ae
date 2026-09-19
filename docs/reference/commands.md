# Commands

```text
ae                     Attach to the fleet server's most recently used session;
                       inside that server, list the fleet instead
ae <name> [--local|--copy|--worktree] [--dir <path>] [--no-attach]
                       Start or reattach a session. --dir selects its origin;
                       --no-attach prints the exact attach command and exits
ae <name> --solo       lead only, no colead
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
ae brief [name] [--all] [--since <dur>] [--seat <agent>]
                       Card a session: goal, the latest note per memo topic, each agent's
                       declared state, and who is waiting on you. Read-only.
                       --seat prints ONE seat's seed pack instead of the cards
ae quota               Show bounded local quota snapshots for every configured account
ae usage [name…] [--json]
                       Show offline API-equivalent usage for all live sessions, or
                       only the named live sessions
ae board [session…] [--since <ts>] [--json] [--follow] [--lines <n>] [--assistant]
                       The filtered cross-fleet record: genuine human turns from
                       every seat's harness transcript (Claude Code, Codex, Grok,
                       Muse and Antigravity)
                       --assistant adds the model's replies (text only; off by default)
                       --follow keeps printing new rows and coverage changes every 5 s
                       --lines clips each text body to its first <n> lines, with a
                       marker for the dropped remainder (refused with --json)
ae orchestrator        Start or reattach the orchestrator seat: a local session named
                       orchestrator, drawn as a `◆` button after the menu glyph
ae orchestrator --popup
                       Pick a live session in a tmux menu; its lead pane gets the
                       client. Opened above the bottom-left status-bar ≡/= button,
                       before the session list,
                       or +N overflow count. Needs tmux >= 3.4
ae orchestrator --settings
                       Internal status-button route for the bottom-right,
                       exact-client settings menu. Installed bindings supply --client
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Regenerate helper scripts and workspace.md in existing sessions
ae init [--yes] [--lead <profile>] [--colead <profile>|--solo]
        [--orchestrator <profile>|--no-orchestrator] [--palette <p>] [--force]
                       Discover harnesses and propose or write the global config
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
ae compact [name]
                       Compact every fixed seat of a session in place: checkpoint
                       each seat's durable state, then paste its compaction command
ae reboot [-f] [--digest-only] [--keep-history] [name]
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
a launch and a launch takes its single positional as a session name — a bare `ae status` would
otherwise create a session called `status`.

| Word | What now |
|---|---|
| `ae status [name]` | Refuses (exit 2). `ae list` answers the same question from one implementation, and its per-session sub-line already carries the state, goal and attention rollup `status` printed. Inside a session, the `peek` helper shows one agent's recent output |
| `ae hub` | Refuses (exit 2). The orchestrator seat is [`ae orchestrator`](#ae-orchestrator); the fleet picker is [`ae orchestrator --popup`](#ae-orchestrator---popup) |
| `ae transfer <name> <ssh-target>` | Gone, no arm. Cross-machine session sync was ruled cut rather than ported |

Any other `_`-prefixed word nobody serves also fails closed with exit 2, for the same
fall-through reason.

## `ae init`

`ae init` resolves `claude`, `codex`, `grok`, `agy`, `opencode`, and `gemini` through the same
PATH resolver as `ae doctor`. It runs no child process, performs no network or authentication
check, and reports those limits beside each found executable. Shell aliases are not executable
files and are therefore invisible. Its proposal includes one `[clients]` no-op alias per found
tool (`claude = claude`, for example), followed by the unchanged profiles available for those
clients. Add `config_home` only to a second Claude or Codex client; see the
[second-account recipe](../getting-started/config.md#multiple-identities-of-one-cli).

The default roster is derived from the found harnesses:

| Found set | Lead | Colead | Orchestrator |
|---|---|---|---|
| Claude + Codex, with or without others | `fablex` | `astrax` | `gpt56solx` |
| Claude, without Codex | `fablex` | `opusx` | `fablex` |
| Codex, without Claude | `astrax` | `solx` | `gpt56solx` |
| Neither; Grok + agy, with or without others | `grok46` | `agy` | `grok46` |
| Neither; other combinations | first found | second found, or solo | first found |

The fallback order is `grok46`, `agy`, `opencode`, `gemini`. Known xAI + Google choices satisfy
the provider-diversity note; OpenCode and Gemini proposals keep provider status unverified.
With Claude or Codex alone plus a fallback harness, the standing pair remains the same-provider
default and init names the available reviewer profile.

On a terminal, Enter accepts each bracketed default. EOF, Ctrl-C, an invalid answer, or an
overlong answer writes nothing. Without a terminal the proposal is printed but writing requires
`--yes`. `--solo` removes the colead and selects the `lead-solo` layout;
`--no-orchestrator` removes that roster row.

An absent config is created exclusively. When a regular config already exists, init leaves it
byte-identical, creates `<config>.proposed` exclusively, and prints a pure-Rust unified diff.
`--force` first creates `<config>.<epoch>.bak` exclusively, then atomically replaces the config;
a backup or staging failure leaves the active config untouched. Symlinks and non-regular config
paths are refused. Checkout builds honor `CONFIG_FILE`; installed builds use `~/.ae/config`.

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

Suffix a selection with `@<client>` to run that profile on another account of the SAME harness:
`ae myproject --lead fablex@cc-mic`. Both clients must resolve to the same known tool — an
unrecognized binary on either side refuses, and crossing harnesses needs a duplicate profile.
The override is launch-only: `spawn --using` takes a bare profile and rejects `@`. ae records
the bare profile in `profile.<slot>` and the label in a new `client.<slot>` row; a launch
without an override leaves that row absent. A recorded client is write-once, and the two
resume shapes treat it differently. A flagless resume is "resume as recorded": it honors the
recorded label by itself — no `@` needed — and re-takes the store check, so a label whose
definition moved refuses exactly as if it had been spelled. An EXPLICIT bare profile on a
recorded seat refuses instead: it cannot say whether the label stays or goes, so re-pairing
means `<new-profile>@<same-label>`. Any different label refuses whatever its facts. A first
start whose override resolves to a store ae cannot verify refuses instead of launching blind;
a resume under the same blindness proceeds on its retained store and records nothing new.
Removing
the recorded label from `[clients]` strands the session until the label is restored — that
restore, or `ae end`, is the recovery. A store selected through an exported account variable
the launcher cannot see refuses the same way; pin it in the `[clients]` row as
`config_home=<path>` instead.

On a seat's first start, ae records the canonical config home and whether the tool-specific
variable selected it explicitly or remained unset for the default, before the tool execs. A
default-derived store also records the canonical effective `HOME`, because a symlinked default
store does not reveal which home owns the tool's login and state files. A
retained session keeps that home and mode even if its client later changes or disappears; the
resume prints a `config now points ... retained conversation lives in ...` notice when the path
moves, and the recorded identity wins. This keeps the resume probe, Codex id capture, and execution
on one account without changing Claude's default state-file lookup.

Use `--solo` on a first launch to start only the configured main seat, even when
`[workspace] workers` names standing workers. The main-only roster is recorded, so later resumes
stay solo without repeating the flag. `--lead <profile>` may select that lead's profile. A solo
launch refuses `--colead` or `--seat <worker>`, and `--solo` refuses a resume whose recorded roster
already has workers.

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

Tabular view of ae sessions with per-agent health, declared state, current
`observed:<busy|idle|unknown>` harness frame, and a session-level
`attn:<reason>` marker when a session needs attention. `observed` is distinct
from the agent's own declaration: it reports only what the watchdog positively
recognized in the current terminal frame.
Each session's indented detail line ends with three lifecycle ages:
`created` is the first launch, `started` is the latest launch or resume, and
`active` is the newest ae event. Legacy sessions derive creation from the
main-seat start marker and latest start from their per-seat launch times; a
missing clock renders as `-`. Stopped sessions keep the first two clocks.

The marker is a derived rollup — the single most-actionable reason across the
session's agents, by severity:

| Reason | Meaning |
|--------|---------|
| `attn:dead` | an agent's pane vanished (or the watchdog flagged it missing); clears when the process returns |
| `attn:stale` | the watchdog gave up nudging an idle agent (max nudges) |
| `attn:waiting-user` | an agent declared it's waiting on you |
| `attn:blocked` | an agent declared it's blocked on an external dep |
| `attn:limit` | an agent's pane shows the vendor's own usage limit (rank 3, exactly `blocked`'s); clears when the phrase leaves a live pane |
| `attn:throttled` | an agent is being rate-limited upstream |
| `attn:unanswered` | an inter-agent `ask`/`review` went unanswered past the fixed 1800-second (30-minute) threshold |

(`dead`/`stale`/`throttled`/`limit` reuse the watchdog's own alert events;
`waiting-user`/`blocked` are self-declared and require a reason. A `waiting-user`
reason is a self-contained 2–5 sentence decision in 80–600 characters after trimming:
each option gets one clause, the recommendation gives its reason, and the text points
to any long form in `.local/<file>` or a memo topic. Never use pointers such as “see
pane” or “as discussed.” A spawned (`spawned.<n>`) seat cannot declare `waiting-user`
— it exits 2, because the seat's spawner owns the human question: the seat declares
`waiting-agent` and the spawner escalates. `waiting-agent` declares a wait on ANOTHER ae agent: it is quiet
(no marker) while fresh and ESCALATES to exactly `blocked` (attention marker, and
nudging too only when the idle-nudge cadence is enabled) once it is older than
`idle_nudge_secs * OWN_WORK_AGE_CAP` nudge periods — the same multiplier as the
own-work deferral, except that at `idle_nudge_secs = 0` the deferral is vacuous
while the attention ceiling scales from the documented default 300 s (1200 s),
because zero keeps the marker and only suppresses the nudge; its reason names the
agent and what you need from them. `blocked` keeps a concrete EXTERNAL blocker only — a dependency, a service, a human
decision elsewhere, a broken host — and names what blocks, who or what unblocks it, what
you tried, and a long-form path. `unanswered` flags an `ask`/`review`
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
  "schema_version": 2,
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
         "session_id": "e795c9e9", "alive": true, "observed": "idle", "state": "blocked",
         "reason": "blocked"}
      ]
    }
  ]
}
```

`attention` is the session's single most-actionable reason (see the reason
table above); each agent's `reason` is its own contribution. `observed` is
always `busy`, `idle`, or `unknown`; unsupported, incomplete, modal, and
ambiguous frames are `unknown`. `goal_set_epoch`
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
ae brief aedev --seat lead   # ONE seat's seed pack, for a successor on another tool
```

One card per session, plain text, no colour:

```text
aedev · running · attn:waiting-user · ae 2026.9.5 · created 2d ago · started 1h ago · active 14s ago · s1-brief* · ~/projects/clemens33/ae
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
| header | name, liveness, the `attn:` rollup [`ae list`](#ae-list) shows, the session's ae version, its created/started/active ages, its branch (`*` when the work tree has tracked changes) and its work dir |
| `goal:` | the session's [`goal`](helpers.md), in full, or `none` |
| `topics:` | the **latest** record per `memo` topic, newest topic first — see the topic convention below |
| `agents:` | one line per roster agent: its declared state, how long ago it declared, and the reason it gave |
| `needs you:` | explicit `waiting-user`/`blocked` declarations from the session's main agent or named `colead` (a `waiting-agent` past its ceiling is materialized as `blocked`), with each full reason wrapped across as many bounded lines as needed. Worker declarations and unanswered asks/reviews are intra-session traffic and stay out. Nothing here is inferred, so an empty section reads `none recorded` |

### `--seat` — the seed pack

A seat that has to continue on a different tool — its quota died, its harness fell over —
starts from zero. `--seat` prints everything ae already knows about that ONE seat, as plain
text you paste into whatever tool picks it up. It **writes nothing**, sends nothing and
touches no tmux, and it works on a stopped session as well as a running one.

```bash
ae brief aedev --seat lead > /tmp/seed.md
```

It needs exactly one session — named, or the caller's own — and takes neither `--all` (a pack
is one seat) nor `--since` (which drops exactly the old records the pack exists to carry).
Both are usage errors. An unknown seat exits `1` and names the roster.

`--seat` is the one `ae brief` that reads a harness transcript: the `last turns` section
below always renders, so it always asks the [board](../board.md) readers for that ONE seat's
newest turns. A tool with no reader says so in the section (`incomplete: opencode: not
read`) rather than rendering an empty one, because a successor told nothing cannot tell "the
seat said nothing" from "ae did not look". The read stays read-only.

[`ae reseat`](#ae-reseat-session-agent---using-profile) hands this same pack to a successor
automatically, as its first turn — one builder, so what you read before a move and what the
successor is given are the same document.

| Section | What it holds |
|---|---|
| 1 identity | session, agent, slot and its class (`main` / `fixed` / `spawned`), the spawner of a spawned seat, the recorded profile and tool. No role is claimed: a meta records a slot, not a rank |
| 2 goal | the session [`goal`](helpers.md), in full |
| 3 declared state | this seat's last declaration, its full reason and its age |
| 4 memos | the **full body** for `goal`, `decision`, `parking`, for any topic newer than 48 h, and for any topic this seat wrote. Every other topic is a title and an age |
| 5 requests | pending in this seat's inbox, each with the exact `reply` command; pending it sent; and anything closed in the last 24 h, one line each |
| 6 owned spawns | the seats it spawned that still hold one, with their states |
| 7 roster | the session's seats. Monitor panes are not seats and never appear |
| 8 git | work dir, branch, HEAD, dirty, the latest tag and the last five commit subjects |
| 9 first message | a spawned seat's original brief, bounded at 8 KB, and whether the `brief-*.md` file it names still exists |
| last turns | the seat's own newest six turns, oldest first — words only, no thinking and no tool calls, each bounded at 2 KB and the section at 8 KB. Deliberately unnumbered: sections 1-10 are the RECORD ae keeps, this one is the harness transcript, read live |
| 10 successor instructions | fixed text: everything above is a RECORD written by agents, so verify before acting; re-declare state, read the plans the memos name, answer the pending requests, continue at the parking note |

Two properties are deliberate. The pack targets 24 KB and never exceeds 48 KB; when it has to
clip it drops stale titles first, then closed requests, then the oldest memo bodies, and each
clip leaves a line naming what went. The parking note, the pending requests, the identity, the
first message and the closing block are never clipped. And every field the pack takes out of a
record — memo body, state reason, request summary, goal, carried brief, carried turn, and
the names, slots, profiles and paths beside them — is **neutralised** by one owner. A line
that would otherwise arrive wearing ae's own `⟦ae:` provenance marker arrives prefixed with
`| `, so a record cannot impersonate the setup ae itself injects; and every control byte is
replaced by a space, so nothing a terminal would ACT on survives. That second half is not
cosmetic: `ae reseat` PASTES this document into a pane, where an escape sequence, a bell or
a bracketed-paste terminator would be keystrokes rather than text. Its one cost is
flattening: a tab, and the leading indentation of a nested line, arrive as a single space.
A request id is PROVEN instead: the pack prints it into a `reply` command, so a row whose id
ae did not mint is dropped whole and counted (`[dropped: request id not minted by ae] (N)`).

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
immutable `~/.ae/versions/<V>/`, migrates and repoints every placeable session, reports and
skips stopped unplaceable sessions untouched, restarts companion daemons, then atomically moves
`~/.local/bin/ae` to the new core. Existing agent harnesses stay running. The `install` script
beside `ae-core` is only the bootstrap for a machine with no ae yet.

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
still overlay. Existing seats keep their seeded bytes: after a template change,
copy the new creation stanza into the seat file and restart the seat; the file
change alone never reloads the running prompt. On creation the seat proposes
`name=<n> dir=<canonical path> mode=local|copy|worktree profiles=<profile-flags|defaults>`,
runs `ae quota` once for the profiles the creation will use, carries the confirmed
<profile-flags> (`--lead`/`--colead`/`--seat`) verbatim in the one creation command
(omitted only with profiles=defaults), and takes an unconfirmed profile
decision — a known-exhausted choice or unidentifiable applicability, including
a target-local roster it cannot inspect — to `state waiting-user` with at most
a recommended alternative. ae never substitutes a profile for you: only the human's
confirmation changes the choice. Add `--no-attach` to build or reattach without attaching; ae prints the exact
attach command and exits successfully. The seat keeps its fixed launch shape, so `--dir` remains
an ordinary-session flag. The seat is drawn as a three-cell `◆` button
(`o` with icons off) immediately after the status bar's `≡` menu glyph, before
the fleet strip; the glyph is stable and the verdict rides the foreground
colour. The `--popup` form is the picker, next. From any
ae session on the same tmux server, click the `≡` menu glyph (`=` with icons
off) at the bottom-left, before the session list, or its `+N` overflow count, or
press `<prefix> a` (default `C-b a`), to open it.

The final bottom-right status range is settings: exactly one blank, bare `⚙`, and one blank
(` * ` with icons off). The whole three-cell range highlights and accepts clicks. It never renders
a version. Either mouse button opens one menu at that
client's bottom-right corner (`display-menu -x <client_width - menu_columns> -y S`, with a
saturating numeric coordinate from that client's fit snapshot and final menu budget). Its title uses that client's session-owned
watchdog fact only when it is exactly `ae <CalVer>`, rendering
`ae <CalVer> settings`; a missing or malformed fact renders `ae settings`.
Opening settings clears the fleet button's transient selection and highlights only the settings
button. A selected action clears that highlight before it runs. Escape cannot be observed by ae,
so the watchdog expires a leftover highlight at half its interval; with the watchdog disabled it
persists only until another menu opens or a settings action runs. Drawing or dismissing settings
does not otherwise start, stop or rewrite anything.

Above that control, settings shows one live quota entry — unless the invoking
session is quota-unaware (`[workspace] quota = off`), in which case the menu is
the base control menu with no quota row at all, and invoking the quota-dialog
continuation anyway is refused. Choosing the entry clears the
settings highlight and draws a centred dialog listing every quota window of every
configured client scope, read fresh with the invoking session's recorded project
overlay rather than the command's working directory. Rows keep `ae quota`'s stable
scope order. Client labels that resolve to one source share one scope section,
distinct config homes remain distinct, and several Codex rollout owners under one
source share one section. That is why the dialog can show fewer sections than
`ae quota`, whose full table may repeat a scope to preserve each rollout owner's
provenance.

Each scope section is one header row with the client identity, then one indented row
per window. Every window row keeps the derived percentage and its reason (`xN`,
`unlimited`, or `spend-cap`), the window-reset countdown, observation age, and
`fresh`/`stale` verdict. With no usable observation the section says `unknown`,
`unsupported`, `read-error`, or `truncated`; a config failure becomes
`quota: unavailable`. These rows are keyless and cannot trigger an action; only
`Close` dismisses the dialog. `ae quota`
remains the full view with rollout provenance, raw values, credits, paths, hints, and notes.
Any `read-error` or
`truncated` sibling makes the scope section report that incomplete status instead of its
window rows; an `unknown` sibling does not erase an observed same-source window.

Ae first proves the original orchestrator-only menu fits. It draws the quota entry row only when
it fits by its actual rendered width. Otherwise the
existing blank separator becomes one bounded `+Nr +Nc` overflow notice, leaving the original
Start/Resume/Pause row and base height intact. Very small clients therefore see the overflow
notice instead of the entry; a client too small for even that original menu keeps the
original refusal. Display cells are
printable ASCII and visibly elide long scope or bucket labels from the middle.

The menu offers exactly one orchestrator action from a complete raw metadata
census. No recorded role plus no canonical saved/live namesake offers **Start**.
One stopped `meta_agent=true` role offers **Resume** for that exact name and
recorded tmux server, including a renamed seat. The same proven role running on
the invoking server offers **Pause**. Multiple, damaged, incomplete, foreign or
unproven state is shown unavailable. A captured action is re-proved under the
existing target lifecycle lock: stale Start never becomes Resume, and stale
Resume never attaches, re-homes, or starts a replacement. The canonical lock
serializes Start with operations on the canonical name; it does not claim
fleet-wide uniqueness against separately authorized creation under another
name.

Pause asks once, with Cancel first. It stops the role through the existing
detached Stop owner while preserving state, worktree and conversations. Resume
the canonical seat with `ae orchestrator --no-attach`; after a rename, use the
exact recorded name, for example `ae renamed --no-attach`.

## `ae orchestrator --popup`

The fleet picker is drawn by tmux itself at the left edge above its status button.
Its RUNNING rows come from one live `list-sessions` call on the calling server; one
`list-panes -a` call proves which published lead and agent pane hints still
belong to their sessions. One `list-clients` call resolves the explicitly named
client's current session, process and drawable height and width. Numeric
`display-menu -x 0` is the menu's bottom-left client column on tmux 3.4
([source](https://github.com/tmux/tmux/blob/3.4/cmd-display-menu.c#L214-L233)), so the picker
needs no target pane.

STOPPED rows need a second source, because a stopped session has no tmux session
to list: the picker reads ae's durable session inventory once and asks the same
classifier [`ae list`](#ae-list) uses which of those sessions the calling server
proves stopped. The caller's socket spelling and each record's recorded server
are reconciled through tmux's own socket answer first, so a session recorded as
`-L ae` still answers to the socket `$TMUX` names. A row is drawn exactly where
`ae list` would read `stopped`: the exact name absent from a successful listing
of its own server. A session live on another server, a record with no usable
server pointer, a same-named tmux session whose ae ownership cannot be proven,
and a damaged record are all **unlisted** — `ae list --all` may show some of
them, and that gap is deliberate: the picker never guesses a state it did not
prove. `ae list` remains the complete view.

Left- or right-click the menu glyph or overflow count, or press `<prefix> a`
(default `C-b a`), to open it. The picker shows at most 30 running sessions. Its title starts
with `ae session`, then counts running sessions and those whose mark is needs-you or
dead; a stopped count joins the title whenever a stopped row is listed. An invocation
names its tmux client explicitly through the menu and every action, so another
client watching the same pane is untouched; if that client vanishes, the
picker refuses instead of choosing another.

While the picker is open, the first-cell menu glyph uses the palette's selected
background and ink. The marker belongs to the named client's current session,
never the session under a mouse target. Choosing a row clears it before the
switch; opening the picker clears the settings marker, and opening settings clears the picker
marker. Reopening either menu refreshes its own epoch and keeps only that button lit. The watchdog
clears either marker once it is half a cycle old. tmux exposes no menu-close hook, so Escape, `q`,
or an outside click can leave the last button lit until a later watchdog sample: about a minute,
at most 90 seconds with the default 60-second cycle. With the watchdog disabled, another menu or a
selected action provides the next clear.

```text
# opened with prefix a; its binding supplies --client
┌─ ae session — 3 running · 1 stopped · 1 need you — prefix a ──────────┐
│ gamma ✖ dead    fix/menu   restore its lead pane                   (1) │
│   ✖ lead  fable5    dead                                               │
│ beta  ◌ stale   main       port the watchdog                       (2) │
│   ● lead  gpt56sol  working                                            │
│ alpha · idle    picker     ship the S0 picker                      (3) │
│   ✓ lead  gpt56luna done                                               │
│ notes ✖ stopped feat/docs  park the spend notes                    (4) │
└────────────────────────────────────────────────────────────────────────┘
```

Sessions come in **attention rank order**, then tmux creation order, then name.
Every live admitted ae session on the calling server stays eligible, including
the current and orchestrator sessions. Each row carries bounded name, mark,
state word, branch and goal columns. The branch is sanitized in the tmux
reader without changing its raw option, so delimiters and control bytes cannot
split the record. The current session is followed by an indented
`mark name profile state` row for every recorded agent it has; no other
session's roster is ever expanded. A session whose roster stays collapsed
summarizes it as `· N agents, M working`, counting only working-mark seats —
a waiting-agent seat is quiet, not working, and draws its own `◔` mark.

**Stopped sessions follow the running ones**, most recently live first and then
by name, drawn with the `stopped` state word and the same bounded name, branch
and goal columns. A stopped row is a fleet fact, not a verdict: it carries no
rank, never expands a roster, and no agent row ever hangs under it. Choosing it
starts the session — the row re-execs ae as `orchestrator --picker-resume` with
the exact client that opened the menu, which re-proves that client and the
server before resuming the session through the ordinary launch path — and then
switches that client into the resumed session. The other client watching the
same pane is untouched, exactly like a running row's jump. A row clicked after
its client detached or its server was replaced refuses without starting
anything.

Column widths are fitted PER DRAW: name, state word, branch and profile are each
as wide as the widest content among the rows that draw actually shows, with a
four-cell floor and the layout's own cap (18, 9, 14 and 12 cells). A session row
and the agent rows under it share one name column and one state column, so both
kinds of row stay in a single grid. Two short session names therefore give narrow
columns instead of a field of blanks, and one long name is cut at the cap.

A draw never expands more than that one roster, and when even it is too tall
every session collapses to its summary suffix. The working mark is a frozen
static mark: tmux
draws a menu once and does not animate an open menu. Missing, malformed, more
than two of their own published watchdog intervals old, or more than one such
interval ahead of the local clock, agent facts draw `agents: unavailable`,
never a confident zero. Each fact's interval is bounded to 1–3600 seconds. At
most 30 running session rows; a disabled note names how many were left out.

The client snapshot is also the hard draw budget: item rows plus two borders
must fit its height, and row/title cells plus four borders must fit its width.
When the current session's roster does not fit, ae collapses every session to
`N agents, M working`; when not even the session rows fit, it caps them and adds
an honest `+N sessions omitted` row. Stopped rows share both budgets: at most 30
of their own, counted into the same height ladder and the same omission note,
with the running rows keeping their share of a capped draw. Text is clipped by
terminal cells, not UTF-8 bytes. A missing or malformed client size, fewer than 6
rows, or fewer than 8 columns refuses before tmux can silently drop the menu.

Choosing a row first runs `switch-client` against the captured `$<id>`, so a
rename after the menu opened cannot redirect it. When the one pane snapshot
proved `@ae_main_pane` belonged to that session, the row then checks the pane's
session id again at execution and selects its window and pane. If the pane
moved or vanished meanwhile, the guarded tail does not follow it: the client
remains in the chosen session. A missing or unproven lead hint gives the row a
plain session switch.

Session shortcuts are assigned before agent rows are inserted, so the same
session keeps the same key in every expansion mode. Agent rows are keyless and
use arrow plus Enter on tmux 3.4+, or the mouse where tmux supports menu mouse
selection. Their pane jump uses the same build-time membership and
execution-time session-id guard as a session's lead hint; an empty, moved or
vanished agent pane therefore switches only to its captured session.

**Coming back is tmux's own.** `switch-client -l` (prefix + `L`) returns the client to the
session it came from; within one session, `last-pane` (prefix + `;`) is the equivalent. ae
remembers nothing about where you were — a second answer to a question tmux already answers
is a second chance to disagree with it.

**One server only.** `switch-client` cannot cross tmux servers, so the picker
lists only sessions on the calling server — running ones it saw live, stopped
ones whose recorded server tmux proves to be that same server. Sessions that
live on, or are recorded on, another server are absent (a named gap: `ae list
--all` lists them); the picker never claims a state for a server it did not ask.
If the live session listing fails, ae refuses instead of drawing a confident
empty fleet; if only the bare-name listing for the stopped check fails, the
stopped rows are left off rather than guessed.

### Hotkey and mouse

On an ae-owned server, launch and upgrade bind `<prefix> a` (default `C-b a`) and the
capability-specific status mouse actions. Each picker action captures
`#{client_name}` and uses `display-menu -c <name>` because two clients can
watch the same pane. An ambient server keeps all of the user's bindings
untouched.

The distinct `ae-settings` range carries the same client identity. On both
buttons it opens settings rather than the fleet picker or Flip menu.

Every picker and context menu uses `display-menu -O`. On tmux 3.5 and newer,
Down opens menus with `-M`, the trailing release leaves them open, and rows are
mouse-selectable; launch also removes stale Up bindings. tmux 3.4 has no `-M`:
status clicks open menus on release, menus are keyboard-driven, displayed row
keys choose rows, and `q` or Escape closes. Its Down bindings retain session
and window navigation but do nothing on picker and context-menu ranges, so one
click cannot dispatch twice.

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

## Session context menu

Right-click a session range on the status line (`MouseDown3Status` on tmux 3.5+,
`MouseUp3Status` on 3.4) to open that session's context menu, drawn by the
read-only `_session-menu show` path: it reads every source first, then takes
one final clicker proof (server identity plus client pid), and draws against
the live dimensions that proof returned. No action in the menu writes session
state except Stop, which asks first.

The root shows three blocks, then the actions:

- The facts block: `mode:` (`local`, `copy` or `worktree` — the words the
  human asked for, whatever the meta spells), `dir:`, `source:` for
  copy/worktree sessions only, and `branch:` when the meta records one. A
  missing fact prints `unrecorded`, never a guess.
- The declared states: newest per roster actor, at most three, with truthful
  age. States render only when the live session uuid matches the meta's
  `session_id`; anything else is a named gap, never another incarnation.
- Two keyed rows between the states and Flip: `Activity…` (`a`) and `Memos…`
  (`m`), opening read-only dialogs (see below).

`Flip` (`f`) returns to the previous session; `Stop session...` (`s`) starts
the guarded stop chain. On a short client the root degrades in order — facts
first, then the two dialog rows, then all but the first state — down to
today's status-only menu and floor; it never refuses once the clicker is
proven.

The `Activity` dialog lists the newest 10 records a human cares about, newest
first: `state`, `done`, `goal`, `spawn`, `retire`, `relaunch`, `ask`,
`review`, `reply`, `watchdog-start` and `watchdog-stop` (the watchdog rows draw only for non-`ae:` actors, so ae-driven restarts never flood the dialog), each with actor,
kind, clipped text and truthful age. Watchdog
ticks, quota samples, other audits, delivery records, `memo`, `chat`, `focus`,
`cancel`, `spawn-failed` and lifecycle request/result pairs are not activity
and never render. The `Memos` dialog lists the latest record per memo topic
exactly as `ae brief` computes it, newest first, at most 10. A short client
drops each dialog's oldest rows until title, rows, separator and Close fit.

Dialog rows are informational and keyless; `Close` (`c`) dismisses and writes
nothing. There is no Back row — dismiss and right-click again. Like the
settings quota dialog, a row starting with `-` would read as a tmux separator;
actor names cannot start there, and a hostile memo topic starting with one
renders as a divider line.

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

It also prints the REBOOT EVIDENCE: a `boot` row with the host's boot time, and a
`last-live:<name>` row per session saying when that session last did something only a
live session does. Those two numbers are exactly what a resume compares when a recorded
tmux socket has vanished, so a refusal that says "cannot verify whether tmux session
'<name>' is absent" is read here rather than guessed at. A session with no recorded
activity at all warns: nothing can prove it gone.

Three rows the frozen bash `doctor` printed are **dropped rather than reported as
permanently OK**: `flock` and `timeout` are no longer ae's dependencies (the core locks with
its own `flock(2)` and times out in its own code), and there is no portability-shim layer
left to name in a `userland` row.

An upgrade needs no helper refresh: publication migrates and relinks every placeable session,
reports and skips stopped unplaceable sessions untouched, and restarts companion daemons before
moving the public command pointer. Existing agent harnesses retain their loaded process.
`doctor --refresh` is an explicit repair/development mutation; do not run it unscoped while
sessions are running. After `git pull`, run `just install` (checkout mode), or use tagged
`ae upgrade`:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## `ae quota`

Shows each configured account's locally cached subscription-quota windows. Every `[profiles]` entry
is read, and so is every `[clients]` row on its own: an account no profile names is still an account
whose headroom decides whether to launch a seat against it, and its `manual_resets` declaration is
still a declaration. A client one or more profiles already name is one scope, not two — its PROFILES
cell lists them, while a scope no profile names spells that cell `-`. Scopes are joined on PROVEN
identity only — one canonical vendor source is one account, whatever labels reach it. A discovery
that resolved NO source is never joined to another, because failing to resolve is not evidence of
being the same account, and two refusals shown as one would assert what ae cannot observe; they
render as separate rows, each stating its own refusal. A tool with no local quota source, such as
`grok` or `agy`, can never prove one, so each of its profiles is its own row. The same holds for a
retained Codex rollout whose recorded config home did not resolve: two seats that failed for the
same reason are two failures that read alike, not one account, so each keeps its own row and its own
rollout. The settings quota dialog groups by the same rule, so the two surfaces never disagree about
whether something is one account. On a short client the dialog drops calm rows first, burning rows
last, and admits the dropped count in one inert row; only a client narrower than the dialog, or
below one row plus that admission, refuses — and every refusal is told on the invoking client.

A `manual_resets` declaration belongs to its `[clients]` label, so only that label's OWN scope may
spend it, and only when that scope proved a source. A profile that reaches the same label under a
different HOME is a different account and never receives a count declared elsewhere. When the
label's own scope proved nothing, no scope claims the count and every one shows its raw windows,
because a declared reset increases apparent headroom and every ambiguity resolves toward less of it.
Either way the row says what happened. Both the public command and session helper use Codex
conversation ids recorded across the local ae fleet:

```bash
ae quota
~/.ae/sessions/my-feature/quota
```

Rows preserve canonical tool, config home, rollout, vendor bucket, and actual window even when
their display labels wrap. Scopes are keyed by tool and canonical vendor source path, so client
labels whose quota files resolve through the same symlink are grouped together while distinct
Claude caches stay separate even when their conversation stores coincide. A client-selected scope
lists its distinct client labels; a raw profile keeps the shortened home label.

Quota resolves each configured command through the same client expansion and environment rules as
the launcher. Claude Code reads `<effective HOME>/.claude.json` when `CLAUDE_CONFIG_DIR` is absent
and `<CLAUDE_CONFIG_DIR>/.claude.json` when it is explicit. Codex reads at most the final 256 KiB of
the rollout named by each fleet session's ae-recorded harness id below that seat's recorded config
home. Legacy session metadata without a recorded home uses the profile's configured source. Lookup
checks only the UUIDv7 UTC day and its two neighbouring local-clock days. Exact rollout files are
ordered by mtime before their tails are read so the bounded budget reaches recent activity first;
mtime never supplies the displayed observation age. Its owner is labelled `<session>:<seat>`, and
a retained rollout remains attached to its recorded profile and source rather than being relabelled
when that profile's configured client changes.

Resolution knows the operator's `HOME`, but does not inspect arbitrary pane variables. A configured
home that depends on another variable is reported `unknown` with that variable named in the hint.

### Effective headroom: `EFFECTIVE` and `CREDITS`

`USED` is the vendor's own window percentage. It answers how much of one window is gone, which is
not the same question as how much headroom the subscription has. Two facts move that answer, and
ae uses only the ones it was told or was given:

- **Declared manual resets.** No client reports how many manual window resets an account has in
  hand, so ae cannot discover them. Declare them on the client row:
  `codex = codex manual_resets=1`. The count is `0`–`9`; an unusable value is ignored, the rows stay
  usable, and one note under the table says which value was dropped and why. Two client labels that
  resolve to one config home are one account: the SMALLEST explicit count wins and the disagreement
  is noted, because claiming headroom nobody declared would suppress a real advisory.
  **ae never consumes the count.** Nothing decrements it when a reset is actually used, so a stale
  declaration keeps claiming headroom that is already spent: edit the row after each manual reset.
  A used reset is visible in the table but not attributed — measured on Codex, a manual reset starts
  a FRESH window rather than zeroing the current one, so `USED` collapses and `RESETS` jumps forward
  by about a whole window in the same observation, which is also what an ordinary window roll looks
  like. Only a POSITIVE count derives extra capacity; `0` is a supported declaration that derives
  none and simply states that the raw window is the whole story.
- **Reported credits.** Codex rollouts carry `credits` and `spend_control_reached`. `CREDITS` shows
  the balance literal, `unlimited`, `none`, `available` when credits exist in an amount ae cannot
  state exactly, or `spend-cap`, and `-` when the client reports nothing.

Account facts are read FIELD by FIELD, each with the stamp of the record that last usably reported
it. A record moves only the fields it actually asserts, and only when it names its own bucket and
stamps itself: an absent, null or malformed field neither overwrites the held value nor refreshes
its age, so a proven spend cap is lifted only by a record that explicitly reports it false. One
merge applies that rule everywhere — between two records of one read, between two rollouts of one
scope, and between two cycles of the watchdog — so a later reading can never replace an account
wholesale. Every ambiguity resolves toward LESS apparent headroom, because overstating headroom is
what sends work to a client that is already capped. Age alone therefore never lifts a spend cap,
while an unlimited-credit claim — the one fact that ADDS headroom — relieves a window only when it
was reported no earlier than that window's own observation. A claim ae will not use is still shown
in `CREDITS`, with a note under the table saying the claim was ignored; every other rule still
decides the cells, so a declared reset or a cap may well be what `EFFECTIVE` shows.

A held cap is not a durable ledger. It stands until a record explicitly reports it lifted, but it
lives only as long as the evidence ae holds: a scope whose bounded tail no longer carries the record,
and a watchdog that restarts with no carried state, both start again from what the client reports
now. What ae holds is retained evidence, not a current assertion by the vendor.

`EFFECTIVE` is the percentage ae judges by, and it is `-` whenever nothing was declared or reported.
With `n` declared resets the same usage is spread over `1 + n` windows, so the cell reads
`47.5% x1` for a 95% window with one reset in hand. Unlimited credits read `0%`, because the window
does not bind. A reached spend cap reads `100%` and outranks every declared reset: no window reset
frees it. The watchdog advisory and the delegation guidance read this percentage, not `USED`.

At most three Codex rollout groups appear per scope, newest record observation first. A summary
line counts hidden rollouts and reports the oldest known record observation among parsed hidden
rows. Hidden unreadable rollouts are counted and make the summary `read-error`; if discovery or
reads exhaust the invocation budget, the summary instead carries `truncated` and reports rollouts
that could not be read.

`fresh` means the vendor observation is at most 15 minutes old. Older unexpired observations are
`stale`; expired, missing, or clock-skewed observations are `unknown`. Unexpected file kinds,
oversized files, and malformed complete records are `read-error`. An invocation stops with
an explicit `truncated` summary after 4,096 filesystem entries, 16 MiB of reads, or two seconds. All
table lines are at most 182 columns: the sum of the per-column caps plus the two spaces between
each pair of columns. The two derived columns were paid for in that ceiling rather than by wrapping
every scope and status cell.
The displayed age is when that vendor last wrote its own cache or rollout observation, not when ae
opened the menu or ran the command. Claude can therefore honestly remain stale while another
client scope observed more recently is fresh. The cache is refreshed by the CLIENT, and only
when a `/usage` fetch succeeds there — at most once every five minutes, and the client itself
serves it for at most an hour — so an unused config home can stay `unknown` while ae re-reads
its file on every invocation. Codex rollouts move on the seat's own API turns, so an idle seat
ages out the same way. When a Claude scope holds no usable window, the table names the manual
action in its STATUS cell and the dialog's status line names it too
(`run /usage in a claude session`); the hint and the observation age are derived per render and
are never part of the scope identity, and no percentage is shown while the status is `unknown`.
Cached Claude numbers become `unknown` when the file's current and cached account UUIDs disagree;
the UUIDs are neither retained nor displayed. Untrusted cache labels and terminal escape sequences
are reduced to printable table cells before widths or wrapping are calculated.

Grok Build, Antigravity, Muse Code, OpenCode, and Gemini CLI have no verified reusable local subscription
quota source. They render `unsupported` with an operator hint rather than treating token or cost
history as quota. This command makes no network request, reads no credentials, invokes neither
tmux nor a vendor process, and writes no state.

### Advisories

Each session watchdog reuses this bounded local observation every
`[workspace] quota_every_secs` (default 300 seconds; `0` disables). The first
sample establishes a
baseline. Later transitions into `low` (80%), `critical` (95%), or back to `headroom` are pasted to
that session's main seat and, for a lead-pair, its colead. The thresholds classify the `EFFECTIVE`
percentage, so a window with a declared reset in hand or unlimited credits does not raise one, and a
spend-capped scope is critical whatever its window says — the line then names the cap and does not
send the reader to a reset.

A window belongs to the client scope, not to the conversation that observed it: the tracked state is
keyed by canonical source, bucket, qualifier, and window, never by rollout. Several Codex rollouts
under one config home therefore report one fact and produce ONE advisory per transition, the newest
observation winning. No advisory crosses sessions and ae never reroutes work: the recipient decides
which client should receive new spawns. Silent or older-than-60-minute rows clear the in-memory
baseline; recovery starts with a new silent first sample.

## `ae usage`

Shows token usage and API-equivalent reference-price spend for live sessions. With no names it
reports every running session in this ae home; named sessions keep caller order, and a name that
is not live exits 1. The session helper reads only its own session:

```bash
ae usage my-feature --json
~/.ae/sessions/my-feature/usage
```

The table has one row per seat and model, session totals, then a fleet total. JSON carries the
same raw token counters, micro-USD cost, coverage, observation time, retirement and approximation
facts. Supported unreadable, unlocated or truncated sources show `?` and make known token and cost
totals `(partial)`; unsupported harnesses show `n/a`. A truncated event scan says exactly
`retired seats: unread (events scan truncated)` rather than treating missing history as zero.
Failed session-meta or non-truncated event reads are named in the table, exposed as
`meta_scan_failure` / `retired_scan_failure` in JSON, and make totals partial. A missing optional
events file remains distinct and is not a failure.
Retire events without typed conversation identity collapse into one
`<session>  retired: N seats unlocated (legacy retire events)` line, are excluded from totals and
do not make those totals partial: their usage is unknowable. Retired events that do carry typed
identity keep their own rows and make totals partial when that source cannot be read.

A seat moved by [`ae reseat`](#ae-reseat-session-agent---using-profile) reports its SUCCESSOR's
usage only. The predecessor's conversation is kept addressable in the seat's predecessor list,
but `ae usage` reads the current row, so the tokens the seat spent on the tool it left drop out
of the live figure at the moment of the move. They are not lost on disk — the conversation files
are still there, under the predecessor tool's own config home — and they are not counted here.
Capture a figure before a reseat if the number has to include them.

Usage is derived offline from the conversation identity and config home captured when each seat
first started. Claude assistant records include parent and subagent transcripts, deduplicate growing
stream snapshots, skip parent replays in sidechains and aggregate by model. An overlong Claude
record is skipped, but marks the known subtotal truncated and approximate rather than complete.
Codex uses the last
cumulative token event, splits cached and cache-write subsets from ordinary input, and counts
reasoning only once inside output. It prices that total at the last turn-context model seen in the
bounded tail, or the bounded head when the tail names none. A `~` model prefix means the rollout
changed model (or its cumulative total decreased), so the last-model estimate is approximate; a
named model never becomes an unknown cost merely because the rollout switched. Retire events
preserve enough typed identity to retain a worker row after its roster entry is removed. Grok
Build, Antigravity, OpenCode and Gemini have no adapter in this slice.

Prices answer one narrow question: what the same tokens would cost at API list price. A subscription
seat pays nothing extra, and ae neither meters nor bills it. Bundled rates are reference prices for
the base service tier, base context window and five-minute cache writes. Long-context surcharges and
priority or flex tiers are not modelled. Exact model ids and their exact `-YYYYMMDD` releases match;
unknown models keep tokens and show `?` cost. `[prices]` aliases can override exact model ids.

Metadata, event and rollout discovery use per-seat bounds of 4,096 filesystem entries, 16 MiB and
two seconds. Claude transcripts stream line by line under a 512 MiB total cap per seat; a line over
1 MiB is skipped without buffering the rest. Codex reads a 256 KiB head and 256 KiB tail. This is an
explicit, transcript-sized report rather than a cheap status probe: a measured 5 GiB local fleet
took 14.5 seconds wall / 11.6 seconds user per run. The command makes no network request, invokes no
vendor process and writes no state. A model without a cumulative counter in the bounded tail is
unknown and partial, never a priced zero. Malformed required counters are unreadable; when a malformed
final event follows a valid counter, the valid counter remains as an approximate observation. Ended
archives stay out of scope: retained vendor transcripts may remain unless `--purge-history` was used,
but archive metadata carries no harness ids.

## `ae board`

Shows the filtered cross-fleet record: genuine human turns from every seat's
harness transcript, oldest first, with ae plumbing excluded. With no names it
reads every running session; named sessions are read as given, stopped or
running. The board reads Claude Code, Codex, Grok, Muse and Antigravity
transcripts; every other seat renders
an explicit `coverage incomplete: <session:seat> — <reason>` row naming its
phase, never a silent subset.

The first line is always the scope statement: each seat shows its current
conversation plus its recorded predecessors (up to 4, newest first). Turns ae
itself injected (the four `src/provenance.rs` markers, the Codex passive launch
turn) are hidden and counted per seat in one `hidden: <session:seat> — <n>
ae-injected turns` line, or `{"kind":"hidden",…}` in JSON; there is no flag to
show them back.
`--since <ts>` (strict `YYYY-MM-DDTHH:MM:SSZ`) keeps rows
at or after the instant; `--json` prints NDJSON — one `{"kind":"scope"}` line,
then `coverage` lines, then `row` lines, every row carrying `"generation":n`
(0 = current, n = nth predecessor). See [the board](../board.md).

Text bodies render indented two spaces under their `## HH:MM:SS
<session:seat>` header — predecessor rows append ` · prior n` after the actor —
one blank line closing each row, under a `# YYYY-MM-DD UTC` divider that
reprints only when the UTC day moves past the previously printed row's.
`--lines <n>` clips each text body to
its first `<n>` lines and prints one `  … +k lines` marker for the dropped
remainder; a body of at most `<n>` lines prints whole and gets no marker.
`--lines` is text-only: combined with `--json` it is a usage error, because
NDJSON always carries the whole body.

`--assistant` (off by default) adds the model's replies: one row per transcript
record, ` · assistant` after the actor in the text header and
`"role":"assistant"` in JSON, body joined from the record's text parts alone.
Claude Code reads `type=="assistant"` records (`thinking` and `tool_use` parts
never read; `isApiErrorMessage == true` records excluded); Codex reads its
`response_item`/`message`/`role=="assistant"` record and its `output_text`
parts only — never the `reasoning`, call or `event_msg` twins. Grok joins one
turn's `agent_message_chunk` deltas into a single row, Muse reads each
`assistant_message_committed` event whole, and Antigravity seats read each
DONE planner reply's `content` from the seat's own transcript (tool-result
records never render), printing one coverage line only when the store is
absent. Empty bodies drop
silently, an unstamped record counts into the missing-timestamp coverage, and
without the flag the stream is byte-identical to the human-only board.

`--follow` prints that board once, then — every 5 seconds until Ctrl-C — only
what is new. The selection is fixed at start: a session started later is not
picked up (restart the follow). Each poll re-reads the roster and re-locates
every seat's transcript; offsets bind to the file's identity (dev+inode), so an
append streams from the committed offset while a replaced, shrunken or
rewritten file is rescanned from zero behind one `transcript replaced —
rescanned` / `transcript rewritten — rescanned` coverage line, with every row
printed again. Coverage lines print on CHANGE after the first pass. Batches are
sorted internally by `(ts, file, offset)` but never merged across batches — a
late seat's older row prints later. Every `row` carries its durable identity
(`file` = `path#dev:ino` plus `offset`), so a consumer can dedup across
generations if it wants to.

## Session helpers

Every helper in `~/.ae/sessions/<name>/` has two accepted spellings, and they
run the same helper from the same core:

```bash
ae @my-feature send lead "review ready"          # short form
~/.ae/sessions/my-feature/send lead "review ready"   # the link
```

`@` is attached to the session, and the short form takes canonical session names
only (`^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`) — a session retained from before that
grammar, which the commands above still address, stays reachable by its link
alone. A missing or malformed session, a missing helper and an unknown helper
are usage errors (exit 2) that read no state; a session with no directory under
the state root, or one whose entry is a file, a socket or a symlink of any kind,
refuses with exit 1 and is never followed. `ae <helper> …` is not the short
form, and a helper reached by bare name is still refused — see
[helpers.md](helpers.md).

The typed session selects the session a helper acts on, never who is calling
it: identity stays pane-derived, so a writer that needs a caller refuses from a
plain shell either way, and `--cross-session` still applies.

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

`stop` takes back every fact only a live daemon could vouch for — the agent
roster, the fleet strip, the goal, the version and the branch — but the session
stays a row: its attention goes back to the Stale mark a launch seeds, so every
other session's fleet strip and the picker still list it and can still switch to
it. Its own status line says `◌ watchdog off` (`? watchdog off` with
`[workspace] icons = off`) where a running daemon's health segment sits, and a
session launched with `watchdog = false` carries that from the start. `start`
needs nothing after a stop; the daemon publishes over it on its way up.

The watchdog also observes the same local quota caches as `ae quota` on its persisted
`[workspace] quota_every_secs` cadence — unless the session is quota-unaware
(`[workspace] quota = off`, pinned at launch), in which case it books no quota
advisory and renders no quota throttle line. It sends state changes only to its
own leadership seats,
retries a refused paste once on the next quota sweep, and records cancelled retries as
`quota-advisory-dropped`. Entering `low` or worse also sends every seat on that client scope one
advisory `quota-checkpoint` ask — no request, no reply expected — whose cancelled retries are
recorded as `quota-checkpoint-dropped`. A throttle event includes the worst current quota row only when the last
scheduled observation exactly matches that seat's recorded client source and, for Codex, rollout.
It never performs an extra quota read for throttling.

For modeled Claude Code and Codex frames, a positive empty input box starts the
independent `[workspace] idle_nudge_secs` clock (default 300 seconds; `0`
disables). Pane redraws do not reset this clock. The reminder uses the existing
session `send` path and says `you look idle: declare state or continue`.

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
named seat when the human selects one. When idle, its entire overview turn is
`state done`: it prints nothing and stays done until another change. If a human
instruction is already in progress, the overview is only a notification: the
seat finishes the instruction first, then declares `state done`. Each delivered
change therefore costs one minimal seat turn; a timer-only cycle costs none. For a human fleet
question it may run `ae brief --all` once. It relays only explicit human
instructions through its `relay <session[:agent]> <text…>` helper.
When the human asks about one session, it runs `ae brief <session>` and presents
the card verbatim — including the full `needs you:` reasons — then relays the
human's answer to that session's lead.

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

**Pausing it.** `ae stop orchestrator` pauses the seat and keeps its state;
`ae orchestrator` resumes or reattaches it. `ae end orchestrator -f
--keep-history` retires it. The seat never stops or ends itself.

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
grammar, and the error echoes it verbatim when it does not. Renaming a session to its
own name is a validated no-op: it reports success without locking twice, moving
anything, or restarting anything.

A positively stopped session renames too — it stays stopped, starts no agent, monitor,
or provider, and keeps its UUID, conversation files, history, and repository contents.
The managed working copy converges to the fresh-name address: a `git` worktree moves by
`git worktree move` and a full copy moves on its filesystem, both to
`~/.ae/worktrees/<new>`, preserving HEAD, dirty/untracked/ignored contents, and symlinks.
Local mode keeps the caller-owned repository path. A later resume, `end`, `list`,
`doctor`, and archive preview all resolve the new address; teardown containment accepts
the moved path.

A stopped rename is a recoverable transaction, not an atomic one: durable intent, then
the work move, the state-directory move, the coherent meta, the checked assets, and the
durable result. An interrupted rename reports the committed facts and the proved retry —
re-run the same `ae rename <old> <new>` to converge forward; `doctor` reports a pending
transaction with the same retry. The rename refuses before any write when the source is
unknown, the destination is occupied, the work identity is unprovable, a request is
pending, or an explicit config home would lose exact resume at the new path. An
implicit-home session renames with a warning: the later resume re-proves the
conversation with existing provider behavior.

A LIVE session keeps the previous behavior: the tmux session, state directory, meta,
monitors, and manifest are renamed in place, and a git worktree or full-copy directory
keeps its original path. That path is recorded session state, so a later `stop` and
resume returns every agent to the same working directory instead of creating a second
copy under the new session name. Live renames never move managed work.

## `ae reseat <session> <agent> --using <profile> [--stop-unknown]`

Move ONE seat to another profile, in place. Same slot, same pane, same name, same
records — only the tool changes, and the successor is handed ae's own account of the
seat as its first turn.

The pain it answers: a seat whose vendor quota dies used to be lost with its context.
The only way on was a fresh spawn under a new name plus a handover written by hand,
while the seat's slot, pane, ownership and history stayed bound to a tool that could no
longer answer.

```bash
ae reseat my-feature colead --using lunam
```

**It does the full round.** A seat whose tool is still running is stopped where it
stands and then moved: same pane id, same pane stamps, same scrollback, the shell left
idle in the session's recorded working copy. A seat whose tool is already gone takes
exactly the path it always took. Either way the seat must then pass the same dead proof
[`relaunch`](helpers.md) makes — a pane that exists, is not dead, is not busy, carries an
ae slot, and whose recorded tool is provably not running — so both verbs refuse for the
same reasons in the same order.

**The stop is never silent.** ae reads the seat's harness frame before it ends anything,
through the same classifier the watchdog uses, and only a frame positively proven IDLE
may be stopped:

- a frame that says a turn is running is **refused**, and the refusal names
  `interrupt <agent>` as the next step. No flag lifts this;
- a frame ae cannot read is **refused** unless you pass `--stop-unknown`. The flag says
  "stop it anyway"; it does not make an unreadable frame idle;
- the frame is read **twice**, a second apart, because between a tool receiving Enter and
  drawing its spinner the box is briefly empty and one reading would call that idle.

Which frames ae can read is a property of the tool, not of this verb:

| tool | frame ae can prove idle |
|---|---|
| `claude`, `codex` | yes — these are the two grammars the classifier owns |
| `muse` | no in practice: it declares claude's input model but draws its own frame, so it reads *unknown* and fails closed |
| `gemini`, `agy`, `grok`, `opencode`, anything unrecognised | no — unmodelled composer, always *unknown* |

Even for claude, IDLE is proven from the pane's recent output, so the row above the input
box has to be claude's own `done` summary: a seat that has not finished a turn since it
started reads *unknown* and needs the flag. When in doubt, quit the agent yourself first —
a seat that is already gone never reaches any of this.

The stop happens under the session's lifecycle lock and under the pane's own send-lock, so
no delivery can land a turn between the frame ae read and the tool it ends; a delivery
already in flight is a refusal, not a wait. It happens **after** every refusal the records
can answer, so a typo never kills a seat. Once the tool is gone ae waits up to 10 seconds
for the pane to come back to an idle shell, and refuses by name if it does not — with the
meta untouched, so the seat still records its old profile and `relaunch` brings it back.

The seed the successor receives is exactly the pack
[`ae brief --seat`](#--seat--the-seed-pack) prints, delivered as ae's own setup turn
(`⟦ae:ctx⟧`). It is also kept on disk at `~/.ae/sessions/<session>/seed.<agent>.md`, so a
turn that did not land can be re-sent by hand — the refusal that says so prints the
command.

**What moves.** The seat's profile and recorded binary; a fresh conversation (a new UUID
where the tool takes one at launch, `pending` where its id can only be captured after);
a new launch token and a new capture floor. The predecessor's conversation is kept
addressable in the seat's predecessor list, tagged with the tool that owns it — the
successor's reader looks in a different store entirely.

**What is removed**, as deliberately as what is written: the config home and its implicit
base, which belong to the tool that is leaving; the launch time, which would date a launch
that has not happened; the observed model and its pin, which would read as drift the
moment the new tool answers; and any `profile@client` override a launch recorded, because
it pinned a profile this seat no longer runs. `--using` therefore takes a bare profile.

**Who may run it.** A pane ae stamped is a seat, and a seat may reseat only inside its own
session. A plain shell carries no stamp and may reseat any session — that caller is the
point of the verb, because the moment a lead's own quota dies no agent of that session can
run anything. A seat cannot reseat **itself**: the tool running the command is the one that
would be replaced under it.

**Refusals**, in the order they are answered. Everything durable is answered from the
session's records, so a stopped session diagnoses a typo exactly as a running one does:
an argv that is not `<session> <agent> --using <profile>`; a session ae cannot read; a
caller in another session; a seat the roster does not name (the refusal lists the roster);
a profile the seat already runs (`relaunch` is the verb for that); a profile `[profiles]`
does not define or cannot lex; the caller's own pane; a pane whose stamp disagrees with the
roster; then, only for a seat whose tool is still running, the working copy, a caller
running *underneath* that tool, a delivery holding the pane, a busy frame and an
unreadable frame; then the dead proof's own ladder.

**If it stops half way.** Nothing before the stop needs undoing, and every window is
recoverable by hand. Before the meta is written the seat still records its old profile and
has no start marker, so `relaunch` brings it back on the tool it had — whether the stop
had already happened or not. After the meta is written the pane sits at its shell, which
is exactly what `relaunch` finishes — the refusal says so by name.

Every attempt that reaches the pane is recorded as a `reseat` event, naming its caller (a
seat by its own ref, a plain shell as the human), both profiles and the conversation being
left behind. A stop gets its own record, written once the pane is proven back at its shell
and never before — it names the binary that was ended, because the meta keeps only the
current one. The seat's history is
the only place a later reader can see that its tool changed, and the meta keeps only the
current profile — so the pairing of a predecessor conversation with the profile it ran under
lives on that event and nowhere else.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves
everything on disk: ae state at `~/.ae/sessions/<name>/` plus each agent's conversation files in
its recorded default or client-specific config home. The next `ae <name>` resumes with the full
conversation history. When the recorded server proves the session absent, that build may move it
to the current launch destination; the server pair changes only with the successful build
publication.

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
   `agent_bin.<slot>` and the canonical `config_home.<slot>` recorded at first start;
   Gemini and OpenCode files are always left in place.

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

Compact every fixed seat of a session in place, in roster order, without stopping
anything. `[name]` defaults to the current session when run inside one, else is
required. One nonblocking run lock is held for the whole run; a second invocation
refuses immediately. There is no confirmation prompt: each seat's checkpoint
round-trip is the consent gate.

Per seat: the R7 gate first (spawned seats are skipped untouched, as are seats
whose tool exposes no compaction command or whose input box is unmodelled —
those are named for hand compaction), then a fresh durable checkpoint opened as
`ae:seats:<session-uuid>`: the seat must reply AND write a memo carrying the
request id, both bound to its live incarnation. Only when both facts land is the
seat's compaction command pasted through the guarded deliver operation, which
re-proves identity under the lifecycle lock first. Every outcome advances; the
report carries one line per seat plus the hand-compaction line.

`dispatched` means attempted: the command was pasted and Enter was sent; it is
never proof of submission. `not dispatched (staged text)` means the text may sit
unsubmitted in the seat's composer — clear it before any send. Any destructive
flag (`-f`, `--force`, `--keep-history`, `--digest-only`, `--exec-plan`) on
`ae compact` is refused with a pointer to the verb that owns it (`ae reboot`).

## `ae reboot [name]`

The three commands above, composed into the one move they are usually used for: archive
what this session knows, end it, and start a fresh session under the **same name** that
continues from that archive.

```bash
ae reboot my-feature                 # ask the main agent for a handover first
ae reboot --digest-only my-feature   # skip the ask; the digest is the handover
```

It exists because agents run out of context. The alternative is doing it by hand — end,
copy the uuid out of the output, relaunch with `--from` — which is three commands where
the second one is a transcription and the whole thing is unrecoverable if you fumble it
after the session is already gone.

**v1 is local mode only.** A `git` or `full` session refuses, and the reason is not
caution — it is that reboot would *lie*. The fresh session's workspace is built from the
canonical origin's HEAD, which normally lags the session's own branch, so a rebooted
managed session would report success and hand you back a workspace missing the code it
just archived. Ending it and starting the next one yourself keeps that decision where it
belongs. Managed-mode continuity is tracked separately.

### What it does, in order

1. **Refuses if you are inside the target.** reboot ends the session your terminal is
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
- A session with **spawned agents**. reboot never retires someone else's worker — retire
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
answered `n` all write nothing to it. When the reboot does happen, stdout is exactly four
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
only route back. Anything printed after the contract belongs to the fresh session: reboot
`exec`s into the launch, so from there on you are reading the child.

Piping reboot is supported, including to a consumer that exits early. A reporting failure
never suppresses the relaunch.

Because of that `exec`, reboot's exit status is the launch's: in a terminal it attaches
you to the new session and exits when you detach. With no terminal to attach to, the
launch reports failure the same way a plain `ae <name>` does — the archive and the fresh
session are already there, and the `Recovery:` line names how to reach it.

`ae reboot` distinguishes **declining** from **not being asked**. A typed `n` is an
answer: it prints `Aborted.` and exits 0. End-of-input is not an answer — with no stdin
(a script, cron, `< /dev/null`) reboot reports that it could not obtain confirmation and
exits **non-zero**, because stdout is empty in both cases and the exit status is a
caller's only way to tell "the operator said no" from "the question never reached anyone".
Pass `-f` if you mean to proceed without being asked.

The **Recovery** line is printed *before* the relaunch is invoked, not from a failure
handler. Past that line the archive is published and the source session is gone, and the
process may `exec` into the launch and never return: a recovery command emitted from an
error path is one that does not exist at the moment it is needed. If the relaunch fails,
the line is already on your screen.

`ae reboot` never deletes an archive. Not the one it just published, not an older one —
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
