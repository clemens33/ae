//! The GATE's own hazards — the two files ae still builds itself with.
//!
//! Three guards over the `justfile` and the `install` script — text this crate
//! never runs but every release does. Each one is a bug that already shipped.
//!
//! Every guard is a PURE FUNCTION of file text, and each is exercised against
//! deliberately broken input as well as the real file. A rule that matches
//! nothing is indistinguishable from a clean tree, so the red cases are the
//! half that makes a green run mean something.

#![allow(
    clippy::disallowed_methods,
    reason = "these read repository text; the capability boundary is about what PRODUCT \
              code may reach"
)]

use std::path::{Path, PathBuf};

/// The repository root — this file's crate manifest directory.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|why| panic!("{} must be readable: {why}", path.display()))
}

/// The EXECUTABLE text of one justfile recipe: body lines only, full-line
/// comments and blanks dropped, backslash continuations folded so one command
/// is one line.
fn recipe_text(justfile: &str, header: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut inside = false;
    for line in justfile.lines() {
        if !inside {
            inside = line.starts_with(header);
            continue;
        }
        if !line.starts_with([' ', '\t']) && !line.is_empty() {
            break;
        }
        let mut text = if buf.is_empty() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            trimmed.to_owned()
        } else {
            format!(" {}", line.trim_start())
        };
        let folded = text.trim_end();
        if let Some(head) = folded.strip_suffix('\\') {
            text = head.to_owned();
            buf.push_str(&text);
            continue;
        }
        buf.push_str(&text);
        out.push(std::mem::take(&mut buf));
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// Whether a bare `shellcheck` word starts at `at` in `line`.
fn bare_word_at(line: &str, at: usize, word: &str) -> bool {
    let boundary = |ch: char| !ch.is_ascii_alphanumeric() && !"_.@/-".contains(ch);
    let before = line[..at].chars().next_back().is_none_or(boundary);
    let after = line[at + word.len()..].chars().next().is_none_or(boundary);
    before && after
}

fn bare_word_count(text: &str, word: &str) -> usize {
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = text[from..].find(word).map(|index| index + from) {
        if bare_word_at(text, at, word) {
            count += 1;
        }
        from = at + word.len();
    }
    count
}

/// Whether the `lint` recipe PROTECTS shellcheck's stdin.
fn lint_redirect_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "lint:");
    let joined = lines.join("\n");
    if bare_word_count(&joined, "shellcheck") != 1 {
        return false;
    }
    lines.iter().filter(|line| bearing(line)).count() == 1
}

/// Whether one line IS the shellcheck command and carries the stdin redirect.
fn bearing(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("shellcheck") else {
        return false;
    };
    let Some(head) = rest.trim_end().strip_suffix("/dev/null") else {
        return false;
    };
    let Some(head) = head.trim_end_matches([' ', '\t']).strip_suffix('<') else {
        return false;
    };
    let head = head.strip_suffix('0').unwrap_or(head);
    head.ends_with([' ', '\t']) && !head.contains([';', '&', '|'])
}

/// Whether the pin recipe asks whether shellcheck EXISTS before probing it.
fn pin_availability_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "_shellcheck-pin:");
    let first = |needle: &str, tail: &str| {
        lines.iter().position(|line| {
            line.find(needle).is_some_and(|at| {
                line[at + needle.len()..]
                    .trim_start()
                    .starts_with(tail.trim_start())
            })
        })
    };
    match (
        first("command -v", " shellcheck"),
        first("shellcheck", " --version"),
    ) {
        (Some(available), Some(probe)) => available < probe,
        _ => false,
    }
}

#[test]
fn the_lint_recipe_protects_shellchecks_stdin() {
    assert!(
        lint_redirect_ok(&read(&root().join("justfile"))),
        "the real lint recipe must redirect shellcheck's stdin from /dev/null"
    );

    // RED — the comment-as-evidence lie: the only protected text is a comment.
    assert!(!lint_redirect_ok(
        "lint:\n    # shellcheck -x install < /dev/null\n    shellcheck -x install\n"
    ));
    // RED — the second-call lie: the redirect sits on a command doing no work.
    assert!(!lint_redirect_ok(
        "lint:\n    shellcheck install; shellcheck install < /dev/null\n"
    ));
    // RED — the plain regression this exists to catch.
    assert!(!lint_redirect_ok("lint:\n    shellcheck -x install\n"));
    // RED — the control-operator family.
    for tail in ["&& true", "|| true", "| cat", "& wait"] {
        assert!(
            !lint_redirect_ok(&format!(
                "lint:\n    shellcheck -x install {tail} < /dev/null\n"
            )),
            "'{tail}' takes the redirect"
        );
    }
    // RED — the numeric-fd family: the descriptor redirected is not stdin.
    for prefix in ["1", "2"] {
        assert!(
            !lint_redirect_ok(&format!(
                "lint:\n    shellcheck -x install {prefix}< /dev/null\n"
            )),
            "'{prefix}<' does not redirect stdin"
        );
    }
    // GREEN — the explicit spelling of the same thing IS stdin.
    assert!(lint_redirect_ok(
        "lint:\n    shellcheck -x install 0< /dev/null\n"
    ));
    // GREEN control — a folded multi-line invocation is still recognised, so
    // the guard is not passing by being blind to the shape the recipe uses.
    assert!(lint_redirect_ok(
        "lint:\n    shellcheck -x install \\\n        tests/x \\\n        y < /dev/null\n"
    ));
}

#[test]
fn the_pin_recipe_asks_whether_shellcheck_exists_before_probing_it() {
    assert!(
        pin_availability_ok(&read(&root().join("justfile"))),
        "the real pin recipe must test availability before the version probe"
    );
    // RED — the shape that shipped and was caught in review: probe first, no
    // availability test, so an absent binary kills the recipe at rc 127.
    assert!(!pin_availability_ok(
        "_shellcheck-pin:\n    want=\"0.11.0\"\n    have=\"$(shellcheck --version)\"\n"
    ));
}

/// The joined, comment-free text the portability rules read.
fn installer_lines(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        buf.push_str(line);
        let folded = buf.trim_end();
        if folded.ends_with("||") || folded.ends_with('\\') {
            continue;
        }
        out.push(std::mem::take(&mut buf));
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// What may legally follow the flag letter inside its option cluster.
#[derive(Clone, Copy)]
enum Tail {
    /// The flag takes an argument, which GNU allows ATTACHED — `stat -c%Y`,
    /// `date -dyesterday`.
    Arg,
    /// The flag takes NO argument, so cluster letters may legally FOLLOW it:
    /// `grep -Po` as well as `grep -oP`.
    NoArg,
    /// The argument must be separate — but `-i''` and `-i""` DO count, because
    /// the shell strips the quotes and what reaches sed is a bare `-i`, the
    /// GNU-only spelling that breaks BSD.
    WordFinal,
}

/// Whether `line` calls `cmd` with `flag` set, options in ANY order.
fn calls_with_flag(line: &str, cmd: &str, flag: char, tail: Tail) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(cmd).map(|index| index + from) {
        from = at + cmd.len();
        // A command token begins at start of line, after whitespace, or after a
        // shell operator, and may carry a PATH prefix — so `/usr/bin/date -d`
        // is caught while `iso-date -d` is not.
        let before = line[..at].chars().next_back();
        let starts = match before {
            None => true,
            Some('/') => {
                line[..at]
                    .trim_end_matches(|ch: char| ch != ' ' && ch != '\t')
                    .len()
                    <= at
            }
            Some(ch) => ch.is_whitespace() || "|&;(`".contains(ch),
        };
        if !starts {
            continue;
        }
        let mut rest = &line[at + cmd.len()..];
        while rest.starts_with([' ', '\t']) {
            let word = rest.trim_start_matches([' ', '\t']);
            let Some(cluster) = word.strip_prefix('-') else {
                break;
            };
            let end = cluster.find([' ', '\t']).unwrap_or(cluster.len());
            if cluster_hits(&cluster[..end], flag, tail) {
                return true;
            }
            rest = &cluster[end..];
        }
    }
    false
}

/// Whether an option cluster (the text after its leading `-`) sets `flag`.
fn cluster_hits(cluster: &str, flag: char, tail: Tail) -> bool {
    let Some(at) = cluster.find(flag) else {
        return false;
    };
    if !cluster[..at].chars().all(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    let after = &cluster[at + flag.len_utf8()..];
    match tail {
        Tail::Arg => true,
        Tail::NoArg => after.chars().all(|ch| ch.is_ascii_alphabetic()),
        Tail::WordFinal => matches!(after, "" | "''" | "\"\""),
    }
}

/// Every GNU-only form `source` still carries, by label.
fn portability_flags(source: &str) -> Vec<&'static str> {
    let lines: Vec<String> = installer_lines(source)
        .into_iter()
        // The inline exemption marker is dropped HERE rather than in the reader,
        // so this rule is exercised by the mutation cases too: a filter only the
        // real run passes through is a rule no test can falsify.
        .filter(|line| !line.contains("port-ok:"))
        .collect();
    let mut bad = Vec::new();
    // An `unless` is a REVIEWED pair on the same line — an explicit `command
    // -v` test or the portable spelling beside the GNU one.
    let mut flag = |label: &'static str, hit: &dyn Fn(&str) -> bool, unless: &str| {
        if lines
            .iter()
            .any(|line| hit(line) && (unless.is_empty() || !line.contains(unless)))
        {
            bad.push(label);
        }
    };
    flag(
        "stat-c",
        &|line| calls_with_flag(line, "stat", 'c', Tail::Arg),
        "",
    );
    flag(
        "date-d",
        &|line| calls_with_flag(line, "date", 'd', Tail::Arg),
        "",
    );
    flag(
        "sed-i",
        &|line| calls_with_flag(line, "sed", 'i', Tail::WordFinal),
        "",
    );
    flag(
        "grep-oP",
        &|line| calls_with_flag(line, "grep", 'P', Tail::NoArg),
        "",
    );
    flag("tac", &|line| word_call(line, "tac"), "command -v tac");
    flag(
        "md5sum",
        &|line| word_call(line, "md5sum"),
        "command -v md5sum",
    );
    // `/proc/sys/kernel/random/uuid` is the one allowed read:
    // existence-guarded, with a uuidgen fallback.
    flag(
        "proc",
        &|line| line.contains("/proc/"),
        "/proc/sys/kernel/random/uuid",
    );
    flag(
        "readlink-f",
        &|line| line.contains("readlink -f"),
        "realpath",
    );
    flag(
        "find-printf",
        &|line| {
            line.find("find")
                .is_some_and(|at| line[at..].contains(" -printf"))
        },
        "",
    );
    // GNU-only sed BRE alternation, and the GNU-only regex/replacement escapes.
    flag("sed-BRE-alternation", &|line| line.contains(r"\(^\|"), "");
    flag(
        "sed-GNU-escape",
        &|line| {
            line.find("sed ").is_some_and(|at| {
                let rest = &line[at..];
                let head = rest.split('|').next().unwrap_or(rest);
                ['s', 'n', 'b', 'w', '+', '?']
                    .iter()
                    .any(|ch| head.contains(&format!("\\{ch}")))
            })
        },
        "",
    );
    bad
}

/// Whether `line` calls the bare command `word`.
fn word_call(line: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(word).map(|index| index + from) {
        from = at + word.len();
        let before = line[..at].chars().next_back();
        let after = line[at + word.len()..].chars().next();
        let opens = matches!(before, None | Some(' ' | '(' | '|'));
        let closes = matches!(after, None | Some(' ' | '"' | '|'));
        if opens && closes {
            return true;
        }
    }
    false
}

#[test]
fn the_installer_carries_no_gnu_only_coreutils_and_no_unreviewed_exemption() {
    let source = read(&root().join("install"));
    assert_eq!(
        portability_flags(&source),
        Vec::<&str>::new(),
        "install must run on BSD userland as written"
    );
    // Marker budget.
    assert_eq!(
        source.matches("port-ok:").count(),
        0,
        "a new inline exemption needs review, not a passing suite"
    );

    // The guard must FIRE, not merely pass.
    for (line, label) in [
        (r#"ts="$(date -d "$x" +%s)""#, "date-d"),
        (r#"ts="$(date -u -d "2 hours ago" +%FT%TZ)""#, "date-d"),
        (r#"ts="$(date -dyesterday)""#, "date-d"),
        (r#"sed -i "s/a/b/" "$f""#, "sed-i"),
        (r#"sed -E -i "s/(a)/b/" "$f""#, "sed-i"),
        (r#"sed -i'' "s/a/b/" "$f""#, "sed-i"),
        (r#"m="$(stat -c %Y "$f")""#, "stat-c"),
        (r#"m="$(stat -Lc%Y "$f")""#, "stat-c"),
        (r#"v="$(grep -oP '"k":\s*"\K[^"]+' "$f")""#, "grep-oP"),
        (r#"v="$(grep -Po '\d+' "$f")""#, "grep-oP"),
        (r#"newest="$(tac "$f" | head -1)""#, "tac"),
        (r#"sum="$(md5sum "$f")""#, "md5sum"),
        (r#"ppid="$(cut -d' ' -f4 /proc/$pid/stat)""#, "proc"),
        (r#"real="$(readlink -f "$f")""#, "readlink-f"),
        (r"find . -name '*.x' -printf '%p\n'", "find-printf"),
        (r#"sed -E 's/\(^\|,\)//' "$f""#, "sed-BRE-alternation"),
        (r#"sed -E 's/\s+//' "$f""#, "sed-GNU-escape"),
        (r#"/usr/bin/date -d "$x" +%s"#, "date-d"),
    ] {
        assert!(
            portability_flags(line).contains(&label),
            "the rule set must flag {label} in: {line}"
        );
    }
    // And it must not fire on the portable spellings, or on a command whose
    // NAME merely ends in one it knows.
    for line in [
        r#"m="$(stat -f %m "$f")""#,
        r#"ts="$(date -u -j -f "%FT%TZ" "$x" +%s)""#,
        r#"sed -E 's/(a|b)/c/' "$f" > "$t" && mv "$t" "$f""#,
        r#"newest="$(tail -r "$f" | head -1)""#,
        r#"iso-date -d "$x""#,
        r#"unused-sed -i "s/a/b/""#,
        r#"if command -v tac >/dev/null; then tac "$f"; fi"#,
        r#"uuid="$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen)""#,
        r#"real="$(readlink -f "$f" 2>/dev/null || realpath "$f")""#,
    ] {
        assert_eq!(
            portability_flags(line),
            Vec::<&str>::new(),
            "the rule set must stay quiet on: {line}"
        );
    }
}

#[test]
fn the_bundle_recipe_is_the_one_definition_of_a_bundle_and_both_release_legs_call_it() {
    let justfile = read(&root().join("justfile"));
    let recipe = recipe_text(&justfile, "bundle version platform binary:").join("\n");
    // The three members and their published modes are DEFINED here, so a second
    // open-coded tar elsewhere could drift silently.
    for pin in [
        r#"cp "$binary" "$root/ae-core""#,
        r#"cp install "$root/install""#,
        "sums ae-core install > SHA256SUMS",
        r#"chmod 0555 "$root/ae-core" "$root/install""#,
        r#"chmod 0444 "$root/SHA256SUMS""#,
        r#"chmod 0555 "$root""#,
        // -F IS LOAD-BEARING on the foreign-member proof: without it the dots
        // in a CalVer version are BRE wildcards, so `2026.9.1` matches the
        // bytes `2026x9y1` and a wrong core passes the only check this host
        // can make of a binary it cannot run.
        r#"LC_ALL=C grep -Fqa -- "$version" "$binary""#,
    ] {
        assert_eq!(
            recipe.matches(pin).count(),
            1,
            "the bundle recipe must carry exactly one `{pin}`"
        );
    }

    let release = read(&root().join(".github/workflows/release.yml"));
    assert_eq!(
        release.matches(r#"just bundle "$version""#).count(),
        2,
        "both release legs bundle through the one recipe"
    );
    assert_eq!(
        release
            .matches(r#"got="$($bin --version | head -n1)""#)
            .count(),
        2,
        "both legs check ae-core's version against the tag"
    );
    // No second binary has a version word or is copied into a bundle.
    for retired in ["_AE_ENTRY_VERSION", "AE_VERSION=", r#"$root/ae""#] {
        assert_eq!(
            release.matches(retired).count(),
            0,
            "the release workflow must not name the retired `{retired}`"
        );
    }
}

/// The musl compiler/linker name the justfile pins, from `RUST_MUSL_CC := "…"`.
fn justfile_musl_cc(justfile: &str) -> String {
    justfile
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("RUST_MUSL_CC := "))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

/// The linker `.cargo/config.toml` pins for the musl target: the first
/// `linker = "…"` under `[target.x86_64-unknown-linux-musl]` and no other.
fn cargo_musl_linker(config: &str) -> String {
    let mut inside = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[target.x86_64-unknown-linux-musl]";
            continue;
        }
        if inside && let Some(value) = line.strip_prefix("linker = ") {
            return value.trim().trim_matches('"').to_owned();
        }
    }
    String::new()
}

/// ONE NAME FOR THE CROSS COMPILER, in the two files that have to agree.
#[test]
fn the_musl_cross_compiler_has_one_spelling_in_the_justfile_and_the_cargo_config() {
    let justfile = read(&root().join("justfile"));
    let config = read(&root().join(".cargo/config.toml"));

    let pinned = justfile_musl_cc(&justfile);
    assert!(
        !pinned.is_empty(),
        "the justfile must pin RUST_MUSL_CC — it is the one name three readers share"
    );
    assert_eq!(
        cargo_musl_linker(&config),
        pinned,
        "`.cargo/config.toml`'s musl linker and the justfile's RUST_MUSL_CC must name one compiler"
    );

    // RED — each parser must read its OWN section, not any line that looks like
    // one.
    assert_eq!(
        cargo_musl_linker("[target.aarch64-apple-darwin]\nlinker = \"wrong\"\n"),
        "",
        "a linker pinned for another target is not the musl one"
    );
    assert_eq!(
        cargo_musl_linker("[target.x86_64-unknown-linux-musl]\nlinker = \"cc\"\n"),
        "cc"
    );
    assert_eq!(
        justfile_musl_cc("RUST_CROSS_TARGET := \"x86_64-unknown-linux-musl\"\n"),
        "",
        "the target triple is not the compiler name"
    );
    assert_eq!(
        justfile_musl_cc("RUST_MUSL_CC := \"probe-gcc\"\n"),
        "probe-gcc"
    );
}

/// A RELEASE IS BUILT AND PUBLISHED HERE, and it refuses before it can half
/// finish (human ruling, 2026-09-04: agents release locally, Actions optional).
#[test]
fn a_release_builds_both_bundles_locally_and_proves_its_rights_before_the_bump() {
    let justfile = read(&root().join("justfile"));

    // `just bundle` stays the ONE definition of a bundle: `bundles` calls it
    // once per platform, exactly as the two workflow legs do.
    let bundles = recipe_text(&justfile, "bundles:").join("\n");
    assert!(
        !bundles.is_empty(),
        "the justfile must carry a `bundles` recipe"
    );
    for pin in [
        r#"just bundle "$version" darwin-arm64"#,
        r#"just bundle "$version" linux-x86_64-musl"#,
        "sums -- ae-*.tar.gz > SHA256SUMS",
    ] {
        assert_eq!(
            bundles.matches(pin).count(),
            1,
            "the bundles recipe must carry exactly one `{pin}`"
        );
    }
    // The static proof is LOCAL and it is the pinned toolchain's, not the
    // machine's: macOS ships no readelf, and llvm-readobj arrives with the
    // llvm-tools component rust-toolchain.toml already pins.
    for pin in [
        "rustc --print sysroot",
        "llvm-readobj",
        "--program-headers",
        "PT_INTERP",
    ] {
        assert!(
            bundles.contains(pin),
            "the bundles recipe must prove the musl half static via `{pin}`"
        );
    }

    // THE ORDER OF THE RELEASE, read off the recipe itself.
    let release = recipe_text(&justfile, "release:");
    let step = |needle: &str| {
        release
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("the release recipe must run `{needle}`"))
    };
    let rights = step(".permissions.push");
    let branch = step("releases must be from");
    let bump = step("just bump");
    let build = step("just bundles");
    let assets = step("just bundles did not produce it");
    let push = step("git push");
    let publish = step("gh release create");
    assert!(
        rights < bump && branch < bump,
        "push rights and the branch are proved before the bump writes a version file"
    );
    assert!(
        bump < build,
        "the bundles are built from the version the bump just wrote"
    );
    assert!(
        build < assets && assets < push,
        "the notes and every asset exist before anything is pushed"
    );
    assert!(
        push < publish,
        "the branch is pushed before the release names one of its commits"
    );
    assert!(
        release.iter().any(|line| line.contains("--notes-file")),
        "the release body reaches gh as a file, not as an argv-sized string"
    );

    // THE REMOTE TAG IS THE RELEASE'S, NOT A PUSH'S.
    assert!(
        !release
            .iter()
            .any(|line| line.contains("git push") && line.contains("\"$TAG\"")),
        "the tag must never be pushed ahead of the release that carries its assets"
    );
    assert!(
        release
            .iter()
            .any(|line| line.contains("gh release create") && line.contains("--target")),
        "`gh release create --target` is what creates the remote tag"
    );

    // The dispatch-only workflow is retained as a MANUAL Linux run-proof lane.
    let workflow = read(&root().join(".github/workflows/release.yml"));
    assert!(
        workflow.contains("on:\n  workflow_dispatch:"),
        "the release workflow is dispatch-only"
    );
    assert!(
        !workflow.contains("  push:\n    tags:"),
        "the release workflow must not be tag-triggered — `just release` publishes"
    );

    // Both workflow legs that LINK musl name their own linker, because the
    // pin in `.cargo/config.toml` is the macOS cross toolchain's name and
    // Ubuntu's musl-tools ships no triple-prefixed alias.
    for file in [
        ".github/workflows/release.yml",
        ".github/workflows/rust.yml",
    ] {
        assert_eq!(
            read(&root().join(file))
                .matches("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: musl-gcc")
                .count(),
            1,
            "{file} must name the musl linker its runner actually has"
        );
    }
}

#[test]
fn the_release_workflow_is_a_manual_proof_lane_with_artifacts_only() {
    let workflow = read(&root().join(".github/workflows/release.yml"));
    let header = workflow
        .split_once("on:\n")
        .map_or(workflow.as_str(), |(header, _)| header);
    assert!(
        !["gh release upload", "gh release create", "contents: write"]
            .iter()
            .any(|banned| header.contains(banned)),
        "the workflow header must not mention release mutation or write permission"
    );
    assert!(
        workflow.contains("on:\n  workflow_dispatch:"),
        "the release workflow is dispatch-only"
    );
    assert!(
        !workflow.contains("  push:\n    tags:"),
        "the release workflow must not be tag-triggered — `just release` publishes"
    );
    assert_eq!(
        workflow.matches("uses: actions/upload-artifact@").count(),
        2,
        "both proof legs must upload their bundles as workflow artifacts"
    );
    assert!(
        !workflow.contains("gh release upload"),
        "the proof-only workflow must not upload or overwrite release assets"
    );
    assert!(
        !workflow.contains("contents: write"),
        "the proof-only workflow must not request release write permission"
    );
    assert!(
        !workflow.contains("gh release create"),
        "the proof-only workflow must not create a GitHub release"
    );
}

/// One command in `dir`, with `PATH` led by that fixture's own `bin`.
fn in_fixture(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
    let mut invocation = super::parity::Invocation::new(program)
        .env("PATH", {
            let real = std::env::var("PATH").unwrap_or_default();
            format!("{}:{real}", dir.join("bin").display())
        })
        .env("HOME", dir)
        .env("GIT_CONFIG_GLOBAL", dir.join("gitconfig"));
    for arg in args {
        invocation = invocation.arg(arg);
    }
    for (key, value) in env {
        invocation = invocation.env(key, value);
    }
    let out = dir.join("out");
    let err = dir.join("err");
    let status = super::parity::capture::raw::run(&invocation, dir, &out, &err)
        .unwrap_or_else(|why| panic!("{program} must be runnable: {why}"));
    let code = match status.outcome() {
        super::parity::capture::ExitOutcome::Code(code) => code,
        super::parity::capture::ExitOutcome::Signalled => -1,
    };
    (code, std::fs::read_to_string(&out).unwrap_or_default())
}

/// `just bump` derives the next calendar-version sequence from the repository's OWN tags,
/// and moves both version-bearing files or neither.
#[test]
fn just_bump_derives_the_next_sequence_from_the_tags_and_refuses_a_stale_recovery() {
    let root = root();
    let dir = PathBuf::from(format!("/tmp/ae-gate-bump.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        std::fs::create_dir_all(dir.join("bin")).is_ok(),
        "a fixture"
    );
    for name in ["Cargo.toml", "Cargo.lock", "justfile"] {
        assert!(
            std::fs::copy(root.join(name), dir.join(name)).is_ok(),
            "the fixture needs {name}"
        );
    }
    let shim = dir.join("bin").join("date");
    assert!(
        std::fs::write(
            &shim,
            "#!/bin/sh\ncase \"$2\" in\n+%Y) printf '%s\\n' \"$AE_FIXTURE_YEAR\" ;;\n\
             +%m) printf '%s\\n' \"$AE_FIXTURE_MONTH\" ;;\n*) exit 1 ;;\nesac\n",
        )
        .is_ok(),
        "the date shim"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert!(
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable date shim"
        );
    }
    for words in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "calver@example.invalid"],
        vec!["config", "user.name", "CalVer"],
        vec!["add", "Cargo.toml", "Cargo.lock", "justfile"],
        vec!["commit", "-qm", "fixture"],
    ] {
        let (code, _) = in_fixture(&dir, "git", &words, &[]);
        assert_eq!(code, 0, "git {words:?} must succeed");
    }
    let bump = |year: &str, month: &str| {
        in_fixture(
            &dir,
            "just",
            &["bump"],
            &[("AE_FIXTURE_YEAR", year), ("AE_FIXTURE_MONTH", month)],
        )
    };
    let crate_version = || {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.strip_prefix("version = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_default()
            .to_owned()
    };
    let tag = |name: &str| {
        let (code, _) = in_fixture(&dir, "git", &["tag", name], &[]);
        assert_eq!(code, 0, "the fixture takes tag {name}");
    };

    // No matching tag: the month's first release is sequence 1, and the month
    // is written UNPADDED even though `date +%m` answers `09`.
    let (code, stdout) = bump("2026", "09");
    assert_eq!((code, stdout.trim()), (0, "2026.9.1"));
    assert_eq!(crate_version(), "2026.9.1", "Cargo.toml moved");
    assert!(
        std::fs::read_to_string(dir.join("Cargo.lock"))
            .unwrap_or_default()
            .contains("version = \"2026.9.1\""),
        "and Cargo.lock moved with it — both files or neither"
    );
    assert!(
        !dir.join(".ae-bump-recovery").exists(),
        "a completed bump leaves no recovery marker"
    );

    // Two tags of the same month: one past the HIGHEST, not one past the count.
    tag("v2026.9.1");
    tag("v2026.9.2");
    let (code, stdout) = bump("2026", "09");
    assert_eq!((code, stdout.trim()), (0, "2026.9.3"));

    // A STALE recovery marker refuses before any edit.
    tag("v2026.9.3");
    let before = crate_version();
    assert!(
        std::fs::create_dir(dir.join(".ae-bump-recovery")).is_ok(),
        "a stale marker"
    );
    let (code, stdout) = bump("2026", "09");
    assert_ne!(code, 0, "a stale recovery marker must fail the bump");
    assert_eq!(stdout, "", "and emit no version anyone could act on");
    assert_eq!(crate_version(), before, "and leave the live files alone");
    assert!(
        dir.join(".ae-bump-recovery").exists(),
        "and preserve the marker for the recovery it names"
    );
    assert!(
        std::fs::remove_dir_all(dir.join(".ae-bump-recovery")).is_ok(),
        "the marker clears"
    );

    // The month rolls over and the sequence RESETS, tags of the old month
    // notwithstanding.
    let (code, stdout) = bump("2026", "10");
    assert_eq!((code, stdout.trim()), (0, "2026.10.1"));

    let _ = std::fs::remove_dir_all(&dir);
}
