//! Phase 3 end to end: one classified snapshot in, the surfaces an operator or
//! a consumer sees out.
//!
//! Gate: `docs/migration/p1-phase3-gate.md`, blob
//! `49c20d9ad5d8d2e131c41cbc04f91e9d086d3da2` — fifteen criteria. Each test
//! names the one it answers.
//!
//! # The reference snapshot
//!
//! Four candidates per status group, named to be hostile to every order that is
//! NOT the ratified one: `AlphaR`, `ZetaR`, `alpha10R`, `alpha9R` (and the `U`
//! and `S` suffixes for unknown and stopped). Those names separate C-byte order
//! from case-folded order (`AlphaR` before `ZetaR` before `alpha*` only if case
//! is significant), from natural-number order (`alpha10R` before `alpha9R` only
//! if digits are compared as text), and from creation order (they are planted
//! backwards).
//!
//! Per group the four rows carry independent attention/activity facts:
//! attention-only, activity-only, both, neither.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ae::attention::Reason;
use ae::digest::{AgentEntry, SessionEntry, Status};
use ae::filters::ListArgs;
use ae::inventory::FailedSource;
use ae::json;
use ae::listing::{World, diagnostic, render};
use ae::time::Timestamp;
use std::path::Path;

use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw;

const NOW: Timestamp = Timestamp::from_epoch(1_780_000_000);

/// The four name stems, in the order the gate pre-registers as correct.
const C_ORDER: [&str; 4] = ["Alpha", "Zeta", "alpha10", "alpha9"];

/// The gate's four independent fact pairs, one per row in a group.
const FACTS: [(bool, bool); 4] = [(true, false), (false, true), (true, true), (false, false)];

fn suffix(status: Status) -> &'static str {
    match status {
        Status::Running => "R",
        Status::Unknown => "U",
        Status::Stopped => "S",
    }
}

/// One planted row, and everything the manifest records about it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Planted {
    name: String,
    status: Status,
    attention: bool,
    active: bool,
    degraded: bool,
    creation_position: usize,
}

/// The reference snapshot, as a manifest plus the entries themselves.
///
/// The manifest exists so every later assertion can be read against what was
/// PLANTED rather than against what came out — a fixture that describes itself
/// only through its output cannot tell you it planted the wrong thing.
struct Reference {
    manifest: Vec<Planted>,
    entries: Vec<SessionEntry>,
}

impl Reference {
    /// Build the reference snapshot, supplied in `order`.
    fn new(order: Supply) -> Self {
        let mut manifest = Vec::new();
        let mut creation = 0;
        // Creation order is deliberately OPPOSED to C order: reversed stems,
        // and stopped before unknown before running.
        for status in [Status::Stopped, Status::Unknown, Status::Running] {
            for (stem, (attention, active)) in C_ORDER.iter().rev().zip(FACTS.iter().copied().rev())
            {
                manifest.push(Planted {
                    name: format!("{stem}{}", suffix(status)),
                    status,
                    attention,
                    active,
                    // One degraded row per group, independent of every other
                    // fact, so criterion 5's unknown x degraded pair exists and
                    // criterion 13 can watch degradation survive filtering.
                    degraded: *stem == "Zeta",
                    creation_position: creation,
                });
                creation += 1;
            }
        }
        let mut entries: Vec<SessionEntry> = manifest.iter().map(entry_of).collect();
        order.apply(&mut entries);
        Self { manifest, entries }
    }

    fn world(&self) -> World {
        World::new(NOW, self.entries.clone())
    }

    fn incomplete_world(&self, losses: &[FailedSource]) -> World {
        World::new(NOW, self.entries.clone()).with_losses(losses)
    }

    /// Every planted name whose facts satisfy `keep`, in the order the gate
    /// requires: group order running/unknown/stopped, C-byte within a group.
    fn expected<F: Fn(&Planted) -> bool>(&self, groups: &[Status], keep: F) -> Vec<String> {
        let mut wanted = Vec::new();
        for status in groups {
            let mut group: Vec<&Planted> = self
                .manifest
                .iter()
                .filter(|row| row.status == *status && keep(row))
                .collect();
            group.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
            wanted.extend(group.into_iter().map(|row| row.name.clone()));
        }
        wanted
    }
}

/// How the reference rows are handed to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Supply {
    /// As created — reversed stems, stopped first.
    Creation,
    /// The exact reverse of the required output order.
    ReverseRequired,
    /// Statuses interleaved, so no contiguous group exists in the input.
    Interleaved,
    /// Sorted the way a natural-number comparator would.
    NaturalNumber,
    /// Sorted the way a case-folding comparator would.
    CaseFolded,
}

impl Supply {
    const ALL: [Self; 5] = [
        Self::Creation,
        Self::ReverseRequired,
        Self::Interleaved,
        Self::NaturalNumber,
        Self::CaseFolded,
    ];

    fn apply(self, entries: &mut Vec<SessionEntry>) {
        match self {
            Self::Creation => {}
            Self::ReverseRequired => {
                entries.sort_by(|left, right| {
                    group_rank(right.status)
                        .cmp(&group_rank(left.status))
                        .then_with(|| right.name.as_bytes().cmp(left.name.as_bytes()))
                });
            }
            Self::Interleaved => {
                let mut by_group: BTreeMap<usize, Vec<SessionEntry>> = BTreeMap::new();
                for entry in entries.drain(..) {
                    by_group
                        .entry(group_rank(entry.status))
                        .or_default()
                        .push(entry);
                }
                let mut columns: Vec<Vec<SessionEntry>> = by_group.into_values().collect();
                let deepest = columns.iter().map(Vec::len).max().unwrap_or_default();
                for index in 0..deepest {
                    for column in &mut columns {
                        if index < column.len() {
                            entries.push(column[index].clone());
                        }
                    }
                }
            }
            Self::NaturalNumber => {
                entries.sort_by_key(|entry| natural_key(&entry.name));
            }
            Self::CaseFolded => {
                entries.sort_by_key(|entry| entry.name.to_lowercase());
            }
        }
    }
}

fn group_rank(status: Status) -> usize {
    match status {
        Status::Running => 0,
        Status::Unknown => 1,
        Status::Stopped => 2,
    }
}

/// The key a natural-number comparator would sort by: digits as numbers.
fn natural_key(name: &str) -> (String, u64) {
    let digits: String = name.chars().filter(char::is_ascii_digit).collect();
    let letters: String = name.chars().filter(|c| !c.is_ascii_digit()).collect();
    (letters, digits.parse().unwrap_or_default())
}

fn entry_of(row: &Planted) -> SessionEntry {
    let mut entry = if row.degraded {
        SessionEntry::degraded(&row.name, row.status)
    } else {
        SessionEntry::new(&row.name, row.status)
    };
    if row.attention {
        entry.attention = Some(Reason::Blocked);
    }
    if row.active {
        entry.last_active_epoch = Some(NOW.epoch() - 10);
    }
    entry.agents = vec![AgentEntry {
        reference: "cl:lead".to_owned(),
        alias: "cl".to_owned(),
        name: "lead".to_owned(),
        ..AgentEntry::default()
    }];
    entry
}

/// Invoke the real `list`/`ls` surface and return `(stdout, stderr, code)`.
///
/// Criterion 12 forbids a warning that only `--all` sees, and criterion 11
/// binds BOTH command spellings. Neither obligation is on `render` — they are on
/// the surface an operator invokes, and a test that called the renderer directly
/// could not see a filter gate or an alias gap at all. That is the
/// universal-obligation-checked-on-a-particular shape, so these go through the
/// entry point.
fn invoke_over(spelling: &str, flags: &[&str], world: Option<&World>) -> (String, String, u8) {
    let mut argv = vec![spelling.to_owned()];
    argv.extend(flags.iter().map(|flag| (*flag).to_owned()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = match ae::run_with(&argv, world, &mut out, &mut err) {
        Ok(code) => code,
        Err(why) => panic!("{spelling} {flags:?}: {why}"),
    };
    let decode = |bytes: Vec<u8>| match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(why) => panic!("output must be utf-8: {why}"),
    };
    (decode(out), decode(err), code)
}

fn args(tokens: &[&str]) -> ListArgs {
    match ListArgs::parse(tokens) {
        Ok(parsed) => parsed,
        Err(unknown) => panic!("these are documented flags: {unknown:?}"),
    }
}

/// The identity sequence a HUMAN listing shows.
///
/// A session row starts at column zero; an agent row is indented. Layout beyond
/// that is an open choice (criterion 15), so nothing here reads a column
/// position or a width.
fn human_rows(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.starts_with(char::is_whitespace) && !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            (
                fields.next().unwrap_or_default().to_owned(),
                fields.next().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The identity sequence a JSON document shows.
fn json_rows(text: &str) -> Vec<(String, String)> {
    let document = match json::parse(text.trim_end()) {
        Ok(document) => document,
        Err(why) => panic!("one complete document: {why:?}"),
    };
    let Some(json::Value::Arr(entries)) = document.get("sessions") else {
        panic!("sessions must be an array");
    };
    entries
        .iter()
        .map(|entry| {
            (
                entry.get_str("name").unwrap_or_default().to_owned(),
                entry.get_str("status").unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn names(rows: &[(String, String)]) -> Vec<String> {
    rows.iter().map(|(name, _)| name.clone()).collect()
}

/// Every view the phase-3 criteria exercise, as flag lists WITHOUT `--json`.
fn every_human_view() -> Vec<Vec<&'static str>> {
    let mut views = Vec::new();
    for scope in [vec![], vec!["--running"], vec!["--stopped"], vec!["--all"]] {
        for filter in [
            vec![],
            vec!["--needs-attn"],
            vec!["--active"],
            vec!["--needs-attn", "--active"],
        ] {
            let mut view = scope.clone();
            view.extend(filter);
            views.push(view);
        }
    }
    views
}

// ---- criterion 4: the base status views --------------------------------

#[test]
fn criterion_4_the_active_and_history_views_have_the_exact_ratified_domains() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for (flags, groups) in [
        (vec![], vec![Status::Running, Status::Unknown]),
        (vec!["--running"], vec![Status::Running, Status::Unknown]),
        (vec!["--stopped"], vec![Status::Stopped]),
        (
            vec!["--all"],
            vec![Status::Running, Status::Unknown, Status::Stopped],
        ),
    ] {
        let wanted = reference.expected(&groups, |_| true);
        let human = human_rows(&render(&args(&flags), &world));
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let machine = json_rows(&render(&args(&json_flags), &world));

        assert_eq!(names(&human), wanted, "human {flags:?}");
        assert_eq!(names(&machine), wanted, "json {flags:?}");
        // The failure the row names: unknown dropped from an active view.
        if groups.contains(&Status::Unknown) {
            assert!(
                names(&human).iter().any(|name| name.ends_with('U')),
                "{flags:?}: the active view must show unknown sessions"
            );
        }
        assert_eq!(
            names(&human).iter().any(|name| name.ends_with('S')),
            groups.contains(&Status::Stopped),
            "{flags:?}: stopped membership must match the view"
        );
    }
}

// ---- criterion 5: status is rendered literally --------------------------

#[test]
fn criterion_5_every_status_is_spelled_out_and_no_filter_rewrites_one() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    let human = render(&args(&["--all"]), &world);
    for spelling in ["running", "unknown", "stopped"] {
        assert!(
            human.contains(spelling),
            "the human surface must spell {spelling}: {human}"
        );
    }
    let planted: BTreeMap<String, Status> = reference
        .manifest
        .iter()
        .map(|row| (row.name.clone(), row.status))
        .collect();
    for flags in every_human_view() {
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        for (name, status) in json_rows(&render(&args(&json_flags), &world)) {
            let expected = planted.get(&name).copied().unwrap_or_else(|| {
                panic!("{flags:?}: {name} was rendered but never planted");
            });
            assert_eq!(
                status,
                expected.as_str(),
                "{flags:?}: filtering changed {name}'s status"
            );
        }
        for (name, status) in human_rows(&render(&args(&flags), &world)) {
            let expected = planted.get(&name).copied().unwrap_or_else(|| {
                panic!("{flags:?}: {name} was rendered but never planted");
            });
            assert_eq!(status, expected.as_str(), "{flags:?}: human {name}");
        }
    }
}

// ---- criteria 6, 7, 8: the live-scope filters ---------------------------

#[test]
fn criterion_6_attention_filtering_is_the_positive_fact_on_running_or_unknown() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.push("--needs-attn");
        // Stopped rows carry attention=true too, and must NOT appear.
        let wanted = reference.expected(&groups, |row| {
            row.attention && row.status != Status::Stopped
        });
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        assert!(
            !wanted.is_empty() && wanted.iter().any(|name| name.ends_with('U')),
            "{flags:?}: a matching UNKNOWN row must survive, or the arm proves nothing"
        );
        assert!(
            wanted.len() < reference.manifest.len(),
            "{flags:?}: and the attention=false controls must be excluded"
        );
    }
    for flags in [
        vec!["--stopped", "--needs-attn"],
        vec!["--needs-attn", "--stopped"],
    ] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}: a stopped session never satisfies a live-scope predicate"
        );
    }
}

#[test]
fn criterion_7_activity_filtering_is_the_positive_fact_on_running_or_unknown() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.push("--active");
        let wanted = reference.expected(&groups, |row| row.active && row.status != Status::Stopped);
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        assert!(
            wanted.iter().any(|name| name.ends_with('U')),
            "{flags:?}: a matching UNKNOWN row must survive"
        );
    }
    for flags in [vec!["--stopped", "--active"], vec!["--active", "--stopped"]] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}: empty in both argument orders"
        );
    }
}

#[test]
fn criterion_8_the_two_filters_intersect_rather_than_uniting() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    for scope in [vec![], vec!["--running"], vec!["--all"]] {
        let groups = if scope == vec!["--all"] {
            vec![Status::Running, Status::Unknown, Status::Stopped]
        } else {
            vec![Status::Running, Status::Unknown]
        };
        let mut flags = scope.clone();
        flags.extend(["--needs-attn", "--active"]);
        let wanted = reference.expected(&groups, |row| {
            row.attention && row.active && row.status != Status::Stopped
        });
        assert_eq!(
            names(&human_rows(&render(&args(&flags), &world))),
            wanted,
            "{flags:?}"
        );
        // The union would also admit the attention-only and activity-only rows;
        // last-filter-wins would admit one of those two whole classes.
        let union = reference.expected(&groups, |row| {
            (row.attention || row.active) && row.status != Status::Stopped
        });
        assert!(
            wanted.len() < union.len(),
            "{flags:?}: the fixture must distinguish intersection from union"
        );
    }
    for flags in [
        vec!["--stopped", "--needs-attn", "--active"],
        vec!["--active", "--needs-attn", "--stopped"],
    ] {
        assert!(
            human_rows(&render(&args(&flags), &world)).is_empty(),
            "{flags:?}"
        );
    }
}

// ---- criteria 9 and 10: the product owns the order ----------------------

#[test]
fn criterion_10_every_adversarial_supply_order_differs_from_the_required_one() {
    // CALIBRATION FIRST. Criterion 9 means nothing unless the orders it is
    // supposed to defeat actually differ from the answer — an arm whose control
    // agrees is inconclusive, not a pass.
    let required = Reference::new(Supply::Creation)
        .expected(&[Status::Running, Status::Unknown, Status::Stopped], |_| {
            true
        });
    for supply in Supply::ALL {
        let supplied: Vec<String> = Reference::new(supply)
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_ne!(
            supplied, required,
            "{supply:?}: this supply order already equals the required order, so \
             asserting the required order against it would prove nothing"
        );
    }
    // And the two comparators the row names by name are calibrated on the
    // planted stems, independent of supply order.
    let mut natural = C_ORDER.to_vec();
    natural.sort_by_key(|name| natural_key(name));
    assert_ne!(
        natural,
        C_ORDER.to_vec(),
        "natural-number order must differ from C order on these names"
    );
    let mut folded = C_ORDER.to_vec();
    folded.sort_by_key(|name| name.to_lowercase());
    assert_ne!(
        folded,
        C_ORDER.to_vec(),
        "case-folded order must differ from C order on these names"
    );
}

#[test]
fn criterion_9_the_output_order_is_c_byte_within_group_and_running_unknown_stopped() {
    for supply in Supply::ALL {
        let reference = Reference::new(supply);
        let world = reference.world();
        for (flags, groups) in [
            (vec![], vec![Status::Running, Status::Unknown]),
            (vec!["--running"], vec![Status::Running, Status::Unknown]),
            (vec!["--stopped"], vec![Status::Stopped]),
            (
                vec!["--all"],
                vec![Status::Running, Status::Unknown, Status::Stopped],
            ),
            (
                vec!["--all", "--needs-attn"],
                vec![Status::Running, Status::Unknown, Status::Stopped],
            ),
        ] {
            let attention_only = flags.contains(&"--needs-attn");
            let wanted = reference.expected(&groups, |row| {
                !attention_only || (row.attention && row.status != Status::Stopped)
            });
            let human = names(&human_rows(&render(&args(&flags), &world)));
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let machine = names(&json_rows(&render(&args(&json_flags), &world)));
            assert_eq!(human, wanted, "{supply:?} human {flags:?}");
            assert_eq!(machine, wanted, "{supply:?} json {flags:?}");
        }
    }
}

// ---- criterion 11: the two surfaces agree -------------------------------

#[test]
fn criterion_11_human_and_json_select_and_order_identically() {
    let reference = Reference::new(Supply::ReverseRequired);
    let world = reference.world();
    for spelling in ["list", "ls"] {
        for flags in every_human_view() {
            let (human_text, _, _) = invoke_over(spelling, &flags, Some(&world));
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            let (json_text, _, _) = invoke_over(spelling, &json_flags, Some(&world));
            assert_eq!(
                human_rows(&human_text),
                json_rows(&json_text),
                "{spelling} {flags:?}: the two surfaces disagree"
            );
            // ...and the ALIAS agrees with the primary spelling, which is the
            // half a renderer-level test cannot see at all.
            let (primary, _, _) = invoke_over("list", &flags, Some(&world));
            assert_eq!(
                human_rows(&human_text),
                human_rows(&primary),
                "{spelling} {flags:?}: the alias selected differently"
            );
        }
    }
}

// ---- criterion 12: every human view exposes incompleteness --------------

#[test]
fn criterion_12_every_human_view_warns_with_the_distinct_source_count() {
    let reference = Reference::new(Supply::Creation);
    let one_loss = vec![FailedSource::CanonicalRoot("/home/x/.ae/sessions".into())];
    let two_losses = vec![
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
        FailedSource::WorktreeRoot("/home/x/.ae/worktrees".into()),
    ];
    // One source recorded TWICE is still one source.
    let repeated = vec![
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
        FailedSource::CanonicalRoot("/home/x/.ae/sessions".into()),
    ];

    for (losses, expected) in [
        (Vec::new(), None),
        (one_loss, Some(1_usize)),
        (two_losses, Some(2)),
        (repeated, Some(1)),
    ] {
        let world = reference.incomplete_world(&losses);
        let complete = reference.world();
        // EVERY human view, through BOTH spellings, at the real surface.
        for spelling in ["list", "ls"] {
            for flags in every_human_view() {
                let (stdout, stderr, code) = invoke_over(spelling, &flags, Some(&world));
                assert_eq!(code, 0, "{spelling} {flags:?}");
                match expected {
                    None => assert!(
                        stderr.is_empty(),
                        "{spelling} {flags:?}: a complete snapshot warns about nothing: {stderr}"
                    ),
                    Some(count) => {
                        assert!(
                            !stderr.is_empty(),
                            "{spelling} {flags:?}: this view emitted no warning — a warning \
                             only some views see is one most invocations never get"
                        );
                        assert!(
                            stderr.contains(&count.to_string()),
                            "{spelling} {flags:?}: the warning must carry the COUNT {count}: \
                             {stderr}"
                        );
                    }
                }
                // The found rows never change.
                let (whole, _, _) = invoke_over(spelling, &flags, Some(&complete));
                assert_eq!(
                    human_rows(&stdout),
                    human_rows(&whole),
                    "{spelling} {flags:?}: the found rows changed"
                );
            }
        }
    }
}

#[test]
fn criterion_12_the_count_is_not_hardcoded_because_two_sources_read_two() {
    // The control the criterion names: a boolean or a constant would satisfy the
    // one-loss arm above. Two distinct sources must read `2`, and the same
    // source twice must still read `1`.
    let reference = Reference::new(Supply::Creation);
    let one = reference.incomplete_world(&[FailedSource::CanonicalRoot("/a".into())]);
    let two = reference.incomplete_world(&[
        FailedSource::CanonicalRoot("/a".into()),
        FailedSource::WorktreeRoot("/b".into()),
    ]);
    let (_, one_warning, _) = invoke_over("list", &[], Some(&one));
    let (_, two_warning, _) = invoke_over("list", &[], Some(&two));
    assert_ne!(
        one_warning, two_warning,
        "one loss and two losses must not produce the same warning"
    );
    assert!(one_warning.contains('1') && two_warning.contains('2'));
}

// ---- criterion 13: every JSON document carries completeness -------------

#[test]
fn criterion_13_every_document_carries_version_2_and_the_completeness_boolean() {
    let reference = Reference::new(Supply::Interleaved);
    let losses = vec![FailedSource::WorktreeState(
        "/home/x/.ae/worktrees/a/.ae".into(),
    )];
    for (world, complete) in [
        (reference.world(), true),
        (reference.incomplete_world(&losses), false),
        (World::new(NOW, Vec::new()), true),
        (World::new(NOW, Vec::new()).with_losses(&losses), false),
    ] {
        for (spelling, flags) in ["list", "ls"].into_iter().flat_map(|spelling| {
            every_human_view()
                .into_iter()
                .map(move |view| (spelling, view))
        }) {
            let mut json_flags = flags.clone();
            json_flags.push("--json");
            // Deliberately NOT asserted: whether the machine surface also warns
            // on stderr. Criterion 13 lists JSON stderr warning policy as an
            // OPEN CHOICE, so an implementation that emits the required document
            // AND a warning beside it is correct — and a test rejecting it would
            // fail criterion 15, which fails the gate itself. This build happens
            // not to warn there; that is a choice, not a contract.
            let (text, _stderr, _) = invoke_over(spelling, &json_flags, Some(&world));
            let document = match json::parse(text.trim_end()) {
                Ok(document) => document,
                Err(why) => panic!("{flags:?}: one document: {why:?}"),
            };
            assert_eq!(
                document.get("schema_version"),
                Some(&json::Value::Num(2)),
                "{flags:?}"
            );
            assert_eq!(text.matches("\"schema_version\"").count(), 1, "{flags:?}");
            assert_eq!(
                document.get("inventory_complete"),
                Some(&json::Value::Bool(complete)),
                "{flags:?}"
            );
            assert_eq!(
                text.matches("\"inventory_complete\"").count(),
                1,
                "{flags:?}"
            );
        }
    }
}

#[test]
fn criterion_13_degradation_survives_every_filter_unchanged() {
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    let planted: BTreeMap<String, bool> = reference
        .manifest
        .iter()
        .map(|row| (row.name.clone(), row.degraded))
        .collect();
    for flags in every_human_view() {
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let text = render(&args(&json_flags), &world);
        let document = match json::parse(text.trim_end()) {
            Ok(document) => document,
            Err(why) => panic!("{flags:?}: {why:?}"),
        };
        let Some(json::Value::Arr(entries)) = document.get("sessions") else {
            panic!("sessions must be an array");
        };
        for entry in entries {
            let name = entry.get_str("name").unwrap_or_default().to_owned();
            let degraded = entry.get("degraded") == Some(&json::Value::Bool(true));
            assert_eq!(
                Some(degraded),
                planted.get(&name).copied(),
                "{flags:?}: {name}'s degradation changed under filtering"
            );
        }
    }
}

// ---- criterion 14: completeness is independent of everything else -------

#[test]
fn criterion_14_flipping_completeness_changes_only_the_warning_and_the_boolean() {
    let reference = Reference::new(Supply::Creation);
    let complete = reference.world();
    let incomplete = reference.incomplete_world(&[FailedSource::Server(
        ae::inventory::ServerId::Selected(ae::meta::Selector::Name("gone".to_owned())),
    )]);

    for flags in every_human_view() {
        assert_eq!(
            human_rows(&render(&args(&flags), &complete)),
            human_rows(&render(&args(&flags), &incomplete)),
            "{flags:?}: human selection, status and order moved"
        );
        let mut json_flags = flags.clone();
        json_flags.push("--json");
        let one = render(&args(&json_flags), &complete);
        let other = render(&args(&json_flags), &incomplete);
        assert_eq!(
            one.replace(r#""inventory_complete":true"#, "X"),
            other.replace(r#""inventory_complete":false"#, "X"),
            "{flags:?}: the ONLY difference in the document is the boolean"
        );
    }
    assert!(diagnostic(&complete).is_none());
    assert!(diagnostic(&incomplete).is_some());
}

// ---- criterion 15: the scope guard, turned on this file -----------------

#[test]
fn criterion_15_this_suite_asserts_no_unratified_presentation_choice() {
    // The gate FAILS ITSELF if a test rejects a correct implementation over an
    // open choice. These are the ones phase 3 lists, checked against this
    // file's own source.
    let source = include_str!("phase3.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("fn criterion_9"),
        "the scan reached the tests"
    );
    for forbidden in [
        // Layout, colour and width are open choices.
        concat!("col", "umn_width"),
        concat!("\\u{1b}", "["),
        // Diagnostic wording and exit status are open choices; the COUNT is not,
        // and `contains(&count.to_string())` is how this file checks it.
        concat!("warning: inv", "entory incomplete"),
        // JSON object field order is an open choice: nothing may compare a
        // whole rendered object to a literal.
        concat!("\"name\":\"Alpha", "R\",\"status\""),
    ] {
        assert!(
            !code.contains(forbidden),
            "this suite pins an OPEN CHOICE, which fails the gate itself: {forbidden}"
        );
    }
    // Human bytes are never compared to a literal table.
    assert!(
        !code.contains(concat!("assert_eq!(", "render(")),
        "no test compares a human rendering to expected bytes"
    );
}

/// A readable dump of the manifest, for a failure message worth reading.
#[allow(dead_code)]
fn describe(reference: &Reference) -> String {
    let mut out = String::new();
    for row in &reference.manifest {
        let _ = writeln!(
            out,
            "{} {} attention={} active={} degraded={} created={}",
            row.name,
            row.status.as_str(),
            row.attention,
            row.active,
            row.degraded,
            row.creation_position
        );
    }
    out
}

// ---- criterion 2: presentation starts from one completed snapshot -------

#[test]
fn criterion_2_presentation_starts_from_one_completed_classified_snapshot() {
    // THE MARKER IS THE PRODUCTION ENTRY POINT, not a line this test appends
    // afterwards. An earlier version called the projection and THEN logged
    // "presentation enter", so everything the projection did happened before the
    // marker and was invisible: a reverse sort inserted inside it still passed.
    // `Presentation::enter` is now the first phase-3 operation, and
    // `at_entry()` is what it received, in the order it received it.
    let root = std::env::temp_dir().join(format!("ae-p3-seq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for name in ["AlphaR", "ZetaR", "alpha10R"] {
        let dir = root.join("sessions").join(name);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            std::fs::write(
                dir.join("meta"),
                "mode=local\nagent.main=cl:lead\ntmux_server_kind=name\ntmux_server=B\n",
            )
        });
        assert!(written.is_ok(), "a planted session");
    }

    // `classify complete`: the snapshot, as classification left it.
    let scan = ae::inventory::durable_records(&ae::inventory::Roots::under(&root));
    let snapshot = ae::liveness::classify(ae::inventory::take(scan, None, &Down), &Down);
    let classified: Vec<(String, &str)> = snapshot
        .sessions
        .iter()
        .map(|c| (c.candidate.name.clone(), c.status.as_str()))
        .collect();

    // `presentation enter`: the production boundary, invoked.
    let presentation = ae::listing::Presentation::enter(&snapshot);
    let at_entry = presentation.at_entry();

    assert_eq!(
        at_entry, classified,
        "the presentation input is the completed classified set, IN ITS ORDER — a \
         filter or sort at or before the boundary would show here"
    );
    assert_eq!(at_entry.len(), 3, "and the fixture is not empty");

    // Everything downstream presents from that one input and changes nothing.
    let world = presentation.world(NOW, ae::session::DEFAULT_UNANSWERED_SECS);
    for spelling in ["list", "ls"] {
        for flags in every_human_view() {
            let (stdout, _, _) = invoke_over(spelling, &flags, Some(&world));
            for (name, status) in human_rows(&stdout) {
                assert!(
                    at_entry
                        .iter()
                        .any(|(known, known_status)| *known == name && *known_status == status),
                    "{spelling} {flags:?}: {name} was presented with facts the input never held"
                );
            }
        }
    }
    assert_eq!(
        presentation.at_entry(),
        at_entry,
        "and the input itself is unchanged after every surface has read it"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A backend whose every query fails — the phase-2 seam, held fixed.
struct Down;

impl ae::inventory::Discovery for Down {
    fn enumerate(
        &self,
        _server: &ae::inventory::ServerId,
    ) -> Result<Vec<ae::inventory::DiscoveredSession>, ae::inventory::QueryFailed> {
        Err(ae::inventory::QueryFailed)
    }
}

// ---- criterion 3: a renderer presents facts, it never re-derives them ---

#[test]
fn criterion_3_the_output_is_a_function_of_the_snapshot_and_nothing_else() {
    // WHAT THIS MEASURES, AND WHAT IT DOES NOT. It moves the filesystem
    // underneath a fixed snapshot and requires the output not to move. That
    // catches a re-derivation whose result REACHES the output — and nothing
    // else. A read whose result is discarded changes no byte and is invisible
    // here, which is exactly what colead demonstrated against an earlier version
    // of this test.
    //
    // The CAPABILITY is closed by
    // `criterion_3_the_places_this_crate_can_read_the_world_are_the_inventoried_ones`,
    // which asks the compiler instead of the output. This arm is the behavioural
    // half, and it claims only its half.
    let root = std::env::temp_dir().join(format!("ae-p3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();

    let render_all = || -> Vec<(Vec<String>, String)> {
        every_human_view()
            .into_iter()
            .map(|flags| {
                let (stdout, stderr, _) = invoke_over("list", &flags, Some(&world));
                (names(&human_rows(&stdout)), stderr)
            })
            .collect()
    };
    let before = render_all();

    // A world on disk that contradicts every planted row.
    assert!(
        std::fs::create_dir_all(root.join("sessions")).is_ok(),
        "an opposed world on disk"
    );
    for row in &reference.manifest {
        let dir = root.join("sessions").join(&row.name);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            std::fs::write(
                dir.join("meta"),
                "mode=copy\nagent.main=cl:other\ngoal=the opposite\n",
            )
        });
        assert!(written.is_ok(), "an opposed record");
    }
    let after = render_all();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        before, after,
        "an external fact reached the presentation, so something re-derived it"
    );
}

#[test]
fn criterion_3_the_output_differential_fires_when_a_fact_actually_changes() {
    // CALIBRATION for the arm above, and calibrated for what that arm can see:
    // a change that REACHES the output. It does not calibrate an access
    // recorder, because that arm has none — the compiler probe is the recorder,
    // and its own non-vacuity check is inside it.
    let reference = Reference::new(Supply::Creation);
    let world = reference.world();
    let (stdout, _, _) = invoke_over("list", &[], Some(&world));
    let baseline = names(&human_rows(&stdout));

    let mut leaked = world.sessions.clone();
    if let Some(first) = leaked
        .iter_mut()
        .find(|entry| entry.status == Status::Unknown)
    {
        first.status = Status::Stopped;
    }
    let (leaked_stdout, _, _) = invoke_over("list", &[], Some(&World::new(NOW, leaked)));

    assert_ne!(
        baseline,
        names(&human_rows(&leaked_stdout)),
        "the differential cannot see a changed fact, so its non-difference proves nothing"
    );
}

// ---- criterion 10: the locale arm, with a demonstrated control ----------

#[test]
fn criterion_10_a_non_c_locale_collates_these_names_differently_and_output_does_not() {
    // The product cannot consult a locale — Rust compares `str` by bytes — but
    // the arm is only meaningful if a locale that WOULD disagree exists and is
    // demonstrated to disagree on these exact names. An arm whose control agrees
    // is inconclusive, not a pass, so this fails loudly rather than passing when
    // no such locale is available.
    let scratch = std::env::temp_dir().join(format!("ae-p3-loc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(std::fs::create_dir_all(&scratch).is_ok(), "a scratch dir");
    let names_file = scratch.join("names");
    let planted = C_ORDER
        .iter()
        .map(|stem| format!("{stem}R"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        std::fs::write(&names_file, format!("{planted}\n")).is_ok(),
        "the planted names"
    );

    let sort_under = |locale: &str| -> Option<Vec<String>> {
        let out = scratch.join(format!("out-{locale}"));
        let err = scratch.join(format!("err-{locale}"));
        let invocation = Invocation::new("sort")
            .arg(names_file.display().to_string())
            .env("LC_ALL", locale);
        let status = raw::run(&invocation, &scratch, &out, &err).ok()?;
        if !matches!(status.outcome(), ExitOutcome::Code(0)) {
            return None;
        }
        Some(
            std::fs::read_to_string(&out)
                .ok()?
                .lines()
                .map(ToOwned::to_owned)
                .collect(),
        )
    };

    let c_order = sort_under("C");
    let other = ["en_US.UTF-8", "en_GB.UTF-8", "de_DE.UTF-8"]
        .into_iter()
        .find_map(|locale| sort_under(locale).map(|order| (locale, order)));
    let _ = std::fs::remove_dir_all(&scratch);

    let c_order = c_order.unwrap_or_else(|| panic!("`sort` under LC_ALL=C must be runnable"));
    let (locale, collated) = other.unwrap_or_else(|| {
        panic!("this arm needs a non-C locale to be available to be meaningful")
    });
    assert_ne!(
        collated, c_order,
        "{locale} collates these names the same as C, so this arm is INCONCLUSIVE \
         rather than a pass — pick names or a locale that disagree"
    );

    // The product's answer is the C one, and it did not consult anything.
    let reference = Reference::new(Supply::CaseFolded);
    let (stdout, _, _) = invoke_over("list", &["--running"], Some(&reference.world()));
    // The active view carries running THEN unknown (SC-017m), so the running
    // group is its prefix — and that prefix must be exactly what `sort` under
    // LC_ALL=C produced for the same names.
    let shown = names(&human_rows(&stdout));
    let running_group: Vec<String> = shown
        .iter()
        .filter(|name| name.ends_with('R'))
        .cloned()
        .collect();
    assert_eq!(
        running_group, c_order,
        "the running group must be exactly `sort`'s C order for the same names"
    );
    assert_eq!(
        shown
            .iter()
            .take(c_order.len())
            .cloned()
            .collect::<Vec<_>>(),
        c_order,
        "and it is the PREFIX, because running precedes unknown"
    );
}

// ---- criterion 3: the access boundary, asked of the compiler ------------

/// Every `(file, line)` where this crate may read the outside world, as CLIPPY
/// reports it under `--force-warn`.
///
/// Asked of the compiler rather than of the source text, because the thing being
/// bounded is a CAPABILITY and not a spelling: `Path::exists` is a filesystem
/// observation under none of the names a reader would think to grep for, and an
/// alias or UFCS call defeats a text scan entirely. `--force-warn` cannot be
/// silenced by any `allow`, `expect` or crate-root attribute, so the doors it
/// reports are the doors that exist.
fn world_reading_sites() -> Vec<(String, usize)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = std::env::temp_dir().join(format!("ae-p3-clippy-{}", std::process::id()));
    let err = out.with_extension("err");
    let invocation = Invocation::new(cargo)
        .arg("clippy")
        .arg("--quiet")
        .arg("--locked")
        .arg("--all-targets")
        .arg("--all-features")
        .arg("--message-format=json")
        .arg("--target-dir")
        .arg(
            manifest
                .join("target")
                .join("world-read-guard")
                .display()
                .to_string(),
        )
        .arg("--")
        .arg("--force-warn")
        .arg("clippy::disallowed_methods");
    let status = raw::run(&invocation, manifest, &out, &err);
    assert!(status.is_ok(), "this guard needs cargo and clippy on PATH");

    let stdout = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&err);
    let mut sites = Vec::new();
    for line in stdout.lines() {
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = json::parse(line) else {
            continue;
        };
        if value.get_str("reason") != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let is_boundary = message.get_str("code_text") == Some("clippy::disallowed_methods")
            || message.get("code").and_then(|code| code.get_str("code"))
                == Some("clippy::disallowed_methods");
        if is_boundary && let Some(json::Value::Arr(spans)) = message.get("spans") {
            for span in spans {
                let (Some(file), Some(json::Value::Num(line_no))) =
                    (span.get_str("file_name"), span.get("line_start"))
                else {
                    continue;
                };
                if let Ok(line_no) = usize::try_from(*line_no) {
                    sites.push((file.to_owned(), line_no));
                }
            }
        }
    }
    sites.sort();
    sites.dedup();
    sites
}

/// Whether `line` in `file` is PRODUCT code rather than a test module.
fn is_product_line(file: &str, line: usize) -> bool {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    match text.find("#[cfg(test)]") {
        Some(at) => text[..at].lines().count() >= line,
        None => true,
    }
}

#[test]
fn criterion_3_the_places_this_crate_can_read_the_world_are_the_inventoried_ones() {
    // A TRIPWIRE OVER ENTRY POINTS, AND ITS LIMIT IS THE TEST. `clippy.toml`
    // names eleven resolved paths; safe std still exposes `canonicalize`,
    // `read_link`, `symlink_metadata`, `OpenOptions::open` and `DirEntry`
    // observations, and a discarded call to any of them slips straight past.
    // The empty dependency tables and `unsafe_code = "forbid"` close the
    // THIRD-PARTY and LIBC routes; neither closes an unlisted safe-std one, and
    // reading them as covering the enumeration is what let an earlier version of
    // this file call a name list a boundary.
    //
    // What actually bounds re-derivation is
    // `criterion_3_presentation_cannot_address_ae_s_own_state` below: with no
    // root and no record path in its input, presentation has nothing to open.
    // This test is the cheap early warning for the eleven, and claims only them.
    let sites = world_reading_sites();
    assert!(
        !sites.is_empty(),
        "the force-warn probe found no world-reading call anywhere; it did not run"
    );

    let mut product: Vec<String> = sites
        .iter()
        .filter(|(file, line)| file.starts_with("src/") && is_product_line(file, *line))
        .map(|(file, _)| file.clone())
        .collect();
    product.sort();
    product.dedup();

    assert_eq!(
        product,
        vec![
            "src/events.rs".to_owned(),
            "src/inventory.rs".to_owned(),
            "src/lib.rs".to_owned(),
            "src/meta.rs".to_owned(),
        ],
        "the set of places product code can reach the eleven named entry points changed"
    );
}

#[test]
fn criterion_3_presentation_cannot_address_ae_s_own_state() {
    // THE STRUCTURAL CLAIM, and it holds whatever the spelling. A renderer that
    // wants to re-derive a fact it is presenting needs an ADDRESS for that
    // fact — the sessions root, the worktrees root, or the record's own
    // directory. None of the three is reachable from the presentation input, so
    // the defect that matters is unrepresentable rather than merely forbidden.
    //
    // NOT CLAIMED: that no syscall can occur. A gratuitous `canonicalize("/")`
    // stays expressible and this says nothing about it. What it says is that a
    // read of the thing being presented has nowhere to point.
    let module = product_module("listing.rs");

    // The presentation input, field by field. A path here would be an address.
    let world = module
        .split_once("pub struct World {")
        .and_then(|(_, rest)| rest.split_once('}'))
        .map_or_else(
            || panic!("World must be declared in listing.rs"),
            |(body, _)| body.to_owned(),
        );
    for address in ["PathBuf", "&Path", "&'a Path"] {
        assert!(
            !world.contains(address),
            "the presentation input carries an address ({address}), so a re-derivation \
             has something to open: {world}"
        );
    }
    assert!(
        world.contains("losses: usize"),
        "SC-017o's fact crosses as a COUNT, which is what removed the last path"
    );

    // And the same for what it holds per session: SC-509's `origin`/`work_dir`
    // are the operator's own project directories, carried as payload STRINGS to
    // print. ae's own state root is nowhere in the type.
    let digest = product_module("digest.rs");
    let entry = digest
        .split_once("pub struct SessionEntry {")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map_or_else(
            || panic!("SessionEntry must be declared in digest.rs"),
            |(body, _)| body.to_owned(),
        );
    for address in ["PathBuf", "&Path"] {
        assert!(
            !entry.contains(address),
            "a session entry carries an address ({address}): {entry}"
        );
    }
}

/// One product module's source, comments stripped, tests excluded.
fn product_module(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        panic!("{name} must be readable");
    };
    let code: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    code.split_once("#[cfg(test)]")
        .map_or(code.clone(), |(module, _)| module.to_owned())
}
