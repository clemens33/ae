#![no_main]

use libfuzzer_sys::fuzz_target;

// The one in-place edit ae makes to the human's config: the value of the
// `[roster] orchestrator` row. The first line is the profile to bind and the
// rest is the config text, so the whole rewrite runs on fuzz bytes alone and
// nothing reads the filesystem. The rewriter's own promise is checked from the
// outside: a refusal is a value, never a panic; an edit that says it changed
// nothing is the input byte for byte; an edit that changed something parses to
// the input's identity with exactly the orchestrator binding moved, differs
// from the input only where the old value stood, and a second pick of the same
// profile changes nothing.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (profile, config) = text.split_once('\n').unwrap_or((text.as_ref(), ""));
    let first = ae::config::set_roster_row(config, "orchestrator", profile);
    let Ok(edit) = first else {
        let _ = std::hint::black_box(first.map_err(|why| why.to_string()));
        return;
    };
    assert_eq!(
        edit.changed,
        edit.text != config,
        "changed says whether bytes moved"
    );
    if !edit.changed {
        return;
    }
    let before = ae::config::parse_identity(config).expect("an edited config parsed before");
    let after = ae::config::parse_identity(&edit.text).expect("the edit parses");
    assert_eq!(after.roster_profile("orchestrator"), Some(profile));
    let old = before
        .roster_profile("orchestrator")
        .expect("the edited row was bound");
    let mut expected = before.clone();
    for row in &mut expected.roster {
        if row.0 == "orchestrator" {
            row.1 = profile.to_owned();
        }
    }
    assert_eq!(after, expected, "only the orchestrator binding moved");
    // Every other byte is the input's: strip the common head and tail and what
    // is left of each side lies inside the old and the new value.
    let (a, b) = (config.as_bytes(), edit.text.as_bytes());
    assert_eq!(
        b.len() + old.len(),
        a.len() + profile.len(),
        "exactly the old value was replaced by the new one"
    );
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (cut_old, cut_new) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    assert!(
        old.as_bytes()
            .windows(cut_old.len().max(1))
            .any(|w| w == cut_old)
            || cut_old.is_empty(),
        "what left lies inside the old value"
    );
    assert!(
        profile
            .as_bytes()
            .windows(cut_new.len().max(1))
            .any(|w| w == cut_new)
            || cut_new.is_empty(),
        "what arrived lies inside the new value"
    );
    let again = ae::config::set_roster_row(&edit.text, "orchestrator", profile)
        .expect("a second pick of the same profile places");
    assert!(!again.changed && again.text == edit.text, "idempotent");
});
