//! Quota command surface over a hermetic vendor-cache rig.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the boundary is about what product code may reach"
)]

use std::path::PathBuf;

const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
const FIRST_ID: &str = "01a08046-1974-7352-ade3-81a786200795";
const SECOND_ID: &str = "01a08046-2000-7abc-8abc-aaaaaaaaaaaa";
const THIRD_ID: &str = "01a08046-2100-7abc-8abc-bbbbbbbbbbbb";
const FOURTH_ID: &str = "01a08046-2200-7abc-8abc-cccccccccccc";

fn rig(tag: &str) -> PathBuf {
    let root = PathBuf::from(format!("/tmp/ae-quota-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".codex/sessions/2026/09/08"))
        .expect("quota rig directories");
    std::fs::create_dir_all(root.join("sessions/session")).expect("session directory");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "claude = claude\n",
            "mic = claude config_home=$HOME/.claude-mic\n",
            "codex = codex\n",
            "[profiles]\n",
            "fablex = claude --model fable\n",
            "fable5 = claude --model fable\n",
            "opusx = claude --model opus\n",
            "micx = mic --model fable\n",
            "astrax = codex --model astra\n",
            "solx = codex --model sol\n",
            "grok46 = grok --model grok-4.6\n",
            "agy = agy\n",
            "opencode = opencode\n",
            "gemini = gemini\n",
        ),
    )
    .expect("config");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nmode=local\norigin={}\nwork_dir={}\nseat.main=lead\nprofile.main=astrax\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\nseat.worker.0=reviewer\nprofile.worker.0=solx\nharness_session.worker.0={SECOND_ID}\nagent_bin.worker.0=codex\n",
            root.display(),
            root.display(),
        ),
    )
    .expect("meta");
    std::fs::write(
        root.join(".claude.json"),
        include_bytes!("../fixtures/quota/claude-cache.json"),
    )
    .expect("Claude cache");
    std::fs::create_dir_all(root.join(".claude-mic")).expect("custom Claude home");
    let custom_cache =
        String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
            .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(
        root.join(".claude-mic/.claude.json"),
        custom_cache.as_bytes(),
    )
    .expect("custom Claude cache");
    for id in [FIRST_ID, SECOND_ID] {
        std::fs::write(
            root.join(format!(
                ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
            )),
            include_bytes!("../fixtures/quota/codex-rollout.jsonl"),
        )
        .expect("Codex rollout");
    }
    root
}

fn run_quota(root: &std::path::Path) -> String {
    run_quota_with_home(root, Some(root))
}

fn run_quota_with_home(root: &std::path::Path, home: Option<&std::path::Path>) -> String {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home,
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    String::from_utf8(stdout).expect("UTF-8 table")
}

fn add_codex_session(root: &std::path::Path, name: &str, id: &str, rollout: &[u8]) {
    let session = root.join("sessions").join(name);
    std::fs::create_dir_all(&session).expect("session directory");
    std::fs::write(
        session.join("meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=astrax\nharness_session.main={id}\nagent_bin.main=codex\n"
        ),
    )
    .expect("session meta");
    let day = root.join(".codex/sessions/2026/09/08");
    std::fs::write(
        day.join(format!("rollout-2026-09-08T09-00-00-{id}.jsonl")),
        rollout,
    )
    .expect("Codex rollout");
}

#[test]
fn ae_quota_renders_fixture_caches_missing_scopes_and_unsupported_clients() {
    let root = rig("golden");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(
        String::from_utf8(stdout).expect("UTF-8 table"),
        include_str!("../fixtures/quota/expected.stdout")
    );
    let rendered = include_str!("../fixtures/quota/expected.stdout");
    assert!(rendered.lines().all(|line| line.chars().count() <= 160));
    assert_eq!(rendered.matches("unidentified").count(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn declared_manual_resets_and_reported_credits_drive_the_effective_columns() {
    let root = rig("declared-headroom");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "codex = codex manual_resets=1\n",
            "[profiles]\n",
            "astrax = codex --model astra\n",
        ),
    )
    .expect("declared config");
    let fixture = String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
        .replace("\"used_percent\":7.0", "\"used_percent\":95.0")
        .replace(
            "\"plan_type\":\"pro\"",
            "\"plan_type\":\"pro\",\"credits\":{\"has_credits\":true,\"unlimited\":false,\"balance\":\"12.50\"},\"spend_control_reached\":false",
        );
    for id in [FIRST_ID, SECOND_ID] {
        std::fs::write(
            root.join(format!(
                ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
            )),
            &fixture,
        )
        .expect("rollout with credits");
    }
    let text = run_quota(&root);
    assert!(
        text.contains("EFFECTIVE  CREDITS"),
        "both derived columns are present: {text}"
    );
    assert!(
        text.contains(" 95%   47.5% x1   12.50"),
        "one declared reset halves the judged percentage: {text}"
    );
    assert!(
        text.lines().all(|line| line.chars().count() <= 182),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&root);

    let capped = rig("spend-capped");
    std::fs::write(
        capped.join("config"),
        concat!(
            "[clients]\n",
            "codex = codex manual_resets=4\n",
            "[profiles]\n",
            "astrax = codex --model astra\n",
        ),
    )
    .expect("capped config");
    let fixture = String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
        .replace(
            "\"plan_type\":\"pro\"",
            "\"plan_type\":\"pro\",\"credits\":{\"has_credits\":false,\"unlimited\":false,\"balance\":\"0\"},\"spend_control_reached\":true",
        );
    for id in [FIRST_ID, SECOND_ID] {
        std::fs::write(
            capped.join(format!(
                ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
            )),
            &fixture,
        )
        .expect("spend-capped rollout");
    }
    let text = run_quota(&capped);
    assert!(
        text.contains("100%       spend-cap"),
        "a spend cap outranks four declared resets: {text}"
    );
    let _ = std::fs::remove_dir_all(&capped);
}

#[test]
fn conflicting_declarations_on_one_home_render_the_smallest_count_and_the_conflict() {
    for (first, second) in [
        (
            "a = codex config_home=$HOME/.codex-shared manual_resets=1",
            "b = codex config_home=$HOME/.codex-shared manual_resets=0",
        ),
        (
            "a = codex config_home=$HOME/.codex-shared manual_resets=0",
            "b = codex config_home=$HOME/.codex-shared manual_resets=1",
        ),
    ] {
        let root = rig("declared-conflict");
        let shared = root.join(".codex-shared/sessions/2026/09/08");
        std::fs::create_dir_all(&shared).expect("shared rollout day");
        let fixture =
            String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
                .replace("\"used_percent\":7.0", "\"used_percent\":95.0");
        std::fs::write(
            shared.join(format!("rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl")),
            &fixture,
        )
        .expect("shared rollout");
        std::fs::write(
            root.join("config"),
            format!(
                "[clients]\n{first}\n{second}\n[profiles]\nax = a --model x\nbx = b --model y\n"
            ),
        )
        .expect("conflicting config");
        std::fs::write(
            root.join("sessions/session/meta"),
            format!(
                "schema=2\nseat.main=lead\nprofile.main=ax\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\n"
            ),
        )
        .expect("session meta");
        let text = run_quota(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            text.contains(" 95%   95%"),
            "the pessimistic count decides what ae judges: {text}"
        );
        assert!(
            !text.contains("47.5%"),
            "an explicit zero is never overruled: {text}"
        );
        assert_eq!(
            text.matches("manual_resets declared as 0 and 1 for one scope; using 0")
                .count(),
            1,
            "the conflict is visible exactly once: {text}"
        );
    }
}

#[test]
fn a_proven_spend_cap_survives_a_sibling_field_that_asserts_nothing() {
    let record = |timestamp: &str, tail: &str| {
        format!(
            r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"limit_id":"codex","plan_type":"pro","primary":{{"used_percent":95.0,"window_minutes":10080,"resets_at":1789445400}},"secondary":null{tail}}}}}}}
"#
        )
    };
    let capped = record(
        "2026-09-08T09:00:00Z",
        r#","credits":{"has_credits":false,"unlimited":false,"balance":"0"},"spend_control_reached":true"#,
    );
    for (label, later) in [
        (
            "a null spend field",
            r#","credits":{"has_credits":true,"unlimited":false,"balance":"5.00"},"spend_control_reached":null"#,
        ),
        (
            "a malformed spend field",
            r#","credits":{"has_credits":true,"unlimited":false,"balance":"5.00"},"spend_control_reached":"yes""#,
        ),
        ("a bucketless unlimited claim", ""),
    ] {
        let root = rig("cap-survives");
        let rollout = if label == "a bucketless unlimited claim" {
            capped.clone()
                + r#"{"timestamp":"2026-09-08T09:05:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"credits":{"has_credits":true,"unlimited":true,"balance":"0"}}}}"#
                + "\n"
        } else {
            capped.clone() + &record("2026-09-08T09:05:00Z", later)
        };
        std::fs::write(
            root.join("config"),
            "[clients]\ncodex = codex manual_resets=1\n[profiles]\nastrax = codex --model astra\n",
        )
        .expect("config");
        std::fs::write(
            root.join(format!(
                ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl"
            )),
            &rollout,
        )
        .expect("rollout");
        let text = run_quota(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            text.contains(" 100%       spend-cap"),
            "{label} asserts nothing, so the rendered cap must stand: {text}"
        );
        assert!(
            !text.contains("47.5%") && !text.contains(" 0%"),
            "{label} must not buy the scope any headroom: {text}"
        );
    }
}

#[test]
fn an_unusable_declared_reset_count_is_ignored_with_one_visible_note() {
    let root = rig("declared-note");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "codex = codex manual_resets=nine\n",
            "[profiles]\n",
            "astrax = codex --model astra\n",
            "solx = codex --model sol\n",
        ),
    )
    .expect("bad declaration config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        text.matches("manual_resets=nine ignored").count(),
        1,
        "one note for the scope, not one per rollout: {text}"
    );
    assert!(
        text.contains("not a whole count 0-9") && text.contains(" 7%    -"),
        "the rows stay readable and nothing is derived: {text}"
    );
}

#[test]
fn absent_and_unexpected_sources_are_unknown_and_read_error() {
    let root = rig("source-errors");
    std::fs::remove_file(root.join(".claude.json")).expect("remove planted cache");
    std::fs::create_dir(root.join(".claude.json")).expect("unexpected directory source");
    std::fs::remove_file(root.join(".claude-mic/.claude.json")).expect("remove custom cache");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut stderr,
    )
    .expect("quota runs");
    assert_eq!(code, 0);
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert!(
        text.contains("claude · ~/.claude") && text.contains("read-error"),
        "{text}"
    );
    assert!(
        text.contains("claude · mic") && text.contains("unknown"),
        "{text}"
    );
    assert!(stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn client_labels_sharing_one_home_merge_into_one_scope() {
    let root = rig("shared-client-home");
    std::fs::write(
        root.join("config"),
        concat!(
            "[clients]\n",
            "mic = claude config_home=$HOME/.claude-mic\n",
            "work = claude config_home=$HOME/.claude-mic\n",
            "[profiles]\n",
            "micx = mic\n",
            "workx = work\n",
        ),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("micx workx") && text.contains("claude · mic, work") && text.contains("77%"),
        "{text}"
    );
    assert_eq!(text.matches("claude · mic, work").count(), 1, "{text}");
}

#[test]
fn profile_home_assignment_selects_claude_cache_from_effective_home() {
    let root = rig("effective-home");
    let alternate = root.join("alternate");
    std::fs::create_dir_all(&alternate).expect("alternate home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(alternate.join(".claude.json"), cache.as_bytes())
        .expect("alternate default cache");
    std::fs::write(
        root.join("config"),
        format!("[profiles]\nmoved = HOME={} claude\n", alternate.display()),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("moved")
            && text.contains("claude · ~/alternate/.claude")
            && text.contains("77%"),
        "{text}"
    );
    assert!(!text.contains("66%"), "{text}");
}

#[cfg(unix)]
#[test]
fn symlinked_client_home_and_its_target_are_one_scope() {
    use std::os::unix::fs::symlink;

    let root = rig("symlink-client-home");
    let link = root.join(".claude-mic");
    let target = root.join("claude-target");
    std::fs::remove_dir_all(&link).expect("remove planted client home");
    std::fs::create_dir_all(&target).expect("target client home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(target.join(".claude.json"), cache.as_bytes()).expect("target cache");
    symlink(&target, &link).expect("client-home symlink");
    std::fs::write(
        root.join("config"),
        format!(
            "[clients]\nmic = claude config_home=$HOME/.claude-mic\ntarget = claude config_home={}\n[profiles]\nmicx = mic\ntargetx = target\n",
            target.display()
        ),
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("micx targetx")
            && text.contains("claude · mic, target")
            && text.contains("77%"),
        "{text}"
    );
    assert_eq!(text.matches("claude · mic, target").count(), 1, "{text}");
}

#[test]
fn client_resolution_error_is_unknown_without_hiding_other_scopes() {
    let root = rig("client-resolution-error");
    let good = root.join("good-claude");
    std::fs::create_dir_all(&good).expect("good client home");
    let cache = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(good.join(".claude.json"), cache.as_bytes()).expect("good cache");
    std::fs::write(
        root.join("config"),
        format!(
            "[clients]\nbad = claude config_home=$HOME/.claude-bad\n[profiles]\nbadx = bad\ngoodx = CLAUDE_CONFIG_DIR={} claude\n",
            good.display()
        ),
    )
    .expect("profile config");
    let text = run_quota_with_home(&root, None);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("badx")
            && text.contains("claude · bad")
            && text.contains("unknown")
            && text.contains("HOME")
            && text.contains("unavailable"),
        "{text}"
    );
    assert!(text.contains("goodx") && text.contains("77%"), "{text}");
}

#[cfg(unix)]
#[test]
fn implicit_claude_profiles_do_not_merge_distinct_sources_after_home_canonicalization() {
    use std::os::unix::fs::symlink;

    let make_root = |tag: &str, profile: &str| {
        let root = rig(tag);
        let shared = root.join("shared-store");
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(shared.join(".claude")).expect("shared Claude store");
        std::fs::create_dir_all(&a).expect("home a");
        std::fs::create_dir_all(&b).expect("home b");
        symlink(shared.join(".claude"), a.join(".claude")).expect("a store symlink");
        symlink(shared.join(".claude"), b.join(".claude")).expect("b store symlink");
        let fixture =
            String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
        std::fs::write(
            a.join(".claude.json"),
            fixture.replace("\"percent\": 66", "\"percent\": 11"),
        )
        .expect("a cache");
        std::fs::write(
            b.join(".claude.json"),
            fixture.replace("\"percent\": 66", "\"percent\": 77"),
        )
        .expect("b cache");
        let home = if profile == "a" { &a } else { &b };
        std::fs::write(
            root.join("config"),
            format!("[profiles]\n{profile} = HOME={} claude\n", home.display()),
        )
        .expect("profile config");
        root
    };
    let a_root = make_root("implicit-claude-a", "a");
    let a_text = run_quota(&a_root);
    let _ = std::fs::remove_dir_all(&a_root);
    let b_root = make_root("implicit-claude-b", "b");
    let b_text = run_quota(&b_root);
    let _ = std::fs::remove_dir_all(&b_root);

    // Rebuild with scratch-root HOME assignments so canonicalized stores collide.
    let both_root = rig("implicit-claude-both");
    let shared = both_root.join("shared-store");
    let a = both_root.join("a");
    let b = both_root.join("b");
    std::fs::create_dir_all(shared.join(".claude")).expect("shared Claude store");
    std::fs::create_dir_all(&a).expect("home a");
    std::fs::create_dir_all(&b).expect("home b");
    symlink(shared.join(".claude"), a.join(".claude")).expect("a store symlink");
    symlink(shared.join(".claude"), b.join(".claude")).expect("b store symlink");
    let fixture = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
    std::fs::write(
        a.join(".claude.json"),
        fixture.replace("\"percent\": 66", "\"percent\": 11"),
    )
    .expect("a cache");
    std::fs::write(
        b.join(".claude.json"),
        fixture.replace("\"percent\": 66", "\"percent\": 77"),
    )
    .expect("b cache");
    std::fs::write(
        both_root.join("config"),
        format!(
            "[profiles]\na = HOME={} claude\nb = HOME={} claude\n",
            a.display(),
            b.display()
        ),
    )
    .expect("profile config");
    let both_text = run_quota(&both_root);
    let _ = std::fs::remove_dir_all(&both_root);

    assert!(a_text.contains("11%"), "{a_text}");
    assert!(b_text.contains("77%"), "{b_text}");
    let separate_evidence = both_text.contains("11%") && both_text.contains("77%");
    let explicit_ambiguity =
        both_text.contains("unknown") && !both_text.contains("11%") && !both_text.contains("77%");
    assert!(separate_evidence || explicit_ambiguity, "{both_text}");
}

#[test]
fn retained_codex_rollout_uses_recorded_config_home_after_profile_change() {
    let root = rig("retained-recorded-home");
    let a = root.join("codex-a");
    let b = root.join("codex-b");
    let day_a = a.join("sessions/2026/09/08");
    let day_b = b.join("sessions/2026/09/08");
    std::fs::create_dir_all(&day_a).expect("A rollout dir");
    std::fs::create_dir_all(&day_b).expect("B rollout dir");
    let recorded_a = std::fs::canonicalize(&a).expect("canonical A home");
    let fixture = include_bytes!("../fixtures/quota/codex-rollout.jsonl");
    std::fs::write(
        day_a.join(format!("rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl")),
        fixture,
    )
    .expect("A rollout");
    let changed = String::from_utf8_lossy(fixture).replace("7.0", "91.0");
    std::fs::write(
        day_b.join(format!("rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl")),
        changed,
    )
    .expect("B rollout");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=p\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\nconfig_home.main={}\n",
            recorded_a.display()
        ),
    )
    .expect("recorded meta");
    let meta_before =
        std::fs::read_to_string(root.join("sessions/session/meta")).expect("read recorded meta");
    let parsed_meta = ae::meta::Meta::parse(&meta_before);
    assert_eq!(
        parsed_meta.roster()[0].config_home,
        ae::meta::RecordedConfigHome::Path(recorded_a.clone())
    );
    std::fs::write(
        root.join("config"),
        format!("[profiles]\np = CODEX_HOME={} codex\n", a.display()),
    )
    .expect("control config");
    let control = run_quota(&root);
    std::fs::write(
        root.join("config"),
        format!("[profiles]\np = CODEX_HOME={} codex\n", b.display()),
    )
    .expect("changed config");
    let changed = run_quota(&root);
    let meta_after =
        std::fs::read_to_string(root.join("sessions/session/meta")).expect("read unchanged meta");
    assert_eq!(meta_after, meta_before);
    let _ = std::fs::remove_dir_all(&root);

    assert!(control.contains("7%"), "{control}");
    assert!(!changed.contains("91%"), "{changed}");
    assert!(
        changed.contains("7%") || changed.contains("unknown"),
        "{changed}"
    );
}

#[test]
fn quota_marks_unobserved_parameter_home_unknown_but_launch_resolves_injected_value() {
    let root = rig("parameter-home");
    std::fs::write(
        root.join("config"),
        "[profiles]\np = CLAUDE_CONFIG_DIR=${AE_QUOTA_HOME:-$HOME/.claude-mic} claude\n",
    )
    .expect("profile config");
    let quota_text = run_quota(&root);
    let cfg = ae::config::parse_identity(
        "[profiles]\np = CLAUDE_CONFIG_DIR=${AE_QUOTA_HOME:-$HOME/.claude-mic} claude\n",
    )
    .expect("identity config");
    let command = cfg
        .command("p", Some(&root))
        .expect("profile command")
        .expect("profile");
    let other = root.join("other");
    let resolved =
        ae::launch_cmd::config_home(&command, ae::tool::ToolKind::Claude, &|name| match name {
            "HOME" => Some(root.display().to_string()),
            "AE_QUOTA_HOME" => Some(other.display().to_string()),
            _ => None,
        });
    eprintln!(
        "quota output:\n{quota_text}launch resolved home: {}",
        resolved.shown()
    );
    assert_eq!(resolved, ae::launch_cmd::Resolved::Path(other));
    let _ = std::fs::remove_dir_all(&root);

    assert!(!quota_text.contains("77%"), "{quota_text}");
    let normalized = quota_text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("depends on pane variable AE_QUOTA_HOME"),
        "{quota_text}"
    );
}

#[test]
fn quota_self_referential_claude_config_dir_fallback_is_unknown_before_launch_injection() {
    let root = rig("parameter-home-claude-config-dir");
    let config =
        "[profiles]\np = CLAUDE_CONFIG_DIR=${CLAUDE_CONFIG_DIR:-$HOME/.claude-mic} claude\n";
    std::fs::write(root.join("config"), config).expect("profile config");
    let cfg = ae::config::parse_identity(config).expect("identity config");
    let command = cfg
        .command("p", Some(&root))
        .expect("profile command")
        .expect("profile");
    let other = root.join("other");
    let resolved =
        ae::launch_cmd::config_home(&command, ae::tool::ToolKind::Claude, &|name| match name {
            "HOME" => Some(root.display().to_string()),
            "CLAUDE_CONFIG_DIR" => Some(other.display().to_string()),
            _ => None,
        });
    let quota_text = run_quota(&root);
    eprintln!(
        "quota output:\n{quota_text}launch resolved home: {}",
        resolved.shown()
    );
    assert_eq!(resolved, ae::launch_cmd::Resolved::Path(other));
    let _ = std::fs::remove_dir_all(&root);

    assert!(!quota_text.contains("77%"), "{quota_text}");
    let normalized = quota_text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("depends on pane variable CLAUDE_CONFIG_DIR"),
        "{quota_text}"
    );
}

#[test]
fn quota_nested_home_plus_claude_config_dir_fallback_is_unknown_before_launch_injection() {
    let root = rig("nested-home-plus-claude-config-dir");
    let config = "[profiles]\np = CLAUDE_CONFIG_DIR=${HOME:+${CLAUDE_CONFIG_DIR:-$HOME/.claude-mic}} claude\n";
    std::fs::write(root.join("config"), config).expect("profile config");
    let cfg = ae::config::parse_identity(config).expect("identity config");
    let command = cfg
        .command("p", Some(&root))
        .expect("profile command")
        .expect("profile");
    let other = root.join("other");
    let resolved =
        ae::launch_cmd::config_home(&command, ae::tool::ToolKind::Claude, &|name| match name {
            "HOME" => Some(root.display().to_string()),
            "CLAUDE_CONFIG_DIR" => Some(other.display().to_string()),
            _ => None,
        });
    assert_eq!(resolved, ae::launch_cmd::Resolved::Path(other));
    let quota_text = run_quota(&root);
    eprintln!(
        "quota output:\n{quota_text}launch resolved home: {}",
        resolved.shown()
    );
    let _ = std::fs::remove_dir_all(&root);

    assert!(!quota_text.contains("77%"), "{quota_text}");
    assert!(
        quota_text.contains("unknown")
            || quota_text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains("depends on pane variable CLAUDE_CONFIG_DIR"),
        "{quota_text}"
    );
}

#[test]
fn mixed_recorded_and_legacy_codex_same_profile_do_not_cross_attribute_rollouts() {
    let root = rig("mixed-recorded-legacy");
    let a = root.join("codex-a");
    let b = root.join("codex-b");
    let day_a = a.join("sessions/2026/09/08");
    let day_b = b.join("sessions/2026/09/08");
    std::fs::create_dir_all(&day_a).expect("A rollout dir");
    std::fs::create_dir_all(&day_b).expect("B rollout dir");
    let fixture = include_bytes!("../fixtures/quota/codex-rollout.jsonl");
    let a_seven = fixture.to_vec();
    let b_ninety_one = String::from_utf8_lossy(fixture).replace("7.0", "91.0");
    let a_seventy_seven = String::from_utf8_lossy(fixture).replace("7.0", "77.0");
    std::fs::write(
        day_a.join(format!("rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl")),
        a_seven,
    )
    .expect("A/X rollout");
    std::fs::write(
        day_b.join(format!("rollout-2026-09-08T09-00-00-{SECOND_ID}.jsonl")),
        b_ninety_one,
    )
    .expect("B/Y rollout");
    std::fs::write(
        day_a.join(format!("rollout-2026-09-08T09-00-00-{SECOND_ID}.jsonl")),
        a_seventy_seven,
    )
    .expect("A/Y copied rollout");
    std::fs::write(
        root.join("config"),
        format!("[profiles]\np = CODEX_HOME={} codex\n", b.display()),
    )
    .expect("profile config");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=legacy\nprofile.main=p\nharness_session.main={SECOND_ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("legacy meta");
    let control = run_quota(&root);
    let recorded_a = std::fs::canonicalize(&a).expect("canonical A home");
    std::fs::create_dir_all(root.join("sessions/recorded")).expect("recorded session directory");
    std::fs::write(
        root.join("sessions/recorded/meta"),
        format!(
            "schema=2\nseat.main=recorded\nprofile.main=p\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\nconfig_home.main={}\n",
            recorded_a.display()
        ),
    )
    .expect("recorded meta");
    let meta_before =
        std::fs::read_to_string(root.join("sessions/recorded/meta")).expect("recorded meta before");
    let mixed = run_quota(&root);
    let meta_after =
        std::fs::read_to_string(root.join("sessions/recorded/meta")).expect("recorded meta after");
    assert_eq!(meta_after, meta_before);
    let _ = std::fs::remove_dir_all(&root);

    assert!(control.contains("91%"), "{control}");
    assert!(!control.contains("77%"), "{control}");
    assert!(mixed.contains("7%") && mixed.contains("91%"), "{mixed}");
    assert!(!mixed.contains("77%"), "{mixed}");
}

#[test]
fn changed_config_home_never_relabels_a_retained_rollout() {
    let root = rig("retained-home");
    let changed = root.join(".codex-b/sessions/2026/09/08");
    std::fs::create_dir_all(&changed).expect("changed Codex home");
    let changed_rollout =
        String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
            .replace("7.0", "91.0");
    std::fs::write(
        changed.join(format!("rollout-2026-09-08T09-00-00-{SECOND_ID}.jsonl")),
        changed_rollout,
    )
    .expect("changed-home rollout");
    std::fs::write(
        root.join("config"),
        "[profiles]\nchanged = CODEX_HOME=$HOME/.codex-b codex\n",
    )
    .expect("changed profile");
    std::fs::write(
        root.join("sessions/session/meta"),
        format!(
            "schema=2\nseat.main=lead\nprofile.main=changed\nharness_session.main={FIRST_ID}\nagent_bin.main=codex\n"
        ),
    )
    .expect("changed meta");
    let mut stdout = Vec::new();
    let code = ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut Vec::new(),
    )
    .expect("quota runs");
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert_eq!(code, 0);
    assert!(
        text.contains("codex · ~/.codex-b · unidentified") && text.contains("(session:lead)"),
        "{text}"
    );
    assert!(!text.contains("91%") && !text.contains("7%"), "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quota_helper_is_read_only_and_starts_no_tmux_or_vendor_process() {
    let root = rig("helper-observational");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("sentinel bin");
    let marker = root.join("invoked");
    for name in [
        "tmux", "claude", "codex", "grok", "agy", "opencode", "gemini",
    ] {
        let path = bin.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf called > '{}'\nexit 99\n",
                marker.display()
            ),
        )
        .expect("sentinel program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("executable sentinel");
        }
    }
    let before = crate::cli::byte_tree(&root);
    let session = root.join("sessions/session");
    let out = crate::cli::ae()
        .env("HOME", &root)
        .env("AE_HOME", &root)
        .env("CONFIG_FILE", root.join("config"))
        .env("PATH", &bin)
        .current_dir(&root)
        .args(["_quota", session.to_str().expect("session path")])
        .output()
        .expect("quota helper should run");
    assert!(out.status.success(), "{:?}", out.status);
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("codex · ~/.codex · unidentified") && stdout.contains("(session:lead)"),
        "{stdout}"
    );
    assert_eq!(
        crate::cli::byte_tree(&root),
        before,
        "quota helper changed scratch HOME bytes"
    );
    assert!(!marker.exists(), "quota helper executed a sentinel program");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn fifo_and_symlink_vendor_sources_are_read_errors_without_being_opened() {
    use std::os::unix::fs::symlink;

    let root = rig("hostile-nodes");
    std::fs::remove_file(root.join(".claude.json")).expect("remove regular Claude cache");
    crate::cli::mkfifo(&root.join(".claude.json"));
    let rollout = root.join(format!(
        ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{FIRST_ID}.jsonl"
    ));
    std::fs::remove_file(&rollout).expect("remove regular Codex rollout");
    symlink(root.join("config"), &rollout).expect("rollout symlink");
    let mut stdout = Vec::new();
    ae::quota::run(
        &ae::quota::Inputs {
            home: Some(&root),
            global: Some(&root.join("config")),
            local: None,
            sessions: Some(&root.join("sessions")),
            now: NOW,
        },
        &mut stdout,
        &mut Vec::new(),
    )
    .expect("quota refuses hostile nodes without blocking");
    let text = String::from_utf8(stdout).expect("UTF-8 table");
    assert!(text.matches("read-error").count() >= 2, "{text}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn malformed_hidden_rollout_keeps_read_error_visible_after_display_cap() {
    let root = rig("malformed-hidden");
    let fixture = include_bytes!("../fixtures/quota/codex-rollout.jsonl");
    add_codex_session(&root, "session-2", THIRD_ID, fixture);
    add_codex_session(&root, "session-3", FOURTH_ID, b"{broken}\n");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);

    let control = rig("malformed-control");
    add_codex_session(&control, "session-2", THIRD_ID, b"{broken}\n");
    let control_text = run_quota(&control);
    let _ = std::fs::remove_dir_all(&control);
    assert!(control_text.contains("read-error"), "{control_text}");
    assert!(!control_text.contains("+1 rollout"), "{control_text}");
    assert!(
        text.lines().any(|line| {
            line.contains("+1 rollout not shown")
                && line.contains("1 unreadable")
                && line.contains("read-error")
        }),
        "{text}"
    );
}

#[test]
fn claude_model_display_name_controls_never_reach_quota_stdout() {
    let root = rig("claude-controls");
    let escaped = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"))
        .replace("Fable", "\\u001b[2J\\r\\n");
    std::fs::write(root.join(".claude.json"), escaped.as_bytes()).expect("escaped cache");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.chars().all(|ch| ch == '\n' || !ch.is_control()),
        "{text:?}"
    );
    assert_eq!(
        text.matches('\n').count(),
        include_str!("../fixtures/quota/expected.stdout")
            .matches('\n')
            .count(),
        "vendor newlines must not add renderer lines"
    );
    assert!(text.contains("weekly_scoped ???"), "{text:?}");
    assert!(!text.contains("[2J") && !text.contains('\r') && !text.contains('\u{1b}'));
}

#[test]
fn quoted_and_raw_env_assignments_read_the_same_custom_claude_home() {
    let root = rig("claude-profile-prefix");
    let old = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
    let default_cache = old
        .replace("[REDACTED]", "old-account")
        .replace("\"percent\": 66", "\"percent\": 11");
    let custom_cache = old
        .replace("[REDACTED]", "new-account")
        .replace("\"percent\": 66", "\"percent\": 77");
    std::fs::write(root.join(".claude.json"), default_cache.as_bytes()).expect("default cache");
    std::fs::create_dir_all(root.join(".claude-work")).expect("work cache dir");
    std::fs::write(
        root.join(".claude-work/.claude.json"),
        custom_cache.as_bytes(),
    )
    .expect("custom cache");
    std::fs::write(
        root.join("config"),
        "[profiles]\nquoted = env \"CLAUDE_CONFIG_DIR=$HOME/.claude-work\" claude\nraw = env CLAUDE_CONFIG_DIR=$HOME/.claude-work claude\n",
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("quoted raw")
            && text.contains("claude · ~/.claude-work")
            && text.contains("77%"),
        "{text}"
    );
    assert!(!text.contains("11%"), "{text}");
}

#[test]
fn bare_quoted_assignment_is_an_unknown_command_not_a_home_prefix() {
    let root = rig("claude-bare-quoted");
    std::fs::write(
        root.join("config"),
        "[profiles]\ncontrol = \"CLAUDE_CONFIG_DIR=$HOME/.claude-mic\" claude\n",
    )
    .expect("profile config");
    let text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        text.contains("control")
            && text.contains("unknown · unknown")
            && text.contains("unsupported"),
        "{text}"
    );
    assert!(!text.contains("66%") && !text.contains("77%"), "{text}");
}

#[test]
fn claude_account_switch_hides_mismatched_cached_usage() {
    let root = rig("claude-account-switch");
    let fixture = String::from_utf8_lossy(include_bytes!("../fixtures/quota/claude-cache.json"));
    let mismatch = fixture
        .replacen(
            "{\n",
            "{\n  \"oauthAccount\": {\"accountUuid\": \"new-account\"},\n",
            1,
        )
        .replace("[REDACTED]", "old-account")
        .replace("\"percent\": 66", "\"percent\": 11");
    let same = mismatch.replace("old-account", "new-account");
    std::fs::write(root.join(".claude.json"), same.as_bytes()).expect("same cache");
    let same_text = run_quota(&root);
    std::fs::write(root.join(".claude.json"), mismatch.as_bytes()).expect("mismatch cache");
    let mismatch_text = run_quota(&root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(same_text.contains("11%"), "{same_text}");
    assert!(!mismatch_text.contains("11%"), "{mismatch_text}");
    assert!(mismatch_text.contains("unknown"), "{mismatch_text}");
}

/// One of every `.rs` file in the crate, for a boundary that must see the whole
/// tree rather than the two files that happen to be under review.
///
/// Every failure here is LOUD. A directory that cannot be enumerated or a file
/// that cannot be read would otherwise read as a compliant one, which is the
/// one result this guard must never produce.
fn crate_sources() -> Vec<PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    let mut pending = vec![root.join("src"), root.join("tests")];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|why| panic!("{} cannot be enumerated: {why}", dir.display()));
        for entry in entries {
            let entry = entry
                .unwrap_or_else(|why| panic!("{} has an unreadable entry: {why}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

/// The declaration of one item, from its header to the line that closes it.
fn declaration_of(text: &str, header: &str) -> String {
    let start = text
        .find(header)
        .unwrap_or_else(|| panic!("{header} is no longer declared"));
    let rest = &text[start..];
    let end = rest.find("\n}").unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// Where a file sits relative to the module that owns the quota boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Inside the `quota` tree. Rust privacy admits a descendant, and it cannot
    /// express "visible to the parent but not to a sibling", so here the guard
    /// below is the WHOLE enforcement and it must see every ordinary spelling.
    Child,
    /// Everywhere else. The items are private to `quota`, so the compiler
    /// refuses each reach; the guard watches the qualified spellings for a
    /// widening of that visibility, which is how they became reachable before.
    Outside,
}

/// Whether `text` reaches past the quota boundary, and by which spelling.
///
/// Needles are assembled from fragments so this file does not match itself, and
/// the DECLARATION of an item is not a reach: an occurrence introduced by
/// `struct`, `enum` or `union` is skipped, which is what keeps another module's
/// unrelated `Policy` or `Classified` out of the `Outside` scan.
fn reaches_past_boundary(text: &str, reach: Reach) -> Option<String> {
    let needles: Vec<(&str, String)> = match reach {
        Reach::Child => vec![
            ("a policy constructor", ["Policy", "::new("].concat()),
            ("a policy assembled", ["Policy", " {"].concat()),
            ("a reading assembled", ["Reading", " {"].concat()),
            ("a level bound by hand", ["Classified", " {"].concat()),
            ("the threshold", ["super::", "classify("].concat()),
            ("the threshold", ["quota::", "classify("].concat()),
            // A descendant can WRITE a private field too, so default-then-set
            // is the same reach as assembling the value outright.
            ("a declaration written", [".manual_resets", " ="].concat()),
            ("an account written", [".account", " ="].concat()),
            ("a level written", [".level", " ="].concat()),
            ("a held row written", [".reading", " ="].concat()),
        ],
        Reach::Outside => vec![
            ("a policy constructor", ["quota::", "Policy::new("].concat()),
            ("a policy assembled", ["quota::", "Policy {"].concat()),
            ("a reading assembled", ["quota::", "Reading {"].concat()),
            (
                "a level bound by hand",
                ["quota::", "Classified {"].concat(),
            ),
            ("the threshold", ["quota::", "classify("].concat()),
        ],
    };
    for (form, needle) in needles {
        let mut from = 0;
        while let Some(at) = text[from..].find(&needle) {
            let start = from + at;
            let introducer = text[..start].split_whitespace().next_back().unwrap_or("");
            let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
            let prefix = &text[line_start..start];
            // A brace that opens a FUNCTION BODY is not an assembled value, so
            // a signature naming one of these types as its return type passes.
            let signature = prefix.contains("->") || prefix.contains("fn ");
            if !matches!(introducer, "struct" | "enum" | "union") && !signature {
                return Some(format!("{form} ({needle})"));
            }
            from = start + needle.len();
        }
    }
    if reach == Reach::Child {
        for statement in use_statements(text) {
            if (statement.contains("super::") || statement.contains("quota::"))
                && statement
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|word| word == "classify")
            {
                return Some(format!("the threshold, imported ({statement})"));
            }
        }
    }
    None
}

/// Every `use` statement in one file, each flattened onto one line, because the
/// import that matters is routinely written across several.
fn use_statements(text: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("use ") {
        rest = &rest[at..];
        let end = rest.find(';').unwrap_or(rest.len());
        statements.push(rest[..end].split_whitespace().collect::<Vec<_>>().join(" "));
        rest = rest.get(end + 1..).unwrap_or("");
    }
    statements
}

/// The calibration samples, as data outside the scanned tree.
fn boundary_samples() -> Vec<(bool, Reach, String, String)> {
    let text = include_str!("../fixtures/quota/boundary-samples.txt");
    let mut samples: Vec<(bool, Reach, String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let marker = line
            .strip_prefix("catch ")
            .map(|rest| (true, rest))
            .or_else(|| line.strip_prefix("allow ").map(|rest| (false, rest)));
        if let Some((expected, rest)) = marker {
            let (scope, form) = rest
                .split_once(' ')
                .unwrap_or_else(|| panic!("a marker names no scope: {line}"));
            let reach = match scope {
                "child" => Reach::Child,
                "outside" => Reach::Outside,
                other => panic!("unknown scope {other}: {line}"),
            };
            samples.push((expected, reach, form.to_owned(), String::new()));
        } else if let Some(open) = samples.last_mut() {
            open.3.push_str(line);
            open.3.push('\n');
        } else {
            assert!(
                line.trim().is_empty(),
                "the sample file opens with a line belonging to no entry: {line}"
            );
        }
    }
    assert!(
        samples.len() >= 20,
        "the calibration lost samples: {}",
        samples.len()
    );
    samples
}

/// The quota boundary, asked of the tree rather than remembered.
///
/// The rule is that a LEVEL and the observation it was decided from cannot be
/// supplied separately, and that a policy cannot be assembled from a
/// declaration plus somebody's account facts. For every module OUTSIDE the
/// `quota` tree that is the COMPILER's answer: the constructors and the
/// threshold are private, the fields are private, and each renderer takes one
/// value. Inside the tree it is a CONVENTION, and this test is the whole of its
/// enforcement — Rust privacy admits a descendant, and it has no way to say
/// "visible to the parent but not to a sibling", so `quota::codex` and
/// `quota::claude` retain the access their parent has.
///
/// A guard nobody has proved is not a guard, so every shape is calibrated
/// against `tests/fixtures/quota/boundary-samples.txt` before the tree is
/// scanned: each spelling a child module would actually use must trip it, and
/// each benign form must not. A compile-fail harness would prove the compiler
/// half directly, but it needs a dev-dependency, and adding one is a ruling
/// rather than a commit.
#[test]
fn the_quota_surface_cannot_pair_a_level_with_an_observation_it_did_not_judge() {
    for (expected, reach, form, sample) in boundary_samples() {
        let found = reaches_past_boundary(&sample, reach);
        assert_eq!(
            found.is_some(),
            expected,
            "{reach:?} {form}: the guard answered {found:?} for\n{sample}"
        );
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let quota = std::fs::read_to_string(root.join("src/quota.rs")).expect("src/quota.rs is read");
    let watchdog = std::fs::read_to_string(root.join("src/watchdog_daemon.rs"))
        .expect("src/watchdog_daemon.rs is read");

    // A policy is CONSTRUCTED nowhere but inside the one file that reads a
    // scope. Private fields closed assignment only; this closes construction.
    assert!(
        !quota.contains("pub(crate) const fn new("),
        "the policy constructor is private to src/quota.rs"
    );
    assert!(
        quota.contains("#[cfg(test)]\n    pub(crate) const fn for_tests("),
        "the only wider constructor exists in test builds alone, so a product \
         caller does not compile"
    );

    // Nothing else in the tree — descendant, sibling or test — classifies a
    // percentage or assembles a policy, a reading or a level.
    let mut seen = Vec::new();
    for path in crate_sources() {
        let name = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        seen.push(name.clone());
        if name == "src/quota.rs" {
            continue;
        }
        let reach = if name.starts_with("src/quota/") {
            Reach::Child
        } else {
            Reach::Outside
        };
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|why| panic!("{name} cannot be read: {why}"));
        assert_eq!(
            reaches_past_boundary(&text, reach),
            None,
            "{name} reaches past the quota boundary"
        );
    }

    // An unseen file is not a compliant one, so the scan says what it saw — and
    // the descendants are the whole reason this guard exists.
    for required in [
        "src/quota.rs",
        "src/quota/codex.rs",
        "src/quota/claude.rs",
        "src/watchdog_daemon.rs",
        "tests/it/quota.rs",
    ] {
        assert!(
            seen.iter().any(|name| name == required),
            "{required} was never visited, so this guard proved nothing about it: {seen:?}"
        );
    }
    assert_eq!(
        seen.iter()
            .filter(|name| name.starts_with("src/quota/"))
            .count(),
        2,
        "the quota tree gained or lost a descendant, and each one needs the \
         child scan: {seen:?}"
    );
    assert!(
        seen.len() > 40,
        "the crate has more sources than this scan saw: {}",
        seen.len()
    );

    // The renderer takes a classified reading and nothing else: there is no
    // free-form state to pair with another observation's numbers.
    assert!(
        !quota.contains("state: &str"),
        "no advisory takes a state beside its reading"
    );

    // Nothing holds a level beside a reading, which is the pair that produced
    // the same defect twice.
    for header in ["struct QuotaTracked {", "struct QuotaReadout<'a> {"] {
        let block = declaration_of(&watchdog, header);
        assert!(
            !block.contains("level:"),
            "{header} still carries its own level: {block}"
        );
    }
}
