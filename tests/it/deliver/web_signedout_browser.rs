//! Driver-owned browser checks of a page that has no session. The gate has no
//! JS engine, so a signed-out tab that keeps polling shows only here.
//!
//! The stage runs once, after the four cells, in the same tab. Each load is a
//! full document load (the tab goes through `about:blank` first, so a link's
//! fragment never reads as a same-document move) and each is counted by what
//! the page itself asked for: the fetches it makes to `/api/state`, counted by
//! a script that runs first in every new document.
//!
//! - `signed-out-quiet`: a tab that holds a session the listener no longer
//!   knows asks once, shows the refusal, clears the dead secret from its
//!   storage and then asks nothing more for 12 s, however often it is woken
//!   by a `visibilitychange`.
//! - `signed-out-bare`: a tab opened with no session and no code asks once.
//! - `signed-out-code`: a tab opened with a code the listener refuses asks
//!   nothing more once the exchange said no.
//! - `signed-out-links`: those three loads made the listener print exactly one
//!   fresh link: the first refusal printed it, and a waiting fresh code held
//!   every other back.
//! - `signed-out-resume`: a link's code set in the address bar of the halted
//!   tab (a fragment move, as a paste into the bar is) signs it in without a
//!   reload and polling comes back.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{Checked, Failed, PAGE_WAIT, Tab};
use ae::json::Value;

const KEY: &str = "ae-web-session";
/// A session secret no listener ever handed out.
const DEAD_SECRET: &str = "abababababababababababababababababababababababababababababababab";
/// A code no listener ever minted.
const DEAD_CODE: &str = "feedfacefeedfacefeedfacefeedface";
const NO_SESSION: &str = "open the newest link from ae web";

/// The new document is up: the stale marker of the old one is gone.
const LOADED: &str = "return { ok: window.__aeStale === undefined && document.readyState === \
     'complete' && !!document.querySelector('.app'), saw: { stale: window.__aeStale === 1, \
     state: document.readyState } };";

/// The tab left the page: the document is the blank one.
const BLANK: &str = "return { ok: location.href === 'about:blank', saw: { href: location.href } };";

const BANNER: &str = "const p = document.querySelector('p.problem'); const text = p ? \
     p.textContent : null; return { ok: text === want, saw: { text } };";

const NO_BANNER: &str = "const p = document.querySelector('p.problem'); return { ok: !p, saw: \
     { text: p ? p.textContent : null } };";

const SECRET_GONE: &str = "let kept = null; try { kept = sessionStorage.getItem(want); } catch (e) \
     { kept = 'storage throws'; } return { ok: kept === null, saw: { kept } };";

/// A wake only polls a page that shows: guard the check against a hidden tab.
const SHOWING: &str = "return { ok: document.hidden === false, saw: { hidden: document.hidden } };";

const WAKE: &str = "document.dispatchEvent(new Event('visibilitychange'))";

/// Runs first in every new document and counts the fetches the page makes to
/// `/api/state`, whatever the browser keeps of them afterwards.
const COUNT: &str = "window.__aeAsked = 0; const real = window.fetch; window.fetch = function \
     (input) { try { const url = new URL(typeof input === 'string' ? input : input.url, \
     location.href); if (url.pathname === '/api/state') { window.__aeAsked += 1; } } catch (e) { \
     } return real.apply(this, arguments); };";

const REQUESTS: &str = "window.__aeAsked";

/// What the tab shows when the count is wrong.
const DIAGNOSIS: &str = "JSON.stringify({ asked: window.__aeAsked, resources: \
     performance.getEntriesByType('resource').map((e) => e.name + ' ' + e.responseStatus), \
     hidden: document.hidden, visibility: document.visibilityState, href: location.href })";

const AT_LEAST: &str = "const n = window.__aeAsked; return { ok: n >= Number(want), saw: { n } };";

/// Signed in and drawn: the chip proves the seat and no refusal shows.
const SIGNED_IN: &str = "const chip = document.querySelector('.top .chip'); const p = \
     document.querySelector('p.problem'); return { ok: !!chip && chip.textContent.includes(' · \
     proven') && !p && location.hash === '', saw: { chip: chip ? chip.textContent : null, \
     problem: p ? p.textContent : null, hash: location.hash } };";

/// The whole stage, in the tab the four cells left. `base` is the page's
/// address, `links` the lines the listener printed as fresh links. The counter
/// script is installed for this stage only: on every way out it is removed, so
/// the stages after it start on a document that nothing wrapped.
pub(super) fn check_signed_out(tab: &mut Tab, base: &str, links: &Mutex<Vec<String>>) -> Checked {
    let added = tab.call(
        "Page.addScriptToEvaluateOnNewDocument",
        Value::obj([("source", Value::str(COUNT))]),
    );
    let Some(installed) = added.get_str("identifier").map(ToOwned::to_owned) else {
        return Err(Failed::new(
            "signed-out-quiet",
            "the counter script was not installed",
        ));
    };
    let result = stage(tab, base, links);
    drop(tab.call(
        "Page.removeScriptToEvaluateOnNewDocument",
        Value::obj([("identifier", Value::str(installed.as_str()))]),
    ));
    result
}

fn stage(tab: &mut Tab, base: &str, links: &Mutex<Vec<String>>) -> Checked {
    let before = printed(links).len();
    tab.problems.clear();
    arm(
        tab,
        &Arm {
            check: "signed-out-quiet",
            url: base.to_owned(),
            storage: Some(format!("sessionStorage.setItem('{KEY}', '{DEAD_SECRET}')")),
            hold: Duration::from_secs(12),
            wakes: &[3, 6, 9],
            asked: 1,
        },
    )?;
    arm(
        tab,
        &Arm {
            check: "signed-out-bare",
            url: base.to_owned(),
            storage: None,
            hold: Duration::from_secs(5),
            wakes: &[2],
            asked: 1,
        },
    )?;
    arm(
        tab,
        &Arm {
            check: "signed-out-code",
            url: format!("{base}#c={DEAD_CODE}"),
            storage: None,
            hold: Duration::from_secs(4),
            wakes: &[2],
            asked: 0,
        },
    )?;
    let everything = printed(links);
    let fresh = everything.get(before..).unwrap_or_default();
    if fresh.len() != 1 {
        return Err(Failed::new(
            "signed-out-links",
            format!(
                "{} fresh links printed for three signed-out loads, want 1: {fresh:?}",
                fresh.len()
            ),
        ));
    }
    println!("web-smoke ok signed-out-links (one fresh link for three signed-out loads)");
    let code = fresh
        .first()
        .and_then(|line| line.rsplit_once("#c="))
        .map(|(_, code)| code.to_owned())
        .unwrap_or_default();
    resume(tab, &code)?;
    tab.flush();
    // A refusal is not a fault of the page: the browser logs each refused state
    // read or code exchange as a network error. Anything else is a fault, and
    // only a stage that found none reaches the clear below.
    let faults: Vec<String> = tab
        .problems
        .iter()
        .filter(|problem| !is_refusal(problem))
        .cloned()
        .collect();
    if !faults.is_empty() {
        return Err(Failed::new("signed-out-quiet", faults.join("; ")));
    }
    // What this stage logged is judged; the stages after it start clean.
    tab.problems.clear();
    Ok(())
}

/// The console line the browser logs when the listener refuses a state read
/// or a code exchange with a 401: `console network: <text> <url>`.
fn is_refusal(problem: &str) -> bool {
    problem.starts_with("console network:")
        && problem.contains("status of 401")
        && (problem.ends_with("/api/state") || problem.ends_with("/api/session"))
}

/// One signed-out load: where to go, what the tab's storage holds first, how
/// long to watch, when to wake the page, and how many state reads it may ask.
struct Arm {
    check: &'static str,
    url: String,
    storage: Option<String>,
    hold: Duration,
    /// Seconds into the hold at which a `visibilitychange` wakes the page.
    wakes: &'static [u64],
    asked: i64,
}

fn arm(tab: &mut Tab, spec: &Arm) -> Checked {
    let check = spec.check;
    if let Some(script) = &spec.storage {
        tab.eval(script)
            .map_err(|why| Failed::new(check, format!("storage setup: {why}")))?;
    }
    drop(tab.eval("window.__aeStale = 1"));
    // Through about:blank, and only once it is the document: a fragment-only
    // address on the page the tab still shows would be a same-document move,
    // never the full load this stage counts.
    let blank = Value::obj([("url", Value::str("about:blank"))]);
    let left = tab.call("Page.navigate", blank);
    if let Some(error) = left.get_str("errorText") {
        return Err(Failed::new(
            check,
            format!("leaving the page failed: {error}"),
        ));
    }
    tab.until(check, BLANK, "", PAGE_WAIT)?;
    let loaded = tab.call(
        "Page.navigate",
        Value::obj([("url", Value::str(spec.url.as_str()))]),
    );
    if let Some(error) = loaded.get_str("errorText") {
        return Err(Failed::new(check, format!("the load failed: {error}")));
    }
    tab.until(check, LOADED, "", PAGE_WAIT)?;
    tab.until(check, BANNER, NO_SESSION, PAGE_WAIT)?;
    tab.holds(check, SECRET_GONE, KEY)?;
    tab.holds(check, SHOWING, "")?;
    let start = Instant::now();
    for at in spec.wakes {
        sleep_until(start + Duration::from_secs(*at));
        tab.eval(WAKE)
            .map_err(|why| Failed::new(check, format!("wake: {why}")))?;
    }
    sleep_until(start + spec.hold);
    let asked = requests(tab, check)?;
    if asked != spec.asked {
        let seen = tab
            .eval(DIAGNOSIS)
            .map_or_else(|why| why, |value| value.render());
        return Err(Failed::new(
            check,
            format!(
                "{asked} /api/state requests in {:?} after the refusal, want {}: {seen}",
                spec.hold, spec.asked
            ),
        ));
    }
    tab.holds(check, BANNER, NO_SESSION)?;
    println!(
        "web-smoke ok {check} ({asked} /api/state in {:?})",
        spec.hold
    );
    Ok(())
}

/// A link's code in the address bar of the halted tab.
fn resume(tab: &mut Tab, code: &str) -> Checked {
    let check = "signed-out-resume";
    if code.is_empty() {
        return Err(Failed::new(check, "no fresh link to take a code from"));
    }
    tab.holds(check, BANNER, NO_SESSION)?;
    tab.eval(&format!("location.hash = '#c={code}'"))
        .map_err(|why| Failed::new(check, format!("set the fragment: {why}")))?;
    tab.until(check, SIGNED_IN, "", PAGE_WAIT)?;
    tab.holds(check, NO_BANNER, "")?;
    // Not one read to prove it, but the timer running again.
    tab.until(check, AT_LEAST, "3", PAGE_WAIT)?;
    println!("web-smoke ok {check} (signed in by the fragment, polling again)");
    Ok(())
}

fn requests(tab: &mut Tab, check: &'static str) -> Result<i64, Failed> {
    match tab.eval(REQUESTS) {
        Ok(Value::Num(count)) => Ok(count),
        Ok(other) => Err(Failed::new(
            check,
            format!("the request count is not a number: {}", other.render()),
        )),
        Err(why) => Err(Failed::new(check, why)),
    }
}

fn printed(links: &Mutex<Vec<String>>) -> Vec<String> {
    links.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn sleep_until(at: Instant) {
    let left = at.saturating_duration_since(Instant::now());
    if !left.is_zero() {
        std::thread::sleep(left);
    }
}
