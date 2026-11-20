//! `OpenCode`'s saved model variant against a profile's explicit one (#310).
//!
//! `OpenCode`'s full TUI prefers the per-model preference in its `model.json`
//! (`variant["<provider>/<model>"]`, including the literal `default`) over the
//! variant a profile's `provider/model#variant` asks for. This module reads
//! that file, read only, and says when it differs: ae never writes, moves,
//! locks or scopes `model.json`, and never blocks a launch on what it finds.
//!
//! The state directory is the one the launch would give the tool: the profile
//! command's own `env` prefix is peeled exactly as `_run` peels it
//! ([`crate::run::peel_env`]), so `env -i`, `env -u` and shell-prefix
//! assignments decide `XDG_STATE_HOME` and `HOME` here as they do there. Only
//! a profile command whose model word asks for a variant is judged: any other
//! command reads neither the environment nor the file.

use std::path::{Path, PathBuf};

use crate::store::{Oversized, SourceRead};

/// The largest `model.json` ae reads, in bytes. A real file is a few KiB.
pub const MODEL_JSON_CAP: u64 = 256 * 1024;

/// What a profile's model word asks for: the preference key and the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `provider/model`, the key `model.json` stores a preference under.
    pub key: String,
    /// The variant after the `#`.
    pub variant: String,
}

/// The split of one model word as `OpenCode`'s config reads it: a provider
/// before the first `/`, a model, and one non-empty variant after the one
/// `#`. `None` when the word is not `provider/model#variant` in that grammar.
#[must_use]
pub fn request_of(model_word: &str) -> Option<Request> {
    let (provider, rest) = model_word.split_once('/')?;
    let (model, variant) = rest.split_once('#')?;
    if provider.is_empty() || provider.contains('#') {
        return None;
    }
    if model.is_empty() || variant.is_empty() || variant.contains('#') {
        return None;
    }
    Some(Request {
        key: format!("{provider}/{model}"),
        variant: variant.to_owned(),
    })
}

/// What `model.json` holds for one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// No usable value is saved for the key.
    Unset,
    /// The saved variant, the literal `default` included.
    Variant(String),
}

/// The saved preference for `key` in a `model.json` body, or `None` when the
/// bytes are not a document of the expected shape.
///
/// `OpenCode` drops an empty or non-string value, and a repeated key keeps its
/// last occurrence as `JSON.parse` does, so both match here.
#[must_use]
pub fn saved_for(bytes: &[u8], key: &str) -> Option<Saved> {
    let text = std::str::from_utf8(bytes).ok()?;
    let crate::json::Value::Obj(fields) = crate::json::parse(text).ok()? else {
        return None;
    };
    let Some((_, variants)) = fields.iter().rev().find(|(name, _)| name == "variant") else {
        return Some(Saved::Unset);
    };
    let crate::json::Value::Obj(entries) = variants else {
        return None;
    };
    match entries.iter().rev().find(|(name, _)| name == key) {
        Some((_, crate::json::Value::Str(saved))) if !saved.is_empty() => {
            Some(Saved::Variant(saved.clone()))
        }
        _ => Some(Saved::Unset),
    }
}

/// Why a check could not say whether a saved preference shadows the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The model word is not `provider/model#variant`.
    ModelWord,
    /// The state directory does not resolve to an absolute path.
    StateDir,
    /// `model.json` is a symlink or another node that is not a regular file.
    NotRegular,
    /// `model.json` exists and could not be read.
    Unreadable,
    /// `model.json` is longer than [`MODEL_JSON_CAP`].
    Oversized,
    /// The bytes are not a document of the expected shape.
    Shape,
}

impl Reason {
    /// The closed wording a doctor row gives after `unknown:`.
    const fn text(self) -> &'static str {
        match self {
            Self::ModelWord => "the model word is not provider/model#variant",
            Self::StateDir => {
                "the state directory does not resolve to an absolute path from this command's environment"
            }
            Self::NotRegular => "model.json is a symlink or not a regular file",
            Self::Unreadable => "model.json could not be read",
            Self::Oversized => "model.json is over 256 KiB",
            Self::Shape => "model.json is not JSON of the expected shape",
        }
    }
}

/// The verdict for one profile command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The command carries no explicit variant: nothing was read.
    NoRequest,
    /// The check could not decide.
    Unknown(Reason),
    /// Nothing saved overrides the request.
    Clear {
        /// The preference key.
        key: String,
        /// The variant the profile asks for.
        configured: String,
        /// Whether a saved preference exists and equals it.
        saved_same: bool,
    },
    /// A saved preference differs from the request.
    Shadowed {
        /// The preference key.
        key: String,
        /// The variant the profile asks for.
        configured: String,
        /// The saved variant, the literal `default` included.
        saved: String,
        /// The `model.json` that holds it.
        path: PathBuf,
    },
}

/// One variable as the launch gives it to the tool: the last assignment of the
/// peeled prefix, else nothing when `env -i` or `env -u` removed it, else the
/// inherited value.
fn effective(
    prefix: &crate::run::EnvPrefix,
    name: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some((_, value)) = prefix.assign.iter().rev().find(|(set, _)| set == name) {
        return Some(value.clone());
    }
    if prefix.clear || prefix.unset.iter().any(|unset| unset == name) {
        return None;
    }
    lookup(name)
}

/// Where `OpenCode` keeps `model.json` under the launch's environment:
/// `$XDG_STATE_HOME/opencode/model.json` (an empty value counts as unset),
/// else `$HOME/.local/state/opencode/model.json`. `None` when no absolute
/// state directory follows from the command.
fn model_json_path(command: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let words = crate::words::split_words(command, &|name: &str| lookup(name)).ok()?;
    let (prefix, _) = crate::run::peel_env(words).ok()?;
    let declared = effective(&prefix, "XDG_STATE_HOME", lookup).filter(|dir| !dir.is_empty());
    let state = if let Some(dir) = declared {
        PathBuf::from(dir)
    } else {
        let home = effective(&prefix, "HOME", lookup).filter(|dir| !dir.is_empty())?;
        Path::new(&home).join(".local").join("state")
    };
    state
        .is_absolute()
        .then(|| state.join("opencode").join("model.json"))
}

/// The check for a profile command, over an injected environment `lookup`
/// and an injected capped reader of `model.json`.
///
/// A command without an explicit variant is [`Check::NoRequest`] before any
/// lookup or read.
#[must_use]
pub fn check_with(
    command: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    read: &dyn Fn(&Path) -> Result<SourceRead, Oversized>,
) -> Check {
    let Some(word) = crate::launch_cmd::requested_model(command) else {
        return Check::NoRequest;
    };
    let Some(request) = request_of(&word) else {
        return Check::Unknown(Reason::ModelWord);
    };
    let Some(path) = model_json_path(command, lookup) else {
        return Check::Unknown(Reason::StateDir);
    };
    let Request { key, variant } = request;
    let bytes = match read(&path) {
        Ok(SourceRead::Absent) => {
            return Check::Clear {
                key,
                configured: variant,
                saved_same: false,
            };
        }
        Ok(SourceRead::Ready(bytes)) => bytes,
        Ok(SourceRead::Invalid(_)) => return Check::Unknown(Reason::NotRegular),
        Ok(SourceRead::Unreadable(_)) => return Check::Unknown(Reason::Unreadable),
        Err(Oversized) => return Check::Unknown(Reason::Oversized),
    };
    match saved_for(&bytes, &key) {
        None => Check::Unknown(Reason::Shape),
        Some(Saved::Unset) => Check::Clear {
            key,
            configured: variant,
            saved_same: false,
        },
        Some(Saved::Variant(saved)) if saved == variant => Check::Clear {
            key,
            configured: variant,
            saved_same: true,
        },
        Some(Saved::Variant(saved)) => Check::Shadowed {
            key,
            configured: variant,
            saved,
            path,
        },
    }
}

/// The check over the real environment and the capped store reader.
#[must_use]
pub fn check(command: &str) -> Check {
    check_with(command, &crate::run::env_lookup, &|path| {
        crate::store::read_capped(path, MODEL_JSON_CAP)
    })
}

/// The one launch line for a shadowed request, `None` for every other verdict.
/// Every value is quoted with Rust's `Debug`, so a hostile saved string stays
/// one line of inert text.
#[must_use]
pub fn notice(slot: &str, check: &Check) -> Option<String> {
    let Check::Shadowed {
        key,
        configured,
        saved,
        path,
    } = check
    else {
        return None;
    };
    let path = path.display().to_string();
    Some(format!(
        "ae: seat {slot}: {key:?} asks variant {configured:?} but OpenCode's saved preference is {saved:?} (in {path:?}); OpenCode will use the saved one. Select the variant once in /variants or clear that key (#310)"
    ))
}

/// What one `ae doctor` row says about a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowText {
    /// Whether the row is a warning rather than an OK.
    pub warn: bool,
    /// The row's detail, one line.
    pub detail: String,
}

/// The doctor row for a check, `None` for a command that asks no variant.
#[must_use]
pub fn row_text(check: &Check) -> Option<RowText> {
    let (warn, detail) = match check {
        Check::NoRequest => return None,
        Check::Unknown(reason) => (false, format!("unknown: {}", reason.text())),
        Check::Clear {
            key,
            configured,
            saved_same: false,
        } => (
            false,
            format!("{key:?} asks variant {configured:?}; no saved preference overrides it"),
        ),
        Check::Clear {
            key,
            configured,
            saved_same: true,
        } => (
            false,
            format!("{key:?} asks variant {configured:?}; the saved preference is the same"),
        ),
        Check::Shadowed {
            key,
            configured,
            saved,
            path,
        } => {
            let path = path.display().to_string();
            (
                true,
                format!(
                    "{key:?} asks variant {configured:?} but the saved preference is {saved:?} (in {path:?}); a fresh OpenCode start will use the saved one. Select the variant once in /variants or clear that key (#310)"
                ),
            )
        }
    };
    Some(RowText { warn, detail })
}
