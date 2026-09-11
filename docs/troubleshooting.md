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

## Every session refuses to resume after a reboot

```
Error: cannot verify whether tmux session '<name>' is absent on its recorded server.
       recorded server /tmp/tmux-501/default: socket missing; last live activity
       2026-09-11T05:40:00Z is not before boot 2026-09-11T05:30:55Z — cannot prove
       the session gone. Run 'ae doctor' for the evidence.
```

A reboot takes every tmux socket under `/tmp/tmux-<uid>/` with it, and a missing
socket is not by itself proof that a session is gone: a server that is still
running answers exactly the same error once something unlinks its socket. ae
resolves that with the host's boot time. A session whose own last sign of life
predates the boot cannot be on any server that is running now, so a resume moves
it to the configured server and rewrites its `tmux_server` row.

The message above is the case where ae could **not** prove it: this session was
last live *after* the boot it is being compared against, so it may still be
running somewhere ae cannot reach. `ae doctor` prints both numbers — a `boot`
row, and a `last-live:<name>` row per session. The same evidence decides
`ae list`: a vanished server whose every recorded session predates the boot is
listed as holding nothing, rather than leaving `unknown` rows and an "inventory
incomplete" warning.

`stop`, `end` and `compact` never use this reasoning. They are irreversible, so
they still require the server's own answer.

Three things leave a session unprovable, and `ae doctor` says which:

* **`no recorded live activity`** — a session so old it carries none of the
  facts ae reads. Nothing is damaged; there is simply nothing to compare.
* **`is there and could not be read`** — a record that exists and is unreadable:
  wrong permissions, a directory where a file belongs, or a file whose contents
  are not a moment. Repair or delete the named file and the session becomes
  provable again. Damaged evidence never falls back to an older record, because
  that older record would then authorise an absence the damaged one might
  contradict.
* **the boot time itself is unreadable** — nothing to compare against at all.

`ae stop <name>` is **not** a way out of any of them: stop keeps the strict
proof and refuses the same ENOENT, so it answers exactly the same way a resume
does.

What does work, when you know the session really is gone: give the recorded
server something to answer with, so the strict proof applies again. Start a
throwaway tmux server on that exact socket path, then resume normally — ae sees
a server that answers and does not list the session, which proves it absent and
moves it to the configured server.

```bash
ae doctor | grep -E 'boot|last-live'          # which number is the problem
tmux -S /tmp/tmux-501/default -f /dev/null \
     new-session -d -s ae-recovery-probe sleep 300
ae <name>                                      # resumes and re-homes
```

The throwaway session is inert and dies with its `sleep`; nothing writes to your
session metadata but the resume itself.

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

Codex has no launch-time UUID flag. Its first-task instruction writes an id file, which the
detached capture verifies against the rollout carrying that seat's launch token; the fallback
scans `~/.codex/sessions/YYYY/MM/DD/*.jsonl` for the same token from the capture floor's UTC
day through today. A missing legacy floor is bounded to the last 30 days. A token miss stays
pending. Only a legacy seat with no token may fall back to cwd and the TUI header, and its cwd
scan stays restricted to today and yesterday. Every scan is also filtered by
`capture_floor.<slot>`, published before the tool starts; Codex uses the rollout's creation
timestamp, not its mutable file mtime. An exact resume preserves the original floor.
A token-proven handshake commits immediately and repairs a wrong id already recorded for the
same launch. A tokenless legacy handshake may fill `pending`, but never replace an id.

If a running Codex seat already records the wrong id, run
`<meta_dir>/_register-sid <slot>` in that seat's still-running pane; its rollout token proves
the repair. If that is unavailable, retire and re-spawn the seat as a fresh incarnation. Never
blindly stop/resume it: resume retains `harness_session.<slot>` and reopens the wrong conversation.

You do not have to do anything if it fails. The capture child can die before codex answers —
the machine sleeps, the session is resumed, the process is killed with the pane it was
launched beside — and the watchdog closes that gap: every cycle it takes one look at each
seat still pending and registers whatever it finds. The next tick is the retry. A seat that
stays pending across several cycles means codex never wrote an id worth finding; resume then
falls back to a fresh conversation.

## Pane shows `(null)` agent label

`tmux set-option @ae_agent` failed for that pane. Refresh the session (`ae doctor --refresh <name>`) — it rewrites pane labels and tags, re-links the helpers, and re-renders `workspace.md`. If the pane is missing entirely, that's a different problem (agent CLI exited); `peek <agent>` shows what it printed on the way out.

## Using fish or zsh

Fine. `ae` is a native Rust binary; your interactive shell does not need to be bash as
long as `ae` is launched correctly (and `~/.local/bin` is on `PATH`).

## On WSL2

Primary development target. `ae doctor` is your friend.

## On macOS / non-Ubuntu Linux

Supported, and Homebrew `coreutils` is **not** required. The GNU/BSD-divergent tools
(`tac`, `stat`, `date -d`, `sed -i`, `grep -oP`) are simply not called any more: everything
that used to reach for them is Rust. `flock` and `timeout` are likewise no longer ae's
dependencies — the core locks with its own `flock(2)` and times out in its own code — so
`ae doctor` no longer reports rows for them.

One macOS requirement stands for installation: the `install` bootstrap needs a modern Bash,
because macOS ships 3.2. Install one with Homebrew and put brew's bin directory ahead of
`/bin` on `PATH`. `ae` itself is the Rust core and needs only tmux and git; session helpers
are symlinks to that same binary and do not invoke Bash.

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
