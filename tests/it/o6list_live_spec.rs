//! Real tmux/ps consumer oracle; prescribed S1a records, not a second store-probe test.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private real-process fixture"
)]
use super::cli::{OwnedScratch, ae};
use ae::json::{Value, parse};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::Path,
    time::{Duration, Instant},
};
const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";

fn cells(doc: &Value) -> Vec<Value> {
    let Value::Arr(sessions) = doc.get("sessions").expect("sessions") else {
        panic!("array")
    };
    assert_eq!(
        sessions.len(),
        2,
        "failure cases retain both durable sessions"
    );
    sessions
        .iter()
        .map(|s| {
            let Value::Arr(agents) = s.get("agents").expect("agents") else {
                panic!("array")
            };
            assert_eq!(agents.len(), 1);
            assert_eq!(agents[0].get_str("session_id"), Some("-"));
            agents[0]
                .get("conversation")
                .expect("additive member")
                .clone()
        })
        .collect()
}

fn tmux(root: &Path, words: &[&str]) -> String {
    let mut args = vec!["-S".into(), root.join("sock").display().to_string()];
    args.extend(words.iter().map(|s| (*s).into()));
    let (ok, out) = super::phase2::run_tmux(&args, root);
    assert!(ok, "private tmux {words:?}: {out}");
    out
}

#[test]
#[allow(clippy::too_many_lines)]
fn list_uses_child_tool_pid_one_snapshot_and_unknown_for_failed_external_proof() {
    let mut scratch = OwnedScratch::root("o6b", "live");
    let root = scratch.path().to_owned();
    scratch.add_tmux_server(root.join("sock"));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("private bin");
    // Preserve macOS platform signing; ps sees the symlink's tool name on both hosts.
    std::os::unix::fs::symlink("/bin/sleep", bin.join("codex")).expect("native fixture");
    for name in ["fixture", "other"] {
        let command = format!("set -m; '{}' 600; :", bin.join("codex").display());
        tmux(
            &root,
            &[
                "new-session",
                "-d",
                "-s",
                name,
                "-e",
                &format!("AE_SESSION={name}"),
                "/bin/sh",
                "-c",
                &command,
            ],
        );
        let pane: u32 = tmux(
            &root,
            &[
                "display-message",
                "-p",
                "-t",
                &format!("={name}:"),
                "#{pane_pid}",
            ],
        )
        .trim()
        .parse()
        .expect("pane pid");
        let until = Instant::now() + Duration::from_secs(5);
        let tool = loop {
            if let Some(p) = ae::procs::snapshot().and_then(|rows| {
                rows.into_iter()
                    .find(|p| p.ppid == pane && (p.comm == "codex" || p.comm.ends_with("/codex")))
            }) {
                break p.pid;
            }
            assert!(Instant::now() < until, "native tool under shell {pane}");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_ne!(pane, tool);
        tmux(
            &root,
            &[
                "set-option",
                "-p",
                "-t",
                &format!("={name}:"),
                "@ae_slot",
                "main",
            ],
        );
        let dir = root.join("sessions").join(name);
        fs::create_dir_all(&dir).expect("record");
        fs::write(dir.join("meta"), format!("schema=2\nsession={name}\nsession_id=22222222-2222-4222-8222-222222222222\nmode=local\nwork_dir={}\nseat.main=lead\nprofile.main=fake\nagent_bin.main=codex\nharness_session.main=pending\nlaunch_id.main={LAUNCH}\ntmux_server_kind=socket\ntmux_server={}\n", root.display(), root.join("sock").display())).expect("meta");
        fs::write(dir.join("events.jsonl"), format!("{{\"ts\":\"2026-10-29T00:00:00Z\",\"actor\":\"watchdog\",\"action\":\"conversation-new\",\"ref\":\"main:{LAUNCH}:{tool} - pending\"}}\n")).expect("prescribed S1a decision");
    }
    for (name, body) in [
        (
            "ps",
            "#!/bin/sh\nprintf 'ps\\n' >> \"$O6_LOG\"\n[ \"$O6_FAIL\" = ps ] && exit 1\nexec /bin/ps \"$@\"\n",
        ),
        (
            "tmux",
            "#!/bin/sh\ncase \" $* \" in *' list-panes '*) [ \"$O6_FAIL\" = panes ] && exit 1;; *' list-sessions '*) [ \"$O6_FAIL\" = sessions ] && exit 1;; esac\nPATH=\"$O6_PATH\" exec tmux \"$@\"\n",
        ),
    ] {
        fs::write(bin.join(name), body).expect("passthrough observer");
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).expect("executable");
    }
    let log = root.join("ps.log");
    let run = |json: bool, fail: &str| {
        fs::write(&log, "").expect("reset count");
        let path = std::env::var("PATH").expect("PATH");
        let mut cmd = ae();
        cmd.env("HOME", &root)
            .env("AE_HOME", &root)
            .env("CONFIG_FILE", root.join("config"))
            .env("TMUX_TMPDIR", &root)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", root.join("sock"))
            .env("AE_NO_AUTOSTART", "1")
            .env("O6_LOG", &log)
            .env("O6_PATH", &path)
            .env("O6_FAIL", fail)
            .env("PATH", format!("{}:{path}", bin.display()))
            .current_dir(&root)
            .args(["list", "--all"]);
        if json {
            cmd.arg("--json");
        }
        let out = cmd.output().expect("list");
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF-8")
    };
    let text = run(false, "");
    assert_eq!(text.matches("conv:new").count(), 2, "{text}");
    assert_eq!(fs::read_to_string(&log).expect("ps log").lines().count(), 1);
    let doc = parse(&run(true, "")).expect("JSON");
    assert_eq!(fs::read_to_string(&log).expect("ps log").lines().count(), 1);
    for c in cells(&doc) {
        assert_eq!(c.get_str("current"), Some("yes"));
        assert_eq!(c.get_str("start"), Some("new"));
    }
    for fail in ["ps", "panes", "sessions"] {
        for c in cells(&parse(&run(true, fail)).expect("JSON")) {
            assert_eq!(c.get_str("current"), Some("unknown"), "{fail}: {c}");
        }
    }
    let duplicate = tmux(
        &root,
        &[
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "=fixture:",
            "/bin/sleep",
            "600",
        ],
    );
    tmux(
        &root,
        &[
            "set-option",
            "-p",
            "-t",
            duplicate.trim(),
            "@ae_slot",
            "main",
        ],
    );
    let doc = parse(&run(true, "")).expect("JSON");
    let Value::Arr(sessions) = doc.get("sessions").expect("sessions") else {
        panic!("array")
    };
    for (session, c) in sessions.iter().zip(cells(&doc)) {
        let want = match session.get_str("name") {
            Some("fixture") => "unknown",
            Some("other") => "yes",
            _ => panic!("unexpected session"),
        };
        assert_eq!(c.get_str("current"), Some(want));
    }
}
