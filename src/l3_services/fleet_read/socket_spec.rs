//! Frozen R4 socket identity contract; authored against base 43c76c5d.
//! The parent supplies `SocketPaths`. Move this registration with that owner.

use super::SocketPaths;
use crate::{inventory::ServerId, meta::Selector};
use std::cell::RefCell;

thread_local! {
    static CALLS: RefCell<Vec<ServerId>> = const { RefCell::new(Vec::new()) };
}

fn named(value: &str) -> ServerId {
    ServerId::Selected(Selector::Name(value.into()))
}

fn socket(value: &str) -> ServerId {
    ServerId::Selected(Selector::Socket(value.into()))
}

fn resolve(server: &ServerId) -> Option<String> {
    CALLS.with(|calls| calls.borrow_mut().push(server.clone()));
    if *server == named("failed") || *server == socket("/r4/live") {
        None
    } else {
        Some("/r4/live".into())
    }
}

#[test]
fn r4_socket_aliases_preserve_first_spelling_and_first_read_order() {
    // Mutation: skip deduplication, forget of()'s cache, or retain last alias.
    CALLS.with(|calls| calls.borrow_mut().clear());
    let (first, alias, failed) = (named("first"), named("alias"), named("failed"));
    let mut cache = SocketPaths::asking(resolve);
    assert_eq!(
        cache.deduplicated(vec![
            first.clone(),
            alias.clone(),
            first.clone(),
            failed.clone()
        ]),
        [first.clone(), failed.clone()]
    );
    assert_eq!(
        cache.deduplicated(vec![alias.clone(), failed.clone()]),
        [alias.clone(), failed.clone()]
    );
    CALLS.with(|calls| assert_eq!(*calls.borrow(), [first, alias, failed]));
}

#[test]
fn r4_socket_three_verbs_keep_unknown_failure_and_entitlement_distinct() {
    // Mutations: proven_same uses selector equality; known_same trusts a
    // failed cached socket spelling; equivalent demands positive proof even
    // for identical entitlement selectors; failure is retried into success.
    CALLS.with(|calls| calls.borrow_mut().clear());
    let (recorded, failed, app) = (named("recorded"), named("failed"), socket("/r4/live"));
    let mut cache = SocketPaths::primed(resolve, &recorded, Some("/r4/live".into()));
    assert!(
        cache.known_same(&recorded, &app),
        "uncached held socket spelling"
    );
    assert!(!cache.proven_same(&failed, &failed), "unknown is not proof");
    assert!(
        cache.equivalent(&failed, &failed),
        "identical entitlement selectors"
    );
    assert!(
        !cache.known_same(&failed, &failed),
        "cached failure is not proof"
    );
    assert!(!cache.proven_same(&recorded, &app), "socket query fails");
    assert!(
        !cache.known_same(&recorded, &app),
        "cached failure beats spelling"
    );
    assert!(
        !cache.equivalent(&recorded, &app),
        "different selectors lack proof"
    );
    assert!(
        !cache.proven_same(&recorded, &app),
        "failed socket stays cached"
    );
    CALLS.with(|calls| assert_eq!(*calls.borrow(), [failed, app]));
}
