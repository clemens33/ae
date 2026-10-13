//! Independent base-compatible decoder acceptance; append child declaration
//! to `tests/it/app_mouse_spec.rs`. No new enum symbol needed to compile RED.

use super::*;

#[test]
fn plain_right_press_is_one_mouse_event_with_original_read_time() {
    let origin = Instant::now();
    let mut decoder = Keys::app();
    let events = decoder.feed(b"\x1b[<2;7;6M", origin);
    assert_eq!(events.len(), 1, "right press was dropped");
    assert_eq!(events[0].1, origin);
    assert!(
        matches!(events[0].0, Key::Mouse(Mouse { column: 6, row: 5, kind }) if kind != MouseKind::Click && kind != MouseKind::Release && kind != MouseKind::Drag && kind != MouseKind::WheelUp && kind != MouseKind::WheelDown)
    );
}

#[test]
fn split_right_report_keeps_beginning_origin() {
    let origin = Instant::now();
    let later = origin + Duration::from_millis(10);
    let bytes = b"\x1b[<2;7;6M";
    for split in 1..bytes.len() {
        let mut decoder = Keys::app();
        assert!(decoder.feed(&bytes[..split], origin).is_empty());
        let events = decoder.feed(&bytes[split..], later);
        assert_eq!(events.len(), 1, "split {split}");
        assert_eq!(events[0].1, origin, "split {split} recaptured input time");
    }
}

#[test]
fn right_release_modifiers_and_drag_are_dropped_without_text_leakage() {
    for report in [
        "\x1b[<2;7;6m",
        "\x1b[<6;7;6M",
        "\x1b[<10;7;6M",
        "\x1b[<18;7;6M",
        "\x1b[<34;7;6M",
    ] {
        let mut bytes = report.as_bytes().to_vec();
        bytes.extend_from_slice(b"x");
        let decoded = Keys::app().feed(&bytes, Instant::now());
        assert_eq!(decoded.len(), 1, "unsupported report leaked {report:?}");
        assert_eq!(decoded[0].0, Key::Text(b"x".to_vec()));
    }
}

#[test]
fn chat_discards_right_mouse_and_bracketed_paste_keeps_it_literal() {
    let bytes = b"\x1b[<2;7;6M";
    assert!(Keys::default().feed(bytes, Instant::now()).is_empty());
    let mut pasted = b"\x1b[200~".to_vec();
    pasted.extend_from_slice(bytes);
    pasted.extend_from_slice(b"\x1b[201~");
    let decoded = Keys::app().feed(&pasted, Instant::now());
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].0, Key::Pasted(bytes.to_vec()));
}
