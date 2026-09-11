//! Vendor quota snapshots parsed from local client-owned caches.
//!
//! Parsing stays separate from discovery and rendering: the client files are
//! hostile persisted state, while this module is a pure bytes-to-rows boundary.

pub mod claude;
pub mod codex;

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::json::Value;
use crate::time::Timestamp;
use crate::tool::{QuotaSource, ToolKind};

const CLAUDE_MAX_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_TAIL_BYTES: u64 = 256 * 1024;
const SESSION_META_MAX_BYTES: u64 = 1024 * 1024;
const QUOTA_MAX_FILES: usize = 4_096;
const QUOTA_MAX_BYTES: u64 = 16 * 1024 * 1024;
const QUOTA_MAX_ELAPSED: Duration = Duration::from_secs(2);
const CODEX_DISPLAY_ROLLOUTS: usize = 3;
/// Per-column display caps. Their sum plus two spaces between each pair of
/// columns is [`TABLE_MAX_LINE`], the table's documented width ceiling: a new
/// column is paid for here, in width, and the ceiling says what it cost.
const TABLE_MAX_WIDTHS: [usize; COLUMNS] = [40, 35, 22, 6, 5, 9, 9, 9, 9, 20];

/// Columns in the operator table.
const COLUMNS: usize = 10;

/// The widest line the table can render, cells and separators together. The
/// caps above produce it; `docs/reference/commands.md` publishes it.
#[cfg(test)]
const TABLE_MAX_LINE: usize = 182;

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
    pub(crate) const fn new(manual_resets: Option<u8>, account: Account) -> Self {
        Self {
            manual_resets,
            account,
        }
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
        let newer = incoming.observed_at > self.observed_at;
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
    hint: Option<String>,
    manual_resets: Option<u8>,
    notes: Vec<String>,
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

/// A seat identity precise enough to join its persisted conversation to one
/// observed vendor source without falling back to current profile config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordedIdentity {
    pub(crate) tool: ToolKind,
    pub(crate) source: PathBuf,
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
}

impl Advisory {
    pub(crate) fn current_at(&self, now: i64) -> bool {
        matches!(
            freshness(self.observed_at, self.resets_at, now),
            Status::Fresh | Status::Stale
        ) && self
            .observed_at
            .is_some_and(|at| now.saturating_sub(at) <= 60 * 60)
    }

    pub(crate) fn render(&self, meta_dir: &Path, now: i64) -> String {
        let observed = self.observed_at.map_or_else(
            || "unknown".to_owned(),
            |at| age_label(now.saturating_sub(at)),
        );
        let reset = self.resets_at.map_or_else(
            || "unknown".to_owned(),
            |at| span_label(at.saturating_sub(now)),
        );
        let advice = if self.spend_capped {
            " — spend cap reached: another client or more credits, not a window reset"
        } else if self.recovered {
            " — back to headroom"
        } else {
            " — prefer another client for new spawns"
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
}

impl Observation {
    /// Render one advisory line from the same sanitized, width-bounded cells
    /// as the operator table. The helper path is ae-owned rather than vendor
    /// input and remains complete so the recipient can invoke it verbatim.
    #[cfg(test)]
    pub(crate) fn advisory_line(
        &self,
        group: &Group,
        reading: &Reading,
        state: &str,
        meta_dir: &Path,
    ) -> String {
        self.advisory_line_at(group, reading, state, meta_dir, self.now)
    }

    pub(crate) fn advisory_line_at(
        &self,
        group: &Group,
        reading: &Reading,
        state: &str,
        meta_dir: &Path,
        now: i64,
    ) -> String {
        self.advisory(group, reading, state).render(meta_dir, now)
    }

    /// Render one advisory from a READING: the group supplies only the labels
    /// of the scope, while every number, the derivation and the age come from
    /// the observation that decided the level.
    pub(crate) fn advisory(&self, group: &Group, reading: &Reading, state: &str) -> Advisory {
        let row = &reading.row;
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
        let recovered = state == "back to headroom";
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
            state: if recovered { "headroom" } else { state }.to_owned(),
            recovered,
            derivation,
            spend_capped: reading.policy.spend_capped(),
        }
    }
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
        QuotaSource::Unsupported => return None,
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
}

pub(crate) enum Bounded<T> {
    Ready(T),
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FleetRollout {
    owner: String,
    profile: String,
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
    pub(crate) fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    pub(crate) fn len(&self) -> u64 {
        self.metadata.len()
    }
}

enum RolloutSource {
    File(RolloutFile),
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
    let cfg = crate::config::read_identity(inputs.global, inputs.local)?;
    let mut scopes = configured_scopes(&cfg, inputs.home);
    let mut groups = Vec::new();
    let mut rendered = Vec::new();
    let mut budget = Budget::new();
    let fleet = fleet_rollouts(inputs.sessions, &mut budget);
    add_recorded_codex_scopes(&mut scopes, &fleet);
    for scope in &scopes {
        let quota = scope.tool.adapter().quota;
        match quota.source {
            QuotaSource::ClaudeCache => {
                let observed = rows_or_placeholder(read_claude(scope, inputs.now, &mut budget));
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
                };
                rendered.push(group.clone());
                groups.push(group);
            }
            QuotaSource::CodexRollouts => {
                let observed = codex_groups(scope, &fleet, inputs.now, &mut budget);
                groups.extend(observed.all);
                rendered.extend(observed.rendered);
            }
            QuotaSource::Unsupported => {
                let group = Group {
                    profiles: scope.profiles.clone(),
                    tool: scope.tool,
                    home: scope.home.clone(),
                    source: scope.source_key.clone(),
                    clients: scope.clients.clone(),
                    rollout: None,
                    owner: None,
                    rows: vec![placeholder(Status::Unsupported)],
                    hint: scope
                        .hint
                        .clone()
                        .or_else(|| quota.unsupported_hint.map(str::to_owned)),
                    summary: None,
                    policy: Policy::new(scope.manual_resets, Account::default()),
                    notes: scope.notes.clone(),
                };
                rendered.push(group.clone());
                groups.push(group);
            }
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
    Ok(Observation {
        groups,
        rendered,
        home,
        now: inputs.now,
    })
}

/// Read configured client scopes and print the local quota table.
///
/// The operation is deliberately observational: it opens only client cache
/// files, writes only to the supplied streams, and never starts a process.
///
/// # Errors
///
/// Returns an I/O error only when the supplied output stream cannot be written.
pub fn run(inputs: &Inputs<'_>, out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    let observation = match observe(inputs) {
        Ok(observation) => observation,
        Err(error) => {
            writeln!(err, "{error}")?;
            return Ok(1);
        }
    };
    write!(
        out,
        "{}",
        render_at(
            &observation.rendered,
            observation.home.as_deref().or(inputs.home),
            observation.now
        )
    )?;
    Ok(0)
}

fn configured_scopes(cfg: &crate::config::IdentityConfig, home: Option<&Path>) -> Vec<Scope> {
    let mut scopes: Vec<Scope> = Vec::new();
    for (profile, raw) in &cfg.profiles {
        let resolved = match cfg.command(profile, home) {
            Ok(Some(resolved)) => resolved,
            Ok(None) => {
                scopes.push(unresolved_scope(cfg, profile, raw, None));
                continue;
            }
            Err(error) => {
                scopes.push(unresolved_scope(cfg, profile, raw, Some(&error)));
                continue;
            }
        };
        let tool = resolved_tool(resolved.as_str());
        let declaration = declaration_for(cfg, resolved.client_label());
        let client = resolved
            .client_label()
            .and_then(|label| displayed_client(cfg, label, tool));
        if let Some(variable) = word_expansion_dependency(&resolved, home) {
            scopes.push(unknown_scope(
                profile,
                tool,
                client,
                Some(format!("depends on pane variable {variable}")),
                &declaration,
            ));
            continue;
        }
        let unknown_variable = std::cell::RefCell::new(None);
        let account_variable = tool.adapter().config_home_env;
        let resolution = crate::launch_cmd::config_home_resolution(&resolved, tool, &|name| {
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
        });
        if let Some(variable) = unknown_variable.into_inner() {
            scopes.push(unknown_scope(
                profile,
                tool,
                client,
                Some(format!("depends on pane variable {variable}")),
                &declaration,
            ));
            continue;
        }
        let paths = match resolved_scope_paths(tool, &resolution, home) {
            Ok(paths) => paths,
            Err(error) => {
                scopes.push(unknown_scope(
                    profile,
                    tool,
                    client,
                    Some(error),
                    &declaration,
                ));
                continue;
            }
        };
        let ScopePaths {
            home: config_home,
            source,
            source_key,
            hint,
        } = paths;
        if let Some(scope) = scopes.iter_mut().find(|scope| {
            scope.tool == tool && scope.source_key == source_key && scope.hint == hint
        }) {
            scope.profiles.push(profile.clone());
            scope.configured_profiles.push(profile.clone());
            if let Some(client) = client
                && !scope.clients.contains(&client)
            {
                scope.clients.push(client);
            }
            merge_declaration(scope, &declaration);
        } else {
            scopes.push(Scope {
                tool,
                home: config_home,
                source,
                source_key,
                profiles: vec![profile.clone()],
                configured_profiles: vec![profile.clone()],
                clients: client.into_iter().collect(),
                hint,
                manual_resets: declaration.manual_resets,
                notes: declaration.note.clone().into_iter().collect(),
            });
        }
    }
    scopes
}

/// The scope a profile gets when its own command never resolved to a client.
fn unresolved_scope(
    cfg: &crate::config::IdentityConfig,
    profile: &str,
    raw: &str,
    error: Option<&crate::config::ConfigError>,
) -> Scope {
    let Some(error) = error else {
        return unknown_scope(
            profile,
            resolved_tool(""),
            None,
            None,
            &Declaration::default(),
        );
    };
    let label = config_error_client(error);
    let tool = label
        .and_then(|label| cfg.client(label))
        .map_or_else(|| resolved_tool(raw), |client| client.tool);
    unknown_scope(
        profile,
        tool,
        label.and_then(|label| displayed_client(cfg, label, tool)),
        Some(error.to_string()),
        &declaration_for(cfg, label),
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
    if tool.adapter().quota.source == QuotaSource::Unsupported {
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
        return Ok(ScopePaths {
            home: fallback,
            source: None,
            source_key: None,
            hint,
        });
    }
    let source = match tool.adapter().quota.source {
        QuotaSource::ClaudeCache if resolution.explicit => home
            .as_ref()
            .map(|config_home| config_home.join(".claude.json")),
        QuotaSource::ClaudeCache => match &resolution.base {
            crate::launch_cmd::Resolved::Path(_) => {
                match crate::run::canonical_config_home(&resolution.base)? {
                    crate::launch_cmd::Resolved::Path(base) => Some(base.join(".claude.json")),
                    crate::launch_cmd::Resolved::Absent
                    | crate::launch_cmd::Resolved::Unknown(_) => None,
                }
            }
            crate::launch_cmd::Resolved::Absent => None,
            crate::launch_cmd::Resolved::Unknown(reason) => {
                if hint.is_none() {
                    hint = Some(reason.clone());
                }
                None
            }
        },
        QuotaSource::CodexRollouts => home
            .as_ref()
            .map(|config_home| config_home.join("sessions")),
        QuotaSource::Unsupported => None,
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

fn unknown_scope(
    profile: &str,
    tool: ToolKind,
    client: Option<String>,
    hint: Option<String>,
    declaration: &Declaration,
) -> Scope {
    Scope {
        tool,
        home: None,
        source: None,
        source_key: None,
        profiles: vec![profile.to_owned()],
        configured_profiles: vec![profile.to_owned()],
        clients: client.into_iter().collect(),
        hint,
        manual_resets: declaration.manual_resets,
        notes: declaration.note.clone().into_iter().collect(),
    }
}

/// Read one `[clients]` row's declared extra headroom, if the profile names one.
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

fn read_claude(scope: &Scope, now: i64, budget: &mut Budget) -> ReadRows {
    let Some(path) = scope.source.as_deref() else {
        return ReadRows::Missing;
    };
    let bytes = match bounded_whole_file(path, CLAUDE_MAX_BYTES, budget) {
        Ok(Bounded::Ready(Some(bytes))) => bytes,
        Ok(Bounded::Ready(None)) => return ReadRows::Missing,
        Ok(Bounded::Truncated) => return ReadRows::Truncated,
        Err(_) => return ReadRows::Failed,
    };
    match claude::parse(&bytes, now) {
        Ok(Some(snapshot)) if !snapshot.rows.is_empty() => ReadRows::Rows(Observed {
            rows: snapshot.rows,
            account: Account::default(),
        }),
        Ok(_) => ReadRows::Missing,
        Err(_) => ReadRows::Failed,
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
            rollouts.push(FleetRollout {
                owner: format!("{session}:{}", seat.name),
                profile: profile.clone(),
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
                        hint: None,
                        manual_resets: None,
                        notes: Vec::new(),
                    });
                }
            }
            RolloutLocation::Unknown(reason) => {
                if let Some(scope) = scopes.iter_mut().find(|scope| {
                    scope.tool == rollout.tool
                        && scope.source.is_none()
                        && scope.hint.as_ref() == Some(reason)
                }) {
                    if !scope.profiles.contains(&rollout.profile) {
                        scope.profiles.push(rollout.profile.clone());
                    }
                } else {
                    scopes.push(Scope {
                        tool: rollout.tool,
                        home: None,
                        source: None,
                        source_key: None,
                        profiles: vec![rollout.profile.clone()],
                        configured_profiles: Vec::new(),
                        clients: Vec::new(),
                        hint: Some(reason.clone()),
                        manual_resets: None,
                        notes: Vec::new(),
                    });
                }
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
    CodexGroups { all, rendered }
}

fn scope_rollouts<'a>(scope: &Scope, fleet: &'a FleetRollouts) -> Vec<&'a FleetRollout> {
    fleet
        .rollouts
        .iter()
        .filter(|rollout| match &rollout.location {
            RolloutLocation::Configured => scope.configured_profiles.contains(&rollout.profile),
            RolloutLocation::Recorded { source_key, .. } => {
                scope.source_key.as_ref() == Some(source_key)
            }
            RolloutLocation::Unknown(reason) => {
                scope.source.is_none()
                    && scope.hint.as_ref() == Some(reason)
                    && scope.profiles.contains(&rollout.profile)
            }
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
                Ok(Bounded::Ready(Some(file))) => RolloutSource::File(file),
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
            profiles: Vec::new(),
            tool: scope.tool,
            home: scope.home.clone(),
            source: scope.source_key.clone(),
            clients: scope.clients.clone(),
            rollout: None,
            owner: None,
            rows: Vec::new(),
            hint: None,
            policy: Policy::default(),
            notes: Vec::new(),
            summary: Some(RolloutSummary {
                hidden,
                unreadable,
                oldest_observed,
                not_read: truncated && shown == 0,
                status: summary_status,
            }),
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

fn scope_label(group: &Group, home: Option<&Path>) -> String {
    let mut label = format!("{} · {}", group.tool.as_str(), scope_identity(group, home));
    if group.tool.adapter().quota.source == QuotaSource::CodexRollouts {
        label.push_str(" · unidentified");
        if let Some(owner) = group.owner.as_deref() {
            let _ = write!(label, " ({owner})");
        }
    }
    label
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

fn render_at(groups: &[Group], home: Option<&Path>, now: i64) -> String {
    const HEADER: [&str; COLUMNS] = [
        "PROFILES",
        "SCOPE",
        "BUCKET",
        "WINDOW",
        "USED",
        "EFFECTIVE",
        "CREDITS",
        "RESETS",
        "OBSERVED",
        "STATUS",
    ];
    let mut table = vec![RenderLine::Cells(HEADER.map(str::to_owned))];
    let mut notes: Vec<String> = Vec::new();
    for group in groups {
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
        if let Some(summary) = &group.summary {
            table.push(RenderLine::Summary {
                label: rollout_summary_label(summary, now),
                status: summary
                    .status
                    .map_or_else(String::new, |status| status.as_str().to_owned()),
            });
            continue;
        }
        for (index, row) in group.rows.iter().enumerate() {
            let trustworthy = matches!(row.status, Status::Fresh | Status::Stale);
            let status = group.hint.as_deref().map_or_else(
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
                    scope_label(group, home)
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
                trustworthy
                    .then(|| row.resets_at.map(|reset| reset_label(reset, now)))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
                trustworthy
                    .then(|| row.observed_at.map(|observed| age_label(now - observed)))
                    .flatten()
                    .unwrap_or_else(|| "-".to_owned()),
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

fn profiles_label(profiles: &[String]) -> String {
    let joined = profiles.join(" ");
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

fn age_label(seconds: i64) -> String {
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

#[cfg(test)]
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
        reason = "a door: quota lstat refuses a symlink or non-file before opening one ae-owned Codex rollout"
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
        reason = "a door: opens only the exact rollout named by an ae-recorded harness session id"
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

#[cfg(test)]
mod tests {
    use super::{
        Account, Bounded, Budget, CLAUDE_MAX_BYTES, CODEX_TAIL_BYTES, COLUMNS, Credits,
        Declaration, FRESH_SECS, FUTURE_SKEW_SECS, FleetRollout, FleetRollouts, FleetStatus, Group,
        LocatedRollout, Policy, ReadRows, RenderLine, RolloutLocation, RolloutSource, Row, Scope,
        Status, TABLE_MAX_LINE, TABLE_MAX_WIDTHS, bounded_tail, bounded_whole_file, codex_groups,
        codex_rollout_dirs, configured_scopes, credits_label, derived, effective,
        find_codex_rollout, freshness, merge_declaration, order_located_rollouts, percent_label,
        profiles_label, read_bounded_tail, read_claude, render_at, render_table,
        rows_or_placeholder, sanitize_cell, vendor_timestamp,
    };
    use crate::tool::ToolKind;

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
                id: id.to_owned(),
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            configured_profiles: vec!["codex-profile".to_owned()],
            clients: Vec::new(),
            hint: None,
            manual_resets: None,
            notes: Vec::new(),
        };
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
            id: ids[0].to_owned(),
            tool: ToolKind::Codex,
            location: RolloutLocation::Configured,
        };
        let newer = FleetRollout {
            owner: "newer".to_owned(),
            profile: "p".to_owned(),
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
                id,
                tool: ToolKind::Codex,
                location: RolloutLocation::Configured,
            });
        }
        let scope = Scope {
            tool: ToolKind::Codex,
            home: Some(root.join(".codex")),
            source: Some(root.join(".codex/sessions")),
            source_key: Some(root.join(".codex/sessions")),
            profiles: vec!["codex-profile".to_owned()],
            configured_profiles: vec!["codex-profile".to_owned()],
            clients: Vec::new(),
            hint: None,
            manual_resets: None,
            notes: Vec::new(),
        };
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
                hint: None,
                manual_resets: None,
                notes: Vec::new(),
            };
            match read_claude(&scope, 1_001, &mut Budget::new()) {
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
        }
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
                hint: None,
                manual_resets: first,
                notes: Vec::new(),
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
}
