# Troubleshooting

## `tmux` not found

Install tmux and rerun `ae doctor`. ae fails fast on startup if tmux is missing.

## Agent CLI not found

```bash
ae doctor
```

Look at the `agent:<alias>` lines. Each one verifies the agent's executable is on `PATH`. Fix the command in `~/.ae/config` or add the CLI to `PATH`.

## `ae` installed but command not found

Make sure `~/.local/bin` is on `PATH`. `ae doctor` warns explicitly when it isn't.

## Session won't resume cleanly

```bash
ae stop <name>
ae <name>
```

Resume and session capture for external agent CLIs are best-effort and depend on upstream tool storage formats, which can change. All five supported tools get exact-session resume once their session id is captured; if capture failed, ae falls back gracefully (Claude `--continue`, Codex fresh-start with preserved flags, Gemini `--resume latest`, Grok `--continue`, OpenCode `--continue`).

## Helpers feel out of date after upgrading ae

Upgrading needs no refresh: stop/resume moves a session to the installed
generation, and a running watchdog keeps its loaded body until restarted.
`doctor --refresh` is an explicit repair/development mutation; avoid an
unscoped refresh while sessions run. It calls the same `sync_session_assets`
path used at session start, regenerates helpers and `workspace.md`, and runs
the orphan sweep.

## Session feels stuck

```bash
~/.ae/sessions/<name>/interrupt <agent>            # soft cancel
~/.ae/sessions/<name>/interrupt <agent> "do X"     # cancel + redirect
ae end -f <name>                                    # nuclear option
```

## `send` reports REFUSED, ABANDONED, or UNCONFIRMED

`send` (and `ask` / `review` / `reply` / `interrupt`) report loudly rather than dropping a message. The stderr line names the guard that fired:

- **`send to <target> REFUSED — target pane is a shell, not a running agent`** — the target agent has exited and its pane fell back to a shell. Nothing was pasted (a stray Enter would run your message as a shell command). Re-launch the agent, then re-send.
- **`send to <target> ABANDONED — target has unsent/human input or is busy`** — the target's input box stayed non-empty for ~2s (a human is typing, or it's mid-generation). Nothing was pasted, to avoid clipping that input. Wait, then re-send.
- **`send to <target> UNCONFIRMED — submit not verified`** (or `submit UNCONFIRMED to pane …`) — the message was pasted but ae couldn't confirm it left the input box after retrying Enter. It may or may not have sent; re-send. ae keeps no outbox — the loud failure is your cue.

## `reply` rejected: `request … is assigned to slot …`

```text
Error: request 'ae-…' is assigned to slot 'worker.0'@'my-feature', current pane is slot 'main'@'my-feature'
```

Replies are verified by the request's **slot** (the routing key), not the display name — you're replying from the wrong pane. Run the exact `reply` command from the agent the request was addressed to. `--as` sets the displayed sender only; it cannot satisfy the slot check.

## Watchdog keeps nudging an agent that's done

The agent must call `mark-done` *after* the most recent watchdog nudge:

```bash
~/.ae/sessions/<name>/mark-done "finished my work"
```

`mark-done` emits a `done` event. The watchdog honors it until a newer ae event mentions the agent. If you nudge the agent again afterwards (via `send` / `ask` / etc.), that newer event invalidates the done. To re-mark, run `mark-done` again.

## Watchdog alerts but I'm not at the terminal

Watchdog alerts go to tmux `display-message` (10 seconds) and `events.jsonl`. There's no external notifier. For overnight runs, tail the event log to your pager:

```bash
tail -F ~/.ae/sessions/<name>/events.jsonl \
  | grep --line-buffered '"action":"alert"' \
  | while read line; do <send-yourself-a-push>; done
```

## Codex session id capture failed

Codex has no launch-time UUID flag, so the core runs a chain in a detached child: the id
file codex's own first-task instruction writes, then a launch-token scan of
`~/.codex/sessions/YYYY/MM/DD/*.jsonl`, then a cwd scan of the same files, then the TUI
header. Every scan is filtered by the seat's recorded launch time, so a stale conversation in
the same directory cannot be captured as this one.

You do not have to do anything if it fails. The capture child can die before codex answers —
the machine sleeps, the session is resumed, the process is killed with the pane it was
launched beside — and the watchdog closes that gap: every cycle it takes one look at each
seat still pending and registers whatever it finds. The next tick is the retry. A seat that
stays pending across several cycles means codex never wrote an id worth finding; resume then
falls back to a fresh conversation.

## Pane shows `(null)` agent label

`tmux set-option @ae_agent` failed for that pane. Refresh the session (`ae doctor --refresh <name>`) — it rewrites pane labels and tags along with the helper shims and `workspace.md`. If the pane is missing entirely, that's a different problem (agent CLI exited); `peek <agent>` shows what it printed on the way out.

## Using fish or zsh

Fine. ae runs under bash; your interactive shell does not need to be bash as long as ae is launched correctly (and `~/.local/bin` is on `PATH`).

## On WSL2

Primary development target. `ae doctor` is your friend.

## On macOS / non-Ubuntu Linux

Supported, and Homebrew `coreutils` is **not** required. The GNU/BSD-divergent tools
(`tac`, `stat`, `date -d`, `sed -i`, `grep -oP`) are simply not called any more: everything
that used to reach for them is Rust. `flock` and `timeout` are likewise no longer ae's
dependencies — the core locks with its own `flock(2)` and times out in its own code — so
`ae doctor` no longer reports rows for them.

One macOS requirement stands: `ae` itself needs **bash >= 4.0**, because macOS ships 3.2.
`brew install bash` and put brew's bin directory ahead of `/bin` on `PATH`. The session
helpers do not care — each is a one-line `exec` that runs fine under 3.2 — but the glue does,
and it re-execs itself to get there.

Resume / session-id capture for external CLIs still depends on each tool's local
storage format, which differs across platforms.

## Where to look when things break

In priority order:

1. **`events.jsonl`** — the durable audit trail.
2. **`peek <agent>`** — see what the agent itself thinks happened.
3. **`peek _watchdog`** — watchdog's decision log.
4. **`meta`** — session metadata.
5. **`workspace.md`** — manifest agents are pointed at.

Almost every behavior in ae is observable from those five files.
