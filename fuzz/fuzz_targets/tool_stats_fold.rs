#![no_main]

use ae::meta::RecordedClient;
use ae::tool_stats::{
    Adapter, Ask, Caps, Conversation, Filter, Membership, Page, Role, Scope, Stream, StreamRole,
    View, fold,
};
use ae::usage::{claude_calls, codex_calls};
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;

/// Lines go to one of three conversations and one of two streams by their
/// first byte; the rest of the line is the native record.
fn conversations(data: &[u8]) -> Vec<Conversation> {
    let config = [PathBuf::from("/r/config")];
    let work = [PathBuf::from("/r/work")];
    let roots = claude_calls::Roots {
        config_homes: &config,
        work_dirs: &work,
    };
    let mut out: Vec<Conversation> = (0..3_u8)
        .map(|index| Conversation {
            adapter: if index == 2 {
                Adapter::Codex
            } else {
                Adapter::Claude
            },
            id: format!("conversation-{index}"),
            memberships: vec![Membership {
                session: Some(format!("session-{}", index % 2)),
                slot: "main".into(),
                role: Role::Live,
                client: RecordedClient::Label(format!("client-{}", index % 2)),
            }],
            evidence_complete: true,
            reseated: false,
            streams: Vec::new(),
            gaps: Vec::new(),
            read: true,
        })
        .collect();
    for line in data.split(|byte| *byte == b'\n') {
        let Some((&route, text)) = line.split_first() else {
            continue;
        };
        let conversation = &mut out[usize::from(route % 3)];
        let role = if route & 0x10 == 0 {
            StreamRole::Parent
        } else {
            StreamRole::Child("child".into())
        };
        let copy = u32::from(route >> 7);
        let index = match conversation
            .streams
            .iter()
            .position(|stream| stream.role == role && stream.copy == copy)
        {
            Some(index) => index,
            None => {
                conversation.streams.push(Stream {
                    role,
                    copy,
                    events: Vec::new(),
                    malformed: 0,
                    over_cap: 0,
                });
                conversation.streams.len() - 1
            }
        };
        let stream = &mut conversation.streams[index];
        let parsed = match conversation.adapter {
            Adapter::Claude => claude_calls::line(text, &roots, &mut stream.events),
            Adapter::Codex => codex_calls::line(text, &mut stream.events),
        };
        if parsed == ae::tool_stats::Parsed::Malformed {
            stream.malformed += 1;
        }
    }
    out
}

fn check(view: &View, caps: &Caps) {
    assert!(view.rows.len() <= caps.rows);
    assert!(view.detail.len() <= caps.detail);
    let shown: u64 = view.rows.iter().map(|row| row.cols.attempts()).sum();
    let hidden = view.more.map_or(0, |more| more.attempts);
    assert_eq!(
        shown + hidden,
        view.totals.attempts(),
        "rows cover the totals"
    );
    let detail: u64 = view.detail.iter().map(|row| row.attempts).sum();
    let hidden = view.detail_more.map_or(0, |more| more.attempts);
    assert_eq!(
        detail + hidden,
        view.totals.attempts(),
        "Detail covers the totals"
    );
    assert_eq!(view.filters.first(), Some(&Filter::All));
    assert!(view.filters.windows(2).all(|pair| pair[0] < pair[1]));
    for pair in view.rows.windows(2) {
        assert!(pair[0].cols.attempts() >= pair[1].cols.attempts());
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&shape, rest)) = data.split_first() else {
        return;
    };
    let small = usize::from(shape % 8) + 1;
    // Low room sweeps 4 KiB to 512 KiB over the reserve: one bare call is
    // refused at the bottom and admitted inside it (driver test
    // n2_fuzz_room_sweep_brackets_one_call_admission), so partial
    // admission is exercised.
    let room = 1_usize << (12 + usize::from((shape >> 3) & 7));
    let caps = Caps {
        ledger: if shape & 0x80 == 0 {
            Caps::SLICE.ledger
        } else {
            ae::tool_stats::VIEW_RESERVE + room
        },
        rows: small,
        detail: small,
        per_conversation: if shape & 0x40 == 0 { 64 } else { 4 },
        idents: small,
        turns: small,
        ..Caps::SLICE
    };
    let input = conversations(rest);
    let mut reversed = input.clone();
    reversed.reverse();
    for conversation in &mut reversed {
        for stream in &mut conversation.streams {
            stream.events.reverse();
        }
        conversation.streams.reverse();
    }
    let doubled: Vec<Conversation> = input.iter().chain(&input).cloned().collect();
    // An overlapping split feed (plan §7: any split point): every
    // conversation once with each stream's first half only, then whole.
    let halves: Vec<Conversation> = input
        .iter()
        .cloned()
        .map(|mut conversation| {
            for stream in &mut conversation.streams {
                stream.events.truncate(stream.events.len() / 2);
            }
            conversation
        })
        .chain(input.iter().cloned())
        .collect();
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        for scope in [Scope::All, Scope::This(Some("session-1".into()))] {
            let ask = Ask {
                page,
                scope,
                filter: Filter::All,
            };
            let view = fold(&input, &ask, &caps);
            check(&view, &caps);
            assert_eq!(fold(&reversed, &ask, &caps), view, "order changes nothing");
            assert_eq!(
                fold(&doubled, &ask, &caps),
                view,
                "duplicates change nothing"
            );
            assert_eq!(
                fold(&halves, &ask, &caps),
                view,
                "an overlapping split feed changes nothing"
            );
            for filter in view.filters.iter().skip(1) {
                let ask = Ask {
                    filter: filter.clone(),
                    ..ask.clone()
                };
                let filtered = fold(&input, &ask, &caps);
                check(&filtered, &caps);
                assert!(filtered.totals.attempts() <= view.totals.attempts());
            }
        }
    }
});
