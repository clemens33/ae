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
  terminal that runs `ae web` prints a fresh link (`ae web: new link …`), at most once per 10 s.

## What the page shows

The header names the orchestrator session and its standing: `proven` (it runs and its tmux
session proves its identity), `stopped` (it holds the role but is not proven live), `absent` (no
session holds the role) or `gap` (the role could not be read), with the reason. Below it, the
chat: the newest 500 turns, oldest first, and one line per coverage gap. The page asks for
changes every 2 s while it is visible and every 30 s while it is hidden; an unchanged answer is
a `304` and costs no render.

Every text from an agent is shown as plain text, never as markup: control characters are removed,
a turn's text is capped at 64 KiB and a speaker or qualifier at 200 characters.

## The desk column

The column beside the chat lists the orchestrator's topics: **Needs you**, **Moving** and **All
topics** with a search over slug and title. A narrow screen switches between Chat and Topics.

- A topic is a memo checkpoint under `topic-<slug>`, the slug `[a-z0-9][a-z0-9-]{0,47}`. One
  checkpoint line reads `title: Web UI | state: moving | what: … | last: … | next: … | src: … |
  close: …`; `\|` is a literal pipe and `\\` a backslash, other names are ignored. The states are
  `needs`, `moving`, `quiet`, `parked` and `closed`.
- The memos come from the orchestrator seat's session and from every kept archive that the seat
  published, renamed or not. Each field shows the newest record that carries it, with that
  record's session and time; later incarnations rank above earlier ones and, within one memo,
  later lines above earlier ones. The other records are the card's history.
- **Last reviewed** is the time of the topic's newest record, whatever it carries.
- A topic needs you when its state is `needs` or when something waits for you on it: a DECIDE item
  on the orchestrator's board, or your own open ask from `ae chat`, whose text tags it with
  `#<slug>`. A DECIDE item without a tag is listed under **Untagged decisions**.
- A checkpoint without a readable state is shown as an unreadable checkpoint with its raw text.
- A memo over 8 MiB shows none of its topics, and the column names it until `ae web` restarts.
  Archives that are being written, damaged or not proven to come from the seat are counted, not
  read.

Clicking a row opens its card, which `GET /api/topic/<slug>` answers from the same snapshot as the
chat.

## Sending

The composer under the chat sends one owner ask to the orchestrator's main seat, the same ask `ae
chat` opens: it passes the console's own admission, so the cap of five open asks holds and a cap
refusal shows verbatim. `Attach #<slug>` puts the open topic's tag on the draft's first line,
where it stays visible text. Ctrl+Enter or **Send** sends; opening a topic keeps the draft and the
focus.

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
- When the command link names another core and automatic upgrades are on, `ae web` replaces
  itself with that core on the same port once no draft is queued or carrying; meanwhile a send
  answers `busy, retry`. The new core has a new key, so it prints a new link, and a draft that was
  sent is answered from its claim. A failed switch is printed once per version
  (`ae web: cannot follow version <v>: <why>; still serving <old>`) and the old core keeps serving.

## Security

- The listener binds `127.0.0.1` alone and answers only the `Host` values `127.0.0.1:<port>`,
  `localhost:<port>` and `[::1]:<port>` (anything else: `421`). Each `POST` must carry the matching
  `Origin` (`403`). Every `/api/` route but the code exchange needs the session secret as a bearer
  header (`401`); no cookie exists, so another site's page cannot ride on a session.
- Every answer carries `Cache-Control: no-store`, a content security policy without inline script
  or style, `nosniff`, `no-referrer` and same-origin opener and resource policies, and closes its
  connection.
- A request must arrive whole within 10 s and an answer must leave whole within 10 s; a slow
  client costs one of four readers for that long, and a full queue of 16 answers `503`.
- The listener reads only an origin-form target (`/path`). An absolute-form target such as
  `GET http://127.0.0.1:7480/ HTTP/1.1` gets `400`: a deliberate deviation from
  [RFC 9112 §3.2.2](https://www.rfc-editor.org/rfc/rfc9112#section-3.2.2), which asks a server to
  accept it, because no browser sends absolute-form to an origin. A `GET` that announces content
  also gets `400`.
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
- Four cells, each a fresh load: 1440x900 and 360x740, each light and dark. Each cell checks that
  the modules load under the strict CSP header with no exception, console error, dialog or CSP
  violation (`modules`); that a topic titled `<img src=x onerror=alert(1)> & </p>` shows that
  literal text and the page holds no `img` and no inline script (`hostile-literal`); the scheme's
  page background (`theme`); and no horizontal scroll with the draft box inside the viewport
  (`fits`).
- A wide cell types a draft, clicks a topic row and checks that the card opens while the draft
  keeps its text (`draft-kept`) and the focus (`focus-kept`). A narrow cell takes the same draft
  through Topics, the card and back to Chat (`draft-kept`).
- The 1440x900 dark cell clicks Send and waits for the `sent to …` receipt with its request id;
  the fake TUI must have received the draft once (`sent`).
- Each cell leaves a screenshot, and the run its log, in `.local/web-smoke/<UTC stamp>/`.

`just web-smoke plant` serves `app.mjs` without the topic row's mousedown `preventDefault`, the
guard that keeps the focus in the draft. The recipe passes only when that run fails at
`focus-kept (1440x900 light)` and at no other check.
