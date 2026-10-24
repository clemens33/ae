//! profview (#257 slice A): `ae quota --profiles`, the read-only per-profile view.
//!
//! Oracle: issue #257 "Possible first slice" and "Four separate quantities", lead
//! rulings R1-R6 and plan r2 (sha256 edf1f3bd…). Every rig is planted here; numbers
//! are judged against the planted bytes or against `ae::usage::observe`, the existing
//! reader the view must agree with, never against the view's own code. Every CLI run
//! goes through [`Rig::view`], which also proves R1 on that run: no program started
//! (a sentinel PATH), exit 0, nothing on stderr. Budget pins use the pure seam
//! `ae::quota::run_profiles(inputs, transcript_bytes, out, err)` (plan P18).

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the boundary is about what product code may reach"
)]

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const SENTINELS: &str = "tmux claude codex grok agy opencode gemini muse";
const GIB: u64 = 1 << 30;
const DEAD_URL: &str = "http://127.0.0.1:9/";
const FAR: &str = "2099-01-01T00:00:00+00:00";

fn now() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs();
    i64::try_from(secs).expect("epoch fits i64")
}

/// A canonical lowercase v7 UUID for seat `n`, stamped 2026-09-08 like the rollouts.
fn id(n: u32) -> String {
    format!("01a08046-1974-7000-8000-{n:012}")
}

#[track_caller]
fn pin(ok: bool, why: &str, context: &dyn std::fmt::Debug) {
    assert!(ok, "{why}: {context:#?}");
}

struct Rig {
    root: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let root = super::cli::OwnedScratch::root("profview", tag).keep();
        std::fs::create_dir_all(root.join(".ae/sessions")).expect("sessions root");
        let rig = Self { root };
        let marker = rig.path("invoked");
        for name in SENTINELS.split(' ') {
            let body = format!(
                "#!/bin/sh\nprintf '{name} ' >> '{}'\nexit 99\n",
                marker.display()
            );
            rig.write(&format!("bin/{name}"), body.as_bytes());
            rig.mode(&format!("bin/{name}"), 0o755);
        }
        rig
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, body: &[u8]) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("parent dir");
        std::fs::write(path, body).expect("fixture file");
    }

    #[cfg_attr(not(unix), allow(unused_variables, reason = "modes are unix-only"))]
    fn mode(&self, rel: &str, mode: u32) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let perms = std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(self.path(rel), perms).expect("fixture mode");
        }
    }

    fn config(&self, profiles: &str) {
        let clients = "claude = claude\nmic = claude config_home=$HOME/.claude-mic manual_resets=1\ncodex = codex";
        let text = format!("[clients]\n{clients}\n[profiles]\n{profiles}");
        self.write("config", text.as_bytes());
    }

    /// One session meta: `schema=2` plus the given rows, deliberately no `meta_version`.
    fn session(&self, name: &str, rows: &str) -> PathBuf {
        let rel = format!(".ae/sessions/{name}/meta");
        self.write(&rel, format!("schema=2\nmode=local\n{rows}").as_bytes());
        self.path(&format!(".ae/sessions/{name}"))
    }

    /// The meta rows of one roster seat named `s-<slot>`; its store is the rig's own.
    fn seat(&self, slot: &str, profile: &str, bin: &str, sid: u32) -> String {
        let store = if bin == "codex" {
            "codex-home"
        } else {
            "claude-home"
        };
        let (name, sid, home) = (slot.replace('.', "-"), id(sid), self.path(store));
        let home = home.display();
        format!(
            "seat.{slot}=s-{name}\nprofile.{slot}={profile}\nagent_bin.{slot}={bin}\nharness_session.{slot}={sid}\nconfig_home.{slot}={home}\n"
        )
    }

    fn claude(&self, sid: u32, body: &[u8]) {
        let rel = format!("claude-home/projects/work/{}.jsonl", id(sid));
        self.write(&rel, body);
    }

    fn codex(&self, sid: u32, body: &[u8]) {
        let rel = format!(
            "codex-home/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{}.jsonl",
            id(sid)
        );
        self.write(&rel, body);
    }

    fn command(&self, args: &[&str], url: &str) -> std::process::Output {
        super::cli::ae()
            .env("HOME", &self.root)
            .env("AE_HOME", self.path(".ae"))
            .env("CONFIG_FILE", self.path("config"))
            .env("PATH", self.path("bin"))
            .env("AE_TEST_GO_USAGE_URL", url)
            .env("AE_TEST_COPILOT_USAGE_URL", url)
            .current_dir(&self.root)
            .args(args)
            .output()
            .expect("ae runs")
    }

    fn invoked(&self) -> String {
        std::fs::read_to_string(self.path("invoked")).unwrap_or_default()
    }

    /// `ae quota --profiles`: exit 0, quiet stderr, no program started.
    fn view(&self) -> String {
        let out = self.command(&["quota", "--profiles"], DEAD_URL);
        let ok = out.status.code() == Some(0) && out.stderr.is_empty();
        pin(ok, "exit 0, quiet", &out);
        let ok = self.invoked().is_empty();
        pin(ok, "the view started a program", &self.invoked());
        checked(String::from_utf8(out.stdout).expect("UTF-8 view"))
    }

    /// The pure seam (plan P17/P18): the CLI is this call with the real clock and a
    /// 1 GiB transcript budget. A fixed `now` makes two outputs byte-comparable.
    fn seam(&self, at: i64, transcript_bytes: u64) -> String {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = self.quota(at, |inputs| {
            ae::quota::run_profiles(inputs, transcript_bytes, &mut out, &mut err)
        });
        let ok = code.is_ok_and(|code| code == 0) && err.is_empty();
        pin(ok, "exit 0, quiet", &String::from_utf8_lossy(&err));
        checked(String::from_utf8(out).expect("UTF-8 view"))
    }

    /// Plain `ae quota` at the same instant: the oracle for every window cell.
    fn table(&self, at: i64) -> String {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = self.quota(at, |inputs| ae::quota::run(inputs, &mut out, &mut err));
        let ok = code.is_ok_and(|code| code == 0);
        pin(ok, "rig: ae quota oracle", &String::from_utf8_lossy(&err));
        String::from_utf8(out).expect("UTF-8 table")
    }

    /// This rig's quota inputs at the instant `at`.
    fn quota<T>(&self, at: i64, run: impl FnOnce(&ae::quota::Inputs<'_>) -> T) -> T {
        let (config, sessions) = (self.path("config"), self.path(".ae/sessions"));
        run(&ae::quota::Inputs {
            home: Some(&self.root),
            global: Some(&config),
            local: None,
            sessions: Some(&sessions),
            now: at,
        })
    }

    /// The existing usage reader over one session: the reference the view agrees with.
    fn reference(&self, name: &str, path: &Path, at: i64) -> Vec<ae::usage::SeatUsage> {
        let sessions = [ae::usage::SessionInput {
            name: name.to_owned(),
            path: path.to_owned(),
        }];
        let prices = ae::usage::prices::Book::default();
        let inputs = ae::usage::Inputs {
            home: Some(&self.root),
            sessions: &sessions,
            prices: &prices,
            now: at,
        };
        ae::usage::observe(&inputs).sessions.remove(0).seats
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// R6 (nothing a terminal would act on) and R3 (descriptive: no rank, score,
/// recommendation or policy verdict) hold for every view.
fn checked(text: String) -> String {
    let safe = text.chars().all(|ch| ch == '\n' || !ch.is_control());
    pin(safe, "control char on stdout", &text);
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .collect();
    let banned = "cheaper cheapest recommend recommended best better worse rank ranked score stop park parked steer";
    for word in banned.split(' ') {
        pin(!words.contains(&word), word, &text);
    }
    text
}

fn claude_line(model: &str, input: u64, output: u64) -> String {
    let usage = format!("{{\"input_tokens\":{input},\"output_tokens\":{output}}}");
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"id\":\"m-{model}\",\"model\":\"{model}\",\"usage\":{usage}}}}}\n"
    )
}

/// The lines of one profile's block: from its `<profile> resolves=` header to the
/// next profile header (any of `all`) or the end.
fn block(text: &str, profile: &str, all: &[&str]) -> Vec<String> {
    let header = |line: &str, name: &str| line.starts_with(&format!("{name} resolves="));
    let mut lines = text.lines().skip_while(|line| !header(line, profile));
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("no block for {profile}: {text}"));
    let rest = lines.take_while(|line| !all.iter().any(|name| header(line, name)));
    std::iter::once(first)
        .chain(rest)
        .map(str::to_owned)
        .collect()
}

fn tokens(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| line.split_whitespace().map(str::to_owned))
        .collect()
}

fn has(lines: &[String], token: &str) -> bool {
    tokens(lines).iter().any(|held| held == token)
}

fn says(lines: &[String], phrase: &str) -> bool {
    lines.join(" ").contains(phrase)
}

/// The value of the first `key=` token in the block.
fn value(lines: &[String], key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    tokens(lines)
        .into_iter()
        .find_map(|held| held.strip_prefix(&prefix).map(str::to_owned))
}

fn provider(lines: &[String]) -> Option<String> {
    value(lines, "observed-provider")
}

/// The block prints a number for `key` (a value that starts with a digit).
fn numeric(lines: &[String], key: &str) -> bool {
    value(lines, key).is_some_and(|cell| cell.starts_with(|ch: char| ch.is_ascii_digit()))
}

/// Some line carries `word` and the number `n` as a whole digit run.
fn counted(text: &str, word: &str, n: u32) -> bool {
    let n = n.to_string();
    let digits = |line: &str| {
        line.split(|ch: char| !ch.is_ascii_digit())
            .any(|cell| cell == n)
    };
    text.lines().any(|line| line.contains(word) && digits(line))
}

/// Some line names both words.
fn names(text: &str, first: &str, second: &str) -> bool {
    text.lines()
        .any(|line| line.contains(first) && line.contains(second))
}

fn usd(micro: u64) -> String {
    format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000)
}

#[test]
fn profiles_flag_is_strict_and_combines_with_nothing() {
    let rig = Rig::new("args");
    rig.config("fablex = claude --model fable\n");
    for (args, word) in [
        (&["quota", "--profiles", "--profiles"][..], "--profiles"),
        (&["quota", "--profiles", "--json"][..], "--json"),
        (&["quota", "--json", "--profiles"][..], "--profiles"),
        (&["quota", "--profiles", "extra"][..], "extra"),
    ] {
        let out = rig.command(args, DEAD_URL);
        let named = String::from_utf8_lossy(&out.stderr).contains(word);
        let ok = out.status.code() == Some(2) && out.stdout.is_empty() && named;
        pin(ok, word, &out);
    }
    let json = rig.command(&["quota", "--json"], DEAD_URL);
    let ok = json.status.code() == Some(0)
        && !String::from_utf8_lossy(&json.stdout).contains("resolves=");
    pin(ok, "--json unchanged", &json);
    let ok = rig.invoked().is_empty();
    pin(ok, "no program for any argv", &rig.invoked());
}

type Worker = std::thread::JoinHandle<(std::net::TcpListener, usize)>;

/// A loopback endpoint that counts and drops every connection. The listener is bound
/// before `start` returns, so a connection the child made sits in the backlog at worst.
struct Sink {
    url: String,
    stop: Arc<AtomicBool>,
    worker: Option<Worker>,
}

impl Sink {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().expect("addr").port()
        );
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&stop);
        let pause = std::time::Duration::from_millis(5);
        let worker = std::thread::spawn(move || {
            let mut seen = 0;
            while !done.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    seen += 1;
                    let _ = stream.set_read_timeout(Some(pause * 10));
                    let _ = stream.read(&mut [0_u8; 512]);
                } else {
                    std::thread::sleep(pause);
                }
            }
            (listener, seen)
        });
        let worker = Some(worker);
        Self { url, stop, worker }
    }

    /// Stop and join the worker (each pass is bounded by its pause and read timeout),
    /// then drain the backlog: every connection the finished child made is counted.
    fn hits(mut self) -> usize {
        self.stop.store(true, Ordering::SeqCst);
        let worker = self.worker.take().expect("one count per sink");
        let (listener, mut seen) = worker.join().expect("sink worker");
        while listener.accept().is_ok() {
            seen += 1;
        }
        seen
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// R1: the view spends nothing — no agy leg, no Muse model call, no Go or Copilot
/// fetch, no tmux — and writes no byte. The control run of plain `ae quota` in the
/// SAME rig proves the legs were reachable, so silence here is the view's own.
#[test]
fn the_view_starts_no_process_reaches_no_network_and_writes_nothing() {
    let rig = Rig::new("offline");
    let config = concat!(
        "[workspace]\nmuse_quota = on\nopencode_go_key_file = $HOME/go-usage.key\n",
        "copilot_token_file = $HOME/copilot.token\n",
        "[clients]\nclaude = claude\nopencode = opencode\n",
        "[profiles]\nfablex = claude --model fable\nagyx = agy\n",
        "museq = muse --model muse-spark-1.3\ngo = opencode --model opencode-go/x\n",
    );
    rig.write("config", config.as_bytes());
    for (rel, body) in [
        ("go-usage.key", "go-secret\n"),
        ("copilot.token", "cp-secret\n"),
    ] {
        rig.write(rel, body.as_bytes());
        rig.mode(rel, 0o600);
    }
    rig.session("s", &rig.seat("main", "fablex", "claude", 1));
    let sink = Sink::start();
    let before = super::cli::byte_tree(&rig.root);
    let out = rig.command(&["quota", "--profiles"], &sink.url);
    pin(out.status.code() == Some(0), "exit 0", &out);
    let text = String::from_utf8(out.stdout).expect("UTF-8");
    let all = ["fablex", "agyx", "museq", "go"];
    for profile in all {
        block(&text, profile, &all);
    }
    let secret = text.contains("go-secret") || text.contains("cp-secret");
    pin(!secret, "no secret", &text);
    pin(sink.hits() == 0, "the view reached the network", &text);
    let ok = rig.invoked().is_empty();
    pin(ok, "the view started a program", &rig.invoked());
    let same = super::cli::byte_tree(&rig.root) == before;
    pin(same, "the view wrote bytes", &text);

    let sink = Sink::start();
    let control = rig.command(&["quota"], &sink.url);
    pin(control.status.code() == Some(0), "control exit", &control);
    let ok = rig.invoked().contains("agy");
    pin(ok, "control: quota asks agy", &rig.invoked());
    let ok = sink.hits() >= 1;
    pin(ok, "control: quota fetches Go/Copilot", &control);
}

/// R2: every configured profile, config order; model and effort as the command
/// spells them, unknown when it does not; a resolution failure is still listed.
#[test]
fn every_profile_prints_in_config_order_with_model_and_effort_as_spelled() {
    let rig = Rig::new("columns");
    rig.config(concat!(
        "fablex = claude --model fable --effort xhigh\n",
        "sol61h = codex -m gpt-6.1-sol -c model_reasoning_effort=high\n",
        "bare = claude\npiped = claude --model fable | tee x\ngrok46 = grok --model grok-4.6\n",
        "dup = claude --model fable --model opus --effort high --effort low\n",
        "cdup = codex -m a --model b -c model_reasoning_effort=high -c model_reasoning_effort=low\n",
        "dangle = claude --effort\n",
    ));
    let order = [
        "fablex", "sol61h", "bare", "piped", "grok46", "dup", "cdup", "dangle",
    ];
    let text = rig.view();
    let header = |name: &str| {
        text.lines()
            .position(|line| line.starts_with(&format!("{name} resolves=")))
    };
    let at: Vec<Option<usize>> = order.iter().map(|name| header(name)).collect();
    let ordered = at.iter().all(Option::is_some) && at.windows(2).all(|w| w[0] < w[1]);
    pin(ordered, "config order", &text);
    let note = text.contains("policy eligibility not known to ae");
    let ok = note && text.contains("(syntax only)");
    pin(ok, "eligibility note", &text);
    let fablex = block(&text, "fablex", &order);
    for token in "resolves=yes client=claude configured-model=fable effort=xhigh".split(' ') {
        pin(has(&fablex, token), token, &fablex);
    }
    let ok = !says(&fablex, "claude-fable");
    pin(ok, "no alias resolution", &fablex);
    let sol = block(&text, "sol61h", &order);
    for token in "resolves=yes client=codex configured-model=gpt-6.1-sol effort=high".split(' ') {
        pin(has(&sol, token), token, &sol);
    }
    let bare = block(&text, "bare", &order);
    let unknown = has(&bare, "configured-model=unknown") && has(&bare, "effort=unknown");
    pin(unknown, "never a guessed default", &bare);
    for name in ["dup", "cdup", "dangle"] {
        let lines = block(&text, name, &order);
        let ok = has(&lines, "configured-model=unknown") && has(&lines, "effort=unknown");
        pin(ok, "ambiguous/unparsed is unknown", &lines);
    }
    let ok = has(&block(&text, "piped", &order), "resolves=no");
    pin(ok, "piped", &text);
    for name in order {
        let lines = block(&text, name, &order);
        let unseen = provider(&lines).as_deref() == Some("unknown");
        let ok = unseen && has(&lines, "observed-model=no");
        pin(ok, "no seat, no provider", &lines);
    }
}

/// Plan P7-P10: the served provider comes from the transcript's model id through the
/// closed grammar — never from the client, the config pin or a near miss.
#[test]
fn served_provider_comes_from_transcript_ids_never_from_the_client() {
    let rig = Rig::new("provider");
    rig.config(concat!(
        "fablex = claude --model fable\nsol61h = codex -m gpt-6.1-sol\n",
        "proxy = claude --model gpt-6.1-sol\nnear1 = claude --model a\nnear2 = claude --model b\n",
        "near3 = claude --model c\nnear4 = claude --model d\nmix = claude --model opus\n",
        "idle = claude --model sonnet\nkmix = claude --model fable\n",
    ));
    let all = [
        "fablex", "sol61h", "proxy", "near1", "near2", "near3", "near4", "mix", "idle", "kmix",
    ];
    let mut rows = rig.seat("main", "sol61h", "codex", 2);
    rig.codex(2, include_bytes!("../fixtures/usage/codex-rollout.jsonl"));
    let planted = concat!(
        "fablex=claude-fable-5-1-20260901 proxy=gpt-6.1-sol near1=claude-foo-1 near2=claude-opus- ",
        "near3=gpt-x near4=xgpt-6 mix=claude-opus-5-5 mix=gpt-6.1-sol ",
        "kmix=claude-fable-5-1-20260901 kmix=claude-foo-1",
    );
    for (pair, n) in planted.split(' ').zip(10_u32..) {
        let (profile, model) = pair.split_once('=').expect("profile=model");
        rig.claude(n, claude_line(model, 10, 3).as_bytes());
        rows.push_str(&rig.seat(&format!("spawned.{n}"), profile, "claude", n));
    }
    let path = rig.session("served", &rows);
    let seen: Vec<String> = rig
        .reference("served", &path, now())
        .into_iter()
        .map(|seat| seat.model)
        .collect();
    for model in [
        "claude-fable-5-1-20260901",
        "gpt-6.1-sol",
        "claude-opus-5-5",
    ] {
        let ok = seen.iter().any(|held| held == model);
        pin(ok, "rig: transcript read", &seen);
    }
    let text = rig.view();
    let expected = concat!(
        "fablex=Anthropic sol61h=OpenAI proxy=OpenAI near1=unknown near2=unknown ",
        "near3=unknown near4=unknown mix=mixed idle=unknown",
    );
    for pair in expected.split(' ') {
        let (profile, expected) = pair.split_once('=').expect("profile=provider");
        let lines = block(&text, profile, &all);
        let ok = provider(&lines).as_deref() == Some(expected);
        pin(ok, profile, &lines);
    }
    for profile in [
        "fablex", "sol61h", "proxy", "near1", "near2", "near3", "near4",
    ] {
        let lines = block(&text, profile, &all);
        let known = provider(&lines).as_deref() != Some("unknown");
        let ok = has(&lines, "observed-model=yes/transcript")
            && (!known || lines[0].contains("(transcript)"));
        pin(ok, "unfamiliar id observed, source named", &lines);
    }
    let kmix = block(&text, "kmix", &all);
    let ok = provider(&kmix).as_deref() != Some("Anthropic") || says(&kmix, "partial");
    pin(ok, "known+unfamiliar never complete", &kmix);
}

/// Plan P7/P8/P10: a meta drift row is secondary evidence, labelled as such; an id in
/// the grammar names its provider, a display label names none.
#[test]
fn drift_rows_are_secondary_and_display_labels_name_no_provider() {
    let rig = Rig::new("drift");
    rig.config("opusd = claude --model fable\nmuseq = muse --model muse-spark-1.3\ngrokd = grok --model grok-4.6\nlongd = claude --model fable\n");
    let all = ["opusd", "museq", "grokd", "longd"];
    let mut rows = rig.seat("main", "opusd", "claude", 30);
    rows.push_str("observed_model.main=Opus 5 (1M context)\nobserved_model_pin.main=fable\n");
    rows.push_str(&rig.seat("worker.0", "museq", "muse", 31));
    rows.push_str(
        "observed_model.worker.0=muse-spark-1.4\nobserved_model_pin.worker.0=muse-spark-1.3\n",
    );
    rows.push_str(&rig.seat("spawned.0", "grokd", "grok", 32));
    rows.push_str("observed_model.spawned.0=Grok 4.6\n");
    rows.push_str(&rig.seat("spawned.1", "longd", "claude", 33));
    let _ = writeln!(
        rows,
        "observed_model.spawned.1=muse-spark-{}",
        "9".repeat(300)
    );
    rig.session("drift", &rows);
    let text = rig.view();
    for (profile, expected) in [
        ("opusd", "unknown"),
        ("museq", "Meta"),
        ("grokd", "unknown"),
    ] {
        let lines = block(&text, profile, &all);
        let ok = provider(&lines).as_deref() == Some(expected);
        pin(ok, profile, &lines);
        let ok = has(&lines, "observed-model=yes/drift");
        pin(ok, "drift evidence", &lines);
    }
    let muse = block(&text, "museq", &all);
    let ok = muse[0].contains("observed-provider=Meta (drift)");
    pin(ok, "drift label", &muse);
    let long = block(&text, "longd", &all);
    let ok = has(&long, "observed-model=no") && provider(&long).as_deref() == Some("unknown");
    pin(ok, "refused drift row is no evidence", &long);
}

/// R2/R3: coverage per profile, and the seat summary agrees with the existing usage
/// reader — output separate, reasoning stated per harness, missing never zero.
#[test]
fn coverage_and_seat_summary_agree_with_the_usage_reader_and_never_invent_zero() {
    let rig = Rig::new("summary");
    rig.config(concat!(
        "fablex = claude --model fable\nopus9 = claude --model opus\n",
        "sol61h = codex -m gpt-6.1-sol\ngrok46 = grok --model grok-4.6\nbare = claude\n",
    ));
    let all = ["fablex", "opus9", "sol61h", "grok46", "bare"];
    rig.claude(1, include_bytes!("../fixtures/usage/claude-main.jsonl"));
    rig.claude(3, claude_line("claude-opus-9-9", 5, 2).as_bytes());
    rig.codex(2, include_bytes!("../fixtures/usage/codex-rollout.jsonl"));
    let mut rows = rig.seat("main", "fablex", "claude", 1);
    rows.push_str(&rig.seat("worker.0", "sol61h", "codex", 2));
    rows.push_str(&rig.seat("spawned.0", "opus9", "claude", 3));
    rows.push_str(&rig.seat("spawned.1", "grok46", "grok", 4));
    let path = rig.session("summary", &rows);
    let reference = rig.reference("summary", &path, now());
    let seats = |slot: &str| -> Vec<&ae::usage::SeatUsage> {
        reference.iter().filter(|seat| seat.slot == slot).collect()
    };
    let fable = seats("main")
        .first()
        .copied()
        .expect("reference fable seat");
    let micro = fable.usd_micro.expect("rig precondition: fable is priced");
    let codex: u64 = seats("worker.0")
        .iter()
        .map(|seat| seat.tokens.output)
        .sum();
    pin(codex > 0, "rig precondition: codex output", &reference);
    let opus9 = seats("spawned.0");
    let unpriced = !opus9.is_empty() && opus9.iter().all(|seat| seat.usd_micro.is_none());
    pin(unpriced, "rig precondition: unpriced id", &reference);
    let text = rig.view();
    let ok = text.to_lowercase().contains("descriptive");
    pin(ok, "R3 label", &text);

    let lines = block(&text, "fablex", &all);
    let output = format!("output={}", fable.tokens.output);
    let spend = format!("API-equivalent-USD={}", usd(micro));
    for token in ["usage=yes", "priced=yes", "seats=1", &output, &spend] {
        pin(has(&lines, token), token, &lines);
    }
    let ok = says(&lines, "not separately reported");
    pin(ok, "claude reasoning", &lines);

    let lines = block(&text, "sol61h", &all);
    pin(has(&lines, "seats=1"), "one seat, many rows", &lines);
    let ok = has(&lines, &format!("output={codex}"));
    pin(ok, "codex output", &lines);
    let ok = says(&lines, "included in output");
    pin(ok, "codex reasoning", &lines);

    let lines = block(&text, "opus9", &all);
    pin(has(&lines, "priced=no"), "unpriced", &lines);
    let ok = has(&lines, "input=5") && has(&lines, "output=2");
    pin(ok, "observed counts as planted", &lines);
    let ok = !numeric(&lines, "API-equivalent-USD");
    pin(ok, "unpriced: no spend number", &lines);
    let ok = provider(&lines).as_deref() == Some("Anthropic");
    pin(ok, "id", &lines);

    let lines = block(&text, "grok46", &all);
    let ok = has(&lines, "usage=no") && says(&lines, "unsupported");
    pin(ok, "grok", &lines);
    let bare = block(&text, "bare", &all);
    let ok = has(&bare, "usage=yes") && has(&bare, "seats=0") && !has(&bare, "priced=yes");
    pin(ok, "nothing observed", &bare);
    for key in ["input", "output", "API-equivalent-USD"] {
        let ok = !numeric(&lines, key) && !numeric(&bare, key);
        pin(ok, "unsupported/unseen: no number", &lines);
    }
}

/// R3 + plan P21: elapsed lifetime of the CURRENT incarnation from `launch_time.<slot>`,
/// never `started`/`created`; a missing or future stamp is unknown, never zero.
#[test]
fn lifetime_is_the_current_incarnation_and_unknown_without_a_usable_stamp() {
    let rig = Rig::new("lifetime");
    rig.config("timed = claude --model a\nnostamp = claude --model b\nfuture = claude --model c\n");
    let all = ["timed", "nostamp", "future"];
    let t = now();
    let mut rows = format!("started={}\ncreated={}\n", t - 99_999, t - 99_999);
    rows.push_str(&rig.seat("main", "timed", "claude", 40));
    let _ = writeln!(rows, "launch_time.main={}", t - 7200);
    rows.push_str(&rig.seat("worker.0", "nostamp", "claude", 41));
    rows.push_str(&rig.seat("spawned.0", "future", "claude", 42));
    let _ = writeln!(rows, "launch_time.spawned.0={}", t + 3600);
    rig.session("life", &rows);
    let text = rig.seam(t, GIB);
    let life = |profile: &str| value(&block(&text, profile, &all), "current-incarnation-lifetime");
    let ok = life("timed").as_deref() == Some("7200s");
    pin(ok, "launch_time to now", &text);
    for profile in ["nostamp", "future"] {
        pin(life(profile).as_deref() == Some("unknown"), profile, &text);
    }
}

/// Plan P20/P22: a retired seat counts for the profile its validated retire summary
/// names; a retire with no such summary is counted unattributed; a bad journal line
/// and a symlinked meta are counted and named, never dropped.
#[test]
fn retired_and_damaged_records_are_counted_and_named_never_dropped() {
    let rig = Rig::new("damage");
    rig.config("fablex = claude --model fable\nidle = claude --model opus\n");
    let all = ["fablex", "idle"];
    let home = rig.path("claude-home");
    rig.claude(
        51,
        claude_line("claude-fable-5-1-20260901", 7, 4).as_bytes(),
    );
    let dir = rig.session("zq7", &rig.seat("main", "fablex", "claude", 50));
    let ts = |minute: u32| format!("\"ts\":\"2026-10-01T09:{minute:02}:00Z\",\"actor\":\"lead\"");
    let journal = format!(
        concat!(
            "{{{},\"action\":\"spawn\",\"target\":\"past\",\"target_slot\":\"spawned.1\"}}\n",
            "{{{},\"action\":\"retire\",\"target\":\"past\",\"ref\":\"{}\",\"target_slot\":\"spawned.1\",",
            "\"summary\":\"tool=claude profile=fablex config_home={} config_home_base=\"}}\n",
            "{{{},\"action\":\"retire\",\"target\":\"ghost\",\"target_slot\":\"spawned.2\",\"summary\":\"bye\"}}\n",
            "not json at all\n{{broken\n",
        ),
        ts(0),
        ts(10),
        id(51),
        home.display(),
        ts(20)
    );
    std::fs::write(dir.join("events.jsonl"), journal).expect("journal");
    std::fs::create_dir_all(rig.path(".ae/sessions/linked")).expect("linked session");
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("meta"), rig.path(".ae/sessions/linked/meta"))
        .expect("link");
    let seats = rig.reference("zq7", &dir, now());
    let past = seats
        .iter()
        .any(|seat| seat.slot == "spawned.1" && seat.retired && seat.tokens.output == 4);
    let ghost = seats
        .iter()
        .any(|seat| seat.slot == "spawned.2" && seat.coverage == ae::usage::Coverage::Unlocated);
    pin(past && ghost, "rig: retired seats read", &seats);
    let text = rig.view();
    let fablex = block(&text, "fablex", &all);
    pin(has(&fablex, "seats=2"), "live + retired, no link", &fablex);
    pin(has(&block(&text, "idle", &all), "seats=0"), "idle", &text);
    let ok = counted(&text, "unattributed", 1);
    pin(ok, "unattributed retire counted", &text);
    let ok = names(&text, "zq7", "journal");
    pin(ok, "bad journal line named with its session", &text);
    let ok = counted(&text, "journal", 2);
    pin(ok, "2 bad journal lines counted", &text);
    #[cfg(unix)]
    {
        let named = text
            .lines()
            .any(|line| line.contains("linked") && !line.contains(" resolves="));
        pin(named, "symlinked meta named, not dropped", &text);
        let ok = counted(&text, "meta", 1);
        pin(ok, "the damaged meta is counted, not merely named", &text);
    }
    let ok = says(&fablex, "partial") || says(&fablex, "at least");
    pin(ok, "damage makes totals partial", &fablex);
}

/// R6: a hostile config value or legacy session name reaches stdout escaped, never raw
/// and never dropped (`view` runs `checked` on every byte).
#[test]
fn control_bytes_in_config_values_and_session_names_never_reach_stdout_raw() {
    let rig = Rig::new("hostile");
    rig.config("esc = claude --model fa\u{1b}[31mble --effort hi\u{7}gh\n");
    let plain = rig.command(&["quota"], DEAD_URL);
    pin(plain.status.success(), "rig: config accepted", &plain);
    let dir = rig.session("ctl\u{7}zz", &rig.seat("main", "esc", "claude", 60));
    std::fs::write(dir.join("events.jsonl"), "not json\n").expect("journal");
    let text = rig.view();
    let esc = block(&text, "esc", &["esc"]);
    let ok = esc
        .first()
        .is_some_and(|line| line.starts_with("esc resolves="));
    pin(ok, "hostile profile shown", &text);
    let ok = has(&esc, "configured-model=unknown") || says(&esc, "configured-model=fa");
    pin(ok, "model cell present, escaped", &esc);
    let ok = text
        .lines()
        .any(|line| !line.contains(" resolves=") && names(line, "ctl", "zz"));
    pin(ok, "control-named session named, not dropped", &text);
}

/// One `ae quota` table row's cells from USED on (USED EFFECTIVE [CREDITS] PACE RESETS
/// OBSERVED STATUS), split on runs of two spaces as the table pads them: the oracle.
fn table_cells(plain: &str, bucket: &str, used: &str) -> Vec<String> {
    let (bucket_word, used_word) = (format!(" {bucket} "), format!(" {used} "));
    let line = plain
        .lines()
        .find(|line| line.contains(&bucket_word) && line.contains(&used_word))
        .unwrap_or_else(|| panic!("rig precondition: ae quota lists {bucket} {used}: {plain}"));
    let all: Vec<String> = line
        .split("  ")
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(str::to_owned)
        .collect();
    let at = all.iter().position(|cell| cell == used).expect("USED cell");
    all[at..].to_vec()
}

/// The view's window line naming `bucket` and `used` in a profile block.
fn window(lines: &[String], bucket: &str, used: &str) -> String {
    let found = lines
        .iter()
        .find(|line| line.contains(used) && line.split_whitespace().any(|word| word == bucket));
    found
        .unwrap_or_else(|| panic!("{bucket} {used} window listed: {lines:?}"))
        .clone()
}

/// Plan P11-P13: every window of the profile's own scope, each on its own line with the
/// cells `ae quota` derives (oracle: `ae::quota::run` at the same instant); the level is
/// the dialog's first-sight one, labelled so; a spend cap is carried, not dropped.
#[test]
fn quota_rows_list_every_window_of_the_own_scope_with_the_first_sight_level() {
    let rig = Rig::new("windows");
    rig.config(
        "fablex = claude --model fable\nmicx = mic --model opus\ncapx = codex -m gpt-6.1-sol\n",
    );
    let all = ["fablex", "micx", "capx"];
    let t = now();
    let limit = |kind: &str, group: &str, pct: u32, scope: &str| {
        format!(
            r#"{{"kind":"{kind}","group":"{group}","percent":{pct},"resets_at":"{FAR}","scope":{scope}}}"#
        )
    };
    let fetched = (t - 60) * 1000;
    let cache = |session: u32, weekly: u32, scoped: u32| {
        let fable = r#"{"model":{"id":null,"display_name":"Fable"},"surface":null}"#;
        let limits = [
            limit("session", "session", session, "null"),
            limit("weekly_all", "weekly", weekly, "null"),
            limit("weekly_scoped", "weekly", scoped, fable),
        ]
        .join(",");
        let usage = format!(
            r#"{{"accountUuid":"x","fetchedAtMs":{fetched},"utilization":{{"limits":[{limits}]}}}}"#
        );
        format!(r#"{{"cachedUsageUtilization":{usage}}}"#)
    };
    rig.write(".claude.json", cache(66, 85, 100).as_bytes());
    rig.write(".claude-mic/.claude.json", cache(40, 12, 30).as_bytes());
    let capped = String::from_utf8_lossy(include_bytes!("../fixtures/quota/codex-rollout.jsonl"))
        .replace(
            "\"plan_type\":\"pro\"",
            "\"plan_type\":\"pro\",\"credits\":{\"has_credits\":false,\"unlimited\":false,\"balance\":\"0\"},\"spend_control_reached\":true",
        );
    let rollout = format!(
        ".codex/sessions/2026/09/08/rollout-2026-09-08T09-00-00-{}.jsonl",
        id(5)
    );
    rig.write(&rollout, capped.as_bytes());
    let seat = format!(
        "seat.main=cap\nprofile.main=capx\nagent_bin.main=codex\nharness_session.main={}\n",
        id(5)
    );
    rig.session("q", &seat);
    let plain = rig.table(t);
    let windows = concat!(
        "fablex session 66% headroom,fablex weekly_all 85% low,fablex weekly_scoped 100% critical,",
        "micx session 40% -,micx weekly_all 12% -,micx weekly_scoped 30% -",
    );
    let specs: Vec<Vec<&str>> = windows
        .split(',')
        .map(|spec| spec.split(' ').collect())
        .collect();
    let oracle: Vec<Vec<String>> = specs
        .iter()
        .map(|word| table_cells(&plain, word[1], word[2]))
        .collect();
    let ok = oracle[3][1] != "40%" && plain.contains("spend-cap");
    pin(ok, "rig: EFFECTIVE derived, spend cap read", &plain);
    let text = rig.seam(t, GIB);
    let (fablex, micx, capx) = (
        block(&text, "fablex", &all),
        block(&text, "micx", &all),
        block(&text, "capx", &all),
    );
    for (word, cells) in specs.iter().zip(&oracle) {
        let (bucket, used, level) = (word[1], word[2], word[3]);
        let line = window(&block(&text, word[0], &all), bucket, used);
        pin(level == "-" || line.contains(level), bucket, &line);
        for cell in cells {
            pin(line.contains(cell.as_str()), cell, &line);
        }
    }
    // The fixture rollout is weeks old: its windows show no USED, but stay listed.
    let listed = |bucket: &str| {
        capx.iter()
            .filter(|line| line.split_whitespace().any(|word| word == bucket))
            .count()
    };
    let ok = listed("codex") == 1 && listed("codex_bengalfox") == 2 && says(&capx, "spend-cap");
    pin(ok, "stale windows listed, spend cap carried", &capx);
    for lines in [&fablex, &micx] {
        let ok = window(lines, "weekly_scoped", "%").contains("Fable");
        pin(ok, "model-scoped window listed with its qualifier", lines);
    }
    pin(says(&fablex, "first-sight"), "level labelled", &fablex);
    pin(!says(&micx, "66%"), "never another account's window", &micx);
}

/// Plan P15-P17: at most 128 sessions, newest recorded sign of life first; the rest are
/// counted as skipped and their totals partial; archives are out of scope; one tree at
/// one instant prints one byte sequence.
#[test]
fn the_session_bound_is_deterministic_counted_and_excludes_archives() {
    let rig = Rig::new("bound");
    rig.config("many = claude --model a\nold = claude --model b\narch = claude --model c\n");
    let all = ["many", "old", "arch"];
    let t = now();
    for n in 0..130_u32 {
        let mut rows = rig.seat(
            "main",
            if n < 2 { "old" } else { "many" },
            "claude",
            100 + n,
        );
        let _ = writeln!(rows, "launch_time.main={}", t - 200_000 + i64::from(n) * 60);
        rig.session(&format!("b{n:03}"), &rows);
    }
    let arch = rig.seat("main", "arch", "claude", 999);
    rig.write(
        &format!(".ae/archive/{}/meta", id(999)),
        format!("schema=2\n{arch}").as_bytes(),
    );
    let first = rig.seam(t, GIB);
    pin(rig.seam(t, GIB) == first, "deterministic", &first);
    let ok = has(&block(&first, "many", &all), "seats=128");
    pin(ok, "128 newest", &first);
    let ok = has(&block(&first, "arch", &all), "seats=0");
    pin(ok, "archives excluded", &first);
    let old = block(&first, "old", &all);
    let ok = says(&old, "partial") || says(&old, "at least");
    pin(ok, "skipped is partial", &old);
    let ok = counted(&first, "skipped", 2);
    pin(ok, "two oldest sessions skipped", &first);
    let label = "ordered by last recorded sign of life; no liveness probe";
    pin(first.contains(label), "order label", &first);
    let ok = rig.view().contains(label);
    pin(ok, "the CLI is the same view", &label);
}

/// Plan P17: at most 256 attributable seats; the rest are counted, never dropped.
#[test]
fn the_seat_bound_counts_what_it_did_not_read() {
    let rig = Rig::new("seats");
    rig.config("crowd = claude --model a\n");
    let t = now();
    for s in 0..3_u32 {
        let mut rows = format!("started={}\n", t - 1000 + i64::from(s));
        for n in 0..100_u32 {
            rows.push_str(&rig.seat(&format!("spawned.{n}"), "crowd", "claude", s * 100 + n));
        }
        let dir = rig.session(&format!("c{s}"), &rows);
        let seen = rig.reference("c", &dir, t).len();
        let ok = seen == 100;
        pin(ok, "rig: 100 seats/session", &seen);
    }
    let text = rig.seam(t, GIB);
    let crowd = block(&text, "crowd", &["crowd"]);
    pin(has(&crowd, "seats=256"), "256 read", &crowd);
    let ok = says(&crowd, "partial") || says(&crowd, "at least");
    pin(ok, "partial", &crowd);
    let ok = counted(&text, "skipped", 44);
    pin(ok, "44 seats over the bound counted", &text);
}

/// Plan P18: the transcript budget is whole-seat — a seat is read fully or named as not
/// read — and a seat it could not afford makes the totals partial, never a smaller sum.
#[test]
fn the_transcript_budget_reads_whole_seats_or_names_them() {
    let rig = Rig::new("budget");
    rig.config("p = claude --model fable\n");
    let body: &[u8] = include_bytes!("../fixtures/usage/claude-main.jsonl");
    let t = now();
    let mut dirs = Vec::new();
    for (sid, session, age) in [(70_u32, "tnew", 100), (71, "told", 1000)] {
        rig.claude(sid, body);
        let mut rows = rig.seat("main", "p", "claude", sid);
        let _ = writeln!(rows, "launch_time.main={}", t - age);
        dirs.push(rig.session(session, &rows));
    }
    let one = rig.reference("tnew", &dirs[0], t)[0].tokens.output;
    pin(one > 0, "rig precondition: transcript read", &one);
    let len = u64::try_from(body.len()).expect("small");
    let tight = rig.seam(t, len + len / 2);
    let lines = block(&tight, "p", &["p"]);
    let ok = has(&lines, "seats=2") && says(&lines, "partial");
    pin(ok, "both seen, partial", &lines);
    let ok = has(&lines, &format!("output={one}"));
    pin(ok, "only the newer seat read", &lines);
    let ok = names(&tight, "told", "budget");
    pin(ok, "the seat not read is named with its reason", &tight);
    let wide = rig.seam(t, 2 * len + 64);
    let lines = block(&wide, "p", &["p"]);
    let ok = has(&lines, &format!("output={}", 2 * one));
    pin(ok, "both read", &lines);
    pin(!names(&wide, "told", "budget"), "nothing skipped", &wide);
}

/// Plan P7/P8/P23 (colead 18e71035): a drift row naming another provider never
/// overrides transcript evidence; one read seat plus one missing transcript is partial,
/// and a missing transcript is never a zero.
#[test]
fn conflicting_and_missing_evidence_is_labelled_never_resolved_silently() {
    let rig = Rig::new("evidence");
    rig.config(
        "both = claude --model fable\nhalf = claude --model fable\nlost = claude --model fable\n",
    );
    let all = ["both", "half", "lost"];
    rig.claude(
        80,
        claude_line("claude-fable-5-1-20260901", 9, 5).as_bytes(),
    );
    rig.claude(
        81,
        claude_line("claude-fable-5-1-20260901", 9, 5).as_bytes(),
    );
    let mut rows = rig.seat("main", "both", "claude", 80);
    rows.push_str("observed_model.main=muse-spark-1.4\nobserved_model_pin.main=fable\n");
    rows.push_str(&rig.seat("worker.0", "half", "claude", 81));
    rows.push_str(&rig.seat("spawned.0", "half", "claude", 82));
    rows.push_str(&rig.seat("spawned.1", "lost", "claude", 83));
    let dir = rig.session("ev", &rows);
    let seats = rig.reference("ev", &dir, now());
    let read = |slot: &str| {
        seats
            .iter()
            .any(|seat| seat.slot == slot && seat.coverage == ae::usage::Coverage::Read)
    };
    let ok = read("main") && read("worker.0") && !read("spawned.0") && !read("spawned.1");
    pin(ok, "rig: 2 read, 2 missing", &seats);
    let text = rig.view();
    let both = block(&text, "both", &all);
    let ok = provider(&both).as_deref() != Some("Meta") && both[0].contains("(transcript)");
    let ok = ok && has(&both, "observed-model=yes/transcript");
    pin(ok, "drift never overrides the transcript", &both);
    let half = block(&text, "half", &all);
    let ok = has(&half, "seats=2") && has(&half, "output=5");
    pin(ok, "the read seat summed, the missing one not added", &half);
    let ok = says(&half, "partial") || says(&half, "at least");
    pin(ok, "known + missing is partial", &half);
    let lost = block(&text, "lost", &all);
    let ok = has(&lost, "seats=1") && provider(&lost).as_deref() == Some("unknown");
    pin(ok, "a seat with no transcript names no provider", &lost);
    for key in ["input", "output", "API-equivalent-USD"] {
        let ok = !numeric(&lost, key);
        pin(ok, "missing: no number", &lost);
    }
}

/// Plan P22: a retired-only profile's lifetime is its one unambiguous spawn->retire pair
/// (600 s here); a name spawned twice before its retire prints no number.
#[test]
fn retired_lifetime_is_spawn_to_retire_only_when_unambiguous() {
    let rig = Rig::new("retlife");
    rig.config("once = claude --model fable\nreuse = claude --model fable\n");
    let all = ["once", "reuse"];
    let home = rig.path("claude-home");
    for sid in [90, 91] {
        rig.claude(
            sid,
            claude_line("claude-fable-5-1-20260901", 3, 2).as_bytes(),
        );
    }
    let dir = rig.session("rl", "");
    let event = |minute: u32, action: &str, who: &str, extra: &str| {
        let slot = if who == "past" {
            "spawned.1"
        } else {
            "spawned.2"
        };
        format!(
            "{{\"ts\":\"2026-10-01T09:{minute:02}:00Z\",\"actor\":\"lead\",\"action\":\"{action}\",\"target\":\"{who}\",\"target_slot\":\"{slot}\"{extra}}}\n"
        )
    };
    let summary = |sid: u32, profile: &str| {
        format!(
            ",\"ref\":\"{}\",\"summary\":\"tool=claude profile={profile} config_home={} config_home_base=\"",
            id(sid),
            home.display()
        )
    };
    let journal = [
        event(0, "spawn", "past", ""),
        event(1, "spawn", "solo", ""),
        event(5, "spawn", "past", ""),
        event(10, "retire", "past", &summary(90, "reuse")),
        event(11, "retire", "solo", &summary(91, "once")),
    ];
    std::fs::write(dir.join("events.jsonl"), journal.concat()).expect("journal");
    let seats = rig.reference("rl", &dir, now());
    let read = seats
        .iter()
        .filter(|seat| seat.retired && seat.tokens.output == 2)
        .count();
    let ok = read == 2;
    pin(ok, "rig: both retired read", &seats);
    let text = rig.view();
    let lifetimes = |profile: &str| -> Vec<String> {
        let lines = block(&text, profile, &all);
        let cells = tokens(&lines)
            .into_iter()
            .filter(|cell| cell.contains("lifetime="));
        cells.collect()
    };
    let ok = has(&block(&text, "once", &all), "seats=1");
    pin(ok, "retired seat counted", &text);
    let ok = lifetimes("once").iter().any(|cell| cell.ends_with("=600s"));
    pin(ok, "spawn 09:01 -> retire 09:11", &text);
    let reuse = lifetimes("reuse");
    let ok = !reuse.is_empty() && reuse.iter().all(|cell| cell.ends_with("=unknown"));
    pin(ok, "two spawns, one retire: no number", &text);
}

/// Plan P17: the meta/journal byte budget (16 MiB) is deterministic; what it could not
/// read is named and makes the totals partial; under the budget it skips nothing.
#[test]
fn the_metadata_budget_counts_the_sessions_it_did_not_read() {
    let rig = Rig::new("metabudget");
    rig.config("j = claude --model fable\n");
    let t = now();
    let memo = r#"{"ts":"2026-10-01T09:00:00Z","actor":"lead","action":"memo","summary":"X"}"#;
    let journal = format!("{}\n", memo.replace('X', &"x".repeat(1000))).repeat(3 * 1024);
    let usage = claude_line("claude-fable-5-1-20260901", 1, 1);
    for n in 0..8_u32 {
        rig.claude(200 + n, usage.as_bytes());
        let dir = rig.session(&format!("j{n}"), &rig.seat("main", "j", "claude", 200 + n));
        std::fs::write(dir.join("events.jsonl"), &journal).expect("journal");
        let ok = rig.reference("j", &dir, t)[0].tokens.output == 1;
        pin(ok, "rig: every seat's transcript is read", &n);
    }
    let text = rig.seam(t, GIB);
    pin(rig.seam(t, GIB) == text, "deterministic", &text);
    let lines = block(&text, "j", &["j"]);
    let ok = says(&lines, "partial") || says(&lines, "at least");
    pin(ok, "8 x 3 MiB journals overrun 16 MiB: partial", &lines);
    let ok = names(&text, "skipped", "budget");
    pin(ok, "budget skip named", &text);
    for n in 0..4 {
        let journal = rig.path(&format!(".ae/sessions/j{n}/events.jsonl"));
        std::fs::write(journal, "").expect("empty four journals");
    }
    // Budget-specific evidence only: usage partial (P23) is never banned here.
    let under = rig.seam(t, GIB);
    let ok = !names(&under, "skipped", "budget") && !names(&under, "partial", "budget");
    pin(
        ok,
        "4 x 3 MiB fits: no budget skip or budget partial",
        &under,
    );
}
