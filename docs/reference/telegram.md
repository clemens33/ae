# Telegram bridge

> Machine-global daemon that bridges the shared chat of every running ae session on the host to one Telegram chat: it forwards the human-facing chat out, and (when `allowed_user_ids` is set) asks your phone messages in a session's shared chat. Single bot, single user, low traffic.

## What it does

**Outbound (ae → chat):**

- Reads `events.jsonl` of every **running** session that has a lead pair; a stopped session, or one with no lead pair, relays nothing, and a session that starts later joins on the next pass (about 2 s).
- Forwards only the shared chat's human-facing rows, labelled `[session] <speaker>`: the asks typed in the [chat](../chat.md#telegram) or the `ae app`, the answers to them, and a lead-pair seat's or the watchdog's [`say`](helpers.md) line. Worker-to-worker traffic, states, alerts and tool output stay local, and your own phone message is not echoed back.
- Persists a per-session byte cursor, so a restart does not replay what was sent; see [Restarts](#lifecycle-and-auto-start) for the cutoff a start applies.
- Runs as a background tmux session named `ae-telegram`.
- **Reply to a forwarded message in Telegram** to ask the shared chat it came from — the two-way loop.

**Inbound (chat → ae):** enabled only when `allowed_user_ids` is set (see [Inbound commands](#inbound-commands-chat--ae)). An authorized message is one more writer of the session's shared chat: it becomes a chat ask exactly as one typed in the chat or the `ae app`, and is never pasted into a seat directly. With no `allowed_user_ids`, the bridge stays outbound-only.

See the [bridge protocol](../internals/bridge-protocol.md) for the substrate it builds on.

## Dependencies

The bridge needs **only a configured ae core** — it *is* the ae core binary, run in a background tmux session. There are no extra CLI dependencies. On a machine with no usable core, the public command refuses every operational command — `ae telegram start`, `ae <name>`, `ae list`, and `ae stop` alike — before any operational code runs, naming `ae upgrade` as the repair path. `ae upgrade` and `ae version` remain available so repair and version queries survive breakage.

```bash
ae doctor              # reports telegram.core OK/WARN when telegram is configured
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
| `include` / `exclude` | — | No longer read by the bridge: the outbound filter is the fixed human-facing set above. `ae telegram status` still prints an `include` it finds |
| `allowed_user_ids` | *(from setup)* | Comma/space list of Telegram numeric user ids permitted to send **inbound commands**. Empty → inbound disabled (outbound-only). |

Set `enabled = false` (or remove the block) to disable autostart without uninstalling.

## Inbound commands (chat → ae)

Inbound is active **only when `allowed_user_ids` is non-empty** (`ae telegram setup` seeds it with your own id). With no allow-list, the bridge is outbound-only.

**Trust boundary.** Every incoming message must satisfy **all** of:

- `from.id` is numeric and listed in `allowed_user_ids`,
- `chat.id` equals the configured `chat_id` (the 1:1 control channel),
- the chat is `private`.

Anything else is silently dropped. Commands are never accepted from groups or any other chat, even from an allow-listed user.

Every message is asked in **one session's shared chat** — admitted, capped at five open asks and routed between the lead pair exactly as a line typed in the [chat](../chat.md#telegram). Plain text is never guessed onto a session, not even when only one is running:

1. **Reply-to-routing.** **Reply** (Telegram swipe-reply) to any message the bridge forwarded and your message is asked in the chat of that message's session (of the seat that spoke it, when it is a lead-pair seat). It wins over the selection below.
2. **Compact prefix — `@session <msg>` or `@session:seat <msg>`.** Asks that session's chat once, without changing the selection.
3. **Selection — `/use <session> [seat]`.** Plain messages (no slash, no `@`) then go to that session's chat; `/use` shows the selection; `/use clear` drops it. With nothing selected, a plain message is answered `No session selected — nothing was sent` and goes nowhere. A selection whose session stopped, or whose seat is no longer in the lead pair, is refused rather than redirected.

**Precedence** (per message, after trimming): starts with `/` → **always a command** (even sent as a reply); else a reply → that message's session; else `@…` → that session; else plain text → the `/use` selection, else the answer above.

**Grammar (explicit commands):**

```
/help                                       show this list
/list                                       running sessions: name, session_id[:8], last activity
/use <name|id-prefix> [seat]                select a session for plain messages
/use                                        show the selection
/use clear                                  drop the selection
/session <name|id-prefix> ask  <seat> <msg…>    ask that session's chat, for that seat
/session <name|id-prefix> send <seat> <msg…>    the same ask (the old spelling still works)
@session[:seat] <msg…>                      ask that session's chat once
```

- `<name|id-prefix>` resolves only against **running** sessions (exact name, or a unique `session_id` prefix). Two matches, none, or a stopped session refuse with nothing sent.
- `[seat]` must be one of the session's **lead pair** (`lead` or `claude:lead` alike); a worker is refused. With no seat the chat's own router chooses between the lead pair, as it does for a line typed in the chat.
- An ask is recorded as the chat's own (`human:chat` on screen), so the seat answers it with the reply command it carries, and that answer is forwarded back here.

**Command menu.** When inbound is enabled, the daemon registers the slash commands (`/list`, `/use`, `/session`, `/help`) with Telegram (`setMyCommands`) on startup, so they show up in the chat's `/` menu — no need to memorise the grammar. Best-effort: a registration failure is logged and ignored.

**Replay safety.** The daemon advances its `getUpdates` offset (persisted in `~/.ae/telegram/tg_offset`) only *after* an update is durably handled. Each ask's request id is derived from its Telegram update, so an update replayed after a crash finds its recorded ask and is not asked again. An authorized target that stays undeliverable gets a bounded give-up: after a bounded number of retries the daemon sends the text back to the chat and, *only once Telegram accepts that notice*, advances — so one dead pane can't stall the queue, and nothing is dropped without a delivered notice. Delivery is at least once, never exactly once; the windows that can repeat one message are listed in [the chat's Telegram section](../chat.md#telegram).

## Talking to the orchestrator

The [orchestrator seat](commands.md#ae-orchestrator) is a session like any other here: `/use orchestrator` and your plain messages are asked in its chat (`objective: …`, `status`, `what next`, … are plain messages). Its `say` reports reach you as `[orchestrator] …`. Nothing is routed to it by default.

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
| `ae <name>` (start or resume) | If `enabled = true` and the daemon is not running, spawn it — the spawn resolves the ae core and a missing/unusable core is a one-line stderr warning. **Never blocks session start** — any failure is a one-line stderr warning, agent launch continues. |
| `ae telegram stop` | Kill daemon, persist `enabled = false`. |
| Watchdog | While any session's [watchdog](../internals/watchdog.md) is running, it best-effort revives the daemon every ~`AE_WATCHDOG_TG_SUPERVISE_SEC` seconds (default 120) if `enabled = true` and it died. Idempotent + respects the `enabled` flag, so a deliberate `ae telegram stop` is **not** undone. This is the closest thing to supervision without systemd — see below. |
| Reboot | Daemon dies with tmux. Next `ae <name>` (or a running watchdog's supervision tick) triggers the autostart hook. What was written while it was down is subject to the start cutoff below. |

**Each start fixes a cutoff ten minutes before it started**, and it does not move while that daemon runs. **Outbound:** a chat row older than the cutoff is not sent, even behind a valid cursor; it stays in the session's `events.jsonl`, its count goes to `~/.ae/telegram/backlog.log`, and the chat hears one notice. A recent row that failed to send is still owed and retried. **Inbound:** Telegram holds undelivered `getUpdates` for about 24 h; a phone message sent before the cutoff is not acted on but kept in `~/.ae/telegram/stale-inbound.jsonl`, and the chat hears one notice. So a start says at most two notices, one per direction, and floods nothing.

**Watchdog supervision (best-effort).** The per-session watchdog re-runs the autostart check every ~120s (`AE_WATCHDOG_TG_SUPERVISE_SEC`, set `0` to disable), so a crashed daemon is revived within a couple of minutes **as long as at least one session's watchdog is alive**. start / stop / supervise serialize on a control lock (`~/.ae/telegram/control.lock`) and the revive re-checks `enabled` under that lock, so a deliberate `ae telegram stop` is never undone by an in-flight tick, and concurrent watchdogs across sessions don't fight. The supervise call inherits the session's tmux server, so multi-server setups revive on the right server. This is not a hard supervisor: if every session is stopped (or `ae watchdog` is off everywhere), nothing watches the bridge.

> **No crash-loop backoff (yet).** If the daemon exits immediately while `enabled = true` (e.g. a bad token slips past validation), each running watchdog will keep re-spawning it on its ~120s cadence — visible as the `ae-telegram` tmux session repeatedly reappearing and dying. It won't duplicate daemons (a single tmux session name is the guard), just churn. `ae telegram stop` halts it.

**systemd supervision is deferred.** For a *hard* guarantee independent of running sessions, a user unit would need a foreground daemon mode (`ae telegram start` currently spawns the tmux session and exits, so `Restart=on-failure` can't catch a crash). That's Stage 5 (see [issue #1](https://github.com/clemens33/ae/issues/1)).

To have the bridge alive before the first `ae <name>` of a session, run `ae telegram start` from your shell login (e.g. `~/.bashrc`, `~/.config/fish/conf.d/`).

## State files

There is **one** bridge implementation: the ae core binary, run in the
`ae-telegram` tmux session.

| Path | Purpose |
|---|---|
| `~/.ae/telegram/tg_offset` | Durable inbound `getUpdates` cursor. Advances only *after* an update is durably routed — at-least-once (see **Replay safety**, above) |
| `~/.ae/telegram/current_target` | The `/use` selection: `<session>\t<seat>`, `*` for the chat's own router |
| `~/.ae/telegram/stale-inbound.jsonl` | Phone messages a start held back as older than its cutoff, one JSON object each with their text (0600, never read back) |
| `~/.ae/telegram/backlog.log` | One line per outbound pass that held rows back: time, session, count, cutoff; no text (0600) |
| `~/.ae/telegram/autostart-refusal` | Last autostart refusal as a fixed category (`same-token-live`, `token-unreadable`, `probe-failed`, or `spawn-failed`) plus its UTC timestamp; retained after a later successful start |
| `<session-meta>/telegram-outbound.cursor` | Per-session outbound forward cursor `(inode, byte_offset)`, written beside each session's `events.jsonl` so restarts don't replay what was sent; a clean pass checkpoints past rows it skipped. There is deliberately **no** single global outbound file — progress is per session |
| `~/.ae/telegram/control.lock` | Serializes `start` / `stop` / `supervise` so concurrent watchdogs across sessions don't fight |
| `~/.config/ae/telegram-bot.token` | The Bot API token (chmod 600, owner-only) |

The bot token never appears in process argv or in any log: it rides the **URL path** (`/bot<TOKEN>/sendMessage`, `/bot<TOKEN>/getUpdates`) over TLS via the core's locked HTTP client (proxy off, https-only, no redirects, finite timeouts), and every surfaced API error is redacted before it is printed.

## Troubleshooting

**`ae telegram start` reports `Error: token file not owned by you`**
Reset ownership: `chown $USER ~/.config/ae/telegram-bot.token`. Or check the path matches what's in `~/.ae/config` (`telegram.token_file`).

**`ae telegram start` reports `Error: token file must be chmod 600`**
Fix perms: `chmod 600 ~/.config/ae/telegram-bot.token`.

**Daemon is running but no messages arrive**
The bridge runs in its own tmux session (`ae-telegram`), not as an ae agent — the `peek` helper won't find it. Read its live output straight from the pane:

```bash
tmux capture-pane -pt ae-telegram -S -100   # last 100 lines
tmux attach -t ae-telegram                  # follow live (Ctrl-b d detaches)
```

Likely causes: wrong `chat_id`, bot was blocked from the user side, network outage, or Telegram returned a 4xx (surfaced in the pane with the token redacted). Config-level problems (an unusable core, a bad token file) are caught up front by `ae telegram status` and `ae doctor`.

**Daemon won't stay up**
`ae telegram status` shows it not running. A startup refusal prints a single `ae: telegram: …` line to the `ae-telegram` pane before the daemon exits (missing/unreadable token file, wrong token permissions, no usable ae core, an uncreatable state dir). Autostart refusals are also persisted as a fixed redacted category and timestamp in `~/.ae/telegram/autostart-refusal`; `ae telegram status` and `ae doctor` show that last refusal even while the bridge is down. Note the pane is **not** a persistent log — it dies with the tmux session, so use the status/doctor row for the durable refusal record.

**Two ae machines run the same bot**
They'll fight over `getUpdates` (Telegram 409) and duplicate outbound messages. Use one bot per machine, or stop the daemon on the inactive host.

**Inbound command silently ignored**
Check `allowed_user_ids` includes your numeric id, that you're messaging the bot in a **private** chat, and that `chat_id` matches that chat. Any mismatch is dropped by design (logged in the daemon pane).

## What it does NOT do (yet)

- **Multi-platform.** Discord, Slack, etc. would be parallel commands (`ae discord ...`), not a generic bridge abstraction.
- **Rate-limit aggregation.** Each chat row becomes one Telegram message. A noisy session can hit Telegram's 1 msg/sec per-chat soft limit. Stage 5.
- **Multi-user / group control.** `allowed_user_ids` may list several ids, but commands are only accepted in the single configured private `chat_id`. Group chats and `allowed_chat_ids` are out of scope.
