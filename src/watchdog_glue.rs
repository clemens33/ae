//! What the watchdog PANE still did in bash after the core owned the loop.
//!
//! Slice A.3's cut: `helper_watchdog_main`'s `_run` becomes a pane that execs
//! `ae-core _watchdog-run`, so everything that wrapper did AROUND the core child
//! has to be here — otherwise a session whose pane runs only the core silently
//! loses it. The frozen wrapper (ae:14355-14477) did five things:
//!
//! * published its pid ATOMICALLY, so a serialized starter can never read a
//!   half-written pidfile and spawn a duplicate (ae:14352-14356);
//! * printed the pane's banner;
//! * kept `@ae_branch_status` / `@ae_branch_name` fresh every cycle — a git
//!   read the daemon loop never owned;
//! * ticked the two deferred concerns: pending tool-session-id recovery and the
//!   Telegram bridge revive, both by running the recorded `ae` binary. The
//!   recovery is IN-PROCESS now — [`recover`] takes one look per pending seat
//!   through the core's own capture — and only the bridge revive still runs a
//!   binary;
//! * on the lifecycle edges, reaped a pre-rename (`_shepherd` / `_loop`)
//!   watchdog through an OWNERSHIP-CHECKED kill (ae:13219-13245).
//!
//! # Why the ownership check is the load-bearing part
//!
//! A tmux pane id is server-local and REUSED after a server restart on the same
//! socket. A process holding a stale id would kill a stranger — measured in the
//! integration suite, where a leftover ae process took a later section's freshly
//! spawned pane three seconds after it was created. So the facts are read from
//! the PANE ITSELF at the moment of the kill and the kill is authorised by a
//! POSITIVE match, never by the absence of a contradiction: a probe that fails
//! (server gone, tmux unavailable) and a probe that answers with an empty
//! session (tmux renders an empty format for an unknown target rather than
//! failing) BOTH refuse. A pane that is not there is nothing to kill, and an
//! unreadable pane is exactly the one this guard may not take on faith.
//!
//! # Where the argv lives
//!
//! The two tmux shapes this module needs — the pane-ownership probe and
//! `kill-pane` — are built HERE rather than in [`crate::tmux`], and
//! [`crate::transport`] holds only the two thin runners. That is a concession to
//! a concurrent-edit rule, not a design claim: they belong beside their
//! siblings in `tmux.rs` and should move there when that file is quiet.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::inventory::ServerId;
use crate::tmux::{self, OptionScope};
use crate::transport;

/// The DISPLAY branch segment the status bar renders — `' <branch><dirty>'`,
/// trimmed. Deliberately not the machine value: see [`tmux::BRANCH_OPTION`].
pub const BRANCH_STATUS_OPTION: &str = "@ae_branch_status";

/// The frozen display trim for a branch name (ae:13868).
pub const BRANCH_DISPLAY_MAX: usize = 24;

/// The pre-rename watchdog names a session can still carry, newest first.
///
/// A `doctor --refresh` rewrites helpers but does NOT restart a running
/// watchdog, so a legacy process can outlive the rename (ae:13279-13300).
pub const LEGACY_WATCHDOG_NAMES: [&str; 2] = ["shepherd", "loop"];

/// The daemon's pidfile, relative to the session's meta dir.
const PIDFILE_NAME: &str = ".watchdog.pid";

/// Flatten and trim `text` to `max` display characters, the frozen
/// `_watchdog_trim` (ae:13830-13839).
///
/// Newlines and carriage returns become spaces FIRST — a status bar renders one
/// line, and a value carrying a newline would break the option, not wrap it.
/// Over the cap the value keeps `max - 1` characters and gains a `~`, so the
/// result is never longer than `max`.
#[must_use]
pub fn trim_display(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    let kept: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}~")
}

/// The two branch facts the frozen segment publishes, from ONE git reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchReading {
    /// The FULL, untrimmed branch (or short sha on a detached HEAD), with no
    /// dirty marker — `@ae_branch_name`, the machine value `ae list --json`
    /// reads back.
    pub full: String,
    /// `' <branch><dirty>'` — the display segment, trimmed to
    /// [`BRANCH_DISPLAY_MAX`] and suffixed `*` when the work tree has tracked
    /// modifications. Leading space because it sits inside the `[ae …]` bracket.
    pub display: String,
}

/// Read the work tree's branch, or `None` when there is nothing to publish.
///
/// `None` for every frozen no-branch case (ae:13857-13875): no `work_dir`, a
/// path that is not inside a work tree, and a HEAD that names nothing. The
/// caller must then UNSET `@ae_branch_name` rather than publish an empty one —
/// see [`publish_branch`].
#[must_use]
pub fn branch_reading(work_dir: Option<&str>) -> Option<BranchReading> {
    let work_dir = work_dir.filter(|dir| !dir.is_empty())?;
    let full = crate::git::branch_head(work_dir.as_bytes())?;
    let branch = trim_display(&full, BRANCH_DISPLAY_MAX);
    if branch.is_empty() {
        return None;
    }
    let dirty = if crate::git::work_tree_dirty(work_dir.as_bytes()) {
        "*"
    } else {
        ""
    };
    Some(BranchReading {
        full,
        display: format!(" {branch}{dirty}"),
    })
}

/// Publish (or retract) the two branch options on `session_id`.
///
/// `None` publishes the display option as EMPTY and UNSETS the machine one,
/// exactly as the frozen segment does: the bar must collapse to just the path,
/// and a machine consumer must read "no branch", never a stale one.
pub fn publish_branch(server: &ServerId, session_id: &str, reading: Option<&BranchReading>) {
    let Some(reading) = reading else {
        let _ = transport::clear_option(
            server,
            OptionScope::Session,
            session_id,
            tmux::BRANCH_OPTION,
        );
        let _ = transport::publish_option(
            server,
            OptionScope::Session,
            session_id,
            BRANCH_STATUS_OPTION,
            "",
        );
        return;
    };
    let _ = transport::publish_option(
        server,
        OptionScope::Session,
        session_id,
        tmux::BRANCH_OPTION,
        &reading.full,
    );
    let _ = transport::publish_option(
        server,
        OptionScope::Session,
        session_id,
        BRANCH_STATUS_OPTION,
        &reading.display,
    );
}

/// Retract both branch options — the exit half of `_watchdog_clear_bar_options`
/// (ae:14010), which unsets rather than blanking so the bar falls back cleanly.
#[must_use]
pub fn clear_branch(server: &ServerId, session_id: &str) -> bool {
    let mut ok = true;
    for name in [BRANCH_STATUS_OPTION, tmux::BRANCH_OPTION] {
        ok &= transport::clear_option(server, OptionScope::Session, session_id, name);
    }
    ok
}

/// What a pane says about itself when asked who owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneOwner {
    /// `#{session_name}` — empty is IMPOSSIBLE here, because an empty reading
    /// is [`interpret_pane_owner`]'s refusal.
    pub session: String,
    /// `@ae_agent`, empty when the pane carries no stamp.
    pub agent: String,
}

/// `display-message -p -t <pane> '#{session_name}\t#{@ae_agent}'`.
#[must_use]
pub fn pane_owner_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = tmux::server_args(server);
    args.extend(
        [
            "display-message",
            "-p",
            "-t",
            pane,
            "#{session_name}\t#{@ae_agent}",
        ]
        .map(ToOwned::to_owned),
    );
    args
}

/// What a completed ownership probe means.
///
/// `None` — the refusal — for a FAILED run and for a successful run whose
/// session field is empty. Measured 2026-09-03: an unknown pane answers rc 0
/// with `"\t"`, and an unknown server answers rc 1. Both are "no owner named",
/// and a kill may not be authorised by either.
#[must_use]
pub fn interpret_pane_owner(succeeded: bool, stdout: &str) -> Option<PaneOwner> {
    if !succeeded {
        return None;
    }
    let line = stdout.lines().next().unwrap_or_default();
    // The frozen `${have%%\t*}` / `${have#*\t}` pair: with no tab BOTH expand to
    // the whole string, so a tabless reading is kept faithfully rather than
    // reinterpreted here.
    let (session, agent) = line.split_once('\t').unwrap_or((line, line));
    if session.is_empty() {
        return None;
    }
    Some(PaneOwner {
        session: session.to_owned(),
        agent: agent.to_owned(),
    })
}

/// `kill-pane -t <pane>`.
#[must_use]
pub fn kill_pane_args(server: &ServerId, pane: &str) -> Vec<String> {
    let mut args = tmux::server_args(server);
    args.extend(["kill-pane", "-t", pane].map(ToOwned::to_owned));
    args
}

/// What [`kill_owned_pane`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillOutcome {
    /// Nothing to do: an empty pane id.
    Nothing,
    /// The probe named no owner. Refused, silently — a pane that is not there
    /// is not a stranger-kill worth a diagnostic.
    Unreadable,
    /// The pane belongs to a different session. Refused, loudly.
    WrongSession(String),
    /// The pane carries a different `@ae_agent` stamp. Refused, loudly.
    WrongAgent(String),
    /// A positive match on every fact the caller named; `kill-pane` was run.
    Killed,
}

/// Kill `pane` ONLY if it is still the pane the caller means.
///
/// A refusal is LOUD but non-fatal: the caller's own status stands, which is why
/// nothing here is an error. See the module docs for why the check is positive.
///
/// # Errors
///
/// Only writing the refusal diagnostic to `err`.
pub fn kill_owned_pane(
    server: &ServerId,
    pane: &str,
    want_session: &str,
    want_agent: Option<&str>,
    err: &mut impl Write,
) -> crate::Result<KillOutcome> {
    if pane.is_empty() {
        return Ok(KillOutcome::Nothing);
    }
    let Some(have) = transport::observe_pane_owner(server, pane) else {
        return Ok(KillOutcome::Unreadable);
    };
    if want_session.is_empty() || have.session != want_session {
        let wanted = if want_session.is_empty() {
            "<unknown>"
        } else {
            want_session
        };
        writeln!(
            err,
            "ae: refusing to kill pane {pane}: it belongs to session '{}', not '{wanted}' \
             (stale pane id).",
            have.session
        )?;
        return Ok(KillOutcome::WrongSession(have.session));
    }
    if let Some(want_agent) = want_agent.filter(|name| !name.is_empty())
        && have.agent != want_agent
    {
        let stamped = if have.agent.is_empty() {
            "<unstamped>"
        } else {
            have.agent.as_str()
        };
        writeln!(
            err,
            "ae: refusing to kill pane {pane}: it is stamped '{stamped}', not '{want_agent}' \
             (stale pane id).",
        )?;
        return Ok(KillOutcome::WrongAgent(have.agent));
    }
    let _ = transport::kill_pane(server, pane);
    Ok(KillOutcome::Killed)
}

/// Reap any pre-rename watchdog still running under legacy artifacts.
///
/// Idempotent and quiet. For each legacy name: if its `_<name>` pane is live in
/// this session, kill it through [`kill_owned_pane`]; either way remove the
/// `.<name>.pid` / `.<name>.status` artifacts. Returns the names whose pane was
/// present, so a caller can report "stopped" for a session that ran only a
/// legacy watchdog (ae:13304-13316).
///
/// # Errors
///
/// Only writing a refusal diagnostic to `err`.
pub fn reap_legacy(
    server: &ServerId,
    session: &str,
    meta_dir: &Path,
    err: &mut impl Write,
) -> crate::Result<Vec<&'static str>> {
    // ONE enumeration for both names: two `list-panes` runs could disagree, and
    // an enumeration that FAILED is not evidence that anything is gone.
    let observed = transport::observe_agents(server, session).unwrap_or_default();
    let mut found = Vec::new();
    for name in LEGACY_WATCHDOG_NAMES {
        let stamp = format!("_{name}");
        let pane = observed
            .iter()
            .find(|seen| seen.agent == stamp)
            .map(|seen| seen.pane.clone());
        if let Some(pane) = pane {
            found.push(name);
            // The recorded pid is deliberately NOT signalled here: the legacy
            // daemon IS the process in that pane, so the ownership-checked
            // `kill-pane` takes it with the pane, and a bare kill of a recorded
            // pid is the stranger-kill this module exists to refuse.
            kill_owned_pane(server, &pane, session, Some(&stamp), err)?;
        }
        let _ = std::fs::remove_file(meta_dir.join(format!(".{name}.pid")));
        let _ = std::fs::remove_file(meta_dir.join(format!(".{name}.status")));
    }
    Ok(found)
}

/// The daemon's published pid, and the ownership rule for taking it back.
#[derive(Debug, Clone)]
pub struct PidFile {
    path: PathBuf,
    pid: u32,
}

impl PidFile {
    /// Publish this process's pid ATOMICALLY (temp + rename).
    ///
    /// `echo >file` creates the file empty and only THEN writes, so a reader can
    /// observe a zero-byte pidfile mid-write; a serialized starter that saw it
    /// would read no pid, delete the file and spawn a duplicate. `rename(2)`
    /// makes the pidfile appear only fully written, which is what the start
    /// path's registration wait depends on (ae:14346-14356).
    ///
    /// # Errors
    ///
    /// The write or the rename failing — the caller reports and keeps watching,
    /// because a watchdog that exits over its own bookkeeping stops watching a
    /// live session.
    pub fn publish(meta_dir: &Path) -> std::io::Result<Self> {
        let pid = std::process::id();
        let path = meta_dir.join(PIDFILE_NAME);
        let staged = meta_dir.join(format!("{PIDFILE_NAME}.tmp.{pid}"));
        std::fs::write(&staged, format!("{pid}\n"))?;
        std::fs::rename(&staged, &path)?;
        Ok(Self { path, pid })
    }

    /// The published path, for a caller that wants to name it in a diagnostic.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the pidfile, but ONLY while it still names this process.
    ///
    /// A stop/start in quick succession can leave the old daemon's cleanup
    /// running after the new one is already publishing; unconditional removal
    /// would then delete the REPLACEMENT's registration — the dying process
    /// vandalising its successor (ae:13996-14000). Returns whether it was ours
    /// to remove.
    #[must_use]
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the pidfile read that decides whether this daemon still owns its own \
                  registration — see clippy.toml"
    )]
    pub fn release(&self) -> bool {
        let owner = std::fs::read_to_string(&self.path).unwrap_or_default();
        if owner.trim() != self.pid.to_string() {
            return false;
        }
        std::fs::remove_file(&self.path).is_ok()
    }
}

/// RAII: the registration dies with the value that published it.
///
/// `run` used to release only after the loop returned, so any `?` between
/// publish and watch — the banner write hitting a closed pipe, a reap diagnostic
/// failing — left `.watchdog.pid` naming an exited process (colead gate
/// 135cf36a, deterministic repro). The release stays ownership-checked, so a
/// successor that already re-published is never touched, and a second release
/// is a no-op.
impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// The pane banner the frozen wrapper printed (ae:14365-14371).
#[must_use]
pub fn banner(session: &str, interval_secs: u64, stale_secs: u64, max_nudges: u32) -> String {
    let stale_min = stale_secs / 60;
    format!(
        "\u{1b}[1;36m\
         ╭─ ae watchdog — session: {session} ─────────────────────────╮\n\
         │ interval: {interval_secs}s   stale: {stale_min}m   max nudges: {max_nudges}\n\
         │ read-only pane (input disabled). use peek _watchdog or stop with `watchdog stop`.\n\
         ╰────────────────────────────────────────────────────────╯\n\
         \u{1b}[0m\n"
    )
}

/// One recovered tool session id — a seat that was `pending` and is not any
/// more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// The agent's display reference — the event's target.
    pub agent: String,
    /// The tool kind whose id was captured (`codex`, `gemini`, `opencode`).
    pub tool: String,
    /// The captured session id — the event's reference.
    pub captured: String,
}

/// One recovery pass over a session's roster: a single look at every seat whose
/// id is still pending, and the rows worth an event.
///
/// This is what `ae _recover-pending <session>` used to be. The frozen arm ran
/// the whole selection and every capture chain in bash and reported them as TSV
/// for this module to read back; the core owns both halves now, so the walk is
/// a call and the "row" is a value rather than a line that has to survive a
/// pipe. Only a seat that ACTUALLY landed an id is returned — the frozen
/// wrapper likewise acted on `ok` rows alone and left `already` / `miss` /
/// `skip` to doctor.
///
/// A look that finds nothing is not a failure: the seat stays pending and the
/// next cycle looks again, which is what made the frozen arm's
/// `2>/dev/null || true` the right reading of a failed run.
#[must_use]
pub fn recover(dir: &Path, roster: &[crate::meta::RosterEntry]) -> Vec<Recovered> {
    let mut rows = Vec::new();
    for seat in crate::session_launch::capture::pending_seats(roster) {
        let Some(captured) = crate::session_launch::capture::attempt(dir, &seat.slot) else {
            continue;
        };
        // The write goes through the roster the core owns, under its own meta
        // lock — the same call the launch's capture child makes, so a recovery
        // racing a late child rewrites one fact with itself.
        crate::session_launch::capture::register(dir, &seat.slot, &captured);
        rows.push(Recovered {
            agent: seat.agent,
            tool: seat.tool.as_str().to_owned(),
            captured,
        });
    }
    rows
}

/// The event summary a recovery is recorded with (ae:14440).
///
/// The reference is the full id; the summary quotes its first eight characters,
/// which is the form every other ae surface abbreviates an id with.
#[must_use]
pub fn recovered_summary(row: &Recovered) -> String {
    let short: String = row.captured.chars().take(8).collect();
    format!("captured {} session id ({short})", row.tool)
}

/// Whether a supervise tick is due.
///
/// `0` disables it outright, and a daemon that has never ticked is due
/// immediately — the frozen `last_tg_supervise=0` makes the first cycle's
/// `now - 0 >= TG_SUPERVISE_SECS` true (ae:14446-14451).
#[must_use]
pub fn supervise_due(every_secs: u64, last: Option<SystemTime>, now: SystemTime) -> bool {
    if every_secs == 0 {
        return false;
    }
    let Some(last) = last else {
        return true;
    };
    now.duration_since(last).unwrap_or(Duration::ZERO).as_secs() >= every_secs
}

/// The one deferred concern the cycle still owns: the Telegram bridge revive.
///
/// A value rather than a loose local because the supervise throttle is state
/// the tick owns, and a caller that forgot to carry it would revive the bridge
/// on every cycle. Pending-id recovery was the other half until the core grew
/// its own — see [`recover`], which needs none of this.
///
/// THE REVIVE IS IN-PROCESS. It used to run `<ae_path> telegram _supervise`
/// through the recorded glue, and that word had no parser on the other side:
/// [`crate::telegram_lifecycle`] accepts `start|stop|status` and nothing else,
/// so every throttle window spent a fork on a usage error whose exit status was
/// discarded. The core calls [`crate::telegram_lifecycle::autostart`] instead —
/// the same call a launch makes, which respects `[telegram] enabled`, never
/// writes that flag, takes the control lock non-blocking so a same-second
/// `stop` wins, and refuses on the tri-state UNKNOWN rather than spawning a
/// second bridge.
#[derive(Debug, Clone)]
pub struct Deferred {
    /// The ae home this session's state lives under, and the config the
    /// `[telegram]` section is read from — `None` when neither could be
    /// derived, in which case the tick is a no-op exactly as the frozen
    /// `[[ -x "${AE_PATH_BIN:-}" ]]` guard made it.
    paths: Option<crate::telegram::bridge::Paths>,
    /// This session's own directory — where a refusal's event mirror lands.
    /// Held rather than rebuilt from the home and the name, so a session whose
    /// directory and name ever disagree still records against the real one.
    dir: PathBuf,
    /// Seconds between supervise ticks; `0` disables.
    every_secs: u64,
    last: Option<SystemTime>,
}

impl Deferred {
    /// Build the tick from this session's own directory and its recorded config.
    ///
    /// The ae home is the sessions root's parent — `<ae-home>/sessions/<name>`
    /// is the one layout the core creates — and the config is meta's `config=`
    /// row, which is the file the launch resolved. An empty or missing row
    /// leaves [`Paths::under`]'s `<ae-home>/config` standing, which is what the
    /// glue's own `--config` default was.
    #[must_use]
    pub fn new(meta_dir: &Path, recorded_config: Option<&str>, every_secs: u64) -> Self {
        let paths = meta_dir.parent().and_then(Path::parent).map(|ae_home| {
            let mut paths = crate::telegram::bridge::Paths::under(ae_home);
            if let Some(config) = recorded_config.filter(|value| !value.is_empty()) {
                paths.config = PathBuf::from(config);
            }
            paths
        });
        Self {
            paths,
            dir: meta_dir.to_path_buf(),
            every_secs,
            last: None,
        }
    }

    /// Revive the Telegram bridge if the throttle allows it this cycle.
    ///
    /// Best-effort and idempotent: the autostart is a no-op with no network
    /// when Telegram is off. Returns whether a tick actually ran.
    ///
    /// `server` is the cycle's own — the one a rebind may just have adopted —
    /// so a revive reaches the session's server rather than an ambient one.
    /// That is what the `AE_TMUX_SERVER` export used to buy, without an
    /// environment the core may not set.
    ///
    /// THE DIAGNOSTIC IS DISCARDED, and deliberately: this runs in the
    /// watchdog's read-only pane, where a line printed every throttle window
    /// would bury the roster the pane exists to show. The frozen leg dropped
    /// the supervise's stderr for that reason and so does this. Nothing is
    /// lost — a refusal writes the durable `autostart-refusal` record that
    /// `telegram status` and `doctor` already display.
    pub fn supervise(
        &mut self,
        server: &crate::inventory::ServerId,
        session: &str,
        now: SystemTime,
    ) -> bool {
        let Some(paths) = self.paths.as_ref() else {
            return false;
        };
        if !supervise_due(self.every_secs, self.last, now) {
            return false;
        }
        self.last = Some(now);
        let _ = crate::telegram_lifecycle::autostart(
            paths,
            server,
            session,
            &self.dir,
            &mut std::io::sink(),
        );
        true
    }
}

/// The daemon's pidfile path under `meta_dir` — `.watchdog.pid`.
///
/// Published here rather than spelled again at the lifecycle's call sites: the
/// name is one fact, and a start that polled a different filename from the one
/// the daemon publishes would wait out its bound every time.
#[must_use]
pub fn pidfile(meta_dir: &Path) -> PathBuf {
    meta_dir.join(PIDFILE_NAME)
}

/// The pid a session's pidfile names, or `None` when it names nothing usable.
///
/// The read the LIFECYCLE needs, beside [`PidFile`]'s ownership-checked release:
/// `status` reports the pid, `start` refuses to duplicate a daemon that already
/// published one, and `stop` removes only the registration it observed. A
/// zero-byte or non-numeric file is `None` — the publish is atomic, so a partial
/// read means the file is not a pidfile.
#[must_use]
#[allow(
    clippy::disallowed_methods,
    reason = "a door: the pidfile read the start/stop/status decisions rest on — see clippy.toml"
)]
pub fn read_pid(meta_dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(pidfile(meta_dir)).ok()?;
    text.trim().parse::<u32>().ok()
}

/// Remove the pidfile, but ONLY while it still names `pid`.
///
/// [`PidFile::release`]'s rule from the OTHER side: the stopper reads a pid,
/// kills the pane it belongs to and then retracts the registration — and
/// between those steps a replacement may already have published its own. The
/// ownership check is what keeps a stop from vandalising the next daemon's
/// pidfile, exactly as it keeps a dying daemon from vandalising its successor's.
/// Returns whether the file was removed.
#[must_use]
pub fn clear_pid(meta_dir: &Path, pid: u32) -> bool {
    if read_pid(meta_dir) != Some(pid) {
        return false;
    }
    std::fs::remove_file(pidfile(meta_dir)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{
        BRANCH_DISPLAY_MAX, Deferred, PaneOwner, Recovered, interpret_pane_owner, kill_pane_args,
        pane_owner_args, recovered_summary, supervise_due, trim_display,
    };
    use crate::inventory::ServerId;
    use crate::meta::Selector;
    use std::time::{Duration, SystemTime};

    fn named(name: &str) -> ServerId {
        ServerId::Selected(Selector::Name(name.to_owned()))
    }

    #[test]
    fn a_branch_at_the_cap_is_kept_whole_and_one_over_it_is_marked() {
        let exact = "a".repeat(BRANCH_DISPLAY_MAX);
        assert_eq!(trim_display(&exact, BRANCH_DISPLAY_MAX), exact);
        let over = "b".repeat(BRANCH_DISPLAY_MAX + 1);
        let trimmed = trim_display(&over, BRANCH_DISPLAY_MAX);
        assert_eq!(trimmed.chars().count(), BRANCH_DISPLAY_MAX);
        assert!(trimmed.ends_with('~'), "the trim marker: {trimmed}");
    }

    #[test]
    fn a_newline_in_a_branch_never_reaches_the_option() {
        // A status option carrying a newline breaks the bar rather than
        // wrapping it, so the flatten happens BEFORE the cap.
        assert_eq!(trim_display("we\nird", 24), "we ird");
        assert_eq!(trim_display("we\r\nird", 24), "we  ird");
    }

    #[test]
    fn the_ownership_probe_refuses_every_reading_that_names_no_owner() {
        // Measured 2026-09-03: an unknown pane answers rc 0 with "\t", an
        // unknown server rc 1. Both must refuse — a kill authorised by silence
        // is the stranger-kill this guard exists to prevent.
        assert_eq!(interpret_pane_owner(false, "demo\t_watchdog\n"), None);
        assert_eq!(interpret_pane_owner(true, "\t"), None);
        assert_eq!(interpret_pane_owner(true, ""), None);
        assert_eq!(
            interpret_pane_owner(true, "demo\t_watchdog\n"),
            Some(PaneOwner {
                session: "demo".to_owned(),
                agent: "_watchdog".to_owned(),
            })
        );
        assert_eq!(
            interpret_pane_owner(true, "demo\t\n"),
            Some(PaneOwner {
                session: "demo".to_owned(),
                agent: String::new(),
            }),
            "an unstamped pane still names its session"
        );
    }

    #[test]
    fn the_probe_and_the_kill_address_the_recorded_server() {
        let server = named("work");
        assert_eq!(
            pane_owner_args(&server, "%3"),
            vec![
                "-L".to_owned(),
                "work".to_owned(),
                "display-message".to_owned(),
                "-p".to_owned(),
                "-t".to_owned(),
                "%3".to_owned(),
                "#{session_name}\t#{@ae_agent}".to_owned(),
            ]
        );
        assert_eq!(
            kill_pane_args(&server, "%3"),
            vec![
                "-L".to_owned(),
                "work".to_owned(),
                "kill-pane".to_owned(),
                "-t".to_owned(),
                "%3".to_owned(),
            ]
        );
    }

    #[test]
    fn a_recovery_is_summarised_by_the_tool_and_the_head_of_its_id() {
        let row = Recovered {
            agent: "w3".to_owned(),
            tool: "codex".to_owned(),
            captured: "0191aaaa-bbbb".to_owned(),
        };
        assert_eq!(
            recovered_summary(&row),
            "captured codex session id (0191aaaa)"
        );
        // An id shorter than the abbreviation is quoted whole rather than
        // padded — the frozen `${id:0:8}` on a short value.
        let short = Recovered {
            captured: "abc".to_owned(),
            ..row
        };
        assert_eq!(recovered_summary(&short), "captured codex session id (abc)");
    }

    #[test]
    fn the_supervise_throttle_fires_first_and_then_waits() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert!(
            supervise_due(120, None, now),
            "the frozen last=0 makes the first cycle due"
        );
        assert!(!supervise_due(0, None, now), "zero disables it outright");
        assert!(!supervise_due(120, Some(now), now));
        assert!(!supervise_due(
            120,
            Some(now),
            now + Duration::from_secs(119)
        ));
        assert!(supervise_due(120, Some(now), now + Duration::from_mins(2)));
    }

    #[test]
    fn a_session_directory_with_no_derivable_home_supervises_nothing() {
        // The frozen `[[ -x "${AE_PATH_BIN:-}" ]]` guard, in its new subject: a
        // meta directory with no grandparent names no ae home, so there is no
        // config to read intent from — and the throttle must not record a tick
        // that never ran, or the first real one would be delayed by a no-op.
        let mut deferred = Deferred::new(std::path::Path::new("meta"), None, 120);
        let server = named("nothing");
        assert!(!deferred.supervise(&server, "demo", SystemTime::UNIX_EPOCH));
        assert!(!deferred.supervise(&server, "demo", SystemTime::UNIX_EPOCH));
    }

    #[test]
    fn the_recorded_config_row_overrides_the_ae_home_default() {
        // `<ae-home>/sessions/<name>` is the one layout the core creates, so the
        // home is two parents up; the config is meta's own row when it has one.
        let dir = std::path::Path::new("/srv/.ae/sessions/demo");
        let default = Deferred::new(dir, None, 120);
        assert_eq!(
            default.paths.as_ref().map(|paths| paths.config.clone()),
            Some("/srv/.ae/config".into())
        );
        let recorded = Deferred::new(dir, Some("/etc/ae.conf"), 120);
        assert_eq!(
            recorded.paths.as_ref().map(|paths| paths.config.clone()),
            Some("/etc/ae.conf".into())
        );
        // An EMPTY row is not a config: it leaves the derived default standing.
        let empty = Deferred::new(dir, Some(""), 120);
        assert_eq!(
            empty.paths.as_ref().map(|paths| paths.config.clone()),
            Some("/srv/.ae/config".into())
        );
    }
}
