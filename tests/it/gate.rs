//! The GATE's own hazards — the two files ae still builds itself with.
//!
//! Slice Z4 retired the bash suites, and three guards in them had no subject in
//! `src` to move to: they are about the `justfile` and the `install` script,
//! which are text this crate never runs but every release does. They come here
//! rather than disappearing, because each one is a bug that already shipped.
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
///
/// A `#` line is only a comment when no fold is in progress — mid-continuation
/// it is argument text, not commentary.
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
///
/// A shell TOKEN boundary, not an identifier one: an identifier boundary
/// treats `-` and `.` as separators, so `pcre-shellcheck` would match the name
/// inside a longer one.
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
///
/// The linter reads stdin when fd 0 is open, and an agent harness hands its
/// tool calls a UNIX SOCKET. If that socket's peer has not closed by the time
/// the read happens it never returns EOF and the process blocks FOREVER at 0.0%
/// CPU — wedges observed at 4h40m, 8h40m, 16h52m and 18h33m beside successful
/// runs of the same command, with nothing in between: a race on the peer's
/// close, not slowness. Every input is already on the argv, so `< /dev/null`
/// costs nothing and removes the race.
///
/// SOURCE SHAPE ONLY, and the name says so: this proves the redirect is
/// WRITTEN, not that a wedge cannot occur. The behaviour is falsifiable only
/// against a fifo that is never closed — a rig no test can own without risking
/// the hang it is testing for.
///
/// Two ways a guard of this kind lies, both closed below because both were
/// demonstrated against its first version:
///
/// * a COMMENT is not evidence — a commented redirect satisfies any grep over
///   raw text while the real command runs unprotected, so comments are stripped
///   before anything is asserted;
/// * a SECOND CALL hides behind the first — the tail test alone passes
///   `shellcheck ae; shellcheck install < /dev/null`, where the protected
///   command is not the one doing the work. So the count of `shellcheck` tokens
///   in EXECUTABLE text must be exactly one, and that one line must BE the
///   command rather than merely contain it.
fn lint_redirect_ok(justfile: &str) -> bool {
    let lines = recipe_text(justfile, "lint:");
    let joined = lines.join("\n");
    if bare_word_count(&joined, "shellcheck") != 1 {
        return false;
    }
    lines.iter().filter(|line| bearing(line)).count() == 1
}

/// Whether one line IS the shellcheck command and carries the stdin redirect.
///
/// Every shell control operator hands the redirect to a LATER command while
/// shellcheck keeps the inherited fd — `&& true`, `|| true`, `| cat` and
/// `& wait` all did exactly that against an earlier version of this predicate —
/// so the segment between the token and the redirect must carry none of `;`,
/// `&`, `|`.
///
/// The `<` must also be BARE, or the explicit `0<`. A NUMERIC fd prefix
/// redirects the wrong descriptor and leaves stdin exactly as inherited:
/// measured, `printf X | bash -c 'cat 2< /dev/null'` still prints X, and
/// `1< /dev/null` only makes stdout read-only, which a run with no findings
/// never notices. Both spellings keep the wedge while reading green.
///
/// Deliberately loud in the false-positive direction: a legitimate `2>&1` on
/// this line is rejected, its `&` being indistinguishable here from a
/// backgrounding one. If that day comes, split the redirection onto its own
/// construct or widen this with a case that is MEASURED — do not delete the
/// check to make a recipe pass.
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
///
/// Under `set -e` with pipefail, `have="$(shellcheck --version | awk …)"`
/// ABORTS at rc 127 when the binary is absent: the probe's stderr is already
/// redirected, so the recipe dies with a bare exit code and none of the install
/// guidance ever prints — on exactly the fresh machine that needs it. ORDER is
/// the invariant, not mere presence, so a later reordering is caught too.
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
    // RED — the control-operator family. Each has exactly one `shellcheck`
    // token, starts the line with it and still ends in `< /dev/null`, but the
    // redirect belongs to the command AFTER the operator.
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
///
/// `||`- and `\`-continued lines are folded so a GNU call and its BSD fallback
/// on the next line read as ONE line; otherwise every multi-line fallback
/// trips.
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
    /// `date -dyesterday`. Nothing may be required after the letter.
    Arg,
    /// The flag takes NO argument, so cluster letters may legally FOLLOW it:
    /// `grep -Po` as well as `grep -oP`.
    NoArg,
    /// The argument must be separate — but `-i''` and `-i""` DO count, because
    /// the shell strips the quotes and what reaches sed is a bare `-i`, the
    /// GNU-only spelling that breaks BSD. `-i.bak` stays deliberately quiet.
    WordFinal,
}

/// Whether `line` calls `cmd` with `flag` set, options in ANY order.
///
/// `date -u -d` and `sed -E -i` are the same GNU-only calls as `date -d` and
/// `sed -i`; a pattern anchored to the canonical order lets them through
/// unflagged, which is how two of these shipped.
fn calls_with_flag(line: &str, cmd: &str, flag: char, tail: Tail) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(cmd).map(|index| index + from) {
        from = at + cmd.len();
        // A command token begins at start of line, after whitespace, or after a
        // shell operator, and may carry a PATH prefix — so `/usr/bin/date -d`
        // is caught while `iso-date -d` is not. The prefix must end in `/`,
        // which no suffix-named command does.
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
///
/// Each form fails SILENTLY on BSD/macOS: the command errors, the `|| fallback`
/// value lands, and the feature reads as "nothing found" rather than "broken".
/// That whole class shipped — see AGENTS.md, "GNU vs BSD userland".
///
/// The subject is `install`, and since slice Z3 it is the whole subject: the
/// one bash file ae still ships, running on both userlands, on the
/// highest-authority path there is — a silently-wrong install is not a
/// diagnosable one.
fn portability_flags(source: &str) -> Vec<&'static str> {
    let lines: Vec<String> = installer_lines(source)
        .into_iter()
        // The inline exemption marker is dropped HERE rather than in the reader,
        // so this rule is exercised by the mutation cases too: a filter only the
        // real run passes through is a rule no test can falsify.
        .filter(|line| !line.contains("port-ok:"))
        .collect();
    let mut bad = Vec::new();
    // An `unless` is a REVIEWED pair on the same line — an explicit `command -v`
    // test or the portable spelling beside the GNU one. There is deliberately no
    // generic `GNU || BSD` allow: it could not tell an EQUIVALENT pair from a
    // mismatched one, and `stat -c %Y fileA || stat -f %z fileB` passed it.
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
    // `/proc/sys/kernel/random/uuid` is the one allowed read: existence-guarded,
    // with a uuidgen fallback. Any OTHER /proc path is Linux-only and silently
    // empty.
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
    // Marker budget. Each `port-ok:` is a reviewed, irreducible exception. The
    // only one ae ever carried was `ae transfer`'s ssh heredoc, which ran on the
    // REMOTE host; transfer was cut, so the budget is ZERO.
    assert_eq!(
        source.matches("port-ok:").count(),
        0,
        "a new inline exemption needs review, not a passing suite"
    );

    // The guard must FIRE, not merely pass. Each case feeds ONE offending line
    // through the real rule set — the option-ordered spellings included, because
    // `date -u -d` and `sed -E -i` are exactly what an order-anchored pattern
    // let through.
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
    // open-coded tar elsewhere could drift silently. That is what these refuse.
    for pin in [
        r#"cp "$binary" "$root/ae-core""#,
        r#"cp install "$root/install""#,
        "sums ae-core install > SHA256SUMS",
        r#"chmod 0555 "$root/ae-core" "$root/install""#,
        r#"chmod 0444 "$root/SHA256SUMS""#,
        r#"chmod 0555 "$root""#,
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
        release.matches(r#"got="$($bin --version)""#).count(),
        2,
        "both legs check ae-core's version against the tag"
    );
    // No wrapper survives to have a version word or to be copied into a bundle.
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
///
/// `.cargo/config.toml` pins the linker rustc invokes for the musl target and
/// the justfile pins `CC_x86_64_unknown_linux_musl` for the C ring compiles.
/// They are the same program, and a laptop that links the Linux half with one
/// while compiling C with another produces an artifact nobody chose.
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

    // RED — each parser must read its OWN section, not any line that looks
    // like one. A rule that matches anything cannot notice drift.
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
///
/// The ordering is the safety property. Push rights are proved before the bump
/// writes a version file; the bundles are built and proven before a tag exists;
/// only then is anything pushed or attached. A release that dies after
/// `git push --tags` has published a version with no assets behind it, and
/// every assertion here is one step of the order that prevents it.
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
    let bump = step("just bump");
    let build = step("just bundles");
    let tag = step("git tag");
    let publish = step("gh release create");
    assert!(
        rights < bump,
        "push rights are proved before the bump writes a version file"
    );
    assert!(
        bump < build && build < tag,
        "both bundles are built and proven after the bump and before the tag"
    );
    assert!(
        tag < publish,
        "the release object is created only once the tag exists"
    );
    assert!(
        release.iter().any(|line| line.contains("--notes-file")),
        "the release body reaches gh as a file, not as an argv-sized string"
    );

    // The tag-triggered workflow is retained as a MANUAL Linux run-proof lane.
    // Re-arming its push trigger would put Actions back on the critical path,
    // which is the thing the ruling removed.
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
///
/// The scheme is SemVer-compatible calendar versioning, `YYYY.M.N`: N is one past the highest
/// tag of the current month, it RESETS when the month rolls over, and the month
/// is never zero-padded — a padded `v2026.09.1` would sort and compare as a
/// different version from the `2026.9.1` the crate carries.
///
/// The fixture is its own git repository with its own tags, and a `date` shim
/// supplies a deterministic UTC year and month, so nothing here depends on when
/// it runs. The recipe under test is the REAL one, copied in beside the two
/// files it edits.
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

    // A STALE recovery marker refuses before any edit. The marker is how an
    // interrupted bump says the two files may disagree, so starting another one
    // over it would publish a version derived from a tree nobody has inspected.
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
