//! One fleet read's owner: what each tmux server answered, asked once.

use crate::{inventory, meta};

/// The socket path each server answers with, asked once per server.
///
/// One read per DISTINCT server, however many sessions name it: a fleet on one
/// server is one question, not one per row.
pub(crate) struct SocketPaths {
    /// Who is asked. A parameter so the collision the cache exists to catch can
    /// be proven without two real servers.
    resolve: fn(&inventory::ServerId) -> Option<String>,
    seen: Vec<(inventory::ServerId, Option<String>)>,
}

impl SocketPaths {
    /// A cache that asks `resolve`.
    pub(crate) const fn asking(resolve: fn(&inventory::ServerId) -> Option<String>) -> Self {
        Self {
            resolve,
            seen: Vec::new(),
        }
    }

    /// A cache already knowing what the CALLER server answered its own batched
    /// read, so the picker's stopped scan does not ask the same server twice.
    pub(crate) fn primed(
        resolve: fn(&inventory::ServerId) -> Option<String>,
        caller: &inventory::ServerId,
        socket: Option<String>,
    ) -> Self {
        Self {
            resolve,
            seen: vec![(caller.clone(), socket)],
        }
    }

    /// What `server` calls its own socket, or `None` when it did not answer.
    fn of(&mut self, server: &inventory::ServerId) -> Option<String> {
        if let Some((_, path)) = self.seen.iter().find(|(known, _)| known == server) {
            return path.clone();
        }
        let path = (self.resolve)(server);
        self.seen.push((server.clone(), path.clone()));
        path
    }

    /// Whether both server spellings answered with the same socket path.
    /// Unknown is never equal, even when the spellings themselves match.
    pub(crate) fn proven_same(
        &mut self,
        left: &inventory::ServerId,
        right: &inventory::ServerId,
    ) -> bool {
        let left = self.of(left);
        left.is_some() && self.of(right) == left
    }

    /// Reuse this fleet read's proof; an uncached app socket is already held
    /// from tmux's own marker. A cached failure always wins over that spelling.
    pub(crate) fn known_same(
        &self,
        recorded: &inventory::ServerId,
        app: &inventory::ServerId,
    ) -> bool {
        let cached = |server| self.seen.iter().find(|(known, _)| known == server);
        let Some((_, Some(path))) = cached(recorded) else {
            return false;
        };
        match cached(app) {
            Some((_, answer)) => answer.as_ref() == Some(path),
            None => matches!(app, inventory::ServerId::Selected(meta::Selector::Socket(socket))
                if socket.as_os_str().to_str() == Some(path.as_str())),
        }
    }

    /// Keep the first spelling of each server whose socket identity tmux can
    /// prove. Recorded pointers precede the default named entitlement, so a
    /// durable record retains its own spelling when both name one server.
    pub(crate) fn deduplicated(
        &mut self,
        servers: Vec<inventory::ServerId>,
    ) -> Vec<inventory::ServerId> {
        let mut paths: Vec<String> = Vec::new();
        let mut distinct = Vec::new();
        for server in servers {
            let path = self.of(&server);
            if path.as_ref().is_some_and(|path| paths.contains(path)) {
                continue;
            }
            if let Some(path) = path {
                paths.push(path);
            }
            distinct.push(server);
        }
        distinct
    }

    /// Whether two spellings were proven to name one socket while building a
    /// de-duplicated entitlement set.
    pub(crate) fn equivalent(
        &self,
        left: &inventory::ServerId,
        right: &inventory::ServerId,
    ) -> bool {
        if left == right {
            return true;
        }
        let known = |server: &inventory::ServerId| {
            self.seen
                .iter()
                .find(|(candidate, _)| candidate == server)
                .and_then(|(_, path)| path.as_deref())
        };
        known(left).is_some() && known(left) == known(right)
    }
}

#[cfg(test)]
mod socket_spec;

#[cfg(test)]
mod tests {
    #[test]
    fn live_seat_socket_proof_is_cache_only_and_unknown_never_matches() {
        use crate::{inventory::ServerId, meta::Selector};
        let name = |s: &str| ServerId::Selected(Selector::Name(s.into()));
        let socket = |s: &str| ServerId::Selected(Selector::Socket(s.into()));
        let (recorded, alias) = (name("ae"), name("alias"));
        let no_read = |_: &ServerId| -> Option<String> {
            panic!("new socket read");
        };
        let mut cache = super::SocketPaths::primed(no_read, &recorded, Some("/tmp/ae".into()));
        cache.seen.push((alias.clone(), Some("/tmp/ae".into())));
        assert!(cache.known_same(&recorded, &alias));
        assert!(cache.known_same(&recorded, &socket("/tmp/ae")));
        assert!(!cache.known_same(&recorded, &socket("/private/tmp/ae")));
        assert!(!cache.known_same(&recorded, &socket("/tmp/other")));
        assert!(!cache.known_same(&name("uncached"), &name("uncached")));
        assert!(!cache.known_same(&socket("/tmp/new"), &socket("/tmp/new")));
        cache
            .seen
            .push((socket("/tmp/ae"), Some("/tmp/elsewhere".into())));
        assert!(!cache.known_same(&recorded, &socket("/tmp/ae")));
        cache.seen.push((name("failed"), None));
        assert!(!cache.known_same(&recorded, &name("failed")));
        assert!(!cache.known_same(&name("failed"), &alias));
        cache.seen.push((socket("/tmp/missing"), None));
        assert!(!cache.known_same(&recorded, &socket("/tmp/missing")));
    }

    fn aliased_socket_of(server: &crate::inventory::ServerId) -> Option<String> {
        match server {
            crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name))
                if name == crate::doors::DEFAULT_SERVER_NAME =>
            {
                Some("/private/tmux/ae".to_owned())
            }
            crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(path)) => {
                Some(path.display().to_string())
            }
            _ => None,
        }
    }

    fn named(name: &str) -> crate::inventory::ServerId {
        crate::inventory::ServerId::Selected(crate::meta::Selector::Name(name.to_owned()))
    }

    #[test]
    fn a_recorded_socket_suppresses_the_default_name_only_when_tmux_proves_the_alias() {
        let recorded = crate::inventory::ServerId::Selected(crate::meta::Selector::Socket(
            "/private/tmux/ae".into(),
        ));
        let default = named(crate::doors::DEFAULT_SERVER_NAME);
        let unresolved = crate::inventory::ServerId::Ambient;
        let mut sockets = super::SocketPaths::asking(aliased_socket_of);
        let candidates = vec![recorded.clone(), default.clone(), unresolved.clone()];
        assert_eq!(
            sockets.deduplicated(candidates),
            [recorded.clone(), unresolved],
            "the durable spelling wins; an unproved server is never discarded"
        );
        assert!(sockets.equivalent(&recorded, &default));
    }
}
