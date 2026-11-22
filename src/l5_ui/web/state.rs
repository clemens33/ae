//! What `/api/state` shows: the orchestrator's standing and its chat, read
//! through the same staged console read as the app's panel, rendered at most
//! once per second into one snapshot every page shares.
//!
//! The render is the outbound boundary: whatever a [`Source`] returns, the
//! document carries at most [`chat::ROWS_MAX`] rows and each text within its
//! cap, control characters removed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::chat::{self, Chat};
use super::clock::Clock;
use crate::console::{Console, Reach};
use crate::events::Held;
use crate::session::RecordSnapshot;

/// The most stages one read drives a fresh console through: the journal, the
/// current conversations, their predecessors and the follow.
const STAGES_MAX: usize = 8;

/// Where the state document comes from.
pub trait Source: Send + Sync {
    /// The orchestrator and its chat as they are now.
    fn read(&self) -> Read;
}

/// One read of the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Read {
    /// Who holds the orchestrator role and whether it is proven live.
    pub orchestrator: Orchestrator,
    /// Its chat.
    pub chat: Chat,
}

/// The orchestrator seat as one read found it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Orchestrator {
    /// The session holding the role, when one does.
    pub name: Option<String>,
    /// Whether it is proven live.
    pub state: Standing,
    /// Why it is not proven, empty when it is.
    pub why: String,
}

/// The orchestrator's standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Standing {
    /// One session holds the role and its tmux session proves its identity.
    Proven,
    /// One session holds the role but is not proven live.
    Stopped,
    /// No session holds the role.
    #[default]
    Absent,
    /// The role or its session could not be read.
    Gap,
}

impl Standing {
    /// The word the state document carries.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Proven => "proven",
            Self::Stopped => "stopped",
            Self::Absent => "absent",
            Self::Gap => "gap",
        }
    }
}

/// One rendered state document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The JSON bytes.
    pub body: Arc<[u8]>,
    /// Its entity tag, quotes included.
    pub etag: String,
    /// The clock second of the read it renders.
    pub read_at: i64,
}

/// How often the source may be read: once per this span of the monotonic
/// clock.
const CADENCE: Duration = Duration::from_secs(1);

/// The one snapshot every page shares: the source is read at most once per
/// [`CADENCE`] of the monotonic clock, and a read equal to the last keeps its
/// snapshot whole, so an unchanged poll keeps its entity tag.
#[derive(Default)]
pub struct Cache {
    held: Mutex<Option<Kept>>,
}

struct Kept {
    /// When the source was last read, on the monotonic clock.
    at: Instant,
    read: Read,
    snapshot: Arc<Snapshot>,
}

impl Cache {
    /// The kept snapshot while its read is younger than [`CADENCE`], else a
    /// fresh read's. The time is sampled under the lock, so a worker that
    /// waited for it never reads again after a newer snapshot; the wall clock
    /// only stamps `read_at`.
    pub fn get(&self, clock: &dyn Clock, source: &dyn Source, version: &str) -> Arc<Snapshot> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let now = clock.now();
        if let Some(kept) = held.as_ref()
            && now.saturating_duration_since(kept.at) < CADENCE
        {
            return Arc::clone(&kept.snapshot);
        }
        let read_at = clock.epoch();
        let read = source.read();
        if let Some(kept) = held.as_mut()
            && kept.read == read
        {
            kept.at = now;
            return Arc::clone(&kept.snapshot);
        }
        let snapshot = Arc::new(render(&read, version, read_at));
        *held = Some(Kept {
            at: now,
            read,
            snapshot: Arc::clone(&snapshot),
        });
        snapshot
    }
}

/// The state document of `read`, bounded at this boundary: the newest
/// [`chat::ROWS_MAX`] rows, the rest counted in `cut`, every text made plain
/// and capped. Written in one pass, because a legitimate snapshot reaches tens
/// of MiB and is rendered while a page waits for its first byte.
#[must_use]
pub fn render(read: &Read, version: &str, read_at: i64) -> Snapshot {
    use std::fmt::Write as _;

    let seat = &read.orchestrator;
    let chat = &read.chat;
    let over = chat.rows.len().saturating_sub(chat::ROWS_MAX);
    let rows = chat.rows.get(over..).unwrap_or_default();
    // A tab or a quote doubles on the wire; reserve for the worst case.
    let texts: usize = rows
        .iter()
        .map(|row| row.body.len().min(chat::BODY_MAX))
        .sum();
    let mut out = String::with_capacity(texts.saturating_mul(2).saturating_add(4096));
    out.push_str("{\"v\":1,\"version\":");
    string(&mut out, &chat::words(version));
    let _ = write!(out, ",\"read_at\":{read_at},\"orchestrator\":{{\"name\":");
    match seat.name.as_deref() {
        Some(name) => string(&mut out, &chat::words(name)),
        None => out.push_str("null"),
    }
    out.push_str(",\"state\":");
    string(&mut out, seat.state.word());
    out.push_str(",\"why\":");
    string(&mut out, &chat::plain(&seat.why, chat::BODY_MAX));
    out.push_str("},\"chat\":{\"rows\":[");
    for (at, row) in rows.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"key\":\"{:016x}\",\"micros\":{}",
            row.key, row.micros
        );
        for (name, text) in [
            ("kind", &row.kind),
            ("who", &row.who),
            ("what", &row.what),
            ("voice", &row.voice),
        ] {
            let _ = write!(out, ",\"{name}\":");
            string(&mut out, &chat::words(text));
        }
        out.push_str(",\"body\":");
        string(&mut out, &chat::plain(&row.body, chat::BODY_MAX));
        out.push('}');
    }
    out.push_str("],\"coverage\":[");
    for (at, gap) in chat.coverage.iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        string(&mut out, &chat::plain(gap, chat::BODY_MAX));
    }
    let cut = chat.cut.saturating_add(over);
    let _ = write!(out, "],\"cut\":{cut}}},\"desk\":null,\"route\":null}}");
    let body: Arc<[u8]> = out.into_bytes().into();
    let digest = crate::install::sha256_hex(&body);
    let etag = format!("\"{}\"", digest.get(..16).unwrap_or_default());
    Snapshot {
        body,
        etag,
        read_at,
    }
}

/// `text` as a JSON string: quote, backslash and every ASCII control escaped.
/// An indexed loop, not an iterator chain, so an unoptimized build escapes a
/// full chat within the first-byte budget: the runs that need no escape are
/// copied whole, and a run of one escaped byte is written by doubling.
fn string(out: &mut String, text: &str) {
    out.push('"');
    let bytes = text.as_bytes();
    let (mut run, mut at) = (0, 0);
    while at < bytes.len() {
        let byte = bytes[at];
        if byte >= 0x20 && byte != b'"' && byte != b'\\' && byte != 0x7f {
            at += 1;
            continue;
        }
        out.push_str(text.get(run..at).unwrap_or_default());
        let mut end = at + 1;
        while end < bytes.len() && bytes[end] == byte {
            end += 1;
        }
        escapes(out, byte, end - at);
        run = end;
        at = end;
    }
    out.push_str(text.get(run..).unwrap_or_default());
    out.push('"');
}

/// `count` copies of the JSON escape of ASCII byte `byte`, the first written
/// once and the rest copied from it in doubling steps.
fn escapes(out: &mut String, byte: u8, count: usize) {
    use std::fmt::Write as _;

    let start = out.len();
    match byte {
        b'"' => out.push_str("\\\""),
        b'\\' => out.push_str("\\\\"),
        b'\n' => out.push_str("\\n"),
        b'\t' => out.push_str("\\t"),
        b'\r' => out.push_str("\\r"),
        _ => {
            let _ = write!(out, "\\u{byte:04x}");
        }
    }
    let total = (out.len() - start) * count;
    while out.len() - start < total {
        let have = out.len() - start;
        out.extend_from_within(start..start + have.min(total - have));
    }
}

/// The live orchestrator under one state root, read the way the app's panel
/// reads it: one standing console per incarnation, driven through every
/// stage, the journal's proven prefix kept between reads.
pub struct LiveSource {
    root: PathBuf,
    global: PathBuf,
    followed: Mutex<Option<Followed>>,
}

struct Followed {
    name: String,
    dir: PathBuf,
    uuid: Option<String>,
    console: Console,
    records: Held,
}

impl LiveSource {
    /// The source for state root `root` whose global config is `global`.
    #[must_use]
    pub fn new(root: PathBuf, global: PathBuf) -> Self {
        Self {
            root,
            global,
            followed: Mutex::new(None),
        }
    }
}

impl Source for LiveSource {
    fn read(&self) -> Read {
        let (orchestrator, found) = orchestrator(&self.root);
        let mut followed = self.followed.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((name, dir, uuid)) = found else {
            *followed = None;
            return Read {
                orchestrator,
                chat: Chat::default(),
            };
        };
        // Any change of name, place or incarnation opens a fresh console; an
        // unknown incarnation never matches a known one.
        let stale = followed
            .as_ref()
            .is_none_or(|kept| kept.name != name || kept.dir != dir || kept.uuid != uuid);
        if stale {
            *followed = Some(Followed {
                console: Console::open_standing(name.clone(), dir.clone()),
                name,
                dir,
                uuid,
                records: Held::default(),
            });
        }
        let Some(kept) = followed.as_mut() else {
            return Read {
                orchestrator,
                chat: Chat::default(),
            };
        };
        Read {
            orchestrator,
            chat: kept.chat(&self.global),
        }
    }
}

impl Followed {
    /// The chat, the console driven through every stage it has left.
    fn chat(&mut self, global: &Path) -> Chat {
        let records = &mut self.records;
        let mut snapshot = |dir: &Path| RecordSnapshot::read_with(dir, || records.session(dir));
        let mut read =
            crate::app::read_lane(&mut self.console, Reach::Every, global, &mut snapshot);
        for _ in 1..STAGES_MAX {
            if !read.read || !self.console.staging(Reach::Every) {
                break;
            }
            read = crate::app::read_lane(&mut self.console, Reach::Every, global, &mut snapshot);
        }
        let main = self.console.seats().ok().and_then(|seats| {
            let named = seats.iter().find(|seat| seat.slot == "main");
            named
                .or_else(|| seats.first())
                .map(|seat| seat.name.clone())
        });
        chat::rows(&read.lane, main.as_deref().unwrap_or_default())
    }
}

/// The orchestrator's standing under `root`, and the session to read when
/// one holds the role: its name, its directory and the incarnation to follow
/// (proven live, or recorded by a meta that still proves the seat).
fn orchestrator(root: &Path) -> (Orchestrator, Option<(String, PathBuf, Option<String>)>) {
    let name = match crate::settings_menu::role_holder(root) {
        Ok(Some(name)) => name,
        Ok(None) => return (Orchestrator::default(), None),
        Err(why) => {
            let seat = Orchestrator {
                name: None,
                state: Standing::Gap,
                why,
            };
            return (seat, None);
        }
    };
    let Some(dir) = crate::console::locate(root, &name) else {
        let seat = Orchestrator {
            name: Some(name),
            state: Standing::Gap,
            why: "no session directory holds the role's session".to_owned(),
        };
        return (seat, None);
    };
    match crate::console::target::prove_role(root, &name, &dir) {
        Ok((uuid, _)) => {
            let seat = Orchestrator {
                name: Some(name.clone()),
                state: Standing::Proven,
                why: String::new(),
            };
            (seat, Some((name, dir, Some(uuid))))
        }
        Err(why) => {
            // Not proven live: the directory still has to prove the seat by
            // its own meta, read again now, and that meta's incarnation is the
            // one to follow, so a stopped seat replaced in place is followed.
            let meta = crate::meta::read_bytes(&dir)
                .ok()
                .filter(|meta| crate::meta::is_seat(meta, root));
            let Some(meta) = meta else {
                let seat = Orchestrator {
                    name: Some(name),
                    state: Standing::Gap,
                    why: "the role's session no longer proves the orchestrator seat".to_owned(),
                };
                return (seat, None);
            };
            let uuid =
                crate::archive::canonical_uuid(&crate::lifecycle::meta_value(&meta, "session_id"));
            let seat = Orchestrator {
                name: Some(name.clone()),
                state: Standing::Stopped,
                why,
            };
            (seat, Some((name, dir, (!uuid.is_empty()).then_some(uuid))))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::{Cache, LiveSource, Read, Source, Standing, string};
    use crate::web::clock::Clock;

    /// One monotonic instant for every thread; the wall second a thread
    /// named `late` reads is one behind every other thread's, and its first
    /// wall reading is reported.
    struct Split {
        at: Instant,
        sampled: Mutex<Option<Sender<()>>>,
    }

    impl Clock for Split {
        fn now(&self) -> Instant {
            self.at
        }

        fn epoch(&self) -> i64 {
            if std::thread::current().name() != Some("late") {
                return 101;
            }
            let taken = self
                .sampled
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(sampled) = taken {
                let _ = sampled.send(());
            }
            100
        }
    }

    /// A source whose first read waits for its gate.
    struct Gated {
        reads: AtomicUsize,
        gate: Mutex<Option<Receiver<()>>>,
    }

    impl Source for Gated {
        fn read(&self) -> Read {
            if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                let gate = self
                    .gate
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(gate) = gate {
                    let _ = gate.recv();
                }
            }
            Read::default()
        }
    }

    #[test]
    fn a_worker_that_waited_for_the_cache_never_reads_again_within_the_second() {
        let (sampled_tx, sampled) = mpsc::channel();
        let (open, gate) = mpsc::channel::<()>();
        let clock = Arc::new(Split {
            at: Instant::now(),
            sampled: Mutex::new(Some(sampled_tx)),
        });
        let source = Arc::new(Gated {
            reads: AtomicUsize::new(0),
            gate: Mutex::new(Some(gate)),
        });
        let cache = Arc::new(Cache::default());
        let first = {
            let (cache, clock, source) = (cache.clone(), clock.clone(), source.clone());
            std::thread::spawn(move || cache.get(&*clock, &*source, "v"))
        };
        let started = Instant::now();
        while source.reads.load(Ordering::SeqCst) == 0 && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::yield_now();
        }
        let late = {
            let (cache, clock, source) = (cache.clone(), clock.clone(), source.clone());
            std::thread::Builder::new()
                .name("late".to_owned())
                .spawn(move || cache.get(&*clock, &*source, "v"))
                .unwrap_or_else(|error| panic!("spawn: {error}"))
        };
        // A reader that samples its second before the lock does so now.
        let _ = sampled.recv_timeout(Duration::from_millis(300));
        drop(open);
        let first = first.join().unwrap_or_else(|_| panic!("first reader"));
        let late = late.join().unwrap_or_else(|_| panic!("late reader"));
        assert_eq!(source.reads.load(Ordering::SeqCst), 1);
        assert_eq!(first.read_at, 101);
        assert_eq!(late.read_at, 101);
    }

    fn escaped(text: &str) -> String {
        let mut out = String::new();
        string(&mut out, text);
        out
    }

    #[test]
    fn runs_of_one_escape_and_mixed_escapes_write_every_byte() {
        assert_eq!(escaped(""), r#""""#);
        assert_eq!(escaped("plain é"), r#""plain é""#);
        assert_eq!(escaped("\t"), r#""\t""#);
        assert_eq!(escaped(&"\t".repeat(7)), format!("\"{}\"", r"\t".repeat(7)));
        assert_eq!(
            escaped("a\t\t\tb\"\"\\\u{1}\u{1}\u{7f}\r\nz"),
            r#""a\t\t\tb\"\"\\\u0001\u0001\u007f\r\nz""#
        );
        assert_eq!(escaped("\n\t\n\t"), r#""\n\t\n\t""#);
    }

    /// A scratch state root removed when it drops.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
            let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
            let root =
                std::env::temp_dir().join(format!("ae-web-live-{}-{sequence}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap_or_else(|error| panic!("scratch: {error}"));
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn seat_meta(root: &std::path::Path, dir: &std::path::Path, uuid: &str) {
        let meta = format!(
            "schema=2\nsession=stopped-seat\nsession_id={uuid}\nlayout=lead-pair\nseat.main=lead\n\
             meta_agent=true\nlocal_config={}\ntmux_server_kind=socket\ntmux_server={}\n",
            root.join("orchestrator.config").display(),
            root.join("no-server").display(),
        );
        std::fs::write(dir.join("meta"), meta).unwrap_or_else(|error| panic!("meta: {error}"));
    }

    fn replaced(read: &Read) -> bool {
        read.chat
            .coverage
            .iter()
            .any(|line| line.contains("replaced or renamed"))
    }

    #[test]
    fn a_stopped_role_replaced_in_place_is_followed_to_its_new_incarnation() {
        let scratch = Scratch::new();
        let root = scratch.0.clone();
        std::fs::write(
            root.join("orchestrator.config"),
            "[workspace]\norchestrator = true\n",
        )
        .unwrap_or_else(|error| panic!("overlay: {error}"));
        let dir = root.join("sessions").join("stopped-seat");
        std::fs::create_dir_all(dir.join("messages"))
            .unwrap_or_else(|error| panic!("session: {error}"));
        std::fs::write(dir.join("events.jsonl"), "")
            .unwrap_or_else(|error| panic!("journal: {error}"));
        seat_meta(&root, &dir, "0199c0de-1111-4890-abcd-ef0123456789");
        let source = LiveSource::new(root.clone(), root.join("config"));
        let first = source.read();
        assert_eq!(first.orchestrator.state, Standing::Stopped, "{first:?}");
        assert!(!replaced(&first), "{:?}", first.chat.coverage);

        seat_meta(&root, &dir, "0199c0de-2222-4890-abcd-ef0123456789");
        let second = source.read();
        assert_eq!(second.orchestrator.state, Standing::Stopped, "{second:?}");
        assert!(!replaced(&second), "{:?}", second.chat.coverage);
    }
}
