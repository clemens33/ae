//! The composer address, suspicious-word rule and bound redirect destination.

use std::path::{Path, PathBuf};

use super::{Console, lane::Seat, term};
use crate::{archive, inventory::ServerId, session_menu, tmux};

/// The session this composer addresses and whether it holds the orchestrator role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An ordinary session.
    Session(String),
    /// A session with the orchestrator role.
    Orchestrator(String),
}

impl Target {
    pub(crate) fn of(name: &str, orchestrator: bool) -> Self {
        if orchestrator {
            Self::Orchestrator(name.to_owned())
        } else {
            Self::Session(name.to_owned())
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Session(name) | Self::Orchestrator(name) => name,
        }
    }

    pub(crate) fn orchestrator(&self) -> bool {
        matches!(self, Self::Orchestrator(_))
    }
}

/// A plain, terminal-safe composer address. The role marker is visible in mono.
#[must_use]
pub fn label(target: &Target) -> String {
    let name = crate::board::terminal_text(target.name()).replace(['\r', '\n', '\t'], " ");
    if target.orchestrator() {
        format!("[orchestrator] to {name}")
    } else {
        format!("to {name}")
    }
}

/// The first token that would prompt a routing decision on this ordinary draft.
pub(super) fn suspect<'a>(
    text: &'a str,
    target: &Target,
    running: Option<&[String]>,
) -> Option<&'a str> {
    let Target::Session(current) = target else {
        return None;
    };
    let boundary = |c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-'));
    text.split(boundary).find(|token| {
        crate::desk::parse_id(token).is_some()
            || (token.chars().take(3).count() == 3
                && token != current
                && running.is_some_and(|names| names.iter().any(|name| name == token)))
    })
}

pub(super) fn warning(token: &str, target: &Target, redirect: bool) -> String {
    let target = label(target);
    let reroute = if redirect {
        " · Alt+o sends to orchestrator"
    } else {
        ""
    };
    format!("Check {token} · Enter sends {target} · Esc keeps editing{reroute}")
}

/// A running role seat as the warning knew it: its record, server, UUID and pair.
#[derive(Debug, Clone)]
pub(crate) struct RoleRoute {
    pub(crate) name: String,
    pub(crate) uuid: String,
    root: PathBuf,
    dir: PathBuf,
    server: ServerId,
    pair: Vec<Seat>,
}

impl RoleRoute {
    pub(crate) fn bind(
        root: &Path,
        name: &str,
        dir: &Path,
        proof: (String, ServerId),
        pair: Vec<Seat>,
    ) -> Option<Self> {
        let (uuid, server) = proof;
        let pair = term::pair_of(pair).ok()?;
        Some(Self {
            name: name.to_owned(),
            uuid,
            root: root.to_owned(),
            dir: dir.to_owned(),
            server,
            pair,
        })
    }

    pub(crate) fn console(&self) -> Console {
        Console::bound(self.name.clone(), self.dir.clone(), self.uuid.clone())
    }

    pub(crate) fn seat(&self) -> &str {
        self.pair.first().map_or("", |seat| seat.name.as_str())
    }

    pub(crate) fn pin(&self) -> crate::tracked::RolePin<'_> {
        crate::tracked::RolePin {
            name: &self.name,
            root: &self.root,
            server: &self.server,
            uuid: &self.uuid,
        }
    }

    /// The same role, incarnation, server and pair must still own the route.
    pub(crate) fn verify(&self, console: &Console) -> Result<(), String> {
        let holder = crate::settings_menu::role_holder(&self.root)?;
        if holder.as_deref() != Some(self.name.as_str()) {
            return Err("the orchestrator role changed".to_owned());
        }
        let now = prove_role(&self.root, &self.name, &self.dir)?;
        if now.0 != self.uuid || now.1 != self.server {
            return Err("the orchestrator session was replaced or moved".to_owned());
        }
        term::still(&self.pair, console.seats())
    }
}

/// The current recorded server and live UUID of a role holder. The census
/// separately proves the holder; prove its record still carries the overlay.
pub(crate) fn prove_role(
    root: &Path,
    name: &str,
    dir: &Path,
) -> Result<(String, ServerId), String> {
    if !crate::meta::read_bytes(dir).is_ok_and(|meta| crate::meta::is_seat(&meta, root)) {
        return Err("meta: local_config is not the seat overlay".to_owned());
    }
    let server = crate::session_launch::recorded_server_resolved(dir);
    let option = server
        .as_ref()
        .map_or(tmux::OptionReading::Unknown, |server| {
            session_menu::live_identity(server, name)
        });
    let meta = session_menu::MetaSource::read(dir);
    if let Some(gap) = session_menu::correlation_gap(&option, &meta) {
        return Err(gap);
    }
    let live = match option {
        tmux::OptionReading::Set(value) => archive::canonical_uuid(&value),
        _ => String::new(),
    };
    let server = server.ok_or("meta: no recorded orchestrator server")?;
    match sole_uuid(dir) {
        Some(sole) if sole == live => Ok((live, server)),
        _ => Err("meta: session_id not recorded once".to_owned()),
    }
}

/// The canonical `session_id` the raw meta names exactly once.
pub(crate) fn sole_uuid(dir: &Path) -> Option<String> {
    let bytes = crate::meta::read_bytes(dir).ok()?;
    let one = crate::meta::sole_value(&bytes, "session_id")?;
    let one = archive::canonical_uuid(&String::from_utf8_lossy(one));
    (!one.is_empty()).then_some(one)
}
