//! T4 topic navigation in the real browser, listener and orchestrator rig.

use std::io::Write;
use std::path::{Path, PathBuf};

use ae::json::Value;

use super::{Cell, Checked, Failed, PAGE_WAIT, READY, Rig, Tab, ok, smoke_rig, start_listener};

const WIDE: Cell = Cell {
    width: 1440,
    height: 900,
    dark: true,
};
const PHONE: Cell = Cell {
    width: 360,
    height: 740,
    dark: true,
};
const ALL: &str = "[data-role=tab-all]";
const FOLD: &str = "[data-group=rest] [data-action=toggle-group]";
const TOPICS: &str = "[data-role=view-topics]";
const CONVERSATION: &str = "0199c0de-3144-4890-abcd-ef0123456789";
const TITLE: &str = "<img data-t4-hostile src=x onerror=alert(1)> Quarterly supplier escalation with outstanding origin declarations and evidence from all regional brokers";

const GROUPS: &str = r"
const side = document.querySelector('nav[role=tablist]');
const groups = [...(side?.querySelectorAll('[data-group]') || [])];
const all = side?.firstElementChild;
const saw = {
  all: all?.textContent.trim(),
  selected: all?.getAttribute('aria-selected'),
  orientation: side?.getAttribute('aria-orientation'),
  groups: groups.map(group => [
    group.dataset.group,
    group.querySelector('.sg')?.textContent.trim().replace(/\s*\d+$/, ''),
    [...group.querySelectorAll('[role=tab]')].map(row => row.dataset.slug).sort()
  ])
};
const expected = JSON.parse(want);
return {ok: saw.all === 'All' && saw.selected === 'true' && saw.orientation === 'vertical' &&
  saw.groups.length === expected.length && expected.every(([id, label, slugs], at) => {
    const [actualId, actualLabel, actualSlugs] = saw.groups[at];
    return actualId === id && actualLabel === label &&
      actualSlugs.filter(slug => id !== 'moving' || !slug.startsWith('t4-fill-')).join(' ') === slugs.join(' ');
  }) &&
  new Set(saw.groups.flatMap(group => group[2])).size === saw.groups.flatMap(group => group[2]).length,
  saw};
";

const FOLDED: &str = r"
const fold = document.querySelector('[data-group=rest]');
const expanded = fold?.querySelector('[data-action=toggle-group]')?.getAttribute('aria-expanded');
const rows = [...(fold?.querySelectorAll('[role=tab]') || [])];
const saw = {expanded, slugs: rows.map(row => row.dataset.slug).sort(),
  selected: rows.filter(row => row.getAttribute('aria-selected') === 'true').map(row => row.dataset.slug)};
const [open, slugs, selected] = JSON.parse(want);
return {ok: saw.expanded === open && saw.slugs.join(' ') === slugs &&
  saw.selected.join(' ') === selected, saw};
";

const ORDER: &str = r"
const rows = [...document.querySelectorAll('[data-group=moving] [role=tab]')];
const saw = rows.map(row => row.dataset.slug).filter(slug => slug.startsWith('t4-order-'));
return {ok: saw.join(' ') === want, saw};
";

const ROW: &str = r"
const row = document.querySelector('[data-group=needs] [data-slug=t4-choice]');
const title = row?.querySelector('.tl');
const dot = row?.querySelector('.dot')?.getBoundingClientRect();
const saw = {text: title?.textContent, tooltip: row?.title, clamp: title && getComputedStyle(title).webkitLineClamp,
  height: title?.clientHeight, scrollHeight: title?.scrollHeight, dot: dot?.width,
  decisions: row?.querySelector('[data-role=decisions]')?.textContent.trim(),
  pinned: row?.hasAttribute('data-pinned'), hostile: !!document.querySelector('[data-t4-hostile]')};
return {ok: saw.text === want && !!saw.tooltip?.startsWith(want) && saw.clamp === '2' &&
  saw.height > 0 && saw.scrollHeight > saw.height && saw.dot > 0 && saw.decisions === '2 decisions' &&
  saw.pinned === true && !saw.hostile, saw};
";

const PALETTE: &str = r"
const color = raw => {
  const text = raw.trim();
  if (/^#[a-f0-9]{6}$/i.test(text)) return text.match(/[a-f0-9]{2}/gi).map(part => parseInt(part, 16));
  return /^rgb\((\d+),\s*(\d+),\s*(\d+)\)$/.exec(text)?.slice(1).map(Number);
};
const lum = rgb => rgb.map(channel => channel / 255)
  .map(s => s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4)
  .reduce((total, channel, at) => total + channel * [0.2126, 0.7152, 0.0722][at], 0);
const ratio = (a, b) => {
  if (!a || !b) return 0;
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};
const root = getComputedStyle(document.documentElement);
const token = name => color(root.getPropertyValue('--' + name));
const style = (selector, prop) => {
  const node = document.querySelector(selector);
  return node && color(getComputedStyle(node)[prop]);
};
const need = token('need');
const [r, g, b] = need || [];
const hue = r > g && g > b ? 60 * (g - b) / (r - b) : null;
const heading = style('[data-group=needs] .sg', 'color');
const dot = style('[data-slug=t4-choice] .dot', 'backgroundColor');
const badge = '[data-slug=t4-choice] [data-role=decisions]';
const badgeText = style(badge, 'color');
const badgeFill = style(badge, 'backgroundColor');
const saw = {hue, need, heading, dot, badgeText, badgeFill,
  dark: matchMedia('(prefers-color-scheme: dark)').matches,
  text: [ratio(heading, token('bg')), ratio(need, token('panel')), ratio(badgeText, badgeFill)],
  marker: [ratio(dot, token('bg')), ratio(dot, token('soft')), ratio(dot, token('accent-soft'))]};
return {ok: saw.dark === (want === 'dark') && (want === 'dark' || (hue >= 15 && hue <= 45)) &&
  need?.join() === heading?.join() && need?.join() === dot?.join() &&
  saw.text.every(value => value >= 4.5) && saw.marker.every(value => value >= 3), saw};
";

const LAYOUT: &str = r"
const side = document.querySelector('nav[role=tablist]');
const chat = document.querySelector('main .chat')?.getBoundingClientRect();
const right = document.querySelector('main .panel')?.getBoundingClientRect();
const bounds = side?.getBoundingClientRect();
const saw = {sideWidth: bounds?.width, chatWidth: chat?.width, rightWidth: right?.width,
  leftOfChat: bounds?.right <= chat?.left + 1, rightOfChat: chat?.right <= right?.left + 1,
  sideScrolls: side?.scrollHeight > side?.clientHeight && /auto|scroll/.test(getComputedStyle(side).overflowY),
  page: [document.documentElement.scrollWidth, document.documentElement.scrollHeight], viewport: [innerWidth, innerHeight]};
return {ok: Math.abs(saw.sideWidth - Number(want)) <= 2 && saw.chatWidth >= 259 && saw.rightWidth > 0 &&
  saw.leftOfChat && saw.rightOfChat && saw.sideScrolls && saw.page[0] <= innerWidth && saw.page[1] <= innerHeight, saw};
";

const VIEWS: &str = r"
const shown = selector => (document.querySelector(selector)?.getBoundingClientRect().height ?? 0) > 0;
const buttons = [...document.querySelectorAll('nav.views button')];
const saw = {nav: buttons.filter(button => button.getBoundingClientRect().height > 0)
  .map(button => button.textContent.trim()).join('|'),
  view: [['topics', 'main nav[role=tablist]'], ['chat', 'main .chat'], ['panel', 'main .panel']]
    .filter(([, selector]) => shown(selector)).map(([name]) => name).join(' '),
  width: document.documentElement.scrollWidth};
const [view, nav] = JSON.parse(want);
return {ok: saw.view === view && saw.nav === nav && saw.width <= innerWidth, saw};
";

const FILTERED: &str = r"
const chat = document.querySelector('main .chat');
const tag = chat?.querySelector('[data-role=tag-chip]');
const rows = [...(chat?.querySelectorAll('article.msg .bub') || [])].map(row => row.textContent);
const selected = document.querySelector('[role=tab][aria-selected=true]')?.dataset.slug;
const saw = {tag: tag?.dataset.slug, selected, rows};
return {ok: saw.selected === want && saw.tag === want &&
  rows.some(row => row.includes('T4 old tagged turn')) &&
  rows.some(row => row.includes('T4 new tagged turn')) &&
  !rows.some(row => row.includes('T4 other topic turn')), saw};
";

const VISIBLE: &str = r"
const side = document.querySelector('nav[role=tablist]');
const active = side?.querySelector('[role=tab][aria-selected=true]');
const outer = side?.getBoundingClientRect();
const inner = active?.getBoundingClientRect();
const saw = {slug: active?.dataset.slug, top: inner?.top, bottom: inner?.bottom,
  panelTop: outer?.top, panelBottom: outer?.bottom};
return {ok: saw.slug === want && inner.height > 0 && saw.top >= saw.panelTop - 1 &&
  saw.bottom <= saw.panelBottom + 1, saw};
";

const CARD: &str = r#"
const card = document.querySelector('[data-panel=topic][data-slug="' + want + '"]');
const saw = {slug: card?.dataset.slug, height: card?.getBoundingClientRect().height};
return {ok: saw.slug === want && saw.height > 0, saw};
"#;

const FIND_ALL_ROW: &str = r"
const row = [...document.querySelectorAll('[data-panel=all] button.row')].find(button =>
  button.querySelector('.ln')?.textContent.split(' ')[0] === '#' + want);
if (row) row.dataset.t4AllRow = want;
return {ok: !!row, saw: row?.textContent};
";

fn wait(tab: &mut Tab, check: &'static str, body: &str, want: &str) -> Checked {
    tab.until(check, body, want, PAGE_WAIT).map(drop)
}

fn script(tab: &mut Tab, check: &'static str, body: &str) -> Checked {
    tab.eval(body)
        .map(drop)
        .map_err(|why| Failed::new(check, why))
}

fn memo(rig: &Rig, time: &str, slug: &str, fields: &str) {
    ae::store::open(&rig.dir)
        .append_memo(format!("2026-11-24T10:00:{time}Z\ttui\ttopic-{slug}\t{fields}\n").as_bytes())
        .unwrap_or_else(|error| panic!("publish T4 topic #{slug}: {error}"));
}

fn turn(path: &Path, second: u8, body: &str) {
    let body = Value::str(body).render();
    let line = format!(
        concat!(
            r#"{{"type":"user","timestamp":"2026-11-24T10:00:{second:02}Z","message":{{"role":"user","content":"T4 fixture prompt"}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"2026-11-24T10:00:{second:02}Z","message":{{"role":"assistant","content":[{{"type":"text","text":{body}}}]}}}}"#,
            "\n",
        ),
        second = second,
        body = body,
    );
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|error| panic!("open T4 transcript: {error}"))
        .write_all(line.as_bytes())
        .unwrap_or_else(|error| panic!("append T4 transcript turn: {error}"));
}

fn seed(rig: &Rig) -> PathBuf {
    memo(
        rig,
        "00",
        "t4-choice",
        &format!("title: {TITLE} | state: quiet"),
    );
    memo(rig, "00", "t4-priority", "state: moving");
    memo(rig, "00", "t4-moving-pin", "state: moving");
    memo(rig, "00", "t4-pinned", "state: quiet");
    memo(
        rig,
        "00",
        "t4-pinned-other",
        "title: An unreadable pinned topic",
    );
    memo(rig, "00", "t4-other", "title: An unreadable topic");
    memo(
        rig,
        "00",
        "t4-order-a",
        "title: Earlier review | state: moving",
    );
    memo(
        rig,
        "05",
        "t4-order-z",
        "title: Later review | state: moving",
    );
    for i in 0..32 {
        memo(
            rig,
            "00",
            &format!("t4-fill-{i:02}"),
            "title: More moving work | state: moving",
        );
    }
    let items = ae::desk::parse_input(
        b"DECIDE\nD71 | Proceed? 1) Go 2) Hold #t4-choice #t4-priority\n\
          D72 | Schedule? 1) Now 2) Later #t4-choice\n",
    )
    .unwrap_or_else(|error| panic!("T4 fixture decision grammar: {error:?}"));
    ae::deskstore::set(
        &rig.dir,
        items,
        ae::time::Timestamp::from_epoch(1_795_000_000),
        "tui",
    )
    .unwrap_or_else(|error| panic!("publish T4 fixture board: {error}"));
    let ask = Value::obj([
        ("ts", Value::str("2026-11-24T10:00:00Z")),
        ("actor", Value::str(ae::tracked::CONSOLE_SINK)),
        ("action", Value::str("ask")),
        ("target", Value::str("lead")),
        ("target_slot", Value::str("main")),
        ("target_session", Value::str(&rig.session)),
        ("ref", Value::str("ae-20261124T100000Z-00000001")),
        (
            "summary",
            Value::str("#t4-bare A topic with only an open ask"),
        ),
    ]);
    ae::store::open(&rig.dir)
        .append_event(&format!("{}\n", ask.render()))
        .unwrap_or_else(|error| panic!("publish T4 fixture ask: {error}"));

    let store = rig.scratch.join("claude-t4");
    let transcript = store.join("projects/t4");
    std::fs::create_dir_all(&transcript)
        .unwrap_or_else(|error| panic!("create T4 transcript project: {error}"));
    ae::meta::rewrite(&rig.dir, "harness_session.main", Some(CONVERSATION))
        .unwrap_or_else(|error| panic!("publish T4 fixture conversation: {error:?}"));
    ae::meta::rewrite(&rig.dir, "config_home.main", store.to_str())
        .unwrap_or_else(|error| panic!("publish T4 fixture home: {error:?}"));
    let path = transcript.join(format!("{CONVERSATION}.jsonl"));
    turn(&path, 0, "#t4-order-z T4 other topic turn");
    turn(&path, 1, "#t4-order-a T4 old tagged turn");
    path
}

fn groups(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "t4-groups";
    wait(
        tab,
        CHECK,
        GROUPS,
        r#"[
      ["needs","Needs you",["smoke","t4-choice","t4-priority"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z"]],
      ["other","Other",["t4-other","t4-pinned-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    tab.holds(CHECK, FOLDED, r#"["false","",""]"#)?;
    tab.click_selector(CHECK, FOLD)?;
    wait(
        tab,
        CHECK,
        FOLDED,
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z t4-bare t4-pinned",""]"#,
    )?;
    tab.click_selector(CHECK, FOLD)?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
    script(
        tab,
        CHECK,
        r#"
const key = 'ae-web-pins';
localStorage.setItem(key, '["t4-choice","t4-priority","t4-moving-pin","t4-pinned","t4-pinned-other"]');
dispatchEvent(new StorageEvent('storage', {key, storageArea: localStorage}));
"#,
    )?;
    wait(
        tab,
        CHECK,
        GROUPS,
        r#"[
      ["needs","Needs you",["smoke","t4-choice","t4-priority"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z"]],
      ["pinned","Pinned",["t4-pinned","t4-pinned-other"]],
      ["other","Other",["t4-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    memo(rig, "03", "t4-other", "state: moving");
    wait(
        tab,
        CHECK,
        GROUPS,
        r#"[
      ["needs","Needs you",["smoke","t4-choice","t4-priority"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z","t4-other"]],
      ["pinned","Pinned",["t4-pinned","t4-pinned-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    ok(CHECK, WIDE);
    Ok(())
}

fn opened(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t4-opened";
    tab.holds(CHECK, FIND_ALL_ROW, "s2-z")?;
    tab.click_selector(CHECK, "[data-t4-all-row=s2-z]")?;
    wait(tab, CHECK, CARD, "s2-z")?;
    tab.holds(CHECK, FOLDED, r#"["false","s2-z","s2-z"]"#)?;
    tab.click_selector(CHECK, ALL)?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
    tab.click_selector(CHECK, FOLD)?;
    wait(
        tab,
        CHECK,
        FOLDED,
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z t4-bare",""]"#,
    )?;
    script(tab, CHECK, "window.__aeStale = 1")?;
    tab.call("Page.reload", Value::Obj(Vec::new()));
    wait(tab, CHECK, READY, "")?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
    tab.click_selector(CHECK, FOLD)?;
    wait(
        tab,
        CHECK,
        FOLDED,
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z t4-bare",""]"#,
    )?;
    tab.click_selector(CHECK, "[data-group=rest] [data-slug=s2-parked]")?;
    wait(tab, CHECK, CARD, "s2-parked")?;
    tab.click_selector(CHECK, FOLD)?;
    wait(tab, CHECK, FOLDED, r#"["false","s2-parked","s2-parked"]"#)?;
    tab.click_selector(CHECK, ALL)?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
    ok(CHECK, WIDE);
    Ok(())
}

fn layout(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t4-layout";
    for (width, panel) in [(1440, "260"), (1000, "220"), (760, "220")] {
        let cell = Cell { width, ..WIDE };
        tab.view(cell);
        wait(tab, CHECK, LAYOUT, panel)?;
        ok(CHECK, cell);
    }
    Ok(())
}

fn palette(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t4-palette";
    for dark in [false, true] {
        let cell = Cell { dark, ..WIDE };
        tab.view(cell);
        wait(tab, CHECK, PALETTE, cell.scheme())?;
        ok(CHECK, cell);
    }
    Ok(())
}

fn narrow(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t4-narrow";
    tab.view(PHONE);
    wait(tab, CHECK, VIEWS, r#"["chat","Topics|Chat|Overview"]"#)?;
    tab.click_selector(CHECK, TOPICS)?;
    wait(tab, CHECK, VIEWS, r#"["topics","Topics|Chat|Overview"]"#)?;
    tab.click_selector(CHECK, "[data-role=tab][data-slug=t4-order-a]")?;
    wait(tab, CHECK, VIEWS, r#"["chat","Topics|Chat|Topic"]"#)?;
    wait(tab, CHECK, FILTERED, "t4-order-a")?;
    tab.click_selector(CHECK, "[data-role=view-panel]")?;
    wait(tab, CHECK, VIEWS, r#"["panel","Topics|Chat|Topic"]"#)?;
    wait(tab, CHECK, CARD, "t4-order-a")?;
    tab.click_selector(CHECK, TOPICS)?;
    wait(tab, CHECK, VIEWS, r#"["topics","Topics|Chat|Topic"]"#)?;
    wait(tab, CHECK, VISIBLE, "t4-order-a")?;
    ok(CHECK, PHONE);
    Ok(())
}

pub(super) fn check_t4(tab: &mut Tab) -> Checked {
    let rig = smoke_rig();
    let transcript = seed(&rig);
    let listener = start_listener(&rig.scratch);
    tab.view(WIDE);
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(listener.link()))]),
    );
    let result = (|| {
        if let Some(error) = loaded.get_str("errorText") {
            return Err(Failed::new("t4-start", error));
        }
        wait(tab, "t4-start", READY, "")?;
        groups(tab, &rig)?;
        wait(tab, "t4-order", ORDER, "t4-order-z t4-order-a")?;
        turn(&transcript, 10, "#t4-order-a T4 new tagged turn");
        wait(tab, "t4-order", ORDER, "t4-order-a t4-order-z")?;
        ok("t4-order", WIDE);
        tab.holds("t4-row", ROW, TITLE)?;
        ok("t4-row", WIDE);
        palette(tab)?;
        opened(tab)?;
        layout(tab)?;
        narrow(tab)
    })();
    listener.shutdown();
    result
}
