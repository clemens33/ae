# Telegram bridge

> Machine-global daemon that connects ae shared chat to one Telegram chat. By default, the proven orchestrator's chat is the only session chat it forwards. When `allowed_user_ids` is set, the bridge routes phone messages into shared chat.

## What it does

**Outbound (ae → chat):**

- In the default `scope = orchestrator`, each pass checks the same role census as inbound routing. If the census proves a seat, the bridge forwards its chat by canonical session directory and includes any running session listed in `also`. A stopped but proven seat still sets the scope. Without a proven seat, or with `scope = all`, the bridge forwards every running session's chat. A session needs a lead pair to forward.
- Forwarded rows carry the label `[session] <speaker>`. They include asks typed in chat or `ae app`, answers, and chat lines from the lead pair, watchdog or ae itself. The global `[workspace] relay_replies = on` adds the orchestrator's raw relay answer lines to its forwarded chat. [Chat forwarding](../chat.md#telegram)
- A reply to an ask sent from Telegram can reach the phone from any session while this bridge process remembers the ask. Phone asks stay out of Telegram echoes while the bridge remembers them. After a bridge restart, a pending answer from a session outside the configured scope can stay in ae chat and the app.
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
| `scope` | `orchestrator` | `orchestrator` forwards the proven seat's chat and any `also` sessions. `all` forwards eligible chat from every running session. |
| `also` | Empty | Comma-separated exact session names or full UUIDs. Names are case-sensitive and UUIDs are case-insensitive. Each entry must match one running session to add it. |
| `allowed_user_ids` | *(from setup)* | Comma/space list of Telegram numeric user ids permitted to send **inbound commands**. Empty → inbound disabled (outbound-only). |

Restart the bridge to apply `scope` or `also` changes. Invalid settings and repeated keys refuse startup with the file and line. Rows skipped by scope stay in the session journal and advance its outbound cursor.

Set `enabled = false` (or remove the block) to disable autostart without uninstalling.

## Inbound commands (chat → ae)

Inbound is active **only when `allowed_user_ids` is non-empty** (`ae telegram setup` seeds it with your own id). With no allow-list, the bridge is outbound-only.

**Trust boundary.** Every incoming message must satisfy **all** of:

- `from.id` is numeric and listed in `allowed_user_ids`,
- `chat.id` equals the configured `chat_id` (the 1:1 control channel),
- the chat is `private`.

Anything else is silently dropped. Commands are never accepted from groups or any other chat, even from an allow-listed user.

Every message is asked in **one session's shared chat** — admitted, capped at five open asks and routed between the lead pair exactly as a line typed in the [chat](../chat.md#telegram). Plain text with no selection goes to the orchestrator seat and to no other session, not even when only one is running:

1. **Reply-to-routing.** **Reply** (Telegram swipe-reply) to any message the bridge forwarded and your message is asked in the chat of that message's session (of the seat that spoke it, when it is a lead-pair seat). It wins over `@session` and the selection below. A reply the bridge cannot route — one whose quoted message carries no recognizable session label, such as a photo, a sticker or a message from another chat — is answered `Couldn't tell which session that was a reply to` and never becomes a default ask.
2. **Compact prefix — `@session <msg>` or `@session:seat <msg>`.** Asks that session's chat once, without changing the selection.
3. **Selection — `/use <session> [seat]`.** Plain messages (no slash, no `@`) then go to that session's chat; `/use` shows the selection; `/use clear` drops it, and plain messages ask the orchestrator again. A selection whose session stopped, or whose seat is no longer in the lead pair, is refused rather than redirected. An unreadable selection is refused too, and the answer names `/use clear`.
4. **Default — the orchestrator seat.** With nothing selected, a plain message is asked in the orchestrator seat's chat, exactly as a plain message to a selected session: the same admission, the same cap of five open asks and the same reply threading back here. The seat is the session the role census proves, the one the settings menu's orchestrator control uses: `meta_agent=true` together with the seat overlay. It is found by its canonical state directory, never by its name, so a renamed seat works, while a workspace that carries only the role bit and a worktree session that shares the seat's name do not. When the seat cannot take the message, nothing is sent and the chat gets one line that names the cause and the alternatives `@session` and `/use`. The causes are: no seat recorded, a role claim that cannot be proven (damaged, duplicate, or without the seat overlay), the seat not running, and the seat having no lead pair. The daemon never redirects the message to another session.

**Precedence** (per message, after trimming): starts with `/` → **always a command** (even sent as a reply); else a reply → that message's session, or the refusal above; else `@…` → that session; else plain text → the `/use` selection, else the orchestrator seat, else the refusal of rung 4. Who is authorized does not change: the allow-list and the private-chat rule above apply to every rung.

**Grammar (explicit commands):**

```
/help                                       show this list
/list                                       running sessions: name, session_id[:8], last activity
/quota                                      cached quota by account, with resets and freshness
<msg…>                                     (no slash, no @) ask the selected session, else the orchestrator
/use <name|id-prefix> [seat]                select a session for plain messages
/use                                        show the selection
/use clear                                  drop the selection (plain messages ask the orchestrator again)
/session <name|id-prefix> ask  <seat> <msg…>    ask that session's chat, for that seat
/session <name|id-prefix> send <seat> <msg…>    the same ask (the old spelling still works)
@session[:seat] <msg…>                      ask that session's chat once
```

- `<name|id-prefix>` resolves only against **running** sessions (exact name, or a unique `session_id` prefix). Two matches, none, or a stopped session refuse with nothing sent.
- `[seat]` must be one of the session's **lead pair** (`lead` or `claude:lead` alike); a worker is refused. With no seat the chat's own router chooses between the lead pair, as it does for a line typed in the chat.
- An ask is recorded as the chat's own (`human:chat` on screen), so the seat answers it with the reply command it carries, and that answer is forwarded back here.

**Command menu.** When inbound is enabled, the daemon registers the slash commands (`/list`, `/quota`, `/use`, `/session`, `/help`) with Telegram (`setMyCommands`) on startup, so they show up in the chat's `/` menu — no need to memorise the grammar. Best-effort: a registration failure is logged and ignored.

`/quota` answers from locally held evidence, with effective usage, each window's reset and observation age, and stale or limit notes. It never refreshes vendor quota or calls a model. agy says `not checked`: asking it runs the agy CLI, which this view never does. Long replies end with the number of scopes not shown in full.

**Replay safety.** The daemon advances its `getUpdates` offset (persisted in `~/.ae/telegram/tg_offset`) only *after* an update is durably handled. Each ask's request id is derived from its Telegram update, so an update replayed after a crash finds its recorded ask and is not asked again. An authorized target that stays undeliverable gets a bounded give-up: after a bounded number of retries the daemon sends the text back to the chat and, *only once Telegram accepts that notice*, advances — so one dead pane can't stall the queue, and nothing is dropped without a delivered notice. The refusal of an unavailable orchestrator default is held the same way: it is sent confirmed, and the offset advances only once Telegram accepted it. Delivery is at least once, never exactly once; the windows that can repeat one message are listed in [the chat's Telegram section](../chat.md#telegram).

## Talking to the orchestrator

The [orchestrator seat](commands.md#ae-orchestrator) is where plain messages go by default: with no `/use` selection, your plain messages are asked in its chat (`objective: …`, `status`, `what next`, … are plain messages). `/use <session>` selects another session, `/use clear` returns to the seat, and `@session <msg>` asks any session once. Its `say` reports reach you as `[orchestrator] …`. When the seat is stopped or cannot be proven, you get the one-line refusal of [rung 4](#inbound-commands-chat--ae), never a redirect.

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
| Watchdog | While any session's [watchdog](../internals/watchdog.md) is running, it best-effort revives the daemon every 120 seconds (fixed) if `enabled = true` and it died. Idempotent + respects the `enabled` flag, so a deliberate `ae telegram stop` is **not** undone. This is the closest thing to supervision without systemd — see below. |
| Reboot | Daemon dies with tmux. Next `ae <name>` (or a running watchdog's supervision tick) triggers the autostart hook. What was written while it was down is subject to the start cutoff below. |

**Each start fixes a cutoff ten minutes before startup.** Outbound chat older than the cutoff stays in the session journal. The bridge counts rows within the current scope in `~/.ae/telegram/backlog.log` and sends one notice. It retries recent rows that failed to send. The bridge keeps inbound phone messages older than the cutoff in `~/.ae/telegram/stale-inbound.jsonl` without asking them and sends one notice.

**Watchdog supervision (best-effort).** The per-session watchdog re-runs the autostart check every ~120s (fixed; the internal `--tg-supervise-secs` flag changes it, `0` disables), so a crashed daemon is revived within a couple of minutes **as long as at least one session's watchdog is alive**. start / stop / supervise serialize on a control lock (`~/.ae/telegram/control.lock`) and the revive re-checks `enabled` under that lock, so a deliberate `ae telegram stop` is never undone by an in-flight tick, and concurrent watchdogs across sessions don't fight. The supervise call inherits the session's tmux server, so multi-server setups revive on the right server. This is not a hard supervisor: if every session is stopped (or `ae watchdog` is off everywhere), nothing watches the bridge.

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
| `~/.ae/telegram/autostart-refusal` | Last autostart refusal as a fixed category, such as `settings-invalid`, `token-unreadable` or `spawn-failed`, with its UTC timestamp. The record remains after a successful start. |
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
