//! The typed wait target a `waiting-agent` declaration may name with
//! `--on`, and the ONE fold that follows those targets inside a session.
//!
//! PURE. The target is checked against the existing name grammars and
//! nothing else: ae never asks whether it exists here, never infers one from
//! reason prose. A same-session edge is followed to its end; an edge that
//! leaves the session ends the chain there, which is the one answer both the
//! ceiling exemption and the drawn words read.

use crate::config::is_agent_name;
use crate::session_launch::name::is_session_name;

/// A declared wait target: its spelling as declared and what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    label: String,
    kind: Kind,
    /// `@<current session>`: accepted and drawn as declared, but it names no
    /// other session, so a chain ending there stays in this one.
    own: bool,
}

/// What a target names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A seat of this session: `<agent>` or `<current session>:<agent>`.
    Seat(String),
    /// A seat of another session: `<session>:<agent>`.
    Remote { session: String, agent: String },
    /// Another session as a whole: `@<session>`.
    Session(String),
}

impl Target {
    /// Read `raw` against the agent and session grammars, from inside
    /// `current_session`. `None` for anything else.
    ///
    /// ```
    /// use ae::wait_on::{Kind, Target};
    ///
    /// let seat = Target::parse("mac:colimafix", "mac").unwrap();
    /// assert_eq!(seat.label(), "mac:colimafix");
    /// assert_eq!(seat.kind(), &Kind::Seat("colimafix".to_owned()));
    /// assert!(Target::parse("@owner:lead", "mac").is_none());
    /// ```
    #[must_use]
    pub fn parse(raw: &str, current_session: &str) -> Option<Self> {
        let (kind, own) = if let Some(session) = raw.strip_prefix('@') {
            if !is_session_name(session) {
                return None;
            }
            (
                Kind::Session(session.to_owned()),
                session == current_session,
            )
        } else if let Some((session, agent)) = raw.split_once(':') {
            if !is_session_name(session) || !is_agent_name(agent) {
                return None;
            }
            if session == current_session {
                (Kind::Seat(agent.to_owned()), false)
            } else {
                let (session, agent) = (session.to_owned(), agent.to_owned());
                (Kind::Remote { session, agent }, false)
            }
        } else if is_agent_name(raw) {
            (Kind::Seat(raw.to_owned()), false)
        } else {
            return None;
        };
        Some(Self {
            label: raw.to_owned(),
            kind,
            own,
        })
    }

    /// The target exactly as it was declared.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// What the target names.
    #[must_use]
    pub const fn kind(&self) -> &Kind {
        &self.kind
    }

    /// Whether the target is PROVEN absent: its session off a COMPLETE
    /// `sessions` inventory, or its seat off a COMPLETE roster, which
    /// `roster_of` answers for this session (`None`) or a named one. A `None`
    /// answer proves nothing.
    #[must_use]
    pub fn missing<'r>(
        &self,
        roster_of: impl Fn(Option<&str>) -> Option<&'r [String]>,
        sessions: Option<&[&str]>,
    ) -> bool {
        let off = |names: &[String], agent: &str| !names.iter().any(|name| name == agent);
        let gone = |session: &str| sessions.is_some_and(|known| !known.contains(&session));
        match (&self.kind, self.session_elsewhere()) {
            (Kind::Seat(agent), _) => roster_of(None).is_some_and(|names| off(names, agent)),
            (Kind::Remote { session, agent }, _) => {
                gone(session) || roster_of(Some(session)).is_some_and(|names| off(names, agent))
            }
            (_, Some(session)) => gone(session),
            (_, None) => false,
        }
    }

    /// The seat's words: the target as declared, `(not found)` when
    /// [`Self::missing`] proved it.
    #[must_use]
    pub fn words(&self, missing: bool) -> String {
        let gap = if missing { " (not found)" } else { "" };
        format!("waiting on {}{gap}", self.label)
    }

    /// The other session this target names, if it names one.
    #[must_use]
    pub fn session_elsewhere(&self) -> Option<&str> {
        match &self.kind {
            Kind::Session(session) if !self.own => Some(session),
            Kind::Remote { session, .. } => Some(session),
            Kind::Seat(_) | Kind::Session(_) => None,
        }
    }
}

/// One seat as the fold reads it.
#[derive(Debug, Clone, Copy)]
pub struct Link<'a> {
    /// The seat's name, the one a same-session target names.
    pub name: &'a str,
    /// Its declared target, when its current wait is typed.
    pub on: Option<&'a Target>,
    /// Its CURRENT declaration is `waiting-agent` and it is live.
    pub waiting: bool,
    /// It is working.
    pub working: bool,
}

/// Where a wait chain ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// In another session, entered from `via`, the last seat of this one.
    External { session: String, via: String },
    /// In one other session, entered from several seats of this one: only
    /// [`session_end`] answers it.
    Session { session: String },
    /// At a seat that is working.
    Working,
    /// Inside this session: untyped, cyclic, a missing or idle seat, or this
    /// session named as a whole.
    InSession,
}

impl End {
    /// The words a surface draws for this end.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::External { session, via } => format!("waiting on @{session} (via {via})"),
            Self::Session { session } => format!("waiting on @{session}"),
            Self::Working => "working".to_owned(),
            Self::InSession => "waiting (in session)".to_owned(),
        }
    }
}

/// Where `seat`'s wait chain ends; `None` unless `seat` is waiting.
#[must_use]
pub fn end_from(links: &[Link<'_>], seat: &str) -> Option<End> {
    let mut at = links
        .iter()
        .find(|link| link.name == seat && link.waiting)?;
    let mut visited = vec![at.name];
    loop {
        let Some(target) = at.on else {
            return Some(End::InSession);
        };
        if let Some(session) = target.session_elsewhere() {
            return Some(End::External {
                session: session.to_owned(),
                via: at.name.to_owned(),
            });
        }
        let Kind::Seat(next) = target.kind() else {
            return Some(End::InSession);
        };
        let Some(link) = links.iter().find(|link| link.name == next) else {
            return Some(End::InSession);
        };
        if visited.contains(&link.name) {
            return Some(End::InSession);
        }
        if link.working {
            return Some(End::Working);
        }
        if !link.waiting {
            return Some(End::InSession);
        }
        visited.push(link.name);
        at = link;
    }
}

/// Where the session's waits end, `None` when no seat waits: a working end
/// wins; when every waiting seat ends in ONE other session, that session,
/// through their one `via` when they share it; anything else stays here.
#[must_use]
pub fn session_end(links: &[Link<'_>]) -> Option<End> {
    let ends: Vec<End> = links
        .iter()
        .filter_map(|link| end_from(links, link.name))
        .collect();
    let first = ends.first()?;
    if ends.contains(&End::Working) {
        return Some(End::Working);
    }
    let End::External { session, .. } = first else {
        return Some(End::InSession);
    };
    if ends.iter().all(|end| end == first) {
        return Some(first.clone());
    }
    let one = |end: &End| matches!(end, End::External { session: other, .. } if other == session);
    Some(if ends.iter().all(one) {
        End::Session {
            session: session.clone(),
        }
    } else {
        End::InSession
    })
}

/// Whether `seat`'s wait chain ends in another session — the ceiling
/// exemption's one question.
#[must_use]
pub fn ends_elsewhere(links: &[Link<'_>], seat: &str) -> bool {
    matches!(end_from(links, seat), Some(End::External { .. }))
}

/// One live seat's wait facts, owned, read from the journal the way its
/// state cell reads it — the one builder both the read side and the
/// watchdog fold from, so they cannot disagree about a chain.
#[derive(Debug, Clone)]
pub struct Facts {
    name: String,
    on: Option<Target>,
    waiting: bool,
    working: bool,
}

impl Facts {
    /// Each live `(slot, reference, name)` seat of `session`, from its
    /// declaration in `events`: waiting only while its CURRENT declaration
    /// is `waiting-agent`, typed only when that declaration carries a target.
    #[must_use]
    pub fn read(
        events: &[crate::events::Event],
        session: &str,
        seats: &[(&str, &str, &str)],
    ) -> Vec<Self> {
        seats
            .iter()
            .map(|(slot, reference, name)| {
                let declaration = crate::session::declaration_of(events, session, slot, reference);
                let state = declaration.and_then(|(event, _)| event.declared_state());
                let current = declaration.is_some_and(|(_, current)| current);
                let waiting = current && state == Some("waiting-agent");
                Self {
                    name: (*name).to_owned(),
                    on: declaration
                        .filter(|_| waiting)
                        .and_then(|(event, _)| event.wait_on.as_deref())
                        .and_then(|raw| Target::parse(raw, session)),
                    waiting,
                    working: state == Some("working"),
                }
            })
            .collect()
    }

    /// The seat's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The seat's declared target, when its current wait is typed.
    #[must_use]
    pub const fn on(&self) -> Option<&Target> {
        self.on.as_ref()
    }

    /// The fold's view of these facts.
    #[must_use]
    pub fn links(facts: &[Self]) -> Vec<Link<'_>> {
        facts
            .iter()
            .map(|fact| Link {
                name: &fact.name,
                on: fact.on.as_ref(),
                waiting: fact.waiting,
                working: fact.working,
            })
            .collect()
    }
}
