//! Historical build characterization at a7a91c93; no desired-behavior changes.
use super::*;
use std::collections::BTreeMap;

const TRACE_HOOK: &str = r#"
if (grep { $_ eq 'new-session' } @ARGV) {
    my $fault = $ENV{AE_BUILDX_FAULT} // '';
    my $meta = $ENV{AE_BUILDX_META} // '';
    if ($fault eq 'read') {
        rename($meta, "$meta.saved") or die "save meta: $!";
        mkdir($meta) or die "block meta read: $!";
    } elsif ($fault =~ /^(invalid|added|different)$/) {
        open(my $in, '<', $meta) or die "read fault meta: $!";
        local $/; my $body = <$in>; close($in);
        $body =~ s/^client\.main=.*\n//mg;
        $body .= 'client.main=' . ($fault eq 'invalid' ? '' : 'claude') . "\n";
        open(my $out, '>', $meta) or die "write fault meta: $!";
        print $out $body; close($out);
    }
}
if (($ENV{AE_BUILDX_FAULT} // '') eq 'worker' && grep { $_ eq 'split-window' } @ARGV) {
    exit 1;
}
if ($ENV{AE_BUILDX_HOLD} && grep { $_ eq 'load-buffer' } @ARGV) {
    exit 1 if grep { $_ eq 'ae-launch-main' } @ARGV;
}
if ($ENV{AE_BUILDX_HOLD} && grep { $_ eq 'capture-pane' } @ARGV) {
    my $held = $ENV{AE_BUILDX_HOLD};
    open(my $mark, '>', "$held.seen") or die "hold marker: $!";
    close($mark);
    my $end = time + 30;
    while (-e $held && time < $end) { select(undef, undef, undef, 0.02); }
}
"#;

fn traced(rig: &Rig, command: &mut Runner) -> PathBuf {
    let log = launchx_trace_command(command, &rig.scratch);
    let wrapper = rig.scratch.join("trace-bin/tmux");
    let original =
        std::fs::read_to_string(&wrapper).unwrap_or_else(|why| panic!("read trace wrapper: {why}"));
    let marker = "exec {$ENV{AE_LAUNCHX_REAL_TMUX}}";
    assert_eq!(original.matches(marker).count(), 1, "one real-tmux exec");
    std::fs::write(
        &wrapper,
        original.replace(marker, &format!("{TRACE_HOOK}\n{marker}")),
    )
    .unwrap_or_else(|why| panic!("install trace hooks: {why}"));
    command.env("AE_LAUNCHX_VERSION", "3.5");
    log
}

fn normalized(text: &str, scratch: &Path) -> String {
    let core = Path::new(env!("CARGO_BIN_EXE_ae"));
    let core_resolved =
        std::fs::canonicalize(core).unwrap_or_else(|why| panic!("canonical core: {why}"));
    let scratch_resolved =
        std::fs::canonicalize(scratch).unwrap_or_else(|why| panic!("canonical scratch: {why}"));
    let mut paths = [
        (core.display().to_string(), "<core>"),
        (core_resolved.display().to_string(), "<core>"),
        (scratch.display().to_string(), "<scratch>"),
        (scratch_resolved.display().to_string(), "<scratch>"),
    ];
    paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
    let mut text = text.to_owned();
    for (path, replacement) in paths {
        text = text.replace(&path, replacement);
    }
    text = text.replace(&format!("ae {}", ae::VERSION), "ae <version>");
    for (prefix, replacement) in [
        ("#{==:#{pid},", "<pid>"),
        ("#{==:#{start_time},", "<epoch>"),
        ("#{==:#{session_created},", "<epoch>"),
    ] {
        text = normalized_comparison(&text, prefix, replacement);
    }
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < chars.len() {
        let tail: String = chars[index..].iter().take(36).collect();
        if tail.len() == 36
            && tail.bytes().enumerate().all(|(index, byte)| {
                if [8, 13, 18, 23].contains(&index) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        {
            output.push_str("<uuid>");
            index += 36;
        } else if chars[index..].starts_with(&['a', 'e', '-', 's', 'e', 'n', 'd', '-']) {
            output.push_str("<buffer>");
            index += 8;
            while chars
                .get(index)
                .is_some_and(|ch| ch.is_ascii_digit() || *ch == '-' || *ch == '_')
            {
                index += 1;
            }
        } else if ['%', '$', '@'].contains(&chars[index])
            && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            let start = index;
            index += 1;
            while chars.get(index).is_some_and(char::is_ascii_digit) {
                index += 1;
            }
            let id: String = chars[start..index].iter().collect();
            let next = ids.keys().filter(|id| id.starts_with(chars[start])).count() + 1;
            let replacement = ids
                .entry(id)
                .or_insert_with(|| format!("{}<{next}>", chars[start]));
            output.push_str(replacement);
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }
    output
}

fn normalized_comparison(text: &str, prefix: &str, replacement: &str) -> String {
    let mut output = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(prefix) {
        let end = index + prefix.len();
        output.push_str(&rest[..end]);
        rest = &rest[end..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            output.push_str(replacement);
            rest = &rest[digits..];
        }
    }
    output.push_str(rest);
    output
}

fn trace_text(log: &Path, rig: &Rig) -> String {
    let text =
        std::fs::read_to_string(log).unwrap_or_else(|why| panic!("complete tmux trace: {why}"));
    normalized(&text, &rig.scratch)
}

fn command(rig: &Rig, session: &str) -> Runner {
    rig.launch_command_with_server(
        "socket",
        &rig.sock.display().to_string(),
        &["--local", session],
    )
}

fn keeper(rig: &Rig) {
    let path = std::env::var_os("PATH").unwrap_or_else(|| panic!("test PATH"));
    let invocation = super::super::parity::Invocation::new("tmux")
        .env_cleared()
        .env("HOME", &rig.scratch)
        .env("PATH", path)
        .env("TMUX_TMPDIR", &rig.scratch)
        .arg("-u")
        .arg("-S")
        .arg(&rig.sock)
        .arg("-f")
        .arg("/dev/null")
        .arg("new-session")
        .arg("-d")
        .arg("-s")
        .arg("keeper");
    let status = super::super::parity::capture::raw::run(
        &invocation,
        &rig.scratch,
        &rig.scratch.join("keeper-out"),
        &rig.scratch.join("keeper-err"),
    )
    .unwrap_or_else(|why| panic!("isolated keeper: {why}"));
    assert!(
        matches!(
            status.outcome(),
            super::super::parity::capture::ExitOutcome::Code(0)
        ),
        "isolated keeper must start"
    );
}

fn golden(label: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/buildx")
        .join(format!("{label}.txt"));
    let expected =
        std::fs::read_to_string(path).unwrap_or_else(|why| panic!("frozen {label}: {why}"));
    assert_eq!(actual, expected, "frozen build trace {label}");
}

fn idle_layout(rig: &Rig, layout: &str, many: bool) {
    let workers = if many {
        "workers = colead, builder, checker\n"
    } else {
        ""
    };
    let config = format!(
        "[profiles]\nidle = \"sleep 600\"\n\n[roster]\nlead = idle\ncolead = idle\nbuilder = idle\nchecker = idle\n\n[workspace]\nmain = lead\n{workers}layout = {layout}\nwatchdog = false\nchat = off\n"
    );
    std::fs::write(&rig.config, config).unwrap_or_else(|why| panic!("layout config: {why}"));
}

#[test]
fn buildx_normalizer_keeps_distinct_ids_and_scrubs_both_path_spellings() {
    let scratch = OwnedScratch::root("ln", "buildx-normalization");
    let actual = scratch.join("actual");
    std::fs::create_dir(&actual).expect("real scratch");
    let alias = scratch.join("alias");
    std::os::unix::fs::symlink(&actual, &alias).expect("scratch alias");
    let resolved = std::fs::canonicalize(&alias).expect("canonical alias");
    let core = Path::new(env!("CARGO_BIN_EXE_ae"));
    let resolved_core = std::fs::canonicalize(core).expect("canonical core");
    let input = format!(
        "{} {} {} {} %10 %11 %10 $5 @8 42 ae-send-321-_10 0e3c0a4e-7f7e-4c8e-9a3b-2d1f00000001 ae {} #{{==:#{{pid}},123}} #{{==:#{{start_time}},1793671330}} #{{==:#{{session_created}},1793671331}}",
        alias.display(),
        resolved.display(),
        core.display(),
        resolved_core.display(),
        ae::VERSION
    );
    assert_eq!(
        normalized(&input, &alias),
        "<scratch> <scratch> <core> <core> %<1> %<2> %<1> $<1> @<1> 42 <buffer> <uuid> ae <version> #{==:#{pid},<pid>} #{==:#{start_time},<epoch>} #{==:#{session_created},<epoch>}"
    );
}

#[test]
fn buildx_full_34_create_and_resume_keep_legacy_input_map() {
    if skip() {
        return;
    }
    let rig = Rig::idle("buildx-34");
    rig.without_chat();
    keeper(&rig);
    for resume in [false, true] {
        if resume {
            stop(&rig, "bxTrace");
        }
        let mut launch = command(&rig, "bxTrace");
        let log = traced(&rig, &mut launch);
        launch.env("AE_LAUNCHX_VERSION", "3.4");
        let result = launch.output().expect("3.4 create/resume");
        assert!(result.status.success(), "{result:?}");
        golden(
            if resume { "3.4-resume" } else { "3.4-create" },
            &trace_text(&log, &rig),
        );
    }
}

#[test]
fn buildx_full_create_and_resume_trace_covers_worker_placements() {
    if skip() {
        return;
    }
    for (layout, many) in [
        ("vertical", false),
        ("vertical", true),
        ("horizontal", true),
        ("lead-pair", true),
        ("lead-solo", true),
    ] {
        let rig = Rig::idle("buildx-full");
        idle_layout(&rig, layout, many);
        keeper(&rig);
        for resume in [false, true] {
            if resume {
                stop(&rig, "bxTrace");
            }
            let mut launch = command(&rig, "bxTrace");
            let log = traced(&rig, &mut launch);
            let result = launch.output().expect("full traced launch");
            assert!(result.status.success(), "{result:?}");
            let trace = trace_text(&log, &rig);
            assert!(trace.contains("\tnew-session\t"));
            assert!(trace.contains("\t@ae_slot\tmain\n"));
            assert!(trace.contains("\t_events\n"));
            assert_eq!(
                trace.lines().last().unwrap_or_default().split('\t').nth(3),
                Some("select-pane")
            );
            let label = format!(
                "{layout}-{}-{}",
                if many { "many" } else { "single" },
                if resume { "resume" } else { "create" }
            );
            golden(&label, &trace);
            let meta = rig.meta("bxTrace");
            assert_eq!(
                meta.lines().any(|line| line.starts_with("launch_settled=")),
                resume,
                "{meta}"
            );
            assert_eq!(
                rig.panes("bxTrace")
                    .iter()
                    .filter(|(_, slot, _)| !slot.is_empty())
                    .count(),
                if many { 4 } else { 1 }
            );
        }
    }
}

#[test]
fn buildx_spawned_resume_preserves_missing_profile_and_restores_available_worker() {
    if skip() {
        return;
    }
    let rig = Rig::idle("buildx-spawned");
    rig.without_chat();
    keeper(&rig);
    let (code, stdout, stderr) = rig.launch(&["--local", "bxSpawned"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    stop(&rig, "bxSpawned");
    let original = rig.meta("bxSpawned");
    let additions = "seat.spawned.2=available\nprofile.spawned.2=idle\nagent_bin.spawned.2=sleep\nharness_session.spawned.2=pending\nseat.spawned.7=kept\nprofile.spawned.7=unconfigured\nagent_bin.spawned.7=sleep\nharness_session.spawned.7=kept-conversation\n";
    std::fs::write(
        rig.dir("bxSpawned").join("meta"),
        format!("{original}{additions}"),
    )
    .expect("saved spawned seats");
    let mut launch = command(&rig, "bxSpawned");
    let log = traced(&rig, &mut launch);
    let result = launch.output().expect("spawned resume");
    assert!(result.status.success(), "{result:?}");
    golden("spawned-resume", &trace_text(&log, &rig));
    let meta = rig.meta("bxSpawned");
    for row in additions.lines() {
        assert!(meta.lines().any(|line| line == row), "lost {row}: {meta}");
    }
    let live = rig.panes("bxSpawned");
    assert!(
        live.iter()
            .any(|(_, slot, name)| slot == "spawned.2" && name == "available")
    );
    assert!(!live.iter().any(|(_, slot, _)| slot == "spawned.7"));
}

#[test]
fn buildx_chat_create_stamps_hook_and_selects_chat_after_main() {
    if skip() {
        return;
    }
    let rig = Rig::idle("buildx-chat");
    keeper(&rig);
    let mut launch = command(&rig, "bxChat");
    let log = traced(&rig, &mut launch);
    let result = launch.output().expect("chat create");
    assert!(result.status.success(), "{result:?}");
    let trace = trace_text(&log, &rig);
    golden("chat-create", &trace);
    let tail: Vec<_> = trace.lines().rev().take(2).collect();
    assert!(tail[0].contains("\tselect-window\t"), "{trace}");
    assert!(tail[1].contains("\tselect-pane\t"), "{trace}");
    assert!(
        trace.contains("\tset-hook\t") && trace.contains("@ae_console"),
        "{trace}"
    );
    let (ok, selected) = rig.tmux(&[
        "list-windows",
        "-t",
        "=bxChat",
        "-F",
        "#{window_name}|#{window_active}",
    ]);
    assert!(
        ok && selected.lines().any(|line| line == "chat|1"),
        "ok={ok}, windows={selected}"
    );
}

fn wait_file(path: &Path, child: &mut OwnedChild) {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if path.is_file() {
            return;
        }
        assert!(
            matches!(child.try_wait(), Ok(None)),
            "launcher exited before capture hold"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("capture hold was never reached: {}", path.display());
}

#[test]
fn buildx_resume_settles_only_after_held_delivery_and_preserves_monitor_progress() {
    if skip() {
        return;
    }
    let rig = Rig::new("bxSettled", &["codex"], Some("main"));
    rig.without_chat();
    keeper(&rig);
    let (code, stdout, stderr) = rig.launch(&["--local", "bxSettled"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(!rig.meta("bxSettled").contains("launch_settled="));
    stop(&rig, "bxSettled");
    let hold = rig.scratch.join("delivery-hold");
    std::fs::write(&hold, b"hold").expect("hold delivery");
    let mut launch = command(&rig, "bxSettled");
    let _log = traced(&rig, &mut launch);
    launch.env("AE_BUILDX_HOLD", &hold);
    let mut child = launch
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("held resume");
    wait_file(&hold.with_extension("seen"), &mut child);
    let held_meta = rig.meta("bxSettled");
    let scratch_home = std::fs::canonicalize(&rig.scratch).expect("canonical fixture HOME");
    let expected_home = format!(
        "config_home.main=implicit:{}/.codex",
        scratch_home.display()
    );
    let expected_base = format!("config_home_base.main={}", scratch_home.display());
    assert!(
        held_meta.lines().any(|line| line == expected_home),
        "tool store escaped fixture HOME: {held_meta}"
    );
    assert!(
        held_meta.lines().any(|line| line == expected_base),
        "tool HOME escaped fixture: {held_meta}"
    );
    assert!(
        !held_meta.contains("launch_settled="),
        "settled before delivery ended: {held_meta}"
    );
    let panes = rig.panes("bxSettled");
    assert!(
        panes.iter().any(|(_, _, name)| name == "_events"),
        "monitors precede delivery"
    );
    let main = panes
        .iter()
        .find(|(_, slot, _)| slot == "main")
        .expect("live main");
    let (ok, stamp) = rig.tmux(&["show-options", "-wv", "-t", &main.0, "@ae_theme"]);
    assert!(
        ok && !stamp.trim().is_empty(),
        "window look precedes delivery"
    );
    // The ready fake keeps running. TRACE_HOOK refuses only the phase-2
    // ae-launch-main load, ending that delivery durably without a 45s wait.
    std::fs::remove_file(&hold).expect("release capture");
    let result = child.wait_with_output().expect("finished resume");
    assert!(result.status.success(), "{result:?}");
    let final_meta = rig.meta("bxSettled");
    let settled = final_meta
        .lines()
        .find_map(|line| line.strip_prefix("launch_settled="))
        .expect("settled row after delivery");
    assert!(
        settled.parse::<i64>().is_ok_and(|epoch| epoch > 0),
        "{final_meta}"
    );
    let events = std::fs::read_to_string(rig.dir("bxSettled").join("events.jsonl"))
        .expect("durable delivery failure");
    assert!(events.contains("launch-delivery-failed"), "{events}");
}

#[test]
fn buildx_after_preflight_client_and_meta_failures_rollback_but_keep_saved_state() {
    if skip() {
        return;
    }
    for fault in ["invalid", "added", "different", "read"] {
        let rig = if fault == "different" {
            client_rig("buildx-refusal")
        } else {
            Rig::idle("buildx-refusal")
        };
        rig.without_chat();
        keeper(&rig);
        let args = if fault == "different" {
            vec!["--local", "bxRefusal", "--lead", "fablex@cc-mic"]
        } else {
            vec!["--local", "bxRefusal"]
        };
        let (code, stdout, stderr) = rig.launch(&args);
        assert_eq!(code, Some(0), "{stdout}{stderr}");
        stop(&rig, "bxRefusal");
        let dir = rig.dir("bxRefusal");
        std::fs::write(dir.join("kept-payload"), b"preserved state")
            .expect("saved unrelated state");
        let mut launch = command(&rig, "bxRefusal");
        let log = traced(&rig, &mut launch);
        launch
            .env("AE_BUILDX_FAULT", fault)
            .env("AE_BUILDX_META", dir.join("meta"));
        let result = launch.output().expect("faulted resume");
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        let trace = trace_text(&log, &rig);
        assert!(
            trace.contains("\tnew-session\t")
                && trace.contains("\t@ae_slot\tmain\n")
                && trace.contains("\tkill-session\t"),
            "fault missed build: {trace}"
        );
        assert!(!rig.sessions().contains(&"bxRefusal".to_owned()));
        assert_eq!(
            std::fs::read(dir.join("kept-payload")).expect("saved state kept"),
            b"preserved state"
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        let expected = match fault {
            "invalid" => "Error: session 'bxRefusal' seat 'lead' records an unusable client (client.main is empty, duplicated or malformed) — fix the meta row, or end the session (ae end bxRefusal).".to_owned(),
            "added" => "Error: session 'bxRefusal' seat 'lead' recorded client override 'claude' while this launch was starting — nothing was resumed; retry the launch.".to_owned(),
            "different" => "Error: session 'bxRefusal' seat 'lead' recorded client override 'claude' but this launch selects 'cc-mic' — a recorded client is write-once. Resume with 'fablex@claude', restore the 'claude' client, or end the session (ae end bxRefusal).".to_owned(),
            "read" => "cannot read the meta to carry recorded clients: Is a directory (os error 21)".to_owned(),
            _ => unreachable!("closed fault cases"),
        };
        assert_eq!(
            stderr,
            format!(
                "ae: launch failed — rolling back 'bxRefusal'.\n{expected}\nae: session state KEPT — it predates this attempt; retry with 'ae bxRefusal'.\n"
            )
        );
    }
}

#[test]
fn buildx_worker_failure_removes_only_the_new_session_state() {
    if skip() {
        return;
    }
    let rig = Rig::idle("buildx-worker-failure");
    idle_layout(&rig, "lead-pair", true);
    keeper(&rig);
    let mut launch = command(&rig, "bxWorkerFail");
    let log = traced(&rig, &mut launch);
    launch.env("AE_BUILDX_FAULT", "worker");
    let result = launch.output().expect("worker failure");
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "ae: launch failed — rolling back 'bxWorkerFail'.\nError: could not create a pane for worker 0\nae: session state removed — this launch created it; nothing left to reattach to.\n"
    );
    let trace = trace_text(&log, &rig);
    assert!(
        trace.contains("\tnew-session\t")
            && trace.contains("\tsplit-window\t")
            && trace.contains("\tkill-session\t")
    );
    assert!(!rig.dir("bxWorkerFail").exists());
    assert!(rig.sessions().contains(&"keeper".to_owned()));
}

#[test]
fn buildx_attempt_stamp_failure_keeps_existing_state_and_creates_no_session() {
    if skip() {
        return;
    }
    let rig = Rig::idle("buildx-attempt-failure");
    rig.without_chat();
    keeper(&rig);
    let dir = rig.dir("bxAttemptFail");
    std::fs::create_dir_all(dir.join(".launch-attempt")).expect("block stamp publication");
    std::fs::write(dir.join("kept-payload"), b"old bytes").expect("existing state");
    let mut launch = command(&rig, "bxAttemptFail");
    let log = traced(&rig, &mut launch);
    let result = launch.output().expect("attempt stamp failure");
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(
        stderr,
        "Error: could not record the launch attempt for 'bxAttemptFail' (Is a directory (os error 21)). No tmux session was created.\nae: session state KEPT — it predates this attempt; retry with 'ae bxAttemptFail'.\n"
    );
    assert!(!trace_text(&log, &rig).contains("\tnew-session\t"));
    assert_eq!(
        std::fs::read(dir.join("kept-payload")).expect("existing payload kept"),
        b"old bytes"
    );
    assert!(dir.join(".launch-attempt").is_dir());
    assert!(!rig.sessions().contains(&"bxAttemptFail".to_owned()));
}

#[test]
fn buildx_telegram_autostart_obeys_suppression_after_launch() {
    if skip() {
        return;
    }
    for suppressed in [false, true] {
        let rig = Rig::idle("buildx-telegram");
        rig.without_chat();
        keeper(&rig);
        let config = std::fs::read_to_string(&rig.config).expect("fixture config");
        std::fs::write(
            &rig.config,
            format!("{config}\n[telegram]\nenabled = true\n"),
        )
        .expect("bridge requested without credentials");
        // Missing token_file refuses before any daemon or network operation.
        // Its warning and durable refusal expose whether launch called autostart.
        let mut launch = if suppressed {
            rig.launch_command_no_autostart(&["--local", "bxTelegram"])
        } else {
            command(&rig, "bxTelegram")
        };
        let result = launch.output().expect("launch with requested bridge");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(rig.sessions().contains(&"bxTelegram".to_owned()));
        assert!(!rig.sessions().contains(&"ae-telegram".to_owned()));
        let refusal = rig.home.join("telegram/autostart-refusal");
        let events = std::fs::read_to_string(rig.dir("bxTelegram").join("events.jsonl"))
            .expect("launch journal");
        assert_eq!(
            stderr.contains("ae telegram: skipped autostart"),
            !suppressed,
            "suppressed={suppressed}: {stderr}"
        );
        assert_eq!(refusal.exists(), !suppressed, "suppressed={suppressed}");
        assert_eq!(
            events.contains("\"action\":\"telegram_autostart_refused\""),
            !suppressed,
            "suppressed={suppressed}: {events}"
        );
        if !suppressed {
            assert!(stderr.contains("token_file is not set"), "{stderr}");
        }
    }
}
