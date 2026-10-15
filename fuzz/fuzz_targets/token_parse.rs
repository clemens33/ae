#![no_main]

use ae::token::{self, Kind, Piece};
use libfuzzer_sys::fuzz_target;

// The attachment token is read back out of text a person, a kept draft or the
// history can hold: a stored ask body is persisted state, and a draft restored
// from a kept file is hand-editable. `parse` must be lossless (the pieces put
// back together, a token as its wire text, are the text byte for byte), read a
// token only in its exact grammar, and `collapse` must show a label and never a
// path. `classify_paste` judges an arbitrary paste and must never panic or
// hand back a path that could not sit in a token.
fuzz_target!(|data: &[u8]| {
    if let Some(found) = token::classify_paste(data) {
        assert!(!found.is_empty() && found.len() <= 16);
        for (kind, path) in &found {
            assert!(token::fits(path), "{path:?}");
            assert!(matches!(kind, Kind::Image(0) | Kind::File));
            assert!(token::wire(Kind::File, path).is_some());
        }
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let pieces = token::parse(text);
    let mut rebuilt = String::with_capacity(text.len());
    let mut last_was_text = false;
    for piece in &pieces {
        match piece {
            Piece::Text(run) => {
                assert!(!run.is_empty() && !last_was_text, "text runs are maximal");
                rebuilt.push_str(run);
                last_was_text = true;
            }
            Piece::Token { kind, path } => {
                assert!(token::fits(path), "{path:?}");
                if let Kind::Image(n) = kind {
                    assert!((1..=token::MAX_N).contains(n));
                }
                let wire = token::wire(*kind, path).expect("a parsed token has a wire form");
                assert_eq!(token::parse(&wire), [*piece], "a token parses alone");
                rebuilt.push_str(&wire);
                last_was_text = false;
            }
        }
    }
    assert_eq!(rebuilt, text, "parse is lossless");
    let collapsed = token::collapse(text);
    for piece in &pieces {
        if let Piece::Token { kind, path } = piece {
            let label = token::label(*kind, path);
            assert!(!label.chars().any(char::is_control), "{label:?}");
            assert!(label.chars().count() <= "[File: ]".len() + 40);
            assert!(collapsed.contains(&label));
        }
    }
});
