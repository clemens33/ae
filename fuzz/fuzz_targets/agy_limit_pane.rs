#![no_main]

use libfuzzer_sys::fuzz_target;

// A pane agy (or any seat) drew: hostile bytes read for the vendor's usage-limit
// row of every tool that has one, and agy's stop proof over them. No reader may
// panic. An agy stop that stands needs the whole shape written in the pane: the
// `⚠ Individual quota reached` row at column 0, an `Error ID: ` row at column 0,
// the composer's `? for shortcuts` footer as the last ink, and a limit verdict
// the notice reader agrees with. Agy has no transient class, so it never reads
// as throttled, whatever the bytes.
fuzz_target!(|data: &[u8]| {
    let pane = String::from_utf8_lossy(data);
    for bin in ["claude", "codex", "muse", "opencode", "agy", "gemini", ""] {
        let _ = std::hint::black_box(ae::watchdog::limit_notice(&pane, bin));
        let _ = std::hint::black_box(ae::watchdog::limit_standing(&pane, bin));
        let _ = std::hint::black_box(ae::watchdog::throttle_class(&pane, bin));
    }
    assert_ne!(
        ae::watchdog::throttle_class(&pane, "agy"),
        Some(ae::watchdog::Throttle::Throttled)
    );
    if ae::watchdog::limit_standing(&pane, "agy") == ae::watchdog::LimitStanding::Stands {
        assert_eq!(
            ae::watchdog::throttle_class(&pane, "agy"),
            Some(ae::watchdog::Throttle::LimitReached)
        );
        assert!(ae::watchdog::limit_notice(&pane, "agy").is_some());
        assert!(
            pane.lines()
                .any(|row| row.starts_with("⚠ Individual quota reached"))
        );
        assert!(pane.lines().any(|row| row.starts_with("Error ID: ")));
        let last = pane.lines().rev().find(|row| !row.trim().is_empty());
        assert!(last.is_some_and(|row| row.contains("? for shortcuts")));
    }
});
