//! `ae brief` — one plain-text card per session: what it is for, what was
//! decided, who is stuck.
//!
//! The card answers the question `ae list` cannot: a listing names sessions and
//! their attention markers, and leaves the human to open each one to learn WHY.
//! A brief carries the reasons across — the goal, the latest checkpoint per memo
//! topic, each agent's declared state, and the two explicit sources of "somebody
//! is waiting on you": a `waiting-user`/`blocked` declaration, and an `ask` or
//! `review` nobody has answered.
//!
//! Two properties are deliberate:
//!
//! * **Explicit sources only.** Nothing here infers that a human is needed. A
//!   `needs you:` line exists because an agent DECLARED it or because a tracked
//!   request is still open. There is no pane capture and no heuristic, so the
//!   section is either evidence or the words `none recorded`.
//! * **It writes nothing.** No tmux option, no event, no meta. The card is a
//!   read of the session store plus the world `ae list` already builds, which is
//!   why it is safe to run against somebody else's session.
//!
//! ## The two reads, and why they cannot contradict each other
//!
//! A card is built from TWO observations of one session: the world, read during
//! discovery, and the event container this module reads for the detail the world
//! does not carry. They are milliseconds apart and a session can move between
//! them, so the seam is closed rather than assumed away:
//!
//! * The STATE of an agent is always the world's — the same cell `ae list`
//!   prints. Only the age and the reason come from the second read, and only
//!   when the declaration found there still names that state. A newer
//!   declaration is DROPPED, not shown against the old cell.
//! * `attn:` is the world's rollup and `needs you:` is this read's detail, so
//!   the two can differ only in the safe direction: this read sees every open
//!   request, where the rollup counts one only past its staleness threshold.
//! * A session whose record could not be fully read is marked `degraded`, and
//!   its empty sections say `unknown` rather than claiming nothing was recorded.
//!
//! The module splits at the same seam the rest of the crate does: [`Card`] and
//! [`render`] are pure and carry every formatting decision, and [`card_for`] is
//! the one place that reads a session directory.

use std::path::Path;

use crate::attention::Reason;
use crate::digest::{SessionEntry, Status};
use crate::requests::{self, Status as RequestStatus};
use crate::state;
use crate::time::Timestamp;

/// What `ae brief` prints when its argv does not parse.
pub const USAGE: &str = "Usage: ae brief [session] [--all] [--since <duration>]\n\n  \
                         session          the session to card; default is the caller's own,\n                   \
                         and --all when ae cannot name one\n  \
                         --all            every running session, most attention first\n  \
                         --since <dur>    drop topic records older than <dur> (90s, 30m, 2h, 3d)\n";

/// The width a card is written to fit — a terminal half, not a full one, so two
/// cards sit side by side in a split.
///
/// It bounds the FREE TEXT: a goal, a memo record, a reason and a question are
/// clipped to it. An identity is not — a name, a branch or a path is what the
/// reader has to type back, and a truncated one is worse than a long line.
pub const WIDTH: usize = 100;

/// Which sessions a brief covers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Target {
    /// No session was named: the caller's own, and `--all` when ae cannot name
    /// one.
    #[default]
    Caller,
    /// `--all` — every running session.
    All,
    /// A named session, whatever its status.
    Named(String),
}

/// A parsed `ae brief` argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// Which sessions to card.
    pub target: Target,
    /// `--since`, in seconds — topic records older than this are dropped.
    pub since_secs: Option<i64>,
}

/// The argv did not parse; the offending token, when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub Option<String>);

impl Usage {
    /// The stderr text, offending token first when one was named.
    #[must_use]
    pub fn render(&self) -> String {
        match &self.0 {
            Some(token) => format!("ae brief: unexpected {token}\n{USAGE}"),
            None => USAGE.to_owned(),
        }
    }
}

/// Read `tail` — everything after the word `brief`.
///
/// ```
/// use ae::brief::{Args, Target, parse};
///
/// let words = |items: &[&str]| items.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
/// assert_eq!(parse(&[]), Ok(Args::default()));
/// assert_eq!(
///     parse(&words(&["--all", "--since", "2h"])),
///     Ok(Args { target: Target::All, since_secs: Some(7_200) })
/// );
/// assert_eq!(
///     parse(&words(&["aedev"])),
///     Ok(Args { target: Target::Named("aedev".to_owned()), since_secs: None })
/// );
/// assert!(parse(&words(&["--frobnicate"])).is_err());
/// ```
///
/// # Errors
///
/// [`Usage`] for an unknown flag, a second session name, `--since` without a
/// readable duration, or `--all` alongside a name.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    let mut index = 0;
    while let Some(token) = tail.get(index) {
        match token.as_str() {
            "--all" => {
                if matches!(args.target, Target::Named(_)) {
                    return Err(Usage(Some(token.clone())));
                }
                args.target = Target::All;
            }
            "--since" => {
                let value = tail.get(index + 1).ok_or(Usage(Some(token.clone())))?;
                args.since_secs = Some(duration_secs(value).ok_or(Usage(Some(value.clone())))?);
                index += 1;
            }
            // A `-`/`--` token nothing above defines is a usage error, exactly
            // as a `list` tail treats one. A bare word is a session name.
            flag if flag.starts_with('-') => return Err(Usage(Some(token.clone()))),
            name => {
                if args.target != Target::Caller {
                    return Err(Usage(Some(token.clone())));
                }
                args.target = Target::Named(name.to_owned());
            }
        }
        index += 1;
    }
    Ok(args)
}

/// `90`, `45s`, `30m`, `2h`, `3d`, `1w` in seconds — a bare number is seconds.
fn duration_secs(text: &str) -> Option<i64> {
    let (digits, scale) = match text.as_bytes().last() {
        Some(b's') => (&text[..text.len() - 1], 1),
        Some(b'm') => (&text[..text.len() - 1], 60),
        Some(b'h') => (&text[..text.len() - 1], 3_600),
        Some(b'd') => (&text[..text.len() - 1], 86_400),
        Some(b'w') => (&text[..text.len() - 1], 604_800),
        _ => (text, 1),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()?.checked_mul(scale)
}

/// One memo topic's latest checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicLine {
    /// The topic the record was filed under.
    pub topic: String,
    /// How old the record is, in seconds; `None` when its timestamp is unreadable.
    pub age_secs: Option<i64>,
    /// Who wrote it.
    pub author: String,
    /// The record's text.
    pub text: String,
}

/// One agent's declared work state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLine {
    /// The agent's name.
    pub name: String,
    /// Its declared state, or `-` when it has declared none.
    pub state: String,
    /// How long ago it declared that.
    pub age_secs: Option<i64>,
    /// The reason it declared with the state, empty when it gave none.
    pub reason: String,
}

/// One entry of `needs you:` — an EXPLICIT claim on the human's attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    /// An agent declared `waiting-user` or `blocked`.
    Declared {
        /// Who declared it.
        owner: String,
        /// Which of the two states.
        state: String,
        /// How long ago.
        age_secs: Option<i64>,
        /// The reason it gave, empty when it gave none.
        reason: String,
    },
    /// A tracked `ask`/`review` nobody has answered.
    Unanswered {
        /// `ask` or `review`.
        kind: String,
        /// Who asked.
        from: String,
        /// Who was asked.
        to: String,
        /// How long it has waited.
        age_secs: i64,
        /// The question, as the opening event recorded it.
        question: String,
    },
}

/// One session's whole card, as data — every formatting decision lives in
/// [`render`], so a test can assert the facts without asserting the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The session's name.
    pub name: String,
    /// `running` / `unknown` / `stopped`.
    pub status: &'static str,
    /// The session-level attention rollup, when it is exact enough to show.
    pub attention: Option<Reason>,
    /// The ae version the session was created under.
    pub ae_version: Option<String>,
    /// The live branch, when one is known.
    pub branch: Option<String>,
    /// Whether that work tree has tracked modifications — the watchdog's `*`.
    pub dirty: bool,
    /// The work dir, already shortened for display.
    pub work_dir: Option<String>,
    /// The session goal, in full.
    pub goal: Option<String>,
    /// The latest record per memo topic, newest first.
    pub topics: Vec<TopicLine>,
    /// One line per roster agent.
    pub agents: Vec<AgentLine>,
    /// Every explicit claim on the human's attention.
    pub needs: Vec<Need>,
    /// Whether the session's own record could not be fully read — the world's
    /// `degraded`. An empty section on a degraded card is `unknown`, never a
    /// claim that nothing was recorded.
    pub degraded: bool,
    /// Whether a memo file that EXISTS could not be read. Distinct from
    /// `degraded`: the memo is not part of the record the world reads, and a
    /// session can lose it while everything else is intact.
    pub memo_unreadable: bool,
}

impl Card {
    /// What an EMPTY section says.
    ///
    /// The distinction the whole card rests on: `none recorded` is a claim that
    /// the session recorded nothing, and only a complete read may make it. A
    /// degraded record supports no such claim, so it says `unknown` instead.
    #[must_use]
    pub const fn empty_section(&self) -> &'static str {
        if self.degraded {
            " unknown"
        } else {
            " none recorded"
        }
    }

    /// The severity this card sorts by — higher is more actionable.
    #[must_use]
    pub fn urgency(&self) -> i64 {
        self.attention.map_or(0, Reason::rank)
    }
}

/// The whole output for `cards`, in the order given, one blank line between.
#[must_use]
pub fn render(cards: &[Card]) -> String {
    let mut out = String::new();
    for (index, card) in cards.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        push_card(&mut out, card);
    }
    out
}

/// What `ae brief` prints when nothing matched.
pub const NOTHING: &str = "ae brief: no session to brief\n";

/// Append one card.
fn push_card(out: &mut String, card: &Card) {
    push_header(out, card);
    out.push_str("  goal: ");
    out.push_str(&clip(
        card.goal.as_deref().filter(|goal| !goal.is_empty()),
        "none",
        WIDTH - 8,
    ));
    out.push('\n');

    out.push_str("  topics:");
    if card.topics.is_empty() {
        out.push_str(if card.memo_unreadable {
            " unreadable"
        } else {
            card.empty_section()
        });
    }
    out.push('\n');
    for topic in &card.topics {
        out.push_str("    ");
        push_padded(out, &topic.topic, 12);
        push_padded(out, &age(topic.age_secs), 6);
        push_padded(out, &topic.author, 14);
        out.push_str(&clip(Some(&topic.text), "", WIDTH - 36));
        out.push('\n');
    }

    out.push_str("  agents:");
    if card.agents.is_empty() {
        out.push_str(card.empty_section());
    }
    out.push('\n');
    for agent in &card.agents {
        out.push_str("    ");
        push_padded(out, &agent.name, 14);
        push_padded(out, &agent.state, 14);
        push_padded(out, &age(agent.age_secs), 6);
        if !agent.reason.is_empty() {
            out.push('"');
            out.push_str(&clip(Some(&agent.reason), "", WIDTH - 40));
            out.push('"');
        }
        // A line whose last column is empty must not carry the padding of one.
        trim_line_end(out);
        out.push('\n');
    }

    out.push_str("  needs you:");
    if card.needs.is_empty() {
        out.push_str(card.empty_section());
    }
    out.push('\n');
    for need in &card.needs {
        out.push_str("    ");
        push_need(out, need);
        out.push('\n');
    }
}

/// `<name> · <status> · attn:<reason> · ae <ver> · <branch><dirty> · <path>` —
/// a segment ae does not know is left out rather than printed empty.
fn push_header(out: &mut String, card: &Card) {
    let mut segments: Vec<String> = vec![card.name.clone(), card.status.to_owned()];
    if let Some(reason) = card.attention {
        segments.push(format!("attn:{}", reason.as_str()));
    }
    if let Some(version) = card.ae_version.as_deref().filter(|v| !v.is_empty()) {
        segments.push(format!("ae {version}"));
    }
    if let Some(branch) = card.branch.as_deref().filter(|b| !b.is_empty()) {
        segments.push(format!("{branch}{}", if card.dirty { "*" } else { "" }));
    }
    if let Some(dir) = card.work_dir.as_deref().filter(|d| !d.is_empty()) {
        segments.push(dir.to_owned());
    }
    // LAST, so it reads as a caveat on everything before it.
    if card.degraded {
        segments.push("degraded".to_owned());
    }
    out.push_str(&segments.join(" · "));
    out.push('\n');
}

/// One `needs you:` entry.
fn push_need(out: &mut String, need: &Need) {
    match need {
        Need::Declared {
            owner,
            state,
            age_secs,
            reason,
        } => {
            push_padded(out, owner, 14);
            push_padded(out, state, 14);
            push_padded(out, &age(*age_secs), 6);
            out.push_str(&clip(Some(reason), "no reason given", WIDTH - 40));
        }
        Need::Unanswered {
            kind,
            from,
            to,
            age_secs,
            question,
        } => {
            // The SAME three columns a declared need uses, so the section reads
            // as one grid rather than as two tables that happen to be adjacent.
            push_padded(out, kind, 14);
            push_padded(out, &format!("{from} → {to}"), 14);
            push_padded(out, &age(Some(*age_secs)), 6);
            out.push_str(&clip(Some(question), "no text recorded", WIDTH - 40));
        }
    }
}

/// `field` then spaces to `width`, and at least one space when it overruns.
fn push_padded(out: &mut String, field: &str, width: usize) {
    out.push_str(field);
    let printed = field.chars().count();
    for _ in printed..width {
        out.push(' ');
    }
    if printed >= width {
        out.push(' ');
    }
}

/// Drop the trailing spaces of the line being built.
fn trim_line_end(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
}

/// `text` capped at `max` characters, the last reserved for an ellipsis, or
/// `fallback` when there is nothing to show.
fn clip(text: Option<&str>, fallback: &str, max: usize) -> String {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return fallback.to_owned();
    };
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// A compact age: `12s`, `4m`, `3h`, `2d`, `>7d`, or `-`.
#[must_use]
pub fn age(secs: Option<i64>) -> String {
    let Some(secs) = secs else {
        return "-".to_owned();
    };
    let secs = secs.max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else if secs < 604_800 {
        format!("{}d", secs / 86_400)
    } else {
        ">7d".to_owned()
    }
}

// ── collection: the one place a session directory is read ───────────────────

/// Build `entry`'s card from the session store at `dir`.
///
/// `home` shortens the work dir to a `~` path when it is under it. `dirty` is
/// the work-tree fact the caller observed, passed in so a card can be built
/// without asking git anything.
///
/// **Everything but the memo file comes off ONE read of the event container**,
/// through the same two sensors the helpers use — [`crate::state::latest`] for a
/// declaration and [`crate::requests::states`] for an open request. A brief does
/// not open a session's meta or its event log a second time: the world it is
/// handed was already built from that read, and a second reader is a second
/// chance to disagree with it.
#[must_use]
pub fn card_for(
    entry: &SessionEntry,
    dir: &Path,
    home: Option<&Path>,
    dirty: bool,
    now: Timestamp,
    since_secs: Option<i64>,
) -> Card {
    let store = crate::store::open(dir);
    let container = store.container();
    let agents = agent_lines(entry, &container, now);
    // LOUD, deliberately: `memo_bytes_or_empty` is the compaction handover's
    // quiet read, and rendering an unreadable memory as an empty one is the
    // one thing `store.rs` says a human read must not do.
    let memo = store.memo_bytes();
    Card {
        name: entry.name.clone(),
        status: entry.status.as_str(),
        // The same exactness rule the table applies: a class marker is not
        // printed off partial evidence.
        attention: entry.attention.filter(|_| entry.attention_is_exact()),
        ae_version: entry.ae_version.clone(),
        branch: entry.branch.clone(),
        dirty,
        work_dir: entry.work_dir.as_deref().map(|path| short_path(path, home)),
        goal: entry.goal.clone(),
        topics: topic_lines(memo.as_deref().unwrap_or_default(), now, since_secs),
        memo_unreadable: memo.is_err(),
        needs: needs(&agents, &container, now),
        agents,
        degraded: entry.degraded,
    }
}

/// The work dir as a card shows it: `~`-relative under `home`, else verbatim.
fn short_path(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home.map(|home| home.to_string_lossy().into_owned()) else {
        return path.to_owned();
    };
    if home.is_empty() {
        return path.to_owned();
    }
    match path.strip_prefix(&home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

/// The latest record per topic in a `memo.tsv`, newest first.
///
/// The file is read by [`crate::memo::records`] — the ONE parser of this
/// hand-editable container, shared with [`crate::memo::render`], so a brief and
/// a `memo read` can never disagree about what a record is. Totality over
/// hostile bytes is that parser's property and is proven there.
///
/// What is added here is total too: [`String::from_utf8_lossy`], a
/// [`Timestamp::parse`] that returns `None`, and a saturating subtraction — no
/// index, no slice range, no `unwrap`.
///
/// ```
/// use ae::brief::topic_lines;
/// use ae::time::Timestamp;
///
/// let now = Timestamp::parse("2026-09-06T12:00:00Z").unwrap();
/// let file = concat!(
///     "2026-09-06T11:00:00Z\tcl:lead\tdecision\troute review through ae\n",
///     "2026-09-06T11:30:00Z\tcl:lead\tdecision\tand gate once per merge\n",
///     "2026-09-06T11:50:00Z\tcl:brief\tparking\tresume here: renderer half done\n",
/// );
/// let lines = topic_lines(file.as_bytes(), now, None);
/// // One line per topic, newest topic first, each carrying that topic's LATEST record.
/// assert_eq!(
///     lines.iter().map(|line| line.topic.as_str()).collect::<Vec<_>>(),
///     ["parking", "decision"]
/// );
/// assert_eq!(lines[1].text, "and gate once per merge");
/// assert_eq!(lines[1].age_secs, Some(1_800));
/// ```
#[must_use]
pub fn topic_lines(container: &[u8], now: Timestamp, since_secs: Option<i64>) -> Vec<TopicLine> {
    let mut latest: Vec<TopicLine> = Vec::new();
    for record in crate::memo::records(container) {
        let stamp = Timestamp::parse(&String::from_utf8_lossy(record.ts));
        let line = TopicLine {
            topic: String::from_utf8_lossy(record.topic).into_owned(),
            age_secs: stamp.map(|stamp| stamp.seconds_until(now)),
            author: String::from_utf8_lossy(record.author).into_owned(),
            text: String::from_utf8_lossy(record.text).into_owned(),
        };
        // Append-only, so a later record for a topic REPLACES the one held.
        match latest.iter().position(|held| held.topic == line.topic) {
            Some(at) => latest[at] = line,
            None => latest.push(line),
        }
    }
    // `--since` drops what is older; a record whose timestamp did not parse has
    // no age to judge, so it is kept rather than silently dropped.
    if let Some(window) = since_secs {
        latest.retain(|line| line.age_secs.is_none_or(|age| age <= window));
    }
    // Newest first: an unreadable timestamp sorts last rather than first.
    latest.sort_by_key(|line| line.age_secs.unwrap_or(i64::MAX));
    latest
}

/// One line per agent, in roster order.
///
/// The STATE is the world's — the same cell `ae list` prints, under the same
/// exactness rule. Only the age and the reason are read here, and only for a
/// declaration that still agrees with that cell: a `Latest` naming some other
/// value describes a superseded declaration, and its reason would be a caption
/// on the wrong state.
fn agent_lines(entry: &SessionEntry, container: &[u8], now: Timestamp) -> Vec<AgentLine> {
    entry
        .agents
        .iter()
        .map(|agent| {
            let state = match (entry.agent_state_is_exact(), agent.state.as_deref()) {
                (true, Some(state)) => state.to_owned(),
                (true, None) => "-".to_owned(),
                (false, _) => "unknown".to_owned(),
            };
            let declared = state::latest(container, &agent.reference)
                .filter(|latest| latest.value == state.as_bytes());
            AgentLine {
                name: agent.name.clone(),
                state,
                age_secs: declared.as_ref().and_then(|latest| {
                    Timestamp::parse(&String::from_utf8_lossy(&latest.ts))
                        .map(|ts| ts.seconds_until(now))
                }),
                reason: declared
                    .map(|latest| String::from_utf8_lossy(&latest.reason).into_owned())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// The two explicit sources of a claim on the human's attention: a declaration
/// of `waiting-user`/`blocked`, and a tracked request nobody has closed.
fn needs(agents: &[AgentLine], container: &[u8], now: Timestamp) -> Vec<Need> {
    let mut needs: Vec<Need> = agents
        .iter()
        .filter(|agent| matches!(agent.state.as_str(), "waiting-user" | "blocked"))
        .map(|agent| Need::Declared {
            owner: agent.name.clone(),
            state: agent.state.clone(),
            age_secs: agent.age_secs,
            reason: agent.reason.clone(),
        })
        .collect();
    for request in requests::states(container) {
        if request.status != RequestStatus::Pending {
            continue;
        }
        let text = |field: &[u8]| String::from_utf8_lossy(field).into_owned();
        needs.push(Need::Unanswered {
            kind: text(&request.kind),
            from: text(&request.from),
            to: text(&request.to),
            age_secs: Timestamp::parse(&text(&request.at))
                .map_or(0, |sent| sent.seconds_until(now)),
            question: text(&request.summary),
        });
    }
    needs
}

/// The cards for `entries`, most actionable first, ties broken by name.
///
/// Sorting lives here rather than at the call site because "most attention
/// first" is what `--all` MEANS, and a second ordering path is a second chance
/// to disagree with it.
#[must_use]
pub fn ordered(mut cards: Vec<Card>) -> Vec<Card> {
    cards.sort_by(|left, right| {
        right
            .urgency()
            .cmp(&left.urgency())
            .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
    });
    cards
}

/// Whether this entry's work tree is worth asking git about — a stopped or
/// degraded session has nothing live to be dirty.
#[must_use]
pub fn wants_git(entry: &SessionEntry) -> bool {
    entry.status != Status::Stopped
        && !entry.degraded
        && entry.work_dir.as_deref().is_some_and(|dir| !dir.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        AgentLine, Args, Card, Need, Target, TopicLine, age, clip, duration_secs, ordered, parse,
        render, short_path, topic_lines,
    };
    use crate::attention::Reason;
    use crate::time::Timestamp;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn card(name: &str) -> Card {
        Card {
            name: name.to_owned(),
            status: "running",
            attention: None,
            ae_version: Some("2026.9.5".to_owned()),
            branch: Some("main".to_owned()),
            dirty: false,
            work_dir: Some("~/projects/ae".to_owned()),
            goal: Some("ship S1".to_owned()),
            topics: Vec::new(),
            agents: Vec::new(),
            needs: Vec::new(),
            degraded: false,
            memo_unreadable: false,
        }
    }

    #[test]
    fn a_bare_word_is_a_session_and_an_unknown_flag_is_a_usage_error() {
        assert_eq!(
            parse(&words(&["aedev"])),
            Ok(Args {
                target: Target::Named("aedev".to_owned()),
                since_secs: None
            })
        );
        assert!(parse(&words(&["--frobnicate"])).is_err());
        // Two names, or a name plus --all, is a contradiction rather than a
        // silent last-wins.
        assert!(parse(&words(&["one", "two"])).is_err());
        assert!(parse(&words(&["one", "--all"])).is_err());
    }

    #[test]
    fn since_reads_every_unit_and_refuses_anything_else() {
        assert_eq!(duration_secs("90"), Some(90));
        assert_eq!(duration_secs("45s"), Some(45));
        assert_eq!(duration_secs("30m"), Some(1_800));
        assert_eq!(duration_secs("2h"), Some(7_200));
        assert_eq!(duration_secs("3d"), Some(259_200));
        assert_eq!(duration_secs("1w"), Some(604_800));
        for refused in ["", "m", "-5", "2y", "1 h", "9223372036854775807d"] {
            assert_eq!(duration_secs(refused), None, "{refused}");
        }
        assert!(parse(&words(&["--since"])).is_err(), "no operand");
        assert!(parse(&words(&["--since", "soon"])).is_err());
    }

    #[test]
    fn the_header_leaves_out_what_ae_does_not_know_rather_than_printing_it_empty() {
        let mut bare = card("alpha");
        bare.ae_version = None;
        bare.branch = None;
        bare.work_dir = None;
        bare.goal = None;
        let rendered = render(&[bare]);
        let header = rendered.lines().next().unwrap_or_default();
        assert_eq!(header, "alpha · running", "{rendered}");
        assert!(
            !header.contains("··") && !header.ends_with(" ·"),
            "an absent segment must not leave its separator: {header}"
        );
        assert!(rendered.contains("  goal: none\n"), "{rendered}");
    }

    #[test]
    fn a_dirty_work_tree_marks_the_branch_and_a_clean_one_does_not() {
        let mut dirty = card("alpha");
        dirty.dirty = true;
        assert!(render(&[dirty]).starts_with("alpha · running · ae 2026.9.5 · main* · "));
        assert!(render(&[card("alpha")]).contains(" · main · "));
    }

    #[test]
    fn every_empty_section_says_none_recorded_rather_than_vanishing() {
        let rendered = render(&[card("alpha")]);
        for section in [
            "  topics: none recorded",
            "  agents: none recorded",
            "  needs you: none recorded",
        ] {
            assert!(rendered.contains(section), "{section}: {rendered}");
        }
    }

    #[test]
    fn needs_you_carries_both_explicit_sources_and_nothing_else() {
        let mut entry = card("alpha");
        entry.needs = vec![
            Need::Declared {
                owner: "brief".to_owned(),
                state: "blocked".to_owned(),
                age_secs: Some(720),
                reason: "waiting on the codex read".to_owned(),
            },
            Need::Unanswered {
                kind: "ask".to_owned(),
                from: "lead".to_owned(),
                to: "colead".to_owned(),
                age_secs: 2_460,
                question: "does the strip pin orchestrator first?".to_owned(),
            },
        ];
        let rendered = render(&[entry]);
        assert!(rendered.contains("brief"), "{rendered}");
        assert!(rendered.contains("blocked"), "{rendered}");
        assert!(rendered.contains("12m"), "{rendered}");
        assert!(rendered.contains("waiting on the codex read"), "{rendered}");
        assert!(rendered.contains("lead → colead"), "{rendered}");
        assert!(rendered.contains("41m"), "{rendered}");
        assert!(
            rendered.contains("does the strip pin orchestrator first?"),
            "{rendered}"
        );
    }

    #[test]
    fn an_agent_with_no_reason_leaves_no_empty_quotes_and_no_trailing_space() {
        let mut entry = card("alpha");
        entry.agents = vec![
            AgentLine {
                name: "lead".to_owned(),
                state: "working".to_owned(),
                age_secs: Some(180),
                reason: String::new(),
            },
            AgentLine {
                name: "brief".to_owned(),
                state: "blocked".to_owned(),
                age_secs: Some(720),
                reason: "codex read".to_owned(),
            },
        ];
        let rendered = render(&[entry]);
        assert!(
            rendered.contains("    lead          working       3m\n"),
            "{rendered}"
        );
        assert!(rendered.contains(r#""codex read""#), "{rendered}");
        assert!(!rendered.contains(r#""""#), "{rendered}");
        for line in rendered.lines() {
            assert!(!line.ends_with(' '), "trailing space: {line:?}");
        }
    }

    #[test]
    fn cards_order_by_attention_then_by_name() {
        let mut blocked = card("zeta");
        blocked.attention = Some(Reason::Blocked);
        let mut dead = card("mid");
        dead.attention = Some(Reason::Dead);
        let quiet = card("alpha");
        let order: Vec<String> = ordered(vec![quiet, blocked, dead])
            .into_iter()
            .map(|card| card.name)
            .collect();
        assert_eq!(order, ["mid", "zeta", "alpha"]);
    }

    #[test]
    fn two_cards_are_separated_by_exactly_one_blank_line() {
        let rendered = render(&[card("alpha"), card("zeta")]);
        assert!(
            rendered.contains("none recorded\n\nzeta · running"),
            "{rendered}"
        );
        assert!(!rendered.contains("\n\n\n"), "{rendered}");
    }

    #[test]
    fn topics_keep_the_latest_record_per_topic_and_since_drops_the_older_ones() {
        let now = Timestamp::parse("2026-09-06T12:00:00Z").expect("a timestamp");
        let file = concat!(
            "2026-09-06T09:00:00Z\tcl:lead\tdecision\tfirst call\n",
            "2026-09-06T11:30:00Z\tcl:lead\tdecision\tsecond call\n",
            "2026-09-06T04:00:00Z\tcl:brief\tparking\tresume here: renderer\n",
            "not-a-record\n",
        );
        let all = topic_lines(file.as_bytes(), now, None);
        assert_eq!(
            all.iter()
                .map(|line| line.topic.as_str())
                .collect::<Vec<_>>(),
            ["decision", "parking"],
            "newest topic first"
        );
        assert_eq!(all[0].text, "second call", "the LATEST record wins");
        let recent = topic_lines(file.as_bytes(), now, Some(3_600));
        assert_eq!(recent.len(), 1, "--since drops the older topic: {recent:?}");
        assert_eq!(recent[0].topic, "decision");
    }

    #[test]
    fn a_record_whose_timestamp_does_not_parse_is_kept_and_sorts_last() {
        let now = Timestamp::parse("2026-09-06T12:00:00Z").expect("a timestamp");
        let file = concat!(
            "not-a-time\tcl:lead\tbroken\tstill worth showing\n",
            "2026-09-06T11:30:00Z\tcl:lead\tdecision\treadable\n",
        );
        let lines = topic_lines(file.as_bytes(), now, Some(3_600));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.topic.as_str())
                .collect::<Vec<_>>(),
            ["decision", "broken"],
            "an unreadable age is not a reason to drop the record: {lines:?}"
        );
        assert_eq!(lines[1].age_secs, None);
    }

    #[test]
    fn a_long_field_is_clipped_with_an_ellipsis_and_the_card_stays_within_its_width() {
        let mut entry = card("alpha");
        entry.goal = Some("g".repeat(400));
        entry.topics = vec![TopicLine {
            topic: "decision".to_owned(),
            age_secs: Some(60),
            author: "cl:lead".to_owned(),
            text: "t".repeat(400),
        }];
        let rendered = render(&[entry]);
        for line in rendered.lines() {
            assert!(
                line.chars().count() <= super::WIDTH,
                "{} chars: {line}",
                line.chars().count()
            );
        }
        assert!(rendered.contains('…'), "{rendered}");
    }

    #[test]
    fn clip_counts_characters_never_bytes() {
        // A byte cut here would panic on a multi-byte boundary rather than
        // shorten anything.
        assert_eq!(clip(Some("ααααα"), "", 3), "αα…");
        assert_eq!(clip(Some("αα"), "", 8), "αα");
        assert_eq!(clip(None, "none", 8), "none");
        assert_eq!(clip(Some(""), "none", 8), "none");
    }

    #[test]
    fn an_age_reads_in_the_largest_unit_that_fits() {
        assert_eq!(age(None), "-");
        assert_eq!(age(Some(-5)), "0s", "a clock skew is not a negative age");
        assert_eq!(age(Some(59)), "59s");
        assert_eq!(age(Some(60)), "1m");
        assert_eq!(age(Some(3_599)), "59m");
        assert_eq!(age(Some(3_600)), "1h");
        assert_eq!(age(Some(86_400)), "1d");
        assert_eq!(age(Some(604_800)), ">7d");
    }

    #[test]
    fn hostile_bytes_still_leave_one_line_per_topic() {
        // The PARSER's totality is proven at its home
        // (`memo::no_byte_sequence_makes_the_record_parser_panic`). What is
        // proven here is what this consumer adds on top of it: a timestamp that
        // does not parse, a topic that repeats, and a `--since` window must
        // still leave exactly one line per topic.
        let now = Timestamp::parse("2026-09-06T12:00:00Z").expect("a timestamp");
        let mut seed = 0x2026_0906_u64;
        let mut soup = Vec::new();
        for _ in 0..512 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_possible_truncation, reason = "a byte is what this wants")]
            soup.push((seed >> 33) as u8);
        }
        let containers: [&[u8]; 5] = [
            b"",
            b"\t\t\t\n\t\t\t",
            b"not-a-time\thuman\tt\tx\nnot-a-time\thuman\tt\ty",
            b"9223372036854775807\thuman\tt\tx\n-9223372036854775808\thuman\tt\ty",
            &soup,
        ];
        for container in containers {
            for window in [None, Some(0), Some(3_600), Some(i64::MAX)] {
                let lines = topic_lines(container, now, window);
                let mut topics: Vec<&str> = lines.iter().map(|line| line.topic.as_str()).collect();
                let before = topics.len();
                topics.sort_unstable();
                topics.dedup();
                assert_eq!(before, topics.len(), "a topic was rendered twice");
            }
        }
    }

    #[test]
    fn a_degraded_card_says_unknown_where_a_complete_one_says_none_recorded() {
        // THE DISTINCTION THE CARD RESTS ON. `none recorded` is a claim about
        // the session; a record ae could not fully read supports no such claim.
        let complete = render(&[card("alpha")]);
        assert!(complete.contains("  topics: none recorded\n"), "{complete}");
        assert!(!complete.contains("degraded"), "{complete}");

        let mut lost = card("alpha");
        lost.degraded = true;
        let rendered = render(&[lost]);
        for section in [
            "  topics: unknown\n",
            "  agents: unknown\n",
            "  needs you: unknown\n",
        ] {
            assert!(rendered.contains(section), "{section}: {rendered}");
        }
        assert!(
            rendered
                .lines()
                .next()
                .is_some_and(|line| line.ends_with(" · degraded")),
            "the header carries the caveat last: {rendered}"
        );
    }

    #[test]
    fn a_memo_that_cannot_be_read_is_said_out_loud_not_rendered_as_an_empty_memory() {
        let mut unreadable = card("alpha");
        unreadable.memo_unreadable = true;
        let rendered = render(&[unreadable]);
        assert!(rendered.contains("  topics: unreadable\n"), "{rendered}");
        // And it is NOT the same claim as a degraded record: the memo is not
        // part of what the world reads.
        assert!(!rendered.contains("degraded"), "{rendered}");
    }

    #[test]
    fn a_work_dir_under_home_shortens_and_one_that_only_shares_a_prefix_does_not() {
        let home = std::path::Path::new("/Users/x");
        assert_eq!(
            short_path("/Users/x/projects/ae", Some(home)),
            "~/projects/ae"
        );
        assert_eq!(short_path("/Users/x", Some(home)), "~");
        assert_eq!(
            short_path("/Users/xavier/ae", Some(home)),
            "/Users/xavier/ae",
            "a shared prefix is not containment"
        );
        assert_eq!(short_path("/srv/ae", Some(home)), "/srv/ae");
        assert_eq!(short_path("/Users/x/ae", None), "/Users/x/ae");
    }
}
