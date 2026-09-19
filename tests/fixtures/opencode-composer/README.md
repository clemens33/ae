# OpenCode composer fixtures

Frozen frames of the real `opencode` TUI, for the unmodelled readiness
question. `e737b6b3` asked only whether a pane was SETTLED (two byte-identical
non-empty captures), and a blank boot frame answers that — so a spawn pasted
its brief into an uninitialised TUI, the paste was dropped, and the unmodelled
submit verdict (`Unknown`) reported success. These two frames pin the COMPOSED
half of readiness: one the check must REFUSE, one it must GRANT.

| Fixture | Tool | Frame | Captured | Provenance |
|---|---|---|---|---|
| `opencode-boot-frame.esc` | opencode 1.18.31 | the literal boot frame: **blank** (one SGR reset, then 23 newlines). Stable for ~2.7 s — exactly what the settle accepted | 2026-09-15 07:07Z, live boot capture on the `ae-dev` socket, 80×24, plain and `-e` frames every ~150 ms | `tmux capture-pane -e -p` bytes, sha256 `312f2bb0c01c7299e4f672f784ca461034d0203a2ce1e448d620ede993c063d8`; the full series and timestamps are in the producing worktree's `.local/bootcap/` |
| `opencode-composed-frame.esc` | opencode 1.18.31 | the composed welcome: logo, the `┃` box with `Ask anything… "Fix broken tests"`, `Build · DeepSeek V4.1 Flash OpenRouter · max`, the `╹▀▀…` bottom edge | 2026-09-15 06:20:5xZ, spawnproof session `sp`, pane `%3` | `capture-pane -e` bytes, sha256 `581d6b2cdb67565946fb0b9c1f35e49ed6c65d7fa8db4aaa0b9fb8320d54de82`; the file is `.local/spawnproof-opencode-pane.esc` in the checkout that captured it (the spawn that lost its brief on `e737b6b3`) |
| `opencode-history-empty-80x24.txt` | opencode 1.18.31 | one submitted turn (`reply with the single word ok` → `ok`); the box with a blank interior, NO placeholder, status row above the edge | 2026-09-19, private probe server, scratch cwd | `capture-pane -p -J` bytes, scrubbed (tmpdir → `/home/ocprobe`) |
| `opencode-history-empty-200x50.txt` | opencode 1.18.31 | same moment, wide: the right sidebar (tokens, `$`, MCP, LSP, cwd path) shares the box rows past the edge run | 2026-09-19, private probe server, scratch cwd | `capture-pane -p -J` bytes, scrubbed |
| `opencode-history-draft-80x24.txt` | opencode 1.18.31 | `half a draft here` typed, unsubmitted, on the placeholder row; the placeholder is gone (a wide draft with sidebar is pinned synthetically) | 2026-09-19, private probe server, scratch cwd | `capture-pane -p -J` bytes, scrubbed |
| `opencode-palette-80x24.txt` | opencode 1.18.31 | ctrl+p palette open (`Commands … esc`, `Search`, items) over a drawn box, overlaying the box top | 2026-09-19, private probe server, scratch cwd | `capture-pane -p -J` bytes, scrubbed |
| `opencode-sesslist-200x50.txt` | opencode 1.18.31 | session list open (`Sessions … esc`, `Search`, `Today`, one entry) over a drawn, empty wide box | 2026-09-19, private probe server, scratch cwd | `capture-pane -p -J` bytes, scrubbed |

The two oldest frames are `capture-pane -e` bytes, the same serialisation
`tests/fixtures/muse-composer/` uses. The five newer frames are PLAIN
`capture-pane -p -J` bytes — the serialisation ae itself captures in
production (`tmux::capture_pane_args`, no `-e`), so the identity grammar
(which never strips SGR) reads them exactly as the watchdog would.

**What is load-bearing.** The boot frame must carry NEITHER marker below; the
composed frame must carry the composer box. The logo block, the rotating
placeholder suggestion, the tip row, the cwd/model footer and the MCP status
segment are incidental capture noise. A later "cleanup" of those rows must not
touch the composer box or these pins silently stop testing what they name.

**The markers, and why these two.** `╹` (U+2579), the composer's bottom-left
corner, is the structural primary: it is layout, not copy, and it is drawn only
when the composer is. `Ask anything…` is the semantic second: the placeholder
affordance itself. Both were measured appearing together on the first non-blank
frame, ~+3.0 s after process start, on opencode 1.18.31 at 80×24 — and the boot
frame carries neither. Since the history frames landed, the placeholder's role
is narrower: a seat with history draws NO placeholder, so marker presence is
never required — `Ask anything…` only names the placeholder PREFIX an empty
interior may carry. All strings are ONE version's UI, so version drift is
an INHERITED hazard this slice names rather than fixes: a renamed or restyled
composer REFUSES visibly, which is the safe direction. The dialog titles
(`Commands`, `Sessions`) carry the same hazard one row up: a renamed dialog
stops refusing, so a new dialog chrome needs a new measured title here.

**The readiness moment was validated live** (2026-09-15, `ae-dev`, 80×24,
opencode 1.18.31): a paste at composed-and-settled time lands in the composer
(0.35 s capture shows the text staged, no turn), and any `Enter` after the
composer is up submits it (`esc interrupt` and the moving-dot indicator). The
same paste at ~+1 s, while the frame was the blank boot frame, is what the
pre-fix settle lost — spawnproof measured that 2/2 on `e737b6b3` and 2/2
visible refusals on its parent.