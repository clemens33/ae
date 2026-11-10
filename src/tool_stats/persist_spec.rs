//! P1–P3: durable summaries, incremental evidence and crash-safe publication.
#![allow(
    clippy::expect_used,
    reason = "independent frozen acceptance assertions"
)]

use super::incr::{self, Association, Conv, FileState, Published, StreamKey};
use super::{doc, store, *};
use crate::stats::Cell;
use crate::stats_store::disk::{Memory, Op};
use crate::tool_stats_fixture as f;

fn stream(role: StreamRole, copy: u8) -> StreamKey {
    StreamKey {
        role,
        copy: [copy; 16],
    }
}

fn ask(page: Page) -> Ask {
    Ask {
        page,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn attempt(id: u8, name: &str, model: Option<&str>, turn: Option<Key>) -> Event {
    Event::Attempt {
        id: Key([id; 16]),
        tool: Ident::Name(name.into()),
        mcp: None,
        model: model.map(|name| Ident::Name(name.into())),
        turn,
        message: None,
    }
}

fn result(id: u8, error: bool) -> Event {
    Event::Result {
        id: Key([id; 16]),
        error,
        load: Some(Load::Unknown),
    }
}

fn association() -> Association {
    Association {
        memberships: Vec::new(),
        evidence_complete: true,
        reseated: false,
    }
}

fn published(conv: &Conv, generation: u64) -> Published {
    Published {
        generation,
        convs: vec![incr::publish(&conv.summary, Some(&association()))],
        over_cap: 0,
    }
}

fn view(conv: &Conv, page: Page) -> View {
    incr::combine(&published(conv, 1), &ask(page), &Caps::SLICE)
}

fn tool_counts(view: &View) -> ToolCols {
    match view.totals {
        Cols::Tool(cols) => cols,
        other => panic!("Tools: {other:?}"),
    }
}

fn skill_counts(view: &View) -> SkillCols {
    match view.totals {
        Cols::Skill(cols) => cols,
        other => panic!("Skills: {other:?}"),
    }
}

fn round_trip(conv: &Conv) -> Conv {
    let bytes = doc::render_conv(conv);
    let restored = doc::parse_conv(&bytes).expect("valid synthetic conversation document");
    assert_eq!(
        &restored, conv,
        "summary AND resume state survive the codec"
    );
    assert_eq!(doc::render_conv(&restored), bytes);
    restored
}

fn cold(adapter: Adapter, streams: Vec<Stream>) -> Conversation {
    Conversation {
        adapter,
        id: f::C.into(),
        memberships: Vec::new(),
        evidence_complete: true,
        reseated: false,
        streams,
        gaps: Vec::new(),
        read: true,
    }
}

fn native_stream(role: StreamRole, copy: u32, events: &[Event]) -> Stream {
    Stream {
        role,
        copy,
        events: events.to_vec(),
        malformed: 0,
        over_cap: 0,
    }
}

fn ready(conv: &mut Conv) {
    conv.summary.read = true;
}

#[test]
fn p1_summary_resume_round_trip_preserves_orphans_pending_results_and_sticky_conflicts() {
    let s = stream(StreamRole::Parent, 0);
    let events = [
        result(2, false),
        attempt(1, "Read", Some("model-a"), None),
        result(1, false),
        attempt(2, "Bash", Some("model-b"), None),
        result(1, true),
    ];
    let mut conv = Conv::new(Adapter::Claude, f::C);
    for (index, event) in events.iter().enumerate() {
        conv.apply(&s, std::slice::from_ref(event), &Caps::SLICE);
        ready(&mut conv);
        conv = round_trip(&conv);
        let reference = cold(
            Adapter::Claude,
            vec![native_stream(StreamRole::Parent, 0, &events[..=index])],
        );
        assert_eq!(
            view(&conv, Page::Tools),
            fold(&[reference], &ask(Page::Tools), &Caps::SLICE)
        );
    }
    assert_eq!(
        tool_counts(&view(&conv, Page::Tools)),
        ToolCols {
            attempts: 2,
            returned: 2,
            error: 0,
            unresolved: 0
        }
    );
    assert_eq!(view(&conv, Page::Tools).orphans, 0);
    assert!(
        view(&conv, Page::Tools)
            .gaps
            .iter()
            .any(|gap| gap.kind == GapKind::Conflict)
    );
    let before = conv.clone();
    conv.apply(&s, &events, &Caps::SLICE);
    assert_eq!(
        conv, before,
        "replayed evidence must settle, not refold forever"
    );
}

#[test]
fn p1_codex_context_after_return_preserves_attempt_attribution_and_conflict_across_restart() {
    let s = stream(StreamRole::Parent, 0);
    let turn = Key([9; 16]);
    let mut conv = Conv::new(Adapter::Codex, f::C);
    conv.apply(
        &s,
        &[
            attempt(1, "read", None, Some(turn)),
            Event::Result {
                id: Key([1; 16]),
                error: false,
                load: None,
            },
            Event::Mcp {
                id: Key([10; 16]),
                server: Ident::Name("server-a".into()),
                tool: Ident::Name("search".into()),
                outcome: McpOutcome::Success,
                turn: Some(turn),
            },
            Event::Mcp {
                id: Key([11; 16]),
                server: Ident::Name("server-b".into()),
                tool: Ident::Name("search".into()),
                outcome: McpOutcome::Error,
                turn: Some(turn),
            },
        ],
        &Caps::SLICE,
    );
    ready(&mut conv);
    assert_eq!(tool_counts(&view(&conv, Page::Tools)).attempts, 1);
    assert_eq!(view(&conv, Page::Tools).detail[0].model, Cell::Unknown);
    conv = round_trip(&conv);
    let mcp = view(&conv, Page::Mcp);
    assert_eq!(
        mcp.totals,
        Cols::Mcp(McpCols {
            attempts: 2,
            returned: 2,
            success: 1,
            error: 1,
            unresolved: 0
        })
    );
    assert_eq!(
        mcp.rows.len(),
        2,
        "same tool on two servers survives the codec"
    );
    let context = Event::Context {
        turn,
        model: Ident::Name("model-a".into()),
        effort: Ident::Name("high".into()),
    };
    conv.apply(&s, std::slice::from_ref(&context), &Caps::SLICE);
    let v = view(&conv, Page::Tools);
    assert_eq!(
        tool_counts(&v),
        ToolCols {
            attempts: 1,
            returned: 1,
            error: 0,
            unresolved: 0
        }
    );
    assert_eq!(v.detail[0].model, Cell::Value("model-a".into()));
    assert_eq!(v.detail[0].effort, Cell::Value("high".into()));
    conv = round_trip(&conv);
    conv.apply(
        &s,
        &[Event::Context {
            turn,
            model: Ident::Name("model-b".into()),
            effort: Ident::Name("low".into()),
        }],
        &Caps::SLICE,
    );
    let v = view(&conv, Page::Tools);
    assert_eq!(tool_counts(&v).attempts, 1);
    assert_eq!(v.detail[0].model, Cell::Unknown);
    assert_eq!(v.detail[0].effort, Cell::Unknown);
    assert!(v.partial());
}

#[test]
fn p1_late_skill_proof_then_conflicting_load_retracts_inferred_reads_after_round_trip() {
    let s = stream(StreamRole::Parent, 0);
    let directory = Dir([7; 16]);
    let mut conv = Conv::new(Adapter::Claude, f::C);
    let read = [
        attempt(1, "Read", Some("model-a"), None),
        Event::Read {
            id: Key([1; 16]),
            direct: None,
            ancestors: vec![directory],
            over: false,
            skills: false,
        },
        result(1, false),
    ];
    conv.apply(&s, &read, &Caps::SLICE);
    ready(&mut conv);
    assert_eq!(skill_counts(&view(&conv, Page::Skills)).reads, 0);
    conv = round_trip(&conv);
    conv.apply(
        &s,
        &[
            attempt(2, "Skill", Some("model-a"), None),
            Event::Skill {
                id: Key([2; 16]),
                skill: Ident::Name("custom".into()),
            },
            Event::SkillBase {
                id: Key([2; 16]),
                class: ScopeClass::Other,
                resource: Resource([3; 6]),
                dir: directory,
            },
            Event::Result {
                id: Key([2; 16]),
                error: false,
                load: Some(Load::Loaded),
            },
        ],
        &Caps::SLICE,
    );
    assert_eq!(
        skill_counts(&view(&conv, Page::Skills)),
        SkillCols {
            attempts: 1,
            loaded: 1,
            reads: 1,
            ..SkillCols::default()
        }
    );
    conv = round_trip(&conv);
    conv.apply(
        &s,
        &[Event::Result {
            id: Key([2; 16]),
            error: false,
            load: Some(Load::NotLoaded),
        }],
        &Caps::SLICE,
    );
    assert_eq!(
        skill_counts(&view(&conv, Page::Skills)),
        SkillCols {
            attempts: 1,
            unknown: 1,
            ..SkillCols::default()
        }
    );
    assert!(view(&conv, Page::Skills).partial());
}

#[test]
fn p1_split_and_interleaved_copies_and_child_replay_keep_the_hand_count() {
    let mut parent = attempt(1, "Read", Some("model-a"), None);
    if let Event::Attempt { message, .. } = &mut parent {
        *message = Some(Key([4; 16]));
    }
    let original = [parent, result(1, false)];
    let independent = [
        attempt(2, "Bash", Some("child-model"), None),
        result(2, false),
    ];
    for reverse in [false, true] {
        for split in 0..=original.len() {
            let mut conv = Conv::new(Adapter::Claude, f::C);
            let parent = stream(StreamRole::Parent, 0);
            let copy = stream(StreamRole::Parent, 1);
            let child = stream(StreamRole::Child("native-child".into()), 0);
            if reverse {
                conv.apply(&child, &original, &Caps::SLICE);
            }
            conv.apply(&parent, &original[..split], &Caps::SLICE);
            conv = round_trip(&conv);
            conv.apply(&copy, &original, &Caps::SLICE);
            conv.apply(&child, &independent, &Caps::SLICE);
            conv.apply(&parent, &original[split..], &Caps::SLICE);
            if !reverse {
                conv.apply(&child, &original, &Caps::SLICE);
            }
            ready(&mut conv);
            assert_eq!(
                tool_counts(&view(&conv, Page::Tools)),
                ToolCols {
                    attempts: 2,
                    returned: 2,
                    error: 0,
                    unresolved: 0
                }
            );
            let reference = cold(
                Adapter::Claude,
                vec![
                    native_stream(StreamRole::Parent, 0, &original),
                    native_stream(StreamRole::Parent, 1, &original),
                    native_stream(
                        StreamRole::Child("native-child".into()),
                        0,
                        &[original.to_vec(), independent.to_vec()].concat(),
                    ),
                ],
            );
            for page in [Page::Skills, Page::Mcp, Page::Tools] {
                assert_eq!(
                    view(&conv, page),
                    fold(std::slice::from_ref(&reference), &ask(page), &Caps::SLICE)
                );
            }
        }
    }
}

fn status(cursor: u64) -> FileState {
    FileState {
        dev: 1,
        ino: 2,
        len: cursor,
        mtime: (3, 0),
        ctime: (4, 0),
        cursor,
        skipping: false,
        blocks: vec![[5; 32]],
    }
}

fn committed(count: u8, cursor: u64) -> Conv {
    let s = stream(StreamRole::Parent, 0);
    let mut conv = Conv::new(Adapter::Claude, f::C);
    for id in 1..=count {
        conv.apply(
            &s,
            &[attempt(id, "Read", Some("model-a"), None)],
            &Caps::SLICE,
        );
    }
    conv.resume.set_file(s.clone(), status(cursor));
    conv.summary.files.insert(s, incr::Source::Present);
    ready(&mut conv);
    conv
}

#[test]
fn p1_p2_every_publish_crash_prefix_keeps_counters_and_cursor_in_one_generation() {
    let old = committed(1, 100);
    let new = committed(2, 200);
    let old_view = published(&old, 1);
    let new_view = published(&new, 2);
    let mut base = Memory::new();
    base.plant("stats/epoch", b"1\n");
    let seed = store::publish(&mut base, &old, &old_view).expect("first publication plan");
    seed.apply(&mut base, usize::MAX)
        .expect("old state committed");
    let plan = store::publish(&mut base, &new, &new_view).expect("next publication plan");
    let name = store::conv_name(Adapter::Claude, f::C);
    let commits: Vec<_> = plan
        .ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| match op {
            Op::Rename { to, .. } if *to == name => Some(index),
            _ => None,
        })
        .collect();
    assert_eq!(
        commits.len(),
        1,
        "one authoritative conversation commit point"
    );
    for upto in 0..=plan.ops.len() {
        let mut crashed = base.clone();
        plan.apply(&mut crashed, upto)
            .expect("real operation prefix");
        let recovery = store::recovery(&mut crashed).expect("recover interrupted publication");
        recovery
            .apply(&mut crashed, usize::MAX)
            .expect("real recovery plan");
        let expected = if upto > commits[0] { &new } else { &old };
        let restored = match store::load_conv(&mut crashed, Adapter::Claude, f::C) {
            store::Loaded::Ok(conv) => conv,
            other => panic!("readable recovered Conv: {other:?}"),
        };
        assert_eq!(&restored, expected, "cursor and counters at prefix {upto}");
        let view = match store::load_view(&mut crashed) {
            store::Loaded::Ok(view) => view,
            other => panic!("readable recovered view: {other:?}"),
        };
        let expected_view = if upto > commits[0] {
            &new_view
        } else {
            &old_view
        };
        assert_eq!(
            &view, expected_view,
            "all published cells and generation at prefix {upto}"
        );
        // Replaying input after a crash must not manufacture another attempt.
        let mut retry = restored;
        retry.apply(
            &stream(StreamRole::Parent, 0),
            &[
                attempt(1, "Read", Some("model-a"), None),
                attempt(2, "Read", Some("model-a"), None),
            ],
            &Caps::SLICE,
        );
        ready(&mut retry);
        assert_eq!(tool_counts(&self::view(&retry, Page::Tools)).attempts, 2);
    }
}

#[test]
fn p2_codec_rejects_a_future_version_and_torn_or_extra_records() {
    let conv = committed(1, 100);
    let encoded = doc::render_conv(&conv);
    let first_lf = encoded
        .iter()
        .position(|byte| *byte == b'\n')
        .expect("head LF");
    let mut head =
        crate::json::parse(std::str::from_utf8(&encoded[..first_lf]).expect("UTF-8 head"))
            .expect("head JSON");
    let crate::json::Value::Obj(fields) = &mut head else {
        panic!("object head")
    };
    let version = fields
        .iter_mut()
        .find(|(key, _)| key == "tools_conv")
        .expect("version field");
    version.1 = crate::json::Value::Num(2);
    let future = [head.render().as_bytes(), &encoded[first_lf..]].concat();
    assert!(matches!(
        doc::parse_conv(&future),
        Err(doc::DocError::Format(2))
    ));
    assert!(
        doc::parse_conv(&encoded[..=first_lf]).is_err(),
        "head without trailer is torn"
    );
    let extra = [encoded.as_slice(), b"{\"end\":0}\n"].concat();
    assert!(
        doc::parse_conv(&extra).is_err(),
        "a trailer cannot hide later records"
    );
    let view = published(&conv, 1);
    assert_eq!(doc::parse_view(&doc::render_view(&view)), Ok(view));
    let assoc = doc::Assoc {
        sources: std::collections::BTreeMap::default(),
        memberships: std::collections::BTreeMap::default(),
    };
    assert_eq!(doc::parse_assoc(&doc::render_assoc(&assoc)), Ok(assoc));
    assert_eq!(doc::TOOLS_FORMAT, 1);
}
