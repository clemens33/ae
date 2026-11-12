//! Independent receipt and P1 release acceptance.
//!
//! The original P2 strict parser, local matcher, writer and window contracts
//! remain frozen. The local matcher is now relevant only to the human-only
//! `AE_RELEASE_HP=local` override.
//!
//! Superseding release requirements: lead ruling-r0 F2/F3-C/F4/F5, plan r2.
//! Default release verifies an authenticated hp receipt for the clean pre-bump
//! HEAD/tree and runs the guarded Mac subset. Missing proof refuses. The final
//! identity recheck follows deny and immediately precedes the bump. All later
//! non-nextest checks, version-path proof and bounded binary smoke remain.
//!
//! `releaseproof_spec` supplies real-recipe behaviour controls with fake external
//! cargo/SSH dependencies. The producer installs every frozen file unchanged.

mod releaseproof_spec;

use std::path::{Path, PathBuf};

use super::{read, recipe_text, root};
use crate::parity::capture::ExitOutcome;
use crate::parity::capture::raw;
use crate::parity::Invocation;

// ---------------------------------------------------------------------------
// Receipt format oracle.
// ---------------------------------------------------------------------------

/// The required keys of one receipt, in file order. One `key=value` per line,
/// UTF-8, `LF` endings, no blank or comment lines.
const REQUIRED_KEYS: [&str; 14] = [
    "version",
    "tree",
    "head",
    "lock_hash",
    "toolchain_pin",
    "rustc",
    "nextest",
    "tmux",
    "harness",
    "inventory",
    "inventory_count",
    "scratch",
    "host",
    "time",
];

/// The seven harnesses, in `src/tool.rs::ToolKind` order: the adapter row and
/// the binary that answers `--version`.
const HARNESS_NAMES: [&str; 7] =
    ["claude", "codex", "gemini", "agy", "grok", "muse", "opencode"];

/// The maximum age of a reusable receipt, in seconds (24 h).
const MAX_AGE_SECS: i64 = 86_400;

/// Whether `text` is `len` lowercase hex characters.
fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether `text` is whole seconds in base ten, no sign, no leading zero,
/// no padding.
fn is_plain_u64(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 18
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}

/// Days since an epoch day number (civil-from-days, Howard Hinnant).
/// Used only on contract-validated inputs.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = (year - era * 400) as i64;
    let mp = ((month + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Whether `text` is a UTC `YYYY-MM-DDTHH:MM:SSZ` instant with a real
/// calendar date and clock time.
fn is_rfc3339_utc(text: &str) -> bool {
    let Some(stamp) = text.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = stamp.split_once('T') else {
        return false;
    };
    let digits = |s: &str, n: usize| {
        s.len() == n && s.bytes().all(|byte| byte.is_ascii_digit())
    };
    let (Some(y), Some(m), Some(d)) = (date.get(0..4), date.get(5..7), date.get(8..10)) else {
        return false;
    };
    if date.get(4..5) != Some("-") || date.get(7..8) != Some("-") || date.len() != 10 {
        return false;
    }
    let (Some(hh), Some(mm), Some(ss)) = (time.get(0..2), time.get(3..5), time.get(6..8)) else {
        return false;
    };
    if time.get(2..3) != Some(":") || time.get(5..6) != Some(":") || time.len() != 8 {
        return false;
    }
    if !digits(y, 4) || !digits(m, 2) || !digits(d, 2)
        || !digits(hh, 2) || !digits(mm, 2) || !digits(ss, 2)
    {
        return false;
    }
    let (year, month, day, hour, min, sec): (i64, u32, u32, u32, u32, u32) = (
        y.parse().unwrap_or(-1),
        m.parse().unwrap_or(0),
        d.parse().unwrap_or(0),
        hh.parse().unwrap_or(99),
        mm.parse().unwrap_or(99),
        ss.parse().unwrap_or(99),
    );
    if !(1..=12).contains(&month) || hour > 23 || min > 59 || sec > 60 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let dim = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    day >= 1 && day <= dim
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since the epoch (civil-from-days
/// inverse). Used to stamp the stale-receipt fixture.
fn secs_to_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    let (hour, min, sec) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Seconds since the epoch of a validated [`is_rfc3339_utc`] stamp, or `None`.
fn stamp_secs(text: &str) -> Option<i64> {
    if !is_rfc3339_utc(text) {
        return None;
    }
    let stamp = text.strip_suffix('Z')?;
    let (date, time) = stamp.split_once('T')?;
    let year: i64 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    let hour: i64 = time.get(0..2)?.parse().ok()?;
    let min: i64 = time.get(3..5)?.parse().ok()?;
    let sec: i64 = time.get(6..8)?.parse().ok()?;
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + min * 60 + sec)
}

/// One parsed receipt: exactly the required keys, each once, in file order,
/// with grammars validated.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SpecReceipt {
    keys: Vec<&'static str>,
    map: std::collections::BTreeMap<&'static str, std::path::PathBuf>,
}

/// Parse one receipt file's bytes under the strict grammar.
///
/// `Ok` means the grammar holds; `Err` says why it does not. The match
/// decision is separate: [`spec_decision`] compares a grammar-valid receipt
/// against live facts. A malformed receipt is hostile input: every `Err`
/// path funnels into the release's full-gate branch.
fn spec_parse(bytes: &[u8]) -> Result<SpecReceipt, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "malformed: the receipt is not valid UTF-8".to_owned())?;
    if text.contains('\r') {
        return Err("malformed: the receipt carries a carriage return".to_owned());
    }
    if text.is_empty() {
        return Err("malformed: the receipt is empty".to_owned());
    }
    let mut keys = Vec::new();
    let mut map: std::collections::BTreeMap<&'static str, PathBuf> = Default::default();
    for line in text.lines() {
        if line.is_empty() {
            return Err("malformed: the receipt has a blank line".to_owned());
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("malformed: a line is not `key=value`: {line:?}"));
        };
        let Some(&known) = REQUIRED_KEYS.iter().find(|known| **known == key) else {
            return Err(format!("malformed: an unknown key `{key}`"));
        };
        if map.contains_key(known) {
            return Err(format!("malformed: the key `{key}` is repeated"));
        }
        let value_ok = match known {
            "version" => value == "1",
            "tree" | "head" => is_hex(value, 40),
            "lock_hash" | "toolchain_pin" | "inventory" => is_hex(value, 64),
            "inventory_count" => is_plain_u64(value),
            "time" => is_rfc3339_utc(value),
            "rustc" | "nextest" | "tmux" | "scratch" | "host" => {
                !value.is_empty() && value.trim() == value
            }
            "harness" => {
                let parts: Vec<&str> = value.split(';').collect();
                parts.len() == HARNESS_NAMES.len()
                    && parts.iter().zip(HARNESS_NAMES.iter()).all(|(part, name)| {
                        part.strip_prefix(name).and_then(|rest| rest.strip_prefix('='))
                            .is_some_and(|version| !version.is_empty())
                    })
            }
            _ => false,
        };
        if !value_ok {
            return Err(format!("malformed: the value of `{key}` breaks its grammar"));
        }
        keys.push(known);
        map.insert(known, PathBuf::from(value));
    }
    if keys != REQUIRED_KEYS {
        return Err(format!(
            "malformed: the keys are not the required fourteen in order: {keys:?}"
        ));
    }
    Ok(SpecReceipt { keys, map })
}

/// The release's decision over one receipt: reuse, or run the gate in full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpecDecision {
    /// Reuse: identical tree, HEAD, every fact and a fresh timestamp.
    Match,
    /// Run `just test` in full; the payload names the fact or the malformed
    /// receipt's defect for the release log.
    Full(&'static str),
}

/// The live facts the release recomputes before the bump. `now_secs` is the
/// release's own wall clock; `stamp` is the receipt's `time`.
struct LiveFacts {
    tree: String,
    head: String,
    lock_hash: String,
    toolchain_pin: String,
    rustc: String,
    nextest: String,
    tmux: String,
    inventory: String,
    inventory_count: String,
    scratch: String,
    host: String,
    now_secs: i64,
}

/// The contract's match rule: the receipt file for the pre-bump tree exists,
/// parses under the strict grammar, and every deciding recorded fact equals
/// its live value, on the same host, at most 24 h old. Anything else —
/// missing, unreadable, malformed, stale, other host, any deciding fact
/// differs — is `Full`. Only `harness` is recorded without deciding: the
/// gate never executes a real harness binary, so its versions carry no
/// gate evidence, and they churn independently of the tree.
/// `time` decides only through its age, never by equality.
///
/// `SpecReceipt.map` holds the receipt's values; `LiveFacts` holds the live
/// ones. A `None` receipt is a missing or unreadable file.
fn spec_decision(receipt: Option<&SpecReceipt>, live: &LiveFacts) -> SpecDecision {
    let Some(receipt) = receipt else {
        return SpecDecision::Full("missing");
    };
    let same = |key: &str, value: &str| receipt.map[key].to_str() == Some(value);
    for (key, old) in [
        ("tree", live.tree.as_str()),
        ("head", live.head.as_str()),
        ("lock_hash", live.lock_hash.as_str()),
        ("toolchain_pin", live.toolchain_pin.as_str()),
        ("rustc", live.rustc.as_str()),
        ("nextest", live.nextest.as_str()),
        ("tmux", live.tmux.as_str()),
        ("inventory", live.inventory.as_str()),
        ("inventory_count", live.inventory_count.as_str()),
        ("scratch", live.scratch.as_str()),
        ("host", live.host.as_str()),
    ] {
        if !same(key, old) {
            return SpecDecision::Full(key);
        }
    }
    let Some(stamp) = stamp_secs(&receipt.map["time"].to_string_lossy()) else {
        return SpecDecision::Full("malformed");
    };
    let age = live.now_secs - stamp;
    if age < 0 || age > MAX_AGE_SECS {
        return SpecDecision::Full("stale");
    }
    SpecDecision::Match
}

/// The version files a release commit may change: the bump files, the fuzz
/// lock, the two badge files, and the generated changelog. Nothing else.
const VERSION_PATHS: [&str; 6] = [
    "Cargo.toml",
    "Cargo.lock",
    "fuzz/Cargo.lock",
    "README.md",
    "docs/index.md",
    "CHANGELOG.md",
];

/// Whether the bump proof allows `path`: an exact version path, never a
/// directory, a wildcard, or any path outside the tree.
fn is_version_path(path: &str) -> bool {
    VERSION_PATHS.contains(&path)
}

/// A synthetic grammar-valid receipt on fixed facts. Every expected outcome
/// below derives from this fixture and `LiveFacts::spec()`, never from what
/// the implementation happens to emit.
fn spec_good() -> String {
    format!(
        "version=1\ntree={}\nhead={}\nlock_hash={}\ntoolchain_pin={}\n\
         rustc=rustc 1.97.1 (x)\nnextest=cargo-nextest 0.9.143\ntmux=tmux 3.5\n\
         harness=claude=2.1.306;codex=0.159.2;gemini=0.1;agy=1.1.11;grok=4.7;muse=1.0;opencode=absent\n\
         inventory={}\ninventory_count=2\nscratch=apfs\nhost=spec-host\ntime=2026-11-11T09:00:00Z\n",
        "a".repeat(40),
        "a".repeat(40),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )
}

impl LiveFacts {
    fn spec() -> Self {
        Self {
            tree: "a".repeat(40),
            head: "a".repeat(40),
            lock_hash: "b".repeat(64),
            toolchain_pin: "c".repeat(64),
            rustc: "rustc 1.97.1 (x)".to_owned(),
            nextest: "cargo-nextest 0.9.143".to_owned(),
            tmux: "tmux 3.5".to_owned(),
            inventory: "d".repeat(64),
            inventory_count: "2".to_owned(),
            scratch: "apfs".to_owned(),
            host: "spec-host".to_owned(),
            now_secs: stamp_secs("2026-11-11T10:00:00Z").unwrap_or(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Text pins over the justfile.
// ---------------------------------------------------------------------------

/// Whether the justfile writes gate receipts only for a whole green run, in
/// the main checkout's `.local`, keyed by tree hash, atomically.
///
/// The write lives on the `rust-check` path (the body after its
/// dependencies), never inside `rust-test` or `_tmux-isolated`: a filtered
/// `_tmux-isolated test` run, a direct `rust-test`, and the advisory `pre`
/// must never produce a receipt, and the lane must never know the receipt
/// exists. The writer resolves the main checkout like the lane lock
/// (`--git-common-dir`), refuses a dirty tree (`status --porcelain`), and
/// publishes via a same-directory temp and `mv`.
fn receipt_writer_ok(justfile: &str) -> bool {
    let check = recipe_text(justfile, "rust-check:");
    let rust_test = recipe_text(justfile, "rust-test:");
    let lane = recipe_text(justfile, "_tmux-isolated lane *args:");
    let pre = recipe_text(justfile, "pre:");
    let pre_test = recipe_text(justfile, "_pre-test:");
    let joined = |lines: &[String]| lines.join("\n");
    let has = |lines: &[String], needle: &str| lines.iter().any(|line| line.contains(needle));
    let writes = joined(&check).contains("gate-receipts")
        || has(&check, "gate-receipt");
    if !writes {
        return false;
    }
    if !has(&check, ".local/gate-receipts")
        && !(joined(&check).contains("gate-receipts") && has(&check, ".local"))
    {
        return false;
    }
    if !has(&check, "--git-common-dir") {
        return false;
    }
    if !has(&check, "status --porcelain") {
        return false;
    }
    if !has(&check, "mv") && !has(&check, "rename") {
        return false;
    }
    if !has(&check, "gate-receipt") && !has(&check, "gate-receipts") {
        return false;
    }
    // No receipt logic in the lanes a filtered, partial or advisory run uses.
    for lines in [&rust_test, &lane, &pre, &pre_test] {
        if lines.iter().any(|line| line.contains("gate-receipt")) {
            return false;
        }
    }
    true
}

/// The release order and retained non-nextest obligations, from plan r2.
fn receipt_release_ok(justfile: &str) -> bool {
    let release = recipe_text(justfile, "release:");
    let position = |needle: &str| release.iter().position(|line| line.contains(needle));
    let (Some(proof), Some(deny), Some(bump)) =
        (position("just release-proof"), position("just rust-deny"), position("just bump"))
    else { return false; };
    if !(proof < deny && deny < bump) { return false; }
    for step in ["rust-fmt-check", "rust-lint", "rust-fuzz-check", "test --doc"] {
        if !position(step).is_some_and(|at| at > bump) { return false; }
    }
    for path in VERSION_PATHS {
        if !release.iter().any(|line| line.contains(path)) { return false; }
    }
    if !release.iter().any(|line| line.contains("exit 1")) { return false; }
    let probes: Vec<_> = release.iter().filter(|line|
        (line.contains("--version") || line.contains("doctor")) &&
        (line.contains("release/ae") || line.contains("$bin"))).collect();
    probes.iter().any(|line| line.contains("--version")) &&
        probes.iter().any(|line| line.contains("doctor")) &&
        probes.iter().all(|line| line.contains("timeout") || line.contains("gtimeout"))
}

/// The synthetic justfile the writer pin is proved against: both entry
/// points gate through `rust-check`, the writer lives there, and the lanes
/// a filtered run uses carry no receipt logic.
fn receipt_writer_green() -> String {
    [
        "test: rust-check\n",
        "rust-check: rust-fmt-check rust-lint rust-fuzz-check rust-test\n",
        "    #!/usr/bin/env bash\n",
        "    common=\"$(git rev-parse --git-common-dir)\"\n",
        "    root=\"${common%/.git}\"\n",
        "    [ -z \"$(git status --porcelain)\" ] || exit 1\n",
        "    tmp=\"$root/.local/gate-receipts/.tmp.$$\"\n",
        "    printf '%s\\n' \"version=1\" >\"$tmp\"\n",
        "    mv \"$tmp\" \"$root/.local/gate-receipts/$tree\"\n",
        "\n",
        "rust-test:\n",
        "    just _tmux-isolated test\n",
        "\n",
        "pre: rust-fmt-check rust-lint _pre-test\n",
        "\n",
        "_pre-test:\n",
        "    just _tmux-isolated test -E 'kind(lib)'\n",
        "\n",
        "_tmux-isolated lane *args:\n",
        "    #!/usr/bin/env bash\n",
        "    echo test) detached cargo nextest run\n",
    ]
    .join("")
}

// ---------------------------------------------------------------------------
// Fixture checkout and fakes.
// ---------------------------------------------------------------------------

fn spec_put(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|why| panic!("{}: {why}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

fn spec_install(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::remove_file(path);
    spec_put(path, body);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

/// One command in `cwd` with the fixture's own `HOME`: `(code, out, err)`.
fn spec_run(scratch: &Path, cwd: &Path, program: &str, args: &[&str]) -> (i32, String, String) {
    let mut invocation = Invocation::new(program)
        .env("HOME", scratch)
        .env("GIT_CONFIG_GLOBAL", scratch.join("gitconfig"));
    for arg in args {
        invocation = invocation.arg(arg);
    }
    let tag = format!(
        "spec-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let io = scratch.join("io");
    std::fs::create_dir_all(&io).expect("the fixture's io dir");
    let (out, err) = (io.join(format!("{tag}.out")), io.join(format!("{tag}.err")));
    let status = raw::run(&invocation, cwd, &out, &err)
        .unwrap_or_else(|why| panic!("{program} must be runnable: {why}"));
    let code = match status.outcome() {
        ExitOutcome::Code(code) => code,
        ExitOutcome::Signalled => -1,
    };
    (
        code,
        std::fs::read_to_string(out).unwrap_or_default(),
        std::fs::read_to_string(err).unwrap_or_default(),
    )
}

/// A throwaway git checkout holding the real justfile, a minimal ae crate,
/// and a `bin` directory that is the recipe's whole `PATH` first entry.
///
/// The fake `cargo` answers every gate call in constant time and records
/// `fmt`, `clippy`, `check`, `nextest`, `doctest`, `deny` and `build` into
/// `$AE_SPEC_REC`, one per line. `AE_SPEC_NEXT_EXIT` fails the nextest run.
/// Fake harness binaries answer `--version` with a fixed string. `hostname`
/// answers `spec-host` unless `AE_SPEC_HOST` names another.
struct ReceiptFixture {
    base: crate::cli::OwnedScratch,
    tree: PathBuf,
    linked: PathBuf,
}

impl ReceiptFixture {
    fn new(tag: &str, linked: bool) -> Self {
        let base = crate::cli::OwnedScratch::root("gate", &format!("receipt-{tag}"));
        let tree = base.join("tree");
        let linked_dir = base.join("linked");
        let fixture = Self { base, tree, linked: linked_dir };
        fixture.stock();
        fixture.git_init();
        if linked {
            let (code, out, err) = fixture.git(&["worktree", "add", "-q", "../linked", "-b", "linked"]);
            assert_eq!(code, 0, "git worktree add: {out}{err}");
        }
        fixture
    }

    fn bin(&self) -> PathBuf {
        self.base.join("bin")
    }

    fn stock(&self) {
        std::fs::create_dir_all(self.bin()).expect("the fakes' dir");
        // Link the real just/tmux/git: the lane needs a real tmux for its
        // sentry, and the receipt writer needs a real git.
        let path = std::env::var_os("PATH").unwrap_or_default();
        for tool in ["just", "tmux", "git"] {
            let real = std::env::split_paths(&path)
                .map(|dir| dir.join(tool))
                .find(|file| file.is_file())
                .unwrap_or_else(|| panic!("{tool} must be on PATH"));
            std::os::unix::fs::symlink(&real, self.bin().join(tool))
                .unwrap_or_else(|why| panic!("the linked {tool}: {why}"));
        }
        // Fake cargo: instant answers for every gate tool.
        spec_install(
            &self.bin().join("cargo"),
            "#!/bin/sh\n\
             case \"$1\" in\n\
             fmt) echo fmt >>\"$AE_SPEC_REC\"; exit 0 ;;\n\
             clippy) echo clippy >>\"$AE_SPEC_REC\"; exit 0 ;;\n\
             check) echo check >>\"$AE_SPEC_REC\"; exit 0 ;;\n\
             nextest) case \"$2\" in --version) echo 'cargo-nextest 0.9.143'; exit 0 ;;\n\
                 list) printf '%s\\n' 'spec_one: test' 'spec_two: test'; exit 0 ;;\n\
                 run) echo nextest >>\"$AE_SPEC_REC\"; exit \"${AE_SPEC_NEXT_EXIT:-0}\" ;;\n\
                 esac ;;\n\
             test) case \"$2\" in --doc) echo doctest >>\"$AE_SPEC_REC\"; exit 0 ;;\n\
                 esac ;;\n\
             deny) echo deny >>\"$AE_SPEC_REC\"; exit 0 ;;\n\
             metadata) v=$(sed -n 's/^version = \"\\([^\"]*\\)\"/\\1/p' Cargo.toml | head -1); sed \"s/^version = \\\"[0-9][^\\\"]*\\\"/version = \\\"$v\\\"/\" fuzz/Cargo.lock >fuzz/Cargo.lock.new && mv fuzz/Cargo.lock.new fuzz/Cargo.lock; if [ \"$AE_SPEC_TOUCH\" = 1 ]; then echo '// side effect' >>src/lib.rs; fi; echo '{\"packages\":[],\"workspace_members\":[],\"version\":1}'; exit 0 ;;\n\
             build) echo build >>\"$AE_SPEC_REC\"; mkdir -p target/release target/aarch64-apple-darwin/release target/x86_64-unknown-linux-musl/release; v=$(sed -n 's/^version = \"\\([^\"]*\\)\"/\\1/p' Cargo.toml | head -1); printf '%s\\n' '#!/bin/sh' \"echo \\\"ae $v\\\"\" >target/release/ae; chmod +x target/release/ae; cp target/release/ae target/aarch64-apple-darwin/release/ae; cp target/release/ae target/x86_64-unknown-linux-musl/release/ae; exit \"${AE_SPEC_BUILD_EXIT:-0}\" ;;\n\
             esac\nexit 0\n",
        );
        // Fake taplo, shfmt, shellcheck, hostname and harness binaries.
        spec_install(&self.bin().join("taplo"), "#!/bin/sh\nexit 0\n");
        spec_install(&self.bin().join("shfmt"), "#!/bin/sh\nexit 0\n");
        // Real shellcheck prints two lines; `_shellcheck-pin` reads the
        // second one (`awk '/^version:/ {print $2}'`).
        spec_install(
            &self.bin().join("shellcheck"),
            "#!/bin/sh\ncase \"$1\" in --version) printf '%s\\n' 'ShellCheck - shell script analysis tool' 'version: 0.11.0' ;; esac\nexit 0\n",
        );
        spec_install(
            &self.bin().join("hostname"),
            "#!/bin/sh\necho \"${AE_SPEC_HOST:-spec-host}\"\n",
        );
        for name in HARNESS_NAMES {
            spec_install(
                &self.bin().join(name),
                &format!("#!/bin/sh\necho '{name} 9.9.9-spec'\n"),
            );
        }
        // Fake gh: push rights true, release succeeds.
        spec_install(
            &self.bin().join("gh"),
            "#!/bin/sh\ncase \"$1\" in\n\
             auth) exit 0 ;;\n\
             repo) echo 'owner/repo' ;;\n\
             api) echo true ;;\n\
             release) exit 0 ;;\n\
             esac\nexit 0\n",
        );
        // Fake git-cliff: succeed, record argv, write the `-o` output.
        spec_install(
            &self.bin().join("git-cliff"),
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >\"$AE_SPEC_CLIFF\"\nprev=; out=\nfor a in \"$@\"; do if [ \"$prev\" = -o ]; then out=$a; fi; prev=$a; done\n\
             if [ -n \"$out\" ]; then printf '%s\\n' '# changelog fixture' >\"$out\"; else echo 'changelog fixture'; fi\nexit 0\n",
        );
        // Fake rustc, llvm-readobj and file: the static-proof lane needs them.
        spec_install(
            &self.bin().join("rustc"),
            "#!/bin/sh\ncase \"$1\" in --print) echo \"$AE_SPEC_SYSROOT\" ;;\n\
             -vV) printf '%s\\n' 'rustc 1.97.1 (spec)' 'host: aarch64-apple-darwin' ;;\n\
             --version) echo 'rustc 1.97.1 (spec)' ;;\nesac\nexit 0\n",
        );
        let sysroot = self.base.join("sysroot");
        let readobj_dir = sysroot.join("lib/rustlib/aarch64-apple-darwin/bin");
        std::fs::create_dir_all(&readobj_dir).expect("the fake sysroot");
        spec_install(
            &readobj_dir.join("llvm-readobj"),
            "#!/bin/sh\necho 'Program headers:'\nexit 0\n",
        );
        spec_install(
            &self.bin().join("file"),
            "#!/bin/sh\necho 'statically linked'\nexit 0\n",
        );
        // The musl compiler name the real justfile pins.
        let musl = super::justfile_musl_cc(&read(&root().join("justfile")));
        assert!(!musl.is_empty(), "the justfile must pin RUST_MUSL_CC");
        spec_install(&self.bin().join(&musl), "#!/bin/sh\nexit 0\n");
    }

    fn git_init(&self) {
        std::fs::create_dir_all(&self.tree).expect("the tree");
        spec_put(&self.tree.join("Cargo.toml"), "[package]\nname = \"ae\"\nversion = \"0.0.0\"\n");
        spec_put(
            &self.tree.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        );
        spec_put(
            &self.tree.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.97.1\"\n",
        );
        spec_put(
            &self.tree.join(".config/nextest.toml"),
            "[profile.default]\ntest-threads = 4\n",
        );
        spec_put(&self.tree.join("src/lib.rs"), "");
        spec_put(&self.tree.join("fuzz/Cargo.toml"), "[package]\nname = \"ae-fuzz\"\n");
        spec_put(
            &self.tree.join("fuzz/Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"ae\"\nversion = \"0.0.0\"\n",
        );
        for name in ["justfile", "install", "README.md"] {
            std::fs::copy(root().join(name), self.tree.join(name))
                .unwrap_or_else(|why| panic!("the fixture needs {name}: {why}"));
        }
        std::fs::create_dir_all(self.tree.join("docs")).expect("the docs dir");
        std::fs::copy(root().join("docs/index.md"), self.tree.join("docs/index.md"))
            .expect("the fixture needs docs/index.md");
        spec_put(&self.tree.join(".gitignore"), ".local/\n/dist/\n/target/\n");
        for words in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "receipt@example.invalid"],
            vec!["config", "user.name", "Receipt"],
            vec!["add", "-A"],
            vec!["commit", "-qm", "fixture"],
        ] {
            let (code, out, err) = self.git(&words);
            assert_eq!(code, 0, "git {words:?}: {out}{err}");
        }
        // A bare remote so `git fetch`/`pull`/`push` in `release` succeed.
        let remote = self.base.join("remote.git");
        let (code, out, err) = spec_run(
            &self.base,
            &self.base,
            "git",
            &["init", "-q", "--bare", remote.to_str().unwrap_or_default()],
        );
        assert_eq!(code, 0, "git init bare: {out}{err}");
        let (code, out, err) = self.git(&["remote", "add", "origin", remote.to_str().unwrap_or_default()]);
        assert_eq!(code, 0, "git remote add: {out}{err}");
        let (code, out, err) = self.git(&["push", "-q", "origin", "main"]);
        assert_eq!(code, 0, "git push: {out}{err}");
    }

    fn git(&self, words: &[&str]) -> (i32, String, String) {
        let mut full = vec!["-C", self.tree.to_str().unwrap_or_default()];
        full.extend_from_slice(words);
        spec_run(&self.base, &self.tree, "git", &full)
    }

    /// `just <recipe> <args>` in `cwd`, with the fakes first on `PATH`.
    fn just_in(
        &self,
        cwd: &Path,
        recipe: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> (i32, String, String) {
        let mut invocation = Invocation::new("just")
            .env_cleared()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("HOME", &*self.base)
            .env("TMPDIR", &*self.base)
            .env("AE_TEST_TMPDIR", &*self.base)
            .env("GIT_CONFIG_GLOBAL", self.base.join("gitconfig"))
            .env("AE_SPEC_REC", self.base.join("rec"))
            .env("AE_SPEC_CLIFF", self.base.join("cliff-argv"))
            .env("AE_SPEC_SYSROOT", self.base.join("sysroot"))
            .arg("--justfile")
            .arg(self.tree.join("justfile"))
            .arg("--working-directory")
            .arg(cwd)
            .arg(recipe);
        for arg in args {
            invocation = invocation.arg(arg);
        }
        for (key, value) in env {
            invocation = invocation.env(key, value);
        }
        let (out, err) = (self.base.join("gate.out"), self.base.join("gate.err"));
        let status = raw::run(&invocation, cwd, &out, &err)
            .unwrap_or_else(|why| panic!("the gate runs: {why}"));
        let code = match status.outcome() {
            ExitOutcome::Code(code) => code,
            ExitOutcome::Signalled => -1,
        };
        (
            code,
            std::fs::read_to_string(out).unwrap_or_default(),
            std::fs::read_to_string(err).unwrap_or_default(),
        )
    }

    /// `just release` in the checkout. `abort` fails the fake cargo build
    /// inside `bundles`, which lands after the gate decision and the bump.
    fn release(&self, env: &[(&str, &str)]) -> (i32, String, String) {
        let mut invocation = Invocation::new("just")
            .env_cleared()
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.bin().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("HOME", &*self.base)
            .env("TMPDIR", &*self.base)
            .env("AE_TEST_TMPDIR", &*self.base)
            .env("GIT_CONFIG_GLOBAL", self.base.join("gitconfig"))
            .env("AE_SPEC_REC", self.base.join("rec"))
            .env("AE_SPEC_CLIFF", self.base.join("cliff-argv"))
            .env("AE_SPEC_SYSROOT", self.base.join("sysroot"))
            .arg("--justfile")
            .arg(self.tree.join("justfile"))
            .arg("--working-directory")
            .arg(&self.tree)
            .arg("release");
        for (key, value) in env {
            invocation = invocation.env(key, value);
        }
        let (out, err) = (self.base.join("rel.out"), self.base.join("rel.err"));
        let status = raw::run(&invocation, &self.tree, &out, &err)
            .unwrap_or_else(|why| panic!("the release runs: {why}"));
        let code = match status.outcome() {
            ExitOutcome::Code(code) => code,
            ExitOutcome::Signalled => -1,
        };
        (
            code,
            std::fs::read_to_string(out).unwrap_or_default(),
            std::fs::read_to_string(err).unwrap_or_default(),
        )
    }

    /// The main checkout's receipt for `tree`, or `None` when absent.
    fn receipt(&self, tree: &str) -> Option<String> {
        let path = self.tree.join(".local/gate-receipts").join(tree);
        if !path.is_file() || path.is_symlink() {
            return None;
        }
        std::fs::read_to_string(&path).ok()
    }

    fn receipts_dir(&self) -> PathBuf {
        self.tree.join(".local/gate-receipts")
    }

    fn tree_head(&self) -> (String, String) {
        let (code, out, err) = self.git(&["rev-parse", "HEAD"]);
        assert_eq!(code, 0, "HEAD: {out}{err}");
        let head = out.trim().to_owned();
        let (code, out, err) = self.git(&["rev-parse", "HEAD^{tree}"]);
        assert_eq!(code, 0, "tree: {out}{err}");
        (head, out.trim().to_owned())
    }

    /// Every `fmt|clippy|check|nextest|doctest|deny|build` label the fake
    /// cargo recorded, in run order.
    fn recorded(&self) -> Vec<String> {
        std::fs::read_to_string(self.base.join("rec"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn clear_record(&self) {
        let _ = std::fs::remove_file(self.base.join("rec"));
    }
}

/// `just test` in the checkout.
fn spec_gate(fixture: &ReceiptFixture, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    fixture.just_in(&fixture.tree, "test", args, env)
}

/// The sha256 of `path`'s bytes, via the host's `shasum` or `sha256sum`.
fn spec_sha256(path: &Path) -> String {
    let scratch = crate::cli::OwnedScratch::root("gate", "hash");
    for tool in ["shasum", "sha256sum"] {
        let path_found = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(tool))
            .find(|file| file.is_file());
        if let Some(tool_path) = path_found {
            let mut invocation = Invocation::new(tool_path)
                .env("PATH", std::env::var_os("PATH").unwrap_or_default());
            if tool == "shasum" {
                invocation = invocation.arg("-a").arg("256");
            }
            invocation = invocation.arg(path);
            let (out, err) = (scratch.join("hash.out"), scratch.join("hash.err"));
            raw::run(&invocation, &scratch, &out, &err)
                .unwrap_or_else(|why| panic!("{tool} runs: {why}"));
            let text = std::fs::read_to_string(&out).unwrap_or_default();
            let hash = text.split_whitespace().next().unwrap_or_default().to_owned();
            assert!(is_hex(&hash, 64), "{tool} output: {text:?}");
            return hash;
        }
    }
    panic!("shasum or sha256sum must be installed");
}

// ---------------------------------------------------------------------------
// The oracle: match only on identical fresh facts; malformed never skips.
// ---------------------------------------------------------------------------

#[test]
fn the_receipt_oracle_matches_only_on_identical_fresh_facts() {
    let live = LiveFacts::spec();
    let good = spec_parse(spec_good().as_bytes()).expect("the synthetic receipt parses");
    assert_eq!(
        good.keys,
        REQUIRED_KEYS.to_vec(),
        "the keys are the required fourteen in order"
    );
    assert_eq!(
        spec_decision(Some(&good), &live),
        SpecDecision::Match,
        "identical fresh facts reuse"
    );

    // A missing or unreadable receipt runs the gate in full.
    assert_eq!(
        spec_decision(None, &live),
        SpecDecision::Full("missing"),
        "a missing or unreadable receipt never reuses"
    );

    // Every recorded fact, flipped alone, runs the gate in full and names it.
    let head_flip = spec_good().replacen(
        &format!("head={}", "a".repeat(40)),
        &format!("head={}", "f".repeat(40)),
        1,
    );
    assert!(spec_parse(head_flip.as_bytes()).is_ok());
    assert_eq!(
        spec_decision(
            Some(&spec_parse(head_flip.as_bytes()).expect("valid")),
            &live
        ),
        SpecDecision::Full("head"),
        "a different HEAD runs the gate"
    );
    let tree_flip = spec_good().replacen(
        &format!("tree={}", "a".repeat(40)),
        &format!("tree={}", "f".repeat(40)),
        1,
    );
    assert_eq!(
        spec_decision(
            Some(&spec_parse(tree_flip.as_bytes()).expect("valid")),
            &live
        ),
        SpecDecision::Full("tree"),
        "a different tree runs the gate"
    );
    for (from, to, key) in [
        (
            format!("lock_hash={}", "b".repeat(64)),
            format!("lock_hash={}", "f".repeat(64)),
            "lock_hash",
        ),
        (
            format!("toolchain_pin={}", "c".repeat(64)),
            format!("toolchain_pin={}", "f".repeat(64)),
            "toolchain_pin",
        ),
        (
            "rustc=rustc 1.97.1 (x)".to_owned(),
            "rustc=rustc 1.98.0 (x)".to_owned(),
            "rustc",
        ),
        (
            "nextest=cargo-nextest 0.9.143".to_owned(),
            "nextest=cargo-nextest 0.9.144".to_owned(),
            "nextest",
        ),
        ("tmux=tmux 3.5".to_owned(), "tmux=tmux 3.6".to_owned(), "tmux"),
        (
            format!("inventory={}", "d".repeat(64)),
            format!("inventory={}", "f".repeat(64)),
            "inventory",
        ),
        (
            "inventory_count=2".to_owned(),
            "inventory_count=3".to_owned(),
            "inventory_count",
        ),
        ("scratch=apfs".to_owned(), "scratch=tmpfs".to_owned(), "scratch"),
        (
            "host=spec-host".to_owned(),
            "host=other-host".to_owned(),
            "host",
        ),
    ] {
        let text = spec_good().replacen(&from, &to, 1);
        let parsed = spec_parse(text.as_bytes())
            .unwrap_or_else(|why| panic!("{key} flip stays grammar-valid: {why}"));
        assert_eq!(
            spec_decision(Some(&parsed), &live),
            SpecDecision::Full(key),
            "a changed {key} runs the gate"
        );
    }

    // Harness versions are recorded only and never decide: every harness
    // moved to another grammar-valid version still reuses. (The suite
    // drives real tmux but never a real harness binary; tmux above decides,
    // harness does not.)
    let moved = spec_good().replacen(
        "harness=claude=2.1.306;codex=0.159.2;gemini=0.1;agy=1.1.11;grok=4.7;muse=1.0;opencode=absent",
        "harness=claude=2.2.0;codex=0.160.0;gemini=0.2;agy=1.2.0;grok=4.8;muse=1.1;opencode=1.0",
        1,
    );
    let parsed = spec_parse(moved.as_bytes()).expect("harness moves parse");
    assert_eq!(
        spec_decision(Some(&parsed), &live),
        SpecDecision::Match,
        "a changed harness version still reuses"
    );

    // The 24 h bound: exactly 24 h reuses, one second past it does not, and a
    // future stamp does not either.
    let edge = |stamp: &str, now: &str| -> String {
        let text = spec_good().replacen(
            "time=2026-11-11T09:00:00Z",
            &format!("time={stamp}"),
            1,
        );
        let mut live = LiveFacts::spec();
        live.now_secs = stamp_secs(now).unwrap_or(0);
        let parsed = spec_parse(text.as_bytes()).expect("the edge receipt parses");
        match spec_decision(Some(&parsed), &live) {
            SpecDecision::Match => "match".to_owned(),
            SpecDecision::Full(why) => format!("full:{why}"),
        }
    };
    assert_eq!(
        edge("2026-11-11T09:00:00Z", "2026-11-12T09:00:00Z"),
        "match",
        "exactly 24 h old reuses"
    );
    assert_eq!(
        edge("2026-11-11T09:00:00Z", "2026-11-12T09:00:01Z"),
        "full:stale",
        "24 h and one second runs the gate"
    );
    assert_eq!(
        edge("2026-11-11T09:00:01Z", "2026-11-11T09:00:00Z"),
        "full:stale",
        "a future stamp runs the gate"
    );

    // Malformed is hostile input: each defect below parses as no receipt at
    // all, so the release's full-gate branch is the only successor. Keep the
    // family exhaustive: empty, non-UTF8, CR, blank line, non key=value,
    // unknown key, repeated key, wrong order, missing key, and every
    // grammar broken above (bad tree hex, short hash, bad count, bad time).
    let non_utf8: Vec<u8> = (0..8_u8).map(|b| b.wrapping_add(200)).collect();
    for (name, bytes) in [
        ("empty", Vec::new()),
        ("non-utf8", non_utf8),
        ("carriage return", spec_good().replace('\n', "\r\n").into_bytes()),
        ("blank line", spec_good().replacen("host=", "\nhost=", 1).into_bytes()),
        (
            "not key=value",
            spec_good().replace("host=", "host\n").into_bytes(),
        ),
        (
            "unknown key",
            spec_good().replace("version=1", "version=1\nextra=1").into_bytes(),
        ),
        (
            "repeated key",
            spec_good().replace("host=", "host=spec-host\nhost=").into_bytes(),
        ),
        (
            "wrong order",
            spec_good()
                .replace(&format!("version=1\ntree={}\nhead={}\n", "a".repeat(40), "a".repeat(40)), &format!("version=1\nhead={}\ntree={}\n", "a".repeat(40), "a".repeat(40)))
                .into_bytes(),
        ),
        (
            "missing key",
            spec_good().replace("host=spec-host\n", "").into_bytes(),
        ),
        ("bad tree hex", {
            let bad = spec_good().replacen(
                &format!("tree={}", "a".repeat(40)),
                "tree=zz",
                1,
            );
            bad.into_bytes()
        }),
        ("short hash", {
            let bad = spec_good().replacen(
                &format!("lock_hash={}", "b".repeat(64)),
                "lock_hash=bb",
                1,
            );
            bad.into_bytes()
        }),
        (
            "bad count",
            spec_good().replace("inventory_count=2", "inventory_count=many").into_bytes(),
        ),
        (
            "bad time",
            spec_good().replace("time=2026-11-11T09:00:00Z", "time=tomorrow").into_bytes(),
        ),
        (
            "bad harness grammar",
            spec_good().replace("opencode=absent", "opencode=").into_bytes(),
        ),
        (
            "harness out of order",
            spec_good()
                .replace("gemini=0.1;agy=1.1.11", "agy=1.1.11;gemini=0.1")
                .into_bytes(),
        ),
    ] {
        assert!(
            spec_parse(&bytes).is_err(),
            "the {name} receipt must be malformed"
        );
    }

    // The version allowlist: exact paths only, never a wildcard or neighbor.
    for path in VERSION_PATHS {
        assert!(is_version_path(path), "{path} is a version path");
    }
    for path in [
        "src/lib.rs",
        "Cargo.toml.bak",
        "fuzz/Cargo.toml",
        "README.md~",
        "../Cargo.toml",
        "CHANGELOG.md/extra",
        "",
    ] {
        assert!(!is_version_path(path), "{path:?} is not a version path");
    }
}

// ---------------------------------------------------------------------------
// The writer: a whole green clean run, and nothing else.
// ---------------------------------------------------------------------------

/// The canonical inventory bytes: the fake nextest list, sorted, each line
/// LF-terminated. The contract fixes this canonicalisation; the hash check
/// below pins it.
fn canonical_inventory() -> String {
    "spec_one: test\nspec_two: test\n".to_owned()
}

/// `just test` in a fixture: exits 0, or the test panics with the log.
fn green_gate(fixture: &ReceiptFixture, env: &[(&str, &str)]) {
    let (code, out, err) = spec_gate(fixture, &[], env);
    assert_eq!(code, 0, "a whole green run: {out}{err}");
}

#[test]
fn the_writer_writes_one_valid_receipt_keyed_by_the_tree_hash() {
    let fixture = ReceiptFixture::new("write", false);
    green_gate(&fixture, &[]);
    let (head, tree) = fixture.tree_head();
    let text = fixture
        .receipt(&tree)
        .unwrap_or_else(|| panic!("the receipt for {tree} after a whole green run"));
    // Keyed by tree hash in the main checkout's .local, never in the tree.
    let receipt_path = fixture.receipts_dir().join(&tree);
    assert!(receipt_path.is_file() && !receipt_path.is_symlink());
    assert!(receipt_path.starts_with(fixture.tree.join(".local")));
    let left: Vec<String> = std::fs::read_dir(fixture.receipts_dir())
        .expect("the receipts dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, [tree.clone()], "one atomic receipt, no temp left");
    // The strict grammar holds, and every fact recomputes.
    let parsed = spec_parse(text.as_bytes()).expect("the written receipt parses");
    let get = |key: &str| parsed.map[key].to_string_lossy().into_owned();
    assert_eq!(get("tree"), tree);
    assert_eq!(get("head"), head);
    assert_eq!(get("lock_hash"), spec_sha256(&fixture.tree.join("Cargo.lock")));
    assert_eq!(
        get("toolchain_pin"),
        spec_sha256(&fixture.tree.join("rust-toolchain.toml"))
    );
    assert!(get("rustc").starts_with("rustc "), "{:?}", get("rustc"));
    assert_eq!(get("nextest"), "cargo-nextest 0.9.143");
    assert!(get("tmux").starts_with("tmux "), "{:?}", get("tmux"));
    assert_eq!(get("host"), "spec-host");
    assert!(
        get("scratch").split_whitespace().count() == 1,
        "scratch is one device token: {:?}",
        get("scratch")
    );
    assert_eq!(get("inventory_count"), "2");
    let want = {
        let inv = fixture.base.join("inventory-canonical");
        spec_put(&inv, &canonical_inventory());
        spec_sha256(&inv)
    };
    assert_eq!(get("inventory"), want, "sorted LF inventory hash");
    assert!(
        get("harness").split(';').count() == HARNESS_NAMES.len(),
        "{:?}",
        get("harness")
    );
    // Fresh: the stamp sits within five minutes behind this test's clock.
    let stamp = stamp_secs(&get("time")).expect("a real UTC stamp");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    assert!((0..=300).contains(&(now - stamp)), "stamp {stamp}, now {now}");
    // The writer also prints which receipt it wrote, for the human.
    let (_, out, err) = spec_gate(&fixture, &[], &[]);
    assert!(
        out.contains("gate-receipt") || err.contains("gate-receipt"),
        "the gate names the receipt it wrote: {out}{err}"
    );
}

#[test]
fn a_filtered_partial_or_advisory_run_writes_no_receipt() {
    let fixture = ReceiptFixture::new("filtered", false);
    let (_, tree) = fixture.tree_head();
    for (name, recipe, args) in [
        ("rust-test", "rust-test", &[][..]),
        ("pre", "pre", &[][..]),
        ("lane-bare", "_tmux-isolated", &["test"][..]),
        ("lane-filtered", "_tmux-isolated", &["test", "-E", "kind(lib)"][..]),
    ] {
        let (code, out, err) = fixture.just_in(&fixture.tree, recipe, args, &[]);
        assert_eq!(code, 0, "{name} still runs green: {out}{err}");
        assert!(
            fixture.receipt(&tree).is_none(),
            "{name} wrote a receipt for {tree}"
        );
        let left: Vec<String> = std::fs::read_dir(fixture.receipts_dir())
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(left.is_empty(), "{name} left a receipt: {left:?}");
    }
}

#[test]
fn a_failed_run_writes_no_receipt_and_leaves_the_prior_receipt() {
    let fixture = ReceiptFixture::new("failed", false);
    green_gate(&fixture, &[]);
    let (_, tree) = fixture.tree_head();
    let prior = fixture
        .receipt(&tree)
        .unwrap_or_else(|| panic!("the prior receipt for {tree}"));
    let (code, out, err) = spec_gate(&fixture, &[], &[("AE_SPEC_NEXT_EXIT", "1")]);
    assert_ne!(code, 0, "a failing nextest fails the gate: {out}{err}");
    assert_eq!(
        fixture.receipt(&tree).as_deref(),
        Some(prior.as_str()),
        "a failed run neither overwrites nor deletes the prior receipt"
    );
    // A failed first run (no prior) writes nothing.
    let fresh = ReceiptFixture::new("failed-first", false);
    let (code, out, err) = spec_gate(&fresh, &[], &[("AE_SPEC_NEXT_EXIT", "1")]);
    assert_ne!(code, 0, "a failing nextest fails the gate: {out}{err}");
    let (_, fresh_tree) = fresh.tree_head();
    assert!(
        fresh.receipt(&fresh_tree).is_none(),
        "a failing first run wrote a receipt"
    );
}

#[test]
fn a_dirty_tree_writes_no_receipt() {
    let fixture = ReceiptFixture::new("dirty", false);
    let (_, tree) = fixture.tree_head();
    // Unstaged tracked edit.
    std::fs::write(fixture.tree.join("src/lib.rs"), "fn spec_dirty() {}\n").expect("the edit");
    let (code, out, err) = spec_gate(&fixture, &[], &[]);
    assert_eq!(code, 0, "a dirty gate still runs: {out}{err}");
    assert!(
        fixture.receipt(&tree).is_none(),
        "an unstaged tracked edit wrote a receipt"
    );
    let (code, _, err) = fixture.git(&["checkout", "--", "src/lib.rs"]);
    assert_eq!(code, 0, "restore: {err}");
    // Staged tracked edit.
    std::fs::write(fixture.tree.join("src/lib.rs"), "fn spec_staged() {}\n").expect("the edit");
    let (code, _, err) = fixture.git(&["add", "src/lib.rs"]);
    assert_eq!(code, 0, "stage: {err}");
    let (code, out, err) = spec_gate(&fixture, &[], &[]);
    assert_eq!(code, 0, "a dirty gate still runs: {out}{err}");
    assert!(
        fixture.receipt(&tree).is_none(),
        "a staged tracked edit wrote a receipt"
    );
    let (code, _, err) = fixture.git(&["reset", "--hard", "-q", "HEAD"]);
    assert_eq!(code, 0, "restore: {err}");
    // Untracked file.
    spec_put(&fixture.tree.join("loose.txt"), "untracked\n");
    let (code, out, err) = spec_gate(&fixture, &[], &[]);
    assert_eq!(code, 0, "a dirty gate still runs: {out}{err}");
    assert!(
        fixture.receipt(&tree).is_none(),
        "an untracked file wrote a receipt"
    );
    let _ = std::fs::remove_file(fixture.tree.join("loose.txt"));
    // And a clean run afterwards still writes.
    green_gate(&fixture, &[]);
    assert!(
        fixture.receipt(&tree).is_some(),
        "a clean run after restore writes again"
    );
}

#[test]
fn a_worktree_run_writes_the_receipt_to_the_main_checkout() {
    let fixture = ReceiptFixture::new("worktree", true);
    let (code, out, err) = fixture.just_in(&fixture.linked, "test", &[], &[]);
    assert_eq!(code, 0, "the worktree gate runs green: {out}{err}");
    let (_, tree) = fixture.tree_head();
    assert!(
        fixture.receipt(&tree).is_some(),
        "the main checkout holds the worktree run's receipt"
    );
    assert!(
        !fixture.linked.join(".local/gate-receipts").exists(),
        "the worktree's own .local holds no receipt"
    );
    let left: Vec<String> = std::fs::read_dir(fixture.receipts_dir())
        .expect("the receipts dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, [tree], "one atomic receipt, no temp left");
}

#[test]
fn a_receipt_write_failure_leaves_a_green_gate_green() {
    let fixture = ReceiptFixture::new("writefail", false);
    // The receipts path stands as a file, so the writer cannot create its dir.
    std::fs::create_dir_all(fixture.tree.join(".local")).expect("the .local dir");
    spec_put(&fixture.tree.join(".local/gate-receipts"), "not a directory\n");
    let (code, out, err) = spec_gate(&fixture, &[], &[]);
    assert_eq!(
        code, 0,
        "a green gate stays green when the receipt write fails: {out}{err}"
    );
    assert!(
        err.contains("receipt"),
        "the failure is named on stderr: {err}"
    );
}

// ---------------------------------------------------------------------------
// The P1 release supersedes only the old local-receipt release oracles.
// ---------------------------------------------------------------------------

#[test]
fn a_release_uses_hp_proof_and_the_mac_subset_before_deny() {
    releaseproof_spec::normal_release_uses_hp_then_mac();
}

#[test]
fn a_release_without_hp_proof_refuses_before_bump() {
    releaseproof_spec::missing_hp_is_a_refusal();
}

#[test]
fn the_bump_proof_refuses_a_non_version_path() {
    releaseproof_spec::non_version_bump_is_rejected();
}

// ---------------------------------------------------------------------------
// Text pins and docs.
// ---------------------------------------------------------------------------

#[test]
fn the_justfile_writes_receipts_only_for_a_whole_green_run() {
    assert!(
        receipt_writer_ok(&receipt_writer_green()),
        "the synthetic writer satisfies every rule"
    );
    // Each planted mutation breaks exactly one rule.
    let green = receipt_writer_green();
    let writer_in_rust_test = green.replace(
        "rust-test:\n    just _tmux-isolated test\n",
        "rust-test:\n    just _tmux-isolated test\n    echo gate-receipt-here\n",
    );
    assert!(
        !receipt_writer_ok(&writer_in_rust_test),
        "the writer inside rust-test writes for a partial run"
    );
    let writer_in_lane = green.replace(
        "    echo test) detached cargo nextest run\n",
        "    echo gate-receipt\n    echo test) detached cargo nextest run\n",
    );
    assert!(
        !receipt_writer_ok(&writer_in_lane),
        "the writer inside the filtered lane writes for a partial run"
    );
    assert!(
        !receipt_writer_ok(&green.replace("    mv \"$tmp\"", "    cp \"$tmp\"")),
        "a copy is not an atomic publish"
    );
    assert!(
        !receipt_writer_ok(&green.replace("git rev-parse --git-common-dir", "echo $PWD")),
        "the main checkout is not resolved like the lane lock"
    );
    assert!(
        !receipt_writer_ok(&green.replace("git status --porcelain", "true")),
        "a dirty-tree refusal removed writes for a dirty tree"
    );
    // And the real justfile must carry the writer after P2.
    assert!(
        receipt_writer_ok(&read(&root().join("justfile"))),
        "the real justfile must write receipts only for a whole green run, in the main \
         .local, keyed by the tree hash, atomically"
    );
}

#[test]
fn the_justfile_proves_hp_and_mac_before_the_bump_and_keeps_other_checks() {
    let green = concat!(
        "release:\n",
        "    just release-proof expect=$head:$tree\n",
        "    just rust-deny\n",
        "    VERSION=$(just bump)\n",
        "    just rust-fmt-check\n",
        "    just rust-lint\n",
        "    just rust-fuzz-check\n",
        "    cargo test --doc --locked --all-features\n",
        "    binary=target/release/ae\n",
        "    timeout 10 \"$binary\" --version\n",
        "    timeout 10 \"$binary\" doctor\n",
        "    diff --name-only Cargo.toml Cargo.lock fuzz/Cargo.lock README.md docs/index.md CHANGELOG.md || exit 1\n",
    );
    assert!(receipt_release_ok(green), "synthetic contract control");
    for (name, mutated) in [
        ("missing proof", green.replace("    just release-proof expect=$head:$tree\n", "")),
        ("proof after bump", green.replace("    just release-proof expect=$head:$tree\n", "")
            .replace("    VERSION=$(just bump)\n", "    VERSION=$(just bump)\n    just release-proof\n")),
        ("deny after bump", green.replace("    just rust-deny\n", "")
            .replace("    VERSION=$(just bump)\n", "    VERSION=$(just bump)\n    just rust-deny\n")),
        ("missing doctests", green.replace("    cargo test --doc --locked --all-features\n", "")),
        ("missing fuzz compile", green.replace("    just rust-fuzz-check\n", "")),
        ("unbounded smoke", green.replace("timeout 10 \"$binary\" --version", "\"$binary\" --version")),
    ] {
        assert!(!receipt_release_ok(&mutated), "must reject {name}");
    }
    assert!(receipt_release_ok(&read(&root().join("justfile"))),
        "release proves hp and Mac before deny and bump, then keeps all non-nextest checks and bounded smoke");
}

#[test]
fn the_contract_names_the_receipt_behaviour() {
    let agents = read(&root().join("AGENTS.md"));
    let development = read(&root().join("docs/development.md"));
    for (label, text) in [("AGENTS.md", &agents), ("docs/development.md", &development)] {
        for needle in ["test-macos", "release-proof", "hp"] {
            assert!(text.contains(needle), "{label} must name {needle}");
        }
    }
    let prose = development.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    for phrase in [
        "ae_release_hp=local",
        "ae_release_hp_root",
        "human-only override",
        "agents never set `ae_release_hp=local`",
        "a later red run does not invalidate a fresh green receipt for the same tree",
    ] {
        assert!(prose.contains(phrase), "development contract must state {phrase}");
    }
}
