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
    let root = std::env::temp_dir().join(format!("ae-usage-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    assert!(parsed.has_token_count);
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
    assert!(zero.has_token_count);
    assert_eq!(zero.tokens, Tokens::default());
    assert!(
        !codex::parse(
            br#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            true,
        )
        .has_token_count
    );
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
