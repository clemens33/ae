//! Frozen acceptance (#248 P1a): `ae list --ended`.
//!
//! Oracle: #248 gap 3 "`ae list --ended` (uuid, name, ended-at, work dir),
//! read-only and never selectable for start/stop"; lead ruling I4 (reboot archives
//! MARKED, never hidden; legacy reason `unknown`); plan r2 L13-L15 and residual
//! r2-2 (an unreadable registration fails the whole listing: exit 1, no rows).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "fixtures publish real archives and plant registrations on disk"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

const A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const B: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
const C: &str = "cccccccc-0000-4000-8000-00000000000c";
const D: &str = "dddddddd-0000-4000-8000-00000000000d";
const E: &str = "eeeeeeee-0000-4000-8000-00000000000e";
const F: &str = "ffffffff-0000-4000-8000-00000000000f";

struct Home(PathBuf);
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(tag: &str) -> Home {
    let root = super::cli::OwnedScratch::root("ended", tag).keep();
    std::fs::create_dir_all(root.join("tmux")).unwrap();
    Home(root)
}

fn run(home: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let mut cmd = super::cli::ae();
    cmd.env("AE_HOME", home)
        .env("CONFIG_FILE", home.join("config"))
        .env("TMUX_TMPDIR", home.join("tmux"))
        .env("AE_NO_AUTOSTART", "1")
        .env_remove("TMUX")
        .env_remove("TMUX_PANE");
    for arg in args {
        cmd.arg(arg);
    }
    let out = super::cli::bounded(
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn"),
        Duration::from_secs(20),
    )
    .expect("returned");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Register `name` with `uuid` (a live-registration meta only).
fn register(home: &Path, name: &str, uuid: &str, work_dir: &str) -> PathBuf {
    let dir = home.join("sessions").join(name);
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    std::fs::write(
        dir.join("meta"),
        format!(
            "session={name}\nsession_id={uuid}\nsession_id_origin=session\nmode=local\n\
             origin={work_dir}\nwork_dir={work_dir}\nlayout=vertical\nschema=2\n\
             seat.main=lead\nprofile.main=fake\nagent_bin.main=sh\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("memo.tsv"), "").unwrap();
    std::fs::write(dir.join("events.jsonl"), "").unwrap();
    dir
}

/// Archive `name`/`uuid` at `at`; `keep` leaves the registration standing.
fn archive(home: &Path, name: &str, uuid: &str, at: &str, work_dir: &str, keep: bool) {
    std::fs::create_dir_all(home.join("archive")).unwrap();
    let dir = register(home, name, uuid, work_dir);
    let d = dir.to_str().unwrap();
    let (code, _, err) = run(
        home,
        &["_archive-publish", d, "not-managed", "-", "-", "-", at],
    );
    assert_eq!(code, Some(0), "publish {name}: {err}");
    if !keep {
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Rewrite one archived meta line (0600 kept), the way an edit or an older core would.
fn edit_meta(home: &Path, uuid: &str, edit: impl Fn(&str) -> Option<String>) {
    let path = home.join("archive").join(uuid).join("meta");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(kept) = edit(line) {
            out.push_str(&kept);
            out.push('\n');
        }
    }
    std::fs::write(&path, out).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// The one stdout row naming `uuid`, asserting its facts appear in column order.
fn row<'a>(stdout: &'a str, uuid: &str, facts: &[&str]) -> &'a str {
    let rows: Vec<&str> = stdout.lines().filter(|l| l.contains(uuid)).collect();
    assert_eq!(rows.len(), 1, "exactly one row for {uuid}: {stdout}");
    let mut from = 0;
    for fact in std::iter::once(&uuid).chain(facts) {
        let at = rows[0][from..]
            .find(fact)
            .unwrap_or_else(|| panic!("{fact} after column {from} in {:?}", rows[0]));
        from += at + fact.len();
    }
    rows[0]
}

/// Every regular file under the archive root with its bytes.
fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(p) = stack.pop() {
        let meta = std::fs::symlink_metadata(&p).unwrap();
        if meta.is_dir() {
            for entry in std::fs::read_dir(&p).unwrap() {
                stack.push(entry.unwrap().path());
            }
        } else {
            out.push((p.clone(), std::fs::read(&p).unwrap_or_default()));
        }
    }
    out.sort();
    out
}

#[test]
fn ended_lists_each_archive_once_newest_first_and_marks_a_reboot() {
    let h = home("order");
    archive(&h.0, "alpha", A, "2026-08-01T00:00:00Z", "/w/a", false);
    archive(&h.0, "beta", B, "2026-08-03T00:00:00Z", "/w/b", false);
    archive(&h.0, "alpha", C, "2026-08-02T00:00:00Z", "/w/c", false);
    edit_meta(&h.0, C, |l| {
        Some(if l == "reason=end" {
            "reason=reboot".to_owned()
        } else {
            l.to_owned()
        })
    });
    let before = tree(&h.0.join("archive"));
    let (code, out, err) = run(&h.0, &["list", "--ended"]);
    assert_eq!(code, Some(0), "{err}");
    row(&out, B, &["beta", "2026-08-03T00:00:00Z", "end", "/w/b"]);
    row(
        &out,
        C,
        &["alpha", "2026-08-02T00:00:00Z", "reboot", "/w/c"],
    );
    row(&out, A, &["alpha", "2026-08-01T00:00:00Z", "end", "/w/a"]);
    let at = |id: &str| out.find(id).unwrap();
    assert!(at(B) < at(C) && at(C) < at(A), "newest first: {out}");
    assert_eq!(tree(&h.0.join("archive")), before, "listing writes nothing");
    assert!(
        !h.0.join("sessions").join("alpha").exists(),
        "nothing re-registered"
    );
}

#[test]
fn ended_hides_a_still_registered_incarnation_but_not_a_reused_name() {
    let h = home("suppress");
    archive(&h.0, "gamma", D, "2026-08-01T00:00:00Z", "/w/d", true);
    archive(&h.0, "delta", E, "2026-08-02T00:00:00Z", "/w/e", false);
    register(&h.0, "delta", F, "/w/f");
    let (code, out, err) = run(&h.0, &["list", "--ended"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        !out.contains(D),
        "a registered incarnation is not ended: {out}"
    );
    row(&out, E, &["delta", "2026-08-02T00:00:00Z"]);
    assert!(
        !out.contains(F),
        "a live session is not an ended row: {out}"
    );
}

#[test]
fn ended_reads_legacy_facts_as_unknown() {
    let h = home("legacy");
    archive(&h.0, "old", A, "2026-08-01T00:00:00Z", "/w/old", false);
    edit_meta(&h.0, A, |l| {
        (!l.starts_with("reason=") && !l.starts_with("source_work_dir=")).then(|| l.to_owned())
    });
    let _ = std::fs::remove_file(h.0.join("archive").join(A).join("harness.tsv"));
    let (code, out, err) = run(&h.0, &["list", "--ended"]);
    assert_eq!(code, Some(0), "{err}");
    let line = row(
        &out,
        A,
        &["old", "2026-08-01T00:00:00Z", "unknown", "unknown"],
    );
    assert!(!line.contains("/w/old"), "no guessed work dir: {line}");
}

#[test]
fn ended_fails_closed_on_an_unreadable_or_ambiguous_registration() {
    // A registration whose incarnation cannot be named might be any archive's: no
    // row is safe to show, so the listing refuses whole (plan L14, residual r2-2).
    let ids = [
        "unreadable",
        "no session_id",
        "two session_ids",
        "malformed",
    ];
    for (n, what) in ids.into_iter().enumerate() {
        let h = home(&format!("amb{n}"));
        archive(&h.0, "eta", A, "2026-08-01T00:00:00Z", "/w/eta", false);
        let meta = register(&h.0, "zeta", B, "/w/zeta").join("meta");
        let other = |l: &str| !l.starts_with("session_id=");
        match what {
            "unreadable" => {
                std::fs::set_permissions(&meta, std::fs::Permissions::from_mode(0o000)).unwrap();
            }
            "no session_id" => rewrite(&meta, other, None),
            "two session_ids" => rewrite(&meta, |_| true, Some(&format!("session_id={C}"))),
            _ => rewrite(&meta, other, Some("session_id=not-a-uuid")),
        }
        if what == "unreadable" && std::fs::read(&meta).is_ok() {
            eprintln!("skipped {what}: running with privileges that read a 000 file");
            continue;
        }
        let (code, out, err) = run(&h.0, &["list", "--ended"]);
        let _ = std::fs::set_permissions(&meta, std::fs::Permissions::from_mode(0o600));
        assert_eq!(code, Some(1), "{what}: {out}{err}");
        assert!(
            err.contains("zeta"),
            "{what}: names the registration: {err}"
        );
        assert!(!out.contains(A), "{what}: no row when unprovable: {out}");
    }
}

/// Keep the lines `keep` accepts, then append `extra`.
fn rewrite(meta: &Path, keep: impl Fn(&str) -> bool, extra: Option<&str>) {
    let text = std::fs::read_to_string(meta).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| keep(l)).chain(extra).collect();
    std::fs::write(meta, lines.join("\n") + "\n").unwrap();
}

#[test]
fn ended_skips_claims_links_and_invalid_trees_and_reports_the_invalid() {
    let h = home("skip");
    archive(&h.0, "valid", A, "2026-08-01T00:00:00Z", "/w/v", false);
    archive(&h.0, "bravo", B, "2026-08-02T00:00:00Z", "/w/b", false);
    let root = h.0.join("archive");
    let claim = root.join(format!(".publishing.{B}"));
    std::fs::create_dir(&claim).unwrap();
    std::fs::create_dir(root.join("notes")).unwrap();
    let elsewhere = h.0.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(root.join(A), elsewhere.join("copy")).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("copy"), root.join(C)).unwrap();
    std::fs::create_dir(root.join(D)).unwrap();
    std::fs::write(root.join(D).join("meta"), format!("archive_id={D}\n")).unwrap();
    let before = tree(&root);
    let (code, out, err) = run(&h.0, &["list", "--ended"]);
    assert_eq!(
        code,
        Some(0),
        "an invalid archive is reported, not fatal: {err}"
    );
    row(&out, A, &["valid", "2026-08-01T00:00:00Z"]);
    for id in [B, C, D] {
        assert!(!out.contains(id), "{id} is not an ended row: {out}");
    }
    assert!(err.contains(D), "the invalid tree is named: {err}");
    assert!(
        claim.is_dir(),
        "a claimed archive is suppressed, never cleared"
    );
    assert_eq!(tree(&root), before, "listing writes nothing");
}

#[test]
fn ended_with_no_archive_root_lists_nothing() {
    let h = home("empty");
    let (code, out, err) = run(&h.0, &["list", "--ended"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!out.contains("-4000-"), "{out}");
}

#[test]
fn ended_refuses_json_and_live_filters() {
    let h = home("flags");
    for args in [
        ["list", "--ended", "--json"],
        ["list", "--json", "--ended"],
        ["list", "--ended", "--all"],
        ["list", "--ended", "--stopped"],
        ["list", "--running", "--ended"],
        ["list", "--ended", "--needs"],
        ["list", "--ended", "--active"],
    ] {
        let (code, out, _) = run(&h.0, &args);
        assert_eq!(code, Some(2), "{args:?} is a usage error");
        assert!(out.is_empty(), "{args:?}: {out}");
    }
}

#[test]
fn ended_names_are_not_lifecycle_targets_and_their_archive_is_untouched() {
    let h = home("target");
    archive(&h.0, "omega", A, "2026-08-01T00:00:00Z", "/w/o", false);
    let before = tree(&h.0.join("archive"));
    for args in [
        vec!["_stop", "omega", "-y"],
        vec!["_end", "-f", "omega"],
        vec!["_end", "-f", A],
    ] {
        let (code, _, err) = run(&h.0, &args);
        assert_eq!(code, Some(1), "{args:?}: {err}");
        assert!(err.contains("not found"), "{args:?}: {err}");
    }
    assert_eq!(
        tree(&h.0.join("archive")),
        before,
        "the archive is untouched"
    );
    assert!(!h.0.join("sessions").join("omega").exists());
}
