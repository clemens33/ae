#![no_main]

use std::fmt::Write as _;

use ae::web::choice;
use libfuzzer_sys::fuzz_target;

// The choices a DECIDE text offers `ae web`'s page, read from the
// orchestrator board on disk, which a hand may edit. Choices come only as two
// to nine options with a trimmed non-empty question and labels and a
// recommendation that names an option or none, and the form written back from
// them reads as the same choices. The written form puts each label right after
// its `)`, so a label that starts with its own `N)` stays inside its word.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let Some(choices) = choice::parse(&text) else {
        return;
    };
    let count = choices.options.len();
    assert!((2..=9).contains(&count), "two to nine options");
    assert!(
        choices
            .recommend
            .is_none_or(|number| (1..=count).contains(&number)),
        "a recommendation names an option"
    );
    for part in std::iter::once(&choices.question).chain(&choices.options) {
        assert!(!part.is_empty() && part.trim() == part, "a trimmed part");
    }
    let mut written = choices.question.clone();
    for (at, label) in choices.options.iter().enumerate() {
        let _ = write!(written, " {}){label}", at + 1);
    }
    let _ = write!(written, " (recommend {})", choices.recommend.unwrap_or(0));
    assert_eq!(
        choice::parse(&written).as_ref(),
        Some(&choices),
        "{written:?}"
    );
});
