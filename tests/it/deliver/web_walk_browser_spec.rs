//! S2 acceptance cells for the real headless browser.

use super::{Cell, Checked, Failed, PAGE_WAIT, Rig, SEND_WAIT, Tab, ok};
use ae::json::Value;

pub(super) const EXTRA_MEMOS: &str = concat!(
    "2026-11-22T08:00:00Z\ttui\ttopic-s2-z\ttitle: <b>Oldest quiet</b> | state: quiet | ",
    "what: An open topic | last: Previously checked | next: Read the log\n",
    "2026-11-22T09:00:00Z\ttui\ttopic-s2-a\ttitle: Middle quiet | state: quiet\n",
    "2026-11-22T10:00:00Z\ttui\ttopic-s2-m\ttitle: Newest quiet | state: quiet\n",
    "2026-11-22T08:00:00Z\ttui\ttopic-s2-parked\tstate: parked\n",
    "2026-11-22T08:00:00Z\ttui\ttopic-s2-closed\tstate: closed | close: Already shipped\n",
);

const OPEN: &str = ".revisit button.rv-walk";
const KEEP: &str = ".walk button[data-walk=keep]";
const STOP: &str = ".walk button.w-stop";
const BACK: &str = ".walk-back";
const REASON: &str = ".walk input.reason";
const CLOSE: &str = ".walk button.w-closeok";
const CHECK: &str = ".walk button.w-check";
const RETRY: &str = ".walk button.w-retry";
const CLOSED: &str =
    "const found = !!document.querySelector('.walk'); return {ok: !found, saw: found};";
const UNKNOWN: &str = "unknown: may have landed, not resent";
const CAP: &str = "cap-full: 5/5 console asks open; open ids: ae-20261122T200000Z-00000001, \
 ae-20261122T200000Z-00000002, ae-20261122T200000Z-00000003, ae-20261122T200000Z-00000004, \
 ae-20261122T200000Z-00000005; owner /close can withdraw an ask";

fn shown(tab: &mut Tab, selector: &str, slug: &str) -> Checked {
    let script = format!(
        "const e = document.querySelector({}); const visible = !!e && e.getClientRects().length > 0; \
         const selected = e?.dataset.slug; return {{ok: visible && selected === want, \
         saw: {{visible, selected}}}};",
        Value::str(selector).render()
    );
    tab.until("revisit", &script, slug, PAGE_WAIT).map(drop)
}

pub(super) fn check_revisit(tab: &mut Tab, cell: Cell) -> Checked {
    let selector = if cell.wide() { ".revisit" } else { ".pq" };
    shown(tab, selector, "s2-z")?;
    if cell.wide() {
        tab.holds(
            "revisit",
            "const e = document.querySelector('.revisit'); const saw = e?.textContent; \
             return {ok: e?.querySelector('.nm')?.textContent === want && \
             saw.includes('Read the log') && !e.querySelector('b'), saw};",
            "<b>Oldest quiet</b>",
        )?;
    }
    let next = format!("{selector} button.rv-next");
    for slug in ["s2-a", "s2-m", "s2-z", "s2-a"] {
        tab.click_selector("revisit", &next)?;
        shown(tab, selector, slug)?;
    }
    if !cell.wide() {
        tab.click_selector("revisit", "[data-role=view-panel]")?;
        shown(tab, ".revisit", "s2-a")?;
        check_walk_fits(tab)?;
        tab.click_selector("revisit", "[data-role=view-chat]")?;
        shown(tab, ".pq", "s2-a")?;

        let draft = "Revisit preserves this draft";
        tab.click("revisit", "box")?;
        tab.call(
            "Input.insertText",
            Value::obj([("text", Value::str(draft))]),
        );
        tab.click_selector("revisit", "[data-role=tab][data-slug=smoke]")?;
        tab.holds(
            "revisit",
            "const saw = document.querySelector('[data-role=tab][data-slug=smoke]')?.ariaSelected; \
             return {ok: saw === 'true', saw};",
            "",
        )?;
        tab.click_selector("revisit", "[data-role=tab-all]")?;
        shown(tab, ".pq", "s2-a")?;
        tab.holds(
            "revisit",
            "const saw = document.querySelector('.composer textarea')?.value; \
             return {ok: saw === want, saw};",
            draft,
        )?;
        tab.eval(
            "const draft = document.querySelector('.composer textarea'); draft.value = ''; \
             draft.dispatchEvent(new InputEvent('input', {bubbles: true}));",
        )
        .map_err(|why| Failed::new("revisit", why))?;
    }
    ok("revisit", cell);
    Ok(())
}

fn check_walk_fits(tab: &mut Tab) -> Checked {
    tab.click_selector("walk-fits", OPEN)?;
    tab.until(
        "walk-fits",
        "const d = document.querySelector('.walk[role=dialog][aria-modal=true]'); \
         const r = d?.getBoundingClientRect(); const focus = document.activeElement; \
         const saw = r && [r.left, r.top, r.right, r.bottom]; \
         return {ok: !!r && r.left >= 0 && r.top >= 0 && r.right <= innerWidth && \
         r.bottom <= innerHeight && focus?.matches('.walk button[data-walk=keep]'), saw};",
        "",
        PAGE_WAIT,
    )?;
    tab.click_selector("walk-fits", STOP)?;
    tab.until("walk-fits", CLOSED, "", PAGE_WAIT).map(drop)
}

fn step(tab: &mut Tab, check: &'static str, title: &str) -> Checked {
    tab.until(
        check,
        "const saw = document.querySelector('.walk .nm')?.textContent?.trim(); \
         return {ok: saw === want, saw};",
        title,
        PAGE_WAIT,
    )
    .map(drop)
}

fn insert(tab: &mut Tab, text: &str) {
    tab.call("Input.insertText", Value::obj([("text", Value::str(text))]));
}

fn key(tab: &mut Tab, name: &str) {
    for event in ["keyDown", "keyUp"] {
        tab.call(
            "Input.dispatchKeyEvent",
            Value::obj([
                ("type", Value::str(event)),
                ("key", Value::str(name)),
                ("code", Value::str(name)),
            ]),
        );
    }
}

fn wait_for_send(
    tab: &mut Tab,
    rig: &Rig,
    check: &'static str,
    line: &str,
    count: usize,
) -> Checked {
    tab.until(
        check,
        "const note = document.querySelector('.walk .w-note')?.textContent; \
         const next = document.querySelector('.walk .w-step')?.textContent; \
         return {ok: !!next && next.includes(want) || !!note && note.includes(want), \
         saw: {next, note}};",
        match count {
            2 => "2 of 3",
            3 => "3 of 3",
            _ => "Walk done",
        },
        SEND_WAIT,
    )?;
    let received = rig.submitted();
    let expected = format!(" from human:chat: {line}\n");
    if rig.enter_count() != count || received.matches(&expected).count() != 1 {
        return Err(Failed::new(
            check,
            format!("wanted one new submission with body {line:?}; got {received:?}"),
        ));
    }
    Ok(())
}

pub(super) fn check_walk(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    // S1 has already sent one composer draft in this fixture.
    tab.click_selector("walk-keep", OPEN)?;
    step(tab, "walk-keep", "<b>Oldest quiet</b>")?;
    tab.holds(
        "walk-keep",
        "const saw = document.querySelector('.walk')?.textContent || ''; \
         return {ok: ['An open topic', 'Previously checked', 'Read the log'].every(s => \
         saw.includes(s)), saw};",
        "",
    )?;
    tab.click_selector("walk-keep", KEEP)?;
    wait_for_send(tab, rig, "walk-keep", "#s2-z keep", 2)?;
    ok("walk-keep", cell);

    step(tab, "walk-ask", "Middle quiet")?;
    tab.click_selector("walk-ask", ".walk button[data-walk=ask]")?;
    wait_for_send(tab, rig, "walk-ask", "#s2-a ask", 3)?;
    ok("walk-ask", cell);

    step(tab, "walk-close", "Newest quiet")?;
    tab.click_selector("walk-close", ".walk button[data-walk=close]")?;
    tab.click_selector("walk-close", REASON)?;
    insert(tab, "  No longer needed  ");
    tab.click_selector("walk-close", CLOSE)?;
    tab.until(
        "walk-close",
        "const saw = document.querySelectorAll('.walk .w-done li').length; \
         return {ok: saw === Number(want), saw};",
        "3",
        SEND_WAIT,
    )?;
    let received = rig.submitted();
    let expected = " from human:chat: #s2-m close No longer needed\n";
    if rig.enter_count() != 4 || received.matches(expected).count() != 1 {
        return Err(Failed::new("walk-close", received));
    }
    ok("walk-close", cell);
    tab.click_selector("walk-close", BACK)?;
    tab.until("walk-close", CLOSED, "", PAGE_WAIT).map(drop)
}

// Hold one response to create a real in-flight page state. The stub represents
// only the external send endpoint. Reads, nonces, rotation, focus and UI events
// run through the page and fixture listener. Accelerate the one-second retries
// without changing their number or how the page interprets a response.
const SEND_EDGE: &str = r#"(() => {
  const fetchReal = window.fetch.bind(window), later = window.setTimeout.bind(window);
  const edge = window.__s2 = { posts: [], mode: "hold", release: null };
  window.setTimeout = (fn, ms, ...args) => later(fn, ms === 1000 ? 0 : ms, ...args);
  const response = (status, data) => new Response(JSON.stringify(data), {
    status, headers: {"Content-Type": "application/json"}
  });
  window.fetch = (url, init) => {
    if (url !== "/api/send") return fetchReal(url, init);
    edge.posts.push(JSON.parse(init.body));
    if (edge.mode === "hold") return new Promise(resolve => {
      edge.release = () => resolve(response(202, {state: "queued"}));
    });
    if (edge.mode === "queue") return Promise.resolve(response(202, {state: "queued"}));
    return Promise.resolve(response(200, {
      outcome: edge.mode, id: "ae-20261122T200000Z-00000001",
      words: edge.words, seat: "tui", to: "websmoke"
    }));
  };
})()"#;

fn eval(tab: &mut Tab, check: &'static str, script: &str) -> Checked {
    tab.eval(script)
        .map(drop)
        .map_err(|error| Failed::new(check, error))
}

fn carry_state(tab: &mut Tab, check: &'static str, state: &str) -> Checked {
    tab.until(
        check,
        "const saw = document.querySelector('.walk')?.dataset.carry; return {ok: saw === want, saw};",
        state,
        PAGE_WAIT,
    )
    .map(drop)
}

fn inert(tab: &mut Tab, state: &str) -> Checked {
    const CASE: &str = "walk-dismissal";
    key(tab, "Escape");
    carry_state(tab, CASE, state)?;
    tab.click_selector(CASE, BACK)?;
    carry_state(tab, CASE, state)?;
    tab.holds(
        CASE,
        "const button = document.querySelector('.walk button.w-stop'); \
         return {ok: !button || button.disabled, saw: button?.outerHTML};",
        "",
    )
}

fn append(rig: &Rig, text: &str) -> Checked {
    ae::store::open(&rig.dir)
        .append_memo(text.as_bytes())
        .map_err(|error| Failed::new("walk-edges", format!("{error:?}")))
}

fn check_pending(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    const CASE: &str = "walk-dismissal";
    shown(tab, ".revisit", "s2-edge-a")?;
    tab.click_selector(CASE, ".revisit button.rv-next")?;
    shown(tab, ".revisit", "s2-edge-b")?;
    tab.click_selector(CASE, OPEN)?;
    step(tab, CASE, "Edge first")?;
    tab.click_selector(CASE, KEEP)?;
    tab.until(
        CASE,
        "const saw = typeof window.__s2.release; return {ok: saw === 'function', saw};",
        "",
        PAGE_WAIT,
    )?;
    inert(tab, "pending")?;
    eval(
        tab,
        CASE,
        "window.__s2.mode = 'queue'; window.__s2.release()",
    )?;
    carry_state(tab, CASE, "noanswer")?;
    inert(tab, "noanswer")?;

    let words = Value::str(UNKNOWN).render();
    eval(
        tab,
        CASE,
        &format!("window.__s2.mode = 'unknown'; window.__s2.words = {words}"),
    )?;
    tab.click_selector(CASE, CHECK)?;
    step(tab, CASE, "Edge second")?;
    tab.holds(
        CASE,
        "const posts = window.__s2.posts; const before = posts[0]; \
         const ok = posts.length === 121 && before?.text === '#s2-edge-a keep' && \
         typeof before.nonce === 'string' && posts.every(p => p.text === before.text && \
         p.nonce === before.nonce && p.route === before.route); \
         return {ok, saw: {count: posts.length, same: ok}};",
        "",
    )?;

    append(
        rig,
        "2026-11-22T12:00:00Z\ttui\ttopic-s2-edge-b\tstate: moving\n",
    )?;
    tab.until(
        CASE,
        "const d = document.querySelector('.walk'); const saw = d?.textContent || ''; \
         const actions = Array.from(d?.querySelectorAll('button:not(:disabled)') || []); \
         return {ok: saw.toLowerCase().includes('no longer quiet') && actions.length === 2 && \
         actions.some(e => e.matches('.w-skip')) && actions.some(e => e.matches('.w-stop')), saw};",
        "",
        PAGE_WAIT,
    )?;
    shown(tab, ".revisit", "s2-edge-a")?;
    tab.click_selector(CASE, ".walk button.w-skip")?;
    tab.until(
        CASE,
        "const saw = [...document.querySelectorAll('.walk .w-done li')].map(e => e.textContent); \
         return {ok: saw.length === 2 && saw[0].includes(want) && \
         saw[1].toLowerCase().includes('skipped'), saw};",
        "may have landed",
        PAGE_WAIT,
    )?;
    key(tab, "Escape");
    tab.until(CASE, CLOSED, "", PAGE_WAIT)?;
    ok(CASE, cell);
    Ok(())
}

fn cap_refusal(tab: &mut Tab, cell: Cell) -> Checked {
    const CASE: &str = "walk-cap";
    let words = Value::str(CAP).render();
    eval(
        tab,
        CASE,
        &format!("window.__s2.mode = 'refused'; window.__s2.words = {words}"),
    )?;
    tab.click_selector(CASE, OPEN)?;
    step(tab, CASE, "Edge first")?;
    tab.click_selector(CASE, KEEP)?;
    tab.until(
        CASE,
        "const saw = document.querySelector('.walk .w-note')?.textContent?.trim(); \
         const retry = document.querySelector('.walk .w-retry'); \
         return {ok: saw === want && !!retry && !retry.disabled, saw};",
        CAP,
        PAGE_WAIT,
    )?;
    step(tab, CASE, "Edge first")?;
    tab.click_selector(CASE, RETRY)?;
    tab.until(
        CASE,
        "const posts = window.__s2.posts; const a = posts.at(-2), b = posts.at(-1); \
         return {ok: a?.text === want && b?.text === want && a.route === b.route && \
         a.nonce !== b.nonce, saw: posts.length};",
        "#s2-edge-a keep",
        PAGE_WAIT,
    )?;
    carry_state(tab, CASE, "idle")?;
    tab.click_selector(CASE, STOP)?;
    tab.until(CASE, CLOSED, "", PAGE_WAIT)?;
    ok(CASE, cell);
    Ok(())
}

fn open_card(tab: &mut Tab, slug: &str) -> Checked {
    const CASE: &str = "walk-card";
    tab.click_selector(CASE, "[data-role=tab-all]")?;
    tab.holds(
        CASE,
        "const row = [...document.querySelectorAll('[data-panel=all] button.row')].find(e => \
         e.querySelector('.ln')?.textContent?.split(' ')[0] === '#' + want); \
         if (row) row.dataset.s2TestSlug = want; return {ok: !!row, saw: want};",
        slug,
    )?;
    tab.click_selector(
        CASE,
        &format!("[data-panel=all] button[data-s2-test-slug='{slug}']"),
    )?;
    tab.until(
        CASE,
        "const saw = document.querySelector('.card .sub')?.textContent; \
         return {ok: !!saw && saw.startsWith('#' + want + ' '), saw};",
        slug,
        PAGE_WAIT,
    )
    .map(drop)
}

fn check_blocked_card(tab: &mut Tab, slug: &str) -> Checked {
    open_card(tab, slug)?;
    tab.holds(
        "walk-card",
        "const e = document.querySelector('.card button.cl-line'); \
         const modal = !!document.querySelector('.walk'); \
         return {ok: (!e || e.disabled) && !modal, saw: {button: e?.outerHTML, modal}};",
        "",
    )
}

fn disabled(tab: &mut Tab) -> Checked {
    tab.holds(
        "walk-reason",
        "const button = document.querySelector('.walk button.w-closeok'); \
         const count = window.__s2.posts.length; \
         return {ok: !!button && button.disabled && count === window.__s2.beforeReason, saw: count};",
        "",
    )
}

fn clear_reason(tab: &mut Tab) -> Checked {
    tab.click_selector("walk-reason", REASON)?;
    eval(
        tab,
        "walk-reason",
        "document.querySelector('.walk input.reason').select()",
    )
}

fn check_reason(tab: &mut Tab, cell: Cell) -> Checked {
    const CASE: &str = "walk-reason";
    check_blocked_card(tab, "s2-z")?;
    check_blocked_card(tab, "s2-closed")?;
    open_card(tab, "s2-edge-b")?;
    eval(
        tab,
        CASE,
        "window.__s2.beforeReason = window.__s2.posts.length",
    )?;
    tab.click_selector(CASE, ".card button.cl-line")?;
    step(tab, CASE, "Edge second")?;
    tab.click_selector(CASE, REASON)?;
    disabled(tab)?;
    insert(tab, "   ");
    disabled(tab)?;
    clear_reason(tab)?;
    insert(tab, "one\u{2028}two");
    disabled(tab)?;
    clear_reason(tab)?;
    insert(tab, " ");
    tab.holds(
        CASE,
        r#"const data = new DataTransfer(); data.setData("text/plain", "one\ntwo");
           const input = document.querySelector(".walk input.reason");
           const before = input.value;
           const accepted = input.dispatchEvent(new ClipboardEvent("paste", {
             clipboardData: data, bubbles: true, cancelable: true
           }));
           return {ok: before === " " && !accepted && input.value === before,
             saw: {accepted, before, after: input.value}};"#,
        "",
    )?;
    disabled(tab)?;
    clear_reason(tab)?;
    insert(tab, &"🦊".repeat(201));
    disabled(tab)?;
    clear_reason(tab)?;
    let boundary = "🦊".repeat(200);
    insert(tab, &format!(" {boundary} "));
    eval(tab, CASE, "window.__s2.mode = 'sent'")?;
    tab.click_selector(CASE, CLOSE)?;
    tab.until(
        CASE,
        "const saw = window.__s2.posts.at(-1)?.text; return {ok: saw === want, saw};",
        &format!("#s2-edge-b close {boundary}"),
        PAGE_WAIT,
    )?;
    tab.until(CASE, CLOSED, "", PAGE_WAIT)?;
    ok(CASE, cell);
    Ok(())
}

pub(super) fn check_edges(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    append(
        rig,
        concat!(
            "2026-11-22T06:00:00Z\ttui\ttopic-s2-edge-a\ttitle: Edge first | state: quiet\n",
            "2026-11-22T07:00:00Z\ttui\ttopic-s2-edge-b\ttitle: Edge second | state: quiet\n",
        ),
    )?;
    tab.until(
        "walk-edges",
        "const saw = document.querySelector('.revisit button.rv-walk')?.textContent; \
         return {ok: !!saw && saw.includes(want), saw};",
        "(2)",
        PAGE_WAIT,
    )?;
    eval(tab, "walk-edges", SEND_EDGE)?;
    check_pending(tab, rig, cell)?;
    cap_refusal(tab, cell)?;
    check_reason(tab, cell)
}
