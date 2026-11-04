//! Frozen characterization of `_compact` orchestration at base 550c4cd7.
//! Oracle: commands.md's reboot contract where stated; other exact strings and
//! refusal ordering are explicitly current-base behavior pins, not bug fixes.
//! Real tmux is confined by the existing `lifecycle::Rig` and isolated lane.

use super::*;
use std::collections::BTreeMap;
use std::io::Write as _;

const USAGE: &str =
    "Usage: _compact [-f] [--keep-history] [--digest-only] <session-name> [--exec-plan <path>]";
const REFERENCE: &str = "ae-20260829T000000Z-abcd1234";

/// Three independent bits: force, keep history, digest only.
fn argv(rig: &Rig, flags: u8) -> Vec<&str> {
    let mut args = vec!["_compact", rig.name.as_str()];
    for (bit, flag) in [(1, "-f"), (2, "--keep-history"), (4, "--digest-only")] {
        if flags & bit != 0 {
            args.push(flag);
        }
    }
    args
}

fn set_row(rig: &Rig, key: &str, value: &str) {
    let path = rig.dir.join("meta");
    let bytes = std::fs::read_to_string(&path).expect("fixture meta");
    let mut rows: Vec<String> = bytes
        .lines()
        .filter(|row| row.split_once('=').is_none_or(|(k, _)| k != key))
        .map(str::to_owned)
        .collect();
    rows.push(format!("{key}={value}"));
    std::fs::write(path, format!("{}\n", rows.join("\n"))).expect("fixture row");
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_owned(),
                    std::fs::read(path).expect("fixture bytes"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(dir, dir, &mut files);
    files
}

fn preserved(rig: &Rig, before: &BTreeMap<PathBuf, Vec<u8>>) {
    assert_eq!(&snapshot(&rig.dir), before, "source files unchanged");
    assert!(rig.session_is_live(), "source tmux session remains live");
    assert!(!rig.archive().exists(), "no archive on refusal");
}

fn facts(rig: &Rig) -> String {
    format!(
        "reboot: {} (local) resolved and frozen.\nsession: {}\nuuid: {UUID} (session)\n\
         mode: local\norigin: {}\nconfig: {}\nconversation files: kept\narchive: {}\n",
        rig.name,
        rig.name,
        rig.home.display(),
        rig.home.join("config").display(),
        rig.archive().display()
    )
}

fn confirmation(rig: &Rig, digest: bool) -> String {
    let handover = if digest {
        "  - No handover will be requested (--digest-only); the digest is the handover."
    } else {
        "  - Your main agent will be asked to hand over first, and must reply AND write a handover memo."
    };
    format!(
        "Reboot session '{}'?\n  - Session id: {UUID} (session), mode: local\n  - Origin:     {}\n\
         \x20 - ARCHIVES its memory to {}/, then ENDS it.\n\
         \x20 - Then starts a FRESH session '{}' that inherits that archive's digest.\n\
         \x20 - The fresh session starts the roster your config names now: main=fake workers=-\n\
         \x20 - NOT carried over: provider conversations, panes, spawned agents, launch scratch.\n\
         \x20 - KEPT: the archive, and the claude/codex/agy conversation files on disk.\n\
         {handover}\nContinue? [y/N] ",
        rig.name,
        rig.home.display(),
        rig.archive().display(),
        rig.name
    )
}

fn busy(rig: &Rig) -> String {
    format!(
        "Error: another lifecycle operation is in progress for '{}' — retry shortly. Nothing was changed.\n",
        rig.name
    )
}

fn command(rig: &Rig, args: &[&str]) -> super::super::cli::Runner {
    let mut cmd = ae();
    cmd.env("AE_HOME", &rig.home)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("AE_COMPACT_HANDOVER_SECS")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

fn answered(rig: &Rig, args: &[&str], reply: &str) -> (Option<i32>, String, String) {
    let input = rig.home.join("answer");
    std::fs::write(&input, reply).expect("fixture answer");
    let output = bounded(
        command(rig, args)
            .stdin(std::fs::File::open(input).expect("fixture input"))
            .spawn()
            .expect("core starts"),
        Duration::from_secs(30),
    )
    .expect("core returned");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn compactrun_parser_refusals_precede_target_observation() {
    let rig = Rig::new("crparse");
    let before = snapshot(&rig.dir);
    let cases = [
        (vec![], format!("{USAGE}\n")),
        (vec!["--exec-plan"], "Error: --exec-plan needs a path.\n".to_owned()),
        (vec!["--purge-history"], "Error: --purge-history contradicts reboot, which exists to keep the archive.\n  To end a session and delete its archive: ae end --purge-history <name>\n".to_owned()),
        (vec!["--assume-stopped"], "Error: --assume-stopped is an 'ae end' acknowledgement; reboot stops the session itself.\n".to_owned()),
        (vec!["--from=x"], "Error: --from is not a reboot flag — reboot inherits from the archive it just wrote.\n".to_owned()),
        (vec!["all"], "Error: reboot takes one session name; 'all' has no meaning for it.\n".to_owned()),
        (vec!["use"], "Error: 'use' is not a reboot argument — the fresh session starts the roster your config names now.\n".to_owned()),
        (vec!["--bogus", "--purge-history"], format!("Error: unknown flag '--bogus'.\n{USAGE}\n")),
        (vec!["..", "extra"], "Error: unexpected extra argument 'extra' — reboot takes one session name.\n".to_owned()),
    ];
    for (tail, expected) in cases {
        let mut args = vec!["_compact"];
        args.extend(tail);
        let (code, out, err) = rig.run_inside(&args);
        assert_eq!(
            (code, out, err),
            (Some(2), String::new(), expected),
            "{args:?}"
        );
    }
    for flag in ["--local", "--copy", "--worktree"] {
        let expected = format!(
            "Error: '{flag}' is not a reboot flag — the fresh session keeps the mode the archived one had.\n"
        );
        assert_eq!(
            rig.run(&["_compact", flag]),
            (Some(2), String::new(), expected)
        );
    }
    for word in ["all", "use"] {
        let first = rig.run(&["_compact", word]);
        assert_eq!(
            rig.run(&["_compact", &rig.name, word]),
            first,
            "reserved positional wins over extra name"
        );
    }
    assert_eq!(
        rig.run(&["_compact", ".."]),
        (
            Some(1),
            String::new(),
            "ae: '..' is not a usable session name.\n".to_owned()
        )
    );
    preserved(&rig, &before);
}

#[test]
fn compactrun_current_target_beats_mode_history_and_spawned_refusals_for_all_flags() {
    let rig = Rig::new("crinside");
    set_row(&rig, "mode", "full");
    let config = rig.home.join("config");
    let text = std::fs::read_to_string(&config).expect("config");
    std::fs::write(&config, format!("{text}purge_agent_history = true\n")).expect("purge config");
    set_row(&rig, "seat.spawned.0", "helper");
    set_row(&rig, "profile.spawned.0", "fake");
    let before = snapshot(&rig.dir);
    let expected = format!(
        "Error: cannot reboot the current session. Detach, then run: ae reboot {}\n",
        rig.name
    );
    for flags in 0..8 {
        assert_eq!(
            rig.run_inside(&argv(&rig, flags)),
            (Some(1), String::new(), expected.clone()),
            "flags={flags}"
        );
    }
    preserved(&rig, &before);
}

#[test]
fn compactrun_mode_and_missing_state_refusals_ignore_every_force_keep_digest_combination() {
    let rig = Rig::new("crmodes");
    for mode in ["git", "full", "", "unknown"] {
        set_row(&rig, "mode", mode);
        set_row(&rig, "origin", "");
        let before = snapshot(&rig.dir);
        let expected = match mode {
            "git" | "full" => format!(
                "reboot: '{}' is {mode} mode; reboot is local-mode only. A fresh {mode} workspace would be based on the canonical origin's HEAD, which normally lags the session's own branch — you would get a session missing the code it just archived. Use: ae end {}, then start a new session yourself.\n",
                rig.name, rig.name
            ),
            "" => format!(
                "reboot: session '{}' records no mode — refusing to reboot a session ae cannot classify.\n",
                rig.name
            ),
            _ => format!(
                "reboot: session '{}' records an unknown mode '{mode}'.\n",
                rig.name
            ),
        };
        for flags in 0..8 {
            assert_eq!(
                rig.run(&argv(&rig, flags)),
                (Some(1), String::new(), expected.clone()),
                "mode={mode:?}, flags={flags}"
            );
        }
        preserved(&rig, &before);
    }
    for flags in 0..8 {
        let args = argv(&rig, flags);
        let args: Vec<&str> = args
            .iter()
            .map(|a| if *a == rig.name { "missing" } else { *a })
            .collect();
        assert_eq!(
            rig.run(&args),
            (
                Some(1),
                String::new(),
                "reboot: no ae session state for 'missing' — nothing to reboot.\n".to_owned()
            )
        );
    }
}

#[test]
fn compactrun_history_override_reaches_confirmation_then_spawn_refusal() {
    let rig = Rig::new("crpurge");
    let config = rig.home.join("config");
    let original = std::fs::read_to_string(&config).expect("config");
    std::fs::write(&config, format!("{original}purge_agent_history = true\n"))
        .expect("purge config");
    set_row(&rig, "seat.spawned.0", "helper");
    set_row(&rig, "profile.spawned.0", "fake");
    let before = snapshot(&rig.dir);
    for flags in 0..8 {
        let (code, out, err) = rig.run(&argv(&rig, flags));
        assert_eq!(code, Some(1), "flags={flags}: {err}");
        assert!(out.is_empty());
        if flags & 2 == 0 {
            assert_eq!(
                err,
                format!(
                    "reboot: session '{}' has purge_agent_history enabled, which contradicts reboot. To proceed: ae reboot --keep-history {}.\n",
                    rig.name, rig.name
                )
            );
        } else if flags & 1 == 0 {
            assert_eq!(
                err,
                format!(
                    "{}{}Error: could not obtain confirmation — no input on stdin.\n  Nothing was stopped, archived or changed. Run it from a terminal, or\n  pass -f if you mean to proceed without being asked.\n",
                    facts(&rig),
                    confirmation(&rig, flags & 4 != 0)
                )
            );
        } else {
            assert_eq!(
                err,
                format!(
                    "{}reboot: '{}' still has spawned agents (after confirmation): helper. reboot never retires someone else's worker — retire them, then re-run.\n",
                    facts(&rig),
                    rig.name
                )
            );
        }
        preserved(&rig, &before);
    }
}

#[test]
fn compactrun_decline_and_eof_keep_exact_prompt_and_distinct_exit_codes() {
    let rig = Rig::new("cranswer");
    let before = snapshot(&rig.dir);
    for flags in [0, 2, 4, 6] {
        for answer in ["n\n", "\n", "not yes\n"] {
            assert_eq!(
                answered(&rig, &argv(&rig, flags), answer),
                (
                    Some(0),
                    String::new(),
                    format!(
                        "{}{}Aborted.\n",
                        facts(&rig),
                        confirmation(&rig, flags & 4 != 0)
                    )
                )
            );
        }
        let expected = format!(
            "{}{}Error: could not obtain confirmation — no input on stdin.\n  Nothing was stopped, archived or changed. Run it from a terminal, or\n  pass -f if you mean to proceed without being asked.\n",
            facts(&rig),
            confirmation(&rig, flags & 4 != 0)
        );
        assert_eq!(
            answered(&rig, &argv(&rig, flags), ""),
            (Some(1), String::new(), expected)
        );
        preserved(&rig, &before);
    }
}

#[test]
fn compactrun_first_lock_refuses_after_facts_before_any_handover() {
    let rig = Rig::new("crlock");
    let path = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.name));
    let held = ae::store::lock(&path, Duration::ZERO).expect("hold first lock");
    let before = snapshot(&rig.dir);
    assert_eq!(
        rig.run(&argv(&rig, 7)),
        (
            Some(1),
            String::new(),
            format!("{}{}", facts(&rig), busy(&rig))
        )
    );
    preserved(&rig, &before);
    drop(held);
    let plan = rig.home.join("plan");
    let mut args = argv(&rig, 7);
    args.extend(["--exec-plan", plan.to_str().expect("plan path")]);
    assert_eq!(
        rig.run(&args).0,
        Some(0),
        "same target works when lock released"
    );
}

fn wait_for(path: &Path, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "never observed {needle:?}: {text}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_logged(
    rig: &Rig,
    args: &[&str],
    budget: &str,
) -> (super::super::cli::OwnedChild, PathBuf) {
    let log = rig.home.join("compactrun-stderr");
    let child = command(rig, args)
        .env("AE_COMPACT_HANDOVER_SECS", budget)
        .stdin(std::process::Stdio::piped())
        .stderr(std::fs::File::create(&log).expect("child stderr"))
        .spawn()
        .expect("core starts");
    (child, log)
}

/// A prompt is an observed barrier; changes below happen after the freeze and
/// before the answer. Each field's refusal is a current-base behavior pin.
#[test]
fn compactrun_revalidates_each_authorized_fact_after_the_answer_before_handover() {
    for digest in [false, true] {
        for field in [
            "uuid", "mode", "origin", "config", "purge", "roster", "spawn",
        ] {
            let rig = Rig::new("crdrift");
            let args = argv(&rig, if digest { 4 } else { 0 });
            let (mut child, log) = spawn_logged(&rig, &args, "1");
            wait_for(&log, "Continue? [y/N] ");
            let why = drift(&rig, field);
            let changed = snapshot(&rig.dir);
            child
                .stdin
                .take()
                .expect("prompt input")
                .write_all(b"y\n")
                .expect("answer");
            let output = bounded(child, Duration::from_secs(10)).expect("refusal returned");
            assert_eq!(
                output.status.code(),
                Some(1),
                "field={field}, digest={digest}"
            );
            assert!(output.stdout.is_empty());
            let err = std::fs::read_to_string(&log).expect("stderr bytes");
            assert_eq!(
                err,
                format!("{}{}{why}\n", facts(&rig), confirmation(&rig, digest)),
                "field={field}"
            );
            preserved(&rig, &changed);
        }
    }
}

fn drift(rig: &Rig, field: &str) -> String {
    let when = "after confirmation";
    let name = &rig.name;
    let config = rig.home.join("config");
    match field {
        "uuid" => {
            let other = "99999999-9999-9999-9999-999999999999";
            set_row(rig, "session_id", other);
            format!(
                "reboot: '{name}' is not the session that was authorized ({when}) — authorized id {UUID}, on disk now {other}. Nothing was stopped and nothing was archived."
            )
        }
        "mode" => {
            set_row(rig, "mode", "git");
            format!("reboot: '{name}' changed mode from local to git ({when}); refusing.")
        }
        "origin" => {
            let other = rig.home.join("other-origin");
            std::fs::create_dir(&other).expect("other valid origin");
            set_row(rig, "origin", other.to_str().expect("origin path"));
            format!(
                "reboot: '{name}' changed origin from {} to {} ({when}); refusing.",
                rig.home.display(),
                other.display()
            )
        }
        "config" => {
            let other = rig.home.join("other-config");
            std::fs::copy(&config, &other).expect("other valid config");
            set_row(rig, "config", other.to_str().expect("config path"));
            format!(
                "reboot: '{name}' changed its recorded config from '{}' to '{}' ({when}); refusing.",
                config.display(),
                other.display()
            )
        }
        "purge" => {
            let text = std::fs::read_to_string(&config).expect("config");
            std::fs::write(&config, format!("{text}purge_agent_history = true\n"))
                .expect("purge flip");
            format!(
                "reboot: '{name}' now has purge_agent_history enabled ({when}), which contradicts the authorized reboot. Re-run with --keep-history if that is what you want."
            )
        }
        "roster" => {
            let text = std::fs::read_to_string(&config).expect("config");
            std::fs::write(&config, text.replace("main = fake", "main = changed"))
                .expect("roster flip");
            format!(
                "reboot: '{name}' would now start a different roster than the one shown ({when}); refusing. Shown: main=fake workers=-. Now: main=changed workers=-. Nothing was stopped and nothing was archived."
            )
        }
        "spawn" => {
            set_row(rig, "seat.spawned.0", "helper");
            set_row(rig, "profile.spawned.0", "fake");
            format!(
                "reboot: '{name}' still has spawned agents ({when}): helper. reboot never retires someone else's worker — retire them, then re-run."
            )
        }
        _ => panic!("unknown fixture field {field}"),
    }
}

/// Legacy unbound request rows are still valid persisted product state. The
/// body carries its own baseline; rerunning must reuse both bytes and id.
fn pending(rig: &Rig) {
    let messages = rig.dir.join("messages");
    std::fs::create_dir(&messages).expect("messages");
    let body = messages.join("handover.ask.txt");
    std::fs::write(&body, "REBOOT HANDOVER\nAE-COMPACT-MEMO-BASELINE=0\n").expect("body");
    std::fs::write(rig.dir.join("events.jsonl"), format!(
        "{{\"ts\":\"2026-08-29T00:00:00Z\",\"actor\":\"ae:reboot:{UUID}\",\"action\":\"ask\",\"target\":\"fake:lead\",\"ref\":\"{REFERENCE}\",\"body_file\":\"{}\",\"actor_session\":\"{}\",\"target_slot\":\"main\",\"target_session\":\"{}\"}}\n",
        body.display(), rig.name, rig.name
    )).expect("outstanding request");
}

fn reply_and_memo(rig: &Rig) {
    let path = rig.dir.join("events.jsonl");
    let ask = std::fs::read_to_string(&path).expect("ask");
    let reply = format!(
        "{{\"ts\":\"2026-08-29T00:01:00Z\",\"actor\":\"fake:lead\",\"action\":\"reply\",\"ref\":\"{REFERENCE}\",\"actor_slot\":\"main\",\"actor_session\":\"{}\"}}\n",
        rig.name
    );
    std::fs::write(path, format!("{ask}{reply}")).expect("reply");
    std::fs::write(
        rig.dir.join("memo.tsv"),
        "2026-08-29T00:01:00Z\tfake:lead\thandover\tresume here\n",
    )
    .expect("new memo");
}

#[test]
fn compactrun_timeout_reuses_the_request_and_holds_no_lifecycle_lock_during_wait() {
    let rig = Rig::new("crwait");
    pending(&rig);
    let before = snapshot(&rig.dir);
    for _ in 0..2 {
        let (child, log) = spawn_logged(&rig, &argv(&rig, 1), "2");
        wait_for(&log, "reusing the outstanding handover request");
        let path = rig
            .home
            .join("sessions")
            .join(format!(".lifecycle.{}.lock", rig.name));
        let free = ae::store::lock(&path, Duration::ZERO).expect("wait holds no lifecycle lock");
        drop(free);
        let output = bounded(child, Duration::from_secs(10)).expect("bounded wait returned");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let err = std::fs::read_to_string(log).expect("stderr bytes");
        assert_eq!(
            err,
            format!(
                "{}reboot: reusing the outstanding handover request {REFERENCE}.\nreboot: no complete handover within 2s — a reply AND a new handover memo are both required.\n  The request stays open: re-run ae reboot {} to keep waiting on it, or ae reboot --digest-only {} to archive the digest as it stands.\n",
                facts(&rig),
                rig.name,
                rig.name
            )
        );
        preserved(&rig, &before);
    }
    let events = std::fs::read(rig.dir.join("events.jsonl")).expect("ledger");
    let requests = ae::requests::states(&events);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, ae::requests::Status::Pending);
}

fn contract(rig: &Rig) -> String {
    format!(
        "Archived {UUID}\nArchive: {}\nDigest: {}/digest.md\nRecovery: {}\n",
        rig.archive().display(),
        rig.archive().display(),
        recovery(rig)
    )
}

fn recovery(rig: &Rig) -> String {
    format!(
        "cd '{}' && ae --local '{}' --from {UUID}",
        rig.home.display(),
        rig.name
    )
}

fn crossed(rig: &Rig) {
    assert!(!rig.dir.exists(), "live source removed");
    assert!(!rig.session_is_live(), "recorded tmux session stopped");
    assert!(rig.archive().join("digest.md").is_file(), "durable archive");
    let archive_root = rig.home.join("archive");
    let (code, _, err) = rig.run(&[
        "_archive-from-preflight",
        archive_root.to_str().expect("archive root"),
        UUID,
    ]);
    assert_eq!(code, Some(0), "archive can be inherited: {err}");
    let lock = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.name));
    assert!(
        ae::store::lock(&lock, Duration::ZERO).is_ok(),
        "second lock released before boundary/relaunch"
    );
}

#[test]
fn compactrun_digest_withdraws_before_archive_and_ignores_invalid_handover_budget() {
    let rig = Rig::new("crdigest");
    pending(&rig);
    let plan = rig.home.join("plan");
    let mut args = argv(&rig, 7);
    args.extend(["--exec-plan", plan.to_str().expect("plan path")]);
    let (code, out, err) = rig.run_with_env(&[("AE_COMPACT_HANDOVER_SECS", "invalid")], &args);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out, contract(&rig));
    assert!(err.starts_with(&format!("{}reboot: withdrew {REFERENCE} (--digest-only).\nreboot: semantic handover skipped (--digest-only); the digest is the handover.\n", facts(&rig))));
    assert!(
        !err.contains("AE_COMPACT_HANDOVER_SECS"),
        "digest does not read budget"
    );
    assert!(err.ends_with(&format!("Recovery: {}\nkept agent conversation files (claude/codex/agy token history; purge: ae end --purge-history)\nStarting fresh session {} from archive {UUID}...\n", recovery(&rig), rig.name)));
    crossed(&rig);
    let archived = std::fs::read(rig.archive().join("events.jsonl")).expect("archived ledger");
    let requests = ae::requests::states(&archived);
    assert_eq!(requests.len(), 1, "no second handover");
    assert_eq!(
        requests[0].status,
        ae::requests::Status::Cancelled,
        "withdrawal archived"
    );
    let record = std::fs::read_to_string(plan).expect("exec plan");
    let fields: Vec<&str> = record.trim_end().split('\u{1f}').collect();
    assert_eq!(fields.len(), 6);
    assert_eq!(
        &fields[..4],
        &[
            rig.name.as_str(),
            UUID,
            rig.home.to_str().expect("origin"),
            rig.home.join("config").to_str().expect("config")
        ]
    );
    assert!(fields[4].starts_with(UUID), "inheritance proof");
    assert_eq!(fields[5], "main=fake workers=-");
}

#[test]
fn compactrun_invalid_budget_warns_then_accepts_reply_plus_new_memo() {
    let rig = Rig::new("crbudget");
    pending(&rig);
    let plan = rig.home.join("plan");
    let mut args = argv(&rig, 1);
    args.extend(["--exec-plan", plan.to_str().expect("plan path")]);
    let (child, log) = spawn_logged(&rig, &args, "invalid");
    wait_for(&log, "using 300.");
    reply_and_memo(&rig);
    let output = bounded(child, Duration::from_secs(15)).expect("completed handover returns");
    let err = std::fs::read_to_string(log).expect("stderr bytes");
    assert_eq!(output.status.code(), Some(0), "{err}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), contract(&rig));
    assert!(err.starts_with(&format!("{}reboot: reusing the outstanding handover request {REFERENCE}.\nnote: AE_COMPACT_HANDOVER_SECS='invalid' is not a positive number of seconds — using 300.\nreboot: handover complete (reply {REFERENCE} and a new handover memo).\n", facts(&rig))));
    crossed(&rig);
}

#[test]
fn compactrun_second_lock_refuses_after_handover_before_stop_or_archive() {
    let rig = Rig::new("crlocktwo");
    pending(&rig);
    let (child, log) = spawn_logged(&rig, &argv(&rig, 1), "20");
    wait_for(&log, "reusing the outstanding handover request");
    let path = rig
        .home
        .join("sessions")
        .join(format!(".lifecycle.{}.lock", rig.name));
    let held = ae::store::lock(&path, Duration::ZERO).expect("hold only second lock");
    reply_and_memo(&rig);
    let before = snapshot(&rig.dir);
    let output = bounded(child, Duration::from_secs(25)).expect("second lock refuses");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let err = std::fs::read_to_string(log).expect("stderr bytes");
    assert_eq!(
        err,
        format!(
            "{}reboot: reusing the outstanding handover request {REFERENCE}.\nreboot: handover complete (reply {REFERENCE} and a new handover memo).\n{}",
            facts(&rig),
            busy(&rig)
        )
    );
    preserved(&rig, &before);
    drop(held);
}

#[test]
fn compactrun_revalidates_identity_again_after_unlocked_handover() {
    let rig = Rig::new("crwaitdrift");
    pending(&rig);
    let (child, log) = spawn_logged(&rig, &argv(&rig, 1), "20");
    wait_for(&log, "reusing the outstanding handover request");
    let other = "99999999-9999-9999-9999-999999999999";
    set_row(&rig, "session_id", other);
    reply_and_memo(&rig);
    let before = snapshot(&rig.dir);
    let output = bounded(child, Duration::from_secs(10)).expect("post-wait refusal returned");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let err = std::fs::read_to_string(log).expect("stderr bytes");
    assert_eq!(
        err,
        format!(
            "{}reboot: reusing the outstanding handover request {REFERENCE}.\nreboot: handover complete (reply {REFERENCE} and a new handover memo).\nreboot: '{}' is not the session that was authorized (after the handover wait) — authorized id {UUID}, on disk now {other}. Nothing was stopped and nothing was archived.\n",
            facts(&rig),
            rig.name
        )
    );
    preserved(&rig, &before);
}

#[test]
fn compactrun_plan_failure_keeps_durable_recovery_after_removing_source() {
    let rig = Rig::new("crplanfail");
    let plan = rig.home.join("missing-parent").join("plan");
    let why = std::fs::write(&plan, b"reference").expect_err("absent parent");
    let mut args = argv(&rig, 5);
    args.extend(["--exec-plan", plan.to_str().expect("plan path")]);
    let (code, out, err) = rig.run(&args);
    assert_eq!(code, Some(1), "{err}");
    assert!(
        out.is_empty(),
        "behavior pin: no stdout even after teardown if plan fails"
    );
    assert!(err.ends_with(&format!("Error: the archive is published and the source session is gone, but the exec plan could not be written to {}: {why}\n  Start the fresh session by hand:\n      {}\n", plan.display(), recovery(&rig))), "{err}");
    assert!(!plan.exists());
    assert!(!err.contains("Starting fresh session"));
    crossed(&rig);
}

#[test]
fn compactrun_yes_prefix_and_uppercase_answer_cross_the_boundary() {
    for answer in ["y\n", "  YES\n"] {
        let rig = Rig::new("cryes");
        let plan = rig.home.join("plan");
        let args = [
            "_compact",
            "--digest-only",
            &rig.name,
            "--exec-plan",
            plan.to_str().expect("plan path"),
        ];
        let (code, out, err) = answered(&rig, &args, answer);
        assert_eq!(code, Some(0), "{err}");
        assert_eq!(out, contract(&rig));
        assert!(err.starts_with(&format!(
            "{}{}reboot: semantic handover skipped (--digest-only); the digest is the handover.\n",
            facts(&rig),
            confirmation(&rig, true)
        )));
        crossed(&rig);
    }
}
