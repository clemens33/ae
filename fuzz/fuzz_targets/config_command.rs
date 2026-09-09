#![no_main]

use libfuzzer_sys::fuzz_target;

// The `[clients]` expansion, not just the lexing: parse the identity text, then
// resolve one profile through it twice — once with a home, once without, which
// is the branch a `$HOME`-rooted client path turns into a refusal. The first
// line names the profile and the rest is the config, so the whole resolution
// runs on fuzz bytes alone; nothing reads the filesystem.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (profile, config) = text.split_once('\n').unwrap_or((text.as_ref(), ""));
    let Ok(cfg) = ae::config::parse_identity(config) else {
        return;
    };
    let _ = std::hint::black_box(cfg.command(profile, Some(std::path::Path::new("/home/x"))));
    let _ = std::hint::black_box(cfg.command(profile, None));
});
