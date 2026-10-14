//! Independent live menu acceptance, child of `app_open_live_spec.rs`.
//! Existing private Rig only; real modal, detached Stop and clients.

use super::*;

fn owned(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    rig.tmux(&["set-environment", "-t", HOME, "AE_SESSION", "1"]);
    rig
}

fn menu(rig: &Rig, app: &str, name: &str) {
    rig.raw(app, "m");
    rig.wait_screen(app, "painted menu", |s| {
        s.contains(name) && s.contains("Stop")
    });
}

fn question(rig: &Rig, app: &str, name: &str) -> String {
    menu(rig, app, name);
    rig.raw(app, "s");
    rig.wait_screen(app, "named Stop question", |s| {
        s.contains(name) && s.contains("Cancel") && s.contains("Stop") && s.contains("resume")
    })
}

fn live(rig: &Rig, name: &str) -> bool {
    let args = vec![
        "-S".to_owned(),
        rig.socket.display().to_string(),
        "has-session".to_owned(),
        "-t".to_owned(),
        format!("={name}"),
    ];
    super::super::phase2::run_tmux(&args, rig.scratch.path()).0
}

fn events(rig: &Rig, name: &str) -> String {
    fs::read_to_string(rig.scratch.join("sessions").join(name).join("events.jsonl"))
        .expect("fixture journal")
}

fn owner_gone(rig: &Rig) {
    let out = rig.scratch.join("owner-ps");
    let err = rig.scratch.join("owner-ps-err");
    let invocation = super::super::parity::Invocation::new("ps")
        .arg("-A")
        .arg("-o")
        .arg("args=");
    Rig::wait("the exact scratch-bound detached owner exited", || {
        let status =
            super::super::parity::capture::raw::run(&invocation, rig.scratch.path(), &out, &err)
                .expect("scoped owner process observation");
        assert!(matches!(
            status.outcome(),
            super::super::parity::capture::ExitOutcome::Code(0)
        ));
        !fs::read_to_string(&out)
            .expect("process argv")
            .lines()
            .any(|line| {
                line.contains("_stop --supervise")
                    && line.contains(&rig.socket.display().to_string())
            })
    });
}

/// A started terminal on a second private server, outside the app's fleet.
fn tty_viewer(rig: &Rig) -> (PathBuf, String) {
    let host = rig.scratch.join("hv");
    rig.at(
        &host,
        &[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "v",
            "-x",
            "160",
            "-y",
            "45",
            "env",
            "-u",
            "TMUX",
            "-u",
            "TMUX_PANE",
            "TERM=xterm-256color",
            "tmux",
            "-u",
            "-S",
            &rig.socket.display().to_string(),
            "attach-session",
            "-t",
            HOME,
        ],
    );
    let tty = rig
        .at(&host, &["display-message", "-p", "-t", "v", "#{pane_tty}"])
        .trim()
        .to_owned();
    Rig::wait("started tty viewer attached at full size", || {
        let clients = rig.tmux(&["list-clients", "-F", ae::tmux::MENU_CLIENT_FORMAT]);
        ae::tmux::interpret_menu_client(true, &clients, &tty).is_some_and(|client| {
            client.session_name == HOME && client.width == 160 && client.height >= 40
        })
    });
    (host, tty)
}

fn right_card(rig: &Rig, app: &str, name: &str) {
    let screen = rig.screen(app);
    let row = screen
        .lines()
        .position(|row| {
            row.split('│')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .any(|word| word == name)
        })
        .expect("painted card");
    rig.raw(app, &format!("\x1b[<2;9;{}M", row + 1));
    rig.wait_screen(app, "right menu for painted card", |s| {
        s.contains(name) && s.contains("Stop")
    });
}

#[test]
fn buffered_stop_yes_cannot_affirm_unseen_question() {
    let rig = owned("menu-stop-buffered");
    let app = rig.app(&rig.socket);
    let _viewer = tty_viewer(&rig);
    let before = events(&rig, HOME);
    rig.raw(&app, "msy");
    rig.wait_screen(
        &app,
        "opening batch cannot act on either unseen modal",
        |s| s.contains("Stop") && !s.contains("Cancel"),
    );
    assert!(live(&rig, HOME));
    assert_eq!(events(&rig, HOME), before);
    rig.raw(&app, "sy");
    rig.wait_screen(&app, "question survived same-read yes", |s| {
        s.contains("Cancel") && s.contains("resume")
    });
    assert!(live(&rig, HOME));
    assert_eq!(events(&rig, HOME), before);
}

#[test]
fn replacement_client_cannot_answer_old_question() {
    let rig = owned("menu-stop-client");
    let app = rig.app(&rig.socket);
    let (host, viewer) = tty_viewer(&rig);
    let clients = rig.tmux(&["list-clients", "-F", ae::tmux::MENU_CLIENT_FORMAT]);
    let first = ae::tmux::interpret_menu_client(true, &clients, &viewer).expect("first tty viewer");
    question(&rig, &app, HOME);
    let before = events(&rig, HOME);
    rig.at(
        &host,
        &[
            "respawn-pane",
            "-k",
            "-t",
            "v",
            "env",
            "-u",
            "TMUX",
            "-u",
            "TMUX_PANE",
            "TERM=xterm-256color",
            "tmux",
            "-u",
            "-S",
            &rig.socket.display().to_string(),
            "attach-session",
            "-t",
            HOME,
        ],
    );
    let replacement = rig
        .at(&host, &["display-message", "-p", "-t", "v", "#{pane_tty}"])
        .trim()
        .to_owned();
    Rig::wait("replacement tty, new pid and same geometry", || {
        let clients = rig.tmux(&["list-clients", "-F", ae::tmux::MENU_CLIENT_FORMAT]);
        let location = rig.tmux(&[
            "list-clients",
            "-F",
            "#{client_name}|#{session_name}|#{pane_id}",
        ]);
        location.trim() == format!("{replacement}|{HOME}|{app}")
            && clients.lines().count() == 1
            && ae::tmux::interpret_menu_client(true, &clients, &replacement).is_some_and(|client| {
                client.pid != first.pid
                    && client.session_name == HOME
                    && client.width == first.width
                    && client.height == first.height
            })
    });
    rig.raw(&app, "y");
    rig.wait_screen(&app, "replacement client refuses frozen question", |s| {
        s.contains("client") && (s.contains("refus") || s.contains("changed") || s.contains("gone"))
    });
    assert!(live(&rig, HOME));
    assert_eq!(events(&rig, HOME), before);
}

#[test]
fn replaced_or_duplicated_identity_after_question_never_changes_target() {
    for duplicate in [false, true] {
        let rig = owned(if duplicate {
            "menu-stop-double"
        } else {
            "menu-stop-replaced"
        });
        let app = rig.app(&rig.socket);
        let _viewer = tty_viewer(&rig);
        question(&rig, &app, HOME);
        let path = rig.scratch.join("sessions").join(HOME).join("meta");
        let raw = fs::read_to_string(&path).expect("meta before");
        let after = if duplicate {
            format!("{raw}session_id={UUID}\n")
        } else {
            raw.replace(UUID, OTHER_UUID)
        };
        fs::write(&path, &after).expect("external replacement");
        let before = events(&rig, HOME);
        rig.raw(&app, "y");
        rig.wait_screen(&app, "target identity drift visibly refuses", |s| {
            ["replaced", "identity", "session_id", "changed"]
                .iter()
                .any(|why| s.contains(why))
        });
        assert!(live(&rig, HOME));
        assert_eq!(fs::read_to_string(path).expect("meta after"), after);
        assert_eq!(
            events(&rig, HOME),
            before,
            "refusal appended lifecycle audit"
        );
    }
}

#[test]
fn tiny_question_refuses_and_cannot_accept_yes() {
    let rig = owned("menu-stop-tiny");
    let app = rig.app(&rig.socket);
    let _viewer = tty_viewer(&rig);
    question(&rig, &app, HOME);
    rig.tmux(&["resize-window", "-t", &app, "-x", "40", "-y", "8"]);
    rig.wait_screen(&app, "whole consequence cannot fit", |s| {
        s.contains("small") || s.contains("size") || s.contains("fit") || s.contains("resize")
    });
    rig.tmux(&["resize-window", "-t", &app, "-x", "160", "-y", "45"]);
    rig.raw(&app, "y");
    let seen = rig.wait_screen(&app, "fresh y reached browse Copy", |s| {
        s.contains("no turn marked")
    });
    assert!(!seen.contains("Stopping"), "{seen}");
    assert!(live(&rig, HOME));
    assert!(!events(&rig, HOME).contains("stop-request"));
}

#[test]
fn home_stop_finishes_after_app_dies_and_leaves_no_supervisor() {
    let rig = owned("menu-stop-home");
    rig.foreign();
    let app = rig.app(&rig.socket);
    let _viewer = tty_viewer(&rig);
    let question = question(&rig, &app, HOME);
    assert!(
        question.contains("this app closes"),
        "home consequence absent: {question}"
    );
    assert!(
        question.contains("interrupt") && (question.contains("kept") || question.contains("keep")),
        "Stop consequence incomplete: {question}"
    );
    let dir = rig.scratch.join("sessions").join(HOME);
    fs::write(dir.join("memory.md"), "history kept\n").expect("persistent sentinel");
    rig.raw(&app, "y");
    Rig::wait(
        "home session gone and detached Stop finished its journal",
        || {
            !live(&rig, HOME)
                && events(&rig, HOME).contains("stopped: verified gone on its recorded server")
        },
    );
    owner_gone(&rig);
    assert!(dir.join("meta").is_file());
    assert_eq!(
        fs::read_to_string(dir.join("memory.md")).expect("state kept"),
        "history kept\n"
    );
    let journal = events(&rig, HOME);
    assert!(journal.contains("stop confirmed by client"), "{journal}");
    assert!(!journal.contains("on the session menu"), "{journal}");
    assert_eq!(journal.matches("\"action\":\"stop-request\"").count(), 1);
    assert_eq!(journal.matches("\"action\":\"stop-result\"").count(), 1);
    assert!(live(&rig, FOREIGN), "home Stop widened to sibling");
}

#[test]
fn a_row_literally_named_all_stops_only_that_row() {
    let rig = owned("menu-literal-all");
    let main = rig.session("all", OTHER_UUID);
    rig.tmux(&["set-environment", "-t", "all", "AE_SESSION", "1"]);
    let peer = rig.pane("all", "colead");
    let scout = rig.pane("all", "scout");
    for (pane, slot, name) in [
        (&main, "main", "lead"),
        (&peer, "worker.0", "colead"),
        (&scout, "spawned.0", "scout"),
    ] {
        rig.stamp(pane, slot, name);
    }
    rig.meta("all", OTHER_UUID, &rig.socket, "spawned.0");
    rig.publish(
        "all",
        &[("lead", &main), ("colead", &peer), ("scout", &scout)],
    );
    let app = rig.app(&rig.socket);
    let (_, viewer) = tty_viewer(&rig);
    let before = events(&rig, HOME);
    right_card(&rig, &app, "all");
    rig.raw(&app, "s");
    rig.wait_screen(&app, "literal all question", |s| {
        s.contains("all") && s.contains("Cancel") && s.contains("resume")
    });
    rig.raw(&app, "y");
    rig.wait_screen(&app, "affirm visibly acknowledged", |s| {
        s.contains("Stopping 'all'")
    });
    Rig::wait("only literal all finishes Stop", || {
        !live(&rig, "all")
            && events(&rig, "all").contains("stopped: verified gone on its recorded server")
    });
    owner_gone(&rig);
    assert!(live(&rig, HOME));
    assert_eq!(events(&rig, HOME), before);
    assert!(
        rig.tmux(&[
            "list-clients",
            "-F",
            "#{client_name}|#{session_name}|#{pane_id}"
        ])
        .lines()
        .any(|row| row == format!("{viewer}|{HOME}|{app}"))
    );
    assert!(rig.scratch.join("sessions/all/meta").is_file());
}
