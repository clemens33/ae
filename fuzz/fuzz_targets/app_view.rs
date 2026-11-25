#![no_main]

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;

use ae::app::handover;
use libfuzzer_sys::fuzz_target;

// The `@ae_app_view` option can be set by any client on the server, and the
// durable `app-view` file by anything that can write under the state root.
// Lossy UTF-8 still reaches the parser, which accepts only text it could have
// written itself, plus the one view token an older core wrote, an empty
// trailing panel width and the retired `|desk=` suffix a core with the board
// wrote (read by its strict grammar and never written back). The file is the
// same line behind more rules (one line feed, a byte cap, strict UTF-8, the
// canonical spelling or that suffix) and is read by the production reader.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(handover) = handover::parse(&text) {
        let written = handover.render();
        assert!(canonical_or_legacy(&text, &written));
        assert_eq!(handover::parse(&written), Some(handover.clone()));
        let _ = std::hint::black_box(handover.admits("%7", "2026.10.99", 1_760_259_000));
    }
    file_leg(data);
});

// The older core's `overview` view token reads as `agents`, an empty
// fifteenth field, the panel width, reads as the 14-field line without it and a
// retired `|desk=` suffix reads and is dropped; every other accepted text
// renders back to itself.
fn canonical_or_legacy(text: &str, written: &str) -> bool {
    let text = text.rsplit_once("|desk=").map_or(text, |(base, _)| base);
    let empty_panel = text.split('|').count() == 15 && text.strip_suffix('|') == Some(written);
    let legacy =
        text.starts_with("v1|") && (text.split('|').nth(6) == Some("overview") || empty_panel);
    written == text || legacy
}

// The bytes as a durable file: the pure decoder and the production reader (the
// no-follow, never-blocking, capped open) must agree on every input, and what
// the file accepts must be exactly what `render_file` writes, but for a
// retired `|desk=` suffix: the file has one spelling, so neither legacy
// spelling of the live parser is accepted.
fn file_leg(data: &[u8]) {
    let parsed = handover::parse_file(data);
    let dir = std::env::temp_dir().join(format!("ae-fuzz-app-view-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(handover::FILE);
    let _ = std::fs::remove_file(&file);
    let Ok(mut private) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
    else {
        return;
    };
    if private.write_all(data).is_err() {
        return;
    }
    drop(private);
    let read = ae::app::saved::read_file(&file);
    let _ = std::fs::remove_file(&file);
    assert_eq!(read, parsed, "the reader and the decoder disagree");
    let Some(held) = parsed else {
        return;
    };
    let written = handover::render_file(&held);
    // A file is strict UTF-8 and ends in exactly one line feed; the retired
    // suffix is the only text the file may carry that `render_file` does not
    // write, so the rewritten bytes are the suffix-free body plus that one LF.
    let Ok(text) = std::str::from_utf8(data) else {
        panic!("an accepted file is strict UTF-8");
    };
    let Some(body) = text.strip_suffix('\n') else {
        panic!("an accepted file ends in a line feed");
    };
    assert!(!body.contains('\n'), "an accepted file is one line");
    let canonical = body.rsplit_once("|desk=").map_or(body, |(base, _)| base);
    assert_eq!(
        written,
        format!("{canonical}\n"),
        "an accepted file is the canonical bytes, less a retired board suffix"
    );
    assert_eq!(handover::parse_file(written.as_bytes()), Some(held.clone()));
    assert!(written.len() <= handover::FILE_CAP);
    for (row, scanned) in [(true, true), (false, true), (true, false), (false, false)] {
        let _ = std::hint::black_box(handover::decide_saved(
            &held.view,
            row,
            Some(held.view.id.as_str()),
            scanned,
        ));
        let _ = std::hint::black_box(handover::decide_saved(&held.view, row, None, scanned));
    }
}
