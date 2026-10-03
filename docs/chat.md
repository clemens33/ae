# The chat

`ae chat [session] [--follow] [--all]` is the human's lane of one session
as text: what the lead pair said to you and what it needs from you. It is a
labelled **preview of data ae already keeps**, every field passing the board's
terminal renderer; only its [input](#input) writes an event.

A new or resumed session opens it as its **first window**, `chat`, after the
seats, and attaching lands there (`[workspace] chat = off` keeps the old
layout). Running sessions are not changed by an upgrade; `prefix h` gives them
the window on demand. See [Keys](#keys).

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
  tagged `preview (600-char summary)`.
- **Chat asks** as `you → <seat>`, each answer whole from the reply's stored
  body (at most 65536 bytes), and each `/close` as `you closed an ask`. No row
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
  `--follow` prints `-- closed: …` when one stops standing.
- **Coverage rows** for anything it cannot read: a torn or absent transcript,
  an unsupported harness, an unreadable journal, skipped journal lines.
- `--all` shows every assistant reply of the lead pair, answers to agents
  included, and reads OpenCode and Antigravity replies too.

## Look

On a terminal the chat is drawn like a group chat. Each speaker keeps one
colour from the session's own palette: you, the lead seat, the other lead-pair
seat, and ae itself (`said ae`, coverage rows, `-- closed:` and the rewrite
notice, all dim). A bar (`▌`, or `|` when `icons` is off) runs down the left of
every message, its header in the speaker's colour after a dim time; headers of
`you → <seat>` are bold. Status lines are green (`sent`), amber (`uncertain`,
`no record of`) or red (`not delivered`, `refused`), and the input prompt wears
your colour. Times and the day divider use the viewer's zone: the chat asks
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
before an escape is added, and no escape is written inside a message. When
stdout is a pipe or a file, the output is exactly the plain text above, in
UTC, and the chat asks tmux for none of look, zone or pane width; with
`[workspace] theme = off` on a terminal it reads the look, finds it undrawn, and
prints the same plain text without asking for the zone or the width. The colours are
the palette's accents drawn on the terminal's own background, which ae cannot
see; against the palette's `base` every hue clears 3.0:1, and on Darcula the
`working`, `stale`, `dim` and `done` hues stay under the 4.5:1 text bar (`dim`
also on palettes `a` and `b`).

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

The window runs `--input` behind a fixed `stty` wrapper; only the owner chat
(first live stamped pane) takes keys, another is read-only. A line asks the
speaker, `@<seat> text` either lead-pair seat, `/close` withdraws your newest open ask,
`/close <id>` a named one (ids are not drawn, a script may still name one), other
`/word`s are refused, five asks open at most. A lead pair changed since
opening is refused: `C-c`, then `prefix h`, restarts it. No terminal: `input off`.
Recorded asks show their header and body before the submit result. If the row
stays missing for two readable passes, the result prints on a line of its own;
unknown results and refusals print immediately.

The speaker is the seat of the last `@<seat> text` line that asked (`@<main> text`
included); it is the prompt (`to <speaker>> `) and where an unprefixed line goes.
A refused line, `/close` and a demotion change nothing; a restarted chat
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
cursor is. Up, Down, Insert, PgUp, PgDn and modified navigation keys are
consumed and do nothing; any other plain control byte is a literal draft byte.
Inside a bracketed paste every byte stays literal, an Enter included.

When a chat becomes the owner it puts a kept draft back in the composer
literally — a line kept before a crash or an unconfirmed ask — under `Kept line,
maybe already sent: check <every lead-pair seat> panes (prefix H) before Enter`.
Nothing in it is read as a command until you press Enter; the file stays until an
ask is confirmed (`^U` clears the composer, not the file, so a later promotion
shows it again). A draft over 65536 bytes, a non-file or an unreadable one is
refused by name and not restored. A read-only chat restores nothing.
Residual: the draft keeps only the bytes you typed, not who they were for, so a
restored line without an `@<seat>` prefix goes to the speaker at that Enter — the
main seat after a restart — even if it was first asked of the other seat; the
banner names every lead-pair seat for that reason.

Both lead-pair seats — never a worker — are told in their context that a turn
whose first line is `⟦ae:msg from human:chat⟧` is the human's words, to be
answered once with the reply command it carries, and that text typed in their
pane is mirrored but not threaded, so what the human must see goes out by `say`.
The same context tells them how to shape that reply for a glance — answer first
in one short line, short lines, numbered lines for choices, no tables or bold
markers — and a chat ask's footer repeats the guidance in one line.
`human:chat` is display only: the journal still records the ask as
`console:local`, and older records show that spelling for the same route.
