//! S1b source proof, precedence and memo bounds through product archives.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use ae::topic::{Card, Field, Group};
use ae::web::topics::{self, Column, Sticky};

use crate::cli;

const AT_OLD: &str = "2026-08-01T12:00:00Z";
const AT_TIED: &str = "2026-09-01T12:00:00Z";
const AT_NEW: &str = "2026-10-01T12:00:00Z";

const OLD: &str = "0199c0de-8888-4890-abcd-ef0123456789";
const NEXT: &str = "0199c0de-1111-4890-abcd-ef0123456789";
const TIED: &str = "0199c0de-2222-4890-abcd-ef0123456789";
const CURRENT: &str = "0199c0de-3333-4890-abcd-ef0123456789";
const CLAIMED: &str = "0199c0de-4444-4890-abcd-ef0123456789";
const LEGACY: &str = "0199c0de-5555-4890-abcd-ef0123456789";
const FALSE: &str = "0199c0de-6666-4890-abcd-ef0123456789";
const DAMAGED: &str = "0199c0de-7777-4890-abcd-ef0123456789";
const DUPLICATE: &str = "0199c0de-9999-4890-abcd-ef0123456789";

const CURRENT_NAME: &str = "renamed-live";

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn find<'a>(column: &'a Column, slug: &str) -> Option<(&'a Group, &'a Card)> {
    column.groups.iter().find_map(|(group, cards)| {
        cards
            .iter()
            .find(|card| card.slug == slug)
            .map(|card| (group, card))
    })
}

fn matched_field(field: Option<&Field>, value: &str, session: &str, uuid: &str, at: &str) {
    let field = field.unwrap_or_else(|| panic!("expected field {value}"));
    assert_eq!(field.value, value);
    assert_eq!(field.origin.session, session);
    assert_eq!(field.origin.uuid, uuid);
    assert_eq!(field.origin.at, at);
}

#[test]
fn real_archives_rank_by_time_then_id_and_keep_only_one_copy_of_the_current_incarnation() {
    let rig = Archives::new("precedence");
    let old = rig.publish_retained(
        "earlier-renamed",
        OLD,
        AT_OLD,
        "2026-12-01T12:00:00Z\tlead\ttopic-release\ttitle: earliest archive title | what: kept across incarnations | state: closed\n",
    );
    let old_meta = read(&old.join("meta"));
    assert_eq!(
        old_meta
            .lines()
            .filter(|line| *line == "source_meta_agent=true")
            .count(),
        1
    );
    let overlay = format!(
        "source_local_config={}",
        rig.root().join("orchestrator.config").display()
    );
    assert_eq!(
        old_meta
            .lines()
            .filter(|line| *line == overlay.as_str())
            .count(),
        1
    );

    rig.publish_retained(
        "later-seat",
        NEXT,
        AT_TIED,
        "2026-11-01T12:00:00Z\tlead\ttopic-release\ttitle: lower-id title | last: earlier append\n\
         2026-10-01T12:00:00Z\tlead\ttopic-release\tlast: later append wins\n",
    );
    rig.publish_retained(
        "later-with-greater-id",
        TIED,
        AT_TIED,
        "2026-07-01T12:00:00Z\tlead\ttopic-release\ttitle: greatest id at tied time\n",
    );
    let current_dir = rig.source(CURRENT_NAME, CURRENT, "orchestrator.config");
    let memo = current_dir.join("memo.tsv");
    write(
        &memo,
        "2026-10-02T12:00:00Z\tlead\ttopic-release\ttitle: transit title | src: archived duplicate must not win\n",
    );
    let transitional = rig.publish(&current_dir, CURRENT, AT_NEW);
    let source_id = format!("source_session_id={CURRENT}");
    assert!(
        read(&transitional.join("meta"))
            .lines()
            .any(|line| line == source_id)
    );
    write(
        &memo,
        "2026-03-03T12:00:00Z\tlead\ttopic-release\tstate: parked | next: current source wins\n",
    );

    let column = topics::collect(rig.root(), &mut Sticky::default());
    let (group, card) = find(&column, "release").unwrap_or_else(|| panic!("retained release card"));
    assert!(matches!(group, Group::Parked));
    matched_field(
        card.title.as_ref(),
        "greatest id at tied time",
        "later-with-greater-id",
        TIED,
        "2026-07-01T12:00:00Z",
    );
    matched_field(
        card.what.as_ref(),
        "kept across incarnations",
        "earlier-renamed",
        OLD,
        "2026-12-01T12:00:00Z",
    );
    matched_field(
        card.last.as_ref(),
        "later append wins",
        "later-seat",
        NEXT,
        "2026-10-01T12:00:00Z",
    );
    matched_field(
        card.next.as_ref(),
        "current source wins",
        CURRENT_NAME,
        CURRENT,
        "2026-03-03T12:00:00Z",
    );
    assert_eq!(card.reviewed, "2026-03-03T12:00:00Z");
    assert!(card.src.is_none());
    assert_eq!(card.history.len(), 1);
    assert_eq!(card.history[0].origin.uuid, NEXT);
    assert!(card.history[0].text.contains("lower-id title"));
    assert_eq!(
        column.skipped.claimed + column.skipped.unproven + column.skipped.invalid,
        0
    );
}

#[test]
fn claimed_unproven_and_invalid_archives_each_stay_out_for_their_own_reason() {
    let rig = Archives::new("skips");
    rig.publish_retained(
        "proved-seat",
        OLD,
        AT_OLD,
        "2026-01-01T12:00:00Z\tlead\ttopic-real\tstate: quiet | title: proved retained source\n",
    );
    rig.publish_retained(
        "claimed-seat",
        CLAIMED,
        AT_NEW,
        "2026-01-01T12:00:00Z\tlead\ttopic-claimed\tstate: needs\n",
    );
    directory(
        &rig.root()
            .join("archive")
            .join(format!(".publishing.{CLAIMED}")),
    );

    let legacy = rig.publish_retained(
        "legacy-seat",
        LEGACY,
        AT_NEW,
        "2026-01-01T12:00:00Z\tlead\ttopic-legacy\tstate: needs\n",
    );
    let legacy_meta = legacy.join("meta");
    let legacy_bytes = read(&legacy_meta);
    let mut old_shape = legacy_bytes
        .lines()
        .filter(|line| {
            !line.starts_with("source_meta_agent=") && !line.starts_with("source_local_config=")
        })
        .collect::<Vec<_>>()
        .join("\n");
    old_shape.push('\n');
    write(&legacy_meta, old_shape);

    rig.publish_with_overlay(
        "false-seat",
        FALSE,
        AT_NEW,
        "2026-01-01T12:00:00Z\tlead\ttopic-false\tstate: needs\n",
        "other.config",
    );
    let duplicate = rig.source("duplicate-seat", DUPLICATE, "orchestrator.config");
    let duplicate_meta = duplicate.join("meta");
    let twice = format!("{}meta_agent=true\n", read(&duplicate_meta));
    write(&duplicate_meta, twice);
    write(
        &duplicate.join("memo.tsv"),
        "2026-01-01T12:00:00Z\tlead\ttopic-duplicate\tstate: needs\n",
    );
    let _ = rig.publish(&duplicate, DUPLICATE, AT_NEW);
    std::fs::remove_dir_all(&duplicate)
        .unwrap_or_else(|error| panic!("remove fixture source: {error}"));

    let damaged = rig.publish_retained(
        "damaged-seat",
        DAMAGED,
        AT_NEW,
        "2026-01-01T12:00:00Z\tlead\ttopic-damaged\tstate: needs\n",
    );
    write(&damaged.join("unexpected"), "tree validator must see this");

    // A current impostor cannot contribute its memo even when its config
    // content matches the proved overlay's content.
    let false_live = rig.source("current-impostor", CURRENT, "other.config");
    write(
        &false_live.join("memo.tsv"),
        "2026-01-01T12:00:00Z\tlead\ttopic-current-impostor\tstate: needs\n",
    );

    let column = topics::collect(rig.root(), &mut Sticky::default());
    let (group, control) =
        find(&column, "real").unwrap_or_else(|| panic!("valid archived control"));
    assert!(matches!(group, Group::Quiet));
    assert_eq!(
        control.title.as_ref().map(|field| field.value.as_str()),
        Some("proved retained source")
    );
    assert_eq!(column.skipped.claimed, 1);
    assert_eq!(column.skipped.unproven, 3);
    assert_eq!(column.skipped.invalid, 1);
    for slug in [
        "claimed",
        "legacy",
        "false",
        "duplicate",
        "damaged",
        "current-impostor",
    ] {
        assert!(find(&column, slug).is_none(), "unexpected card from {slug}");
    }
}

#[test]
fn a_memo_over_the_inclusive_cap_contributes_nothing_and_leaves_a_sticky_gap() {
    let rig = Archives::new("cap");
    let seat = rig.source(CURRENT_NAME, CURRENT, "orchestrator.config");
    let memo = seat.join("memo.tsv");
    let valid = b"2026-01-01T12:00:00Z\tlead\ttopic-under-cap\tstate: quiet\n";
    let mut sized = valid.to_vec();
    sized.resize(8 * 1024 * 1024 - 1, b'x');
    sized.push(b'\n');
    write(&memo, &sized);
    let mut sticky = Sticky::default();
    let exact = topics::collect(rig.root(), &mut sticky);
    assert!(find(&exact, "under-cap").is_some());
    assert!(exact.gaps.iter().all(|gap| !gap.contains("exceeds 8 MiB")));

    sized.push(b'\n');
    write(&memo, &sized);
    let refused = topics::collect(rig.root(), &mut sticky);
    assert!(find(&refused, "under-cap").is_none());
    let expected =
        format!("memo of {CURRENT_NAME} ({CURRENT}) exceeds 8 MiB: its topics are not shown");
    assert!(refused.gaps.contains(&expected));

    write(&memo, valid);
    let later = topics::collect(rig.root(), &mut sticky);
    assert!(later.gaps.contains(&expected));
    let fresh = topics::collect(rig.root(), &mut Sticky::default());
    assert!(find(&fresh, "under-cap").is_some());
    assert!(!fresh.gaps.contains(&expected));
}

#[allow(
    clippy::disallowed_methods,
    reason = "read only the fixture's own published archive"
)]
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn directory(path: &Path) {
    std::fs::create_dir_all(path)
        .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
}

struct Archives {
    owned: cli::OwnedScratch,
}

impl Archives {
    fn new(tag: &str) -> Self {
        let owned = cli::OwnedScratch::root("wtpc", tag);
        directory(&owned.path().join("archive"));
        write(
            &owned.path().join("orchestrator.config"),
            "[workspace]\norchestrator = true\n",
        );
        write(
            &owned.path().join("other.config"),
            "[workspace]\norchestrator = true\n",
        );
        Self { owned }
    }

    fn root(&self) -> &Path {
        self.owned.path()
    }

    fn source(&self, name: &str, uuid: &str, overlay: &str) -> PathBuf {
        let dir = self.root().join("sessions").join(name);
        directory(&dir.join("messages"));
        write(
            &dir.join("meta"),
            format!(
                "schema=2\nsession={name}\nsession_id={uuid}\nsession_id_origin=session\n\
                 mode=worktree\nlayout=lead-pair\nseat.main=lead\nseat.worker.0=colead\n\
                 agent.main=cl:lead:sid-fixture\nagent.worker.0=cl:colead:sid-fixture2\n\
                 tmux_server_kind=socket\ntmux_server={}\nmeta_agent=true\nlocal_config={}\n",
                self.root().join("no-server").display(),
                self.root().join(overlay).display(),
            ),
        );
        write(&dir.join("events.jsonl"), "");
        dir
    }

    fn publish(&self, dir: &Path, uuid: &str, at: &str) -> PathBuf {
        let child = cli::ae()
            .env("AE_HOME", self.root())
            .arg("_archive-publish")
            .arg(dir)
            .args(["not-managed", "-", "-", "-", at])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("launch _archive-publish: {error}"));
        let output = cli::bounded(child, Duration::from_secs(10))
            .unwrap_or_else(|| panic!("archive publication for {uuid} timed out"));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let target = self.root().join("archive").join(uuid);
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with(&format!("{}\t", target.display()))
        );
        target
    }

    fn publish_retained(&self, name: &str, uuid: &str, at: &str, memo: &str) -> PathBuf {
        self.publish_with_overlay(name, uuid, at, memo, "orchestrator.config")
    }

    fn publish_with_overlay(
        &self,
        name: &str,
        uuid: &str,
        at: &str,
        memo: &str,
        overlay: &str,
    ) -> PathBuf {
        let dir = self.source(name, uuid, overlay);
        write(&dir.join("memo.tsv"), memo);
        let archive = self.publish(&dir, uuid, at);
        std::fs::remove_dir_all(&dir)
            .unwrap_or_else(|error| panic!("retire the scratch source: {error}"));
        archive
    }
}
