//! Phase-1 characterisation of `lifecycle::end::run` at 441e1662.
//!
//! Oracle: documented per-target acknowledgement, history flag precedence,
//! caller targeting and capture-before-delete guarantees. Exact diagnostics,
//! last mode wins and refusal order are BASELINE BEHAVIOUR pins. These tests
//! deliberately preserve oddities; they do not prescribe a behaviour fix.
//! Every filesystem and tmux effect uses the parent Rig/OwnedScratch fixtures.

use super::*;

const USAGE: &str =
    "Usage: _end [-f] [--purge-history|--keep-history] [--assume-stopped] <session-name|all>\n";
const SUPERVISOR: &str = "Error: end supervisor received no matching confirmed plan.\n";
const ALL_ACK: &str = "Error: --assume-stopped is per-target only — not valid with 'all'.\n";

fn words(tail: &[&str]) -> Vec<String> {
    tail.iter().map(|word| (*word).to_owned()).collect()
}

fn run(rig: &Rig, tail: &[String], inside: bool) -> (Option<i32>, String, String) {
    let mut argv = vec!["_end"];
    argv.extend(tail.iter().map(String::as_str));
    if inside {
        rig.run_inside(&argv)
    } else {
        rig.run(&argv)
    }
}

fn unchanged(rig: &Rig, tail: &[String], inside: bool, expected: (i32, &str, &str)) {
    let before = guard_spec::tree(&rig.dir);
    let archives = rig.home.join("archive");
    let before_archive = archives.exists().then(|| guard_spec::tree(&archives));
    assert_eq!(
        run(rig, tail, inside),
        (
            Some(expected.0),
            expected.1.to_owned(),
            expected.2.to_owned()
        ),
        "argv={tail:?}, inside={inside}"
    );
    assert_eq!(
        guard_spec::tree(&rig.dir),
        before,
        "no target writes: {tail:?}"
    );
    assert_eq!(
        archives.exists().then(|| guard_spec::tree(&archives)),
        before_archive,
        "no archive writes: {tail:?}"
    );
    assert!(rig.session_is_live(), "original session survives: {tail:?}");
}

fn carried(name: &str) -> Vec<String> {
    words(&[
        &format!("--confirmed-target={name}"),
        "--confirmed-action=keep",
        "--confirmed-detail=/frozen/archive",
        "--confirmed-purge=off",
        "--confirmed-source=explicit",
        "--confirmed-tmux=unknown",
    ])
}

/// BASELINE BEHAVIOUR: argv errors win before target/caller/mode resolution;
/// ordinary words fail left to right, then completed carried rows are checked.
#[test]
fn endrun_grammar_refusal_precedence_table() {
    let rig = Rig::new("ergrammar");
    let unknown = "Error: unknown flag '--bogus'. Use -f, --purge-history, --keep-history, --assume-stopped.\n";
    let extra =
        "Error: unexpected extra argument 'extra' — _end takes one session name (or 'all').\n";
    let table = [
        (words(&["--bogus", &rig.name, "extra"]), unknown),
        (words(&[&rig.name, "extra", "--bogus"]), extra),
        (words(&["--supervise", "--bogus"]), unknown),
        (
            words(&["--pane", "--bogus"]),
            "Error: --pane needs a pane id as its next argument.\n",
        ),
        (
            words(&["--confirmed-target=", "--bogus"]),
            "Error: a carried end plan has no target.\n",
        ),
        (words(&["--confirmed-target=alpha", "--bogus"]), unknown),
        (
            words(&["--confirmed-target=alpha"]),
            "Error: carried end plan for 'alpha' is incomplete.\n",
        ),
        (
            words(&["--confirmed-action=keep"]),
            "Error: '--confirmed-action=keep' has no carried end target.\n",
        ),
    ];
    for (tail, err) in table {
        unchanged(&rig, &tail, false, (2, "", err));
    }
}

/// Missing target precedes mode dispatch. A proven caller supplies exactly
/// one target even in supervisor mode; it does not produce public usage.
#[test]
fn endrun_missing_and_inferred_target_table() {
    let rig = Rig::new("ertarget");
    for mode in [&[][..], &["--handoff"][..], &["--supervise"][..]] {
        for flags in [
            &[][..],
            &["-f"][..],
            &["--assume-stopped", "--purge-history"][..],
        ] {
            let mut tail = words(mode);
            tail.extend(words(flags));
            unchanged(&rig, &tail, false, (2, "", USAGE));
        }
    }
    let mut tail = words(&["--supervise"]);
    tail.push(format!("--pane={}", rig.pane));
    unchanged(&rig, &tail, true, (2, "", SUPERVISOR));
    tail.push(rig.name.clone());
    unchanged(&rig, &tail, true, (2, "", SUPERVISOR));
    unchanged(
        &rig,
        &words(&["--supervise", "--pane="]),
        true,
        (2, "", USAGE),
    );
}

/// BASELINE BEHAVIOUR: the last mode flag wins. Supervise's matching-plan
/// refusal precedes all+assume-stopped and every ordinary existence check.
#[test]
fn endrun_mode_flag_target_refusal_table() {
    let rig = Rig::new("ermodes");
    for target in ["all", "missing", "bad/name", rig.name.as_str()] {
        for history in [&[][..], &["--purge-history"][..], &["--keep-history"][..]] {
            let mut tail = words(&[target, "--handoff", "--supervise", "--assume-stopped", "-f"]);
            tail.extend(words(history));
            unchanged(&rig, &tail, false, (2, "", SUPERVISOR));
        }
    }
    for mode in [
        &[][..],
        &["--handoff"][..],
        &["--supervise", "--handoff"][..],
    ] {
        let mut tail = words(&["all", "--assume-stopped", "-f"]);
        tail.extend(words(mode));
        unchanged(&rig, &tail, false, (2, "", ALL_ACK));
        for target in ["missing", "bad/name"] {
            let mut tail = words(&[target, "-f"]);
            tail.extend(words(mode));
            let message = format!("Session '{target}' not found.\n");
            unchanged(&rig, &tail, false, (1, "", &message));
        }
    }
}

/// Carried lists are checked before existence or rename only in handoff;
/// duplicates are refused even when 'all' would accept the operand.
#[test]
fn endrun_carried_plan_refusal_precedence_table() {
    let rig = Rig::new("ercarried");
    for target in ["missing", "bad/name", rig.name.as_str()] {
        let mut tail = words(&[target, "--handoff", "-f"]);
        tail.extend(carried("other"));
        let message = format!("Error: carried end plans do not match '{target}'.\n");
        unchanged(&rig, &tail, false, (2, "", &message));
        tail.push("--supervise".to_owned());
        unchanged(&rig, &tail, false, (2, "", SUPERVISOR));
    }
    let mut tail = words(&["all", "--handoff", "-f"]);
    tail.extend(carried(&rig.name));
    tail.extend(carried(&rig.name));
    unchanged(
        &rig,
        &tail,
        false,
        (2, "", "Error: carried end plans do not match 'all'.\n"),
    );
    tail.push("--assume-stopped".to_owned());
    unchanged(&rig, &tail, false, (2, "", ALL_ACK));
    let mut direct = words(&["missing", "-f"]);
    direct.extend(carried("other"));
    unchanged(
        &rig,
        &direct,
        false,
        (1, "", "Session 'missing' not found.\n"),
    );
}

/// Resolve target existence before rename, and rename before freezing any
/// policy, prompting or starting a detached handoff. The damaged carrier
/// fixture is intentionally persisted hostile input, a documented refusal.
#[test]
fn endrun_pending_rename_precedes_prompt_and_handoff() {
    let rig = Rig::new("errename");
    let carrier = rig
        .home
        .join("sessions")
        .join(format!(".rename.{}.other.intent", rig.name));
    std::fs::write(&carrier, b"broken\n").expect("damaged carrier");
    let message = format!(
        "Error: a damaged rename carrier for '{}' → 'other' is pending ({} is not a valid rename intent (rename intent has a line without '=': \"broken\")) — repair or remove it by hand; this operation stands aside. Nothing was ended.\n",
        rig.name,
        carrier.display()
    );
    for mode in [&[][..], &["--handoff"][..]] {
        let mut tail = words(&[&rig.name]);
        tail.extend(words(mode));
        unchanged(&rig, &tail, false, (1, "", &message));
        tail.push("--supervise".to_owned());
        unchanged(&rig, &tail, false, (2, "", SUPERVISOR));
    }
    unchanged(
        &rig,
        &words(&["missing"]),
        false,
        (1, "", "Session 'missing' not found.\n"),
    );

    // Distinct carried names for all are accepted even when the first is
    // absent; the second's rename guard, not existence, is the next refusal.
    let mut tail = words(&["all", "--handoff"]);
    tail.extend(carried("missing"));
    tail.extend(carried(&rig.name));
    unchanged(&rig, &tail, false, (1, "", &message));
    let mut mismatch = words(&[&rig.name, "--handoff"]);
    mismatch.extend(carried(&rig.name));
    mismatch.extend(carried("other"));
    unchanged(
        &rig,
        &mismatch,
        false,
        (
            2,
            "",
            &format!("Error: carried end plans do not match '{}'.\n", rig.name),
        ),
    );
}

/// BASELINE BEHAVIOUR: the target need only have a worktree directory to
/// reach confirmation. Requiring a session directory would change this arm.
#[test]
fn endrun_worktree_only_target_reaches_confirmation() {
    let rig = Rig::new("erwtonly");
    let worktree = rig.home.join("worktrees").join("orphan");
    std::fs::create_dir_all(&worktree).expect("worktree-only target");
    let body = format!(
        "  - orphan: CANNOT be archived (its meta is missing or unreadable, so ae cannot identify {} — session memory or retained evidence is still there) — end will refuse it",
        rig.home.join("sessions/orphan").display()
    );
    let err = no_stdin_prompt(&rig, "This will END the session:", &body);
    unchanged(&rig, &words(&["orphan"]), false, (1, "", &err));
    assert!(worktree.is_dir(), "confirmation failure preserves worktree");
}

fn no_stdin_prompt(rig: &Rig, header: &str, body: &str) -> String {
    format!(
        "{header}\n{body}\nWork in a managed workspace is committed and pushed to ae/<session> first; the session is then removed from {}.\nContinue? [y/N] Error: could not obtain confirmation — no input on stdin.\n  Nothing was stopped and nothing was deleted. Run it from a terminal, or\n  pass -f if you mean to proceed without being asked.\n",
        rig.home.join("sessions").display(),
    )
}

/// Documented CLI history flags override config; BASELINE BEHAVIOUR keeps
/// the last conflicting flag. The frozen plan is what the prompt describes.
#[test]
fn endrun_history_policy_prompt_table() {
    let rig = Rig::new("erpolicy");
    for default_purge in [false, true] {
        let config = rig.home.join("config");
        let original = std::fs::read_to_string(&config).expect("config");
        std::fs::write(
            &config,
            format!("{original}purge_agent_history = {default_purge}\n"),
        )
        .expect("policy");
        for (flags, purge) in [
            (words(&[]), default_purge),
            (words(&["--keep-history"]), false),
            (words(&["--purge-history"]), true),
            (words(&["--purge-history", "--keep-history"]), false),
            (words(&["--keep-history", "--purge-history"]), true),
        ] {
            let mut tail = words(&[&rig.name]);
            tail.extend(flags);
            let archive = rig.archive();
            let body = if purge {
                format!(
                    "  - {}: NO archive, and {}/ is DELETED if it exists · conversation files DELETED",
                    rig.name,
                    archive.display()
                )
            } else {
                format!(
                    "  - {}: archive -> {}/ · conversation files KEPT",
                    rig.name,
                    archive.display()
                )
            };
            let err = no_stdin_prompt(&rig, "This will END the session:", &body);
            unchanged(&rig, &tail, false, (1, "", &err));
        }
        std::fs::write(&config, original).expect("restore config");
    }
}

/// BASELINE BEHAVIOUR: an empty public fleet asks before announcing empty;
/// handoff and force return immediately. No caller is inferred in this case.
#[test]
fn endrun_empty_fleet_confirmation_order_table() {
    let rig = Rig::new("erempty");
    let parked = rig.home.join("parked");
    std::fs::rename(&rig.dir, &parked).expect("empty census");
    let err = no_stdin_prompt(&rig, "This will END every ae session:", "  (none)");
    for tail in [words(&["all"]), words(&["all", "--keep-history"])] {
        assert_eq!(
            run(&rig, &tail, false),
            (Some(1), String::new(), err.clone())
        );
    }
    for tail in [
        words(&["all", "-f"]),
        words(&["all", "--handoff"]),
        words(&["all", "--supervise", "--handoff"]),
    ] {
        assert_eq!(
            run(&rig, &tail, false),
            (Some(0), "No ae sessions.\n".to_owned(), String::new())
        );
    }
    assert_eq!(
        run(&rig, &words(&["all", "--handoff", "--supervise"]), false),
        (Some(2), String::new(), SUPERVISOR.to_owned())
    );
    assert!(rig.session_is_live());
    assert!(!rig.archive().exists());
    std::fs::rename(parked, &rig.dir).expect("restore registration");
}

fn answered(rig: &Rig, tail: &[&str], answer: &[u8]) -> (Option<i32>, String, String) {
    let input = rig.home.join("endrun-answer");
    std::fs::write(&input, answer).expect("supplied stdin line");
    let mut cmd = ae();
    cmd.env("AE_HOME", &rig.home);
    cmd.env_remove("TMUX");
    cmd.env_remove("TMUX_PANE");
    cmd.args(tail);
    let output = bounded(
        cmd.stdin(std::fs::File::open(input).expect("answer file"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("ae starts"),
        Duration::from_secs(30),
    )
    .expect("ae returns");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// BASELINE BEHAVIOUR/B1: declining the terminal question exits zero.
/// An accepted empty fleet prints its empty notice AFTER the question.
#[test]
fn endrun_supplied_reply_classification_table() {
    let rig = Rig::new("erreply");
    let before = guard_spec::tree(&rig.dir);
    let body = format!(
        "  - {}: archive -> {}/ · conversation files KEPT",
        rig.name,
        rig.archive().display()
    );
    let full = no_stdin_prompt(&rig, "This will END the session:", &body);
    let prefix = full
        .split_once("Error: could not obtain")
        .expect("prompt boundary")
        .0;
    for answer in [b"n\n".as_slice(), b"\n", b"no\n"] {
        assert_eq!(
            answered(&rig, &["_end", &rig.name], answer),
            (Some(0), String::new(), format!("{prefix}Aborted.\n"))
        );
        assert_eq!(guard_spec::tree(&rig.dir), before);
        assert!(rig.session_is_live());
        assert!(!rig.archive().exists());
    }
    let parked = rig.home.join("parked");
    std::fs::rename(&rig.dir, &parked).expect("empty census");
    let full = no_stdin_prompt(&rig, "This will END every ae session:", "  (none)");
    let prefix = full
        .split_once("Error: could not obtain")
        .expect("prompt boundary")
        .0;
    for answer in [b"y\n".as_slice(), b"  Yes\n"] {
        assert_eq!(
            answered(&rig, &["_end", "all"], answer),
            (Some(0), "No ae sessions.\n".to_owned(), prefix.to_owned())
        );
    }
    assert!(rig.session_is_live());
    std::fs::rename(parked, &rig.dir).expect("restore registration");
}

fn wait_ended(rig: &Rig) {
    let deadline = Instant::now() + SETUP_WAIT;
    while Instant::now() < deadline {
        if rig.archive().is_dir() && !rig.dir.exists() && !rig.session_is_live() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("supervisor did not finish: {}", rig.name);
}

/// Caller membership selects the handoff, including an inferred name and
/// fleet. These exact start lines distinguish the branch from direct end.
#[test]
fn endrun_caller_membership_handoff_table() {
    for (tag, target) in [("erself", "self"), ("erfleet", "all")] {
        let rig = Rig::new(tag);
        let mut tail = words(&["-f"]);
        if target == "all" {
            tail.push("all".to_owned());
        }
        let got = run(&rig, &tail, true);
        let expected = if target == "all" {
            "Ending all ae sessions out of pane.\n".to_owned()
        } else {
            format!("Ending '{}' out of pane; this pane will close.\n", rig.name)
        };
        assert_eq!(got, (Some(0), expected, String::new()));
        wait_ended(&rig);
        let events = std::fs::read_to_string(rig.archive().join("events.jsonl"))
            .expect("archived handoff journal");
        assert!(events.contains("\"action\":\"end-request\""), "{events}");
        assert!(!events.contains("\"action\":\"end-result\""), "{events}");
    }
}

/// A caller outside the target set does not cause handoff. An unresolvable
/// pane is likewise harmless when an explicit target supplies the intent.
#[test]
fn endrun_explicit_target_direct_despite_other_or_unreadable_caller() {
    for (tag, pane) in [("erother", None), ("erunread", Some("%999999"))] {
        let rig = Rig::new(tag);
        let target = "victim";
        let dir = rig.home.join("sessions").join(target);
        std::fs::create_dir(&dir).expect("target dir");
        let text = std::fs::read_to_string(rig.dir.join("meta")).expect("meta");
        let other_id = "44444444-4444-4444-8444-444444444444";
        let text = text
            .replace(&format!("session={}\n", rig.name), "session=victim\n")
            .replace(UUID, other_id);
        assert!(rig.tmux(&["new-session", "-d", "-s", target, "sh"]).0);
        let pane_id = first_pane(
            |args| rig.tmux(args),
            &["-s", "-t", "=victim"],
            Instant::now() + SETUP_WAIT,
        );
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &pane_id, "@ae_agent", "lead"])
                .0
        );
        let text = text.replace(
            &format!("main_pane={}\n", rig.pane),
            &format!("main_pane={pane_id}\n"),
        );
        std::fs::write(dir.join("meta"), text).expect("target registration");
        assert!(
            ae::migrate::session_noted(&dir, target).is_some(),
            "fixture meta migrated before the observation"
        );
        let before = guard_spec::tree(&rig.dir);
        let mut tail = words(&[target, "-f"]);
        if let Some(pane) = pane {
            tail.push(format!("--pane={pane}"));
        }
        let got = run(&rig, &tail, true);
        assert_eq!(got.0, Some(0), "{got:?}");
        assert!(got.2.is_empty(), "{got:?}");
        assert!(!got.1.contains("out of pane"), "{got:?}");
        assert!(!dir.exists(), "explicit target removed");
        assert!(rig.session_is_live(), "caller survives");
        assert_eq!(guard_spec::tree(&rig.dir), before);
        let archive = rig.home.join("archive").join(other_id);
        assert!(archive.is_dir(), "target archived");
        let events = std::fs::read_to_string(archive.join("events.jsonl")).unwrap_or_default();
        assert!(
            !events.contains("\"action\":\"end-request\""),
            "direct path: {events}"
        );
    }
}
