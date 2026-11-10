#![no_main]

use ae::tool_stats::{Event, Ident, Parsed, READ_PATH_MAX};
use ae::usage::claude_calls::{self, Roots};
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;

/// Every kept identity obeys the display grammar.
fn word(ident: &Ident) {
    if let Ident::Name(name) = ident {
        assert!(!name.is_empty() && !name.starts_with('.'));
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.:+-".contains(&byte)),
            "{name:?}"
        );
        assert!(name.len() <= 64);
    }
}

fuzz_target!(|data: &[u8]| {
    // The seeds' fixture roots, plus a short and a whole-filesystem root.
    let homes = [
        PathBuf::from("/fixture/config"),
        PathBuf::from("/h"),
        PathBuf::from("/"),
    ];
    let works = [PathBuf::from("/fixture/work"), PathBuf::from("/w")];
    let roots = Roots {
        config_homes: &homes,
        work_dirs: &works,
    };
    for line in data.split(|byte| *byte == b'\n') {
        let mut out = vec![Event::Stamp(0)];
        let parsed = claude_calls::line(line, &roots, &mut out);
        assert_eq!(out.first(), Some(&Event::Stamp(0)), "earlier events stay");
        if parsed == Parsed::Malformed {
            assert_eq!(out.len(), 1, "a malformed line keeps nothing");
        }
        // The same line twice gives the same events: the adapter is stateless.
        let mut again = vec![Event::Stamp(0)];
        assert_eq!(claude_calls::line(line, &roots, &mut again), parsed);
        assert_eq!(again, out);
        for event in &out[1..] {
            match event {
                Event::Attempt {
                    tool, mcp, model, ..
                } => {
                    word(tool);
                    if let Some((server, tool)) = mcp {
                        word(server);
                        word(tool);
                    }
                    if let Some(model) = model {
                        word(model);
                    }
                }
                Event::Skill { skill, .. } => word(skill),
                Event::Read {
                    direct,
                    ancestors,
                    over,
                    ..
                } => {
                    if *over {
                        assert!(ancestors.is_empty() && direct.is_none());
                    }
                    assert!(ancestors.len() <= READ_PATH_MAX / 2);
                    if let Some((name, _, _)) = direct {
                        word(name);
                    }
                }
                Event::Mcp { .. } | Event::Context { .. } => {
                    panic!("Codex-only event from the Claude adapter")
                }
                Event::Stamp(_) | Event::SkillBase { .. } | Event::Result { .. } => {}
            }
        }
    }
});
