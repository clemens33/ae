//! The tmux floor's BOUNDARY: which commands it may refuse.
//!
//! The floor is a gate, and a gate on the wrong command is worse than no gate:
//! a machine whose tmux is too old still has to be able to see what is running
//! (`ae list`), find out which ae it has (`ae version`) and fix itself
//! (`ae upgrade`). Those three, and every helper, must never reach the check.
//!
//! The refusal itself is unit-tested in `src/tmux_floor.rs`; what is asked here
//! is WHERE it is spoken, asked of the tree so a new call site is a review
//! rather than a diff. Conditionals ae hands tmux stay binary for the same
//! reason: 3.4 reads only the first two operands of one.

#![allow(
    clippy::disallowed_methods,
    reason = "this reads the crate's own source; the capability boundary is about PRODUCT code"
)]

use std::fs;
use std::path::{Path, PathBuf};

use std::os::unix::fs::PermissionsExt as _;

use ae::tmux_floor::{self, Probe};

use super::parity::Invocation;
use super::parity::capture::raw;

/// Every `.rs` file under `src/`.
fn product_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found.sort();
    assert!(
        found.len() > 5,
        "the source walk found {} files; a guard that scans nothing passes forever",
        found.len()
    );
    found
}

/// Every product file that decides whether the floor is cleared, with its count.
fn gate_sites() -> Vec<(String, usize)> {
    let mut sites = Vec::new();
    for path in product_sources() {
        // The owner module states the verdict; it does not gate on it.
        if path.ends_with("tmux_floor.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        // The DECISION, not the module: `tmux_floor::clears` is also the
        // parser's own helper and is called from its doctests.
        let count =
            text.matches(".clears_floor()").count() + text.matches("floor_refusal(").count();
        if count > 0 {
            let name = path
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(&path)
                .display()
                .to_string();
            sites.push((name, count));
        }
    }
    sites
}

/// Exactly two commands REFUSE on the floor: the picker tmux draws, and the
/// launch that would create a session ae then has to draw into.
///
/// The reporting surfaces — `ae version`, `ae doctor`, the publish warning —
/// state the same verdict through [`ae::tmux_floor::Probe::verdict`] and
/// refuse nothing, which is why they are not counted here.
#[test]
fn the_floor_is_asked_at_exactly_the_two_sites_that_create_or_draw() {
    assert_eq!(
        gate_sites(),
        vec![
            // `ae orchestrator --popup`'s own `clears_floor`, and nothing else:
            // the public launch path decides NOTHING about the floor, so a
            // launch has exactly one gate and every write sits below it.
            ("src/lib.rs".to_owned(), 1),
            // `floor_refusal`'s definition, the `clears_floor` inside it, and
            // `_launch`'s call to it ahead of the first-run config seed and the
            // migration chain — which the `compact` relaunch goes through too.
            ("src/session_launch.rs".to_owned(), 3),
        ],
        "a new floor gate is a ruling, not a diff — see AGENTS.md"
    );
}

/// The commands a stranded operator needs are the ones that must not be gated.
///
/// Named by their routing constants rather than by prose: `ae list` reports the
/// fleet, `ae version` says which core is installed, and `ae upgrade`
/// diagnoses and repairs an install, which is how a machine below the floor
/// gets a core that no longer needs it.
#[test]
fn the_recovery_commands_never_reach_the_gate() {
    let gated: Vec<String> = gate_sites().into_iter().map(|(file, _)| file).collect();
    for module in ["src/listing.rs", "src/upgrade.rs", "src/install.rs"] {
        assert!(
            !gated.contains(&module.to_owned()),
            "{module} decides the floor; ae list / version / upgrade must work below it"
        );
    }
}

/// Every tmux conjunction in src/ takes exactly two operands: tmux 3.4 reads
/// only the first two and silently drops a third (#186). The opener names its
/// unit — doubled `{{` counts braces at depth two, single `{` at depth one.
#[test]
fn every_tmux_conjunction_in_src_is_binary() {
    let operands = |tail: &str, unit: u32| {
        let (mut depth, mut count) = (unit, 1);
        for byte in tail.bytes().skip(4) {
            match byte {
                b'{' => depth += 1,
                b'}' if depth == 1 => break,
                b'}' => depth -= 1,
                b',' if depth == unit => count += 1,
                _ => {}
            }
        }
        count
    };
    for (tail, unit, expected) in [
        ("{||:#{==:x,1},#{==:y,2},#{==:z,3}}", 1, 3),
        ("{&&:##{==:##{w},2},##{==:##{z},0}}", 1, 2),
        ("{&&:{x},#{{==:#{{{y}}},z}},w}}", 2, 3),
        ("{&&:#{{==:#{{pid}},{x}}},#{{==:#{{{x}}},}}}}", 2, 2),
        ("{&&:#{?a,b,c},d}", 1, 2),
    ] {
        assert_eq!(operands(tail, unit), expected);
    }
    let (mut hits, mut offenders) = (0, Vec::new());
    for path in product_sources() {
        let text =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        for (at, _) in text.match_indices("{&&:").chain(text.match_indices("{||:")) {
            hits += 1;
            let unit = 1 + u32::from(at > 0 && text.as_bytes()[at - 1] == b'{');
            if operands(&text[at..], unit) != 2 {
                let line = text[..at].matches('\n').count() + 1;
                let name = path
                    .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                offenders.push(format!("{name}:{line}"));
            }
        }
    }
    assert!(hits > 0, "the scan matched no conditional");
    assert!(offenders.is_empty(), "non-binary: {offenders:?}");
}

/// One string literal: its source body, whether raw, and the line it opens on.
struct Lit<'a>(&'a str, bool, usize);

/// The content start and hash count of a raw string opening at `i`, if any:
/// `r"…"` or `r#"…"#`; `b"…"` is not raw and its quote opens a literal.
fn raw(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let r = match b[i] {
        b'r' => i,
        b'b' if b.get(i + 1) == Some(&b'r') => i + 1,
        _ => return None,
    };
    if i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') {
        return None;
    }
    let hashes = (r + 1..b.len())
        .take_while(|&j| b.get(j) == Some(&b'#'))
        .count();
    (b.get(r + 1 + hashes) == Some(&b'"')).then_some((r + 2 + hashes, hashes))
}

/// The bytes a char literal at `i` spans: an escape, or one UTF-8 char between
/// its apostrophes. An apostrophe that is a lifetime or a label consumes
/// itself only, because a fixed three-byte skip lands on the closing quote of
/// any longer char, re-opens there and eats the bytes after it — a `//` comment
/// can lose its opener and the quote pairing inverts.
fn char_len(b: &[u8], i: usize) -> usize {
    match b.get(i + 1) {
        Some(b'\\') => match b.get(i + 2) {
            Some(b'x') => 6,
            Some(b'u') if b.get(i + 3) == Some(&b'{') => b[i + 4..]
                .iter()
                .position(|&c| c == b'}')
                .map_or(2, |off| off + 6),
            _ => 4,
        },
        Some(&c) if c < 0x80 => {
            if b.get(i + 2) == Some(&b'\'') {
                3
            } else {
                1
            }
        }
        Some(&c) => {
            let width = match c {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            if b.get(i + 1 + width) == Some(&b'\'') {
                1 + width + 1
            } else {
                1
            }
        }
        None => 1,
    }
}

/// Every string literal in `text`: comments and char literals are skipped, a
/// raw body is consumed whole, and `\` escapes the byte after it, so a quote
/// in prose, a `'"'` char or one inside `r#"…"#` opens or ends no literal.
fn lits(text: &str) -> Vec<Lit<'_>> {
    let b = text.as_bytes();
    let line = |at: usize| text[..at].matches('\n').count() + 1;
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            i += text[i..].find('\n').map_or(b.len() - i, |at| at + 1);
        } else if b[i..].starts_with(b"/*") {
            let mut depth = 1;
            i += 2;
            while depth > 0 && i < b.len() {
                let (open, close) = (b[i..].starts_with(b"/*"), b[i..].starts_with(b"*/"));
                depth += i32::from(open) - i32::from(close);
                i += if open || close { 2 } else { 1 };
            }
        } else if b[i] == b'\'' {
            i += char_len(b, i);
        } else if let Some((start, hashes)) = raw(b, i) {
            let end = (start..b.len())
                .find(|&j| b[j] == b'"' && (0..hashes).all(|h| b.get(j + 1 + h) == Some(&b'#')))
                .unwrap_or(b.len());
            out.push(Lit(&text[start..end], true, line(i)));
            i = (end + 1 + hashes).min(b.len());
        } else if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            out.push(Lit(&text[i + 1..j], false, line(i)));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the literal would PRODUCE a control byte: a raw `0x00`-`0x1f` or
/// `0x7f`, or a (non-raw) escape that decodes to one. `\`+newline is a
/// continuation and produces nothing.
fn control(body: &str, raw: bool) -> bool {
    let bad = |v: u32| v < 0x20 || v == 0x7f;
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] < 0x20 || b[i] == 0x7f {
            return true;
        }
        if b[i] != b'\\' || raw {
            i += 1;
            continue;
        }
        i += match b.get(i + 1) {
            Some(b't' | b'n' | b'r' | b'0') => return true,
            Some(b'x') => {
                let hex = body
                    .get(i + 2..i + 4)
                    .and_then(|s| u32::from_str_radix(s, 16).ok());
                if hex.is_some_and(bad) {
                    return true;
                }
                4
            }
            Some(b'u') if b.get(i + 2) == Some(&b'{') => {
                let end = body[i + 3..].find('}').map(|off| i + 3 + off);
                let hex = end.and_then(|e| u32::from_str_radix(&body[i + 3..e], 16).ok());
                if hex.is_some_and(bad) {
                    return true;
                }
                end.map_or(2, |e| e + 1 - i)
            }
            _ => 2,
        };
    }
    false
}

/// The product prefix of a source text: what lies before its terminal unit-test
/// module.
///
/// The cut is PROVEN, because product code after the module would otherwise
/// leave the scan silently: the `mod tests {` line must sit under a
/// `#[cfg(test)]` in the non-blank block above it, at most one may exist, and
/// after it every column-0 line must be the single closing `}`. Literal bodies
/// are blanked first, so a fixture string at column 0 — tmux OUTPUT being
/// decoded — cannot read as code; anything else refuses with `file:line`.
fn product_head<'a>(text: &'a str, name: &str) -> Result<&'a str, String> {
    let Some(cut) = text.find("\nmod tests {") else {
        return Ok(text);
    };
    let line = text[..cut].matches('\n').count() + 1;
    let tail = &text[cut + 1..];
    let mut blanked = tail.as_bytes().to_vec();
    for lit in lits(tail) {
        let at = lit.0.as_ptr() as usize - tail.as_ptr() as usize;
        blanked[at..at + lit.0.len()].fill(b' ');
    }
    let blanked = String::from_utf8(blanked).unwrap_or_default();
    let col0: Vec<&str> = blanked
        .split('\n')
        .filter(|l| !l.is_empty() && !matches!(l.as_bytes().first(), Some(b' ' | b'\t')))
        .collect();
    let cfg = text[..cut]
        .lines()
        .rev()
        .take_while(|l| !l.trim().is_empty())
        .any(|l| l.trim() == "#[cfg(test)]");
    if col0 != ["mod tests {", "}"] || !cfg || tail.contains("\nmod tests {") {
        return Err(format!(
            "{name}:{line}: untrusted `mod tests` layout: {col0:?}"
        ));
    }
    Ok(&text[..cut])
}

/// Every tmux format literal in product code carries printable bytes only
/// (#195): a control byte reaches a tmux client as `_`, and the reading side
/// cannot parse what it cannot see (#187). The unit pin in `tmux.rs` names
/// that module's constants; this one reads every string literal in src/ that
/// carries a format (`#{`), wherever it lives.
///
/// Named residuals: a test-only format inside `mod tests` is not scanned (the
/// module is cut away, `product_head` proves the cut), and a format assembled
/// from pieces, where the control byte sits in a piece without `#{`
/// (`concat!`, `format!`), escapes the scan.
#[test]
fn no_tmux_format_literal_in_src_carries_a_control_character() {
    let cases = [
        r##""#{a}\t#{b}""##,
        r##""#{a}\\t""##,
        "r\"#{a}\t#{b}\"",
        "r#\"#{a}\t#{b}\"#",
        "'\\n' // \"x\"\n\"#{a}\\t\"",
        "'é' // \"x\"\n\"#{a}\\t\"",
        "'\\u{41}' // \"x\"\n\"#{a}\\t\"",
        "&'a str = \"#{a}\\t\";",
    ];
    let verdicts: Vec<(usize, bool)> = cases
        .iter()
        .map(|s| {
            let found = lits(s);
            (
                found.len(),
                found.first().is_some_and(|l| control(l.0, l.1)),
            )
        })
        .collect();
    assert_eq!(
        verdicts,
        [
            (1, true),
            (1, false),
            (1, true),
            (1, true),
            (1, true),
            (1, true),
            (1, true),
            (1, true)
        ],
        "scanner self-check"
    );
    let module = "const A: u8 = 1;\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n";
    assert_eq!(
        product_head(module, "self").ok(),
        Some("const A: u8 = 1;\n#[cfg(test)]")
    );
    let after = "const A: u8 = 1;\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\nconst B: u8 = 2;\n";
    assert!(
        product_head(after, "self").is_err(),
        "code after the module"
    );

    let (mut hits, mut offenders) = (0, Vec::new());
    for path in product_sources() {
        let text =
            fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        let name = path
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(&path)
            .display()
            .to_string();
        let product = product_head(&text, &name).unwrap_or_else(|why| panic!("{why}"));
        for lit in lits(product).iter().filter(|l| l.0.contains("#{")) {
            hits += 1;
            if control(lit.0, lit.1) {
                offenders.push(format!("{name}:{}", lit.2));
            }
        }
    }
    assert!(hits > 0, "the scan matched no format literal");
    assert!(
        offenders.is_empty(),
        "a tmux format would carry a control byte: {offenders:?}"
    );
}

/// The tmux this machine runs the suite against clears the floor.
///
/// A PANIC with a stated reason, never a skip. Below the floor every launch in
/// this suite is refused, so the arms that create a session would fail one by
/// one with no shared explanation; the CI lane that installs tmux is what keeps
/// this green there, and this is the test that says so when it does not.
#[test]
fn the_local_tmux_clears_the_floor_the_suite_launches_against() {
    let scratch = super::cli::OwnedScratch::root("floor", "floor").keep();
    // No tmux at all is the ABSENCE the other arms already state for
    // themselves; this test is about a tmux that IS there and is too old.
    if !super::phase2::tmux_present(&scratch) {
        let _ = fs::remove_dir_all(&scratch);
        return;
    }
    let out = scratch.join("version-out");
    let err = scratch.join("version-err");
    let ran = raw::run(&Invocation::new("tmux").arg("-V"), &scratch, &out, &err).is_ok();
    let printed = fs::read_to_string(&out).unwrap_or_default();
    let _ = fs::remove_dir_all(&scratch);

    let found = if ran {
        printed
            .trim()
            .strip_prefix("tmux ")
            .unwrap_or_default()
            .to_owned()
    } else {
        String::new()
    };
    let probe = Probe::Executable(found);
    assert!(
        probe.clears_floor(),
        "{}\nEvery launch this suite makes is refused below the floor, so the arms that \
         create a session cannot prove anything here.",
        tmux_floor::summary(&probe)
    );
}

// ---------------------------------------------------------------------------
// Below the floor, through the binary.
// ---------------------------------------------------------------------------

/// What a planted `tmux` says when it is asked about a SERVER.
enum Server {
    /// Nothing is running there, which is what sends the floor to `-V`.
    None,
    /// A server answered `#{version}` with this.
    Answers(&'static str),
    /// A server that could not be asked at all — not the same as an absent one,
    /// and the case the gate has to fail CLOSED on.
    Unreachable,
}

/// A `tmux` shim planted early on `PATH`, answering `-V` with `program` and the
/// version query per `server`.
///
/// A shim rather than a real old tmux: the point is what ae DOES at a version
/// it will not run on, and no machine can be asked to keep a 3.3a around to
/// prove it.
fn plant_tmux(dir: &Path, program: &str, server: &Server) -> PathBuf {
    let bin = dir.join("shim-bin");
    assert!(fs::create_dir_all(&bin).is_ok(), "a shim directory");
    let shim = bin.join("tmux");
    // Only the VERSION QUERY is answered specially; every other tmux word this
    // shim is asked fails the way tmux fails with no server, which is what a
    // machine with an old server and no session on it actually looks like.
    let answer = match server {
        Server::None => "echo \"no server running on /nonexistent\" >&2\nexit 1\n".to_owned(),
        Server::Answers(version) => format!("echo \"{version}\"\nexit 0\n"),
        // A real tmux says this when the socket is there and the server behind
        // it is not answering — a failure, and NOT an absence.
        Server::Unreachable => {
            "echo \"error connecting to /nonexistent (Connection refused)\" >&2\nexit 1\n"
                .to_owned()
        }
    };
    let script = format!(
        "#!/bin/sh\n\
         for word in \"$@\"; do\n\
         \tif [ \"$word\" = \"-V\" ]; then echo \"tmux {program}\"; exit 0; fi\n\
         \tif [ \"$word\" = '#{{version}}' ]; then\n\
         {answer}\
         \tfi\n\
         done\n\
         echo \"no server running on /nonexistent\" >&2\n\
         exit 1\n"
    );
    assert!(fs::write(&shim, script).is_ok(), "the shim");
    assert!(
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).is_ok(),
        "the shim is executable"
    );
    bin
}

/// The shim the two "below the floor" arms are written against: an old binary
/// and no server anywhere.
fn plant_old_tmux(dir: &Path) -> PathBuf {
    plant_tmux(dir, "3.3a", &Server::None)
}

/// `PATH` with `bin` in front of whatever the suite inherited — `git` and `sh`
/// must still resolve, so the real `PATH` is kept behind it.
fn path_with(bin: &Path) -> String {
    let inherited = std::env::var("PATH").unwrap_or_default();
    format!("{}:{inherited}", bin.display())
}

/// A LAUNCH below the floor refuses, and leaves nothing behind.
///
/// The second half is the one that matters: the refusal is only worth having if
/// it happens before the first-run config seeding and before the migration
/// chain, so a machine that cannot run ae is not also a machine ae has written
/// state onto.
#[test]
fn a_launch_below_the_floor_refuses_and_writes_nothing() {
    let (mut command, scratch) = super::cli::ae_hermetic();
    let home = scratch.path().to_owned();
    let bin = plant_old_tmux(&home);
    let out = command
        .env("PATH", path_with(&bin))
        .arg("belowfloor")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("found:    3.3a"), "{stderr}");
    assert!(stderr.contains("required: 3.4 or newer"), "{stderr}");
    assert!(stderr.contains("brew install tmux"), "{stderr}");

    // NOTHING under the ae home: no seeded config, no session directory, no
    // worktree. The shim directory is the only thing this test put there.
    for left in ["config", "sessions", "worktrees", ".ae"] {
        assert!(
            !home.join(left).exists(),
            "a refused launch wrote {left} under {}",
            home.display()
        );
    }
}

/// The two OFF-DIAGONALS: the gate asks the tmux a launch would actually use,
/// and it fails closed when it cannot ask at all.
///
/// A unit test cannot catch either — both are about which probe the transport
/// composes for the gate, and both are the cases where the two tmuxes on a
/// machine disagree, which is exactly when the answer matters.
#[test]
fn a_live_old_server_is_preferred_over_a_new_binary_on_path() {
    let (mut command, scratch) = super::cli::ae_hermetic();
    let home = scratch.path().to_owned();
    // A NEW tmux on `PATH`, and the server it would draw into is old: the
    // launch lands on the server, so the server is what decides.
    let bin = plant_tmux(&home, "4.0", &Server::Answers("3.3a"));
    let out = command
        .env("PATH", path_with(&bin))
        .arg("oldserver")
        .output()
        .expect("the ae binary should run");

    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("found:    3.3a"), "{stderr}");
    assert!(
        !stderr.contains("found:    4.0"),
        "the binary on PATH is not the one that would draw: {stderr}"
    );
    assert!(
        !home.join("config").exists(),
        "a refused launch wrote config"
    );
}

#[test]
fn a_server_that_cannot_be_asked_refuses_even_with_a_new_binary_on_path() {
    let (mut command, scratch) = super::cli::ae_hermetic();
    let home = scratch.path().to_owned();
    let bin = plant_tmux(&home, "4.0", &Server::Unreachable);
    let out = command
        .env("PATH", path_with(&bin))
        .arg("unreachable")
        .output()
        .expect("the ae binary should run");

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unprovable floor is a refused one: {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("found:    (no answer)"), "{stderr}");
    assert!(
        !home.join("config").exists(),
        "a refused launch wrote config"
    );
}

/// `list`, `version` and `upgrade` answer below the floor — which is how a
/// machine below it sees the problem and leaves it behind.
#[test]
fn the_recovery_commands_still_answer_below_the_floor() {
    let (_probe, scratch) = super::cli::ae_hermetic();
    let home = scratch.path().to_owned();
    let bin = plant_old_tmux(&home);
    let path = path_with(&bin);

    let run = |args: &[&str]| {
        let (mut command, _keep) = super::cli::ae_hermetic();
        let out = command
            .env("PATH", &path)
            .env("HOME", &home)
            .env("AE_HOME", home.join(".ae-state"))
            .args(args)
            .output()
            .expect("the ae binary should run");
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (code, stdout, stderr) = run(&["list"]);
    assert_eq!(code, Some(0), "list: {stdout}{stderr}");

    let (code, stdout, _) = run(&["version"]);
    assert_eq!(code, Some(0), "{stdout}");
    let floor = stdout.lines().nth(1).unwrap_or_default();
    assert!(floor.contains("3.3a"), "{stdout}");
    assert!(floor.contains("BELOW"), "{stdout}");

    // `upgrade` reaches its OWN vocabulary rather than the floor's.
    let (mut command, _keep) = super::cli::ae_hermetic();
    let out = command
        .env("PATH", &path)
        .env("AE_VERSION", "not-a-version")
        .arg("upgrade")
        .output()
        .expect("the ae binary should run");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("AE_VERSION"), "{stderr}");
    assert!(
        !stderr.contains("required: 3.4 or newer"),
        "the floor refused the one word that repairs a machine below it: {stderr}"
    );
}
