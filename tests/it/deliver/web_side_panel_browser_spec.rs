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
const ALL: &str = "button[data-role=side-all]";
const FOLD: &str = "[data-group=rest] [data-action=toggle-group]";
const TOPICS: &str = "[data-role=view-topics]";
const ASK: &str = "ae-20261124T100000Z-00000001";
const CONVERSATION: &str = "0199c0de-3144-4890-abcd-ef0123456789";
const TITLE: &str = "<img data-t4-hostile src=x onerror=alert(1)> Quarterly supplier escalation with outstanding origin declarations and evidence from all regional brokers";

const GROUPS: &str = r"
const side = document.querySelector('nav[aria-label=Topics]');
const groups = [...(side?.querySelectorAll('[data-group]') || [])];
const all = side?.firstElementChild?.querySelector('button');
const tops = [...(side?.querySelectorAll('button:is([data-role=side-all],[data-role=side-row])') || [])]
  .map(row => row.getBoundingClientRect().top);
const saw = {
  all: all?.textContent.trim(),
  selected: all?.getAttribute('aria-current'),
  vertical: tops.length > 1 && tops.every((top, at) => at === 0 || top > tops[at - 1]),
  groups: groups.map(group => [
    group.dataset.group,
    group.querySelector('.sg')?.textContent.trim().replace(/\s*\d+$/, ''),
    [...group.querySelectorAll('button[data-role=side-row]')].map(row => row.dataset.slug).sort()
  ])
};
const expected = JSON.parse(want);
return {ok: saw.all === 'All' && saw.selected === 'true' && saw.vertical &&
  saw.groups.length === expected.length && expected.every(([id, label, slugs], at) => {
    const [actualId, actualLabel, actualSlugs] = saw.groups[at];
    return actualId === id && actualLabel === label &&
      actualSlugs.filter(slug => id !== 'moving' || !slug.startsWith('t4-fill-')).join(' ') === slugs.join(' ');
  }) &&
  new Set(saw.groups.flatMap(group => group[2])).size === saw.groups.flatMap(group => group[2]).length,
  saw};
";

const ASK_ONLY: &str = r"
const rows = ['s2-a', 't4-bare', 't4-waiting'].map(slug => document.querySelector(
  '[data-group=needs] [data-slug=' + slug + ']'));
const saw = rows.map(row => ({
  marker: !!row?.querySelector('.dot.needs_you'),
  decisions: !!row?.querySelector('[data-role=decisions]')
}));
return {ok: saw.every(row => row.marker && !row.decisions), saw};
";

const FOLDED: &str = r"
const fold = document.querySelector('[data-group=rest]');
const expanded = fold?.querySelector('[data-action=toggle-group]')?.getAttribute('aria-expanded');
const rows = [...(fold?.querySelectorAll('button[data-role=side-row]') || [])];
const saw = {expanded, slugs: rows.map(row => row.dataset.slug).sort(),
  selected: rows.filter(row => row.getAttribute('aria-current') === 'true').map(row => row.dataset.slug)};
const [open, slugs, selected] = JSON.parse(want);
return {ok: saw.expanded === open && saw.slugs.join(' ') === slugs &&
  saw.selected.join(' ') === selected, saw};
";

const ORDER: &str = r"
const rows = [...document.querySelectorAll('[data-group=moving] button[data-role=side-row]')];
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
const side = document.querySelector('nav[aria-label=Topics]');
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
  view: [['topics', 'main nav[aria-label=Topics]'], ['chat', 'main .chat'], ['panel', 'main .panel']]
    .filter(([, selector]) => shown(selector)).map(([name]) => name).join(' '),
  width: document.documentElement.scrollWidth};
const [view, nav] = JSON.parse(want);
return {ok: saw.view === view && saw.nav === nav && saw.width <= innerWidth, saw};
";

const FILTERED: &str = r"
const chat = document.querySelector('main .chat');
const tag = chat?.querySelector('[data-role=tag-chip]');
const rows = [...(chat?.querySelectorAll('article.msg .bub') || [])].map(row => row.textContent);
const selected = document.querySelector('nav[aria-label=Topics] button[aria-current=true]')?.dataset.slug;
const saw = {tag: tag?.dataset.slug, selected, rows};
return {ok: saw.selected === want && saw.tag === want &&
  rows.some(row => row.includes('T4 old tagged turn')) &&
  rows.some(row => row.includes('T4 new tagged turn')) &&
  !rows.some(row => row.includes('T4 other topic turn')), saw};
";

const VISIBLE: &str = r"
const side = document.querySelector('nav[aria-label=Topics]');
const active = side?.querySelector('button[aria-current=true]');
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

const WAITING_CARD: &str = r"
const row = document.querySelector('[data-group=needs] button[data-role=side-row][data-slug=t4-order-a][aria-current=true]');
const card = document.querySelector('[data-panel=topic][data-slug=t4-order-a]');
const fields = [...(card?.querySelectorAll('.fld p.raw') || [])].map(field => field.textContent);
const waits = [...(card?.querySelectorAll('.fld.need li') || [])].map(wait => wait.textContent);
const saw = {selected: !!row, fields, waits};
return {ok: saw.selected && fields.includes('quiet') && waits.length === 1 && waits[0].includes(want), saw};
";

const ALL_CARD: &str = r"
const all = document.querySelector('button[data-role=side-all]');
const card = document.querySelector('[data-panel=all]');
const saw = {selected: all?.getAttribute('aria-current'), overview: !!card};
return {ok: saw.selected === 'true' && saw.overview, saw};
";

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

fn publish_ask(rig: &Rig, id: &str, summary: &str) {
    let ask = Value::obj([
        ("ts", Value::str("2026-11-24T10:00:00Z")),
        ("actor", Value::str(ae::tracked::CONSOLE_SINK)),
        ("action", Value::str("ask")),
        ("target", Value::str("lead")),
        ("target_slot", Value::str("main")),
        ("target_session", Value::str(&rig.session)),
        ("ref", Value::str(id)),
        ("summary", Value::str(summary)),
    ]);
    ae::store::open(&rig.dir)
        .append_event(&format!("{}\n", ask.render()))
        .unwrap_or_else(|error| panic!("publish T4 fixture ask: {error}"));
}

fn seed(rig: &Rig) -> PathBuf {
    memo(
        rig,
        "00",
        "t4-choice",
        &format!("title: {TITLE} | state: quiet"),
    );
    memo(rig, "00", "t4-priority", "state: moving");
    memo(rig, "00", "t4-waiting", "state: moving");
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
    publish_ask(
        rig,
        ASK,
        "#s2-a #t4-bare #t4-waiting Topics with only an open ask",
    );

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
      ["needs","Needs you",["s2-a","smoke","t4-bare","t4-choice","t4-priority","t4-waiting"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z"]],
      ["other","Other",["t4-other","t4-pinned-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    tab.holds(CHECK, ASK_ONLY, "")?;
    tab.holds(CHECK, FOLDED, r#"["false","",""]"#)?;
    tab.click_selector(CHECK, FOLD)?;
    wait(
        tab,
        CHECK,
        FOLDED,
        r#"["true","s2-closed s2-m s2-parked s2-z t4-pinned",""]"#,
    )?;
    tab.click_selector(CHECK, FOLD)?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
    script(
        tab,
        CHECK,
        r#"
const key = 'ae-web-pins';
localStorage.setItem(key, '["t4-choice","t4-priority","t4-moving-pin","t4-pinned","t4-pinned-other","t4-waiting"]');
dispatchEvent(new StorageEvent('storage', {key, storageArea: localStorage}));
"#,
    )?;
    wait(
        tab,
        CHECK,
        GROUPS,
        r#"[
      ["needs","Needs you",["s2-a","smoke","t4-bare","t4-choice","t4-priority","t4-waiting"]],
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
      ["needs","Needs you",["s2-a","smoke","t4-bare","t4-choice","t4-priority","t4-waiting"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z","t4-other"]],
      ["pinned","Pinned",["t4-pinned","t4-pinned-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    ae::console::close(&rig.dir, &rig.session, Some(ASK))
        .unwrap_or_else(|error| panic!("close T4 fixture ask: {error}"));
    wait(
        tab,
        CHECK,
        GROUPS,
        r#"[
      ["needs","Needs you",["smoke","t4-choice","t4-priority"]],
      ["moving","Moving",["calm","t4-moving-pin","t4-order-a","t4-order-z","t4-other","t4-waiting"]],
      ["pinned","Pinned",["t4-pinned","t4-pinned-other"]],
      ["rest","Quiet, parked, closed",[]]
    ]"#,
    )?;
    tab.click_selector(CHECK, FOLD)?;
    wait(
        tab,
        CHECK,
        FOLDED,
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z",""]"#,
    )?;
    tab.click_selector(CHECK, FOLD)?;
    wait(tab, CHECK, FOLDED, r#"["false","",""]"#)?;
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
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z",""]"#,
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
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z",""]"#,
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
    tab.click_selector(CHECK, "button[data-role=side-row][data-slug=t4-order-a]")?;
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

fn wait_ends(tab: &mut Tab, rig: &Rig) -> Checked {
    const CONTROL: &str = "t4c-close-control";
    const SETUP: &str = "t4c-waiting";
    const KEEP: &str = "t4c-keep-selection";
    const WAIT: &str = "ae-20261124T100000Z-00000002";
    const NEEDS_ROW: &str = "[data-group=needs] [data-slug=t4-order-a]";
    const EXISTS: &str = "const saw = !!document.querySelector(want); return {ok: saw, saw};";
    const ABSENT: &str = "const saw = !!document.querySelector(want); return {ok: !saw, saw};";

    tab.view(WIDE);
    script(tab, CONTROL, "window.__aeStale = 1")?;
    tab.call("Page.reload", Value::Obj(Vec::new()));
    wait(tab, CONTROL, READY, "")?;
    wait(tab, CONTROL, ALL_CARD, "")?;
    tab.click_selector(CONTROL, FOLD)?;
    wait(
        tab,
        CONTROL,
        FOLDED,
        r#"["true","s2-a s2-closed s2-m s2-parked s2-z",""]"#,
    )?;
    tab.click_selector(CONTROL, "[data-group=rest] [data-slug=s2-parked]")?;
    wait(tab, CONTROL, CARD, "s2-parked")?;
    tab.click_selector(CONTROL, FOLD)?;
    wait(tab, CONTROL, FOLDED, r#"["false","s2-parked","s2-parked"]"#)?;
    memo(
        rig,
        "11",
        "s2-parked",
        "state: closed | close: T4c topic closure",
    );
    wait(tab, CONTROL, ALL_CARD, "")?;
    ok(CONTROL, WIDE);

    memo(rig, "11", "t4-order-a", "state: quiet");
    publish_ask(rig, WAIT, "#t4-order-a T4c open ask");
    wait(tab, SETUP, EXISTS, NEEDS_ROW)?;
    tab.click_selector(SETUP, NEEDS_ROW)?;
    wait(tab, SETUP, WAITING_CARD, WAIT)?;
    tab.holds(SETUP, FILTERED, "t4-order-a")?;
    tab.holds(SETUP, FOLDED, r#"["false","",""]"#)?;
    ok(SETUP, WIDE);

    ae::console::close(&rig.dir, &rig.session, Some(WAIT))
        .unwrap_or_else(|error| panic!("close T4c fixture ask: {error}"));
    // First observe the new snapshot, then read the selection it rendered.
    wait(tab, "t4c-wait-ended", ABSENT, NEEDS_ROW)?;
    tab.holds(KEEP, FOLDED, r#"["false","t4-order-a","t4-order-a"]"#)?;
    tab.holds(KEEP, CARD, "t4-order-a")?;
    tab.holds(KEEP, FILTERED, "t4-order-a")?;
    ok(KEEP, WIDE);
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
        narrow(tab)?;
        wait_ends(tab, &rig)
    })();
    listener.shutdown();
    result
}

/// The side panel's structure, read from `side` alone so that it also judges a
/// detached copy: a navigation landmark named Topics with no tab roles, All as
/// the first row, every row a button in a list item of a list, each group's one
/// list named by a heading of that group outside the list, exactly one element
/// with `aria-current`, the selected row with the value `true`, and the fold
/// button outside every list, its `aria-expanded` as wanted and its
/// `aria-controls` naming the last group's list, which holds `rest` items.
const SHAPE: &str = r"
const ROWS = 'button:is([data-role=side-all],[data-role=side-row])';
const shape = (side, [current, expanded, rest]) => {
  const byId = id => id ? side.querySelector('#' + CSS.escape(id)) : null;
  const rows = [...side.querySelectorAll(ROWS)];
  const groups = [...side.querySelectorAll('[data-group]')];
  const marked = [...side.querySelectorAll('[aria-current]')];
  const fold = side.querySelector('[data-action=toggle-group]');
  const target = byId(fold?.getAttribute('aria-controls'));
  const listed = row => row.parentElement?.tagName === 'LI' &&
    row.parentElement.parentElement?.tagName === 'UL';
  const named = groups.map(group => {
    const lists = group.querySelectorAll('ul');
    const list = lists[0];
    const head = byId(list?.getAttribute('aria-labelledby'));
    return lists.length === 1 && !!head && /^H[1-6]$/.test(head.tagName) && group.contains(head) &&
      !list.contains(head) && head.textContent.trim() !== '' &&
      [...list.children].every(item => item.tagName === 'LI');
  });
  const saw = {
    nav: side.tagName === 'NAV' && !side.hasAttribute('role') && side.getAttribute('aria-label') === 'Topics',
    tabRoles: side.querySelectorAll('[role=tab],[role=tablist],[role=tabpanel],[aria-selected],[aria-orientation]').length,
    allFirst: rows[0]?.dataset.role === 'side-all' && rows[0].textContent.trim() === 'All' &&
      rows.slice(1).every(row => row.dataset.role === 'side-row'),
    listed: rows.length > 1 && rows.every(listed),
    named: named.length > 0 && named.every(Boolean),
    current: marked.map(el => [el.matches(ROWS) ? el.dataset.slug || '' : el.tagName, el.getAttribute('aria-current')]),
    fold: !!fold && fold.tagName === 'BUTTON' && !fold.closest('ul') && fold.getAttribute('aria-expanded') === expanded,
    target: target?.tagName === 'UL' && !!fold && target.closest('[data-group]') === fold.closest('[data-group]') &&
      target.closest('[data-group]')?.dataset.group === 'rest' && target.children.length === rest
  };
  return {ok: saw.nav && saw.tabRoles === 0 && saw.allFirst && saw.listed && saw.named &&
    saw.current.length === 1 && saw.current[0][0] === current && saw.current[0][1] === 'true' &&
    saw.fold && saw.target, saw};
};
";

/// The live panel, found by its groups rather than by its name, has the shape
/// `want` names, and each group's heading is shown.
const LIVE_SHAPE: &str = r"
const side = document.querySelector('[data-group]')?.closest('nav');
if (!side) return {ok: false, saw: 'no nav holds the groups'};
const seen = shape(side, JSON.parse(want));
const heads = [...side.querySelectorAll('[data-group] ul[aria-labelledby]')]
  .map(list => side.querySelector('#' + CSS.escape(list.getAttribute('aria-labelledby'))))
  .map(head => (head?.getBoundingClientRect().height ?? 0) > 0);
return {ok: seen.ok && heads.length > 0 && heads.every(Boolean), saw: {...seen.saw, heads}};
";

/// The shape rejects each broken copy of the live panel, and accepts the
/// unbroken copy: the false controls of [`SHAPE`].
const BROKEN_SHAPES: &str = r"
const side = document.querySelector('nav[aria-label=Topics]');
const expected = JSON.parse(want);
const tabs = saw => saw.tabRoles !== 0;
const marks = saw => saw.current.length !== 1;
const breaks = {
  navRole: [saw => !saw.nav, copy => copy.setAttribute('role', 'tablist')],
  rowRole: [tabs, copy => copy.querySelector('button[data-role=side-row]').setAttribute('role', 'tab')],
  selected: [tabs, copy => copy.querySelector('button[data-role=side-row]').setAttribute('aria-selected', 'true')],
  twoCurrent: [marks, copy => copy.querySelector('button[data-role=side-row]:not([aria-current])').setAttribute('aria-current', 'true')],
  falseCurrent: [marks, copy => copy.querySelector('button[data-role=side-row]:not([aria-current])').setAttribute('aria-current', 'false')],
  unlabelled: [saw => !saw.named, copy => copy.querySelector('[data-group] ul').removeAttribute('aria-labelledby')],
  notHeading: [saw => !saw.named, copy => {
    const head = copy.querySelector('#' + CSS.escape(copy.querySelector('[data-group] ul').getAttribute('aria-labelledby')));
    const div = document.createElement('div');
    div.id = head.id;
    div.append(...head.childNodes);
    head.replaceWith(div);
  }],
  lostControls: [saw => !saw.target, copy => copy.querySelector('[data-action=toggle-group]').setAttribute('aria-controls', 'side-missing-list')],
  foldInList: [saw => !saw.fold, copy => {
    const fold = copy.querySelector('[data-action=toggle-group]');
    const list = document.createElement('ul');
    const item = document.createElement('li');
    fold.replaceWith(list);
    list.append(item);
    item.append(fold);
  }],
  allLast: [saw => !saw.allFirst, copy => copy.append(copy.querySelector('button[data-role=side-all]').closest('ul'))],
  bareRow: [saw => !saw.listed, copy => {
    const row = copy.querySelector('button[data-role=side-row]');
    row.closest('ul').before(row);
  }]
};
const whole = shape(side.cloneNode(true), expected);
const saw = Object.fromEntries(Object.entries(breaks).map(([name, [clause, broken]]) => {
  const copy = side.cloneNode(true);
  broken(copy);
  const seen = shape(copy, expected);
  return [name, {rejected: !seen.ok, clause: clause(seen.saw)}];
}));
return {ok: !!side && whole.ok && Object.values(saw).every(seen => seen.rejected && seen.clause), saw: {whole: whole.ok, ...saw}};
";

/// Where the focus is, which rows carry `aria-current`, which panel shows and
/// whether the last group is open, against `[focus, current, card, expanded]`.
/// A row reads `row:<slug>:<group>`, All reads `all`, the fold button `fold`.
const KEYS_STATE: &str = r"
const side = document.querySelector('nav[aria-label=Topics]');
const name = el => !el ? 'none' : el.matches('[data-action=toggle-group]') ? 'fold' :
  el.matches('button[data-role=side-all]') ? 'all' :
  el.matches('button[data-role=side-row]') ? 'row:' + el.dataset.slug + ':' + el.closest('[data-group]')?.dataset.group :
  el.tagName;
const panel = document.querySelector('[data-panel=topic], [data-panel=all]');
const saw = {focus: name(document.activeElement),
  current: [...(side?.querySelectorAll('[aria-current]') || [])].map(el => el.dataset.slug || 'all').join(' '),
  card: panel?.dataset.panel === 'all' ? 'all' : panel?.dataset.slug || '',
  expanded: side?.querySelector('[data-action=toggle-group]')?.getAttribute('aria-expanded')};
const [focus, current, card, expanded] = JSON.parse(want);
return {ok: saw.focus === focus && saw.current === current && saw.card === card && saw.expanded === expanded, saw};
";

/// The last row before a group boundary and the first row after it, in the
/// panel's order: `want` names the group that ends, or `rest` for the rows
/// before and after the fold button. Every row is a focusable button.
const BOUNDARY: &str = r"
const side = document.querySelector('nav[aria-label=Topics]');
const rows = [...(side?.querySelectorAll('button:is([data-role=side-all],[data-role=side-row])') || [])];
const group = row => row.closest('[data-group]')?.dataset.group || 'all';
const at = want === 'rest' ? rows.findIndex(row => group(row) === 'rest') - 1 :
  rows.findLastIndex(row => group(row) === want);
const [before, after] = [rows[at], rows[at + 1]];
const saw = {before: before && 'row:' + before.dataset.slug + ':' + group(before),
  after: after && 'row:' + after.dataset.slug + ':' + group(after),
  tabbable: rows.every(row => row.tabIndex >= 0 && !row.disabled)};
return {ok: at >= 0 && !!before && !!after && group(before) !== group(after) && saw.tabbable,
  before: before?.dataset.slug, after: after?.dataset.slug, beforeRow: saw.before, afterRow: saw.after, saw};
";

const FOCUS_ROW: &str = r"
const row = document.querySelector('nav[aria-label=Topics] button[data-role=side-row][data-slug=' + CSS.escape(want) + ']');
row?.focus();
return {ok: !!row && document.activeElement === row, saw: document.activeElement?.outerHTML.slice(0, 80)};
";

const FOCUS_ALL: &str = r"
const all = document.querySelector('nav[aria-label=Topics] button[data-role=side-all]');
all?.focus();
return {ok: !!all && all.tabIndex >= 0 && document.activeElement === all, saw: document.activeElement?.outerHTML.slice(0, 80)};
";

const FOCUS_FOLD: &str = r"
const fold = document.querySelector('nav[aria-label=Topics] [data-action=toggle-group]');
fold?.focus();
return {ok: !!fold && document.activeElement === fold, saw: document.activeElement?.outerHTML.slice(0, 80)};
";

const ROW_GONE: &str = r"
const saw = !!document.querySelector('nav[aria-label=Topics] [data-slug=' + CSS.escape(want) + ']');
return {ok: !saw, saw};
";

const DRAFT_KEPT: &str = r"
const saw = {active: tag(document.activeElement), value: box()?.value};
return {ok: document.activeElement === box() && saw.value === want, saw};
";

/// Groups as the fresh T4 fixture shows them, with each list's name in the
/// accessibility tree containing the group's label in any case: the headings'
/// CSS uppercases the label, and Chrome names the list with the shown text.
const LABELS: [&str; 4] = ["Needs you", "Moving", "Other", "Quiet, parked, closed"];

/// One key, down and up, as a keyboard sends it: `text` makes the key press
/// that activates a button.
fn key(tab: &mut Tab, key: &str, code: &str, vk: i64, text: &str) {
    for kind in ["keyDown", "keyUp"] {
        let mut fields = vec![
            ("type", Value::str(kind)),
            ("key", Value::str(key)),
            ("code", Value::str(code)),
            ("windowsVirtualKeyCode", Value::Num(vk)),
        ];
        if kind == "keyDown" && !text.is_empty() {
            fields.push(("text", Value::str(text)));
        }
        tab.call("Input.dispatchKeyEvent", Value::obj(fields));
    }
}

fn down(tab: &mut Tab) {
    key(tab, "ArrowDown", "ArrowDown", 40, "");
}

fn up(tab: &mut Tab) {
    key(tab, "ArrowUp", "ArrowUp", 38, "");
}

fn state(focus: &str, current: &str, card: &str, expanded: &str) -> String {
    Value::Arr(
        [focus, current, card, expanded]
            .into_iter()
            .map(Value::str)
            .collect(),
    )
    .render()
}

fn shape_holds(
    tab: &mut Tab,
    check: &'static str,
    current: &str,
    expanded: &str,
    rest: i64,
) -> Checked {
    let want = Value::Arr(vec![
        Value::str(current),
        Value::str(expanded),
        Value::Num(rest),
    ])
    .render();
    wait(tab, check, &format!("{SHAPE}{LIVE_SHAPE}"), &want)
}

/// The accessibility tree Chrome builds for the page: one navigation landmark
/// named Topics, no tab, tab list or tab panel, a button named All, and one
/// list per group whose name holds the group's label, compared without case.
fn tree(tab: &mut Tab, check: &'static str) -> Checked {
    tab.try_call("Accessibility.enable", Value::Obj(Vec::new()))
        .map_err(|why| Failed::new(check, why))?;
    let tree = tab
        .try_call("Accessibility.getFullAXTree", Value::Obj(Vec::new()))
        .map_err(|why| Failed::new(check, why))?;
    let Some(Value::Arr(nodes)) = tree.get("nodes") else {
        return Err(Failed::new(check, format!("no tree: {}", tree.render())));
    };
    let seen: Vec<(String, String)> = nodes
        .iter()
        .filter(|node| node.get("ignored") != Some(&Value::Bool(true)))
        .map(|node| {
            let field = |name: &str| {
                node.get(name)
                    .and_then(|field| field.get_str("value"))
                    .unwrap_or_default()
                    .to_owned()
            };
            (field("role"), field("name"))
        })
        .collect();
    let count = |role: &str, name: &dyn Fn(&str) -> bool| {
        seen.iter()
            .filter(|(seen_role, seen_name)| seen_role == role && name(seen_name.as_str()))
            .count()
    };
    let landmarks = count("navigation", &|name| name == "Topics");
    let tabs = ["tab", "tablist", "tabpanel"]
        .into_iter()
        .map(|role| count(role, &|_| true))
        .sum::<usize>();
    let all = count("button", &|name| name == "All");
    let lists: Vec<usize> = LABELS
        .into_iter()
        .map(|label| {
            let label = label.to_lowercase();
            count("list", &|name| name.to_lowercase().contains(&label))
        })
        .collect();
    if landmarks == 1 && tabs == 0 && all == 1 && lists.iter().all(|found| *found == 1) {
        return Ok(());
    }
    let roles: Vec<String> = seen
        .iter()
        .filter(|(role, _)| {
            ["navigation", "list", "tab", "tablist", "tabpanel"].contains(&role.as_str())
        })
        .map(|(role, name)| format!("{role}:{name}"))
        .collect();
    Err(Failed::new(
        check,
        format!(
            "landmarks {landmarks}, tab roles {tabs}, All buttons {all}, lists {lists:?}: {roles:?}"
        ),
    ))
}

/// The rows on both sides of a boundary (see [`BOUNDARY`]): each slug and its
/// [`KEYS_STATE`] name.
struct Boundary {
    before: String,
    before_row: String,
    after_row: String,
}

fn boundary(tab: &mut Tab, check: &'static str, group: &str) -> Result<Boundary, Failed> {
    let seen = tab.verdict(check, BOUNDARY, group)?;
    match (
        seen.get_str("before"),
        seen.get_str("beforeRow"),
        seen.get_str("afterRow"),
    ) {
        (Some(before), Some(before_row), Some(after_row)) => Ok(Boundary {
            before: before.to_owned(),
            before_row: before_row.to_owned(),
            after_row: after_row.to_owned(),
        }),
        _ => Err(Failed::new(check, seen.render())),
    }
}

/// The slug in a [`KEYS_STATE`] row name `row:<slug>:<group>`.
fn slug(row: &str) -> String {
    row.split(':').nth(1).unwrap_or_default().to_owned()
}

fn structure(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t5-a11y";
    shape_holds(tab, CHECK, "", "false", 0)?;
    let want = Value::Arr(vec![Value::str(""), Value::str("false"), Value::Num(0)]).render();
    tab.holds(CHECK, &format!("{SHAPE}{BROKEN_SHAPES}"), &want)?;
    tab.click_selector(CHECK, FOLD)?;
    shape_holds(tab, CHECK, "", "true", 5)?;
    tree(tab, CHECK)?;
    tab.click_selector(CHECK, FOLD)?;
    shape_holds(tab, CHECK, "", "false", 0)?;
    tab.click_selector(CHECK, "button[data-role=side-row][data-slug=t4-order-a]")?;
    shape_holds(tab, CHECK, "t4-order-a", "false", 0)?;
    tab.click_selector(CHECK, FOLD)?;
    tab.click_selector(
        CHECK,
        "[data-group=rest] button[data-role=side-row][data-slug=s2-parked]",
    )?;
    shape_holds(tab, CHECK, "s2-parked", "true", 5)?;
    tab.click_selector(CHECK, FOLD)?;
    shape_holds(tab, CHECK, "s2-parked", "false", 1)?;
    tab.click_selector(CHECK, ALL)?;
    shape_holds(tab, CHECK, "", "false", 0)?;
    ok(CHECK, WIDE);
    Ok(())
}

fn keys(tab: &mut Tab) -> Checked {
    const CHECK: &str = "t5-keys";
    // On a freshly loaded page nothing has the focus, and the first Tab
    // reaches All: the header before the panel holds no control.
    script(tab, CHECK, "window.__aeStale = 1")?;
    tab.call("Page.reload", Value::Obj(Vec::new()));
    wait(tab, CHECK, READY, "")?;
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state("BODY", "all", "all", "false"),
    )?;
    key(tab, "Tab", "Tab", 9, "");
    wait(tab, CHECK, KEYS_STATE, &state("all", "all", "all", "false"))?;
    let needs = boundary(tab, CHECK, "needs")?;
    let (last, first) = (&needs.before_row, &needs.after_row);
    tab.holds(CHECK, FOCUS_ALL, "")?;
    up(tab);
    wait(tab, CHECK, KEYS_STATE, &state("all", "all", "all", "false"))?;
    tab.holds(CHECK, FOCUS_ROW, &needs.before)?;
    down(tab);
    wait(tab, CHECK, KEYS_STATE, &state(first, "all", "all", "false"))?;
    up(tab);
    wait(tab, CHECK, KEYS_STATE, &state(last, "all", "all", "false"))?;
    down(tab);
    wait(tab, CHECK, KEYS_STATE, &state(first, "all", "all", "false"))?;
    key(tab, "Enter", "Enter", 13, "\r");
    let chosen = slug(first);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(first, &chosen, &chosen, "false"),
    )?;
    up(tab);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(last, &chosen, &chosen, "false"),
    )?;
    key(tab, " ", "Space", 32, " ");
    let chosen = slug(last);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(last, &chosen, &chosen, "false"),
    )?;
    tab.click_selector(CHECK, ALL)?;
    wait(tab, CHECK, KEYS_STATE, &state(last, "all", "all", "false"))?;

    // The fold button sits between the last group's rows and the ones above:
    // Down skips it, and it ignores Up and Down itself.
    tab.click_selector(CHECK, FOLD)?;
    let fold = boundary(tab, CHECK, "rest")?;
    tab.holds(CHECK, FOCUS_ROW, &fold.before)?;
    down(tab);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(&fold.after_row, "all", "all", "true"),
    )?;
    up(tab);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(&fold.before_row, "all", "all", "true"),
    )?;
    tab.holds(CHECK, FOCUS_FOLD, "")?;
    down(tab);
    up(tab);
    wait(tab, CHECK, KEYS_STATE, &state("fold", "all", "all", "true"))?;
    key(tab, "Enter", "Enter", 13, "\r");
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state("fold", "all", "all", "false"),
    )?;

    // With the last group folded, the row above the fold button is the last row.
    tab.holds(CHECK, FOCUS_ROW, &fold.before)?;
    down(tab);
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state(&fold.before_row, "all", "all", "false"),
    )?;
    ok(CHECK, WIDE);
    Ok(())
}

fn focus_kept(tab: &mut Tab, rig: &Rig) -> Checked {
    const CHECK: &str = "t5-focus";
    const DRAFT: &str = "T5 draft kept";
    // A focused row that moves to another group keeps the focus.
    tab.holds(CHECK, FOCUS_ROW, "t4-other")?;
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state("row:t4-other:other", "all", "all", "false"),
    )?;
    memo(rig, "20", "t4-other", "state: moving");
    wait(
        tab,
        CHECK,
        KEYS_STATE,
        &state("row:t4-other:moving", "all", "all", "false"),
    )?;
    // A focused row that leaves the panel gives the focus to the current row.
    tab.holds(CHECK, FOCUS_ROW, "t4-moving-pin")?;
    memo(rig, "21", "t4-moving-pin", "state: quiet");
    wait(tab, CHECK, ROW_GONE, "t4-moving-pin")?;
    wait(tab, CHECK, KEYS_STATE, &state("all", "all", "all", "false"))?;
    // The false control: a row had the focus, then the draft took it; a row
    // that leaves the panel then leaves the focus and the draft where they are.
    tab.holds(CHECK, FOCUS_ROW, "t4-order-z")?;
    tab.click(CHECK, "box")?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str(DRAFT))]),
    );
    tab.holds(CHECK, DRAFT_KEPT, DRAFT)?;
    memo(rig, "22", "t4-order-z", "state: quiet");
    wait(tab, CHECK, ROW_GONE, "t4-order-z")?;
    tab.holds(CHECK, DRAFT_KEPT, DRAFT)?;
    ok(CHECK, WIDE);
    Ok(())
}

/// T5: the side panel as assistive technology and the keyboard reach it, on a
/// fresh T4 fixture of its own.
pub(super) fn check_t5(tab: &mut Tab) -> Checked {
    let rig = smoke_rig();
    seed(&rig);
    let listener = start_listener(&rig.scratch);
    tab.view(WIDE);
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(listener.link()))]),
    );
    let result = (|| {
        if let Some(error) = loaded.get_str("errorText") {
            return Err(Failed::new("t5-start", error));
        }
        wait(tab, "t5-start", READY, "")?;
        // The T4 stage pinned rows on a listener of its own; should this one
        // reuse its port, and so its origin, those pins start cleared.
        script(
            tab,
            "t5-start",
            "localStorage.removeItem('ae-web-pins'); \
             dispatchEvent(new StorageEvent('storage', {key: 'ae-web-pins', storageArea: localStorage}));",
        )?;
        wait(
            tab,
            "t5-start",
            "const saw = document.querySelectorAll('[data-pinned]').length; return {ok: saw === 0, saw};",
            "",
        )?;
        // The overview and the four groups, read without a T5 locator, so that
        // the product before T5 meets the first T5 check rather than this wait.
        wait(
            tab,
            "t5-start",
            "const saw = {overview: !!document.querySelector('[data-panel=all]'), \
             groups: document.querySelectorAll('[data-group]').length}; \
             return {ok: saw.overview && saw.groups === 4, saw};",
            "",
        )?;
        structure(tab)?;
        keys(tab)?;
        focus_kept(tab, &rig)
    })();
    listener.shutdown();
    result
}
