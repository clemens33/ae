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
| `codex-working-hint-0.159.2-215w.txt` | Codex 0.159.2 (the host's `codex --version`; the running seat's own header row was not captured), footer model `GPT-6.1-Sol` | working: `• Working` row, empty prompt, footer `GPT-6.1-Sol high`, then the `? for shortcuts` hint row with a right-aligned `⚠ 1 warning` | 215 wide, final 8 rows | 2026-09-30, session `aedev`, agent `console-p1-nav`, read-only `ae peek`; working row through hint row, trailing spaces trimmed, transcript above replaced by a redaction |
| `codex-idle-hint-0.159.2.txt` | Codex 0.159.2 (same caveat), footer model `GPT-6.1-Sol xhigh` | idle: empty prompt, footer, `? for shortcuts` hint row alone | width not recorded, final 3 rows | 2026-09-30, session `aedev`, agent `tmuxcrash`, read-only `ae peek`; composer through hint row, trailing spaces trimmed, transcript above replaced by a redaction |
| `codex-starfield-216x6.txt` | Codex 0.155.1 | idle starfield: braille rows above and below the composer, dots on the composer row | 216 wide | 2026-09-22, session `aedev`, agent `archaudit`: plain capture suffix, `Worked for` row through footer, unedited |
| `claude-compacted-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | after `/compact`: `⎿  Compacted (ctrl+o to see full summary)` nearest above the box | 180 wide, final 10 rows | 2026-09-23 19:10:29Z, ctxprobe sandbox `compact-025` (`capture-pane -p -J -S -40 -E -`); `✻ Crunched` row through footer, unedited. Tests swap the manual footer for a SYNTHETIC bypass one |
| `claude-opus55-1m-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5 (1M context) chosen in-pane on a seat launched `--model fable`, bypass mode | idle: empty box, footer `Opus 5.5 (1M context) (max)` | 99 wide, final 5 rows | 2026-09-24 11:11:24Z, session `aedev`, agent `lead`, read-only `ae peek`; composer top rule through footer only, no transcript row, unedited |
| `claude-compacting-2.1.280.txt` | Claude Code 2.1.280, Opus 5.5, manual mode | `/compact` running: `· Compacting conversation…` over an `8%` progress bar, nearest above the box | 180 wide, final 12 rows | 2026-09-23 19:10:11Z, ctxprobe sandbox `compact-016` (`capture-pane -p -J -S -40 -E -`); `✻ Crunched` row through footer, unedited. Tests swap the manual footer for a SYNTHETIC bypass one |
| `claude-limit-2.1.296-180x50.txt` | Claude Code 2.1.296, Opus 5.5, bypass mode | weekly usage limit: four limited turns, the last `✻ … done` row above an empty box, the `⚠ Usage limit reached` and `Continuing automatically at` rows BELOW the box | 180x50, all 50 rows | 2026-10-13 08:23Z, private `tmux -S` probe (`capture-pane -p -J -S -40 -E -`); sanitized: launch path, timezone (same width) and plan name replaced, vendor rows and layout byte-exact |
| `claude-limit-echo-2.1.317.txt` | Claude Code 2.1.317, Opus 5.5, auto mode | limited turn with delivered-message echo above the cell: echo, `  ⎿` cell, `⏺` head, `✻ … done` row, empty box, limit footers | 200 wide, echo through footer (23 rows) | 2026-11-03, private scratch tmux (`capture-pane -p -J -S -100`); sanitized: echo text redacted (shape kept), blank gap cropped, timezone same-width swapped, vendor rows byte-exact |
| `muse-limit-plain.txt` | Muse CLI (version not recorded), `muse-wonder-full-1.0`, max, YOLO | usage-limit stop: `◆ Usage limit reached · /upgrade …` head, three indented follow-up rows, `⚠ Usage nearly exhausted` and `This task may stop before it finishes.` furniture, `── Voice input` rule, empty box, bottom rule, model footer; no background task | width not recorded, final 11 rows | 2026-11-03, read-only `ae peek` of seat `endlocked-nav`, saved as `.local/limitframe-muse/endlocked-nav-peek.txt` (capped at 25 rows); final vendor block through footer only; sanitized: transcript prefix removed, footer path replaced by `~/project` |
| `muse-limit-watching.txt` | Muse CLI (version not recorded), `muse-wonder-full-1.0`, max, YOLO | usage-limit stop with a running background task: bare `◆ Usage limit reached` head plus a `Your usage resets at …` row, the same furniture and empty box, and BELOW the bottom rule `main · ↓ to select` and `└ ◇ … watching … 22m 01s` over the model footer | width not recorded, final 14 rows | 2026-11-03, read-only `ae peek` of seat `limitwake-nav`, saved as `.local/limitframe-muse/limitwake-nav-peek.txt`; final vendor block through footer only; sanitized: transcript prefix removed, footer path replaced by `~/project`, task name replaced by `background evidence watch` |
| `codex-service-wait-0.170.0-80w.txt` | Codex 0.170.0 | service wait ("safety buffering") menu, retry offered: title, dim message, `› 1. Retry with a faster model`, `2. Dismiss and keep waiting`, `3. Learn more`, the "No action is required" footer; drawn in place of the composer and footer | 80 wide | VENDOR-RENDERED: rows 3 on are `codex-rs/tui/src/chatwidget/snapshots/codex_tui__chatwidget__tests__safety_buffering_retry_prompt.snap` at tag `rust-v0.170.0` (render width 80), insta header removed, byte-identical otherwise; row 1 is a redacted transcript row and row 2 a blank, both added |
| `codex-service-wait-0.152.1-80w.txt` | Codex 0.152.1 | the same menu under its older title "Our systems are thinking a bit more about this request before responding." (issue #126) | 80 wide | VENDOR-RENDERED: the same snapshot path at tag `rust-v0.152.1`, the same two rows added |
| `codex-service-wait-noretry-0.170.0-80w.txt` | Codex 0.170.0 | the same menu with no faster model to offer: title only, `› 1. Dismiss and keep waiting`, `2. Learn more`, footer | 80 wide | VENDOR-RENDERED: `codex_tui__chatwidget__tests__safety_buffering_status_without_retry.snap` at tag `rust-v0.170.0`, the same two rows added |
| `codex-service-wait-0.170.0-200w.txt` | Codex 0.170.0 | the retry-offered menu on a wide pane | 200 wide | RECONSTRUCTED: the 0.170.0 80-column rows with the two wrapped paragraphs (message, footer) each on one row, as the vendor's word wrap draws them on a pane wide enough that neither paragraph wraps (the message is 105 cells, the footer 100); strings from `codex-rs/tui/src/chatwidget/safety_buffering.rs` at `rust-v0.170.0` |
| `codex-idle-quoting-service-wait-0.156.1-200w.txt` | Codex 0.156.1 grammar | idle: a transcript row quoting the whole 0.170.0 menu above codex's live empty composer and footer | 200 wide | COMPOSITE control: a synthetic `•` transcript row, the 0.170.0 snapshot rows, then the final rows of `codex-idle-0.156.1-200x40.txt` (`12:50` through the footer) |
| `claude-voice-idle-2.1.332-120x40.txt` | Claude Code 2.1.332 | idle, voice on in hold mode (the same bytes as tap mode and voice off) | 120x40 | 2026-11-26, private tmux server, scratch seat on `--settings` voice hold (`.local/d54/cap2/hold.w120.idle.plain`); the banner rows replaced by one redaction row, the composer and footer rows kept |
| `claude-voice-warming-2.1.332-120x40.txt` | Claude Code 2.1.332 | hold-mode warmup: ` · keep holding…` on the mode row after two Space key events in one `send-keys` | 120x40 | the same seat (`cap2/hold.w120.warm2.plain`), the same redaction |
| `claude-voice-warming-2.1.332-120x40.esc` | Claude Code 2.1.332 | the same warmup frame, `capture-pane -p -e` (the suffix in `\e[38;5;246m`) | 120x40 | `cap2/hold.w120.warm2.esc`, the same redaction |
| `claude-voice-holding-2.1.332-120x41.txt` | Claude Code 2.1.332 | hold recording, right-aligned `listening…` above the upper composer border and `▁` in the prompt | 120x41, final 6 rows | 2026-11-26 owner microphone, private tmux, `.local/d54/owner/20261126T161304Z/p1a_no_repeat/0022_001512.plain`, rows 36–41 unchanged |
| `claude-voice-holding-2.1.332-120x41.esc` | Claude Code 2.1.332 | styled hold recording, the label and waveform in dim ink | 120x41, final 6 rows | `0022_001512.esc`, rows 36–41 unchanged. Both captures fall within the same observed hold interval, with the plain read completed at 1523 ms and the escaped read at 1535 ms. |
| `claude-voice-speaking-2.1.332-120x41.txt` | Claude Code 2.1.332 | hold recording with speech, waveform `▇` | 120x41, final 6 rows | attached owner, `.local/d54/owner/20261126T162001Z/p1_emulated_hold/0029_001853.plain`, rows 36–41 with usage removed from the custom status row |

The hold fixture gives the current box position and the baseline waveform. The
test also derives variants from the installed 2.1.332 binary (SHA256
`5b8214f8f082f3187420b27b4063b9b4f4d9412cd66ea436d6d1966ad582700f`). Its
waveform renderer (`eao`, rendered by `Iro`) maps audio levels to U+2581–U+2588
and omits the waveform under `prefersReducedMotion`. Its hold status renderer
(`abt`) emits `listening…` independently. The column in the captured frame is
an observation at 120 cells. The label is alone and preceded by whitespace on
the row directly above the upper border.

| Fixture | Tool/version | Frame | Terminal | Capture |
|---|---|---|---|---|
| `claude-voice-transcribing-2.1.332-120x41.{txt,esc}` | Claude Code 2.1.332 | hold released, `Transcribing…` above the upper composer border, empty composer | 120x41, final 6 rows | attached owner, private tmux, `.local/d54/owner/20261126T162001Z/p1_emulated_hold/0046_002815.{plain,esc}`, rows 36–41 with usage removed from the custom status row. Plain read completed at 2825 ms and styled at 2835 ms. |
| `claude-voice-tap-2.1.332-120x50.{txt,esc}` | Claude Code 2.1.332 | tap recording, `⏺ REC · tap to send` above the upper composer border and a waveform inside the composer | 120x50, final 6 rows | attached owner, same private tmux, `.local/d54/owner/20261126T162001Z/p3_tap_rec/0020_001555.{plain,esc}`, rows 45–50 unchanged. Plain read completed at 1565 ms and styled at 1577 ms. |
| `claude-voice-interim-2.1.332-120x41.{txt,esc}` | Claude Code 2.1.332 | hold recording with a dim interim transcript followed by a waveform glyph in a separate foreground run | 120x41, rows 36–41 | 2026-11-27 owner microphone, private tmux, `.local/d54/owner/20261127T075230Z/p5_long_hold/0271_009327_p0.plain` and `0272_009327_p0e.esc`. A five-cell spoken token was replaced with `Term.`. All remaining bytes are unchanged. |

The wrapped interim variant in `d54_voice::warming` derives from the 2.1.332
renderer, which inserts interim text into the composer and places the waveform
at the cursor after that text. The waveform renderer varies the bar colour
with audio level and time. The wrapped case and the changed-colour case use
synthetic rows. The one-row styled frame above is the owner-captured oracle.
