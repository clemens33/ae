//! Stop-all counts an already-stopped session as a failure when a recorded
//! orphan view survives cleanup. Real observations and aggregate behavior;
//! only the external tmux kill of that exact private view is refused.

use super::*;

#[test]
fn runstop_public_all_accepts_an_already_stopped_session_without_views() {
    let rig = Rig::new("rsnoview");
    guard_spec::keepalive(&rig);
    ae::migrate::session(&rig.dir).expect("current fixture meta");
    let before = std::fs::read(rig.dir.join("meta")).expect("original meta");
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={}", rig.name)])
            .0
    );
    assert!(!rig.session_is_live());
    assert_eq!(
        rig.run(&["_stop", "all", "-y"]),
        (
            Some(0),
            String::new(),
            format!("Session '{}' is not running.\n", rig.name),
        ),
        "already down without an orphan view satisfies the fleet stop"
    );
    assert_eq!(
        std::fs::read(rig.dir.join("meta")).expect("kept meta"),
        before
    );
    assert!(!rig.session_is_live());
    assert!(rig.tmux(&["has-session", "-t", "=keepalive"]).0);
}

#[test]
fn runstop_public_all_counts_a_stopped_session_whose_orphan_view_survives() {
    let rig = Rig::new("rsorphan");
    guard_spec::keepalive(&rig);
    ae::migrate::session(&rig.dir).expect("current fixture meta");
    let before = std::fs::read(rig.dir.join("meta")).expect("original meta");
    let view = "_aeview-rsorphan";
    let original_target = format!("={}", rig.name);
    assert!(
        rig.tmux(&["new-session", "-d", "-s", view, "-t", &original_target])
            .0,
        "a real grouped view shares the target's windows"
    );
    let view_target = format!("={view}:");
    assert!(
        rig.tmux(&["set-option", "-t", &view_target, "@ae_view_uuid", UUID])
            .0
    );
    let (ok, id) = rig.tmux(&["display-message", "-p", "-t", &view_target, "#{session_id}"]);
    assert!(ok);
    let id = id.trim();
    assert!(id.starts_with('$'), "a real view session id: {id}");
    assert!(
        rig.tmux(&["kill-session", "-t", &format!("={}", rig.name)])
            .0
    );
    assert!(
        !rig.session_is_live(),
        "the recorded target is already down"
    );
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &rig.pane, "#{pane_id}"])
            .1
            .trim(),
        rig.pane,
        "the orphan view still holds the original pane"
    );

    let denied = format!("={id}");
    let mut cmd = ae();
    cmd.env("AE_HOME", &rig.home)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env("AE_RUNSTOP_DENY_VIEW", &denied)
        .args(["_stop", "all", "-y"]);
    let trace = super::super::session_launch::launchx_trace_command(&mut cmd, &rig.home);
    let wrapper = rig.home.join("trace-bin/tmux");
    let text = std::fs::read_to_string(&wrapper).expect("owned tmux wrapper");
    let marker = "exec {$ENV{AE_LAUNCHX_REAL_TMUX}}";
    assert_eq!(text.matches(marker).count(), 1);
    let hook = r"
if ((grep { $_ eq 'kill-session' } @ARGV)
    && (grep { $_ eq $ENV{AE_RUNSTOP_DENY_VIEW} } @ARGV)) { exit 1; }
";
    std::fs::write(&wrapper, text.replace(marker, &format!("{hook}\n{marker}")))
        .expect("refuse only this view's kill");
    let output = bounded(
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the core starts"),
        Duration::from_secs(30),
    )
    .expect("the fleet stop finishes");
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr={err}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        err,
        format!(
            "Session '{name}' is not running.\nError: session '{name}' is gone, but pane view session(s) {id} still hold its windows, so its agents still run — state preserved. Retry 'ae stop {name}'.\n1 session(s) failed to stop. See errors above.\n",
            name = rig.name,
        )
    );
    let calls = std::fs::read_to_string(trace).expect("tmux trace");
    assert!(
        calls.lines().any(|line| {
            line.split('\t').any(|arg| arg == "kill-session")
                && line.split('\t').any(|arg| arg == denied)
        }),
        "cleanup tried the exact view kill: {calls}"
    );
    assert!(rig.tmux(&["has-session", "-t", &format!("={view}")]).0);
    assert_eq!(
        std::fs::read(rig.dir.join("meta")).expect("kept meta"),
        before
    );
    assert!(!rig.session_is_live());
}
