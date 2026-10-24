//! End's complete snapshot inventory. Files are streamed through the existing
//! archive store door; links are classified by text and never traversed.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use super::store::{self, read_dir, symlink_meta};

/// Relative source path -> directory (otherwise plain file). Registration
/// entries are excluded only after lstat has established their exact kind.
pub(super) type Inventory = BTreeMap<PathBuf, bool>;

fn refusal(path: &Path, why: impl std::fmt::Display) -> String {
    format!(
        "archive: cannot capture {} ({why}); remove it, then ae end",
        path.display()
    )
}

fn archive_problem(path: &Path, why: impl std::fmt::Display) -> String {
    format!(
        "archive: cannot validate or stage {} ({why})",
        path.display()
    )
}

/// Presence by lstat; an unknown error cannot prove absence.
pub(crate) fn node_exists(path: &Path) -> bool {
    !matches!(symlink_meta(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Report legacy state without traversing a link in its `.ae` component.
pub(crate) fn legacy_state(base: &Path, name: &str) -> Option<PathBuf> {
    let parent = base.join(".ae");
    match symlink_meta(&parent) {
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => None,
        Ok(meta) if meta.is_dir() => {
            let path = parent.join(name);
            node_exists(&path).then_some(path)
        }
        _ => Some(parent),
    }
}

fn helper(name: &str) -> bool {
    name == "_send-deliver" || crate::shim::HELPERS.iter().any(|h| h.name == name)
}

fn registration(name: &str) -> bool {
    if crate::store::registration_file(name)
        || [
            "workspace.md",
            ".watchdog.start.lock",
            ".watchdog.pid",
            ".watchdog-beat",
            "meta-agent-state.json",
            "seatcompact.lock",
        ]
        .contains(&name)
    {
        return true;
    }
    for (prefix, suffix) in [
        ("codex.", ".sid"),
        ("launch.", ".started"),
        ("auto-reseat.", ".lock"),
    ] {
        if name
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(suffix))
            .is_some_and(super::valid_slot)
        {
            return true;
        }
    }
    name.strip_prefix('.')
        .and_then(|n| n.rsplit_once(".tmp."))
        .is_some_and(|(base, pid)| {
            (helper(base)
                || [
                    "workspace.md",
                    crate::store::LAUNCH_ATTEMPT.trim_start_matches('.'),
                ]
                .contains(&base))
                && pid
                    .parse::<u32>()
                    .is_ok_and(|n| n > 0 && n.to_string() == pid)
        })
}

/// Lexical only: neither a dangling helper nor an unowned target is opened.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn registration_link(dir: &Path, path: &Path, name: &str) -> Result<bool, String> {
    let target = store::read_link(path).map_err(|why| refusal(path, why))?;
    if helper(name) {
        return Ok(true);
    }
    let target = normalized(&dir.join(target));
    let versions = store::archive_root_of(dir)
        .and_then(|r| r.parent().map(|p| normalized(&p.join("versions"))));
    Ok(versions.is_some_and(|root| target.starts_with(root))
        || crate::shape::current()
            .command_link()
            .is_some_and(|p| normalized(&p) == target))
}

pub(super) fn inventory(dir: &Path) -> Result<Inventory, String> {
    let mut found = Inventory::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(rel) = todo.pop() {
        let here = dir.join(&rel);
        for path in read_dir(&here).map_err(|why| refusal(&here, why))? {
            let name = path
                .file_name()
                .ok_or_else(|| refusal(&path, "missing name"))?;
            let child = rel.join(name);
            let meta = symlink_meta(&path).map_err(|why| refusal(&path, why))?;
            let direct = rel.as_os_str().is_empty();
            if meta.file_type().is_symlink() {
                if direct && registration_link(dir, &path, &name.to_string_lossy())? {
                    continue;
                }
                return Err(refusal(&path, "unowned symlink"));
            }
            if meta.is_file() && direct && name.to_str().is_some_and(registration) {
                continue;
            }
            if !meta.is_file() && !meta.is_dir() {
                return Err(refusal(&path, "not a plain file or directory"));
            }
            if meta.is_dir() {
                todo.push(child.clone());
            }
            found.insert(child, meta.is_dir());
        }
    }
    Ok(found)
}

/// Advisory read-only refusal before migration. Publication repeats the full
/// proof under its existing source holds; this adds no hold or transaction.
pub(crate) fn preflight(dir: &Path) -> Result<(), String> {
    for (rel, directory) in inventory(dir)? {
        if !directory {
            let path = dir.join(rel);
            store::prove_readable(&path).map_err(|why| refusal(&path, why))?;
        }
    }
    Ok(())
}

/// Missing meta cannot turn arbitrary retained evidence into `Action::Nothing`.
pub(crate) fn has_evidence(dir: &Path) -> bool {
    inventory(dir).map_or(true, |tree| {
        tree.keys().any(|p| {
            p != Path::new(crate::store::META)
                && p != Path::new(crate::store::MEMO)
                && p != Path::new(crate::store::EVENTS)
                && p != Path::new("messages")
        })
    })
}

fn destination(rel: &Path, directory: bool) -> Option<PathBuf> {
    if [crate::store::META, crate::store::MEMO, crate::store::EVENTS]
        .iter()
        .any(|name| rel == Path::new(name))
        || rel == Path::new("messages") && directory
    {
        None
    } else if rel.parent() == Some(Path::new("messages"))
        && !directory
        && rel.extension().is_some_and(|ext| ext == "txt")
    {
        Some(rel.to_path_buf())
    } else {
        Some(Path::new("retained").join(rel))
    }
}

fn parents(path: &Path, root: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut current = path;
    while current != root && !store::exists(current) {
        missing.push(current);
        current = current
            .parent()
            .ok_or_else(|| archive_problem(path, "missing parent"))?;
    }
    for dir in missing.into_iter().rev() {
        store::mkdir_0700(dir).map_err(|why| archive_problem(dir, why))?;
    }
    Ok(())
}

pub(super) fn stage(dir: &Path, payload: &Path, tree: &Inventory) -> Result<(), String> {
    let retained = payload.join("retained");
    store::mkdir_0700(&retained).map_err(|why| archive_problem(&retained, why))?;
    for (rel, directory) in tree {
        let Some(dest) = destination(rel, *directory) else {
            continue;
        };
        let dest = payload.join(dest);
        if *directory {
            parents(&dest, payload)?;
        } else {
            parents(dest.parent().unwrap_or(payload), payload)?;
            store::copy_file_0600(&dir.join(rel), &dest).map_err(|why| {
                archive_problem(
                    &dest,
                    format!("copy from {} failed: {why}", dir.join(rel).display()),
                )
            })?;
        }
    }
    // Persist every directory bottom-up, including empty directories.
    let mut dirs: Vec<_> = plain_tree(&retained)?
        .into_iter()
        .filter_map(|(rel, directory)| directory.then_some(retained.join(rel)))
        .collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for path in dirs.into_iter().chain([retained]) {
        store::fsync_dir(&path).map_err(|why| archive_problem(&path, why))?;
    }
    Ok(())
}

/// Full inventory and byte re-proof immediately before publication. Core
/// sources are compared separately with the coherent buffers held by publish.
pub(super) fn reprove(dir: &Path, payload: &Path, tree: &Inventory) -> Result<(), String> {
    if inventory(dir)? != *tree {
        return Err(refusal(dir, "inventory changed while publishing; retry"));
    }
    for (rel, directory) in tree {
        if let Some(dest) = destination(rel, *directory)
            && !directory
            && !store::files_equal(&dir.join(rel), &payload.join(dest))
                .map_err(|why| refusal(&dir.join(rel), why))?
        {
            return Err(refusal(
                &dir.join(rel),
                "bytes changed while publishing; retry",
            ));
        }
    }
    Ok(())
}

fn plain_tree(root: &Path) -> Result<Inventory, String> {
    let mut tree = Inventory::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(rel) = todo.pop() {
        let path = root.join(&rel);
        let meta = symlink_meta(&path).map_err(|why| archive_problem(&path, why))?;
        if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
            return Err(archive_problem(&path, "archive node is not inert"));
        }
        store::check_mode(&meta, if meta.is_dir() { 0o700 } else { 0o600 })
            .map_err(|mode| archive_problem(&path, format!("archive mode {mode}")))?;
        if meta.is_dir() {
            for child in read_dir(&path).map_err(|why| archive_problem(&path, why))? {
                let name = child
                    .file_name()
                    .ok_or_else(|| archive_problem(&child, "missing name"))?;
                todo.push(rel.join(name));
            }
        }
        tree.insert(rel, meta.is_dir());
    }
    Ok(tree)
}

pub(super) fn validate(root: &Path) -> Result<(), String> {
    plain_tree(root).map(|_| ())
}

pub(super) fn same_tree(left: &Path, right: &Path) -> Result<bool, String> {
    let left_tree = plain_tree(left)?;
    if left_tree != plain_tree(right)? {
        return Ok(false);
    }
    for (rel, directory) in left_tree {
        if !directory
            && !store::files_equal(&left.join(&rel), &right.join(rel))
                .map_err(|why| archive_problem(right, why))?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
