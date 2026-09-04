//! `telegram start|stop|status` — the machine-global bridge's LIFECYCLE.
//!
//! P4.3 gave the core the bridge itself (`_telegram-run`, see
//! [`crate::telegram::bridge`]) and left bash managing it: `cmd_telegram_start`
//! / `_stop` / `_status` (ae:8202-8330), `_telegram_spawn_daemon` (ae:7926) and
//! `_telegram_autostart_if_enabled` (ae:8076) owned the intent flag, the
//! control lock, the tmux session and the autostart. This module is that
//! management, ported.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use crate::inventory::ServerId;
use crate::session_tmux::{Op, argv};
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::telegram::bridge::Paths;
use crate::{lifecycle, tmux, transport};

/// The frozen usage line, plus the two path and two server flags.
pub const USAGE: &str = "Usage: ae telegram <start|stop|status> [--config <file>] [--home <dir>] [--server-kind <kind>] [--server <value>]";

/// The tmux session the bridge runs in — the frozen `_TELEGRAM_TMUX_SESSION`.
pub const TMUX_SESSION: &str = "ae-telegram";

/// The bridge's own state directory under `<ae-home>`.
const STATE_DIR: &str = "telegram";

/// The config section this lifecycle reads and rewrites.
const SECTION: &str = "telegram";

/// The control lock's name inside it.
const CONTROL_LOCK: &str = "control.lock";

/// The last-refusal record's name inside it.
const REFUSAL_FILE: &str = "autostart-refusal";

/// How long `start`/`stop` wait for the control lock — the frozen `flock -w 5`.
const CONTROL_WAIT: Duration = Duration::from_secs(5);

/// The two state files `status` reports on, in the frozen order.
const STATE_FILES: [&str; 2] = ["tg_offset", "current_target"];

/// What the argv asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Start,
    Stop,
    Status,
}

impl Action {
    /// The action a word names, or `None` when it names none.
    fn parse(word: &str) -> Option<Self> {
        match word {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "status" => Some(Self::Status),
            _ => None,
        }
    }
}

/// Why an autostart did not start the bridge — the CLOSED grammar the record
/// file and `status` share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The configured token file could not be validated.
    TokenUnreadable,
    /// The spawn itself failed.
    SpawnFailed,
}

impl Refusal {
    /// The recorded spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TokenUnreadable => "token-unreadable",
            Self::SpawnFailed => "spawn-failed",
        }
    }

    /// The refusal a recorded word names, or `None` for anything else.
    #[must_use]
    pub fn parse(word: &str) -> Option<&'static str> {
        match word {
            "token-unreadable" => Some("token-unreadable"),
            "spawn-failed" => Some("spawn-failed"),
            "aewatch-live" => Some("aewatch-live"),
            "same-token-live" => Some("same-token-live"),
            "probe-failed" => Some("probe-failed"),
            _ => None,
        }
    }
}

/// `telegram <start|stop|status> [--config <file>] [--home <dir>]`.
///
/// # Errors
///
/// Only writing to `out`/`err`; every refusal is an exit code, not an `Err`.
pub fn run(
    ae_home: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let mut paths = Paths::under(ae_home);
    let mut action = None;
    let mut server_kind = String::new();
    let mut server_value = String::new();
    let mut rest = tail;
    while let [word, after @ ..] = rest {
        rest = after;
        match word.as_str() {
            "--config" | "--home" | "--server-kind" | "--server" => {
                let Some((value, tail)) = rest.split_first() else {
                    writeln!(err, "{USAGE}")?;
                    return Ok(EXIT_USAGE);
                };
                match word.as_str() {
                    "--config" => paths.config = value.into(),
                    "--home" => paths.home = value.into(),
                    "--server-kind" => value.clone_into(&mut server_kind),
                    _ => value.clone_into(&mut server_value),
                }
                rest = tail;
            }
            _ if action.is_none() => {
                let Some(parsed) = Action::parse(word) else {
                    writeln!(err, "{USAGE}")?;
                    return Ok(EXIT_USAGE);
                };
                action = Some(parsed);
            }
            extra => {
                writeln!(
                    err,
                    "Error: unexpected extra argument '{extra}' — ae telegram takes one subcommand."
                )?;
                return Ok(EXIT_USAGE);
            }
        }
    }
    let server = match server_of(&server_kind, &server_value) {
        Ok(server) => server,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(EXIT_USAGE);
        }
    };
    // The frozen dispatcher defaulted a bare `ae telegram` to `status`.
    match action.unwrap_or(Action::Status) {
        Action::Status => status(&paths, &server, out),
        Action::Start => start(&paths, &server, out, err),
        Action::Stop => stop(&paths, &server, out, err),
    }
}

/// The tmux server these commands address, or the refusal to guess one.
///
/// # Errors
///
/// The operator-facing line, naming the kind it could not use.
fn server_of(kind: &str, value: &str) -> Result<ServerId, String> {
    ServerId::from_typed_flags(kind, value)
}

/// `telegram start` — validate, enable, spawn, and prove it came up.
fn start(
    paths: &Paths,
    server: &ServerId,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    // BEFORE the lock and before the intent flag: a start that cannot possibly
    // work must not leave `enabled = true` behind for a later autostart to act
    // on.
    if let Err(why) = crate::telegram::load_settings(&paths.config, &paths.home) {
        writeln!(err, "Error: {why}")?;
        return Ok(EXIT_FAILED);
    }
    let Ok(_held) = control_lock(&paths.ae_home, CONTROL_WAIT) else {
        writeln!(
            err,
            "ae telegram: busy (start/stop/supervise in progress) — try again"
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Err(why) = persist_intent(&paths.config, true) {
        writeln!(err, "Error: could not record telegram.enabled ({why})")?;
        return Ok(EXIT_FAILED);
    }
    match daemon_running(server) {
        Some(true) => {
            writeln!(
                out,
                "ae telegram: already running (tmux session {TMUX_SESSION})"
            )?;
            return Ok(0);
        }
        // Only a VERIFIED absence may start a machine-global daemon: a second
        // bridge long-polling one bot token loses updates, silently.
        None => {
            writeln!(
                err,
                "ae telegram: tmux did not answer — a running bridge cannot be ruled out, so nothing was started"
            )?;
            return Ok(EXIT_FAILED);
        }
        Some(false) => {}
    }
    spawn_daemon(paths, server);
    if daemon_running(server) == Some(true) {
        writeln!(out, "ae telegram: started (tmux session {TMUX_SESSION})")?;
        return Ok(0);
    }
    writeln!(err, "Error: failed to spawn daemon")?;
    Ok(EXIT_FAILED)
}

/// `telegram stop` — disable, then kill, under one lock.
fn stop(
    paths: &Paths,
    server: &ServerId,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let Ok(_held) = control_lock(&paths.ae_home, CONTROL_WAIT) else {
        writeln!(
            err,
            "ae telegram: busy (start/stop/supervise in progress) — try again"
        )?;
        return Ok(EXIT_FAILED);
    };
    if let Err(why) = persist_intent(&paths.config, false) {
        writeln!(err, "Error: could not record telegram.enabled ({why})")?;
        return Ok(EXIT_FAILED);
    }
    match daemon_running(server) {
        Some(true) => {
            // By ID, never by name: `kill-session -t <name>` PREFIX-MATCHES, so a
            // neighbour called `ae-telegram-old` is exactly what a name target
            // can take instead.
            if let Some(id) = transport::observe_session_id(server, TMUX_SESSION) {
                let _ = transport::kill_session(server, &id);
            }
            writeln!(out, "ae telegram: stopped")?;
        }
        Some(false) => writeln!(out, "ae telegram: was not running")?,
        None => {
            writeln!(
                err,
                "Error: tmux did not answer — the bridge could not be stopped and may still be running (intent is now disabled)."
            )?;
            return Ok(EXIT_FAILED);
        }
    }
    Ok(0)
}

/// `telegram status` — intent, backend, runtime, and the facts behind them.
fn status(paths: &Paths, server: &ServerId, out: &mut impl Write) -> crate::Result<u8> {
    let config = read_config(&paths.config);
    let intent = config.as_deref().is_some_and(enabled_in);
    let runtime = match daemon_running(server) {
        Some(true) => "running",
        Some(false) => "stopped",
        None => "unknown (tmux did not answer)",
    };
    let state_dir = paths.ae_home.join(STATE_DIR);
    writeln!(out, "ae telegram:")?;
    writeln!(
        out,
        "  intent:  enabled={intent} (in {})",
        paths.config.display()
    )?;
    writeln!(out, "  backend: ae core (state in {})", state_dir.display())?;
    writeln!(
        out,
        "  runtime: daemon {runtime} (tmux session {TMUX_SESSION})"
    )?;
    if let Some((category, at)) = last_refusal(&state_dir) {
        writeln!(out, "  autostart: last refusal={category} at {at}")?;
    }
    // The RESOLVED path, so the row names the binary a daemon would actually
    // be started as rather than the link this status happened to be typed
    // through.
    match crate::shape::resolved_exe() {
        Some(core) => writeln!(out, "  core:    {}", core.display())?,
        None => writeln!(out, "  core:    unknown (this process cannot name itself)")?,
    }
    // The token row reports the DAEMON's own verdict, not a second opinion: the
    // same loader, so a status that says OK is a start that will not refuse.
    match section_value(config.as_deref().unwrap_or_default(), "token_file") {
        Some(token_file) if !token_file.is_empty() => {
            match crate::telegram::load_settings(&paths.config, &paths.home) {
                Ok(_) => writeln!(out, "  token:   OK ({token_file})")?,
                Err(why) => writeln!(out, "  token:   ERROR — {why}")?,
            }
        }
        _ => writeln!(out, "  token:   not configured")?,
    }
    let present: Vec<&str> = STATE_FILES
        .into_iter()
        .filter(|name| lifecycle::path_exists(&state_dir.join(name)))
        .collect();
    if present.is_empty() {
        writeln!(out, "  state:   none yet (in {})", state_dir.display())?;
    } else {
        writeln!(
            out,
            "  state:   {} (in {})",
            present.join(" "),
            state_dir.display()
        )?;
    }
    // An explicit include= that omits `chat` silently drops agents' `say`
    // replies.
    if let Some(include) = section_value(config.as_deref().unwrap_or_default(), "include")
        && !include.is_empty()
    {
        writeln!(out, "  include: {include}")?;
        if !include
            .split([',', ' ', '\t'])
            .any(|word| word.trim() == "chat")
        {
            writeln!(
                out,
                "           WARN: 'chat' not in include — agent 'say' replies will NOT forward to Telegram. Add 'chat' to [telegram] include."
            )?;
        }
    }
    Ok(0)
}

/// The launch's best-effort revive: start the bridge IF the config asks for one
/// and none is running.
///
/// # Errors
///
/// Only writing the warning to `err`. A launch is never failed by this.
pub fn autostart(
    paths: &Paths,
    server: &ServerId,
    session: &str,
    session_dir: &Path,
    err: &mut impl Write,
) -> crate::Result<bool> {
    let Some(config) = read_config(&paths.config) else {
        return Ok(false);
    };
    if !enabled_in(&config) {
        return Ok(false);
    }
    if daemon_running(server) != Some(false) {
        // Running, or unanswerable: neither is licence to spawn a second bridge.
        return Ok(false);
    }
    if let Err(why) = crate::telegram::load_settings(&paths.config, &paths.home) {
        record_refusal(paths, Refusal::TokenUnreadable, session, session_dir);
        writeln!(err, "ae telegram: skipped autostart — {why}")?;
        return Ok(false);
    }
    // NON-BLOCKING: if a start/stop is mid-flight this tick is skipped, because
    // autostart must never delay a session launch.
    let Ok(_held) = control_lock(&paths.ae_home, Duration::ZERO) else {
        return Ok(false);
    };
    // Re-read intent and liveness UNDER the lock: a same-second `stop` must win.
    let Some(config) = read_config(&paths.config) else {
        return Ok(false);
    };
    if !enabled_in(&config) || daemon_running(server) != Some(false) {
        return Ok(false);
    }
    spawn_daemon(paths, server);
    if daemon_running(server) == Some(true) {
        return Ok(true);
    }
    record_refusal(paths, Refusal::SpawnFailed, session, session_dir);
    writeln!(
        err,
        "ae telegram: autostart failed (continuing without bridge)"
    )?;
    Ok(false)
}

/// Whether the bridge's tmux session is live — `None` when tmux did not answer.
#[must_use]
pub fn daemon_running(server: &ServerId) -> Option<bool> {
    match transport::verify_session_absent(server, TMUX_SESSION) {
        crate::tmux::StopProbe::Present => Some(true),
        crate::tmux::StopProbe::Absent => Some(false),
        crate::tmux::StopProbe::Unknown => None,
    }
}

/// Start the bridge in its own tmux session, and dress that session.
fn spawn_daemon(paths: &Paths, server: &ServerId) {
    // RESOLVED, never raw: the daemon's command is direct argv, and an
    // unresolved macOS answer would hand tmux whichever link started it.
    let Some(core) = crate::shape::resolved_exe() else {
        return;
    };
    let command = vec![
        core.display().to_string(),
        crate::cli::TELEGRAM_RUN.to_owned(),
        paths.ae_home.display().to_string(),
        "--config".to_owned(),
        paths.config.display().to_string(),
        "--home".to_owned(),
        paths.home.display().to_string(),
    ];
    let work_dir = paths.home.display().to_string();
    let (succeeded, _) = transport::run_tmux_op(&argv(
        server,
        &Op::NewDaemonSession {
            name: TMUX_SESSION,
            work_dir: &work_dir,
            command: &command,
        },
    ));
    if !succeeded {
        return;
    }
    // Cosmetic only — these may fail without the bridge being any less started.
    if let Some(id) = transport::observe_session_id(server, TMUX_SESSION) {
        let _ = transport::publish_option(server, tmux::OptionScope::Session, &id, "status", "off");
        let _ = transport::rename_window(server, &id, TMUX_SESSION);
    }
}

/// Take the machine-global control lock, waiting at most `wait`.
fn control_lock(ae_home: &Path, wait: Duration) -> io::Result<std::fs::File> {
    let dir = ae_home.join(STATE_DIR);
    std::fs::create_dir_all(&dir)?;
    crate::state::acquire(&dir.join(CONTROL_LOCK), wait)
}

/// Persist one refusal category, and mirror it into the launching session's
/// event ledger when there is a usable one.
fn record_refusal(paths: &Paths, refusal: Refusal, session: &str, session_dir: &Path) {
    let at = crate::time::Timestamp::now().to_string();
    let dir = paths.ae_home.join(STATE_DIR);
    if std::fs::create_dir_all(&dir).is_ok() {
        let record = format!("{}\t{at}\n", refusal.as_str());
        let _ = publish(&dir.join(REFUSAL_FILE), record.as_bytes(), 0o600);
    }
    // A session name is optional in the record and is never needed to identify
    // the refusal — so it is validated before it becomes an event target, rather
    // than letting legacy metadata become a new interpolation sink.
    if !lifecycle::name_is_valid(session) {
        return;
    }
    let _ = crate::state::emit(
        session_dir,
        &crate::tracked::event_line(&crate::tracked::EventFields {
            ts: crate::time::Timestamp::now(),
            actor: "ae",
            action: "telegram_autostart_refused",
            target: session,
            reference: "",
            actor_slot: "",
            actor_session: "",
            target_slot: "",
            target_session: "",
            summary: &format!("category={}", refusal.as_str()),
            body_file: "",
        }),
    );
}

/// The last recorded refusal, when the file holds ONE row of the closed
/// grammar.
#[must_use]
pub fn last_refusal(state_dir: &Path) -> Option<(&'static str, String)> {
    let raw = read_file(&state_dir.join(REFUSAL_FILE))?;
    interpret_refusal(&raw)
}

/// The record grammar: `<category>\t<YYYY-MM-DDTHH:MM:SSZ>`, one row.
fn interpret_refusal(raw: &str) -> Option<(&'static str, String)> {
    let row = raw.strip_suffix('\n').unwrap_or(raw);
    if row.contains('\n') {
        return None;
    }
    let (category, at) = row.split_once('\t')?;
    let category = Refusal::parse(category)?;
    if !timestamp_shaped(at) {
        return None;
    }
    Some((category, at.to_owned()))
}

/// Whether `text` is exactly `YYYY-MM-DDTHH:MM:SSZ`.
fn timestamp_shaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| match index {
        4 | 7 => *byte == b'-',
        10 => *byte == b'T',
        13 | 16 => *byte == b':',
        19 => *byte == b'Z',
        _ => byte.is_ascii_digit(),
    })
}

/// Whether `[telegram] enabled` in this config text is a config-affirmative.
#[must_use]
pub fn enabled_in(config: &str) -> bool {
    matches!(
        section_value(config, "enabled")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// One `[telegram]` key's value, last assignment winning.
#[must_use]
pub fn section_value(config: &str, key: &str) -> Option<String> {
    let mut found = None;
    let mut in_section = false;
    for raw in config.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = inner.trim() == SECTION;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let value = value.trim();
        let value = if let Some(quoted) = value
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        {
            quoted.to_owned()
        } else {
            value
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        found = Some(value);
    }
    found
}

/// Write `enabled = <value>` into `[telegram]` without disturbing anything else.
///
/// # Errors
///
/// The read or the atomic publish failing.
pub fn persist_intent(config: &Path, value: bool) -> io::Result<()> {
    let current = read_config(config);
    let mode = current.as_ref().map_or(0o600, |_| config_mode(config));
    let next = rewritten(current.as_deref(), value);
    publish(config, next.as_bytes(), mode)
}

/// The config text with `[telegram] enabled` set — the frozen awk pass, ported.
#[must_use]
pub fn rewritten(current: Option<&str>, value: bool) -> String {
    let value = if value { "true" } else { "false" };
    let Some(current) = current else {
        return format!("[{SECTION}]\nenabled = {value}\n");
    };
    let header = format!("[{SECTION}]");
    if !current.lines().any(|line| line.trim() == header) {
        let separator = if current.is_empty() || current.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return format!("{current}{separator}\n[{SECTION}]\nenabled = {value}\n");
    }
    let mut out = String::with_capacity(current.len() + 32);
    let setting = format!("enabled = {value}\n");
    let mut in_section = false;
    let mut done = false;
    for line in current.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            in_section = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Leaving the section without having written the key: it belongs
            // BEFORE the next header, not after it.
            if in_section && !done {
                out.push_str(&setting);
                done = true;
            }
            in_section = false;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_section
            && !done
            && let Some((name, _)) = trimmed.split_once('=')
            && name.trim() == "enabled"
        {
            out.push_str(&setting);
            done = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_section && !done {
        out.push_str(&setting);
    }
    out
}

/// Publish `bytes` at `path` atomically: a temp beside it, the mode set on the
/// temp, then a rename — so a reader sees the old file or the whole new one.
fn publish(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let directory = path.parent().unwrap_or(Path::new("."));
    let temp = directory.join(format!(
        "{}.tmp.{}",
        path.file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("ae-telegram"),
        std::process::id()
    ));
    // `create_new` is `O_EXCL`, which never follows an existing node: the temp
    // name is predictable, and without this a planted symlink there would be
    // followed and whatever it points at truncated.
    let _ = std::fs::remove_file(&temp);
    let staged = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp);
    let mut file = staged?;
    if let Err(why) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(why);
    }
    drop(file);
    if let Err(why) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(why);
    }
    Ok(())
}

/// The config file's own mode, so a rewrite does not re-permission it.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the config's own mode, so the atomic rewrite preserves it — see clippy.toml"
)]
fn config_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).map_or(0o600, |meta| meta.permissions().mode() & 0o7777)
}

/// The config file's text, or `None` when there is nothing readable there.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the INI config this lifecycle reads and rewrites — see clippy.toml"
)]
fn read_config(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// A small state file's text, or `None`.
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the autostart-refusal record — see clippy.toml"
)]
fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        Action, Refusal, USAGE, enabled_in, interpret_refusal, rewritten, section_value,
        timestamp_shaped,
    };

    #[test]
    fn the_three_subcommands_are_the_whole_grammar() {
        assert_eq!(Action::parse("start"), Some(Action::Start));
        assert_eq!(Action::parse("stop"), Some(Action::Stop));
        assert_eq!(Action::parse("status"), Some(Action::Status));
        // `setup` was DROPPED, not renamed: it must not parse into anything.
        for word in ["setup", "restart", "_supervise", "", "Status"] {
            assert_eq!(Action::parse(word), None, "'{word}' is not an action");
        }
        assert!(USAGE.contains("start|stop|status"));
    }

    #[test]
    fn only_a_config_affirmative_enables_the_bridge() {
        for text in [
            "[telegram]\nenabled = true\n",
            "[telegram]\nenabled = TRUE\n",
            "[telegram]\nenabled = 1\n",
            "[telegram]\nenabled = yes\n",
            "[telegram]\nenabled = on\n",
            "[telegram]\nenabled = \"true\"\n",
        ] {
            assert!(enabled_in(text), "{text:?} enables");
        }
        for text in [
            "",
            "[telegram]\n",
            "[telegram]\nenabled = false\n",
            "[telegram]\nenabled =\n",
            "[telegram]\nenabled = true-ish\n",
            // The exact hazard the section scope exists for: another section's
            // key must never be read as this one's.
            "[workspace]\nenabled = true\n",
            "[telegram]\n[workspace]\nenabled = true\n",
            // A commented-out value is not a value.
            "[telegram]\n# enabled = true\n",
        ] {
            assert!(!enabled_in(text), "{text:?} does not enable");
        }
    }

    #[test]
    fn an_inline_comment_is_not_part_of_the_value() {
        assert_eq!(
            section_value("[telegram]\ninclude = chat,state # only these\n", "include").as_deref(),
            Some("chat,state")
        );
        // A quoted value keeps everything inside the quotes, `#` included.
        assert_eq!(
            section_value("[telegram]\ntoken_file = \"~/my # token\"\n", "token_file").as_deref(),
            Some("~/my # token")
        );
    }

    #[test]
    fn the_rewrite_replaces_inside_the_section_and_leaves_everything_else() {
        let before = "[workspace]\nmain = lead\nenabled = true\n\n[telegram]\nchat_id = 7\nenabled = false\n\n[prompt]\ninstructions = hi\n";
        let after = rewritten(Some(before), true);
        assert_eq!(
            after,
            "[workspace]\nmain = lead\nenabled = true\n\n[telegram]\nchat_id = 7\nenabled = true\n\n[prompt]\ninstructions = hi\n",
            "only [telegram]'s enabled changes"
        );
    }

    #[test]
    fn the_rewrite_inserts_before_the_next_section_when_the_key_is_absent() {
        let after = rewritten(Some("[telegram]\nchat_id = 7\n\n[prompt]\nx = 1\n"), false);
        assert_eq!(
            after, "[telegram]\nchat_id = 7\n\nenabled = false\n[prompt]\nx = 1\n",
            "the key lands inside [telegram], never after the next header"
        );
    }

    #[test]
    fn the_rewrite_appends_the_section_or_writes_a_whole_file() {
        assert_eq!(
            rewritten(None, true),
            "[telegram]\nenabled = true\n",
            "no config at all"
        );
        assert_eq!(
            rewritten(Some("[workspace]\nmain = lead\n"), true),
            "[workspace]\nmain = lead\n\n[telegram]\nenabled = true\n",
            "a config with no [telegram] section gains one"
        );
        assert_eq!(
            rewritten(Some("[workspace]\nmain = lead"), true),
            "[workspace]\nmain = lead\n\n[telegram]\nenabled = true\n",
            "a config with no trailing newline is not joined onto the header"
        );
        assert_eq!(
            rewritten(Some("[telegram]"), true),
            "[telegram]\nenabled = true\n",
            "a section that is the last line gains the key at EOF"
        );
    }

    #[test]
    fn a_rewrite_round_trips_through_the_reader() {
        let text = rewritten(Some("[telegram]\nchat_id = 7\n"), true);
        assert!(enabled_in(&text));
        let text = rewritten(Some(&text), false);
        assert!(!enabled_in(&text));
    }

    #[test]
    fn only_a_closed_category_and_a_real_timestamp_are_a_record() {
        assert_eq!(
            interpret_refusal("spawn-failed\t2026-09-03T10:11:12Z\n"),
            Some(("spawn-failed", "2026-09-03T10:11:12Z".to_owned()))
        );
        // The retired categories still DISPLAY: a machine that ran the frozen
        // glue can hold one, and refusing it would hide a refusal that happened.
        assert!(interpret_refusal("aewatch-live\t2026-09-03T10:11:12Z\n").is_some());
        for raw in [
            "",
            "spawn-failed",
            "spawn-failed\tyesterday\n",
            "whatever\t2026-09-03T10:11:12Z\n",
            // Two rows: the writer emits one, so more than one is not our record.
            "spawn-failed\t2026-09-03T10:11:12Z\nspawn-failed\t2026-09-03T10:11:13Z\n",
            // The display sink an arbitrary value would reach.
            "$(rm -rf /)\t2026-09-03T10:11:12Z\n",
        ] {
            assert_eq!(interpret_refusal(raw), None, "{raw:?} is not a record");
        }
    }

    #[test]
    fn the_recorded_categories_round_trip() {
        for refusal in [Refusal::TokenUnreadable, Refusal::SpawnFailed] {
            assert_eq!(Refusal::parse(refusal.as_str()), Some(refusal.as_str()));
        }
    }

    #[test]
    fn the_timestamp_shape_is_the_whole_check() {
        assert!(timestamp_shaped("2026-09-03T10:11:12Z"));
        for text in [
            "2026-09-03T10:11:12",
            "2026-09-03 10:11:12Z",
            "26-09-03T10:11:12Z",
            "2026-09-03T10:11:12Z ",
            "aaaa-bb-ccTdd:ee:ffZ",
        ] {
            assert!(!timestamp_shaped(text), "{text:?} is not a timestamp");
        }
    }
}
