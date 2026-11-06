//! `ae reseat <session> <agent> --using <profile[@client]> [--stop-unknown]`:
//! move ONE seat to another profile, in place — or, with `profile@client`, to a
//! profile on another account of the same harness, for a pair the human's
//! GLOBAL config approves under `[client_pairs]`.
//!
//! The pain, measured across this fleet: a seat whose vendor quota dies is lost
//! WITH ITS CONTEXT. The only way on was a fresh spawn under a new name plus a
//! handover written by hand, and the seat's slot, pane, name and history stayed
//! bound to a tool that can no longer answer.
//!
//! This moves the SAME seat: same slot, same pane, same name, same records. The
//! successor arrives on another tool and is handed ae's own account of the seat
//! — the seed pack `ae brief --seat` renders, the predecessor's last turns
//! included — as its first turn.
//!
//! IT DOES THE FULL ROUND. A seat whose tool is still running is STOPPED IN
//! PLACE first — same pane id, same pane options, same scrollback, the shell
//! left idle in the session's recorded working copy — and only then moved. A
//! seat that is already gone takes exactly the path it took before.
//!
//! The stop is never silent and never guessed: [`stop_running_tool`] owns its
//! rule. Past it the seat must still pass
//! [`crate::seat_relaunch::prove_dead`] — the same owner, asked with this
//! verb's word, so the two commands refuse for the same reasons in the same
//! order.
//!
//! The order is the contract, and everything before the stop is undone by
//! doing nothing:
//!
//! 1. the argv, the session, the caller, the seat and the profile resolve —
//!    a pair through the spawn's own approval and adapter gate
//!    ([`crate::session_launch::approved_pair`]) — and no state is written;
//! 2. under the session's lifecycle lock, a running tool is stopped and the
//!    stop is recorded; then the seat is proven dead;
//! 3. the seed pack is built (it reads the seat's recorded first message,
//!    which step 4 removes) and published as `seed.<agent>.md`;
//! 4. every per-slot launch file goes, so the successor's `_run` has no stale
//!    first message, no stale sid file and no start marker to resume from;
//! 5. the meta rows move in ONE guarded replacement;
//! 6. the pane line is pasted and the new tool observed;
//! 7. past the lock: the tool's own launch turn, then the seed.
//!
//! Three crash windows, all named and all recoverable by hand: between the
//! stop and 4 the seat still records the OLD profile and its pane is at a
//! shell, so `relaunch` brings it back on the tool it had; between 4 and 5 the
//! same is true with the launch files gone; between 5 and 6 the meta names the
//! NEW profile and the pane sits at its shell, which is exactly what
//! `relaunch` finishes.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::autoreseat::{CauseClass, Fate, LongLimitGap};
use crate::harness_state::HarnessState;
use crate::inventory::ServerId;
use crate::procs::{self, Descendancy};
use crate::seat_relaunch::{Proven, RESEAT_VERB, Started, Target};
use crate::session_tmux::Op;
use crate::state::{EXIT_FAILED, EXIT_USAGE};
use crate::time::Timestamp;
use crate::tmux::ObservedPaneProbe;
use crate::tool::ToolKind;
use crate::tracked::{self, EventFields};
use crate::transport;

#[cfg(test)]
#[path = "reseat/agylimit_reason2_spec.rs"]
mod agylimit_reason2_spec;
#[cfg(test)]
#[path = "reseat/agylimit_reason_spec.rs"]
mod agylimit_reason_spec;
#[cfg(test)]
#[path = "reseat/agylimit_spec.rs"]
mod agylimit_spec;
#[cfg(test)]
#[path = "reseat/limitmuse_spec.rs"]
mod limitmuse_spec;
#[cfg(test)]
#[path = "reseat/long_limit_tests.rs"]
mod long_limit_tests;
#[cfg(test)]
#[path = "reseat/muse_gap_tests.rs"]
mod muse_gap_tests;

/// The usage line.
pub const RESEAT_USAGE: &str =
    "Usage: ae reseat <session> <agent> --using <profile[@client]> [--stop-unknown]";

/// The event action a reseat records, whatever its outcome.
pub(crate) const RESEAT_ACTION: &str = "reseat";

/// The outcome words of every record a reseat writes AFTER the seat's meta
/// moved: the move whose tool started, and the two whose tool did not. The
/// stop record is written before the move and begins with none of them.
const MOVED: &str = "reseated ";
const NOT_SEEN: &str = "pasted, tool not seen";
const STOPPED_AFTER: &str = "tool stopped after the reseat";

/// Whether a `reseat` record's summary says the seat now records another
/// profile.
#[must_use]
pub(crate) fn moved_seat(summary: &str) -> bool {
    [MOVED, NOT_SEEN, STOPPED_AFTER]
        .iter()
        .any(|outcome| summary.starts_with(outcome))
}

/// How long the stop waits for the pane to come back to an idle shell.
const STOP_POLLS: u32 = 50;

/// The pause between those polls — 50 x 200ms is the 10s bound the docs state.
const STOP_POLL: Duration = Duration::from_millis(200);

// The gap between the two frame readings the stop takes, shared with the
// trust answer. ONE reading is not enough: between a tool receiving Enter and
// drawing its spinner the box is empty and no turn is drawn yet, and a single
// frame in that gap reads IDLE for a seat that has just been given work.
use crate::deliver::FRAME_GAP;

/// How long the stop waits for the pane's send-lock.
///
/// Short, and its own refusal, because this is asked while the session's
/// lifecycle lock is already held: a delivery in flight is a reason to come
/// back, never a reason to hold a session out of its own lifecycle.
const SEND_LOCK_WAIT: Duration = Duration::from_secs(5);

/// The argv, validated.
struct Parsed {
    session: String,
    agent: String,
    /// The BARE profile the seat moves to — never carries `@`.
    profile: String,
    /// The `[clients]` label of a `--using profile@client` move: the account
    /// the profile runs on instead of its own. `None` is a bare move, which
    /// leaves the seat on the profile's own account.
    client: Option<String>,
    stop: StopPolicy,
    /// The watchdog's move, which must not drop an observed choice (#269). A
    /// human's `--using` is a new choice and clears it.
    automatic: bool,
    /// A limit episode's move, which may end the seat's background tasks on a
    /// long limit (#304); `None` for every other stop.
    long_limit: Option<LongLimit>,
    /// A return's move: what the seat must still be under the lifecycle lock
    /// for it to go home (#305); `None` for every other move.
    expect: Option<crate::autoreseat::leg::Expect>,
    /// The titles of the background tasks the stop ended, for the caller.
    ended: std::cell::RefCell<Vec<String>>,
}

/// A limit episode's automatic move, as its stop reads it to end a seat's
/// background tasks (#304).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongLimit {
    /// The episode's key, epoch seconds.
    pub(crate) key: i64,
    /// When the attempt making the move opened, epoch seconds: a client's
    /// input on the pane since then was no part of the judgement that opened it.
    pub(crate) opened: i64,
}

/// Which frames the stop may end a running tool on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopPolicy {
    /// A frame proven idle, or proven on the vendor's usage limit over an
    /// empty box, on both readings.
    Proven,
    /// `--stop-unknown`: a frame ae cannot read too. It lifts the UNKNOWN
    /// refusal and nothing else — a frame that reads BUSY is refused with or
    /// without it.
    Unknown,
    /// As `Proven`, and [`crate::autoreseat::quiet_pair`] over the two readings
    /// proves the stop too. The auto reseat leg's own route for a resume's
    /// urgent seat: no flag reaches it.
    QuietStart,
}

/// Parse `<session> <agent> --using <profile>`.
///
/// The flag may sit anywhere, as `spawn`'s does, because that is where a hand
/// puts it. Anything else beginning with `-` is refused rather than swallowed
/// as a name.
fn parse(tail: &[String]) -> Result<Parsed, String> {
    let mut names: Vec<&str> = Vec::new();
    let mut profile = String::new();
    let mut stop = StopPolicy::Proven;
    let mut words = tail.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            // Anywhere, like `--using`, because that is where a hand puts it.
            "--stop-unknown" => stop = StopPolicy::Unknown,
            "--using" => {
                let Some(value) = words.next() else {
                    return Err("Error: --using needs a profile name.".to_owned());
                };
                profile.clone_from(value);
            }
            flag if flag.starts_with("--using=") => {
                flag["--using=".len()..].clone_into(&mut profile);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Error: unknown flag '{flag}'."));
            }
            name => names.push(name),
        }
    }
    let [session, agent] = names.as_slice() else {
        return Err(RESEAT_USAGE.to_owned());
    };
    if profile.is_empty() {
        return Err("Error: reseat needs --using <profile>.".to_owned());
    }
    // The one grammar a launch `--seat` selection and a spawn use: one `@`
    // splits the profile from its client, so every later row carries the bare
    // profile.
    let Some((profile, client)) = crate::session_launch::split_profile_client(&profile) else {
        return Err(format!(
            "Error: --using '{profile}' is not <profile> or <profile>@<client> (e.g. --using opus55x@cc-mic)."
        ));
    };
    Ok(Parsed {
        session: (*session).to_owned(),
        agent: (*agent).to_owned(),
        profile,
        client,
        stop,
        automatic: false,
        long_limit: None,
        expect: None,
        ended: std::cell::RefCell::default(),
    })
}

/// What the carry question answered, and what the audit line says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Carried {
    /// Not a carry question at all — a tool change, a move inside one account,
    /// a seat with no conversation, or a tool whose store ae has not measured.
    /// SILENT: this move behaves exactly as it did before carrying existed, and
    /// says nothing new on either stream or in its record.
    No,
    /// The conversation's files are in the new account.
    Yes,
    /// They are, and the new account's own older or differing copies of them
    /// were ARCHIVED first: moved aside, never deleted, into one directory of
    /// this move that the record names.
    Archived { count: usize, dir: PathBuf },
    /// The carry was live and REFUSED, or a conversation that was to be kept
    /// could not be. The move still happens — a fresh conversation and the
    /// seed pack — and this is what it could not do.
    Seeded(String),
    /// The SAME tool in the SAME account resumes the conversation on another
    /// model or effort ([`keeps`]): nothing travelled, nothing was left.
    Kept,
}

impl Carried {
    /// The word the reseat record carries, empty when there was no question.
    fn word(&self) -> String {
        match self {
            Self::No => String::new(),
            Self::Archived { count, dir } => format!(
                "{}, archived {} in {}",
                Fate::Carried.word(),
                crate::carry::files(*count),
                dir.display()
            ),
            Self::Seeded(why) => format!("{} ({why})", Fate::Seeded.word()),
            Self::Yes | Self::Kept => self.fate().word().to_owned(),
        }
    }

    /// What became of the conversation, in the one vocabulary a caller of the
    /// verb journals it in.
    const fn fate(&self) -> Fate {
        match self {
            Self::Yes | Self::Archived { .. } => Fate::Carried,
            Self::Kept => Fate::Kept,
            Self::No | Self::Seeded(_) => Fate::Seeded,
        }
    }

    /// The word the record files the old account's conversation id under. A
    /// CARRIED conversation is not left behind — it is the SAME one, continuing
    /// in the other account — so calling it `prior` would make a durable record
    /// say the opposite of what happened. Every other arm really did leave one.
    fn field(&self) -> &'static str {
        match self {
            Self::Yes | Self::Archived { .. } | Self::Kept => "conversation",
            Self::No | Self::Seeded(_) => "prior",
        }
    }

    /// Does the seat go on in the conversation it had? Archiving on the way
    /// changes what the record says, never what the move does.
    fn travelled(&self) -> bool {
        self.fate() != Fate::Seeded
    }
}

/// Does a record name the seat's conversation at all?
///
/// A STOP does not. Which binary was ended in which pane, and which move it
/// belongs to, is the whole of that event: a conversation id is not a fact
/// about stopping a tool. It matters most where the move is about to CARRY,
/// because the stop is written before the verdict — any word it chose for that
/// conversation would be a guess, and `prior` would be the wrong one, naming a
/// conversation that is about to continue in another account as abandoned.
/// Deferring the record instead is not open: it must be durable at the moment
/// the pane is proven back at its shell.
#[derive(Debug, Clone, Copy)]
enum Mentions {
    Conversation,
    NothingOfIt,
}

/// Does this move's conversation travel? `None` is every silent arm.
///
/// Reads the seat's recorded account rather than re-resolving the profile it is
/// leaving: for a retained conversation the RECORD is what names the store, and
/// re-deriving it here could send the copy looking in a home this seat never
/// used.
fn carry_plan(
    locked: &[u8],
    slot: &str,
    moving: &Moving,
    id: &str,
    work_dir: &str,
) -> Option<crate::carry::Plan> {
    let recorded = crate::meta::Meta::parse(&String::from_utf8_lossy(locked))
        .roster()
        .iter()
        .find(|row| row.slot == slot)
        .map(|row| row.config_home.clone())?;
    let (crate::meta::RecordedConfigHome::Path(from)
    | crate::meta::RecordedConfigHome::Implicit(from)) = recorded
    else {
        return None;
    };
    let account = moving.account.as_ref()?;
    let from_binary = crate::lifecycle::meta_value(locked, &format!("agent_bin.{slot}"));
    crate::carry::plan(&crate::carry::Move {
        from_binary: &from_binary,
        to_binary: &moving.binary,
        tool: moving.tool,
        id,
        work_dir: Path::new(work_dir),
        from: Some(&from),
        to: Some(&account.path),
    })
}

/// One KEEP question's inputs: the seat's recorded rows (raw, empty when
/// absent) and its recorded profile resolved now, beside the new profile's.
struct Keep<'a> {
    from_binary: &'a str,
    to_binary: &'a str,
    tool: ToolKind,
    id: &'a str,
    from_home: &'a str,
    from_base: &'a str,
    to_home: Option<(&'a str, Option<&'a str>)>,
    from_command: Option<&'a str>,
    to_command: &'a str,
}

/// Does this move KEEP the seat's conversation? PURE. The same binary and
/// recorded account (path, mode and implicit base), an id the tool's resume
/// takes, and commands equal under [`crate::launch_cmd::keep_commands`].
fn keeps(keep: &Keep<'_>) -> bool {
    use crate::meta::RecordedConfigHome;
    let Some((to_home, to_base)) = keep.to_home else {
        return false;
    };
    let recorded = RecordedConfigHome::parse(keep.from_home);
    let base_matches = match recorded {
        RecordedConfigHome::Path(_) => keep.from_base.is_empty() && to_base.is_none(),
        RecordedConfigHome::Implicit(_) => {
            !keep.from_base.is_empty() && to_base == Some(keep.from_base)
        }
        _ => false,
    };
    !keep.from_binary.is_empty()
        && keep.from_binary == keep.to_binary
        && crate::launch::id_probeable(keep.id)
        && base_matches
        && recorded.record_value().as_deref() == Some(to_home)
        && keep
            .from_command
            .is_some_and(|from| crate::launch_cmd::keep_commands(from, keep.to_command, keep.tool))
}

/// PURE: does a move to the seat's OWN profile ask for the client the seat
/// already records — the "nothing to move" half beside the profile compare?
///
/// A bare request asks for the profile's own account: that is where a seat
/// recording no client already is, and where a seat whose client row ae
/// cannot read is refused exactly as before. Only a seat recording a USABLE
/// label is somewhere else, so a bare move returns it home. A pair asks for
/// exactly its label.
fn same_client(asked: Option<&str>, recorded: &crate::meta::RecordedClient) -> bool {
    use crate::meta::RecordedClient;
    match (asked, recorded) {
        (None, RecordedClient::Label(_))
        | (Some(_), RecordedClient::Missing | RecordedClient::Invalid) => false,
        (None, RecordedClient::Missing | RecordedClient::Invalid) => true,
        (Some(asked), RecordedClient::Label(label)) => asked == label,
    }
}

/// The command the seat's recorded profile resolves to NOW, exactly as `_run`
/// resolves it — a recorded client override included — or `None`.
fn recorded_command(dir: &Path, bytes: &[u8], seated: &crate::meta::RosterEntry) -> Option<String> {
    let cfg = identity_config(dir, bytes).ok()?;
    crate::run::read_seat_command(
        &cfg,
        seated.profile.as_deref()?,
        &seated.name,
        &seated.slot,
        &seated.client,
        crate::doors::home().as_deref(),
    )
    .ok()
    .map(|command| command.as_str().to_owned())
}

/// The model and effort each profile sets, for the line naming a kept move.
fn kept_change(from: &str, to: &str, tool: ToolKind) -> String {
    let crate::tool::KeepSpec::Measured { model_flags, .. } = tool.adapter().resume.keep else {
        return String::new();
    };
    let model = |cmd| crate::launch_cmd::sole_flag_value(cmd, model_flags).unwrap_or_default();
    let effort = |cmd| crate::launch_cmd::effort_flag(cmd, tool).map_or("default", |l| l.word());
    let (was, now) = ((model(from), effort(from)), (model(to), effort(to)));
    format!(
        " (model {} -> {}, effort {} -> {})",
        was.0, now.0, was.1, now.1
    )
}

/// Where this seat's seed is published — beside the meta, 0600, and KEPT after
/// a successful move: it is what a human re-sends by hand when the turn did not
/// land, and what tells a later reader what the successor was told.
///
/// The name goes through the same sanitiser a slot does, for the same reason:
/// this is a FILENAME COMPONENT, and the agent it is built from is read back
/// out of a pane option and a hand-editable meta rather than re-validated
/// against the agent grammar here. ONE owner, so the path this publishes and
/// the path a refusal tells a human to `cat` cannot disagree.
fn seed_file(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("seed.{}.md", crate::launch::safe_slot(agent)))
}

/// Who ran this, read ONCE: the caller rule, the self-reseat refusal and the
/// event's actor are three questions about the same pane.
#[derive(Default)]
struct Caller {
    /// `$TMUX_PANE`, when this ran inside tmux at all.
    pane: Option<String>,
    /// `@ae_agent` — the display ref a record names. Empty for a plain shell,
    /// which the record renders as `human`, exactly as `relaunch` does.
    display: String,
}

/// THE CALLER RULE. `Err` is the refusal, `Ok` the caller.
///
/// A pane ae STAMPED is a seat, and a seat may reseat only inside its own
/// session — the boundary `relaunch` draws, for the same reason. A plain shell
/// carries no stamp and may reseat any session, because the human is who this
/// verb is for: the moment a lead's own quota dies, no agent of that session
/// can run anything.
fn caller_of(session: &str) -> Result<Caller, String> {
    let pane = crate::doors::calling_pane_id();
    let Some((viewer, _)) = crate::observe_calling_pane().require()? else {
        return Ok(Caller {
            pane,
            display: String::new(),
        });
    };
    let display = viewer.agent.clone().unwrap_or_default();
    let Some(slot) = viewer.slot.clone().filter(|slot| !slot.is_empty()) else {
        return Ok(Caller { pane, display });
    };
    match viewer.session.as_deref() {
        Some(name) if name == session => Ok(Caller { pane, display }),
        Some(name) => Err(format!(
            "Error: this pane is slot {slot} of '{name}', and reseat works on the caller's own \
             session only — run it from '{session}', or from a shell outside ae."
        )),
        None => Err(format!(
            "Error: this pane carries ae slot {slot} but tmux did not say which session it is in \
             — ae will not reseat '{session}' for a caller it cannot place."
        )),
    }
}

/// Resolve `agent` to a seat of `session`.
///
/// Deliberately NOT `relaunch`'s resolver: that one refuses every session but
/// the caller's, and this verb's caller may be a shell in none. The refusal
/// that matters here is [`caller_refusal`], already made.
fn resolve(dir: &Path, agent: &str, session: &str) -> Result<Target, String> {
    let (resolved, server): (tracked::Resolved, ServerId) =
        tracked::resolve_on(agent, session, dir)
            .map_err(|why| tracked::ResolveError::message(&why))?;
    if !resolved.session.is_empty() && resolved.session != session {
        return Err(format!(
            "Error: '{agent}' is on session '{}', not '{session}'.",
            resolved.session
        ));
    }
    Ok(Target {
        agent: if resolved.agent.is_empty() {
            agent.to_owned()
        } else {
            resolved.agent
        },
        slot: resolved.slot,
        pane: resolved.pane,
        server,
    })
}

/// The profiles a seat of the session at `dir` may be moved to: the global
/// file its meta records plus the origin's overlay — never the orchestrator
/// seat's own overlay, exactly as `read_seat` has it, because reading it here
/// would offer a profile the launch would then refuse.
///
/// # Errors
///
/// The identity config's own refusal.
pub(crate) fn identity_config(
    dir: &Path,
    bytes: &[u8],
) -> Result<crate::config::IdentityConfig, String> {
    let value = |key: &str| crate::lifecycle::meta_value(bytes, key);
    let global = value("config");
    let local = crate::config::local_overlay(dir, &value("origin"));
    let orchestrator_seat = local.as_deref().is_some_and(|path| {
        dir.parent()
            .and_then(Path::parent)
            .is_some_and(|home| crate::orchestrator::is_seat_overlay(path, home))
    });
    crate::config::read_identity(
        (!global.is_empty()).then(|| Path::new(&global)),
        (!orchestrator_seat).then_some(local.as_deref()).flatten(),
    )
    .map_err(|why| why.to_string())
}

/// The one-line warning for `profile` when it has no auto-reseat exit off its
/// account, from the same two files a move reads: the session's recorded global
/// config (its switch and map) and the layered identity (its profiles).
///
/// A `profile@client` move passes the command its pair resolved to, and the
/// verdict judges the account THAT command selects.
fn cover_warning(dir: &Path, bytes: &[u8], profile: &str, pair: Option<&str>) -> Option<String> {
    let cfg = identity_config(dir, bytes).ok()?;
    let global = crate::lifecycle::meta_value(bytes, "config");
    let settings = crate::autoreseat::settings((!global.is_empty()).then(|| Path::new(&global)));
    let home = crate::doors::home();
    match pair {
        None => crate::autoreseat::leg::cover_warning(&cfg, &settings, profile, home.as_deref()),
        Some(command) => crate::autoreseat::leg::cover_warning_through(
            &cfg,
            &settings,
            profile,
            &crate::config::IdentityConfig::resolved_snapshot(command),
            home.as_deref(),
        ),
    }
}

/// The new profile resolved to the two facts the meta records.
///
/// The SAME resolution `crate::run::read_seat` makes for a seat that records no
/// client override, so a profile this verb accepts is one `_run` can launch —
/// asked BEFORE anything is written, because a profile that does not lex is a
/// refusal and not a half-moved seat.
fn resolve_profile(dir: &Path, bytes: &[u8], profile: &str, agent: &str) -> Result<Moving, String> {
    let cfg = identity_config(dir, bytes)?;
    let home = crate::doors::home();
    let command = cfg
        .command(profile, home.as_deref())
        .map_err(|why| why.to_string())?;
    let Some(command) = command.filter(|command| !command.as_str().trim().is_empty()) else {
        return Err(format!(
            "profile '{profile}' is not configured on this machine — '{agent}' cannot be moved to it"
        ));
    };
    let parsed = crate::launch_cmd::lex_simple_command(command.as_str())
        .map_err(|why| format!("profile '{profile}' is not one simple command — {why}"))?;
    // NO tool-class judgement here, deliberately: a seat may be moved to
    // exactly the profiles a LAUNCH accepts, and the launch asks the adapter
    // rows for its behaviour rather than refusing a binary it does not know.
    Ok(Moving {
        account: account_of(&command, parsed.tool(), home.as_deref()),
        tool: parsed.tool(),
        pin: crate::launch_cmd::model_flag_value(command.as_str(), parsed.tool()),
        binary: parsed.binary,
        command: command.as_str().to_owned(),
        pair: None,
    })
}

/// A `profile@client` move resolved to the facts the meta records — or the
/// refusal, every one of them before anything is stopped.
///
/// The gate is the spawn's own ([`crate::session_launch::approved_pair`]):
/// the session's recorded GLOBAL config alone, approval before existence, the
/// same-known-adapter gate and the first-start store proof. Then the ACCOUNT:
/// a pair whose account ae cannot name is refused here, because the move
/// publishes that account with the client in one replacement and a client
/// without its account is no identity at all.
fn resolve_pair(bytes: &[u8], profile: &str, label: &str, agent: &str) -> Result<Moving, String> {
    let global = crate::lifecycle::meta_value(bytes, "config");
    let home = crate::doors::home();
    let (command, parsed) = crate::session_launch::approved_pair(
        (!global.is_empty()).then(|| Path::new(&global)),
        agent,
        profile,
        label,
        home.as_deref(),
        "Nothing was reseated.",
    )?;
    let Some(account) = account_of(&command, parsed.tool(), home.as_deref()) else {
        return Err(format!(
            "Error: --using '{profile}@{label}': ae cannot name the account '{label}' selects for \
             '{profile}', so the move cannot record it. Nothing was reseated."
        ));
    };
    Ok(Moving {
        account: Some(account),
        tool: parsed.tool(),
        pin: crate::launch_cmd::model_flag_value(command.as_str(), parsed.tool()),
        binary: parsed.binary,
        command: command.as_str().to_owned(),
        pair: Some(label.to_owned()),
    })
}

/// The new profile's recorded facts.
struct Moving {
    tool: ToolKind,
    /// The model its `--model` flag pins, when it pins one.
    pin: Option<String>,
    binary: String,
    /// The command it resolves to.
    command: String,
    /// Where this profile's conversations live, when ae can say.
    account: Option<Account>,
    /// The approved `[clients]` label of a `profile@client` move, whose
    /// account [`resolve_pair`] proved is `account`; `None` is a bare move.
    pair: Option<String>,
}

/// The account a profile selects, resolved once and kept whole: the path a
/// carry copies INTO, and the two rows that record it.
///
/// Held together because they are one identity — an implicit store and an
/// explicit one over the same path are different accounts, and a row published
/// without its base would say the wrong one.
pub(crate) struct Account {
    /// The canonical directory itself.
    pub(crate) path: PathBuf,
    /// `config_home.<slot>`, in [`crate::meta::RecordedConfigHome`]'s grammar.
    row: String,
    /// `config_home_base.<slot>` — the canonical effective `HOME` an implicit
    /// store was selected against, and `None` for an explicit one.
    base: Option<String>,
}

/// Resolve the account a profile's command would give its tool.
///
/// The lookup is CONTROLLED, not ambient — `HOME` is the launch home and every
/// other variable is unset — because this runs in the CALLER's process and the
/// caller's environment is not the pane's. It is the reading
/// `session_launch::refuse_store_conflict` takes for exactly the same reason,
/// and it maps to a recorded row by `run::config_home_identity`'s rule for a
/// seat with nothing recorded yet.
///
/// `None` means ae cannot name this profile's account: a `$VAR` only the pane
/// can see, no effective `HOME`, or a path that will not canonicalize. Nothing
/// is refused on it — a move whose account ae cannot name simply cannot carry.
pub(crate) fn account_of(
    command: &crate::config::ResolvedCommand,
    tool: ToolKind,
    home: Option<&Path>,
) -> Option<Account> {
    let home_value = home.map(|path| path.display().to_string());
    let resolution = crate::launch_cmd::config_home_resolution(command, tool, &|name| {
        (name == "HOME").then(|| home_value.clone()).flatten()
    });
    let crate::launch_cmd::Resolved::Path(path) =
        crate::run::canonical_config_home(&resolution.home).ok()?
    else {
        return None;
    };
    if resolution.explicit {
        return Some(Account {
            row: path.display().to_string(),
            path,
            base: None,
        });
    }
    // An IMPLICIT store is only an identity together with the HOME that
    // selected it, so a base ae cannot canonicalize is not half an answer.
    let crate::launch_cmd::Resolved::Path(base) =
        crate::run::canonical_config_home(&resolution.base).ok()?
    else {
        return None;
    };
    Some(Account {
        row: format!("implicit:{}", path.display()),
        path,
        base: Some(base.display().to_string()),
    })
}

/// How one reseat ended, for a caller that must journal it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The seat runs its new profile, and this is what became of its
    /// conversation.
    Moved { fate: Fate },
    /// Nothing of the seat changed, for the named cause, decided where it was
    /// refused. A transient one ([`CauseClass::transient`]) may pass on a retry.
    Refused(CauseClass),
    /// Something was stopped or written and the move did not finish, or the
    /// machinery itself refused.
    Failed,
}

/// The class a carry refusal is journaled under: the stage that refused it
/// named the [`crate::carry::Kind`], and this only spells it.
const fn carry_class(kind: crate::carry::Kind) -> CauseClass {
    match kind {
        crate::carry::Kind::SourceMissing => CauseClass::CarrySourceMissing,
        crate::carry::Kind::TargetDiffers => CauseClass::CarryTargetDiffers,
        crate::carry::Kind::Unreadable => CauseClass::CarryUnreadable,
    }
}

/// What the stop step concluded.
enum Stop {
    /// The seat's tool was not running. NOTHING was read beyond the pane, and
    /// every refusal a dead or unreadable seat deserves belongs to
    /// [`crate::seat_relaunch::prove_dead`], which runs next — this arm is how
    /// an already-dead seat stays byte-identical to what it was before the
    /// stop existed.
    NotRunning,
    /// The tool was stopped and the pane is back at an idle shell, carrying
    /// the binary that was stopped for the record.
    Stopped(String),
    /// A refusal was printed and nothing was killed. A transient cause — a
    /// delivery in flight or a running turn — may pass on a retry.
    Refused(CauseClass),
    /// A refusal was printed and the machinery is what refused: tmux would not
    /// respawn the pane, or the pane was not back at a shell after the kill.
    Failed,
}

/// Where the caller stands relative to the seat whose tool is about to be
/// ended — PURE, from the readings the stop already took.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CallerStanding {
    /// The caller's process tree does not place it under the seat's pane.
    Clear,
    /// The caller runs BENEATH that pane: ending the tool would kill it.
    Under,
    /// A reading is missing, so the two cannot be told apart — named.
    Unprovable(String),
}

/// FAIL-CLOSED BY CONSTRUCTION: every gap — no pid, no table, no row for the
/// pane or for this command, an ancestry broken before either — is
/// [`CallerStanding::Unprovable`]. `Clear` is the only answer a stop acts on.
fn caller_standing(
    pane_pid: Option<u32>,
    table: Option<&[procs::Proc]>,
    me: u32,
) -> CallerStanding {
    let (Some(pane_pid), Some(rows)) = (pane_pid, table) else {
        let gap = crate::seat_relaunch::unproven_gap(pane_pid, "", table.is_some());
        return CallerStanding::Unprovable(gap);
    };
    if !rows.iter().any(|row| row.pid == pane_pid) {
        let gap = format!("the pane's own process (pid {pane_pid}) is not in the process table");
        return CallerStanding::Unprovable(gap);
    }
    match procs::lineage(rows, pane_pid, me) {
        procs::Lineage::Under => CallerStanding::Under,
        procs::Lineage::Clear => CallerStanding::Clear,
        procs::Lineage::Broken(at) => CallerStanding::Unprovable(if at == me {
            format!("this command's own process (pid {me}) is not in the process table")
        } else {
            format!("this command's ancestry breaks at pid {at}, before the pane or pid 1")
        }),
    }
}

/// PURE: the refusal for a pane whose tree runs a harness the seat does NOT
/// record beside the one it does — `None` when the tree holds only its own.
///
/// `respawn-pane -k` ends the pane's WHOLE tree, so a stop proven only for
/// the recorded tool would end the other one too. Asked from the SAME snapshot
/// the caller proof read, before the send-lock, the frame and the respawn, and
/// no flag lifts it: `--stop-unknown` is about an unreadable frame, and this is
/// a readable tree ae does not own whole.
fn mixed_refusal(
    rows: &[procs::Proc],
    pane_pid: u32,
    recorded: &crate::seat_relaunch::Recorded<'_>,
    target: &Target,
) -> Option<String> {
    let foreign: Vec<String> = procs::harness_rows(rows, pane_pid)
        .into_iter()
        .filter(|(_, proc, _)| !procs::name_matches(&proc.comm, recorded.binary))
        .map(|(tool, proc, place)| match place {
            procs::Lineage::Broken(at) => format!(
                "{} (pid {}, which ae cannot place: its ancestry breaks at pid {at})",
                tool.as_str(),
                proc.pid
            ),
            _ => format!("{} (pid {}, parent {})", tool.as_str(), proc.pid, proc.ppid),
        })
        .collect();
    if foreign.is_empty() {
        return None;
    }
    let profile = if recorded.profile.is_empty() {
        "no profile".to_owned()
    } else {
        format!("profile '{}'", recorded.profile)
    };
    Some(format!(
        "Error: '{}' runs its recorded {} ({profile}) in pane {}, but a harness it does not \
         record runs in, or cannot be placed outside, that pane's tree: {} — a stop ends the \
         whole tree, and ae ends only the tool a seat records, so nothing was stopped. Wait for \
         it to exit, or end it yourself, then re-run the reseat.",
        target.agent,
        recorded.binary,
        target.pane,
        foreign.join(", ")
    ))
}

/// PURE: is `probe` a pane back at an IDLE shell?
///
/// The same two questions [`crate::seat_relaunch::prove_dead`] asks, asked of
/// the readings the wait already has — a shell in the foreground and nothing
/// running under it. An unreadable pid or an unusable process table is FALSE,
/// so the wait keeps waiting and then refuses rather than declaring a pane
/// idle it could not read.
fn pane_back_at_shell(probe: &ObservedPaneProbe, table: Option<&[procs::Proc]>) -> bool {
    probe.pid.is_some()
        && crate::watchdog::command_is_shell(&probe.command)
        && table.is_some_and(|table| !procs::has_any_descendant(table, probe.pid))
}

/// How long a respawned shell must read idle, unbroken, before the dead proof.
const STOP_SETTLE: Duration = Duration::from_secs(1);

/// The index of the reading that completes [`STOP_SETTLE`] of unbroken quiet
/// since the respawn, or `None`: a fresh shell reads idle before its startup
/// hooks spawn, so any unclean reading, a failed probe included, restarts it.
/// Named residual: a child later than the window still races the dead proof.
fn settled(readings: impl IntoIterator<Item = (Duration, bool)>) -> Option<usize> {
    let mut since = None;
    for (at, (elapsed, back)) in readings.into_iter().enumerate() {
        if !back {
            since = None;
            continue;
        }
        let first = *since.get_or_insert(elapsed);
        if elapsed.saturating_sub(first) >= STOP_SETTLE {
            return Some(at);
        }
    }
    None
}

/// The post-stop wait: at most [`STOP_POLLS`] of `tick` (pause, then time since
/// the respawn) and `read` (probe and table, `None` if it failed), [`settled`].
fn wait_back_at_shell(
    mut tick: impl FnMut() -> Duration,
    mut read: impl FnMut() -> Option<(ObservedPaneProbe, Option<Vec<procs::Proc>>)>,
) -> bool {
    settled((0..STOP_POLLS).map(|_| {
        let at = tick();
        let back =
            read().is_some_and(|(probe, table)| pane_back_at_shell(&probe, table.as_deref()));
        (at, back)
    }))
    .is_some()
}

/// What a timed-out stop SAYS, from the last reading it could take.
///
/// The two cases need different advice and conflating them wastes a human's
/// time. A SHELL in the foreground means the respawn took and something runs
/// under it, which usually finishes — re-running the reseat is the right next
/// step. A NON-shell foreground means the pane came back running something
/// else, which for an ae seat pane cannot happen and for a pane made by hand
/// means the respawn restarted that pane's own start command: no number of
/// retries changes it, so the advice is to look rather than try again.
fn stop_timeout_advice(probe: Option<&ObservedPaneProbe>) -> (String, &'static str) {
    let Some(probe) = probe else {
        return (
            "the pane stopped answering".to_owned(),
            "look at the pane before trying again.",
        );
    };
    if crate::watchdog::command_is_shell(&probe.command) {
        return (
            "its shell is back but something still runs under it".to_owned(),
            "re-run the reseat once that finishes.",
        );
    }
    (
        format!("'{}' holds its foreground", probe.command),
        "look at the pane: the respawn did not leave a shell, so re-running will not help.",
    )
}

/// Whether one capture PROVES the seat sits on its vendor's usage limit over an
/// empty input box.
///
/// The limit is the vendor's own row, read by the watchdog's one reader. The
/// EMPTY box is the frame grammar's own recognition: it reads a frame only
/// when the prompt row holds nothing, so a draft reads no frame at all. A
/// tool with neither grammar proves nothing.
///
/// The row must be CURRENT: an unknown verdict proves the limit only when the
/// nearest row above the box is that cell, never a limit left in the transcript,
/// or claude 2.1.296's own row for a limit wait the human cancelled (measured),
/// or when nothing but blank rows, user-echo blocks and the vendor's own limit
/// rows follow the cell down to the box (`watchdog::limit_standing`): a message
/// delivered to a limited seat, or the vendor's head drawn under its cell
/// (claude 2.1.317), is not a turn.
///
/// A tool that draws its limit as a MENU in place of the box has no box to
/// read: the menu as the capture's live bottom, with no turn drawn above it
/// (`watchdog::limit_menu_idle`), is the proof, and nothing is answered in it.
///
/// Muse's frame is no frame of the grammar above (it classifies unknown), so its
/// stop is proven by `watchdog::limit_standing` alone: the usage-limit block as
/// the newest transcript block over an empty box, and no background task below.
/// Agy's input is unmodelled (`read_frame` reads none), so its limit stop is proven
/// the same way and ONLY the limit's: the `⚠` row, its `Error ID:` row and
/// exactly the measured empty composer, nothing else (`watchdog::agy_limit_standing`).
fn limit_proven(capture: &str, agent_bin: &str, tool: ToolKind) -> bool {
    if crate::watchdog::limit_menu_idle(capture, agent_bin) {
        return true;
    }
    // Muse draws a frame claude's grammar does not read (`classify` says unknown
    // for every muse frame, idle ones included, and stays so): its limit stop is
    // proven by its own scan alone, which reads the block, the empty box and the
    // footer below it, and refuses a background task.
    if tool.limit_screen_scanned() {
        return crate::watchdog::limit_standing(capture, agent_bin)
            == crate::watchdog::LimitStanding::Stands;
    }
    let (Some(rows), Some(frame)) = (
        crate::watchdog::limit_rows(capture, agent_bin),
        crate::harness_state::read_frame(capture, tool),
    ) else {
        return false;
    };
    frame.state != HarnessState::Unknown
        || frame.current.is_some_and(|current| {
            tool.limit_wait_cancelled(&current)
                || rows
                    .iter()
                    .any(|row| row.trim().replace('\u{a0}', " ") == current)
        })
        || crate::watchdog::limit_standing(capture, agent_bin)
            == crate::watchdog::LimitStanding::Stands
}

/// One frame reading: its state, whether it proves the usage limit, and the
/// capture it was read from (`None` when ae could not take one).
type Reading<'a> = (HarnessState, bool, Option<&'a str>);

/// The reason a refused stop names first, so the head a journal keeps carries
/// it: the first reading that proves nothing decides; two readings that each
/// prove but not alike (a frame that changed between them) say so.
fn refusal_why(first: Reading<'_>, second: Reading<'_>, agent_bin: &str, tool: ToolKind) -> String {
    frame_gap(first, agent_bin, tool)
        .or_else(|| frame_gap(second, agent_bin, tool))
        .unwrap_or_else(|| "the frame changed between the two readings".to_owned())
}

/// Why one reading did not prove the frame, as the refusal names it first — a
/// journal record keeps only the line's head — or `None` when the reading
/// proves the stop on its own (idle, or the limit proven).
fn frame_gap(reading: Reading<'_>, agent_bin: &str, tool: ToolKind) -> Option<String> {
    let (state, limited, capture) = reading;
    if state == HarnessState::Idle || limited {
        return None;
    }
    // Muse's frame is read by its own scan, never by the claude grammar the
    // classifier routes it to, so a muse capture is always a frame to judge; so
    // is an agy capture, which no grammar reads.
    let Some(capture) = capture.filter(|capture| {
        tool.limit_screen_scanned() || crate::harness_state::read_frame(capture, tool).is_some()
    }) else {
        return Some("no frame".to_owned());
    };
    Some(match crate::watchdog::limit_standing(capture, agent_bin) {
        crate::watchdog::LimitStanding::RowAfterCell(row) => {
            format!("row after the limit row: \"{}\"", row.replace('"', ""))
        }
        crate::watchdog::LimitStanding::BackgroundTask(row) => {
            format!("background task: \"{}\"", row.replace('"', ""))
        }
        // A tool whose input no grammar reads and whose scan finds no limit
        // cell has no frame at all, as it had before the scan: agy's refusal
        // for a seat that is not on its limit keeps saying so.
        crate::watchdog::LimitStanding::NoCell if !tool.input_model().is_modelled() => {
            "no frame".to_owned()
        }
        crate::watchdog::LimitStanding::NoBox if tool.limit_screen_scanned() => {
            "no empty box under the limit row".to_owned()
        }
        crate::watchdog::LimitStanding::Stands if tool.limit_screen_scanned() => return None,
        _ => "no limit row".to_owned(),
    })
}

/// Whether the reading that decides a refusal — the first that proves nothing,
/// as [`refusal_why`] reads them — shows nothing but the stop and the seat's
/// background tasks: the one frame a refusal names `background-task` (#304).
fn background_only(first: Reading<'_>, second: Reading<'_>, tool: ToolKind) -> bool {
    tool.background_tasks_read()
        && [first, second]
            .into_iter()
            .find(|(state, limited, _)| *state != HarnessState::Idle && !limited)
            .and_then(|(_, _, capture)| capture)
            .and_then(crate::watchdog::muse_background_tasks)
            .is_some()
}

/// The last word on a stop that ends a seat's background tasks (#304), from
/// the two readings that proved it and a third taken under every lock, the
/// clock `now`, and the server's clients (`None` when they could not be read)
/// with the pane they are judged for: the titles the stop ends, or why it may
/// not.
fn long_limit_last_word(
    captures: [Option<&str>; 3],
    ask: LongLimit,
    now: i64,
    (clients, pane): (Option<&[crate::tmux::ObservedClient]>, &str),
) -> Result<Vec<String>, &'static str> {
    let Some(clients) = clients else {
        return Err("ae cannot read the pane's clients");
    };
    // `viewing_input` skips a client whose input it cannot read; here that
    // client may have just typed, so it refuses.
    if clients
        .iter()
        .any(|client| client.pane == pane && client.activity.is_none())
    {
        return Err("ae cannot read the input of a client viewing the pane");
    }
    let input = crate::watchdog_daemon::viewing_input(Some(clients), pane);
    if input.is_some_and(|at| i64::try_from(at).unwrap_or(i64::MAX) >= ask.opened) {
        return Err("a client gave the pane input since the move began");
    }
    let [Some(first), Some(second), Some(third)] = captures else {
        return Err("ae could not read the frame again");
    };
    crate::autoreseat::long_limit_stop(&[first, second, third], ask.key, now)
        .map(|proof| proof.tasks)
        .map_err(LongLimitGap::words)
}

/// The seat's current harness frame, fail-closed, whether that SAME capture
/// proves the seat sits on its usage limit, and the capture itself.
///
/// A capture ae could not take is [`HarnessState::Unknown`], never idle: the
/// absence of a reading is not evidence that a turn is not running.
fn frame(target: &Target, tool: ToolKind, agent_bin: &str) -> (HarnessState, bool, Option<String>) {
    transport::capture_pane(&target.server, &target.pane).map_or(
        (HarnessState::Unknown, false, None),
        |capture| {
            (
                crate::harness_state::classify(&capture, tool),
                limit_proven(&capture, agent_bin, tool),
                Some(capture),
            )
        },
    )
}

/// Whether two readings of a seat's frame, neither of them `Busy`, let a stop
/// act under `policy`. An idle box on BOTH is one proof. A seat on its usage
/// limit draws the vendor's row where a finished turn would sit, so its frame
/// never reads idle: that row over an EMPTY box on BOTH is the other. One
/// reading is never enough, because a seat handed work between the two reads
/// differently the second time. `--stop-unknown` lifts the rest.
fn may_stop(
    (first, first_limited): (HarnessState, bool),
    (second, second_limited): (HarnessState, bool),
    policy: StopPolicy,
) -> bool {
    let proven_idle = first == HarnessState::Idle && second == HarnessState::Idle;
    let proven_limited = first_limited && second_limited;
    proven_idle || proven_limited || policy == StopPolicy::Unknown
}

/// STOP THE SEAT'S TOOL, in place. Runs UNDER the lifecycle lock, BEFORE the
/// dead proof and before anything durable is written.
///
/// The mechanism is `respawn-pane -k`, the same tmux verb ae already uses to
/// replace a monitor process: measured on tmux 3.7b it keeps the pane id, the
/// pane's `@ae_*` options, its index and its scrollback, kills the pane's
/// process tree, and leaves tmux's default shell in the directory `-c` names.
/// No new process door, and no signal ae would need `unsafe` to send.
#[allow(
    clippy::too_many_lines,
    reason = "the stop ladder, kept in one place beside the order it enforces"
)]
fn stop_running_tool(
    dir: &Path,
    target: &Target,
    bytes: &[u8],
    parsed: &Parsed,
    hook: &mut dyn FnMut() -> bool,
    err: &mut impl Write,
) -> io::Result<Stop> {
    let policy = parsed.stop;
    let agent_bin = crate::deliver::recorded_binary(dir, &target.slot);
    // IS IT RUNNING? Read the way the dead proof reads it, from ONE probe and
    // ONE process-table snapshot. A seat with no recorded tool, or a pane that
    // will not answer, is not something this step may kill: the dead proof
    // names both, in its own words, a moment from now.
    let Some(probe) = transport::observe_pane_probe(&target.server, &target.pane) else {
        return Ok(Stop::NotRunning);
    };
    let table = procs::snapshot();
    let walk = probe.pid.map_or(Descendancy::Unknown, |pid| {
        procs::descendancy(table.as_deref(), pid, &agent_bin)
    });
    if !crate::seat_relaunch::identity_proven(&probe.command, &agent_bin, walk) {
        return Ok(Stop::NotRunning);
    }

    // FROM HERE THE TOOL IS RUNNING, and every arm below refuses before the
    // first write. The working copy first, because it is a DURABLE fact the
    // records already carry and the shell is respawned into it: refusing on it
    // after the kill would be the worst order this verb could take.
    let work_dir = match crate::seat_relaunch::usable_work_dir(bytes) {
        Ok(work_dir) => work_dir,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Ok(Stop::Refused(CauseClass::Other));
        }
    };
    // NOT FROM UNDER THE SEAT ITSELF. The pane check upstream catches a caller
    // ae stamped; this catches the other half — a process the target's own
    // tool started, which carries no `$TMUX_PANE` of its own to compare. Both
    // halves need the pid and the table, so a gap in either REFUSES: ae will
    // not kill a tool it cannot prove is not its own parent.
    match caller_standing(probe.pid, table.as_deref(), std::process::id()) {
        CallerStanding::Clear => {
            // Clear PROVES the pid and the table, so the tree is read from the
            // very snapshot the caller proof just used.
            if let (Some(pid), Some(rows)) = (probe.pid, table.as_deref()) {
                let recorded = crate::seat_relaunch::Recorded {
                    binary: &agent_bin,
                    profile: &crate::lifecycle::meta_value(
                        bytes,
                        &format!("profile.{}", target.slot),
                    ),
                };
                if let Some(line) = mixed_refusal(rows, pid, &recorded, target) {
                    writeln!(err, "{line}")?;
                    return Ok(Stop::Refused(CauseClass::Other));
                }
            }
        }
        CallerStanding::Under => {
            writeln!(
                err,
                "Error: this command runs UNDER '{}' (pane {}) — stopping that tool would kill \
                 the process asking for the reseat. Run it from another seat, or from a plain \
                 shell.",
                target.agent, target.pane
            )?;
            return Ok(Stop::Refused(CauseClass::Other));
        }
        CallerStanding::Unprovable(gap) => {
            writeln!(
                err,
                "Error: '{}' is running in pane {} and ae cannot prove this command is not \
                 running under it ({gap}) — nothing was stopped.",
                target.agent, target.pane
            )?;
            return Ok(Stop::Refused(CauseClass::Other));
        }
    }
    // THE PANE'S SEND-LOCK, so no delivery can land a turn between the frame
    // ae reads and the kill it decides from. Held across the reading, the
    // respawn and the wait, and dropped with this scope — the seed turn later
    // takes no such lock and cannot deadlock against it.
    let Some(_send_lock) = crate::deliver::lock_target(dir, &target.pane, SEND_LOCK_WAIT) else {
        writeln!(
            err,
            "Error: a delivery holds pane {} of '{}' — nothing was stopped. Try again once it \
             finishes.",
            target.pane, target.agent
        )?;
        return Ok(Stop::Refused(CauseClass::Lock));
    };
    // THE FRAME. Only a POSITIVELY proven idle box may be stopped, and it is
    // proven TWICE: `Busy` short-circuits on the first reading, and a seat
    // that was handed work between the two readings is caught by the second.
    let tool = ToolKind::from_binary_name(&agent_bin);
    let busy = |err: &mut dyn Write| -> io::Result<()> {
        writeln!(
            err,
            "Error: '{}' is BUSY — a turn is running in pane {}. Wait for it, or \
             `{}/interrupt {}` first, then re-run the reseat.",
            target.agent,
            target.pane,
            dir.display(),
            target.agent
        )
    };
    let (first, first_limited, first_capture) = frame(target, tool, &agent_bin);
    if first == HarnessState::Busy {
        busy(err)?;
        return Ok(Stop::Refused(CauseClass::Busy));
    }
    std::thread::sleep(FRAME_GAP);
    let (second, second_limited, second_capture) = frame(target, tool, &agent_bin);
    if second == HarnessState::Busy {
        busy(err)?;
        return Ok(Stop::Refused(CauseClass::Busy));
    }
    let quiet = policy == StopPolicy::QuietStart
        && crate::autoreseat::quiet_pair(
            first_capture.as_deref(),
            second_capture.as_deref(),
            &agent_bin,
        );
    let proven = quiet || may_stop((first, first_limited), (second, second_limited), policy);
    // A limit episode's move may end the seat's background tasks when both
    // readings show nothing else and the vendor's own row proves the limit
    // long (#304); every other unproven frame refuses as it always has.
    let long = parsed
        .long_limit
        .filter(|_| !proven && tool.background_tasks_read())
        .map(
            |ask| match (first_capture.as_deref(), second_capture.as_deref()) {
                (Some(first), Some(second)) => crate::autoreseat::long_limit_stop(
                    &[first, second],
                    ask.key,
                    Timestamp::now().epoch(),
                ),
                _ => Err(LongLimitGap::NotBackgroundOnly),
            },
        );
    if !proven && !matches!(long, Some(Ok(_))) {
        // The next step FIRST, with its precondition: a record keeps only the
        // line's head, and `--stop-unknown` is safe only on what a human sees.
        let limited = transport::capture_pane(&target.server, &target.pane)
            .is_some_and(|capture| crate::watchdog::limit_notice(&capture, &agent_bin).is_some());
        let first = (first, first_limited, first_capture.as_deref());
        let second = (second, second_limited, second_capture.as_deref());
        // WHY first, so the head a journal keeps names it: the first reading
        // that proves nothing decides; two readings that each prove but not
        // alike (a frame that changed between them) say so.
        let why = refusal_why(first, second, &agent_bin, tool);
        if background_only(first, second, tool) {
            let gap = long
                .and_then(Result::err)
                .map_or_else(String::new, |gap| format!("; {}", gap.words()));
            writeln!(
                err,
                "Error: frame not proven ({why}{gap}) — once its box is empty and its background \
                 task may end, run: ae reseat {} {} --using {} --stop-unknown — a stop would end \
                 the task '{}' runs in pane {}. Nothing was reseated.",
                parsed.session, target.agent, parsed.profile, agent_bin, target.pane
            )?;
            return Ok(Stop::Refused(CauseClass::BackgroundTask));
        }
        writeln!(
            err,
            "Error: frame not proven ({why}) — once its box is empty and {}, run: ae reseat {} {} \
             --using {} --stop-unknown — ae cannot read '{}'s frame in pane {}, so it cannot tell \
             a running turn from an idle box. Nothing was reseated.",
            if limited {
                "the limit line is its newest row"
            } else {
                "no turn runs"
            },
            parsed.session,
            target.agent,
            parsed.profile,
            agent_bin,
            target.pane
        )?;
        return Ok(Stop::Refused(CauseClass::UnknownFrame));
    }

    // The caller's last word under every lock: the effort leg reserves here.
    if !hook() {
        return Ok(Stop::Refused(CauseClass::Other));
    }
    // A stop that ends background tasks takes ITS last word after the
    // caller's, under every lock, immediately before the write: the send-lock
    // does not hold the keyboard, so a THIRD reading must prove what the two
    // did, at a fresh clock, and no client may have given the pane input since
    // the move began. Any gap refuses, and nothing was stopped.
    let ended = match (long, parsed.long_limit) {
        (Some(Ok(_)), Some(ask)) => {
            let third = transport::capture_pane(&target.server, &target.pane);
            let clients = transport::observe_clients(&target.server);
            let captures = [
                first_capture.as_deref(),
                second_capture.as_deref(),
                third.as_deref(),
            ];
            let viewing = (clients.as_deref(), target.pane.as_str());
            match long_limit_last_word(captures, ask, Timestamp::now().epoch(), viewing) {
                Ok(tasks) => tasks,
                Err(why) => {
                    writeln!(
                        err,
                        "Error: background task not proven at the stop ({why}) — once its box is \
                         empty and its background task may end, run: ae reseat {} {} --using {} \
                         --stop-unknown — nothing was stopped. Nothing was reseated.",
                        parsed.session, target.agent, parsed.profile
                    )?;
                    return Ok(Stop::Refused(CauseClass::BackgroundTask));
                }
            }
        }
        _ => Vec::new(),
    };
    // THE ONLY WRITE. A tmux that refuses the argv leaves the tool running,
    // and says so in its own words rather than the timeout's.
    let (respawned, _) = transport::run_tmux_op(&crate::session_tmux::argv(
        &target.server,
        &Op::RespawnPane {
            pane: &target.pane,
            work_dir: &work_dir,
            command: &[],
        },
    ));
    if !respawned {
        writeln!(
            err,
            "Error: tmux would not respawn pane {} of '{}' — the tool is still running and \
             nothing was reseated.",
            target.pane, target.agent
        )?;
        return Ok(Stop::Failed);
    }
    // The tool is gone, and its background tasks with it.
    parsed.ended.replace(ended);
    let respawned_at = std::time::Instant::now();
    let tick = || {
        std::thread::sleep(STOP_POLL);
        respawned_at.elapsed()
    };
    let read = || {
        transport::observe_pane_probe(&target.server, &target.pane)
            .map(|probe| (probe, procs::snapshot()))
    };
    if wait_back_at_shell(tick, read) {
        return Ok(Stop::Stopped(agent_bin));
    }
    // PAST THE KILL and still not idle: say what holds the pane, because this
    // is the one refusal where the seat's tool has already been stopped. The
    // meta is untouched, so `relaunch` brings the seat back on the profile it
    // still records, and a second reseat is free to try again.
    let (holding, next) =
        stop_timeout_advice(transport::observe_pane_probe(&target.server, &target.pane).as_ref());
    writeln!(
        err,
        "Error: '{}' was stopped but pane {} is not back at an idle shell after {}s — {holding}. \
         Nothing was moved: '{}' still records its old profile, so {next}",
        target.agent,
        target.pane,
        (STOP_POLL * STOP_POLLS).as_secs(),
        target.agent
    )?;
    Ok(Stop::Failed)
}

/// The effort leg's actuator: restart `agent` IN PLACE on its own records —
/// the stop with `hook` as its last word, the dead proof, `_run`, all under the
/// lifecycle lock, recorded as no reseat or relaunch. `None`: nothing stopped.
pub(crate) fn restart_in_place(
    root: &Path,
    dir: &Path,
    session: &str,
    agent: &str,
    hook: &mut dyn FnMut() -> bool,
    err: &mut impl Write,
) -> io::Result<Option<bool>> {
    let Ok(target) = resolve(dir, agent, session) else {
        return Ok(None);
    };
    let Ok(_lifecycle) = crate::lifecycle::lock(root, session) else {
        return Ok(None);
    };
    let locked = crate::meta::read_bytes(dir).unwrap_or_default();
    let parsed = Parsed {
        session: session.to_owned(),
        agent: agent.to_owned(),
        profile: crate::lifecycle::meta_value(&locked, &format!("profile.{}", target.slot)),
        client: None,
        stop: StopPolicy::Proven,
        automatic: false,
        long_limit: None,
        expect: None,
        ended: std::cell::RefCell::default(),
    };
    match stop_running_tool(dir, &target, &locked, &parsed, hook, err)? {
        Stop::Stopped(_) => {}
        Stop::Failed => return Ok(Some(false)),
        Stop::NotRunning | Stop::Refused(_) => return Ok(None),
    }
    let Some(proven) = crate::seat_relaunch::prove_dead(dir, &target, RESEAT_VERB, err)? else {
        return Ok(Some(false));
    };
    let started =
        crate::seat_relaunch::start(dir, &target, &proven, Timestamp::now(), RESEAT_VERB, err)?;
    Ok(Some(matches!(started, Started::Running { .. })))
}

/// `ae reseat <session> <agent> --using <profile[@client]> [--stop-unknown]`.
///
/// # Errors
///
/// Only a failure to write `out` or `err`. Every refusal is an exit code.
pub(crate) fn run(
    root: &Path,
    world: Option<&crate::listing::World>,
    tail: &[String],
    now: Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<u8> {
    let parsed = match parse(tail) {
        Ok(parsed) => parsed,
        Err(line) => {
            writeln!(err, "{line}")?;
            if line != RESEAT_USAGE {
                writeln!(err, "{RESEAT_USAGE}")?;
            }
            return Ok(EXIT_USAGE);
        }
    };
    let caller = caller_of(&parsed.session);
    Ok(
        match run_parsed(root, world, &parsed, caller, now, &mut || true, out, err)? {
            Ended::Moved { .. } => 0,
            Ended::Refused(_) | Ended::Failed => EXIT_FAILED,
        },
    )
}

/// Move `agent` of `session` to `profile` AS THE WATCHDOG: both `reseat`
/// records name it, and a running tool is stopped only on a proven frame.
///
/// No caller rule: the leg that asks is ae's own, run from the watchdog. A
/// limit episode's move names its episode in `long_limit`, and only such a move
/// may end the seat's background tasks; their titles come back beside the
/// ending, empty when it ended none.
///
/// # Errors
///
/// Only a failure to write `out` or `err`.
#[allow(
    clippy::too_many_arguments,
    reason = "the verb's own inputs, spelled out rather than bundled"
)]
pub(crate) fn run_as_watchdog(
    root: &Path,
    world: Option<&crate::listing::World>,
    session: &str,
    agent: &str,
    profile: &str,
    now: Timestamp,
    urgent: Option<&mut dyn FnMut() -> bool>,
    long_limit: Option<LongLimit>,
    expect: Option<crate::autoreseat::leg::Expect>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<(Ended, Vec<String>)> {
    let mut open = || true;
    let (stop, hook): (_, &mut dyn FnMut() -> bool) = match urgent {
        Some(hook) => (StopPolicy::QuietStart, hook),
        None => (StopPolicy::Proven, &mut open),
    };
    let parsed = Parsed {
        session: session.to_owned(),
        agent: agent.to_owned(),
        profile: profile.to_owned(),
        client: None,
        stop,
        automatic: true,
        long_limit,
        expect,
        ended: std::cell::RefCell::default(),
    };
    let caller = Caller {
        pane: crate::doors::calling_pane_id(),
        display: crate::watchdog::WATCHDOG_ACTOR.to_owned(),
    };
    let ended = run_parsed(root, world, &parsed, Ok(caller), now, hook, out, err)?;
    Ok((ended, parsed.ended.take()))
}

/// A profile as a reseat to it would launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// The model its `--model` flag pins, when it pins one.
    pub(crate) pin: Option<String>,
    /// The tool whose pin rule reads `pin`.
    pub(crate) tool: ToolKind,
}

/// Profile `profile` for the seat whose session dir is `dir`, resolved exactly
/// as a reseat to it would resolve it, or `None` when it does not resolve.
pub(crate) fn resolved(dir: &Path, profile: &str) -> Option<Resolved> {
    let bytes = crate::meta::read_bytes(dir).unwrap_or_default();
    resolve_profile(dir, &bytes, profile, "")
        .ok()
        .map(|moving| Resolved {
            pin: moving.pin,
            tool: moving.tool,
        })
}

/// The move itself, from a parsed argv and the caller rule's verdict. Every
/// ending says what became of the seat.
#[allow(
    clippy::too_many_arguments,
    reason = "the verb's own inputs, spelled out rather than bundled"
)]
fn run_parsed(
    root: &Path,
    world: Option<&crate::listing::World>,
    parsed: &Parsed,
    caller: Result<Caller, String>,
    now: Timestamp,
    hook: &mut dyn FnMut() -> bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Ended> {
    let ask = Ask {
        root,
        world,
        parsed,
        now,
    };
    match move_seat(&ask, caller, hook, out, err) {
        Ok(ended) | Err(Halt::End(ended)) => Ok(ended),
        Err(Halt::Io(why)) => Err(why),
    }
}

/// The verb's read-only inputs, asked of by every step.
struct Ask<'a> {
    root: &'a Path,
    world: Option<&'a crate::listing::World>,
    parsed: &'a Parsed,
    now: Timestamp,
}

/// Why a step ended the move early: the ending it decided, its refusal
/// already printed, or a stream that would not take that refusal.
enum Halt {
    End(Ended),
    Io(io::Error),
}

impl From<io::Error> for Halt {
    fn from(why: io::Error) -> Self {
        Self::Io(why)
    }
}

/// The order is the contract (the module docs): every step below is called
/// once, in this order, and a step that ends the move has said why.
fn move_seat(
    ask: &Ask<'_>,
    caller: Result<Caller, String>,
    hook: &mut dyn FnMut() -> bool,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Ended, Halt> {
    let seat = admit(ask, caller, err)?;
    // THE LOCK, taken the way every lifecycle writer takes it: the dead proof,
    // the seed, the slot cleanup, the meta move and the paste are ONE
    // transaction, so no stop, compact, relaunch or second reseat interleaves.
    let Ok(lifecycle) = crate::lifecycle::lock(ask.root, &ask.parsed.session) else {
        writeln!(
            err,
            "Error: another lifecycle operation holds '{}' — it waited and gave up. Try again \
             once that finishes.",
            ask.parsed.session
        )?;
        return Ok(Ended::Refused(CauseClass::Lock));
    };
    // What the records say NOW, under the lock: the stop reads the working copy
    // from it, and the audit line reads the conversation the predecessor holds.
    let locked = crate::meta::read_bytes(&seat.dir).unwrap_or_default();
    let prepared = prepare(ask, &seat, &locked, err)?;
    let mut at = Record {
        caller: &seat.caller,
        now: ask.now,
        from: &seat.from,
        to: &seat.to,
        prior: &prepared.prior,
        // Filled in once the carry question is answered, below. Every record
        // written before that is a STOP, and a stop names no conversation at
        // all ([`Mentions`]), so no record can carry this placeholder into an
        // audit line.
        carry: Carried::No,
    };
    let (before, broke) = stop_and_prove_dead(ask, &seat, &locked, &at, hook, err)?;
    let travel = conversation(&seat, &locked, &prepared, &before, broke, out, err)?;
    at.carry = travel.carried.clone();
    let seed = build_seed(ask, &seat, &travel, broke, err)?;
    clear_launch_files(&seat, &travel, broke, err)?;
    let after = publish_move(ask, &seat, &travel, &prepared.prior, before, broke, err)?;
    let started =
        crate::seat_relaunch::start(&seat.dir, &seat.target, &after, ask.now, RESEAT_VERB, err)?;
    // PAST THE LOCK before any readiness wait: a gated turn blocks up to 45s,
    // and nothing else may be held out of the session's lifecycle for that.
    drop(lifecycle);
    let (dir, target, profile) = (&seat.dir, &seat.target, &ask.parsed.profile);
    match started {
        // Nothing reached the pane and the refusing site already said why. The
        // meta HAS moved, so the next step is named rather than left implicit.
        Started::NotPasted => {
            writeln!(
                err,
                "  '{}' records profile '{profile}' now — `relaunch {}` starts it.",
                target.agent, target.agent
            )?;
            Ok(Ended::Failed)
        }
        Started::NotSeen => Ok(not_seen(
            &seat,
            profile,
            seed.as_deref(),
            &mut at,
            out,
            err,
        )?),
        Started::Running { resuming_seat } => Ok(finish(
            dir,
            target,
            &after,
            resuming_seat,
            ask.parsed,
            seed.as_deref(),
            &mut at,
            &seat.from_command.as_ref().map_or_else(String::new, |from| {
                kept_change(from, &seat.moving.command, seat.moving.tool)
            }),
            out,
            err,
        )?),
    }
}

/// The seat this move is about, every fact of it resolved BEFORE the lock.
struct Admitted<'w> {
    /// The session as the caller's world enumerated it, for the seed pack.
    entry: &'w crate::digest::SessionEntry,
    dir: PathBuf,
    caller: Caller,
    /// The profile the seat recorded BEFORE the move, as the two messages that
    /// say what a half-finished move left behind spell it.
    was: String,
    /// The record's `from` and `to`: each profile with the client it runs on,
    /// `profile@client`, and the bare profile where there is none. Text a human
    /// reads; no reader parses a profile name out of it.
    from: String,
    to: String,
    moving: Moving,
    /// The command the recorded profile resolves to now, for the keep question.
    from_command: Option<String>,
    target: Target,
}

/// Step 1 of the order: the session, the caller, the seat and the profile
/// resolve, and nothing is written. Every refusal here is
/// [`Ended::Refused`] for [`CauseClass::Other`]: nothing of the seat changed.
fn admit<'w>(
    ask: &Ask<'w>,
    caller: Result<Caller, String>,
    err: &mut impl Write,
) -> Result<Admitted<'w>, Halt> {
    let (parsed, refused) = (ask.parsed, Halt::End(Ended::Refused(CauseClass::Other)));
    // The seed is built from the world the caller already enumerated, so the
    // pack a successor is handed is the one `ae brief --seat` prints this
    // instant — and an incomplete enumeration refuses rather than packs a
    // session it only half saw.
    let Some(entry) = ask.world.and_then(|world| {
        world
            .sessions
            .iter()
            .find(|entry| entry.name == parsed.session)
    }) else {
        writeln!(
            err,
            "Error: no session named '{}' that ae could read.",
            parsed.session
        )?;
        return Err(refused);
    };
    let dir = crate::inventory::Roots::under(ask.root)
        .sessions()
        .join(&parsed.session);
    let caller = match caller {
        Ok(caller) => caller,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Err(refused);
        }
    };
    // THE ROSTER FIRST, and tmux only after it. Everything a typo gets wrong —
    // the seat's name, the profile — is a DURABLE fact, and answering it from
    // the meta means a stopped session diagnoses the argv exactly as a running
    // one does instead of refusing for want of a pane.
    let bytes = crate::meta::read_bytes(&dir).unwrap_or_default();
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let Some(seated) = meta.roster().iter().find(|row| row.name == parsed.agent) else {
        let roster: Vec<&str> = meta.roster().iter().map(|row| row.name.as_str()).collect();
        writeln!(
            err,
            "Error: '{}' has no seat named '{}'; its roster is {}.",
            parsed.session,
            parsed.agent,
            if roster.is_empty() {
                "empty".to_owned()
            } else {
                roster.join(", ")
            }
        )?;
        return Err(refused);
    };
    let (slot, recorded) = (
        seated.slot.clone(),
        seated.profile.clone().unwrap_or_default(),
    );
    // What the seat recorded BEFORE the move, for the two messages that must
    // say what a half-finished move left behind. A seat with no profile row is
    // legal input: the move is what gives it one, and an empty string never
    // equals the non-empty profile the parse guarantees.
    let was = if recorded.is_empty() {
        "no profile".to_owned()
    } else {
        format!("profile '{recorded}'")
    };
    let moving = match destination(&dir, &bytes, parsed, seated, &recorded) {
        Ok(moving) => moving,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Err(refused);
        }
    };
    let from_command = recorded_command(&dir, &bytes, seated);
    // The moment of a human's choice: say once if the seat has no exit off
    // the account it moves to — a `profile@client` move's own, not the
    // profile's default. The watchdog's own move is no choice of anyone's.
    if caller.display != crate::watchdog::WATCHDOG_ACTOR
        && let Some(line) = cover_warning(
            &dir,
            &bytes,
            &parsed.profile,
            moving.pair.is_some().then_some(moving.command.as_str()),
        )
    {
        writeln!(err, "{line}")?;
    }
    let target = place(&dir, parsed, &caller, &slot, err)?;
    let spelled = |profile: &str, client: Option<&str>| {
        client.map_or_else(|| profile.to_owned(), |label| format!("{profile}@{label}"))
    };
    let from = match &seated.client {
        crate::meta::RecordedClient::Label(label) => spelled(&recorded, Some(label)),
        _ => recorded.clone(),
    };
    let to = spelled(&parsed.profile, parsed.client.as_deref());
    Ok(Admitted {
        entry,
        dir,
        caller,
        was,
        from,
        to,
        moving,
        from_command,
        target,
    })
}

/// Where the seat moves, resolved — or the refusal line, nothing stopped or
/// written: something must move ([`same_client`]), the profile (bare) or the
/// pair ([`resolve_pair`]) must resolve, and a move must not land on a client
/// row ae cannot read.
fn destination(
    dir: &Path,
    bytes: &[u8],
    parsed: &Parsed,
    seated: &crate::meta::RosterEntry,
    recorded: &str,
) -> Result<Moving, String> {
    if recorded == parsed.profile && same_client(parsed.client.as_deref(), &seated.client) {
        return Err(match parsed.client.as_deref() {
            None => format!(
                "Error: '{}' already runs profile '{}' — there is nothing to move. `relaunch {}` \
                 brings it back on the same profile.",
                parsed.agent, parsed.profile, parsed.agent
            ),
            Some(label) => format!(
                "Error: '{}' already runs profile '{}' on client '{label}' — there is nothing to \
                 move. `relaunch {}` brings it back on the same profile.",
                parsed.agent, parsed.profile, parsed.agent
            ),
        });
    }
    let moving = match parsed.client.as_deref() {
        None => resolve_profile(dir, bytes, &parsed.profile, &parsed.agent)
            .map_err(|why| format!("Error: {why}. Nothing was reseated."))?,
        Some(label) => resolve_pair(bytes, &parsed.profile, label, &parsed.agent)?,
    };
    // A move onto a seat whose client row ae cannot read is refused HERE,
    // before anything is stopped, with the words the seat's own read uses: a
    // move must not stop a tool to then publish over a record it never read.
    if seated.client == crate::meta::RecordedClient::Invalid {
        return Err(format!(
            "Error: {}. Nothing was reseated.",
            crate::run::unusable_client(&seated.slot)
        ));
    }
    Ok(moving)
}

/// The seat's PANE, resolved after every durable fact: the target, never the
/// caller's own pane, and stamped with the slot the roster seats it at.
fn place(
    dir: &Path,
    parsed: &Parsed,
    caller: &Caller,
    slot: &str,
    err: &mut impl Write,
) -> Result<Target, Halt> {
    let refused = Halt::End(Ended::Refused(CauseClass::Other));
    let target = match resolve(dir, &parsed.agent, &parsed.session) {
        Ok(target) => target,
        Err(line) => {
            writeln!(err, "{line}")?;
            return Err(refused);
        }
    };
    // NOT YOUR OWN SEAT. The dead proof would refuse a live caller anyway, so
    // this closes no hole today — it states the rule instead of leaving it to
    // an accident of ordering, and it says the useful thing: the seat running
    // this command cannot be the seat whose tool is replaced under it.
    if caller.pane.as_deref() == Some(target.pane.as_str()) {
        writeln!(
            err,
            "Error: '{}' is THIS pane — a seat cannot reseat itself, because the tool running the \
             command is the one that would be replaced. Ask another seat, or run it from a shell.",
            target.agent
        )?;
        return Err(refused);
    }
    // The pane's own stamp against the roster. They disagree only when a stamp
    // is stale or a meta was hand-edited, and the move would then be published
    // for one seat and pasted into another's pane.
    if target.slot != slot {
        writeln!(
            err,
            "Error: pane {} is stamped slot {} but '{}' seats '{}' at {slot} — fix the meta, or \
             stop and resume the session.",
            target.pane, target.slot, parsed.session, parsed.agent
        )?;
        return Err(refused);
    }
    Ok(target)
}

/// What the records under the lock answered before anything is stopped.
struct Prepared {
    /// The conversation id the seat records, `harness_session.<slot>`.
    prior: String,
    /// Whether the move KEEPS that conversation ([`keeps`]).
    keeping: bool,
    /// What the carry Check proved, kept for the copy: `None` when no carry
    /// was asked of it.
    checked: Option<crate::carry::Crossing>,
}

/// Under the lock and before the stop: the automatic gate, the keep question,
/// the carry Check and the tool-change note — reads and stderr only.
fn prepare(
    ask: &Ask<'_>,
    seat: &Admitted<'_>,
    locked: &[u8],
    err: &mut impl Write,
) -> Result<Prepared, Halt> {
    let (target, moving) = (&seat.target, &seat.moving);
    // THE ONE AUTOMATIC GATE, under the lock and before anything is checked,
    // copied or stopped: the persisted record at this read decides. A later
    // observation written under the meta lock alone is not seen (named residual).
    if ask.parsed.automatic
        && let Some(hold) = crate::run::observed_standing(&seat.dir, &target.slot)
    {
        writeln!(err, "Error: {}", crate::autoreseat::observed_summary(&hold))?;
        return Err(Halt::End(Ended::Refused(CauseClass::ObservedChoice)));
    }
    // THE PAIR, read again under the same lock (#305): every automatic move of
    // either half takes it, so the partner's row here is the one its own last
    // move committed. A destination that would collapse the pair holds.
    if ask.parsed.automatic
        && let Some(why) =
            crate::autoreseat::leg::pair_changed(&seat.dir, locked, &target.slot, &moving.command)
    {
        writeln!(err, "Error: {why}")?;
        return Err(Halt::End(Ended::Refused(CauseClass::PairChanged)));
    }
    // A RETURN, read again under the same lock (#305): a human's or an agent's
    // reseat since the return was chosen is a choice, and wins.
    if let Some(expect) = ask.parsed.expect.as_ref()
        && let Some(why) = crate::autoreseat::leg::return_changed(
            &seat.dir,
            locked,
            (&ask.parsed.session, &target.slot, &target.agent),
            expect,
        )
    {
        writeln!(err, "Error: {why}")?;
        return Err(Halt::End(Ended::Refused(CauseClass::ReturnChanged)));
    }
    // THE LEAD FLOOR, read again under the same lock (#309): the global config and
    // the destination as they stand now, so a floor raised or a profile re-pinned
    // after the choice cannot carry a lead past it. A manual reseat never asks.
    if ask.parsed.automatic {
        let global = crate::autoreseat::settings(Some(&crate::doors::config_file(
            crate::shape::current(),
            ask.root,
        )));
        let destination = crate::autoreseat::Model {
            pin: moving.pin.clone(),
            tool: Some(moving.tool),
        };
        if let Some(why) = crate::autoreseat::leg::floor_changed(
            &global,
            (&seat.dir, locked),
            (&ask.parsed.session, &target.slot, &target.agent),
            (
                seat.from_command.as_deref(),
                &ask.parsed.profile,
                &destination,
            ),
            ask.parsed.expect.is_some(),
        ) {
            writeln!(err, "Error: {why}")?;
            return Err(Halt::End(Ended::Refused(CauseClass::FloorChanged)));
        }
    }
    let prior = crate::lifecycle::meta_value(locked, &format!("harness_session.{}", target.slot));
    // DOES THE SEAT KEEP ITS CONVERSATION? From the records under the lock. A
    // kept move stays in its account, which a carry never does.
    let row = |key: &str| crate::lifecycle::meta_value(locked, &format!("{key}.{}", target.slot));
    let keeping = keeps(&Keep {
        from_binary: &row("agent_bin"),
        to_binary: &moving.binary,
        tool: moving.tool,
        id: &prior,
        from_home: &row("config_home"),
        from_base: &row("config_home_base"),
        to_home: (moving.account.as_ref())
            .map(|account| (account.row.as_str(), account.base.as_deref())),
        from_command: seat.from_command.as_deref(),
        to_command: &moving.command,
    });
    // A CARRY THAT WOULD FAIL refuses HERE, before a running tool is ended: the
    // whole copy set is proven by the copy's own walk, writing nothing. A
    // working copy ae cannot use is left to the stop and the dead proof, which
    // name it in their own words.
    // What it proved is KEPT: the copy after the stop archives exactly what
    // this Check decided, or nothing.
    let mut checked = None;
    if let Ok(work_dir) = crate::seat_relaunch::usable_work_dir(locked)
        && let Some(plan) = carry_plan(locked, &target.slot, moving, &prior, &work_dir)
    {
        match crate::carry::run(&plan, crate::carry::Mode::Check) {
            Ok(crossing) => checked = Some(crossing),
            Err(why) => {
                let (from, to) = plan.homes();
                writeln!(
                    err,
                    "Error: '{}' could not carry conversation {} from {} to {} ({why}) — nothing \
                     was stopped or reseated.",
                    target.agent,
                    plan.id(),
                    from.display(),
                    to.display()
                )?;
                return Err(Halt::End(Ended::Refused(carry_class(why.kind()))));
            }
        }
    }
    // A TOOL CHANGE says what it costs before anything happens, a dead pane
    // included ([`tool_change_note`]).
    let from_binary = crate::lifecycle::meta_value(locked, &format!("agent_bin.{}", target.slot));
    if let Some(note) = tool_change_note(
        &target.agent,
        &ask.parsed.profile,
        &from_binary,
        &moving.binary,
        &prior,
    ) {
        writeln!(err, "{note}")?;
    }
    Ok(Prepared {
        prior,
        keeping,
        checked,
    })
}

/// THE STOP, before the dead proof and before anything durable is written,
/// then the dead proof itself. Returns the proof with how every later refusal
/// ends ([`past_the_stop`]), decided HERE from whether this move stopped the
/// tool.
fn stop_and_prove_dead(
    ask: &Ask<'_>,
    seat: &Admitted<'_>,
    locked: &[u8],
    at: &Record<'_>,
    hook: &mut dyn FnMut() -> bool,
    err: &mut impl Write,
) -> Result<(Proven, Ended), Halt> {
    let (dir, target) = (&seat.dir, &seat.target);
    let stopped = match stop_running_tool(dir, target, locked, ask.parsed, &mut *hook, err)? {
        Stop::Refused(class) => return Err(Halt::End(Ended::Refused(class))),
        Stop::Failed => return Err(Halt::End(Ended::Failed)),
        // The record goes in only once the pane is PROVEN back at its shell:
        // an audit line claiming a stop that did not finish would outlive
        // every refusal above it.
        Stop::Stopped(binary) => {
            record(
                dir,
                at,
                target,
                &format!("stopped {binary} in place (pane {})", target.pane),
                Mentions::NothingOfIt,
            );
            true
        }
        // Nothing to stop, so the stop gave the caller no last word: this is it.
        Stop::NotRunning if !hook() => return Err(Halt::End(Ended::Refused(CauseClass::Other))),
        Stop::NotRunning => false,
    };
    let broke = past_the_stop(stopped);
    let Some(before) = crate::seat_relaunch::prove_dead(dir, target, RESEAT_VERB, err)? else {
        return Err(Halt::End(broke));
    };
    Ok((before, broke))
}

/// What became of the conversation, past the dead proof.
struct Travel {
    carried: Carried,
    /// A carry was planned or a keep proven: the one path where ae has just
    /// PROVEN the start marker was there. Its own fact, never derived from
    /// [`Carried::travelled`]: a failed copy is Seeded and still planned.
    planned: bool,
    /// The keep probe found the conversation in its store.
    keep_on: bool,
    /// The slot's start marker, which `_run` reads to resume.
    marker: PathBuf,
}

/// DOES THE CONVERSATION TRAVEL? The keep probe and what it says, the marker
/// proven writable, then the copy and what it says — in that order.
fn conversation(
    seat: &Admitted<'_>,
    locked: &[u8],
    prepared: &Prepared,
    before: &Proven,
    broke: Ended,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<Travel, Halt> {
    let (dir, target, moving, prior) = (&seat.dir, &seat.target, &seat.moving, &prepared.prior);
    // Asked here, past the dead proof and before anything is removed, so a
    // refusal costs only the reading. Every arm that does not carry leaves the
    // rest of the move exactly as it was.
    let plan = carry_plan(locked, &target.slot, moving, prior, &before.work_dir);
    // A conversation to keep must still be in the tool's own store, asked by
    // the probe `_run` resumes with, keyed by the seat's recorded working copy.
    // When it is not, the move seeds and says what it could not keep.
    let kept = match moving.account.as_ref() {
        Some(account) if prepared.keeping => Some(
            if crate::run::resumable_in(
                moving.tool,
                prior,
                &crate::launch_cmd::Resolved::Path(account.path.clone()),
                Some(Path::new(&before.work_dir)),
            ) {
                Carried::Kept
            } else {
                let why = format!("not in its store at {}", account.path.display());
                writeln!(
                    out,
                    "could not keep conversation {prior} ({why}) — moving it on a fresh \
                     conversation with its seed pack instead"
                )?;
                Carried::Seeded(why)
            },
        ),
        _ => None,
    };
    let keep_on = kept == Some(Carried::Kept);
    let marker = crate::run::started_marker(dir, &target.slot);
    // THE RESUME MARKER, PROVEN WRITABLE BEFORE ANYTHING MOVES. `clear_slot`
    // below removes it and a carried seat has to have it back, or `_run` would
    // open a NEW conversation beside the store this is about to copy. The two
    // files cannot be one write, so the failure is taken HERE instead: nothing
    // has been removed, nothing copied, and the seat is exactly what it was.
    // Past this point the removal and the rewrite are two writes, and what can
    // separate them is a crash OR a rewrite that fails after the removal: the
    // clear arm below and the put-back arm both take that case, and both say
    // what they left. Re-running the same reseat converges — the source store
    // is untouched and the seat still records the conversation.
    if (plan.is_some() || keep_on)
        && let Err(why) = crate::launch::publish_data(&marker, b"")
    {
        writeln!(
            err,
            "Error: {why} — nothing was reseated and '{}' still records {}.",
            target.agent, seat.was
        )?;
        return Err(Halt::End(broke));
    }
    let planned = plan.is_some() || keep_on;
    let carried = match plan {
        None => kept.unwrap_or(Carried::No),
        Some(plan) => copy(&plan, prepared.checked.as_ref(), &target.agent, out, err)?,
    };
    Ok(Travel {
        carried,
        planned,
        keep_on,
        marker,
    })
}

/// The carry's COPY, with the proof its Check left, and what it says.
fn copy(
    plan: &crate::carry::Plan,
    checked: Option<&crate::carry::Crossing>,
    agent: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Carried> {
    let (from, to) = plan.homes();
    // A move whose Check never ran — its working copy was not usable
    // then — copies with an empty proof, which archives nothing.
    let unproven = crate::carry::Proof::default();
    let proof = checked.map_or(&unproven, crate::carry::Crossing::proof);
    Ok(
        match crate::carry::run(plan, crate::carry::Mode::Copy(proof)) {
            Ok(crossing) => {
                // RULING 1: typing the reseat with the other account's
                // profile IS the consent, so there is no prompt — but the
                // crossing is never silent. Said HERE rather than with the
                // final line because it is true from this moment: the
                // bytes are in the other account whatever the steps below
                // decide, and the source is untouched either way.
                writeln!(
                    out,
                    "carried conversation {} from {} to {}{}",
                    plan.id(),
                    from.display(),
                    to.display(),
                    if crossing.memory_kept {
                        " (the target already had a project memory; it was kept, never merged)"
                    } else {
                        ""
                    }
                )?;
                // WHAT IT ARCHIVED, said on the one line that names the
                // directory: the target account's own files were moved,
                // so the human is told where they are now.
                match crossing.archived() {
                    Some((count, dir)) => {
                        writeln!(
                            out,
                            "archived {} the target account held for this conversation in \
                         {} (moved aside, never deleted)",
                            crate::carry::files(count),
                            dir.display()
                        )?;
                        Carried::Archived {
                            count,
                            dir: dir.to_path_buf(),
                        }
                    }
                    None => Carried::Yes,
                }
            }
            Err(why) => {
                // RULING 4: the move still happens, on a fresh conversation
                // with the seed pack, and what failed is named.
                writeln!(
                    err,
                    "note: '{agent}' could not carry conversation {} from {} to {} ({why}) — \
                 moving it on a fresh conversation with its seed pack instead.",
                    plan.id(),
                    from.display(),
                    to.display()
                )?;
                Carried::Seeded(why.to_string())
            }
        },
    )
}

/// THE SEED PACK, BUILT FIRST, because the pack reads the seat's recorded
/// first message and the cleanup removes that file. A CARRIED seat gets
/// neither: the conversation itself travelled, so there is nothing for ae to
/// tell the successor and nothing for a human to re-send by hand. A KEPT
/// seat's pack is built too but published only if its resume falls back
/// ([`finish`]): this is the last moment its first message is readable.
fn build_seed(
    ask: &Ask<'_>,
    seat: &Admitted<'_>,
    travel: &Travel,
    broke: Ended,
    err: &mut impl Write,
) -> Result<Option<String>, Halt> {
    if travel.carried.fate() == Fate::Carried {
        return Ok(None);
    }
    let pack = match crate::seat_pack(
        seat.entry,
        &seat.dir,
        crate::doors::home().as_deref(),
        ask.now,
        &seat.target.agent,
    ) {
        Ok(pack) => crate::provenance::first_line(&crate::provenance::ctx(), &pack),
        Err(why) => {
            writeln!(err, "Error: {why}. Nothing was reseated.")?;
            return Err(Halt::End(broke));
        }
    };
    if !travel.keep_on
        && let Err(why) =
            crate::launch::publish_data(&seed_file(&seat.dir, &seat.target.agent), pack.as_bytes())
    {
        writeln!(err, "Error: {why}. Nothing was reseated.")?;
        return Err(Halt::End(broke));
    }
    Ok(Some(pack))
}

/// Every per-slot launch file goes, then a seat whose conversation travels
/// gets its start marker back.
fn clear_launch_files(
    seat: &Admitted<'_>,
    travel: &Travel,
    broke: Ended,
    err: &mut impl Write,
) -> Result<(), Halt> {
    let (target, carried, was) = (&seat.target, &travel.carried, &seat.was);
    let marker = &travel.marker;
    if let Err(why) = crate::run::clear_slot(&seat.dir, &target.slot) {
        // THE MARKER, BACK. `clear_slot` removes it FIRST and can fail on a
        // later file, and the meta has NOT moved — so the seat is still its old
        // self and still needs the marker its old self had. Only where a carry
        // was planned, because that is the one path where ae has just PROVEN
        // the marker was there; a seat that never started must not be handed
        // one it never had.
        let stranded = travel.planned && crate::launch::publish_data(marker, b"").is_err();
        let note = uncleared_note(stranded, carried.fate() == Fate::Carried);
        writeln!(
            err,
            "Error: the launch files of slot {} could not be cleared ({why}) — the seat was not \
             moved and '{}' still records {was}.{note}",
            target.slot, target.agent
        )?;
        return Err(Halt::End(broke));
    }
    // THE RESUME MARKER, PUT BACK. `clear_slot` removes it with the rest of the
    // slot's launch files, and `_run` reads exactly that file to decide between
    // creating a conversation and resuming one: without this the store would be
    // copied and the successor would then open a BRAND NEW conversation beside
    // it, which is the whole failure this slice exists to prevent. The write was
    // already proven possible before anything moved, so a failure here is a
    // transient one and is tried once more; a second failure REFUSES with the
    // meta unmoved, which is the seat's old self — it still records its old
    // profile, the SOURCE store is untouched and the copy already in the other
    // account is inert because no record names it, so re-running the same
    // reseat puts the marker back and carries again.
    if carried.travelled() {
        let mut back = crate::launch::publish_data(marker, b"");
        if back.is_err() {
            back = crate::launch::publish_data(marker, b"");
        }
        if let Err(why) = back {
            let copied = carried.fate() == Fate::Carried;
            writeln!(
                err,
                "Error: {why} — {}slot {} could not be marked as resuming, so the seat was not \
                 moved and '{}' still records {was}. Re-run the same reseat once {} can be \
                 written; until then relaunching '{}' would start a NEW conversation on its old \
                 account.{}",
                if copied {
                    "the conversation was copied but "
                } else {
                    ""
                },
                target.slot,
                target.agent,
                marker.display(),
                target.agent,
                if copied {
                    " The source account still holds the conversation; the copy in the other \
                     account is inert, since no record names it."
                } else {
                    ""
                }
            )?;
            return Err(Halt::End(broke));
        }
    }
    Ok(())
}

/// The meta rows move in ONE guarded replacement, then the seat is re-read:
/// the command, the tool and the conversation are the NEW profile's now, and
/// `start` pastes what `read_seat` composes from the moved rows.
fn publish_move(
    ask: &Ask<'_>,
    seat: &Admitted<'_>,
    travel: &Travel,
    prior: &str,
    before: Proven,
    broke: Ended,
    err: &mut impl Write,
) -> Result<Proven, Halt> {
    let (target, moving, carried) = (&seat.target, &seat.moving, &travel.carried);
    // ONE account feeds both the pair's identity and a carried conversation's
    // rows ([`seat_conversation`]): `moving.account`, which `resolve_pair`
    // required, so the client and the store it names cannot disagree.
    let pair = moving
        .pair
        .as_deref()
        .zip(moving.account.as_ref())
        .map(|(client, account)| crate::meta::PairIdentity {
            client,
            config_home: &account.row,
            config_home_base: account.base.as_deref(),
        });
    let conversation = if crate::launch::takes_launch_session_id(moving.tool) {
        crate::launch::generate_uuid()
    } else {
        crate::launch::PENDING.to_owned()
    };
    let moved = crate::meta::publish_seat_move(
        &seat.dir,
        &target.slot,
        &target.agent,
        &crate::meta::SeatMove {
            profile: &ask.parsed.profile,
            binary: &moving.binary,
            launch_id: &crate::session_launch::launch_token(moving.tool, None),
            conversation: seat_conversation(
                carried,
                moving.account.as_ref(),
                &conversation,
                ask.now,
            ),
            pair,
        },
    );
    if let Err(why) = moved {
        writeln!(
            err,
            "Error: the seat move could not be recorded ({}) — slot {} has no launch files now \
             and still records {}. Re-run the reseat.",
            why.cause(),
            target.slot,
            seat.was
        )?;
        return Err(Halt::End(broke));
    }
    // A pair starts on EXACTLY the command its gate judged — the global
    // definitions, never a project overlay's redefinition of either name —
    // as a spawn does. A bare move reads the seat as it always did.
    let snapshot = moving.pair.is_some().then_some(moving.command.as_str());
    match crate::run::read_seat(&seat.dir, &target.slot, snapshot) {
        Ok(read) => Ok(Proven {
            seat: read,
            agent_bin: moving.binary.clone(),
            work_dir: before.work_dir,
            id_before: if carried.travelled() {
                prior.to_owned()
            } else {
                conversation
            },
        }),
        Err(why) => {
            writeln!(
                err,
                "Error: {why}. '{}' already records profile '{}' and its pane is at a shell, so \
                 `relaunch {}` finishes the move once the profile reads.",
                target.agent, ask.parsed.profile, target.agent
            )?;
            Err(Halt::End(Ended::Failed))
        }
    }
}

/// The pasted tool was not seen: `_run` may have fallen back before its tool
/// failed — a fresh id proves that, and an unchanged one proves no resume
/// either.
fn not_seen(
    seat: &Admitted<'_>,
    profile: &str,
    seed: Option<&str>,
    at: &mut Record<'_>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Ended> {
    let (dir, target) = (&seat.dir, &seat.target);
    let unproven =
        at.carry == Carried::Kept && !resume_fell_back(dir, target, seed, at, None, out, err)?;
    let mentions = if unproven {
        Mentions::NothingOfIt
    } else {
        Mentions::Conversation
    };
    record(dir, at, target, NOT_SEEN, mentions);
    writeln!(
        err,
        "Error: '{}' moved to '{profile}' and its tool was not seen: look at the pane; \
         `relaunch {}` finishes the move once it is back at a shell.",
        target.agent, target.agent
    )?;
    Ok(Ended::Failed)
}

/// PURE: what a TOOL CHANGE says before anything happens, a dead pane
/// included — `None` when the seat records no tool or keeps the one it has.
/// The successor cannot read the old tool's store, and ae looks for no
/// conversation its records do not name.
fn tool_change_note(
    agent: &str,
    profile: &str,
    from_binary: &str,
    to_binary: &str,
    prior: &str,
) -> Option<String> {
    if from_binary.is_empty() || from_binary == to_binary {
        return None;
    }
    Some(format!(
        "note: '{agent}' records {from_binary} and '{profile}' runs {to_binary}: if this move \
         succeeds it starts a fresh {to_binary} conversation{}. ae searches for no conversation \
         its records do not name; any other stays in its own account.",
        if crate::session_launch::capture::is_lowercase_uuid(prior) {
            format!(" and its recorded conversation {prior} stays behind as a predecessor")
        } else {
            String::new()
        }
    ))
}

/// PURE: how a refusal PAST THE STOP ends. A tool this move stopped is left
/// stopped, which is something done; a seat that was already dead is exactly
/// as it was, which is a refusal.
const fn past_the_stop(stopped: bool) -> Ended {
    if stopped {
        Ended::Failed
    } else {
        Ended::Refused(CauseClass::Other)
    }
}

/// PURE: what an uncleared slot adds to its refusal. A marker that could not be
/// put back is said first, because it changes what the next start does; a
/// carry has already written in the other account, so "nothing was touched"
/// would not be true of it and the inert copy is SAID.
const fn uncleared_note(stranded: bool, copied: bool) -> &'static str {
    match (stranded, copied) {
        (true, _) => {
            " Its start marker could not be put back either, so its next start would open a \
             NEW conversation beside the one it records."
        }
        (false, true) => {
            " Its start marker is back; the copy already in the other account is inert, since \
             no record names it."
        }
        (false, false) => "",
    }
}

/// PURE: what the seat move publishes for the conversation. A kept one stays;
/// a carried one is published in the SAME replacement as the account rows
/// that name its store, because a conversation whose store no row names is
/// the window a later reader would resolve to the home the seat just left;
/// every other move opens `id` fresh from `now`.
///
/// A carry proves an account, so a carried fate with none is unreachable; it is
/// spelled rather than unwrapped, and it is today's move.
fn seat_conversation<'a>(
    carried: &Carried,
    account: Option<&'a Account>,
    id: &'a str,
    now: Timestamp,
) -> crate::meta::Conversation<'a> {
    match (carried, account) {
        (Carried::Kept, _) => crate::meta::Conversation::Kept,
        (Carried::Yes | Carried::Archived { .. }, Some(account)) => {
            crate::meta::Conversation::Carried {
                config_home: &account.row,
                config_home_base: account.base.as_deref(),
            }
        }
        _ => crate::meta::Conversation::Fresh {
            id,
            capture_floor: now.epoch(),
        },
    }
}

/// A KEPT conversation is proven only by the id `_run` left: one that fell back
/// recorded a fresh id and gets the seed pack, and nothing says kept. A proven
/// keep is named on stdout only with a `kept_line`, for a tool seen running.
fn resume_fell_back(
    dir: &Path,
    target: &Target,
    seed: Option<&str>,
    at: &mut Record<'_>,
    kept_line: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<bool> {
    let id_now = crate::lifecycle::meta_value(
        &crate::meta::read_bytes(dir).unwrap_or_default(),
        &format!("harness_session.{}", target.slot),
    );
    if id_now == at.prior {
        if let Some(change) = kept_line {
            writeln!(
                out,
                "kept conversation {} from profile '{}' to '{}'{change}",
                at.prior, at.from, at.to
            )?;
        }
        return Ok(false);
    }
    let file = seed_file(dir, &target.agent);
    writeln!(
        out,
        "could not keep conversation {} (resume fell back) — its seed pack is {}",
        at.prior,
        file.display()
    )?;
    at.carry = Carried::Seeded("resume fell back".to_owned());
    if let Some(pack) = seed
        && let Err(why) = crate::launch::publish_data(&file, pack.as_bytes())
    {
        writeln!(err, "note: the seed could not be kept on disk ({why}).")?;
    }
    Ok(true)
}

/// Everything past the lock: the tool's own launch turn, the seed turn, the
/// post-launch capture, the last identity check and the exit rule.
#[allow(
    clippy::too_many_arguments,
    reason = "the frozen helper's inputs, spelled out rather than bundled"
)]
fn finish(
    dir: &Path,
    target: &Target,
    proven: &Proven,
    resuming_seat: bool,
    parsed: &Parsed,
    seed: Option<&str>,
    at: &mut Record<'_>,
    change: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> io::Result<Ended> {
    let tool = proven.seat.tool;
    let seed = if at.carry == Carried::Kept
        && !resume_fell_back(dir, target, seed, at, Some(change), out, err)?
    {
        None
    } else {
        seed
    };
    // The tool's OWN launch turn first, where its adapter has one: codex's
    // rollout does not exist until a user turn, and the registration handshake
    // it carries must not be buried under a 24 KB seed.
    let prompt = crate::launch::initial_prompt_for(tool, dir, &target.slot);
    let launch_turn = if prompt.is_empty()
        || !crate::session_launch::launch_turn_is_pasted(tool, resuming_seat)
    {
        None
    } else {
        Some(crate::session_launch::deliver_launch_turn(
            dir,
            &target.server,
            &target.slot,
            &target.pane,
            tool,
            &prompt,
            crate::session_launch::OnHumanPrompt::FailFast,
            err,
        )?)
    };
    // THE SEED, as ae's own setup turn: `deliver_launch_turn` pastes its text
    // verbatim, so the marker is put on here by the ONE owner that spells it.
    // A CARRIED seat is handed none: it resumed the conversation itself, and a
    // pack recounting what it already remembers would be noise on its first
    // turn.
    let seed_turn = match seed {
        Some(seed) => Some(crate::session_launch::deliver_launch_turn(
            dir,
            &target.server,
            &target.slot,
            &target.pane,
            tool,
            seed,
            crate::session_launch::OnHumanPrompt::FailFast,
            err,
        )?),
        None => None,
    };
    // Only a slot that reads `pending` NOW needs capturing: `_run`'s own
    // in-pane fallback may have cleared it after the move.
    let id_after = crate::lifecycle::meta_value(
        &crate::meta::read_bytes(dir).unwrap_or_default(),
        &format!("harness_session.{}", target.slot),
    );
    if tool.adapter().capture.is_needed() && id_after == crate::launch::PENDING {
        crate::session_launch::capture::start(
            dir,
            &[crate::session_launch::capture::Target {
                slot: target.slot.clone(),
                tool,
                pane: target.pane.clone(),
            }],
        );
    }
    // LAST: the new tool may have died while the turns were delivered, and
    // exit 0 means the seat is up NOW.
    if !crate::seat_relaunch::observe_identity(target, &proven.agent_bin, true) {
        record(dir, at, target, STOPPED_AFTER, Mentions::Conversation);
        writeln!(
            err,
            "Error: '{}' started on '{}' and is gone again (pane {}) — look at the pane.",
            target.agent, parsed.profile, target.pane
        )?;
        return Ok(Ended::Failed);
    }
    let (launch_ok, launch_word) =
        launch_turn.map_or((true, ""), crate::seat_relaunch::turn_verdict);
    // A seat that was handed no seed has no seed verdict to report, and says
    // what it DID instead: the conversation came with it.
    let (seed_ok, seed_note) =
        seed_turn.map_or((true, format!(", {}", at.carry.fate().word())), |turn| {
            let (ok, word) = crate::seat_relaunch::turn_verdict(turn);
            (ok, format!(", seed {word}"))
        });
    let launch_note = if launch_word.is_empty() {
        String::new()
    } else {
        format!(", launch turn {launch_word}")
    };
    let line = format!(
        "{MOVED}{} (pane {}, slot {}) to {}{launch_note}{seed_note}",
        target.agent, target.pane, target.slot, parsed.profile
    );
    record(dir, at, target, &line, Mentions::Conversation);
    if !launch_ok || !seed_ok {
        let blocked =
            [launch_turn, seed_turn].contains(&Some(crate::session_launch::TurnOutcome::Blocked));
        write!(
            err,
            "{}",
            never_landed(dir, target, &line, blocked, seed.is_some())
        )?;
        return Ok(Ended::Failed);
    }
    writeln!(out, "{line}")?;
    Ok(Ended::Moved {
        fate: at.carry.fate(),
    })
}

/// What a move whose turn never landed tells the human. A turn a human-only
/// prompt blocked says so, and the next step starts with the answer only the
/// human can give; a kept seed is then re-sent by hand either way.
fn never_landed(dir: &Path, target: &Target, line: &str, blocked: bool, seeded: bool) -> String {
    let (why, first) = if blocked {
        (
            format!(
                ": pane {} waits on the prompt named above, which only the human may answer",
                target.pane
            ),
            format!("answer it in pane {}, then", target.pane),
        )
    } else {
        (String::new(), "the seat is up;".to_owned())
    };
    let head = format!("Error: {line} — a turn never landed{why}.\n");
    if !seeded {
        return head;
    }
    format!(
        "{head}  {first} send its seed by hand: {}/send {} \"$(cat {})\"\n",
        dir.display(),
        target.agent,
        seed_file(dir, &target.agent).display()
    )
}

/// What every `reseat` record carries besides its outcome: WHO asked, and the
/// move itself.
///
/// The move is on the record because the meta only keeps the CURRENT profile:
/// a later reader asking what this seat used to run has the predecessor's id
/// in the roster but nothing that names the profile it ran under, and the
/// event is the only place that pairing can live.
struct Record<'a> {
    caller: &'a Caller,
    now: Timestamp,
    from: &'a str,
    to: &'a str,
    prior: &'a str,
    /// What became of the conversation. The FACT, not a rendered word: it
    /// decides both what the record says happened AND what it calls the id it
    /// names, and two fields that had to agree would be a way for them not to.
    /// [`Carried::No`] writes what a tool change always wrote, byte for byte.
    /// OWNED, because the verdict is only reached after the stop has already
    /// written its own record through this.
    carry: Carried,
}

/// One `reseat` record for every attempt that REACHED the pane — the seat's
/// history is the only place a later reader can see that its TOOL changed, and
/// the only place that must not claim a paste that never was.
/// The event's own sentence. A row ae cannot prove reads `none`, and that
/// includes [`crate::launch::PENDING`]: it is the word for a conversation that
/// never resolved, `reseated` hands on no prior row for it, and an audit line
/// must not read as if one were being left behind.
///
/// The conversation id is filed under [`Carried::field`], which is the whole of
/// the difference: a carried conversation was not left behind, so calling it
/// `prior` in a durable record would be the opposite of what happened.
fn summary(
    outcome: &str,
    from: &str,
    to: &str,
    prior: &str,
    carry: &Carried,
    mentions: Mentions,
) -> String {
    let named = |value: &str| {
        if value.is_empty() || value == crate::launch::PENDING {
            "none".to_owned()
        } else {
            value.to_owned()
        }
    };
    let about = match mentions {
        Mentions::NothingOfIt => String::new(),
        Mentions::Conversation => {
            let word = carry.word();
            format!(
                ", {} {}{}",
                carry.field(),
                named(prior),
                if word.is_empty() {
                    String::new()
                } else {
                    format!(", {word}")
                }
            )
        }
    };
    format!("{outcome} [from {} to {to}{about}]", named(from))
}

fn record(dir: &Path, at: &Record<'_>, target: &Target, outcome: &str, mentions: Mentions) {
    let summary = summary(outcome, at.from, at.to, at.prior, &at.carry, mentions);
    let _ = crate::store::open(dir).append_event(&tracked::event_line(&EventFields {
        ts: at.now,
        // The caller's own display ref, exactly as `relaunch` records it: an
        // agent that moves a peer's seat must not appear in the audit trail as
        // the human, and a plain shell IS the human.
        actor: if at.caller.display.is_empty() {
            "human"
        } else {
            &at.caller.display
        },
        action: RESEAT_ACTION,
        target: &target.agent,
        reference: "",
        actor_slot: "",
        actor_session: "",
        target_slot: &target.slot,
        target_session: "",
        actor_session_id: "",
        target_session_id: "",
        target_server: "",
        target_pane: &target.pane,
        target_session_uuid: "",
        caller_server: "",
        caller_pane: at.caller.pane.as_deref().unwrap_or_default(),
        caller_session_uuid: "",
        identity_gap: "",
        summary: &summary,
        body_file: "",
    }));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn parsed(words: &[&str]) -> Result<(String, String, String), String> {
        super::parse(&argv(words)).map(|parsed| (parsed.session, parsed.agent, parsed.profile))
    }

    #[test]
    fn no_word_of_the_verb_reaches_the_legs_own_stop() {
        for word in ["--stop-quiet", "--quiet-start", "--quiet", "--stop-start"] {
            assert!(super::parse(&argv(&["work", "lead", "--using", "lunam", word])).is_err());
        }
        for words in [
            &["work", "lead", "--using", "lunam"][..],
            &["work", "lead", "--using", "lunam", "--stop-unknown"][..],
        ] {
            let stop = super::parse(&argv(words)).map(|parsed| parsed.stop);
            assert_ne!(stop, Ok(super::StopPolicy::QuietStart), "{words:?}");
        }
    }

    #[test]
    fn the_stop_flag_sits_anywhere_and_is_off_unless_it_is_typed() {
        // `--using`'s own rule, for the same reason: this is where a hand puts
        // a flag, and a seat must never be stopped by a spelling accident.
        for words in [
            &["work", "lead", "--using", "lunam", "--stop-unknown"][..],
            &["work", "--stop-unknown", "lead", "--using", "lunam"][..],
            &["--stop-unknown", "work", "lead", "--using", "lunam"][..],
        ] {
            let parsed = super::parse(&argv(words)).expect("a valid argv");
            assert_eq!(parsed.stop, super::StopPolicy::Unknown, "{words:?}");
            assert_eq!(parsed.agent, "lead", "{words:?}");
        }
        let plain = super::parse(&argv(&["work", "lead", "--using", "lunam"])).expect("valid");
        assert_eq!(plain.stop, super::StopPolicy::Proven, "the flag is opt-in");
        // And it is not a name: a seat called `--stop-unknown` cannot exist,
        // and the ladder must not swallow the word as one.
        assert_eq!(
            super::parse(&argv(&["work", "--stop-unknown", "--using", "lunam"]))
                .err()
                .as_deref(),
            Some(super::RESEAT_USAGE)
        );
    }

    #[test]
    fn a_caller_the_stop_cannot_place_is_never_cleared_to_kill() {
        use super::CallerStanding;
        use crate::procs::Proc;
        // Parentage is the whole question, so one helper builds the rows.
        let row = |pid, ppid| Proc {
            pid,
            ppid,
            comm: "x".to_owned(),
        };
        let rows = vec![row(10, 1), row(20, 10), row(30, 20), row(40, 1)];
        // The command runs under the seat's own tool: ending it would kill the
        // process asking.
        assert_eq!(
            super::caller_standing(Some(10), Some(&rows), 30),
            CallerStanding::Under
        );
        assert_eq!(
            super::caller_standing(Some(10), Some(&rows), 40),
            CallerStanding::Clear
        );
        // Every gap refuses, NAMED, because ae may not end a tool it cannot
        // prove is not its own parent — a missing row included.
        let (full, broken) = (&rows[..], &[row(10, 1), row(50, 60)][..]);
        for (pane, table, me, gap) in [
            (None, Some(full), 40, "no pid for the pane"),
            (Some(10), None, 40, "table could not be read"),
            (Some(99), Some(full), 40, "pane's own process (pid 99)"),
            (Some(10), Some(full), 77, "own process (pid 77) is not"),
            (Some(10), Some(broken), 50, "ancestry breaks at pid 60"),
        ] {
            let standing = super::caller_standing(pane, table, me);
            assert!(
                matches!(&standing, CallerStanding::Unprovable(why) if why.contains(gap)),
                "{gap}: {standing:?}"
            );
        }
    }

    #[test]
    fn a_timed_out_stop_tells_a_human_which_of_the_two_states_the_pane_is_in() {
        use crate::tmux::ObservedPaneProbe;
        let probe = |command: &str| ObservedPaneProbe {
            command: command.to_owned(),
            pid: Some(10),
        };
        // The respawn took and the shell is busy: waiting is what helps.
        let (holding, next) = super::stop_timeout_advice(Some(&probe("zsh")));
        assert!(holding.contains("under it"), "{holding}");
        assert!(next.contains("re-run"), "{next}");
        // Something else came back in the pane. Re-running cannot change that,
        // so the advice must not send a human round the same loop.
        let (holding, next) = super::stop_timeout_advice(Some(&probe("sleep")));
        assert!(holding.contains("'sleep'"), "{holding}");
        assert!(next.contains("will not help"), "{next}");
        let (holding, next) = super::stop_timeout_advice(None);
        assert!(holding.contains("stopped answering"), "{holding}");
        assert!(next.contains("look at the pane"), "{next}");
    }

    #[test]
    fn a_pane_is_back_at_a_shell_only_when_both_readings_prove_it() {
        use crate::procs::Proc;
        use crate::tmux::ObservedPaneProbe;
        let shell = |pid| ObservedPaneProbe {
            command: "zsh".to_owned(),
            pid,
        };
        let empty: Vec<Proc> = Vec::new();
        assert!(super::pane_back_at_shell(&shell(Some(10)), Some(&empty)));
        // A process under the shell is not an idle shell: the seat's tool may
        // be dying, or a human may have started something.
        let busy = vec![Proc {
            pid: 20,
            ppid: 10,
            comm: "vim".to_owned(),
        }];
        assert!(!super::pane_back_at_shell(&shell(Some(10)), Some(&busy)));
        // The tool still holds the foreground.
        assert!(!super::pane_back_at_shell(
            &ObservedPaneProbe {
                command: "codex".to_owned(),
                pid: Some(10),
            },
            Some(&empty)
        ));
        // EVERY gap is false, so the bounded wait times out and refuses rather
        // than calling a pane it could not read idle.
        assert!(!super::pane_back_at_shell(&shell(None), Some(&empty)));
        assert!(!super::pane_back_at_shell(&shell(Some(10)), None));
    }

    #[test]
    fn a_tree_running_a_harness_the_seat_does_not_record_is_never_stopped() {
        use crate::procs::Proc;
        use crate::seat_relaunch::{Recorded, Target};
        let row = |pid, ppid, comm: &str| Proc {
            pid,
            ppid,
            comm: comm.to_owned(),
        };
        let target = Target {
            agent: "colead".to_owned(),
            slot: "main".to_owned(),
            pane: "%68".to_owned(),
            server: crate::inventory::ServerId::Ambient,
        };
        let codex = Recorded {
            binary: "codex",
            profile: "opus5x",
        };
        // The recorded tool, its own helpers, and a harness under ANOTHER pane:
        // the tree is the seat's alone, so the stop goes on exactly as before.
        let own = vec![
            row(100, 1, "sh"),
            row(200, 100, "/opt/bin/codex"),
            row(210, 200, "node"),
            row(300, 1, "claude"),
        ];
        assert_eq!(super::mixed_refusal(&own, 100, &codex, &target), None);
        // A foreign harness anywhere in the tree refuses, naming each by tool,
        // pid and parent, and the records beside them.
        let mixed = vec![
            row(100, 1, "sh"),
            row(200, 100, "/opt/bin/codex"),
            row(210, 200, "/Users/x/.local/bin/claude"),
            row(220, 100, "gemini"),
        ];
        let line = super::mixed_refusal(&mixed, 100, &codex, &target).unwrap_or_default();
        for part in [
            "'colead' runs its recorded codex (profile 'opus5x') in pane %68",
            "claude (pid 210, parent 200), gemini (pid 220, parent 100)",
            "nothing was stopped",
            "Wait for it to exit, or end it yourself, then re-run the reseat",
        ] {
            assert!(line.contains(part), "{part:?} missing: {line}");
        }
        // A harness ae cannot place outside the tree is not proven absent.
        let unplaced = [row(100, 1, "sh"), row(900, 850, "claude")];
        let line = super::mixed_refusal(&unplaced, 100, &codex, &target).unwrap_or_default();
        assert!(
            line.contains(
                "claude (pid 900, which ae cannot place: its ancestry breaks at pid 850)"
            )
        );
        // The pane's OWN process is its tree too.
        assert!(
            super::mixed_refusal(&[row(100, 1, "grok")], 100, &codex, &target)
                .is_some_and(|line| line.contains("grok (pid 100, parent 1)"))
        );
    }

    #[test]
    fn a_frame_grammar_the_classifier_does_not_own_is_never_read_as_idle() {
        use crate::harness_state::{HarnessState, classify};
        use crate::tool::ToolKind;
        // Muse's adapter declares the BorderDelimited input model, which
        // routes it to CLAUDE's frame grammar — a grammar muse does not draw.
        // The stop therefore may never take a muse frame for an idle claude
        // one. Measured muse capture, plain, 80x24, its composer genuinely
        // idle: the row above the prompt is muse's own `── Voice input …`
        // rule, which is not a claude border, so the frame fails closed.
        let idle_muse =
            include_str!("../tests/fixtures/runtime-identity/muse-idle-plain-80x24.txt");
        assert_eq!(classify(idle_muse, ToolKind::Muse), HarnessState::Unknown);
        // An unmodelled tool has no grammar at all, and an empty capture is
        // the shape a failed read takes.
        assert_eq!(
            classify(idle_muse, ToolKind::OpenCode),
            HarnessState::Unknown
        );
        assert_eq!(classify("", ToolKind::Claude), HarnessState::Unknown);
    }

    #[test]
    fn a_conversation_that_never_resolved_is_nothing_to_hand_on() {
        // The seat moved, and what it leaves behind is a word rather than an
        // id. The meta is already right — `reseated` writes no prior row it
        // cannot prove — and the audit line has to say the same thing.
        assert_eq!(
            super::summary(
                "reseated",
                "a",
                "b",
                crate::launch::PENDING,
                &super::Carried::No,
                super::Mentions::Conversation
            ),
            "reseated [from a to b, prior none]"
        );
        assert_eq!(
            super::summary(
                "reseated",
                "",
                "b",
                "11111111-1111-4111-8111-111111111111",
                &super::Carried::No,
                super::Mentions::Conversation
            ),
            "reseated [from none to b, prior 11111111-1111-4111-8111-111111111111]"
        );
        assert_eq!(
            super::Carried::Seeded("no transcript".to_owned()).word(),
            "seeded (no transcript)"
        );
    }

    /// The console reads a reseat's own words to know a seat moved after an
    /// ask: every record written past the meta move says so, the stop record
    /// written before it does not.
    #[test]
    fn only_a_record_written_after_the_move_says_the_seat_moved() {
        let record = |outcome: &str| {
            let (carry, mentions) = (super::Carried::No, super::Mentions::Conversation);
            super::summary(outcome, "a", "b", "", &carry, mentions)
        };
        let started = format!("{}lead (pane %1, slot main) to b", super::MOVED);
        for outcome in [started.as_str(), super::NOT_SEEN, super::STOPPED_AFTER] {
            assert!(super::moved_seat(&record(outcome)), "{outcome}");
        }
        for outcome in ["stopped claude in place (pane %1)", "reseat", ""] {
            assert!(!super::moved_seat(&record(outcome)), "{outcome}");
        }
        assert!(super::Carried::No.word().is_empty());
    }

    #[test]
    fn a_carried_conversation_is_never_recorded_as_the_one_left_behind() {
        const ID: &str = "11111111-1111-4111-8111-111111111111";
        // THE CARRY ARM. The id names the conversation that CONTINUES in the
        // other account, so the record must not file it under the word for what
        // a move abandoned. A reader scanning records for a dead conversation
        // would otherwise find a live one and act on it.
        assert_eq!(
            super::summary(
                "reseated",
                "a",
                "b",
                ID,
                &super::Carried::Yes,
                super::Mentions::Conversation
            ),
            format!("reseated [from a to b, conversation {ID}, carried]")
        );
        // THE OTHER ARM of the same question. A seeded move really did leave
        // that conversation behind — a fresh one and a seed pack took its
        // place — so `prior` is the truth there and stays.
        assert_eq!(
            super::summary(
                "reseated",
                "a",
                "b",
                ID,
                &super::Carried::Seeded("no transcript".to_owned()),
                super::Mentions::Conversation
            ),
            format!("reseated [from a to b, prior {ID}, seeded (no transcript)]")
        );
        // A move that was never a carry question at all records what a tool
        // change has always recorded, byte for byte.
        assert_eq!(
            super::summary(
                "reseated",
                "a",
                "b",
                ID,
                &super::Carried::No,
                super::Mentions::Conversation
            ),
            format!("reseated [from a to b, prior {ID}]")
        );
        // THE STOP names no conversation at all, whatever the fate field would
        // have said: ending a tool in a pane is not an event about a
        // conversation, and this one is written BEFORE the carry verdict, so
        // any word here would be a guess.
        for fate in [
            super::Carried::No,
            super::Carried::Yes,
            super::Carried::Seeded("no transcript".to_owned()),
        ] {
            assert_eq!(
                super::summary(
                    "stopped claude in place (pane %0)",
                    "a",
                    "b",
                    ID,
                    &fate,
                    super::Mentions::NothingOfIt
                ),
                "stopped claude in place (pane %0) [from a to b]"
            );
        }
    }

    #[test]
    fn the_flag_sits_anywhere_and_every_other_dash_word_is_refused() {
        let want = Ok(("work".to_owned(), "lead".to_owned(), "lunam".to_owned()));
        // Where a hand puts it: after the names, between them, in front, and
        // in the `=` spelling a shell completion produces.
        for words in [
            &["work", "lead", "--using", "lunam"][..],
            &["work", "--using", "lunam", "lead"][..],
            &["--using", "lunam", "work", "lead"][..],
            &["work", "lead", "--using=lunam"][..],
        ] {
            assert_eq!(parsed(words), want, "{words:?}");
        }
        // A dash word is never a name: swallowing one would reseat a seat the
        // caller did not type.
        assert_eq!(
            parsed(&["work", "-f", "lead", "--using", "lunam"]),
            Err("Error: unknown flag '-f'.".to_owned())
        );
    }

    #[test]
    fn an_incomplete_argv_names_what_is_missing() {
        assert_eq!(parsed(&[]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(parsed(&["work"]), Err(super::RESEAT_USAGE.to_owned()));
        assert_eq!(
            parsed(&["work", "lead", "extra", "--using", "lunam"]),
            Err(super::RESEAT_USAGE.to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead"]),
            Err("Error: reseat needs --using <profile>.".to_owned())
        );
        assert_eq!(
            parsed(&["work", "lead", "--using"]),
            Err("Error: --using needs a profile name.".to_owned())
        );
    }

    #[test]
    fn a_client_override_splits_into_the_bare_profile_and_its_label() {
        // The grammar a launch `--seat` and a spawn use: one `@` splits, every
        // later row carries the bare profile, and a malformed value is refused
        // naming it.
        let split = |words: &[&str]| {
            super::parse(&argv(words)).map(|parsed| (parsed.profile, parsed.client))
        };
        assert_eq!(
            split(&["work", "lead", "--using", "lunam@cc-mic"]),
            Ok(("lunam".to_owned(), Some("cc-mic".to_owned())))
        );
        assert_eq!(
            split(&["work", "lead", "--using=lunam"]),
            Ok(("lunam".to_owned(), None))
        );
        for bad in ["lunam@", "@cc-mic", "lunam@cc@mic"] {
            let refusal = split(&["work", "lead", "--using", bad]).expect_err("malformed");
            assert!(
                refusal.contains(&format!("'{bad}' is not <profile> or <profile>@<client>")),
                "{refusal}"
            );
        }
    }

    /// The nothing-to-move half beside the profile compare: a bare request
    /// asks for the profile's own account, a pair for exactly its label, and
    /// only a USABLE recorded label is somewhere else.
    #[test]
    fn a_same_profile_move_is_nothing_only_on_the_client_already_recorded() {
        use crate::meta::RecordedClient::{Invalid, Label, Missing};
        let label = Label("cc-mic".to_owned());
        assert!(super::same_client(None, &Missing));
        assert!(super::same_client(None, &Invalid));
        assert!(!super::same_client(None, &label));
        assert!(super::same_client(Some("cc-mic"), &label));
        assert!(!super::same_client(Some("cc-p"), &label));
        assert!(!super::same_client(Some("cc-mic"), &Missing));
        assert!(!super::same_client(Some("cc-mic"), &Invalid));
    }

    #[test]
    fn the_seed_is_published_beside_the_meta_under_the_seats_own_name() {
        // Named for the SEAT, not the slot: it is what a human `cat`s when a
        // turn did not land, and the refusal that says so prints this path.
        assert_eq!(
            super::seed_file(Path::new("/s/work"), "colead"),
            Path::new("/s/work/seed.colead.md")
        );
        // A name that is not a filename component cannot escape the session
        // directory, whatever put it in the meta or the pane option. The dot
        // SURVIVES, because a slot is `spawned.0` and the sanitiser is shared;
        // what cannot survive is the separator, so `..` is inert here.
        let escaped = super::seed_file(Path::new("/s/work"), "../../etc/x");
        assert_eq!(escaped, Path::new("/s/work/seed..._.._etc_x.md"));
        assert_eq!(escaped.parent(), Some(Path::new("/s/work")));
    }

    /// The two failure texts. A blocked turn names the pane and the answer
    /// first; an unblocked one keeps the plain hand-send line; a move with no
    /// kept seed prints no hand-send at all.
    #[test]
    fn a_turn_a_human_prompt_blocked_starts_its_next_step_with_the_answer() {
        let target = crate::seat_relaunch::Target {
            agent: "scout".to_owned(),
            slot: "spawned.0".to_owned(),
            pane: "%7".to_owned(),
            server: crate::inventory::ServerId::Ambient,
        };
        let dir = Path::new("/s/work");
        let send = "send its seed by hand: /s/work/send scout \"$(cat /s/work/seed.scout.md)\"";
        assert_eq!(
            super::never_landed(dir, &target, "moved", true, true),
            format!(
                "Error: moved — a turn never landed: pane %7 waits on the prompt named above, \
                 which only the human may answer.\n  answer it in pane %7, then {send}\n"
            )
        );
        assert_eq!(
            super::never_landed(dir, &target, "moved", false, true),
            format!("Error: moved — a turn never landed.\n  the seat is up; {send}\n")
        );
        assert_eq!(
            super::never_landed(dir, &target, "moved", true, false),
            "Error: moved — a turn never landed: pane %7 waits on the prompt named above, \
             which only the human may answer.\n"
        );
    }

    /// The limit proof reads ONE capture two ways, and each half is pinned on
    /// its own: the vendor's own limit row, and the tool's own grammar
    /// recognising its input box — which it does only while the box is empty.
    /// A busy frame still proves the limit; the stop refuses it before asking.
    #[test]
    fn the_limit_is_proven_only_by_the_vendors_row_over_an_empty_box() {
        const LIMIT: &str = "❯ a synthetic prompt\n  ⎿  You've hit your session limit · resets 3pm\n     \
             /upgrade to increase your usage limit.\n\n";
        const RULE: &str = "────────────────────────────────────────────────────────────";
        let chrome = |prompt: &str| {
            format!(
                "{RULE}\n{prompt}\n{RULE}\n  🧠 Opus 5.5 (xhigh)  📁 repo  🌿 main\n  \
                 ⏵⏵ bypass permissions on (shift+tab to cycle)"
            )
        };
        let codex = "■ You’ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage \
             to purchase more\ncredits or try again at Sep 26th, 2026 10:11 AM.\n\n\n› Ask Codex to \
             do anything\n\n  gpt-6-astra medium · ~/projects/ae · main · Context 51% used";
        for (capture, bin, limit, frame, proven) in [
            (
                format!("{LIMIT}{}", chrome("❯")),
                "claude",
                true,
                true,
                true,
            ),
            (
                format!("{LIMIT}{}", chrome("❯ half a sentence")),
                "claude",
                true,
                false,
                false,
            ),
            (
                format!("⏺ ok\n\n{}", chrome("❯")),
                "claude",
                false,
                true,
                false,
            ),
            (
                format!(
                    "{LIMIT}✳ Thinking… (3s · esc to interrupt)\n{}",
                    chrome("❯")
                ),
                "claude",
                true,
                true,
                true,
            ),
            (codex.to_owned(), "codex", true, true, true),
            (
                format!("{LIMIT}{}", chrome("❯")),
                "muse",
                false,
                true,
                false,
            ),
        ] {
            let tool = super::ToolKind::from_binary_name(bin);
            assert_eq!(
                crate::watchdog::limit_notice(&capture, bin).is_some(),
                limit,
                "{bin}: the limit row\n{capture}"
            );
            assert_eq!(
                crate::harness_state::read_frame(&capture, tool).is_some(),
                frame,
                "{bin}: the box\n{capture}"
            );
            assert_eq!(
                super::limit_proven(&capture, bin, tool),
                proven,
                "{bin}\n{capture}"
            );
        }
    }

    /// Each proof must hold on BOTH readings: a seat that reads idle or
    /// limited once and anything else the second time may have been handed
    /// work in between. Only `--stop-unknown` stops without a proof.
    #[test]
    fn a_stop_acts_only_on_a_proof_both_readings_agree_on() {
        use super::{HarnessState::Idle, HarnessState::Unknown, StopPolicy};
        for (first, second, proven) in [
            ((Idle, false), (Idle, false), true),
            ((Idle, false), (Unknown, false), false),
            ((Unknown, false), (Idle, false), false),
            ((Unknown, true), (Unknown, true), true),
            ((Unknown, true), (Unknown, false), false),
            ((Unknown, false), (Unknown, true), false),
            ((Unknown, false), (Unknown, false), false),
        ] {
            assert_eq!(
                super::may_stop(first, second, StopPolicy::Proven),
                proven,
                "{first:?} then {second:?}"
            );
            assert!(
                super::may_stop(first, second, StopPolicy::Unknown),
                "{first:?} then {second:?} under --stop-unknown"
            );
        }
    }

    /// An unknown verdict proves the limit when only blank rows, an echo and the
    /// vendor's limit rows follow the cell, never over a work row, and never
    /// over a nearest row that is neither the cell nor such furniture.
    #[test]
    fn a_limit_over_an_echo_is_proven_and_over_work_is_not() {
        const LIMITED: &str =
            include_str!("../tests/fixtures/harness-state/claude-limit-2.1.296-180x50.txt");
        const DONE: &str = "✻ Baked for 0s · done 10:23 AM";
        let tool = super::ToolKind::Claude;
        for (below, proven) in [
            (DONE, true),
            ("", true),
            ("❯ please continue", true),
            ("❯ first\n  second", true),
            ("⏺ Running a tool", false),
            ("❯ one\n\n⏺ Running a tool", false),
            ("✻ Sautéed for 6s", false),
        ] {
            let frame = LIMITED.replacen(DONE, below, 1);
            assert_eq!(
                super::limit_proven(&frame, "claude", tool),
                proven,
                "{below}"
            );
        }
    }

    /// The refusal's first clause names the first reading that proves nothing;
    /// readings that each prove, but not alike, say the frame changed.
    #[test]
    fn a_refusal_names_the_first_reading_that_proves_nothing() {
        use super::{HarnessState::Idle, HarnessState::Unknown, refusal_why};
        const LIMITED: &str =
            include_str!("../tests/fixtures/harness-state/claude-limit-2.1.296-180x50.txt");
        const IDLE: &str = include_str!("../tests/fixtures/harness-state/claude-idle-167x40.txt");
        let tool = super::ToolKind::Claude;
        let working = LIMITED.replace("✻ Baked for 0s · done 10:23 AM", "⏺ a \"tool\" row");
        for (first, second, why) in [
            // No capture, a box that reads no frame: nothing to prove.
            ((Unknown, false, None), (Idle, false, None), "no frame"),
            (
                (Unknown, false, Some("not a claude frame")),
                (Unknown, false, Some(IDLE)),
                "no frame",
            ),
            // A frame with no limit cell in view.
            (
                (Unknown, false, Some(IDLE)),
                (Idle, false, None),
                "no limit row",
            ),
            // A row after the cell: quoted, its double quotes removed.
            (
                (Unknown, false, Some(working.as_str())),
                (Unknown, false, Some(IDLE)),
                "row after the limit row: \"⏺ a tool row\"",
            ),
            // The FIRST failing reading decides, not the second.
            (
                (Idle, false, None),
                (Unknown, false, Some(working.as_str())),
                "row after the limit row: \"⏺ a tool row\"",
            ),
            // Each reading proves on its own, in different ways.
            (
                (Idle, false, None),
                (Unknown, true, None),
                "the frame changed between the two readings",
            ),
            (
                (Unknown, true, None),
                (Idle, false, None),
                "the frame changed between the two readings",
            ),
        ] {
            assert_eq!(
                refusal_why(first, second, "claude", tool),
                why,
                "{first:?} then {second:?}"
            );
        }
    }

    /// The measured real frame must justify the limit stop on BOTH captures.
    /// One limited frame beside an unreadable, unproven frame cannot do so.
    #[test]
    fn limitstop_the_real_limit_requires_two_proven_captures() {
        const REAL: &str =
            include_str!("../tests/fixtures/harness-state/claude-limit-2.1.296-180x50.txt");
        let limited = (
            crate::harness_state::classify(REAL, super::ToolKind::Claude),
            super::limit_proven(REAL, "claude", super::ToolKind::Claude),
        );
        let unread = (super::HarnessState::Unknown, false);
        assert!(
            super::may_stop(limited, limited, super::StopPolicy::Proven),
            "two real terminal usage-limit captures must authorize the proven stop"
        );
        assert!(
            !super::may_stop(limited, unread, super::StopPolicy::Proven),
            "a limit only in the first read must refuse"
        );
        assert!(
            !super::may_stop(unread, limited, super::StopPolicy::Proven),
            "a limit only in the second read must refuse"
        );
    }

    #[test]
    fn limitstop_measured_cancelled_continue_is_finished_only_when_current() {
        const CANCELLED: &str = include_str!(
            "../tests/fixtures/harness-state/claude-limit-cancelled-2.1.296-180x50.txt"
        );
        let current = (
            crate::harness_state::classify(CANCELLED, super::ToolKind::Claude),
            super::limit_proven(CANCELLED, "claude", super::ToolKind::Claude),
        );
        assert!(
            super::may_stop(current, current, super::StopPolicy::Proven),
            "measured cancelled continuation with an empty box is a finished limit on both reads"
        );
        let historical = CANCELLED.replace(
            "⏺ Automatic continue cancelled · /rate-limit-options to re-arm",
            "⏺ Automatic continue cancelled · /rate-limit-options to re-arm\n\n⏺ Running a tool after the earlier limit",
        );
        assert_eq!(
            crate::harness_state::classify(&historical, super::ToolKind::Claude),
            super::HarnessState::Unknown,
            "newest current row is unproven"
        );
        assert!(
            crate::watchdog::limit_notice(&historical, "claude").is_some(),
            "limit still visible"
        );
        let active = (
            super::HarnessState::Unknown,
            super::limit_proven(&historical, "claude", super::ToolKind::Claude),
        );
        assert!(
            !super::may_stop(active, active, super::StopPolicy::Proven),
            "a historical cancelled row above an unproven current turn cannot authorize stopping"
        );
        assert!(
            !super::may_stop(current, active, super::StopPolicy::Proven),
            "both reads must prove finished"
        );
        assert!(
            !super::may_stop(active, current, super::StopPolicy::Proven),
            "second read cannot repair the first"
        );
    }

    /// Claude's `/rate-limit-options` menu replaces the box, so the frame
    /// grammar reads nothing there: the menu as the live bottom, with no turn
    /// drawn above its rule, is the limit proof on both reads, and a menu
    /// that is only quoted above a drawn box, or sits under a running turn,
    /// proves nothing.
    #[test]
    fn a_claude_limit_menu_is_a_limit_proof_only_as_the_live_bottom_with_no_turn_above() {
        const MENU: &str =
            include_str!("../tests/fixtures/claude-limit-menu/claude-limit-menu-39col.txt");
        let claude = super::ToolKind::Claude;
        assert_eq!(
            crate::harness_state::read_frame(MENU, claude),
            None,
            "the menu draws no box: only the menu reading can prove this frame"
        );
        let read = (
            crate::harness_state::classify(MENU, claude),
            super::limit_proven(MENU, "claude", claude),
        );
        assert_eq!(read, (super::HarnessState::Unknown, true));
        assert!(
            super::may_stop(read, read, super::StopPolicy::Proven),
            "the measured menu is a finished limit on both reads"
        );

        let busy = include_str!("../tests/fixtures/harness-state/claude-busy-167x40.txt")
            .lines()
            .find(|row| row.starts_with("✻ Enchanting"))
            .unwrap_or_default();
        assert!(!busy.is_empty(), "the measured busy row");
        let running = MENU.replace("✻ Sautéed for 6s · done Tuesday 3:03 PM", busy);
        assert!(
            crate::watchdog::limit_notice(&running, "claude").is_some(),
            "still the limit"
        );
        assert!(
            !super::limit_proven(&running, "claude", claude),
            "a turn above the dialog's rule is no proof"
        );
        let quoted = format!(
            "{MENU}\n────────────────────────────────────────\n❯\u{a0}\n\
             ────────────────────────────────────────\n  🧠 Opus 5.5 (xhigh)\n  ⏵⏵ accept edits on\n"
        );
        assert!(
            !super::limit_proven(&quoted, "claude", claude),
            "a menu quoted above a drawn box is no proof"
        );
        for bin in ["codex", "muse", "opencode"] {
            let tool = super::ToolKind::from_binary_name(bin);
            assert!(!super::limit_proven(MENU, bin, tool), "{bin}");
        }
    }

    fn limitstop_shell_probe(
        clean: bool,
    ) -> (
        crate::tmux::ObservedPaneProbe,
        Option<Vec<crate::procs::Proc>>,
    ) {
        use crate::procs::Proc;
        let mut table = vec![Proc {
            pid: 10,
            ppid: 1,
            comm: "fish".to_owned(),
        }];
        if !clean {
            table.push(Proc {
                pid: 20,
                ppid: 10,
                comm: "startup-hook".to_owned(),
            });
        }
        (
            crate::tmux::ObservedPaneProbe {
                command: "fish".to_owned(),
                pid: Some(10),
            },
            Some(table),
        )
    }

    /// The production wait consumes actual probes/tables through its source,
    /// with a deterministic clock. `None` is a FAILED pane probe, not a child.
    fn limitstop_wait(points: &[(u64, Option<bool>)]) -> (bool, usize, usize) {
        let ticks = std::cell::Cell::new(0);
        let reads = std::cell::Cell::new(0);
        let complete = super::wait_back_at_shell(
            || {
                let at = ticks.get();
                assert!(
                    at < 50,
                    "production wait must stop before a fifty-first poll"
                );
                ticks.set(at + 1);
                std::time::Duration::from_millis(points.get(at).map_or(10_000, |point| point.0))
            },
            || {
                let at = reads.get();
                reads.set(at + 1);
                points
                    .get(at)
                    .and_then(|point| point.1)
                    .map(limitstop_shell_probe)
            },
        );
        (complete, ticks.get(), reads.get())
    }

    #[test]
    fn limitstop_production_wait_resets_after_a_failed_probe() {
        let points = [
            (200, Some(true)),
            (400, None),
            (600, Some(true)),
            (800, Some(true)),
            (1000, Some(true)),
            (1200, Some(true)),
            (1400, Some(true)),
            (1600, Some(true)),
        ];
        assert_eq!(
            limitstop_wait(&points),
            (true, 8, 8),
            "failed probe breaks quiet; production wait consumes one second after fresh clean probe"
        );
    }

    #[test]
    fn limitstop_production_wait_is_bounded_and_does_not_take_the_first_clean_probe() {
        let points: Vec<_> = (1..=super::STOP_POLLS)
            .map(|poll| (u64::from(poll) * 200, Some(poll == 1)))
            .collect();
        assert_eq!(
            limitstop_wait(&points),
            (false, 50, 50),
            "production wait refuses at its ten-second bound despite an early clean poll"
        );
        assert_eq!(points.last().map(|point| point.0), Some(10_000));
    }

    #[test]
    fn limitstop_timeout_advice_keeps_the_other_two_branches() {
        let missing = super::stop_timeout_advice(None);
        assert_eq!(missing.0, "the pane stopped answering");
        assert_eq!(missing.1, "look at the pane before trying again.");
        let (mut probe, _) = limitstop_shell_probe(true);
        probe.command = "worker".to_owned();
        let running = super::stop_timeout_advice(Some(&probe));
        assert_eq!(running.0, "'worker' holds its foreground");
        assert_eq!(
            running.1,
            "look at the pane: the respawn did not leave a shell, so re-running will not help."
        );
    }

    /// Deterministic process-table readings, not sleeps or mocked stop results.
    /// A false item presents a real descendant beneath the fresh fish shell.
    fn limitstop_shell_readings(points: &[(u64, bool)]) -> Vec<(std::time::Duration, bool)> {
        use crate::procs::Proc;
        let probe = crate::tmux::ObservedPaneProbe {
            command: "fish".to_owned(),
            pid: Some(10),
        };
        points
            .iter()
            .map(|&(millis, clean)| {
                let mut table = vec![Proc {
                    pid: 10,
                    ppid: 1,
                    comm: "fish".to_owned(),
                }];
                if !clean {
                    table.push(Proc {
                        pid: 20,
                        ppid: 10,
                        comm: "startup-hook".to_owned(),
                    });
                }
                (
                    std::time::Duration::from_millis(millis),
                    super::pane_back_at_shell(&probe, Some(&table)),
                )
            })
            .collect()
    }

    #[test]
    fn limitstop_a_child_at_the_fourth_probe_resets_the_quiet_window() {
        let readings = limitstop_shell_readings(&[
            (0, true),
            (200, true),
            (400, true),
            (600, false),
            (800, true),
            (1000, true),
            (1200, true),
            (1400, true),
            (1600, true),
            (1800, true),
        ]);
        assert_eq!(
            super::settled(readings),
            Some(9),
            "a startup child at probe four resets quiet; settle only one second after it disappears"
        );
    }

    #[test]
    fn limitstop_quiet_time_is_measured_from_the_first_clean_probe() {
        assert_eq!(
            super::settled(limitstop_shell_readings(&[(0, true), (999, true)])),
            None,
            "999 milliseconds of observed quiet cannot settle the shell"
        );
    }

    #[test]
    fn limitstop_exactly_one_second_of_observed_quiet_settles() {
        assert_eq!(
            super::settled(limitstop_shell_readings(&[
                (0, true),
                (999, true),
                (1000, true)
            ])),
            Some(2),
            "exactly one second of uninterrupted quiet settles the shell"
        );
    }

    #[test]
    fn limitstop_bound_exhaustion_with_a_startup_child_cannot_settle() {
        let points: Vec<_> = (1..=super::STOP_POLLS)
            .map(|poll| (u64::from(poll) * 200, poll == 1))
            .collect();
        assert_eq!(
            super::settled(limitstop_shell_readings(&points)),
            None,
            "the ten-second bound ends without success when startup never becomes quiet"
        );
        let probe = crate::tmux::ObservedPaneProbe {
            command: "fish".to_owned(),
            pid: Some(10),
        };
        assert_eq!(
            super::stop_timeout_advice(Some(&probe)),
            (
                "its shell is back but something still runs under it".to_owned(),
                "re-run the reseat once that finishes."
            ),
            "a timeout keeps the existing actionable refusal"
        );
    }

    // Frozen requirements: #252 + lead-ruling-r1/r1a. Author: keepconv-nav.
    #[cfg(test)]
    mod keepconv_spec {
        use super::super::{Keep, keeps};
        use crate::tool::ToolKind;

        const ID: &str = "22222222-2222-4222-8222-222222222222";
        const FROM: &str = "claude --model opus --effort high";
        const TO: &str = "claude --model sonnet --effort low";

        fn baseline() -> Keep<'static> {
            Keep {
                from_binary: "claude",
                to_binary: "claude",
                tool: ToolKind::Claude,
                id: ID,
                from_home: "/account",
                from_base: "",
                to_home: Some(("/account", None)),
                from_command: Some(FROM),
                to_command: TO,
            }
        }

        #[test]
        fn model_effort_and_both_changes_keep_only_normalized_values() {
            for (from, to) in [
                (FROM, "claude --model sonnet --effort high"),
                (FROM, "claude --model opus --effort low"),
                (FROM, TO),
                (FROM, FROM),
                ("claude --model opus", "claude --model sonnet"),
                (FROM, "claude --model sonnet"),
                ("claude --model opus", TO),
                ("claude --model=opus --effort=high", TO),
            ] {
                let keep = Keep {
                    from_command: Some(from),
                    to_command: to,
                    ..baseline()
                };
                assert!(
                    keeps(&keep),
                    "only model/effort values or presence differ: {from} -> {to}"
                );
            }
        }

        #[test]
        fn third_word_permissions_assignments_and_flag_order_are_not_normalized() {
            for to in [
                "claude --model sonnet --effort low --verbose",
                "claude --model sonnet --effort low --permission-mode bypassPermissions",
                "X=1 claude --model sonnet --effort low",
                "/elsewhere/claude --model sonnet --effort low",
            ] {
                assert!(
                    !keeps(&Keep {
                        to_command: to,
                        ..baseline()
                    }),
                    "extra argv change: {to}"
                );
            }
        }

        #[test]
        fn absent_duplicate_missing_or_unreadable_model_never_keeps() {
            for to in [
                "claude --effort low",
                "claude --model",
                "claude --model= --effort low",
                "claude --model sonnet --model opus --effort low",
                "claude --model sonnet ; echo surprise",
            ] {
                assert!(
                    !keeps(&Keep {
                        to_command: to,
                        ..baseline()
                    }),
                    "unproven model: {to}"
                );
            }
            assert!(!keeps(&Keep {
                from_command: None,
                ..baseline()
            }));
            assert!(!keeps(&Keep {
                from_command: Some("claude --effort high"),
                ..baseline()
            }));
        }

        #[test]
        fn duplicate_unknown_or_environment_effort_is_not_silently_dropped() {
            for to in [
                "claude --model sonnet --effort high --effort low",
                "claude --model sonnet --effort bananas",
                "claude --model sonnet --effort",
                "CLAUDE_CODE_EFFORT_LEVEL=high claude --model sonnet --effort low",
            ] {
                assert!(
                    !keeps(&Keep {
                        to_command: to,
                        ..baseline()
                    }),
                    "unreadable effort: {to}"
                );
            }
        }

        #[test]
        fn conversation_id_uses_resume_grammar_including_opencode_ids() {
            for id in [
                "",
                "pending",
                "../x",
                "id with space",
                "claude:abc",
                "id\nnext",
            ] {
                assert!(!keeps(&Keep { id, ..baseline() }), "unusable id: {id:?}");
            }
            assert!(keeps(&Keep {
                id: "ses_alpha_123",
                ..baseline()
            }));
        }

        #[test]
        fn binary_and_account_mode_path_and_implicit_base_are_identity() {
            for keep in [
                Keep {
                    from_binary: "",
                    ..baseline()
                },
                Keep {
                    to_binary: "codex",
                    ..baseline()
                },
                Keep {
                    to_home: None,
                    ..baseline()
                },
                Keep {
                    to_home: Some(("/another", None)),
                    ..baseline()
                },
                Keep {
                    from_home: "",
                    ..baseline()
                },
                Keep {
                    from_home: "unknown",
                    ..baseline()
                },
                Keep {
                    from_home: "implicit:/account",
                    from_base: "/base",
                    ..baseline()
                },
                Keep {
                    from_base: "/base",
                    ..baseline()
                },
                Keep {
                    from_home: "implicit:/account",
                    from_base: "/base",
                    to_home: Some(("implicit:/account", Some("/other"))),
                    ..baseline()
                },
            ] {
                assert!(!keeps(&keep), "identity change must not keep");
            }
            assert!(keeps(&Keep {
                from_home: "implicit:/account",
                from_base: "/base",
                to_home: Some(("implicit:/account", Some("/base"))),
                ..baseline()
            }));
        }

        #[test]
        fn codex_measured_model_and_config_pair_effort_keep() {
            let keep = Keep {
                from_binary: "codex",
                to_binary: "codex",
                tool: ToolKind::Codex,
                from_command: Some("codex -m gpt-6-luna -c model_reasoning_effort=high"),
                to_command: "codex -m gpt-6.1-sol -c model_reasoning_effort=medium",
                ..baseline()
            };
            assert!(keeps(&keep));
            assert!(keeps(&Keep {
                to_command: "codex -m gpt-6.1-sol",
                ..keep
            }));
        }

        #[test]
        fn opencode_model_changes_cannot_keep_a_stored_model_other_rows_unmeasured() {
            let keep = Keep {
                from_binary: "opencode",
                to_binary: "opencode",
                tool: ToolKind::OpenCode,
                id: "ses_alpha_123",
                from_command: Some("opencode --model provider/old --agent build"),
                to_command: "opencode --model provider/new --agent build",
                ..baseline()
            };
            assert!(
                !keeps(&keep),
                "V2 resumes the stored model, not the requested one"
            );
            assert!(!keeps(&Keep {
                to_command: "opencode --model provider/new --agent build --variant high",
                ..keep
            }));
            for tool in [
                ToolKind::Gemini,
                ToolKind::Agy,
                ToolKind::Grok,
                ToolKind::Muse,
                ToolKind::Unknown,
            ] {
                assert!(
                    !keeps(&Keep { tool, ..baseline() }),
                    "unmeasured tool {tool:?}"
                );
            }
        }
    }

    // Oracle: docs/internals/reseat.md step 7a — a recorded `agent_bin` that
    // differs from the new binary prints ONE note, a dead pane included; a
    // UUID-grammar id stays behind as a predecessor; nothing else says a word.
    #[test]
    fn only_a_recorded_tool_that_differs_is_noted_and_only_a_uuid_is_named_as_left_behind() {
        const UUID: &str = "11111111-1111-4111-8111-111111111111";
        let note = |from: &str, to: &str, prior: &str| {
            super::tool_change_note("w1", "fake-claude", from, to, prior)
        };
        assert_eq!(note("", "claude", UUID), None, "no recorded tool, no note");
        assert_eq!(
            note("claude", "claude", UUID),
            None,
            "the same tool, no note"
        );
        assert_eq!(
            note("codex", "claude", UUID).as_deref(),
            Some(
                "note: 'w1' records codex and 'fake-claude' runs claude: if this move succeeds it \
                 starts a fresh claude conversation and its recorded conversation \
                 11111111-1111-4111-8111-111111111111 stays behind as a predecessor. ae searches \
                 for no conversation its records do not name; any other stays in its own account."
            )
        );
        for prior in [
            "",
            "pending",
            "ses_123",
            "11111111-1111-4111-8111-11111111111A",
        ] {
            assert_eq!(
                note("codex", "claude", prior).as_deref(),
                Some(
                    "note: 'w1' records codex and 'fake-claude' runs claude: if this move succeeds \
                     it starts a fresh claude conversation. ae searches for no conversation its \
                     records do not name; any other stays in its own account."
                ),
                "{prior:?} is no conversation the note may name"
            );
        }
    }

    // Oracle: `Ended` — Refused means nothing of the seat changed, Failed that
    // something was stopped or written and the move did not finish.
    #[test]
    fn a_refusal_past_the_stop_is_a_failure_only_when_this_move_stopped_the_tool() {
        use super::{CauseClass, Ended, past_the_stop};
        assert_eq!(past_the_stop(true), Ended::Failed);
        assert_eq!(past_the_stop(false), Ended::Refused(CauseClass::Other));
    }

    // Oracle: docs/internals/reseat.md crash windows and the carry ladder — a
    // marker that cannot be put back changes the next start and is said first;
    // a carry's copy is said to be inert; a plain move adds nothing.
    #[test]
    fn an_uncleared_slot_names_a_lost_marker_first_and_an_inert_copy_otherwise() {
        use super::uncleared_note;
        let lost = " Its start marker could not be put back either, so its next start would \
                    open a NEW conversation beside the one it records.";
        assert_eq!(uncleared_note(true, false), lost);
        assert_eq!(uncleared_note(true, true), lost, "the lost marker wins");
        assert_eq!(
            uncleared_note(false, true),
            " Its start marker is back; the copy already in the other account is inert, since \
             no record names it."
        );
        assert_eq!(uncleared_note(false, false), "");
    }

    // Oracle: `meta::Conversation` — Kept leaves every row; Carried rewrites the
    // store rows to the target account in the same replacement; anything else
    // opens the given id fresh from the move's own instant.
    #[test]
    fn the_seat_move_keeps_carries_with_its_account_or_opens_fresh() {
        use super::{Account, Carried, seat_conversation};
        use crate::meta::Conversation;
        let now = crate::time::Timestamp::from_epoch(1_700_000_000);
        let implicit = Account {
            path: std::path::PathBuf::from("/h/.claude"),
            row: "implicit:/h/.claude".to_owned(),
            base: Some("/h".to_owned()),
        };
        let explicit = Account {
            path: std::path::PathBuf::from("/x/claude"),
            row: "/x/claude".to_owned(),
            base: None,
        };
        let fresh = |conversation: &Conversation<'_>| {
            matches!(
                conversation,
                Conversation::Fresh {
                    id: "new-id",
                    capture_floor: 1_700_000_000
                }
            )
        };
        for account in [None, Some(&implicit)] {
            assert!(matches!(
                seat_conversation(&Carried::Kept, account, "new-id", now),
                Conversation::Kept
            ));
        }
        assert!(matches!(
            seat_conversation(&Carried::Yes, Some(&implicit), "new-id", now),
            Conversation::Carried {
                config_home: "implicit:/h/.claude",
                config_home_base: Some("/h"),
            }
        ));
        let archived = Carried::Archived {
            count: 2,
            dir: std::path::PathBuf::from("/a"),
        };
        assert!(matches!(
            seat_conversation(&archived, Some(&explicit), "new-id", now),
            Conversation::Carried {
                config_home: "/x/claude",
                config_home_base: None,
            }
        ));
        for carried in [Carried::No, Carried::Seeded("why".to_owned())] {
            assert!(fresh(&seat_conversation(
                &carried,
                Some(&implicit),
                "new-id",
                now
            )));
            assert!(fresh(&seat_conversation(&carried, None, "new-id", now)));
        }
        assert!(
            fresh(&seat_conversation(&Carried::Yes, None, "new-id", now)),
            "a carry with no account is spelled as today's fresh move"
        );
    }
}
