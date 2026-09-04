//! The Telegram bridge: the network boundary, the secret, and the OUTBOUND
//! half — `say`'s chat events, forwarded to `sendMessage` over TLS.
//!
//! The INBOUND half lives in this module's own submodules rather than beside
//! it, so that everything hardened here — the one locked [`Api`], the one
//! [`open_regular`], the one [`durable_write`] — is reachable from it WITHOUT
//! being widened to the crate: [`inbound`] runs one `getUpdates` cycle,
//! [`routing`] decides where a message goes, and [`bridge`] is the daemon that
//! turns the two halves into two loops.
//!
//! This is ae's only surface that talks to the public internet, and everything
//! unusual about it follows from two facts:
//!
//! 1. **The bot token is in the URL PATH** (`/bot<TOKEN>/sendMessage`), so every
//!    value that could carry a URL — a `ureq::Error`, a request builder's debug
//!    output, a log line — is a token leak waiting for a `{}`. The countermeasure
//!    is structural rather than careful: [`SendFailure`] carries no string at
//!    all, so there is no shape of this module's error type that a formatter
//!    could print a token out of. [`Token`] has a redacting [`fmt::Debug`] and
//!    no [`fmt::Display`], so the secret itself cannot reach a format string by
//!    accident either.
//! 2. **The cursor is a byte offset, so the ledger must be append-only.** It
//!    is: `crate::state`'s locked `OpenOptions::append(true)` is the only
//!    production writer, and replacement always changes the inode. The full
//!    proof, and the trigger that would invalidate it, are at
//!    [`Outbound::start`].
//! 3. **Telegram's answer is untrusted network input.** It is parsed with
//!    [`crate::json`], whose `MAX_DEPTH` already bounds recursion, and it is
//!    read through a hard byte cap that does not consult `Content-Length` —
//!    a declared length is the remote's claim, not a bound.
//!
//! ## Delivery semantics: at-least-once, with one honest crash window
//!
//! `say` appends a `chat` event to `events.jsonl`; this module forwards each one
//! and records how far it has got in a durable cursor ([`Cursor`]). The cursor
//! advances ONLY past an event Telegram ACCEPTED — HTTP 2xx *and* a parsed root
//! `ok == true` — and the advance is persisted with an fsync of both the temp
//! file and its directory before the next event is considered.
//!
//! So a normal restart re-sends nothing, and the only duplicate possible is the
//! single event that was accepted in the instant before its checkpoint reached
//! the disk. That bound holds under REPEATED checkpoint failure too, because the
//! in-memory scan advances only with the durable cursor: a disk that cannot be
//! written degrades to throttled re-delivery of that one owed event (the
//! `failures` streak feeds [`backoff_delay`], capping re-attempts at a minute),
//! never to an unbounded replay of everything since the last good checkpoint. That window is real and is not closed here: closing it needs a
//! two-phase commit with a party that does not offer one. It is bounded to ONE
//! event, and it duplicates rather than loses.
//!
//! Advancing on 2xx alone would be the silent-loss bug: Telegram answers 200
//! with `{"ok":false,"description":"chat not found"}`, and a bridge that reads
//! only the status code marks that event delivered forever.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::json::{self, Value};

// The inbound half, in submodules of THIS one rather than beside it.
pub mod bridge;
pub mod inbound;
pub mod routing;

// ─── the network boundary ────────────────────────────────────────────────

/// The production API root.
const TELEGRAM_API: &str = "https://api.telegram.org";

/// How long to wait for the TCP+TLS handshake.
const TIMEOUT_CONNECT: Duration = Duration::from_secs(10);
/// How long to wait for the response head after the request is sent.
const TIMEOUT_RECV_RESPONSE: Duration = Duration::from_secs(20);
/// The whole-call ceiling.
const TIMEOUT_GLOBAL: Duration = Duration::from_secs(30);

/// The most response body this module will hold in memory, in bytes.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// The most a `getUpdates` long poll may ask Telegram to hold the connection.
const LONG_POLL_MAX: Duration = Duration::from_secs(10);

/// THE COMPILER holds that relationship, not a comment and not a test: raising
/// the long poll past the agent's receive ceiling — or lowering the ceiling
/// under it — fails the build rather than turning every quiet poll into a
/// timeout nobody would connect to this line.
const _: () = assert!(
    LONG_POLL_MAX.as_secs() < TIMEOUT_RECV_RESPONSE.as_secs(),
    "the long poll must be answerable within the agent's receive timeout"
);

/// Telegram's own `text` limit is 4096 UTF-8 characters; a longer message is
/// rejected with a 400.
const MAX_TEXT_CHARS: usize = 3900;

/// Where a request can be sent.
#[derive(Debug, Clone)]
enum Egress {
    /// `api.telegram.org` over TLS.
    Production,
    /// TEST ONLY: a plaintext loopback server, e.g. `http://127.0.0.1:53312`.
    #[cfg(test)]
    Loopback(String),
}

impl Egress {
    /// The scheme+authority every request is built on.
    fn base(&self) -> &str {
        match self {
            Self::Production => TELEGRAM_API,
            #[cfg(test)]
            Self::Loopback(base) => base,
        }
    }

    /// Whether the agent refuses plaintext.
    fn https_only(&self) -> bool {
        match self {
            Self::Production => true,
            // A loopback socket in a test process is not a downgrade path; it
            // is the only way to exercise the real client against a real
            // server without shipping a TLS server and a private key.
            #[cfg(test)]
            Self::Loopback(_) => false,
        }
    }
}

// ─── the secret ──────────────────────────────────────────────────────────

/// A bot token.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wrap a token value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw token.
    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// The bot token plus the chat it posts to.
#[derive(Debug, Clone)]
pub struct Credentials {
    token: Token,
    chat_id: String,
}

impl Credentials {
    /// Pair a token with a chat id.
    #[must_use]
    pub fn new(token: Token, chat_id: impl Into<String>) -> Self {
        Self {
            token,
            chat_id: chat_id.into(),
        }
    }

    /// The configured control chat, verbatim.
    #[must_use]
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// The `chat_id` as JSON: a number when it is one, a string otherwise
    /// (`@channelusername` is a legal chat id and is not a number).
    fn chat_id_value(&self) -> Value {
        self.chat_id
            .parse::<i64>()
            .map_or_else(|_| Value::str(self.chat_id.clone()), Value::Num)
    }
}

/// Why credentials could not be loaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum CredentialsError {
    /// The config file could not be read.
    Config(PathBuf),
    /// `[telegram] token_file` is missing or empty.
    NoTokenFile,
    /// `[telegram] chat_id` is missing or empty.
    NoChatId,
    /// The token file named by `token_file` could not be read.
    TokenUnreadable(PathBuf),
    /// The token file was readable but held nothing.
    TokenEmpty,
    /// The config path is not a regular file — a FIFO, a device, a directory.
    ConfigNotRegular(PathBuf),
    /// The token path is not a regular file.
    TokenNotRegular(PathBuf),
    /// The token file is readable by group or other.
    TokenInsecurePermissions(PathBuf, u32),
}

impl fmt::Display for CredentialsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(path) => write!(f, "telegram: unreadable config {}", path.display()),
            Self::NoTokenFile => f.write_str("telegram: [telegram] token_file is not set"),
            Self::NoChatId => f.write_str("telegram: [telegram] chat_id is not set"),
            Self::TokenUnreadable(path) => {
                write!(f, "telegram: unreadable token file {}", path.display())
            }
            Self::TokenEmpty => f.write_str("telegram: the token file is empty"),
            Self::ConfigNotRegular(path) => write!(
                f,
                "telegram: config {} is not a regular file",
                path.display()
            ),
            Self::TokenNotRegular(path) => write!(
                f,
                "telegram: token file {} is not a regular file",
                path.display()
            ),
            Self::TokenInsecurePermissions(path, mode) => write!(
                f,
                "telegram: token file {} is readable by others (mode {mode:04o}); chmod 600 it",
                path.display()
            ),
        }
    }
}

/// **THE ONLY `File::open` IN THIS MODULE.**
fn open_regular(path: &Path) -> Result<(fs::File, fs::Metadata), NotRegular> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: classifying a path before opening it — see clippy.toml"
    )]
    let probed = fs::metadata(path);
    match probed {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(NotRegular::Node),
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Err(NotRegular::Absent),
        Err(why) => return Err(NotRegular::Unreadable(why)),
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the file itself, after classification — the module's ONE open; \
                  see clippy.toml"
    )]
    let opened = fs::File::open(path);
    let file = opened.map_err(|why| match why.kind() {
        io::ErrorKind::NotFound => NotRegular::Absent,
        _ => NotRegular::Unreadable(why),
    })?;
    let metadata = file.metadata().map_err(NotRegular::Unreadable)?;
    if !metadata.is_file() {
        // The name was re-pointed between the classification and the open.
        return Err(NotRegular::Node);
    }
    Ok((file, metadata))
}

/// Read a regular file whole.
fn read_regular_file(path: &Path) -> Result<(String, u32), NotRegular> {
    let (mut file, metadata) = open_regular(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(NotRegular::Unreadable)?;
    Ok((text, metadata.mode() & 0o7777))
}

/// Why [`open_regular`] gave up.
enum NotRegular {
    /// Nothing at that path.
    Absent,
    /// Something is there, and it is not a regular file.
    Node,
    /// Present, and it could not be read.
    Unreadable(io::Error),
}

impl std::error::Error for CredentialsError {}

/// Read `[telegram] token_file` and `[telegram] chat_id` from an ae config, and
/// the token out of the file the first names.
///
/// # Errors
///
/// [`CredentialsError`] for a config that cannot be read, either key missing,
/// or a token file that is unreadable or empty. Absence is never treated as an
/// empty value: a bridge that starts with no token would post nowhere and say
/// nothing about it.
pub fn load_credentials(config: &Path, home: &Path) -> Result<Credentials, CredentialsError> {
    load_settings(config, home).map(|settings| settings.credentials)
}

/// Everything the bridge reads out of an ae config: the outbound credentials,
/// and the inbound allow-list.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The bot token and the chat the outbound half posts to.
    pub credentials: Credentials,
    /// **Inbound exists ONLY with a non-empty allow-list.**
    pub allowed_user_ids: Vec<String>,
}

/// Read `[telegram]` whole: `token_file`, `chat_id` and `allowed_user_ids`.
///
/// # Errors
///
/// [`CredentialsError`], exactly as [`load_credentials`] — the allow-list has
/// no failure of its own, because ABSENT is a legal, meaningful value for it
/// (outbound-only bridge) rather than a missing requirement.
pub fn load_settings(config: &Path, home: &Path) -> Result<Settings, CredentialsError> {
    let text = match read_regular_file(config) {
        Ok((text, _)) => text,
        Err(NotRegular::Node) => return Err(CredentialsError::ConfigNotRegular(config.to_owned())),
        // Absent and unreadable are one refusal here: a config the operator
        // selected and this process cannot read is a failure either way.
        Err(NotRegular::Absent | NotRegular::Unreadable(_)) => {
            return Err(CredentialsError::Config(config.to_owned()));
        }
    };
    let mut token_file = None;
    let mut chat_id = None;
    let mut allowed = None;
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = section_header(line) {
            section = name;
            continue;
        }
        if section != "telegram" {
            continue;
        }
        let Some((key, value)) = setting(line) else {
            continue;
        };
        match key.as_str() {
            "token_file" => token_file = Some(value),
            "chat_id" => chat_id = Some(value),
            "allowed_user_ids" => allowed = Some(value),
            _ => {}
        }
    }

    let token_file = token_file
        .filter(|v| !v.is_empty())
        .ok_or(CredentialsError::NoTokenFile)?;
    let chat_id = chat_id
        .filter(|v| !v.is_empty())
        .ok_or(CredentialsError::NoChatId)?;
    let token_path = expand_home(&token_file, home);
    let (raw_token, mode) = match read_regular_file(&token_path) {
        Ok(read) => read,
        Err(NotRegular::Node) => {
            return Err(CredentialsError::TokenNotRegular(token_path.clone()));
        }
        Err(NotRegular::Absent | NotRegular::Unreadable(_)) => {
            return Err(CredentialsError::TokenUnreadable(token_path.clone()));
        }
    };
    // CUSTODY, checked on the descriptor that was actually read.
    if mode & 0o077 != 0 {
        return Err(CredentialsError::TokenInsecurePermissions(token_path, mode));
    }
    let token = raw_token.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ' || c == '\t');
    if token.is_empty() {
        return Err(CredentialsError::TokenEmpty);
    }
    Ok(Settings {
        credentials: Credentials::new(Token::new(token), chat_id),
        allowed_user_ids: allowed.as_deref().map(parse_id_list).unwrap_or_default(),
    })
}

/// Split an `allowed_user_ids` value the way the frozen bash does: on commas
/// AND spaces, discarding empties.
fn parse_id_list(value: &str) -> Vec<String> {
    value
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// `[name]` → `name`, for a line that is one.
fn section_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.trim().to_owned())
}

/// `key = value` → `(key, value)`, with the same tolerance the frozen bash
/// parser has: an optionally quoted value, and an unquoted one truncated at a
/// `#` comment.
fn setting(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    let rest = rest.trim();
    let value = if let Some(quoted) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        quoted.to_owned()
    } else {
        rest.split('#').next().unwrap_or("").trim().to_owned()
    };
    Some((key.to_owned(), value))
}

/// Expand a leading `~` against the supplied home.
fn expand_home(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        return home.to_owned();
    }
    value
        .strip_prefix("~/")
        .map_or_else(|| PathBuf::from(value), |rest| home.join(rest))
}

// ─── failures, redacted by construction ──────────────────────────────────

/// The class of an HTTP status, which is all of it that is ever presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    /// 3xx.
    Redirect,
    /// 4xx — Telegram refused the request itself.
    ClientError,
    /// 5xx — Telegram is having a bad day; worth retrying.
    ServerError,
    /// Anything else a server can put on a status line.
    Other,
}

impl StatusClass {
    /// Classify a status code.
    #[must_use]
    pub fn of(code: u16) -> Self {
        match code {
            300..=399 => Self::Redirect,
            400..=499 => Self::ClientError,
            500..=599 => Self::ServerError,
            _ => Self::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Redirect => "redirect",
            Self::ClientError => "client-error",
            Self::ServerError => "server-error",
            Self::Other => "unexpected-status",
        }
    }
}

/// Why one `sendMessage` did not land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendFailure {
    /// A non-2xx status.
    Status(StatusClass, u16),
    /// HTTP 2xx, and Telegram still said no: root `ok` was `false`, missing, or
    /// not a boolean.
    Rejected,
    /// The response was not UTF-8, not JSON, or not a JSON object.
    Malformed,
    /// The response body passed [`MAX_RESPONSE_BYTES`] while streaming.
    TooLarge,
    /// A configured timeout fired.
    Timeout,
    /// A 3xx.
    Redirected,
    /// Connect, DNS, TLS or socket failure — deliberately one class, because
    /// the distinctions live in strings this type refuses to carry.
    Transport,
}

/// THE COMPILER, not a convention, is what keeps a token out of [`SendFailure`].
const _: fn() = || {
    fn holds_no_owned_text<T: Copy>() {}
    holds_no_owned_text::<SendFailure>();
    // The endpoint-carrying form inherits the property rather than restating
    // it: a `Copy` struct cannot own a `String` either, and this is the type
    // the INBOUND half returns.
    holds_no_owned_text::<ApiFailure>();
};

/// Which Telegram method a failure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `sendMessage` — the outbound half.
    SendMessage,
    /// `getUpdates` — the inbound long-poll.
    GetUpdates,
    /// `setMyCommands` — the startup command-menu registration.
    SetMyCommands,
}

impl Method {
    /// The bare method name, which is all of the URL that is ever printed.
    const fn label(self) -> &'static str {
        match self {
            Self::SendMessage => "sendMessage",
            Self::GetUpdates => "getUpdates",
            Self::SetMyCommands => "setMyCommands",
        }
    }
}

/// One API call that did not land: which method, and which redacted class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiFailure {
    /// The method that failed.
    pub method: Method,
    /// Why, in the redacted classes [`SendFailure`] enumerates.
    pub kind: SendFailure,
}

impl ApiFailure {
    /// Pair a failure class with the endpoint that produced it.
    #[must_use]
    pub const fn at(method: Method, kind: SendFailure) -> Self {
        Self { method, kind }
    }
}

impl fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every arm is `{method} {endpoint}: {class}`.
        write!(f, "POST {}: ", self.method.label())?;
        match self.kind {
            SendFailure::Status(class, code) => write!(f, "{} ({code})", class.label()),
            SendFailure::Rejected => f.write_str("rejected by telegram (ok != true)"),
            SendFailure::Malformed => f.write_str("response was not the expected JSON"),
            SendFailure::TooLarge => {
                write!(f, "response body exceeded {MAX_RESPONSE_BYTES} bytes")
            }
            SendFailure::Timeout => f.write_str("timed out"),
            SendFailure::Redirected => f.write_str("redirected (refused)"),
            SendFailure::Transport => f.write_str("transport failure"),
        }
    }
}

impl std::error::Error for ApiFailure {}

impl fmt::Display for SendFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A bare `SendFailure` is the OUTBOUND half's, and it renders exactly
        // as it always has — byte for byte.
        ApiFailure::at(Method::SendMessage, *self).fmt(f)
    }
}

impl std::error::Error for SendFailure {}

impl SendFailure {
    /// Whether retrying the same event could plausibly succeed.
    #[must_use]
    pub fn is_transient(self) -> bool {
        !matches!(self, Self::Status(StatusClass::ClientError, _))
    }
}

// ─── the locked client ───────────────────────────────────────────────────

/// A Telegram API client bound to one bot token and one chat.
pub struct Api {
    agent: ureq::Agent,
    egress: Egress,
    credentials: Credentials,
}

impl fmt::Debug for Api {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither the agent (whose config holds nothing secret but whose debug
        // output is unbounded) nor the credentials.
        f.write_str("Api { .. }")
    }
}

impl Api {
    /// The production client: `api.telegram.org` over TLS.
    #[must_use]
    pub fn production(credentials: Credentials) -> Self {
        Self::with_egress(Egress::Production, credentials)
    }

    /// TEST ONLY: a client pointed at a loopback HTTP server.
    #[cfg(test)]
    fn loopback(base: impl Into<String>, credentials: Credentials) -> Self {
        Self::with_egress(Egress::Loopback(base.into()), credentials)
    }

    fn with_egress(egress: Egress, credentials: Credentials) -> Self {
        Self {
            agent: Self::agent(&egress),
            egress,
            credentials,
        }
    }

    /// **The one and only `ureq::Agent` construction site in this crate.**
    fn agent(egress: &Egress) -> ureq::Agent {
        let crypto = Arc::new(rustls::crypto::ring::default_provider());
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::Rustls)
            .unversioned_rustls_crypto_provider(crypto)
            .build();
        let config = ureq::Agent::config_builder()
            .https_only(egress.https_only())
            .proxy(None)
            .max_redirects(0)
            .timeout_connect(Some(TIMEOUT_CONNECT))
            .timeout_recv_response(Some(TIMEOUT_RECV_RESPONSE))
            .timeout_global(Some(TIMEOUT_GLOBAL))
            .tls_config(tls)
            .build();
        ureq::Agent::new_with_config(config)
    }

    /// The agent's resolved configuration, for the tests that assert the lock
    /// is actually on.
    #[cfg(test)]
    fn config(&self) -> &ureq::config::Config {
        self.agent.config()
    }

    /// POST one message to `sendMessage`.
    ///
    /// # Errors
    ///
    /// [`SendFailure`], which is redacted by construction. Note what counts as
    /// success: HTTP 2xx **and** a JSON root with `ok == true`. A 200 carrying
    /// `{"ok":false}` is [`SendFailure::Rejected`], because Telegram answers
    /// "chat not found" that way and treating it as delivered loses the message.
    pub fn send_message(&self, text: &str) -> Result<(), SendFailure> {
        // The one place the token becomes part of a string.
        let url = format!(
            "{}/bot{}/sendMessage",
            self.egress.base(),
            self.credentials.token.expose()
        );
        let body = Value::obj([
            ("chat_id", self.credentials.chat_id_value()),
            ("text", Value::str(truncate_chars(text, MAX_TEXT_CHARS))),
        ])
        .render();

        let sent = self
            .agent
            .post(&url)
            .content_type("application/json")
            .send(body.as_str());
        // `sent`'s error is dropped into `classify` and never named again: a
        // `ureq::Error` can quote the URI, and the URI is the token.
        let mut response = sent.map_err(classify)?;
        let status = response.status().as_u16();
        let bytes = read_bounded(response.body_mut().as_reader())?;
        if !(200..300).contains(&status) {
            // Unreachable while `http_status_as_error` is ureq's default `true`
            // — kept because that default is a setting, and a future change to
            // it must not silently turn a 500 into a parse attempt.
            return Err(SendFailure::Status(StatusClass::of(status), status));
        }
        accepted(&bytes)
    }

    /// Register the slash-command menu, so the chat's `/` list offers ae's
    /// grammar instead of requiring the operator to remember it.
    ///
    /// # Errors
    ///
    /// [`SendFailure`], redacted like every other failure here. Same acceptance
    /// rule as [`Api::send_message`]: 2xx **and** `ok:true`.
    pub fn set_my_commands(&self, commands: &[(&str, &str)]) -> Result<(), SendFailure> {
        let url = format!(
            "{}/bot{}/setMyCommands",
            self.egress.base(),
            self.credentials.token.expose()
        );
        let body = Value::obj([(
            "commands",
            Value::Arr(
                commands
                    .iter()
                    .map(|&(command, description)| {
                        Value::obj([
                            ("command", Value::str(command)),
                            ("description", Value::str(description)),
                        ])
                    })
                    .collect(),
            ),
        )])
        .render();
        let sent = self
            .agent
            .post(&url)
            .content_type("application/json")
            .send(body.as_str());
        let mut response = sent.map_err(classify)?;
        let status = response.status().as_u16();
        let bytes = read_bounded(response.body_mut().as_reader())?;
        if !(200..300).contains(&status) {
            return Err(SendFailure::Status(StatusClass::of(status), status));
        }
        accepted(&bytes)
    }

    /// Long-poll `getUpdates` for everything from `offset` onwards.
    ///
    /// # Errors
    ///
    /// [`ApiFailure`] — the same redacted classes the outbound half uses, named
    /// with this endpoint. A failure here means the caller must NOT advance the
    /// durable offset: see [`inbound`]'s module docs.
    pub fn get_updates(
        &self,
        offset: i64,
        limit: u32,
        wait: Duration,
    ) -> Result<Vec<Value>, ApiFailure> {
        self.updates(offset, limit, wait)
            .map_err(|kind| ApiFailure::at(Method::GetUpdates, kind))
    }

    /// [`Api::get_updates`] before the endpoint is attached to its failure.
    fn updates(&self, offset: i64, limit: u32, wait: Duration) -> Result<Vec<Value>, SendFailure> {
        // The one place the token becomes part of a string, for this endpoint.
        let url = format!(
            "{}/bot{}/getUpdates",
            self.egress.base(),
            self.credentials.token.expose()
        );
        let seconds = wait.min(LONG_POLL_MAX).as_secs();
        let body = Value::obj([
            ("offset", Value::Num(offset)),
            ("limit", Value::Num(i64::from(limit))),
            (
                "timeout",
                Value::Num(i64::try_from(seconds).unwrap_or(i64::MAX)),
            ),
            // ONLY `message`.
            ("allowed_updates", Value::Arr(vec![Value::str("message")])),
        ])
        .render();

        let sent = self
            .agent
            .post(&url)
            .content_type("application/json")
            .send(body.as_str());
        let mut response = sent.map_err(classify)?;
        let status = response.status().as_u16();
        let bytes = read_bounded(response.body_mut().as_reader())?;
        if !(200..300).contains(&status) {
            return Err(SendFailure::Status(StatusClass::of(status), status));
        }
        match envelope(&bytes)?.get("result") {
            Some(Value::Arr(updates)) => Ok(updates.clone()),
            // A 200 with `ok:true` and no array to go with it is not a quiet
            // empty poll — it is a response this reader does not understand,
            // and treating it as "no updates" would advance nothing while
            _ => Err(SendFailure::Malformed),
        }
    }
}

/// Map a `ureq::Error` to a redacted class.
#[allow(
    clippy::needless_pass_by_value,
    reason = "BY VALUE ON PURPOSE: this function's contract is that the error dies here. \
              A reference would leave the caller holding a value whose Display quotes the \
              request URI — which is the bot token."
)]
fn classify(error: ureq::Error) -> SendFailure {
    match error {
        ureq::Error::StatusCode(code) => SendFailure::Status(StatusClass::of(code), code),
        ureq::Error::Timeout(_) => SendFailure::Timeout,
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => SendFailure::Redirected,
        // Io, Tls, HostNotFound, ConnectionFailed, Protocol, Http, BadUri,
        // InvalidProxyUrl, BodyExceedsLimit and whatever a `#[non_exhaustive]`
        // enum adds next.
        _ => SendFailure::Transport,
    }
}

/// Read a response body with a hard ceiling, ignoring `Content-Length`.
fn read_bounded(reader: impl Read) -> Result<Vec<u8>, SendFailure> {
    let mut buffer = Vec::new();
    let read = reader
        .take(as_bytes_count(MAX_RESPONSE_BYTES) + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| SendFailure::Transport)?;
    if read > MAX_RESPONSE_BYTES {
        return Err(SendFailure::TooLarge);
    }
    Ok(buffer)
}

/// Did Telegram accept it?
fn accepted(bytes: &[u8]) -> Result<(), SendFailure> {
    envelope(bytes).map(|_| ())
}

/// [`accepted`], keeping the parsed body for a caller that needs its `result`.
fn envelope(bytes: &[u8]) -> Result<Value, SendFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| SendFailure::Malformed)?;
    let value = json::parse(text).map_err(|_| SendFailure::Malformed)?;
    match value.get("ok") {
        Some(Value::Bool(true)) => Ok(value),
        // `Some(Bool(false))`, `Some(anything else)` and `None` are one answer:
        // not accepted.
        _ => Err(SendFailure::Rejected),
    }
}

/// Truncate to a character count, on a character boundary, marking the cut.
fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…(truncated)")
}

// ─── the durable cursor ──────────────────────────────────────────────────

/// How far into an `events.jsonl` the bridge has been ACCEPTED.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// The inode of the log this offset belongs to.
    pub inode: u64,
    /// Bytes of that log already accepted by Telegram.
    pub offset: u64,
}

/// The on-disk form: a version tag and two numbers.
const CURSOR_TAG: &str = "ae-telegram-outbound-v1";

impl Cursor {
    /// Render the one line this cursor is stored as.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{CURSOR_TAG} {} {}\n", self.inode, self.offset)
    }

    /// Parse the one line a cursor is stored as.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut fields = text.split_whitespace();
        if fields.next()? != CURSOR_TAG {
            return None;
        }
        let inode = fields.next()?.parse().ok()?;
        let offset = fields.next()?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self { inode, offset })
    }
}

/// Why the cursor could not be read or written.
#[derive(Debug)]
#[non_exhaustive]
pub enum CursorError {
    /// The file exists but is not a cursor this version understands.
    Unrecognised,
    /// The cursor path is not a regular file.
    NotRegular(PathBuf),
    /// Reading it failed for a reason other than absence.
    Unreadable(io::Error),
    /// The write did not become durable.
    NotWritten(io::Error),
}

impl fmt::Display for CursorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrecognised => f.write_str("telegram cursor: unrecognised contents"),
            Self::NotRegular(path) => write!(
                f,
                "telegram cursor: {} is not a regular file",
                path.display()
            ),
            Self::Unreadable(source) => write!(f, "telegram cursor: unreadable: {source}"),
            Self::NotWritten(source) => write!(f, "telegram cursor: not written: {source}"),
        }
    }
}

impl std::error::Error for CursorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unrecognised | Self::NotRegular(_) => None,
            Self::Unreadable(source) | Self::NotWritten(source) => Some(source),
        }
    }
}

/// Read the cursor.
///
/// # Errors
///
/// [`CursorError::Unrecognised`] for a file that is not a cursor, and
/// [`CursorError::Unreadable`] for any read failure that is not "absent". A
/// damaged cursor is an error rather than a silent restart from zero.
pub fn load_cursor(path: &Path) -> Result<Option<Cursor>, CursorError> {
    // Through [`open_regular`], like every other read in this module: a FIFO
    // planted at the cursor's name would otherwise hang the bridge here, before
    // any of its timeouts or retries exist to help.
    let (text, _) = match read_regular_file(path) {
        Ok(read) => read,
        Err(NotRegular::Absent) => return Ok(None),
        Err(NotRegular::Node) => return Err(CursorError::NotRegular(path.to_owned())),
        Err(NotRegular::Unreadable(why)) => return Err(CursorError::Unreadable(why)),
    };
    Cursor::parse(&text)
        .map(Some)
        .ok_or(CursorError::Unrecognised)
}

/// Write the cursor DURABLY, through the bridge's one durable write.
///
/// # Errors
///
/// [`CursorError::NotWritten`] for any failure. The caller must treat a failed
/// checkpoint as an un-checkpointed delivery — the event was sent, and the
/// duplicate on restart is the designed cost.
pub fn store_cursor(path: &Path, cursor: &Cursor) -> Result<(), CursorError> {
    durable_write(path, &cursor.render()).map_err(CursorError::NotWritten)
}

/// The stem a checkpoint's temp file falls back to when the target path has no
/// usable file name of its own.
const CHECKPOINT_STEM: &str = "ae-telegram-checkpoint";

/// Write `contents` to `path` DURABLY: temp file, `fsync` the temp, rename,
/// `fsync` the directory.
///
/// # Errors
///
/// The underlying [`io::Error`]. A caller must treat a failed write as an
/// UN-checkpointed step: the work was done, and the duplicate on restart is the
/// designed cost.
pub(crate) fn durable_write(path: &Path, contents: &str) -> io::Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    let temp = directory.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .map_or(CHECKPOINT_STEM, |n| n.to_str().unwrap_or(CHECKPOINT_STEM)),
        std::process::id()
    ));
    let staged = (|| {
        // `create_new(true)` is `O_EXCL`, and `O_EXCL` NEVER follows an
        // existing node.
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = match options.open(&temp) {
            Ok(file) => file,
            Err(why) if why.kind() == io::ErrorKind::AlreadyExists => {
                // A leftover from a dead process with our pid — or a plant.
                fs::remove_file(&temp)?;
                options.open(&temp)?
            }
            Err(why) => return Err(why),
        };
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if let Err(why) = staged {
        let _ = fs::remove_file(&temp);
        return Err(why);
    }
    fs::OpenOptions::new()
        .read(true)
        .open(directory)
        .and_then(|handle| handle.sync_all())
}

// ─── the pump ────────────────────────────────────────────────────────────

/// The file name the cursor is stored under, inside a session's meta directory.
pub const CURSOR_FILE: &str = "telegram-outbound.cursor";

/// The event action `say` emits, and the only one this bridge forwards.
const CHAT_ACTION: &str = "chat";

/// The most bytes of log one pass will read.
const MAX_PASS_BYTES: u64 = 1024 * 1024;

/// Retry schedule after a failed delivery: doubling, and capped.
const BACKOFF_BASE: Duration = Duration::from_secs(1);
/// The ceiling on that doubling.
const BACKOFF_MAX: Duration = Duration::from_mins(1);

/// How long to wait before retrying, after `failures` consecutive failures.
///
/// ```
/// use ae::telegram::backoff_delay;
/// use std::time::Duration;
/// assert_eq!(backoff_delay(0), Duration::from_secs(0));
/// assert_eq!(backoff_delay(1), Duration::from_secs(1));
/// assert_eq!(backoff_delay(3), Duration::from_secs(4));
/// // capped, and it stays capped
/// assert_eq!(backoff_delay(99), Duration::from_secs(60));
/// ```
#[must_use]
pub fn backoff_delay(failures: u32) -> Duration {
    if failures == 0 {
        return Duration::ZERO;
    }
    let shift = failures.saturating_sub(1).min(16);
    let seconds = BACKOFF_BASE.as_secs().saturating_mul(1_u64 << shift);
    Duration::from_secs(seconds.min(BACKOFF_MAX.as_secs()))
}

/// What one pass of the pump did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pass {
    /// Chat events Telegram accepted, and whose cursor advance is durable.
    pub delivered: usize,
    /// Events skipped because they are not `say`'s chat events.
    pub skipped: usize,
    /// The failure that ended the pass, if one did.
    pub failure: Option<PassFailure>,
    /// How long to wait before the next pass, given the failure streak so far.
    pub retry_after: Duration,
}

/// Why a pass stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PassFailure {
    /// Telegram did not accept an event.
    Send(SendFailure),
    /// The cursor could not be read, or a checkpoint could not be made durable.
    Cursor(String),
    /// The log could not be opened or read.
    Log(String),
}

impl fmt::Display for PassFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send(failure) => write!(f, "{failure}"),
            Self::Cursor(why) | Self::Log(why) => f.write_str(why),
        }
    }
}

/// The outbound bridge for ONE session's event log.
#[derive(Debug)]
pub struct Outbound {
    log: PathBuf,
    cursor_path: PathBuf,
    label: String,
    scanned: Option<Cursor>,
    failures: u32,
}

impl Outbound {
    /// Bind a bridge to a session's meta directory.
    #[must_use]
    pub fn new(meta: &Path, label: impl Into<String>) -> Self {
        Self {
            log: meta.join(crate::events::LEGACY_CONTAINER),
            cursor_path: meta.join(CURSOR_FILE),
            label: label.into(),
            scanned: None,
            failures: 0,
        }
    }

    /// The durable cursor, as it is on disk right now.
    ///
    /// # Errors
    ///
    /// [`CursorError`] when the file exists and cannot be understood or read.
    pub fn cursor(&self) -> Result<Option<Cursor>, CursorError> {
        load_cursor(&self.cursor_path)
    }

    /// Forward every chat event after the cursor, stopping at the first that is
    /// not accepted.
    pub fn pump(&mut self, api: &Api) -> Pass {
        match self.pass(api) {
            Ok(pass) => pass,
            Err(failure) => {
                self.failures = self.failures.saturating_add(1);
                Pass {
                    delivered: 0,
                    skipped: 0,
                    failure: Some(failure),
                    retry_after: backoff_delay(self.failures),
                }
            }
        }
    }

    /// The body of a pass.
    fn pass(&mut self, api: &Api) -> Result<Pass, PassFailure> {
        // ONE OPEN, and everything about the file comes off THAT descriptor.
        let (mut file, metadata) = match open_regular(&self.log) {
            Ok(opened) => opened,
            // No log yet is not a failure: a session that has emitted no event
            // has nothing to forward.
            Err(NotRegular::Absent) => return Ok(self.quiet_pass()),
            // A directory, socket or FIFO wearing the log's name.
            Err(NotRegular::Node) => {
                return Err(PassFailure::Log(format!(
                    "telegram: event log {} is not a regular file",
                    self.log.display()
                )));
            }
            Err(NotRegular::Unreadable(why)) => {
                return Err(PassFailure::Log(format!("telegram: event log: {why}")));
            }
        };
        let inode = metadata.ino();
        let length = metadata.len();

        let durable = self
            .cursor()
            .map_err(|why| PassFailure::Cursor(why.to_string()))?;
        let mut position = self.start(durable, inode, length);

        let from = position.offset;
        let window = read_window(&mut file, from, length)
            .map_err(|why| PassFailure::Log(format!("telegram: event log: {why}")))?;

        let mut pass = Pass {
            delivered: 0,
            skipped: 0,
            failure: None,
            retry_after: Duration::ZERO,
        };
        // A FULL WINDOW WITH NO COMPLETE LINE IN IT is a record longer than the
        // pass can hold, and it would otherwise wedge the bridge in SILENCE:
        // every later pass reads the same prefix, finds no line, advances
        if as_bytes_count(window.len()) >= MAX_PASS_BYTES
            && complete_lines(&window).next().is_none()
        {
            self.failures = self.failures.saturating_add(1);
            pass.failure = Some(PassFailure::Log(format!(
                "telegram: a record at offset {from} is longer than the {MAX_PASS_BYTES}-byte \
                 pass window and cannot be forwarded"
            )));
            pass.retry_after = backoff_delay(self.failures);
            return Ok(pass);
        }
        for line in complete_lines(&window) {
            let width = as_bytes_count(line.len());
            match self.forward(api, line) {
                Forwarded::Skipped => {
                    pass.skipped += 1;
                    position.offset += width;
                }
                Forwarded::Delivered => {
                    // THE ADVANCE IS THE CHECKPOINT, and neither happens
                    // without the other.
                    let advanced = Cursor {
                        inode: position.inode,
                        offset: position.offset + width,
                    };
                    if let Err(why) = store_cursor(&self.cursor_path, &advanced) {
                        // The message IS delivered; the checkpoint is not.
                        pass.delivered += 1;
                        pass.failure = Some(PassFailure::Cursor(why.to_string()));
                        break;
                    }
                    position = advanced;
                    pass.delivered += 1;
                }
                Forwarded::Failed(failure) => {
                    // Cursor untouched: this event is still owed.
                    pass.failure = Some(PassFailure::Send(failure));
                    break;
                }
            }
        }

        self.scanned = Some(position);
        if pass.failure.is_some() {
            self.failures = self.failures.saturating_add(1);
        } else {
            self.failures = 0;
        }
        pass.retry_after = backoff_delay(self.failures);
        Ok(pass)
    }

    /// A pass that did nothing because there was nothing to do.
    fn quiet_pass(&mut self) -> Pass {
        self.failures = 0;
        Pass {
            delivered: 0,
            skipped: 0,
            failure: None,
            retry_after: Duration::ZERO,
        }
    }

    /// Where this pass starts reading.
    ///
    /// # THE INVARIANT THIS RESTS ON: `events.jsonl` IS APPEND-ONLY
    ///
    /// A byte offset is only a position in a document that never rewrites its
    /// past. The heuristic above cannot tell a same-inode file whose bytes were
    /// REPLACED from one that merely grew. That case is unreachable in ae, and
    /// the reason is a property of the producer:
    ///
    /// * the ONLY production writer of the ledger is [`crate::state::emit`] ->
    ///   `append_locked` -> `append`, which opens with `OpenOptions::append(true)`
    ///   under the container's `flock`;
    /// * `compact` only READS the ledger, never opens it for writing;
    /// * measured across `src/`, no product-side `fs::write`, `truncate(true)`,
    ///   `remove_file` or `rename` of the ledger exists;
    /// * replacement therefore always arrives as a NEW INODE — rotation, a
    ///   restored archive — and that case IS handled above.
    ///
    /// No fingerprint and no lock are added here on purpose: a guard against an
    /// unreachable case is speculative machinery.
    ///
    /// **ANY future in-place rewrite of the events ledger — an in-place ledger
    /// compaction is the obvious candidate — INVALIDATES this and must revisit
    /// the cursor.** The trigger is written here because this is
    /// where the assumption is spent.
    fn start(&self, durable: Option<Cursor>, inode: u64, length: u64) -> Cursor {
        let base = match durable {
            Some(cursor) if cursor.inode == inode && cursor.offset <= length => cursor,
            _ => Cursor { inode, offset: 0 },
        };
        match self.scanned {
            Some(scanned)
                if scanned.inode == inode
                    && scanned.offset >= base.offset
                    && scanned.offset <= length =>
            {
                scanned
            }
            _ => base,
        }
    }

    /// Forward one raw log line, if it is a chat event.
    fn forward(&self, api: &Api, line: &[u8]) -> Forwarded {
        // A line that is not UTF-8 is not an event this reader can frame.
        let Ok(text) = std::str::from_utf8(line) else {
            return Forwarded::Skipped;
        };
        let Ok(event) = crate::events::Event::parse_line(text.trim_end_matches('\n')) else {
            // A line this reader cannot parse is not a chat event it can
            // forward. tolerance applies to KEYS, not to a line that
            // is not an event at all.
            return Forwarded::Skipped;
        };
        if event.action != CHAT_ACTION {
            return Forwarded::Skipped;
        }
        match api.send_message(&self.render(&event)) {
            Ok(()) => Forwarded::Delivered,
            Err(failure) => Forwarded::Failed(failure),
        }
    }

    /// The message body one chat event becomes.
    fn render(&self, event: &crate::events::Event) -> String {
        let head = format!("[{}] {}", self.label, event.actor);
        match event.summary.as_deref() {
            Some(text) if !text.is_empty() => format!("{head}\n{text}"),
            _ => head,
        }
    }
}

/// What happened to one line.
enum Forwarded {
    /// Not a chat event; nothing was sent.
    Skipped,
    /// Telegram accepted it.
    Delivered,
    /// Telegram did not.
    Failed(SendFailure),
}

/// Read at most [`MAX_PASS_BYTES`] of the log, starting at `from`.
fn read_window(file: &mut fs::File, from: u64, length: u64) -> io::Result<Vec<u8>> {
    use std::io::{Seek as _, SeekFrom};
    if from >= length {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(from))?;
    let want = (length - from).min(MAX_PASS_BYTES);
    let mut buffer = Vec::new();
    file.take(want).read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// The COMPLETE lines in `bytes`, each including its trailing `\n`.
fn complete_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.last() == Some(&b'\n'))
}

/// A `usize` as a byte count, saturating rather than wrapping.
fn as_bytes_count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real files; the boundary is about what PRODUCT \
                  code may reach"
    )]

    use super::{
        Api, Credentials, CredentialsError, Cursor, MAX_RESPONSE_BYTES, Outbound, PassFailure,
        SendFailure, StatusClass, Token, backoff_delay, load_credentials, load_cursor,
        load_settings, parse_id_list, store_cursor, truncate_chars,
    };
    use std::collections::VecDeque;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// A token that is obviously fake and obviously searchable.
    pub(super) const FAKE_TOKEN: &str = "123456789:AAqRSTUVwxyzTOKENnotarealoneKKKK7";
    pub(super) const CHAT: &str = "-1001234567890";

    // ─── the fake Telegram ───────────────────────────────────────────────

    /// One request the fake server saw.
    #[derive(Debug, Clone)]
    pub(super) struct Seen {
        pub(super) method: String,
        pub(super) path: String,
        pub(super) content_type: Option<String>,
        pub(super) body: String,
        /// The durable cursor AT THE MOMENT this request arrived, when the fake
        /// was told to watch one.
        pub(super) cursor_on_arrival: Option<Cursor>,
    }

    /// What the fake server answers with.
    #[derive(Debug, Clone)]
    pub(super) enum Reply {
        /// A normal framed response.
        Body {
            /// The status line's code.
            status: u16,
            /// The framed body.
            body: String,
        },
        /// A chunked response of `chunks` × `chunk`, with no `Content-Length`
        /// at all — the shape a cap that trusts a declared length cannot bound.
        Chunked {
            chunk: String,
            chunks: usize,
            written: Arc<AtomicUsize>,
        },
        /// A response that DECLARES `declared` bytes and really sends them —
        /// far past the cap.
        Oversized {
            declared: usize,
            written: Arc<AtomicUsize>,
        },
    }

    impl Reply {
        pub(super) fn ok() -> Self {
            Self::json(200, r#"{"ok":true,"result":{"message_id":42}}"#)
        }
        pub(super) fn json(status: u16, body: &str) -> Self {
            Self::Body {
                status,
                body: body.to_owned(),
            }
        }
    }

    pub(super) struct Fake {
        base: String,
        seen: Arc<Mutex<Vec<Seen>>>,
        replies: Arc<Mutex<VecDeque<Reply>>>,
        stop: Arc<AtomicBool>,
    }

    impl Fake {
        /// Start a server that answers with `replies` in order, repeating the
        /// last one once the script runs out.
        pub(super) fn start(replies: Vec<Reply>) -> Self {
            Self::watching(replies, None)
        }

        /// Start a server that also reads `watch` on every request arrival.
        fn watching(replies: Vec<Reply>, watch: Option<PathBuf>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
            let stop = Arc::new(AtomicBool::new(false));
            let fake = Self {
                base: format!("http://127.0.0.1:{port}"),
                seen: Arc::clone(&seen),
                replies: Arc::clone(&queue),
                stop: Arc::clone(&stop),
            };
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // BSD/macOS hands back an accepted socket that has
                            // INHERITED the listener's O_NONBLOCK; Linux does
                            // not.
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(10)))
                                .unwrap();
                            let seen = Arc::clone(&seen);
                            let queue = Arc::clone(&queue);
                            let watch = watch.clone();
                            std::thread::spawn(move || {
                                serve(stream, &seen, &queue, watch.as_deref());
                            });
                        }
                        Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            fake
        }

        pub(super) fn one(reply: Reply) -> Self {
            Self::start(vec![reply])
        }

        pub(super) fn requests(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        pub(super) fn api(&self) -> Api {
            Api::loopback(
                self.base.clone(),
                Credentials::new(Token::new(FAKE_TOKEN), CHAT),
            )
        }

        /// Replace the scripted replies mid-test.
        pub(super) fn script(&self, replies: Vec<Reply>) {
            *self.replies.lock().unwrap() = VecDeque::from(replies);
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    fn serve(
        mut stream: TcpStream,
        seen: &Mutex<Vec<Seen>>,
        replies: &Mutex<VecDeque<Reply>>,
        watch: Option<&Path>,
    ) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
            return;
        }
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let mut length = 0usize;
        let mut content_type = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).is_err() {
                return;
            }
            let header = header.trim_end().to_owned();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim().to_owned();
                if name == "content-length" {
                    length = value.parse().unwrap_or(0);
                } else if name == "content-type" {
                    content_type = Some(value);
                }
            }
        }
        let mut body = vec![0_u8; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        seen.lock().unwrap().push(Seen {
            method,
            path,
            content_type,
            body: String::from_utf8_lossy(&body).into_owned(),
            cursor_on_arrival: watch.and_then(|path| load_cursor(path).ok().flatten()),
        });

        let reply = {
            let mut queue = replies.lock().unwrap();
            if queue.len() > 1 {
                queue.pop_front()
            } else {
                queue.front().cloned()
            }
        }
        .unwrap_or_else(Reply::ok);

        // Every write below is best-effort: a client that hits its body cap
        // drops the connection mid-response, and the resulting EPIPE is the
        // test working, not the server failing.
        match reply {
            Reply::Body { status, body } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                    phrase(status),
                    body.len()
                );
                let _ = stream.write_all(body.as_bytes());
            }
            Reply::Chunked {
                chunk,
                chunks,
                written,
            } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                );
                for sent in 0..chunks {
                    if write!(stream, "{:x}\r\n", chunk.len()).is_err()
                        || stream.write_all(chunk.as_bytes()).is_err()
                        || stream.write_all(b"\r\n").is_err()
                    {
                        return;
                    }
                    written.store(sent + 1, Ordering::Relaxed);
                }
                let _ = stream.write_all(b"0\r\n\r\n");
            }
            Reply::Oversized { declared, written } => {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {declared}\r\nConnection: close\r\n\r\n"
                );
                let block = vec![b'x'; 8192];
                let mut sent = 0;
                while sent < declared {
                    let want = block.len().min(declared - sent);
                    if stream.write_all(&block[..want]).is_err() {
                        break;
                    }
                    sent += want;
                    written.store(sent, Ordering::Relaxed);
                }
            }
        }
        let _ = stream.flush();
    }

    fn phrase(status: u16) -> &'static str {
        match status {
            200 => "OK",
            302 => "Found",
            400 => "Bad Request",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "Status",
        }
    }

    // ─── fixtures ────────────────────────────────────────────────────────

    /// A temp directory that removes itself.
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Self {
            Self::under(&std::env::temp_dir(), &format!("ae-telegram-{tag}"))
        }

        /// A SHORT temp path, for the tests that put a unix socket in it.
        fn short(tag: &str) -> Self {
            Self::under(Path::new("/tmp"), &format!("ae-tg-{tag}"))
        }

        fn under(root: &Path, stem: &str) -> Self {
            let base = root.join(format!(
                "{stem}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            Self(base)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn chat_event(actor: &str, text: &str) -> String {
        format!(
            r#"{{"ts":"2026-08-29T10:00:00Z","actor":"{actor}","action":"chat","summary":"{text}"}}"#
        )
    }

    /// A watchdog `nudge` — the action BOTH the stale nudge and the
    /// orchestrator's sweep prompt carry, on ONE line, because a record that
    /// spans two lines is skipped for being unframed rather than for being a
    /// nudge, and a fixture that passes for that reason proves nothing.
    fn nudge_event(ts: &str, summary: &str) -> String {
        format!(
            r#"{{"ts":"{ts}","actor":"watchdog","action":"nudge","target":"claude:lead","summary":"{summary}"}}"#
        )
    }

    fn other_event(actor: &str) -> String {
        format!(r#"{{"ts":"2026-08-29T10:00:00Z","actor":"{actor}","action":"done"}}"#)
    }

    fn append(meta: &Path, lines: &[String]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(meta.join("events.jsonl"))
            .unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    fn texts(fake: &Fake) -> Vec<String> {
        fake.requests()
            .iter()
            .map(|seen| {
                let value = crate::json::parse(&seen.body).unwrap();
                value.get_str("text").unwrap_or_default().to_owned()
            })
            .collect()
    }

    // ─── the request Telegram actually receives ──────────────────────────

    #[test]
    fn the_request_is_a_json_post_to_the_bot_path_carrying_chat_id_and_text() {
        let fake = Fake::one(Reply::ok());
        fake.api().send_message("hello from ae").unwrap();

        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        let seen = &requests[0];
        assert_eq!(seen.method, "POST");
        assert_eq!(seen.path, format!("/bot{FAKE_TOKEN}/sendMessage"));
        assert_eq!(seen.content_type.as_deref(), Some("application/json"));

        let body = crate::json::parse(&seen.body).unwrap();
        assert_eq!(body.get_str("text"), Some("hello from ae"));
        // A numeric chat id is JSON's number, not its string — Telegram accepts
        // both, and a quoted `@channelusername` is the case that needs the other.
        assert_eq!(
            body.get("chat_id"),
            Some(&crate::json::Value::Num(-1_001_234_567_890))
        );
    }

    #[test]
    fn a_non_numeric_chat_id_is_sent_as_a_string() {
        let fake = Fake::one(Reply::ok());
        let api = Api::loopback(
            fake.base.clone(),
            Credentials::new(Token::new(FAKE_TOKEN), "@ae_channel"),
        );
        api.send_message("hi").unwrap();
        let body = crate::json::parse(&fake.requests()[0].body).unwrap();
        assert_eq!(body.get_str("chat_id"), Some("@ae_channel"));
    }

    // ─── the token never reaches a surface anything can print ────────────

    #[test]
    fn no_failure_this_module_can_produce_carries_the_token() {
        // The URL the request is built on DOES contain the token — that is the
        // hazard, and this asserts the blast radius of it.
        let failures = [
            SendFailure::Status(StatusClass::ClientError, 401),
            SendFailure::Status(StatusClass::ServerError, 500),
            SendFailure::Status(StatusClass::Redirect, 302),
            SendFailure::Status(StatusClass::Other, 100),
            SendFailure::Rejected,
            SendFailure::Malformed,
            SendFailure::TooLarge,
            SendFailure::Timeout,
            SendFailure::Redirected,
            SendFailure::Transport,
        ];
        for failure in failures {
            for rendered in [format!("{failure}"), format!("{failure:?}")] {
                assert!(
                    !rendered.contains(FAKE_TOKEN) && !rendered.contains("123456789:"),
                    "a failure rendered the token: {rendered}"
                );
                assert!(
                    !rendered.contains("api.telegram.org") && !rendered.contains("/bot"),
                    "a failure rendered the URL: {rendered}"
                );
            }
        }
    }

    #[test]
    fn a_real_transport_failure_renders_no_token_either() {
        // Not a synthetic value: an Api pointed at a port nobody is listening on,
        // so the failure comes out of ureq's own error path.
        let api = Api::loopback(
            "http://127.0.0.1:1".to_owned(),
            Credentials::new(Token::new(FAKE_TOKEN), CHAT),
        );
        let failure = api.send_message("nobody home").unwrap_err();
        let rendered = format!("{failure} / {failure:?}");
        assert!(!rendered.contains(FAKE_TOKEN), "leaked: {rendered}");
        assert!(
            !rendered.contains("127.0.0.1"),
            "leaked the host: {rendered}"
        );
    }

    #[test]
    fn the_token_type_refuses_to_print_itself() {
        let token = Token::new(FAKE_TOKEN);
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        // And a struct that derives Debug around it inherits the redaction.
        let credentials = Credentials::new(token, CHAT);
        assert!(!format!("{credentials:?}").contains(FAKE_TOKEN));
        assert!(!format!("{:?}", Api::production(credentials)).contains(FAKE_TOKEN));
    }

    // ─── the locked agent ────────────────────────────────────────────────

    #[test]
    fn the_production_agent_is_locked_shut() {
        let api = Api::production(Credentials::new(Token::new(FAKE_TOKEN), CHAT));
        let config = api.config();
        assert!(config.https_only(), "cleartext is not refused");
        assert!(
            config.proxy().is_none(),
            "an environment-named proxy would receive the token"
        );
        assert_eq!(config.max_redirects(), 0, "a 3xx must not be followed");
        assert!(
            config.max_redirects_will_error(),
            "a redirect must be an error, not a silently returned 3xx"
        );
        let timeouts = config.timeouts();
        assert!(timeouts.connect.is_some(), "connect is unbounded");
        assert!(
            timeouts.recv_response.is_some(),
            "the response wait is unbounded"
        );
        assert!(timeouts.global.is_some(), "the whole call is unbounded");
        assert!(!config.tls_config().disable_verification());
        assert!(
            config
                .tls_config()
                .unversioned_rustls_crypto_provider()
                .is_some(),
            "the crypto provider is ureq's default rather than the pinned ring one"
        );
    }

    #[test]
    fn the_production_agent_refuses_a_cleartext_destination() {
        // The lock is not decoration: https_only(true) makes a plaintext URL
        // fail before a byte of the token leaves the process.
        let api = Api::production(Credentials::new(Token::new(FAKE_TOKEN), CHAT));
        let failure = api
            .agent
            .post("http://127.0.0.1:1/plain")
            .content_type("application/json")
            .send("{}")
            .map(|_| ())
            .unwrap_err();
        assert!(
            matches!(super::classify(failure), SendFailure::Transport),
            "a cleartext request should not have been attempted"
        );
    }

    // ─── success is Telegram's word, not the status code's ───────────────

    #[test]
    fn a_200_that_is_not_ok_true_is_a_failure() {
        for body in [
            r#"{"ok":false,"description":"chat not found"}"#,
            r#"{"result":{"message_id":1}}"#,
            r#"{"ok":"true"}"#,
            r#"{"ok":1}"#,
            r#"{"ok":null}"#,
            r#"[{"ok":true}]"#,
        ] {
            let fake = Fake::one(Reply::json(200, body));
            assert_eq!(
                fake.api().send_message("x"),
                Err(SendFailure::Rejected),
                "accepted a 200 that Telegram did not accept: {body}"
            );
        }
    }

    #[test]
    fn a_non_2xx_is_a_status_failure_with_its_class() {
        for (status, class) in [
            (400_u16, StatusClass::ClientError),
            (429, StatusClass::ClientError),
            (500, StatusClass::ServerError),
        ] {
            let fake = Fake::one(Reply::json(status, r#"{"ok":false}"#));
            assert_eq!(
                fake.api().send_message("x"),
                Err(SendFailure::Status(class, status))
            );
        }
    }

    #[test]
    fn a_client_error_is_not_transient_and_everything_else_is() {
        assert!(!SendFailure::Status(StatusClass::ClientError, 400).is_transient());
        assert!(SendFailure::Status(StatusClass::ServerError, 503).is_transient());
        assert!(SendFailure::Timeout.is_transient());
        assert!(SendFailure::Rejected.is_transient());
    }

    // ─── hostile responses ───────────────────────────────────────────────

    #[test]
    fn a_body_nested_past_max_depth_is_refused_rather_than_recursed() {
        let deep = format!("{}{}{}", "{\"a\":".repeat(200), "1", "}".repeat(200));
        let fake = Fake::one(Reply::json(200, &deep));
        assert_eq!(fake.api().send_message("x"), Err(SendFailure::Malformed));
    }

    #[test]
    fn a_body_that_is_not_json_is_refused() {
        let fake = Fake::one(Reply::json(200, "<html>upstream proxy says no</html>"));
        assert_eq!(fake.api().send_message("x"), Err(SendFailure::Malformed));
    }

    #[test]
    fn an_endless_chunked_body_is_capped_not_buffered() {
        // No Content-Length at all: the only thing that can stop this is a cap
        // on what is actually streamed.
        let written = Arc::new(AtomicUsize::new(0));
        let chunks = 8192; // 64 MiB in 8 KiB chunks, a thousand times the cap
        let fake = Fake::one(Reply::Chunked {
            chunk: "x".repeat(8192),
            chunks,
            written: Arc::clone(&written),
        });
        assert_eq!(fake.api().send_message("x"), Err(SendFailure::TooLarge));
        let got_out = written.load(Ordering::Relaxed);
        assert!(
            got_out < chunks / 4,
            "the client took {got_out} of {chunks} chunks — the cap is not stopping the \
             stream, only judging it afterwards"
        );
    }

    #[test]
    fn a_declared_length_far_past_the_cap_is_capped_while_streaming() {
        // The declared length is 64 MiB.
        let written = Arc::new(AtomicUsize::new(0));
        let declared = MAX_RESPONSE_BYTES * 1024;
        let fake = Fake::one(Reply::Oversized {
            declared,
            written: Arc::clone(&written),
        });
        assert_eq!(fake.api().send_message("x"), Err(SendFailure::TooLarge));
        let got_out = written.load(Ordering::Relaxed);
        assert!(
            got_out < declared / 4,
            "the client read {got_out} of {declared} declared bytes — the cap is not \
             stopping the stream, only judging it afterwards"
        );
    }

    #[test]
    fn a_body_exactly_at_the_cap_is_read() {
        // The boundary in the other direction: the cap refuses what is OVER it,
        // and a test that only proves the refusal cannot tell a correct cap from
        // one that refuses everything.
        let padding = MAX_RESPONSE_BYTES - r#"{"ok":true,"pad":""}"#.len();
        let body = format!(r#"{{"ok":true,"pad":"{}"}}"#, "p".repeat(padding));
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        let fake = Fake::one(Reply::json(200, &body));
        assert_eq!(fake.api().send_message("x"), Ok(()));
    }

    #[test]
    fn an_id_too_large_for_i64_neither_overflows_nor_panics() {
        let fake = Fake::one(Reply::json(
            200,
            r#"{"ok":true,"result":{"message_id":99999999999999999999999,"chat":{"id":-1009999999999999999999}}}"#,
        ));
        assert_eq!(fake.api().send_message("x"), Ok(()));
    }

    #[test]
    fn a_redirect_is_refused_rather_than_followed() {
        let fake = Fake::one(Reply::json(302, r#"{"ok":true}"#));
        // MEASURED, and it is not what the ureq docs' wording suggests:
        // `max_redirects(0)` does not raise `TooManyRedirects` — with no
        // redirect budget the 3xx is simply returned, and `http_status_as_error`
        assert_eq!(
            fake.api().send_message("x"),
            Err(SendFailure::Status(StatusClass::Redirect, 302))
        );
        assert_eq!(fake.requests().len(), 1, "the redirect was followed");
    }

    // ─── the pump and its cursor ─────────────────────────────────────────

    #[test]
    fn chat_events_are_forwarded_and_everything_else_is_skipped() {
        let temp = Temp::new("skip");
        append(
            temp.path(),
            &[
                other_event("claude:lead"),
                chat_event("claude:lead", "first"),
                other_event("codex:worker"),
                chat_event("codex:worker", "second"),
            ],
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "aerewrite");
        let pass = outbound.pump(&fake.api());

        assert_eq!(pass.delivered, 2);
        assert_eq!(pass.skipped, 2);
        assert_eq!(pass.failure, None);
        assert_eq!(
            texts(&fake),
            vec![
                "[aerewrite] claude:lead\nfirst".to_owned(),
                "[aerewrite] codex:worker\nsecond".to_owned()
            ]
        );
    }

    #[test]
    fn a_sweep_nudge_is_not_forwarded_by_default() {
        // The orchestrator's sweep prompt and the stale nudge are both the
        // `nudge` action (only their summaries differ), and neither is in the
        // default Telegram include.
        let temp = Temp::new("sweep");
        append(
            temp.path(),
            &[
                nudge_event("2026-08-29T10:00:00Z", "sweep cadence"),
                nudge_event("2026-08-29T10:00:01Z", "stale 900s"),
                chat_event("claude:lead", "this one is a say"),
            ],
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "aerewrite");
        let pass = outbound.pump(&fake.api());

        assert_eq!(pass.delivered, 1);
        assert_eq!(pass.skipped, 2, "a nudge reached the chat");
        assert_eq!(
            texts(&fake),
            vec!["[aerewrite] claude:lead\nthis one is a say".to_owned()]
        );
    }

    #[test]
    fn a_normal_restart_forwards_nothing_already_sent() {
        let temp = Temp::new("restart");
        append(
            temp.path(),
            &[chat_event("a", "one"), chat_event("a", "two")],
        );
        let fake = Fake::start(vec![Reply::ok()]);

        let mut first = Outbound::new(temp.path(), "s");
        assert_eq!(first.pump(&fake.api()).delivered, 2);
        drop(first);

        // A brand new Outbound over the same files is exactly what a restart is.
        let mut second = Outbound::new(temp.path(), "s");
        let pass = second.pump(&fake.api());
        assert_eq!(pass.delivered, 0, "a restart re-sent an accepted event");
        assert_eq!(fake.requests().len(), 2);

        // And it picks up where it left off when the log grows.
        append(temp.path(), &[chat_event("a", "three")]);
        assert_eq!(second.pump(&fake.api()).delivered, 1);
        assert_eq!(texts(&fake).last().unwrap(), "[s] a\nthree");
    }

    #[test]
    fn a_fault_between_acceptance_and_the_checkpoint_replays_exactly_one_event() {
        // THE CRASH WINDOW, modelled exactly as the contract states it:
        // Telegram accepted an event and the process died before that event's
        // checkpoint reached the disk.
        let temp = Temp::new("crashwindow");
        append(
            temp.path(),
            &[chat_event("a", "one"), chat_event("a", "two")],
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let cursor_path = temp.path().join(super::CURSOR_FILE);

        let mut original = Outbound::new(temp.path(), "s");
        assert_eq!(original.pump(&fake.api()).delivered, 2);
        let checkpointed = load_cursor(&cursor_path).unwrap().unwrap();

        // Roll the cursor back by exactly one event: "two" was accepted, its
        // checkpoint was not.
        let after_one = super::as_bytes_count(format!("{}\n", chat_event("a", "one")).len());
        assert!(
            after_one < checkpointed.offset,
            "the fixture rewound nothing"
        );
        store_cursor(
            &cursor_path,
            &Cursor {
                inode: checkpointed.inode,
                offset: after_one,
            },
        )
        .unwrap();

        // A third event arrives while the process is down, so the restart has to
        // do both jobs: replay the un-checkpointed one and deliver the new one.
        append(temp.path(), &[chat_event("a", "three")]);

        let mut restarted = Outbound::new(temp.path(), "s");
        assert_eq!(restarted.pump(&fake.api()).delivered, 2);

        let sent = texts(&fake);
        let count = |needle: &str| sent.iter().filter(|text| text.ends_with(needle)).count();
        assert_eq!(
            count("one"),
            1,
            "an already-checkpointed event was replayed"
        );
        assert_eq!(
            count("two"),
            2,
            "the crash-window event is not exactly one duplicate"
        );
        assert_eq!(
            count("three"),
            1,
            "the event written during the outage was lost"
        );
        assert_eq!(sent.len(), 4, "unexpected traffic: {sent:?}");
    }

    #[test]
    fn each_accepted_event_is_checkpointed_before_the_next_one_is_sent() {
        // R1's transaction, observed rather than inferred.
        let temp = Temp::new("checkpointorder");
        append(
            temp.path(),
            &[
                chat_event("a", "one"),
                chat_event("a", "two"),
                chat_event("a", "three"),
            ],
        );
        let cursor_path = temp.path().join(super::CURSOR_FILE);
        let fake = Fake::watching(vec![Reply::ok()], Some(cursor_path.clone()));
        let mut outbound = Outbound::new(temp.path(), "s");
        assert_eq!(outbound.pump(&fake.api()).delivered, 3);

        let widths: Vec<u64> = ["one", "two"]
            .iter()
            .map(|text| super::as_bytes_count(format!("{}\n", chat_event("a", text)).len()))
            .collect();
        let inode = load_cursor(&cursor_path).unwrap().unwrap().inode;
        let observed: Vec<Option<u64>> = fake
            .requests()
            .iter()
            .map(|seen| seen.cursor_on_arrival.map(|cursor| cursor.offset))
            .collect();
        assert_eq!(
            observed,
            vec![None, Some(widths[0]), Some(widths[0] + widths[1])],
            "a checkpoint was not durable before the next event was sent — the crash \
             window is wider than one event"
        );
        for seen in fake.requests() {
            if let Some(cursor) = seen.cursor_on_arrival {
                assert_eq!(cursor.inode, inode);
            }
        }
    }

    /// Make `dir` unwritable, run `body`, then put it back whatever happens.
    pub(super) fn with_unwritable_dir<T>(dir: &Path, body: impl FnOnce() -> T) -> T {
        let original = std::fs::metadata(dir).unwrap().permissions();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        std::fs::set_permissions(dir, original).unwrap();
        match outcome {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    #[test]
    fn a_failed_checkpoint_does_not_let_the_scan_run_past_the_owed_event() {
        // THE BOUND ON DUPLICATION.
        let temp = Temp::new("checkpointfail");
        append(
            temp.path(),
            &[
                chat_event("a", "one"),
                chat_event("a", "two"),
                chat_event("a", "three"),
            ],
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");

        let sent = with_unwritable_dir(temp.path(), || {
            let first = outbound.pump(&fake.api());
            assert_eq!(first.delivered, 1, "the send itself should have succeeded");
            assert!(
                matches!(first.failure, Some(PassFailure::Cursor(_))),
                "expected a checkpoint failure, got {:?}",
                first.failure
            );
            assert_eq!(outbound.cursor().unwrap(), None, "nothing was checkpointed");

            // Three more passes while the disk stays unwritable.
            let mut retry_after = first.retry_after;
            for _ in 0..3 {
                let again = outbound.pump(&fake.api());
                assert_eq!(again.delivered, 1);
                assert!(matches!(again.failure, Some(PassFailure::Cursor(_))));
                retry_after = again.retry_after;
            }
            (texts(&fake), retry_after)
        });
        let (sent, retry_after) = sent;

        assert!(
            sent.iter().all(|text| text.ends_with("one")),
            "the scan ran past the un-checkpointed event: {sent:?}"
        );
        assert_eq!(sent.len(), 4, "one re-send per pass, no more: {sent:?}");

        // And the re-attempts are THROTTLED by the existing failure streak
        // rather than stopped by a threshold: a persistent checkpoint failure
        // degrades to rate-capped at-least-once re-delivery of that one event.
        assert_eq!(
            retry_after,
            Duration::from_secs(8),
            "four failures should be 8s"
        );

        // Writable again: the owed event lands, is checkpointed, and the rest
        // follows.
        let recovered = outbound.pump(&fake.api());
        assert_eq!(recovered.delivered, 3);
        assert_eq!(recovered.failure, None);
        let sent = texts(&fake);
        assert_eq!(
            sent.iter().filter(|text| text.ends_with("two")).count(),
            1,
            "event two was sent more than once: {sent:?}"
        );
        assert_eq!(
            sent.iter().filter(|text| text.ends_with("three")).count(),
            1
        );
    }

    #[test]
    fn the_backoff_ceiling_bounds_a_persistent_checkpoint_failure() {
        // The documented degradation, stated as a number: whatever else a
        // broken disk does, the re-delivery of the owed event is capped at the
        // backoff ceiling rather than spinning at full speed.
        let temp = Temp::new("checkpointthrottle");
        append(temp.path(), &[chat_event("a", "one")]);
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");
        let last = with_unwritable_dir(temp.path(), || {
            let mut last = Duration::ZERO;
            for _ in 0..10 {
                last = outbound.pump(&fake.api()).retry_after;
            }
            last
        });
        assert_eq!(
            last,
            Duration::from_mins(1),
            "the ceiling is not being applied"
        );
    }

    #[test]
    fn a_failed_delivery_leaves_the_cursor_where_it_was_and_asks_for_backoff() {
        let temp = Temp::new("failhold");
        append(
            temp.path(),
            &[chat_event("a", "one"), chat_event("a", "two")],
        );
        let fake = Fake::start(vec![Reply::json(500, r#"{"ok":false}"#)]);
        let mut outbound = Outbound::new(temp.path(), "s");

        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.delivered, 0);
        assert_eq!(
            pass.failure,
            Some(PassFailure::Send(SendFailure::Status(
                StatusClass::ServerError,
                500
            )))
        );
        assert_eq!(pass.retry_after, Duration::from_secs(1));
        assert_eq!(
            outbound.cursor().unwrap(),
            None,
            "the cursor moved on a failure"
        );

        // Still failing: the wait doubles, and the same event is retried.
        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.retry_after, Duration::from_secs(2));
        assert_eq!(fake.requests().len(), 2);
        assert_eq!(texts(&fake), vec!["[s] a\none".to_owned(); 2]);

        // Recovery clears the streak and drains the backlog.
        fake.script(vec![Reply::ok()]);
        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.delivered, 2);
        assert_eq!(pass.retry_after, Duration::ZERO);
    }

    #[test]
    fn a_200_with_ok_false_holds_the_cursor_exactly_like_a_500() {
        // The case a status-code-only bridge gets wrong: HTTP says fine,
        // Telegram says no, and the event must still be owed.
        let temp = Temp::new("okfalse");
        append(temp.path(), &[chat_event("a", "one")]);
        let fake = Fake::start(vec![Reply::json(
            200,
            r#"{"ok":false,"error_code":400,"description":"chat not found"}"#,
        )]);
        let mut outbound = Outbound::new(temp.path(), "s");

        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.delivered, 0);
        assert_eq!(pass.failure, Some(PassFailure::Send(SendFailure::Rejected)));
        assert_eq!(outbound.cursor().unwrap(), None);

        fake.script(vec![Reply::ok()]);
        assert_eq!(outbound.pump(&fake.api()).delivered, 1);
        assert_eq!(
            fake.requests().len(),
            2,
            "the rejected event was not retried"
        );
    }

    #[test]
    fn a_rotated_log_starts_at_the_new_file_and_never_replays_the_old() {
        let temp = Temp::new("rotate");
        append(temp.path(), &[chat_event("a", "old-one")]);
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");
        assert_eq!(outbound.pump(&fake.api()).delivered, 1);
        let first_inode = outbound.cursor().unwrap().unwrap().inode;

        // Rotate: a genuinely different file under the same name — and a LONGER
        // one than the offset carried over from the old file.
        std::fs::rename(
            temp.path().join("events.jsonl"),
            temp.path().join("events.jsonl.1"),
        )
        .unwrap();
        append(
            temp.path(),
            &[
                chat_event("a", "new-one"),
                chat_event("a", "new-two"),
                chat_event("a", "new-three"),
            ],
        );

        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.delivered, 3, "the rotated file was mis-read");
        let cursor = outbound.cursor().unwrap().unwrap();
        assert_ne!(
            cursor.inode, first_inode,
            "the cursor kept the old file's identity"
        );
        assert_eq!(
            texts(&fake),
            vec![
                "[s] a\nold-one".to_owned(),
                "[s] a\nnew-one".to_owned(),
                "[s] a\nnew-two".to_owned(),
                "[s] a\nnew-three".to_owned(),
            ],
            "the old file's events were replayed, or the new file's were skipped"
        );
    }

    #[test]
    fn a_log_truncated_in_place_is_read_from_the_start_rather_than_skipped() {
        let temp = Temp::new("truncate");
        append(
            temp.path(),
            &[chat_event("a", "one"), chat_event("a", "two")],
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");
        assert_eq!(outbound.pump(&fake.api()).delivered, 2);

        // Same inode, shorter file: the cursor now points past the end.
        std::fs::write(
            temp.path().join("events.jsonl"),
            chat_event("a", "fresh") + "\n",
        )
        .unwrap();
        let mut restarted = Outbound::new(temp.path(), "s");
        assert_eq!(restarted.pump(&fake.api()).delivered, 1);
        assert_eq!(texts(&fake).last().unwrap(), "[s] a\nfresh");
    }

    #[test]
    fn a_half_written_line_is_not_consumed() {
        let temp = Temp::new("partial");
        let log = temp.path().join("events.jsonl");
        std::fs::write(
            &log,
            format!(
                "{}\n{}",
                chat_event("a", "complete"),
                r#"{"ts":"2026-08-29T10"#
            ),
        )
        .unwrap();
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");
        assert_eq!(outbound.pump(&fake.api()).delivered, 1);

        // The appender finishes the record; the pump now sees a whole line.
        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(
            file,
            r#":00:00Z","actor":"a","action":"chat","summary":"rest"}}"#
        )
        .unwrap();
        drop(file);
        assert_eq!(outbound.pump(&fake.api()).delivered, 1);
        assert_eq!(
            texts(&fake),
            vec!["[s] a\ncomplete".to_owned(), "[s] a\nrest".to_owned()]
        );
    }

    #[test]
    fn a_record_too_long_for_the_pass_window_fails_loudly_instead_of_stalling() {
        // The silent-wedge case: one unterminated record wider than the window.
        let temp = Temp::new("giant");
        std::fs::write(
            temp.path().join("events.jsonl"),
            "x".repeat(usize::try_from(super::MAX_PASS_BYTES).unwrap() + 10),
        )
        .unwrap();
        let fake = Fake::start(vec![Reply::ok()]);
        let mut outbound = Outbound::new(temp.path(), "s");
        let pass = outbound.pump(&fake.api());
        assert_eq!(pass.delivered, 0);
        assert!(
            matches!(pass.failure, Some(PassFailure::Log(_))),
            "an over-long record reported success: {pass:?}"
        );
        assert!(pass.retry_after > Duration::ZERO);
        assert_eq!(fake.requests().len(), 0);
        assert_eq!(outbound.cursor().unwrap(), None, "the record was dropped");
    }

    #[test]
    fn an_absent_log_is_quiet_rather_than_an_error() {
        let temp = Temp::new("nolog");
        let fake = Fake::start(vec![Reply::ok()]);
        let pass = Outbound::new(temp.path(), "s").pump(&fake.api());
        assert_eq!(pass.delivered, 0);
        assert_eq!(pass.failure, None);
        assert_eq!(pass.retry_after, Duration::ZERO);
    }

    #[test]
    fn an_unrecognised_cursor_stops_the_pass_rather_than_guessing() {
        // Guessing low re-sends the whole log; guessing high loses everything
        // before the guess.
        let temp = Temp::new("badcursor");
        append(temp.path(), &[chat_event("a", "one")]);
        std::fs::write(temp.path().join(super::CURSOR_FILE), "garbage\n").unwrap();
        let fake = Fake::start(vec![Reply::ok()]);
        let pass = Outbound::new(temp.path(), "s").pump(&fake.api());
        assert!(matches!(pass.failure, Some(PassFailure::Cursor(_))));
        assert_eq!(
            fake.requests().len(),
            0,
            "it posted despite an unreadable cursor"
        );
    }

    // ─── the cursor itself ───────────────────────────────────────────────

    #[test]
    fn a_cursor_round_trips_and_refuses_anything_else() {
        let cursor = Cursor {
            inode: 8_675_309,
            offset: 4096,
        };
        assert_eq!(Cursor::parse(&cursor.render()), Some(cursor));
        for bad in [
            "",
            "garbage",
            "ae-telegram-outbound-v1 1",
            "ae-telegram-outbound-v1 1 2 3",
            "ae-telegram-outbound-v2 1 2",
            "ae-telegram-outbound-v1 x 2",
            "ae-telegram-outbound-v1 1 -2",
        ] {
            assert_eq!(Cursor::parse(bad), None, "accepted {bad:?}");
        }
    }

    #[test]
    fn storing_a_cursor_leaves_no_temp_behind_and_reads_back_exactly() {
        let temp = Temp::new("cursorio");
        let path = temp.path().join(super::CURSOR_FILE);
        let cursor = Cursor {
            inode: 12,
            offset: 34,
        };
        store_cursor(&path, &cursor).unwrap();
        assert_eq!(load_cursor(&path).unwrap(), Some(cursor));

        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "a temp file survived: {leftovers:?}");
    }

    #[test]
    fn an_absent_cursor_is_absence_and_not_zero() {
        let temp = Temp::new("nocursor");
        assert_eq!(load_cursor(&temp.path().join("nope")).unwrap(), None);
    }

    // ─── backoff ─────────────────────────────────────────────────────────

    #[test]
    fn the_backoff_doubles_from_one_second_and_stops_at_a_minute() {
        assert_eq!(backoff_delay(0), Duration::ZERO);
        let seconds: Vec<u64> = (1..=8).map(|n| backoff_delay(n).as_secs()).collect();
        assert_eq!(seconds, vec![1, 2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(backoff_delay(u32::MAX), Duration::from_mins(1));
    }

    // ─── credentials ─────────────────────────────────────────────────────

    /// Write a token file with the custody `load_credentials` demands.
    fn write_token(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn write_config(home: &Path, body: &str) -> PathBuf {
        let path = home.join("config");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn credentials_come_from_the_telegram_section_and_the_token_file() {
        let temp = Temp::new("creds");
        write_token(&temp.path().join("tg.token"), &format!("{FAKE_TOKEN}\n"));
        let config = write_config(
            temp.path(),
            &format!(
                "[workspace]\nmain = claude\nchat_id = 999\n\n\
                 [telegram]\ntoken_file = \"~/tg.token\"\nchat_id = {CHAT}   # the control chat\n"
            ),
        );
        let credentials = load_credentials(&config, temp.path()).unwrap();
        assert_eq!(
            credentials.token.expose(),
            FAKE_TOKEN,
            "the trailing newline survived"
        );
        assert_eq!(credentials.chat_id, CHAT, "the comment was not stripped");
    }

    #[test]
    fn the_allow_list_splits_on_every_separator_and_repairs_nothing() {
        // The allow-list IS the trust predicate's input: every id that survives
        // this decides who may drive this machine's sessions from a chat.
        assert_eq!(parse_id_list("42"), vec!["42".to_owned()]);
        assert_eq!(
            parse_id_list(" 42 ,7\t9 "),
            vec!["42".to_owned(), "7".to_owned(), "9".to_owned()],
            "comma, space and tab all separate, and the parts are trimmed"
        );
        assert!(
            parse_id_list("").is_empty(),
            "no ids is not one empty id — an empty list means inbound DISABLED, \
             and a phantom entry would switch it on with an allow-list matching nobody"
        );
        assert!(
            parse_id_list("  ,\t, ,").is_empty(),
            "separators alone admit nobody"
        );
        assert_eq!(
            parse_id_list("42,not-an-id"),
            vec!["42".to_owned(), "not-an-id".to_owned()],
            "a malformed id is KEPT, so it fails to match loudly instead of being repaired \
             away — the operator sees their typo refuse traffic, not silently pass it"
        );
    }

    #[test]
    fn the_settings_carry_the_chat_id_and_the_allow_list_the_daemon_admits_on() {
        // The two values the inbound policy is built from, read back through
        // the accessors the daemon actually calls.
        let temp = Temp::new("settings");
        write_token(&temp.path().join("tg.token"), FAKE_TOKEN);
        let config = write_config(
            temp.path(),
            &format!(
                "[telegram]\ntoken_file = ~/tg.token\nchat_id = {CHAT}\n\
                 allowed_user_ids = 42, 7\n"
            ),
        );
        let settings = load_settings(&config, temp.path()).unwrap();
        assert_eq!(settings.credentials.chat_id(), CHAT);
        assert_eq!(
            settings.allowed_user_ids,
            vec!["42".to_owned(), "7".to_owned()]
        );
    }

    #[test]
    fn a_token_file_is_trimmed_of_every_line_ending_a_text_editor_can_leave() {
        // A token with a stray `\r` or trailing space is a 404 from Telegram
        // and an opaque one: the failure type CANNOT carry the token, so the
        // operator gets "not found" with nothing to look at.
        let temp = Temp::new("tokentrim");
        write_token(
            &temp.path().join("tg.token"),
            &format!(" \t{FAKE_TOKEN}\r\n"),
        );
        let config = write_config(
            temp.path(),
            &format!("[telegram]\ntoken_file = ~/tg.token\nchat_id = {CHAT}\n"),
        );
        let credentials = load_credentials(&config, temp.path()).unwrap();
        assert_eq!(credentials.token.expose(), FAKE_TOKEN);
    }

    #[test]
    fn a_commented_out_setting_is_never_honoured() {
        // Comments are skipped BEFORE the key grammar sees them, and the key
        // grammar would refuse them anyway — defence in depth over one
        // property: a setting an operator commented out must not be live.
        let temp = Temp::new("commented");
        write_token(&temp.path().join("tg.token"), FAKE_TOKEN);
        let config = write_config(
            temp.path(),
            &format!(
                "[telegram]\n# chat_id = 999\n; chat_id = 998\n\n   \n\
                 token_file = ~/tg.token\nchat_id = {CHAT}\n# allowed_user_ids = 666\n"
            ),
        );
        let settings = load_settings(&config, temp.path()).unwrap();
        assert_eq!(settings.credentials.chat_id(), CHAT);
        assert!(
            settings.allowed_user_ids.is_empty(),
            "a commented-out allow-list must leave inbound DISABLED, not admit 666"
        );
    }

    #[test]
    fn a_chat_id_outside_the_telegram_section_is_not_a_chat_id() {
        // The `chat_id = 999` above sits under `[workspace]`.
        let temp = Temp::new("section");
        write_token(&temp.path().join("tg.token"), FAKE_TOKEN);
        let config = write_config(
            temp.path(),
            "[workspace]\nchat_id = 999\n[telegram]\ntoken_file = ~/tg.token\n",
        );
        assert!(matches!(
            load_credentials(&config, temp.path()),
            Err(CredentialsError::NoChatId)
        ));
    }

    #[test]
    fn every_missing_piece_is_its_own_refusal() {
        let temp = Temp::new("credsmissing");
        let absent = temp.path().join("no-such-config");
        assert!(matches!(
            load_credentials(&absent, temp.path()),
            Err(CredentialsError::Config(_))
        ));

        let config = write_config(temp.path(), "[telegram]\nchat_id = 5\n");
        assert!(matches!(
            load_credentials(&config, temp.path()),
            Err(CredentialsError::NoTokenFile)
        ));

        let config = write_config(
            temp.path(),
            "[telegram]\ntoken_file = ~/gone\nchat_id = 5\n",
        );
        assert!(matches!(
            load_credentials(&config, temp.path()),
            Err(CredentialsError::TokenUnreadable(_))
        ));

        write_token(&temp.path().join("empty"), "\n\n");
        let config = write_config(
            temp.path(),
            "[telegram]\ntoken_file = ~/empty\nchat_id = 5\n",
        );
        assert!(matches!(
            load_credentials(&config, temp.path()),
            Err(CredentialsError::TokenEmpty)
        ));
    }

    #[test]
    fn no_config_key_can_name_a_destination() {
        // The base URL is a constant, and this is the assertion that it stays
        // one: a config full of plausible redirection keys changes nothing.
        let temp = Temp::new("nobase");
        write_token(&temp.path().join("tg.token"), FAKE_TOKEN);
        let config = write_config(
            temp.path(),
            &format!(
                "[telegram]\ntoken_file = ~/tg.token\nchat_id = {CHAT}\n\
                 base_url = http://attacker.example\napi_url = http://attacker.example\n\
                 endpoint = http://attacker.example\nhost = attacker.example\n"
            ),
        );
        let credentials = load_credentials(&config, temp.path()).unwrap();
        let api = Api::production(credentials);
        assert_eq!(api.egress.base(), "https://api.telegram.org");
    }

    // ─── text handling ───────────────────────────────────────────────────

    // ─── credential custody (FIX 4) ──────────────────────────────────────

    #[test]
    fn a_credential_path_that_is_not_a_regular_file_is_refused_before_it_is_opened() {
        // THE HAZARD IS `open(2)` ITSELF.
        let temp = Temp::new("nodekind");
        write_token(&temp.path().join("tg.token"), FAKE_TOKEN);

        // A directory where the config should be.
        let as_dir = temp.path().join("config-dir");
        std::fs::create_dir(&as_dir).unwrap();
        assert!(matches!(
            load_credentials(&as_dir, temp.path()),
            Err(CredentialsError::ConfigNotRegular(_))
        ));

        // A unix socket where the token should be.
        let socket = temp.path().join("tg.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let config = write_config(
            temp.path(),
            &format!("[telegram]\ntoken_file = ~/tg.sock\nchat_id = {CHAT}\n"),
        );
        assert!(matches!(
            load_credentials(&config, temp.path()),
            Err(CredentialsError::TokenNotRegular(_))
        ));

        // A symlink to a REGULAR file still works: the classification follows
        // links deliberately, so an operator whose config lives in a dotfiles
        // repo is not punished for the FIFO's sake.
        let linked = temp.path().join("linked.token");
        std::os::unix::fs::symlink(temp.path().join("tg.token"), &linked).unwrap();
        let config = write_config(
            temp.path(),
            &format!("[telegram]\ntoken_file = ~/linked.token\nchat_id = {CHAT}\n"),
        );
        assert!(load_credentials(&config, temp.path()).is_ok());
    }

    #[test]
    fn a_token_file_others_can_read_is_refused() {
        let temp = Temp::new("custody");
        let token = temp.path().join("tg.token");
        let config = write_config(
            temp.path(),
            &format!("[telegram]\ntoken_file = ~/tg.token\nchat_id = {CHAT}\n"),
        );
        for mode in [0o644_u32, 0o640, 0o604, 0o666, 0o601] {
            std::fs::write(&token, FAKE_TOKEN).unwrap();
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(mode)).unwrap();
            match load_credentials(&config, temp.path()) {
                Err(CredentialsError::TokenInsecurePermissions(_, got)) => {
                    assert_eq!(got, mode, "the reported mode is not the file's");
                }
                other => panic!("mode {mode:04o} was accepted: {other:?}"),
            }
        }
        // And the message says what to do about it, without quoting the token.
        let refusal = load_credentials(&config, temp.path())
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("chmod 600"),
            "unhelpful refusal: {refusal}"
        );
        assert!(
            !refusal.contains(FAKE_TOKEN),
            "the refusal quoted the token"
        );

        for mode in [0o600_u32, 0o400] {
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(mode)).unwrap();
            assert!(
                load_credentials(&config, temp.path()).is_ok(),
                "mode {mode:04o} should be acceptable"
            );
        }
    }

    // ─── special nodes at the cursor and the log (FIX 6) ─────────────────

    /// Run `body` on a thread and fail if it does not finish in time.
    fn within<T: Send + 'static>(label: &str, body: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(body());
        });
        rx.recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|why| {
                panic!("{label} did not return ({why}) — it is blocked on a special node")
            })
    }

    /// The two non-regular nodes a test can build without libc or a subprocess.
    fn plant_non_regular(path: &Path, kind: &str) -> Option<std::os::unix::net::UnixListener> {
        match kind {
            "directory" => {
                std::fs::create_dir(path).unwrap();
                None
            }
            _ => Some(std::os::unix::net::UnixListener::bind(path).unwrap()),
        }
    }

    #[test]
    fn a_special_node_at_the_cursor_path_is_a_typed_refusal_not_a_hang() {
        for kind in ["directory", "socket"] {
            // The socket case needs a short path (see `Temp::short`); the
            // directory case does not, and using both shapes keeps the ordinary
            // fixture on the ordinary path.
            let temp = if kind == "socket" {
                Temp::short("curnode")
            } else {
                Temp::new(&format!("cursornode-{kind}"))
            };
            let cursor_path = temp.path().join(super::CURSOR_FILE);
            let _held = plant_non_regular(&cursor_path, kind);

            let probe = cursor_path.clone();
            let loaded = within("load_cursor", move || {
                load_cursor(&probe).map_err(|why| why.to_string())
            });
            match loaded {
                Err(message) => assert!(
                    message.contains("is not a regular file"),
                    "{kind}: unexpected refusal: {message}"
                ),
                Ok(other) => panic!("{kind}: a special node read as a cursor: {other:?}"),
            }

            // And the pump stops on it rather than posting with a cursor it
            // could not read — the same rule as an unrecognised cursor.
            append(temp.path(), &[chat_event("a", "one")]);
            let fake = Fake::start(vec![Reply::ok()]);
            let mut outbound = Outbound::new(temp.path(), "s");
            let api = fake.api();
            let pass = within("pump", move || outbound.pump(&api));
            assert!(
                matches!(pass.failure, Some(PassFailure::Cursor(_))),
                "{kind}: {pass:?}"
            );
            assert_eq!(fake.requests().len(), 0, "{kind}: it posted anyway");
        }
    }

    #[test]
    fn a_special_node_at_the_log_path_is_a_typed_refusal_not_a_hang() {
        for kind in ["directory", "socket"] {
            let temp = if kind == "socket" {
                Temp::short("lognode")
            } else {
                Temp::new(&format!("lognode-{kind}"))
            };
            let log = temp.path().join("events.jsonl");
            let _held = plant_non_regular(&log, kind);

            let fake = Fake::start(vec![Reply::ok()]);
            let mut outbound = Outbound::new(temp.path(), "s");
            let api = fake.api();
            let pass = within("pump", move || outbound.pump(&api));
            match pass.failure {
                Some(PassFailure::Log(ref message)) => assert!(
                    message.contains("is not a regular file"),
                    "{kind}: unexpected refusal: {message}"
                ),
                other => panic!("{kind}: a special node read as an event log: {other:?}"),
            }
            assert_eq!(pass.delivered, 0);
            assert_eq!(fake.requests().len(), 0, "{kind}: it posted anyway");
        }
    }

    #[test]
    fn absence_still_means_absence_at_both_paths() {
        // The refusal above must not have swallowed the two states that are NOT
        // failures: no cursor yet is a fresh start, and no log yet is a quiet
        // pass.
        let temp = Temp::new("stillabsent");
        assert_eq!(
            load_cursor(&temp.path().join(super::CURSOR_FILE)).unwrap(),
            None
        );
        let fake = Fake::start(vec![Reply::ok()]);
        let pass = Outbound::new(temp.path(), "s").pump(&fake.api());
        assert_eq!(pass.failure, None);
        assert_eq!(pass.delivered, 0);
    }

    // ─── the cursor temp (FIX 5) ─────────────────────────────────────────

    #[test]
    fn a_symlink_planted_at_the_temp_path_does_not_get_its_target_truncated() {
        // The temp name is predictable — it has to be, so a leftover can be
        // cleaned up — which makes it a place to plant a symlink.
        let temp = Temp::new("symlinktemp");
        let victim = temp.path().join("precious");
        std::fs::write(&victim, "do not lose me").unwrap();
        let cursor_path = temp.path().join(super::CURSOR_FILE);
        let planted =
            temp.path()
                .join(format!("{}.tmp.{}", super::CURSOR_FILE, std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).unwrap();

        let cursor = Cursor {
            inode: 7,
            offset: 21,
        };
        store_cursor(&cursor_path, &cursor).unwrap();

        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "do not lose me",
            "the symlink was followed and the victim truncated"
        );
        assert_eq!(load_cursor(&cursor_path).unwrap(), Some(cursor));
        assert!(!planted.exists(), "the planted link survived the write");
    }

    #[test]
    fn a_leftover_temp_from_a_dead_process_does_not_wedge_the_checkpoint() {
        let temp = Temp::new("leftovertemp");
        let cursor_path = temp.path().join(super::CURSOR_FILE);
        let leftover =
            temp.path()
                .join(format!("{}.tmp.{}", super::CURSOR_FILE, std::process::id()));
        std::fs::write(&leftover, "half a cursor from a process that died").unwrap();
        let cursor = Cursor {
            inode: 3,
            offset: 9,
        };
        store_cursor(&cursor_path, &cursor).unwrap();
        assert_eq!(load_cursor(&cursor_path).unwrap(), Some(cursor));
    }

    #[test]
    fn the_cursor_is_written_owner_only() {
        // `mode(0o600)` at creation, not a chmod afterwards: the rename carries
        // the temp's bits, so the cursor is never briefly group-readable.
        let temp = Temp::new("cursormode");
        let cursor_path = temp.path().join(super::CURSOR_FILE);
        store_cursor(
            &cursor_path,
            &Cursor {
                inode: 1,
                offset: 2,
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&cursor_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the cursor is not owner-only");
    }

    #[test]
    fn a_long_message_is_truncated_on_a_character_boundary() {
        let multibyte = "é".repeat(5000);
        let truncated = truncate_chars(&multibyte, super::MAX_TEXT_CHARS);
        assert!(truncated.ends_with("…(truncated)"));
        assert_eq!(
            truncated.chars().count(),
            super::MAX_TEXT_CHARS + "…(truncated)".chars().count()
        );
        // Short text is untouched, marker and all.
        assert_eq!(truncate_chars("short", super::MAX_TEXT_CHARS), "short");
    }

    #[test]
    fn a_chat_event_with_no_summary_still_carries_its_header() {
        let temp = Temp::new("nosummary");
        std::fs::write(
            temp.path().join("events.jsonl"),
            "{\"ts\":\"2026-08-29T10:00:00Z\",\"actor\":\"a\",\"action\":\"chat\"}\n",
        )
        .unwrap();
        let fake = Fake::start(vec![Reply::ok()]);
        assert_eq!(
            Outbound::new(temp.path(), "s").pump(&fake.api()).delivered,
            1
        );
        assert_eq!(texts(&fake), vec!["[s] a".to_owned()]);
    }
}
