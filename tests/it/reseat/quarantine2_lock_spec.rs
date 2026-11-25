//! Driver pin of the reseat's lock-time block check (quarantine2, lead ruling
//! 1): a block stamped in a later second than the request, while the move
//! waits for the lifecycle lock, holds the move with the original untouched.
//! A removed recheck, or one judged at the request's own instant (to which
//! the record is future-dated, so no block), would stop the seat. A block
//! that already stands is named at admission, before the move waits for the
//! lock at all: without that check a held lock answers with its own timeout.

#![allow(
    clippy::expect_used,
    reason = "fixture setup failures name the missing fact"
)]

use super::Rig;
use std::fs;
use std::io::BufRead as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REASON: &str = "Fixture vendor account suspended late";

/// Auto reseat on (so the destination's missing exit is warned about right
/// after admission), and a blocked profile that names Copilot on the
/// `ocfixture` client.
fn install_scoped_profile(rig: &Rig) {
    let path = rig.scratch.join("config");
    let config = fs::read_to_string(&path).expect("fixture config");
    let profile = format!(
        "blocked-profile = \"ocfixture {} --agent build --model github-copilot/gpt-6.1-sol\"\n\
         [clients]\nocfixture = {}\n",
        rig.scratch.join("opencode.pl").display(),
        rig.scratch.join("tools/opencode").display(),
    );
    fs::write(
        path,
        config
            .replace("[workspace]", &format!("{profile}[workspace]"))
            .replace(
                "layout = vertical\n",
                "layout = vertical\nauto_reseat = on\n",
            ),
    )
    .expect("the blocked profile");
}

/// The Copilot provider's block record, stamped `since`.
fn plant(rig: &Rig, since: u64, seat: &str, reason: &str) {
    let dir = rig.scratch.join("vendorblock");
    fs::create_dir_all(&dir).expect("block directory");
    fs::write(
        dir.join("provider-github-copilot.rec"),
        format!(
            "version=1\ntool=opencode\nclient=ocfixture\nprovider=github-copilot\n\
             since={since}\nkind=account\nseat={seat}\nreason={reason}\n"
        ),
    )
    .expect("a block record");
}

fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

fn lifecycle_lock(rig: &Rig) -> std::path::PathBuf {
    rig.scratch
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.session))
}

#[test]
fn a_standing_block_is_named_at_admission_while_the_lock_is_held() {
    const STANDING: &str = "Fixture vendor account suspended earlier";
    let rig = Rig::new("q2adm");
    let pane = rig.seat("spawned.0", "scout", "claude");
    install_scoped_profile(&rig);
    plant(&rig, epoch() - 60, "earlier", STANDING);
    let original = rig.tool_pid(&pane, "claude").expect("original tool");
    let held =
        ae::store::lock(&lifecycle_lock(&rig), Duration::ZERO).expect("fixture lifecycle lock");

    let (code, _, err) = rig.run_top(
        &rig.main_pane,
        &[
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "blocked-profile",
        ],
    );
    drop(held);

    assert_ne!(code, Some(0), "{err}");
    for part in ["provider-github-copilot", STANDING] {
        assert!(err.contains(part), "missing {part:?} in {err}");
    }
    assert!(
        !err.contains("another lifecycle operation holds"),
        "a standing block is named before the lock wait: {err}"
    );
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(original), "{err}");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude");
}

#[test]
fn a_block_stamped_while_the_move_waits_for_the_lock_holds_it() {
    let rig = Rig::new("q2late");
    let pane = rig.seat("spawned.0", "scout", "claude");
    install_scoped_profile(&rig);
    let original = rig.tool_pid(&pane, "claude").expect("original tool");
    let held =
        ae::store::lock(&lifecycle_lock(&rig), Duration::ZERO).expect("fixture lifecycle lock");

    let mut child = super::super::cli::ae()
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env("AE_HOME", &rig.scratch)
        .env_remove("AE_SENDER_OVERRIDE")
        .args([
            "reseat",
            &rig.session,
            "scout",
            "--using",
            "blocked-profile",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("reseat child");
    let stderr = child.stderr.take().expect("piped stderr");
    let (lines, seen) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            let _ = lines.send(line);
        }
    });
    let mut err = String::new();
    let until = Instant::now() + Duration::from_secs(10);
    // The cover warning is written after admission and before the lock wait.
    while !err.contains("has no auto-reseat exit") {
        let left = until.saturating_duration_since(Instant::now());
        let line = seen
            .recv_timeout(left)
            .expect("the move was admitted and waits for the lock");
        err.push_str(&line);
        err.push('\n');
    }
    std::thread::sleep(Duration::from_millis(1_200));
    plant(&rig, epoch(), "late", REASON);
    drop(held);

    let status = child.wait().expect("the move ends");
    reader.join().expect("stderr reader");
    err.extend(seen.try_iter().map(|line| format!("{line}\n")));
    assert!(!status.success(), "{err}");
    for part in ["provider-github-copilot", REASON, "Nothing was reseated"] {
        assert!(err.contains(part), "missing {part:?} in {err}");
    }
    assert_eq!(rig.tool_pid(&pane, "claude"), Some(original), "{err}");
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-claude");
}
