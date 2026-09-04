//! The PREAMBLE entry, black-box: the whole surface `ae-glue`'s `case`
//! statement used to answer.
//!
//! The subject is the real binary, because the subject IS argv handling: what
//! the wrapper hands over, which word routes where, what a cut word does
//! instead of becoming a session name, and what a first run leaves behind.

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

    /// The preamble this rig implies, as the wrapper would spell it.
    fn preamble(&self) -> Vec<String> {
        vec![
            "--home".to_owned(),
            self.home.display().to_string(),
            "--cwd".to_owned(),
            self.project.display().to_string(),
            "--global".to_owned(),
            self.config().display().to_string(),
            "--bash-major".to_owned(),
            "5".to_owned(),
            "--no-attach".to_owned(),
            "--no-autostart".to_owned(),
        ]
    }

    /// Run the product with this rig's preamble, then `--`, then `argv`.
    fn run(&self, argv: &[&str]) -> (Option<i32>, String, String) {
        self.run_with(&self.preamble(), argv)
    }

    /// The same, with an arbitrary preamble — what the parse arms need.
    fn run_with(&self, preamble: &[String], argv: &[&str]) -> (Option<i32>, String, String) {
        let out = ae()
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_HOME", &self.home)
            .env("TMUX_TMPDIR", &self.scratch)
            .args(preamble)
            .arg("--")
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

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

// ---------------------------------------------------------------------------
// (1) the preamble parse
// ---------------------------------------------------------------------------

/// The wrapper's contract, end to end: the facts before `--`, the user's argv
/// after it, and an entry that still answers.
#[test]
fn the_preamble_carries_the_wrappers_facts_and_the_user_argv_follows_it() {
    let rig = Rig::new("carry");
    let (code, stdout, stderr) = rig.run(&["version"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{}\n", ae::version_line()));
    // The preamble alone writes NOTHING: only a launch seeds a config.
    assert!(!rig.config().exists(), "a version query created state");
}

/// A preamble that is present and wrong is a usage error, never a launch.
///
/// The wrapper is the only caller, so a malformed one is a broken install —
/// answering it with a session named after one of its words would hide exactly
/// the failure that needs to be loud.
#[test]
fn a_missing_home_is_a_usage_error() {
    let rig = Rig::new("nohome");
    let (code, stdout, stderr) = rig.run_with(
        &words(&["--cwd", &rig.project.display().to_string()]),
        &["list"],
    );
    assert_eq!(code, Some(2), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("--home and --cwd are required"), "{stderr}");
    assert!(stderr.contains("Usage: ae-core --home"), "{stderr}");
    assert!(
        stdout.is_empty(),
        "a refusal must not reach stdout: {stdout}"
    );
    assert!(!rig.sessions().exists(), "a refused preamble built state");
}

/// The same for the other half of the pair, and for a flag with no value.
#[test]
fn a_missing_cwd_or_a_dangling_flag_is_a_usage_error() {
    let rig = Rig::new("nocwd");
    let (code, _, stderr) = rig.run_with(
        &words(&["--home", &rig.home.display().to_string()]),
        &["list"],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--home and --cwd are required"), "{stderr}");

    let out = ae()
        .env_remove("TMUX")
        .args(["--home"])
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("offending word: --home"), "{text}");
}

/// An unknown flag INSIDE the preamble is refused with the word that caused it.
#[test]
fn an_unknown_preamble_flag_names_itself() {
    let rig = Rig::new("unknownflag");
    let mut preamble = rig.preamble();
    preamble.push("--frobnicate".to_owned());
    preamble.push("x".to_owned());
    let (code, _, stderr) = rig.run_with(&preamble, &["list"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("offending word: --frobnicate"), "{stderr}");
}

/// NOTHING CHANGES FOR AN ENTRY CALLED BARE. Every session helper shim execs
/// the core directly, with no preamble at all, and must keep parsing exactly as
/// it did.
#[test]
fn an_entry_invoked_without_a_preamble_keeps_its_own_grammar() {
    let rig = Rig::new("nopreamble");
    let dir = rig.sessions().join("s1");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
    assert!(
        std::fs::write(dir.join("meta"), "session=s1\n").is_ok(),
        "meta"
    );
    let out = ae()
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .arg("_requests")
        .arg(&dir)
        .arg("all")
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert_ne!(
        out.status.code(),
        Some(2),
        "a bare helper entry must not be read as a preamble: {}",
        String::from_utf8_lossy(&out.stderr)
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
    let mut preamble = rig.preamble();
    preamble.extend(words(&[
        "--server-kind",
        "socket",
        "--server",
        &rig.sock.display().to_string(),
    ]));
    let (code, stdout, stderr) = rig.run_with(&preamble, &["--local", "entryone"]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
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
    let mut preamble = rig.preamble();
    preamble.extend(words(&[
        "--server-kind",
        "socket",
        "--server",
        &rig.sock.display().to_string(),
    ]));
    let (code, stdout, stderr) = rig.run_with(&preamble, &[]);
    assert_eq!(code, Some(0), "stdout: {stdout}\nstderr: {stderr}");
    // Help would have printed the command list and written nothing. This ran
    // the launch prelude and DERIVED a name from the preamble's cwd, which is
    // the whole difference the preamble makes to an empty argv.
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
        "the name is derived from the preamble cwd: {started:?}"
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
    // A bash the gate REFUSES. Everything else about the invocation is a launch.
    let mut refused = rig.preamble();
    for word in &mut refused {
        if word == "5" {
            "3".clone_into(word);
        }
    }
    let (code, _, stderr) = rig.run_with(&refused, &["seedme"]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("ae requires bash >= 4.0"), "{stderr}");
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
fn doctor_reports_the_bash_the_wrapper_named() {
    let rig = Rig::new("doctor");
    let (_, stdout, stderr) = rig.run(&["doctor"]);
    assert!(
        format!("{stdout}{stderr}").contains("bash 5"),
        "the preamble's --bash-major did not reach doctor:\n{stdout}{stderr}"
    );
    // And a different value reaches it as that value, so the row is the
    // wrapper's answer rather than anything the core measured for itself.
    let mut older = rig.preamble();
    for word in &mut older {
        if word == "5" {
            "3".clone_into(word);
        }
    }
    let (_, stdout, stderr) = rig.run_with(&older, &["doctor"]);
    assert!(
        format!("{stdout}{stderr}").contains("bash 3 (ae needs bash >= 4)"),
        "{stdout}{stderr}"
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
