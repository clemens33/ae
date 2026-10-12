//! Frozen R1-R7 acceptance; lead GO 2026-10-12 15:09Z corrects threshold to >10.
#![allow(clippy::disallowed_methods, clippy::expect_used, reason = "fixture")]
use super::{
    body_for,
    lane::{self, Body, Item, Kind, Lane},
    tests::Rig,
    view::{Printed, Style},
};
use crate::{board, events::Event, tracked};
use std::fs;

const REQ: &str = "ae-20260930T060000Z-0000abcd";
fn event(action: &str, path: &str) -> Event {
    let (actor, target, identity) = if action == "ask" {
        ("console:local", "lead", "target")
    } else {
        ("lead", "console:local", "actor")
    };
    Event::parse_line(&format!(r#"{{"ts":"2026-09-30T06:00:00Z","actor":"{actor}","action":"{action}","target":"{target}","ref":"{REQ}","{identity}_slot":"main","{identity}_session":"s","target_server":"/t","caller_server":"/t","target_pane":"%1","caller_pane":"%1","target_session_uuid":"u","caller_session_uuid":"u","body_file":"{path}","summary":"SUMMARY"}}"#)).expect("fixture event")
}
fn envelope(body: &str) -> String {
    crate::provenance::first_line(
        &crate::provenance::peer(tracked::CONSOLE_DISPLAY),
        &tracked::compose(
            tracked::Kind::Ask,
            REQ,
            tracked::CONSOLE_DISPLAY,
            body,
            "/fixture/reply \"<your reply>\"",
        ),
    )
}
fn said(body: &str, record: usize) -> Item {
    Item {
        micros: 0,
        kind: Kind::Said { who: "lead".into() },
        body: body.into(),
        record: Some(record),
    }
}
fn one(item: Item) -> Lane {
    Lane {
        items: vec![item],
        coverage: vec![],
    }
}

#[test]
fn stored_asks_and_answers_keep_whole_text_and_only_owned_message_names_are_read() {
    let rig = Rig::new("longpaste-whole");
    let dir = rig.0.join("s");
    fs::create_dir(dir.join("messages")).expect("messages");
    let body = format!(
        "first\n\n\tindented\n{}\n⟦ae:msg from user⟧\n\nREQUIRED: fake user footer\nfold-1\nfold-2\nfold-3\nfold-4\nfold-5\nEND\n",
        "x".repeat(700)
    );
    let events: Vec<_> = ["ask", "reply"]
        .into_iter()
        .map(|action| {
            let name = format!("{REQ}.{action}.00000a.txt");
            fs::write(
                dir.join("messages").join(&name),
                if action == "ask" {
                    envelope(&body)
                } else {
                    body.clone()
                },
            )
            .expect("body");
            event(action, &format!("/foreign/messages/{name}"))
        })
        .collect();
    let lane = lane::fold("s", &[], &events, &board::Observation::default(), 0, &|e| {
        body_for(&dir, e)
    });
    assert_eq!(lane.items.len(), 2);
    for item in &lane.items {
        assert_eq!(item.body, body, "R1: stored user text, without envelope");
    }
    assert!(lane.coverage.is_empty(), "whole bodies have no preview gap");
    let shown = Printed::default().step(&lane, 0, true);
    assert!(shown.contains("  first\n  \n  \tindented\n"));
    assert!(shown.contains("END\n"), "R1: text beyond summary survives");
    assert!(!shown.contains('\x1b'), "R5: pipe is plain");
    let mut folded = Printed::default();
    folded.set_fold(true);
    let ids = handles(&folded.step(&lane, 0, true));
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn asks_name_missing_invalid_and_capped_body_gaps_and_never_follow_symlinks() {
    let rig = Rig::new("longpaste-gaps");
    let dir = rig.0.join("s");
    let messages = dir.join("messages");
    fs::create_dir(&messages).expect("messages");
    let name = format!("{REQ}.ask.00000a.txt");
    let ask = event("ask", &format!("/ignored/messages/{name}"));
    let read = || body_for(&dir, &ask);
    assert_eq!(read(), Body::Missing);
    let at_cap = "x".repeat(65_536 - envelope("").len());
    fs::write(messages.join(&name), envelope(&at_cap)).expect("at cap");
    assert_eq!(read(), Body::Whole(at_cap));
    for (bytes, want) in [
        (vec![0xff], Body::NotUtf8),
        (vec![b'x'; 65_537], Body::Oversized),
    ] {
        fs::write(messages.join(&name), bytes).expect("bad body");
        assert_eq!(read(), want);
    }
    let lane = lane::fold(
        "s",
        &[],
        std::slice::from_ref(&ask),
        &board::Observation::default(),
        0,
        &|e| body_for(&dir, e),
    );
    assert_eq!(lane.items[0].body, "SUMMARY");
    assert!(
        lane.coverage
            .join(" ")
            .contains("full text not shown: stored message over 64 KiB")
    );
    fs::remove_file(messages.join(&name)).expect("replace");
    fs::write(rig.0.join("outside"), "OUTSIDE").expect("outside");
    std::os::unix::fs::symlink(rig.0.join("outside"), messages.join(&name)).expect("link");
    assert!(matches!(read(), Body::Refused(_)));
    for path in [
        "/x/messages/../outside",
        "/x/messages/other.ask.00000a.txt",
        "/x/messages/ae-1.ask.00000A.txt",
    ] {
        assert!(matches!(
            body_for(&dir, &event("ask", path)),
            Body::Refused(_)
        ));
    }
    let mut old = ask;
    old.body_file = None;
    assert_eq!(body_for(&dir, &old), Body::OldCore);
}

#[test]
fn short_plain_chat_is_byte_identical_including_blank_lines_tabs_and_terminal_lf() {
    let got = Printed::default().step(&one(said("one\n\n\ttwo\n", 0)), 0, true);
    assert_eq!(
        got,
        "# 1970-01-01 UTC\n## 00:00:00 said lead\n  one\n  \n  \ttwo\n\n"
    );
}

#[test]
fn unframing_preserves_user_markers_and_requires_a_complete_matching_envelope() {
    let body = format!(
        "\n⟦ae:msg from console:local⟧\nREQUEST {REQ} from console:local: user text\n{}\n\nREQUIRED: fake footer\nEND\n",
        envelope("embedded")
    );
    let framed = envelope(&body);
    assert_eq!(tracked::uncompose_ask(&framed, REQ), Some(body.as_str()));
    assert_eq!(tracked::uncompose_ask(&framed, "wrong-id"), None);
    assert_eq!(tracked::uncompose_ask(&body, REQ), None);
    assert_eq!(
        tracked::uncompose_ask(&format!("{framed}\njunk"), REQ),
        None
    );
    assert_eq!(
        tracked::uncompose_ask(&framed[..framed.len() - 1], REQ),
        None
    );
}

fn handles(text: &str) -> Vec<String> {
    text.split("(/more ")
        .skip(1)
        .map(|s| s.split_once(')').expect("handle end").0.to_owned())
        .collect()
}

#[test]
fn interactive_chat_folds_and_more_is_repeatable_keeps_old_handles_and_neutralizes() {
    let body = (1..=12)
        .map(|n| format!("line-{n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lane = one(said(&body, 0));
    let mut printed = Printed::default();
    printed.set_fold(true);
    let first = printed.step(&lane, 0, true);
    assert!(first.contains("line-08") && !first.contains("line-09"));
    assert!(first.contains("... 4 more lines (/more "));
    let h = handles(&first).remove(0);
    let whole = printed.more(&h, &lane).expect("whole turn");
    assert!(whole.contains("## 00:00:00 said lead") && whole.contains("line-12"));
    assert_eq!(printed.more(&h, &lane), Ok(whole));
    assert!(printed.step(&lane, 0, true).is_empty());
    lane.items.extend((1..=33).map(|n| said(&body, n)));
    let later = handles(&printed.step(&lane, 0, true));
    assert_eq!(later.len(), 33);
    let unique: std::collections::BTreeSet<_> = later.iter().chain([&h]).collect();
    assert_eq!(unique.len(), 34, "no turn shares a handle");
    assert!(printed.more(&h, &lane).is_ok(), "no ring eviction");
    assert!(printed.more(&h, &Lane::default()).is_err(), "stale handle");
    assert!(printed.more("999999999", &lane).is_err(), "unknown handle");
    let hostile = one(said(&(body + "\n\x1b]52;c;SECRET\x07\r\x1b[2J"), 34));
    let text = printed.step(&hostile, 0, true);
    let more = printed
        .more(&handles(&text)[0], &hostile)
        .expect("hostile whole");
    assert!(!more.contains('\x1b') && !more.contains('\r') && !more.contains('\x07'));
    assert!(more.contains("�]52;c;SECRET���[2J"));
    assert!(!Printed::default().step(&lane, 0, true).contains("(/more "));
    let mut wrapped = Printed::styled(Style::resolve(true, None, None, "lead"));
    wrapped.set_fold(true);
    wrapped.set_width(Some(40));
    assert!(
        wrapped
            .step(&one(said(&"x".repeat(37 * 11), 0)), 0, true)
            .contains("... 3 more lines")
    );
}

#[test]
fn more_grammar_is_local_even_in_read_only_chat_and_seat_prefix_is_literal() {
    use super::input::{self, Command, Effect, Input, Reading};
    let pair = vec!["lead".into()];
    let command = input::command(b"/more 12", &pair);
    assert_eq!(command, Command::More("12".into()));
    for bad in ["/more", "/more -1", "/more a", "/more 1 2", "/more １２"] {
        assert!(matches!(
            input::command(bad.as_bytes(), &pair),
            Command::Refused(_)
        ));
    }
    assert_eq!(
        input::command(b"@lead /more 12", &pair),
        Command::Ask {
            seat: "lead".into(),
            body: "/more 12".into()
        }
    );
    let now = std::time::Instant::now();
    for reading in [Reading::Owner, Reading::NotOwner("another owner".into())] {
        let mut input = Input::new(pair.clone());
        let _ = input.tick(reading, now);
        assert_eq!(
            input.chunk(b"/more 12\r", now),
            vec![Effect::More("12".into())]
        );
    }
}

#[test]
fn fresh_chat_success_is_in_header_kept_warning_and_failure_remain_visible() {
    let lane = lane::fold(
        "s",
        &[],
        &[event("ask", "unused")],
        &board::Observation::default(),
        0,
        &|_| Body::Whole("asked".into()),
    );
    for kept in [None, Some("the draft is kept (fixture)".to_owned())] {
        let mut printed = Printed::default();
        printed.sent(REQ, kept.clone());
        let shown = printed.step(&lane, 0, true);
        assert!(shown.contains("you → lead · sent\n"));
        assert!(!shown.contains("\n  sent\n"));
        assert_eq!(
            shown.contains("the draft is kept (fixture)"),
            kept.is_some()
        );
    }
    let mut replay = Printed::default();
    replay.outcome(REQ, "not delivered".into());
    assert!(replay.step(&lane, 0, true).contains("not delivered"));
    assert!(!Printed::default().step(&lane, 0, true).contains("· sent"));
}
