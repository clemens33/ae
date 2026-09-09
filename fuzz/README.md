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

## TODO — targets that wait on their parser

- `config_command` — the `[clients]` expansion, not just the lexing. Blocked on
  worker `clients`, whose branch lands the API at a3ca1774:
  `ae::config::IdentityConfig::command(&self, profile: &str, home:
  Option<&std::path::Path>) -> Result<Option<ae::config::ResolvedCommand>,
  ae::config::ConfigError>`. Once that is on main, the target is
  `parse_identity(&text)` then `.command(profile, Some(Path::new("/home/x")))`
  with the profile name taken from the same fuzz bytes — no filesystem in the
  loop. This branch is cut from 9a56eda8 and cannot compile it yet.
- `quota_claude_cache`, `quota_codex_rollout` — added when the quota parsers
  land. Both read vendor-written state ae does not control, which is the
  definition of hostile.

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
