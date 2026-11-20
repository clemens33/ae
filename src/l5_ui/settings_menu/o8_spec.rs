//! O8 Option B grammar pins: unattended applies carry only the captured
//! action, target, UUID and deadline. Client attachment proof cannot be
//! mixed with that form. The shared parser keeps the legacy attached form.

#![allow(clippy::expect_used, reason = "pure acceptance parser fixtures")]

use super::{LaunchAction, parse_apply};

const UUID: &str = "0199c0de-1234-4890-abcd-ef0123456789";

fn unattended(action: &str) -> Vec<String> {
    [
        "--settings-apply",
        action,
        "--target",
        "orchestrator",
        "--uuid",
        if action == "start" { "" } else { UUID },
        "--unattended",
        "--deadline",
        "1789109780",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

fn attached() -> Vec<String> {
    [
        "--settings-apply",
        "resume",
        "--target",
        "renamed",
        "--uuid",
        UUID,
        "--client",
        "/dev/ttys004",
        "--client-pid",
        "4242",
        "--server-pid",
        "911",
        "--server-start",
        "1789109660",
        "--deadline",
        "1789109780",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

fn replace(args: &mut [String], flag: &str, value: &str) {
    let at = args
        .iter()
        .position(|arg| arg == flag)
        .expect("fixture flag");
    args[at + 1] = value.to_owned();
}

/// GREEN at base: the unattended extension keeps existing exact-client
/// continuations accepted and preserves their captured role and identity.
#[test]
fn the_legacy_attached_apply_keeps_its_captured_identity() {
    let saved = parse_apply(&attached()).expect("the existing attached form");
    assert_eq!(saved.action, LaunchAction::Resume);
    assert_eq!(saved.target, "renamed");
    assert_eq!(saved.uuid, UUID);
    assert_eq!(saved.deadline, 1_789_109_780);
}

#[test]
fn unattended_start_and_resume_keep_the_exact_captured_expectation() {
    for (action, want, uuid) in [
        ("start", LaunchAction::Start, ""),
        ("resume", LaunchAction::Resume, UUID),
    ] {
        let parsed = parse_apply(&unattended(action));
        assert!(
            parsed.is_ok(),
            "o8 must accept the strict unattended {action} form"
        );
        let parsed = parsed.expect("the asserted unattended form");
        assert_eq!(parsed.action, want);
        assert_eq!(parsed.target, "orchestrator");
        assert_eq!(parsed.uuid, uuid);
        assert_eq!(parsed.deadline, 1_789_109_780);
    }
}

#[test]
fn unattended_refuses_every_attachment_flag_in_either_order() {
    let accepted = unattended("resume");
    assert!(
        parse_apply(&accepted).is_ok(),
        "the unattended control must parse before refusal assertions"
    );
    for (flag, value) in [
        ("--client", "/dev/ttys004"),
        ("--client-pid", "4242"),
        ("--server-pid", "911"),
        ("--server-start", "1789109660"),
    ] {
        let mut after = accepted.clone();
        after.extend([flag.to_owned(), value.to_owned()]);
        assert!(parse_apply(&after).is_err(), "unattended plus {flag}");
        let mut before = accepted.clone();
        before.splice(2..2, [flag.to_owned(), value.to_owned()]);
        assert!(parse_apply(&before).is_err(), "{flag} before unattended");
    }
    let mut mixed = attached();
    mixed.push("--unattended".to_owned());
    assert!(
        parse_apply(&mixed).is_err(),
        "both attachment forms together"
    );
}

#[test]
fn unattended_refuses_missing_duplicate_unknown_and_malformed_words() {
    let accepted = unattended("resume");
    assert!(
        parse_apply(&accepted).is_ok(),
        "the unattended control must parse before refusal assertions"
    );
    for flag in ["--target", "--uuid", "--deadline"] {
        let at = accepted
            .iter()
            .position(|arg| arg == flag)
            .expect("fixture flag");
        let mut missing = accepted.clone();
        missing.drain(at..at + 2);
        assert!(parse_apply(&missing).is_err(), "missing {flag}");
        let mut duplicate = accepted.clone();
        duplicate.extend([flag.to_owned(), accepted[at + 1].clone()]);
        assert!(parse_apply(&duplicate).is_err(), "duplicate {flag}");
    }
    let mut twice = accepted.clone();
    twice.push("--unattended".to_owned());
    assert!(parse_apply(&twice).is_err(), "duplicate unattended flag");
    for extra in ["yes", "--unknown"] {
        let mut unknown = accepted.clone();
        unknown.push(extra.to_owned());
        assert!(parse_apply(&unknown).is_err(), "unexpected word {extra}");
    }
    for flag in ["--target", "--uuid", "--deadline"] {
        let mut dangling = accepted.clone();
        dangling.push(flag.to_owned());
        assert!(parse_apply(&dangling).is_err(), "dangling {flag}");
    }
    for action in ["Start", "Resume", "pause", ""] {
        let mut bad = accepted.clone();
        bad[1] = action.to_owned();
        assert!(parse_apply(&bad).is_err(), "unknown action {action:?}");
    }
    for (flag, bad) in [
        ("--target", "../seat"),
        ("--target", ""),
        ("--target", "renamed"),
        ("--uuid", ""),
        ("--uuid", "not-a-uuid"),
        ("--deadline", "later"),
        ("--deadline", "18446744073709551616"),
    ] {
        let mut malformed = accepted.clone();
        replace(&mut malformed, flag, bad);
        assert!(parse_apply(&malformed).is_err(), "{flag} = {bad:?}");
    }
    for (flag, bad) in [("--target", "renamed"), ("--uuid", UUID)] {
        let mut start = unattended("start");
        replace(&mut start, flag, bad);
        assert!(parse_apply(&start).is_err(), "Start {flag} = {bad:?}");
    }
    let mut no_form = accepted;
    let at = no_form
        .iter()
        .position(|arg| arg == "--unattended")
        .expect("form flag");
    no_form.remove(at);
    assert!(parse_apply(&no_form).is_err(), "neither attachment form");
}
