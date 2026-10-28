//! `ae init`: discover supported harnesses and publish a first global config.
//!
//! Discovery reuses doctor's PATH resolver and runs nothing. Proposal building
//! is pure; filesystem mutation starts only after argv and any terminal input
//! have been validated.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::entry::{EXIT_FAILED, EXIT_USAGE, PROFILE_CATALOG, Profile};

mod roles;

use roles::{Pick, Role};

/// Public command grammar.
pub const USAGE: &str = "Usage: ae init [--yes] \
     [--lead <profile>|--lead-harness <h> --lead-model <m> --lead-effort <e>] \
     [--colead <profile>|--colead-harness <h> --colead-model <m> --colead-effort <e>|--solo] \
     [--orchestrator <profile>|--no-orchestrator] [--palette <darcula|a|b>] [--force]\n";

const SUPPORTED: [&str; 6] = ["claude", "codex", "grok", "agy", "opencode", "gemini"];
const EXTRA_ORDER: [(&str, &str); 4] = [
    ("grok", "grok46"),
    ("agy", "agy"),
    ("opencode", "opencode"),
    ("gemini", "gemini"),
];
const MAX_ANSWER_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Args {
    yes: bool,
    force: bool,
    lead: Option<String>,
    lead_parts: Parts,
    colead: SeatArg,
    colead_parts: Parts,
    orchestrator: SeatArg,
    palette: Option<String>,
}

/// The harness, model and effort flags of one role; each is its own choice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Parts {
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
}

impl Parts {
    fn any(&self) -> bool {
        self.harness.is_some() || self.model.is_some() || self.effort.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum SeatArg {
    #[default]
    Unset,
    Profile(String),
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Discovery {
    tool: &'static str,
    path: Option<PathBuf>,
}

/// One roster seat: a catalog profile by name, or a harness/model/effort pick
/// that renders its own profile.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seat {
    Profile(String),
    Pick(Pick),
}

impl Seat {
    fn name(&self) -> String {
        match self {
            Self::Profile(name) => name.clone(),
            Self::Pick(pick) => pick.name(),
        }
    }

    fn pick(&self) -> Option<&Pick> {
        match self {
            Self::Pick(pick) => Some(pick),
            Self::Profile(_) => None,
        }
    }

    /// The executable the seat launches.
    fn harness(&self) -> &str {
        match self {
            Self::Pick(pick) => pick.harness.name,
            Self::Profile(name) => PROFILE_CATALOG
                .iter()
                .find(|profile| profile.name == name)
                .map_or("", |profile| profile.harness),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Choices {
    lead: Seat,
    colead: Option<Seat>,
    orchestrator: Option<Seat>,
    palette: String,
}

impl Choices {
    fn seats(&self) -> impl Iterator<Item = &Seat> {
        std::iter::once(&self.lead)
            .chain(&self.colead)
            .chain(&self.orchestrator)
    }
}

fn parse(tail: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut rest = tail;
    while let [word, after @ ..] = rest {
        match word.as_str() {
            "--yes" if !args.yes => {
                args.yes = true;
                rest = after;
            }
            "--force" if !args.force => {
                args.force = true;
                rest = after;
            }
            "--solo" if args.colead == SeatArg::Unset => {
                args.colead = SeatArg::Disabled;
                rest = after;
            }
            "--no-orchestrator" if args.orchestrator == SeatArg::Unset => {
                args.orchestrator = SeatArg::Disabled;
                rest = after;
            }
            "--lead" | "--colead" | "--orchestrator" | "--palette" | "--lead-harness"
            | "--lead-model" | "--lead-effort" | "--colead-harness" | "--colead-model"
            | "--colead-effort" => {
                let Some((value, tail)) = after.split_first() else {
                    return Err(word.clone());
                };
                if value.starts_with('-') {
                    return Err(word.clone());
                }
                let (lead, colead) = (&mut args.lead_parts, &mut args.colead_parts);
                let slot = match word.as_str() {
                    "--lead" if args.lead.is_none() => &mut args.lead,
                    "--lead-harness" if lead.harness.is_none() => &mut lead.harness,
                    "--lead-model" if lead.model.is_none() => &mut lead.model,
                    "--lead-effort" if lead.effort.is_none() => &mut lead.effort,
                    "--colead-harness" if colead.harness.is_none() => &mut colead.harness,
                    "--colead-model" if colead.model.is_none() => &mut colead.model,
                    "--colead-effort" if colead.effort.is_none() => &mut colead.effort,
                    "--palette" if args.palette.is_none() => &mut args.palette,
                    "--colead" if args.colead == SeatArg::Unset => {
                        args.colead = SeatArg::Profile(value.clone());
                        rest = tail;
                        continue;
                    }
                    "--orchestrator" if args.orchestrator == SeatArg::Unset => {
                        args.orchestrator = SeatArg::Profile(value.clone());
                        rest = tail;
                        continue;
                    }
                    _ => return Err(word.clone()),
                };
                *slot = Some(value.clone());
                rest = tail;
            }
            _ => return Err(word.clone()),
        }
    }
    if args.lead.is_some() && args.lead_parts.any() {
        return Err("--lead-*".to_owned());
    }
    if args.colead != SeatArg::Unset && args.colead_parts.any() {
        return Err("--colead-*".to_owned());
    }
    Ok(args)
}

fn discover() -> Vec<Discovery> {
    discover_with(crate::doctor::resolve_on_path)
}

fn discover_with(mut resolve: impl FnMut(&str) -> Option<PathBuf>) -> Vec<Discovery> {
    SUPPORTED
        .into_iter()
        .map(|tool| Discovery {
            tool,
            path: resolve(tool),
        })
        .collect()
}

fn is_found(found: &[Discovery], tool: &str) -> bool {
    found
        .iter()
        .any(|item| item.tool == tool && item.path.is_some())
}

fn available<'a>(found: &[Discovery], profile: &'a Profile) -> Option<&'a Profile> {
    is_found(found, profile.harness).then_some(profile)
}

fn profile(found: &[Discovery], name: &str) -> Option<&'static Profile> {
    PROFILE_CATALOG
        .iter()
        .find(|candidate| candidate.name == name)
        .and_then(|candidate| available(found, candidate))
}

fn default_choices(found: &[Discovery]) -> Option<Choices> {
    let claude = is_found(found, "claude");
    let codex = is_found(found, "codex");
    let profile_seat = |name: &str| Seat::Profile(name.to_owned());
    let (lead, colead, orchestrator) = if claude || codex {
        let pick = |role, harness| {
            Pick::resolve(role, harness, None, None, None)
                .ok()
                .map(Seat::Pick)
        };
        let lead = pick(Role::Lead, if claude { "claude" } else { "codex" })?;
        let colead = pick(Role::Colead, if codex { "codex" } else { "claude" })?;
        let orchestrator = if codex {
            profile_seat("gpt61solx")
        } else {
            lead.clone()
        };
        (lead, Some(colead), Some(orchestrator))
    } else {
        let extras: Vec<&str> = EXTRA_ORDER
            .into_iter()
            .filter_map(|(tool, profile)| is_found(found, tool).then_some(profile))
            .collect();
        let lead = *extras.first()?;
        (
            profile_seat(lead),
            extras.get(1).map(|name| profile_seat(name)),
            Some(profile_seat(lead)),
        )
    };
    Some(Choices {
        lead,
        colead,
        orchestrator,
        palette: "darcula".to_owned(),
    })
}

/// A role's pick from its flags; what they leave out is filled from the table.
fn pick_seat(role: Role, current: Option<&Seat>, parts: &Parts) -> Result<Seat, String> {
    let fallback = current
        .and_then(Seat::pick)
        .map_or(role.defaults().0, |pick| pick.harness.name);
    Pick::resolve(
        role,
        fallback,
        parts.harness.as_deref(),
        parts.model.as_deref(),
        parts.effort.as_deref(),
    )
    .map(Seat::Pick)
}

fn apply_args(mut choices: Choices, args: &Args, found: &[Discovery]) -> Result<Choices, String> {
    if let Some(lead) = &args.lead {
        choices.lead = Seat::Profile(lead.clone());
    } else if args.lead_parts.any() {
        choices.lead = pick_seat(Role::Lead, Some(&choices.lead), &args.lead_parts)?;
    }
    match &args.colead {
        SeatArg::Unset if args.colead_parts.any() => {
            let seat = pick_seat(Role::Colead, choices.colead.as_ref(), &args.colead_parts)?;
            choices.colead = Some(seat);
        }
        SeatArg::Unset => {}
        SeatArg::Profile(profile) => choices.colead = Some(Seat::Profile(profile.clone())),
        SeatArg::Disabled => choices.colead = None,
    }
    match &args.orchestrator {
        SeatArg::Unset => {}
        SeatArg::Profile(profile) => choices.orchestrator = Some(Seat::Profile(profile.clone())),
        SeatArg::Disabled => choices.orchestrator = None,
    }
    if let Some(palette) = &args.palette {
        choices.palette.clone_from(palette);
    }
    validate_choices(&choices, found)?;
    Ok(choices)
}

fn validate_choices(choices: &Choices, found: &[Discovery]) -> Result<(), String> {
    for seat in choices.seats() {
        let (usable, offender) = match seat {
            Seat::Profile(name) => (
                crate::config::is_agent_name(name) && profile(found, name).is_some(),
                name.as_str(),
            ),
            Seat::Pick(pick) => (is_found(found, pick.harness.name), pick.harness.name),
        };
        if !usable {
            return Err(offender.to_owned());
        }
    }
    if !matches!(choices.palette.as_str(), "darcula" | "a" | "b") {
        return Err(choices.palette.clone());
    }
    Ok(())
}

fn provider(found: &[Discovery], seat: &Seat) -> Option<&'static str> {
    match seat {
        Seat::Profile(name) => profile(found, name).and_then(|item| item.provider),
        Seat::Pick(pick) => Some(pick.harness.provider),
    }
}

fn notes(found: &[Discovery], choices: &Choices) -> Vec<String> {
    let found_count = found.iter().filter(|item| item.path.is_some()).count();
    let mut notes = Vec::new();
    if let Some(colead) = &choices.colead {
        let lead_provider = provider(found, &choices.lead);
        let colead_provider = provider(found, colead);
        match (lead_provider, colead_provider) {
            (Some(lead), Some(colead)) if lead == colead => {
                notes.push(
                    "same provider: cross-provider review needs a profile on a DIFFERENT provider"
                        .to_owned(),
                );
            }
            (Some(_), Some(_)) => {}
            _ => {
                notes
                    .push("provider unverified: cross-provider review cannot be proven".to_owned());
            }
        }
    } else if found_count == 1 {
        notes.push("single provider: add a second harness for cross-provider review".to_owned());
    }

    let claude_xor_codex = is_found(found, "claude") ^ is_found(found, "codex");
    if claude_xor_codex {
        for (tool, name) in EXTRA_ORDER {
            if is_found(found, tool) {
                let provider = profile(found, name)
                    .and_then(|item| item.provider)
                    .unwrap_or("provider unverified");
                notes.push(format!("reviewer profile available: {name} ({provider})"));
            }
        }
    }
    notes
}

fn render(found: &[Discovery], choices: &Choices, now: crate::time::Timestamp) -> String {
    let timestamp = now.to_string();
    let date = timestamp.get(..10).unwrap_or(timestamp.as_str());
    let mut output = format!(
        "# written by ae init on {date}; model pins as of ae {}\n",
        crate::VERSION
    );
    let mut section = "";
    for line in crate::entry::DEFAULT_CONFIG.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            section = line;
        }
        if section == "[clients]"
            && let Some(name) = catalog_client_line(line)
        {
            if is_found(found, name) {
                output.push_str(line);
                output.push('\n');
            }
            continue;
        }
        if section == "[profiles]"
            && let Some(name) = catalog_profile_line(line)
        {
            if profile(found, name).is_some() {
                output.push_str(line);
                output.push('\n');
            }
            continue;
        }
        if section == "[roster]" {
            if line == "lead = fable5" {
                output.push_str("lead = ");
                output.push_str(&choices.lead.name());
                output.push('\n');
                if let Some(colead) = &choices.colead {
                    output.push_str("colead = ");
                    output.push_str(&colead.name());
                    output.push('\n');
                }
                if let Some(orchestrator) = &choices.orchestrator {
                    output.push_str("orchestrator = ");
                    output.push_str(&orchestrator.name());
                    output.push('\n');
                }
                continue;
            }
            if line == "colead = gpt6astra" || line == "orchestrator = gpt56luna" {
                continue;
            }
        }
        if section == "[workspace]" {
            if line == "workers = colead" {
                if choices.colead.is_some() {
                    output.push_str(line);
                    output.push('\n');
                }
                continue;
            }
            if line == "layout = lead-pair" {
                output.push_str("layout = ");
                output.push_str(if choices.colead.is_some() {
                    "lead-pair"
                } else {
                    "lead-solo"
                });
                output.push('\n');
                continue;
            }
            if line.starts_with("palette = ") {
                continue;
            }
        }
        output.push_str(line);
        output.push('\n');
        if line == "[profiles]" {
            push_picks(&mut output, choices);
        }
        if section == "[workspace]" && line == "watchdog = true" {
            output.push_str("palette = ");
            output.push_str(&choices.palette);
            output.push('\n');
        }
    }
    output
}

/// One generated profile line per distinct pick, ahead of the catalog ones.
fn push_picks(output: &mut String, choices: &Choices) {
    let mut seen: Vec<String> = Vec::new();
    for pick in choices.seats().filter_map(Seat::pick) {
        if seen.contains(&pick.name()) {
            continue;
        }
        if seen.is_empty() {
            output.push_str("# ae init picks: harness, model and effort per role; edit freely.\n");
        }
        let _ = writeln!(output, "{} = \"{}\"", pick.name(), pick.command());
        seen.push(pick.name());
    }
}

fn catalog_client_line(line: &str) -> Option<&str> {
    let (name, executable) = line.split_once(" = ")?;
    (name == executable
        && PROFILE_CATALOG
            .iter()
            .any(|profile| profile.harness == name))
    .then_some(name)
}

fn catalog_profile_line(line: &str) -> Option<&str> {
    let (name, command) = line.split_once(" = \"")?;
    if !command.ends_with('"') {
        return None;
    }
    PROFILE_CATALOG
        .iter()
        .any(|profile| profile.name == name)
        .then_some(name)
}

fn print_discovery(found: &[Discovery], out: &mut impl Write) -> io::Result<()> {
    for item in found {
        if let Some(path) = &item.path {
            writeln!(
                out,
                "{:<9} executable found: {} (login, model access and config home unverified)",
                item.tool,
                path.display()
            )?;
        } else {
            writeln!(out, "{:<9} not on PATH", item.tool)?;
        }
    }
    Ok(())
}

fn prompt_choices(
    args: &Args,
    choices: &mut Choices,
    found: &[Discovery],
    input: &mut impl Read,
    out: &mut impl Write,
) -> io::Result<bool> {
    match prompt_all(args, choices, found, input, out) {
        Err(why) if why.kind() == io::ErrorKind::InvalidData => Ok(false),
        done => done,
    }
}

fn prompt_all(
    args: &Args,
    choices: &mut Choices,
    found: &[Discovery],
    input: &mut impl Read,
    out: &mut impl Write,
) -> io::Result<bool> {
    if args.lead.is_none() {
        let current = Some(&choices.lead);
        let Some(seat) = prompt_role(Role::Lead, &args.lead_parts, current, found, input, out)?
        else {
            return Ok(false);
        };
        choices.lead = seat;
    }
    if args.colead == SeatArg::Unset {
        let current = choices.colead.as_ref();
        choices.colead = prompt_role(Role::Colead, &args.colead_parts, current, found, input, out)?;
    }
    if args.orchestrator == SeatArg::Unset {
        let default = choices
            .orchestrator
            .as_ref()
            .map_or_else(|| "none".to_owned(), Seat::name);
        write!(out, "Orchestrator profile [{default}] (or 'none'): ")?;
        out.flush()?;
        let Some(answer) = read_answer(input)? else {
            return Ok(false);
        };
        if answer == "none" {
            choices.orchestrator = None;
        } else if !answer.is_empty() {
            choices.orchestrator = Some(Seat::Profile(answer));
        }
    }
    if args.palette.is_none() {
        let default = choices.palette.clone();
        write!(out, "Palette [{default}] (darcula, a, or b): ")?;
        out.flush()?;
        let Some(answer) = read_answer(input)? else {
            return Ok(false);
        };
        if !answer.is_empty() {
            choices.palette = answer;
        }
    }
    Ok(validate_choices(choices, found).is_ok())
}

/// One prompt: Enter takes `default`; a missing or unlisted answer is `InvalidData`.
fn ask(
    input: &mut impl Read,
    out: &mut impl Write,
    label: &str,
    default: &str,
    options: &[&str],
    solo: bool,
) -> io::Result<String> {
    let tail = if solo { " or 'solo'" } else { "" };
    write!(out, "{label} [{default}] ({}{tail}): ", options.join(", "))?;
    out.flush()?;
    let mut answer = read_answer(input)?.ok_or(io::ErrorKind::InvalidData)?;
    if answer.is_empty() {
        default.clone_into(&mut answer);
    }
    if options.contains(&answer.as_str()) || (solo && answer == "solo") {
        Ok(answer)
    } else {
        Err(io::ErrorKind::InvalidData.into())
    }
}

/// A role's harness, then its model, then its effort; a flag settles its own
/// step, and `None` is `solo`. Each default is recomputed for what was just
/// chosen, so a harness change never carries the old model or effort across.
fn prompt_role(
    role: Role,
    parts: &Parts,
    current: Option<&Seat>,
    found: &[Discovery],
    input: &mut impl Read,
    out: &mut impl Write,
) -> io::Result<Option<Seat>> {
    let kept = current.and_then(Seat::pick);
    let mut say = |step: &str, default: &str, options: &[&str], solo: bool| {
        ask(
            input,
            out,
            &format!("{role:?} {step}"),
            default,
            options,
            solo,
        )
    };
    let mut harness = parts.harness.clone();
    if harness.is_none() && parts.model.is_none() {
        let options: Vec<&str> = SUPPORTED
            .into_iter()
            .filter(|tool| is_found(found, tool))
            .collect();
        let default = current.map_or("solo", Seat::harness);
        let answer = say("harness", default, &options, role == Role::Colead)?;
        let extra = EXTRA_ORDER.into_iter().find(|(tool, _)| *tool == answer);
        if answer == "solo" || extra.is_some() {
            // Neither has a model or effort, so a flagged one would be dropped.
            if parts.any() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            return Ok(extra.map(|(_, name)| Seat::Profile(name.to_owned())));
        }
        harness = Some(answer);
    }
    let resolve = |model: Option<&str>, effort: Option<&str>| {
        Pick::resolve(role, role.defaults().0, harness.as_deref(), model, effort)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
    };
    let mut model = parts.model.clone();
    if model.is_none() {
        let base = resolve(None, None)?;
        let default = kept
            .filter(|pick| pick.harness == base.harness)
            .unwrap_or(&base);
        let options: Vec<&str> = base.harness.models.iter().map(|row| row.id).collect();
        model = Some(say("model", default.model.id, &options, false)?);
    }
    let mut effort = parts.effort.clone();
    if effort.is_none() {
        let base = resolve(model.as_deref(), None)?;
        let default = kept
            .filter(|pick| pick.model == base.model)
            .unwrap_or(&base);
        effort = Some(say("effort", default.effort, base.model.efforts, false)?);
    }
    resolve(model.as_deref(), effort.as_deref()).map(|pick| Some(Seat::Pick(pick)))
}

fn read_answer(input: &mut impl Read) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        match input.read(&mut byte)? {
            0 => return Ok(None),
            _ if byte[0] == b'\n' => break,
            _ if bytes.len() == MAX_ANSWER_BYTES => return Ok(None),
            _ => bytes.push(byte[0]),
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(String::from_utf8(bytes).ok())
}

#[allow(
    clippy::disallowed_methods,
    reason = "a door: init classifies and reads the selected global config itself before proposing or replacing it — see clippy.toml"
)]
fn existing_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(why) if why.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(why) => return Err(format!("could not inspect {} ({why})", path.display())),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!(
            "{} is not a regular config file; refusing to replace it",
            path.display()
        ));
    }
    fs::read(path)
        .map(Some)
        .map_err(|why| format!("could not read {} ({why})", path.display()))
}

static EXCLUSIVE_TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

fn exclusive_temp(path: &Path, mode: u32) -> io::Result<(PathBuf, fs::File)> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "config path has no file name",
        ));
    };
    loop {
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(
            ".init.{}.{}.tmp",
            std::process::id(),
            EXCLUSIVE_TEMP_NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temp = path.with_file_name(temp_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(why) if why.kind() == io::ErrorKind::AlreadyExists => {}
            Err(why) => return Err(why),
        }
    }
}

fn create_exclusive_before(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    before_publish: impl FnOnce(),
) -> io::Result<()> {
    let (temp, mut file) = exclusive_temp(path, mode)?;
    if let Err(why) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(why);
    }
    before_publish();
    let published = fs::hard_link(&temp, path);
    drop(file);
    let _ = fs::remove_file(&temp);
    published
}

pub(crate) fn create_exclusive(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    create_exclusive_before(path, bytes, mode, || {})
}

fn atomic_replace(path: &Path, bytes: &[u8], epoch: i64) -> io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "config path has no file name",
        ));
    };
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".init.{epoch}.{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);
    create_exclusive(&temp, bytes, 0o600)?;
    if let Err(why) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(why);
    }
    Ok(())
}

fn backup_path(path: &Path, epoch: i64) -> Option<PathBuf> {
    let name = path.file_name()?;
    let mut backup = name.to_os_string();
    backup.push(format!(".{epoch}.bak"));
    Some(path.with_file_name(backup))
}

fn proposed_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let mut proposed = name.to_os_string();
    proposed.push(".proposed");
    Some(path.with_file_name(proposed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLine<'a> {
    Common(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

impl<'a> DiffLine<'a> {
    fn consumes_old(self) -> bool {
        !matches!(self, Self::Added(_))
    }

    fn consumes_new(self) -> bool {
        !matches!(self, Self::Removed(_))
    }

    fn changed(self) -> bool {
        !matches!(self, Self::Common(_))
    }

    fn content(self) -> &'static str {
        match self {
            Self::Common(_) => " ",
            Self::Removed(_) => "-",
            Self::Added(_) => "+",
        }
    }

    fn text(self) -> &'a str {
        match self {
            Self::Common(line) | Self::Removed(line) | Self::Added(line) => line,
        }
    }
}

fn line_diff<'a>(old: &'a str, new: &'a str) -> Vec<DiffLine<'a>> {
    let old: Vec<&str> = old.split_inclusive('\n').collect();
    let new: Vec<&str> = new.split_inclusive('\n').collect();
    let mut lcs = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
    for old_at in (0..old.len()).rev() {
        for new_at in (0..new.len()).rev() {
            lcs[old_at][new_at] = if old[old_at] == new[new_at] {
                lcs[old_at + 1][new_at + 1] + 1
            } else {
                lcs[old_at + 1][new_at].max(lcs[old_at][new_at + 1])
            };
        }
    }

    let mut lines = Vec::with_capacity(old.len() + new.len());
    let (mut old_at, mut new_at) = (0, 0);
    while old_at < old.len() && new_at < new.len() {
        if old[old_at] == new[new_at] {
            lines.push(DiffLine::Common(old[old_at]));
            old_at += 1;
            new_at += 1;
        } else if lcs[old_at + 1][new_at] >= lcs[old_at][new_at + 1] {
            lines.push(DiffLine::Removed(old[old_at]));
            old_at += 1;
        } else {
            lines.push(DiffLine::Added(new[new_at]));
            new_at += 1;
        }
    }
    lines.extend(old[old_at..].iter().copied().map(DiffLine::Removed));
    lines.extend(new[new_at..].iter().copied().map(DiffLine::Added));
    lines
}

fn hunk_bounds(lines: &[DiffLine<'_>]) -> Vec<(usize, usize)> {
    const CONTEXT: usize = 3;
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if !line.changed() {
            continue;
        }
        let start = at.saturating_sub(CONTEXT);
        let end = (at + CONTEXT + 1).min(lines.len());
        if let Some((_, previous_end)) = hunks.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
        } else {
            hunks.push((start, end));
        }
    }
    hunks
}

fn push_diff_line(diff: &mut String, line: DiffLine<'_>) {
    diff.push_str(line.content());
    diff.push_str(line.text());
    if !line.text().ends_with('\n') {
        diff.push('\n');
        diff.push_str("\\ No newline at end of file\n");
    }
}

fn unified_diff(old_path: &Path, new_path: &Path, old: &str, new: &str) -> String {
    let lines = line_diff(old, new);
    let mut diff = format!("--- {}\n+++ {}\n", old_path.display(), new_path.display());
    for (start, end) in hunk_bounds(&lines) {
        let old_before = lines[..start]
            .iter()
            .filter(|line| line.consumes_old())
            .count();
        let new_before = lines[..start]
            .iter()
            .filter(|line| line.consumes_new())
            .count();
        let old_len = lines[start..end]
            .iter()
            .filter(|line| line.consumes_old())
            .count();
        let new_len = lines[start..end]
            .iter()
            .filter(|line| line.consumes_new())
            .count();
        let old_start = old_before + usize::from(old_len != 0);
        let new_start = new_before + usize::from(new_len != 0);
        let _ = writeln!(
            &mut diff,
            "@@ -{old_start},{old_len} +{new_start},{new_len} @@"
        );
        for line in &lines[start..end] {
            push_diff_line(&mut diff, *line);
        }
    }
    diff
}

fn publish(
    path: &Path,
    proposal: &str,
    force: bool,
    now: crate::time::Timestamp,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let old = match existing_bytes(path) {
        Ok(old) => old,
        Err(why) => {
            writeln!(err, "ae init: {why}.")?;
            return Ok(EXIT_FAILED);
        }
    };
    if let Some(parent) = path.parent()
        && let Err(why) = fs::create_dir_all(parent)
    {
        writeln!(
            err,
            "ae init: could not create {} ({why}).",
            parent.display()
        )?;
        return Ok(EXIT_FAILED);
    }
    match (old, force) {
        (None, _) => match create_exclusive(path, proposal.as_bytes(), 0o600) {
            Ok(()) => {
                writeln!(out, "Wrote config to {}", path.display())?;
                Ok(0)
            }
            Err(why) => {
                writeln!(
                    err,
                    "ae init: could not create {} exclusively ({why}); config left untouched.",
                    path.display()
                )?;
                Ok(EXIT_FAILED)
            }
        },
        (Some(bytes), false) => {
            let Some(proposed) = proposed_path(path) else {
                writeln!(
                    err,
                    "ae init: {} is not a config file path.",
                    path.display()
                )?;
                return Ok(EXIT_FAILED);
            };
            let Ok(old) = String::from_utf8(bytes) else {
                writeln!(
                    err,
                    "ae init: {} is not UTF-8; config left untouched.",
                    path.display()
                )?;
                return Ok(EXIT_FAILED);
            };
            if let Err(why) = create_exclusive(&proposed, proposal.as_bytes(), 0o600) {
                writeln!(
                    err,
                    "ae init: could not create {} exclusively ({why}); config left untouched.",
                    proposed.display()
                )?;
                return Ok(EXIT_FAILED);
            }
            write!(out, "{}", unified_diff(path, &proposed, &old, proposal))?;
            writeln!(
                out,
                "Review {}, then move it into place, or rerun with --force.",
                proposed.display()
            )?;
            Ok(0)
        }
        (Some(bytes), true) => {
            let epoch = now.epoch();
            let Some(backup) = backup_path(path, epoch) else {
                writeln!(
                    err,
                    "ae init: {} is not a config file path.",
                    path.display()
                )?;
                return Ok(EXIT_FAILED);
            };
            if let Err(why) = create_exclusive(&backup, &bytes, 0o600) {
                writeln!(
                    err,
                    "ae init: could not create backup {} exclusively ({why}); config left untouched.",
                    backup.display()
                )?;
                return Ok(EXIT_FAILED);
            }
            if let Err(why) = atomic_replace(path, proposal.as_bytes(), epoch) {
                writeln!(
                    err,
                    "ae init: could not replace {} atomically ({why}); config left untouched.",
                    path.display()
                )?;
                return Ok(EXIT_FAILED);
            }
            writeln!(out, "Backed up config to {}", backup.display())?;
            writeln!(out, "Replaced config at {}", path.display())?;
            Ok(0)
        }
    }
}

/// Discover harnesses, obtain or accept choices, then publish the selected
/// global config.
///
/// # Errors
///
/// Propagates writes to the caller's streams and reads from terminal stdin.
pub fn run(
    path: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    run_with(
        path,
        tail,
        &discover(),
        interactive.then(|| std::io::stdin().lock()).as_mut(),
        out,
        err,
    )
}

/// [`run`] with the discovery and the terminal injected: `terminal` is the
/// input to prompt on, or `None` when stdin is not a terminal.
fn run_with(
    path: &Path,
    tail: &[String],
    found: &[Discovery],
    mut terminal: Option<&mut impl Read>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let args = match parse(tail) {
        Ok(args) => args,
        Err(word) => {
            writeln!(err, "ae init: invalid argument or value: {word}")?;
            write!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        }
    };
    print_discovery(found, out)?;
    let Some(defaults) = default_choices(found) else {
        writeln!(
            err,
            "no supported harness on PATH; ae launches: {}",
            SUPPORTED.join(", ")
        )?;
        return Ok(EXIT_FAILED);
    };
    let mut choices = match apply_args(defaults, &args, found) {
        Ok(choices) => choices,
        Err(value) => {
            writeln!(
                err,
                "ae init: selection is unavailable or unsupported: {value}"
            )?;
            write!(err, "{USAGE}")?;
            return Ok(EXIT_USAGE);
        }
    };
    if let Some(input) = terminal.as_deref_mut()
        && !args.yes
        && !prompt_choices(&args, &mut choices, found, input, out)?
    {
        writeln!(
            err,
            "ae init: invalid, incomplete, or overlong answer; config was not written."
        )?;
        return Ok(EXIT_FAILED);
    }
    let now = crate::time::Timestamp::now();
    let proposal = render(found, &choices, now);
    for note in notes(found, &choices) {
        writeln!(out, "note: {note}")?;
    }
    if terminal.is_none() && !args.yes {
        writeln!(out, "\nProposed config for {}:\n{proposal}", path.display())?;
        writeln!(
            err,
            "ae init: stdin is not a terminal; rerun with --yes to write the proposal."
        )?;
        return Ok(EXIT_FAILED);
    }
    publish(path, &proposal, args.force, now, out, err)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "unit fixtures inspect their own temporary config files"
    )]

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("ae-init-unit-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("unit scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn found(tools: &[&str]) -> Vec<Discovery> {
        discover_with(|tool| {
            tools
                .contains(&tool)
                .then(|| PathBuf::from(format!("/bin/{tool}")))
        })
    }

    #[test]
    fn discovery_asks_only_for_supported_executable_names() {
        let mut asked = Vec::new();
        let discovery = discover_with(|tool| {
            asked.push(tool.to_owned());
            (tool == "claude").then(|| PathBuf::from("/fake/claude"))
        });
        assert_eq!(asked, SUPPORTED);
        assert_eq!(discovery[0].path, Some(PathBuf::from("/fake/claude")));
        assert!(discovery[1..].iter().all(|item| item.path.is_none()));
    }

    #[test]
    fn every_found_set_row_has_the_ruled_defaults() {
        let cases = [
            (
                vec!["claude", "codex", "grok"],
                ("opus55-high", Some("sol61-xhigh"), Some("gpt61solx")),
            ),
            (
                vec!["claude"],
                ("opus55-high", Some("opus55-xhigh"), Some("opus55-high")),
            ),
            (
                vec!["codex"],
                ("sol61-high", Some("sol61-xhigh"), Some("gpt61solx")),
            ),
            (vec!["grok", "agy"], ("grok46", Some("agy"), Some("grok46"))),
            (vec!["grok"], ("grok46", None, Some("grok46"))),
            (vec!["agy"], ("agy", None, Some("agy"))),
            (vec!["opencode"], ("opencode", None, Some("opencode"))),
            (vec!["gemini"], ("gemini", None, Some("gemini"))),
            (
                vec!["agy", "opencode"],
                ("agy", Some("opencode"), Some("agy")),
            ),
        ];
        for (tools, expected) in cases {
            let choices = default_choices(&found(&tools)).expect("a supported harness");
            let name = |seat: &Option<Seat>| seat.as_ref().map(Seat::name);
            assert_eq!(
                (
                    choices.lead.name(),
                    name(&choices.colead),
                    name(&choices.orchestrator)
                ),
                (
                    expected.0.to_owned(),
                    expected.1.map(str::to_owned),
                    expected.2.map(str::to_owned)
                ),
                "{tools:?}"
            );
        }
        assert_eq!(default_choices(&found(&[])), None);
    }

    #[test]
    fn rendered_profiles_and_roster_share_the_catalog() {
        let discovered = found(&["claude", "grok"]);
        let choices = default_choices(&discovered).expect("defaults");
        let text = render(
            &discovered,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        assert!(text.starts_with("# written by ae init on 2026-09-08;"));
        assert!(text.contains("\n[clients]\n"));
        assert!(text.contains("\nclaude = claude\n"));
        assert!(text.contains("\ngrok = grok\n"));
        assert!(!text.contains("\ncodex = codex\n"));
        assert!(
            text.contains(
                "\nlead = opus55-high\ncolead = opus55-xhigh\norchestrator = opus55-high\n"
            )
        );
        assert!(text.contains("grok46 = \"grok"));
        assert!(!text.contains("astrax = \"codex"));
        assert!(text.contains("palette = darcula\n"));
        for seat in choices.seats() {
            assert!(text.contains(&format!("{} = \"", seat.name())), "{seat:?}");
        }
        assert_eq!(
            text.matches("\nopus55-high = ").count(),
            1,
            "one line per distinct pick"
        );

        let solo_found = found(&["gemini"]);
        let solo = default_choices(&solo_found).expect("solo defaults");
        let solo_text = render(
            &solo_found,
            &solo,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        assert!(solo_text.contains("layout = lead-solo\n"));
        assert!(!solo_text.contains("layout = vertical\n"));
    }

    #[test]
    fn a_one_line_roster_change_has_one_removed_and_one_added_diff_line() {
        let old = crate::entry::DEFAULT_CONFIG;
        let new = old.replacen("lead = fable5", "lead = fablex", 1);
        let diff = unified_diff(Path::new("config"), Path::new("config.proposed"), old, &new);
        let removed: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .collect();
        let added: Vec<&str> = diff
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .collect();
        assert_eq!(removed, vec!["-lead = fable5"]);
        assert_eq!(added, vec!["+lead = fablex"]);
        assert!(diff.lines().any(|line| line.starts_with(' ')));
        assert!(diff.lines().count() < 20, "context is not bounded:\n{diff}");
    }

    #[test]
    fn flags_override_or_drop_each_choice_and_reject_unavailable_values() {
        let discovered = found(&["claude", "codex", "grok"]);
        let args = parse(&[
            "--lead".to_owned(),
            "grok46".to_owned(),
            "--solo".to_owned(),
            "--no-orchestrator".to_owned(),
            "--palette".to_owned(),
            "b".to_owned(),
        ])
        .expect("valid flags");
        let choices = apply_args(
            default_choices(&discovered).expect("defaults"),
            &args,
            &discovered,
        )
        .expect("available override");
        assert_eq!(choices.lead, Seat::Profile("grok46".to_owned()));
        assert_eq!(choices.colead, None);
        assert_eq!(choices.orchestrator, None);
        assert_eq!(choices.palette, "b");

        let unavailable = parse(&["--lead".to_owned(), "agy".to_owned()]).expect("shape");
        assert_eq!(
            apply_args(
                default_choices(&discovered).expect("defaults"),
                &unavailable,
                &discovered
            ),
            Err("agy".to_owned())
        );
        assert!(
            parse(&[
                "--solo".to_owned(),
                "--colead".to_owned(),
                "opusx".to_owned()
            ])
            .is_err()
        );
    }

    #[test]
    fn terminal_answers_are_bounded_and_validated_before_a_write() {
        let discovered = found(&["claude", "codex"]);
        let args = Args::default();
        let mut choices = default_choices(&discovered).expect("defaults");
        let mut out = Vec::new();
        assert!(
            prompt_choices(
                &args,
                &mut choices,
                &discovered,
                &mut "grok46\n".as_bytes(),
                &mut out
            )
            .is_ok_and(|accepted| !accepted)
        );
        let overlong = format!("{}\n", "x".repeat(MAX_ANSWER_BYTES + 1));
        assert!(read_answer(&mut overlong.as_bytes()).is_ok_and(|answer| answer.is_none()));
        assert!(read_answer(&mut "partial".as_bytes()).is_ok_and(|answer| answer.is_none()));
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    fn prompted(tools: &[&str], script: &str) -> (Choices, String, bool) {
        let discovered = found(tools);
        let mut choices = default_choices(&discovered).expect("defaults");
        let mut out = Vec::new();
        let accepted = prompt_choices(
            &Args::default(),
            &mut choices,
            &discovered,
            &mut script.as_bytes(),
            &mut out,
        )
        .expect("scripted terminal");
        (
            choices,
            String::from_utf8_lossy(&out).into_owned(),
            accepted,
        )
    }

    #[test]
    fn a_harness_change_recomputes_the_model_and_effort_defaults() {
        let (choices, out, accepted) =
            prompted(&["claude", "codex"], "codex\n\n\nclaude\n\n\n\n\n");
        assert!(accepted, "{out}");
        assert_eq!(choices.lead.name(), "sol61-high");
        assert_eq!(
            choices.colead.as_ref().map(Seat::name).as_deref(),
            Some("opus55-xhigh")
        );
        assert!(out.contains("Lead model [gpt-6.1-sol]"), "{out}");
        assert!(out.contains("Colead model [claude-opus-5-5]"), "{out}");
        assert!(
            out.contains("Colead effort [xhigh] (low, medium, high, xhigh, max)"),
            "{out}"
        );
    }

    #[test]
    fn solo_skips_the_colead_model_and_effort_and_a_flag_skips_its_own_step() {
        let (choices, out, accepted) = prompted(&["claude", "codex"], "\n\n\nsolo\n\n\n");
        assert!(accepted && choices.colead.is_none(), "{out}");
        assert!(
            !out.contains("Colead model") && !out.contains("Colead effort"),
            "{out}"
        );

        let discovered = found(&["claude", "codex"]);
        let args = parse(&words(&[
            "--lead-model",
            "claude-sonnet-5-5",
            "--colead",
            "solx",
        ]))
        .expect("flags");
        let mut choices = apply_args(default_choices(&discovered).expect("d"), &args, &discovered)
            .expect("applied");
        let mut out = Vec::new();
        let script = "medium\n\n\n";
        assert!(
            prompt_choices(
                &args,
                &mut choices,
                &discovered,
                &mut script.as_bytes(),
                &mut out
            )
            .expect("scripted")
        );
        let out = String::from_utf8_lossy(&out);
        assert!(
            !out.contains("Lead harness") && !out.contains("Lead model"),
            "{out}"
        );
        assert!(!out.contains("Colead"), "{out}");
        assert_eq!(choices.lead.name(), "sonnet55-medium");
    }

    #[test]
    fn invalid_overlong_and_eof_answers_exit_one_and_write_nothing() {
        let discovered = found(&["claude", "codex"]);
        let scratch = Scratch::new("bad-answers");
        let path = scratch.0.join("config");
        let overlong = "x".repeat(MAX_ANSWER_BYTES + 1);
        for script in [
            "bogus\n",
            "codex\nmodel-x\n",
            "codex\n\nultra-x\n",
            "",
            overlong.as_str(),
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(
                &path,
                &[],
                &discovered,
                Some(&mut script.as_bytes()),
                &mut out,
                &mut err,
            )
            .expect("streams");
            assert_eq!(code, EXIT_FAILED, "{script:?}");
            assert!(!path.exists(), "{script:?}");
            assert!(String::from_utf8_lossy(&err).contains("config was not written"));
        }
    }

    #[test]
    fn a_flagged_component_is_never_dropped_by_a_table_less_or_solo_terminal_choice() {
        let discovered = found(&["claude", "codex", "grok"]);
        let scratch = Scratch::new("flagged-then-table-less");
        let path = scratch.0.join("config");
        let cases: [(&[&str], &str); 3] = [
            (&["--lead-effort", "low", "--colead", "solx"], "grok\n"),
            (&["--colead-effort", "low", "--lead", "fablex"], "grok\n"),
            (&["--colead-effort", "low", "--lead", "fablex"], "solo\n"),
        ];
        for (flags, script) in cases {
            let mut tail = words(&["--no-orchestrator", "--palette", "a"]);
            tail.extend(words(flags));
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(
                &path,
                &tail,
                &discovered,
                Some(&mut script.as_bytes()),
                &mut out,
                &mut err,
            )
            .expect("streams");
            assert_eq!(code, EXIT_FAILED, "{flags:?} {script:?}");
            assert!(!path.exists(), "{flags:?} {script:?}");
        }
    }

    #[test]
    fn role_flags_refuse_conflicts_and_unsupported_values_before_any_write() {
        let discovered = found(&["claude", "codex"]);
        let scratch = Scratch::new("refusals");
        let path = scratch.0.join("config");
        let cases: &[&[&str]] = &[
            &["--lead-harness", "claude", "--lead-effort", "ultra"],
            &["--lead-model", "gpt-6-luna", "--lead-effort", "ultra"],
            &[
                "--colead-harness",
                "claude",
                "--colead-model",
                "gpt-6.1-sol",
            ],
            &["--lead-harness", "grok"],
            &["--lead", "fablex", "--lead-effort", "low"],
            &["--colead-effort", "low", "--colead", "solx"],
            &["--colead-model", "gpt-6.1-sol", "--solo"],
            &["--solo", "--colead-model", "gpt-6.1-sol"],
            &["--lead-model", "claude-opus-5-5", "--lead-model", "fable"],
            &["--lead-effort"],
        ];
        for flags in cases {
            let mut tail = words(&["--yes"]);
            tail.extend(words(flags));
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_with(
                &path,
                &tail,
                &discovered,
                None::<&mut &[u8]>,
                &mut out,
                &mut err,
            )
            .expect("streams");
            assert_eq!(code, EXIT_USAGE, "{flags:?}");
            assert!(String::from_utf8_lossy(&err).contains(USAGE), "{flags:?}");
            assert!(!path.exists(), "{flags:?}");
        }
        let lone = found(&["claude"]);
        let args = parse(&words(&["--colead-harness", "codex"])).expect("shape");
        assert_eq!(
            apply_args(default_choices(&lone).expect("d"), &args, &lone),
            Err("codex".to_owned())
        );
    }

    #[test]
    fn rendered_picks_parse_resolve_and_dedupe() {
        let discovered = found(&["claude", "codex"]);
        let args = parse(&words(&[
            "--lead-effort",
            "low",
            "--colead-harness",
            "claude",
        ]))
        .expect("flags");
        let choices = apply_args(default_choices(&discovered).expect("d"), &args, &discovered)
            .expect("applied");
        let text = render(
            &discovered,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        let config = crate::config::parse_identity(&text).expect("ordinary INI");
        for role in ["lead", "colead", "orchestrator"] {
            let (_, profile) = config
                .roster
                .iter()
                .find(|(name, _)| name == role)
                .expect("role");
            assert!(
                matches!(config.command(profile, None), Ok(Some(_))),
                "{role}"
            );
        }
        let lead = text
            .lines()
            .find(|line| line.starts_with("opus55-low = "))
            .expect("lead line");
        assert_eq!(
            lead,
            "opus55-low = \"claude --permission-mode bypassPermissions --model claude-opus-5-5 --effort low\""
        );
        assert!(text.contains("\nopus55-xhigh = \"claude "), "{text}");

        let claude_only = found(&["claude"]);
        let choices = default_choices(&claude_only).expect("d");
        let text = render(
            &claude_only,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        assert_eq!(text.matches("\nopus55-high = ").count(), 1);
        assert!(text.contains("orchestrator = opus55-high\n"));

        let args = parse(&words(&["--lead-effort", "low"])).expect("flags");
        let choices = apply_args(
            default_choices(&claude_only).expect("d"),
            &args,
            &claude_only,
        )
        .expect("applied");
        let text = render(
            &claude_only,
            &choices,
            crate::time::Timestamp::from_epoch(1_788_825_600),
        );
        let config = crate::config::parse_identity(&text).expect("ordinary INI");
        for (role, effort) in [
            ("lead", "low"),
            ("colead", "xhigh"),
            ("orchestrator", "high"),
        ] {
            let (_, profile) = config
                .roster
                .iter()
                .find(|(name, _)| name == role)
                .expect("role");
            let resolved = config
                .command(profile, None)
                .expect("resolves")
                .expect("defined");
            let command = resolved.as_str();
            assert!(
                command.ends_with(&format!("--effort {effort}")),
                "{role}: {command}"
            );
        }
    }

    #[test]
    fn known_different_providers_need_no_caution_but_unverified_pairs_do() {
        let xai_google = found(&["grok", "agy"]);
        let choices = default_choices(&xai_google).expect("defaults");
        assert!(notes(&xai_google, &choices).is_empty());

        let unverified = found(&["agy", "opencode"]);
        let choices = default_choices(&unverified).expect("defaults");
        assert!(notes(&unverified, &choices)[0].starts_with("provider unverified:"));

        for tool in ["claude", "codex"] {
            let one_harness = found(&[tool]);
            let choices = default_choices(&one_harness).expect("defaults");
            assert!(notes(&one_harness, &choices)[0].starts_with("same provider:"));
        }

        let one_solo = found(&["gemini"]);
        let choices = default_choices(&one_solo).expect("defaults");
        assert!(notes(&one_solo, &choices)[0].starts_with("single provider:"));
    }

    #[test]
    fn catalog_and_default_config_cannot_drift_apart() {
        for harness in SUPPORTED {
            assert!(
                crate::entry::DEFAULT_CONFIG
                    .lines()
                    .any(|line| catalog_client_line(line) == Some(harness)),
                "{harness}"
            );
        }
        let client_section = crate::entry::DEFAULT_CONFIG
            .split_once("[clients]\n")
            .and_then(|(_, tail)| tail.split_once("\n[profiles]"))
            .map_or("", |(clients, _)| clients);
        for line in client_section
            .lines()
            .filter(|line| !line.starts_with('#') && line.contains(" = "))
        {
            assert!(catalog_client_line(line).is_some(), "uncatalogued: {line}");
        }
        for profile in PROFILE_CATALOG {
            assert!(
                crate::entry::DEFAULT_CONFIG
                    .lines()
                    .any(|line| catalog_profile_line(line) == Some(profile.name)),
                "{}",
                profile.name
            );
        }
        let profile_section = crate::entry::DEFAULT_CONFIG
            .split_once("[profiles]\n")
            .and_then(|(_, tail)| tail.split_once("\n[roster]"))
            .map_or("", |(profiles, _)| profiles);
        for line in profile_section
            .lines()
            .filter(|line| !line.starts_with('#') && line.contains(" = \""))
        {
            assert!(catalog_profile_line(line).is_some(), "uncatalogued: {line}");
        }
    }

    #[test]
    fn exclusive_config_publish_has_one_whole_winner() {
        let scratch = Scratch::new("race");
        let path = std::sync::Arc::new(scratch.0.join("config"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for text in [b"seeded\n".as_slice(), b"initialized\n".as_slice()] {
            let path = std::sync::Arc::clone(&path);
            let barrier = std::sync::Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                create_exclusive(&path, text, 0o600)
            }));
        }
        barrier.wait();
        let outcomes: Vec<bool> = threads
            .into_iter()
            .map(|thread| thread.join().expect("publisher joins").is_ok())
            .collect();
        assert_eq!(outcomes.iter().filter(|won| **won).count(), 1);
        let written = fs::read(&*path).expect("winner is readable");
        assert!(written == b"seeded\n" || written == b"initialized\n");
    }

    #[test]
    fn a_reader_never_sees_an_exclusive_publish_before_complete_bytes() {
        let scratch = Scratch::new("paused-writer");
        let path = scratch.0.join("config");
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            create_exclusive_before(&writer_path, b"complete config\n", 0o600, || {
                ready_tx.send(()).expect("announce pause");
                release_rx.recv().expect("release publisher");
            })
        });
        ready_rx.recv().expect("writer paused before publication");
        let visible_before_seed = fs::read(&path);
        let mut err = Vec::new();
        let seed_result =
            crate::seed_default_config(&path, "complete config\n", "default", &mut err)
                .expect("seed probe");
        let visible_after_seed = fs::read(&path);
        release_tx.send(()).expect("release paused writer");
        let writer_result = writer.join().expect("publisher joins");

        assert_eq!(
            visible_before_seed
                .expect_err("unpublished path is absent")
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(seed_result, None);
        assert_eq!(
            visible_after_seed.expect("seed winner is readable"),
            b"complete config\n"
        );
        assert_eq!(
            writer_result.expect_err("paused publisher loses").kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read(&path).expect("published path is readable"),
            b"complete config\n"
        );
    }

    #[test]
    fn a_backup_collision_prevents_force_from_replacing_the_config() {
        let scratch = Scratch::new("backup-collision");
        let config = scratch.0.join("config");
        fs::write(&config, b"original\n").expect("old config");
        let now = crate::time::Timestamp::from_epoch(123);
        let backup = backup_path(&config, now.epoch()).expect("backup name");
        fs::write(&backup, b"standing\n").expect("standing backup");
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            publish(&config, "replacement\n", true, now, &mut out, &mut err)
                .expect("stream writes"),
            EXIT_FAILED
        );
        assert_eq!(fs::read(&config).expect("config"), b"original\n");
        assert_eq!(fs::read(&backup).expect("backup"), b"standing\n");
    }
}
