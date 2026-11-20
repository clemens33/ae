//! #310: shell-prefix values removed by env cannot select native state.

use std::cell::RefCell;
use std::path::PathBuf;

use ae::opencode_variant::{Check, check_with};
use ae::store::SourceRead;

#[test]
fn env_unset_and_clear_remove_prior_shell_state_assignments() {
    // The launch oracle is run::peel_env, src/l3_services/run.rs:1953-1972 at ea6def362:
    // -i clears prefix.assign; -u removes that name from prefix.assign.
    // HOME=/h is assigned later. Neither command can keep /old as its XDG root.
    for prefix in [
        "XDG_STATE_HOME=/old env -u XDG_STATE_HOME HOME=/h",
        "XDG_STATE_HOME=/old env -i HOME=/h",
    ] {
        let observed = RefCell::new(Vec::new());
        let result = check_with(
            &format!("{prefix} opencode --agent build --model p/m#max"),
            &|name| match name {
                "HOME" => Some("/ambient-home".into()),
                "XDG_STATE_HOME" => Some("/ambient-state".into()),
                _ => None,
            },
            &|path| {
                observed.borrow_mut().push(path.to_path_buf());
                // Opposed readable values prevent a wrong-path check from
                // accidentally producing the same verdict as the right path.
                let saved = if path == std::path::Path::new("/h/.local/state/opencode/model.json") {
                    "low"
                } else {
                    "max"
                };
                Ok(SourceRead::Ready(
                    format!("{{\"variant\":{{\"p/m\":\"{saved}\"}}}}").into_bytes(),
                ))
            },
        );
        assert_eq!(
            *observed.borrow(),
            vec![PathBuf::from("/h/.local/state/opencode/model.json")],
            "{prefix}"
        );
        assert!(
            matches!(&result, Check::Shadowed { saved, .. } if saved == "low"),
            "{prefix}: {result:?}"
        );
    }
}
