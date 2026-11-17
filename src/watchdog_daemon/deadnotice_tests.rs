//! The pure rules of the spawner's death notice (#211): whom, which episode,
//! what a try leaves. The tmux-backed behaviour is `tests/spec/deadnotify.rs`
//! and `tests/spec/deadnotify_impl.rs`.

use super::{
    ACTION, BACK_ACTION, Death, Descendancy, Event, Notice, QuotaDelivery, RosterEntry, Standing,
    TRIES, after, after_reading, alive, episode, recipient, spent, text,
};
use crate::meta::Meta;

const SESSION: &str = "current";
const SEAT: (&str, &str) = ("spawned.0", "worker");

fn event(actor: &str, action: &str, target: &str, reference: &str) -> Event {
    let target = if target.is_empty() {
        String::new()
    } else {
        format!(r#","target":"{target}""#)
    };
    let reference = if reference.is_empty() {
        String::new()
    } else {
        format!(r#","ref":"{reference}""#)
    };
    Event::parse_line(&format!(
        r#"{{"ts":"2026-11-15T00:00:00Z","actor":"{actor}","action":"{action}"{target}{reference}}}"#
    ))
    .expect("a fixture line")
}

/// A record the seat wrote from its own pane: it carries the actor's routing key.
fn own(actor: &str, action: &str, slot: &str) -> Event {
    Event::parse_line(&format!(
        r#"{{"ts":"2026-11-15T00:00:00Z","actor":"{actor}","action":"{action}","actor_slot":"{slot}","actor_session":"{SESSION}"}}"#
    ))
    .expect("a fixture line")
}

fn told(events: &[Event], seat: (&str, &str)) -> bool {
    episode(events, SESSION, seat).told
}

fn notice(about: &str) -> Event {
    event("watchdog", ACTION, "parent", about)
}

fn roster() -> Vec<RosterEntry> {
    Meta::parse(
        "session=current\nseat.spawned.0=worker\nseat.spawned.1=parent\n\
         agent_bin.spawned.0=sleep\nagent_bin.spawned.1=sleep\n",
    )
    .roster()
    .to_vec()
}

#[test]
fn the_notice_names_the_seat_and_the_observed_state() {
    assert_eq!(
        text("worker", Death::Shell),
        "worker worker is DEAD — process dropped to shell"
    );
    assert_eq!(
        text("api", Death::PaneGone),
        "worker api is DEAD — pane gone"
    );
}

#[test]
fn only_a_roster_seat_other_than_the_worker_is_told() {
    let roster = roster();
    let spawned = |by: &str| vec![event(by, "spawn", "worker", "")];
    assert_eq!(
        recipient(&spawned("parent"), &roster, SESSION, SEAT),
        Some("parent")
    );
    for nobody in ["human", "retired", "worker"] {
        assert_eq!(
            recipient(&spawned(nobody), &roster, SESSION, SEAT),
            None,
            "{nobody} is nobody"
        );
    }
    assert_eq!(recipient(&[], &roster, SESSION, SEAT), None, "no spawn");
    let newest_human = [
        event("parent", "spawn", "worker", ""),
        event("human", "spawn", "worker", ""),
    ];
    assert_eq!(recipient(&newest_human, &roster, SESSION, SEAT), None);
    let newest_parent = [
        event("human", "spawn", "worker", ""),
        event("parent", "spawn", "worker", ""),
    ];
    assert_eq!(
        recipient(&newest_parent, &roster, SESSION, SEAT),
        Some("parent")
    );
}

#[test]
fn an_episode_stays_told_until_the_seat_lives_or_is_reborn() {
    assert!(!told(&[], SEAT), "no record, not told");
    assert!(told(&[notice("worker")], SEAT));
    assert!(
        !told(&[notice("api")], SEAT),
        "another seat's notice is not this seat's"
    );
    assert!(
        !told(&[event("watchdog", "alert", "worker", "worker")], SEAT),
        "a human alert about the seat is no notice"
    );
    let after = |closer: Event| [notice("worker"), closer];
    for (what, closer) in [
        ("its own record", own("worker", "state", "spawned.0")),
        (
            "its own record from before routing keys",
            event("worker", "state", "", "done"),
        ),
        (
            "a retraction of the alert",
            event("watchdog", "dead-cleared", "worker", ""),
        ),
        (
            "the pane returning",
            event("watchdog", BACK_ACTION, "worker", ""),
        ),
        ("a new spawn", event("parent", "spawn", "worker", "")),
        (
            "a reseat",
            event("watchdog", crate::reseat::RESEAT_ACTION, "worker", ""),
        ),
    ] {
        assert!(!told(&after(closer), SEAT), "{what} ends it");
    }
    for (what, other) in [
        ("an ask to it", event("parent", "ask", "worker", "")),
        (
            "a retraction for another seat",
            event("watchdog", "dead-cleared", "api", ""),
        ),
        (
            "another seat's return",
            event("watchdog", BACK_ACTION, "api", ""),
        ),
        (
            "another seat's own record",
            own("parent", "state", "spawned.1"),
        ),
    ] {
        assert!(told(&after(other), SEAT), "{what} ends nothing");
    }
    let again = [
        notice("worker"),
        event("watchdog", BACK_ACTION, "worker", ""),
        notice("worker"),
    ];
    assert!(told(&again, SEAT), "the newest notice decides");
}

#[test]
fn a_legacy_seat_named_watchdog_reads_no_keyless_record_as_its_own_life() {
    // `watchdog` is reserved at creation only: an older recorded seat of that
    // name stays valid. Everything keyless that names `watchdog` is the
    // daemon's or a sender speaking for it, whatever its action, so none of it
    // ends the episode the seat's notice opened, a refusal of that very notice
    // included. This is a named residual: such a seat's own keyless records
    // from before routing keys end nothing either.
    let seat = ("spawned.0", "watchdog");
    let told_by = [notice("watchdog")];
    assert!(told(&told_by, seat));
    for later in [
        event("watchdog", "nudge", "api", ""),
        event("watchdog", "alert", "watchdog", "watchdog"),
        event("watchdog", "limit", "watchdog", ""),
        event("watchdog", "delivery-abandoned", "parent", "watchdog"),
        event("watchdog", "state", "", "working"),
        notice("watchdog"),
    ] {
        let events = [notice("watchdog"), later];
        assert_eq!(
            episode(&events, SESSION, seat),
            episode(&told_by, SESSION, seat),
            "a keyless record under a reserved name moves neither key nor told"
        );
    }
    // What does end it: the daemon's addressed retraction or return, and the
    // seat's own keyed record.
    for closer in [
        event("watchdog", BACK_ACTION, "watchdog", ""),
        event("watchdog", "dead-cleared", "watchdog", ""),
        own("watchdog", "state", "spawned.0"),
    ] {
        let events = [notice("watchdog"), closer];
        assert!(!told(&events, seat));
        assert_eq!(episode(&events, SESSION, seat).key, 1);
    }
}

#[test]
fn only_a_positive_reading_says_the_seat_lives() {
    assert!(alive(false, "sleep", Descendancy::Absent), "no shell");
    assert!(alive(false, "sh", Descendancy::Present), "harness under it");
    assert!(!alive(false, "sh", Descendancy::Absent), "a bare shell");
    assert!(!alive(false, "sh", Descendancy::Unknown), "a probe gap");
    assert!(!alive(true, "sleep", Descendancy::Present), "latched dead");
}

#[test]
fn one_notice_is_tried_three_times_and_only_a_proven_refusal_is_retried() {
    assert_eq!(
        after(QuotaDelivery::Delivered, 1),
        (Notice::Settled, None),
        "a delivery ends it"
    );
    assert_eq!(
        after(QuotaDelivery::Retryable, 0),
        (Notice::Owed { attempts: 1 }, None)
    );
    assert_eq!(
        after(QuotaDelivery::Retryable, 1),
        (Notice::Owed { attempts: 2 }, None)
    );
    let (settled, said) = after(QuotaDelivery::Retryable, 2);
    assert_eq!(settled, Notice::Settled, "the third refusal is the last");
    assert!(said.is_some(), "and it is said");
    for spent in 0..TRIES {
        let (settled, said) = after(QuotaDelivery::Uncertain, spent);
        assert_eq!(
            settled,
            Notice::Settled,
            "an uncertain try is never repeated"
        );
        assert!(said.is_some());
    }
}

#[test]
fn the_tries_spent_are_what_the_standing_counts() {
    assert_eq!(spent(Notice::Owed { attempts: 2 }), 2);
    assert_eq!(spent(Notice::Idle), 0);
    assert_eq!(spent(Notice::Settled), 0);
}

const IDLE: Standing = Standing {
    notice: Notice::Idle,
    key: 0,
};

const fn standing(notice: Notice, key: usize) -> Standing {
    Standing { notice, key }
}

#[test]
fn a_dead_reading_arms_an_idle_standing_and_keeps_every_other() {
    // Latched dead on a shell leg or a pane-gone leg alike: ONE standing, so a
    // try the helper could not prove refused is not given a fresh budget.
    let dead = (false, true, false);
    assert_eq!(
        after_reading(IDLE, dead, 3),
        standing(Notice::Owed { attempts: 0 }, 3)
    );
    for held in [
        standing(Notice::Owed { attempts: 2 }, 3),
        standing(Notice::Settled, 3),
    ] {
        assert_eq!(after_reading(held, dead, 3), held);
        assert_eq!(after_reading(held, (true, true, false), 3), held);
    }
}

#[test]
fn only_the_latch_clearing_or_a_positive_reading_ends_the_standing() {
    for held in [
        IDLE,
        standing(Notice::Owed { attempts: 1 }, 2),
        standing(Notice::Settled, 2),
    ] {
        assert_eq!(
            after_reading(held, (true, false, false), 2),
            IDLE,
            "the daemon's own clear of the latch: {held:?}"
        );
        assert_eq!(
            after_reading(held, (false, false, true), 2),
            IDLE,
            "a positive reading that the seat lives: {held:?}"
        );
        assert_eq!(
            after_reading(held, (false, false, false), 2),
            held,
            "a launch grace or a probe gap is no sign of life: {held:?}"
        );
    }
}

#[test]
fn a_closer_since_the_standing_was_armed_starts_a_new_episode_and_an_old_one_never_does() {
    let dead = (true, true, false);
    for held in [
        standing(Notice::Owed { attempts: 2 }, 4),
        standing(Notice::Settled, 4),
    ] {
        assert_eq!(
            after_reading(held, dead, 5),
            standing(Notice::Owed { attempts: 0 }, 5),
            "a closer landed: a fresh budget, armed at the new key"
        );
        assert_eq!(after_reading(held, dead, 4), held, "no closer since");
    }
    // A closer that landed before the seat died is part of the key the
    // standing was armed at: the cycles after it keep the budget they spent.
    let armed = after_reading(IDLE, (false, true, false), 7);
    let spent_once = armed.then(Notice::Owed { attempts: 1 });
    assert_eq!(after_reading(spent_once, dead, 7), spent_once);
}

#[test]
fn the_key_counts_every_closer_the_seat_has_and_nothing_else() {
    let key = |events: &[Event]| episode(events, SESSION, SEAT).key;
    assert_eq!(key(&[]), 0);
    // Not closers: the notice itself, a human alert about the seat, an ask to
    // it, and another seat's records.
    assert_eq!(
        key(&[
            notice("worker"),
            event("watchdog", "alert", "worker", "worker"),
            event("parent", "ask", "worker", ""),
            event("watchdog", "dead-cleared", "api", ""),
            own("parent", "state", "spawned.1"),
            event("parent", "state", "", "working"),
        ]),
        0
    );
    // Closers: its own record, keyed or from before routing keys, and a
    // retraction, return, spawn or reseat addressed to it.
    let closers = [
        own("worker", "state", "spawned.0"),
        event("worker", "state", "", "working"),
        event("watchdog", "dead-cleared", "worker", ""),
        event("watchdog", BACK_ACTION, "worker", ""),
        event("parent", "spawn", "worker", ""),
        event("human", crate::reseat::RESEAT_ACTION, "worker", ""),
    ];
    for count in 0..=closers.len() {
        assert_eq!(key(&closers[..count]), count);
    }
}

#[test]
fn a_notice_belongs_to_the_episode_it_follows() {
    let notice_after = [
        own("worker", "state", "spawned.0"),
        notice("worker"),
        event("parent", "ask", "worker", ""),
    ];
    assert_eq!(
        episode(&notice_after, SESSION, SEAT),
        super::Episode { key: 1, told: true }
    );
    let closed = [
        notice("worker"),
        event("watchdog", "dead-cleared", "worker", ""),
    ];
    assert_eq!(
        episode(&closed, SESSION, SEAT),
        super::Episode {
            key: 1,
            told: false
        }
    );
}
