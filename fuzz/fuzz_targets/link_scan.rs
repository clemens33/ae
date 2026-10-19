#![no_main]

use libfuzzer_sys::fuzz_target;

// A lane body is hostile persisted text: a human, a crashed write or another
// tool can leave any bytes in a journal, and `link::scan` walks every drawn
// body to decide what becomes an OSC 8 hyperlink. It must scan anything
// without a panic, hand back ranges that index the text at char boundaries,
// ascending and disjoint, keep whitespace and controls out of every link's
// visible text, and emit a URL of printable ASCII only — no control byte can
// ride a link into a terminal, whatever the body held. `wrapped` is the wrap
// that carries the links: it must wrap everything (the non-whitespace
// characters survive in order) and slice every row's links inside its piece,
// with each URL's parts, in row order, spelling that URL's scanned text.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let scanned = std::hint::black_box(ae::console::link::scan(&text, &Default::default()));
    let mut last = 0usize;
    for link in &scanned {
        assert!(
            last <= link.start
                && link.start < link.end
                && link.end <= text.len()
                && text.is_char_boundary(link.start)
                && text.is_char_boundary(link.end),
            "range {}..{} is not in order inside 0..{} at char boundaries",
            link.start,
            link.end,
            text.len()
        );
        last = link.end;
        let visible = &text[link.start..link.end];
        assert!(
            !visible
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control()),
            "a link's text holds whitespace or a control: {visible:?}"
        );
        assert!(
            ae::console::link::is_safe_url(&link.url),
            "the url is not a safe URL: {:?}",
            link.url
        );
    }
    let rows = std::hint::black_box(ae::console::link::wrapped(
        &text,
        24,
        2,
        1,
        &Default::default(),
    ));
    let letters =
        |text: &str| -> String { text.chars().filter(|ch| !ch.is_whitespace()).collect() };
    let kept: String = rows.iter().map(|row| letters(&row.piece)).collect();
    assert_eq!(kept, letters(&text), "the wrap loses no visible character");
    // Per URL, in text order: the rows' parts spell that URL's scanned texts
    // concatenated. Keyed by URL, not per link, because the same URL can
    // occur twice (and /a/b:1 and /a/b:2 share one file URL) and a link cut
    // by a wrap is spread over rows; both sides concatenate, so duplicates
    // and splits pair up.
    let mut parts: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    let mut wanted: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for row in &rows {
        for link in &row.links {
            assert!(
                link.start < link.end
                    && link.end <= row.piece.len()
                    && row.piece.is_char_boundary(link.start)
                    && row.piece.is_char_boundary(link.end),
                "the row link {}..{} does not index its piece of {} bytes",
                link.start,
                link.end,
                row.piece.len()
            );
            let part = &row.piece[link.start..link.end];
            assert!(
                !part.chars().any(|ch| ch.is_whitespace() || ch.is_control()),
                "a link part holds whitespace or a control: {part:?}"
            );
            parts.entry(link.url.as_str()).or_default().push_str(part);
        }
    }
    for link in &scanned {
        wanted
            .entry(link.url.as_str())
            .or_default()
            .push_str(&text[link.start..link.end]);
    }
    for (url, want) in &wanted {
        let joined = parts.get(url).map(String::as_str).unwrap_or("");
        assert_eq!(
            letters(joined),
            letters(want),
            "the rows' parts do not spell the scanned links of {url:?}"
        );
    }
});
