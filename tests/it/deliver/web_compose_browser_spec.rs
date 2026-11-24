//! S3a browser acceptance. One live listener and fake seat deliver both lines.

use std::time::{Duration, Instant};

use ae::json::Value;

use super::{
    Cell, Checked, Failed, PAGE_WAIT, READY, Rig, SEND_WAIT, Tab, VIEW, ok, smoke_rig,
    start_listener,
};

const DESK: &str = r#".col .decision-answer[data-item="D67"]"#;
const CARD: &str = r#".card .decision-answer[data-item="D67"]"#;
const TOPIC: &str = ".new-topic";
const QUESTION: &str = "Choose <img src=x onerror=alert(1)> #smoke";
const REPLACEMENT: &str = "Replacement question #smoke";
const TITLE: &str = "Übergabe | #note";
const SLUG: &str = "uebergabe-note";
const TOPIC_LINE: &str = "#uebergabe-note new Übergabe | #note";
const CASE: &str = "s3a-compose";
const INSTALL: &str = include_str!("web_compose_browser_fixture.js");

fn script(tab: &mut Tab, check: &'static str, code: &str) -> Checked {
    tab.eval(code)
        .map(drop)
        .map_err(|why| Failed::new(check, why))
}

fn wait(tab: &mut Tab, check: &'static str, body: &str, want: &str) -> Checked {
    tab.until(check, body, want, PAGE_WAIT).map(drop)
}

fn edit(tab: &mut Tab, which: &str, field: &str, value: &str) -> Checked {
    let value = Value::str(value).render();
    script(
        tab,
        CASE,
        &format!("__s3.set('{which}', '{field}', {value})"),
    )
}

fn click(tab: &mut Tab, check: &'static str, root: &str, action: &str) -> Checked {
    tab.click_selector(check, &format!(r#"{root} button[data-action="{action}"]"#))
}

fn open_card(tab: &mut Tab, check: &'static str) -> Checked {
    tab.click(check, "row")?;
    wait(
        tab,
        check,
        "const saw = document.querySelector('.card .sub')?.textContent; \
         return {ok: saw?.startsWith('#smoke '), saw};",
        "",
    )
}

fn publish_board(rig: &Rig, question: &str) {
    let input = format!("DECIDE\nD67 | {question}\n");
    let items = ae::desk::parse_input(input.as_bytes()).expect("S3a browser fixture board");
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "tui",
    )
    .expect("publish S3a browser fixture board");
}

fn measure(tab: &mut Tab, root: &str, action: &str, field: &str) -> Checked {
    let args = format!(
        "{}, {}, {}",
        Value::str(root).render(),
        Value::str(action).render(),
        Value::str(field).render()
    );
    tab.holds(
        "s3a-phone-controls",
        &format!(
            "const [scope, action, field] = [{args}]; const root = document.querySelector(scope); \
             const b = root?.querySelector('button[data-action=' + action + ']'); \
             const f = root?.querySelector('input[data-field=' + field + ']'); \
             const r = b?.getBoundingClientRect(), i = f?.getBoundingClientRect(); \
             const fs = f ? parseFloat(getComputedStyle(f).fontSize) : 0; \
             const wide = document.documentElement.scrollWidth; \
             const saw = {{button: r?.toJSON(), input: i?.toJSON(), fs, wide, innerWidth}}; \
             return {{ok: !!r && !!i && r.width >= 44 && r.height >= 44 && \
             i.width >= 44 && i.height >= 44 && fs >= 16 && wide <= innerWidth, saw}};"
        ),
        "",
    )
}

fn phone(tab: &mut Tab, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-phone-controls";
    tab.view(cell);
    tab.call(
        "Emulation.setTouchEmulationEnabled",
        Value::obj([
            ("enabled", Value::Bool(true)),
            ("maxTouchPoints", Value::Num(1)),
        ]),
    );
    tab.click(CHECK, "topics")?;
    wait(tab, CHECK, VIEW, "view-topics")?;
    measure(tab, DESK, "answer", "answer")?;
    measure(tab, TOPIC, "new-topic", "title")?;
    open_card(tab, CHECK)?;
    measure(tab, CARD, "answer", "answer")?;
    tab.click_selector(CHECK, ".card button[aria-label='Close topic']")?;
    ok(CHECK, cell);
    Ok(())
}

fn controls_and_parser_parity(tab: &mut Tab) -> Checked {
    const CHECK: &str = "s3a-parser-parity";
    edit(tab, "topic", "title", "Übergabe Mac mini")?;
    tab.holds(
        CHECK,
        "const saw = __s3.preview(); return {ok: saw === want, saw};",
        "#uebergabe-mac-mini new Übergabe Mac mini",
    )?;
    click(tab, CHECK, TOPIC, "edit-slug")?;

    let slugs = [
        "".to_owned(),
        "x".repeat(48),
        "x".repeat(49),
        "1".to_owned(),
        "one-".to_owned(),
        "two--parts".to_owned(),
        "-front".to_owned(),
        "Upper".to_owned(),
        "ümlaut".to_owned(),
        "a_b".to_owned(),
        "a.b".to_owned(),
        "a b".to_owned(),
    ];
    for slug in slugs {
        edit(tab, "topic", "slug", &slug)?;
        tab.verdict(
            CHECK,
            "await __s3.press('topic', 'new-topic'); const saw = !!__s3.button('topic', 'undo'); \
             if (saw) await __s3.press('topic', 'undo'); return {ok: saw === (want === 'true'), saw};",
            if ae::topic::is_slug(&slug) { "true" } else { "false" },
        ).map_err(|why| Failed::new(CHECK, format!("slug {slug:?}: {}", why.detail)))?;
    }

    // A real Closed row is an occupied slug, even though the desk has no
    // open work for it. Slug parity above uses values absent from the desk.
    edit(tab, "topic", "slug", "s2-closed")?;
    tab.holds(CHECK,
        "await __s3.press('topic', 'new-topic'); const saw = !!__s3.button('topic', 'undo');          return {ok: !saw, saw};", "")?;
    edit(tab, "topic", "slug", "s3-parity-only")?;

    for (which, field, action, limit) in [
        ("desk", "answer", "answer", 120),
        ("card", "answer", "answer", 120),
        ("topic", "title", "new-topic", 80),
    ] {
        let cases = [
            ("🧭".repeat(limit), true),
            ("🧭".repeat(limit + 1), false),
            ("".to_owned(), false),
            (" \u{3000}".to_owned(), false),
            ("\u{feff}one\u{feff}".to_owned(), true),
            ("one\u{0085}two".to_owned(), false),
            ("one\u{007f}two".to_owned(), false),
            ("one\u{2028}two".to_owned(), false),
            ("one\u{2029}two".to_owned(), false),
            ("one\u{00a0}two".to_owned(), true),
            ("\u{00a0}\u{2003}trim me\u{202f}\u{3000}".to_owned(), true),
        ];
        for (input, accepted) in cases {
            edit(tab, which, field, &input)?;
            let shown = input.trim();
            let expectation = Value::str(shown).render();
            let body = format!(
                "const button = __s3.button('{which}', '{action}'); \
                 const available = '{field}' !== 'answer' || (!!button && button.disabled === !{accepted}); \
                 await __s3.press('{which}', '{action}'); \
                 const saw = !!__s3.button('{which}', 'undo'); const status = __s3.status('{which}'); \
                 if (saw) await __s3.press('{which}', 'undo'); \
                 const bounded = !saw || status.includes({expectation}); \
                 return {{ok: available && saw === {accepted} && bounded, saw: {{available, saw, status}}}};"
            );
            tab.holds(CHECK, &body, "")
                .map_err(|why| Failed::new(CHECK, format!("{field} {input:?}: {}", why.detail)))?;
        }

        edit(tab, which, field, "original")?;
        for invalid in ["one\ntwo", "one\u{0001}two", "one\u{2028}two"] {
            let invalid = Value::str(invalid).render();
            tab.holds(
                "s3a-paste",
                &format!(
                    "const cancelled = __s3.paste('{which}', '{field}', {invalid}); \
                     await new Promise(done => setTimeout(done, 0)); \
                     const value = __s3.field('{which}', '{field}')?.value; \
                     const status = __s3.status('{which}'); \
                     return {{ok: cancelled && value === 'original' && /paste cancelled/i.test(status), \
                     saw: {{cancelled, value, status}}}};"
                ),
                "",
            )?;
        }
    }
    no_requests(tab, CHECK)
}

fn no_requests(tab: &mut Tab, check: &'static str) -> Checked {
    tab.holds(
        check,
        "const saw = {nonce: __s3.nonce, posts: __s3.posts.length}; \
         return {ok: saw.nonce === 0 && saw.posts === 0, saw};",
        "",
    )
}

fn no_carry_for(tab: &mut Tab, rig: &Rig, check: &'static str, until: Instant) -> Checked {
    if let Some(remaining) = until.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining);
    }
    no_requests(tab, check)?;
    assert_eq!(
        rig.enter_count(),
        0,
        "an undone or cancelled draft reached the seat"
    );
    Ok(())
}

fn both_buttons(tab: &mut Tab, check: &'static str, enabled: bool) -> Checked {
    wait(
        tab,
        check,
        "const a = __s3.button('desk', 'answer'), b = __s3.button('card', 'answer'); \
         const saw = [a?.disabled, b?.disabled]; \
         return {ok: !!a && !!b && saw.every(v => v === (want === 'disabled')), saw};",
        if enabled { "enabled" } else { "disabled" },
    )
}

fn enable_aligned_answers(tab: &mut Tab, check: &'static str) -> Checked {
    edit(tab, "desk", "answer", "1")?;
    edit(tab, "card", "answer", "1")?;
    both_buttons(tab, check, true)
}

fn undo_and_hide(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-undo";
    enable_aligned_answers(tab, CHECK)?;
    let until = Instant::now() + Duration::from_secs(7);
    click(tab, CHECK, DESK, "answer")?;
    both_buttons(tab, CHECK, false)?;
    click(tab, CHECK, DESK, "undo")?;
    enable_aligned_answers(tab, CHECK)?;
    edit(tab, "topic", "title", TITLE)?;
    if tab
        .verdict(
            CHECK,
            "return {ok: !!__s3.field('topic','slug'), saw: 'manual field'};",
            "",
        )
        .is_ok()
    {
        edit(tab, "topic", "slug", SLUG)?;
    }
    let topic_until = Instant::now() + Duration::from_secs(7);
    click(tab, CHECK, TOPIC, "new-topic")?;
    tab.holds(
        CHECK,
        "return {ok: !__s3.chip(want), saw: __s3.status('topic')};",
        SLUG,
    )?;
    click(tab, CHECK, TOPIC, "undo")?;
    no_carry_for(tab, rig, CHECK, until.max(topic_until))?;
    ok(CHECK, cell);

    const HIDDEN: &str = "s3a-hidden";
    edit(tab, "desk", "answer", "1")?;
    let until = Instant::now() + Duration::from_secs(7);
    click(tab, HIDDEN, DESK, "answer")?;
    script(tab, HIDDEN, "__s3.hidden(true)")?;
    wait(
        tab,
        HIDDEN,
        "const saw = __s3.status('desk'); return {ok: /cancelled/i.test(saw), saw};",
        "",
    )?;
    script(tab, HIDDEN, "__s3.hidden(false)")?;
    no_carry_for(tab, rig, HIDDEN, until)?;
    ok(HIDDEN, cell);
    Ok(())
}

fn card_skew(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-card-skew";
    enable_aligned_answers(tab, CHECK)?;
    let literal = "const a = __s3.root('desk'), b = __s3.root('card');                    const saw = [a?.textContent, b?.textContent];                    return {ok: saw.every(x => x?.includes(want)) &&                    !a.querySelector('img') && !b.querySelector('img'), saw};";
    tab.holds(CHECK, literal, QUESTION)?;

    for skew in [REPLACEMENT, "missing", QUESTION] {
        if skew == QUESTION {
            publish_board(rig, REPLACEMENT);
            wait(
                tab,
                CHECK,
                "const saw = __s3.root('desk')?.textContent; return {ok: saw?.includes(want), saw};",
                REPLACEMENT,
            )?;
        }
        let value = Value::str(skew).render();
        script(tab, CHECK, &format!("__s3.skew = {value}"))?;
        tab.click_selector(CHECK, ".card button[aria-label='Close topic']")?;
        open_card(tab, CHECK)?;
        let selector = if skew == "missing" {
            r#".card .decision-answer[data-item="D98"]"#
        } else {
            CARD
        };
        let selector = Value::str(selector).render();
        let body = format!(
            "const root = document.querySelector({selector}); \
             const field = root?.querySelector('[data-field=answer]'); \
             if (field && !field.disabled && !field.readOnly && field.value !== '1') {{ \
               field.value = '1'; field.dispatchEvent(new InputEvent('input', {{bubbles: true}})); \
               await new Promise(done => setTimeout(done, 0)); \
             }} \
             const button = root?.querySelector('[data-action=answer]'); \
             const saw = root?.textContent; const frozen = !!button && button.disabled; \
             if (frozen) button.click(); \
             return {{ok: frozen && saw?.includes(want) && __s3.posts.length === 0, saw}};"
        );
        wait(
            tab,
            CHECK,
            &body,
            if skew == "missing" { QUESTION } else { skew },
        )?;
    }
    script(tab, CHECK, "__s3.skew = null")?;
    ae::store::open(&rig.dir)
        .append_memo(b"2026-11-22T10:10:00Z\ttui\ttopic-smoke\twhat: Refresh the open card\n")
        .expect("publish a real snapshot change to refresh the open card");
    wait(tab, CHECK, literal, REPLACEMENT)?;
    enable_aligned_answers(tab, CHECK)?;
    ok(CHECK, cell);
    Ok(())
}

fn cancel_on_board_poll(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-board-poll";
    edit(tab, "desk", "answer", "1")?;
    let until = Instant::now() + Duration::from_secs(7);
    click(tab, CHECK, DESK, "answer")?;
    tab.holds(
        CHECK,
        "__s3.pollBefore = __s3.stateCount; __s3.wake(); return {ok: true};",
        "",
    )?;
    wait(
        tab,
        CHECK,
        "const saw = __s3.stateCount; return {ok: saw > __s3.pollBefore, saw};",
        "",
    )?;
    tab.holds(
        CHECK,
        "const saw = __s3.status('desk'); return {ok: !!__s3.button('desk','undo'), saw};",
        "",
    )?;

    publish_board(rig, QUESTION);
    wait(
        tab,
        CHECK,
        "const saw = __s3.status('desk'); return {ok: /the decision changed/i.test(saw), saw};",
        "",
    )?;
    no_carry_for(tab, rig, CHECK, until)?;
    enable_aligned_answers(tab, CHECK)?;
    ok(CHECK, cell);
    Ok(())
}

#[allow(
    clippy::disallowed_methods,
    reason = "read the test rig's own route fixture"
)]
fn change_route(rig: &Rig, before: &str, after: &str) {
    let path = rig.dir.join("meta");
    let original = std::fs::read_to_string(&path).expect("read fixture route");
    assert_eq!(original.matches(before).count(), 1);
    let next = rig.dir.join("fixture-meta-s3a-next");
    std::fs::write(&next, original.replacen(before, after, 1)).expect("write fixture route");
    std::fs::rename(next, path).expect("publish fixture route");
}

fn route_moves(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-route";
    edit(tab, "desk", "answer", "1")?;
    edit(tab, "topic", "title", TITLE)?;
    let until = Instant::now() + Duration::from_secs(7);
    for (root, action) in [(DESK, "answer"), (TOPIC, "new-topic")] {
        click(tab, CHECK, root, action)?;
    }
    change_route(rig, "seat.worker.0=side-old", "seat.worker.0=side-s3a");
    wait(
        tab,
        CHECK,
        "const saw = ['desk','topic'].map(w => __s3.status(w)); \
         return {ok: saw.every(x => /the orchestrator changed/i.test(x)), saw};",
        "",
    )?;
    no_carry_for(tab, rig, CHECK, until)?;

    script(
        tab,
        CHECK,
        "__s3.reset(); __s3.oldRoute = __s3.state.route.id; __s3.holdState = true;",
    )?;
    edit(tab, "desk", "answer", "1")?;
    edit(tab, "topic", "title", TITLE)?;
    for (root, action) in [(DESK, "answer"), (TOPIC, "new-topic")] {
        click(tab, CHECK, root, action)?;
    }
    change_route(rig, "seat.worker.0=side-s3a", "seat.worker.0=side-old");
    wait(
        tab,
        CHECK,
        "const saw = ['desk','topic'].map(w => __s3.status(w)); \
         return {ok: saw.every(x => /the orchestrator changed/i.test(x)), saw};",
        "",
    )?;
    tab.holds(
        CHECK,
        "const bodies = __s3.posts.map(p => JSON.parse(p.body)); __s3.spent = bodies.map(b => b.nonce); \
         const saw = {nonce: __s3.nonce, bodies, status: __s3.lastStatus}; \
         return {ok: saw.nonce === 2 && bodies.length === 2 && saw.status === 409 && \
         bodies.every(b => b.route === __s3.oldRoute) && bodies.some(b => b.text === 'D67 1') && \
         bodies.some(b => b.text === want), saw};",
        TOPIC_LINE,
    )?;
    assert_eq!(rig.enter_count(), 0, "a stale route reached the seat");
    script(tab, CHECK, "__s3.holdState = false; __s3.wake()")?;
    wait(
        tab,
        CHECK,
        "const saw = __s3.state.route.id; return {ok: saw !== __s3.oldRoute, saw};",
        "",
    )?;
    ok(CHECK, cell);
    Ok(())
}

fn check_again(tab: &mut Tab, rig: &Rig, cell: Cell, decision: bool) -> Checked {
    let (check, root, which, field, input, action, line, count) = if decision {
        (
            "s3a-answer",
            DESK,
            "desk",
            "answer",
            "1",
            "answer",
            "D67 1",
            1,
        )
    } else {
        (
            "s3a-new-topic",
            TOPIC,
            "topic",
            "title",
            TITLE,
            "new-topic",
            TOPIC_LINE,
            2,
        )
    };
    if decision {
        enable_aligned_answers(tab, check)?;
    } else {
        edit(tab, which, field, input)?;
    }
    script(tab, check, "__s3.reset(); __s3.loss = true;")?;
    click(tab, check, root, action)?;
    let body = format!(
        "const saw = __s3.status('{which}'); return {{ok: \
         !!__s3.button('{which}', 'undo') && saw.includes(want), saw}};"
    );
    tab.holds(check, &body, line)?;
    if decision {
        both_buttons(tab, check, false)?;
    }
    if !decision {
        tab.holds(
            check,
            "return {ok: !__s3.chip(want), saw: __s3.status('topic')};",
            SLUG,
        )?;
    }
    let body = format!(
        "const button = __s3.button('{which}', 'check'); \
         return {{ok: !!button && !button.disabled && button.textContent.includes('Check again'), \
         saw: __s3.status('{which}')}};"
    );
    tab.until(check, &body, "", SEND_WAIT)?;
    let body = format!(
        "const field = __s3.field('{which}', '{field}'), root = __s3.root('{which}'); \
         const action = __s3.button('{which}', '{action}'); \
         const saw = {{value: field?.value, nonce: __s3.nonce, posts: __s3.posts.length}}; \
         return {{ok: saw.value === want && (!!field && (field.readOnly || field.matches(':disabled'))) \
         && (!action || action.disabled) && saw.nonce === 1 && saw.posts === 1 && \
         __s3.realReceipt?.outcome === 'sent' && !!root, saw}};"
    );
    tab.holds(check, &body, input)?;
    if !decision {
        tab.holds(
            check,
            "return {ok: !__s3.chip(want), saw: __s3.status('topic')};",
            SLUG,
        )?;
    }
    assert_eq!(rig.enter_count(), count);
    click(tab, check, root, "check")?;
    if decision {
        wait(
            tab,
            check,
            "const saw = ['desk','card'].map(w => __s3.status(w)); \
             return {ok: saw.every(x => /answered.*waiting for the orchestrator/i.test(x)), saw};",
            "",
        )?;
    } else {
        wait(
            tab,
            check,
            "return {ok: __s3.chip(want), saw: __s3.status('topic')};",
            SLUG,
        )?;
    }
    let proof = if decision {
        "const current = __s3.state.desk.decide.find(row => row.item === 'D67'); \
         const proof = body.decide?.item === 'D67' && body.decide?.bind === current?.bind;"
    } else {
        "const proof = !('decide' in body);"
    };
    let body = format!(
        "const [first, second] = __s3.posts, body = JSON.parse(first?.body || '{{}}');          {proof}          const saw = {{count: __s3.posts.length, nonce: __s3.nonce, elapsed: first?.elapsed, body, proof}};          return {{ok: saw.count === 2 && saw.nonce === 1 && first.body === second?.body &&          first.elapsed >= 4990 && first.elapsed < 7200 && body.text === want && proof && \
         !__s3.spent?.includes(body.nonce), saw}};"
    );
    tab.holds(check, &body, line)?;
    assert_eq!(rig.enter_count(), count);
    let submitted = rig.submitted();
    let expected = format!(" from human:chat: {line}\n");
    if submitted.matches(&expected).count() != 1 {
        return Err(Failed::new(
            check,
            format!("wanted one {line:?} in {submitted:?}"),
        ));
    }
    ok(check, cell);
    Ok(())
}

#[allow(
    clippy::disallowed_methods,
    reason = "read the test rig's own memo fixture"
)]
fn finish_topic(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "s3a-topic-chip";
    assert!(
        !rig.dir.join("memo.tsv").exists()
            || !std::fs::read_to_string(rig.dir.join("memo.tsv"))
                .expect("fixture memo")
                .contains(&format!("\ttopic-{SLUG}\t"))
    );
    ae::store::open(&rig.dir)
        .append_memo(
            format!("2026-11-22T12:00:00Z\ttui\ttopic-{SLUG}\ttitle: Created | state: quiet\n")
                .as_bytes(),
        )
        .expect("simulated orchestrator records the requested topic after the receipt");
    wait(
        tab,
        CHECK,
        "const row = [...document.querySelectorAll('.col button.row .ln')].find(\
         e => e.textContent.split(' ')[0] === '#' + want);          const saw = {row: !!row, chip: __s3.chip(want)}; return {ok: saw.row && !saw.chip, saw};",
        SLUG,
    )
}

fn cap_refusal(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CHECK: &str = "s3a-cap";
    for number in 1..=3 {
        let id = format!("ae-20261122T200000Z-{number:08x}");
        let event = Value::obj([
            ("ts", Value::str("2026-11-22T20:00:00Z")),
            ("actor", Value::str("console:local")),
            ("action", Value::str("ask")),
            ("target", Value::str("tui")),
            ("actor_session", Value::str(rig.session.as_str())),
            ("target_session", Value::str(rig.session.as_str())),
            ("target_slot", Value::str("main")),
            ("ref", Value::str(id)),
        ]);
        ae::store::open(&rig.dir)
            .append_event(&format!("{}\n", event.render()))
            .expect("fill the remaining console slots");
    }
    publish_board(rig, REPLACEMENT);
    wait(
        tab,
        CHECK,
        "const saw = __s3.root('desk')?.textContent; return {ok: saw?.includes(want), saw};",
        REPLACEMENT,
    )?;
    edit(tab, "desk", "answer", "keep this answer")?;
    click(tab, CHECK, DESK, "answer")?;
    wait(
        tab,
        CHECK,
        "const saw = __s3.status('desk'); const input = __s3.field('desk','answer'); \
         return {ok: __s3.cap?.startsWith('cap-full: 5/5') && saw.includes(__s3.cap) && \
         !!input && !input.matches(':disabled') && !input.readOnly && input.value === want, saw};",
        "keep this answer",
    )?;
    assert_eq!(rig.enter_count(), 2, "a cap refusal carried input");
    ok(CHECK, cell);
    Ok(())
}

pub(super) fn check_s3a(tab: &mut Tab) -> Checked {
    let rig = smoke_rig();
    publish_board(&rig, QUESTION);
    let listener = start_listener(&rig.scratch);
    let phone_small = Cell {
        width: 360,
        height: 740,
        dark: false,
    };
    let phone_large = Cell {
        width: 390,
        height: 844,
        dark: true,
    };
    let wide = Cell {
        width: 1440,
        height: 900,
        dark: true,
    };
    tab.view(phone_small);
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(listener.link()))]),
    );
    if let Some(error) = loaded.get_str("errorText") {
        listener.shutdown();
        return Err(Failed::new(
            CASE,
            format!("could not load test page: {error}"),
        ));
    }
    let result = (|| {
        wait(tab, CASE, READY, "")?;
        phone(tab, phone_small)?;
        phone(tab, phone_large)?;
        tab.view(wide);
        tab.call(
            "Emulation.setTouchEmulationEnabled",
            Value::obj([("enabled", Value::Bool(false))]),
        );
        script(tab, CASE, INSTALL)?;
        open_card(tab, CASE)?;
        both_buttons(tab, "s3a-answer-disabled", false)?;
        controls_and_parser_parity(tab)?;
        undo_and_hide(tab, &rig, wide)?;
        card_skew(tab, &rig, wide)?;
        cancel_on_board_poll(tab, &rig, wide)?;
        route_moves(tab, &rig, wide)?;
        check_again(tab, &rig, wide, true)?;
        check_again(tab, &rig, wide, false)?;
        finish_topic(tab, &rig)?;
        cap_refusal(tab, &rig, wide)
    })();
    listener.shutdown();
    result
}
