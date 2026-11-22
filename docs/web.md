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
- The page's three library files (Preact 11.2.0, its hooks and htm 3.1.1) are vendored and pinned
  by SHA-256 in `src/l5_ui/web/assets/VENDOR.md`; there is no build step. Their licence texts
  (MIT for Preact, Apache-2.0 for htm) are served unchanged at `/assets/licenses.txt`, which needs
  no session.
