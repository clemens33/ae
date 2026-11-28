//! `just web-smoke`: the page in a real headless Chrome against a real
//! listener. The gate has no JS engine, so every other web test reads
//! the wire; this one reads what a browser makes of it. It is NEVER in `just
//! test`, `just test-affected`, the Mac subset or mutants: the live test is
//! `#[ignore]`d AND gated on `AE_WEB_SMOKE=1`, which only the recipe sets. The
//! one other test here needs no browser and runs in the gate: it holds the
//! run's rigs apart.
//!
//! One listener on 127.0.0.1 over a scratch state root whose orchestrator is
//! the fake TUI of [`super::Rig`], one Chrome started from the explicit path in
//! `AE_WEB_SMOKE_CHROME` with a scratch profile and home, and one tab driven
//! over the `DevTools` protocol. Every click is a real pointer press and
//! release at the control's centre, never `HTMLElement.click()`, which skips
//! the mousedown that decides where the focus goes. Four cells, 1440x900 and
//! 360x740, each light and dark, each a fresh load of the page:
//!
//! - `modules`: the page rendered the orchestrator's state under the strict
//!   CSP header, with no exception, console error, dialog or CSP violation.
//! - `hostile-literal`: a topic titled with markup shows that literal text,
//!   and the page holds no `img` and no inline `script`.
//! - `theme`: the page background is the scheme's own.
//! - `fits`: no horizontal scroll, and the draft box lies inside the viewport.
//! - wide cells, `draft-kept` and `focus-kept`: a click into the draft box,
//!   typed text, then a click on a topic row opens its card while the draft
//!   keeps its text and the focus.
//! - narrow cells, `draft-kept`: the same draft survives Overview, the topic
//!   and the way back to Chat.
//! - `sent`, in the 1440x900 dark cell only: on the All tab, a click on Send
//!   shows the receipt with its request id, and the fake TUI received the draft
//!   once.
//!
//! After the four cells one signed-out stage runs in the same tab (see
//! `web_signedout_browser`): a page whose session is gone asks the listener
//! once and then stops until a code is redeemed in its tab. The compose cases
//! (`web_compose_browser_spec`), the topic tab cases
//! (`web_topic_tabs_browser_spec`), the tab lifecycle cases
//! (`web_topic_lifecycle_browser_spec`), the side panel cases and then the
//! side panel's accessibility and keyboard cases (both in
//! `web_side_panel_browser_spec`) follow, each on listeners of their own.
//!
//! `AE_WEB_SMOKE_PLANT=focus` serves `app.mjs` without the topic row's
//! mousedown `preventDefault`. That run must fail at `focus-kept`: `just
//! web-smoke plant` passes only when it fails there and nowhere else.
//! `AE_WEB_SMOKE_PLANT=walk` serves `app.mjs` with the walk's Keep sending the
//! Ask line. That run must fail at `walk-keep (1440x900 dark)`: `just web-smoke
//! plant-walk` passes only when it fails there and nowhere else.
//! `AE_WEB_SMOKE_PLANT=undo` serves `app.mjs` with the Undo of a counting owner
//! line hiding the line but leaving its countdown to send it one second after
//! its deadline. That run must fail at `s3a-undo (1440x900 dark)`: `just
//! web-smoke plant-undo` passes only when it fails there and nowhere else.
//! `AE_WEB_SMOKE_PLANT=tab` serves `app.mjs` without the side panel row's
//! mousedown `preventDefault`. That run must fail at `tab-focus-kept (1440x900 dark)`:
//! `just web-smoke plant-tab` passes only when it fails there and nowhere else.

use std::collections::BTreeMap;
use std::hash::BuildHasher as _;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ae::json::{self, Value};
use ae::web::clock::SystemClock;
use ae::web::send::Sends;
use ae::web::server::{self, NoProbe, Options, Running};
use ae::web::state::LiveSource;

use super::{Rig, await_evidence};
use crate::cli;
use crate::parity::capture::raw;
use crate::phase2::run_tmux;

const GATE: &str = "AE_WEB_SMOKE";
const CHROME: &str = "AE_WEB_SMOKE_CHROME";
const PLANT: &str = "AE_WEB_SMOKE_PLANT";
const OUT: &str = "AE_WEB_SMOKE_OUT";

/// Chrome writing its `DevTools` port, and its first tab appearing.
const START_WAIT: Duration = Duration::from_secs(30);
/// One page state: a load, a card, a view switch.
const PAGE_WAIT: Duration = Duration::from_secs(30);
/// The send's receipt: the carry to the fake TUI included.
const SEND_WAIT: Duration = Duration::from_secs(90);
/// One read on the `DevTools` socket.
const READ_WAIT: Duration = Duration::from_mins(1);
/// Chrome's own exit after `Browser.close`.
const CLOSE_WAIT: Duration = Duration::from_secs(10);
/// The largest `DevTools` message read; a screenshot is far below it.
const FRAME_MAX: u64 = 64 << 20;

const SLUG: &str = "smoke";
const HOSTILE: &str = "<img src=x onerror=alert(1)> & </p>";
const HOSTILE_WHAT: &str = "<script>alert(2)</script>";
const DRAFT: &str = "web smoke: one owner ask";
const LIGHT_BG: &str = "rgb(247, 248, 250)";
const DARK_BG: &str = "rgb(43, 43, 43)";

/// The topic row's focus guard as `app.mjs` serves it, and what the plant
/// serves instead: the same row without its mousedown `preventDefault`.
const ROW_GUARD: &str =
    "onMouseDown=${(event) => event.preventDefault()}\n    onClick=${() => choose(row.slug)}";
const ROW_PLANTED: &str = "onClick=${() => choose(row.slug)}";

/// The walk's Keep button as `app.mjs` serves it, and what the walk plant
/// serves instead: the same button, which sends the Ask line.
const WALK_GUARD: &str = "onClick=${() => act(\"keep\")}";
const WALK_PLANTED: &str = "onClick=${() => act(\"ask\")}";

/// An owner line's Undo as `app.mjs` serves it: the countdown is stopped and the
/// line is idle. The undo plant serves the same Undo with the countdown left
/// running, so the line is sent a second after its deadline.
const UNDO_GUARD: &str = "clearTimeout(r.timer);\n      r.phase = \"idle\";";
const UNDO_PLANTED: &str = "r.phase = \"idle\";\n      setTimeout(() => {\n        r.phase = \"count\";\n        api.fire(key);\n      }, Math.max(0, r.deadline - Date.now()) + 1000);";

/// A side panel row's focus guard as `app.mjs` serves it, and what the tab plant
/// serves instead: the same row without its mousedown `preventDefault`.
const TAB_GUARD: &str =
    "onMouseDown=${(press) => press.preventDefault()}\n    onClick=${() => select(slug)}";
const TAB_PLANTED: &str = "onClick=${() => select(slug)}";

/// What each check's script sees: `tag` names an element, `row` finds the
/// smoke topic's first row, `box` is the draft box.
const PRELUDE: &str = "const tag = (el) => (el ? el.tagName + (el.className ? '.' + \
     el.className : '') : null); const row = () => [...document.querySelectorAll('button.row')]\
     .find((b) => b.querySelector('.ln').textContent.startsWith('#smoke')); const box = () => \
     document.querySelector('.composer textarea');";

const READY: &str = "const chip = document.querySelector('.top .chip'); const rows = \
     document.querySelectorAll('button.row').length; const ok = window.__aeStale === undefined \
     && !!chip && chip.textContent.includes(' · proven') && rows >= 2 && !!box() && \
     box().placeholder.startsWith('Message '); return { ok, saw: { stale: window.__aeStale === 1, \
     chip: chip ? chip.textContent : null, rows, placeholder: box() ? box().placeholder : null } };";

const MODULES: &str = "const served = await fetch('/', { cache: 'no-store' }); const csp = \
     served.headers.get('content-security-policy') || ''; const inline = \
     document.querySelectorAll('script:not([src])').length; const module = \
     !!document.querySelector('script[type=\"module\"][src=\"/assets/app.mjs\"]'); const \
     violations = window.__aeCsp === undefined ? null : window.__aeCsp; const ok = csp === want \
     && inline === 0 && module && Array.isArray(violations) && violations.length === 0; return \
     { ok, saw: { csp, inline, module, violations } };";

const HOSTILE_ROW: &str = "const name = row() ? row().querySelector('.nm').textContent : null; \
     const imgs = document.querySelectorAll('img').length; return { ok: name === want && imgs \
     === 0, saw: { name, imgs } };";

const THEME: &str = "const bg = getComputedStyle(document.body).backgroundColor; const dark = \
     matchMedia('(prefers-color-scheme: dark)').matches; return { ok: bg === want, saw: { bg, \
     dark } };";

const FITS: &str = "const r = box().getBoundingClientRect(); const wide = \
     document.documentElement.scrollWidth; const ok = wide <= innerWidth && r.width > 0 && \
     r.height > 0 && r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= \
     innerHeight; return { ok, saw: { scrollWidth: wide, innerWidth, innerHeight, box: [r.left, \
     r.top, r.right, r.bottom].map(Math.round) } };";

const TYPED: &str = "return { ok: box().value === want && document.activeElement === box(), \
     saw: { value: box().value, active: tag(document.activeElement) } };";

const CARD: &str = "const h = document.querySelector('.card h2'); const sub = \
     document.querySelector('.card .sub'); return { ok: !!h && h.textContent === want && !!sub && \
     sub.textContent.startsWith('#smoke '), saw: { title: h ? h.textContent : null, sub: sub ? \
     sub.textContent : null } };";

const HOSTILE_CARD: &str = "const raws = [...document.querySelectorAll('.card .fld .raw')]\
     .map((p) => p.textContent); const imgs = document.querySelectorAll('img').length; const \
     inline = document.querySelectorAll('script:not([src])').length; return { ok: \
     raws.includes(want) && imgs === 0 && inline === 0, saw: { raws, imgs, inline } };";

const DRAFT_KEPT: &str = "return { ok: box().value === want, saw: { value: box().value } };";

const FOCUS_KEPT: &str = "return { ok: document.activeElement === box(), saw: { active: \
     tag(document.activeElement), want: 'TEXTAREA' } };";

const VIEW: &str = "const app = document.querySelector('.app'); return { ok: !!app && \
     app.classList.contains(want), saw: { app: app ? app.className : null } };";

const RECEIPT: &str = "const sent = document.querySelector('.composer .receipt.k-sent'); const \
     any = document.querySelector('.composer .receipt'); const id = sent && \
     sent.querySelector('.mute') ? sent.querySelector('.mute').textContent.trim() : ''; return { \
     ok: !!sent && sent.textContent.trim().startsWith(want) && box().value === '', saw: { \
     receipt: any ? any.className + ': ' + any.textContent.trim() : null, value: box().value }, \
     id };";

/// The centre of `want`'s element and whether a press there lands on it.
const SPOT: &str = "const el = want === 'box' ? box() : want === 'row' ? row() : want === 'send' \
     ? document.querySelector('.composer button.send') : want === 'panel' ? \
     document.querySelector('[data-role=view-panel]') : want === 'chat' ? \
     document.querySelector('[data-role=view-chat]') : null; if (!el) { return { ok: false, saw: \
     'no ' + want }; } el.scrollIntoView({ block: 'nearest' }); const r = \
     el.getBoundingClientRect(); const x = Math.round(r.left + r.width / 2); const y = \
     Math.round(r.top + r.height / 2); const hit = document.elementFromPoint(x, y); return { ok: \
     !!hit && (hit === el || el.contains(hit)), x, y, saw: { x, y, hit: tag(hit) } };";

/// The press point of the element `want` selects, and whether a press there
/// lands on it: its centre, or 4 px in from its top-left corner when it covers
/// the whole viewport (an overlay's centre is its content). The overlay itself
/// must receive the press; any other element may receive it through a child.
const SPOT_SELECTOR: &str = "const el = document.querySelector(want); if (!el) { return { ok: \
     false, saw: 'no ' + want }; } el.scrollIntoView({ block: 'nearest' }); const r = \
     el.getBoundingClientRect(); const covers = r.left <= 0 && r.top <= 0 && r.right >= \
     innerWidth && r.bottom >= innerHeight; const x = covers ? Math.round(r.left) + 4 : \
     Math.round(r.left + r.width / 2); const y = covers ? Math.round(r.top) + 4 : \
     Math.round(r.top + r.height / 2); const hit = document.elementFromPoint(x, y); return { ok: \
     !!hit && (hit === el || (!covers && el.contains(hit))), x, y, saw: { x, y, hit: tag(hit) } };";

/// Before every document: count CSP violations where the checks read them.
const WATCH: &str = "window.__aeCsp = []; document.addEventListener('securitypolicyviolation', \
     (e) => window.__aeCsp.push(e.violatedDirective + ' ' + e.blockedURI));";

#[derive(Debug, Clone, Copy)]
struct Cell {
    width: i64,
    height: i64,
    dark: bool,
}

const CELLS: [Cell; 4] = [
    Cell {
        width: 1440,
        height: 900,
        dark: false,
    },
    Cell {
        width: 360,
        height: 740,
        dark: false,
    },
    Cell {
        width: 360,
        height: 740,
        dark: true,
    },
    Cell {
        width: 1440,
        height: 900,
        dark: true,
    },
];

/// The one cell that sends.
const SEND_CELL: usize = 3;

impl Cell {
    fn wide(self) -> bool {
        self.width >= 760
    }

    fn scheme(self) -> &'static str {
        if self.dark { "dark" } else { "light" }
    }

    fn name(self) -> String {
        format!("{}x{} {}", self.width, self.height, self.scheme())
    }

    fn file(self) -> String {
        format!("{}x{}-{}", self.width, self.height, self.scheme())
    }
}

/// A check that did not hold: its name and what the page showed instead.
struct Failed {
    check: &'static str,
    detail: String,
}

impl Failed {
    fn new(check: &'static str, detail: impl Into<String>) -> Self {
        Self {
            check,
            detail: detail.into(),
        }
    }
}

type Checked = Result<(), Failed>;

fn ok(check: &str, cell: Cell) {
    println!("web-smoke ok {check} ({})", cell.name());
}

#[allow(
    clippy::disallowed_methods,
    reason = "the smoke's own switches: the recipe sets them, and nothing else reads them"
)]
fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn gated() -> bool {
    env_text(GATE).as_deref() == Some("1")
}

/// Chrome is the explicit absolute path the recipe passes, never a `PATH`
/// lookup that could start another binary.
#[allow(
    clippy::disallowed_methods,
    reason = "the smoke checks the browser path it is told to start before it starts it"
)]
fn chrome_path() -> PathBuf {
    let named = env_text(CHROME).unwrap_or_default();
    let path = PathBuf::from(&named);
    assert!(
        path.is_absolute() && std::fs::metadata(&path).is_ok_and(|meta| meta.is_file()),
        "web-smoke FAILED at chrome-path: {CHROME}={named:?} is not an absolute path to a file"
    );
    path
}

fn planted() -> Option<&'static str> {
    match env_text(PLANT).as_deref() {
        None => None,
        Some("focus") => Some("focus"),
        Some("walk") => Some("walk"),
        Some("undo") => Some("undo"),
        Some("tab") => Some("tab"),
        Some(other) => {
            panic!(
                "web-smoke: {PLANT}={other:?}; the planted faults are `focus`, `walk`, `undo` and `tab`"
            )
        }
    }
}

fn out_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env_text(OUT)?);
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|error| panic!("web-smoke: create {}: {error}", dir.display()));
    Some(dir)
}

/// The orchestrator role on the compact rig, as the send specs set it up, and
/// a memo with one hostile topic that needs the owner and one calm one.
///
/// A run builds more of these while earlier ones are alive, and a rig's scratch
/// root, tmux socket and session are all named by its tag, with a root removed
/// before it is made. So every call takes its own tag. The count is per process
/// because every test process already has a scratch namespace of its own.
fn smoke_rig() -> Rig {
    static BUILT: AtomicUsize = AtomicUsize::new(0);
    let nth = BUILT.fetch_add(1, Ordering::Relaxed);
    let rig = Rig::compact(&format!("websmoke{nth}"));
    let overlay = rig.scratch.join("orchestrator.config");
    std::fs::write(&overlay, "[workspace]\norchestrator = true\n")
        .unwrap_or_else(|error| panic!("write role overlay: {error}"));
    let meta_path = rig.dir.join("meta");
    let meta =
        std::fs::read_to_string(&meta_path).unwrap_or_else(|error| panic!("read meta: {error}"));
    std::fs::write(
        meta_path,
        format!(
            "{meta}schema=2\nlayout=lead-pair\nmeta_agent=true\nlocal_config={}\n\
             main_pane={}\nseat.worker.0=side-old\nlaunch_id.worker.0=worker-launch\n",
            overlay.display(),
            rig.pane,
        ),
    )
    .unwrap_or_else(|error| panic!("write role meta: {error}"));
    std::fs::write(rig.scratch.join("config"), "[workspace]\n")
        .unwrap_or_else(|error| panic!("write global fixture config: {error}"));
    std::fs::write(
        rig.dir.join("memo.tsv"),
        format!(
            "2026-11-22T10:00:00Z\ttui\ttopic-{SLUG}\ttitle: {HOSTILE} | state: needs | what: \
             {HOSTILE_WHAT}\n2026-11-22T10:01:00Z\ttui\ttopic-calm\ttitle: Calm topic | state: \
             moving\n{}",
            web_walk_browser_spec::EXTRA_MEMOS
        ),
    )
    .unwrap_or_else(|error| panic!("write memo: {error}"));
    rig
}

/// A rig's tmux server and fake TUI, each by pid and start time, so a reused
/// pid never passes for the same process.
///
/// The query is the argv `Rig::tmux` builds, run through `phase2::run_tmux`:
/// calling `Rig::tmux` from this file would make it a shared boundary helper
/// that needs a pin of its own in the Mac subset.
fn rig_processes(rig: &Rig, probe: &Path) -> Vec<(u32, String)> {
    let mut args = ae::tmux::server_args(&rig.server());
    let query = [
        "display-message",
        "-p",
        "-t",
        &rig.pane,
        "#{pid} #{pane_pid}",
    ];
    args.extend(query.map(ToOwned::to_owned));
    let (answered, line) = run_tmux(&args, &rig.scratch);
    assert!(answered, "the rig's server answers on its socket: {line:?}");
    let processes: Vec<(u32, String)> = line
        .split_whitespace()
        .map(|word| {
            let pid = word
                .parse()
                .unwrap_or_else(|error| panic!("a pid in {line:?}: {error}"));
            let started = raw::process_identity(pid, probe)
                .unwrap_or_else(|| panic!("a start time for pid {pid}"));
            (pid, started)
        })
        .collect();
    assert_eq!(processes.len(), 2, "a server and a fake TUI: {line:?}");
    processes
}

/// Waits, bounded, until every one of `processes` is gone: `kill-server` can
/// answer before the pane's process has ended. One that cannot be judged counts
/// as running.
fn await_ended(processes: &[(u32, String)], probe: &Path) {
    let ended = await_evidence(super::WITNESS_WAIT, || {
        processes
            .iter()
            .all(|(pid, started)| raw::owner_is_dead(*pid, Some(started), probe))
            .then_some(())
    });
    assert!(ended.is_some(), "still running: {processes:?}");
}

/// A web-smoke run builds more rigs while earlier ones are alive. `just
/// web-smoke` is never in the gate, so this is what keeps those rigs apart: a
/// later rig leaves an earlier one whole, and each ends its own tmux server and
/// fake TUI when it drops, an unwinding stage included.
#[test]
fn each_smoke_rig_keeps_to_itself_and_ends_its_server_and_fake_tui() {
    let probe = cli::OwnedScratch::root("dlv", "wsprobe");
    let outer = smoke_rig();
    let outer_ran = rig_processes(&outer, probe.path());
    let nested = smoke_rig();
    let nested_ran = rig_processes(&nested, probe.path());
    // Printed first: a failure past this line may leave these running.
    eprintln!("smoke rigs (pid, start): outer {outer_ran:?} nested {nested_ran:?}");
    assert_ne!(nested.scratch, outer.scratch);
    assert_ne!(nested.sock, outer.sock);
    assert_ne!(nested.session, outer.session);
    let unwound = std::panic::catch_unwind(AssertUnwindSafe(move || {
        let _held_by_the_stage = nested;
        panic!("planted: a stage panics while its rig is alive");
    }));
    assert!(unwound.is_err());
    await_ended(&nested_ran, probe.path());
    assert_eq!(rig_processes(&outer, probe.path()), outer_ran);
    assert!(outer.dir.join("memo.tsv").is_file());
    // While the rig lives `owner_is_dead` says no, so `await_ended` waits for a real end.
    for (pid, started) in &outer_ran {
        assert!(
            !raw::owner_is_dead(*pid, Some(started), probe.path()),
            "pid {pid} judged ended while its rig is alive"
        );
    }
    drop(outer);
    await_ended(&outer_ran, probe.path());
}

fn start_listener(root: &Path) -> Running {
    start_listener_with(root, Arc::new(|_| {}))
}

/// The same listener; the fresh link lines it prints are kept in `fresh`.
fn start_listener_noting(root: &Path, fresh: &Arc<Mutex<Vec<String>>>) -> Running {
    let kept = Arc::clone(fresh);
    start_listener_with(
        root,
        Arc::new(move |line| {
            kept.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.to_owned());
        }),
    )
}

fn start_listener_with(root: &Path, links: Arc<dyn Fn(&str) + Send + Sync>) -> Running {
    server::start_with(
        Options {
            port: 0,
            clock: Arc::new(SystemClock),
            source: Arc::new(LiveSource::new(root.to_path_buf(), root.join("config"))),
            probe: Arc::new(NoProbe),
            links,
            version: "web-smoke".to_owned(),
        },
        Sends::live(root.to_path_buf()),
    )
    .unwrap_or_else(|error| panic!("web-smoke: start listener: {error}"))
}

/// The live smoke; see the module docs.
#[test]
#[ignore = "live: starts headless Chrome against a real listener; needs AE_WEB_SMOKE=1 and AE_WEB_SMOKE_CHROME (just web-smoke)"]
fn live_web_smoke() {
    if !gated() {
        eprintln!("web smoke: {GATE}=1 is not set — nothing attempted");
        return;
    }
    let chrome = chrome_path();
    let plant = planted();
    let out = out_dir();
    let rig = smoke_rig();
    let fresh = Arc::new(Mutex::new(Vec::new()));
    let listen = start_listener_noting(&rig.scratch, &fresh);
    assert!(
        listen.addr().ip().is_loopback(),
        "web-smoke: the listener left loopback"
    );
    let mut browser = Browser::start(&chrome, &rig.scratch);
    let mut tab = browser.tab();
    tab.prepare(plant.is_some());
    tab.fault = plant;
    let link = listen.link();
    let mut failure = None;
    for (at, cell) in CELLS.into_iter().enumerate() {
        // The first load trades the link's code; the later ones reload the tab,
        // whose session the page kept.
        let url = (at == 0).then_some(link.as_str());
        let mut result = run_cell(&mut tab, &rig, cell, url, at == SEND_CELL);
        if plant.is_some() && tab.planted == 0 {
            result = Err(Failed::new(
                "plant",
                "/assets/app.mjs was never intercepted",
            ));
        }
        // A failed cell's picture is evidence; a passing cell's is the receipt.
        match (tab.shot(out.as_deref(), cell, result.is_err()), &result) {
            (Err(why), Ok(())) => result = Err(Failed::new("shot", why)),
            (Err(why), Err(_)) => eprintln!("web-smoke shot: {why}"),
            (Ok(()), _) => {}
        }
        if let Err(failed) = result {
            failure = Some(format!(
                "web-smoke FAILED at {} ({}): {}",
                failed.check,
                cell.name(),
                failed.detail
            ));
            break;
        }
    }
    // The signed-out stage runs once, right after the cells, in the tab they
    // left and on their listener (a dead session secret is stored for that
    // origin); a planted run has already failed at its own check and never
    // reaches it. It leaves the tab on a signed-in page and clears the console
    // entries of its own refusals, so the compose cases below start as before.
    if failure.is_none() && plant.is_none() {
        let base = format!("http://{}/", listen.addr());
        if let Err(failed) = web_signedout_browser::check_signed_out(&mut tab, &base, &fresh) {
            failure = Some(format!(
                "web-smoke FAILED at {} (signed-out): {}",
                failed.check, failed.detail
            ));
        }
    }
    // The S3a compose cases follow on the same tab, then the topic tab cases, the
    // tab lifecycle cases, the side panel cases and the side panel's accessibility
    // and keyboard cases, each only when every earlier check passed, so a plant
    // that fails an earlier check never reaches them.
    if failure.is_none() {
        let ran = web_compose_browser_spec::check_s3a(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    if failure.is_none() {
        let ran = web_topic_tabs_browser_spec::check_t1(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    if failure.is_none() {
        let ran = web_topic_lifecycle_browser_spec::check_t2(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    if failure.is_none() {
        let ran = web_side_panel_browser_spec::check_t4(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    if failure.is_none() {
        let ran = web_side_panel_browser_spec::check_t5(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    if failure.is_none() {
        let ran = web_p1_browser_spec::check_p1(&mut tab);
        failure = later_stage(&mut tab, out.as_deref(), ran);
    }
    // A checked failure ends Chrome the way a pass does, before it is told:
    // the kill on drop stays for a protocol failure that panicked mid-cell.
    tab.close();
    browser.wait();
    listen.shutdown();
    if let Some(words) = failure {
        panic!("{words}");
    }
    println!(
        "web-smoke PASS: 4 cells, sends in the one send cell, then the signed-out stage, the \
         compose cases, the topic tab cases, the tab lifecycle cases, the side panel cases and \
         the side panel's accessibility and keyboard cases, then desktop polish"
    );
}

/// One stage after the cells, in the wide dark viewport they end in, and its
/// failure as the run tells it. Only a failure is pictured here: a pass keeps
/// the receipt shot of the 1440x900 dark cell, taken before the stages changed
/// the page.
fn later_stage(tab: &mut Tab, out: Option<&Path>, ran: Checked) -> Option<String> {
    let cell = Cell {
        width: 1440,
        height: 900,
        dark: true,
    };
    let failed = ran.err()?;
    if let Err(why) = tab.shot(out, cell, true) {
        eprintln!("web-smoke shot: {why}");
    }
    Some(format!(
        "web-smoke FAILED at {} ({}): {}",
        failed.check,
        cell.name(),
        failed.detail
    ))
}

fn run_cell(tab: &mut Tab, rig: &Rig, cell: Cell, url: Option<&str>, send: bool) -> Checked {
    tab.view(cell);
    tab.problems.clear();
    // The old document, if any, answers no check of the new one.
    drop(tab.eval("window.__aeStale = 1"));
    let loaded = match url {
        Some(url) => tab.call("Page.navigate", Value::obj([("url", Value::str(url))])),
        None => tab.call(
            "Page.reload",
            Value::obj([("ignoreCache", Value::Bool(true))]),
        ),
    };
    if let Some(error) = loaded.get_str("errorText") {
        return Err(Failed::new("modules", format!("the load failed: {error}")));
    }
    tab.until("modules", READY, "", PAGE_WAIT)?;
    if cell.wide() {
        web_p1_browser_spec::favicon(tab, cell)?;
    }
    tab.flush();
    if !tab.problems.is_empty() {
        return Err(Failed::new("modules", tab.problems.join("; ")));
    }
    tab.holds("modules", MODULES, ae::web::http::CSP)?;
    ok("modules", cell);
    tab.holds("hostile-literal", HOSTILE_ROW, HOSTILE)?;
    ok("hostile-literal", cell);
    tab.holds("theme", THEME, if cell.dark { DARK_BG } else { LIGHT_BG })?;
    ok("theme", cell);
    tab.holds("fits", FITS, "")?;
    ok("fits", cell);
    web_walk_browser_spec::check_revisit(tab, cell)?;
    tab.click("typed", "box")?;
    tab.call(
        "Input.insertText",
        Value::obj([("text", Value::str(DRAFT))]),
    );
    tab.until("typed", TYPED, DRAFT, PAGE_WAIT)?;
    if cell.wide() {
        tab.click("card-open", "row")?;
        tab.until("card-open", CARD, HOSTILE, PAGE_WAIT)?;
        tab.holds("hostile-literal", HOSTILE_CARD, HOSTILE_WHAT)?;
        tab.holds("draft-kept", DRAFT_KEPT, DRAFT)?;
        ok("draft-kept", cell);
        tab.holds("focus-kept", FOCUS_KEPT, "")?;
        ok("focus-kept", cell);
    } else {
        tab.click("card-open", "panel")?;
        tab.until("card-open", VIEW, "view-panel", PAGE_WAIT)?;
        tab.click("card-open", "row")?;
        tab.until("card-open", VIEW, "view-panel", PAGE_WAIT)?;
        tab.until("card-open", CARD, HOSTILE, PAGE_WAIT)?;
        tab.click("draft-kept", "chat")?;
        tab.until("draft-kept", VIEW, "view-chat", PAGE_WAIT)?;
        tab.holds("draft-kept", DRAFT_KEPT, DRAFT)?;
        ok("draft-kept", cell);
    }
    if send {
        // On the All tab the draft goes as typed, and the walk is there.
        show_all(tab, "sent")?;
        sent(tab, rig)?;
        ok("sent", cell);
        web_walk_browser_spec::check_walk(tab, rig, cell)?;
        web_walk_browser_spec::check_edges(tab, rig, cell)?;
        show_all(tab, "guard-skip")?;
        web_guard_browser::check_guards(tab, rig, cell)?;
    }
    tab.flush();
    if !tab.problems.is_empty() {
        return Err(Failed::new("modules", tab.problems.join("; ")));
    }
    Ok(())
}

/// A real click on the All row, until the draft carries no topic tag.
fn show_all(tab: &mut Tab, check: &'static str) -> Checked {
    tab.click_selector(check, "button[data-role=side-all]")?;
    tab.until(
        check,
        "const saw = {chip: !!document.querySelector('[data-role=tag-chip]'), all: \
         !!document.querySelector('[data-panel=all]')}; return {ok: !saw.chip && saw.all, saw};",
        "",
        PAGE_WAIT,
    )
    .map(drop)
}

/// A real click on Send, its receipt in the page, and the draft once in the
/// seat's input.
fn sent(tab: &mut Tab, rig: &Rig) -> Checked {
    tab.click("sent", "send")?;
    let want = format!("sent to tui in {}", rig.session);
    let seen = tab.until("sent", RECEIPT, &want, SEND_WAIT)?;
    let id = seen.get_str("id").unwrap_or_default();
    if !ae::tracked::is_request_id(id) {
        return Err(Failed::new(
            "sent",
            format!("the receipt names no request id: {}", seen.render()),
        ));
    }
    let received = rig.submitted();
    if received.matches(DRAFT).count() != 1 {
        return Err(Failed::new(
            "sent",
            format!("the seat received the draft not exactly once: {received:?}"),
        ));
    }
    Ok(())
}

/// Chrome, started from its explicit path with a scratch profile and home;
/// killed on drop if `Browser.close` did not end it.
struct Browser {
    child: cli::OwnedChild,
    port: u16,
    log: PathBuf,
}

impl Browser {
    fn start(chrome: &Path, scratch: &Path) -> Self {
        let profile = scratch.join("chrome-profile");
        let home = scratch.join("chrome-home");
        for dir in [&profile, &home] {
            std::fs::create_dir_all(dir)
                .unwrap_or_else(|error| panic!("web-smoke: create {}: {error}", dir.display()));
        }
        let log = scratch.join("chrome.log");
        let sink = std::fs::File::create(&log)
            .unwrap_or_else(|error| panic!("web-smoke: create {}: {error}", log.display()));
        let profile_arg = format!("--user-data-dir={}", profile.display());
        let mut command = cli::helper(chrome);
        command
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .args([
                "--headless",
                "--remote-debugging-port=0",
                profile_arg.as_str(),
                "--no-first-run",
                "--no-default-browser-check",
                "--no-proxy-server",
                "--disable-extensions",
                "--disable-sync",
                "--disable-background-networking",
                "--disable-component-update",
                "--disable-default-apps",
                "--password-store=basic",
                "--use-mock-keychain",
                "--window-size=1440,900",
                "about:blank",
            ])
            .stdout(Stdio::null())
            .stderr(sink);
        let mut child = command
            .spawn()
            .unwrap_or_else(|error| panic!("web-smoke FAILED at chrome-start: {error}"));
        let active = profile.join("DevToolsActivePort");
        let port = await_evidence(START_WAIT, || {
            if let Ok(Some(status)) = child.try_wait() {
                panic!(
                    "web-smoke FAILED at chrome-start: Chrome exited {status}: {}",
                    tail(&log)
                );
            }
            let text = std::fs::read_to_string(&active).ok()?;
            text.lines().next()?.trim().parse::<u16>().ok()
        })
        .unwrap_or_else(|| {
            panic!(
                "web-smoke FAILED at chrome-start: no DevTools port within {START_WAIT:?}: {}",
                tail(&log)
            )
        });
        println!("web-smoke chrome: {} on 127.0.0.1:{port}", chrome.display());
        Self { child, port, log }
    }

    /// The first page target, attached over its own socket.
    fn tab(&self) -> Tab {
        let port = self.port;
        let path = await_evidence(START_WAIT, || {
            let list = json::parse(&http_get(port, "/json/list")?).ok()?;
            let Value::Arr(targets) = list else {
                return None;
            };
            targets
                .iter()
                .find(|target| target.get_str("type") == Some("page"))
                .and_then(|target| target.get_str("webSocketDebuggerUrl"))
                .and_then(|url| url.find("/devtools/").map(|at| url[at..].to_owned()))
        })
        .unwrap_or_else(|| {
            panic!(
                "web-smoke FAILED at chrome-start: no page target within {START_WAIT:?}: {}",
                tail(&self.log)
            )
        });
        Tab {
            socket: Socket::open(port, &path),
            next: 1,
            replies: BTreeMap::new(),
            problems: Vec::new(),
            planted: 0,
            fault: None,
        }
    }

    /// Wait for Chrome's own exit after `Browser.close`; the drop kills it
    /// otherwise.
    fn wait(&mut self) {
        let child = &mut self.child;
        let ended = await_evidence(CLOSE_WAIT, || child.try_wait().ok().flatten());
        if ended.is_none() {
            eprintln!("web-smoke: Chrome outlived Browser.close by {CLOSE_WAIT:?}; killing it");
        }
    }
}

fn tail(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(20);
    format!("chrome log tail: {}", lines[from..].join(" | "))
}

/// One GET to Chrome's `DevTools` HTTP endpoint: the body, once whole.
fn http_get(port: u16, path: &str) -> Option<String> {
    let host = SocketAddr::from(([127, 0, 0, 1], port));
    let mut tcp = TcpStream::connect_timeout(&host, READ_WAIT).ok()?;
    tcp.set_read_timeout(Some(READ_WAIT)).ok()?;
    write!(
        tcp,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut seen = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(end) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&seen[..end]).to_ascii_lowercase();
            let len = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())?;
            if seen.len() >= end + 4 + len {
                return String::from_utf8(seen[end + 4..end + 4 + len].to_vec()).ok();
            }
        }
        let read = tcp.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        seen.extend_from_slice(&chunk[..read]);
    }
}

/// A client `WebSocket` (RFC 6455) to one `DevTools` target: masked frames
/// out, whole messages in, pings answered.
struct Socket {
    tcp: TcpStream,
    mask: u32,
}

impl Socket {
    fn open(port: u16, path: &str) -> Self {
        let host = SocketAddr::from(([127, 0, 0, 1], port));
        let mut tcp = TcpStream::connect_timeout(&host, READ_WAIT)
            .unwrap_or_else(|error| panic!("web-smoke: DevTools connect: {error}"));
        tcp.set_read_timeout(Some(READ_WAIT))
            .unwrap_or_else(|error| panic!("web-smoke: DevTools read deadline: {error}"));
        write!(
            tcp,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap_or_else(|error| panic!("web-smoke: DevTools upgrade: {error}"));
        // Byte by byte, so no frame after the head is consumed with it.
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            tcp.read_exact(&mut byte)
                .unwrap_or_else(|error| panic!("web-smoke: DevTools upgrade answer: {error}"));
            head.push(byte[0]);
            assert!(
                head.len() < 8192,
                "web-smoke: DevTools upgrade head too long"
            );
        }
        let head = String::from_utf8_lossy(&head);
        assert!(
            head.starts_with("HTTP/1.1 101"),
            "web-smoke: DevTools refused the upgrade: {head}"
        );
        let seed = std::collections::hash_map::RandomState::new().hash_one(port);
        Self {
            tcp,
            mask: u32::try_from(seed & 0x7FFF_FFFF).unwrap_or(1) | 1,
        }
    }

    fn send(&mut self, opcode: u8, payload: &[u8]) {
        let mut frame = vec![0x80 | opcode];
        let len = payload.len();
        if let Ok(short) = u8::try_from(len)
            && short < 126
        {
            frame.push(0x80 | short);
        } else if let Ok(medium) = u16::try_from(len) {
            // The mask bit, then 126 (0x7E): a 16-bit length follows.
            frame.push(0x80 | 0x7E);
            frame.extend_from_slice(&medium.to_be_bytes());
        } else {
            // The mask bit, then 127 (0x7F): a 64-bit length follows.
            frame.push(0x80 | 0x7F);
            frame.extend_from_slice(&u64::try_from(len).unwrap_or(u64::MAX).to_be_bytes());
        }
        // xorshift32: a fresh masking key per frame, as clients must send.
        self.mask ^= self.mask << 13;
        self.mask ^= self.mask >> 17;
        self.mask ^= self.mask << 5;
        let key = self.mask.to_be_bytes();
        frame.extend_from_slice(&key);
        frame.extend(
            payload
                .iter()
                .zip(key.iter().cycle())
                .map(|(byte, mask)| byte ^ mask),
        );
        self.tcp
            .write_all(&frame)
            .unwrap_or_else(|error| panic!("web-smoke: DevTools write: {error}"));
    }

    fn read_exact(&mut self, len: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; len];
        self.tcp
            .read_exact(&mut bytes)
            .unwrap_or_else(|error| panic!("web-smoke: DevTools read: {error}"));
        bytes
    }

    /// The next whole text message.
    fn message(&mut self) -> String {
        let mut data = Vec::new();
        loop {
            let head = self.read_exact(2);
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0F;
            let masked = head[1] & 0x80 != 0;
            let len = match head[1] & 0x7F {
                126 => {
                    let medium = self.read_exact(2);
                    u64::from(u16::from_be_bytes([medium[0], medium[1]]))
                }
                127 => {
                    let wide = self.read_exact(8);
                    u64::from_be_bytes(wide.try_into().unwrap_or([0; 8]))
                }
                short => u64::from(short),
            };
            assert!(len <= FRAME_MAX, "web-smoke: DevTools frame of {len} bytes");
            let key = if masked {
                self.read_exact(4)
            } else {
                Vec::new()
            };
            let mut payload = self.read_exact(usize::try_from(len).unwrap_or(0));
            if masked {
                for (byte, mask) in payload.iter_mut().zip(key.iter().cycle()) {
                    *byte ^= mask;
                }
            }
            match opcode {
                0x0..=0x2 => {
                    data.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(data)
                            .unwrap_or_else(|error| panic!("web-smoke: DevTools text: {error}"));
                    }
                }
                0x8 => panic!("web-smoke: Chrome closed the DevTools socket"),
                0x9 => self.send(0xA, &payload),
                _ => {}
            }
        }
    }
}

/// One page target driven over `DevTools`: calls wait for their reply, and the
/// events read on the way are recorded or answered at once.
struct Tab {
    socket: Socket,
    next: i64,
    replies: BTreeMap<i64, Value>,
    /// Exceptions, console errors and dialogs since the cell's load.
    problems: Vec<String>,
    /// How many times the plant served its `app.mjs`.
    planted: usize,
    /// The planted fault, when the run has one: `focus`, `walk`, `undo` or
    /// `tab`.
    fault: Option<&'static str>,
}

impl Tab {
    fn try_call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        let request = Value::obj([
            ("id", Value::Num(id)),
            ("method", Value::str(method)),
            ("params", params),
        ]);
        self.socket.send(0x1, request.render().as_bytes());
        loop {
            if let Some(reply) = self.replies.remove(&id) {
                if let Some(error) = reply.get("error") {
                    return Err(format!("{method}: {}", error.render()));
                }
                return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
            }
            let text = self.socket.message();
            let message = json::parse(&text)
                .unwrap_or_else(|error| panic!("web-smoke: DevTools JSON: {error}"));
            if let Some(Value::Num(got)) = message.get("id") {
                self.replies.insert(*got, message);
            } else {
                self.event(&message);
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.try_call(method, params)
            .unwrap_or_else(|error| panic!("web-smoke: DevTools {error}"))
    }

    fn event(&mut self, message: &Value) {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match message.get_str("method") {
            Some("Fetch.requestPaused") => self.plant(&params),
            Some("Runtime.exceptionThrown") => {
                let details = params
                    .get("exceptionDetails")
                    .cloned()
                    .unwrap_or(Value::Null);
                let thrown = details
                    .get("exception")
                    .and_then(|exception| exception.get_str("description"))
                    .unwrap_or_default()
                    .to_owned();
                let text = details.get_str("text").unwrap_or_default();
                self.problems.push(format!("exception: {text} {thrown}"));
            }
            Some("Log.entryAdded") => {
                let entry = params.get("entry").cloned().unwrap_or(Value::Null);
                let url = entry.get_str("url").unwrap_or_default();
                if entry.get_str("level") == Some("error") {
                    self.problems.push(format!(
                        "console {}: {} {url}",
                        entry.get_str("source").unwrap_or_default(),
                        entry.get_str("text").unwrap_or_default(),
                    ));
                }
            }
            Some("Runtime.consoleAPICalled")
                if matches!(params.get_str("type"), Some("error" | "assert")) =>
            {
                self.problems.push(format!(
                    "console.error: {}",
                    params.get("args").map(Value::render).unwrap_or_default()
                ));
            }
            Some("Page.javascriptDialogOpening") => {
                self.problems.push(format!(
                    "dialog: {}",
                    params.get_str("message").unwrap_or_default()
                ));
                drop(self.try_call(
                    "Page.handleJavaScriptDialog",
                    Value::obj([("accept", Value::Bool(false))]),
                ));
            }
            _ => {}
        }
    }

    /// The planted fault: `app.mjs` served with its topic row's or side panel
    /// row's focus guard removed, with the walk's Keep sending the Ask line, or with
    /// an owner line's Undo leaving its countdown running; every other byte and
    /// header as the listener sent them.
    fn plant(&mut self, params: &Value) {
        let id = params.get_str("requestId").unwrap_or_default().to_owned();
        let body = self.call(
            "Fetch.getResponseBody",
            Value::obj([("requestId", Value::str(id.as_str()))]),
        );
        let raw = body.get_str("body").unwrap_or_default();
        let text = if body.get("base64Encoded") == Some(&Value::Bool(true)) {
            String::from_utf8(base64_decode(raw))
                .unwrap_or_else(|error| panic!("web-smoke plant: app.mjs text: {error}"))
        } else {
            raw.to_owned()
        };
        let (guard, planted, what) = match self.fault {
            Some("walk") => (WALK_GUARD, WALK_PLANTED, "walk Keep buttons"),
            Some("undo") => (UNDO_GUARD, UNDO_PLANTED, "owner line Undo cancels"),
            Some("tab") => (TAB_GUARD, TAB_PLANTED, "side panel row focus guards"),
            _ => (ROW_GUARD, ROW_PLANTED, "topic row focus guards"),
        };
        let sites = text.matches(guard).count();
        assert_eq!(
            sites, 1,
            "web-smoke plant did not apply: {sites} {what} in app.mjs, want 1"
        );
        let served = text.replacen(guard, planted, 1);
        let headers = match params.get("responseHeaders") {
            Some(Value::Arr(headers)) => headers
                .iter()
                .filter(|header| {
                    !header
                        .get_str("name")
                        .is_some_and(|name| name.eq_ignore_ascii_case("content-length"))
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        self.call(
            "Fetch.fulfillRequest",
            Value::obj([
                ("requestId", Value::str(id.as_str())),
                (
                    "responseCode",
                    params
                        .get("responseStatusCode")
                        .cloned()
                        .unwrap_or(Value::Num(200)),
                ),
                ("responseHeaders", Value::Arr(headers)),
                ("body", Value::str(base64_encode(served.as_bytes()))),
            ]),
        );
        self.planted += 1;
        if self.fault == Some("undo") {
            println!(
                "web-smoke plant: /assets/app.mjs served without the owner line's stopped \
                 countdown (Undo hides the line and the countdown still sends it, 1 site); \
                 s3a-undo must fail"
            );
        } else if self.fault == Some("tab") {
            println!(
                "web-smoke plant: /assets/app.mjs served without the side panel row's \
                 mousedown preventDefault (1 site); tab-focus-kept must fail"
            );
        } else if self.fault == Some("walk") {
            println!(
                "web-smoke plant: /assets/app.mjs served without the walk's own Keep line (Keep \
                 sends the Ask line, 1 site); walk-keep must fail"
            );
        } else {
            println!(
                "web-smoke plant: /assets/app.mjs served without the topic row's mousedown \
                 preventDefault (1 site); focus-kept must fail"
            );
        }
    }

    fn prepare(&mut self, plant: bool) {
        for domain in ["Page.enable", "Runtime.enable", "Log.enable"] {
            self.call(domain, Value::Obj(Vec::new()));
        }
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            Value::obj([("source", Value::str(WATCH))]),
        );
        if plant {
            self.call(
                "Fetch.enable",
                Value::obj([(
                    "patterns",
                    Value::Arr(vec![Value::obj([
                        ("urlPattern", Value::str("*/assets/app.mjs")),
                        ("requestStage", Value::str("Response")),
                    ])]),
                )]),
            );
        }
    }

    fn view(&mut self, cell: Cell) {
        self.call(
            "Emulation.setDeviceMetricsOverride",
            Value::obj([
                ("width", Value::Num(cell.width)),
                ("height", Value::Num(cell.height)),
                ("deviceScaleFactor", Value::Num(1)),
                ("mobile", Value::Bool(!cell.wide())),
            ]),
        );
        self.call(
            "Emulation.setEmulatedMedia",
            Value::obj([(
                "features",
                Value::Arr(vec![Value::obj([
                    ("name", Value::str("prefers-color-scheme")),
                    ("value", Value::str(cell.scheme())),
                ])]),
            )]),
        );
    }

    fn eval(&mut self, script: &str) -> Result<Value, String> {
        let reply = self.try_call(
            "Runtime.evaluate",
            Value::obj([
                ("expression", Value::str(script)),
                ("returnByValue", Value::Bool(true)),
                ("awaitPromise", Value::Bool(true)),
            ]),
        )?;
        if let Some(details) = reply.get("exceptionDetails") {
            return Err(details.render());
        }
        Ok(reply
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Every event Chrome sent before this point has been read.
    fn flush(&mut self) {
        drop(self.eval("0"));
    }

    /// Run `body` once with `want` bound: its `{ ok, saw }` decides.
    fn verdict(&mut self, check: &'static str, body: &str, want: &str) -> Result<Value, Failed> {
        let script = format!(
            "(async () => {{ const want = {}; {PRELUDE} return await (async () => {{ {body} }})(); }})()",
            Value::str(want).render()
        );
        match self.eval(&script) {
            Ok(seen) if seen.get("ok") == Some(&Value::Bool(true)) => Ok(seen),
            Ok(seen) => Err(Failed::new(
                check,
                seen.get("saw").map_or_else(|| seen.render(), Value::render),
            )),
            Err(why) => Err(Failed::new(check, why)),
        }
    }

    fn holds(&mut self, check: &'static str, body: &str, want: &str) -> Checked {
        self.verdict(check, body, want).map(drop)
    }

    /// Poll `body` until it holds or `within` has passed.
    fn until(
        &mut self,
        check: &'static str,
        body: &str,
        want: &str,
        within: Duration,
    ) -> Result<Value, Failed> {
        let deadline = Instant::now() + within;
        loop {
            match self.verdict(check, body, want) {
                Ok(seen) => return Ok(seen),
                Err(failed) if Instant::now() >= deadline => {
                    return Err(Failed::new(
                        check,
                        format!("not within {within:?}: {}", failed.detail),
                    ));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// A real left click at the centre of `target` (see [`SPOT`]): pointer
    /// move, press and release, after proving the press lands on it.
    fn click(&mut self, check: &'static str, target: &str) -> Checked {
        let spot = self.verdict(check, SPOT, target)?;
        self.press(check, &spot)
    }

    /// A real left click on the element `selector` selects (see
    /// [`SPOT_SELECTOR`]): its centre, or 4 px inside the top-left corner of an
    /// overlay, after proving the press lands on it.
    fn click_selector(&mut self, check: &'static str, selector: &str) -> Checked {
        let spot = self.verdict(check, SPOT_SELECTOR, selector)?;
        self.press(check, &spot)
    }

    /// Pointer move, press and release at the `x` and `y` of `spot`.
    fn press(&mut self, check: &'static str, spot: &Value) -> Checked {
        let (Some(Value::Num(x)), Some(Value::Num(y))) = (spot.get("x"), spot.get("y")) else {
            return Err(Failed::new(check, format!("no spot: {}", spot.render())));
        };
        let (x, y) = (*x, *y);
        for (kind, button, buttons) in [
            ("mouseMoved", "none", 0),
            ("mousePressed", "left", 1),
            ("mouseReleased", "left", 0),
        ] {
            self.call(
                "Input.dispatchMouseEvent",
                Value::obj([
                    ("type", Value::str(kind)),
                    ("x", Value::Num(x)),
                    ("y", Value::Num(y)),
                    ("button", Value::str(button)),
                    ("buttons", Value::Num(buttons)),
                    ("clickCount", Value::Num(i64::from(kind != "mouseMoved"))),
                ]),
            );
        }
        Ok(())
    }

    /// The cell as it looks now, kept under [`OUT`] when the recipe names it.
    /// A PNG that is not one, or that could not be written, is the error.
    fn shot(&mut self, out: Option<&Path>, cell: Cell, failed: bool) -> Result<(), String> {
        let Some(dir) = out else { return Ok(()) };
        let suffix = if failed { "-FAILED" } else { "" };
        let path = dir.join(format!("{}{suffix}.png", cell.file()));
        let shot = self.try_call(
            "Page.captureScreenshot",
            Value::obj([("format", Value::str("png"))]),
        )?;
        let png = base64_decode(shot.get_str("data").unwrap_or_default());
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(format!("{}: Chrome sent no PNG", path.display()));
        }
        std::fs::write(&path, &png).map_err(|error| format!("{}: {error}", path.display()))?;
        println!("web-smoke shot: {} ({} bytes)", path.display(), png.len());
        Ok(())
    }

    /// Ask Chrome to quit; its reply may never come, as the socket closes.
    fn close(&mut self) {
        let request = Value::obj([
            ("id", Value::Num(self.next)),
            ("method", Value::str("Browser.close")),
            ("params", Value::Obj(Vec::new())),
        ]);
        self.socket.send(0x1, request.render().as_bytes());
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let one = chunk[0];
        let two = chunk.get(1).copied().unwrap_or(0);
        let three = chunk.get(2).copied().unwrap_or(0);
        let sixes = [
            one >> 2,
            ((one & 0x03) << 4) | (two >> 4),
            ((two & 0x0F) << 2) | (three >> 6),
            three & 0x3F,
        ];
        for (at, six) in sixes.iter().enumerate() {
            if at <= chunk.len() {
                out.push(char::from(BASE64[usize::from(*six)]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0_u32;
    let mut bits = 0_u32;
    for byte in text.bytes() {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' => continue,
            other => panic!("web-smoke: not base64: byte {other}"),
        };
        acc = ((acc << 6) | u32::from(six)) & 0xFFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).unwrap_or(0));
        }
    }
    out
}

#[path = "web_walk_browser_spec.rs"]
mod web_walk_browser_spec;

#[path = "web_guard_browser.rs"]
mod web_guard_browser;

#[allow(
    clippy::expect_used,
    clippy::items_after_statements,
    clippy::manual_string_new,
    clippy::too_many_lines,
    reason = "the frozen spec's case tables and one long check keep the bytes nav froze"
)]
#[path = "web_compose_browser_spec.rs"]
mod web_compose_browser_spec;

#[allow(
    clippy::expect_used,
    clippy::format_push_string,
    clippy::items_after_statements,
    clippy::too_many_lines,
    reason = "the frozen spec's case tables and long checks keep the bytes nav froze"
)]
#[path = "web_topic_tabs_browser_spec.rs"]
mod web_topic_tabs_browser_spec;

#[allow(
    clippy::expect_used,
    reason = "the frozen spec's fixture writes keep the bytes nav froze"
)]
#[path = "web_topic_lifecycle_browser_spec.rs"]
mod web_topic_lifecycle_browser_spec;

#[path = "web_signedout_browser.rs"]
mod web_signedout_browser;

#[path = "web_side_panel_browser_spec.rs"]
mod web_side_panel_browser_spec;

#[path = "web_p1_browser_spec.rs"]
mod web_p1_browser_spec;
