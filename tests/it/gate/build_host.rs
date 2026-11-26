//! D59: the `MacBook` never builds ae; the hp and m2 do.
//!
//! Releases and whole gates that ran on the `MacBook` filled `target/` there until
//! the disk was full. `_build-host` refuses on a Mac with an internal battery and
//! is silent everywhere else. Pinned here:
//!
//! - every recipe that compiles or runs the workspace, directly or through a
//!   recipe that does, refuses before any cargo call of its own runs. The set
//!   is DERIVED from the justfile text, so a new such recipe is covered or the
//!   test is red;
//! - the lightweight recipes (the bash lane, formatting, the lane lock and the
//!   like) never reach the guard;
//! - the detector refuses a `pmset -g batt` that names an internal battery and
//!   nothing else: a Mac mini, a failed or unreadable `pmset`, no `pmset` at all,
//!   and the human override all pass;
//! - every derived recipe, run for real in a scratch copy, refuses before any
//!   tool it would drive runs.
//!
//! LIMIT. The derivation reads recipe bodies, the dependency headers and the
//! `{{ VARIABLE }}` shell texts, and it follows a `just <recipe>` that is not
//! inside quotes. A build reached through a script file is not seen: the
//! justfile runs none.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use super::{bare_word_at, justfile_variable, read, recipe_text, root};
use crate::cli::OwnedScratch;
use crate::parity::Invocation;
use crate::parity::capture::ExitOutcome;
use crate::parity::capture::raw;

const GUARD: &str = "_build-host";

/// The guard's shell text as a recipe body names it: `_build-host` is this and
/// nothing else, and `release` opens with it.
const GUARD_TEXT: &str = "{{ BUILD_HOST }}";

/// What the guard says first.
const REFUSAL: &str = "build-host: refused: ";

/// The recipes that compile or run the workspace today. The derivation must
/// still find each: a detector that goes blind cannot pass.
const BUILDS: [&str; 27] = [
    "_linux-gate",
    "_pre-test",
    "_tmux-isolated",
    "bundles",
    "guards",
    "harness-smoke",
    "install",
    "pre",
    "release",
    "release-proof",
    "rust-build-release",
    "rust-check",
    "rust-cov",
    "rust-fuzz",
    "rust-fuzz-all",
    "rust-fuzz-check",
    "rust-linux",
    "rust-linux-26",
    "rust-lint",
    "rust-mutants",
    "rust-stress",
    "rust-test",
    "rust-watch",
    "test",
    "test-affected",
    "test-macos",
    "web-smoke",
];

/// What stays allowed on the `MacBook`: nothing here compiles the workspace.
const LIGHT: [&str; 11] = [
    "check",
    "format",
    "format-check",
    "lane-lock",
    "lint",
    "rust-deny",
    "rust-fmt",
    "rust-fmt-check",
    "rust-prune",
    "rust-setup",
    "rust-vet",
];

/// The arguments a derived recipe needs before `just` will run it.
const ARGUMENTS: [(&str, &[&str]); 4] = [
    ("_linux-gate", &["linux", "image", "off"]),
    ("_tmux-isolated", &["test"]),
    ("rust-fuzz", &["target=config_parse"]),
    ("rust-stress", &["test(=a)"]),
];

/// The tools a build recipe drives. Each is a stub that only records its call.
/// `chrome` stands in for the browser `web-smoke` asks for first.
const DRIVEN: [&str; 11] = [
    "bacon", "cargo", "chrome", "curl", "docker", "gh", "rsync", "rustup", "scp", "ssh", "tmux",
];

/// What `pmset -g batt` prints on the `MacBook`, as measured on it.
const MACBOOK: &str = "Now drawing from 'AC Power'\n \
     -InternalBattery-0 (id=25296995)\t96%; charging; 0:27 remaining present: true";

/// What it prints on m2, a Mac mini.
const MAC_MINI: &str = "Now drawing from 'AC Power'";

// ── The derivation ───────────────────────────────────────────────────

struct Recipe {
    /// The recipe's dependencies, in the order `just` runs them.
    deps: Vec<String>,
    /// The code of its body: comments dropped, `{{ VARIABLE }}` texts expanded.
    code: Vec<String>,
    /// The guard's text is the first command of its body.
    opens_with_guard: bool,
}

/// The name and the dependencies of a recipe header line, or `None` for any
/// other line: a comment, an assignment, a setting or an attribute.
fn header(line: &str) -> Option<(String, Vec<String>)> {
    if !line.starts_with(|first: char| first.is_ascii_alphabetic() || "_@".contains(first)) {
        return None;
    }
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(at, ch)| {
        quoted ^= ch == '"';
        (ch == ':' && !quoted).then_some(at)
    })?;
    if line[colon + 1..].starts_with('=') {
        return None;
    }
    let name = line[..colon].split_whitespace().next()?;
    if ["set", "export", "import", "alias", "mod"].contains(&name) {
        return None;
    }
    let mut deps = Vec::new();
    let mut rest = line[colon + 1..].trim_start();
    while !rest.is_empty() && !rest.starts_with("&&") {
        let (word, after) = match rest.strip_prefix('(') {
            Some(call) => {
                let (inside, after) = call.split_once(')')?;
                (inside.split_whitespace().next()?, after)
            }
            None => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
        };
        deps.push(word.to_owned());
        rest = after.trim_start();
    }
    Some((name.trim_start_matches('@').to_owned(), deps))
}

/// Every recipe of `justfile` by name.
fn recipes(justfile: &str) -> BTreeMap<String, Recipe> {
    let mut found = BTreeMap::new();
    let mut in_text = false;
    for line in justfile.lines() {
        if in_text {
            in_text = !line.starts_with("'''");
            continue;
        }
        if line.ends_with(":= '''") {
            in_text = true;
            continue;
        }
        let Some((name, deps)) = header(line) else {
            continue;
        };
        let body = recipe_text(justfile, line);
        let opens_with_guard = body
            .iter()
            .find(|text| *text != "set -euo pipefail")
            .is_some_and(|text| text == GUARD_TEXT);
        let mut code = Vec::new();
        for text in body {
            let variable = text
                .strip_prefix("{{ ")
                .and_then(|rest| rest.strip_suffix(" }}"))
                .map(|variable| justfile_variable(justfile, variable))
                .filter(|body| !body.is_empty());
            match variable {
                Some(body) => code.extend(
                    body.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty() && !line.starts_with('#'))
                        .map(str::to_owned),
                ),
                None => code.push(text),
            }
        }
        let recipe = Recipe {
            deps,
            code,
            opens_with_guard,
        };
        found.insert(name, recipe);
    }
    found
}

/// The words after each bare `cargo` of `line`: the toolchain override dropped,
/// the quotes and brackets around each word trimmed.
fn cargo_words(line: &str) -> Vec<Vec<String>> {
    let line = line.replace("{{ ", "{{").replace(" }}", "}}");
    line.match_indices("cargo")
        .filter(|(at, _)| bare_word_at(&line, *at, "cargo"))
        .map(|(at, word)| {
            line[at + word.len()..]
                .split_whitespace()
                .skip_while(|word| word.starts_with('+'))
                .map(|word| word.trim_matches(|ch| "\"'();".contains(ch)).to_owned())
                .collect()
        })
        .collect()
}

/// Whether the cargo call with these words compiles or runs a package.
fn compiles(words: &[String], line: &str) -> bool {
    let first = words.first().map_or("", String::as_str);
    let second = words.get(1).map_or("", String::as_str);
    if second == "--version" {
        return false;
    }
    match first {
        "b" | "bench" | "build" | "c" | "check" | "clippy" | "d" | "doc" | "fix" | "llvm-cov"
        | "miri" | "mutants" | "package" | "publish" | "r" | "run" | "rustc" | "rustdoc" | "t"
        | "test" | "watch" => true,
        "nextest" => ["archive", "list", "r", "run"].contains(&second),
        "fuzz" => ["build", "check", "cmin", "coverage", "run", "tmin"].contains(&second),
        "install" => line.contains("--path"),
        _ => false,
    }
}

fn builds_itself(recipe: &Recipe) -> bool {
    recipe.code.iter().any(|line| {
        line.split_whitespace().next() == Some("bacon")
            || cargo_words(line).iter().any(|words| compiles(words, line))
    })
}

/// The recipes the body runs through `just`. A name inside quotes, as one line
/// opens them, is a message, not a call.
fn calls(recipe: &Recipe) -> Vec<String> {
    let mut found = Vec::new();
    for line in &recipe.code {
        for (at, word) in line.match_indices("just ") {
            let quoted = ['"', '\'']
                .iter()
                .any(|quote| line[..at].matches(*quote).count() % 2 == 1);
            if quoted || !bare_word_at(line, at, "just") {
                continue;
            }
            if let Some(name) = line[at + word.len()..].split_whitespace().next() {
                found.push(name.trim_matches(|ch| "\"');".contains(ch)).to_owned());
            }
        }
    }
    found
}

/// The recipes that compile or run the workspace, directly or through a recipe
/// that does.
fn building(recipes: &BTreeMap<String, Recipe>) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = recipes
        .iter()
        .filter(|(_, recipe)| builds_itself(recipe))
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let more: Vec<String> = recipes
            .iter()
            .filter(|(name, recipe)| {
                !found.contains(*name)
                    && recipe
                        .deps
                        .iter()
                        .cloned()
                        .chain(calls(recipe))
                        .any(|other| found.contains(&other))
            })
            .map(|(name, _)| name.clone())
            .collect();
        if more.is_empty() {
            return found;
        }
        found.extend(more);
    }
}

/// Whether `name` reaches the guard before any command that could build: its
/// first dependency is the guard or is itself covered (`just` runs dependencies
/// first, in order), or it has no dependency and either its body opens with the
/// guard's text, or it names no cargo at all and every building recipe it calls
/// is covered.
fn covered(
    recipes: &BTreeMap<String, Recipe>,
    built: &BTreeSet<String>,
    name: &str,
    depth: usize,
) -> bool {
    let Some(recipe) = recipes.get(name) else {
        return false;
    };
    if depth > recipes.len() {
        return false;
    }
    if let Some(first) = recipe.deps.first() {
        return first == GUARD || covered(recipes, built, first, depth + 1);
    }
    if recipe.opens_with_guard {
        return true;
    }
    let delegates: Vec<String> = calls(recipe)
        .into_iter()
        .filter(|other| built.contains(other))
        .collect();
    !delegates.is_empty()
        && recipe.code.iter().all(|line| cargo_words(line).is_empty())
        && delegates
            .iter()
            .all(|other| covered(recipes, built, other, depth + 1))
}

/// The building recipes that could build before they meet the guard.
fn uncovered(justfile: &str) -> Vec<String> {
    let all = recipes(justfile);
    let built = building(&all);
    built
        .iter()
        .filter(|name| !covered(&all, &built, name, 0))
        .cloned()
        .collect()
}

/// Whether `name` runs the guard anywhere: in its dependency tree or its body.
fn reaches_guard(recipes: &BTreeMap<String, Recipe>, name: &str) -> bool {
    name == GUARD
        || recipes.get(name).is_some_and(|recipe| {
            recipe.opens_with_guard
                || recipe
                    .deps
                    .iter()
                    .cloned()
                    .chain(calls(recipe))
                    .any(|other| other != name && reaches_guard(recipes, &other))
        })
}

#[test]
fn every_recipe_that_builds_the_workspace_refuses_on_the_macbook_first() {
    let justfile = read(&root().join("justfile"));
    let all = recipes(&justfile);
    let built = building(&all);
    let blind: Vec<&&str> = BUILDS
        .iter()
        .filter(|name| !built.contains(**name))
        .collect();
    assert!(
        blind.is_empty(),
        "the scan must see each recipe that builds today; it misses {blind:?} in {built:?}"
    );
    let open = uncovered(&justfile);
    assert!(
        open.is_empty(),
        "these recipes compile or run the workspace and can do so before `{GUARD}`: {open:?}. \
         Name `{GUARD}` as the first dependency, so a MacBook refuses before any cargo call"
    );
    for name in LIGHT {
        assert!(
            all.contains_key(name),
            "the justfile has no recipe `{name}`"
        );
        assert!(
            !built.contains(name) && !reaches_guard(&all, name),
            "`{name}` builds nothing and stays allowed on the MacBook"
        );
    }
}

#[test]
fn the_build_scan_sees_each_way_a_recipe_can_build() {
    let good = "FUZZ := \"nightly\"\n\
                LIST := '''\n    cargo nextest list --locked\n'''\n\
                test: rust-check\n\
                rust-check: _build-host rust-fmt-check rust-lint\n\
                rust-lint: _build-host\n    detached nice -n 10 cargo clippy --locked\n\
                rust-fmt-check:\n    cargo fmt --all --check\n\
                run:\n    exec just _lane test\n\
                _lane lane *args: _build-host\n    cargo nextest run --locked\n\
                list: _build-host\n    {{ LIST }}\n\
                fuzz target: _build-host _preflight\n    cargo +{{ FUZZ }} fuzz run \"$1\"\n\
                _preflight:\n    cargo fuzz --version\n    cargo +{{ FUZZ }} metadata --locked\n\
                linux: (_gate \"a b\" \"off\")\n\
                _gate flavor bus: _build-host\n    docker run x bash -c 'set -e\n    just test'\n\
                setup:\n    ensure cargo-mutants 'cargo mutants --version'\n    \
                cargo install --locked taplo-cli\n    echo \"ready — run: just test\"\n    \
                printf 'absent; just test cannot run\\n'\n\
                ship:\n    #!/usr/bin/env bash\n    set -euo pipefail\n    \
                {{ BUILD_HOST }}\n    cargo test --doc --locked\n";
    let all = recipes(good);
    assert_eq!(
        building(&all).into_iter().collect::<Vec<_>>(),
        [
            "_gate",
            "_lane",
            "fuzz",
            "linux",
            "list",
            "run",
            "rust-check",
            "rust-lint",
            "ship",
            "test"
        ],
        "the format check, the preflight probes and a quoted `just` build nothing"
    );
    assert_eq!(uncovered(good), Vec::<String>::new());
    assert!(reaches_guard(&all, "run") && !reaches_guard(&all, "setup"));
    assert!(reaches_guard(&all, "ship"), "the guard's text counts");

    // RED — one recipe without the guard, and everything that leans on it first.
    assert_eq!(
        uncovered(&good.replace("rust-lint: _build-host\n", "rust-lint:\n")),
        ["rust-lint"]
    );
    assert_eq!(
        uncovered(&good.replace("_lane lane *args: _build-host\n", "_lane lane *args:\n")),
        ["_lane", "run"]
    );
    // RED — the guard is there but runs second: the format check ran cargo first.
    assert_eq!(
        uncovered(&good.replace(
            "rust-check: _build-host rust-fmt-check",
            "rust-check: rust-fmt-check _build-host"
        )),
        ["rust-check", "test"]
    );
    assert_eq!(
        uncovered(&good.replace(": _build-host _preflight", ": _preflight _build-host")),
        ["fuzz"]
    );
    // RED — a delegator that calls cargo itself before the guarded recipe.
    assert_eq!(
        uncovered(&good.replace(
            "    exec just _lane test\n",
            "    cargo fmt\n    exec just _lane test\n"
        )),
        ["run"]
    );
    // RED — the guard's text is gone from a body, or a command precedes it.
    let opened = "    {{ BUILD_HOST }}\n    cargo test --doc --locked\n";
    for body in [
        "    cargo test --doc --locked\n",
        "    cargo test --doc --locked\n    {{ BUILD_HOST }}\n",
        "    cargo metadata --locked\n    {{ BUILD_HOST }}\n    cargo test --doc\n",
    ] {
        assert_eq!(uncovered(&good.replace(opened, body)), ["ship"], "{body}");
    }
    // RED — each spelling of a new recipe that builds.
    for body in [
        "cargo build --release --locked",
        "cargo run --locked --example affected",
        "cargo test --doc",
        "cargo +{{ FUZZ }} fuzz build",
        "cargo llvm-cov nextest",
        "cargo mutants --jobs 1",
        "cargo install --path .",
        "{{ LIST }}",
        "bacon clippy-all",
        "just run",
    ] {
        assert_eq!(
            uncovered(&format!("{good}probe:\n    cargo fmt\n    {body}\n")),
            ["probe"],
            "{body}"
        );
    }
}

// ── The behaviour ────────────────────────────────────────────────────

/// A scratch copy of the real justfile with a `bin` directory that holds the
/// linked `just`.
struct Bench {
    base: OwnedScratch,
    work: PathBuf,
    bin: PathBuf,
}

/// `(exit code, stdout, stderr)`; a signal is -1.
struct Ran {
    code: i32,
    out: String,
    err: String,
}

fn put_executable(path: &Path, body: &str) {
    std::fs::write(path, body)
        .and_then(|()| std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)))
        .unwrap_or_else(|why| panic!("{}: {why}", path.display()));
}

impl Bench {
    fn new(tag: &str) -> Self {
        let base = OwnedScratch::root("gate", tag);
        let (work, bin) = (base.join("work"), base.join("bin"));
        for dir in [&work, &bin] {
            std::fs::create_dir_all(dir).unwrap_or_else(|why| panic!("{}: {why}", dir.display()));
        }
        std::fs::write(work.join("justfile"), read(&root().join("justfile")))
            .unwrap_or_else(|why| panic!("the fixture justfile: {why}"));
        // No real git. These tests run real recipe bodies when a guard is
        // missing, and `release` must then reach no repository, not even one
        // that encloses the scratch root. It records nothing: the justfile's own
        // variables ask git on every `just` call.
        put_executable(&bin.join("git"), "#!/bin/sh\nexit 128\n");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let Some(just) = std::env::split_paths(&path)
            .map(|dir| dir.join("just"))
            .find(|file| file.is_file())
        else {
            panic!("just must be on PATH");
        };
        symlink(just, bin.join("just")).unwrap_or_else(|why| panic!("the linked just: {why}"));
        Self { base, work, bin }
    }

    /// A `pmset` that answers `-g batt`, and only that, with `text` and `status`.
    fn pmset(&self, text: &str, status: i32) {
        put_executable(
            &self.bin.join("pmset"),
            &format!(
                "#!/bin/sh\n[ \"$*\" = '-g batt' ] || exit 64\n\
                 cat <<'BATT'\n{text}\nBATT\nexit {status}\n"
            ),
        );
    }

    /// `just <words>` with nothing inherited. `system` is the PATH after `bin`.
    fn just(&self, words: &[&str], system: &str, human: Option<&str>) -> Ran {
        let mut run = Invocation::new("just")
            .env_cleared()
            .env("PATH", format!("{}:{system}", self.bin.display()))
            .env("HOME", &*self.base)
            .env("TMPDIR", &*self.base)
            .env("TMUX_TMPDIR", &*self.base)
            .env("AE_TEST_TMPDIR", &*self.base)
            .env("AE_WEB_SMOKE_CHROME", self.bin.join("chrome"))
            .env("CALLS", self.base.join("calls"));
        if let Some(value) = human {
            run = run.env("AE_ALLOW_MACBOOK_BUILD", value);
        }
        for word in words {
            run = run.arg(word);
        }
        let (out, err) = (self.base.join("out"), self.base.join("err"));
        let ran = raw::run(&run, &self.work, &out, &err)
            .unwrap_or_else(|why| panic!("just {words:?}: {why}"));
        Ran {
            code: match ran.outcome() {
                ExitOutcome::Code(code) => code,
                ExitOutcome::Signalled => -1,
            },
            out: read(&out),
            err: read(&err),
        }
    }
}

const SYSTEM: &str = "/usr/bin:/bin";

/// Whether `ran` is the guard's refusal: exit 1 and nothing but its one line.
fn refused(ran: &Ran) -> bool {
    ran.code == 1
        && ran.out.is_empty()
        && ran.err.lines().count() == 1
        && ran.err.starts_with(REFUSAL)
}

#[test]
fn the_guard_refuses_a_mac_with_an_internal_battery_and_nothing_else() {
    let bench = Bench::new("bhdetect");

    bench.pmset(MACBOOK, 0);
    let ran = bench.just(&[GUARD], SYSTEM, None);
    assert!(
        refused(&ran),
        "a MacBook is refused: {} {}",
        ran.code,
        ran.err
    );
    for word in ["the hp or m2", "AE_ALLOW_MACBOOK_BUILD=1", "human"] {
        assert!(
            ran.err.contains(word),
            "the line must say {word:?}: {}",
            ran.err
        );
    }
    // The override is the exact value 1 and nothing near it.
    for value in ["", "0", "true", "yes", "11"] {
        let ran = bench.just(&[GUARD], SYSTEM, Some(value));
        assert!(refused(&ran), "{value:?} does not override: {}", ran.err);
    }
    let ran = bench.just(&[GUARD], SYSTEM, Some("1"));
    assert_eq!(
        (ran.code, ran.out.as_str(), ran.err.as_str()),
        (0, "", ""),
        "the human override passes silently"
    );

    // ANY DOUBT DOES NOT REFUSE.
    for (what, text, status) in [
        ("m2, a Mac mini", MAC_MINI, 0),
        (
            "a Mac mini on a UPS",
            "Now drawing from 'UPS Power'\n -Back-UPS ES 700 (id=5)\t100%; charged; present: true",
            0,
        ),
        ("an empty answer", "", 0),
        (
            "an unreadable answer",
            "pmset: unable to read the power sources",
            0,
        ),
        ("a failed pmset", MACBOOK, 1),
    ] {
        bench.pmset(text, status);
        let ran = bench.just(&[GUARD], SYSTEM, None);
        assert_eq!(
            (ran.code, ran.out.as_str(), ran.err.as_str()),
            (0, "", ""),
            "{what} must not refuse"
        );
    }
    // No pmset at all, as on the hp and CI: `/bin` holds none.
    std::fs::remove_file(bench.bin.join("pmset"))
        .unwrap_or_else(|why| panic!("the pmset stub: {why}"));
    let ran = bench.just(&[GUARD], "/bin", None);
    assert_eq!(
        (ran.code, ran.err.as_str()),
        (0, ""),
        "a host without pmset must not refuse"
    );
}

#[test]
fn every_recipe_that_builds_refuses_before_it_drives_any_tool() {
    let bench = Bench::new("bhrecipes");
    for tool in DRIVEN {
        // `list-sessions` fails so that no cleanup waits for a server.
        put_executable(
            &bench.bin.join(tool),
            &format!(
                "#!/bin/sh\necho \"{tool} $*\" >>\"$CALLS\"\n\
                 case \"$*\" in *list-sessions*) exit 1 ;; esac\nexit 0\n"
            ),
        );
    }
    let calls = || std::fs::read_to_string(bench.base.join("calls")).unwrap_or_default();
    bench.pmset(MACBOOK, 0);
    let justfile = read(&bench.work.join("justfile"));
    let mut open = Vec::new();
    for name in building(&recipes(&justfile)) {
        let mut words = vec![name.as_str()];
        if let Some((_, arguments)) = ARGUMENTS.iter().find(|(recipe, _)| *recipe == name) {
            words.extend_from_slice(arguments);
        }
        let ran = bench.just(&words, SYSTEM, None);
        // A recipe that delegates may add `just`'s own lines after the refusal,
        // and one that pipes its delegate to a log shows the refusal on stdout.
        let mut said = ran.err.lines().chain(ran.out.lines());
        if ran.code == 0 || !said.any(|line| line.starts_with(REFUSAL)) {
            open.push(format!(
                "{name}: exit {} {:?} {:?}",
                ran.code, ran.err, ran.out
            ));
        }
        let ran_tools = calls();
        if !ran_tools.is_empty() {
            open.push(format!(
                "{name} drove a tool before it refused: {ran_tools:?}"
            ));
            let _ = std::fs::remove_file(bench.base.join("calls"));
        }
    }
    assert!(open.is_empty(), "{open:#?}");

    // The stubs do record: the same recipe builds for the human override and on a
    // Mac mini.
    for (battery, human) in [(MACBOOK, Some("1")), (MAC_MINI, None)] {
        bench.pmset(battery, 0);
        let ran = bench.just(&["rust-build-release"], SYSTEM, human);
        assert!(
            !ran.err.contains(REFUSAL),
            "no refusal here ({human:?}): {}",
            ran.err
        );
        assert_eq!(calls(), "cargo build --release --locked\n", "{}", ran.err);
        let _ = std::fs::remove_file(bench.base.join("calls"));
    }
}
