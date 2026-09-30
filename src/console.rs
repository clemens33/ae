//! `ae console` — the human's lane of a session, as a program.
//!
//! Phase 1 is a READ view of data ae already keeps: nothing here submits,
//! answers or writes an event.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::board::{self, Inputs, Row, follow::Follow, terminal_text};
use crate::events::Event;
use crate::store::{self, Oversized, SourceRead};
use crate::{archive, doors, inventory, lifecycle, meta, reply, session, usage, watchdog_daemon};
use lane::Seat;

pub mod lane;
pub(crate) mod toggle;
pub mod view;

/// The usage text.
pub const USAGE: &str = "Usage: ae console [session] [--follow] [--all]\n\n  session   the session to read (default: the session this pane belongs to)\n  --follow  keep printing what is new every 5 s until interrupted (Ctrl-C to stop)\n  --all     add the lead pair's assistant replies (off by default)\n";

/// A parsed `ae console` argv.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub session: Option<String>,
    pub follow: bool,
    pub all: bool,
}

/// The argv did not parse; the offending token, when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub Option<String>);

impl Usage {
    /// The stderr text, offending token first when one was named.
    #[must_use]
    pub fn render(&self) -> String {
        match &self.0 {
            Some(token) => format!("ae console: unexpected {token}\n{USAGE}"),
            None => USAGE.to_owned(),
        }
    }
}

/// Read `tail` — everything after the word `console`.
///
/// # Errors
///
/// [`Usage`] for an unknown flag or a second session name.
pub fn parse(tail: &[String]) -> Result<Args, Usage> {
    let mut args = Args::default();
    for token in tail {
        match token.as_str() {
            "--follow" => args.follow = true,
            "--all" => args.all = true,
            flag if flag.starts_with('-') => return Err(Usage(Some(token.clone()))),
            name if args.session.is_none() => args.session = Some(name.to_owned()),
            _ => return Err(Usage(Some(token.clone()))),
        }
    }
    Ok(args)
}

/// One console: the session it is bound to and what it has printed of it.
struct Console {
    name: String,
    dir: PathBuf,
    uuid: String,
    assistant: bool,
    home: Option<PathBuf>,
    printed: view::Printed,
    rows: Vec<Row>,
    follow: Option<Follow>,
    /// The last journal read whole, which every later read must begin with.
    journal: Option<Vec<Event>>,
}

impl Console {
    /// The session's identity and lead-pair seats, read from its meta NOW:
    /// `Err` says why this console no longer follows the session it opened.
    fn seats(&self) -> Result<Vec<Seat>, String> {
        let bytes =
            meta::read_bytes(&self.dir).map_err(|err| format!("meta unreadable ({err})"))?;
        let uuid = archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id"));
        if uuid.is_empty() {
            return Err("session_id is missing or malformed, so it cannot be bound".to_owned());
        }
        if uuid != self.uuid {
            return Err("the session was replaced or renamed".to_owned());
        }
        let lead_pair = lifecycle::meta_value(&bytes, "layout") == "lead-pair";
        let meta = meta::Meta::parse(&String::from_utf8_lossy(&bytes));
        Ok(meta
            .roster()
            .iter()
            .filter(|entry| watchdog_daemon::in_lead_pair(&entry.slot, lead_pair))
            .map(|entry| Seat {
                slot: entry.slot.clone(),
                name: entry.name.clone(),
            })
            .collect())
    }

    /// One pass: the first reads everything, each later one asks the board's
    /// follow for the new transcript bytes and re-reads the whole journal, so a
    /// replaced or shrunk journal can never hide a record. The text is what
    /// this console has not printed yet.
    fn pass(&mut self) -> Result<String, String> {
        let seats = self.seats()?;
        let slots: Vec<&str> = seats.iter().map(|seat| seat.slot.as_str()).collect();
        let keep = |entry: &meta::RosterEntry| slots.contains(&entry.slot.as_str());
        let sessions = [usage::SessionInput {
            name: self.name.clone(),
            path: self.dir.clone(),
        }];
        let inputs = Inputs {
            home: self.home.as_deref(),
            sessions: &sessions,
            assistant: self.assistant,
        };
        let seen = match self.follow.as_mut() {
            None => {
                let first = board::observe_selected(&inputs, None, None, &keep);
                self.follow = Some(Follow::seeded(&first.seeds, &first.coverage, None));
                first
            }
            Some(follow) => board::follow_poll_selected(&inputs, follow, &keep),
        };
        // A rescan re-reads a rewritten transcript, so a record at the same file
        // and offset is the NEW words: the shared collect keeps the first.
        let fresh: BTreeSet<(&str, u64)> = seen
            .rows
            .iter()
            .map(|row| (row.file.as_str(), row.offset))
            .collect();
        self.rows
            .retain(|row| !fresh.contains(&(row.file.as_str(), row.offset)));
        self.rows.extend(seen.rows.iter().cloned());
        self.rows = board::collect(std::mem::take(&mut self.rows));
        let board_gaps = seen.coverage.len();
        let observation = board::Observation {
            rows: self.rows.clone(),
            coverage: seen.coverage,
            ..board::Observation::default()
        };
        let (events, skipped, read_gap) = match session::RecordSnapshot::read(&self.dir).events {
            Some(read) => (read.events, read.skipped.len(), None),
            None => (Vec::new(), 0, Some("journal — unreadable".to_owned())),
        };
        // A journal no longer beginning with the last read was rewritten (resume
        // trims its head): positions name other records, so the thread reprints.
        let mut text = String::new();
        if let (None, Some(last)) = (&read_gap, &self.journal)
            && !events.starts_with(last)
        {
            text = self.printed.rebase(last.len(), events.len());
        }
        let body = |event: &Event| body_for(&self.dir, event);
        let mut lane = lane::fold(&self.name, &seats, &events, &observation, skipped, &body);
        let settled = read_gap.is_none();
        lane.coverage.extend(read_gap);
        text.push_str(&self.printed.step(&lane, board_gaps, settled));
        if settled {
            self.journal = Some(events);
        }
        Ok(text)
    }
}

/// A console reply's stored body, read only from THIS session's `messages/`:
/// the recorded path must end in a `messages` directory and a name the reply
/// writer gives this request, and the read takes that name from `dir` itself,
/// never the recorded directory, so no record can point the console elsewhere.
fn body_for(dir: &Path, event: &Event) -> lane::Body {
    const CAP: u64 = reply::CONSOLE_BODY_CAP as u64;
    let Some(recorded) = event.body_file.as_deref().map(Path::new) else {
        return lane::Body::OldCore;
    };
    let id = event.reference.as_deref().unwrap_or("");
    let in_messages = recorded.parent().and_then(Path::file_name) == Some("messages".as_ref());
    let name = recorded.file_name().and_then(|name| name.to_str());
    let Some(name) = name.filter(|name| in_messages && is_reply_body_name(id, name)) else {
        return lane::Body::Refused("not a reply body in this session's messages".to_owned());
    };
    match store::read_capped_in(&dir.join("messages"), name, CAP) {
        Ok(SourceRead::Ready(bytes)) => {
            String::from_utf8(bytes).map_or(lane::Body::NotUtf8, lane::Body::Whole)
        }
        Ok(SourceRead::Absent) => lane::Body::Missing,
        Ok(SourceRead::Invalid(what)) => lane::Body::Refused(what),
        Ok(SourceRead::Unreadable(why)) => lane::Body::Refused(format!("unreadable ({why})")),
        Err(Oversized) => lane::Body::Oversized,
    }
}

/// `<id>.reply.<six lowercase hex>.txt` — the name `deliver::store_body`
/// publishes a reply to request `id` under.
fn is_reply_body_name(id: &str, name: &str) -> bool {
    let hex = name
        .strip_prefix(id)
        .and_then(|rest| rest.strip_prefix(".reply."))
        .and_then(|rest| rest.strip_suffix(".txt"));
    let lower_hex = |byte: u8| matches!(byte, b'0'..=b'9' | b'a'..=b'f');
    !id.is_empty() && hex.is_some_and(|hex| hex.len() == 6 && hex.bytes().all(lower_hex))
}

/// The session directory `name` records, or why there is none.
pub(crate) fn locate(root: &Path, name: &str) -> Option<PathBuf> {
    let roots = inventory::Roots::under(root);
    let scan = inventory::durable_meta_records(&roots);
    scan.records
        .iter()
        .find(|record| record.name == name)
        .map(|record| record.path.clone())
}

/// The canonical `session_id` the session's meta records, or empty.
pub(crate) fn recorded_uuid(dir: &Path) -> String {
    meta::read_bytes(dir)
        .map(|bytes| archive::canonical_uuid(&lifecycle::meta_value(&bytes, "session_id")))
        .unwrap_or_default()
}

/// `ae console [session] [--follow] [--all]`.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
pub fn run(tail: &[String], out: &mut impl Write, err: &mut impl Write) -> crate::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(usage) => {
            write!(err, "{}", terminal_text(&usage.render()))?;
            return Ok(crate::entry::EXIT_USAGE);
        }
    };
    let Some(root) = crate::state_root() else {
        writeln!(err, "ae: {}", crate::NO_STATE_ROOT)?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let Some(name) = args.session.clone().or_else(crate::calling_session_name) else {
        writeln!(
            err,
            "ae console: no session named and this pane is in none\n{USAGE}"
        )?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let Some(dir) = locate(&root, &name) else {
        writeln!(
            err,
            "{}",
            terminal_text(&format!("ae console: no session named {name}"))
        )?;
        return Ok(crate::EXIT_UNAVAILABLE);
    };
    let mut console = Console {
        uuid: recorded_uuid(&dir),
        name,
        dir,
        assistant: args.all,
        home: doors::home(),
        printed: view::Printed::default(),
        rows: Vec::new(),
        follow: None,
        journal: None,
    };
    match console.seats() {
        Ok(seats) => write!(out, "{}", view::header(&console.name, &seats))?,
        Err(why) => {
            writeln!(
                err,
                "{}",
                terminal_text(&format!("ae console: {}: {why}", console.name))
            )?;
            return Ok(crate::EXIT_UNAVAILABLE);
        }
    }
    loop {
        match console.pass() {
            Ok(text) => write!(out, "{text}")?,
            Err(why) => {
                let text = format!(
                    "ae console: {}: {why} — this console no longer follows it; reopen it with prefix h (or ae console <session>)",
                    console.name
                );
                writeln!(err, "{}", terminal_text(&text))?;
                return Ok(crate::EXIT_UNAVAILABLE);
            }
        }
        out.flush()?;
        if !args.follow {
            return Ok(0);
        }
        std::thread::sleep(std::time::Duration::from_secs(board::follow::POLL_SECS));
    }
}

#[cfg(test)]
mod tests {
    use super::{Console, lane::Body, view::Printed};
    use crate::events::Event;
    use std::fs;
    use std::path::PathBuf;

    const ID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
    const REQ: &str = "ae-20260930T060000Z-0000abcd";

    /// A lead-pair session directory under a scratch that removes itself.
    struct Rig(PathBuf);

    impl Rig {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("ae-console-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("s")).expect("a scratch dir");
            fs::write(root.join("s/meta"), format!("schema=2\nsession_id={ID}\nlayout=lead-pair\nseat.main=lead\nagent_bin.main=claude\nharness_session.main={ID}\nconfig_home.main={}\n", root.join("claude").display())).expect("meta");
            Self(root)
        }

        fn console(&self) -> Console {
            Console {
                name: "s".to_owned(),
                dir: self.0.join("s"),
                uuid: ID.to_owned(),
                assistant: false,
                home: Some(self.0.clone()),
                printed: Printed::default(),
                rows: Vec::new(),
                follow: None,
                journal: None,
            }
        }

        /// The lead's one-line transcript, its mtime `secs` after now.
        fn transcript(&self, words: &str, secs: u64) {
            let dir = self.0.join("claude/projects/w");
            fs::create_dir_all(&dir).expect("a project dir");
            let path = dir.join(format!("{ID}.jsonl"));
            let line = format!(
                r#"{{"type":"user","timestamp":"2026-09-30T06:00:00Z","message":{{"role":"user","content":"{words}"}}}}"#
            );
            fs::write(&path, line + "\n").expect("transcript");
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(secs);
            let file = fs::File::options().write(true).open(&path).expect("reopen");
            file.set_modified(later).expect("mtime");
        }

        fn journal(&self, lines: &[&str]) {
            fs::write(self.0.join("s/events.jsonl"), lines.join("\n") + "\n").expect("journal");
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A reply to console request `REQ` naming `path` as its body.
    fn with_body(path: &str) -> Event {
        let line = format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{REQ}","body_file":"{path}"}}"#
        );
        Event::parse_line(&line).expect("a reply record")
    }

    #[test]
    fn a_console_reply_body_is_read_only_from_this_sessions_messages() {
        let rig = Rig::new("bodies");
        let dir = rig.0.join("s");
        let name = |hex: &str| format!("{REQ}.reply.{hex}.txt");
        let messages = dir.join("messages");
        fs::create_dir_all(&messages).expect("messages");
        fs::write(messages.join(name("00000a")), "whole ü\n  indented").expect("a body");
        fs::write(messages.join(name("00000b")), vec![b'x'; 65_537]).expect("a big body");
        fs::write(messages.join(name("00000c")), [0xff, 0xfe]).expect("bytes");
        fs::write(rig.0.join("outside"), "secret").expect("outside");
        std::os::unix::fs::symlink(rig.0.join("outside"), messages.join(name("00000d")))
            .expect("link");
        let body = |path: &str| super::body_for(&dir, &with_body(path));
        let at = |hex: &str| format!("/elsewhere/messages/{}", name(hex));
        // Read from THIS session's messages, never from the recorded directory,
        // wherever it is: a foreign `messages`, or one reached through `..`.
        let foreign = rig.0.join("foreign/messages");
        fs::create_dir_all(&foreign).expect("a foreign messages dir");
        fs::write(foreign.join(name("00000a")), "FOREIGN").expect("a foreign body");
        let whole = Body::Whole("whole ü\n  indented".to_owned());
        let foreign = format!("{}/{}", foreign.display(), name("00000a"));
        for path in [
            at("00000a"),
            foreign,
            format!("/x/../y/messages/{}", name("00000a")),
        ] {
            assert_eq!(body(&path), whole, "{path}");
        }
        assert_eq!(body(&at("00000b")), Body::Oversized);
        assert_eq!(body(&at("00000c")), Body::NotUtf8);
        assert_eq!(body(&at("00000d")), Body::Refused("a symlink".to_owned()));
        assert_eq!(body(&at("00000e")), Body::Missing);
        let refused = Body::Refused("not a reply body in this session's messages".to_owned());
        for path in [
            format!("{}/../outside", messages.display()),
            format!("/x/messages/../{}", name("00000a")),
            format!("/x/other/{}", name("00000a")),
            format!("/x/messages/{REQ}.ask.00000a.txt"),
            format!("/x/messages/{REQ}.reply.00000A.txt"),
            "/x/messages/other.reply.00000a.txt".to_owned(),
        ] {
            assert_eq!(body(&path), refused, "{path}");
        }
        fs::rename(&messages, dir.join("real")).expect("move");
        std::os::unix::fs::symlink(dir.join("real"), &messages).expect("linked messages");
        let linked = Body::Refused("its directory is a symlink".to_owned());
        assert_eq!(body(&at("00000a")), linked);
        let old = r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","ref":"r"}"#;
        let old = Event::parse_line(old).expect("an old-core reply");
        assert_eq!(super::body_for(&dir, &old), Body::OldCore);
    }

    #[test]
    fn a_pass_shows_an_admitted_answer_whole_from_its_stored_body() {
        let rig = Rig::new("answer");
        let file = rig.0.join(format!("s/messages/{REQ}.reply.0000aa.txt"));
        fs::create_dir_all(rig.0.join("s/messages")).expect("messages");
        fs::write(&file, "line one\nline two").expect("a body");
        let ask = format!(
            r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{REQ}","target_slot":"main","target_session":"s","target_server":"/t","target_pane":"%1","target_session_uuid":"{ID}","summary":"q"}}"#
        );
        let answer = format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{REQ}","actor_slot":"main","actor_session":"s","caller_server":"/t","caller_pane":"%1","caller_session_uuid":"{ID}","body_file":"{}","summary":"line one line two"}}"#,
            file.display()
        );
        rig.journal(&[&ask, &answer]);
        let shown = rig.console().pass().expect("a pass");
        let whole = format!("lead answers {REQ}\n  line one\n  line two");
        assert!(shown.contains(&whole), "{shown}");
    }

    const ASK: &str = r#"{"ts":"2026-09-30T06:00:00Z","actor":"lead","action":"state","ref":"waiting-user","summary":"ship it?"}"#;
    const SAY_A: &str =
        r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"chat","summary":"aaaa"}"#;
    const SAY_B: &str =
        r#"{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"chat","summary":"bbbb"}"#;
    const WORK: &str = r#"{"ts":"2026-09-30T06:02:00Z","actor":"lead","action":"state","ref":"working","summary":"go"}"#;

    #[test]
    fn a_replaced_same_size_journal_shows_its_new_record_and_a_cleared_card_closes() {
        let rig = Rig::new("journal");
        let mut console = rig.console();
        rig.journal(&[ASK, SAY_A]);
        let first = console.pass().expect("first pass");
        assert!(
            first.contains("DECISION lead") && first.contains("aaaa"),
            "{first}"
        );
        assert_eq!(console.pass().expect("quiet"), "");
        assert_eq!(SAY_A.len(), SAY_B.len());
        rig.journal(&[ASK, SAY_B]);
        let swapped = console.pass().expect("replaced");
        assert!(
            swapped.contains("bbbb") && !swapped.contains("aaaa"),
            "{swapped}"
        );
        rig.journal(&[ASK, SAY_B, WORK]);
        let closed = console.pass().expect("answered");
        assert!(
            closed.contains("-- closed: DECISION lead · waiting-user"),
            "{closed}"
        );
    }

    #[test]
    fn a_same_size_rewrite_of_a_transcript_prints_its_new_words_on_the_next_pass() {
        let rig = Rig::new("rewrite");
        let mut console = rig.console();
        rig.transcript("aaaa", 0);
        let first = console.pass().expect("first pass");
        assert!(first.contains("aaaa"), "{first}");
        rig.transcript("bbbb", 2);
        let second = console.pass().expect("rewritten");
        assert!(
            second.contains("bbbb") && second.contains("rescanned"),
            "{second}"
        );
    }

    /// Console ask `n`: its own request id and words.
    fn asked(n: u8, words: &str) -> String {
        format!(
            r#"{{"ts":"2026-09-30T06:0{n}:00Z","actor":"console:local","action":"ask","target":"lead","ref":"ae-20260930T06000{n}Z-0000abcd","summary":"{words}"}}"#
        )
    }

    const REWRITTEN: &str = "-- journal rewritten";

    #[test]
    fn a_journal_trimmed_at_its_head_is_named_and_hides_no_newer_console_row() {
        let (rig, three) = (Rig::new("trimmed"), asked(3, "charlie"));
        let mut console = rig.console();
        rig.journal(&[&asked(1, "alpha"), &asked(2, "bravo"), &three]);
        assert!(console.pass().expect("first").contains("charlie"));
        rig.journal(&[&three, &asked(4, "delta")]);
        let got = console.pass().expect("trimmed");
        let notice = format!("{REWRITTEN} (3 records before, 2 now)");
        assert!(got.contains(&notice) && got.contains("delta"), "{got}");
        assert!(!got.contains("alpha"), "{got}");
        let quiet = console.pass().expect("unchanged");
        assert_eq!(quiet, "", "one notice per rewrite");
    }

    #[test]
    fn a_rewritten_head_is_named_even_across_an_unreadable_read() {
        let (rig, two) = (Rig::new("head"), asked(2, "bravo"));
        let mut console = rig.console();
        rig.journal(&[&asked(1, "alpha"), &two]);
        assert!(console.pass().expect("first").contains("alpha"));
        rig.journal(&[&asked(5, "echo"), &two]);
        let got = console.pass().expect("swapped");
        assert!(got.contains(REWRITTEN) && got.contains("echo"), "{got}");
        let path = rig.0.join("s/events.jsonl");
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("a directory where the journal was");
        assert!(!console.pass().expect("unreadable").contains(REWRITTEN));
        fs::remove_dir(&path).expect("restore");
        rig.journal(&[&asked(6, "foxtrot"), &two]);
        let got = console.pass().expect("rewritten while unread");
        assert!(got.contains(REWRITTEN) && got.contains("foxtrot"), "{got}");
    }

    #[test]
    fn a_transiently_unreadable_journal_neither_closes_a_card_nor_reprints_it() {
        let rig = Rig::new("unreadable");
        let mut console = rig.console();
        rig.journal(&[ASK]);
        assert!(console.pass().expect("first").contains("DECISION lead"));
        let path = rig.0.join("s/events.jsonl");
        fs::remove_file(&path).expect("remove");
        fs::create_dir(&path).expect("a directory where the journal was");
        let blind = console.pass().expect("unreadable");
        assert!(
            blind.contains("journal — unreadable") && !blind.contains("closed"),
            "{blind}"
        );
        fs::remove_dir(&path).expect("restore");
        rig.journal(&[ASK]);
        assert_eq!(console.pass().expect("recovered"), "");
    }

    #[test]
    fn a_console_stops_when_its_session_is_replaced_renamed_or_gone() {
        let rig = Rig::new("bound");
        let mut console = rig.console();
        assert!(console.pass().is_ok());
        let other =
            "session_id=0199c0de-bbbb-4890-abcd-ef0123456789\nlayout=lead-pair\nseat.main=lead\n";
        fs::write(rig.0.join("s/meta"), format!("schema=2\n{other}")).expect("replaced");
        assert!(
            console.pass().is_err(),
            "a new session id is not this session"
        );
        fs::remove_dir_all(rig.0.join("s")).expect("gone");
        assert!(
            console.pass().is_err(),
            "a missing directory is not this session"
        );
    }

    /// The whole argv grammar: every flag arm, the one-session guard and the
    /// unknown-flag arm, each with the token a refusal names.
    #[test]
    fn the_argv_grammar_accepts_flags_and_one_session_in_any_order_and_names_a_refusal() {
        let args = |session: Option<&str>, follow, all| {
            Ok(super::Args {
                session: session.map(str::to_owned),
                follow,
                all,
            })
        };
        let refused = |token: &str| Err(super::Usage(Some(token.to_owned())));
        let cases: [(&[&str], Result<super::Args, super::Usage>); 12] = [
            (&[], args(None, false, false)),
            (&["s"], args(Some("s"), false, false)),
            (&["--follow"], args(None, true, false)),
            (&["--all"], args(None, false, true)),
            (&["s", "--follow", "--all"], args(Some("s"), true, true)),
            (&["--all", "--follow", "s"], args(Some("s"), true, true)),
            (&["--follow", "s", "--all"], args(Some("s"), true, true)),
            (&["--frob"], refused("--frob")),
            (&["-"], refused("-")),
            (&["s", "--frob"], refused("--frob")),
            (&["s", "t"], refused("t")),
            (&["--follow", "s", "t", "--all"], refused("t")),
        ];
        for (words, want) in cases {
            let words: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
            assert_eq!(super::parse(&words), want, "{words:?}");
        }
    }
}
