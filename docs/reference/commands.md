# Commands

```text
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list [--all|--stopped|--needs-attn]
                       List sessions (running by default; --all adds stopped
                       history, --needs-attn only those needing attention)
ae status [name]       Show agent output without attaching
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
ae steward [--attach|--init]
                       Ensure the detached steward (meta-agent) is running; --attach
                       switches to it (--init scaffolds config + charter)
ae stop [name]         Pause session, keep ae + agent conversation state for resume
ae transfer <name> <ssh-target> [--pull]  Move a stopped session (incl. Claude/Codex conversation files) to/from another machine
ae archive preview [name]
                       Print the digest an end would archive. Read-only: writes nothing,
                       emits no event, does not stop the session
ae [name] --from <archive-uuid>
                       Start a NEW session that explicitly continues an archived one
ae end|rm [-f] [--purge-history|--keep-history] [name]
                       End session: commit, push to ae/<name>, ARCHIVE the session's memory
                       to ~/.ae/archive/<session-uuid>/, then remove ae state. KEEPS the
                       per-session claude/codex conversation files by default (token history);
                       --purge-history deletes them AND writes no archive.
ae version             Show version
ae help                Show short help
```

When run inside an ae session, `stop`, `end`, `status`, `watchdog`, and `doctor --refresh` detect the current session automatically.

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
| `attn:unanswered` | an inter-agent `ask`/`review` went unanswered past the threshold (`AE_ATTN_REQUEST_SECS`, default 30 min) |

(`dead`/`stale`/`throttled` reuse the watchdog's own alert events;
`waiting-user`/`blocked` are self-declared; `unanswered` flags an `ask`/`review`
whose target never replied within `AE_ATTN_REQUEST_SECS` (default 30 min) — the
lowest-severity reason.)

By default it shows **running sessions only** — stopped sessions are usually the
bulk of the list and just noise for monitoring. Flags:

| Flag | Shows |
|------|-------|
| *(none)* / `--running` | running sessions only |
| `--all` | running sessions, then stopped ones |
| `--stopped` | stopped sessions only |
| `--needs-attn` | only running sessions with an `attn:` reason; aliases: `--needs-me`, `--needs`, `--attn` |
| `--active` | only running sessions with recent activity (an ae event within ~5 min; `AE_LIST_ACTIVE_SECS` to tune); alias: `--busy` |
| `--json` | machine-readable digest (honours the filters above) |

For a live dashboard, wrap it with `watch`:

```bash
watch -n 10 'ae list'            # live view of running sessions
watch -n 10 'ae list --needs-attn' # only what needs your attention
```

### `--json` digest

`ae list --json` emits a single JSON object — a snapshot for a monitoring
script or agent. Pure bash output; no `jq` required to produce it. The filters
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
(e.g. the steward) the session's context without any manual bookkeeping.
`schema_version` lets consumers gate on shape. `attention_rank` is the numeric
severity (`dead` 6 → `unanswered` 1); richer per-agent timing fields are a
planned addition.

## `ae status [name]`

Prints the last ~80 lines from each agent's pane without attaching. Useful for a quick "what is everyone doing" snapshot. Marks each agent's binary name and pane id.

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

Pre-flight + post-upgrade self-test. Walks a fixed checklist of `OK / WARN / FAIL` items: bash/tmux/git presence, config file, agent executables, sessions directory, and so on. Returns non-zero if anything failed.

With `--refresh`, also regenerates every session helper from the currently-installed ae binary. Run after `git pull`:

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

The [watchdog](../internals/watchdog.md) is on by default — only an explicit `false` / `no` / `off` / `0` in config or session meta keeps it off. `watchdog start` is idempotent; running it again just confirms the meta flag.

### Meta-agent (steward) sweep cadence

A session marked as the fleet steward with `[workspace] steward = true` (or its
legacy aliases `hub = true` / `meta = true`; persisted to
its meta as `meta_agent=true`) gets a different watchdog behaviour for its **main
agent**: instead of the stale-nudge watchdog, the watchdog sends a *"run your sweep
now"* nudge every `AE_WATCHDOG_SWEEP_SEC` seconds (default 300) and never escalates
the steward to a stale `attn:` alert (idle between sweeps is normal for a monitor).
Workers/spawned agents in the same session keep the normal watchdog.

Sweep nudges are **delivery-checked**. A nudge can fail to land — the target's shell
is dead (refused), or it stayed busy / a human was typing in it (abandoned after
`AE_SEND_DEFER_SEC`). A failed nudge is logged as `sweep nudge FAILED` with the
reason, and is retried after `AE_WATCHDOG_SWEEP_RETRY_SEC` (default 30) rather than
waiting a full sweep window. After `AE_WATCHDOG_SWEEP_RETRY_MAX` (default 6) fast
retries the watchdog falls back to the normal cadence and raises one
`meta-agent unreachable` alert, cleared when a nudge next lands. Delivery is
**at-least-once**: a nudge that lands but fails to write its event is retried, so the
steward may occasionally sweep twice — a redundant sweep is cheap, a silently dropped
one is not.

Liveness is still guarded two ways: the dead/missing-pane checks catch a crashed
steward, and a **heartbeat** check catches a *live-but-not-sweeping* steward (model
stall, upstream throttle, wedge) — the steward's sweep helper rewrites
`~/.ae/sessions/<steward>/meta-agent-state.json` on each real sweep, and if that mtime
stops advancing past ~`2×AE_WATCHDOG_SWEEP_SEC` the watchdog raises one alert (cleared on
recovery). This is the file [`contrib/aemonitor`](../../contrib/aemonitor/) writes
by default; if you override its `--state` path for the steward, point it at this same
file or the watchdog heartbeat will false-alarm. The sweep nudges use `action=nudge`,
which is **not in the default telegram include set**, so routine sweeps don't
reach your phone (a custom `include` containing `nudge` would forward them).

## `ae steward`

The **steward** — your fleet's chief of staff: a single ae session that monitors
all your *other* ae sessions and is your one point of contact to them (it relays
your instructions to the other sessions and reports what needs you, via the
Telegram `say` channel). Once you set an objective (`objective: …` over Telegram) it also holds it, parks
your ideas, and answers `what next` — and may proactively nudge you when you drift,
but only through hard gates (concrete signal held two sweeps, a rate budget, quiet
hours, suggest-only; ignore a couple and it self-mutes for the day). See
[`contrib/aesteward`](../../contrib/aesteward/). It
is a monitor + relay + focus aide: per its charter it never ends/stops/edits
another session on its own, and only suggests — it dispatches nothing without
your say-so.

```text
ae steward          Ensure the detached `steward` session is running
ae steward --attach Switch/attach to the `steward` session
ae steward --init   Scaffold ~/.ae/steward/{steward.config,CHARTER.md} (never overwrites)
ae steward --help   Usage
```

`ae steward` launches the `steward` session with **full config isolation**: it
uses `~/.ae/steward/steward.config` as the config and neutralizes any
project-local `./.ae/config`, so the global config's `workers` never leak into
the single-agent steward regardless of the directory you run it from. The config
dir defaults to `${AE_HOME:-~/.ae}/steward` and is overridable with
`AE_STEWARD_DIR` (so an isolated `AE_HOME` run keeps its steward state out of
your live `~/.ae`).
Unlike normal `ae <name>` session starts, bare `ae steward` does **not** attach
or switch the current tmux client; use `ae steward --attach` when you want to
inspect the steward pane directly.

First time: run `ae steward --init` to scaffold the config + charter from
[`contrib/aesteward`](../../contrib/aesteward/) (placeholders for the charter and
[`aemonitor`](../../contrib/aemonitor/) paths are substituted), edit them to
taste, then `ae steward`. The charter wires the deterministic sweep to
`aemonitor`, defines the objective-armed focus aide, and tells the agent its only channel to you is
`say`.

To talk to the steward from your phone, run the [Telegram bridge](telegram.md):
plain messages route to the running steward automatically (no `/use` setup), and
`/use <session> <agent>` redirects to another session when you want (`/use clear`
returns to the steward) — see
[Steward-centric routing](telegram.md#steward-centric-routing-talk-to-the-meta-agent-not-ten-sessions).

**Deprecated alias + legacy scaffolds:** `ae hub` still works and maps to the
same launcher. A pre-rename `~/.ae/meta-hub/hub.config` scaffold (from
`ae hub --init`) is still honoured — it keeps its `hub` session name so its baked
charter paths and resume state stay consistent (`AE_HUB_DIR` is honoured too).
Migrate with `ae end hub && ae steward --init && ae steward`.

`steward` is a reserved subcommand (as is `hub`). If you ever need a normal
session literally named `steward`, `ae --local steward` reaches the generic start
path (the first argument is then no longer `steward`).

## `ae telegram`

```bash
ae telegram setup       # interactive: writes [telegram] config + token file
ae telegram start       # spawn daemon now, persist enabled=true
ae telegram stop        # kill daemon, persist enabled=false
ae telegram status      # report intent + runtime + deps + token validation
```

Machine-global daemon that bridges every ae session on this host to one Telegram chat. Single instance per machine (lock-guarded). Outbound forwards filtered events to chat. Inbound (when `allowed_user_ids` is set) offers three ways to reach an agent: **reply** to a forwarded event (routes to that agent), the compact **`@session:agent <msg>`** prefix, and a sticky **`/use <session> <agent>`** default for plain messages — plus the explicit `/list` and `/session <name|id-prefix> send|ask <agent> <msg>`. All paths share the same session/agent revalidation. Inbound is from the configured private chat only — auth requires matching `from.id` + `chat.id` + a private chat.

`jq` + `curl` are feature-only dependencies; ae's core commands work without them. See the [Telegram bridge](telegram.md) page for setup, config schema, inbound trust boundary, and lifecycle.

## `ae rename old-name new-name`

Rename a session. Renames the tmux session, moves the session directory, updates `session=` in meta, and regenerates `workspace.md` to reflect the new name. Running tmux server stays up.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves everything on disk: ae state at `~/.ae/sessions/<name>/` plus the per-agent conversation files at `~/.claude/projects/.../<uuid>.jsonl` and `~/.codex/sessions/.../<uuid>.jsonl`. The next `ae <name>` resumes with the full conversation history.

Use this when you're done for the day, switching contexts, or moving to another machine via `ae transfer`.

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

`ae stop all` stops every session **ae's own metadata owns** — not whatever happens to be
running on the ambient tmux server, which `AE_TMUX_SERVER` can redirect.

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
  in `agent.<slot>` it carries the *provider conversation UUID*, the one field that could
  re-open somebody's real transcript. The archive records `alias:name` and drops the rest.
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

## Hidden subcommands

The following are internal helpers ae invokes itself, prefixed with `_`. Don't call them directly:

- `_spawn`, `_retire` — pane lifecycle (called via `spawn` / `retire` session helpers).
- `_recover-pending` — re-attempt post-launch session ID capture (called by the watchdog).
- `_register-sid` — Codex first-task to self-register its session UUID (injected via `developer_instructions`).
- `_stop-supervisor <name>`, `_stop-fleet-supervisor <op-id> <name> <session-id>…` — the
  detached workers behind `ae stop` (self) and `ae stop all`. They exist so the kill runs in a
  process the kill cannot take down with it. The fleet worker is handed its targets explicitly
  and looks for no others, which is why the list is an argument rather than something it works
  out itself; each target is named as a *pair*, so the worker stops the session that was
  confirmed rather than whatever holds the name by the time it gets there.

They're listed only for transparency — your interface is the public commands above.
