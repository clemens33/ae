//! #169: `ae list` marks a seat whose pane runs a known harness its records do
//! not name. DIAGNOSIS only: a process-table gap, a shell, a matching tool or
//! a seat that records no tool never reads as a mismatch, and the listing still
//! costs ONE `ps`. Oracle: the issue text, `ae doctor`'s `seat-identity`
//! disagreement and the existing one-snapshot listing contract. Real tmux/ps.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "private real-process fixture"
)]
use super::cli::{OwnedScratch, ae};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::Path,
    time::{Duration, Instant},
};

const LAUNCH: &str = "11111111-1111-4111-8111-111111111111";

fn tmux(root: &Path, words: &[&str]) -> String {
    let mut args = vec!["-S".into(), root.join("sock").display().to_string()];
    args.extend(words.iter().map(|s| (*s).into()));
    let (ok, out) = super::phase2::run_tmux(&args, root);
    assert!(ok, "private tmux {words:?}: {out}");
    out
}

/// One live session whose single pane runs the harness `runs` and whose meta
/// records `records` as `agent_bin.main` (`None`: the row is absent).
fn live_session(root: &Path, name: &str, records: Option<&str>, runs: &str) {
    let command = format!(
        "set -m; '{}' -e 'sleep 600'; :",
        root.join("bin").join(runs).display()
    );
    tmux(
        root,
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
        root,
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
    // The tool is a CHILD of the pane's shell: wait until `ps` shows it.
    let until = Instant::now() + Duration::from_secs(5);
    while !ae::procs::snapshot().is_some_and(|rows| {
        rows.iter()
            .any(|p| p.ppid == pane && (p.comm == runs || p.comm.ends_with(&format!("/{runs}"))))
    }) {
        assert!(Instant::now() < until, "{runs} under shell {pane}");
        std::thread::sleep(Duration::from_millis(20));
    }
    tmux(
        root,
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
    let binary = records.map_or_else(String::new, |tool| format!("agent_bin.main={tool}\n"));
    fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nsession={name}\nsession_id=22222222-2222-4222-8222-222222222222\nmode=local\nwork_dir={}\nseat.main=lead\nprofile.main=fake\n{binary}harness_session.main=pending\nlaunch_id.main={LAUNCH}\ntmux_server_kind=socket\ntmux_server={}\n",
            root.display(),
            root.join("sock").display()
        ),
    )
    .expect("meta");
}

/// Every seat line of a table listing, with the session heading it.
fn seat_lines(text: &str) -> Vec<(String, String)> {
    let mut session = String::new();
    let mut seats = Vec::new();
    for line in text.lines() {
        if !line.starts_with(' ') {
            line.split_whitespace()
                .next()
                .unwrap_or_default()
                .clone_into(&mut session);
        } else if line.contains(" · observed:") {
            seats.push((session.clone(), line.to_owned()));
        }
    }
    seats
}

#[test]
#[allow(clippy::too_many_lines)]
fn list_marks_a_seat_whose_pane_runs_another_harness_and_never_guesses() {
    let mut scratch = OwnedScratch::root("i169", "live");
    let root = scratch.path().to_owned();
    scratch.add_tmux_server(root.join("sock"));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("private bin");
    // A symlink keeps macOS platform signing, and ps sees its name as the tool's
    // on both hosts. The target is perl, not sleep: a multi-call coreutils
    // (uutils on Ubuntu 26.04) refuses to run under a name that is not its own.
    for tool in ["codex", "claude"] {
        std::os::unix::fs::symlink("/usr/bin/perl", bin.join(tool)).expect("native fixture");
    }
    // THE INCIDENT: records say claude, a codex runs there.
    live_session(&root, "clash", Some("claude"), "codex");
    // Agreement, and a seat that records no tool at all.
    live_session(&root, "same", Some("codex"), "codex");
    live_session(&root, "blank", None, "codex");
    for (name, body) in [
        (
            "ps",
            "#!/bin/sh\nprintf 'ps\\n' >> \"$I169_LOG\"\n[ \"$I169_FAIL\" = ps ] && exit 1\nexec /bin/ps \"$@\"\n",
        ),
        (
            "tmux",
            "#!/bin/sh\ncase \" $* \" in *' list-panes '*) [ \"$I169_FAIL\" = panes ] && exit 1;; esac\nPATH=\"$I169_PATH\" exec tmux \"$@\"\n",
        ),
    ] {
        fs::write(bin.join(name), body).expect("passthrough observer");
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).expect("executable");
    }
    let log = root.join("ps.log");
    let run = |fail: &str| {
        fs::write(&log, "").expect("reset count");
        let path = std::env::var("PATH").expect("PATH");
        let out = ae()
            .env("HOME", &root)
            .env("AE_HOME", &root)
            .env("CONFIG_FILE", root.join("config"))
            .env("TMUX_TMPDIR", &root)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", root.join("sock"))
            .env("AE_NO_AUTOSTART", "1")
            .env("I169_LOG", &log)
            .env("I169_PATH", &path)
            .env("I169_FAIL", fail)
            .env("PATH", format!("{}:{path}", bin.display()))
            .current_dir(&root)
            .args(["list", "--all"])
            .output()
            .expect("list");
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF-8")
    };
    let seat_of = |text: &str, session: &str| {
        let seats = seat_lines(text);
        let [(_, line)] = seats
            .iter()
            .filter(|(name, _)| name == session)
            .collect::<Vec<_>>()[..]
        else {
            panic!("one seat line of {session}: {text}");
        };
        line.clone()
    };
    let marked = |text: &str, session: &str| seat_of(text, session).contains(" · harness:");

    // The disagreement is a CELL on the seat's own line; agreement and a seat
    // with no recorded tool say nothing; the listing still reads ps once.
    let text = run("");
    assert_eq!(seat_lines(&text).len(), 3, "{text}");
    assert!(
        seat_of(&text, "clash").contains(" · harness:codex (records claude)"),
        "{text}"
    );
    assert!(!marked(&text, "same"), "{text}");
    assert!(!marked(&text, "blank"), "{text}");
    assert_eq!(fs::read_to_string(&log).expect("ps log").lines().count(), 1);

    // A gap in either reading is UNKNOWN, never a mismatch.
    for fail in ["ps", "panes"] {
        let text = run(fail);
        assert!(
            !text.contains("harness:"),
            "{fail} failing must not read as a mismatch: {text}"
        );
    }

    // Two panes claiming the slot cannot be attributed to the seat.
    let duplicate = tmux(
        &root,
        &[
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "=clash:",
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
    let text = run("");
    assert!(
        !marked(&text, "clash"),
        "an ambiguous pane is unknown, not a mismatch: {text}"
    );
}

fn proc(id: u32, parent: u32, comm: &str) -> ae::procs::Proc {
    ae::procs::Proc {
        pid: id,
        ppid: parent,
        comm: comm.to_owned(),
    }
}

/// `clash_of` over an independent table: the pane's root is pid 100.
fn clash_of(
    recorded: Option<&str>,
    pid: Option<u32>,
    foreground: Option<&str>,
    table: Option<&[ae::procs::Proc]>,
) -> Option<ae::digest::HarnessClash> {
    ae::session::harness_clash_of(recorded, pid, foreground, &|| table)
}

#[test]
fn only_a_covered_reading_that_disagrees_with_the_records_is_a_clash() {
    use ae::tool::ToolKind::{Claude, Codex};
    let codex = [proc(100, 1, "sh"), proc(120, 100, "/opt/bin/codex")];
    let clash = |runs: Vec<_>, records: &str| ae::digest::HarnessClash {
        runs,
        records: records.to_owned(),
    };
    // THE INCIDENT: the records say claude, a codex runs under the pane.
    assert_eq!(
        clash_of(Some("claude"), Some(100), Some("codex"), Some(&codex)),
        Some(clash(vec![Codex], "claude"))
    );
    // The recorded tool alongside another is never clean ...
    let both = [codex[0].clone(), codex[1].clone(), proc(130, 100, "claude")];
    assert_eq!(
        clash_of(Some("codex"), Some(100), Some("sh"), Some(&both)),
        Some(clash(vec![Claude, Codex], "codex"))
    );
    // ... but exactly the recorded tool is, and so is its `.exe` spelling.
    for recorded in ["codex", "codex.exe"] {
        assert_eq!(
            clash_of(Some(recorded), Some(100), Some("codex"), Some(&codex)),
            None
        );
    }
    // Everything that proves nothing says nothing: no recorded tool, no pane
    // pid, no process table, a table without the pane's root, a harness ae
    // cannot place, a shell, and a foreground ae does not know.
    let unplaced = [codex[0].clone(), codex[1].clone(), proc(900, 850, "claude")];
    let shell = [proc(100, 1, "fish")];
    let editor = [proc(100, 1, "vim")];
    for (recorded, pid, foreground, table) in [
        (None, Some(100), Some("codex"), Some(&codex[..])),
        (Some(""), Some(100), Some("codex"), Some(&codex[..])),
        (Some("claude"), None, Some("codex"), Some(&codex[..])),
        (Some("claude"), Some(100), Some("codex"), None),
        (Some("claude"), Some(100), Some("codex"), Some(&[][..])),
        (
            Some("claude"),
            Some(100),
            Some("codex"),
            Some(&unplaced[..]),
        ),
        (Some("claude"), Some(100), Some("fish"), Some(&shell[..])),
        (Some("claude"), Some(100), Some("vim"), Some(&editor[..])),
    ] {
        assert_eq!(clash_of(recorded, pid, foreground, table), None);
    }
}

#[test]
fn the_process_table_is_not_read_for_a_seat_that_can_claim_nothing() {
    let asked = std::cell::Cell::new(0);
    let get = || {
        asked.set(asked.get() + 1);
        None
    };
    for (recorded, pid) in [
        (None, Some(100)),
        (Some(""), Some(100)),
        (Some("codex"), None),
    ] {
        assert_eq!(
            ae::session::harness_clash_of(recorded, pid, Some("codex"), &get),
            None
        );
    }
    assert_eq!(asked.get(), 0, "no recorded tool or pid: no ps");
}

#[test]
fn the_cell_names_what_runs_and_what_the_records_say_and_draws_nothing_else() {
    use ae::tool::ToolKind::{Claude, Codex};
    let rendered = |clash| {
        let mut session = ae::digest::SessionEntry::new("fixture", ae::digest::Status::Running);
        session.agents.push(ae::digest::AgentEntry {
            reference: "lead".into(),
            name: "lead".into(),
            harness_clash: clash,
            ..ae::digest::AgentEntry::default()
        });
        ae::listing::render(
            &ae::filters::ListArgs::parse::<&str>(&[]).expect("default list"),
            &ae::listing::World::new(
                ae::time::Timestamp::from_epoch(1_792_000_000),
                vec![session],
            ),
        )
    };
    let text = rendered(Some(ae::digest::HarnessClash {
        runs: vec![Claude, Codex],
        records: "codex".into(),
    }));
    assert!(
        text.contains(" · harness:claude+codex (records codex)"),
        "{text}"
    );
    // A recorded word is hostile persisted state: no escape reaches the table.
    let text = rendered(Some(ae::digest::HarnessClash {
        runs: vec![Codex],
        records: "claude\u{1b}[31m".into(),
    }));
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(!rendered(None).contains("harness:"));
}
