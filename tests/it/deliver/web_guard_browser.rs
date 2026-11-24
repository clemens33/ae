//! Driver-owned browser checks for three guards of the walk that no frozen
//! cell reaches: a step with no receipt stays put when its topic leaves Quiet,
//! the Revisit selection does not jump back to a topic that left and returned,
//! and a walk sends nothing new once the orchestrator route changed while an
//! attempt already carried keeps checking its own receipt.
//!
//! The cell runs after `check_edges` in the same page, so it reuses the
//! `window.__s2` stub of the walk spec: it replaces only the replies of
//! `/api/send`. A second wrapper below replaces only the `route.id` of the
//! `/api/state` replies. Reads, nonces, events and memos stay live.

use super::{Cell, Checked, Failed, PAGE_WAIT, Rig, Tab, ok};
use ae::json::Value;

const OPEN: &str = ".revisit button.rv-walk";
const NEXT: &str = ".revisit button.rv-next";
const KEEP: &str = ".walk button[data-walk=keep]";
const SKIP: &str = ".walk button.w-skip";
const CHECK: &str = ".walk button.w-check";
const STOP: &str = ".walk button.w-stop";
const BACK: &str = ".walk-back";
const CLOSED: &str =
    "const found = !!document.querySelector('.walk'); return {ok: !found, saw: found};";
const UNKNOWN: &str = "unknown: may have landed, not resent";

/// Three quiet topics older than every topic the earlier cells made.
const MEMOS: &str = concat!(
    "2026-11-22T04:00:00Z\ttui\ttopic-g-a\ttitle: Guard A | state: quiet\n",
    "2026-11-22T04:10:00Z\ttui\ttopic-g-b\ttitle: Guard B | state: quiet\n",
    "2026-11-22T04:20:00Z\ttui\ttopic-g-c\ttitle: Guard C | state: quiet\n",
);

/// A second wrapper around the page's `fetch`: while `window.__s2.route` is
/// set, every `/api/state` read is made unconditional and answered with the
/// real body whose `route.id` is changed. Nothing else is touched.
const ROUTE_STUB: &str = r#"(() => {
  const inner = window.fetch.bind(window);
  window.fetch = async (url, init) => {
    if (url !== "/api/state" || !window.__s2.route) return inner(url, init);
    const headers = Object.assign({}, init && init.headers);
    delete headers["If-None-Match"];
    const answer = await inner(url, Object.assign({}, init, { headers }));
    if (answer.status !== 200) return answer;
    const data = await answer.json();
    if (data.route) data.route = Object.assign({}, data.route, { id: data.route.id + "-moved" });
    const kept = new Headers(answer.headers);
    kept.delete("content-length");
    return new Response(JSON.stringify(data), { status: 200, headers: kept });
  };
})()"#;

fn eval(tab: &mut Tab, check: &'static str, script: &str) -> Checked {
    tab.eval(script)
        .map(drop)
        .map_err(|error| Failed::new(check, error))
}

fn append(rig: &Rig, check: &'static str, text: &str) -> Checked {
    ae::store::open(&rig.dir)
        .append_memo(text.as_bytes())
        .map_err(|error| Failed::new(check, format!("{error:?}")))
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

fn carry_state(tab: &mut Tab, check: &'static str, state: &str) -> Checked {
    tab.until(
        check,
        "const saw = document.querySelector('.walk')?.dataset.carry; return {ok: saw === want, saw};",
        state,
        PAGE_WAIT,
    )
    .map(drop)
}

/// The Revisit line lists `count` topics: its walk button says so.
fn counted(tab: &mut Tab, check: &'static str, count: &str) -> Checked {
    tab.until(
        check,
        "const saw = document.querySelector('.revisit button.rv-walk')?.textContent; \
         return {ok: !!saw && saw.includes(want), saw};",
        count,
        PAGE_WAIT,
    )
    .map(drop)
}

/// The Revisit line shows `slug` and stays on it for the moment this reads.
fn shows(tab: &mut Tab, check: &'static str, slug: &str) -> Checked {
    tab.holds(
        check,
        "const e = document.querySelector('.revisit'); const saw = e?.dataset.slug; \
         return {ok: !!e && e.getClientRects().length > 0 && saw === want, saw};",
        slug,
    )
}

/// The Revisit line comes to `slug`, by Next presses, in at most four presses.
fn come_to(tab: &mut Tab, check: &'static str, slug: &str) -> Checked {
    for _ in 0..5 {
        let seen = tab
            .eval("document.querySelector('.revisit')?.dataset.slug ?? ''")
            .map_err(|error| Failed::new(check, error))?;
        if seen.as_str() == Some(slug) {
            return Ok(());
        }
        tab.click_selector(check, NEXT)?;
        tab.until(
            check,
            "const saw = document.querySelector('.revisit')?.dataset.slug; return {ok: saw !== want, saw};",
            seen.as_str().unwrap_or_default(),
            PAGE_WAIT,
        )?;
    }
    Err(Failed::new(
        check,
        format!("Next did not come to {slug} within five presses"),
    ))
}

/// A step that has no receipt keeps its Check again and its nonce when its
/// topic leaves Quiet: Skip, Escape and the backdrop do nothing.
fn skip_waits_for_the_receipt(tab: &mut Tab, rig: &Rig) -> Checked {
    const CASE: &str = "guard-skip";
    append(rig, CASE, MEMOS)?;
    counted(tab, CASE, "(4)")?;
    eval(tab, CASE, "window.__s2.mode = 'queue'")?;
    tab.click_selector(CASE, OPEN)?;
    step(tab, CASE, "Guard A")?;
    tab.click_selector(CASE, KEEP)?;
    carry_state(tab, CASE, "noanswer")?;
    append(
        rig,
        CASE,
        "2026-11-22T04:30:00Z\ttui\ttopic-g-a\tstate: moving\n",
    )?;
    tab.until(
        CASE,
        "const saw = document.querySelector('.walk')?.textContent?.toLowerCase() || ''; \
         return {ok: saw.includes('no longer quiet'), saw};",
        "",
        PAGE_WAIT,
    )?;
    tab.holds(
        CASE,
        "const skip = document.querySelector('.walk button.w-skip'); \
         const check = document.querySelector('.walk button.w-check'); \
         const stop = document.querySelector('.walk button.w-stop'); \
         return {ok: !!skip && skip.disabled && !!check && !check.disabled && !!stop && stop.disabled, \
         saw: {skip: skip?.outerHTML, check: check?.outerHTML, stop: stop?.outerHTML}};",
        "",
    )?;
    // A disabled control still takes the pointer, after the hit test, and does nothing.
    tab.click_selector(CASE, SKIP)?;
    key(tab, "Escape");
    tab.click_selector(CASE, BACK)?;
    tab.holds(
        CASE,
        "const walk = document.querySelector('.walk'); const text = walk?.textContent || ''; \
         return {ok: !!walk && walk.dataset.carry === 'noanswer' && text.includes('1 of 4') && \
         !walk.querySelector('.w-done li') && walk.querySelector('.nm')?.textContent?.trim() === want, \
         saw: text};",
        "Guard A",
    )?;
    let words = Value::str(UNKNOWN).render();
    eval(
        tab,
        CASE,
        &format!("window.__s2.mode = 'unknown'; window.__s2.words = {words}"),
    )?;
    tab.click_selector(CASE, CHECK)?;
    tab.until(
        CASE,
        "const saw = document.querySelector('.walk .w-step')?.textContent; \
         return {ok: !!saw && saw.includes(want), saw};",
        "2 of 4",
        PAGE_WAIT,
    )?;
    tab.click_selector(CASE, STOP)?;
    tab.until(CASE, CLOSED, "", PAGE_WAIT).map(drop)
}

/// A shown topic that left the list is replaced by the first one for good: it
/// does not take the selection back when it returns, and a shown topic that
/// stays is not moved by a new topic at the front.
fn selection_stays_where_it_was_put(tab: &mut Tab, rig: &Rig) -> Checked {
    const CASE: &str = "guard-selection";
    counted(tab, CASE, "(3)")?;
    come_to(tab, CASE, "g-c")?;
    append(
        rig,
        CASE,
        "2026-11-22T04:40:00Z\ttui\ttopic-g-c\tstate: moving\n",
    )?;
    counted(tab, CASE, "(2)")?;
    shows(tab, CASE, "g-b")?;
    append(
        rig,
        CASE,
        "2026-11-22T07:30:00Z\ttui\ttopic-g-c\tstate: quiet\n",
    )?;
    counted(tab, CASE, "(3)")?;
    shows(tab, CASE, "g-b")?;
    come_to(tab, CASE, "s2-edge-a")?;
    append(
        rig,
        CASE,
        "2026-11-22T03:00:00Z\ttui\ttopic-g-0\ttitle: Guard Zero | state: quiet\n",
    )?;
    counted(tab, CASE, "(4)")?;
    shows(tab, CASE, "s2-edge-a")
}

/// After the route changed, a walk sends nothing new, and the attempt that
/// was already open still checks its receipt with its first body.
fn route_change_stops_new_sends(tab: &mut Tab) -> Checked {
    const CASE: &str = "guard-route";
    eval(tab, CASE, ROUTE_STUB)?;
    eval(
        tab,
        CASE,
        "window.__s2.mode = 'queue'; window.__s2.route = false; window.__s2.base = window.__s2.posts.length",
    )?;
    tab.click_selector(CASE, OPEN)?;
    step(tab, CASE, "Guard Zero")?;
    tab.click_selector(CASE, KEEP)?;
    carry_state(tab, CASE, "noanswer")?;
    eval(tab, CASE, "window.__s2.route = true")?;
    tab.until(
        CASE,
        "const hints = [...document.querySelectorAll('.walk .w-hint')].map(e => e.textContent); \
         return {ok: hints.some(t => t.includes('orchestrator changed')), saw: hints};",
        "",
        PAGE_WAIT,
    )?;
    tab.holds(
        CASE,
        "const check = document.querySelector('.walk button.w-check'); \
         const keep = document.querySelector('.walk button[data-walk=keep]'); \
         return {ok: !!check && !check.disabled && (!keep || keep.disabled), \
         saw: {check: check?.outerHTML, keep: keep?.outerHTML}};",
        "",
    )?;
    let words = Value::str(UNKNOWN).render();
    eval(
        tab,
        CASE,
        &format!("window.__s2.mode = 'unknown'; window.__s2.words = {words}"),
    )?;
    tab.click_selector(CASE, CHECK)?;
    tab.until(
        CASE,
        "const saw = document.querySelector('.walk .w-step')?.textContent; \
         return {ok: !!saw && saw.includes(want), saw};",
        "2 of 4",
        PAGE_WAIT,
    )?;
    tab.holds(
        CASE,
        "const posts = window.__s2.posts.slice(window.__s2.base); const first = posts[0]; \
         const ok = posts.length === 121 && posts.every(p => p.text === first.text && \
         p.nonce === first.nonce && p.route === first.route); \
         return {ok, saw: {count: posts.length, first}};",
        "",
    )?;
    step(tab, CASE, "Guard B")?;
    tab.holds(
        CASE,
        "const keep = document.querySelector('.walk button[data-walk=keep]'); \
         return {ok: !!keep && keep.disabled, saw: keep?.outerHTML};",
        "",
    )?;
    tab.click_selector(CASE, KEEP)?;
    tab.holds(
        CASE,
        "const walk = document.querySelector('.walk'); \
         return {ok: !!walk && walk.dataset.carry === 'idle' && \
         window.__s2.posts.length === window.__s2.base + 121, \
         saw: {carry: walk?.dataset.carry, count: window.__s2.posts.length - window.__s2.base}};",
        "",
    )?;
    tab.click_selector(CASE, STOP)?;
    tab.until(CASE, CLOSED, "", PAGE_WAIT)?;
    eval(tab, CASE, "window.__s2.route = false")
}

pub(super) fn check_guards(tab: &mut Tab, rig: &Rig, cell: Cell) -> Checked {
    skip_waits_for_the_receipt(tab, rig)?;
    ok("guard-skip", cell);
    selection_stays_where_it_was_put(tab, rig)?;
    ok("guard-selection", cell);
    route_change_stops_new_sends(tab)?;
    ok("guard-route", cell);
    Ok(())
}
