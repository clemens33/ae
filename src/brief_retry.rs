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

use std::io::Write;
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
    /// The node at the name is not a regular file — a directory, a symlink, a
    /// device. TAMPERING, and permanent: nothing transient turns a record into
    /// a directory.
    NotRegular,
    /// The file is THERE and the open or the read FAILED, so ae never saw the
    /// bytes. Distinct from every grammar arm on purpose, and from
    /// [`Damage::NotRegular`] too: this one may be a passing `EMFILE` or `EIO`,
    /// so it is never destroyed on sight.
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
            Self::NotRegular => "the name holds something that is not a regular file",
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

/// A record that is not one, and when the file was last written.
///
/// The moment comes from the stat the read ALREADY made, never from a second
/// look at the world: it is what lets a read failure be told apart from a
/// permanent one without holding any state between cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damaged {
    /// What is wrong.
    pub kind: Damage,
    /// The file's mtime as an epoch, when the stat that found it succeeded.
    pub modified: Option<i64>,
}

impl Damaged {
    /// What is wrong, for a caller that does not care when.
    #[must_use]
    pub const fn kind(self) -> Damage {
        self.kind
    }

    /// Damage observed without a moment to date it.
    const fn undated(kind: Damage) -> Self {
        Self {
            kind,
            modified: None,
        }
    }
}

/// Whether damage is worth destroying the record over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permanence {
    /// The record will never become readable: ae saw the bytes and they are
    /// wrong, or the node is not a file at all.
    Permanent,
    /// ae never saw the bytes, and a next cycle may. Destroying this would
    /// throw away a brief that was fine.
    Transient,
}

/// How long a brief may wait for delivery before it is given up.
pub const AGE_BOUND_SECS: i64 = 1_800;

/// Whether `damaged` should be destroyed, or left for a later cycle.
///
/// Bytes ae SAW and refused are permanent, and so is a node that is not a
/// regular file — nothing transient turns a record into a directory. A read
/// that FAILED is the only ambiguous one, and it is dated rather than guessed:
/// a file still unreadable past the age bound was never going to be read, while
/// a younger one may be a passing `EMFILE`. Undated damage stays transient,
/// because a stat that did not answer is not evidence of anything.
#[must_use]
pub const fn permanence(damaged: &Damaged, now: i64) -> Permanence {
    match damaged.kind {
        Damage::Unreadable => match damaged.modified {
            Some(modified) if now.saturating_sub(modified) > AGE_BOUND_SECS => {
                Permanence::Permanent
            }
            _ => Permanence::Transient,
        },
        _ => Permanence::Permanent,
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

/// The bytes at `slot`'s record name, with the moment the stat observed.
///
/// `Ok(None)` is no record at all — the ordinary case for every seat whose
/// brief landed. The node is classified WITHOUT following a link and refused
/// unless it is a regular file, so a symlink planted at the name is never
/// opened, and the cap binds twice: on the observed length, and again on the
/// read that allocates.
///
/// The residual is the same one [`crate::store::read_source`] carries and is
/// stated rather than papered over: a replacement between the observation and
/// the open is not atomic, so the claim is "an observed non-regular node is
/// refused before the open", never atomicity.
struct Slurped {
    bytes: Vec<u8>,
    modified: Option<i64>,
}

fn slurp(dir: &Path, slot: &str) -> Result<Option<Slurped>, Damaged> {
    use std::io::Read as _;

    let path = path(dir, slot);
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: a retry record is classified WITHOUT following a link to it, so a link planted at the name cannot make the watchdog read a file outside this session — see the module docs"
    )]
    let probe = std::fs::symlink_metadata(&path);
    let meta = match probe {
        Ok(meta) => meta,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Damaged::undated(Damage::Unreadable)),
    };
    // The SAME stat answers both questions: what the node is, and when it was
    // last written. Dating a read failure needs no second look at the world.
    let modified = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_secs()).ok());
    let dated = |kind| Damaged { kind, modified };
    if !meta.is_file() {
        return Err(dated(Damage::NotRegular));
    }
    if meta.len() > RECORD_CAP {
        return Err(dated(Damage::Oversize));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the record ae itself published is the only source of a retried brief — see the module docs"
    )]
    let opened = std::fs::File::open(&path);
    let Ok(file) = opened else {
        return Err(dated(Damage::Unreadable));
    };
    let mut bytes = Vec::new();
    // The cap AGAIN, on the read itself: the size above was a different moment,
    // and this one is what actually allocates.
    if file.take(RECORD_CAP + 1).read_to_end(&mut bytes).is_err() {
        return Err(dated(Damage::Unreadable));
    }
    Ok(Some(Slurped { bytes, modified }))
}

/// Read the record for `slot`, if there is one.
///
/// `None` means no record — which is the ordinary case for every seat whose
/// brief landed. `Some(Err(..))` is a file that is there and is not a record,
/// carrying the moment [`permanence`] dates it by.
#[must_use]
pub fn read(dir: &Path, slot: &str) -> Option<Result<Record, Damaged>> {
    match slurp(dir, slot) {
        Ok(None) => None,
        Ok(Some(found)) => Some(parse(&found.bytes).map_err(|kind| Damaged {
            kind,
            modified: found.modified,
        })),
        Err(damaged) => Some(Err(damaged)),
    }
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
    write_record(dir, record, Occupied::Refuse)
}

/// What a write does when the destination name is already taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occupied {
    /// Report it: no caller publishes over a live record.
    Refuse,
    /// Replace it: the caller proved, under the record lock, that what is there
    /// is the very record this flight wrote.
    Replace,
}

fn write_record(dir: &Path, record: &Record, occupied: Occupied) -> Result<(), String> {
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
    let taken = std::fs::symlink_metadata(&dest);
    if occupied == Occupied::Refuse && taken.is_ok() {
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

/// How far in the future a record may claim to have been created before that
/// claim is itself the fault. A host whose clock steps backwards is plausible;
/// half an hour of it is not.
pub const FUTURE_SKEW_SECS: i64 = 300;

/// What a cycle should do with one record. The ONE gate, and the only place
/// the bounds, the incarnation and the readiness are weighed together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Take the record: publish the flight mark and enter delivery.
    Deliver,
    /// Destroy it, loudly, for this reason.
    GiveUp(&'static str),
    /// Leave it exactly as it is and look again next cycle.
    Skip(&'static str),
}

/// What a cycle knows about the seat a record names.
///
/// Each field names the store it came from, because they are different stores
/// and a reader that forgets which is which is how an incarnation check starts
/// trusting the wrong one: the name and the launch token are META, the live
/// pane is THIS CYCLE'S tmux read, and liveness and readiness are the delivery
/// module's own owners.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Facts<'a> {
    /// The roster name meta gives this slot, or `None` if meta did not answer.
    pub meta_name: Option<&'a str>,
    /// `launch_id.<slot>` as meta spells it now, or `None` if meta did not
    /// answer.
    pub meta_launch_id: Option<&'a str>,
    /// The pane this cycle saw carrying the slot, or `None` if none did.
    pub live_pane: Option<&'a str>,
    /// What the one liveness owner says about that pane.
    pub liveness: crate::deliver::PaneLiveness,
    /// Whether the input box proved idle.
    pub ready: bool,
    /// Now.
    pub now: i64,
}

/// Weigh one record against what the cycle knows. FAIL CLOSED: every answer
/// that is not positive proof is [`Decision::Skip`], which changes nothing.
///
/// The order is the contract. A record mid-flight is decided before anything
/// else, because its outcome is unknown and no later fact can make pasting it
/// again safe. An incarnation is refused only on POSITIVE proof — meta ANSWERED
/// and named a different seat — so a meta that could not be read skips rather
/// than destroying a brief, the same shape [`crate::tmux::classify_absence`]
/// uses for a session. Busy and human-typing land on the readiness arm, which
/// is what keeps them free: they skip the cycle and spend no attempt.
pub(crate) fn decide(record: &Record, facts: &Facts<'_>) -> Decision {
    // THE CRASH WINDOW. A flight published this before entering delivery and
    // never recorded an outcome, so ae cannot know whether the paste landed.
    // Pasting again could deliver the brief twice; this is the arm that makes
    // that impossible.
    if record.phase == Phase::Pasting {
        return Decision::GiveUp("paste outcome unknown");
    }
    let (Some(_name), Some(launch_id)) = (facts.meta_name, facts.meta_launch_id) else {
        return Decision::Skip("the session meta did not answer for this slot");
    };
    // POSITIVE PROOF of a new incarnation: meta answered, and it names someone
    // else's seat. Delivering here would paste one agent's brief into another.
    if launch_id != record.launch_id {
        return Decision::GiveUp("the seat was relaunched under a new launch token");
    }
    if let Some(pane) = facts.live_pane
        && pane != record.pane
    {
        return Decision::GiveUp("the slot moved to a different pane");
    }
    if record.created.saturating_sub(facts.now) > FUTURE_SKEW_SECS {
        return Decision::GiveUp("the record is dated in the future");
    }
    if facts.now.saturating_sub(record.created) > AGE_BOUND_SECS {
        return Decision::GiveUp("the brief went undelivered for 30 minutes");
    }
    if record.attempts >= MAX_ATTEMPTS {
        return Decision::GiveUp("delivery was attempted twice");
    }
    if facts.live_pane.is_none() {
        return Decision::Skip("no live pane carries the slot this cycle");
    }
    if facts.liveness != crate::deliver::PaneLiveness::Alive {
        return Decision::Skip("the pane is not a proven live agent");
    }
    // WHERE BUSY AND HUMAN-TYPING LAND, and why they cost nothing: readiness is
    // proved BEFORE any attempt is published, so a seat whose box is occupied
    // is simply looked at again next cycle.
    if !facts.ready {
        return Decision::Skip("the input box is not a confirmed-idle state");
    }
    // Package 2 inserts its human-prompt latch HERE, as one more Skip arm, so
    // it never has to reinterpret anything above it.
    Decision::Deliver
}

/// Move a damaged record aside so it can never be read as one again, keeping
/// whatever was moved aside FIRST.
///
/// The plain name is tried before the dated one because a single damaged record
/// is the ordinary case and its file should be easy to find. A collision never
/// overwrites: the first forensics are the ones worth keeping, and a second
/// damaged record at the same slot is a rarer event than losing the evidence of
/// the first.
///
/// # Errors
///
/// Both names taken, or the rename itself failed — the caller reports and
/// skips, and never loops on it.
pub(crate) fn mark_damaged(dir: &Path, slot: &str, now: i64) -> Result<PathBuf, String> {
    let from = path(dir, slot);
    let plain = damaged_path(dir, slot);
    let dated = {
        let mut name = plain.clone().into_os_string();
        name.push(format!(".{now}"));
        PathBuf::from(name)
    };
    for candidate in [plain, dated] {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: the keep-first check classifies the destination WITHOUT following a link to it — see `mark_damaged`"
        )]
        let taken = std::fs::symlink_metadata(&candidate).is_ok();
        if taken {
            continue;
        }
        return match std::fs::rename(&from, &candidate) {
            Ok(()) => Ok(candidate),
            Err(why) => Err(format!("could not set {} aside: {why}", from.display())),
        };
    }
    Err(format!(
        "{} is damaged and both set-aside names are taken; it was left alone",
        from.display()
    ))
}

/// Replace the record at `slot` ONLY while its bytes are still `witness`.
///
/// The compare-and-swap that keeps a finished flight from resurrecting a record
/// that someone else replaced: a `retire` plus a re-spawn during the flight
/// leaves a SUCCESSOR record at the same slot, and writing this flight's
/// outcome over it would hand the successor a stranger's attempt count.
///
/// `Ok(false)` is that mismatch — the record changed or vanished under the
/// flight — and it is not an error: it means this flight no longer owns
/// anything and must write nothing.
///
/// # Errors
///
/// The write that failed, named. The caller gives up loudly rather than
/// leaving a flight mark behind.
pub(crate) fn rearm_if_unchanged(
    dir: &Path,
    witness: &[u8],
    next: &Record,
) -> Result<bool, String> {
    match slurp(dir, &next.slot) {
        Ok(Some(found)) if found.bytes == witness => {
            write_record(dir, next, Occupied::Replace).map(|()| true)
        }
        _ => Ok(false),
    }
}

/// Drop the record at `slot` ONLY while its bytes are still `witness`.
///
/// The same guard as [`rearm_if_unchanged`], for the two arms that finish a
/// flight: a delivered brief and a given-up one both delete, and neither may
/// delete a SUCCESSOR record that a re-spawn wrote while the flight was in the
/// air. `false` means this flight no longer owns the record and removed
/// nothing.
pub(crate) fn remove_if_unchanged(dir: &Path, slot: &str, witness: &[u8]) -> bool {
    match slurp(dir, slot) {
        Ok(Some(found)) if found.bytes == witness => {
            remove(dir, slot);
            true
        }
        _ => false,
    }
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

// ---------------------------------------------------------------------------
// The delivery leg.

/// The action a caller names to reach this leg. It selects the leg and NOTHING
/// else: the text and the actor come from the record, so the worst a forged
/// trigger can do is re-fire a brief the spawner already authorized, sooner
/// than the watchdog would have.
pub const RETRY_ACTION: &str = "brief-retry";

/// The event a landed retry writes.
pub const DELIVERED_ACTION: &str = "brief-delivered";

/// The event a record's end writes, whatever ended it.
pub const GAVE_UP_ACTION: &str = "brief-gave-up";

/// The action the body store names the recovery file after — the SAME one the
/// original spawn used, because this is that spawn's brief and not a new
/// message.
const SPAWN_ACTION: &str = "spawn";

/// How many readiness polls a retry spends. Short on purpose: a cycle that
/// finds the box busy simply looks again next cycle, and spends no attempt.
const RETRY_READY_POLLS: u32 = 4;

/// Deliver the brief `slot`'s record holds, or say why it did not.
///
/// The argv named only WHICH seat. Everything that reaches the pane —- the
/// text, and the actor its provenance line names -— is read from the record,
/// so no caller can put words in a brief's mouth.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
pub fn run(
    dir: &Path,
    target: &str,
    own_session: &str,
    now: crate::time::Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> std::io::Result<u8> {
    use crate::state::EXIT_FAILED;

    let (resolved, server) = match crate::tracked::resolve_on(target, own_session, dir) {
        Ok(resolved) => resolved,
        Err(why) => {
            writeln!(err, "{}", why.message())?;
            return Ok(EXIT_FAILED);
        }
    };
    // OWN SESSION ONLY, outright. The record is named from this session's own
    // directory, so a target in another one could only ever be a mistake or an
    // attempt to aim someone else's brief.
    if !resolved.session.is_empty() && resolved.session != own_session {
        writeln!(
            err,
            "ae: {RETRY_ACTION} refused — {target} is in session '{}', and a brief is retried only into its own",
            resolved.session
        )?;
        return Ok(EXIT_FAILED);
    }
    if resolved.slot.is_empty() {
        writeln!(err, "ae: {RETRY_ACTION} refused — {target} carries no slot")?;
        return Ok(EXIT_FAILED);
    }
    // THE RECORD LOCK, held across the whole read-decide-write sequence. A
    // second helper — an orphan of a restarted daemon, or a forged trigger —
    // waits, fails, and skips, so two flights can never both publish a flight
    // mark and both paste.
    let Ok(_held) = crate::store::lock(&path(dir, &resolved.slot), crate::store::LOCK_WAIT) else {
        writeln!(
            err,
            "ae: {RETRY_ACTION} skipped — another flight holds {}'s record",
            resolved.slot
        )?;
        return Ok(EXIT_FAILED);
    };
    let Some(reading) = read(dir, &resolved.slot) else {
        writeln!(
            err,
            "ae: {RETRY_ACTION} refused — {target} has no undelivered brief on record"
        )?;
        return Ok(EXIT_FAILED);
    };
    let record = match reading {
        Ok(record) => record,
        // Damage is the sweep's to classify and set aside; a delivery leg that
        // acted on it would be deciding with bytes it could not read.
        Err(damaged) => {
            writeln!(
                err,
                "ae: {RETRY_ACTION} refused — {}'s record is damaged: {}",
                resolved.slot,
                damaged.kind()
            )?;
            return Ok(EXIT_FAILED);
        }
    };
    let seat = seat_facts(dir, &resolved.slot);
    let input = seat.tool.adapter().input;
    // The pane the NAME resolves to now. If the slot moved, this is a different
    // pane than the record names, and the gate refuses on that.
    let live_pane = (resolved.slot == record.slot).then(|| resolved.pane.clone());
    let liveness =
        crate::deliver::observe_pane_liveness(&server, dir, &resolved.pane, &resolved.slot);
    let ready = liveness == crate::deliver::PaneLiveness::Alive
        && crate::deliver::wait_input_ready(
            &server,
            &resolved.pane,
            input.model,
            input.composed,
            RETRY_READY_POLLS,
        );
    let facts = Facts {
        meta_name: seat.name.as_deref(),
        meta_launch_id: seat.launch_id.as_deref(),
        live_pane: live_pane.as_deref(),
        liveness,
        ready,
        now: now.epoch(),
    };
    let name = seat.name.as_deref().unwrap_or(target);
    match decide(&record, &facts) {
        Decision::Skip(why) => {
            writeln!(err, "ae: {RETRY_ACTION} skipped for {name} — {why}")?;
            Ok(EXIT_FAILED)
        }
        Decision::GiveUp(why) => {
            let witness = render(&record);
            give_up(dir, &record, name, why, witness.as_bytes(), now, err)?;
            writeln!(err, "ae: brief for {name} given up — {why}")?;
            Ok(EXIT_FAILED)
        }
        Decision::Deliver => fly(
            &Flight {
                dir,
                server: &server,
                pane: &resolved.pane,
                own_session,
                name,
                record: &record,
                composed: input.composed,
                now,
            },
            out,
            err,
        ),
    }
}

/// What meta says about the seat a slot holds right now.
struct Seat {
    /// The roster name, or `None` when meta did not answer for this slot.
    name: Option<String>,
    /// `launch_id.<slot>`, or `None` when meta did not answer.
    launch_id: Option<String>,
    /// The seat's tool, for the input grammar readiness is proved against.
    tool: crate::tool::ToolKind,
}

/// Read the seat's own facts, ONCE, from the meta document.
fn seat_facts(dir: &Path, slot: &str) -> Seat {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let meta = crate::meta::Meta::parse(&text);
    let entry = meta.roster().iter().find(|entry| entry.slot == slot);
    Seat {
        name: entry.map(|entry| entry.name.clone()),
        launch_id: crate::meta::sole_value(&bytes, &format!("launch_id.{slot}"))
            .map(String::from_utf8_lossy)
            .filter(|value| !value.is_empty())
            .map(std::borrow::Cow::into_owned),
        tool: crate::tool::ToolKind::from_binary_name(
            entry
                .and_then(|entry| entry.binary.as_deref())
                .unwrap_or(""),
        ),
    }
}

/// Everything one flight needs, so the call that takes off stays one statement.
struct Flight<'a> {
    dir: &'a Path,
    server: &'a crate::inventory::ServerId,
    pane: &'a str,
    own_session: &'a str,
    name: &'a str,
    record: &'a Record,
    composed: crate::tool::Composed,
    now: crate::time::Timestamp,
}

/// Publish the flight mark, deliver, and record what happened.
///
/// THE ORDER IS THE PROOF. The bumped attempt and the `pasting` mark are made
/// durable BEFORE the paste, so a crash anywhere after this point leaves a
/// record the next cycle refuses to paste again. Nothing about the outcome can
/// undo that: only a failure that proves NOTHING was staged re-arms it.
fn fly(flight: &Flight<'_>, out: &mut impl Write, err: &mut impl Write) -> std::io::Result<u8> {
    use crate::state::EXIT_FAILED;

    let witness = render(flight.record);
    let mut taking_off = flight.record.clone();
    taking_off.attempts = taking_off.attempts.saturating_add(1);
    taking_off.phase = Phase::Pasting;
    let mark = render(&taking_off);
    if let Err(why) = rearm_if_unchanged(flight.dir, witness.as_bytes(), &taking_off) {
        writeln!(
            err,
            "ae: brief for {} not attempted — its flight mark could not be published: {why}",
            flight.name
        )?;
        return Ok(EXIT_FAILED);
    }
    let request = crate::deliver::Request {
        dir: flight.dir,
        server: flight.server,
        pane: flight.pane,
        logged_target: flight.name,
        target_session: flight.own_session,
        pane_slot: &flight.record.slot,
        own_session: flight.own_session,
        action: SPAWN_ACTION,
        reference: &flight.record.reference,
        actor: &flight.record.actor,
        body: &flight.record.body,
        shape: crate::deliver::Shape::Launch,
        defer: crate::deliver::DEFAULT_DEFER,
        composed: flight.composed,
    };
    let outcome = crate::deliver::deliver(&request, err)?;
    let age = flight.now.epoch().saturating_sub(flight.record.created);
    match outcome {
        Ok(delivered) => {
            if remove_if_unchanged(flight.dir, &flight.record.slot, mark.as_bytes()) {
                let _ = std::fs::remove_file(
                    flight.dir.join(format!("undelivered.{}.txt", flight.name)),
                );
            }
            record_event(
                flight.dir,
                DELIVERED_ACTION,
                flight.record,
                flight.name,
                &format!(
                    "brief delivered on attempt {} after {age}s",
                    taking_off.attempts
                ),
                &delivered.body_file,
                flight.now,
            );
            writeln!(out, "Delivered the undelivered brief to {}", flight.name)?;
            Ok(0)
        }
        // PROVEN pre-stage: deliver refuses these before the first key reaches
        // the pane, so nothing was staged and the record may be armed again.
        Err(
            failure @ (crate::deliver::Failure::DeadPane
            | crate::deliver::Failure::Lock
            | crate::deliver::Failure::Abandoned
            | crate::deliver::Failure::NotComposed { .. }),
        ) => {
            rearm_after_prestage(flight, &taking_off, &mark, &failure, err)?;
            Ok(EXIT_FAILED)
        }
        // Everything else may have staged something. The brief is given up
        // rather than risked twice — the file is kept, so nothing is lost.
        Err(failure) => {
            let why = if matches!(failure, crate::deliver::Failure::Unconfirmed { .. }) {
                "submit unconfirmed; the brief may be staged unsent"
            } else {
                "delivery failed in a way that proves nothing about what was pasted"
            };
            give_up(
                flight.dir,
                &taking_off,
                flight.name,
                why,
                mark.as_bytes(),
                flight.now,
                err,
            )?;
            Ok(EXIT_FAILED)
        }
    }
}

/// Put a record back after a refusal that PROVED nothing was staged.
///
/// The attempt is already spent — ae entered delivery, which is what the count
/// means — so the record goes back armed with the higher count, and the seat
/// gets whatever attempts remain. The write is compare-and-swapped: a retire
/// and a re-spawn during the flight leave a SUCCESSOR record at this slot, and
/// handing it this flight's attempt count would charge a new brief for an old
/// one's failures.
fn rearm_after_prestage(
    flight: &Flight<'_>,
    taking_off: &Record,
    mark: &str,
    failure: &crate::deliver::Failure,
    err: &mut impl Write,
) -> std::io::Result<()> {
    if taking_off.attempts >= MAX_ATTEMPTS {
        return give_up(
            flight.dir,
            taking_off,
            flight.name,
            "delivery was attempted twice",
            mark.as_bytes(),
            flight.now,
            err,
        );
    }
    let mut armed = taking_off.clone();
    armed.phase = Phase::Armed;
    match rearm_if_unchanged(flight.dir, mark.as_bytes(), &armed) {
        Ok(true) => writeln!(
            err,
            "ae: brief for {} refused before anything was pasted ({failure:?}) — it stays on record",
            flight.name
        ),
        Ok(false) => writeln!(
            err,
            "ae: brief for {} was replaced while its delivery was in the air — nothing was written back",
            flight.name
        ),
        Err(why) => writeln!(
            err,
            "ae: brief for {} could not be re-armed ({why}) — its flight mark stands, so it will be given up rather than pasted twice",
            flight.name
        ),
    }
}

/// End a record: drop it if this flight still owns it, KEEP the preserved
/// `.txt`, and say so in the ledger.
///
/// The file stays on purpose. A given-up brief is one a human now has to hand
/// over, and the event is the signal to do it.
fn give_up(
    dir: &Path,
    record: &Record,
    name: &str,
    why: &str,
    witness: &[u8],
    now: crate::time::Timestamp,
    err: &mut impl Write,
) -> std::io::Result<()> {
    if !remove_if_unchanged(dir, &record.slot, witness) {
        writeln!(
            err,
            "ae: brief for {name} was replaced before it could be given up — nothing was removed"
        )?;
        return Ok(());
    }
    let age = now.epoch().saturating_sub(record.created);
    record_event(
        dir,
        GAVE_UP_ACTION,
        record,
        name,
        &format!(
            "{why} (attempts {}, age {age}s); the brief is preserved at undelivered.{name}.txt",
            record.attempts
        ),
        "",
        now,
    );
    Ok(())
}

/// Append one brief event, named by the ORIGINAL spawner.
///
/// Never the watchdog and never ae: the authority this brief carries is the
/// one that spawned the seat, and the ledger says the same thing the pane's
/// provenance line does.
fn record_event(
    dir: &Path,
    action: &str,
    record: &Record,
    name: &str,
    summary: &str,
    body_file: &str,
    now: crate::time::Timestamp,
) {
    let _ = crate::store::open(dir).append_event(&crate::tracked::event_line(
        &crate::tracked::EventFields {
            ts: now,
            actor: &record.actor,
            action,
            target: name,
            reference: &record.reference,
            actor_slot: "",
            actor_session: "",
            target_slot: &record.slot,
            target_session: "",
            target_server: "",
            target_pane: "",
            target_session_uuid: "",
            caller_server: "",
            caller_pane: "",
            caller_session_uuid: "",
            identity_gap: "",
            summary,
            body_file,
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::{
        Damage, Damaged, LAUNCH_ID_CAP, MAX_ATTEMPTS, PANE_CAP, Phase, RECORD_CAP, Record,
        SLOT_CAP, damaged_path, parse, path, publish, read, remove, render,
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
            read(&dir, "spawned.1").map(|reading| reading.map_err(Damaged::kind)),
            Some(Err(Damage::NotRegular)),
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
        assert_eq!(
            read(&dir, "spawned.1").map(|reading| reading.map_err(Damaged::kind)),
            Some(Err(Damage::Magic))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_planted_at_the_record_name_is_refused_rather_than_opened() {
        let dir = scratch("nonregular");
        std::fs::create_dir_all(path(&dir, "spawned.1")).expect("the planted directory");
        assert_eq!(
            read(&dir, "spawned.1").map(|reading| reading.map_err(Damaged::kind)),
            Some(Err(Damage::NotRegular))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
