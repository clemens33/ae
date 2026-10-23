//! Vendor quota snapshots parsed from local client-owned caches.
//!
//! The parsers under `quota/` are pure: hostile bytes in, rows out. This
//! parent module is not — it discovers and reads the client files (hostile
//! persisted state), runs the one on-demand agy leg for [`run`], folds the
//! rows and renders the operator table.

pub mod agy;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod feed;
pub mod go;
pub mod grok;
mod machine;
pub mod muse;
mod pace;
pub mod view;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::json::Value;
use crate::time::Timestamp;
use crate::tool::{QuotaSource, ToolKind};
use crate::transport::AgyRun;

const CLAUDE_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The statusline feed's record ceiling, as the store reads take it.
const FEED_CAP: u64 = feed::RECORD_MAX as u64;
const CODEX_TAIL_BYTES: u64 = 256 * 1024;
const SESSION_META_MAX_BYTES: u64 = 1024 * 1024;
const QUOTA_MAX_FILES: usize = 4_096;
const QUOTA_MAX_BYTES: u64 = 16 * 1024 * 1024;
const QUOTA_MAX_ELAPSED: Duration = Duration::from_secs(2);
const CODEX_DISPLAY_ROLLOUTS: usize = 3;
/// Where the grok CLI keeps its shared debug log, under its config home.
const GROK_LOG: &str = "logs/unified.jsonl";
/// Says why a full grok window may not block work.
const ON_DEMAND_NOTE: &str = "on-demand spend enabled: a full window may not block work";
/// Per-column display caps. Their sum plus two spaces between each pair of
/// columns is [`TABLE_MAX_LINE`], the table's documented width ceiling: a new
/// column is paid for here, in width, and the ceiling says what it cost.
const TABLE_MAX_WIDTHS: [usize; COLUMNS] = [40, 35, 22, 6, 5, 9, 9, 16, 9, 9, 20];

/// Columns in the operator table.
const COLUMNS: usize = 11;

/// The widest line the table can render, cells and separators together. The
/// caps above produce it; `docs/reference/commands.md` publishes it.
#[cfg(test)]
const TABLE_MAX_LINE: usize = 200;

/// Maximum age of an observation that may be called fresh.
pub const FRESH_SECS: i64 = 15 * 60;

/// Future clock skew at which an observation becomes unknown.
pub const FUTURE_SKEW_SECS: i64 = 5 * 60;

/// Whether a parsed quota row is safe to use as current evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Observed no more than 15 minutes ago.
    Fresh,
    /// Observed more than 15 minutes ago, before its reset.
    Stale,
    /// Missing, expired, skewed, or otherwise not applicable.
    Unknown,
    /// The client exposes no usable local quota state.
    Unsupported,
    /// A present source could not be read or parsed.
    ReadError,
    /// The invocation-wide file, byte, or wall-clock budget was exhausted.
    Truncated,
}

impl Status {
    /// Stable spelling used in the operator table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Unsupported => "unsupported",
            Self::ReadError => "read-error",
            Self::Truncated => "truncated",
        }
    }
}

/// Only a window PROVEN at least one day long decides effort and auto reseat
/// headroom: a shorter one resets before either pays for itself, and a length
/// the vendor never stated proves nothing.
pub const LONG_MINUTES: u32 = 1440;

/// Whether a window of `minutes` is proven at least [`LONG_MINUTES`] long.
#[must_use]
pub const fn proven_long(minutes: Option<u32>) -> bool {
    matches!(minutes, Some(minutes) if minutes >= LONG_MINUTES)
}

/// One window's identity inside its scope — bucket, qualifier and length, the
/// identity the settings dialog folds rows by — never a display label.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowKey {
    /// Vendor bucket identifier.
    pub bucket: String,
    /// Optional vendor qualifier.
    pub qualifier: Option<String>,
    /// Window duration in minutes, where the vendor states one.
    pub minutes: Option<u32>,
}

impl WindowKey {
    /// The identity of `row`.
    #[must_use]
    pub fn of(row: &Row) -> Self {
        Self {
            bucket: row.bucket.clone(),
            qualifier: row.qualifier.clone(),
            minutes: row.window_minutes,
        }
    }
}

/// One vendor bucket and one of its windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Vendor bucket identifier (`weekly_all`, `codex_bengalfox`, and so on).
    pub bucket: String,
    /// Optional vendor qualifier: Claude model scope or Codex plan.
    pub qualifier: Option<String>,
    /// Window duration reported or defined by the vendor, in minutes.
    pub window_minutes: Option<u32>,
    /// Vendor utilization numeric literal, without a percent sign.
    pub used_percent: Option<String>,
    /// Window reset as Unix epoch seconds.
    pub resets_at: Option<i64>,
    /// Cache or rollout observation as Unix epoch seconds.
    pub observed_at: Option<i64>,
    /// Freshness and applicability verdict for this row.
    pub status: Status,
}

/// Vendor credit state for one client scope, exactly as the client reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Credits {
    /// The client reports no credit state at all.
    #[default]
    Unreported,
    /// Credits are declared unlimited, so a window percentage does not bind.
    Unlimited,
    /// A spendable balance, kept as the vendor's own literal.
    Available(String),
    /// Credits are available in an amount the client did not state in a form
    /// ae can show exactly. A clipped literal is a different number, so the
    /// amount is withheld rather than misreported.
    AvailableUnknown,
    /// The client reports credits and has none.
    Exhausted,
}

/// Account-wide facts a client reports beside its windows.
///
/// They belong to the scope, not to one window: every window of one account
/// shares them, and a spend cap constrains work no window reset can free.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    /// Credit state, when the client reports one.
    pub credits: Credits,
    /// When the credit state was last USABLY reported.
    pub credits_observed_at: Option<i64>,
    /// Whether the account's own spend control is reached.
    pub spend_control_reached: Option<bool>,
    /// When the spend-control state was last USABLY reported.
    pub spend_observed_at: Option<i64>,
}

impl Account {
    /// The account is spend-capped: no window has headroom until that changes.
    ///
    /// Age alone never lifts this verdict — only a report that explicitly says
    /// the cap is gone does, for as long as the fact is held. It is not durable
    /// beyond that: a scope whose bounded tail no longer carries the record,
    /// and a daemon that restarts with no held state, both begin again from
    /// what the client reports now. While it IS held, keeping it can only
    /// understate headroom, and that is the error worth making.
    #[must_use]
    pub const fn spend_capped(&self) -> bool {
        matches!(self.spend_control_reached, Some(true))
    }

    /// Whether an unlimited-credit claim may relieve a window observed at
    /// `window`.
    ///
    /// Credits are the one account fact that ADDS apparent headroom, so the
    /// claim has to be evidence about the measurement it is changing: a claim
    /// older than that window says nothing about it, and an unstamped claim
    /// says nothing at all.
    #[must_use]
    pub fn credits_relieve(&self, window: Option<i64>) -> bool {
        self.credits == Credits::Unlimited
            && match (self.credits_observed_at, window) {
                (Some(credits), Some(window)) => credits >= window,
                _ => false,
            }
    }

    /// Adopt each fact `incoming` actually reports whose own stamp is newer
    /// than the one held for that fact, and nothing else.
    ///
    /// This is the ONE place an account fact moves. A report says nothing about
    /// the fields it does not carry, so absence never overwrites a held value
    /// and never refreshes its age; a report older than what is held loses,
    /// whichever order the two arrive in. The rule is the same within one read
    /// of a source and across two cycles of the watchdog, because both ask
    /// this function.
    pub(crate) fn absorb(&mut self, incoming: &Self) {
        if let Some(at) = incoming.credits_observed_at
            && self.credits_observed_at.is_none_or(|held| at > held)
        {
            self.credits = incoming.credits.clone();
            self.credits_observed_at = Some(at);
        }
        if let Some(at) = incoming.spend_observed_at
            && self.spend_observed_at.is_none_or(|held| at > held)
        {
            self.spend_control_reached = incoming.spend_control_reached;
            self.spend_observed_at = Some(at);
        }
    }
}

/// Everything ae judges a window BY: the operator's declaration for the scope,
/// and the account facts the client reported with their own provenance.
///
/// The two halves are one value deliberately. Outside this module a policy can
/// only be read from a group, cloned, or merged with [`Policy::absorb`] — no
/// caller can pair one scope's declaration with another observation's account,
/// and none can replace an account fact by assignment. That is what keeps the
/// field-level provenance of a single read identical to the rule that governs
/// what a later read may replace.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    manual_resets: Option<u8>,
    account: Account,
}

impl Policy {
    /// Bind a scope's declaration to the account facts read with it.
    ///
    /// Private, and that is the point: a policy is assembled where a scope is
    /// read and nowhere else, so no other module can pair one scope's
    /// declaration with another observation's facts. Elsewhere a policy exists
    /// only by being read from a [`Group`], cloned, or merged.
    const fn new(manual_resets: Option<u8>, account: Account) -> Self {
        Self {
            manual_resets,
            account,
        }
    }

    /// The same, for unit tests that build a scope by hand.
    ///
    /// Test builds only: a product caller of this does not compile, which is
    /// what keeps the constructor above the only one that exists.
    #[cfg(test)]
    pub(crate) const fn for_tests(manual_resets: Option<u8>, account: Account) -> Self {
        Self::new(manual_resets, account)
    }

    /// Merge `incoming` into this policy, and report whether anything moved.
    ///
    /// The DECLARATION is local configuration, re-read every cycle and carrying
    /// no vendor clock, so the incoming one replaces what was held. The ACCOUNT
    /// is vendor evidence, so it merges fact by fact on each stamp.
    pub(crate) fn absorb(&mut self, incoming: &Self) -> bool {
        let before = self.clone();
        self.manual_resets = incoming.manual_resets;
        self.account.absorb(&incoming.account);
        *self != before
    }

    /// The account is spend-capped, which no window reset can free.
    pub(crate) const fn spend_capped(&self) -> bool {
        self.account.spend_capped()
    }
}

/// Longest vendor balance literal ae can state exactly in its column. A longer
/// one is reported as available-without-an-amount, never clipped.
pub(crate) const CREDIT_BALANCE_MAX: usize = 9;

/// The percentage ae judges a window by, and how it got there.
///
/// The raw window percentage answers "how much of this window is gone", which
/// is not the operator's question when a manual reset is in hand or credits
/// carry work past the window. Only declared or reported facts move it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Effective {
    /// Nothing declared and nothing reported: the raw window is the story.
    Raw,
    /// The same usage spread over `1 + resets` windows of capacity.
    Resets { resets: u8, percent: f64 },
    /// Credits are unlimited, so the window does not constrain new work.
    Unlimited,
    /// The account's spend control is reached: no headroom at any window.
    SpendCapped,
}

impl Effective {
    /// The percentage an advisory threshold must classify.
    pub(crate) fn judged(self, used: f64) -> f64 {
        match self {
            Self::Raw => used,
            Self::Resets { percent, .. } => percent,
            Self::Unlimited => 0.0,
            Self::SpendCapped => 100.0,
        }
    }

    /// The EFFECTIVE cell. `-` whenever nothing was derived, never a guess.
    pub(crate) fn cell(self) -> String {
        match self {
            Self::Raw => "-".to_owned(),
            Self::Resets { resets: 0, percent } => percent_label(&format!("{percent:.1}")),
            Self::Resets { resets, percent } => {
                format!("{} x{resets}", percent_label(&format!("{percent:.1}")))
            }
            Self::Unlimited => "0%".to_owned(),
            Self::SpendCapped => "100%".to_owned(),
        }
    }

    /// How the judged percentage was derived, for one advisory line.
    fn derivation(self) -> Option<String> {
        match self {
            Self::Raw | Self::Resets { resets: 0, .. } => None,
            Self::Resets { resets, percent } => Some(format!(
                "effective {} over 1+{resets} declared resets",
                percent_label(&format!("{percent:.1}"))
            )),
            Self::Unlimited => Some("credits unlimited".to_owned()),
            Self::SpendCapped => Some("spend cap reached".to_owned()),
        }
    }
}

/// Derive the judged headroom for one window from what was declared and
/// reported. A spend cap outranks credits, which outrank declared resets.
pub(crate) fn effective(policy: &Policy, used: f64, window: Option<i64>) -> Effective {
    if policy.account.spend_capped() {
        return Effective::SpendCapped;
    }
    if policy.account.credits_relieve(window) {
        return Effective::Unlimited;
    }
    match policy.manual_resets {
        Some(resets) => Effective::Resets {
            resets,
            percent: used / (f64::from(resets) + 1.0),
        },
        None => Effective::Raw,
    }
}

/// The CREDITS cell for one scope.
pub(crate) fn credits_label(policy: &Policy) -> String {
    if policy.spend_capped() {
        return "spend-cap".to_owned();
    }
    match &policy.account.credits {
        Credits::Unreported => "-".to_owned(),
        Credits::Unlimited => "unlimited".to_owned(),
        Credits::Available(balance) => balance.clone(),
        Credits::AvailableUnknown => "available".to_owned(),
        Credits::Exhausted => "none".to_owned(),
    }
}

/// One row's derivation: the raw window and the judged percentage, together.
///
/// The table and the watchdog threshold must never diverge, so neither derives
/// anything itself — both read this, produced in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Derived {
    used: f64,
    effective: Effective,
}

impl Derived {
    /// Apply the rule to a percentage that is already parsed: the ONE place a
    /// raw number and a judged one are bound together.
    fn judge(policy: &Policy, used: f64, window: Option<i64>) -> Self {
        Self {
            used,
            effective: effective(policy, used, window),
        }
    }

    /// The percentage a threshold classifies.
    pub(crate) fn judged(self) -> f64 {
        self.effective.judged(self.used)
    }

    /// The EFFECTIVE cell the table renders.
    pub(crate) fn cell(self) -> String {
        self.effective.cell()
    }

    /// The compact settings cell keeps both the judged percentage and the
    /// existing reason it differs from the raw window. A window reset cannot
    /// be mistaken for relief from an account-wide spend cap.
    fn settings_cell(self) -> String {
        let judged = || percent_label(&format!("{:.1}", self.judged().clamp(0.0, 100.0)));
        match self.effective {
            Effective::Raw | Effective::Resets { resets: 0, .. } => judged(),
            Effective::Resets { resets, .. } => format!("{} x{resets}", judged()),
            Effective::Unlimited => "0% unlimited".to_owned(),
            Effective::SpendCapped => "100% spend-cap".to_owned(),
        }
    }

    /// How the judged percentage was derived, for one advisory line.
    pub(crate) fn derivation(self) -> Option<String> {
        self.effective.derivation()
    }
}

/// Derive one row under the policy it is judged by, or `None` when the row
/// states no usable percentage.
pub(crate) fn derived(policy: &Policy, row: &Row) -> Option<Derived> {
    Some(Derived::judge(policy, row_percent(row)?, row.observed_at))
}

/// The window percentage a row states, when it states a usable one.
fn row_percent(row: &Row) -> Option<f64> {
    row.used_percent
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

/// One judged observation: the row ae accepted, the policy it was judged
/// under, and that row's own provenance.
///
/// A level and the numbers it was decided from travel as ONE value. No caller
/// can pair a level with another observation's percentage, derivation or age,
/// which is exactly what an advisory quoting a REFUSED sample used to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    row: Row,
    policy: Policy,
    used: f64,
    observed_at: i64,
}

/// What an incoming reading moved in the reading it was offered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Adopted {
    /// Neither clock moved: the raw observation is not newer and no local fact
    /// changed, so there is nothing to judge again.
    Nothing,
    /// The declaration or an account fact moved, so the HELD observation is
    /// judged again under it.
    Policy,
    /// A strictly newer raw observation replaced the held one.
    Observation,
}

impl Reading {
    /// Judge `row` under `policy`.
    ///
    /// `None` unless the row states both a usable percentage and its own
    /// observation stamp: a reading is the provenance every caller reads a
    /// level from, and it is not one with either half missing.
    pub(crate) fn of(policy: Policy, row: Row) -> Option<Self> {
        let used = row_percent(&row)?;
        let observed_at = row.observed_at?;
        Some(Self {
            row,
            policy,
            used,
            observed_at,
        })
    }

    /// Adopt what `incoming` proves, under the two clocks ae keeps apart: a
    /// raw observation is replaced only by a strictly newer stamp, whatever the
    /// policy says, while the declaration and the account facts merge on their
    /// own provenance.
    pub(crate) fn adopt(&mut self, incoming: &Self) -> Adopted {
        let newer = strictly_newer(Some(self.observed_at), Some(incoming.observed_at));
        let repolicied = self.policy.absorb(&incoming.policy);
        if newer {
            self.row = incoming.row.clone();
            self.used = incoming.used;
            self.observed_at = incoming.observed_at;
            return Adopted::Observation;
        }
        if repolicied {
            Adopted::Policy
        } else {
            Adopted::Nothing
        }
    }

    /// The percentage a threshold classifies, derived exactly as the table
    /// derives the cell it renders.
    pub(crate) fn judged(&self) -> f64 {
        self.derived().judged()
    }

    /// When the observation this reading holds was made.
    pub(crate) const fn observed_at(&self) -> i64 {
        self.observed_at
    }

    fn derived(&self) -> Derived {
        Derived::judge(&self.policy, self.used, Some(self.observed_at))
    }
}

/// Advisory state for one stable vendor quota key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum QuotaLevel {
    Headroom,
    Low,
    Critical,
}

impl QuotaLevel {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Headroom => "headroom",
            Self::Low => "low",
            Self::Critical => "critical",
        }
    }
}

/// The threshold, with the hysteresis that keeps a window hovering on a
/// boundary from alternating: a state is left only well inside the next one.
fn classify(used: f64, prior: Option<QuotaLevel>) -> QuotaLevel {
    match prior {
        Some(QuotaLevel::Critical) if used >= 90.0 => QuotaLevel::Critical,
        Some(QuotaLevel::Critical | QuotaLevel::Low) if used >= 75.0 => {
            if used >= 95.0 {
                QuotaLevel::Critical
            } else {
                QuotaLevel::Low
            }
        }
        _ if used >= 95.0 => QuotaLevel::Critical,
        _ if used >= 80.0 => QuotaLevel::Low,
        _ => QuotaLevel::Headroom,
    }
}

/// A reading whose level has been decided, and the ONLY thing that carries one.
///
/// The level is a property of this observation, not a separate value beside it.
/// Nothing can set it, nothing can classify a percentage on its own, and every
/// renderer takes this whole value — so a level and the number, derivation and
/// age rendered with it cannot come from different observations. That pairing
/// is the defect this type exists to make unrepresentable.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Classified {
    reading: Reading,
    level: QuotaLevel,
}

impl Classified {
    /// Classify a window seen for the first time: there is no prior state for
    /// the hysteresis to hold on to.
    pub(crate) fn first(reading: Reading) -> Self {
        let level = classify(reading.judged(), None);
        Self { reading, level }
    }

    /// Offer an incoming reading to this one.
    ///
    /// What it may move is the reading's own rule — a raw observation only on a
    /// strictly newer stamp, account facts on their own stamps. If anything
    /// moves, the level is decided again from what is then HELD, anchored on
    /// the level held until now so the hysteresis is unchanged.
    pub(crate) fn adopt(&mut self, incoming: &Reading) -> Adopted {
        let adopted = self.reading.adopt(incoming);
        if adopted != Adopted::Nothing {
            self.level = classify(self.reading.judged(), Some(self.level));
        }
        adopted
    }

    /// The level this observation was classified at.
    pub(crate) const fn level(&self) -> QuotaLevel {
        self.level
    }

    /// Whether this is an account-wide weekly window (see [`weekly_context`]).
    fn account_weekly(&self, tool: ToolKind) -> bool {
        let row = &self.reading.row;
        row.window_minutes == Some(10_080)
            && (row.qualifier.is_none()
                || tool.adapter().quota.source == QuotaSource::CodexRollouts)
    }

    /// The percentage that level was decided from, for ranking one scope
    /// against another.
    pub(crate) fn judged(&self) -> f64 {
        self.reading.judged()
    }

    /// When the observation behind the level was made.
    pub(crate) const fn observed_at(&self) -> i64 {
        self.reading.observed_at()
    }
}

/// Parsing failed before a trustworthy snapshot could be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Input was not valid UTF-8.
    Utf8,
    /// A complete record was not valid JSON.
    Json,
    /// A named quota container had an incompatible shape.
    Shape,
}

/// Derive a freshness verdict from the record's own clocks.
#[must_use]
pub const fn freshness(observed_at: Option<i64>, resets_at: Option<i64>, now: i64) -> Status {
    let (Some(observed), Some(reset)) = (observed_at, resets_at) else {
        return Status::Unknown;
    };
    if reset <= now || observed.saturating_sub(now) >= FUTURE_SKEW_SECS {
        return Status::Unknown;
    }
    if now.saturating_sub(observed) <= FRESH_SECS {
        Status::Fresh
    } else {
        Status::Stale
    }
}

/// Read one non-negative numeric JSON literal for later display.
pub(crate) fn percent(value: Option<&Value>) -> Option<String> {
    let literal = match value? {
        Value::Num(number) => number.to_string(),
        Value::Raw(raw) => raw.clone(),
        _ => return None,
    };
    literal
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|_| literal)
}

/// Read an integral JSON number as an epoch.
pub(crate) fn epoch(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Num(number) => Some(*number),
        Value::Raw(raw) => raw.parse().ok(),
        _ => None,
    }
}

/// Read a positive integral JSON number as minutes.
pub(crate) fn minutes(value: Option<&Value>) -> Option<u32> {
    let number = epoch(value)?;
    u32::try_from(number).ok().filter(|minutes| *minutes > 0)
}

/// Read the UTC RFC 3339 spellings present in vendor caches.
pub(crate) fn vendor_timestamp(text: &str) -> Option<i64> {
    let date = text.get(..19)?;
    let suffix = text.get(19..)?;
    let utc = suffix == "Z"
        || suffix == "+00:00"
        || suffix.strip_prefix('.').is_some_and(|tail| {
            tail.strip_suffix('Z')
                .or_else(|| tail.strip_suffix("+00:00"))
                .is_some_and(|fraction| {
                    !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    if !utc {
        return None;
    }
    let mut canonical = String::with_capacity(20);
    canonical.push_str(date);
    canonical.push('Z');
    Timestamp::parse(&canonical).map(Timestamp::epoch)
}

/// World facts selected by the command entry before quota reads vendor state.
pub struct Inputs<'a> {
    /// The operator's home, where default client config homes live.
    pub home: Option<&'a Path>,
    /// Selected global ae config.
    pub global: Option<&'a Path>,
    /// Selected project-local ae config.
    pub local: Option<&'a Path>,
    /// Canonical ae sessions directory whose durable metas name Codex rollouts.
    /// Its parent is the state root whose shared `OpenCode` Go usage cache is
    /// read and, for `ae quota`, refreshed: see [`Inputs::state_root`].
    pub sessions: Option<&'a Path>,
    /// Observation instant.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Scope {
    tool: ToolKind,
    home: Option<PathBuf>,
    source: Option<PathBuf>,
    source_key: Option<PathBuf>,
    profiles: Vec<String>,
    configured_profiles: Vec<String>,
    clients: Vec<String>,
    /// Every RAW `[clients]` label this scope is an instance of — the
    /// correlation a scope that never resolved a source has left. Unlike
    /// `clients` this is not a display list: a default alias is recorded here
    /// even though it is deliberately not shown.
    accounts: Vec<String>,
    /// The recorded rollouts this scope stands for when it proved NO source.
    /// An unattributable rollout has exactly one honest identity — itself —
    /// so it is named by id rather than gathered with others that merely
    /// failed for the same reason. Empty for every scope that proved one.
    rollouts: Vec<String>,
    hint: Option<String>,
    manual_resets: Option<u8>,
    notes: Vec<String>,
    /// An agy command that keeps the caller's HOME: [`run`] may ask agy itself.
    leg: bool,
    /// An opencode profile on the `OpenCode` Go plan: its windows come from the
    /// shared usage cache for the key file in `source`.
    go: bool,
}

/// What one `[clients]` row declared about its own extra headroom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Declaration {
    manual_resets: Option<u8>,
    note: Option<String>,
}

struct ScopePaths {
    home: Option<PathBuf>,
    source: Option<PathBuf>,
    source_key: Option<PathBuf>,
    hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) profiles: Vec<String>,
    pub(crate) tool: ToolKind,
    pub(crate) home: Option<PathBuf>,
    pub(crate) source: Option<PathBuf>,
    pub(crate) clients: Vec<String>,
    pub(crate) rollout: Option<String>,
    pub(crate) owner: Option<String>,
    pub(crate) rows: Vec<Row>,
    pub(crate) hint: Option<String>,
    pub(crate) summary: Option<RolloutSummary>,
    /// What these windows are judged BY: the scope's declared manual resets
    /// and the account facts read with them, as one value.
    pub(crate) policy: Policy,
    /// Operator-facing notes about this scope's own declaration.
    pub(crate) notes: Vec<String>,
    /// The plan the account reports, when its client states one. A label to
    /// show: it is no part of [`Policy`] and no verdict reads it.
    pub(crate) plan: Option<claude::Plan>,
}

/// One bounded read of every configured quota scope.
///
/// `groups` is deliberately uncapped. The operator table uses the separate
/// rendered projection so its three-rollout display bound remains unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) groups: Vec<Group>,
    pub(crate) rendered: Vec<Group>,
    pub(crate) home: Option<PathBuf>,
    pub(crate) now: i64,
}

const SETTINGS_SCOPE_MAX: usize = 36;
const SETTINGS_BUCKET_MAX: usize = 30;

#[derive(Debug, Clone)]
struct SettingsReading {
    reading: Reading,
}

#[derive(Debug, Clone)]
struct SettingsScope {
    group: Group,
    policy: Policy,
    readings: Vec<SettingsReading>,
    statuses: Vec<Status>,
    /// The newest observation stamp any absorbed row carried, for the
    /// provenance a status-only line shows.
    observed: Option<i64>,
}

/// A seat identity precise enough to join its persisted conversation to one
/// observed vendor source without falling back to current profile config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedIdentity {
    pub(crate) tool: ToolKind,
    pub(crate) source: PathBuf,
}

/// Every journal action the watchdog's quota pass writes, spelled once.
///
/// The wire names are a contract with every reader of the journal, so they
/// stay byte-identical; a reader that matches on one takes it from here.
pub(crate) mod action {
    /// A lead-pair advisory delivery.
    pub(crate) const ADVISORY: &str = "quota-advisory";
    /// A booked advisory given up before it was delivered.
    pub(crate) const ADVISORY_DROPPED: &str = "quota-advisory-dropped";
    /// A checkpoint ask delivery. Its `ref` is the ask's receipt.
    pub(crate) const CHECKPOINT: &str = "quota-checkpoint";
    /// A booked checkpoint ask given up before it was delivered: its delivery
    /// failed twice or was uncertain, or it was cancelled because its seat
    /// left, its facts aged out or its window went silent.
    pub(crate) const CHECKPOINT_DROPPED: &str = "quota-checkpoint-dropped";
    /// A booked checkpoint ask withdrawn because its seat now runs another
    /// model family. Written with NO target, so no fold reads it as news to
    /// the seat; the summary names the agent.
    pub(crate) const CHECKPOINT_CANCELLED: &str = "quota-checkpoint-cancelled";
}

/// The Claude model families a window can be scoped to, in the versionless
/// form the family rule accepts as its first argument.
///
/// CLOSED on purpose: a qualifier or a model label outside this list proves
/// nothing, and an unproven family never silences an ask.
const CLAUDE_FAMILIES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

/// The recognized model family `text` names, for a tool whose windows can be
/// model-scoped at all. Decided by [`crate::model_drift::satisfies`], the one
/// family rule the drift mark already uses.
fn model_family(tool: ToolKind, text: &str) -> Option<&'static str> {
    if tool != ToolKind::Claude {
        return None;
    }
    CLAUDE_FAMILIES.into_iter().find(|family| {
        crate::model_drift::satisfies(family, text, crate::tool::PinMatch::FamilyVersion)
    })
}

/// Whether a window scoped to `qualifier` is PROVEN not to bind a seat whose
/// own frame last showed `live`.
///
/// True only when both name a recognized family and the families differ. An
/// account-wide window, an unknown qualifier, a plan name, an unknown label
/// and no observed model all answer false: the seat is asked.
pub(crate) fn scoped_elsewhere(
    tool: ToolKind,
    qualifier: Option<&str>,
    live: Option<&str>,
) -> bool {
    let window = qualifier.and_then(|text| model_family(tool, text));
    let seat = live.and_then(|text| model_family(tool, text));
    matches!((window, seat), (Some(window), Some(seat)) if window != seat)
}

/// The account-wide weekly reading among `held`, as its judged cell, for the
/// notice about a SHORTER window of the same scope to name beside its reset.
///
/// Account-wide: exactly a week long, and not scoped to a model. A qualifier
/// scopes a window only for Claude, whose qualifier is a model; Codex spells
/// its plan there. `None` when `trigger` is itself that window, or when nothing
/// held qualifies. Several: the worst by JUDGED percentage, the first on a tie.
/// Which readings are the scope's is the caller's rule; this trusts its input.
pub(crate) fn weekly_context<'a>(
    tool: ToolKind,
    trigger: &Classified,
    held: impl IntoIterator<Item = &'a Classified>,
) -> Option<String> {
    if trigger.account_weekly(tool) {
        return None;
    }
    held.into_iter()
        .filter(|reading| reading.account_weekly(tool))
        .fold(None::<&Classified>, |worst, reading| match worst {
            Some(worst) if worst.judged() >= reading.judged() => Some(worst),
            _ => Some(reading),
        })
        .map(|reading| reading.reading.derived().settings_cell())
}

/// Sanitized, bounded quota facts retained across a deferred advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Advisory {
    tool: ToolKind,
    scope: String,
    owner: Option<String>,
    bucket: String,
    window: String,
    used: String,
    observed_at: Option<i64>,
    resets_at: Option<i64>,
    state: String,
    recovered: bool,
    /// How the judged percentage differs from the raw window, when it does.
    derivation: Option<String>,
    /// The account is spend-capped, which no window reset can free.
    spend_capped: bool,
    /// The JUDGED percentage in its compact spelling, carrying the derivation
    /// that made it differ from the raw window. Set in the one constructor
    /// below from the very reading that decided the level, so the checkpoint
    /// ask cannot quote a percentage some other observation produced.
    judged: String,
    /// The same scope's account-wide weekly reading as its judged cell, when
    /// the caller held one (see [`weekly_context`]). Never guessed.
    weekly: Option<String>,
}

/// The checkpoint ask's ONE text owner: fixed wording, three substitutions and
/// nothing a sender may re-word. `{facts}` is the HELD reading's own scope,
/// level, judged percentage and reset; `{cause}` says why now, empty when a
/// reset cannot help; `{memo}` is this session's memo helper.
const CHECKPOINT_ASK: &str = "{facts}. Checkpoint NOW{cause}: {memo} add \
                              --topic <your slice>, then --topic parking; each carries next exact \
                              command · decisions + why · dead ends · constraints VERBATIM · files \
                              + git · owed replies (ids). No reply needed.";

/// The ask rides ONE paste into a live input box, so it stays short enough to
/// read at a glance and well inside any harness's single-turn comfort.
///
/// A PIN rather than a clip: the wording above is fixed and every fact rendered
/// into it is already a bounded cell, so the length cannot drift without
/// somebody editing that text and reading this.
#[cfg(test)]
pub(crate) const CHECKPOINT_ASK_MAX: usize = 600;

impl Advisory {
    /// When the window these facts describe resets, when the vendor said.
    pub(crate) const fn resets_at(&self) -> Option<i64> {
        self.resets_at
    }

    pub(crate) fn current_at(&self, now: i64) -> bool {
        matches!(
            freshness(self.observed_at, self.resets_at, now),
            Status::Fresh | Status::Stale
        ) && self
            .observed_at
            .is_some_and(|at| now.saturating_sub(at) <= 60 * 60)
    }

    /// These facts plus the scope's account-wide weekly reading, or none.
    pub(crate) fn with_weekly(mut self, weekly: Option<String>) -> Self {
        self.weekly = weekly;
        self
    }

    /// When the window resets, as a span from `now`: `unknown` when the vendor
    /// stated none, never a guessed duration. With the weekly reading beside it.
    fn reset_words(&self, now: i64) -> String {
        let reset = self.resets_at.map_or_else(
            || "unknown".to_owned(),
            |at| span_label(at.saturating_sub(now)),
        );
        match &self.weekly {
            Some(weekly) => format!("{reset}; weekly {weekly}"),
            None => reset,
        }
    }

    pub(crate) fn render(&self, meta_dir: &Path, now: i64) -> String {
        let observed = self.observed_at.map_or_else(
            || "unknown".to_owned(),
            |at| age_label(now.saturating_sub(at)),
        );
        let reset = self.reset_words(now);
        let advice = if self.spend_capped {
            " — spend cap reached: another client or more credits, not a window reset"
        } else if self.recovered {
            " — back to headroom"
        } else {
            " — assess this window's reset and weekly headroom before new work"
        };
        let derivation = self
            .derivation
            .as_deref()
            .map_or_else(String::new, |why| format!("; {why}"));
        format!(
            "quota: {} · {}{} {} {} {} ({}{derivation}, observed {observed}), resets in {reset}{advice}; table: {}/quota",
            self.tool.as_str(),
            self.scope,
            self.owner
                .as_deref()
                .map_or_else(String::new, |owner| format!(" · {owner}")),
            self.bucket,
            self.window,
            self.used,
            self.state,
            meta_dir.display()
        )
    }

    /// The quota-Low CHECKPOINT ASK for one seat on this scope.
    ///
    /// Every fact it states — the scope, the level and the judged percentage —
    /// comes from this advisory, which [`Observation::advisory`] built from the
    /// one classified reading that decided the level. There is no path here for
    /// a sample the freshness rule refused.
    ///
    /// Advisory only: it asks for a durable checkpoint and explicitly wants no
    /// reply, so it opens no request and changes no state.
    pub(crate) fn checkpoint_ask(&self, meta_dir: &Path, now: i64) -> String {
        // A reset frees a window, never a spend cap: say so instead of promising
        // that the pool returns.
        let reset = self.reset_words(now);
        let (resets, cause) = if self.spend_capped {
            (
                format!("window resets in {reset}; spend cap is not freed by a reset"),
                "",
            )
        } else {
            (
                format!("resets in {reset}"),
                " before this pool can run out",
            )
        };
        let facts = format!(
            "quota {} · {}{} {} {} — {} at {}, {}",
            self.tool.as_str(),
            self.scope,
            self.owner
                .as_deref()
                .map_or_else(String::new, |owner| format!(" · {owner}")),
            self.bucket,
            self.window,
            self.state,
            self.judged,
            resets,
        );
        // The facts go in LAST: a vendor-supplied label is never scanned for
        // another placeholder.
        CHECKPOINT_ASK
            .replace("{cause}", cause)
            .replace("{memo}", &format!("{}/memo", meta_dir.display()))
            .replace("{facts}", &facts)
    }
}

impl Observation {
    /// The one display order: readable scopes first, most headroom first.
    /// Judge adopted windows under the merged scope policy, as the view does;
    /// exhausted windows have zero headroom and equal keys keep input order.
    fn ordered_scopes(&self) -> Vec<SettingsScope> {
        let mut ranked: Vec<_> = self
            .dialog_scopes()
            .into_iter()
            .map(|scope| {
                let incomplete = scope
                    .statuses
                    .iter()
                    .any(|status| matches!(status, Status::ReadError | Status::Truncated));
                let worst = if incomplete {
                    f64::INFINITY
                } else {
                    scope
                        .readings
                        .iter()
                        .filter_map(|reading| {
                            derived(&scope.policy, &reading.reading.row)
                                .map(|value| value.judged().clamp(0.0, 100.0))
                        })
                        .reduce(f64::max)
                        .unwrap_or(f64::INFINITY)
                };
                (worst, scope)
            })
            .collect();
        ranked.sort_by(|left, right| left.0.total_cmp(&right.0));
        ranked.into_iter().map(|(_, scope)| scope).collect()
    }

    /// Preserve rollout order; unmatched table-only groups keep input order last.
    fn ordered_groups<'a>(&self, groups: &'a [Group]) -> Vec<&'a Group> {
        let scopes = self.ordered_scopes();
        scopes
            .iter()
            .flat_map(|scope| {
                groups
                    .iter()
                    .filter(move |group| settings_same_scope(&scope.group, group))
            })
            .chain(groups.iter().filter(|group| {
                !scopes
                    .iter()
                    .any(|scope| settings_same_scope(&scope.group, group))
            }))
            .collect()
    }

    /// Group every known client scope once, merging repeated windows through
    /// [`Reading::adopt`] — the same provenance rule the watchdog uses.
    ///
    /// Codex rollout owners are evidence about one scope, not more scopes.
    fn dialog_scopes(&self) -> Vec<SettingsScope> {
        let mut scopes: Vec<SettingsScope> = Vec::new();
        for group in &self.groups {
            if let Some(scope) = scopes
                .iter_mut()
                .find(|scope| settings_same_scope(&scope.group, group))
            {
                scope.absorb(group, self.now);
            } else {
                let mut scope = SettingsScope {
                    group: group.clone(),
                    policy: Policy::default(),
                    readings: Vec::new(),
                    statuses: Vec::new(),
                    observed: None,
                };
                scope.absorb(group, self.now);
                scopes.push(scope);
            }
        }
        // `groups` is deliberately the uncapped window feed the watchdog
        // consumes. Codex budget completeness lives only in `rendered`'s
        // summary so it cannot create a fake watchdog window; overlay only its
        // hard failure status onto this dialog projection.
        for group in &self.rendered {
            let hard_status = group.summary.as_ref().and_then(|summary| {
                summary
                    .status
                    .filter(|status| matches!(status, Status::ReadError | Status::Truncated))
            });
            if let Some(status) = hard_status
                && let Some(scope) = scopes
                    .iter_mut()
                    .find(|scope| settings_summary_scope(&scope.group, group))
            {
                scope.statuses.push(status);
            }
        }
        scopes
    }

    /// [`Self::quota_view_paced`] with no history: the facts without a burn.
    #[cfg(test)]
    pub(crate) fn quota_view(&self) -> Vec<view::Scope> {
        self.quota_view_paced(&pace::PaceTable::default())
    }

    /// Every scope as view facts, headroom first: the one projection both quota
    /// screens lay out. Every percentage comes from [`derived`], the same
    /// derivation the table and the watchdog read, and every level from the one
    /// classifier; a scope with nothing usable states why instead of a guess. `pace`
    /// is the table `ae quota` draws its PACE column from, read for display only.
    pub(crate) fn quota_view_paced(&self, pace: &pace::PaceTable) -> Vec<view::Scope> {
        self.ordered_scopes()
            .iter()
            .map(|scope| scope.viewed(self.home.as_deref(), self.now, pace))
            .collect()
    }

    /// Every Claude scope with a resolved source — the ones a statusline feed
    /// can key — and the newest observation any of its rows carries. `None` is
    /// a scope with no reading at all. Reads the same groups the table does,
    /// so `ae doctor` can never name a scope `ae quota` does not list.
    pub(crate) fn claude_seen(&self) -> Vec<(String, Option<i64>)> {
        self.groups
            .iter()
            .filter(|group| {
                group.source.is_some()
                    && group.tool.adapter().quota.source == QuotaSource::ClaudeCache
            })
            .map(|group| {
                let seen = group.rows.iter().filter_map(|row| row.observed_at).max();
                (scope_identity(group, self.home.as_deref()), seen)
            })
            .collect()
    }

    /// Render one advisory line from the same sanitized, width-bounded cells
    /// as the operator table. The helper path is ae-owned rather than vendor
    /// input and remains complete so the recipient can invoke it verbatim.
    #[cfg(test)]
    pub(crate) fn state_line(
        &self,
        group: &Group,
        classified: &Classified,
        meta_dir: &Path,
    ) -> String {
        self.state_line_at(group, classified, meta_dir, self.now)
    }

    /// The line that quotes a scope's CURRENT state, such as beside a throttle
    /// nudge. The same cells as a notice, without the language of a change.
    pub(crate) fn state_line_at(
        &self,
        group: &Group,
        classified: &Classified,
        meta_dir: &Path,
        now: i64,
    ) -> String {
        self.advisory(group, classified, false)
            .render(meta_dir, now)
    }

    /// The notice a TRANSITION books. A level that has just arrived at
    /// headroom says so; there is no other way to spell a state, so no caller
    /// can describe one observation with another's words.
    pub(crate) fn transition(&self, group: &Group, classified: &Classified) -> Advisory {
        self.advisory(
            group,
            classified,
            classified.level() == QuotaLevel::Headroom,
        )
    }

    /// Render one advisory from a CLASSIFIED reading: the group supplies only
    /// the labels of the scope, while the level, every number, the derivation
    /// and the age all come from the one observation that decided it.
    fn advisory(&self, group: &Group, classified: &Classified, recovered: bool) -> Advisory {
        let reading = &classified.reading;
        let row = &reading.row;
        let state = classified.level().label();
        let scope = bounded_cell(&scope_identity(group, self.home.as_deref()), 1);
        let owner = group.owner.as_deref().map(|owner| bounded_cell(owner, 0));
        let bucket = row.qualifier.as_deref().map_or_else(
            || row.bucket.clone(),
            |qualifier| format!("{} {qualifier}", row.bucket),
        );
        let bucket = bounded_cell(&bucket, 2);
        let window = row
            .window_minutes
            .map_or_else(|| "-".to_owned(), window_label);
        let used = advisory_percent(row.used_percent.as_deref());
        let derivation = reading.derived().derivation();
        Advisory {
            tool: group.tool,
            scope,
            owner,
            bucket,
            window,
            used,
            observed_at: row.observed_at,
            resets_at: row.resets_at,
            state: state.to_owned(),
            recovered,
            derivation,
            spend_capped: reading.policy.spend_capped(),
            // The SAME derivation the EFFECTIVE column and the settings dialog
            // read, taken from the reading that decided `state` just above.
            judged: reading.derived().settings_cell(),
            weekly: None,
        }
    }
}

impl SettingsScope {
    fn absorb(&mut self, group: &Group, now: i64) {
        self.policy.absorb(&group.policy);
        for row in &group.rows {
            if let Some(observed) = row.observed_at {
                // NEWEST, not oldest: this is the stamp the scope last heard,
                // and the only one a status-only line can honestly quote as
                // "seen". Numbers stay blank, so the optimistic direction
                // implies no headroom.
                self.observed = Some(self.observed.map_or(observed, |held| held.max(observed)));
            }
            let status = if matches!(row.status, Status::Fresh | Status::Stale) {
                freshness(row.observed_at, row.resets_at, now)
            } else {
                row.status
            };
            self.statuses.push(status);
            if !matches!(status, Status::Fresh | Status::Stale) {
                continue;
            }
            let Some(reading) = Reading::of(group.policy.clone(), row.clone()) else {
                continue;
            };
            if let Some(existing) = self
                .readings
                .iter_mut()
                .find(|existing| settings_same_window(&existing.reading.row, &reading.row))
            {
                existing.reading.adopt(&reading);
            } else {
                self.readings.push(SettingsReading { reading });
            }
        }
    }

    /// This scope as view facts: its windows in a
    /// stable bucket/qualifier/window order that no input order can change.
    fn viewed(&self, home: Option<&Path>, now: i64, pace: &pace::PaceTable) -> view::Scope {
        let title = format!(
            "{} · {}",
            self.group.tool.as_str(),
            settings_cell(&scope_identity(&self.group, home), SETTINGS_SCOPE_MAX)
        );
        let profiles = self
            .group
            .profiles
            .iter()
            .map(|profile| settings_cell(profile, SETTINGS_SCOPE_MAX))
            .collect::<Vec<_>>()
            .join(" ");
        let account = self.account_words(now);
        let incomplete = self
            .statuses
            .iter()
            .any(|status| matches!(status, Status::ReadError | Status::Truncated));
        if incomplete || self.readings.is_empty() {
            return view::Scope {
                title,
                profiles,
                account,
                only: Some(self.unusable_words(now)),
                windows: Vec::new(),
            };
        }
        let mut readings: Vec<&SettingsReading> = self.readings.iter().collect();
        readings.sort_by(|left, right| {
            let left = &left.reading.row;
            let right = &right.reading.row;
            left.bucket
                .cmp(&right.bucket)
                .then_with(|| left.qualifier.cmp(&right.qualifier))
                .then_with(|| left.window_minutes.cmp(&right.window_minutes))
        });
        // agy's note beside the answer it kept past a refresh that had none.
        let note = leg_hint(&self.group).map(str::to_owned);
        let windows = readings
            .into_iter()
            .map(|reading| self.window_view(reading, now, pace))
            .collect();
        view::Scope {
            title,
            profiles,
            account,
            only: note,
            windows,
        }
    }

    /// The account facts that belong to the scope, not to one window: the
    /// credit state, then the plan the client reports.
    fn account_words(&self, now: i64) -> Option<String> {
        let plan = self
            .group
            .plan
            .as_ref()
            .and_then(|plan| plan_words(plan, now));
        match (self.credit_words(), plan) {
            (Some(credits), Some(plan)) => Some(format!("{credits} · {plan}")),
            (credits, plan) => credits.or(plan),
        }
    }

    fn credit_words(&self) -> Option<String> {
        let credits = &self.policy.account.credits;
        if self.policy.spend_capped() {
            return Some(match credits {
                Credits::Available(balance) => format!(
                    "credits {} · spend cap reached",
                    settings_cell(balance, CREDIT_BALANCE_MAX)
                ),
                _ => "spend cap reached".to_owned(),
            });
        }
        match credits {
            Credits::Unreported => None,
            Credits::Unlimited => Some("credits unlimited".to_owned()),
            Credits::Available(balance) => Some(format!(
                "credits {}",
                settings_cell(balance, CREDIT_BALANCE_MAX)
            )),
            Credits::AvailableUnknown => Some("credits available".to_owned()),
            Credits::Exhausted => Some("no credits".to_owned()),
        }
    }

    /// The words for a scope with nothing usable. The age belongs only to a
    /// scope with NO reading: it is the only provenance such a line has, and an
    /// incomplete scope that still adopted one must not borrow a sibling's.
    fn unusable_words(&self, now: i64) -> String {
        let agy = self.group.tool.quota_is_asked();
        let mut words = match settings_status(&self.statuses) {
            Status::Unsupported => "this client reports no quota",
            Status::ReadError if agy => "quota unreadable",
            Status::ReadError => "quota file unreadable",
            Status::Truncated => "read cut short",
            _ => "no reading ae can trust",
        }
        .to_owned();
        if self.readings.is_empty()
            && let Some(observed) = stated_observed_at(self.observed, now)
        {
            let said = if agy { "asked" } else { "last report" };
            let _ = write!(
                words,
                " ({said} {} ago)",
                span_label(now.saturating_sub(observed))
            );
        }
        // agy's hint is the leg's own reason, or the command to run by hand.
        let own = leg_hint(&self.group);
        let hint = manual_refresh_hint(self.group.tool, self.statuses.iter().copied()).or(own);
        if let Some(hint) = hint {
            let _ = write!(words, " — {hint}");
        }
        words
    }

    /// One adopted window as view facts.
    fn window_view(
        &self,
        reading: &SettingsReading,
        now: i64,
        pace: &pace::PaceTable,
    ) -> view::Window {
        let row = &reading.reading.row;
        let derived = derived(&self.policy, row);
        let judged = derived.map(Derived::judged);
        let raw = derived.map(|derived| pct_label(derived.used));
        // COUNTS names the judged percentage only where it reads differently
        // from the vendor's: a reset in hand that saturates at 100% moved
        // nothing the operator can see.
        let counts = judged
            .map(pct_label)
            .filter(|judged| raw.as_deref() != Some(judged.as_str()));
        let fill = judged.map(bar_cells);
        let why = derived.and_then(|derived| match derived.effective {
            Effective::Resets { resets, .. } if resets > 0 => Some(view::Why::Resets(resets)),
            Effective::Unlimited => Some(view::Why::Unlimited),
            Effective::SpendCapped => Some(view::Why::SpendCap),
            Effective::Raw | Effective::Resets { .. } => None,
        });
        // The drop order's ONLY judgement: the one classifier, fresh — the
        // dialog holds no hysteresis state.
        let level = match Classified::first(reading.reading.clone()).level() {
            QuotaLevel::Headroom => view::Level::Headroom,
            QuotaLevel::Low => view::Level::Low,
            QuotaLevel::Critical => view::Level::Critical,
        };
        let stale = freshness(row.observed_at, row.resets_at, now) == Status::Stale;
        view::Window {
            name: view_window_name(row),
            raw,
            counts,
            fill,
            why,
            level,
            stale,
            resets: row.resets_at.map(|at| span_label(at.saturating_sub(now))),
            seen: row.observed_at.map(|at| span_label(now.saturating_sub(at))),
            burn: self.burn_view(pace, row, why),
        }
    }

    /// How fast this window fills, read from the ONE verdict the `PACE` column draws: no
    /// second rate, and no burn where the judged percentage is already settled by an
    /// account constraint (unlimited credits, a spend cap).
    fn burn_view(
        &self,
        pace: &pace::PaceTable,
        row: &Row,
        why: Option<view::Why>,
    ) -> Option<view::Burn> {
        if matches!(why, Some(view::Why::Unlimited | view::Why::SpendCap)) {
            return None;
        }
        let Some(source) = self.group.source.as_deref() else {
            return Some(view::Burn::NoHistory);
        };
        let verdict = pace.verdict_for(source, row)?;
        match (verdict.pace, verdict.basis) {
            (pace::Pace::Absent, _)
            | (pace::Pace::Bound { .. } | pace::Pace::Rate { .. }, None) => None,
            (pace::Pace::Unknown, None) => Some(view::Burn::Unusable),
            (pace::Pace::Unknown, Some(basis)) => Some(view::Burn::Short {
                seen: span_label(basis.span_secs),
                need: span_label(basis.need_secs),
            }),
            (found @ (pace::Pace::Bound { .. } | pace::Pace::Rate { .. }), Some(basis)) => {
                Some(view::Burn::Rate {
                    rate: found.cell(),
                    span: span_label(basis.span_secs),
                })
            }
        }
    }
}

/// The plain name of a window: `Session`, `Weekly`, `Weekly Fable`, a client's
/// own bucket id as it is, then the window length.
fn view_window_name(row: &Row) -> String {
    let base = match (row.bucket.as_str(), row.qualifier.as_deref()) {
        ("session", Some(qualifier)) if !qualifier.is_empty() => format!("Session {qualifier}"),
        ("session", _) => "Session".to_owned(),
        ("weekly_all", _) => "Weekly".to_owned(),
        ("weekly_scoped", Some(qualifier)) => format!("Weekly {qualifier}"),
        (bucket, Some(qualifier)) => format!("{bucket} {qualifier}"),
        (bucket, None) => bucket.to_owned(),
    };
    let base = settings_cell(&base, SETTINGS_BUCKET_MAX);
    row.window_minutes.map_or_else(
        || base.clone(),
        |minutes| format!("{base} {}", window_label(minutes)),
    )
}

/// A percentage as the dialog spells it, clamped to a window's own range.
fn pct_label(value: f64) -> String {
    percent_label(&format!("{:.1}", value.clamp(0.0, 100.0)))
}

/// The filled cells of an 8-cell bar for a judged percentage.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the clamped, rounded value lies in 0..=8"
)]
fn bar_cells(judged: f64) -> u8 {
    (judged.clamp(0.0, 100.0) / 100.0 * 8.0).round() as u8
}

fn settings_same_scope(left: &Group, right: &Group) -> bool {
    left.tool == right.tool
        && left.home == right.home
        && left.source == right.source
        && left.profiles == right.profiles
        && left.clients == right.clients
        && left.hint == right.hint
        // A scope that proved NO source is not an account, so it has no
        // identity to be grouped by except the rollout it stands for: two of
        // them collapse only if they ARE the same rollout. A scope that proved
        // one keeps the shipped rule above — its rollout owners are evidence
        // about that one account, not about more accounts. Without this the
        // dialog would still gather what the table now holds apart, and the
        // two surfaces would disagree about the same question.
        && (left.source.is_some() || left.rollout == right.rollout)
}

/// Correlate a capped-render completeness summary back to its uncapped scope.
/// The full tuple keeps unresolved sources with distinct profile hints apart.
fn settings_summary_scope(scope: &Group, summary: &Group) -> bool {
    settings_same_scope(scope, summary)
}

fn settings_same_window(left: &Row, right: &Row) -> bool {
    WindowKey::of(left) == WindowKey::of(right)
}

fn settings_status(statuses: &[Status]) -> Status {
    [
        Status::Truncated,
        Status::ReadError,
        Status::Unsupported,
        Status::Unknown,
    ]
    .into_iter()
    .find(|status| statuses.contains(status))
    .unwrap_or(Status::Unknown)
}

fn settings_cell(text: &str, max: usize) -> String {
    crate::event_text::display_cell(text, max)
}

/// Resolve only a complete, recorded seat identity. Legacy/default homes and
/// inconsistent mode/base pairs are intentionally not guessed.
pub(crate) fn recorded_identity(entry: &crate::meta::RosterEntry) -> Option<RecordedIdentity> {
    use crate::meta::{RecordedConfigHome, RecordedConfigHomeBase};

    let tool = ToolKind::from_binary_name(entry.binary.as_deref()?);
    let source = match tool.adapter().quota.source {
        QuotaSource::ClaudeCache => match (&entry.config_home, &entry.config_home_base) {
            (RecordedConfigHome::Path(home), RecordedConfigHomeBase::Missing) => {
                home.join(".claude.json")
            }
            (RecordedConfigHome::Implicit(_), RecordedConfigHomeBase::Path(base)) => {
                base.join(".claude.json")
            }
            _ => return None,
        },
        QuotaSource::CodexRollouts => match (&entry.config_home, &entry.config_home_base) {
            (RecordedConfigHome::Path(home), RecordedConfigHomeBase::Missing)
            | (RecordedConfigHome::Implicit(home), RecordedConfigHomeBase::Path(_)) => {
                home.join("sessions")
            }
            _ => return None,
        },
        QuotaSource::GrokLog | QuotaSource::Unsupported => return None,
    };
    // A Codex seat without a recorded conversation is not a proven identity;
    // the id itself is not part of the identity, because the window it would
    // name belongs to the config home rather than to one conversation.
    if tool.adapter().quota.source == QuotaSource::CodexRollouts && entry.harness_session.is_none()
    {
        return None;
    }
    Some(RecordedIdentity {
        tool,
        source: canonical_source(source).ok()?,
    })
}

struct CodexGroups {
    all: Vec<Group>,
    rendered: Vec<Group>,
    /// Each rollout's window history, with the canonical source it came from.
    series: Vec<(PathBuf, pace::Series)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RolloutSummary {
    hidden: usize,
    unreadable: usize,
    oldest_observed: Option<i64>,
    not_read: bool,
    status: Option<Status>,
}

enum ReadRows {
    Rows(Observed),
    Missing,
    Failed,
    Truncated,
}

/// One source's windows plus the account facts it reported with them.
struct Observed {
    rows: Vec<Row>,
    account: Account,
    /// The thinned history of each window, when the source keeps one.
    series: Vec<pace::Series>,
}

pub(crate) enum Bounded<T> {
    Ready(T),
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FleetRollout {
    owner: String,
    profile: String,
    /// The seat's recorded `client.<slot>` label, if it records one: the
    /// account an override selected, which a bare profile name cannot name.
    client: Option<String>,
    id: String,
    tool: ToolKind,
    location: RolloutLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RolloutLocation {
    Configured,
    Recorded {
        home: PathBuf,
        source: PathBuf,
        source_key: PathBuf,
    },
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FleetStatus {
    Complete,
    Failed,
    Truncated,
}

struct FleetRollouts {
    rollouts: Vec<FleetRollout>,
    status: FleetStatus,
}

pub(crate) struct RolloutFile {
    path: PathBuf,
    metadata: std::fs::Metadata,
    modified: Option<SystemTime>,
}

impl RolloutFile {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn metadata(&self) -> &std::fs::Metadata {
        &self.metadata
    }

    pub(crate) fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    pub(crate) fn len(&self) -> u64 {
        self.metadata.len()
    }
}

enum RolloutSource {
    File(Box<RolloutFile>),
    Missing,
    Failed,
}

struct LocatedRollout<'a> {
    rollout: &'a FleetRollout,
    source: RolloutSource,
    modified: Option<SystemTime>,
}

struct RankedGroup {
    group: Group,
    observed: Option<i64>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "a table line lives for one render; boxing the cells buys nothing"
)]
enum RenderLine {
    Cells([String; COLUMNS]),
    Summary { label: String, status: String },
}

pub(crate) struct Budget {
    files_left: usize,
    bytes_left: u64,
    started: Instant,
    max_elapsed: Duration,
}

impl Budget {
    pub(crate) fn new() -> Self {
        Self {
            files_left: QUOTA_MAX_FILES,
            bytes_left: QUOTA_MAX_BYTES,
            started: Instant::now(),
            max_elapsed: QUOTA_MAX_ELAPSED,
        }
    }

    pub(crate) fn expired(&self) -> bool {
        self.started.elapsed() >= self.max_elapsed
    }

    pub(crate) fn claim_file(&mut self) -> bool {
        if self.expired() || self.files_left == 0 {
            return false;
        }
        self.files_left -= 1;
        true
    }

    pub(crate) fn reserve_bytes(&mut self, bytes: u64) -> bool {
        if self.expired() || bytes > self.bytes_left {
            return false;
        }
        self.bytes_left -= bytes;
        true
    }

    pub(crate) fn refund_bytes(&mut self, bytes: u64) {
        self.bytes_left = self.bytes_left.saturating_add(bytes);
    }
}

/// Read every configured client scope under one invocation-wide budget.
///
/// The returned data is uncapped even though the operator table deliberately
/// displays only the three newest Codex rollouts per scope.
pub(crate) fn observe(inputs: &Inputs<'_>) -> Result<Observation, crate::config::ConfigError> {
    observe_full(inputs).map(|(observation, _)| observation)
}

/// [`observe`], plus the thinned history the sources kept beside their rows.
///
/// The operator table and the dialogs read the history, for DISPLAY only: the watchdog
/// and the autoreseat leg keep calling [`observe`], and no pace reaches a level.
pub(crate) fn observe_full(
    inputs: &Inputs<'_>,
) -> Result<(Observation, pace::SeriesSet), crate::config::ConfigError> {
    let cfg = crate::config::read_identity(inputs.global, inputs.local)?;
    Ok(observe_scopes(
        inputs,
        configured_scopes_with(&cfg, inputs.home, &OptIns::of(inputs)),
    ))
}

/// [`observe_full`] over scopes already read from the config.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per quota source over one scope table; each arm is the whole of its source's read"
)]
fn observe_scopes(inputs: &Inputs<'_>, mut scopes: Vec<Scope>) -> (Observation, pace::SeriesSet) {
    let mut groups = Vec::new();
    let mut rendered = Vec::new();
    let mut series = pace::SeriesSet::default();
    let mut budget = Budget::new();
    let fleet = fleet_rollouts(inputs.sessions, &mut budget);
    add_recorded_codex_scopes(&mut scopes, &fleet);
    for scope in &scopes {
        if scope.go {
            let group = go_group(scope, inputs, &mut series);
            rendered.push(group.clone());
            groups.push(group);
            continue;
        }
        let quota = scope.tool.adapter().quota;
        match quota.source {
            QuotaSource::ClaudeCache => {
                let (observed, plan) = claude_observed(scope, inputs, &mut budget);
                let group = Group {
                    profiles: scope.profiles.clone(),
                    tool: scope.tool,
                    home: scope.home.clone(),
                    source: scope.source_key.clone(),
                    clients: scope.clients.clone(),
                    rollout: None,
                    owner: None,
                    rows: observed.rows,
                    hint: scope.hint.clone(),
                    summary: None,
                    policy: Policy::new(scope.manual_resets, observed.account),
                    notes: scope.notes.clone(),
                    plan,
                };
                rendered.push(group.clone());
                groups.push(group);
            }
            QuotaSource::CodexRollouts => {
                let observed = codex_groups(scope, &fleet, inputs.now, &mut budget);
                for (source, held) in observed.series {
                    series.add(&source, held);
                }
                groups.extend(observed.all);
                rendered.extend(observed.rendered);
            }
            QuotaSource::Unsupported | QuotaSource::GrokLog => {
                let (read, on_demand) = match quota.source {
                    QuotaSource::GrokLog => read_grok(scope, inputs.now, &mut budget),
                    _ => (ReadRows::Missing, false),
                };
                // No readable window is the row this tool always had.
                let unsupported = matches!(read, ReadRows::Missing);
                let observed = if unsupported {
                    Observed {
                        rows: vec![placeholder(Status::Unsupported)],
                        account: Account::default(),
                        series: Vec::new(),
                    }
                } else {
                    rows_or_placeholder(read)
                };
                let mut notes = scope.notes.clone();
                if on_demand {
                    notes.push(ON_DEMAND_NOTE.to_owned());
                }
                let group = Group {
                    profiles: scope.profiles.clone(),
                    tool: scope.tool,
                    home: scope.home.clone(),
                    source: scope.source_key.clone(),
                    clients: scope.clients.clone(),
                    rollout: None,
                    owner: None,
                    rows: observed.rows,
                    hint: scope.hint.clone().or_else(|| {
                        quota
                            .unsupported_hint
                            .filter(|_| unsupported)
                            .map(str::to_owned)
                    }),
                    summary: None,
                    policy: Policy::new(scope.manual_resets, observed.account),
                    notes,
                    plan: None,
                };
                rendered.push(group.clone());
                groups.push(group);
            }
        }
    }
    if let Some(group) = copilot_group(inputs) {
        for held in [&mut groups, &mut rendered] {
            held.retain_mut(|old| {
                let was_empty = old.profiles.is_empty();
                old.profiles
                    .retain(|profile| !group.profiles.contains(profile));
                was_empty || !old.profiles.is_empty()
            });
            held.push(group.clone());
        }
    }
    let home = inputs.home.and_then(|home| {
        crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(home.to_path_buf()))
            .ok()
            .and_then(|resolved| match resolved {
                crate::launch_cmd::Resolved::Path(path) => Some(path),
                crate::launch_cmd::Resolved::Absent | crate::launch_cmd::Resolved::Unknown(_) => {
                    None
                }
            })
    });
    (
        Observation {
            groups,
            rendered,
            home,
            now: inputs.now,
        },
        series,
    )
}

const COPILOT_SCOPE: &str = "github copilot · plan ";

fn is_copilot(group: &Group) -> bool {
    group
        .clients
        .first()
        .is_some_and(|label| label.starts_with(COPILOT_SCOPE))
}

/// One explicitly opted-in account, even when no configured profile uses it.
/// No source key: these monthly rows never enter automation or pace history.
fn copilot_group(inputs: &Inputs<'_>) -> Option<Group> {
    use crate::go_usage::{Provider, Read};
    let provider = Provider::Copilot;
    let configured = crate::go_usage::configured_for(provider, inputs.global, inputs.home);
    if matches!(configured, Ok(None)) {
        return None;
    }
    let mut parsed = copilot::Parsed::default();
    let mut notes = Vec::new();
    match configured {
        Ok(Some(key)) => {
            if let Some(root) = inputs.state_root() {
                match crate::go_usage::read_for(provider, root, &key) {
                    Read::Custody(failure, mode) => {
                        notes.extend(provider.key_note(failure, &key, mode));
                    }
                    Read::Cached(cache) => {
                        if cache.outcome != "ok" {
                            notes.push(copilot_failure_note(
                                &cache.outcome,
                                inputs.now.saturating_sub(cache.attempt_at),
                            ));
                        }
                        if let Some(sample) = cache.samples.last() {
                            parsed = copilot::parse(sample.body.as_bytes(), sample.at, inputs.now);
                        }
                    }
                    Read::Missing => {
                        notes
                            .push("github copilot: no usage fetched yet for this token".to_owned());
                    }
                }
            }
        }
        Err(why) => notes.push(format!("github copilot: {why}")),
        Ok(None) => return None,
    }
    notes.append(&mut parsed.notes);
    let cfg = crate::config::read_identity(inputs.global, inputs.local).ok()?;
    let profiles = cfg
        .profiles
        .iter()
        .filter_map(|(name, _)| {
            let command = cfg.command(name, inputs.home).ok()??;
            let route = resolved_tool(command.as_str()).adapter().quota.plan?;
            crate::tool::PlanRoute {
                prefix: "github-copilot/",
                ..route
            }
            .routes(command.as_str())
            .then(|| name.clone())
        })
        .collect();
    let account = Account {
        credits: if parsed.unlimited {
            Credits::Unlimited
        } else {
            Credits::Unreported
        },
        credits_observed_at: parsed.rows.first().and_then(|row| row.observed_at),
        ..Account::default()
    };
    Some(Group {
        profiles,
        tool: resolved_tool("opencode"),
        home: None,
        source: None,
        clients: vec![format!(
            "{COPILOT_SCOPE}{}",
            parsed.plan.as_deref().unwrap_or("unknown")
        )],
        rollout: None,
        owner: None,
        rows: if parsed.rows.is_empty() {
            vec![placeholder(Status::Unknown)]
        } else {
            parsed.rows
        },
        hint: None,
        summary: None,
        policy: Policy::new(None, account),
        notes,
        plan: None,
    })
}

fn copilot_failure_note(outcome: &str, age: i64) -> String {
    let class = match outcome {
        code if code == "unauthorized" || code.starts_with("unauthorized:") => {
            "token rejected (401): check copilot_token_file".to_owned()
        }
        "not-entitled" => "token rejected (403): check copilot_token_file".to_owned(),
        "throttled" => "the vendor throttled the quota call (429)".to_owned(),
        "timeout" => "the quota call hit its timeout".to_owned(),
        "transport" => "the quota call could not connect".to_owned(),
        "too-large" => "the answer passed its size cap".to_owned(),
        "malformed" => "the quota answer was unparseable".to_owned(),
        "redirected" => "the answer was a redirect, which is never followed".to_owned(),
        other => other
            .strip_prefix("status-")
            .filter(|code| code.len() == 3)
            .and_then(|code| code.parse::<u16>().ok())
            .filter(|code| (100..=599).contains(code))
            .map_or_else(
                || "unknown outcome".to_owned(),
                |code| format!("HTTP {code}"),
            ),
    };
    format!(
        "github copilot: last refresh {} failed — {class}",
        age_label(age.max(0))
    )
}

/// One `OpenCode` Go scope's group, its windows' history added to `series`.
fn go_group(scope: &Scope, inputs: &Inputs<'_>, series: &mut pace::SeriesSet) -> Group {
    let (observed, notes) = go_observed(scope, inputs);
    if let Some(source) = &scope.source_key {
        for held in observed.series {
            series.add(source, held);
        }
    }
    Group {
        profiles: scope.profiles.clone(),
        tool: scope.tool,
        home: scope.home.clone(),
        source: scope.source_key.clone(),
        clients: scope.clients.clone(),
        rollout: None,
        owner: None,
        rows: observed.rows,
        hint: scope.hint.clone(),
        summary: None,
        policy: Policy::new(scope.manual_resets, observed.account),
        notes: scope.notes.iter().cloned().chain(notes).collect(),
        plan: None,
    }
}

/// What an `OpenCode` Go scope says before its first fetch landed.
const GO_NOT_YET: &str = "opencode go: no usage fetched yet for this key";

/// One `OpenCode` Go scope's windows, read from the shared usage cache ONLY:
/// [`observe`] never reaches the network, whoever calls it.
fn go_observed(scope: &Scope, inputs: &Inputs<'_>) -> (Observed, Vec<String>) {
    let unknown = |notes: Vec<String>| {
        (
            Observed {
                rows: vec![placeholder(Status::Unknown)],
                account: Account::default(),
                series: Vec::new(),
            },
            notes,
        )
    };
    let (Some(key), Some(root)) = (scope.source.as_deref(), inputs.state_root()) else {
        return unknown(Vec::new());
    };
    let cached = match crate::go_usage::read(root, key) {
        crate::go_usage::Read::Custody(failure, mode) => {
            return unknown(failure.key_note(key, mode).into_iter().collect());
        }
        crate::go_usage::Read::Missing => return unknown(vec![GO_NOT_YET.to_owned()]),
        crate::go_usage::Read::Cached(cached) => cached,
    };
    let mut notes = Vec::new();
    if cached.outcome != "ok" {
        notes.push(go_failure_note(
            &cached.outcome,
            inputs.now.saturating_sub(cached.attempt_at),
        ));
    }
    let Some(newest) = cached.samples.last() else {
        notes.push(GO_NOT_YET.to_owned());
        return unknown(notes);
    };
    let parsed = go::parse(newest.body.as_bytes(), newest.at, inputs.now);
    notes.extend(parsed.notes);
    let series = go_series(&cached.samples, &parsed.rows, inputs.now);
    (
        Observed {
            rows: parsed.rows,
            account: Account::default(),
            series,
        },
        notes,
    )
}

/// The note a failed last attempt leaves beside the rows still shown.
fn go_failure_note(outcome: &str, age: i64) -> String {
    let class = match outcome {
        "unauthorized" => "key rejected (401): check opencode_go_key_file".to_owned(),
        "not-entitled" => "no Go subscription (403)".to_owned(),
        "throttled" => "the vendor throttled the usage call (429)".to_owned(),
        "timeout" => "the usage call hit its timeout".to_owned(),
        "transport" => "the usage call could not connect".to_owned(),
        "too-large" => "the answer passed its size cap".to_owned(),
        "malformed" => "the answer had no readable window".to_owned(),
        "redirected" => "the answer was a redirect, which is never followed".to_owned(),
        "unauthorized:api_key_expired" => {
            "key expired (401 api_key_expired): renew it in the OpenCode Console".to_owned()
        }
        other => other.strip_prefix("status-").map_or_else(
            || "unknown outcome".to_owned(),
            |code| format!("HTTP {code}"),
        ),
    };
    format!(
        "opencode go: last refresh {} failed — {class}",
        age_label(age.max(0))
    )
}

/// Each stated-length window's history, replayed from every kept answer
/// through the same parser, oldest first.
fn go_series(samples: &[go::Sample], newest: &[Row], now: i64) -> Vec<pace::Series> {
    newest
        .iter()
        .filter_map(|row| {
            let window_minutes = row.window_minutes?;
            let points = samples
                .iter()
                .filter_map(|sample| {
                    go::parse(sample.body.as_bytes(), sample.at, now)
                        .rows
                        .into_iter()
                        .find(|held| held.bucket == row.bucket)
                        .and_then(|held| {
                            Some(pace::Point {
                                observed_at: sample.at,
                                used: row_percent(&held)?,
                                resets_at: held.resets_at?,
                            })
                        })
                })
                .collect();
            Some(pace::Series {
                bucket: row.bucket.clone(),
                qualifier: row.qualifier.clone(),
                window_minutes,
                points,
            })
        })
        .collect()
}

/// Read the bounded local sources and return the quota view facts.
/// Configuration failure is informational in this surface: it becomes one
/// honest scope and cannot remove the dialog. Agy is asked through [`view_cached`] (up to 11 s).
pub(crate) fn view_of(inputs: &Inputs<'_>) -> Vec<view::Scope> {
    view_cached(inputs, &mut AgyCache::default())
}

/// How long one agy answer is held; below [`FRESH_SECS`], so a held answer still reads Fresh.
const AGY_TTL_SECS: i64 = 300;
const _: () = assert!(AGY_TTL_SECS < FRESH_SECS);

/// What an agy scope says while its first ask is out.
const AGY_ASKING: &str = "asking agy /quota";
const AGY_ABSENT: &str = "agy not found on PATH";

/// What the Muse scopes say when they hold no rows. The fixed words of a refusal
/// to ask or record live with their gap, in [`crate::muse_usage::Gap::words`].
const MUSE_UNUSABLE: &str = "muse_quota: expected on or off";
const MUSE_UNASKED: &str = "muse quota not asked yet — run ae quota";
const MUSE_NO_ALLOWANCE: &str = "muse exec reported no allowance";
const MUSE_TIMEOUT: &str = "muse exec passed its 90 s deadline";
const MUSE_FAILED: &str = "muse exec gave no usable answer";
/// The row beside a held usage limit: the full bar and the reset are what the vendor's
/// message implies, not a measured percentage, and the words say so.
const MUSE_LIMIT: &str = "muse usage limit reached (limit-derived, not a measured %)";

/// The agy leg, shared with the worker that asks it off the app's reader.
pub(crate) type AgyLeg = Arc<dyn Fn(&Path) -> AgyRun + Send + Sync>;

/// The agy answer held between dialog reads, and the leg that refreshes it: the
/// app's reader, re-reading every few seconds, holds one; the one-shot tmux dialog none.
pub(crate) struct AgyCache {
    leg: AgyLeg,
    held: Option<AgyHeld>,
    /// The last answer with a window, shown in place of a later attempt that has none.
    usable: Option<AgyHeld>,
}

/// An agy answer, the home it was asked for, and the epochs it was asked and completed at.
#[derive(Clone)]
struct AgyHeld {
    home: PathBuf,
    asked_at: i64,
    done_at: i64,
    answer: AgyRun,
}

/// What the agy scopes show: rows, the words beside them, and the epoch the attempt shown
/// completed at (none while the first ask is out).
type Shown = (Vec<Row>, Option<String>, Option<i64>);

impl AgyCache {
    pub(crate) fn new(leg: impl Fn(&Path) -> AgyRun + Send + Sync + 'static) -> Self {
        Self {
            leg: Arc::new(leg),
            held: None,
            usable: None,
        }
    }

    /// The leg, for a worker that asks it off the reader.
    pub(crate) fn leg(&self) -> AgyLeg {
        Arc::clone(&self.leg)
    }

    /// Whether `home` needs a new ask at `now`: nothing held for it, or the held ask is
    /// [`AGY_TTL_SECS`] old or from the future. A failure is held too, so a hung agy
    /// costs its deadline once per TTL.
    pub(crate) fn due(&self, home: &Path, now: i64) -> bool {
        !self.held.as_ref().is_some_and(|held| {
            held.home == home && (0..AGY_TTL_SECS).contains(&now.saturating_sub(held.asked_at))
        })
    }

    /// Hold an attempt for `home`; one with a window is also kept to show past later attempts.
    pub(crate) fn settle(&mut self, home: &Path, asked_at: i64, done_at: i64, answer: AgyRun) {
        let held = AgyHeld {
            home: home.to_owned(),
            asked_at,
            done_at,
            answer,
        };
        if windowed(&held.answer, done_at) {
            self.usable = Some(held.clone());
        }
        self.held = Some(held);
    }

    /// The answer for `home` and its completion epoch: the held one unless [`Self::due`],
    /// else a new call on this thread.
    fn answer(&mut self, home: &Path, now: i64) -> (AgyRun, i64) {
        if !self.due(home, now)
            && let Some(held) = &self.held
        {
            return (held.answer.clone(), held.done_at);
        }
        let (answer, done_at) = ask(&self.leg, home, now);
        self.settle(home, now, done_at, answer.clone());
        (answer, done_at)
    }

    /// What the agy scopes show for `home`: the last attempt; a kept answer with its own
    /// age and a note when the last attempt has no window; the asking words while `pending`
    /// and nothing is held.
    fn shown(&self, home: &Path, pending: bool) -> Option<Shown> {
        let mine = |held: &&AgyHeld| held.home == home;
        let Some(last) = self.held.as_ref().filter(mine) else {
            let asking = Some(AGY_ASKING.to_owned());
            return pending.then_some((vec![placeholder(Status::Unknown)], asking, None));
        };
        let kept = self.usable.as_ref().filter(mine);
        if let Some(kept) = kept.filter(|_| !windowed(&last.answer, last.done_at)) {
            let note = Some(kept_note(&last.answer, last.done_at));
            let rows = attempt_rows(&kept.answer, kept.done_at).0;
            return Some((rows, note, Some(last.done_at)));
        }
        let (rows, hint) = attempt_rows(&last.answer, last.done_at);
        Some((rows, hint.map(str::to_owned), Some(last.done_at)))
    }
}

impl Default for AgyCache {
    fn default() -> Self {
        Self::new(crate::transport::run_agy_quota)
    }
}

/// Run the agy `leg` for `home`, asked at `asked_at`: its answer and the epoch it completed
/// at. The app's reader calls this on a worker thread, never on its own.
pub(crate) fn ask(leg: &AgyLeg, home: &Path, asked_at: i64) -> (AgyRun, i64) {
    let started = Instant::now();
    let answer = leg(home);
    (answer, completed(asked_at, started.elapsed()))
}

/// [`view_of`] with the agy answer held in `cache`: the dialogs' one reach to the agy leg,
/// so [`observe`] and its readers stay spawn-free; the answer lands in these groups only.
pub(crate) fn view_cached(inputs: &Inputs<'_>, cache: &mut AgyCache) -> Vec<view::Scope> {
    let Ok(cfg) = crate::config::read_identity(inputs.global, inputs.local) else {
        return view::unavailable();
    };
    let scopes = configured_scopes_with(&cfg, inputs.home, &OptIns::of(inputs));
    let legs = agy_legs(&scopes);
    let shown = inputs.home.filter(|_| !legs.is_empty()).map(|home| {
        let (answer, done_at) = cache.answer(home, inputs.now);
        let (rows, hint) = attempt_rows(&answer, done_at);
        (rows, hint.map(str::to_owned), Some(done_at))
    });
    compose(inputs, scopes, &legs, shown)
}

/// [`view_cached`] that never runs the leg: the agy scopes show what `cache` holds, or the
/// asking words. `flying` says an ask is already out; the home returned is the one a new
/// ask is due for, which the caller runs off its own thread and settles into `cache`.
pub(crate) fn view_held(
    inputs: &Inputs<'_>,
    cache: &AgyCache,
    flying: bool,
) -> (Vec<view::Scope>, Option<PathBuf>) {
    let Ok(cfg) = crate::config::read_identity(inputs.global, inputs.local) else {
        return (view::unavailable(), None);
    };
    let scopes = configured_scopes_with(&cfg, inputs.home, &OptIns::of(inputs));
    let legs = agy_legs(&scopes);
    let home = inputs.home.filter(|_| !legs.is_empty());
    let due = home.filter(|home| !flying && cache.due(home, inputs.now));
    let shown = home.and_then(|home| cache.shown(home, flying || due.is_some()));
    (
        compose(inputs, scopes, &legs, shown),
        due.map(Path::to_path_buf),
    )
}

/// The view facts of `scopes` with the agy scopes showing `shown`, judged at the later of
/// the read and the instant the shown attempt completed: no row is judged before agy returned.
fn compose(
    inputs: &Inputs<'_>,
    scopes: Vec<Scope>,
    legs: &[Leg],
    shown: Option<Shown>,
) -> Vec<view::Scope> {
    let done_at = shown.as_ref().and_then(|(_, _, done_at)| *done_at);
    let now = done_at.map_or(inputs.now, |done_at| inputs.now.max(done_at));
    let muse = muse_held(inputs, &scopes, now);
    let (mut observation, series) = observe_scopes(&Inputs { now, ..*inputs }, scopes);
    if let Some((rows, hint, _)) = shown {
        replace_agy(&mut observation.groups, legs, rows, hint.as_deref());
    }
    if let Some((spoken, rows, hint)) = muse {
        replace_muse(&mut observation.groups, &spoken, rows, hint.as_deref());
    }
    // The same table `ae quota` draws its PACE column from, so one rate decides both.
    let pace = pace::PaceTable::build(&observation.groups, &series, observation.now);
    observation.quota_view_paced(&pace)
}

/// One attempt's rows, or a placeholder and its reason. A row with no windows still says
/// when agy was asked, so the dialog can age it.
fn attempt_rows(answer: &AgyRun, done_at: i64) -> (Vec<Row>, Option<&'static str>) {
    let (mut rows, hint) = agy_rows(answer, done_at)
        .unwrap_or_else(|| (vec![placeholder(Status::Unsupported)], Some(AGY_ABSENT)));
    for row in &mut rows {
        row.observed_at = row.observed_at.or(Some(done_at));
    }
    (rows, hint)
}

/// What the dialogs show of Muse: the held record, read and never asked for. A dialog
/// re-reads on a cadence of its own, so a paid ask from here would be a poll of the very
/// account it measures; `ae quota` is the one asker.
fn muse_held(
    inputs: &Inputs<'_>,
    scopes: &[Scope],
    now: i64,
) -> Option<(Vec<Leg>, Vec<Row>, Option<String>)> {
    let legs = muse_legs(scopes);
    let config_home = muse_home(scopes)?;
    let held = inputs
        .state_root()
        .map(|root| crate::muse_usage::read(root, &config_home));
    let (rows, hint) = match held {
        Some(crate::muse_usage::Read::Cached(record)) => muse_rows(Some(&record), None, now),
        Some(crate::muse_usage::Read::Missing) => muse_rows(None, None, now),
        None => muse_rows(None, Some(crate::muse_usage::Gap::NoStore), now),
    };
    Some((legs, rows, hint))
}

/// The config home of the scope the Muse leg speaks for, when the config opted in.
fn muse_home(scopes: &[Scope]) -> Option<PathBuf> {
    scopes
        .iter()
        .find(|scope| scope.leg && scope.tool == ToolKind::Muse)
        .and_then(|scope| scope.home.clone())
}

/// Which way a paid attempt left no answer, in words an operator can act on; the leading
/// `no usable answer` is every failure's, the class follows in brackets.
fn muse_failure_words(outcome: muse::Outcome) -> String {
    match outcome {
        muse::Outcome::Exited(code) => format!("{MUSE_FAILED} (exited {code})"),
        muse::Outcome::NotUtf8 => format!("{MUSE_FAILED} (output not UTF-8)"),
        muse::Outcome::Unparsed => format!("{MUSE_FAILED} (output not parsed)"),
        muse::Outcome::TooLarge => format!("{MUSE_FAILED} (output over 256 KiB)"),
        muse::Outcome::Done
        | muse::Outcome::TimedOut
        | muse::Outcome::Failed
        | muse::Outcome::Limit => MUSE_FAILED.to_owned(),
    }
}

/// The rows and words the Muse scope shows: a record's windows, or one placeholder and the
/// reason there are none. `gap` rides beside held rows, so a paid answer that could not be
/// recorded still shows what it cost.
fn muse_rows(
    record: Option<&muse::Cached>,
    gap: Option<crate::muse_usage::Gap>,
    now: i64,
) -> (Vec<Row>, Option<String>) {
    use crate::muse_usage::Gap;
    let Some(record) = record else {
        return match gap {
            Some(Gap::Absent) => (
                vec![placeholder(Status::Unsupported)],
                Some(Gap::Absent.words().to_owned()),
            ),
            Some(Gap::Busy) => (
                vec![placeholder(Status::Unknown)],
                Some(Gap::Busy.words().to_owned()),
            ),
            Some(gap) => (
                vec![placeholder(Status::ReadError)],
                Some(gap.words().to_owned()),
            ),
            None => (
                vec![placeholder(Status::Unknown)],
                Some(MUSE_UNASKED.to_owned()),
            ),
        };
    };
    let (status, words) = match record.outcome {
        muse::Outcome::Done => {
            let rows = record.rows(now);
            if !rows.is_empty() {
                return (rows, gap.map(|gap| gap.words().to_owned()));
            }
            (Status::Unknown, MUSE_NO_ALLOWANCE.to_owned())
        }
        muse::Outcome::Limit => {
            let rows = record.rows(now);
            if !rows.is_empty() {
                let words = gap.map_or_else(
                    || MUSE_LIMIT.to_owned(),
                    |gap| format!("{MUSE_LIMIT}; {}", gap.words()),
                );
                return (rows, Some(words));
            }
            (Status::ReadError, MUSE_FAILED.to_owned())
        }
        muse::Outcome::TimedOut => (Status::Truncated, MUSE_TIMEOUT.to_owned()),
        outcome => (Status::ReadError, muse_failure_words(outcome)),
    };
    // What the call answered, then why the next `ae quota` may pay again.
    let words = gap.map_or_else(|| words.clone(), |gap| format!("{words}; {}", gap.words()));
    (vec![placeholder(status)], Some(words))
}

/// Whether an answer carries a window: only such an answer is kept past later attempts.
fn windowed(answer: &AgyRun, at: i64) -> bool {
    matches!(answer, AgyRun::Done { text }
        if agy::parse(text.as_bytes(), at).is_ok_and(|rows| !rows.is_empty()))
}

/// The words beside a kept answer: a failed refresh says so with its reason; a missing or
/// too old agy, or a report with no windows, is said bare.
fn kept_note(answer: &AgyRun, at: i64) -> String {
    match (answer, agy_rows(answer, at)) {
        (_, None) => AGY_ABSENT.to_owned(),
        (_, Some((_, None))) => "agy /quota reported no windows".to_owned(),
        (AgyRun::TooOld, Some((_, Some(hint)))) => hint.to_owned(),
        (_, Some((_, Some(hint)))) => format!("last refresh failed — {hint}"),
    }
}

/// Read configured client scopes and print the quota table.
///
/// It opens client cache files and writes only to the supplied streams. The
/// one process it may start is agy, once, when a scope's agy command keeps the
/// caller's HOME ([`crate::transport::run_agy_quota`]); every row is then
/// judged at the instant that call returned. [`observe`] never starts it.
/// Once the GLOBAL config says `muse_quota = on` it may also spend one model
/// call of Muse's account, through the record every ae process shares: that is
/// the only paid entry, and the dialogs and the watchdog only read the record.
///
/// # Errors
///
/// Returns an I/O error only when the supplied output stream cannot be written.
pub fn run(inputs: &Inputs<'_>, out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    run_as(inputs, Format::Table, out, err)
}

/// How `ae quota` prints its one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// The operator table.
    #[default]
    Table,
    /// One JSON document with the same rows (`--json`).
    Json,
}

/// Parse `ae quota [--json]` without reading the world.
///
/// # Errors
///
/// Returns the first duplicate or unknown word.
pub fn parse_args(tail: &[String]) -> Result<Format, String> {
    let mut format = Format::Table;
    for word in tail {
        match word.as_str() {
            "--json" if format == Format::Table => format = Format::Json,
            _ => return Err(word.clone()),
        }
    }
    Ok(format)
}

/// [`run`] printing `format`: the same one observation either way.
///
/// # Errors
///
/// Returns an I/O error only when the supplied output stream cannot be written.
pub fn run_as(
    inputs: &Inputs<'_>,
    format: Format,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    run_with(inputs, format, out, err, crate::transport::run_agy_quota)
}

/// [`run_as`] with the agy leg passed in, so a test can count its calls.
fn run_with(
    inputs: &Inputs<'_>,
    format: Format,
    out: &mut impl Write,
    err: &mut impl Write,
    leg: impl FnOnce(&Path) -> AgyRun,
) -> crate::Result<u8> {
    // ONE config read: the scopes the leg was judged on are the scopes folded.
    let scopes = match crate::config::read_identity(inputs.global, inputs.local) {
        Ok(cfg) => configured_scopes_with(&cfg, inputs.home, &OptIns::of(inputs)),
        Err(error) => {
            writeln!(err, "{error}")?;
            return Ok(1);
        }
    };
    let legs = agy_legs(&scopes);
    let started = Instant::now();
    let go = go_refresh(inputs, &scopes);
    let copilot = copilot_refresh(inputs);
    let answer = inputs.home.filter(|_| !legs.is_empty()).map(leg);
    let muse_ask = ask_muse(inputs, &scopes);
    let fetched = go.as_ref().is_some_and(|(_, refreshed)| refreshed.fetched);
    let paid = muse_ask
        .as_ref()
        .is_some_and(|(_, refreshed, _)| refreshed.asked);
    let now = if answer.is_some()
        || fetched
        || paid
        || copilot.as_ref().is_some_and(|(_, result)| result.fetched)
    {
        completed(inputs.now, started.elapsed())
    } else {
        inputs.now
    };
    let (mut observation, series) = observe_scopes(&Inputs { now, ..*inputs }, scopes);
    if let Some(answer) = answer
        && let Some((rows, hint)) = agy_rows(&answer, now)
    {
        replace_agy(&mut observation.groups, &legs, rows.clone(), hint);
        replace_agy(&mut observation.rendered, &legs, rows, hint);
    }
    if let Some((spoken, refreshed, held)) = muse_ask {
        // The record this very ask built, else the held one: never a re-read of a
        // record that could not be published.
        let record = refreshed.answer.as_ref().or(held.as_ref());
        let (rows, hint) = muse_rows(record, refreshed.gap, now);
        replace_muse(
            &mut observation.groups,
            &spoken,
            rows.clone(),
            hint.as_deref(),
        );
        replace_muse(&mut observation.rendered, &spoken, rows, hint.as_deref());
    }
    if let Some((key, refreshed)) = &go {
        note_go_key(&mut observation.groups, key, *refreshed);
        note_go_key(&mut observation.rendered, key, *refreshed);
    }
    if let Some((key, refreshed)) = copilot {
        for groups in [&mut observation.groups, &mut observation.rendered] {
            for group in groups.iter_mut().filter(|group| is_copilot(group)) {
                if let Some(note) = refreshed.outcome.and_then(|failure| {
                    crate::go_usage::Provider::Copilot.key_note(failure, &key, None)
                }) && !group
                    .notes
                    .iter()
                    .any(|held| held.starts_with("github copilot: token file"))
                {
                    group.notes.push(note);
                }
            }
        }
    }
    let pace = pace::PaceTable::build(&observation.groups, &series, observation.now);
    let groups = observation.ordered_groups(&observation.rendered);
    let home = observation.home.as_deref().or(inputs.home);
    match format {
        Format::Table => write!(
            out,
            "{}",
            render_with_pace(groups, &pace, home, observation.now)
        )?,
        Format::Json => writeln!(
            out,
            "{}",
            machine::document(groups, &pace, home, observation.now).render()
        )?,
    }
    Ok(0)
}

fn copilot_refresh(inputs: &Inputs<'_>) -> Option<(PathBuf, crate::go_usage::Refreshed)> {
    if !configured_quota_on(inputs.global, inputs.local) {
        return None;
    }
    let key = crate::go_usage::configured_for(
        crate::go_usage::Provider::Copilot,
        inputs.global,
        inputs.home,
    )
    .ok()??;
    let result = crate::go_usage::refresh_for(
        crate::go_usage::Provider::Copilot,
        inputs.state_root()?,
        &key,
        inputs.now,
    );
    Some((key, result))
}

/// `ae quota`'s one Muse ask: the scopes the leg speaks for, what the refresh did, and the
/// record held after it when the refresh made no ask of its own. `None` unless the global
/// config opted in, quota is on and a scope kept the caller's HOME.
fn ask_muse(
    inputs: &Inputs<'_>,
    scopes: &[Scope],
) -> Option<(Vec<Leg>, crate::muse_usage::Refreshed, Option<muse::Cached>)> {
    if !configured_quota_on(inputs.global, inputs.local) {
        return None;
    }
    let legs = muse_legs(scopes);
    let config_home = muse_home(scopes)?;
    let (Some(root), Some(home)) = (inputs.state_root(), inputs.home) else {
        let refreshed = crate::muse_usage::Refreshed {
            asked: false,
            gap: Some(crate::muse_usage::Gap::NoStore),
            answer: None,
        };
        return Some((legs, refreshed, None));
    };
    let refreshed = crate::muse_usage::refresh(root, &config_home, home, inputs.now);
    // The paid answer, else the held record: a failed publication is never read back.
    let held = if refreshed.answer.is_none() {
        match crate::muse_usage::read(root, &config_home) {
            crate::muse_usage::Read::Cached(record) => Some(record),
            crate::muse_usage::Read::Missing => None,
        }
    } else {
        None
    };
    Some((legs, refreshed, held))
}

/// `ae quota`'s one inline `OpenCode` Go refresh, and the key it was made for:
/// only with quota on, a state root to keep the shared cache in, and a
/// Go-scoped profile whose key the global config names. The fleet bound is
/// the cache's, so this asks the vendor only when no process did lately.
fn go_refresh(
    inputs: &Inputs<'_>,
    scopes: &[Scope],
) -> Option<(PathBuf, crate::go_usage::Refreshed)> {
    let root = inputs.state_root()?;
    if !configured_quota_on(inputs.global, inputs.local) {
        return None;
    }
    let key = go_key_of(scopes)?;
    let refreshed = crate::go_usage::refresh(root, &key, inputs.now);
    Some((key, refreshed))
}

/// A key failure only the read of the key itself finds — its metadata was
/// fine, so the cache read named nothing — told on every Go scope.
fn note_go_key(groups: &mut [Group], key: &Path, refreshed: crate::go_usage::Refreshed) {
    let Some(note) = refreshed
        .outcome
        .and_then(|failure| failure.key_note(key, None))
    else {
        return;
    };
    for group in groups
        .iter_mut()
        .filter(|group| group.tool.adapter().quota.plan.is_some() && group.source.is_some())
    {
        if !group
            .notes
            .iter()
            .any(|held| held.starts_with("opencode go: key file"))
        {
            group.notes.push(note.clone());
        }
    }
}

/// The key file of the first Go-scoped profile, when the opt-in named one.
fn go_key_of(scopes: &[Scope]) -> Option<PathBuf> {
    scopes
        .iter()
        .find(|scope| scope.go)
        .and_then(|scope| scope.source.clone())
}

/// The key file a watchdog pass may refresh the shared Go cache for: the
/// global opt-in AND at least one Go-scoped profile, or `None`.
pub(crate) fn go_key(inputs: &Inputs<'_>) -> Option<PathBuf> {
    let cfg = crate::config::read_identity(inputs.global, inputs.local).ok()?;
    go_key_of(&configured_scopes_with(
        &cfg,
        inputs.home,
        &OptIns::of(inputs),
    ))
}

/// Whether the configured `[workspace] quota` lets ae act on vendor quota.
pub(crate) fn configured_quota_on(global: Option<&Path>, local: Option<&Path>) -> bool {
    let configured = crate::config::read_workspace_keys(global, local, &["quota"])
        .into_iter()
        .next()
        .flatten();
    crate::config::resolve_quota_aware(None, configured.as_deref())
}

/// `now` plus `elapsed` rounded UP to whole seconds: no row is judged before agy returned.
fn completed(now: i64, elapsed: Duration) -> i64 {
    let seconds = elapsed.as_nanos().div_ceil(1_000_000_000);
    now.saturating_add(i64::try_from(seconds).unwrap_or(i64::MAX))
}

/// One scope the agy leg answers for: its profiles and its clients.
type Leg = (Vec<String>, Vec<String>);

/// What the scopes agy speaks for are told: its rows and the hint beside them. `None` is no
/// agy at all, which each surface words for itself.
fn agy_rows(answer: &AgyRun, now: i64) -> Option<(Vec<Row>, Option<&'static str>)> {
    const UNREADABLE: &str = "agy /quota gave no usable answer";
    let (status, hint) = match answer {
        AgyRun::Absent => return None,
        AgyRun::Done { text } => match agy::parse(text.as_bytes(), now) {
            Ok(rows) if !rows.is_empty() => return Some((rows, None)),
            Ok(_) => (Status::Unknown, None),
            Err(_) => (Status::ReadError, Some(UNREADABLE)),
        },
        AgyRun::TooOld => (Status::Unsupported, Some("agy 1.1.11+ required for /quota")),
        AgyRun::TimedOut => (
            Status::Truncated,
            Some("agy /quota passed its 8 s deadline"),
        ),
        AgyRun::Failed => (Status::ReadError, Some(UNREADABLE)),
    };
    Some((vec![placeholder(status)], hint))
}

/// Every scope the agy leg may speak for: its profiles and its clients.
fn agy_legs(scopes: &[Scope]) -> Vec<Leg> {
    legs_of(scopes, ToolKind::quota_is_asked)
}

/// Every scope the Muse leg may speak for; empty unless the global config opted in.
fn muse_legs(scopes: &[Scope]) -> Vec<Leg> {
    legs_of(scopes, |tool| tool == ToolKind::Muse)
}

fn legs_of(scopes: &[Scope], is: impl Fn(ToolKind) -> bool) -> Vec<Leg> {
    scopes
        .iter()
        .filter(|scope| scope.leg && is(scope.tool))
        .map(|scope| (scope.profiles.clone(), scope.clients.clone()))
        .collect()
}

/// agy's one answer replaces the placeholder of every scope it speaks for, as ONE group (the
/// leg asked one login).
fn replace_agy(groups: &mut Vec<Group>, legs: &[Leg], rows: Vec<Row>, hint: Option<&str>) {
    replace_leg(|tool| tool == ToolKind::Agy, groups, legs, rows, hint);
}

/// Muse's one answer replaces the placeholder of the scope it speaks for: one login.
fn replace_muse(groups: &mut Vec<Group>, legs: &[Leg], rows: Vec<Row>, hint: Option<&str>) {
    replace_leg(|tool| tool == ToolKind::Muse, groups, legs, rows, hint);
}

fn replace_leg(
    is: impl Fn(ToolKind) -> bool,
    groups: &mut Vec<Group>,
    legs: &[Leg],
    rows: Vec<Row>,
    hint: Option<&str>,
) {
    let spoken = |group: &Group| {
        is(group.tool) && legs.contains(&(group.profiles.clone(), group.clients.clone()))
    };
    let first = groups.iter().position(spoken);
    let spoke = groups
        .extract_if(.., |group| spoken(group))
        .reduce(|mut merged, group| {
            merged.profiles.extend(group.profiles);
            merged.clients.extend(group.clients);
            merged
        });
    if let (Some(first), Some(mut merged)) = (first, spoke) {
        let mut seen = std::collections::BTreeSet::new();
        merged.clients.retain(|client| seen.insert(client.clone()));
        (merged.rows, merged.hint) = (rows, hint.map(str::to_owned));
        groups.insert(first, merged);
    }
}

/// [`configured_scopes_with`] with no `OpenCode` Go opt-in.
#[cfg(test)]
fn configured_scopes(cfg: &crate::config::IdentityConfig, home: Option<&Path>) -> Vec<Scope> {
    configured_scopes_with(
        cfg,
        home,
        &OptIns {
            go: GoKeyConfig::Off,
            muse: crate::muse_usage::Setting::Off,
        },
    )
}

/// What the GLOBAL config says about the `OpenCode` Go key file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GoKeyConfig {
    /// No opt-in: every opencode profile is the unsupported scope it was.
    Off,
    /// The expanded key path.
    Key(PathBuf),
    /// An opt-in ae cannot use, and why.
    Unusable(String),
}

/// The opt-ins the GLOBAL config makes. Each turns a tool that reports no local
/// quota into a scope ae may read, and none is read from a project config.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OptIns {
    go: GoKeyConfig,
    muse: crate::muse_usage::Setting,
}

impl OptIns {
    fn of(inputs: &Inputs<'_>) -> Self {
        Self {
            go: GoKeyConfig::of(inputs),
            muse: crate::muse_usage::setting(inputs.global),
        }
    }
}

impl Inputs<'_> {
    /// The state root these inputs name, as `<root>/sessions`'s parent —
    /// never the process's own state root, so a reader handed a scratch tree
    /// reads that tree's cache and nothing else.
    fn state_root(&self) -> Option<&Path> {
        self.sessions.and_then(Path::parent)
    }
}

impl GoKeyConfig {
    fn of(inputs: &Inputs<'_>) -> Self {
        match crate::go_usage::configured_key(inputs.global, inputs.home) {
            Ok(None) => Self::Off,
            Ok(Some(path)) => Self::Key(path),
            Err(why) => Self::Unusable(why),
        }
    }
}

/// A Go scope's source, correlation key and hint. ONE Go key is one Go
/// account, so its canonical path is the identity every Go profile shares,
/// whatever client label or opencode home it names. Every other opencode
/// profile keeps no source, as before.
fn go_identity(go_key: &GoKeyConfig) -> (Option<PathBuf>, Option<PathBuf>, Option<String>) {
    match go_key {
        GoKeyConfig::Key(path) => (
            Some(path.clone()),
            Some(canonical_source(path.clone()).unwrap_or_else(|_| path.clone())),
            None,
        ),
        GoKeyConfig::Unusable(why) => (
            None,
            None,
            Some(format!("{}: {why}", crate::go_usage::KEY_SETTING)),
        ),
        GoKeyConfig::Off => (None, None, None),
    }
}

/// Every quota scope the config declares, with the `OpenCode` Go opt-in the
/// global config makes.
///
/// TWO discoveries, one scope table. Profiles come first, because a named
/// profile is what the operator usually reads a row by. Then every `[clients]`
/// row on its own: a client no profile names is still an ACCOUNT, its
/// `manual_resets` is still a declaration, and the operator decides whether to
/// launch a seat against it from exactly this table.
///
/// Correlation is by PROVEN identity only — one canonical vendor source is one
/// account, whatever labels reach it. A discovery that RESOLVED NO SOURCE is
/// never correlated with anything, because "could not resolve" and "no
/// distinguishing identity" are different facts and only the second licenses a
/// merge. Two refusals we cannot show belong to one account are two rows: that
/// is the honest rendering, and collapsing them would assert what we do not
/// know. The declaration that made a false merge tempting is counted at the
/// DECLARATION instead, by [`count_declarations`].
fn configured_scopes_with(
    cfg: &crate::config::IdentityConfig,
    home: Option<&Path>,
    opt: &OptIns,
) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for (profile, raw) in &cfg.profiles {
        match cfg.command(profile, home) {
            Ok(Some(resolved)) => {
                absorb_resolved(
                    &mut scopes,
                    cfg,
                    home,
                    opt,
                    Some(profile.as_str()),
                    &resolved,
                );
            }
            Ok(None) => scopes.push(unresolved_scope(cfg, profile, raw, None)),
            Err(error) => scopes.push(unresolved_scope(cfg, profile, raw, Some(&error))),
        }
    }
    // Where each `[clients]` row's OWN discovery landed. The declaration belongs
    // to that row, so this is the only scope entitled to spend it.
    let mut owned: Vec<(&str, usize)> = Vec::new();
    for (label, _) in &cfg.clients {
        let index = match cfg.client_command(label, home) {
            Ok(Some(resolved)) => absorb_resolved(&mut scopes, cfg, home, opt, None, &resolved),
            // The label came from `cfg.clients`, so it always resolves to a row.
            Ok(None) => continue,
            Err(error) => {
                scopes.push(unresolved_client_scope(cfg, label, &error));
                scopes.len() - 1
            }
        };
        owned.push((label.as_str(), index));
    }
    count_declarations(&mut scopes, cfg, &owned);
    if opt.muse == crate::muse_usage::Setting::Unusable {
        for scope in scopes
            .iter_mut()
            .filter(|scope| scope.tool == ToolKind::Muse)
        {
            push_note(scope, Some(MUSE_UNUSABLE.to_owned()));
        }
    }
    scopes
}

/// Count each `[clients]` row's declaration on exactly ONE scope, or on none.
///
/// A declaration belongs to a LABEL, and a label has exactly one by
/// construction, so it may be spent once however many scopes that label
/// reached. The scope entitled to it is the one the `[clients]` row's OWN
/// discovery produced, and only when that discovery PROVED a source.
///
/// Not merely a stable choice — an ACCOUNT choice. A declared reset INCREASES
/// apparent headroom, so handing it to a scope the label reached by some other
/// route grants headroom to an arbitrary account under ambiguous attribution.
/// `[profiles] p = HOME=/other cc` is exactly that: visited first, it would
/// take the count for an account the operator never declared it against. When
/// the owning discovery proved nothing there is no entitled scope, so the
/// count is spent NOWHERE and the ambiguity is reported. Every ambiguity
/// resolves toward LESS apparent headroom.
fn count_declarations(
    scopes: &mut [Scope],
    cfg: &crate::config::IdentityConfig,
    owned: &[(&str, usize)],
) {
    for (label, _) in &cfg.clients {
        let declaration = declaration_for(cfg, Some(label));
        let carrying: Vec<usize> = scopes
            .iter()
            .enumerate()
            .filter(|(_, scope)| scope.accounts.iter().any(|account| account == label))
            .map(|(index, _)| index)
            .collect();
        let Some(first) = carrying.first().copied() else {
            continue;
        };
        // Why a declared value was unusable describes the row the operator
        // wrote, not an account, so it is said whatever the attribution is.
        if let Some(note) = declaration.note.clone()
            && let Some(scope) = scopes.get_mut(first)
        {
            push_note(scope, Some(note));
        }
        let Some(count) = declaration.manual_resets else {
            continue;
        };
        let entitled = owned
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, index)| *index)
            .filter(|index| {
                scopes
                    .get(*index)
                    .is_some_and(|scope| scope.source_key.is_some())
            });
        let Some(index) = entitled else {
            let Some(scope) = scopes.get_mut(first) else {
                continue;
            };
            push_note(
                scope,
                Some(format!(
                    "client '{label}' declares manual_resets={count}, but its own scope proved no source, so no scope claims the count and every one uses its raw windows"
                )),
            );
            continue;
        };
        let spread = carrying.len() > 1;
        let Some(scope) = scopes.get_mut(index) else {
            continue;
        };
        merge_declaration(
            scope,
            &Declaration {
                manual_resets: Some(count),
                note: None,
            },
        );
        if spread {
            push_note(
                scope,
                Some(format!(
                    "client '{label}' could not be shown as one account, so its manual_resets is counted on this scope only; the others use their raw windows"
                )),
            );
        }
    }
}

fn push_note(scope: &mut Scope, note: Option<String>) {
    if let Some(note) = note
        && !scope.notes.contains(&note)
    {
        scope.notes.push(note);
    }
}

/// Fold one resolved command into the scope table.
///
/// `owner` is the profile that resolved it, or `None` for a bare `[clients]`
/// row. A `None` owner contributes no profile name to the scope: it carries
/// the account, not a way to launch it.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered derivation of a scope's identity; each override reads best beside the facts it replaces"
)]
fn absorb_resolved(
    scopes: &mut Vec<Scope>,
    cfg: &crate::config::IdentityConfig,
    home: Option<&Path>,
    opt: &OptIns,
    owner: Option<&str>,
    resolved: &crate::config::ResolvedCommand,
) -> usize {
    let tool = resolved_tool(resolved.as_str());
    let account = resolved.client_label();
    let client = account.and_then(|label| displayed_client(cfg, label, tool));
    if let Some(variable) = word_expansion_dependency(resolved, home) {
        scopes.push(unknown_scope(
            owner,
            account,
            tool,
            client,
            Some(format!("depends on pane variable {variable}")),
        ));
        return scopes.len() - 1;
    }
    let unknown_variable = std::cell::RefCell::new(None);
    let account_variable = tool.adapter().config_home_env;
    let lookup = |name: &str| {
        if name == "HOME" {
            return home.map(|path| path.display().to_string());
        }
        if account_variable == Some(name) {
            return None;
        }
        let mut unknown = unknown_variable.borrow_mut();
        if unknown.is_none() {
            *unknown = Some(name.to_owned());
        }
        None
    };
    let resolution = crate::launch_cmd::config_home_resolution(resolved, tool, &lookup);
    // Read `unknown_variable` after both readers: agy's resolution alone never asks.
    let asks = tool == ToolKind::Agy
        || (tool == ToolKind::Muse && opt.muse == crate::muse_usage::Setting::On);
    let leg = asks && crate::launch_cmd::inherits_home(resolved, &lookup);
    if let Some(variable) = unknown_variable.into_inner() {
        scopes.push(unknown_scope(
            owner,
            account,
            tool,
            client,
            Some(format!("depends on pane variable {variable}")),
        ));
        return scopes.len() - 1;
    }
    let paths = match resolved_scope_paths(tool, &resolution, home) {
        Ok(paths) => paths,
        Err(error) => {
            scopes.push(unknown_scope(owner, account, tool, client, Some(error)));
            return scopes.len() - 1;
        }
    };
    let ScopePaths {
        home: config_home,
        source,
        source_key,
        hint,
    } = paths;
    let leg = leg && config_home.is_some();
    let go = opt.go != GoKeyConfig::Off
        && tool
            .adapter()
            .quota
            .plan
            .is_some_and(|plan| plan.routes(resolved.as_str()));
    let (source, source_key, hint) = if go {
        go_identity(&opt.go)
    } else if leg && tool == ToolKind::Muse {
        // One keychain login is one account: its config home is the identity
        // every Muse profile shares, and the declaration a client row makes.
        (config_home.clone(), config_home.clone(), hint)
    } else {
        (source, source_key, hint)
    };
    // A PROVEN source is the whole of the correlation. `source_key: None` means
    // this discovery resolved none, not that it has no distinguishing identity,
    // and matching two of those would join accounts on the absence of evidence
    // — the same defect, through the branch that shipped it. An unsupported
    // tool has no source by construction, so its labels are separate scopes
    // too: that is what ae can honestly say about them.
    let held = scopes.iter().position(|scope| {
        scope.source_key.is_some()
            && scope.tool == tool
            && scope.source_key == source_key
            && scope.hint == hint
    });
    let Some(index) = held.and_then(|index| scopes.get_mut(index).map(|scope| (index, scope)))
    else {
        scopes.push(Scope {
            tool,
            home: config_home,
            source,
            source_key,
            profiles: owner.map(str::to_owned).into_iter().collect(),
            configured_profiles: owner.map(str::to_owned).into_iter().collect(),
            clients: client.into_iter().collect(),
            accounts: account.map(str::to_owned).into_iter().collect(),
            rollouts: Vec::new(),
            hint,
            manual_resets: None,
            notes: Vec::new(),
            leg,
            go,
        });
        return scopes.len() - 1;
    };
    let (index, scope) = index;
    if let Some(profile) = owner {
        scope.profiles.push(profile.to_owned());
        scope.configured_profiles.push(profile.to_owned());
    }
    if let Some(client) = client
        && !scope.clients.contains(&client)
    {
        scope.clients.push(client);
    }
    if let Some(account) = account
        && !scope.accounts.iter().any(|held| held == account)
    {
        scope.accounts.push(account.to_owned());
    }
    index
}

/// The scope a profile gets when its own command never resolved to a client.
fn unresolved_scope(
    cfg: &crate::config::IdentityConfig,
    profile: &str,
    raw: &str,
    error: Option<&crate::config::ConfigError>,
) -> Scope {
    // No error means the profile never named a client at all, so there is no
    // account to attribute this row to and none is guessed: it stays its own.
    let Some(error) = error else {
        return unknown_scope(Some(profile), None, resolved_tool(""), None, None);
    };
    let label = config_error_client(error);
    let tool = label
        .and_then(|label| cfg.client(label))
        .map_or_else(|| resolved_tool(raw), |client| client.tool);
    unknown_scope(
        Some(profile),
        label,
        tool,
        label.and_then(|label| displayed_client(cfg, label, tool)),
        Some(error.to_string()),
    )
}

/// The scope a bare `[clients]` row gets when its own expansion refused.
///
/// A declared account that will not resolve is a fact the operator needs, so
/// it is reported as `unknown` with the refusal, exactly as an unresolvable
/// profile is — never dropped.
fn unresolved_client_scope(
    cfg: &crate::config::IdentityConfig,
    label: &str,
    error: &crate::config::ConfigError,
) -> Scope {
    let tool = cfg
        .client(label)
        .map_or_else(|| resolved_tool(""), |client| client.tool);
    unknown_scope(
        None,
        Some(label),
        tool,
        displayed_client(cfg, label, tool),
        Some(error.to_string()),
    )
}

fn word_expansion_dependency(
    command: &crate::config::ResolvedCommand,
    home: Option<&Path>,
) -> Option<String> {
    let unknown_variable = std::cell::RefCell::new(None);
    let _ = crate::words::split_words(command.as_str(), &|name| {
        if name == "HOME" {
            return home.map(|path| path.display().to_string());
        }
        let mut unknown = unknown_variable.borrow_mut();
        if unknown.is_none() {
            *unknown = Some(name.to_owned());
        }
        None
    });
    unknown_variable.into_inner()
}

fn resolved_tool(command: &str) -> ToolKind {
    crate::launch_cmd::lex_simple_command(command)
        .map_or_else(|_| ToolKind::from_binary_name(""), |parsed| parsed.tool())
}

fn config_error_client(error: &crate::config::ConfigError) -> Option<&str> {
    match error {
        crate::config::ConfigError::ClientEnvConflict { client, .. }
        | crate::config::ConfigError::ClientHome { client, .. } => Some(client),
        _ => None,
    }
}

fn displayed_client(
    cfg: &crate::config::IdentityConfig,
    label: &str,
    tool: ToolKind,
) -> Option<String> {
    let client = cfg.client(label)?;
    let default_alias =
        label == tool.as_str() && client.executable == label && client.config_home.is_none();
    (!default_alias).then(|| label.to_owned())
}

fn resolved_scope_paths(
    tool: ToolKind,
    resolution: &crate::launch_cmd::ConfigHomeResolution,
    operator_home: Option<&Path>,
) -> Result<ScopePaths, String> {
    let canonical_home = crate::run::canonical_config_home(&resolution.home)?;
    let (home, mut hint) = match canonical_home {
        crate::launch_cmd::Resolved::Path(path) => (Some(path), None),
        crate::launch_cmd::Resolved::Absent => (None, None),
        crate::launch_cmd::Resolved::Unknown(reason) => (None, Some(reason)),
    };
    if matches!(
        tool.adapter().quota.source,
        QuotaSource::Unsupported | QuotaSource::GrokLog
    ) {
        let fallback = tool
            .adapter()
            .quota
            .default_home
            .and_then(|name| operator_home.map(|base| base.join(name)));
        let fallback = match fallback {
            Some(path) => {
                match crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(path))? {
                    crate::launch_cmd::Resolved::Path(path) => Some(path),
                    crate::launch_cmd::Resolved::Absent
                    | crate::launch_cmd::Resolved::Unknown(_) => None,
                }
            }
            None => None,
        };
        // Grok has no config-home variable, so its log is only ever under the
        // default home: one HOME, one source, one account.
        let source = (tool.adapter().quota.source == QuotaSource::GrokLog)
            .then(|| fallback.as_ref().map(|grok| grok.join(GROK_LOG)))
            .flatten();
        let source_key = source.clone().map(canonical_source).transpose()?;
        return Ok(ScopePaths {
            home: fallback,
            source,
            source_key,
            hint,
        });
    }
    let source = match tool.adapter().quota.source {
        QuotaSource::ClaudeCache => {
            if let (false, crate::launch_cmd::Resolved::Unknown(reason)) =
                (resolution.explicit, &resolution.base)
                && hint.is_none()
            {
                hint = Some(reason.clone());
            }
            claude_source(resolution.explicit, home.as_deref(), &resolution.base)?
        }
        QuotaSource::CodexRollouts => home
            .as_ref()
            .map(|config_home| config_home.join("sessions")),
        QuotaSource::GrokLog | QuotaSource::Unsupported => None,
    };
    let source_key = source.clone().map(canonical_source).transpose()?;
    Ok(ScopePaths {
        home,
        source,
        source_key,
        hint,
    })
}

fn canonical_source(path: PathBuf) -> Result<PathBuf, String> {
    match crate::run::canonical_config_home(&crate::launch_cmd::Resolved::Path(path))? {
        crate::launch_cmd::Resolved::Path(path) => Ok(path),
        crate::launch_cmd::Resolved::Absent | crate::launch_cmd::Resolved::Unknown(_) => {
            Err("quota source did not resolve to a path".to_owned())
        }
    }
}

/// The RAW `.claude.json` a Claude scope reads: under its canonical config
/// home when the home is explicit, else under its canonical effective HOME.
/// The cache read opens exactly this path, so its own link refusal stands;
/// only an identity is ever canonicalised past it, by [`canonical_source`].
fn claude_source(
    explicit: bool,
    home: Option<&Path>,
    base: &crate::launch_cmd::Resolved,
) -> Result<Option<PathBuf>, String> {
    if explicit {
        return Ok(home.map(|home| home.join(".claude.json")));
    }
    Ok(match crate::run::canonical_config_home(base)? {
        crate::launch_cmd::Resolved::Path(base) => Some(base.join(".claude.json")),
        crate::launch_cmd::Resolved::Absent | crate::launch_cmd::Resolved::Unknown(_) => None,
    })
}

/// The Claude scope a statusline feed runs under — the same key a configured
/// scope of that config home carries — or `None` when the caller's
/// `CLAUDE_CONFIG_DIR` is set but empty or relative, which names no home ae
/// could join.
fn claude_feed_scope(config_dir: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Option<PathBuf> {
    use crate::launch_cmd::Resolved;
    let source = match config_dir.map(Path::new) {
        Some(dir) if dir.is_absolute() => {
            match crate::run::canonical_config_home(&Resolved::Path(dir.to_path_buf())).ok()? {
                Resolved::Path(home) => claude_source(true, Some(&home), &Resolved::Absent),
                Resolved::Absent | Resolved::Unknown(_) => return None,
            }
        }
        Some(_) => return None,
        None => {
            let base = home.filter(|home| home.is_absolute())?.to_path_buf();
            claude_source(false, None, &Resolved::Path(base))
        }
    };
    canonical_source(source.ok()??).ok()
}

/// `ae _quota-feed` past its drain and its gate: record what `payload` states
/// for the caller's own Claude scope under the state root `root`, or nothing.
/// An unchanged payload is not written again for [`feed::REWRITE_SECS`].
pub(crate) fn publish_feed(
    root: &Path,
    config_dir: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    payload: &[u8],
    now: i64,
) {
    use std::os::unix::fs::DirBuilderExt as _;
    let windows = feed::extract(payload);
    let Some(scope) = claude_feed_scope(config_dir, home).filter(|_| !windows.is_empty()) else {
        return;
    };
    let dir = root.join(feed::DIR);
    let name = feed::file_name(&scope);
    let record = feed::Record {
        scope,
        received_at: now,
        windows,
    };
    let held = match crate::store::read_capped_in(&dir, &name, FEED_CAP) {
        Ok(crate::store::SourceRead::Ready(bytes)) => feed::parse(&bytes).ok(),
        _ => None,
    };
    if held.is_some_and(|held| !feed::due(&held, &record)) {
        return;
    }
    let Some(text) = feed::render(&record) else {
        return;
    };
    // A directory already there is the expected case; anything that is not a
    // real directory is refused by the publish below.
    let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
    let _ = crate::store::publish_private_in(&dir, &name, text.as_bytes());
}

/// A Claude scope's windows: its `/usage` cache with its statusline feed beside it.
fn claude_observed(
    scope: &Scope,
    inputs: &Inputs<'_>,
    budget: &mut Budget,
) -> (Observed, Option<claude::Plan>) {
    let (read, plan) = read_claude(scope, inputs.now, budget);
    let fed = read_feed(
        inputs.sessions,
        scope.source_key.as_deref(),
        inputs.now,
        budget,
    );
    (rows_or_placeholder(with_feed(read, fed)), plan)
}

/// The windows this scope's statusline feed recorded, judged at `now`. A
/// record that is missing, damaged, or names another scope supplies nothing,
/// and every read is charged to the budget before it happens, whatever it finds.
fn read_feed(
    sessions: Option<&Path>,
    key: Option<&Path>,
    now: i64,
    budget: &mut Budget,
) -> Vec<Row> {
    let (Some(root), Some(key)) = (sessions.and_then(Path::parent), key) else {
        return Vec::new();
    };
    // The capped read takes one byte past the cap to see an oversized record.
    let allowance = FEED_CAP + 1;
    if !budget.claim_file() || !budget.reserve_bytes(allowance) {
        return Vec::new();
    }
    let read = crate::store::read_capped_in(&root.join(feed::DIR), &feed::file_name(key), FEED_CAP);
    // Only what was read stays charged: nothing for an absent record, its
    // length for a whole one, the whole allowance for anything else.
    let (bytes, charged) = match read {
        Ok(crate::store::SourceRead::Ready(bytes)) => {
            let len = u64::try_from(bytes.len()).unwrap_or(allowance);
            (bytes, len)
        }
        Ok(crate::store::SourceRead::Absent) => (Vec::new(), 0),
        _ => (Vec::new(), allowance),
    };
    budget.refund_bytes(allowance.saturating_sub(charged));
    let record = match feed::parse(&bytes) {
        Ok(record) if record.scope == key && !budget.expired() => record,
        _ => return Vec::new(),
    };
    record
        .windows
        .into_iter()
        .map(|window| Row {
            bucket: window.bucket.to_owned(),
            qualifier: None,
            window_minutes: Some(window.window_minutes),
            used_percent: Some(window.used_percent),
            resets_at: Some(window.resets_at),
            observed_at: Some(record.received_at),
            status: freshness(Some(record.received_at), Some(window.resets_at), now),
        })
        .collect()
}

/// The cache's rows with the feed's beside them. Per window the strictly newer
/// observation stays — the clock [`Reading::adopt`] keeps — and a model-scoped
/// window, which the feed never states, stays the cache's. A damaged cache keeps
/// its placeholder, so the damage stays in view.
fn with_feed(read: ReadRows, fed: Vec<Row>) -> ReadRows {
    if fed.is_empty() {
        return read;
    }
    let mut observed = match read {
        ReadRows::Missing => Observed {
            rows: Vec::new(),
            account: Account::default(),
            series: Vec::new(),
        },
        other => rows_or_placeholder(other),
    };
    for row in fed {
        let held = observed
            .rows
            .iter_mut()
            .find(|held| held.bucket == row.bucket && held.qualifier.is_none());
        match held {
            Some(held) if strictly_newer(held.observed_at, row.observed_at) => *held = row,
            Some(_) => {}
            None => observed.rows.push(row),
        }
    }
    observed.rows.sort_by(|left, right| {
        left.bucket
            .cmp(&right.bucket)
            .then_with(|| left.qualifier.cmp(&right.qualifier))
    });
    ReadRows::Rows(observed)
}

/// The one observation clock: a stamp replaces the held one only when it is
/// strictly newer, and any stamp is newer than none.
fn strictly_newer(held: Option<i64>, incoming: Option<i64>) -> bool {
    incoming > held
}

fn unknown_scope(
    owner: Option<&str>,
    account: Option<&str>,
    tool: ToolKind,
    client: Option<String>,
    hint: Option<String>,
) -> Scope {
    Scope {
        tool,
        home: None,
        source: None,
        source_key: None,
        profiles: owner.map(str::to_owned).into_iter().collect(),
        configured_profiles: owner.map(str::to_owned).into_iter().collect(),
        clients: client.into_iter().collect(),
        accounts: account.map(str::to_owned).into_iter().collect(),
        rollouts: Vec::new(),
        hint,
        manual_resets: None,
        notes: Vec::new(),
        leg: false,
        go: false,
    }
}

/// Read one `[clients]` row's declared extra headroom.
fn declaration_for(cfg: &crate::config::IdentityConfig, label: Option<&str>) -> Declaration {
    let Some(client) = label.and_then(|label| cfg.client(label)) else {
        return Declaration::default();
    };
    Declaration {
        manual_resets: client.manual_resets,
        note: client.manual_resets_note.clone(),
    }
}

/// Fold another client label's declaration into the scope they share.
///
/// Two labels resolving to one config home are one account, so the counts must
/// be reconciled rather than applied in config order. The SMALLEST explicit
/// count wins: claiming headroom the operator never declared would suppress a
/// real advisory, while under-claiming only leaves the raw window in charge.
/// The disagreement is said out loud either way.
fn merge_declaration(scope: &mut Scope, declaration: &Declaration) {
    if let Some(note) = declaration.note.clone()
        && !scope.notes.contains(&note)
    {
        scope.notes.push(note);
    }
    let (Some(next), held) = (declaration.manual_resets, scope.manual_resets) else {
        return;
    };
    let Some(held) = held else {
        scope.manual_resets = Some(next);
        return;
    };
    if held == next {
        return;
    }
    let (low, high) = (held.min(next), held.max(next));
    scope.manual_resets = Some(low);
    let note = format!("manual_resets declared as {low} and {high} for one scope; using {low}");
    if !scope.notes.contains(&note) {
        scope.notes.push(note);
    }
}

fn read_claude(scope: &Scope, now: i64, budget: &mut Budget) -> (ReadRows, Option<claude::Plan>) {
    let Some(path) = scope.source.as_deref() else {
        return (ReadRows::Missing, None);
    };
    let bytes = match bounded_whole_file(path, CLAUDE_MAX_BYTES, budget) {
        Ok(Bounded::Ready(Some(bytes))) => bytes,
        Ok(Bounded::Ready(None)) => return (ReadRows::Missing, None),
        Ok(Bounded::Truncated) => return (ReadRows::Truncated, None),
        Err(_) => return (ReadRows::Failed, None),
    };
    match claude::parse(&bytes, now) {
        Ok(parsed) => match parsed.snapshot {
            Some(snapshot) if !snapshot.rows.is_empty() => (
                ReadRows::Rows(Observed {
                    rows: snapshot.rows,
                    account: Account::default(),
                    series: Vec::new(),
                }),
                parsed.plan,
            ),
            _ => (ReadRows::Missing, parsed.plan),
        },
        Err(_) => (ReadRows::Failed, None),
    }
}

/// The newest billing record in the Grok debug log, as one window.
///
/// A missing log or a record that names no usable window is `Missing`, which
/// the caller shows as the row this tool always had; the flag is the vendor
/// saying a full window does not stop spend.
fn read_grok(scope: &Scope, now: i64, budget: &mut Budget) -> (ReadRows, bool) {
    let Some(path) = scope.source.as_deref() else {
        return (ReadRows::Missing, false);
    };
    let (bytes, boundary) = match bounded_tail(path, grok::TAIL_CAP, budget) {
        Ok(Bounded::Ready(read)) => read,
        Ok(Bounded::Truncated) => return (ReadRows::Truncated, false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (ReadRows::Missing, false),
        Err(_) => return (ReadRows::Failed, false),
    };
    let snapshot = grok::parse(&bytes, boundary, now);
    match snapshot.row {
        Some(row) => (
            ReadRows::Rows(Observed {
                rows: vec![row],
                account: Account::default(),
                series: Vec::new(),
            }),
            snapshot.on_demand_enabled,
        ),
        None => (ReadRows::Missing, false),
    }
}

fn fleet_rollouts(sessions: Option<&Path>, budget: &mut Budget) -> FleetRollouts {
    let Some(sessions) = sessions else {
        return FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
    };
    let paths = match session_paths(sessions, budget) {
        Ok(Bounded::Ready(paths)) => paths,
        Ok(Bounded::Truncated) => {
            return FleetRollouts {
                rollouts: Vec::new(),
                status: FleetStatus::Truncated,
            };
        }
        Err(_) => {
            return FleetRollouts {
                rollouts: Vec::new(),
                status: FleetStatus::Failed,
            };
        }
    };
    let mut rollouts = Vec::new();
    let mut status = FleetStatus::Complete;
    for path in paths {
        let meta_path = crate::store::open(&path).meta_path();
        let bytes = match bounded_whole_file(&meta_path, SESSION_META_MAX_BYTES, budget) {
            Ok(Bounded::Ready(Some(bytes))) => bytes,
            Ok(Bounded::Ready(None)) => continue,
            Ok(Bounded::Truncated) => {
                status = FleetStatus::Truncated;
                break;
            }
            Err(_) => {
                status = FleetStatus::Failed;
                continue;
            }
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            status = FleetStatus::Failed;
            continue;
        };
        let meta = crate::meta::Meta::parse(text);
        let session = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        for seat in meta.roster() {
            let Some(binary) = seat.binary.as_deref() else {
                continue;
            };
            let tool = ToolKind::from_binary_name(binary);
            if tool.adapter().quota.source != QuotaSource::CodexRollouts {
                continue;
            }
            let (Some(profile), Some(id)) = (&seat.profile, &seat.harness_session) else {
                continue;
            };
            let location = recorded_rollout_location(&seat.config_home);
            let client = match &seat.client {
                crate::meta::RecordedClient::Label(label) => Some(label.clone()),
                crate::meta::RecordedClient::Missing | crate::meta::RecordedClient::Invalid => None,
            };
            rollouts.push(FleetRollout {
                owner: format!("{session}:{}", seat.name),
                profile: profile.clone(),
                client,
                id: id.clone(),
                tool,
                location,
            });
        }
    }
    FleetRollouts { rollouts, status }
}

fn recorded_rollout_location(home: &crate::meta::RecordedConfigHome) -> RolloutLocation {
    match home {
        crate::meta::RecordedConfigHome::Missing => RolloutLocation::Configured,
        crate::meta::RecordedConfigHome::Path(home)
        | crate::meta::RecordedConfigHome::Implicit(home) => {
            let source = home.join("sessions");
            match canonical_source(source.clone()) {
                Ok(source_key) => RolloutLocation::Recorded {
                    home: home.clone(),
                    source,
                    source_key,
                },
                Err(reason) => RolloutLocation::Unknown(reason),
            }
        }
        crate::meta::RecordedConfigHome::Absent => {
            RolloutLocation::Unknown("recorded config home is absent".to_owned())
        }
        crate::meta::RecordedConfigHome::Unknown => {
            RolloutLocation::Unknown("recorded config home is unknown".to_owned())
        }
        crate::meta::RecordedConfigHome::Invalid => {
            RolloutLocation::Unknown("recorded config home is invalid".to_owned())
        }
    }
}

fn add_recorded_codex_scopes(scopes: &mut Vec<Scope>, fleet: &FleetRollouts) {
    for rollout in &fleet.rollouts {
        match &rollout.location {
            RolloutLocation::Configured => {}
            RolloutLocation::Recorded {
                home,
                source,
                source_key,
            } => {
                if let Some(scope) = scopes.iter_mut().find(|scope| {
                    scope.tool == rollout.tool && scope.source_key.as_ref() == Some(source_key)
                }) {
                    if !scope.profiles.contains(&rollout.profile) {
                        scope.profiles.push(rollout.profile.clone());
                    }
                } else {
                    scopes.push(Scope {
                        tool: rollout.tool,
                        home: Some(home.clone()),
                        source: Some(source.clone()),
                        source_key: Some(source_key.clone()),
                        profiles: vec![rollout.profile.clone()],
                        configured_profiles: Vec::new(),
                        clients: Vec::new(),
                        accounts: Vec::new(),
                        rollouts: Vec::new(),
                        hint: None,
                        manual_resets: None,
                        notes: Vec::new(),
                        leg: false,
                        go: false,
                    });
                }
            }
            // A rollout whose recorded home did not resolve proved NO account.
            // Two of them failing for the SAME REASON is two failures that read
            // alike, not one identity, so they are never gathered: a matching
            // reason string would otherwise attribute each seat's profile to
            // the other's rollout. Each stands for itself, by id.
            RolloutLocation::Unknown(reason) => {
                scopes.push(Scope {
                    tool: rollout.tool,
                    home: None,
                    source: None,
                    source_key: None,
                    profiles: vec![rollout.profile.clone()],
                    configured_profiles: Vec::new(),
                    clients: Vec::new(),
                    accounts: Vec::new(),
                    rollouts: vec![rollout.id.clone()],
                    hint: Some(reason.clone()),
                    manual_resets: None,
                    notes: Vec::new(),
                    leg: false,
                    go: false,
                });
            }
        }
    }
}

fn session_paths(root: &Path, budget: &mut Budget) -> io::Result<Bounded<Vec<PathBuf>>> {
    if !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: bounded canonical-session enumeration for quota rollout provenance"
    )]
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Bounded::Ready(Vec::new()));
        }
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sessions root is not a directory",
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: enumerates only bounded direct children of the canonical ae sessions root"
    )]
    let entries = std::fs::read_dir(root)?;
    let mut paths = Vec::new();
    for entry in entries {
        if !budget.claim_file() {
            return Ok(Bounded::Truncated);
        }
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() && !kind.is_symlink() {
            paths.push(entry.path());
        }
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    paths.sort();
    Ok(Bounded::Ready(paths))
}

fn codex_groups(
    scope: &Scope,
    fleet: &FleetRollouts,
    now: i64,
    budget: &mut Budget,
) -> CodexGroups {
    let candidates = scope_rollouts(scope, fleet);
    let total = candidates.len();
    let mut truncated = fleet.status == FleetStatus::Truncated;
    let located = locate_rollouts(scope, &candidates, budget, &mut truncated);

    let mut ranked = Vec::new();
    let mut series = Vec::new();
    for located in located {
        let rows = match located.source {
            RolloutSource::File(file) => read_codex_file(&file, now, budget),
            RolloutSource::Missing => ReadRows::Missing,
            RolloutSource::Failed => ReadRows::Failed,
        };
        if matches!(rows, ReadRows::Truncated) {
            truncated = true;
            break;
        }
        let read = rows_or_placeholder(rows);
        if let Some(source) = &scope.source_key {
            series.extend(read.series.into_iter().map(|held| (source.clone(), held)));
        }
        let observed = read.rows.iter().filter_map(|row| row.observed_at).max();
        ranked.push(RankedGroup {
            group: Group {
                profiles: scope.profiles.clone(),
                tool: scope.tool,
                home: scope.home.clone(),
                source: scope.source_key.clone(),
                clients: scope.clients.clone(),
                rollout: Some(located.rollout.id.clone()),
                owner: Some(located.rollout.owner.clone()),
                rows: read.rows,
                hint: scope.hint.clone(),
                summary: None,
                policy: Policy::new(scope.manual_resets, read.account),
                notes: scope.notes.clone(),
                plan: None,
            },
            observed,
        });
    }
    order_ranked_groups(&mut ranked);

    let mut all: Vec<Group> = ranked.iter().map(|ranked| ranked.group.clone()).collect();
    let rendered = summarize_codex_groups(scope, fleet.status, ranked, total, truncated);
    if all.is_empty()
        && let Some(placeholder) = rendered.iter().find(|group| group.summary.is_none())
    {
        all.push(placeholder.clone());
    }
    CodexGroups {
        all,
        rendered,
        series,
    }
}

fn scope_rollouts<'a>(scope: &Scope, fleet: &'a FleetRollouts) -> Vec<&'a FleetRollout> {
    fleet
        .rollouts
        .iter()
        .filter(|rollout| match &rollout.location {
            RolloutLocation::Configured => match &rollout.client {
                // An override seat names its account, not its profile's
                // default store: it belongs to the scope that account
                // reached. No fallback to the profile name — a label no
                // scope claims is unknown, and guessing a scope would read
                // the wrong store's windows as this seat's.
                Some(client) => scope.accounts.iter().any(|account| account == client),
                None => scope.configured_profiles.contains(&rollout.profile),
            },
            RolloutLocation::Recorded { source_key, .. } => {
                scope.source_key.as_ref() == Some(source_key)
            }
            // Named by id: the scope stands for this rollout and no other.
            RolloutLocation::Unknown(_) => scope.rollouts.contains(&rollout.id),
        })
        .fold(Vec::new(), |mut unique, rollout| {
            if !unique
                .iter()
                .any(|seen: &&FleetRollout| seen.id == rollout.id)
            {
                unique.push(rollout);
            }
            unique
        })
}

fn locate_rollouts<'a>(
    scope: &Scope,
    candidates: &[&'a FleetRollout],
    budget: &mut Budget,
    truncated: &mut bool,
) -> Vec<LocatedRollout<'a>> {
    let mut located = Vec::new();
    for &rollout in candidates {
        let sessions = match &rollout.location {
            RolloutLocation::Configured => scope.source.as_deref(),
            RolloutLocation::Recorded { source, .. } => Some(source.as_path()),
            RolloutLocation::Unknown(_) => None,
        };
        let source = match sessions {
            Some(sessions) => match find_codex_rollout(sessions, &rollout.id, budget) {
                Ok(Bounded::Ready(Some(file))) => RolloutSource::File(Box::new(file)),
                Ok(Bounded::Ready(None)) => RolloutSource::Missing,
                Ok(Bounded::Truncated) => {
                    *truncated = true;
                    break;
                }
                Err(_) => RolloutSource::Failed,
            },
            None => RolloutSource::Missing,
        };
        let modified = match &source {
            RolloutSource::File(file) => file.modified,
            RolloutSource::Missing | RolloutSource::Failed => None,
        };
        located.push(LocatedRollout {
            rollout,
            source,
            modified,
        });
    }
    order_located_rollouts(&mut located);
    located
}

fn summarize_codex_groups(
    scope: &Scope,
    fleet_status: FleetStatus,
    ranked: Vec<RankedGroup>,
    total: usize,
    truncated: bool,
) -> Vec<Group> {
    let shown = ranked.len().min(CODEX_DISPLAY_ROLLOUTS);
    let hidden = total.saturating_sub(shown);
    let oldest_observed = ranked
        .iter()
        .skip(shown)
        .flat_map(|ranked| ranked.group.rows.iter())
        .filter_map(|row| row.observed_at)
        .min();
    let unreadable = ranked
        .iter()
        .skip(shown)
        .filter(|ranked| {
            ranked
                .group
                .rows
                .iter()
                .any(|row| row.status == Status::ReadError)
        })
        .count();
    let mut groups: Vec<Group> = ranked
        .into_iter()
        .take(CODEX_DISPLAY_ROLLOUTS)
        .map(|ranked| ranked.group)
        .collect();
    if groups.is_empty() {
        groups.push(Group {
            profiles: scope.profiles.clone(),
            tool: scope.tool,
            home: scope.home.clone(),
            source: scope.source_key.clone(),
            clients: scope.clients.clone(),
            rollout: None,
            owner: None,
            rows: vec![placeholder(Status::Unknown)],
            hint: scope.hint.clone(),
            summary: None,
            policy: Policy::new(scope.manual_resets, Account::default()),
            notes: scope.notes.clone(),
            plan: None,
        });
    }
    let summary_status = if truncated {
        Some(Status::Truncated)
    } else if fleet_status == FleetStatus::Failed || unreadable > 0 {
        Some(Status::ReadError)
    } else {
        None
    };
    if hidden > 0 || summary_status.is_some() {
        groups.push(Group {
            // Summary rows render no identity cells, but retain the exact
            // scope tuple so settings completeness cannot attach to a sibling
            // unresolved source with the same client label.
            profiles: scope.profiles.clone(),
            tool: scope.tool,
            home: scope.home.clone(),
            source: scope.source_key.clone(),
            clients: scope.clients.clone(),
            rollout: None,
            owner: None,
            rows: Vec::new(),
            hint: scope.hint.clone(),
            policy: Policy::default(),
            notes: Vec::new(),
            summary: Some(RolloutSummary {
                hidden,
                unreadable,
                oldest_observed,
                not_read: truncated && shown == 0,
                status: summary_status,
            }),
            plan: None,
        });
    }
    groups
}

fn order_located_rollouts(rollouts: &mut [LocatedRollout<'_>]) {
    rollouts.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.rollout.owner.cmp(&right.rollout.owner))
            .then_with(|| left.rollout.id.cmp(&right.rollout.id))
    });
}

fn order_ranked_groups(groups: &mut [RankedGroup]) {
    groups.sort_by(|left, right| {
        right
            .observed
            .cmp(&left.observed)
            .then_with(|| left.group.owner.cmp(&right.group.owner))
            .then_with(|| left.group.rollout.cmp(&right.group.rollout))
    });
}

fn read_codex_file(file: &RolloutFile, now: i64, budget: &mut Budget) -> ReadRows {
    let (bytes, starts_at_boundary) = match bounded_tail_after_lstat(file, CODEX_TAIL_BYTES, budget)
    {
        Ok(Bounded::Ready(tail)) => tail,
        Ok(Bounded::Truncated) => return ReadRows::Truncated,
        Err(_) => return ReadRows::Failed,
    };
    match codex::parse(&bytes, starts_at_boundary, now) {
        Ok(snapshot) if !snapshot.rows.is_empty() => ReadRows::Rows(Observed {
            rows: snapshot.rows,
            account: snapshot.account,
            series: snapshot.points,
        }),
        Ok(_) => ReadRows::Missing,
        Err(_) => ReadRows::Failed,
    }
}

fn rows_or_placeholder(read: ReadRows) -> Observed {
    let rows = match read {
        ReadRows::Rows(observed) => return observed,
        ReadRows::Missing => vec![placeholder(Status::Unknown)],
        ReadRows::Failed => vec![placeholder(Status::ReadError)],
        ReadRows::Truncated => vec![placeholder(Status::Truncated)],
    };
    Observed {
        rows,
        account: Account::default(),
        series: Vec::new(),
    }
}

fn placeholder(status: Status) -> Row {
    Row {
        bucket: "-".to_owned(),
        qualifier: None,
        window_minutes: None,
        used_percent: None,
        resets_at: None,
        observed_at: None,
        status,
    }
}

fn scope_identity(group: &Group, home: Option<&Path>) -> String {
    if group.clients.is_empty() {
        group
            .home
            .as_deref()
            .map_or_else(|| "unknown".to_owned(), |path| short_path(path, home))
    } else {
        group.clients.join(", ")
    }
}

/// The scope as the table names it before any plan: its tool and identity, and
/// a Codex scope's rollout owner. The machine document states this on its own.
fn scope_name(group: &Group, home: Option<&Path>) -> String {
    if is_copilot(group) {
        return scope_identity(group, home);
    }
    let mut label = format!("{} · {}", group.tool.as_str(), scope_identity(group, home));
    if group.tool.adapter().quota.source == QuotaSource::CodexRollouts {
        label.push_str(" · unidentified");
        if let Some(owner) = group.owner.as_deref() {
            let _ = write!(label, " ({owner})");
        }
    }
    label
}

fn scope_label(group: &Group, home: Option<&Path>, now: i64) -> String {
    let mut label = scope_name(group, home);
    if let Some(words) = group.plan.as_ref().and_then(|plan| plan_words(plan, now)) {
        let _ = write!(label, " · {words}");
    }
    label
}

/// The plan an account reports, as the scope's label draws it: the words the
/// client states and the age of that report, or just that it is reported when
/// the stamp is missing or lies in the future. A name, never a capacity.
fn plan_words(plan: &claude::Plan, now: i64) -> Option<String> {
    let label = plan.label()?;
    let reported = stated_observed_at(plan.fetched_at, now).map_or_else(
        || "reported".to_owned(),
        |at| format!("reported {}", age_label(now - at)),
    );
    Some(sanitize_cell(&format!("plan: {label} ({reported})")))
}

fn short_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_owned();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

/// The manual refresh a Claude scope can be told to run when it holds nothing
/// usable: Claude Code's cache moves only when a `/usage` fetch succeeds and
/// the statusline feed only when a statusline hands ae a payload, so ae cannot
/// refresh either, and the honest answer is to name the actions that can.
const CLAUDE_REFRESH_HINT: &str = "run /usage in a claude session or feed ae from your statusline";

/// The manual refresh a scope's own renderer may show, or `None`.
///
/// Derived at RENDER time from the observation, deliberately never written
/// back into [`Group::hint`]. That field is part of `settings_same_scope`'s
/// identity tuple, so a per-observation fact stored there would split one
/// account into two dialog scopes the moment two groups of the same scope
/// disagreed about staleness. The hints that ARE stored describe unresolved
/// sources, which is identity; this one describes an observation, which is
/// not, so the two must not share a field.
fn manual_refresh_hint(
    tool: ToolKind,
    statuses: impl Iterator<Item = Status>,
) -> Option<&'static str> {
    if tool != ToolKind::Claude {
        return None;
    }
    let mut any = false;
    for status in statuses {
        any = true;
        if status != Status::Unknown {
            return None;
        }
    }
    any.then_some(CLAUDE_REFRESH_HINT)
}

/// The leg's own words beside a scope: agy's always, Muse's only once a leg
/// spoke, which is any hint but the adapter's standing "no local source" one.
fn leg_hint(group: &Group) -> Option<&str> {
    if is_copilot(group) {
        return Some("refresh: ae quota (at most every 5 min)");
    }
    let hint = group.hint.as_deref()?;
    let spoke = group.tool.quota_is_asked()
        || (group.tool == ToolKind::Muse
            && group.tool.adapter().quota.unsupported_hint != Some(hint));
    spoke.then_some(hint)
}

/// The table with no history behind it: every PACE cell is `-`.
#[cfg(test)]
fn render_at(groups: &[Group], home: Option<&Path>, now: i64) -> String {
    render_observation_at(&Observation {
        groups: groups.to_vec(),
        rendered: groups.to_vec(),
        home: home.map(Path::to_path_buf),
        now,
    })
}

#[cfg(test)]
fn render_observation_at(observation: &Observation) -> String {
    render_with_pace(
        observation.ordered_groups(&observation.rendered),
        &pace::PaceTable::default(),
        observation.home.as_deref(),
        observation.now,
    )
}

fn render_with_pace<'a>(
    groups: impl IntoIterator<Item = &'a Group>,
    pace: &pace::PaceTable,
    home: Option<&Path>,
    now: i64,
) -> String {
    const HEADER: [&str; COLUMNS] = [
        "PROFILES",
        "SCOPE",
        "BUCKET",
        "WINDOW",
        "USED",
        "EFFECTIVE",
        "CREDITS",
        "PACE",
        "RESETS",
        "OBSERVED",
        "STATUS",
    ];
    let mut table = vec![RenderLine::Cells(HEADER.map(str::to_owned))];
    let mut notes: Vec<String> = Vec::new();
    let mut claimed = std::collections::BTreeSet::new();
    for group in groups {
        collect_notes(&mut notes, group);
        if let Some(summary) = &group.summary {
            table.push(RenderLine::Summary {
                label: rollout_summary_label(summary, now),
                status: summary
                    .status
                    .map_or_else(String::new, |status| status.as_str().to_owned()),
            });
            continue;
        }
        let refresh_hint = manual_refresh_hint(group.tool, group.rows.iter().map(|row| row.status));
        for (index, row) in group.rows.iter().enumerate() {
            let trustworthy = matches!(row.status, Status::Fresh | Status::Stale);
            let status = group.hint.as_deref().or(refresh_hint).map_or_else(
                || row.status.as_str().to_owned(),
                |hint| format!("{} ({hint})", row.status.as_str()),
            );
            table.push(RenderLine::Cells([
                if index == 0 {
                    profiles_label(&group.profiles)
                } else {
                    String::new()
                },
                if index == 0 {
                    scope_label(group, home, now)
                } else {
                    String::new()
                },
                bucket_label(row),
                trustworthy
                    .then(|| row.window_minutes.map(window_label))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| row.used_percent.as_deref().map(percent_label))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| effective_cell(group, row))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                if index == 0 {
                    credits_label(&group.policy)
                } else {
                    String::new()
                },
                pace.cell_for(group, row, &mut claimed),
                trustworthy
                    .then(|| row.resets_at.map(|reset| reset_label(reset, now)))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                stated_observed_at(row.observed_at, now)
                    .map_or_else(|| "-".to_owned(), |observed| age_label(now - observed)),
                status,
            ]));
        }
    }
    for note in notes {
        table.push(RenderLine::Summary {
            label: note,
            status: String::new(),
        });
    }
    render_table(&table)
}

/// Add a group's notes to the report's, each once.
fn collect_notes(notes: &mut Vec<String>, group: &Group) {
    for note in group
        .notes
        .iter()
        .cloned()
        .chain(untrusted_credit_note(group))
    {
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
}

/// Say once why a reported unlimited-credit claim did not relieve this scope.
///
/// The claim is simply not used. Every other rule still applies to each window
/// here, so the note says what was ignored rather than what `EFFECTIVE` shows:
/// a declared reset or a spend cap may well be deciding these cells.
fn untrusted_credit_note(group: &Group) -> Option<String> {
    let newest = group.rows.iter().filter_map(|row| row.observed_at).max();
    let account = &group.policy.account;
    (account.credits == Credits::Unlimited && !account.credits_relieve(newest)).then(|| {
        "credits unlimited was reported older than the newest window here, so the claim is ignored; EFFECTIVE follows the remaining rules".to_owned()
    })
}

/// The EFFECTIVE cell for one window, or `None` when its percentage is absent.
fn effective_cell(group: &Group, row: &Row) -> Option<String> {
    derived(&group.policy, row).map(Derived::cell)
}

fn rollout_summary_label(summary: &RolloutSummary, now: i64) -> String {
    if summary.hidden == 0 {
        return "rollout inventory incomplete".to_owned();
    }
    let noun = if summary.hidden == 1 {
        "rollout"
    } else {
        "rollouts"
    };
    let disposition = if summary.not_read {
        "not read"
    } else {
        "not shown"
    };
    let mut label = format!("+{} {noun} {disposition}", summary.hidden);
    if summary.unreadable > 0 {
        let _ = write!(label, " ({} unreadable)", summary.unreadable);
    }
    if let Some(observed) = summary.oldest_observed {
        let _ = write!(
            label,
            " (oldest observed {})",
            age_label(now.saturating_sub(observed))
        );
    }
    label
}

fn render_table(table: &[RenderLine]) -> String {
    let sanitized: Vec<RenderLine> = table
        .iter()
        .map(|line| match line {
            RenderLine::Cells(row) => {
                RenderLine::Cells(std::array::from_fn(|column| sanitize_cell(&row[column])))
            }
            RenderLine::Summary { label, status } => RenderLine::Summary {
                label: sanitize_cell(label),
                status: sanitize_cell(status),
            },
        })
        .collect();
    let table = sanitized.as_slice();
    let mut widths = [0_usize; COLUMNS];
    for row in table.iter().filter_map(|line| match line {
        RenderLine::Cells(row) => Some(row),
        RenderLine::Summary { .. } => None,
    }) {
        for (column, value) in row.iter().enumerate() {
            widths[column] = widths[column]
                .max(value.chars().count())
                .min(TABLE_MAX_WIDTHS[column]);
        }
    }
    let mut out = String::new();
    for line in table {
        let row = match line {
            RenderLine::Cells(row) => row,
            RenderLine::Summary { label, status } => {
                let indent = widths[0] + 2;
                out.extend(std::iter::repeat_n(' ', indent));
                out.push_str(label);
                if !status.is_empty() {
                    let status_column =
                        widths[..COLUMNS - 1].iter().sum::<usize>() + 2 * (COLUMNS - 1);
                    let used = indent + label.chars().count();
                    out.extend(std::iter::repeat_n(
                        ' ',
                        status_column.saturating_sub(used).max(2),
                    ));
                    out.push_str(status);
                }
                out.push('\n');
                continue;
            }
        };
        let wrapped: Vec<Vec<String>> = row
            .iter()
            .enumerate()
            .map(|(column, value)| wrap_cell(value, widths[column]))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line in 0..height {
            let line_start = out.len();
            for (column, values) in wrapped.iter().enumerate() {
                if column > 0 {
                    out.push_str("  ");
                }
                let value = values.get(line).map_or("", String::as_str);
                out.push_str(value);
                if column + 1 < row.len() {
                    out.extend(std::iter::repeat_n(
                        ' ',
                        widths[column] - value.chars().count(),
                    ));
                }
            }
            let line_end = out.trim_end().len().max(line_start);
            out.truncate(line_end);
            out.push('\n');
        }
    }
    out
}

fn sanitize_cell(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut clean = String::with_capacity(text.len());
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            clean.push('?');
            consume_escape(&mut chars);
        } else if ch.is_control() {
            clean.push('?');
        } else {
            clean.push(ch);
        }
    }
    clean
}

fn bounded_cell(text: &str, column: usize) -> String {
    sanitize_cell(text)
        .chars()
        .take(TABLE_MAX_WIDTHS[column])
        .collect()
}

fn consume_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        Some('[') => {
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(ch) = chars.next() {
                if ch == '\u{7}' {
                    break;
                }
                if ch == '\u{1b}' && chars.next_if_eq(&'\\').is_some() {
                    break;
                }
            }
        }
        Some(_) | None => {}
    }
}

/// The PROFILES cell for one scope.
///
/// A scope discovered from its `[clients]` row alone names no profile, and an
/// empty first cell reads like a rendering bug. It gets the table's own
/// absent spelling, `-`, which every other column already uses for "nothing
/// to state here" — no new vocabulary for the operator to learn.
fn profiles_label(profiles: &[String]) -> String {
    let joined = profiles.join(" ");
    if joined.is_empty() {
        return "-".to_owned();
    }
    if joined.chars().count() <= TABLE_MAX_WIDTHS[0] {
        return joined;
    }
    format!(
        "{} profiles: {}",
        profiles.len(),
        profiles
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn wrap_cell(value: &str, width: usize) -> Vec<String> {
    if value.is_empty() || width == 0 {
        return vec![String::new()];
    }
    let mut rest: Vec<char> = value.chars().collect();
    let mut lines = Vec::new();
    while rest.len() > width {
        let cut = rest[..=width]
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .filter(|cut| *cut > 0)
            .unwrap_or(width);
        lines.push(rest[..cut].iter().collect());
        rest.drain(..cut);
        while rest.first().is_some_and(|ch| ch.is_whitespace()) {
            rest.remove(0);
        }
    }
    lines.push(rest.iter().collect());
    lines
}

/// A window as the quota table names it, bucket then span: `weekly_all 7d`.
pub(crate) fn window_name(row: &Row) -> String {
    row.window_minutes.map_or_else(
        || bucket_label(row),
        |minutes| format!("{} {}", bucket_label(row), window_label(minutes)),
    )
}

fn bucket_label(row: &Row) -> String {
    match row.qualifier.as_deref() {
        Some(qualifier) if row.bucket == "weekly_scoped" => {
            format!("{} {qualifier}", row.bucket)
        }
        Some(qualifier) => format!("{} ({qualifier})", row.bucket),
        None => row.bucket.clone(),
    }
}

fn window_label(minutes: u32) -> String {
    match minutes {
        10_080 => "7d".to_owned(),
        minutes if minutes.is_multiple_of(1_440) => format!("{}d", minutes / 1_440),
        minutes if minutes.is_multiple_of(60) => format!("{}h", minutes / 60),
        minutes => format!("{minutes}m"),
    }
}

fn percent_label(value: &str) -> String {
    let trimmed = if value.contains('.') {
        value.trim_end_matches('0').trim_end_matches('.')
    } else {
        value
    };
    format!("{}%", if trimmed.is_empty() { "0" } else { trimmed })
}

fn advisory_percent(value: Option<&str>) -> String {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map_or_else(
            || "-".to_owned(),
            |value| percent_label(&format!("{:.1}", value.clamp(0.0, 100.0))),
        )
}

fn reset_label(reset: i64, now: i64) -> String {
    format!("in {}", span_label(reset.saturating_sub(now)))
}

/// The observation stamp whose age ae may STATE, or `None` when it may state
/// none.
///
/// A stamp after `now` states no age: the row is `unknown` because `freshness`
/// refused the skew (`FUTURE_SKEW_SECS`), and a clamped `0m ago` would claim
/// the freshest possible observation for the one input ae distrusts. Callers
/// render `None` as their own absence — `-` in the table, no `seen` in the
/// dialog — never as a clipped number.
fn stated_observed_at(observed_at: Option<i64>, now: i64) -> Option<i64> {
    observed_at.filter(|observed| *observed <= now)
}

pub(crate) fn age_label(seconds: i64) -> String {
    format!("{} ago", span_label(seconds.max(0)))
}

fn span_label(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    let days = minutes / 1_440;
    let hours = (minutes % 1_440) / 60;
    let mins = minutes % 60;
    if days > 0 {
        if hours > 0 {
            format!("{days}d {hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        format!("{hours}h{mins:02}m")
    } else {
        format!("{mins}m")
    }
}

fn bounded_whole_file(
    path: &Path,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<Option<Vec<u8>>>> {
    if !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat refuses symlinks and oversized hostile client caches before opening them"
    )]
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Bounded::Ready(None));
        }
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > cap
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache is not a bounded regular file",
        ));
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only the lstat-checked Claude quota cache, bounded again while reading"
    )]
    let file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&metadata, &opened) || opened.len() > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache changed identity or size before the bounded read",
        ));
    }
    if !budget.reserve_bytes(opened.len()) {
        return Ok(Bounded::Truncated);
    }
    let mut bytes = Vec::new();
    file.take(opened.len()).read_to_end(&mut bytes)?;
    let actual = u64::try_from(bytes.len()).unwrap_or(opened.len());
    budget.refund_bytes(opened.len().saturating_sub(actual));
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(Some(bytes)))
}

fn bounded_tail(
    path: &Path,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    if cap == 0 || !budget.claim_file() {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: quota lstat refuses a symlink or non-file before opening one ae-owned Codex rollout or the Grok debug log"
    )]
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout is not a regular file",
        ));
    }
    let modified = metadata.modified().ok();
    bounded_tail_after_lstat(
        &RolloutFile {
            path: path.to_owned(),
            metadata,
            modified,
        },
        cap,
        budget,
    )
}

pub(crate) fn bounded_tail_after_lstat(
    rollout: &RolloutFile,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    if cap == 0 {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens only the exact rollout named by an ae-recorded harness session id, or the lstat-proven Grok debug log"
    )]
    let mut file = File::open(&rollout.path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&rollout.metadata, &opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout changed identity before the bounded read",
        ));
    }
    read_bounded_tail(&mut file, opened.len(), cap, budget)
}

pub(crate) fn bounded_head_after_lstat(
    rollout: &RolloutFile,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<Vec<u8>>> {
    if cap == 0 {
        return Ok(Bounded::Truncated);
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: opens the exact rollout once more for its bounded model header"
    )]
    let file = File::open(&rollout.path)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() || !same_file(&rollout.metadata, &opened) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollout changed identity before the bounded read",
        ));
    }
    let planned = opened.len().min(cap);
    if !budget.reserve_bytes(planned) {
        return Ok(Bounded::Truncated);
    }
    let mut bytes = Vec::new();
    file.take(planned).read_to_end(&mut bytes)?;
    let actual = u64::try_from(bytes.len()).unwrap_or(planned);
    budget.refund_bytes(planned.saturating_sub(actual));
    if actual != planned {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "rollout changed length during the bounded read",
        ));
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(bytes))
}

fn read_bounded_tail(
    file: &mut (impl Read + Seek),
    opened_len: u64,
    cap: u64,
    budget: &mut Budget,
) -> io::Result<Bounded<(Vec<u8>, bool)>> {
    let planned = opened_len.min(cap);
    if !budget.reserve_bytes(planned) {
        return Ok(Bounded::Truncated);
    }
    let starts_at_file = opened_len <= cap;
    if !starts_at_file {
        file.seek(io::SeekFrom::Start(opened_len - cap))?;
    }
    let mut raw = Vec::new();
    Read::by_ref(file).take(planned).read_to_end(&mut raw)?;
    let actual = u64::try_from(raw.len()).unwrap_or(planned);
    budget.refund_bytes(planned.saturating_sub(actual));
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    if starts_at_file {
        return Ok(Bounded::Ready((raw, true)));
    }
    let starts_at_boundary = raw.first() == Some(&b'\n');
    let bytes = raw.get(1..).unwrap_or_default().to_vec();
    Ok(Bounded::Ready((bytes, starts_at_boundary)))
}

#[cfg(unix)]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

pub(crate) fn find_codex_rollout(
    root: &Path,
    id: &str,
    budget: &mut Budget,
) -> io::Result<Bounded<Option<RolloutFile>>> {
    let Some(dirs) = codex_rollout_dirs(root, id) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recorded Codex id carries no usable start time",
        ));
    };
    let mut found = None;
    let suffix = format!("-{id}.jsonl");
    for dir in dirs {
        if !budget.claim_file() {
            return Ok(Bounded::Truncated);
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: quota lstat classifies only the three UTC neighbours of the Codex id day"
        )]
        let metadata = match std::fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rollout root is not a directory",
            ));
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: bounded enumeration of three possible local-clock days for one exact Codex id"
        )]
        let entries = std::fs::read_dir(&dir)?;
        for entry in entries {
            if !budget.claim_file() {
                return Ok(Bounded::Truncated);
            }
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().ends_with(&suffix) {
                let path = entry.path();
                #[allow(
                    clippy::disallowed_methods,
                    reason = "a door: one budgeted lstat validates and ranks an exact ae-owned Codex rollout"
                )]
                let metadata = std::fs::symlink_metadata(&path)?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "recorded rollout is not a regular file",
                    ));
                }
                if found.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "recorded rollout id is not unique",
                    ));
                }
                let modified = metadata.modified().ok();
                found = Some(RolloutFile {
                    path,
                    metadata,
                    modified,
                });
            }
        }
    }
    if budget.expired() {
        return Ok(Bounded::Truncated);
    }
    Ok(Bounded::Ready(found))
}

fn codex_rollout_dirs(root: &Path, id: &str) -> Option<Vec<PathBuf>> {
    if crate::archive::canonical_uuid(id) != id || id.as_bytes().get(14) != Some(&b'7') {
        return None;
    }
    let millis = u64::from_str_radix(&format!("{}{}", id.get(..8)?, id.get(9..13)?), 16).ok()?;
    let seconds = i64::try_from(millis / 1_000).ok()?;
    [0, -86_400, 86_400]
        .into_iter()
        .map(|offset| dated_dir(root, seconds.saturating_add(offset)))
        .collect()
}

fn dated_dir(root: &Path, seconds: i64) -> Option<PathBuf> {
    let timestamp = Timestamp::from_epoch(seconds).to_string();
    let (date, _) = timestamp.split_once('T')?;
    let mut fields = date.split('-');
    let (year, month, day) = (fields.next()?, fields.next()?, fields.next()?);
    if year.len() != 4 || fields.next().is_some() {
        return None;
    }
    Some(root.join(year).join(month).join(day))
}

// Append this module byte-identically to src/quota.rs. Frozen acceptance tests.
#[cfg(test)]
mod quotaburn_acceptance {
    use super::view::{Geometry, Scope, layout};
    use super::{AgyCache, Inputs};
    use std::fmt::Write as _;
    use std::path::PathBuf;

    const NOW: i64 = 1_788_858_600;
    const ID: &str = "01a08046-1974-7352-ade3-81a786200795";

    struct Rig {
        root: PathBuf,
        config: PathBuf,
        sessions: PathBuf,
    }

    impl Rig {
        // Every tuple is (seconds before newest, short USED, weekly USED).
        fn new(tag: &str, age: i64, samples: &[(i64, f64, f64)], resets: u8) -> Self {
            let root = std::env::temp_dir()
                .join(format!("ae-quotaburn-spec-{tag}-{}", std::process::id()));
            std::fs::create_dir(&root).expect("claim a fresh scratch fixture");
            let day = root.join(".codex/sessions/2026/09/08");
            let sessions = root.join("sessions");
            std::fs::create_dir_all(&day).expect("rollout day");
            std::fs::create_dir_all(sessions.join("spec")).expect("session fixture");
            let config = root.join("config");
            std::fs::write(
                &config,
                format!(
                    "[clients]\ncx = codex manual_resets={resets}\n[profiles]\np = cx --model sol\n"
                ),
            )
            .expect("identity config");
            std::fs::write(
                sessions.join("spec/meta"),
                format!(
                    "schema=2\nseat.main=lead\nprofile.main=p\nharness_session.main={ID}\nagent_bin.main=codex\n"
                ),
            )
            .expect("session meta");
            let mut records = String::new();
            for &(ago, short, weekly) in samples {
                let timestamp = crate::time::Timestamp::from_epoch(NOW - age - ago);
                let _ = writeln!(
                    records,
                    "{{\"timestamp\":\"{timestamp}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":{short},\"window_minutes\":300,\"resets_at\":{}}},\"secondary\":{{\"used_percent\":{weekly},\"window_minutes\":10080,\"resets_at\":{}}}}}}}}}",
                    NOW + 2 * 3600,
                    NOW + 3 * 86400,
                );
            }
            std::fs::write(day.join(format!("rollout-test-{ID}.jsonl")), records)
                .expect("rollout history");
            Self {
                root,
                config,
                sessions,
            }
        }

        fn inputs(&self) -> Inputs<'_> {
            Inputs {
                home: Some(&self.root),
                global: Some(&self.config),
                local: None,
                sessions: Some(&self.sessions),
                now: NOW,
            }
        }

        fn views(&self) -> Vec<Scope> {
            let cache = AgyCache::new(|_| panic!("no paid quota ask in acceptance test"));
            let (held, due) = super::view_held(&self.inputs(), &cache, false);
            assert!(due.is_none());
            let mut cache = AgyCache::new(|_| panic!("no paid quota ask in acceptance test"));
            assert_eq!(
                held,
                super::view_cached(&self.inputs(), &mut cache),
                "app held view and tmux cached view must carry the same quota facts"
            );
            held
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn lines(scopes: &[Scope], width: usize) -> Vec<String> {
        layout(
            scopes,
            Geometry {
                width,
                rows: None,
                icons: false,
            },
        )
        .into_iter()
        .map(|line| line.text)
        .collect()
    }

    fn window(lines: &[String], duration: &str) -> String {
        lines
            .iter()
            .find(|line| token(line, duration))
            .unwrap_or_else(|| panic!("missing {duration} window: {lines:?}"))
            .clone()
    }

    fn token(line: &str, expected: &str) -> bool {
        line.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == expected)
    }

    fn has(line: &str, pieces: &[&str]) {
        for piece in pieces {
            let present = if piece.chars().all(|c| c.is_ascii_alphanumeric()) {
                token(line, piece)
            } else {
                line.contains(piece)
            };
            assert!(present, "missing {piece:?}: {line}");
        }
    }

    #[test]
    fn quotaburn_settings_reads_history_and_keeps_short_weekly_span_reset_and_seen_separate() {
        // 3 pp / 0.5 h = 6 pp/h; 12 pp / 6 h = 2 pp/h.
        let rig = Rig::new(
            "history",
            120,
            &[(21600, 0.0, 8.0), (1800, 20.0, 19.0), (0, 23.0, 20.0)],
            0,
        );
        let views = rig.views();
        assert_eq!(views.len(), 1);
        let shown = lines(&views, 180);
        has(&window(&shown, "5h"), &["6.0%/h", "30m", "2h00m", "2m"]);
        has(&window(&shown, "7d"), &["2.0%/h", "6h00m", "3d", "2m"]);
    }

    #[test]
    fn quotaburn_insufficient_history_explains_measured_and_required_span_per_window() {
        // Pace's short minimum is 1 h / 3 = 20 m; weekly is 12 h / 3 = 4 h.
        let rig = Rig::new(
            "insufficient",
            120,
            &[(600, 10.0, 10.0), (0, 16.0, 16.0)],
            0,
        );
        let shown = lines(&rig.views(), 180);
        let short = window(&shown, "5h");
        has(&short, &["insufficient", "10m", "20m"]);
        // Weekly thinning needs >=1350 s between retained samples.
        let weekly_rig = Rig::new(
            "insufficient-weekly",
            120,
            &[(1500, 10.0, 10.0), (0, 16.0, 16.0)],
            0,
        );
        let weekly = window(&lines(&weekly_rig.views(), 180), "7d");
        has(&weekly, &["insufficient", "25m", "4h00m"]);
        assert!(!short.contains("%/h") && !weekly.contains("%/h"));
    }

    #[test]
    fn quotaburn_one_sample_is_explicitly_insufficient_never_bare_unknown() {
        let rig = Rig::new("one", 120, &[(0, 23.0, 20.0)], 0);
        let shown = lines(&rig.views(), 180);
        has(&window(&shown, "5h"), &["insufficient", "0m", "20m"]);
        has(&window(&shown, "7d"), &["insufficient", "0m", "4h00m"]);
    }

    #[test]
    fn quotaburn_minimum_span_boundary_is_the_pace_rule() {
        // A 5 h window needs exactly 1200 s, not a new view-specific threshold.
        let below = Rig::new("below", 120, &[(1199, 10.0, 10.0), (0, 16.0, 16.0)], 0);
        has(
            &window(&lines(&below.views(), 180), "5h"),
            &["insufficient", "20m"],
        );
        let exact = Rig::new("exact", 120, &[(1200, 10.0, 10.0), (0, 16.0, 16.0)], 0);
        has(
            &window(&lines(&exact.views(), 180), "5h"),
            &["18.0%/h", "20m"],
        );
    }

    #[test]
    fn quotaburn_quantized_weekly_bound_rounds_up_and_reports_its_span() {
        // Existing Pace rule: <2 pp over 9 h => upper bound 2/9, rounded UP.
        let rig = Rig::new("bound", 120, &[(32400, 10.0, 10.0), (0, 11.99, 11.99)], 0);
        has(
            &window(&lines(&rig.views(), 180), "7d"),
            &["<0.3%/h", "9h00m"],
        );
    }

    #[test]
    fn quotaburn_rate_uses_judged_percentage_with_manual_resets() {
        // One declared reset halves judged movement: 6 raw pp / 0.5 h / 2.
        let rig = Rig::new("judged", 120, &[(1800, 20.0, 20.0), (0, 26.0, 26.0)], 1);
        has(&window(&lines(&rig.views(), 180), "5h"), &["6.0%/h", "30m"]);
    }

    #[test]
    fn quotaburn_reset_drop_restarts_the_observation_span() {
        let rig = Rig::new(
            "restart",
            120,
            &[(3600, 50.0, 50.0), (600, 5.0, 5.0), (0, 9.0, 9.0)],
            0,
        );
        let shown = lines(&rig.views(), 180);
        has(&window(&shown, "5h"), &["insufficient", "10m", "20m"]);
        // Keep the post-drop weekly sample beyond its 1350 s thinning spacing.
        let weekly_rig = Rig::new(
            "restart-weekly",
            120,
            &[(3600, 50.0, 50.0), (1500, 5.0, 5.0), (0, 9.0, 9.0)],
            0,
        );
        has(
            &window(&lines(&weekly_rig.views(), 180), "7d"),
            &["insufficient", "25m", "4h00m"],
        );
    }

    #[test]
    fn quotaburn_forecast_is_conditional_and_does_not_become_a_stop_instruction() {
        let rig = Rig::new("forecast", 120, &[(1800, 60.0, 60.0), (0, 90.0, 90.0)], 0);
        let short = window(&lines(&rig.views(), 180), "5h");
        has(&short, &["60.0%/h", "30m"]);
        assert!(
            short.contains("at this rate") || (short.contains('~') && short.contains('!')),
            "conditional projection or existing ~ETA! required: {short}"
        );
        let lower = short.to_ascii_lowercase();
        for word in ["stop", "park", "exhausted"] {
            assert!(!lower.contains(word), "forecast is guidance: {short}");
        }
    }

    #[test]
    fn quotaburn_narrow_settings_preserves_complete_rate_span_and_reset_numbers() {
        let rig = Rig::new(
            "width",
            120,
            &[(21600, 0.0, 8.0), (1800, 20.0, 19.0), (0, 26.0, 20.0)],
            0,
        );
        let views = rig.views();
        for width in [100, 90, 80] {
            let shown = lines(&views, width);
            let short = window(&shown, "5h");
            has(&short, &["12.0%/h", "30m", "2h00m"]);
            has(&window(&shown, "7d"), &["2.0%/h", "6h00m", "3d"]);
            assert!(
                shown.iter().all(|line| line.chars().count() <= width),
                "overflow at width {width}: {shown:?}"
            );
        }
    }

    #[test]
    fn quotaburn_stale_row_keeps_a_trusted_rate_and_reports_its_own_age() {
        let rig = Rig::new("stale", 1200, &[(1800, 20.0, 20.0), (0, 26.0, 26.0)], 0);
        has(
            &window(&lines(&rig.views(), 180), "5h"),
            &["12.0%/h", "30m", "stale", "20m"],
        );
    }

    #[test]
    fn quotaburn_operator_table_pace_cells_keep_their_existing_spelling() {
        use super::pace::Pace;
        assert_eq!(Pace::Unknown.cell(), "unknown");
        assert_eq!(Pace::Absent.cell(), "-");
        assert_eq!(Pace::Bound { per_h: 2.0 / 9.0 }.cell(), "<0.3%/h");
        assert_eq!(
            Pace::Rate {
                per_h: 6.0,
                eta_secs: 50400.0,
                before_reset: false
            }
            .cell(),
            "6.0%/h ~14h00m"
        );
        assert_eq!(
            Pace::Rate {
                per_h: 60.0,
                eta_secs: 600.0,
                before_reset: true
            }
            .cell(),
            "60.0%/h ~10m!"
        );
    }

    #[test]
    fn quotaburn_no_history_provider_states_no_burn_history_in_both_settings_paths() {
        let rig = Rig::new("no-history", 120, &[(0, 20.0, 20.0)], 0);
        std::fs::write(&rig.config, "[profiles]\na = agy\n").expect("agy-only config");
        let payload = r#"{"status":"SUCCESS","num_turns":0,"command":{"name":"usage","data":{"groups":[{"name":"Gemini Models","buckets":[{"id":"gemini-5h","window":"5h","remaining_fraction":0.75,"reset_time":"2026-09-08T11:10:00Z"},{"id":"gemini-weekly","window":"weekly","remaining_fraction":0.8,"reset_time":"2026-09-11T09:10:00Z"}]}]}}}"#;
        let mut cache = AgyCache::new(|_| panic!("held answer must prevent a paid ask"));
        cache.settle(
            &rig.root,
            NOW,
            NOW,
            super::AgyRun::Done {
                text: payload.to_owned(),
            },
        );
        let inputs = Inputs {
            sessions: None,
            ..rig.inputs()
        };
        let (held, due) = super::view_held(&inputs, &cache, false);
        assert!(due.is_none());
        assert_eq!(held, super::view_cached(&inputs, &mut cache));
        let shown = lines(&held, 180);
        for duration in ["5h", "7d"] {
            let row = window(&shown, duration);
            has(&row, &["no burn history"]);
            assert!(
                !row.contains("%/h") && !row.contains("insufficient"),
                "{row}"
            );
        }
    }

    #[test]
    fn quotaburn_unusable_rate_says_insufficient_without_a_false_short_span_pair() {
        // Finite USED=1e308 over 1/3 hour overflows the rate: existing Pace::Unknown.
        let rig = Rig::new("unusable", 120, &[(1200, 0.0, 0.0), (0, 1e308, 1e308)], 0);
        let row = window(&lines(&rig.views(), 180), "5h");
        has(&row, &["insufficient", "data"]);
        assert!(!row.contains("%/h"), "{row}");
        assert!(
            !token(&row, "20m") && !row.contains("needed"),
            "span is already sufficient; do not claim a short span: {row}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Account, Bounded, Budget, CLAUDE_MAX_BYTES, CODEX_TAIL_BYTES, COLUMNS, Credits,
        Declaration, FRESH_SECS, FUTURE_SKEW_SECS, FleetRollout, FleetRollouts, FleetStatus, Group,
        LocatedRollout, Policy, QuotaLevel, ReadRows, RenderLine, RolloutLocation, RolloutSource,
        Row, Scope, Status, TABLE_MAX_LINE, TABLE_MAX_WIDTHS, bounded_tail, bounded_whole_file,
        classify, codex_groups, codex_rollout_dirs, configured_scopes, credits_label, derived,
        effective, find_codex_rollout, freshness, merge_declaration, order_located_rollouts,
        percent_label, placeholder, profiles_label, read_bounded_tail, read_claude, render_at,
        render_table, rows_or_placeholder, sanitize_cell, scoped_elsewhere, settings_same_scope,
        vendor_timestamp,
    };
    use crate::tool::ToolKind;
    use crate::transport::AgyRun;
    use std::path::Path;

    /// Oracle: `.local/brief-copilot.md` I5 — every Copilot 401 reads "token
    /// rejected (401)" naming `copilot_token_file`, one "ago"; a coded 401
    /// outcome, which only the Go route measures, never reads "unknown outcome".
    #[test]
    fn a_coded_401_outcome_on_the_copilot_group_still_reads_token_rejected() {
        assert_eq!(
            super::copilot_failure_note("unauthorized:api_key_expired", 60),
            "github copilot: last refresh 1m ago failed — token rejected (401): check copilot_token_file"
        );
    }

    /// Oracle: issue #123 (2026-10-22 live setup) and `.local/brief-govendor.md` —
    /// a 401 whose vendor code is `api_key_expired` tells the operator to renew
    /// the key, naming the code, with one "ago"; any other outcome reads as before.
    #[test]
    fn the_go_failure_note_names_the_expired_key_code_and_nothing_else_changes() {
        assert_eq!(
            super::go_failure_note("unauthorized:api_key_expired", 60),
            "opencode go: last refresh 1m ago failed — key expired (401 api_key_expired): renew it in the OpenCode Console"
        );
        for (outcome, class) in [
            (
                "unauthorized",
                "key rejected (401): check opencode_go_key_file",
            ),
            ("not-entitled", "no Go subscription (403)"),
            ("status-502", "HTTP 502"),
            ("unauthorized:made_up", "unknown outcome"),
        ] {
            assert_eq!(
                super::go_failure_note(outcome, 60),
                format!("opencode go: last refresh 1m ago failed — {class}"),
                "{outcome}"
            );
        }
    }

    #[test]
    fn only_two_recognized_and_different_families_prove_a_window_binds_elsewhere() {
        let claude = ToolKind::Claude;
        for (window, live) in [
            ("Fable", "Opus 5.5"),
            ("Fable", "Opus 5 (1M context)"),
            ("Fable", "Opus 5.5 (1M context)"),
            ("Opus", "claude-haiku-4-5-20251001"),
            ("Sonnet", "Fable 5.1"),
            ("Haiku", "Sonnet 5"),
            ("Fable 5.1", "Opus 5"),
        ] {
            assert!(
                scoped_elsewhere(claude, Some(window), Some(live)),
                "{window} vs {live}"
            );
        }
        for (window, live) in [
            (Some("Fable"), Some("Fable 5.1")),
            (Some("fable"), Some("Fable")),
            (Some("Fable"), None),
            (None, Some("Opus 5.5")),
            (Some("Fable"), Some("Mystery 9")),
            (Some("Fable"), Some("Opus X 5")),
            (Some("Fable"), Some("")),
            (Some("Mystery"), Some("Opus 5")),
        ] {
            assert!(
                !scoped_elsewhere(claude, window, live),
                "{window:?} vs {live:?}"
            );
        }
        // Only Claude scopes a window to a model family; a Codex qualifier is
        // its plan.
        assert!(!scoped_elsewhere(
            ToolKind::Codex,
            Some("Fable"),
            Some("Opus 5")
        ));
        assert!(!scoped_elsewhere(
            ToolKind::Codex,
            Some("pro"),
            Some("gpt-6-sol")
        ));
    }

    #[test]
    fn freshness_boundaries_are_exact() {
        let now = 10_000;
        assert_eq!(
            freshness(Some(now - FRESH_SECS), Some(now + 1), now),
            Status::Fresh
        );
        assert_eq!(
            freshness(Some(now - FRESH_SECS - 1), Some(now + 1), now),
            Status::Stale
        );
        assert_eq!(freshness(Some(now), Some(now), now), Status::Unknown);
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS), Some(now + 1_000), now),
            Status::Unknown
        );
        assert_eq!(
            freshness(Some(now + FUTURE_SKEW_SECS - 1), Some(now + 1_000), now),
            Status::Fresh
        );
        assert_eq!(freshness(None, Some(now + 1), now), Status::Unknown);
        assert_eq!(freshness(Some(now), None, now), Status::Unknown);
    }

    #[test]
    fn vendor_utc_timestamps_accept_the_two_observed_clients() {
        let expected = vendor_timestamp("2026-09-08T09:10:25Z");
        assert!(expected.is_some());
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25.781Z"), expected);
        assert_eq!(
            vendor_timestamp("2026-09-08T09:10:25.759599+00:00"),
            expected
        );
        assert_eq!(vendor_timestamp("2026-09-08T09:10:25+02:00"), None);
    }

    #[test]
    fn codex_uuidv7_lookup_probes_its_utc_day_then_both_local_clock_neighbours() {
        let root = std::path::Path::new("/rollouts");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        assert_eq!(
            codex_rollout_dirs(root, id),
            Some(vec![
                root.join("2026/09/08"),
                root.join("2026/09/07"),
                root.join("2026/09/09"),
            ])
        );
        assert_eq!(codex_rollout_dirs(root, "not-a-session-id"), None);
    }

    #[test]
    fn codex_uuidv7_lookup_finds_an_exact_rollout_on_a_neighbour_day() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-neighbour-day-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join("2026/09/07");
        std::fs::create_dir_all(&day).expect("neighbour day");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        let rollout = day.join(format!("rollout-local-clock-{id}.jsonl"));
        std::fs::write(&rollout, b"{}\n").expect("neighbour rollout");
        let found = find_codex_rollout(&root, id, &mut Budget::new())
            .expect("bounded lookup on neighbours");
        let Bounded::Ready(Some(found)) = found else {
            panic!("neighbour rollout was not found");
        };
        assert_eq!(found.path, rollout);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The plain configured Codex scope the rollout suites read through.
    /// The settings dialog must not gather what the table holds apart.
    ///
    /// Both halves are pinned together on purpose. A scope that proved NO
    /// source has no identity but its rollout, so two of them are one dialog
    /// row only when they ARE one rollout. A scope that PROVED a source keeps
    /// the shipped rule that its rollout owners are evidence about one
    /// account — collapsing those is correct and must not regress.
    #[test]
    fn the_quota_dialog_groups_unproven_scopes_by_rollout_and_proven_ones_by_source() {
        let group = |source: Option<&str>, rollout: &str, owner: &str| Group {
            profiles: vec!["p".to_owned()],
            tool: ToolKind::Codex,
            home: source.map(|_| std::path::PathBuf::from("/tmp/cx")),
            source: source.map(std::path::PathBuf::from),
            clients: Vec::new(),
            rollout: Some(rollout.to_owned()),
            owner: Some(owner.to_owned()),
            rows: vec![placeholder(Status::Unknown)],
            hint: source
                .is_none()
                .then(|| "recorded config home is absent".to_owned()),
            summary: None,
            policy: Policy::default(),
            notes: Vec::new(),
            plan: None,
        };
        let unproven_a = group(None, "rollout-a", "session:lead");
        let unproven_b = group(None, "rollout-b", "session:rev");
        assert!(
            !settings_same_scope(&unproven_a, &unproven_b),
            "two unattributable rollouts are two dialog scopes"
        );
        assert!(
            settings_same_scope(&unproven_a, &unproven_a.clone()),
            "one rollout is still itself"
        );
        let proven_a = group(Some("/tmp/cx/sessions"), "rollout-a", "session:lead");
        let proven_b = group(Some("/tmp/cx/sessions"), "rollout-b", "session:rev");
        assert!(
            settings_same_scope(&proven_a, &proven_b),
            "one proven source is one account, whatever rollouts observed it"
        );
    }

    fn configured_codex_scope(root: &std::path::Path) -> Scope {
        Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            configured_profiles: vec!["codex-profile".to_owned()],
            clients: Vec::new(),
            accounts: Vec::new(),
            rollouts: Vec::new(),
            hint: None,
            manual_resets: None,
            notes: Vec::new(),
            leg: false,
            go: false,
        }
    }

    /// An unstarted override seat's rollout is attributed to its EFFECTIVE
    /// client: the recorded label selects the scope, never the bare profile
    /// name — which would land the seat on its default account. A label no
    /// scope claims matches nothing: unknown, never guessed.
    #[test]
    fn configured_rollout_with_recorded_client_matches_the_override_scope() {
        let scope = |profiles: &[&str], accounts: &[&str]| Scope {
            tool: ToolKind::Codex,
            home: None,
            source: None,
            source_key: None,
            profiles: profiles.iter().map(ToString::to_string).collect(),
            configured_profiles: profiles.iter().map(ToString::to_string).collect(),
            clients: Vec::new(),
            accounts: accounts.iter().map(ToString::to_string).collect(),
            rollouts: Vec::new(),
            hint: None,
            manual_resets: None,
            notes: Vec::new(),
            leg: false,
            go: false,
        };
        let default = scope(&["solx"], &["codex"]);
        let mic = scope(&[], &["codex-mic"]);
        let rollout = |profile: &str, client: Option<&str>, id: &str| FleetRollout {
            owner: format!("session:{id}"),
            profile: profile.to_owned(),
            client: client.map(str::to_owned),
            id: id.to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let fleet = FleetRollouts {
            rollouts: vec![
                rollout("solx", Some("codex-mic"), "overridden"),
                rollout("solx", None, "legacy"),
                rollout("solx", Some("ghost"), "unclaimed"),
            ],
            status: FleetStatus::Complete,
        };
        let owners = |scope: &Scope| {
            super::scope_rollouts(scope, &fleet)
                .iter()
                .map(|rollout| rollout.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(owners(&mic), vec!["overridden".to_owned()]);
        assert_eq!(owners(&default), vec!["legacy".to_owned()]);
    }

    #[test]
    fn codex_rollouts_read_newest_files_first_then_display_three_by_record_clock() {
        const NOW: i64 = 1_788_858_600;
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-five-rollouts-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join(".codex/sessions/2026/09/08");
        std::fs::create_dir_all(&day).expect("rollout day");
        let ids = [
            "01a08046-2000-7abc-8abc-000000000000",
            "01a08046-2001-7abc-8abc-000000000001",
            "01a08046-2002-7abc-8abc-000000000002",
            "01a08046-2003-7abc-8abc-000000000003",
            "01a08046-2004-7abc-8abc-000000000004",
        ];
        let timestamps = [
            "2026-09-08T09:00:00Z",
            "2026-09-08T09:01:00Z",
            "2026-09-08T09:02:00Z",
            "2026-09-08T09:03:00Z",
            "2026-09-08T09:04:00Z",
        ];
        let mut fleet = FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
        for (index, (id, timestamp)) in ids.into_iter().zip(timestamps).enumerate() {
            let record = format!(
                "{{\"timestamp\":\"{timestamp}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":{index},\"window_minutes\":300,\"resets_at\":1788861600}}}}}}}}\n"
            );
            std::fs::write(day.join(format!("rollout-test-{id}.jsonl")), record)
                .expect("rollout record");
            fleet.rollouts.push(FleetRollout {
                owner: format!("session:seat-{index}"),
                profile: "codex-profile".to_owned(),
                client: None,
                id: id.to_owned(),
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = configured_codex_scope(&root);
        let groups = codex_groups(&scope, &fleet, NOW, &mut Budget::new());
        let shown: Vec<_> = groups
            .rendered
            .iter()
            .filter(|group| group.summary.is_none())
            .filter_map(|group| group.owner.as_deref())
            .collect();
        assert_eq!(
            shown,
            ["session:seat-4", "session:seat-3", "session:seat-2"]
        );
        assert_eq!(groups.all.len(), 5, "observation keeps capped-out rollouts");
        let rendered = render_at(&groups.rendered, Some(&root), NOW);
        assert!(rendered.contains("+2 rollouts not shown"), "{rendered}");
        assert!(rendered.contains("oldest observed 10m ago"), "{rendered}");
        assert!(!rendered.contains("session:seat-0"), "{rendered}");
        assert!(rendered.lines().all(|line| line.chars().count() <= 160));
        assert_eq!(CODEX_TAIL_BYTES, 256 * 1024);

        let older = FleetRollout {
            owner: "older".to_owned(),
            profile: "p".to_owned(),
            client: None,
            id: ids[0].to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let newer = FleetRollout {
            owner: "newer".to_owned(),
            profile: "p".to_owned(),
            client: None,
            id: ids[1].to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let epoch = std::time::UNIX_EPOCH;
        let mut located = [
            LocatedRollout {
                rollout: &older,
                source: RolloutSource::Missing,
                modified: Some(epoch),
            },
            LocatedRollout {
                rollout: &newer,
                source: RolloutSource::Missing,
                modified: Some(epoch + std::time::Duration::from_secs(1)),
            },
        ];
        order_located_rollouts(&mut located);
        assert_eq!(located[0].rollout.owner, "newer");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_budget_exhaustion_marks_the_summary_not_a_rollout_group() {
        const NOW: i64 = 1_788_858_600;
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-truncated-summary-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join(".codex/sessions/2026/09/08");
        std::fs::create_dir_all(&day).expect("rollout day");
        let mut fleet = FleetRollouts {
            rollouts: Vec::new(),
            status: FleetStatus::Complete,
        };
        for index in 0..5 {
            let id = format!("01a08046-2{index:03}-7abc-8abc-00000000000{index}");
            std::fs::write(day.join(format!("rollout-test-{id}.jsonl")), b"{}\n")
                .expect("rollout record");
            fleet.rollouts.push(FleetRollout {
                owner: format!("session:seat-{index}"),
                profile: "codex-profile".to_owned(),
                client: None,
                id,
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = configured_codex_scope(&root);
        let mut budget = Budget {
            files_left: 4_096,
            bytes_left: 0,
            started: std::time::Instant::now(),
            max_elapsed: std::time::Duration::from_secs(1),
        };
        let groups = codex_groups(&scope, &fleet, NOW, &mut budget);
        assert!(groups.rendered.iter().all(|group| {
            group.summary.is_some() || group.rows.iter().all(|row| row.status != Status::Truncated)
        }));
        let summary = groups
            .rendered
            .iter()
            .find_map(|group| group.summary.as_ref())
            .expect("truncated summary");
        assert_eq!(summary.hidden, 5);
        assert_eq!(summary.unreadable, 0);
        assert!(summary.not_read);
        assert_eq!(summary.status, Some(Status::Truncated));
        let rendered = render_at(&groups.rendered, Some(&root), NOW);
        assert!(
            rendered.lines().any(|line| {
                line.contains("+5 rollouts not read") && line.contains("truncated")
            }),
            "{rendered}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn view_facts_carry_real_partial_codex_failures_into_the_scope() {
        const NOW: i64 = 1_788_858_600;
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-settings-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let day = root.join(".codex/sessions/2026/09/08");
        std::fs::create_dir_all(&day).expect("rollout day");
        let record = concat!(
            "{\"timestamp\":\"2026-09-08T09:04:00Z\",\"type\":\"event_msg\",",
            "\"payload\":{\"type\":\"token_count\",\"rate_limits\":{",
            "\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{",
            "\"used_percent\":30,\"window_minutes\":300,\"resets_at\":1788861600}}}}\n",
        );
        let ids = [
            "01a08046-2000-7abc-8abc-000000000000",
            "01a08046-2001-7abc-8abc-000000000001",
        ];
        let paths = ids.map(|id| day.join(format!("rollout-test-{id}.jsonl")));
        for path in &paths {
            std::fs::write(path, record).expect("usable rollout");
        }
        let fleet = FleetRollouts {
            rollouts: ids
                .into_iter()
                .enumerate()
                .map(|(index, id)| FleetRollout {
                    owner: format!("session:seat-{index}"),
                    profile: "codex-profile".to_owned(),
                    client: None,
                    id: id.to_owned(),
                    tool: ToolKind::Codex,
                    location: RolloutLocation::Configured,
                })
                .collect(),
            status: FleetStatus::Complete,
        };
        let scope = Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            configured_profiles: vec!["codex-profile".to_owned()],
            clients: vec!["codex-client".to_owned()],
            accounts: Vec::new(),
            rollouts: Vec::new(),
            hint: Some("configured scope hint".to_owned()),
            manual_resets: None,
            notes: Vec::new(),
            leg: false,
            go: false,
        };

        let project = |groups: super::CodexGroups| {
            super::Observation {
                groups: groups.all,
                rendered: groups.rendered,
                home: Some(root.clone()),
                now: NOW,
            }
            .quota_view()
        };
        let mut short = Budget {
            files_left: 4_096,
            bytes_left: record.len() as u64,
            started: std::time::Instant::now(),
            max_elapsed: std::time::Duration::from_secs(1),
        };
        let truncated = codex_groups(&scope, &fleet, NOW, &mut short);
        assert_eq!(truncated.all.len(), 1, "one usable rollout was retained");
        let summary = truncated
            .rendered
            .iter()
            .find(|group| group.summary.is_some())
            .expect("producer completeness summary");
        assert_eq!(summary.profiles, scope.profiles);
        assert_eq!(summary.hint, scope.hint);
        assert!(super::settings_summary_scope(&truncated.all[0], summary));
        let truncated = project(truncated);
        assert!(
            truncated
                .iter()
                .any(|scope| scope.only.as_deref() == Some("read cut short")
                    && scope.windows.is_empty()),
            "{truncated:?}"
        );

        std::fs::remove_file(&paths[1]).expect("replace one rollout with damage");
        std::fs::create_dir(&paths[1]).expect("an unreadable rollout-shaped entry");
        let failed = project(codex_groups(&scope, &fleet, NOW, &mut Budget::new()));
        assert!(
            failed.iter().any(
                |scope| scope.only.as_deref() == Some("quota file unreadable")
                    && scope.windows.is_empty()
            ),
            "{failed:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_scopes_use_resolved_client_identity_and_vendor_paths() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-resolved-scopes-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("operator home");
        let canonical_root = match crate::run::canonical_config_home(
            &crate::launch_cmd::Resolved::Path(root.clone()),
        )
        .expect("canonical operator home")
        {
            crate::launch_cmd::Resolved::Path(path) => path,
            other => panic!("unexpected canonical home: {other:?}"),
        };
        let cfg = crate::config::IdentityConfig {
            clients: vec![
                (
                    "claude".to_owned(),
                    crate::config::Client {
                        executable: "claude".to_owned(),
                        config_home: None,
                        manual_resets: None,
                        manual_resets_note: None,
                        tool: ToolKind::Claude,
                    },
                ),
                (
                    "mic".to_owned(),
                    crate::config::Client {
                        executable: "claude".to_owned(),
                        config_home: Some("$HOME/.claude-mic".to_owned()),
                        manual_resets: None,
                        manual_resets_note: None,
                        tool: ToolKind::Claude,
                    },
                ),
            ],
            profiles: vec![
                ("default".to_owned(), "claude".to_owned()),
                ("custom".to_owned(), "mic".to_owned()),
                ("moved-home".to_owned(), "HOME=/other claude".to_owned()),
            ],
            ..crate::config::IdentityConfig::default()
        };
        let scopes = configured_scopes(&cfg, Some(&root));
        assert_eq!(scopes.len(), 3);
        assert_eq!(scopes[0].home, Some(canonical_root.join(".claude")));
        assert_eq!(scopes[0].source, Some(canonical_root.join(".claude.json")));
        assert!(scopes[0].clients.is_empty(), "default alias stays concise");
        assert_eq!(scopes[1].home, Some(canonical_root.join(".claude-mic")));
        assert_eq!(
            scopes[1].source,
            Some(canonical_root.join(".claude-mic/.claude.json"))
        );
        assert_eq!(scopes[1].clients, ["mic"]);
        assert_eq!(
            scopes[2].home,
            Some(std::path::PathBuf::from("/other/.claude"))
        );
        assert_eq!(
            scopes[2].source,
            Some(std::path::PathBuf::from("/other/.claude.json"))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_and_custom_claude_homes_read_only_their_own_cache() {
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-quota-claude-homes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let custom = root.join(".claude-mic");
        std::fs::create_dir_all(&custom).expect("custom client home");
        let cache = |percent| {
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":1000000,\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"percent\":{percent},\"resets_at\":\"1970-01-01T01:00:00Z\"}}]}}}}}}"
            )
        };
        std::fs::write(root.join(".claude.json"), cache(11)).expect("default cache");
        std::fs::write(custom.join(".claude.json"), cache(77)).expect("custom cache");
        let read = |home: std::path::PathBuf, source: std::path::PathBuf| {
            let scope = Scope {
                tool: ToolKind::Claude,
                home: Some(home),
                source: Some(source),
                source_key: None,
                profiles: vec!["p".to_owned()],
                configured_profiles: vec!["p".to_owned()],
                clients: Vec::new(),
                accounts: Vec::new(),
                rollouts: Vec::new(),
                hint: None,
                manual_resets: None,
                notes: Vec::new(),
                leg: false,
                go: false,
            };
            let (read, plan) = read_claude(&scope, 1_001, &mut Budget::new());
            assert!(plan.is_none(), "this cache reports no plan");
            match read {
                ReadRows::Rows(observed) => observed
                    .rows
                    .first()
                    .and_then(|row| row.used_percent.clone()),
                _ => None,
            }
        };
        assert_eq!(
            read(root.join(".claude"), root.join(".claude.json")),
            Some("11".to_owned())
        );
        assert_eq!(
            read(custom.clone(), custom.join(".claude.json")),
            Some("77".to_owned())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn effective_headroom_is_derived_only_from_declared_resets_and_reported_credits() {
        let plain = Account::default();
        let unlimited = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(9_900),
            spend_control_reached: None,
            spend_observed_at: None,
        };
        let capped = Account {
            credits: Credits::Exhausted,
            credits_observed_at: Some(9_900),
            spend_control_reached: Some(true),
            spend_observed_at: Some(9_900),
        };
        let balance = Account {
            credits: Credits::Available("12.50".to_owned()),
            credits_observed_at: Some(9_900),
            spend_control_reached: Some(false),
            spend_observed_at: Some(9_900),
        };
        // Every account here is stamped with the window it is judged against,
        // so this table is about the arithmetic, not about provenance.
        let case = |declared, account: &Account, used: f64| {
            let policy = Policy::new(declared, account.clone());
            let effective = effective(&policy, used, Some(9_900));
            (effective.cell(), effective.judged(used))
        };
        assert_eq!(case(None, &plain, 95.0), ("-".to_owned(), 95.0));
        assert_eq!(
            case(Some(0), &plain, 95.0),
            ("95%".to_owned(), 95.0),
            "a declared zero derives nothing: no stray x0"
        );
        assert_eq!(case(Some(1), &plain, 95.0), ("47.5% x1".to_owned(), 47.5));
        assert_eq!(
            case(Some(2), &plain, 95.0),
            ("31.7% x2".to_owned(), 95.0 / 3.0)
        );
        assert_eq!(case(Some(9), &plain, 100.0), ("10% x9".to_owned(), 10.0));
        assert_eq!(
            case(None, &unlimited, 95.0),
            ("0%".to_owned(), 0.0),
            "unlimited credits mean the window does not bind"
        );
        assert_eq!(
            case(Some(2), &capped, 10.0),
            ("100%".to_owned(), 100.0),
            "a spend cap outranks every declared reset"
        );
        assert_eq!(
            case(None, &balance, 95.0),
            ("-".to_owned(), 95.0),
            "an unquantifiable balance never invents headroom"
        );
        let label = |account: &Account| credits_label(&Policy::new(None, account.clone()));
        assert_eq!(label(&plain), "-");
        assert_eq!(label(&unlimited), "unlimited");
        assert_eq!(label(&capped), "spend-cap");
        assert_eq!(label(&balance), "12.50");
        assert_eq!(
            label(&Account {
                credits: Credits::Exhausted,
                credits_observed_at: Some(9_900),
                spend_control_reached: None,
                spend_observed_at: None,
            }),
            "none"
        );
    }

    #[test]
    fn quota_takes_one_json_flag_and_nothing_else() {
        let args = |words: &[&str]| {
            super::parse_args(
                &words
                    .iter()
                    .map(|word| (*word).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(args(&[]), Ok(super::Format::Table));
        assert_eq!(args(&["--json"]), Ok(super::Format::Json));
        assert_eq!(args(&["--json", "--json"]), Err("--json".to_owned()));
        assert_eq!(args(&["--json", "extra"]), Err("extra".to_owned()));
        assert_eq!(args(&["extra", "--json"]), Err("extra".to_owned()));
        assert_eq!(args(&["-x"]), Err("-x".to_owned()));
        assert_eq!(args(&["--JSON"]), Err("--JSON".to_owned()));
    }

    fn declared_group(manual_resets: Option<u8>, used: &str) -> Group {
        Group {
            profiles: vec!["solx".to_owned()],
            tool: ToolKind::Codex,
            home: Some(std::path::PathBuf::from("/tmp/cx")),
            source: Some(std::path::PathBuf::from("/tmp/cx/sessions")),
            clients: vec!["cx".to_owned()],
            rollout: None,
            owner: None,
            rows: vec![Row {
                bucket: "codex".to_owned(),
                qualifier: Some("pro".to_owned()),
                window_minutes: Some(10_080),
                used_percent: Some(used.to_owned()),
                resets_at: Some(20_000),
                observed_at: Some(9_900),
                status: Status::Fresh,
            }],
            hint: None,
            summary: None,
            policy: Policy::new(manual_resets, Account::default()),
            notes: Vec::new(),
            plan: None,
        }
    }

    fn unusable_claude_group(now: i64, observed_at: Option<i64>) -> Group {
        Group {
            profiles: vec!["claude-mic".to_owned()],
            tool: ToolKind::Claude,
            home: Some(std::path::PathBuf::from("/tmp/mic/.claude-mic")),
            source: Some(std::path::PathBuf::from(
                "/tmp/mic/.claude-mic/.claude.json",
            )),
            clients: Vec::new(),
            rollout: None,
            owner: None,
            rows: vec![
                Row {
                    bucket: "session".to_owned(),
                    qualifier: None,
                    window_minutes: Some(300),
                    used_percent: Some("8".to_owned()),
                    resets_at: Some(now - 60),
                    observed_at,
                    status: Status::Unknown,
                },
                Row {
                    bucket: "weekly_all".to_owned(),
                    qualifier: None,
                    window_minutes: Some(10_080),
                    used_percent: Some("59".to_owned()),
                    resets_at: Some(now - 120),
                    observed_at,
                    status: Status::Unknown,
                },
            ],
            hint: None,
            summary: None,
            policy: Policy::default(),
            notes: Vec::new(),
            plan: None,
        }
    }

    #[test]
    fn an_expired_claude_cache_observes_end_to_end_without_storing_its_hint() {
        let root = std::path::PathBuf::from(format!(
            "/tmp/ae-quota-claude-refresh-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".claude-mic")).expect("claude-mic home");
        let config = root.join("config");
        std::fs::write(
            &config,
            "[profiles]\nclaude-mic = CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --model fable\n",
        )
        .expect("identity config");
        let now = crate::time::Timestamp::parse("2026-09-13T09:00:00Z")
            .expect("fixed instant")
            .epoch();
        let fetched = now - 15 * 3_600;
        let cache = format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[\
             {{\"kind\":\"session\",\"percent\":8,\"resets_at\":\"2026-09-12T22:00:00Z\"}},\
             {{\"kind\":\"weekly_all\",\"percent\":59,\"resets_at\":\"2026-09-13T00:00:00Z\"}}]}}}}}}",
            fetched * 1_000
        );
        std::fs::write(root.join(".claude-mic/.claude.json"), cache).expect("claude cache");

        let observation = super::observe(&super::Inputs {
            home: Some(&root),
            global: Some(&config),
            local: None,
            sessions: None,
            now,
        })
        .expect("observation");
        let group = observation
            .rendered
            .iter()
            .find(|group| group.tool == ToolKind::Claude)
            .expect("the claude scope");
        assert!(group.rows.iter().all(|row| row.status == Status::Unknown));
        assert!(
            group.hint.is_none(),
            "the refresh hint is render-time; storing it would poison settings_same_scope"
        );
        let table = render_at(&observation.rendered, Some(&root), now);
        assert!(table.contains("15h00m"), "{table}");
        assert!(table.contains("/usage"), "{table}");
        assert!(!table.contains("8%"), "{table}");
        let scopes = observation.quota_view();
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].title, "claude · ~/.claude-mic");
        assert_eq!(
            scopes[0].only.as_deref(),
            Some(
                "no reading ae can trust (last report 15h00m ago) — run /usage in a claude session or feed ae from your statusline"
            )
        );
        assert!(scopes[0].windows.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn observe_is_observe_full_without_the_history() {
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-quota-observe-full-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".claude-mic")).expect("claude-mic home");
        let config = root.join("config");
        std::fs::write(
            &config,
            "[profiles]\nclaude-mic = CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --model fable\n",
        )
        .expect("identity config");
        let now = 1_788_858_600;
        let cache = format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[\
             {{\"kind\":\"session\",\"percent\":8,\"resets_at\":\"2026-09-08T11:00:00Z\"}}]}}}}}}",
            (now - 60) * 1_000
        );
        std::fs::write(root.join(".claude-mic/.claude.json"), cache).expect("claude cache");
        let inputs = super::Inputs {
            home: Some(&root),
            global: Some(&config),
            local: None,
            sessions: None,
            now,
        };
        let (full, series) = super::observe_full(&inputs).expect("full read");
        assert_eq!(
            super::observe(&inputs),
            Ok(full),
            "the same observation, exactly"
        );
        assert!(series.is_empty(), "a claude cache keeps no history");
        // The error path is the same one too: a directory is not a config file.
        let bad = super::Inputs {
            global: Some(&root),
            ..inputs
        };
        assert!(super::observe(&bad).is_err());
        assert_eq!(
            super::observe(&bad).map(|_| ()),
            super::observe_full(&bad).map(|_| ()),
            "one failure, whichever door"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_grok_log_observes_one_credit_window_and_clients_on_one_home_share_it() {
        let root = std::path::PathBuf::from(format!("/tmp/ae-quota-grok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".grok/logs")).expect("grok logs");
        let config = root.join("config");
        std::fs::write(
            &config,
            "[clients]\ngx = grok\ngy = grok\n[profiles]\na = gx --model grok-4.6\nb = gy --model grok-4.6\n",
        )
        .expect("identity config");
        let now = 1_790_769_600;
        let record = "{\"ts\":\"2026-09-30T11:55:00Z\",\"msg\":\"billing: fetched credits config\",\"ctx\":{\"config\":{\"creditUsagePercent\":37,\"billingPeriodStart\":\"2026-09-28T00:00:00Z\",\"billingPeriodEnd\":\"2026-10-05T00:00:00Z\"},\"onDemandEnabled\":true,\"subscriptionTier\":\"SuperGrok\"}}\n";
        std::fs::write(root.join(".grok/logs/unified.jsonl"), record).expect("grok log");
        let observation = super::observe(&super::Inputs {
            home: Some(&root),
            global: Some(&config),
            local: None,
            sessions: None,
            now,
        })
        .expect("observation");
        let groups: Vec<&Group> = observation
            .groups
            .iter()
            .filter(|group| group.tool == ToolKind::Grok)
            .collect();
        assert_eq!(groups.len(), 1, "one HOME is one account: {groups:?}");
        let group = groups[0];
        assert!(group.source.is_some(), "the advisory path needs a source");
        assert_eq!(group.profiles, ["a", "b"]);
        assert_eq!(group.rows.len(), 1);
        assert_eq!(group.rows[0].bucket, "credits");
        assert_eq!(group.rows[0].used_percent.as_deref(), Some("37"));
        assert_eq!(group.rows[0].status, Status::Fresh);
        assert_eq!(
            group.notes,
            ["on-demand spend enabled: a full window may not block work"]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_an_agy_command_on_the_callers_home_asks_agy_once() {
        use {super::AgyRun, std::path::Path, std::time::Duration};
        let root = std::path::PathBuf::from(format!("/tmp/ae-agy-leg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch home");
        let config = root.join("config");
        // A claude cache 900 s old is fresh at `now` and stale one second later.
        let now = 1_788_858_600;
        let cache = format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[\
             {{\"kind\":\"session\",\"percent\":8,\"resets_at\":\"2026-09-08T11:00:00Z\"}}]}}}}}}",
            (now - 900) * 1_000
        );
        std::fs::write(root.join(".claude.json"), cache).expect("claude cache");
        // No antigravity-cli store: agy is still asked. The last leg edits the config.
        let here = Some(root.as_path());
        // `kept` counts the agy profiles left unasked, each still on its own `unsupported` row.
        for (profiles, home, edit, calls, kept) in [
            ("c = claude\na = agy\nb = agy -m\n", here, "", 1, 0),
            ("c = claude\na = HOME=/x agy\n", here, "", 0, 1),
            ("c = claude\na = agy\n", None, "", 0, 1),
            (
                "c = claude\na = agy\n",
                here,
                "[profiles]\nb = claude\n",
                1,
                0,
            ),
            ("c = claude\na = agy\nb = HOME=/x agy\n", here, "", 1, 1),
        ] {
            std::fs::write(&config, format!("[profiles]\n{profiles}")).expect("identity config");
            let inputs = super::Inputs {
                home,
                global: Some(&config),
                local: None,
                sessions: None,
                now,
            };
            let unasked = [placeholder(Status::Unsupported)];
            for group in super::observe(&inputs).expect("observation").groups {
                assert!(group.tool != ToolKind::Agy || group.rows == unasked);
            }
            let asked = std::cell::Cell::new(0);
            let leg = |home: &Path| {
                asked.set(asked.get() + usize::from(home == root));
                std::thread::sleep(Duration::from_millis(1));
                assert!(edit.is_empty() || std::fs::write(&config, edit).is_ok());
                AgyRun::Failed
            };
            let mut out = Vec::new();
            let code = super::run_with(
                &inputs,
                super::Format::Table,
                &mut out,
                &mut Vec::new(),
                leg,
            )
            .ok();
            let table = String::from_utf8_lossy(&out);
            let (errors, stale) = (table.matches("read-error").count(), table.contains("stale"));
            let unsupported = table.matches("unsupported").count();
            let got = (code, asked.get(), errors, stale, unsupported);
            assert_eq!(got, (Some(0), calls, calls, calls == 1, kept), "{table}");
        }
        let ceil = |millis| super::completed(10, Duration::from_millis(millis));
        assert_eq!([0, 1, 1000, 1001].map(ceil), [10, 11, 11, 12]);
        let _ = std::fs::remove_dir_all(root);
    }

    use super::view::{Level, Why};

    fn viewed(groups: Vec<super::Group>) -> Vec<super::view::Scope> {
        super::Observation {
            groups,
            rendered: Vec::new(),
            home: None,
            now: 10_000,
        }
        .quota_view()
    }

    fn named(client: &str, used: &str, resets: Option<u8>) -> super::Group {
        let mut group = declared_group(resets, used);
        group.clients = vec![client.to_owned()];
        group
    }

    #[test]
    fn view_facts_list_every_window_with_derivation_cells() {
        let mut group = declared_group(Some(1), "90");
        group.rows[0].bucket = "session".to_owned();
        group.rows[0].qualifier = None;
        group.rows[0].window_minutes = Some(300);
        group.rows[0].resets_at = Some(10_600);
        group.rows.push(Row {
            bucket: "weekly_all".to_owned(),
            qualifier: None,
            window_minutes: Some(10_080),
            used_percent: Some("12".to_owned()),
            resets_at: Some(20_000),
            observed_at: Some(9_900),
            status: Status::Fresh,
        });
        let scopes = viewed(vec![group]);
        assert_eq!(scopes.len(), 1);
        let scope = &scopes[0];
        assert_eq!(scope.title, "codex · cx");
        assert_eq!(scope.profiles, "solx");
        assert!(scope.account.is_none());
        let names: Vec<&str> = scope.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Session 5h", "Weekly 7d"]);
        let first = &scope.windows[0];
        assert_eq!(
            (first.raw.as_deref(), first.counts.as_deref()),
            (Some("90%"), Some("45%"))
        );
        assert_eq!(first.why, Some(Why::Resets(1)));
        assert_eq!(first.fill, Some(4));
        assert_eq!(first.level, Level::Headroom);
        assert!(!first.stale);
        assert_eq!(
            (first.resets.as_deref(), first.seen.as_deref()),
            (Some("10m"), Some("1m"))
        );
        assert_eq!(scope.windows[1].counts.as_deref(), Some("6%"));
    }

    #[test]
    fn view_facts_keep_windows_in_stable_bucket_order() {
        let mut group = declared_group(None, "100");
        group.rows[0].bucket = "weekly".to_owned();
        group.rows[0].qualifier = Some("pro".to_owned());
        group.rows.push(Row {
            bucket: "session".to_owned(),
            qualifier: Some("team".to_owned()),
            window_minutes: Some(300),
            used_percent: Some("100".to_owned()),
            resets_at: Some(20_000),
            observed_at: Some(9_900),
            status: Status::Fresh,
        });
        group.rows.push(Row {
            bucket: "monthly".to_owned(),
            qualifier: Some("enterprise".to_owned()),
            window_minutes: Some(43_200),
            used_percent: Some("100".to_owned()),
            resets_at: Some(20_000),
            observed_at: Some(9_900),
            status: Status::Fresh,
        });
        let mut flipped = group.clone();
        flipped.rows.reverse();
        let names = |group| {
            viewed(vec![group])[0]
                .windows
                .iter()
                .map(|w| w.name.clone())
                .collect::<Vec<_>>()
        };
        let (forward, reverse) = (names(group), names(flipped));
        assert_eq!(forward, reverse);
        assert_eq!(
            forward,
            ["monthly enterprise 30d", "Session team 5h", "weekly pro 7d"]
        );
    }

    #[test]
    fn view_facts_sort_scopes_by_available_headroom() {
        let mut stale = named("stale30", "30", None);
        stale.rows[0].observed_at = Some(9_000);
        let mut capped = named("capped", "10", None);
        capped.policy = Policy::new(
            None,
            Account {
                credits: Credits::Available("12.40".to_owned()),
                credits_observed_at: Some(9_900),
                spend_control_reached: Some(true),
                spend_observed_at: Some(9_900),
            },
        );
        let mut unknown = named("unknown", "0", None);
        unknown.rows = vec![super::placeholder(Status::Unknown)];
        let scopes = viewed(vec![
            named("calm", "30", None),
            named("low85", "85", None),
            capped,
            stale,
            named("low90", "90", None),
            unknown,
        ]);
        let titles: Vec<&str> = scopes.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "codex · calm",
                "codex · stale30",
                "codex · low85",
                "codex · low90",
                "codex · capped",
                "codex · unknown"
            ]
        );
        assert_eq!(
            scopes[4].account.as_deref(),
            Some("credits 12.40 · spend cap reached")
        );
    }

    #[test]
    fn a_feed_read_is_charged_to_the_budget_whatever_the_record_holds() {
        let root =
            std::path::PathBuf::from(format!("/tmp/ae-quota-feed-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(super::feed::DIR);
        std::fs::create_dir_all(&dir).expect("feed directory");
        let (key, sessions) = (Path::new("/k/.claude.json"), root.join("sessions"));
        let charged = |bytes: Option<&[u8]>, budget: &mut Budget| {
            let path = dir.join(super::feed::file_name(key));
            let _ = std::fs::remove_file(&path);
            if let Some(bytes) = bytes {
                std::fs::write(&path, bytes).expect("record");
            }
            let before = budget.bytes_left;
            let rows = super::read_feed(Some(&sessions), Some(key), 1_000, budget).len();
            (rows, before - budget.bytes_left)
        };
        let good =
            "ae-quota-feed 1\nscope /k/.claude.json\nreceived 999\nwindow session 1 4070908800\n";
        let foreign = good.replace("/k/", "/other/");
        let mut budget = Budget::new();
        let len = |text: &str| u64::try_from(text.len()).unwrap_or(u64::MAX);
        assert_eq!(charged(Some(b"damaged"), &mut budget), (0, 7));
        assert_eq!(
            charged(Some(foreign.as_bytes()), &mut budget),
            (0, len(&foreign))
        );
        assert_eq!(charged(Some(good.as_bytes()), &mut budget), (1, len(good)));
        // The capped read takes one byte past the cap to see an oversized
        // record, so that byte is charged too, however long the file is.
        assert_eq!(charged(Some(&[b'x'; 4096]), &mut budget), (0, 1025));
        assert_eq!(charged(None, &mut budget), (0, 0));
        budget.bytes_left = 1025;
        assert_eq!(charged(Some(good.as_bytes()), &mut budget), (1, len(good)));
        budget.bytes_left = 1024;
        assert_eq!(charged(Some(good.as_bytes()), &mut budget), (0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_relative_claude_config_dir_names_no_feed_scope_even_where_it_exists() {
        // `.` always exists under the cwd, so only the relative refusal keeps it
        // from canonicalising into a scope of whatever directory ae runs in.
        for dir in ["", ".", "relative"] {
            let scope =
                super::claude_feed_scope(Some(std::ffi::OsStr::new(dir)), Some(Path::new("/h")));
            assert_eq!(scope, None, "{dir:?}");
        }
    }

    #[test]
    fn a_fed_window_joins_the_cache_by_its_stamp_and_never_hides_damage() {
        let row = |bucket: &str, qualifier: Option<&str>, observed_at: Option<i64>| super::Row {
            bucket: bucket.to_owned(),
            qualifier: qualifier.map(str::to_owned),
            window_minutes: Some(300),
            used_percent: Some("1".to_owned()),
            resets_at: Some(9),
            observed_at,
            status: Status::Fresh,
        };
        let cache = |rows| {
            ReadRows::Rows(super::Observed {
                rows,
                account: super::Account::default(),
                series: Vec::new(),
            })
        };
        let seen = |read| match read {
            ReadRows::Rows(observed) => observed
                .rows
                .into_iter()
                .map(|row| (row.bucket, row.qualifier.is_some(), row.observed_at))
                .collect(),
            _ => Vec::new(),
        };
        let fed = || {
            vec![
                row("session", None, Some(5)),
                row("weekly_all", None, Some(5)),
            ]
        };
        assert!(matches!(
            super::with_feed(ReadRows::Missing, Vec::new()),
            ReadRows::Missing
        ));
        assert_eq!(
            seen(super::with_feed(ReadRows::Failed, fed())),
            [
                ("-".to_owned(), false, None),
                ("session".to_owned(), false, Some(5)),
                ("weekly_all".to_owned(), false, Some(5))
            ]
        );
        let held = vec![
            row("session", None, None),
            row("weekly_all", None, Some(7)),
            row("weekly_all", Some("Fable"), Some(1)),
        ];
        assert_eq!(
            seen(super::with_feed(cache(held), fed())),
            [
                ("session".to_owned(), false, Some(5)),
                ("weekly_all".to_owned(), false, Some(7)),
                ("weekly_all".to_owned(), true, Some(1)),
            ]
        );
        assert!(super::strictly_newer(None, Some(i64::MIN)));
        assert!(!super::strictly_newer(Some(5), Some(5)));
    }

    #[test]
    fn scope_order_uses_worst_effective_window_and_keeps_ties_and_unknowns_stable() {
        let mut stale = named("stale", "20", None);
        stale.rows[0].observed_at = Some(9_000);
        let mut capped = named("capped", "5", Some(9));
        capped.policy.account = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(9_900),
            spend_control_reached: Some(true),
            spend_observed_at: Some(9_900),
        };
        let mut unlimited = named("unlimited", "100", None);
        unlimited.policy.account = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(9_900),
            ..Account::default()
        };
        let mut worst = named("worst", "1", None);
        let mut weekly = worst.rows[0].clone();
        weekly.bucket = "weekly".to_owned();
        weekly.used_percent = Some("70".to_owned());
        worst.rows.push(weekly);
        let missing = |client, status| {
            let mut group = named(client, "0", None);
            group.rows[0].status = status;
            group
        };
        let groups = vec![
            missing("unknown", Status::Unknown),
            worst,
            capped,
            stale,
            named("reset", "90", Some(2)),
            named("tie", "30", None),
            missing("unsupported", Status::Unsupported),
            unlimited,
            named("exhausted", "150", None),
            missing("unreadable", Status::ReadError),
            named("invalid", "NaN", None),
        ];
        let expected = [
            "unlimited",
            "stale",
            "reset",
            "tie",
            "worst",
            "capped",
            "exhausted",
            "unknown",
            "unsupported",
            "unreadable",
            "invalid",
        ];
        let table = render_at(&groups, None, 10_000);
        let positions: Vec<usize> = expected
            .iter()
            .map(|client| table.find(&format!("codex · {client}")).expect("scope row"))
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{table}"
        );
        let scopes = viewed(groups);
        let heads: Vec<String> = super::view::layout(
            &scopes,
            super::view::Geometry {
                width: 160,
                rows: None,
                icons: false,
            },
        )
        .into_iter()
        .filter(|line| line.tone == super::view::Tone::Head)
        .map(|line| {
            line.text
                .split("   ")
                .next()
                .expect("scope title")
                .to_owned()
        })
        .collect();
        let expected: Vec<String> = expected
            .iter()
            .map(|client| format!("codex · {client}"))
            .collect();
        assert_eq!(heads, expected, "dialog and app use this shared layout");
    }

    #[test]
    fn scope_order_keeps_rollouts_together_and_uses_the_newest_window() {
        let mut first = named("multi", "10", None);
        first.owner = Some("first".to_owned());
        let mut hidden = first.clone();
        hidden.owner = Some("hidden".to_owned());
        hidden.rows[0].used_percent = Some("95".to_owned());
        hidden.rows[0].observed_at = Some(9_800);
        let mut second = first.clone();
        second.owner = Some("second".to_owned());
        let other = named("other", "50", None);
        let observation = super::Observation {
            groups: vec![hidden, first.clone(), second.clone(), other.clone()],
            rendered: vec![first, second, other],
            home: None,
            now: 10_000,
        };
        let table = super::render_observation_at(&observation);
        let other = table.find("codex · other").expect("other scope");
        let first = table.find("first").expect("first owner");
        let second = table.find("second").expect("second owner");
        assert!(first < second && second < other, "{table}");
        assert!(
            !table.contains("hidden"),
            "display cap is unchanged: {table}"
        );
        let scopes = observation.quota_view();
        assert_eq!(
            scopes
                .iter()
                .map(|scope| scope.title.as_str())
                .collect::<Vec<_>>(),
            ["codex · multi", "codex · other"]
        );
        assert_eq!(scopes[0].windows[0].raw.as_deref(), Some("10%"));
    }

    #[test]
    fn scope_order_keeps_rendered_only_unresolved_summaries() {
        let mut rollout = named("unresolved", "10", None);
        rollout.source = None;
        rollout.rollout = Some("one".to_owned());
        let mut summary = rollout.clone();
        summary.rollout = None;
        summary.rows.clear();
        summary.summary = Some(super::RolloutSummary {
            hidden: 2,
            unreadable: 1,
            oldest_observed: Some(9_900),
            not_read: false,
            status: Some(Status::ReadError),
        });
        let other = named("other", "50", None);
        let observation = super::Observation {
            groups: vec![rollout.clone(), other.clone()],
            rendered: vec![rollout, summary, other],
            home: None,
            now: 10_000,
        };
        let table = super::render_observation_at(&observation);
        assert!(
            table.contains("read-error"),
            "summary must stay visible: {table}"
        );
        assert!(
            table.contains("+2 rollouts not shown (1 unreadable)"),
            "hidden count must stay visible: {table}"
        );
        assert!(table.contains("codex · unresolved"), "{table}");
        assert!(table.contains("codex · other"), "{table}");
    }

    #[test]
    fn scope_order_keeps_unreadable_scopes_after_usable_siblings() {
        for status in [Status::ReadError, Status::Truncated] {
            let good = named("incomplete", "1", None);
            let mut incomplete = good.clone();
            incomplete.rows = vec![super::placeholder(status)];
            let groups = vec![good, named("usable", "100", None), incomplete];
            let table = render_at(&groups, None, 10_000);
            assert!(
                table.find("codex · usable").expect("usable scope")
                    < table.find("codex · incomplete").expect("incomplete scope"),
                "{table}"
            );
            let scopes = viewed(groups);
            assert_eq!(scopes[0].title, "codex · usable");
            assert_eq!(scopes[1].title, "codex · incomplete");
            assert!(scopes[1].only.is_some());
            assert!(scopes[1].windows.is_empty());
        }
    }

    #[test]
    fn scope_order_judges_the_newest_window_under_merged_account_policy() {
        let window = named("credited", "80", None);
        let mut account = window.clone();
        account.rows = vec![super::placeholder(Status::Unknown)];
        account.policy.account = Account {
            credits: Credits::Unlimited,
            credits_observed_at: Some(9_950),
            ..Account::default()
        };
        let groups = vec![named("other", "20", None), window, account];
        let table = render_at(&groups, None, 10_000);
        assert!(
            table.find("codex · credited").expect("credited scope")
                < table.find("codex · other").expect("other scope"),
            "{table}"
        );
        let scopes = viewed(groups);
        assert_eq!(scopes[0].title, "codex · credited");
        assert_eq!(scopes[0].windows[0].counts.as_deref(), Some("0%"));
        assert_eq!(scopes[0].windows[0].why, Some(Why::Unlimited));
    }

    #[test]
    fn view_facts_report_unusable_scopes_without_numbers() {
        for (index, status) in [
            Status::Unknown,
            Status::Unsupported,
            Status::ReadError,
            Status::Truncated,
        ]
        .iter()
        .enumerate()
        {
            let mut group = named(&format!("client-{index}"), "0", None);
            group.rows = vec![super::placeholder(*status)];
            let scopes = viewed(vec![group]);
            assert_eq!(scopes.len(), 1);
            assert!(scopes[0].windows.is_empty());
            let want = match status {
                Status::Unsupported => "this client reports no quota",
                Status::ReadError => "quota file unreadable",
                Status::Truncated => "read cut short",
                _ => "no reading ae can trust",
            };
            assert_eq!(scopes[0].only.as_deref(), Some(want));
        }
        let scopes = viewed(vec![unusable_claude_group(10_000, Some(9_900))]);
        assert_eq!(
            scopes[0].only.as_deref(),
            Some(
                "no reading ae can trust (last report 1m ago) — run /usage in a claude session or feed ae from your statusline"
            )
        );
        let scopes = viewed(vec![unusable_claude_group(10_000, None)]);
        assert_eq!(
            scopes[0].only.as_deref(),
            Some(
                "no reading ae can trust — run /usage in a claude session or feed ae from your statusline"
            )
        );
        let scopes = viewed(vec![unusable_claude_group(
            10_000,
            Some(10_000 + FUTURE_SKEW_SECS),
        )]);
        assert_eq!(
            scopes[0].only.as_deref(),
            Some(
                "no reading ae can trust — run /usage in a claude session or feed ae from your statusline"
            )
        );
    }

    #[test]
    fn view_facts_keep_incomplete_evidence_without_sibling_numbers() {
        let mut good = named("cx", "30", None);
        good.rollout = Some("good".to_owned());
        let mut incomplete = good.clone();
        incomplete.rollout = Some("incomplete".to_owned());
        incomplete.rows = vec![super::placeholder(Status::ReadError)];
        let scopes = viewed(vec![good, incomplete]);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].only.as_deref(), Some("quota file unreadable"));
        assert!(scopes[0].windows.is_empty());
    }

    #[test]
    fn view_facts_sanitize_hostile_cells_before_ae_glyphs() {
        let mut group = named(
            &format!("#[fg=red]#\u{1b}[31m\u{7}{}中⚙️", "long".repeat(40)),
            "80",
            None,
        );
        group.rows[0].bucket = format!("bucket-{}", "x".repeat(80));
        let scopes = viewed(vec![group]);
        let title = &scopes[0].title;
        assert!(title.contains("#[fg=red]#"), "{title}");
        assert!(title.contains("..."), "{title}");
        assert!(
            title.chars().all(|ch| ch == '·' || ch.is_ascii()),
            "{title}"
        );
        assert!(!title.contains('中') && !title.contains('⚙'), "{title}");
        let tail = |tail: &str| {
            let pad = "scope-".repeat(8);
            let mut entry = named(&format!("{pad}claude{tail}"), "80", None);
            entry.source = Some(std::path::PathBuf::from(format!("/s{tail}")));
            entry
        };
        let scopes = viewed(vec![tail(""), tail("-mic")]);
        assert!(scopes[0].title.contains("...") && scopes[0].title.contains("claude"));
        assert!(!scopes[0].title.contains("claude-mic"));
        assert!(scopes[1].title.contains("claude-mic"));
    }

    #[test]
    fn view_facts_name_why_the_judged_moved() {
        let reset = named("reset", "90", Some(1));
        let mut unlimited = named("unlimited", "90", None);
        unlimited.policy = Policy::new(
            None,
            Account {
                credits: Credits::Unlimited,
                credits_observed_at: Some(9_900),
                spend_control_reached: None,
                spend_observed_at: None,
            },
        );
        let mut capped = named("capped", "10", None);
        capped.policy = Policy::new(
            None,
            Account {
                credits: Credits::Unlimited,
                credits_observed_at: Some(9_900),
                spend_control_reached: Some(true),
                spend_observed_at: Some(9_900),
            },
        );
        let scopes = viewed(vec![
            reset,
            unlimited,
            capped,
            named("zero", "90", Some(0)),
            named("huge", "1e100", Some(1)),
        ]);
        let by = |client: &str| {
            scopes
                .iter()
                .find(|s| s.title == format!("codex · {client}"))
                .expect("scope")
        };
        assert_eq!(by("reset").windows[0].counts.as_deref(), Some("45%"));
        assert_eq!(by("reset").windows[0].why, Some(Why::Resets(1)));
        assert_eq!(by("unlimited").windows[0].counts.as_deref(), Some("0%"));
        assert_eq!(
            by("unlimited").account.as_deref(),
            Some("credits unlimited")
        );
        assert_eq!(by("capped").windows[0].counts.as_deref(), Some("100%"));
        assert_eq!(by("capped").account.as_deref(), Some("spend cap reached"));
        assert_eq!(by("zero").windows[0].counts, None);
        assert_eq!(by("huge").windows[0].raw.as_deref(), Some("100%"));
        assert_eq!(by("huge").windows[0].counts, None);
        let observed = named("cx", "10", None);
        let mut account_only = observed.clone();
        account_only.rows = vec![super::placeholder(Status::Unknown)];
        account_only.policy = Policy::new(
            None,
            Account {
                credits: Credits::Unreported,
                credits_observed_at: None,
                spend_control_reached: Some(true),
                spend_observed_at: Some(9_950),
            },
        );
        let merged = viewed(vec![observed, account_only]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].windows[0].counts.as_deref(), Some("100%"));
    }

    #[test]
    fn conflicting_declarations_for_one_scope_take_the_minimum_in_either_order() {
        let merged = |first: Option<u8>, second: Option<u8>| {
            let mut scope = Scope {
                tool: ToolKind::Codex,
                home: Some(std::path::PathBuf::from("/tmp/cx")),
                source: Some(std::path::PathBuf::from("/tmp/cx/sessions")),
                source_key: Some(std::path::PathBuf::from("/tmp/cx/sessions")),
                profiles: vec!["ax".to_owned()],
                configured_profiles: vec!["ax".to_owned()],
                clients: vec!["a".to_owned()],
                accounts: Vec::new(),
                rollouts: Vec::new(),
                hint: None,
                manual_resets: first,
                notes: Vec::new(),
                leg: false,
                go: false,
            };
            merge_declaration(
                &mut scope,
                &Declaration {
                    manual_resets: second,
                    note: None,
                },
            );
            (scope.manual_resets, scope.notes)
        };
        for (first, second) in [(Some(0), Some(1)), (Some(1), Some(0))] {
            let (resets, notes) = merged(first, second);
            assert_eq!(
                resets,
                Some(0),
                "an explicit zero is never overruled by an optimistic sibling: {first:?} then {second:?}"
            );
            assert!(
                notes
                    .iter()
                    .any(|note| note.contains("0 and 1") && note.contains("using 0")),
                "the conflict stays visible and says which count won: {notes:?}"
            );
        }
        assert_eq!(
            merged(Some(2), Some(2)),
            (Some(2), Vec::new()),
            "agreement is silent"
        );
        assert_eq!(
            merged(None, Some(3)).0,
            Some(3),
            "a lone declaration stands"
        );
        assert_eq!(
            merged(Some(3), None).0,
            Some(3),
            "an undeclared sibling claims nothing"
        );
    }

    #[test]
    fn the_table_cell_and_the_threshold_read_one_derivation() {
        let group = declared_group(Some(1), "95.0");
        let derivation =
            derived(&group.policy, &group.rows[0]).expect("a fresh percentage derives");
        // Bit equality: the pin is that one value is shared, not that two
        // near-enough values agree.
        assert_eq!(derivation.judged().to_bits(), 47.5_f64.to_bits());
        assert_eq!(derivation.cell(), "47.5% x1");
        let table = render_at(std::slice::from_ref(&group), None, 10_000);
        assert!(
            table.contains(&derivation.cell()),
            "the table renders the one derivation: {table}"
        );
        let zero = declared_group(Some(0), "95.0");
        let zero_derivation =
            derived(&zero.policy, &zero.rows[0]).expect("a fresh percentage derives");
        assert_eq!(zero_derivation.judged().to_bits(), 95.0_f64.to_bits());
        assert_eq!(zero_derivation.cell(), "95%");
        assert!(
            derived(
                &declared_group(Some(1), "not-a-number").policy,
                &declared_group(Some(1), "not-a-number").rows[0]
            )
            .is_none()
        );
    }

    #[test]
    fn a_permissive_credit_fact_is_not_trusted_older_than_the_window_it_relieves() {
        let unlimited_at = |at: Option<i64>| Account {
            credits: Credits::Unlimited,
            credits_observed_at: at,
            spend_control_reached: None,
            spend_observed_at: None,
        };
        let row_at = 9_900;
        let judge = |account: &Account| {
            let effective = effective(&Policy::new(None, account.clone()), 95.0, Some(row_at));
            (effective.cell(), effective.judged(95.0))
        };
        assert_eq!(
            judge(&unlimited_at(Some(row_at))),
            ("0%".to_owned(), 0.0),
            "credits reported with the window they relieve are trusted"
        );
        assert_eq!(
            judge(&unlimited_at(Some(row_at + 60))),
            ("0%".to_owned(), 0.0),
            "and so are credits newer than it"
        );
        assert_eq!(
            judge(&unlimited_at(Some(row_at - 1))),
            ("-".to_owned(), 95.0),
            "a claim older than the window it would relieve is not evidence about it"
        );
        assert_eq!(
            judge(&unlimited_at(None)),
            ("-".to_owned(), 95.0),
            "an unstamped claim is never trusted"
        );

        // The restrictive direction never ages out: keeping a cap can only
        // understate headroom, which is the safe error.
        let stale_cap = Account {
            credits: Credits::Unreported,
            credits_observed_at: None,
            spend_control_reached: Some(true),
            spend_observed_at: Some(row_at - 100_000),
        };
        assert_eq!(judge(&stale_cap), ("100%".to_owned(), 100.0));

        // The table says why a reported claim did not move the number.
        let mut group = declared_group(None, "95.0");
        group.policy = Policy::new(None, unlimited_at(Some(row_at - 1)));
        let table = render_at(std::slice::from_ref(&group), None, 10_000);
        assert!(
            table.contains("unlimited"),
            "the fact is still reported: {table}"
        );
        assert!(
            table.contains("older than the newest window here, so the claim is ignored"),
            "and the table says why it was not used: {table}"
        );
        assert!(
            !table.contains("keeps the raw window"),
            "the claim is ignored, not a rule of its own: {table}"
        );

        // Ignoring the claim leaves every other rule in force, so the note must
        // not promise the raw window either.
        let mut declared = declared_group(Some(1), "95.0");
        declared.policy = Policy::new(Some(1), unlimited_at(Some(row_at - 1)));
        let table = render_at(std::slice::from_ref(&declared), None, 10_000);
        assert!(
            table.contains("47.5% x1"),
            "the declared reset still derives the cell: {table}"
        );
        assert!(
            table.contains("EFFECTIVE follows the remaining rules"),
            "{table}"
        );
    }

    #[test]
    fn the_documented_width_ceiling_is_the_sum_of_the_column_caps() {
        let separators = 2 * (COLUMNS - 1);
        assert_eq!(
            TABLE_MAX_WIDTHS.iter().sum::<usize>() + separators,
            TABLE_MAX_LINE,
            "the documented ceiling and the caps that produce it must agree"
        );
        let widest = render_table(&[RenderLine::Cells(std::array::from_fn(|column| {
            "W".repeat(TABLE_MAX_WIDTHS[column] + 40)
        }))]);
        assert!(
            widest
                .lines()
                .all(|line| line.chars().count() <= TABLE_MAX_LINE),
            "a maximal row stays inside the ceiling"
        );
    }

    #[test]
    fn percentages_trim_only_fractional_zeroes() {
        assert_eq!(percent_label("100"), "100%");
        assert_eq!(percent_label("12.0"), "12%");
        assert_eq!(percent_label("3.50"), "3.5%");
    }

    #[test]
    fn table_cells_replace_controls_and_terminal_escape_sequences() {
        assert_eq!(sanitize_cell("safe\u{1b}[2J\r\n\u{7f}tail"), "safe????tail");
        assert_eq!(sanitize_cell("a\u{1b}]0;title\u{7}b"), "a?b");
    }

    #[test]
    fn long_profile_lists_are_summarized_within_the_column_cap() {
        let profiles: Vec<_> = (0..50).map(|index| format!("profile-{index}")).collect();
        let label = profiles_label(&profiles);
        assert_eq!(
            label, "50 profiles: profile-0 profile-1 profile-2",
            "the first three profile names stay intact"
        );
        let rendered = render_table(&[RenderLine::Cells([
            label,
            "scope".to_owned(),
            "bucket".to_owned(),
            "5h".to_owned(),
            "1%".to_owned(),
            "-".to_owned(),
            "-".to_owned(),
            "-".to_owned(),
            "in 1h".to_owned(),
            "1m ago".to_owned(),
            "fresh".to_owned(),
        ])]);
        assert!(rendered.contains("profile-2"), "{rendered}");
        assert!(rendered.lines().all(|line| line.chars().count() <= 160));
    }

    #[cfg(unix)]
    #[test]
    fn quota_cache_reader_refuses_symlinks_and_oversized_regular_files() {
        use std::os::unix::fs::symlink;

        let dir = std::path::PathBuf::from(format!("/tmp/ae-quota-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fixture directory");
        let regular = dir.join("regular");
        std::fs::write(&regular, b"{}").expect("regular source");
        let link = dir.join("link");
        symlink(&regular, &link).expect("source symlink");
        assert!(bounded_whole_file(&link, CLAUDE_MAX_BYTES, &mut Budget::new()).is_err());

        let oversized = dir.join("oversized");
        let file = std::fs::File::create(&oversized).expect("oversized source");
        file.set_len(CLAUDE_MAX_BYTES + 1)
            .expect("sparse oversized source");
        assert!(bounded_whole_file(&oversized, CLAUDE_MAX_BYTES, &mut Budget::new()).is_err());

        let rollouts = dir.join("sessions/2026/09/08");
        std::fs::create_dir_all(&rollouts).expect("rollout tree");
        let id = "01a08046-1974-7352-ade3-81a786200795";
        symlink(
            &regular,
            rollouts.join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
        )
        .expect("rollout symlink");
        assert!(find_codex_rollout(&dir.join("sessions"), id, &mut Budget::new()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_tail_defines_left_and_right_record_boundaries_within_its_cap() {
        let dir = std::path::PathBuf::from(format!("/tmp/ae-quota-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tail fixture directory");
        let line = concat!(
            r#"{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":7,"window_minutes":300,"resets_at":1788861600}}}}"#,
            "\n"
        );
        let aligned = dir.join("aligned");
        std::fs::write(&aligned, format!("x\n{line}")).expect("aligned tail");
        let cap = u64::try_from(line.len() + 1).expect("bounded cap");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&aligned, cap, &mut Budget::new()).expect("bounded aligned tail")
        else {
            panic!("aligned tail was truncated");
        };
        assert!(starts);
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .len(),
            1
        );

        let unaligned = dir.join("unaligned");
        std::fs::write(&unaligned, format!("abcdef\n{line}")).expect("unaligned tail");
        let cap = u64::try_from(line.len() + 4).expect("bounded cap");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&unaligned, cap, &mut Budget::new()).expect("bounded unaligned tail")
        else {
            panic!("unaligned tail was truncated");
        };
        assert!(!starts);
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .len(),
            1
        );

        let growing = dir.join("growing");
        std::fs::write(&growing, vec![b'x'; 128]).expect("oversized record");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&growing, 32, &mut Budget::new()).expect("bounded growing file")
        else {
            panic!("growing file was truncated by invocation budget");
        };
        assert!(bytes.len() <= 31);
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .is_empty()
        );

        let eof = dir.join("eof");
        std::fs::write(&eof, line.trim_end()).expect("incomplete EOF record");
        let Bounded::Ready((bytes, starts)) =
            bounded_tail(&eof, 1_024, &mut Budget::new()).expect("bounded EOF tail")
        else {
            panic!("EOF file was truncated");
        };
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn codex_tail_stays_bounded_when_the_open_file_grows_or_truncates() {
        let line = concat!(
            r#"{"timestamp":"2026-09-08T09:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":7,"window_minutes":300,"resets_at":1788861600}}}}"#,
            "\n"
        );
        let initial_len = u64::try_from(line.len()).expect("bounded fixture length");
        let mut grown = std::io::Cursor::new(format!("{line}incomplete growth").into_bytes());
        let Bounded::Ready((bytes, starts)) =
            read_bounded_tail(&mut grown, initial_len, 1_024, &mut Budget::new())
                .expect("bounded growth-race read")
        else {
            panic!("growth-race read was truncated");
        };
        assert_eq!(bytes.len(), line.len());
        assert_eq!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .len(),
            1
        );

        let mut truncated = std::io::Cursor::new(Vec::new());
        let Bounded::Ready((bytes, starts)) =
            read_bounded_tail(&mut truncated, initial_len, 1_024, &mut Budget::new())
                .expect("bounded truncation-race read")
        else {
            panic!("truncation-race read was truncated");
        };
        assert!(bytes.is_empty());
        assert!(
            super::codex::parse(&bytes, starts, 1_788_858_600)
                .unwrap()
                .rows
                .is_empty()
        );
    }

    #[test]
    fn every_invocation_budget_limit_has_an_explicit_truncated_row() {
        let mut budget = Budget {
            files_left: 0,
            bytes_left: 1,
            started: std::time::Instant::now(),
            max_elapsed: std::time::Duration::from_secs(1),
        };
        assert!(!budget.claim_file());
        budget.files_left = 1;
        assert!(!budget.reserve_bytes(2));
        budget.bytes_left = 2;
        budget.max_elapsed = std::time::Duration::ZERO;
        assert!(!budget.claim_file());
        assert_eq!(
            rows_or_placeholder(ReadRows::Truncated).rows[0].status,
            Status::Truncated
        );
    }

    #[test]
    fn quota_classification_hysteresis_has_exact_boundaries() {
        let mut state = None;
        let mut seen = Vec::new();
        for used in [79.0, 80.0, 95.0, 94.0, 89.0, 74.0] {
            let next = classify(used, state);
            seen.push(next);
            state = Some(next);
        }
        assert_eq!(
            seen,
            [
                QuotaLevel::Headroom,
                QuotaLevel::Low,
                QuotaLevel::Critical,
                QuotaLevel::Critical,
                QuotaLevel::Low,
                QuotaLevel::Headroom,
            ]
        );
    }

    #[test]
    fn a_level_is_only_ever_decided_from_the_reading_it_belongs_to() {
        let group = declared_group(Some(1), "95.0");
        let row = group.rows[0].clone();
        let reading =
            super::Reading::of(group.policy.clone(), row.clone()).expect("a stamped percentage");

        // One declared reset spreads 95% over two windows, so the level this
        // reading carries is the level of 47.5% — never of the raw number.
        let classified = super::Classified::first(reading);
        assert_eq!(classified.level(), QuotaLevel::Headroom);
        assert_eq!(classified.judged().to_bits(), 47.5_f64.to_bits());
        assert_eq!(classified.observed_at(), row.observed_at.expect("stamped"));

        // Withdrawing the declaration re-judges the SAME observation, and the
        // level that comes back is the one the held reading decided.
        let withdrawn = super::Reading::of(Policy::for_tests(None, Account::default()), row)
            .expect("a stamped percentage");
        let mut held = classified;
        assert_eq!(held.adopt(&withdrawn), super::Adopted::Policy);
        assert_eq!(held.level(), QuotaLevel::Critical);
        assert_eq!(held.judged().to_bits(), 95.0_f64.to_bits());
    }

    #[test]
    fn adopting_a_newer_observation_keeps_the_hysteresis_anchor() {
        // The threshold's hysteresis is only as good as the anchor it is given,
        // and `Classified::adopt` is the one place that anchor is passed now.
        // Strictly increasing stamps, so every step is a raw observation the
        // clock accepts and the level is the only thing under test.
        let group = declared_group(None, "95.0");
        let reading = |used: &str, at: i64| {
            let mut row = group.rows[0].clone();
            row.used_percent = Some(used.to_owned());
            row.observed_at = Some(at);
            super::Reading::of(group.policy.clone(), row).expect("a stamped percentage")
        };

        let mut held = super::Classified::first(reading("95.0", 9_900));
        assert_eq!(held.level(), QuotaLevel::Critical);
        for (used, at, level) in [
            ("94.0", 9_901, QuotaLevel::Critical),
            ("89.0", 9_902, QuotaLevel::Low),
            ("74.0", 9_903, QuotaLevel::Headroom),
        ] {
            assert_eq!(
                held.adopt(&reading(used, at)),
                super::Adopted::Observation,
                "{used} at {at} is a newer raw observation"
            );
            assert_eq!(
                held.level(),
                level,
                "{used} holds the state it was anchored on"
            );
        }
    }

    const AGY_REPORT: &str = r#"{"status":"SUCCESS","num_turns":0,"command":{"name":"usage",
      "data":{"groups":[
        {"name":"Gemini Models","buckets":[
          {"id":"gemini-5h","window":"5h","remaining_fraction":0.75,"reset_time":"2099-01-01T00:00:00Z"},
          {"id":"gemini-weekly","window":"weekly","remaining_fraction":1,"reset_time":"2099-01-02T00:00:00Z"}]},
        {"name":"Claude and GPT models","buckets":[
          {"id":"3p-5h","window":"5h","remaining_fraction":0.5,"reset_time":"2099-01-03T00:00:00Z"},
          {"id":"3p-weekly","window":"weekly","remaining_fraction":0,"reset_time":"2099-01-04T00:00:00Z"}]}]}}}"#;
    const AGY_NOW: i64 = 1_788_858_600;

    /// A scratch directory that removes itself, with a config naming `profiles` in it.
    struct Scratch(std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn agy_dialog(tag: &str, profiles: &str) -> (Scratch, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("ae-agy-dialog-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch home");
        let config = root.join("config");
        std::fs::write(&config, format!("[profiles]\n{profiles}")).expect("identity config");
        (Scratch(root), config)
    }

    /// A cache whose leg answers `answer` and counts its calls.
    fn counting(
        answer: AgyRun,
    ) -> (
        super::AgyCache,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = std::sync::Arc::clone(&calls);
        let cache = super::AgyCache::new(move |_| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            answer.clone()
        });
        (cache, calls)
    }

    fn dialog(
        config: &Path,
        home: Option<&Path>,
        cache: &mut super::AgyCache,
        now: i64,
    ) -> Vec<super::view::Scope> {
        let inputs = super::Inputs {
            home,
            global: Some(config),
            local: None,
            sessions: None,
            now,
        };
        super::view_cached(&inputs, cache)
    }

    fn agy_scopes(scopes: &[super::view::Scope]) -> Vec<&super::view::Scope> {
        scopes
            .iter()
            .filter(|scope| scope.title.starts_with("agy"))
            .collect()
    }

    #[test]
    fn the_dialog_shows_agys_four_windows_from_one_leg_call() {
        let (root, config) = agy_dialog("rows", "a = agy\nb = agy -m\n");
        let (mut cache, calls) = counting(AgyRun::Done {
            text: AGY_REPORT.to_owned(),
        });
        let scopes = dialog(&config, Some(&root.0), &mut cache, AGY_NOW);
        let [scope] = agy_scopes(&scopes)[..] else {
            panic!("two agy profiles on one login are one scope: {scopes:?}");
        };
        let raw: Vec<_> = scope
            .windows
            .iter()
            .filter_map(|w| w.raw.as_deref())
            .collect();
        assert_eq!(raw, ["50%", "100%", "25%", "0%"], "{scope:?}");
        let seen = scope
            .windows
            .iter()
            .all(|w| w.seen.as_deref() == Some("0m"));
        assert!(scope.only.is_none() && seen, "{scope:?}");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_leg_with_no_rows_is_named_with_its_reason_and_age() {
        for (answer, reason) in [
            (AgyRun::TimedOut, "8 s deadline"),
            (AgyRun::TooOld, "1.1.11+"),
            (AgyRun::Failed, "no usable answer"),
            (AgyRun::Absent, "not found"),
        ] {
            let (root, config) = agy_dialog("named", "a = agy\n");
            let (mut cache, _) = counting(answer.clone());
            let scopes = dialog(&config, Some(&root.0), &mut cache, AGY_NOW);
            let [scope] = agy_scopes(&scopes)[..] else {
                panic!("{answer:?}: {scopes:?}");
            };
            let only = scope.only.as_deref().unwrap_or_default();
            assert!(
                scope.windows.is_empty() && only.contains(reason),
                "{answer:?}: {only}"
            );
            assert!(only.contains("asked 0m ago"), "{answer:?}: {only}");
            assert!(!only.contains("file"), "{answer:?}: {only}");
        }
    }

    #[test]
    fn the_held_answer_is_reused_until_its_ttl_or_another_home() {
        let (root, config) = agy_dialog("ttl", "a = agy\n");
        let (here, other) = (root.0.as_path(), root.0.join("other"));
        std::fs::create_dir_all(&other).expect("second home");
        let (mut cache, calls) = counting(AgyRun::Done {
            text: AGY_REPORT.to_owned(),
        });
        let asked = || calls.load(std::sync::atomic::Ordering::SeqCst);
        let ttl = super::AGY_TTL_SECS;
        dialog(&config, Some(here), &mut cache, AGY_NOW);
        let held = dialog(&config, Some(here), &mut cache, AGY_NOW + 190);
        let seen = agy_scopes(&held)[0].windows[0].seen.clone();
        assert_eq!((asked(), seen.as_deref()), (1, Some("3m")));
        dialog(&config, Some(here), &mut cache, AGY_NOW + ttl - 1);
        assert_eq!(asked(), 1, "one second inside the ttl");
        dialog(&config, Some(here), &mut cache, AGY_NOW + ttl);
        assert_eq!(asked(), 2, "the ttl itself asks again");
        dialog(&config, Some(&other), &mut cache, AGY_NOW + ttl);
        assert_eq!(asked(), 3, "another home is another login");
        dialog(&config, Some(&other), &mut cache, AGY_NOW);
        assert_eq!(asked(), 4, "an answer from the future is never held");
    }

    #[test]
    fn no_leg_runs_without_an_agy_scope_on_the_callers_home() {
        for (tag, profiles, home, shown) in [
            ("off", "a = HOME=/x agy\n", true, "run agy -p"),
            ("claude", "a = claude\n", true, ""),
            ("nohome", "a = agy\n", false, "run agy -p"),
        ] {
            let (root, config) = agy_dialog(tag, profiles);
            let (mut cache, calls) = counting(AgyRun::Failed);
            let scopes = dialog(
                &config,
                home.then_some(root.0.as_path()),
                &mut cache,
                AGY_NOW,
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "{tag}");
            let words = agy_scopes(&scopes).first().and_then(|s| s.only.clone());
            assert!(
                words.unwrap_or_default().contains(shown),
                "{tag}: {scopes:?}"
            );
        }
    }

    #[test]
    fn an_unreadable_config_is_the_one_unavailable_scope() {
        let (root, config) = agy_dialog("broken", "a = agy\n");
        std::fs::write(&config, "[agents]\nx = y\n").expect("surviving agents section");
        let (mut cache, calls) = counting(AgyRun::Failed);
        let scopes = dialog(&config, Some(&root.0), &mut cache, AGY_NOW);
        assert_eq!(scopes, super::view::unavailable());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    const AGY_EMPTY: &str =
        r#"{"status":"SUCCESS","num_turns":0,"command":{"name":"usage","data":{"groups":[]}}}"#;

    fn held_view(
        config: &Path,
        home: Option<&Path>,
        cache: &super::AgyCache,
        flying: bool,
    ) -> (Vec<super::view::Scope>, Option<std::path::PathBuf>) {
        let inputs = super::Inputs {
            home,
            global: Some(config),
            local: None,
            sessions: None,
            now: AGY_NOW,
        };
        super::view_held(&inputs, cache, flying)
    }

    fn done(text: &str) -> AgyRun {
        AgyRun::Done {
            text: text.to_owned(),
        }
    }

    /// I1: the app's held view draws exactly what the synchronous dialog draws for the
    /// same attempt, over every answer, no home and an unreadable config, and runs no leg.
    #[test]
    fn the_held_view_draws_what_the_dialog_draws_for_the_same_attempt() {
        for (tag, answer, home, broken) in [
            ("rows", done(AGY_REPORT), true, false),
            ("empty", done(AGY_EMPTY), true, false),
            ("failed", AgyRun::Failed, true, false),
            ("late", AgyRun::TimedOut, true, false),
            ("old", AgyRun::TooOld, true, false),
            ("absent", AgyRun::Absent, true, false),
            ("nohome", done(AGY_REPORT), false, false),
            ("broken", done(AGY_REPORT), true, true),
        ] {
            let (root, config) = agy_dialog(&format!("same-{tag}"), "a = agy\nc = claude\n");
            if broken {
                std::fs::write(&config, "[agents]\nx = y\n").expect("surviving agents section");
            }
            let home = home.then_some(root.0.as_path());
            let (mut asked, _) = counting(answer);
            let dialog = dialog(&config, home, &mut asked, AGY_NOW);
            let (mut cache, calls) = counting(AgyRun::Failed);
            if let Some(h) = &asked.held {
                cache.settle(&h.home, h.asked_at, h.done_at, h.answer.clone());
            }
            let (view, due) = held_view(&config, home, &cache, false);
            let calls = calls.load(std::sync::atomic::Ordering::SeqCst);
            assert_eq!((view, due, calls), (dialog, None, 0), "{tag}");
        }
    }

    /// I2: a refresh with no window keeps the last answer that had one, at its own age,
    /// under the refresh's own words; an expired answer shows at its age while one is due.
    #[test]
    fn a_kept_answer_keeps_its_own_age_under_the_refreshs_words() {
        let failed = "last refresh failed — agy /quota gave no usable answer";
        let late = "last refresh failed — agy /quota passed its 8 s deadline";
        for (answer, words) in [
            (AgyRun::Failed, failed),
            (done("not json"), failed),
            (AgyRun::TimedOut, late),
            (AgyRun::Absent, "agy not found on PATH"),
            (AgyRun::TooOld, "agy 1.1.11+ required for /quota"),
            (done(AGY_EMPTY), "agy /quota reported no windows"),
        ] {
            let (root, config) = agy_dialog("kept", "a = agy\n");
            let (mut cache, calls) = counting(AgyRun::Failed);
            cache.settle(&root.0, AGY_NOW - 600, AGY_NOW - 600, done(AGY_REPORT));
            cache.settle(&root.0, AGY_NOW - 120, AGY_NOW - 118, answer.clone());
            let (view, due) = held_view(&config, Some(&root.0), &cache, false);
            let scope = agy_scopes(&view)[0];
            assert_eq!(scope.only.as_deref(), Some(words), "{answer:?}");
            let seen: Vec<_> = scope.windows.iter().map(|w| w.seen.as_deref()).collect();
            assert_eq!(seen, [Some("10m"); 4], "{answer:?}: its own age");
            let calls = calls.load(std::sync::atomic::Ordering::SeqCst);
            assert_eq!((due, calls), (None, 0), "{answer:?}: held for its TTL");
        }
        let (root, config) = agy_dialog("expired", "a = agy\n");
        let (mut cache, _) = counting(AgyRun::Failed);
        cache.settle(&root.0, AGY_NOW - 600, AGY_NOW - 599, done(AGY_REPORT));
        let (view, due) = held_view(&config, Some(&root.0), &cache, false);
        assert_eq!(due.as_deref(), Some(root.0.as_path()), "expired: due");
        let scope = agy_scopes(&view)[0];
        let seen: Vec<_> = scope.windows.iter().map(|w| w.seen.as_deref()).collect();
        assert!(scope.only.is_none(), "{scope:?}");
        assert_eq!(seen, [Some("9m"); 4], "shown at its real age while due");
        let (flying, due) = held_view(&config, Some(&root.0), &cache, true);
        assert_eq!((flying, due), (view, None), "one ask out: the same drawing");
    }

    /// Every paid outcome keeps its own words AND the reason the next ask may be paid again.
    #[test]
    fn a_paid_muse_outcome_keeps_its_words_beside_an_unrecorded_gap() {
        use super::{MUSE_FAILED, MUSE_NO_ALLOWANCE, MUSE_TIMEOUT, muse_rows};
        use crate::muse_usage::Gap;
        use crate::quota::muse::{Cached, Outcome, extract};
        let record = |outcome, allowance| Cached {
            home: "/synthetic/muse-home".to_owned(),
            attempt_at: 1_792_000_000,
            outcome,
            allowance,
            resets_at: None,
        };
        let empty = extract(br#"{"record_type":"status","payload_type":"run.usage","payload":{}}"#)
            .expect("a record with no allowance");
        let failed = |class: &str| format!("{MUSE_FAILED} ({class})");
        let table = [
            (
                record(Outcome::TimedOut, None),
                Status::Truncated,
                MUSE_TIMEOUT.to_owned(),
            ),
            (
                record(Outcome::Failed, None),
                Status::ReadError,
                MUSE_FAILED.to_owned(),
            ),
            (
                record(Outcome::TooLarge, None),
                Status::ReadError,
                failed("output over 256 KiB"),
            ),
            (
                record(Outcome::Exited(3), None),
                Status::ReadError,
                failed("exited 3"),
            ),
            (
                record(Outcome::NotUtf8, None),
                Status::ReadError,
                failed("output not UTF-8"),
            ),
            (
                record(Outcome::Unparsed, None),
                Status::ReadError,
                failed("output not parsed"),
            ),
            (
                record(Outcome::Done, Some(empty)),
                Status::Unknown,
                MUSE_NO_ALLOWANCE.to_owned(),
            ),
        ];
        for (held, status, words) in table {
            let (rows, hint) = muse_rows(Some(&held), Some(Gap::Unrecorded), 1_792_000_100);
            assert_eq!(rows.len(), 1, "{held:?}");
            assert_eq!(rows[0].status, status, "{held:?}");
            let hint = hint.expect("words beside the placeholder");
            assert!(hint.contains(&words), "{hint}");
            assert!(hint.contains(Gap::Unrecorded.words()), "{hint}");
            let (_, plain) = muse_rows(Some(&held), None, 1_792_000_100);
            assert_eq!(
                plain.as_deref(),
                Some(words.as_str()),
                "no gap, no extra words"
            );
        }
    }

    #[test]
    fn measured_limit_rows_use_attempt_age_and_never_infer_a_window_duration() {
        use super::muse_rows;
        use crate::muse_usage::Gap;
        use crate::quota::muse;

        const ATTEMPT: i64 = 1_893_452_400; // Synthetic 2029-12-31T23:00:00Z
        const RESET: i64 = 1_893_456_000; // Synthetic 2030-01-01T00:00:00Z
        let input = format!(
            r#"{{"v":1,"home":"/synthetic/muse-home","attempt_at":{ATTEMPT},"outcome":"limit","resets_at":{RESET}}}"#
        );
        let held = muse::parse_cache(input.as_bytes()).expect("normalized limit record");
        for (now, status) in [
            (ATTEMPT, Status::Fresh),
            (ATTEMPT + 900, Status::Fresh),
            (ATTEMPT + 901, Status::Stale),
            (ATTEMPT - 300, Status::Unknown),
            (RESET, Status::Unknown),
        ] {
            let (rows, words) = muse_rows(Some(&held), None, now);
            assert_eq!(
                rows,
                vec![Row {
                    bucket: "limit".to_owned(),
                    qualifier: None,
                    window_minutes: None,
                    used_percent: Some("100".to_owned()),
                    resets_at: Some(RESET),
                    observed_at: Some(ATTEMPT),
                    status,
                }],
                "at {now}"
            );
            let words = words.expect("limit explanation");
            assert!(words.contains("muse usage limit reached"), "{words}");
            assert!(words.contains("limit-derived"), "{words}");
        }
        let (plain, _) = muse_rows(Some(&held), None, ATTEMPT);
        let (unrecorded, words) = muse_rows(Some(&held), Some(Gap::Unrecorded), ATTEMPT);
        assert_eq!(
            unrecorded, plain,
            "publication failure must retain the paid answer"
        );
        let words = words.expect("limit and publication explanation");
        assert!(words.contains("muse usage limit reached"), "{words}");
        assert!(words.contains("limit-derived"), "{words}");
        assert!(words.contains(Gap::Unrecorded.words()), "{words}");
    }

    fn plan_max_20x(fetched_at: Option<i64>) -> super::claude::Plan {
        super::claude::Plan {
            organization_type: Some("claude_max".to_owned()),
            organization_rate_limit_tier: Some("default_claude_max_20x".to_owned()),
            user_rate_limit_tier: None,
            seat_tier: None,
            fetched_at,
        }
    }

    /// The plan cell is the ONE difference a reported plan may make: the
    /// labelled scope cell, and the age `profileFetchedAt` supplies. A stamp in
    /// the future or none at all says `reported` and no age, because ae never
    /// quotes a moment it cannot place.
    #[test]
    fn a_plan_cell_names_the_scope_and_nothing_else() {
        let mut group = unusable_claude_group(10_000, Some(9_000));
        assert_eq!(
            super::scope_label(&group, None, 10_000),
            "claude · /tmp/mic/.claude-mic"
        );

        group.plan = Some(plan_max_20x(Some(9_000)));
        assert_eq!(
            super::scope_label(&group, None, 10_000),
            "claude · /tmp/mic/.claude-mic · plan: claude max 20x (reported 16m ago)"
        );

        group.plan = Some(plan_max_20x(Some(20_000)));
        assert_eq!(
            super::scope_label(&group, None, 10_000),
            "claude · /tmp/mic/.claude-mic · plan: claude max 20x (reported)"
        );

        group.plan = Some(plan_max_20x(None));
        assert_eq!(
            super::scope_label(&group, None, 10_000),
            "claude · /tmp/mic/.claude-mic · plan: claude max 20x (reported)"
        );
    }

    /// Whatever the wrapping does, the plan contributes its own tokens and not
    /// one cell more: every other token in the table is the plan-less table's.
    #[test]
    fn a_reported_plan_adds_one_scope_label_and_no_verdict() {
        let mut group = unusable_claude_group(10_000, Some(9_000));
        let without = render_at(std::slice::from_ref(&group), None, 10_000);
        group.plan = Some(plan_max_20x(Some(9_000)));
        let with = render_at(std::slice::from_ref(&group), None, 10_000);

        assert_eq!(with.matches("plan:").count(), 1, "{with}");

        let tokens = |text: &str| {
            text.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<String>>()
        };
        let mut got = tokens(&with);
        for word in "· plan: claude max 20x (reported 16m ago)".split_whitespace() {
            let at = got
                .iter()
                .position(|candidate| candidate.as_str() == word)
                .expect("the plan stands whole in the scope cell");
            got.remove(at);
        }
        got.sort_unstable();
        let mut plain = tokens(&without);
        plain.sort_unstable();
        assert_eq!(got, plain, "a plan changes the scope cell and nothing else");
    }

    /// The dialog's head row is the settings surface: the plan follows the
    /// account words it already had, in every branch, and stands alone when the
    /// scope reports no account fact at all.
    #[test]
    fn the_settings_head_appends_the_plan_to_whatever_account_words_it_had() {
        let plan_words = "plan: claude max 20x (reported 16m ago)";
        let mut group = unusable_claude_group(10_000, Some(9_000));
        group.plan = Some(plan_max_20x(Some(9_000)));

        group.policy = Policy::new(
            None,
            Account {
                credits: Credits::Available("12.40".to_owned()),
                credits_observed_at: Some(9_900),
                ..Account::default()
            },
        );
        let scope = viewed(vec![group.clone()]).remove(0);
        assert_eq!(
            scope.account.as_deref(),
            Some(format!("credits 12.40 · {plan_words}").as_str())
        );

        group.policy = Policy::new(
            None,
            Account {
                spend_control_reached: Some(true),
                spend_observed_at: Some(9_900),
                ..Account::default()
            },
        );
        let scope = viewed(vec![group.clone()]).remove(0);
        assert_eq!(
            scope.account.as_deref(),
            Some(format!("spend cap reached · {plan_words}").as_str())
        );

        group.policy = Policy::new(None, Account::default());
        let scope = viewed(vec![group.clone()]).remove(0);
        assert_eq!(scope.account.as_deref(), Some(plan_words));

        group.plan = None;
        let scope = viewed(vec![group]).remove(0);
        assert_eq!(
            scope.account, None,
            "no plan and no account fact is silence"
        );
    }
}
