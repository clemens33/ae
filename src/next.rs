//! `ae next` / `ae jump` — the attention navigator.
//!
//! Frozen `cmd_next` names the top-ranked RUNNING session that needs a human and
//! either prints it or jumps to it. It re-used `_session_attn_rollup` — the same
//! rollup `ae list` reads — precisely so the two could never disagree about what
//! wants attention; this module inherits that property by construction, because
//! it selects over the very [`World`] the list path renders.

use crate::attention::Reason;
use crate::digest::{SessionEntry, Status};
use crate::listing::World;

/// Frozen's refusal code when nothing needs a human — non-zero "so it composes
/// in scripts and Layer 3", and distinct from the usage 2.
pub const EXIT_NONE: u8 = 1;

/// Frozen's code for an argument it does not accept.
pub const EXIT_USAGE: u8 = 2;

/// What frozen prints when no running session needs attention — stderr.
pub const NOTHING: &str = "ae next: no running session needs attention.";

/// `--help`, verbatim. Frozen wrote it to STDERR and exited 0; both are frozen
/// behaviour and neither is this module's to improve.
pub const USAGE: &str = "\
Usage: ae next [--attach]   Name the top running session needing attention.
       ae jump [--attach]   Alias for ae next.

Read-only by default: prints \"<session>  attn:<reason>  rank:<n>  <agent>\" and
exits 0, or a message on stderr and non-zero when nothing needs attention.
--attach (alias --switch) jumps to that session: switch-client inside tmux,
attach-session outside.
";

/// What the argv asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Args {
    /// `--attach` (or its `--switch` spelling): jump, rather than print.
    pub attach: bool,
}

/// An argv frozen refuses, or the help it treats as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// `-h` / `--help` — the text, at exit 0.
    Help,
    /// A word `ae next` does not accept, carried for its message.
    Unknown(String),
}

impl Usage {
    /// The stderr this refusal prints, terminating newline included.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Help => USAGE.to_owned(),
            Self::Unknown(token) => {
                format!("ae next: unknown argument '{token}' (see: ae next --help)\n")
            }
        }
    }

    /// The exit it takes. Help is a SUCCESS in frozen bash, not a usage error.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Help => 0,
            Self::Unknown(_) => EXIT_USAGE,
        }
    }
}

/// Read `ae next`'s flags, in the order frozen read them.
///
/// # Errors
///
/// [`Usage::Help`] for the help spellings, [`Usage::Unknown`] for anything else
/// that is not `--attach`/`--switch`.
///
/// ```
/// use ae::next::{parse, Args, Usage};
/// assert_eq!(parse(&[]), Ok(Args { attach: false }));
/// assert_eq!(parse(&["--switch".to_owned()]), Ok(Args { attach: true }));
/// assert_eq!(parse(&["--nope".to_owned()]), Err(Usage::Unknown("--nope".to_owned())));
/// ```
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    for word in tail {
        match word.as_str() {
            "-h" | "--help" => return Err(Usage::Help),
            "--attach" | "--switch" => args.attach = true,
            other => return Err(Usage::Unknown(other.to_owned())),
        }
    }
    Ok(args)
}

/// The session `ae next` names, with the facts its line prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The session's name.
    pub name: String,
    /// Its attention rollup — the reason and, via [`Reason::rank`], the number.
    pub reason: Reason,
    /// The agent that raised it, or empty when no agent owns the reason.
    pub agent: String,
}

impl Choice {
    /// Frozen's one line: `<session>  attn:<reason>  rank:<n>  <agent>`, two
    /// spaces between fields, terminating newline included.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{}  attn:{}  rank:{}  {}\n",
            self.name,
            self.reason.as_str(),
            self.reason.rank(),
            self.agent
        )
    }
}

/// The top-ranked running session needing attention, or `None`.
///
/// ```
/// use ae::attention::Reason;
/// use ae::digest::{SessionEntry, Status};
/// use ae::listing::World;
/// use ae::time::Timestamp;
///
/// let mut hot = SessionEntry::new("hot", Status::Running);
/// hot.attention = Some(Reason::Dead);
/// let mut mild = SessionEntry::new("mild", Status::Running);
/// mild.attention = Some(Reason::Blocked);
/// let world = World::new(Timestamp::from_epoch(0), vec![mild, hot]);
/// assert_eq!(ae::next::choose(&world).unwrap().name, "hot");
/// ```
#[must_use]
pub fn choose(world: &World) -> Option<Choice> {
    let mut best: Option<(&SessionEntry, Reason)> = None;
    for session in &world.sessions {
        if session.status != Status::Running {
            continue;
        }
        let Some(reason) = session.attention else {
            continue;
        };
        let replaces = match best {
            None => true,
            Some((incumbent, held)) => better(
                (reason, epoch_of(session), &session.name),
                (held, epoch_of(incumbent), &incumbent.name),
            ),
        };
        if replaces {
            best = Some((session, reason));
        }
    }
    best.map(|(session, reason)| Choice {
        name: session.name.clone(),
        reason,
        agent: raiser(session, reason),
    })
}

/// Frozen's `_next_better`: rank, then recency, then name ascending.
fn better(candidate: (Reason, i64, &String), incumbent: (Reason, i64, &String)) -> bool {
    let (rank, epoch, name) = candidate;
    let (best_rank, best_epoch, best_name) = incumbent;
    (rank.rank(), epoch, std::cmp::Reverse(name))
        > (best_rank.rank(), best_epoch, std::cmp::Reverse(best_name))
}

/// Frozen's `_session_active_epoch`, as the digest already answers it. An
/// unknown activity is frozen's `0` — never a reason to skip the session.
fn epoch_of(session: &SessionEntry) -> i64 {
    session.last_active_epoch.unwrap_or(0)
}

/// The agent whose own reason IS the session's rollup, in roster order.
fn raiser(session: &SessionEntry, reason: Reason) -> String {
    session
        .agents
        .iter()
        .find(|agent| agent.reason == Some(reason))
        .map(|agent| agent.reference.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{Args, Choice, EXIT_NONE, EXIT_USAGE, Usage, choose, parse};
    use crate::attention::Reason;
    use crate::digest::{AgentEntry, SessionEntry, Status};
    use crate::listing::World;
    use crate::time::Timestamp;

    fn world(sessions: Vec<SessionEntry>) -> World {
        World::new(Timestamp::from_epoch(1_780_000_000), sessions)
    }

    fn needing(name: &str, reason: Reason, epoch: i64) -> SessionEntry {
        let mut entry = SessionEntry::new(name, Status::Running);
        entry.attention = Some(reason);
        entry.last_active_epoch = Some(epoch);
        entry
    }

    fn agent(reference: &str, reason: Option<Reason>) -> AgentEntry {
        AgentEntry {
            reference: reference.to_owned(),
            alias: "cl".to_owned(),
            name: reference.to_owned(),
            session_id: None,
            alive: Some(true),
            state: None,
            reason,
        }
    }

    #[test]
    fn a_quiet_world_names_nothing() {
        assert_eq!(choose(&world(Vec::new())), None);
        let calm = SessionEntry::new("calm", Status::Running);
        assert_eq!(choose(&world(vec![calm])), None);
    }

    #[test]
    fn a_stopped_session_is_never_a_candidate_however_loudly_it_asks() {
        // Frozen iterated `list_ae_sessions`, which asks tmux: a session that is
        // not running cannot appear there at all. The exclusion is the status,
        // and it beats the highest rank there is.
        let mut stopped = SessionEntry::new("gone", Status::Stopped);
        stopped.attention = Some(Reason::Dead);
        assert_eq!(choose(&world(vec![stopped])), None);

        let mut unknown = SessionEntry::new("unsure", Status::Unknown);
        unknown.attention = Some(Reason::Dead);
        assert_eq!(choose(&world(vec![unknown])), None);
    }

    #[test]
    fn severity_outranks_recency() {
        let old_and_dead = needing("dead", Reason::Dead, 100);
        let fresh_and_blocked = needing("blocked", Reason::Blocked, 9_999);
        let chosen = choose(&world(vec![fresh_and_blocked, old_and_dead])).unwrap();
        assert_eq!(chosen.name, "dead");
        assert_eq!(chosen.reason, Reason::Dead);
    }

    #[test]
    fn equal_rank_breaks_on_recency_then_on_name_ascending() {
        let older = needing("bbb", Reason::Stale, 100);
        let newer = needing("aaa", Reason::Stale, 200);
        assert_eq!(choose(&world(vec![older, newer])).unwrap().name, "aaa");

        // Same rank AND same epoch: the name decides, ascending, whichever order
        // the world listed them in — frozen's "stable and scriptable, not
        // tmux-list order".
        let first = needing("zeta", Reason::Stale, 500);
        let second = needing("alpha", Reason::Stale, 500);
        assert_eq!(
            choose(&world(vec![first.clone(), second.clone()]))
                .unwrap()
                .name,
            "alpha"
        );
        assert_eq!(choose(&world(vec![second, first])).unwrap().name, "alpha");
    }

    #[test]
    fn an_unknown_activity_is_zero_and_still_a_candidate() {
        let mut silent = SessionEntry::new("silent", Status::Running);
        silent.attention = Some(Reason::Dead);
        // No last_active_epoch at all — frozen's `printf '0'` when no file has
        // an mtime. It still wins against nothing.
        assert_eq!(choose(&world(vec![silent.clone()])).unwrap().name, "silent");
        // …and loses the tie-break to a session that HAS activity.
        let timed = needing("timed", Reason::Dead, 1);
        assert_eq!(choose(&world(vec![silent, timed])).unwrap().name, "timed");
    }

    #[test]
    fn the_line_names_the_agent_that_raised_the_reason() {
        let mut entry = needing("sess", Reason::Dead, 7);
        entry.agents = vec![
            agent("cl:quiet", None),
            agent("cl:blocked", Some(Reason::Blocked)),
            agent("cl:gone", Some(Reason::Dead)),
        ];
        let chosen = choose(&world(vec![entry])).unwrap();
        assert_eq!(chosen.agent, "cl:gone");
        assert_eq!(chosen.line(), "sess  attn:dead  rank:6  cl:gone\n");
    }

    #[test]
    fn a_session_level_reason_no_agent_owns_names_no_agent() {
        // `unanswered` is the pair fact the core keeps off every agent. The line
        // still renders — with an empty last field, exactly as frozen's printf
        // does when the rollup found no contributing agent.
        let mut entry = needing("waiting", Reason::Unanswered, 7);
        entry.agents = vec![agent("cl:lead", None)];
        let chosen = choose(&world(vec![entry])).unwrap();
        assert_eq!(chosen.agent, "");
        assert_eq!(chosen.line(), "waiting  attn:unanswered  rank:1  \n");
    }

    #[test]
    fn every_reason_renders_its_own_rank() {
        for reason in Reason::BY_SEVERITY {
            let choice = Choice {
                name: "s".to_owned(),
                reason,
                agent: "cl:a".to_owned(),
            };
            assert_eq!(
                choice.line(),
                format!(
                    "s  attn:{}  rank:{}  cl:a\n",
                    reason.as_str(),
                    reason.rank()
                )
            );
        }
    }

    #[test]
    fn the_flags_frozen_accepts_and_the_ones_it_refuses() {
        assert_eq!(parse(&[]), Ok(Args { attach: false }));
        for spelling in ["--attach", "--switch"] {
            assert_eq!(
                parse(&[spelling.to_owned()]),
                Ok(Args { attach: true }),
                "{spelling}"
            );
        }
        // Repeated is not an error; frozen just sets the flag again.
        assert_eq!(
            parse(&["--attach".to_owned(), "--switch".to_owned()]),
            Ok(Args { attach: true })
        );
        for spelling in ["-h", "--help"] {
            assert_eq!(
                parse(&[spelling.to_owned()]),
                Err(Usage::Help),
                "{spelling}"
            );
        }
        assert_eq!(
            parse(&["nope".to_owned()]),
            Err(Usage::Unknown("nope".to_owned()))
        );
    }

    #[test]
    fn the_first_word_that_answers_decides() {
        // Frozen's loop returns where it stands. A refused word BEFORE --help is
        // the refusal; --help before a refused word is the help.
        assert_eq!(
            parse(&["--bogus".to_owned(), "--help".to_owned()]),
            Err(Usage::Unknown("--bogus".to_owned()))
        );
        assert_eq!(
            parse(&["--help".to_owned(), "--bogus".to_owned()]),
            Err(Usage::Help)
        );
    }

    #[test]
    fn help_exits_zero_and_a_refusal_exits_two() {
        assert_eq!(Usage::Help.code(), 0);
        assert_eq!(Usage::Unknown("x".to_owned()).code(), EXIT_USAGE);
        assert_eq!(
            Usage::Unknown("--zap".to_owned()).render(),
            "ae next: unknown argument '--zap' (see: ae next --help)\n"
        );
        assert!(
            Usage::Help
                .render()
                .starts_with("Usage: ae next [--attach]")
        );
        assert_eq!(EXIT_NONE, 1, "nothing-to-do is not a usage error");
    }
}
