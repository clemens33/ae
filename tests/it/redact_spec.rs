//! Acceptance spec for distill S0 (plan r6, sections 3 and 5).

#![allow(
    clippy::disallowed_methods,
    reason = "the acceptance spec replays the tracked redactor fuzz seeds"
)]

use std::fs;
use std::path::Path;

use ae::redact::{Kind, redact_text};

fn assert_one(kind: Kind, before: &str, secret: &str, after: &str) {
    let input = format!("{before}{secret}{after}");
    let (text, spans) = redact_text(&input);
    assert_eq!(spans, [(kind, before.len()..before.len() + secret.len())]);
    assert_eq!(text, format!("{before}[redacted:{}]{after}", kind.label()));
}

#[test]
fn each_supported_token_shape_replaces_its_whole_run() {
    let cases = [
        (Kind::Github, "ghp_", "A0".repeat(18)),
        (Kind::Github, "gho_", "A0".repeat(18)),
        (Kind::Github, "ghu_", "A0".repeat(18)),
        (Kind::Github, "ghs_", "A0".repeat(18)),
        (Kind::Github, "ghr_", "A0".repeat(18)),
        (Kind::Github, "ghp_", "A0".repeat(24)),
        (Kind::GithubPat, "github_pat_", "A0_-".repeat(12)),
        (Kind::Anthropic, "sk-ant-", "A0_-".repeat(8)),
        (Kind::OpenAi, "sk-", "A0".repeat(10)),
        (Kind::OpenAi, "sk-proj-", "A0_-".repeat(8)),
        (Kind::Slack, "xoxa-", "1234-".repeat(4)),
        (Kind::Slack, "xoxb-", "1234-".repeat(4)),
        (Kind::Slack, "xoxp-", "1234-".repeat(4)),
        (Kind::Slack, "xoxr-", "1234-".repeat(4)),
        (Kind::Slack, "xoxs-", "1234-".repeat(4)),
        (Kind::Aws, "AKIA", "AB01".repeat(4)),
        (Kind::Aws, "ASIA", "AB01".repeat(4)),
        (Kind::Google, "AIza", "A0_Ab".repeat(7)),
        (Kind::Gitlab, "glpat-", "A0_-".repeat(5)),
    ];
    for (kind, prefix, suffix) in cases {
        assert_one(kind, "π (", &format!("{prefix}{suffix}"), "), Ω\n");
    }
    assert_one(
        Kind::Jwt,
        "tuple: (",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkdW1teSJ9.ZmFrZXNpZw",
        "), done",
    );
}

#[test]
fn token_boundaries_include_the_start_and_end_of_the_input() {
    let cases = [
        (Kind::OpenAi, "sk-", "A0".repeat(10)),
        (Kind::Anthropic, "sk-ant-", "A0_-".repeat(8)),
        (Kind::Aws, "AKIA", "AB01".repeat(4)),
        (Kind::Aws, "ASIA", "AB01".repeat(4)),
        (Kind::Github, "ghp_", "A0".repeat(24)),
    ];
    for (kind, prefix, suffix) in cases {
        assert_one(kind, "", &format!("{prefix}{suffix}"), "");
    }
    assert_one(
        Kind::Jwt,
        "",
        "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJkdW1teSJ9.ZmFrZXNpZw",
        "",
    );
    assert_one(Kind::Bearer, "Bearer ", &"A0".repeat(10), "");
}

#[test]
fn bearer_and_url_keep_their_nonsensitive_context() {
    assert_one(
        Kind::Bearer,
        "Authorization: bearer\t",
        &"Fake.Bearer~+/_=".repeat(2),
        "\nfollowing line",
    );
    assert_one(
        Kind::UrlPassword,
        "fetch https://demo:",
        "dummy%3Apw",
        "@example.test:8443/path?x=y",
    );
}

#[test]
fn private_key_span_includes_its_lines_and_fails_closed_without_an_end() {
    let begin = "-----BEGIN PRIVATE KEY-----\nZHVtbXkta2V5\n";
    assert_one(
        Kind::PrivateKey,
        "before\n",
        &format!("{begin}-----END PRIVATE KEY-----"),
        "\nafter",
    );
    assert_one(
        Kind::PrivateKey,
        "before\n",
        &format!("{begin}still inside the unterminated block"),
        "",
    );
    assert_one(
        Kind::PrivateKey,
        "",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nZHVtbXkta2V5\n-----END OPENSSH PRIVATE KEY-----",
        "\nafter",
    );
}

#[test]
fn short_tokens_and_a_word_ending_in_sk_remain_unchanged() {
    let cases = [
        format!("sk-{}", "a".repeat(19)),
        format!("ghp_{}", "a".repeat(35)),
        format!("AKIA{}", "A".repeat(15)),
        format!("Bearer {}", "a".repeat(19)),
        format!("task-{}", "a".repeat(32)),
        "https://demo@example.test/".to_owned(),
        "https://demo:@example.test/".to_owned(),
        "example only\n\t[redacted:x]".to_owned(),
    ];
    for input in cases {
        let (text, spans) = redact_text(&input);
        assert_eq!(text, input);
        assert!(spans.is_empty(), "{input:?}: {spans:?}");
    }
}

#[test]
fn literal_marker_text_does_not_count_as_a_replacement() {
    assert_one(
        Kind::OpenAi,
        "é\t[redacted:x] before ",
        &format!("sk-{}", "a".repeat(20)),
        " after [redacted:x]\n",
    );
}

#[test]
fn the_end_of_a_redacted_span_is_a_boundary_for_the_next_shape() {
    let aws = "AKIAABCDEFGHIJKLMNOP";
    let openai = format!("sk-{}", "a".repeat(20));
    let (text, spans) = redact_text(&format!("{aws}{openai}"));
    assert_eq!(
        spans,
        [
            (Kind::Aws, 0..aws.len()),
            (Kind::OpenAi, aws.len()..aws.len() + openai.len()),
        ]
    );
    assert_eq!(
        text,
        format!(
            "[redacted:{}][redacted:{}]",
            Kind::Aws.label(),
            Kind::OpenAi.label()
        )
    );
}

#[test]
fn tracked_redactor_seeds_replay_with_valid_spans_and_preserved_outside_text() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/redact_text");
    let mut replayed = 0;
    let mut matched = 0;
    for entry in fs::read_dir(path).expect("tracked redact_text seeds") {
        let entry = entry.expect("seed entry");
        if !entry.file_type().expect("seed file type").is_file() {
            continue;
        }
        let bytes = fs::read(entry.path()).expect("seed bytes");
        let Ok(input) = str::from_utf8(&bytes) else {
            continue;
        };
        let (text, spans) = redact_text(input);
        matched += spans.len();
        let mut reconstructed = String::new();
        let mut outside = Vec::new();
        let mut cursor = 0;
        for (kind, range) in spans {
            assert!(range.start < range.end, "{:?}: {range:?}", entry.path());
            let unchanged = input
                .get(cursor..range.start)
                .expect("ascending spans on UTF-8 boundaries");
            outside.push(reconstructed.len()..reconstructed.len() + unchanged.len());
            reconstructed.push_str(unchanged);
            assert!(input.get(range.clone()).is_some(), "{range:?}");
            reconstructed.push_str("[redacted:");
            reconstructed.push_str(kind.label());
            reconstructed.push(']');
            cursor = range.end;
        }
        let tail = &input[cursor..];
        outside.push(reconstructed.len()..reconstructed.len() + tail.len());
        reconstructed.push_str(tail);
        assert_eq!(text, reconstructed, "{:?}", entry.path());
        let (_, second_pass) = redact_text(&text);
        for (_, range) in second_pass {
            assert!(
                !outside
                    .iter()
                    .any(|unchanged| unchanged.start <= range.start && range.end <= unchanged.end),
                "{:?}: a known shape survived in unchanged text at {range:?}",
                entry.path()
            );
        }
        replayed += 1;
    }
    assert!(replayed > 0, "at least one UTF-8 seed must be replayed");
    assert!(
        matched > 0,
        "at least one tracked seed must contain a match"
    );
}
