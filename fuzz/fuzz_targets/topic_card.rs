#![no_main]

use ae::topic::{self, Group, Source, State};
use libfuzzer_sys::fuzz_target;

// The topic checkpoints of a memo, hand-editable persisted state `ae web`
// reads every second, and the `#slug` tags of free text. The cards come out
// sorted by slug, each slug once and whole; every shown field carries a
// value; `close` shows only under a `closed` state; an unreadable card is one
// without a state word; a wait always puts a card in Needs You. Every tag is a
// whole slug that follows a `#` in the text, each once.
fuzz_target!(|data: &[u8]| {
    let cards = topic::cards(&[Source {
        session: "fuzz".to_owned(),
        uuid: "0199c0de-0000-4890-abcd-ef0123456789".to_owned(),
        incarnation: 0,
        memo: data.to_vec(),
    }]);
    for pair in cards.windows(2) {
        assert!(pair[0].slug < pair[1].slug, "cards sorted, each slug once");
    }
    for card in &cards {
        assert!(topic::is_slug(&card.slug), "a whole slug");
        let state = card
            .state
            .as_ref()
            .and_then(|field| State::parse(&field.value));
        assert_eq!(card.unreadable.is_some(), state.is_none());
        assert!(card.close.is_none() || state == Some(State::Closed));
        for field in [
            &card.title,
            &card.state,
            &card.what,
            &card.last,
            &card.next,
            &card.src,
            &card.close,
        ]
        .into_iter()
        .flatten()
        {
            assert!(!field.value.is_empty(), "a shown field has a value");
        }
        assert!(matches!(topic::group(card, true), Group::NeedsYou));
    }

    let text = String::from_utf8_lossy(data);
    let tags = topic::tags(&text);
    for (at, tag) in tags.iter().enumerate() {
        assert!(topic::is_slug(tag), "a whole slug");
        assert!(text.contains(&format!("#{tag}")));
        assert!(!tags[..at].contains(tag), "each tag once");
    }
    let fields = topic::parse_record(&text);
    for value in [
        &fields.title,
        &fields.state,
        &fields.what,
        &fields.last,
        &fields.next,
        &fields.src,
        &fields.close,
    ]
    .into_iter()
    .flatten()
    {
        assert!(!value.is_empty(), "an empty value carries nothing");
    }
});
