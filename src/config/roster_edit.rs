//! The one in-place edit ae makes to the human's global config: the value of
//! the `[roster] orchestrator` row.
//!
//! BATCH 1 STUB: the public seam and its refusals, with no behaviour, so the
//! frozen acceptance fails on behaviour and not on a missing symbol.

use std::fmt;
use std::path::{Path, PathBuf};

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
            Self::Duplicate => f.write_str("the config has more than one [roster] orchestrator row"),
            Self::Shape { line, why } => write!(f, "line {line}: {why}"),
            Self::ProfileGrammar => f.write_str("the profile is not a profile name"),
            Self::ProfileUnknown(profile) => {
                write!(f, "profile '{profile}' is not defined in [profiles]")
            }
        }
    }
}

impl std::error::Error for RosterEditRefusal {}

/// Rewrite the value of the `[roster] <name>` row to `profile`, touching no
/// other byte. Only the orchestrator row is editable.
///
/// # Errors
///
/// A [`RosterEditRefusal`] for a text or a row the edit cannot place exactly.
pub fn set_roster_row(
    _text: &str,
    _name: &str,
    _profile: &str,
) -> Result<RosterEdit, RosterEditRefusal> {
    Err(RosterEditRefusal::Absent)
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
    /// The file's owner is not the current user.
    Owner(PathBuf),
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
            Self::Unreadable(path) => write!(f, "{} cannot be read as config text", path.display()),
            Self::ChangedSince(path) => write!(
                f,
                "{} changed since it was shown; nothing was written",
                path.display()
            ),
            Self::Owner(path) => write!(
                f,
                "{} is not owned by the current user; nothing was written",
                path.display()
            ),
            Self::BackupExists(path) => write!(
                f,
                "backup {} already exists (same second); retry",
                path.display()
            ),
            Self::Edit(why) => why.fmt(f),
            Self::Publish { why, backup } => match backup {
                Some(backup) => write!(f, "{why}; the original is intact, backup kept at {}", backup.display()),
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

/// Read the config for display, refusing what an apply could never write.
///
/// # Errors
///
/// An [`ApplyRefusal`] for a symlink, a non-regular file or unreadable text.
pub fn read_config_seen(path: &Path) -> Result<ConfigSeen, ApplyRefusal> {
    Err(ApplyRefusal::Unreadable(path.to_path_buf()))
}

/// Bind `[roster] <name>` to `profile` in the file at `path`, if the file is
/// still what `seen` displayed.
///
/// # Errors
///
/// An [`ApplyRefusal`]; the original file is then untouched.
pub fn apply_roster_row(
    path: &Path,
    _seen: &ConfigSeen,
    _name: &str,
    _profile: &str,
    _now_epoch: i64,
) -> Result<Applied, ApplyRefusal> {
    Err(ApplyRefusal::Unreadable(path.to_path_buf()))
}
