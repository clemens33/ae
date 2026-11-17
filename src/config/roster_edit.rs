//! The one in-place edit ae makes to the human's global config: the value of
//! the `[roster] orchestrator` row.
//!
//! `ae init` creates the file, or replaces it whole after a backup. Nothing
//! else edits an existing hand-edited config, so this module is deliberately
//! narrow. The pure half, [`set_roster_row`], rewrites the value span of that
//! one row and every other byte stays as it was; a layout it cannot place
//! exactly is a refusal, never a guess. The file half, [`apply_roster_row`],
//! publishes only while the bytes are still the bytes the human was shown.
//!
//! The rewriter reads the text through the reader's own line iterator
//! ([`config_lines`]) and the reader's own value grammar, so it sees the rows
//! the launch sees: the body of a `"""` block is never mistaken for a row.
//! After the rewrite it parses the result again and refuses unless the only
//! difference from the input's parse is that one binding.

use std::fmt;
use std::io;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use super::{
    ConfigLine, IdentityConfig, config_lines, is_config_key, key_claim, parse_identity_in,
    read_selected, section_header, upsert,
};

/// The one editable `[roster]` row. It is the orchestrator seat's name, spelled
/// here because the seat module is a higher layer than config.
const ROW: &str = "orchestrator";

/// The section that holds it.
const ROSTER: &str = "roster";

/// What a diagnostic names when the text has no file.
const SOURCE: &str = "<config>";

/// The text a rewrite produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEdit {
    /// The whole config text after the edit.
    pub text: String,
    /// Whether the edit changed anything: `false` for the pick already bound.
    pub changed: bool,
}

/// Why a config text cannot take the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterEditRefusal {
    /// The row to edit is not the orchestrator row.
    Row(String),
    /// The text is outside the config grammar the reader accepts.
    Config(String),
    /// No `[roster] orchestrator` row.
    Absent,
    /// More than one `[roster] orchestrator` row.
    Duplicate,
    /// The row has a shape the writer cannot place exactly.
    Shape {
        /// 1-based line of the row.
        line: usize,
        /// What is wrong with it.
        why: &'static str,
    },
    /// The profile is not a profile name.
    ProfileGrammar,
    /// The profile is not defined in `[profiles]`.
    ProfileUnknown(String),
}

impl fmt::Display for RosterEditRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Row(name) => write!(f, "only the orchestrator row is editable, not '{name}'"),
            Self::Config(why) => write!(f, "the config cannot be read: {why}"),
            Self::Absent => f.write_str("the config has no [roster] orchestrator row"),
            Self::Duplicate => {
                f.write_str("the config has more than one [roster] orchestrator row")
            }
            Self::Shape { line, why } => write!(f, "line {line}: {why}"),
            Self::ProfileGrammar => f.write_str("the profile is not a profile name"),
            Self::ProfileUnknown(profile) => {
                write!(f, "profile '{profile}' is not defined in [profiles]")
            }
        }
    }
}

impl std::error::Error for RosterEditRefusal {}

/// Where the value of the one row sits.
struct Placed {
    /// 1-based line of the row.
    line: usize,
    /// The value's byte range in the whole text: the bare token, or what lies
    /// between a quoted value's quotes.
    span: std::ops::Range<usize>,
    /// The value as the reader binds it.
    current: String,
}

/// Rewrite the value of the `[roster] <name>` row to `profile`, touching no
/// other byte. Only the orchestrator row is editable, and `profile` must be
/// defined in the same text's `[profiles]`.
///
/// # Errors
///
/// A [`RosterEditRefusal`] for a text, a row or a profile the edit cannot
/// place exactly.
pub fn set_roster_row(
    text: &str,
    name: &str,
    profile: &str,
) -> Result<RosterEdit, RosterEditRefusal> {
    if name != ROW {
        return Err(RosterEditRefusal::Row(name.to_owned()));
    }
    if !is_config_key(profile) {
        return Err(RosterEditRefusal::ProfileGrammar);
    }
    let placed = find_row(text)?;
    let before = parse_identity_in(Path::new(SOURCE), text)
        .map_err(|why| RosterEditRefusal::Config(why.to_string()))?;
    if before.profile(profile).is_none() {
        return Err(RosterEditRefusal::ProfileUnknown(profile.to_owned()));
    }
    if placed.current == profile {
        return Ok(RosterEdit {
            text: text.to_owned(),
            changed: false,
        });
    }
    let mut edited = String::with_capacity(text.len() + profile.len());
    edited.push_str(&text[..placed.span.start]);
    edited.push_str(profile);
    edited.push_str(&text[placed.span.end..]);
    if !only_the_binding_moved(&before, &edited, profile) {
        return Err(RosterEditRefusal::Shape {
            line: placed.line,
            why: "the rewrite would change more than the one value",
        });
    }
    Ok(RosterEdit {
        text: edited,
        changed: true,
    })
}

/// Whether `edited` parses to `before` with the orchestrator binding set to
/// `profile` and nothing else different.
fn only_the_binding_moved(before: &IdentityConfig, edited: &str, profile: &str) -> bool {
    let Ok(after) = parse_identity_in(Path::new(SOURCE), edited) else {
        return false;
    };
    let mut expected = before.clone();
    upsert(&mut expected.roster, ROW, profile.to_owned());
    after == expected
}

/// The one `[roster] orchestrator` claim of `text`, placed.
fn find_row(text: &str) -> Result<Placed, RosterEditRefusal> {
    let mut section = String::new();
    let mut found: Option<(usize, &str)> = None;
    for item in config_lines(Path::new(SOURCE), text) {
        let item = item.map_err(|why| RosterEditRefusal::Config(why.to_string()))?;
        let ConfigLine::Plain(line, raw) = item else {
            continue;
        };
        let trimmed = raw.trim();
        if let Some(name) = section_header(trimmed) {
            section = name;
            continue;
        }
        if section != ROSTER || key_claim(trimmed) != Some(ROW) {
            continue;
        }
        if found.replace((line, raw)).is_some() {
            return Err(RosterEditRefusal::Duplicate);
        }
    }
    let (line, raw) = found.ok_or(RosterEditRefusal::Absent)?;
    place(text, line, raw)
}

/// Locate the value inside the row `raw`, which is line `line` of `text`.
///
/// The value grammar is the reader's `entry_value`: a value whose last
/// character closes the quote that opens it keeps its quotes and binds what is
/// between them; any other value ends at its first `#` and loses its trailing
/// blanks. A value that opens a quote it does not close at the end of the line
/// is what the reader binds WITH the quote in it, so it is not placeable.
fn place(text: &str, line: usize, raw: &str) -> Result<Placed, RosterEditRefusal> {
    let shape = |why| RosterEditRefusal::Shape { line, why };
    let start: usize = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum();
    if text.get(start..start + raw.len()) != Some(raw) {
        return Err(shape("the row cannot be located exactly"));
    }
    let eq = raw
        .find('=')
        .ok_or_else(|| shape("the row has no equals sign"))?;
    let after = &raw[eq + 1..];
    let value_at = eq + 1 + (after.len() - after.trim_start().len());
    let body = raw[value_at..].trim_end();
    if let Some(inner) = body
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        let begin = start + value_at + 1;
        return Ok(Placed {
            line,
            span: begin..begin + inner.len(),
            current: inner.to_owned(),
        });
    }
    if body.starts_with('"') {
        return Err(shape(
            "a quoted value with a trailing comment cannot be placed exactly",
        ));
    }
    let token = body.find('#').map_or(body, |hash| &body[..hash]).trim_end();
    if token.is_empty() {
        return Err(shape("the row has no value"));
    }
    let begin = start + value_at;
    Ok(Placed {
        line,
        span: begin..begin + token.len(),
        current: token.to_owned(),
    })
}

/// The config bytes a display read, which an apply must still find on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSeen {
    text: String,
}

/// Why a read or an apply refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// The path is a symlink: edit the file by hand.
    Symlink(PathBuf),
    /// The path is not a regular file.
    NotRegular(PathBuf),
    /// The file cannot be read as bounded UTF-8 config text.
    Unreadable(PathBuf),
    /// The file changed since it was displayed.
    ChangedSince(PathBuf),
    /// A backup of this second already exists: retry.
    BackupExists(PathBuf),
    /// The text refuses the edit.
    Edit(RosterEditRefusal),
    /// A write step failed; the original is intact and `backup` remains.
    Publish {
        /// The failed step.
        why: String,
        /// The backup this apply made before it failed.
        backup: Option<PathBuf>,
    },
}

impl fmt::Display for ApplyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symlink(path) => write!(
                f,
                "{} is a symlink; ae never writes through it, edit the file by hand",
                path.display()
            ),
            Self::NotRegular(path) => write!(f, "{} is not a regular file", path.display()),
            Self::Unreadable(path) => {
                write!(f, "{} cannot be read as config text", path.display())
            }
            Self::ChangedSince(path) => write!(
                f,
                "{} changed since it was shown; nothing was written",
                path.display()
            ),
            Self::BackupExists(path) => write!(
                f,
                "backup {} already exists (same second); retry",
                path.display()
            ),
            Self::Edit(why) => fmt::Display::fmt(why, f),
            Self::Publish { why, backup } => match backup {
                Some(backup) => write!(
                    f,
                    "{why}; the original is intact, backup kept at {}",
                    backup.display()
                ),
                None => write!(f, "{why}; the original is intact"),
            },
        }
    }
}

impl std::error::Error for ApplyRefusal {}

/// What an apply did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Whether the file changed.
    pub changed: bool,
    /// The backup of the previous bytes, when the file changed.
    pub backup: Option<PathBuf>,
}

/// The metadata of the regular file at `path`, never following a link.
fn regular(path: &Path) -> Result<std::fs::Metadata, ApplyRefusal> {
    match crate::store::node_meta(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(ApplyRefusal::Symlink(path.to_owned())),
        Ok(meta) if meta.file_type().is_file() => Ok(meta),
        Ok(_) => Err(ApplyRefusal::NotRegular(path.to_owned())),
        Err(_) => Err(ApplyRefusal::Unreadable(path.to_owned())),
    }
}

/// Read the config for display, refusing what an apply could never write.
///
/// # Errors
///
/// An [`ApplyRefusal`] for a symlink, a non-regular file, or text that is not
/// bounded UTF-8.
pub fn read_config_seen(path: &Path) -> Result<ConfigSeen, ApplyRefusal> {
    regular(path)?;
    let text = read_selected(path).map_err(|_| ApplyRefusal::Unreadable(path.to_owned()))?;
    Ok(ConfigSeen { text })
}

/// Bind `[roster] <name>` to `profile` in the file at `path`, if the file is
/// still what `seen` displayed.
///
/// The order is the policy: the path is a regular file and no link; its bytes
/// equal `seen`'s; the rewrite places exactly; a same-profile pick stops here,
/// writing nothing; otherwise one `<file>.<now_epoch>.bak` of the old bytes
/// (never overwriting a backup of the same second), then a temp file that
/// takes the original mode, whose owner must be the original's, whose source
/// bytes are compared once more, and which replaces the file by rename.
///
/// # Errors
///
/// An [`ApplyRefusal`]; the original file is then untouched.
pub fn apply_roster_row(
    path: &Path,
    seen: &ConfigSeen,
    name: &str,
    profile: &str,
    now_epoch: i64,
) -> Result<Applied, ApplyRefusal> {
    apply_with(path, seen, name, profile, now_epoch, |from, to| {
        std::fs::rename(from, to)
    })
}

/// [`apply_roster_row`] with its last step handed in, so a test can fail the
/// rename itself.
fn apply_with(
    path: &Path,
    seen: &ConfigSeen,
    name: &str,
    profile: &str,
    now_epoch: i64,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<Applied, ApplyRefusal> {
    regular(path)?;
    let current = read_selected(path).map_err(|_| ApplyRefusal::Unreadable(path.to_owned()))?;
    if current != seen.text {
        return Err(ApplyRefusal::ChangedSince(path.to_owned()));
    }
    let edit = set_roster_row(&current, name, profile).map_err(ApplyRefusal::Edit)?;
    if !edit.changed {
        return Ok(Applied {
            changed: false,
            backup: None,
        });
    }
    let backup =
        backup_path(path, now_epoch).ok_or_else(|| ApplyRefusal::Unreadable(path.to_owned()))?;
    publish_backup(&backup, current.as_bytes()).map_err(|why| {
        if why.kind() == io::ErrorKind::AlreadyExists {
            ApplyRefusal::BackupExists(backup.clone())
        } else {
            ApplyRefusal::Publish {
                why: format!("could not write the backup: {why}"),
                backup: None,
            }
        }
    })?;
    let mut refused = None;
    let published = crate::store::publish_private_with(path, edit.text.as_bytes(), |temp, to| {
        replace_step(temp, to, &current, &backup, rename).map_err(|why| {
            let text = why.to_string();
            refused = Some(why);
            io::Error::other(text)
        })
    });
    match published {
        Ok(()) => Ok(Applied {
            changed: true,
            backup: Some(backup),
        }),
        Err(why) => Err(refused.unwrap_or(ApplyRefusal::Publish {
            why: format!("could not replace the config: {why}"),
            backup: Some(backup),
        })),
    }
}

/// `<file>.<epoch>.bak` beside the file: the family `ae init --force` writes.
fn backup_path(path: &Path, epoch: i64) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(format!(".{epoch}.bak"));
    Some(path.with_file_name(name))
}

/// Publish `bytes` at `backup` exclusively: a name that exists refuses and is
/// never overwritten.
fn publish_backup(backup: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::store::publish_private_with(backup, bytes, |temp, to| {
        let linked = std::fs::hard_link(temp, to);
        let _ = std::fs::remove_file(temp);
        linked
    })
}

/// The last step of a replace, run with the staged temp: the file at `to` is
/// still a regular file and no link, the temp is owned by the file's owner and
/// takes the file's mode, the bytes on disk are still `seen`, and then the
/// rename.
fn replace_step(
    temp: &Path,
    to: &Path,
    seen: &str,
    backup: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), ApplyRefusal> {
    let publish = |why: String| ApplyRefusal::Publish {
        why,
        backup: Some(backup.to_owned()),
    };
    let original = regular(to).map_err(|why| publish(why.to_string()))?;
    let staged = crate::store::node_meta(temp)
        .map_err(|why| publish(format!("could not inspect the staged copy: {why}")))?;
    if staged.uid() != original.uid() {
        return Err(publish(format!(
            "{} is not owned by the current user",
            to.display()
        )));
    }
    std::fs::set_permissions(
        temp,
        std::fs::Permissions::from_mode(original.mode() & 0o7777),
    )
    .map_err(|why| publish(format!("could not keep the file mode: {why}")))?;
    let now =
        read_selected(to).map_err(|_| publish("the config could not be read again".to_owned()))?;
    if now != seen {
        return Err(publish(
            "the config changed while it was being replaced".to_owned(),
        ));
    }
    rename(temp, to).map_err(|why| publish(format!("could not rename the staged copy: {why}")))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "test code reads and inspects its own scratch files"
)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ae-roster-edit-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a scratch directory");
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    const CONFIG: &str = "[profiles]\nold = sleep 600\nnew = sleep 601\n[roster]\norchestrator = old\n[workspace]\nmain = orchestrator\n";

    #[test]
    fn a_failed_rename_leaves_the_original_whole_removes_the_temp_and_names_the_backup() {
        let dir = scratch("rename");
        let path = dir.join("config");
        assert!(std::fs::write(&path, CONFIG).is_ok());
        assert!(
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).is_ok(),
            "the original mode"
        );
        let before = std::fs::metadata(&path).expect("original metadata");
        let seen = read_config_seen(&path).expect("display read");
        let refusal = apply_with(&path, &seen, "orchestrator", "new", 7, |_, _| {
            Err(io::Error::other("injected rename failure"))
        })
        .expect_err("the rename is failed");
        let backup = dir.join("config.7.bak");
        assert_eq!(
            refusal,
            ApplyRefusal::Publish {
                why: "could not rename the staged copy: injected rename failure".to_owned(),
                backup: Some(backup.clone()),
            }
        );
        assert!(refusal.to_string().contains("config.7.bak"), "{refusal}");
        let after = std::fs::metadata(&path).expect("original metadata after");
        assert_eq!(std::fs::read_to_string(&path).expect("bytes"), CONFIG);
        assert_eq!(after.mode() & 0o7777, 0o640);
        assert_eq!((after.dev(), after.ino()), (before.dev(), before.ino()));
        assert_eq!(std::fs::read_to_string(&backup).expect("backup"), CONFIG);
        assert_eq!(names(&dir), ["config", "config.7.bak"], "no temp stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_backup_name_refuses_before_the_original_is_touched() {
        let dir = scratch("backup");
        let path = dir.join("config");
        assert!(std::fs::write(&path, CONFIG).is_ok());
        assert!(std::fs::write(dir.join("config.9.bak"), "someone else").is_ok());
        let seen = read_config_seen(&path).expect("display read");
        let refusal = apply_roster_row(&path, &seen, "orchestrator", "new", 9)
            .expect_err("the name is taken");
        assert_eq!(
            refusal,
            ApplyRefusal::BackupExists(dir.join("config.9.bak"))
        );
        assert!(refusal.to_string().contains("retry"), "{refusal}");
        assert_eq!(std::fs::read_to_string(&path).expect("bytes"), CONFIG);
        assert_eq!(
            std::fs::read_to_string(dir.join("config.9.bak")).expect("backup"),
            "someone else"
        );
        assert_eq!(names(&dir), ["config", "config.9.bak"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_backup_name_is_the_init_family() {
        assert_eq!(
            backup_path(Path::new("/home/h/.ae/config"), 42),
            Some(PathBuf::from("/home/h/.ae/config.42.bak"))
        );
        assert_eq!(backup_path(Path::new("/"), 42), None);
    }

    #[test]
    fn the_refusals_say_what_to_do() {
        let link = ApplyRefusal::Symlink(PathBuf::from("/h/config"));
        assert!(link.to_string().contains("edit the file by hand"), "{link}");
        let changed = ApplyRefusal::ChangedSince(PathBuf::from("/h/config"));
        assert!(
            changed.to_string().contains("nothing was written"),
            "{changed}"
        );
        let edit = ApplyRefusal::Edit(RosterEditRefusal::Absent);
        assert!(edit.to_string().contains("[roster] orchestrator"), "{edit}");
    }

    #[test]
    fn a_quoted_value_that_is_empty_is_replaced_between_its_quotes() {
        let text = CONFIG.replace("orchestrator = old", "orchestrator = \"\"");
        let edit = set_roster_row(&text, "orchestrator", "new").expect("empty quoted value");
        assert_eq!(
            edit.text,
            CONFIG.replace("orchestrator = old", "orchestrator = \"new\"")
        );
    }

    #[test]
    fn a_lone_quote_is_a_literal_the_reader_would_bind_and_so_is_refused() {
        let text = CONFIG.replace("orchestrator = old", "orchestrator = \"");
        assert!(matches!(
            set_roster_row(&text, "orchestrator", "new"),
            Err(RosterEditRefusal::Shape { line: 5, .. })
        ));
    }
}
