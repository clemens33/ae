# Harness start-dialog fixtures (dialogseat)

Frozen frames of real harness start dialogs, captured 2026-11-21 in private
tmux servers (`tmux -S <scratch socket>`, `TMUX` and `TMUX_PANE` unset, status
line off, pane at the size named), each harness started directly as the pane
command in a never-trusted scratch directory. No dialog was ever answered: the
servers were killed with the dialog still drawn.

`.history.txt` is `capture-pane -p -S -` of a pane SHORTER than the modal: the
rule and the title scrolled into history, which is what a visible-screen
capture misses. Every other `.txt` is `capture-pane -p` of a pane that holds
the whole frame.

| Fixture | Harness | Frame |
|---|---|---|
| `claude-2.1.330-plain-80x24.txt` | claude 2.1.330 | the folder-trust modal, no project settings: the measured shape, still answerable |
| `claude-2.1.330-settings-80x24.txt` | claude 2.1.330 | `.claude/settings.json` with 1 permission: one `⚠` warning and the "Trusting this folder loads" row |
| `claude-2.1.330-settings-40x12.history.txt` | claude 2.1.330 | the same at 40x12, history read; title and question above the screen |
| `claude-2.1.330-settings-only-120x40.txt` | claude 2.1.330 | a settings file with no permission or hook: only the "Trusting this folder loads" row |
| `claude-2.1.330-hooks-80x24.history.txt` | claude 2.1.330 | 6 permissions and a `PreToolUse` hook at 80x24, history read; rule to hint is 25 rows |
| `claude-2.1.330-hooks-80x50.txt` | claude 2.1.330 | the same at 80x50, whole on screen |
| `claude-2.1.330-hooks-200x50.txt` | claude 2.1.330 | the same at 200x50 |
| `claude-2.1.330-mcp40-80x24.history.txt` | claude 2.1.330 | 40 permissions, a hook and a project `.mcp.json` at 80x24, history read: Claude lists 8 and draws "and 32 more"; rule to hint is 27 rows |
| `claude-2.1.330-mcp40-34x12.history.txt` | claude 2.1.330 | the same settings at 34x12, `-S -40` history read; rule to hint is 47 rows (captured by dialogseat) |
| `codex-0.171.0-login-120x40.txt` | codex 0.171.0 | the sign-in method chooser of a logged-out home |
| `agy-1.5.14-login-120x40.txt` | agy 1.5.14 | the login method chooser of a signed-out home |
| `muse-1.4.24-trust-120x40.txt` | muse 1.4.24 | the workspace-trust question |
| `grok-1.0.92-login-120x40.txt` | grok 1.0.92 | the browser sign-in wait; the one-time code is scrubbed to `ABCD-WXYZ` |
| `opencode-2.3.2-composer-120x40.txt` | opencode 2.3.2 | a READY composer: the control, no dialog |

Scrubbed: probe directories to `/tmp/claude-trust-probe`, `/tmp/ae-dialog-probe/…`
and `/tmp/ae-harness-probe/…`; the 34x12 frame keeps its random
`/private/tmp/ae-trust-width.*` scratch path. No account, user name or home path
appears.

Not measured, so not promised: a Claude login past its theme picker, the
Gemini CLI (absent on the capture host), and any Claude settings modal taller
than the history read (at 30x12 the full warning set loses its title even
with `-S -40`).
