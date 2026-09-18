//! The durable record a spawn leaves behind when its BRIEF could not be
//! delivered, and the grammar that record is written and read in.
//!
//! A brief that misses its readiness window used to be dumped raw to
//! `undelivered.<name>.txt` and forgotten: the spawner had to notice, and its
//! hand re-send arrived as a PEER message, so the brief marker — the task
//! contract's authority (rule 8b) — was lost. This record is what lets the
//! session's own watchdog deliver that brief LATER, byte-identical to what
//! `spawn` would have pasted, or give it up LOUDLY.
//!
//! THE ONE WRITER is `spawn` on its undelivered path. Nothing else creates a
//! record, and no glob ever qualifies a file: a reader names
//! `brief-retry.<slot>.rec` from a roster slot it already holds, so a legacy
//! `undelivered.*.txt` — which carries no record — stays inert forever, and a
//! file planted at any other name is never read.
//!
//! # What the two bounds mean
//!
//! `attempts` is the number of times ae ENTERED [`crate::deliver::deliver`]
//! for this brief. Readiness, busy and human-typing are seen BEFORE anything is
//! published, and they skip the cycle with `attempts` untouched; only the
//! narrow race where a pane goes busy between the readiness proof and the
//! target lock burns one. `created` carries the wall bound instead: a record
//! older than 30 minutes is given up whatever its attempts say.
//!
//! # Trust
//!
//! A record file is trusted exactly as far as the meta store is: `0600` stops
//! other uids, not this one, so a same-uid shell can rewrite one and the
//! watchdog will paste it with the full authority of `brief(<actor>)`. That is
//! the same boundary every other piece of session state sits behind. What the
//! record DOES buy is the process boundary: the delivery leg takes its text and
//! its actor from here and never from argv, the environment or the caller, so
//! forging the trigger can at most re-fire a brief the spawner already
//! authorized.
//!
//! # The grammar
//!
//! Hostile, hand-editable persisted state, so the parse is bounded before it
//! happens and every deviation is DAMAGE rather than a guess:
//!
//! ```text
//! brief-retry 1
//! slot=spawned.1
//! reference=spawn-spawned.1
//! pane=%105
//! launch_id=claude-1789105855
//! actor=lead
//! attempts=0
//! created=1789105855
//! phase=armed
//! body
//! <the brief, verbatim, to EOF>
//! ```
//!
//! The eight headers are in FIXED order, which is what makes a duplicate, an
//! unknown key and a reordering all one refusal instead of three. A value is
//! everything after the FIRST `=`, so a value may itself contain one. The body
//! is everything after the `body` line, byte for byte — a brief carries
//! newlines, and nothing may normalize them.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The first line of every record, version included. A record that does not
/// begin with exactly this is not one.
const MAGIC: &str = "brief-retry 1";

/// The line that ends the headers and begins the body.
const BODY_MARKER: &str = "body";

/// The eight headers, in the order a record spells them.
const KEYS: [&str; 8] = [
    "slot",
    "reference",
    "pane",
    "launch_id",
    "actor",
    "attempts",
    "created",
    "phase",
];

/// The most a record may be. A brief is a paragraph and a path; anything past
/// this is either damage or a brief that was never retryable, and reading it
/// would let whoever planted it size an allocation on the watchdog's path.
pub const RECORD_CAP: u64 = 65_536;

/// How many times ae may enter a delivery for one brief before it is given up.
pub const MAX_ATTEMPTS: u32 = 2;

/// The longest a slot may be.
const SLOT_CAP: usize = 64;

/// The longest a pane id may be.
const PANE_CAP: usize = 16;

/// The longest a launch token may be.
const LAUNCH_ID_CAP: usize = 128;

/// Where this brief's flight stands, and the whole of the crash-window proof.
///
/// [`Phase::Pasting`] is published DURABLY before the paste, so a record found
/// in that phase is one whose outcome nobody recorded. It is never pasted
/// again — it is given up — which is what makes a crash between the paste and
/// the delete unable to deliver a brief twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No flight is in progress; this record may be taken.
    Armed,
    /// A flight published this before entering delivery and never came back.
    Pasting,
}

impl Phase {
    /// The word a record spells this phase with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Pasting => "pasting",
        }
    }

    /// The phase that word names, or `None`.
    const fn from_word(word: &str) -> Option<Self> {
        match word.as_bytes() {
            b"armed" => Some(Self::Armed),
            b"pasting" => Some(Self::Pasting),
            _ => None,
        }
    }
}

/// Why a record is not one. Every arm is INERT: the record is never acted on,
/// and the reason is what the give-up says out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// Past [`RECORD_CAP`].
    Oversize,
    /// The file is THERE and could not be read as bytes at all — not a regular
    /// file, or the open or the read failed. Distinct from every grammar arm
    /// on purpose: ae did not decide the bytes were wrong, it never saw them.
    Unreadable,
    /// Not UTF-8. A brief is pasted into a terminal; bytes that are not text
    /// were never one.
    NotUtf8,
    /// The first line is not [`MAGIC`].
    Magic,
    /// A header line is missing, out of order, duplicated, unknown, or carries
    /// no `=`.
    Header,
    /// The slot is not a slot.
    Slot,
    /// The reference does not name this record's own slot.
    Reference,
    /// The pane id is not `%<digits>`.
    Pane,
    /// The launch token is empty, oversize, or not printable.
    LaunchId,
    /// The actor is neither an agent name nor the unverified spelling — and it
    /// reaches a provenance first line, so it is an allowlist.
    Actor,
    /// The attempt count is not a canonical decimal within the bound.
    Attempts,
    /// The creation moment is not a canonical, strictly positive epoch.
    Created,
    /// The phase word is neither `armed` nor `pasting`.
    PhaseWord,
    /// The `body` marker line is missing.
    BodyMarker,
    /// The body is absent, blank, or carries a control byte no terminal paste
    /// may.
    Body,
}

impl Damage {
    /// The reason a give-up names, in one clause.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Oversize => "record is larger than the 64 KiB bound",
            Self::Unreadable => "record could not be read at all",
            Self::NotUtf8 => "record is not UTF-8",
            Self::Magic => "record does not begin with its version line",
            Self::Header => "a header line is missing, duplicated, unknown or out of order",
            Self::Slot => "the slot is not a slot",
            Self::Reference => "the reference does not name the record's own slot",
            Self::Pane => "the pane id is not %<digits>",
            Self::LaunchId => "the launch token is empty, oversize or unprintable",
            Self::Actor => "the actor is neither an agent name nor the unverified spelling",
            Self::Attempts => "the attempt count is not a canonical decimal within the bound",
            Self::Created => "the creation moment is not a canonical positive epoch",
            Self::PhaseWord => "the phase is neither armed nor pasting",
            Self::BodyMarker => "the body marker line is missing",
            Self::Body => "the body is absent, blank or carries a control byte",
        }
    }
}

impl std::fmt::Display for Damage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.reason())
    }
}

/// One undelivered brief, as it survives a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The seat's slot. The file is named after it, so the two cannot diverge.
    pub slot: String,
    /// The spawn's request reference, always `spawn-<slot>`.
    pub reference: String,
    /// The pane the seat was launched into.
    pub pane: String,
    /// `launch_id.<slot>` as it stood when the brief was composed: the
    /// incarnation guard.
    pub launch_id: String,
    /// The ORIGINAL spawner, which is the actor the brief marker names — never
    /// the watchdog that carries it.
    pub actor: String,
    /// How many times ae has entered delivery for this brief.
    pub attempts: u32,
    /// When the brief failed, as a strictly positive epoch.
    pub created: i64,
    /// Whether a flight is outstanding.
    pub phase: Phase,
    /// The brief itself, byte for byte as `spawn` would have pasted it.
    pub body: String,
}

/// The record file for `slot` under `dir`.
///
/// Named from a slot the caller already holds — never from a directory
/// listing, so no glob can qualify a file into a brief.
#[must_use]
pub fn path(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!(
        "brief-retry.{}.rec",
        crate::launch::safe_slot(slot)
    ))
}

/// Where a damaged record is moved so it can never be read as one again.
#[must_use]
pub fn damaged_path(dir: &Path, slot: &str) -> PathBuf {
    let mut name = path(dir, slot).into_os_string();
    name.push(".damaged");
    PathBuf::from(name)
}

/// Render `record` exactly as a file holds it.
#[must_use]
pub fn render(record: &Record) -> String {
    let mut text = String::with_capacity(record.body.len() + 256);
    text.push_str(MAGIC);
    text.push('\n');
    for (key, value) in KEYS.iter().zip([
        record.slot.as_str(),
        record.reference.as_str(),
        record.pane.as_str(),
        record.launch_id.as_str(),
        record.actor.as_str(),
        &record.attempts.to_string(),
        &record.created.to_string(),
        record.phase.word(),
    ]) {
        text.push_str(key);
        text.push('=');
        text.push_str(value);
        text.push('\n');
    }
    text.push_str(BODY_MARKER);
    text.push('\n');
    text.push_str(&record.body);
    text
}

/// Take one line off `rest`, advancing past its newline.
///
/// `None` once `rest` is empty. A final line with no newline is returned whole,
/// which is what makes a truncated record miss its body marker rather than
/// silently borrow the next field.
fn take_line<'a>(rest: &mut &'a str) -> Option<&'a str> {
    if rest.is_empty() {
        return None;
    }
    let Some(at) = rest.find('\n') else {
        let line = *rest;
        *rest = "";
        return Some(line);
    };
    let line = &rest[..at];
    *rest = &rest[at + 1..];
    Some(line)
}

/// A canonical unsigned decimal: digits only, and no leading zero unless the
/// value IS zero. `01`, `+1` and a space-padded count are damage, because two
/// spellings of one number are two records that compare unequal.
fn canonical_decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if value.len() > 1 && value.starts_with('0') {
        return None;
    }
    value.parse::<u64>().ok()
}

/// A slot: alphanumeric first, then the characters a file name may carry. `.`
/// and `..` are refused by the first-character rule, so a slot can never walk
/// out of its own directory.
fn is_slot(slot: &str) -> bool {
    let mut bytes = slot.bytes();
    match bytes.next() {
        Some(byte) if byte.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    slot.len() <= SLOT_CAP
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A pane id as tmux spells one.
fn is_pane(pane: &str) -> bool {
    let Some(digits) = pane.strip_prefix('%') else {
        return false;
    };
    !digits.is_empty() && pane.len() <= PANE_CAP && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// A body ae may paste: present, not blank, and carrying no control byte
/// beyond the tab and newline a brief legitimately has.
fn is_body(body: &str) -> bool {
    !body.is_empty()
        && body.chars().any(|ch| !ch.is_whitespace())
        && !body
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

/// What a record's BYTES say — the pure half, and the one the fuzz lane drives.
///
/// Bounded before it allocates, and clock-free on purpose: whether a record has
/// EXPIRED is a question about now, and it belongs to the caller that holds a
/// clock, not to the grammar.
///
/// # Errors
///
/// [`Damage`], naming the first deviation found.
pub fn parse(bytes: &[u8]) -> Result<Record, Damage> {
    if bytes.len() as u64 > RECORD_CAP {
        return Err(Damage::Oversize);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(Damage::NotUtf8);
    };
    let mut rest = text;
    if take_line(&mut rest) != Some(MAGIC) {
        return Err(Damage::Magic);
    }
    let mut values: Vec<&str> = Vec::with_capacity(KEYS.len());
    for key in KEYS {
        let Some(line) = take_line(&mut rest) else {
            return Err(Damage::Header);
        };
        // The FIRST `=` only: a launch token or a brief reference may carry one
        // of its own, and splitting on the last would hand the value's tail to
        // the key.
        let Some((found, value)) = line.split_once('=') else {
            return Err(Damage::Header);
        };
        if found != key {
            return Err(Damage::Header);
        }
        values.push(value);
    }
    let [
        slot,
        reference,
        pane,
        launch_id,
        actor,
        attempts,
        created,
        phase,
    ] = values[..]
    else {
        return Err(Damage::Header);
    };
    if take_line(&mut rest) != Some(BODY_MARKER) {
        return Err(Damage::BodyMarker);
    }
    if !is_slot(slot) {
        return Err(Damage::Slot);
    }
    if reference != format!("spawn-{slot}") {
        return Err(Damage::Reference);
    }
    if !is_pane(pane) {
        return Err(Damage::Pane);
    }
    if launch_id.is_empty()
        || launch_id.len() > LAUNCH_ID_CAP
        || !launch_id.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Damage::LaunchId);
    }
    if !crate::config::is_agent_name(actor) && actor != crate::deliver::UNVERIFIED {
        return Err(Damage::Actor);
    }
    let Some(attempts) = canonical_decimal(attempts).and_then(|count| u32::try_from(count).ok())
    else {
        return Err(Damage::Attempts);
    };
    if attempts > MAX_ATTEMPTS {
        return Err(Damage::Attempts);
    }
    let created = match canonical_decimal(created).and_then(|epoch| i64::try_from(epoch).ok()) {
        Some(epoch) if epoch > 0 => epoch,
        _ => return Err(Damage::Created),
    };
    let Some(phase) = Phase::from_word(phase) else {
        return Err(Damage::PhaseWord);
    };
    if !is_body(rest) {
        return Err(Damage::Body);
    }
    Ok(Record {
        slot: slot.to_owned(),
        reference: reference.to_owned(),
        pane: pane.to_owned(),
        launch_id: launch_id.to_owned(),
        actor: actor.to_owned(),
        attempts,
        created,
        phase,
        body: rest.to_owned(),
    })
}

/// Read the record for `slot`, if there is one.
///
/// `None` means no record — which is the ordinary case for every seat whose
/// brief landed. `Some(Err(..))` is a file that is there and is not a record.
///
/// The node is classified WITHOUT following a link and refused unless it is a
/// regular file, so a symlink planted at the name is never opened, and the cap
/// binds twice — on the observed length and again on the read that allocates.
/// The residual is the same one [`crate::store::read_source`] carries and is
/// stated rather than papered over: a replacement between the observation and
/// the open is not atomic, so the claim is "an observed non-regular node is
/// refused before the open", never atomicity.
#[must_use]
pub fn read(dir: &Path, slot: &str) -> Option<Result<Record, Damage>> {
    use std::io::Read as _;

    let path = path(dir, slot);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a retry record is classified WITHOUT following a link to it, so a link planted at the name cannot make the watchdog read a file outside this session — see the module docs"
    )]
    let probe = std::fs::symlink_metadata(&path);
    let meta = match probe {
        Ok(meta) => meta,
        // The ordinary case by far: almost every seat's brief landed.
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(Err(Damage::Unreadable)),
    };
    if !meta.is_file() {
        return Some(Err(Damage::Unreadable));
    }
    if meta.len() > RECORD_CAP {
        return Some(Err(Damage::Oversize));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the record ae itself published is the only source of a retried brief — see the module docs"
    )]
    let opened = std::fs::File::open(&path);
    let Ok(file) = opened else {
        return Some(Err(Damage::Unreadable));
    };
    let mut bytes = Vec::new();
    // The cap AGAIN, on the read itself: the size above was a different moment,
    // and this one is what actually allocates.
    if file.take(RECORD_CAP + 1).read_to_end(&mut bytes).is_err() {
        return Some(Err(Damage::Unreadable));
    }
    Some(parse(&bytes))
}

/// Publish `record` durably, at `0600`, for a slot that has NONE yet.
///
/// # This function does not serialize itself
///
/// It is one of the mutations of a slot's record, and every one of them —
/// this, [`remove`], and the compare-and-swap re-arm the delivery leg makes —
/// must be performed under that slot's RECORD LOCK. Nothing here takes it,
/// because the lock has to span a caller's whole read-modify-write, not one
/// write inside it. A caller that mutates a record without holding it is the
/// defect this sentence exists to prevent.
///
/// # An occupied name is refused, loudly
///
/// A rename would replace an existing record in silence, which would reset its
/// attempt count and destroy a `pasting` mark — the crash-window proof — with
/// no sound at all. No caller has a reason to publish over a live record: a
/// spawn clears a stale one first, and a re-arm goes through the
/// compare-and-swap, never through here. So an occupied name is a wiring
/// defect, and it is reported rather than absorbed. The check is sound because
/// of the lock contract above, not on its own.
///
/// # Durability
///
/// Temp, `fsync`, rename — the shape [`crate::store::SessionStore::stamp_launch_attempt`]
/// uses. The file's own bytes are synced, so a record survives THIS PROCESS
/// dying; the containing directory is not, so a machine that stops may still
/// lose the rename. Stated rather than overclaimed: the residual is the same
/// one the stamp carries, and the delivery leg fails closed over it, because a
/// record that vanished is a brief nobody retries rather than one delivered
/// twice. The mode is set ON the create, not after it, because the body is the
/// brief and a window where it is world-readable is a window too many. A temp
/// carries this process's pid, so a crash between the create and the rename
/// leaves a file that blocks only a later process reusing that pid.
///
/// The writer PROVES the reader will accept what it wrote: the rendered bytes
/// are parsed back before they are published, and a record that would not parse
/// is refused loudly instead of being born inert.
///
/// # Errors
///
/// The refusal, named: too large, unreadable by its own parser, a name already
/// occupied, or the write, `fsync` or rename that failed.
pub fn publish(dir: &Path, record: &Record) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let text = render(record);
    if text.len() as u64 > RECORD_CAP {
        return Err(format!(
            "the brief is {} B, past the {RECORD_CAP} B durable-retry bound",
            text.len()
        ));
    }
    if let Err(damage) = parse(text.as_bytes()) {
        return Err(format!("the record would not read back: {damage}"));
    }
    let dest = path(dir, &record.slot);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the occupied-name refusal classifies the destination WITHOUT following a link to it — see the durability note"
    )]
    let occupied = std::fs::symlink_metadata(&dest);
    if occupied.is_ok() {
        return Err(format!(
            "{} already holds a record — nothing was overwritten; a live record is never published over",
            dest.display()
        ));
    }
    let temp = {
        let mut name = dest.clone().into_os_string();
        name.push(format!(".tmp.{}", std::process::id()));
        PathBuf::from(name)
    };
    // EXCLUSIVE and 0600 at once: the name is predictable and sits in state a
    // human edits, so a plain create would FOLLOW a link planted there, and a
    // mode set after the write would expose the brief in between.
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp);
    let mut file = match created {
        Ok(file) => file,
        Err(why) => {
            return Err(format!(
                "{}: {why} — nothing was overwritten; remove that file if it is stale",
                temp.display()
            ));
        }
    };
    let published = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| std::fs::rename(&temp, &dest));
    if let Err(why) = published {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("could not publish {}: {why}", dest.display()));
    }
    Ok(())
}

/// Drop the record for `slot`, if any. Absent is success: a deletion that
/// finds nothing has already happened.
///
/// Like [`publish`], this does not serialize itself: it is a mutation of the
/// slot's record and belongs under that slot's record lock, held across the
/// caller's whole sequence.
pub fn remove(dir: &Path, slot: &str) {
    let _ = std::fs::remove_file(path(dir, slot));
}

#[cfg(test)]
mod tests {
    use super::{
        Damage, LAUNCH_ID_CAP, MAX_ATTEMPTS, PANE_CAP, Phase, RECORD_CAP, Record, SLOT_CAP,
        damaged_path, parse, path, publish, read, remove, render,
    };
    use std::path::PathBuf;

    /// A record every test starts from, so a test names only what it changes.
    fn record() -> Record {
        Record {
            slot: "spawned.1".to_owned(),
            reference: "spawn-spawned.1".to_owned(),
            pane: "%105".to_owned(),
            launch_id: "claude-1789105855".to_owned(),
            actor: "lead".to_owned(),
            attempts: 0,
            created: 1_789_105_855,
            phase: Phase::Armed,
            body: "Read the brief at /tmp/brief.md".to_owned(),
        }
    }

    /// A scratch directory of this test's own.
    fn scratch(tag: &str) -> PathBuf {
        let dir = PathBuf::from(format!("/tmp/ae-brr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch dir");
        dir
    }

    /// The bytes of `record` with one header line's VALUE replaced.
    fn with(key: &str, value: &str) -> String {
        render(&record())
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{key}=")) {
                    format!("{key}={value}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_record_renders_back_to_exactly_what_it_parsed_from() {
        let original = record();
        let text = render(&original);
        assert_eq!(parse(text.as_bytes()), Ok(original));
    }

    #[test]
    fn a_body_keeps_its_newlines_and_its_tabs_byte_for_byte() {
        let mut original = record();
        original.body = "first\n\tsecond\n\nthird".to_owned();
        let text = render(&original);
        let parsed = parse(text.as_bytes()).expect("the record parses");
        assert_eq!(parsed.body, "first\n\tsecond\n\nthird");
    }

    #[test]
    fn a_body_may_contain_the_body_marker_as_a_line_of_its_own() {
        // The marker only ends the HEADERS. Once the body has begun, nothing
        // in it is a delimiter, so a brief that talks about a body line is
        // still that brief.
        let mut original = record();
        original.body = "body\nbody\n".to_owned();
        assert_eq!(parse(render(&original).as_bytes()), Ok(original));
    }

    #[test]
    fn a_value_may_carry_the_separator_because_the_split_is_at_the_first_one() {
        let mut original = record();
        original.launch_id = "k=v=w".to_owned();
        assert_eq!(parse(render(&original).as_bytes()), Ok(original));
    }

    #[test]
    fn oversize_is_refused_before_anything_is_parsed() {
        let huge = vec![b'x'; usize::try_from(RECORD_CAP).expect("the cap fits") + 1];
        assert_eq!(parse(&huge), Err(Damage::Oversize));
    }

    #[test]
    fn bytes_that_are_not_text_are_not_a_record() {
        assert_eq!(parse(b"brief-retry 1\n\xff\xfe"), Err(Damage::NotUtf8));
    }

    #[test]
    fn a_record_must_begin_with_its_own_version_line() {
        assert_eq!(parse(b""), Err(Damage::Magic));
        let text = render(&record()).replacen("brief-retry 1", "brief-retry 2", 1);
        assert_eq!(parse(text.as_bytes()), Err(Damage::Magic));
    }

    #[test]
    fn a_reordered_duplicated_or_unknown_header_is_one_refusal() {
        let text = render(&record());
        let reordered = text.replacen(
            "slot=spawned.1\nreference=spawn-spawned.1",
            "reference=spawn-spawned.1\nslot=spawned.1",
            1,
        );
        assert_eq!(parse(reordered.as_bytes()), Err(Damage::Header));
        let duplicated = text.replacen("slot=spawned.1\n", "slot=spawned.1\nslot=spawned.1\n", 1);
        assert_eq!(parse(duplicated.as_bytes()), Err(Damage::Header));
        let unknown = text.replacen("pane=", "panel=", 1);
        assert_eq!(parse(unknown.as_bytes()), Err(Damage::Header));
        let no_separator = text.replacen("pane=%105", "pane %105", 1);
        assert_eq!(parse(no_separator.as_bytes()), Err(Damage::Header));
    }

    #[test]
    fn a_slot_can_never_walk_out_of_its_own_directory() {
        for hostile in ["..", ".", "../../etc", "/etc/passwd", ""] {
            let text = with("slot", hostile);
            assert_eq!(
                parse(text.as_bytes()),
                Err(Damage::Slot),
                "a slot of {hostile:?} must be damage"
            );
        }
    }

    #[test]
    fn the_reference_must_name_the_records_own_slot() {
        assert_eq!(
            parse(with("reference", "spawn-other").as_bytes()),
            Err(Damage::Reference)
        );
        assert_eq!(
            parse(with("reference", "spawned.1").as_bytes()),
            Err(Damage::Reference)
        );
    }

    #[test]
    fn a_pane_id_is_a_percent_and_digits() {
        for hostile in ["105", "%", "%1a", "%-1", "%99999999999999999999"] {
            assert_eq!(
                parse(with("pane", hostile).as_bytes()),
                Err(Damage::Pane),
                "a pane of {hostile:?} must be damage"
            );
        }
    }

    #[test]
    fn the_actor_is_an_allowlist_because_it_reaches_a_provenance_first_line() {
        // It names the brief's authority. A value that is neither an agent name
        // nor the unverified spelling is damage rather than a marker nobody
        // can attribute.
        assert_eq!(
            parse(with("actor", "bad actor").as_bytes()),
            Err(Damage::Actor)
        );
        assert_eq!(parse(with("actor", "").as_bytes()), Err(Damage::Actor));
        assert_eq!(
            parse(with("actor", "-leading").as_bytes()),
            Err(Damage::Actor)
        );
        let human = with("actor", crate::deliver::UNVERIFIED);
        assert_eq!(
            parse(human.as_bytes()).map(|record| record.actor),
            Ok(crate::deliver::UNVERIFIED.to_owned()),
            "a human-spawned seat's brief is UNVERIFIED and must parse"
        );
    }

    #[test]
    fn an_attempt_count_is_canonical_and_within_its_bound() {
        for hostile in ["01", "+1", " 1", "1 ", "", "-1", "3"] {
            assert_eq!(
                parse(with("attempts", hostile).as_bytes()),
                Err(Damage::Attempts),
                "an attempts of {hostile:?} must be damage"
            );
        }
        for good in 0..=MAX_ATTEMPTS {
            let text = with("attempts", &good.to_string());
            assert_eq!(
                parse(text.as_bytes()).map(|record| record.attempts),
                Ok(good)
            );
        }
    }

    #[test]
    fn a_creation_moment_is_canonical_and_strictly_positive() {
        for hostile in ["0", "-1", "01", "+1", "", "nine"] {
            assert_eq!(
                parse(with("created", hostile).as_bytes()),
                Err(Damage::Created),
                "a created of {hostile:?} must be damage"
            );
        }
        // The far edge parses: refusing it here would be the grammar deciding
        // an expiry question that belongs to the clock's owner.
        let far = with("created", &i64::MAX.to_string());
        assert_eq!(
            parse(far.as_bytes()).map(|record| record.created),
            Ok(i64::MAX)
        );
        // Past i64 it is not a moment at all.
        let past = with("created", "9223372036854775808");
        assert_eq!(parse(past.as_bytes()), Err(Damage::Created));
    }

    #[test]
    fn a_phase_is_one_of_exactly_two_words() {
        assert_eq!(
            parse(with("phase", "halfway").as_bytes()),
            Err(Damage::PhaseWord)
        );
        assert_eq!(parse(with("phase", "").as_bytes()), Err(Damage::PhaseWord));
        let pasting = with("phase", "pasting");
        assert_eq!(
            parse(pasting.as_bytes()).map(|record| record.phase),
            Ok(Phase::Pasting)
        );
    }

    #[test]
    fn a_missing_marker_or_an_unpasteable_body_is_damage() {
        let headers_only = render(&record());
        let cut = headers_only
            .split_once("body\n")
            .map(|(head, _)| head.to_owned())
            .expect("the marker is there");
        assert_eq!(parse(cut.as_bytes()), Err(Damage::BodyMarker));
        let mut blank = record();
        blank.body = "   \n\t".to_owned();
        assert_eq!(parse(render(&blank).as_bytes()), Err(Damage::Body));
        let mut empty = record();
        empty.body = String::new();
        assert_eq!(parse(render(&empty).as_bytes()), Err(Damage::Body));
        let mut control = record();
        control.body = "before\u{7}after".to_owned();
        assert_eq!(parse(render(&control).as_bytes()), Err(Damage::Body));
    }

    #[test]
    fn a_published_record_reads_back_and_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch("publish");
        let original = record();
        assert_eq!(publish(&dir, &original), Ok(()));
        assert_eq!(read(&dir, &original.slot), Some(Ok(original.clone())));
        #[allow(
            clippy::disallowed_methods,
            reason = "a test inspecting the mode it just asserted about; the boundary is over what PRODUCT code may reach"
        )]
        let observed = std::fs::metadata(path(&dir, &original.slot));
        let mode = observed.expect("the record is there").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the body is the brief: owner-only, always");
        remove(&dir, &original.slot);
        assert_eq!(read(&dir, &original.slot), None, "a removed record is gone");
        remove(&dir, &original.slot);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_writer_refuses_to_publish_what_its_own_reader_would_reject() {
        // The record is born readable or it is not born. A record written past
        // the cap, or one whose fields the parser would refuse, would sit on
        // disk as a brief that can never be retried and never be explained.
        let dir = scratch("writerguard");
        let mut oversize = record();
        oversize.body = "x".repeat(usize::try_from(RECORD_CAP).expect("the cap fits"));
        let refused = publish(&dir, &oversize).expect_err("an oversize record is refused");
        assert!(refused.contains("durable-retry bound"), "{refused}");
        assert_eq!(read(&dir, &oversize.slot), None, "and nothing was written");

        let mut unparseable = record();
        unparseable.actor = "not an actor".to_owned();
        let refused = publish(&dir, &unparseable).expect_err("an unreadable record is refused");
        assert!(refused.contains("would not read back"), "{refused}");
        assert_eq!(
            read(&dir, &unparseable.slot),
            None,
            "and nothing was written"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_record_is_never_published_over_in_silence() {
        // A rename would reset the attempt count and destroy a `pasting`
        // mark — the crash-window proof — without a sound. No caller has a
        // reason to do it, so an occupied name is a wiring defect and says so.
        let dir = scratch("occupied");
        let first = record();
        assert_eq!(publish(&dir, &first), Ok(()));
        let mut second = record();
        second.attempts = 2;
        second.phase = Phase::Pasting;
        second.body = "a different brief entirely".to_owned();
        let refused = publish(&dir, &second).expect_err("an occupied name is refused");
        assert!(refused.contains("already holds a record"), "{refused}");
        assert_eq!(
            read(&dir, &first.slot),
            Some(Ok(first)),
            "and the record that was there is untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_planted_at_the_record_name_is_refused_rather_than_followed() {
        // The headline claim of the read: a link planted at a predictable name
        // in state a human edits must never make the watchdog read — and then
        // PASTE — a file from somewhere else entirely.
        let dir = scratch("symlink");
        let elsewhere = dir.join("elsewhere");
        std::fs::write(&elsewhere, render(&record())).expect("the link target");
        std::os::unix::fs::symlink(&elsewhere, path(&dir, "spawned.1")).expect("the planted link");
        assert_eq!(
            read(&dir, "spawned.1"),
            Some(Err(Damage::Unreadable)),
            "a symlink is not a regular file, and is refused before any open"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_bounded_field_is_pinned_at_its_own_edge() {
        // Each bound is pinned on BOTH sides, because a bound tested only from
        // the inside survives being widened.
        let at_cap = "s".repeat(SLOT_CAP);
        let over_cap = "s".repeat(SLOT_CAP + 1);
        for (slot, ok) in [(at_cap.as_str(), true), (over_cap.as_str(), false)] {
            let text = render(&Record {
                slot: slot.to_owned(),
                reference: format!("spawn-{slot}"),
                ..record()
            });
            assert_eq!(
                parse(text.as_bytes()).is_ok(),
                ok,
                "slot of {} bytes",
                slot.len()
            );
        }
        let launch_at = "l".repeat(LAUNCH_ID_CAP);
        let launch_over = "l".repeat(LAUNCH_ID_CAP + 1);
        assert!(parse(with("launch_id", &launch_at).as_bytes()).is_ok());
        assert_eq!(
            parse(with("launch_id", &launch_over).as_bytes()),
            Err(Damage::LaunchId)
        );
        // A pane id at the cap and one byte past it.
        let pane_at = format!("%{}", "9".repeat(PANE_CAP - 1));
        let pane_over = format!("%{}", "9".repeat(PANE_CAP));
        assert!(parse(with("pane", &pane_at).as_bytes()).is_ok());
        assert_eq!(
            parse(with("pane", &pane_over).as_bytes()),
            Err(Damage::Pane)
        );
        // Past u32 the attempt count is not a count ae could have written.
        assert_eq!(
            parse(with("attempts", "4294967296").as_bytes()),
            Err(Damage::Attempts)
        );
    }

    #[test]
    fn a_record_is_named_from_a_slot_and_never_from_a_listing() {
        let dir = PathBuf::from("/tmp/session");
        assert_eq!(
            path(&dir, "spawned.1"),
            PathBuf::from("/tmp/session/brief-retry.spawned.1.rec")
        );
        assert_eq!(
            damaged_path(&dir, "spawned.1"),
            PathBuf::from("/tmp/session/brief-retry.spawned.1.rec.damaged")
        );
        // A hostile slot cannot escape the session directory even before the
        // grammar refuses it: every separator is sanitized away, so whatever
        // dots survive, the name is still one leaf inside this session's dir.
        let escaped = path(&dir, "../../etc/passwd");
        assert_eq!(
            escaped,
            PathBuf::from("/tmp/session/brief-retry..._.._etc_passwd.rec")
        );
        assert_eq!(
            escaped.parent(),
            Some(dir.as_path()),
            "a sanitized name never leaves the session directory"
        );
    }

    #[test]
    fn a_damaged_record_is_read_as_damage_and_never_as_a_brief() {
        let dir = scratch("damaged");
        std::fs::write(path(&dir, "spawned.1"), "not a record at all\n").expect("the planted file");
        assert_eq!(read(&dir, "spawned.1"), Some(Err(Damage::Magic)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_planted_at_the_record_name_is_refused_rather_than_opened() {
        let dir = scratch("nonregular");
        std::fs::create_dir_all(path(&dir, "spawned.1")).expect("the planted directory");
        assert_eq!(read(&dir, "spawned.1"), Some(Err(Damage::Unreadable)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
