//! Frozen apprestart spec: brief + rev3 + lead 15:09Z/15:15Z rulings.
//! Same-binary bridge pins carrier consumption; TWO-real-build proof is a
//! separate required freeze receipt. No future-only API: assertion RED on base.

use super::{InstalledRig, UUID, WAIT};
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

struct Hand {
    rig: InstalledRig,
    pane: String,
}

impl Hand {
    fn new(tag: &str, outside: bool, operand: bool) -> Self {
        let rig = InstalledRig::new(tag, true);
        let named = rig.scratch.join(format!(
            "tmux-{}",
            fs::metadata(&rig.scratch).unwrap().uid()
        ));
        fs::create_dir_all(&named).unwrap();
        fs::set_permissions(&named, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&rig.socket, named.join("ae")).unwrap();
        let q = |p: &std::path::Path| quote(&p.display().to_string());
        let outside = if outside { "-u TMUX -u TMUX_PANE" } else { "" };
        let operand = if operand {
            quote(&rig.session)
        } else {
            String::new()
        };
        let command = format!(
            "exec 3<&0; printf '%s\\n' \"$$\" \"$PWD\" > {}; env -u AE_NO_AUTOSTART -u AE_SESSION -u CODEX_HOME -u CLAUDE_CONFIG_DIR {outside} HOME={} AE_HOME={} CONFIG_FILE={} TMUX_TMPDIR={} AE_RESTART_PROBE='literal $probe' {} app {operand} <&3 3<&- & pid=$!; printf '%s\\n' \"$pid\" > {}; wait \"$pid\"; rc=$?; stty -a > {}; printf 'BACK rc=%s\\n' \"$rc\"; sleep 30",
            q(&rig.scratch.join("shell")),
            q(&rig.home),
            q(&rig.ae_home),
            q(&rig.ae_home.join("config")),
            q(&rig.scratch),
            q(&rig.link),
            q(&rig.scratch.join("pid")),
            q(&rig.scratch.join("after-mode")),
        );
        let pane = rig
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                &rig.session,
                "-c",
                &rig.scratch.display().to_string(),
                "-P",
                "-F",
                "#{pane_id}",
                &command,
            ])
            .trim()
            .to_owned();
        rig.tmux(&["set-window-option", "-t", &pane, "remain-on-exit", "on"]);
        rig.wait(&pane, WAIT, "GUARD manual app paints", |s| {
            s.contains("▣ 1 ")
        });
        Self { rig, pane }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.rig.scratch.join(name)
    }

    fn pid(&self) -> String {
        fs::read_to_string(self.path("pid"))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn wait(&self, why: &str, met: impl Fn(&str) -> bool) -> String {
        self.rig.wait(&self.pane, WAIT, why, met)
    }

    fn keys(&self, keys: &str) {
        self.rig.tmux(&["send-keys", "-t", &self.pane, keys]);
    }

    fn option(&self, name: &str) -> String {
        self.rig
            .tmux(&[
                "display-message",
                "-p",
                "-t",
                &self.pane,
                &format!("#{{{name}}}"),
            ])
            .trim()
            .to_owned()
    }

    fn next(&self, version: &str, body: &str, mode: u32) {
        self.rig.repoint_link(version);
        let core = self
            .rig
            .ae_home
            .join("versions")
            .join(version)
            .join("ae-core");
        fs::write(&core, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(core, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn receipt(&self, name: &str) -> String {
        let until = Instant::now() + WAIT;
        loop {
            if let Ok(text) = fs::read_to_string(self.path(name))
                && !text.is_empty()
            {
                return text;
            }
            assert!(
                Instant::now() < until,
                "restart must write {name}; {}",
                self.rig.screen(&self.pane)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn recorder(&self, version: &str) {
        let q = |name| quote(&self.path(name).display().to_string());
        self.next(version, &format!("stty -a > {}; printf '%s\\n' \"$$\" \"$PPID\" \"$PWD\" \"$AE_RESTART_PROBE\" \"$@\" > {}", q("entry-mode"), q("entry")), 0o755);
    }

    fn cooked(&self, name: &str) {
        let text = self.receipt(name);
        for word in ["icanon", "echo"] {
            assert!(
                text.split_whitespace()
                    .any(|w| w.trim_end_matches(';') == word),
                "terminal restored before exec/after quit: {text}"
            );
        }
    }

    fn bridge(&self, rewrite: bool) {
        let q = |p: &std::path::Path| quote(&p.display().to_string());
        let core = self
            .rig
            .ae_home
            .join("versions")
            .join(ae::VERSION)
            .join("ae-core");
        let carry = if rewrite {
            format!(
                "carry=$(printf '%s' \"$carry\" | awk -F '|' -v v={} 'BEGIN{{OFS=\"|\"}} {{$3=v;print}}'); rm {}; ln -s {} {}",
                quote(ae::VERSION),
                q(&self.rig.link),
                q(&core),
                q(&self.rig.link)
            )
        } else {
            ":".to_owned()
        };
        self.next("2026.10.99", &format!("printf 'hop\\n' >> {}; for carry do :; done; printf '%s\\n' \"$carry\" > {}; {carry}; exec {} app {} --app-view \"$carry\"", q(&self.path("hops")), q(&self.path("wire")), q(&core), quote(&self.rig.session)), 0o755);
    }
}

#[test]
fn exec_preserves_pid_parent_shell_operands_environment_cwd_and_quit_mode() {
    for (tag, outside, operand) in [
        ("rxin", false, true),
        ("rxout", true, true),
        ("rxbare", false, false),
    ] {
        let hand = Hand::new(tag, outside, operand);
        let pid = hand.pid();
        let shell = hand.receipt("shell");
        let shell: Vec<_> = shell.lines().collect();
        hand.recorder("2026.10.99");
        let entry = hand.receipt("entry");
        let entry: Vec<_> = entry.lines().collect();
        assert_eq!(
            &entry[..4],
            &[pid.as_str(), shell[0], shell[1], "literal $probe"]
        );
        let offset = if operand {
            assert_eq!(&entry[4..6], &["app", hand.rig.session.as_str()]);
            6
        } else {
            assert_eq!(entry[4], "app");
            5
        };
        assert_eq!(entry.len(), offset + 2, "no synthesized human operand");
        assert_eq!(entry[offset], "--app-view");
        let view =
            ae::app::handover::parse(entry[offset + 1]).expect("existing hostile-input grammar");
        assert_eq!(view.pane, format!("%{pid}"));
        assert_eq!(view.version, "2026.10.99");
        assert_eq!(view.view.session, hand.rig.session);
        hand.cooked("entry-mode");
        hand.wait("parent shell survives exec", |s| s.contains("BACK rc=0"));
        assert_eq!(hand.option("alternate_on"), "0");
    }
}

#[test]
fn rollback_exec_preserves_shell_and_omits_the_carrier() {
    let hand = Hand::new("rxold", true, true);
    hand.recorder("2026.10.40");
    let entry = hand.receipt("entry");
    assert_eq!(
        entry.lines().skip(4).collect::<Vec<_>>(),
        vec!["app", hand.rig.session.as_str()]
    );
    hand.wait("rollback shell survives", |s| s.contains("BACK rc=0"));
}

#[test]
fn failed_exec_keeps_interaction_and_terminal_and_does_not_retry_same_version() {
    let hand = Hand::new("rxfail", false, true);
    hand.rig.tmux(&[
        "pipe-pane",
        "-t",
        &hand.pane,
        &format!("cat > {}", quote(&hand.path("trace").display().to_string())),
    ]);
    hand.next("2026.10.99", "exit 0", 0o644);
    hand.wait("failed exec resumes app with reason", |s| {
        s.contains("could not restart into ae 2026.10.99 -")
    });
    std::thread::sleep(Duration::from_secs(12));
    let trace = hand.receipt("trace");
    assert_eq!(
        trace.matches("\x1b[?1049l").count(),
        1,
        "no repeated failed attempts"
    );
    assert!(
        !hand.path("after-mode").exists(),
        "app stays alive in parent shell"
    );
    assert_eq!(hand.option("alternate_on"), "1");
    hand.keys(",");
    hand.wait("settings works after exec failure", |s| {
        s.contains("Config") && s.contains("Instructions")
    });
    hand.keys("Escape");
    hand.wait("settings closed", |s| !s.contains("Instructions"));
    hand.keys("C-c");
    hand.wait("quit still returns to shell", |s| s.contains("BACK rc=0"));
    hand.cooked("after-mode");
    assert_eq!(hand.option("alternate_on"), "0");
}

#[test]
fn outside_tmux_carrier_restores_scrolled_view_and_never_consumes_chat_option() {
    let hand = Hand::new("rxview", true, true);
    let mut events = String::new();
    for n in 0..80 {
        writeln!(events, "{{\"ts\":\"2026-10-03T06:00:00Z\",\"actor\":\"lead\",\"action\":\"chat\",\"summary\":\"turn-{n:03}\"}}").unwrap();
    }
    fs::write(
        hand.rig
            .ae_home
            .join("sessions")
            .join(&hand.rig.session)
            .join("events.jsonl"),
        events,
    )
    .unwrap();
    hand.wait("GUARD transcript loaded", |s| s.contains("turn-079"));
    hand.keys("PageUp");
    let before = hand.wait("GUARD scrolled transcript", |s| {
        s.contains("turn-") && !s.contains("turn-079")
    });
    hand.rig.tmux(&[
        "set-option",
        "-t",
        &hand.rig.session,
        "@ae_app_view",
        "another-chats-option",
    ]);
    hand.bridge(true);
    let wire = hand.receipt("wire");
    let wire = ae::app::handover::parse(wire.trim()).expect("published carrier");
    assert!(
        wire.view.pages > 0 || wire.view.body_top > 0,
        "actual scrolled view carried"
    );
    let expected = before.lines().find(|s| s.contains("turn-")).unwrap().trim();
    hand.wait("carrier restores actual scroll", |s| {
        s.contains(expected) && !s.contains("turn-079")
    });
    assert_eq!(
        hand.rig
            .tmux(&["show-option", "-v", "-t", &hand.rig.session, "@ae_app_view"])
            .trim(),
        "another-chats-option"
    );
    let shell = hand.receipt("shell");
    assert_eq!(
        shell.lines().next().unwrap(),
        hand.option("pane_pid"),
        "parent shell owns unchanged pane"
    );
}

#[test]
fn a_carrier_that_landed_in_another_core_suppresses_a_restart_loop() {
    let hand = Hand::new("rxguard", true, true);
    hand.bridge(false);
    hand.receipt("hops");
    hand.wait("wrong-core guard hints and stays usable", |s| {
        s.contains("ae 2026.10.99 installed - restart ae app to use it")
    });
    std::thread::sleep(Duration::from_secs(12));
    assert_eq!(hand.receipt("hops"), "hop\n", "one exec only");
    hand.keys("C-c");
    hand.wait("guarded app remains interactive", |s| {
        s.contains("BACK rc=0")
    });
}

#[test]
fn a_stamped_chat_with_refused_restart_proof_never_falls_back_to_exec() {
    let hand = Hand::new("rxchat", false, true);
    hand.rig
        .tmux(&["set-option", "-p", "-t", &hand.pane, "@ae_console", UUID]);
    hand.rig.tmux(&[
        "set-option",
        "-t",
        &hand.rig.session,
        "@ae_session_uuid",
        "different",
    ]);
    hand.recorder("2026.10.99");
    hand.wait("refused chat stays hint-only", |s| {
        s.contains("ae 2026.10.99 installed - restart ae app to use it")
    });
    std::thread::sleep(Duration::from_secs(12));
    assert!(
        !hand.path("entry").exists(),
        "chat proof refusal must not exec"
    );
    assert_eq!(hand.option("alternate_on"), "1");
}

#[test]
fn internal_carrier_tail_is_accepted_but_incomplete_or_reordered_pairs_are_usage() {
    for tail in [
        vec!["app", "--app-view", "hostile"],
        vec!["app", "web", "--app-view", "hostile"],
    ] {
        let out = super::super::cli::ae().args(tail).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "carrier grammar accepts hostile text for parser judgment"
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "ae app draws a terminal UI; use ae chat to print the lane\n"
        );
    }
    for tail in [
        vec!["app", "--app-view"],
        vec!["app", "--app-view", "hostile", "web"],
        vec!["app", "web", "extra", "--app-view", "hostile"],
    ] {
        assert_eq!(
            super::super::cli::ae()
                .args(tail)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
}
