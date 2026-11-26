//! T2 tab lifecycle with a real browser, listener, and desk.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use ae::json::Value;

use super::{Cell, Checked, Failed, PAGE_WAIT, READY, Rig, Tab, ok, smoke_rig, start_listener};

const WIDE: Cell = Cell {
    width: 1440,
    height: 900,
    dark: true,
};
const NARROW: Cell = Cell {
    width: 360,
    height: 740,
    dark: true,
};
const ALL: &str = "[data-role=tab-all]";
const PIN: &str = "button[data-action=pin]";
const TOPICS_VIEW: &str = "[data-role=view-topics]";
const TARGET: &str = "t2-99";
const CURRENT: &str = "0199c0de-3122-4890-abcd-ef0123456789";
const NEXT: &str = "0199c0de-3123-4890-abcd-ef0123456789";

const CLIPPED: &str = r#"
const bar = document.querySelector('[role=tablist]');
const all = bar.querySelector('[data-role=tab-all][aria-selected=true]');
const target = bar.querySelector('[role=tab][data-slug="' + want + '"]');
const edge = bar.getBoundingClientRect().bottom;
const saw = {all: !!all, allVisible: all?.getBoundingClientRect().bottom <= edge,
  bottom: target?.getBoundingClientRect().bottom, edge, offset: bar.scrollTop,
  width: document.documentElement.scrollWidth};
return {ok: bar.clientHeight > 0 && saw.all && saw.allVisible && saw.bottom > edge + 1 &&
  saw.offset < 1 && saw.width <= innerWidth, saw};
"#;

const VISIBLE: &str = r"
const bar = document.querySelector('[role=tablist]');
const selected = bar.querySelector('[role=tab][aria-selected=true]');
const outer = bar.getBoundingClientRect();
const inner = selected?.getBoundingClientRect();
const saw = {slug: selected?.dataset.slug, top: inner?.top, bottom: inner?.bottom,
  panelTop: outer.top, panelBottom: outer.bottom, offset: bar.scrollTop};
return {ok: inner?.height > 0 && saw.slug === want && saw.offset > 0 &&
  saw.top >= saw.panelTop - 1 && saw.bottom <= saw.panelBottom + 1, saw};
";

const WHEEL_POINT: &str = r"
const bar = document.querySelector('[role=tablist]');
const r = bar.getBoundingClientRect();
const x = Math.floor(r.left + r.width / 2);
const y = Math.floor(r.top + r.height / 2);
const saw = {x, y, canScroll: bar.scrollHeight > bar.clientHeight};
return {ok: saw.canScroll && bar.contains(document.elementFromPoint(x, y)), x, y, saw};
";

const WHEEL_MOVED: &str = r"
const bar = document.querySelector('[role=tablist]');
const saw = {offset: bar.scrollTop, page: scrollY};
return {ok: saw.offset > 0 && saw.page === 0, saw};
";

const SCROLL_STAYS: &str = r"
const bar = document.querySelector('[role=tablist]');
const selected = bar.querySelector('[role=tab][aria-selected=true]');
const marker = bar.querySelector('[data-slug=t2-00]');
const clipped = selected?.getBoundingClientRect().bottom > bar.getBoundingClientRect().bottom + 1;
const saw = {offset: bar.scrollTop, slug: selected?.dataset.slug, marker: !!marker, clipped};
return {ok: saw.marker && saw.slug === want && saw.offset < 1 && saw.clipped, saw};
";

const KNOWN_LATE: &str = r"
const saw = !!document.querySelector('[role=tab][data-slug=t2-future]');
return {ok: saw, saw};
";

const BADGES: &str = r"
const [expected, marker] = JSON.parse(want);
const tabs = [...document.querySelectorAll('[role=tablist] [role=tab]')];
const badges = Object.fromEntries(tabs.flatMap(t => {
  const count = t.querySelector('[data-role=unread]');
  return count ? [[t.dataset.slug || 'all', count.textContent.trim()]] : [];
}));
const messages = [...document.querySelectorAll('article.msg .bub')];
const shown = messages.some(row => row.textContent.includes(marker));
const saw = {badges, shown};
return {ok: shown && Object.keys(badges).length === Object.keys(expected).length &&
  Object.entries(expected).every(([slug, count]) => badges[slug] === count), saw};
";

const ROLE_ABSENT: &str = r"
const chip = document.querySelector('.top .chip')?.textContent;
const rows = document.querySelectorAll('article.msg').length;
return {ok: chip === 'no orchestrator · absent' && rows === 0, saw: {chip, rows}};
";

const PRIOR: &str = r"
const old = [...document.querySelectorAll('article.msg')].find(row => row.textContent.includes(want));
const saw = old?.querySelector('.who span')?.textContent || '';
return {ok: saw.includes('prior 1'), saw};
";

const SELECTED: &str = r"
const tab = document.querySelector('[role=tab][aria-selected=true]');
const panel = document.querySelector(want ? '[data-panel=topic]' : '[data-panel=all]');
const saw = {tab: tab ? tab.dataset.slug || '' : null,
  card: panel ? panel.dataset.slug || '' : null};
return {ok: saw.tab === want && saw.card === want, saw};
";

const MARK_ROW: &str = r"
const row = [...document.querySelectorAll('[data-panel=all] button.row')].find(button =>
  button.querySelector('.ln')?.textContent.split(' ')[0] === '#' + want);
if (row) row.dataset.t2Row = want;
return {ok: !!row, saw: row?.textContent};
";

const PIN_STATE: &str = r#"
const [slug, expected] = JSON.parse(want);
const panel = document.querySelector('[data-panel=topic][data-slug="' + slug + '"]');
const button = panel?.querySelector('button[data-action=pin]');
const tab = document.querySelector('[role=tab][data-slug="' + slug + '"]');
const saw = {pressed: button?.getAttribute('aria-pressed'), disabled: button?.disabled,
  pinned: !!tab?.hasAttribute('data-pinned'), height: button?.getBoundingClientRect().height};
return {ok: saw.pressed === String(expected) && saw.pinned === expected && saw.height >= 44, saw};
"#;

const STORAGE: &str = r"
const raw = localStorage.getItem('ae-web-pins');
const saw = raw === null ? null : JSON.parse(raw);
const expected = JSON.parse(want);
return {ok: Array.isArray(saw) && saw.length === expected.length &&
  expected.every(slug => saw.includes(slug)), saw};
";

const CORRUPT: &str = r"
const saw = {raw: localStorage.getItem('ae-web-pins'), pins: document.querySelectorAll('[data-pinned]').length,
  all: document.querySelector('[data-role=tab-all]')?.getAttribute('aria-selected')};
return {ok: saw.raw === want && saw.pins === 0 && saw.all === 'true', saw};
";

const PINNED_STATE: &str = r#"
const [slug, state] = JSON.parse(want);
const panel = document.querySelector('[data-panel=topic][data-slug="' + slug + '"]');
const selected = document.querySelector('[role=tab][aria-selected=true][data-slug="' + slug + '"]');
const fields = [...(panel?.querySelectorAll('.fld p.raw') || [])].map(p => p.textContent);
const saw = {pinned: !!selected?.hasAttribute('data-pinned'), fields};
return {ok: saw.pinned && fields.includes(state), saw};
"#;

const CLOSED_DECISION: &str = r#"
const card = document.querySelector('[data-panel=topic][data-slug="' + want + '"]');
const selected = document.querySelector('[role=tab][aria-selected=true][data-slug="' + want + '"]');
const fields = [...(card?.querySelectorAll('.fld p.raw') || [])].map(p => p.textContent);
const choice = card?.querySelector('form[data-item=D71]');
const saw = {selected: !!selected, closed: fields.includes('closed'), choice: !!choice};
return {ok: saw.selected && saw.closed && saw.choice, saw};
"#;

const LEFT_BAR: &str = r#"
const all = document.querySelector('[data-role=tab-all]')?.getAttribute('aria-selected');
const gone = !document.querySelector('[role=tab][data-slug="' + want + '"]');
const saw = {all, gone};
return {ok: all === 'true' && gone && !!document.querySelector('[data-panel=all]'), saw};
"#;

const CAP: &str = r#"
const panel = document.querySelector('[data-panel=topic][data-slug="' + want + '"]');
const button = panel?.querySelector('button[data-action=pin]');
const pins = [...document.querySelectorAll('[role=tab][data-pinned]')].map(tab => tab.dataset.slug);
const expected = Array.from({length: 32}, (_, i) => 't2-' + String(i + 10));
const saw = {pins, disabled: button?.disabled, pressed: button?.getAttribute('aria-pressed')};
return {ok: saw.disabled === true && saw.pressed === 'false' &&
  pins.length === expected.length && expected.every(slug => pins.includes(slug)), saw};
"#;

fn script(tab: &mut Tab, check: &'static str, code: &str) -> Checked {
    tab.eval(code)
        .map(drop)
        .map_err(|why| Failed::new(check, why))
}

fn wait(tab: &mut Tab, check: &'static str, body: &str, want: &str) -> Checked {
    tab.until(check, body, want, PAGE_WAIT).map(drop)
}

fn select(tab: &mut Tab, check: &'static str, slug: &str) -> Checked {
    let selector = if slug.is_empty() {
        ALL.to_owned()
    } else {
        format!(r#"[role=tab][data-slug="{slug}"]"#)
    };
    tab.click_selector(check, &selector)?;
    wait(tab, check, SELECTED, slug)
}

fn open_row(tab: &mut Tab, check: &'static str, slug: &str) -> Checked {
    tab.holds(check, MARK_ROW, slug)?;
    tab.click_selector(check, &format!(r#"[data-t2-row="{slug}"]"#))?;
    wait(tab, check, SELECTED, slug)
}

fn memo(rig: &Rig, slug: &str, fields: &str) {
    ae::store::open(&rig.dir)
        .append_memo(format!("2026-11-24T08:00:00Z\ttui\ttopic-{slug}\t{fields}\n").as_bytes())
        .expect("publish T2 topic record");
}

fn board(rig: &Rig, decision: bool) {
    let items = if decision {
        ae::desk::parse_input(b"DECIDE\nD71 | Proceed? 1) Go 2) Hold #t2-99\n")
            .expect("T2 fixture decision")
    } else {
        Vec::new()
    };
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "tui",
    )
    .expect("publish T2 fixture board");
}

fn turn(path: &Path, second: u8, human: &str, assistant: &str) {
    let human = Value::str(human).render();
    let assistant = Value::str(assistant).render();
    let lines = format!(
        concat!(
            r#"{{"type":"user","timestamp":"2026-11-24T10:00:{second:02}Z","message":{{"role":"user","content":{human}}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"2026-11-24T10:00:{second:02}Z","message":{{"role":"assistant","content":[{{"type":"text","text":{assistant}}}]}}}}"#,
            "\n",
        ),
        second = second,
        human = human,
        assistant = assistant,
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open T2 transcript")
        .write_all(lines.as_bytes())
        .expect("append T2 transcript turn");
}

#[allow(
    clippy::disallowed_methods,
    reason = "read this isolated browser rig's own meta fixture"
)]
fn conversation(rig: &Rig, id: &str, prior: Option<&str>) -> PathBuf {
    let path = rig.dir.join("meta");
    let before = std::fs::read_to_string(&path).expect("T2 fixture meta");
    let store = rig.scratch.join("claude-t2");
    let lines = before
        .lines()
        .filter(|line| {
            !line.starts_with("harness_session.main=")
                && !line.starts_with("config_home.main=")
                && !line.starts_with("harness_session_prior.main=")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let prior = prior.map_or_else(String::new, |prior| {
        format!("harness_session_prior.main=claude:{prior}\n")
    });
    let replacement = path.with_extension("t2-new");
    std::fs::write(
        &replacement,
        format!(
            "{lines}\nharness_session.main={id}\nconfig_home.main={}\n{prior}",
            store.display()
        ),
    )
    .expect("publish T2 fixture conversation");
    std::fs::rename(replacement, path).expect("replace T2 fixture meta");
    let transcript = store.join("projects/t2");
    std::fs::create_dir_all(&transcript).expect("create T2 transcript project");
    transcript.join(format!("{id}.jsonl"))
}

fn overflow(tab: &mut Tab, rig: &Rig) -> Checked {
    const WHEEL: &str = "t2-wheel";
    const SCROLL: &str = "t2-scroll";
    tab.holds(WHEEL, CLIPPED, TARGET)?;
    let point = tab.until(WHEEL, WHEEL_POINT, "", PAGE_WAIT)?;
    tab.call(
        "Input.dispatchMouseEvent",
        Value::obj([
            ("type", Value::str("mouseWheel")),
            ("x", point.get("x").expect("wheel x").clone()),
            ("y", point.get("y").expect("wheel y").clone()),
            ("deltaX", Value::Num(0)),
            ("deltaY", Value::Num(700)),
        ]),
    );
    wait(tab, WHEEL, WHEEL_MOVED, "")?;
    ok(WHEEL, WIDE);

    tab.view(NARROW);
    tab.click_selector(SCROLL, TOPICS_VIEW)?;
    script(
        tab,
        SCROLL,
        "document.querySelector('[role=tablist]').scrollTop = 0",
    )?;
    wait(tab, SCROLL, CLIPPED, TARGET)?;
    tab.click(SCROLL, "panel")?;
    // A side row click would scroll that row inside the harness before the product acted.
    open_row(tab, SCROLL, TARGET)?;
    tab.click_selector(SCROLL, TOPICS_VIEW)?;
    wait(tab, SCROLL, VISIBLE, TARGET)?;
    script(
        tab,
        SCROLL,
        "document.querySelector('[role=tablist]').scrollTop = 0",
    )?;
    memo(rig, "t2-00", "title: Newly inserted | state: needs");
    wait(tab, SCROLL, SCROLL_STAYS, TARGET)?;
    ok(SCROLL, NARROW);

    tab.view(WIDE);
    Ok(())
}

fn unread(tab: &mut Tab, rig: &Rig, path: &Path) -> Checked {
    const CHECK: &str = "t2-unread";
    select(tab, CHECK, "")?;
    wait(tab, CHECK, BADGES, r#"[{},"T2 baseline"]"#)?;
    turn(
        path,
        2,
        "#t2-01 T2 owner",
        "#t2-01 #t2-02 #t2-future T2 pair",
    );
    wait(
        tab,
        CHECK,
        BADGES,
        r#"[{"t2-01":"1","t2-02":"1"},"T2 pair"]"#,
    )?;
    tab.holds(CHECK, BADGES, r#"[{"t2-01":"1","t2-02":"1"},"T2 owner"]"#)?;
    // Two independently recorded turns can have the same visible identity.
    turn(
        path,
        2,
        "T2 repeat prompt",
        "#t2-01 #t2-02 #t2-future T2 pair",
    );
    wait(
        tab,
        CHECK,
        BADGES,
        r#"[{"t2-01":"2","t2-02":"2"},"T2 repeat prompt"]"#,
    )?;
    // Knowing a topic now can add a tag to an old row without adding a turn.
    memo(rig, "t2-future", "title: Later topic | state: moving");
    wait(tab, CHECK, KNOWN_LATE, "")?;
    tab.holds(
        CHECK,
        BADGES,
        r#"[{"t2-01":"2","t2-02":"2"},"T2 baseline"]"#,
    )?;
    select(tab, CHECK, "t2-01")?;
    wait(tab, CHECK, BADGES, r#"[{"t2-02":"2"},"T2 pair"]"#)?;
    turn(path, 3, "T2 next prompt", "#t2-01 #t2-02 T2 next");
    wait(tab, CHECK, BADGES, r#"[{"t2-02":"3"},"T2 next"]"#)?;

    // A standing console reads predecessors only when it opens. Let the
    // listener see the role absent, then restore it without reloading the page.
    let next = conversation(rig, NEXT, Some(CURRENT));
    turn(&next, 4, "T2 new conversation", "T2 untagged answer");
    ae::meta::rewrite(&rig.dir, "meta_agent", None).expect("remove the T2 fixture role");
    let gap = wait(tab, CHECK, ROLE_ABSENT, "").and_then(|()| wait(tab, CHECK, SELECTED, ""));
    ae::meta::rewrite(&rig.dir, "meta_agent", Some("true")).expect("restore the T2 fixture role");
    gap?;
    wait(tab, CHECK, PRIOR, "T2 baseline")?;
    tab.holds(CHECK, BADGES, r#"[{"t2-02":"3"},"T2 next"]"#)?;
    select(tab, CHECK, "t2-01")?;
    ok(CHECK, WIDE);
    Ok(())
}

fn pins_and_close(tab: &mut Tab, rig: &Rig) -> Checked {
    const PIN_CHECK: &str = "t2-pin";
    const CLOSE: &str = "t2-close";
    let pin = r#"["t2-01",true]"#;
    wait(tab, PIN_CHECK, PIN_STATE, r#"["t2-01",false]"#)?;
    // A different tab can write the same key before this tab sees its event.
    script(
        tab,
        PIN_CHECK,
        r#"localStorage.setItem('ae-web-pins', '["t2-02"]')"#,
    )?;
    tab.click_selector(PIN_CHECK, PIN)?;
    wait(tab, PIN_CHECK, PIN_STATE, pin)?;
    tab.holds(PIN_CHECK, STORAGE, r#"["t2-01","t2-02"]"#)?;
    memo(rig, "t2-01", "state: quiet");
    wait(tab, PIN_CHECK, PINNED_STATE, r#"["t2-01","quiet"]"#)?;
    select(tab, PIN_CHECK, "")?;
    select(tab, PIN_CHECK, "t2-01")?;

    memo(rig, "t2-01", "state: closed | close: T2 pinned close");
    wait(tab, CLOSE, PINNED_STATE, r#"["t2-01","closed"]"#)?;
    script(tab, PIN_CHECK, "window.__aeStale = 1")?;
    tab.call("Page.reload", Value::Obj(Vec::new()));
    wait(tab, PIN_CHECK, READY, "")?;
    wait(tab, PIN_CHECK, SELECTED, "")?;
    wait(tab, PIN_CHECK, BADGES, r#"[{},"T2 next"]"#)?;
    select(tab, PIN_CHECK, "t2-01")?;
    wait(tab, PIN_CHECK, PIN_STATE, pin)?;
    tab.click_selector(PIN_CHECK, PIN)?;
    wait(tab, PIN_CHECK, PIN_STATE, r#"["t2-01",false]"#)?;
    tab.holds(PIN_CHECK, STORAGE, r#"["t2-02"]"#)?;
    select(tab, PIN_CHECK, "")?;
    tab.holds(PIN_CHECK, LEFT_BAR, "t2-01")?;
    ok(PIN_CHECK, WIDE);

    open_row(tab, CLOSE, "t2-manual")?;
    memo(
        rig,
        "t2-manual",
        "state: closed | close: T2 temporary close",
    );
    wait(tab, CLOSE, LEFT_BAR, "t2-manual")?;
    open_row(tab, CLOSE, "t2-manual")?;
    select(tab, CLOSE, "t2-99")?;
    memo(
        rig,
        "t2-99",
        "state: closed | close: T2 decision still open",
    );
    wait(tab, CLOSE, CLOSED_DECISION, "t2-99")?;
    board(rig, false);
    wait(tab, CLOSE, LEFT_BAR, "t2-99")?;
    ok(CLOSE, WIDE);
    Ok(())
}

fn pin_input(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t2-pin-input";
    script(
        tab,
        CHECK,
        "localStorage.setItem('ae-web-pins', '{broken'); window.__aeStale = 1",
    )?;
    tab.call("Page.reload", Value::Obj(Vec::new()));
    wait(tab, CHECK, READY, "")?;
    tab.holds(CHECK, CORRUPT, "{broken")?;
    select(tab, CHECK, "t2-02")?;
    script(
        tab,
        CHECK,
        r"
const pins = ['MIXED', null, {slug: 't2-02'}, '-invalid', 'a'.repeat(49),
  't2-10', 't2-10', ...Array.from({length: 32}, (_, i) => 't2-' + String(i + 11))];
const key = 'ae-web-pins';
localStorage.setItem(key, JSON.stringify(pins));
dispatchEvent(new StorageEvent('storage', {key, storageArea: localStorage}));
",
    )?;
    wait(tab, CHECK, CAP, "t2-02")?;
    select(tab, CHECK, "t2-10")?;
    tab.click_selector(CHECK, PIN)?;
    let want = Value::Arr((11..42).map(|i| Value::str(format!("t2-{i}"))).collect()).render();
    tab.holds(CHECK, STORAGE, &want)?;
    ok(CHECK, WIDE);
    Ok(())
}

pub(super) fn check_t2(tab: &mut Tab) -> Checked {
    let rig = smoke_rig();
    for i in (1..=2).chain(10..=42).chain([99]) {
        memo(
            &rig,
            &format!("t2-{i:02}"),
            &format!("title: T2 topic {i:02} with plenty of words | state: moving"),
        );
    }
    memo(&rig, "t2-manual", "title: T2 manual | state: quiet");
    let path = conversation(&rig, CURRENT, None);
    turn(
        &path,
        1,
        "T2 initial question",
        "#t2-01 #t2-future T2 baseline",
    );
    let listener = start_listener(&rig.scratch);
    tab.view(WIDE);
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(listener.link()))]),
    );
    let result = if let Some(error) = loaded.get_str("errorText") {
        Err(Failed::new(
            "t2-start",
            format!("could not load T2 page: {error}"),
        ))
    } else {
        (|| {
            wait(tab, "t2-ready", READY, "")?;
            overflow(tab, &rig)?;
            board(&rig, true);
            unread(tab, &rig, &path)?;
            pins_and_close(tab, &rig)?;
            pin_input(tab)
        })()
    };
    drop(tab.eval("localStorage.removeItem('ae-web-pins')"));
    listener.shutdown();
    result
}
