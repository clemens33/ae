//! Characterisation of `run_with` before extraction: output, refusal and IO order.
//! Rootless cases re-exec this test binary; no process-global environment writes.

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

use super::cli::helper;

fn words(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn rootless_dispatch_cases_run_in_a_separate_process() {
    let binary = std::env::current_exe().expect("the integration test binary");
    for name in [
        "root_gates_child",
        "list_ended_child",
        "unwired_orchestrator_child",
        "list_with_world_but_no_root_child",
    ] {
        let name = format!("runwith_spec::{name}");
        let child = helper(&binary)
            .args(["--exact", &name, "--ignored", "--nocapture"])
            .env_remove("HOME")
            .env_remove("AE_HOME")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .expect("isolated test child runs");
        assert!(child.status.success(), "{name}: {child:?}");
        let said = String::from_utf8_lossy(&child.stdout);
        assert!(said.contains("running 1 test"), "{name}: {said}");
        assert!(said.contains("1 passed"), "{name}: {said}");
    }
}

#[test]
#[ignore = "run only by rootless_dispatch_cases_run_in_a_separate_process"]
fn root_gates_child() {
    // Every root-dependent match arm, including public words that run() gates earlier.
    let cases: &[&[&str]] = &[
        &["_relaunch", "/unread/session"],
        &["reseat"],
        &["_end"],
        &["_stop"],
        &["_session-menu"],
        &["_compact"],
        &["doctor"],
        &["init"],
        &["rename"],
        &["_auto-reseat"],
        &["_auto-effort"],
        &["_watchdog"],
        &["_telegram"],
    ];
    for case in cases {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = ae::run_with(&words(case), None, &mut out, &mut err);
        assert_eq!(result.expect("refusal writes"), 1, "{case:?}");
        assert!(out.is_empty(), "{case:?}: {out:?}");
        assert_eq!(
            err,
            format!("ae: {}\n", ae::NO_STATE_ROOT).as_bytes(),
            "{case:?}"
        );
        let trace = Trace::default();
        let (mut out, mut err) = (
            writer("out", &trace, false, false),
            writer("err", &trace, true, false),
        );
        let failed = ae::run_with(&words(case), None, &mut out, &mut err);
        assert!(matches!(failed, Err(ae::Error::Io(_))), "{case:?}");
        assert_eq!(
            &*trace.borrow(),
            &[("err", "write")],
            "{case:?}: no flush on arm error"
        );
    }
}

#[test]
#[ignore = "run only by rootless_dispatch_cases_run_in_a_separate_process"]
fn list_ended_child() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = ae::run_with(&words(&["list", "--ended"]), None, &mut out, &mut err);
    assert_eq!(result.expect("refusal writes"), 1);
    assert!(out.is_empty());
    // Existing inconsistency, deliberately preserved rather than repaired here.
    assert_eq!(err, format!("{}\n", ae::NO_STATE_ROOT).as_bytes());
}

#[test]
#[ignore = "run only by rootless_dispatch_cases_run_in_a_separate_process"]
fn list_with_world_but_no_root_child() {
    let world = ae::listing::World {
        now: ae::time::Timestamp::now(),
        sessions: vec![],
        losses: 1,
    };
    for json in [false, true] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let case = if json {
            vec!["list", "--json"]
        } else {
            vec!["list"]
        };
        let result = ae::run_with(&words(&case), Some(&world), &mut out, &mut err);
        assert_eq!(result.expect("world is sufficient to render list"), 0);
        assert!(!out.is_empty());
        let expected = if json {
            ""
        } else {
            "ae: warning: inventory incomplete — 1 logical source could not be enumerated; sessions they may hold are absent from this listing\n"
        };
        assert_eq!(err, expected.as_bytes(), "{case:?}");
    }
}

#[test]
#[ignore = "run only by rootless_dispatch_cases_run_in_a_separate_process"]
fn unwired_orchestrator_child() {
    for flag in ["--popup", "--settings", "--quota-dialog"] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = ae::run_with(&words(&["orchestrator", flag]), None, &mut out, &mut err);
        assert_eq!(result.expect("menu refusal writes"), 1, "{flag}");
        assert!(out.is_empty(), "{flag}");
        assert_eq!(
            err,
            format!("ae: {}\n", ae::NO_STATE_ROOT).as_bytes(),
            "{flag}"
        );
    }
    for tail in [vec!["orchestrator"], vec!["orchestrator", "--client", "x"]] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = ae::run_with(&words(&tail), None, &mut out, &mut err);
        assert_eq!(result.expect("seat refusal writes"), 2, "{tail:?}");
        assert!(out.is_empty());
        assert_eq!(
            err,
            b"ae orchestrator: the seat launches through the ae command\n"
        );
    }
}

#[test]
fn orchestrator_usage_and_help_keep_their_streams_text_and_codes() {
    let cases: &[(&[&str], u8, &str)] = &[
        (
            &["orchestrator", "--bogus"],
            2,
            "ae orchestrator: unknown argument '--bogus' (see: ae orchestrator --help)\n",
        ),
        (
            &["orchestrator", "--client"],
            2,
            "ae orchestrator: --client requires a nonempty tmux client name\n",
        ),
        (&["orchestrator", "--help"], 0, ae::orchestrator::USAGE),
    ];
    for (case, code, expected) in cases {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let actual = ae::run_with(&words(case), None, &mut out, &mut err);
        assert_eq!(actual.expect("usage writes"), *code, "{case:?}");
        assert!(out.is_empty(), "{case:?}");
        assert_eq!(err, expected.as_bytes(), "{case:?}");
    }
}

#[test]
fn all_three_missing_operand_nouns_are_preserved() {
    for (command, noun) in [
        ("_net-probe", "a host to resolve"),
        ("_telegram-run", "an ae home directory"),
        ("_watchdog-run", "a session meta directory"),
    ] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let actual = ae::run_with(&words(&[command]), None, &mut out, &mut err);
        assert_eq!(actual.expect("operand refusal writes"), 2, "{command}");
        assert!(out.is_empty(), "{command}");
        assert_eq!(err, format!("ae: {command} needs {noun}\n").as_bytes());
    }
}

#[test]
fn no_world_refuses_list_and_monitor_before_their_delegates() {
    for case in [vec!["list"], vec!["_monitor", "sweep", "/unread/session"]] {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let actual = ae::run_with(&words(&case), None, &mut out, &mut err);
        assert_eq!(actual.expect("world refusal writes"), 1, "{case:?}");
        assert!(out.is_empty(), "{case:?}");
        assert_eq!(err, format!("ae: {}\n", ae::NO_STATE_ROOT).as_bytes());
    }
}

#[test]
fn list_warning_cases_run_with_an_isolated_state_root() {
    let scratch = super::cli::OwnedScratch::root("runwith", "list");
    // macOS cwd is canonical; use that same spelling for AE_HOME and expected paths.
    let root = scratch.canonicalize().expect("canonical fixture root");
    let binary = std::env::current_exe().expect("the integration test binary");
    let output = helper(&binary)
        .args([
            "--exact",
            "runwith_spec::list_warnings_child",
            "--ignored",
            "--nocapture",
        ])
        .env("AE_HOME", &root)
        .current_dir(&root)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .expect("isolated list child runs");
    assert!(output.status.success(), "{output:?}");
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(
        said.contains("running 1 test") && said.contains("1 passed"),
        "{said}"
    );
}

#[test]
#[ignore = "run only by list_warning_cases_run_with_an_isolated_state_root"]
fn list_warnings_child() {
    let root = std::env::current_dir().expect("explicit fixture cwd");
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).expect("session root");
    for (old, new, phase) in [
        ("lpA", "lpB", "prepared"),
        ("apA", "apB", "prepared"),
        ("cpA", "cpB", "complete"),
    ] {
        let body = format!(
            "rename_intent=1\nsession_id=00000000-0000-4000-8000-000000000001\nold={old}\nnew={new}\nmode=local\nold_work=/w\nnew_work=/w\norigin=/w\nserver_kind=ambient\nserver_value=\nphase={phase}\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n"
        );
        std::fs::write(sessions.join(format!(".rename.{old}.{new}.intent")), body)
            .expect("pending intent");
    }
    let damaged = sessions.join(".rename.dgA.dgB.intent");
    std::fs::write(&damaged, "rename_intent=1\nthis line has no equals\n")
        .expect("damaged carrier");
    let world = ae::listing::World {
        now: ae::time::Timestamp::now(),
        sessions: vec![],
        losses: 1,
    };
    let warning = "ae: warning: inventory incomplete — 1 logical source could not be enumerated; sessions they may hold are absent from this listing\n";
    let pending = format!(
        "ae: warning: rename 'apA' → 'apB' is in progress at phase 'prepared' — retry 'ae rename apA apB'\nae: warning: rename 'lpA' → 'lpB' is in progress at phase 'prepared' — retry 'ae rename lpA lpB'\nae: warning: a damaged rename carrier is pending ({} is not a valid rename intent (rename intent has a line without '=': \"this line has no equals\")) — repair or remove it by hand\n",
        damaged.display()
    );
    for json in [false, true] {
        let trace = Trace::default();
        let (mut out, mut err) = (
            writer("out", &trace, false, false),
            writer("err", &trace, false, false),
        );
        let case = if json {
            vec!["list", "--json"]
        } else {
            vec!["list"]
        };
        let result = ae::run_with(&words(&case), Some(&world), &mut out, &mut err);
        assert_eq!(result.expect("list writes"), 0);
        let expected = if json {
            pending.clone()
        } else {
            format!("{warning}{pending}")
        };
        assert_eq!(err.bytes, expected.as_bytes(), "{case:?}");
        assert!(!out.bytes.is_empty());
        if json {
            assert_eq!(out.bytes.first(), Some(&b'{'));
        }
        let events = trace.borrow();
        let first_out = events
            .iter()
            .position(|event| *event == ("out", "write"))
            .expect("table writes");
        let last_err = events
            .iter()
            .rposition(|event| *event == ("err", "write"))
            .expect("warnings write");
        assert!(last_err < first_out, "warnings precede stdout: {events:?}");
    }
}

#[test]
fn unread_viewer_refusal_keeps_exact_text_exit_and_no_effects_for_every_arm() {
    use super::cli::{OwnedScratch, ae, byte_tree};

    let scratch = OwnedScratch::root("runwith", "viewer");
    let dir = scratch.join("sessions/one");
    std::fs::create_dir_all(&dir).expect("session directory");
    std::fs::write(dir.join("meta"), "session=one\n").expect("session marker");
    let before = byte_tree(&dir);
    let caller = format!("{},0,0", scratch.join("absent.sock").display());
    for command in ["_send", "_interrupt", "_spawn", "_retire", "_relaunch"] {
        let output = ae()
            .arg(command)
            .arg(&dir)
            .env("AE_HOME", &scratch)
            .env("TMUX", &caller)
            .env("TMUX_PANE", "%1")
            .env_remove("AE_SENDER_OVERRIDE")
            .output()
            .expect("the binary runs");
        assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
        assert!(output.stdout.is_empty(), "{command}");
        assert_eq!(output.stderr,
            b"Error: could not detect current agent identity; retry from your own ae pane, or unset stale TMUX and TMUX_PANE markers in a shell outside tmux\n",
            "{command}");
        assert_eq!(byte_tree(&dir), before, "{command}: session unchanged");
    }
}

#[test]
fn spawn_role_and_unreadable_meta_refusals_keep_exact_exit_one_without_effects() {
    use super::cli::{OwnedScratch, ae, byte_tree};

    for damaged in [false, true] {
        let scratch = OwnedScratch::root("runwith", if damaged { "badmeta" } else { "role" });
        let dir = scratch.join("sessions/S");
        std::fs::create_dir_all(&dir).expect("session directory");
        if damaged {
            std::fs::create_dir(dir.join("meta")).expect("non-regular meta");
        } else {
            std::fs::write(dir.join("meta"), "session=S\nmeta_agent=true\n").expect("role marker");
        }
        let before = byte_tree(&dir);
        let output = ae()
            .arg("_spawn")
            .arg(&dir)
            .args(["x", "--using", "y"])
            .env("AE_HOME", &scratch)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("AE_SENDER_OVERRIDE")
            .output()
            .expect("the binary runs");
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        let expected = if damaged {
            "ae: delivery REFUSED — session meta unreadable or damaged; orchestrator role cannot be ruled out; nothing further typed. Report through your own lead; owner routes for an orchestrator role: pane, ae chat, ae app, Telegram.\n"
        } else {
            "ae: delivery REFUSED — Error: session 'S' records no usable tmux server — refresh or migrate the session, then retry; nothing further typed. Report through your own lead; owner routes for an orchestrator role: pane, ae chat, ae app, Telegram.\n"
        };
        assert_eq!(output.stderr, expected.as_bytes());
        assert_eq!(byte_tree(&dir), before);
    }
}

type Trace = Rc<RefCell<Vec<(&'static str, &'static str)>>>;

struct Writer {
    name: &'static str,
    trace: Trace,
    fail_write: bool,
    fail_flush: bool,
    bytes: Vec<u8>,
}

impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.trace.borrow_mut().push((self.name, "write"));
        if self.fail_write {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.trace.borrow_mut().push((self.name, "flush"));
        if self.fail_flush {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            Ok(())
        }
    }
}

fn writer(name: &'static str, trace: &Trace, fail_write: bool, fail_flush: bool) -> Writer {
    Writer {
        name,
        trace: Rc::clone(trace),
        fail_write,
        fail_flush,
        bytes: Vec::new(),
    }
}

#[test]
fn refusal_flushes_stderr_before_stdout_and_preserves_the_exit_code() {
    for case in [
        vec!["--bad"],
        vec!["_net-probe"],
        vec!["orchestrator"],
        vec!["unwired-session"],
        vec!["_monitor", "sweep", "/unused"],
    ] {
        let trace = Trace::default();
        let (mut out, mut err) = (
            writer("out", &trace, false, false),
            writer("err", &trace, false, false),
        );
        let actual = ae::run_with(&words(&case), None, &mut out, &mut err);
        let expected = if matches!(case[0], "unwired-session" | "_monitor") {
            1
        } else {
            2
        };
        assert_eq!(actual.expect("refusal completes"), expected, "{case:?}");
        let events = trace.borrow();
        assert!(
            events
                .iter()
                .filter(|(_, op)| *op == "write")
                .all(|(name, _)| *name == "err")
        );
        assert_eq!(
            &events[events.len() - 2..],
            &[("err", "flush"), ("out", "flush")]
        );
    }
}

#[test]
fn stderr_write_or_flush_error_short_circuits_the_remaining_flushes() {
    for (fail_write, fail_flush) in [(true, false), (false, true)] {
        let trace = Trace::default();
        let (mut out, mut err) = (
            writer("out", &trace, false, false),
            writer("err", &trace, fail_write, fail_flush),
        );
        let actual = ae::run_with(&words(&["--bad"]), None, &mut out, &mut err);
        assert!(matches!(actual, Err(ae::Error::Io(_))));
        let events = trace.borrow();
        assert!(events.iter().all(|(name, _)| *name == "err"));
        if fail_write {
            assert_eq!(&*events, &[("err", "write")]);
        } else {
            assert_eq!(events.last(), Some(&("err", "flush")));
        }
    }
}

#[test]
fn stdout_flush_failure_is_returned_after_stderr_flush() {
    let trace = Trace::default();
    let (mut out, mut err) = (
        writer("out", &trace, false, true),
        writer("err", &trace, false, false),
    );
    let actual = ae::run_with(&words(&["--bad"]), None, &mut out, &mut err);
    assert!(matches!(actual, Err(ae::Error::Io(_))));
    let events = trace.borrow();
    assert_eq!(
        &events[events.len() - 2..],
        &[("err", "flush"), ("out", "flush")]
    );
}
