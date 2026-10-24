# fuzz — the hostile-parser lane

AGENTS.md: *a parser of hostile persisted state gets cargo-fuzz BEFORE it cuts
over.* Fuzz execution is human-run and REPORTS. Separately, `just rust-fuzz-check`
type-checks every target with the product compiler in `just test` and CI, without
running a fuzzer. CI carries no nightly or cargo-fuzz.

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
| `quota_history` | `quota::history::parse` + `render` + pure `fold` | bounded JSONL rows; versions, duplicate keys, samples, workload, scope/policy segmentation and world-clock alignment; never a filesystem path |
| `config_parse` | `config::parse_identity` | one identity v2 config text |
| `meta_parse` | `meta::Meta::parse` + `meta::meta_agent_role` + `rename::parse_intent` + `meta::raw_seat_work_dir` (fixed slots) + `meta::named_conversations` + `meta::resolve_seat_dir` + `meta::explicit_seat_row` + `meta::resolve_seat_target` + `meta::contained_in` + `meta::effective_seat_dir` + `meta::explicit_token_only` + `seatcompact::incarnation` (fixed slots, fixed synthetic roots, pure only — never a fuzz-selected host path; raw-bytes resolver judgment + provenance policy only, no filesystem-existence or JIT-consumer coverage) | one session meta document, including the byte-exact orchestrator-role claim and seat-dir rows in every damage shape, or one stopped-rename intent document |
| `archive_harness` | `archive::harness::{parse,encode,from_meta}` (fixed archive UUID; canonical byte round-trip) | inert `harness.tsv` or source session meta; parser bounded at 1 MiB/1024 slots, no filesystem or deletion |
| `launch_cmd_lex` | `launch_cmd::lex_simple_command` | one profile command string |
| `config_command` | `config::IdentityConfig::command` | first line the profile, the rest one config text |
| `quota_claude_cache` | `quota::claude::parse` (cache rows and the `oauthAccount` plan), plus `Plan::label` on whatever plan survived | one Claude settings file, at a fixed and a chosen clock |
| `quota_feed` | `quota::feed::extract` + `quota::feed::parse`, plus `render` on whatever parsed | one Claude statusline payload, or one `quota-feed` record; a record that parses must render back to its own bytes |
| `quota_codex_rollout` | `quota::codex::parse` (rows, account facts and the thinned per-window history) | first byte the record boundary, the rest a rollout tail |
| `quota_grok_log` | `quota::grok::parse` | first byte the line boundary, the rest a grok debug-log tail, at a fixed and a chosen clock |
| `quota_agy_output` | `quota::agy::{version, supports_quota, parse}` | one agy stdout capture: a `--version` answer and a `/quota` report, at a fixed and a chosen clock |
| `quota_go_usage` | `quota::go::parse` + `quota::go::rejection` + `quota::go::parse_cache` | first byte selects the parser (`0` vendor body, read both as a usage answer and as a rejection's error body, anything else the cache record); the rest hostile bytes, parsed at a fixed clock and at a clock the next eight bytes choose; a code that `rejection` reads must be a cache outcome |
| `quota_copilot_user` | `quota::copilot::parse` | one internal GitHub Copilot user answer, at a fixed clock and at an input-chosen clock; optional fields, invalid counts, unknown plans and resets |
| `quota_muse_usage` | `quota::muse::{parse, extract, limit, Allowance::rows}` + `quota::muse::parse_cache` (+ `render` round trip, `due`, `rows`) | first byte selects the parser (`0` the `muse exec --json` stream, `2` a failed run's stdout read for its usage limit, anything else the held cache record); the rest hostile bytes, the stream parsed at a fixed clock and at a clock the next eight bytes choose, a record that parses must render back to a record that parses to the same value, and a limit read from stdout must be a believable clock that survives the cache round trip |
| `usage_claude_transcript` | `usage::claude::parse` | one Claude assistant transcript JSONL stream |
| `usage_codex_rollout` | `usage::codex::parse_with_head` | first byte the record boundary, then split bounded head/tail Codex rollout bytes |
| `usage_prices` | `usage::prices::parse_row` | one `[prices]` alias row |
| `harness_observed` | `harness_state::{decode_idle, observed_from_option}` | one watchdog-owned `@ae_observed` pane option |
| `picker_agents` | `tmux::parse_picker_agents` | one watchdog-owned `@ae_agents` value at a fixed clock |
| `launch_stamp` | `store::parse_launch_attempt` | one `.launch-attempt` stamp's bytes |
| `console_draft_name` | `store::console_draft_instance` | one hostile session-directory entry; only an exact `console.draft.<canonical-uuid>` is an instance record, never the legacy name, a lock sibling, a temp, a path or a malformed instance |
| `last_live_epochs` | `inventory::launch_epochs` | one session meta, read for its launch moments |
| `codex_developer_record` | `session_launch::developer_record_carries` | one Codex rollout line, as the transcript holds it (tool outputs and compaction replays included), tested against a fixed launch marker: a line may prove ownership only when the real JSON parser sees a `response_item` message with role `developer` whose content text carries the marker, and an empty marker proves nothing. Seeds pin the proving record in real, spaced and reordered spellings and every look-alike that must not prove: an `event_msg` quote, a compaction replay, a user turn, a tool-output kind, a marker in an id or another key, an escaped copy inside a string, truncated or garbage-trailed JSON, a content text that is not a list, a nesting bomb and an empty line |
| `sanitize_field` | `sanitize::sanitize` (both `Field`s) | one record field's raw bytes |
| `request_id_select` | `tracked::is_request_id` + `sanitize::select_ids` | whole input as one candidate id, plus newline-split as positioned refs |
| `events_parse` | `events::Event::parse_line` + `from_json` + `requests::states` + `requests::bound_opening` | hostile JSONL; the first-row id drives the whole ledger vs an opening-only mirror, with no repair and exact last-row repair. `from_json` uses the real JSON parser, never a hand-built tree |
| `journal_append` | `events::Held` exact-prefix core + `EventLog::parse_bytes`, `console::snapshot_at` beside the whole strict `console::snapshot` | first two bytes size an initial journal, then op/size/payload triples append, rewrite in place, shrink or replace it; after every operation, held and whole lenient records/skips/cursor and strict answers (including refusal text) must match; strict is checked both alone and primed by follow; no filesystem paths are chosen |
| `board_claude` | `board::Splitter` (chunked feeds) + `board::claude::read_stream`, beside the same chunks through `Splitter::feed_each` into `board::claude::read_fed` (rows, coverage and ending asserted equal) + `board::claude::is_wrapped_ae_turn` over every produced row body + `board::collect` | first byte sizes the chunks 1..=256, second byte is the reply mode (bit 2 the console default, else `& 1` the `--assistant` flag; the `window-interleave` seed carries `{$` so it reads the console default), the rest one Claude transcript JSONL stream (synthetic records in the real shape; content is hand-written, never copied; the `wrapped-*` seeds carry `{`+LF framing so the flag reads off and the stream parses whole) |
| `board_codex` | `board::Splitter` (chunked feeds) + `board::codex::read_stream`, beside the same chunks through `Splitter::feed_each` into `board::codex::read_fed` (rows, coverage and ending asserted equal) + `board::collect` | first byte sizes the chunks 1..=256, second byte is the reply mode (bit 2 the console default, else `& 1` the `--assistant` flag; the `window-interleave` seed carries `{$` so it reads the console default), the rest one Codex rollout JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_grok` | `board::Splitter` (chunked feeds) + `board::grok::read_stream`, beside the same chunks through `Splitter::feed_each` into `board::grok::read_fed` (rows, coverage and ending asserted equal) + `board::collect` | first byte sizes the chunks 1..=256, second byte is the reply mode (bit 2 the console default, else `& 1` the `--assistant` flag; the `window-interleave` seed carries `{$` so it reads the console default), the rest one Grok updates JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_muse` | `board::Splitter` (chunked feeds) + `board::muse::read_stream`, beside the same chunks through `Splitter::feed_each` into `board::muse::read_fed` (rows, coverage and ending asserted equal) + `board::collect` | first byte sizes the chunks 1..=256, second byte is the reply mode (bit 2 the console default, else `& 1` the `--assistant` flag; the `window-interleave` seed carries `{$` so it reads the console default), the rest one Muse session JSONL stream (synthetic records in the real shape; content is hand-written, never copied) |
| `board_agy` | `board::Splitter` (chunked feeds) + `board::agy::read_stream` or `board::agy_transcript::read_stream` at a fixed seat id, beside the same chunks through `Splitter::feed_each` into that reader's `read_fed` (rows and coverage asserted equal) + `board::collect` | first byte sizes the chunks 1..=256, second byte is the flag word (bit0 `--assistant`, bit1 entry 0 history / 1 transcript, bit2 the transcript leg opened the truncated sibling, bit3 rows found, bit4 read once), the rest one Antigravity history or transcript JSONL stream (synthetic records in the real shape; content is hand-written, never copied; every seed carries its two framing bytes up front, so its stream parses whole) |
| `board_opencode` | `board::opencode::read` (whole document) at a fixed session id + `board::collect` | first byte is the `--assistant` flag (`& 1`), the rest ONE `opencode export` JSON document. No chunk-size byte, deliberately: this reader has no splitter, because an export is one JSON document, never JSONL (synthetic records in the real shape; content is hand-written, never copied) |
| `brief_retry_record` | `brief_retry::parse`, plus `render` on whatever parsed | one seat's undelivered-brief retry record; a parse that succeeds must render back to bytes that parse identically |
| `challenge_summary` | `watchdog::challenge_named` | one watchdog record's summary: a done or wait challenge as written, `[unconfirmed]`, with a refused-pre-paste tail, or behind a delivery-abandoned refusal reason |
| `console_command` | `console::input::Input` (a kept draft restored through `restore`, then chunked feeds) + `console::submit::Draft` + `console::input::paint` / `Screen` (the composer laid out by `Input::view`) | first byte sizes the reads 1..=256, second byte sizes the kept draft restored before any key (taken from the front of the rest, at most what is there), third byte sizes the pane (width `1 + b % 120`, height `1 + b / 5 % 24`), the rest what a terminal sent an owning console; every seed carries all three header bytes up front. A read boundary changes no effect and no layout; no row is wider than its pane (a non-ASCII character counted as two cells), the rows stay within the row cap and the cursor row begins with the text before it; a restored draft only prints (the `Kept line, maybe already sent` banner) and is never read as a command until a fresh Enter; every ask equals an independent copy of the grammar written in the harness, speaker memory included (an `@seat ` line asks and moves the speaker, any other line asks the speaker, never a bare slash line, never a blank body, always a lead-pair seat); the same stream closed with a paste end and Enter enters exactly the lines an independent copy of the key table and editor writes (arrows in both cursor-key modes, Home/End, Delete, Backspace, ^A/^E/^U, a stray byte that is not UTF-8 as a unit of its own, text inserted contiguously at a byte offset and any other key snapping to the end of the character it sits in, a paste literal to its end with CR and CRLF folded to LF, ^J and Alt+Enter a text-like newline and only a typed CR entering), one effect per line, each ask carrying its line byte for byte; and every paint, erase and settle is only printable text, line breaks and the composer's own sequences (autowrap and paste switches, relative up/down moves, clear, cursor save and restore) |
| `app_view` | `app::handover::parse` + `Handover::render` + `Handover::admits` | one `@ae_app_view` session option or `ae app --app-view` argument at a fixed clock; any text parsed must render back to itself byte for byte, except the older core's `overview` view token, which renders as `agents` and reads back as the same value |
| `ask_unframe` | `tracked::uncompose_ask` (the console's reader of a stored ask body), plus the envelope `tracked::compose` and `provenance::first_line` write around any text | first byte sizes the id the stored text is read for, taken from the front of the rest, the rest the stored file read lossily. The answer is `None` or a piece of that very text behind the peer marker; any text framed for either console name comes back byte for byte, whatever markers or footers it carries |
| `lease_holder` | `console::holder::parse`, plus `encode` on whatever parsed | one writer-lease holder record (`1 <session> <window index>` and a line feed, at most 160 bytes) as the lock file holds it; the grammar has one spelling per place, so a record that parses must encode back to the very bytes read. Seeds pin every refusal: a missing line feed, two lines, another version, a 129-byte session, a six-digit or non-digit window, a CRLF, invalid UTF-8, a NUL, and input over the cap |
| `card_body` | `console::card::layout` | one decision-card reason as the journal holds it, read through `String::from_utf8_lossy` because the chat reads the journal lossily; any text lays out without a panic, and the words its parts draw equal the reason's words, the standalone `|` option separators aside. Seeds: the two brief shapes, the STATE_GUIDANCE shape, wordless and doubled pipes, a lone `|`, an unbalanced paren, abbreviations and ellipses, versions and `file:line`, a label family, NBSP and control bytes, invalid UTF-8 |
| `effort_flag` | `launch_cmd::{effort_flag, effort_skeleton, replace_effort_flag}` | first line the harness binary, the rest one profile command; a command that proves a level must agree with its skeleton, rewrite to `high`, read `high` back and keep the same skeleton. Seeds cover claude split, joined, account-prefixed and quoted forms, codex split and joined config pairs, and every refusal: an overriding environment variable, a second or look-alike occurrence, a missing, empty or unknown value, a spaced or nested config key, a non-simple command and an unsupported harness |
| `effort_record` | `effort::Record::parse` + `effort::stored` (fixed slots `main` and `spawned.0`) | one `effort.<slot>` value or one session meta holding such rows; a value that parses must encode back to the very text read, and a present record must be found again, under its own slot, from its own encoding. Seeds cover every state, both harnesses, percent-encoded buckets and qualifiers, the bounds, and every refusal: a duplicated, bare or foreign-slot row, another version, bytes that are not UTF-8, lowercase or needless hex, a leading zero, a short window, an incoherent step, a duplicate window and a trailing newline |
| `effort_fold` | `events::Event::parse_line` (refused lines skipped) + `effort::fold` (fixed session, slot `spawned.0`, agent `scout`) + `effort::decide` over a fixed table of scopes, records and live levels | one seat journal, one `events.jsonl` record per line; the fold is a function of its records, and no decision steps down, steps up, opens or settles an episode the fold does not allow. The strict damaged-journal ledger is the caller's and is NOT covered here. Seeds cover every leg, duplicate and closed-key outcomes, a rename-proof restart, a failed step each way, a yield, a lifecycle reset, a forged actor, a foreign slot and every refusal: a non-canonical or foreign ref, an episode ref that is not its own epoch, a strict threshold, an outcome with no reservation, an up before a down and unknown words |
| `token_parse` | `token::parse` + `wire` + `label` + `collapse` on valid UTF-8, `token::classify_paste` on the raw bytes | one text the history, a kept draft or a paste can hold. `parse` is lossless (its pieces, a token as its wire text, rebuild the input byte for byte, text runs are maximal), a token alone parses back to itself, a number is 1..=999999 without a leading zero, a path is absolute with no `"` or control; `collapse` shows each label and `classify_paste` never returns a path that could not sit in a token |
| `link_scan` | `console::link::scan` + `wrapped` + `placed` + `is_safe_url` (lossy UTF-8) | one lane body the journal can hold. `scan` is total; its ranges index the text at char boundaries, ascending and disjoint; a link's visible text holds no whitespace or control; every URL is printable ASCII. `wrapped` loses no non-whitespace character, slices each row's links inside its piece, and each URL's parts in row order spell its scanned text. `placed` preserves each original file occurrence, complete suffix and source order across rows at varying widths; fragments keyed by source range spell that occurrence, including duplicates. Seeds add repeated paths with different suffixes, long wraps, wide/combining paths, card options/evidence and clipped excerpts; existing seeds pin prose, balanced and unbalanced parens, escaped look-alikes (`ESC ] 8 ;;` payloads), Unicode and CJK, huge `:line` runs, the non-prefix boundaries (`/open`, `and/or`, `1/2`), card pipes, NUL/C1/DEL, and an empty line |
| `reader_markdown` | `app::markdown::render` (pulldown events -> bounded semantic rows) | two little-endian width bytes, then hostile lossy UTF-8; frozen TABLE/HOSTILE fixtures, nested containers, raw HTML/URIs, row/cell/table/depth bombs; rows/cells/controls/anchors checked |
| `reader_decode` | `app::reader::decode` + `flow` + `window` / `top_for` + `status` (raw bytes) | one file the in-app reader opens. `decode` is total; a NUL inside the first 8192 bytes is a refusal whole, a file past 50,000 kept lines the only other one; text keeps the caller's size, at most 50,000 lines of 2000 scalars with no line break or trailing `\r` in any, and a last line that is not blank. `flow` is total at any width: a row for every line, each line's first row in order and inside the rows, past-the-end asking the last line. The viewport math keeps the top inside its bounds. Seeds pin prose, CRLF, empty and blank-tail files, a NUL in and past the probe, invalid UTF-8, escape payloads and C1 controls, CJK, tabs, over-long lines and many short ones |
| `panel_episode` | `autotrust::dismiss_due_at` (fixed session, slot `main`, agent `lead`, one fixed tool start) | one seat journal, one `events.jsonl` record per line, at clocks around its newest record and at both extremes, with no input, old input and input inside the 60 s quiet span. The answer is a function of its records and never doubt; input inside the span holds; a clear appended after the journal closes the run; a reservation for every moment the journal names spends the run, whichever record supplied its first sight. The strict damaged-journal ledger is the caller's and is NOT covered here. Seeds pin one and two namings, a reserved-then-failed Escape, a trust reservation that spends nothing, a clear then reopen, another prompt and a dead alert ending the run, an older or unproven start, a foreign slot or session, the measured `2 of 2` and empty panels, a drifted hint and a damaged line |
| `unreach_fold` | `session::alert_reason_in` (fixed session `s`, slot `main`, agent `lead`) | one seat journal, one `events.jsonl` record per line. The fold is a function of its records; an unreachable `alert` appended last reads `Stale` and keeps it over a later prompt naming and that naming's clear and over an `[unconfirmed]` delivery, yields `Dead` to a dead alert, and reads nothing after a landed delivery, the seat's own record or the reachable-again clear. The strict damaged-journal ledger is the caller's and is NOT covered here. Seeds pin a failure alone, across restarts, ended by an own record but not a foreign seat's, ended by a landed send and its clear, kept over an unconfirmed challenge and over a panel naming and clear, outranked by a dead pane, the plain stale alerts that are no failure, another session and a damaged line |

`console_command` also takes `/more <digits>` lines as one `Effect::More`, every other `/more` shape refused, and `@seat /more n` an ask; the `more-*` seeds carry both.

`console_command` also checks the editing keys against an independent copy of their rules: ^W, Opt+Backspace (`ESC DEL`) and ^K take a word or the rest of the line into the slot ^U fills, ^Y puts it back once and Enter empties it, and ^V asks for an image (`Effect::ImagePaste`) and types nothing. The `kills-and-yank`, `opt-backspace`, `kill-to-end`, `clear-and-yank` and `image-key*` seeds carry them.

`console_command` also holds attachment tokens to an independent copy of the grammar: a restored draft (and a refused line put back) turns each well-formed `[Image #N: "<path>"]` / `[File: "<path>"]` into one atom (at most 32; N 1..=999999 without a leading zero; the path absolute, at most 4096 bytes, no `"`, no control) that the cursor, Delete, Backspace and the word kills take whole, and Enter sends back as its wire text; a typed or pasted lookalike stays text, and an outside `0xFF` byte enters as U+FFFD. The same stream is also fed through `Keys::app` into `Input::keyed`, and the effects, line and layout must be the same for any read size (a paste of file paths is judged again with each fragment). The `token-*` seeds carry them.

`console_command` also feeds the same terminal stream to `Keys::app`, including
SGR mouse reports and legacy X10 payloads. Whole and split reads must yield the
same keys and origin stamps after adjacent literal runs are joined and idle
sequences expire. The chat decoder is checked to emit no mouse keys. Tracked
seeds cover clicks, both wheel directions, a left press dragged and released,
a plain right press (the session menu) beside its dropped release and modified twin,
releases and left motion among modified and other-button reports that stay
dropped, X10 with an ESC payload byte, and an oversized report followed by a
valid click.

NOTE — production also emits `chat`, `focus`, `refused`, `delivery-failed` and `telegram_autostart_refused`, which carry no seeds: `from_json` never branches on `action`, so they add zero coverage to THIS target — but a future target driving `ref_meaning` or `alert_meaning` would need them.

`events_parse` also drives `requests::states` over the hostile journal bytes. The `seat-launch-keys` seed carries both optional seat-incarnation fields; the strict event reader rejects duplicate or non-string launch keys and preserves absent versus empty values. Older cores treat the new keys as unknown data.

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
it. `just release` refreshes it between the bump and the version commit with the
product compiler. Any refresh error stops before committing. The whole lock must
match its committed bytes with only the ae package version updated; unchanged
locks and any other delta stop the release. Deliberate refresh omits `--locked`;
the compile gate and fuzz execution both require a current lock.

To refresh deliberately outside a release (review any dependency changes):

```sh
cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 >/dev/null
git add fuzz/Cargo.lock
```
