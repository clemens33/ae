# Agy composer fixtures

Frozen frames of the real `agy` TUI (1.2.6, `agy
--dangerously-skip-permissions`, no prompt), for the unmodelled readiness
question. Captured 2026-09-18 in private probe panes (`tmux capture-pane -p`).

| Fixture | Frame |
|---|---|
| `agy-boot-frame.txt` | boot at ~+1 s, 200×50: logo + "Signing in…" spinner, no composer |
| `agy-composed-frame.txt` | settled composer at ~+5 s, 200×50: `─` rule, `>` row, `─` rule, `? for shortcuts` footer |
| `agy-composed-frame-80x24.txt` | same composer at 80×24 |
| `agy-draft-frame-80x24.txt` | a single-line draft (`draft from human`) typed on the `>` row, 80×24: the fence stays; the footer's `? for shortcuts` disappears and only the model label remains |
| `agy-wrapped-draft-frame-80x24.txt` | a draft long enough to wrap, 80×24: the `>` row plus an indented continuation row between the two rules |
| `agy-trust-modal-frame.txt` | folder-trust modal in a fresh dir, stable: question + two options, NO rules, NO composer — must NEVER read as composed |

Scrubbed: account email → `someone@example.com`, worktree path
`~/projects/clemens33/ae` → `~/projects/u/ae`, fresh-dir path →
`/tmp/agy-trust-probe`. **Load-bearing:** the rule/`>`/rule fence, the
footer literal, the footer as the last non-blank row at BOTH sizes, and the
modal carrying the model label (`Gemini 3.8 Flash · high`) WITHOUT rules.
The draft frames are load-bearing the other way: the fence survives the
draft (the marker alone is not empty-composer proof), a wrapped draft's
continuation row breaks the rule/prompt/rule scan, and the footer hint is
absent while text sits in the prompt.
