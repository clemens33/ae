//! The board pass's source selection, against a plain world: the seat the
//! census holds is a source only while a running orchestrator stands in its
//! own directory, and a running namesake elsewhere is told once per episode.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "these unit tests own their scratch files"
)]

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use crate::telegram::routing::{RunningSession, Seat, World};
use crate::telegram::tests::Fake;

use super::super::board_pass;

struct Plain {
    seat: Seat,
    running: RefCell<Vec<RunningSession>>,
}

impl World for Plain {
    fn running(&self) -> Vec<RunningSession> {
        self.running.borrow().clone()
    }

    fn seat(&self) -> Seat {
        self.seat.clone()
    }
}

fn session(name: &str, dir: &Path, meta_agent: bool) -> RunningSession {
    RunningSession {
        name: name.to_owned(),
        dir: dir.to_owned(),
        session_id: String::new(),
        meta_agent,
        main: None,
        agents: Vec::new(),
        pair: Vec::new(),
        last_active: None,
    }
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(tag: &str) -> Scratch {
    let path = Path::new("/tmp").join(format!(
        "ae-desk-ns-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("held")).expect("held directory");
    fs::create_dir_all(path.join("state")).expect("state directory");
    Scratch(path)
}

const LINE: &str = "desk skipped: running session seat is at ";

fn told(err: &[u8]) -> usize {
    String::from_utf8_lossy(err).matches(LINE).count()
}

#[test]
fn a_running_namesake_elsewhere_is_told_once_per_episode_and_the_seat_ends_it() {
    let dir = scratch("episode");
    let held = dir.0.join("held");
    let other = dir.0.join("elsewhere");
    let fake = Fake::start(Vec::new());
    let api = fake.api();
    let mut board = crate::telegram::desk::Machine::new(dir.0.join("state"));
    let mut err = Vec::new();
    let world = Plain {
        seat: Seat::Held {
            name: "seat".to_owned(),
            dir: held.clone(),
        },
        running: RefCell::new(vec![session("seat", &other, true)]),
    };
    board_pass(&api, &world, &mut board, &mut err);
    board_pass(&api, &world, &mut board, &mut err);
    assert_eq!(told(&err), 1, "one line for the episode");
    let text = String::from_utf8_lossy(&err).into_owned();
    assert!(text.contains(&other.display().to_string()), "{text}");
    assert!(text.contains(&held.display().to_string()), "{text}");
    assert!(fake.requests().is_empty(), "a namesake is no source");

    world.running.borrow_mut().clear();
    board_pass(&api, &world, &mut board, &mut err);
    world
        .running
        .borrow_mut()
        .push(session("seat", &other, true));
    board_pass(&api, &world, &mut board, &mut err);
    assert_eq!(told(&err), 2, "a new episode is told again");

    *world.running.borrow_mut() = vec![session("seat", &held, true)];
    board_pass(&api, &world, &mut board, &mut err);
    *world.running.borrow_mut() = vec![session("seat", &other, true)];
    board_pass(&api, &world, &mut board, &mut err);
    assert_eq!(told(&err), 3, "the seat itself ends an episode");

    for gone in [Seat::Vacant, Seat::Unproven("two claims".to_owned())] {
        let away = Plain {
            seat: gone,
            running: RefCell::new(vec![session("seat", &other, true)]),
        };
        board_pass(&api, &away, &mut board, &mut err);
        board_pass(&api, &world, &mut board, &mut err);
    }
    assert_eq!(told(&err), 5, "a seat that is not held ends an episode");
}

#[test]
fn only_a_running_orchestrator_in_the_seat_directory_is_a_source() {
    let dir = scratch("source");
    let held = dir.0.join("held");
    fs::write(
        held.join("desk-record"),
        "ae-desk 1\nupdated 2026-11-20T08:42:12Z\nR6\tRun the checks\n",
    )
    .expect("a valid board");
    let fake = Fake::start(Vec::new());
    let api = fake.api();
    let mut board = crate::telegram::desk::Machine::new(dir.0.join("state"));
    let mut err = Vec::new();
    let held_seat = Seat::Held {
        name: "seat".to_owned(),
        dir: held.clone(),
    };
    for (world_seat, running) in [
        (held_seat.clone(), vec![session("seat", &held, false)]),
        (held_seat.clone(), Vec::new()),
        (Seat::Vacant, vec![session("seat", &held, true)]),
        (
            Seat::Unproven("two claims".to_owned()),
            vec![session("seat", &held, true)],
        ),
    ] {
        let world = Plain {
            seat: world_seat,
            running: RefCell::new(running),
        };
        board_pass(&api, &world, &mut board, &mut err);
    }
    assert_eq!(told(&err), 0, "none of these is a namesake elsewhere");
    assert!(fake.requests().is_empty());
    assert!(
        !dir.0.join("state/desk-pin").exists(),
        "a pass that is no source reads no checkpoint"
    );
}

#[test]
fn a_running_seat_with_no_board_makes_no_call_and_leaves_no_file() {
    let dir = scratch("noboard");
    let held = dir.0.join("held");
    let fake = Fake::start(Vec::new());
    let api = fake.api();
    let mut board = crate::telegram::desk::Machine::new(dir.0.join("state"));
    let mut err = Vec::new();
    let world = Plain {
        seat: Seat::Held {
            name: "seat".to_owned(),
            dir: held.clone(),
        },
        running: RefCell::new(vec![session("seat", &held, true)]),
    };
    board_pass(&api, &world, &mut board, &mut err);
    assert!(fake.requests().is_empty());
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    assert!(!dir.0.join("state/desk-pin").exists());
}

#[test]
fn a_damaged_board_of_the_running_seat_makes_no_call_and_is_told() {
    let dir = scratch("damaged");
    let held = dir.0.join("held");
    fs::write(held.join("desk-record"), "ae-desk 1\nupdated damaged\n").expect("damaged board");
    let fake = Fake::start(Vec::new());
    let api = fake.api();
    let mut board = crate::telegram::desk::Machine::new(dir.0.join("state"));
    let mut err = Vec::new();
    let world = Plain {
        seat: Seat::Held {
            name: "seat".to_owned(),
            dir: held.clone(),
        },
        running: RefCell::new(vec![session("seat", &held, true)]),
    };
    board_pass(&api, &world, &mut board, &mut err);
    assert!(fake.requests().is_empty());
    let text = String::from_utf8_lossy(&err).into_owned();
    assert!(text.contains("the board is unreadable"), "{text}");
    assert!(text.contains("the pin keeps its last text"), "{text}");
}
