//! S1b topic grammar, precedence, provenance and grouping.

use ae::topic::{self, Card, Field, Group, Source, State};

const FIRST_UUID: &str = "0199c0de-1111-4890-abcd-ef0123456789";
const LAST_UUID: &str = "0199c0de-2222-4890-abcd-ef0123456789";

fn source(session: &str, uuid: &str, incarnation: usize, memo: &str) -> Source {
    Source {
        session: session.to_owned(),
        uuid: uuid.to_owned(),
        incarnation,
        memo: memo.as_bytes().to_vec(),
    }
}

fn card<'a>(cards: &'a [Card], slug: &str) -> &'a Card {
    cards
        .iter()
        .find(|entry| entry.slug == slug)
        .unwrap_or_else(|| panic!("card {slug}"))
}

fn field(field: Option<&Field>) -> &Field {
    field.unwrap_or_else(|| panic!("populated field"))
}

#[test]
fn a_memo_topic_requires_the_whole_ascii_slug() {
    let limit = format!("0{}-", "z".repeat(46));
    for slug in ["0", "web", "m2-release", "trailing-", &limit] {
        assert!(topic::is_slug(slug), "{slug}");
        let name = format!("topic-{slug}");
        assert_eq!(topic::slug_of(&name), Some(slug));
    }
    let too_long = format!("{limit}0");
    for slug in ["", "-web", " Web", "Web", "w_eb", "wéb", "web ", &too_long] {
        assert!(!topic::is_slug(slug), "{slug:?}");
        assert_eq!(topic::slug_of(&format!("topic-{slug}")), None, "{slug:?}");
    }
    for bare in ["web", "topics-web", "Topic-web", "topic", "topic--web"] {
        assert_eq!(topic::slug_of(bare), None, "{bare}");
    }
}

#[test]
fn memo_fields_unescape_pipe_and_backslash_and_keep_the_last_value() {
    let fields = topic::parse_record(
        r"before colon | title: replaced | what: C:\\tmp\q | title: Web \| UI | state: quiet | What: ignored | src: source: 1; /tmp/2 | next: ",
    );
    assert_eq!(fields.title.as_deref(), Some("Web | UI"));
    assert_eq!(fields.what.as_deref(), Some(r"C:\tmp\q"));
    assert_eq!(fields.state.as_deref(), Some("quiet"));
    assert_eq!(fields.src.as_deref(), Some("source: 1; /tmp/2"));
    assert_eq!(fields.last, None);
    assert_eq!(fields.next, None);
    assert_eq!(fields.close, None);
    for (word, parsed) in [
        ("needs", State::Needs),
        ("moving", State::Moving),
        ("quiet", State::Quiet),
        ("parked", State::Parked),
        ("closed", State::Closed),
    ] {
        assert_eq!(State::parse(word).map(|state| state.word()), Some(word));
        assert_eq!(parsed.word(), word);
    }
    for invalid in ["NEEDS", "new", " quiet", "", "closed extra"] {
        assert!(State::parse(invalid).is_none(), "{invalid:?}");
    }
}

#[test]
fn tags_obey_both_token_boundaries_and_keep_first_occurrence() {
    assert_eq!(
        topic::tags("#start #punct, #nested!)\n#start\u{a0}#nb\u{2003}#em #last"),
        ["start", "punct", "nested", "nb", "em", "last"]
    );
    assert_eq!(
        topic::tags(
            "a#one (#two) ##three #four#five #x,#y #Web #webX #web_2 #web/path #webé #web— #tail-"
        ),
        ["x", "tail-"]
    );
    let slug = "z".repeat(48);
    let text = format!("#{slug} #{slug}z #after.");
    assert_eq!(topic::tags(&text), [slug.as_str(), "after"]);
}

#[test]
fn fields_follow_incarnation_and_append_order_and_reviewed_uses_any_latest_record() {
    let last = source(
        "current",
        LAST_UUID,
        12,
        "2026-04-30T12:00:00Z\tlead\ttopic-release\ttitle: replaced by append | state: moving\n\
         2026-04-29T12:00:00Z\tlead\ttopic-release\ttitle: current title | next: choose\n\
         2026-04-28T12:00:00Z\tlead\ttopic-release\tignored: newest checkpoint\n\
         2026-04-28T12:00:00Z\tlead\ttopic-alpha\tstate: parked\n",
    );
    let first = source(
        "older",
        FIRST_UUID,
        7,
        "2026-10-15T12:00:00Z\tlead\ttopic-release\ttitle: prior title | what: retained what | state: closed | close: prior reason\n\
         2026-10-15T12:00:00Z\tlead\ttopic-Invalid\tstate: closed\n",
    );
    // The slice order, clocks and actual rank point in different directions.
    let cards = topic::cards(&[last, first]);
    assert_eq!(
        cards
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "release"]
    );
    let release = card(&cards, "release");
    assert_eq!(field(release.title.as_ref()).value, "current title");
    assert_eq!(
        field(release.title.as_ref()).origin.at,
        "2026-04-29T12:00:00Z"
    );
    assert_eq!(field(release.title.as_ref()).origin.session, "current");
    assert_eq!(field(release.title.as_ref()).origin.uuid, LAST_UUID);
    assert_eq!(field(release.what.as_ref()).value, "retained what");
    assert_eq!(
        field(release.what.as_ref()).origin.at,
        "2026-10-15T12:00:00Z"
    );
    assert_eq!(field(release.what.as_ref()).origin.session, "older");
    assert_eq!(field(release.what.as_ref()).origin.uuid, FIRST_UUID);
    assert_eq!(field(release.state.as_ref()).value, "moving");
    assert_eq!(field(release.next.as_ref()).value, "choose");
    assert!(release.close.is_none());
    assert_eq!(release.reviewed, "2026-04-28T12:00:00Z");
    assert_eq!(release.history.len(), 1);
    assert_eq!(release.history[0].text, "ignored: newest checkpoint");
}

#[test]
fn closed_and_unreadable_checkpoints_have_distinct_projections() {
    let entries = topic::cards(&[source(
        "current",
        LAST_UUID,
        0,
        "2026-03-01T12:00:00Z\tlead\ttopic-done\tstate: closed | close: approved\n\
         2026-03-01T12:00:00Z\tlead\ttopic-blocked\tstate: needs\n\
         2026-03-01T12:00:00Z\tlead\ttopic-pending\tstate: quiet\n\
         2026-03-01T12:00:00Z\tlead\ttopic-damaged\tstate: moving | what: kept\n\
         2026-03-01T12:00:00Z\tlead\ttopic-damaged\tstate: unrecognized | unparsed: evidence\n\
         2026-03-01T12:00:00Z\tlead\ttopic-missing\tunknown: text still shown\n",
    )]);
    let done = card(&entries, "done");
    assert_eq!(field(done.close.as_ref()).value, "approved");
    assert!(done.unreadable.is_none());
    assert!(matches!(topic::group(done, false), Group::Closed));
    let pending = card(&entries, "pending");
    assert!(matches!(topic::group(pending, false), Group::Quiet));
    assert!(matches!(topic::group(pending, true), Group::NeedsYou));
    assert!(matches!(
        topic::group(card(&entries, "blocked"), false),
        Group::NeedsYou
    ));
    let damaged = card(&entries, "damaged");
    assert_eq!(
        damaged.unreadable.as_deref(),
        Some("state: unrecognized | unparsed: evidence")
    );
    assert!(matches!(topic::group(damaged, false), Group::Unreadable));
    let missing = card(&entries, "missing");
    assert_eq!(
        missing.unreadable.as_deref(),
        Some("unknown: text still shown")
    );
    assert!(matches!(topic::group(missing, false), Group::Unreadable));
}
