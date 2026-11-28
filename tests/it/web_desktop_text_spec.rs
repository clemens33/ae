//! P1 redaction at the state and topic response bodies, before the browser.

use std::sync::Arc;

use ae::json::{self, Value};
use ae::topic::{self, Group};
use ae::web::chat::{BODY_MAX, Row, WORDS_MAX};
use ae::web::state::{self, Read};
use ae::web::topics::{DecideItem, Wait, WaitKind};

const SLUG: &str = "desktop-polish";
const MASKED: &str = "#c=…";

fn property<'a>(object: &'a Value, name: &str) -> &'a Value {
    object.get(name).unwrap_or_else(|| panic!("missing {name}"))
}

fn text<'a>(object: &'a Value, name: &str) -> &'a str {
    object
        .get_str(name)
        .unwrap_or_else(|| panic!("missing text {name}"))
}

fn array<'a>(object: &'a Value, name: &str) -> &'a [Value] {
    let Value::Arr(items) = property(object, name) else {
        panic!("expected array {name}");
    };
    items
}

fn document(bytes: &[u8]) -> Value {
    let response =
        std::str::from_utf8(bytes).unwrap_or_else(|error| panic!("UTF-8 response: {error}"));
    json::parse(response).unwrap_or_else(|error| panic!("JSON response: {error:?}"))
}

#[test]
fn state_masks_complete_codes_before_the_text_limits() {
    let code = "aB12".repeat(8);
    let longer = "eF34".repeat(10);
    let short = "8".repeat(31);
    let (first, last) = code.split_at(16);
    let line = format!(
        "right #c={code}, split #c={first}\u{7}{last}; long #c={longer}, short #c={short}!"
    );
    let label_edge = "L".repeat(WORDS_MAX - 4);
    let body_edge = "B".repeat(BODY_MAX - 6);
    let mut read = Read::default();
    read.chat.rows.extend([
        Row {
            key: 1,
            body: line,
            ..Row::default()
        },
        Row {
            key: 2,
            what: format!("{label_edge}#c={code} tail"),
            body: format!("{body_edge}#c={code} tail"),
            ..Row::default()
        },
    ]);
    let state = document(&state::render(Arc::new(read), "fixture", 0, &[0; 32]).body);
    let rows = array(property(&state, "chat"), "rows");
    assert_eq!(
        text(&rows[0], "body"),
        format!("right {MASKED}, split {MASKED}; long {MASKED}, short #c={short}!")
    );
    for (field, prefix) in [("what", label_edge), ("body", body_edge)] {
        let shown = text(&rows[1], field);
        assert!(
            shown
                .strip_prefix(&prefix)
                .is_some_and(|tail| tail == MASKED),
            "{field} does not mask a code at the cap"
        );
    }
}

#[test]
fn topic_text_is_masked_in_state_and_in_the_topic_response() {
    let marker = format!("#c={}", "0123aBCd".repeat(4));
    let memo = format!(
        "2026-11-24T10:00:00Z\tlead\ttopic-{SLUG}\ttitle: Earlier {marker} | state: quiet | next: Later {marker}\n\
         2026-11-24T11:00:00Z\tlead\ttopic-{SLUG}\ttitle: Topic {marker} | state: quiet | next: Now {marker}\n\
         2026-11-24T12:00:00Z\tlead\ttopic-p1-unreadable\tUnknown {marker}\n"
    );
    let mut cards = topic::cards(&[topic::Source {
        session: "redaction-fixture".into(),
        uuid: "0199c0de-aaaa-4890-abcd-ef0123456789".into(),
        incarnation: 0,
        memo: memo.into_bytes(),
    }]);
    let unreadable = cards.remove(1);
    assert_eq!(unreadable.slug, "p1-unreadable");
    let mut read = Read::default();
    read.desk
        .groups
        .extend([(Group::Quiet, cards), (Group::NeedsYou, vec![unreadable])]);
    let item = ae::desk::Id {
        section: ae::desk::Section::Decide,
        number: 1,
    };
    let question = format!("Approve {marker}? 1) With {marker} 2) No #p1-unreadable");
    read.desk.decide.push(DecideItem {
        item,
        text: question.clone(),
        tags: vec!["p1-unreadable".into()],
    });
    read.desk.waits.insert(
        "p1-unreadable".into(),
        vec![Wait {
            kind: WaitKind::Decide,
            text: question,
            id: None,
            item: Some(item),
        }],
    );
    let read = Arc::new(read);
    let snapshot = state::render(Arc::clone(&read), "fixture", 0, &[0; 32]);
    let value = document(&snapshot.body);
    let desk = property(&value, "desk");
    let quiet = array(desk, "groups")
        .iter()
        .find(|group| group.get_str("group") == Some("quiet"))
        .expect("quiet group");
    assert_eq!(
        text(&array(quiet, "rows")[0], "title"),
        format!("Topic {MASKED}")
    );
    let revisit = &array(desk, "revisit")[0];
    assert_eq!(text(revisit, "line"), format!("Now {MASKED}"));
    let decide = &array(desk, "decide")[0];
    let choices = property(decide, "choices");
    assert_eq!(text(choices, "question"), format!("Approve {MASKED}?"));
    assert_eq!(
        text(&array(choices, "options")[0], "label"),
        format!("With {MASKED}")
    );
    let asked = text(decide, "text");
    assert_eq!(
        asked,
        format!("Approve {MASKED}? 1) With {MASKED} 2) No #p1-unreadable")
    );
    assert!(
        !std::str::from_utf8(&snapshot.body)
            .expect("UTF-8 state")
            .contains(&marker)
    );

    let (bytes, _) = state::topic(&read, SLUG).expect("topic response");
    let shown = document(&bytes);
    let fields = property(&shown, "fields");
    assert_eq!(
        text(property(fields, "next"), "value"),
        format!("Now {MASKED}")
    );
    assert_eq!(
        text(&array(&shown, "history")[0], "text"),
        format!("title: Earlier {MASKED} | state: quiet | next: Later {MASKED}")
    );
    assert!(
        !std::str::from_utf8(&bytes)
            .expect("UTF-8 topic")
            .contains(&marker)
    );

    let (bytes, _) = state::topic(&read, "p1-unreadable").expect("unreadable response");
    let shown = document(&bytes);
    assert_eq!(text(&shown, "unreadable"), format!("Unknown {MASKED}"));
    assert_eq!(text(&array(&shown, "waits")[0], "text"), asked);
    assert!(
        !std::str::from_utf8(&bytes)
            .expect("UTF-8 topic")
            .contains(&marker)
    );
}
