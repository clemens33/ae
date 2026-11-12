#![no_main]

use libfuzzer_sys::fuzz_target;

// `git worktree list --porcelain` describes directories anyone can create, name
// and lock, so its text is hostile: a path may hold any byte, a newline
// included. The git leg decodes it lossily, and so does this target. The parser
// must never panic, and an entry it calls unknown must never pass the
// porcelain step of the `ae worktree gc` verdict.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for entry in ae::git::parse_worktree_porcelain(&text) {
        if entry.path.contains('\u{fffd}') || !entry.path.starts_with('/') {
            assert!(entry.unknown, "a lossy or relative path stayed known");
        }
        if entry.unknown {
            assert!(
                ae::worktrees::porcelain_keep(&entry).is_some(),
                "an unknown entry passed the porcelain step"
            );
        }
        std::hint::black_box(&entry);
    }
});
