//! Frozen batch2 terminal acceptance against o8look f788bf6c.
//! Drives the real app, loader, keys, file publisher and drawn terminal.
//! Scratch config and a private -S socket are inherited from the Settings
//! rig. No live global config, harness invocation or test-only write door.

use std::fs;
use std::os::unix::fs::symlink;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{FRAME, Rig, WAIT};

const CONFIG: &str = "[profiles]\nzz_first = /bin/echo first\naa_current = /bin/echo current\nmm_last = /bin/echo last\nbad_chain = /bin/echo one && /bin/echo two\n\n[roster]\norchestrator = aa_current\n\n[workspace]\nchat = app\nquota = off\n";
const FRESH: &str = "applies at next fresh start (after ae end)";
const SAVED: &str = "orchestrator profile saved";

fn fixture(tag: &str, text: &str) -> Rig {
    let rig = Rig::new(tag);
    fs::write(rig.root.join("config"), text).expect("scratch global config");
    let orchestrator = rig.root.join("sessions/orchestrator");
    fs::create_dir_all(&orchestrator).expect("saved orchestrator fixture");
    // A saved paused orchestrator has the same profile facts as a live
    // one. The app must not derive them from its unrelated home session.
    fs::write(
        orchestrator.join("meta"),
        "schema=2\nsession=orchestrator\nprofile.main=recorded_only\n",
    )
    .expect("recorded orchestrator fixture");
    rig.meta("profile.main=home_only\n");
    rig
}

fn config_tab(rig: &Rig) -> String {
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "2", "Config");
    rig.wait(&pane, WAIT, "next fresh start profile drawn", |s| {
        s.contains("orchestrator = aa_current") && s.contains(FRESH)
    });
    pane
}

fn choose_first(rig: &Rig, pane: &str) {
    rig.keys(pane, "o");
    rig.wait(pane, FRAME, "picker actually opened", |s| {
        s.contains("Orchestrator profile")
    });
    rig.keys(pane, "k");
    rig.keys(pane, "Enter");
}

fn refused(rig: &Rig, pane: &str, terms: &[&str]) -> String {
    let shown = rig.wait(pane, WAIT, "human sees refusal after apply", |s| {
        terms.iter().all(|term| s.contains(term))
    });
    assert!(!shown.contains(SAVED), "refusal claimed save: {shown}");
    assert!(
        !shown.to_lowercase().contains("success"),
        "refusal claimed success: {shown}"
    );
    shown
}

fn names(rig: &Rig) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(&rig.root)
        .expect("fixture root")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("config.") || name.starts_with(".config"))
        .collect();
    names.sort();
    names
}

#[test]
fn b1_real_loader_shows_current_global_separately_from_recorded_and_saves_only_next_start() {
    let rig = fixture("orchprof-b1", CONFIG);
    let recorded_global = rig.root.join("recorded-global");
    fs::write(
        &recorded_global,
        CONFIG.replace("orchestrator = aa_current", "orchestrator = mm_last"),
    )
    .expect("different meta-recorded config");
    rig.meta(&format!(
        "config={}\nprofile.main=home_only\n",
        recorded_global.display()
    ));
    let pane = config_tab(&rig);
    let shown = rig.screen(&pane);
    let row = shown
        .lines()
        .find(|line| line.contains("orchestrator = aa_current"))
        .expect("current global wins for editable row");
    assert!(row.contains("global"), "{shown}");
    assert!(
        shown.contains("recorded orchestrator = recorded_only"),
        "{shown}"
    );
    assert!(
        shown.contains("Pause/Resume keeps the recorded profile"),
        "{shown}"
    );
    let config = rig.root.join("config");
    let original = fs::read(&config).expect("before choosing");
    let record_path = rig.root.join("sessions/orchestrator/meta");
    let record = fs::read(&record_path).expect("saved seat before choosing");
    rig.keys(&pane, "o");
    let picker = rig.wait(&pane, FRAME, "real picker drawn", |s| {
        s.contains("Orchestrator profile")
    });
    let list = picker
        .split_once("Orchestrator profile")
        .expect("heading")
        .1;
    let order: Vec<_> = list
        .lines()
        .filter_map(|line| {
            ["zz_first", "aa_current", "mm_last"]
                .into_iter()
                .find(|name| line.split_whitespace().any(|word| word == *name))
        })
        .collect();
    assert_eq!(order, ["zz_first", "aa_current", "mm_last"], "{picker}");
    assert!(!list.contains("bad_chain"));
    rig.keys(&pane, "j");
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "cancel returns to config tab", |s| {
        s.contains("orchestrator = aa_current") && !s.contains("Orchestrator profile")
    });
    assert_eq!(fs::read(&config).expect("cancel bytes"), original);
    assert!(names(&rig).is_empty(), "cancel made no backup or temp");

    choose_first(&rig, &pane);
    let saved = rig.wait(
        &pane,
        WAIT,
        "real run-loop save completes and refreshes",
        |s| s.contains(SAVED) && s.contains("orchestrator = zz_first"),
    );
    assert!(saved.contains(FRESH) && saved.contains("recorded orchestrator = recorded_only"));
    assert_eq!(
        fs::read_to_string(&config).expect("saved config"),
        CONFIG.replace("orchestrator = aa_current", "orchestrator = zz_first")
    );
    assert_eq!(
        fs::read(&record_path).expect("recorded seat unchanged"),
        record
    );
    assert_eq!(
        fs::read_to_string(&recorded_global).expect("recorded config unchanged"),
        CONFIG.replace("orchestrator = aa_current", "orchestrator = mm_last")
    );
    let backup_names = names(&rig);
    assert_eq!(
        backup_names.len(),
        1,
        "one backup, no leaked temp: {backup_names:?}"
    );
    assert!(
        std::path::Path::new(&backup_names[0])
            .extension()
            .is_some_and(|ext| ext == "bak")
    );
    assert_eq!(
        fs::read(rig.root.join(&backup_names[0])).expect("exact prior backup"),
        original
    );
}

#[test]
fn b2_changed_since_display_reaches_the_terminal_and_preserves_the_external_edit() {
    let rig = fixture("orchprof-b2", CONFIG);
    let pane = config_tab(&rig);
    let changed = format!("{CONFIG}# another editor changed only this comment\n");
    fs::write(rig.root.join("config"), &changed).expect("external edit after display");
    choose_first(&rig, &pane);
    refused(
        &rig,
        &pane,
        &["changed since it was shown", "nothing was written"],
    );
    assert_eq!(
        fs::read_to_string(rig.root.join("config")).expect("external bytes"),
        changed
    );
    assert!(names(&rig).is_empty());
}

#[test]
fn b3_symlink_after_display_reaches_the_terminal_and_never_writes_through_it() {
    let rig = fixture("orchprof-b3", CONFIG);
    let pane = config_tab(&rig);
    let config = rig.root.join("config");
    let target = rig.root.join("link-target");
    fs::rename(&config, &target).expect("external replacement");
    symlink(&target, &config).expect("scratch symlink after display");
    choose_first(&rig, &pane);
    refused(&rig, &pane, &["symlink", "edit the file by hand"]);
    assert!(
        fs::symlink_metadata(&config)
            .expect("still symlink")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).expect("target untouched"),
        CONFIG
    );
    assert!(names(&rig).is_empty());
}

#[test]
fn b4_growing_past_the_cap_reaches_the_terminal_without_backup_or_replace() {
    // zz_first is shorter than aa_current, so for this arm select mm_last
    // after padding and rename that profile key to a longer valid key.
    let base = CONFIG.replace("mm_last", "much_longer_profile");
    let cap = 1_048_576;
    let mut text = base;
    text.push('#');
    text.push_str(&"x".repeat(cap - text.len()));
    assert_eq!(text.len(), cap);
    let rig = fixture("orchprof-b4", &text);
    let pane = config_tab(&rig);
    rig.keys(&pane, "o");
    rig.wait(&pane, FRAME, "cap picker drawn", |s| {
        s.contains("Orchestrator profile")
    });
    rig.keys(&pane, "j");
    rig.keys(&pane, "Enter");
    refused(
        &rig,
        &pane,
        &["larger than", "1 MiB", "edit the file by hand"],
    );
    assert_eq!(
        fs::read_to_string(rig.root.join("config")).expect("cap bytes unchanged"),
        text
    );
    assert!(names(&rig).is_empty());
}

#[test]
fn b5_existing_backup_is_a_retry_refusal_without_any_success_claim() {
    let rig = fixture("orchprof-b5", CONFIG);
    let pane = config_tab(&rig);
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("epoch")
            .as_secs(),
    )
    .expect("epoch fits");
    // No clock injection: reserve a bounded five-minute wall-clock window.
    // The entire user interaction above/below is bounded by the rig waits.
    let backups: Vec<_> = (now - 5..=now + 300)
        .map(|stamp| {
            let path = rig.root.join(format!("config.{stamp}.bak"));
            fs::write(&path, "existing backup\n").expect("occupied backup stamp");
            path
        })
        .collect();
    let before = names(&rig);
    choose_first(&rig, &pane);
    refused(&rig, &pane, &["backup", "already exists", "retry"]);
    assert_eq!(
        fs::read_to_string(rig.root.join("config")).expect("unchanged config"),
        CONFIG
    );
    assert_eq!(names(&rig), before, "collision left no new temp or backup");
    for path in backups {
        assert_eq!(
            fs::read_to_string(path).expect("prior backup"),
            "existing backup\n"
        );
    }
}

#[test]
fn b6_real_loader_explains_uneditable_and_unreadable_global_before_any_pick() {
    for (tag, config_text, reasons) in [
        (
            "missing",
            CONFIG.replace("orchestrator = aa_current\n", ""),
            vec!["no [roster] orchestrator row"],
        ),
        (
            "duplicate",
            CONFIG.replace(
                "orchestrator = aa_current\n",
                "orchestrator = aa_current\norchestrator = zz_first\n",
            ),
            vec!["duplicate", "more than one"],
        ),
        (
            "shape",
            CONFIG.replace(
                "orchestrator = aa_current\n",
                "orchestrator = \"aa_current\" # a quoted trailing comment\n",
            ),
            vec!["cannot be placed", "trailing comment"],
        ),
        (
            "unreadable",
            CONFIG.to_owned(),
            vec!["not a regular file", "cannot be read"],
        ),
    ] {
        let rig = fixture(&format!("orchprof-b6-{tag}"), &config_text);
        let config = rig.root.join("config");
        if tag == "unreadable" {
            fs::remove_file(&config).expect("replace own config");
            fs::create_dir(&config).expect("unreadable nonregular global");
        }
        let (pane, _) = rig.direct(220, 60, &config);
        rig.ready(&pane);
        rig.settings(&pane);
        rig.tab(&pane, "2", "Config");
        let shown = rig.wait(&pane, WAIT, "upfront reason on Config tab", |s| {
            s.contains("orchestrator") && reasons.iter().any(|reason| s.contains(reason))
        });
        assert!(!shown.contains(SAVED));
        rig.keys(&pane, "o");
        rig.keys(&pane, "Enter");
        // A round trip through another tab lets the input queue drain;
        // merely sampling the old pre-input frame would prove nothing.
        rig.tab(&pane, "1", "Quota");
        rig.tab(&pane, "2", "Config");
        let after = rig.wait(&pane, FRAME, "refusal stays after attempted pick", |s| {
            reasons.iter().any(|reason| s.contains(reason))
        });
        assert!(
            !after.contains(SAVED) && !after.contains("Orchestrator profile"),
            "{tag}: {after}"
        );
        if tag == "unreadable" {
            assert!(config.is_dir());
            assert_eq!(
                fs::read_dir(&config).expect("directory untouched").count(),
                0
            );
        } else {
            assert_eq!(
                fs::read_to_string(&config).expect("uneditable bytes unchanged"),
                config_text
            );
        }
        assert!(names(&rig).is_empty(), "{tag}: no backup or staged copy");
    }
}
