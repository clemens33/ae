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
// `files`, the reader's view of the same walk, is exactly the file subset of
// `scan` (same ranges, same order, never a web link), its absolute path
// re-encoded by an independent encoder spells that link's URL, and its
// `:line` / `:col` sit inside the range.
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
    let base = ae::console::link::Base {
        dir: Some("/w".to_owned()),
        home: Some("/h".to_owned()),
    };
    let based = ae::console::link::scan(&text, &base);
    let files = std::hint::black_box(ae::console::link::files(&text, &base));
    let file_links: Vec<_> = based
        .iter()
        .filter(|link| link.url.starts_with("file://"))
        .collect();
    assert_eq!(files.len(), file_links.len(), "files is the file subset");
    for (file, link) in files.iter().zip(&file_links) {
        assert_eq!(
            (file.start, file.end),
            (link.start, link.end),
            "files keeps scan's range"
        );
        assert!(file.path.starts_with('/'), "a path is absolute: {file:?}");
        let mut encoded = String::from("file://");
        for byte in file.path.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                encoded.push(char::from(byte));
            } else {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
        assert_eq!(link.url, encoded, "the url spells the path");
        let visible = &text[file.start..file.end];
        assert!(
            (file.line.is_none() && file.col.is_none()) || visible.contains(':'),
            "a line or column sits inside the range: {visible:?}"
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
    // Original source ranges, rather than URLs, identify occurrences. A split
    // suffix and repeated paths must never be merged or attached to a neighbor.
    let width = usize::from(data.first().copied().unwrap_or(0)) % 128;
    let placed = ae::console::link::placed(&text, width, 2, 1, &base);
    let mut occurrences = std::collections::BTreeMap::<(usize, usize), String>::new();
    let mut previous = 0;
    for piece in &placed {
        let mut previous_slot = None;
        for (slot, file) in &piece.files {
            assert!(previous_slot.is_none_or(|last| last < *slot));
            previous_slot = Some(*slot);
            assert!(previous <= file.start, "occurrences preserve source order");
            previous = file.start;
            let original = files
                .binary_search_by_key(&file.start, |found| found.start)
                .map(|index| &files[index])
                .expect("placed file comes from the source");
            assert_eq!(file, original, "placement keeps path and complete suffix");
            let fragment = piece
                .row
                .links
                .get(*slot)
                .expect("file slot indexes a row link");
            let text = piece
                .row
                .piece
                .get(fragment.start..fragment.end)
                .expect("fragment indexes the row at character boundaries");
            occurrences
                .entry((file.start, file.end))
                .or_default()
                .push_str(text);
        }
    }
    assert_eq!(
        occurrences.len(),
        files.len(),
        "every occurrence is placed once"
    );
    for file in &files {
        assert_eq!(
            occurrences.get(&(file.start, file.end)).map(String::as_str),
            text.get(file.start..file.end),
            "row fragments spell this occurrence"
        );
    }
});
