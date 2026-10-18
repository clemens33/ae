#![no_main]

use libfuzzer_sys::fuzz_target;

// Session-directory entries are hostile state too: only an exact record
// spelling may ever reach the liveness/claim path. No fuzz-selected paths or
// filesystem reads enter this target.
fuzz_target!(|data: &[u8]| {
    if let Ok(name) = std::str::from_utf8(data) {
        let parsed = std::hint::black_box(ae::store::console_draft_instance(name));
        if let Some(instance) = parsed {
            assert_eq!(name, format!("console.draft.{instance}"));
            assert_eq!(instance.len(), 36);
            for at in [8, 13, 18, 23] {
                assert_eq!(instance.as_bytes()[at], b'-');
            }
            assert!(instance.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
            assert!(!instance.bytes().any(|b| b.is_ascii_uppercase()));
        }
    }
});
