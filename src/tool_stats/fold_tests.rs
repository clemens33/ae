//! #288 B2 review r1 regressions (t288-spec F1-F3): driver tests pinned RED
//! before the fold fix. Expected values are hand facts from plan-r3 §2, §7
//! and §8 U4, never read off the fold.

use super::*;
use crate::tool_stats::Role;

fn ask() -> Ask {
    Ask {
        page: Page::Tools,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn attempt(id: u8, tool: &str, turn: Option<Key>) -> Event {
    Event::Attempt {
        id: Key([id; 16]),
        tool: Ident::Name(tool.into()),
        mcp: None,
        model: None,
        turn,
        message: None,
    }
}

fn conversation(adapter: Adapter, streams: Vec<(u32, Vec<Event>)>) -> Conversation {
    Conversation {
        adapter,
        id: "conversation".into(),
        memberships: Vec::new(),
        evidence_complete: true,
        reseated: false,
        streams: streams
            .into_iter()
            .map(|(copy, events)| Stream {
                role: StreamRole::Parent,
                copy,
                events,
                malformed: 0,
                over_cap: 0,
            })
            .collect(),
        gaps: Vec::new(),
        read: true,
    }
}

fn claude(events: Vec<Event>) -> Conversation {
    conversation(Adapter::Claude, vec![(0, events)])
}

fn conflicts(view: &View) -> u64 {
    view.gaps
        .iter()
        .filter(|line| line.kind == GapKind::Conflict)
        .map(|line| line.count)
        .sum()
}

/// F1: the observation cap counts the conversation's observation SET, so an
/// overlapping, split or repeated feed of the same records is admitted
/// exactly like their union (plan §7: any duplicate feeding or split point).
#[test]
fn f1_overlapping_split_and_repeated_feeds_are_admitted_like_their_union() {
    let (a, b) = (attempt(1, "aaa", None), attempt(2, "bbb", None));
    let caps = Caps {
        per_conversation: 2,
        ..Caps::SLICE
    };
    let union = fold(&[claude(vec![a.clone(), b.clone()])], &ask(), &caps);
    assert_eq!(
        union.totals.attempts(),
        2,
        "two observations fit a cap of 2"
    );
    let overlapping = [claude(vec![a.clone(), b.clone()]), claude(vec![a.clone()])];
    assert_eq!(
        fold(&overlapping, &ask(), &caps),
        union,
        "overlapping prefix"
    );
    let split = [claude(vec![a.clone()]), claude(vec![b.clone()])];
    assert_eq!(fold(&split, &ask(), &caps), union, "split point");
    let one = Caps {
        per_conversation: 1,
        ..Caps::SLICE
    };
    assert_eq!(
        fold(&[claude(vec![a.clone(), a.clone()])], &ask(), &one),
        fold(&[claude(vec![a])], &ask(), &one),
        "a repeated record is one observation"
    );
}

/// The smallest power-of-two room over the reserve that admits `input`.
fn fitting(input: &Conversation) -> Option<u32> {
    (0..=20).find(|power| {
        let caps = Caps {
            ledger: VIEW_RESERVE + (1 << power),
            ..Caps::SLICE
        };
        fold(std::slice::from_ref(input), &ask(), &caps)
            .totals
            .attempts()
            == 1
    })
}

/// F2: retained membership and gap strings are ledger bytes. A conversation
/// carrying 1 MiB of them cannot fit the room that admits it bare.
#[test]
fn f2_retained_membership_and_gap_bytes_are_charged() {
    let bare = claude(vec![attempt(1, "aaa", None)]);
    let Some(power) = fitting(&bare) else {
        panic!("one bare call fits within 1 MiB of room");
    };
    assert!(power < 20, "the bare room stays below 1 MiB");
    let caps = Caps {
        ledger: VIEW_RESERVE + (1 << power),
        ..Caps::SLICE
    };
    let mut members = bare.clone();
    members.memberships = (0..64)
        .map(|index| Membership {
            session: Some(format!("{index:0>16384}")),
            slot: "main".into(),
            role: Role::Live,
            client: RecordedClient::Missing,
        })
        .collect();
    assert_eq!(
        fold(&[members], &ask(), &caps).totals.attempts(),
        0,
        "1 MiB of membership strings is retained"
    );
    let mut gaps = bare;
    gaps.gaps = vec![GapKind::Unsupported("x".repeat(1 << 20))];
    assert_eq!(
        fold(&[gaps], &ask(), &caps).totals.attempts(),
        0,
        "1 MiB of gap text is retained"
    );
}

/// N2 (B2 review r2): the fuzz target's low-room caps sweep 4 KiB to
/// 512 KiB over the reserve (`fuzz/fuzz_targets/tool_stats_fold.rs`). One
/// bare call is refused at the bottom of that sweep and admitted inside
/// it, so the sweep exercises both refusal and a positive admission.
#[test]
fn n2_fuzz_room_sweep_brackets_one_call_admission() {
    let bare = claude(vec![attempt(1, "aaa", None)]);
    let Some(power) = fitting(&bare) else {
        panic!("one bare call fits within 1 MiB of room");
    };
    assert!(
        (13..=18).contains(&power),
        "one call needs more than 4 KiB and at most 256 KiB of room, got 2^{power}"
    );
}

/// F2 (B2 review r2): overlapping prefix feeds `[A]`, `[A, B]`, ... merge
/// into one stream that owns each distinct event once, never a stream copy
/// per feed (quadratic storage against a linear charge).
#[test]
fn f2_overlapping_prefix_feeds_merge_into_their_canonical_union() {
    let events: Vec<Event> = (0..64).map(|id| attempt(id, "aaa", None)).collect();
    let feeds: Vec<Conversation> = (1..=events.len())
        .map(|len| claude(events[..len].to_vec()))
        .collect();
    let merged = merge(&feeds);
    assert_eq!(merged.len(), 1, "one (adapter, id) key");
    let held: usize = merged[0]
        .streams
        .iter()
        .map(|stream| stream.events.len())
        .sum();
    assert_eq!(held, 64, "the merge owns each distinct event once");
    assert_eq!(merged[0].streams.len(), 1, "one (role, copy) stream");
}

/// F2 (B2 review r2): a stream's unequal feeds keep their largest malformed
/// and over-cap counts, as `cover` reads them per role (plan §8 U4).
#[test]
fn f2_merged_stream_keeps_the_largest_line_counts() {
    let mut low = claude(vec![attempt(1, "aaa", None)]);
    low.streams[0].malformed = 2;
    low.streams[0].over_cap = 7;
    let mut high = claude(vec![attempt(2, "bbb", None)]);
    high.streams[0].malformed = 5;
    high.streams[0].over_cap = 3;
    let feeds = [low, high];
    let merged = merge(&feeds);
    assert_eq!(merged.len(), 1, "one (adapter, id) key");
    assert_eq!(merged[0].streams.len(), 1, "one (role, copy) stream");
    assert_eq!(merged[0].streams[0].malformed, 5);
    assert_eq!(merged[0].streams[0].over_cap, 7);
    assert_eq!(merged[0].streams[0].events.len(), 2);
}

fn name(ident: &Ident) -> usize {
    match ident {
        Ident::Name(name) => name.len(),
        Ident::Unresolved(_) => 0,
    }
}

/// The names and paths one event owns, walked independently of the fold.
fn event_bytes(event: &Event) -> usize {
    match event {
        Event::Attempt {
            tool, mcp, model, ..
        } => {
            name(tool)
                + mcp
                    .as_ref()
                    .map_or(0, |(server, tool)| name(server) + name(tool))
                + model.as_ref().map_or(0, name)
        }
        Event::Skill { skill, .. } => name(skill),
        Event::Read {
            direct, ancestors, ..
        } => {
            direct.as_ref().map_or(0, |(skill, ..)| name(skill))
                + ancestors.len() * size_of::<Dir>()
        }
        Event::Mcp { server, tool, .. } => name(server) + name(tool),
        Event::Context { model, effort, .. } => name(model) + name(effort),
        Event::Stamp(_) | Event::SkillBase { .. } | Event::Result { .. } => 0,
    }
}

/// Owned bytes an independent walk finds: the conversation's own strings;
/// each distinct event of each `(role, copy)` stream (the raw copy); and
/// each distinct non-timestamp event of each role again (the gathered
/// record that clones it during the fold; copies alias).
fn owned_floor(conversation: &Conversation) -> usize {
    let mut bytes = conversation.id.len();
    for membership in &conversation.memberships {
        bytes += membership.session.as_ref().map_or(0, String::len) + membership.slot.len();
        if let RecordedClient::Label(label) = &membership.client {
            bytes += label.len();
        }
    }
    for gap in &conversation.gaps {
        if let GapKind::Unsupported(tool) = gap {
            bytes += tool.len();
        }
    }
    let mut raw = BTreeSet::new();
    let mut gathered = BTreeSet::new();
    for stream in &conversation.streams {
        if let StreamRole::Child(child) = &stream.role {
            bytes += child.len();
        }
        for event in &stream.events {
            if raw.insert((&stream.role, stream.copy, event)) {
                bytes += event_bytes(event);
            }
            if !matches!(event, Event::Stamp(_)) && gathered.insert((&stream.role, event)) {
                bytes += event_bytes(event);
            }
        }
    }
    bytes
}

fn stream(role: StreamRole, copy: u32, events: Vec<Event>) -> Stream {
    Stream {
        role,
        copy,
        events,
        malformed: 0,
        over_cap: 0,
    }
}

/// F2: the charge bounds what the conversation and its gathered copy own,
/// over valid-length names, MCP pairs, Read paths, a conflicting copy and a
/// separate child role.
#[test]
fn f2_charge_covers_retained_and_gathered_owned_bytes() {
    let word = |byte: char| Ident::Name(byte.to_string().repeat(64));
    let call = Event::Attempt {
        id: Key([1; 16]),
        tool: word('t'),
        mcp: Some((word('s'), word('u'))),
        model: Some(word('m')),
        turn: None,
        message: None,
    };
    let read = Event::Read {
        id: Key([2; 16]),
        direct: Some((word('k'), ScopeClass::User, Resource([7; 6]))),
        ancestors: vec![Dir([1; 16]), Dir([2; 16]), Dir([3; 16])],
        over: false,
    };
    let mcp = Event::Mcp {
        id: Key([3; 16]),
        server: word('a'),
        tool: word('b'),
        outcome: McpOutcome::Success,
        turn: None,
    };
    // The copy disagrees on the call's tool: a conflict, two variants.
    let conflicting = Event::Attempt {
        id: Key([1; 16]),
        tool: word('x'),
        mcp: None,
        model: None,
        turn: None,
        message: None,
    };
    let mut input = claude(Vec::new());
    input.streams = vec![
        stream(
            StreamRole::Parent,
            0,
            vec![call.clone(), read.clone(), mcp, Event::Stamp(5)],
        ),
        stream(StreamRole::Parent, 1, vec![conflicting, read]),
        stream(StreamRole::Child("child".into()), 0, vec![call]),
    ];
    input.memberships.push(Membership {
        session: Some("s".repeat(300)),
        slot: "main".into(),
        role: Role::Live,
        client: RecordedClient::Label("c".repeat(300)),
    });
    input.gaps.push(GapKind::Unsupported("other".into()));
    let floor = owned_floor(&input) + size_of::<Conversation>();
    assert!(
        charge(&input) >= floor,
        "charge {} below the owned floor {floor}",
        charge(&input)
    );
}

/// F2: each declared ledger component is charged where the model says.
#[test]
fn f2_declared_components_are_charged() {
    for record in [
        size_of::<Call>(),
        size_of::<ReadRec>(),
        size_of::<McpRec>(),
        size_of::<(Ident, Ident)>(),
        size_of::<Base>(),
        size_of::<Reply>(),
    ] {
        assert!(
            record <= size_of::<Event>(),
            "an Event bounds every gathered record"
        );
    }
    let base = claude(vec![attempt(1, "aaa", None)]);
    let charged = charge(&base);
    let mut repeated = base.clone();
    repeated.streams[0].events.push(attempt(1, "aaa", None));
    assert_eq!(charge(&repeated), charged, "a repeated record adds nothing");
    let mut overlapping = base.clone();
    overlapping.streams.push(base.streams[0].clone());
    assert_eq!(
        charge(&overlapping),
        charged,
        "an overlapping stream entry adds nothing"
    );
    let mut member = base.clone();
    member.memberships.push(Membership {
        session: Some("s".repeat(100)),
        slot: "t".repeat(50),
        role: Role::Live,
        client: RecordedClient::Label("c".repeat(25)),
    });
    assert!(
        charge(&member) >= charged + size_of::<Membership>() + 175,
        "a membership adds its struct and its strings"
    );
    let mut copy = base.clone();
    copy.streams
        .push(stream(StreamRole::Parent, 1, vec![attempt(1, "aaa", None)]));
    let added = charge(&copy) - charged;
    assert!(
        added >= size_of::<Stream>() + size_of::<Event>() + 3,
        "a copy adds its raw stream"
    );
    assert!(added < temporary(), "a copy adds no second fold temporary");
    let mut more = base.clone();
    more.streams[0].events.push(attempt(2, "bbb", None));
    assert!(
        charge(&more) - charged >= size_of::<Event>() + temporary() + 3 * (1 + CLONES),
        "a distinct observation adds its raw event and its fold temporaries"
    );
    let mut child = base.clone();
    child.streams.push(stream(
        StreamRole::Child("x".into()),
        0,
        vec![attempt(3, "ccc", None)],
    ));
    assert!(
        charge(&child) - charged >= group_floor() + temporary(),
        "a new role adds its gathered group floor"
    );
}

/// F3: differing contexts for one Codex turn are a Conflict line, counted
/// once per conversation however many copies hold them; the call stays
/// counted with an unknown model and effort, and the view is Partial
/// (plan §2, §8 U4).
#[test]
fn f3_conflicting_codex_contexts_are_one_conflict_and_keep_the_call() {
    let turn = Key([9; 16]);
    let context = |model: &str, effort: &str| Event::Context {
        turn,
        model: Ident::Name(model.into()),
        effort: Ident::Name(effort.into()),
    };
    let events = vec![
        attempt(1, "read", Some(turn)),
        context("model-a", "high"),
        context("model-b", "low"),
    ];
    let input = conversation(Adapter::Codex, vec![(0, events.clone()), (1, events)]);
    let view = fold(&[input], &ask(), &Caps::SLICE);
    assert_eq!(view.totals.attempts(), 1, "the call stays counted");
    assert_eq!(view.detail.len(), 1);
    assert_eq!(view.detail[0].model, Cell::Unknown);
    assert_eq!(view.detail[0].effort, Cell::Unknown);
    assert_eq!(conflicts(&view), 1, "one conflicting turn, one line count");
    assert!(view.partial());
}
