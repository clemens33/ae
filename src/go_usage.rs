//! The `OpenCode` Go usage door: the key file, the shared cache, and the one
//! HTTPS GET to `opencode.ai`.
//!
//! OPT-IN: nothing here runs unless the GLOBAL config names a key file with
//! `[workspace] opencode_go_key_file`. A project `.ae/config` never votes, so a
//! checkout cannot point ae at some other private file and have it sent away
//! as a Bearer token.
//!
//! ONE FETCH FOR THE FLEET. Every session watchdog and every `ae quota` share
//! one cache under the state root. A fetch is due only when the cache holds no
//! attempt for this key file in the last [`TTL_SECS`], and only the process
//! that takes the zero-wait lock makes it; every attempt is recorded, a failed
//! one too, so a refusing endpoint is asked at most once per window as well.
//!
//! THE KEY never leaves [`GoKey`]: it has a redacting `Debug`, no `Display`,
//! and becomes part of a string only in the one request header. [`Failure`]
//! is `Copy`, so it cannot carry text a formatter could print a key out of.
//! The cache records the key file's IDENTITY (path, device, inode, length,
//! mtime), never its content, so a rotated key makes the next refresh due.
//!
//! The endpoint is undocumented: its answer is parsed by the pure, fuzzed
//! [`crate::quota::go`], and anything it was not measured against is a note,
//! never a number.

use std::fmt;
use std::io::{self, Read as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::quota::go::{self, Cached, KeyIdentity, Sample};

/// The production endpoint.
const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

/// How long one attempt answers for the whole fleet.
pub const TTL_SECS: i64 = 300;

/// The global `[workspace]` key that opts in.
pub const KEY_SETTING: &str = "opencode_go_key_file";

/// TCP+TLS handshake bound.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(4);
/// Response-head bound after the request is sent.
const TIMEOUT_RECV_RESPONSE: Duration = Duration::from_secs(4);
/// Whole-call ceiling: agy's quota deadline.
const TIMEOUT_GLOBAL: Duration = Duration::from_secs(8);

/// The most answer this door holds, whatever `Content-Length` claims.
pub const MAX_BODY: usize = 64 * 1024;
/// The most cache record a reader holds.
const MAX_CACHE: u64 = 256 * 1024;
/// The longest key accepted, after trimming.
const MAX_KEY: usize = 4096;

/// Samples are kept this long for pace…
const SAMPLE_HORIZON_SECS: i64 = 12 * 3600;
/// …this far apart, except the newest.
const SAMPLE_SPACING_SECS: i64 = 15 * 60;

/// The cache directory under the state root, and its two files.
const DIR: &str = "quota";
const CACHE: &str = "opencode-go.json";
const LOCK: &str = ".opencode-go.lock";

/// The key, held for exactly one request.
pub struct GoKey(String);

impl fmt::Debug for GoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GoKey(<redacted>)")
    }
}

impl GoKey {
    /// The one place the key becomes part of a string: the request header.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

/// Why a refresh did not land a usable answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// HTTP 401: the key was rejected.
    Unauthorized,
    /// HTTP 403: the account has no Go subscription.
    NotEntitled,
    /// HTTP 429.
    Throttled,
    /// Any other non-2xx status.
    Status(u16),
    /// A configured timeout fired.
    Timeout,
    /// A 3xx, which is never followed.
    Redirected,
    /// Connect, DNS, TLS or socket failure.
    Transport,
    /// The answer passed [`MAX_BODY`].
    TooLarge,
    /// A 2xx whose answer has no readable window.
    Malformed,
    /// The key file is readable by group or others.
    KeyInsecure,
    /// The key path names something other than a regular file.
    KeyNotRegular,
    /// The key file is absent or could not be read.
    KeyUnreadable,
    /// The key file holds nothing but whitespace.
    KeyEmpty,
    /// The key holds a byte outside printable ASCII.
    KeyNotPrintable,
    /// The key is longer than [`MAX_KEY`].
    KeyTooLong,
}

impl Failure {
    /// The outcome slug the cache records, or `None` for a key failure: no
    /// request was made, so there is no attempt to record.
    #[must_use]
    pub fn slug(self) -> Option<String> {
        Some(
            match self {
                Self::Unauthorized => "unauthorized",
                Self::NotEntitled => "not-entitled",
                Self::Throttled => "throttled",
                Self::Status(code) => return Some(format!("status-{code:03}")),
                Self::Timeout => "timeout",
                Self::Redirected => "redirected",
                Self::Transport => "transport",
                Self::TooLarge => "too-large",
                Self::Malformed => "malformed",
                Self::KeyInsecure
                | Self::KeyNotRegular
                | Self::KeyUnreadable
                | Self::KeyEmpty
                | Self::KeyNotPrintable
                | Self::KeyTooLong => return None,
            }
            .to_owned(),
        )
    }

    /// What the operator is told about a key failure at `path`, or `None` for
    /// a request failure, which the cache's outcome speaks for.
    #[must_use]
    pub fn key_note(self, path: &Path, mode: Option<u32>) -> Option<String> {
        let path = path.display();
        Some(match self {
            Self::KeyInsecure => match mode {
                Some(mode) => format!(
                    "opencode go: key file {path} is readable by others (mode {mode:04o}); chmod 600 it"
                ),
                None => format!("opencode go: key file {path} is readable by others; chmod 600 it"),
            },
            Self::KeyNotRegular => format!("opencode go: key file {path} is not a regular file"),
            Self::KeyUnreadable => format!("opencode go: key file {path} could not be read"),
            Self::KeyEmpty => format!("opencode go: key file {path} is empty"),
            Self::KeyNotPrintable => {
                format!("opencode go: key file {path} holds a byte that is not printable ASCII")
            }
            Self::KeyTooLong => {
                format!("opencode go: key file {path} is longer than {MAX_KEY} bytes")
            }
            _ => return None,
        })
    }
}

/// What one refresh did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refreshed {
    /// Whether the fetch ran.
    pub fetched: bool,
    /// Why it failed, or why no fetch could be made; `None` when it landed or
    /// was not due.
    pub outcome: Option<Failure>,
}

/// The one fetch decision a watchdog pass makes: quota on, the session
/// quota-aware, a Go-scoped profile configured, and no fetch in flight.
#[must_use]
#[allow(
    clippy::fn_params_excessive_bools,
    reason = "the frozen spec pins these four facts as one table, each a bool the caller already holds"
)]
pub const fn watchdog_due(
    quota_on: bool,
    aware: bool,
    go_scope_present: bool,
    flying: bool,
) -> bool {
    quota_on && aware && go_scope_present && !flying
}

/// A fetch in flight. Arming sets the flag; dropping clears it, a panic's
/// unwind included, so a failed fetch cannot leave the watchdog believing one
/// is still out.
pub struct Flying<'a>(&'a AtomicBool);

impl<'a> Flying<'a> {
    /// Set `flag` until the returned guard is dropped.
    #[must_use]
    pub fn arm(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for Flying<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The key path the global config names, expanded, or why it names none ae
/// will use. `Ok(None)` is no opt-in at all.
///
/// # Errors
///
/// The config could not be read, or the value is not `~/…`, `$HOME/…` or
/// absolute.
pub fn configured_key(
    global: Option<&Path>,
    home: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let Some(global) = global else {
        return Ok(None);
    };
    let Some(raw) = crate::config::read_global_workspace_key(global, KEY_SETTING)? else {
        return Ok(None);
    };
    let value = raw.trim();
    let under_home = |rest: &str| {
        home.map(|home| home.join(rest))
            .ok_or_else(|| format!("{KEY_SETTING} names $HOME, which is not set"))
    };
    let path = if let Some(rest) = value.strip_prefix("~/") {
        under_home(rest)?
    } else if let Some(rest) = value
        .strip_prefix("$HOME/")
        .or_else(|| value.strip_prefix("${HOME}/"))
    {
        under_home(rest)?
    } else if value.starts_with('/') {
        PathBuf::from(value)
    } else {
        return Err(format!(
            "{KEY_SETTING} must be ~/…, $HOME/… or an absolute path"
        ));
    };
    Ok(Some(path))
}

/// What the cache holds for the key at `key_path`, read without the network
/// and without reading the key itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// The key file fails custody; the mode is known when it was readable.
    Custody(Failure, Option<u32>),
    /// No usable record for this key file: none yet, another key's, or damaged.
    Missing,
    /// The record written for this key file.
    Cached(Cached),
}

/// Read the cache for `key_path` under `root`.
#[must_use]
pub fn read(root: &Path, key_path: &Path) -> Read {
    let identity = match key_identity(key_path) {
        Ok(identity) => identity,
        Err((failure, mode)) => return Read::Custody(failure, mode),
    };
    match read_cache(root) {
        Some(cached) if cached.key == identity => Read::Cached(cached),
        _ => Read::Missing,
    }
}

/// Refresh the shared cache for `key_path` under `root` when due, over the
/// production endpoint.
#[must_use]
pub fn refresh(root: &Path, key_path: &Path, now: i64) -> Refreshed {
    refresh_with(root, key_path, now, fetch_network)
}

/// [`refresh`] with the fetch handed in.
///
/// Order: the key file's custody from its metadata, the cache's own verdict,
/// the zero-wait lock and the verdict again under it, the key's bytes, and
/// only then `fetch`. Every attempt is recorded before this returns.
pub fn refresh_with(
    root: &Path,
    key_path: &Path,
    now: i64,
    fetch: impl FnOnce(&GoKey) -> Result<Vec<u8>, Failure>,
) -> Refreshed {
    let skipped = |outcome| Refreshed {
        fetched: false,
        outcome,
    };
    let identity = match key_identity(key_path) {
        Ok(identity) => identity,
        Err((failure, _)) => return skipped(Some(failure)),
    };
    let held = read_cache(root).filter(|cached| cached.key == identity);
    if !due(held.as_ref(), now) {
        return skipped(None);
    }
    let dir = root.join(DIR);
    if ensure_dir(&dir).is_err() {
        return skipped(None);
    }
    let Ok(_lock) = crate::store::lock(&dir.join(LOCK), Duration::ZERO) else {
        return skipped(None);
    };
    let held = read_cache(root).filter(|cached| cached.key == identity);
    if !due(held.as_ref(), now) {
        return skipped(None);
    }
    let key = match read_key(key_path) {
        Ok(key) => key,
        Err(failure) => return skipped(Some(failure)),
    };
    let answer = fetch(&key).and_then(|body| {
        if body.len() > MAX_BODY {
            return Err(Failure::TooLarge);
        }
        if !go::parse(&body, now, now).usable() {
            return Err(Failure::Malformed);
        }
        String::from_utf8(body).map_err(|_| Failure::Malformed)
    });
    drop(key);
    let mut samples = held.map(|cached| cached.samples).unwrap_or_default();
    let (outcome, slug) = match answer {
        Ok(body) => {
            samples.push(Sample { at: now, body });
            (None, "ok".to_owned())
        }
        Err(failure) => (
            Some(failure),
            failure.slug().unwrap_or_else(|| "malformed".to_owned()),
        ),
    };
    let record = Cached {
        key: identity,
        attempt_at: now,
        outcome: slug,
        samples: kept(samples, now),
    };
    let _ = crate::store::publish_private_in(&dir, CACHE, record.render().as_bytes());
    Refreshed {
        fetched: true,
        outcome,
    }
}

/// Whether the attempt `held` records is outside this fleet's window.
fn due(held: Option<&Cached>, now: i64) -> bool {
    held.is_none_or(|cached| !(0..TTL_SECS).contains(&now.saturating_sub(cached.attempt_at)))
}

/// The samples pace reads: within the horizon, spaced, newest always kept,
/// bounded, oldest first.
fn kept(mut samples: Vec<Sample>, now: i64) -> Vec<Sample> {
    samples
        .retain(|sample| sample.at <= now && now.saturating_sub(sample.at) <= SAMPLE_HORIZON_SECS);
    samples.sort_by_key(|sample| sample.at);
    let mut kept: Vec<Sample> = Vec::new();
    for sample in samples.into_iter().rev() {
        let spaced = kept
            .last()
            .is_none_or(|newer| newer.at.saturating_sub(sample.at) >= SAMPLE_SPACING_SECS);
        if spaced && kept.len() < go::SAMPLES_MAX {
            kept.push(sample);
        }
    }
    kept.reverse();
    kept
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Err(why) if why.kind() != io::ErrorKind::AlreadyExists => Err(why),
        _ => Ok(()),
    }
}

fn read_cache(root: &Path) -> Option<Cached> {
    match crate::store::read_capped_in(&root.join(DIR), CACHE, MAX_CACHE) {
        Ok(crate::store::SourceRead::Ready(bytes)) => go::parse_cache(&bytes),
        _ => None,
    }
}

/// The key file's custody and identity, from its metadata alone.
fn key_identity(path: &Path) -> Result<KeyIdentity, (Failure, Option<u32>)> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: classifies the opted-in Go key file, following a link as telegram's token file does, before anything reads it — see clippy.toml"
    )]
    let probed = std::fs::metadata(path);
    let metadata = match probed {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Err((Failure::KeyNotRegular, None)),
        Err(_) => return Err((Failure::KeyUnreadable, None)),
    };
    custody(path, &metadata)
}

fn custody(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<KeyIdentity, (Failure, Option<u32>)> {
    let mode = metadata.mode() & 0o7777;
    if mode & 0o077 != 0 {
        return Err((Failure::KeyInsecure, Some(mode)));
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Ok(KeyIdentity {
        path: canonical.to_string_lossy().into_owned(),
        dev: metadata.dev(),
        ino: metadata.ino(),
        len: metadata.len(),
        mtime_ns: metadata
            .mtime()
            .saturating_mul(1_000_000_000)
            .saturating_add(metadata.mtime_nsec()),
    })
}

/// The key's bytes, custody checked again on the descriptor that was read.
fn read_key(path: &Path) -> Result<GoKey, Failure> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the opted-in Go key file itself, after classification — this module's ONE open of it; see clippy.toml"
    )]
    let opened = std::fs::File::open(path);
    let file = opened.map_err(|_| Failure::KeyUnreadable)?;
    let metadata = file.metadata().map_err(|_| Failure::KeyUnreadable)?;
    if !metadata.is_file() {
        return Err(Failure::KeyNotRegular);
    }
    custody(path, &metadata).map_err(|(failure, _)| failure)?;
    let mut raw = Vec::new();
    file.take(MAX_KEY as u64 * 2 + 2)
        .read_to_end(&mut raw)
        .map_err(|_| Failure::KeyUnreadable)?;
    let trimmed = raw.trim_ascii();
    if trimmed.is_empty() {
        return Err(Failure::KeyEmpty);
    }
    if trimmed.len() > MAX_KEY {
        return Err(Failure::KeyTooLong);
    }
    if !trimmed.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
        return Err(Failure::KeyNotPrintable);
    }
    String::from_utf8(trimmed.to_vec())
        .map(GoKey)
        .map_err(|_| Failure::KeyNotPrintable)
}

// ─── the locked client ───────────────────────────────────────────────────

/// Where the GET goes: the production endpoint, or — in the CHECKOUT shape
/// only — `AE_TEST_GO_USAGE_URL`, so the hermetic suite can point it at a
/// refused loopback and never reach `opencode.ai`.
fn endpoint() -> String {
    if crate::shape::current().honours_environment() {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: AE_TEST_GO_USAGE_URL keeps the hermetic suite off opencode.ai — CHECKOUT only"
        )]
        let raw = std::env::var_os("AE_TEST_GO_USAGE_URL");
        if let Some(url) = raw
            .and_then(|raw| raw.into_string().ok())
            .filter(|url| !url.is_empty())
        {
            return url;
        }
    }
    USAGE_URL.to_owned()
}

/// Whether `url` is a plaintext loopback, the one non-TLS endpoint allowed.
fn loopback(url: &str) -> bool {
    [
        "http://127.0.0.1:",
        "http://127.0.0.1/",
        "http://[::1]:",
        "http://localhost:",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}

/// **The THIRD `ureq::Agent` construction site in this crate**, with
/// telegram's lock: no proxy, no redirect, bounded timeouts, the pinned ring
/// provider, and TLS required for everything but a loopback test endpoint.
fn agent(https_only: bool) -> ureq::Agent {
    let crypto = Arc::new(rustls::crypto::ring::default_provider());
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(crypto)
        .build();
    let config = ureq::Agent::config_builder()
        .https_only(https_only)
        .proxy(None)
        .max_redirects(0)
        .timeout_connect(Some(TIMEOUT_CONNECT))
        .timeout_recv_response(Some(TIMEOUT_RECV_RESPONSE))
        .timeout_global(Some(TIMEOUT_GLOBAL))
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

fn fetch_network(key: &GoKey) -> Result<Vec<u8>, Failure> {
    get(&endpoint(), key)
}

/// GET `url` with the key as a Bearer, holding at most [`MAX_BODY`] bytes.
fn get(url: &str, key: &GoKey) -> Result<Vec<u8>, Failure> {
    let sent = agent(!loopback(url))
        .get(url)
        .header("Authorization", key.bearer())
        .header("Accept", "application/json")
        .call();
    let mut response = sent.map_err(classify)?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| Failure::Transport)?;
    if body.len() > MAX_BODY {
        return Err(Failure::TooLarge);
    }
    match status {
        200..=299 => Ok(body),
        300..=399 => Err(Failure::Redirected),
        other => Err(status_failure(other)),
    }
}

const fn status_failure(code: u16) -> Failure {
    match code {
        401 => Failure::Unauthorized,
        403 => Failure::NotEntitled,
        429 => Failure::Throttled,
        other => Failure::Status(other),
    }
}

/// Map a `ureq::Error` to a class. By value: the error dies here, whatever
/// text it carries.
#[allow(
    clippy::needless_pass_by_value,
    reason = "BY VALUE ON PURPOSE: the error dies here, so no caller can format it"
)]
fn classify(error: ureq::Error) -> Failure {
    match error {
        ureq::Error::StatusCode(code) => status_failure(code),
        ureq::Error::Timeout(_) => Failure::Timeout,
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => Failure::Redirected,
        _ => Failure::Transport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;

    /// One loopback HTTP exchange: answers `response` and hands back the
    /// request head it read.
    fn serve(response: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback");
        let url = format!(
            "http://{}/zen/go/v1/usage",
            listener.local_addr().expect("addr")
        );
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("timeout");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).expect("line") == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut stream = stream;
            // The client may hang up mid-answer (the cap test); that is its
            // right, not the server's failure.
            let _ = stream.write_all(&response);
            head
        });
        (url, handle)
    }

    fn answer(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn the_real_agent_sends_the_bearer_and_accept_headers() {
        let body = br#"{"usage":{}}"#;
        let (url, server) = serve(answer("200 OK", "", body));
        let got = get(&url, &GoKey("synthetic-key".to_owned()));
        assert_eq!(got.as_deref(), Ok(&body[..]));
        let head = server.join().expect("server").to_ascii_lowercase();
        assert!(head.starts_with("get /zen/go/v1/usage "), "{head}");
        assert!(
            head.contains("authorization: bearer synthetic-key\r\n"),
            "{head}"
        );
        assert!(head.contains("accept: application/json\r\n"), "{head}");
    }

    #[test]
    fn a_redirect_is_never_followed() {
        let (url, server) = serve(answer(
            "302 Found",
            "Location: http://127.0.0.1:9/elsewhere\r\n",
            b"",
        ));
        assert_eq!(get(&url, &GoKey("k".to_owned())), Err(Failure::Redirected));
        server.join().expect("server");
    }

    #[test]
    fn a_body_past_the_cap_is_too_large_and_never_held_whole() {
        let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        response.extend(std::iter::repeat_n(b'x', MAX_BODY + 1));
        let (url, server) = serve(response);
        assert_eq!(get(&url, &GoKey("k".to_owned())), Err(Failure::TooLarge));
        server.join().expect("server");
    }

    #[test]
    fn statuses_map_to_their_classes() {
        for (status, want) in [
            ("401 Unauthorized", Failure::Unauthorized),
            ("403 Forbidden", Failure::NotEntitled),
            ("429 Too Many Requests", Failure::Throttled),
            ("502 Bad Gateway", Failure::Status(502)),
        ] {
            let (url, server) = serve(answer(status, "", b"{}"));
            assert_eq!(get(&url, &GoKey("k".to_owned())), Err(want), "{status}");
            server.join().expect("server");
        }
        assert_eq!(Failure::Status(502).slug().as_deref(), Some("status-502"));
        assert!(go::is_outcome(&Failure::Status(502).slug().expect("slug")));
    }

    #[test]
    fn only_a_plaintext_loopback_is_exempt_from_tls() {
        assert!(loopback("http://127.0.0.1:9/"));
        assert!(!loopback("http://127.0.0.1.evil.example/"));
        assert!(!loopback(USAGE_URL));
        assert!(!loopback("http://opencode.ai/zen/go/v1/usage"));
    }

    #[test]
    fn kept_samples_are_spaced_bounded_and_oldest_first() {
        let now = 100_000;
        let samples = (0..200)
            .map(|step| Sample {
                at: now - step * 60,
                body: String::new(),
            })
            .collect();
        let kept = kept(samples, now);
        assert_eq!(kept.last().map(|sample| sample.at), Some(now));
        assert!(kept.len() <= go::SAMPLES_MAX);
        assert!(
            kept.windows(2)
                .all(|pair| pair[1].at - pair[0].at >= SAMPLE_SPACING_SECS)
        );
    }
}
