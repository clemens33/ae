//! The durable claim of one draft id: `<root>/web/claims/<sha256 of the
//! nonce>`, created before any input reaches a pane and settled with the
//! outcome after. A claim that exists, readable or not, never carries again:
//! a retry is answered from it, so one draft id is at most one ask.
//!
//! The record is a few `key value` lines under one header, at most [`CAP`]
//! bytes. [`decode`] fails closed: a missing, unknown or repeated key, a value
//! out of shape, a partial outcome, a second header or a truncated last line
//! is no claim, and the server answers such a file as "unknown".

use std::collections::BTreeSet;
use std::io;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::auth;
use crate::store::{self, SourcePresence, SourceRead};

/// The most bytes a claim record takes, inclusive.
pub const CAP: usize = 4096;
/// The seconds a claim is kept after its creation: seven days, far past the
/// one day its nonce is acceptable, so a pruned retry is expired, never fresh.
pub const KEEP: u64 = 7 * 86_400;
/// The first line of every record.
const HEADER: &str = "ae-web-claim 1";
/// The keys a record may carry, each at most once.
const KEYS: [&str; 8] = [
    "payload", "route", "id", "created", "outcome", "seat", "to", "words",
];

/// One claim: the draft's payload hash and route, the request id the server
/// minted for it, the listener second it was created and, once settled, what
/// became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// SHA-256 hex of the draft's text.
    pub payload: String,
    /// The route id the draft was sent under.
    pub route: String,
    /// The request id of the ask.
    pub id: String,
    /// The listener second the claim was created.
    pub created: u64,
    /// What became of it, once settled.
    pub outcome: Option<Outcome>,
}

/// What became of a claimed draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Its kind.
    pub word: Word,
    /// The seat it was for.
    pub seat: String,
    /// The session it was for.
    pub to: String,
    /// One line of words for the page; empty for none.
    pub words: String,
}

/// The kinds of outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word {
    /// The ask was delivered and recorded.
    Sent,
    /// Nothing was delivered; `words` says why.
    Refused,
    /// The route changed before anything was delivered.
    Changed,
    /// It may have landed; it is never sent again.
    Unknown,
}

impl Word {
    /// The word a record and the wire carry.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Refused => "refused",
            Self::Changed => "changed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::Sent, Self::Refused, Self::Changed, Self::Unknown]
            .into_iter()
            .find(|word| word.word() == text)
    }
}

/// Why bytes are no claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub &'static str);

/// The claim `bytes` record.
///
/// # Errors
///
/// [`Refused`] for anything but one whole, well-formed record of at most
/// [`CAP`] bytes.
pub fn decode(bytes: &[u8]) -> Result<Claim, Refused> {
    if bytes.len() > CAP {
        return Err(Refused("over the size cap"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Refused("not UTF-8"))?;
    let body = text
        .strip_suffix('\n')
        .ok_or(Refused("the last line is not whole"))?;
    let mut lines = body.split('\n');
    if lines.next() != Some(HEADER) {
        return Err(Refused("no claim header"));
    }
    let mut seen = BTreeSet::new();
    let mut fields: [Option<&str>; KEYS.len()] = [None; KEYS.len()];
    for line in lines {
        let (key, value) = line.split_once(' ').ok_or(Refused("a line is no field"))?;
        let at = KEYS
            .iter()
            .position(|known| *known == key)
            .ok_or(Refused("an unknown field"))?;
        if !seen.insert(at) {
            return Err(Refused("a repeated field"));
        }
        if let Some(slot) = fields.get_mut(at) {
            *slot = Some(value);
        }
    }
    let [payload, route, id, created, outcome, seat, to, words] = fields;
    let hash = |value: Option<&str>| {
        value
            .filter(|value| auth::is_hex(value, 64))
            .map(str::to_owned)
            .ok_or(Refused("a hash out of shape"))
    };
    let id = id
        .filter(|id| crate::tracked::is_request_id(id))
        .ok_or(Refused("no request id"))?;
    let created = created
        .filter(|digits| {
            (1..=20).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
        })
        .and_then(|digits| digits.parse::<u64>().ok())
        .ok_or(Refused("no creation second"))?;
    let outcome = match (outcome, seat, to) {
        (None, None, None) if words.is_none() => None,
        (Some(word), Some(seat), Some(to)) => {
            let word = Word::parse(word).ok_or(Refused("an unknown outcome"))?;
            // A refusal is whole only with the words it was refused with.
            let words = match words {
                Some(words) => one_line(words).ok_or(Refused("words out of shape"))?,
                None if word == Word::Refused => return Err(Refused("a refusal without words")),
                None => String::new(),
            };
            Some(Outcome {
                word,
                seat: one_line(seat).ok_or(Refused("a seat out of shape"))?,
                to: one_line(to).ok_or(Refused("a session out of shape"))?,
                words,
            })
        }
        _ => return Err(Refused("a partial outcome")),
    };
    Ok(Claim {
        payload: hash(payload)?,
        route: hash(route)?,
        id: id.to_owned(),
        created,
        outcome,
    })
}

/// A non-empty value with no control character, owned.
fn one_line(value: &str) -> Option<String> {
    (!value.is_empty() && !value.chars().any(char::is_control)).then(|| value.to_owned())
}

/// The record of `claim`: what [`decode`] reads back, given values in shape.
#[must_use]
pub fn encode(claim: &Claim) -> String {
    use std::fmt::Write as _;

    let mut out = format!(
        "{HEADER}\npayload {}\nroute {}\nid {}\ncreated {}\n",
        claim.payload, claim.route, claim.id, claim.created
    );
    if let Some(outcome) = &claim.outcome {
        let _ = write!(
            out,
            "outcome {}\nseat {}\nto {}\n",
            outcome.word.word(),
            outcome.seat,
            outcome.to
        );
        if !outcome.words.is_empty() {
            let _ = writeln!(out, "words {}", outcome.words);
        }
    }
    out
}

/// What the claim store holds under one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// Nothing: the draft id was never claimed, or its claim was pruned.
    Absent,
    /// A claim that decodes.
    Known(Claim),
    /// Something that is no claim: never carried, answered as unknown.
    Unreadable,
}

/// The claim store of one state root. Both directories under the root are
/// proven real directories before anything is read, written or listed under
/// them, so a linked `web` or `claims` is never followed.
#[derive(Debug, Clone)]
pub struct Claims {
    root: PathBuf,
    web: PathBuf,
    dir: PathBuf,
    /// Whether the entries of `web` in the root and of `claims` in `web` were
    /// made durable by this store: until then every create syncs both.
    parents: Arc<AtomicBool>,
}

impl Claims {
    /// The store under state root `root`; nothing is created until a claim is.
    #[must_use]
    pub fn open(root: &Path) -> Self {
        let web = root.join("web");
        Self {
            root: root.to_owned(),
            dir: web.join("claims"),
            web,
            parents: Arc::new(AtomicBool::new(false)),
        }
    }

    /// What is held under `name`, read without following a link and capped.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Lookup {
        if !auth::is_hex(name, 64) {
            return Lookup::Unreadable;
        }
        match real(&self.web) {
            Ok(true) => {}
            Ok(false) => return Lookup::Absent,
            Err(_) => return Lookup::Unreadable,
        }
        let cap = u64::try_from(CAP).unwrap_or(u64::MAX);
        match store::read_capped_in(&self.dir, name, cap) {
            Ok(SourceRead::Absent) => Lookup::Absent,
            Ok(SourceRead::Ready(bytes)) => {
                decode(&bytes).map_or(Lookup::Unreadable, Lookup::Known)
            }
            _ => Lookup::Unreadable,
        }
    }

    /// Create the claim `name`, mode 0600, only if nothing holds the name, and
    /// make it durable. The directories are made and proven first and their
    /// own entries synced before the claim exists, so a failure there leaves
    /// no claim behind; the claim is published whole, then its directory is
    /// synced.
    ///
    /// # Errors
    ///
    /// A directory that is no real directory, a taken name, or the write or
    /// any `fsync`.
    pub fn create(&self, name: &str, claim: &Claim) -> io::Result<()> {
        plain(name)?;
        let made_web = make_dir(&self.web)?;
        store::prove_real_directory(&self.web)?;
        let made_claims = make_dir(&self.dir)?;
        store::prove_real_directory(&self.dir)?;
        if made_web || made_claims || !self.parents.load(Ordering::SeqCst) {
            sync_dir(&self.root)?;
            sync_dir(&self.web)?;
            self.parents.store(true, Ordering::SeqCst);
        }
        crate::init::create_exclusive(&self.dir.join(name), encode(claim).as_bytes(), 0o600)?;
        sync_dir(&self.dir)
    }

    /// Replace the claim `name` with `claim`, whole or not at all: a temp
    /// beside it, `fsync`, rename, then the directory's `fsync`.
    ///
    /// # Errors
    ///
    /// A `web` that is no real directory, the publish or the `fsync`.
    pub fn settle(&self, name: &str, claim: &Claim) -> io::Result<()> {
        plain(name)?;
        store::prove_real_directory(&self.web)?;
        store::publish_private_in(&self.dir, name, encode(claim).as_bytes())?;
        sync_dir(&self.dir)
    }

    /// Remove every claim created more than [`KEEP`] seconds before listener
    /// second `now`. A file that is no claim is never removed.
    pub fn prune(&self, now: u64) {
        if !matches!(real(&self.web), Ok(true)) || store::prove_real_directory(&self.dir).is_err() {
            return;
        }
        let mut removed = false;
        for name in names(&self.dir) {
            if let Lookup::Known(claim) = self.lookup(&name)
                && claim.created.saturating_add(KEEP) < now
            {
                removed |= std::fs::remove_file(self.dir.join(&name)).is_ok();
            }
        }
        if removed {
            drop(sync_dir(&self.dir));
        }
    }
}

/// Whether `dir` is a real directory: `Ok(false)` when nothing is there, an
/// error for a link or any other node.
fn real(dir: &Path) -> io::Result<bool> {
    match store::source_presence(dir) {
        SourcePresence::Absent => Ok(false),
        _ => store::prove_real_directory(dir).map(|()| true),
    }
}

/// Refuse a name that is not one claim name.
fn plain(name: &str) -> io::Result<()> {
    if auth::is_hex(name, 64) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a claim name",
        ))
    }
}

/// Create `dir` at mode 0700: whether this call created it.
fn make_dir(dir: &Path) -> io::Result<bool> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(true),
        Err(why) if why.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(why) => Err(why),
    }
}

/// Make the entries of `dir` durable: open the directory and `fsync` it.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens a web claim directory or its parent to fsync it for a durable claim — see clippy.toml"
    )]
    let handle = std::fs::File::open(dir)?;
    handle.sync_all()
}

/// The claim names in the claim directory; none when it cannot be listed.
fn names(dir: &Path) -> Vec<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: lists the web claim directory for the seven-day prune — see clippy.toml"
    )]
    let listing = std::fs::read_dir(dir);
    listing
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| auth::is_hex(name, 64))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{Claim, Claims, Outcome, Refused, SourcePresence, Word, decode, encode, store};

    #[test]
    fn a_settled_claim_without_words_and_one_with_words_round_trip() {
        let mut claim = Claim {
            payload: "a".repeat(64),
            route: "b".repeat(64),
            id: "ae-20261122T200000Z-0000000a".to_owned(),
            created: 0,
            outcome: None,
        };
        assert_eq!(decode(encode(&claim).as_bytes()), Ok(claim.clone()));
        for (word, words) in [(Word::Sent, ""), (Word::Refused, "cap-full: 5/5")] {
            claim.outcome = Some(Outcome {
                word,
                seat: "tui".to_owned(),
                to: "sample".to_owned(),
                words: words.to_owned(),
            });
            assert_eq!(decode(encode(&claim).as_bytes()), Ok(claim.clone()));
        }
        let text = encode(&claim);
        // A refusal whose words were cut off is not a complete record.
        let cut = text.replace("words cap-full: 5/5\n", "");
        assert!(decode(cut.as_bytes()).is_err());
        assert!(decode(text.trim_end().as_bytes()).is_err());
        assert!(decode(format!("{text}words again\n").as_bytes()).is_err());
        assert!(decode(text.replace("seat tui\n", "").as_bytes()).is_err());
    }

    #[test]
    fn a_creation_second_is_decimal_digits_only() {
        let claim = Claim {
            payload: "a".repeat(64),
            route: "b".repeat(64),
            id: "ae-20261122T200000Z-0000000a".to_owned(),
            created: 5,
            outcome: None,
        };
        let text = encode(&claim);
        assert_eq!(decode(text.as_bytes()), Ok(claim));
        // `u64::from_str` takes a leading `+`; the record does not.
        let signed = text.replace("created 5\n", "created +5\n");
        assert_eq!(
            decode(signed.as_bytes()),
            Err(Refused("no creation second"))
        );
    }

    /// A scratch directory this test created itself, so its cleanup removes
    /// nothing it did not make.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir();
        for at in 0..64 {
            let root = base.join(format!("ae-web-{tag}-{}-{at}", std::process::id()));
            if std::fs::create_dir(&root).is_ok() {
                return root;
            }
        }
        panic!("no unused scratch directory under {}", base.display());
    }

    #[test]
    fn a_name_that_is_no_claim_name_is_refused_before_anything_is_made() {
        let root = scratch("claim-name");
        let claims = Claims::open(&root);
        let claim = Claim {
            payload: "a".repeat(64),
            route: "b".repeat(64),
            id: "ae-20261122T200000Z-0000000a".to_owned(),
            created: 5,
            outcome: None,
        };
        let created = claims
            .create("../escape", &claim)
            .map_err(|error| error.kind());
        let settled = claims
            .settle("../escape", &claim)
            .map_err(|error| error.kind());
        let web = store::source_presence(&root.join("web"));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(created, Err(io::ErrorKind::InvalidInput));
        assert_eq!(settled, Err(io::ErrorKind::InvalidInput));
        assert!(
            matches!(web, SourcePresence::Absent),
            "nothing is made for a refused name"
        );
    }
}
