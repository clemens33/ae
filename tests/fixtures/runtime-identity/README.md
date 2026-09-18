# Runtime identity synthetic fixtures

These fixtures are synthetic mutations of recorded harness-state suffixes.
They cover independent unknown fields, changed effort, historical-vs-current
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
`e086277a93f190367f8d6328a7d4b105e47a5fe795d3ed389f9b708bd4ae1055`) and
`muse-occupied-composer.esc` (source sha256
`8536d03c560fb380b408bae27548b8070f64513fad160e1e43efc44bbbde0b47`);
`opencode-composed-plain-80x24.txt` is the same strip of
`tests/fixtures/opencode-composer/opencode-composed-frame.esc` (source sha256
`581d6b2cdb67565946fb0b9c1f35e49ed6c65d7fa8db4aaa0b9fb8320d54de82`). The
sha256s name the SOURCE specimens, not the derived files. Each strip was then
scrubbed: the home path becomes `/home/u` and the ae-dev session path becomes
`/home/u/.ae/sessions/work`. No other edit; row shapes and widths are kept,
and the transcript text above each composer stays as the anchor-negative
evidence the parsers are pinned against.

`muse-transcript-only.txt`, `opencode-transcript-only.txt`,
`grok-transcript-only.txt` and `agy-transcript-only.txt` are synthetic: each
quotes the tool's own identity row above a frame that does not carry that
tool's anchor, and each must read as unobserved.

