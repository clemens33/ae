//! The identity v2 core entries: `_launch-plan`, `_meta-init` and `_roster`.
//!
//! Four things the core decides for an alias-free session identity — which
//! config the workspace resolves to,
//! what the first meta says, how a seat is added or removed, and what the
//! roster currently is. Each is an underscored core entry (never human-typed)
//! and each speaks ONE framing, described below.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{self, IdentityConfig};
use crate::launch_cmd;
use crate::meta::{self, Meta};
use crate::roster::{self, SeatLines};

/// The unit separator that frames every record these entries print.
pub const US: char = '\u{1f}';

/// [`US`] as a string, for joining.
const SEP: &str = "\u{1f}";

/// The field a record uses to say "absent" — an empty field would be
/// indistinguishable from a field the writer forgot.
const NONE: &str = "-";

/// A refusal: the reason is on stderr and nothing was published.
pub const EXIT_REFUSED: u8 = 1;

/// A usage error: argv itself was wrong.
pub const EXIT_USAGE: u8 = 2;

/// One record: `fields` joined by [`US`] and `\n`-terminated.
///
/// # Errors
///
/// The first field that would break the framing — one carrying the separator
/// (which forges fields) or a newline (which splits the record).
fn record(fields: &[&str]) -> Result<String, String> {
    if let Some(bad) = fields.iter().find(|f| f.contains(US) || f.contains('\n')) {
        return Err((*bad).to_owned());
    }
    Ok(format!("{}\n", fields.join(SEP)))
}

/// The trailer every stdout ends with.
fn trailer(count: usize) -> String {
    format!("end{US}{count}\n")
}

/// Write a built document to stdout.
fn emit(body: &str, out: &mut impl Write) -> crate::Result<u8> {
    out.write_all(body.as_bytes())?;
    Ok(0)
}

/// A refusal: one line on stderr, nothing on stdout.
fn refuse(message: &str, err: &mut impl Write) -> crate::Result<u8> {
    writeln!(err, "Error: {message}")?;
    Ok(EXIT_REFUSED)
}

/// A usage error: the offending word named on stderr.
fn usage(entry: &str, word: &str, err: &mut impl Write) -> crate::Result<u8> {
    writeln!(err, "ae: {entry}: unexpected argument: {word}")?;
    Ok(EXIT_USAGE)
}

/// Whether a field carries a control byte that a `key=value` record file, or
/// this framing, cannot round-trip.
fn control_free(field: &str) -> bool {
    !field.chars().any(char::is_control)
}

/// The `--global` / `--local` pair two entries share.
#[derive(Debug, Default)]
struct ConfigFiles {
    global: Option<PathBuf>,
    local: Option<PathBuf>,
}

impl ConfigFiles {
    /// Read the identity config these files select.
    fn read(&self) -> Result<IdentityConfig, config::ConfigError> {
        config::read_identity(self.global.as_deref(), self.local.as_deref())
    }
}

/// Read `--global`/`--local` out of `tail`, refusing any other word.
///
/// # Errors
///
/// The offending word: an unknown flag, or a flag with no value.
fn config_files(tail: &[String]) -> Result<ConfigFiles, String> {
    let mut files = ConfigFiles::default();
    let mut rest = tail;
    while let [flag, after @ ..] = rest {
        let Some((value, tail)) = after.split_first() else {
            return Err(flag.clone());
        };
        match flag.as_str() {
            "--global" => files.global = Some(value.into()),
            "--local" => files.local = Some(value.into()),
            _ => return Err(flag.clone()),
        }
        rest = tail;
    }
    Ok(files)
}

/// `_launch-plan`'s argv.
#[derive(Debug, Default)]
struct LaunchFlags {
    files: ConfigFiles,
    /// `--main <name>`: the launch line's `use <name>`, replacing
    /// `[workspace] main` for this launch.
    main: Option<String>,
    /// `--workers <a,b>`: REPLACES `[workspace] workers` for this launch —
    /// compact's frozen roster passes the names it froze.
    workers: Option<String>,
}

/// Read `_launch-plan`'s flags.
///
/// # Errors
///
/// The offending word: an unknown flag, or a flag with no value.
fn launch_flags(tail: &[String]) -> Result<LaunchFlags, String> {
    let mut flags = LaunchFlags::default();
    let mut rest = tail;
    while let [flag, after @ ..] = rest {
        let Some((value, tail)) = after.split_first() else {
            return Err(flag.clone());
        };
        match flag.as_str() {
            "--global" => flags.files.global = Some(value.into()),
            "--local" => flags.files.local = Some(value.into()),
            "--main" => flags.main = Some(value.clone()),
            "--workers" => flags.workers = Some(value.clone()),
            _ => return Err(flag.clone()),
        }
        rest = tail;
    }
    Ok(flags)
}

/// `_launch-plan [--global <f>] [--local <f>] [--main <name>] [--workers <a,b>]`
/// — resolve the workspace into the seats a launch will create.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
pub fn launch_plan(
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let flags = match launch_flags(tail) {
        Ok(flags) => flags,
        Err(word) => return usage(crate::cli::LAUNCH_PLAN, &word, err),
    };
    let mut cfg = match flags.files.read() {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_REFUSED);
        }
    };
    if let Some(workers) = &flags.workers {
        // `-` is the shell-safe spelling of "none": a caller passing an empty
        // argument is easy to write and easy to lose, and both must mean the
        // same thing or a frozen roster with no workers would silently keep
        // the config's.
        cfg.workers = Some(if workers == NONE {
            String::new()
        } else {
            workers.clone()
        });
    }
    let home = crate::doors::home();
    let plan = match config::launch_plan(&cfg, flags.main.as_deref(), home.as_deref()) {
        Ok(plan) => plan,
        Err(violations) => {
            write!(err, "{}", config::render_violations(&violations))?;
            return Ok(EXIT_REFUSED);
        }
    };
    let mut body = String::new();
    for seat in &plan.seats {
        match record(&[
            "seat",
            &seat.slot,
            &seat.name,
            &seat.profile,
            &seat.binary,
            seat.tool.as_str(),
            NONE,
            seat.command.as_str(),
        ]) {
            Ok(line) => body.push_str(&line),
            Err(bad) => {
                return refuse(
                    &format!(
                        "[profiles] {}: the launch command contains a control byte (U+001F or newline) that would corrupt the plan record: {bad:?}",
                        seat.profile
                    ),
                    err,
                );
            }
        }
    }
    body.push_str(&trailer(plan.seats.len()));
    emit(&body, out)
}

/// `_meta-init`'s argv.
#[derive(Debug, Default)]
struct MetaInitFlags {
    base: Option<PathBuf>,
    replace: bool,
}

/// Read `_meta-init`'s flags.
///
/// # Errors
///
/// The offending word: an unknown flag, or `--base` with no value.
fn meta_init_flags(tail: &[String]) -> Result<MetaInitFlags, String> {
    let mut flags = MetaInitFlags::default();
    let mut rest = tail;
    while let [flag, after @ ..] = rest {
        if flag == "--replace" {
            flags.replace = true;
            rest = after;
            continue;
        }
        let Some((value, tail)) = after.split_first() else {
            return Err(flag.clone());
        };
        match flag.as_str() {
            "--base" => flags.base = Some(value.into()),
            _ => return Err(flag.clone()),
        }
        rest = tail;
    }
    Ok(flags)
}

/// Parse `_meta-init`'s stdin: `seat<US>slot<US>name<US>profile<US>binary<US>sid`
/// records and the `end<US><count>` trailer.
///
/// # Errors
///
/// The refusal text, ready for stderr.
/// `restored` is the `--replace` publish: a resume republishing the records
/// `_roster list` just handed back out of the session's OWN meta. Those names
/// were minted before this grammar may have existed, and refusing them here
/// would make that session unresumable with nothing the human could edit to
/// fix it (the name the tool refuses is the one it refuses to read). So a
/// restored name is taken verbatim — non-empty and control-free — and left to
/// the interpolation-site guard, which drops the identity line quietly. A
/// FRESH publish (a launch) validates the grammar: its records came out of
/// `_launch-plan`, so this is defence in depth against the glue, not a boundary.
fn parse_seat_records(stdin: &str, restored: bool) -> Result<Vec<SeatLines>, String> {
    let mut lines: Vec<&str> = stdin.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let Some((last, rows)) = lines.split_last() else {
        return Err(
            "_meta-init read no records at all on stdin — not even the end trailer.".to_owned(),
        );
    };
    let trailer: Vec<&str> = last.split(US).collect();
    let declared = match trailer.as_slice() {
        ["end", count] => count
            .parse::<usize>()
            .map_err(|_| format!("the end trailer's count is not a number: {count:?}"))?,
        _ => {
            return Err(format!(
                "the last stdin record is not the end trailer: {last:?}"
            ));
        }
    };
    if declared != rows.len() {
        return Err(format!(
            "the end trailer declares {declared} seat records, but stdin carried {}",
            rows.len()
        ));
    }
    let mut seats: Vec<SeatLines> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let fields: Vec<&str> = row.split(US).collect();
        let ["seat", slot, name, profile, binary, sid] = fields.as_slice() else {
            return Err(format!(
                "stdin record {} is not a 6-field seat record: {row:?}",
                index + 1
            ));
        };
        if !fields.iter().copied().all(control_free) {
            return Err(format!(
                "stdin record {} carries a control byte no meta line can round-trip: {row:?}",
                index + 1
            ));
        }
        if slot.is_empty() || slot.contains('=') {
            return Err(format!(
                "stdin record {} has no usable slot: {slot:?}",
                index + 1
            ));
        }
        if restored {
            if name.is_empty() {
                return Err(format!(
                    "stdin record {}: a restored seat has no name at all.",
                    index + 1
                ));
            }
        } else if !config::is_agent_name(name) {
            return Err(format!(
                "stdin record {}: invalid agent name {name:?}. Names must match {}.",
                index + 1,
                config::AGENT_NAME_GRAMMAR
            ));
        }
        if profile.is_empty() {
            return Err(format!(
                "stdin record {}: seat '{name}' names no profile — a v2 seat is a name bound to a profile.",
                index + 1
            ));
        }
        if seats.iter().any(|seat| seat.name == *name) {
            return Err(format!(
                "the name {name:?} is claimed by more than one seat."
            ));
        }
        if seats.iter().any(|seat| seat.slot == *slot) {
            return Err(format!(
                "the slot {slot:?} is claimed by more than one seat."
            ));
        }
        seats.push(SeatLines {
            slot: (*slot).to_owned(),
            name: (*name).to_owned(),
            profile: (*profile).to_owned(),
            // The 6-field stdin record predates the client row and names no
            // override; absence of evidence, never a derived label.
            client: None,
            binary: optional(binary),
            harness_session: optional(sid),
            config_home: None,
            config_home_base: None,
            // `_meta-init` seats start in the session dir; only a spawn
            // records an explicit target.
            work_dir: None,
        });
    }
    Ok(seats)
}

/// A record field that may say "absent": [`NONE`] and the empty string both do.
fn optional(field: &str) -> Option<String> {
    (field != NONE && !field.is_empty()).then(|| field.to_owned())
}

/// `_meta-init <dir> --base <file> [--replace]` — publish a session's whole
/// meta as ONE document.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
pub fn meta_init(
    dir: &Path,
    tail: &[String],
    stdin: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let flags = match meta_init_flags(tail) {
        Ok(flags) => flags,
        Err(word) => return usage(crate::cli::META_INIT, &word, err),
    };
    let Some(base) = flags.base else {
        writeln!(err, "ae: {} needs --base <file>", crate::cli::META_INIT)?;
        return Ok(EXIT_USAGE);
    };
    let mut seats = match parse_seat_records(stdin, flags.replace) {
        Ok(seats) => seats,
        Err(why) => return refuse(&why, err),
    };
    if flags.replace {
        // The 6-field stdin record predates the client row and cannot carry
        // it — so a replace PRESERVES the current per-slot rows instead of
        // laundering every `Label` into `Missing`. A fresh publish keeps its
        // absent rows: nothing recorded an override there.
        let current = match meta::read_bytes(dir) {
            Ok(bytes) => Meta::parse(&String::from_utf8_lossy(&bytes)),
            Err(why) => {
                return refuse(
                    &format!("cannot read the current meta to preserve its client rows: {why}"),
                    err,
                );
            }
        };
        for seat in &mut seats {
            let Some(entry) = current
                .roster()
                .iter()
                .find(|entry| entry.slot == seat.slot)
            else {
                continue;
            };
            match &entry.client {
                meta::RecordedClient::Label(label) => {
                    seat.client = Some(label.clone());
                }
                meta::RecordedClient::Missing => {}
                // Unrepresentable in a republish — and dropping it toward
                // `Missing` would be a silent fallback, so the replace
                // refuses instead.
                meta::RecordedClient::Invalid => {
                    return refuse(
                        &format!(
                            "seat '{}' ({}) records an unusable client (client.{} is empty, duplicated or malformed) — \
                             fix the meta row before replacing",
                            seat.name, seat.slot, seat.slot
                        ),
                        err,
                    );
                }
            }
        }
    }
    let facts = match meta::read_base(&base) {
        Ok(text) => text,
        Err(why) => {
            return refuse(
                &format!("cannot read the base facts {}: {why}", base.display()),
                err,
            );
        }
    };
    if !facts.is_empty() && !facts.ends_with('\n') {
        return refuse(
            &format!(
                "the base facts {} do not end in a newline — appending the roster would fuse two records into one line.",
                base.display()
            ),
            err,
        );
    }
    let content = format!("{facts}{}", roster::render(&seats));
    let published = if flags.replace {
        meta::replace(dir, &content)
    } else {
        meta::init(dir, &content)
    };
    if let Err(why) = published {
        return match why {
            meta::RewriteError::NotWritten(cause) => refuse(
                &format!("the meta was not published, and nothing changed: {cause}"),
                err,
            ),
            meta::RewriteError::Unknown(cause) => refuse(
                &format!(
                    "the meta IS published but its directory entry was not synced, so whether it survives a crash is unknown: {cause}"
                ),
                err,
            ),
        };
    }
    if let Err(why) = std::fs::remove_file(&base) {
        writeln!(
            err,
            "ae: the meta is published, but the consumed base facts {} could not be removed: {why}",
            base.display()
        )?;
    }
    emit(&trailer(0), out)
}

/// `_roster <dir> <subcommand> …` — the five roster operations.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
pub fn roster(
    dir: &Path,
    tail: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let Some((subcommand, rest)) = tail.split_first() else {
        writeln!(
            err,
            "ae: {} needs a subcommand: add-seat, remove-seat, set-harness-session or list",
            crate::cli::ROSTER
        )?;
        return Ok(EXIT_USAGE);
    };
    match (subcommand.as_str(), rest) {
        ("add-seat", [name, flags @ ..]) => add_seat(dir, name, flags, out, err),
        ("remove-seat", [name]) => remove_seat(dir, name, out, err),
        ("set-harness-session", [slot, sid]) => set_harness_session(dir, slot, sid, out, err),
        ("list", flags) => list(dir, flags, out, err),
        ("add-seat" | "remove-seat" | "set-harness-session", _) => {
            writeln!(
                err,
                "ae: {} {subcommand}: wrong number of operands",
                crate::cli::ROSTER
            )?;
            Ok(EXIT_USAGE)
        }
        _ => usage(crate::cli::ROSTER, subcommand, err),
    }
}

/// Every anomaly that puts the roster's IDENTITY in doubt, rendered: a v1
/// row, one name on two seats, a malformed roster row or line, and a
/// duplicate or unreadable key under an identity prefix.
fn identity_doubts(current: &Meta) -> Vec<String> {
    current
        .anomalies()
        .iter()
        .filter(|anomaly| roster::roster_doubting(anomaly))
        .map(ToString::to_string)
        .collect()
}

/// The meta's text.
///
/// # Errors
///
/// The refusal text: the read failed, or the document is not UTF-8.
fn text_of(dir: &Path) -> Result<String, String> {
    let bytes = meta::read_bytes(dir).map_err(|why| format!("cannot read the meta: {why}"))?;
    String::from_utf8(bytes).map_err(|_| "the meta is not valid UTF-8".to_owned())
}

/// The parsed meta, for the two entries that need no bytes.
///
/// # Errors
///
/// The refusal text: the read failed, or the document is not UTF-8.
fn parse_meta(dir: &Path) -> Result<Meta, String> {
    text_of(dir).map(|text| Meta::parse(&text))
}

/// Publish `content` as the whole meta under a lock the caller holds, turning
/// either failure into its refusal text.
///
/// # Errors
///
/// The refusal text, which says WHAT IS KNOWN: nothing changed, or the meta is
/// visible but its directory entry is not known to be durable.
fn publish(dir: &Path, content: &str) -> Result<(), String> {
    meta::publish_locked(dir, content).map_err(|why| match why {
        meta::RewriteError::NotWritten(cause) => {
            format!("the meta was not published, and nothing changed: {cause}")
        }
        meta::RewriteError::Unknown(cause) => format!(
            "the meta IS published but its directory entry was not synced, so whether it survives a crash is unknown: {cause}"
        ),
    })
}

/// Every record of `text`, as [`meta::rewritten`] counts them: what precedes
/// each `\n`, plus a final unterminated remainder.
fn records(text: &str) -> Vec<&str> {
    let mut rows: Vec<&str> = text.split('\n').collect();
    if rows.last() == Some(&"") {
        rows.pop();
    }
    rows
}

/// A record's KEY — everything before its first `=`, a record without one being
/// its own key.
fn key_of(record: &str) -> &str {
    record.split_once('=').map_or(record, |(key, _)| key)
}

/// `_roster <dir> add-seat <name> --using <profile> --binary <bin> [--session <sid>]`
/// — take the lowest free `spawned.<n>` and append the seat.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
fn add_seat(
    dir: &Path,
    name: &str,
    flags: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let AddSeatFlags {
        profile,
        binary,
        sid,
    } = match add_seat_flags(flags) {
        Ok(parsed) => parsed,
        Err(word) => return usage(crate::cli::ROSTER, &word, err),
    };
    let (Some(profile), Some(binary)) = (profile, binary) else {
        writeln!(
            err,
            "ae: {} add-seat needs --using <profile> and --binary <bin>",
            crate::cli::ROSTER
        )?;
        return Ok(EXIT_USAGE);
    };
    match add_seat_slot(dir, name, &profile, &binary, sid.as_deref(), None) {
        Ok(slot) => {
            let mut body = match record(&["slot", &slot]) {
                Ok(line) => line,
                Err(bad) => return refuse(&format!("the slot {bad:?} cannot be framed"), err),
            };
            body.push_str(&trailer(1));
            emit(&body, out)
        }
        Err(why) => refuse(&why, err),
    }
}

/// What target (if any) a seat add records. The locked core owns the
/// single lock→parse→check→allocate→publish sequence for all three:
/// `None` seats without a row, `LegacySpelling` grammar-checks a `&str`
/// exactly as before, and `Explicit` validates a typed path against the
/// same parsed bytes it publishes.
#[derive(Clone, Copy)]
pub(crate) enum TargetSpec<'a> {
    /// No row recorded: today's seat shape, byte for byte.
    None,
    /// A caller spelling, grammar-checked only (the `_roster` path).
    LegacySpelling(&'a str),
    /// A typed spawn target: representability, then full record
    /// validation (mode, legacy, strict proof, containment) under the
    /// one lock, published atomically with the seat block.
    Explicit {
        /// The spelled target; refusal precedes any write.
        target: &'a Path,
        /// The state root containment proves against; `None` refuses.
        state_root: Option<&'a Path>,
        /// The invoker cwd a relative spelling joins.
        invoker_cwd: &'a Path,
    },
}

/// Take the lowest free `spawned.<n>` for `name` and publish the seat, under
/// one hold of the meta lock — the decision half of `add-seat`, as a value.
///
/// A seat and its explicit target publish ATOMICALLY: one locked rewrite
/// carries the seat rows and the `work_dir.<slot>` row together, so no
/// seat-without-target state is ever observable. An explicit target is
/// validated against the same parsed bytes the seat publishes, so no
/// ae-side drift can invalidate validation between check and write.
/// `None` records no row.
///
/// # Errors
///
/// The refusal, phrased as [`refuse`] prints it: a bad or taken name, a v1 or
/// doubtful roster, an unwritable meta, a control byte in a value, an
/// unusable seat dir, or a refused explicit target — every refusal lands
/// before any write.
pub(crate) fn add_seat_slot_core(
    dir: &Path,
    name: &str,
    profile: &str,
    binary: &str,
    sid: Option<&str>,
    target: TargetSpec<'_>,
) -> Result<String, String> {
    let _held = meta::lock(dir).map_err(|why| format!("cannot take the meta lock: {why}"))?;
    let text = text_of(dir)?;
    let current = Meta::parse(&text);
    if let Some(why) = seat_write_refusal(dir, &current) {
        return Err(why);
    }
    if !config::is_agent_name(name) {
        return Err(format!(
            "invalid agent name '{name}'. Names must match {}.",
            config::AGENT_NAME_GRAMMAR
        ));
    }
    if current.roster().iter().any(|entry| entry.name == name) {
        return Err(format!(
            "'{name}' already holds a seat — under v2 the name IS the identity."
        ));
    }
    for field in [profile, binary].into_iter().chain(sid) {
        if !control_free(field) || field.contains('\n') {
            return Err(format!(
                "the value {field:?} carries a control byte no meta line can round-trip."
            ));
        }
    }
    let slot = format!("spawned.{}", lowest_free_spawned(&text));
    let work_dir = match target {
        TargetSpec::None => None,
        TargetSpec::LegacySpelling(dir) => Some(meta::checked_seat_work_dir(&slot, dir)?),
        TargetSpec::Explicit {
            target,
            state_root,
            invoker_cwd,
        } => {
            // Authoritative representability: `plan_` owns these wordings.
            meta::plan_spawn_target(Some(target), state_root)?;
            let (Some(spelling), Some(root)) = (target.to_str(), state_root) else {
                // Unreachable: `plan_` just proved both.
                return Err("internal error: a planned target failed its own proof.".to_owned());
            };
            Some(meta::record_seat_target(
                &current,
                &slot,
                spelling,
                root,
                invoker_cwd,
            )?)
        }
    };
    let mut next = text;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    let block = roster::render(&[SeatLines {
        slot: slot.clone(),
        name: name.to_owned(),
        profile: profile.to_owned(),
        // `spawn --using` takes a bare profile; a client override is
        // launch-only, so a spawned seat never records one.
        client: None,
        binary: Some(binary.to_owned()),
        harness_session: sid.map(ToOwned::to_owned),
        config_home: None,
        config_home_base: None,
        work_dir,
    }]);
    // `render` opens the block it builds with `schema=2`.
    next.push_str(block.strip_prefix("schema=2\n").unwrap_or(&block));
    publish(dir, &next)?;
    Ok(slot)
}

/// Add a seat with an optional caller spelling: the pre-typed entry point,
/// kept byte-identical for spawn and `_roster` by delegating to the one
/// locked core.
///
/// # Errors
///
/// Whatever [`add_seat_slot_core`] refuses with for the mapped spec.
pub fn add_seat_slot(
    dir: &Path,
    name: &str,
    profile: &str,
    binary: &str,
    sid: Option<&str>,
    work_dir: Option<&str>,
) -> Result<String, String> {
    let target = match work_dir {
        None => TargetSpec::None,
        Some(spelling) => TargetSpec::LegacySpelling(spelling),
    };
    add_seat_slot_core(dir, name, profile, binary, sid, target)
}

/// Read `add-seat`'s flags: `--using <profile>`, `--binary <bin>`,
/// `--session <sid>`.
///
/// # Errors
///
/// The offending word: an unknown flag, or a flag with no value.
fn add_seat_flags(flags: &[String]) -> Result<AddSeatFlags, String> {
    let mut parsed = AddSeatFlags::default();
    let mut rest = flags;
    while let [flag, after @ ..] = rest {
        let Some((value, tail)) = after.split_first() else {
            return Err(flag.clone());
        };
        match flag.as_str() {
            "--using" => parsed.profile = Some(value.clone()),
            "--binary" => parsed.binary = Some(value.clone()),
            "--session" => parsed.sid = optional(value),
            _ => return Err(flag.clone()),
        }
        rest = tail;
    }
    Ok(parsed)
}

/// `add-seat`'s flags.
#[derive(Debug, Default)]
struct AddSeatFlags {
    profile: Option<String>,
    binary: Option<String>,
    sid: Option<String>,
}

/// The refusal a v1 roster earns.
///
/// ae does not migrate `agent.<slot>` into a v2 seat, so the only way forward
/// from one is a fresh session. Names the session, because the operator
/// meets this through a helper that does not repeat which one it acted on.
fn fresh_start_refusal(dir: &Path) -> String {
    let session = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("this session");
    format!(
        "session {session:?} carries the retired v1 roster (agent.<slot>). \
         ae no longer migrates it: end the session and start a fresh one \
         (`ae end {session}`, then `ae {session}`)."
    )
}

/// Why a meta may not be WRITTEN to as a v2 roster, or `None` when it may.
fn seat_write_refusal(dir: &Path, current: &Meta) -> Option<String> {
    if current.schema() != Some("2") {
        return Some(match current.schema() {
            Some(other) => format!(
                "this session's meta declares schema={other}, not 2 — ae cannot write to it."
            ),
            None => fresh_start_refusal(dir),
        });
    }
    let doubts = identity_doubts(current);
    (!doubts.is_empty()).then(|| {
        format!(
            "this session's roster is in doubt and may not be written to: {}",
            doubts.join("; ")
        )
    })
}

/// The lowest `n` no key in `text` ends `.spawned.<n>` with.
fn lowest_free_spawned(text: &str) -> usize {
    let keys: Vec<&str> = records(text).into_iter().map(key_of).collect();
    let mut index = 0usize;
    loop {
        let suffix = format!(".spawned.{index}");
        if !keys.iter().any(|key| key.ends_with(suffix.as_str())) {
            return index;
        }
        index += 1;
    }
}

/// `_roster <dir> remove-seat <name>` — drop every line the seat owns.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
fn remove_seat(
    dir: &Path,
    name: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    match remove_seat_slot(dir, name) {
        Ok(slot) => {
            let mut body = match record(&["slot", &slot]) {
                Ok(line) => line,
                Err(bad) => return refuse(&format!("the slot {bad:?} cannot be framed"), err),
            };
            body.push_str(&trailer(1));
            emit(&body, out)
        }
        Err(why) => refuse(&why, err),
    }
}

/// Prove the seat named `name` may be removed and return its slot — pure,
/// so a kill-first retire asks it before any mutation. Only a spawned seat
/// is removable; a launch seat belongs to `ae end`.
///
/// # Errors
///
/// The refusal: an unknown name or a launch seat.
pub fn removable_slot(current: &Meta, name: &str) -> Result<String, String> {
    let Some(slot) = current
        .roster()
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.slot.clone())
    else {
        return Err(format!("no seat is named '{name}' in this session."));
    };
    if slot == "main" || slot.starts_with("worker.") {
        return Err(format!(
            "cannot retire '{name}' ({slot}) — it is a launch seat the workspace promised, not a spawned one; use 'ae end' to end the session."
        ));
    }
    Ok(slot)
}

/// Prove the seat named `name` may be removed — the lockless read half of
/// [`remove_seat_slot`], for a kill-first retire to ask before any mutation.
/// Reads what [`remove_seat_slot`] reads and words failures the same way.
///
/// # Errors
///
/// The refusal: an unknown name, a launch seat, an unreadable meta.
pub fn prove_removable(dir: &Path, name: &str) -> Result<String, String> {
    let text = text_of(dir)?;
    removable_slot(&Meta::parse(&text), name)
}

/// Drop every line the seat named `name` owns and return its slot — the
/// decision half of `remove-seat`, as a value.
///
/// # Errors
///
/// The refusal: an unknown name, a launch seat, an unwritable meta.
pub fn remove_seat_slot(dir: &Path, name: &str) -> Result<String, String> {
    let _held = meta::lock(dir).map_err(|why| format!("cannot take the meta lock: {why}"))?;
    let text = text_of(dir)?;
    let current = Meta::parse(&text);
    let slot = removable_slot(&current, name)?;
    let suffix = format!(".{slot}");
    let mut next = String::new();
    for row in records(&text) {
        if key_of(row).ends_with(suffix.as_str()) {
            continue;
        }
        next.push_str(row);
        next.push('\n');
    }
    publish(dir, &next)?;
    Ok(slot)
}

/// `_roster <dir> set-harness-session <slot> <sid>` — record one seat's
/// captured conversation id.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
fn set_harness_session(
    dir: &Path,
    slot: &str,
    sid: &str,
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let current = match parse_meta(dir) {
        Ok(current) => current,
        Err(why) => return refuse(&why, err),
    };
    if !current.roster().iter().any(|entry| entry.slot == slot) {
        return refuse(
            &format!("'{slot}' is not a seat in this session's roster."),
            err,
        );
    }
    if !control_free(sid) {
        return refuse(
            &format!("the session id {sid:?} carries a control byte no meta line can round-trip."),
            err,
        );
    }
    if let Err(why) = meta::rewrite(dir, &format!("harness_session.{slot}"), Some(sid)) {
        return refuse(
            &format!("the harness session id was not recorded: {}", why.cause()),
            err,
        );
    }
    emit(&trailer(0), out)
}

/// What resolving one listed seat's command answers: the command (`None`
/// is an unconfigured profile, which lists as `unresolved`), or a refusal
/// already explained on `err`.
enum SeatResolution {
    Resolved(Option<config::ResolvedCommand>),
    Refused(u8),
}

/// Resolve one listed seat's command: the recorded label through the
/// override substitution when one is recorded, else the legacy default.
///
/// # Errors
///
/// [`crate::Error::Io`] when `err` cannot be written.
fn list_seat_command(
    cfg: &IdentityConfig,
    entry: &meta::RosterEntry,
    home: Option<&Path>,
    err: &mut impl Write,
) -> Result<SeatResolution, crate::Error> {
    let Some(profile) = entry.profile.as_deref() else {
        return Ok(SeatResolution::Resolved(None));
    };
    match &entry.client {
        meta::RecordedClient::Label(label) => {
            match cfg.command_with_client(profile, label, home) {
                Ok(resolved) => Ok(SeatResolution::Resolved(Some(resolved.command))),
                // An unconfigured profile reads `unresolved`, exactly as the
                // legacy arm below.
                Err(config::OverrideError::UnknownProfile) => Ok(SeatResolution::Resolved(None)),
                Err(config::OverrideError::UnknownClient) => {
                    writeln!(
                        err,
                        "seat '{}' ({}) recorded client override '{label}' but no [clients] row names it now — \
                         restore the '{label}' client, or end the session",
                        entry.name, entry.slot
                    )?;
                    Ok(SeatResolution::Refused(EXIT_REFUSED))
                }
                Err(config::OverrideError::ProfileNotSimple(why)) => {
                    writeln!(err, "profile '{profile}' is not one simple command — {why}")?;
                    Ok(SeatResolution::Refused(EXIT_REFUSED))
                }
                Err(config::OverrideError::Refused(why)) => {
                    writeln!(err, "{why}")?;
                    Ok(SeatResolution::Refused(EXIT_REFUSED))
                }
            }
        }
        meta::RecordedClient::Missing => match cfg.command(profile, home) {
            Ok(command) => Ok(SeatResolution::Resolved(command)),
            Err(why) => {
                writeln!(err, "{why}")?;
                Ok(SeatResolution::Refused(EXIT_REFUSED))
            }
        },
        // Unreachable past the doubt gate above — and still refused, because
        // emitting a record for a hostile seat is what that gate exists to
        // prevent.
        meta::RecordedClient::Invalid => {
            writeln!(
                err,
                "seat '{}' ({}) records an unusable client (client.{} is empty, duplicated or malformed) — \
                 repair the meta by hand, or start over from its archive",
                entry.name, entry.slot, entry.slot
            )?;
            Ok(SeatResolution::Refused(EXIT_REFUSED))
        }
    }
}

/// `_roster <dir> list [--global <f>] [--local <f>]` — what the roster is now,
/// resolved against the config.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
fn list(
    dir: &Path,
    flags: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let files = match config_files(flags) {
        Ok(files) => files,
        Err(word) => return usage(crate::cli::ROSTER, &word, err),
    };
    let current = match parse_meta(dir) {
        Ok(current) => current,
        Err(why) => return refuse(&why, err),
    };
    if current.schema() != Some("2") {
        return refuse(
            &match current.schema() {
                Some(other) => format!(
                    "this session's meta declares schema={other}, not 2 — ae cannot read its roster."
                ),
                None => fresh_start_refusal(dir),
            },
            err,
        );
    }
    // FAIL CLOSED on a roster in doubt, BEFORE emitting a record: `Meta::parse`
    // drops the seats an anomaly touches, so the list would be shorter than the
    // file — and the resume that consumes this list republishes it through
    // `_meta-init --replace`, deleting the dropped seats and their metadata for
    // good (colead, integrated gate). Refusing here leaves the meta exactly as
    // it is, for a human to repair.
    let doubts = identity_doubts(&current);
    if !doubts.is_empty() {
        return refuse(
            &format!(
                "this session's roster is in doubt and may not be listed (nothing was emitted; repair the meta by hand, or start over from its archive): {}",
                doubts.join("; ")
            ),
            err,
        );
    }
    let cfg = match files.read() {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_REFUSED);
        }
    };
    let home = crate::doors::home();
    let mut body = String::new();
    for entry in current.roster() {
        // The seat's own profile row, resolved as an OPTION — never through the
        // rendered `-`, or a config that happened to define a profile literally
        // named `-` would resolve a seat that has no profile at all. A recorded
        // client label resolves through the override substitution, so the
        // emitted command is the one the seat runs — the default client's
        // would be a lie about an override seat.
        let command = match list_seat_command(&cfg, entry, home.as_deref(), err)? {
            SeatResolution::Resolved(command) => command,
            SeatResolution::Refused(exit) => return Ok(exit),
        };
        let resolved = command.and_then(|command| {
            launch_cmd::lex_simple_command(command.as_str())
                .ok()
                .map(|lexed| (lexed, command))
        });
        let profile = entry.profile.clone().unwrap_or_else(|| NONE.to_owned());
        let sid = entry
            .harness_session
            .clone()
            .unwrap_or_else(|| NONE.to_owned());
        let fields = match &resolved {
            Some((lexed, command)) => vec![
                "seat",
                &entry.slot,
                &entry.name,
                &profile,
                &lexed.binary,
                lexed.tool().as_str(),
                &sid,
                command.as_str(),
            ],
            None => vec![
                "unresolved",
                &entry.slot,
                &entry.name,
                &profile,
                entry.binary.as_deref().unwrap_or(NONE),
                NONE,
                &sid,
                NONE,
            ],
        };
        match record(&fields) {
            Ok(line) => body.push_str(&line),
            Err(bad) => {
                return refuse(
                    &format!(
                        "seat '{}' ({}) carries a value that would corrupt the record: {bad:?}",
                        entry.name, entry.slot
                    ),
                    err,
                );
            }
        }
    }
    body.push_str(&trailer(current.roster().len()));
    emit(&body, out)
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the capability boundary is about \
              what PRODUCT code may reach"
)]
mod tests {
    use super::{EXIT_REFUSED, EXIT_USAGE, US};
    use std::path::{Path, PathBuf};

    /// A scratch directory, unique per INSTANCE (pid + counter) because plain
    /// `cargo test` runs these in threads and a shared path would let one test
    /// publish over another's meta.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "ae-identity-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }

        fn dir(&self) -> &Path {
            &self.0
        }

        /// Write `text` to `name` inside the scratch and return its path.
        fn file(&self, name: &str, text: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, text).expect("fixture");
            path
        }

        fn meta(&self) -> String {
            std::fs::read_to_string(self.0.join("meta")).expect("meta")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The config both fixtures resolve against.
    const CONFIG: &str = "[profiles]\n\
                          fable5 = claude --model opus\n\
                          gpt56 = codex --yolo\n\
                          \n\
                          [roster]\n\
                          lead = fable5\n\
                          colead = gpt56\n\
                          \n\
                          [workspace]\n\
                          main = lead\n\
                          workers = colead\n";

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// `(code, stdout, stderr)` — every entry writes to injected streams, so a
    /// test reads exactly what a caller would.
    type Outcome = (u8, String, String);

    fn decode(code: u8, out: Vec<u8>, err: Vec<u8>) -> Outcome {
        (
            code,
            String::from_utf8(out).expect("stdout is utf-8"),
            String::from_utf8(err).expect("stderr is utf-8"),
        )
    }

    fn plan(tail: &[&str]) -> Outcome {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = super::launch_plan(&argv(tail), &mut out, &mut err).expect("streams");
        decode(code, out, err)
    }

    fn init(dir: &Path, tail: &[&str], stdin: &str) -> Outcome {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = super::meta_init(dir, &argv(tail), stdin, &mut out, &mut err).expect("streams");
        decode(code, out, err)
    }

    fn roster(dir: &Path, tail: &[&str]) -> Outcome {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = super::roster(dir, &argv(tail), &mut out, &mut err).expect("streams");
        decode(code, out, err)
    }

    /// Every record of `stdout` as its field vector, the trailer included.
    fn rows(stdout: &str) -> Vec<Vec<&str>> {
        stdout
            .lines()
            .map(|line| line.split(US).collect())
            .collect()
    }

    /// Publish a v2 meta with `lead` on main and `colead` on worker.0.
    fn seeded(tag: &str) -> Scratch {
        let scratch = Scratch::new(tag);
        let base = scratch.file("base", "mode=local\nwork_dir=/tmp/x\n");
        let stdin = format!(
            "seat{US}main{US}lead{US}fable5{US}claude{US}-\n\
             seat{US}worker.0{US}colead{US}gpt56{US}codex{US}e795\n\
             end{US}2\n"
        );
        let (code, _, err) = init(scratch.dir(), &["--base", &base.to_string_lossy()], &stdin);
        assert_eq!(code, 0, "seed failed: {err}");
        scratch
    }

    #[test]
    fn a_launch_plan_is_eight_fields_a_seat_with_the_sid_absent_and_a_counted_trailer() {
        let scratch = Scratch::new("plan");
        let cfg = scratch.file("config", CONFIG);
        let (code, out, err) = plan(&["--global", &cfg.to_string_lossy()]);
        assert_eq!((code, err.as_str()), (0, ""));
        let records = rows(&out);
        assert_eq!(
            records,
            vec![
                vec![
                    "seat",
                    "main",
                    "lead",
                    "fable5",
                    "claude",
                    "claude",
                    "-",
                    "claude --model opus"
                ],
                vec![
                    "seat",
                    "worker.0",
                    "colead",
                    "gpt56",
                    "codex",
                    "codex",
                    "-",
                    "codex --yolo"
                ],
                vec!["end", "2"],
            ]
        );
        // The uniform arity is the contract a fixed-field reader depends on: a
        // record that grew or shrank would silently shift the command into the
        // wrong field.
        for record in &records[..2] {
            assert_eq!(record.len(), 8, "{record:?}");
        }
    }

    #[test]
    fn the_main_and_workers_overrides_replace_the_config_and_a_dash_means_no_workers() {
        let scratch = Scratch::new("override");
        let cfg = scratch.file("config", CONFIG);
        let path = cfg.to_string_lossy().into_owned();
        // Swapping the two seats is a legal roster.
        let (code, out, err) = plan(&["--global", &path, "--main", "colead", "--workers", "lead"]);
        assert_eq!((code, err.as_str()), (0, ""));
        let records = rows(&out);
        assert_eq!(records[0][1..3], ["main", "colead"]);
        assert_eq!(records[1][1..3], ["worker.0", "lead"]);
        // `-` is no workers: `colead` is then a [roster] row bound to no seat,
        // which is LEGAL (ruled 2026-09-02 — it is what `use <name>` selects),
        // and the plan is main alone.
        let (code, out, err) = plan(&["--global", &path, "--workers", "-"]);
        assert_eq!((code, err.as_str()), (0, ""));
        let records = rows(&out);
        assert_eq!(records.len(), 2, "main + trailer: {out:?}");
        assert_eq!(records[0][1..3], ["main", "lead"]);
        assert_eq!(records[1], ["end", "1"]);
        // The empty string means the same thing as `-`.
        let (empty, empty_out, _) = plan(&["--global", &path, "--workers", ""]);
        assert_eq!((empty, empty_out), (0, out));
    }

    #[test]
    fn a_launch_plan_refuses_the_whole_config_and_names_every_violation_at_once() {
        let scratch = Scratch::new("violations");
        let cfg = scratch.file("config", CONFIG);
        let (code, out, err) = plan(&["--global", &cfg.to_string_lossy(), "--workers", "ghost"]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty());
        assert!(
            err.contains("'ghost' is not bound to a profile in [roster]"),
            "{err}"
        );
        assert!(
            !err.contains("colead"),
            "an unseated [roster] row is legal, not a second violation: {err}"
        );
        // A config error is the module's own Display, not a violation list.
        let broken = scratch.file("broken", "[agents]\ncl = claude\n");
        let (code, _, err) = plan(&["--global", &broken.to_string_lossy()]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("[agents] is not a v2 section"), "{err}");
        // An unknown flag is a USAGE error, told apart from a refusal.
        let (code, _, err) = plan(&["--nope", "x"]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("--nope"), "{err}");
    }

    #[test]
    fn a_command_carrying_the_separator_is_refused_rather_than_forging_a_field() {
        let scratch = Scratch::new("framing");
        let cfg = scratch.file(
            "config",
            &format!(
                "[profiles]\nbad = claude --tag a{US}b\n[roster]\nlead = bad\n[workspace]\nmain = lead\n"
            ),
        );
        let (code, out, err) = plan(&["--global", &cfg.to_string_lossy()]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty(), "nothing was published: {out:?}");
        assert!(err.contains("control byte"), "{err}");
    }

    #[test]
    fn meta_init_publishes_one_document_creates_only_and_consumes_its_base() {
        let scratch = seeded("init");
        assert_eq!(
            scratch.meta(),
            "mode=local\n\
             work_dir=/tmp/x\n\
             schema=2\n\
             seat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\n\
             seat.worker.0=colead\nprofile.worker.0=gpt56\nagent_bin.worker.0=codex\n\
             harness_session.worker.0=e795\n"
        );
        assert!(
            !scratch.dir().join("base").exists(),
            "the consumed base facts were not unlinked"
        );
        // A second create refuses: init is a create, never a clobber.
        let base = scratch.file("base2", "mode=full\n");
        let stdin = format!("seat{US}main{US}solo{US}fable5{US}claude{US}-\nend{US}1\n");
        let (code, out, err) = init(scratch.dir(), &["--base", &base.to_string_lossy()], &stdin);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty());
        assert!(err.contains("nothing changed"), "{err}");
        assert!(
            scratch.meta().contains("seat.main=lead"),
            "the refused init left the first meta whole"
        );
        assert!(base.exists(), "a refusal leaves the base file in place");
        // --replace publishes over it.
        let (code, out, err) = init(
            scratch.dir(),
            &["--base", &base.to_string_lossy(), "--replace"],
            &stdin,
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("end{US}0\n"));
        assert_eq!(
            scratch.meta(),
            "mode=full\nschema=2\nseat.main=solo\nprofile.main=fable5\nagent_bin.main=claude\n"
        );
        assert!(!base.exists());
    }

    #[test]
    fn a_replace_publish_takes_a_pre_grammar_name_verbatim_and_a_create_refuses_it() {
        // The resume half: the name came out of the session's own meta,
        // not from a human, and refusing it would strand the session. A fresh
        // launch (no --replace) still holds the grammar.
        let scratch = seeded("restored");
        let stdin = format!("seat{US}main{US}old boss (v1){US}cl{US}claude{US}-\nend{US}1\n");
        let base = scratch.file("base-fresh", "mode=local\n");
        let (code, _, err) = init(scratch.dir(), &["--base", &base.to_string_lossy()], &stdin);
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("invalid agent name"), "{err}");
        let base = scratch.file("base-resume", "mode=local\n");
        let (code, out, err) = init(
            scratch.dir(),
            &["--base", &base.to_string_lossy(), "--replace"],
            &stdin,
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("end{US}0\n"));
        assert!(
            scratch.meta().contains("seat.main=old boss (v1)\n"),
            "{}",
            scratch.meta()
        );
        // Empty is still refused on a resume: a seat nobody is named for.
        let stdin = format!("seat{US}main{US}{US}cl{US}claude{US}-\nend{US}1\n");
        let base = scratch.file("base-empty", "mode=local\n");
        let (code, _, err) = init(
            scratch.dir(),
            &["--base", &base.to_string_lossy(), "--replace"],
            &stdin,
        );
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("no name at all"), "{err}");
    }

    #[test]
    fn meta_init_refuses_every_way_stdin_can_be_wrong_and_publishes_nothing() {
        let bad = [
            (
                format!("seat{US}main{US}lead{US}fable5{US}claude\nend{US}1\n"),
                "6-field",
            ),
            (
                format!("seat{US}main{US}lead{US}fable5{US}claude{US}-\n"),
                "end trailer",
            ),
            (
                format!("seat{US}main{US}lead{US}fable5{US}claude{US}-\nend{US}7\n"),
                "declares 7",
            ),
            (
                format!(
                    "seat{US}main{US}lead{US}fable5{US}claude{US}-\n\
                     seat{US}worker.0{US}lead{US}gpt56{US}codex{US}-\nend{US}2\n"
                ),
                "more than one seat",
            ),
            (
                format!(
                    "seat{US}main{US}lead{US}fable5{US}claude{US}-\n\
                     seat{US}main{US}other{US}gpt56{US}codex{US}-\nend{US}2\n"
                ),
                "more than one seat",
            ),
            (
                format!("seat{US}main{US}bad:name{US}fable5{US}claude{US}-\nend{US}1\n"),
                "invalid agent name",
            ),
            (
                format!("seat{US}main{US}lead{US}{US}claude{US}-\nend{US}1\n"),
                "names no profile",
            ),
            (String::new(), "no records at all"),
        ];
        for (stdin, expected) in bad {
            let scratch = Scratch::new("stdin");
            let base = scratch.file("base", "mode=local\n");
            let (code, out, err) =
                init(scratch.dir(), &["--base", &base.to_string_lossy()], &stdin);
            assert_eq!(code, EXIT_REFUSED, "{stdin:?} -> {err}");
            assert!(out.is_empty(), "{stdin:?} published {out:?}");
            assert!(err.contains(expected), "{stdin:?} said {err}");
            assert!(
                !scratch.dir().join("meta").exists(),
                "{stdin:?} published a meta"
            );
            assert!(base.exists(), "{stdin:?} consumed the base anyway");
        }
    }

    #[test]
    fn meta_init_refuses_base_facts_that_do_not_end_in_a_newline() {
        let scratch = Scratch::new("unterminated");
        let base = scratch.file("base", "mode=local");
        let stdin = format!("seat{US}main{US}lead{US}fable5{US}claude{US}-\nend{US}1\n");
        let (code, _, err) = init(scratch.dir(), &["--base", &base.to_string_lossy()], &stdin);
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("fuse two records"), "{err}");
        assert!(!scratch.dir().join("meta").exists());
        // No --base at all is a usage error, not a refusal.
        let (code, _, err) = init(scratch.dir(), &[], &stdin);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("--base"), "{err}");
    }

    #[test]
    fn add_seat_takes_the_lowest_free_index_across_every_key_family_and_leaves_gaps() {
        let scratch = seeded("add");
        let (code, out, err) = roster(
            scratch.dir(),
            &[
                "add-seat", "helper", "--using", "fable5", "--binary", "claude",
            ],
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("slot{US}spawned.0\nend{US}1\n"));
        // A stale row nobody cleaned up still CLAIMS its index: allocating
        // over it would let a fresh seat inherit a stale launch id.
        std::fs::write(
            scratch.dir().join("meta"),
            format!("{}launch_id.spawned.1=stale\n", scratch.meta()),
        )
        .expect("append");
        let (_, out, _) = roster(
            scratch.dir(),
            &[
                "add-seat",
                "helper2",
                "--using",
                "gpt56",
                "--binary",
                "codex",
                "--session",
                "sid9",
            ],
        );
        assert_eq!(out, format!("slot{US}spawned.2\nend{US}1\n"));
        let meta = scratch.meta();
        assert!(meta.contains("seat.spawned.2=helper2\n"), "{meta}");
        assert!(meta.contains("harness_session.spawned.2=sid9\n"), "{meta}");
        // The schema marker is NOT re-emitted: a second one is a duplicate key,
        // and `Meta::parse` invalidates a duplicated key — the meta would stop
        // reading as v2 the moment a seat was added.
        assert_eq!(meta.matches("schema=2").count(), 1, "{meta}");
        // A retire leaves a gap, and the next add fills it rather than
        // renumbering anything.
        roster(scratch.dir(), &["remove-seat", "helper"]);
        let (_, out, _) = roster(
            scratch.dir(),
            &[
                "add-seat", "helper3", "--using", "gpt56", "--binary", "codex",
            ],
        );
        assert_eq!(out, format!("slot{US}spawned.0\nend{US}1\n"));
    }

    #[test]
    fn add_seat_refuses_a_bad_name_a_taken_name_a_v1_meta_and_a_roster_in_doubt() {
        let scratch = seeded("add-refuse");
        let cases = [
            (
                vec![
                    "add-seat", "bad:name", "--using", "fable5", "--binary", "claude",
                ],
                "invalid agent name",
            ),
            (
                vec![
                    "add-seat", "lead", "--using", "fable5", "--binary", "claude",
                ],
                "already holds a seat",
            ),
        ];
        for (tail, expected) in cases {
            let before = scratch.meta();
            let (code, out, err) = roster(scratch.dir(), &tail);
            assert_eq!(code, EXIT_REFUSED, "{tail:?}");
            assert!(out.is_empty(), "{tail:?} published {out:?}");
            assert!(err.contains(expected), "{tail:?} said {err}");
            assert_eq!(scratch.meta(), before, "{tail:?} changed the meta");
        }
        // Missing required flags are USAGE, not a refusal.
        let (code, _, err) = roster(scratch.dir(), &["add-seat", "helper", "--using", "fable5"]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("--binary"), "{err}");
        // A v1 meta may not take a v2 seat: the result would be a mixed roster.
        let v1 = Scratch::new("add-v1");
        v1.file("meta", "mode=local\nagent.main=fable5:lead\n");
        let (code, _, err) = roster(
            v1.dir(),
            &[
                "add-seat", "helper", "--using", "fable5", "--binary", "claude",
            ],
        );
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("retired v1 roster"), "{err}");
        // A roster in doubt may not be written to: the uniqueness check above
        // would have been answered by an incomplete list.
        let doubtful = Scratch::new("add-doubt");
        doubtful.file(
            "meta",
            "schema=2\nseat.main=lead\nseat.worker.0=lead\nprofile.main=fable5\n",
        );
        let (code, _, err) = roster(
            doubtful.dir(),
            &[
                "add-seat", "helper", "--using", "fable5", "--binary", "claude",
            ],
        );
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("in doubt"), "{err}");
    }

    #[test]
    fn remove_seat_drops_every_line_the_slot_owns_including_the_bash_era_rows() {
        let scratch = seeded("remove");
        roster(
            scratch.dir(),
            &[
                "add-seat",
                "helper",
                "--using",
                "fable5",
                "--binary",
                "claude",
                "--session",
                "s1",
            ],
        );
        std::fs::write(
            scratch.dir().join("meta"),
            format!(
                "{}launch_id.spawned.0=uuid-1\nlaunch_time.spawned.0=12345\n\
                 capture_floor.spawned.0=12344\n\
                 claude_launch_id.spawned.0=uuid-2\nlaunch_id.spawned.10=keep-me\n",
                scratch.meta()
            ),
        )
        .expect("append");
        let (code, out, err) = roster(scratch.dir(), &["remove-seat", "helper"]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("slot{US}spawned.0\nend{US}1\n"));
        let meta = scratch.meta();
        assert!(
            !meta.contains("spawned.0"),
            "a row outlived its seat: {meta}"
        );
        assert!(
            meta.contains("launch_id.spawned.10=keep-me\n"),
            "the suffix matched a NEIGHBOURING index: {meta}"
        );
        assert!(
            meta.contains("seat.main=lead\n") && meta.contains("mode=local\n"),
            "{meta}"
        );
    }

    #[test]
    fn remove_seat_refuses_a_launch_seat_and_an_unknown_name() {
        let scratch = seeded("remove-refuse");
        for (name, expected) in [
            ("lead", "use 'ae end'"),
            ("colead", "use 'ae end'"),
            ("nobody", "no seat is named"),
        ] {
            let before = scratch.meta();
            let (code, out, err) = roster(scratch.dir(), &["remove-seat", name]);
            assert_eq!(code, EXIT_REFUSED, "{name}");
            assert!(out.is_empty(), "{name} published {out:?}");
            assert!(err.contains(expected), "{name} said {err}");
            assert_eq!(scratch.meta(), before, "{name} changed the meta");
        }
    }

    #[test]
    fn set_harness_session_records_one_key_and_refuses_a_slot_that_is_not_a_seat() {
        let scratch = seeded("sid");
        let (code, out, err) = roster(scratch.dir(), &["set-harness-session", "main", "abc123"]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("end{US}0\n"), "no records, just the trailer");
        assert!(
            scratch.meta().contains("harness_session.main=abc123\n"),
            "{}",
            scratch.meta()
        );
        // A second write REPLACES rather than appending a duplicate key.
        roster(scratch.dir(), &["set-harness-session", "main", "def456"]);
        let meta = scratch.meta();
        assert_eq!(meta.matches("harness_session.main=").count(), 1, "{meta}");
        assert!(meta.contains("harness_session.main=def456\n"), "{meta}");
        let (code, out, err) = roster(scratch.dir(), &["set-harness-session", "spawned.9", "x"]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty());
        assert!(err.contains("is not a seat"), "{err}");
    }

    #[test]
    fn list_resolves_what_the_config_still_defines_and_says_unresolved_for_the_rest() {
        let scratch = seeded("list");
        let cfg = scratch.file("config", CONFIG);
        let (code, out, err) = roster(scratch.dir(), &["list", "--global", &cfg.to_string_lossy()]);
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            rows(&out),
            vec![
                vec![
                    "seat",
                    "main",
                    "lead",
                    "fable5",
                    "claude",
                    "claude",
                    "-",
                    "claude --model opus"
                ],
                vec![
                    "seat",
                    "worker.0",
                    "colead",
                    "gpt56",
                    "codex",
                    "codex",
                    "e795",
                    "codex --yolo"
                ],
                vec!["end", "2"],
            ]
        );
        // With the profiles gone, nothing launchable can be said — but the
        // META's binary survives, which is what lets the launcher hand the seat
        // straight back to `_meta-init`.
        let empty = scratch.file("empty", "[workspace]\nmain = lead\n");
        let (code, out, err) = roster(
            scratch.dir(),
            &["list", "--global", &empty.to_string_lossy()],
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(
            rows(&out),
            vec![
                vec![
                    "unresolved",
                    "main",
                    "lead",
                    "fable5",
                    "claude",
                    "-",
                    "-",
                    "-"
                ],
                vec![
                    "unresolved",
                    "worker.0",
                    "colead",
                    "gpt56",
                    "codex",
                    "-",
                    "e795",
                    "-"
                ],
                vec!["end", "2"],
            ]
        );
        // A profile whose command is not one simple command is
        // unresolved too: it is defined, and it still may not reach a pane.
        let broken = scratch.file(
            "broken",
            "[profiles]\nfable5 = claude; rm -rf /\ngpt56 = codex\n[roster]\nlead = fable5\n",
        );
        let (_, out, _) = roster(
            scratch.dir(),
            &["list", "--global", &broken.to_string_lossy()],
        );
        assert_eq!(rows(&out)[0][0], "unresolved");
        assert_eq!(rows(&out)[1][0], "seat", "the lexable one still resolves");
    }

    /// A recorded client label resolves through the override substitution:
    /// the emitted command is the one the seat runs, not the default client's.
    #[test]
    fn list_resolves_a_recorded_client_label_through_the_override() {
        let scratch = seeded("list-client");
        let meta = scratch.meta().replacen(
            "profile.main=fable5\n",
            "profile.main=fable5\nclient.main=cc-mic\n",
            1,
        );
        scratch.file("meta", &meta);
        let cfg = scratch.file(
            "config",
            "[clients]\n\
             claude = claude\n\
             cc-mic = claude config_home=/tmp/x/.claude-mic\n\
             \n\
             [profiles]\n\
             fable5 = claude --model opus\n\
             gpt56 = codex --yolo\n\
             \n\
             [roster]\n\
             lead = fable5\n\
             colead = gpt56\n\
             \n\
             [workspace]\n\
             main = lead\n\
             workers = colead\n",
        );
        let (code, out, err) = roster(scratch.dir(), &["list", "--global", &cfg.to_string_lossy()]);
        assert_eq!((code, err.as_str()), (0, ""));
        let main = &rows(&out)[0];
        assert_eq!(main[0], "seat");
        assert!(
            main[7].contains(".claude-mic") && main[7].contains("claude --model opus"),
            "the override expansion, not the default: {main:?}"
        );
        assert_eq!(
            rows(&out)[1][7],
            "codex --yolo",
            "the legacy seat is untouched"
        );
    }

    /// `--replace` preserves the current per-slot client rows: the 6-field
    /// stdin record cannot carry them, and dropping them would launder every
    /// `Label` into `Missing`.
    #[test]
    fn replace_preserves_recorded_client_rows_and_refuses_an_invalid_one() {
        let scratch = seeded("replace-client");
        let meta = scratch.meta().replacen(
            "profile.main=fable5\n",
            "profile.main=fable5\nclient.main=cc-mic\n",
            1,
        );
        scratch.file("meta", &meta);
        let base = scratch.file("base", "mode=local\nwork_dir=/tmp/x\n");
        let stdin = format!(
            "seat{US}main{US}lead{US}fable5{US}claude{US}-\n\
             seat{US}worker.0{US}colead{US}gpt56{US}codex{US}e795\n\
             end{US}2\n"
        );
        let (code, _, err) = init(
            scratch.dir(),
            &["--base", &base.to_string_lossy(), "--replace"],
            &stdin,
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(
            scratch.meta().contains("client.main=cc-mic\n"),
            "the label survives the republish: {}",
            scratch.meta()
        );
        // An unusable row cannot be preserved and must not be laundered.
        let damaged = scratch
            .meta()
            .replace("client.main=cc-mic\n", "client.main=\n");
        scratch.file("meta", &damaged);
        let (code, _, err) = init(
            scratch.dir(),
            &["--base", &base.to_string_lossy(), "--replace"],
            &stdin,
        );
        assert_eq!(code, EXIT_REFUSED);
        assert!(
            err.contains("client.main") && err.contains("fix the meta row before replacing"),
            "{err}"
        );
        assert_eq!(scratch.meta(), damaged, "the refusal published nothing");
    }

    #[test]
    fn list_refuses_a_roster_in_doubt_and_leaves_the_meta_untouched() {
        // One name on two seats must REFUSE: a list showing the main seat alone
        // would be consumed by a resume and republished, deleting both worker
        // seats for good.
        let doubtful = [
            (
                "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\n\
                 seat.worker.0=helper\nprofile.worker.0=x\nagent_bin.worker.0=claude\n\
                 seat.worker.1=helper\nprofile.worker.1=x\nagent_bin.worker.1=claude\n",
                "helper",
            ),
            (
                "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\n\
                 seat.worker.0=colead\nprofile.worker.0=x\nagent_bin.worker.0=codex\n\
                 seat.worker.0=other\n",
                "seat.worker.0",
            ),
            (
                "schema=2\nseat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\n\
                 agent.worker.0=gpt56:colead:pending\nagent_bin.worker.0=codex\n",
                "worker.0",
            ),
        ];
        for (meta, named) in doubtful {
            let scratch = Scratch::new("list-doubt");
            let path = scratch.file("meta", meta);
            let (code, out, err) = roster(scratch.dir(), &["list"]);
            assert_eq!(code, EXIT_REFUSED, "case {named}: out={out:?} err={err:?}");
            assert!(out.is_empty(), "a refusal emits no record: {out:?}");
            assert!(err.contains("in doubt and may not be listed"), "{err}");
            assert!(err.contains(named), "the doubt is named: {err}");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                meta,
                "list never writes, and a refusal leaves the meta byte-identical"
            );
        }
    }

    #[test]
    fn list_refuses_a_meta_that_is_not_v2_and_an_absent_one() {
        let v1 = Scratch::new("list-v1");
        v1.file("meta", "mode=local\nagent.main=fable5:lead\n");
        let (code, out, err) = roster(v1.dir(), &["list"]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty());
        assert!(err.contains("retired v1 roster"), "{err}");
        assert!(err.contains("start a fresh one"), "{err}");
        assert!(
            err.contains("list-v1"),
            "the refusal names the session: {err}"
        );
        let gone = Scratch::new("list-gone");
        let (code, _, err) = roster(gone.dir(), &["list"]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(err.contains("cannot read the meta"), "{err}");
    }

    #[test]
    fn the_roster_subcommands_are_a_closed_set_and_a_wrong_arity_is_a_usage_error() {
        let scratch = seeded("dispatch");
        for tail in [
            vec!["nonsense"],
            vec!["remove-seat"],
            vec!["remove-seat", "a", "b"],
            vec!["set-harness-session", "main"],
        ] {
            let (code, out, _) = roster(scratch.dir(), &tail);
            assert_eq!(code, EXIT_USAGE, "{tail:?}");
            assert!(out.is_empty(), "{tail:?}");
        }
        let (code, _, err) = roster(scratch.dir(), &[]);
        assert_eq!(code, EXIT_USAGE);
        assert!(err.contains("needs a subcommand"), "{err}");
    }

    #[test]
    fn every_entry_is_reachable_through_argv_and_none_can_shadow_a_session_name() {
        use crate::cli::{LAUNCH_PLAN, META_INIT, ROSTER, Request};
        for spelling in [LAUNCH_PLAN, META_INIT, ROSTER] {
            // `_validate_session_name` forbids a leading `_`, so no legal
            // session name can reach these arms — the property that keeps
            // "a bare word is a launch candidate" whole.
            assert!(spelling.starts_with('_'), "{spelling}");
        }
        assert_eq!(
            Request::parse(&argv(&[LAUNCH_PLAN, "--main", "lead"])),
            Request::LaunchPlan {
                tail: argv(&["--main", "lead"])
            }
        );
        assert_eq!(
            Request::parse(&argv(&[META_INIT, "/s/one", "--base", "/s/b"])),
            Request::MetaInit {
                dir: "/s/one".into(),
                tail: argv(&["--base", "/s/b"])
            }
        );
        assert_eq!(
            Request::parse(&argv(&[ROSTER, "/s/one", "list"])),
            Request::Roster {
                dir: "/s/one".into(),
                tail: argv(&["list"])
            }
        );
        // A missing directory is the MissingOperand class (exit 2), not a
        // refusal about a directory named "".
        for spelling in [META_INIT, ROSTER] {
            let request = Request::parse(&argv(&[spelling]));
            assert_eq!(request, Request::MissingOperand(spelling));
            assert_eq!(request.exit_code(), Some(2));
        }
        // `_launch-plan` needs no operand at all: a flagless call resolves the
        // ambient config and answers with its own violations.
        assert_eq!(
            Request::parse(&argv(&[LAUNCH_PLAN])),
            Request::LaunchPlan { tail: Vec::new() }
        );
    }

    #[test]
    fn add_seat_slot_publishes_seat_and_dir_in_one_rewrite() {
        let scratch = Scratch::new("seat-dir-atomic");
        scratch.file("meta", "schema=2\nseat.main=lead\nprofile.main=fable5\n");
        let slot = super::add_seat_slot(
            scratch.dir(),
            "scout",
            "fable5",
            "claude",
            None,
            Some("/w/target"),
        )
        .expect("a seat");
        assert_eq!(slot, "spawned.0");
        let meta = scratch.meta();
        assert!(meta.contains("seat.spawned.0=scout\n"), "{meta}");
        assert!(meta.contains("work_dir.spawned.0=/w/target\n"), "{meta}");
        // No seat-without-target state is observable: one publish, both rows.
        let parsed = crate::meta::Meta::parse(&meta);
        assert_eq!(parsed.roster().len(), 2);
        assert_eq!(
            parsed.roster()[1].work_dir,
            crate::meta::RecordedWorkDir::Path("/w/target".into())
        );
    }

    #[test]
    fn add_seat_slot_refuses_a_bad_dir_before_any_write() {
        let scratch = Scratch::new("seat-dir-refuse");
        scratch.file("meta", "schema=2\nseat.main=lead\nprofile.main=fable5\n");
        for bad in ["", "relative/path", "/x/\u{7}/y"] {
            let before = scratch.meta();
            let why =
                super::add_seat_slot(scratch.dir(), "scout", "fable5", "claude", None, Some(bad))
                    .expect_err("a refusal");
            assert!(why.contains("work_dir"), "{why}");
            assert_eq!(scratch.meta(), before, "the meta moved on a refusal");
        }
        // None records no row: today's shape, byte for byte.
        let slot = super::add_seat_slot(scratch.dir(), "scout", "fable5", "claude", None, None)
            .expect("a seat");
        assert_eq!(slot, "spawned.0");
        assert!(!scratch.meta().contains("work_dir."), "{}", scratch.meta());
    }

    #[test]
    fn concurrent_add_seat_slot_allocates_distinct_slots() {
        let scratch = Scratch::new("seat-dir-race");
        scratch.file("meta", "schema=2\nseat.main=lead\nprofile.main=fable5\n");
        let dir = scratch.dir();
        let slots: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|n| {
                    scope.spawn(move || {
                        super::add_seat_slot(
                            dir,
                            &format!("scout{n}"),
                            "fable5",
                            "claude",
                            None,
                            Some("/w/target"),
                        )
                        .expect("a seat")
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("a thread"))
                .collect()
        });
        let mut sorted = slots.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "slots collided: {slots:?}");
        let parsed = crate::meta::Meta::parse(&scratch.meta());
        assert_eq!(parsed.roster().len(), 5);
        assert!(
            parsed
                .roster()
                .iter()
                .skip(1)
                .all(|entry| matches!(entry.work_dir, crate::meta::RecordedWorkDir::Path(_))),
            "a seat lost its dir: {:?}",
            parsed.roster()
        );
    }

    /// Local-mode session meta for core-target tests. Targets live beside
    /// `state/`, never under it, so containment holds by construction.
    fn local_core_dirs(tag: &str) -> Scratch {
        let scratch = Scratch::new(tag);
        scratch.file(
            "meta",
            "schema=2\nmode=local\nseat.main=lead\nprofile.main=fable5\n",
        );
        std::fs::create_dir(scratch.dir().join("state")).expect("state root");
        scratch
    }

    /// One explicit-target core add against a fixture.
    fn add_explicit(
        scratch: &Scratch,
        name: &str,
        target: &Path,
        state: Option<&Path>,
    ) -> Result<String, String> {
        super::add_seat_slot_core(
            scratch.dir(),
            name,
            "fable5",
            "claude",
            None,
            super::TargetSpec::Explicit {
                target,
                state_root: state,
                invoker_cwd: scratch.dir(),
            },
        )
    }

    // B2-spawn U4: differing targets per seat, no cross-talk.
    #[test]
    fn core_records_distinct_targets_per_seat() {
        let scratch = local_core_dirs("core-distinct");
        let state = scratch.dir().join("state");
        let mut canons = Vec::new();
        for (name, tag) in [("scout", "one"), ("sage", "two")] {
            let target = scratch.dir().join(tag);
            std::fs::create_dir(&target).unwrap();
            let slot =
                add_explicit(&scratch, name, &target, Some(state.as_path())).expect("a seat");
            canons.push((slot, std::fs::canonicalize(&target).unwrap()));
        }
        let meta = scratch.meta();
        assert_ne!(canons[0].1, canons[1].1);
        for (slot, canon) in &canons {
            let row = format!("work_dir.{slot}={}\n", canon.display());
            assert!(meta.contains(&row), "{meta}");
            let parsed = crate::meta::Meta::parse(&meta);
            assert_eq!(
                crate::meta::checked_explicit_pane_start_dir(&parsed, slot),
                Ok(canon.display().to_string())
            );
        }
    }

    // B2-spawn U5: spaces record + resolve.
    #[test]
    fn core_records_a_target_with_spaces() {
        let scratch = local_core_dirs("core-spaces");
        let target = scratch.dir().join("my repo");
        std::fs::create_dir(&target).unwrap();
        let state = scratch.dir().join("state");
        let slot = add_explicit(&scratch, "scout", &target, Some(state.as_path())).expect("a seat");
        let canon = std::fs::canonicalize(&target).unwrap();
        let meta = scratch.meta();
        assert!(
            meta.contains(&format!("work_dir.{slot}={}\n", canon.display())),
            "{meta}"
        );
        let parsed = crate::meta::Meta::parse(&meta);
        assert_eq!(
            crate::meta::checked_explicit_pane_start_dir(&parsed, &slot),
            Ok(canon.display().to_string())
        );
    }

    // B2-spawn U6: a non-git dir records + resolves (no repo needed).
    #[test]
    fn core_records_a_plain_non_git_dir() {
        let scratch = local_core_dirs("core-nongit");
        let target = scratch.dir().join("not-a-repo");
        std::fs::create_dir(&target).unwrap();
        let state = scratch.dir().join("state");
        let slot = add_explicit(&scratch, "scout", &target, Some(state.as_path())).expect("a seat");
        let canon = std::fs::canonicalize(&target).unwrap();
        let meta = scratch.meta();
        assert!(
            meta.contains(&format!("work_dir.{slot}={}\n", canon.display())),
            "{meta}"
        );
        let parsed = crate::meta::Meta::parse(&meta);
        assert_eq!(
            crate::meta::checked_explicit_pane_start_dir(&parsed, &slot),
            Ok(canon.display().to_string())
        );
    }

    // B2-spawn U7: slot reuse is row-clean across add/remove/add.
    #[test]
    fn core_slot_reuse_leaves_no_stale_row() {
        let scratch = local_core_dirs("core-reuse");
        let target = scratch.dir().join("repo");
        std::fs::create_dir(&target).unwrap();
        let state = scratch.dir().join("state");
        let first =
            add_explicit(&scratch, "scout", &target, Some(state.as_path())).expect("a seat");
        assert!(
            scratch.meta().contains(&format!("work_dir.{first}=")),
            "{}",
            scratch.meta()
        );
        super::remove_seat_slot(scratch.dir(), "scout").expect("retired");
        assert!(!scratch.meta().contains("work_dir."), "{}", scratch.meta());
        let second = super::add_seat_slot(scratch.dir(), "sage", "fable5", "claude", None, None)
            .expect("a seat");
        assert_eq!(second, first, "the freed slot is reused");
        assert!(!scratch.meta().contains("work_dir."), "{}", scratch.meta());
    }

    // #197.5: the kill-first retire PROVES a slot before the kill. When the
    // name is retired and re-spawned under the same name in between, the
    // removal must not follow the name onto the successor's slot.
    #[test]
    fn a_removal_that_follows_a_moved_name_does_not_take_the_successor() {
        let scratch = local_core_dirs("moved-name");
        let target = scratch.dir().join("repo");
        std::fs::create_dir(&target).unwrap();
        let state = scratch.dir().join("state");
        let proven =
            add_explicit(&scratch, "scout", &target, Some(state.as_path())).expect("a seat");
        assert_eq!(
            super::prove_removable(scratch.dir(), "scout").expect("a proven seat"),
            proven
        );
        // Retired and re-spawned under the same name: every row moved to a new
        // slot while the proof still names the old one.
        let moved = scratch.meta().replace(&proven, "spawned.9");
        super::publish(scratch.dir(), &moved).expect("the moved meta");

        let removal = super::remove_seat_slot(scratch.dir(), "scout");
        assert!(
            removal.is_err(),
            "the removal followed the name onto the successor: {removal:?}"
        );
        let kept = scratch.meta();
        assert!(kept.contains("seat.spawned.9=scout"), "{kept}");
        assert!(kept.contains("work_dir.spawned.9="), "{kept}");
    }

    // B2-spawn U8: managed/legacy refuses atomically, record order kept.
    #[test]
    fn core_refuses_managed_without_a_partial_seat() {
        // Managed mode=git, the real spelling: mode refuses before empty,
        // so the record order survives the core.
        let managed = Scratch::new("core-managed");
        managed.file(
            "meta",
            "schema=2\nmode=git\nseat.main=lead\nprofile.main=fable5\n",
        );
        let target = managed.dir().join("repo");
        std::fs::create_dir(&target).unwrap();
        let state = managed.dir().join("state");
        std::fs::create_dir(&state).unwrap();
        for spelled in [target.as_path(), Path::new("")] {
            let before = managed.meta();
            let why = add_explicit(&managed, "scout", spelled, Some(state.as_path()))
                .expect_err("a refusal");
            assert_eq!(
                why,
                "explicit seat targets record on local sessions only — \
                 this session's mode is 'git'."
                    .to_owned()
            );
            assert_eq!(managed.meta(), before, "the meta moved on a refusal");
        }
        // A legacy agent.<slot> row IS roster-doubting, so the core's
        // seat_write_refusal fires before record_: the pin asserts that
        // routing, byte-identical.
        let legacy = Scratch::new("core-legacy");
        legacy.file(
            "meta",
            "schema=2\nmode=local\nseat.main=lead\nprofile.main=fable5\nagent.spawned.9=ghost\n",
        );
        let target = legacy.dir().join("repo");
        std::fs::create_dir(&target).unwrap();
        let state = legacy.dir().join("state");
        std::fs::create_dir(&state).unwrap();
        let before = legacy.meta();
        let why =
            add_explicit(&legacy, "scout", &target, Some(state.as_path())).expect_err("a refusal");
        assert_eq!(
            why,
            "this session's roster is in doubt and may not be written to: \
             slot spawned.9 carries the retired v1 roster agent.spawned.9 (line 5): \
             this session is not served by this ae"
                .to_owned()
        );
        assert_eq!(legacy.meta(), before, "the meta moved on a refusal");
    }

    // B2-spawn U9: the CORE rejects Some + no state root, byte-identical.
    #[test]
    fn core_refuses_some_without_a_state_root() {
        let scratch = local_core_dirs("core-noroot");
        let target = scratch.dir().join("repo");
        std::fs::create_dir(&target).unwrap();
        let before = scratch.meta();
        let why = add_explicit(&scratch, "scout", &target, None).expect_err("a refusal");
        assert_eq!(
            why,
            "no ae state root is available — an explicit target cannot prove containment."
                .to_owned()
        );
        assert_eq!(scratch.meta(), before, "the meta moved on a refusal");
    }

    // B2-spawn U10: empty + control spellings refuse with owned wordings.
    #[test]
    fn core_refuses_empty_and_control_spellings() {
        let scratch = local_core_dirs("core-spellings");
        let state = scratch.dir().join("state");
        let before = scratch.meta();
        let why = add_explicit(&scratch, "scout", Path::new(""), Some(state.as_path()))
            .expect_err("a refusal");
        assert_eq!(
            why,
            "work_dir.spawned.0 is present but unusable (empty value) — \
             restore the recorded path or retire the seat."
                .to_owned()
        );
        // A control byte that PROVES: the shared row wording, never absence.
        // A nonexistent path would let strict absence win and pass even if
        // control-row validation were deleted — so the dir is real.
        let weird = scratch.dir().join("we\u{7}ird");
        std::fs::create_dir(&weird).unwrap();
        let why =
            add_explicit(&scratch, "scout", &weird, Some(state.as_path())).expect_err("a refusal");
        assert_eq!(
            why,
            "work_dir.spawned.0 is present but unusable (control characters) — \
             restore the recorded path or retire the seat."
                .to_owned()
        );
        assert_eq!(scratch.meta(), before, "the meta moved on a refusal");
    }

    // #194 B1(a): only a spawned seat is removable; the proof is pure so the
    // kill-first retire asks it before any mutation.
    #[test]
    fn removable_slot_proves_only_spawned_seats() {
        let meta =
            crate::meta::Meta::parse("seat.main=lead\nseat.worker.0=fixed\nseat.spawned.1=scout\n");
        for (name, want) in [
            ("scout", Ok("spawned.1")),
            ("ghost", Err("no seat is named 'ghost' in this session.")),
            (
                "lead",
                Err(
                    "cannot retire 'lead' (main) — it is a launch seat the workspace promised, \
                     not a spawned one; use 'ae end' to end the session.",
                ),
            ),
            (
                "fixed",
                Err(
                    "cannot retire 'fixed' (worker.0) — it is a launch seat the workspace promised, \
                     not a spawned one; use 'ae end' to end the session.",
                ),
            ),
        ] {
            assert_eq!(
                super::removable_slot(&meta, name),
                want.map(str::to_owned).map_err(str::to_owned),
                "row {name}"
            );
        }
    }
}
