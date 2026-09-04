//! `ae upgrade` — hand the terminal to the immutable sibling installer.
//!
//! The one command that must work on a BROKEN install, because it is the thing
//! that repairs one. It therefore runs AHEAD of the version-directory gate
//! ([`crate::shape::validate`]) and reads none of ae's state: everything it
//! needs is its own location and one environment variable.
//!
//! # `AE_VERSION` is the pin, and it is scoped to this word
//!
//! The installer takes its target version from `AE_VERSION`, which reaches it
//! by INHERITANCE — this process's environment, unchanged, across the exec.
//! Nothing else in ae reads it, and [`crate::transport`]'s spawn door removes
//! it from every child, so an operator pin cannot freeze into the tmux server a
//! launch creates and silently pin an upgrade months later.
//!
//! # Why `exec` and not spawn-and-report
//!
//! The wrapper ran the installer as a child and then printed a session report
//! it derived by reading meta and probing tmux itself. That report is gone with
//! the bash: `ae list` answers "which sessions are running" from one
//! implementation, and a second reader of session state is exactly what slice
//! Z1 removed. What is left is a handover — the installer owns the terminal,
//! its exit status is ae's, and there is no ae process left to misreport it.

use std::path::{Path, PathBuf};

/// The refusal a nonempty argv gets, before the installer is even named.
pub const USAGE: &str = "Usage: AE_VERSION=<calver> ae upgrade";

/// The second line of that refusal: the only supported input is the pin.
pub const USAGE_DETAIL: &str = "ae: upgrade takes no arguments; pin the target with AE_VERSION.";

/// Whether `path` is an immutable bundle member: REGULAR, NON-SYMLINK,
/// executable.
///
/// C51/C67, and the order matters. `is_file` and the executable bit both FOLLOW
/// a symlink, so a member that is a link to a mutable file outside the version
/// directory passes both and is then EXECUTED as if it were ours — on the
/// highest-authority path ae has. lstat first; the mode test only afterwards.
#[must_use]
pub fn is_bundle_member(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the sibling installer is proven a regular, non-symlink, executable member BEFORE it is executed — see clippy.toml"
    )]
    let probe = std::fs::symlink_metadata(path);
    probe.is_ok_and(|meta| meta.file_type().is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The installer this binary would run, or `None` when there is no proven one.
#[must_use]
pub fn installer(exe: &Path) -> Option<PathBuf> {
    let candidate = exe.parent()?.join(crate::shape::INSTALLER);
    is_bundle_member(&candidate).then_some(candidate)
}

/// `ae upgrade` — refuse an argv, prove the sibling, then become it.
///
/// Returns only when it did NOT hand over: a usage error, an unprovable
/// sibling, or an exec that failed. A successful exec has no return.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    tail: &[String],
    out: &mut impl std::io::Write,
    err: &mut impl std::io::Write,
) -> crate::Result<u8> {
    if !tail.is_empty() {
        writeln!(err, "{USAGE}")?;
        writeln!(err, "{USAGE_DETAIL}")?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    }
    let Some(exe) = crate::shape::resolved_exe() else {
        writeln!(
            err,
            "ae: this binary cannot name its own path, so it cannot find the installer beside it."
        )?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    let Some(installer) = installer(&exe) else {
        writeln!(
            err,
            "ae: no installer beside {} — an installed ae upgrades through the '{}' its version directory ships; a checkout build upgrades by rebuilding it.",
            exe.display(),
            crate::shape::INSTALLER
        )?;
        err.flush()?;
        return Ok(crate::entry::EXIT_USAGE);
    };
    writeln!(out, "ae upgrade: running {}", installer.display())?;
    out.flush()?;
    err.flush()?;
    let why = exec(&installer);
    writeln!(err, "ae: could not run {}: {why}", installer.display())?;
    err.flush()?;
    Ok(crate::entry::EXIT_FAILED)
}

/// Become the installer. Returns only the error that stopped it.
///
/// THE THIRD PRODUCT CROSSING of `clippy.toml`'s `Command` deny, and the second
/// that is an `exec` rather than a spawn: like `_run`'s, this process IS
/// replaced, so there is no parent left to misreport what the installer did and
/// its exit status reaches the caller unmediated.
fn exec(installer: &Path) -> std::io::Error {
    use std::os::unix::process::CommandExt as _;
    #[allow(
        clippy::disallowed_types,
        reason = "upgrade's own exec: ae BECOMES the immutable sibling installer, so its exit status is ae's and no parent can misreport the repair"
    )]
    let mut command = std::process::Command::new(installer);
    command.exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_is_the_sibling_of_the_running_binary() {
        // Nothing is proven on disk here — that is the door's job — but the
        // NAME and the directory are this function's, and a wrong one would
        // send the highest-authority path at the wrong file.
        assert_eq!(
            Path::new("/u/me/.ae/versions/1.2.3/ae-core")
                .parent()
                .map(|dir| dir.join(crate::shape::INSTALLER)),
            Some(PathBuf::from("/u/me/.ae/versions/1.2.3/install"))
        );
    }

    #[test]
    fn an_argv_is_a_usage_error_before_anything_is_named() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&["2026.9.1".to_owned()], &mut out, &mut err).expect("writes");
        assert_eq!(code, crate::entry::EXIT_USAGE);
        assert!(out.is_empty(), "nothing is run");
        let text = String::from_utf8(err).expect("utf8");
        assert!(text.contains(USAGE), "{text}");
        assert!(text.contains("AE_VERSION"), "{text}");
    }
}
