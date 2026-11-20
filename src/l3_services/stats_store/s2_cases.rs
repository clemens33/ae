//! Frozen S2 fixtures and independent journal-fold oracle. No disk wire parsing here.
#![allow(
    dead_code,
    reason = "shared by unit acceptance tests and the fuzz target"
)]
#![allow(
    clippy::expect_used,
    reason = "asserted fixture and contract operations"
)]

#[cfg(test)]
use crate as ae;
use ae::stats::{Fold, Reading, Source, SourceGap, View};
use ae::stats_store::disk::{
    self, Decision, Dir, Limits, Memory, MemoryJournals, Next, Plan, Prepared, Shown, SourceSpec,
};

pub const S: &str = "11111111-1111-4111-8111-111111111111";
pub const T: &str = "22222222-2222-4222-8222-222222222222";
pub const PRIVATE: &str = "PRIVATE-S2-PROMPT-https://private.invalid/secret";

pub fn birth(session: &str, n: u32) -> Vec<u8> {
    format!("{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"worker-birth\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session_id\":\"{session}\",\"birth_v\":1,\"birth\":\"aaaaaaaa-aaaa-4aaa-8aaa-{n:012x}\",\"birth_tool\":\"codex\",\"birth_profile\":\"preset\",\"birth_model\":\"sol\",\"birth_effort\":\"high\",\"summary\":\"{PRIVATE}\"}}\n").into_bytes()
}

pub fn dated(width: usize) -> Vec<u8> {
    let mut line = format!(
        "{{\"ts\":\"2026-10-01T10:00:00Z\",\"actor\":\"lead\",\"action\":\"memo\",\"summary\":\"{PRIVATE}"
    );
    assert!(width >= line.len() + 2);
    line.push_str(&"x".repeat(width - line.len() - 2));
    line.push_str("\"}");
    assert_eq!(line.len(), width);
    line.push('\n');
    line.into_bytes()
}

#[derive(Clone, Debug)]
pub struct Feed {
    pub spec: SourceSpec,
    pub bytes: Vec<u8>,
    pub cursor: u64,
    pub active: bool,
    pub early: Vec<(u64, Vec<u8>)>,
    pub ino: u64,
}

impl Feed {
    pub fn live(name: &str, session: Option<&str>, bytes: Vec<u8>) -> Self {
        Self {
            spec: SourceSpec::Live {
                id: name.to_owned(),
                label: name.to_owned(),
                meta: session.map_or_else(Vec::new, |s| format!("session_id={s}\n").into_bytes()),
            },
            bytes,
            cursor: 0,
            active: false,
            early: Vec::new(),
            ino: 10,
        }
    }

    pub fn id(&self) -> &str {
        match &self.spec {
            SourceSpec::Live { id, .. }
            | SourceSpec::Archive { id, .. }
            | SourceSpec::Failed { id, .. } => id,
        }
    }

    pub fn group(&self) -> Option<&str> {
        match &self.spec {
            SourceSpec::Live { meta, .. } => std::str::from_utf8(meta)
                .ok()
                .and_then(|m| m.strip_prefix("session_id="))
                .map(str::trim),
            SourceSpec::Archive { uuid, .. } => Some(uuid),
            SourceSpec::Failed { session, .. } => session.as_deref(),
        }
    }

    pub fn selected(&self) -> Vec<u8> {
        let end = usize::try_from(self.cursor).expect("fixture cursor");
        let mut bytes = self.bytes[..end].to_vec();
        for (at, line) in &self.early {
            if *at >= self.cursor {
                bytes.extend_from_slice(line);
                bytes.push(b'\n');
            }
        }
        bytes
    }

    pub fn source(&self) -> Source {
        match &self.spec {
            SourceSpec::Live { label, meta, .. } => Source::live(label, meta, Ok(self.selected())),
            SourceSpec::Archive { uuid, .. } => Source::archive(uuid, Ok(self.selected())),
            SourceSpec::Failed {
                label,
                session,
                gap,
                ..
            } => Source::failed(label, session.as_deref(), *gap),
        }
    }
}

pub fn oracle(feeds: &[Feed], scope: Option<&str>, line_cap: usize) -> View {
    let mut fold = Fold::default();
    for feed in feeds.iter().filter(|f| f.active) {
        let reading = match &feed.spec {
            SourceSpec::Live { label, meta, .. } => Reading::live(label, meta, u64::MAX),
            SourceSpec::Archive { uuid, .. } => Reading::archive(uuid, u64::MAX),
            SourceSpec::Failed { .. } => {
                fold.add(feed.source());
                continue;
            }
        };
        let mut reading = reading.expect("fixture reading");
        let selected = feed.selected();
        let mut rows = selected.split(|b| *b == b'\n').peekable();
        while let Some(line) = rows.next() {
            if line.is_empty() && rows.peek().is_none() {
                break;
            }
            if line.len() > line_cap {
                reading.over_long();
            } else {
                reading.line(line).expect("unlimited oracle room");
            }
        }
        fold.add_reading(reading);
    }
    fold.finish(scope)
}

pub fn complete(plan: &Plan, dir: &mut dyn Dir) {
    plan.apply(dir, plan.ops.len()).expect("full plan");
}

pub fn recover(dir: &mut dyn Dir) {
    let plan = disk::recovery(dir).expect("recovery plan");
    complete(&plan, dir);
    assert!(
        disk::recovery(dir)
            .expect("idempotent recovery")
            .ops
            .is_empty()
    );
}

pub fn view(dir: &mut dyn Dir, scope: Option<&str>) -> View {
    match disk::shown(dir, scope) {
        Shown::Ready { view, .. } => view,
        other => panic!("expected materialized answer, got {other:?}"),
    }
}

pub fn check(dir: &mut dyn Dir, feeds: &[Feed], line: usize) {
    if !feeds.iter().any(|f| f.active) {
        return;
    }
    let expected = oracle(feeds, None, line);
    let actual = view(dir, None);
    assert_eq!(actual, expected, "all fields against original fold");
    assert!(actual.all.balanced());
    let Shown::Ready { view: sum, .. } = disk::resum(dir).expect("independent re-sum") else {
        panic!("re-sum ready");
    };
    assert_eq!(
        sum.all, expected.all,
        "delta root equals independent session sum"
    );
    for session in [S, T] {
        if feeds.iter().filter(|f| f.active).any(|f| {
            f.group() == Some(session)
                || f.selected()
                    .windows(session.len())
                    .any(|w| w == session.as_bytes())
        }) {
            assert_eq!(
                view(dir, Some(session)),
                oracle(feeds, Some(session), line),
                "scope {session}"
            );
        }
    }
}

pub fn published(decision: Decision) -> Plan {
    match decision {
        Decision::Publish(plan) => plan,
        other @ Decision::Discard(_) => panic!("expected publish, got {other:?}"),
    }
}

pub struct Rig {
    pub dir: Memory,
    pub journals: MemoryJournals,
    pub feeds: Vec<Feed>,
    pub limits: Limits,
}

impl Rig {
    pub fn new(feeds: Vec<Feed>, chunk: u64, line: usize) -> Self {
        let mut rig = Self {
            dir: Memory::new(),
            journals: MemoryJournals::new(),
            feeds,
            limits: Limits { chunk, line },
        };
        for i in 0..rig.feeds.len() {
            rig.sync(i);
        }
        rig
    }
    pub fn sync(&mut self, i: usize) {
        let f = &self.feeds[i];
        self.journals.set(f.id(), 1, f.ino, &f.bytes);
    }
    pub fn ready(&mut self, i: usize) -> Prepared {
        recover(&mut self.dir);
        let mut held = None;
        for _ in 0..self.feeds[i].bytes.len().saturating_add(8) {
            match Prepared::read(
                &mut self.dir,
                &mut self.journals,
                &self.feeds[i].spec,
                self.limits,
                held,
            )
            .expect("prepare")
            {
                Next::Work(work) => return work,
                Next::Holding(next) => held = Some(next),
                Next::Nothing => panic!("expected work"),
            }
        }
        panic!("bounded prepare progress")
    }
    pub fn apply(&mut self, i: usize, work: &Prepared) {
        let plan = published(
            disk::publish(&mut self.dir, &mut self.journals, work).expect("publish decision"),
        );
        complete(&plan, &mut self.dir);
        self.accept(i, work);
        self.check();
    }
    pub fn accept(&mut self, i: usize, work: &Prepared) {
        if work.is_reset() {
            self.feeds[i].early.clear();
        }
        self.feeds[i].cursor = work.span().1;
        self.feeds[i].active = true;
    }
    pub fn catchup(&mut self, i: usize) {
        // Crash recovery may retain a payload already at this source's end.
        // Probe the real product quiet path rather than skipping a completed
        // source. ready remains strict for outstanding or unregistered work.
        if self.feeds[i].active && self.feeds[i].cursor == self.feeds[i].bytes.len() as u64 {
            recover(&mut self.dir);
            let snapshot = |dir: &Memory| {
                dir.names()
                    .into_iter()
                    .map(|name| {
                        let bytes = dir.file(&name).expect("durable fixture file").to_vec();
                        (name, bytes)
                    })
                    .collect::<Vec<_>>()
            };
            let before = snapshot(&self.dir);
            let next = Prepared::read(
                &mut self.dir,
                &mut self.journals,
                &self.feeds[i].spec,
                self.limits,
                None,
            )
            .expect("read an already completed source");
            assert!(
                matches!(next, Next::Nothing),
                "the real product must find no work for a completed source: {next:?}"
            );
            assert_eq!(
                snapshot(&self.dir),
                before,
                "the quiet read changes no durable file or source cursor"
            );
            self.check();
            return;
        }
        let max = self.feeds[i].bytes.len().saturating_add(8);
        for _ in 0..max {
            let work = self.ready(i);
            let end = work.span().1;
            self.apply(i, &work);
            if end == self.feeds[i].bytes.len() as u64 {
                return;
            }
        }
        panic!("catch-up made no progress")
    }
    pub fn check(&mut self) {
        check(&mut self.dir, &self.feeds, self.limits.line);
    }
    pub fn event(&mut self, i: usize, offset: u64, line: &[u8]) {
        let f = &self.feeds[i];
        let plan = published(
            disk::event(&mut self.dir, f.group(), f.id(), offset, line).expect("event decision"),
        );
        complete(&plan, &mut self.dir);
        self.feeds[i].early.push((offset, line.to_vec()));
        self.check();
    }
}

pub fn fixtures() -> Vec<Feed> {
    let mut a = birth(S, 1);
    a.extend(birth(T, 2)); // foreign certificate, touches two groups
    a.extend(
        String::from_utf8(dated(180))
            .expect("fixture UTF-8")
            .replace("2026-10-01", "2020-01-01")
            .as_bytes(),
    );
    let mut b = birth(T, 3);
    b.extend_from_slice(b"not-json\n");
    b.extend(format!("{{\"ts\":\"2028-12-31T00:00:00Z\",\"actor\":\"lead\",\"action\":\"spawn\",\"target\":\"legacy\",\"summary\":\"{PRIVATE}\"}}\n").as_bytes());
    let conflict = String::from_utf8(birth(S, 1))
        .expect("fixture UTF-8")
        .replace("\"birth_model\":\"sol\"", "\"birth_model\":\"other\"")
        .into_bytes();
    vec![
        Feed::live("a", Some(S), a),
        Feed::live("b", Some(T), b),
        Feed::live("idless", None, b"bad\n".to_vec()),
        Feed::live("conflict", Some(S), conflict),
        failure_feed(),
    ]
}

pub fn failure_feed() -> Feed {
    Feed {
        spec: SourceSpec::Failed {
            id: "missing".to_owned(),
            label: "missing".to_owned(),
            session: None,
            gap: SourceGap::Missing,
        },
        bytes: Vec::new(),
        cursor: 0,
        active: false,
        early: Vec::new(),
        ino: 20,
    }
}
