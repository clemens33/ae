//! Frozen banked-resets acceptance r4: the 48 h notice reaches the live lead
//! through the real watchdog quota cadence, once, and survives a restart.
//!
//! INSTALL (driver, byte-identical): copy to `tests/it/banked_runtime_spec.rs` and
//! add `mod banked_runtime_spec;` to `tests/it/main.rs` before `mod banked_spec;`.
//! Additive to r3 (`b35d929751b3501bbe7756ee8db5f98295183418`); no r3 pin moves.
//! Oracle: `.local/brief-banked.md` ruling 4 ("once per declared reset, when it
//! expires within 48 h, the session lead gets one notice (an event plus the
//! existing notice path); never repeated for the same date"), the r3 seam rules
//! (`resetexpiry` by `watchdog` is the receipt) and the driver's runtime statement
//! in ae-20261127T095932Z-b0446775 (per-pane quiet check, at-most-once claim).
//!
//! Existing API only: on the base commit this file COMPILES. The two delivery
//! tests fail at their first wait for a `resetexpiry` record; the undated guard
//! holds on base and must keep holding.
//!
//! r6: the second test pins the shared send gate as it is. A draft in the
//! lead's composer refuses delivery and claims no receipt. An empty composer
//! mid-turn is admitted, as for every other watchdog notice. r4 and r5 pinned
//! a busy refusal that the existing notice path never promised.
//!
//! Real parts: a private tmux server ([`Rig`]), the lead's main pane running
//! the frame-drawing fake claude started through `_run`, and a real stepped
//! `_watchdog-run` with quota on. The declared scope is a client-only Claude
//! account whose `.claude.json` proves its source. Nothing decides the notice
//! but the product.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures write a scratch config, a scratch quota cache and read the journal"
)]

use std::time::Duration;

use ae::events::Event;
use ae::time::Timestamp;

use super::seat_relaunch::Rig;
use super::watchdog_step::Stepper;

/// Longer than one stepped cycle that holds a 30 s defer twice.
const WAIT: Duration = Duration::from_secs(90);

/// The declared account's config home, under the rig's scratch root.
const ACCOUNT: &str = "banked-account";

/// An expiry `ahead` seconds from now, at minute precision with an explicit
/// `+00:00` offset, rounded down so it never lands later than asked.
fn expiry_in(ahead: i64) -> String {
    let stamp = Timestamp::from_epoch(Timestamp::now().epoch() + ahead).to_string();
    format!("{}+00:00", &stamp[..16])
}

/// Write the rig config with one client-only Claude account `cc` declaring
/// `declaration`, ahead of the rig's own sections. Rewritten in place, so a
/// running watchdog reads the new declaration on its next quota cycle.
fn declare(rig: &Rig, declaration: &str) {
    let path = rig.scratch.join("config");
    let held = rig.scratch.join("config.rig");
    if !held.exists() {
        std::fs::copy(&path, &held).expect("keep the rig's own config");
    }
    let base = std::fs::read_to_string(&held).expect("rig config");
    std::fs::write(
        &path,
        format!(
            "[clients]\ncc = claude config_home={} manual_resets={declaration}\n{base}",
            rig.scratch.join(ACCOUNT).display()
        ),
    )
    .expect("declared config");
}

/// A fresh, calm weekly window for the declared account: its source exists,
/// and nothing here is low enough to book any other quota notice.
fn account_cache(rig: &Rig) {
    let now = Timestamp::now().epoch();
    let dir = rig.scratch.join(ACCOUNT);
    std::fs::create_dir_all(&dir).expect("scratch account");
    std::fs::write(
        dir.join(".claude.json"),
        format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":10.0,\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
            now * 1_000,
            Timestamp::from_epoch(now + 5 * 86_400),
        ),
    )
    .expect("scratch quota cache");
}

fn rebind(rig: &Rig, key: &str, value: &str) {
    let path = rig.dir.join("meta");
    let text = std::fs::read_to_string(&path).expect("fixture meta");
    let prefix = format!("{key}=");
    assert!(text.lines().any(|line| line.starts_with(&prefix)), "{key}");
    let text: String = text
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                format!("{prefix}{value}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(path, text).expect("rebound fixture row");
}

/// The lead's main pane as a real live seat: the frame-drawing fake claude,
/// started through `_run` with scratch identity, idle and composed.
fn live_lead(rig: &Rig) {
    rebind(rig, "profile.main", "fake-claude");
    rebind(rig, "agent_bin.main", "claude");
    let home = rig.scratch.join("home");
    std::fs::create_dir_all(&home).expect("scratch home");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            &rig.main_pane,
            "env",
            "-u",
            "CLAUDE_CONFIG_DIR",
            "-u",
            "CODEX_HOME",
            &format!("HOME={}", home.display()),
            &format!("AE_HOME={}", rig.scratch.display()),
            &format!("CONFIG_FILE={}", rig.scratch.join("config").display()),
            "/bin/sh",
        ])
        .0,
        "scratch shell environment for the lead"
    );
    rig.start(&rig.main_pane, "main", "claude");
}

/// A stepped watchdog and its stepper; the daemon goes first on drop.
struct Watch {
    _daemon: super::cli::OwnedChild,
    stepper: Stepper,
}

fn watch(rig: &Rig) -> Watch {
    static STARTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let start = STARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let channel = format!("bk-step-{start}");
    let trace = rig.scratch.join("banked-quota-trace");
    std::fs::write(&trace, "").expect("fresh per-start observation barrier");
    let send = rig.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send).expect("fixture send link");
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rig.scratch.join("banked-daemon-err"))
        .expect("scratch daemon log");
    let daemon = super::cli::ae()
        .arg("_watchdog-run")
        .arg(&rig.dir)
        .args(["--interval", "1", "--stale-secs", "999999"])
        .args(["--tg-supervise-secs", "0", "--quota-every-secs", "1"])
        .args(["--step-channel", &channel])
        .env("HOME", rig.scratch.join("home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", rig.scratch.join("config"))
        .env("AE_TEST_QUOTA_TRACE", trace)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_SENDER_OVERRIDE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .expect("real stepped watchdog");
    let mut stepper = Stepper::new(&rig.sock, &channel);
    stepper.ready();
    Watch {
        _daemon: daemon,
        stepper,
    }
}

fn diagnostics(rig: &Rig) -> String {
    format!(
        "journal:\n{}\ndaemon:\n{}\nreceived:\n{}",
        rig.events(),
        std::fs::read_to_string(rig.scratch.join("banked-daemon-err")).unwrap_or_default(),
        rig.received(),
    )
}

/// Completed quota observations of THIS daemon start.
fn observed(rig: &Rig) -> usize {
    std::fs::read_to_string(rig.scratch.join("banked-quota-trace"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("observed "))
        .count()
}

/// Step until two more quota observations completed, so an absence cannot
/// outrun the cycle that would have booked the notice.
fn sweeps(rig: &Rig, watch: &mut Watch) {
    let before = observed(rig);
    let stepped = watch
        .stepper
        .step_until_within(WAIT, || observed(rig) >= before + 2);
    assert!(
        stepped.is_some(),
        "no two quota observations: {}",
        diagnostics(rig)
    );
}

fn until(rig: &Rig, watch: &mut Watch, what: &str, ready: impl Fn() -> bool) {
    let stepped = watch.stepper.step_until_within(WAIT, ready);
    assert!(stepped.is_some(), "{what}: {}", diagnostics(rig));
}

/// Every `resetexpiry` record in the journal.
fn receipts(rig: &Rig) -> Vec<Event> {
    rig.events()
        .lines()
        .filter_map(|line| Event::parse_line(line).ok())
        .filter(|event| event.action == "resetexpiry")
        .collect()
}

/// How often the lead's pane was typed the words of a banked-reset notice.
/// A later count is compared with the count right after the first delivery,
/// so the notice may name the words more than once.
fn typed(rig: &Rig) -> usize {
    rig.received()
        .to_lowercase()
        .matches("banked reset")
        .count()
}

fn assert_one_receipt_to_the_lead(rig: &Rig) {
    let found = receipts(rig);
    assert_eq!(found.len(), 1, "exactly one receipt: {}", diagnostics(rig));
    let event = &found[0];
    assert_eq!(event.actor, "watchdog", "{event:?}");
    assert_eq!(event.target.as_deref(), Some("lead"), "{event:?}");
    assert!(
        event
            .reference
            .as_deref()
            .is_some_and(|reference| !reference.is_empty()),
        "the receipt carries its reference: {event:?}"
    );
    assert!(
        event
            .summary
            .as_deref()
            .is_some_and(|summary| summary.to_lowercase().contains("banked reset")),
        "the journal carries the notice text: {event:?}"
    );
}

#[test]
fn banked_runtime_the_live_lead_hears_a_due_reset_once_and_not_again_after_a_restart() {
    let rig = Rig::new("bkonce");
    account_cache(&rig);
    // 72 h ahead: declared, not due. The running cadence books nothing.
    declare(&rig, &expiry_in(72 * 3_600));
    live_lead(&rig);
    let mut daemon = watch(&rig);
    sweeps(&rig, &mut daemon);
    assert!(
        receipts(&rig).is_empty(),
        "not due yet: {}",
        diagnostics(&rig)
    );
    assert_eq!(typed(&rig), 0, "not due yet: {}", diagnostics(&rig));

    // 24 h ahead: due. The same running daemon reads the edited declaration.
    declare(&rig, &expiry_in(24 * 3_600));
    until(&rig, &mut daemon, "the due reset reaches the lead", || {
        !receipts(&rig).is_empty() && typed(&rig) > 0
    });
    assert_one_receipt_to_the_lead(&rig);
    let once = typed(&rig);

    // Later cycles of the same daemon: never repeated.
    sweeps(&rig, &mut daemon);
    assert_one_receipt_to_the_lead(&rig);
    assert_eq!(typed(&rig), once, "no repeat: {}", diagnostics(&rig));

    // A restarted daemon holds no state: the journal receipt alone stops it.
    drop(daemon);
    let mut restarted = watch(&rig);
    sweeps(&rig, &mut restarted);
    assert_one_receipt_to_the_lead(&rig);
    assert_eq!(
        typed(&rig),
        once,
        "no repeat after a restart: {}",
        diagnostics(&rig)
    );
}

#[test]
fn banked_runtime_a_drafting_lead_gets_no_receipt_until_its_composer_is_clear_then_exactly_one() {
    // The shared send gate refuses a pane whose composer holds a draft; it
    // admits an empty composer even mid-turn (r6: the notice rides that gate,
    // brief ruling 4 "the existing notice path", and adds no gate of its own).
    let rig = Rig::new("bkdraft");
    account_cache(&rig);
    declare(&rig, &expiry_in(24 * 3_600));
    live_lead(&rig);
    rig.mark_draft(&rig.main_pane);
    let mut daemon = watch(&rig);
    sweeps(&rig, &mut daemon);
    assert!(
        receipts(&rig).is_empty(),
        "a refused delivery claims nothing, so a later cycle can still tell: {}",
        diagnostics(&rig)
    );
    assert_eq!(
        typed(&rig),
        0,
        "nothing typed over a draft: {}",
        diagnostics(&rig)
    );

    std::fs::remove_file(rig.scratch.join("__DRAFT__")).expect("the draft is cleared");
    until(&rig, &mut daemon, "the clear lead hears it", || {
        !receipts(&rig).is_empty() && typed(&rig) > 0
    });
    let once = typed(&rig);
    sweeps(&rig, &mut daemon);
    assert_one_receipt_to_the_lead(&rig);
    assert_eq!(typed(&rig), once, "no repeat: {}", diagnostics(&rig));
}

#[test]
fn banked_runtime_an_undated_count_never_books_a_notice() {
    let rig = Rig::new("bkundated");
    account_cache(&rig);
    declare(&rig, "2");
    live_lead(&rig);
    let mut daemon = watch(&rig);
    sweeps(&rig, &mut daemon);
    sweeps(&rig, &mut daemon);
    assert!(receipts(&rig).is_empty(), "{}", diagnostics(&rig));
    assert_eq!(typed(&rig), 0, "{}", diagnostics(&rig));
}
