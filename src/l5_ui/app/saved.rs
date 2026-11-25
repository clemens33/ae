//! The durable app view: the one private file `<state root>/app-view` that a
//! fresh `ae` or `ae app` restores the app's layout from.
//!
//! The file holds exactly what [`handover::render_file`] writes: one canonical
//! handover line, bound to no pane, and one line feed. An older core's file
//! may also carry the retired `|desk=` suffix of its board column between the
//! line and the line feed: the reader drops it and the next write omits it. It
//! is hostile input
//! like the restart handover, so the read never follows a link, never opens a
//! node that is not a regular file, never waits on one and never reads past
//! [`handover::FILE_CAP`]; anything else it finds reads as no file. A damaged
//! or foreign file therefore costs only the restore, never the start.
//!
//! The write is private (0600), whole or not at all (a temp file beside it,
//! `fsync`, rename) and refuses a link or any other node already standing at
//! the path. The app's writes go through one thread, [`Saver`], which
//! publishes in the order it was handed them, so a slow older write can never
//! land after a newer one and the app's final write is always the last. A
//! debounced write waits for the one before it to end and for the cadence to
//! pass, so a slow disk can never turn two writes into a burst.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::handover::{self, Handover};
use crate::store::{self, SourcePresence, SourceRead};

/// The least time from the end of one debounced write to the next.
pub(super) const DEBOUNCE: Duration = Duration::from_secs(2);

/// Where the durable view of state root `root` lives.
#[must_use]
pub fn path(root: &Path) -> PathBuf {
    root.join(handover::FILE)
}

/// The handover the file at `path` holds, or `None` for anything else: no
/// file, a link or any other node that is not a regular file, a file over
/// [`handover::FILE_CAP`], bytes that are not exactly what
/// [`handover::render_file`] writes (a retired `|desk=` suffix before the line
/// feed, which `render_file` omits, also reads), or a target that cannot refuse links. The
/// open itself refuses a link swapped in after the check and never waits on a
/// FIFO swapped in.
#[must_use]
pub fn read_file(path: &Path) -> Option<Handover> {
    match store::read_nofollow_capped(path, handover::FILE_CAP as u64) {
        Ok(SourceRead::Ready(bytes)) => handover::parse_file(&bytes),
        Ok(_) | Err(_) => None,
    }
}

/// Publish `text` at `path`, private, whole or not at all.
///
/// # Errors
///
/// Why it was not written: a link or any other node already at `path` (never
/// replaced and never written through), or the temp file, its `fsync` or the
/// rename.
pub fn write_file(path: &Path, text: &str) -> Result<(), String> {
    match store::source_presence(path) {
        SourcePresence::Absent | SourcePresence::Regular => {}
        SourcePresence::Irregular(what) => {
            return Err(format!("{} is {what}; it is not replaced", path.display()));
        }
    }
    store::publish_private(path, text.as_bytes())
        .map_err(|why| format!("could not write {}: {why}", path.display()))
}

/// The publish of one text: the file door, or a recorder in a test.
type Sink = dyn Fn(&str) -> Result<(), String> + Send + Sync;

/// What one write is handed to the thread as: the text and, for the final one,
/// where its outcome goes.
struct Job {
    text: String,
    ack: Option<Sender<Result<(), String>>>,
}

/// What the thread and the app both read of the writes between them.
#[derive(Default)]
struct Pace {
    /// Texts handed to the thread whose write has not ended.
    waiting: usize,
    /// When the thread last ended a write, published or refused.
    done: Option<Instant>,
}

/// Publishes the durable view for the app: one thread, `ae-app-view`, takes
/// the texts in the order they were handed in and writes the newest of any
/// that are waiting together. A debounced text goes in only when no write is
/// in flight and the last one ended at least a cadence ago, so a slow write
/// can never let two publishes land back to back. Only the final write is
/// waited for, without a time limit, and it bypasses the cadence; the others
/// never block the app.
pub(super) struct Saver {
    jobs: Option<Sender<Job>>,
    sink: Arc<Sink>,
    pace: Arc<Mutex<Pace>>,
    /// The least time from the end of one debounced write to the next.
    gap: Duration,
}

impl Saver {
    /// The writer of the durable view: `sink` publishes one text and `say`
    /// is told, once, why a debounced write failed. A thread that cannot be
    /// started leaves the app without debounced writes; its final write then
    /// runs on the caller's thread.
    pub(super) fn start(
        sink: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
        say: impl FnMut(String) + Send + 'static,
    ) -> Self {
        Self::paced(DEBOUNCE, sink, say)
    }

    fn paced(
        gap: Duration,
        sink: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
        say: impl FnMut(String) + Send + 'static,
    ) -> Self {
        let sink: Arc<Sink> = Arc::new(sink);
        let pace = Arc::new(Mutex::new(Pace::default()));
        let (jobs, queue) = mpsc::channel();
        let (worker, shared) = (Arc::clone(&sink), Arc::clone(&pace));
        let spawned = std::thread::Builder::new()
            .name("ae-app-view".to_owned())
            .spawn(move || work(&queue, worker.as_ref(), &shared, say));
        Self {
            jobs: spawned.is_ok().then_some(jobs),
            sink,
            pace,
            gap,
        }
    }

    fn pace(&self) -> MutexGuard<'_, Pace> {
        self.pace.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a debounced text may go in at `now`: the thread runs, none of
    /// its writes is in flight, and the last one ended at least a cadence ago.
    pub(super) fn ready(&self, now: Instant) -> bool {
        self.jobs.is_some() && {
            let pace = self.pace();
            pace.waiting == 0
                && pace
                    .done
                    .is_none_or(|done| now.saturating_duration_since(done) >= self.gap)
        }
    }

    /// Hand `text` to the thread unless a write is in flight or the last one
    /// ended less than a cadence ago: whether it was handed in.
    pub(super) fn submit(&mut self, text: String, now: Instant) -> bool {
        if !self.ready(now) {
            return false;
        }
        let Some(jobs) = self.jobs.as_ref() else {
            return false;
        };
        self.pace().waiting += 1;
        if jobs.send(Job { text, ack: None }).is_err() {
            self.jobs = None;
            return false;
        }
        true
    }

    /// Write `text` as the last word, waiting for it with no time limit: it
    /// is queued behind every older text, so nothing older can land after it,
    /// and it takes no account of the cadence. A thread that is gone is
    /// detected on the answer channel, and the write then runs here.
    ///
    /// # Errors
    ///
    /// Why the write failed.
    pub(super) fn finish(&mut self, text: &str) -> Result<(), String> {
        if let Some(jobs) = self.jobs.as_ref() {
            let (ack, answer) = mpsc::channel();
            let job = Job {
                text: text.to_owned(),
                ack: Some(ack),
            };
            self.pace
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .waiting += 1;
            if jobs.send(job).is_ok()
                && let Ok(outcome) = answer.recv()
            {
                return outcome;
            }
        }
        (self.sink)(text)
    }
}

/// The thread: take a job, fold in every job already waiting, write the newest
/// text, record that the write ended, answer every final write that was folded
/// in. A failure nobody waits for goes to `say` once.
fn work(queue: &Receiver<Job>, sink: &Sink, pace: &Mutex<Pace>, mut say: impl FnMut(String)) {
    let mut said = false;
    while let Ok(mut job) = queue.recv() {
        let mut acks: Vec<_> = job.ack.take().into_iter().collect();
        let mut taken = 1;
        while let Ok(next) = queue.try_recv() {
            job.text = next.text;
            acks.extend(next.ack);
            taken += 1;
        }
        let outcome = sink(&job.text);
        {
            let mut pace = pace.lock().unwrap_or_else(PoisonError::into_inner);
            pace.waiting = pace.waiting.saturating_sub(taken);
            pace.done = Some(Instant::now());
        }
        if acks.is_empty()
            && let Err(why) = &outcome
            && !std::mem::replace(&mut said, true)
        {
            say(why.clone());
        }
        for ack in acks {
            let _ = ack.send(outcome.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Short enough for a test, long enough that a loaded machine does not
    /// slip a whole cadence between two statements.
    const GAP: Duration = Duration::from_millis(600);

    type Log = Arc<Mutex<Vec<String>>>;

    fn recorder() -> (
        Log,
        impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        (log, move |text: &str| {
            seen.lock().expect("log").push(text.to_owned());
            Ok(())
        })
    }

    fn until(what: &str, met: impl Fn() -> bool) {
        let by = Instant::now() + Duration::from_secs(10);
        while !met() {
            assert!(Instant::now() < by, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_final_write_lands_last_and_is_waited_for() {
        let (log, sink) = recorder();
        let mut saver = Saver::paced(GAP, sink, |_| {});
        assert!(saver.submit("first".into(), Instant::now()));
        assert!(
            !saver.submit("again".into(), Instant::now()),
            "a second background write inside the cadence is refused"
        );
        assert_eq!(saver.finish("final"), Ok(()));
        let written = log.lock().expect("log").clone();
        assert_eq!(written.last().map(String::as_str), Some("final"));
        assert!(
            written.iter().all(|text| text != "again"),
            "a refused text is never written: {written:?}"
        );
    }

    #[test]
    fn a_slow_write_holds_the_next_background_one_until_the_cadence_allows_it() {
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        let sink = move |text: &str| {
            if text == "a" {
                let _ = gate.lock().expect("gate").recv();
            }
            seen.lock().expect("log").push(text.to_owned());
            Ok(())
        };
        let mut saver = Saver::paced(GAP, sink, |_| {});
        assert!(saver.submit("a".into(), Instant::now()));
        // Past the cadence, counted from the hand-in, with `a` still unpublished.
        std::thread::sleep(GAP + Duration::from_millis(150));
        assert!(!saver.ready(Instant::now()), "a is still in flight");
        assert!(!saver.submit("b".into(), Instant::now()));
        release.send(()).expect("open the gate");
        until("a is published", || log.lock().expect("log").len() == 1);
        assert!(
            !saver.ready(Instant::now()),
            "the cadence runs from the publish, not from the hand-in"
        );
        std::thread::sleep(GAP + Duration::from_millis(100));
        assert!(saver.ready(Instant::now()));
        assert!(saver.submit("c".into(), Instant::now()));
        assert_eq!(
            saver.finish("final"),
            Ok(()),
            "the final write bypasses the cadence"
        );
        let written = log.lock().expect("log").clone();
        assert_eq!(written.first().map(String::as_str), Some("a"));
        assert_eq!(written.last().map(String::as_str), Some("final"));
        assert!(written.iter().all(|text| text != "b"), "{written:?}");
    }

    #[test]
    fn a_slow_older_write_never_lands_after_the_final_one() {
        let (release, gate) = mpsc::channel::<()>();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&log);
        let gate = Mutex::new(gate);
        let sink = move |text: &str| {
            if text == "slow" {
                let _ = gate.lock().expect("gate").recv();
            }
            seen.lock().expect("log").push(text.to_owned());
            Ok(())
        };
        let mut saver = Saver::start(sink, |_| {});
        assert!(saver.submit("slow".into(), Instant::now()));
        std::thread::sleep(Duration::from_millis(100));
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let _ = release.send(());
        });
        assert_eq!(saver.finish("final"), Ok(()));
        releaser.join().expect("releaser");
        let written = log.lock().expect("log").clone();
        assert_eq!(written, ["slow", "final"], "in the order handed in");
    }

    #[test]
    fn a_failed_debounced_write_is_said_once_and_a_failed_final_one_is_returned() {
        let said = Arc::new(Mutex::new(Vec::new()));
        let told = Arc::clone(&said);
        let mut saver = Saver::paced(
            Duration::from_millis(30),
            |_: &str| Err("refused".to_owned()),
            move |why| told.lock().expect("said").push(why),
        );
        for turn in 0..3u32 {
            until("the cadence allows the next write", || {
                saver.ready(Instant::now())
            });
            assert!(saver.submit(format!("t{turn}"), Instant::now()));
        }
        assert_eq!(saver.finish("end"), Err("refused".to_owned()));
        assert_eq!(said.lock().expect("said").as_slice(), ["refused"]);
    }

    #[test]
    fn a_dead_thread_is_detected_and_the_final_write_runs_here() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (log, sink) = recorder();
        let died = AtomicBool::new(false);
        // The first call, on the thread, panics and ends it; the call that
        // follows, on the caller's thread, writes.
        let sink = move |text: &str| {
            assert!(died.swap(true, Ordering::SeqCst), "the thread dies here");
            sink(text)
        };
        let mut saver = Saver::start(sink, |_| {});
        assert_eq!(saver.finish("final"), Ok(()));
        assert_eq!(log.lock().expect("log").as_slice(), ["final"]);
    }

    /// Texts queued behind each other are folded into one write of the newest,
    /// and every one of them leaves the in-flight count, whatever its kind.
    #[test]
    fn folded_texts_all_leave_the_in_flight_count_and_every_final_one_is_answered() {
        let (send, queue) = mpsc::channel();
        let ((first, heard_first), (second, heard_second)) = (mpsc::channel(), mpsc::channel());
        for (text, ack) in [("a", None), ("b", Some(first)), ("c", Some(second))] {
            let job = Job {
                text: text.to_owned(),
                ack,
            };
            assert!(send.send(job).is_ok());
        }
        drop(send);
        let pace = Mutex::new(Pace {
            waiting: 3,
            done: None,
        });
        let (log, sink) = recorder();
        work(&queue, &sink, &pace, |_| {});
        assert_eq!(*log.lock().expect("log"), ["c"], "one write, the newest");
        let pace = pace.lock().expect("pace");
        assert_eq!(pace.waiting, 0, "three texts were taken, three leave");
        assert!(pace.done.is_some(), "the end of the write is recorded");
        assert_eq!(heard_first.recv(), Ok(Ok(())));
        assert_eq!(heard_second.recv(), Ok(Ok(())));
    }

    /// A background write that fails is said once, for the first failure, and
    /// a later one stays quiet.
    #[test]
    fn the_first_failed_background_write_is_the_one_said() {
        let (send, queue) = mpsc::channel();
        let (second, said) = (send.clone(), Mutex::new(Vec::new()));
        let later = Mutex::new(Some(second));
        let sink = move |text: &str| {
            if text == "one" {
                // While the first write runs, the next text is queued: it is
                // a cycle of its own, and the last sender goes with it.
                let next = later.lock().expect("sender").take();
                if let Some(next) = next {
                    let _ = next.send(Job {
                        text: "two".to_owned(),
                        ack: None,
                    });
                }
                Err("first".to_owned())
            } else {
                Err("second".to_owned())
            }
        };
        let job = Job {
            text: "one".to_owned(),
            ack: None,
        };
        assert!(send.send(job).is_ok());
        drop(send);
        let pace = Mutex::new(Pace {
            waiting: 2,
            done: None,
        });
        work(&queue, &sink, &pace, |why| {
            said.lock().expect("said").push(why);
        });
        assert_eq!(*said.lock().expect("said"), ["first"]);
    }

    /// While the final write runs the thread counts it in flight, as it does
    /// a background one, and the count is empty once it has landed.
    #[test]
    fn the_final_write_is_counted_in_flight_until_it_lands() {
        let (open, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (log, record) = recorder();
        let mut saver = Saver::paced(
            GAP,
            move |text: &str| {
                let _ = gate.lock().expect("gate").recv();
                record(text)
            },
            |_| {},
        );
        let pace = Arc::clone(&saver.pace);
        let last = std::thread::spawn(move || saver.finish("final"));
        until("the final write is counted in flight", || {
            pace.lock().expect("pace").waiting == 1
        });
        assert!(open.send(()).is_ok());
        assert_eq!(last.join().expect("the final write ends"), Ok(()));
        assert_eq!(pace.lock().expect("pace").waiting, 0);
        assert_eq!(*log.lock().expect("log"), ["final"]);
    }
}
