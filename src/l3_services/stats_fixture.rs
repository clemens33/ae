//! Journal fixtures only; no statistics implementation or expected-total helper.
#![allow(
    dead_code,
    reason = "shared fixtures serve several independent spec modules"
)]

pub(crate) const S: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const T: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const PRIVATE: &str = "PRIVATE-PROMPT-BRIEF-COMMAND-TOKEN-https://private.invalid";

pub(crate) fn id(n: u32) -> String {
    format!("aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}")
}

pub(crate) fn event(action: &str, extra: &str) -> String {
    format!(
        "{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"scout\"{extra}}}\n"
    )
}

pub(crate) fn birth(session: &str, n: u32, fields: &str) -> String {
    event(
        "worker-birth",
        &format!(
            ",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{session}\",\"birth_v\":1,\"birth\":\"{}\"{fields}",
            id(n)
        ),
    )
}

pub(crate) fn link(n: u32) -> String {
    event(
        "spawn",
        &format!(",\"birth\":\"{}\",\"summary\":\"{PRIVATE}\"", id(n)),
    )
}

pub(crate) fn legacy(summary: &str) -> String {
    event("spawn", &format!(",\"summary\":\"{summary}\""))
}

pub(crate) fn fact(session: &str, n: u32, kind: &str, fields: &str) -> String {
    event(
        "worker-fact",
        &format!(
            ",\"target_session_id\":\"{session}\",\"birth_v\":1,\"birth\":\"{}\",\"fact\":\"{kind}\"{fields}",
            id(n)
        ),
    )
}

pub(crate) fn fields(
    tool: &str,
    client: Option<&str>,
    model: Option<&str>,
    effort: Option<&str>,
) -> String {
    let mut cells = format!(",\"birth_tool\":\"{tool}\",\"birth_profile\":\"preset\"");
    for (key, value) in [
        ("birth_client", client),
        ("birth_model", model),
        ("birth_effort", effort),
    ] {
        if let Some(value) = value {
            use std::fmt::Write as _;
            write!(cells, ",\"{key}\":\"{value}\"").expect("write to String");
        }
    }
    cells
}

/// Oracle A in README: S has births1–7, two identical legacy spawns, lost10.
/// T has births8–9. Linked modern announcements never add another birth.
pub(crate) fn matrix() -> (Vec<u8>, Vec<u8>) {
    let s_rows = [
        fields("claude", Some("claude-p"), Some("sonnet"), Some("high")),
        fields("claude", Some("claude-mic"), Some("sonnet"), Some("high")),
        fields("claude", Some("claude-p"), Some("opus"), Some("max")),
        fields("codex", Some("codex"), Some("sol"), Some("high")),
        format!(
            "{},\"birth_model_unproved\":\"unsupported\",\"birth_effort_unproved\":\"unsupported\"",
            fields("muse", None, None, None)
        ),
        format!(
            "{},\"birth_model_unproved\":\"ambiguous-model\"",
            fields("claude", Some("claude-p"), None, Some("high"))
        ),
        format!(
            "{},\"birth_effort_unproved\":\"unknown-effort\"",
            fields("codex", Some("codex"), Some("sol"), None)
        ),
    ];
    let mut s = String::new();
    for (n, cells) in (1..=7).zip(s_rows) {
        s.push_str(&birth(S, n, &cells));
        s.push_str(&link(n));
    }
    s.push_str(&legacy(PRIVATE));
    s.push_str(&legacy(PRIVATE));
    s.push_str(&link(10));
    let mut t = birth(
        T,
        8,
        &fields(
            "claude",
            Some("claude-p"),
            Some("claude-sonnet-5-5"),
            Some("high"),
        ),
    );
    t.push_str(&link(8));
    t.push_str(&birth(T, 9, &fields("codex", None, None, None)));
    t.push_str(&link(9));
    (s.into_bytes(), t.into_bytes())
}
