//! Frozen acceptance from the human behavior list and reviewed plan r3.
//! Existing input APIs only: no token parser is used as its own oracle.
use super::{CAP, Effect, Input, Key, Keys, Reading, Size};
use crate::console::submit::Draft;
use std::time::Instant;

fn owner(at: Instant) -> Input {
    let mut input = Input::new(vec!["lead".to_owned(), "colead".to_owned()]);
    input.tick(Reading::Owner, at);
    input
}

fn key(input: &mut Input, at: Instant, key: Key) -> Vec<Effect> {
    input.keyed(vec![(key, at)])
}

fn feed(input: &mut Input, decoder: &mut Keys, bytes: &[u8], at: Instant, chunk: usize) {
    for bytes in bytes.chunks(chunk) {
        let effects = input.keyed(decoder.feed(bytes, at));
        assert!(effects.is_empty(), "drafting must not send: {effects:?}");
    }
}

fn sent(input: &mut Input, at: Instant) -> (Vec<u8>, String) {
    let effects = key(input, at, Key::Enter);
    assert_eq!(effects.len(), 1, "one Enter, one ask: {effects:?}");
    match effects.into_iter().next().unwrap() {
        Effect::Ask { raw, seat, body } => {
            assert_eq!(seat, "lead");
            assert_eq!(raw, body.as_bytes());
            assert!(std::str::from_utf8(&raw).is_ok());
            (raw, body)
        }
        other => panic!("expected an ask, got {other:?}"),
    }
}

#[test]
fn imgtoken_ctrl_v_and_token_send_work_in_both_decoders() {
    let at = Instant::now();
    for mut decoder in [Keys::default(), Keys::app()] {
        let mut input = owner(at);
        assert_eq!(input.keyed(decoder.feed(b"\x16", at)), [Effect::ImagePaste]);
        feed(&mut input, &mut decoder, b"look ", at, 1);
        input.image_path("/tmp/a picture.png").unwrap();
        assert_eq!(input.draft(), "look [Image #1]");
        assert!(!input.draft().contains("/tmp/"));
        assert_eq!(
            sent(&mut input, at).1,
            "look [Image #1: \"/tmp/a picture.png\"]"
        );
        assert!(input.draft().is_empty());
    }
}

#[test]
fn imgtoken_cursor_deletion_and_literal_marker_are_distinct() {
    let at = Instant::now();
    for deletion in [Key::Backspace, Key::Delete] {
        let mut input = owner(at);
        input.image_path("/tmp/one.png").unwrap();
        if deletion == Key::Delete {
            key(&mut input, at, Key::Left);
        }
        key(&mut input, at, deletion);
        assert!(
            input.draft().is_empty(),
            "one erase removes the whole token"
        );
    }
    let mut literal = owner(at);
    literal.chunk(b"[Image #1]", at);
    key(&mut literal, at, Key::Backspace);
    assert_eq!(literal.draft(), "[Image #1");
    assert_eq!(sent(&mut literal, at).1, "[Image #1");
}

#[test]
fn imgtoken_mid_text_and_numbering_preserve_each_reference() {
    let at = Instant::now();
    let mut input = owner(at);
    input.chunk(b"ab", at);
    key(&mut input, at, Key::Left);
    input.image_path("/tmp/one.png").unwrap();
    assert_eq!(input.draft(), "a [Image #1] b");
    key(&mut input, at, Key::End);
    input.image_path("/tmp/two.png").unwrap();
    assert_eq!(input.draft(), "a [Image #1] b [Image #2]");
    assert_eq!(
        sent(&mut input, at).1,
        "a [Image #1: \"/tmp/one.png\"] b [Image #2: \"/tmp/two.png\"]"
    );
}

#[test]
fn imgtoken_clear_and_each_kill_yank_the_reference_with_the_token() {
    let at = Instant::now();
    for erase in [
        Key::ClearLine,
        Key::KillBigWord,
        Key::KillWord,
        Key::KillToEnd,
    ] {
        let mut input = owner(at);
        input.image_path("/tmp/undo.png").unwrap();
        if erase == Key::KillToEnd {
            key(&mut input, at, Key::Home);
        }
        key(&mut input, at, erase);
        assert!(input.draft().is_empty());
        key(&mut input, at, Key::Yank);
        assert_eq!(input.draft(), "[Image #1]");
        assert_eq!(sent(&mut input, at).1, "[Image #1: \"/tmp/undo.png\"]");
    }
    let mut input = owner(at);
    input.chunk(b"prefix", at);
    input.image_path("/tmp/word.png").unwrap();
    key(&mut input, at, Key::KillBigWord);
    assert_eq!(input.draft(), "prefix ", "adjacent atom is its own word");
}

#[test]
fn imgtoken_restore_put_back_and_new_draft_never_remap_paths() {
    let at = Instant::now();
    let mut input = owner(at);
    input.image_path("/tmp/old.png").unwrap();
    let (raw, _) = sent(&mut input, at);
    input.image_path("/tmp/new.png").unwrap();
    input.put_back(&raw);
    let (_, body) = sent(&mut input, at);
    assert_eq!(body.matches("/tmp/old.png").count(), 1);
    assert_eq!(body.matches("/tmp/new.png").count(), 1);
    assert!(body.find("/tmp/old.png") < body.find("/tmp/new.png"));
    let mut restored = owner(at);
    let effects = restored.restore(Draft::Kept(raw.clone()));
    assert!(effects.iter().all(|e| matches!(e, Effect::Print(_))));
    assert_eq!(restored.draft(), "[Image #1]");
    assert_eq!(sent(&mut restored, at).0, raw);
}

#[test]
fn imgtoken_reserved_external_bytes_never_alias_a_live_reference() {
    let at = Instant::now();
    for mut decoder in [Keys::default(), Keys::app()] {
        for chunk in 1..=4 {
            let mut input = owner(at);
            input.image_path("/tmp/real.png").unwrap();
            feed(&mut input, &mut decoder, b"\xff\x80x\xff", at, chunk);
            assert_eq!(input.draft().matches("[Image #1]").count(), 1);
            assert!(input.draft().contains('\u{fffd}'));
        }
    }
    let mut input = owner(at);
    input.image_path("/tmp/real.png").unwrap();
    input.chunk(b"\xff", at);
    assert_eq!(
        sent(&mut input, at).1,
        "[Image #1: \"/tmp/real.png\"]\u{fffd}"
    );
    for restore in [true, false] {
        let mut input = owner(at);
        input.image_path("/tmp/real.png").unwrap();
        if restore {
            input.restore(Draft::Kept(vec![0xff, 0x80]));
        } else {
            input.put_back(&[0xff, 0x80]);
        }
        assert_eq!(input.draft().matches("[Image #1]").count(), 1);
    }
}

#[test]
fn imgtoken_narrow_rows_fit_and_cursor_is_only_before_or_after_label() {
    let at = Instant::now();
    for wire in ["[Image #1: \"/tmp/a.png\"]", "[File: \"/tmp/report.pdf\"]"] {
        for width in 1..=24 {
            let mut input = owner(at);
            input.restore(Draft::Kept(wire.as_bytes().to_vec()));
            for position in [Key::Home, Key::End] {
                key(&mut input, at, position.clone());
                let view = input.bare_view(Size { width, height: 24 });
                assert!(!view.rows.is_empty());
                assert!(view.cursor_row < view.rows.len());
                for row in &view.rows {
                    assert!(row.chars().count() <= width, "width {width}: {row:?}");
                    assert!(!row.contains("/tmp/"));
                }
                assert!(view.rows[view.cursor_row].starts_with(&view.before));
                if position == Key::Home {
                    assert!(view.before.is_empty());
                } else {
                    assert_eq!(view.before, view.rows[view.cursor_row]);
                }
            }
            assert_eq!(sent(&mut input, at).1, wire);
        }
    }
}

#[test]
fn imgtoken_resolved_cap_and_reference_slot_limits_refuse_without_loss() {
    let at = Instant::now();
    let mut input = owner(at);
    let text = vec![b'x'; CAP - 10];
    input.chunk(&text, at);
    assert!(input.image_path("/tmp/long.png").is_err());
    assert_eq!(input.draft().as_bytes(), text);
    assert_eq!(sent(&mut input, at).0, text);
    let mut input = owner(at);
    for index in 0..32 {
        input.image_path(&format!("/tmp/{index}.png")).unwrap();
    }
    let before = input.draft();
    assert!(input.image_path("/tmp/refused.png").is_err());
    assert_eq!(input.draft(), before);
    let body = sent(&mut input, at).1;
    for index in 0..32 {
        assert!(body.contains(&format!("/tmp/{index}.png")));
    }
    assert!(!body.contains("refused.png"));
}

#[test]
fn imgtoken_numeric_limit_round_trips_and_refuses_next_mint() {
    let at = Instant::now();
    for number in [1, 9, 10, 999_999] {
        let mut input = owner(at);
        let wire = format!("[Image #{number}: \"/tmp/a.png\"]");
        input.restore(Draft::Kept(wire.as_bytes().to_vec()));
        assert_eq!(input.draft(), format!("[Image #{number}]"));
        if number == 999_999 {
            assert!(input.image_path("/tmp/refused.png").is_err());
        }
        assert_eq!(sent(&mut input, at).1, wire);
    }
    for invalid in [
        "[Image #0: \"/tmp/a.png\"]",
        "[Image #01: \"/tmp/a.png\"]",
        "[Image #1000000: \"/tmp/a.png\"]",
    ] {
        let mut input = owner(at);
        input.restore(Draft::Kept(invalid.as_bytes().to_vec()));
        assert_eq!(input.draft(), invalid);
        assert_eq!(sent(&mut input, at).1, invalid);
    }
}

#[test]
fn imgtoken_app_file_paste_is_independent_of_read_boundaries() {
    let at = Instant::now();
    let paste = b"\x1b[200~/tmp/alpha.png\n'/tmp/report one.pdf'\x1b[201~";
    for chunk in 1..=paste.len() {
        let mut input = owner(at);
        let mut decoder = Keys::app();
        feed(&mut input, &mut decoder, paste, at, chunk);
        assert!(input.draft().contains("[Image #1]"), "chunk {chunk}");
        assert!(
            input.draft().contains("[File: report one.pdf]"),
            "chunk {chunk}"
        );
        assert!(!input.draft().contains("/tmp/"));
        let body = sent(&mut input, at).1;
        assert!(body.contains("[Image #1: \"/tmp/alpha.png\"]"));
        assert!(body.contains("[File: \"/tmp/report one.pdf\"]"));
    }
    let mut input = owner(at);
    feed(&mut input, &mut Keys::default(), paste, at, 1);
    key(&mut input, at, Key::Home);
    key(&mut input, at, Key::Text(b"look ".to_vec()));
    assert_eq!(
        sent(&mut input, at).1,
        "look /tmp/alpha.png\n'/tmp/report one.pdf'"
    );
}

#[test]
fn imgtoken_plain_paste_and_typed_path_stay_literal() {
    let at = Instant::now();
    for text in [
        "explain /tmp/a.png please",
        "/tmp/not.allowed",
        "[Image #1]",
        "/tmp/a.png",
    ] {
        let mut input = owner(at);
        input.chunk(text.as_bytes(), at);
        assert_eq!(input.draft(), text);
        // A leading slash uses the existing command grammar; drafting still
        // must not turn a typed path into an attachment.
        if !text.starts_with('/') {
            assert_eq!(sent(&mut input, at).1, text);
        }
    }
    let mut input = owner(at);
    let paste = b"\x1b[200~explain /tmp/a.png please\x1b[201~";
    feed(&mut input, &mut Keys::app(), paste, at, 1);
    assert_eq!(sent(&mut input, at).1, "explain /tmp/a.png please");
}

#[test]
fn imgtoken_clipboard_completion_closes_partial_paste_without_overwriting_it() {
    let at = Instant::now();
    let mut input = owner(at);
    input.chunk(b"look ", at);
    key(&mut input, at, Key::Pasted(b"/a/b.p".to_vec()));
    input.image_path("/tmp/clipboard.png").unwrap();
    key(&mut input, at, Key::Pasted(b"ng".to_vec()));
    assert_eq!(input.draft(), "look /a/b.p [Image #1]ng");
    assert_eq!(
        sent(&mut input, at).1,
        "look /a/b.p [Image #1: \"/tmp/clipboard.png\"]ng"
    );
}

#[test]
fn imgtoken_oversized_paste_remains_plain_in_later_fragments() {
    let at = Instant::now();
    let text = format!("/tmp/{}.png\n/tmp/tail.pdf", "a".repeat(16_384));
    for chunk in [1, 128, text.len()] {
        let mut input = owner(at);
        let stream = format!("\x1b[200~{text}\x1b[201~");
        feed(&mut input, &mut Keys::app(), stream.as_bytes(), at, chunk);
        assert!(!input.draft().contains("[File:"));
        assert!(!input.draft().contains("[Image #"));
        key(&mut input, at, Key::Home);
        key(&mut input, at, Key::Text(b"look ".to_vec()));
        assert_eq!(sent(&mut input, at).1, format!("look {text}"));
    }
}

#[test]
fn imgtoken_history_collapses_console_ask() {
    use crate::console::lane::{Body, fold};
    use crate::events::Event;
    let event = Event::parse_line(r#"{"ts":"2026-10-15T10:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"ae-20261015T100000Z-00000001","body_file":"image.ask.txt","summary":"short"}"#).unwrap();
    let stored = "look [Image #7: \"/tmp/saved.png\"] and [File: \"/tmp/report.pdf\"]".to_owned();
    let lane = fold(
        "fixture",
        &[],
        &[event],
        &crate::board::Observation::default(),
        0,
        &|_| Body::Whole(stored.clone()),
    );
    assert_eq!(lane.items.len(), 1);
    assert_eq!(lane.items[0].body, "look [Image #7] and [File: report.pdf]");
    assert!(!lane.items[0].body.contains("/tmp/"));
}

#[test]
fn imgtoken_left_and_right_independently_skip_surrounded_atom() {
    let at = Instant::now();
    for (start, movement, expected) in [
        (Key::End, Key::Left, "![Image #1]"),
        (Key::Home, Key::Right, "[Image #1]!"),
    ] {
        let mut input = owner(at);
        input.image_path("/tmp/a.png").unwrap();
        key(&mut input, at, start);
        key(&mut input, at, movement);
        key(&mut input, at, Key::Text(b"!".to_vec()));
        assert_eq!(input.draft(), expected);
    }
    for (prefix, movement, expected) in [
        (true, Key::Left, "a ![Image #1] b"),
        (false, Key::Right, "a [Image #1]! b"),
    ] {
        let mut input = owner(at);
        input.chunk(b"a ", at);
        input.image_path("/tmp/a.png").unwrap();
        input.chunk(b" b", at);
        if prefix {
            key(&mut input, at, Key::Left);
            key(&mut input, at, Key::Left);
        } else {
            key(&mut input, at, Key::Home);
            key(&mut input, at, Key::Right);
            key(&mut input, at, Key::Right);
        }
        key(&mut input, at, movement);
        key(&mut input, at, Key::Text(b"!".to_vec()));
        assert_eq!(input.draft(), expected);
        let body = sent(&mut input, at).1;
        assert_eq!(body.matches("/tmp/a.png").count(), 1);
    }
}

#[test]
fn imgtoken_arrow_cursor_at_narrow_widths_observes_atom_boundaries() {
    let at = Instant::now();
    for width in 1..=18 {
        for (start, arrow, before_token) in
            [(Key::End, Key::Left, true), (Key::Home, Key::Right, false)]
        {
            let mut input = owner(at);
            input.image_path("/tmp/a.png").unwrap();
            key(&mut input, at, start);
            key(&mut input, at, arrow);
            let view = input.bare_view(Size { width, height: 24 });
            assert!(view.rows.iter().all(|row| row.chars().count() <= width));
            if before_token {
                assert!(view.before.is_empty(), "width {width}: {view:?}");
            } else {
                assert_eq!(view.before, view.rows[view.cursor_row], "width {width}");
            }
            key(&mut input, at, Key::Text(b"!".to_vec()));
            let (draft, wire) = if before_token {
                ("![Image #1]", "![Image #1: \"/tmp/a.png\"]")
            } else {
                ("[Image #1]!", "[Image #1: \"/tmp/a.png\"]!")
            };
            assert_eq!(input.draft(), draft);
            assert_eq!(sent(&mut input, at).1, wire);
        }
    }
}

#[test]
fn imgtoken_owned_release_queue_drains_and_unowned_paths_never_enter_it() {
    let at = Instant::now();
    for erase in [Key::Backspace, Key::Delete] {
        let mut input = owner(at);
        input.image_path("/tmp/owned.png").unwrap();
        assert!(input.take_released().is_empty());
        if erase == Key::Delete {
            key(&mut input, at, Key::Home);
        }
        key(&mut input, at, erase);
        assert_eq!(input.take_released(), ["/tmp/owned.png"]);
        assert!(
            input.take_released().is_empty(),
            "release queue drains once"
        );
    }
    let mut input = owner(at);
    input.image_path("/tmp/undo.png").unwrap();
    key(&mut input, at, Key::ClearLine);
    assert!(
        input.take_released().is_empty(),
        "undo slot retains ownership"
    );
    key(&mut input, at, Key::Yank);
    assert!(input.take_released().is_empty());
    sent(&mut input, at);
    assert!(
        input.take_released().is_empty(),
        "Enter transfers, never deletes"
    );
    for wire in [
        "[Image #1: \"/tmp/user.png\"]",
        "[File: \"/tmp/report.pdf\"]",
    ] {
        let mut input = owner(at);
        input.restore(Draft::Kept(wire.as_bytes().to_vec()));
        assert!(input.draft() == "[Image #1]" || input.draft() == "[File: report.pdf]");
        key(&mut input, at, Key::Backspace);
        assert!(input.draft().is_empty());
        assert!(
            input.take_released().is_empty(),
            "parsed refs have no delete rights"
        );
    }
    let mut input = owner(at);
    feed(
        &mut input,
        &mut Keys::app(),
        b"\x1b[200~/tmp/user.png\n/tmp/report.pdf\x1b[201~",
        at,
        1,
    );
    assert!(input.draft().contains("[Image #1]"));
    assert!(input.draft().contains("[File: report.pdf]"));
    key(&mut input, at, Key::End);
    // Remove every displayed unit, whatever separator the paste classifier uses.
    for _ in 0..input.draft().len() {
        key(&mut input, at, Key::Backspace);
    }
    assert!(input.draft().is_empty());
    assert!(
        input.take_released().is_empty(),
        "pasted user files remain unowned"
    );
}

#[test]
fn imgtoken_numbering_counts_the_clear_slot() {
    let at = Instant::now();
    let mut input = owner(at);
    input.image_path("/tmp/first.png").unwrap();
    key(&mut input, at, Key::ClearLine);
    input.image_path("/tmp/second.png").unwrap();
    assert_eq!(input.draft(), "[Image #2]");
    key(&mut input, at, Key::Yank);
    assert_eq!(input.draft(), "[Image #2][Image #1]");
    let body = sent(&mut input, at).1;
    assert_eq!(
        body,
        "[Image #2: \"/tmp/second.png\"][Image #1: \"/tmp/first.png\"]"
    );
}

#[test]
fn imgtoken_lane_negative_controls_keep_nonconsole_wire_literal() {
    use crate::console::lane::{Body, fold};
    use crate::events::Event;
    let wire = "[Image #1: \"/tmp/peer.png\"]";
    for fields in [
        r#""actor":"telegram:42","action":"ask","target":"lead""#,
        r#""actor":"lead","action":"reply","target":"telegram:42""#,
        r#""actor":"ae:notice","action":"chat""#,
    ] {
        let row = format!(
            r#"{{"ts":"2026-10-15T10:00:00Z",{fields},"ref":"q","summary":"[Image #1: \"/tmp/peer.png\"]"}}"#
        );
        let event = Event::parse_line(&row).unwrap();
        let lane = fold(
            "fixture",
            &[],
            &[event],
            &crate::board::Observation::default(),
            0,
            &|_| Body::Whole(wire.to_owned()),
        );
        assert_eq!(
            lane.items.len(),
            1,
            "negative control must render: {fields}"
        );
        assert_eq!(lane.items[0].body, wire, "{fields}");
    }
}

#[test]
fn imgtoken_closing_or_losing_ownership_releases_unsent_live_and_cleared_images() {
    let at = Instant::now();
    for close in [false, true] {
        let mut input = owner(at);
        input.image_path("/tmp/cleared-owned.png").unwrap();
        key(&mut input, at, Key::ClearLine);
        input.image_path("/tmp/live-owned.png").unwrap();
        input.image_path("/tmp/already-released.png").unwrap();
        key(&mut input, at, Key::Backspace);
        if close {
            input.closed();
        } else {
            input.tick(Reading::NotOwner("lease lost".to_owned()), at);
        }
        assert!(input.draft().is_empty());
        let mut released = input.take_released();
        released.sort();
        assert_eq!(
            released,
            [
                "/tmp/already-released.png",
                "/tmp/cleared-owned.png",
                "/tmp/live-owned.png",
            ],
            "close={close}: dropping unsent bytes and undo releases their owned files, retaining pending releases"
        );
        assert!(input.take_released().is_empty(), "drains once");
    }
}
