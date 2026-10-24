//! Usage transcript and pricing behavior over frozen hostile inputs.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixture setup crosses the filesystem boundary the product observes"
)]

use ae::usage::{
    Coverage, Inputs, Observation, SeatUsage, SessionInput, SessionUsage, Tokens, UsageTotal,
    claude, codex, prices,
};
use std::path::PathBuf;

const CLAUDE_ID: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const RETIRED_ID: &str = "0199c0de-1234-4890-abcd-ef0123456790";
const CODEX_ID: &str = "01a08046-1974-7352-ade3-81a786200795";

fn rig(tag: &str) -> PathBuf {
    let root = super::cli::OwnedScratch::root("usage", tag).keep();
    std::fs::create_dir_all(root.join("sessions/live")).expect("session");
    root
}

#[test]
fn claude_streaming_duplicates_keep_greater_usage_and_subagent_usage_adds() {
    let main = claude::parse(include_bytes!("../fixtures/usage/claude-main.jsonl"));
    let subagent = claude::parse(include_bytes!("../fixtures/usage/claude-subagent.jsonl"));
    let parsed = claude::reduce(&main, &[subagent]);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].model, "claude-fable-5-1-20260901");
    assert_eq!(
        parsed[0].tokens,
        Tokens {
            input: 127,
            cache_write: 20,
            cache_read: 33,
            output: 42,
        }
    );
    let hostile = claude::parse(
        br#"{"type":"assistant","message":{"id":"m","model":"bad\nmodel","usage":{"input_tokens":1}}}"#,
    );
    assert!(claude::reduce(&hostile, &[]).is_empty());
}

#[test]
fn codex_uses_last_cumulative_total_and_does_not_double_reasoning_output() {
    let parsed = codex::parse(
        include_bytes!("../fixtures/usage/codex-rollout.jsonl"),
        true,
    );
    assert_eq!(parsed.models, ["gpt-5.6-luna", "gpt-5.6-sol"]);
    assert!(parsed.has_token_count());
    assert!(parsed.approximate);
    assert_eq!(
        parsed.tokens,
        Tokens {
            input: 70,
            cache_write: 10,
            cache_read: 20,
            output: 40,
        }
    );
    assert!(
        codex::parse(
            br#"{"type":"turn_context","payload":{"model":"bad\nmodel"}}"#,
            true
        )
        .models
        .is_empty()
    );
    let zero = codex::parse(
        br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":0}}}}
"#,
        true,
    );
    assert!(zero.has_token_count());
    assert_eq!(zero.tokens, Tokens::default());
    assert!(
        !codex::parse(
            br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            true,
        )
        .has_token_count()
    );
    assert!(
        !codex::parse(
            br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":null}}}
"#,
            true,
        )
        .has_token_count()
    );
    assert!(
        !codex::parse(
            br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{}}}}
"#,
            true,
        )
        .has_token_count()
    );
}

#[test]
fn codex_malformed_counters_never_become_zero_and_last_valid_survives() {
    let malformed = [
        r#"{"input_tokens":"10","output_tokens":2}"#,
        r#"{"input_tokens":10,"output_tokens":"2"}"#,
        r#"{"input_tokens":-10,"output_tokens":2}"#,
        r#"{"input_tokens":10,"output_tokens":-2}"#,
    ];
    for total in malformed {
        let rollout = concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":TOTAL}}}"#,
            "\n"
        )
        .replace("TOTAL", total);
        let parsed = codex::parse(rollout.as_bytes(), true);
        assert!(
            !parsed.has_token_count(),
            "accepted malformed total: {total}"
        );
        assert!(
            parsed.malformed_token_count(),
            "lost malformed total: {total}"
        );
    }

    let valid_then_malformed = codex::parse(
        br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":11,"cached_input_tokens":2,"cache_write_input_tokens":1,"output_tokens":3}}}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":null}}}
"#,
        true,
    );
    assert!(valid_then_malformed.has_token_count());
    assert!(valid_then_malformed.malformed_token_count());
    assert_eq!(
        valid_then_malformed.tokens,
        Tokens {
            input: 8,
            cache_write: 1,
            cache_read: 2,
            output: 3,
        }
    );
    assert!(valid_then_malformed.approximate);

    let root = rig("codex-malformed-counter");
    let store = root.join("codex");
    let path = store.join(format!(
        "sessions/2026/09/08/rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"
    ));
    std::fs::create_dir_all(store.join("sessions/2026/09/08")).expect("Codex day");
    std::fs::write(
        path,
        br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":null}}}
"#,
    )
    .expect("malformed rollout");
    std::fs::write(root.join("sessions/live/meta"), format!("schema=2\nseat.main=lead\nharness_session.main={CODEX_ID}\nagent_bin.main=codex\nconfig_home.main={}\n", store.display())).expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(
        observed.sessions[0].seats[0].coverage,
        Coverage::Unreadable("malformed token usage".to_owned())
    );
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bundled_prices_match_exact_then_longest_prefix_and_unknown_stays_unpriced() {
    assert_eq!(
        prices::lookup("claude-opus-5").map(|p| p.output),
        Some(25_000_000)
    );
    assert_eq!(
        prices::lookup("claude-fable-5-1-20260901").map(|p| p.cache_read),
        Some(250_000)
    );
    assert_eq!(prices::lookup("claude-opus-5-preview"), None);
    assert_eq!(prices::lookup("claude-opus-5-2026090"), None);
    assert_eq!(prices::lookup("unlisted-model"), None);
    assert_eq!(
        prices::lookup("claude-fable-5").map(|p| p.cache_read),
        Some(1_000_000),
        "the pinned source deliberately differs from Fable 5.1"
    );
}

#[test]
fn claude_observer_streams_large_files_and_skips_an_overlong_line() {
    let root = rig("large-claude");
    let store = root.join("claude");
    std::fs::create_dir_all(store.join("projects/work")).expect("Claude project");
    let mut transcript = vec![b'x'; 4 * 1024 * 1024 + 1];
    transcript.push(b'\n');
    transcript.extend_from_slice(include_bytes!("../fixtures/usage/claude-main.jsonl"));
    std::fs::write(
        store.join(format!("projects/work/{CLAUDE_ID}.jsonl")),
        transcript,
    )
    .expect("large transcript");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!(
            "schema=2\nseat.main=lead\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main={}\n",
            store.display()
        ),
    )
    .expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats[0].coverage, Coverage::Truncated);
    assert_eq!(observed.sessions[0].seats[0].tokens.input, 120);
    assert!(observed.sessions[0].seats[0].approximate);
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn claude_large_valid_record_marks_partial_after_short_control() {
    let root = rig("large-valid-claude");
    let store = root.join("claude");
    let path = store.join(format!("projects/work/{CLAUDE_ID}.jsonl"));
    std::fs::create_dir_all(store.join("projects/work")).expect("Claude project");
    std::fs::write(
        &path,
        br#"{"type":"assistant","message":{"id":"small","model":"claude-fable-5-1-20260901","usage":{"input_tokens":3,"output_tokens":2}}}
"#,
    )
    .expect("short transcript");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!("schema=2\nseat.main=lead\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main={}\n", store.display()),
    ).expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let short = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(short.sessions[0].seats[0].coverage, Coverage::Read);
    assert!(!short.sessions[0].total.partial);
    let mut large = std::fs::read(&path).expect("short transcript read");
    large.extend_from_slice(format!("{{\"type\":\"assistant\",\"message\":{{\"id\":\"large\",\"model\":\"claude-fable-5-1-20260901\",\"usage\":{{\"input_tokens\":5}},\"padding\":\"{}\"}}}}\n", "x".repeat(1024 * 1024)).as_bytes());
    std::fs::write(path, large).expect("large transcript");
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    let seat = &observed.sessions[0].seats[0];
    assert_eq!(seat.coverage, Coverage::Truncated);
    assert!(seat.approximate);
    assert_eq!(seat.tokens.input, 3);
    assert_eq!(seat.tokens.output, 2);
    assert_eq!(observed.sessions[0].total.tokens, seat.tokens);
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_observer_unions_head_and_tail_models() {
    let root = rig("codex-head-tail");
    let store = root.join("codex");
    std::fs::create_dir_all(store.join("sessions/2026/09/08")).expect("Codex day");
    let mut rollout = vec![b'x'; 128 * 1024];
    rollout.extend_from_slice(
        br#"
{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
"#,
    );
    rollout.extend(std::iter::repeat_n(b'\n', 300 * 1024));
    rollout.extend_from_slice(
        br#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":10,"output_tokens":40,"reasoning_output_tokens":9}}}}
"#,
    );
    std::fs::write(
        store.join(format!(
            "sessions/2026/09/08/rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"
        )),
        rollout,
    )
    .expect("large rollout");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!(
            "schema=2\nseat.main=lead\nharness_session.main={CODEX_ID}\nagent_bin.main=codex\nconfig_home.main={}\n",
            store.display()
        ),
    )
    .expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats[0].coverage, Coverage::Read);
    assert_eq!(observed.sessions[0].seats[0].model, "gpt-5.6-luna");
    assert_eq!(observed.sessions[0].seats[0].tokens.input, 70);
    assert!(observed.sessions[0].seats[0].usd_micro.is_some());
    assert!(observed.sessions[0].seats[0].approximate);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn codex_head_usage_requires_tail_evidence_after_short_control() {
    let root = rig("codex-head-no-tail-count");
    let store = root.join("codex");
    let path = store.join(format!(
        "sessions/2026/09/08/rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"
    ));
    std::fs::create_dir_all(store.join("sessions/2026/09/08")).expect("Codex day");
    let head = br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":11,"cached_input_tokens":2,"cache_write_input_tokens":1,"output_tokens":3,"reasoning_output_tokens":1}}}}
"#;
    std::fs::write(&path, head).expect("short rollout");
    std::fs::write(root.join("sessions/live/meta"), format!("schema=2\nseat.main=lead\nharness_session.main={CODEX_ID}\nagent_bin.main=codex\nconfig_home.main={}\n", store.display())).expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let short = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(short.sessions[0].seats[0].coverage, Coverage::Read);
    assert!(short.sessions[0].seats[0].tokens.input > 0);
    let mut large = head.to_vec();
    while large.len() <= 512 * 1024 {
        large.extend_from_slice(
            br#"{"type":"event_msg","payload":{"type":"other"}}
"#,
        );
    }
    std::fs::write(path, large).expect("large rollout");
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats[0].coverage, Coverage::Truncated);
    assert_eq!(observed.sessions[0].seats[0].tokens, Tokens::default());
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_retire_events_collapse_without_making_totals_partial() {
    let root = rig("legacy-retire");
    std::fs::write(
        root.join("sessions/live/meta"),
        "schema=2\nseat.main=lead\nagent_bin.main=grok\n",
    )
    .expect("meta");
    std::fs::write(
        root.join("sessions/live/events.jsonl"),
        "{\"ts\":\"2026-09-10T09:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"aemenu1\"}\n\
         {\"ts\":\"2026-09-10T09:01:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"aemenu2\"}\n\
         {\"ts\":\"2026-09-10T09:02:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"aemenu3\"}\n",
    )
    .expect("legacy retire event");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats.len(), 1);
    assert!(!observed.sessions[0].total.partial);
    assert!(
        ae::usage::render(&observed, false)
            .contains("live  retired: 3 seats unlocated (legacy retire events)\n")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_summary_and_identified_retire_keep_distinct_coverage() {
    let root = rig("mixed-retire");
    std::fs::write(
        root.join("sessions/live/meta"),
        "schema=2\nseat.main=lead\nagent_bin.main=grok\n",
    )
    .expect("meta");
    std::fs::write(
        root.join("sessions/live/events.jsonl"),
        include_bytes!("../fixtures/usage/retired-events.jsonl"),
    )
    .expect("retire events");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats.len(), 2);
    assert_eq!(observed.sessions[0].seats[1].seat, "identified (retired)");
    assert!(matches!(
        observed.sessions[0].seats[1].coverage,
        Coverage::Unreadable(_)
    ));
    assert!(observed.sessions[0].total.partial);
    let json = ae::usage::render(&observed, true);
    assert!(json.contains("\"legacy_retired_unlocated\":3"), "{json}");
    let _ = std::fs::remove_dir_all(root);
}

/// A retire summary carrying a `profile@client` spelling is not attributed:
/// the roster grammar admits no `@`, so the seat stays unlocated rather than
/// attributing spend to a profile that was never recorded.
#[test]
fn an_at_spelling_in_a_retire_summary_stays_unlocated() {
    let root = rig("at-retire");
    std::fs::write(
        root.join("sessions/live/meta"),
        "schema=2\nseat.main=lead\nagent_bin.main=claude\n",
    )
    .expect("meta");
    std::fs::write(
        root.join("sessions/live/events.jsonl"),
        format!(
            "{{\"ts\":\"2026-09-10T09:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"past\",\"ref\":\"{RETIRED_ID}\",\"target_slot\":\"spawned.1\",\"summary\":\"tool=claude profile=fablex@cc-mic config_home={} config_home_base=\"}}\n",
            root.display()
        ),
    )
    .expect("retire event");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats.len(), 2);
    let seat = &observed.sessions[0].seats[1];
    assert_eq!(seat.seat, "past (retired)");
    assert_eq!(seat.tool, "?", "no attribution without a grammar profile");
    assert!(matches!(seat.coverage, Coverage::Unlocated));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn price_math_stays_integer_and_override_rows_refuse_malformed_text() {
    let price = prices::parse_override("1.25,2.5,0.125,10").expect("valid override");
    let tokens = Tokens {
        input: 1_000_000,
        cache_write: 1_000,
        cache_read: 10_000,
        output: 100,
    };
    assert_eq!(prices::cost(tokens, price), Some(1_254_750));
    assert_eq!(
        prices::cost(
            Tokens {
                input: 1,
                ..Tokens::default()
            },
            prices::parse_override("0.5,0,0,0").expect("fractional override")
        ),
        Some(1),
        "half a micro-dollar rounds up"
    );
    assert_eq!(
        prices::cost(
            Tokens {
                input: 3,
                ..Tokens::default()
            },
            prices::parse_override("0.2,0,0,0").expect("tiny rate")
        ),
        Some(1),
        "tiny entries are summed before the one division"
    );
    assert_eq!(
        prices::cost(
            Tokens {
                input: u64::MAX,
                ..Tokens::default()
            },
            prices::parse_override("1,0,0,0").expect("unit override")
        ),
        Some(u64::MAX),
        "multiplication is wider than u64"
    );
    assert!(prices::parse_override("1,wat,3,4").is_err());
    assert!(prices::parse_row("sol = gpt-5.6-sol,1,2,3").is_err());
}

#[test]
fn observer_reads_each_supported_store_and_propagates_unsupported_coverage() {
    let root = rig("observe");
    let claude_home = root.join("claude home");
    let codex_home = root.join("codex-home");
    std::fs::create_dir_all(claude_home.join("projects/work")).expect("Claude project");
    std::fs::create_dir_all(codex_home.join("sessions/2026/09/08")).expect("Codex day");
    std::fs::write(
        claude_home.join(format!("projects/work/{CLAUDE_ID}.jsonl")),
        include_bytes!("../fixtures/usage/claude-main.jsonl"),
    )
    .expect("Claude transcript");
    std::fs::write(
        claude_home.join(format!("projects/work/{RETIRED_ID}.jsonl")),
        include_bytes!("../fixtures/usage/claude-subagent.jsonl"),
    )
    .expect("retired Claude transcript");
    std::fs::write(
        codex_home.join(format!(
            "sessions/2026/09/08/rollout-2026-09-08T09-00-00-{CODEX_ID}.jsonl"
        )),
        String::from_utf8_lossy(include_bytes!("../fixtures/usage/codex-rollout.jsonl"))
            .replace("gpt-5.6-luna", "gpt-5.6-sol"),
    )
    .expect("Codex rollout");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=fable5\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main={}\nseat.worker.0=colead\nprofile.worker.0=sol\nharness_session.worker.0={CODEX_ID}\nagent_bin.worker.0=codex\nconfig_home.worker.0={}\nseat.spawned.0=other\nprofile.spawned.0=grok46\nagent_bin.spawned.0=grok\n",
            claude_home.display(),
            codex_home.display(),
        ),
    )
    .expect("meta");
    std::fs::write(
        root.join("sessions/live/events.jsonl"),
        format!(
            "{{\"ts\":\"2026-09-10T09:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"past\",\"ref\":\"{RETIRED_ID}\",\"target_slot\":\"spawned.1\",\"summary\":\"tool=claude profile=fable5 config_home={} config_home_base=\"}}\n{{\"ts\":\"2026-09-10T09:01:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"lost\",\"ref\":\"not-a-uuid\",\"target_slot\":\"spawned.2\",\"summary\":\"tool=claude profile=fable5 config_home={} config_home_base=\"}}\n",
            claude_home.display(),
            claude_home.display()
        ),
    )
    .expect("retire event");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    let rows = &observed.sessions[0].seats;
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].coverage, Coverage::Read);
    assert_eq!(rows[0].tokens.input, 120);
    assert_eq!(rows[1].tokens.input, 70);
    assert!(rows[1].approximate);
    assert_eq!(rows[2].coverage, Coverage::Unsupported);
    assert_eq!(rows[3].seat, "past (retired)");
    assert!(rows[3].retired);
    assert_eq!(rows[3].coverage, Coverage::Read);
    assert_eq!(rows[4].seat, "lost (retired)");
    assert_eq!(rows[4].coverage, Coverage::Unlocated);
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn typed_unreadable_config_home_is_partial_and_never_falls_back() {
    let root = rig("typed-home");
    std::fs::create_dir_all(root.join(".claude/projects/work")).expect("tempting legacy default");
    std::fs::write(
        root.join(format!(".claude/projects/work/{CLAUDE_ID}.jsonl")),
        include_bytes!("../fixtures/usage/claude-main.jsonl"),
    )
    .expect("default transcript");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!("schema=2\nseat.main=lead\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main=unknown\n"),
    )
    .expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert!(matches!(
        observed.sessions[0].seats[0].coverage,
        Coverage::Unreadable(_)
    ));
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn implicit_claude_home_uses_the_recorded_store_not_its_base() {
    let root = rig("implicit-home");
    let base = root.join("base");
    let store = base.join(".claude");
    std::fs::create_dir_all(store.join("projects/work")).expect("recorded Claude store");
    std::fs::write(
        store.join(format!("projects/work/{CLAUDE_ID}.jsonl")),
        include_bytes!("../fixtures/usage/claude-main.jsonl"),
    )
    .expect("transcript");
    std::fs::write(
        root.join("sessions/live/meta"),
        format!(
            "schema=2\nseat.main=lead\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main=implicit:{}\nconfig_home_base.main={}\n",
            store.display(),
            base.display()
        ),
    )
    .expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(observed.sessions[0].seats[0].coverage, Coverage::Read);
    assert_eq!(observed.sessions[0].seats[0].tokens.input, 120);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn claude_intermediate_symlink_never_combines_outside_sidechain_usage() {
    let root = rig("claude-sidechain-symlink");
    let store = root.join("claude");
    let project = store.join("projects/work");
    let uuid_dir = project.join(CLAUDE_ID);
    let outside = root.join("outside");
    std::fs::create_dir_all(project.join(CLAUDE_ID).join("subagents")).expect("sidechain");
    std::fs::create_dir_all(outside.join("subagents")).expect("outside");
    std::fs::write(
        project.join(format!("{CLAUDE_ID}.jsonl")),
        include_bytes!("../fixtures/usage/claude-main.jsonl"),
    )
    .expect("parent");
    std::fs::write(uuid_dir.join("subagents/control.jsonl"), br#"{"type":"assistant","message":{"id":"control","model":"claude-fable-5-1-20260901","usage":{"input_tokens":7}}}
"#).expect("control");
    std::fs::write(outside.join("subagents/outside.jsonl"), br#"{"type":"assistant","message":{"id":"outside","model":"claude-fable-5-1-20260901","usage":{"input_tokens":999}}}
"#).expect("outside transcript");
    std::fs::write(root.join("sessions/live/meta"), format!("schema=2\nseat.main=lead\nharness_session.main={CLAUDE_ID}\nagent_bin.main=claude\nconfig_home.main={}\n", store.display())).expect("meta");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let control = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert_eq!(control.sessions[0].seats[0].tokens.input, 127);
    std::fs::remove_dir_all(&uuid_dir).expect("replace UUID dir");
    std::os::unix::fs::symlink(&outside, &uuid_dir).expect("intermediate symlink");
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert!(matches!(
        observed.sessions[0].seats[0].coverage,
        Coverage::Unreadable(_)
    ));
    assert_eq!(observed.sessions[0].seats[0].tokens, Tokens::default());
    assert!(observed.sessions[0].total.partial);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn price_config_uses_alias_rows_and_refuses_duplicate_models() {
    let root = rig("prices");
    let good = root.join("good");
    std::fs::write(&good, "[prices]\nsol = gpt-5.6-sol,1.25,2.5,0.125,10\n").expect("config");
    assert_eq!(
        prices::read(Some(&good), None)
            .expect("valid prices")
            .price("gpt-5.6-sol")
            .map(|price| price.input),
        Some(1_250_000)
    );
    let duplicate = root.join("duplicate");
    std::fs::write(
        &duplicate,
        "[prices]\na = model-x,1,2,3,4\nb = model-x,4,3,2,1\n",
    )
    .expect("config");
    let error = prices::read(Some(&duplicate), None).expect_err("duplicate model refused");
    assert_eq!(error.exit_code(), 2);
    assert!(error.to_string().contains("both 'a' and 'b'"));
    assert!(prices::parse_row("x = bad\u{1b}model,1,2,3,4").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn usage_arguments_and_live_selection_refuse_unknown_sessions() {
    let parsed =
        ae::usage::parse_args(&["beta".to_owned(), "--json".to_owned(), "alpha".to_owned()])
            .expect("valid usage arguments");
    assert!(parsed.json);
    assert_eq!(parsed.sessions, ["beta", "alpha"]);
    assert!(ae::usage::parse_args(&["--wat".to_owned()]).is_err());

    let live = vec![
        SessionInput {
            name: "alpha".to_owned(),
            path: "/sessions/alpha".into(),
        },
        SessionInput {
            name: "beta".to_owned(),
            path: "/sessions/beta".into(),
        },
    ];
    assert_eq!(
        ae::usage::select_sessions(&live, &parsed.sessions)
            .expect("both requested sessions are live")
            .iter()
            .map(|session| session.name.as_str())
            .collect::<Vec<_>>(),
        ["beta", "alpha"]
    );
    assert_eq!(
        ae::usage::select_sessions(&live, &["gone".to_owned()])
            .expect_err("unknown session is refused"),
        "gone"
    );
}

#[test]
fn usage_table_and_json_pin_partial_coverage() {
    let observation = Observation {
        sessions: vec![SessionUsage {
            name: "demo".to_owned(),
            seats: vec![
                SeatUsage {
                    seat: "lead".to_owned(),
                    slot: "main".to_owned(),
                    tool: "codex".to_owned(),
                    model: "gpt-5.6-sol".to_owned(),
                    tokens: Tokens {
                        input: 1_234_567,
                        cache_write: 89_000,
                        cache_read: 42,
                        output: 7_000,
                    },
                    usd_micro: Some(1_250_000),
                    observed_at: Some(1_788_858_540),
                    retired: false,
                    coverage: Coverage::Read,
                    approximate: false,
                },
                SeatUsage {
                    seat: "worker (retired)".to_owned(),
                    slot: "spawned.0".to_owned(),
                    tool: "claude".to_owned(),
                    model: "?".to_owned(),
                    tokens: Tokens::default(),
                    usd_micro: None,
                    observed_at: None,
                    retired: true,
                    coverage: Coverage::Unreadable("transcript not found".to_owned()),
                    approximate: false,
                },
            ],
            total: UsageTotal {
                tokens: Tokens {
                    input: 1_234_567,
                    cache_write: 89_000,
                    cache_read: 42,
                    output: 7_000,
                },
                usd_micro: 1_250_000,
                partial: true,
            },
            retired_scan_truncated: false,
            legacy_retired_unlocated: 0,
            meta_scan_failure: None,
            retired_scan_failure: None,
        }],
        unpriced: Vec::new(),
        now: 1_788_858_600,
    };
    assert_eq!(
        ae::usage::render(&observation, false),
        include_str!("../fixtures/usage/expected.stdout")
    );
    let json = ae::usage::render(&observation, true);
    assert!(json.contains("\"coverage\":\"unreadable\""), "{json}");
    assert!(
        json.contains("\"coverage_reason\":\"transcript not found\""),
        "{json}"
    );
    assert!(json.contains("\"input_tokens\":1234567"), "{json}");
    assert!(json.contains("\"usd_micro\":1250000"), "{json}");
    assert!(json.contains("\"partial\":true"), "{json}");
}

#[test]
fn public_usage_names_an_unknown_live_session_and_refuses_bad_prices() {
    let root = rig("public-errors");
    let unknown = crate::cli::ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", root.join("config"))
        .current_dir(&root)
        .args(["usage", "gone"])
        .output()
        .expect("usage command");
    assert_eq!(unknown.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("live session not found: gone"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );

    std::fs::write(
        root.join("config"),
        "[prices]\na = same,1,2,3,4\nb = same,4,3,2,1\n",
    )
    .expect("malformed selected config");
    let malformed = crate::cli::ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", root.join("config"))
        .current_dir(&root)
        .arg("usage")
        .output()
        .expect("usage command");
    assert_eq!(malformed.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&malformed.stderr).contains("both 'a' and 'b'"),
        "{}",
        String::from_utf8_lossy(&malformed.stderr)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn usage_helper_reads_only_its_own_session() {
    let root = rig("helper");
    std::fs::write(root.join("config"), "[workspace]\nmain = lead\n").expect("global config");
    std::fs::write(
        root.join("sessions/live/meta"),
        "schema=2\nsession=live\nseat.main=lead\nprofile.main=grok46\nagent_bin.main=grok\n",
    )
    .expect("session meta");
    let before = crate::cli::byte_tree(&root);
    let output = crate::cli::ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", root.join("config"))
        .current_dir(&root)
        .args([
            "_usage",
            root.join("sessions/live").to_str().expect("session path"),
        ])
        .output()
        .expect("usage helper");
    assert!(output.status.success(), "{:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line
            .split_whitespace()
            .take(5)
            .eq(["live", "lead", "grok", "n/a", "n/a"])),
        "{stdout}"
    );
    assert_eq!(crate::cli::byte_tree(&root), before, "helper wrote state");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_oversized_events_scan_is_explicitly_truncated_and_partial() {
    let root = rig("events-cap");
    std::fs::write(
        root.join("sessions/live/meta"),
        "schema=2\nseat.main=other\nagent_bin.main=grok\n",
    )
    .expect("meta");
    std::fs::write(
        root.join("sessions/live/events.jsonl"),
        vec![b'x'; 4 * 1024 * 1024 + 1],
    )
    .expect("oversized events");
    let sessions = [SessionInput {
        name: "live".to_owned(),
        path: root.join("sessions/live"),
    }];
    let observed = ae::usage::observe(&Inputs {
        home: Some(&root),
        sessions: &sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    assert!(observed.sessions[0].retired_scan_truncated);
    assert!(observed.sessions[0].total.partial);
    assert!(
        ae::usage::render(&observed, false)
            .contains("retired seats: unread (events scan truncated)")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn meta_and_events_read_failures_are_visible_and_partial_but_missing_events_are_not() {
    let meta_root = rig("meta-read-failure");
    std::fs::create_dir_all(meta_root.join("sessions/live/meta")).expect("invalid meta directory");
    std::fs::write(
        meta_root.join("sessions/live/events.jsonl"),
        format!(
            "{{\"ts\":\"2026-09-10T09:00:00Z\",\"actor\":\"lead\",\"action\":\"retire\",\"target\":\"past\",\"ref\":\"{RETIRED_ID}\",\"target_slot\":\"spawned.1\",\"summary\":\"tool=grok profile=grok46 config_home= config_home_base=\"}}\n"
        ),
    )
    .expect("retire event");
    let meta_sessions = [SessionInput {
        name: "live".to_owned(),
        path: meta_root.join("sessions/live"),
    }];
    let meta_observed = ae::usage::observe(&Inputs {
        home: Some(&meta_root),
        sessions: &meta_sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    let meta_table = ae::usage::render(&meta_observed, false);
    assert!(meta_observed.sessions[0].total.partial);
    assert!(meta_observed.sessions[0].meta_scan_failure.is_some());
    assert!(
        meta_table.contains("live  seats: unread (session meta scan failed:"),
        "{meta_table}"
    );
    assert!(
        ae::usage::render(&meta_observed, true).contains("\"meta_scan_failure\":"),
        "meta failure missing from JSON"
    );

    let events_root = rig("events-read-failure");
    std::fs::write(
        events_root.join("sessions/live/meta"),
        "schema=2\nseat.main=other\nagent_bin.main=grok\n",
    )
    .expect("meta");
    std::fs::create_dir_all(events_root.join("sessions/live/events.jsonl"))
        .expect("invalid events directory");
    let events_sessions = [SessionInput {
        name: "live".to_owned(),
        path: events_root.join("sessions/live"),
    }];
    let events_observed = ae::usage::observe(&Inputs {
        home: Some(&events_root),
        sessions: &events_sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    let events_table = ae::usage::render(&events_observed, false);
    assert!(events_observed.sessions[0].total.partial);
    assert!(events_observed.sessions[0].retired_scan_failure.is_some());
    assert!(
        events_table.contains("live  retired seats: unread (events scan failed:"),
        "{events_table}"
    );
    assert!(
        ae::usage::render(&events_observed, true).contains("\"retired_scan_failure\":"),
        "events failure missing from JSON"
    );

    let missing_root = rig("events-missing");
    std::fs::write(
        missing_root.join("sessions/live/meta"),
        "schema=2\nseat.main=other\nagent_bin.main=grok\n",
    )
    .expect("meta");
    let missing_sessions = [SessionInput {
        name: "live".to_owned(),
        path: missing_root.join("sessions/live"),
    }];
    let missing_observed = ae::usage::observe(&Inputs {
        home: Some(&missing_root),
        sessions: &missing_sessions,
        prices: &prices::Book::default(),
        now: 1_788_858_600,
    });
    let missing_table = ae::usage::render(&missing_observed, false);
    assert!(!missing_observed.sessions[0].total.partial);
    assert!(missing_observed.sessions[0].retired_scan_failure.is_none());
    assert!(!missing_table.contains("scan failed"), "{missing_table}");

    let _ = std::fs::remove_dir_all(meta_root);
    let _ = std::fs::remove_dir_all(events_root);
    let _ = std::fs::remove_dir_all(missing_root);
}

/// #246 `ae usage --hourly`, frozen acceptance spec (usagehour-nav). Every oracle
/// is a hand sum over the synthetic transcripts written here, never the reducer.
/// Clock 2026-09-08T12:30:00Z: the six buckets are 07:00 … 12:00 (open).
mod hourly_spec {
    use super::{CLAUDE_ID, CODEX_ID, RETIRED_ID, SessionInput, rig};
    use ae::json::Value;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    const NOW: i64 = 1_788_870_600;
    const OLDEST: i64 = 1_788_850_800;
    const MIB: u64 = 1024 * 1024;
    const DEFAULT_HELPER: &str =
        "session  seat   tool  model  in   cache-w  cache-r  out  usd   age
live     lead   grok  n/a    n/a  n/a      n/a      n/a  n/a   n/a
live     total  -     -      0    0        0        0    0.00  -
fleet    total  -     -      0    0        0        0    0.00  -
";
    const DEFAULT_JSON: &str = r#""sessions":[],"fleet_input_tokens":0,"fleet_cache_write_tokens":0,"fleet_cache_read_tokens":0,"fleet_output_tokens":0,"fleet_usd_micro":0,"fleet_partial":false,"unpriced":[]}"#;
    const ASSISTANT: &str = r#"{"type":"assistant",@"requestId":"req-$0","message":{"id":"$0","model":"claude-fable-5-1","usage":{"input_tokens":$2,"cache_creation_input_tokens":$3,"cache_read_input_tokens":$4,"output_tokens":$5}}}"#;
    const COUNTER: &str = r#"{@"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":$1,"cached_input_tokens":$2,"output_tokens":$3,"reasoning_output_tokens":$3,"total_tokens":$7},"last_token_usage":{"input_tokens":$4,"cached_input_tokens":$5,"output_tokens":$6,"reasoning_output_tokens":$6,"total_tokens":$8}}}}"#;
    const MARK: &str = r#"{@"type":"$0","payload":{"model":"gpt-6.1-sol"},"message":{"role":"user","content":"go"}}"#;

    /// Fixture rows, one record each. Claude `id stamp input cache_write cache_read
    /// output`; Codex `stamp input cached output last_input last_cached last_output`
    /// (input includes cached; reasoning = output, so adding it is caught);
    /// `user|turn_context stamp`; `-` = no stamp; any other row is copied verbatim.
    fn rows(text: &str) -> String {
        let mut out = String::new();
        for row in text.lines().map(str::trim).filter(|row| !row.is_empty()) {
            let mut words: Vec<String> = row.split(' ').map(str::to_owned).collect();
            let number = |word: &String| word.parse::<u64>().expect("count");
            let (template, at) = match words.len() {
                6 => (ASSISTANT, 1),
                7 => {
                    let total = number(&words[1]) + number(&words[3]);
                    let last = number(&words[4]) + number(&words[6]);
                    words.extend([total.to_string(), last.to_string()]);
                    (COUNTER, 0)
                }
                2 if words[0] == "user" || words[0] == "turn_context" => (MARK, 1),
                _ => (row, usize::MAX),
            };
            let stamp = match words.get(at) {
                Some(at) if at != "-" => format!(r#""timestamp":"2026-09-08T{at}","#),
                _ => String::new(),
            };
            let mut record = template.replacen('@', &stamp, 1);
            for (index, word) in words.iter().enumerate().filter(|_| at != usize::MAX) {
                record = record.replace(&format!("${index}"), word);
            }
            out += &record;
            out.push('\n');
        }
        out
    }

    /// A 2 MiB but valid record: no line cap applies to an in-memory tail.
    fn padded(record: &str) -> String {
        let pad = format!(r#"{{"pad":"{}","#, "a".repeat(2 * 1024 * 1024));
        record.replacen('{', &pad, 1)
    }

    fn id(n: u8) -> String {
        format!("0199c0de-1234-4890-abcd-ef01234567{n:02x}")
    }

    fn world(tag: &str) -> PathBuf {
        let root = rig(tag);
        std::fs::create_dir_all(root.join("claude/projects/work")).expect("Claude store");
        std::fs::create_dir_all(root.join("codex/sessions/2026/09/08")).expect("Codex day");
        root
    }

    fn claude_file(root: &Path, id: &str) -> PathBuf {
        root.join(format!("claude/projects/work/{id}.jsonl"))
    }

    fn codex_file(root: &Path, id: &str) -> PathBuf {
        root.join(format!(
            "codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
        ))
    }

    fn seat(root: &Path, slot: &str, name: &str, bin: &str, id: &str) -> String {
        let home = root
            .join(if bin == "codex" { "codex" } else { "claude" })
            .display()
            .to_string();
        format!(
            "seat.{slot}={name}\nprofile.{slot}=p\nharness_session.{slot}={id}\nagent_bin.{slot}={bin}\nconfig_home.{slot}={home}\n"
        )
    }

    fn write(path: &Path, body: &str) {
        std::fs::write(path, body).expect("fixture");
    }

    /// `head`, a NUL hole, then a newline and `tail` as the last bytes: a 64 MiB
    /// tail read starts inside the hole, so only `tail` can be seen.
    fn sparse(path: &Path, head: &str, hole: u64, tail: &str) {
        write(path, head);
        let file = std::fs::OpenOptions::new().append(true).open(path);
        let mut file = file.expect("append");
        let len = u64::try_from(head.len()).expect("head") + hole;
        file.set_len(len).expect("hole");
        file.write_all(format!("\n{tail}").as_bytes())
            .expect("tail");
    }

    fn observe(root: &Path, seats: &str) -> (Vec<Value>, String) {
        let meta = format!("schema=2\n{seats}");
        write(&root.join("sessions/live/meta"), &meta);
        let path = root.join("sessions/live");
        let sessions = [SessionInput {
            name: "live".to_owned(),
            path,
        }];
        let inputs = ae::usage::HourlyInputs {
            home: Some(root),
            sessions: &sessions,
            now: NOW,
        };
        let observed = ae::usage::observe_hourly(&inputs);
        let table = ae::usage::hourly::render(&observed, false);
        let json = ae::usage::hourly::render(&observed, true);
        let wide = table.lines().any(|line| line.chars().count() > 100);
        assert!(!wide, "{table}");
        assert!(!format!("{table}{json}").contains("usd"), "{json}");
        let json = ae::json::parse(&json).expect("hourly JSON");
        assert_eq!(num(json.get("as_of")), u64::try_from(NOW).ok());
        let open = u64::try_from(OLDEST + 5 * 3600).ok();
        assert_eq!(num(json.get("open_hour")), open);
        let _ = list(json.get("exclusions"));
        let session = &list(json.get("sessions"))[0];
        assert_eq!(session.get_str("name"), Some("live"));
        (list(session.get("seats")).to_vec(), table)
    }

    fn list(value: Option<&Value>) -> &[Value] {
        let Some(Value::Arr(items)) = value else {
            panic!("not an array: {value:?}")
        };
        items
    }

    fn num(value: Option<&Value>) -> Option<u64> {
        let Some(Value::Num(number)) = value else {
            return None;
        };
        u64::try_from(*number).ok()
    }

    fn named<'a>(seats: &'a [Value], name: &str) -> &'a Value {
        let hit = |seat: &&Value| seat.get_str("seat").is_some_and(|s| s.starts_with(name));
        let found = seats.iter().find(hit);
        found.unwrap_or_else(|| panic!("no {name} seat in {seats:?}"))
    }

    /// Each hour as the oracle spells it — `n` complete, `n+?` partial, `?`
    /// nothing known, `~` approximate — after checking the JSON contract, which
    /// makes every Codex hour approximate.
    fn cells(seat: &Value) -> String {
        let hours = list(seat.get("hours"));
        assert_eq!(hours.len(), 6, "{seat:?}");
        let tool = seat.get_str("tool");
        let ids = seat.get_str("slot").is_some() && matches!(tool, Some("claude" | "codex"));
        let flag = matches!(seat.get("retired"), Some(Value::Bool(_)));
        let counts = num(seat.get("unbucketed_tokens")).and(num(seat.get("predecessors_not_read")));
        let reason = seat.get("coverage_reason").is_some();
        assert!(ids && flag && reason && counts.is_some(), "{seat:?}");
        let mut out = Vec::new();
        for (index, hour) in (0_i64..).zip(hours) {
            let start = u64::try_from(OLDEST + index * 3600).ok();
            assert_eq!(num(hour.get("start")), start);
            assert_eq!(hour.get("open"), Some(&Value::Bool(index == 5)));
            let coded = |code: Option<&str>| CODES.contains(&format!(" {} ", code.unwrap_or("-")));
            let reasons = list(hour.get("reasons"));
            assert!(reasons.iter().map(Value::as_str).all(coded), "{seat:?}");
            let known = num(hour.get("known_tokens")).expect("known_tokens");
            assert!(matches!(hour.get("approximate"), Some(Value::Bool(_))));
            let approximate = hour.get("approximate") == Some(&Value::Bool(true));
            assert!(approximate || tool != Some("codex"), "{seat:?}");
            let tilde = ["", "~"][usize::from(approximate)];
            out.push(match (hour.get_str("coverage"), hour.get("tokens")) {
                (Some("complete"), tokens) if num(tokens) == Some(known) => {
                    format!("{known}{tilde}")
                }
                (Some("partial"), Some(Value::Null)) if known > 0 => format!("{known}{tilde}+?"),
                (Some("unknown"), Some(Value::Null)) if known == 0 => "?".to_owned(),
                other => panic!("hour {index} breaks the coverage contract: {other:?} {known}"),
            });
        }
        out.join(" ")
    }

    fn row(table: &str, label: &str) -> String {
        let first = |line: &&str| line.split_whitespace().next() == Some(label);
        let line = table.lines().find(first);
        let line = line.unwrap_or_else(|| panic!("no {label} in\n{table}"));
        let words: Vec<_> = line.split_whitespace().skip(1).collect();
        words.join(" ")
    }

    const UNKNOWN: &str = "? ? ? ? ? ?";
    const CODES: &str = " before_tail gap future budget unreadable conflict unbucketed ";

    // Catches: mtime/local zone/rolling window/.999999 up/+02:00 ignored/future
    // counted; snapshots summed, equal repeat moved, vector conflict hidden, replay
    // recharged; unstamped guessed; id-less summed; 1 MiB cap; whole-window taint.
    #[test]
    fn claude_whole_files_bucket_by_utc_stamp_once_per_request() {
        let root = world("hourly-claude");
        let clock = rows(
            "a 11:59:59.999999Z 1 0 0 0
            b 12:00:00Z 2 0 0 0
            c 12:30:00+02:00 0 4 0 0
            d 09:00:00.5Z 0 0 8 0
            e 12:30:01Z 16 0 0 0
            f 06:59:59Z 32 0 0 0
            g 12:30:00.9Z 0 0 0 64",
        );
        let snapshots = rows(
            "grow 10:59:50Z 100 0 0 0
            rep 09:59:59Z 10 5 5 10
            grow 11:00:10Z 150 0 0 0
            rep 10:00:01Z 10 5 5 10
            late 08:00:30Z 7 0 0 0
            late 07:59:50Z 7 0 0 0
            tie 09:10:00Z 5 0 0 0
            tie 09:20:00Z 0 5 0 0",
        );
        let unstamped = rows("t 10:10:00Z 5 0 0 0\nu - 9 0 0 0");
        let in_flight =
            r#"{"type":"assistant","timestamp":"2026-09-08T12:10:00Z","message":{"id":"z""#;
        let malformed = rows("p 08:10:00Z 1 0 0 0\n{not json\nq 09:20:00Z 2 0 0 0")
            + &padded(&rows("big 10:30:00Z 8 0 0 0"))
            + &rows("r 11:10:00Z 4 0 0 0")
            + in_flight;
        let anonymous = rows("x 10:30:00Z 50 0 0 0").replace(r#""id":"x","#, "")
            + &rows("v 10:40:00Z 60 0 0 0").replace(r#""requestId":"req-v","#, "");
        let noid = rows("k 10:10:00Z 1 0 0 0") + &anonymous + &rows("m 10:50:00Z 2 0 0 0");
        let mut roster = String::new();
        let bodies = [
            ("clock", clock),
            ("snapshots", snapshots),
            ("unstamped", unstamped),
            ("malformed", malformed),
            ("noid", noid),
        ];
        for (n, (name, body)) in (0xa0..).zip(bodies) {
            write(&claude_file(&root, &id(n)), &body);
            let slot = format!("spawned.{}", n - 0xa0);
            roster += &seat(&root, &slot, name, "claude", &id(n));
        }
        let subagents = root.join(format!("claude/projects/work/{}/subagents", id(0xa1)));
        std::fs::create_dir_all(&subagents).expect("subagents");
        let side = rows("side 11:30:00Z 0 0 0 3\nrep 10:00:01Z 10 5 5 10");
        write(&subagents.join("agent-1.jsonl"), &side);
        let (seats, table) = observe(&root, &roster);
        let clock = named(&seats, "clock");
        assert_eq!(cells(clock), "0 0 8 4 1 66+?");
        let reasons = list(list(clock.get("hours"))[5].get("reasons"));
        assert!(reasons.contains(&Value::str("future")), "{reasons:?}");
        assert_eq!(row(&table, "live:clock"), "0 0 8 4 1 66+?");
        assert!(table.contains("UTC"), "{table}");
        let now = table.lines().any(|line| line.trim_end().ends_with(" now"));
        assert!(now, "{table}");
        assert_eq!(cells(named(&seats, "snapshots")), "7 0 35~ 0 153 0");
        let unstamped = named(&seats, "unstamped");
        assert_eq!(cells(unstamped), "0 0 0 5+? ? ?");
        assert_eq!(num(unstamped.get("unbucketed_tokens")), Some(9));
        assert!(table.contains("unbucketed"), "{table}");
        assert_eq!(cells(named(&seats, "malformed")), "0 1+? 2+? 8 4 0");
        assert_eq!(cells(named(&seats, "noid")), "0 0 0 3+? 0 0");
        let _ = std::fs::remove_dir_all(root);
    }

    // Catches: a read past the 64 MiB tail, a cut first record consumed, a 16 MiB refusal,
    // absent history as 0, a G=600 off-by-one per tool, a turn_context witness, a lifetime charge.
    #[test]
    fn tails_are_complete_only_behind_the_witness_and_codex_baselines() {
        let root = world("hourly-tails");
        // The 64 MiB tail starts on the `{` of `trap`: a valid record whose line
        // began one byte earlier, so it must be skipped as a partial first line.
        let cut = |path: &Path, trap: &str, tail: &str| {
            let hole = 64 * MIB - u64::try_from(trap.len() + 1 + tail.len()).expect("len");
            sparse(path, &format!("a{trap}"), hole, tail);
        };
        let codex = |n: u8| format!("01a08046-1974-7352-ade3-81a7862007a{n}");
        let trap = rows("old 08:20:00Z 1000 0 0 0");
        let counter = rows("08:20:00Z 800 0 0 800 0 0");
        for (n, witness) in [(0, "08:50:00Z"), (1, "08:50:01Z")] {
            let tail = format!("user {witness}\nin 09:10:00Z 5 0 0 0\non 11:00:00Z 7 0 0 0");
            cut(&claude_file(&root, &id(0xa0 + n)), &trap, &rows(&tail));
            let chain = format!("{witness} 900 0 0 40 0 0\n09:10:00Z 905 0 0 5 0 0");
            let chain = rows(&format!("{chain}\n11:00:00Z 912 0 0 7 0 0"));
            cut(&codex_file(&root, &codex(n)), &counter, &chain);
        }
        let tail =
            rows("turn_context 08:40:00Z\n09:20:00Z 900 0 100 40 0 0\n10:10:00Z 930 0 110 30 0 10");
        let head = rows("08:00:00Z 5000 0 0 5000 0 0");
        sparse(&codex_file(&root, CODEX_ID), &head, 64 * MIB, &tail);
        let roster = seat(&root, "main", "exact", "claude", &id(0xa0))
            + &seat(&root, "worker.0", "baseline", "codex", CODEX_ID)
            + &seat(&root, "spawned.0", "short", "claude", &id(0xa1))
            + &seat(&root, "spawned.1", "cexact", "codex", &codex(0))
            + &seat(&root, "spawned.2", "cshort", "codex", &codex(1));
        let (seats, _) = observe(&root, &roster);
        assert_eq!(cells(named(&seats, "exact")), "? ? 5 0 7 0");
        assert_eq!(cells(named(&seats, "short")), "? ? 5+? 0 7 0");
        assert_eq!(cells(named(&seats, "cexact")), "? ? 5~ 0~ 7~ 0~");
        assert_eq!(cells(named(&seats, "cshort")), "? ? 5~+? 0~ 7~ 0~");
        assert_eq!(cells(named(&seats, "baseline")), "? ? ? 40~ 0~ 0~");
        let _ = std::fs::remove_dir_all(root);
    }

    // Catches: summed counters/last_usage, reasoning or cache twice, a clamped or
    // unmarked reset; inherited baseline charged, unstamped swept, malformed read as
    // 0, non-counter lines tainting, width or spelling drift past 1e9.
    #[test]
    fn codex_charges_counter_deltas_to_receipt_hours_and_names_gaps() {
        let root = world("hourly-codex");
        let chain = rows(
            "turn_context 09:00:00Z
            09:10:00Z 60 20 40 60 20 40
            10:20:00Z 80 20 60 20 0 20
            10:30:00Z 80 20 60 20 0 20
            11:40:00Z 900 20 140 820 0 80
            11:45:00Z 15 0 5 15 0 5
            12:10:00Z 35 0 15 20 0 10",
        );
        write(&codex_file(&root, CODEX_ID), &chain);
        let gaps_id = "01a08046-1974-7352-ade3-81a786200796";
        let overlong =
            r#"{"timestamp":"2026-09-08T12:05:00Z","type":"response_item","payload":{}}"#;
        let gaps = rows(
            r#"09:10:00Z 900000000 0 100000000 40 0 0
            - 980000000 0 120000000 40 0 0
            10:30:00Z 1020000000 0 130000000 40 0 0
            {"timestamp":"2026-09-08T11:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":"x","output_tokens":1}}}}
            11:20:00Z 1060000000 0 140000000 40 0 0
            {garbage"#,
        ) + &padded(overlong)
            + "\n"
            + &rows(
                "12:10:00Z 1110000000 0 150000000 40 0 0\n12:20:00Z 3100000000 0 160000000 40 0 0",
            );
        write(&codex_file(&root, gaps_id), &gaps);
        let long = "seat-name-of-twenty-seven-c";
        let roster = seat(&root, "worker.0", "colead", "codex", CODEX_ID)
            + &seat(&root, "spawned.0", long, "codex", gaps_id);
        let (seats, table) = observe(&root, &roster);
        let chain = named(&seats, "colead");
        assert_eq!(cells(chain), "0~ 0~ 100~ 40~ 900~+? 30~");
        assert_eq!(row(&table, "live:colead"), "0~ 0~ 100~ 40~ 900~+? 30~");
        let gaps = named(&seats, long);
        assert_eq!(cells(gaps), "? ? ? 50000000~+? ? 2060000000~");
        assert_eq!(num(gaps.get("unbucketed_tokens")), Some(100_000_000));
        let label = format!("live:{long}");
        assert_eq!(label.len(), 32);
        let cells = row(&table, &label);
        let (head, last) = cells.rsplit_once(' ').expect("cells");
        assert_eq!(head, "? ? ? 50.0M~+? ?");
        let science = last.strip_suffix('~').expect("approximate");
        assert!(science.contains('e') && science.len() <= 6, "{science}");
        assert!(table.contains("subagent"), "{table}");
        let _ = std::fs::remove_dir_all(root);
    }

    // Catches: predecessor read, resume clipped at launch, symlink followed, shared
    // transcript double counted, same owner twice, stores merged by UUID, failed
    // parent's child complete, retired dropped.
    #[test]
    fn owners_lineage_and_links_decide_what_is_read() {
        let root = world("hourly-owners");
        let files = [
            (CLAUDE_ID.to_owned(), "x 11:10:00Z 5 0 0 0"),
            (RETIRED_ID.to_owned(), "z 10:10:00Z 3 0 0 0"),
            (id(0xa0), "n 10:10:00Z 5 0 0 0"),
            (id(0xa1), "o 11:20:00Z 1000 0 0 0"),
            (id(0xa2), "y 09:10:00Z 6 0 0 0"),
            (id(0xa4), "l 10:10:00Z 2 0 0 0"),
        ];
        for (id, body) in files {
            write(&claude_file(&root, &id), &rows(body));
        }
        let outside = root.join("outside.jsonl");
        write(&outside, &rows("out 11:00:00Z 77 0 0 0"));
        std::os::unix::fs::symlink(&outside, claude_file(&root, &id(0xa3))).expect("symlink");
        let child = root.join(format!("claude/projects/work/{}/subagents", id(0xa3)));
        std::fs::create_dir_all(&child).expect("child");
        write(&child.join("agent-1.jsonl"), &rows("kid 11:00:00Z 9 0 0 0"));
        let other = root.join("claude2/projects/work");
        std::fs::create_dir_all(&other).expect("second store");
        let right = other.join(format!("{}.jsonl", id(0xa4)));
        write(&right, &rows("r 10:20:00Z 4 0 0 0"));
        let home = root.join("claude").display().to_string();
        let retire = |target: &str, id: &str, slot: &str| {
            format!(
                r#"{{"ts":"2026-09-08T10:00:00Z","actor":"lead","action":"retire","target":"{target}","ref":"{id}","target_slot":"{slot}","summary":"tool=claude profile=p config_home={home} config_home_base="}}"#
            ) + "\n"
        };
        let events = retire("shadow", CLAUDE_ID, "spawned.5")
            + &retire("past", RETIRED_ID, "spawned.6")
            + &retire("past", RETIRED_ID, "spawned.6");
        write(&root.join("sessions/live/events.jsonl"), &events);
        let roster = seat(&root, "main", "lead", "claude", CLAUDE_ID)
            + &seat(&root, "worker.0", "heir", "claude", &id(0xa0))
            + &format!("harness_session_prior.worker.0=claude:{}\n", id(0xa1))
            + "launch_time.worker.0=1788868800\n"
            + &seat(&root, "spawned.0", "solo", "claude", &id(0xa2))
            + &seat(&root, "spawned.1", "twin", "claude", &id(0xa2))
            + &seat(&root, "spawned.2", "linky", "claude", &id(0xa3))
            + &seat(&root, "spawned.3", "left", "claude", &id(0xa4))
            + &seat(&root, "spawned.4", "right", "claude", &id(0xa4))
                .replace("/claude\n", "/claude2\n");
        let (seats, table) = observe(&root, &roster);
        for name in ["lead", "solo", "twin", "shadow"] {
            assert_eq!(cells(named(&seats, name)), UNKNOWN, "{name}");
            let why = named(&seats, name).get_str("coverage_reason");
            assert!(why.is_some_and(|w| w.contains("conflict")), "{name}");
        }
        assert!(table.contains("conflict"), "{table}");
        let heir = named(&seats, "heir");
        assert_eq!(cells(heir), "0 0 0 5 0 0");
        assert_eq!(num(heir.get("predecessors_not_read")), Some(1));
        assert!(table.contains("predecessors not read (1)"), "{table}");
        let past = named(&seats, "past");
        assert_eq!(past.get("retired"), Some(&Value::Bool(true)));
        assert_eq!(cells(past), "0 0 0 3 0 0");
        let past = |seat: &&Value| seat.get_str("seat") == Some("past");
        assert_eq!(seats.iter().filter(past).count(), 1);
        assert_eq!(cells(named(&seats, "left")), "0 0 0 2 0 0");
        assert_eq!(cells(named(&seats, "right")), "0 0 0 4 0 0");
        let linky = cells(named(&seats, "linky"));
        assert!(linky.split(' ').all(|cell| cell.ends_with('?')), "{linky}");
        assert!(named(&seats, "linky").get_str("coverage_reason").is_some());
        assert_eq!(row(&table, "live:linky"), linky);
        let _ = std::fs::remove_dir_all(root);
    }

    // Catches: an unbounded call (all ten read), a seat-count cap (512 / 64 = 8
    // refuses the ninth) or a refusal not named per seat in slot order.
    #[test]
    fn the_512_mib_call_budget_names_every_seat_it_could_not_read() {
        let root = world("hourly-budget");
        let mut roster = String::new();
        for n in 0..10 {
            let path = claude_file(&root, &id(0xa0 + n));
            let hole = 60 * MIB * u64::from(n > 0);
            sparse(&path, "", hole, &rows("w 11:00:00Z 1 0 0 0"));
            let (slot, name) = (format!("spawned.{n}"), format!("s{n}"));
            roster += &seat(&root, &slot, &name, "claude", &id(0xa0 + n));
        }
        let (seats, table) = observe(&root, &roster);
        assert_eq!(seats.len(), 10);
        for (n, seat) in seats[..9].iter().enumerate() {
            assert_eq!(seat.get_str("slot"), Some(format!("spawned.{n}").as_str()));
            assert_eq!(seat.get("coverage_reason"), Some(&Value::Null), "{seat:?}");
            assert_eq!(num(list(seat.get("hours"))[4].get("known_tokens")), Some(1));
        }
        let why = seats[9].get_str("coverage_reason");
        assert_eq!(why, Some("not read (budget)"));
        assert_eq!(cells(&seats[9]), UNKNOWN);
        assert!(table.contains("not read (budget)"), "{table}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn run(root: &Path, args: &[&str]) -> (Option<i32>, String) {
        let mut ae = crate::cli::ae();
        ae.env("HOME", root).env("AE_HOME", root);
        ae.env("CONFIG_FILE", root.join("config")).current_dir(root);
        let output = ae.args(args).output().expect("ae");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        (output.status.code(), stdout)
    }

    // Catches: a broken flag grammar, the hourly path loading the price book, a
    // helper that drops or widens its one flag or reads the wall clock wrong,
    // stale help, changed default bytes, fuzz targets that miss the hourly parsers.
    #[test]
    fn hourly_surface_reads_no_price_book_and_leaves_default_output_unchanged() {
        let root = world("hourly-surface");
        let path = root.join("sessions/live");
        let dir = path.to_str().expect("session path");
        write(&root.join("config"), "[workspace]\nmain = lead\n");
        let meta =
            "schema=2\nsession=live\nseat.main=lead\nprofile.main=grok46\nagent_bin.main=grok\n";
        write(&path.join("meta"), meta);
        let (code, out) = run(&root, &["_usage", dir]);
        assert!(code == Some(0) && out == DEFAULT_HELPER, "{out}");
        let (code, json) = run(&root, &["usage", "--json"]);
        assert!(code == Some(0) && json.starts_with(r#"{"now":"#), "{json}");
        let fleet = json.split_once(',').map(|(_, rest)| rest.trim_end());
        assert_eq!(fleet, Some(DEFAULT_JSON), "{json}");
        let prices = "[workspace]\nmain = lead\n[prices]\na = same,1,2,3,4\nb = same,4,3,2,1\n";
        write(&root.join("config"), prices);
        assert_eq!(run(&root, &["usage"]).0, Some(2));
        assert_eq!(run(&root, &["_usage", dir, "--json"]).0, Some(2));
        let parsed = ae::usage::parse_args(&["--json", "--hourly", "live"].map(String::from));
        let parsed = parsed.expect("args");
        assert!(parsed.hourly && parsed.json && parsed.sessions == ["live"]);
        assert!(!ae::usage::parse_args(&[]).expect("bare").hourly);
        let twice = ae::usage::parse_args(&["--hourly", "--hourly"].map(String::from));
        assert_eq!(twice, Err("--hourly".to_owned()));
        let now = ae::time::Timestamp::now().epoch();
        let stamp = ae::time::Timestamp::from_epoch(now - 7200).to_string();
        let body = rows("k 11:10:00Z 5 0 0 0").replace("2026-09-08T11:10:00Z", &stamp);
        write(&claude_file(&root, CLAUDE_ID), &body);
        let counter = rows("09:00:00Z 40 0 0 40 0 0").replace("2026-09-08T09:00:00Z", &stamp);
        write(&codex_file(&root, CODEX_ID), &counter);
        let w = seat(&root, "worker.0", "w", "claude", CLAUDE_ID);
        let c = seat(&root, "spawned.0", "c", "codex", CODEX_ID);
        write(&path.join("meta"), &format!("{meta}{w}{c}"));
        assert_eq!(run(&root, &["usage", "--hourly"]).0, Some(0));
        let (code, table) = run(&root, &["@live", "usage", "--hourly"]);
        assert_eq!(code, Some(0), "{table}");
        assert_eq!(row(&table, "live:lead"), UNKNOWN);
        let either = ["0 0 0 5 0 0", "0 0 5 0 0 0"];
        assert!(either.contains(&row(&table, "live:w").as_str()), "{table}");
        let either = ["0~ 0~ 0~ 40~ 0~ 0~", "0~ 0~ 40~ 0~ 0~ 0~"];
        assert!(either.contains(&row(&table, "live:c").as_str()), "{table}");
        let twice = run(&root, &["_usage", dir, "--hourly", "--hourly"]);
        assert_eq!(twice.0, Some(2));
        let (code, help) = run(&root, &["@live", "usage", "--help"]);
        assert!(code == Some(0) && help.contains("--hourly"), "{help}");
        let (code, help) = run(&root, &["--help"]);
        assert!(code == Some(0) && help.contains("--hourly"), "{help}");
        let fuzz = format!("{}/fuzz/fuzz_targets", env!("CARGO_MANIFEST_DIR"));
        for target in ["usage_claude_transcript", "usage_codex_rollout"] {
            let source = std::fs::read_to_string(format!("{fuzz}/{target}.rs")).expect("fuzz");
            assert!(source.contains("usage::hourly::"), "{target}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
