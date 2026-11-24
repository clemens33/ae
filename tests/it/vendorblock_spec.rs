//! FROZEN SPEC (vendorblock S1+S2). Install byte-identically.
//!
//! Lead rulings (2026-11-24): attention stays `limit` at rank 3. The kind is
//! the list cell and the journal summary. The block clears only on
//! `ae quota clear client-<label>` or `ae quota clear provider-<provider>`. A latch release does not clear it.
//!
//! RED on `b93155785`: the new rows are unclassified and no scope prints
//! `blocked (`. GREEN guards name a behaviour that already holds.
//!
//! Do not add a field to [`ae::watchdog_daemon::Observation`]. This file
//! builds that struct by its fields. The kind rides in `limit_notice` and in
//! `Effect::BlockScope`. [`ae::digest::AgentEntry`] may grow a field only if
//! it stays [`Default`], because the cell test fills the rest with `..`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the fixture writes a scratch state root and reads captures"
)]

use ae::watchdog::Throttle;
use ae::watchdog_daemon::{Effect, Knobs, Observation, PaneState, Verdict, account};

const MUSE_402: &str = include_str!("../fixtures/vendorblock/muse-402.reconstructed.txt");
const MUSE_HEALTHY: &str = include_str!("../fixtures/vendorblock/muse-healthy-hi.raw.txt");
const OPENCODE_140: &str = include_str!("../fixtures/vendorblock/opencode-copilot-quota.raw.txt");
const OPENCODE_100: &str =
    include_str!("../fixtures/vendorblock/opencode-copilot-quota-100x30.raw.txt");
const OPENCODE_TWO: &str =
    include_str!("../fixtures/vendorblock/opencode-copilot-quota-two-turns.raw.txt");
const OPENCODE_TOAST: &str =
    include_str!("../fixtures/vendorblock/opencode-copilot-quota.toast.raw.txt");

const PHRASE: &str = "You have exceeded your monthly quota";
const REQUEST_ID: &str = "00000000-0000-4000-8000-000000000000";
const SINCE: i64 = 1_788_858_600;
const SINCE_ISO: &str = "2026-09-08T09:10:00Z";

fn class(frame: &str, bin: &str) -> Option<Throttle> {
    ae::watchdog::throttle_class(frame, bin)
}

fn notice(frame: &str, bin: &str) -> Option<String> {
    ae::watchdog::limit_notice(frame, bin)
}

fn emits(effects: &[Effect]) -> Vec<(&str, &str)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { action, summary } => Some((*action, summary.as_str())),
            _ => None,
        })
        .collect()
}

fn idle() -> Observation {
    Observation {
        now_epoch: 10_000,
        hash: 7,
        harness: ae::watchdog_daemon::HarnessObservation {
            frame: ae::harness_state::HarnessState::Unknown,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: None,
        ended_wait: None,
        done_progress: ae::watchdog::DoneProgress::None,
        wait_progress: ae::watchdog::WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: ae::procs::Descendancy::Present,
        launch_attempt: ae::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 0,
        declared_age_secs: 0,
        suspended: ae::watchdog::Suspensions::default(),
        sweep: None,
        own_work: ae::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}

fn booked(frame: &str, bin: &str) -> ae::watchdog_daemon::Accounting {
    let mut seen = idle();
    seen.throttle = class(frame, bin);
    seen.limit_notice = notice(frame, bin);
    account(&PaneState::default(), &seen, &Knobs::default())
}

#[test]
fn the_reconstructed_muse_402_is_a_billing_block() {
    assert_eq!(
        class(MUSE_402, "muse"),
        Some(Throttle::LimitReached),
        "a 402 is the limit class, kind billing"
    );
    let quoted = notice(MUSE_402, "muse");
    let text = quoted.as_deref().unwrap_or("");
    assert!(
        text.contains("Billing verification failed"),
        "the line's reason: {quoted:?}"
    );
    assert!(
        text.contains("Billing Issue"),
        "the reconstructed tail: {quoted:?}"
    );
    assert!(!text.contains("request_id"), "scrub the id: {quoted:?}");
    assert!(
        !text.contains(REQUEST_ID),
        "scrub the placeholder: {quoted:?}"
    );
}

#[test]
fn the_measured_opencode_quota_rows_are_an_account_block() {
    let renamed = OPENCODE_100.replace("     Build · GPT-6.1 Sol ·", "     Plan · GPT-6.1 Sol ·");
    for (name, frame) in [
        ("140 columns, sidebar", OPENCODE_140),
        ("100 columns, row ends", OPENCODE_100),
        ("two turns, the newest row", OPENCODE_TWO),
        (
            "another agent name on the same footer grammar",
            renamed.as_str(),
        ),
    ] {
        assert_eq!(
            class(frame, "opencode"),
            Some(Throttle::LimitReached),
            "{name}"
        );
        let quoted = notice(frame, "opencode");
        assert!(
            quoted.as_deref().is_some_and(|text| text.contains(PHRASE)),
            "{name}: {quoted:?}"
        );
    }
}

/// GREEN on main. A match that is wider than the measured row turns this red.
#[test]
fn an_unmeasured_or_stale_row_stays_unclassified() {
    let indented = MUSE_402.replace("\n◆ model failed:", "\n ◆ model failed:");
    let quoted = format!(
        "  the pane said:\n{}",
        MUSE_402
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let five_hundred = MUSE_402.replace("API error 402", "API error 500");
    let four_spaces = OPENCODE_100.replace("     Error:", "    Error:");
    let glued = OPENCODE_100.replace("monthly quota", "monthly quota.");
    let seconds = OPENCODE_100.replace("692ms", "2s").replace("334ms", "3s");
    let later_row = OPENCODE_100.replace(
        "\n     Error: You have exceeded your monthly quota\n\n     Build · GPT-6.1 Sol · 334ms\n",
        "\n",
    );
    let toast_only = OPENCODE_TOAST
        .lines()
        .filter(|line| !line.starts_with("     Error:"))
        .collect::<Vec<_>>()
        .join("\n");
    let shifted = shift_sidebar(OPENCODE_140, -1);
    let shifted_right = shift_sidebar(OPENCODE_140, 1);
    for (name, frame, bin) in [
        ("healthy muse", MUSE_HEALTHY, "muse"),
        ("indented 402", indented.as_str(), "muse"),
        ("quoted 402", quoted.as_str(), "muse"),
        ("api 500", five_hundred.as_str(), "muse"),
        ("four spaces", four_spaces.as_str(), "opencode"),
        ("glued quota.", glued.as_str(), "opencode"),
        ("seconds footer", seconds.as_str(), "opencode"),
        ("a later user row", later_row.as_str(), "opencode"),
        (
            "toast without the transcript row",
            toast_only.as_str(),
            "opencode",
        ),
        ("sidebar one column early", shifted.as_str(), "opencode"),
        (
            "sidebar one column late",
            shifted_right.as_str(),
            "opencode",
        ),
        (
            "opencode availability",
            "   ⬝⬝⬝ Endpoint is unavailable [retrying in 5s attempt #1]",
            "opencode",
        ),
    ] {
        assert_eq!(class(frame, bin), None, "{name}");
        assert_eq!(notice(frame, bin), None, "{name}");
    }
}

/// Lead ruling: a pane-height window must not book a quote. Another indent
/// is the `four spaces` case above. These two are the other pins: the
/// sentence inside tool output, and the measured row followed by a later
/// assistant row. The wide measured row still books; its extra text is the
/// sidebar at `L + 4`, not a quote.
#[test]
fn a_quote_or_a_later_assistant_row_is_not_an_account_block() {
    let tool_output = OPENCODE_100.replace(
        "     Error: You have exceeded your monthly quota",
        "     stdout: Error: You have exceeded your monthly quota",
    );
    let later_assistant = OPENCODE_100.replace(
        "\n     Error: You have exceeded your monthly quota\n\n     Build · GPT-6.1 Sol · 334ms\n",
        "\n     Exit code 0.\n",
    );
    assert!(
        later_assistant.contains("     Error: You have exceeded your monthly quota"),
        "the first measured row remains"
    );
    assert!(
        later_assistant.contains("     Exit code 0."),
        "an assistant row follows it"
    );
    assert!(
        !tool_output.contains("\n     Error: You have exceeded your monthly quota\n"),
        "tool output is a longer row, not the measured one"
    );
    for (name, frame) in [
        ("inside tool output", tool_output.as_str()),
        ("a later assistant row", later_assistant.as_str()),
    ] {
        assert_eq!(class(frame, "opencode"), None, "{name}");
        assert_eq!(notice(frame, "opencode"), None, "{name}");
    }
}

/// Lead ruling: at a width with no measured sidebar column, only the end of
/// the row counts. The 100-column capture is that width. Widths 110 and 120
/// are the other measured ones and have no capture here.
#[test]
fn a_width_with_no_sidebar_column_rejects_text_after_the_phrase() {
    let phrase = "     Error: You have exceeded your monthly quota";
    let continued = OPENCODE_100.replace(phrase, &format!("{phrase}    • not a measured sidebar"));
    assert!(
        continued.contains("    • not a measured sidebar"),
        "the row no longer ends at the phrase"
    );
    assert_eq!(class(&continued, "opencode"), None);
    assert_eq!(notice(&continued, "opencode"), None);
}

/// The measured 140-column sidebar starts at `L + 4`. Shifting it is the
/// negative above; this pins the fixture the positive uses.
#[test]
fn the_measured_sidebar_starts_four_columns_past_the_rule() {
    let rule = OPENCODE_140
        .lines()
        .find(|line| line.contains('╹'))
        .expect("the composer rule");
    let error = OPENCODE_140
        .lines()
        .find(|line| line.contains(PHRASE))
        .expect("the error row");
    let phrase = format!("     Error: {PHRASE}");
    let rest = error
        .strip_prefix(&phrase)
        .expect("the row starts with the phrase");
    let gap = rest.chars().take_while(|ch| *ch == ' ').count();
    let column = phrase.chars().count() + gap;
    let length = rule.trim_end().chars().count();
    assert_eq!(length, 96, "the 140-column rule row");
    assert_eq!(column, 100, "sidebar column is L + 4");
    assert_eq!(column, length + 4);
}

/// A continuation with no composer rule row is not a measured sidebar.
#[test]
fn a_sidebar_continuation_with_no_rule_row_does_not_book() {
    let bare = OPENCODE_140
        .lines()
        .filter(|line| !line.contains('╹'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(bare.contains(PHRASE), "the error row remains");
    assert!(!bare.contains('╹'), "the rule row is gone");
    assert_eq!(class(&bare, "opencode"), None);
    assert_eq!(notice(&bare, "opencode"), None);
}

fn shift_sidebar(frame: &str, delta: isize) -> String {
    let phrase = format!("     Error: {PHRASE}");
    frame
        .lines()
        .map(|line| {
            let Some(rest) = line.strip_prefix(&phrase) else {
                return line.to_owned();
            };
            let gap = rest.chars().take_while(|ch| *ch == ' ').count();
            let sidebar = rest.trim_start();
            if sidebar.is_empty() {
                return line.to_owned();
            }
            let gap = usize::try_from(
                isize::try_from(gap)
                    .unwrap_or(0)
                    .saturating_add(delta)
                    .max(0),
            )
            .unwrap_or(0);
            format!("{phrase}{}{sidebar}", " ".repeat(gap))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_first_sight_journals_the_kind_once_and_a_dead_pane_still_wins() {
    for (frame, bin, prefix, reason) in [
        (
            MUSE_402,
            "muse",
            "vendor billing blocked",
            "Billing verification failed",
        ),
        (OPENCODE_100, "opencode", "vendor account blocked", PHRASE),
    ] {
        let first = booked(frame, bin);
        let emitted = emits(&first.effects);
        assert_eq!(emitted.len(), 1, "{bin}: {:?}", first.effects);
        assert_eq!(emitted[0].0, "limit", "{bin}");
        assert!(emitted[0].1.starts_with(prefix), "{bin}: {}", emitted[0].1);
        assert!(emitted[0].1.contains(reason), "{bin}: {}", emitted[0].1);
        assert!(!emitted[0].1.contains(REQUEST_ID), "{bin}");
        let debug = format!("{:?}", first.effects);
        assert!(debug.contains("BlockScope"), "{bin}: {debug}");
        assert!(!debug.contains("LimitWake"), "{bin}: a block has no reset");
        assert_eq!(first.verdict, Verdict::Limit, "{bin}");
        assert_eq!(first.verdict.reason(), "limit", "{bin}");
        assert_eq!(first.next.limit_since, Some(10_000), "{bin}");
        let mut seen = idle();
        seen.throttle = class(frame, bin);
        seen.limit_notice = notice(frame, bin);
        let second = account(&first.next, &seen, &Knobs::default());
        assert!(
            emits(&second.effects).is_empty(),
            "{bin} second cycle: {:?}",
            second.effects
        );
    }
}

/// GREEN on main. A block must not rewrite a usage-limit summary, and a dead
/// pane is never a block episode.
#[test]
fn a_usage_limit_summary_and_a_dead_pane_stay_what_they_are() {
    let mut seen = idle();
    seen.throttle = Some(Throttle::LimitReached);
    seen.limit_notice = Some("You've hit your session limit".to_owned());
    let booked = account(&PaneState::default(), &seen, &Knobs::default());
    let emitted = emits(&booked.effects);
    assert_eq!(emitted.len(), 1, "{:?}", booked.effects);
    assert!(
        emitted[0].1.starts_with("vendor usage limit reached"),
        "{}",
        emitted[0].1
    );
    let mut dead = idle();
    dead.is_dead = true;
    dead.throttle = class(MUSE_402, "muse");
    dead.limit_notice = notice(MUSE_402, "muse");
    let dead = account(&PaneState::default(), &dead, &Knobs::default());
    assert_eq!(dead.verdict, Verdict::Dead);
    assert_eq!(dead.next.limit_since, None);
}

#[test]
fn the_list_cell_names_the_kind_and_keeps_the_usage_limit_cell() {
    assert_eq!(ae::theme::seat_word("billing"), "needs you (billing)");
    assert_eq!(ae::theme::seat_word("account"), "needs you (account)");
    assert_eq!(ae::theme::seat_word("limit"), "needs you (usage limit)");
    let billing = cell(Some("billing"), false, false);
    assert!(billing.contains("needs you (billing)"), "{billing}");
    assert!(!billing.contains("needs you (usage limit)"), "{billing}");
    let moving = cell(Some("account"), false, true);
    assert!(
        moving.contains("needs you (account) · auto-reseat in flight"),
        "{moving}"
    );
    let usage = cell(Some("working"), true, true);
    assert!(
        usage.contains("needs you (usage limit) · auto-reseat in flight"),
        "{usage}"
    );
    assert!(!usage.contains("needs you (billing)"), "{usage}");
}

fn cell(state: Option<&str>, limit: bool, moving: bool) -> String {
    let mut session = ae::digest::SessionEntry::new("solo", ae::digest::Status::Running);
    session.agents = vec![ae::digest::AgentEntry {
        reference: "lead".to_owned(),
        name: "lead".to_owned(),
        alias: "lead".to_owned(),
        alive: Some(true),
        state: state.map(str::to_owned),
        reason: limit.then_some(ae::attention::Reason::Limit),
        auto_reseat_open: moving,
        ..ae::digest::AgentEntry::default()
    }];
    ae::listing::table(&[&session])
}

const CONFIG: &str = "\
[clients]
muse = muse
opencode = opencode
codex = codex

[profiles]
emberm = muse --model muse-ember-1.0
cpsol61h = opencode --model github-copilot/gpt-6.1-sol
flashgo = opencode --model opencode-go/deepseek-v4-flash
zen = opencode --model opencode/big-pickle
bare = opencode --model glm-5
solx = codex --model sol
";

fn record(
    tool: &str,
    client: &str,
    provider: &str,
    kind: &str,
    seat: &str,
    reason: &str,
    since: i64,
) -> String {
    let provider_line = if provider.is_empty() {
        String::new()
    } else {
        format!("provider={provider}\n")
    };
    format!(
        "\
version=1
tool={tool}
client={client}
{provider_line}since={since}
kind={kind}
seat={seat}
reason={reason}
"
    )
}

struct Root {
    scratch: crate::cli::OwnedScratch,
    ae_home: std::path::PathBuf,
}

impl Root {
    fn new(tag: &str) -> Self {
        let scratch = crate::cli::OwnedScratch::root("vendorblock", tag);
        let ae_home = scratch.path().join(".ae");
        std::fs::create_dir_all(ae_home.join("vendorblock")).expect("state");
        std::fs::create_dir_all(ae_home.join("sessions")).expect("sessions");
        std::fs::write(ae_home.join("config"), CONFIG).expect("config");
        Self { scratch, ae_home }
    }

    fn write(&self, stem: &str, body: &str) {
        std::fs::write(
            self.ae_home.join("vendorblock").join(format!("{stem}.rec")),
            body,
        )
        .expect("record");
    }
}

fn quota_table(root: &Root) -> (u8, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(root.scratch.path()),
            global: Some(&root.ae_home.join("config")),
            local: None,
            sessions: Some(&root.ae_home.join("sessions")),
            now: SINCE,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    (
        code,
        String::from_utf8(stdout).expect("utf-8"),
        String::from_utf8(stderr).expect("utf-8"),
    )
}

#[test]
fn ae_quota_shows_a_block_and_reading_it_does_not_clear_it() {
    let root = Root::new("show");
    root.write(
        "client-muse",
        &record(
            "muse",
            "muse",
            "",
            "billing",
            "testpins-rev",
            "Billing verification failed",
            SINCE,
        ),
    );
    root.write(
        "provider-github-copilot",
        &record(
            "opencode",
            "opencode",
            "github-copilot",
            "account",
            "trustfix-nav",
            "You have exceeded your monthly quota",
            SINCE,
        ),
    );
    // A different opencode client, a future stamp, a mismatched tool and a
    // duplicate key do not become a second block.
    root.write(
        "provider-opencode-go",
        &record(
            "opencode",
            "opencode",
            "opencode-go",
            "account",
            "x",
            "nope",
            SINCE + 1,
        ),
    );
    root.write(
        "client-codex",
        "version=1\ntool=codex\nclient=codex\nsince=1\nsince=2\nkind=billing\nseat=x\nreason=x\n",
    );
    let muse_path = root.ae_home.join("vendorblock/client-muse.rec");
    let before = std::fs::read(&muse_path).expect("record");
    let (code, out, err) = quota_table(&root);
    assert_eq!(code, 0, "{err}\n{out}");
    assert!(
        out.contains(&format!("blocked (billing since {SINCE_ISO}")),
        "{out}"
    );
    assert!(out.contains("ae quota clear client-muse"), "{out}");
    assert!(
        out.contains(&format!("blocked (account since {SINCE_ISO}")),
        "{out}"
    );
    assert!(
        out.contains("ae quota clear provider-github-copilot"),
        "{out}"
    );
    assert!(
        !out.contains("ae quota clear provider-opencode-go"),
        "a future stamp is not a block: {out}"
    );
    assert!(
        !out.contains("ae quota clear client-codex"),
        "a damaged record is not a block: {out}"
    );
    assert_eq!(std::fs::read(&muse_path).expect("still there"), before);
}

fn ae_quota(root: &Root, args: &[&str]) -> std::process::Output {
    let mut runner = crate::cli::ae();
    runner
        .current_dir(root.scratch.path())
        .env("HOME", root.scratch.path())
        .env("AE_HOME", &root.ae_home)
        .args(args)
        .output()
        .expect("ae quota")
}

#[test]
fn ae_quota_clear_removes_one_client_and_a_second_clear_refuses() {
    let root = Root::new("clear");
    root.write(
        "client-muse",
        &record(
            "muse",
            "muse",
            "",
            "billing",
            "testpins-rev",
            "Billing verification failed",
            SINCE,
        ),
    );
    root.write(
        "provider-github-copilot",
        &record(
            "opencode",
            "opencode",
            "github-copilot",
            "account",
            "trustfix-nav",
            "You have exceeded your monthly quota",
            SINCE,
        ),
    );
    let cleared = ae_quota(&root, &["quota", "clear", "client-muse"]);
    let err = String::from_utf8_lossy(&cleared.stderr);
    let out = String::from_utf8_lossy(&cleared.stdout);
    assert_eq!(cleared.status.code(), Some(0), "{err}{out}");
    assert!(!root.ae_home.join("vendorblock/client-muse.rec").exists());
    assert!(
        root.ae_home
            .join("vendorblock/provider-github-copilot.rec")
            .exists()
    );
    let again = ae_quota(&root, &["quota", "clear", "client-muse"]);
    let err = String::from_utf8_lossy(&again.stderr);
    let out = String::from_utf8_lossy(&again.stdout);
    assert_ne!(again.status.code(), Some(0), "{err}{out}");
    assert!(
        format!("{err}{out}").contains("no block recorded for client-muse"),
        "{err}{out}"
    );
    let other = ae_quota(&root, &["quota", "clear", "provider-opencode"]);
    let err = String::from_utf8_lossy(&other.stderr);
    let out = String::from_utf8_lossy(&other.stdout);
    assert_ne!(other.status.code(), Some(0), "{err}{out}");
    assert!(
        format!("{err}{out}").contains("no block recorded for provider-opencode"),
        "provider-opencode is not client-muse: {err}{out}"
    );
    assert!(
        root.ae_home
            .join("vendorblock/provider-github-copilot.rec")
            .exists()
    );
    let unknown = ae_quota(&root, &["quota", "clear", "nosuch"]);
    let err = String::from_utf8_lossy(&unknown.stderr);
    let out = String::from_utf8_lossy(&unknown.stdout);
    assert_ne!(unknown.status.code(), Some(0), "{err}{out}");
    assert!(
        format!("{err}{out}").contains("unknown scope nosuch"),
        "{err}{out}"
    );
}
