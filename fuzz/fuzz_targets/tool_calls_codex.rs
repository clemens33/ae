#![no_main]

use ae::tool_stats::{Event, Ident, Parsed};
use ae::usage::codex_calls;
use libfuzzer_sys::fuzz_target;

/// Every kept identity obeys the display grammar; a namespaced tool joins
/// two such words with exactly one `/`.
fn word(ident: &Ident, joined: bool) {
    if let Ident::Name(name) = ident {
        let parts: Vec<&str> = if joined {
            name.splitn(2, '/').collect()
        } else {
            vec![name.as_str()]
        };
        for part in parts {
            assert!(!part.is_empty() && !part.starts_with('.'), "{name:?}");
            assert!(
                part.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.:+-".contains(&byte)),
                "{name:?}"
            );
            assert!(part.len() <= 64);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    for line in data.split(|byte| *byte == b'\n') {
        let mut out = vec![Event::Stamp(0)];
        let parsed = codex_calls::line(line, &mut out);
        assert_eq!(out.first(), Some(&Event::Stamp(0)), "earlier events stay");
        if parsed == Parsed::Malformed {
            assert_eq!(out.len(), 1, "a malformed line keeps nothing");
        }
        let mut again = vec![Event::Stamp(0)];
        assert_eq!(codex_calls::line(line, &mut again), parsed);
        assert_eq!(again, out);
        for event in &out[1..] {
            match event {
                Event::Attempt {
                    tool, mcp, model, ..
                } => {
                    word(tool, true);
                    assert!(mcp.is_none() && model.is_none());
                }
                Event::Mcp { server, tool, .. } => {
                    word(server, false);
                    word(tool, false);
                }
                Event::Context { model, effort, .. } => {
                    word(model, false);
                    word(effort, false);
                }
                Event::Result { load, .. } => assert!(load.is_none()),
                Event::Skill { .. } | Event::SkillBase { .. } | Event::Read { .. } => {
                    panic!("Claude-only event from the Codex adapter")
                }
                Event::Stamp(_) => {}
            }
        }
    }
});
