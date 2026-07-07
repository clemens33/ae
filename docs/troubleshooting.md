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

Resume and session capture for external agent CLIs are best-effort and depend on upstream tool storage formats, which can change. If exact-session resume fails, ae falls back gracefully (e.g. Claude `--continue`, Codex fresh-start with preserved flags, Gemini `--resume latest`).

## Helpers feel out of date after upgrading ae

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # single session
```

`--refresh` calls the same `sync_session_assets` path that runs on every session start. Regenerates every helper and `workspace.md`. Also runs the orphan sweep.

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

## Watchdog nudges right after I marked done

Bug fixed in `de2575e`. If you're seeing it on a session with a long-running watchdog, the running process loaded the old code. Stop and restart:

```bash
~/.ae/sessions/<name>/watchdog stop
~/.ae/sessions/<name>/watchdog start
```

`ae doctor --refresh` alone does not restart the running watchdog.

## Watchdog alerts but I'm not at the terminal

Watchdog alerts go to tmux `display-message` (10 seconds) and `events.jsonl`. There's no external notifier. For overnight runs, tail the event log to your pager:

```bash
tail -F ~/.ae/sessions/<name>/events.jsonl \
  | grep --line-buffered '"action":"alert"' \
  | while read line; do <send-yourself-a-push>; done
```

## Codex session id capture failed

Codex has no launch-time UUID flag. ae captures it post-launch by scanning `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filtered by launch token and CWD. If the capture failed (network blip, codex crashed before writing its file, etc.), the watchdog retries the capture every cycle as step 9. To force a manual retry:

```bash
~/.ae/sessions/<name>/_register-sid worker.0   # adjust slot as needed
```

## Pane shows `(null)` agent label

`tmux set-option @ae_agent` failed for that pane. Refresh the helpers (`ae doctor --refresh <name>`) — `regenerate_manifest` rewrites pane labels and tags. If the pane is missing entirely, that's a different problem (agent CLI exited); use `ae status <name>` to inspect.

## Using fish or zsh

Fine. ae runs under bash; your interactive shell does not need to be bash as long as ae is launched correctly (and `~/.local/bin` is on `PATH`).

## On WSL2

Primary development target. `ae doctor` is your friend.

## On macOS / non-Ubuntu Linux

Best-effort. ae uses some GNU coreutils-isms (`tac`, `date -d`, `stat -c %Y`). On macOS you'll likely need `coreutils` from Homebrew (then `gtac`, `gdate`, `gstat`). Resume / session-id capture for external CLIs depends on their local storage formats which differ across platforms.

## Where to look when things break

In priority order:

1. **`events.jsonl`** — the durable audit trail.
2. **`peek <agent>`** — see what the agent itself thinks happened.
3. **`peek _watchdog`** — watchdog's decision log.
4. **`meta`** — session metadata.
5. **`workspace.md`** — manifest agents are pointed at.

Almost every behavior in ae is observable from those five files.
