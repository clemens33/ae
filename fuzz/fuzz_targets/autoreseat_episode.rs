#![no_main]

use std::sync::{Once, OnceLock};

use ae::autoreseat::{
    ATTEMPT_ACTION, DONE_ACTION, Episode, FAILED_ACTION, HEADROOM_ACTION, HELD_ACTION, MAX_ROUNDS,
    Outcome, REARMED_ACTION, REFUSED_ACTION, RETRY_COOLDOWN_SECS, Trigger, episode,
    headroom_episode, keyed,
};
use ae::events::{Event, RoutingMember};
use ae::time::Timestamp;
use libfuzzer_sys::fuzz_target;

const SESSION: &str = "seat-session";
const SLOT: &str = "spawned.0";
const AGENT: &str = "scout";
const WATCHDOG: &str = "watchdog";
const OPEN: &str = "limit";
const CLEARED: &str = "alert-cleared";
const SPAWN: &str = "spawn";
const CHOICE: &str = "observed-choice: ";
const T0: i64 = 1_700_000_000;
const KEY: &str = "2023-11-14T22:13:20Z";
// Space-separated tables: the record kinds, and what the noise draws from.
const ACTIONS: &str = "auto-reseat auto-reseat-held auto-reseat-done auto-reseat-refused auto-reseat-failed auto-reseat-headroom auto-reseat-headroom-cleared limit alert-cleared spawn";
const GUARDED: &str = "auto-reseat auto-reseat-held auto-reseat-refused auto-reseat-failed auto-reseat-headroom-cleared";
const WRONG_REFS: &str = "x-foreign 1700000000 2023-11-14T22:13:20+00:00";
const ACTORS: &str = "lead human Watchdog claude:watchdog";
const QUIET: &str = "nudge state Limit auto-reseat-notice";
// "target|slot|session|session id", empty = absent. The seats name this seat
// in five legal spellings; the strangers name nobody here.
const SEATS: [&str; 5] = [
    "scout|spawned.0|seat-session|",
    "scout|spawned.0|seat-session|11111111-1111-4111-8111-111111111111",
    "scout|||",
    "@seat-session:scout|||",
    "renamed|spawned.0|seat-session|",
];
const STRANGERS: [&str; 8] = [
    "scout|spawned.9|seat-session|",
    "scout|spawned.0|other-session|",
    "other|||",
    "@other-session:scout|||",
    "@seat-session:other|||",
    "scout|spawned.0|seat-session|bad-id",
    "scout|spawned.0||",
    "|||",
];

type Folds = (Option<Episode>, Option<Episode>);

fn rec(ts: i64, actor: &str, action: &str, r: Option<&str>, say: Option<&str>, to: &str) -> Event {
    static BARE: OnceLock<Event> = OnceLock::new();
    let line = r#"{"ts":"2023-11-14T22:13:20Z","actor":"x","action":"x"}"#;
    let mut event = BARE
        .get_or_init(|| Event::parse_line(line).expect("a bare record parses"))
        .clone();
    let mut part = to.split('|').map(|s| (!s.is_empty()).then(|| s.to_owned()));
    event.target = part.next().flatten();
    let mut member = || {
        part.next()
            .flatten()
            .map_or(RoutingMember::Absent, RoutingMember::Value)
    };
    event.target_slot = member();
    event.target_session = member();
    event.target_session_id = member();
    event.ts = Timestamp::from_epoch(ts);
    event.actor = actor.to_owned();
    event.action = action.to_owned();
    event.reference = r.map(str::to_owned);
    event.summary = say.map(str::to_owned);
    event
}

fn watch(ts: i64, action: &str, r: Option<&str>, say: Option<&str>) -> Event {
    rec(ts, WATCHDOG, action, r, say, SEATS[0])
}

fn folds(events: &[Event]) -> Folds {
    (
        episode(events, SESSION, SLOT, AGENT),
        headroom_episode(events, SESSION, SLOT, AGENT),
    )
}

fn append(events: &[Event], record: Event) -> Folds {
    let mut log = events.to_vec();
    log.push(record);
    folds(&log)
}

fn nth(table: &str, n: usize) -> &str {
    let items: Vec<&str> = table.split(' ').collect();
    items[n % items.len()]
}

fn mix(seed: usize, i: usize) -> usize {
    seed.wrapping_add(i).wrapping_mul(2_654_435_761) >> 7
}

// Everything an episode says must be traceable to a record of the journal.
fn sound(e: &Episode, events: &[Event]) {
    let r = e.reference();
    let limit = e.trigger == Trigger::Limit;
    let tally = |actions: &[&str], named: Option<&str>, keep: &dyn Fn(&Event) -> bool| {
        let ours = |x: &&Event| {
            x.actor == WATCHDOG
                && actions.contains(&x.action.as_str())
                && named.is_none_or(|n| x.reference.as_deref() == Some(n))
                && keep(*x)
        };
        events.iter().filter(ours).count()
    };
    let choice = |x: &Event| x.summary.as_deref().is_some_and(|s| s.starts_with(CHOICE));
    assert_eq!(r, e.key.to_string());
    let opener = if limit { OPEN } else { HEADROOM_ACTION };
    let opens = tally(&[opener], None, &|x| {
        x.ts == e.key && (limit || x.summary.as_deref().unwrap_or("") == e.cause())
    });
    assert!(opens > 0, "an episode opens on an authorized record");
    assert!(usize::from(e.attempts) <= tally(&[ATTEMPT_ACTION], Some(r), &|_| true));
    assert!(usize::from(e.rounds) <= tally(&[REFUSED_ACTION, FAILED_ACTION], None, &|_| true));
    assert!(if limit {
        e.rounds <= MAX_ROUNDS && e.cause().is_empty()
    } else {
        e.rounds <= 1
    });
    assert!(
        !(e.held && e.open.is_some()),
        "a hold is not an open attempt"
    );
    assert!(!e.held || tally(&[HELD_ACTION], Some(r), &|_| true) > 0);
    assert!(!e.choice_held || tally(&[HELD_ACTION], Some(r), &choice) > 0);
    let spent = matches!(e.terminal, Some(Outcome::Refused | Outcome::Failed));
    assert_eq!(e.gave_up(), limit && spent && e.rounds >= MAX_ROUNDS);
}

// Two episodes may share a key; the one in flight, then the limit one, is taken.
fn selection(events: &[Event], base: &Folds) {
    let found: Vec<&Episode> = [&base.0, &base.1].into_iter().flatten().collect();
    for probe in found.iter().map(|e| e.key) {
        let named: Vec<&Episode> = found.iter().copied().filter(|e| e.key == probe).collect();
        let flying = named
            .iter()
            .copied()
            .find(|e| e.open.is_some() && e.terminal.is_none());
        let got = keyed(events, (SESSION, SLOT, AGENT), probe);
        let want = flying.or_else(|| named.first().copied());
        assert_eq!(
            got.as_ref(),
            want,
            "keyed: in flight, then limit, then headroom"
        );
    }
}

// A bounded mix of records no authority backs: wherever they land, they change
// nothing. The input picks which ones, so the fuzzer steers the mix.
fn noise(live: &[String], at: i64, seed: usize) -> Vec<Event> {
    let live: Vec<Option<&str>> = live
        .iter()
        .map(|r| Some(r.as_str()))
        .chain([None])
        .collect();
    (0..16)
        .map(|i| {
            let n = mix(seed, i);
            let (action, r) = (nth(ACTIONS, n), live[(n / 10) % live.len()]);
            let wrong = (n % 7 != 0).then(|| nth(WRONG_REFS, n / 5));
            match (n / 100) % 4 {
                // Not the watchdog (a spawn from anyone is authorized, so not that).
                0 if action != SPAWN => rec(at, nth(ACTORS, n / 400), action, r, None, SEATS[0]),
                // Not this seat.
                0 | 1 => rec(at, WATCHDOG, action, r, None, STRANGERS[(n / 400) % 8]),
                // This seat, but a ref that is not the episode's key.
                2 => rec(
                    at,
                    WATCHDOG,
                    nth(GUARDED, n / 3),
                    wrong,
                    None,
                    SEATS[(n / 400) % 5],
                ),
                // This seat, but an action nothing reads.
                _ => rec(
                    at,
                    WATCHDOG,
                    nth(QUIET, n / 400),
                    r,
                    None,
                    SEATS[(n / 1600) % 5],
                ),
            }
        })
        .collect()
}

// Pair each episode of `base` with its successor in `after`: the one keyed `k`
// is judged by `named`; with `rest_same` every other one must be untouched.
fn judge(
    base: &Folds,
    after: &Folds,
    k: &str,
    rest_same: bool,
    named: impl Fn(&Episode, &Episode),
) {
    let was = [&base.0, &base.1].into_iter().flatten().count();
    let seen: Vec<(&Episode, &Episode)> = [(&base.0, &after.0), (&base.1, &after.1)]
        .into_iter()
        .filter_map(|(old, new)| Some((old.as_ref()?, new.as_ref()?)))
        .collect();
    assert_eq!(seen.len(), was, "no such record ends an episode");
    for (old, new) in seen {
        if old.reference() == k {
            named(old, new);
        } else if rest_same {
            assert_eq!(new, old);
        }
    }
}

// One record of each kind the watchdog addresses to this seat, each judged
// against the contract of its kind. The input picks the address spelling, the
// hold, the outcome and the done's ref; a dozen folds per input at most.
fn authorized(events: &[Event], base: &Folds, at: i64, seed: usize) {
    let to = SEATS[seed % 5];
    let with = |ts: i64, action: &str, r: Option<&str>, say: Option<&str>| {
        append(events, rec(ts, WATCHDOG, action, r, say, to))
    };
    // A spawn from anyone starts a new incarnation: nothing before it counts.
    let spawn = rec(
        at,
        nth("lead watchdog human", seed / 5),
        SPAWN,
        None,
        None,
        to,
    );
    assert_eq!(append(events, spawn.clone()), (None, None));
    let replay: Vec<Event> = [events, std::slice::from_ref(&spawn), events].concat();
    assert_eq!(folds(&replay), *base, "a new incarnation remembers nothing");
    // A limit opens an episode only when none is open, and leaves headroom be.
    let opened = with(at, OPEN, None, None);
    match &base.0 {
        Some(_) => assert_eq!(opened.0, base.0),
        None => {
            let e = opened.0.as_ref().expect("a limit record opens an episode");
            let state = (
                e.key.epoch(),
                e.attempts,
                e.rounds,
                e.open,
                e.terminal,
                e.held,
            );
            assert_eq!(state, (at, 0, 0, None, None, false));
        }
    }
    assert_eq!(opened.1, base.1);
    // A headroom record opens its own episode when none is open, quotes its
    // summary as the cause, and leaves the limit episode be.
    let why = "headroom 96% weekly";
    let armed = with(at, HEADROOM_ACTION, None, Some(why));
    match &base.1 {
        Some(_) => assert_eq!(armed.1, base.1),
        None => {
            let e = armed
                .1
                .as_ref()
                .expect("a headroom record opens an episode");
            let state = (e.trigger, e.key.epoch(), e.attempts, e.rounds);
            assert_eq!(state, (Trigger::Headroom, at, 0, 0));
            let rest = (e.open, e.terminal, e.held, e.choice_held);
            assert_eq!(rest, (None, None, false, false));
            assert_eq!(e.cause(), why);
        }
    }
    assert_eq!(armed.0, base.0);
    assert_eq!(with(at, CLEARED, None, None), (None, base.1.clone()));
    for old in [&base.0, &base.1].into_iter().flatten() {
        let k = old.reference();
        if old.trigger == Trigger::Headroom {
            assert_eq!(
                with(at, REARMED_ACTION, Some(k), None),
                (base.0.clone(), None)
            );
        }
        // An attempt inside the cooldown counts and leaves a closed round closed.
        let early = i64::MIN;
        judge(
            base,
            &with(early, ATTEMPT_ACTION, Some(k), None),
            k,
            true,
            |old, new| {
                let was = (
                    old.attempts.saturating_add(1),
                    Some(Timestamp::from_epoch(early)),
                    false,
                );
                assert_eq!((new.attempts, new.open, new.held), was);
                assert_eq!((new.rounds, new.terminal), (old.rounds, old.terminal));
            },
        );
        // A hold ends the open attempt, is quoted back, and a choice sticks.
        let say = ["busy: turn", "draft: box", "observed-choice: model stays"][(seed / 15) % 3];
        judge(
            base,
            &with(at, HELD_ACTION, Some(k), Some(say)),
            k,
            true,
            |old, new| {
                assert!(new.held && new.open.is_none() && new.holds_with(say));
                assert!(!new.holds_with("another hold"));
                assert_eq!(new.choice_held, old.choice_held || say.starts_with(CHOICE));
                assert_eq!((new.attempts, new.rounds), (old.attempts, old.rounds));
            },
        );
        // An outcome under a live key ends that round and counts it; a round
        // already closed keeps its first outcome. (It ends every open episode,
        // so the others are not judged.)
        let (action, outcome) = match (seed / 45) % 2 {
            0 => (REFUSED_ACTION, Outcome::Refused),
            _ => (FAILED_ACTION, Outcome::Failed),
        };
        judge(
            base,
            &with(early, action, Some(k), None),
            k,
            false,
            |old, new| {
                if old.terminal.is_some() {
                    return assert_eq!(new, old);
                }
                let ended = (new.terminal, new.rounds, new.open, new.held);
                assert_eq!(
                    ended,
                    (Some(outcome), old.rounds.saturating_add(1), None, false)
                );
            },
        );
    }
    // A done names the profile left, so its ref guards nothing: headroom is
    // dropped and the limit episode ends without counting a round.
    let named = base.0.as_ref().map(Episode::reference);
    let r = [None, Some("x-foreign"), named][(seed / 90) % 3];
    let done = with(at, DONE_ACTION, r, None);
    assert!(done.1.is_none());
    match (&base.0, &done.0) {
        (None, new) => assert!(new.is_none()),
        (Some(old), Some(new)) if old.terminal == Some(Outcome::Done) => assert_eq!(new, old),
        (Some(old), Some(new)) => {
            let ended = (new.terminal, new.rounds, new.attempts, new.open, new.held);
            assert_eq!(
                ended,
                (Some(Outcome::Done), old.rounds, old.attempts, None, false)
            );
        }
        (Some(_), None) => panic!("a done ends the limit episode, it does not remove it"),
    }
}

// Two hand-worked lifecycles, run once: the retry rounds and the holds.
fn lifecycle() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let spelled = format!(
            "{ATTEMPT_ACTION} {HELD_ACTION} {DONE_ACTION} {REFUSED_ACTION} {FAILED_ACTION} {HEADROOM_ACTION} {REARMED_ACTION} {OPEN} {CLEARED} {SPAWN}"
        );
        assert_eq!(ACTIONS, spelled);
        assert_eq!((MAX_ROUNDS, RETRY_COOLDOWN_SECS), (3, 1800));
        assert_eq!(Timestamp::from_epoch(T0).to_string(), KEY);
        let cd = RETRY_COOLDOWN_SECS;
        let (r, f) = (Some(Outcome::Refused), Some(Outcome::Failed));
        let mut log = vec![watch(T0, OPEN, None, None)];
        let mut at = |t: i64, action: &str| {
            log.push(watch(T0 + t, action, Some(KEY), None));
            let e = episode(&log, SESSION, SLOT, AGENT).expect("the episode stays");
            (e.attempts, e.rounds, e.open.map(|o| o.epoch() - T0), e.terminal, e.gave_up())
        };
        assert_eq!(at(1, ATTEMPT_ACTION), (1, 0, Some(1), None, false));
        assert_eq!(at(2, REFUSED_ACTION), (1, 1, None, r, false));
        // Inside the cooldown an attempt counts and the round stays closed.
        assert_eq!(at(3, ATTEMPT_ACTION), (2, 1, Some(3), r, false));
        // Exactly the cooldown after the closing: the next round.
        assert_eq!(at(2 + cd - 1, ATTEMPT_ACTION), (3, 1, Some(2 + cd - 1), r, false));
        assert_eq!(at(2 + cd, ATTEMPT_ACTION), (1, 1, Some(2 + cd), None, false));
        assert_eq!(at(3 + cd, FAILED_ACTION), (1, 2, None, f, false));
        // The first outcome of a round wins; a closing past the cooldown
        // reopens the round and ends it: round three, nothing left.
        assert_eq!(at(3 + 2 * cd - 1, REFUSED_ACTION), (1, 2, None, f, false));
        assert_eq!(at(3 + 2 * cd, REFUSED_ACTION), (0, 3, None, r, true));
        // Spent rounds: attempts count, nothing reopens or recounts.
        assert_eq!(at(4 + 3 * cd, ATTEMPT_ACTION), (1, 3, Some(4 + 3 * cd), r, true));
        // A done is final and counts no round.
        assert_eq!(at(6 + 5 * cd, DONE_ACTION), (1, 3, None, Some(Outcome::Done), false));

        let mut log = vec![watch(T0, OPEN, None, None), watch(T0 + 1, ATTEMPT_ACTION, Some(KEY), None)];
        let mut hold = |t: i64, r: &str, say: &str| {
            log.push(watch(T0 + t, HELD_ACTION, Some(r), Some(say)));
            let e = episode(&log, SESSION, SLOT, AGENT).expect("the episode stays");
            (e.held, e.open.is_none(), e.choice_held, e.holds_with(say))
        };
        assert_eq!(hold(2, KEY, "busy: turn"), (true, true, false, true));
        assert_eq!(hold(3, KEY, "draft: box"), (true, true, false, true));
        // A hold under another key is no hold of this episode.
        assert_eq!(hold(4, "x-foreign", "observed-choice: no"), (true, true, false, false));
        assert_eq!(hold(5, KEY, "observed-choice: x"), (true, true, true, true));
        // No later hold unsays the choice.
        assert_eq!(hold(6, KEY, "busy: turn"), (true, true, true, true));
    });
}

// A hostile journal folded into its auto-reseat episodes through the public
// wrappers. Each line is one `events.jsonl` record; a line the journal parser
// refuses is skipped, because the strict damaged-journal ledger is the
// caller's. No input may panic, an episode must trace back to authorized
// records, records no authority backs must change nothing, and authorized ones
// must move the path by the contract of their kind.
fuzz_target!(|data: &[u8]| {
    lifecycle();
    let text = String::from_utf8_lossy(data);
    let lines = text.lines().take(256);
    let events: Vec<Event> = lines
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let base = std::hint::black_box(folds(&events));
    assert_eq!(folds(&events), base, "a fold is a function of its records");
    for found in [&base.0, &base.1].into_iter().flatten() {
        sound(found, &events);
    }
    selection(&events, &base);
    let at = events.iter().map(|e| e.ts.epoch()).max().unwrap_or(0);
    let live: Vec<String> = [&base.0, &base.1]
        .into_iter()
        .flatten()
        .map(|e| e.reference().to_owned())
        .collect();
    let fold = |a: usize, b: &u8| a.wrapping_mul(31).wrapping_add(usize::from(*b));
    let seed = data.iter().fold(0usize, fold);
    let mut noisy = events.clone();
    for (i, record) in noise(&live, at, seed).into_iter().enumerate() {
        let place = mix(seed, 100 + i) % (noisy.len() + 1);
        noisy.insert(place, record);
    }
    assert_eq!(
        folds(&noisy),
        base,
        "records no authority backs change nothing"
    );
    authorized(&events, &base, at, seed);
});
