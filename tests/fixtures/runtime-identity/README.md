# Runtime identity synthetic fixtures

These fixtures are synthetic mutations of recorded harness-state suffixes.
They cover independent unknown fields (a claude suffix that is no effort voids
the model: `claude-known-model-unknown-effort.txt`), changed effort, historical-vs-current
footer anchoring, empty-path rejection, and `codex-control-token.txt`, whose
U+001B ESC appears inside the model token after `gpt-`. `codex-modal-two-lines-above.txt` pins a modal outside the current
immediate frame anchor. They are not
claims about vendor display. Positive cases use recorded fixtures under
`tests/fixtures/harness-state/`.

## Plain-text identity fixtures

The watchdog reads panes with `capture-pane -p -J` (plain), so the muse and
opencode identity pins need the plain serialisation of frames whose recorded
specimens are `-e` ANSI captures. `muse-idle-plain-80x24.txt` and
`muse-occupied-plain-80x24.txt` are the SGR-strip of
`tests/fixtures/muse-composer/muse-idle-composer.esc` (source sha256
`38f8895b2ffe983cb564803f3170e166ba80a7ab82fb554d1c103a55c4f5c0f3`) and
`muse-occupied-composer.esc` (source sha256
`0ee80b0d906be3afac11a70780c357776d922ca573f1320336747c9db9230a32`);
`opencode-composed-plain-80x24.txt` is the same strip of
`tests/fixtures/opencode-composer/opencode-composed-frame.esc` (source sha256
`fb83ee1d70ad46de6b273abcdb0493109fb98b44c9ca079c4bf72acc0b625f60`). The
sha256s name the SOURCE specimens, not the derived files. Each strip was then
scrubbed: the home path becomes `/home/u`, the ae-dev session path becomes
`/home/u/.ae/sessions/work`, and `~/projects/<org>` becomes `~/projects/u`.
The opencode bottom row was re-padded with spaces so its version column stays
at the measured right edge. No other edit; row shapes and widths are kept,
and the transcript text above each composer stays as the anchor-negative
evidence the parsers are pinned against.

`muse-transcript-only.txt`, `opencode-transcript-only.txt`,
`grok-transcript-only.txt` and `agy-transcript-only.txt` are synthetic: each
quotes the tool's own identity row above a frame that does not carry that
tool's anchor, and each must read as unobserved.

