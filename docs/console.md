# The console

`ae console [session] [--follow] [--all]` is the human's lane of one session
as text: what the lead pair said to you and what it needs from you. It is a
labelled **preview of data ae already keeps**, every field passing the board's
terminal renderer; only its [input](#input) writes an event.

## What it shows

- **Pane turns** of both lead-pair seats, from their transcripts, as
  `<seat> pane (transcript)` (` · prior n` for a recorded predecessor).
  Workers are not mirrored; an ae-injected turn never renders as one.
- **Chat-bridge asks and replies** (`telegram:` / `discord:`), every thread
  with its target named. A reply is the journal's 600-character summary,
  tagged `preview (600-char summary)`.
- **Console asks** as `you → <seat>`, each answer whole from the reply's stored
  body (at most 65536 bytes), and each `/close` as `you closed <id>`. An answer
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
- `--all` adds the lead pair's assistant replies.

## Keys

`prefix h` opens (or returns from) a `console` window running
`ae console <session> --follow`; `prefix H` jumps to the lead pane. The window
is stamped `@ae_console` (the session uuid), never `@ae_agent`, so the watchdog
reads no agent in it. `remain-on-exit` keeps a console that stopped following
(a renamed or replaced session) on screen with its reopen hint; `prefix h`
respawns it. Residual: a card can outlive input typed in its pane, because
only the watchdog daemon ends a wait on pane input.

## Input

The window runs `--input` behind a fixed `stty` wrapper; only the owner console
(first live stamped pane) takes keys, another is read-only. A line asks the
speaker, `@<seat> text` either lead-pair seat, `/close <id>` withdraws an open ask,
other `/word`s are refused, five asks open at most. A lead pair changed since
opening is refused: `C-c`, then `prefix h`, restarts it. No terminal: `input off`.

The speaker is the seat of the last `@<seat> text` line that asked (`@<main> text`
included); it is the prompt (`to <speaker>> `) and where an unprefixed line goes.
A refused line, `/close` and a demotion change nothing; a restarted console
speaks to the main seat (the speaker is process memory only).

The composer shows the whole draft wrapped at the pane's width: the prompt on
the first row, the rest indented under it, each pasted line break its own row.
It grows to ten rows (fewer in a short pane) and elides the top as `… +N lines
above`; the last row is always the cursor's. Lane output prints above it — ae
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

When a console becomes the owner it puts a kept draft back in the composer
literally — a line kept before a crash or an unconfirmed ask — under `Kept line,
maybe already sent: check <every lead-pair seat> panes (prefix H) before Enter`.
Nothing in it is read as a command until you press Enter; the file stays until an
ask is confirmed (`^U` clears the composer, not the file, so a later promotion
shows it again). A draft over 65536 bytes, a non-file or an unreadable one is
refused by name and not restored. A read-only console restores nothing.
Residual: the draft keeps only the bytes you typed, not who they were for, so a
restored line without an `@<seat>` prefix goes to the speaker at that Enter — the
main seat after a restart — even if it was first asked of the other seat; the
banner names every lead-pair seat for that reason.

Both lead-pair seats — never a worker — are told in their context that a turn
whose first line is `⟦ae:msg from console:local⟧` is the human's words, to be
answered once with the reply command it carries, and that text typed in their
pane is mirrored but not threaded, so what the human must see goes out by `say`.
