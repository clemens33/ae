//! The read-only settings overlay's bodies: quota rows, the resolved
//! `[workspace]` table and the about facts. The loader reads through the
//! existing owners on its thread; this module resolves and carries, and
//! [`super::draw`] renders. Nothing here reads the world except through
//! `config`'s readers, and only when the loader calls it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where one config row's value came from, most specific first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigSource {
    /// Pinned in the home session's meta at launch.
    Launch,
    /// The session's origin local overlay.
    Session,
    /// The global file the meta names, else the current global.
    Global,
    /// The owning reader's default.
    Default,
}

impl ConfigSource {
    /// The word the row prints in its provenance column.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Session => "session",
            Self::Global => "global",
            Self::Default => "default",
        }
    }
}

/// One drawn config row: raw value, never judged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigRow {
    pub(crate) key: &'static str,
    pub(crate) value: String,
    pub(crate) source: ConfigSource,
    /// Read from the current global only; the row says `global only`.
    pub(crate) global_only: bool,
}

/// The paths each source word stands for, named once in the tab header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ConfigHeader {
    /// The origin overlay, when the meta selects one.
    pub(crate) session: Option<String>,
    /// The meta-recorded file, or the current global when that row is empty.
    pub(crate) global: String,
    /// The current global, only when it differs from `global`.
    pub(crate) current_global: Option<String>,
}

/// The config tab's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigView {
    /// No answer yet.
    Loading,
    /// The resolved table.
    Rows {
        header: ConfigHeader,
        rows: Vec<ConfigRow>,
    },
    /// One honest row naming the file that could not be read.
    Unreadable(String),
}

/// One drawn quota row: the owner's label, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuotaRow {
    pub(crate) label: String,
    pub(crate) header: bool,
}

/// The about tab's read facts. The ae version and the links are draw-site
/// constants, so they travel no channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AboutFacts {
    pub(crate) tmux_version: String,
    pub(crate) tmux_verdict: &'static str,
    pub(crate) state_root: String,
    pub(crate) config_file: String,
    pub(crate) server: String,
}

/// The `[prompt] instructions` in force for a session and where they come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CustomInstructions {
    /// `Global` (the meta-recorded file) or `Session` (its local overlay).
    pub(crate) source: ConfigSource,
    pub(crate) file: String,
    pub(crate) text: String,
}

/// One roster seat's section: the text ae renders for it, or why it renders
/// none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeatProtocol {
    pub(crate) name: String,
    pub(crate) slot: String,
    pub(crate) role: &'static str,
    pub(crate) text: Result<String, String>,
}

/// What the instructions tab draws for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Protocol {
    pub(crate) session: String,
    /// The config files the launch layering reads, global first.
    pub(crate) files: Vec<String>,
    pub(crate) custom: Option<CustomInstructions>,
    pub(crate) seats: Vec<SeatProtocol>,
}

/// The instructions tab's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstructionsView {
    /// No answer yet.
    Loading,
    /// One honest row: the session or a config file cannot be rendered from.
    Gap(String),
    Ready(Protocol),
}

/// Everything the overlay draws. Cold until the first answer lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsBodies {
    pub(crate) quota: Option<Vec<QuotaRow>>,
    pub(crate) config: ConfigView,
    pub(crate) about: Option<AboutFacts>,
    pub(crate) instructions: InstructionsView,
}

impl Default for SettingsBodies {
    fn default() -> Self {
        Self {
            quota: None,
            config: ConfigView::Loading,
            about: None,
            instructions: InstructionsView::Loading,
        }
    }
}

/// How one key resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Meta pin, else the layered files, else the default.
    Pinned,
    /// Origin overlay, else the meta-recorded global, else the default.
    Layered,
    /// The current global only, else the default.
    GlobalOnly,
}

/// Every documented `[workspace]` key in draw order: the key, its default as
/// drawn when no source declares it, and how it resolves. Defaults are the
/// owning readers' (plan §7); this table only spells them.
const KEYS: [(&str, &str, Scope); 20] = [
    ("main", "(unset)", Scope::Layered),
    ("workers", "(unset)", Scope::Layered),
    ("layout", "vertical", Scope::Pinned),
    ("auto_upgrade", "on", Scope::GlobalOnly),
    ("fleet_order", "(none)", Scope::GlobalOnly),
    ("restore", "on", Scope::GlobalOnly),
    ("palette", "darcula", Scope::Layered),
    ("icons", "on", Scope::Layered),
    ("theme", "on", Scope::Layered),
    ("motion", "on", Scope::Layered),
    ("chat", "on", Scope::Layered),
    ("quota", "on", Scope::Pinned),
    ("quota_every_secs", "300", Scope::Pinned),
    ("idle_nudge_secs", "300", Scope::Pinned),
    ("done_confirmations", "2", Scope::Pinned),
    ("auto_reseat", "off", Scope::GlobalOnly),
    ("auto_reseat_sessions", "(every session)", Scope::GlobalOnly),
    ("auto_reseat_grace_secs", "600", Scope::GlobalOnly),
    ("auto_reseat_at", "95", Scope::GlobalOnly),
    ("purge_agent_history", "off", Scope::Layered),
];

/// Resolve the config tab for the home session at `meta_dir`, whose current
/// global is `current_global`. `launch > session > global > default`; a
/// missing file contributes nothing, an unreadable or unparsable one turns
/// the whole tab into its one honest row. Without a home session, or with a
/// meta that cannot be read, the sources are unknown — also one honest row,
/// never guessed defaults.
pub(crate) fn resolve_config(meta_dir: Option<&Path>, current_global: &Path) -> ConfigView {
    let Some(meta_dir) = meta_dir else {
        return ConfigView::Unreadable("no home session".to_owned());
    };
    let bytes = match crate::meta::read_bytes(meta_dir) {
        Ok(bytes) => bytes,
        Err(why) => {
            return ConfigView::Unreadable(format!(
                "{}: {}",
                crate::store::open(meta_dir).meta_path().display(),
                why.to_string().escape_debug()
            ));
        }
    };
    let value = |key| crate::lifecycle::meta_value(&bytes, key);
    let pins: BTreeMap<&str, String> = [
        "layout",
        "quota",
        "quota_every_secs",
        "idle_nudge_secs",
        "done_confirmations",
    ]
    .into_iter()
    .map(|key| (key, value(key)))
    .filter(|(_, pin)| !pin.is_empty())
    .collect();
    let origin = value("origin");
    let overlay = if origin.is_empty() && value(crate::config::LOCAL_CONFIG_KEY).is_empty() {
        None
    } else {
        crate::config::local_overlay(meta_dir, &origin)
    };
    let recorded = value("config");
    let global = if recorded.is_empty() {
        current_global.to_path_buf()
    } else {
        PathBuf::from(recorded)
    };
    let current = current_global.to_path_buf();
    let mut files: Vec<PathBuf> = Vec::new();
    for candidate in overlay.iter().chain([&global, &current]) {
        if !files.contains(candidate) {
            files.push(candidate.clone());
        }
    }
    let mut read: BTreeMap<PathBuf, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for file in &files {
        match read_workspace_file(file) {
            Ok(entries) => drop(read.insert(file.clone(), entries)),
            Err(why) => return ConfigView::Unreadable(why),
        }
    }
    let entries = |file: &Path| read.get(file);
    let overlay_entries = overlay.as_ref().and_then(|file| entries(file));
    let global_entries = entries(&global);
    let current_entries = entries(current_global);
    let rows = KEYS
        .iter()
        .map(|(key, default, scope)| match scope {
            Scope::Pinned => match pins.get(key) {
                Some(pin) => row(key, pin.clone(), ConfigSource::Launch, false),
                None => layered_row(key, default, overlay_entries, global_entries),
            },
            Scope::Layered => layered_row(key, default, overlay_entries, global_entries),
            Scope::GlobalOnly => {
                let (value, source) = current_entries
                    .and_then(|entries| declared(entries, key))
                    .map_or_else(
                        || ((*default).to_owned(), ConfigSource::Default),
                        |value| (value, ConfigSource::Global),
                    );
                row(key, value, source, true)
            }
        })
        .collect();
    ConfigView::Rows {
        header: ConfigHeader {
            session: overlay.as_ref().map(|path| path.display().to_string()),
            global: global.display().to_string(),
            current_global: (global.as_path() != current_global)
                .then(|| current_global.display().to_string()),
        },
        rows,
    }
}

/// Resolve the instructions tab for session `name` whose records sit at
/// `meta_dir`: the custom instructions in force and, for every roster seat,
/// the text `render::seat_context_document` renders from the records now,
/// with the inputs `_run` hands it. A session, config file or seat it cannot
/// render from is a named gap, never a guess.
pub(crate) fn resolve_instructions(meta_dir: Option<&Path>, name: &str) -> InstructionsView {
    let gap = InstructionsView::Gap;
    let Some(dir) = meta_dir else {
        return gap(format!("{name}: no records on this machine"));
    };
    let bytes = match crate::meta::read_bytes(dir) {
        Ok(bytes) => bytes,
        Err(why) => {
            return gap(format!(
                "{}: {}",
                crate::store::open(dir).meta_path().display(),
                why.to_string().escape_debug()
            ));
        }
    };
    let value = |key| crate::lifecycle::meta_value(&bytes, key);
    let recorded = value("config");
    let global = (!recorded.is_empty()).then(|| PathBuf::from(recorded));
    let local = crate::config::local_overlay(dir, &value("origin"));
    let files = crate::config::ctx_config_files(global.as_deref(), local.as_deref());
    let sources: Vec<ConfigSource> = global
        .iter()
        .map(|_| ConfigSource::Global)
        .chain(local.iter().map(|_| ConfigSource::Session))
        .collect();
    for file in &files {
        if let Err(why) = readable_config(file) {
            return gap(why);
        }
    }
    let custom = crate::render::prompt_instructions(&files).and_then(|(at, text)| {
        Some(CustomInstructions {
            source: *sources.get(at)?,
            file: files.get(at)?.display().to_string(),
            text,
        })
    });
    let session = value("session");
    let session = if session.is_empty() {
        name.to_owned()
    } else {
        session
    };
    let meta = crate::meta::Meta::parse(&String::from_utf8_lossy(&bytes));
    if meta.roster().is_empty() {
        return gap(format!("{session}: the records name no seats"));
    }
    let seats = meta
        .roster()
        .iter()
        .map(|entry| {
            let role = crate::render::seat_role(&bytes, &entry.slot);
            SeatProtocol {
                name: entry.name.clone(),
                slot: entry.slot.clone(),
                role: role.word(),
                text: seat_text(dir, &bytes, &session, &files, entry, role),
            }
        })
        .collect();
    InstructionsView::Ready(Protocol {
        session,
        files: files
            .iter()
            .map(|file| file.display().to_string())
            .collect(),
        custom,
        seats,
    })
}

/// One seat's rendered context, or the fact that stops it. The work directory
/// is the one `_run` would hand over, as recorded: a launch also proves it
/// exists, which a settings read does not.
fn seat_text(
    dir: &Path,
    bytes: &[u8],
    session: &str,
    files: &[PathBuf],
    entry: &crate::meta::RosterEntry,
    role: crate::render::SeatRole,
) -> Result<String, String> {
    if role == crate::render::SeatRole::Other || !crate::requests::is_slot(&entry.slot) {
        return Err("unknown slot: ae renders no protocol for it".to_owned());
    }
    if !crate::config::is_agent_name(&entry.name) {
        return Err("the recorded seat name is not a valid agent name".to_owned());
    }
    let inherited = crate::lifecycle::meta_value(bytes, "work_dir");
    let (work_dir, provenance) = match crate::meta::raw_seat_work_dir(bytes, &entry.slot)? {
        Some(row) => (row, crate::meta::SeatProvenance::Explicit),
        None if inherited.is_empty() => {
            return Err("the session records no work_dir".to_owned());
        }
        None => (inherited, crate::meta::SeatProvenance::Inherited),
    };
    Ok(crate::render::seat_context_document(
        dir,
        session,
        &work_dir,
        &entry.slot,
        files,
        provenance,
    ))
}

/// Why `file` cannot be rendered from, when it exists but cannot be read or
/// is torn. Only the verdict is taken from the generic reader: the text a
/// launch injects is `render`'s own grammar.
fn readable_config(file: &Path) -> Result<(), String> {
    let text = crate::config::read_global_text(file)
        .map_err(|why| format!("{}: {why}", file.display()))?;
    if let Some(text) = text {
        crate::config::section_entries(file, &text, "prompt")
            .map_err(|why| format!("{}: {}", file.display(), why.escape_debug()))?;
    }
    Ok(())
}

/// One layered row: the overlay wins over the meta-recorded global, which
/// wins over the default.
fn layered_row(
    key: &str,
    default: &str,
    overlay: Option<&BTreeMap<String, Option<String>>>,
    global: Option<&BTreeMap<String, Option<String>>>,
) -> ConfigRow {
    let (value, source) = overlay
        .and_then(|entries| declared(entries, key))
        .map(|value| (value, ConfigSource::Session))
        .or_else(|| {
            global
                .and_then(|entries| declared(entries, key))
                .map(|value| (value, ConfigSource::Global))
        })
        .unwrap_or_else(|| (default.to_owned(), ConfigSource::Default));
    row(key, value, source, false)
}

fn row(key: &str, value: String, source: ConfigSource, global_only: bool) -> ConfigRow {
    let key = KEYS
        .iter()
        .find(|(known, _, _)| *known == key)
        .map_or("unknown", |(known, _, _)| *known);
    ConfigRow {
        key,
        value,
        source,
        global_only,
    }
}

/// The value `entries` last declares for `key`: a declaration outside the
/// entry grammar reads `unusable`, never absent.
fn declared(entries: &BTreeMap<String, Option<String>>, key: &str) -> Option<String> {
    entries
        .get(key)
        .map(|value| value.clone().unwrap_or_else(|| "unusable".to_owned()))
}

/// The `[workspace]` entries `file` declares, last wins. A missing file is
/// no entries; anything else unreadable names the file and the reason.
fn read_workspace_file(file: &Path) -> Result<BTreeMap<String, Option<String>>, String> {
    let text = crate::config::read_global_text(file)
        .map_err(|why| format!("{}: {why}", file.display()))?;
    let Some(text) = text else {
        return Ok(BTreeMap::new());
    };
    let entries = crate::config::section_entries(file, &text, "workspace")
        .map_err(|why| format!("{}: {}", file.display(), why.escape_debug()))?;
    let mut last = BTreeMap::new();
    for entry in entries {
        last.insert(entry.key, entry.value);
    }
    Ok(last)
}
