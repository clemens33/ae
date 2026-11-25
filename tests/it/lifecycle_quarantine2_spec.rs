//! Frozen phase A. Exercise quota clear through a real caller and real tmux client.

use super::{Duration, Instant, Rig};
use std::path::PathBuf;

const STEM: &str = "client-muse";
const RECORD: &str = "version=1\ntool=muse\nclient=muse\nsince=1788858600\nkind=billing\nseat=ember\nreason=Billing verification failed\n";

fn plant(rig: &Rig) -> PathBuf {
    let dir = rig.home.join("vendorblock");
    std::fs::create_dir_all(&dir).expect("vendorblock directory");
    let file = dir.join("client-muse.rec");
    std::fs::write(&file, RECORD).expect("billing record");
    file
}

fn refused(rig: &Rig, result: (Option<i32>, String, String)) {
    let (code, out, err) = result;
    assert_ne!(code, Some(0), "{out}{err}");
    assert!(out.is_empty(), "{out}");
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(
        err.contains("ae quota clear")
            && ["human", "terminal", "client"]
                .iter()
                .any(|word| err.to_ascii_lowercase().contains(word)),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(rig.home.join("vendorblock/client-muse.rec"))
            .expect("refusal preserves the record"),
        RECORD
    );
}

#[test]
fn quota_clear_refuses_stamped_seats_even_with_yes() {
    let rig = Rig::new("q2seat");
    let _block = plant(&rig);
    for (slot, agent) in [("main", "lead"), ("spawned.1", "helper")] {
        for (name, value) in [("@ae_slot", slot), ("@ae_agent", agent)] {
            assert!(
                rig.tmux(&["set-option", "-p", "-t", &rig.pane, name, value])
                    .0
            );
        }
        refused(&rig, rig.run_inside(&["quota", "clear", STEM]));
        refused(&rig, rig.run_inside(&["quota", "clear", STEM, "-y"]));
    }
}

#[test]
fn quota_clear_refuses_when_the_caller_pane_cannot_be_proved() {
    let rig = Rig::new("q2doubt");
    let _block = plant(&rig);
    refused(
        &rig,
        rig.run_from(&["quota", "clear", STEM], Some((&rig.sock, "%99999"))),
    );
    refused(
        &rig,
        rig.run_with_env(
            &[("TMUX", "relative,0,0"), ("TMUX_PANE", &rig.pane)],
            &["quota", "clear", STEM, "-y"],
        ),
    );
}

#[test]
fn quota_clear_without_a_client_or_terminal_names_the_human_yes_route() {
    let rig = Rig::new("q2notty");
    let block = plant(&rig);
    let (code, out, err) = rig.run(&["quota", "clear", STEM]);
    assert_ne!(code, Some(0), "{out}{err}");
    assert!(err.contains("-y"), "{out}{err}");
    assert_eq!(
        std::fs::read_to_string(block).expect("unanswered block"),
        RECORD
    );
}

fn wait_for(test: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if test() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn quota_clear_uses_the_humans_attached_client_for_no_and_yes() {
    let rig = Rig::new("q2confirm");
    let block = plant(&rig);
    assert!(
        rig.tmux(&["set-option", "-pu", "-t", &rig.pane, "@ae_agent"])
            .0,
        "the requesting shell is not a seat"
    );
    let (controller, _client) = rig.attach_client();
    for answer in ["n", "y"] {
        std::thread::scope(|scope| {
            let running = scope.spawn(|| rig.run_inside(&["quota", "clear", STEM]));
            assert!(
                wait_for(|| {
                    let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &controller]);
                    screen.contains(STEM) && screen.contains("(y/n)")
                }),
                "the attached client must receive the clear confirmation"
            );
            assert!(block.exists(), "the request alone cannot clear the block");
            assert!(
                rig.tmux(&["send-keys", "-t", &controller, answer]).0,
                "answer on the attached client"
            );
            let result = running.join().expect("quota caller returned");
            assert!(
                wait_for(|| {
                    let (_, screen) = rig.tmux(&["capture-pane", "-p", "-t", &controller]);
                    !screen.contains("(y/n)")
                }),
                "the client consumes the answer: {result:?}"
            );
        });
        if answer == "n" {
            assert_eq!(
                std::fs::read_to_string(&block).expect("declined block"),
                RECORD
            );
        } else {
            assert!(wait_for(|| !block.exists()), "yes clears the record");
        }
    }
}
