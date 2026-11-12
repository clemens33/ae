//! Frozen issue 314 loader acceptance tests, authored before the fix.
//! Install as `src/app/loader/early_settings_spec.rs` and wire from loader.rs under cfg(test).
#![allow(clippy::expect_used, reason = "bounded acceptance fixtures")]

use std::time::Instant;

use super::{Answer, Job, Reader, Request, SettingsPaths};
use crate::app::settings::InstructionsView;
use crate::app::tests::{Root, session};

fn reader(tag: &str, home: Option<&str>) -> (Root, Reader) {
    let root = Root::new(tag);
    let global = root.0.join("config");
    std::fs::write(&global, "").expect("isolated config");
    for name in ["api", "web"] {
        session(&root, name, &format!("config={}\n", global.display()));
    }
    let mut reader = Reader::new(root.0.clone(), home.map(str::to_owned), None);
    reader.settings_paths(SettingsPaths {
        home: Some(root.0.join("home")),
        global,
        local: None,
    });
    reader.take(Request::Settings {
        open: true,
        generation: 41,
    });
    (root, reader)
}

fn assert_instructions(reader: &Reader, name: &str) {
    let InstructionsView::Ready(protocol) = reader.instructions() else {
        panic!(
            "the adopted selection must supply Instructions: {:?}",
            reader.instructions()
        );
    };
    assert_eq!(protocol.session, name);
}

#[test]
fn selection_is_adopted_before_or_after_the_first_settings_read() {
    for completed in [false, true] {
        let (_root, mut reader) = reader(&format!("issue314-adopt-{completed}"), None);
        assert_eq!(reader.settings_session, None);
        if completed {
            let Answer::Settings { instructions, .. } = reader.settings() else {
                panic!("a Settings read answers Settings");
            };
            assert_eq!(
                instructions,
                Some(InstructionsView::Gap("no session selected".into()))
            );
            assert!(reader.settings_full);
        }
        reader.take(Request::Focus(Some("api".into())));
        assert_eq!(reader.settings_session.as_deref(), Some("api"));
        assert!(
            !reader.settings_full,
            "Instructions must be read again after adoption"
        );
        assert_eq!(
            reader.settings,
            Some((41, None)),
            "the same generation is due now"
        );
        assert_instructions(&reader, "api");
        reader.fleet_at = Some(Instant::now());
        reader.looks.clear();
        assert!(matches!(reader.next(Instant::now()), Job::Settings));
    }
}

#[test]
fn adoption_is_once_and_never_retargets_a_pinned_or_home_overlay() {
    for home in [None, Some("api")] {
        let (_root, mut reader) = reader(&format!("issue314-pinned-{home:?}"), home);
        reader.take(Request::Focus(Some("api".into())));
        assert_instructions(&reader, "api");
        reader.take(Request::Focus(Some("web".into())));
        assert_eq!(reader.settings_session.as_deref(), Some("api"));
        assert_instructions(&reader, "api");
    }
}

#[test]
fn absent_selection_and_closed_overlay_never_adopt_or_schedule_settings() {
    let (_root, mut reader) = reader("issue314-no-selection", None);
    reader.take(Request::Focus(None));
    assert_eq!(reader.settings_session, None);
    reader.take(Request::Settings {
        open: false,
        generation: 41,
    });
    reader.take(Request::Focus(Some("api".into())));
    assert_eq!(reader.settings_session, None);
    assert_eq!(reader.settings, None);
}
