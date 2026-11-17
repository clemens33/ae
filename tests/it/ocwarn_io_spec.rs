//! Independent I/O-boundary acceptance for #310. Only external reads are faked.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use ae::opencode_variant::{Check, check_with};
use ae::store::SourceRead;

const COMMAND: &str = "opencode --agent build --model p/m#max";

#[test]
fn no_explicit_variant_means_zero_environment_and_preference_reads() {
    for cmd in [
        "opencode",
        "opencode --agent build",
        "opencode --agent build --model p/m",
        "claude --model p/m#max",
        "codex --model p/m#max",
        "opencode --model p/m#max",
    ] {
        let environment_reads = Cell::new(0);
        let file_reads = Cell::new(0);
        let result = check_with(
            cmd,
            &|_| {
                environment_reads.set(environment_reads.get() + 1);
                Some("/must-not-be-read".into())
            },
            &|_| {
                file_reads.set(file_reads.get() + 1);
                Ok(SourceRead::Ready(br#"{"variant":{"p/m":"low"}}"#.to_vec()))
            },
        );
        assert_eq!(result, Check::NoRequest, "{cmd}");
        assert_eq!(environment_reads.get(), 0, "no env read for {cmd}");
        assert_eq!(file_reads.get(), 0, "no file read for {cmd}");
    }
    // Sensitivity control: the same injected reader must be observed when the
    // explicit request qualifies. An inert or never-called seam cannot pass.
    let reads = Cell::new(0);
    let result = check_with(COMMAND, &|_| Some("/fixture".into()), &|_| {
        reads.set(reads.get() + 1);
        Ok(SourceRead::Ready(br#"{"variant":{"p/m":"low"}}"#.to_vec()))
    });
    assert!(matches!(result, Check::Shadowed { .. }), "{result:?}");
    assert_eq!(reads.get(), 1);
}

#[test]
fn effective_launch_environment_selects_the_preference_path() {
    for (prefix, inherited_xdg, expected) in [
        ("", Some("/inherited"), "/inherited/opencode/model.json"),
        ("", None, "/home-fixture/.local/state/opencode/model.json"),
        (
            "",
            Some(""),
            "/home-fixture/.local/state/opencode/model.json",
        ),
        (
            "env -u XDG_STATE_HOME ",
            Some("/inherited"),
            "/home-fixture/.local/state/opencode/model.json",
        ),
        (
            "env XDG_STATE_HOME=/selected ",
            Some("/inherited"),
            "/selected/opencode/model.json",
        ),
        (
            "XDG_STATE_HOME=/selected ",
            Some("/inherited"),
            "/selected/opencode/model.json",
        ),
        (
            "env -i HOME=/isolated ",
            Some("/inherited"),
            "/isolated/.local/state/opencode/model.json",
        ),
        (
            "env -u XDG_STATE_HOME XDG_STATE_HOME=/last ",
            Some("/inherited"),
            "/last/opencode/model.json",
        ),
        (
            "env XDG_STATE_HOME=$STATE_ROOT ",
            Some("/inherited"),
            "/expanded/opencode/model.json",
        ),
    ] {
        let paths = RefCell::new(Vec::new());
        let result = check_with(
            &format!("{prefix}{COMMAND}"),
            &|name| match name {
                "HOME" => Some("/home-fixture".into()),
                "XDG_STATE_HOME" => inherited_xdg.map(str::to_owned),
                "STATE_ROOT" => Some("/expanded".into()),
                _ => None,
            },
            &|path| {
                paths.borrow_mut().push(path.to_path_buf());
                Ok(SourceRead::Ready(br#"{"variant":{"p/m":"low"}}"#.to_vec()))
            },
        );
        assert_eq!(*paths.borrow(), vec![PathBuf::from(expected)], "{prefix}");
        assert!(
            matches!(result, Check::Shadowed { .. }),
            "{prefix}: {result:?}"
        );
    }
}

#[test]
fn unresolved_state_is_unknown_without_guessing_a_file() {
    for cmd in [
        format!("env -i {COMMAND}"),
        format!("env -i HOME=relative {COMMAND}"),
        format!("env XDG_STATE_HOME=relative {COMMAND}"),
    ] {
        let reads = Cell::new(0);
        let result = check_with(
            &cmd,
            &|name| (name == "HOME").then(|| "/fixture".into()),
            &|_| {
                reads.set(reads.get() + 1);
                Ok(SourceRead::Absent)
            },
        );
        assert!(matches!(result, Check::Unknown(_)), "{cmd}: {result:?}");
        assert_eq!(reads.get(), 0, "do not guess a path: {cmd}");
    }
}

#[test]
fn only_a_usable_conflicting_value_for_the_exact_model_is_a_shadow() {
    for (bytes, expected) in [
        (br"{}".as_slice(), "clear"),
        (br#"{"variant":{}}"#, "clear"),
        (br#"{"variant":{"other/m":"low"}}"#, "clear"),
        (br#"{"variant":{"p/m":"max"}}"#, "clear"),
        // v2.3.0 model-preference.ts:109-112 reads JSON through
        // decodeModelPreference (:77-84), whose variants() (:30-38) drops
        // empty strings and non-strings before preferredSelection sees them.
        // local.tsx:157,162-171 installs that decoded map. Thus these values
        // cannot stop the preferred ?? configured chain at local.tsx:242-250.
        // https://github.com/anomalyco/opencode/blob/v2.3.0/packages/tui/src/model-preference.ts#L30-L38
        (br#"{"variant":{"p/m":""}}"#, "clear"),
        (br#"{"variant":{"p/m":12}}"#, "clear"),
        (br#"{"variant":{"p/m":"low"}}"#, "low"),
        (br#"{"variant":{"p/m":"default"}}"#, "default"),
        (br#"{"variant":{"p/m":"low","p/m":"max"}}"#, "clear"),
        (br#"{"variant":{"p/m":"max","p/m":"low"}}"#, "low"),
        (
            br#"{"variant":{"p/m":"low"},"variant":{"p/m":"max"}}"#,
            "clear",
        ),
        (
            br#"{"variant":{"p/m":"max"},"variant":{"p/m":"low"}}"#,
            "low",
        ),
        (b"{", "unknown"),
        (b"\xff", "unknown"),
        (br"[]", "unknown"),
        (br#"{"variant":[]}"#, "unknown"),
    ] {
        let result = check_with(COMMAND, &|_| Some("/fixture".into()), &|_: &Path| {
            Ok(SourceRead::Ready(bytes.to_vec()))
        });
        match (expected, &result) {
            ("clear", Check::Clear { .. }) | ("unknown", Check::Unknown(_)) => {}
            (
                expected_saved,
                Check::Shadowed {
                    key,
                    configured,
                    saved,
                    ..
                },
            ) => {
                assert_eq!(key, "p/m");
                assert_eq!(configured, "max");
                assert_eq!(saved, expected_saved);
            }
            _ => panic!("{bytes:?}: expected {expected}, got {result:?}"),
        }
    }
}

#[test]
fn reader_failures_are_unknown_but_absence_is_clear() {
    for input in [
        Ok(SourceRead::Unreadable("permission denied".into())),
        Ok(SourceRead::Invalid("symbolic link".into())),
        Err(ae::store::Oversized),
    ] {
        let result = check_with(COMMAND, &|_| Some("/fixture".into()), &|_| input.clone());
        assert!(matches!(result, Check::Unknown(_)), "{result:?}");
    }
    let result = check_with(COMMAND, &|_| Some("/fixture".into()), &|_| {
        Ok(SourceRead::Absent)
    });
    assert!(matches!(result, Check::Clear { .. }), "{result:?}");
}
