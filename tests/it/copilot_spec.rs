//! Frozen copilot acceptance: `ae quota` shows the GitHub Copilot
//! premium-request allowance from an opt-in token file.
//!
//! Oracle: `.local/brief-copilot.md` invariants I1–I8, the lead's I6 ruling
//! (2026-10-23, option A: display-only), and the response schema in
//! `.local/copilotquota/digest.md` §3.4 (Tier B/C, so every field is optional).
//! Never the code under test.
//!
//! BLACK-BOX by construction: every behaviour is driven through the `ae quota`
//! binary against an in-test loopback HTTP server named by the CHECKOUT-only
//! `AE_TEST_COPILOT_USAGE_URL`, so this file compiles and lints on a base that
//! has no Copilot code at all and no test reaches `api.github.com`.
//!
//! Pins this file freezes:
//! * `[workspace] copilot_token_file` is global-only; a project `.ae/config`
//!   never votes. `quota = off`, `ae doctor` and the watchdog make no request.
//! * The request is a GET carrying `Authorization: token <T>`, `Editor-Version`,
//!   `Editor-Plugin-Version`, `User-Agent` and `Accept: application/json`; the
//!   token appears nowhere else — not on stdout/stderr, not in the state tree.
//! * One request per TTL for the fleet, failure included; a rotated token is due.
//! * USED% = (entitlement − remaining) / entitlement × 100 when both counts are
//!   valid integral numbers (counts win over `percent_remaining`); only when
//!   BOTH counts are absent, 100 − `percent_remaining`. Anything else is a note,
//!   never a number. `unlimited: true` reads `unlimited` in CREDITS.
//! * One `github copilot` group however many profiles map to it, shown with
//!   `-` PROFILES when none do; `opencode --model|-m github-copilot/<m>`
//!   profiles map there, other opencode profiles stay today's unsupported row.
//! * 401/403 → `token rejected (<code>)` + `copilot_token_file`; failure notes
//!   say `ago` once; vendor text never reaches the output.
//! * r1.1: the premium row's WINDOW and PACE cells are `-` on a fresh row; a
//!   second `ae quota` during an in-flight fetch neither waits nor fetches.
//! * A Copilot note is a line below the table that starts `github copilot:`,
//!   as a Go note starts `opencode go:`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private scratch files and a loopback server, and reads their effects"
)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ae::time::Timestamp;

const SECRET: &str = "SYNTH-COPILOT-TOKEN-LEAK-9d41";
const VENDOR_TEXT: &str = "VENDOR-TEXT-7731";
const SEAM: &str = "AE_TEST_COPILOT_USAGE_URL";
const SETTING: &str = "copilot_token_file";
const UNSUPPORTED_HINT: &str = "local stats are cost history, not quota";

// --- loopback server -------------------------------------------------------

/// What the loopback endpoint answers with.
#[derive(Clone)]
enum Reply {
    /// A status line, a JSON body and optional extra header lines.
    Status(u16, String, String),
    /// Accept, read the request, then answer nothing until the test ends.
    Silent,
    /// Hold the response until [`Server::release`], then answer 200 with
    /// this body; a client that hangs up first marks the server abandoned.
    Hold(String),
}

/// What the server thread and every connection thread share.
struct Shared {
    reply: Reply,
    url: String,
    seen: Mutex<Vec<String>>,
    stop: AtomicBool,
    release: AtomicBool,
    abandoned: AtomicBool,
}

/// A one-endpoint HTTP/1.1 server on 127.0.0.1 that records every request
/// head it receives, one thread per connection so a held answer never
/// queues the next client. Dropped, it stops and joins its threads.
struct Server {
    url: String,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let port = listener.local_addr().expect("local addr").port();
        let url = format!("http://127.0.0.1:{port}/copilot_internal/user");
        let shared = Arc::new(Shared {
            reply,
            url: url.clone(),
            seen: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            release: AtomicBool::new(false),
            abandoned: AtomicBool::new(false),
        });
        let handle = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || serve(&listener, &shared))
        };
        Self {
            url,
            shared,
            handle: Some(handle),
        }
    }

    fn ok(body: &str) -> Self {
        Self::start(Reply::Status(200, body.to_owned(), String::new()))
    }

    fn requests(&self) -> Vec<String> {
        self.shared.seen.lock().expect("request log").clone()
    }

    fn count(&self) -> usize {
        self.requests().len()
    }

    /// Let a [`Reply::Hold`] answer.
    fn release(&self) {
        self.shared.release.store(true, Ordering::SeqCst);
    }

    /// Whether a held client hung up before its answer.
    fn abandoned(&self) -> bool {
        self.shared.abandoned.load(Ordering::SeqCst)
    }

    /// Block until `n` request heads arrived, at most `limit`.
    fn wait_for(&self, n: usize, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            if self.count() >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.count() >= n
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(listener: &TcpListener, shared: &Arc<Shared>) {
    let deadline = Instant::now() + Duration::from_mins(1);
    let mut connections = Vec::new();
    while !shared.stop.load(Ordering::SeqCst) && Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = Arc::clone(shared);
                connections.push(std::thread::spawn(move || answer(stream, &shared)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
    for connection in connections {
        let _ = connection.join();
    }
}

fn answer(mut stream: TcpStream, shared: &Shared) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut head = Vec::new();
    let mut buf = [0_u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 64 * 1024 {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    shared
        .seen
        .lock()
        .expect("request log")
        .push(String::from_utf8_lossy(&head).into_owned());
    let until = Instant::now() + Duration::from_secs(20);
    match &shared.reply {
        Reply::Silent => {
            while !shared.stop.load(Ordering::SeqCst) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        Reply::Hold(body) => {
            let _ = stream.set_nonblocking(true);
            let mut probe = [0_u8; 1];
            while !shared.release.load(Ordering::SeqCst)
                && !shared.stop.load(Ordering::SeqCst)
                && Instant::now() < until
            {
                if matches!(stream.peek(&mut probe), Ok(0)) {
                    shared.abandoned.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = stream.set_nonblocking(false);
            respond(&mut stream, 200, body, "");
        }
        Reply::Status(code, body, extra) => {
            respond(
                &mut stream,
                *code,
                body,
                &extra.replace("{url}", &shared.url),
            );
        }
    }
}

fn respond(stream: &mut TcpStream, code: u16, body: &str, extra: &str) {
    let reason = match code {
        200 => "OK",
        302 => "Found",
        401 => "Unauthorized",
        403 => "Forbidden",
        _ => "Status",
    };
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

// --- bodies ----------------------------------------------------------------

/// A reset well ahead of the wall clock, so the child's row reads Fresh.
fn future_reset() -> String {
    Timestamp::parse("2099-01-01T00:00:00Z")
        .expect("UTC literal")
        .to_string()
}

/// A body in the digest §3.4 shape with the `premium_interactions` fields given.
fn body_with(plan: &str, premium: &str) -> String {
    format!(
        r#"{{"login":"octocat","copilot_plan":"{plan}","chat_enabled":true,"quota_reset_date":"{reset}","quota_snapshots":{{"premium_interactions":{{{premium}}},"chat":{{"entitlement":0,"remaining":0,"percent_remaining":100.0,"unlimited":true}}}},"endpoints":{{"api":"https://api.githubcopilot.com"}}}}"#,
        reset = future_reset()
    )
}

/// 300 allowed, 210 left => 30% used; the inconsistent `percent_remaining`
/// (99 => 1%) proves the counts win.
fn counts_body() -> String {
    body_with(
        "individual",
        r#""entitlement":300,"remaining":210,"percent_remaining":99.0,"unlimited":false,"overage":false,"overage_entitlement":0"#,
    )
}

// --- rig -------------------------------------------------------------------

struct Rig {
    root: super::cli::OwnedScratch,
    config: PathBuf,
    state: PathBuf,
    token: PathBuf,
}

impl Rig {
    fn new(tag: &str, config: &str) -> Self {
        let root = super::cli::OwnedScratch::root("cq", tag);
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).expect("state dir");
        std::fs::create_dir_all(root.path().join(".ae")).expect("project overlay dir");
        let config_path = root.path().join("config");
        std::fs::write(&config_path, config).expect("global config");
        Self {
            config: config_path,
            state,
            token: root.path().join("copilot.token"),
            root,
        }
    }

    /// The global config naming the token file, with these `[profiles]` rows.
    fn with_token(tag: &str, profiles: &str) -> Self {
        let rig = Self::new(
            tag,
            &format!(
                "[workspace]\n{SETTING} = $HOME/copilot.token\n\
                 [clients]\nopencode = opencode\n\
                 [profiles]\n{profiles}"
            ),
        );
        rig.write_token(&format!("{SECRET}\n"), 0o600);
        rig
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn write_token(&self, body: &str, mode: u32) {
        let _ = std::fs::remove_file(&self.token);
        std::fs::write(&self.token, body).expect("token file");
        std::fs::set_permissions(&self.token, std::fs::Permissions::from_mode(mode))
            .expect("token mode");
    }

    /// One hermetic child of `ae <args>` against `url`; returns (rc, stdout, stderr).
    fn run(&self, url: &str, args: &[&str]) -> (Option<i32>, String, String) {
        let output = super::cli::ae()
            .env("HOME", self.path())
            .env("AE_HOME", &self.state)
            .env("CONFIG_FILE", &self.config)
            .env(SEAM, url)
            .current_dir(self.path())
            .args(args)
            .output()
            .expect("hermetic ae");
        let out = String::from_utf8(output.stdout).expect("UTF-8 stdout");
        let err = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(!out.contains(SECRET), "token leaked on stdout: {out}");
        assert!(!err.contains(SECRET), "token leaked on stderr: {err}");
        (output.status.code(), out, err)
    }

    /// `ae quota` against `server`; exit 0 and the table.
    fn table(&self, server: &Server) -> String {
        self.table_at(&server.url)
    }

    fn table_at(&self, url: &str) -> String {
        let (code, out, err) = self.run(url, &["quota"]);
        assert_eq!(code, Some(0), "ae quota exit: {out}\n{err}");
        out
    }

    /// No file under the state tree holds the token.
    fn assert_state_clean(&self) {
        let mut stack = vec![self.state.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("state dir").flatten() {
                let path = entry.path();
                let meta = std::fs::symlink_metadata(&path).expect("state entry");
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    let bytes = std::fs::read(&path).expect("state file");
                    assert!(
                        !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
                        "token bytes in {}",
                        path.display()
                    );
                }
            }
        }
    }
}

// --- table readers ---------------------------------------------------------

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Notes render below the table, indented to the SCOPE column
/// (`quota.rs` `RenderLine::Summary`); a Copilot note starts with this prefix,
/// as a Go note starts with `opencode go:`.
const NOTE_PREFIX: &str = "github copilot:";

fn is_copilot_note(line: &str) -> bool {
    line.trim_start().to_lowercase().starts_with(NOTE_PREFIX)
}

/// The rows of the group whose first line starts at `start`: that line plus
/// every continuation line (blank first column) up to the first note, collapsed.
fn block_from(lines: &[&str], start: usize) -> String {
    let mut end = start + 1;
    while end < lines.len()
        && lines[end].chars().next().is_none_or(char::is_whitespace)
        && !is_copilot_note(lines[end])
    {
        end += 1;
    }
    collapsed(&lines[start..end].join("\n"))
}

/// Every table line that opens a group labelled GitHub Copilot.
fn copilot_starts(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            line.to_lowercase().contains("github copilot") && !line.starts_with(char::is_whitespace)
        })
        .map(|(index, _)| index)
        .collect()
}

/// The one Copilot group, collapsed, and its PROFILES cell: the words on its
/// first line before the label.
fn copilot_block(text: &str) -> (String, String) {
    let lines: Vec<&str> = text.lines().collect();
    let starts = copilot_starts(text);
    assert_eq!(starts.len(), 1, "exactly one github copilot group: {text}");
    let start = starts[0];
    let first = lines[start];
    let label = first
        .to_lowercase()
        .find("github copilot")
        .expect("label on the first line");
    let profiles = collapsed(first.get(..label).unwrap_or_default());
    (block_from(&lines, start), profiles)
}

/// The block of the profile row whose first word is `name`.
fn profile_block(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.split_whitespace().next() == Some(name))?;
    Some(block_from(&lines, start))
}

/// Every `N%` cell in a block, as numbers.
fn percents(block: &str) -> Vec<f64> {
    block
        .split_whitespace()
        .filter_map(|word| word.strip_suffix('%'))
        .filter_map(|number| number.parse::<f64>().ok())
        .collect()
}

fn assert_no_percent(block: &str, why: &str) {
    assert!(
        percents(block).is_empty(),
        "{why}: no number may be invented: {block}"
    );
}

/// The Copilot block reads `expected` percent used (USED and EFFECTIVE alike).
fn assert_used(text: &str, expected: f64) {
    let (block, _) = copilot_block(text);
    let found = percents(&block);
    assert!(
        !found.is_empty() && found.iter().all(|value| (value - expected).abs() < 0.051),
        "expected {expected}% used: {block}"
    );
}

/// The cell of `line` under the header column `name`: the run of
/// non-blank characters starting at that column (cells are left-aligned,
/// padded by character count).
fn cell(text: &str, line: &str, name: &str) -> String {
    let header = text
        .lines()
        .find(|row| row.starts_with("PROFILES"))
        .expect("table header");
    let column = header
        .find(name)
        .unwrap_or_else(|| panic!("no {name} column: {header}"));
    let column = header[..column].chars().count();
    line.chars()
        .skip(column)
        .take_while(|c| !c.is_whitespace())
        .collect()
}

/// The Copilot notes below the table, trimmed.
fn copilot_notes(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| is_copilot_note(line))
        .map(|line| line.trim().to_owned())
        .collect()
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn product_text(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).expect(path)
}

fn product_part(path: &str) -> String {
    let src = product_text(path);
    src.split_once("#[cfg(test)]\nmod tests {")
        .map_or(src.as_str(), |(product, _)| product)
        .to_owned()
}

const ONE_PROFILE: &str = "cp = opencode --model github-copilot/gpt-5\n";

// --- the request ------------------------------------------------------------

#[test]
fn the_request_is_one_get_with_the_token_header_and_the_editor_headers() {
    let rig = Rig::with_token("hdr", ONE_PROFILE);
    let server = Server::ok(&counts_body());
    let _ = rig.table(&server);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "one request: {requests:?}");
    let request = &requests[0];
    let line = request.lines().next().unwrap_or_default();
    assert!(line.starts_with("GET "), "a GET: {line}");
    assert!(!line.contains(SECRET), "token never in the URL: {line}");
    assert_eq!(
        header(request, "authorization"),
        Some(format!("token {SECRET}").as_str()),
        "Authorization: token <T>"
    );
    for name in ["editor-version", "editor-plugin-version", "user-agent"] {
        let value = header(request, name).unwrap_or_default();
        assert!(!value.is_empty(), "{name} header: {request}");
        assert!(!value.contains(SECRET), "{name} carries no token");
    }
    assert_eq!(header(request, "accept"), Some("application/json"));
    let elsewhere = request.lines().filter(|line| line.contains(SECRET)).count();
    assert_eq!(elsewhere, 1, "the token rides exactly one header line");
    rig.assert_state_clean();
}

// --- display ----------------------------------------------------------------

#[test]
fn counts_set_used_percent_and_the_group_names_github_copilot_and_its_plan() {
    let rig = Rig::with_token("counts", ONE_PROFILE);
    let server = Server::ok(&counts_body());
    let text = rig.table(&server);
    let (block, profiles) = copilot_block(&text);
    assert!(block.contains("individual"), "plan in the label: {block}");
    assert!(block.contains("premium"), "premium bucket: {block}");
    assert_eq!(profiles, "cp", "the mapped profile owns the group: {text}");
    assert_used(&text, 30.0);
    assert!(
        block.contains("in "),
        "RESETS from quota_reset_date: {block}"
    );
    let cp = profile_block(&text, "cp").expect("cp row");
    assert!(
        !cp.contains("unsupported"),
        "a github-copilot profile leaves the unsupported row: {cp}"
    );
    rig.assert_state_clean();
}

#[test]
fn percent_remaining_is_the_fallback_only_when_both_counts_are_absent() {
    let rig = Rig::with_token("fallback", ONE_PROFILE);
    let server = Server::ok(&body_with(
        "business",
        r#""percent_remaining":75,"unlimited":false"#,
    ));
    let text = rig.table(&server);
    assert_used(&text, 25.0);
    assert!(copilot_block(&text).0.contains("business"), "{text}");
}

#[test]
fn a_decimal_percent_remaining_falls_back_to_its_complement() {
    let rig = Rig::with_token("decimal", ONE_PROFILE);
    let server = Server::ok(&body_with(
        "enterprise",
        r#""percent_remaining":63.1,"unlimited":false"#,
    ));
    let text = rig.table(&server);
    assert_used(&text, 36.9);
}

#[test]
fn supplied_but_unusable_counts_are_a_note_and_never_fall_back() {
    for (tag, premium) in [
        (
            "zeroent",
            r#""entitlement":0,"remaining":0,"percent_remaining":50"#,
        ),
        (
            "over",
            r#""entitlement":300,"remaining":301,"percent_remaining":50"#,
        ),
        (
            "negrem",
            r#""entitlement":300,"remaining":-1,"percent_remaining":50"#,
        ),
        (
            "negent",
            r#""entitlement":-5,"remaining":1,"percent_remaining":50"#,
        ),
        (
            "strent",
            r#""entitlement":"300","remaining":210,"percent_remaining":50"#,
        ),
        (
            "fracrem",
            r#""entitlement":300,"remaining":1.5,"percent_remaining":50"#,
        ),
        ("onlyent", r#""entitlement":300,"percent_remaining":50"#),
        ("onlyrem", r#""remaining":210,"percent_remaining":50"#),
        (
            "nullrem",
            r#""entitlement":300,"remaining":null,"percent_remaining":50"#,
        ),
    ] {
        let rig = Rig::with_token(tag, ONE_PROFILE);
        let server = Server::ok(&body_with("individual", premium));
        let text = rig.table(&server);
        assert_eq!(server.count(), 1, "{tag}: the call was made");
        let (block, _) = copilot_block(&text);
        assert_no_percent(&block, tag);
        assert!(
            !copilot_notes(&text).is_empty(),
            "{tag}: an unusable count is a note: {text}"
        );
    }
}

#[test]
fn an_unusable_percent_remaining_without_counts_is_a_note_and_no_number() {
    for (tag, premium) in [
        ("p150", r#""percent_remaining":150"#),
        ("pneg", r#""percent_remaining":-1"#),
        ("pstr", r#""percent_remaining":"50""#),
        ("pnone", r#""unlimited":false"#),
    ] {
        let rig = Rig::with_token(tag, ONE_PROFILE);
        let server = Server::ok(&body_with("individual", premium));
        let text = rig.table(&server);
        assert_eq!(server.count(), 1, "{tag}: the call was made");
        assert_no_percent(&copilot_block(&text).0, tag);
        assert!(
            !copilot_notes(&text).is_empty(),
            "{tag}: missing number is a note: {text}"
        );
    }
}

#[test]
fn unlimited_reads_unlimited_in_credits_and_never_divides_by_zero() {
    let rig = Rig::with_token("unlim", ONE_PROFILE);
    let server = Server::ok(&body_with(
        "enterprise",
        r#""entitlement":0,"remaining":0,"percent_remaining":100.0,"unlimited":true"#,
    ));
    let text = rig.table(&server);
    let (block, _) = copilot_block(&text);
    assert!(block.contains("unlimited"), "CREDITS unlimited: {block}");
    let lower = text.to_lowercase();
    assert!(
        !lower.contains("nan") && !lower.contains("inf%"),
        "no NaN/inf: {text}"
    );
}

#[test]
fn a_reset_with_an_offset_is_converted_to_utc() {
    // Reset 10d 5h from now, spelled at +02:00. Ignoring the offset would read
    // two hours off; the elapsed seconds of the run may cost one hour.
    let reset = Timestamp::now().epoch() + 10 * 86_400 + 5 * 3_600 + 1_800;
    let spelled = Timestamp::from_epoch(reset + 7_200)
        .to_string()
        .replace('Z', "+02:00");
    let body = counts_body().replace(&future_reset(), &spelled);
    let rig = Rig::with_token("offset", ONE_PROFILE);
    let server = Server::ok(&body);
    let text = rig.table(&server);
    let (block, _) = copilot_block(&text);
    assert!(
        block.contains("in 10d 5h") || block.contains("in 10d 4h"),
        "reset from {spelled}: {block}"
    );
}

#[test]
fn hostile_plan_reads_unknown_and_is_never_printed() {
    let rig = Rig::with_token("plan", ONE_PROFILE);
    let body = counts_body().replace(
        "\"copilot_plan\":\"individual\"",
        &format!("\"copilot_plan\":\"{VENDOR_TEXT}\\u001b[31m\""),
    );
    let server = Server::ok(&body);
    let text = rig.table(&server);
    assert!(!text.contains(VENDOR_TEXT), "vendor plan echoed: {text}");
    assert!(!text.contains('\u{1b}'), "control byte reached the table");
    let (block, _) = copilot_block(&text);
    assert!(block.contains("unknown"), "plan unknown: {block}");
    assert_used(&text, 30.0);
}

#[test]
fn an_object_without_premium_interactions_says_the_account_has_none() {
    for (tag, body) in [
        (
            "nosnap",
            r#"{"login":"octocat","copilot_plan":"individual"}"#.to_owned(),
        ),
        (
            "nopremium",
            r#"{"copilot_plan":"individual","quota_snapshots":{"chat":{"unlimited":true}}}"#
                .to_owned(),
        ),
    ] {
        let rig = Rig::with_token(tag, ONE_PROFILE);
        let server = Server::ok(&body);
        let text = rig.table(&server);
        assert_no_percent(&copilot_block(&text).0, tag);
        let lower = text.to_lowercase();
        assert!(
            lower.contains("no copilot") || lower.contains("no premium"),
            "{tag}: say the account has no premium quota: {text}"
        );
    }
}

#[test]
fn unparseable_answers_are_named_and_invent_nothing() {
    for (tag, body) in [
        ("notjson", "not json".to_owned()),
        ("array", "[]".to_owned()),
        ("snapnum", r#"{"quota_snapshots":5}"#.to_owned()),
        (
            "message",
            format!(r#"{{"message":"{VENDOR_TEXT}","documentation_url":"x"}}"#),
        ),
    ] {
        let rig = Rig::with_token(tag, ONE_PROFILE);
        let server = Server::ok(&body);
        let text = rig.table(&server);
        assert_eq!(server.count(), 1, "{tag}: the call was made");
        assert_no_percent(&copilot_block(&text).0, tag);
        assert!(
            !copilot_notes(&text).is_empty(),
            "{tag}: a note names it: {text}"
        );
        assert!(!text.contains(VENDOR_TEXT), "{tag}: vendor text: {text}");
    }
}

// --- profiles and scope -----------------------------------------------------

#[test]
fn every_github_copilot_profile_maps_to_one_group_and_others_stay_unsupported() {
    let rig = Rig::new(
        "profiles",
        &format!(
            "[workspace]\n{SETTING} = $HOME/copilot.token\n\
             [clients]\nopencode = opencode\noc2 = opencode\n\
             [profiles]\n\
             cpa = opencode --model github-copilot/gpt-5\n\
             cpb = oc2 -m github-copilot/claude-x\n\
             ns = opencode --model github-copilot\n\
             or = opencode --model openrouter/foo\n"
        ),
    );
    rig.write_token(SECRET, 0o600);
    let server = Server::ok(&counts_body());
    let text = rig.table(&server);
    assert_eq!(server.count(), 1, "one account, one request");
    let (_, profiles) = copilot_block(&text);
    let words: Vec<&str> = profiles.split_whitespace().collect();
    assert!(
        words.contains(&"cpa") && words.contains(&"cpb"),
        "both mapped profiles on the one group: {profiles}"
    );
    assert!(
        !words.contains(&"ns") && !words.contains(&"or"),
        "unmapped profiles stay out: {profiles}"
    );
    for name in ["ns", "or"] {
        let row = profile_block(&text, name).expect("unmapped profile row");
        assert!(
            row.contains("unsupported") && row.contains(UNSUPPORTED_HINT),
            "{name} stays today's unsupported row: {row}"
        );
    }
}

#[test]
fn a_token_with_no_copilot_profile_still_shows_the_group_with_dash_profiles() {
    let rig = Rig::with_token("noprof", "or = opencode --model openrouter/foo\n");
    let server = Server::ok(&counts_body());
    let text = rig.table(&server);
    assert_eq!(server.count(), 1, "the group is fetched without profiles");
    let (_, profiles) = copilot_block(&text);
    assert_eq!(profiles, "-", "no profiles spelled -: {text}");
    assert_used(&text, 30.0);
}

#[test]
fn no_token_setting_shows_no_copilot_group_and_asks_nothing() {
    let rig = Rig::new(
        "nosetting",
        &format!("[clients]\nopencode = opencode\n[profiles]\n{ONE_PROFILE}"),
    );
    rig.write_token(SECRET, 0o600);
    let server = Server::ok(&counts_body());
    let text = rig.table(&server);
    assert_eq!(server.count(), 0, "nothing runs without the opt-in");
    assert!(copilot_starts(&text).is_empty(), "{text}");
}

// --- opt-in and custody -----------------------------------------------------

#[test]
fn a_project_config_naming_the_token_never_votes() {
    let rig = Rig::new(
        "local",
        &format!("[clients]\nopencode = opencode\n[profiles]\n{ONE_PROFILE}"),
    );
    std::fs::write(
        rig.path().join(".ae/config"),
        format!("[workspace]\n{SETTING} = $HOME/copilot.token\n"),
    )
    .expect("project overlay");
    rig.write_token(SECRET, 0o600);
    let server = Server::ok(&counts_body());
    let text = rig.table(&server);
    assert_eq!(server.count(), 0, "a project .ae/config must not opt in");
    assert!(copilot_starts(&text).is_empty(), "{text}");
}

#[test]
fn quota_off_makes_no_request() {
    let rig = Rig::new(
        "off",
        &format!(
            "[workspace]\nquota = off\n{SETTING} = $HOME/copilot.token\n\
             [clients]\nopencode = opencode\n[profiles]\n{ONE_PROFILE}"
        ),
    );
    rig.write_token(SECRET, 0o600);
    let server = Server::ok(&counts_body());
    let _ = rig.table(&server);
    assert_eq!(server.count(), 0, "quota = off => no Copilot network");
}

#[test]
fn a_group_or_world_readable_token_is_refused_with_chmod_600() {
    for (tag, mode) in [("g640", 0o640), ("w644", 0o644)] {
        let rig = Rig::with_token(tag, ONE_PROFILE);
        rig.write_token(SECRET, mode);
        let server = Server::ok(&counts_body());
        let text = rig.table(&server);
        assert_eq!(server.count(), 0, "{tag}: an exposed token is never sent");
        assert!(text.contains("chmod 600"), "{tag}: {text}");
        assert!(text.contains("copilot.token"), "{tag}: path named: {text}");
        rig.assert_state_clean();
    }
}

#[test]
fn a_missing_directory_or_empty_token_file_is_refused_without_a_request() {
    let rig = Rig::with_token("missing", ONE_PROFILE);
    std::fs::remove_file(&rig.token).expect("remove token");
    let server = Server::ok(&counts_body());
    let _ = rig.table(&server);
    assert_eq!(server.count(), 0, "absent token file");

    let rig = Rig::with_token("dir", ONE_PROFILE);
    std::fs::remove_file(&rig.token).expect("remove token");
    std::fs::create_dir(&rig.token).expect("token path as a directory");
    let server = Server::ok(&counts_body());
    let _ = rig.table(&server);
    assert_eq!(server.count(), 0, "a directory is not a token file");

    let rig = Rig::with_token("empty", ONE_PROFILE);
    rig.write_token("  \n", 0o600);
    let server = Server::ok(&counts_body());
    let _ = rig.table(&server);
    assert_eq!(server.count(), 0, "a blank token is not sent");
}

// --- one fetch for the fleet -------------------------------------------------

#[test]
fn a_second_quota_inside_the_ttl_asks_nothing_and_a_rotated_token_is_due() {
    let rig = Rig::with_token("ttl", ONE_PROFILE);
    let server = Server::ok(&counts_body());
    let first = rig.table(&server);
    assert_used(&first, 30.0);
    let second = rig.table(&server);
    assert_eq!(server.count(), 1, "the cache answers inside the TTL");
    assert_used(&second, 30.0);
    rig.write_token(&format!("{SECRET}-ROTATED\n"), 0o600);
    let _ = rig.table(&server);
    assert_eq!(
        server.count(),
        2,
        "a rotated token makes the next refresh due"
    );
    let rotated = format!("token {SECRET}-ROTATED");
    assert_eq!(
        header(&server.requests()[1], "authorization"),
        Some(rotated.as_str())
    );
    rig.assert_state_clean();
}

#[test]
fn a_failed_attempt_is_recorded_too_and_is_not_retried_inside_the_ttl() {
    let rig = Rig::with_token("failttl", ONE_PROFILE);
    let server = Server::start(Reply::Status(
        500,
        format!(r#"{{"message":"{VENDOR_TEXT}"}}"#),
        String::new(),
    ));
    let first = rig.table(&server);
    let second = rig.table(&server);
    assert_eq!(server.count(), 1, "a failure answers for the TTL as well");
    for text in [&first, &second] {
        assert!(!copilot_notes(text).is_empty(), "failure named: {text}");
        assert!(!text.contains(VENDOR_TEXT), "vendor text: {text}");
    }
}

// --- failures ---------------------------------------------------------------

#[test]
fn rejected_tokens_name_the_code_and_the_setting_with_one_ago() {
    for code in [401_u16, 403] {
        let rig = Rig::with_token(&format!("rej{code}"), ONE_PROFILE);
        let server = Server::start(Reply::Status(
            code,
            format!(r#"{{"message":"{VENDOR_TEXT}"}}"#),
            String::new(),
        ));
        let text = rig.table(&server);
        let needle = format!("token rejected ({code})");
        let note = text
            .lines()
            .find(|line| line.contains(&needle))
            .unwrap_or_else(|| panic!("{needle} missing: {text}"));
        assert!(text.contains(SETTING), "{code}: setting named: {text}");
        assert_eq!(note.matches("ago").count(), 1, "{code}: one ago: {note}");
        assert!(!text.contains(VENDOR_TEXT), "{code}: vendor text: {text}");
        assert_no_percent(&copilot_block(&text).0, "rejected");
    }
}

#[test]
fn a_redirect_is_never_followed() {
    let rig = Rig::with_token("redirect", ONE_PROFILE);
    let server = Server::start(Reply::Status(
        302,
        String::new(),
        "Location: {url}?again=1\r\n".to_owned(),
    ));
    let text = rig.table(&server);
    assert_eq!(server.count(), 1, "the Location is not asked");
    assert_no_percent(&copilot_block(&text).0, "redirect");
    assert!(!copilot_notes(&text).is_empty(), "redirect named: {text}");
}

#[test]
fn an_oversized_answer_is_refused_and_not_parsed() {
    let rig = Rig::with_token("huge", ONE_PROFILE);
    let pad = "x".repeat(64 * 1024 + 1);
    let body = counts_body().replacen('{', &format!("{{\"pad\":\"{pad}\","), 1);
    let server = Server::ok(&body);
    let text = rig.table(&server);
    assert_eq!(server.count(), 1);
    assert_no_percent(&copilot_block(&text).0, "oversize");
    assert!(!copilot_notes(&text).is_empty(), "size cap named: {text}");
}

#[test]
fn an_unreachable_endpoint_is_named() {
    let rig = Rig::with_token("unreach", ONE_PROFILE);
    let text = rig.table_at("http://127.0.0.1:9/copilot_internal/user");
    assert_no_percent(&copilot_block(&text).0, "unreachable");
    let notes = copilot_notes(&text).join("\n").to_lowercase();
    assert!(notes.contains("connect"), "unreachable named: {text}");
    assert_eq!(
        text.lines()
            .find(|line| line.to_lowercase().contains("connect"))
            .map(|line| line.matches("ago").count()),
        Some(1),
        "one ago: {text}"
    );
}

#[test]
fn a_silent_endpoint_hits_the_timeout_and_is_named() {
    let rig = Rig::with_token("silent", ONE_PROFILE);
    let server = Server::start(Reply::Silent);
    let started = Instant::now();
    let text = rig.table(&server);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the call is bounded"
    );
    assert_eq!(server.count(), 1);
    assert_no_percent(&copilot_block(&text).0, "timeout");
    let notes = copilot_notes(&text).join("\n").to_lowercase();
    assert!(notes.contains("timeout"), "timeout named: {text}");
}

// --- window length and fleet contention (r1.1) -------------------------------

/// Oracle: brief I4 — the window is monthly / unstated and must never read as
/// a 5h/7d window or feed PACE, however distant the reset. STATUS `fresh` and
/// USED `30%` prove the row is trustworthy, so `-` here is the window's own
/// spelling, not the untrusted-row blank.
#[test]
fn the_premium_window_states_no_length_and_no_pace_despite_a_distant_reset() {
    let rig = Rig::with_token("window", ONE_PROFILE);
    let server = Server::ok(&counts_body());
    let fetched = rig.table(&server);
    let cached = rig.table(&server);
    for text in [&fetched, &cached] {
        let start = copilot_starts(text);
        assert_eq!(start.len(), 1, "one group: {text}");
        let line = text.lines().nth(start[0]).expect("group line");
        assert_eq!(
            cell(text, line, "STATUS"),
            "fresh",
            "trustworthy row: {text}"
        );
        assert_eq!(cell(text, line, "USED"), "30%", "{text}");
        assert!(cell(text, line, "RESETS").starts_with("in"), "{text}");
        assert_eq!(cell(text, line, "WINDOW"), "-", "unstated length: {text}");
        assert_eq!(cell(text, line, "PACE"), "-", "no pace input: {text}");
    }
    assert_eq!(server.count(), 1);
}

/// Oracle: brief I2 — one fetch for the fleet behind a ZERO-WAIT lock. The
/// first `ae quota` holds its request open at the server; a second one must
/// finish while that request is still open (it neither waits on the answer nor
/// outlives the first's own timeout, which would hang the held socket up) and
/// must make no request of its own. Causal, not timed: the only budget is the
/// first call's response timeout, which a waiting second run would exhaust.
#[test]
fn a_second_quota_during_an_in_flight_fetch_neither_waits_nor_fetches_again() {
    let rig = Rig::with_token("inflight", ONE_PROFILE);
    let server = Server::start(Reply::Hold(counts_body()));
    std::thread::scope(|scope| {
        let first = scope.spawn(|| rig.table(&server));
        assert!(
            server.wait_for(1, Duration::from_secs(20)),
            "the first ae quota's request reached the server"
        );
        let second = rig.table(&server);
        assert!(
            !server.abandoned(),
            "the second run returned only after the held request was given up: it waited"
        );
        assert_eq!(
            server.count(),
            1,
            "no second fetch while one is in flight: {second}"
        );
        assert_eq!(
            copilot_starts(&second).len(),
            1,
            "group still shown: {second}"
        );
        server.release();
        let first = first.join().expect("first ae quota");
        assert_used(&first, 30.0);
    });
    assert_eq!(server.count(), 1, "one request in all");
    rig.assert_state_clean();
}

// --- display-only (I6 = A) --------------------------------------------------

#[test]
fn doctor_never_asks_the_endpoint() {
    let rig = Rig::with_token("doctor", ONE_PROFILE);
    let server = Server::ok(&counts_body());
    let _ = rig.run(&server.url, &["doctor"]);
    assert_eq!(server.count(), 0, "observe readers never fetch");
}

#[test]
fn the_watchdog_neither_fetches_nor_feeds_copilot() {
    let daemon = product_part("src/watchdog_daemon.rs").to_lowercase();
    assert!(
        !daemon.contains("copilot"),
        "display-only: the watchdog carries no Copilot path"
    );
    let quota = product_part("src/quota.rs");
    assert!(
        !quota.contains("api.github.com"),
        "quota.rs must not name the vendor URL"
    );
    let observe = quota
        .split_once("pub(crate) fn observe(")
        .expect("observe")
        .1
        .split_once("pub fn run(")
        .expect("run after observe")
        .0;
    assert!(
        !observe.contains("go_usage::refresh") && !observe.contains("copilot_usage"),
        "the observe family never fetches"
    );
}

// --- inventory, fuzz, docs --------------------------------------------------

#[test]
fn no_new_agent_and_no_foreign_auth_store() {
    let mut agents = 0;
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("source");
                agents += text.matches("Agent::new_with_config").count();
                for store in ["hosts.yml", "auth.json", "github-copilot/apps.json"] {
                    assert!(
                        !text.contains(store),
                        "{} names another tool's auth store {store}",
                        path.display()
                    );
                }
            }
        }
    }
    assert_eq!(agents, 3, "no new ureq Agent construction site");
}

#[test]
fn the_hermetic_baseline_pins_the_copilot_seam_to_a_refused_loopback() {
    let cli = product_text("tests/it/cli.rs");
    let line = cli
        .lines()
        .find(|line| line.contains(SEAM))
        .unwrap_or_else(|| panic!("cli::isolated must pin {SEAM}"));
    assert!(
        line.contains("http://127.0.0.1:9/"),
        "refused loopback, never api.github.com: {line}"
    );
    let agents = product_text("AGENTS.md");
    assert!(agents.contains(SEAM), "AGENTS.md Environment doors row");
}

#[test]
fn the_parser_has_a_fuzz_target_with_tracked_seeds() {
    let manifest = product_text("fuzz/Cargo.toml");
    assert!(
        manifest.contains("name = \"quota_copilot_user\""),
        "fuzz [[bin]]"
    );
    let target = product_text("fuzz/fuzz_targets/quota_copilot_user.rs");
    assert!(target.contains("copilot"), "the target drives the parser");
    let readme = product_text("fuzz/README.md");
    assert!(readme.contains("`quota_copilot_user`"), "README row");
    let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/quota_copilot_user");
    let count = std::fs::read_dir(&seeds)
        .expect("seed dir")
        .flatten()
        .filter(|entry| entry.path().is_file())
        .count();
    assert!(count >= 3, "at least three seeds, got {count}");
}

#[test]
fn the_config_doc_says_how_to_make_the_file_without_ae() {
    let doc = product_text("docs/getting-started/config.md");
    assert!(doc.contains(SETTING), "config row");
    assert!(doc.contains("GitHub Copilot quota"), "section heading");
    assert!(doc.contains("gh auth token --user"), "token recipe");
    assert!(doc.contains("chmod 600"), "custody");
    assert!(
        doc.contains("copilot_internal/user"),
        "the endpoint is named as internal"
    );
}
