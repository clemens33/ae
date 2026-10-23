//! Quota-driven reasoning-effort adaptation: the PURE half.
//!
//! A seat whose own account runs hot on its LONG quota window steps its
//! reasoning effort down ONE tier (max to xhigh, xhigh to high, never past
//! high) in its OWN profile command — the effort value alone changes, and a
//! configured twin profile at the tier below proves that command — then steps
//! back once that window is relieved. This module owns the vocabulary, the
//! twin derivation over `[profiles]`, the quota entry and recovery rules, the
//! persisted `effort.<slot>` record, the journal fold and the one decision.
//! Nothing here reads the world or acts on a pane; [`leg`] is the live half.

pub(crate) mod leg;

use std::path::Path;

use crate::autoreseat::{REARM_GAP, Room};
use crate::config::IdentityConfig;
use crate::events::{Event, SessionKey};
use crate::launch_cmd::{EffortRefusal, effort_skeleton, model_flag_value};
use crate::meta::RecordedClient;
use crate::quota::{LONG_MINUTES, Status, WindowKey, proven_long};
use crate::time::Timestamp;
use crate::tool::ToolKind;

/// One reasoning-effort level, in ascending order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// `minimal`.
    Minimal,
    /// `low`.
    Low,
    /// `medium`.
    Medium,
    /// `high`: the floor ae never steps below.
    High,
    /// `xhigh`.
    Xhigh,
    /// `max`.
    Max,
    /// `ultra`.
    Ultra,
}

impl Level {
    pub(crate) const ALL: [Self; 7] = [
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
        Self::Ultra,
    ];

    /// The level's one spelling, in a command, a record and a journal summary.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    /// The level spelled exactly `word`; any other spelling is `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.word() == word)
    }

    /// The ONE tier below: max to xhigh, xhigh to high, nothing else.
    #[must_use]
    pub const fn step_down(self) -> Option<Self> {
        match self {
            Self::Max => Some(Self::Xhigh),
            Self::Xhigh => Some(Self::High),
            _ => None,
        }
    }
}

/// Which way a transition moves effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the baseline to the tier below.
    Down,
    /// Back from that tier to the baseline.
    Up,
}

/// One effort move. Only max/xhigh and xhigh/high, each way, exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// Which way it moves.
    pub direction: Direction,
    /// The level it leaves.
    pub from: Level,
    /// The level it reaches.
    pub to: Level,
}

impl Transition {
    /// The step down from `base`, when `base` has one.
    #[must_use]
    pub fn down(base: Level) -> Option<Self> {
        base.step_down().map(|to| Self {
            direction: Direction::Down,
            from: base,
            to,
        })
    }

    /// The exact inverse of [`Self::down`]: back up to `base`.
    #[must_use]
    pub fn up(base: Level) -> Option<Self> {
        base.step_down().map(|from| Self {
            direction: Direction::Up,
            from,
            to: base,
        })
    }
}

/// A seat's baseline profile and the configured profiles one tier below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Twin {
    /// The level the seat's own profile runs.
    pub baseline: Level,
    /// The one tier below.
    pub target: Level,
    /// Every profile at `target` differing from the baseline in its effort
    /// value alone, in config order.
    pub aliases: Vec<String>,
}

/// Why a seat has no effort twin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwinRefusal {
    /// The harness has no effort setting ae reads.
    Unsupported,
    /// The seat's profile does not resolve to a command on this machine.
    Unresolved,
    /// The baseline command proves no single effort level.
    Flag(EffortRefusal),
    /// The baseline command pins no single model, so a twin cannot be proven
    /// to run the same one.
    Model,
    /// The baseline already sits at or below the floor.
    Floor(Level),
    /// The baseline level has no tier ae steps from.
    Level(Level),
    /// No profile runs the tier below with everything else equal.
    NoTwin(Level),
}

impl TwinRefusal {
    /// The stable word a journal record names this refusal by.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unresolved => "unresolved",
            Self::Flag(why) => why.word(),
            Self::Model => "model-unproven",
            Self::Floor(_) => "floor",
            Self::Level(_) => "unsupported-level",
            Self::NoTwin(_) => "no-twin",
        }
    }
}

/// The effort twin of `profile` for a seat recorded with `client` on `tool`.
///
/// Every profile is resolved through the SAME seat resolution `_run` uses,
/// under the seat's own client override, so a twin never crosses an account.
/// A candidate qualifies only when its command equals the baseline's in every
/// word but the effort value, and that value is exactly one tier lower: a
/// missing xhigh twin is never skipped to high.
///
/// # Errors
///
/// The first [`TwinRefusal`] met, in declaration order.
pub fn twin_in(
    cfg: &IdentityConfig,
    home: Option<&Path>,
    profile: &str,
    client: &RecordedClient,
    tool: ToolKind,
) -> Result<Twin, TwinRefusal> {
    if !tool.adapter().effort.supported() {
        return Err(TwinRefusal::Unsupported);
    }
    let resolve = |name: &str| crate::run::read_seat_command(cfg, name, "", "", client, home).ok();
    let baseline = resolve(profile).ok_or(TwinRefusal::Unresolved)?;
    let (skeleton, level) = effort_skeleton(baseline.as_str(), tool).map_err(TwinRefusal::Flag)?;
    if model_flag_value(baseline.as_str(), tool).is_none_or(|model| model.is_empty()) {
        return Err(TwinRefusal::Model);
    }
    let Some(target) = level.step_down() else {
        return Err(if level > Level::Max {
            TwinRefusal::Level(level)
        } else {
            TwinRefusal::Floor(level)
        });
    };
    let mut aliases: Vec<String> = Vec::new();
    for (name, _) in &cfg.profiles {
        let candidate = resolve(name).and_then(|cmd| effort_skeleton(cmd.as_str(), tool).ok());
        if candidate.is_some_and(|(words, at)| at == target && words == skeleton)
            && !aliases.contains(name)
        {
            aliases.push(name.clone());
        }
    }
    if aliases.is_empty() {
        return Err(TwinRefusal::NoTwin(target));
    }
    Ok(Twin {
        baseline: level,
        target,
        aliases,
    })
}

/// The most windows one record carries.
pub const MAX_WINDOWS: usize = 8;

/// One quota window of the seat's own scope, judged by the quota module.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// Its identity in the scope.
    pub key: WindowKey,
    /// The derived percentage quota judges it by.
    pub judged: f64,
    /// The reading still stands: fresh, or stale and not yet reset.
    pub usable: bool,
    /// When the vendor observed it.
    pub observed_at: i64,
    /// When it resets, where the vendor says.
    pub resets_at: Option<i64>,
}

impl Window {
    /// Whether this window decides effort: usable and proven long.
    #[must_use]
    pub fn long(&self) -> bool {
        self.usable && proven_long(self.key.minutes)
    }
}

/// The seat's own scope as effort reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    /// The windows that bind the seat's model family.
    pub windows: Vec<Window>,
    /// A usable row of the scope reports a reached spend cap.
    pub spend_capped: bool,
}

/// [`crate::autoreseat::leg::own_scope`] in effort's terms.
pub(crate) fn own_scope(
    quota: Option<&crate::quota::Observation>,
    entry: &crate::meta::RosterEntry,
    pin: Option<&str>,
    now: i64,
) -> Option<Scope> {
    let scope = crate::autoreseat::leg::own_scope(quota, entry, pin, now)?;
    let windows = scope.windows.into_iter().map(|(key, window)| Window {
        key,
        judged: window.judged,
        usable: matches!(window.status, Status::Fresh | Status::Stale),
        observed_at: window.observed_at,
        resets_at: window.resets_at,
    });
    Some(Scope {
        windows: windows.collect(),
        spend_capped: scope.spend_capped,
    })
}

/// One long window an episode was opened on, as its record keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pressured {
    /// Its identity in the scope.
    pub key: WindowKey,
    /// When the reading that pressed it was observed.
    pub observed_at: i64,
    /// When it resets, where the vendor said.
    pub resets_at: Option<i64>,
}

/// Whether a scope opens an episode at threshold `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// The account reports a reached spend cap: effort moves neither way.
    SpendCapped,
    /// No usable long window: nothing to judge by.
    Unread,
    /// Every long window reads under the threshold.
    Below,
    /// Every long window at or past it, in key order.
    Crossed(Vec<Pressured>),
    /// The pressed set cannot be recorded whole.
    Unrecordable,
}

/// What a recorded episode's windows say now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// The account reports a reached spend cap.
    SpendCapped,
    /// Some recorded window is not relieved, or a long window reads at or past
    /// the threshold.
    Hold,
    /// Every recorded window is relieved and none vetoes.
    Relieved,
}

/// Whether `scope` opens an episode at threshold `at`.
#[must_use]
pub fn entry(scope: &Scope, at: u8) -> Entry {
    if scope.spend_capped {
        return Entry::SpendCapped;
    }
    let long = scope.windows.iter().filter(|window| window.long());
    let mut pressured: Vec<Pressured> = long
        .clone()
        .filter(|window| window.judged >= f64::from(at))
        .map(|window| Pressured {
            key: window.key.clone(),
            observed_at: window.observed_at,
            resets_at: window.resets_at,
        })
        .collect();
    if long.count() == 0 {
        return Entry::Unread;
    }
    if pressured.is_empty() {
        return Entry::Below;
    }
    pressured.sort_by(|left, right| left.key.cmp(&right.key));
    if pressured.len() > MAX_WINDOWS
        || pressured.windows(2).any(|pair| pair[0].key == pair[1].key)
        || pressured.iter().any(|window| window_word(window).is_none())
    {
        return Entry::Unrecordable;
    }
    Entry::Crossed(pressured)
}

/// Whether the `recorded` windows of an applied episode at `threshold` are
/// relieved. A recorded window clears on a later-or-equal reading of the same
/// window under the threshold by [`REARM_GAP`], or once its recorded reset
/// has passed; any long window at or past the threshold vetoes.
#[must_use]
pub fn recovery(recorded: &[Pressured], threshold: u8, scope: &Scope, now: i64) -> Recovery {
    if scope.spend_capped {
        return Recovery::SpendCapped;
    }
    let long = scope.windows.iter().filter(|window| window.long());
    if long
        .clone()
        .any(|window| window.judged >= f64::from(threshold))
    {
        return Recovery::Hold;
    }
    let cleared = |pressed: &Pressured| {
        pressed.resets_at.is_some_and(|reset| reset <= now)
            || long.clone().any(|window| {
                window.key == pressed.key
                    && window.observed_at >= pressed.observed_at
                    && window.judged < f64::from(threshold) - REARM_GAP
            })
    };
    if recorded.iter().all(cleared) {
        Recovery::Relieved
    } else {
        Recovery::Hold
    }
}

/// Whether `scope` has ROOM under `threshold`, as auto reseat's own
/// [`crate::autoreseat::room`] judges it over the long windows alone. A spend
/// cap never has room; no long window is unread.
#[must_use]
pub fn relieved(scope: &Scope, threshold: u8) -> Option<bool> {
    if scope.spend_capped {
        return Some(false);
    }
    // Only usable windows pass `long`, and room reads fresh and stale alike.
    let long: Vec<crate::autoreseat::Window> = scope
        .windows
        .iter()
        .filter(|window| window.long())
        .map(|window| crate::autoreseat::Window {
            judged: window.judged,
            critical: false,
            long: true,
            observed_at: window.observed_at,
            resets_at: window.resets_at,
            status: Status::Fresh,
            label: String::new(),
        })
        .collect();
    match crate::autoreseat::room(&long, threshold) {
        Room::Unread => None,
        Room::Relieved(_) => Some(true),
        Room::Crossed(_) | Room::Near => Some(false),
    }
}

/// The longest encoded text field a record carries.
const TEXT_MAX: usize = 64;

/// The only hex digits a record spells a byte with.
const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// A record's spelling of free text: every byte outside `[A-Za-z0-9._-]` as
/// uppercase `%XX`, at most [`TEXT_MAX`] bytes once encoded. Empty text and a
/// control character have no spelling.
fn encode_text(text: &str) -> Option<String> {
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"._-".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    (encoded.len() <= TEXT_MAX).then_some(encoded)
}

/// One recorded window as its record spells it:
/// `<bucket>/<qualifier|~>/<minutes>/<observed>/<reset|~>`.
fn window_word(window: &Pressured) -> Option<String> {
    let minutes = window
        .key
        .minutes
        .filter(|minutes| *minutes >= LONG_MINUTES)?;
    let qualifier = match &window.key.qualifier {
        Some(qualifier) => encode_text(qualifier)?,
        None => "~".to_owned(),
    };
    let reset = match window.resets_at {
        Some(reset) if reset > 0 => reset.to_string(),
        Some(_) => return None,
        None => "~".to_owned(),
    };
    (window.observed_at > 0).then(|| {
        Some(format!(
            "{}/{qualifier}/{minutes}/{}/{reset}",
            encode_text(&window.key.bucket)?,
            window.observed_at
        ))
    })?
}

/// The longest `effort.<slot>` value.
pub const RECORD_MAX: usize = 4096;

/// Where an episode's one-tier move stands, as the seat's meta records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The step down is reserved and not yet proven.
    PendingDown,
    /// The seat runs the tier below.
    Applied,
    /// The step back up is reserved and not yet proven.
    PendingUp,
    /// The step down is reserved AND its restart authorized: a resume of the
    /// same conversation replays it when the journal proves the reservation.
    RestartDown,
    /// The step back up, likewise.
    RestartUp,
}

impl State {
    const ALL: [Self; 5] = [
        Self::PendingDown,
        Self::Applied,
        Self::PendingUp,
        Self::RestartDown,
        Self::RestartUp,
    ];

    /// The state's one spelling.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::PendingDown => "pending-down",
            Self::Applied => "applied",
            Self::PendingUp => "pending-up",
            Self::RestartDown => "restart-down",
            Self::RestartUp => "restart-up",
        }
    }
}

/// One seat's `effort.<slot>` meta record: the episode, the seat identity it
/// was written for, and the windows it was opened on. No command, path,
/// credential or quota source is ever kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Where the move stands.
    pub state: State,
    /// The roster slot, equal to the key's.
    pub slot: String,
    /// The seat's baseline level.
    pub base: Level,
    /// The tier below it.
    pub low: Level,
    /// The threshold the episode opened at.
    pub threshold: u8,
    /// The episode key: the epoch of its opening record.
    pub episode: i64,
    /// The meta `session_id`.
    pub session_id: String,
    /// The agent name.
    pub agent: String,
    /// The harness.
    pub tool: ToolKind,
    /// The launch the move was attempted under.
    pub launch: String,
    /// The tool process the move was attempted under.
    pub pid: u32,
    /// The conversation.
    pub conversation: String,
    /// The baseline profile.
    pub profile: String,
    /// The recorded client override, if any.
    pub client: Option<String>,
    /// The model the baseline pins.
    pub model: String,
    /// The windows the episode opened on, in key order.
    pub windows: Vec<Pressured>,
}

/// Why a stored record cannot be trusted. Damage is never a new episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    /// The key appears more than once.
    Duplicated,
    /// The key appears with no value.
    Bare,
    /// The value is not ASCII.
    Encoding,
    /// The value is not version `v1`.
    Version,
    /// The tokens are not the record's.
    Shape,
    /// This field is outside its grammar.
    Field(&'static str),
    /// The levels are not one valid step.
    Incoherent,
}

/// A seat's record as its meta stores it.
#[allow(
    clippy::large_enum_variant,
    reason = "one record per seat per read; the contract matches `Present(record)` by value"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    /// No record.
    Absent,
    /// A record ae cannot trust.
    Damaged(Damage),
    /// One well-formed record.
    Present(Record),
}

/// A canonical unsigned decimal: digits only, no leading zero.
fn decimal<T: std::str::FromStr>(text: &str) -> Option<T> {
    let canonical = text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.is_empty() && !text.starts_with('0'));
    canonical.then(|| text.parse().ok()).flatten()
}

/// A positive epoch in canonical decimal.
fn epoch(text: &str) -> Option<i64> {
    decimal(text).filter(|value: &i64| *value > 0)
}

/// The text a canonical [`encode_text`] spelling names.
fn decode_text(text: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        if byte == b'%' {
            let mut digit = || {
                rest.next()
                    .and_then(|d| HEX.iter().position(|hex| *hex == d))
            };
            let value = digit()? * 16 + digit()?;
            bytes.push(u8::try_from(value).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    let decoded = String::from_utf8(bytes).ok()?;
    (encode_text(&decoded)? == text).then_some(decoded)
}

/// One recorded window from its [`window_word`] spelling.
fn parse_window(word: &str) -> Option<Pressured> {
    let mut parts = word.split('/');
    let mut part = || parts.next();
    let bucket = decode_text(part()?)?;
    let qualifier = match part()? {
        "~" => None,
        text => Some(decode_text(text)?),
    };
    let minutes = decimal::<u32>(part()?).filter(|minutes| *minutes >= LONG_MINUTES)?;
    let observed_at = epoch(part()?)?;
    let resets_at = match part()? {
        "~" => None,
        text => Some(epoch(text)?),
    };
    part().is_none().then_some(Pressured {
        key: WindowKey {
            bucket,
            qualifier,
            minutes: Some(minutes),
        },
        observed_at,
        resets_at,
    })
}

/// A record's windows: one to [`MAX_WINDOWS`], no key twice.
fn parse_windows(text: &str) -> Option<Vec<Pressured>> {
    let windows: Vec<Pressured> = text.split(',').map(parse_window).collect::<Option<_>>()?;
    let unique = windows
        .iter()
        .enumerate()
        .all(|(i, window)| windows[..i].iter().all(|other| other.key != window.key));
    (unique && windows.len() <= MAX_WINDOWS).then_some(windows)
}

/// The canonical UUID `text` is, exactly.
fn uuid(text: &str) -> Option<String> {
    (!text.is_empty() && crate::archive::canonical_uuid(text) == text).then(|| text.to_owned())
}

/// `text`, kept when `valid`.
fn owned(text: &str, valid: bool) -> Option<String> {
    valid.then(|| text.to_owned())
}

/// The next token, which must be `<name>:<value>`, its value read by `parse`.
fn take<'v, T>(
    tokens: &mut std::str::Split<'v, char>,
    name: &'static str,
    parse: impl FnOnce(&'v str) -> Option<T>,
) -> Result<T, Damage> {
    match tokens.next().and_then(|token| token.split_once(':')) {
        Some((found, text)) if found == name => parse(text).ok_or(Damage::Field(name)),
        _ => Err(Damage::Shape),
    }
}

impl Record {
    /// The record a `v1` value spells, refusing anything else.
    ///
    /// # Errors
    ///
    /// The [`Damage`] met first, field by field in the record's order.
    pub fn parse(value: &str) -> Result<Self, Damage> {
        if value.len() > RECORD_MAX {
            return Err(Damage::Shape);
        }
        if !value.is_ascii() {
            return Err(Damage::Encoding);
        }
        let mut tokens = value.split(' ');
        if tokens.next() != Some("v1") {
            return Err(Damage::Version);
        }
        let state = tokens.next().ok_or(Damage::Shape)?;
        let t = &mut tokens;
        let record = Self {
            state: State::ALL
                .into_iter()
                .find(|known| known.word() == state)
                .ok_or(Damage::Field("state"))?,
            slot: take(t, "slot", |slot| {
                owned(slot, crate::requests::is_slot(slot) && slot.len() <= 32)
            })?,
            base: take(t, "base", Level::parse)?,
            low: take(t, "low", Level::parse)?,
            threshold: take(t, "at", |at| {
                decimal(at).filter(|at| (50..=100).contains(at))
            })?,
            episode: take(t, "ep", epoch)?,
            session_id: take(t, "sid", uuid)?,
            agent: take(t, "agent", |agent| {
                owned(agent, crate::config::is_agent_name(agent))
            })?,
            tool: take(t, "tool", |name| {
                let tool = ToolKind::from_binary_name(name);
                (tool.adapter().effort.supported() && tool.adapter().name == name).then_some(tool)
            })?,
            launch: take(t, "launch", |launch| {
                let bytes = launch.bytes();
                let spelled = bytes
                    .into_iter()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b));
                owned(launch, spelled && (1..=64).contains(&launch.len()))
            })?,
            pid: take(t, "pid", |pid| decimal(pid).filter(|pid| *pid > 0))?,
            conversation: take(t, "conv", uuid)?,
            profile: take(t, "profile", |profile| {
                owned(
                    profile,
                    crate::config::is_config_key(profile) && profile.len() <= 64,
                )
            })?,
            client: take(t, "client", |client| match client {
                "-" => Some(None),
                label => owned(label, crate::config::is_agent_name(label)).map(Some),
            })?,
            model: take(t, "model", |model| {
                owned(
                    model,
                    crate::launch_cmd::is_bare_flag_value(model) && model.len() <= 256,
                )
            })?,
            windows: take(t, "win", parse_windows)?,
        };
        if tokens.next().is_some() {
            return Err(Damage::Shape);
        }
        if record.base.step_down() != Some(record.low) {
            return Err(Damage::Incoherent);
        }
        Ok(record)
    }

    /// The record's one spelling. A record [`Self::parse`] would refuse spells
    /// a value it refuses too, so a writer proves `parse(encode(r)) == r`.
    #[must_use]
    pub fn encode(&self) -> String {
        let windows: Vec<String> = self
            .windows
            .iter()
            .map(|window| window_word(window).unwrap_or_default())
            .collect();
        format!(
            "v1 {} slot:{} base:{} low:{} at:{} ep:{} sid:{} agent:{} tool:{} launch:{} pid:{} \
             conv:{} profile:{} client:{} model:{} win:{}",
            self.state.word(),
            self.slot,
            self.base.word(),
            self.low.word(),
            self.threshold,
            self.episode,
            self.session_id,
            self.agent,
            self.tool.adapter().name,
            self.launch,
            self.pid,
            self.conversation,
            self.profile,
            self.client.as_deref().unwrap_or("-"),
            self.model,
            windows.join(",")
        )
    }

    /// The move a pending or restart state reserved; `None` once applied.
    #[must_use]
    pub fn pending(&self) -> Option<Transition> {
        match self.state {
            State::PendingDown | State::RestartDown => Transition::down(self.base),
            State::PendingUp | State::RestartUp => Transition::up(self.base),
            State::Applied => None,
        }
    }
}

/// The meta key family every effort record lives in: `effort.<slot>`.
pub const META_PREFIX: &str = "effort.";

/// The `effort.<slot>` record in a session meta's bytes. A line
/// `effort.<slot>` alone or `effort.<slot>=…` is an occurrence; any other key,
/// `effort.<slot>x=…` included, is not.
#[must_use]
pub fn stored(meta: &[u8], slot: &str) -> Stored {
    let key = format!("{META_PREFIX}{slot}");
    let mut found = Vec::new();
    for line in meta.split(|byte| *byte == b'\n') {
        if line == key.as_bytes() {
            found.push(None);
        } else if let Some(value) = line
            .strip_prefix(key.as_bytes())
            .and_then(|rest| rest.strip_prefix(b"="))
        {
            found.push(Some(value));
        }
    }
    let value = match found[..] {
        [] => return Stored::Absent,
        [None] => return Stored::Damaged(Damage::Bare),
        [Some(value)] => value,
        _ => return Stored::Damaged(Damage::Duplicated),
    };
    let Ok(value) = std::str::from_utf8(value) else {
        return Stored::Damaged(Damage::Encoding);
    };
    match Record::parse(value) {
        Ok(record) if record.slot == slot => Stored::Present(record),
        Ok(_) => Stored::Damaged(Damage::Field("slot")),
        Err(damage) => Stored::Damaged(damage),
    }
}

/// The seat as it stands now, for [`guard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatNow<'a> {
    /// The meta `session_id`.
    pub session_id: &'a str,
    /// The roster slot.
    pub slot: &'a str,
    /// The agent name.
    pub agent: &'a str,
    /// The recorded harness.
    pub tool: ToolKind,
    /// The recorded conversation, when one is proven.
    pub conversation: Option<&'a str>,
    /// The recorded profile.
    pub profile: &'a str,
    /// The recorded client override.
    pub client: &'a RecordedClient,
    /// The model the profile pins, when it pins one.
    pub model: Option<&'a str>,
    /// The profile's effort level, when it proves one.
    pub baseline: Option<Level>,
}

/// Which identity a record does not share with the seat now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foreign {
    /// Another session.
    Session,
    /// Another slot.
    Slot,
    /// Another agent.
    Agent,
    /// Another harness.
    Tool,
    /// Another, or no proven, conversation.
    Conversation,
    /// Another profile.
    Profile,
    /// Another, or an unusable, client override.
    Client,
    /// Another, or no proven, model.
    Model,
    /// Another, or no proven, baseline level.
    Baseline,
}

/// Whether `record` belongs to the seat `now`, checked in [`Foreign`]'s
/// order. The launch and pid are attempt provenance and are not compared; the
/// session NAME is never stored, so a rename keeps the record.
///
/// # Errors
///
/// The first identity that differs or is unproven.
pub fn guard(record: &Record, now: &SeatNow<'_>) -> Result<(), Foreign> {
    let client = match (&record.client, now.client) {
        (None, RecordedClient::Missing) => true,
        (Some(recorded), RecordedClient::Label(label)) => recorded == label,
        _ => false,
    };
    for (same, foreign) in [
        (record.session_id == now.session_id, Foreign::Session),
        (record.slot == now.slot, Foreign::Slot),
        (record.agent == now.agent, Foreign::Agent),
        (record.tool == now.tool, Foreign::Tool),
        (
            now.conversation == Some(record.conversation.as_str()),
            Foreign::Conversation,
        ),
        (record.profile == now.profile, Foreign::Profile),
        (client, Foreign::Client),
        (now.model == Some(record.model.as_str()), Foreign::Model),
        (now.baseline == Some(record.base), Foreign::Baseline),
    ] {
        if !same {
            return Err(foreign);
        }
    }
    Ok(())
}

/// An episode opens: ref `<ep>`, summary `threshold <at>`. `<ep>` is the
/// record's own epoch.
pub const EPISODE: &str = "effort-episode";
/// A move is reserved: ref `<ep>-<down|up>`, summary `<from>-><to>`.
pub const RESERVED: &str = "effort-reserved";
/// The step down is proven: ref `<ep>-down`, summary `<from>-><to>`.
pub const APPLIED: &str = "effort-applied";
/// The step back up is proven, closing the episode: ref `<ep>-up`.
pub const RESTORED: &str = "effort-restored";
/// A reserved move is proven to have failed: ref `<ep>-<down|up>`.
pub const FAILED: &str = "effort-failed";
/// A reason named once per episode: ref `<ep>`, summary the reason word.
pub const HELD: &str = "effort-held";
/// The human owns this episode now: ref `<ep>`, summary
/// `manual-effort` or `baseline-changed`.
pub const YIELDED: &str = "effort-yielded";
/// The episode ends unmoved or yielded: ref `<ep>`.
pub const REARMED: &str = "effort-rearmed";
/// A resume dropped the seat's record and everything the journal remembers of
/// its episodes: ref `<ep>` or empty, summary `off` or `fresh`.
pub const DISCARDED: &str = "effort-discarded";

/// Where one direction of an episode stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    /// Never reserved.
    Unspent,
    /// Reserved and not yet proven either way: the reservation alone spends it.
    Reserved(Transition),
    /// Proven moved.
    Done(Transition),
    /// Proven not moved.
    Failed(Transition),
}

/// One open episode, as the journal folds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Episode {
    /// The episode key: the epoch of its opening record.
    pub key: i64,
    /// The threshold it opened at, kept whatever the config says later.
    pub threshold: u8,
    /// The step down.
    pub down: Leg,
    /// The step back up.
    pub up: Leg,
    /// The reasons already named, each once.
    pub named: Vec<String>,
    /// The human owns this episode.
    pub yielded: bool,
}

/// What a seat's journal says about effort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fold {
    /// No episode is open.
    Armed,
    /// One is.
    Open(Episode),
    /// The trace is one ae would not have written: nothing may move.
    Doubt(&'static str),
}

/// The reasons a [`HELD`] record may name.
pub(crate) const REASONS: [&str; 17] = [
    "background-task",
    "model-drift",
    "unresumable",
    "spend-cap",
    "unrecordable",
    "unsupported",
    "unresolved",
    "no-effort-flag",
    "ambiguous-effort",
    "effort-missing-value",
    "unknown-effort",
    "env-effort",
    "not-simple",
    "model-unproven",
    "floor",
    "unsupported-level",
    "no-twin",
];

/// The transition a `<from>-><to>` summary names in `direction`.
fn pair(summary: &str, direction: Direction) -> Option<Transition> {
    let (from, to) = summary.split_once("->")?;
    let (from, to) = (Level::parse(from)?, Level::parse(to)?);
    let named = match direction {
        Direction::Down => Transition::down(from)?,
        Direction::Up => Transition::up(to)?,
    };
    (named.from == from && named.to == to).then_some(named)
}

/// The leg an outcome record moves, and the leg it leaves behind.
fn outcome(action: &str, direction: Direction, t: Transition) -> Option<Leg> {
    match (action, direction) {
        (APPLIED, Direction::Down) | (RESTORED, Direction::Up) => Some(Leg::Done(t)),
        (FAILED, _) => Some(Leg::Failed(t)),
        _ => None,
    }
}

/// The grammar of a record that names no direction: a hold names a known
/// reason, a yield one of its two causes, and nothing else is an action.
fn undirected(action: &str, summary: &str) -> Result<(), &'static str> {
    match action {
        HELD if REASONS.contains(&summary) => Ok(()),
        HELD => Err("unknown effort reason"),
        YIELDED if matches!(summary, "manual-effort" | "baseline-changed") => Ok(()),
        YIELDED => Err("unknown effort yield"),
        REARMED => Ok(()),
        _ => Err("unknown effort action"),
    }
}

/// The episode an [`EPISODE`] record opens, when `fresh`: keyed by its own
/// epoch, with no episode open and none closed under that key.
fn opened(key: i64, summary: &str, fresh: bool) -> Result<Episode, &'static str> {
    let threshold = summary
        .strip_prefix("threshold ")
        .and_then(decimal)
        .filter(|at| (50..=100).contains(at))
        .ok_or("unreadable effort threshold")?;
    if !fresh {
        return Err("effort episode opened out of order");
    }
    Ok(Episode {
        key,
        threshold,
        down: Leg::Unspent,
        up: Leg::Unspent,
        named: Vec::new(),
        yielded: false,
    })
}

/// One effort record against the open episode and the closed ones.
fn step(
    open: &mut Option<Episode>,
    closed: &mut Vec<Episode>,
    event: &Event,
) -> Result<(), &'static str> {
    let reference = event.reference.as_deref().unwrap_or_default();
    let summary = event.summary.as_deref().unwrap_or_default();
    let summary = summary.split_once("; ").map_or(summary, |(head, _)| head);
    let action = event.action.as_str();
    let (key, direction) = match reference.split_once('-') {
        Some((key, "down")) => (key, Some(Direction::Down)),
        Some((key, "up")) => (key, Some(Direction::Up)),
        Some(_) => return Err("unreadable effort ref"),
        None => (reference, None),
    };
    let key = epoch(key).ok_or("unreadable effort ref")?;
    let directed = matches!(action, RESERVED | APPLIED | RESTORED | FAILED);
    if directed != direction.is_some() {
        return Err("effort ref does not fit its action");
    }
    if action == EPISODE {
        let fresh =
            key == event.ts.epoch() && open.is_none() && closed.iter().all(|e| e.key != key);
        *open = Some(opened(key, summary, fresh)?);
        return Ok(());
    }
    let moved = match direction {
        Some(direction) => Some((
            direction,
            pair(summary, direction).ok_or("unreadable effort pair")?,
        )),
        None => None,
    };
    if let Some(done) = closed.iter().find(|e| e.key == key) {
        let Some((direction, t)) = moved else {
            return undirected(action, summary);
        };
        let leg = if direction == Direction::Down {
            done.down
        } else {
            done.up
        };
        return (outcome(action, direction, t) == Some(leg))
            .then_some(())
            .ok_or("effort record conflicts with a closed episode");
    }
    let Some(episode) = open.as_mut().filter(|e| e.key == key) else {
        return Err("effort record names no open episode");
    };
    if let Some((direction, t)) = moved {
        let inverse = match episode.down {
            Leg::Done(d) => d.from == t.to && d.to == t.from,
            _ => false,
        };
        let (leg, ready) = match direction {
            Direction::Down => (&mut episode.down, true),
            Direction::Up => (&mut episode.up, inverse),
        };
        *leg = match (action, *leg) {
            (RESERVED, Leg::Unspent) if ready => Leg::Reserved(t),
            (_, Leg::Reserved(r)) if r == t && action != RESERVED => {
                outcome(action, direction, t).ok_or("effort outcome does not fit its leg")?
            }
            (_, done) if Some(done) == outcome(action, direction, t) => done,
            _ => return Err("effort record does not fit its leg"),
        };
        if episode.up != Leg::Done(t) {
            return Ok(());
        }
    } else if let Err(why) = undirected(action, summary) {
        return Err(why);
    } else if action == HELD {
        if !episode.named.iter().any(|named| named == summary) {
            episode.named.push(summary.to_owned());
        }
        return Ok(());
    } else if action == YIELDED {
        episode.yielded = true;
        return Ok(());
    } else if matches!(episode.down, Leg::Reserved(_))
        || matches!(episode.up, Leg::Reserved(_) | Leg::Failed(_))
        || matches!(episode.down, Leg::Done(_)) && !episode.yielded
    {
        return Err("effort episode re-armed while it owns a move");
    }
    closed.extend(open.take());
    Ok(())
}

/// Whether a watchdog [`DISCARDED`] record is one ae writes: ref empty or a
/// canonical epoch, summary exactly a [`Discarded`] word.
fn well_formed_discard(event: &Event) -> bool {
    let reference = event.reference.as_deref().unwrap_or_default();
    let summary = event.summary.as_deref().unwrap_or_default();
    (reference.is_empty() || epoch(reference).is_some())
        && [Discarded::Off, Discarded::Fresh]
            .iter()
            .any(|why| why.word() == summary)
}

/// Fold the seat's effort records, in journal order. Only records addressed
/// to the seat count, effort records only from the watchdog; a `spawn` or
/// `reseat` of the seat, or a well-formed [`DISCARDED`], starts again with
/// nothing remembered, doubt included.
#[must_use]
pub fn fold(events: &[Event], session: SessionKey<'_>, slot: &str, agent: &str) -> Fold {
    let mut open = None;
    let mut closed = Vec::new();
    let mut doubt = None;
    for event in events {
        if !crate::watchdog::event_is_addressed_to_in(event, session, slot, agent) {
            continue;
        }
        let discard = event.actor == "watchdog" && event.action == DISCARDED;
        if matches!(event.action.as_str(), "spawn" | "reseat")
            || discard && well_formed_discard(event)
        {
            open = None;
            closed.clear();
            doubt = None;
        } else if discard {
            doubt = Some("unreadable effort discard");
        } else if doubt.is_none()
            && event.actor == "watchdog"
            && event.action.starts_with("effort-")
            && let Err(why) = step(&mut open, &mut closed, event)
        {
            doubt = Some(why);
        }
    }
    match doubt {
        Some(why) => Fold::Doubt(why),
        None => open.map_or(Fold::Armed, Fold::Open),
    }
}

/// Everything one decision reads, already read.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The configured threshold a NEW episode opens at.
    pub at: u8,
    /// The config lets a new step down be attempted.
    pub new_attempts: bool,
    /// The seat's own scope, when it was read.
    pub scope: Option<&'a Scope>,
    /// The seat is on a drawn vendor usage limit.
    pub limited: bool,
    /// The journal fold.
    pub fold: &'a Fold,
    /// The meta record.
    pub stored: &'a Stored,
    /// The record's identity against the seat now; `None` when unread.
    pub guard: Option<Result<(), Foreign>>,
    /// The seat's twin.
    pub twin: &'a Result<Twin, TwinRefusal>,
    /// The effort the seat's pane draws, when read.
    pub live: Option<Level>,
    /// Now, in epoch seconds.
    pub now: i64,
}

/// Why nothing happens this time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The seat is on a vendor usage limit.
    Limit,
    /// The journal trace is in doubt.
    Doubt,
    /// The meta record is damaged.
    Damaged,
    /// Something needed was not read.
    Unread,
    /// The spend cap was already named.
    SpendCap,
    /// Under the threshold, not yet relieved.
    Near,
    /// Applied, and the recorded windows are not relieved.
    Pressure,
    /// A static reason was already named.
    Skipped,
    /// The step down failed; waiting for relief.
    Spent,
    /// The step back up failed; nothing more is tried.
    TerminalUp,
    /// The human owns the episode; waiting for relief.
    Yielded,
    /// The config allows no new step down.
    Disabled,
}

/// The ONE thing to do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do.
    Idle,
    /// Nothing to do, for this reason.
    Hold(Hold),
    /// Open an episode at this threshold.
    Open {
        /// The threshold, kept by the episode.
        threshold: u8,
    },
    /// Name this reason, once.
    Name(&'static str),
    /// Step down to the twin's level on these windows.
    Down {
        /// The twin.
        twin: Twin,
        /// The windows the record keeps.
        windows: Vec<Pressured>,
    },
    /// Step back up.
    Up,
    /// Settle this direction's reservation first.
    Reconcile(Direction),
    /// Hand the episode to the human, for this reason.
    Yield(&'static str),
    /// Remove the record of a yielded episode.
    Retire,
    /// End the episode.
    Rearm,
}

/// The ONE decision for a seat this cycle.
#[must_use]
pub fn decide(inputs: &Inputs<'_>) -> Decision {
    let present = match inputs.stored {
        Stored::Present(record) => Some(record),
        Stored::Absent | Stored::Damaged(_) => None,
    };
    let episode = match inputs.fold {
        _ if inputs.limited => return Decision::Hold(Hold::Limit),
        Fold::Doubt(_) => return Decision::Hold(Hold::Doubt),
        _ if matches!(inputs.stored, Stored::Damaged(_)) => return Decision::Hold(Hold::Damaged),
        Fold::Open(episode) => Some(episode),
        Fold::Armed => None,
    };
    if let Some(episode) = episode {
        if matches!(episode.down, Leg::Reserved(_)) {
            return Decision::Reconcile(Direction::Down);
        }
        if matches!(episode.up, Leg::Reserved(_)) {
            return Decision::Reconcile(Direction::Up);
        }
    }
    let Some(scope) = inputs.scope else {
        return Decision::Hold(Hold::Unread);
    };
    let Some(episode) = episode else {
        return match entry(scope, inputs.at) {
            _ if present.is_some() => Decision::Hold(Hold::Doubt),
            Entry::Unread => Decision::Hold(Hold::Unread),
            Entry::Below => Decision::Idle,
            _ => Decision::Open {
                threshold: inputs.at,
            },
        };
    };
    let named = |word: &str| episode.named.iter().any(|named| named == word);
    if scope.spend_capped {
        return if named("spend-cap") {
            Decision::Hold(Hold::SpendCap)
        } else {
            Decision::Name("spend-cap")
        };
    }
    let threshold = episode.threshold;
    let relief = |held: Hold| match relieved(scope, threshold) {
        None => Decision::Hold(Hold::Unread),
        Some(true) => Decision::Rearm,
        Some(false) => Decision::Hold(held),
    };
    if episode.yielded {
        return if present.is_some() {
            Decision::Retire
        } else {
            relief(Hold::Yielded)
        };
    }
    match episode.down {
        Leg::Unspent if present.is_some() => Decision::Hold(Hold::Doubt),
        Leg::Unspent => down(inputs, episode, scope, relief(Hold::Near)),
        Leg::Failed(_) => relief(Hold::Spent),
        Leg::Done(done) => match present {
            Some(record)
                if record.state == State::Applied
                    && record.episode == episode.key
                    && record.threshold == threshold
                    && (record.base, record.low) == (done.from, done.to) =>
            {
                up(inputs, episode, scope, record, done)
            }
            _ => Decision::Hold(Hold::Doubt),
        },
        Leg::Reserved(_) => Decision::Reconcile(Direction::Down),
    }
}

/// An open episode whose step down is unspent: `relief` is what the long
/// windows say when they are under the threshold.
fn down(inputs: &Inputs<'_>, episode: &Episode, scope: &Scope, relief: Decision) -> Decision {
    let crossed = entry(scope, episode.threshold);
    if !matches!(crossed, Entry::Crossed(_) | Entry::Unrecordable) {
        return relief;
    }
    let named = |word: &str| episode.named.iter().any(|named| named == word);
    if named("unrecordable") {
        return Decision::Hold(Hold::Skipped);
    }
    let twin = match inputs.twin {
        Err(refusal) if named(refusal.word()) => return Decision::Hold(Hold::Skipped),
        Err(refusal) => return Decision::Name(refusal.word()),
        Ok(twin) => twin,
    };
    if !inputs.new_attempts {
        return Decision::Hold(Hold::Disabled);
    }
    match inputs.live {
        None => return Decision::Hold(Hold::Unread),
        Some(live) if live != twin.baseline => return Decision::Yield("manual-effort"),
        Some(_) => {}
    }
    match crossed {
        Entry::Crossed(windows) => Decision::Down {
            twin: twin.clone(),
            windows,
        },
        _ => Decision::Name("unrecordable"),
    }
}

/// An open episode whose step down is done and agrees with its record.
fn up(
    inputs: &Inputs<'_>,
    episode: &Episode,
    scope: &Scope,
    record: &Record,
    done: Transition,
) -> Decision {
    match inputs.guard {
        None => return Decision::Hold(Hold::Unread),
        Some(Err(_)) => return Decision::Yield("baseline-changed"),
        Some(Ok(())) => {}
    }
    match inputs.live {
        None => return Decision::Hold(Hold::Unread),
        Some(live) if live != done.to => return Decision::Yield("manual-effort"),
        Some(_) => {}
    }
    match (
        recovery(&record.windows, episode.threshold, scope, inputs.now),
        episode.up,
    ) {
        (Recovery::Relieved, Leg::Unspent) => Decision::Up,
        (Recovery::Relieved, Leg::Failed(_)) => Decision::Hold(Hold::TerminalUp),
        (Recovery::Relieved, _) => Decision::Hold(Hold::Doubt),
        _ => Decision::Hold(Hold::Pressure),
    }
}

/// `[workspace] auto_effort`, as a resume reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// On.
    On,
    /// The kill switch: a resume drops the record.
    Off,
    /// Malformed or unreadable: neither replay nor drop.
    Unusable,
}

/// Why a resume drops a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discarded {
    /// `auto_effort = off`.
    Off,
    /// The seat does not resume the record's conversation.
    Fresh,
}

impl Discarded {
    /// The [`DISCARDED`] summary.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fresh => "fresh",
        }
    }
}

/// Why a resume leaves a record alone and launches the profile as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// The switch is unusable.
    Unusable,
    /// The record cannot be trusted.
    Damaged(Damage),
    /// The record belongs to another seat or setup.
    Foreign(Foreign),
    /// The human owns the episode.
    Yielded,
    /// The journal does not prove the move the record claims.
    Ledger,
}

/// What a resume does with the seat's effort record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// There is none.
    None,
    /// Launch the profile with its effort value rewritten to this level.
    Replay(Level),
    /// Launch the profile as written and keep the record.
    Keep(Kept),
    /// Drop the record, journaling why, and launch the profile as written.
    Discard {
        /// The record's episode, when it is readable.
        episode: Option<i64>,
        /// Why.
        why: Discarded,
    },
    /// An unresolved move: refuse the launch.
    Refuse(State),
    /// An authorized step down whose conversation is not the one this launch
    /// keeps: journal it failed, drop the record, launch the profile as written.
    Failed {
        /// The record's episode.
        episode: i64,
    },
}

/// The ONE resume decision. `retained` says the launch resumes the recorded
/// conversation, proven present; `ledger` is the seat's fold over a strictly
/// read journal, `None` when that read failed. The first rule that matches
/// decides: absent, not retained, off, unusable, damaged, another seat, another
/// conversation, unresolved, another setup, then the journal.
#[must_use]
pub fn on_resume(
    stored: &Stored,
    guard: Result<(), Foreign>,
    ledger: Option<&Fold>,
    switch: Switch,
    retained: bool,
) -> Resume {
    let record = match stored {
        Stored::Absent => return Resume::None,
        Stored::Damaged(_) => None,
        Stored::Present(record) => Some(record),
    };
    let episode = record.map(|record| record.episode);
    let discard = |why| Resume::Discard { episode, why };
    // A fresh conversation spends an authorized step down rather than
    // forgetting it, or the pressure that reserved it would restart again.
    let fresh = match record {
        Some(record) if record.state == State::RestartDown => Resume::Failed {
            episode: record.episode,
        },
        _ => discard(Discarded::Fresh),
    };
    if !retained {
        return fresh;
    }
    match (switch, stored, record) {
        (Switch::Off, ..) => return discard(Discarded::Off),
        (Switch::Unusable, ..) => return Resume::Keep(Kept::Unusable),
        (Switch::On, Stored::Damaged(damage), _) => return Resume::Keep(Kept::Damaged(*damage)),
        _ => {}
    }
    let Some(record) = record else {
        return Resume::None;
    };
    match guard {
        Err(Foreign::Conversation) => return fresh,
        Err(foreign @ (Foreign::Session | Foreign::Slot | Foreign::Agent | Foreign::Tool)) => {
            return Resume::Keep(Kept::Foreign(foreign));
        }
        Ok(()) if matches!(record.state, State::RestartDown | State::RestartUp) => {
            return authorized(record, ledger);
        }
        _ if record.state != State::Applied => return Resume::Refuse(record.state),
        Err(foreign) => return Resume::Keep(Kept::Foreign(foreign)),
        Ok(()) => {}
    }
    let Some(Fold::Open(open)) = ledger else {
        return Resume::Keep(Kept::Ledger);
    };
    if open.key != record.episode {
        return Resume::Keep(Kept::Ledger);
    }
    if open.yielded {
        return Resume::Keep(Kept::Yielded);
    }
    let down = Transition {
        direction: Direction::Down,
        from: record.base,
        to: record.low,
    };
    let moved = matches!(open.down, Leg::Reserved(t) | Leg::Done(t) if t == down);
    let unspent = matches!(open.up, Leg::Unspent | Leg::Reserved(_) | Leg::Failed(_));
    if open.threshold == record.threshold && moved && unspent {
        Resume::Replay(record.low)
    } else {
        Resume::Keep(Kept::Ledger)
    }
}

/// A restart record's replay: only THIS episode, at its own threshold, not
/// yielded, with exactly the record's move reserved and the other leg where
/// that move needs it. Anything else refuses, as a pending record does.
fn authorized(record: &Record, ledger: Option<&Fold>) -> Resume {
    let refuse = Resume::Refuse(record.state);
    let (Some(t), Some(Fold::Open(open))) = (record.pending(), ledger) else {
        return refuse;
    };
    let legs = match t.direction {
        Direction::Down => open.down == Leg::Reserved(t) && open.up == Leg::Unspent,
        Direction::Up => {
            Transition::down(record.base).is_some_and(|down| open.down == Leg::Done(down))
                && open.up == Leg::Reserved(t)
        }
    };
    if legs && open.key == record.episode && open.threshold == record.threshold && !open.yielded {
        Resume::Replay(t.to)
    } else {
        refuse
    }
}

/// The seat a [`DISCARDED`] record is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscardSeat<'a> {
    /// The session name.
    pub session: &'a str,
    /// The meta `session_id`.
    pub session_id: &'a str,
    /// The roster slot.
    pub slot: &'a str,
    /// The agent name.
    pub agent: &'a str,
}

/// The ONE [`DISCARDED`] journal line, as the watchdog, routed to the seat by
/// slot and session id so a rename keeps it addressed. No trailing newline.
#[must_use]
pub fn discard_line(
    now: Timestamp,
    seat: &DiscardSeat<'_>,
    episode: Option<i64>,
    why: Discarded,
) -> String {
    let reference = episode.map(|key| key.to_string()).unwrap_or_default();
    line(now, seat, DISCARDED, &reference, why.word())
}

/// The ONE effort journal line, as the watchdog, routed to the seat by slot and
/// session id so a rename keeps it addressed. No trailing newline.
#[must_use]
pub fn line(
    now: Timestamp,
    seat: &DiscardSeat<'_>,
    action: &str,
    reference: &str,
    summary: &str,
) -> String {
    let line = crate::tracked::event_line(&crate::tracked::EventFields {
        ts: now,
        actor: crate::watchdog::WATCHDOG_ACTOR,
        action,
        target: seat.agent,
        reference,
        actor_slot: "",
        actor_session: "",
        target_slot: seat.slot,
        target_session: seat.session,
        actor_session_id: "",
        target_session_id: seat.session_id,
        target_server: "",
        target_pane: "",
        target_session_uuid: "",
        caller_server: "",
        caller_pane: "",
        caller_session_uuid: "",
        identity_gap: "",
        summary,
        body_file: "",
    });
    line.trim_end_matches('\n').to_owned()
}
