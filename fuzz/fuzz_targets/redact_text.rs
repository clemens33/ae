#![no_main]

use libfuzzer_sys::fuzz_target;

// The secret-shape redactor over hostile text. Every span is a non-empty, ascending, disjoint
// byte range on char boundaries; the output is the input with each span replaced by its marker
// and every byte outside the spans copied; the call is deterministic; and a second pass over
// the output finds no shape that lies wholly inside text the first pass left alone.
fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let (text, spans) = ae::redact::redact_text(input);
    assert_eq!(
        (text.clone(), spans.clone()),
        ae::redact::redact_text(input)
    );

    let mut expected = String::new();
    let mut outside = Vec::new();
    let mut cursor = 0;
    for (kind, range) in &spans {
        assert!(range.start < range.end, "empty span {range:?}");
        assert!(range.start >= cursor, "overlap or disorder at {range:?}");
        assert!(
            input.is_char_boundary(range.start) && input.is_char_boundary(range.end),
            "span {range:?} splits a char"
        );
        let unchanged = &input[cursor..range.start];
        outside.push(expected.len()..expected.len() + unchanged.len());
        expected.push_str(unchanged);
        expected.push_str("[redacted:");
        expected.push_str(kind.label());
        expected.push(']');
        cursor = range.end;
    }
    let tail = &input[cursor..];
    outside.push(expected.len()..expected.len() + tail.len());
    expected.push_str(tail);
    assert_eq!(text, expected);

    let (_, second) = ae::redact::redact_text(&text);
    for (kind, range) in second {
        assert!(
            !outside
                .iter()
                .any(|kept| kept.start <= range.start && range.end <= kept.end),
            "a {} shape survived in unchanged text at {range:?}",
            kind.label()
        );
    }
});
