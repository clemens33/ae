//! Driver pins of the persisted model against the cold fold: the identity
//! cap applied AFTER a directory's proof is chosen (review r1 R2), and the
//! proof seed of `tool_stats_incr` exercising the inferred-Read branch.
#![allow(clippy::expect_used, reason = "test assertions")]

use super::*;
use crate::tool_stats::{Filter, Ident, Load, Resource, ScopeClass, SkillCols, fold};
use crate::usage::claude_calls;
use std::path::PathBuf;

fn id(n: u8) -> Key {
    Key([n; 16])
}

fn parent() -> StreamKey {
    StreamKey {
        role: StreamRole::Parent,
        copy: [0; 16],
    }
}

fn cold(adapter: Adapter, id: &str, events: Vec<Event>) -> Conversation {
    Conversation {
        adapter,
        id: id.to_owned(),
        memberships: Vec::new(),
        evidence_complete: false,
        reseated: false,
        streams: vec![Stream {
            role: StreamRole::Parent,
            copy: 0,
            events,
            malformed: 0,
            over_cap: 0,
        }],
        gaps: Vec::new(),
        read: false,
    }
}

fn skill(n: u8, name: &str) -> Event {
    Event::Skill {
        id: id(n),
        skill: Ident::Name(name.to_owned()),
    }
}

fn loaded(n: u8, class: ScopeClass, dir: Dir) -> [Event; 2] {
    [
        Event::SkillBase {
            id: id(n),
            class,
            resource: Resource([n; 6]),
            dir,
        },
        Event::Result {
            id: id(n),
            error: false,
            load: Some(Load::Loaded),
        },
    ]
}

/// Three Skill names under an identity cap of one: `b` (Other) and `c`
/// (User) prove the SAME directory, and a successful Read under it is
/// inferred. The cold fold maps `b` and `c` to unresolved first and then
/// chooses the directory's row; the persisted view must choose the same.
#[test]
fn a_proof_is_chosen_after_the_identity_cap_maps_its_names() {
    let dir = Dir([9; 16]);
    let mut events = vec![skill(1, "a"), skill(2, "b"), skill(3, "c")];
    events.extend(loaded(2, ScopeClass::Other, dir));
    events.extend(loaded(3, ScopeClass::User, dir));
    events.push(Event::Read {
        id: id(4),
        direct: None,
        ancestors: vec![dir],
        over: false,
        skills: false,
    });
    events.push(Event::Result {
        id: id(4),
        error: false,
        load: None,
    });
    let caps = Caps {
        idents: 1,
        ..Caps::SLICE
    };
    let mut conv = Conv::new(Adapter::Claude, "c1");
    conv.apply(&parent(), &events, &caps);
    let published = Published {
        generation: 1,
        convs: vec![publish(&conv.summary, None)],
        over_cap: 0,
    };
    let ask = Ask {
        page: Page::Skills,
        scope: Scope::All,
        filter: Filter::All,
    };
    let want = fold(&[cold(Adapter::Claude, "c1", events)], &ask, &caps);
    assert!(
        want.rows.iter().any(|row| matches!(
            &row.item,
            Item::Skill {
                name: Ident::Unresolved(Kind::Skill),
                scope: Some((ScopeClass::User, _)),
            }
        ) && matches!(
            row.cols,
            Cols::Skill(SkillCols { reads: 1, .. })
        )),
        "the cold fold attributes the Read to the User row: {:?}",
        want.rows
    );
    assert_eq!(combine(&published, &ask, &caps), want);
    assert_eq!(
        super::super::doc::parse_view(&super::super::doc::render_view(&published)),
        Ok(published),
        "the proofs survive the view codec"
    );
}

const PROOF_SEED: &[u8] =
    include_bytes!("../../fuzz/seeds/tool_stats_incr/claude-proof-replay-copy");

/// The proof seed, routed as `tool_stats_incr` routes it (shape and split
/// bytes, then one route byte a line), holds a successful Read that only a
/// Skill call's proven base directory attributes (Q2 b).
#[test]
fn the_proof_seed_counts_an_inferred_read() {
    let config = [PathBuf::from("/r/config")];
    let work = [PathBuf::from("/r/work")];
    let roots = claude_calls::Roots {
        config_homes: &config,
        work_dirs: &work,
    };
    let body = PROOF_SEED.get(2..).expect("shape and split bytes");
    let mut input: Vec<Conversation> = (0..2)
        .map(|index| {
            let mut one = cold(
                Adapter::Claude,
                &format!("conversation-{index}"),
                Vec::new(),
            );
            one.streams.clear();
            one
        })
        .collect();
    for line in body.split(|byte| *byte == b'\n') {
        let Some((&route, text)) = line.split_first() else {
            continue;
        };
        let index = usize::from(route % 3);
        assert!(index < 2, "Claude conversations only");
        let role = if route & 0x10 == 0 {
            StreamRole::Parent
        } else {
            StreamRole::Child("child".into())
        };
        let copy = u32::from(route >> 7);
        let streams = &mut input[index].streams;
        let at = streams
            .iter()
            .position(|stream| stream.role == role && stream.copy == copy)
            .unwrap_or_else(|| {
                streams.push(Stream {
                    role,
                    copy,
                    events: Vec::new(),
                    malformed: 0,
                    over_cap: 0,
                });
                streams.len() - 1
            });
        claude_calls::line(text, &roots, &mut streams[at].events);
    }
    let events: Vec<&Event> = input
        .iter()
        .flat_map(|one| &one.streams)
        .flat_map(|stream| &stream.events)
        .collect();
    assert!(
        events.iter().any(|event| matches!(
            event,
            Event::Read {
                direct: None,
                over: false,
                ..
            }
        )),
        "an indirect Read"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            Event::Read {
                direct: Some(_),
                ..
            }
        )),
        "no Read names its skill directly"
    );
    let ask = Ask {
        page: Page::Skills,
        scope: Scope::All,
        filter: Filter::All,
    };
    let view = fold(&input, &ask, &Caps::SLICE);
    assert!(
        matches!(view.totals, Cols::Skill(SkillCols { reads: 1, .. })),
        "the proof attributes the Read: {:?}",
        view.totals
    );
}

/// Review r2 R3: line counts are never truncated. A parent stream's 2^40
/// malformed lines and a child's one sum as the cold fold sums them, past
/// any bound, and `note` keeps every line it is told.
#[test]
fn line_counts_past_two_to_the_forty_stay_exact_against_the_fold() {
    let big = 1_u64 << 40;
    let child = StreamKey {
        role: StreamRole::Child("child".into()),
        copy: [1; 16],
    };
    // The review's counterexample first: parent 2^40, child 1.
    let mut conv = Conv::new(Adapter::Claude, "c1");
    conv.note(&parent(), big, 0).expect("representable");
    conv.note(&child, 1, 0).expect("representable");
    let mut cold = cold(Adapter::Claude, "c1", Vec::new());
    cold.streams[0].malformed = big;
    cold.streams.push(Stream {
        role: StreamRole::Child("child".into()),
        copy: 1,
        events: Vec::new(),
        malformed: 1,
        over_cap: 0,
    });
    let same = |conv: &Conv, cold: &Conversation| {
        let published = Published {
            generation: 1,
            convs: vec![publish(&conv.summary, None)],
            over_cap: 0,
        };
        for page in [Page::Skills, Page::Mcp, Page::Tools] {
            let ask = Ask {
                page,
                scope: Scope::All,
                filter: Filter::All,
            };
            assert_eq!(
                combine(&published, &ask, &Caps::SLICE),
                fold(std::slice::from_ref(cold), &ask, &Caps::SLICE)
            );
        }
    };
    assert_eq!(
        conv.summary.gaps.get(&GapKind::Malformed),
        Some(&(big + 1)),
        "2^40 parent lines and one child line"
    );
    same(&conv, &cold);
    // One more parent line, and over-cap lines past the bound too.
    conv.note(&parent(), 1, big + 1).expect("representable");
    assert_eq!(conv.resume.lines.get(&parent()), Some(&(big + 1, big + 1)));
    cold.streams[0].malformed = big + 1;
    cold.streams[0].over_cap = big + 1;
    same(&conv, &cold);
    let doc = super::super::doc::render_conv(&conv);
    assert_eq!(super::super::doc::parse_conv(&doc), Ok(conv));
}

/// A conversation loaded from an accepted document whose parent stream
/// holds `malformed` lines.
fn loaded_near(adapter: Adapter, malformed: u64) -> Conv {
    let mut conv = Conv::new(adapter, "c1");
    conv.note(&parent(), malformed, 0).expect("representable");
    let doc = super::super::doc::render_conv(&conv);
    super::super::doc::parse_conv(&doc).expect("an accepted document")
}

fn child() -> StreamKey {
    StreamKey {
        role: StreamRole::Child("child".into()),
        copy: [1; 16],
    }
}

/// Review r3 R3: a live update of an accepted document never overflows a
/// summary. One that a summary could not add up routes the conversation to
/// damage: it is rebuilt from its sources, identity kept, nothing of the
/// update accepted. A representable update stays exact.
#[test]
fn a_live_update_past_u64_rebuilds_the_conversation_and_a_representable_one_stays_exact() {
    let max = u64::MAX;
    let mut conv = loaded_near(Adapter::Claude, max - 1);
    assert_eq!(conv.note(&child(), 2, 0), Err(Overflow));
    // B2 (lead ae-20261109T165907Z-00e19e67): the reset leaves only the
    // visible damage mark.
    assert!(
        conv.resume.lines.is_empty()
            && conv.summary.gaps == BTreeMap::from([(GapKind::RebuiltFromSource, 1)]),
        "rebuilt from the sources: {:?}",
        conv.summary.gaps
    );
    assert_eq!(
        (conv.summary.adapter, conv.summary.id.as_str()),
        (Adapter::Claude, "c1")
    );
    let mut conv = loaded_near(Adapter::Claude, max - 1);
    assert_eq!(
        conv.set_gaps(vec![GapKind::Malformed, GapKind::Malformed]),
        Err(Overflow)
    );
    assert!(conv.resume.lines.is_empty() && conv.resume.marks.is_empty());
    let mut conv = loaded_near(Adapter::Claude, max - 3);
    conv.note(&child(), 1, 0).expect("representable");
    conv.set_gaps(vec![GapKind::Malformed])
        .expect("representable");
    assert_eq!(
        conv.summary.gaps.get(&GapKind::Malformed),
        Some(&(max - 1)),
        "max - 3 parent lines, one child line, one mark: exact"
    );
}

/// Review r3 R3, the view side: two accepted conversations whose gap lines
/// share a short id cannot both enter one published view when a combine
/// could not add them up.
#[test]
fn a_published_view_never_holds_counts_a_combine_cannot_add_up() {
    let max = u64::MAX;
    let mut published = Published {
        generation: 1,
        convs: Vec::new(),
        over_cap: 0,
    };
    let first = publish(&loaded_near(Adapter::Claude, max - 1).summary, None);
    published.admit(first).expect("one conversation adds up");
    let before = published.clone();
    let second = publish(&loaded_near(Adapter::Codex, max - 1).summary, None);
    assert_eq!(published.admit(second), Err(Overflow));
    assert_eq!(published, before, "the view is kept as it was");
    // A conversation admitted again replaces its entry.
    let small = publish(&loaded_near(Adapter::Claude, 5).summary, None);
    published.admit(small.clone()).expect("replaces");
    assert_eq!(published.convs, vec![small]);
    let ask = Ask {
        page: Page::Tools,
        scope: Scope::All,
        filter: Filter::All,
    };
    let view = combine(&published, &ask, &Caps::SLICE);
    assert!(view.gaps.iter().all(|gap| gap.count < max));
}

/// A conversation with one Read attempt, published alone.
fn one_attempt(adapter: Adapter, id: &str) -> PublishedConv {
    let mut conv = Conv::new(adapter, id);
    conv.apply(
        &parent(),
        &[Event::Attempt {
            id: Key([1; 16]),
            tool: Ident::Name("Read".into()),
            mcp: None,
            model: None,
            turn: None,
            message: None,
        }],
        &Caps::SLICE,
    );
    publish(&conv.summary, None)
}

/// Review r4 R6: a view holds each conversation once, in `(adapter, id)`
/// order, so `admit` replaces in place. A decoded view out of order or
/// with a conversation twice is damage.
#[test]
fn a_view_holds_each_conversation_once_in_order() {
    let view = |convs: Vec<PublishedConv>| Published {
        generation: 1,
        convs,
        over_cap: 0,
    };
    let (a, z) = (
        one_attempt(Adapter::Claude, "a"),
        one_attempt(Adapter::Claude, "z"),
    );
    let codex = one_attempt(Adapter::Codex, "a");
    for bad in [
        view(vec![z.clone(), a.clone()]),
        view(vec![a.clone(), a.clone()]),
        view(vec![codex.clone(), a.clone()]),
    ] {
        assert!(
            matches!(
                super::super::doc::parse_view(&super::super::doc::render_view(&bad)),
                Err(super::super::doc::DocError::Damaged(_))
            ),
            "{bad:?}"
        );
    }
    let ordered = view(vec![a.clone(), z.clone(), codex.clone()]);
    let mut loaded = super::super::doc::parse_view(&super::super::doc::render_view(&ordered))
        .expect("an ordered view");
    let mut newer = z;
    newer.summary.read = true;
    loaded.admit(newer.clone()).expect("replaces");
    assert_eq!(loaded.convs, vec![a, newer, codex]);
}

/// R3-2: a read from 0 taken into a refused conversation replaces the
/// stream's line counts and never adds to them.
#[test]
fn a_restart_replaces_line_counts_even_when_refused() {
    let mut conv = Conv::new(Adapter::Claude, "c1");
    conv.note(&child(), 2, 1).expect("representable");
    conv.refuse(Refused::Observations);
    conv.restart(&child());
    conv.note(&child(), 2, 1).expect("representable");
    assert_eq!(conv.resume.lines.get(&child()), Some(&(2, 1)));
}

/// B2 (lead ae-20261109T165907Z-00e19e67): held counts no real source makes
/// reset the conversation whole and mark it, visibly; a later source
/// rebuild and the codec keep the mark.
#[test]
fn a_damage_reset_marks_the_conversation_and_every_rebuild_keeps_it() {
    let mut conv = loaded_near(Adapter::Claude, u64::MAX - 1);
    assert_eq!(
        conv.summary.gaps.get(&GapKind::RebuiltFromSource),
        None,
        "control: not damaged yet"
    );
    assert_eq!(conv.note(&child(), 2, 0), Err(Overflow));
    assert!(conv.resume.rebuilt, "the reset marks it");
    assert_eq!(conv.summary.gaps.get(&GapKind::RebuiltFromSource), Some(&1));
    conv.rebuild();
    assert!(conv.resume.rebuilt, "a source rebuild keeps it");
    let round = super::super::doc::parse_conv(&super::super::doc::render_conv(&conv))
        .expect("an accepted document");
    assert!(round.resume.rebuilt, "the document keeps it");
    assert_eq!(
        round.summary.gaps.get(&GapKind::RebuiltFromSource),
        Some(&1)
    );
}

// --- #288 mutants slot 1 survivors (incr.rs:300, 402, 703, 710, 719) -------

fn context(n: u8) -> Event {
    Event::Context {
        turn: id(n),
        model: Ident::Name("model".to_owned()),
        effort: Ident::Name("high".to_owned()),
    }
}

/// incr.rs:300: a conversation of exactly `turns` turn contexts is counted;
/// one more refuses it.
#[test]
fn the_turn_cap_admits_its_own_count_and_refuses_one_more() {
    let caps = Caps {
        turns: 2,
        ..Caps::SLICE
    };
    let mut conv = Conv::new(Adapter::Codex, "c1");
    conv.apply(&parent(), &[context(1), context(2)], &caps);
    assert_eq!(conv.summary.turns, 2, "control: two turns held");
    assert_eq!(conv.summary.refused, None, "exactly the cap is counted");
    conv.apply(&parent(), &[context(3)], &caps);
    assert_eq!(
        conv.summary.refused,
        Some(Refused::Turns),
        "one more is refused"
    );
}

/// The gap lines `coverage` adds to an empty view under `scope`.
fn coverage_lines(coverage: &Coverage, scope: Scope) -> Vec<(GapKind, u64)> {
    let ask = Ask {
        page: Page::Tools,
        scope,
        filter: Filter::All,
    };
    let mut view = combine(
        &Published {
            generation: coverage.generation,
            convs: Vec::new(),
            over_cap: 0,
        },
        &ask,
        &Caps::SLICE,
    );
    let before = view.gaps.len();
    coverage.apply(&mut view, &ask);
    // The order of the lines is not the requirement: compare them as a set.
    let mut lines: Vec<(GapKind, u64)> = view.gaps[before..]
        .iter()
        .map(|line| (line.kind.clone(), line.count))
        .collect();
    lines.sort_by_key(|(kind, count)| (format!("{kind:?}"), *count));
    lines
}

/// incr.rs:703, 710, 719: each coverage count draws its line only when it is
/// not zero, and the omitted line only in a scope that shows a session.
#[test]
fn coverage_draws_each_nonzero_count_once_and_omitted_only_where_a_session_shows() {
    let session = || Scope::This(Some("s1".to_owned()));
    let none = Coverage {
        generation: 1,
        ..Coverage::default()
    };
    for scope in [Scope::All, Scope::This(None), session()] {
        assert_eq!(
            coverage_lines(&none, scope.clone()),
            Vec::new(),
            "zero counts draw no line under {scope:?}"
        );
    }
    let counted = Coverage {
        generation: 1,
        hidden: 2,
        oversize: 3,
        omitted: 4,
        ..Coverage::default()
    };
    // Sorted by kind name, as coverage_lines returns them.
    let all = vec![
        (GapKind::FieldOverLimit, 3),
        (GapKind::Ledger, 4),
        (GapKind::Unreadable, 2),
    ];
    assert_eq!(coverage_lines(&counted, Scope::All), all);
    assert_eq!(coverage_lines(&counted, session()), all);
    assert_eq!(
        coverage_lines(&counted, Scope::This(None)),
        vec![(GapKind::FieldOverLimit, 3), (GapKind::Unreadable, 2)],
        "no session shown: the omitted line is left out"
    );
}

/// incr.rs:402: a read from 0 taken into a REFUSED conversation that found
/// no bad lines leaves no stale malformed or over-cap gap behind; neither
/// the refused `apply` nor an all-zero `note` would refresh the summary.
#[test]
fn a_restart_of_a_refused_conversation_drops_its_stale_line_gaps() {
    let mut conv = Conv::new(Adapter::Claude, "c1");
    conv.note(&child(), 2, 1).expect("representable");
    conv.refuse(Refused::Observations);
    assert_eq!(
        (
            conv.summary.gaps.get(&GapKind::Malformed),
            conv.summary.gaps.get(&GapKind::OverCap)
        ),
        (Some(&2), Some(&1)),
        "control: the refused conversation shows its line gaps"
    );
    conv.restart(&child());
    conv.apply(&child(), &[], &Caps::SLICE);
    conv.note(&child(), 0, 0).expect("representable");
    assert_eq!(
        (
            conv.summary.gaps.get(&GapKind::Malformed),
            conv.summary.gaps.get(&GapKind::OverCap)
        ),
        (None, None),
        "the read from 0 replaced the line counts: {:?}",
        conv.summary.gaps
    );
}
