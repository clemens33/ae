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
| `meta_parse` | `meta::Meta::parse` | one session meta document |
| `launch_cmd_lex` | `launch_cmd::lex_simple_command` | one profile command string |
| `config_command` | `config::IdentityConfig::command` | first line the profile, the rest one config text |
| `quota_claude_cache` | `quota::claude::parse` | one Claude settings file, at a fixed and a chosen clock |
| `quota_codex_rollout` | `quota::codex::parse` | first byte the record boundary, the rest a rollout tail |
| `usage_claude_transcript` | `usage::claude::parse` | one Claude assistant transcript JSONL stream |
| `usage_codex_rollout` | `usage::codex::parse_with_head` | first byte the record boundary, then split bounded head/tail Codex rollout bytes |
| `usage_prices` | `usage::prices::parse_row` | one `[prices]` alias row |
| `harness_observed` | `harness_state::{decode_idle, observed_from_option}` | one watchdog-owned `@ae_observed` pane option |
| `picker_agents` | `tmux::parse_picker_agents` | one watchdog-owned `@ae_agents` value at a fixed clock |
| `picker_spend` | `tmux::parse_picker_spend` | one watchdog-owned `@ae_spend` value at a fixed clock |
| `launch_stamp` | `store::parse_launch_attempt` | one `.launch-attempt` stamp's bytes |
| `last_live_epochs` | `inventory::launch_epochs` | one session meta, read for its launch moments |

### The two reboot-proof reducers

`launch_stamp` and `last_live_epochs` are the readers behind the boot-time absence proof
(`docs/internals/stop-identity-contract.md`). Both are on the RESUME and LISTING paths, and
both read files a human edits, so they are fuzzed before the proof they feed is trusted:

- `launch_stamp` reads `.launch-attempt`, whose whole content is one epoch. The reducer is
  BOUNDED (`store::LAUNCH_ATTEMPT_CAP`), so oversize input must be refused rather than
  parsed — the seeds carry that boundary, an unbounded epoch, invalid UTF-8 and the
  non-positive sentinel.
- `last_live_epochs` reads a session meta for its `started` / `launch_time.<slot>` /
  `capture_floor.<slot>` rows. `meta_parse` does NOT reach it: that target drives
  `Meta::parse`, which asks a different question of the same bytes. This one also drives the
  per-row claim reading and the AGGREGATE fold (`tmux::Evidence::claim` and
  `Evidence::folded`), because it folds every row as it reads it.

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
