# Grok composer fixtures

Frozen frames of the real `grok` TUI (1.0.34, `grok --always-approve -m
grok-4.6 --effort high`, no prompt), for the unmodelled readiness question.
Captured 2026-09-18 in private probe panes (`tmux capture-pane -p`, no `-e`:
plain, the serialisation the readiness read uses).

| Fixture | Frame |
|---|---|
| `grok-boot-frame.txt` | boot at ~+1 s, 200×50: **blank**, stable until the composer draws |
| `grok-composed-frame.txt` | composed welcome at ~+2 s, 200×50: rounded `╭`/`│`/`╰╯` box, `│ ❯` input row, quota footer on the edge row |
| `grok-composed-frame-80x24.txt` | same composer at 80×24 |
| `grok-draft-frame-80x24.txt` | a single-line draft (`draft from human`) typed after `❯`, 80×24: the box stays, the rail row reads `│ ❯ draft … │`, and the hint row replaces the version line |
| `grok-wrapped-draft-frame-80x24.txt` | a draft long enough to wrap, 80×24: the input row plus two continuation `│` rail rows, the box grown by two |

Scrubbed: worktree path `~/p/clemens33/ae` → `~/p/u/ae`. The footer quota
number, model label and flags are incidental capture noise, never markers.
**Load-bearing:** the box glyphs, `❯` on the input row, and the edge row two
rows above the last non-blank row (the version line) at BOTH sizes. The
draft frames are load-bearing the other way: `❯` stays drawn while text sits
on the input row, so the marker alone is not empty-composer proof; the
wrapped frame grows the box with continuation rail rows.
