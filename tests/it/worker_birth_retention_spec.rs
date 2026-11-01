//! Frozen stats287 s1: retained certificates, not retained prompts.
//! Oracle: issue #287 retained-history rule; approved plan tests 5 and 8.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "frozen acceptance fixtures build and inspect their own scratch journals"
)]

use super::cli::OwnedScratch;

const SID: &str = "11111111-1111-4111-8111-111111111111";
const BID: &str = "22222222-2222-4222-8222-222222222222";

fn ordinary(n: usize) -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"human\",\"action\":\"memo\",\"summary\":\"ordinary-{n:04}\"}}"
    )
}

fn family(action: &str, extra: &str) -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{SID}\",\"birth_v\":1,\"birth\":\"{BID}\"{extra}}}"
    )
}

fn clean(n: usize) -> String {
    format!(
        "{{\"ts\":\"2026-11-01T10:00:0{n}Z\",\"actor\":\"human\",\"action\":\"stop-result\",\"summary\":\"already stopped\"}}"
    )
}

fn joined(lines: &[String]) -> String {
    format!("{}\n", lines.join("\n"))
}

#[test]
fn all_family_lines_survive_a_3000_line_trim_in_original_order() {
    let root = OwnedScratch::root("wb", "retain");
    let mut lines: Vec<_> = (0..3000).map(ordinary).collect();
    lines[4] = family("worker-birth", ",\"birth_tool\":\"claude\"");
    lines[9] = family(
        "worker-fact",
        ",\"fact\":\"start-attempted\",\"start\":\"spawned.0:x:41\"",
    );
    lines[15] = clean(1);
    lines[19] = family(
        "worker-fact",
        ",\"fact\":\"observed\",\"observed_source\":\"frame\"",
    );
    lines[27] = family("worker-birth", ",\"birth_tool\":\"claude\"")
        .replace("\"birth_v\":1", "\"birth_v\":2");
    // Damage in the family payload remains a parseable event, and is durable.
    lines[39] = family("worker-birth", ",\"birth_model\":7");
    lines[49] = clean(2);
    // Retaining a modern spawn announcement would retain the prompt: forbidden.
    lines[59] = family("spawn", ",\"summary\":\"PROMPT_MUST_AGE_OUT\"");
    lines[999] = family("worker-fact", ",\"fact\":\"brief-failed\"");
    lines[2000] = family("worker-birth", ",\"birth_tool\":\"codex\"");
    let path = root.join("events.jsonl");
    std::fs::write(&path, joined(&lines)).expect("input journal");
    ae::store::open(&root).retain_events(1000, ae::restore::pins_clean_stop);

    let kept = [4, 9, 19, 27, 39, 49, 999]
        .into_iter()
        .map(|i| lines[i].clone())
        .chain(lines[2000..].iter().cloned())
        .collect::<Vec<_>>();
    let actual = std::fs::read_to_string(&path).expect("retained journal");
    assert_eq!(
        actual,
        joined(&kept),
        "retention is exact, ordered and prompt-free"
    );
    assert!(!actual.contains("PROMPT_MUST_AGE_OUT"));
    // Repeating resume retention neither multiplies nor reorders a pinned fact.
    ae::store::open(&root).retain_events(1000, ae::restore::pins_clean_stop);
    assert_eq!(std::fs::read_to_string(path).expect("second trim"), actual);
}

#[test]
fn journals_without_family_keep_the_prior_window_and_clean_stop_bytes() {
    let root = OwnedScratch::root("wb", "oldtrim");
    let path = root.join("events.jsonl");
    for stop_in_window in [false, true] {
        let mut lines: Vec<_> = (0..3000).map(ordinary).collect();
        lines[10] = clean(1);
        lines[50] = clean(2);
        if stop_in_window {
            lines[2500] = clean(3);
        }
        std::fs::write(&path, joined(&lines)).expect("input journal");
        ae::store::open(&root).retain_events(1000, ae::restore::pins_clean_stop);
        let expected = if stop_in_window {
            joined(&lines[2000..])
        } else {
            joined(
                &std::iter::once(lines[50].clone())
                    .chain(lines[2000..].iter().cloned())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(std::fs::read_to_string(&path).expect("old trim"), expected);
    }
    let small = format!("{}\n{}", ordinary(0), ordinary(1));
    std::fs::write(&path, &small).expect("short unterminated journal");
    ae::store::open(&root).retain_events(1000, ae::restore::pins_clean_stop);
    assert_eq!(std::fs::read_to_string(path).expect("short journal"), small);
}
