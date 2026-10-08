# The app

`ae app [session]` draws the fleet as a terminal UI: every session in a
sidebar, the selected session's Overview or Agents beside it, and the selected
session's chat on the right. It is a second view of the data [the chat](chat.md)
already reads, through the same owners, and it writes nothing into any session
but the asks you type into the selected one. `ae chat` is unchanged beside it.

It needs a terminal on both stdin and stdout. Without one it prints
`ae app draws a terminal UI; use ae chat to print the lane` on stderr and exits 1.

## Home

The **home** session is the one the app opens on and `Esc` returns to: the
`session` argument, else the session whose pane runs the app. You can type into
whichever session is selected (see [Ownership](#ownership)). Outside any session
and with no argument there is no home and the first session is selected; with
no session at all the composer says `ae app <session> picks one`. A named
session ae cannot find refuses, exit 1.

`[workspace] chat = app` makes a new or resumed session's `chat` window run
`ae app <session>` instead of `ae chat`, so `prefix h` opens the app. `on` and
`off` keep their meaning; the key is read as `chat` always was, the session's
recorded config first.

## Layout

- **Sidebar** (44 cells from 140 wide, 34 from 90x20): the session count, what
  needs you (`Nothing needs you.`, `<name> needs you` for one, `<N> need you`
  for several, then `· above` / `· below` — with a count when several — for
  needy rows outside the list), then one row per session — its mark, its
  number, its name (`this window` on home), the seat counts on the right, and
  under it the goal or the question that needs you. Running sessions come in
  this order: the orchestrator, then the sessions your `[workspace]
  fleet_order` names (the pins), then the rest by your latest activity, newest
  first, and a session with none known after them in creation order; stopped
  ones come last. A needy row carries a `│` edge and `!` reaches it. The wheel moves a list that does not fit one session a notch; a
  selection key brings the selection back into view, a click keeps the list
  where it is.
- **Tabs** under the list: **Overview** (how the session was launched — `mode`,
  `dir` and, for a copy or worktree, `source`, spelled as the session menu
  spells them and `unrecorded` where the record says nothing — then the goal,
  what waits on you, the latest decision memo, the topics) and **Agents**
  (every seat with its client,
  profile, model and state, from the watchdog's published roster). A session
  on another tmux server, or one whose watchdog publishes nothing, names that
  gap instead of seats. The tab stays as you move between sessions. A tab
  body that does not fit gives its last row to the rows it hides,
  `↑ a · ↓ b more rows`, and the wheel moves it three rows a notch; another
  session or tab starts at its first row.
- **Chat** column: the selected session's lane, newest at the bottom, with its
  coverage rows, then the composer. Only the rows on screen and a page beyond
  are drawn: older turns are drawn as you scroll back to them, and the oldest
  stops the scroll. The composer is one row while browsing; while writing it
  grows with the draft to ten rows, taking them from the lane, which keeps at
  least three rows: a short pane shrinks the composer first, to one row at
  least. A resize sizes it again, and leaving writing gives the rows back.

The session list takes two thirds of the rows it shares with the tab body,
and never fewer than the 11 rows (18 from 40 high) it once had, though never
more than the rows it shares (10 at 90x20, which leaves the tab body one row);
it is sized to its sessions when they need fewer, so 24 sessions show 11 at
160x45 where they showed 8. The launch facts come from the same listing the sidebar row comes
from, not from a second read of the session's record. A session replaced
between two reads of its record can therefore show the previous incarnation's
facts for at most one refresh (5 seconds); a selection the listing does not
hold says `launch facts unavailable: session not in this read`.

Both borders can be dragged with the mouse. The rule between the sidebar and
the chat sets the sidebar's width: at least 30 cells, and the chat keeps at
least 40. The rule under the tabs moves the split between the session list and
the tab: the list keeps one session and its more row, the tab three rows (or
as few as it had undragged). The rule being dragged is drawn in the title
colour, reversed with `theme = off`; letting go ends the drag. The sizes last
for this run of the app and are not remembered; a smaller pane narrows them
only while it is small.

Below 90x20 the sidebar goes and the chat takes the pane; below 40x8 the app
only says how large it needs to be. Colours come from the session's own look
(home first, then the selected session, then the default palette); with
`theme = off` it draws no colour, and the selected name is bold and reversed.

### Order

Your activity in a session is the later of two things: the last time a tmux
client attached to it, switched into it or gave it a key or mouse input
(tmux's `session_activity`, counted only once it is later than the session's
creation, so a session nobody has touched stays unknown), and the newest
question you asked it through the chat or the app. Output from agents never
counts, nor do ae's own commands. A new read of the fleet can move a row, and
only when a frame is drawn: a key or click always acts on the row it showed.
The status strip and the picker keep their own order.

## Reading

A background reader does every read — the fleet, each session's lane, the
looks, the memos, whether the session being written is still proven — so no key waits on one. It reads the
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
| Wheel over chat or a tab body | scroll three rows per notch, also while writing |
| Wheel over the session list | scroll one session per notch, also while writing |
| Drag a border | resize the sidebar or the session list, also while writing |
| Click composer | write the selected session, when no other app writes it |
| `Esc` | back to home |
| `Enter` or `i` | write the selected session, when no other app writes it |
| `s` | open Settings |
| Click the gear at the keys row's right end | open Settings, also while writing |
| Click a Settings tab title | show that tab, like `Tab` and `1`-`4` |
| Click `Esc close` at the Settings title row's right end, or the gear | close Settings, like `Esc` |
| `q`, `^C` | quit |

A paste while browsing is swallowed whole, never read as keys.

Writing: the draft is the chat's own input — a line asks the speaker,
`@<seat> text` either lead-pair seat, `/close` withdraws your newest open ask,
five asks open at most; see [Input](chat.md#input). The composer shows the
whole draft as the chat does — the same wrap, each pasted line break its own
row, the rest indented under the first text cell, and past its rows the window
around the cursor with `… +N lines above` and `… +N lines below` among them
(no markers below three rows) — and the terminal cursor sits where the next
character goes. The cursor shows only while writing with the composer drawn:
browsing, a held composer and Settings hide it. Every composer row and its hint
row are one click target. `Enter` sends, `Esc` goes back to browsing and keeps
the draft, `^C` quits. `/open` is refused: the app
selects no pane, `ae chat` does. Each outcome shows as an `ae` line in the lane
of the session it is about; a line about a session no longer recorded shows in
whichever lane is selected.

Writing starts the moment this app takes the session's writer lease. Keys read
before that moment — the rest of the read that started writing included — are
dropped, and one `ae` line says `dropped the keys typed before writing
started`; `Esc` and `^C` are never dropped. While another app writes, or when
the lease's lock file is not a regular file, writing does not start: the
composer reads `not writing: an ae app is writing to <session> · Esc browses` (or
`not writing: the writer lease is not a regular file · Esc browses`) and every
key is swallowed until `Esc` browses, `^C` quits, a click acts or `Enter` tries
again; while the session is not proven, `Enter` stays held. The other app letting go
does not start writing by itself.

While writing, clicking a session row or tab returns to browsing and keeps the
draft, then selects the row or tab. Blank rows and the "more" row do nothing.
A press within a cell of a border grabs it unless a row or tab is drawn there.
Any key, paste, press or wheel notch ends a drag whose release never arrived,
then acts as it always does.

## Settings

`s` opens the read-only Settings overlay over the whole pane, as does the
gear (`*` with `icons = off`) at the keys row's right end — the gear also
while writing, where `s` stays draft text; `Esc`, `q` or `s` closes it, `^C`
quits. While open the overlay owns every key and click: `Tab`, `1`-`4` and a
click on a tab title switch tabs, a click on the drawn `Esc close` label or on
the gear closes it like `Esc`, `j` / `k` and `↓` / `↑` scroll a line, the
wheel three rows per notch, `PgUp` / `PgDn` a page, everything else is
swallowed (every other click too), and a draft being written is kept untouched
until it closes. Four
tabs: Quota shows the same scope rows `ae quota` prints; Config shows every
`[workspace]` key the home session runs
with and where each came from — `launch` (pinned in its meta), `session` (its
origin overlay), `global` (the meta-recorded file, the current global only
when that row is empty) or `default` — with `global only` on the keys that
read the current global alone; About names the versions, the state root, the
config file, the recorded server and the repo links; Instructions shows what ae
tells the agents of the SELECTED session (the one selected when Settings opened),
from its records alone, so a stopped session shows too: the custom `[prompt]
instructions` in force with the file they come from — `global` (the meta-recorded
file) or `session` (its local overlay), the last file declaring the key wins even
when empty — or a line saying there are none; then one section per roster seat,
titled `name · slot · lead` or `worker`, holding the text a launch injects
for that seat, as the one renderer produces it now, wrapped to the pane. A
session, config file or seat ae cannot render from shows a named `gap` row
instead of a guess: no records, an unreadable meta or config (the whole tab), an
unknown slot, a seat name that is no agent name, a refused or missing work dir.
Quota reads again every refresh while open; config, about and instructions read
once per open. A torn config shows one honest row naming the file. The
Instructions head states the residual below: this is today's render, not what
a running seat was handed.

## Ownership

An app writes the selected session only while it holds that session's
**writer lease**, the lock file `.console-writer.lock`: one app at a time per
session, in any pane or outside tmux, and at most one lease per app. No key or
wheel notch moves the selection while writing; a click on a session row leaves
writing first. The composer names its target: `to <session> › <speaker>` at
home, `to <session> (not home) › <speaker>` anywhere else. A stopped session,
one with no recorded `session_id`, one whose lead pair cannot be read and one
not recorded in this state root are read-only and say why.

Every way out of writing releases the lease — `Esc`, `^C`, a crash, a click on
a session row or tab, and a read that finds the session no longer proven — and
Settings keeps it while open. Each ask and `/close` goes through the chat's
admission, which re-proves the lease and re-reads the meta's `session_id` and
lead pair against those the entry proved, then asks as `console:local` in that
session under the same five-open cap; the ask reaches the session's own recorded
tmux server, and one whose recorded server cannot be named is refused in the
delivery's own words. A session replaced under its name, a changed lead pair, a
meta that is gone or a server that cannot be named refuses the ask and ends the
writing: the lease is released and the composer is held as
`not writing: <why> · Esc browses`, the refused line back in the draft (a
`/close` as `/close` or `/close <id>`), kept in memory only, and every key
swallowed as above. Without a key, the read every 5 seconds re-proves the
writer's `session_id` and lead pair against the meta and ends the writing the
same way when the session was replaced, its pair changed or its meta is gone; a
fleet read that proves the session stopped ends it too, held as
`<session> is stopped`, and so does a complete read that no longer lists it,
held as `<session> is no longer listed`. A state ae could not read, or an
incomplete read that leaves the session out, ends nothing and keeps the
selection; no read ever ends the writing into browsing. An entry
already held, a refused start included, takes its reason from each fleet read
of the selection — replaced or gone, stopped, no `session_id`, a lead pair
unreadable or changed, a meta that is gone — and stays held. After `Esc` the
composer reads `read-only · <why> - prefix h opens it` until a read proves the
session again.

Each session keeps its own draft and speaker in memory, per incarnation, so
moving between sessions keeps both. Its lead pair is fixed the first time this
run of the app writes it: a different pair refuses as above, and the original
pair coming back writes again with the kept draft. A fleet read that proves a
session replaced under its name (a new `session_id`) or gone drops its draft
from memory, once, with one `ae` line `draft for <session> dropped: <why>`; an
id ae could not read drops nothing. The kept draft on disk comes back only when
writing starts into an empty composer, under the `Kept line, maybe already
sent` banner; a draft kept in memory wins, never merged. The owner `ae chat`
tries the lease inside its own admission: while an app writes its session, it
refuses its sends and `/close`.

## Residuals

- Typing in the app counts as activity in the session that hosts it, so that
  session rises while you work in the app. A key typed in the same second a
  session was created leaves it unknown. A stray click counts as activity too.
- A list scrolled with the wheel keeps its row positions across a reorder, so
  it can show different sessions after one.
- The Instructions tab renders from the records NOW: a running seat received the
  text of its own launch time, so a config or ae upgrade since then shows here
  and not in that seat. A seat's work dir is shown as recorded; a launch also
  proves it exists.
- Settings shows the launch-pinned values for `layout`, `quota`,
  `quota_every_secs`, `idle_nudge_secs` and `done_confirmations`: a running
  session keeps what it launched with even after the config changes.
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
- After `Esc` from an entry its admission refused, the composer can offer
  writing until the next fleet read reads the change; entering is refused again.
- `Enter` in the up to 5 seconds between a session's stop and the next fleet
  read still asks: the admission reads no liveness, only that read does.
- One very long turn is wrapped whole whenever any of its rows is on screen.
- A copy of one key stream read by a second app later than the first app's
  whole write session can submit twice.
- An app left writing, or with Settings open over a draft, keeps the owner chat
  and every other app from writing until it leaves writing or quits.
- Mouse dragging does not select text in the app. Hold Shift (Option in iTerm)
  for terminal text selection; inside tmux an ordinary drag goes to the app.
- Dragging a border needs button-event mouse reports (`?1002h`) in SGR form;
  a terminal that sends only legacy X10 reports gets no drag. Inside tmux a
  binding that replaces the default `MouseDrag1Pane` keeps drags from the app.
- Dragged sizes are not remembered when the app quits.
