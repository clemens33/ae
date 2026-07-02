# Telegram bridge

> Machine-global daemon that bridges every ae session on the host to one Telegram chat: it forwards filtered events out, and (when `allowed_user_ids` is set) accepts a small set of commands back in. Single bot, single user, low traffic.

## What it does

**Outbound (ae → chat):**

- Reads `events.jsonl` from every session under `~/.ae/sessions/<name>/`.
- Forwards events matching the configured include set (default: `send`, `ask`, `review`, `reply`, `done`, `alert`, `throttled`, `chat`) to your Telegram chat via the Bot API.
- Persists per-session byte offsets so daemon restarts don't replay history.
- Runs as a background tmux session named `ae-telegram`.
- `chat` events are agents' free-text replies, sent with the [`say`](helpers.md) helper. They forward like any other event, and because they carry the standard `[session] chat  actor` header you can **reply to them in Telegram to answer that agent** — the two-way conversation loop. An explicit `include` that omits `chat` silently disables this; `ae telegram status` warns when that's the case.

**Inbound (chat → ae):** enabled only when `allowed_user_ids` is set (see [Inbound commands](#inbound-commands-chat--ae)). The daemon polls for messages and lets an authorized user drive sessions from their phone. With no `allowed_user_ids`, the bridge stays outbound-only.

See the [bridge protocol](../internals/bridge-protocol.md) for the substrate it builds on.

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
| `include` | `send,ask,review,reply,done,alert,throttled,chat` | Comma-separated action allow-list (`chat` carries agents' `say` replies — drop it and the two-way loop goes silent) |
| `exclude` | *(empty)* | Comma-separated action deny-list (applied after include) |
| `allowed_user_ids` | *(from setup)* | Comma/space list of Telegram numeric user ids permitted to send **inbound commands**. Empty → inbound disabled (outbound-only). |

Set `enabled = false` (or remove the block) to disable autostart without uninstalling.

## Inbound commands (chat → ae)

Inbound is active **only when `allowed_user_ids` is non-empty** (`ae telegram setup` seeds it with your own id). With no allow-list, the bridge is outbound-only.

**Trust boundary.** Every incoming message must satisfy **all** of:

- `from.id` is numeric and listed in `allowed_user_ids`,
- `chat.id` equals the configured `chat_id` (the 1:1 control channel),
- the chat is `private`.

Anything else is silently dropped. Commands are never accepted from groups or any other chat, even from an allow-listed user.

Three ways to reach an agent, easiest first:

**1. Reply-to-routing (the easy path).** **Reply** (Telegram swipe-reply) to any event the bridge forwarded and your message goes straight to that event's agent — no session/agent typing. The bridge reads the replied-to message's header (`[session] action  actor …`) and revalidates it exactly like `/session … send`, so a reply can only ever reach a real agent you could already address.

**2. Compact prefix — `@session:agent <msg>`.** Start a new thread without `/session`: `@mdk:claude:lead deploy now`. `session` is up to the first `:`; `agent` is the rest (keeps its `alias:name` colon). Sent as a `send`.

**3. Plain messages → the steward (auto-default).** With no override set, a **plain message** (no slash, no `@`) goes to your running **steward** (the `meta_agent` session — canonical `steward`, else legacy `hub`) as a `send`. So once `ae steward` is up you just talk to it — no setup. If no steward is running you're guided to start one (or use a target form above).

**4. Sticky override — `/use`.** `/use <session> <agent>` redirects plain messages to another session; `/use` shows the current routing; `/use clear` drops the override and plain messages go back to the steward. Use this to hold a conversation with one specific session for a while.

**Precedence** (per message, after trimming): starts with `/` → **always a command** (even sent as a reply); else a reply → routes to that event's agent; else `@…` → compact send; else a plain message → the `/use` override if set, else the running steward (auto-default), else a hint to start one. Every path funnels through the same session/agent revalidation — none can escape a real running session.

**Grammar (explicit commands):**

```
/help                                     show this list
/list                                     running sessions: name, session_id[:8], last activity
/use <name|id-prefix> <agent>             redirect plain messages to this session (override the steward default)
/use clear                                drop the override; plain messages go to the steward again
/session <name|id-prefix> send <agent> <msg…>   one-way message into an agent pane
/session <name|id-prefix> ask  <agent> <msg…>   tracked request; the reply routes back to chat
@session:agent <msg…>                     compact one-off send (no /session)
```

- `<name|id-prefix>` resolves only against **running** sessions (exact name, or a unique `session_id` prefix). Stopped sessions are not offered or addressable.
- `<agent>` is validated against the resolved session's real agents (exact `alias:name` or a unique bare name) and canonicalized before dispatch. `%pane-id`, `@other-session:agent`, and `telegram:`/`discord:` targets are rejected — a command can't escape the named session.
- Commands run with the sender identity `telegram:<your-id>`; `ask` replies and any agent message targeting `telegram:<your-id>` flow back out via the outbound path.

**Command menu.** When inbound is enabled, the daemon registers the slash commands (`/list`, `/use`, `/session`, `/help`) with Telegram (`setMyCommands`) on startup, so they show up in the chat's `/` menu — no need to memorise the grammar. Best-effort: a registration failure is logged and ignored.

**Replay safety.** The daemon advances its `getUpdates` offset (persisted in `~/.ae/telegram/tg_offset`) before dispatching, so a crash can't re-run a side-effecting command on restart (at-most-once).

## Steward-centric routing: talk to the meta-agent, not ten sessions

The [`ae steward`](commands.md#ae-steward) meta-agent turns the bridge from a
*broadcast* (every session shouting events at you) into a *conversation* (you talk
to one agent that watches the rest and relays for you). This is **not a new
mechanism** — it's a setup on top of the routing above:

1. **Run the bridge and the steward together.** The steward reports to you only
   through its [`say`](helpers.md) helper, which emits `chat` events the bridge
   forwards like any other — so it appears in your chat as
   `[steward] chat  claude:steward …`.
2. **The steward is your default correspondent — automatically.** Every plain
   message you type (no slash, no `@`) goes to the running steward as a `send`;
   no `/use` needed. So you just talk to it. (`/use <session> <agent>` still
   redirects to a specific session when you want that; `/use clear` returns to
   the steward.) This is also how the focus-mode protocol travels: `objective: …`,
   `idea: …`, `focus`, `status`, `what next`, `snooze` are just plain messages.
3. **The loop closes both ways:**
   - **you → steward** — plain text (auto-default) *or* a swipe-reply to any
     of its messages (reply-to-routing) reaches `steward:claude:steward`.
   - **steward → you** — its `say` reports land in the chat.
   - **steward → other sessions** — it relays your instruction with `send`/`ask`
     to `@othersession:agent` and reports back what it sent (and the request id).
   - **other sessions → you** — their events still forward directly (the bridge is
     machine-global), so you also see raw activity, not only the steward's summary.

### Tuning the signal

Because the bridge forwards **every** session's events, with the steward running
you receive both its curated reports *and* the raw event stream. To lean on the
steward as your primary signal and quiet the rest, narrow the outbound filter — e.g.
keep the conversational + alerting actions and drop routine relays:

```toml
[telegram]
include = "chat,alert,throttled,ask,reply,done"   # steward 'say' = chat; keep alerts
```

`nudge` is already outside the default include, so the steward's own sweep prompts
never reach your phone. **Keep `chat` in the include** or the steward cannot talk to
you at all.

> The steward does not replace the bridge — it sits on top of it. You can still
> address any session directly (`@session:agent …`, `/session …`) while the steward
> is your sticky default; the steward is a convenience, not a gatekeeper.

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
| Watchdog | While any session's [watchdog](../internals/watchdog.md) is running, it best-effort revives the daemon every ~`AE_WATCHDOG_TG_SUPERVISE_SEC` seconds (default 120) if `enabled = true` and it died. Idempotent + respects the `enabled` flag, so a deliberate `ae telegram stop` is **not** undone. This is the closest thing to supervision without systemd — see below. |
| Reboot | Daemon dies with tmux. Next `ae <name>` (or a running watchdog's supervision tick) triggers the autostart hook. For sessions the daemon already tracked, the events written while it was down stay in `events.jsonl` and are forwarded from the saved offset when it restarts. A session first seen *after* the restart is initialized at end-of-file — its pre-restart events are not backfilled. |

**Recovery is bounded, not magic.** For the **outbound** path, Telegram queues nothing — what buffers missed events is the local `events.jsonl` plus the per-session offset in `state.tsv`: a tracked session resumes exactly where it left off, while a brand-new session starts from EOF (no history flood, by design). For the **inbound** path, Telegram *does* hold undelivered `getUpdates` for ~24h, so commands sent while the daemon is down are processed when it next polls (and the offset in `tg_offset` prevents re-running already-handled ones).

**Watchdog supervision (best-effort).** The per-session watchdog re-runs the autostart check every ~120s (`AE_WATCHDOG_TG_SUPERVISE_SEC`, set `0` to disable), so a crashed daemon is revived within a couple of minutes **as long as at least one session's watchdog is alive**. start / stop / supervise serialize on a control lock (`~/.ae/telegram/control.lock`) and the revive re-checks `enabled` under that lock, so a deliberate `ae telegram stop` is never undone by an in-flight tick, and concurrent watchdogs across sessions don't fight. The supervise call inherits the session's tmux server, so multi-server setups revive on the right server. This is not a hard supervisor: if every session is stopped (or `ae watchdog` is off everywhere), nothing watches the bridge.

> **No crash-loop backoff (yet).** If the daemon exits immediately while `enabled = true` (e.g. a bad token slips past validation), each running watchdog will keep re-spawning it on its ~120s cadence — visible as repeated `daemon started` / `daemon exiting` pairs in `daemon.log`. It won't duplicate daemons (lock-guarded), just churn. Watch the log; `ae telegram stop` halts it.

**systemd supervision is deferred.** For a *hard* guarantee independent of running sessions, a user unit would need a foreground daemon mode (`ae telegram start` currently spawns the tmux session and exits, so `Restart=on-failure` can't catch a crash). That's Stage 5 (see [issue #1](https://github.com/clemens33/ae/issues/1)).

To have the bridge alive before the first `ae <name>` of a session, run `ae telegram start` from your shell login (e.g. `~/.bashrc`, `~/.config/fish/conf.d/`).

## State files

| Path | Purpose |
|---|---|
| `~/.ae/telegram-daemon` | The generated daemon script (regenerated on every `ae telegram start`) |
| `~/.ae/telegram/state.tsv` | Per-session `(session_id, inode, byte_offset, last_ts)` so restarts don't replay events |
| `~/.ae/telegram/daemon.lock` | `flock` guard preventing two daemons from running at once |
| `~/.ae/telegram/daemon.log` | All daemon output (stderr+stdout), so a crash is diagnosable after the tmux session is gone. Rotated to `daemon.log.1` once at startup past ~1 MiB (`AE_TELEGRAM_LOG_MAX_BYTES`) |
| `~/.config/ae/telegram-bot.token` | The Bot API token (chmod 600, owner-only) |

The bot token never appears in process argv: the daemon passes the URL to `curl` via `-K -` (config on stdin), and logs are passed through a `bot<TOKEN>` → `bot<redacted>` redactor.

## Troubleshooting

**`ae telegram start` reports `Error: token file not owned by you`**
Reset ownership: `chown $USER ~/.config/ae/telegram-bot.token`. Or check the path matches what's in `~/.ae/config` (`telegram.token_file`).

**`ae telegram start` reports `Error: token file must be chmod 600`**
Fix perms: `chmod 600 ~/.config/ae/telegram-bot.token`.

**Daemon is running but no messages arrive**
The bridge runs in its own tmux session (`ae-telegram`), not as an ae agent — the `peek` helper won't find it. Read its log file (persists across restarts and across the session dying):

```bash
tail -n 50 ~/.ae/telegram/daemon.log     # recent output
tail -f  ~/.ae/telegram/daemon.log        # follow live
```

Likely causes: wrong `chat_id`, bot was blocked from the user side, network outage, or Telegram returned a 4xx (visible in the log with the token redacted).

**Daemon died and you want to know why**
Because all output (including the bash error that killed it) is captured to the log, the cause survives the session. Check the tail and the rotated copy:

```bash
tail -n 30 ~/.ae/telegram/daemon.log      # the "daemon exiting (rc=…)" line + any error above it
tail -n 30 ~/.ae/telegram/daemon.log.1    # previous run, if it rotated
```

**`ae <name>` prints `ae telegram: skipped autostart — missing deps:jq`**
Install `jq`. The session continues without the bridge.

**Two ae machines run the same bot**
They'll fight over `getUpdates` (Telegram 409) and duplicate outbound messages. Use one bot per machine, or stop the daemon on the inactive host.

**Inbound command silently ignored**
Check `allowed_user_ids` includes your numeric id, that you're messaging the bot in a **private** chat, and that `chat_id` matches that chat. Any mismatch is dropped by design (logged in the daemon pane).

## What it does NOT do (yet)

- **Multi-platform.** Discord, Slack, etc. would be parallel commands (`ae discord ...`), not a generic bridge abstraction.
- **Rate-limit aggregation.** Each event becomes one Telegram message. A noisy session can hit Telegram's 1 msg/sec per-chat soft limit. Stage 5.
- **Multi-user / group control.** `allowed_user_ids` may list several ids, but commands are only accepted in the single configured private `chat_id`. Group chats and `allowed_chat_ids` are out of scope.
