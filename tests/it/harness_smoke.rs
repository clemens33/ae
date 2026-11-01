//! The on-demand live harness smoke: ae's adapter rows against the REAL tools.
//!
//! Every other test in this suite drives a fake tool, and vendors change their
//! TUI, argv and session stores weekly, so the first witness of a broken
//! adapter row (input readiness, frame classifier, id capture, exact resume)
//! was a live seat. This file is the lane that asks the real binaries, with a
//! real login, on demand: `just harness-smoke`. It is NEVER in `just test`,
//! `just pre`, `rust-check`, mutants or CI: both tests are `#[ignore]`d AND
//! gated on `AE_HARNESS_SMOKE=1`, the precedent `telegram.rs`'s live test set.
//!
//! One tool, one scratch world, one session, driven through the PUBLIC verbs a
//! human types, each step with a deadline and no retry:
//!
//! `bin` (SKIP when absent) → `version` → `launch` → `ready` (a drawn, idle
//! input box) → `id` (the harness session id captured) → `turn1` (one delivered
//! turn, answered) → `idle` (the frame reads idle after it) → `stop` →
//! `resume` (same id, ae's journal says resumed) → `turn2` (a recall turn: the
//! resumed conversation must still hold turn1's answer).
//!
//! A tool reads PASS only when every step ran and passed. A step that cannot
//! be observed (no frame grammar for the tool) reads SKIP and the tool can no
//! longer read PASS; a vendor usage limit reads LIMIT; anything else that goes
//! wrong reads FAIL naming the step. Never green by omission.
//!
//! WHY the answer cannot be forged by an echo. Turn 1 asks for the sum of two
//! random four-digit numbers; the sum never occurs in the prompt, so a row that is
//! the sum ALONE (bullet or box decoration aside, never a status row that merely
//! holds the digits) AFTER the row echoing the prompt's marker is the model's
//! answer. Turn 2 (after the resume) asks which number it replied with; its
//! prompt holds no number, so a blank or stale conversation cannot pass, and
//! ae's metadata and journal alone cannot pass either.
//!
//! SPEND. At most two turns are sent by this runner per tool, never retried. A
//! tool whose adapter has a launch turn (codex's registration turn) also gets
//! ae's own turn on create and on resume: lifecycle, counted as `ae=` ATTEMPTS.
//!
//! ISOLATION (the AGENTS.md one-shot probe): the real `HOME` (the logins), a
//! scratch `AE_HOME` / `CONFIG_FILE` / tmux server socket, `TMUX` and
//! `TMUX_PANE` removed. The fixed work directory is one canonical path with an
//! exclusive lock, so a vendor's trust list gains ONE entry, not one per run.
//! Pane text is read in memory through the product's own capture leg and feeds a
//! decision. The one thing ever written is the last frame of a readiness wait that
//! failed: bounded, neutralised, owner-only, and only in the report directory a
//! human asked for. It is never printed.
//!
//! WHAT A VENDOR WRITES TO ITS REAL CONFIG (documented, never cleaned): claude
//! records the folder trust in `<config_home>/.claude.json` and the
//! conversation under `<config_home>/projects/`; codex writes a rollout under
//! `~/.codex/sessions/` (and a trust entry if its trust modal fires, which ae
//! does not answer); muse, opencode, agy and grok keep their own stores. The
//! conversations a run leaves are NAMED in the report, never purged.

#![allow(
    clippy::disallowed_methods,
    reason = "a live fixture reads the environment, the work directory and the scratch session \
              store; the boundary is about what PRODUCT code may reach"
)]

use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use ae::harness_state::{self, HarnessState};
use ae::inventory::ServerId;
use ae::meta::Selector;
use ae::tool::ToolKind;

use super::cli::{self, OwnedChild, OwnedScratch, Runner};

/// The second gate: the tests do nothing unless this is exactly `1`.
const GATE: &str = "AE_HARNESS_SMOKE";

/// The seat name `[roster]` gives the session's main seat; `send` addresses it.
const SEAT: &str = "lead";

/// How often a pane is re-read while a step waits.
const POLL: Duration = Duration::from_millis(400);

/// The gap between the two reads that prove a frame is stable.
const FRAME_GAP: Duration = Duration::from_secs(1);

/// The first idle frame may take this long when the watchdog answers a trust
/// block (see `Run::ready_bound`).
const WATCHDOG_READY: Duration = Duration::from_mins(3);

/// How long a drained pipe may keep a finished child's reader alive.
const DRAIN: Duration = Duration::from_secs(3);

/// The tools the smoke can drive, and the profile command each runs by default:
/// the cheapest model each, the lowest effort the tool takes. Every command is
/// overridable with `AE_HARNESS_SMOKE_<TOOL>_CMD`; a stale default reads as a
/// FAIL at `ready` or `launch`, which is the truth about the command.
struct ToolSpec {
    /// The binary name, the report label and the `AE_HARNESS_SMOKE_<TOOL>_CMD`
    /// stem.
    name: &'static str,
    /// The default profile command.
    cmd: &'static str,
}

const TOOLS: [ToolSpec; 7] = [
    ToolSpec {
        name: "claude",
        cmd: "claude --model claude-haiku-5-5 --effort low",
    },
    ToolSpec {
        name: "codex",
        cmd: "codex --yolo -m gpt-6-luna -c model_reasoning_effort=low",
    },
    ToolSpec {
        name: "muse",
        cmd: "muse --provider meta --model muse-spark-1.3 --reasoning-effort low --contributor-mode off",
    },
    ToolSpec {
        name: "opencode",
        cmd: "opencode --model opencode/glm-5.3-flash-free",
    },
    ToolSpec {
        name: "agy",
        cmd: "agy",
    },
    ToolSpec {
        name: "grok",
        cmd: "grok",
    },
    ToolSpec {
        name: "gemini",
        cmd: "gemini",
    },
];

/// Run by default: the tools whose TUI ae models first. The rest are opt-in.
const DEFAULT_TOOLS: &str = "claude,codex,muse,opencode";

// ─── steps, marks, verdicts ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Bin,
    Version,
    Launch,
    Ready,
    Id,
    Turn1,
    Idle,
    Stop,
    Resume,
    Turn2,
}

const STEPS: [Step; 10] = [
    Step::Bin,
    Step::Version,
    Step::Launch,
    Step::Ready,
    Step::Id,
    Step::Turn1,
    Step::Idle,
    Step::Stop,
    Step::Resume,
    Step::Turn2,
];

impl Step {
    const fn name(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Version => "version",
            Self::Launch => "launch",
            Self::Ready => "ready",
            Self::Id => "id",
            Self::Turn1 => "turn1",
            Self::Idle => "idle",
            Self::Stop => "stop",
            Self::Resume => "resume",
            Self::Turn2 => "turn2",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Bin => 0,
            Self::Version => 1,
            Self::Launch => 2,
            Self::Ready => 3,
            Self::Id => 4,
            Self::Turn1 => 5,
            Self::Idle => 6,
            Self::Stop => 7,
            Self::Resume => 8,
            Self::Turn2 => 9,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mark {
    NotReached,
    Ok,
    Fail,
    Skip,
    Limit,
}

impl Mark {
    const fn glyph(self) -> char {
        match self {
            Self::NotReached => '-',
            Self::Ok => '+',
            Self::Fail => 'X',
            Self::Skip => 's',
            Self::Limit => 'L',
        }
    }
}

/// What one tool's run established. Shared with the worker so an outer bound
/// can still report how far it got.
#[derive(Clone, Debug)]
struct Outcome {
    marks: [Mark; 10],
    /// The first FAIL or SKIP/LIMIT reason, a fixed word string.
    detail: String,
    failed: Option<Step>,
    limited: Option<Step>,
    skipped: Option<Step>,
    version: String,
    /// Turns this runner sent (never above two).
    runner_turns: u8,
    /// Turns ae itself pasted (the adapter's launch turn on create / resume).
    ae_turns: u8,
    /// The harness session id(s) this run left in the vendor's store.
    strays: Vec<String>,
    /// The newest pane frame a readiness wait saw, already neutralised and
    /// bounded ([`frame_evidence`]), published as it is seen and cleared when the
    /// wait passes: what is left on a failed run is the one witness of why the
    /// tool never drew its box.
    frame: Option<String>,
}

impl Outcome {
    fn new() -> Self {
        Self {
            marks: [Mark::NotReached; 10],
            detail: String::new(),
            failed: None,
            limited: None,
            skipped: None,
            version: "-".to_owned(),
            runner_turns: 0,
            ae_turns: 0,
            strays: Vec::new(),
            frame: None,
        }
    }

    /// Keep the pane as a readiness wait last saw it.
    fn keep_frame(&mut self, capture: &str) {
        self.frame = Some(frame_evidence(capture));
    }

    /// FAIL > LIMIT > SKIP > PASS, each naming its step.
    fn verdict(&self) -> String {
        if let Some(step) = self.failed {
            format!("FAIL({})", step.name())
        } else if let Some(step) = self.limited {
            format!("LIMIT({})", step.name())
        } else if let Some(step) = self.skipped {
            format!("SKIP({})", step.name())
        } else if self.marks.iter().all(|mark| *mark == Mark::Ok) {
            "PASS".to_owned()
        } else {
            // Never green by omission: a run that stopped without a reason.
            "FAIL(incomplete)".to_owned()
        }
    }

    fn steps(&self) -> String {
        self.marks.iter().map(|mark| mark.glyph()).collect()
    }

    fn is_fail(&self) -> bool {
        self.verdict().starts_with("FAIL")
    }
}

// ─── the gate: cancellation and the one chokepoint for spawning ───────────

struct Shared {
    cancelled: bool,
    step: Step,
    outcome: Outcome,
}

/// Shared state between the driver thread and the thread that bounds it.
///
/// Every child this driver starts goes through [`Gate::spawn`], which checks
/// the flag and spawns under ONE lock, so once [`Gate::cancel`] returns an
/// abandoned worker can start nothing: no launch, no send, no resume, no spend.
#[derive(Clone)]
struct Gate(Arc<Mutex<Shared>>);

impl Gate {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Shared {
            cancelled: false,
            step: Step::Bin,
            outcome: Outcome::new(),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cancel(&self) {
        self.lock().cancelled = true;
    }

    fn cancelled(&self) -> bool {
        self.lock().cancelled
    }

    fn at(&self, step: Step) {
        self.lock().step = step;
    }

    fn step(&self) -> Step {
        self.lock().step
    }

    fn snapshot(&self) -> Outcome {
        self.lock().outcome.clone()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Outcome) -> R) -> R {
        f(&mut self.lock().outcome)
    }

    /// Like [`Self::with`], but only while the run is live.
    fn with_live<R>(&self, f: impl FnOnce(&mut Outcome) -> R) -> Option<R> {
        Some(f(&mut self.lock().outcome))
    }

    /// Start `runner` unless the run was cancelled; the check and the spawn
    /// happen under one lock.
    fn spawn(&self, runner: &mut Runner) -> Result<OwnedChild, Stop> {
        let shared = self.lock();
        if shared.cancelled {
            return Err(Stop::Cancelled);
        }
        let child = runner.spawn();
        drop(shared);
        child.map_err(|_| Stop::Fail("could not start a child".to_owned()))
    }
}

/// Why a step ended without passing.
#[derive(Debug)]
enum Stop {
    /// The step failed; the text is a fixed-word reason.
    Fail(String),
    /// A vendor usage limit is on screen.
    Limit,
    /// The tool is not installed.
    Absent,
    /// The tool cannot be observed at this step.
    Skip(&'static str),
    /// The run was cancelled by its bound; nothing more may happen.
    Cancelled,
}

type Flow = Result<(), Stop>;

struct Done {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn drain<P: std::io::Read + Send + 'static>(pipe: Option<P>) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    if let Some(pipe) = pipe {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.take(1 << 20).read_to_end(&mut buf);
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
    }
    rx
}

/// Run `runner` for at most `limit`: pipes are drained by reader threads, the
/// child is killed at the limit or on cancellation, and a grandchild holding a
/// pipe open cannot keep this call alive past [`DRAIN`].
fn run_bounded(gate: &Gate, runner: &mut Runner, limit: Duration) -> Result<Done, Stop> {
    runner
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = gate.spawn(runner)?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let started = Instant::now();
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(_) => {
                timed_out = true;
                break None;
            }
        }
        if gate.cancelled() || started.elapsed() >= limit {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if gate.cancelled() {
        return Err(Stop::Cancelled);
    }
    Ok(Done {
        code,
        stdout: out.recv_timeout(DRAIN).unwrap_or_default(),
        stderr: err.recv_timeout(DRAIN).unwrap_or_default(),
        timed_out,
    })
}

// ─── pure helpers (pinned by the unit tests below) ────────────────────────

/// A short pseudo-random value: no `rand` dependency, never the same twice in a
/// process, never predictable from the prompt.
fn entropy(salt: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let mut x = u64::try_from(nanos & u128::from(u64::MAX)).unwrap_or(1)
        ^ (u64::from(std::process::id()) << 32)
        ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
    }
    x
}

/// `len` lowercase letters drawn from `seed`.
fn letters(mut seed: u64, len: usize) -> String {
    let mut out = String::new();
    for _ in 0..len {
        let digit = u8::try_from(seed % 26).unwrap_or(0);
        out.push(char::from(b'a' + digit));
        seed /= 26;
        if seed == 0 {
            seed = entropy(u64::try_from(len).unwrap_or(0));
        }
    }
    out
}

/// Whether the answer `token`, ALONE on a row, stands AFTER the last row echoing
/// `marker`: the model's answer, not the echoed prompt and not a status row that
/// happens to hold the same digits. With no echo to anchor on, an answer row that
/// was absent before the send (`baseline` zero) and is present now is the answer.
fn answered(capture: &str, marker: &str, token: &str, baseline: usize) -> bool {
    let rows: Vec<&str> = capture.lines().collect();
    match rows.iter().rposition(|row| row.contains(marker)) {
        Some(at) => rows[at + 1..].iter().any(|row| is_answer_row(row, token)),
        None => baseline == 0 && answer_rows(capture, token) > 0,
    }
}

/// Two random 4-digit operands, 1000..=4999 each, so their sum always has four digits.
fn operands(seed: u64) -> (u32, u32) {
    let pick = |n: u64| 1000 + u32::try_from(n % 4000).unwrap_or(0);
    (pick(seed), pick(seed / 4000))
}

/// The line every prompt the runner sends opens with: a tool that can tell it is
/// being tested has no reason to refuse a harmless question.
const PREFACE: &str = "Automated harness smoke test: harmless, no files and no commands.";

/// The first turn: the sum of two numbers, asked for as bare digits. The sum is
/// never in the prompt, so an echo cannot answer it, and a digit total is one token
/// to write, where eight letters to join came back hyphenated, with a space inside
/// and as a refusal (measured live, codex).
fn turn1_prompt(operands: (u32, u32), marker: &str) -> String {
    let (a, b) = operands;
    format!(
        "{PREFACE} What is {a} + {b}? Reply with only the digits of the sum, \
         no commas and no other text. ({marker})"
    )
}

/// The recall turn: the number back, asked for without naming any number.
fn turn2_prompt(marker: &str) -> String {
    format!(
        "{PREFACE} Which number did you reply with before? Reply with only the digits \
         of that number, no other text. ({marker})"
    )
}

/// Whether `row` is the answer `token` and nothing else: only whitespace, a bullet, a
/// prompt mark or a box edge around it, and at most a closing full stop. A status or
/// footer row that merely holds the same digits ("6912 tokens", a context count) is
/// not an answer, and neither is a number glued to other text.
fn is_answer_row(row: &str, token: &str) -> bool {
    const DECORATION: &[char] = &[
        '•', '●', '⏺', '◦', '○', '▪', '▌', '│', '┃', '>', '❯', '›', '|', '*',
    ];
    let bare = row.trim_matches(|c: char| c.is_whitespace() || DECORATION.contains(&c));
    bare.strip_suffix('.').unwrap_or(bare) == token
}

/// How many rows of `capture` are the answer `token` and nothing else.
fn answer_rows(capture: &str, token: &str) -> usize {
    capture
        .lines()
        .filter(|row| is_answer_row(row, token))
        .count()
}

/// The version a tool printed, reduced to the ONE shape that cannot carry a path
/// or a token: the first word of the first line that begins with 2-4 dotted
/// numbers (`1.2`, `2.1.4`), cut at the first character that is not a digit or a
/// dot. A word that does not begin that way contributes nothing, so a diagnostic
/// that happens to hold a key reads `-`. Character filtering is not redaction,
/// so no other character of the output is ever kept, and no raw stderr or stdout
/// line of a tool is ever printed or stored anywhere else in this file.
fn tidy_version(raw: &str) -> String {
    let line = raw.lines().next().unwrap_or_default();
    line.split_whitespace()
        .find_map(|word| {
            let word = word.strip_prefix('v').unwrap_or(word);
            let numeric: String = word
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let parts: Vec<&str> = numeric.split('.').collect();
            let shaped = (2..=4).contains(&parts.len())
                && parts.iter().all(|part| {
                    (1..=6).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit())
                });
            shaped.then_some(numeric)
        })
        .unwrap_or_else(|| "-".to_owned())
}

/// The tools a run is asked for: `wanted` is a comma list of known names, with
/// no empty item and no repeat. A paid session is never started twice by one
/// typo, and an empty selection is an error instead of a run with no rows.
fn select_tools(wanted: &str) -> Result<Vec<&'static ToolSpec>, String> {
    let mut tools: Vec<&'static ToolSpec> = Vec::new();
    for word in wanted.split(',').map(str::trim) {
        if word.is_empty() {
            return Err("AE_HARNESS_SMOKE_TOOLS holds an empty tool name".to_owned());
        }
        let Some(spec) = TOOLS.iter().find(|spec| spec.name == word) else {
            return Err(format!(
                "AE_HARNESS_SMOKE_TOOLS names an unknown tool: {word}"
            ));
        };
        if tools.iter().any(|seen| seen.name == spec.name) {
            return Err(format!("AE_HARNESS_SMOKE_TOOLS names {word} twice"));
        }
        tools.push(spec);
    }
    Ok(tools)
}

/// The profile command a tool runs. The control's is pinned to the bare shim name
/// whatever the environment says, so an inherited `AE_HARNESS_SMOKE_*_CMD` can
/// never point the "no spend" control at a real binary.
fn command_for(shim: bool, spec: &ToolSpec, overridden: Option<String>) -> String {
    if shim {
        "claude".to_owned()
    } else {
        overridden.unwrap_or_else(|| spec.cmd.to_owned())
    }
}

fn on_path(path: &str, bin: &str) -> bool {
    path.split(':').any(|dir| {
        !dir.is_empty()
            && std::fs::metadata(Path::new(dir).join(bin))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}

/// Every conversation decision a session journal holds, oldest first: the
/// decoded record, or `None` for a record that names a conversation action but
/// breaks its grammar. Lines that are not events, and events that are not
/// conversation decisions, are not decisions.
fn conversation_decisions(journal: &str) -> Vec<Option<ae::conversation::Record>> {
    journal
        .lines()
        .filter_map(|line| ae::events::Event::parse_line(line).ok())
        .filter_map(|event| ae::conversation::decode(&event))
        .map(Result::ok)
        .collect()
}

/// The start keys (`<slot>:<launch_id>:<pid>`) of the decisions already in the
/// journal: what a later decision must not be one of to count as new.
fn decided_starts(journal: &str) -> Vec<String> {
    conversation_decisions(journal)
        .into_iter()
        .flatten()
        .map(|record| record.start_key)
        .collect()
}

/// What the journal says about THIS resume so far.
#[derive(Debug, PartialEq, Eq)]
enum Resumption {
    /// A new, parsed decision to resume `id` for the main seat.
    Proven,
    /// No decision of this resume is journalled yet: the tool start writes it
    /// from the pane, after `ae` has already returned, so the step waits.
    NotYet(&'static str),
    /// A decision that cannot pass, for good.
    Refused(&'static str),
}

/// Judge the journal's NEWEST conversation decision: it must be a resume of `id`
/// made by a tool start the journal did not hold before the stop (`before`), for
/// the main seat. A missing, stale, damaged, fresh-start, lost or
/// other-conversation answer never passes, so ae's metadata alone cannot pass
/// the step; `NotYet` is the only answer worth asking again.
fn judge_resume(before: &[String], journal: &str, id: &str) -> Resumption {
    use ae::conversation::Start;
    let decisions = conversation_decisions(journal);
    let Some(newest) = decisions.last() else {
        return Resumption::NotYet("no conversation decision journalled");
    };
    let Some(record) = newest else {
        return Resumption::Refused("the newest conversation decision is damaged");
    };
    if before.contains(&record.start_key) {
        return Resumption::NotYet("no new conversation decision after the stop");
    }
    if !record.start_key.starts_with("main:") {
        return Resumption::Refused("the new decision is not the main seat's");
    }
    match record.start {
        // `Unproven` is what a tool with no store probe journals for an exact
        // resume; the recall turn is then the proof of the conversation.
        Start::Resumed | Start::Unproven if record.id == id => Resumption::Proven,
        Start::Resumed | Start::Unproven => {
            Resumption::Refused("resume journalled another conversation")
        }
        Start::New => Resumption::Refused("resume journalled a new conversation"),
        Start::Lost(_) => Resumption::Refused("resume journalled a lost conversation"),
    }
}

fn meta_value(session_dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(session_dir.join("meta")).ok()?;
    let prefix = format!("{key}=");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::to_owned)
}

// ─── configuration ────────────────────────────────────────────────────────

fn env_text(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn env_secs(key: &str, default: u64) -> u64 {
    env_text(key)
        .and_then(|value| value.parse().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(default)
}

/// Whether a gate variable's value opens the lane: exactly `1`.
fn opens(value: Option<&str>) -> bool {
    value == Some("1")
}

fn gated() -> bool {
    opens(std::env::var(GATE).ok().as_deref())
}

/// What one run of the smoke is asked to do.
struct Plan {
    tools: Vec<&'static ToolSpec>,
    /// Per-step wait.
    step: Duration,
    /// Hard bound per tool, whatever its steps are doing.
    per_tool: Duration,
    /// Hard bound for the whole run; tools not started read SKIP.
    total: Duration,
    /// The claude account (`config_home`), when not the default.
    claude_home: Option<String>,
    /// The control: a hanging `claude` shim ahead of PATH, a scratch work
    /// directory instead of the locked fixed one, and no watchdog.
    shim: bool,
}

impl Plan {
    /// The plan the environment asks for, or why it asks for nothing runnable.
    /// Built before any work directory, lock or scratch state exists.
    fn live() -> Result<Self, String> {
        let wanted = env_text("AE_HARNESS_SMOKE_TOOLS").unwrap_or_else(|| DEFAULT_TOOLS.to_owned());
        let tools = select_tools(&wanted)?;
        let step = env_secs("AE_HARNESS_SMOKE_STEP_SECS", 60);
        Ok(Self {
            tools,
            step: Duration::from_secs(step),
            per_tool: Duration::from_secs(env_secs(
                "AE_HARNESS_SMOKE_TOOL_SECS",
                step.saturating_mul(12),
            )),
            total: Duration::from_secs(env_secs("AE_HARNESS_SMOKE_TOTAL_SECS", 1_800)),
            claude_home: env_text("AE_HARNESS_SMOKE_CLAUDE_HOME"),
            shim: false,
        })
    }

    fn control() -> Self {
        Self {
            tools: TOOLS.iter().take(1).collect(),
            step: Duration::from_secs(8),
            per_tool: Duration::from_mins(1),
            total: Duration::from_secs(90),
            claude_home: None,
            shim: true,
        }
    }
}

// ─── the fixed work directory and its exclusive lock ──────────────────────

/// `<base>/ae-harness-smoke/work`, held under `<…>/lock` (an atomic `mkdir`).
///
/// The directory is fixed so a vendor's trust list gains one entry rather than
/// one per run. A second run REFUSES while the lock stands; a stale lock is
/// reported and cleared by hand, never stolen. Only the holder removes the
/// work directory, only after proving it is a real directory under this root.
struct WorkDir {
    root: PathBuf,
    work: PathBuf,
}

impl WorkDir {
    fn acquire() -> Result<Self, String> {
        Self::acquire_in(&super::scratch::base())
    }

    /// [`Self::acquire`] under an explicit base, so the lock's rules can be
    /// pinned on a scratch of a test's own instead of the fixed live path.
    fn acquire_in(base: &Path) -> Result<Self, String> {
        let root = base.join("ae-harness-smoke");
        std::fs::create_dir_all(&root).map_err(|why| format!("no run directory: {why}"))?;
        let root =
            std::fs::canonicalize(&root).map_err(|why| format!("no run directory: {why}"))?;
        let lock = root.join("lock");
        if let Err(why) = std::fs::create_dir(&lock) {
            let holder = std::fs::read_to_string(lock.join("owner")).unwrap_or_default();
            return Err(format!(
                "another harness smoke holds {} ({}); {why}. A dead holder is cleared by hand",
                lock.display(),
                holder.trim()
            ));
        }
        let held = Self {
            work: root.join("work"),
            root,
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        std::fs::write(
            held.lock().join("owner"),
            format!("pid {} since {stamp}\n", std::process::id()),
        )
        .map_err(|why| format!("could not record the lock owner: {why}"))?;
        held.clear_work();
        std::fs::create_dir(&held.work).map_err(|why| format!("no work directory: {why}"))?;
        Ok(held)
    }

    fn lock(&self) -> PathBuf {
        self.root.join("lock")
    }

    /// Remove the work directory, only when it is a plain directory directly
    /// under the run root this holder locked.
    fn clear_work(&self) {
        let plain = std::fs::symlink_metadata(&self.work).is_ok_and(|meta| meta.is_dir());
        if plain && self.work.parent() == Some(self.root.as_path()) {
            let _ = std::fs::remove_dir_all(&self.work);
        }
    }

    fn path(&self) -> &Path {
        &self.work
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        self.clear_work();
        let _ = std::fs::remove_file(self.lock().join("owner"));
        let _ = std::fs::remove_dir(self.lock());
        // The run root stays when a report lives in it.
        let _ = std::fs::remove_dir(&self.root);
    }
}

// ─── one tool's scratch world and its driver ──────────────────────────────

/// The world one tool runs in. The scratch is owned by the bounding thread, so
/// its Drop can kill the tmux server a stuck worker is waiting on.
struct Rig {
    /// Held for its Drop: it kills the private tmux server and removes the root.
    _scratch: OwnedScratch,
    root: PathBuf,
}

impl Rig {
    fn new(tool: &ToolSpec) -> Self {
        let scratch = OwnedScratch::root("hs", tool.name);
        let root = scratch.path().to_owned();
        for dir in ["state", "bin", "work"] {
            assert!(
                std::fs::create_dir_all(root.join(dir)).is_ok(),
                "a rig directory"
            );
        }
        Self {
            _scratch: scratch,
            root,
        }
    }
}

/// What a frame must show for a step to go on.
#[derive(Clone, Copy, Debug)]
enum Want {
    /// Quiet enough to type at: the classifier proves `Idle`, or the frame is a
    /// positively recognised FRESH START ([`fresh_claude_start`]). A tool that
    /// has not finished a turn reads `Unknown` to the classifier (measured
    /// live: a fresh claude does, after its trust block), but `Unknown` is also
    /// what an active compaction or any activity row ae has no grammar for
    /// reads, so no other `Unknown` frame is ever quiet.
    Drawn,
    /// The classifier proves a finished turn: `Idle`.
    Idle,
}

impl Want {
    fn holds(self, capture: &str, kind: ToolKind) -> bool {
        let state = harness_state::classify(capture, kind);
        match self {
            Self::Drawn => {
                state == HarnessState::Idle
                    || (state == HarnessState::Unknown
                        && harness_state::read_frame(capture, kind).is_some()
                        && matches!(kind, ToolKind::Claude)
                        && fresh_claude_start(capture))
            }
            Self::Idle => state == HarnessState::Idle,
        }
    }
}

/// Whether `capture` is claude's frame BEFORE ITS FIRST TURN and nothing else:
/// the three logo rows (`Claude Code v…`, the model row, the working-directory
/// row), then the box (rule, bare prompt, rule), then footer rows. Every row is
/// accounted for, so a compaction, a queued dispatch, a spinner or a row this
/// file has never seen anywhere above the box, or any text above the logo,
/// makes it not a fresh start.
fn fresh_claude_start(capture: &str) -> bool {
    let rows: Vec<&str> = capture
        .lines()
        .map(str::trim)
        .filter(|row| !row.is_empty())
        .collect();
    let rule = |row: &str| row.chars().all(|c| c == '─');
    let logo = |row: &str| row.starts_with(['▐', '▛', '▜', '▝', '▘', '▀', '█', '▌']);
    let [first, second, third, top, prompt, bottom, footer @ ..] = rows.as_slice() else {
        return false;
    };
    first.contains("Claude Code v")
        && logo(first)
        && logo(second)
        && logo(third)
        && rule(top)
        && *prompt == "❯"
        && rule(bottom)
        && !footer.is_empty()
        && footer
            .iter()
            .all(|row| row.starts_with('🧠') || row.starts_with('⏵'))
}

/// The driver for one tool: every field is plain data, so it moves to a thread.
struct Run {
    gate: Gate,
    tool: &'static ToolSpec,
    kind: ToolKind,
    session: String,
    state: PathBuf,
    config: PathBuf,
    sock: PathBuf,
    scratch: PathBuf,
    work: PathBuf,
    path_env: String,
    home: String,
    server: ServerId,
    step: Duration,
    watchdog: bool,
    /// The newest capture, kept only to ask ae's own limit detector at a miss.
    last: RefCell<String>,
    token: String,
    operands: (u32, u32),
}

impl Run {
    fn envs(&self, runner: &mut Runner) {
        runner
            .env("HOME", &self.home)
            .env("PATH", &self.path_env)
            .env("AE_HOME", &self.state)
            .env("CONFIG_FILE", &self.config)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.sock)
            .env("TMUX_TMPDIR", &self.scratch)
            .env("PWD", &self.work)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .current_dir(&self.work);
        if !self.watchdog {
            runner.env("AE_NO_AUTOSTART", "1");
        }
    }

    fn ae(&self, args: &[&str]) -> Runner {
        let mut runner = cli::ae();
        self.envs(&mut runner);
        runner.args(args);
        runner
    }

    fn session_dir(&self) -> PathBuf {
        self.state.join("sessions").join(&self.session)
    }

    fn pane(&self) -> Result<String, Stop> {
        meta_value(&self.session_dir(), "main_pane")
            .filter(|pane| !pane.is_empty())
            .ok_or_else(|| Stop::Fail("no pane recorded".to_owned()))
    }

    fn nap(&self, span: Duration) -> Flow {
        let until = Instant::now() + span;
        while Instant::now() < until {
            if self.gate.cancelled() {
                return Err(Stop::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(50).min(span));
        }
        Ok(())
    }

    /// The pane as text, in memory. `None` when tmux cannot answer (the server
    /// or the pane is gone). A vendor limit on screen ends the step at once.
    fn look(&self, pane: &str) -> Result<Option<String>, Stop> {
        if self.gate.cancelled() {
            return Err(Stop::Cancelled);
        }
        let capture = ae::transport::capture_pane(&self.server, pane);
        if let Some(text) = &capture {
            if ae::watchdog::limit_notice(text, self.tool.name).is_some() {
                return Err(Stop::Limit);
            }
            self.last.replace(text.clone());
        }
        Ok(capture)
    }

    /// Re-read until `ready` holds or `bound` passes; `Ok(false)` on timeout.
    fn until(
        &self,
        pane: &str,
        bound: Duration,
        mut ready: impl FnMut(&str) -> bool,
    ) -> Result<bool, Stop> {
        let deadline = Instant::now() + bound;
        loop {
            if let Some(text) = self.look(pane)?
                && ready(&text)
            {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            self.nap(POLL)?;
        }
    }

    /// One look of a READINESS wait (the tool has drawn its box and is quiet):
    /// the frame is published to the shared outcome as the evidence a timeout
    /// needs. It is published as it is seen, not at the miss, because the outer
    /// bound cancels the run and snapshots the outcome before this thread can
    /// react. [`Self::ready_passed`] clears it again.
    fn sight(&self, pane: &str, publish: bool) -> Result<Option<String>, Stop> {
        let capture = self.look(pane)?;
        if publish && let Some(text) = &capture {
            self.gate.with(|out| out.keep_frame(text));
        }
        Ok(capture)
    }

    /// A readiness wait passed: the frames it published on the way are no
    /// evidence of anything, and a passing run keeps none.
    fn ready_passed(&self) {
        self.gate.with(|out| out.frame = None);
    }

    /// A readiness wait begins.
    fn ready_begins(&self) {}

    /// The frame test `want` names, twice [`FRAME_GAP`] apart: a frame that
    /// flickers through the state proves nothing. Waiting for [`Want::Drawn`]
    /// is the readiness wait (the first frame, the frame after a resume and the
    /// quiet before each send): it publishes what it sees. [`Want::Idle`], the
    /// wait after a turn, publishes nothing.
    fn frame_twice(&self, pane: &str, bound: Duration, want: Want) -> Result<bool, Stop> {
        let publish = matches!(want, Want::Drawn);
        if publish {
            self.ready_begins();
        }
        let deadline = Instant::now() + bound;
        loop {
            let first = self.sight(pane, publish)?;
            if first.is_some_and(|text| want.holds(&text, self.kind)) {
                self.nap(FRAME_GAP)?;
                if self
                    .sight(pane, publish)?
                    .is_some_and(|text| want.holds(&text, self.kind))
                {
                    if publish {
                        self.ready_passed();
                    }
                    return Ok(true);
                }
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            self.nap(POLL)?;
        }
    }

    /// The pane stopped changing across two reads: the precondition for typing
    /// at a tool ae has no frame grammar for. A readiness wait too, so it
    /// publishes what it sees like [`Self::frame_twice`] does.
    fn settled(&self, pane: &str, bound: Duration) -> Result<bool, Stop> {
        self.ready_begins();
        let deadline = Instant::now() + bound;
        loop {
            let first = self.sight(pane, true)?;
            self.nap(FRAME_GAP)?;
            let second = self.sight(pane, true)?;
            if first.as_ref().is_some_and(|text| !text.trim().is_empty()) && first == second {
                self.ready_passed();
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
        }
    }

    fn miss(&self, what: &str) -> Stop {
        Self::miss_in(what, self.step)
    }

    fn miss_in(what: &str, bound: Duration) -> Stop {
        Stop::Fail(format!("{what} within {}s", bound.as_secs()))
    }

    /// How long the first idle frame may take. With the watchdog running, a
    /// tool that opens on a folder-trust block (claude) is answered by the
    /// watchdog's own cycle, 60 s apart, so the first answer can come a full
    /// cycle after the prompt is drawn (measured live: 60 s was too short).
    fn ready_bound(&self) -> Duration {
        if self.watchdog {
            self.step.max(WATCHDOG_READY)
        } else {
            self.step
        }
    }

    fn write_config(&self, claude_home: Option<&str>, command: &str) -> Flow {
        let mut text = String::new();
        let mut command = command.to_owned();
        if let Some(home) = claude_home.filter(|_| self.tool.name == "claude") {
            let _ = writeln!(text, "[clients]\nsc = claude config_home={home}\n");
            if let Some(rest) = command.strip_prefix("claude") {
                command = format!("sc{rest}");
            }
        }
        let _ = write!(
            text,
            "[profiles]\nsmoke = \"{command}\"\n\n[roster]\n{SEAT} = smoke\n\n[workspace]\n\
             main = {SEAT}\nlayout = vertical\nchat = off\nquota = off\nauto_trust = on\n\
             idle_nudge_secs = 0\n"
        );
        std::fs::write(&self.config, text).map_err(|_| Stop::Fail("no scratch config".to_owned()))
    }

    // ── the steps ──

    fn step_bin(&self) -> Flow {
        if on_path(&self.path_env, self.tool.name) {
            Ok(())
        } else {
            Err(Stop::Absent)
        }
    }

    fn step_version(&self) -> Flow {
        let mut runner = cli::helper_by_name(self.tool.name);
        self.envs(&mut runner);
        runner.arg("--version");
        let done = run_bounded(&self.gate, &mut runner, Duration::from_secs(10))?;
        if done.timed_out {
            return Err(Stop::Fail("version timed out".to_owned()));
        }
        match done.code {
            Some(0) => {
                let raw = if done.stdout.trim().is_empty() {
                    &done.stderr
                } else {
                    &done.stdout
                };
                let version = tidy_version(raw);
                self.gate.with(|out| out.version = version);
                Ok(())
            }
            code => Err(Stop::Fail(format!("version rc={}", code_word(code)))),
        }
    }

    /// Run a public `ae` verb and require exit 0.
    fn ae_verb(&self, step: &str, args: &[&str], limit: Duration) -> Flow {
        let mut runner = self.ae(args);
        let done = run_bounded(&self.gate, &mut runner, limit)?;
        if done.timed_out {
            return Err(Stop::Fail(format!("{step} timed out")));
        }
        if done.code == Some(0) {
            return Ok(());
        }
        // Only the exit code is kept: a tool's own diagnostic can hold a key.
        Err(Stop::Fail(format!("{step} rc={}", code_word(done.code))))
    }

    fn step_launch(&self) -> Flow {
        let work = self.work.display().to_string();
        self.ae_verb(
            "launch",
            &[&self.session, "--dir", &work, "--no-attach"],
            self.step.max(Duration::from_mins(2)),
        )?;
        self.pane().map(|_| ())
    }

    fn step_ready(&self, pane: &str) -> Flow {
        let bound = self.ready_bound();
        if harness_state::observable(self.kind) {
            if self.frame_twice(pane, bound, Want::Drawn)? {
                Ok(())
            } else {
                Err(Self::miss_in("no idle input frame", bound))
            }
        } else if self.settled(pane, bound)? {
            Err(Stop::Skip("no frame grammar"))
        } else {
            Err(Self::miss_in("pane never settled", bound))
        }
    }

    fn step_id(&self) -> Flow {
        let dir = self.session_dir();
        let deadline = Instant::now() + self.step;
        loop {
            if let Some(id) = meta_value(&dir, "harness_session.main")
                && !id.is_empty()
                && id != "pending"
            {
                // The id is named in the report: only a plain id is ever kept.
                if !ae::launch::id_probeable(&id) || id.len() > 80 {
                    return Err(Stop::Fail("harness session id is malformed".to_owned()));
                }
                self.gate
                    .with(|out| out.strays.push(format!("{} {id}", self.tool.name)));
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(self.miss("no harness session id"));
            }
            self.nap(POLL)?;
        }
    }

    /// Send one turn through the `send` helper and wait for the model's answer.
    fn turn(&self, pane: &str, prompt: &str, marker: &str) -> Flow {
        // Nothing is typed into a tool that is still working: ae's own launch turn
        // (codex's registration) may be in flight when the id lands, and a prompt
        // sent into it was folded into that turn and answered as something else
        // (measured live, twice). Idle twice first; the turn is counted after.
        let quiet = if harness_state::observable(self.kind) {
            self.frame_twice(pane, self.step, Want::Drawn)?
        } else {
            self.settled(pane, self.step)?
        };
        if !quiet {
            return Err(self.miss("frame never quiet before the send"));
        }
        let baseline = answer_rows(&self.last.borrow(), &self.token);
        self.gate.with(|out| out.runner_turns += 1);
        let helper = self.session_dir().join("send");
        let mut runner = cli::helper(&helper);
        self.envs(&mut runner);
        runner.args([SEAT, prompt]);
        let done = run_bounded(&self.gate, &mut runner, self.step * 2)?;
        if done.timed_out {
            return Err(Stop::Fail("send timed out".to_owned()));
        }
        if done.code != Some(0) {
            return Err(Stop::Fail(format!("send rc={}", code_word(done.code))));
        }
        let token = self.token.clone();
        let seen = self.until(pane, self.step, |text| {
            answered(text, marker, &token, baseline)
        })?;
        if seen {
            Ok(())
        } else {
            Err(self.miss("no answer"))
        }
    }

    fn step_turn1(&self, pane: &str) -> Flow {
        let marker = format!("ref {}", letters(entropy(1), 6));
        self.turn(pane, &turn1_prompt(self.operands, &marker), &marker)
    }

    fn step_idle(&self, pane: &str) -> Flow {
        if !harness_state::observable(self.kind) {
            return Err(Stop::Skip("no frame grammar"));
        }
        if self.frame_twice(pane, self.step, Want::Idle)? {
            Ok(())
        } else {
            Err(self.miss("frame not idle"))
        }
    }

    /// The journal of the scratch session, or empty when it cannot be read: an
    /// unreadable journal proves nothing, and every consumer treats empty so.
    fn journal(&self) -> String {
        std::fs::read_to_string(self.session_dir().join("events.jsonl")).unwrap_or_default()
    }

    /// `ae stop`, then the product's own STRICT absence proof (the server
    /// answered without the session, or reported a clean exit). A pane that
    /// merely fails to capture is not proof: only `Absent` passes, and a proof
    /// that stays unknown or present for the whole window is a FAIL.
    fn step_stop(&self) -> Flow {
        self.ae_verb("stop", &["stop", &self.session], Duration::from_mins(1))?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match ae::transport::verify_session_absent(&self.server, &self.session) {
                ae::tmux::StopProbe::Absent => return Ok(()),
                ae::tmux::StopProbe::Present if Instant::now() >= deadline => {
                    return Err(Stop::Fail("session still present after stop".to_owned()));
                }
                ae::tmux::StopProbe::Unknown if Instant::now() >= deadline => {
                    return Err(Stop::Fail("absence after stop not proven".to_owned()));
                }
                _ => self.nap(POLL)?,
            }
        }
    }

    fn step_resume(&self, id: &str, before: &[String]) -> Flow {
        self.ae_verb(
            "resume",
            &[&self.session, "--no-attach"],
            self.step.max(Duration::from_mins(2)),
        )?;
        let now = meta_value(&self.session_dir(), "harness_session.main").unwrap_or_default();
        if now != id {
            return Err(Stop::Fail("resume changed the session id".to_owned()));
        }
        let deadline = Instant::now() + self.step;
        loop {
            match judge_resume(before, &self.journal(), id) {
                Resumption::Proven => break,
                Resumption::Refused(why) => return Err(Stop::Fail(why.to_owned())),
                Resumption::NotYet(why) if Instant::now() >= deadline => {
                    return Err(Stop::Fail(why.to_owned()));
                }
                Resumption::NotYet(_) => self.nap(POLL)?,
            }
        }
        let pane = self.pane()?;
        self.last.replace(String::new());
        if harness_state::observable(self.kind) {
            if self.frame_twice(&pane, self.step, Want::Drawn)? {
                Ok(())
            } else {
                Err(self.miss("no idle input frame after resume"))
            }
        } else if self.settled(&pane, self.step)? {
            Ok(())
        } else {
            Err(self.miss("pane never settled after resume"))
        }
    }

    fn step_turn2(&self, pane: &str) -> Flow {
        let marker = format!("ref {}", letters(entropy(2), 6));
        self.turn(pane, &turn2_prompt(&marker), &marker)
    }

    /// Whether the vendor's usage limit is on the pane RIGHT NOW (one fresh
    /// in-memory read, no pane text kept): the question every FAIL asks before
    /// it is believed, because a limit reached during a send, a launch or an id
    /// wait would otherwise read as a broken adapter row.
    fn limit_showing(&self) -> bool {
        if self.gate.cancelled() {
            return false;
        }
        self.pane()
            .ok()
            .and_then(|pane| ae::transport::capture_pane(&self.server, &pane))
            .is_some_and(|text| ae::watchdog::limit_notice(&text, self.tool.name).is_some())
    }

    /// Run one step, record its mark and carry its verdict.
    fn run_step(&self, step: Step, f: impl FnOnce() -> Flow) -> Flow {
        self.gate.at(step);
        let result = match f() {
            Err(Stop::Fail(_)) if self.limit_showing() => Err(Stop::Limit),
            other => other,
        };
        let (mark, reason) = match &result {
            Ok(()) => (Mark::Ok, None),
            Err(Stop::Fail(why)) => (Mark::Fail, Some(why.clone())),
            Err(Stop::Limit) => (Mark::Limit, Some("vendor usage limit on screen".to_owned())),
            Err(Stop::Absent) => (Mark::Skip, Some("not on PATH".to_owned())),
            Err(Stop::Skip(why)) => (Mark::Skip, Some((*why).to_owned())),
            Err(Stop::Cancelled) => (Mark::Fail, Some("outer bound reached".to_owned())),
        };
        self.gate.with(|out| {
            out.marks[step.index()] = mark;
            if out.detail.is_empty()
                && let Some(reason) = reason
            {
                out.detail = reason;
            }
            match &result {
                Err(Stop::Fail(_) | Stop::Cancelled) => {
                    out.failed.get_or_insert(step);
                }
                Err(Stop::Limit) => {
                    out.limited.get_or_insert(step);
                }
                Err(Stop::Absent | Stop::Skip(_)) => {
                    out.skipped.get_or_insert(step);
                }
                Ok(()) => {}
            }
        });
        result
    }

    /// A step whose SKIP does not end the tool: the run goes on past it.
    fn soft(&self, step: Step, f: impl FnOnce() -> Flow) -> Flow {
        match self.run_step(step, f) {
            Err(Stop::Skip(_)) => Ok(()),
            other => other,
        }
    }

    /// The whole sequence. The first FAIL, LIMIT, absence or cancellation ends
    /// the tool; a SKIP at `ready` or `idle` does not.
    fn drive(&self, claude_home: Option<&str>, command: &str) {
        let flow = self.sequence(claude_home, command);
        if matches!(flow, Err(Stop::Cancelled)) {
            return;
        }
        // A session left running by an early exit is stopped through the
        // product, once; the rig's Drop then removes whatever remains.
        if self.session_dir().join("meta").exists() && !self.gate.cancelled() {
            let _ = run_bounded(
                &self.gate,
                &mut self.ae(&["stop", &self.session]),
                Duration::from_secs(30),
            );
        }
    }

    fn sequence(&self, claude_home: Option<&str>, command: &str) -> Flow {
        self.write_config(claude_home, command)?;
        self.run_step(Step::Bin, || self.step_bin())?;
        self.run_step(Step::Version, || self.step_version())?;
        self.run_step(Step::Launch, || self.step_launch())?;
        let pane = self.pane()?;
        self.soft(Step::Ready, || self.step_ready(&pane))?;
        self.run_step(Step::Id, || self.step_id())?;
        let id = meta_value(&self.session_dir(), "harness_session.main").unwrap_or_default();
        self.run_step(Step::Turn1, || self.step_turn1(&pane))?;
        self.soft(Step::Idle, || self.step_idle(&pane))?;
        let before = decided_starts(&self.journal());
        self.run_step(Step::Stop, || self.step_stop())?;
        self.run_step(Step::Resume, || self.step_resume(&id, &before))?;
        let pane = self.pane()?;
        self.run_step(Step::Turn2, || self.step_turn2(&pane))
    }
}

fn code_word(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_owned(), |code| code.to_string())
}

// ─── the bounding thread, the report, the two tests ───────────────────────

struct Row {
    /// The scratch root the run used (gone after it).
    root: PathBuf,
    tool: String,
    outcome: Outcome,
    cleanup: Duration,
    wall: Duration,
}

fn shim_script() -> &'static str {
    // Answers --version at once, then hangs forever at startup: the failure the
    // control exists to prove is bounded and named.
    "#!/usr/bin/perl\n\
     if (@ARGV && $ARGV[0] eq '--version') { print \"claude-hang-shim 0.0.0\\n\"; exit 0; }\n\
     $| = 1;\n\
     while (1) { sleep 1; }\n"
}

/// Run one tool for at most `budget` (its own bound, already clamped to what is
/// left of the whole run's), then clean its world up.
fn run_tool(
    plan: &Plan,
    spec: &'static ToolSpec,
    shared_work: Option<&Path>,
    budget: Duration,
) -> Row {
    let rig = Rig::new(spec);
    let real_path = std::env::var("PATH").unwrap_or_default();
    let path_env = if plan.shim {
        let shim = rig.root.join("bin").join("claude");
        assert!(
            std::fs::write(&shim, shim_script()).is_ok(),
            "the hang shim"
        );
        assert!(
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable hang shim"
        );
        format!("{}:{real_path}", rig.root.join("bin").display())
    } else {
        real_path
    };
    let work = shared_work.map_or_else(|| rig.root.join("work"), Path::to_path_buf);
    let sock = rig.root.join("sock");
    let command = command_for(
        plan.shim,
        spec,
        env_text(&format!(
            "AE_HARNESS_SMOKE_{}_CMD",
            spec.name.to_ascii_uppercase()
        )),
    );
    let gate = Gate::new();
    let operands = operands(entropy(7));
    let sum = operands.0 + operands.1;
    let run = Run {
        gate: gate.clone(),
        tool: spec,
        kind: ToolKind::from_binary_name(spec.name),
        session: format!("hs-{}", spec.name),
        state: rig.root.join("state"),
        config: rig.root.join("config"),
        server: ServerId::Selected(Selector::Socket(sock.clone())),
        sock,
        scratch: rig.root.clone(),
        work,
        path_env,
        home: std::env::var("HOME").unwrap_or_default(),
        step: plan.step,
        watchdog: !plan.shim,
        last: RefCell::new(String::new()),
        token: sum.to_string(),
        operands,
    };
    // ae pastes its own registration turn where the adapter has one: once on
    // create, once more on resume.
    let ae_turn =
        u8::from(!ae::launch::initial_prompt_for(run.kind, Path::new("/"), SEAT).is_empty());
    let claude_home = plan.claude_home.clone();
    let started = Instant::now();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        run.drive(claude_home.as_deref(), &command);
        let _ = done_tx.send(());
    });
    let finished = done_rx.recv_timeout(budget);
    let wall = started.elapsed();
    // From here nothing the worker still holds may start a child or spend.
    gate.cancel();
    let mut outcome = gate.snapshot();
    match finished {
        Ok(()) => {}
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let step = gate.step();
            outcome.marks[step.index()] = Mark::Fail;
            outcome.failed.get_or_insert(step);
            "outer bound reached".clone_into(&mut outcome.detail);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let step = gate.step();
            outcome.marks[step.index()] = Mark::Fail;
            outcome.failed.get_or_insert(step);
            "driver panicked".clone_into(&mut outcome.detail);
        }
    }
    // ATTEMPTS of ae's own launch turn, an upper bound: one when the launch was
    // tried, one more when the resume was. A delivery is not observed here.
    outcome.ae_turns = ae_turn
        * (u8::from(outcome.marks[Step::Launch.index()] != Mark::NotReached)
            + u8::from(outcome.marks[Step::Resume.index()] != Mark::NotReached));
    let root = rig.root.clone();
    let cleanup_start = Instant::now();
    drop(rig);
    let cleanup = cleanup_start.elapsed();
    if finished.is_ok() {
        let _ = worker.join();
    }
    Row {
        root,
        tool: spec.name.to_owned(),
        outcome,
        cleanup,
        wall,
    }
}

fn skipped_row(spec: &ToolSpec, why: &str) -> Row {
    let mut outcome = Outcome::new();
    outcome.skipped = Some(Step::Bin);
    outcome.marks[Step::Bin.index()] = Mark::Skip;
    why.clone_into(&mut outcome.detail);
    Row {
        root: PathBuf::new(),
        tool: spec.name.to_owned(),
        outcome,
        cleanup: Duration::ZERO,
        wall: Duration::ZERO,
    }
}

fn run_plan(plan: &Plan) -> Vec<Row> {
    let work = (!plan.shim).then(|| match WorkDir::acquire() {
        Ok(work) => work,
        Err(why) => panic!("harness smoke refused to start: {why}"),
    });
    let started = Instant::now();
    let mut rows = Vec::new();
    for spec in &plan.tools {
        let left = plan.total.saturating_sub(started.elapsed());
        if left.is_zero() {
            rows.push(skipped_row(
                spec,
                "total bound reached before this tool started",
            ));
            continue;
        }
        // A tool started with a second left gets a second, not its full bound.
        let budget = plan.per_tool.min(left);
        rows.push(run_tool(
            plan,
            spec,
            work.as_ref().map(WorkDir::path),
            budget,
        ));
    }
    drop(work);
    rows
}

fn render(rows: &[Row]) -> (String, String) {
    let mut table = String::from("tool | version | steps | turns | verdict | detail\n");
    let mut tsv = String::from(
        "tool\tversion\tsteps\trunner\tae\tverdict\tdetail\twall_s\tcleanup_s\tstrays\n",
    );
    for row in rows {
        let outcome = &row.outcome;
        let detail: String = outcome
            .detail
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(80)
            .collect();
        let _ = writeln!(
            table,
            "{} | {} | {} | runner={} ae={} | {} | {}",
            row.tool,
            outcome.version,
            outcome.steps(),
            outcome.runner_turns,
            outcome.ae_turns,
            outcome.verdict(),
            detail
        );
        let strays = if outcome.strays.is_empty() {
            "-".to_owned()
        } else {
            outcome.strays.join(";")
        };
        let _ = writeln!(
            tsv,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{strays}",
            row.tool,
            outcome.version,
            outcome.steps(),
            outcome.runner_turns,
            outcome.ae_turns,
            outcome.verdict(),
            detail,
            row.wall.as_secs(),
            row.cleanup.as_secs_f32()
        );
    }
    let steps: Vec<&str> = STEPS.iter().map(|step| step.name()).collect();
    let _ = writeln!(
        table,
        "steps: {} ('+' ok, 'X' fail, 's' skip, 'L' limit, '-' not reached)",
        steps.join(" ")
    );
    let _ = writeln!(
        table,
        "runner= turns this runner sent; ae= launch/resume attempts that carry ae's own \
         registration turn (an upper bound: its delivery is not observed)"
    );
    for row in rows {
        for stray in &row.outcome.strays {
            let _ = writeln!(table, "stray vendor conversation: {stray}");
        }
    }
    (table, tsv)
}

/// Write the TSV report into `dir`; an error names why, never the content.
fn write_report(dir: &Path, stamp: u64, tsv: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|why| format!("cannot create the report directory: {why}"))?;
    let file = dir.join(format!("report-{stamp}.tsv"));
    std::fs::write(&file, tsv).map_err(|why| format!("cannot write the report file: {why}"))?;
    Ok(file)
}

/// The most rows, characters (Unicode scalars, not terminal cells) per row and
/// bytes of a pane frame the report keeps.
const FRAME_ROWS: usize = 80;
const FRAME_CHARS: usize = 240;
const FRAME_BYTES: usize = 16 * 1024;

/// A pane frame fit for a file a human opens in a terminal: every control
/// character but the row break and the tab renders as U+FFFD (the rule of
/// `board::terminal_text`, which is crate-private and so out of this test
/// crate's reach), each row is clipped, and the newest rows are kept under the
/// row and byte bounds. A capture with no text says so rather than reading blank.
fn frame_evidence(capture: &str) -> String {
    let neutral: String = capture
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\t' || !ch.is_control() {
                ch
            } else {
                '\u{FFFD}'
            }
        })
        .collect();
    let mut rows: Vec<String> = neutral
        .lines()
        .map(|row| {
            if row.chars().count() > FRAME_CHARS {
                row.chars()
                    .take(FRAME_CHARS - 1)
                    .chain(std::iter::once('…'))
                    .collect()
            } else {
                row.to_owned()
            }
        })
        .collect();
    if rows.len() > FRAME_ROWS {
        rows.drain(..rows.len() - FRAME_ROWS);
    }
    let mut bytes = 0;
    let mut first = rows.len();
    for (at, row) in rows.iter().enumerate().rev() {
        bytes += row.len() + 1;
        if bytes > FRAME_BYTES {
            break;
        }
        first = at;
    }
    rows.drain(..first);
    if rows.iter().all(|row| row.trim().is_empty()) {
        return "(no text captured)\n".to_owned();
    }
    let mut kept = rows.join("\n");
    kept.push('\n');
    kept
}

/// Write one kept frame into `dir`, owner-only; an error names why, never the
/// content. The name is created exclusively, so an existing file is never
/// overwritten and a link is never followed.
fn write_frame(dir: &Path, stamp: u64, tool: &str, frame: &str) -> Result<PathBuf, String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::create_dir_all(dir)
        .map_err(|why| format!("cannot create the report directory: {why}"))?;
    let file = dir.join(format!("frame-{stamp}-{tool}.txt"));
    let cannot = |why: std::io::Error| format!("cannot write the frame file for {tool}: {why}");
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
        .map_err(cannot)?;
    out.write_all(frame.as_bytes()).map_err(cannot)?;
    Ok(file)
}

/// Keep every frame the rows carry: the lines to print, and the problems. With
/// no report directory nothing is written and the line says how to keep one;
/// the frame itself is never printed.
fn keep_frames(dir: Option<&Path>, stamp: u64, rows: &[Row]) -> (Vec<String>, Vec<String>) {
    let mut notes = Vec::new();
    let mut problems = Vec::new();
    for row in rows {
        let Some(frame) = &row.outcome.frame else {
            continue;
        };
        // A readiness wait that passes clears its frame, so one left on a PASS row
        // is a wiring fault, never evidence.
        if row.outcome.verdict() == "PASS" {
            problems.push(format!("{} kept a frame but read PASS", row.tool));
            continue;
        }
        match dir {
            Some(dir) => match write_frame(dir, stamp, &row.tool, frame) {
                Ok(file) => notes.push(format!("frame: {}", file.display())),
                Err(why) => problems.push(format!("{} frame not kept: {why}", row.tool)),
            },
            None => notes.push(format!(
                "frame: {} kept none; set AE_HARNESS_SMOKE_REPORT to keep the last frame of a \
                 ready timeout",
                row.tool
            )),
        }
    }
    (notes, problems)
}

/// Print the table, keep the report, and fail the test on any FAIL or on a
/// report or a frame that was asked for and could not be written.
fn conclude(rows: &[Row]) {
    let (table, tsv) = render(rows);
    println!("\n{table}");
    let mut problems: Vec<String> = Vec::new();
    let dir = env_text("AE_HARNESS_SMOKE_REPORT").map(PathBuf::from);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    if let Some(dir) = &dir {
        match write_report(dir, stamp, &tsv) {
            Ok(file) => println!("report: {}", file.display()),
            Err(why) => problems.push(format!("AE_HARNESS_SMOKE_REPORT was set but {why}")),
        }
    }
    let (notes, trouble) = keep_frames(dir.as_deref(), stamp, rows);
    for note in notes {
        println!("{note}");
    }
    problems.extend(trouble);
    problems.extend(
        rows.iter()
            .filter(|row| row.outcome.is_fail())
            .map(|row| format!("{} {}", row.tool, row.outcome.verdict())),
    );
    assert!(
        problems.is_empty(),
        "harness smoke FAIL: {}",
        problems.join(", ")
    );
}

/// The suite's runners register their scratch with a reaper that lives under
/// `TMUX_TMPDIR`; `just harness-smoke` sets it, and a bare nextest run has none.
fn require_lane() {
    assert!(
        env_text("TMUX_TMPDIR").is_some(),
        "harness smoke: TMUX_TMPDIR is unset; run it through `just harness-smoke`"
    );
}

/// The live smoke over the real tools; see the module docs.
#[test]
#[ignore = "live: starts real harness binaries and spends a few short model turns; needs AE_HARNESS_SMOKE=1, installed tools and their logins (just harness-smoke)"]
fn live_harness_smoke() {
    if !gated() {
        eprintln!("harness smoke: {GATE}=1 is not set — nothing attempted");
        return;
    }
    require_lane();
    let plan = Plan::live().unwrap_or_else(|why| panic!("harness smoke refused to start: {why}"));
    conclude(&run_plan(&plan));
}

/// The control: a `claude` that answers `--version` and then hangs must read
/// FAIL at `ready`, inside the bound, with the scratch and the server gone.
#[test]
#[ignore = "live: starts a real tmux server and a hanging claude shim, no vendor and no spend; needs AE_HARNESS_SMOKE=1 (just harness-smoke control)"]
fn live_harness_smoke_hang_control() {
    if !gated() {
        eprintln!("harness smoke: {GATE}=1 is not set — nothing attempted");
        return;
    }
    require_lane();
    let plan = Plan::control();
    let rows = run_plan(&plan);
    let (table, _) = render(&rows);
    println!("\n{table}");
    let row = &rows[0];
    let outcome = &row.outcome;
    assert_eq!(
        outcome.verdict(),
        "FAIL(ready)",
        "the hang must be named at the step it hung\n{table}"
    );
    assert_eq!(
        &outcome.marks[..4],
        &[Mark::Ok, Mark::Ok, Mark::Ok, Mark::Fail],
        "version answered, launch worked, ready failed\n{table}"
    );
    // A smoke that fails blind is the gap: the hang must leave its last frame.
    let frame = outcome.frame.as_deref().unwrap_or("");
    assert!(
        !frame.is_empty(),
        "a ready timeout keeps the last frame\n{table}"
    );
    assert!(
        frame
            .chars()
            .all(|c| c == '\n' || c == '\t' || !c.is_control()),
        "the kept frame is neutralised: {frame:?}"
    );
    assert!(
        row.wall < Duration::from_secs(45),
        "the control must not hang: {:?}",
        row.wall
    );
    // Nothing of the run is left: no scratch root, no process whose argv names it
    // (the private tmux server, the hanging shim).
    assert!(
        !row.root.exists(),
        "the scratch root is gone: {}",
        row.root.display()
    );
    let gate = Gate::new();
    let mut probe = cli::helper_by_name("pgrep");
    probe.args(["-f", &row.root.display().to_string()]);
    let done = run_bounded(&gate, &mut probe, Duration::from_secs(10))
        .unwrap_or_else(|_| panic!("pgrep answers"));
    assert_eq!(
        done.code,
        Some(1),
        "a process still names the scratch root: {}",
        done.stdout
    );
    println!(
        "hang control: wall {:?}, cleanup {:?}",
        row.wall, row.cleanup
    );

    // The same hang under an OUTER bound shorter than the ready wait: the run is
    // cancelled mid-wait and snapshotted before the worker can react, so the
    // frame must already be published, and the verdict still names the step.
    let mut tight = Plan::control();
    tight.per_tool = Duration::from_secs(6);
    let rows = run_plan(&tight);
    let (table, _) = render(&rows);
    println!("\n{table}");
    let outcome = &rows[0].outcome;
    assert_eq!(
        outcome.verdict(),
        "FAIL(ready)",
        "the outer bound names the step it cut\n{table}"
    );
    assert_eq!(outcome.detail, "outer bound reached", "{table}");
    let frame = outcome.frame.as_deref().unwrap_or("");
    assert!(
        !frame.is_empty(),
        "an outer bound during ready keeps the last frame\n{table}"
    );
    assert!(!rows[0].root.exists(), "the scratch root is gone");
}

#[cfg(test)]
mod pins {
    use super::*;

    #[test]
    fn the_sum_is_asked_for_and_never_in_the_prompt() {
        let marker = "ref abcdef";
        for step in 0..20_000_u64 {
            let (a, b) = operands(step.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ entropy(3));
            assert!((1000..=4999).contains(&a) && (1000..=4999).contains(&b));
            let sum = (a + b).to_string();
            assert_eq!(sum.len(), 4, "{a} + {b}");
            let prompt = turn1_prompt((a, b), marker);
            assert!(
                !prompt.contains(&sum),
                "the prompt must not hold the answer: {prompt}"
            );
            assert!(
                prompt.contains(&a.to_string()) && prompt.contains(&b.to_string()),
                "{prompt}"
            );
        }
        let prompt = turn1_prompt((1234, 5678), marker);
        assert!(
            prompt.starts_with(PREFACE),
            "benign preface first: {prompt}"
        );
        assert!(
            prompt.contains("sum") && prompt.contains("digits"),
            "{prompt}"
        );
        assert!(
            !prompt.to_ascii_lowercase().contains("letter"),
            "no letter joining is asked for: {prompt}"
        );
        assert!(prompt.ends_with("(ref abcdef)"), "{prompt}");
    }

    #[test]
    fn the_recall_turn_names_the_number_and_holds_none() {
        let prompt = turn2_prompt("ref abcdef");
        assert!(prompt.starts_with(PREFACE), "{prompt}");
        assert!(
            prompt.contains("number") && prompt.contains("digits"),
            "{prompt}"
        );
        assert!(
            !prompt.chars().any(|c| c.is_ascii_digit()),
            "a recall prompt that held a number could answer itself: {prompt}"
        );
    }

    #[test]
    fn only_a_row_that_is_the_answer_alone_counts_as_the_sum() {
        for row in [
            "● 6912",
            "• 6912",
            "⏺ 6912",
            "6912",
            "6912.",
            "│ 6912 │",
            "> 6912",
            "  ●   6912  ",
        ] {
            assert_eq!(answer_rows(row, "6912"), 1, "{row}");
        }
        for row in [
            "● 16912",
            "● 69123",
            "● a6912",
            "● 6912th",
            "● 6 912",
            "● 6,912",
            "6912 tokens · idle",
            "gpt-6 · 6912 left",
            "● sum: 6912",
            "● 6912 + 1",
        ] {
            assert_eq!(answer_rows(row, "6912"), 0, "{row}");
        }
        assert_eq!(answer_rows("● 6912\n\n6912 tokens\n• 6912\n", "6912"), 2);
    }

    #[test]
    fn a_status_row_carrying_the_digits_is_never_the_answer_to_either_turn() {
        let echo1 = "> Automated harness smoke test. What is 3456 + 3456? (ref abcdef)\n";
        let footer = "6912 tokens · idle\n";
        // Turn 1, anchored on its echo: baseline zero and baseline one alike.
        for baseline in [0, 1] {
            assert!(!answered(
                &format!("{echo1}{footer}"),
                "ref abcdef",
                "6912",
                baseline
            ));
        }
        // Turn 1 with no echo to anchor on (scrolled away or wrapped).
        assert!(!answered(footer, "ref abcdef", "6912", 0));
        // Turn 2: the replayed history holds the real answer above the new echo,
        // and a status row below the echo must not stand in for a new one.
        let echo2 = "> Automated harness smoke test. Which number did you reply with before? (ref uvwxyz)\n";
        let history = format!("{echo1}● 6912\n{echo2}{footer}");
        assert!(!answered(&history, "ref uvwxyz", "6912", 1));
        // A real answer row still counts when a status row follows it.
        assert!(answered(
            &format!("{echo1}● 6912\n{footer}"),
            "ref abcdef",
            "6912",
            0
        ));
        assert!(answered(
            &format!("{history}● 6912\n{footer}"),
            "ref uvwxyz",
            "6912",
            1
        ));
    }

    /// The frame a real claude 2.1.314 draws once its trust block is answered
    /// and before any turn (captured live; the cwd row is a placeholder).
    const FRESH: &str = " ▐▛███▛█   Claude Code v2.1.314\n\
                         ▝▜██████▀  Haiku 5.5 with low effort · Claude Pro\n \
                         ▝▝   ▝▝   /work\n\
                         ────────────────────────────────────────────────\n\
                         ❯ \n\
                         ────────────────────────────────────────────────\n  \
                         🧠 Haiku 5.5 (low)  📁 work  $0.00\n  \
                         ⏵⏵ auto mode on (shift+tab to cycle)\n";

    /// SYNTHETIC, as `harness_state`'s own tests do it: the one manual `⏸` footer
    /// row of a captured frame swapped for the bypass footer a seat draws.
    fn bypass(frame: &str) -> String {
        let manual: Vec<&str> = frame
            .lines()
            .filter(|row| row.trim_start().starts_with('⏸'))
            .collect();
        assert_eq!(manual.len(), 1, "exactly one manual footer row");
        frame.replacen(manual[0], "  ⏵⏵ bypass permissions on", 1)
    }

    #[test]
    fn a_fresh_claude_start_is_ready_but_not_yet_idle() {
        let claude = ToolKind::Claude;
        assert_eq!(
            harness_state::classify(FRESH, claude),
            HarnessState::Unknown,
            "premise: the classifier has no verdict for a frame before any turn"
        );
        assert!(Want::Drawn.holds(FRESH, claude), "the fresh start is ready");
        assert!(
            !Want::Idle.holds(FRESH, claude),
            "idle needs a finished turn, which a fresh frame has not had"
        );
        assert!(fresh_claude_start(FRESH));
    }

    #[test]
    fn a_finished_turn_is_ready_and_idle() {
        // Captured frames the classifier proves Idle: a finished turn, and a
        // resumed conversation. Both are quiet by the classifier alone.
        for idle in [
            include_str!("../fixtures/harness-state/claude-idle-167x40.txt"),
            include_str!("../fixtures/harness-state/claude-resumed-idle-149x37.txt"),
        ] {
            assert!(Want::Drawn.holds(idle, ToolKind::Claude));
            assert!(Want::Idle.holds(idle, ToolKind::Claude));
        }
    }

    #[test]
    fn no_other_unknown_frame_is_ready() {
        let claude = ToolKind::Claude;
        // Active compaction, and a dispatch left in or queued above the box: the
        // classifier says Unknown for each, and none may authorise a send.
        let compacting = bypass(include_str!(
            "../fixtures/harness-state/claude-compacting-2.1.280.txt"
        ));
        let staged = bypass(include_str!(
            "../fixtures/claude-composer/claude-compact-staged-2.1.280.txt"
        ));
        let queued = bypass(include_str!(
            "../fixtures/claude-composer/claude-compact-queued-2.1.280.txt"
        ));
        // The compacting frame is a RECOGNISED input frame the classifier has no
        // verdict for: the exact frame the readiness test must refuse.
        assert!(
            harness_state::read_frame(&compacting, claude).is_some(),
            "premise: the input frame is recognised"
        );
        for (name, frame) in [
            ("compacting", &compacting),
            ("staged", &staged),
            ("queued", &queued),
        ] {
            assert_eq!(
                harness_state::classify(frame, claude),
                HarnessState::Unknown,
                "premise ({name}): no verdict"
            );
            assert!(!Want::Drawn.holds(frame, claude), "{name} is not quiet");
            assert!(!Want::Idle.holds(frame, claude), "{name} is not idle");
        }
        // An activity row the grammar has never seen, in every place a row can
        // sit between the logo and the box, above the logo, and among the footers.
        let unfamiliar = "◈ Reticulating splines (4s)";
        for frame in [
            FRESH.replacen("────", &format!("{unfamiliar}\n────"), 1),
            format!("{unfamiliar}\n{FRESH}"),
            FRESH.replacen("  ⏵⏵", &format!("  {unfamiliar}\n  ⏵⏵"), 1),
            FRESH.replacen(
                " ▝▝   ▝▝   /work\n",
                " ▝▝   ▝▝   /work\n▶ Running hook…\n",
                1,
            ),
        ] {
            assert!(!fresh_claude_start(&frame), "{frame}");
            assert!(!Want::Drawn.holds(&frame, claude), "{frame}");
        }
        // A running turn, nothing drawn, and the fresh frame on another tool.
        let busy = FRESH.replacen("────", "✻ Cooking… (3s · esc to interrupt)\n────", 1);
        assert!(
            !Want::Drawn.holds(&busy, claude),
            "a running turn is not ready"
        );
        assert!(!Want::Idle.holds(&busy, claude));
        assert!(!Want::Drawn.holds("nothing drawn here\n", claude));
        assert!(!Want::Drawn.holds(FRESH, ToolKind::Muse));
    }

    #[test]
    fn only_a_row_after_the_echoed_marker_counts_as_the_answer() {
        let marker = "ref abc123";
        let echo = "> Join these letters: a-b-c (ref abc123)\n";
        // The echo alone is not an answer, even though it holds the letters.
        assert!(!answered(echo, marker, "abc", 0));
        // The token on a row below the echo is.
        let reply = format!("{echo}● abc\n> ");
        assert!(answered(&reply, marker, "abc", 0));
        // Wrapped echo rows (marker last) stay above the answer.
        let wrapped = "> Join these\n  letters: a-b-c (ref abc123)\n● abc\n";
        assert!(answered(wrapped, marker, "abc", 0));
    }

    #[test]
    fn a_token_above_the_latest_echo_is_history_not_an_answer() {
        // After a resume the replayed history holds turn1's token ABOVE turn2's
        // echo; only a token below that echo answers turn2.
        let history = "> first (ref one111)\n● abc\n> which word (ref two222)\n";
        assert!(!answered(history, "ref two222", "abc", 1));
        let answered_now = format!("{history}● abc\n");
        assert!(answered(&answered_now, "ref two222", "abc", 1));
    }

    #[test]
    fn without_an_echo_a_new_token_answers_only_from_a_clean_baseline() {
        assert!(answered("● abc\n", "ref nomark", "abc", 0));
        assert!(!answered("● abc\n", "ref nomark", "abc", 1));
        assert!(!answered("nothing\n", "ref nomark", "abc", 0));
    }

    #[test]
    fn the_version_is_one_numeric_token_and_nothing_else_survives() {
        assert_eq!(tidy_version("2.1.4 (Claude Code)\nmore"), "2.1.4");
        assert_eq!(tidy_version("codex-cli 0.159.0"), "0.159.0");
        assert_eq!(tidy_version("v1.2"), "1.2");
        assert_eq!(tidy_version("tool 3.4.5-beta+x"), "3.4.5");
        // A diagnostic that holds a key, a path or an escape yields no byte of them.
        for raw in [
            "error: invalid api key sk-ant-api03-ABCdef123",
            "/home/x/.claude-p/bin v1",
            "\u{1b}[31m\n",
            "key=abc123DEF456ghi789",
            "",
            "1234567.1 7.8.9.10.11",
        ] {
            assert_eq!(tidy_version(raw), "-", "{raw:?}");
        }
        let mixed = tidy_version("token sk-1.2.3abc 9.9");
        assert_eq!(mixed, "9.9");
        assert!(tidy_version(&"9.".repeat(40)).len() <= 24);
    }

    #[test]
    fn a_tool_selection_is_known_nonempty_and_without_repeats() {
        let names = |wanted: &str| {
            select_tools(wanted).map(|tools| tools.iter().map(|spec| spec.name).collect::<Vec<_>>())
        };
        assert_eq!(names("claude,codex"), Ok(vec!["claude", "codex"]));
        assert_eq!(names(" claude , muse "), Ok(vec!["claude", "muse"]));
        assert_eq!(names(DEFAULT_TOOLS).map(|v| v.len()), Ok(4));
        for bad in ["", " ", ",", ", ,", "claude,", ",claude", "claude,,codex"] {
            assert!(names(bad).is_err(), "{bad:?} must be refused");
        }
        assert!(names("claude,claude").is_err(), "a repeat spends twice");
        assert!(names("claude, claude").is_err(), "a repeat spends twice");
        assert!(names("claude,nosuchtool").is_err());
    }

    #[test]
    fn the_control_runs_the_shim_whatever_the_environment_says() {
        let claude = &TOOLS[0];
        let real = Some("/opt/homebrew/bin/claude --model x".to_owned());
        assert_eq!(command_for(true, claude, real.clone()), "claude");
        assert_eq!(command_for(true, claude, None), "claude");
        assert_eq!(
            command_for(false, claude, real.clone()),
            "/opt/homebrew/bin/claude --model x"
        );
        assert_eq!(command_for(false, claude, None), claude.cmd);
    }

    const UUID_A: &str = "123e4567-e89b-42d3-a456-426614174000";
    const UUID_B: &str = "223e4567-e89b-42d3-a456-426614174001";
    const ID: &str = "323e4567-e89b-42d3-a456-426614174002";

    fn decision(action: &str, key: &str, prior: &str, id: &str) -> String {
        format!(
            "{{\"ts\":\"2026-10-31T21:00:00Z\",\"actor\":\"ae:lead\",\"action\":\"{action}\",\
             \"ref\":\"{key} {prior} {id}\"}}\n"
        )
    }

    #[test]
    fn a_journal_decision_is_parsed_not_scanned_for_words() {
        let started = decision("conversation-new", &format!("main:{UUID_A}:11"), "-", ID);
        let journal = format!("not json\n{started}{{\"action\":\"conversation-resumed\"}}\n");
        // The bare word in an unparseable line is no decision at all.
        assert_eq!(decided_starts(&journal), vec![format!("main:{UUID_A}:11")]);
        assert_eq!(conversation_decisions(&journal).len(), 1);
        assert!(conversation_decisions("").is_empty());
    }

    #[test]
    fn only_a_new_resume_of_the_same_conversation_proves_the_step() {
        let first = decision("conversation-new", &format!("main:{UUID_A}:11"), "-", ID);
        let before = decided_starts(&first);
        let key_b = format!("main:{UUID_B}:22");
        let resumed = |action: &str, prior: &str, id: &str| {
            format!("{first}{}", decision(action, &key_b, prior, id))
        };
        // The one pass.
        assert_eq!(
            judge_resume(&before, &resumed("conversation-resumed", "-", ID), ID),
            Resumption::Proven
        );
        assert_eq!(
            judge_resume(&before, &resumed("conversation-unproven", "-", ID), ID),
            Resumption::Proven,
            "a tool with no store probe journals unproven; the recall turn proves it"
        );
        // Nothing journalled yet, or only the stale first start: ask again.
        assert!(matches!(
            judge_resume(&before, "", ID),
            Resumption::NotYet(_)
        ));
        assert!(matches!(
            judge_resume(&before, &first, ID),
            Resumption::NotYet(_)
        ));
        // Every other answer is final.
        let other = "423e4567-e89b-42d3-a456-426614174003";
        for (journal, why) in [
            (
                resumed("conversation-resumed", "-", other),
                "another conversation",
            ),
            (resumed("conversation-new", "-", ID), "a new conversation"),
            (
                resumed("conversation-lost-not-found", ID, "pending"),
                "a lost conversation",
            ),
            (
                resumed("conversation-resumed", "-", "not an id!"),
                "a damaged record",
            ),
            (
                format!(
                    "{first}{}",
                    decision(
                        "conversation-resumed",
                        &format!("worker.0:{UUID_B}:22"),
                        "-",
                        ID
                    )
                ),
                "another seat",
            ),
        ] {
            assert!(
                matches!(judge_resume(&before, &journal, ID), Resumption::Refused(_)),
                "{why}"
            );
        }
    }

    #[test]
    fn a_report_that_cannot_be_written_is_an_error_and_the_strays_are_in_it() {
        let scratch = OwnedScratch::root("hs", "report");
        let file = scratch.path().join("a-file");
        std::fs::write(&file, "x").expect("a plain file");
        // A directory that is a file cannot be created or written under.
        assert!(write_report(&file.join("reports"), 1, "t\n").is_err());
        let ok = write_report(&scratch.path().join("reports"), 7, "t\n")
            .unwrap_or_else(|why| panic!("a writable report: {why}"));
        assert!(ok.ends_with("report-7.tsv"));

        let mut outcome = Outcome::new();
        outcome.strays.push(format!("claude {ID}"));
        outcome.runner_turns = 2;
        outcome.ae_turns = 1;
        let row = Row {
            root: PathBuf::new(),
            tool: "claude".to_owned(),
            outcome,
            cleanup: Duration::ZERO,
            wall: Duration::ZERO,
        };
        let (table, tsv) = render(&[row]);
        assert!(
            tsv.lines()
                .nth(1)
                .is_some_and(|l| l.ends_with(&format!("claude {ID}"))),
            "{tsv}"
        );
        assert!(table.contains(&format!("stray vendor conversation: claude {ID}")));
        assert!(table.contains("runner=2 ae=1"));
    }

    fn row_with_frame(tool: &str, frame: Option<&str>) -> Row {
        let mut outcome = Outcome::new();
        if let Some(frame) = frame {
            outcome.keep_frame(frame);
        }
        Row {
            root: PathBuf::new(),
            tool: tool.to_owned(),
            outcome,
            cleanup: Duration::ZERO,
            wall: Duration::ZERO,
        }
    }

    #[test]
    fn a_plain_frame_is_kept_as_it_was_drawn() {
        let drawn = " ▐▛███▛█   Claude Code v2.1.314\n  Welcome back\n\n❯ \n";
        assert_eq!(frame_evidence(drawn), drawn);
        assert_eq!(frame_evidence("a\nb"), "a\nb\n", "ends on a row break");
    }

    #[test]
    fn a_frame_with_no_text_says_so_instead_of_being_blank() {
        for blank in ["", "\n", "   \n \n"] {
            assert_eq!(frame_evidence(blank), "(no text captured)\n", "{blank:?}");
        }
    }

    #[test]
    fn a_frame_never_carries_a_control_character_but_the_row_break_and_tab() {
        let hostile = "ok\u{1b}[2J\u{1b}]52;c;aGk=\u{7}tail\r\nrow\u{0}two\u{9b}31m\u{7f}\tkept\n";
        let kept = frame_evidence(hostile);
        assert!(
            kept.chars()
                .all(|c| c == '\n' || c == '\t' || !c.is_control()),
            "{kept:?}"
        );
        for word in ["ok", "tail", "row", "two", "kept"] {
            assert!(kept.contains(word), "{word} survives as text: {kept:?}");
        }
        assert!(kept.contains('\t'), "a tab is layout, not a control");
    }

    #[test]
    fn a_frame_is_bounded_in_rows_columns_and_bytes_and_keeps_its_newest_rows() {
        let tall = (0..500).fold(String::new(), |mut text, n| {
            let _ = writeln!(text, "row {n}");
            text
        });
        let kept = frame_evidence(&tall);
        assert!(
            kept.lines().count() <= FRAME_ROWS,
            "{}",
            kept.lines().count()
        );
        assert_eq!(kept.lines().last(), Some("row 499"), "the newest row stays");
        assert_ne!(kept.lines().next(), Some("row 0"), "the oldest rows go");

        let wide = format!("{}\nshort\n", "x".repeat(10_000));
        let kept = frame_evidence(&wide);
        assert!(
            kept.lines().all(|row| row.chars().count() <= FRAME_CHARS),
            "no row outgrows the column bound"
        );
        assert!(
            kept.starts_with("xxx") && kept.contains("…\n"),
            "a clipped row is marked"
        );
        assert!(kept.ends_with("short\n"));

        let heavy = (0..FRAME_ROWS).fold(String::new(), |mut text, n| {
            let _ = writeln!(text, "{n:02}{}", "█".repeat(FRAME_CHARS - 2));
            text
        });
        assert!(
            heavy.len() > FRAME_BYTES,
            "the fixture outgrows the byte bound"
        );
        let kept = frame_evidence(&heavy);
        assert!(kept.len() <= FRAME_BYTES, "{}", kept.len());
        assert!(
            kept.lines().last().is_some_and(|row| row.starts_with("79")),
            "the newest row stays under the byte bound too"
        );
    }

    #[test]
    fn a_kept_frame_is_the_neutralised_evidence_and_leaves_the_verdict_alone() {
        let mut outcome = Outcome::new();
        assert!(
            outcome.frame.is_none(),
            "a run keeps nothing until a wait runs out"
        );
        outcome.keep_frame("hello\u{1b}[31m red\n");
        let frame = outcome.frame.clone().expect("a kept frame");
        assert!(
            frame.contains("hello") && !frame.contains('\u{1b}'),
            "{frame:?}"
        );
        assert_eq!(
            outcome.verdict(),
            "FAIL(incomplete)",
            "keeping a frame is no pass"
        );
    }

    #[test]
    fn a_frame_file_is_private_named_for_its_tool_and_never_overwritten_or_followed() {
        let scratch = OwnedScratch::root("hs", "frame");
        let dir = scratch.path().join("reports");
        let file = write_frame(&dir, 7, "claude", "box\n")
            .unwrap_or_else(|why| panic!("a writable frame: {why}"));
        assert!(file.ends_with("frame-7-claude.txt"), "{}", file.display());
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), "box\n");
        let mode = std::fs::metadata(&file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "a frame can show an account: owner only"
        );

        let why = write_frame(&dir, 7, "claude", "OTHER\n").expect_err("no overwrite");
        assert!(
            !why.contains("OTHER"),
            "an error never carries the content: {why}"
        );
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), "box\n");

        let target = scratch.path().join("target");
        std::fs::write(&target, "keep").expect("a target");
        std::os::unix::fs::symlink(&target, dir.join("frame-8-claude.txt")).expect("a link");
        assert!(
            write_frame(&dir, 8, "claude", "x\n").is_err(),
            "a link is not followed"
        );
        assert_eq!(
            std::fs::read_to_string(&target).expect("the target"),
            "keep"
        );

        let blocker = scratch.path().join("a-file");
        std::fs::write(&blocker, "x").expect("a plain file");
        assert!(write_frame(&blocker.join("reports"), 1, "claude", "x\n").is_err());
    }

    #[test]
    fn only_a_row_that_kept_a_frame_gets_a_frame_file_and_a_line_naming_it() {
        let scratch = OwnedScratch::root("hs", "frames");
        let dir = scratch.path().join("reports");
        let rows = [
            row_with_frame("claude", Some("trust this folder?\n")),
            row_with_frame("codex", None),
        ];
        let (notes, problems) = keep_frames(Some(&dir), 9, &rows);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("frame-9-claude.txt"), "{notes:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("frame-9-claude.txt")).expect("the frame"),
            "trust this folder?\n"
        );
        assert!(!dir.join("frame-9-codex.txt").exists());
    }

    #[test]
    fn a_frame_with_no_report_directory_says_it_was_not_kept_and_never_prints_it() {
        let rows = [row_with_frame("claude", Some("SECRET-LOOKING\n"))];
        let (notes, problems) = keep_frames(None, 9, &rows);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("AE_HARNESS_SMOKE_REPORT"), "{notes:?}");
        assert!(!notes[0].contains("SECRET-LOOKING"), "{notes:?}");
        let (notes, problems) = keep_frames(None, 9, &[row_with_frame("claude", None)]);
        assert!(
            notes.is_empty() && problems.is_empty(),
            "{notes:?} {problems:?}"
        );
    }

    /// `path` as one single-quoted shell word, whatever the scratch root holds.
    fn shell_quoted(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    /// A driver over a private tmux server whose one pane (`%0`) runs `script`:
    /// no ae and no vendor, only the real capture leg and the real waits.
    fn run_over(script: &str, tag: &str) -> (Run, OwnedScratch) {
        let mut scratch = OwnedScratch::root("hs", tag);
        let root = scratch.path().to_owned();
        let sock = root.join("sock");
        scratch.add_tmux_server(sock.clone());
        let gate = Gate::new();
        let mut tmux = cli::helper_by_name("tmux");
        tmux.env("TMUX_TMPDIR", &root).env_remove("TMUX").args([
            "-S",
            &sock.display().to_string(),
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "hs",
            "-x",
            "100",
            "-y",
            "30",
            "sh",
            "-c",
            script,
        ]);
        let done = run_bounded(&gate, &mut tmux, Duration::from_secs(10)).expect("tmux starts");
        assert_eq!(done.code, Some(0), "a private pane: {}", done.stderr);
        let run = Run {
            gate,
            tool: &TOOLS[0],
            kind: ToolKind::Claude,
            session: "hs".to_owned(),
            state: root.join("state"),
            config: root.join("config"),
            server: ServerId::Selected(Selector::Socket(sock.clone())),
            sock,
            scratch: root.clone(),
            work: root.clone(),
            path_env: String::new(),
            home: String::new(),
            step: Duration::from_secs(2),
            watchdog: false,
            last: RefCell::new(String::new()),
            token: "0".to_owned(),
            operands: (0, 0),
        };
        (run, scratch)
    }

    #[test]
    fn a_readiness_wait_that_runs_out_leaves_its_last_frame_even_if_the_run_is_cut_at_once() {
        let (run, _scratch) = run_over(
            "printf 'Do you trust this folder? (y/n)\\n'; sleep 120",
            "wait-miss",
        );
        let two = Duration::from_secs(2);
        assert!(!run.frame_twice("%0", two, Want::Drawn).expect("a wait"));
        let kept = run
            .gate
            .snapshot()
            .frame
            .expect("the last frame is published");
        assert!(kept.contains("trust this folder"), "{kept:?}");
        // The outer bound cancels and snapshots before the worker reacts: the
        // frame was published during the wait, so that snapshot still has it.
        run.gate.cancel();
        assert!(
            run.gate
                .snapshot()
                .frame
                .is_some_and(|f| f.contains("trust this folder"))
        );

        // The pre-send quiet wait on a tool with no frame grammar is the other
        // readiness wait: a pane that never settles leaves its frame too.
        let (run, _scratch) = run_over(
            "i=0; while :; do i=$((i+1)); echo tick $i; sleep 0.2; done",
            "settle-miss",
        );
        assert!(!run.settled("%0", two).expect("a wait"));
        let kept = run
            .gate
            .snapshot()
            .frame
            .expect("the last frame is published");
        assert!(kept.contains("tick"), "{kept:?}");
    }

    #[test]
    fn a_readiness_wait_that_passes_clears_the_frame_it_published_on_the_way() {
        // The pane is blank first, so the wait publishes a frame on the way, then
        // draws a real fresh claude start and must clear it.
        let files = OwnedScratch::root("hs", "fresh");
        let fresh = files.path().join("fresh.txt");
        std::fs::write(&fresh, FRESH).expect("the fresh frame");
        let script = format!("sleep 1.5; cat {}; sleep 120", fresh.display());
        let (run, _scratch) = run_over(&script, "wait-pass");
        let bound = Duration::from_secs(8);
        assert!(run.frame_twice("%0", bound, Want::Drawn).expect("a wait"));
        assert!(
            run.gate.snapshot().frame.is_none(),
            "a pass leaves no frame"
        );

        let (run, _scratch) = run_over("printf 'quiet\\n'; sleep 120", "settle-pass");
        assert!(run.settled("%0", bound).expect("a wait"));
        assert!(
            run.gate.snapshot().frame.is_none(),
            "a pass leaves no frame"
        );
    }

    #[test]
    fn a_readiness_wait_that_never_gets_a_capture_leaves_the_placeholder() {
        // The pane does not exist: every look answers nothing, so the evidence is
        // the placeholder that says so, for both readiness waits.
        let (run, _scratch) = run_over("sleep 120", "no-pane");
        let one = Duration::from_secs(1);
        assert!(!run.frame_twice("%99", one, Want::Drawn).expect("a wait"));
        assert_eq!(
            run.gate.snapshot().frame.as_deref(),
            Some("(no text captured)\n")
        );
        let (run, _scratch) = run_over("sleep 120", "no-pane-settle");
        assert!(!run.settled("%99", one).expect("a wait"));
        assert_eq!(
            run.gate.snapshot().frame.as_deref(),
            Some("(no text captured)\n")
        );
    }

    #[test]
    fn a_later_failed_capture_does_not_erase_the_last_successful_frame() {
        // The pane prints a line and exits, taking the session (and the server)
        // with it: the looks after that answer nothing.
        let (run, _scratch) = run_over("echo last-words; sleep 1.2", "pane-exits");
        let bound = Duration::from_secs(4);
        assert!(!run.frame_twice("%0", bound, Want::Drawn).expect("a wait"));
        let kept = run.gate.snapshot().frame.expect("a frame");
        assert!(kept.contains("last-words"), "{kept:?}");
    }

    #[test]
    fn an_outer_cut_before_any_capture_returns_still_finds_the_placeholder() {
        let (run, _scratch) = run_over("sleep 120", "cut-first");
        let gate = run.gate.clone();
        let waiting = std::thread::spawn(move || {
            run.frame_twice("%99", Duration::from_secs(30), Want::Drawn)
        });
        std::thread::sleep(Duration::from_millis(300));
        gate.cancel();
        let ended = waiting.join().expect("the wait thread");
        assert!(matches!(ended, Err(Stop::Cancelled)), "{ended:?}");
        assert_eq!(
            gate.snapshot().frame.as_deref(),
            Some("(no text captured)\n"),
            "the cut run carries evidence although no capture ever returned"
        );
    }

    #[test]
    fn a_readiness_pass_that_lands_after_the_outer_cut_cannot_erase_the_frame() {
        let (run, _scratch) = run_over("printf 'stuck here\\n'; sleep 120", "cut-then-pass");
        assert!(
            !run.frame_twice("%0", Duration::from_secs(1), Want::Drawn)
                .expect("a wait")
        );
        // The outer bound cancels; the snapshot is taken a lock later. A pass
        // (or a late look) in between must change nothing the snapshot sees.
        run.gate.cancel();
        run.ready_passed();
        run.gate.with_live(|out| out.keep_frame("a later frame\n"));
        let kept = run
            .gate
            .snapshot()
            .frame
            .expect("the frame survives the cut");
        assert!(kept.contains("stuck here"), "{kept:?}");
    }

    #[test]
    fn a_wait_for_idle_after_a_turn_publishes_nothing() {
        let (run, _scratch) = run_over("printf 'Working...\\n'; sleep 120", "idle-miss");
        let two = Duration::from_secs(2);
        assert!(!run.frame_twice("%0", two, Want::Idle).expect("a wait"));
        assert!(
            run.gate.snapshot().frame.is_none(),
            "only readiness waits publish"
        );
    }

    #[test]
    fn a_frame_kept_by_a_run_that_read_pass_is_a_problem_not_evidence() {
        // A frame is the witness of a readiness wait that failed; a wait that
        // passed clears it, so one still standing on a PASS row is a wiring bug.
        let scratch = OwnedScratch::root("hs", "frames-pass");
        let dir = scratch.path().join("reports");
        let mut row = row_with_frame("claude", Some("left behind\n"));
        row.outcome.marks = [Mark::Ok; 10];
        assert_eq!(row.outcome.verdict(), "PASS");
        let (notes, problems) = keep_frames(Some(&dir), 9, &[row]);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("claude") && problems[0].contains("PASS"),
            "{problems:?}"
        );
        assert!(!dir.join("frame-9-claude.txt").exists());
    }

    #[test]
    fn a_frame_that_cannot_be_written_is_a_problem_that_names_why_and_not_the_frame() {
        let scratch = OwnedScratch::root("hs", "frames-bad");
        let blocker = scratch.path().join("a-file");
        std::fs::write(&blocker, "x").expect("a plain file");
        let rows = [row_with_frame("claude", Some("SECRET-LOOKING\n"))];
        let (_, problems) = keep_frames(Some(&blocker.join("reports")), 9, &rows);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("claude"), "{problems:?}");
        assert!(!problems[0].contains("SECRET-LOOKING"), "{problems:?}");
    }

    #[test]
    fn a_run_reads_pass_only_when_every_step_passed() {
        let mut outcome = Outcome::new();
        assert_eq!(
            outcome.verdict(),
            "FAIL(incomplete)",
            "nothing ran is not a pass"
        );
        outcome.marks = [Mark::Ok; 10];
        assert_eq!(outcome.verdict(), "PASS");
        outcome.marks[6] = Mark::Skip;
        outcome.skipped = Some(Step::Idle);
        assert_eq!(outcome.verdict(), "SKIP(idle)");
        outcome.limited = Some(Step::Turn1);
        assert_eq!(outcome.verdict(), "LIMIT(turn1)");
        outcome.failed = Some(Step::Resume);
        assert_eq!(outcome.verdict(), "FAIL(resume)");
    }

    #[test]
    fn a_cancelled_gate_starts_nothing() {
        let gate = Gate::new();
        gate.cancel();
        let mut runner = cli::helper_by_name("true");
        assert!(matches!(gate.spawn(&mut runner), Err(Stop::Cancelled)));
    }

    #[test]
    fn a_child_that_outlives_its_bound_is_killed_and_reported() {
        let gate = Gate::new();
        let mut runner = cli::helper_by_name("sleep");
        runner.arg("30");
        let started = Instant::now();
        let done = run_bounded(&gate, &mut runner, Duration::from_millis(300))
            .unwrap_or_else(|_| panic!("a bounded run answers"));
        assert!(done.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the bound must hold"
        );
    }

    #[test]
    fn the_work_directory_lock_refuses_a_second_holder_and_removes_only_its_own() {
        let scratch = OwnedScratch::root("hs", "lock");
        let first = WorkDir::acquire_in(scratch.path())
            .unwrap_or_else(|why| panic!("a free lock is taken: {why}"));
        let work = first.path().to_owned();
        assert!(work.is_dir(), "the holder gets its work directory");
        // A second holder is refused and touches nothing of the first's.
        std::fs::write(work.join("mine"), "x").expect("a file in the holder's work");
        let second = WorkDir::acquire_in(scratch.path());
        assert!(second.is_err(), "a held lock refuses a second run");
        assert!(
            work.join("mine").is_file(),
            "the refused run removed nothing"
        );
        drop(first);
        assert!(!work.exists(), "the holder removes its work directory");
        assert!(
            WorkDir::acquire_in(scratch.path()).is_ok(),
            "a released lock can be taken again"
        );
    }

    #[test]
    fn a_stale_lock_is_reported_and_never_stolen() {
        let scratch = OwnedScratch::root("hs", "stale");
        let lock = scratch.path().join("ae-harness-smoke").join("lock");
        std::fs::create_dir_all(&lock).expect("a stale lock");
        std::fs::write(lock.join("owner"), "pid 1 since 0\n").expect("an owner line");
        let refused = WorkDir::acquire_in(scratch.path())
            .err()
            .unwrap_or_default();
        assert!(
            refused.contains("pid 1 since 0"),
            "the holder is named: {refused}"
        );
        assert!(
            lock.join("owner").is_file(),
            "the stale lock is left standing"
        );
    }

    #[test]
    fn the_gate_variable_must_be_exactly_one() {
        assert!(opens(Some("1")));
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("true"),
            Some("11"),
            Some(" 1"),
        ] {
            assert!(!opens(value), "{value:?}");
        }
    }
}
