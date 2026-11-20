//! Frozen acceptance of slice composercopy, the chat side (pure input).
//!
//! Author composercopy-rev (Anthropic), frozen against main 201244e57 and the
//! interface sent to the driver on 2026-11-20. The driver installs this file
//! byte-identically as `src/l5_ui/console/input/composercopy_spec.rs`, wired
//! by `#[cfg(test)] mod composercopy_spec;` beside `inputfeed_spec`.
//!
//! Oracles: the owner wish (one key copies the whole draft without sending
//! it, typing goes on), the brief (Alt+y must not clash with any composer key
//! or chattarget's Alt+o), xterm Meta-sends-escape (`ESC y` is Alt+y), the
//! module's own `terminal_text` rule for controls, and `Input::draft`, the
//! drawn one-line form, as an independent reference for atom labels.

use super::{Effect, Input, Key, Keys, Reading, Size};
use std::time::{Duration, Instant};

/// An owning chat input, and an instant after it took ownership.
fn owned() -> (Input, Instant) {
    let mut input = Input::new(vec!["lead".to_owned()]);
    let _ = input.tick(Reading::Owner, Instant::now());
    (input, Instant::now() + Duration::from_millis(1))
}

fn keys(input: &mut Input, at: Instant, keys: Vec<Key>) -> Vec<Effect> {
    input.keyed(keys.into_iter().map(|key| (key, at)).collect())
}

/// What the draft looks like everywhere a copy could disturb it.
fn shape(input: &Input) -> (String, super::View) {
    let size = Size {
        width: 40,
        height: 24,
    };
    (input.draft(), input.bare_view(size))
}

/// C1: `ESC y` is Alt+y in both decoders, the chat's and the app's, in one read
/// and split across two reads; chattarget's `ESC o` and the neighbours stay
/// what they were.
#[test]
fn alt_y_is_copy_draft_in_the_chat_and_the_app_decoder() {
    let at = Instant::now();
    for app in [false, true] {
        let fresh = || if app { Keys::app() } else { Keys::default() };
        let mut whole = fresh();
        assert_eq!(
            whole.feed(b"\x1by", at),
            [(Key::CopyDraft, at)],
            "one read, app={app}"
        );
        let mut split = fresh();
        assert!(split.feed(b"\x1b", at).is_empty(), "held, app={app}");
        assert_eq!(
            split.feed(b"y", at + Duration::from_millis(5)),
            [(Key::CopyDraft, at)],
            "split read keeps the first byte's stamp, app={app}"
        );
        let mut plain = fresh();
        assert_eq!(
            plain.feed(b"y", at),
            [(Key::Text(b"y".to_vec()), at)],
            "a plain y is text, app={app}"
        );
        let mut pasted = fresh();
        let inside = pasted.feed(b"\x1b[200~a\x1byb\x1b[201~", at);
        assert!(
            !inside.iter().any(|(key, _)| *key == Key::CopyDraft),
            "inside a bracketed paste ESC y is text, app={app}: {inside:?}"
        );
        let mut other = fresh();
        for seq in [&b"\x1bY"[..], b"\x1bx", b"\x1bo"] {
            assert!(
                !other
                    .feed(seq, at)
                    .iter()
                    .any(|(key, _)| *key == Key::CopyDraft),
                "{seq:?} is not Alt+y, app={app}"
            );
        }
    }
}

/// C2: Alt+y hands the whole draft, rows the view elides included, to the
/// caller and changes nothing: no send, the same draft, rows and cursor, and
/// typing goes on where it was.
#[test]
fn alt_y_copies_the_whole_draft_and_leaves_it_as_it_was() {
    let (mut input, at) = owned();
    let middle: Vec<String> = (3..=14).map(|n| format!("l{n:02}")).collect();
    let mut typed = vec![
        Key::Text(b"first line".to_vec()),
        Key::Newline,
        Key::Text(b"  two\tspaced".to_vec()),
    ];
    for line in &middle {
        typed.extend([Key::Newline, Key::Text(line.as_bytes().to_vec())]);
    }
    typed.extend([
        Key::Newline,
        Key::Text(b"third, longer than the pane is wide".to_vec()),
        Key::Left,
        Key::Left,
    ]);
    let said = keys(&mut input, at, typed);
    assert!(said.is_empty(), "typing says nothing: {said:?}");
    let before = shape(&input);
    assert!(
        before.1.rows.iter().any(|row| row.contains("lines above")),
        "the view elides the head: {:?}",
        before.1.rows
    );
    assert!(
        !before.1.rows.iter().any(|row| row.contains("first line")),
        "the first line is not shown: {:?}",
        before.1.rows
    );
    let whole = format!(
        "first line\n  two\tspaced\n{}\nthird, longer than the pane is wide",
        middle.join("\n")
    );
    let copied = keys(&mut input, at, vec![Key::CopyDraft]);
    assert_eq!(
        copied,
        [Effect::Copy(whole.clone())],
        "the logical draft: breaks, spaces and the unshown head kept"
    );
    assert_eq!(shape(&input), before, "draft, rows and cursor unchanged");
    let _ = keys(&mut input, at, vec![Key::Text(b"X".to_vec())]);
    assert!(
        input.draft().ends_with("wiXde"),
        "the cursor stayed two left of the end: {}",
        input.draft()
    );
    let after = whole.replace("wide", "wiXde");
    assert_eq!(
        keys(&mut input, at, vec![Key::CopyDraft, Key::CopyDraft]),
        [Effect::Copy(after.clone()), Effect::Copy(after)],
        "a repeated Alt+y copies each time and only copies"
    );
}

/// C3: an empty draft is an empty copy, never an ask, and the key leaves the
/// composer empty.
#[test]
fn alt_y_on_an_empty_draft_is_an_empty_copy() {
    let (mut input, at) = owned();
    assert_eq!(
        keys(&mut input, at, vec![Key::CopyDraft]),
        [Effect::Copy(String::new())]
    );
    assert!(input.draft().is_empty());
}

/// C4: the copy is inert: an attachment shows as its label, never its stored
/// path, and a control byte other than a line break or tab is U+FFFD.
#[test]
fn alt_y_copies_labels_and_neutralises_controls() {
    let (mut input, at) = owned();
    let _ = keys(&mut input, at, vec![Key::Text(b"see".to_vec())]);
    input
        .image_path("/private/composercopy/shot.png")
        .expect("an image fits");
    let _ = keys(
        &mut input,
        at,
        vec![Key::Pasted(b" bell\x07esc\x1b[31m".to_vec())],
    );
    let copied = keys(&mut input, at, vec![Key::CopyDraft]);
    let [Effect::Copy(text)] = copied.as_slice() else {
        panic!("one copy: {copied:?}");
    };
    let text = text.clone();
    assert!(text.contains("[Image #1]"), "the label: {text:?}");
    assert!(!text.contains("/private/"), "never the path: {text:?}");
    assert!(
        !text
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t'),
        "no live control: {text:?}"
    );
    assert!(text.contains("bell\u{fffd}esc\u{fffd}[31m"), "{text:?}");
    assert_eq!(
        text.replace(['\n', '\t'], " "),
        input.draft(),
        "the same text the composer draws, breaks restored"
    );
}

/// C5: Alt+y is not an edit: the text `^U` cleared still comes back with
/// `^Y` after it, once.
#[test]
fn alt_y_keeps_the_yank() {
    let (mut input, at) = owned();
    let _ = keys(
        &mut input,
        at,
        vec![
            Key::Text(b"gone".to_vec()),
            Key::ClearLine,
            Key::Text(b"new ".to_vec()),
        ],
    );
    assert_eq!(
        keys(&mut input, at, vec![Key::CopyDraft]),
        [Effect::Copy("new ".to_owned())]
    );
    let _ = keys(&mut input, at, vec![Key::Yank]);
    assert_eq!(input.draft(), "new gone", "^Y still brings back the clear");
}
