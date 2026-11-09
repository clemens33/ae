#![no_main]

use ae::tool_stats::incr::{self, Conv, Overflow, Published, StreamKey};
use ae::tool_stats::{Ask, Caps, Filter, GapKind, Page, Scope, StreamRole, doc};
use libfuzzer_sys::fuzz_target;

/// Every page of `view` over all conversations and over each session's: an
/// accepted document is consumed without a panic or an overflow.
fn consume(view: &Published) {
    let mut scopes = vec![Scope::All];
    for conv in &view.convs {
        scopes.extend(conv.sessions.iter().cloned().map(Some).map(Scope::This));
    }
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        for scope in &scopes {
            let ask = Ask {
                page,
                scope: scope.clone(),
                filter: Filter::All,
            };
            let all = incr::combine(view, &ask, &Caps::SLICE);
            for filter in all.filters.iter().skip(1) {
                let ask = Ask {
                    filter: filter.clone(),
                    ..ask.clone()
                };
                std::hint::black_box(incr::combine(view, &ask, &Caps::SLICE));
            }
        }
    }
}

/// `conv` published alone, or `None` when that view would not add up.
fn alone(conv: &Conv) -> Option<Published> {
    let mut view = Published {
        generation: 1,
        convs: Vec::new(),
        over_cap: 0,
    };
    view.admit(incr::publish(&conv.summary, None)).ok()?;
    Some(view)
}

/// An update either holds or routes the conversation to a rebuild: no
/// counted row is left behind it.
fn updated(conv: &mut Conv, outcome: Result<(), Overflow>) {
    if outcome.is_err() {
        assert!(conv.summary.rows.is_empty(), "rebuilt from the sources");
    }
    if let Some(view) = alone(conv) {
        consume(&view);
    }
}

/// Small live updates of an accepted conversation, as appends and a new
/// discovery round make them: each holds exactly or routes to a rebuild.
fn mutate(mut conv: Conv) {
    let streams: Vec<StreamKey> = conv.summary.files.keys().cloned().collect();
    let child = StreamKey {
        role: StreamRole::Child("fuzz".into()),
        copy: [7; 16],
    };
    for stream in streams.iter().chain([&child]) {
        let outcome = conv.note(stream, 2, 1);
        updated(&mut conv, outcome);
    }
    let outcome = conv.set_gaps(vec![
        GapKind::Malformed,
        GapKind::OverCap,
        GapKind::Malformed,
    ]);
    updated(&mut conv, outcome);
}

// Hostile bytes as each `stats/tools/` document: no panic, a document that
// parses re-renders to exactly its own bytes (canonical form only), and an
// accepted conversation or view is consumed safely, as loaded and after
// small live updates.
fuzz_target!(|data: &[u8]| {
    let Some((&kind, bytes)) = data.split_first() else {
        return;
    };
    match kind % 6 {
        0 => {
            if let Ok(conv) = doc::parse_conv(bytes) {
                assert_eq!(doc::render_conv(&conv), bytes);
                if let Some(view) = alone(&conv) {
                    consume(&view);
                }
                mutate(conv);
            }
        }
        1 => {
            if let Ok(view) = doc::parse_view(bytes) {
                assert_eq!(doc::render_view(&view), bytes);
                consume(&view);
                // The decoded view itself: each of its conversations admitted
                // again replaces itself in place, and admitted in any order
                // into a fresh view makes the same view.
                let mut again = view.clone();
                let mut fresh = Published {
                    generation: view.generation,
                    convs: Vec::new(),
                    over_cap: view.over_cap,
                };
                for one in view.convs.iter().rev() {
                    assert_eq!(
                        again.admit(one.clone()),
                        Ok(()),
                        "a part of an accepted view"
                    );
                    assert_eq!(
                        fresh.admit(one.clone()),
                        Ok(()),
                        "a part of an accepted view"
                    );
                }
                assert_eq!(again, view, "admit replaces in place");
                assert_eq!(fresh, view, "one order, each conversation once");
                // A changed conversation replaces its entry: still a canonical
                // view of the same conversations.
                for one in &view.convs {
                    let mut newer = one.clone();
                    newer.summary.read = !newer.summary.read;
                    assert_eq!(again.admit(newer), Ok(()), "the counts are unchanged");
                }
                assert_eq!(again.convs.len(), view.convs.len());
                assert!(doc::parse_view(&doc::render_view(&again)).is_ok());
                consume(&again);
                // Twins under the other harness may not add up.
                for one in &view.convs {
                    let mut twin = one.clone();
                    twin.summary.adapter = match twin.summary.adapter {
                        ae::tool_stats::Adapter::Claude => ae::tool_stats::Adapter::Codex,
                        ae::tool_stats::Adapter::Codex => ae::tool_stats::Adapter::Claude,
                    };
                    let _ = again.admit(twin);
                }
                consume(&again);
            }
        }
        2 => {
            if let Ok(assoc) = doc::parse_assoc(bytes) {
                assert_eq!(doc::render_assoc(&assoc), bytes);
            }
        }
        3 => {
            if let Ok(intent) = doc::parse_intent(bytes) {
                assert_eq!(doc::render_intent(&intent), bytes);
            }
        }
        4 => {
            if let Ok(sources) = doc::parse_sources(bytes) {
                assert_eq!(doc::render_sources(&sources), bytes);
            }
        }
        _ => {
            if let Ok(coverage) = doc::parse_coverage(bytes) {
                assert_eq!(doc::render_coverage(&coverage), bytes);
                // Applied to any page it adds only path-free lines.
                let ask = Ask {
                    page: Page::Tools,
                    scope: Scope::All,
                    filter: Filter::All,
                };
                let mut view = incr::combine(
                    &Published {
                        generation: coverage.generation,
                        convs: Vec::new(),
                        over_cap: 0,
                    },
                    &ask,
                    &Caps::SLICE,
                );
                coverage.apply(&mut view, &ask);
                assert!(view.gaps.iter().all(|gap| gap.conversation.is_none()));
            }
        }
    }
});
