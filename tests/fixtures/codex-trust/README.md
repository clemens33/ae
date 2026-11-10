# Codex folder-trust modal fixtures

Frozen frames of the real codex 0.167.0 folder-trust modal ("Folder access"),
drawn in a fresh, never-trusted git folder (`git init`; codex drew no modal in a
plain non-git temp folder). Captured 2026-11-10. The modal was never answered:
each probe server was killed with the modal still drawn.

| Fixture | Frame |
|---|---|
| `codex-trust-modal-80x24.txt` | `codex --yolo` in a private tmux server, status line off, pane 80×24, `capture-pane -p` at ~+8 s |
| `codex-trust-modal-80x24.esc` | the same frame with SGR escapes (`capture-pane -p -e`) |
| `codex-trust-modal-200x50.txt` | the same at 200×50 (a fresh start at 200×50 and an 80×24 pane resized to 200×50 drew identical frames) |
| `codex-trust-modal-200x50.esc` | the 200×50 frame with escapes |
| `codex-trust-modal-39x23.txt` | the worker seat pane (39×23) of a private `ae` probe session, `codex --yolo` started by ae, `capture-pane -p` |

Scrubbed: the probe folder (`/private/tmp/<probe>.<random>/cfolder` or `/folder`) →
`/tmp/codex-trust-probe`. In the `.esc` frames the folder row is drawn dim and
padded with trailing spaces to the pane width; the scrub shortens that padding.
No account, user name or home path appears.

**Load-bearing:** no rule above the title; `Folder access` directly above the ONE
folder row, which sits directly above the paragraph opening `Trust this folder?`;
the selected row `› 1. Trust and continue` (bold + reverse in the `.esc` frames)
above `2. Back to Agent Command Center`, so trust-yes is preselected and the
answer is Enter alone; and the key-hint row `enter continue · esc back` as the
LAST non-blank row. A live codex composer (`› Ask Codex …`) is also a `›` row, but
always with a footer under it, which is why the hint must be the last row.
