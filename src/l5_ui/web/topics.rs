//! The desk column: the topic cards of the orchestrator's memo incarnations
//! and the owner waits that join them by `#slug` tag.
//!
//! Sources: the live role holder's session, proven by [`crate::meta::is_seat`],
//! and each kept archive that validates and whose meta proves it was published
//! from that seat (`source_meta_agent` plus `source_local_config`, read by the
//! same predicate). Waits: the orchestrator board's DECIDE items and the
//! owner's own console asks still open and not retired.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::events::{Event, SessionKey};
use crate::store::SourceRead;
use crate::topic::{self, Card, Group};

/// The largest `memo.tsv` a source may have, in bytes; one byte more and the
/// source contributes nothing.
pub const MEMO_MAX: u64 = 8 * 1024 * 1024;

/// The tag that marks a DECIDE item the page may only show. It names no
/// topic: no wait joins it and no chat row carries it.
pub const RESERVED: &str = "reserved";

/// What kind of owner action a wait is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    /// A DECIDE item on the orchestrator's board.
    Decide,
    /// The owner's own console ask, still open.
    Ask,
}

impl WaitKind {
    /// The kind's word on the wire.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Decide => "decide",
            Self::Ask => "ask",
        }
    }
}

/// One owner action a topic waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub kind: WaitKind,
    pub text: String,
    /// The ask's request id; a DECIDE item has none.
    pub id: Option<String>,
    /// The board id of a DECIDE item, `D67`; an ask has none.
    pub item: Option<crate::desk::Id>,
}

/// One DECIDE item of the board as the page may answer it: its id, the stored
/// text a `bind` signs, and the topics its text tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecideItem {
    pub item: crate::desk::Id,
    /// The item's text as the board stores it, never a rendered form.
    pub text: String,
    pub tags: Vec<String>,
}

/// The archives left out, by reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Skipped {
    /// A publisher or a purge holds its claim.
    pub claimed: usize,
    /// No proof it was published from the orchestrator seat.
    pub unproven: usize,
    /// Its tree does not validate.
    pub invalid: usize,
}

/// The desk column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Column {
    /// All six groups in [`Group::ALL`] order, empty ones kept; cards by slug.
    pub groups: Vec<(Group, Vec<Card>)>,
    /// The waits of each tagged slug.
    pub waits: BTreeMap<String, Vec<Wait>>,
    /// DECIDE items without a tag.
    pub untagged: Vec<Wait>,
    /// Every DECIDE item once, in board order, tagged or not.
    pub decide: Vec<DecideItem>,
    /// What could not be read, one line each.
    pub gaps: Vec<String>,
    pub skipped: Skipped,
    /// Where each slug's highest-ranked record came from: the record a card
    /// names in its header.
    pub newest: BTreeMap<String, topic::Origin>,
}

/// What lives as long as the listener: the memos once seen over
/// [`MEMO_MAX`], by session uuid, with their gap line, and each kept
/// archive's verdict with the tree identity it was judged at.
#[derive(Debug, Default)]
pub struct Sticky {
    over: BTreeMap<String, String>,
    judged: BTreeMap<String, (String, Judged)>,
}

/// What one kept archive proved when its whole tree was validated.
#[derive(Debug, Clone)]
enum Judged {
    /// Its tree does not validate.
    Invalid,
    /// A valid archive and the meta facts the column reads.
    Valid {
        source_id: String,
        archived_at: String,
        session: String,
        /// Whether its meta proves it was published from the seat.
        proven: bool,
    },
}

/// The live seat as the collector reads it.
struct Seat {
    name: String,
    dir: PathBuf,
    uuid: String,
}

/// The desk column of state root `root`.
pub fn collect(root: &Path, sticky: &mut Sticky) -> Column {
    let mut column = Column::default();
    let seat = live_seat(root, &mut column.gaps);
    let mut sources = archived(root, seat.as_ref(), sticky, &mut column);
    if let Some(seat) = &seat {
        sources.push((
            seat.name.clone(),
            seat.uuid.clone(),
            seat.dir.join(crate::store::MEMO),
        ));
    }
    let mut memos = Vec::new();
    for (incarnation, (session, uuid, path)) in sources.into_iter().enumerate() {
        if let Some(memo) = memo(&session, &uuid, &path, sticky, &mut column.gaps) {
            memos.push(topic::Source {
                session,
                uuid,
                incarnation,
                memo,
            });
        }
    }
    column.gaps.extend(sticky.over.values().cloned());
    if let Some(seat) = &seat {
        decide_waits(seat, &mut column);
        ask_waits(seat, &mut column);
    }
    column.newest = newest(&memos);
    column.groups = arrange(topic::cards(&memos), &column.waits);
    column
}

/// The origin of each slug's highest-ranked record of `sources`, ranked as
/// [`topic::entries`] ranks: a later incarnation above an earlier one and,
/// within one memo, a later record above an earlier one, whatever their
/// timestamps say. One pass that keeps no record text.
fn newest(sources: &[topic::Source]) -> BTreeMap<String, topic::Origin> {
    let mut found: BTreeMap<String, ((usize, usize), &topic::Source, &[u8])> = BTreeMap::new();
    for source in sources {
        for (place, record) in crate::memo::records(&source.memo).enumerate() {
            let Some(slug) = std::str::from_utf8(record.topic)
                .ok()
                .and_then(topic::slug_of)
            else {
                continue;
            };
            let rank = (source.incarnation, place);
            match found.get_mut(slug) {
                Some(kept) if kept.0 < rank => *kept = (rank, source, record.ts),
                Some(_) => {}
                None => {
                    found.insert(slug.to_owned(), (rank, source, record.ts));
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(slug, (_, source, ts))| {
            let origin = topic::Origin {
                session: source.session.clone(),
                uuid: source.uuid.clone(),
                at: String::from_utf8_lossy(ts).into_owned(),
            };
            (slug, origin)
        })
        .collect()
}

/// The six groups in [`Group::ALL`] order, each with its cards by slug but
/// the Quiet group, which is by review ([`topic::review_order`]): the memo
/// cards of `cards`, sorted by slug as [`topic::cards`] returns them, and one
/// bare card for each slug a wait tags that no memo files, inserted at its
/// place so the order the search needs holds for the next slug.
fn arrange(mut cards: Vec<Card>, waits: &BTreeMap<String, Vec<Wait>>) -> Vec<(Group, Vec<Card>)> {
    for slug in waits.keys() {
        if let Err(at) = cards.binary_search_by(|card| card.slug.as_str().cmp(slug)) {
            cards.insert(at, Card::bare(slug));
        }
    }
    let mut groups: Vec<(Group, Vec<Card>)> = Group::ALL
        .iter()
        .map(|group| (*group, Vec::new()))
        .collect();
    for card in cards {
        let group = topic::group(&card, waits.contains_key(&card.slug));
        if let Some((_, list)) = groups.iter_mut().find(|(each, _)| *each == group) {
            list.push(card);
        }
    }
    // Quiet topics come the oldest review first: that order is the Revisit
    // cycle's. The other groups keep their slug order.
    if let Some((_, quiet)) = groups.iter_mut().find(|(each, _)| *each == Group::Quiet) {
        quiet.sort_by(topic::review_order);
    }
    groups
}

/// The role holder's session when its meta proves the seat.
fn live_seat(root: &Path, gaps: &mut Vec<String>) -> Option<Seat> {
    let name = match crate::settings_menu::role_holder(root) {
        Ok(name) => name?,
        Err(why) => {
            gaps.push(format!("orchestrator role: {why}"));
            return None;
        }
    };
    let dir = crate::console::locate(root, &name)?;
    let meta = crate::meta::read_bytes(&dir).ok()?;
    if !crate::meta::is_seat(&meta, root) {
        return None;
    }
    let uuid = crate::meta::sole_value(&meta, "session_id")
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default();
    Some(Seat { name, dir, uuid })
}

/// The archived incarnations of the seat, oldest first: `(session, uuid, memo
/// path)` by `archived_at`, then archive id. Counts each archive left out.
/// An archive is judged again only when its tree identity moved: a kept
/// archive does not change, so a poll costs a metadata walk, not a read.
fn archived(
    root: &Path,
    seat: Option<&Seat>,
    sticky: &mut Sticky,
    column: &mut Column,
) -> Vec<(String, String, PathBuf)> {
    let kept = match crate::archive::kept_archives(root) {
        Ok(kept) => kept,
        Err(why) => {
            column.gaps.push(format!("archives: {why}"));
            return Vec::new();
        }
    };
    sticky
        .judged
        .retain(|id, _| kept.iter().any(|archive| archive.id == *id));
    let mut found: Vec<(String, String, String, PathBuf)> = Vec::new();
    for archive in kept {
        if archive.claimed {
            column.skipped.claimed += 1;
            continue;
        }
        // The identity is taken before the judgment, so a tree that moves
        // while it is judged is judged again at the next poll.
        let identity = crate::archive::tree_identity(&archive.path).ok();
        let judged = match (&identity, sticky.judged.get(&archive.id)) {
            (Some(now), Some((then, judged))) if now == then => judged.clone(),
            _ => {
                let judged = judge(&archive.path, &archive.id, root);
                if let Some(identity) = identity {
                    sticky
                        .judged
                        .insert(archive.id.clone(), (identity, judged.clone()));
                }
                judged
            }
        };
        match judged {
            Judged::Invalid => column.skipped.invalid += 1,
            Judged::Valid { source_id, .. } if seat.is_some_and(|seat| source_id == seat.uuid) => {}
            Judged::Valid { proven: false, .. } => column.skipped.unproven += 1,
            Judged::Valid {
                archived_at,
                session,
                ..
            } => {
                let memo = archive.path.join(crate::store::MEMO);
                found.push((archived_at, archive.id, session, memo));
            }
        }
    }
    found.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    found
        .into_iter()
        .map(|(_, id, session, memo)| (session, id, memo))
        .collect()
}

/// Whether an archive's `meta` proves its source was the orchestrator seat
/// of `root`: its `source_meta_agent` and `source_local_config` rows read as
/// a live meta's role claim and overlay row pass [`crate::meta::is_seat`].
fn published_from_seat(meta: &[u8], root: &Path) -> bool {
    let mut view = Vec::new();
    for line in meta.split(|byte| *byte == b'\n') {
        if let Some(row) = line.strip_prefix(b"source_".as_slice())
            && (row.starts_with(b"meta_agent") || row.starts_with(b"local_config"))
        {
            view.extend_from_slice(row);
            view.push(b'\n');
        }
    }
    crate::meta::is_seat(&view, root)
}

/// The verdict on the kept archive `id` at `path`: its whole tree validated
/// by the archive's own validator, then its meta read for the column.
fn judge(path: &Path, id: &str, root: &Path) -> Judged {
    let meta = crate::archive::validated_journal(path, id, &mut whole)
        .ok()
        .and_then(|_| whole(&path.join(crate::meta::FILE)).ok());
    let Some(meta) = meta else {
        return Judged::Invalid;
    };
    let value = |key: &str| {
        crate::meta::sole_value(&meta, key)
            .map(|value| String::from_utf8_lossy(value).into_owned())
            .unwrap_or_default()
    };
    Judged::Valid {
        source_id: value("source_session_id"),
        archived_at: value("archived_at"),
        session: value("source_session"),
        proven: published_from_seat(&meta, root),
    }
}

/// A whole regular file, as the archive validator reads its members: no size
/// bound, because a valid archive's digest grows with its memo's handovers,
/// and a node that is no regular file is refused before any open.
fn whole(path: &Path) -> std::io::Result<Vec<u8>> {
    match crate::store::read_source(path) {
        SourceRead::Ready(bytes) => Ok(bytes),
        SourceRead::Absent => Err(std::io::ErrorKind::NotFound.into()),
        SourceRead::Invalid(why) | SourceRead::Unreadable(why) => Err(std::io::Error::other(why)),
    }
}

/// The memo bytes of one source, or `None` when it has none to show. A memo
/// over [`MEMO_MAX`], now or before in this listener's life, shows nothing.
fn memo(
    session: &str,
    uuid: &str,
    path: &Path,
    sticky: &mut Sticky,
    gaps: &mut Vec<String>,
) -> Option<Vec<u8>> {
    if sticky.over.contains_key(uuid) {
        return None;
    }
    match crate::store::read_capped(path, MEMO_MAX) {
        Ok(SourceRead::Ready(bytes)) => Some(bytes),
        Ok(SourceRead::Absent) => None,
        Ok(SourceRead::Invalid(why) | SourceRead::Unreadable(why)) => {
            gaps.push(format!("memo of {session} ({uuid}): {why}"));
            None
        }
        Err(crate::store::Oversized) => {
            let line =
                format!("memo of {session} ({uuid}) exceeds 8 MiB: its topics are not shown");
            sticky.over.insert(uuid.to_owned(), line);
            None
        }
    }
}

/// Whether `text` marks its item [`RESERVED`].
#[must_use]
pub fn is_reserved(text: &str) -> bool {
    topic::tags(text).contains(&RESERVED)
}

/// The topics `text` tags: its `#slug` tags without [`RESERVED`].
fn topic_tags(text: &str) -> Vec<&str> {
    let mut tags = topic::tags(text);
    tags.retain(|tag| *tag != RESERVED);
    tags
}

/// Add `wait` to each slug its text tags, or to the untagged list when
/// `untagged` takes them.
fn join(column: &mut Column, tags: &[&str], wait: Wait, untagged: bool) {
    if tags.is_empty() {
        if untagged {
            column.untagged.push(wait);
        }
        return;
    }
    for tag in tags {
        column
            .waits
            .entry((*tag).to_owned())
            .or_default()
            .push(wait.clone());
    }
}

/// The DECIDE items of the seat's board.
fn decide_waits(seat: &Seat, column: &mut Column) {
    let desk = match crate::deskstore::read(&seat.dir) {
        Ok(desk) => desk,
        Err(why) => {
            column.gaps.push(format!("board of {}: {why}", seat.name));
            return;
        }
    };
    for item in desk.into_iter().flat_map(|desk| desk.items) {
        if item.id.section != crate::desk::Section::Decide {
            continue;
        }
        let tags = topic_tags(&item.text);
        let wait = Wait {
            kind: WaitKind::Decide,
            text: item.text.clone(),
            id: None,
            item: Some(item.id),
        };
        column.decide.push(DecideItem {
            item: item.id,
            text: item.text.clone(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        });
        join(column, &tags, wait, true);
    }
}

/// The owner's console asks of the seat still open and not retired.
fn ask_waits(seat: &Seat, column: &mut Column) {
    let container = crate::store::open(&seat.dir).container();
    let events: Vec<Event> = container
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| Event::parse_line(line).ok())
        .collect();
    let key = SessionKey::new(&seat.name, &seat.uuid);
    let requests = crate::requests::states_in(&container, key);
    let open: BTreeSet<&str> = crate::console::lane::open_asks(&events, &seat.name)
        .into_iter()
        .collect();
    for id in open {
        let Some(opening) = events.iter().rev().find(|event| {
            event.reference.as_deref() == Some(id)
                && event.actor == crate::tracked::CONSOLE_SINK
                && event.action == crate::tracked::Kind::Ask.action()
        }) else {
            continue;
        };
        if opening.relay_mode.is_some() {
            continue;
        }
        let retired = requests
            .iter()
            .find(|request| request.id.as_slice() == id.as_bytes())
            .is_some_and(|request| {
                matches!(
                    request.lifetime_in(&container, key),
                    crate::requests::Lifetime::Retired(_)
                )
            });
        if retired {
            continue;
        }
        let text = match crate::console::ask_body(&seat.dir, opening) {
            crate::console::lane::Body::Whole(text) => text,
            _ => opening.summary.clone().unwrap_or_default(),
        };
        let tags = topic_tags(&text);
        let wait = Wait {
            kind: WaitKind::Ask,
            text: text.lines().next().unwrap_or_default().to_owned(),
            id: Some(id.to_owned()),
            item: None,
        };
        join(column, &tags, wait, false);
    }
}

#[cfg(test)]
mod web_topics_waits_spec;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Column, Sticky, Wait, WaitKind, arrange, collect, is_reserved, topic_tags};
    use crate::json::Value;
    use crate::topic::{self, Group};

    /// docs/web.md: a card's header names its highest-ranked record. A later
    /// incarnation ranks above an earlier one at the same timestamp, and within
    /// one memo a later record ranks above an earlier one with a newer
    /// timestamp. The sources' order does not matter, and the result is the
    /// first record `topic::entries` ranks for every slug.
    #[test]
    fn newest_ranks_by_incarnation_and_place_not_by_timestamp() {
        let source = |session: &str, incarnation: usize, memo: &str| topic::Source {
            session: session.to_owned(),
            uuid: format!("uuid-{session}"),
            incarnation,
            memo: memo.as_bytes().to_vec(),
        };
        let sources = [
            source(
                "archive",
                0,
                "2026-11-24T10:00:00Z\tlead\ttopic-tie\tstate: quiet\n\
                 2026-11-24T12:00:00Z\tlead\ttopic-order\tstate: moving\n",
            ),
            source(
                "live",
                1,
                "2026-11-24T10:00:00Z\tlead\ttopic-tie\tstate: moving\n\
                 2026-11-24T11:00:00Z\tlead\ttopic-order\ttitle: First\n\
                 2026-11-24T09:00:00Z\tlead\ttopic-order\taudit: later line, older time\n\
                 2026-11-24T13:00:00Z\tlead\ttopic-other\tstate: quiet\n",
            ),
        ];
        let newest = super::newest(&sources);
        let at = |slug: &str| {
            newest
                .get(slug)
                .map(|origin| (origin.session.as_str(), origin.at.as_str()))
        };
        assert_eq!(at("tie"), Some(("live", "2026-11-24T10:00:00Z")));
        assert_eq!(at("order"), Some(("live", "2026-11-24T09:00:00Z")));
        assert_eq!(at("other"), Some(("live", "2026-11-24T13:00:00Z")));
        assert_eq!(newest.len(), 3);
        let [archive, live] = sources.clone();
        assert_eq!(super::newest(&[live, archive]), newest);
        let entries = topic::entries(&sources);
        for (slug, origin) in &newest {
            let first = entries.get(slug).and_then(|list| list.first());
            assert_eq!(first.map(|entry| &entry.origin), Some(origin), "{slug}");
        }
    }

    /// A scratch state root removed when it drops.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
            let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir()
                .join(format!("ae-web-topics-{}-{sequence}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("archive"))
                .unwrap_or_else(|error| panic!("scratch: {error}"));
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &std::path::Path, bytes: impl AsRef<[u8]>) {
        std::fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }

    /// `#reserved` marks an item and names no topic; only the whole tag does.
    #[test]
    fn the_reserved_tag_marks_and_names_no_topic() {
        let text = "Go? #reserved #ship #reserved_extra";
        assert!(is_reserved(text));
        assert_eq!(topic_tags(text), ["ship"]);
        assert!(!is_reserved("Go? #reserved_extra #reserved-too #ship"));
        assert_eq!(topic_tags("#reserved-too"), ["reserved-too"]);
    }

    #[test]
    fn a_valid_archive_whose_digest_outgrows_four_mib_still_shows_its_topics() {
        const UUID: &str = "0199c0de-4321-4890-abcd-ef0123456789";
        let scratch = Scratch::new();
        let root = &scratch.0;
        write(
            &root.join("orchestrator.config"),
            "[workspace]\norchestrator = true\n",
        );
        let dir = root.join("sessions").join("long-handovers");
        std::fs::create_dir_all(dir.join("messages"))
            .unwrap_or_else(|error| panic!("session: {error}"));
        write(
            &dir.join("meta"),
            format!(
                "schema=2\nsession=long-handovers\nsession_id={UUID}\nsession_id_origin=session\n\
                 mode=worktree\nlayout=lead-pair\nseat.main=lead\n\
                 tmux_server_kind=socket\ntmux_server={}\nmeta_agent=true\nlocal_config={}\n",
                root.join("no-server").display(),
                root.join("orchestrator.config").display(),
            ),
        );
        write(&dir.join("events.jsonl"), "");
        // 5 MiB of handover in a memo well under its 8 MiB cap: the digest
        // renders it whole and indented, so it passes 4 MiB.
        let mut memo = b"2026-08-01T10:00:00Z\tlead\thandover\t".to_vec();
        memo.resize(memo.len() + 5 * 1024 * 1024, b'h');
        memo.extend_from_slice(b"\n2026-08-01T11:00:00Z\tlead\ttopic-kept\tstate: quiet\n");
        write(&dir.join("memo.tsv"), &memo);
        let ops = crate::archive::publish::Ops {
            reason: crate::archive::publish::Reason::End,
            push_outcome: "not-managed",
            push_ref: "-",
            preserved: "-",
            workdir: "-",
            archived_at: "2026-08-01T12:00:00Z",
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = crate::archive::publish::run(&dir, &ops, &mut out, &mut err)
            .unwrap_or_else(|error| panic!("publish: {error}"));
        assert_eq!(code, 0, "publish: {}", String::from_utf8_lossy(&err));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|error| panic!("retire source: {error}"));
        let crate::store::SourceRead::Ready(digest) =
            crate::store::read_source(&root.join("archive").join(UUID).join("digest.md"))
        else {
            panic!("the published digest");
        };
        assert!(digest.len() > 4 << 20, "digest of {} bytes", digest.len());

        let column = collect(root, &mut Sticky::default());
        assert_eq!(column.skipped.invalid, 0);
        let kept = column
            .groups
            .iter()
            .flat_map(|(_, cards)| cards)
            .find(|card| card.slug == "kept")
            .unwrap_or_else(|| panic!("the archived topic: {:?}", column.skipped));
        assert_eq!(
            kept.state.as_ref().map(|state| state.origin.uuid.as_str()),
            Some(UUID)
        );
    }

    #[test]
    fn a_wait_on_a_filed_slug_joins_its_card_and_an_unfiled_slug_gets_one_bare_card() {
        let cards = topic::cards(&[topic::Source {
            session: "seat".to_owned(),
            uuid: "0199c0de-1234-4890-abcd-ef0123456789".to_owned(),
            incarnation: 0,
            memo: b"2026-01-01T00:00:00Z\tlead\ttopic-m\tstate: quiet\n\
                    2026-01-02T00:00:00Z\tlead\ttopic-z\tstate: moving | title: full z\n"
                .to_vec(),
        }]);
        let waits: BTreeMap<String, Vec<Wait>> = ["a", "b", "z"]
            .into_iter()
            .map(|slug| {
                let wait = Wait {
                    kind: WaitKind::Decide,
                    text: format!("decide #{slug}"),
                    id: None,
                    item: None,
                };
                (slug.to_owned(), vec![wait])
            })
            .collect();
        let groups = arrange(cards, &waits);
        let words: Vec<(Group, Vec<&str>)> = groups
            .iter()
            .map(|(group, cards)| {
                (
                    *group,
                    cards.iter().map(|card| card.slug.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            words,
            [
                (Group::NeedsYou, vec!["a", "b", "z"]),
                (Group::Moving, vec![]),
                (Group::Quiet, vec!["m"]),
                (Group::Parked, vec![]),
                (Group::Closed, vec![]),
                (Group::Unreadable, vec![]),
            ]
        );
        let needs = &groups[0].1;
        assert!(needs[0].title.is_none() && needs[0].reviewed.is_empty());
        assert_eq!(
            needs[2].title.as_ref().map(|title| title.value.as_str()),
            Some("full z")
        );
    }

    const FIRST: &str = "0199c0de-5551-4890-abcd-ef0123456789";
    const SECOND: &str = "0199c0de-5552-4890-abcd-ef0123456789";

    /// The orchestrator seat `name` of incarnation `uuid` under `root`, with an
    /// empty journal; its directory.
    fn seat(root: &std::path::Path, name: &str, uuid: &str) -> std::path::PathBuf {
        write(
            &root.join("orchestrator.config"),
            "[workspace]\norchestrator = true\n",
        );
        let dir = root.join("sessions").join(name);
        std::fs::create_dir_all(dir.join("messages"))
            .unwrap_or_else(|error| panic!("session: {error}"));
        write(
            &dir.join("meta"),
            format!(
                "schema=2\nsession={name}\nsession_id={uuid}\nsession_id_origin=session\n\
                 mode=worktree\nlayout=lead-pair\nseat.main=lead\n\
                 tmux_server_kind=socket\ntmux_server={}\nmeta_agent=true\nlocal_config={}\n",
                root.join("no-server").display(),
                root.join("orchestrator.config").display(),
            ),
        );
        write(&dir.join("events.jsonl"), "");
        dir
    }

    /// The seat `name` of incarnation `uuid` with the one topic `slug`,
    /// published to the archive at `at` as an end publishes it and then
    /// removed; its archive.
    fn published(
        root: &std::path::Path,
        name: &str,
        uuid: &str,
        slug: &str,
        at: &str,
    ) -> std::path::PathBuf {
        let dir = seat(root, name, uuid);
        write(
            &dir.join("memo.tsv"),
            format!("2026-08-01T11:00:00Z\tlead\ttopic-{slug}\tstate: quiet\n"),
        );
        let ops = crate::archive::publish::Ops {
            reason: crate::archive::publish::Reason::End,
            push_outcome: "not-managed",
            push_ref: "-",
            preserved: "-",
            workdir: "-",
            archived_at: at,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = crate::archive::publish::run(&dir, &ops, &mut out, &mut err)
            .unwrap_or_else(|error| panic!("publish: {error}"));
        assert_eq!(code, 0, "publish: {}", String::from_utf8_lossy(&err));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|error| panic!("retire source: {error}"));
        root.join("archive").join(uuid)
    }

    /// The slug of every card in the column.
    fn slugs(column: &Column) -> Vec<&str> {
        column
            .groups
            .iter()
            .flat_map(|(_, cards)| cards)
            .map(|card| card.slug.as_str())
            .collect()
    }

    /// The doc of `archived`: an archive is judged again when its tree
    /// identity moved. An archive whose meta goes away between two polls of
    /// one listener no longer validates, so its topic leaves the column.
    #[test]
    fn a_kept_archive_whose_tree_moves_is_judged_again() {
        let scratch = Scratch::new();
        let root = &scratch.0;
        let archive = published(root, "first-seat", FIRST, "kept", "2026-08-01T12:00:00Z");
        let mut sticky = Sticky::default();
        let before = collect(root, &mut sticky);
        assert_eq!(before.skipped.invalid, 0, "{:?}", before.gaps);
        assert_eq!(slugs(&before), ["kept"], "{:?}", before.skipped);

        std::fs::remove_file(archive.join(crate::meta::FILE))
            .unwrap_or_else(|error| panic!("remove the archive's meta: {error}"));
        let after = collect(root, &mut sticky);
        assert_eq!(after.skipped.invalid, 1, "{:?}", after.gaps);
        assert!(slugs(&after).is_empty(), "{:?}", slugs(&after));
    }

    /// The doc of `archived`: a kept archive is judged once per tree identity,
    /// so a later poll at the same identity reads metadata only. A content
    /// read that fails on such a poll (a transient I/O error) cannot reach
    /// the verdict: the archive keeps its card and the column its skipped
    /// counts. A test cannot fail a read without moving the tree's lstat
    /// facts, so it fails the meta read by removing the meta and then holds
    /// the verdict at the moved identity: the state a transient failure at
    /// an unchanged identity leaves.
    #[test]
    fn a_kept_archive_at_its_held_identity_keeps_its_verdict() {
        let scratch = Scratch::new();
        let root = &scratch.0;
        let archive = published(root, "first-seat", FIRST, "kept", "2026-08-01T12:00:00Z");
        let mut sticky = Sticky::default();
        let before = collect(root, &mut sticky);
        assert_eq!(before.skipped.invalid, 0, "{:?}", before.gaps);
        assert_eq!(slugs(&before), ["kept"], "{:?}", before.skipped);

        std::fs::remove_file(archive.join(crate::meta::FILE))
            .unwrap_or_else(|error| panic!("remove the archive's meta: {error}"));
        let unchanged = crate::archive::tree_identity(&archive)
            .unwrap_or_else(|error| panic!("tree identity: {error}"));
        let held = sticky
            .judged
            .get_mut(FIRST)
            .unwrap_or_else(|| panic!("no verdict held for {FIRST}"));
        held.0 = unchanged;
        let after = collect(root, &mut sticky);
        assert_eq!(after, before);
    }

    /// The verdicts live as long as the listener, so they are held for the
    /// archives kept now and for no other: the memory of a long-running
    /// listener is bound by the archives it keeps, not by every archive it
    /// ever saw.
    #[test]
    fn the_verdicts_held_are_those_of_the_kept_archives() {
        let scratch = Scratch::new();
        let root = &scratch.0;
        let gone = published(root, "first-seat", FIRST, "first", "2026-08-01T12:00:00Z");
        published(
            root,
            "second-seat",
            SECOND,
            "second",
            "2026-08-01T13:00:00Z",
        );
        let held = |sticky: &Sticky| sticky.judged.keys().cloned().collect::<Vec<_>>();
        let mut sticky = Sticky::default();
        let both = collect(root, &mut sticky);
        assert_eq!(slugs(&both), ["first", "second"], "{:?}", both.skipped);
        assert_eq!(held(&sticky), [FIRST, SECOND]);

        std::fs::remove_dir_all(&gone)
            .unwrap_or_else(|error| panic!("remove the first archive: {error}"));
        let one = collect(root, &mut sticky);
        assert_eq!(slugs(&one), ["second"], "{:?}", one.skipped);
        assert_eq!(held(&sticky), [SECOND]);
    }

    /// Each open ask waits by its own lifetime: in a seat retired with one ask
    /// open that took a new ask after the retire, the retired ask waits no
    /// more and the new one waits.
    #[test]
    fn a_retired_ask_and_a_later_live_ask_each_keep_their_own_lifetime() {
        const RETIRED: &str = "ae-20261016T090000Z-00000001";
        const LIVE: &str = "ae-20261016T090200Z-00000002";
        let scratch = Scratch::new();
        let root = &scratch.0;
        let dir = seat(root, "owner-seat", FIRST);
        let store = crate::store::open(&dir);
        let append = |fields: &[(&str, &str)]| {
            let event = Value::obj(fields.iter().map(|(key, value)| (*key, Value::str(*value))));
            store
                .append_event(&format!("{}\n", event.render()))
                .unwrap_or_else(|error| panic!("append: {error}"));
        };
        let ask = |ts: &str, id: &str, summary: &str| {
            append(&[
                ("ts", ts),
                ("actor", crate::tracked::CONSOLE_SINK),
                ("action", "ask"),
                ("target", "lead"),
                ("target_slot", "main"),
                ("target_session", "owner-seat"),
                ("ref", id),
                ("summary", summary),
            ]);
        };
        ask("2026-10-16T09:00:00Z", RETIRED, "#first-ask waits");
        append(&[
            ("ts", "2026-10-16T09:01:00Z"),
            ("actor", "lead"),
            ("action", "retire"),
            ("target", "lead"),
            ("target_slot", "main"),
            ("target_session", "owner-seat"),
            ("target_session_id", FIRST),
        ]);
        ask("2026-10-16T09:02:00Z", LIVE, "#second-ask waits");

        let column = collect(root, &mut Sticky::default());
        let waiting = |slug: &str| {
            column
                .waits
                .get(slug)
                .into_iter()
                .flatten()
                .filter_map(|wait| wait.id.as_deref())
                .collect::<Vec<_>>()
        };
        assert_eq!(waiting("second-ask"), [LIVE], "{:?}", column.waits);
        assert!(waiting("first-ask").is_empty(), "{:?}", column.waits);
    }
}
