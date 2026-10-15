//! The console's input: which console may take it, and one ask carried to a
//! seat. Each act holds the console admission lock, taken before the
//! journal's, from its ownership read through its read-back.

use std::path::Path;

use super::lane;
use crate::events::Event;
use crate::store::{self, Oversized, SourceRead};
use crate::tmux::WindowPane;
use crate::tracked;

/// The most console asks one session holds open at once.
pub const OPEN_CAP: usize = 5;

/// The first pane stamped with console `uuid` that is `live` (or dead), by
/// window index then pane index — the order the session shows, never tmux's
/// listing order. A live one is the console that owns the session's input.
#[must_use]
pub fn first_console<'a>(
    panes: &'a [WindowPane],
    uuid: &str,
    live: bool,
) -> Option<&'a WindowPane> {
    panes
        .iter()
        .filter(|pane| pane.console.as_deref() == Some(uuid) && pane.dead != live)
        .min_by_key(|pane| (pane.window_index, pane.pane_index))
}

/// Whether the console in pane `me` takes input for the session tmux stamps
/// `stamp` and its meta records as `bound`.
///
/// # Errors
///
/// Why this console is read-only, naming the owner where there is one.
pub fn owner(
    panes: &[WindowPane],
    me: Option<&str>,
    stamp: &str,
    bound: &str,
) -> Result<(), String> {
    let Some(me) = me else {
        return Err("this chat cannot name its own pane, so it takes no input".to_owned());
    };
    if bound.is_empty() || bound != stamp {
        let why =
            "the session id in its meta is not the one tmux carries, so this chat takes no input";
        return Err(why.to_owned());
    }
    match first_console(panes, stamp, true) {
        None => Err("no live chat window holds this session".to_owned()),
        Some(owner) if owner.pane_id == me => Ok(()),
        Some(owner) => Err(format!("input owned by window {}", owner.window_id)),
    }
}

/// What became of one submitted ask, by its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Delivered; the draft is cleared, or why it could not be.
    Sent(String, Option<String>),
    /// Pasted, its submit not confirmed.
    Uncertain(String),
    /// ae gave up delivering it.
    NotDelivered(String),
    /// No record of it, and what was said instead — it may have been delivered.
    Unknown(String, String),
}

/// The console's own record of ask `id` as an outcome; `said` is what the
/// delivery wrote to stderr, kept for when there is none.
#[must_use]
pub fn read_back(events: &[Event], id: &str, said: &str) -> Outcome {
    let own = events.iter().rev().find(|event| {
        let actions = [tracked::Kind::Ask.action(), tracked::ABANDONED_ACTION];
        event.actor == tracked::CONSOLE_SINK
            && event.reference.as_deref() == Some(id)
            && actions.contains(&event.action.as_str())
    });
    let id = id.to_owned();
    match own {
        None => Outcome::Unknown(id, said.trim_end().to_owned()),
        Some(event) if event.action == tracked::ABANDONED_ACTION => Outcome::NotDelivered(id),
        Some(event) if tracked::summary_is_unconfirmed(event.summary.as_deref()) => {
            Outcome::Uncertain(id)
        }
        Some(_) => Outcome::Sent(id, None),
    }
}

/// The kept draft as promotion finds it: never acted on, only shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    Nothing,
    Kept(Vec<u8>),
    /// Why the kept draft is not put back.
    Refused(String),
}

/// The session's kept draft, by the store's bounded read: a plain read, no
/// lock, so a submit still in flight can leave a line that is already sent.
#[must_use]
pub fn restore(dir: &Path) -> Draft {
    let no = |what: String| Draft::Refused(format!("the kept draft {what} and was not restored"));
    match store::open(dir).console_draft() {
        Ok(SourceRead::Absent) => Draft::Nothing,
        Ok(SourceRead::Ready(bytes)) if bytes.is_empty() => Draft::Nothing,
        Ok(SourceRead::Ready(bytes)) => Draft::Kept(bytes),
        Ok(SourceRead::Invalid(what)) => no(format!("is not a regular file ({what})")),
        Ok(SourceRead::Unreadable(why)) => no(format!("could not be read ({why})")),
        Err(Oversized) => no(format!("is over {} bytes", store::CONSOLE_DRAFT_CAP)),
    }
}

/// Carry `raw`, the composer's literal bytes kept as the draft, to a seat as
/// console ask `id`; only [`Outcome::Sent`] clears the draft.
///
/// # Errors
///
/// Why nothing was delivered.
pub fn submit(
    dir: &Path,
    session: &str,
    raw: &[u8],
    id: &str,
    owns: impl FnOnce() -> Result<(), String>,
    deliver: impl FnOnce() -> String,
) -> Result<Outcome, String> {
    submit_held(
        &mut crate::events::Held::default(),
        dir,
        session,
        raw,
        id,
        owns,
        deliver,
    )
}

pub(crate) fn submit_held(
    held: &mut crate::events::Held,
    dir: &Path,
    session: &str,
    raw: &[u8],
    id: &str,
    owns: impl FnOnce() -> Result<(), String>,
    deliver: impl FnOnce() -> String,
) -> Result<Outcome, String> {
    let admitted = admit(held, dir, session, raw, owns)?;
    Ok(admitted.finish(held, id, deliver))
}

/// A submit past its admission: the console admission lock is held, the
/// ownership proven, the draft kept and the open-ask cap passed. Dropping it
/// lets the lock go; only [`Admitted::finish`] carries the ask.
pub(crate) struct Admitted {
    store: store::SessionStore,
    dir: std::path::PathBuf,
    _lock: std::fs::File,
}

/// Everything a submit proves before anything is delivered: the admission
/// lock, `owns`, the kept draft and the open-ask cap, in that order.
///
/// # Errors
///
/// Why nothing may be delivered.
pub(crate) fn admit(
    held: &mut crate::events::Held,
    dir: &Path,
    session: &str,
    raw: &[u8],
    owns: impl FnOnce() -> Result<(), String>,
) -> Result<Admitted, String> {
    let store = store::open(dir);
    let lock = admission(dir)?;
    owns()?;
    store
        .publish_console_draft(raw)
        .map_err(|why| format!("the draft could not be kept ({why})"))?;
    let events = super::snapshot_in(held, dir)?;
    let open = lane::open_asks(&events, session);
    if open.len() >= OPEN_CAP {
        let count = open.len();
        return Err(format!(
            "{count} requests open; /close withdraws the newest"
        ));
    }
    Ok(Admitted {
        store,
        dir: dir.to_owned(),
        _lock: lock,
    })
}

impl Admitted {
    /// Deliver the ask `id` and read back what became of it, under the lock
    /// taken at the admission; only [`Outcome::Sent`] clears the kept draft.
    pub(crate) fn finish(
        self,
        held: &mut crate::events::Held,
        id: &str,
        deliver: impl FnOnce() -> String,
    ) -> Outcome {
        let said = deliver();
        let outcome = match super::snapshot_in(held, &self.dir) {
            Ok(events) => read_back(&events, id, &said),
            Err(why) => Outcome::Unknown(id.to_owned(), why),
        };
        let Outcome::Sent(id, None) = outcome else {
            return outcome;
        };
        let kept = self.store.clear_console_draft().err();
        let kept = kept.map(|why| format!("the draft is kept ({why})"));
        Outcome::Sent(id, kept)
    }
}

/// `/close [id]` under the same admission and ownership as a submit.
///
/// # Errors
///
/// Why nothing was closed.
pub fn close_owned(
    dir: &Path,
    session: &str,
    which: Option<&str>,
    owns: impl FnOnce() -> Result<(), String>,
) -> Result<super::Closed, String> {
    close_owned_held(
        &mut crate::events::Held::default(),
        dir,
        session,
        which,
        owns,
    )
}

pub(crate) fn close_owned_held(
    held: &mut crate::events::Held,
    dir: &Path,
    session: &str,
    which: Option<&str>,
    owns: impl FnOnce() -> Result<(), String>,
) -> Result<super::Closed, String> {
    let _admitted = admission(dir)?;
    owns()?;
    super::close_held(held, dir, session, which)
}

fn admission(dir: &Path) -> Result<std::fs::File, String> {
    let held = store::open(dir).console_admission();
    held.map_err(|why| format!("the chat admission lock is not free ({why})"))
}

#[cfg(test)]
pub(in crate::console) mod tests {
    use super::{Draft, Outcome, close_owned, owner, restore, submit};
    use crate::store::{self, SourceRead};
    use crate::tmux::WindowPane;
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    const U: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
    const NEW: &str = "ae-20260930T060500Z-000000ff";

    /// A pane whose indexes are its ids' numbers: `%3` in `@2` is (2, 3).
    pub(in crate::console) fn pane(
        id: &str,
        window: &str,
        console: Option<&str>,
        dead: bool,
    ) -> WindowPane {
        WindowPane {
            pane_id: id.to_owned(),
            window_id: window.to_owned(),
            theme: String::new(),
            reader_src: None,
            console: console.map(str::to_owned),
            dead,
            window_index: window[1..].parse().unwrap(),
            pane_index: id[1..].parse().unwrap(),
            agent: None,
        }
    }

    /// A scratch session directory holding console asks `1..=open`.
    fn session(tag: &str, open: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ae-submit-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (1..=open).for_each(|n| append(&dir, &ask(&id(n), "q")));
        dir
    }

    fn id(n: u32) -> String {
        format!("ae-20260930T060000Z-{n:08x}")
    }

    /// The console's ask `id` to `lead` in `main`, pane `%1`.
    fn ask(id: &str, summary: &str) -> String {
        format!(
            r#"{{"ts":"2026-09-30T06:00:00Z","actor":"console:local","action":"ask","target":"lead","ref":"{id}","target_slot":"main","target_session":"s","target_server":"/t","target_pane":"%1","target_session_uuid":"{U}","summary":"{summary}"}}
"#
        )
    }

    /// `lead`'s reply to `id` from pane `%9`, which the ask did not reach.
    fn stale(id: &str) -> String {
        format!(
            r#"{{"ts":"2026-09-30T06:01:00Z","actor":"lead","action":"reply","target":"console:local","ref":"{id}","actor_slot":"main","actor_session":"s","caller_server":"/t","caller_pane":"%9","caller_session_uuid":"{U}"}}
"#
        )
    }

    fn append(dir: &Path, line: &str) {
        store::open(dir).append_event(line).unwrap();
    }

    fn draft(dir: &Path) -> SourceRead {
        store::open(dir).console_draft().unwrap()
    }

    fn journal(dir: &Path) -> Vec<u8> {
        store::open(dir).container()
    }

    /// A submit of ask `NEW` by the console that owns the input.
    fn run(dir: &Path, raw: &[u8], deliver: impl FnOnce() -> String) -> Result<Outcome, String> {
        submit(dir, "s", raw, NEW, || Ok(()), deliver)
    }

    /// A delivery that lands `records` and says `said`.
    fn lands<'a>(dir: &'a Path, records: &'a str, said: &'a str) -> impl FnOnce() -> String + 'a {
        move || {
            append(dir, records);
            said.to_owned()
        }
    }

    fn never() -> String {
        panic!("nothing is delivered past a refusal")
    }

    #[test]
    fn a_kept_draft_comes_back_whole_or_as_a_named_refusal_and_never_changes() {
        let dir = session("restore", 0);
        let path = dir.join(store::CONSOLE_DRAFT);
        let no = |what: &str| Draft::Refused(format!("the kept draft {what} and was not restored"));
        assert_eq!(restore(&dir), Draft::Nothing, "none kept");
        fs::write(&path, b"").unwrap();
        assert_eq!(restore(&dir), Draft::Nothing, "an empty file");
        fs::write(&path, b"@colead a\nb").unwrap();
        assert_eq!(restore(&dir), Draft::Kept(b"@colead a\nb".to_vec()));
        let cap = usize::try_from(store::CONSOLE_DRAFT_CAP).unwrap();
        fs::write(&path, vec![b'x'; cap]).unwrap();
        assert!(matches!(restore(&dir), Draft::Kept(bytes) if bytes.len() == cap));
        fs::write(&path, vec![b'x'; cap + 1]).unwrap();
        assert_eq!(restore(&dir), no("is over 65536 bytes"));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(restore(&dir), no("is not a regular file (a directory)"));
        fs::remove_dir(&path).unwrap();
        fs::write(&path, b"kept").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
        let got = restore(&dir);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            matches!(&got, Draft::Refused(why) if why.starts_with("the kept draft could not be read (")),
            "{got:?}"
        );
        assert_eq!(
            draft(&dir),
            SourceRead::Ready(b"kept".to_vec()),
            "a read changes nothing"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_first_live_console_takes_input_and_every_other_says_why_not() {
        let (lead, late) = (
            pane("%1", "@0", None, false),
            pane("%7", "@5", Some(U), false),
        );
        let (first, dead) = (
            pane("%3", "@2", Some(U), false),
            pane("%2", "@1", Some(U), true),
        );
        let foreign = pane("%4", "@0", Some("other"), false);
        let panes = [lead.clone(), late.clone(), first, dead.clone(), foreign];
        let refused = |why: &str| Err(why.to_owned());
        let owned = refused("input owned by window @2");
        assert_eq!(owner(&panes, Some("%3"), U, U), Ok(()));
        assert_eq!(owner(&panes, Some("%7"), U, U), owned);
        assert_eq!(
            owner(&panes, Some("%1"), U, U),
            owned,
            "an unstamped console"
        );
        assert_eq!(owner(&[lead.clone(), late, dead], Some("%7"), U, U), Ok(()));
        let unnamed = "this chat cannot name its own pane, so it takes no input";
        assert_eq!(owner(&panes, None, U, U), refused(unnamed));
        let gap =
            "the session id in its meta is not the one tmux carries, so this chat takes no input";
        for (stamp, bound) in [(U, ""), (U, "other"), ("", U)] {
            assert_eq!(
                owner(&panes, Some("%3"), stamp, bound),
                refused(gap),
                "{stamp:?} {bound:?}"
            );
        }
        let none = refused("no live chat window holds this session");
        assert_eq!(owner(&[lead], Some("%1"), U, U), none);
    }

    #[test]
    fn a_sixth_ask_is_refused_naming_the_five_open_and_every_refusal_keeps_the_raw_draft() {
        let dir = session("sixth", 5);
        append(&dir, &stale(&id(1)));
        let (raw, before) = (b"@colead first line\nsecond line".as_slice(), journal(&dir));
        let full = "5 requests open; /close withdraws the newest".to_owned();
        assert_eq!(
            run(&dir, raw, never),
            Err(full),
            "a stale reply is not the answer"
        );
        let kept = (before.clone(), SourceRead::Ready(raw.to_vec()));
        assert_eq!((journal(&dir), draft(&dir)), kept);
        let owned = || Err("input owned by window @2".to_owned());
        assert_eq!(
            submit(&dir, "s", b"newer", NEW, owned, never).map(drop),
            owned()
        );
        assert_eq!(
            close_owned(&dir, "s", Some(&id(2)), owned).map(drop),
            owned()
        );
        assert_eq!(
            (journal(&dir), draft(&dir)),
            kept,
            "a read-only console changes nothing"
        );
        let events = dir.join("events.jsonl");
        let strict: [(&[u8], &str, u32); 3] = [
            (b"{\"ts\"", "the journal ends in a partial record", 0o600),
            (b"{not json\n", "journal line 1 is not a record", 0o600),
            (b"{}\n", "the journal is unreadable", 0),
        ];
        for (bytes, why, mode) in strict {
            fs::write(&events, bytes).unwrap();
            fs::set_permissions(&events, fs::Permissions::from_mode(mode)).unwrap();
            let got = run(&dir, bytes, never);
            fs::set_permissions(&events, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                got.as_ref().is_err_and(|got| got.starts_with(why)),
                "{got:?}"
            );
            assert_eq!(draft(&dir), SourceRead::Ready(bytes.to_vec()), "{why}");
        }
        fs::write(&events, &before).unwrap();
        let closed = close_owned(&dir, "s", Some(&id(2)), || Ok(()));
        assert_eq!(
            closed.map(|closed| (closed.id, closed.summary)),
            Ok((id(2), "q".to_owned()))
        );
        let sent = run(&dir, raw, lands(&dir, &ask(NEW, "q"), ""));
        assert_eq!(
            sent,
            Ok(Outcome::Sent(NEW.to_owned(), None)),
            "a close frees a slot"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// What the delivery helpers journal themselves for an ask that never
    /// landed — a deferral abandoned, a notice never proven on screen — reads
    /// back as NOT delivered with the draft kept: the writer's action is the
    /// reader's, and neither is spelled here.
    #[test]
    fn what_the_helpers_journal_for_an_undelivered_ask_reads_back_as_not_delivered() {
        use crate::deliver::{DeferHeld, Failure};
        let failures = [
            Failure::Abandoned {
                held: DeferHeld::ComposerOccupied,
            },
            Failure::Unconfirmed {
                body_file: "/m/notice.txt".to_owned(),
                framed: "framed".to_owned(),
                notice: true,
            },
        ];
        for failure in failures {
            let dir = session("undelivered", 0);
            let got = run(&dir, b"kept", || {
                let fields = crate::tracked::EventFields {
                    ts: crate::time::Timestamp::parse("2026-09-30T06:05:00Z").unwrap(),
                    actor: crate::tracked::CONSOLE_SINK,
                    action: "ask",
                    target: "lead",
                    reference: NEW,
                    actor_slot: "",
                    actor_session: "s",
                    target_slot: "main",
                    target_session: "s",
                    actor_session_id: "",
                    target_session_id: "",
                    target_server: "/t",
                    target_pane: "%1",
                    target_session_uuid: U,
                    caller_server: "",
                    caller_pane: "",
                    caller_session_uuid: "",
                    identity_gap: "",
                    summary: "q",
                    body_file: "",
                };
                let mut said = Vec::new();
                crate::tracked::record_tracked_delivery(
                    &dir,
                    &fields,
                    Err(failure),
                    None,
                    &mut said,
                )
                .unwrap();
                String::from_utf8(said).unwrap()
            });
            assert_eq!(got, Ok(Outcome::NotDelivered(NEW.to_owned())));
            assert_eq!(draft(&dir), SourceRead::Ready(b"kept".to_vec()));
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_submit_holds_admission_through_delivery_and_clears_the_draft_only_when_sent() {
        let dir = session("guard", 4);
        let (mut admission, mut journal_free) = (None, false);
        let sent = run(&dir, b"raw", || {
            let held = store::lock(&dir.join(".console-admission.lock"), Duration::ZERO);
            admission = held.err().map(|why| why.kind());
            let events = store::lock_path(&store::open(&dir).events_path());
            journal_free = store::lock(&events, Duration::ZERO).is_ok();
            lands(&dir, &ask(NEW, "q"), "")()
        });
        assert_eq!(sent, Ok(Outcome::Sent(NEW.to_owned(), None)));
        let held = (Some(std::io::ErrorKind::WouldBlock), true);
        assert_eq!(
            (admission, journal_free),
            held,
            "admission held, journal free"
        );
        assert_eq!(draft(&dir), SourceRead::Absent);
        let abandoned =
            ask(NEW, "q").replace(r#""action":"ask""#, r#""action":"delivery-abandoned""#);
        let other = ask(&id(9), "q") + &ask(NEW, "q").replace("console:local", "lead");
        let unknown = |why: &str| Outcome::Unknown(NEW.to_owned(), why.to_owned());
        let cases = [
            (
                ask(NEW, "[unconfirmed] q"),
                "",
                Outcome::Uncertain(NEW.to_owned()),
            ),
            (abandoned, "", Outcome::NotDelivered(NEW.to_owned())),
            (other, "ae: boom\n", unknown("ae: boom")),
            (
                "{\"ts\"".to_owned(),
                "",
                unknown("the journal ends in a partial record"),
            ),
        ];
        for (records, said, want) in cases {
            fs::write(dir.join("events.jsonl"), "").unwrap();
            assert_eq!(run(&dir, b"kept", lands(&dir, &records, said)), Ok(want));
            assert_eq!(draft(&dir), SourceRead::Ready(b"kept".to_vec()));
        }
        fs::write(dir.join("events.jsonl"), "").unwrap();
        let path = dir.join(store::CONSOLE_DRAFT);
        let got = run(&dir, b"kept", || {
            fs::remove_file(&path).unwrap();
            fs::create_dir_all(path.join("in")).unwrap();
            lands(&dir, &ask(NEW, "q"), "")()
        });
        let kept = matches!(&got, Ok(Outcome::Sent(_, Some(why))) if why.starts_with("the draft is kept ("));
        assert!(kept, "{got:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A console on its own thread submitting ask `n`; `first` runs as it delivers.
    fn racer(
        dir: &Path,
        n: u32,
        first: impl FnOnce() + Send + 'static,
    ) -> JoinHandle<Result<Outcome, String>> {
        let dir = dir.to_owned();
        std::thread::spawn(move || {
            submit(
                &dir,
                "s",
                b"raw",
                &id(n),
                || Ok(()),
                || {
                    first();
                    lands(&dir, &ask(&id(n), "q"), "")()
                },
            )
        })
    }

    #[test]
    fn two_consoles_racing_for_the_fifth_slot_deliver_exactly_one() {
        let dir = session("race", 4);
        let ((a_in, a_seen), (go, a_go), (b_in, b_seen)) =
            (mpsc::channel(), mpsc::channel(), mpsc::channel());
        let a = racer(&dir, 10, move || {
            a_in.send(()).unwrap();
            a_go.recv_timeout(Duration::from_secs(30)).unwrap();
        });
        a_seen.recv_timeout(Duration::from_secs(30)).unwrap();
        let b = racer(&dir, 11, move || b_in.send(()).unwrap());
        let early = b_seen.recv_timeout(Duration::from_millis(500));
        assert!(
            early.is_err(),
            "the second delivered while the first held the fifth slot"
        );
        go.send(()).unwrap();
        assert_eq!(a.join().unwrap(), Ok(Outcome::Sent(id(10), None)));
        let b = b.join().unwrap();
        assert!(
            b.as_ref()
                .is_err_and(|why| why.starts_with("5 requests open;")),
            "{b:?}"
        );
        assert!(b_seen.try_recv().is_err(), "the second never delivered");
        let _ = fs::remove_dir_all(&dir);
    }
}
