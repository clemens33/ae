#![no_main]

use libfuzzer_sys::fuzz_target;

// The dictation client's stdout is tmux's control-mode stream: hostile input
// that ae parses while it holds a recording. The first byte picks up to four
// expectations queued after the attach, the second the chunk size the stream
// arrives in. Chunks must give the same events as the whole stream, nothing
// may follow a terminal event or a finish, each other event answers the
// expectation queued for it in order, and a capture holds exactly the rows
// that the facts block right before it named. The same bytes then go through
// the one-shot hook and pane-proof readers, which must not panic.
fuzz_target!(|data: &[u8]| {
    use ae::tmux::control::{
        ControlEvent, ControlParser, Expect, hooks_eligible, interpret_pane_proof,
        interpret_sidecar_hooks,
    };

    let [schedule, size, stream @ ..] = data else {
        return;
    };
    let kinds = [Expect::Empty, Expect::Facts, Expect::Capture, Expect::Empty];
    let queued: Vec<Expect> = (0..=usize::from(schedule & 3))
        .map(|at| kinds[(usize::from(*schedule) >> (2 + 2 * at)) & 3])
        .collect();
    let parse = |chunk: usize| {
        let mut parser = ControlParser::new();
        for expect in &queued {
            parser.expect(*expect);
        }
        let mut events = Vec::new();
        for piece in stream.chunks(chunk) {
            events.extend(parser.feed(piece));
        }
        events.extend(parser.finish());
        assert!(
            parser.feed(b"%exit\n").is_empty() && parser.finish().is_empty(),
            "a finished parser reads nothing more"
        );
        events
    };
    let whole = parse(stream.len().max(1));
    assert_eq!(
        parse(usize::from(size % 16) + 1),
        whole,
        "the chunk size changed the events"
    );

    let mut expects = std::iter::once(Expect::Attach).chain(queued.iter().copied());
    let mut facts_height = None;
    for (at, event) in whole.iter().enumerate() {
        if event.is_terminal() {
            assert_eq!(at + 1, whole.len(), "an event followed a terminal one");
            continue;
        }
        let Some(expect) = expects.next() else {
            panic!("{event:?} answers no queued expectation");
        };
        let height = facts_height.take();
        match (expect, event) {
            (Expect::Attach, ControlEvent::Attached) | (Expect::Empty, ControlEvent::Empty) => {}
            (Expect::Facts, ControlEvent::Facts(facts)) => facts_height = Some(facts.height),
            (Expect::Capture, ControlEvent::Capture(screen)) => {
                let Some(rows) = height else {
                    panic!("a capture without the facts block right before it");
                };
                let drawn = if screen.is_empty() && rows <= 1 {
                    usize::from(rows)
                } else {
                    screen.split('\n').count()
                };
                assert_eq!(drawn, usize::from(rows), "the capture's rows");
                assert!(screen.len() <= ControlParser::MAX_BODY);
            }
            (expect, event) => panic!("{event:?} answers {expect:?}"),
        }
    }

    let text = String::from_utf8_lossy(stream);
    if let Some(hooks) = interpret_sidecar_hooks(&text) {
        for (name, session, global) in hooks {
            let _ = std::hint::black_box(hooks_eligible(name, &session, &global));
        }
    }
    for succeeded in [true, false] {
        let _ = std::hint::black_box(interpret_pane_proof(succeeded, &text, "$1", "%2"));
    }
});
