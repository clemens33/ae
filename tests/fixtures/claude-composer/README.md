# Claude composer fixtures

| Fixture | Tool | Frame | Provenance |
|---|---|---|---|
| `claude-queued-busy.esc` | Claude Code 2.1.280, Haiku 4.5 | a turn pasted and submitted WHILE busy: the queued text above the box with `ctrl+x ctrl+s to send now`, the box row `❯` NBSP then a DIM `Press up to edit queued messages` | 2026-09-23, private tmux server, `capture-pane -e` 300 ms after Enter; spinner row through footer, unedited |
| `claude-compact-staged-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | `ae compact`'s `/compact` dispatch text still in the box, a turn running above it | 2026-09-23 19:10:01Z, the ctxprobe sandbox's `compact-011` capture (`capture-pane -p -J`, no escapes); spinner row through footer, unedited |
| `claude-compact-queued-2.1.280.txt` | same seat, 2 s later | that dispatch QUEUED: its preview and `ctrl+x ctrl+s to send now` above the box, the box row `❯ Press up to edit queued messages` | same run, `compact-012`; spinner row through footer, unedited |
