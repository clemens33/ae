# OpenCode free-tier usage-limit fixtures

Frames for the watchdog's OpenCode limit row (`watchdog::OPENCODE_BANNER`).
OpenCode 1.19.3 draws its usage limit as a spinner footer row, not a transcript
cell: the spinner run, the vendor's words, the retry time and its key hint.

| Fixture | What it is | Provenance |
|---|---|---|
| `spinner-dots.txt` | the limit footer, spinner all `⬝`: `Free usage exceeded, subscribe to Go [retrying in 16h 10m attempt #1]`, `esc interrupt`, `• OpenCode 1.19.3` | footer, `╹▀` edge and status row copied from a live pane capture, OpenCode 1.19.3, 2026-10-21; rows above the status row are a minimal synthetic transcript |
| `spinner-mixed.txt` | the same footer a frame later: spinner `■■■■⬝⬝⬝⬝`, `[retrying in 16h 9m attempt #1]`, `esc again to interrupt` | same capture series, variant row as observed; rows above synthetic |
| `endpoint-unavailable.txt` | `Error: Upstream request failed: Endpoint is unavailable.`, an availability failure `opencode run` prints | observed `opencode run` output, 2026-10-21; the command line above it is synthetic |
| `transcript-quote.txt` | an idle frame whose transcript QUOTES the limit: a `┃` rail user turn, a 5-column assistant row, and a 5-space spinner-shaped row, all inside the bottom window | synthetic; the idle footer row is invented |

**Load-bearing:** the limit frames end on the spinner footer (the capture's last
non-blank row) with EXACTLY three spaces before the spinner run and a space after
it. The quote frame keeps every look-alike inside the bottom window, so a
reader that matched the words anywhere in the window would book it.
The endpoint error is NOT a usage limit and must book nothing.
