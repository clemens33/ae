//! Old filesystem helper addresses retained by live rename. An alias is only
//! a symlink to a direct sibling; public session lookup still refuses links.
//! Cooperating writers hold the relevant lifecycle locks. As with teardown,
//! pathname operations retain the same-UID substitution residual of std fs.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::teardown::{DirKind, classify_dir};

/// Recognize without following: only a symlink naming a direct sibling can
/// be reclaimed. The target may be absent, so dangling aliases are reusable.
pub(crate) fn target(alias: &Path) -> Option<PathBuf> {
    if !matches!(classify_dir(alias), DirKind::Symlink) {
        return None;
    }
    let sessions = alias.parent()?;
    let link = fs::read_link(alias).ok()?;
    let target = if link.is_absolute() {
        link
    } else {
        sessions.join(link)
    };
    (target.parent() == Some(sessions) && target.file_name().is_some() && target != alias)
        .then_some(target)
}

/// Resolve a helper directory once, to a real directory rather than a chain.
pub(crate) fn resolve(alias: &Path) -> Option<PathBuf> {
    target(alias).filter(|path| matches!(classify_dir(path), DirKind::RealDir))
}

/// Under the wanted name's lifecycle lock, reclaim only a recognized alias.
pub(crate) fn reclaim(alias: &Path) -> io::Result<()> {
    if target(alias).is_some() {
        fs::remove_file(alias)?;
    }
    Ok(())
}

/// Creation is exclusive: a raced file, directory or symlink stays untouched.
fn create_alias(alias: &Path, target: &Path) -> io::Result<()> {
    // A relative state root is already relative to the caller. From inside
    // sessions, only the sibling name avoids repeating that root in the link.
    let target = if target.is_absolute() {
        target
    } else {
        Path::new(
            target
                .file_name()
                .ok_or_else(|| io::Error::other("the alias target has no sibling name"))?,
        )
    };
    std::os::unix::fs::symlink(target, alias)?;
    if fs::read_link(alias)? != target {
        return Err(io::Error::other("the alias target did not verify"));
    }
    Ok(())
}

fn not_kept(alias: &Path, why: &io::Error, err: &mut impl Write) -> io::Result<()> {
    let alias = alias.to_string_lossy().escape_debug().to_string();
    let why = why.to_string().replace(['\r', '\n'], " ");
    writeln!(
        err,
        "Warning: old helper path {alias} was not kept ({why})."
    )
}

/// A failed alias never rolls back or fails the rename. Only reporting can fail.
fn keep_alias(alias: &Path, target: &Path, err: &mut impl Write) -> io::Result<()> {
    if let Err(why) = create_alias(alias, target) {
        not_kept(alias, &why, err)?;
    }
    Ok(())
}

fn entries(root: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let mut aliases = Vec::new();
    if let Some(entries) = super::session_entries(root)? {
        for entry in entries {
            let entry = entry?;
            let alias = entry.path();
            if !entry.file_name().to_string_lossy().starts_with('.')
                && let Some(target) = target(&alias)
            {
                aliases.push((alias, target));
            }
        }
    }
    Ok(aliases)
}

fn retarget_one(
    alias: &Path,
    prior: &Path,
    old_dir: &Path,
    new_dir: &Path,
    err: &mut impl Write,
) -> io::Result<()> {
    if prior == old_dir && target(alias).as_deref() == Some(old_dir) {
        match fs::remove_file(alias) {
            Ok(()) => keep_alias(alias, new_dir, err)?,
            Err(why) => not_kept(alias, &why, err)?,
        }
    }
    Ok(())
}

/// Retarget every prior alias directly, including after a stopped rename.
/// Never replace an entry that has ceased to be our alias.
pub(crate) fn retarget(root: &Path, old: &str, new: &str, err: &mut impl Write) -> io::Result<()> {
    let sessions = super::sessions_dir(root);
    let old_dir = sessions.join(old);
    let new_dir = sessions.join(new);
    let aliases = match entries(root) {
        Ok(aliases) => aliases,
        Err(why) => return not_kept(&old_dir, &why, err),
    };
    for (alias, prior) in aliases {
        retarget_one(&alias, &prior, &old_dir, &new_dir, err)?;
    }
    Ok(())
}

/// Publish before success: flatten prior addresses, then keep the fresh one.
pub(crate) fn keep_live(root: &Path, old: &str, new: &str, err: &mut impl Write) -> io::Result<()> {
    retarget(root, old, new, err)?;
    let sessions = super::sessions_dir(root);
    let new_dir = sessions.join(new);
    if matches!(classify_dir(&new_dir), DirKind::RealDir) {
        keep_alias(&sessions.join(old), &new_dir, err)?;
    }
    Ok(())
}

/// Remove dangling aliases. A target's lock prevents doctor from mistaking
/// the gap during rename for removal. Teardown already holds `removed`'s lock.
pub(crate) fn prune(root: &Path, removed: Option<&Path>) -> io::Result<()> {
    for (alias, prior) in entries(root)? {
        let _held = if removed == Some(prior.as_path()) {
            None
        } else {
            let Some(name) = prior.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let lock = super::sessions_dir(root).join(format!(".lifecycle.{name}.lock"));
            let Ok(held) = crate::store::lock(&lock, std::time::Duration::ZERO) else {
                continue;
            };
            Some(held)
        };
        if matches!(classify_dir(&prior), DirKind::Absent)
            && target(&alias).as_ref() == Some(&prior)
        {
            fs::remove_file(alias)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/it/regsid_aliases_spec.rs"]
mod regsid_aliases_spec;

#[cfg(test)]
#[path = "../../tests/it/regsid_maintenance_spec.rs"]
mod regsid_maintenance_spec;

#[cfg(test)]
#[path = "../../tests/it/regsid_retarget_spec.rs"]
mod regsid_retarget_spec;
