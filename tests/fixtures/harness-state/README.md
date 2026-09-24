# Harness frame fixtures

Captured read-only on 2026-09-10 from the live `ae` tmux server with
`capture-pane -p -J -S -40 -E -`; transcript text was replaced by bracketed
redactions while TUI suffixes, line breaks, and recorded pane geometry were kept.

| Fixture | Tool/version | Frame | Geometry | Provenance |
|---|---|---|---|---|
| `codex-busy-280x40.txt` | Codex 0.153.4 | busy + visible prompt | 280x40 | `%485` |
| `codex-idle-112x40.txt` | Codex 0.153.4 | idle | 112x40 | `%338` |
| `claude-busy-280x40.txt` | Claude Code 2.1.267 | busy/Whisking | 280x40 | `%462` |
| `claude-busy-167x40.txt` | Claude Code 2.1.267 | busy/Enchanting | 167x40 | `%419` |
| `claude-idle-167x40.txt` | Claude Code 2.1.263 | done/idle, themed | 167x40 | `%78` |
| `claude-resumed-idle-149x37.txt` | Claude Code 2.1.266 | resumed idle | 149x37 | `%298` |
| `claude-busy-nbsp-tip-101x41.txt` | Claude Code 2.1.270 | busy + live NBSP tip | 101x41 | `%0`, 2026-09-14: live suffix, transcript redacted; tip line is `⎿` SP NBSP (`e2 8e bf 20 c2 a0`) |
| `codex-idle-old-busy-280x40.txt` | Codex 0.153.4 | old quoted busy, current idle | 280x40 | `%485`, later capture |
| `codex-idle-112x20.txt` | Codex 0.153.4 | idle, plain short crop | 112x20 | final 20 visible rows of `%338` |
| `codex-idle-wrapped-112x40.txt` | Codex 0.153.4 | narrow joined wrap | 112x40 | `%338` |
| `codex-modal-112x40.txt` | Codex 0.153.4 grammar | modal/unknown | 112x40 | live suffix with modal text substituted |
| `codex-idle-0.155.1-200x40.txt` | Codex 0.155.1 | idle, lowercase footer model | 200x40 | 2026-09-23, private tmux server; plain twin of `../codex-composer/codex-idle-0.155.1-200x40.esc` |
| `codex-idle-0.156.1-200x40.txt` | Codex 0.156.1 | idle, footer model `GPT-6-Astra` | 200x40 | 2026-09-23, private tmux server; plain twin of `../codex-composer/codex-idle-0.156.1-200x40.esc` |
| `codex-starfield-216x6.txt` | Codex 0.155.1 | idle starfield: braille rows above and below the composer, dots on the composer row | 216 wide | 2026-09-22, session `aedev`, agent `archaudit`: plain capture suffix, `Worked for` row through footer, unedited |
| `claude-compacted-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | after `/compact`: `⎿  Compacted (ctrl+o to see full summary)` nearest above the box | 180 wide, final 10 rows | 2026-09-23 19:10:29Z, ctxprobe sandbox `compact-025` (`capture-pane -p -J -S -40 -E -`); `✻ Crunched` row through footer, unedited. Tests swap the manual footer for a SYNTHETIC bypass one |
| `claude-compacting-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | `/compact` running: `· Compacting conversation…` over an `8%` progress bar, nearest above the box | 180 wide, final 12 rows | 2026-09-23 19:10:11Z, ctxprobe sandbox `compact-016` (`capture-pane -p -J -S -40 -E -`); `✻ Crunched` row through footer, unedited. Tests swap the manual footer for a SYNTHETIC bypass one |
