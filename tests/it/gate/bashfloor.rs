//! #337: a gate recipe never exits 0 when it did not do its job.
//!
//! `just test-affected` uses bash 4 syntax (`[[ -v ]]`, `exec {fd}<`, `local -A`,
//! `mapfile`). Under macOS `/bin/bash` 3.2 the script stops at a parse error and
//! the recipe's own `trap ... EXIT` then runs a successful `rm -rf`: bash 3.2
//! hands the shell the trap's last status, not the failed parse's, so `just`
//! reports success for a run that selected and ran nothing. Measured on 3.2: the
//! trap sees `$?` as 0 after that abort, so a status-capturing trap cannot repair
//! it. The only fix that works is to refuse before bash reads the broken syntax.
//!
//! The recipe therefore opens with the `BASH_FLOOR` guard, a POSIX `case` on
//! `$BASH_VERSION` placed BEFORE `set -euo pipefail`, so a shell that does not
//! know `pipefail` (dash) reaches it too. Pinned here:
//!
//! - the real recipe, run through `just`, refuses under a stand-in shell and
//!   under a real bash 3.x where one exists (the Mac's `/bin/bash`);
//! - the guard is the first command, matches versions as the floor says, and
//!   every bash recipe that holds bash 4 syntax carries it;
//! - on a host with a real 3.x, every recipe without the guard still parses;
//! - an inline `EXIT` trap keeps the status of the failure it cleans up after.
//!
//! LIMITS. The construct list below names what is known to need bash 4; it does
//! not prove a recipe correct under 4.4 or 3.2 (an empty `"${a[@]}"` under
//! `set -u` needs 4.4 and no text scan sees it). The behavioural tests run the
//! real recipe bytes; the structural ones only read them.

use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use super::{justfile_variable, read, root};
use crate::parity::Invocation;
use crate::parity::capture::ExitOutcome;
use crate::parity::capture::raw;

/// What the guard says first, with the recipe name from `$0` in front.
const REFUSAL: &str = "test-affected: refused: needs bash 4.4 or newer";

/// The recipe's script, as a `bash` older than the floor would be asked to run it.
const GUARDED: &str = "test-affected";

/// Bash text that needs a version above 3.2, matched as a substring of a line of
/// code: the text, and the version that introduced it.
const NEEDS_BASH_4_TEXT: [(&str, &str); 12] = [
    ("[[ -v ", "4.2"),
    ("declare -A", "4.0"),
    ("local -A", "4.0"),
    ("typeset -A", "4.0"),
    ("local -n", "4.3"),
    ("declare -n", "4.3"),
    ("^^}", "4.0"),
    (",,}", "4.0"),
    ("|&", "4.0"),
    ("&>>", "4.0"),
    ("wait -n", "4.3"),
    (";;&", "4.0"),
];

/// Bash words that need a version above 3.2, matched as whole words.
const NEEDS_BASH_4_WORD: [(&str, &str); 3] =
    [("mapfile", "4.0"), ("readarray", "4.0"), ("coproc", "4.0")];

/// Whether `line` holds `word` as a whole word.
fn has_word(line: &str, word: &str) -> bool {
    let inside = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    line.match_indices(word).any(|(at, _)| {
        let before = line.as_bytes()[..at].last().copied();
        let after = line.as_bytes().get(at + word.len()).copied();
        !before.is_some_and(inside) && !after.is_some_and(inside)
    })
}

/// Whether `line` holds the descriptor-allocating redirect of bash 4.1: `{name}`
/// at the start of a word and directly before `<` or `>`, as in `exec {fd}<file`
/// or `{fd}<&-`. Not `${name}<file`, and not the Perl block `exec { $ARGV[0] } @argv`.
fn named_descriptor(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.iter().enumerate().any(|(open, byte)| {
        if *byte != b'{' || (open > 0 && !matches!(bytes[open - 1], b' ' | b'\t')) {
            return false;
        }
        let name = &bytes[open + 1..];
        let length = name
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
            .count();
        length > 0
            && !name[0].is_ascii_digit()
            && name.get(length) == Some(&b'}')
            && matches!(name.get(length + 1), Some(b'<' | b'>'))
    })
}

/// The bash 4 syntax `script` holds, as `(what, since)`: code lines only, so a
/// comment that names `mapfile` does not count.
fn bash4_syntax(script: &str) -> Vec<(&'static str, &'static str)> {
    let code: Vec<&str> = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    let mut found = Vec::new();
    for (text, since) in NEEDS_BASH_4_TEXT {
        if code.iter().any(|line| line.contains(text)) {
            found.push((text, since));
        }
    }
    for (word, since) in NEEDS_BASH_4_WORD {
        if code.iter().any(|line| has_word(line, word)) {
            found.push((word, since));
        }
    }
    if code.iter().any(|line| named_descriptor(line)) {
        found.push(("a named descriptor ({fd}<)", "4.1"));
    }
    found
}

fn put_executable(path: &Path, body: &str) {
    std::fs::write(path, body)
        .and_then(|()| std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

/// `(exit outcome, stdout, stderr)` of one program in `cwd`.
struct Ran {
    outcome: ExitOutcome,
    out: String,
    err: String,
}

fn run_in(base: &Path, cwd: &Path, run: &Invocation) -> Ran {
    let (out, err) = (base.join("out"), base.join("err"));
    let ran = raw::run(run, cwd, &out, &err).unwrap_or_else(|why| panic!("the run: {why}"));
    Ran {
        outcome: ran.outcome(),
        out: read(&out),
        err: read(&err),
    }
}

/// The version `shell` reports in `$BASH_VERSION`, empty for a shell that is not bash.
fn bash_version_of(tag: &str, shell: &str) -> String {
    let base = crate::cli::OwnedScratch::root("gate", tag);
    let run = Invocation::new(shell)
        .env_cleared()
        .arg("-c")
        .arg("printf %s \"${BASH_VERSION-}\"");
    run_in(&base, &base, &run).out
}

/// Whether `version` (a `$BASH_VERSION`) is older than the 4.4 floor. A shell that
/// reports none is not bash and counts as older.
fn below_floor(version: &str) -> bool {
    let mut parts = version.split(['.', '(']);
    let major: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    (major, minor) < (4, 4)
}

/// `just test-affected print=yes` in a scratch copy of the real justfile, with a
/// `bash` first on `PATH` that execs `shell`: the same path `#!/usr/bin/env bash`
/// takes on a machine whose first bash is that shell.
fn recipe_under(tag: &str, shell: &str) -> Ran {
    let base = crate::cli::OwnedScratch::root("gate", tag);
    let (work, bin) = (base.join("work"), base.join("bin"));
    for dir in [&work, &bin] {
        std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
    }
    std::fs::write(work.join("justfile"), read(&root().join("justfile")))
        .unwrap_or_else(|why| panic!("the fixture justfile: {why}"));
    put_executable(
        &bin.join("bash"),
        &format!("#!/bin/sh\nexec {shell} \"$@\"\n"),
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(just) = std::env::split_paths(&path)
        .map(|dir| dir.join("just"))
        .find(|file| file.is_file())
    else {
        panic!("just must be on PATH");
    };
    symlink(just, bin.join("just")).unwrap_or_else(|why| panic!("the linked just: {why}"));
    let run = Invocation::new("just")
        .env_cleared()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &*base)
        .arg("test-affected")
        .arg("print=yes");
    run_in(&base, &work, &run)
}

fn assert_refused(who: &str, ran: &Ran) {
    assert!(
        matches!(ran.outcome, ExitOutcome::Code(1)),
        "{who}: the recipe must refuse with exit 1, not report success for a run that did nothing \
         (stdout {:?}, stderr {:?})",
        ran.out,
        ran.err
    );
    assert!(
        ran.err.contains(REFUSAL),
        "{who}: the refusal names the need ({REFUSAL:?}); stderr was {:?}",
        ran.err
    );
    assert!(
        ran.out.is_empty(),
        "{who}: a refused gate selects nothing, stdout was {:?}",
        ran.out
    );
}

/// The real recipe under a stand-in shell that cannot be bash 4.4: the host's
/// `/bin/sh` (dash on Linux, bash 3.2 in POSIX mode on a Mac). A host whose
/// `/bin/sh` is a new bash cannot stand in, and says so.
#[test]
fn the_merge_gate_refuses_under_a_shell_older_than_its_floor() {
    let version = bash_version_of("b32v1", "/bin/sh");
    if !below_floor(&version) {
        eprintln!("SKIPPED: /bin/sh is bash {version}, no stand-in for an old bash on this host");
        return;
    }
    assert_refused("stand-in /bin/sh", &recipe_under("b32t1", "/bin/sh"));
}

/// The real recipe under a real bash 3.x: the faithful #337 regression. On main
/// this exits 0 with one parse error and nothing run.
#[test]
fn the_merge_gate_refuses_under_a_real_bash_3() {
    let version = bash_version_of("b32v2", "/bin/bash");
    if !version.starts_with('3') {
        eprintln!(
            "SKIPPED: /bin/bash is {version:?}, not a bash 3.x; the stand-in test covers this host"
        );
        return;
    }
    assert_refused("real /bin/bash", &recipe_under("b32t2", "/bin/bash"));
}

/// The first body lines of one recipe, shebang included, as the justfile spells them.
fn first_lines(justfile: &str, header: &str, count: usize) -> Vec<String> {
    justfile
        .lines()
        .skip_while(|line| !line.starts_with(header))
        .skip(1)
        .take(count)
        .map(|line| line.trim().to_owned())
        .collect()
}

/// The guard is the first command: after the shebang and before `set`, so a shell
/// that rejects `set -o pipefail` and a bash that has not read any 4 syntax yet
/// both reach it.
#[test]
fn the_guard_is_the_first_command_of_the_merge_gate_before_set() {
    let justfile = read(&root().join("justfile"));
    assert_eq!(
        first_lines(&justfile, "test-affected *args:", 3),
        [
            "#!/usr/bin/env bash",
            "{{ BASH_FLOOR }}",
            "set -euo pipefail"
        ],
        "test-affected opens with its shebang, the guard, then `set -euo pipefail`"
    );
}

/// The guard text matches versions as the 4.4 floor says. Run under `/bin/sh` with
/// `BASH_VERSION` assigned, so one host checks every version it cannot run.
#[test]
fn the_guard_refuses_exactly_the_versions_below_four_four() {
    let guard = justfile_variable(&read(&root().join("justfile")), "BASH_FLOOR");
    assert!(!guard.trim().is_empty(), "the justfile defines BASH_FLOOR");
    let base = crate::cli::OwnedScratch::root("gate", "b32g");
    let cases = [
        ("3.2.57(1)-release", true),
        ("4.0.44(1)-release", true),
        ("4.3.48(1)-release", true),
        ("4.4.0(1)-release", false),
        ("4.4.23(1)-release", false),
        ("5.0.0(1)-release", false),
        ("5.2.21(1)-release", false),
        ("5.3.9(1)-release", false),
        ("", true),
    ];
    for (version, refused) in cases {
        let script = format!("BASH_VERSION=\"$1\"\n{guard}\necho reached\n");
        let run = Invocation::new("/bin/sh")
            .env_cleared()
            .arg("-c")
            .arg(script)
            .arg(GUARDED)
            .arg(version);
        let ran = run_in(&base, &base, &run);
        if refused {
            assert!(
                matches!(ran.outcome, ExitOutcome::Code(1)) && ran.err.contains(REFUSAL),
                "BASH_VERSION {version:?} must be refused with exit 1 and {REFUSAL:?}: {:?} {:?}",
                ran.out,
                ran.err
            );
            assert!(
                !ran.out.contains("reached"),
                "BASH_VERSION {version:?} ran past the guard"
            );
        } else {
            assert!(
                matches!(ran.outcome, ExitOutcome::Code(0)) && ran.out.trim() == "reached",
                "BASH_VERSION {version:?} must pass the guard: {:?} {:?}",
                ran.out,
                ran.err
            );
        }
    }
}

/// One shebang-bash recipe: its name, the arguments it needs to be asked for, and
/// whether the guard opens it.
struct BashRecipe {
    name: String,
    required: Vec<String>,
    guarded: bool,
}

/// Every `#!/usr/bin/env bash` recipe of the justfile.
fn bash_recipes(justfile: &str) -> Vec<BashRecipe> {
    let lines: Vec<&str> = justfile.lines().collect();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if line.trim() != "#!/usr/bin/env bash" || !line.starts_with(' ') {
            continue;
        }
        let Some(header) = lines[..at]
            .iter()
            .rev()
            .find(|row| !row.is_empty() && !row.starts_with([' ', '#', '\t']))
        else {
            continue;
        };
        let Some((head, _deps)) = header.split_once(": ").or_else(|| header.split_once(':')) else {
            continue;
        };
        let mut words = head.split_whitespace();
        let Some(name) = words.next() else {
            continue;
        };
        let required = words
            .filter(|word| !word.starts_with('*') && !word.contains('='))
            .map(|_| "x".to_owned())
            .collect();
        found.push(BashRecipe {
            name: name.to_owned(),
            required,
            guarded: lines
                .get(at + 1)
                .is_some_and(|next| next.trim() == "{{ BASH_FLOOR }}"),
        });
    }
    found
}

/// The script `just` would run for `recipe`: its dependencies and every `{{ }}`
/// resolved, nothing executed.
fn script_of(recipe: &BashRecipe, base: &Path) -> String {
    let run = recipe.required.iter().fold(
        Invocation::new("just")
            .arg("--dry-run")
            .arg("--no-deps")
            .arg(&recipe.name),
        Invocation::arg,
    );
    let ran = run_in(base, &root(), &run);
    assert!(
        matches!(ran.outcome, ExitOutcome::Code(0)),
        "just --dry-run {} must print the script: {}",
        recipe.name,
        ran.err
    );
    ran.err
}

/// A recipe that holds bash 4 syntax must carry the guard: the next recipe added
/// with `mapfile` or `local -A` cannot repeat #337.
#[test]
fn a_bash_recipe_with_bash_4_syntax_carries_the_guard() {
    let justfile = read(&root().join("justfile"));
    let recipes = bash_recipes(&justfile);
    assert!(
        recipes.len() >= 20,
        "the scan must find the shebang recipes, found {}",
        recipes.len()
    );
    assert!(
        recipes.iter().any(|recipe| recipe.name == GUARDED),
        "the scan must find test-affected"
    );
    let base = crate::cli::OwnedScratch::root("gate", "b32c");
    let mut unguarded = Vec::new();
    for recipe in recipes.iter().filter(|recipe| !recipe.guarded) {
        for (what, since) in bash4_syntax(&script_of(recipe, &base)) {
            unguarded.push(format!("{} uses `{what}` (bash {since})", recipe.name));
        }
    }
    assert!(
        unguarded.is_empty(),
        "these recipes need bash 4 and open without `{{{{ BASH_FLOOR }}}}`: {unguarded:#?}"
    );
    // The detector is not vacuous on the real file: it sees what #337 was about.
    let target = recipes.iter().find(|recipe| recipe.name == GUARDED);
    let seen = target.map(|recipe| bash4_syntax(&script_of(recipe, &base)));
    let seen: Vec<&str> = seen.iter().flatten().map(|(what, _)| *what).collect();
    for what in [
        "[[ -v ",
        "local -A",
        "mapfile",
        "a named descriptor ({fd}<)",
    ] {
        assert!(
            seen.contains(&what),
            "the detector must find `{what}` in the real test-affected script: {seen:?}"
        );
    }
}

/// The detector reads syntax, not text: it finds each construct on lines taken from
/// the real recipe, and it passes the Perl `exec { $ARGV[0] } @ARGV` block of
/// `_tmux-isolated` (a recipe that parses and runs under bash 3.2), a `${fd}<file`
/// expansion, a whole-word near miss and a comment that names the constructs.
#[test]
fn the_bash_4_detector_reads_syntax_and_not_text() {
    for (line, what) in [
        (
            "    exec {fd}<\"$3\" || return 1",
            "a named descriptor ({fd}<)",
        ),
        ("        exec {fd}<&-", "a named descriptor ({fd}<)"),
        ("    exec {out_fd}>\"$log\"", "a named descriptor ({fd}<)"),
        ("[[ -v text ]] || return 1", "[[ -v "),
        ("    local -A seen=()", "local -A"),
        ("    mapfile -t lines <\"$1\"", "mapfile"),
        ("    readarray -t rows < file", "readarray"),
        ("    coproc cat", "coproc"),
        ("    echo \"${name^^}\"", "^^}"),
        ("    make |& tee log", "|&"),
    ] {
        let found: Vec<&str> = bash4_syntax(line).iter().map(|(what, _)| *what).collect();
        assert!(found.contains(&what), "`{line}` holds `{what}`: {found:?}");
    }
    for line in [
        "            exec { $ARGV[0] } @ARGV or die \"detached: $ARGV[0]: $!\\n\";",
        "    cat ${fd}<file",
        "    echo {a,b}>out",
        "    echo coprocessor mapfiles",
        "    # mapfile, local -A and exec {fd}< are bash 4",
    ] {
        assert!(
            bash4_syntax(line).is_empty(),
            "`{line}` is not bash 4 syntax: {:?}",
            bash4_syntax(line)
        );
    }
}

/// On a host with a real bash 3.x, every recipe without the guard parses under it:
/// the class of #337 caught by the shell itself, not by a text list.
#[test]
fn every_unguarded_bash_recipe_parses_under_a_real_bash_3() {
    let version = bash_version_of("b32v3", "/bin/bash");
    if !version.starts_with('3') {
        eprintln!("SKIPPED: /bin/bash is {version:?}, not a bash 3.x");
        return;
    }
    let justfile = read(&root().join("justfile"));
    let base = crate::cli::OwnedScratch::root("gate", "b32p");
    let mut broken = Vec::new();
    for recipe in bash_recipes(&justfile)
        .iter()
        .filter(|recipe| !recipe.guarded)
    {
        let file: PathBuf = base.join(format!("{}.sh", recipe.name));
        std::fs::write(&file, script_of(recipe, &base))
            .unwrap_or_else(|why| panic!("{}: {why}", file.display()));
        let run = Invocation::new("/bin/bash").arg("-n").arg(&file);
        let ran = run_in(&base, &base, &run);
        if !matches!(ran.outcome, ExitOutcome::Code(0)) {
            broken.push(format!("{}: {}", recipe.name, ran.err.trim()));
        }
    }
    assert!(
        broken.is_empty(),
        "bash {version} cannot parse these recipes, and a parse error under 3.2 can exit 0: {broken:#?}"
    );
}

/// The inline `trap '...' EXIT` line of each recipe whose cleanup is one command
/// list in the justfile (the other trap recipes name a `cleanup` function that
/// captures the status itself): `(recipe, handler text)`.
fn inline_exit_traps(justfile: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut recipe = String::new();
    for line in justfile.lines() {
        if !line.starts_with([' ', '\t', '#']) && !line.is_empty() {
            line.split([' ', ':'])
                .next()
                .unwrap_or_default()
                .clone_into(&mut recipe);
        }
        let text = line.trim();
        if let Some(rest) = text.strip_prefix("trap '")
            && let Some(handler) = rest.strip_suffix("' EXIT")
        {
            found.push((recipe.clone(), handler.to_owned()));
        }
    }
    found
}

/// An inline `EXIT` trap captures `$?` first and exits with it, so its own cleanup
/// can never become the status of the run. Pinned as text for every such recipe,
/// and run: the real handler cleans up and leaves an ordinary failure, an explicit
/// status and a success as they were. (A parse abort under 3.2 is not covered here:
/// the trap sees `$?` as 0 there, which is why the version guard exists.)
#[test]
fn an_inline_exit_trap_keeps_the_status_of_the_run_it_cleans_up_after() {
    let justfile = read(&root().join("justfile"));
    let traps = inline_exit_traps(&justfile);
    let names: Vec<&str> = traps.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            "test-affected",
            "release-proof",
            "_linux-gate",
            "rust-linux-smoke"
        ],
        "the inline EXIT traps of the justfile"
    );
    let base = crate::cli::OwnedScratch::root("gate", "b32x");
    for (name, handler) in &traps {
        for (body, want) in [("false", 1), ("exit 7", 7), ("true", 0)] {
            let dir = base.join(format!("{name}-{want}"));
            std::fs::create_dir_all(&dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
            let script = format!(
                "set -euo pipefail\nscratch={0} reply_dir={0} lane={0}\ntrap '{handler}' EXIT\n{body}\n",
                dir.display()
            );
            let file = base.join(format!("{name}-{want}.sh"));
            std::fs::write(&file, script).unwrap_or_else(|why| panic!("{}: {why}", file.display()));
            for shell in ["bash", "/bin/bash"] {
                let ran = run_in(&base, &base, &Invocation::new(shell).arg(&file));
                assert!(
                    matches!(ran.outcome, ExitOutcome::Code(code) if code == want),
                    "{name} under {shell}: `{body}` must leave exit {want}: {:?}",
                    ran.err
                );
                assert!(
                    !dir.exists(),
                    "{name} under {shell}: the trap must still remove what it owns"
                );
                std::fs::create_dir_all(&dir)
                    .unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
            }
        }
        assert!(
            handler.starts_with("status=$?; ") && handler.ends_with("; exit \"$status\""),
            "{name}: the EXIT trap must capture `status=$?` first and end with `exit \"$status\"`: {handler}"
        );
    }
}
