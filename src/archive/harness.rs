//! Inert conversation evidence, separate from the metadata `--from` inherits.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::meta::{self, RecordedConfigHome, RecordedConfigHomeBase};

/// The optional, immutable archive member; no resume path reads its contents.
pub const FILE: &str = "harness.tsv";
const BYTE_CAP: usize = 1 << 20;
const SLOT_CAP: usize = 1024;

/// A tool's conversation name, never a filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// Recorded tool basename.
    pub tool: String,
    /// Canonical lowercase UUID.
    pub id: String,
}

/// One roster slot's recorded purge evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// `main`, `worker.N` or `spawned.N`.
    pub slot: String,
    /// Recorded profile, empty when absent.
    pub profile: String,
    /// Recorded binary, retained as inert data even when unrecognised.
    pub binary: String,
    /// Account-store evidence; invalid and missing remain distinct.
    pub config_home: RecordedConfigHome,
    /// The home that selected an implicit account store.
    pub config_home_base: RecordedConfigHomeBase,
    /// Current conversation, when safely nameable.
    pub current: Option<Conversation>,
    /// Newest four predecessors, oldest first.
    pub priors: Vec<Conversation>,
}

/// One archive's independent conversation record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The immutable archive's UUID.
    pub archive_id: String,
    /// Roster slots in their recorded order.
    pub slots: Vec<Slot>,
}

/// Extract only roster-owned evidence, reporting every unusable or omitted id.
/// Publication has no size limit: the reader's bound must never make End refuse.
#[must_use]
pub fn from_meta(bytes: &[u8], archive_id: &str) -> (Record, Vec<String>) {
    let text = String::from_utf8_lossy(bytes);
    let meta = meta::Meta::parse(&text);
    let mut notes = damaged_evidence(meta.anomalies());
    let mut slots = Vec::new();
    for entry in meta.roster() {
        if !slot_valid(&entry.slot) {
            notes.push(format!(
                "slot {:?}: invalid roster slot omitted",
                entry.slot
            ));
            continue;
        }
        let binary = entry.binary.clone().unwrap_or_default();
        let note = |what: &str| format!("slot {}: {what}", entry.slot);
        let current = entry.harness_session.as_deref().and_then(|id| {
            let conversation = named(id, &binary);
            if conversation.is_none() && !id.is_empty() && id != "pending" {
                notes.push(note(
                    "current conversation omitted (id or tool is unprovable)",
                ));
            }
            conversation
        });
        let mut priors = Vec::new();
        for raw in meta.harness_session_prior(&entry.slot) {
            if let Some(conversation) = named(raw, &binary) {
                priors.push(conversation);
            } else {
                notes.push(note("predecessor omitted (id or tool is unprovable)"));
            }
        }
        let omitted = priors.len().saturating_sub(meta::PRIOR_MAX);
        for _ in 0..omitted {
            notes.push(note(
                "oldest predecessor omitted (record keeps newest four)",
            ));
        }
        priors.drain(..omitted);
        if entry.config_home == RecordedConfigHome::Invalid
            || entry.config_home_base == RecordedConfigHomeBase::Invalid
        {
            notes.push(note("invalid account-store evidence retained as invalid"));
        }
        slots.push(Slot {
            slot: entry.slot.clone(),
            profile: entry.profile.clone().unwrap_or_default(),
            binary,
            config_home: entry.config_home.clone(),
            config_home_base: entry.config_home_base.clone(),
            current,
            priors,
        });
    }
    let record = Record {
        archive_id: archive_id.to_owned(),
        slots,
    };
    if record.slots.len() > SLOT_CAP || encode(&record).len() > BYTE_CAP {
        notes.push("whole harness record retained; exceeds 1048576-byte/1024-slot reader bound, so client-record purge will skip it".to_owned());
    }
    (record, notes)
}

fn damaged_evidence(anomalies: &[meta::Anomaly]) -> Vec<String> {
    anomalies
        .iter()
        .filter_map(|anomaly| match anomaly {
            meta::Anomaly::DuplicateKey { key, .. }
            | meta::Anomaly::MalformedRosterEntry { key, .. } => key
                .strip_prefix("harness_session.")
                .or_else(|| key.strip_prefix("harness_session_prior."))
                .map(|slot| format!("slot {slot:?}: damaged {key:?} evidence omitted")),
            _ => None,
        })
        .collect()
}

fn tool_valid(tool: &str) -> bool {
    !tool.is_empty()
        && tool.len() <= 64
        && tool
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn named(raw: &str, binary: &str) -> Option<Conversation> {
    let (tool, id) = raw.split_once(':').unwrap_or((binary, raw));
    let id = super::canonical_uuid(id);
    (tool_valid(tool) && !id.is_empty()).then(|| Conversation {
        tool: tool.to_owned(),
        id,
    })
}

fn atom(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch == '%' || ch.is_control() {
            let mut bytes = [0; 4];
            for byte in ch.encode_utf8(&mut bytes).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn home(value: &RecordedConfigHome) -> String {
    match value {
        RecordedConfigHome::Missing => "missing".to_owned(),
        RecordedConfigHome::Absent => "absent".to_owned(),
        RecordedConfigHome::Unknown => "unknown".to_owned(),
        RecordedConfigHome::Invalid => "invalid".to_owned(),
        RecordedConfigHome::Path(p) => format!("path:{}", atom(&p.to_string_lossy())),
        RecordedConfigHome::Implicit(p) => format!("implicit:{}", atom(&p.to_string_lossy())),
    }
}

fn base(value: &RecordedConfigHomeBase) -> String {
    match value {
        RecordedConfigHomeBase::Missing => "missing".to_owned(),
        RecordedConfigHomeBase::Invalid => "invalid".to_owned(),
        RecordedConfigHomeBase::Path(p) => format!("path:{}", atom(&p.to_string_lossy())),
    }
}

fn conversation(value: &Conversation) -> String {
    format!("{}:{}", value.tool, value.id)
}

/// Encode all evidence without clipping it or consulting the filesystem.
#[must_use]
pub fn encode(record: &Record) -> Vec<u8> {
    let mut out = format!("version\t1\narchive_id\t{}\n", record.archive_id);
    for slot in &record.slots {
        let priors: Vec<_> = slot.priors.iter().map(conversation).collect();
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            slot.slot,
            atom(&slot.profile),
            atom(&slot.binary),
            home(&slot.config_home),
            base(&slot.config_home_base),
            slot.current.as_ref().map(conversation).unwrap_or_default(),
            priors.join(",")
        );
    }
    out.into_bytes()
}

fn unatom(value: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut source = value.as_bytes().iter().copied();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let pair = [source.next(), source.next()];
            let [Some(a), Some(b)] = pair else {
                return Err("harness: short escape".to_owned());
            };
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            let (Some(a), Some(b)) = (hex(a), hex(b)) else {
                return Err("harness: invalid escape".to_owned());
            };
            bytes.push(a * 16 + b);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| "harness: invalid UTF-8 atom".to_owned())
}

fn path(value: &str) -> Result<PathBuf, String> {
    let decoded = unatom(value)?;
    if !Path::new(&decoded).is_absolute() || decoded.chars().any(char::is_control) {
        return Err("harness: path must be absolute and control-free".to_owned());
    }
    Ok(PathBuf::from(decoded))
}

fn parse_home(value: &str) -> Result<RecordedConfigHome, String> {
    Ok(match value {
        "missing" => RecordedConfigHome::Missing,
        "absent" => RecordedConfigHome::Absent,
        "unknown" => RecordedConfigHome::Unknown,
        "invalid" => RecordedConfigHome::Invalid,
        _ if value.starts_with("path:") => RecordedConfigHome::Path(path(&value[5..])?),
        _ if value.starts_with("implicit:") => RecordedConfigHome::Implicit(path(&value[9..])?),
        _ => return Err("harness: invalid home token".to_owned()),
    })
}

fn parse_base(value: &str) -> Result<RecordedConfigHomeBase, String> {
    Ok(match value {
        "missing" => RecordedConfigHomeBase::Missing,
        "invalid" => RecordedConfigHomeBase::Invalid,
        _ if value.starts_with("path:") => RecordedConfigHomeBase::Path(path(&value[5..])?),
        _ => return Err("harness: invalid base token".to_owned()),
    })
}

fn slot_valid(value: &str) -> bool {
    value == "main"
        || value
            .strip_prefix("worker.")
            .or_else(|| value.strip_prefix("spawned."))
            .is_some_and(|n| {
                !n.is_empty()
                    && (n == "0" || !n.starts_with('0'))
                    && n.bytes().all(|b| b.is_ascii_digit())
            })
}

fn parse_conversation(value: &str) -> Result<Conversation, String> {
    let parsed = named(value, "").ok_or("harness: invalid conversation")?;
    if conversation(&parsed) != value {
        return Err("harness: noncanonical conversation".to_owned());
    }
    Ok(parsed)
}

/// Parse hostile bytes, proving one canonical encoding and the archive identity.
///
/// # Errors
/// Refuses oversized, malformed, noncanonical or wrong-archive records wholesale.
pub fn parse(bytes: &[u8], expected_archive_id: &str) -> Result<Record, String> {
    if bytes.len() > BYTE_CAP {
        return Err("harness: exceeds 1048576-byte reader bound".to_owned());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "harness: invalid UTF-8")?;
    let mut lines = text.split_terminator('\n');
    if lines.next() != Some("version\t1") {
        return Err("harness: unsupported version".to_owned());
    }
    let aid = lines
        .next()
        .and_then(|s| s.strip_prefix("archive_id\t"))
        .ok_or("harness: missing archive id")?;
    if aid.is_empty() || super::canonical_uuid(aid) != aid || aid != expected_archive_id {
        return Err("harness: archive id mismatch or noncanonical UUID".to_owned());
    }
    let mut record = Record {
        archive_id: aid.to_owned(),
        slots: Vec::new(),
    };
    let mut seen = BTreeSet::new();
    for line in lines {
        let fields: Vec<_> = line.split('\t').collect();
        let [slot, profile, binary, home, base, current, priors] = fields.as_slice() else {
            return Err("harness: expected seven slot fields".to_owned());
        };
        if record.slots.len() >= SLOT_CAP || !slot_valid(slot) || !seen.insert(*slot) {
            return Err("harness: invalid/duplicate slot or exceeds 1024-slot bound".to_owned());
        }
        let priors: Vec<_> = if priors.is_empty() {
            Vec::new()
        } else {
            priors
                .split(',')
                .map(parse_conversation)
                .collect::<Result<_, _>>()?
        };
        if priors.len() > meta::PRIOR_MAX {
            return Err("harness: exceeds four priors".to_owned());
        }
        record.slots.push(Slot {
            slot: (*slot).to_owned(),
            profile: unatom(profile)?,
            binary: unatom(binary)?,
            config_home: parse_home(home)?,
            config_home_base: parse_base(base)?,
            current: if current.is_empty() {
                None
            } else {
                Some(parse_conversation(current)?)
            },
            priors,
        });
    }
    if encode(&record) != bytes {
        return Err("harness: noncanonical encoding".to_owned());
    }
    Ok(record)
}
