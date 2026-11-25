//! T1 in the real browser, listener and fake orchestrator seat.

use std::time::Duration;

use ae::json::Value;

use super::{
    Cell, Checked, Failed, PAGE_WAIT, READY, Rig, SEND_WAIT, Tab, ok, smoke_rig, start_listener,
};

const ALL: &str = "[data-role=tab-all]";
const D61: &str = "form.decision-answer[data-item=D61]";
const QUESTION: &str = "Deploy <b data-t1-hostile>this</b>?";
const OPTION: &str = "<i data-t1-hostile>Hold</i>";
const CONVERSATION: &str = "0199c0de-3162-4890-abcd-ef0123456789";
const CELL: Cell = Cell {
    width: 1440,
    height: 900,
    dark: true,
};
const RECORD: &str = r"(() => {
  const send = window.fetch.bind(window);
  const proof = window.__t1 = {posts: [], nonces: 0, firstClick: 0};
  addEventListener('click', event => {
    if (event.target.closest('form[data-item=D61] [data-action=answer]')) {
      proof.firstClick = performance.now();
    }
  }, true);
  window.fetch = async (url, init) => {
    if (url === '/api/state') {
      const headers = new Headers(init?.headers);
      if (!proof.state) headers.delete('If-None-Match');
      const response = await send(url, {...init, headers});
      if (response.status === 200) proof.state = await response.clone().json();
      return response;
    }
    if (url === '/api/nonce') proof.nonces++;
    if (url !== '/api/send') return send(url, init);
    const post = {body: JSON.parse(init.body), elapsed: performance.now() - proof.firstClick};
    proof.posts.push(post);
    const response = await send(url, init);
    if (response.status === 200) post.receipt = await response.clone().json();
    return response;
  };
})()";

fn script(tab: &mut Tab, check: &'static str, code: &str) -> Checked {
    tab.eval(code)
        .map(drop)
        .map_err(|why| Failed::new(check, why))
}

fn wait(tab: &mut Tab, check: &'static str, body: &str, want: &str) -> Checked {
    tab.until(check, body, want, PAGE_WAIT).map(drop)
}

fn aboard(rig: &Rig, first: &str) {
    let board = format!(
        "DECIDE\n\
         D61 | {QUESTION} 1) {first} 2) {OPTION} (recommend 2) #smoke\n\
         D62 | Wohin? 1) Hier 2) Dort (Empfehlung 1) #calm\n\
         D63 | Reserved? #reserved 1) Go 2) Hold (recommend 2) #smoke\n\
         D64 | Reserved without topic? 1) Go 2) Hold (recommend 2) #reserved\n\
         D65 | Decide while quiet? 1) Go 2) Hold (recommend 2) #s2-m\n\
         D66 | Legacy prose #smoke\n\
         D67 | No topic legacy prose\n\
         D68 | Agree? 1) part2) #C 2) Ordinary #reserved_extra #calm\n\
         D69 | Missing option? 1) Go 3) Hold (recommend 3) #smoke\n"
    );
    let items = ae::desk::parse_input(board.as_bytes()).expect("T1 fixture decision grammar");
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "tui",
    )
    .expect("publish T1 fixture board");
}

#[allow(
    clippy::disallowed_methods,
    reason = "read this isolated browser rig's own meta fixture"
)]
fn seed(rig: &Rig) {
    let meta = rig.dir.join("meta");
    let mut lines = std::fs::read_to_string(&meta).expect("browser fixture meta");
    let store = rig.scratch.join("claude-t1");
    lines.push_str(&format!(
        "\nharness_session.main={CONVERSATION}\nconfig_home.main={}\n",
        store.display(),
    ));
    std::fs::write(meta, lines).expect("publish browser fixture meta");
    let transcript = store.join("projects/t1");
    std::fs::create_dir_all(&transcript).expect("create transcript project");
    let messages = [
        "#smoke Smoke-only row",
        "#calm Calm-only row",
        "#smoke: #calm. Both-tabs row <b data-t1-hostile>",
        "Unknown-only row #does-not-exist",
        "Untagged row x#smoke #smoke_x",
    ];
    let log: String = messages
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let words = Value::str(*text).render();
            format!(
                concat!(
                    r#"{{"type":"user","timestamp":"2026-11-23T10:0{i}:00Z","message":{{"role":"user","content":"Fixture question {i}"}}}}"#,
                    "\n",
                    r#"{{"type":"assistant","timestamp":"2026-11-23T10:0{i}:01Z","message":{{"role":"assistant","content":[{{"type":"text","text":{words}}}]}}}}"#,
                ),
                i = i,
                words = words,
            ) + "\n"
        })
        .collect();
    std::fs::write(transcript.join(format!("{CONVERSATION}.jsonl")), log)
        .expect("publish browser fixture transcript");
    ae::store::open(&rig.dir)
        .append_memo(
            b"2026-11-23T08:00:00Z\ttui\ttopic-t1-unreadable\ttitle: Unreadable T1 topic\n\
              2026-11-23T08:01:00Z\ttui\ttopic-reserved\ttitle: Legacy reserved memo | state: quiet\n\
              2026-11-23T08:02:00Z\ttui\ttopic-t1-moving\ttitle: Moving T1 topic | state: moving\n",
        )
        .expect("publish browser fixture topics");
    aboard(rig, "Go");
}

fn state_check(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t1-alle";
    wait(
        tab,
        CHECK,
        "const tabs = [...document.querySelectorAll('[role=tablist] [role=tab]')]; \
         const first = tabs.shift(); const panel = document.querySelector('[data-panel=all]'); \
         const slugs = tabs.map(t => t.dataset.slug).sort(); \
         const rows = [...(panel?.querySelectorAll('button.row .ln') || [])].map(e => e.textContent); \
         const decisions = panel?.querySelector('[data-section=decisions]'); \
         const all = panel?.textContent || ''; \
         const saw = {slugs, rows, selected: first?.getAttribute('aria-selected'), all}; \
         return {ok: first?.dataset.role === 'tab-all' && saw.selected === 'true' && \
         slugs.join(' ') === 'calm s2-m smoke t1-moving t1-unreadable' && \
         ['s2-z','s2-parked','s2-closed','t1-unreadable'].every(s => rows.some(x => x.split(' ')[0] === '#' + s)) && \
         !!panel.querySelector('.revisit .rv-walk') && !!panel.querySelector('.new-topic') && \
         !!decisions?.querySelector('[data-action=open-tab][data-slug=smoke]') && \
         ['D61','D62','D63','D64','D65','D66','D67','D68','D69'].every(id => all.includes(id)) && \
         !panel.querySelector('form[data-item=D61], form[data-item=D66]') && \
         !!panel.querySelector('[data-section=untagged] form[data-item=D67] [data-field=answer]'), saw};",
        "",
    )?;
    wait(
        tab,
        "t1-reserved-routing",
        "const desk = __t1.state?.desk; const a = desk?.decide.find(d => d.item === 'D63'); \
         const b = desk?.decide.find(d => d.item === 'D64'); \
         const memo = desk?.groups.flatMap(g => g.rows).find(r => r.slug === 'reserved'); \
         const saw = {a, b, memo}; return {ok: a?.reserved === true && a.tags.join(',') === 'smoke' && \
         b?.reserved === true && b.tags.length === 0 && a.text.includes('#reserved') && \
         memo?.state === 'quiet' && !memo.waits.some(w => w.kind === 'decide'), saw};",
        "",
    )?;
    reserved(tab, "D64")?;
    ok(CHECK, CELL);
    Ok(())
}

fn visible_rows(tab: &mut Tab, check: &'static str, expected: &str) -> Checked {
    wait(
        tab,
        check,
        "const rows = [...document.querySelectorAll('article.msg .bub')].map(el => el.textContent); \
         const names = ['Smoke-only row','Calm-only row','Both-tabs row','Unknown-only row','Untagged row']; \
         const saw = names.filter(name => rows.some(row => row.includes(name))).join(','); \
         return {ok: saw === want && !document.querySelector('[data-t1-hostile]'), saw: {saw, rows}};",
        expected,
    )
}

fn choose(tab: &mut Tab, check: &'static str, slug: &str) -> Checked {
    let selector = if slug.is_empty() {
        ALL.to_owned()
    } else {
        format!("[data-role=tab][data-slug={slug}]")
    };
    tab.click_selector(check, &selector)?;
    wait(
        tab,
        check,
        "const active = document.querySelector('[role=tab][aria-selected=true]'); \
         const saw = active?.dataset.slug || ''; \
         const panel = document.querySelector(want ? '[data-panel=topic]' : '[data-panel=all]'); \
         return {ok: !!active && saw === want && !!panel && (panel.dataset.slug || '') === want, saw};",
        slug,
    )
}

fn tabs_and_filter(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "t1-filter";
    visible_rows(
        tab,
        CHECK,
        "Smoke-only row,Calm-only row,Both-tabs row,Unknown-only row,Untagged row",
    )?;
    tab.click_selector(
        CHECK,
        "[data-section=decisions] [data-action=open-tab][data-slug=smoke]",
    )?;
    visible_rows(tab, CHECK, "Smoke-only row,Both-tabs row")?;
    tab.click(CHECK, "box")?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str("draft survives tabs"))]),
    );
    choose(tab, CHECK, "calm")?;
    visible_rows(tab, CHECK, "Calm-only row,Both-tabs row")?;
    tab.holds(
        "tab-focus-kept",
        "const box = document.querySelector('.composer textarea'); \
         const saw = {value: box?.value, focused: document.activeElement === box}; \
         return {ok: saw.value === want && saw.focused, saw};",
        "draft survives tabs",
    )?;
    for (item, recommendation) in [("D62", "1"), ("D68", "")] {
        let selector = Value::str(format!("form[data-item={item}]")).render();
        wait(
            tab,
            "t1-choice-grammar",
            &format!(
                "const form = document.querySelector({selector}); \
                 const radios = [...(form?.querySelectorAll('input[type=radio][data-choice]') || [])]; \
                 const recommended = form?.querySelector('[data-recommended] input'); \
                 const saw = {{n: radios.map(r => r.dataset.choice), rec: recommended?.dataset.choice || '', text: form?.textContent}}; \
                 return {{ok: saw.n.join(',') === '1,2' && saw.rec === want && \
                 (want || (saw.text.includes('part2) #C') && saw.text.includes('#reserved_extra'))), saw}};"
            ),
            recommendation,
        )?;
    }

    choose(tab, CHECK, "t1-moving")?;
    ae::store::open(&rig.dir)
        .append_memo(b"2026-11-24T08:02:00Z\ttui\ttopic-t1-moving\tstate: quiet\n")
        .expect("quiet the selected automatic topic");
    wait(
        tab,
        CHECK,
        "const saw = document.querySelector('[data-role=tab-all]')?.getAttribute('aria-selected'); \
         return {ok: saw === 'true' && !document.querySelector('[role=tab][data-slug=t1-moving]'), saw};",
        "",
    )?;
    tab.holds(
        CHECK,
        "const row = [...document.querySelectorAll('[data-panel=all] button.row')].find(row => \
         row.querySelector('.ln')?.textContent.split(' ')[0] === '#s2-parked'); \
         if (row) row.dataset.t1Parked = ''; return {ok: !!row, saw: row?.textContent};",
        "",
    )?;
    tab.click_selector(CHECK, "[data-t1-parked]")?;
    wait(
        tab,
        CHECK,
        "const saw = document.querySelector('[role=tab][data-slug=s2-parked]')?.getAttribute('aria-selected'); \
         return {ok: saw === 'true', saw};",
        "",
    )?;
    choose(tab, CHECK, "")?;
    tab.holds(CHECK, "const saw = !!document.querySelector('[role=tab][data-slug=s2-parked]'); return {ok: !saw, saw};", "")?;
    visible_rows(
        tab,
        CHECK,
        "Smoke-only row,Calm-only row,Both-tabs row,Unknown-only row,Untagged row",
    )?;
    ok(CHECK, CELL);
    Ok(())
}

fn reserved(tab: &mut Tab, item: &str) -> Checked {
    wait(
        tab,
        "t1-reserved-display",
        "const row = document.querySelector('form[data-item=' + want + '][data-reserved]'); \
         const note = row?.querySelector('[data-role=reserved]')?.textContent; \
         const controls = row?.querySelector('input,button,select,textarea'); \
         return {ok: !!row && /pane/i.test(note) && /telegram/i.test(note) && !controls, saw: row?.textContent};",
        item,
    )
}

fn decision(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "t1-choice-and-undo";
    choose(tab, CHECK, "smoke")?;
    reserved(tab, "D63")?;
    wait(
        tab,
        "t1-legacy-answer",
        "const forms = ['D66','D69'].map(id => document.querySelector('form[data-item=' + id + ']')); \
         const saw = forms.map(f => f?.textContent); \
         return {ok: forms.every(f => f?.querySelector('input[data-field=answer]') && \
         !f.querySelector('input[type=radio]')), saw};",
        "",
    )?;
    let texts = Value::Arr(vec![Value::str(QUESTION), Value::str(OPTION)]).render();
    wait(
        tab,
        CHECK,
        &format!(
            "const form = document.querySelector('{D61}'); const saw = form?.textContent; \
             const radios = [...(form?.querySelectorAll('input[type=radio][data-choice]') || [])]; \
             const [question, option] = {texts}; \
             return {{ok: saw?.includes(question) && saw.includes(option) && \
             radios.map(r => r.dataset.choice).join(',') === '1,2' && radios[0].name === radios[1].name && \
             !form.querySelector('[data-t1-hostile], input[type=checkbox], input:checked') && \
             !!form.querySelector('[data-recommended] input[data-choice=2]'), saw}};"
        ),
        "",
    )?;
    tab.click_selector(CHECK, &format!("{D61} [data-action=recommend]"))?;
    wait(
        tab,
        CHECK,
        "const picked = [...document.querySelectorAll('form[data-item=D61] input:checked')].map(n => n.dataset.choice); \
         const saw = {picked, nonces: __t1.nonces, posts: __t1.posts.length}; \
         return {ok: picked.join('') === '2' && saw.nonces === 0 && saw.posts === 0, saw};",
        "",
    )?;
    tab.click_selector(CHECK, &format!("{D61} [data-action=answer]"))?;
    wait(
        tab,
        CHECK,
        "const form = document.querySelector('form[data-item=D61]'); const saw = form?.textContent; \
         return {ok: !!form.querySelector('[data-action=undo]') && saw.includes('D61 2') && __t1.nonces === 0, saw};",
        "",
    )?;

    // Discussion is an independent plain send during the answer countdown.
    tab.click_selector(CHECK, &format!("{D61} [data-action=talk]"))?;
    tab.click_selector(CHECK, &format!("{D61} [data-field=talk]"))?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str("Please explain"))]),
    );
    tab.click_selector(CHECK, &format!("{D61} [data-action=talk-send]"))?;
    tab.click_selector(CHECK, &format!("{D61} [data-action=undo]"))?;
    std::thread::sleep(Duration::from_secs(6));
    tab.holds(
        CHECK,
        "const saw = __t1.posts.map(p => p.body); return {ok: __t1.nonces === 1 && saw.length >= 1 && \
         saw.every(b => b.text === want && !('decide' in b) && b.nonce === saw[0].nonce), saw};",
        "#smoke D61: Please explain",
    )?;

    script(tab, CHECK, "__t1.posts.length = 0; __t1.nonces = 0;")?;
    const CHANGED: &str = "t1-choice-changed";
    tab.click_selector(CHANGED, &format!("{D61} input[data-choice=1]"))?;
    tab.click_selector(CHANGED, &format!("{D61} [data-action=answer]"))?;
    aboard(rig, "Go later");
    wait(
        tab,
        CHANGED,
        "const form = document.querySelector('form[data-item=D61]'); const saw = form?.textContent; \
         return {ok: saw?.includes('Go later') && /decision changed/i.test(saw) && \
         !form.querySelector('[data-action=undo]'), saw};",
        "",
    )?;
    std::thread::sleep(Duration::from_secs(6));
    tab.holds(
        CHANGED,
        "const saw = {posts: __t1.posts, nonces: __t1.nonces}; \
         return {ok: saw.posts.length === 0 && saw.nonces === 0, saw};",
        "",
    )?;
    assert_eq!(rig.enter_count(), 1, "only the discussion reached the seat");
    aboard(rig, "Go");
    wait(
        tab,
        CHANGED,
        "const form = document.querySelector('form[data-item=D61]'); const saw = form?.textContent; \
         return {ok: !!form.querySelector('input[data-choice=1]:enabled') && !saw.includes('Go later'), saw};",
        "",
    )?;
    ok(CHANGED, CELL);

    tab.click_selector(CHECK, &format!("{D61} input[data-choice=1]"))?;
    tab.click_selector(CHECK, &format!("{D61} [data-action=answer]"))?;
    choose(tab, CHECK, "calm")?;
    choose(tab, CHECK, "smoke")?;
    wait(
        tab,
        CHECK,
        "const saw = document.querySelector('form[data-item=D61] [data-role=status]')?.textContent; \
         return {ok: saw?.includes('D61 1') && !!document.querySelector('form[data-item=D61] [data-action=undo]'), saw};",
        "",
    )?;
    tab.until(
        CHECK,
        "const [first] = __t1.posts; const saw = __t1.posts; \
         return {ok: !!first && first.elapsed >= 4990 && first.body.text === want && \
         first.body.decide?.item === 'D61' && /^[0-9a-f]{64}$/.test(first.body.decide?.bind) && \
         first.body.decide.bind === __t1.state?.desk.decide.find(d => d.item === 'D61')?.bind && \
         __t1.nonces === 1 && saw.every(p => JSON.stringify(p.body) === JSON.stringify(first.body)) && \
         saw.some(p => p.receipt?.outcome === 'sent'), saw};",
        "D61 1",
        SEND_WAIT,
    )?;
    assert_eq!(
        rig.enter_count(),
        2,
        "discussion and one selected answer reached the seat"
    );
    for words in ["#smoke D61: Please explain", "D61 1"] {
        let line = format!(" from human:chat: {words}\n");
        assert_eq!(
            rig.submitted().matches(&line).count(),
            1,
            "carried {words:?}"
        );
    }
    ok(CHECK, CELL);
    Ok(())
}

fn send_drafts(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "t1-draft-prefix";
    choose(tab, CHECK, "smoke")?;
    for (at, (text, expected)) in [
        ("close the loop", "#smoke: close the loop"),
        ("keep the note", "keep the note"),
        ("keep the context", "#smoke: keep the context"),
        ("ask Anna to review", "#smoke: ask Anna to review"),
        ("new idea", "#smoke: new idea"),
    ]
    .into_iter()
    .enumerate()
    {
        let chip = "[data-role=tag-chip][data-slug=smoke]";
        wait(
            tab,
            CHECK,
            "const chip = document.querySelector(want); const saw = chip?.textContent; \
             return {ok: saw?.includes('#smoke') && !!chip.querySelector('[data-action=untag]'), saw};",
            chip,
        )?;
        if at == 1 {
            tab.click_selector(CHECK, &format!("{chip} [data-action=untag]"))?;
            tab.holds(
                CHECK,
                "const saw = {chip: !!document.querySelector(want), \
                 active: document.querySelector('[role=tab][aria-selected=true]')?.dataset.slug}; \
                 return {ok: !saw.chip && saw.active === 'smoke', saw};",
                chip,
            )?;
        }
        tab.click(CHECK, "box")?;
        tab.call("Input.insertText", Value::obj([("text", Value::str(text))]));
        tab.click(CHECK, "send")?;
        tab.until(
            CHECK,
            "const saw = document.querySelector('.composer .receipt.k-sent')?.textContent; \
             const box = document.querySelector('.composer textarea'); \
             return {ok: !!saw && box?.value === '', saw};",
            "",
            SEND_WAIT,
        )?;
        assert_eq!(rig.enter_count(), at + 1, "one more send reaches the seat");
        let line = format!(" from human:chat: {expected}\n");
        assert_eq!(
            rig.submitted().matches(&line).count(),
            1,
            "carried {expected:?}"
        );
    }
    ok(CHECK, CELL);
    Ok(())
}

fn run(
    tab: &mut Tab,
    fixture: impl FnOnce(&Rig),
    check: impl FnOnce(&mut Tab, &Rig) -> Checked,
) -> Checked {
    let rig = smoke_rig();
    fixture(&rig);
    let listener = start_listener(&rig.scratch);
    tab.view(CELL);
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(listener.link()))]),
    );
    let result = if let Some(error) = loaded.get_str("errorText") {
        Err(Failed::new(
            "t1-start",
            format!("could not load T1 page: {error}"),
        ))
    } else {
        (|| {
            wait(tab, "t1-ready", READY, "")?;
            check(tab, &rig)
        })()
    };
    listener.shutdown();
    result
}

pub(super) fn check_t1(tab: &mut Tab) -> Checked {
    run(tab, seed, |tab, rig| {
        script(tab, "t1-observe", RECORD)?;
        state_check(tab)?;
        tabs_and_filter(tab, rig)?;
        decision(tab, rig)
    })?;
    run(tab, |_| {}, send_drafts)
}
