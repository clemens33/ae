//! A prior fleet failure remains a failure when the last successful stop's
//! report meets closed stdout. Oracle: stop-all aggregates failure status;
//! BASELINE BEHAVIOUR: the first broken output write ends the loop.

use super::*;

#[test]
fn runstop_public_all_carries_a_prior_failure_into_closed_last_output() {
    let rig = Rig::new("rspipefailure");
    guard_spec::keepalive(&rig);
    let unowned = rig.home.join("sessions/aaa-unowned");
    std::fs::create_dir_all(&unowned).expect("an earlier unowned record");
    let meta = b"schema=2\nsession=aaa-unowned\ntmux_server_kind=ambiguous\ntmux_server=unused\n";
    std::fs::write(unowned.join("meta"), meta).expect("unowned server record");
    ae::migrate::session(&unowned).expect("current fixture meta before snapshot");
    let before = std::fs::read(unowned.join("meta")).expect("original unowned meta");

    // Drop the read end before spawning: the first actual stdout write must
    // meet EPIPE, without a race against another process closing the reader.
    let (reader, writer) = std::io::pipe().expect("a pipe");
    drop(reader);
    let output = bounded(
        ae().env("AE_HOME", &rig.home)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(["_stop", "all", "-y"])
            .stdin(std::process::Stdio::null())
            .stdout(writer)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the core starts"),
        Duration::from_secs(30),
    )
    .expect("the fleet stop finishes");
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr={err}");
    assert!(
        err.contains("session 'aaa-unowned' has no positive server record"),
        "the earlier target really failed: {err}"
    );
    assert!(!err.contains("Broken pipe"), "stderr={err}");
    assert!(
        !rig.session_is_live(),
        "the last healthy target stopped before its report hit the pipe"
    );
    assert_eq!(
        std::fs::read(unowned.join("meta")).expect("kept meta"),
        before
    );
    assert!(rig.tmux(&["has-session", "-t", "=keepalive"]).0);
}
