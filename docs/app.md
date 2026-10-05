# The app

`ae app [session]` draws the fleet as a terminal UI: every session in a
sidebar, the selected session's Overview or Agents beside it, and the selected
session's chat on the right. It is a second view of the data [the chat](chat.md)
already reads, through the same owners, and it writes nothing into any session
but the asks you type at home. `ae chat` is unchanged beside it.

It needs a terminal on both stdin and stdout. Without one it prints
`ae app draws a terminal UI; use ae chat to print the lane` on stderr and exits 1.

## Home

The **home** session is the one you can type into: the `session` argument, else
the session whose pane runs the app. Outside any session and with no argument
there is no home; every view is read-only and the composer says
`ae app <session> picks one`. A named session ae cannot find refuses, exit 1.

`[workspace] chat = app` makes a new or resumed session's `chat` window run
`ae app <session>` instead of `ae chat`, so `prefix h` opens the app. `on` and
`off` keep their meaning; the key is read as `chat` always was, the session's
recorded config first.

## Layout

- **Sidebar** (44 cells from 140 wide, 34 from 90x20): the session count, what
  needs you, then one row per session — its mark, its number, its name
  (`this window` on home), the seat counts on the right, and under it the goal
  or the question that needs you. Running sessions come in fleet order with the
  orchestrator first, stopped ones after. A needy row carries a `│` edge and `!`
  reaches it.
- **Tabs** under the list: **Overview** (goal, what waits on you, the latest
  decision memo, the topics) and **Agents** (every seat with its client,
  profile, model and state, from the watchdog's published roster). A session
  on another tmux server, or one whose watchdog publishes nothing, names that
  gap instead of seats. The tab stays as you move between sessions.
- **Chat** column: the selected session's lane, newest at the bottom, with its
  coverage rows, then the composer. Only the rows on screen and a page beyond
  are drawn: older turns are drawn as you scroll back to them, and the oldest
  stops the scroll.

Below 90x20 the sidebar goes and the chat takes the pane; below 40x8 the app
only says how large it needs to be. Colours come from the session's own look
(home first, then the selected session, then the default palette); with
`theme = off` it draws no colour, and the selected name is bold and reversed.

## Reading

A background reader does every read — the fleet, each session's lane, the
looks, the memos, who owns the input — so no key waits on one. It reads the
fleet first, then a lane the composer just wrote into, the selected session,
the fleet again every 5 seconds, home, the sessions one and then two rows from
the selection, and the rest in sidebar order, keeping at most 16 lanes; the
selected session and home are read again every 5 seconds. Until the first
fleet read the list says `loading`. A session whose lane was not read yet
shows one dim `loading` row, and one read before shows its last lane at once.
An answer that lands after you moved on is kept for its own session and never
drawn under another.

## Keys

Browsing:

| Key | Does |
|---|---|
| `1`-`9` | select that session |
| `j` / `k`, `↓` / `↑` | next / previous session |
| `!` | the next session that needs you |
| `Tab` | Overview / Agents |
| `PgUp` / `PgDn` | scroll the chat a page |
| Click session row | select that session |
| Click Overview / Agents | show that tab |
| Wheel over chat | scroll three rows per notch, also while writing |
| Click composer | write (home only, when this app owns the input) |
| `Esc` | back to home |
| `Enter` or `i` | write (home only, when this app owns the input) |
| `q`, `^C` | quit |

A paste while browsing is swallowed whole, never read as keys.

Writing: the draft is the chat's own input — a line asks the speaker,
`@<seat> text` either lead-pair seat, `/close` withdraws your newest open ask,
five asks open at most; see [Input](chat.md#input). `Enter` sends, `Esc` goes
back to browsing and keeps the draft, `^C` quits. `/open` is refused: the app
selects no pane, `ae chat` does. Each outcome shows in the home lane as an `ae`
line.

While writing, clicking a session row or tab returns to browsing and keeps the
draft, then selects the row or tab. Blank rows and the "more" row do nothing.

## Ownership

Only the **owner** writes: the first live pane stamped as this session's chat,
exactly as for `ae chat`. An app in its own `chat` window (`chat = app`) is
that pane; an app anywhere else is read-only and names why, for example
`read-only · input owned by window @2 - prefix h opens it`. Keys typed
before the app became owner are dropped. A foreign session is always
read-only, and typing still goes home: the composer says
`typing writes to <home> › <speaker>`.

## Residuals

- A SIGKILL or SIGTERM leaves the terminal in raw mode on the alternate screen
  (`reset` restores it). In the `chat` window `remain-on-exit` keeps the dead
  pane on screen and `prefix h` respawns it; any other tmux pane closes with
  the process. Every exit ae controls, a panic included, puts the terminal back.
- A lone `Esc` waits 50 ms to tell itself from an escape sequence.
- `^Z` and `^\` are dropped and never suspend or quit.
- The panic hook that restores the terminal stays installed after the app
  leaves its screen; it would only restore the same mode again.
- Truecolour only; no 256-colour fallback.
- A session replaced under the same name can show its previous lane until the
  next fleet read notices.
- One very long turn is wrapped whole whenever any of its rows is on screen.
- Mouse dragging does not select text in the app. Hold Shift (Option in iTerm)
  for terminal text selection; inside tmux an ordinary drag goes to the app.
