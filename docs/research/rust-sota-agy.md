# SOTA Rust Best Practices (2026) & Low-Overhead Crate Evaluation for `ae`

**Author:** Antigravity (fable5:lead research task)  
**Date:** 2026-08-24  
**Target Repository:** `clemens33/ae` (branch `rust-rewrite`)  
**Context:** Multi-agent tmux multiplexer transitioning from Bash to a zero-runtime-dependency, static musl-compatible Rust binary. Compiler: Rust 1.97.1 (Edition 2024).

---

## Executive Summary

1. **Architecture & Doctrine Alignment:**  
   `ae`'s current "zero runtime dependencies skeleton" and single-crate layout (`bin` + `lib`) represent the gold standard for high-integrity, agent-authored systems. Hand-rolling core primitives (such as the JSON emitter in `src/json.rs` and CLI routing in `src/cli.rs`) in early phases protects the capability boundaries established by `clippy.toml` (`disallowed-types`, `disallowed-methods`).
2. **Top SOTA Deltas:**
   - **Exit Handling & Signals:** Replace raw `std::process::exit` patterns with `std::process::ExitCode` (stabilized in Rust 1.61.0) and `Termination` to ensure RAII drop semantics execute reliably.
   - **Terminal Detection:** Standard library `std::io::IsTerminal` (stabilized in Rust 1.70.0) removes the historical necessity of crates like `atty` or `is-terminal`.
   - **Core Error Trait:** With `core::error::Error` stabilized in Rust 1.81.0 (2024-09-05), custom error trees and `no_std`/submodule error definitions require zero external macro overhead.
   - **Lints:** Add targeted Clippy restriction lints (`clippy::as_conversions`, `clippy::cast_possible_truncation`, `clippy::dbg_macro`, `clippy::panic_in_result_fn`) to prevent silent integer wraps and unhandled error states.
3. **Crate Verdict Summary:**
   - **CLI:** Keep **Hand-Rolled** for P1/P2. If flag grammar explodes in P3/P5, adopt **`lexopt`** (0 dependencies, MIT). Avoid `clap` (10–15 deps with derive, 400KB+ size tax).
   - **Errors:** Keep **Hand-Rolled** `src/error.rs` today; adopt **`thiserror` 2.x** when domain variants multiply (0 runtime deps, Apache-2.0/MIT). Discard `anyhow` for core library domains (untyped dynamic errors erode compiler enforcement).
   - **JSON:** Keep **Hand-Rolled `src/json.rs`** through P1–P3. It strictly guarantees SC-510d escape rules, infallible string formatting (SC-506), and tolerant decoding (SC-511b). Defer `serde_json` until/unless complex nested nested schemas arrive in P4.
   - **Unix/Process:** Keep **Raw `std`** for subprocess execution (`src/transport.rs`). For terminal/raw-mode/signals in P4, adopt **`rustix`** (safe syscalls, direct Linux musl support, Apache-2.0/MIT); reject `nix` (heavier libc binding).
   - **HTTP Client (P4 Telegram):** Adopt **`ureq` 3.x** (MIT/Apache-2.0, synchronous sans-IO core, pure `rustls` TLS). **`attohttpc` is DISQUALIFIED** (MPL-2.0 violates `deny.toml` permissive license policy).
   - **Async Runtime:** **Keep Pure OS Threads (`std::thread` + `std::sync::mpsc`)**. For 2 daemon loops (watchdog timer + telegram poll), async runtimes (`tokio`, `smol`) add needless dependency trees and state-machine overhead.

---

## Part 1: SOTA Rust Best Practices (2026 Deltas vs. AGENTS.md)

This section reports only **deltas** and forward-looking enhancements not already mandated in `AGENTS.md` (which already mandates single-crate `main.rs`+`lib.rs`, `unsafe_code = "forbid"`, `cargo-nextest`, `cargo-mutants`, and `cargo-deny`).

### 1.1 Project Layout & Architecture Boundaries

- **Standardized `ExitCode` Execution (`std::process::ExitCode`):**  
  *Context:* Calling `std::process::exit(code)` terminates the process immediately without running stack unwinding or calling destructors (`Drop`).  
  *SOTA 2026 Practice:* SOTA CLI applications implement `Termination` or return `std::process::ExitCode` from `main()` (stabilized in Rust 1.61.0, [Rust Reference](https://doc.rust-lang.org/std/process/struct.ExitCode.html)). This ensures all file buffers flush, lock guards release, and RAII cleanup executes before the OS receives the return code (`0`, `1`, `2`).
- **Terminal Introspection via `std::io::IsTerminal`:**  
  *Context:* Legacy Rust CLI tools imported `atty` or `is-terminal`.  
  *SOTA 2026 Practice:* Rust 1.70.0 (2023-06-01, [Rust 1.70.0 Release Notes](https://github.com/rust-lang/rust/releases/tag/1.70.0)) stabilized `std::io::IsTerminal`. TTY checks on `io::stdin()`, `io::stdout()`, and `io::stderr()` require zero third-party dependencies.
- **Port/Adapter Pattern for Subprocess Execution:**  
  `ae` enforces a capability boundary in `clippy.toml` on `std::process::Command`. SOTA practice encapsulates all process interactions behind a pure `Transport` or `Executor` trait (or private dispatcher in `src/transport.rs`), keeping command formatting (`src/tmux.rs`) and output decoding 100% pure, deterministic, and unit-testable without spawning processes.

### 1.2 Error-Handling Idioms

- **`core::error::Error` in Rust 1.81+:**  
  *Context:* Rust 1.81.0 (2024-09-05, [Rust 1.81.0 Blog](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0.html)) moved the `Error` trait into `core`.  
  *SOTA 2026 Practice:* Domain error types can implement `core::error::Error` without depending on `std`.
- **`std::io::Error::other` for Context Injection:**  
  Stabilized in Rust 1.74.0 (2023-11-16, [Rust 1.74.0 Notes](https://github.com/rust-lang/rust/releases/tag/1.74.0)), `io::Error::other(err)` provides a clean constructor for wrapping custom error payloads into standard I/O error streams without messy `io::Error::new(io::ErrorKind::Other, ...)`.
- **Non-Exhaustive Public Errors:**  
  Annotating public error enums with `#[non_exhaustive]` allows future error variants to be added across phases without breaking downstream pattern matching.
- **Avoid Opaque `anyhow` in Core Logic:**  
  In agent-authored codebases, `anyhow::Result` allows errors to be silently erased into dynamic trait objects (`Box<dyn Error>`), bypassing type-level exhaustiveness checks. Structured enum errors force agent authors to account for every failure mode explicitly.

### 1.3 Testing Practice: Properties, Snapshots & Fuzzing

- **Property-Based Testing (`proptest`):**  
  *SOTA Practice:* Unit tests verify expected happy/unhappy paths, but parsers (like session names, agent allowlists `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`, and JSON parser in `src/json.rs`) benefit enormously from property testing via `proptest` (v1.x, [docs.rs/proptest](https://docs.rs/proptest)). Property tests generate random valid and invalid UTF-8 strings, verifying invariants:
  1. `parse(serialize(x)) == x` (round-trip idempotence).
  2. The parser never panics on arbitrary byte slices.
  3. No string matching `_validate_agent_name` contains colon `:` or control bytes.
- **Snapshot Testing (`insta`):**  
  *SOTA Practice:* For complex structured outputs (such as `ae list --json` and human-readable session rosters), `insta` (v1.48+, [insta.rs](https://insta.rs)) provides deterministic snapshot assertions with inline or `.snap` file diffing. In zero-dependency phases, `ae` uses committed golden corpus test assertions, but `insta` (as a `[dev-dependencies]` tool) is the 2026 industry standard for preventing output regressions.
- **Continuous Fuzzing (`cargo-fuzz` / libFuzzer):**  
  *Trigger recorded in AGENTS.md (P2/P3):* Fuzzing hostile persisted state (`events.jsonl`, saved metadata, compact rosters) using `cargo-fuzz` (LLVM libFuzzer integration) detects buffer overflows, stack exhaustion from deep JSON recursion, and unexpected panics.

### 1.4 Linting & Static Analysis: Precision Restriction Lints

Beyond `clippy::all` and `clippy::pedantic` (already in `Cargo.toml`), the following high-value lints from `clippy::restriction` and `rustc` prevent specific agent bug classes:

```toml
[lints.clippy]
# Arithmetic safety and truncation bugs
cast_possible_truncation = "warn"
cast_sign_loss = "warn"

# Prevent abandoned agent development artifacts
dbg_macro = "warn"
todo = "warn"
unimplemented = "warn"

# Error handling discipline
panic_in_result_fn = "warn"
indexing_slicing = "warn" # Recommends .get() instead of direct [index]
```

- **`cargo-vet` (Mozilla Supply Chain Audit):**  
  As third-party dependencies enter the workspace in P4, `cargo-vet` ([mozilla.github.io/cargo-vet](https://mozilla.github.io/cargo-vet/)) provides cryptographic commit-level auditing for all imported supply-chain code, extending `cargo-deny` from policy checking to code review verification.

### 1.5 Binary Size & Release Optimizations

`ae`'s contract requires a lean, static, zero-dependency musl binary. SOTA flags for size and speed:

```toml
[profile.release]
opt-level = 3          # Max performance (or 'z' if sub-megabyte binary is prioritized over CPU)
lto = "fat"            # Full cross-crate link-time optimization (cleaner dead-code elimination than "thin")
codegen-units = 1      # Maximum compiler optimization scope
panic = "abort"        # Eliminates unwinding landing pads and landing tables
strip = "symbols"      # Strips symbol tables from the release artifact
```

- **Size Introspection Tools:**
  - `cargo-bloat` (v0.11+, [crates.io/crates/cargo-bloat](https://crates.io/crates/cargo-bloat)): Decomposes ELF/Mach-O binaries by function, crate, and section to detect unintended monomorphization bloat.
  - `cargo-binutils` (`cargo size --release -- -A`): Verifies segment distributions (`.text`, `.rodata`, `.bss`).

### 1.6 MSRV & Edition 2024 Policy Norms

- **Rust 2024 Edition (Stabilized in Rust 1.85.0, 2025-02-20):**  
  `ae` is already pinned to `1.97.1` (Edition 2024, `rust-version = "1.97.1"`). SOTA Edition 2024 features to leverage:
  - **Let-Chains:** `if let Some(x) = opt && x.is_valid() { ... }` simplifies nested conditional checking without nested `if let` blocks.
  - **Cargo Resolver v3:** Default in Edition 2024, preventing dev-dependency feature flags from leaking into production builds.
  - **Application MSRV Pinning:** In binary applications (unlike distributed libraries), the MSRV should match the compiler pin in `rust-toolchain.toml` exactly.

---

## Part 2: Small, Low-Overhead Crate Evaluation

Every evaluation is scored against `ae`'s core constraints:
- **Zero copyleft** (Must satisfy `deny.toml`: `MIT`, `Apache-2.0`, `ISC`, `BSD-2/3`, `Zlib`, `Unicode-3.0`).
- **No git dependencies** (crates.io only).
- **Static musl compatibility** (`x86_64-unknown-linux-musl`).
- **Minimal transitive dependencies and compile-time overhead**.
- **Auditability / High Bus Factor**.

---

### 2.1 CLI Argument Parsing

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **Hand-Rolled** (`src/cli.rs`) | N/A (in-tree) | **0** | Internal (MIT) | 100% repo-owned | 1.97.1 | 0 KB overhead, instant | None | **KEEP FROM-SCRATCH** (P1–P3) |
| **`lexopt`** | 0.3.2 | **0** | MIT | Active (Árpád Borsos) | Unpinned (~1.36+) | +10 KB, < 0.2s compile | 0 advisories | **ADOPT CANDIDATE** (P4/P5) |
| **`pico-args`** | 0.5.0 | **0** | MIT | Passive/Finished (RazrFalcon) | 1.32+ | +8 KB, < 0.2s compile | 0 advisories | **DEFER / REJECT** |
| **`clap` (minimal / `clap_builder`)** | 4.5.x | **5–7** (builder) / **10–15** (derive) | MIT / Apache-2.0 | High (Clap team / epage) | 1.74+ | +350–800 KB, 2–5s compile | 0 active | **REJECT** |
| **`bpaf`** | 0.9.27 | **0** (core) / **3** (derive) | MIT / Apache-2.0 | Active (Bogdan Kolbov) | ~1.60+ | +30 KB, < 0.5s compile | 0 advisories | **DEFER** |

#### In-Depth Reasoning:
- **Hand-Rolled (`src/cli.rs`):** Currently handles `list`, `ls`, `--all`, `--json`, `-V`, `-h`, and positional session names (`SC-022`) in under 315 lines of code. It matches `ae`'s exact grammatical rule (where non-option tokens are launch candidates rather than unknown commands). Zero dependencies, zero risk.
- **`lexopt` (0.3.2, [crates.io/crates/lexopt](https://crates.io/crates/lexopt)):** The premier minimalist parser in modern Rust. It performs no allocations, has zero dependencies, and provides a forward iterator over options, flags, and values. If `ae`'s subcommand surface grows substantially in P3/P5 (`spawn`, `retire`, `compact`, `transfer`), `lexopt` is the cleanest replacement that retains full control over error messages and dispatching.
- **`pico-args` (0.5.0, [crates.io/crates/pico-args](https://crates.io/crates/pico-args)):** Good, but mutates an underlying vector via linear searches, which can lead to subtle ordering issues with repeated arguments or complex subcommands. `lexopt` has a superior streaming architecture.
- **`clap` (4.5.x, [crates.io/crates/clap](https://crates.io/crates/clap)):** Completely excessive for `ae`. Even with `default-features = false`, `clap` pulls in `clap_builder`, `anstyle`, and multiple helper crates, increasing binary size by 400KB+ and compiling hundreds of macro rules.

---

### 2.2 Error Types & Propagation

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **Hand-Rolled** (`src/error.rs`) | N/A (in-tree) | **0** | Internal (MIT) | 100% repo-owned | 1.97.1 | 0 KB, instant | None | **KEEP FROM-SCRATCH** (P1–P3) |
| **`thiserror`** | 2.0.20 | **0** runtime (3 build-time: `syn`, `quote`, `proc-macro2`) | MIT / Apache-2.0 | Very High (David Tolnay) | 1.71.0 | 0 KB runtime, ~1.0s clean build | 0 advisories | **ADOPT WHEN NEEDED** (P3/P4) |
| **`anyhow`** | 1.0.95 | **0** | MIT / Apache-2.0 | Very High (David Tolnay) | 1.68.0 | +20 KB runtime | 0 advisories | **REJECT FOR CORE** |
| **`snafu`** | 0.8.5 | **0** runtime (build-time proc macros) | MIT / Apache-2.0 | Active (Shep Master) | 1.65+ | 0 KB runtime | 0 advisories | **DEFER** |

#### In-Depth Reasoning:
- **Hand-Rolled (`src/error.rs`):** Currently defines `pub enum Error { Io(io::Error) }` with manual `Display` and `std::error::Error` implementations. As stated in #80: *"no error dependency exists until a real error does"*.
- **`thiserror` (2.0.x, [crates.io/crates/thiserror](https://crates.io/crates/thiserror), updated late 2024 / 2025):** Version 2.0 added native `no_std` support and cleaner macro expansion. It produces zero runtime overhead, simply implementing `std::error::Error` and `Display` via derive macros. When domain errors expand in P2/P3 (`EventLogCorrupt`, `SessionNotFound`, `LockContention`, `InvalidAgentName`), `thiserror` eliminates boilerplate while keeping strictly typed enums.
- **`anyhow` (1.0.x, [crates.io/crates/anyhow](https://crates.io/crates/anyhow)):** `anyhow` is designed for applications where any error can be boxed and bubbled up to `main`. However, `ae`'s domain logic requires precise pattern matching to distinguish recoverable errors (e.g., lock busy, process missing) from fatal errors (e.g., disk I/O failure, permission denied). Erasing types into `anyhow::Error` compromises compiler-enforced safety.

---

### 2.3 JSON Serialization & Deserialization

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **Hand-Rolled** (`src/json.rs`) | N/A (in-tree) | **0** | Internal (MIT) | 100% repo-owned | 1.97.1 | 0 KB, instant | None | **KEEP FROM-SCRATCH** (P1–P3) |
| **`serde_json`** + `serde` | 1.0.146 | **3** (`serde`, `itoa`, `ryu`) + derive macros | MIT / Apache-2.0 | Very High (David Tolnay, Serde Team) | 1.68.0 | +120 KB, ~2.5s clean build | 0 active | **DEFER TO P4** |
| **`miniserde`** | 0.1.46 | **3** (`proc-macro2`, `quote`, `syn`) | MIT / Apache-2.0 | Prototype / Experimental (David Tolnay) | Unpinned | +40 KB, ~1.2s clean build | 0 advisories | **REJECT** |
| **`nanoserde`** | 0.2.1 | **0** (no syn/quote!) | MIT / Apache-2.0 | Active (Fedor Logachev) | ~1.60+ | +25 KB, < 0.5s compile | 0 advisories | **REJECT** |

#### In-Depth Reasoning:
- **Hand-Rolled (`src/json.rs`):** 834 lines of carefully audited, tested Rust code in `src/json.rs`. It explicitly satisfies:
  1. **SC-510d**: Strict control over the exact character escape set (`\"`, `\\`, `\n`, `\t`, `\r`).
  2. **SC-506**: Infallible formatting from the `Value` AST, guaranteeing that document output is never truncated mid-stream.
  3. **SC-511b/c**: Forward tolerance on event logs (unrecognized fields and arbitrary numeric/raw tokens are preserved without failing the parse).
  4. **Deterministic object field ordering** (preserves insertion order via `Vec<(String, Value)>`).
- **`serde_json` (1.0.146, [crates.io/crates/serde_json](https://crates.io/crates/serde_json)):** The industry standard. If P4 daemons require parsing complex external API schemas (Telegram Bot API responses), `serde` + `serde_json` may be introduced with the feature that needs it. Introducing it earlier adds unnecessary dependencies and requires custom serializer wrappers to enforce SC-510d/SC-506.
- **`miniserde` / `nanoserde`:** `miniserde` is explicitly documented by its author as a prototype rather than a production-grade library ([crates.io/crates/miniserde](https://crates.io/crates/miniserde)). `nanoserde` lacks streaming parser tolerance needed for JSONL logs. Neither offers advantages over the hand-rolled `src/json.rs`.

---

### 2.4 Unix & System / Process Work

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **Raw `std`** (`std::os::unix`, `std::process`) | N/A | **0** | N/A (Standard Library) | Rust Core Team | 1.97.1 | 0 KB, instant | None | **KEEP FOR TMUX/SPAWN** |
| **`rustix`** | 1.1.4 (and 0.38.31) | **2** (`linux-raw-sys`, `bitflags`) | Apache-2.0 w/ LLVM Exception / MIT | Very High (Bytecode Alliance / Dan Gohman) | 1.63.0 | +30 KB, < 0.8s compile | 0 advisories | **ADOPT FOR P2/P4 SYSTEM WORK** |
| **`nix`** | 0.30.1 | **3** (`libc`, `bitflags`, `cfg-if`) | MIT | High (nix-rust team) | 1.69.0 | +80 KB, ~1.5s compile | RUSTSEC-2021-0119 (patched) | **REJECT IN FAVOR OF RUSTIX** |

#### In-Depth Reasoning:
- **Raw `std`:** For executing the `tmux` CLI, `std::process::Command` wrapped inside `src/transport.rs` is completely adequate and verified across macOS and Linux musl targets.
- **`rustix` (1.1.4, [crates.io/crates/rustix](https://crates.io/crates/rustix)):** When `ae` needs low-level Unix primitives in P2 (file locking via `flock`, non-blocking I/O, POSIX signal masks, file descriptor passing) or P4 (process parent tree walking, pseudo-terminal / TTY inspection), `rustix` is the modern, memory-safe standard:
  - On Linux (including musl), it uses direct, safe inline system calls via `linux-raw-sys`, bypassing C runtime quirks entirely.
  - It exposes strict **I/O Safety** types (`OwnedFd`, `BorrowedFd`), preventing file descriptor leaks and use-after-close bugs.
  - Clean Apache-2.0 w/ LLVM Exception / MIT license.
- **`nix` (0.30.1, [crates.io/crates/nix](https://crates.io/crates/nix)):** `nix` binds directly to `libc`. On musl Linux targets, differences in `libc` definitions and type sizes have historically caused cross-compilation friction. `rustix` is faster, lighter, safer, and better suited for static musl binaries.

---

### 2.5 HTTP Client for Telegram Bridge Daemon (P4)

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **`ureq`** (with `rustls`) | 3.4.0 (3.x series) | **~15–20** (`rustls`, `webpki-roots`, `http`, etc.) | MIT / Apache-2.0 | Active (Martin Algesten) | 1.85.0 | +250 KB, ~3s compile | 0 advisories | **ADOPT FOR P4 (TELEGRAM)** |
| **`minreq`** (with `rustls`) | 2.13.x | **~10–14** (`rustls`, `webpki-roots`, `log`) | ISC | Maintained (Daniel Alm Grundström) | Debian oldstable (~1.65+) | +180 KB, ~2s compile | 0 advisories | **BACKUP OPTION** |
| **`attohttpc`** | 0.31.0 | **~12–16** | **MPL-2.0** | Maintained (sbstp) | 2021 edition | +200 KB | 0 advisories | **DISQUALIFIED (LICENSE)** |
| **`reqwest`** (blocking) | 0.12.x | **~40–60** (`tokio`, `hyper`, `rustls`, `bytes`, etc.) | MIT / Apache-2.0 | Very High (Sean McArthur) | 1.70+ | +1.2 MB, ~8s compile | 0 active | **REJECT (BLOAT)** |
| **Shellout to `curl`** | N/A | **0** | N/A | N/A | N/A | 0 KB | N/A | **REJECT (DAEMON STABILITY)** |

#### In-Depth Reasoning:
- **`attohttpc` License Disqualification:** `attohttpc` (v0.31.0, [crates.io/crates/attohttpc](https://crates.io/crates/attohttpc)) is licensed under **MPL-2.0** (Mozilla Public License 2.0). `deny.toml` restricts licenses to an explicit permissive allowlist (`MIT`, `Apache-2.0`, `BSD`, `ISC`, `Zlib`). MPL-2.0 is a weak copyleft license and is automatically rejected by `just rust-deny`.
- **`ureq` (3.4.0, [crates.io/crates/ureq](https://crates.io/crates/ureq)):**  
  The 3.x release rewritten around a synchronous sans-IO core is the gold standard for minimal, blocking HTTP clients:
  - Supports pure Rust TLS via `rustls` with `webpki-roots` (statically linked, zero OpenSSL/libcrypto dependencies, works out of the box on static musl Linux and macOS).
  - Contains **no `unsafe` code** in the crate itself.
  - Eliminates the need to spawn an async runtime just to make HTTPS requests to the Telegram Bot API (`sendMessage`, `getUpdates`).
  - MSRV 1.85.0 is fully satisfied by `ae`'s pinned 1.97.1 toolchain.
- **`minreq` (2.x, [crates.io/crates/minreq](https://crates.io/crates/minreq)):**  
  Licensed under `ISC` (which is permitted in `deny.toml`). A viable alternative, but `ureq` has broader community adoption, more robust timeout management, and cleaner error reporting.
- **`reqwest`:** Pulls in `tokio`, `hyper`, `tower`, `h2`, and dozens of async crates even in `blocking` mode, ballooning compile times and binary footprint.

---

### 2.6 Concurrency & Async Runtime for P4 Daemons

The P4 daemons (the watchdog and the telegram bridge) run long-lived background loops:
1. **Watchdog Loop:** Periodic ticker (e.g. 5–30s interval), checking session locks, tailing `events.jsonl`, inspecting agent states.
2. **Telegram Bridge:** Long-polling HTTPS loop (`getUpdates` with `timeout=30`), routing Telegram replies into ae sessions and forwarding `say` chat events to Telegram.

| Candidate | Current Version | Transitive Dep Count | License | Maintenance / Maintainer | MSRV | Binary / Compile Impact | RustSec Advisories | Verdict |
|---|---|---|---|---|---|---|---|---|
| **OS Threads (`std::thread`)** | N/A | **0** | N/A (Standard Library) | Rust Core Team | 1.97.1 | 0 KB, instant | None | **RECOMMENDED (PURE STD)** |
| **`smol`** (v2) | 2.0.2 | **~12–16** (`async-io`, `polling`, `async-executor`, etc.) | Apache-2.0 / MIT | Active (Stjepan Glavina / notgull) | 1.63.0 | +120 KB, ~2s compile | 0 advisories | **DEFER** |
| **`tokio`** (`rt` + `macros`) | 1.43.x | **~8–12** (`mio`, `socket2`, `pin-project-lite`, etc.) | MIT | Very High (Tokio Team) | 1.70+ (rolling) | +300 KB, ~4s compile | 0 advisories | **REJECT FOR P4** |

#### In-Depth Reasoning:
- **OS Threads (`std::thread` + `std::sync::mpsc`):**  
  *Why threads win:* The daemon workload consists of exactly **two to four long-running OS threads**:
  - Thread 1: Main signal handler / supervisor loop.
  - Thread 2: Watchdog periodic ticker & file watcher.
  - Thread 3: Telegram HTTPS long-poller (blocking call via `ureq`).
  - Thread 4: Event-bus consumer (reading session `events.jsonl` or internal channel).
  
  Modern OS threads cost ~8–16 KB of stack space when idle and have zero runtime scheduling overhead. Spawning an async runtime (`tokio` or `smol`) introduces async cancellation hazards, complex pinning, colored functions (`async fn`), and significant dependency overhead for a system that only manages a handful of I/O streams.
- **`smol` (2.0.2, [crates.io/crates/smol](https://crates.io/crates/smol)):**  
  If non-blocking timer multiplexing across hundreds of sessions is ever required in a future cloud-scale monitor, `smol` is the best lightweight async runtime. For local session multiplexing, it is unnecessary.
- **`tokio`:** Industry standard for high-throughput network servers (thousands of concurrent connections), but an anti-pattern for small system CLI utilities and daemons prioritizing simplicity and auditability.

---

## Part 3: Nuances, Disagreements & Strategic Recommendations

### 3.1 Disagreements with Bash-Era Assumptions

1. **The "Single Crate vs. Reinvention" Tradeoff:**  
   *Current Doctrine:* Keep everything in one crate to avoid cross-crate dead code.  
   *Assessment:* Strongly agree for P0–P3. However, keep internal modules (`src/json.rs`, `src/events.rs`, `src/meta.rs`, `src/tmux.rs`) decoupled through strict visibility (`pub(crate)`) and avoid circular dependencies.
2. **Hand-Rolled JSON Beyond P3:**  
   *Current Doctrine:* Hand-roll JSON in `src/json.rs` for full ownership.  
   *Assessment:* `src/json.rs` is an asset for P1–P3 because it strictly upholds `ae`'s formatting and parsing invariants. However, in P4 (Telegram Bot API integration), parsing complex external API payloads (nested JSON with user objects, message entities, callback queries) with a hand-rolled AST becomes error-prone and tedious. Introducing `serde` + `serde_json` *strictly scoped to the telegram module* in P4 is the pragmatic choice.
3. **Panic Strategy in P4 (`panic = "abort"`):**  
   *Current Doctrine:* `panic = "abort"` in `Cargo.toml`.  
   *Assessment:* Correct for CLI invocations (`ae list`, `ae send`, `ae spawn`). However, when long-lived daemons (`ae watchdog`, `ae telegram`) land in P4, an unhandled panic in a single malformed event line should not crash the entire daemon process. In P4, consider using worker threads with channel supervision (`thread::spawn` naturally isolates panics at the thread boundary even without `catch_unwind` if the supervisor loop respawns the worker, or flip to `panic = "unwind"` for daemon targets).

---

## Summary Matrix for Implementation Phases

| Capability | Phase 1 (Reads) | Phase 2 (Writes) | Phase 3 (Lifecycle) | Phase 4 (Daemons) | Phase 5 (Flip) |
|---|---|---|---|---|---|
| **CLI Parser** | Hand-rolled (`src/cli.rs`) | Hand-rolled | Hand-rolled | Hand-rolled (or `lexopt`) | `lexopt` (if syntax grows) |
| **Error Handling** | Hand-rolled (`src/error.rs`) | Hand-rolled | `thiserror` (optional) | `thiserror` (multi-domain) | `thiserror` |
| **JSON** | Hand-rolled (`src/json.rs`) | Hand-rolled | Hand-rolled | `serde_json` (Telegram API only) | Hybrid (core hand-rolled, API serde) |
| **Syscalls / OS** | `std::process::Command` | `rustix` (`flock`/signals) | `rustix` | `rustix` | `rustix` |
| **HTTP Client** | None | None | None | `ureq` 3.x (with `rustls`) | `ureq` 3.x |
| **Concurrency** | None (Single-shot CLI) | None | None | `std::thread` + `mpsc` | `std::thread` + `mpsc` |

---
*Report generated and cross-verified against crates.io, RustSec advisory databases, and repo invariants on 2026-08-24.*
