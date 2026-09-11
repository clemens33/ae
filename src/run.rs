//! `_run <session-dir> <slot>` — what a pane runs, and the whole of it.
//!
//! The pane's command is `<core> _run <session-dir> <slot>`: read the seat,
//! build the tool command with the SAME builders the launch uses, decide
//! create-vs-resume, then `exec` the tool.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::launch;
use crate::tool::{ResumeForm, StoreProbe, ToolKind};

/// Which form of the tool command a run builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No start marker: this seat has no conversation yet.
    Create,
    /// The marker is there: this seat has run before.
    Resume,
}

impl Mode {
    /// The word `--print` reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Resume => "resume",
        }
    }
}

/// Everything an `exec` needs: the environment deltas peeled off the command's
/// `env` prefix, and the argv itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Create or resume.
    pub mode: Mode,
    /// The tool the seat launches.
    pub tool: ToolKind,
    /// Does the prefix start from an EMPTY environment (`env -i`)?
    pub clear: bool,
    /// Names the `env -u` prefix removes.
    pub unset: Vec<String>,
    /// Assignments the `env` prefix makes, in order.
    pub set: Vec<(String, String)>,
    /// The tool and its arguments — never empty.
    pub argv: Vec<String>,
    /// A first start publishes this row before its marker and exec.
    config_home_row: Option<String>,
    /// An implicit first start publishes its effective `HOME` with the store.
    config_home_base_row: Option<String>,
    /// A retained conversation whose current config points elsewhere.
    config_home_notice: Option<String>,
}

impl Plan {
    /// The one JSON line `--print` emits.
    #[must_use]
    pub fn render(&self) -> String {
        use crate::json::Value;
        Value::Obj(vec![
            ("mode".to_owned(), Value::Str(self.mode.as_str().to_owned())),
            ("tool".to_owned(), Value::Str(self.tool.as_str().to_owned())),
            ("env_clear".to_owned(), Value::Bool(self.clear)),
            (
                "env_unset".to_owned(),
                Value::Arr(self.unset.iter().cloned().map(Value::Str).collect()),
            ),
            (
                "env_set".to_owned(),
                Value::Obj(
                    self.set
                        .iter()
                        .map(|(key, value)| (key.clone(), Value::Str(value.clone())))
                        .collect(),
                ),
            ),
            (
                "argv".to_owned(),
                Value::Arr(self.argv.iter().cloned().map(Value::Str).collect()),
            ),
        ])
        .render()
    }
}

/// Forget every generated file a previous occupant of `slot` left behind.
///
/// # Errors
///
/// A removal that failed for any reason but absence.
pub fn clear_slot(dir: &Path, slot: &str) -> std::io::Result<()> {
    let safe = launch::safe_slot(slot);
    for path in [
        started_marker(dir, slot),
        prompt_file(dir, slot),
        dir.join(format!("launch.{safe}.sh")),
        dir.join(format!("codex.{safe}.sid")),
        dir.join(format!("opencode.{safe}.md")),
        dir.join(format!("opencode.{safe}.json")),
    ] {
        match std::fs::remove_file(&path) {
            Err(why) if why.kind() != std::io::ErrorKind::NotFound => return Err(why),
            _ => {}
        }
    }
    Ok(())
}

/// Record the first user message this seat's tool is to be launched with.
///
/// # Errors
///
/// The publication failure, named.
pub fn publish_prompt(dir: &Path, slot: &str, text: &str) -> Result<(), String> {
    launch::publish_data(&prompt_file(dir, slot), text.as_bytes())
}

/// The line a pane runs — the core, this entry, the session and the seat.
#[must_use]
pub fn pane_command(core: &Path, dir: &Path, slot: &str) -> String {
    format!(
        "{} {} {} {}",
        launch::shell_quote(&core.display().to_string()),
        crate::cli::RUN,
        launch::shell_quote(&dir.display().to_string()),
        launch::shell_quote(slot)
    )
}

/// The line a re-paired pane runs, carrying the command preflight validated.
#[must_use]
pub fn pane_command_with_snapshot(core: &Path, dir: &Path, slot: &str, command: &str) -> String {
    format!(
        "{} {} --command-snapshot {} {} {}",
        launch::shell_quote(&core.display().to_string()),
        crate::cli::RUN,
        launch::shell_quote(command),
        launch::shell_quote(&dir.display().to_string()),
        launch::shell_quote(slot)
    )
}

/// The exit code for a `_run` that could not build or start its agent.
const EXIT_FAILED: u8 = 1;

/// What a resuming run says before it becomes its tool.
pub const RESUMING: &str = "ae: re-run — resuming this agent, not creating a second session.";

/// The marker whose presence means "this seat has already been launched once".
#[must_use]
pub fn started_marker(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("launch.{}.started", launch::safe_slot(slot)))
}

/// When any seat here last BECAME its tool, epoch seconds.
///
/// The marker above is `_run`'s own pre-exec record, written for every tool and
/// older than every meta row a launch publishes — so it is what a session whose
/// meta predates `started` still has. One of the facts
/// [`crate::inventory::last_live`] weighs against the host's boot time.
///
/// Read by NAME rather than by roster, so a seat the meta no longer lists still
/// counts: the question is when ae last put a tool in a pane here, not who the
/// roster says is seated.
#[must_use]
pub fn newest_start_marker(dir: &Path) -> crate::tmux::Evidence {
    use crate::tmux::Evidence;
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the start markers this module WRITES are also read here, as the oldest universal record of a pane becoming its tool"
    )]
    let listing = std::fs::read_dir(dir);
    let listing = match listing {
        Ok(listing) => listing,
        // A session directory that is not there has no markers and no damage.
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Evidence::Silent,
        Err(_) => return Evidence::Unreadable,
    };
    let mut folded = Evidence::Silent;
    for entry in listing {
        // An entry this enumeration could not read might BE a marker, so it is
        // damage rather than one fewer file.
        let Ok(entry) = entry else {
            return Evidence::Unreadable;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with("launch.") && name.ends_with(".started")) {
            continue;
        }
        // `DirEntry::metadata` does not traverse a link, so a marker replaced
        // by one is classified rather than followed.
        let Ok(meta) = entry.metadata() else {
            return Evidence::Unreadable;
        };
        if !meta.is_file() {
            return Evidence::Unreadable;
        }
        folded = folded.and(Evidence::at_mtime(meta.modified()));
        if folded == Evidence::Unreadable {
            return folded;
        }
    }
    folded
}

/// The optional first user message a spawn recorded for this seat.
#[must_use]
pub fn prompt_file(dir: &Path, slot: &str) -> PathBuf {
    dir.join(format!("launch.{}.prompt", launch::safe_slot(slot)))
}

/// `_run <session-dir> <slot>` — build this seat's command and become it.
///
/// # Errors
///
/// Propagates a write failure on the caller's streams.
pub fn run(
    dir: &Path,
    slot: &str,
    print: bool,
    command_snapshot: Option<&str>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let plan = match build_with_snapshot(dir, slot, command_snapshot) {
        Ok(plan) => plan,
        Err(why) => {
            writeln!(err, "ae: {why}")?;
            err.flush()?;
            return Ok(EXIT_FAILED);
        }
    };
    if print {
        writeln!(out, "{}", plan.render())?;
        out.flush()?;
        return Ok(0);
    }
    if let Some(value) = plan.config_home_row.as_deref()
        && let Err(why) =
            crate::meta::record_config_home(dir, slot, value, plan.config_home_base_row.as_deref())
    {
        writeln!(
            err,
            "ae: could not record config_home.{slot} before launch ({}) — refusing to start",
            why.cause()
        )?;
        err.flush()?;
        return Ok(EXIT_FAILED);
    }
    if let Some(notice) = &plan.config_home_notice {
        writeln!(err, "{notice}")?;
        err.flush()?;
    }
    // BEFORE the exec, because after it there is no "after" — and REFUSING when
    // it cannot be written.
    let marker = started_marker(dir, slot);
    if plan.mode == Mode::Create {
        if let Err(why) = publish_marker(&marker) {
            writeln!(
                err,
                "ae: could not record the start marker {} ({why}) — refusing to launch, because a re-run of this pane would create a second conversation",
                marker.display()
            )?;
            err.flush()?;
            return Ok(EXIT_FAILED);
        }
    } else {
        // Said on every resume: a human who arrow-upped the pane's command
        // gets an answer to "did that just start a second conversation?", and a
        // resumed pane says why it is not empty. It survives on screen only
        // until the tool draws over it.
        writeln!(err, "{RESUMING}")?;
        err.flush()?;
    }
    let why = exec(&plan);
    // Reached only because the exec did NOT happen, so this seat has not been
    // launched after all: take the marker back rather than leave a seat that
    // never started looking like one that did.
    if plan.mode == Mode::Create {
        let _ = std::fs::remove_file(&marker);
    }
    writeln!(err, "ae: could not start {} ({why})", plan.argv[0])?;
    err.flush()?;
    Ok(EXIT_FAILED)
}

/// Create the start marker DURABLY, or say why it could not be.
///
/// # Errors
///
/// The path that could not be written and the reason, ready to print.
fn publish_marker(marker: &Path) -> Result<(), String> {
    let temp = PathBuf::from(format!("{}.tmp.{}", marker.display(), std::process::id()));
    let write = std::fs::File::create(&temp).and_then(|file| file.sync_all());
    if let Err(why) = write {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{} — {why}", temp.display()));
    }
    if let Err(why) = std::fs::rename(&temp, marker) {
        let _ = std::fs::remove_file(&temp);
        return Err(why.to_string());
    }
    Ok(())
}

/// Replace this process with the planned command.
fn exec(plan: &Plan) -> std::io::Error {
    use std::os::unix::process::CommandExt as _;

    #[allow(
        clippy::disallowed_types,
        reason = "the pane's own exec: _run BECOMES the tool, which is what keeps pane_current_command reporting it"
    )]
    let mut command = std::process::Command::new(&plan.argv[0]);
    command.args(&plan.argv[1..]);
    // `env -i` FIRST, so the ordered unsets and sets that follow it are applied
    // to the empty environment the operator asked for rather than to the pane's.
    if plan.clear {
        command.env_clear();
    }
    for name in &plan.unset {
        command.env_remove(name);
    }
    for (name, value) in &plan.set {
        command.env(name, value);
    }
    command.exec()
}

/// Compose this seat's plan from the session's own state.
///
/// # Errors
///
/// The reason, ready to print after `ae: ` — a missing session, an unknown
/// seat, a profile this machine does not configure, or a command line the
/// direct exec cannot run.
pub fn build(dir: &Path, slot: &str) -> Result<Plan, String> {
    build_with_snapshot(dir, slot, None)
}

fn build_with_snapshot(
    dir: &Path,
    slot: &str,
    command_snapshot: Option<&str>,
) -> Result<Plan, String> {
    let seat = read_seat(dir, slot, command_snapshot)?;
    let mode = if crate::lifecycle::path_exists(&started_marker(dir, slot)) {
        Mode::Resume
    } else {
        Mode::Create
    };
    let ctx = crate::render::context_document(
        dir,
        &seat.session,
        &seat.work_dir,
        slot,
        &seat.config_files,
    );
    let current = crate::launch_cmd::config_home_resolution(&seat.command, seat.tool, &env_lookup);
    let canonical_current = canonical_config_home(&current.home);
    let canonical_base = canonical_config_home(&current.base);
    let identity = config_home_identity(
        slot,
        &seat.config_home,
        &seat.config_home_base,
        current.explicit,
        canonical_current,
        canonical_base,
    )?;
    prove_implicit_store(slot, seat.tool, &identity)?;
    let config_home_notice = (mode == Mode::Resume
        && seat.config_home != crate::meta::RecordedConfigHome::Missing
        && identity.current != identity.effective)
        .then(|| {
            format!(
                "ae: seat {slot}: config now points {} at {}; the retained conversation lives in {}, resuming there — end the session to adopt {}",
                seat.tool.as_str(),
                identity.current.shown(),
                identity.effective.shown(),
                identity.current.shown()
            )
        });
    let composed = compose(dir, slot, &seat, &ctx, mode, &identity.effective);
    let words = crate::words::split_words(&composed, &env_lookup)?;
    let (mut prefix, mut argv) = peel_env(words)?;
    if let Some((mut inner, binary_at)) = nested_env_prefix(&argv) {
        reconcile_config_home(&mut inner, seat.tool, &current, &identity);
        rebuild_nested_env(&mut argv, inner, binary_at);
    } else {
        reconcile_config_home(&mut prefix, seat.tool, &current, &identity);
    }
    Ok(Plan {
        mode,
        tool: seat.tool,
        clear: prefix.clear,
        unset: prefix.unset,
        set: prefix.assign,
        argv,
        config_home_row: identity.new_row,
        config_home_base_row: identity.new_base_row,
        config_home_notice,
    })
}

struct ConfigHomeIdentity {
    current: crate::launch_cmd::Resolved,
    recorded: crate::meta::RecordedConfigHome,
    effective: crate::launch_cmd::Resolved,
    current_base: crate::launch_cmd::Resolved,
    recorded_base: crate::meta::RecordedConfigHomeBase,
    new_row: Option<String>,
    new_base_row: Option<String>,
}

fn config_home_identity(
    slot: &str,
    stored: &crate::meta::RecordedConfigHome,
    stored_base: &crate::meta::RecordedConfigHomeBase,
    current_explicit: bool,
    canonical_current: Result<crate::launch_cmd::Resolved, String>,
    canonical_base: Result<crate::launch_cmd::Resolved, String>,
) -> Result<ConfigHomeIdentity, String> {
    let was_missing = stored == &crate::meta::RecordedConfigHome::Missing;
    let current = if was_missing {
        canonical_current?
    } else {
        current_notice(canonical_current)
    };
    let recorded = match stored {
        crate::meta::RecordedConfigHome::Missing => match &current {
            crate::launch_cmd::Resolved::Path(path) if current_explicit => {
                crate::meta::RecordedConfigHome::Path(path.clone())
            }
            crate::launch_cmd::Resolved::Path(path) => {
                crate::meta::RecordedConfigHome::Implicit(path.clone())
            }
            crate::launch_cmd::Resolved::Absent => crate::meta::RecordedConfigHome::Absent,
            crate::launch_cmd::Resolved::Unknown(_) => crate::meta::RecordedConfigHome::Unknown,
        },
        crate::meta::RecordedConfigHome::Invalid => {
            return Err(format!(
                "seat '{slot}' has malformed or duplicate config_home metadata"
            ));
        }
        other => other.clone(),
    };
    let needs_base = matches!(recorded, crate::meta::RecordedConfigHome::Implicit(_));
    let current_base = if was_missing && needs_base {
        canonical_base?
    } else {
        current_notice(canonical_base)
    };
    let recorded_base = if was_missing && needs_base {
        match &current_base {
            crate::launch_cmd::Resolved::Path(path) => {
                crate::meta::RecordedConfigHomeBase::Path(path.clone())
            }
            _ => {
                return Err(format!(
                    "seat '{slot}' has no usable HOME for its implicit config home"
                ));
            }
        }
    } else {
        stored_base.clone()
    };
    let base_is_valid = matches!(
        (&recorded, &recorded_base),
        (
            crate::meta::RecordedConfigHome::Implicit(_),
            crate::meta::RecordedConfigHomeBase::Path(_)
        ) | (
            crate::meta::RecordedConfigHome::Path(_)
                | crate::meta::RecordedConfigHome::Absent
                | crate::meta::RecordedConfigHome::Unknown,
            crate::meta::RecordedConfigHomeBase::Missing
        )
    );
    if !base_is_valid {
        return Err(format!(
            "seat '{slot}' has malformed or inconsistent config_home_base metadata"
        ));
    }
    let effective = match &recorded {
        crate::meta::RecordedConfigHome::Path(path)
        | crate::meta::RecordedConfigHome::Implicit(path) => {
            crate::launch_cmd::Resolved::Path(path.clone())
        }
        crate::meta::RecordedConfigHome::Absent => crate::launch_cmd::Resolved::Absent,
        crate::meta::RecordedConfigHome::Unknown => {
            crate::launch_cmd::Resolved::Unknown("recorded as unknown".to_owned())
        }
        crate::meta::RecordedConfigHome::Missing | crate::meta::RecordedConfigHome::Invalid => {
            return Err(format!("seat '{slot}' has unusable config_home metadata"));
        }
    };
    let new_row = was_missing.then(|| recorded.record_value()).flatten();
    let new_base_row = was_missing.then(|| recorded_base.record_value()).flatten();
    Ok(ConfigHomeIdentity {
        current,
        recorded,
        effective,
        current_base,
        recorded_base,
        new_row,
        new_base_row,
    })
}

fn current_notice(
    canonical: Result<crate::launch_cmd::Resolved, String>,
) -> crate::launch_cmd::Resolved {
    canonical.unwrap_or_else(|why| {
        crate::launch_cmd::Resolved::Unknown(format!("current config unresolvable: {why}"))
    })
}

/// Prove that a retained implicit environment still selects its recorded
/// conversation store. The store path alone cannot prove this because the
/// default directory below `HOME` may be a retargeted symbolic link.
fn prove_implicit_store(
    slot: &str,
    tool: ToolKind,
    identity: &ConfigHomeIdentity,
) -> Result<(), String> {
    let (
        crate::meta::RecordedConfigHome::Implicit(recorded),
        crate::meta::RecordedConfigHomeBase::Path(base),
    ) = (&identity.recorded, &identity.recorded_base)
    else {
        return Ok(());
    };
    let Some(default) = tool.adapter().config_home_default else {
        return Err(format!(
            "seat {slot}: {} has no implicit config-home default",
            tool.as_str()
        ));
    };
    let selected_path = base.join(default);
    let selected = canonical_config_home(&crate::launch_cmd::Resolved::Path(selected_path.clone()))
        .map_err(|why| {
            format!(
                "seat {slot}: could not resolve {} against the retained conversation store {} ({why}) — restore the link or end the session",
                selected_path.display(),
                recorded.display()
            )
        })?;
    let crate::launch_cmd::Resolved::Path(selected) = selected else {
        return Err(format!(
            "seat {slot}: could not resolve {} against the retained conversation store {} — restore the link or end the session",
            selected_path.display(),
            recorded.display()
        ));
    };
    if selected == *recorded {
        return Ok(());
    }
    Err(format!(
        "seat {slot}: {} now resolves to {}; the retained conversation lives in {} — restore the link or end the session",
        selected_path.display(),
        selected.display(),
        recorded.display()
    ))
}

/// The composed shell command line, in builder order.
fn compose(
    dir: &Path,
    slot: &str,
    seat: &Seat,
    ctx: &str,
    mode: Mode,
    config_home: &crate::launch_cmd::Resolved,
) -> String {
    if mode == Mode::Resume {
        let (resume_form, fallback_form) =
            resume_forms(seat.command.as_str(), seat.tool, &seat.harness_session);
        // DECIDE, THEN INJECT.
        let form = if resumable(seat.tool, &seat.harness_session, config_home) {
            resume_form
        } else {
            fallback_form
        };
        let injected = launch::inject_ae_context(&form, dir, slot, ctx, &seat.launch_id);
        // A resume carries no inline first message: codex's is delivered once
        // its UI returns, and no other tool has one.
        return launch::build_launch_command(&injected.cmd, "");
    }
    let pre = launch::inject_session_id(seat.command.as_str(), &seat.harness_session);
    let injected = launch::inject_ae_context(&pre, dir, slot, ctx, &seat.launch_id);
    let prompt =
        read_prompt(dir, slot).unwrap_or_else(|| launch::initial_prompt_for(seat.tool, dir, slot));
    launch::build_launch_command(&injected.cmd, &prompt)
}

/// Should this seat be resumed with the id its meta records?
fn resumable(tool: ToolKind, id: &str, config_home: &crate::launch_cmd::Resolved) -> bool {
    if !launch::id_probeable(id) {
        return false;
    }
    match tool.adapter().resume.probe {
        // Claude keeps a transcript per conversation at a path derived from the
        // working directory, so the file's existence IS the answer.
        StoreProbe::ProjectTranscript => {
            let crate::launch_cmd::Resolved::Path(home) = config_home else {
                return true;
            };
            let Some(cwd) = working_dir() else {
                return false;
            };
            let key: String = cwd
                .display()
                .to_string()
                .chars()
                .map(|ch| if ch == '/' { '-' } else { ch })
                .collect();
            crate::lifecycle::path_exists(
                &home.join("projects").join(key).join(format!("{id}.jsonl")),
            )
        }
        // Codex records under dated directories, so the id is searched for.
        StoreProbe::DatedRollouts => {
            let crate::launch_cmd::Resolved::Path(home) = config_home else {
                return true;
            };
            contains_id(&home.join("sessions"), id, 4)
        }
        // agy keeps ONE SQLite file per conversation, named for the id, in one
        // flat directory (measured 2026-09-04) — so the file's existence is the
        // answer, with no walk and no guess.
        StoreProbe::ConversationDatabase => {
            let Some(home) = env_lookup("HOME") else {
                return false;
            };
            crate::lifecycle::path_exists(
                &Path::new(&home)
                    .join(crate::session_launch::capture::AGY_CONVERSATIONS)
                    .join(format!("{id}.db")),
            )
        }
        StoreProbe::RecordedId => true,
    }
}

/// Is there a `*<id>*.jsonl` anywhere within `depth` levels of `root`?
fn contains_id(root: &Path, id: &str, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the resume probe reads the TOOL's own session store, which is the only evidence that a conversation exists"
    )]
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let log = Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"));
        if log && name.contains(id) {
            return true;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && contains_id(&entry.path(), id, depth - 1)
        {
            return true;
        }
    }
    false
}

/// Canonicalize a config home before it becomes or is compared with seat
/// identity.
///
/// A tool may create its account directory on first launch. In that case the
/// longest existing ancestor is canonicalized and the still-missing tail is
/// appended without following anything in that tail.
pub(crate) fn canonical_config_home(
    resolved: &crate::launch_cmd::Resolved,
) -> Result<crate::launch_cmd::Resolved, String> {
    let crate::launch_cmd::Resolved::Path(path) = resolved else {
        return Ok(resolved.clone());
    };
    let mut probe = path.as_path();
    let mut tail = Vec::new();
    loop {
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: launch pins or proves the tool store's longest existing canonical ancestor before exec"
        )]
        match std::fs::canonicalize(probe) {
            Ok(mut canonical) => {
                for component in tail.iter().rev() {
                    canonical.push(component);
                }
                return Ok(crate::launch_cmd::Resolved::Path(canonical));
            }
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => {
                return Err(format!(
                    "could not canonicalize config home {} ({why})",
                    path.display()
                ));
            }
        }
        #[allow(
            clippy::disallowed_methods,
            reason = "a door: a dangling link in a future config-home tail must be refused rather than recorded lexically"
        )]
        match std::fs::symlink_metadata(probe) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "config home {} has a symbolic link in its non-existing tail",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => {
                return Err(format!(
                    "could not inspect config home {} ({why})",
                    path.display()
                ));
            }
        }
        match probe.components().next_back() {
            Some(std::path::Component::Normal(component)) => tail.push(component.to_os_string()),
            Some(std::path::Component::ParentDir) => {
                return Err(format!(
                    "config home {} has '..' in its non-existing tail",
                    path.display()
                ));
            }
            _ => {
                return Err(format!(
                    "config home {} has no existing ancestor",
                    path.display()
                ));
            }
        }
        let Some(parent) = probe.parent() else {
            return Err(format!(
                "config home {} has no existing ancestor",
                path.display()
            ));
        };
        probe = parent;
    }
}

/// The resume form of a profile's command, and the form to use when the
/// conversation cannot be found.
#[must_use]
pub fn resume_forms(cmd: &str, tool: ToolKind, session_id: &str) -> (String, String) {
    match tool.adapter().resume.form {
        ResumeForm::Flags { exact, fallback } => (
            format!("{cmd} {exact} {session_id}"),
            format!("{cmd} {fallback}"),
        ),
        ResumeForm::StrippedFlags {
            grammar,
            exact,
            fallback,
        } => {
            let clean = launch::strip_session_grammar(cmd, grammar);
            (
                format!("{clean} {exact} {session_id}"),
                format!("{clean} {fallback}"),
            )
        }
        ResumeForm::Subcommand { grammar, command } => {
            let clean = launch::strip_session_grammar(cmd, grammar);
            (format!("{clean} {command} {session_id}"), clean)
        }
        ResumeForm::None => (cmd.to_owned(), cmd.to_owned()),
    }
}

/// An `env` prefix, split into what it clears, removes and assigns.
#[derive(Debug, Default, PartialEq, Eq)]
struct EnvPrefix {
    /// `env -i`: start from an empty environment.
    clear: bool,
    /// Names `env -u` removes.
    unset: Vec<String>,
    /// `NAME=value` assignments, in order.
    assign: Vec<(String, String)>,
}

/// Peel the environment prefix off a split command line.
///
/// # Errors
///
/// A command line that is nothing but an environment prefix — there is no
/// binary to exec, and guessing one is how a mis-shaped command reaches a live
/// pane.
fn peel_env(words: Vec<crate::words::Word>) -> Result<(EnvPrefix, Vec<String>), String> {
    let mut prefix = EnvPrefix::default();
    let mut rest = words.into_iter().peekable();
    // A shell's own prefix: assignments up to the command word, decided AS
    // WRITTEN.
    while rest.peek().is_some_and(|word| word.assignment) {
        let Some(word) = rest.next() else { break };
        let Some((name, value)) = word.value.split_once('=') else {
            break;
        };
        prefix.assign.push((name.to_owned(), value.to_owned()));
    }
    // EXACTLY ONE `env`, because `launch_binary` peels exactly one. A second is
    // a COMMAND WORD to the classifier, so a loop here would run a tool the
    // classifier had typed as something else. Widen both or neither.
    if rest.peek().is_some_and(|word| word.value == "env") {
        rest.next();
        // `env`'s own operands.
        while let Some(word) = rest.peek() {
            match word.value.as_str() {
                "-i" => {
                    prefix.clear = true;
                    rest.next();
                }
                "-u" => {
                    rest.next();
                    match rest.next() {
                        Some(name) => prefix.unset.push(name.value),
                        None => {
                            return Err("an `env -u` with no name in a launch command".to_owned());
                        }
                    }
                }
                // On the VALUE, because `env` reads its own argv after the
                // shell has unquoted it — and because `launch_binary` does.
                value if crate::launch_cmd::is_assignment(value) => {
                    let Some(word) = rest.next() else { break };
                    if let Some((name, value)) = word.value.split_once('=') {
                        prefix.assign.push((name.to_owned(), value.to_owned()));
                    }
                }
                _ => break,
            }
        }
    }
    let argv: Vec<String> = rest.map(|word| word.value).collect();
    if argv.is_empty() {
        return Err("a launch command with no binary to run".to_owned());
    }
    Ok((prefix, argv))
}

/// Read the original command's `env` prefix when ae's own injected prefix has
/// become the outer layer. Reconciliation must happen in the innermost layer:
/// an inner `env -i` would otherwise discard the recorded HOME override.
fn nested_env_prefix(argv: &[String]) -> Option<(EnvPrefix, usize)> {
    if argv.first().is_none_or(|word| word != "env") {
        return None;
    }
    let mut prefix = EnvPrefix::default();
    let mut index = 1;
    while let Some(word) = argv.get(index) {
        match word.as_str() {
            "-i" => {
                prefix.clear = true;
                index += 1;
            }
            "-u" => {
                let name = argv.get(index + 1)?;
                prefix.unset.push(name.clone());
                index += 2;
            }
            value if crate::launch_cmd::is_assignment(value) => {
                if let Some((name, value)) = value.split_once('=') {
                    prefix.assign.push((name.to_owned(), value.to_owned()));
                }
                index += 1;
            }
            _ => break,
        }
    }
    (index < argv.len()).then_some((prefix, index))
}

fn rebuild_nested_env(argv: &mut Vec<String>, prefix: EnvPrefix, binary_at: usize) {
    let command = argv.split_off(binary_at);
    argv.clear();
    argv.push("env".to_owned());
    if prefix.clear {
        argv.push("-i".to_owned());
    }
    for name in prefix.unset {
        argv.push("-u".to_owned());
        argv.push(name);
    }
    for (name, value) in prefix.assign {
        argv.push(format!("{name}={value}"));
    }
    argv.extend(command);
}

/// Reconcile mutable config with the store identity recorded by the seat.
fn reconcile_config_home(
    prefix: &mut EnvPrefix,
    tool: ToolKind,
    current: &crate::launch_cmd::ConfigHomeResolution,
    identity: &ConfigHomeIdentity,
) {
    match &identity.recorded {
        crate::meta::RecordedConfigHome::Implicit(_) => {
            if current.explicit {
                use_default_config_home(prefix, tool);
            }
            if let crate::meta::RecordedConfigHomeBase::Path(base) = &identity.recorded_base
                && identity.current_base != crate::launch_cmd::Resolved::Path(base.clone())
            {
                apply_recorded_home(prefix, base);
            }
        }
        crate::meta::RecordedConfigHome::Path(path) => {
            let effective = crate::launch_cmd::Resolved::Path(path.clone());
            if !current.explicit || current.home != effective {
                apply_config_home(prefix, tool, &effective);
            }
        }
        crate::meta::RecordedConfigHome::Missing
        | crate::meta::RecordedConfigHome::Absent
        | crate::meta::RecordedConfigHome::Unknown
        | crate::meta::RecordedConfigHome::Invalid => {}
    }
}

/// Restore the HOME recorded alongside a retained implicit store.
fn apply_recorded_home(prefix: &mut EnvPrefix, home: &Path) {
    prefix.unset.retain(|name| name != "HOME");
    prefix.assign.retain(|(name, _)| name != "HOME");
    prefix
        .assign
        .push(("HOME".to_owned(), home.display().to_string()));
}

/// Select a recorded default store without relocating the harness into it.
fn use_default_config_home(prefix: &mut EnvPrefix, tool: ToolKind) {
    let Some(variable) = tool.adapter().config_home_env else {
        return;
    };
    prefix.assign.retain(|(name, _)| name != variable);
    if !prefix.unset.iter().any(|name| name == variable) {
        prefix.unset.push(variable.to_owned());
    }
}

/// Make a recorded concrete store override mutable profile/environment state.
fn apply_config_home(
    prefix: &mut EnvPrefix,
    tool: ToolKind,
    resolved: &crate::launch_cmd::Resolved,
) {
    let (Some(variable), crate::launch_cmd::Resolved::Path(path)) =
        (tool.adapter().config_home_env, resolved)
    else {
        return;
    };
    prefix.unset.retain(|name| name != variable);
    prefix.assign.retain(|(name, _)| name != variable);
    prefix
        .assign
        .push((variable.to_owned(), path.display().to_string()));
}

/// One seat, read back out of the session's own state.
struct Seat {
    session: String,
    work_dir: String,
    config_files: Vec<PathBuf>,
    command: crate::config::ResolvedCommand,
    tool: ToolKind,
    harness_session: String,
    launch_id: String,
    config_home: crate::meta::RecordedConfigHome,
    config_home_base: crate::meta::RecordedConfigHomeBase,
}

/// Read the seat `slot` names, refusing anything that is not launchable.
fn read_seat(dir: &Path, slot: &str, command_snapshot: Option<&str>) -> Result<Seat, String> {
    if !crate::lifecycle::dir_exists(dir) {
        return Err(format!("no session state at {}", dir.display()));
    }
    let bytes = crate::meta::read_bytes(dir)
        .map_err(|why| format!("could not read the session meta ({why})"))?;
    let parsed_meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    let value = |key: &str| crate::lifecycle::meta_value(&bytes, key);
    let name = value(&format!("seat.{slot}"));
    if name.is_empty() {
        return Err(format!("no seat '{slot}' in {}", dir.display()));
    }
    let config_home = parsed_meta
        .roster()
        .iter()
        .find(|entry| entry.slot == slot)
        .map(|entry| entry.config_home.clone())
        .unwrap_or_default();
    let config_home_base = parsed_meta
        .roster()
        .iter()
        .find(|entry| entry.slot == slot)
        .map(|entry| entry.config_home_base.clone())
        .unwrap_or_default();
    let profile = value(&format!("profile.{slot}"));
    if profile.is_empty() {
        return Err(format!("seat '{slot}' has no profile recorded"));
    }
    let origin = value("origin");
    let mut config_files: Vec<PathBuf> = Vec::new();
    let global = value("config");
    if !global.is_empty() {
        config_files.push(PathBuf::from(&global));
    }
    let local = crate::config::local_overlay(dir, &origin);
    if let Some(local) = &local {
        config_files.push(local.clone());
    }
    let orchestrator_seat = local.as_deref().is_some_and(|path| {
        dir.parent()
            .and_then(Path::parent)
            .is_some_and(|home| crate::orchestrator::is_seat_overlay(path, home))
    });
    let command = if let Some(command) = command_snapshot {
        crate::config::IdentityConfig::resolved_snapshot(command)
    } else {
        let cfg = crate::config::read_identity(
            (!global.is_empty()).then(|| Path::new(&global)),
            (!orchestrator_seat).then_some(local.as_deref()).flatten(),
        )
        .map_err(|why| why.to_string())?;
        let home = crate::doors::home();
        let command = cfg
            .command(&profile, home.as_deref())
            .map_err(|why| why.to_string())?;
        let Some(command) = command.filter(|cmd| !cmd.as_str().trim().is_empty()) else {
            return Err(format!(
                "profile '{profile}' is not configured on this machine — '{name}' cannot be launched"
            ));
        };
        command
    };
    // An ordinary/manual `_run` reads the profile fresh, so it re-asks the same
    // validator. A launch-provided snapshot already passed that validator, but
    // validating the transported bytes again keeps this entry safe on its own.
    let parsed = crate::launch_cmd::lex_simple_command(command.as_str()).map_err(|why| {
        format!(
            "profile '{profile}' is not one simple command — {why} — '{name}' cannot be launched"
        )
    })?;
    let tool = parsed.tool();
    if command_snapshot.is_some() {
        let recorded = ToolKind::from_binary_name(&value(&format!("agent_bin.{slot}")));
        if tool != recorded {
            return Err(format!(
                "profile '{profile}' command snapshot changed tool kind from {} to {} — '{name}' cannot be launched",
                recorded.as_str(),
                tool.as_str()
            ));
        }
    }
    Ok(Seat {
        session: value("session"),
        work_dir: value("work_dir"),
        config_files,
        tool,
        command,
        harness_session: value(&format!("harness_session.{slot}")),
        launch_id: value(&format!("launch_id.{slot}")),
        config_home,
        config_home_base,
    })
}

/// The first user message a spawn recorded for this seat, if it recorded one.
fn read_prompt(dir: &Path, slot: &str) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the spawn's own recorded first message, published beside the session meta"
    )]
    let text = std::fs::read_to_string(prompt_file(dir, slot));
    text.ok().filter(|body| !body.is_empty())
}

/// One environment variable of the process this run inherits.
fn env_lookup(name: &str) -> Option<String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the pane's own environment is what `bash -lc` expanded a profile command against"
    )]
    let value = std::env::var_os(name);
    value.map(|value| value.to_string_lossy().into_owned())
}

/// The pane's working directory — what claude derives its transcript path from.
fn working_dir() -> Option<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a door: the resume probe's `$PWD`, read at run time exactly as the frozen shell test read it"
    )]
    let cwd = std::env::current_dir();
    cwd.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test fixture creates and inspects the complete artifact set"
    )]
    fn clearing_a_slot_removes_every_launch_artifact() {
        let dir = std::env::temp_dir().join(format!("ae-clear-slot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a fixture dir");
        let artifacts = [
            "launch.spawned.7.started",
            "launch.spawned.7.prompt",
            "launch.spawned.7.sh",
            "codex.spawned.7.sid",
            "opencode.spawned.7.md",
            "opencode.spawned.7.json",
        ];
        for name in artifacts {
            std::fs::write(dir.join(name), "stale").expect("a stale artifact");
        }

        clear_slot(&dir, "spawned.7").expect("the slot clears");

        for name in artifacts {
            assert!(
                !dir.join(name).exists(),
                "a future occupant could inherit {name}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_env_prefix_becomes_environment_deltas_and_the_tool_becomes_argv() {
        let words = crate::words::split_words(
            "env -u CLAUDECODE -u CLAUDE_CODE_SESSION CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0 claude --session-id x",
            &|_| None,
        )
        .expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.unset, ["CLAUDECODE", "CLAUDE_CODE_SESSION"]);
        assert_eq!(
            prefix.assign,
            [(
                "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION".to_owned(),
                "0".to_owned()
            )]
        );
        assert_eq!(argv, ["claude", "--session-id", "x"]);
    }

    #[test]
    fn a_command_line_that_is_only_a_prefix_has_no_binary_to_exec() {
        let words = crate::words::split_words("env -u A", &|_| None).expect("splits");
        assert!(peel_env(words).is_err());
    }

    #[test]
    fn a_plain_command_keeps_its_whole_argv_and_touches_no_environment() {
        let words = crate::words::split_words("codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix, EnvPrefix::default());
        assert_eq!(argv, ["codex", "--yolo"]);
    }

    #[test]
    fn the_printed_plan_is_one_decodable_json_line() {
        let plan = Plan {
            mode: Mode::Resume,
            tool: ToolKind::Claude,
            clear: false,
            unset: vec!["CLAUDECODE".to_owned()],
            set: vec![("K".to_owned(), "0".to_owned())],
            argv: vec!["claude".to_owned(), "a\nb".to_owned()],
            config_home_row: None,
            config_home_base_row: None,
            config_home_notice: None,
        };
        let line = plan.render();
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains(r#""mode":"resume""#), "{line}");
        assert!(line.contains(r#""tool":"claude""#), "{line}");
        assert!(line.contains(r#""argv":["claude","a\nb"]"#), "{line}");
    }

    #[test]
    fn a_bare_leading_assignment_is_an_environment_delta_not_the_binary() {
        // Colead Z2 BLOCKER-1: `A=1 codex --yolo` classified as codex and then
        // `exec`ed a binary literally named `A=1`.
        let words = crate::words::split_words("A=1 B=2 codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert!(!prefix.clear);
        assert_eq!(
            prefix.assign,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned())
            ]
        );
        assert_eq!(argv, ["codex", "--yolo"]);
        // …and the two forms compose, in the order a shell applies them.
        let words =
            crate::words::split_words("A=1 env -u KEEPOUT B=2 claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.unset, ["KEEPOUT"]);
        assert_eq!(
            prefix.assign,
            [
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned())
            ]
        );
        assert_eq!(argv, ["claude"]);
        // The env vocabulary is the CLASSIFIER's, down to what it does not
        // know: `--` is a command word to `launch_binary`, so it is one here.
        let words = crate::words::split_words("env -- claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, []);
        assert_eq!(argv, ["--", "claude"]);
    }

    #[test]
    fn a_quoted_leading_assignment_is_the_command_word_not_an_assignment() {
        // The B3 defect pointed at B1: `words::split` erased quoting, so
        // peel_env re-derived assignment-shape from the DECODED value and
        // called `'A=1'` an assignment where the validator calls it the binary.
        let words = crate::words::split_words("'A=1' codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, [], "a quoted word assigns nothing");
        assert_eq!(argv, ["A=1", "codex", "--yolo"]);

        let words = crate::words::split_words("A=1 codex --yolo", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, [("A".to_owned(), "1".to_owned())]);
        assert_eq!(argv, ["codex", "--yolo"]);

        // Every spelling that quotes the `=` itself is the command word too…
        for cmd in [r"A\=1 codex", "A'='1 codex", "\"A=1\" codex"] {
            let words = crate::words::split_words(cmd, &|_| None).expect("splits");
            let (prefix, argv) = peel_env(words).expect("peels");
            assert_eq!(prefix.assign, [], "{cmd:?}");
            assert_eq!(argv[0], "A=1", "{cmd:?}");
        }
        // …while quoting only the VALUE leaves an ordinary assignment.
        let words = crate::words::split_words("A=\"1 2\" codex", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, [("A".to_owned(), "1 2".to_owned())]);
        assert_eq!(argv, ["codex"]);

        // `env`'s OWN operands are the other rule: env parses an argv the shell
        // has already unquoted, so a quoted one does assign — and that is what
        // `launch_binary` says too.
        let words = crate::words::split_words("env 'A=1' codex", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert_eq!(prefix.assign, [("A".to_owned(), "1".to_owned())]);
        assert_eq!(argv, ["codex"]);
    }

    #[test]
    fn an_env_dash_i_clears_the_environment_instead_of_being_consumed() {
        // Colead Z2 BLOCKER-1, the other half: `-i` was peeled and dropped, so
        // the pane's whole environment reached a tool asked to start clean.
        let words = crate::words::split_words("env -i claude", &|_| None).expect("splits");
        let (prefix, argv) = peel_env(words).expect("peels");
        assert!(prefix.clear);
        assert_eq!(argv, ["claude"]);
        let words = crate::words::split_words("claude", &|_| None).expect("splits");
        let (prefix, _) = peel_env(words).expect("peels");
        assert!(!prefix.clear, "a plain command clears nothing");
    }

    #[test]
    fn the_binary_this_peel_leaves_is_the_one_the_classifier_named() {
        // The classifier decides `agent_bin` and the tool kind; this peel
        // decides what is `exec`ed.
        for cmd in [
            "claude",
            "/usr/bin/claude --flag",
            "A=1 codex --yolo",
            "A=1 B=2 codex",
            "env -u CLAUDECODE claude",
            "env -i claude",
            "env -i -u A B=2 claude",
            "A=1 env -u B C=3 claude",
            "env -- claude",
            "env --ignore-environment claude",
            "--flag=x claude",
            // Quoting decides assignment-shape, and both modules read it off
            // the word AS WRITTEN.
            "'A=1' codex --yolo",
            r"A\=1 codex",
            "A'='1 codex",
            "\"A=1\" codex",
            "A=\"1 2\" codex",
            "env 'A=1' codex",
            // A SECOND `env` is a command word, not a second prefix.
            "env env claude",
            "A=1 env env claude",
            "env -i env -u B claude",
        ] {
            let named = crate::launch_cmd::lex_simple_command(cmd)
                .map(|parsed| parsed.binary)
                .unwrap_or_default();
            let words = crate::words::split_words(cmd, &|_| None).expect("splits");
            let (_, argv) = peel_env(words).expect("peels");
            let run = argv[0].rsplit('/').next().unwrap_or(&argv[0]).to_owned();
            assert_eq!(named, run, "{cmd:?}: classified one way, exec'ed another");
        }
    }

    #[test]
    fn the_printed_plan_reports_whether_the_environment_is_cleared() {
        let plan = Plan {
            mode: Mode::Create,
            tool: ToolKind::Unknown,
            clear: true,
            unset: Vec::new(),
            set: Vec::new(),
            argv: vec!["/usr/bin/env".to_owned()],
            config_home_row: None,
            config_home_base_row: None,
            config_home_notice: None,
        };
        assert!(
            plan.render().contains(r#""env_clear":true"#),
            "{}",
            plan.render()
        );
    }

    #[test]
    fn a_future_config_home_refuses_parent_segments_and_dangling_links() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("ae-future-config-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("existing ancestor");
        let parent = crate::launch_cmd::Resolved::Path(root.join("missing/../account"));
        assert!(
            canonical_config_home(&parent)
                .expect_err("a parent segment in the missing tail refuses")
                .contains("'..'")
        );

        let dangling = root.join("dangling");
        symlink(root.join("absent-target"), &dangling).expect("dangling link");
        let linked = crate::launch_cmd::Resolved::Path(dangling.join("account"));
        assert!(
            canonical_config_home(&linked)
                .expect_err("a dangling link in the missing tail refuses")
                .contains("symbolic link")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_start_marker_that_cannot_be_published_is_an_error_not_a_shrug() {
        // Colead Z2 BLOCKER-2: the marker is the create-once discriminator, so
        // a failure to write it has to reach the caller rather than be shrugged
        // off into a seat that re-creates on its next run.
        use std::os::unix::fs::PermissionsExt as _;

        let dir = PathBuf::from(format!("/tmp/aemarker.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a fixture dir");
        assert!(publish_marker(&started_marker(&dir, "main")).is_ok());
        std::fs::remove_file(started_marker(&dir, "main")).expect("the marker is there");

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("a read-only fixture dir");
        let refused = publish_marker(&started_marker(&dir, "main"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("restored so the fixture can be removed");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(refused.is_err(), "an unwritable session directory refuses");
    }

    #[test]
    fn agy_resumes_by_conversation_and_falls_back_to_continue() {
        // The pair, read directly rather than through a launch: agy has no
        // `--resume` at all, and its operator-facing session flags are stripped
        // off BOTH forms so nothing an operator pinned can stack with, or be
        // read instead of, the id ae is putting on.
        let (exact, fallback) = resume_forms(
            "agy --conversation OLD -c --dangerously-skip-permissions",
            ToolKind::Agy,
            "u-9",
        );
        assert_eq!(
            exact,
            "agy --dangerously-skip-permissions --conversation u-9"
        );
        assert_eq!(fallback, "agy --dangerously-skip-permissions --continue");
        assert!(!exact.contains("OLD") && !fallback.contains("OLD"));
        assert!(!exact.contains("--resume"), "agy has no --resume: {exact}");
    }

    #[test]
    fn a_missing_id_is_the_only_thing_that_makes_a_seat_unresumable_without_a_probe() {
        // No probe exists for these three, and that is not evidence of
        // absence: the recorded id is still this seat's own conversation.
        for tool in [ToolKind::Gemini, ToolKind::Grok, ToolKind::OpenCode] {
            assert!(
                resumable(tool, "3f2a-1", &crate::launch_cmd::Resolved::Absent),
                "{}",
                tool.as_str()
            );
            assert!(
                !resumable(tool, "pending", &crate::launch_cmd::Resolved::Absent),
                "{}",
                tool.as_str()
            );
            assert!(
                !resumable(tool, "", &crate::launch_cmd::Resolved::Absent),
                "{}",
                tool.as_str()
            );
        }
        // A tool ae CAN probe still has to pass it, and a `pending` id never
        // reaches the probe at all.
        assert!(!resumable(
            ToolKind::Claude,
            "pending",
            &crate::launch_cmd::Resolved::Absent
        ));
        assert!(!resumable(
            ToolKind::Codex,
            "",
            &crate::launch_cmd::Resolved::Absent
        ));
    }
}
