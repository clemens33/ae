#![no_main]

use ae::watchdog::{DialogKind, human_prompt_class};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let pane = String::from_utf8_lossy(data);
    for bin in [
        "claude", "codex", "agy", "grok", "muse", "opencode", "gemini", "",
    ] {
        let Some(prompt) = human_prompt_class(&pane, bin) else {
            continue;
        };
        if prompt.kind == DialogKind::Login || bin == "muse" || !prompt.warnings.is_empty() {
            assert!(prompt.trust.is_none());
            assert!(!prompt.dismiss);
            for text in prompt
                .warnings
                .iter()
                .chain([&prompt.question, &prompt.keys])
            {
                assert!(text.chars().count() <= 160);
                assert!(!text.chars().any(|ch| ch.is_control() || ch == '\u{202e}'));
            }
        }
        if !prompt.warnings.is_empty() {
            assert_eq!(bin, "claude");
            assert_eq!(prompt.kind, DialogKind::FolderTrust);
            assert!(prompt.warnings.len() <= 3);
        }
    }
});
