# Codex composer fixtures

Styled (`capture-pane -e`) codex frames for the delivery grammar, suffix only:
the last transcript row through the footer.

| Fixture | Tool | Frame | Provenance |
|---|---|---|---|
| `codex-idle-0.155.1-200x40.esc` | codex-cli 0.155.1, gpt-6-astra low | idle after one turn: bold `›`, dim placeholder, blank separator, footer in truecolor with dim ` · ` | 2026-09-23, private tmux server, measured |
| `codex-idle-0.156.1-200x40.esc` | codex-cli 0.156.1, gpt-6-astra low | the same; the footer spells `GPT-6-Astra` and carries no dim separators | 2026-09-23, private tmux server, measured |
| `codex-starfield-216.esc` | codex-cli 0.155.1, gpt-6-astra xhigh | the idle starfield: braille dots over the row above the composer, the composer row (one dot in place of the space after `›`) and the blank separator above the footer | COMPOSITE, see below |

`codex-starfield-216.esc` is the one fixture here not captured whole. Its rows,
cells and glyphs are the measured plain capture of a live seat (2026-09-22,
session `aedev`, agent `archaudit`; the plain twin is
`../harness-state/codex-starfield-216x6.txt`, byte-identical text). The starfield
could not be reproduced on demand (0.155.1 and 0.156.1, 2026-09-23), so its SGR
was added from two sources: the composer and footer styling measured in the two
idle frames above, and the dots as non-dim truecolor greys between 66 and 165,
the styling a public report measured on codex 0.154 (firstmate PR #4532). The
region tests pin every styling a dot may carry, so no verdict rests on that
choice.
