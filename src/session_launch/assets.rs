//! What a session directory holds besides its meta: the helper LINKS and the
//! workspace manifest.
//!
//! Slice Z2 finished the move the B ruling started. A helper was a four-line
//! bash shim that exec'd the core; it is now a SYMLINK to the core binary, and
//! the shim's one `exec` line lives in [`crate::shim`], which reads the helper's
//! identity and its session directory off `argv[0]`. Helper names and argv are
//! unchanged, which is the compatibility contract that matters — every agent in
//! a live workspace calls them by name.
//!
//! The 422-line `sync_session_assets` generator, the `_lib` library it emitted,
//! the `declare -f` template pattern behind both, and now the shim bodies
//! themselves are all gone: there is no bash left in a session directory.
//!
//! Each artifact is published temp + rename, the frozen
//! `_publish_executable_artifact` shape: a writer that dies mid-publish leaves
//! the previous artifact whole rather than a half-made one a session would then
//! run.

use std::io;
use std::path::Path;

/// Publish a non-executable artifact atomically, mode 0600.
///
/// # Errors
///
/// The underlying [`io::Error`].
pub(crate) fn publish_document(dest: &Path, body: &str) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let (dir, name) = split_dest(dest)?;
    let temp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&temp, dest)
}

/// A destination split into the directory it is published in and its name.
fn split_dest(dest: &Path) -> io::Result<(&Path, &str)> {
    let Some(dir) = dest.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an artifact path with no directory",
        ));
    };
    let Some(name) = dest.file_name().and_then(|n| n.to_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an artifact path with no name",
        ));
    };
    Ok((dir, name))
}

/// Publish one helper link: symlink a temp beside the destination, then rename
/// it over.
fn link(dest: &Path, core: &Path) -> io::Result<()> {
    let (dir, name) = split_dest(dest)?;
    let temp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    std::os::unix::fs::symlink(core, &temp)?;
    match std::fs::rename(&temp, dest) {
        Ok(()) => Ok(()),
        Err(why) => {
            let _ = std::fs::remove_file(&temp);
            Err(why)
        }
    }
}

/// Link every helper of a session into `dir`, pointing at `core`.
///
/// # Errors
///
/// The first artifact that could not be published, with its path — a session
/// missing a helper is not a session, so the caller rolls back rather than
/// starting agents that cannot talk to each other.
pub(crate) fn write_helpers(dir: &Path, core: &Path) -> Result<(), String> {
    for helper in crate::shim::HELPERS {
        let dest = dir.join(helper.name);
        link(&dest, core).map_err(|why| format!("could not link {} ({why})", dest.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests read back what the door wrote; the boundary is on product code — see clippy.toml"
)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_named_helper_is_a_link_to_the_core() {
        let dir = tempdir();
        let core = dir.join("ae-core");
        std::fs::write(&core, b"#!/bin/sh\n").unwrap();
        write_helpers(&dir, &core).unwrap();
        for helper in crate::shim::HELPERS {
            let path = dir.join(helper.name);
            let meta = std::fs::symlink_metadata(&path)
                .unwrap_or_else(|why| panic!("{} was not published ({why})", helper.name));
            assert!(meta.file_type().is_symlink(), "{}", helper.name);
            assert_eq!(
                std::fs::read_link(&path).unwrap(),
                core,
                "{} points at the core",
                helper.name
            );
        }
    }

    #[test]
    fn a_republish_replaces_a_link_that_is_already_there() {
        let dir = tempdir();
        let old = dir.join("old-core");
        let new = dir.join("new-core");
        write_helpers(&dir, &old).unwrap();
        write_helpers(&dir, &new).unwrap();
        assert_eq!(std::fs::read_link(dir.join("send")).unwrap(), new);
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp.")),
            "no temp survives a publish"
        );
    }

    #[test]
    fn a_destination_that_cannot_be_replaced_names_itself() {
        let dir = tempdir();
        std::fs::create_dir(dir.join("peek")).unwrap();
        std::fs::write(dir.join("peek").join("occupied"), b"x").unwrap();
        let why =
            write_helpers(&dir, Path::new("/c")).expect_err("a non-empty dir cannot be replaced");
        assert!(why.contains("peek"), "{why}");
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "ae-assets-{}-{}",
            std::process::id(),
            crate::launch::generate_uuid()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }
}
