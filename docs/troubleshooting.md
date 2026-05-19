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

## Loop watchdog keeps nudging an agent that's done

The agent must call `mark-done` *after* the most recent loop nudge:

```bash
~/.ae/sessions/<name>/mark-done "finished my work"
```

`mark-done` emits a `done` event. The watchdog honors it until a newer ae event mentions the agent. If you nudge the agent again afterwards (via `send` / `ask` / etc.), that newer event invalidates the done. To re-mark, run `mark-done` again.

## Loop nudges right after I marked done

Bug fixed in `de2575e`. If you're seeing it on a session with a long-running watchdog, the running process loaded the old code. Stop and restart:

```bash
~/.ae/sessions/<name>/loop stop
~/.ae/sessions/<name>/loop start
```

`ae doctor --refresh` alone does not restart the running watchdog.

## Loop alerts but I'm not at the terminal

Loop alerts go to tmux `display-message` (10 seconds) and `events.jsonl`. There's no external notifier. For overnight runs, tail the event log to your pager:

```bash
tail -F ~/.ae/sessions/<name>/events.jsonl \
  | grep --line-buffered '"action":"alert"' \
  | while read line; do <send-yourself-a-push>; done
```

## Codex session id capture failed

Codex has no launch-time UUID flag. ae captures it post-launch by scanning `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filtered by launch token and CWD. If the capture failed (network blip, codex crashed before writing its file, etc.), the loop watchdog retries the capture every cycle as step 9. To force a manual retry:

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
3. **`peek _loop`** — watchdog's decision log.
4. **`meta`** — session metadata.
5. **`workspace.md`** — manifest agents are pointed at.

Almost every behavior in ae is observable from those five files.
