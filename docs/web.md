# Web

`ae web` shows the orchestrator's chat in a browser tab on the same machine. It is the app's chat
panel read the same way, not a second implementation: the page shows the same turns, the same
header words and the same coverage lines as `ae app` shows for the orchestrator session.

## Start and stop

```text
$ ae web
ae web: http://127.0.0.1:7480/#c=<32 hex>
ae web: open it in a browser on this machine; Ctrl-C stops it
```

- `ae web` runs in the foreground and listens on `127.0.0.1` only. `--port N` picks another port
  (1..=65535). A busy port refuses the start: `ae web: port 7480 in use; not starting`, exit 1.
- One `ae web` runs per state root. A second one refuses with
  `ae web: another ae web runs for this state root`, exit 1. The lock is `<state root>/web/lock`.
- Ctrl-C stops it. The listener's key, every link and every browser session end with it, so a
  restarted `ae web` prints a new link and an old tab must open it.

## The link and the session

The printed link carries a one-time code in its fragment (`#c=`), which the browser never sends in
a request. The page takes the code, removes it from the address bar and trades it once for a
session secret, which it keeps in the tab's `sessionStorage`.

- A code is good for 10 minutes and for one exchange.
- A session lives while it is used: 12 idle hours end it. At most 8 sessions live at once; a ninth
  ends the least recently used one.
- A page without a live session shows `no session: open the newest link from ae web`, and the
  terminal that runs `ae web` prints a fresh link (`ae web: new link …`). The listener prints at
  most one fresh link per 10 s, and none while the code of an earlier fresh link is unredeemed and
  unexpired. The code printed at start never holds a fresh link back. So a client that keeps
  asking cannot fill the set of kept codes and push out the code the owner is about to use. An
  owner who lost both links waits up to 10 minutes for the next one, or restarts `ae web`.
- A page that gets a `401` on its poll stops asking: the refusal that told it already printed the
  link. It asks again only after a code is redeemed in that tab, by a link opened in the tab's
  address bar (the page takes the code from the fragment without a reload) or by a reload that
  carries the link. A tab opened with no session asks once. A tab opened with a code the listener
  refuses asks nothing more. A tab that an older `ae web` loaded keeps asking until it is
  reloaded once.

## What the page shows

The header names the orchestrator session and its standing: `proven` (it runs and its tmux
session proves its identity), `stopped` (it holds the role but is not proven live), `absent` (no
session holds the role) or `gap` (the role could not be read), with the reason. Below it, the
chat: the newest 500 turns, oldest first, and one line per coverage gap. The page asks for
changes every 2 s while it is visible and every 30 s while it is hidden; an unchanged answer is
a `304` and costs no render.

Every text from an agent is shown as plain text, never as markup: control characters are removed,
a turn's text is capped at 64 KiB and a speaker or qualifier at 200 characters. An `ae web` one-time
code in any served text, `#c=` followed by 32 or more hex digits, is shown as `#c=…`, so neither
`GET /api/state` nor `GET /api/topic/<slug>` carries it. The journals, transcripts and memos keep
their text unchanged.

While the chat shows its newest turn, it stays at the newest turn when a turn arrives, the window
is resized or a narrow screen switches views. Scrolled up, it stays where it is.

## Topic side panel

A panel on the left lists the topics. **All** comes first and is selected when the page loads.
Below it come these groups; a topic is in the first one that fits, and an empty group is not shown:

1. **Needs you:** the state `needs`, or something waits for you on it: an open DECIDE item or your
   own open ask tags it. The topic leaves the group when its last wait ends, unless its state is
   `needs`.
2. **Moving:** the state `moving`.
3. **Pinned:** a pinned topic that fits no group above.
4. **Unreadable:** a filed topic (one with a title, a state or a review) whose state word cannot
   be read.
5. **Quiet, parked, closed:** every other topic.

Inside a group the topic with the newest activity comes first: the later of its last review and the
newest chat turn that tags it. The last group starts folded. Its heading opens or folds it, and a
reload folds it again; while folded, it still shows the selected topic. A row shows the state dot,
the title in at most two lines (the row's tooltip has the whole title, the slug, the state and the
counts), the unread number and the number of open DECIDE items that tag the topic. Opening the
last group scrolls it into view when it is cut off: whole when it fits, else its heading at the top.

Selecting a row shows the topic's chat, tags the draft and shows its card on the right. A topic in
the last group also opens from All, the Revisit line or the strip. A selected topic stays selected
when it moves to another group, also into the last one, for example when its last wait ends. Closing
is the exception (see **Closed topics**). The panel is 260 px
wide (220 px below 1100 px) and scrolls on its own. A narrow screen (up to 759 px) shows one view
at a time, chosen at the bottom: **Topics** (the panel), **Chat** or **Overview** (with a topic
selected, **Topic**). The page opens on Chat, and a row press shows the chat.

The **Topics** navigation region gives each group a list named by its heading and marks the
current row. Tab and Shift+Tab move through the controls. Up and Down on a row move focus
between visible rows, including across groups. Enter and Space select a focused row or toggle the
last group when its button has focus.

- **Pins.** The button at the top of a topic's card pins or unpins it. A pinned topic is marked
  with a star and stays out of the last group, whatever its state, until you unpin it. When you
  unpin the selected topic and no group above fits it, it stays selected until you select another
  one or it closes. At most 32 topics are pinned: then the button of another topic is disabled and
  the card says why. Your browser keeps the pins (see [Security](#security)). A pin can outlive its
  topic: when the limit is reached and no pinned topic shows, clearing the site's data in the
  browser frees the pins.
- **Closed topics.** A closed topic is in the last group unless it is pinned or something waits for
  you on it (see **Needs you**). When the selected topic closes with neither, the page shows All,
  also when you had opened it from the last group. It also shows All when the last wait of a
  selected closed topic ends and the topic is not pinned. A topic that is already closed and in the
  last group when you open it stays selected until you select another one.
- **Unread.** A row shows the number of turns with its tag that arrived since you last had the
  topic selected in this page; selecting it clears the number. Your own turns do not count. A tag
  that a turn gets after it arrived does not count either: a new topic can tag older turns. A turn
  that first shows up more than 120 s older than the newest one already seen is taken for an old
  one that came back and does not count, even when it is new. The counts live in the page: after a
  reload every topic starts at zero, and the turns the page reads first are not unread. The folded
  group shows no total.
- **Scrolling.** The panel scrolls by touch, with a trackpad or with the mouse wheel. The row you
  select scrolls into view, and stays in view when rows move around it. When you scroll away from
  it, the panel does not pull you back. With many topics, **All topics** and its search reach
  every one.
- **All** shows the whole chat. Its panel holds the new-topic line first, then the Revisit line,
  every DECIDE item under **Decisions**, **All topics** with a search over slug and title, and the
  desk's gaps. **All topics** lists every topic in the side panel's order: its groups, the folded
  one too, each newest activity first. A decision that tags no topic is answered there; a tagged one is listed with a link
  to each tag's topic.
- A selected topic shows the chat turns tagged with it, and the right panel its card with the
  decisions that wait on it. A turn is tagged with each `#<slug>` of a desk topic in the first 512
  bytes of its text, read as the board reads tags; a tag cut off by that limit does not count.
  `GET /api/state` names them in each row's `tags`. A topic with no tagged turn says so.
- `#reserved` marks a DECIDE item and names no topic.

- A topic is a memo checkpoint under `topic-<slug>`, the slug `[a-z0-9][a-z0-9-]{0,47}`. One
  checkpoint line reads `title: Web UI | state: moving | what: … | last: … | next: … | src: … |
  close: …`; `\|` is a literal pipe and `\\` a backslash, other names are ignored. The states are
  `needs`, `moving`, `quiet`, `parked` and `closed`.
- The memos come from the orchestrator seat's session and from every kept archive that the seat
  published, renamed or not. Each field shows the newest record that carries it; later
  incarnations rank above earlier ones and, within one memo, later lines above earlier ones. The
  other records are the card's history, **Earlier records**, each shown as its fields, one
  `name: value` line per field.
- The card's header names the session and time of the topic's newest record once; its tooltip
  holds that incarnation's uuid. A field or an earlier record names its own session and time only
  when its record has another origin than the header's.
- **Last reviewed** is the time of the topic's newest record, whatever it carries.
- A topic needs you when its state is `needs` or when something waits for you on it: a DECIDE item
  on the orchestrator's board, or your own open ask from `ae chat`, whose text tags it with
  `#<slug>`. The card of a tagged topic shows each DECIDE item that tags it with its answer (see
  [Owner lines](#owner-lines-from-the-desk)).
- A checkpoint without a readable state is shown as an unreadable checkpoint with its raw text.
- A memo over 8 MiB shows none of its topics, and the column names it until `ae web` restarts.
  Archives that are being written, damaged or not proven to come from the seat are counted, not
  read.

Clicking a row in **All topics** selects the topic. `GET /api/topic/<slug>` serves its card from
the same snapshot as the chat.

## Revisit and the walk

All holds the Revisit line: one quiet topic by name, with its next step (else its last
state, else what it is), its last review time, **Open topic**, **Next** and **Walk the quiet
topics (n)**. On a narrow screen the same line also sits above the chat while All is selected. The
shown topic lives only in the browser tab, and one that stops being quiet falls back to the first.

- Quiet topics are ordered by last review, the oldest first. A review time that is no
  `YYYY-MM-DDTHH:MM:SSZ` (empty or hand-edited) counts as the oldest, and equal times go by slug.
  `desk.revisit` of `GET /api/state` lists them in that order and each entry's `next` names the
  following slug, the last one pointing at the first, so Next wraps and a single topic points at
  itself.
- The walk shows one quiet topic at a time with its what, last and next, and **Keep**, **Ask for a
  next step**, **Close with one line**, **Skip** (only when the topic is no longer quiet) and
  **Stop**. A card has **Close with one line** too, the same dialog for one topic, for every Quiet,
  Moving or Parked topic. A topic under **Needs you** (a request waits for you or for the
  orchestrator, or its state says it needs you), a closed one and an unreadable one cannot be
  closed. `GET /api/topic/<slug>` carries `verdict`: `null` for those, else the lines
  `keep`, `ask` and `close` (the start of the closing line, `#<slug> close `, ending in a space) and
  `reason_max`. The page reads the card again before each send; a topic that left the quiet group
  shows `no longer quiet` with Skip and Stop only.
- A verdict is one ordinary owner ask in the composer's own path (cap of five open asks, claim,
  receipt): `#<slug> keep`, `#<slug> ask` or `#<slug> close <reason>`. The page holds no store
  of its own and the server never validates a verdict. The reason is trimmed and has 1 to
  `reason_max` (200) Unicode scalar values, with no control character and no line break, so a
  verdict stays one line. The page checks every input and cancels a paste with a line break.
- The orchestrator seat turns a verdict into a topic record and answers the ask: `review: keep` or
  `review: ask` moves the review time and nothing else, and `state: closed` with `close: <reason>`
  (and `review: close`) closes the topic. The record comes first, so the topic moves in one step.
  Until the ask is answered the topic shows under **Needs you** with an **Owed to you** wait, and
  it is not offered a second verdict. A reason that tags another topic (`#other`) also makes that
  topic wait until the answer.
- **This page does nothing visible until the orchestrator's charter teaches the topic records and
  the verdict lines.** The charter is not part of `ae web`. A seat that writes `status:` or prose
  in `state:` leaves every topic unreadable, so no topic is Quiet: the Revisit line, the strip
  above the chat and the walk do not appear. A seat without the verdict rule reads
  a verdict line as a plain chat message and writes no record.
- A server receipt settles a step. `sent` and `unknown: may have landed, not resent` are final:
  the walk shows the outcome and moves on, and the end screen lists each topic. A `refused`
  receipt stays on its topic with **Try again**, which sends a new draft; the owner may edit the
  reason first. A refusal is a definite answer of the server for that draft: nothing was sent. An
  absent answer is not one, see **Check again** below. A single close from a card dismisses its
  dialog on a `sent` receipt and leaves the result, `Closing line sent for #<slug>: …`, on that
  card as a status line. When the orchestrator changes while a step is open, the walk sends no new
  line: the open attempt still checks its own receipt with its first body, and every later step
  waits until the walk is opened again. With five asks open the console refuses with `cap-full: 5/5 …` and the walk waits
  at that step: answer or withdraw one open ask, then press **Try again**.
- Without any receipt (about two minutes, or ae web not reachable) the step shows **Check again**,
  which posts the same body and draft id once more. Until a receipt arrives, **Stop**, Escape and a
  click outside the dialog do nothing. Reloading the page is the only other way out; it ends the
  walk and the page can no longer say what happened to that draft.

## Sending

The composer under the chat sends one owner ask to the orchestrator's main seat, the same ask `ae
chat` opens: it passes the console's own admission, so the cap of five open asks holds and a cap
refusal shows verbatim. Its opening also records `origin` as `web:local`, and the actor stays
`console:local`. The label grants nothing: no authority check reads it, and only
`requests --verify` prints it, as its `origin:` line
([origin on an event](internals/events.md)).

With a topic selected, a chip names the tag, and the draft is sent as `#<slug>: <draft>`; the
chip's **×** sends that one draft without it. While an attempt is open the chip shows the tag that
attempt carries. Ctrl+Enter or **Send** sends; selecting a topic keeps the draft and the focus.

- Every draft gets one draft id from `GET /api/nonce`. The page posts the text, the draft id and
  the route it shows (`POST /api/send`) and repeats that same post until it gets a receipt, so a
  lost answer or a restarted listener never sends the draft twice. When no receipt comes in about
  two minutes, or ae web is not reachable, **Check again** posts the same body once more; the
  draft cannot change until an answer settles it.
- The receipt names the request id and one outcome: `sent`, `refused` with the console's words,
  or `unknown: may have landed, not resent` when input may have reached the seat. A sent or
  unknown draft leaves the composer; a refused one stays to be edited and sent as a new draft.
- The route is the orchestrator session, its lead pair and each seat's launch, signed with the
  listener's key. A draft is carried only to the route it was written against: when the role or
  the pair changed, the answer is `the orchestrator changed, reload` and nothing is sent.
- A draft id is good for 24 hours. An older one, or one from a stopped listener that never sent it,
  answers `expired draft id: send it as a new message`.
- One draft is carried at a time and two more wait; the next one answers `busy, retry`, and the
  page retries it.
- Before any input, the send worker writes a claim for the draft id under
  `<state root>/web/claims/` (mode 0600, synced to disk) and settles it with the outcome
  afterwards. A claim without an outcome is never carried again: its retry answers unknown. When
  no claim can be written, nothing is sent and the next retry answers `refused` with the reason.
  Claims older than 7 days are removed.
- An installed `ae web` follows a change of the command link. When the link changes to another
  core and automatic upgrades are on, it replaces itself with that core on the same port once no
  draft is queued or carrying; meanwhile a send answers `busy, retry`. A different version that
  the link already named when the listener started is never followed. The new core has a new
  key, so it prints a new link, and a draft that was sent is answered from its claim. A failed
  switch is printed once per version
  (`ae web: cannot follow version <v>: <why>; still serving <old>`) and the old core keeps serving.

## Owner lines from the desk

Two more lines go to the orchestrator's main seat through the same send path, the same draft id and
the same admission as the composer, so the cap of five open asks holds and a cap refusal shows
verbatim.

- **Answer a decision.** A DECIDE item is answered on the card of each topic it tags, or on All
  when it tags none. The page sends one chat line, `D<n> <answer>`: for example the item `D67` and
  the answer `1` send `D67 1`.
  - A text `Question? 1) a 2) b (recommend 2) #slug` offers the options as one choice. The options
    are numbered `1)` to at most `9)` in order, each at the start of a word, and `(recommend N)` or
    `(Empfehlung N)` may follow the last one. Trailing `#slug` words belong to no option. The
    answer is the chosen number. **Pick the recommended** only selects it.
  - Any other text takes a typed answer of 1 to 120 Unicode scalar values, with no control
    character, line break or paragraph break, and no space at either end (the page trims it as
    Rust's `str::trim` does).
  - **Answer** stays off while nothing valid is chosen or typed.
  - `GET /api/state` names each item's `choices` (`null` for a typed answer) and `reserved`.
  - **Talk it over** sends a question or objection instead, as the ordinary chat line
    `#<slug> D<n>: <text>` (without `#<slug> ` on All), with no countdown and no decision proof.
    It does not answer the item.
- **Reserved items.** An item tagged `#reserved` is shown and cannot be answered here: confirm it
  in the orchestrator pane or on Telegram. This is the page's rule, not a server one.
- **No proven orchestrator.** Without a proven orchestrator, **Pick the recommended**, **Talk it
  over**, the talk line's **Send** and a card's **Close with one line** are off, and they say the
  composer's words, `No proven orchestrator to send to`. An open attempt keeps **Check again**.
- **Ask for a new topic.** The box at the top of All's panel takes a title of 1 to 80 scalar values
  and suggests a slug: umlauts folded, lowercase, every other run of characters one hyphen, at most
  48 characters. **Edit slug** turns the slug into a field of its own, and it stays one. The line is
  `#<slug> new <title>`, shown before it is sent. A slug that does not fit `[a-z0-9][a-z0-9-]*` or
  that is a topic already cannot be sent. After a receipt the page shows `asked for #<slug>` until
  the orchestrator has filed that topic (a row with a title, a state or a review; the bare row
  that the open ask itself makes does not count), or for 10 minutes at most.
- **Undo, 5 s.** A press sends nothing. The line counts down for 5 s with an **Undo**, and only
  then does the page ask for a draft id and post. An Undo, a hidden page, a changed orchestrator
  route, a changed decision and a new topic whose slug has become a topic cancel the line before
  it is sent. The press keeps the route it was
  made against and never retargets.
- **Proof against the board.** The state names each decision with a `bind`, an HMAC under the
  listener's key over the item and its stored text. The post carries `decide:{item,bind}` and the
  line must be exactly `<item> <answer>`. The server refuses with `409 the decision changed,
  reload` and claims nothing when the board no longer holds the item with that text. The send
  worker proves it again beside the route, so a board replaced while the draft waited settles as
  changed and nothing reaches the seat. Both are point-in-time proofs: the board can still change
  between the last proof and the seat reading the line. A card whose text differs from the
  board's shows its text with **Answer** off until the next refresh.
- **Check again.** A lost receipt keeps the body and the draft id. **Check again** posts the same
  body once more and the claim answers with the outcome that landed.

The orchestrator charter now teaches these lines; that change waited for the owner's decision D20.
A web `sent` receipt confirms delivery to the orchestrator. The orchestrator removes a DECIDE item
only when its relay reports sent. On not-delivered or uncertain, it keeps the item, reports the
outcome, and does not resend automatically. An older charter may still read the lines as plain chat.

## Security

- The listener binds `127.0.0.1` alone and answers only the `Host` values `127.0.0.1:<port>`,
  `localhost:<port>` and `[::1]:<port>` (anything else: `421`). Each `POST` must carry the matching
  `Origin` (`403`). Every `/api/` route but the code exchange needs the session secret as a bearer
  header (`401`); no cookie exists, so another site's page cannot ride on a session.
- The page keeps two things in the browser. The session secret is in the tab's `sessionStorage`,
  as above. The pins are in `localStorage` under the key `ae-web-pins`: a JSON list of topic slugs
  and nothing else, no chat, card or memo text. The browser keeps it for this profile and this
  address (host and port), also after `ae web` stops, so a later page served at the same address
  can read it, and a slug can give away what a topic is about. Clearing the site's data in the
  browser removes it. The page reads the list as untrusted: it takes only valid slugs, each once,
  at most 32. When the browser refuses to store the list, the pins live in that page only until
  it is reloaded, and the page stops reading and writing the stored list.
- Every answer carries `Cache-Control: no-store`, a content security policy without inline script
  or style, `nosniff`, `no-referrer` and same-origin opener and resource policies, and closes its
  connection.
- A request must arrive whole within 10 s and an answer must leave whole within 10 s; a slow
  client costs one of four readers for that long, and a full queue of 16 answers `503`.
- The listener reads only a target that is `/` followed by visible ASCII. That is close to
  [origin-form](https://www.rfc-editor.org/rfc/rfc9112#section-3.2.1), but a raw `#` passes. An
  absolute-form target such as `GET http://127.0.0.1:7480/ HTTP/1.1` gets `400`: a deliberate
  deviation from [RFC 9112 §3.2.2](https://www.rfc-editor.org/rfc/rfc9112#section-3.2.2), which
  asks a server to accept it, because no browser sends absolute-form to an origin. A `GET` whose
  `Content-Length` is positive gets `400`, or `413` when the length is over the 64 KiB body limit.
- The page's icon is `/assets/icon.svg`, a static image served like the other files, so the
  browser asks for no `/favicon.ico` and logs no error for it. A `data:` icon would break the
  content security policy (`img-src 'self'`). Every form field has a name.
- The page's three library files (Preact 11.2.0, its hooks and htm 3.1.1) are vendored and pinned
  by SHA-256 in `src/l5_ui/web/assets/VENDOR.md`; there is no build step. Their licence texts
  (MIT for Preact, Apache-2.0 for htm) are served unchanged at `/assets/licenses.txt`, which needs
  no session.

## Browser smoke

The gate has no JavaScript engine, so it proves the wire and pins the page's sources, but it
never runs the page. `just web-smoke` does, on demand: it is a required receipt at a web freeze
and before each web release, and it is never part of `just test`, `just test-affected`, the Mac
subset, mutants or CI. The test (`tests/it/deliver/web_smoke.rs`) is `#[ignore]`d and runs only
when `AE_WEB_SMOKE=1`, which only the recipe sets.

- The run starts one listener on `127.0.0.1` over a scratch state root whose orchestrator is the
  delivery tests' fake TUI, and one headless Chrome with a scratch profile and home. Chrome is the
  absolute path in `AE_WEB_SMOKE_CHROME`, else `/usr/bin/google-chrome` on Linux and
  `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` on macOS; the recipe never looks
  it up through `PATH`.
- It drives one tab over the DevTools protocol. Every click is a real pointer press and release
  at the control's centre, after a hit test proves the press lands on that control.
- Four cells, each a fresh load: 1440x900 light, 360x740 light, 360x740 dark and 1440x900 dark.
  Each cell checks that
  the modules load under the strict CSP header with no exception, console error, dialog or CSP
  violation (`modules`); that a topic titled `<img src=x onerror=alert(1)> & </p>` shows that
  literal text and the page holds no `img` and no inline script (`hostile-literal`); the scheme's
  page background (`theme`); and no horizontal scroll with the draft box inside the viewport
  (`fits`).
- A wide cell types a draft, clicks a topic row and checks that the card opens while the draft
  keeps its text (`draft-kept`) and the focus (`focus-kept`). A narrow cell takes the same draft
  through Overview, the topic and back to Chat (`draft-kept`).
- Every cell checks the Revisit line: the oldest quiet topic shows, Next cycles through the
  fixture's three quiet topics and wraps (`revisit`), and on a narrow screen the walk dialog fits
  the viewport with the focus on Keep (`walk-fits`).
- The 1440x900 dark cell selects All, clicks Send and waits for the `sent to …` receipt with its request id;
  the fake TUI must have received the draft once (`sent`). It then walks the three quiet topics,
  each verdict reaching the fake TUI once as the exact line (`walk-keep`, `walk-ask`,
  `walk-close`), and runs the edge cases against a held send endpoint: no dismissal while a
  carry is open, an unknown receipt advancing, a cap refusal with Try again (`walk-dismissal`,
  `walk-cap`), the card's close button for ineligible topics (`walk-card`) and the reason's
  bounds, 200 accepted and 201, blank, line break and pasted line break refused (`walk-reason`).
- The same cell then checks three guards in one page (`guard-skip`, `guard-selection`,
  `guard-route`): a step without a receipt keeps **Check again** and its draft id when its topic
  leaves Quiet, so Skip, Escape and a click outside do nothing; the Revisit line keeps its topic
  when another one leaves and returns; and a changed orchestrator route stops new sends while the
  open attempt checks its receipt with its first body. The checks replace only the replies of
  `/api/send` and the route id of `/api/state`.
- After the four cells, one signed-out stage runs in the same tab. Each load is a full document
  load, and the page's own requests are counted by a script that runs first in each new document
  and counts the fetches the page makes to `/api/state` (the browser's resource timing does not
  list a refused fetch). A tab that
  holds a dead session secret asks `/api/state` once, shows the refusal and clears the secret, and
  then asks nothing for 12 s although three `visibilitychange` events wake it
  (`signed-out-quiet`). A tab with no session asks once, and one with a refused code asks nothing
  (`signed-out-bare`, `signed-out-code`). The three loads made the listener print exactly one
  fresh link (`signed-out-links`). That link's code, set in the halted tab's address bar as a
  fragment move, signs the tab in without a reload and brings the poll back
  (`signed-out-resume`).
- After the signed-out stage, a compose cell runs the owner lines on a second scratch listener, on
  the two phone sizes and then in 1440x900 dark (`s3a-*` checks): touch targets of at least 44 px
  and 16 px inputs, the line and slug rules against the same cases as the server, Undo and a hidden
  page, a card that is behind the board, a board that changes during the countdown, a route that
  changes, a lost receipt and Check again, the chip of a new topic and a cap refusal. While the
  new topic waits, the rows its open ask adds appear without moving its form or **Check again**.
- After the compose cell, the topic selection cases run on listeners of their own in 1440x900
  dark (`t1-*` and `tab-focus-kept`): which topics have a row, the chat each one shows, a row
  press that keeps the draft and its focus, choices with and without a recommendation, reserved
  items, an answer by choice with Undo beside a discussion, a board that changes under a choice,
  and tagged, untagged and verdict-like drafts as the fake TUI receives them.
- After the selection cases, the lifecycle cases run on a listener of their own, in 1440x900 dark
  and 360x740 dark (`t2-*`): the mouse wheel over the panel, the selected row in view and a
  scrolled panel left alone, unread counts across a new conversation, pins across a reload and
  another tab's write, a closed topic's row, and a damaged or hostile stored pin list.
- The side panel cases run on a listener of their own (`t4-*`): the groups (also under open
  asks and after one closes), their order and the fold, newest activity first, a long hostile title
  in two lines, the need colour and its contrast in both schemes, a topic opened from the folded
  group, the columns at 1440, 1000 and 760 px, and the three views at 360x740. The `t4c-*` cases
  end them: the selected topic that closes shows All, and the one whose last wait ends stays
  selected in the folded group.
- The Topics accessibility cases follow (`t5-*`). They check the landmark and named
  lists in Chrome's accessibility tree, the fold and current row, and keyboard navigation and
  activation. They check focus when a row moves or leaves and when the draft has focus.
- The desktop polish cases finish the run on a listener of their own (`p1-*`): the chat's newest
  turn across a resize and a view switch, the opened last group in view, **All topics** in the
  panel's order, earlier records as fields, the card's provenance once, a name on every form
  field, and the send controls off without a proven orchestrator. Every cell's console check
  counts a `/favicon.ico` error like any other, and each wide cell checks the page's icon
  (`favicon`).
- Each cell leaves a screenshot, and the run its log, in `.local/web-smoke/<UTC stamp>/`.

`just web-smoke plant` serves `app.mjs` without the topic row's mousedown `preventDefault`, the
guard that keeps the focus in the draft. The recipe passes only when that run fails at
`focus-kept (1440x900 light)` and at no other check. `just web-smoke plant-walk` serves `app.mjs`
with the walk's Keep sending the Ask line and passes only when the run fails at `walk-keep
(1440x900 dark)` and at no other check. `just web-smoke plant-undo` serves `app.mjs` with the Undo
of an owner line hiding the line while its countdown still sends it one second late, and passes
only when the run fails at `s3a-undo (1440x900 dark)` and at no other check. `just web-smoke
plant-tab` serves `app.mjs` without the side panel row's mousedown `preventDefault` and passes
only when the run fails at `tab-focus-kept (1440x900 dark)` and at no other check.
