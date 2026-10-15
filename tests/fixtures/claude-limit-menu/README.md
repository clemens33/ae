# Claude usage-limit menu fixture

`claude-limit-menu-39col.txt` is the real `/rate-limit-options` dialog of a Claude
Code seat whose account was at its weekly limit (100%), the `cc-mic` login, drawn
in a 39-column pane. Captured 2026-10-15 ~15:32Z from a live ae seat with `peek`
(bottom rows, plain text, the pane's own trailing blanks kept); only the
capture's one comment line was dropped. No account name, email or path appears.

| Row | Meaning |
|---|---|
| `✻ Sautéed for 6s · done Tuesday 3:03 PM` | the finished turn above the echo |
| `❯ /rate-limit-options` | the command echo; the input box is NOT drawn |
| 39 × `─` | the dialog's own full-width rule |
| `What do you want to do?` | its title |
| `❯ 1. Stop and wait for limit to reset` | the selected option |
| `2. Wait here, then continue` / `automatically at Oct 18 at 2am` | option 2, wrapped at 39 columns; the reset time |
| `3. Switch to usage credits` | option 3 |
| `Enter to confirm · Esc to cancel` | the key hint, the LAST non-blank row |

**Load-bearing:** the full-width rule under which the title sits, no composer
anywhere, one `❯`-selected option, the `Stop and wait for limit to reset`
option, and the hint as the last non-blank row. Options 2 and 3 are the
account's and are not matched. Only this one frame was measured: a wider pane
draws option 2 on one row, and the reader's tests build that variant (and the
others) from this frame rather than treat it as measured.
