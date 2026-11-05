//! Frozen #296 part B acceptance. Oracle: brief invariants + lead rulings P1-P7/P2a.
//! Authored by presetreseat-nav (OpenAI/gpt-6.1-sol) before implementation.
//! External fake harnesses report their own executable, account and cwd; real ae
//! performs admission, stop, copy, metadata replacement, launch and resume.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect private account homes and fail loudly"
)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::seat_relaunch::Rig;

const SLOT: &str = "spawned.0";
const ID: &str = "22222222-2222-4222-8222-222222222222";
const BODY: &[u8] = b"synthetic user\nsynthetic assistant\n";
const PAIRS: &str = "preset = alt, codex-alt, unknown, missing-client, unproven\nnext = alt\nmissing-profile = alt\nbad = alt\n";

fn root(rig: &Rig) -> PathBuf {
    std::fs::canonicalize(&rig.scratch).expect("private canonical scratch")
}

fn account(rig: &Rig, name: &str) -> PathBuf {
    root(rig).join(format!("home-{name}"))
}

fn put(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
    std::fs::write(path, body).expect("fixture bytes");
}

fn row(rig: &Rig, key: &str, value: Option<&str>) {
    let prefix = format!("{key}=");
    let mut after = rig
        .meta()
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .fold(String::new(), |mut after, line| {
            writeln!(after, "{line}").expect("string");
            after
        });
    if let Some(value) = value {
        writeln!(after, "{key}={value}").expect("string");
    }
    std::fs::write(rig.dir.join("meta"), after).expect("private meta");
}

fn configured(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    for name in ["a", "b"] {
        std::fs::create_dir_all(account(&rig, name)).expect("private account");
    }
    std::os::unix::fs::symlink("missing-store", rig.scratch.join("unproven"))
        .expect("private dangling store cannot be proved");
    let script = rig.scratch.join("claude.pl");
    let before = std::fs::read_to_string(&script).expect("fake script");
    let signed = before.replacen(
        "close($log);",
        "print $log \"ACCOUNT=\", ($ENV{CLAUDE_CONFIG_DIR} // \"<unset>\"), \" EXEC=$^X\\n\";\nclose($log);",
        1,
    );
    assert_ne!(signed, before, "external receipt instrument was applied");
    std::fs::write(script, signed).expect("signed harness");
    let original = std::fs::read_to_string(rig.scratch.join("config")).expect("fixture config");
    let tools = rig.scratch.join("tools");
    // Unknown executable never runs; its basename must be unknown to ae.
    std::fs::copy(tools.join("claude"), tools.join("mystery")).expect("unknown fake");
    let extra = format!(
        "\n[clients]\nclaude = {tools}/claude config_home={a}\nalt = {tools}/claude config_home={b}\nother = {tools}/claude config_home={b}\ncodex-alt = {tools}/codex config_home={b}\nunknown = {tools}/mystery\nunproven = {tools}/claude config_home={u}\n\n[profiles]\npreset = \"claude {script} --model preset\"\nnext = \"claude {script} --model next\"\nbad = \"claude {script} ; second-command\"\n\n[client_pairs]\n{PAIRS}",
        tools = tools.display(),
        a = account(&rig, "a").display(),
        b = account(&rig, "b").display(),
        u = rig.scratch.join("unproven").display(),
        script = rig.scratch.join("claude.pl").display(),
    );
    std::fs::write(rig.scratch.join("config"), format!("{original}{extra}"))
        .expect("global identity and approved list");
    row(
        &rig,
        "work_dir",
        Some(&root(&rig).join("work").display().to_string()),
    );
    rig
}

fn isolate(rig: &Rig, pane: &str) {
    let home = root(rig).join("isolated-home");
    std::fs::create_dir_all(&home).expect("private HOME");
    assert!(
        rig.tmux(&[
            "respawn-pane",
            "-k",
            "-t",
            pane,
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
        "scratch shell environment"
    );
    super::support::pane_command(
        |args| rig.tmux(args),
        pane,
        std::time::Instant::now() + super::support::PANE_WAIT,
        super::support::is_shell,
    );
}

fn start(rig: &Rig, profile: &str, client: Option<&str>, tool: &str) -> String {
    rig.seat_rows(SLOT, "scout", "claude-a", tool);
    row(rig, "profile.spawned.0", Some(profile));
    row(rig, "client.spawned.0", client);
    let pane = rig.new_pane(SLOT, "scout");
    isolate(rig, &pane);
    assert!(
        std::panic::catch_unwind(|| rig.start(&pane, SLOT, tool)).is_ok(),
        "source fixture did not start; frame={}\nmeta={}",
        rig.capture(&pane),
        rig.meta()
    );
    pane
}

fn reseat(rig: &Rig, using: &str) -> (Option<i32>, String, String) {
    rig.run_top(
        &rig.main_pane,
        &["reseat", &rig.session, "scout", "--using", using],
    )
}

fn must_move(rig: &Rig, using: &str) -> String {
    let (code, out, err) = reseat(rig, using);
    assert_eq!(code, Some(0), "out={out}\nerr={err}\nmeta={}", rig.meta());
    assert!(out.contains("reseated scout"), "{out}");
    out
}

fn refuse(rig: &Rig, pane: &str, using: &str, reason: &str, absent: &str) {
    let before = (
        rig.meta(),
        rig.events(),
        rig.launched(),
        rig.tool_pid(pane, "claude"),
    );
    assert!(before.3.is_some(), "live predecessor before admission");
    let (code, out, err) = reseat(rig, using);
    assert_ne!(code, Some(0), "refused {using}: out={out}, err={err}");
    assert!(
        err.contains(reason),
        "{using}: expected {reason:?}, out={out}, err={err}"
    );
    if !absent.is_empty() {
        assert!(
            !err.contains(absent),
            "policy must precede existence: {err}"
        );
    }
    assert_eq!(rig.meta(), before.0, "refusal changed metadata: {err}");
    assert_eq!(rig.events(), before.1, "refusal wrote an event: {err}");
    assert_eq!(
        rig.launched(),
        before.2,
        "refusal started another tool: {err}"
    );
    assert_eq!(
        rig.tool_pid(pane, "claude"),
        before.3,
        "refusal stopped predecessor: {err}"
    );
}

fn transcript(rig: &Rig) -> PathBuf {
    // Independent spelling of Claude's project key, including punctuation.
    let work = root(rig).join("work");
    let key: String = work
        .to_string_lossy()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    PathBuf::from("projects")
        .join(key)
        .join(format!("{ID}.jsonl"))
}

fn history(rig: &Rig, name: &str) {
    row(rig, "harness_session.spawned.0", Some(ID));
    row(rig, "capture_floor.spawned.0", Some("100"));
    row(
        rig,
        "config_home.spawned.0",
        Some(&account(rig, name).display().to_string()),
    );
    row(rig, "config_home_base.spawned.0", None);
    put(&account(rig, name).join(transcript(rig)), BODY);
}

fn assert_identity(rig: &Rig, client: Option<&str>, home: &str) {
    let meta = rig.meta();
    let rows: Vec<_> = meta
        .lines()
        .filter(|line| line.starts_with("client.spawned.0="))
        .collect();
    match client {
        Some(label) => assert_eq!(rows, vec![format!("client.spawned.0={label}")]),
        None => assert!(
            rows.is_empty(),
            "bare removes client instead of emptying it: {meta}"
        ),
    }
    assert_eq!(
        rig.meta_row("config_home.spawned.0"),
        account(rig, home).display().to_string()
    );
    assert!(
        !meta.contains("config_home_base.spawned.0="),
        "explicit account has no base: {meta}"
    );
    assert_eq!(rig.meta_row("agent_bin.spawned.0"), "claude");
    assert_eq!(rig.meta_row("seat.spawned.0"), "scout");
}

#[test]
fn presetreseat_fresh_pair_publishes_bare_profile_and_runs_override_account_in_work_dir() {
    let rig = configured("prfresh");
    let pane = start(&rig, "fake-opencode", None, "opencode");
    rig.kill_tools(&pane);
    let before = rig.meta_row("launch_id.spawned.0");
    must_move(&rig, "preset@alt");
    assert_identity(&rig, Some("alt"), "b");
    assert_eq!(rig.meta_row("profile.spawned.0"), "preset");
    assert_ne!(rig.meta_row("launch_id.spawned.0"), before);
    assert!(
        rig.tool_pid(&pane, "claude").is_some(),
        "same pane runs successor"
    );
    let log = rig.launched();
    assert!(
        log.contains(&format!("ACCOUNT={} EXEC=", account(&rig, "b").display())),
        "{log}"
    );
    assert!(
        log.contains(&format!("cwd={}", root(&rig).join("work").display())),
        "{log}"
    );
    let seed = rig.dir.join("seed.scout.md");
    assert_eq!(
        std::fs::metadata(seed)
            .expect("published seed")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        rig.events().contains("from fake-opencode to preset@alt"),
        "{}",
        rig.events()
    );
}

#[test]
fn presetreseat_same_profile_other_account_carries_then_bare_same_profile_returns_home() {
    let rig = configured("prcarry");
    let pane = start(&rig, "preset", None, "claude");
    history(&rig, "a");
    let out = must_move(&rig, "preset@alt");
    assert!(out.contains("carried conversation"), "{out}");
    assert_identity(&rig, Some("alt"), "b");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    assert_eq!(rig.meta_row("capture_floor.spawned.0"), "100");
    assert_eq!(
        std::fs::read(account(&rig, "b").join(transcript(&rig))).expect("carried transcript"),
        BODY
    );
    assert_eq!(
        std::fs::read(account(&rig, "a").join(transcript(&rig))).expect("source untouched"),
        BODY
    );
    assert!(
        rig.launched().contains(&format!("--resume {ID}")),
        "{}",
        rig.launched()
    );
    assert!(rig.tool_pid(&pane, "claude").is_some());

    // P2 named exception: bare same-profile clears a usable recorded client.
    let out = must_move(&rig, "preset");
    assert!(out.contains("carried conversation"), "{out}");
    assert_identity(&rig, None, "a");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    assert!(
        rig.events().contains("from preset@alt to preset"),
        "{}",
        rig.events()
    );
    refuse(&rig, &pane, "preset", "already runs profile 'preset'", "");
}

#[test]
fn presetreseat_kept_pair_keeps_conversation_and_relaunch_uses_recorded_account() {
    let rig = configured("prkeep");
    let pane = start(&rig, "preset", Some("alt"), "claude");
    history(&rig, "b");
    let out = must_move(&rig, "next@alt");
    assert!(out.contains("kept conversation"), "{out}");
    assert_identity(&rig, Some("alt"), "b");
    assert_eq!(rig.meta_row("harness_session.spawned.0"), ID);
    assert_eq!(rig.meta_row("capture_floor.spawned.0"), "100");
    assert!(
        !rig.meta().contains("harness_session_prior.spawned.0="),
        "{}",
        rig.meta()
    );
    assert!(
        !rig.meta().contains("observed_model.spawned.0="),
        "stale drift erased"
    );
    rig.kill_tools(&pane);
    std::fs::write(rig.scratch.join("launched"), "").expect("clean resume receipt");
    let (code, out, err) = rig.run("_relaunch", &["scout"]);
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(
        rig.launched()
            .contains(&format!("ACCOUNT={} EXEC=", account(&rig, "b").display())),
        "{}",
        rig.launched()
    );
    assert!(
        rig.launched().contains(&format!("--resume {ID}")),
        "{}",
        rig.launched()
    );
    refuse(&rig, &pane, "next@alt", "already runs", "");
}

#[test]
fn presetreseat_refusals_run_before_stop_and_policy_precedes_existence() {
    let rig = configured("prrefuse");
    let pane = start(&rig, "fake-claude-a", None, "claude");
    for (using, reason, absent) in [
        ("preset@", "not <profile> or <profile>@<client>", ""),
        ("@alt", "not <profile> or <profile>@<client>", ""),
        ("preset@alt@alt", "not <profile> or <profile>@<client>", ""),
        ("preset@other", "not an approved pair", ""),
        (
            "preset@undefined",
            "not an approved pair",
            "not defined in [clients]",
        ),
        (
            "undefined@alt",
            "not an approved pair",
            "not defined in [profiles]",
        ),
        ("missing-profile@alt", "not defined in [profiles]", ""),
        ("preset@missing-client", "not defined in [clients]", ""),
        ("preset@codex-alt", "harness adapter", ""),
        ("preset@unknown", "unrecognized", ""),
        ("preset@unproven", "cannot name the account", ""),
        ("bad@alt", "one simple command", ""),
    ] {
        refuse(&rig, &pane, using, reason, absent);
    }
}

#[test]
fn presetreseat_void_retired_and_unreadable_global_never_approve_a_pair() {
    let rig = configured("prvoid");
    let pane = start(&rig, "fake-claude-a", None, "claude");
    let path = rig.scratch.join("config");
    let full = std::fs::read_to_string(&path).expect("global config");
    for pairs in [
        "",
        "preset = alt\npreset = alt\n",
        "preset = alt,,other\n",
        "preset = other\n",
    ] {
        let body = full.replace(PAIRS, pairs);
        assert_ne!(body, full, "approval mutation applied");
        std::fs::write(&path, body).expect("void/retired approval fixture");
        refuse(&rig, &pane, "preset@alt", "pair", "");
    }
    std::fs::write(&path, &full).expect("restore fixture config");
    std::fs::remove_file(&path).expect("unreadable global fixture");
    std::fs::create_dir(&path).expect("a directory is not a readable config file");
    refuse(&rig, &pane, "preset@alt", "cannot be read", "");
}

#[test]
fn presetreseat_project_overlay_cannot_approve_or_redefine_a_pair() {
    let rig = configured("proverlay");
    let pane = start(&rig, "fake-opencode", None, "opencode");
    rig.kill_tools(&pane);
    let global = rig.scratch.join("config");
    let full = std::fs::read_to_string(&global).expect("global config");
    put(
        &rig.scratch.join(".ae/config"),
        b"[client_pairs]\npreset = alt\n",
    );
    std::fs::write(&global, full.replace(PAIRS, "preset = other\n")).expect("retired global route");
    let before = (rig.meta(), rig.events());
    let (code, out, err) = reseat(&rig, "preset@alt");
    assert_ne!(code, Some(0), "{out}{err}");
    assert!(err.contains("not an approved pair"), "{err}");
    assert_eq!((rig.meta(), rig.events()), before);
    std::fs::write(global, full).expect("global approval restored");
    put(
        &rig.scratch.join(".ae/config"),
        format!(
            "[clients]\nalt = {} config_home={}\n[profiles]\npreset = \"{} {} --model hijacked\"\n",
            rig.scratch.join("tools/codex").display(),
            account(&rig, "a").display(),
            rig.scratch.join("tools/claude").display(),
            rig.scratch.join("claude.pl").display(),
        )
        .as_bytes(),
    );
    must_move(&rig, "preset@alt");
    assert_identity(&rig, Some("alt"), "b");
    assert!(
        rig.launched().contains("--model preset"),
        "global preset ran: {}",
        rig.launched()
    );
    assert!(
        !rig.launched().contains("--model hijacked"),
        "{}",
        rig.launched()
    );
}

#[test]
fn presetreseat_bare_other_profile_clears_client_and_invalid_same_profile_still_refuses() {
    let rig = configured("prbare");
    let pane = start(&rig, "preset", Some("alt"), "claude");
    history(&rig, "b");
    must_move(&rig, "next");
    assert_identity(&rig, None, "a");
    assert_eq!(rig.meta_row("profile.spawned.0"), "next");
    // P2b: Invalid earns no new recovery behavior in this slice.
    row(&rig, "client.spawned.0", Some(""));
    refuse(&rig, &pane, "next", "already runs profile 'next'", "");
    rig.kill_tools(&pane);
    let before = (rig.meta(), rig.events(), rig.launched());
    let (code, out, err) = reseat(&rig, "preset");
    assert_ne!(code, Some(0), "{out}{err}");
    assert!(err.contains("records an unusable client"), "{err}");
    assert_eq!((rig.meta(), rig.events(), rig.launched()), before);
}

#[test]
fn presetreseat_pair_on_invalid_client_is_refused_before_stop() {
    let rig = configured("prinvalid");
    let pane = start(&rig, "preset", None, "claude");
    row(&rig, "client.spawned.0", Some(""));
    refuse(&rig, &pane, "preset@alt", "records an unusable client", "");
}

#[test]
fn presetreseat_roster_command_reads_the_reseated_override() {
    let rig = configured("prroster");
    let pane = start(&rig, "fake-opencode", None, "opencode");
    rig.kill_tools(&pane);
    must_move(&rig, "preset@alt");
    let path = rig.scratch.join("config").display().to_string();
    let (code, out, err) = rig.run(ae::cli::ROSTER, &["list", "--global", &path]);
    assert_eq!(code, Some(0), "{out}{err}");
    let scout = out
        .lines()
        .find(|line| line.contains("scout"))
        .expect("reseated scout in roster");
    assert!(
        scout.contains(&account(&rig, "b").display().to_string()),
        "{scout}"
    );
}

#[test]
fn presetreseat_auto_move_judges_recorded_override_quota_and_lands_on_bare_profile() {
    let rig = configured("prauto");
    let pane = start(&rig, "fake-opencode", None, "opencode");
    rig.kill_tools(&pane);
    must_move(&rig, "preset@alt");
    history(&rig, "b");
    let path = rig.scratch.join("config");
    let config = std::fs::read_to_string(&path).expect("fixture config");
    std::fs::write(&path, format!(
        "{config}\n[workspace]\nauto_reseat = on\nauto_reseat_grace_secs = 0\nauto_reseat_at = 95\n[auto_reseat]\npreset = next\n"
    )).expect("bare automatic destination");
    let now = ae::time::Timestamp::now().epoch();
    for (name, percent) in [("a", 5), ("b", 98)] {
        put(&account(&rig, name).join(".claude.json"), format!(
            "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{percent},\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
            now * 1_000, ae::time::Timestamp::from_epoch(now + 86_400),
        ).as_bytes());
    }
    let key = ae::time::Timestamp::from_epoch(now - 20);
    let at = ae::time::Timestamp::from_epoch(now - 19);
    let events = format!(
        "{}{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"scout\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{}\",\"summary\":\"headroom 98% weekly_all 7d\"}}\n{{\"ts\":\"{at}\",\"actor\":\"watchdog\",\"action\":\"{}\",\"target\":\"scout\",\"target_slot\":\"{SLOT}\",\"target_session\":\"{}\",\"ref\":\"{key}\"}}\n",
        rig.events(),
        rig.session,
        ae::autoreseat::ATTEMPT_ACTION,
        rig.session,
    );
    std::fs::write(rig.dir.join("events.jsonl"), events).expect("pending headroom attempt");
    let out = super::cli::ae()
        .env("HOME", root(&rig).join("isolated-home"))
        .env("AE_HOME", &rig.scratch)
        .env("CONFIG_FILE", &path)
        .env("TMUX", format!("{},0,0", rig.sock.display()))
        .env("TMUX_PANE", &rig.main_pane)
        .env_remove("AE_SENDER_OVERRIDE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .arg("_auto-reseat")
        .arg(&rig.dir)
        .args([SLOT, &key.to_string()])
        .output()
        .expect("real automatic leg");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        rig.meta_row("profile.spawned.0"),
        "next",
        "override quota must bind despite default account's 5%: {out:?}\n{}",
        rig.events()
    );
    assert_identity(&rig, None, "a");
    assert!(rig.tool_pid(&pane, "claude").is_some());
    assert!(
        rig.events().lines().any(|line| {
            ae::events::Event::parse_line(line)
                .is_ok_and(|event| event.action == ae::autoreseat::DONE_ACTION)
        }),
        "the real leg recorded completion: {}",
        rig.events()
    );
}
