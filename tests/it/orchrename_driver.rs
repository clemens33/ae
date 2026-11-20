//! Driver regressions for the bound rename and the lifecycle seat guard at the
//! public binary entry on private tmux: each declaration the bound rename
//! admits or waives, an ordinary seat's refusal whose row cannot be written,
//! a pre-grammar target, a source another record takes while the rename
//! waits on its lock, and the lock order of a stopped bound rename.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures read their private session records and lifecycles"
)]

use super::orchrename_rename_spec::{declare, dispatcher, live, owner_input, refuses, succeeds};
use super::relay_chat_spec::{Out, events, words};
use super::renametell_routing_spec::{A, B, Rig};
use std::time::{Duration, Instant};

const TARGET: &str = "drive-target";
const SEAT: &str = "drive-seat";

fn bound(rig: &Rig, new: &str, pane: &str, input: Option<&str>) -> Out {
    let mut args = vec!["rename", TARGET, new, "--expect-uuid", B];
    if let Some(input) = input {
        args.extend(["--owner-input", input]);
    }
    rig.run(&args, Some(pane))
}

/// The owner puts the session back under its old name. The name it leaves
/// stays a rename alias, which no bound rename reclaims, so each round takes
/// a fresh new name.
fn back(rig: &Rig, new: &str) {
    assert!(live(rig, new) && !live(rig, TARGET), "{new}");
    succeeds(&rig.run(&["rename", new, TARGET], None));
}

/// `done` and `waiting-user` admit the bound rename; `working`,
/// `waiting-agent` and `blocked` each refuse it with the state named, until
/// one genuine owner input that names `rename` and the session waives it.
#[test]
fn each_busy_declaration_refuses_a_bound_rename_until_one_owner_input_waives_it() {
    let rig = Rig::new("ornd-states");
    let caller = dispatcher(&rig);
    let [lead, peer] = rig.create(TARGET, B);
    declare(&rig, TARGET, &lead, "done");
    declare(&rig, TARGET, &peer, "waiting-user");
    succeeds(&bound(&rig, "drive-idle", &caller, None));
    back(&rig, "drive-idle");

    for (update, state) in [(731, "working"), (732, "waiting-agent"), (733, "blocked")] {
        let new = format!("drive-{state}");
        declare(&rig, TARGET, &peer, state);
        let blocked = bound(&rig, &new, &caller, None);
        refuses(&blocked);
        assert!(
            words(&blocked).contains(state),
            "{state}: {}",
            words(&blocked)
        );
        assert!(live(&rig, TARGET) && !live(&rig, &new), "{state}");
        let approved = owner_input(&rig, &format!("rename {TARGET}"), update);
        succeeds(&bound(&rig, &new, &caller, Some(&approved)));
        back(&rig, &new);
    }
}

/// An ordinary seat's rename of another session still refuses, exit 2, when
/// its own record is in another lifecycle operation, and says the row was not
/// written; once the record is free, the same refusal writes its one row.
#[test]
fn an_ordinary_seats_refusal_whose_row_cannot_be_written_still_refuses_and_says_so() {
    let rig = Rig::new("ornd-unjournaled");
    let [caller, _] = rig.create(SEAT, A);
    rig.create(TARGET, B);
    let new = "drive-topic";
    let refused_rows = || {
        events(&rig.dir(SEAT))
            .into_iter()
            .filter(|row| row.action == "lifecycle-refused")
            .count()
    };
    let lock = rig
        .root
        .join("sessions")
        .join(format!(".lifecycle.{SEAT}.lock"));
    let hold = ae::store::lock(&lock, Duration::ZERO).expect("hold the seat's own record");
    let journal = events(&rig.dir(SEAT));

    let out = rig.run(&["rename", TARGET, new], Some(&caller));
    assert_eq!(out.0, Some(2), "{}", words(&out));
    assert!(
        out.2.contains(&format!(
            "the refusal could not be journaled in session '{SEAT}'"
        )),
        "{}",
        words(&out)
    );
    assert!(
        words(&out).contains(&format!("ae rename {TARGET} {new}")),
        "{}",
        words(&out)
    );
    assert_eq!(
        events(&rig.dir(SEAT)),
        journal,
        "no row while the record is held"
    );
    assert!(live(&rig, TARGET) && !live(&rig, new));
    assert!(!rig.dir(new).exists());

    drop(hold);
    let again = rig.run(&["rename", TARGET, new], Some(&caller));
    assert_eq!(again.0, Some(2), "{}", words(&again));
    assert_eq!(refused_rows(), 1, "{}", words(&again));
    assert!(live(&rig, TARGET) && !live(&rig, new));
}

/// An ordinary seat's rename of another session that runs under a
/// pre-grammar name, which the rename itself still reaches, is refused like
/// any other session's, with its one row in the seat's own record.
#[test]
fn an_ordinary_seats_rename_of_a_pre_grammar_session_is_refused_with_one_row() {
    let rig = Rig::new("ornd-legacy");
    let [caller, _] = rig.create(SEAT, A);
    // A leading '_' is outside the grammar, and tmux runs the session.
    let legacy = "_drive-legacy";
    rig.create(legacy, B);
    let new = "drive-fresh";

    let out = rig.run(&["rename", legacy, new], Some(&caller));
    assert_eq!(out.0, Some(2), "{}", words(&out));
    assert!(
        words(&out).contains(&format!("ae rename {legacy} {new}")),
        "{}",
        words(&out)
    );
    let refused: Vec<_> = events(&rig.dir(SEAT))
        .into_iter()
        .filter(|row| row.action == "lifecycle-refused")
        .collect();
    assert_eq!(refused.len(), 1, "{}", words(&out));
    assert!(live(&rig, legacy) && !live(&rig, new));
    assert!(rig.dir(legacy).is_dir() && !rig.dir(new).exists());
}

/// An ordinary seat's rename of its own session, plain and bound, waits on
/// the source's lifecycle lock while another session's record takes the
/// name; under the lock it is refused as the gate refuses another session:
/// nothing is renamed, and no row reaches the record that is not the seat's.
#[test]
fn a_seats_own_rename_whose_source_is_replaced_while_it_waits_is_refused_under_the_lock() {
    for form in ["plain", "bound"] {
        replaced_while_waiting(form);
    }
}

/// One form of the replacement case. The bound form names the uuid the
/// replacement records.
fn replaced_while_waiting(form: &str) {
    let rig = Rig::new(&format!("ornd-replaced-{form}"));
    let [caller, _] = rig.create(SEAT, A);
    // Sorted before the seat's name: the rename holds this lock first, then
    // waits on the source's.
    let new = "drive-a-moved";
    assert!(new < SEAT);
    let sessions = rig.root.join("sessions");
    let lock_of = |name: &str| sessions.join(format!(".lifecycle.{name}.lock"));
    let source = ae::store::lock(&lock_of(SEAT), Duration::ZERO).expect("hold the source's record");
    let mut args = vec!["rename", SEAT, new];
    if form == "bound" {
        args.extend(["--expect-uuid", B]);
    }
    std::thread::scope(|scope| {
        let renaming = scope.spawn(|| rig.run(&args, Some(&caller)));
        let waiting = (0..1000).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            ae::store::lock(&lock_of(new), Duration::ZERO).is_err()
        });
        assert!(
            waiting,
            "the rename never passed the gate to the source's lock"
        );
        let meta = rig.dir(SEAT).join("meta");
        let bytes = std::fs::read_to_string(&meta).expect("the source's meta");
        assert!(bytes.contains(A), "{bytes}");
        std::fs::write(&meta, bytes.replace(A, B))
            .expect("another session's record takes the name");
        // The live session claims the replacement's uuid as well.
        rig.tmux(&["set-option", "-t", SEAT, "@ae_session_uuid", B]);
        let journal = events(&rig.dir(SEAT));
        drop(source);

        let out = renaming.join().expect("the rename returns");
        assert_eq!(out.0, Some(2), "{form}: {}", words(&out));
        assert!(
            words(&out).contains("runs no lifecycle form on another session"),
            "{}",
            words(&out)
        );
        assert!(
            out.2.contains(&format!(
                "the refusal could not be journaled in session '{SEAT}'"
            )),
            "{}",
            words(&out)
        );
        assert_eq!(
            events(&rig.dir(SEAT)),
            journal,
            "no row in a record that is not the seat's"
        );
        assert!(live(&rig, SEAT) && !live(&rig, new));
        assert!(!rig.dir(new).exists());
    });
}

/// A bound rename of a stopped session holds the event lock over its pending
/// read and its moves, and releases it before the meta step: a publisher that
/// holds the meta lock and then waits on the event lock, the order an archive
/// publisher takes them in, gets the event lock, and the rename completes.
#[test]
fn a_stopped_bound_rename_releases_the_event_lock_before_it_takes_the_meta_lock() {
    let rig = Rig::new("ornd-lockorder");
    let caller = dispatcher(&rig);
    rig.create(TARGET, B);
    succeeds(&rig.run(&["stop", TARGET], None));
    assert!(!live(&rig, TARGET));
    let new = "drive-stopped";
    let meta = ae::store::lock(
        &ae::store::open(&rig.dir(TARGET)).meta_lock(),
        Duration::ZERO,
    )
    .expect("the publisher holds the meta lock");
    let moved = rig.dir(new);
    let events = ae::store::lock_path(&ae::store::open(&moved).events_path());
    std::thread::scope(|scope| {
        let publisher = scope.spawn(move || {
            let until = Instant::now() + Duration::from_secs(30);
            while !moved.exists() {
                assert!(Instant::now() < until, "the state directory never moved");
                std::thread::sleep(Duration::from_millis(5));
            }
            let got = ae::store::lock(&events, Duration::from_secs(10)).is_ok();
            drop(meta);
            got
        });
        let out = rig.run(&["rename", TARGET, new, "--expect-uuid", B], Some(&caller));
        let got = publisher.join().expect("the publisher returns");
        assert!(got, "the rename kept the event lock: {}", words(&out));
        succeeds(&out);
    });
    assert!(rig.dir(new).is_dir() && !rig.dir(TARGET).exists());
}
