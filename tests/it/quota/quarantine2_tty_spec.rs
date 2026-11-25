//! The human's own terminal outside tmux confirms `ae quota clear` (quarantine2,
//! lead ruling 2): the command runs as a private pane's process with both tmux
//! markers removed, so its stdin is a real terminal and its caller reads as no
//! tmux caller at all. `n` and an end of input clear nothing; `y` clears.

#![allow(
    clippy::expect_used,
    reason = "fixture setup failures name the missing fact"
)]

use super::super::seat_relaunch::Rig;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STEM: &str = "client-ocfixture";

fn plant(rig: &Rig) -> PathBuf {
    let dir = rig.scratch.join("vendorblock");
    std::fs::create_dir_all(&dir).expect("block directory");
    let block = dir.join(format!("{STEM}.rec"));
    std::fs::write(
        &block,
        "version=1\ntool=opencode\nclient=ocfixture\nsince=1788858600\nkind=account\n\
         seat=earlier\nreason=Fixture vendor account suspended\n",
    )
    .expect("standing record");
    block
}

/// Start `ae quota clear` in its own pane of the rig's server, outside tmux
/// by its environment, and wait for its question; the log is its output.
fn ask(rig: &Rig, tag: &str) -> PathBuf {
    let script = rig.scratch.join(format!("runner-{tag}.sh"));
    let log = rig.scratch.join(format!("clear-{tag}"));
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nenv -u TMUX -u TMUX_PANE AE_HOME={home} {ae} quota clear {STEM} > {log} 2>&1\n\
             echo \"EXIT:$?\" >> {log}\nsleep 30\n",
            home = rig.scratch.display(),
            ae = env!("CARGO_BIN_EXE_ae"),
            log = log.display(),
        ),
    )
    .expect("the runner");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("an executable runner");
    }
    let session = format!("runner-{tag}");
    assert!(
        rig.tmux(&[
            "new-session",
            "-d",
            "-s",
            &session,
            &script.display().to_string()
        ])
        .0,
        "the runner pane starts"
    );
    let said = wait(&log, "[y/N]");
    assert!(
        said.contains(&format!("Clear {STEM}? [y/N]")),
        "the terminal is asked before anything is cleared: {said:?}"
    );
    log
}

fn wait(log: &Path, needle: &str) -> String {
    let mut said = String::new();
    for _ in 0..200 {
        said = std::fs::read_to_string(log).unwrap_or_default();
        if said.contains(needle) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    said
}

#[test]
fn the_humans_terminal_answers_no_end_of_input_and_yes() {
    let rig = Rig::new("q2tty");
    let block = plant(&rig);
    for (tag, keys) in [("no", &["n", "Enter"][..]), ("eof", &["C-d"][..])] {
        let log = ask(&rig, tag);
        let mut send = vec!["send-keys", "-t"];
        let session = format!("runner-{tag}");
        send.push(&session);
        send.extend_from_slice(keys);
        assert!(rig.tmux(&send).0);
        let said = wait(&log, "EXIT:");
        assert!(said.contains("ae quota: not cleared"), "{tag}: {said:?}");
        assert!(said.contains("EXIT:1"), "{tag}: {said:?}");
        assert!(block.exists(), "{tag}: the block stands");
    }
    let log = ask(&rig, "yes");
    assert!(rig.tmux(&["send-keys", "-t", "runner-yes", "y", "Enter"]).0);
    let said = wait(&log, "EXIT:");
    assert!(
        said.contains(&format!("ae quota: cleared the block on {STEM}")),
        "{said:?}"
    );
    assert!(said.contains("EXIT:0"), "{said:?}");
    assert!(!block.exists(), "the human's yes cleared the block");
}
