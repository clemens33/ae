//! `ae rename [old] <new>` — the whole rename, under lifecycle locking.
//!
//! A session name is five things at once: a tmux session, a directory under
//! `<AE_HOME>/sessions`, the `session=` row in its meta, part of the
//! `.lifecycle.<name>.lock` filename, and the text in the status bar. The
//! rename moves all five or none, which is why every check that reads state it
//! then mutates happens INSIDE the lock rather than in front of it.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::inventory::ServerId;
use crate::session_tmux::{Op, argv};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::transport;

/// The frozen usage, both lines.
pub const USAGE: &str = "Usage: ae rename [old-name] <new-name>";

/// The second usage line, printed only for the one-operand form: it explains
/// why the old name was needed.
pub const USAGE_INSIDE: &str =
    "(Run inside an ae tmux session to rename it without specifying the old name.)";

/// `rename [old] <new>` — the whole operation.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    root: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let (old, new) = match tail {
        [old, new] => (old.clone(), new.clone()),
        [new] => {
            let Some(old) = current_session(root) else {
                writeln!(err, "{USAGE}")?;
                writeln!(err, "{USAGE_INSIDE}")?;
                return Ok(EXIT_USAGE);
            };
            (old, new.clone())
        }
        _ => {
            writeln!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        }
    };

    // The TARGET is a creation boundary, so it takes the one grammar — not a
    // separator blacklist, which accepted `has space` and persisted a session
    // every later launch refused.
    if !crate::lifecycle::name_is_valid(&new) {
        writeln!(err, "Error: invalid session name '{new}'.")?;
        writeln!(
            err,
            "       Names must match {} — start with a letter or digit,",
            crate::session_launch::name::SESSION_NAME_GRAMMAR
        )?;
        writeln!(
            err,
            "       then letters, digits, '_' or '-', up to 128 characters."
        )?;
        return Ok(EXIT_FAILED);
    }
    if !crate::lifecycle::name_is_usable(root, &old) {
        writeln!(
            err,
            "Error: session name '{old}' cannot be used to reach a session."
        )?;
        return Ok(EXIT_FAILED);
    }
    let sessions = crate::lifecycle::sessions_dir(root);
    for name in [&old, &new] {
        if is_symlink(&sessions.join(name)) {
            writeln!(
                err,
                "Error: the session path for '{name}' is a symlink; refusing to rename through it."
            )?;
            return Ok(EXIT_FAILED);
        }
    }

    // BOTH lifecycle locks, taken in name-sorted order.
    let (first, second) = if old < new {
        (&old, &new)
    } else {
        (&new, &old)
    };
    let held = crate::lifecycle::lock(root, first)
        .and_then(|one| crate::lifecycle::lock(root, second).map(|two| (one, two)));
    let Ok(_held) = held else {
        writeln!(
            err,
            "Error: another lifecycle operation (start/resume/stop/end) is in progress for '{old}' or '{new}' — retry shortly. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    };
    locked(root, &old, &new, out, err)
}

/// Everything inside the two locks: the reads, then the five moves.
fn locked(
    root: &Path,
    old: &str,
    new: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let sessions = crate::lifecycle::sessions_dir(root);
    let old_dir = sessions.join(old);
    let new_dir = sessions.join(new);
    // The server is the OLD session's own recorded one.
    let bytes = crate::meta::read_bytes(&old_dir).unwrap_or_default();
    let Some(server) = crate::session_launch::recorded_server_resolved(&old_dir) else {
        writeln!(
            err,
            "Error: session '{old}' {}. Nothing was renamed.",
            crate::session_launch::AMBIGUOUS_SERVER
        )?;
        return Ok(EXIT_FAILED);
    };

    // Addressed by the EXACT id from here on: `-t proj` prefix-matches, so a
    // rename addressed by name can move a live `project` and report success.
    let Some(session_id) = crate::lifecycle::live_id(&server, old) else {
        writeln!(err, "Error: session '{old}' is not running.")?;
        return Ok(EXIT_FAILED);
    };
    if transport::session_exists(&server, new) {
        writeln!(err, "Error: session '{new}' already exists.")?;
        return Ok(EXIT_FAILED);
    }
    if crate::lifecycle::path_exists(&new_dir) {
        writeln!(
            err,
            "Error: session directory '{}' already exists.",
            new_dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // 1. The tmux session.
    let (renamed, _) = transport::run_tmux_op(&argv(
        &server,
        &Op::RenameSession {
            target: &session_id,
            name: new,
        },
    ));
    if !renamed {
        writeln!(
            err,
            "Error: tmux refused to rename session '{old}'. Nothing was renamed."
        )?;
        return Ok(EXIT_FAILED);
    }

    // 2.
    if crate::lifecycle::meta_value(&bytes, "layout") != "lead-pair" {
        let target = format!("={new}:0");
        let _ = transport::run_tmux_op(&argv(
            &server,
            &Op::RenameWindow {
                target: &target,
                // A window name is a tmux FORMAT: `#(cmd)` runs a shell.
                name: &crate::tmux::format_literal(new),
            },
        ));
    }

    // 3.
    if crate::lifecycle::dir_exists(&old_dir) && std::fs::rename(&old_dir, &new_dir).is_err() {
        writeln!(
            err,
            "Error: the tmux session was renamed to '{new}' but its state directory could not be moved. Run 'ae doctor --refresh' after fixing {}.",
            old_dir.display()
        )?;
        return Ok(EXIT_FAILED);
    }

    // 4.
    if crate::lifecycle::path_exists(&new_dir.join(crate::meta::FILE)) {
        if let Err(why) = crate::meta::rewrite(&new_dir, "session", Some(new)) {
            writeln!(
                err,
                "Error: '{new}' was renamed but its meta still says '{old}' ({}). Run 'ae doctor --refresh {new}'.",
                why.cause()
            )?;
            return Ok(EXIT_FAILED);
        }
        republish(&new_dir, new, &server);
    }

    writeln!(out, "Renamed '{old}' → '{new}'")?;
    Ok(0)
}

/// The manifest and the status bar, both of which name the session.
fn republish(dir: &Path, name: &str, server: &ServerId) {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let origin = or_dot(value("origin"));
    let work_dir = or_dot(value("work_dir"));
    let mode = value("mode");
    let mode = if mode.is_empty() {
        "local".to_owned()
    } else {
        mode
    };
    let main_pane = value("main_pane");
    let main_pane = if main_pane.is_empty() {
        "%0".to_owned()
    } else {
        main_pane
    };
    let mut config_files: Vec<PathBuf> = Vec::new();
    let recorded = value("config");
    if !recorded.is_empty() {
        config_files.push(PathBuf::from(recorded));
    }
    config_files.push(Path::new(&origin).join(".ae").join("config"));
    let manifest = crate::render::manifest_document(
        dir,
        name,
        &work_dir,
        &origin,
        &mode,
        &main_pane,
        &config_files,
    );
    let _ = crate::session_launch::assets::publish_document(&dir.join("workspace.md"), &manifest);
    crate::session_launch::apply_status_bar(
        server,
        name,
        &crate::session_launch::status_paths(&mode, &origin, &work_dir),
    );
}

/// A missing path fact renders as `.`, the frozen default.
fn or_dot(value: String) -> String {
    if value.is_empty() {
        ".".to_owned()
    } else {
        value
    }
}

/// The session the caller is sitting in — the frozen
/// `detect_current_session`: the ambient server's current session, and only
/// when it is a real session directory.
fn current_session(root: &Path) -> Option<String> {
    let name = transport::observe_current_session(&ServerId::Ambient)?;
    crate::lifecycle::dir_exists(&crate::lifecycle::sessions_dir(root).join(&name)).then_some(name)
}

/// Whether `path` is a symlink — the frozen `_require_session_path_safe`.
fn is_symlink(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the rename's path guard must classify the LINK, not what it points at — see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    probe.is_ok_and(|meta| meta.file_type().is_symlink())
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the doors wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-rename-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sessions")).unwrap();
        dir
    }

    #[test]
    fn no_operand_is_a_usage_refusal() {
        let root = scratch("noargs");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &[], &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
        assert!(String::from_utf8_lossy(&err).contains(USAGE));
        assert!(out.is_empty());
    }

    #[test]
    fn more_than_two_operands_is_a_usage_refusal() {
        let root = scratch("threeargs");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["a", "b", "c"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_USAGE);
    }

    #[test]
    fn the_target_takes_the_grammar_and_the_refusal_quotes_it() {
        let root = scratch("grammar");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["old", "has space"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        let text = String::from_utf8_lossy(&err);
        assert!(text.contains("invalid session name 'has space'"), "{text}");
        assert!(
            text.contains(crate::session_launch::name::SESSION_NAME_GRAMMAR),
            "{text}"
        );
        assert!(out.is_empty(), "nothing was renamed");
    }

    #[test]
    fn a_symlinked_session_path_is_refused_before_any_move() {
        let root = scratch("symlink");
        let sessions = root.join("sessions");
        std::fs::create_dir_all(sessions.join("real")).unwrap();
        std::os::unix::fs::symlink(sessions.join("real"), sessions.join("linked")).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["linked", "fresh"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(
            String::from_utf8_lossy(&err).contains("is a symlink"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        // The link is still a link: nothing was moved through it.
        assert!(is_symlink(&sessions.join("linked")));
    }

    #[test]
    fn a_source_that_is_not_running_is_refused_and_moves_nothing() {
        let root = scratch("notrunning");
        let dir = root.join("sessions").join("old");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta"), "session=old\nmode=local\n").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&root, &words(&["old", "new"]), &mut out, &mut err).unwrap();
        assert_eq!(code, EXIT_FAILED);
        assert!(
            String::from_utf8_lossy(&err).contains("is not running"),
            "{}",
            String::from_utf8_lossy(&err)
        );
        assert!(dir.join("meta").exists(), "the source survived");
        assert!(!root.join("sessions").join("new").exists());
    }

    #[test]
    fn the_status_paths_shape_is_mode_aware() {
        assert_eq!(
            crate::session_launch::status_paths("local", "/o", "/w"),
            "/w"
        );
        assert_eq!(
            crate::session_launch::status_paths("git", "/o", "/w"),
            "/o → /w"
        );
    }
}
