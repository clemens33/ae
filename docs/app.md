# The app

`ae app [session]` draws the fleet as a terminal UI: every session in a
sidebar, the selected session's Overview or Agents beside it, and the selected
session's chat on the right. It is a second view of the data [the chat](chat.md)
already reads, through the same owners, and it writes nothing into any session
but the asks you type into the selected one; opening a seat only moves a tmux
client ([Opening a seat](#opening-a-seat)). `ae chat` is unchanged beside it.

It needs a terminal on both stdin and stdout. Without one it prints
`ae app draws a terminal UI; use ae chat to print the lane` on stderr and exits 1.

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
  stops the scroll. The composer is its address row and one input row while
  browsing; while writing the input grows with the draft to ten rows, taking
  them from the lane, which keeps at least three rows: a short pane shrinks
  the composer first, to one input row at least. A resize sizes it again, and
  leaving writing gives the rows back.

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
only when a frame is drawn: a key or click always acts on the row it showed,
even one the terminal splits over several reads; a click whose frame is gone
when it completes is not taken, and the hint row says so.
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
| `1`-`9` | select that session |
| `j` / `k`, `↓` / `↑` | next / previous session |
| `!` | the next session that needs you |
| `Tab` | Overview / Agents |
| `oo` | open the highlighted seat's pane: the first `o` only arms (`o again to open <seat>`), the second within 2 seconds opens that same seat |
| `n` / `p` | highlight the next / previous seat the tab body shows |
| `PgUp` / `PgDn` | scroll the chat a page |
| `[` / `]` | mark the next older / newer turn; with none marked, the newest turn the chat shows. At the oldest or newest turn the frame produced nothing moves and the mark stays |
| `y` | copy the marked turn ([Copying a turn](#copying-a-turn)) |
| Click a turn | mark it; clicking the marked turn again unmarks it |
| Click session row | select that session |
| Click a seat row | highlight that seat |
| Click Overview / Agents | show that tab |
| Wheel over chat or a tab body | scroll three rows per notch, also while writing |
| Wheel over the session list | scroll one session per notch, also while writing |
| Drag a border | resize the sidebar or the session list, also while writing |
| Click composer | write the selected session, when no other app writes it |
| `Esc` | back to home |
| `Enter` or `i` | write the selected session, when no other app writes it |
| `s` | open Settings |
| `?` | open Settings on its Keys tab, the list of every key |
| Click the gear at the keys row's right end | open Settings, also while writing |
| Click a Settings tab title | show that tab, like `Tab` and `1`-`5` |
| Click `Esc close` at the Settings title row's right end, or the gear | close Settings, like `Esc` |
| `qq` | quit: the first `q` only arms (`q again to quit`), the second within 2 seconds quits |
| `^C` | quit, at once |

No single printable key quits or moves your client: an armed `q` or `o` is
disarmed by any other key or when its 2 seconds pass, so typing a word
while browsing leaves the app up and no key reaches another pane. The keys
row is the mode word (`browse`, `write`, `held` or `settings`), at most one
hint where its key works (`? keys` while browsing, which lists every key;
`Esc close` in Settings; none while writing or held, which take every key),
and at its right end `ae <version>` and the gear. Narrowing drops the version
first, then the gear.

A paste while browsing is never read as keys. On a writable session it starts
writing with the paste as the draft, every fragment of it kept and nothing in
it acting: a pasted line break is draft text, and the draft is sent only by
your own `Enter`. On a read-only session or with the composer held it takes
nothing, and the hint row says why for a few seconds (`paste not taken: …`);
the Settings overlay swallows a paste silently.

Writing: the draft is the chat's own input — a line asks the speaker,
`@<seat> text` either lead-pair seat, `/close` withdraws your newest open ask,
five asks open at most; see [Input](chat.md#input). The composer shows the
whole draft as the chat does — the same wrap, each pasted line break its own
row, all of it under the address row from the column's left edge, and past its rows the window
around the cursor with `… +N lines above` and `… +N lines below` among them
(no markers below three rows) — and the terminal cursor sits where the next
character goes. The cursor shows only while writing with the composer drawn:
browsing, a held composer and Settings hide it. The address row, every draft row
and the note row under them are one click target. `Enter` sends, `Esc` goes back to browsing and keeps
the draft. `^C` over a non-empty draft only arms (`^C again to quit - the draft is lost`): a second within 2 seconds quits and any other key disarms; over an empty composer it quits at once. `/open <seat>` opens that seat of the selected session
([Opening a seat](#opening-a-seat)). While an ask is being delivered the hint row reads `sending to <seat>…`
and nothing behind it acts until it ends. Each outcome shows once, as an `ae`
line in the lane of the session it is about (an unconfirmed or undelivered ask
as its own lane line, with the hint row saying the same until the lane has it); a line about a session no longer recorded shows in
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
draft, then selects the row or tab (a turn: marks it). Blank rows and the "more" row do nothing.
A press within a cell of a border grabs it unless a row or tab is drawn there.
Any key, paste, press or wheel notch ends a drag whose release never arrived,
then acts as it always does.

## Opening a seat

`o` takes your tmux client to the highlighted seat's pane, in any session you
selected, home or not; `/open <seat>` in the composer does the same for a named
seat of the selected session. It is the proof and the guarded select `ae chat`'s
`/open` makes, plus one tmux client: nothing is typed into the seat and no
record is written.

The highlight is a `▸` (`>` with `icons = off`) in the first column of a seat's
rows in the tab body: on Agents every seat, on Overview each **Waiting on you**
entry (both rows of it). It starts on the first seat drawn, `n` / `p` step it
through the drawn seats and wrap, a click on a seat row puts it there, and
changing session or tab resets it. Rows scrolled out of the body are not
stepped to; wheel first. `Enter` still writes.

A key acts on the seats the last frame drew, as it drew them: the seat's slot
and name, and the session incarnation they were read from. A roster that moved
the seat to another slot, a session replaced under the same name, a pane
restamped for another agent, a dead pane or one the server no longer lists in
the session refuses; so does a selection that changed since that frame
(`the view changed`). Nothing is selected on any refusal.

Which client moves. Only a tmux client whose ACTIVE pane is the app's own pane
can have pressed the key.

- The seat is in the app's own session: tmux moves the session, so everyone
  viewing it follows and no client is chosen. At least one client must show the
  app.
- The seat is in another session: that one client is switched to the seat's
  session, then its window and pane are selected. With several clients on the
  app, the one with the newest input wins; two last active in the same second
  refuse (`two clients showing this app were active in the same second`) and
  move nothing. A control-mode client never records input, so two of them always
  tie.
- No client shows the app (`no tmux client is showing this app`), or the app
  runs outside tmux (`ae app is not running inside tmux`): refused.

A stopped session refuses and names `ae <session>`, which resumes it; the app
never resumes one. A session recorded on another tmux server than the one this
app's pane lives on, or whose server ae cannot prove, refuses the same way
(`run ae <session>`). The server is the app's own (`$TMUX`), never the one a
launch would use.

Getting back. The `ae` line in the session's lane says how: `opened <seat> in
<session> - prefix h opens its chat, prefix L goes back` (`prefix h returns` in
the app's own session), and the moved client is shown a short reminder on its
status line for a moment. `prefix L` is tmux's own last-session key; `prefix h` is
ae's. No binding is added.

Named limits. Other clients already attached to the target session follow its
window selection (tmux session state, as the fleet picker does). The client
list is read last, right before the move; a client that detaches in that
instant makes the move fail with `uncertain`. The cursor steps only through the
rows drawn.

## Copying a turn

The app captures the mouse, so a drag selects nothing. To copy one turn out of
the chat, mark it and press `y`.

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
still loaded. The hint row says what happened for a few seconds, and never
that the clipboard was reached, because tmux gives no acknowledgement:

| Hint row | Meaning |
|---|---|
| `copied N lines to the tmux buffer; terminal clipboard if your terminal allows it` | loaded, with a client named |
| `copied N lines to the tmux buffer only - <why>` | loaded, no client named: `no tmux client is showing this app`, `two clients showing this app were active in the same second`, `the tmux clients did not answer` or `the client name failed its grammar` |
| `copied N lines (preview only: <why>) to the tmux buffer…` | the text is not the whole body ([Residuals](#residuals)) |
| `copy failed: …` | tmux refused the load, or the app is not inside tmux (then it says to hold Shift/Option and drag to select) |
| `no turn marked - click one or press [ ]`, `nothing to copy: that turn has no text`, `that turn changed or left the lane - mark it again` | nothing was copied; the last one also clears the mark |

The terminal must take OSC 52 and let tmux use it. iTerm2: Settings > General >
Selection > "Applications in terminal may access clipboard". Terminal.app has
no OSC 52. Alacritty with `TERM=alacritty` needs
`set -as terminal-features ',alacritty*:clipboard'` in tmux (with `TERM=xterm*`
tmux's default `terminal-features` already carry the clipboard). The paste
buffer works regardless.

## Settings

`s` opens the read-only Settings overlay over the whole pane, as does the
gear (`*` with `icons = off`) at the keys row's right end — the gear also
while writing, where `s` stays draft text; `Esc`, `q` or `s` closes it, `^C`
quits. While open the overlay owns every key and click: `Tab`, `1`-`5` and a
click on a tab title switch tabs, a click on the drawn `Esc close` label or on
the gear closes it like `Esc`, `j` / `k` and `↓` / `↑` scroll a line, the
wheel three rows per notch, `PgUp` / `PgDn` a page, everything else is
swallowed (every other click too), and a draft being written is kept untouched
until it closes. Five
tabs: Keys lists every key of browsing, writing, a held composer, Settings and
the mouse
(`?` opens it while browsing); Quota shows the same scope rows `ae quota` prints; Config shows every
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
when empty — or a line saying there are none; then each generic rule text a
launch hands out, ONCE, under its audience — every seat, workers, the lead
pair — as the one renderer produces it now, with `<session>`, `<helpers>` and
`<owner line>` standing for what a launch fills in per seat; then one line per
roster seat, `name · slot · lead` or `worker`, naming the rules it receives.
Everything wraps to the pane. A session, config file or seat ae cannot render
from shows a named `gap` instead of a guess: no records, an unreadable meta or
config (the whole tab), an unknown slot.
Quota reads again every refresh while open; config, about and instructions read
once per open. A torn config shows one honest row naming the file. The
Instructions head states the residual below: this is today's render, not what
a running seat was handed.

## Ownership

An app writes the selected session only while it holds that session's
**writer lease**, the lock file `.console-writer.lock`: one app at a time per
session, in any pane or outside tmux, and at most one lease per app. No key or
wheel notch moves the selection while writing; a click on a session row leaves
writing first. The composer names its target on its own row: `to <session> › <speaker>` at
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
composer reads `read-only · <why>` until a read proves the
session again; `<why>` names only what works for that reason, so a stopped
session reads `<name> is stopped; ae <name> resumes it`.

Each session keeps its own draft and speaker in memory, per incarnation, so
moving between sessions keeps both. Its lead pair is fixed the first time this
run of the app writes it: a different pair refuses as above, and the original
pair coming back writes again with the kept draft. A fleet read that proves a
session replaced under its name (a new `session_id`) or gone drops its draft
from memory, once, with one `ae` line `draft for <session> dropped: <why>`; an
id ae could not read drops nothing. The kept draft on disk comes back only when
writing starts into an empty composer, under the `Kept line, maybe already
sent` banner, which names a way to check the target pair from the app (the
Agents tab: `n` / `p`, `oo` opens a seat), never `prefix H`, which jumps the
host's own pair; a draft kept in memory wins, never merged. The owner `ae chat`
tries the lease inside its own admission: while an app writes its session, it
refuses its sends and `/close`.

## Leaving

Every way the app ends, a stopped background reader included, restores the terminal and leaves one plain line
on the normal screen: `ae app closed - run: ae app (in a chat window, prefix h
reopens it)` inside tmux, `ae app closed - run: ae app` outside it. A reader
that stopped says so on the line above.

## Residuals

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
- Mouse dragging does not select text in the app. Hold Shift (Option in iTerm)
  for terminal text selection; inside tmux an ordinary drag goes to the app.
  One turn copies with [Copying a turn](#copying-a-turn).
- Dragging a border needs button-event mouse reports (`?1002h`) in SGR form;
  a terminal that sends only legacy X10 reports gets no drag. Inside tmux a
  binding that replaces the default `MouseDrag1Pane` keeps drags from the app.
- Dragged sizes are not remembered when the app quits.
- A copy is the whole recorded body for transcript turns, an admitted answer and a short `say` line. An ask, a bridge message, a not-delivered line and a card or needs-you line copy the journal's summary (line breaks flattened, 600 characters), a `say` of 3500 characters or more its capped summary, a bridge reply nothing (the record keeps no body) and an unadmitted reply stays a preview; the hint row says `preview only` for each. An answer whose body could not be read copies its summary with the reason.
- A turn is named by a 64-bit fingerprint of its record, so two identical records mark together. `[` / `]` stop at the oldest row the last frame produced; a second press after the next frame goes on. tmux keeps 50 buffers by default, so the 51st copy drops the oldest.
