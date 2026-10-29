# Troubleshooting

## `tmux` not found

Install tmux and rerun `ae doctor`. ae fails fast on startup if tmux is missing.

## Agent CLI not found

```bash
ae doctor
```

Look at the `agent:<alias>` lines. Each one verifies the agent's executable is on `PATH`. Fix the command in `~/.ae/config` or add the CLI to `PATH`.

## `ae <word>` says `looks like a mistyped ae command`

```
Error: 'sessions' looks like a mistyped ae command, not a new session name. Did you mean: ae list? ...
```

`ae <name>` creates a session when none of that name exists, so a typo for a command (`ae sessions`, `ae agents`, `ae lists`) would have created an empty one. ae refuses to create a session named after a routed command plus `s`, or after `agent`, `seat` or `session` (singular or plural), and exits 2 with nothing written. Run the command it suggests, or pick another name. A session that already carries such a name resumes, stops and lists as usual.

## `ae` installed but command not found

Make sure `~/.local/bin` is on `PATH`. `ae doctor` warns explicitly when it isn't.

## Session won't resume cleanly

```bash
ae stop <name>
ae <name>
```

Resume and session capture for external agent CLIs are best-effort and depend on upstream tool storage formats, which can change. All seven supported tools get exact-session resume once their session id is captured; if capture failed or the recorded conversation is gone, ae starts fresh instead of resuming another seat's newest conversation (Claude and Grok name the new conversation with a fresh `--session-id`; the rest take the bare command).

Under `[workspace] auto_effort = on` (global, default off) the watchdog restarts a lead-pair seat in place one effort level down when its own account's weekly window judges 80%, and back up once every window that moved it reads under 75% again or has passed its recorded reset, with no long window of that account at 80%; the moved seat is told so in one `⟦ae:ctx⟧` turn, and `effort-applied`, `effort-restored` or `effort-failed` lands in the activity dialog. A seat that is busy, has a draft, a human prompt, an unanswered request, a background task shell (a session-leading shell under the tool; an idle wrapper shell does not hold) or recent client input is not restarted until it is idle again; `effort-held` names a lasting reason once — `model-drift` when the pane draws a model other than the one its profile pins, which ae never restarts over. An `effort-failed` spends the episode: ae makes no further attempt until the account recovers. One whose summary says `outcome unproven` lost its record on the way back up, so ae cannot say whether that restart happened; the seat should already be on its profile's effort, as its status line shows. A resume that refuses with `an effort change is unresolved` met a quota-driven effort move ae recorded but never proved finished. Set the global `[workspace] auto_effort = off` (a project `.ae/config` does not count) and resume again: the record is dropped, `effort-discarded` is journaled, and the seat starts at its profile's effort. This is a rollback, not a live change — a running tool keeps the effort it has until it is resumed. Turning the switch back on adopts nothing. A resume that says `resuming at effort <level>` is replaying a move ae still owns; the same switch drops it.

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

`stop`, `end` and `reboot` never use this reasoning. They are irreversible, so
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

## A dead pane's launch line fails after upgrading (`Unknown command: ... ae-core`)

A seat's tool exited, and the shell-history line from before the upgrade names a `versions/<V>/ae-core` the publish has since pruned.
(Other shells word it differently — `No such file or directory` on bash — for the same missing file.)
From another seat of the same session, `<session dir>/relaunch <agent>` does this for you:
it proves the seat dead, restores its recorded working directory and pastes a line that names
the command link. In the dead pane itself, run `~/.local/bin/ae _run <session dir> <slot>`
through the command link instead, with the same session directory and slot the old line carried.
Sessions launched after this fix need no repair: their lines name the link already.

## Session feels stuck

```bash
~/.ae/sessions/<name>/interrupt <agent>            # soft cancel
~/.ae/sessions/<name>/interrupt <agent> "do X"     # cancel + redirect
ae end -f <name>                                    # nuclear option
```

## `send` reports REFUSED, ABANDONED, or UNCONFIRMED

`send` (and `ask` / `review` / `reply` / `interrupt`) report loudly rather than dropping a message. The stderr line names the guard that fired:

- **`send to <target> REFUSED — target pane is a shell, not a running agent`** — the target agent has exited and its pane fell back to a shell. Nothing was pasted (a stray Enter would run your message as a shell command). Relaunch the agent (`relaunch <target>`, from another seat of the same session), then re-send.
- **`send to <target> ABANDONED — target stayed busy (<evidence>)`** — for ~2s the target's input box stayed non-empty or unreadable, or a human was active on the pane. The parenthesis names what held and what ae saw (`composer occupied: 13 content cells on 1 row`, `composer unreadable: no live prompt row`, `human input or attention on the pane: client active 2s ago`), never the text itself. Nothing was pasted, to avoid clipping that input. Wait, then re-send.
- **A trust prompt was named, not answered, with `auto_trust` on** — `auto_trust` answers only the
  measured claude/agy folder-trust block for the seat's own recorded folder, once per tool start; a
  folder or seat it cannot prove, or a start already answered, is only named. An answer refused before
  its reservation says why on stderr (`ae: watchdog: <agent>: auto_trust did not answer the trust
  prompt: <why>`, or a spawn's `ae: auto_trust did not answer …`). One stopped after it is
  `auto-trust-failed` in `events.jsonl`, naming the step, and that start is spent: answer it in the
  pane. A busy pane, or a client's input on it in the last 4 s, holds before the reservation, spends
  nothing and is retried. A damaged or unreadable journal line disables it for that session until
  repaired. A launch with no watchdog pastes no turn to claude or agy, so nothing answers there.
- **A codex seat shows `Warnings · 1 of 1 · Startup` and messages to it are refused** — codex's
  info panel replaces its input box (F2, or a click on its `⚠ N warning · f2 to view` footer). With
  `auto_trust` on, ae presses ONE Escape per panel episode once it has been named for 60 s with no
  input on the pane (`auto-dismiss` in `events.jsonl`); a refused or failed Escape
  (`auto-dismiss-failed`) spends that episode, so press Esc (dismiss) or k (keep) in the pane.
  An unmeasured body is only named. Optional: `--no-daemon` in a codex profile (codex >= 0.156.1;
  older codex refuses the flag) removes the startup warning behind the footer, not the panel.
- **`<verb> to <target> ABANDONED — … human-only prompt on the pane: <question>`** — the target's tool is showing a prompt only you may answer, of a shape ae has measured (claude folder trust, agy trust, codex's Warnings panel); a delivery never answers it (only `auto_trust` may, for its own measured trust block or the codex panel, above), so answer it in the pane, then re-send. Found BEFORE ae's first key (`target stayed busy (human-only prompt on the pane: …)`), nothing was typed and ae refused at once, without the usual wait. Found only AFTER an `interrupt`'s Escape (`Escape sent, message withheld (human-only prompt on the pane: …)`), the Escape was already sent, so the running turn may have been cancelled, and the message was NOT pasted. A tool with no measured prompt shape is unmodelled: ae cannot see its prompts and does not refuse there. A measured tool's pane ae cannot capture refuses the same way, as `composer unreadable: no capture` — check the pane, then re-send. A bare `interrupt` takes the same target lock as every delivery, so it can wait up to 2 minutes behind a send still waiting on a busy pane, and past that is ABANDONED with the target-lock line.
- **`ae list` shows `unreachable (no delivery)` and `attn:unreachable`** — the watchdog's done or wait challenges to that seat stopped landing (`undelivered_max` attempts: the pane was occupied, a prompt or panel was drawn, or its input box could not be read). ONE alert stands until the seat records something itself, a delivery to it lands, or its tool restarts; meanwhile ae retries only when the input box is drawn, empty and idle. `peek` the seat, clear what holds its pane, then send it a message.
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

`tmux set-option @ae_agent` failed for that pane. Refresh the session (`ae doctor --refresh <name>`) — it rewrites pane labels and tags, re-links the helpers, and re-renders `workspace.md`. If the pane is still there but its agent exited, `relaunch <agent>` brings that one seat back in place. If the pane is missing entirely, that's a different problem; `peek <agent>` shows what it printed on the way out, and only `ae stop` plus a resume rebuilds a pane.

## Status-line clicks do nothing

The strip draws, but clicking a session, the picker glyph or the gear does nothing.
The click and picker-hotkey bindings live in the tmux server's key table, and a
`source-file ~/.tmux.conf`, an `unbind-key -a`, or another tool rewriting the root
table removes them silently. `ae doctor` names the missing key on its `tmux.bindings`
row — one row per server, so a server recorded by name in one session and by
socket in another is reported once. `ae <session>` or `ae upgrade` reasserts the map.
`is bound by another ae: <word>` means a different ae (another install or a
checkout) asserted the map last — reassert to take it back; `a foreign command`
means no ae at all bound that key.

## A link in `ae app` does not open on click

A plain click never opens a link: `ae app` and tmux hold the mouse, so the click
goes to ae, which opens no URL. On a web link it toggles the turn mark and, on
release, shows `Shift+Cmd+click to open link` (macOS) or
`Ctrl+click or Shift+click to open link` on the hint row. Hold the modifier your
terminal reserves for links (Ghostty on macOS Shift+Cmd+click, Ubuntu's default
terminal Ctrl+click; the table under [Links](app.md#links) has the rest). A file
link in a turn opens ae's reader on a plain click instead. If the modifier click
does nothing either, the terminal never got the link: Terminal.app has no
hyperlinks, and inside tmux `terminal-features` must declare them
(see [Links](app.md#links)).

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

The `install` bootstrap runs under macOS's stock Bash 3.2; it needs `curl`, `tar` and
`sha256sum` or `shasum`, which macOS ships. `ae` itself is the Rust core and needs only tmux
(3.4 or newer) and git; session helpers are symlinks to that same binary and do not invoke
Bash.

Resume / session-id capture for external CLIs still depends on each tool's local
storage format, which differs across platforms.

## The test suite loads the machine or leaves files behind

The integration suite starts real processes — tmux servers, fake agents, the product — and an
on-access scanner (Microsoft Defender, macOS `syspolicyd` and Spotlight `mds`) inspects every
exec and every scratch file. Where policy allows, point `AE_TEST_TMPDIR` at a dedicated short
absolute directory that exists, and exclude only that one. Lower the process count for one run
with `NEXTEST_TEST_THREADS=4 just test`.

A lane that fails with `error: kept …` left a scratch root or a tmux server that outlived it;
the next `just test` retries the reap, which succeeds once the process holding it has exited.
A `note: kept …` line is the softer case: the sweep could not delete a root (a child was
still writing into it) and left it for the next lane — the lane's own verdict was unaffected.
Most roots from before the single scratch owner (`/tmp/ae*.<pid>.<tag>` and
`tmux -S /tmp/ae…/sock` servers) are never swept: remove them once, with no test running.

## Where to look when things break

In priority order:

1. **`events.jsonl`** — the durable audit trail.
2. **`peek <agent>`** — see what the agent itself thinks happened.
3. **`peek _watchdog`** — watchdog's decision log.
4. **`meta`** — session metadata.
5. **`workspace.md`** — manifest agents are pointed at.

Almost every behavior in ae is observable from those five files.

## Filing a bug

A report without environment facts stalls on the first question, so gather these
before opening an issue (the [bug report form](https://github.com/clemens33/ae/issues/new?template=bug_report.yml)
asks for the same list):

- What happened, what you expected, and steps to reproduce — without a repro the fix is a guess.
- `ae version` output — pins the exact core the behaviour came from.
- `ae doctor` output — the environment checklist plus reboot evidence; the first thing a maintainer asks for.
- Platform (macOS Apple Silicon/Intel, Linux, WSL2, other) and OS version — WSL2 vs bare Linux decides whole failure classes.
- `tmux -V` — the floor is 3.4, and 3.4 vs 3.5+ splits mouse handling.
- Terminal emulator + version and `$TERM` / `$TERM_PROGRAM` / `$COLORTERM` — synchronized output and colour support decide flicker bugs.
- Locale (`locale`, UTF-8 or not) and terminal font — glyph width and alignment live or die here; required for any rendering, flicker, glyph or alignment report.
- Agent harness(es) in the session and their versions (`claude --version`, `codex --version`, `grok --version`, `muse --version`, `agy --version`, `opencode --version`) — resume and capture behaviour is per tool and per version.
- `[workspace]` look settings in effect: `palette`, `icons`, `theme`, `motion`, `layout` (or "defaults") — the status line you see is drawn from these.
- Session shape: single agent, lead-pair, or with workers, plus the profiles used — routing and delivery bugs depend on it.
- Relevant `~/.ae/config` lines and logs if they add anything — and redact first: never paste tokens, `auth.json` contents, credentials, or transcript text; strip private `config_home` paths.

One snippet prints most of it on macOS, Linux and WSL alike:

```bash
uname -srm; tmux -V; ae version; ae doctor
locale; printf '%s\n' "$TERM" "$TERM_PROGRAM" "$COLORTERM"
grep -qi microsoft /proc/version 2>/dev/null && echo WSL2
sw_vers -productVersion 2>/dev/null
. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME"
# redact tokens, credentials and transcript text before pasting anywhere
```
