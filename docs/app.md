# The app

`ae app [session]` draws the fleet as a terminal UI: every session in a
sidebar, the selected session's Agents beside it, and the selected
session's chat on the right. It is a second view of the data [the chat](chat.md)
already reads, through the same owners, and it writes nothing into any session
but the asks you type into the selected one; opening a seat only moves a tmux
client ([Opening a seat](#opening-a-seat)). `ae chat` is unchanged beside it.

It needs a terminal on both stdin and stdout. Without one it prints
`ae app draws a terminal UI; use ae chat to print the lane` on stderr and exits 1.

Plain `ae` opens it too: on a terminal, with at least one recorded session,
running or stopped, bare `ae` restores what a crashed tmux server held and then
lands here, creating and resuming nothing; inside ae's tmux server it lands in
the current pane unless that pane is a seat's, which keeps `ae list`. Without a
terminal, with `--no-attach` or as `ae --tmux` it keeps the tmux view
([tmux server](reference/commands.md#tmux-server)).

`ae <name>` lands here the same way, with `<name>` as the home: a running session
is opened as it is, a stopped one is resumed in its stored working copy first, and
an unknown name is started in the current directory first. From inside ae's tmux
server it lands from any pane that is not a seat's. `ae <name> --tmux` keeps the
tmux view.

## Home

The **home** session is the one the app opens on and `Esc` returns to: the
`session` argument, else the session whose pane runs the app. You can type into
whichever session is selected (see [Ownership](#ownership)); typing reaches that
session's lead pair, never the session the app runs in unless it is selected. Outside any session
and with no argument there is no home and the first session is selected; with
no session at all the composer says `ae app <session> picks one`. A named
session ae cannot find refuses, exit 1.

`[workspace] chat = app` makes a new or resumed session's `chat` window run
`ae app <session>` instead of `ae chat`, so `prefix h` opens the app. `on` and
`off` keep their meaning; the key is read as `chat` always was, the session's
recorded config first.

## Layout

- **Sidebar** (44 cells from 140 wide, 34 from 90x20): a one-line summary
  on the sidebar ground, regular counts with dim labels and separators —
  `Agents 32 · ● Working 10 · ⚠ Needs you 1`: the live agents of the running
  sessions (a stopped session's saved seats and a dead seat are not among
  them), how many work, and how many sessions need you (`+?` where a running
  session's seats are unknown). A line that does not fit gives way in steps:
  `Agt 32 · ● Wrk 10 · ⚠ Need 1`, then `◈ 32 ● 10 ⚠ 1`, then the needs-you count
  alone; Working and Needs you wear the marks their rows wear, a count is never
  cut, and `icons = off` draws `@ * !`. One blank row
  separates it from the list: one row per running session, the selected one too, with its mark, its
  name, `◂` (`<` with `icons = off`) after the name of the window's own
  session, and at the right end its seat tally (`●3 ◔1 ✓2`; a stopped session
  says `stopped <age>`, a running one with no seats `no agents`, one whose
  seats are unknown `?`). No row number is drawn; `1`-`9` still select the Nth
  session of the list.
  Stopped sessions are hidden behind one line after the running ones,
  `▸ Stopped (3)` (`> Stopped (3)` with `icons = off`): `z` or a click opens
  them as rows (`▾`, `v`) with the start and resume controls they always had,
  and again closes them. The group is this window's memory only, closed at every
  start and never saved. No key, digit or click reaches a hidden row and the
  digits count the sessions listed; closing the group while a stopped session is
  selected selects the home session when it is listed, else the first listed
  session, else none. A fleet read opens the group when the selected session
  turned stopped, so it never leaves the list under you, and when it brings back
  a handed-over view of a stopped session. The counts of the summary do not
  change with the group, and neither does where the Agents section starts: the
  list keeps the rows the open group needs (up to its share of the pane) while
  the group is closed. What a session or seat waits on is told in the chat
  header, never under a row. A narrow row gives up the glyph first, then the
  tally down to its most urgent count, then the name's tail. `↑` / `↓` at the band's first row's right end show needy
  rows outside the list, and `!` reaches the next one. The `Agents N` heading
  under the list keeps its raised panel band. Running sessions come in
  this order: the orchestrator, then the sessions your `[workspace]
  fleet_order` names (the pins), then the rest by their newest AI answer to
  you, newest first, and a session with none after them in creation order;
  stopped ones come last. Your own input never reorders the list. A needy row says so by its `⚠` mark, with no edge, and `!` reaches it. One row is lifted: the one the chat column shows, the selected session while its shared chat is and a seat only while that seat's screen is. It wears a `▸` (`>` with `icons = off`) in its first cell and the selected ground over its one row; with `theme = off` the row is reversed. While a seat's screen is shown its session is a context row: no lift, its name keeps the title ink. A session that answered you while you looked elsewhere carries a `▸` (`>` with `icons = off`) in the blank cell left of its status mark, from the next fleet read until you select it; the home session and the selected one never carry it. The order is unchanged: the cue only says which row answered, the same newest AI answer that orders the list. The wheel moves a list that does not fit one session a notch; a
  selection key brings the selection back into view, a click keeps the list
  where it is.
- **Agents** under the list: every seat of the selected session: its mark, its
  name, the model its own frame drew (the client when none was) and the effort
  that frame drew, each only when known, never the profile name, and its state
  at the right end (from the watchdog's published roster), in words a person
  reads: `working`, `finished` (only a seat that declared done), `waiting on an
  agent`, `needs you (waiting on you)` / `(blocked)` / `(throttled)` / `(usage
  limit)` / `(prompt)`, `unknown` and `dead`. In a narrow sidebar the
  parenthetical goes first (`needs you (blocked)` reads `needs you`) so the
  model and effort keep their row, and the head word stays. A seat whose input box is empty
  with no current done, waiting or blocked declaration reads `no current status`: an
  empty box is a fact about the harness, not that the seat finished. A dim row
  under each seat carries its newest declaration's own reason and the age of the
  declaration (`colimafix result · 12m`); `no reason given` when it left none,
  `reason unavailable` when the journal could not be read, never a reason ae
  made up. The tmux picker and the pane border draw the same state words (up
  to 26 cells: `needs you (waiting on you)`; `waiting on an agent` is 19 and
  `no current status` 17, so in the 44-cell sidebar they leave less room and a
  label moves to its second row sooner) but no reason or age. A seat's label takes one row
  when its whole label fits beside its name and state, with the labels of the
  one-row seats lined up in one column; otherwise a second label row carries the
  label under the name, and the dim detail row follows either. That row drops the effort only when it cannot hold both, and
  cuts the model on `…` only when the model alone is longer than the row (24
  cells at the narrowest sidebar). A click on either row is a click on that
  seat; `n` / `p` step seat by seat and the wheel scrolls rows. Short of room the name's tail
  goes, last the state. A seat that waits on the human is told in the chat header (`⚠ <seat>:
  <reason> · <age>`: the viewed seat's ask, else the cursor seat's, else
  the first), not under the seat: the open request text stays in the header,
  while the seat's own declaration reason sits in its detail row. A reason from
  a seat the roster lacks is drawn as a row of its own after the roster, so no
  ask is lost. A session on
  another tmux server, or one whose watchdog publishes nothing, names that gap
  instead of seats. The body that does not fit gives its last row to the rows
  it hides, `↑ a · ↓ b more rows`, and the wheel moves it three rows a notch;
  another session starts at its first row.
- **Chat** column: the selected session's lane, newest at the bottom, with its
  coverage rows, then the composer. Only the rows on screen and a page beyond
  are drawn: older turns are drawn as you scroll back to them, and the oldest
  stops the scroll. The composer is its address row and one input row while
  browsing; while writing the input grows with the draft to ten rows, taking
  them from the lane, which keeps at least three rows: a short pane shrinks
  the composer first, to one input row at least. A resize sizes it again, and
  leaving writing gives the rows back.

The session list takes two thirds of the rows it shares with the Agents body,
and never fewer than the 11 rows (18 from 40 high) it once had, though never
more than the rows it shares (10 at 90x20, which leaves the body one row);
it is sized to its sessions when they need fewer, so 24 one-row sessions show
22 and a more row at 160x45, where two-row cards showed 11. The chat header names the selected session's
recorded working dir beside its name, your `$HOME` as `~`, cut from the left
with `…` when the row is short so the leaf stays; it comes from the same
listing the sidebar row comes from, not from a second read of the session's
record, and a session with none recorded (or none in the listing) draws none.
A session replaced between two reads of its record can therefore show the
previous incarnation's dir for at most one refresh (5 seconds).

Both borders can be dragged with the mouse. The rule between the sidebar and
the chat sets the sidebar's width: at least 30 cells, and the chat keeps at
least 40. The rule under the `Agents` heading moves the split between the session
list and the body: the list keeps one session and its more row, the body three rows (or
as few as it had undragged). The rule being dragged is drawn in the title
colour, reversed with `theme = off`; letting go ends the drag. The sizes last
for this run of the app and are not remembered; a smaller pane narrows them
only while it is small.

Below 90x20 the sidebar goes and the chat takes the pane; below 40x8 the app
only says how large it needs to be. Colours come from the session's own look
(home first, then the selected session, then the default palette); with
`theme = off` it draws no colour, and the selected name is bold and reversed.

### Order

A session's place in the sidebar is its newest AI answer to you: the newest
reply an agent addressed to your chat or your bridge. Nothing you do moves a
row: not a question you ask, not typing, attaching, switching, clicking or
scrolling. An answer a lead gives only in its pane, without a reply record,
moves no row either, and neither do ae's own commands. A new read of the
fleet can move a row, and
only when a frame is drawn: a key or click always acts on the row it showed,
even one the terminal splits over several reads; a click whose frame is gone
when it completes is not taken, and the hint row says so.
The status strip and the picker keep their own order.

## Reading

A background reader does every read — the fleet, each session's lane, the
looks, whether the session being written is still proven — so no key waits on one. It reads the
fleet first, then a lane the composer just wrote into, the selected session,
the fleet again every 5 seconds, home, the sessions one and then two rows from
the selection, and the rest in sidebar order, keeping at most 64 lanes; the
selected session and home are read again every 5 seconds. Until the first
fleet read the list says `loading`, and a key read until then is not lost: `q` and
`^C` act at once, the digits, `!`, `j`, `k`, the arrows, the page keys and
`Esc` wait (64 at most, the oldest go first) and replay on that read, and
anything else is dropped under one hint, `input before sessions loaded was
dropped`. A session whose lane was not read yet
shows one dim `loading` row, and one read before shows its last lane at once.
A lane fills in stages, each its own read: the session's journal first, then
the lead pair's current conversations, then — for the selected session only —
their earlier conversations. Until the last stage lands the lane carries one
coverage row, `pane turns — loading: …`, naming the turns still to come; a
seat a stage could not read keeps its own coverage row after that. A session
that is not selected stops after its current conversations, and selecting it
reads the earlier ones next.
An answer that lands after you moved on is kept for its own session and never
drawn under another.

The reader keeps each listed session's parsed journal and memo between reads
and reuses one only while the file's identity is the one it was read at:
device, inode, length, modification and change times to the nanosecond, type
and mode, read through a link. Any change reads the whole file again, and a
file that is not a regular file, or could not be read, is never kept; a session
that leaves the fleet takes its files with it. A fleet read's "needs you"
sections use that read's own records and its own tmux listing of each running
session's panes. Nothing tmux says is kept from one read to the next.

## Keys

Browsing:

| Key | Does |
|---|---|
| `1`-`9` | select the Nth session of the list, counted by position (no number is drawn) |
| `j` / `k`, `↓` / `↑` | next / previous session |
| `!` | the next session that needs you |
| `oo` | open the highlighted seat's pane: the first `o` only arms (`o again to open <seat>`), the second within 2 seconds opens that same seat |
| `m`, right-click a card | the session menu on that card ([Session menu](#session-menu)) |
| Click a seat, or `v` on the highlighted seat | inside tmux: watch that seat's live screen in the chat column, read-only ([Peeking at an agent](#peeking-at-an-agent)); outside tmux a click works in the seat itself ([Working in a seat](#working-in-a-seat)) |
| `n` / `p` | step the seat cursor (`▹`, `-` with `icons = off`) to the next / previous seat the body shows; with no cursor `n` lands on the first seat and `p` on the last, and both wrap |
| `PgUp` / `PgDn` | scroll the chat a page |
| `[` / `]` | mark the next older / newer turn; with none marked, the newest turn the chat shows. At the oldest or newest turn the frame produced nothing moves and the mark stays |
| `y` | copy the marked turn ([Copying a turn](#copying-a-turn)) |
| `r` | read the first file link of the marked turn in the chat column ([Reading a linked file](#reading-a-linked-file)) |
| Click a turn | mark it; clicking the marked turn again unmarks it |
| Click a fold row, or `Enter` with a marked turn that has one | show that turn whole or fold it again ([Long turns](#long-turns)) |
| Click session row | select that session |
| Click the `Stopped (N)` line | open or close the stopped sessions |
| Click a seat row | highlight that seat |
| Wheel over chat or the Agents body | scroll three rows per notch, also while writing |
| Wheel over the session list | scroll one session per notch, also while writing |
| Drag a border | resize the sidebar or the session list, also while writing |
| Click composer | write the selected session, when no other app writes it |
| `Esc` | clear the seat cursor, else back to home |
| `Enter` or `i` | write the selected session, when no other app writes it |
| `s` | start the selected session if it is stopped, else ask to stop it ([Session menu](#session-menu)) |
| `z` | show or hide the stopped sessions (the `Stopped (N)` line) |
| `,` | open Settings |
| `?` | open Settings on its Keys tab, the list of every key |
| Click the gear at the hint row's right end | open Settings, also while writing |
| Click a Settings tab title | show that tab, like `Tab` and `1`-`6` |
| Click `Esc close` at the Settings title row's right end, or the gear | close Settings, like `Esc` |
| `qq` | quit: the first `q` only arms (`q again to quit`), the second within 2 seconds quits |
| `^C` | quit, at once; over a draft Esc kept it clears the draft first |
| `^V` while writing | save a clipboard PNG and insert it as an `[Image #N]` token; Enter sends its path |

No single printable key quits or moves your client: an armed `q` or `o` is
disarmed by any other key or when its 2 seconds pass, so typing a word
while browsing leaves the app up and no key reaches another pane. The
closed frame's hint row carries the composer's hint and the mode's keys
(`? keys` while browsing, which lists every key; while writing
`Enter sends · Alt+Enter ^J newline · Esc browses`; none while held, which
takes every key), and at its right end `ae <version>` and the gear. A
pending line — an armed key, a refused paste, an ask on its way —
replaces the composer's hint in full ink; the `·` separator and the
mode's keys follow only when the whole of it fits.
Settings and a viewed seat keep one footer row with the mode word
(`settings`, `viewing`), its hint (`Esc close` in Settings, `Esc chat`
while viewing), the version and the gear (no gear while viewing). The
gear's click covers the gear and the blank before it, as the status
line's settings button pads its glyph. Narrowing drops the version first,
then the gear.

`^V` does nothing while browsing. While writing it reads an image without
blocking typing; another `^V` is ignored until the read finishes. It accepts
PNG up to 20 MiB, saved at 0600 under the written session's `messages/`.
Refusals keep the draft. The composer shows the image as one `[Image #N]` unit
(the chat's token, see [chat image paste](chat.md#input) for editing, the sent
`[Image #N: "/abs/path"]` form, tools and client support). A copy of the history
takes the label as drawn; the stored ask keeps the path.

A bracketed paste that is nothing but absolute file paths (separated by line
breaks or spaces, a `\ ` or one level of quotes around a name read back) with a
known extension (png, jpg, jpeg, gif, webp, heic, bmp, tiff become `[Image #N]`;
pdf, docx, xlsx, pptx, csv become `[File: name]`) shows as those tokens, and Enter
sends `[File: "/abs/path"]` for a file. ae reads nothing of the file: a path that
does not exist is sent all the same, and the file is never deleted by ae. Any
other paste stays text, and so does one over 16 KiB. The whole paste is judged
again with each fragment, so the draft is the same however the terminal splits
it; anything else you do to the draft in between ends that paste's run.

A paste while browsing is never read as keys. On a writable session it starts
writing with the paste as the draft, every fragment of it kept and nothing in
it acting: a pasted line break (CR, CRLF or LF) is a line break in the draft, and the draft is sent only by
your own `Enter`. On a read-only session or with the composer held it takes
nothing, and the hint row says why for a few seconds (`paste not taken: …`);
the Settings overlay swallows a paste silently.

Writing: the draft is the chat's own input — a plain line asks the session's lead, or its colead when [routing](#routing-a-line) says the lead cannot take it,
`@<seat> text` either lead-pair seat for that line only, `/close` withdraws your newest open ask,
`!<command>` runs that line as your own command ([Running a command](#running-a-command)),
five asks open at most; see [Input](chat.md#input). `Alt+Enter` and `^J` put a line break in the draft, `Enter` sends, and Shift+Enter is `Enter` (tmux hands the app a plain CR for it). A refusal such as the cap keeps the line in the composer. Writing stays on after a send. The composer shows the
whole draft as the chat does — the same wrap, each pasted line break its own
row, all of it under the address row from the column's left edge, and past its rows the window
around the cursor with `… +N lines above` and `… +N lines below` among them
(no markers below three rows) — and the terminal cursor sits where the next
character goes. The cursor shows only while writing with the composer drawn:
browsing, a held composer and Settings hide it. The address row, every draft row
and the note row under them are one click target. `Enter` sends, `Esc` goes back to browsing and keeps
the draft. `^C` over any text in the composer — typed, or kept by `Esc`, also under an open Settings or peek — clears it whole and never quits (`draft cleared · ^Y brings it back` on the hint row); over an empty composer it quits at once — except within 2 seconds of a clear (by `^C` or `^U`), where the first `^C` only arms (`^C again to quit · ^Y brings draft back`) and a further one quits, so a `^C` meant for the text never ends the app; any other key disarms. `^U` clears the whole draft the same way, every line. `^W` erases the word before the cursor (back to whitespace), `Opt+Backspace` (`ESC DEL`; the terminal must send Option as Meta) the one before it up to a non-letter, non-digit, and `^K` the rest of the cursor's line, or its line break when nothing is left on it. `^Y` puts the last cleared draft or erased text back at the cursor, once: a send ends it, a newer clear or erase replaces it (an erase of nothing does not), and a draft with no room for it keeps it waiting; an erase that empties the draft guards the quit like `^U`. While there is text the hint row also reads `^C clears`. `/open <seat>` opens that seat of the selected session
([Opening a seat](#opening-a-seat)). While an ask is being delivered the hint row reads `sending to <seat>…`;
delivery waits on a busy seat, so it runs off the app's loop, which keeps
drawing and taking keys. One ask flies at a time: a second `Enter` meanwhile is
refused (`not sent to <seat>: an ask to <seat> is still being delivered; the line
is back in the draft`), `/close` waits (`refused: the ask to <seat> is still being
delivered; /close waits for it`), and a quit waits for the delivery, at most four
minutes, saying `finishing the delivery to <seat> (^C stops it)`. Each outcome shows once, as an `ae`
line in the lane of the session it is about (an unconfirmed or undelivered ask
as its own lane line, with the hint row saying the same until the lane has it); a line about a session no longer recorded shows in
whichever lane is selected.

Writing starts the moment `Enter`, `i` or a paste enters the composer; it takes
no writer or admission lock, so any number of `ae app` windows write the same
session at once, each with its own draft. An earlier window's
[kept-line offer](chat.md#input) is accepted only on a fresh Enter, which
fills the composer; the next Enter sends. Esc discards that
offered line. Browsing keeps an undecided offer and its exclusive claim until
this app exits. Each window's own kept lines stay private while it lives.

Keys read before writing starts — the rest of the read that started writing included — are
dropped, and one `ae` line says `dropped the keys typed before writing
started`; `Esc` and `^C` are never dropped. A send or `/close` while another
window is sending to the same session is refused at once, never waited for:
one `ae` line in the lane says `refused: another window is sending to <session>
(held at <holder session>:<window>) — Enter retries`, `(holder unknown)` in place
of the place when the holder left no record that reads, or without the place
while only the chat's admission is busy (the owner `ae chat` or a chat bridge
delivering). The line or the `/close` stays in the composer, writing stays on,
and the next `Enter` tries again; nothing retries by itself. A lease lock file
that is not a regular file refuses the same way, `refused: the writer lease is
not a regular file`. A start the session's own facts refuse holds the composer
as `not writing: <why> · Esc browses`: every key is swallowed until `Esc`
browses, `^C` quits, a click acts or `Enter` tries again; while the session is
not proven, `Enter` stays held.

While writing, clicking a session row returns to browsing and keeps the
draft, then selects the row (a turn: marks it). Blank rows and the "more" row do nothing.
A press within a cell of a border grabs it unless a row is drawn there.
Any key, paste, press or wheel notch ends a drag whose release never arrived,
then acts as it always does.

## Opening a seat

`o` takes your tmux client to the highlighted seat's pane, in any session you
selected, home or not; `/open <seat>` in the composer does the same for a named
seat of the selected session. It makes the proof `ae chat`'s `/open` makes,
then moves one tmux client into a view of the seat: nothing is typed into the
seat and no record is written.

The highlight is the seat cursor: one `▹` (`-` with `icons = off`) in the first
column of the seat's row, never a state colour. No seat carries it at first;
`n` lands on the first seat drawn and `p` on the last, then they step through
the drawn seats and wrap, and changing session resets it. `Esc` clears it
before it goes home, and `o` / `v` with no cursor act on the first seat drawn.
A click on a seat row, or `v`, shows that seat's screen instead, and the
lifted `▸` marks the seat shown. Rows scrolled out of the body are not stepped
to; wheel first. `Enter` still writes.

A key acts on the seats the last frame drew, as it drew them: the seat's slot
and name, and the session incarnation they were read from. A roster that moved
the seat to another slot, a session replaced under the same name, a pane
restamped for another agent, a dead pane or one the server no longer lists in
the session refuses; so does a selection that changed since that frame
(`the view changed`). Nothing is selected on any refusal.

Which client moves. Only a tmux client whose ACTIVE pane is the app's own pane
can have pressed the key, and only that one client moves: every other client
keeps its session, its window and its status line, the seat's session included.

- The client is handed to a clean view of the seat: a session of its own
  (`_aeview-…`) grouped with the seat's session, so it shares that session's
  windows and none of its options. It shows the seat's pane under one status
  line naming the session and the seat and the way back, with no window list
  and no ae bar; what you type reaches the seat's tool. `ae list`, the fleet
  strip, the picker and the session inventory never show the view.
- With several clients on the app, the one with the newest input is the one
  that pressed the key; two last active in the same second refuse (`two clients
  showing this app were active in the same second`) and move nothing, whether
  the seat is in the app's own session or another. A control-mode client never
  records input, so two of them always tie.
- No client shows the app (`no tmux client is showing this app`), or the app
  runs outside tmux (`ae app is not running inside tmux`): refused.
- One app views a session at a time (`another ae app is viewing <session>`),
  and a session on your own tmux rather than an ae-owned server is refused
  (`pane view needs an ae-owned tmux server; <session> runs on your own tmux`).
- When tmux refuses the view part-way, the client is taken back to the app and
  the view destroyed; the terminal is never detached (`refused: /open <seat>:
  tmux refused the view`).

A stopped session refuses and names `ae <session>`, which resumes it; the app
never resumes one. A session recorded on another tmux server than the one this
app's pane lives on, or whose server ae cannot prove, refuses the same way
(`run ae <session>`). The server is the app's own (`$TMUX`), never the one a
launch would use.

Getting back. `F12` or `C-]` takes the viewing client back to the app: to the
app's pane while it is still the pane of the app's window, else to the app's
session with nothing selected; the view is destroyed with it. The lane says
`viewing <seat> · F12 or C-] back to ae app`, and the view's status line says
the same. For every other client both keys reach the pane unchanged.
`prefix d`, `prefix L` or any other switch also leaves the view, which is
destroyed once no client shows it; that client's `F12` or `C-]` in the seat's
session still goes back to the app, and the next open does not wait on it. A
seat's session that ends while viewed takes the client back to the app within
about a second.

Named limits. The client list is read last, right before the move; a client
that detaches in that instant refuses the open. If the app dies while a client
views a seat, `F12` and `C-]` still go back, but nothing takes the client back
when the seat's session ends meanwhile. The cursor steps only through the rows
drawn.

## Session menu

While browsing, `m` opens a menu on the selected session's card; a right press
opens it on the card under the pointer, also while writing or held; the draft
or the held reason is still there when it closes. It offers `Start` on a stopped
session and `Stop…` (`Stop...` with `icons = off`) on a running one, and nothing
on the orchestrator. `s` or a click acts, `Enter` chooses, and `Esc`, `^C`, `m`,
a right press or a press outside close it. Opening it selects nothing. Browsing,
`s` does the menu's one item at once on the selected session: it starts a stopped
session, and on a running one asks the Stop question below.

The menu acts on the card as that frame drew it. A card in an
unknown state, on another tmux server, or whose record names no single
`session_id` shows why in the menu, and its item refuses with that reason. A
record that no longer names the session the menu was drawn from refuses too.

`Start` is the fleet picker's own stopped-row job — the ordinary launch owner
behind the same captured-client proofs, run in the background by tmux — so it
resumes the session and then moves this client to it. Outside tmux there is no
client to move: `Start` resumes the session the same way with nothing attached
(`Starting '<session>'.`), after proving the record is still the incarnation the
card drew. A stopped session recorded on another tmux server refuses. `Stop…`
asks first; outside tmux it asks about the session on its recorded server (a
record naming no tmux server refuses, `its record names no tmux server`), and the
answer re-proves that server, the session's id and the deadline under the stop's
own lock. The question
says what a stop does and, on the app's own session, that this app closes;
`Cancel` is focused. `s` / `c`, `←` / `→` or `Tab` move the focus, `Enter`
presses it, `y` stops and `n`, `q` or `Esc` cancels. The answer is proven again
against what the question captured: its deadline, the session's record and
the client you asked from. The stop is then handed to the same guarded owner
the tmux session menu's Stop uses, which proves the tmux session, its server
and that client again under its lock. `Stopping '<name>'.` means it was handed
over, not that it finished.

A menu or question takes only input made after a frame showing it was written:
a key typed before then is dropped and the hint row says to press again. One
that does not fit the window closes, and the hint row says so.

## Working in a seat

Outside tmux, a click on a seat hands the app's terminal to that seat's pane:
the app steps aside and tmux attaches this terminal to the seat's session with
the seat's pane selected, so you type into the seat, colours and resize
included, and `^C` reaches the seat's tool. The status line names the way
back: `F12` or `C-]` returns to `ae app` (`prefix d` works too), and the app
redraws where it was. `v` outside tmux still refuses (`refused: peek …`);
inside tmux a click keeps opening the read-only peek below.

The seat is proved as `oo` proves it, and only one app views a session at a
time: a second one is refused before it attaches (`refused: view <seat>:
another ae app is viewing <session>`). The return keys act only for the
terminal the app handed over; anyone else's `F12` or `C-]` reaches their pane
as before. If the seat's pane cannot be selected once attached, the terminal
detaches straight back. On return the session's window and pane are put back
as they were, but only when nobody moved them meanwhile. A refusal says why on
the hint row (`refused: view <seat>: <why>`), and the return says
`back from <seat>`.

## Peeking at an agent

Click a seat in the sidebar, or `v` on the highlighted seat, and that seat's
live screen replaces the chat where the chat lane is: the seat's screen and
recent history as plain text. The sidebar stays drawn and clickable, and
clicking another seat retargets the view. Nothing is typed into the seat, no
record is written and no client moves. The seat is the one `oo` would open,
as the last frame drew it; with no session selected or none highlighted, in a
stopped session, with no readable record directory, or outside tmux, the click
or the key opens nothing and the hint row says why (`refused: peek <seat>:
<why>`, or `refused: peek: no session is selected`).

Every capture, the first included, proves the pane as `oo` does: the session is
on the server the app runs on, the roster still seats that name in that slot,
the pane carries it, is alive and is listed in the session. A pane that fails
the proof is not read; the view shows the refusal by name (`refused: peek
<seat>: <why>`) in place of any earlier text. A capture tmux did not return
shows `capture failed: tmux did not return the pane's screen`, an empty screen
`(the pane shows nothing)`; the status line then reads `not captured`.

The reader captures again every 500 milliseconds: 200 lines of history above
the screen, the newest 400 kept, at most 2000 characters a line (`…` ends a cut
one). Pane text is hostile, so each line is neutralised when painted (a
control character or escape shows as `�`) and then wrapped to the chat column,
each continuation row two cells in. No colour is requested.

The column header and rule stay; under the rule a status: `loading` until the
first answer, then `captured Ns ago · R rows`, led by `STALE · ` from 5 seconds
on. `j` / `k`, `↓` / `↑` scroll a line, `PgUp` / `PgDn` a page; the wheel
moves one session a notch over the session list, three rows a notch over the
Agents body or the viewed screen, and nothing anywhere else. Scrolled back,
the view is paused: a newer capture waits
(`paused · B back · newer waiting`) and is shown once you are back at the
newest row; a refusal or a failed capture shows at once and returns to the
newest row. `Esc`, `q`, `v` close it (`q` here is no `qq`), `^C` quits at once;
so do a click on the selected session's row and any session switch, which land
back in that session's chat. Every other key, a paste and every other press is
swallowed, so none reaches the pane; session keys wait for `Esc` first. An
upgrade restart waits while a seat is viewed.

While a seat is viewed the composer writes nowhere: its row reads `viewing
<seat> · read-only · Esc chat`, and the keys row `viewing   Esc chat`. `y`
copies nothing here: it only shows the note `drag to copy text`. A mouse drag
over the viewed text copies as in the chat, and the outcome note rides the
keys row.

Named limits. The pane can change between the proof and the capture of one
cycle; the view only displays, so nothing is moved or typed on that account.
A capture is plain text: styling and images the seat's tool draws are not
shown.

## Reading a linked file

A plain press and release on any file-link cell opens that occurrence in the
chat column, read-only, with the sidebar still live. Wrapped links and wide
characters work on every covered cell. A press changes no mark or draft; dragging
selects text instead. Another press, key, wheel notch or changed view cancels the
pending click. A stale link says `refused: read: the view changed`. Web links keep
the ordinary turn-mark toggle; Cmd/Ctrl-click stays with the terminal.

`r` on a marked turn (`[` / `]`, or a click outside a file link) opens its first
file link. Both paths use the same list over every displayed part, including
options and evidence, before folding, clipping or scrolling. Duplicate occurrences
stay separate; web URLs are omitted. Closed asks use the displayed excerpt;
Accepted turns have no links. A turn with no file link says `no file link in that
turn`. With no mark the hint row says `no turn marked - click one or press [ ]`,
and a mark the lane no longer holds is cleared (`that turn changed or left the
lane - mark it again`).

In the reader, `n` / `p` visit the next / previous file occurrence and stop at the
ends. Each step opens at the top in that link's default mode. More than one link
adds `first of N links` or `K of N links` to the header, including while loading
or refused. One link has no position suffix.

The file is read once, off the UI thread, and only when it is a regular file
whose real path lies under the session's work dir or your home: a path that
leaves both by `..` or a symlink, a missing file, a directory, a file over 1 MiB
or 50,000 lines, and a file with a NUL byte in its first 8192 bytes are not
shown, and the panel says why (`refused: read <path>: <why>`) where the file
would be. The terminal's own link still opens such a path. Text is decoded
lossily; every line is neutralised when painted (a control character or escape
shows as `�`), a tab expands and a line wraps to the column with its
continuation rows two cells in. A line over 2000 characters ends in `…`.

Markdown files (`.md` or `.markdown`, case-insensitive) open rendered: headings,
nested lists, emphasis, code, quotes, rules and tables. Link targets remain dim
text, images show as `[image: alt](target)`, and HTML stays literal. Nothing is
fetched or opened. `m` switches between rendered and raw Markdown without another
read; switching or resizing keeps the top source block in view. Other files keep
their raw view, where `m` does nothing.

Markdown headers add `rendered` or `raw` before `rows A-B/N`. Other files keep
`Read · <path>[:line] · rows A-B/N`. Raw lines are numbered in a gutter. Any
`:line` link opens raw, jumps with three rows of context above the line and
shows its number in bold; `m` can then render it. `j` / `k`, `↓` / `↑` scroll a row, `PgUp` /
`PgDn` a page, `g` / `Home` the top and `G` / `End` the bottom, and the wheel
three rows a notch. `Esc`, `q` or `r` close it and the lane comes back as it
was, with the mark, the scroll and any kept draft untouched; `^C` clears a kept
draft first and quits when there is none, as in the lane. A click on the selected session's row closes it too; a click on another
session or a seat leaves it. Every other key and a paste is swallowed. Nothing
is typed, written or journaled, and ae never opens a web URL itself. An upgrade
restart waits while a file is read.

Rendered output has limits: 50,000 rows, 2,000,000 cells including code padding,
32 table columns, 240 cells per padded table row, and 32 nested quote/list items.
A block beyond a local limit appears raw after one note. Tables wider than the
column appear raw. Past the row or cell limit, the whole document opens raw;
the header and `m` say why.

Named limits. The file can change between the check and the open, or after it
is read; the panel shows the bytes it read. A terminal that reports Cmd-click as
an ordinary press lets ae handle it. Closed-ask excerpts stop at 80 characters:
any file token touching the synthetic `…` is omitted from the reader list and
click cells, even a path ending exactly at character 80 when more text follows.
Its legacy terminal hyperlink remains; those cells keep the turn-mark toggle.

## Long turns

A turn of more than thirty rows, as the app draws them at the chat's width, shows its first
24 and a dim row `... N more lines`; thirty rows or fewer show whole. A fold must save at
least one rendered row; the fold row is one row here, so that rule never moves the
thirty-row line. The fold row is a
click target and `Enter` on a marked turn that has one toggles it too, the mark kept;
the unfolded turn ends in `show fewer lines`. Enter with no mark, on a short turn or
over a composed draft keeps its usual job and never puts the draft aside. A decision
card folds after its options are split. Which turns you opened lives only in this
app's memory, per session incarnation: a different session, a replaced one or a
restart shows every turn folded, and a new turn starts folded. A sent ask's header
reads `to <seat> · sent`; a failed outcome keeps its alert line. The ask body is the
stored whole text; one that could not be read shows its summary and the reason in
the header ([Residuals](#residuals)). `/more` is the tmux chat's: typed here it is
refused with a hint to use the fold row.

## Copying a turn

To copy text you drag over it ([Selecting text](#selecting-text)). To copy one
whole turn out of the chat, mark it and press `y`.

A turn is marked by a click on it, or by `[` / `]` in browse mode (they are
draft text while writing and swallowed in Settings). The mark shows as a bar in
the first cell of each body row (`|` with icons off) and the speaker restyled,
and the chat scrolls to keep the turn in view. It stays after a copy, so `y`
repeats, and clears when you select another session; opening and closing
Settings keeps it, and `Esc` keeps its usual job. A click while writing returns to
browsing, keeps the draft, then marks.

`y` copies the turn's body only, not its `speaker time` line: the recorded text
with every control byte except line breaks and tabs shown as U+FFFD, and
nothing added. It goes to tmux's default paste buffer (`prefix ]` pastes it)
with `tmux load-buffer`, and also to the terminal clipboard (OSC 52) of the one
client provably showing the app: the clients whose active pane is the app's,
the one with the newest input. None or a tie names nobody, and the buffer is
still loaded. Outside tmux there is no buffer and no client: the app writes the
text to its own terminal as an OSC 52 write, up to 65536 bytes. The hint row
says what happened for a few seconds, and never that the clipboard was
reached, because neither tmux nor the terminal gives an acknowledgement:

| Hint row | Meaning |
|---|---|
| `copied N lines to the tmux buffer; terminal clipboard if your terminal allows it` | loaded, with a client named |
| `copied N lines to the tmux buffer only - <why>` | loaded, no client named: `no tmux client is showing this app`, `two clients showing this app were active in the same second`, `the tmux clients did not answer` or `the client name failed its grammar` |
| `copied N lines (preview only: <why>) to the tmux buffer…` | the text is not the whole body ([Residuals](#residuals)) |
| `copy failed: …` | tmux refused the load |
| `sent N lines to the terminal clipboard - unconfirmed` | the app is not inside tmux: it wrote the text to its own terminal as an OSC 52 write, which nothing acknowledges |
| `not copied: N bytes is over the 65536-byte clipboard limit` | outside tmux only: the text is refused whole, never cut |
| `no turn marked - click one or press [ ]`, `nothing to copy: that turn has no text`, `that turn changed or left the lane - mark it again` | nothing was copied; the last one also clears the mark |

The terminal must take OSC 52, and inside tmux let tmux use it. iTerm2: Settings > General >
Selection > "Applications in terminal may access clipboard". Terminal.app has
no OSC 52. Alacritty with `TERM=alacritty` needs
`set -as terminal-features ',alacritty*:clipboard'` in tmux (with `TERM=xterm*`
tmux's default `terminal-features` already carry the clipboard). Inside tmux the
paste buffer works regardless; outside tmux the OSC 52 write is all there is.

## Selecting text

Drag over drawn text with the left button, in the chat, a viewed seat, the
session list or the Agents body: the cells light up while you drag (reverse video,
so text that is already reversed shows its inverse), and letting go copies
exactly the lit text the way `y` does. The hint row answers with the same
`copied N lines to the tmux buffer; …` words (outside tmux,
`sent N lines to the terminal clipboard - unconfirmed`); while a seat is
viewed the answer rides the keys row instead, for a few seconds.

The text is the drawn cells, row by row, each row cut at its last character
and the rows joined by a newline (an empty row stays, nothing follows the last
one). A wide character is taken whole from either of its cells, a control byte
reads as the blank the terminal gets, and a marked turn's bar is never part of
it: the copy is the same marked or not. A drag stays inside the panel it began
in, and the sidebar rule, the header and the composer are not selectable.

A press with no motion is the click it always was, a border still grabs first,
and the wheel is unchanged. `Esc` with a selection held only clears it (a second
`Esc` does what it always did); so does the next press. The selection also
goes, copying nothing, when what you selected changes before the copy leaves:
another session, Settings or a seat view, a resize, or the cells under
it. While the button is down and until the copy is out, a new read of the chat,
the session list or the looks waits (a seat capture, in its column), the
header's ask age stands still and a restart into a new core waits; they land just
after the copy, so the copy is the cells you saw. A drag with no mouse input for
10 s is cleared (`selection cleared: no mouse input for 10 s`). A copy leaves
only after the frame that shows it.

## Links

A web URL (`http://` or `https://`) or a repo-style path in a turn is a real
terminal hyperlink: Cmd-click (Ctrl-click on Linux) opens it with mouse capture
off. While ae or tmux captures the mouse, many terminals use Shift to bypass it:
Ghostty: Shift+Cmd-click; Linux: Shift+Ctrl-click. A plain click goes to ae; in
the app, a plain click on a marked turn's link does not open the terminal link.
The text is underlined while a look is drawn. A path is one of

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

Only the chat column's turns are linked (the Settings About tab's addresses and the
seat view stay text), and the underline needs a drawn look. The visible text, the wrapping and the fold counts are the same with or without a
link, and a link a wrap cuts is two links with one id, so a terminal that
lights a link under the mouse lights both rows. The app writes the pair around exactly the cells
of the link, closing it before any cursor jump, and redraws a link's cells
every frame so a moved or dropped link never keeps a stale one on the terminal. Every body is neutralised
before it is scanned, and a URL is written only as printable ASCII with
anything else percent-encoded, so no record can end a link early, open a
second one or smuggle a control into it.

OSC 8 must reach your terminal. Terminal.app has no hyperlinks. Inside tmux
(3.4 and later, ae's floor) tmux forwards a link only when its
`terminal-features` say your terminal takes them; for most terminals add

```
set -as terminal-features ',xterm*:hyperlinks'
```

to `~/.tmux.conf` (match `$TERM`; Ghostty's TERM is `xterm-ghostty`,
which matches `xterm*`). After `set -as`, reattach with a new client to refresh
capabilities. ae never writes the option; unforwarded links show plain text.
Following a file link inside ae uses a plain click or
[`r`](#reading-a-linked-file) on a marked turn; memos and request threads have no
reader. Dragging over a link selects and copies the visible text, as ever.

## Settings

`,` opens the read-only Settings overlay over the whole pane, as does the
gear (`*` with `icons = off`) at the hint row's right end — the gear also
while writing, where `,` stays draft text; `Esc`, `q` or `,` closes it, `^C`
quits. While open the overlay owns every key and click: `Tab`, `1`-`6` and a
click on a tab title switch tabs, a click on the drawn `Esc close` label or on
the gear closes it like `Esc`, `j` / `k` and `↓` / `↑` scroll a line, the
wheel three rows per notch, `PgUp` / `PgDn` a page, everything else is
swallowed (every other click too), and a draft being written is kept untouched
until it closes. Six
tabs: Keys lists what each of the seven marks means (`✖` dead, `⚠` needs you,
`◌` unknown, `●` working, `◔` waiting on an agent, `✓` finished, `·` no current
status; the `icons = off` letters when icons are off) and then every key of
browsing, writing, a held composer, Settings and the mouse
(`?` opens it while browsing); Quota draws the same table as the tmux quota dialog from one formatter — a column
header, a head per scope, then per window a name, an 8-cell bar, `USED`, `COUNTS`, `NOTE`,
`RESETS` and `SEEN`, determinable scopes first, most available first, unreadable scopes last —
laid out at the pane's width: `SEEN` and then the bar
drop first on a narrow pane, `#`/`.` replace the bar glyphs with icons off, and a pane
narrower than the table's floor clips each row at its edge; Config shows every
`[workspace]` key the SELECTED session (else home) runs, or the current global
alone when neither exists,
with and where each came from — `launch` (pinned in its meta), `session` (its
origin overlay), `global` (the meta-recorded file, the current global only
when that row is empty) or `default` — with `global only` on the keys that
read the current global alone; About names the versions, the state root, the
config file, the recorded server and the repo links; Instructions shows what ae
tells the agents of the SELECTED session (the one selected when Settings opened),
from its records alone, so a stopped session shows too: the custom `[prompt]
instructions` in force with the file they come from — `global` (the meta-recorded
file) or `session` (its local overlay), the last file declaring the key wins even
when empty — or a line saying there are none; then each generic rule text a
launch hands out, ONCE, under its audience — every seat, workers, the lead
pair — as the one renderer produces it now, with `<session>`, `<helpers>` and
`<owner line>` standing for what a launch fills in per seat; then one line per
roster seat, `name · slot · lead` or `worker`, naming the rules it receives.
Everything wraps to the pane. A session, config file or seat ae cannot render
from shows a named `gap` instead of a guess: no records, an unreadable meta or
config (the whole tab), an unknown slot. Clients, the sixth tab (`6`), summarizes
the existing live-seat snapshot: total seats, activity, clients, models and
model + effort, sorted by descending count then name. Unproved clients and models count as
unknown. Unknown-liveness seats are counted separately and excluded from live
totals; an incomplete snapshot marks the counts partial. The summary wraps to
the pane width, above the seats grouped by account. Its aligned columns keep session:agent,
profile, model, effort, observed activity and declared state distinct. Busy rows
use the working color; unknown activity is dim, account gaps use the stale color.
The text keeps its meaning with the theme off. Narrow panes clip each row with
an omission mark and omit the column header; the tab titles shorten to keep all
six clickable. Unknown accounts, unknown liveness, an incomplete snapshot and
the snapshot age remain visible. Quota contains the quota table alone.
Quota and Clients show retained values at once, under a line `last updated <age> ago — refreshing` (the first open of a run says `loading`), and replace them in place when the read lands, the line going with them; a read that fails outright keeps the values under `refresh failed · showing values from <age> ago`. The existing snapshots refresh while Settings is open; config, about and instructions read
once per open. A torn config shows one honest row naming the file. The
Instructions head states the residual below: this is today's render, not what
a running seat was handed.

## Routing a line

A plain line goes to the session's lead. When the lead cannot take it — the same
verdict `ae list` shows: not running, out of quota, blocked, in error, or busy on a
turn that is not your open ask — it goes to the colead instead, and the lane gets
one quiet note (`sent to colead (lead busy)`); the colead's ask opens with one
line, `(routed to you: lead busy)`, only when it was rerouted. An open ask of yours
keeps its follow-ups with its seat even while that seat is busy on it. Both seats
busy: the line waits for the lead (`queued for lead (both busy)`). A leading
`@lead` or `@colead` picks the seat for that line only. Replies are unchanged.

When both seats can take the line and no open ask of yours pins one, quota breaks
the tie. Each seat's pressure is the effective percentage `ae quota` judges (never
the raw one) of the worst usable window of its own account proven at least a day
long; an account whose spend cap is reached reads 100%. The lead keeps the line
unless that reads 80% or more (the doctrine's step-down line) and the colead's
reads at least 20 points lower; then the colead takes it, with the same quiet note (`sent to colead (lead quota 86%)`) and the same
opening line (`(routed to you: lead quota 86%)`). A quota ae cannot read — no
proven account, no usable long window, `quota = off`, a read not landed yet — is no
evidence, and the lead keeps the line. Busy, out-of-quota, an open ask and a
leading `@lead` or `@colead` all outrank it. The read is the app's own, at most
once a minute per session, and starts no process.

## Running a command

`!<command>` in a window that writes runs that one line as your own command, in
the selected session's recorded work dir, and writes nothing to the session: no
journal row, no kept draft, no ask, no seat and no Telegram message — the
outcome is one `ae` line in this window's lane only. Inside tmux it opens the same
popup [`ae chat`'s `!`](chat.md#run-a-command) opens, on the client whose active pane is
this app, and says `! <command> - popup started`. Outside tmux there is no popup:
ae starts `/bin/sh -c <command>` itself — the program fixed, the line one argument,
no `$SHELL` read — detached in a process group of its own, with stdin, stdout and
stderr on `/dev/null`, never waited for, and says `! <command> - started`. Its
output goes nowhere; redirect it to a file to keep it. Quitting the app leaves it
running.

The session is proven first, as a send proves it: a session replaced since
writing started ends the writing and puts the line back as a held draft. Any other
refusal keeps writing on and the line in the composer: `refused: ! <command>:
<why>` — the session records no work dir, its work dir is gone, tmux did not
answer, the tmux clients did not answer, or no client (or two equally recent ones)
shows this app. The composer's address row names no seat for a `!` line, and
`@<seat> !text` sends the text to that seat as an ordinary ask. `!` while
browsing still steps to the next session that needs you; only a draft line that
starts with it runs.

## Ownership

Every `ae app` window writes the selected session on its own, in any pane or
outside tmux, holding no writer or admission lock while it composes: each window
keeps its own draft and its own binding to the session. Only a send or
`/close` takes the session's **writer lease**, the lock file `.console-writer.lock`, and then the
chat's admission lock, both without waiting; the lease stays with an ask until
its delivery ends, so one window's ask is admitted and delivered at a time per
session, in no promised order, and the five-open cap counts every window's
asks. The holding app writes `1 <session> <window index>` into the lease while
it holds it, which is what a refusal names. No key or
wheel notch moves the selection while writing; a click on a session row leaves
writing first. The composer names its target on its own row: `to <session> ·
<seat> (auto)`, or `(named)` once the draft names a seat with `@`, the seat a
line written now would reach, at home or anywhere else (the seat is chosen per
line, [Routing a line](#routing-a-line)). A stopped session,
one with no recorded `session_id`, one whose lead pair cannot be read and one
not recorded in this state root are read-only and say why.

Every way out of writing ends this window's entry and keeps its draft — `Esc`,
`^C`, a click on a session row, opening a seat view, and a read that finds the
session no longer proven — and Settings keeps it while open. An ask still being
delivered keeps the lease whatever happens to the writing. The view keeps the draft it
interrupted: `Esc` returns to the chat and `Enter` writes again to
continue it. Each ask and `/close` goes through the chat's
admission, which re-reads the meta's `session_id` and
lead pair against those this window's entry proved, then asks as `console:local` in that
session under the same five-open cap; the ask reaches the session's own recorded
tmux server, and one whose recorded server cannot be named is refused in the
delivery's own words. A session replaced under its name, a changed lead pair, a
meta that is gone or a server that cannot be named refuses the ask and ends the
writing: the locks are released and the composer is held as
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
composer reads `read-only · <why>` until a read proves the
session again; `<why>` names only what works for that reason, so a stopped
session reads `<name> is stopped; ae <name> resumes it`.

Each session keeps its own draft in memory, per incarnation, so
moving between sessions keeps it. Its lead pair is fixed the first time this
run of the app writes it: a different pair refuses as above, and the original
pair coming back writes again with the kept draft. A fleet read that proves a
session replaced under its name (a new `session_id`) or gone drops its draft
from memory, once, with one `ae` line `draft for <session> dropped: <why>`; an
id ae could not read drops nothing. The kept draft on disk comes back only when
writing starts into an empty composer, under the `Kept line, maybe already
sent` banner, which names a way to check the target pair from the app (the
Agents: `n` / `p`, `oo` opens a seat), never `prefix H`, which jumps the
host's own pair; a draft kept in memory wins, never merged. The owner `ae chat`
tries the lease inside its own admission: while an app is sending to its
session, it refuses its sends and `/close`. A window's `/close` withdraws the
session's newest open ask whichever window or bridge sent it, and every
window's lane shows the close as `you  <time>  closed an ask to <seat>` over
`<id>: <the ask's first words>`.

## Leaving

Every way the app ends, a stopped background reader included, restores the terminal and leaves one plain line
on the normal screen: `ae app closed - run: ae app (in a chat window, prefix h
reopens it)` inside tmux, `ae app closed - run: ae app` outside it. A reader
that stopped says so on the line above.

A restart into another installed core ([upgrade.md](upgrade.md#automatic-checks))
leaves no line. In a chat window tmux replaces the app in the same pane, and a restart
tmux refuses ends the app with `ae app: could not restart into ae <V> - <why>` above the
usual line. Anywhere else the app replaces its own process, so the shell you started it
from stays; an exec that fails keeps the app running with `could not restart into ae <V>
- <why>` on the hint row, and only when the terminal cannot be taken back does it end
with that line.

The restart keeps your view. Just before the respawn the old app sets one
session option, `@ae_app_view`: the selected session (with its uuid),
the seat highlight, the dragged sizes and the scroll. The new app, once it
proves it is the chat window of its session, reads the option and unsets it,
whatever it held. It uses it only for the same pane, its own version, within
60 s, and only on a fleet read that lists that session with that uuid. A
complete read that omits it, or lists another uuid, drops the view; an
incomplete one that cannot say waits for the next. Anything else, any key or
click before it applies included, leaves the default view. Settings, a viewed
seat, drafts and marked turns never carry: they hold a restart back. A core without this feature ignores the option, and a leftover
one is refused and cleared by the next app that reads it or dies with the
tmux server.

An app that execs carries the same line as an internal argument,
`--app-view <view>`, after your operands — not in the usage text, never typed
by hand — bound to its own process id instead of a pane, since exec keeps
it. The new app adopts it only for that pid, its own version and within 60 s,
under the same fleet-read rules; anything else, typed or stale, is ignored and
leaves the default view. A view that names another version means the exec
landed in a core the link did not name: that app shows the hint and never
restarts again. Only a newer core is handed the view, since an older one may
not know the argument.

## Residuals

- A `!` line runs with ae's own permissions, its command line visible in `ps`
  while it runs; outside tmux its output is discarded and nothing reports its exit,
  and inside tmux the popup's caveats are [the chat's](chat.md#run-a-command).
- Typing in the app counts as activity in the session that hosts it, so that
  session rises while you work in the app. A key typed in the same second a
  session was created leaves it unknown. A stray click counts as activity too.
- A list scrolled with the wheel keeps its row positions across a reorder, so
  it can show different sessions after one.
- The Instructions tab renders from the records NOW: a running seat received the
  text of its own launch time, so a config or ae upgrade since then shows here
  and not in that seat. The generic rules carry no seat's work dir or
  identity; the roster below them names each seat.
- Settings shows the launch-pinned values for `layout`, `quota`,
  `quota_every_secs`, `idle_nudge_secs` and `done_confirmations`: a running
  session keeps what it launched with even after the config changes.
- A SIGKILL or SIGTERM leaves the terminal in raw mode on the alternate screen
  (`reset` restores it). In the `chat` window `remain-on-exit` keeps the dead
  pane on screen and `prefix h` respawns it; any other tmux pane closes with
  the process. Every exit ae controls, a panic included, puts the terminal back.
- A lone `Esc` waits 50 ms to tell itself from an escape sequence.
- A restart keeps the view only for an untouched app: a key or a click
  before it applies, or a session that left or was replaced, shows the
  default view. A key typed between the last idle check and the respawn is
  lost. The hint follows the fleet read, so it can be up to 5 s behind the
  link.
- An exec restart into an OLDER core (a rollback) starts with a fresh view.
  A key read but not yet handled at the exec is lost, and a tmux query the
  app had in flight finishes as an orphan of the new app (one zombie process
  until that app exits). The internal `--app-view` argument is visible in
  `ps` while the new app runs: session name, uuid, highlighted seat and
  scroll.
- An app started by a core's own path while the link names another version,
  or one an upgrade overtakes before its first fleet read, only hints for
  that version and never execs into it: it cannot tell either from a link
  whose core is not the version the link names.
- `^Z` and `^\` are dropped and never suspend or quit.
- The panic hook that restores the terminal stays installed after the app
  leaves its screen; it would only restore the same mode again.
- Truecolour only; no 256-colour fallback.
- A session replaced under the same name can show its previous lane until the
  next fleet read notices.
- A journal or memo rewritten in place to the same length inside one
  filesystem timestamp tick (1 ms on Linux before multigrain timestamps, so
  before 6.13) keeps every identity field, and its old contents show until the
  file next changes.
- After `Esc` from an entry its admission refused, the composer can offer
  writing until the next fleet read reads the change; entering is refused again.
- `Enter` in the up to 5 seconds between a session's stop and the next fleet
  read still asks: the admission reads no liveness, only that read does.
- One very long turn is wrapped whole whenever any of its rows is on screen.
- A copy of one key stream read by a second app later than the first app's
  whole write session can submit twice.
- An app left writing, or with Settings open over a draft, keeps the owner chat
  and every other app from writing until it leaves writing or quits.
- A drag selects what the app drew, not the pane's text beyond it, and cannot
  cross panels or scroll for you: wheel first, then drag. Hold Shift (Option in
  iTerm) for the terminal's own selection. An upgrade restart does not wait for
  a selection.
- Dragging a border needs button-event mouse reports (`?1002h`) in SGR form;
  a terminal that sends only legacy X10 reports gets no drag. Inside tmux a
  binding that replaces the default `MouseDrag1Pane` keeps drags from the app.
- Dragged sizes are not remembered when the app quits.
- A copy is the whole recorded body for transcript turns, an admitted answer, an ask whose stored text was read and a short `say` line. An ask whose text could not be read, a bridge message, a not-delivered line and a card or needs-you line copy the journal's summary (line breaks flattened, 600 characters), a `say` of 3500 characters or more its capped summary, a bridge reply nothing (the record keeps no body) and an unadmitted reply stays a preview; the hint row says `preview only` for each. An answer whose body could not be read copies its summary with the reason.
- A turn is named by a 64-bit fingerprint of its record, so two identical records mark together. `[` / `]` stop at the oldest row the last frame produced; a second press after the next frame goes on. tmux keeps 50 buffers by default, so the 51st copy drops the oldest.
- A Stop answered in the app is checked against the tmux session id the menu
  was drawn with only by the stop's own owner, after the app already said
  `Stopping '<name>'.`; a refusal then shows on the asking client's status line.
- The unread cue is memory in the running app: a restart shows no marks, and a session met for the first time is shown as already seen. A session absent from one fleet read loses its mark and is seen afresh when it returns. A mark needs an answer strictly newer than the last moment the app showed that session selected, whole seconds, so an answer in the second you left it is not marked. It appears with the next fleet read (up to 5 s).
- The holder named in a writer refusal is the place the holding app's pane had when it took the lease (`<session>:<window index>`, as tmux answered), not a liveness proof: a crashed app leaves its record behind, so a refusal that meets it can name a place that has let go, and a window renumbered since is named by its old index. A holder outside tmux, one tmux did not answer for, one whose session name is not an ae session name, the owner chat's probe and an app of an older core leave no record: the refusal says `holder unknown`, as it does for a record cut short by an app still writing it.
- The refusal row is one line, clipped from the right: a chat column narrower than `not writing: an ae app is writing to <session> (held at <session>:<window>) · Esc browses` loses `Esc browses` first. `Esc` still browses.
