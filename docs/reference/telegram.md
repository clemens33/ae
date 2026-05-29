# Telegram bridge

> Machine-global daemon that forwards filtered events from every ae session on the host to one Telegram chat. Single bot, single user, low traffic. Bidirectional control (chat → ae) is tracked separately in [issue #1](https://github.com/clemens33/ae/issues/1).

## What it does

- Reads `events.jsonl` from every session under `~/.ae/sessions/<name>/`.
- Forwards events matching the configured include set (default: `send`, `ask`, `review`, `reply`, `done`, `alert`, `throttled`) to your Telegram chat via the Bot API.
- Persists per-session byte offsets so daemon restarts don't replay history.
- Runs as a background tmux session named `ae-telegram`.

Stage 2 is read-only — the bridge does not yet listen for incoming Telegram messages. See the [bridge protocol](../internals/bridge-protocol.md) for the substrate it builds on.

## Dependencies

`jq` and `curl` are **feature-only**: ae's core commands (`ae <name>`, `ae list`, `ae stop`, …) do not require them. The bridge refuses to start if either is missing.

```bash
ae doctor              # reports OK/WARN for the telegram block when configured
```

## Setup

One-time, after creating a bot via [@BotFather](https://t.me/BotFather):

```bash
ae telegram setup
```

The interactive prompt asks for the bot token and your numeric Telegram chat id, writes the token to `~/.config/ae/telegram-bot.token` (chmod 600, owned by you), and appends a `[telegram]` block to `~/.ae/config`:

```toml
[telegram]
enabled = true
token_file = "~/.config/ae/telegram-bot.token"
chat_id = 123456789
allowed_user_ids = "123456789"
```

Optional keys:

| Key | Default | Purpose |
|---|---|---|
| `include` | `send,ask,review,reply,done,alert,throttled` | Comma-separated action allow-list |
| `exclude` | *(empty)* | Comma-separated action deny-list (applied after include) |

Set `enabled = false` (or remove the block) to disable autostart without uninstalling.

## Commands

```bash
ae telegram setup      # interactive token + config setup
ae telegram start      # spawn daemon, persist enabled=true
ae telegram stop       # kill daemon, persist enabled=false
ae telegram status     # show runtime + intent + deps + token validation
```

`start` is idempotent. `stop` does not error when the daemon is already stopped. `status` prints both the persisted intent and the current runtime state.

## Lifecycle and auto-start

The daemon is **per-machine**, not per-session. A single instance serves every ae session on the host.

| Trigger | Behavior |
|---|---|
| `ae telegram start` | Spawn daemon now, persist `enabled = true`. |
| `ae <name>` (start or resume) | If `enabled = true` and daemon not running and deps present, spawn it. **Never blocks session start** — any failure is a one-line stderr warning, agent launch continues. |
| `ae telegram stop` | Kill daemon, persist `enabled = false`. |
| Reboot | Daemon dies with tmux. Next `ae <name>` triggers the autostart hook. For sessions the daemon already tracked, the events written while it was down stay in `events.jsonl` and are forwarded from the saved offset when it restarts. A session first seen *after* the restart is initialized at end-of-file — its pre-restart events are not backfilled. |

**Recovery is bounded, not magic.** This is an outbound-only bridge — Telegram itself queues nothing for the bot. What buffers missed events is the local `events.jsonl` plus the per-session offset in `state.tsv`: a tracked session resumes exactly where it left off, but a brand-new session starts from EOF (no history flood, by design).

**systemd supervision is deferred.** A user unit cannot reliably supervise the current daemon because `ae telegram start` spawns the tmux background session and exits — systemd would see the service as `inactive` immediately, and `Restart=on-failure` would not catch a crashed daemon. A foreground daemon mode for systemd is Stage 5 (see [issue #1](https://github.com/clemens33/ae/issues/1)).

In the meantime, run `ae telegram start` from your shell login (e.g. `~/.bashrc`, `~/.config/fish/conf.d/`) if you want the bridge alive before the first `ae <name>` invocation.

## State files

| Path | Purpose |
|---|---|
| `~/.ae/telegram-daemon` | The generated daemon script (regenerated on every `ae telegram start`) |
| `~/.ae/telegram/state.tsv` | Per-session `(session_id, inode, byte_offset, last_ts)` so restarts don't replay events |
| `~/.ae/telegram/daemon.lock` | `flock` guard preventing two daemons from running at once |
| `~/.config/ae/telegram-bot.token` | The Bot API token (chmod 600, owner-only) |

The bot token never appears in process argv: the daemon passes the URL to `curl` via `-K -` (config on stdin), and logs are passed through a `bot<TOKEN>` → `bot<redacted>` redactor.

## Troubleshooting

**`ae telegram start` reports `Error: token file not owned by you`**
Reset ownership: `chown $USER ~/.config/ae/telegram-bot.token`. Or check the path matches what's in `~/.ae/config` (`telegram.token_file`).

**`ae telegram start` reports `Error: token file must be chmod 600`**
Fix perms: `chmod 600 ~/.config/ae/telegram-bot.token`.

**Daemon is running but no messages arrive**
The bridge runs in its own tmux session (`ae-telegram`), not as an ae agent — the `peek` helper won't find it. Inspect its log directly:

```bash
tmux capture-pane -p -t ae-telegram:0 | tail -50    # snapshot
tmux attach -t ae-telegram                          # interactive (Ctrl+b d to detach)
```

Likely causes: wrong `chat_id`, bot was blocked from the user side, network outage, or Telegram returned a 4xx (visible in the daemon log with the token redacted).

**`ae <name>` prints `ae telegram: skipped autostart — missing deps:jq`**
Install `jq`. The session continues without the bridge.

**Two ae machines run the same bot**
Each one will fight over `getUpdates` once Stage 3 ships. For now (read-only Stage 2), both will duplicate outbound messages. Use one bot per machine, or stop the daemon on the inactive host.

## What it does NOT do (yet)

- **Bidirectional control.** Chat → ae (`/session <prefix> send ...`) is Stage 3.
- **Multi-platform.** Discord, Slack, etc. would be parallel commands (`ae discord ...`), not a generic bridge abstraction.
- **Rate-limit aggregation.** Each event becomes one Telegram message. A noisy session can hit Telegram's 1 msg/sec per-chat soft limit. Stage 5.
- **Multi-user.** `allowed_user_ids` exists in the config schema for forward compatibility with Stage 3; today it's not consulted.
