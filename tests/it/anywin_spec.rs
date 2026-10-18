//! Frozen multi-window S2 acceptance. Oracle: brief-anywin invariants 1-9
//! and DESIGN-r3 S2. Real app processes, kernel locks, admission, delivery
//! and journal-derived lanes on a private tmux server. No submit-path mock.
//!
//! S1 recovery stays pinned by `draftclaim_spec` (only the obsolete
//! composing-held witnesses migrate). Existing flight bound and restart
//! tests remain behavioral gates; the production quit constant is pinned
//! here too, so changing the default cannot silently weaken that gate.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use super::app_mode::writing;
use super::app_writer_spec::{Hold, Rig};
use super::draftclaim_spec::assert_no_kept_draft;
use ae::events::Event;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

const WRITER: &str = ".console-writer.lock";
const ADMISSION: &str = ".console-admission.lock";
// Well below store::LOCK_WAIT (5 s), including real terminal dispatch.
const PROMPT: Duration = Duration::from_secs(3);

fn lock(path: &Path) -> File {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .expect("fixture lock is regular");
    file.try_lock().expect("fixture takes free kernel lock");
    file
}

fn is_free(rig: &Rig, name: &str) -> bool {
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(rig.tool.dir.join(name))
        .expect("kernel witness");
    match file.try_lock() {
        Ok(()) => true,
        Err(TryLockError::WouldBlock) => false,
        Err(why) => panic!("kernel witness failed: {why}"),
    }
}

fn compose(rig: &Rig, pane: &str) {
    rig.raw(pane, "i");
    let screen = rig.wait(pane, "entry reaches composing or a visible refusal", |s| {
        writing(s) || s.contains("not writing:")
    });
    assert!(writing(&screen), "S2 every window may compose: {screen}");
}

fn prompt(rig: &Rig, pane: &str, began: Instant, why: &str, met: impl Fn(&str) -> bool) -> String {
    loop {
        let screen = rig.screen(pane);
        if met(&screen) {
            assert!(began.elapsed() < PROMPT, "{why} waited for a lock");
            return screen;
        }
        assert!(began.elapsed() < PROMPT, "{why}: {screen}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn refused(rig: &Rig, pane: &str, draft: &str) {
    let message = format!("another window is sending to {}", rig.home);
    let before = rig.screen(pane).matches(&message).count();
    let began = Instant::now();
    rig.key(pane, "Enter");
    let screen = prompt(
        rig,
        pane,
        began,
        "S2 Enter refuses without LOCK_WAIT",
        |s| {
            s.matches(&message).count() > before
                && s.contains("Enter retries")
                && writing(s)
                && rig.draft_row(s).contains(draft)
        },
    );
    assert!(writing(&screen), "contention keeps the composer usable");
}

/// A target lock stalls the real delivery AFTER admission. A live app's
/// sending hint plus both independently held locks attest an actual Flight.
fn flight(rig: &Rig, pane: &str, text: &str) -> File {
    let locks = rig.root.join("sessions/.locks");
    fs::create_dir_all(&locks).expect("private target locks");
    let pane_id: String = rig
        .tool
        .pane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let held = lock(&locks.join(format!("send-lock-{pane_id}")));
    rig.typed(pane, text);
    rig.key(pane, "Enter");
    rig.wait(pane, "GUARD real delivery entered Flight", |s| {
        s.contains("sending to lead") && !is_free(rig, WRITER) && !is_free(rig, ADMISSION)
    });
    assert!(
        rig.tool.submitted().is_empty(),
        "target lock still holds delivery"
    );
    held
}

fn complete(rig: &Rig, pane: &str, text: &str, count: usize) {
    rig.wait(
        pane,
        "exact ask finished, composer cleared, lease released",
        |s| {
            let asks = rig.asks();
            asks.len() == count
                && asks
                    .last()
                    .is_some_and(|e| e.summary.as_deref() == Some(text))
                && rig.tool.submitted().contains(text)
                && rig.draft_row(s).trim().is_empty()
                && is_free(rig, WRITER)
                && is_free(rig, ADMISSION)
        },
    );
}

fn cancel_events(rig: &Rig) -> Vec<Event> {
    rig.tool
        .events()
        .lines()
        .filter_map(|row| Event::parse_line(row).ok())
        .filter(|e| e.actor == "console:local" && e.action == "cancel")
        .collect()
}

#[test]
fn controls_real_flight_holds_both_locks_then_releases_on_delivery() {
    let rig = Rig::new("awcontrol");
    let a = rig.console(false);
    compose(&rig, &a);
    let held = flight(&rig, &a, "aw-control");
    let place = rig.tmux(&[
        "display-message",
        "-p",
        "-t",
        &a,
        "#{session_name} #{window_index}",
    ]);
    assert_eq!(
        fs::read_to_string(rig.tool.dir.join(WRITER)).expect("holder record"),
        format!("1 {}\n", place.trim()),
        "Flight holds the submitting pane's place"
    );
    drop(held);
    rig.wait(&a, "GUARD real delivery finishes on baseline too", |s| {
        rig.asks().len() == 1
            && rig.tool.submitted().contains("aw-control")
            && rig.draft_row(s).trim().is_empty()
            && is_free(&rig, ADMISSION)
    });
    rig.key(&a, "Escape");
    rig.free();
    assert!(
        fs::read(rig.tool.dir.join(WRITER))
            .expect("holder after release")
            .is_empty()
    );
}

#[test]
fn two_windows_compose_without_a_lease_and_b_sends_while_a_keeps_typing() {
    let rig = Rig::new("awcompose");
    let (a, _) = rig.outside("a");
    let (b, _) = rig.outside("b");
    compose(&rig, &a);
    rig.typed(&a, "aw-A-draft");
    assert!(is_free(&rig, WRITER), "S2 composing takes no writer lease");
    assert!(
        is_free(&rig, ADMISSION),
        "composing takes no admission lock"
    );
    compose(&rig, &b);
    rig.typed(&b, "aw-B-send");
    assert!(is_free(&rig, WRITER), "two composers hold no lease");
    assert_no_kept_draft(&rig.tool.dir);
    rig.key(&b, "Enter");
    complete(&rig, &b, "aw-B-send", 1);
    rig.typed(&a, "-still-A");
    assert!(
        rig.draft_row(&rig.screen(&a))
            .contains("aw-A-draft-still-A")
    );
    assert!(rig.draft_row(&rig.screen(&b)).trim().is_empty());
    assert_no_kept_draft(&rig.tool.dir);
}

#[test]
fn b_composes_during_a_flight_refuses_at_once_keeps_text_and_enter_retries() {
    let rig = Rig::new("awflight");
    let (a, _) = rig.outside("a");
    let (b, _) = rig.outside("b");
    compose(&rig, &a);
    let held = flight(&rig, &a, "aw-flight-A");
    compose(&rig, &b);
    rig.typed(&b, "aw-flight-B");
    refused(&rig, &b, "aw-flight-B");
    assert!(rig.asks().is_empty(), "contended Enter was not admitted");
    // Contention never enters the old HELD mode that swallowed later keys.
    rig.typed(&b, "-edited");
    drop(held);
    complete(&rig, &a, "aw-flight-A", 1);
    rig.key(&b, "Enter");
    complete(&rig, &b, "aw-flight-B-edited", 2);
    let bodies: Vec<_> = rig.asks().into_iter().map(|e| e.summary).collect();
    assert_eq!(
        bodies,
        [
            Some("aw-flight-A".into()),
            Some("aw-flight-B-edited".into())
        ]
    );
    assert_no_kept_draft(&rig.tool.dir);
}

#[test]
fn send_and_close_never_wait_for_either_lock_and_keep_the_refused_command() {
    for (tag, name) in [("awadmit", ADMISSION), ("awold", WRITER)] {
        let rig = Rig::new(tag);
        let (b, _) = rig.outside("b");
        let held = lock(&rig.tool.dir.join(name));
        if name == WRITER {
            fs::write(rig.tool.dir.join(WRITER), "1 work 0\n").expect("older core holder");
        }
        compose(&rig, &b);
        rig.typed(&b, "aw-busy-send");
        refused(&rig, &b, "aw-busy-send");
        if name == WRITER {
            assert!(
                rig.screen(&b).contains("held at work:0"),
                "mixed-core holder named"
            );
        } else {
            assert!(
                is_free(&rig, WRITER),
                "failed admission releases acquired writer lease"
            );
        }
        assert_no_kept_draft(&rig.tool.dir);
        rig.key(&b, "C-u");
        rig.typed(&b, "/close");
        refused(&rig, &b, "/close");
        assert!(cancel_events(&rig).is_empty());
        assert!(rig.asks().is_empty() && rig.tool.submitted().is_empty());
        assert_no_kept_draft(&rig.tool.dir);
        drop(held);
        // Still the same command: Enter retries, and reaches the journal's
        // ordinary no-open-ask answer only after both locks are free.
        rig.key(&b, "Enter");
        rig.wait(&b, "close retry reaches admission", |s| {
            s.contains("no open ask to close")
        });
        rig.key(&b, "C-u");
        rig.typed(&b, "aw-after-lock");
        rig.key(&b, "Enter");
        complete(&rig, &b, "aw-after-lock", 1);
    }
}

#[test]
fn owner_chat_refuses_send_and_close_during_an_app_flight_then_sends() {
    // Separate terminals cannot mistake a previous identical refusal for
    // the current one in the chat's current view.
    for (tag, command) in [("awchatsend", "aw-chat-busy"), ("awchatclose", "/close")] {
        let rig = Rig::new(tag);
        let chat = rig.console(true);
        let (a, _) = rig.outside("a");
        compose(&rig, &a);
        let held = flight(&rig, &a, "aw-chat-app");
        rig.raw(&chat, command);
        rig.wait(&chat, "GUARD owner chat command composed", |s| {
            s.contains(&format!("to lead> {command}"))
        });
        let began = Instant::now();
        rig.key(&chat, "Enter");
        prompt(
            &rig,
            &chat,
            began,
            "owner chat probes lease before admission",
            |s| s.contains("an ae app is writing to"),
        );
        assert!(cancel_events(&rig).is_empty() && rig.asks().is_empty());
        drop(held);
        complete(&rig, &a, "aw-chat-app", 1);
        rig.key(&chat, "C-u");
        rig.raw(&chat, "aw-chat-after");
        rig.key(&chat, "Enter");
        rig.wait(&chat, "owner chat sends after Flight", |_| {
            rig.asks().len() == 2 && rig.tool.submitted().contains("aw-chat-after")
        });
    }
}

#[test]
fn close_in_another_window_closes_the_global_newest_ask_and_both_lanes_name_it() {
    let rig = Rig::new("awclose");
    let (a, _) = rig.outside("a");
    let (b, _) = rig.outside("b");
    compose(&rig, &a);
    rig.typed(&a, "aw-older-question");
    rig.key(&a, "Enter");
    complete(&rig, &a, "aw-older-question", 1);
    rig.typed(&a, "aw-newest-excerpt");
    rig.key(&a, "Enter");
    complete(&rig, &a, "aw-newest-excerpt", 2);
    let id = rig.asks()[1].reference.clone().expect("newest request id");
    compose(&rig, &b);
    rig.typed(&b, "/close");
    rig.key(&b, "Enter");
    for pane in [&a, &b] {
        rig.wait(
            pane,
            "shared closed-ask row names id, recipient and excerpt",
            |s| {
                let rows: Vec<_> = s.lines().collect();
                rows.iter().enumerate().any(|(at, row)| {
                    if !row.contains("closed an ask") {
                        return false;
                    }
                    let closed = rows[at..rows.len().min(at + 4)].join("\n");
                    closed.contains(&id)
                        && closed.contains("lead")
                        && closed.contains("aw-newest-excerpt")
                })
            },
        );
    }
    let cancelled = cancel_events(&rig);
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].reference.as_deref(), Some(id.as_str()));
    assert_eq!(rig.asks().len(), 2, "close is not a third ask");
}

#[test]
fn five_open_is_session_wide_across_both_composers_and_close_reopens_capacity() {
    let rig = Rig::new("awcap");
    let (a, _) = rig.outside("a");
    let (b, _) = rig.outside("b");
    compose(&rig, &a);
    compose(&rig, &b);
    for at in 1..=5 {
        let pane = if at % 2 == 1 { &a } else { &b };
        let body = format!("aw-cap-{at}");
        rig.typed(pane, &body);
        rig.key(pane, "Enter");
        complete(&rig, pane, &body, at);
    }
    rig.typed(&b, "aw-sixth");
    rig.key(&b, "Enter");
    rig.wait(&b, "shared five-open cap refuses a sixth ask", |s| {
        s.contains("5 requests open") && rig.draft_row(s).contains("aw-sixth")
    });
    assert_eq!(rig.asks().len(), 5);
    rig.typed(&a, "/close");
    rig.key(&a, "Enter");
    rig.wait(&a, "global close frees one shared place", |_| {
        cancel_events(&rig).len() == 1
    });
    rig.key(&b, "Enter");
    complete(&rig, &b, "aw-sixth", 6);
}

#[test]
fn every_window_reproves_its_original_incarnation_pair_and_recorded_server_at_submit() {
    for (tag, replacement) in [
        ("awstaleid", "id"),
        ("awstalepair", "pair"),
        ("awstaleserver", "server"),
    ] {
        let rig = Rig::new(tag);
        let (a, ga) = rig.outside("a");
        let (b, gb) = rig.outside("b");
        compose(&rig, &a);
        compose(&rig, &b);
        rig.typed(&a, "aw-stale-A");
        rig.typed(&b, "aw-stale-B");
        let a_read = Hold::new(&ga, "@world");
        let b_read = Hold::new(&gb, "@world");
        a_read.held();
        b_read.held();
        let meta_path = rig.tool.dir.join("meta");
        let meta = fs::read_to_string(&meta_path).expect("fixture meta");
        let (changed, reason) = match replacement {
            "id" => (
                meta.replace("0199c0de-cccc", "0199c0de-bbbb"),
                "the session was replaced or renamed",
            ),
            "pair" => (
                meta.replace("seat.worker.0=colead", "seat.worker.0=replacement"),
                "the lead pair changed since this chat opened",
            ),
            _ => (
                meta.lines()
                    .filter(|row| !row.starts_with("tmux_server"))
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
                "records no usable tmux server",
            ),
        };
        assert_ne!(changed, meta, "GUARD routing fact was actually replaced");
        fs::write(meta_path, changed).expect("replace live routing facts");
        for pane in [&a, &b] {
            rig.key(pane, "Enter");
            rig.wait(pane, "stale window refuses before writing", |s| {
                s.contains("refused") && s.contains(reason)
            });
        }
        assert!(rig.asks().is_empty() && rig.tool.submitted().is_empty());
        assert_no_kept_draft(&rig.tool.dir);
        assert!(is_free(&rig, WRITER) && is_free(&rig, ADMISSION));
    }
}

#[test]
fn controls_quit_waits_for_the_actual_flight_and_the_default_bound_stays_four_minutes() {
    assert!(
        include_str!("../../src/app.rs")
            .contains("const FLIGHT_QUIT_BOUND: Duration = Duration::from_mins(4);"),
        "default quit bound remains unchanged; behavioral timeout test is app::tests::flight"
    );
    let rig = Rig::new("awquit");
    let (a, _) = rig.outside("a");
    rig.tmux(&["set-window-option", "-t", &a, "remain-on-exit", "on"]);
    compose(&rig, &a);
    let held = flight(&rig, &a, "aw-quit-delivery");
    rig.key(&a, "Escape");
    rig.wait(&a, "browsing keeps the Flight lease", |s| {
        !writing(s) && s.contains("? keys")
    });
    assert!(!is_free(&rig, WRITER));
    rig.raw(&a, "qq");
    rig.wait(&a, "quit names the delivery it waits for", |s| {
        s.contains("finishing the delivery to lead")
    });
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &a, "#{pane_dead}"])
            .trim(),
        "0"
    );
    drop(held);
    rig.wait(&a, "quit completes after delivery", |_| {
        rig.tmux(&["display-message", "-p", "-t", &a, "#{pane_dead}"])
            .trim()
            == "1"
    });
    assert!(rig.tool.submitted().contains("aw-quit-delivery"));
    assert!(is_free(&rig, WRITER) && is_free(&rig, ADMISSION));
}
