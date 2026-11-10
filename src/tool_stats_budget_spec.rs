//! #288 B2 acceptance: plan-r3 section 7 and lead's B2 rulings.
//! These are fold-boundary inputs. Counts and ordering are hand facts.
//! No expected byte charge comes from `size_of` or the implementation.

use super::*;
use crate::tool_stats_fixture as f;

const RESERVE: usize = 2 * (4 << 20);
const ROOM: usize = 1 << 20;

fn ask() -> Ask {
    Ask {
        page: Page::Tools,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn attempt(id: u128, tool: &str, turn: Option<Key>) -> Event {
    Event::Attempt {
        id: Key(id.to_le_bytes()),
        tool: Ident::Name(tool.into()),
        mcp: None,
        model: None,
        turn,
        message: None,
    }
}

fn one(adapter: Adapter, id: &str, key: u128, tool: &str) -> Conversation {
    Conversation {
        adapter,
        id: id.into(),
        memberships: Vec::new(),
        evidence_complete: true,
        reseated: false,
        streams: vec![Stream {
            role: StreamRole::Parent,
            copy: 0,
            events: vec![attempt(key, tool, None)],
            malformed: 0,
            over_cap: 0,
        }],
        gaps: Vec::new(),
        read: true,
    }
}

fn count(view: &View) -> u64 {
    match view.totals {
        Cols::Tool(cols) => cols.attempts,
        other => panic!("Tools: {other:?}"),
    }
}

fn ledger_gap(view: &View) -> bool {
    view.gaps
        .iter()
        .any(|gap| gap.kind == GapKind::Ledger && gap.count > 0)
}

fn small() -> Vec<Conversation> {
    // Deliberately not canonical feed order. All display identities have
    // equal lengths. Canonical order is Claude C, Claude D, Codex C.
    vec![
        one(Adapter::Codex, f::C, 3, "ccc"),
        one(Adapter::Claude, f::D, 2, "bbb"),
        one(Adapter::Claude, f::C, 1, "aaa"),
    ]
}

fn assert_prefix(view: &View) {
    let names = ["aaa", "bbb", "ccc"];
    let admitted = usize::try_from(count(view)).unwrap_or(usize::MAX);
    assert!(admitted <= 3);
    assert_eq!(view.rows.len(), admitted);
    let expected: Vec<_> = names[..admitted]
        .iter()
        .map(|name| Item::Tool(Ident::Name((*name).into())))
        .collect();
    let actual: Vec<_> = view.rows.iter().map(|row| row.item.clone()).collect();
    assert_eq!(
        actual, expected,
        "admission must be a canonical prefix, never a later fitting item"
    );
    if admitted < 3 {
        assert!(ledger_gap(view));
        assert!(view.partial());
    }
}

#[test]
fn b2_fixed_two_view_reserve_is_subtracted_before_direct_fold_admission() {
    assert_eq!(VIEW_RESERVE, RESERVE, "fixed two-view reserve is 8 MiB");
    assert_eq!(Caps::SLICE.ledger, 64 << 20);
    let inputs = [one(Adapter::Claude, f::C, 1, "aaa")];
    let exhausted = fold(
        &inputs,
        &ask(),
        &Caps {
            ledger: RESERVE,
            ..Caps::SLICE
        },
    );
    assert_eq!(count(&exhausted), 0, "views consume the whole raw cap");
    assert!(exhausted.rows.is_empty());
    assert!(ledger_gap(&exhausted));
    let roomy = fold(
        &inputs,
        &ask(),
        &Caps {
            ledger: RESERVE + ROOM,
            ..Caps::SLICE
        },
    );
    assert_eq!(
        count(&roomy),
        1,
        "one tiny call fits within 1 MiB of observation room"
    );
    assert!(!ledger_gap(&roomy));
    // The reserve is fixed, independent of which view the caller asks for.
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let view = fold(
            &inputs,
            &Ask { page, ..ask() },
            &Caps {
                ledger: RESERVE,
                ..Caps::SLICE
            },
        );
        assert!(
            ledger_gap(&view),
            "empty displayed tables do not release view reserve"
        );
    }
}

#[test]
fn b2_admission_is_a_canonical_prefix_under_order_and_duplicate_feeding() {
    let inputs = small();
    let mut saw_between = false;
    // Equal tiny per-conversation charges have a power-of-two budget between
    // one charge and three charges. No exact byte cost is an expected value.
    // Every intermediate result must satisfy the prefix invariant.
    for power in 0..=20 {
        let caps = Caps {
            ledger: RESERVE + (1 << power),
            ..Caps::SLICE
        };
        let expected = fold(&inputs, &ask(), &caps);
        assert_prefix(&expected);
        saw_between |= (1..3).contains(&count(&expected));
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let permuted: Vec<_> = order
                .into_iter()
                .map(|index| inputs[index].clone())
                .collect();
            assert_eq!(fold(&permuted, &ask(), &caps), expected);
            let mut duplicates = permuted.clone();
            duplicates.extend(permuted);
            assert_eq!(fold(&duplicates, &ask(), &caps), expected);
        }
    }
    assert!(
        saw_between,
        "equal tiny conversations must have a partial-admission budget between none and all"
    );
    assert_eq!(count(&fold(&inputs, &ask(), &Caps::SLICE)), 3);
}

#[test]
fn b2_growth_evicts_a_previously_fitting_prefix_without_admitting_later_items() {
    let inputs = small();
    let caps = (0..=20).find_map(|power| {
        let caps = Caps {
            ledger: RESERVE + (1 << power),
            ..Caps::SLICE
        };
        (count(&fold(&inputs, &ask(), &caps)) == 1).then_some(caps)
    });
    // An exact-one budget exists for equal tiny per-conversation charges.
    // This selects an input cap; the expected membership is still the
    // independently known first Claude conversation, never a measured cost.
    let Some(caps) = caps else {
        panic!("no one-conversation admission boundary");
    };
    let before = fold(&inputs, &ask(), &caps);
    assert_eq!(count(&before), 1);
    assert_eq!(before.rows[0].item, Item::Tool(Ident::Name("aaa".into())));
    let mut grown = inputs.clone();
    grown[2].streams[0]
        .events
        .extend((10..4106).map(|id| attempt(id, "aaa", None)));
    assert_eq!(
        count(&fold(&grown, &ask(), &Caps::SLICE)),
        4099,
        "growth remains within the normal conversation and ledger caps"
    );
    let evicted = fold(&grown, &ask(), &caps);
    assert_eq!(
        count(&evicted),
        0,
        "the first conversation no longer fits, so the prefix is empty"
    );
    assert!(ledger_gap(&evicted));
    assert!(evicted.partial());
    let restored = fold(&inputs, &ask(), &caps);
    assert_eq!(
        restored, before,
        "shrinking the input restores exactly the prior admitted set"
    );
}

fn turn_fixture(turns: usize) -> Conversation {
    let mut conversation = one(Adapter::Codex, f::C, 1, "read");
    conversation.streams[0].events[0] = attempt(1, "read", Some(Key(0_u128.to_le_bytes())));
    conversation.streams[0]
        .events
        .extend((0..turns).map(|turn| Event::Context {
            turn: Key(u128::try_from(turn).unwrap_or(u128::MAX).to_le_bytes()),
            model: Ident::Name("model-a".into()),
            effort: Ident::Name("high".into()),
        }));
    conversation
}

#[test]
fn b2_turn_limit_is_distinct_contexts_and_refuses_the_whole_conversation() {
    assert_eq!(Caps::SLICE.turns, 4096);
    for limit in [2, 4096] {
        let caps = Caps {
            turns: limit,
            ..Caps::SLICE
        };
        let at_limit = turn_fixture(limit);
        let fits = fold(std::slice::from_ref(&at_limit), &ask(), &caps);
        assert_eq!(count(&fits), 1);
        assert_eq!(fits.detail.len(), 1);
        assert_eq!(fits.detail[0].model, Cell::Value("model-a".into()));
        assert_eq!(fits.detail[0].effort, Cell::Value("high".into()));
        assert!(!ledger_gap(&fits));
        let mut repeated = at_limit;
        repeated.streams[0].events.push(Event::Context {
            turn: Key(0_u128.to_le_bytes()),
            model: Ident::Name("model-a".into()),
            effort: Ident::Name("high".into()),
        });
        assert_eq!(
            fold(&[repeated], &ask(), &caps),
            fits,
            "repeated identical context is not a new distinct turn"
        );
        let refused = fold(&[turn_fixture(limit + 1)], &ask(), &caps);
        assert_eq!(
            count(&refused),
            0,
            "over-limit conversation is refused whole, never per-turn relabeled"
        );
        assert!(refused.rows.is_empty() && refused.detail.is_empty());
        assert!(ledger_gap(&refused));
        assert!(refused.partial());
        let survivor = fold(
            &[turn_fixture(limit + 1), one(Adapter::Codex, f::D, 2, "bbb")],
            &ask(),
            &caps,
        );
        assert_eq!(
            count(&survivor),
            1,
            "a per-conversation turn refusal must not refuse another conversation"
        );
        assert_eq!(survivor.rows.len(), 1);
        assert_eq!(survivor.rows[0].item, Item::Tool(Ident::Name("bbb".into())));
        assert_eq!(survivor.detail.len(), 1);
        assert_eq!(survivor.detail[0].attempts, 1);
        assert!(ledger_gap(&survivor));
        assert!(survivor.partial());
    }
}

#[test]
fn b2_partial_includes_conflict_shared_id_and_every_positive_gap() {
    let mut view = fold(
        &[one(Adapter::Claude, f::C, 1, "aaa")],
        &ask(),
        &Caps::SLICE,
    );
    assert_eq!((view.read, view.listed), (1, 1));
    assert!(!view.partial());
    let mut unread = one(Adapter::Claude, f::C, 1, "aaa");
    unread.read = false;
    let progress = fold(&[unread], &ask(), &Caps::SLICE);
    assert_eq!((progress.read, progress.listed), (0, 1));
    assert!(progress.gaps.is_empty(), "this case isolates read progress");
    assert!(progress.partial());
    for kind in [
        GapKind::Unlocated,
        GapKind::Unreadable,
        GapKind::Changed,
        GapKind::NotUnique,
        GapKind::Malformed,
        GapKind::OverCap,
        GapKind::Conflict,
        GapKind::Unsupported("other".into()),
        GapKind::Ledger,
        GapKind::ReadPathOverLimit,
        GapKind::IdentCap,
    ] {
        view.gaps = vec![GapLine {
            kind,
            conversation: Some(f::C.into()),
            count: 1,
        }];
        assert!(
            view.partial(),
            "any gap makes complete-read numbers Partial"
        );
    }
    view.gaps.clear();
    assert!(!view.partial());
}
