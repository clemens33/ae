# The chat

`ae chat [session] [--follow] [--all]` is the human's lane of one session
as text: what the lead pair said to you and what it needs from you. It is a
labelled **preview of data ae already keeps**, every field passing the board's
terminal renderer; only its [input](#input) writes an event.

A new or resumed session opens it as its **first window**, `chat`, after the
seats, and attaching lands there (`[workspace] chat = off` keeps the old
layout). Running sessions are not changed by an upgrade; `prefix h` gives them
the window on demand. See [Keys](#keys).
`ae app` draws the same lane beside the whole fleet as a terminal UI; see [App](app.md).

## What it shows

- **Pane turns** of both lead-pair seats, from their transcripts, as
  `<seat> pane (transcript)` (` · prior n` for a recorded predecessor).
  Workers are not mirrored; an ae-injected turn never renders as one.
- **Replies to your pane lines**: the lead pair's assistant text, as
  `<seat> assistant (transcript)`, only while it answers a line you typed in
  that pane: from your line up to the next user turn of any kind (an agent
  message, a chat ask, a watchdog challenge, launch context), so an answer
  to an agent never shows. Claude, Codex, Muse and Grok seats give replies;
  OpenCode and Antigravity seats give none by default (an OpenCode export
  carries no proven order, Antigravity keeps prompts and replies in separate
  stores). A transcript line the reader cannot classify closes the window, and
  a Claude turn that carries an image hides its reply. Text parts only; tool
  calls and thinking never show.
- **Chat-bridge asks and replies** (`telegram:` / `discord:`), every thread
  with its target named. A reply is the journal's 600-character summary,
  tagged `preview (600-char summary)`. The Telegram bridge writes these no
  more: a phone message is a chat ask, below, admitted and routed exactly as
  one typed here.
- **Chat asks** as `you → <seat>`, each ask and each answer whole from its
  stored body (at most 65536 bytes, read only from the session's own
  `messages/`; an ask's file is the delivered message, and its text is what
  that envelope wraps), and each `/close` as `you closed an ask`. A body that
  cannot be read shows the journal's 600-character summary and names why in
  the header and a coverage row (`full text not shown: stored message over 64
  KiB`, `body missing`, `body refused: …`); an ask an older core recorded has
  no stored text and names nothing. An ask that went through says so in its
  header, `you → <seat> · sent`, not on a line of its own. No row
  draws a request id: the journal keeps it, the chat pairs by it. An answer
  counts only from the slot, session, server, pane and session uuid the ask
  reached; any other reply shows as `not admitted: stale|unproven <field>`.
  Answers after a reseat of the asked seat add `speaker <seat> now <profile>`,
  the profile the roster records NOW: a move back to the ask-time profile still
  labels, and a move whose start failed before any record, finished by
  `relaunch`, does not.
- **`say` lines** as `said <seat>` (or `ae` for ae's own notices); a worker's
  is counted in a coverage row, never shown.
- **Decision cards**: a lead-pair seat's CURRENT `waiting-user` declaration as
  `DECISION <seat>`, the watchdog's `human-prompt` as `NEEDS YOU <seat>`.
  In chat and `ae app` a card's reason is laid out for reading, every word kept
  and in its order: the question on its own lines, each ` | `-separated option
  on its own line behind its `1 `/`a)` label or a bullet (`•`, `-` when icons
  are off or on a plain pipe) with wrapped rows hanging under its words, the
  prose after it one sentence per line, and a trailing evidence pointer
  (`.local/…`, `details:`, `Evidence:`, `memo <topic>`, a bare `file:line`)
  last, dim where the look is drawn. One empty row separates those blocks. A
  reason with no ` | ` is split into sentences the same way.
  `--follow` prints `-- closed: …` when one stops standing.
- **Coverage rows** for anything it cannot read: a torn or absent transcript,
  an unsupported harness, an unreadable journal, skipped journal lines.
- `--all` shows every assistant reply of the lead pair, answers to agents
  included, and reads OpenCode and Antigravity replies too.
- **Needs you**, after the lane: see [below](#needs-you).

## Long turns

A turn of more than thirty rows, as the pane draws them, shows its first 24 and a
fold line, `... N more lines (/more n)`; thirty rows or fewer show whole. Only an
input-taking chat whose input AND output are terminals folds: it counts rows by
asking tmux for its pane's width (plain text too, wrapped like the dressed look),
while a pipe, a file, a redirected `--follow --input`, a one-shot print and
`--follow` without a terminal print every turn whole. A fold must save at least one
rendered row: it folds only when the 24 shown rows plus the rows its own line
takes (the line wraps in a narrow pane) are fewer than the turn's rows, else the
turn shows whole; at normal widths that is still every turn of more than thirty rows. `/more n`
prints turn `n` whole, header and every row; it writes no event, is
repeatable, and works in a read-only chat too. A handle names one fold of this
chat: a number it never drew, a turn a rewritten journal has since moved, or one no
longer in the read is refused by name. The rows come through the same
neutraliser as every other row; `/more` takes ASCII digits only (`/more` alone or
`/more x` is refused as `/more takes one fold number`). A decision card folds after its
options are split onto their own lines.

## Needs you

The conversation stays the lead pair's, but the section that follows it lists
EVERY roster seat, workers included, whose attention verdict says you are
needed. The verdict is the one `ae list` reads, kept to the reasons that need you
(dead, `waiting-user`, `blocked`, an escalated `waiting-agent`, a usage limit, a
throttle); a stale seat and unanswered asks are not listed, and the chat derives none
of its own. A row reads

```
  scout · blocked · /open scout · human prompt since 17:50:00 (10m)
    Trust this folder?
```

the seat (`(lead pair)` marks the pair), its reason, the `/open` that shows
its pane, where the verdict comes from (`declared`, the alert's action, `no pane
carries <slot>`, or `source unattributed`) with its time and age, and the
declaration's reason or the alert's words on a line of their own. Rows sort by
the reason's rank as `ae list` ranks it, then oldest first.

Doubt is shown, never hidden. A verdict standing on a fact ae cannot trust adds
`· stale: <cause>`; a seat with no verdict but such a fact is counted under one
line per cause, `unverified: <cause> · <n> seats: <names>`. The causes, in
precedence: `watchdog off` (no beat), `watchdog beat unreadable`, `watchdog
silent since <time>` (no beat for three default verdict intervals, 180 s),
`tmux did not list the panes`, `pane unproven`, `journal partial, <n> lines
unread`. Residual: a watchdog started with a custom `--interval` is judged by
the default, because the override is not recorded.

The header counts the seats with a real verdict and says when it was read,
`-- needs you: 2 seats · as of 18:02:11`; the `unverified` lines carry their
own counts and never join the header's number. In a pane the section takes at
most a third of the height; without a pane size (a pipe, `theme = off`) it
shows twelve lines at most. The `unverified` lines get that room first, then
the seat rows in rank order; a row short of room keeps its seat line and drops
its detail, and the header says `<k> more: ae list` for the seats it left out.
Each row is clipped to the pane's width when the chat knows it. The section
prints again only when what you must act on changes: which seats have a real
verdict, a seat's verdict, or its source. Ages, since stamps, last-beat
stamps, proof words and record positions never replay it. A newly doubted
cause prints one dim notice naming the seats, `stale: watchdog silent · 2
seats: lead, idle`; recovery prints nothing. A need that clears prints `--
needs you: nothing standing (as of …)` once, an unreadable meta or journal,
or a damaged meta that may have lost a seat, prints one warning that keeps
the rows shown earlier standing, and a first read with nothing standing
prints nothing. After the host slept — a wall-clock gap past 60 s between
reads — the watchdog gets 180 s of awake time before its silence is doubt
again; a missing or unreadable beat is never graced.

## Look

On a terminal the chat is drawn like a group chat. Each speaker keeps one
colour from the session's own palette: you, the lead seat, the other lead-pair
seat, and ae itself (`said ae`, coverage rows, `-- closed:` and the rewrite
notice, all dim). A bar (`▌`, or `|` when `icons` is off) runs down the left of
every message, its header in the speaker's colour after a dim time; headers of
`you → <seat>` are bold, and a sent ask's header ends in a dim `· sent`. Status
lines are green (`sent`, only for an ask whose row never shows), amber
(`uncertain`, `no record of`, `the draft is kept`) or red (`not delivered`,
`refused`), and the input prompt wears your colour. Times and the day divider
use the viewer's zone: the chat asks
the session's tmux once for `#{t/f/%z:start_time}` and keeps only a `[+-]HHMM`
answer (`# 2026-10-03 +0200`); any other answer, or a tmux without that
modifier, keeps `UTC`. A zone change mid-session (daylight saving) is not
followed.

The chat wraps what it prints to its pane's width. Each pass it asks tmux for
`#{pane_width}` of its own pane; with a known width, every header, body and
status row breaks at the last space that fits (else between characters), and
every row it makes starts with the entry's bar in the speaker's colour, after
the same indent (a line's own leading spaces, tabs spent as spaces to the next
stop of eight, give way first in a narrow pane). Width is counted
conservatively, ASCII one cell, a tab eight, any other character two, so a row
never overflows and accented Latin may wrap early. When tmux gives no width (no
tmux pane, no answer) the chat stays dressed but unwrapped and the terminal
soft-wraps it, the continuation rows without a bar. Rows already printed are
never reflowed when the pane is resized; the next pass wraps new rows at the
new width. Only a pane narrower than 3 cells can overflow, a row then holding
the bar and one character.

Colour comes only from ae's own printing: every record byte is neutralised
before an escape is added, and no escape is written inside a message except
the link pairs of [Links](#links). When
stdout is a pipe or a file, the output is exactly the plain text above, in
UTC, and the chat asks tmux for none of look, zone or pane width; with
`[workspace] theme = off` on a terminal it reads the look, finds it undrawn, and
prints the same plain text without asking for the zone or the width. The colours are
the palette's accents drawn on the terminal's own background, which ae cannot
see; against the palette's `base` every hue clears 3.0:1, and on Darcula the
`working`, `stale`, `dim` and `done` hues stay under the 4.5:1 text bar (`dim`
also on palettes `a` and `b`).

## Links

A web URL (`http://` or `https://`) or a repo-style path in a turn is a real
terminal hyperlink: Cmd-click (Ctrl-click on Linux) opens it the way the
terminal opens any link, and the text is underlined while a look is drawn. A
path is one of

- `src/…`, `docs/…`, `tests/…` or `.local/…`, against the work dir the session's
  meta records,
- `~/…`, against your home,
- an absolute `/…` with at least two segments (`/open` and `/close` are
  commands, not paths),

with an optional `:line` or `:line:col` that stays in the text and out of the
link; the terminal opens the file, not the line. A relative path in a session
whose meta records no work dir stays text. Nothing else links: not a bare word,
a `memo` topic or a request id. The link is `file://` with an empty host and the
path percent-encoded; ae does not check that the file exists, does not
normalise `..`, and never links a path with a space in it.

Only the chat's text is linked, and only on a terminal whose look is drawn.
Piped, redirected or `theme = off` output is the plain text, byte for byte. The
visible text, the wrapping and the fold counts are the same with or without a
link, and a link a wrap cuts is two links with one id, so a terminal that
lights a link under the mouse lights both rows. Every body is neutralised
before it is scanned, and a URL is written only as printable ASCII with
anything else percent-encoded, so no record can end a link early, open a
second one or smuggle a control into it.

OSC 8 must reach your terminal. Terminal.app has no hyperlinks. Inside tmux
(3.4 and later, ae's floor) tmux forwards a link only when its
`terminal-features` say your terminal takes them; for most terminals add

```
set -as terminal-features ',xterm*:hyperlinks'
```

to `~/.tmux.conf` (the glob must match your `$TERM`). ae never writes that
server option. Where the link is not forwarded the text is shown unchanged.
Following a link inside ae (a reader pane, memos, request threads) is not part
of this.

## Keys

`prefix h` opens (or returns from) a `chat` window running
`ae chat <session> --follow`; `prefix H` jumps to the lead pane. The window
is stamped `@ae_console` (the session uuid), never `@ae_agent`, so the watchdog
reads no agent in it. `remain-on-exit` keeps a chat that stopped following
(a replaced session) on screen with its reopen hint; `prefix h` respawns it.
A launch opens the window itself, at the session's first index. Attaching or
switching to the session lands on that chat while it is live there, and on
the lead pane once it has exited or is gone. `ae rename` restarts every chat
under the new name; one it cannot restart is closed, and the rename exits 1.
A launch whose chat cannot open still starts, says so once and journals
`chat-window-failed`. Residual: a card can outlive input typed in its pane, because
only the watchdog daemon ends a wait on pane input.

## Input

The window runs `--input` behind a fixed `stty` wrapper, which sets `-icrnl` so Enter reads as CR and `^J` as LF. Outside that wrapper (a cooked terminal, ICRNL on) Enter arrives as LF, a newline: `ae chat --input` there never sends, so use the chat window or `ae app`. Only the owner chat
(first live stamped pane) asks, another is read-only and takes only `/open` and `/more`. A line asks the
speaker, `@<seat> text` either lead-pair seat, `/close` withdraws your newest open ask,
`/close <id>` a named one (ids are not drawn, a script may still name one) and
prints `closed <id>: <the ask's first words>`, other `/word`s are refused, five asks
open at most. A refused line (a cap, a refused command, a read-only chat) stays in the
composer, ahead of anything typed since. A lead pair changed since
opening is refused: `C-c`, then `prefix h`, restarts it. No terminal: `input off`.
While an [`ae app`](app.md#ownership) is sending to the session (or an older
core's app writes to it), a send or `/close`
is refused at once with `an ae app is writing to <session> (held at <session>:<window>)`, or `(holder unknown)` when the app left no record that reads, nothing kept or sent:
the owner chat tries the app's writer lease inside its admission and drops it.
`/open <seat>` selects any roster seat's pane, see [Open](#open).
Recorded asks show their header (`· sent` once it went through) and body before
any other submit result. If the row stays missing for two readable passes, the
result prints on a line of its own;
unknown results and refusals print immediately.

The speaker is the seat of the last `@<seat> text` line that asked (`@<main> text`
included); it is the prompt (`to <speaker>> `) and where an unprefixed line goes.
A refused line, `/close` and a demotion change nothing about the speaker; a restarted chat
speaks to the main seat (the speaker is process memory only).

The composer shows the whole draft wrapped at the pane's width: the prompt on
the first row, the rest indented under it, each pasted line break its own row.
Rows break at the last space that fits; a word longer than a row splits by
character. Wrapping changes only the display, never the bytes Enter sends.
It grows to ten rows (fewer in a short pane); a longer draft shows the window
that holds the cursor's row, with the rows cut off named as `… +N lines above`
and `… +N lines below`, markers included in the ten. Lane output prints above it — ae
takes away exactly the composer's rows, prints, and draws it again — and a resize
is honoured at the next paint. The size comes from tmux (80x24 when it will not
say); every non-ASCII character counts as two cells, so a row breaks early, never
late, and no row is wider than the pane. Residual: after the pane narrows ae finds
the composer's top on the screen by its prompt. When the composer was drawn in six
columns or fewer, or the draft itself spells the prompt where tmux split a row,
ae may erase too few rows and leave stale composer rows until they scroll away;
it never erases lane output, never draws a row wider than the pane and never
touches the draft.

Keys edit at a cursor, which the terminal cursor shows where the next character
goes: Left and Right move one character (a byte that is not UTF-8 is its own),
Home and End — also `^A` and `^E` — go to the start or end of the whole draft,
not of a row, Delete and Backspace erase the character at or before it, and
typed or pasted text goes in at it byte for byte, even where it completes a
character with the bytes after it; the next arrow or erase key then acts from
the end of that character. A cursor before a line break at the end of a full
row is drawn on that row's last cell. `^U` clears the whole draft, wherever the
cursor is, `^W` erases the word before the cursor (back to whitespace), `Alt+Backspace` (`ESC DEL`, Option as Meta) the one before it up to a non-letter, non-digit, and `^K` the rest of the cursor's line (its line break when nothing is left on it); `^Y` puts what `^U`, `^W`, `Alt+Backspace` or `^K` last took back at the cursor, once (the newest erase wins; a send ends it). Up, Down, Insert, PgUp, PgDn and modified navigation keys are
consumed and do nothing; `^V` reads a clipboard image; any other plain control byte is a literal draft byte.
`^V` in the owner chat inserts the clipboard image as an attachment token at the
cursor, with spaces where needed. The draft shows it as `[Image #N]` (N counts on
from the highest image the draft holds); it is one unit: the cursor never sits
inside it, one Left, Right, Delete or Backspace passes or erases it whole, and
`^U`, `^W`, `Alt+Backspace` and `^K` take it with their text and `^Y` brings it
back. Enter sends it as `[Image #N: "/abs/path"]`, so the seat gets the absolute
path to read; the stored ask keeps that path, and the chat's history
shows the label `[Image #N]` only. Typing `[Image #1]` yourself is plain text, and
so is a pasted or typed path. A byte `0xFF` from outside the draft becomes U+FFFD.
Images live at 0600 under the session's `messages/`; one erased from the draft
before a send is removed with it (`^U` then `^Y` keeps it), and a sent one stays.
A draft holds at most 32 attachments and the resolved line stays within the cap.
The chat does not turn a pasted file path into a token (the app does, see
[app](app.md)); it stays literal text.
macOS uses osascript; Linux needs wl-paste on Wayland or xclip otherwise.
Only PNG up to 20 MiB is accepted. A failed or eight-second read prints its
refusal and keeps the draft. Typing continues during a read; another `^V` is ignored.
Claude seats can read images by path (verified); other clients are unverified.
See [image paste #228](https://github.com/clemens33/ae/issues/228).
Inside a bracketed paste every byte stays literal, an Enter included, except that a
CR or CRLF becomes one line break (LF). Enter sends; `^J` and Alt+Enter put a line
break in the draft instead. Shift+Enter is Enter: through tmux the terminal sends it
as a plain CR, and telling it apart needs the app to request extended keys (which
re-encodes `^C`, `^U`, `^A`, `^E`) and a tmux `extended-keys` option ae does not set.
Residuals: an Esc and an Enter that arrive in one terminal read are read as Alt+Enter,
and a chat window started by an older core has Enter and `^J` as the same byte until
`prefix h` reopens it (the window sets `-icrnl`, so Enter reads as CR and `^J` as LF).

Each writing process keeps its own recovery record, protected by a lifetime
lock even while it browses. A live window's kept line stays in that window.
When writing starts, a line from an earlier process is offered only if its
lifetime lock proves that process gone. Unknown liveness skips the record.
The offer says `kept line from an earlier window; may already have been sent`,
with `Enter restores` and `Esc discards`. Enter fills the composer only; a
second fresh Enter sends. Other keys leave the offer standing. The offer
exclusively claims one line: another window cannot recover it while the
claimant lives, and quitting or crashing leaves it recoverable. Old unstamped
recovery files follow the same offer flow. A restart, including an installed
app following an upgrade, starts a new process identity.

When a chat becomes the owner it puts its own kept draft back in the composer
literally — a line kept after a refusal or an unconfirmed ask — under `Kept line,
maybe already sent: check <every lead-pair seat> panes (prefix H) before Enter`.
Nothing in it is read as a command until you press Enter; the file stays until an
ask is confirmed (`^U` clears the composer, not the file, so a later promotion
shows it again). A draft over 65536 bytes, a non-file or an unreadable one is
refused by name and not restored. A read-only chat restores nothing.
Residual: the draft keeps only the bytes you typed, not who they were for, so a
restored line without an `@<seat>` prefix goes to the speaker at that Enter — the
main seat after a restart — even if it was first asked of the other seat; the
banner names every lead-pair seat for that reason.

## Open

`/open <seat>` selects a seat's pane in this session, from the owner chat or a
read-only one; a read-only chat composes `/open` and `/more` lines and refuses
anything else. It types nothing into the pane and writes no record. ae proves the seat
against the meta and tmux at that moment — the roster still seats that name in
that slot, the session uuid still matches the chat's, exactly one live pane
carries the slot and the name, and the server lists that pane in this session
— then selects its window and pane behind a tmux guard that checks the session
id and uuid and the pane's slot, name and liveness again, so every client
viewing the session follows and none other moves.
`opened <seat> - prefix h returns` on success; `refused: /open <seat>: <why>;
nothing selected` when a proof or the guard fails; `uncertain: …` when tmux
does not confirm. The seats `/open` knows are those of the last settled read of
the meta and journal. A line with no name or more than one word is refused as
`refused: /open takes one agent name`, and another session's seat by name:
`refused: /open <session:agent> names another session; this chat opens its own
seats`. Residual: a pane whose window is also linked into another session is
refused as changed.

Both lead-pair seats — never a worker — are told in their context that a turn
whose first line is `⟦ae:msg from human:chat⟧` is the human's words, to be
answered once with the reply command it carries, and that text typed in their
pane is mirrored but not threaded, so what the human must see goes out by `say`.
The same context tells them how to shape that reply for a glance — answer first
in one short line, short lines, numbered lines for choices, no tables or bold
markers — and a chat ask's footer repeats the guidance in one line.
`human:chat` is display only: the journal still records the ask as
`console:local`, and older records show that spelling for the same route.

## Telegram

The Telegram bridge (`ae telegram`) is one more writer of this chat. A phone
message to a selected session (`/use <session>`, `@session`, or a reply to a
forwarded line) becomes a chat ask exactly as one typed here: the same
admission, the same lead-pair router and the same cap of five open asks. It
never takes an `ae app`'s writer lease. Back to the phone go each running
session's human-facing chat rows, labelled `[session]`: asks typed here, the
answers, and a lead-pair seat's or the watchdog's `say`. Worker traffic, states
and tool output stay local, and a phone message is not echoed back.

Each bridge start fixes a cutoff ten minutes before it started. An older row is
not sent, even behind a valid cursor: it stays in the session journal and its
count goes to `~/.ae/telegram/backlog.log`. An older phone message is not acted
on: it is kept in `~/.ae/telegram/stale-inbound.jsonl`. The chat hears one notice
per direction, two at most. A phone message
replayed after a crash finds its recorded ask by its update's id and is not
asked again. Delivery is at least once, never exactly once. Three windows can
repeat one message: a crash before the first outbound checkpoint can echo a
phone message back; a send Telegram accepted before its cursor was written is
sent again after a restart; and a turn pasted into a seat before its ask was
journaled is asked again.
