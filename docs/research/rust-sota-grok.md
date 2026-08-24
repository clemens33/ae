# Rust SOTA crate survey — code-anchored (grok46:cratescan)

Date of survey: 2026-08-24.
Repo: `ae` on `rust-rewrite`. Compiler pin `1.97.1`, `unsafe_code = "forbid"`, musl static Linux target, permissive-only `deny.toml`, zero runtime deps.
Method: read the modules first, then crates.io / docs.rs / RustSec / upstream READMEs. Every factual claim carries a URL and a date. Transitive counts are **direct required deps from crates.io API** plus an estimated full default-feature tree; the estimate is labelled as such.

This is the crate-scan seat. A second seat (agy:rustsota) is surveying independently. Do not treat this as a merge of both.

---

## 0. What the code actually is (so recommendations cannot float)

| Module | Role today | Size / shape | Why it exists (from its own docs) |
|---|---|---|---|
| `src/cli.rs` + `filters::ListArgs::parse` | argv → `Request` | ~119 product lines; 10 documented flags; SC-022 split | "A CLI argument parser is a dependency the skeleton does not need" (`cli.rs` L3–6). Grammar is **not** generic getopt: a top-level bare word is a launch candidate, never an unknown-subcommand; the same token inside `list` is usage-error 2. |
| `src/json.rs` | emit + parse the JSON subset ae owns | ~834 lines, recursive-descent, `MAX_DEPTH = 64` | SC-510d escape set; SC-506 infallible render; insertion-order `Obj(Vec<(String,Value)>)`; `Value::Raw` for non-i64 numbers (SC-511b/c). |
| `src/error.rs` | one presentation error | **one variant**: `Error::Io` | "#80: no error dependency exists until a real error does" (`error.rs` L5–7). |
| `src/transport.rs` | the **one** product `Command` door | `run(program, args) -> (bool, String)` | Status decides, payload never does (SC-017k/l). Spawn failure ≡ failed run. |
| `src/inventory.rs` + `src/session.rs` | `read_dir` walk + meta/events parse | `std::fs` only; doors inventoried in `clippy.toml` | Candidate is a directory; unreadable meta costs facts, never the candidate. |
| `src/time.rs` | one timestamp spelling | epoch-seconds + Hinnant civil arithmetic | SC-510a/SC-509: `YYYY-MM-DDTHH:MM:SSZ` only. Fractional seconds and offsets are **refused**. |
| `src/main.rs` | thin argv-in / exit-code-out | 24 lines | Everything testable lives in the lib. |

Two load-bearing constraints that are **not** generic Rust taste:

1. **First runtime dependency destroys a capability boundary.** `clippy.toml` (read 2026-08-24) states both `disallowed-types = ["std::process::Command"]` and the fs/env method deny are premises that hold *because* all four dependency tables are empty **and** `unsafe_code = "forbid"`. Quote: "the day a dependency lands in ANY of those four tables, or the day `forbid` softens, this stops being a capability boundary and becomes a naming convention." A proc-macro like `thiserror` is a paper weakening; `ureq`/`rustix`/`serde_json` is a real one (those crates can wrap `Command` or syscalls the deny cannot see).
2. **Hostile persisted-state trigger.** `AGENTS.md` Rust-era: "cargo-fuzz is required *before* any hostile persisted-state parser cuts over (a P2/P3 entry condition)." `json.rs` is that parser. Events/meta on disk are agent-writable.

`just rust-deny` currently passes `--allow license-not-encountered` **only because there are zero deps**. First dep removes that flag (`justfile` L328–335).

`deny.toml` allow-list (2026-08-24): MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib. Copyleft, git deps, wildcard versions: disqualified.

---

## 1. CLI parsing — `cli.rs` / `ListArgs::parse`

**What we have.** Hand-rolled. Top-level dispatcher in `cli.rs` is a `match` on the first token. Flag grammar lives in `filters.rs` (`--running|--all|--stopped|--needs-attn|--needs-me|--needs|--attn|--active|--busy|--json`). SC-022 is the load-bearing oddity: option-shaped unknown → usage 2; bare word → `LaunchCandidate`. Last-distinct-selector (SC-521) needs the whole tail in order.

### Candidates

| | **lexopt 0.3.2** | **pico-args 0.5.0** | **clap 4.6.6** |
|---|---|---|---|
| Source | https://crates.io/crates/lexopt (updated 2026-02-28); https://lib.rs/crates/lexopt (crawled 2026-08-17) | https://lib.rs/crates/pico-args (0.5.0 dated 2022-06-04) | https://lib.rs/crates/clap (4.6.6 dated 2026-08-06); crates.io API 2026-08-24 |
| License | MIT | MIT | MIT OR Apache-2.0 |
| MSRV | 1.31 (https://github.com/blyxxyz/lexopt README badge, fetched 2026-08-24) | undocumented; 2018-era | **1.85** (crates.io `rust_version`, 2026-08-24) |
| Direct required deps | **0** | **0** | 1 (`clap_builder =4.6.6`); optional `clap_derive` |
| Transitive (default features) | 0 | 0 | clap_builder → `anstyle` + `clap_lex`; default features also pull `anstream`, `strsim`, `unicode-width`, `terminal_size`; derive pulls `syn`/`quote`/`proc-macro2`. **Estimate 10–15 crates.** |
| Maintenance | 6 releases; last 2026-02-28. Alive, slow. | **Last release 2022-06-04.** High download count but the crate itself is frozen. | 461 versions; 4.6.x in Aug 2026. The SOTA CLI crate. |
| RustSec | none found for the crate name (rustsec.org package pages 404 for empty sets, 2026-08-24) | none found | none found at rustsec.org/packages/clap.html (404, 2026-08-24) |
| Binary / compile | argparse-rosetta (https://github.com/rosetta-rs/argparse-rosetta-rs README, rustc 1.94.0 dated 2026-03-02): **37 KiB** overhead, 329ms debug build | same table: **24 KiB**, 300ms | same table: clap **574 KiB** / clap-minimal **377 KiB** / clap_derive **596 KiB**; 2–4s debug. HN 2025-07-01 still quotes ~690 KiB full / ~430 KiB stripped (https://news.ycombinator.com/item?id=44429695). |
| unsafe | single-file lexer; no libc. Treat as **none of ours**. | none | small; not a syscall crate |
| musl | pure Rust, fine | pure Rust, fine | pure Rust, fine |

**Anchored verdict: keep from-scratch.**

Reasons that are about *this* argv, not generic "hand-rolled is holy":

- SC-022 is the opposite of clap's default world (unknown subcommand = error). clap *can* be beaten into "everything else is a positional" with `allow_external_subcommands` / no-subcommand, but that is fighting the crate to recover a 15-line match. The help layout is explicitly unratified (SC-012); clap's selling point is generated help we must not pin.
- pico-args' documented limitation is parse-in-arbitrary-order (https://lib.rs/crates/pico-args, 2026-08-24): `--arg1 --arg2 value` can steal the next token. That is exactly how SC-521 last-distinct-selector gets lost.
- lexopt is the only crate that would *not* fight the grammar — it is a lexer, we would still write the same `match`. Replacing 119 lines with a 0-dep lexer plus the same match is not less code. lexopt's own README: "tedious to use", "may not be worth using if you hate boilerplate" (fetched 2026-08-24).
- Command surface today: `list`/`ls`, `--help`, `--version`, launch-candidate. Revisit when P5 grows a real subcommand tree (end/stop/rename/transfer/spawn-from-CLI). Even then prefer **lexopt** over clap: 37 KiB vs 574 KiB, 0 deps, and we already write help by hand.

`clap_lex` (28 KiB, 0 deps, MSRV 1.85, https://lib.rs via argparse-rosetta 2026-03-02) is clap's own lexer extracted. Same shape as lexopt. Same "would not shrink `cli.rs`" conclusion.

---

## 2. JSON — `src/json.rs`

**What we have.** Both directions. Write path is the contract: SC-510d escape set (`\" \\ \n \t \r` plus `\u00XX` for remaining C0). Render is infallible (SC-506). Objects are insertion-ordered `Vec` pairs — tests pin a documented event line round-tripping **byte-identical** (`json.rs` "a_documented_event_line_round_trips"). Parse is tolerant of unknown keys (SC-511b) and non-i64 numbers (`Value::Raw`). Nesting cap 64, explicit, tested. This is the parser that will face **hostile persisted state** at P2/P3.

### Candidates

| | **serde + serde_json 1.0.151** | **miniserde 0.1.46** | **keep `json.rs`** |
|---|---|---|---|
| Source | https://lib.rs/crates/serde_json (1.0.151, 2026-07-20); crates.io API 2026-08-24; Cargo.toml via docs.rs 2026-08-24 | https://lib.rs/crates/miniserde (0.1.46, 2026-07-18) | this tree |
| License | MIT OR Apache-2.0 | MIT OR Apache-2.0 | n/a |
| MSRV | 1.71 | 1.71 | ours 1.97.1 |
| Direct required | `itoa`, `memchr` (default-features=false), `serde ^1.0.220`, `serde_core ^1.0.220`, `zmij` = **5**. Optional `indexmap` for `preserve_order`. | `itoa`, `zmij`, `mini-internal` (proc-macro → syn/quote/proc-macro2) = **3** | 0 |
| Transitive estimate | 5–8 runtime; +syn tree if `serde_derive` | 2 runtime + syn tree at compile | 0 |
| Maintenance | 183 versions, 1.2B downloads, dtolnay. **The** JSON crate. | 47 releases, same author, still labelled "prototype" in its own README (lib.rs, 2026-08-24) | we own it |
| RustSec | no crate-level advisory found 2026-08-24 (package page 404). Recursion DoS is a **documented design**: default nest cap 128 (https://github.com/serde-rs/json/issues/162, 2016-10-26, still the model; `unbounded_depth` + `serde_stacker` to exceed it, docs.rs/serde_stacker 2026-08-20). | none found | unfuzzed |
| Fuzzing | **OSS-Fuzz's example Rust project** is serde_json (https://google.github.io/oss-fuzz/getting-started/new-project-guide/rust-lang/, fetched 2026-08-24; project.yaml/Dockerfile/build.sh all named `serde_json`). In-tree `fuzz/` on https://github.com/serde-rs/json. | not in OSS-Fuzz | **none yet** — and doctrine requires cargo-fuzz before P2/P3 cutover |
| unsafe | some (perf, validated UTF-8). Allowed in a dep. | low | none |
| musl | pure Rust, fine | pure Rust, fine | fine |
| Binary / compile | serde_json is ~13k SLoC (lib.rs); plus serde_core. Typical "add serde_json" is hundreds of KiB and a noticeable debug compile. `preserve_order` adds indexmap. | ~2.5k SLoC; no monomorphization (miniserde README). Smaller than serde, larger than ours. | already paid |

### Does serde_json's maturity beat fuzzing our own?

**No — not for this module, not at this phase.** Maturity substitutes for *rewriting a JSON grammar*. It does not substitute for:

1. **Contract mismatch on object order.** `serde_json::Map` without `preserve_order` is a **sorted** map: "all JSON maps are always kept in a sorted state" (https://docs.rs/serde_json/1.0.151/serde_json/struct.Map.html, 20 July 2026). Our `Value::obj` preserves insertion order; the events.md example round-trips in documented key order. Enabling `preserve_order` pulls `indexmap` (more deps) and still is not the same type as `Vec<(String,Value)>`.
2. **Contract mismatch on numbers.** We split i64 vs `Raw` so an additive float key cannot break a reader (SC-511b/c). `serde_json::Number` is a different algebra (u64/i64/f64). Mapping it into our digest/event types is new code, not deleted code.
3. **Contract mismatch on write escapes.** SC-510d names the set ae *writes*. serde_json emits a legal-JSON superset; a byte-identical event line is no longer a property we can test without a custom serializer — at which point we have reimplemented `escape_into`.
4. **SC-506.** `Value::render` is infallible. `serde_json::to_string` is `Result`. The document-must-close invariant is currently a type-system fact; a `Result` reopens the mid-array truncate class the module exists to make unrepresentable.
5. **Recursion.** serde_json is recursive (stack) with a 128 cap; we are recursive with a 64 cap, tested. miniserde is the one crate that is *non-recursive* (README, 2026-08-24) — that is a real safety property — but miniserde derive "refuses anything other than a braced struct with named fields or an enum with C-style variants" and "no deserialization error messages." SC-511b unknown keys of arbitrary shape do not fit that derive. miniserde's `Value` exists, but then we are back to an untyped tree we already have, with worse errors and no insertion-order guarantee.
6. **Fuzzing the mapping, not the grammar.** Even if we imported serde_json tomorrow, P2/P3 still requires cargo-fuzz of *our* event/meta readers (unknown keys, `Raw`, generation-aware drain). The expensive bugs in this crate have been semantic (prefix match deleting a session, status-dropped-on-empty-stdout), not `\u` decoder mistakes. OSS-Fuzz on serde_json does not fuzz SC-017j.

**Verdict: keep from-scratch.** At P2 entry, **cargo-fuzz `json::parse` and the event-line reader** (doctrine, not taste). Escape hatch, recorded: if fuzzing spends more time finding *grammar* bugs than *mapping* bugs, swap the parse half to serde_json **without** `preserve_order`/`derive`, keep our `Value` + renderer, and treat serde_json as a lexer. That is a measured-need cut, not a now cut.

serde_json maturity is an argument to **not invent a second parser later** (e.g. a Telegram JSON body). It is not an argument to delete a parser that already encodes the rows.

---

## 3. Error handling — `src/error.rs`

**What we have.** One variant. Display + `source()` + `From<io::Error>`. The file tells you when to stop: "`thiserror` earns its place when the enum has enough variants to make the boilerplate cost real."

### `thiserror` 2.0.20

- Source: https://crates.io/crates/thiserror (2.0.20, created 2026-08-08); https://lib.rs/crates/thiserror
- License: MIT OR Apache-2.0
- MSRV: 1.71
- Direct deps: `thiserror-impl =2.0.20` (proc-macro). Runtime transitive: **0**. Compile-time: proc-macro2, quote, syn 3.
- Maintenance: 91 versions, dtolnay, 1.35B downloads. The SOTA error-derive.
- RustSec: none found 2026-08-24. (Note: **anyhow** has RUSTSEC-2026-0190, unsoundness, issued 2026-06-25, https://rustsec.org/advisories/RUSTSEC-2026-0190.html — do not take anyhow as a "free" alternative.)
- Binary: ~0 (macro only). Compile: syn is the cost, already the most expensive rustc crate in most graphs.
- musl: n/a
- unsafe: none in the product crate

**Verdict: keep from-scratch.** The enum has one variant. thiserror's own README (crates.io, 2026-08-24): "switching from handwritten impls to thiserror or vice versa is not a breaking change" — so the option stays cheap. Adopt **with the feature that needs it**: when P4/P5 grows a real taxonomy (tmux / archive / git / HTTP / telegram) and the `match` + `From` boilerplate is measured in tens of variants, not one. Even then it is a **compile-time** dep; still the first row in `[dependencies]`, still a paper hit on the clippy.toml premise. Prefer to delay until a second *runtime* dep is landing in the same commit so the boundary conversation happens once.

Do not take `anyhow`. It is the application-shaped crate; we have a library-shaped `Error` on purpose (`main.rs` presents it). And it has a 2026 advisory.

---

## 4. Filesystem / unix — `inventory.rs`, `session.rs`, `meta.rs`, `events.rs`

**What we have.** `std::fs::{read_dir, read_to_string, File, metadata}`. Lossy UTF-8 names kept as candidates. `NotFound` on a root is an empty answer, not a failure. `clippy.toml` inventories every read of the outside world. No flock, no unix sockets, no `openat`, no pidfds. `transport.rs` is `Command`, not a raw `execve`.

### Candidates

| | **rustix 1.1.4** | **nix 0.31.3** | **std** |
|---|---|---|---|
| Source | https://lib.rs/crates/rustix (1.1.4, 2026-02-22) | https://lib.rs/crates/nix (0.31.3, 2026-05-11) | std |
| License | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (in deny allow-list) | MIT | MIT-Apache via rustc |
| MSRV | 1.63 | 1.69 | n/a |
| Direct required | bitflags, errno (cfg), libc (cfg), linux-raw-sys (cfg), windows-sys (cfg). Linux default is **linux_raw** (asm, no libc). | bitflags, cfg-if, libc | 0 |
| Transitive on our targets | Linux musl: bitflags + linux-raw-sys (linux_raw is default on Linux x86-64/aarch64 per rustix README, fetched 2026-08-24). macOS: libc + errno + bitflags. | libc + bitflags + cfg-if | 0 |
| Maintenance | Bytecode Alliance, 160 releases, 82M downloads/month | nix-rust, 77 releases, 56M/month | — |
| RustSec | none as a published advisory (an unaccepted DoS discussion exists: https://github.com/rustsec/advisory-db/issues/1808, 2023-10-23). | **RUSTSEC-2021-0119** / CVE-2021-45707, OOB write in `getgrouplist`, patched `>=0.23.0` (https://rustsec.org/advisories/RUSTSEC-2021-0119.html, last modified 2023-06-13). Current 0.31.3 is patched. History is real. | — |
| unsafe | **Heavy.** linux_raw is `asm!` syscalls. This is the crate you take *because* you want to own syscalls. | Wraps libc `unsafe`. | std's, already in the binary |
| musl | linux_raw works on Linux musl (it is still Linux). libc backend has had musl holes (https://github.com/bytecodealliance/rustix/issues/1462, 2025-05-06, missing MADV_* on riscv musl). Our targets are x86_64-musl + aarch64-darwin: should build. **NSS caveat** in AGENTS.md is about musl `getaddrinfo`/`getpwuid`, which rustix does not fix. | libc on musl: works; getgrouplist/NSS is exactly the class of bug they already shipped | works today |

**Verdict: keep from-scratch (std).** rustix/nix would **add** code (new types, new error algebra) in front of `read_dir` we already understand. They do not make `child_dirs` smaller. They would also be the first crate that can perform syscalls the clippy deny cannot name — the exact failure `clippy.toml` warns about.

Adopt **with the feature that needs it**, and the feature is not inventory:

- P4 watchdog/telegram: `flock` on a lockfile, `unix` sockets (sun_path 104 on macOS — AGENTS.md already documents this), maybe `signalfd`/`timerfd`. Then **rustix** (I/O-safe fds, linux_raw, Bytecode Alliance) over nix (libc, past RUSTSEC, not I/O-safe by default).
- Do not pull rustix "for fs walking." `std::fs` is the walking crate.

---

## 5. Time — `src/time.rs`

**What we have.** One spelling, 20 bytes, Hinnant `days_from_civil` / `civil_from_days` (public domain, cited in-module). Tests refuse `.500Z`, `+02:00`, space separator, lowercase `z`, leap-second `60`, 1900-02-29. `now()` is `SystemTime`.

### Candidates

| | **jiff 0.2.35** | **chrono 0.4.45** | **`time` crate** | **std + this module** |
|---|---|---|---|---|
| Source | https://lib.rs/crates/jiff (0.2.35, 2026-07-25) | https://lib.rs/crates/chrono (0.4.45, 2026-06-04) | https://rustsec.org/packages/time.html (page dated 2026-02-06) | this tree |
| License | **Unlicense OR MIT** (MIT is on the allow-list; Unlicense alone would not be) | MIT OR Apache-2.0 | MIT OR Apache-2.0 | n/a |
| MSRV | 1.70 | 1.62 | (not surveyed in depth; see RustSec) | — |
| Direct required | jiff-core, jiff-static, portable-atomic, portable-atomic-util. README claims "zero dependencies on Unix" historically; crates.io 2026-08-24 shows the 0.2.35 split. | `num-traits`; default `clock`/`now` add `iana-time-zone` on Unix | not recommended | 0 |
| 1.0 status | **Not 1.0.** Author (2026-04 note in README, still on 0.2.35 in Jul 2026): "I don't currently have a timeline for a Jiff 1.0 release." | 0.4 since 2014-ish. Stable-in-practice, never 1.0. | 0.3 | — |
| RustSec | none found | **RUSTSEC-2020-0159** / CVE-2020-26235, `localtime_r` segfault, patched `>=0.4.20` (https://rustsec.org/advisories/RUSTSEC-2020-0159.html, last modified 2022-08-04). Current 0.4.45 is patched. The bug class is "env var set from another thread." | **RUSTSEC-2020-0071** (segfault, sibling of chrono's) **and RUSTSEC-2026-0009** (MEDIUM, stack-exhaustion DoS, issued 2026-02-06, https://rustsec.org/packages/time.html). **Disqualified for new code.** | n/a |
| Binary | 53k SLoC crate; TZDB embed on Windows. Unix can use `/usr/share/zoneinfo`. Far larger than we need. | 20k SLoC | — | ~330 lines |
| musl | Unix TZ files; no NSS. Fine. | `clock` feature talks to the OS; NSS caveat irrelevant for UTC | — | `SystemTime` only |

**Verdict: keep from-scratch.** A datetime crate's value is time zones, DST, RFC 3339 *tolerance*, parsing many spellings. Our rows forbid that tolerance. Wrapping jiff/chrono to *reject* `.500Z` is more code than Hinnant plus a 20-byte scanner, and it would make "we accept what the crate accepts" the silent default the tests currently prevent.

If P4 telegram timestamps or a human-facing local-time column appears, **jiff** (BurntSushi, Temporal-shaped, no 2020 segfault class) over chrono (history) over `time` (fresh 2026 advisory). Until then, `Timestamp` is the crate.

---

## 6. P4 daemon stack — HTTP client, async-or-threads

Nothing in this crate speaks HTTP today. P4 is watchdog + telegram. AGENTS.md already flags: musl has no NSS; `panic = "abort"` may want revisiting so a loop can survive a panic.

### HTTP clients

| | **ureq 3.4.0** | **minreq 3.0.0** | **reqwest** (surveyed only to reject) |
|---|---|---|---|
| Source | https://lib.rs/crates/ureq (3.4.0, 2026-08-08) | https://lib.rs/crates/minreq (3.0.0, 2026-06-15) | rustify.rs 2026-06-02 (https://rustify.rs/articles/rust-reqwest-vs-ureq-vs-hyper-2026) |
| License | MIT OR Apache-2.0 | **ISC** (on allow-list) | MIT OR Apache-2.0 |
| MSRV | **1.85** | 1.63 without features; TLS features newer | typically 1.8x + tokio |
| Direct required | base64, log, percent-encoding, ureq-proto, utf8-zero. Default features: **rustls + gzip**. | **0** without features | tokio + hyper + rustls/native-tls… |
| Transitive with TLS | ureq-proto → http, httparse, base64, log; rustls 0.23 → rustls-pki-types, rustls-webpki, once_cell, subtle, zeroize; default crypto provider **ring** (ureq README, 2026-08-24). **Estimate 25–40 crates.** gzip adds flate2. | 0 without TLS; `https-rustls` pulls the rustls+ring (or rustls-platform-verifier) tree | larger than ureq; async runtime on top |
| Maintenance | 113 releases, 18.6M downloads/month, "simple, safe HTTP client", **forbids unsafe** | 52 versions, 444k downloads/month, smaller community | SOTA async |
| RustSec | none found 2026-08-24. (surf, an old async client, was declared unmaintained with ureq as the alternative: RUSTSEC-2026-0169, 2026-06-04, https://rustsec.org/advisories/RUSTSEC-2026-0169.html) | none found | n/a |
| unsafe | ureq: **forbids**. rustls: typically forbid. **ring: heavy unsafe + asm** (https://crates.io/crates/ring 0.17.14, 2025-03-11; license Apache-2.0 AND ISC — both on allow-list). | minreq itself small; TLS is the same ring/rustls question | tokio/hyper/ring |
| musl static | rustls is the documented path for static musl (https://oneuptime.com/blog/post/2026-03-04-compile-rust-musl-static-binaries-rhel/view, 2026-03-04). **native-tls / OpenSSL vendored: disqualified** (C build, not the zero-dep static artifact). ring is the known-working musl TLS primitive; aws-lc-rs is the one that usually fights musl. ureq documents that the default provider is currently ring and "might change in a minor version" (README, 2026-08-24) — pin the provider, do not trust the default forever. | same TLS story once `https-rustls` is on | tokio+rustls can static; native-tls cannot cheaply |
| Binary | TLS is the size. rustls+ring is typically **1–3 MiB** of the binary. gzip extra. **Do not enable ureq's `json` feature** — it pulls serde_json, which we just declined. | author-measured: **+148 KiB** stripped without TLS vs hello-world (minreq README, rustc 1.94.0; lib.rs 2026-08-24). TLS dominates once enabled. | largest |

**Telegram needs TLS.** minreq-without-features cannot talk to `api.telegram.org`. The 0-dep story ends at HTTPS.

**Verdict: adopt with the feature that needs it (P4 telegram/bridge), crate = ureq 3.x, features = rustls (+ gzip if bodies need it), `json` off, native-tls off.** Pin a `CryptoProvider` so a silent ring→aws-lc-rs switch cannot break musl. Use our `json.rs` (or a tiny write of the handful of Telegram request objects) on the body. minreq is the right *shape* for a 148 KiB HTTP/1.1 client, but once TLS is on the graphs converge and ureq is the maintained, unsafe-free-at-the-HTTP-layer one (3.4.0 in Aug 2026 vs minreq's smaller ecosystem).

### Async or threads

ae today is a short-lived CLI: enumerate, exec tmux, print, exit. A telegram poll loop is one blocking socket and a sleep. **std::thread + blocking ureq**, not tokio.

- tokio is a runtime, not a library: multi-crate graph, `unsafe`, different panic/shutdown story than `panic = "abort"`.
- ureq's own README (2026-08-24): "It uses blocking I/O instead of async I/O, because that keeps the API simple and keeps dependencies to a minimum."
- rustify.rs 2026-06-02: "Choose ureq when blocking I/O, tiny binaries, and fast compile times matter."
- Revisit async only if P4 grows concurrent TLS to many hosts (it will not: one bot API, one watchdog tick).

`panic = "abort"` (Cargo.toml, revisit-at-P4 comment): a daemon that must survive a panic in one iteration should flip to unwind **for the daemon binary** or catch at a process supervisor. That is a profile decision, not a crate decision. Do not take tokio to get `catch_unwind`.

---

## 7. Dev-tooling gaps

These are **tools**, not `[dependencies]`. They do not hit the capability boundary unless they land in `[dev-dependencies]` (clippy.toml lists that table as one of the four). A justfile pin like cargo-deny does not.

### cargo-fuzz 0.13.2 — **adopt with P2 (doctrine)**

- https://crates.io/crates/cargo-fuzz (0.13.2, 2026-06-09); https://github.com/rust-fuzz/cargo-fuzz; book https://rust-fuzz.github.io/book/cargo-fuzz.html (fetched 2026-08-22)
- License: MIT OR Apache-2.0
- Nightly + libFuzzer; Unix; x86_64 and Aarch64. **Not a musl-ship concern** (dev machine / CI).
- `AGENTS.md` already named this as a P2/P3 **entry condition**. Targets: `json::parse`, event-line parse, meta INI-ish parse if that is still hand-rolled at cutover.
- Does not replace serde_json. It *is* how we keep json.rs.

### cargo-vet 0.10.2 — **adopt with the first runtime dep**

- https://lib.rs/crates/cargo-vet (0.10.2, 2026-01-13); book https://mozilla.github.io/cargo-vet/ (fetched 2026-08-24)
- License: Apache-2.0/MIT
- MSRV 1.82
- **Orthogonal to cargo-deny.** deny = licenses + RustSec + bans + sources. vet = "a trusted human audited this crate." Mozilla's book: audits can be shared, relative, deferred.
- With zero deps, vet is empty theatre. The day ureq (and ring) land, vet is how we live with 25–40 crates we will not personally read. **Do not run it as a second RustSec lane** — that is what `just rust-deny` is, and AGENTS.md already forbids duplicating cargo-audit.

### cargo-semver-checks 0.50.0 — **keep unused**

- https://lib.rs/crates/cargo-semver-checks (0.50.0, 2026-08-01)
- License: Apache-2.0 OR MIT
- MSRV **1.93** (fits 1.97.1)
- Checks public API breakage vs a baseline. Our crate is `publish = false`, version `0.0.0`, CalVer/semver unresolved until **P5 entry flip** (AGENTS.md). A semver lane on a crate that does not promise semver is a green badge that means nothing. Revisit at P5 if `ae` the library is a thing other people depend on.

### insta 1.48.0 — **do not adopt**

- https://lib.rs/crates/insta (1.48.0, 2026-06-11)
- License: **Apache-2.0** (on allow-list)
- MSRV 1.66
- Direct deps: once_cell, similar, tempfile
- Snapshot tests of `list --json` look tempting. They fight this repo's actual test doctrine: cargo-mutants is the agent-specific lane; agents already write tests that pass; a snapshot the agent can `--accept` is a mutant that never goes red. Digest member *order* is an open choice (phase-3 criterion 15); pinning rendered bytes in `.snap` files would silently ratify an unratified order. Keep explicit `same_members` / field asserts.

### proptest 1.11.0 vs quickcheck 1.1.0

| | **proptest 1.11.0** | **quickcheck 1.1.0** |
|---|---|---|
| Source | https://lib.rs/crates/proptest (2026-03-24) | crates.io API 2026-08-24 (updated 2026-02-10) |
| License | MIT OR Apache-2.0 | Unlicense OR MIT |
| MSRV | **1.85** (stated); "fairly close to feature-complete… passive maintenance" (README via lib.rs) | 1.85 |
| Direct | bitflags, num-traits, rand 0.9, rand_chacha, rand_xorshift, unarray | rand 0.10 |
| Why proptest | per-value strategies + shrinking; the date-parser tutorial is *this module's* bug class | type-level Arbitrary; weaker shrinking |

**Verdict: adopt proptest as a `[dev-dependency]` with the P2 fuzz work, not instead of cargo-fuzz.** Use it on `Timestamp::parse` (alphabet of near-miss 20-byte strings) and `json::parse` (structured well-formed + garbage). cargo-fuzz finds crashes/hangs/unbounded memory; proptest finds "this legal-looking date is accepted." Skip quickcheck; proptest is the one that shrinks.

Do **not** put proptest in the product graph.

---

## 8. Scoreboard (one line each)

| Concern | Crate if any | When | Verdict |
|---|---|---|---|
| CLI | — | P5 subcommand explosion → lexopt, never clap | **keep from-scratch** |
| JSON | — ; cargo-fuzz the parser | P2 entry | **keep from-scratch** |
| Error | thiserror 2 | when variants are many, preferably same commit as first runtime dep | **keep from-scratch** |
| FS/unix | rustix 1 | P4 flock/sockets only | **keep from-scratch** |
| Time | jiff 0.2 | only if we grow TZ/RFC3339 | **keep from-scratch** |
| HTTP | ureq 3 + rustls + ring, json feature off | P4 telegram | **adopt with the feature** |
| Async | std::thread | P4 | **not tokio** |
| cargo-fuzz | 0.13.x as a just pin | **P2, required** | **adopt with the feature** |
| cargo-vet | 0.10.x as a just pin | first runtime dep | **adopt with the feature** |
| cargo-semver-checks | — | P5 if library API is real | **keep unused** |
| insta | — | never, given mutants | **keep unused** |
| proptest | 1.11 as dev-dep | P2 alongside fuzz | **adopt with the feature** |

---

## 9. The first-dep cost (pay it once, on purpose)

When the first `[dependencies]` row lands (almost certainly ureq at P4, unless P2 panics and takes serde_json):

1. Remove `--allow license-not-encountered` from `just rust-deny`.
2. Re-read `clippy.toml`. The Command/fs denies become naming conventions. Either (a) accept that and keep the inventory tests as defence-in-depth, or (b) add a supply-chain allowlist of crates permitted to touch the world (`ureq`, maybe `rustix`) and keep the deny for everything else — clippy cannot do (b) across crate bounds; it is a review rule + `cargo deny` bans.
3. Turn on cargo-vet; audit ring in particular (asm, `unsafe`, crypto).
4. Measure the musl artifact: `readelf -l` still no `PT_INTERP`; `file` still says static. ring has historically been the musl-static TLS stack; re-prove it, do not cite 2020 blog posts as proof.
5. Do not enable incidental features (`ureq/json`, `chrono/serde`, `clap/derive`).

---

## 10. One-paragraph judgment — doctrine or dogma?

Zero dependencies is **doctrine with a recorded trigger**, not a purity cult, and the code already says so three times: Cargo.toml ("deps arrive with the feature that needs them"), error.rs ("until a real error does"), cli.rs (same sentence). The line is: **std is enough for every byte this crate currently owns** (argv of ten flags, a JSON subset whose *rows* are the reason it is not serde, one timestamp spelling, a `read_dir` walk, one `Command` to tmux). Crossing the line for clap/thiserror/chrono/nix/jiff today would spend the first-dep budget — deny.toml relaxation, clippy boundary becoming a convention, syn in the compile, audit surface — on crates that do not delete corresponding code and that in several cases *fight* the semantic contract (clap vs SC-022, serde_json vs insertion order and infallible render, chrono/`time` vs "one spelling"). The line is crossed for things std cannot do: **TLS HTTP** (ureq+rustls+ring at P4), **fuzzing a hostile parser** (cargo-fuzz at P2, as already written down), and **human audit of a 30-crate TLS graph** (cargo-vet with that same first dep). A from-scratch TLS stack, or a from-scratch JSON parser we refuse to fuzz, would be the dogma version of the same sentence. serde_json's OSS-Fuzz maturity is a reason not to write a *second* parser later, and a recorded escape hatch if cargo-fuzz says our grammar is the expensive part; it is not a reason to delete `json.rs` before that measurement exists.

---

## 11. Sources (index)

crates.io API, User-Agent `ae-research/1.0`, pulled 2026-08-24, for: lexopt, pico-args, clap, clap_builder, clap_lex, serde_json, serde, serde_core, miniserde, mini-internal, thiserror, thiserror-impl, rustix, linux-raw-sys, nix, jiff, jiff-core, chrono, ureq, ureq-proto, minreq, insta, proptest, quickcheck, cargo-fuzz, cargo-vet, cargo-semver-checks, ring, rustls, http, itoa, zmij, bitflags, libc, errno.

lib.rs crate pages crawled 2026-08-17–2026-08-24 as cited inline.

RustSec: RUSTSEC-2020-0159 (chrono, last modified 2022-08-04), RUSTSEC-2021-0119 (nix, last modified 2023-06-13), RUSTSEC-2020-0071 and RUSTSEC-2026-0009 (`time`, page 2026-02-06), RUSTSEC-2026-0190 (anyhow, 2026-06-25), RUSTSEC-2026-0169 (surf unmaintained, 2026-06-04), RUSTSEC-2026-0214 (gumdrop unmaintained, 2026-07-23).

Other: argparse-rosetta-rs README (rustc 1.94.0 / 2026-03-02), OSS-Fuzz Rust guide (fetched 2026-08-24), Mozilla cargo-vet book (fetched 2026-08-24), lexopt README (fetched 2026-08-24), serde_json Map docs 1.0.151 (20 July 2026), ureq/miniserde/jiff/rustix READMEs via lib.rs, oneuptime musl+TLS (2026-03-04), rustify reqwest-vs-ureq (2026-06-02), HN clap size (2025-07-01).

In-tree (read 2026-08-24): `src/{lib,cli,json,error,transport,inventory,session,time,main}.rs`, `Cargo.toml`, `deny.toml`, `clippy.toml`, `justfile` rust-* block, `AGENTS.md` Rust-era section.
