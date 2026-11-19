# Codex composer fixtures

Styled (`capture-pane -e`) codex frames for the delivery grammar, suffix only:
the last transcript row through the footer.

| Fixture | Tool | Frame | Provenance |
|---|---|---|---|
| `codex-idle-0.155.1-200x40.esc` | codex-cli 0.155.1, gpt-6-astra low | idle after one turn: bold `›`, dim placeholder, blank separator, footer in truecolor with dim ` · ` | 2026-09-23, private tmux server, measured |
| `codex-idle-0.156.1-200x40.esc` | codex-cli 0.156.1, gpt-6-astra low | the same; the footer spells `GPT-6-Astra` and carries no dim separators | 2026-09-23, private tmux server, measured |
| `codex-starfield-216.esc` | codex-cli 0.155.1, gpt-6-astra xhigh | the idle starfield: braille dots over the row above the composer, the composer row (one dot in place of the space after `›`) and the blank separator above the footer | COMPOSITE, see below |
| `codex-held-startup-0.162.1-200x40.esc` | codex-cli 0.162.1, stub provider | a paste plus Enter that landed during codex's startup: the composer keeps the chip under a `Working` row and the footer reads bold `Waiting for startup` · bold `esc` cancel; codex submits the held turn itself when the startup turn ends | 2026-10-14, private tmux server, scratch `CODEX_HOME`, measured |
| `codex-working-draft-0.162.1-200x40.esc` | codex-cli 0.162.1, stub provider | the same chip pasted mid-turn BEFORE any Enter: footer bold `tab` to queue message, genuinely staged and unsent | 2026-10-14, private tmux server, scratch `CODEX_HOME`, measured |
| `codex-service-wait-0.170.0-80w.esc` | codex-cli 0.170.0 | the service-wait menu with retry offered, drawn in place of the composer: bold title, dim message, the selected option row bold in the accent colour (`›` included, the read that took it for codex's bold prompt in ae-feedback 64: 57 cells on 3 rows), dim footer | RECONSTRUCTED, see below |
| `codex-service-wait-0.152.1-80w.esc` | codex-cli 0.152.1 | the same menu under its older title | RECONSTRUCTED, see below |
| `codex-service-wait-noretry-0.170.0-80w.esc` | codex-cli 0.170.0 | the menu without the retry option, `› 1. Dismiss and keep waiting` selected | RECONSTRUCTED, see below |
| `codex-idle-quoting-service-wait-0.156.1-200w.esc` | codex-cli 0.156.1 grammar | idle: an unstyled transcript quote of the whole 0.170.0 menu above the measured idle composer and footer of `codex-idle-0.156.1-200x40.esc` | COMPOSITE control |

`codex-starfield-216.esc` is a composite frame. Its rows,
cells and glyphs are the measured plain capture of a live seat (2026-09-22,
session `aedev`, agent `archaudit`; the plain twin is
`../harness-state/codex-starfield-216x6.txt`, byte-identical text). The starfield
could not be reproduced on demand (0.155.1 and 0.156.1, 2026-09-23), so its SGR
was added from two sources: the composer and footer styling measured in the two
idle frames above, and the dots as non-dim truecolor greys between 66 and 165,
the styling a public report measured on codex 0.154 (firstmate PR #4532). The
region tests pin every styling a dot may carry, so no verdict rests on that
choice.

The three `codex-service-wait-*.esc` frames were not captured. Their rows and
cells are the plain twins in `../harness-state/` (vendor snapshots). Their SGR
follows the vendor's rendering code at the matching tag: the title
`Line::bold()`, the message and the footer note `dim()`
(`tui/src/chatwidget/safety_buffering.rs`), and every span of the selected row
`accent_style()`, a foreground colour plus bold
(`tui/src/bottom_pane/selection_popup_common.rs::apply_row_state_style`,
`tui/src/style.rs::accent_style_for`); cyan stands in for the accent colour,
which depends on the terminal background. No verdict may rest on the colour.
