// Frozen orchprof batch2, authored against o8look f788bf6c before UI code.
// Oracle: lead's W1 ruling, config-order resolvable profiles, distinct next
// fresh start and recorded seat facts, and the publisher's refusal contract.
// Included inside app::tests to exercise real keys, drawing and Wake routing.
// Only loader/file outcomes cross an injected boundary. No world-read door.

use std::fs;
use std::sync::mpsc;
use std::time::Instant;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

use crate::app::loader::{Request, Wake};
use crate::app::settings::{OrchestratorView, resolve_orchestrator};
use crate::app::tests::Root;
use crate::app::{App, drain, take_keys};
use crate::config::{self, Applied, ApplyRefusal, RosterEditRefusal};
use crate::console::input::{Key, Keys};

const CONFIG: &str = "[clients]\naccount = claude config_home=$HOME/.account\nclient_only = codex\n\n[profiles]\nzz_first = /bin/echo first\nbad_chain = /bin/echo one && /bin/echo two\naa_current = /bin/echo current\nbad_quote = /bin/echo 'unterminated\nmm_client = account --model example\n\n[roster]\norchestrator = aa_current\nhelper = zz_first\n\n[workspace]\nchat = app\n";
const FRESH: &str = "applies at next fresh start (after ae end)";
const RESUME: &str = "Pause/Resume keeps the recorded profile";
const SAVED: &str = "orchestrator profile saved";

fn fixture(tag: &str) -> Root {
    let root = Root::new(tag);
    fs::write(root.0.join("config"), CONFIG).expect("scratch config");
    let dir = root.0.join("sessions/orchestrator");
    fs::create_dir_all(&dir).expect("scratch orchestrator record");
    fs::write(
        dir.join("meta"),
        "schema=2\nsession=orchestrator\nprofile.main=recorded_only\n",
    )
    .expect("recorded profile independent of next start");
    root
}

fn resolved(root: &Root) -> OrchestratorView {
    resolve_orchestrator(
        Some(&root.0.join("sessions/orchestrator")),
        &root.0.join("config"),
        Some(&root.0),
    )
}

fn opened(root: &Root) -> (App, mpsc::Receiver<Request>) {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(None, None, None, Some(tx));
    app.fleeted = true;
    text(&mut app, ",2");
    app.settings_bodies.config = crate::app::settings::resolve_config(None, &root.0.join("config"));
    app.settings_bodies.orchestrator = resolved(root);
    (app, rx)
}

fn frame(app: &mut App) -> String {
    let mut buf = Buffer::empty(Rect::new(0, 0, 220, 60));
    app.frame(&mut buf);
    (0..60)
        .map(|y| (0..220).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(app: &mut App, key: Key) {
    assert!(take_keys(app, vec![(key, Instant::now())]).is_some());
}

fn text(app: &mut App, text: &str) {
    key(app, Key::Text(text.as_bytes().to_vec()));
}

fn land(app: &mut App, generation: u64, outcome: Result<Applied, ApplyRefusal>) {
    let (tx, rx) = mpsc::channel();
    tx.send(Wake::OrchestratorPick {
        generation,
        outcome,
    })
    .expect("loader result boundary");
    assert!(drain(app, &mut Keys::app(), &rx, &mut None).is_some());
}

fn queue_first(app: &mut App) -> crate::app::settings::OrchestratorPick {
    text(app, "ok"); // configured aa_current is second; k selects zz_first.
    key(app, Key::Enter);
    app.take_orchestrator_pick().expect("one queued user pick")
}

fn no_success(shown: &str) {
    assert!(!shown.contains(SAVED), "refusal retained success: {shown}");
    assert!(
        !shown.to_lowercase().contains("success"),
        "false success: {shown}"
    );
}

#[test]
fn u1_configured_global_and_recorded_seat_are_distinct_visible_facts() {
    let root = fixture("orchprof-u1");
    let (mut app, _) = opened(&root);
    let shown = frame(&mut app);
    let configured = shown
        .lines()
        .find(|line| line.contains("orchestrator = aa_current"))
        .expect("configured next-start row");
    assert!(
        configured.contains("global"),
        "source stays on its own row: {shown}"
    );
    assert!(
        shown.contains("recorded orchestrator = recorded_only"),
        "{shown}"
    );
    assert!(shown.contains(FRESH) && shown.contains(RESUME), "{shown}");
    assert!(
        !shown.contains("running orchestrator"),
        "paused records are not live claims"
    );

    fs::remove_file(root.0.join("sessions/orchestrator/meta")).expect("no saved session");
    app.settings_bodies.orchestrator =
        resolve_orchestrator(None, &root.0.join("config"), Some(&root.0));
    assert!(frame(&mut app).contains("recorded orchestrator = (no session)"));
}

#[test]
fn u2_only_resolvable_profile_keys_appear_in_config_order_and_home_is_explicit() {
    let root = fixture("orchprof-u2");
    let OrchestratorView::Ready(ready) = resolved(&root) else {
        panic!("readable fixture")
    };
    assert_eq!(ready.configured, "aa_current");
    assert_eq!(ready.recorded.as_deref(), Some("recorded_only"));
    assert_eq!(ready.choices, ["zz_first", "aa_current", "mm_client"]);
    let OrchestratorView::Ready(no_home) = resolve_orchestrator(None, &root.0.join("config"), None)
    else {
        panic!("a client home failure excludes that choice, not all choices")
    };
    assert_eq!(no_home.choices, ["zz_first", "aa_current"]);

    let (mut app, _) = opened(&root);
    text(&mut app, "o");
    let shown = frame(&mut app);
    let picker = shown
        .split_once("Orchestrator profile")
        .expect("visible picker heading")
        .1;
    let rows: Vec<_> = picker
        .lines()
        .filter_map(|line| {
            ["zz_first", "aa_current", "mm_client"]
                .into_iter()
                .find(|name| line.split_whitespace().any(|word| word == *name))
        })
        .collect();
    assert_eq!(rows, ["zz_first", "aa_current", "mm_client"], "{shown}");
    for excluded in [
        "bad_chain",
        "bad_quote",
        "client_only",
        "account",
        "recorded_only@client",
    ] {
        assert!(!picker.contains(excluded), "excluded {excluded}: {shown}");
    }
}

#[test]
fn u3_picker_preselects_current_keys_move_and_handoff_is_once_without_early_write() {
    let root = fixture("orchprof-u3");
    for (move_key, expected) in [
        (None, "aa_current"),
        (Some(Key::Up), "zz_first"),
        (Some(Key::Down), "mm_client"),
        (Some(Key::Text(b"k".to_vec())), "zz_first"),
        (Some(Key::Text(b"j".to_vec())), "mm_client"),
    ] {
        let (mut app, _) = opened(&root);
        text(&mut app, "o");
        if let Some(move_key) = move_key {
            key(&mut app, move_key);
        }
        assert!(
            app.take_orchestrator_pick().is_none(),
            "navigation never saves"
        );
        key(&mut app, Key::Enter);
        let pick = app
            .take_orchestrator_pick()
            .expect("Enter applies highlighted choice");
        assert_eq!(pick.profile, expected);
        assert_eq!(
            pick.generation,
            app.model.settings().expect("open overlay").generation
        );
        assert!(
            app.take_orchestrator_pick().is_none(),
            "a repaint cannot apply twice"
        );
        assert_eq!(
            fs::read_to_string(root.0.join("config")).expect("still original"),
            CONFIG
        );
    }
    let (mut app, _) = opened(&root);
    text(&mut app, "oj");
    key(&mut app, Key::Escape);
    assert!(
        app.model.settings_open(),
        "first Esc closes only the picker"
    );
    assert!(!frame(&mut app).contains("Orchestrator profile"));
    assert!(
        app.take_orchestrator_pick().is_none(),
        "cancel never applies"
    );
    key(&mut app, Key::Escape);
    assert!(!app.model.settings_open(), "second Esc closes Settings");
}

#[test]
fn u4_real_apply_result_routes_through_wake_and_requests_a_fresh_read() {
    let root = fixture("orchprof-u4");
    let (mut app, rx) = opened(&root);
    let _ = rx.try_iter().count();
    let pick = queue_first(&mut app);
    assert_eq!(pick.profile, "zz_first");
    let outcome = config::apply_roster_row(
        &root.0.join("config"),
        &pick.seen,
        "orchestrator",
        &pick.profile,
        17,
    )
    .expect("real scratch publish");
    assert!(outcome.changed);
    land(&mut app, pick.generation, Ok(outcome));
    let shown = frame(&mut app);
    assert!(
        shown.contains(SAVED),
        "real success reaches the drawn frame: {shown}"
    );
    assert!(shown.contains(FRESH) && shown.contains(RESUME), "{shown}");
    assert!(
        rx.try_iter()
            .any(|request| matches!(request, Request::Settings { open: true, .. })),
        "save requests current bytes and choices again"
    );
    app.settings_bodies.orchestrator = resolved(&root);
    let shown = frame(&mut app);
    assert!(shown.contains("orchestrator = zz_first"), "{shown}");
    assert!(
        shown.contains("recorded orchestrator = recorded_only"),
        "save never changes recorded seat"
    );
}

#[test]
fn u5_each_refusal_replaces_earlier_success_and_keeps_the_complete_reason_visible() {
    let root = fixture("orchprof-u5");
    let path = root.0.join("config");
    let backup = root.0.join("config.17.bak");
    // The first four variants are file outcomes proved through real file
    // operations in the terminal specs/batch1. Publish is the external owner
    // or group failure proved by the publisher's injected-metadata test.
    for refusal in [
        ApplyRefusal::ChangedSince(path.clone()),
        ApplyRefusal::Symlink(path.clone()),
        ApplyRefusal::Edit(RosterEditRefusal::TooLarge),
        ApplyRefusal::BackupExists(backup.clone()),
        ApplyRefusal::Publish {
            why: "owner or group differs; edit the file by hand".into(),
            backup: Some(backup.clone()),
        },
    ] {
        fs::write(&path, CONFIG).expect("reset own scratch config");
        let (mut app, _) = opened(&root);
        let pick = queue_first(&mut app);
        // Drive a genuine successful write first, so stale success cannot
        // pass a refusal test that started with no success to remove.
        let outcome =
            config::apply_roster_row(&path, &pick.seen, "orchestrator", &pick.profile, 17)
                .expect("first genuine save");
        land(&mut app, pick.generation, Ok(outcome));
        assert!(frame(&mut app).contains(SAVED));
        app.settings_bodies.orchestrator = resolved(&root);
        text(&mut app, "oj");
        key(&mut app, Key::Enter);
        let second = app.take_orchestrator_pick().expect("second save requested");
        let expected = refusal.to_string();
        land(&mut app, second.generation, Err(refusal));
        let shown = frame(&mut app);
        assert!(
            shown.contains(&expected),
            "full human reason {expected:?}: {shown}"
        );
        no_success(&shown);
        assert!(shown.contains("recorded orchestrator = recorded_only"));
        fs::remove_file(&backup).expect("remove own prior backup between independent cases");
    }
}

#[test]
fn u6_late_apply_answers_cannot_reopen_or_paint_another_settings_generation() {
    let root = fixture("orchprof-u6");
    let (mut app, _) = opened(&root);
    let pick = queue_first(&mut app);
    key(&mut app, Key::Escape);
    assert!(!app.model.settings_open());
    land(
        &mut app,
        pick.generation,
        Err(ApplyRefusal::ChangedSince(root.0.join("config"))),
    );
    assert!(!app.model.settings_open());
    text(&mut app, ",2");
    app.settings_bodies.orchestrator = resolved(&root);
    let before = frame(&mut app);
    land(
        &mut app,
        pick.generation,
        Ok(Applied {
            changed: true,
            backup: Some(root.0.join("old.bak")),
        }),
    );
    assert_eq!(
        frame(&mut app),
        before,
        "old success cannot paint a new open"
    );
}

#[test]
fn u7_uneditable_or_unreadable_global_explains_itself_before_picking_and_never_queues_a_write() {
    for (tag, config_text, reason_terms) in [
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
            String::new(),
            vec!["not a regular file", "cannot be read"],
        ),
    ] {
        let root = fixture(&format!("orchprof-u7-{tag}"));
        let path = root.0.join("config");
        if tag == "unreadable" {
            fs::remove_file(&path).expect("replace scratch file");
            fs::create_dir(&path).expect("nonregular unreadable global");
        } else {
            fs::write(&path, &config_text).expect("uneditable scratch config");
        }
        let read = resolved(&root);
        let OrchestratorView::Unreadable(reason) = &read else {
            panic!("{tag} must disable editing and name its reason before a pick: {read:?}");
        };
        assert!(
            reason_terms.iter().any(|term| reason.contains(term)),
            "{tag}: {reason}"
        );
        let (mut app, _) = opened(&root);
        let before = frame(&mut app);
        assert!(
            before.contains("orchestrator") && before.contains(reason),
            "{tag}: {before}"
        );
        no_success(&before);
        text(&mut app, "o");
        key(&mut app, Key::Enter);
        assert!(
            app.take_orchestrator_pick().is_none(),
            "{tag}: unavailable row cannot apply"
        );
        let after = frame(&mut app);
        assert!(
            after.contains(reason),
            "{tag}: reason remains visible: {after}"
        );
        assert!(
            !after.contains("Orchestrator profile"),
            "{tag}: no unusable picker"
        );
        no_success(&after);
        if tag == "unreadable" {
            assert!(path.is_dir());
            assert_eq!(
                fs::read_dir(&path)
                    .expect("unreadable directory untouched")
                    .count(),
                0
            );
        } else {
            assert_eq!(
                fs::read_to_string(&path).expect("uneditable bytes unchanged"),
                config_text
            );
        }
        assert!(
            !fs::read_dir(&root.0).expect("root names").any(|entry| {
                let name = entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                name.starts_with("config.") || name.starts_with(".config")
            }),
            "{tag}: no backup or temp"
        );
    }
}
