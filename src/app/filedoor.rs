//! The file reader's one door onto the file system: a file a lane links,
//! read for the reader panel, or a folder it links, listed as a bounded tree
//! — and only from the loader thread.
//!
//! A path in a lane is hostile text (an agent wrote it), and the link scanner
//! links more than the reader opens: any absolute path, `..` kept as written.
//! So the door judges the CANONICAL target against the CANONICAL roots the
//! link base names, the session's work dir and the viewer's home: a path
//! outside both, by `..` or by a symlink, is refused by name (the terminal's
//! own OSC 8 open still reaches it), while a link that stays inside is
//! followed. A target must be a regular file, checked BEFORE it is opened (a
//! FIFO or a device opened blindly would block the loader for good) and again
//! on the opened handle; it is read once, bounded.
//!
//! A folder target ([`open`] only) is listed, never opened entry by entry:
//! each entry's kind comes from the listing itself, which never follows a
//! symlink, so a link is a link entry and is never descended, and a FIFO or a
//! device is named and never opened. Only real folders are listed in turn,
//! breadth first, within [`TREE_MAX_DEPTH`] levels, [`TREE_MAX_ENTRIES`]
//! listed entries and [`TREE_MAX_READ`] entries read; [`TREE_COLLAPSED`]
//! folders are named and never listed.
//!
//! NAMED RESIDUAL: between the canonicalisation and the gate and the open a
//! same-user process can swap the file (to a FIFO, say). The re-check on the
//! opened handle narrows that and does not close it. A folder swapped while it
//! is walked is listed as it is found; a listing opens a folder only, so a
//! swapped-in FIFO fails the listing rather than blocking it. A child folder
//! is listed in turn only while its canonical path is still its parent's
//! joined with its name, so one swapped for a symlink before that check is
//! named `changed while listed` and not descended. One swapped between the
//! check and its listing is still followed: if it points outside both roots,
//! the names under its target are listed (never their contents), down to the
//! depth left.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Read as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::FileTypeExt as _;
use std::path::{Path, PathBuf};

/// The deepest level a folder's tree lists: the linked folder's own entries
/// are level 1, and a folder at this level is listed but not read.
pub const TREE_MAX_DEPTH: usize = 3;
/// The most entries a folder's tree lists in all, every level counted.
pub const TREE_MAX_ENTRIES: usize = 300;
/// The most entries a folder's tree reads from its listings in all, the
/// listed and the cut ones counted: the walk's bound on work. The listing
/// that meets the bound fetches one entry more, unread, only to say whether
/// its cut count is exact; every later folder is not listed at all.
pub const TREE_MAX_READ: usize = 10_000;
/// The folders a tree names and never lists.
pub const TREE_COLLAPSED: [&str; 2] = [".git", "target"];

/// The roots a file may be read under: the session's recorded work dir and the
/// viewer's home. A root that does not canonicalise is no root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    pub dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

/// What a read came to: the file's bytes and its length on disk, a folder's
/// tree, or the named reason it was refused.
#[derive(Debug)]
pub enum Read {
    Bytes { bytes: Vec<u8>, size: u64 },
    Tree(Tree),
    Refused(String),
}

/// What an entry is, by its listing: a symlink is a link whatever it points
/// at, and only a real folder is ever listed in turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File,
    Link,
    Fifo,
    Socket,
    /// A device, or a kind the listing could not say.
    Other,
}

/// What became of a folder's own entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listed {
    /// Listed: its children are its entries, cut ones counted in `more`.
    Walked,
    /// One of [`TREE_COLLAPSED`]: named, never listed.
    Collapsed,
    /// At [`TREE_MAX_DEPTH`]: named, not listed.
    Deep,
    /// The entry or read budget ran out before it was listed.
    Spent,
    /// Its listing failed, by the error's kind.
    Unreadable(String),
    /// Its real path was no longer its parent's joined with its name when
    /// its turn came (swapped for a link, say): named, not listed.
    Moved,
}

/// One entry of a tree. The root is node 0, with an empty name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The entry's name as the listing gave it, bytes kept.
    pub name: OsString,
    pub kind: Kind,
    /// A folder's state; [`Listed::Walked`] for anything else.
    pub listed: Listed,
    /// The node indices of the listed entries, folders first, then by name.
    pub children: Vec<usize>,
    /// Entries read here and not listed.
    pub more: usize,
    /// `false` when the read budget ran out inside this folder's listing, so
    /// `more` is a lower bound.
    pub exact: bool,
}

/// A folder's bounded tree: [`Node`]s, the root first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

/// `why`, as a refusal.
fn refuse(why: impl Into<String>) -> Read {
    Read::Refused(why.into())
}

/// An I/O error as the reason a read stopped: a missing file is `not found`,
/// anything else `unreadable` and its kind.
fn failed(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        "not found".to_owned()
    } else {
        format!("unreadable: {}", error.kind())
    }
}

/// `path` read, at most `max` bytes, if it is a regular file under a root.
/// The refusals come in this order: no usable root, the target missing or
/// unreadable, outside every root, not a regular file, longer than `max`.
/// The reader opens through [`open`]; this file-only door stays for the
/// acceptance that pins its refusals.
#[cfg(test)]
#[must_use]
pub fn read(path: &Path, roots: &Roots, max: usize) -> Read {
    door(path, roots, max, false)
}

/// `path` read as a regular file is (at most `max` bytes, the same refusals
/// in the same order), except that a folder under a root becomes its bounded
/// [`Tree`]. A folder whose own listing fails is refused by the error, as a
/// file is.
#[must_use]
pub fn open(path: &Path, roots: &Roots, max: usize) -> Read {
    door(path, roots, max, true)
}

/// The one gated read behind `read` and [`open`]; `folders` lets a folder
/// through to [`walk`].
fn door(path: &Path, roots: &Roots, max: usize, folders: bool) -> Read {
    let held: Vec<PathBuf> = [roots.dir.as_deref(), roots.home.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    if held.is_empty() {
        return refuse("no base to read against");
    }
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(error) => return refuse(failed(&error)),
    };
    if !held.iter().any(|root| canonical.starts_with(root)) {
        return refuse("outside the session dir and home");
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the reader's regular-file gate, before the target is opened — see clippy.toml"
    )]
    let gate = std::fs::metadata(&canonical);
    match gate {
        Ok(meta) if meta.is_file() => {}
        Ok(meta) if folders && meta.is_dir() => return walk(&canonical),
        Ok(_) => return refuse("not a regular file"),
        Err(error) => return refuse(failed(&error)),
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the reader's one open of a gated regular file, read once and bounded — see clippy.toml"
    )]
    let opened = std::fs::File::open(&canonical);
    let file = match opened {
        Ok(file) => file,
        Err(error) => return refuse(failed(&error)),
    };
    let size = match file.metadata() {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(_) => return refuse("not a regular file"),
        Err(error) => return refuse(failed(&error)),
    };
    let limit = u64::try_from(max).unwrap_or(u64::MAX);
    if size > limit {
        return refuse(format!("too large: {size} bytes (limit {max})"));
    }
    let mut bytes = Vec::new();
    if let Err(error) = file.take(limit.saturating_add(1)).read_to_end(&mut bytes) {
        return refuse(failed(&error));
    }
    // The file grew past `max` since it was measured.
    let taken = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if taken > limit {
        return refuse(format!(
            "too large: {} bytes (limit {max})",
            size.max(taken)
        ));
    }
    Read::Bytes { bytes, size }
}

/// An entry's kind by the listing's own file type, which never follows a
/// symlink.
fn kind_of(kind: std::fs::FileType) -> Kind {
    if kind.is_symlink() {
        Kind::Link
    } else if kind.is_dir() {
        Kind::Folder
    } else if kind.is_file() {
        Kind::File
    } else if kind.is_fifo() {
        Kind::Fifo
    } else if kind.is_socket() {
        Kind::Socket
    } else {
        Kind::Other
    }
}

/// Why the child folder at `path` (its canonical parent joined with its
/// name) is not listed in turn: its canonical path is another
/// ([`Listed::Moved`]) or cannot be had ([`Listed::Unreadable`]). `None` lets
/// it be listed.
fn moved(path: &Path) -> Option<Listed> {
    match std::fs::canonicalize(path) {
        Ok(real) if real == path => None,
        Ok(_) => Some(Listed::Moved),
        Err(error) => Some(Listed::Unreadable(error.kind().to_string())),
    }
}

/// A node with no entries listed yet.
fn node(name: OsString, kind: Kind, listed: Listed) -> Node {
    Node {
        name,
        kind,
        listed,
        children: Vec::new(),
        more: 0,
        exact: true,
    }
}

/// The canonical folder `root`'s tree, breadth first: every folder of one
/// level is listed before any of the next, so a cut falls on the deepest and
/// the last. Each listing reads at most what is left of [`TREE_MAX_READ`],
/// sorts what it read (folders first, then by the name's bytes) and keeps at
/// most what is left of [`TREE_MAX_ENTRIES`]. The root's own listing failing
/// refuses the read; a deeper one failing is that folder's state.
fn walk(root: &Path) -> Read {
    let mut tree = Tree {
        nodes: vec![node(OsString::new(), Kind::Folder, Listed::Walked)],
    };
    let mut queue = VecDeque::from([(0, root.to_path_buf(), 0)]);
    let (mut listed, mut read) = (0, 0);
    while let Some((at, path, depth)) = queue.pop_front() {
        if listed >= TREE_MAX_ENTRIES || read >= TREE_MAX_READ {
            tree.nodes[at].listed = Listed::Spent;
            continue;
        }
        if at != 0
            && let Some(state) = moved(&path)
        {
            tree.nodes[at].listed = state;
            continue;
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the reader's folder tree lists a canonical folder under a root — see clippy.toml"
        )]
        let listing = std::fs::read_dir(&path);
        let entries = match listing {
            Ok(entries) => entries,
            Err(error) if at == 0 => return refuse(failed(&error)),
            Err(error) => {
                tree.nodes[at].listed = Listed::Unreadable(format!("{}", error.kind()));
                continue;
            }
        };
        // A listing that fails part way ends there, its count a lower bound.
        let (mut found, mut exact) = (Vec::new(), true);
        for entry in entries {
            let Ok(entry) = entry else {
                exact = false;
                break;
            };
            if read >= TREE_MAX_READ {
                exact = false;
                break;
            }
            read += 1;
            let kind = entry.file_type().map_or(Kind::Other, kind_of);
            found.push((entry.file_name(), kind));
        }
        found.sort_by(|(a, a_kind), (b, b_kind)| {
            (*b_kind == Kind::Folder)
                .cmp(&(*a_kind == Kind::Folder))
                .then_with(|| a.as_bytes().cmp(b.as_bytes()))
        });
        let keep = found.len().min(TREE_MAX_ENTRIES - listed);
        let cut = found.len() - keep;
        for (name, kind) in found.into_iter().take(keep) {
            let state = if kind != Kind::Folder {
                Listed::Walked
            } else if TREE_COLLAPSED
                .iter()
                .any(|known| name.as_bytes() == known.as_bytes())
            {
                Listed::Collapsed
            } else if depth + 1 >= TREE_MAX_DEPTH {
                Listed::Deep
            } else {
                Listed::Walked
            };
            let child = tree.nodes.len();
            if kind == Kind::Folder && state == Listed::Walked {
                queue.push_back((child, path.join(&name), depth + 1));
            }
            tree.nodes.push(node(name, kind, state));
            tree.nodes[at].children.push(child);
        }
        listed += keep;
        let parent = &mut tree.nodes[at];
        parent.more = cut;
        parent.exact = exact;
    }
    Read::Tree(tree)
}

#[cfg(test)]
mod tests {
    use super::{Kind, Listed, Read, Roots, TREE_MAX_ENTRIES, TREE_MAX_READ, Tree, open};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ae-filedoor-tree-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("the fixture root is made");
        dir
    }

    fn tree_of(root: &Path) -> Tree {
        let roots = Roots {
            dir: Some(root.to_path_buf()),
            home: None,
        };
        let Read::Tree(tree) = open(root, &roots, 100) else {
            panic!("the folder is no tree");
        };
        tree
    }

    #[test]
    fn the_read_bound_stops_a_listing_and_its_count_is_a_lower_bound() {
        let root = fixture("read");
        for n in 0..TREE_MAX_READ + 3 {
            fs::write(root.join(format!("f{n:06}")), b"").expect("the fixture file is written");
        }
        let tree = tree_of(&root);
        let top = &tree.nodes[0];
        assert_eq!(top.children.len(), TREE_MAX_ENTRIES);
        assert_eq!(top.more, TREE_MAX_READ - TREE_MAX_ENTRIES);
        assert!(!top.exact, "the listing was cut by the read bound");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_listing_of_exactly_the_read_bound_is_exact() {
        let root = fixture("exact");
        for n in 0..TREE_MAX_READ {
            fs::write(root.join(format!("f{n:06}")), b"").expect("the fixture file is written");
        }
        let top = tree_of(&root).nodes[0].clone();
        assert_eq!(top.more, TREE_MAX_READ - TREE_MAX_ENTRIES);
        assert!(top.exact, "every entry was read");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_left_when_the_entry_budget_is_spent_is_named_not_listed() {
        let root = fixture("spent");
        fs::create_dir(root.join("aaa-folder")).expect("the folder is made");
        fs::write(root.join("aaa-folder/inner"), b"").expect("the fixture file is written");
        for n in 0..TREE_MAX_ENTRIES {
            fs::write(root.join(format!("f{n:06}")), b"").expect("the fixture file is written");
        }
        let tree = tree_of(&root);
        let top = &tree.nodes[0];
        assert_eq!(top.children.len(), TREE_MAX_ENTRIES);
        assert_eq!(top.more, 1);
        let folder = &tree.nodes[top.children[0]];
        assert_eq!(
            (folder.kind, &folder.listed),
            (Kind::Folder, &Listed::Spent)
        );
        assert!(folder.children.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_child_whose_real_path_moved_is_not_listed_in_turn() {
        let root = fs::canonicalize(fixture("moved")).expect("the fixture root canonicalises");
        let outside = fixture("moved-outside");
        fs::create_dir(root.join("real")).expect("the folder is made");
        std::os::unix::fs::symlink(&outside, root.join("swapped")).expect("the link is made");
        assert_eq!(super::moved(&root.join("real")), None);
        assert_eq!(super::moved(&root.join("swapped")), Some(Listed::Moved));
        assert_eq!(
            super::moved(&root.join("gone")),
            Some(Listed::Unreadable(std::io::ErrorKind::NotFound.to_string()))
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_small_folder_is_its_whole_tree_and_only_real_folders_are_listed_in_turn() {
        let root = fixture("small");
        fs::create_dir_all(root.join("a/b")).expect("the folders are made");
        fs::write(root.join("a/b/c"), b"").expect("the fixture file is written");
        fs::write(root.join("a/f"), b"").expect("the fixture file is written");
        fs::write(root.join("z"), b"").expect("the fixture file is written");
        std::os::unix::fs::symlink(root.join("a"), root.join("l")).expect("the link is made");
        let tree = tree_of(&root);
        let shape: Vec<(String, Kind, Listed, usize)> = tree
            .nodes
            .iter()
            .map(|node| {
                (
                    node.name.to_string_lossy().into_owned(),
                    node.kind,
                    node.listed.clone(),
                    node.children.len(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                (String::new(), Kind::Folder, Listed::Walked, 3),
                ("a".to_owned(), Kind::Folder, Listed::Walked, 2),
                ("l".to_owned(), Kind::Link, Listed::Walked, 0),
                ("z".to_owned(), Kind::File, Listed::Walked, 0),
                ("b".to_owned(), Kind::Folder, Listed::Walked, 1),
                ("f".to_owned(), Kind::File, Listed::Walked, 0),
                ("c".to_owned(), Kind::File, Listed::Walked, 0),
            ]
        );
        assert!(tree.nodes.iter().all(|node| node.more == 0 && node.exact));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_linked_folder_that_cannot_be_listed_is_refused_by_its_error() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = fixture("locked");
        let locked = root.join("locked");
        fs::create_dir(&locked).expect("the folder is made");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))
            .expect("the folder is locked");
        #[allow(
            clippy::disallowed_methods,
            reason = "test: whether this process can list the locked fixture (root can)"
        )]
        let listable = fs::read_dir(&locked).is_ok();
        let roots = Roots {
            dir: Some(root.clone()),
            home: None,
        };
        let answer = open(&locked, &roots, 100);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))
            .expect("the folder is unlocked");
        if !listable {
            assert!(
                matches!(&answer, Read::Refused(why) if why == "unreadable: permission denied"),
                "{answer:?}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_still_refuses_a_folder_that_open_lists() {
        let root = fixture("both");
        let roots = Roots {
            dir: Some(root.clone()),
            home: None,
        };
        assert!(matches!(
            super::read(&root, &roots, 100),
            Read::Refused(why) if why == "not a regular file"
        ));
        assert!(matches!(open(&root, &roots, 100), Read::Tree(_)));
        let _ = fs::remove_dir_all(&root);
    }
}
