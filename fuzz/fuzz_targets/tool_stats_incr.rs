#![no_main]

use ae::meta::RecordedClient;
use ae::tool_stats::incr::{self, Association, Conv, Published, StreamKey};
use ae::tool_stats::{
    Adapter, Ask, Caps, Conversation, Filter, Membership, Page, Role, Scope, Stream, StreamRole,
    View, doc, fold,
};
use ae::usage::{claude_calls, codex_calls};
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;

/// As `tool_stats_fold`: lines go to one of three conversations and one of
/// two streams and copies by their first byte; the rest is the native record.
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

fn key(stream: &Stream) -> StreamKey {
    StreamKey {
        role: stream.role.clone(),
        copy: [u8::try_from(stream.copy).unwrap_or(u8::MAX); 16],
    }
}

/// One conversation applied whole, stream by stream.
fn whole(conversation: &Conversation, caps: &Caps) -> Conv {
    let mut conv = Conv::new(conversation.adapter, &conversation.id);
    for stream in &conversation.streams {
        conv.apply(&key(stream), &stream.events, caps);
        assert_eq!(
            conv.note(&key(stream), stream.malformed, stream.over_cap),
            Ok(()),
            "real line counts add up"
        );
    }
    conv.summary.read = conversation.read;
    conv
}

/// The same conversation applied in input-chosen chunks, the streams
/// interleaved, a codec round trip between the halves, then a replay.
fn incremental(conversation: &Conversation, split: u8, caps: &Caps) -> Conv {
    let mut conv = Conv::new(conversation.adapter, &conversation.id);
    let cut = |stream: &Stream, at: usize| {
        let quarter = usize::from((split >> (2 * (at % 4))) & 3);
        stream.events.len() * quarter / 3
    };
    for (at, stream) in conversation.streams.iter().enumerate() {
        let end = cut(stream, at).min(stream.events.len());
        for chunk in stream.events[..end].chunks(2) {
            conv.apply(&key(stream), chunk, caps);
        }
    }
    let bytes = doc::render_conv(&conv);
    assert_eq!(
        doc::parse_conv(&bytes).as_ref(),
        Ok(&conv),
        "summary and resume survive the codec"
    );
    for (at, stream) in conversation.streams.iter().enumerate().rev() {
        let end = cut(stream, at).min(stream.events.len());
        conv.apply(&key(stream), &stream.events[end..], caps);
        assert_eq!(
            conv.note(&key(stream), stream.malformed, stream.over_cap),
            Ok(()),
            "real line counts add up"
        );
    }
    conv.summary.read = conversation.read;
    if let Some(stream) = conversation.streams.first() {
        let before = conv.clone();
        conv.apply(&key(stream), &stream.events, caps);
        assert_eq!(conv, before, "replayed evidence settles");
    }
    conv
}

/// The view as the reader publishes it: each conversation admitted, in any
/// order, real counts always adding up.
fn published(input: &[Conversation], convs: &[Conv]) -> Published {
    let mut view = Published {
        generation: 1,
        convs: Vec::new(),
        over_cap: 0,
    };
    for (conversation, conv) in input.iter().zip(convs).rev() {
        let one = incr::publish(
            &conv.summary,
            Some(&Association {
                memberships: conversation.memberships.clone(),
                evidence_complete: conversation.evidence_complete,
                reseated: conversation.reseated,
            }),
        );
        assert_eq!(view.admit(one), Ok(()), "real counts add up");
    }
    view
}

fn same(persisted: &View, cold: &View) {
    // Exact under the identity cap too: a proven directory's row is chosen
    // after the cap maps its proofs' names, as the fold chooses it.
    assert_eq!(persisted, cold, "persisted == cold");
}

fuzz_target!(|data: &[u8]| {
    let Some((&shape, rest)) = data.split_first() else {
        return;
    };
    let Some((&split, rest)) = rest.split_first() else {
        return;
    };
    let small = usize::from(shape % 8) + 1;
    let caps = Caps {
        rows: small,
        detail: small,
        per_conversation: if shape & 0x40 == 0 { 64 } else { 4 },
        idents: small,
        turns: small,
        ..Caps::SLICE
    };
    let input = conversations(rest);
    let convs: Vec<Conv> = input
        .iter()
        .map(|conversation| {
            let conv = incremental(conversation, split, &caps);
            assert_eq!(conv, whole(conversation, &caps), "any split and order");
            conv
        })
        .collect();
    let published = published(&input, &convs);
    assert_eq!(
        doc::parse_view(&doc::render_view(&published)).as_ref(),
        Ok(&published),
        "the view survives the codec"
    );
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        for scope in [Scope::All, Scope::This(Some("session-1".into()))] {
            let ask = Ask {
                page,
                scope,
                filter: Filter::All,
            };
            let cold = fold(&input, &ask, &caps);
            same(&incr::combine(&published, &ask, &caps), &cold);
            for filter in cold.filters.iter().skip(1) {
                let ask = Ask {
                    filter: filter.clone(),
                    ..ask.clone()
                };
                same(
                    &incr::combine(&published, &ask, &caps),
                    &fold(&input, &ask, &caps),
                );
            }
        }
    }
});
