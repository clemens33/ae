#![no_main]

use libfuzzer_sys::fuzz_target;

// A harness's `--version` answer is a child's stdout, hostile input, and the
// `version.<slot>` row it becomes is persisted state a hand edit or a crashed
// write can plant. One target drives both grammars. The answer parser may only
// fail with the two reasons it owns and every release it returns must pass the
// row's own grammar. The row reader may only accept what the row writer
// spells, so a value it accepts reads back byte for byte. The cell draws any
// accepted row for every tool and for a hostile label without panicking.
fuzz_target!(|data: &[u8]| {
    use ae::toolversion::{Seen, Why, cell, is_release, parse_answer};
    match parse_answer(data) {
        Ok(release) => {
            assert!(is_release(&release));
            let seen = Seen::Release(release);
            assert_eq!(Seen::parse_row(&seen.row()), Some(seen));
        }
        Err(why) => assert!(matches!(why, Why::Oversize | Why::Unparseable)),
    }
    let text = String::from_utf8_lossy(data);
    if let Some(seen) = Seen::parse_row(&text) {
        assert_eq!(seen.row(), text);
        for tool in [
            ae::tool::ToolKind::Claude,
            ae::tool::ToolKind::Codex,
            ae::tool::ToolKind::Gemini,
            ae::tool::ToolKind::Agy,
            ae::tool::ToolKind::Grok,
            ae::tool::ToolKind::Muse,
            ae::tool::ToolKind::OpenCode,
            ae::tool::ToolKind::Unknown,
        ] {
            let _ = std::hint::black_box(cell("bin", tool, &seen));
        }
    }
    // The same bytes as a whole meta document, through the one row reader.
    for slot in ["main", "worker.0", "spawned.1"] {
        let _ = std::hint::black_box(ae::meta::tool_version(data, slot));
    }
});
