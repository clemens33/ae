# Claude composer fixtures

| Fixture | Tool | Frame | Provenance |
|---|---|---|---|
| `claude-queued-busy.esc` | Claude Code 2.1.280, Haiku 4.5 | a turn pasted and submitted WHILE busy: the queued text above the box with `ctrl+x ctrl+s to send now`, the box row `❯` NBSP then a DIM `Press up to edit queued messages` | 2026-09-23, private tmux server, `capture-pane -e` 300 ms after Enter; spinner row through footer, unedited |
| `claude-compact-staged-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | `ae compact`'s `/compact` dispatch text still in the box, a turn running above it | 2026-09-23 19:10:01Z, the ctxprobe sandbox's `compact-011` capture (`capture-pane -p -J`, no escapes); spinner row through footer, unedited |
| `claude-compact-queued-2.1.280.txt` | same seat, 2 s later | that dispatch QUEUED: its preview and `ctrl+x ctrl+s to send now` above the box, the box row `❯ Press up to edit queued messages` | same run, `compact-012`; spinner row through footer, unedited |

## 2.1.330 placeholders (the sleep-resend bug)

Claude draws a hint in the input row of an EMPTY composer after a failed turn
(and while a turn is queued). ae once read those cells as a typed draft
("composer occupied: 27 content cells on 1 row") and abandoned every send.

Captured 2026-11-24 on the Mac: `claude 2.1.330 --bare` (Opus 5.5, API-key mode)
in a detached tmux 3.7b on a private socket (120x30, no client, a scratch
`CLAUDE_CONFIG_DIR`), talking to a local mock of the Messages API (a probe-only
script; the dummy key and the mock's address appear in no fixture). One frame
per state, `capture-pane -e -p`, whole visible screen, unedited except two
scrubs: the probe folder → `/tmp/claude-sleep-probe`, the mock's port →
`PORT`. The real sleep error could not be produced on demand; claude picks the
hint by the error CLASS (its `nextStepHint`: `try_again`, `fix_setup`,
`edit_message`), never by the error text, and the drop below is a `try_again`
class error like the sleep one (strings read from the 2.1.330 binary:
`Resend your message to try again`, `Once fixed, resend your message`,
`Run /rewind to go back`).

| Fixture | State | Box row, as drawn |
|---|---|---|
| `claude-resend-try-again-2.1.330.esc` | the connection dropped mid-turn (`ECONNRESET`), after four earlier failures | `ESC[39m❯` NBSP, `ESC[7mR` (the cursor cell), `ESC[0;2mesend your message to try again`, `ESC[0m` |
| `claude-once-fixed-resend-2.1.330.esc` | HTTP 401 (`fix_setup`) | the same shape over `Once fixed, resend your message` |
| `claude-rewind-hint-2.1.330.esc` | HTTP 413 (`edit_message`) | the same shape over `Run /rewind to go back` |
| `claude-queued-busy-2.1.330.esc` | a second turn submitted while the first still ran | `ESC[38;5;246m❯` NBSP, `ESC[7m` `ESC[39m` `P`, `ESC[0;2mress up to edit queued messages` |
| `claude-idle-2.1.330.esc` | a fresh idle composer | `ESC[39m❯` NBSP, `ESC[7m` + a space (the bare cursor) |
| `claude-typed-resend-cursor-end-2.1.330.esc` | the hint's words TYPED into the box, cursor at the end | `ESC[39m❯` NBSP `Resend your message to try again` then `ESC[7m` + a space |
| `claude-typed-resend-cursor-home-2.1.330.esc` | the same words typed, the cursor moved to the start (Ctrl-A) | `ESC[7mR` `ESC[0m` then `esend your message to try again`, NOT dim |

**Load-bearing:** a placeholder is the cursor cell (inverse, not dim) followed
by a run in SGR 2 (dim). Typed text is never dim, so the same words typed read
as a draft; the two typed frames are the discriminator. The older
`claude-queued-busy.esc` (2.1.280) drew the whole hint dim with no cursor cell.
Only the four strings above are measured; claude 2.1.330 also holds
`Press up to edit queued messages, Enter to send them now`, `Taking your
message back…` and the rotating `Try "…"` ghost text, which stay unmeasured and
read as a draft.
