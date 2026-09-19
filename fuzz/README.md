# fuzz — the hostile-parser lane

AGENTS.md: *a parser of hostile persisted state gets cargo-fuzz BEFORE it cuts
over.* This crate is that gate. It is human-run and it REPORTS; it never gates
CI, because the runners carry no nightly.

Session meta, config text, journals, archives and anything hand-editable are
hostile input: a human, a crashed write or another tool put them on disk, and
ae parses them on every command.

## Run it

```sh
just rust-fuzz target=meta_parse secs=60     # one target
just rust-fuzz-all secs=60                   # every target
```

`secs` is 1..999999 and nothing else. libFuzzer reads `-max_total_time` into an
int and treats zero as NO LIMIT, so `0`, `00` and a value past the int range are
refused rather than quietly turned into an unbounded run.

Both refuse before running unless the pinned cargo-fuzz, the pinned nightly and
its `rust-src` component are present, `fuzz/Cargo.lock` is current, and these
sources are rustfmt-clean — `cargo fmt` at the root does not reach a crate
outside the workspace, so the lane checks them with the product rustfmt itself.

Each run ends on an evidence line — paste it into the slice report of the
cutover it gates. `seeds=` counts the TRACKED seeds, which is the part of the
run someone else can reproduce; libFuzzer also reads whatever the local
`corpus/` has accumulated:

```
fuzz evidence: target=meta_parse seeds=4 duration=60s commit=<sha> toolchain=<pin> cargo-fuzz=<pin> result=clean
```

A crash writes its input under `artifacts/<target>/`; reproduce it with
`cargo +<pin> fuzz run <target> artifacts/<target>/<file>`.

## Layout

- `fuzz_targets/<name>.rs` — one thin call into an existing parser, result
  discarded through `black_box`. No logic lives here: a target that needs a
  helper is a sign the parser wants a pure entrypoint instead.
- `seeds/<target>/` — TRACKED named seeds, one shape per file, drawn from
  `tests/fixtures` and from the error paths the parser refuses. Add a file, and
  the lane picks it up; nothing else to declare.
- `corpus/<target>/` — the fuzzer's writable, untracked scratch. libFuzzer
  writes discovered inputs here and reads `seeds/` alongside it.
- No `rust-toolchain.toml`: the justfile's `FUZZ_TOOLCHAIN` is the one nightly
  pin, and the lane passes it as `cargo +<pin>`.

## Targets

| Target | Parser | Input |
|---|---|---|
| `config_parse` | `config::parse_identity` | one identity v2 config text |
| `meta_parse` | `meta::Meta::parse` + `meta::meta_agent_role` + `rename::parse_intent` | one session meta document, including the byte-exact orchestrator-role claim, or one stopped-rename intent document |
| `launch_cmd_lex` | `launch_cmd::lex_simple_command` | one profile command string |
| `config_command` | `config::IdentityConfig::command` | first line the profile, the rest one config text |
| `quota_claude_cache` | `quota::claude::parse` | one Claude settings file, at a fixed and a chosen clock |
| `quota_codex_rollout` | `quota::codex::parse` | first byte the record boundary, the rest a rollout tail |
| `usage_claude_transcript` | `usage::claude::parse` | one Claude assistant transcript JSONL stream |
| `usage_codex_rollout` | `usage::codex::parse_with_head` | first byte the record boundary, then split bounded head/tail Codex rollout bytes |
| `usage_prices` | `usage::prices::parse_row` | one `[prices]` alias row |
| `harness_observed` | `harness_state::{decode_idle, observed_from_option}` | one watchdog-owned `@ae_observed` pane option |
| `picker_agents` | `tmux::parse_picker_agents` | one watchdog-owned `@ae_agents` value at a fixed clock |
| `launch_stamp` | `store::parse_launch_attempt` | one `.launch-attempt` stamp's bytes |
| `last_live_epochs` | `inventory::launch_epochs` | one session meta, read for its launch moments |
| `sanitize_field` | `sanitize::sanitize` (both `Field`s) | one record field's raw bytes |
| `request_id_select` | `tracked::is_request_id` + `sanitize::select_ids` | whole input as one candidate id, plus newline-split as positioned refs |
| `events_parse` | `events::Event::parse_line` + `from_json` | one `events.jsonl` line; `from_json` is driven with a value the real JSON parser produced, never a hand-built tree |
| `board_claude` | `board::Splitter` (chunked feeds) + `board::claude::read_stream` + `board::collect` | first byte sizes the chunks 1..=256, second byte is the `--assistant` flag (`& 1`), the rest one Claude transcript JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_codex` | `board::Splitter` (chunked feeds) + `board::codex::read_stream` + `board::collect` | first byte sizes the chunks 1..=256, second byte is the `--assistant` flag (`& 1`), the rest one Codex rollout JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_grok` | `board::Splitter` (chunked feeds) + `board::grok::read_stream` + `board::collect` | first byte sizes the chunks 1..=256, second byte is the `--assistant` flag (`& 1`), the rest one Grok updates JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_muse` | `board::Splitter` (chunked feeds) + `board::muse::read_stream` + `board::collect` | first byte sizes the chunks 1..=256, second byte is the `--assistant` flag (`& 1`), the rest one Muse session JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_agy` | `board::Splitter` (chunked feeds) + `board::agy::read_stream` or `board::agy_transcript::read_stream` at a fixed seat id + `board::collect` | first byte sizes the chunks 1..=256, second byte is the flag word (bit0 `--assistant`, bit1 entry 0 history / 1 transcript, bit2 the transcript leg opened the truncated sibling, bit3 rows found, bit4 read once), the rest one Antigravity history or transcript JSONL stream (synthetic records in the real shape; content is hand-written, never copied; every seed carries its two framing bytes up front, so its stream parses whole) |
| `board_opencode` | `board::opencode::read` (whole document) at a fixed session id + `board::collect` | first byte is the `--assistant` flag (`& 1`), the rest ONE `opencode export` JSON document. No chunk-size byte, deliberately: this reader has no splitter, because an export is one JSON document, never JSONL (synthetic records in the real shape; content is hand-written, never copied) |
| `brief_retry_record` | `brief_retry::parse`, plus `render` on whatever parsed | one seat's undelivered-brief retry record; a parse that succeeds must render back to bytes that parse identically |

NOTE — production also emits `chat`, `focus`, `refused`, `delivery-failed` and `telegram_autostart_refused`, which carry no seeds: `from_json` never branches on `action`, so they add zero coverage to THIS target — but a future target driving `ref_meaning` or `alert_meaning` would need them.

### The two R15 readers

`sanitize_field` and `request_id_select` are the seat-compact verb's readers
of hostile persisted state (goal lines, memo records, ledger refs — all
hand-editable). Know the instrument's input domain: both take BYTES. They
prove byte-level robustness (no panic, no hang, refusal where specified) of
the strip, the grammar and the newest-first 16-cap selection. They cannot
create filesystem nodes, build ledger order (selection positions are
synthetic), or observe caller composition — the verb's ordering and marker
rendering are pinned by tests, not by these targets.

### The two reboot-proof reducers

`launch_stamp` and `last_live_epochs` are the readers behind the boot-time absence proof
(`docs/internals/stop-identity-contract.md`). Both are on the RESUME and LISTING paths, and
both read files a human edits, so they are fuzzed before the proof they feed is trusted:

- `launch_stamp` reads `.launch-attempt`, whose whole content is one epoch. The reducer is
  BOUNDED (`store::LAUNCH_ATTEMPT_CAP`), so oversize input must be refused rather than
  parsed — the seeds carry that boundary, an unbounded epoch, invalid UTF-8, and the zero
  and negative moments a mandatory stamp may never claim.
- `last_live_epochs` reads a session meta for its `started` / `launch_time.<slot>` /
  `capture_floor.<slot>` rows. `meta_parse` does NOT reach it: that target drives
  `Meta::parse`, which asks a different question of the same bytes. This one also drives the
  per-row claim reading and the AGGREGATE fold (`tmux::Evidence::claim` and
  `Evidence::folded`), because it folds every row as it reads it. Its seeds carry the two
  shapes that decide the grammar: a row that names a moment with no `=` after it, and a
  non-positive epoch in a row that is not the one documented to allow it.

The grammar is PER SOURCE, and the seeds pin both sides of it. Only
`capture_floor.<slot>=0` is a legal zero — the "no known origin" sentinel a retained exact
resume publishes — and it stays silent for liveness. Every other non-positive, bare or
unspellable claim is damage.

Both answer in three states, and the fuzz lane's job is that the third one — damaged
evidence — is reached rather than silently collapsed into "nothing recorded".

## Lock refresh after a release

This lock records the path dependency as `ae <version>`, so a CalVer bump moves
it. `just release` refreshes it between the bump and the version commit, with
the pinned nightly, so one commit carries the whole bump and the lane's
`--locked` proof agrees with what was written.

That refresh is BEST EFFORT by design: a release must never fail on a dev
toolchain. On a machine without the nightly the release publishes with this lock
as committed and warns instead, and the lane then refuses until someone runs the
remedy the warning prints:

```sh
cargo +<pin> metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null
git add fuzz/Cargo.lock
```
