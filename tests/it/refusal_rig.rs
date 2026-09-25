//! The ONE link-window refusal rig (#194), shared by the stop, retire and
//! reap tests that must force a deterministic ownership refusal on a real
//! server: a pane the session listing reports as ours but the ownership probe
//! reports as theirs. Takes the caller's tmux runner, so this module adds no
//! process door of its own.

/// Build the rig and return the linked pane's id: session `ours` with `window`
/// carrying one pane stamped `stamp`, then session `theirs` with that window
/// linked in. The listing under `=ours` reads `ours | <stamp>` (presence and
/// the stamp lookups find it); `display-message -t <pane>` answers
/// `theirs | <stamp>` (the later-created session wins), so the
/// ownership-checked kill refuses with `WrongSession("theirs")`.
pub(crate) fn linked(
    run: &dyn Fn(&[&str]) -> (bool, String),
    ours: &str,
    theirs: &str,
    window: &str,
    stamp: &str,
) -> String {
    assert!(
        run(&["new-session", "-d", "-s", ours, "sleep", "60"]).0,
        "session {ours}"
    );
    let target = format!("{ours}:{window}");
    assert!(
        run(&["new-window", "-d", "-t", ours, "-n", window, "sleep", "60"]).0,
        "window {target}"
    );
    let pane = run(&["display-message", "-p", "-t", &target, "#{pane_id}"])
        .1
        .trim()
        .to_owned();
    assert!(!pane.is_empty(), "the linked pane has an id");
    assert!(
        run(&["set-option", "-p", "-t", &pane, "@ae_agent", stamp]).0,
        "stamp {stamp} on {pane}"
    );
    // Creation order decides which session a probe names; the pause keeps it
    // unambiguous under load.
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert!(
        run(&["new-session", "-d", "-s", theirs, "sleep", "60"]).0,
        "session {theirs}"
    );
    let into = format!("{theirs}:");
    assert!(
        run(&["link-window", "-d", "-s", &target, "-t", &into]).0,
        "link {target} into {theirs}"
    );
    pane
}

/// The live pid behind `pane`.
pub(crate) fn pid_of(run: &dyn Fn(&[&str]) -> (bool, String), pane: &str) -> String {
    let pid = run(&["display-message", "-p", "-t", pane, "#{pane_pid}"])
        .1
        .trim()
        .to_owned();
    assert!(!pid.is_empty(), "the pane {pane} has a pid");
    pid
}

/// A live pane stamped `stamp` in `session`: split, stamp, and its pid.
pub(crate) fn stamped(
    run: &dyn Fn(&[&str]) -> (bool, String),
    session: &str,
    stamp: &str,
) -> (String, String) {
    let pane = run(&[
        "split-window",
        "-d",
        "-t",
        session,
        "-P",
        "-F",
        "#{pane_id}",
        "sleep",
        "60",
    ])
    .1
    .trim()
    .to_owned();
    assert!(!pane.is_empty(), "the split pane has an id");
    assert!(
        run(&["set-option", "-p", "-t", &pane, "@ae_agent", stamp]).0,
        "stamp {stamp} on {pane}"
    );
    let pid = pid_of(run, &pane);
    (pane, pid)
}

/// The ownership pair `seed_unwatched` proves — the ae marker plus the state
/// root — as a launch leaves them. Without it the seed writes nothing and a
/// no-seed assert holds vacuously.
pub(crate) fn own(run: &dyn Fn(&[&str]) -> (bool, String), session: &str, root: &std::path::Path) {
    assert!(
        run(&["set-environment", "-t", session, "AE_SESSION", "1"]).0,
        "the ae marker"
    );
    let home = root.display().to_string();
    assert!(
        run(&["set-environment", "-t", session, "AE_HOME", &home]).0,
        "the state root"
    );
}
