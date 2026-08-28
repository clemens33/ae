# archive-preview fixtures

Byte-parity inputs and captured frozen-bash outputs for `ae archive preview`
(P3.1), consumed by `tests/it/archive.rs`.

Each `<case>/`:
- `session/` — the session directory contents (`meta`, and where present
  `memo.tsv`, `events.jsonl`, `messages/`) the preview reads.
- `expected.stdout` / `expected.stderr` — what the frozen `ae archive preview`
  wrote (captured 2026-08-28). `<DIR>` in stderr stands for the session
  directory the test builds.
- `rc` — the frozen exit code.

Cases: `ordinary` (a full session), `empty` (meta only), `malformed`
(readable files with malformed content: non-JSON/truncated/unterminated event
lines, tab-collapsed memo rows, missing body files, out-of-order roster slots).
