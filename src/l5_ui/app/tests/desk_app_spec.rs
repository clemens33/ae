//! The production app reader, compositor and ANSI backend on a private socket.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture owns the scratch records and the in-memory terminal"
)]

use std::fs;

use super::{Answer, ORCH, Rig, cell, text};
use crate::app::{Terminal, backend, draw_frame};

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one app fixture follows the board through wide, narrow, damaged and removed states"
)]
fn the_app_reads_the_bound_board_and_writes_its_links_only_when_the_column_fits() {
    let mut rig = Rig::new(Some("api"));
    let sessions = rig.root.0.join("sessions");
    let path = sessions.join(ORCH).join("desk-record");
    let url = "https://example.test/decision/notes/with/a/long/path?region=west&view=brief";
    let file_url = "file:///tmp/board%20notes/%CE%B1.md";
    fs::write(
        sessions.join("api").join("desk-record"),
        "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tOTHER_SESSION_BOARD\n",
    )
    .expect("ordinary selection's board decoy");
    fs::write(
        &path,
        format!(
            "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD7\tChoose the river plan\t{url}\t/tmp/board notes/α.md\nR6\tRun the checks\n"
        ),
    )
    .expect("board of the bound orchestrator");
    rig.text(b"O");

    let refresh = |rig: &mut Rig| {
        let answer = rig.reader.stage(ORCH).expect("orchestrator stage results");
        assert!(matches!(&answer, Answer::View(_)));
        rig.app.answer(answer);
    };
    refresh(&mut rig);
    let frame = rig.frame(161, 35);
    let wide = text(&frame);
    assert!(wide.contains("BOARD") && wide.contains("D7"));
    let (x, _) = cell(&frame, "Choose the river plan").expect("item appears in the board");
    assert!(x >= 125);
    // The board opens on DECIDE: the RUNNING item shows as a count on its tab.
    assert!(wide.contains("DEC 1") && wide.contains("RUN 1"), "{wide}");
    assert!(!wide.contains("R6") && !wide.contains("Run the checks"));
    assert!(!wide.contains("OTHER_SESSION_BOARD"));

    let runs = &rig.app.layout.links;
    let web_runs: Vec<_> = runs.iter().filter(|run| run.url == url).collect();
    assert!(
        web_runs.len() >= 2,
        "each wrapped fragment keeps the full URL"
    );
    assert!(runs.iter().any(|run| run.url == file_url));
    for run in runs
        .iter()
        .filter(|run| run.url == url || run.url == file_url)
    {
        assert!(run.x >= 125 && run.x + run.width <= 161);
        assert!(run.y < 35 && run.width > 0);
    }

    let mut bytes = Vec::new();
    {
        let mut terminal =
            Terminal::new(backend::Ansi::new(&mut bytes, (161, 35))).expect("test terminal");
        draw_frame(&mut terminal, |buffer| {
            rig.app.frame(buffer);
            (rig.app.layout.cursor, rig.app.layout.links.clone())
        })
        .expect("draw the actual app frame onto ANSI");
    }
    let ansi = String::from_utf8(bytes).expect("terminal bytes");
    for destination in [url, file_url] {
        assert!(
            ansi.contains(&format!(";{destination}\x1b\\")),
            "missing OSC 8 open for {destination:?}"
        );
    }
    assert!(ansi.contains("\x1b]8;;\x1b\\"));

    let narrow = text(&rig.frame(160, 35));
    assert!(narrow.contains("desk 1"), "{narrow}");
    assert!(
        !narrow.contains("desk 2"),
        "the cue counts decisions: {narrow}"
    );
    assert!(!narrow.contains("Choose the river plan"), "{narrow}");
    assert!(
        rig.app
            .layout
            .links
            .iter()
            .all(|run| run.url != url && run.url != file_url)
    );

    fs::write(
        &path,
        "ae-desk 1\nupdated 2026-11-20T08:42:12Z\nR6\tRun the checks\n",
    )
    .expect("valid board without decisions");
    refresh(&mut rig);
    let narrow = text(&rig.frame(160, 35));
    assert!(narrow.contains("desk 0"), "{narrow}");

    fs::write(&path, "ae-desk 1\nupdated damaged\n").expect("damaged replacement record");
    refresh(&mut rig);
    let damaged = text(&rig.frame(161, 35));
    assert!(
        damaged.to_lowercase().contains("desk unreadable"),
        "{damaged}"
    );
    assert!(!damaged.contains("Choose the river plan"), "{damaged}");
    assert!(
        rig.app
            .layout
            .links
            .iter()
            .all(|run| run.url != url && run.url != file_url)
    );

    fs::remove_file(&path).expect("remove the board record");
    refresh(&mut rig);
    let cleared = text(&rig.frame(161, 35));
    let panel = rig.app.layout.panel.expect("the side chat still stands");
    assert_eq!((panel.x, panel.width), (103, 58));
    assert!(!cleared.contains("BOARD") && !cleared.to_lowercase().contains("desk unreadable"));
}
