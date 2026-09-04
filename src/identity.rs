//! The identity v2 core entries: `_launch-plan`, `_meta-init` and `_roster`.
//!
//! These are the four things bash may no longer decide for itself once a
//! session's identity is alias-free — which config the workspace resolves to,
//! what the first meta says, how a seat is added or removed, and what the
//! roster currently is. Each is an underscored core entry (never human-typed)
//! and each speaks ONE framing, described below.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{self, IdentityConfig};
use crate::launch_cmd;
use crate::meta::{self, Meta, RosterSchema};
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
/// The trailer counts the RECORDS that precede it — never a side effect's size.
/// A write-only entry (`_meta-init`, `migrate`, `set-harness-session`) therefore
/// ends with `end<US>0`: the glue's validator compares the count to the body it
/// received, and a trailer that counted seats consumed made it refuse a correct
/// publish (found by the first live launch).
fn trailer(count: usize) -> String {
    format!("end{US}{count}\n")
}

/// Write a built document to stdout. Callers build the WHOLE document first,
/// so a refusal never publishes a prefix.
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
/// this framing, cannot round-trip. `\n` and [`US`] are checked by
/// [`record`]; this is the wider guard for text that is about to become a
/// META line, where a stray CR would ride into a value invisibly.
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
    /// compact's frozen roster passes the names it froze. `-` and the empty
    /// string both mean NO workers, which is a different thing from the flag
    /// being absent (that keeps the config's own list).
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
        cfg.workers = Some(if workers == NONE {
            String::new()
        } else {
            workers.clone()
        });
    }
    let plan = match config::launch_plan(&cfg, flags.main.as_deref()) {
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
            &seat.command,
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
            binary: optional(binary),
            harness_session: optional(sid),
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
    let seats = match parse_seat_records(stdin, flags.replace) {
        Ok(seats) => seats,
        Err(why) => return refuse(&why, err),
    };
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
            "ae: {} needs a subcommand: add-seat, remove-seat, set-harness-session, migrate or list",
            crate::cli::ROSTER
        )?;
        return Ok(EXIT_USAGE);
    };
    match (subcommand.as_str(), rest) {
        ("add-seat", [name, flags @ ..]) => add_seat(dir, name, flags, out, err),
        ("remove-seat", [name]) => remove_seat(dir, name, out, err),
        ("set-harness-session", [slot, sid]) => set_harness_session(dir, slot, sid, out, err),
        ("migrate", flags) => migrate(dir, flags, out, err),
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

/// Every anomaly that puts the roster's IDENTITY in doubt, rendered — the SAME
/// provenance grain as migration (`roster::roster_doubting`): a slot claimed by
/// both schemas, one name on two seats, a malformed roster row or line, and a
/// duplicate or unreadable key under an identity prefix. An `UnknownKey`
/// elsewhere is tolerated: a real meta is full of keys this parser does not
/// interpret (`session`, `layout`, `config`, `launch_id.*`), and refusing on
/// those would refuse every live session. The identity-prefix duplicate IS a
/// doubt, though (colead, integrated gate): `Meta::parse` drops the seat that
/// key belonged to, so a roster read past it is SHORTER than the file — and a
/// resume that republishes the shorter list deletes the seat for good.
fn identity_doubts(current: &Meta) -> Vec<String> {
    let mut doubts: Vec<String> = current
        .anomalies()
        .iter()
        .filter(|anomaly| roster::roster_doubting(anomaly))
        .map(ToString::to_string)
        .collect();
    // A v1 `agent.<slot>` row inside a schema=2 meta is a seat the v2 reader
    // would silently omit — and a resume would then delete. The core never
    // writes one after migration, so its presence is a hand edit or a torn
    if current.schema() == Some("2") {
        doubts.extend(
            current
                .roster()
                .iter()
                .filter(|entry| entry.schema == RosterSchema::V1)
                .map(|entry| format!("v1 row agent.{} inside a schema=2 meta", entry.slot)),
        );
    }
    doubts
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

/// A record's KEY — everything before its first `=`, a record without one
/// being its own key. The same reading [`meta::rewritten`]'s awk does.
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
    match add_seat_slot(dir, name, &profile, &binary, sid.as_deref()) {
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

/// Take the lowest free `spawned.<n>` for `name` and publish the seat, under
/// one hold of the meta lock — the decision half of `add-seat`, as a value.
///
/// # Errors
///
/// The refusal, phrased as [`refuse`] prints it: a bad or taken name, a v1 or
/// doubtful roster, an unwritable meta, a control byte in a value.
pub fn add_seat_slot(
    dir: &Path,
    name: &str,
    profile: &str,
    binary: &str,
    sid: Option<&str>,
) -> Result<String, String> {
    let _held = meta::lock(dir).map_err(|why| format!("cannot take the meta lock: {why}"))?;
    let text = text_of(dir)?;
    let current = Meta::parse(&text);
    if let Some(why) = seat_write_refusal(&current) {
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
    let mut next = text;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    let block = roster::render(&[SeatLines {
        slot: slot.clone(),
        name: name.to_owned(),
        profile: profile.to_owned(),
        binary: Some(binary.to_owned()),
        harness_session: sid.map(ToOwned::to_owned),
    }]);
    // `render` opens the block it builds with `schema=2`. This document already
    // declares it — that is what the gate above proved — and a second one would
    // be a DUPLICATE KEY, which `Meta::parse` invalidates: the meta would stop
    next.push_str(block.strip_prefix("schema=2\n").unwrap_or(&block));
    publish(dir, &next)?;
    Ok(slot)
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

/// `add-seat`'s flags. The two required ones are `Option` here and checked at
/// the call site, so "you did not name a profile" is a usage error with its own
/// message rather than a parse failure naming the wrong word.
#[derive(Debug, Default)]
struct AddSeatFlags {
    profile: Option<String>,
    binary: Option<String>,
    sid: Option<String>,
}

/// Why a meta may not be WRITTEN to as a v2 roster, or `None` when it may.
fn seat_write_refusal(current: &Meta) -> Option<String> {
    if current.schema() != Some("2") {
        return Some(match current.schema() {
            Some(other) => format!(
                "this session's meta declares schema={other}, not 2 — migrate it first ('{} <dir> migrate').",
                crate::cli::ROSTER
            ),
            None => format!(
                "this session's meta declares no schema, so it is a v1 roster — migrate it first ('{} <dir> migrate').",
                crate::cli::ROSTER
            ),
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

/// `_roster <dir> migrate [--global <f>] [--local <f>]` — move a v1 roster to
/// v2, once.
///
/// # Errors
///
/// [`crate::Error::Io`] when `out` or `err` cannot be written.
fn migrate(
    dir: &Path,
    flags: &[String],
    out: &mut impl Write,
    err: &mut impl Write,
) -> crate::Result<u8> {
    let files = match config_files(flags) {
        Ok(files) => files,
        Err(word) => return usage(crate::cli::ROSTER, &word, err),
    };
    let cfg = match files.read() {
        Ok(cfg) => cfg,
        Err(why) => {
            writeln!(err, "{why}")?;
            return Ok(EXIT_REFUSED);
        }
    };
    let _held = match meta::lock(dir) {
        Ok(held) => held,
        Err(why) => return refuse(&format!("cannot take the meta lock: {why}"), err),
    };
    let text = match text_of(dir) {
        Ok(text) => text,
        Err(why) => return refuse(&why, err),
    };
    let current = Meta::parse(&text);
    if current.schema() == Some("2") {
        return emit(&trailer(0), out);
    }
    let seats = match roster::migrate(&current, |profile| cfg.profile(profile).is_some()) {
        Ok(seats) => seats,
        Err(refusals) => {
            write!(err, "{}", roster::render_refusals(&refusals))?;
            return Ok(EXIT_REFUSED);
        }
    };
    let mut next = String::new();
    for row in records(&text) {
        let key = key_of(row);
        if key == "schema" || key.starts_with("agent.") || key.starts_with("agent_bin.") {
            continue;
        }
        next.push_str(row);
        next.push('\n');
    }
    next.push_str(&roster::render(&seats));
    if let Err(why) = publish(dir, &next) {
        return refuse(&why, err);
    }
    emit(&trailer(0), out)
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
            &format!(
                "this session's meta is not an identity v2 roster (schema={}); migrate it first.",
                current.schema().unwrap_or("absent")
            ),
            err,
        );
    }
    // FAIL CLOSED on a roster in doubt, BEFORE emitting a record: `Meta::parse`
    // drops the seats an anomaly touches, so the list would be shorter than the
    // file — and the resume that consumes this list republishes it through
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
    let mut body = String::new();
    for entry in current.roster() {
        // The seat's own profile row, resolved as an OPTION — never through the
        // rendered `-`, or a config that happened to define a profile literally
        // named `-` would resolve a seat that has no profile at all.
        let resolved = entry
            .profile
            .as_deref()
            .and_then(|profile| cfg.profile(profile))
            .and_then(|command| {
                launch_cmd::lex_simple_command(command)
                    .ok()
                    .zip(Some(command))
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
                command,
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
        // The uniform arity is the contract one bash `read` of eight variables
        // depends on: a record that grew or shrank would silently shift the
        // command into the wrong variable.
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
        // and the plan is main alone. That the worker row is gone is the
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
        // #59 C3-2, the resume half: the name came out of the session's own meta,
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
        // A bash-era row nobody cleaned up still CLAIMS its index: allocating
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
        assert!(err.contains("migrate it first"), "{err}");
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
    fn migrate_keeps_every_non_roster_line_byte_identical_and_in_order() {
        let scratch = Scratch::new("migrate");
        let cfg = scratch.file("config", CONFIG);
        scratch.file(
            "meta",
            "mode=local\n\
             work_dir=/tmp/y\n\
             agent.main=fable5:lead:e1\n\
             agent_bin.main=claude\n\
             launch_id.main=uuid-1\n\
             agent.worker.0=gpt56:colead\n\
             agent_bin.worker.0=codex\n\
             goal=ship it\n",
        );
        let (code, out, err) = roster(
            scratch.dir(),
            &["migrate", "--global", &cfg.to_string_lossy()],
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert_eq!(out, format!("end{US}0\n"));
        assert_eq!(
            scratch.meta(),
            "mode=local\n\
             work_dir=/tmp/y\n\
             launch_id.main=uuid-1\n\
             goal=ship it\n\
             schema=2\n\
             seat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nharness_session.main=e1\n\
             seat.worker.0=colead\nprofile.worker.0=gpt56\nagent_bin.worker.0=codex\n",
            "the non-roster lines kept their bytes AND their order"
        );
        // A second migrate is a NO-OP, not a refusal: a resume runs it
        // unconditionally, and refusing would stop the launch.
        let before = scratch.meta();
        let (code, out, err) = roster(
            scratch.dir(),
            &["migrate", "--global", &cfg.to_string_lossy()],
        );
        assert_eq!((code, out, err.as_str()), (0, format!("end{US}0\n"), ""));
        assert_eq!(scratch.meta(), before);
    }

    #[test]
    fn migrate_refuses_a_half_migrated_meta_and_an_undefined_profile_touching_nothing() {
        let scratch = Scratch::new("migrate-refuse");
        let cfg = scratch.file("config", CONFIG);
        let path = cfg.to_string_lossy().into_owned();
        // schema=2 BESIDE v1 agent rows: `schema()` says 2, so this takes the
        // already-migrated branch and reports success without touching a thing.
        // The v1 rows are left exactly as found rather than silently dropped.
        let mixed = Scratch::new("migrate-mixed");
        let before = "schema=2\nagent.main=fable5:lead\n";
        mixed.file("meta", before);
        let (code, out, err) = roster(mixed.dir(), &["migrate", "--global", &path]);
        assert_eq!((code, out, err.as_str()), (0, format!("end{US}0\n"), ""));
        assert_eq!(mixed.meta(), before, "the no-op branch published nothing");
        // schema=1 beside v1 rows and an alias no [profiles] defines: refused
        // with the full list, and the v1 meta is left byte-identical.
        let unbound = "schema=1\nagent.main=ghost:lead\nagent.worker.0=alsogone:colead\n";
        scratch.file("meta", unbound);
        let (code, out, err) = roster(scratch.dir(), &["migrate", "--global", &path]);
        assert_eq!(code, EXIT_REFUSED);
        assert!(out.is_empty());
        assert!(
            err.contains("'ghost'") && err.contains("'alsogone'"),
            "{err}"
        );
        assert_eq!(scratch.meta(), unbound);
    }

    #[test]
    fn migrate_leaves_exactly_one_schema_marker_when_the_v1_meta_declared_one() {
        let scratch = Scratch::new("migrate-schema");
        let cfg = scratch.file("config", CONFIG);
        scratch.file("meta", "schema=1\nagent.main=fable5:lead\n");
        let (code, _, err) = roster(
            scratch.dir(),
            &["migrate", "--global", &cfg.to_string_lossy()],
        );
        assert_eq!((code, err.as_str()), (0, ""));
        let meta = scratch.meta();
        // Two `schema=` records would be a DUPLICATE KEY, which `Meta::parse`
        // invalidates — the just-migrated meta would read as though it had not
        // been, and every later write would refuse it.
        assert_eq!(meta.matches("schema=").count(), 1, "{meta}");
        assert!(meta.contains("schema=2\n"), "{meta}");
        assert_eq!(
            crate::meta::Meta::parse(&meta).schema(),
            Some("2"),
            "the migrated meta reads back as v2"
        );
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
        // A profile whose command is no longer one simple command is
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

    #[test]
    fn list_refuses_a_roster_in_doubt_and_leaves_the_meta_untouched() {
        // Colead, integrated gate: one name on two seats used to list the main
        // seat alone with rc 0 — and the resume consuming that list republished
        // it, deleting both worker seats for good. The doubt grain is migration's.
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
        assert!(err.contains("not an identity v2 roster"), "{err}");
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
}
