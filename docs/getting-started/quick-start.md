# Quick start

## Start a session

```bash
cd ~/projects/my-app
ae init
ae my-app
```

`ae init` discovers supported harness executables and proposes a roster before the first session.
If you skip it, the first launch still creates the broad default config. Session names are
explicit, so the same directory can host distinct sessions.

Pick another configured profile for a standing seat on this launch:

```bash
ae mdk-rust --lead solx --colead astrax
```

`solx` and `astrax` are ordinary rows you define under `[profiles]`. The same general form is
`--seat <agent>=<profile>` and may be repeated. The selection persists when you stop and resume
the session.

Detach any time with `Ctrl+b d`. Agents keep running.

## Reattach

```bash
ae                # attach to the ae server's most recent session
ae my-app         # reattach this named session
ae my-feature     # start or reattach another named session
```

Helpers and `workspace.md` regenerate from the currently-installed ae on every start, so upgrades propagate for free.

## Ask agents to collaborate

Just talk to your main agent. It already knows how to spawn others and coordinate. Examples that work as-is:

- *"Get a second agent to review the changes in `src/`."*
- *"Spin up a pair programmer to help refactor auth."*
- *"Ask codex to verify my test plan."*

Agents pick descriptive names, show up in own tmux windows, and talk to each other through generated shell helpers — no manual wiring.

Under the hood, a complete first round trip looks like this:

### Human terminal

You need installed `ae`, tmux, git, and at least one logged-in agent CLI on `PATH`. Run `ae init` and accept or edit its proposed `[profiles]` and `[roster]`; those profile names are local configuration, not universal names. `ae init` discovers executables but does not authenticate or check model access.

```bash
cd ~/projects/my-app
ae init
ae doctor
ae my-app
```

`ae my-app` starts or reattaches the named session. Detach with `Ctrl+b d`; run it again later to return. The default launch mode is local; a `--worktree` launch additionally needs a Git repository.

### Lead and worker panes

These panes run AI TUIs, not shell prompts. Ask the lead agent to execute the commands below; the worker executes its generated reply footer. You need only run the Human terminal block. Seeing the result arrive in the lead session is proof of life.

Ask the lead agent to execute a harmless read-only spawn. Replace the quoted `PROFILE` placeholder with a bare profile listed in this session's `workspace.md` before pasting:

```bash
~/.ae/sessions/my-app/spawn fact-check --using "PROFILE" -- \
  'Read README.md; report its first heading; edit nothing.'
```

`spawn --using` rejects `profile@client`, and `spawn` has no `--dir`: it inherits the session's recorded `work_dir`. Choose directory and mode when starting the session (`ae my-app --dir ... [--copy|--worktree]`); omit both flags for local mode.

Use the full helper paths shown here, or the short form `ae @my-app <helper> ...`. A bare helper name, or `ae <helper> ...`, has no session and refuses with exit 2.

Ask the lead agent to execute this tracked request:

```bash
~/.ae/sessions/my-app/ask fact-check "What heading did you find? Reply in one line."
```

The worker receives a generated request id and exact reply footer. The live footer already contains the real request id: copy it verbatim and replace only its message text. The illustrative form below uses `REQUEST_ID` for that already-filled id; replace `REPLACE_WITH_OBSERVED_HEADING` with the exact heading observed, never a canned heading:

```bash
~/.ae/sessions/my-app/reply --as "fact-check" "REQUEST_ID" \
  'Observed first heading: REPLACE_WITH_OBSERVED_HEADING'
```

The example uses a single-quoted message body so observed text stays one argument. If it contains a single quote, have the agent serialize or escape the helper argument rather than hand-edit shell quotes.

`reply` validates the request id against the replying slot and session, then routes the answer to the asker. `--as` is advisory for the name, is recorded as the replier in the durable reply event, and is the identity fallback when pane detection fails; it cannot override slot or session checks.

The worker declares its pane finished:

```bash
~/.ae/sessions/my-app/state done "Read-only check complete; no files changed."
```

After the lead accepts the answer, ask the lead agent to preserve the accepted observed result before cleanup:

```bash
~/.ae/sessions/my-app/memo add --topic fact-check \
  'Accepted observed result: REPLACE_WITH_OBSERVED_HEADING; no files changed.'
```

Replace the memo placeholder with the same observed heading before running it; use the same safe quoting rule. Then ask the lead agent to retire the worker only after acceptance and memo preservation:

```bash
~/.ae/sessions/my-app/retire fact-check
```

Retiring removes the spawned pane and seat metadata. Workers do not self-retire; the lead owns cleanup. Retiring also closes requests involving that worker, so preserve the accepted result first. In this walkthrough's local mode, `ae end my-app` archives memory and removes ae state but does not commit, push, or own a worktree. Commit/push and managed copy/worktree cleanup belong to `--copy` and `--worktree` sessions. `ae stop my-app` preserves state for resume.

## Check on agents without attaching

```bash
ae list                 # running sessions with per-agent health
ae list --needs-attn    # only the ones needing you
ae next                 # name the top session needing attention
```

Inside a session, `peek <agent>` shows one agent's recent output.

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
ae reboot my-feature              # asks the main agent for a handover first
ae reboot --digest-only my-feature  # skip the ask; the digest is the handover
```

(`ae compact` is the coming in-place seat compaction — not yet available; the
destructive handover above is `ae reboot`.)

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

Or just tell your main agent to spawn them once you're attached. Either way, every agent gets the session's workspace context through its harness's supported launch channel.
