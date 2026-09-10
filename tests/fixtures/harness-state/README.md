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
| `codex-idle-old-busy-280x40.txt` | Codex 0.153.4 | old quoted busy, current idle | 280x40 | `%485`, later capture |
| `codex-idle-112x20.txt` | Codex 0.153.4 | idle, plain short crop | 112x20 | final 20 visible rows of `%338` |
| `codex-idle-wrapped-112x40.txt` | Codex 0.153.4 | narrow joined wrap | 112x40 | `%338` |
| `codex-modal-112x40.txt` | Codex 0.153.4 grammar | modal/unknown | 112x40 | live suffix with modal text substituted |
