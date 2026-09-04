//! The public entry, black-box: the whole surface `ae-glue`'s `case` statement
//! used to answer, and since slice Z3 the whole surface `ae-entry` answered too.
//!
//! The subject is the real binary, because the subject IS what a human typed at
//! `ae`: which environment DOOR supplies which fact, which word routes where,
//! what a cut word does instead of becoming a session name, and what a first run
//! leaves behind.
//!
//! THERE IS NO PREAMBLE TO SPELL ANY MORE. Every flag the wrapper used to hand
//! over is an environment variable this rig sets, or the working directory it
//! runs in — which is exactly the contract the product now has with a shell.
//!
//! ONE DOOR IS NOT BLACK-BOXED HERE, and the reason is that it has no black-box
//! surface: `TMUX_PANE` only changes an answer when it names a pane that is a
//! REAL ae agent's on the resolved server, and every refusal short of that is
//! the same sentence with the door set or unset (measured). Planting one costs
//! a full launch with an agent in it, which is the bash suite's job. What is
//! pinned here instead is the half that can be: the router appends `--pane` for
//! `stop` and `watchdog` and only when the caller named none
//! (`src/entry.rs`'s `the_pane_is_appended_only_when_the_caller_named_none`),
//! and the read itself is one documented door in `src/doors.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::cli::ae;
use super::phase2::{run_tmux, tmux_present};

/// An isolated ae home and a project directory. No tmux server unless a test
/// asks for one — most of this suite never needs to reach that far.
struct Rig {
    scratch: PathBuf,
    home: PathBuf,
    project: PathBuf,
    sock: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aeentry.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let home = scratch.join("aehome");
        let project = scratch.join("project");
        assert!(std::fs::create_dir_all(&project).is_ok(), "a project dir");
        Self {
            sock: scratch.join("sock"),
            scratch,
            home,
            project,
        }
    }

    fn config(&self) -> PathBuf {
        self.home.join("config")
    }

    fn sessions(&self) -> PathBuf {
        self.home.join("sessions")
    }

    /// Run the product as a shell would: this rig's doors in the environment,
    /// its project as the working directory, and `argv` verbatim.
    ///
    /// The CHECKOUT shape, which is what a test binary under `target/` is: it
    /// honours `AE_HOME`, `CONFIG_FILE` and the server pair, so the rig can
    /// isolate every one of them.
    fn run(&self, argv: &[&str]) -> (Option<i32>, String, String) {
        self.run_on(None, argv)
    }

    /// The same, with the tmux server pair declared — the door the launch needs
    /// so it lands on this rig's own server and not the developer's.
    fn run_on(&self, server: Option<&Path>, argv: &[&str]) -> (Option<i32>, String, String) {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.config())
            .env("AE_NO_AUTOSTART", "1")
            .env("TMUX_TMPDIR", &self.scratch)
            .current_dir(&self.project);
        if let Some(socket) = server {
            command
                .env("AE_TMUX_SERVER_KIND", "socket")
                .env("AE_TMUX_SERVER", socket);
        } else {
            command
                .env_remove("AE_TMUX_SERVER_KIND")
                .env_remove("AE_TMUX_SERVER");
        }
        let out = command
            .args(argv)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        for _ in 0..40 {
            let _ = std::fs::remove_dir_all(&self.scratch);
            if !self.scratch.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

fn skip() -> bool {
    let probe = PathBuf::from(format!("/tmp/aeentry-probe.{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe);
    let present = tmux_present(&probe);
    let _ = std::fs::remove_dir_all(&probe);
    !present
}

// ---------------------------------------------------------------------------
// (1) the preamble parse
// ---------------------------------------------------------------------------

/// The entry's contract, end to end: the facts come from the environment, the
/// argv is the caller's alone, and an entry still answers.
#[test]
fn the_doors_carry_the_facts_and_the_argv_is_the_users_alone() {
    let rig = Rig::new("carry");
    let (code, stdout, stderr) = rig.run(&["version"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{}\n", ae::version_line()));
    // The doors alone write NOTHING: only a launch seeds a config.
    assert!(!rig.config().exists(), "a version query created state");
}

/// `version` answers AHEAD of every gate, including the doors themselves.
///
/// It is how a broken install is diagnosed, so it may not depend on anything an
/// install can break: not the state root, not a config, not tmux.
#[test]
fn version_answers_with_no_environment_at_all() {
    for word in ["version", "--version", "-V"] {
        let out = ae()
            .env_clear()
            .arg(word)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        assert_eq!(out.status.code(), Some(0), "{word}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{}\n", ae::version_line()),
            "{word}"
        );
        assert!(out.stderr.is_empty(), "{word}");
    }
}

/// THE `AE_HOME` DOOR relocates every piece of state, and `CONFIG_FILE` names
/// the global config independently of it. Both are CHECKOUT-shape doors.
///
/// The two are asserted through different surfaces because they REACH
/// different ones: `AE_HOME` is the state root every command derives from, so
/// `doctor` shows it; `CONFIG_FILE` is the global config a LAUNCH reads and
/// seeds, and doctor reports the default beside its own root rather than the
/// launch's file (a gap that predates this slice — nothing appends `--global`
/// to doctor's argv, and nothing did through the wrapper either).
#[test]
fn the_home_and_config_doors_are_honoured_by_a_checkout_build() {
    let rig = Rig::new("homedoor");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    let (_, stdout, stderr) = rig.run(&["doctor"]);
    let report = format!("{stdout}{stderr}");
    assert!(
        report.contains(&rig.sessions().display().to_string()),
        "AE_HOME names the state root: {report}"
    );

    if skip() {
        return;
    }
    // CONFIG_FILE, through the surface that reads it: a first launch SEEDS the
    // global config, and it seeds the file this door names rather than the
    // default beside the home.
    let elsewhere = rig.scratch.join("elsewhere.config");
    let sock = rig.sock.clone();
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", &elsewhere)
        .env("AE_NO_AUTOSTART", "1")
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &sock)
        .current_dir(&rig.project);
    let out = command
        .args(["--local", "cfgdoor"])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert!(
        elsewhere.exists(),
        "the launch seeded the config CONFIG_FILE named: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !rig.config().exists(),
        "and never the default beside the home"
    );
}

/// A MACHINE THAT CANNOT SAY WHERE ITS STATE LIVES is refused before anything —
/// the one door with no default behind it.
#[test]
fn no_home_and_no_ae_home_is_the_one_refusal_the_doors_can_make() {
    let out = ae()
        .env_clear()
        .arg("list")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert_eq!(out.status.code(), Some(1), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "a refusal must not reach stdout");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(ae::NO_STATE_ROOT),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// THE FROZEN PREAMBLE IS GONE, and its flags are ordinary argv now.
///
/// This is the one shape that must NOT be quietly tolerated: `--home /x` used
/// to be a fact the wrapper spoke, and a compat arm accepting it would leave
/// two ways to say where ae's state lives — the exact second answer slice Z1
/// removed from bash and slice Z3 removes from the flag surface.
#[test]
fn a_preamble_flag_is_no_longer_a_flag_ae_answers_to() {
    let rig = Rig::new("nopreamble");
    let (code, stdout, stderr) = rig.run(&["--home", "/x", "--cwd", "/y", "--", "list"]);
    assert_ne!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        !stderr.contains("Usage: ae-core --home"),
        "the preamble usage line is gone with the parse: {stderr}"
    );
    assert!(
        stdout.is_empty(),
        "a refusal must not reach stdout: {stdout}"
    );
    assert!(!rig.sessions().exists(), "a refused argv built state");
}

/// NOTHING CHANGES FOR AN INTERNAL ENTRY CALLED BARE. Every session helper is a
/// link to this binary and reaches the core with no ambient fact at all, so the
/// entry grammar must keep parsing exactly as it did.
#[test]
fn an_internal_entry_keeps_its_own_grammar_and_pays_for_no_door() {
    let rig = Rig::new("internal");
    let dir = rig.sessions().join("s1");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(dir.join("meta"), "session=s1\n").is_ok(),
        "meta"
    );
    let out = ae()
        .env_clear()
        .arg("_requests")
        .arg(&dir)
        .arg("all")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    // env_clear is the assertion: an internal entry carries its own operands,
    // so it must answer with no HOME, no AE_HOME and no PATH to find tmux on.
    assert_eq!(
        out.status.code(),
        Some(0),
        "an internal entry must not need a door: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An unserved `_` word FAILS CLOSED rather than becoming a session named after
/// ae's own namespace.
#[test]
fn an_unserved_internal_word_is_refused_by_name() {
    let rig = Rig::new("unserved");
    let (code, stdout, stderr) = rig.run(&["_recover-pending"]);
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("unknown internal command"), "{stderr}");
    assert!(stderr.contains("_recover-pending"), "{stderr}");
    assert!(!rig.sessions().exists(), "a refused word built state");
}

/// THE SERVER PAIR IS READ BY *SET*, NOT BY NONEMPTY, and a pair that cannot be
/// typed is a refusal rather than a fallback.
///
/// `AE_TMUX_SERVER_KIND=ambiguous AE_TMUX_SERVER=` is exactly the shape the
/// socket probe mints for a relative path it could not prove. A nonempty test
/// read that set-empty half as an absent one: the pair was dropped, the AMBIENT
/// server was resolved, and a launch landed on a server nobody asked for — the
/// one outcome `ambiguous` exists to prevent. This is the whole rule in one
/// invocation, and it needs no tmux to state it.
#[test]
fn an_untypeable_server_pair_refuses_and_never_falls_back() {
    let rig = Rig::new("ambiguous");
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.config())
        .env("AE_TMUX_SERVER_KIND", "ambiguous")
        .env("AE_TMUX_SERVER", "")
        .current_dir(&rig.project);
    let out = command
        .args(["--local", "nowhere"])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("not a tmux server kind"), "{stderr}");
    assert!(
        stderr.contains("will not fall back to the ambient server"),
        "{stderr}"
    );
    assert!(
        !rig.sessions().join("nowhere").exists(),
        "a refused pair built state"
    );
}

// ---------------------------------------------------------------------------
// (2) the launch fall-through
// ---------------------------------------------------------------------------

/// The whole point of the slice: a bare word reaches `_launch` with the
/// preamble's facts and builds a real session.
#[test]
fn a_launch_candidate_becomes_a_session_from_the_preamble_facts() {
    if skip() {
        return;
    }
    let rig = Rig::new("launch");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok(),
        "a config"
    );
    let sock = rig.sock.clone();
    let (code, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "entryone"]);
    // The ATTACH is what decides the code, and it cannot succeed here: `ae
    // <name>` always attaches — the wrapper passed `--attach` unconditionally
    // and there is no door that says otherwise — and a test process has no
    // terminal to hand to tmux. The session is what this row is about, and it
    // is built before the attach is even attempted.
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    assert!(
        rig.sessions().join("entryone").join("meta").exists(),
        "{stdout}"
    );
    let (ok, listed) = rig.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(
        ok && listed.lines().any(|line| line == "entryone"),
        "{listed}"
    );
}

/// An EMPTY argv after the preamble is a LAUNCH, not the help an empty argv
/// gets everywhere else — `ae` with no words starts the default session and
/// always has.
#[test]
fn no_argv_at_all_launches_rather_than_printing_help() {
    if skip() {
        return;
    }
    let rig = Rig::new("bare");
    let sock = rig.sock.clone();
    let (code, stdout, stderr) = rig.run_on(Some(&sock), &[]);
    assert_ne!(code, Some(2), "not a usage error: {stdout}\n{stderr}");
    // Help would have printed the command list and written nothing. This ran
    // the launch prelude and DERIVED a name from the WORKING DIRECTORY, which
    // is the whole difference between `ae` and the core's own empty argv.
    assert!(!stdout.contains("Usage:"), "help was printed: {stdout}");
    assert!(rig.config().exists(), "the launch prelude did not run");
    let started: Vec<String> = std::fs::read_dir(rig.sessions())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        // The lifecycle lock is a sibling FILE in the same root, not a session.
        .filter(|name| !name.starts_with(".lifecycle."))
        .collect();
    assert_eq!(started.len(), 1, "one derived session: {started:?}");
    assert!(
        started[0].contains("project"),
        "the name is derived from the cwd door: {started:?}"
    );
}

// ---------------------------------------------------------------------------
// (3) the refusals
// ---------------------------------------------------------------------------

/// A CUT WORD CREATES NO SESSION. Everything the router does not recognise
/// falls through to a launch, and a launch takes the last positional as a
/// session name — so a deleted arm does not error, it quietly creates a session
/// named after the word.
#[test]
fn a_cut_word_refuses_and_creates_no_session() {
    let rig = Rig::new("cut");
    for (word, expected) in [
        ("status", "Use 'ae list'"),
        ("orchestrator", "Run it as an ordinary session"),
        ("hub", "Run it as an ordinary session"),
        ("transfer", "no cross-machine session sync"),
    ] {
        let (code, stdout, stderr) = rig.run(&[word]);
        assert_eq!(code, Some(2), "'{word}': {stdout}{stderr}");
        assert!(stderr.starts_with("Error: "), "'{word}': {stderr}");
        assert!(stderr.contains(expected), "'{word}': {stderr}");
        assert!(stdout.is_empty(), "'{word}' printed to stdout: {stdout}");
        assert!(
            !rig.sessions().join(word).exists(),
            "'{word}' created a session directory"
        );
        // And the prelude never ran: a refusal writes nothing on its way out.
        assert!(!rig.config().exists(), "'{word}' seeded a config");
    }
}

/// An underscore word nobody serves fails CLOSED for the same reason.
#[test]
fn an_unserved_internal_word_refuses_rather_than_launching() {
    let rig = Rig::new("internal");
    for word in ["_recover-pending", "_stop-supervisor", "_nope"] {
        let (code, stdout, stderr) = rig.run(&[word, "whatever"]);
        assert_eq!(code, Some(2), "'{word}': {stdout}{stderr}");
        assert_eq!(stderr, format!("ae: unknown internal command '{word}'.\n"));
        assert!(
            !rig.sessions().join("whatever").exists(),
            "'{word}' launched"
        );
    }
}

// ---------------------------------------------------------------------------
// (4) help, rendered by the core
// ---------------------------------------------------------------------------

/// `ae help` is the core's, and it is the COMMAND SET: every word the router
/// answers appears, and no word it refuses does.
#[test]
fn help_is_the_command_set_and_names_no_retired_word() {
    let rig = Rig::new("help");
    for spelling in ["help", "-h", "--help"] {
        let (code, stdout, stderr) = rig.run(&[spelling]);
        assert_eq!(code, Some(0), "'{spelling}': {stderr}");
        assert!(stderr.is_empty(), "'{spelling}': {stderr}");
        assert_eq!(stdout, ae::entry::HELP);
    }
    for row in [
        "  ae list [",
        "  ae next [",
        "  ae doctor [",
        "  ae rename ",
        "  ae watchdog ",
        "  ae telegram ",
        "  ae stop [",
        "  ae compact [",
        "  ae archive preview ",
        "  ae end|rm [",
        "  ae version",
        "  ae help",
    ] {
        assert!(ae::entry::HELP.contains(row), "help is missing {row}");
    }
    for retired in ["ae status", "ae orchestrator", "ae transfer"] {
        assert!(
            !ae::entry::HELP.contains(retired),
            "help still advertises the retired {retired}"
        );
    }
}

/// `ae list --help` is the core's too — on STDERR and exit 0, as it was.
///
/// It is routed OUT of the flag parser deliberately: the core parses `--help`
/// only at top level and would answer a `list` tail with a usage error.
#[test]
fn list_help_is_the_ratified_filter_text_on_stderr() {
    let rig = Rig::new("listhelp");
    for spelling in ["--help", "-h"] {
        for word in ["list", "ls"] {
            let (code, stdout, stderr) = rig.run(&[word, spelling]);
            assert_eq!(code, Some(0), "'{word} {spelling}': {stderr}");
            assert!(stdout.is_empty(), "'{word} {spelling}': {stdout}");
            assert_eq!(stderr, ae::entry::LIST_HELP);
        }
    }
    // Any other tail is still the core's to parse, unknown flags included.
    let (code, _, stderr) = rig.run(&["list", "--nope"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("unknown argument: --nope"), "{stderr}");
}

// ---------------------------------------------------------------------------
// (5) the default config
// ---------------------------------------------------------------------------

/// THE ORDER IS THE CONTRACT: the config is the first write of the run, and it
/// happens only after the dependency gate has passed.
///
/// An install that cannot serve a launch must not leave a config behind, or a
/// diagnosis starts from a home the failing run invented.
#[test]
fn the_first_run_seeds_the_config_only_after_the_dependency_check() {
    let rig = Rig::new("seed");
    // A machine with NO TMUX. That is the whole dependency gate now — the bash
    // row went with `ae-entry`, which is the interpreter it was about — and it
    // is still the thing that must refuse before the first write.
    let out = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.config())
        .env("PATH", "/nonexistent")
        .current_dir(&rig.project)
        .arg("seedme")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("tmux"), "{stderr}");
    assert!(
        !rig.config().exists(),
        "the config was seeded before the gate refused"
    );

    // The same launch with a bash the gate accepts writes it, and says so on
    // STDERR — the launch's stdout belongs to the session it is about to become.
    let (_, stdout, stderr) = rig.run(&["seedme"]);
    assert!(rig.config().exists(), "{stderr}");
    assert!(
        stderr.contains(&format!(
            "Created default config at {}",
            rig.config().display()
        )),
        "{stderr}"
    );
    assert!(!stdout.contains("Created default config"), "{stdout}");

    let written = std::fs::read_to_string(rig.config()).unwrap_or_default();
    assert_eq!(written, ae::entry::DEFAULT_CONFIG);
    for section in ["[profiles]", "[roster]", "[workspace]", "[prompt]"] {
        assert!(written.contains(section), "the template lost {section}");
    }
    // No temp file is left beside it.
    let leftovers: Vec<PathBuf> = std::fs::read_dir(&rig.home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().contains(".tmp."))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp files left behind: {leftovers:?}"
    );
}

/// A config that is already there is never rewritten — the seeding is a first
/// run's, not every run's.
#[test]
fn an_existing_config_is_left_exactly_as_it_was() {
    let rig = Rig::new("keepconfig");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(std::fs::write(rig.config(), "# mine\n").is_ok(), "a config");
    let (_, _, stderr) = rig.run(&["keepme"]);
    assert!(!stderr.contains("Created default config"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(rig.config()).unwrap_or_default(),
        "# mine\n"
    );
}

// ---------------------------------------------------------------------------
// the path-object guard
// ---------------------------------------------------------------------------

/// A SYMLINK NAMED `valid-name` satisfies every naming rule.
///
/// Both reuse paths and the rollback's own removal would then run THROUGH the
/// link, out of the sessions root — so the path object is checked before any
/// side effect, and independently of the grammar the name already passed.
#[test]
fn a_symlinked_session_directory_is_refused_before_anything_happens() {
    let rig = Rig::new("symlink");
    let victim = rig.scratch.join("victim");
    assert!(std::fs::create_dir_all(&victim).is_ok(), "a victim dir");
    assert!(
        std::fs::create_dir_all(rig.sessions()).is_ok(),
        "a sessions root"
    );
    let link = rig.sessions().join("linked");
    assert!(
        std::os::unix::fs::symlink(&victim, &link).is_ok(),
        "a symlinked session dir"
    );
    let (code, stdout, stderr) = rig.run(&["linked"]);
    assert_eq!(code, Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("is not a plain directory"), "{stderr}");
    assert!(stderr.contains("escape wearing a valid name"), "{stderr}");
    assert!(victim.exists(), "the link target was touched");
    assert!(
        std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()),
        "the link itself was replaced"
    );
}

/// A DANGLING link is the case an existence test misses: `-e` reads it as
/// absent and waves it through, so the guard is an lstat.
#[test]
fn a_dangling_symlink_is_refused_too() {
    let rig = Rig::new("dangling");
    assert!(
        std::fs::create_dir_all(rig.sessions()).is_ok(),
        "a sessions root"
    );
    let link = rig.sessions().join("gone");
    assert!(
        std::os::unix::fs::symlink(rig.scratch.join("nothing-here"), &link).is_ok(),
        "a dangling session dir"
    );
    let (code, _, stderr) = rig.run(&["gone"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("is not a plain directory"), "{stderr}");
}

/// A name the grammar itself refuses is refused AS A NAME, by the launch, so
/// the message says what is actually wrong rather than blaming the path.
#[test]
fn a_name_the_grammar_refuses_is_the_launchs_refusal_not_the_paths() {
    let rig = Rig::new("badname");
    let (code, _, stderr) = rig.run(&["../victim"]);
    assert_ne!(code, Some(0), "{stderr}");
    assert!(!stderr.contains("is not a plain directory"), "{stderr}");
    assert!(stderr.contains("invalid session name"), "{stderr}");
}

// ---------------------------------------------------------------------------
// the translated words
// ---------------------------------------------------------------------------

/// The human words reach the core entries behind them, environmental facts
/// appended. Each of these is proven by the entry's OWN refusal arriving —
/// which only happens if the translation ran.
#[test]
fn the_human_words_reach_the_core_entries_behind_them() {
    let rig = Rig::new("words");
    let cases: [(&[&str], &str); 5] = [
        (&["end"], "Usage"),
        (&["stop"], "Usage"),
        (&["rename"], "Usage"),
        (&["watchdog", "nope"], "Usage"),
        (&["archive", "nope"], "Usage: ae archive preview"),
    ];
    for (argv, expected) in cases {
        let (code, stdout, stderr) = rig.run(argv);
        assert_ne!(code, Some(0), "{argv:?}: {stdout}{stderr}");
        assert!(
            stderr.contains(expected),
            "{argv:?} did not reach its entry: {stderr}"
        );
    }
}

/// `ae doctor` is told which bash ran the wrapper — the one fact the core
/// cannot see, since probing `bash --version` would report whatever is first on
/// PATH rather than the interpreter ae re-exec'd into.
#[test]
fn doctor_names_the_binary_answering_and_no_interpreter() {
    let rig = Rig::new("doctor");
    let (_, stdout, stderr) = rig.run(&["doctor"]);
    let report = format!("{stdout}{stderr}");
    // The row that replaced it: WHICH core answered. A checkout build is
    // writable by construction, so the deviation is not claimed here — that
    // warning belongs to a published version directory, and `shape` decides
    // which of the two this is.
    assert!(report.contains("core "), "no core row:\n{report}");
    assert!(
        !report.contains("bash "),
        "ae ships no interpreter, so no row may report one:\n{report}"
    );
}

/// `ae archive preview` with a name that has no state says so, and never
/// reaches the tracer.
#[test]
fn archive_preview_refuses_a_session_with_no_state() {
    let rig = Rig::new("preview");
    let (code, stdout, stderr) = rig.run(&["archive", "preview", "ghost"]);
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("no session state for 'ghost'"), "{stderr}");
}

/// And with no name at all, outside a session, it says how to name one.
#[test]
fn archive_preview_outside_a_session_asks_for_a_name() {
    let rig = Rig::new("previewbare");
    let (code, _, stderr) = rig.run(&["archive", "preview"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("Usage: ae archive preview"), "{stderr}");
    assert!(stderr.contains("Run inside an ae tmux session"), "{stderr}");
}

/// The sessions root is where these names resolve — a rig helper the tests
/// above lean on, asserted once so a wrong root cannot make them vacuous.
#[test]
fn the_sessions_root_is_derived_from_the_preamble_home() {
    let rig = Rig::new("root");
    assert_eq!(rig.sessions(), Path::new(&rig.home).join("sessions"));
}

// ---------------------------------------------------------------------------
// (5) the remaining doors, each through the surface that consumes it
// ---------------------------------------------------------------------------

/// The SERVER PAIR decides which tmux server a launch lands on, and the session
/// records the one it was handed rather than asking tmux afterwards.
#[test]
fn the_server_pair_door_decides_where_a_launch_lands() {
    if skip() {
        return;
    }
    let rig = Rig::new("pairdoor");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = false\n",
        )
        .is_ok(),
        "a config"
    );
    let sock = rig.sock.clone();
    let (_, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "pairone"]);
    let meta = rig.sessions().join("pairone").join("meta");
    let Ok(text) = std::fs::read_to_string(&meta) else {
        panic!("a meta at {}: {stdout}{stderr}", meta.display());
    };
    assert!(
        text.contains(&format!("tmux_server={}", sock.display())),
        "the pair the door declared is the pair the session records: {text}"
    );
    assert!(text.contains("tmux_server_kind=socket"), "{text}");
}

/// The CWD door: a launch with no name derives one from the working directory,
/// and `$PWD` is honoured only when it names the same directory.
///
/// `$PWD` is the LOGICAL spelling — what keeps `/tmp` from becoming
/// `/private/tmp` on macOS — but it is an ordinary variable a program that
/// `chdir`s without updating it leaves stale. A lying one must not decide where
/// ae thinks it is.
#[test]
fn the_cwd_door_prefers_the_logical_pwd_only_when_it_agrees() {
    if skip() {
        return;
    }
    let rig = Rig::new("cwddoor");
    let sock = rig.sock.clone();
    let mut command = ae();
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_HOME", &rig.home)
        .env("CONFIG_FILE", rig.config())
        .env("AE_NO_AUTOSTART", "1")
        .env("TMUX_TMPDIR", &rig.scratch)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &sock)
        // A PWD that names a real directory this process is NOT in.
        .env("PWD", "/")
        .current_dir(&rig.project);
    let out = command
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    let started: Vec<String> = std::fs::read_dir(rig.sessions())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(".lifecycle."))
        .collect();
    assert_eq!(
        started.len(),
        1,
        "one derived session: {started:?} ({})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        started[0].contains("project"),
        "the stale PWD did not decide where ae thinks it is: {started:?}"
    );
}

/// The `AE_NO_AUTOSTART` door: `=1` starts NEITHER companion, however the config
/// asks.
#[test]
fn the_no_autostart_door_starts_neither_companion() {
    if skip() {
        return;
    }
    let rig = Rig::new("autostart");
    assert!(std::fs::create_dir_all(&rig.home).is_ok(), "an ae home");
    assert!(
        std::fs::write(
            rig.config(),
            "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\n\n\
             [workspace]\nmain = lead\nlayout = vertical\nwatchdog = true\n",
        )
        .is_ok(),
        "a config that ASKS for the watchdog"
    );
    let sock = rig.sock.clone();
    // `run_on` sets AE_NO_AUTOSTART=1.
    let (_, stdout, stderr) = rig.run_on(Some(&sock), &["--local", "quiet"]);
    assert!(
        rig.sessions().join("quiet").join("meta").exists(),
        "the session was built: {stdout}{stderr}"
    );
    let (ok, windows) = rig.tmux(&["list-windows", "-t", "quiet", "-F", "#{window_name}"]);
    assert!(ok, "{windows}");
    assert!(
        !windows.lines().any(|line| line.contains("watchdog")),
        "the door suppressed the companion the config asked for: {windows}"
    );
}
