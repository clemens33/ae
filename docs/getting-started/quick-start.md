# Quick start

## Start a session

```bash
cd ~/projects/my-app
ae
```

First run creates `~/.ae/config` with sensible defaults and launches your main agent in tmux. The session is named after the current directory.

Detach any time with `Ctrl+b d`. Agents keep running.

## Reattach

```bash
ae                # same directory, reattaches the default session
ae my-feature     # named session
```

Helpers and `workspace.md` regenerate from the currently-installed ae on every start, so upgrades propagate for free.

## Ask agents to collaborate

Just talk to your main agent. It already knows how to spawn others and coordinate. Examples that work as-is:

- *"Get a second agent to review the changes in `src/`."*
- *"Spin up a pair programmer to help refactor auth."*
- *"Ask codex to verify my test plan."*

Agents pick descriptive names, show up in own tmux windows, and talk to each other through generated shell helpers — no manual wiring.

## Check on agents without attaching

```bash
ae status my-feature    # recent output from each agent
ae list                 # all sessions with per-agent health
```

## Finish up

```bash
ae end my-feature       # commit + push to ae/my-feature branch, archive, then clean up
ae rm my-experiment     # same as ae end
```

Both forms leave the working directory clean — session state was always in `~/.ae/sessions/`, not in your repo.

Before it removes that state, `ae end` **archives the session's memory** — goal, memo,
event log, request payloads — to `~/.ae/archive/<session-uuid>/`, and prints the id and
path. The archive is mandatory: if it cannot be written, the end fails and nothing is
deleted. See what it would contain first, without ending anything:

```bash
ae archive preview my-feature      # stdout is the digest; writes nothing
```

Pick it up later in a fresh session — the new lead is told to read the digest before it
starts:

```bash
ae my-feature-2 --from <archive-uuid>
```

When the agent is out of context but the *work* isn't, do both in one move — archive, end,
and relaunch the same name continuing from that archive:

```bash
ae compact my-feature              # asks the main agent for a handover first
ae compact --digest-only my-feature  # skip the ask; the digest is the handover
```

It runs from *outside* the session (it ends the one you would be sitting in), local mode
only for now, and prints a `Recovery:` line before the relaunch so you can start the fresh
session by hand if anything goes wrong after the archive is published.

`ae end --purge-history` is the opposite intent: no archive is written, and any existing
one for that session is deleted along with the agent conversation files.

## Watch the event stream

Every ae session has a hidden `ae-monitor` tmux window with an `_events` pane streaming `events.jsonl`:

- `Ctrl+b w` → pick `ae-monitor`
- `~/.ae/sessions/<name>/peek _events 80` → snapshot view from any pane

The optional [watchdog](../internals/watchdog.md) shares that window — when enabled it adds a `_watchdog` pane with per-cycle decisions.

## Multi-agent at start

If you want more than one agent up immediately, list workers in config:

```toml title="~/.ae/config"
[workspace]
main = claude:lead
workers = codex:reviewer, opencode:tester
```

Or just tell your main agent to spawn them once you're attached. Either way, every agent gets the session's workspace context injected into its system prompt.
