//! Recovery behaviour pins at b9cd091b. Exact diagnostics are base-output pins;
//! non-overwrite and command correlation are recover's documented contract.
#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "characterization fixtures construct and inspect isolated state"
)]

use super::*;

struct Root(PathBuf);
impl Root {
    fn new(tag: &str) -> Self {
        // This spec is compiled inside the library. Reuse rename's unit-test
        // scratch owner; the integration-only OwnedScratch is not in scope.
        let path = super::tests::scratch(&format!("rr-{tag}"));
        Self(path)
    }
    fn side(&self, name: &str, id: &str) {
        let dir = self.0.join("sessions").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        // Invalid server rows prevent an ambient tmux probe in these pure
        // refusal fixtures. The real private-server pins live separately.
        std::fs::write(
            dir.join("meta"),
            format!("session={name}\nsession_id={id}\ntmux_server_kind=invalid\ntmux_server=x\n"),
        )
        .unwrap();
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn intent() -> Intent {
    parse_intent(b"rename_intent=1\nsession_id=e795c9e9-1234-4890-abcd-ef0123456789\nold=sold\nnew=snew\nmode=local\nold_work=/w\nnew_work=/w\norigin=/w\nserver_kind=socket\nserver_value=/private/no-such-rr-socket\nphase=prepared\nwork_dev=0\nwork_ino=0\nadmin_dev=0\nadmin_ino=0\n").unwrap()
}
fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, (char, Vec<u8>)> {
    let mut result = std::collections::BTreeMap::new();
    let mut todo = vec![PathBuf::new()];
    while let Some(relative) = todo.pop() {
        for entry in std::fs::read_dir(root.join(&relative)).unwrap() {
            let path = relative.join(entry.unwrap().file_name());
            let full = root.join(&path);
            let meta = std::fs::symlink_metadata(&full).unwrap();
            let content = if meta.file_type().is_symlink() {
                (
                    'l',
                    std::fs::read_link(&full)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                )
            } else if meta.is_dir() {
                todo.push(path.clone());
                ('d', Vec::new())
            } else {
                ('f', std::fs::read(&full).unwrap())
            };
            result.insert(path, content);
        }
    }
    result
}
fn refused(root: &Root, value: &Intent, old: &str, new: &str, want: &str) {
    let carrier = root.0.join("sessions/sentinel.intent");
    std::fs::write(&carrier, b"unchanged carrier\n").unwrap();
    let before_tree = tree(&root.0.join("sessions"));
    let snapshot: Vec<_> = ["sold", "snew"]
        .into_iter()
        .map(|n| {
            (
                n,
                std::fs::read(root.0.join("sessions").join(n).join("meta")).ok(),
            )
        })
        .collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        recover(&root.0, old, new, value, None, &mut out, &mut err).unwrap(),
        1
    );
    assert_eq!(out, b"", "no success on refusal");
    assert_eq!(String::from_utf8(err).unwrap(), format!("{want}\n"));
    assert_eq!(std::fs::read(carrier).unwrap(), b"unchanged carrier\n");
    for (name, before) in snapshot {
        assert_eq!(
            std::fs::read(root.0.join("sessions").join(name).join("meta")).ok(),
            before
        );
    }
    assert_eq!(
        tree(&root.0.join("sessions")),
        before_tree,
        "refusal preserves every state entry and byte"
    );
}

#[test]
fn recovery_command_guards_preserve_exact_precedence_and_text() {
    let root = Root::new("guards");
    let mut value = intent();
    refused(
        &root,
        &value,
        "different",
        "snew",
        "Error: the rename intent for 'different' → 'snew' names 'sold' → 'snew' — refusing a carrier that does not match this command. Nothing was renamed.",
    );
    // Multiple defects: correlation must precede self-name and local phase.
    value.new = "sold".into();
    value.phase = "work-moved".into();
    refused(
        &root,
        &value,
        "sold",
        "snew",
        "Error: the rename intent for 'sold' → 'snew' names 'sold' → 'sold' — refusing a carrier that does not match this command. Nothing was renamed.",
    );
    refused(
        &root,
        &value,
        "sold",
        "sold",
        "Error: the recorded intent renames 'sold' to itself — refusing damaged intent. Nothing was renamed.",
    );
    value.new = "snew".into();
    root.side("snew", "replacement");
    refused(
        &root,
        &value,
        "sold",
        "snew",
        "Error: the recorded intent is local mode at unreachable phase 'work-moved' — refusing damaged intent. Nothing was renamed.",
    );
}

#[test]
fn recovery_location_matrix_preserves_occupants_and_uses_new_side_when_both_hold() {
    let value = intent();
    let foreign = "0199c0de-1111-4890-abcd-ef0123456789";
    for (index, old, new, arm) in [
        (0, None, None, "gone"),
        (1, Some(foreign), None, "lost"),
        (2, None, Some(foreign), "new"),
        (3, Some(foreign), Some(foreign), "new"),
        (4, Some(value.uuid.as_str()), Some(foreign), "new"),
        (5, Some(foreign), Some(value.uuid.as_str()), "old"),
        (6, Some(value.uuid.as_str()), None, "server"),
        (7, None, Some(value.uuid.as_str()), "server"),
        (
            8,
            Some(value.uuid.as_str()),
            Some(value.uuid.as_str()),
            "server",
        ),
    ] {
        let root = Root::new(&format!("sides-{index}"));
        if let Some(id) = old {
            root.side("sold", id);
        }
        if let Some(id) = new {
            root.side("snew", id);
        }
        if index == 8 {
            // Both hold the UUID; old would yield CHANGED, new AMBIGUOUS.
            // The opposed readable values prove the new-side preference.
            std::fs::write(
                root.0.join("sessions/sold/meta"),
                format!(
                    "session=sold\nsession_id={}\ntmux_server_kind=socket\ntmux_server={}\n",
                    value.uuid,
                    root.0.join("absent-socket").display()
                ),
            )
            .unwrap();
        }
        let want = match arm {
            "gone" => format!("Error: both 'sold' and 'snew' are gone for UUID '{}' — refusing (manual repair only). Nothing was renamed.", value.uuid),
            "lost" => format!("Error: neither 'sold' nor 'snew' holds UUID '{}' anymore — refusing over replaced identity (manual repair only). Nothing was renamed.", value.uuid),
            "new" => format!("Error: '{}' is now occupied by another session — refusing to overwrite a replacement path/UUID. Retry 'ae rename sold snew' after freeing it.", root.0.join("sessions/snew").display()),
            "old" => format!("Error: '{}' is now occupied by another session — refusing to touch a replacement occupant. Retry 'ae rename sold snew' after freeing it.", root.0.join("sessions/sold").display()),
            _ => "Error: the recorded server for this transaction is now ambiguous — refusing a stale plan. Nothing was renamed.".into(),
        };
        refused(&root, &value, "sold", "snew", &want);
    }
}

#[test]
fn recovery_lost_uuid_with_regular_file_is_replaced_identity_not_gone() {
    let root = Root::new("lost-file");
    let value = intent();
    std::fs::write(root.0.join("sessions/sold"), b"replacement file\n").unwrap();
    refused(
        &root,
        &value,
        "sold",
        "snew",
        &format!(
            "Error: neither 'sold' nor 'snew' holds UUID '{}' anymore — refusing over replaced identity (manual repair only). Nothing was renamed.",
            value.uuid
        ),
    );
}

#[test]
fn recovery_server_change_precedes_unknown_liveness() {
    let mut value = intent();
    let root = Root::new("server-change");
    root.side("sold", &value.uuid);
    std::fs::write(
        root.0.join("sessions/sold/meta"),
        format!(
            "session=sold\nsession_id={}\ntmux_server_kind=socket\ntmux_server={}\n",
            value.uuid,
            root.0.join("absent-socket").display()
        ),
    )
    .unwrap();
    refused(
        &root,
        &value,
        "sold",
        "snew",
        &format!(
            "Error: the recorded server for this transaction changed while pending (recorded '{}/{}') — refusing a stale plan. Nothing was renamed.",
            value.server_kind, value.server_value
        ),
    );
    value.server_value = root.0.join("absent-socket").display().to_string();
    refused(
        &root,
        &value,
        "sold",
        "snew",
        "Error: cannot prove session 'sold' stopped (its recorded tmux server is unreachable) — refusing a retry over unknown liveness. Nothing was renamed.",
    );
}

struct PrivateServer {
    root: Root,
    server: ServerId,
    keeper: String,
}
impl PrivateServer {
    fn new(tag: &str) -> Self {
        let root = Root::new(tag);
        let server = ServerId::Selected(Selector::Socket(root.0.join("s")));
        let work = root.0.display().to_string();
        assert!(
            transport::run_tmux_op(&argv(
                &server,
                &Op::NewSession {
                    name: "keeper",
                    work_dir: &work
                }
            ))
            .0
        );
        let keeper = transport::observe_session_identity(&server, "keeper")
            .unwrap()
            .id;
        Self {
            root,
            server,
            keeper,
        }
    }
    fn prepared(&self) -> Intent {
        let mut value = intent();
        value.server_value = self.root.0.join("s").display().to_string();
        self.root.side("sold", &value.uuid);
        std::fs::write(self.root.0.join("sessions/sold/meta"), format!("session=sold\nsession_id={}\nmode=local\norigin=/w\nwork_dir=/w\ntmux_server_kind=socket\ntmux_server={}\nae_core=/bin/sh\n", value.uuid, value.server_value)).unwrap();
        publish_intent(&self.root.0, &value).unwrap();
        value
    }
}
impl Drop for PrivateServer {
    fn drop(&mut self) {
        if let Some(identity) = transport::observe_session_identity(&self.server, "sold") {
            assert!(transport::kill_session(&self.server, &identity.id));
        }
        assert!(transport::kill_session(&self.server, &self.keeper));
    }
}

#[test]
fn recovery_live_source_precedes_replacement_and_reports_original_phase() {
    let fixture = PrivateServer::new("live-precedence");
    let mut value = fixture.prepared();
    value.phase = "assets-published".into();
    fixture.root.side("snew", "replacement");
    assert!(
        transport::run_tmux_op(&argv(
            &fixture.server,
            &Op::NewSession {
                name: "sold",
                work_dir: &fixture.root.0.display().to_string()
            }
        ))
        .0
    );
    refused(
        &fixture.root,
        &value,
        "sold",
        "snew",
        "Error: session 'sold' is live while rename 'sold' → 'snew' is pending at phase 'assets-published' — stop it first, then retry 'ae rename sold snew'. Nothing was renamed.",
    );
    value.phase = "work-moved".into();
    refused(
        &fixture.root,
        &value,
        "sold",
        "snew",
        "Error: the recorded intent is local mode at unreachable phase 'work-moved' — refusing damaged intent. Nothing was renamed.",
    );
}

#[test]
fn recovery_tmux_destination_precedes_managed_address_and_payload_checks() {
    let fixture = PrivateServer::new("destination-precedence");
    let mut value = fixture.prepared();
    value.new = "keeper".into();
    value.mode = WorkMode::Full;
    refused(
        &fixture.root,
        &value,
        "sold",
        "keeper",
        "Error: session 'keeper' already exists — refusing to converge over an occupied destination. Nothing was renamed.",
    );
    value.new = "snew".into();
    refused(
        &fixture.root,
        &value,
        "sold",
        "snew",
        "Error: the managed addresses for this transaction moved while pending — refusing a stale plan. Nothing was renamed.",
    );
    value.old_work = fixture.root.0.join("worktrees/sold").display().to_string();
    value.new_work = fixture.root.0.join("worktrees/snew").display().to_string();
    refused(
        &fixture.root,
        &value,
        "sold",
        "snew",
        &format!(
            "Error: the configured worktrees root '{}' is not a real directory — refusing a retry over moved ground. Nothing was renamed.",
            fixture.root.0.join("worktrees").display()
        ),
    );
}

#[test]
fn recovery_each_managed_address_must_match_before_root_and_payload_checks() {
    let fixture = PrivateServer::new("one-managed-address");
    let mut value = fixture.prepared();
    value.mode = WorkMode::Full;
    for old_moved in [false, true] {
        value.old_work = fixture.root.0.join("worktrees/sold").display().to_string();
        value.new_work = fixture.root.0.join("worktrees/snew").display().to_string();
        if old_moved {
            value.old_work.push_str("-moved");
        } else {
            value.new_work.push_str("-moved");
        }
        refused(
            &fixture.root,
            &value,
            "sold",
            "snew",
            "Error: the managed addresses for this transaction moved while pending — refusing a stale plan. Nothing was renamed.",
        );
    }
}

#[test]
fn recovery_normalization_publish_failure_precedes_journal_and_preserves_state() {
    let fixture = PrivateServer::new("publish-failure");
    let mut value = fixture.prepared();
    value.phase = "assets-published".into();
    let path = intent_path(&fixture.root.0, "sold", "snew");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::create_dir(fixture.root.0.join("sessions/sold/events.jsonl")).unwrap();
    let before = std::fs::read(fixture.root.0.join("sessions/sold/meta")).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        recover(
            &fixture.root.0,
            "sold",
            "snew",
            &value,
            None,
            &mut out,
            &mut err
        )
        .unwrap(),
        1
    );
    let said = String::from_utf8(err).unwrap();
    assert!(
        said.starts_with("Error: the transaction phase corrected but its record failed ("),
        "{said}"
    );
    assert!(
        said.ends_with(" — retry 'ae rename sold snew'. Nothing was renamed.\n"),
        "{said}"
    );
    assert!(out.is_empty());
    assert_eq!(
        std::fs::read(fixture.root.0.join("sessions/sold/meta")).unwrap(),
        before
    );
    assert!(!fixture.root.0.join("sessions/snew").exists());
    assert!(path.is_dir());
}

#[test]
fn recovery_direct_complete_input_normalizes_before_blockers_instead_of_returning_success() {
    let fixture = PrivateServer::new("complete-normalizes");
    let mut value = fixture.prepared();
    complete_transaction_until(
        &fixture.root.0,
        &mut value,
        None,
        &mut Vec::new(),
        std::time::Instant::now(),
    )
    .unwrap();
    assert_eq!(value.phase, "complete");
    // Unreadable journal blocks the continuation after normalization. A direct
    // completed-record shortcut would incorrectly return success here.
    std::fs::create_dir(fixture.root.0.join("sessions/snew/events.jsonl")).unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        recover(
            &fixture.root.0,
            "sold",
            "snew",
            &value,
            None,
            &mut out,
            &mut err
        )
        .unwrap(),
        1
    );
    assert!(out.is_empty());
    let text = std::fs::read_to_string(intent_path(&fixture.root.0, "sold", "snew")).unwrap();
    assert!(
        text.contains("phase=assets-published\n"),
        "normalization is durable before blocker refusal: {text}"
    );
    assert!(
        !String::from_utf8(err)
            .unwrap()
            .contains("drifted completion")
    );
}
