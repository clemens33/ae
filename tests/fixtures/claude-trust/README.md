# Claude folder-trust modal fixtures

Frozen frames of the real Claude Code folder-trust modal (2.1.281), drawn in a
fresh, never-trusted directory. Captured 2026-09-24 in a private tmux server
with the status line off, pane at the exact size named, `claude` started with
no prompt from `/bin/sh`, once two 1 s captures in a row were equal (~+3 s).
The modal was then dismissed with Esc ("No, exit"); nothing was ever accepted.

| Fixture | Frame |
|---|---|
| `claude-trust-modal-80x24.txt` | the modal at 80×24, `capture-pane -p` |
| `claude-trust-modal-200x50.txt` | the modal at 200×50, `capture-pane -p` |
| `claude-trust-modal-80x24.esc` | the 80×24 frame with SGR/OSC escapes (`capture-pane -p -e`) |
| `claude-trust-modal-200x50.esc` | the 200×50 frame with escapes |

The same four captures were taken with `claude --permission-mode
bypassPermissions`: every row below the launch line is byte-identical, so the
bare frames stand for both. `capture-pane -p -J` equals `-p` apart from the
launch line's trailing space: Ink wraps the paragraph with hard newlines.

Scrubbed: the probe directory `/private/tmp/trust157.<random>` →
`/tmp/claude-trust-probe`. No account email, user name or home path appears.
The five rows above the modal are the probe shell's own scrollback (a config
home check, `claude --version`, the launch line), kept as captured.

**Load-bearing:** the full-width `─` rule on top and NO composer anywhere; the
question row opening with `Quick safety check:` and its `?` mid-row, not at
the end; the selected row `❯ No, exit` (not bold) above `Yes, I trust this
folder`; and the key-hint row `Enter to confirm · Esc to cancel` as the LAST
non-blank row, eleven (80×24) or ten (200×50) rows below the question row.
