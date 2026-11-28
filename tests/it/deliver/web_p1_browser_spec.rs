//! Desktop polish through the real listener and browser.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::{
    CELLS, Cell, Checked, Failed, PAGE_WAIT, Rig, Tab, Value, ok, smoke_rig, start_listener,
};

const WIDE: Cell = CELLS[3];
const NARROW: Cell = CELLS[2];
const ALL: &str = "button[data-role=side-all]";
const CARD: &str = "button[data-role=side-row][data-slug=p1-card]";
const NOTE: &str = "button[data-role=side-row][data-slug=p1-note]";
const FOLD: &str = "[data-action=toggle-group]";
const CONVERSATION: &str = "0199c0de-1222-4890-abcd-ef0123456789";
const OLDER_AT: &str = "2026-11-24T10:07:00Z";
const NEWEST_AT: &str = "2026-11-24T10:00:01Z";
const HOSTILE: &str = "<img src=x onerror=alert(1)>";

const READY: &str = "const saw = !!document.querySelector('[data-panel=all] .new-topic'); \
    return {ok: saw && window.__aeStale === undefined, saw};";
const ACTIVE: &str = "const saw = document.querySelector('[data-panel=topic]')?.dataset.slug; \
    return {ok: saw === want, saw};";
const FAVICON: &str = r"
const url = new URL(document.querySelector('link[rel~=icon]')?.href || '/favicon.ico', location.href);
if (url.origin !== location.origin) return {ok: false, saw: url.origin};
const response = await fetch(url);
return {ok: response.ok, saw: {path: url.pathname, status: response.status}};
";

pub(super) fn favicon(tab: &mut Tab, cell: Cell) -> Checked {
    tab.holds("favicon", FAVICON, "")?;
    ok("favicon", cell);
    Ok(())
}

fn run(tab: &mut Tab, check: &'static str, script: &str) -> Result<Value, Failed> {
    tab.eval(script).map_err(|why| Failed::new(check, why))
}

fn expect_page(tab: &mut Tab, check: &'static str, body: &str, want: &str) -> Checked {
    tab.until(check, body, want, PAGE_WAIT).map(drop)
}

fn turn(path: &Path, minute: u8, body: &str) {
    // The rig may have its own dated events. These fixture turns sort after them.
    let at = format!("2037-11-24T11:{minute:02}:00Z");
    let body = Value::str(body).render();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|error| panic!("open P1 transcript: {error}"));
    writeln!(
        log,
        concat!(
            r#"{{"type":"user","timestamp":"{at}","message":{{"role":"user","content":"P1 fixture prompt"}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"{at}","message":{{"role":"assistant","content":[{{"type":"text","text":{body}}}]}}}}"#,
        ),
        at = at,
        body = body,
    )
    .unwrap_or_else(|error| panic!("append P1 transcript turn: {error}"));
}

fn seed(rig: &Rig) -> PathBuf {
    let mut lines = String::new();
    let written = [
        (
            "p1-card",
            "2026-11-24T10:00:00Z",
            format!(r"title: Old card | what: {HOSTILE} \| remains: here | audit: a\\b"),
        ),
        (
            "p1-card",
            OLDER_AT,
            "last: Carried from an older record".into(),
        ),
        (
            "p1-card",
            NEWEST_AT,
            "title: P1 card | state: quiet | what: Current detail | next: Current next step".into(),
        ),
        ("p1-note", OLDER_AT, "title: P1 note | state: moving".into()),
        (
            "p1-note",
            NEWEST_AT,
            "audit: Latest record carries no winning field".into(),
        ),
        (
            "p1-order-a",
            "2026-11-24T10:02:00Z",
            "title: Order A | state: moving".into(),
        ),
        (
            "p1-order-z",
            "2026-11-24T10:03:00Z",
            "title: Order Z | state: moving".into(),
        ),
        (
            "p1-pin",
            "2026-11-24T10:04:00Z",
            "title: Pinned quiet | state: quiet".into(),
        ),
        (
            "p1-unreadable",
            "2026-11-24T10:05:00Z",
            "title: Unreadable fixture".into(),
        ),
    ];
    for (slug, at, line) in written {
        writeln!(lines, "{at}\ttui\ttopic-{slug}\t{line}")
            .unwrap_or_else(|error| panic!("record fixture: {error}"));
    }
    for index in 0..24 {
        writeln!(
            lines,
            "{NEWEST_AT}\ttui\ttopic-p1-moving-{index:02}\tstate: moving"
        )
        .unwrap_or_else(|error| panic!("moving fixture: {error}"));
    }
    ae::store::open(&rig.dir)
        .append_memo(lines.as_bytes())
        .unwrap_or_else(|error| panic!("P1 memo fixture: {error:?}"));
    let items = ae::desk::parse_input(b"DECIDE\nD1 | Pick a setting? 1) Fast 2) Steady (recommend 2)\nD2 | What should change? #p1-note\n")
        .unwrap_or_else(|error| panic!("P1 board grammar: {error:?}"));
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "tui",
    )
    .unwrap_or_else(|error| panic!("P1 board fixture: {error:?}"));

    let home = rig.scratch.join("claude-p1");
    let project = home.join("projects/p1");
    std::fs::create_dir_all(&project)
        .unwrap_or_else(|error| panic!("P1 transcript project: {error}"));
    ae::meta::rewrite(&rig.dir, "harness_session.main", Some(CONVERSATION))
        .unwrap_or_else(|error| panic!("P1 conversation id: {error:?}"));
    ae::meta::rewrite(&rig.dir, "config_home.main", home.to_str())
        .unwrap_or_else(|error| panic!("P1 transcript home: {error:?}"));
    let path = project.join(format!("{CONVERSATION}.jsonl"));
    for minute in 0..24 {
        turn(
            &path,
            minute,
            &"The stream wraps this ordinary transcript text at narrower widths. ".repeat(4),
        );
    }
    turn(&path, 24, "#p1-order-a P1 newest visible turn");
    path
}

fn select(tab: &mut Tab, check: &'static str, locator: &str, slug: &str) -> Checked {
    tab.click_selector(check, locator)?;
    expect_page(tab, check, ACTIVE, slug)
}

fn overview(tab: &mut Tab, check: &'static str) -> Checked {
    tab.click_selector(check, ALL)?;
    expect_page(tab, check, READY, "")
}

fn history(tab: &mut Tab) -> Checked {
    const CHECK: &str = "p1-history-fields";
    select(tab, CHECK, CARD, "p1-card")?;
    tab.holds(CHECK, r"
const heading = [...document.querySelectorAll('.card h3')].find(node => node.textContent === 'Earlier records');
const record = heading?.parentElement.querySelector('li');
const lines = ['title: Old card', 'what: ' + want + ' | remains: here', 'audit: a\\b'];
const shown = (element) => element.textContent.replace(/\s+/g, ' ').trim();
const rows = record ? [...record.querySelectorAll('*')].filter(node => lines.includes(shown(node))) : [];
const text = rows.map(shown);
const noMarkup = !!record && !record.querySelector('img, script, [onerror]');
return {ok: lines.every(line => text.includes(line)) && noMarkup, saw: {text, record: record?.textContent, noMarkup}};
", HOSTILE)?;
    ok(CHECK, WIDE);
    Ok(())
}

fn provenance(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "p1-card-provenance";
    let metadata = std::fs::read_to_string(rig.dir.join("meta"))
        .unwrap_or_else(|error| panic!("P1 meta: {error}"));
    let uuid = metadata
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .unwrap_or_else(|| panic!("P1 UUID missing in fixture meta"));
    assert_eq!(uuid.len(), 36);
    let origin = Value::Arr(
        [rig.session.as_str(), uuid, NEWEST_AT, OLDER_AT]
            .into_iter()
            .map(Value::str)
            .collect(),
    )
    .render();
    for (locator, slug, earlier) in [(CARD, "p1-card", "Last"), (NOTE, "p1-note", "State,Title")] {
        select(tab, CHECK, locator, slug)?;
        let want = format!("[{origin},\"{earlier}\"]");
        tab.holds(CHECK, r#"
const [[session, uuid, current, earlier], expected] = JSON.parse(want);
const card = document.querySelector('.card');
const sub = card?.querySelector('.sub');
const origin = sub?.querySelector('[title*="' + uuid + '"]');
const words = (node) => node?.textContent.replace(/\s+/g, ' ').trim();
const clock = (at) => new Date(at).toLocaleTimeString([], {hour: '2-digit', minute: '2-digit'}).replace(/\s+/g, ' ');
const currentTime = clock(current);
const earlierTime = clock(earlier);
const fields = [...card.querySelectorAll('.fld > h3')];
const prior = fields.filter(node => node.querySelector('[title*="' + uuid + '"]'));
const labels = prior.map(node => node.querySelector('span')?.textContent).sort().join(',');
const header = words(sub);
const times = prior.map(words);
const history = fields.find(node => words(node) === 'Earlier records')?.parentElement;
const historySources = history?.querySelectorAll('[title*="' + uuid + '"]').length;
const once = header?.split(session).length === 2 && header?.split(currentTime).length === 2;
const earlierCorrect = times.every(text => text.includes(session) && text.includes(earlierTime));
const literal = card.textContent.includes(uuid);
return {ok: !!origin && once && labels === expected && earlierCorrect && historySources === (expected === 'Last' ? 1 : 0) && !literal, saw: {header, labels, times, historySources, literal}};
"#, &want)?;
    }
    ok(CHECK, WIDE);
    Ok(())
}

const ORDER: &str = r"
const slug = (node) => node.querySelector('.ln').textContent.match(/^#([a-z0-9-]+)/)?.[1];
const left = [...document.querySelectorAll('nav[aria-label=Topics] [data-role=side-row]')].map(row => row.dataset.slug);
const right = [...document.querySelectorAll('[data-panel=all] button.row')].map(slug);
const expected = want ? left.filter(slug => slug.includes(want)) : left;
const ordered = left.indexOf('p1-order-a') < left.indexOf('p1-order-z') && left.includes('p1-order-a');
const pinned = !!document.querySelector('[data-group=pinned] [data-slug=p1-pin]');
return {ok: ordered && pinned && expected.length > 1 && JSON.stringify(right) === JSON.stringify(expected), saw: {expected, right, pinned}};
";

fn order(tab: &mut Tab) -> Checked {
    const CHECK: &str = "p1-all-order";
    overview(tab, CHECK)?;
    run(
        tab,
        CHECK,
        r#"
const key = 'ae-web-pins';
localStorage.setItem(key, '["p1-pin"]');
dispatchEvent(new StorageEvent('storage', {key, storageArea: localStorage}));
"#,
    )?;
    if run(
        tab,
        CHECK,
        "document.querySelector('[data-action=toggle-group]')?.getAttribute('aria-expanded') === 'false'",
    )? == Value::Bool(true)
    {
        tab.click_selector(CHECK, FOLD)?;
    }
    expect_page(tab, CHECK, ORDER, "")?;
    tab.click_selector(CHECK, "input[type=search]")?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str("p1-order-"))]),
    );
    expect_page(tab, CHECK, ORDER, "p1-order-")?;
    ok(CHECK, WIDE);
    Ok(())
}

const BOTTOM: &str = r"
const stream = document.querySelector('.stream');
const last = stream?.querySelector('.msg:last-of-type .bub');
const remaining = stream ? Math.round(stream.scrollHeight - stream.clientHeight - stream.scrollTop) : null;
const size = stream?.clientHeight;
const frame = stream?.getBoundingClientRect();
const turn = last?.getBoundingClientRect();
const visible = !!turn && turn.top >= frame.top - 2 && turn.bottom <= frame.bottom + 2;
const saw = {remaining, size, visible, last: last?.textContent};
return {ok: !!size && visible && saw.last?.includes(want), saw};
";

fn resize(tab: &mut Tab) -> Checked {
    const CHECK: &str = "p1-resize-anchor";
    const LAST: &str = "P1 newest visible turn";
    expect_page(tab, "p1-resize-start", BOTTOM, LAST)?;
    tab.view(NARROW);
    expect_page(tab, CHECK, BOTTOM, LAST)?;
    run(
        tab,
        CHECK,
        "document.querySelector('.stream').scrollTop = 0",
    )?;
    tab.holds(CHECK, r"
const stream = document.querySelector('.stream');
await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
return {ok: stream.scrollTop === 0 && stream.scrollHeight > stream.clientHeight + 48, saw: stream.scrollTop};
", "")?;
    tab.view(WIDE);
    tab.holds(
        CHECK,
        r"
const stream = document.querySelector('.stream');
await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
const remaining = Math.round(stream.scrollHeight - stream.clientHeight - stream.scrollTop);
return {ok: remaining > 48, saw: remaining};
",
        "",
    )?;
    ok(CHECK, WIDE);
    Ok(())
}

fn view_switch(tab: &mut Tab, transcript: &Path) -> Checked {
    const CHECK: &str = "p1-view-anchor";
    const NEW: &str = "P1 turn arrived while chat was hidden";
    tab.view(NARROW);
    run(
        tab,
        CHECK,
        "const stream = document.querySelector('.stream'); stream.scrollTop = stream.scrollHeight",
    )?;
    expect_page(tab, CHECK, BOTTOM, "P1 newest visible turn")?;
    run(
        tab,
        CHECK,
        "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))",
    )?;
    tab.click_selector(CHECK, "[data-role=view-panel]")?;
    expect_page(
        tab,
        CHECK,
        "const saw = document.querySelector('.stream')?.clientHeight; return {ok: saw === 0, saw};",
        "",
    )?;
    turn(transcript, 25, NEW);
    expect_page(
        tab,
        CHECK,
        "const saw = document.querySelector('.stream')?.textContent.includes(want); return {ok: saw, saw};",
        NEW,
    )?;
    tab.click_selector(CHECK, "[data-role=view-chat]")?;
    expect_page(tab, CHECK, BOTTOM, NEW)?;
    tab.view(WIDE);
    ok(CHECK, NARROW);
    Ok(())
}

fn fold(tab: &mut Tab) -> Checked {
    const CHECK: &str = "p1-fold-visible";
    const VISIBLE: &str = r"
const head = document.querySelector('[data-group=rest] h2');
const nav = head.closest('nav');
const frame = nav.getBoundingClientRect();
const heading = head.getBoundingClientRect();
const rows = [...head.nextElementSibling.querySelectorAll('button')];
const first = rows[0]?.getBoundingClientRect();
const last = rows.at(-1)?.getBoundingClientRect();
const asked = {head: [heading.top, heading.bottom], first: first?.bottom, last: last?.bottom, frame: [frame.top, frame.bottom], count: rows.length};
const shown = heading.top + 2 >= frame.top && heading.bottom <= frame.bottom + 2 && first?.bottom <= frame.bottom + 2;
return {ok: shown && rows.length > 1 && last?.bottom <= frame.bottom + 2, saw: asked};
";
    overview(tab, CHECK)?;
    run(
        tab,
        CHECK,
        "const nav = document.querySelector('nav[aria-label=Topics]'); nav.scrollTop = nav.scrollHeight",
    )?;
    tab.click_selector(CHECK, FOLD)?;
    expect_page(tab, CHECK, VISIBLE, "")?;
    ok(CHECK, WIDE);
    Ok(())
}

const NAMED: &str = r"
const fields = [...document.querySelectorAll('input, textarea, select')];
const present = want.split(',').every(selector => document.querySelector(selector));
const missing = fields.filter(field => !field.id.trim() && !field.name.trim()).map(field => field.getAttribute('aria-label') || field.placeholder || field.className);
return {ok: present && missing.length === 0, saw: {present, missing}};
";

fn inspect_names(tab: &mut Tab, check: &'static str, fields: &str) -> Checked {
    let named = tab.holds(check, NAMED, fields);
    // Chrome builds without the forms audit still get the direct field census.
    if let Ok(audit) = tab.try_call("Audits.checkFormsIssues", Value::Obj(Vec::new()))
        && let Some(Value::Arr(issues)) = audit.get("formIssues")
        && issues.iter().any(|issue| {
            issue.get_str("errorType") == Some("FormEmptyIdAndNameAttributesForInputError")
        })
    {
        return Err(Failed::new(check, format!("Chrome form issue: {audit:?}")));
    }
    named
}

fn form_names(tab: &mut Tab) -> Checked {
    const CHECK: &str = "p1-form-names";
    overview(tab, CHECK)?;
    tab.click_selector(CHECK, "[data-field=title]")?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str("P1 form probe"))]),
    );
    tab.click_selector(CHECK, "[data-action=edit-slug]")?;
    let overview = inspect_names(
        tab,
        CHECK,
        "input[type=search],.new-topic [data-field=title],.new-topic [data-field=slug],.composer textarea",
    );
    select(tab, CHECK, NOTE, "p1-note")?;
    tab.click_selector(CHECK, "[data-action=talk]")?;
    let decision = inspect_names(
        tab,
        CHECK,
        ".card [data-field=answer],.card [data-field=talk],.composer textarea",
    );
    select(tab, CHECK, CARD, "p1-card")?;
    tab.click_selector(CHECK, ".card button.cl-line")?;
    expect_page(
        tab,
        CHECK,
        "return {ok: !!document.querySelector('.walk input.reason')};",
        "",
    )?;
    let closing = inspect_names(tab, CHECK, ".walk input.reason");
    tab.click_selector(CHECK, ".walk button.w-undo")?;
    overview.and(decision).and(closing)?;
    ok(CHECK, WIDE);
    Ok(())
}

const ACTIONS: &str = r"
const reason = box()?.placeholder;
const region = document.querySelector(want);
const buttons = [...region?.querySelectorAll('[data-action=recommend],[data-action=talk],button.cl-line') || []];
return {reason, buttons: buttons.map(button => ({label: button.textContent.trim(), disabled: button.matches(':disabled'), reason: button.title === reason || (region?.textContent || '').includes(reason)}))};
";

fn action_state(tab: &mut Tab, check: &'static str, region: &str) -> Result<Value, Failed> {
    run(
        tab,
        check,
        &format!(
            "(() => {{ const want = {region}; const box = () => document.querySelector('.composer textarea'); {ACTIONS} }})()",
            region = Value::str(region).render()
        ),
    )
}

fn actions(tab: &mut Tab, check: &'static str) -> Result<[Value; 2], Failed> {
    overview(tab, check)?;
    let decision = action_state(tab, check, "[data-item=D1]")?;
    select(tab, check, CARD, "p1-card")?;
    let close = action_state(tab, check, ".card")?;
    Ok([decision, close])
}

fn route(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "p1-route-actions";
    let proven = actions(tab, CHECK)?;
    let absent = rig.scratch.join("no-live-socket");
    ae::meta::rewrite(&rig.dir, "tmux_server_kind", Some("socket"))
        .unwrap_or_else(|error| panic!("P1 stopped selector kind: {error:?}"));
    ae::meta::rewrite(&rig.dir, "tmux_server", absent.to_str())
        .unwrap_or_else(|error| panic!("P1 stopped selector: {error:?}"));
    expect_page(
        tab,
        CHECK,
        "const saw = box()?.placeholder; return {ok: saw === want, saw};",
        "No proven orchestrator to send to",
    )?;
    let stopped = actions(tab, CHECK)?;
    let saw = Value::Arr([Value::Arr(proven.into()), Value::Arr(stopped.into())].into());
    tab.holds(CHECK, r"
const [proven, stopped] = JSON.parse(want);
const labels = ['Pick the recommended', 'Talk it over', 'Close with one line'];
const live = proven.flatMap(arm => arm.buttons);
const idle = stopped.flatMap(arm => arm.buttons);
const ok = live.length === 3 && idle.length === 3 && labels.every(label => live.some(button => button.label === label && !button.disabled) && idle.some(button => button.label === label && button.disabled && button.reason));
return {ok, saw: {proven, stopped}};
", &saw.render())?;
    ok(CHECK, WIDE);
    Ok(())
}

pub(super) fn check_p1(tab: &mut Tab) -> Checked {
    let rig = smoke_rig();
    let transcript = seed(&rig);
    let listener = start_listener(&rig.scratch);
    let result = (|| {
        tab.view(WIDE);
        tab.problems.clear();
        drop(tab.eval("window.__aeStale = 1"));
        let loaded = tab.call(
            "Page.navigate",
            Value::obj([("url", Value::str(listener.link()))]),
        );
        if let Some(error) = loaded.get_str("errorText") {
            return Err(Failed::new("p1-start", error));
        }
        expect_page(tab, "p1-start", READY, "")?;
        resize(tab)?;
        view_switch(tab, &transcript)?;
        fold(tab)?;
        order(tab)?;
        history(tab)?;
        provenance(tab, &rig)?;
        form_names(tab)?;
        route(tab, &rig)?;
        tab.flush();
        if !tab.problems.is_empty() {
            return Err(Failed::new("p1-console", tab.problems.join("; ")));
        }
        Ok(())
    })();
    listener.shutdown();
    result
}
