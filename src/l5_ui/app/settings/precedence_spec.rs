//! Which source wins in the Config and Instructions tabs, decided over inputs
//! already read: no file, environment or clock. Each expected value comes from
//! the doc line its test names or, where no doc states the rule, from the code
//! comment the lead ruled the oracle. The ids in brackets are the decisions of
//! `.local/settingsseam/precedence.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{
    ConfigHeader, ConfigPlan, ConfigRow, ConfigSource, ConfigView, CustomInstructions,
    InstructionsPlan, InstructionsView, prompt_text, workspace_entries,
};

const CURRENT: &str = "/home/u/.ae/config";
const RECORDED: &str = "/home/u/.ae/recorded";
const OVERLAY: &str = "/work/api/.ae/config";

/// The text `files` lists for `file`, `None` when the file does not exist.
fn text_of(files: &[(&str, &str)], file: &Path) -> Option<String> {
    files
        .iter()
        .find(|(path, _)| Path::new(path) == file)
        .map(|(_, text)| (*text).to_owned())
}

/// The config tab of a session with `meta` and the selected `overlay`, where
/// `files` holds the text of every file that exists.
fn config_tab(
    meta: &str,
    overlay: Option<&str>,
    files: &[(&str, &str)],
) -> (ConfigHeader, Vec<ConfigRow>) {
    let plan = ConfigPlan::of(
        meta.as_bytes(),
        overlay.map(PathBuf::from),
        Path::new(CURRENT),
    );
    let mut read = BTreeMap::new();
    for file in plan.files() {
        let entries = workspace_entries(&file, Ok(text_of(files, &file))).expect("config text");
        read.insert(file, entries);
    }
    match plan.view(&read) {
        ConfigView::Rows { header, rows } => (header, rows),
        other => panic!("rows expected: {other:?}"),
    }
}

/// One row: its value, source and global-only mark.
fn row(rows: &[ConfigRow], key: &str) -> (String, ConfigSource, bool) {
    let row = rows
        .iter()
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("no row {key}"));
    (row.value.clone(), row.source, row.global_only)
}

fn drawn(value: &str, source: ConfigSource, global_only: bool) -> (String, ConfigSource, bool) {
    (value.to_owned(), source, global_only)
}

/// [C7, C8] config.md:5-6 "the project file shadows the global file key by
/// key"; app.md:1314-1316: `session` (its origin overlay) over `global` (the
/// meta-recorded file, the current global only when that row is empty) over
/// `default`. Defaults: config.md:281-282.
#[test]
fn a_layered_key_takes_the_overlay_then_the_recorded_global_then_the_default() {
    let meta = format!("origin=/work/api\nconfig={RECORDED}\n");
    let (_, rows) = config_tab(
        &meta,
        Some(OVERLAY),
        &[
            (OVERLAY, "[workspace]\npalette = b\n"),
            (RECORDED, "[workspace]\npalette = a\nicons = off\n"),
            (CURRENT, "[workspace]\ntheme = off\nicons = on\n"),
        ],
    );
    assert_eq!(
        row(&rows, "palette"),
        drawn("b", ConfigSource::Session, false)
    );
    assert_eq!(
        row(&rows, "icons"),
        drawn("off", ConfigSource::Global, false)
    );
    assert_eq!(
        row(&rows, "theme"),
        drawn("on", ConfigSource::Default, false)
    );
}

/// [C8] app.md:1315-1316: `global` is the meta-recorded file, the current
/// global only when that row is empty.
#[test]
fn the_global_is_the_recorded_file_else_the_current_one() {
    let (header, rows) = config_tab(
        "origin=/work/api\n",
        None,
        &[(CURRENT, "[workspace]\npalette = b\n")],
    );
    assert_eq!(header.global, CURRENT);
    assert_eq!(
        row(&rows, "palette"),
        drawn("b", ConfigSource::Global, false)
    );
    let (header, _) = config_tab(&format!("config={RECORDED}\n"), None, &[]);
    assert_eq!(header.global, RECORDED);
}

/// [C1] app.md:1312-1313: with no session the tab is "the current global
/// alone".
#[test]
fn no_session_is_the_current_global_alone() {
    let plan = ConfigPlan::of(b"", None, Path::new(CURRENT));
    assert_eq!(plan.files(), [PathBuf::from(CURRENT)]);
    let (header, rows) = config_tab(
        "",
        None,
        &[(CURRENT, "[workspace]\nicons = off\nlayout = horizontal\n")],
    );
    assert_eq!(header.session, None);
    assert_eq!(header.global, CURRENT);
    assert_eq!(
        row(&rows, "icons"),
        drawn("off", ConfigSource::Global, false)
    );
    assert_eq!(
        row(&rows, "layout"),
        drawn("horizontal", ConfigSource::Global, false)
    );
}

/// [C3, C5] app.md:1764-1766: Settings shows the launch-pinned values of
/// exactly `layout`, `quota`, `quota_every_secs`, `idle_nudge_secs` and
/// `done_confirmations`, whatever the config says now; app.md:1314 names them
/// `launch`.
#[test]
fn exactly_the_five_launch_pins_win_over_every_file() {
    let every_key = super::KEYS
        .iter()
        .map(|(key, _, _)| format!("{key}=pinned\n"))
        .collect::<Vec<_>>()
        .concat();
    let meta = format!("{every_key}origin=/work/api\nconfig={RECORDED}\n");
    let file = "[workspace]\nlayout = vertical\nquota = on\nquota_every_secs = 60\n\
                idle_nudge_secs = 60\ndone_confirmations = 1\npalette = a\n";
    let (_, rows) = config_tab(
        &meta,
        Some(OVERLAY),
        &[(OVERLAY, file), (RECORDED, file), (CURRENT, file)],
    );
    let mut launch: Vec<(&str, &str)> = rows
        .iter()
        .filter(|row| row.source == ConfigSource::Launch)
        .map(|row| (row.key, row.value.as_str()))
        .collect();
    launch.sort_unstable();
    assert_eq!(
        launch,
        [
            ("done_confirmations", "pinned"),
            ("idle_nudge_secs", "pinned"),
            ("layout", "pinned"),
            ("quota", "pinned"),
            ("quota_every_secs", "pinned"),
        ]
    );
    assert_eq!(
        row(&rows, "palette"),
        drawn("a", ConfigSource::Session, false)
    );
}

/// [C6] app.md:1314-1316: a pinnable key the meta does not pin resolves like
/// any layered key: `session`, then `global`, then the default
/// (config.md:256).
#[test]
fn an_unpinned_launch_key_is_layered() {
    let meta = format!("origin=/work/api\nconfig={RECORDED}\n");
    let (_, rows) = config_tab(
        &meta,
        Some(OVERLAY),
        &[
            (OVERLAY, "[workspace]\nlayout = horizontal\n"),
            (
                RECORDED,
                "[workspace]\nlayout = vertical\nquota_every_secs = 60\n",
            ),
        ],
    );
    assert_eq!(
        row(&rows, "layout"),
        drawn("horizontal", ConfigSource::Session, false)
    );
    assert_eq!(
        row(&rows, "quota_every_secs"),
        drawn("60", ConfigSource::Global, false)
    );
    assert_eq!(
        row(&rows, "idle_nudge_secs"),
        drawn("300", ConfigSource::Default, false)
    );
}

/// [C4] No doc line. Oracle (lead ruling): a meta value is "empty when the key
/// is absent" (lifecycle.rs `meta_value`), and a pinned key is "Meta pin, else
/// the layered files" (settings.rs `Scope::Pinned`): an empty pin row is no
/// pin.
#[test]
fn an_empty_pin_row_is_no_pin() {
    let meta = format!("layout=\nconfig={RECORDED}\n");
    let (_, rows) = config_tab(
        &meta,
        None,
        &[(RECORDED, "[workspace]\nlayout = horizontal\n")],
    );
    assert_eq!(
        row(&rows, "layout"),
        drawn("horizontal", ConfigSource::Global, false)
    );
}

/// [C9, C10] app.md:1316-1317: `global only` keys "read the current global
/// alone". Which keys: config.md:258, :261, :262, :275, :276, :277, :279, and
/// config.md:514-517 for the five auto reseat knobs. Default: config.md:275.
#[test]
fn global_only_keys_read_the_current_global_alone() {
    let mut global_only = [
        "auto_upgrade",
        "auto_trust",
        "fleet_order",
        "restore",
        "auto_reseat",
        "auto_reseat_sessions",
        "auto_reseat_grace_secs",
        "auto_reseat_at",
        "auto_reseat_lead_at",
    ];
    global_only.sort_unstable();
    let rows = global_only
        .iter()
        .map(|key| format!("{key} = elsewhere\n"))
        .collect::<Vec<_>>()
        .concat();
    let elsewhere = format!("[workspace]\n{rows}");
    let meta = format!("origin=/work/api\nconfig={RECORDED}\n");
    let (_, rows) = config_tab(
        &meta,
        Some(OVERLAY),
        &[
            (OVERLAY, elsewhere.as_str()),
            (RECORDED, elsewhere.as_str()),
            (CURRENT, "[workspace]\nfleet_order = a, b\n"),
        ],
    );
    let mut marked: Vec<&str> = rows
        .iter()
        .filter(|row| row.global_only)
        .map(|row| row.key)
        .collect();
    marked.sort_unstable();
    assert_eq!(marked, global_only);
    assert_eq!(
        row(&rows, "fleet_order"),
        drawn("a, b", ConfigSource::Global, true)
    );
    assert_eq!(
        row(&rows, "auto_upgrade"),
        drawn("on", ConfigSource::Default, true)
    );
    assert!(rows.iter().all(|row| row.value != "elsewhere"));
}

/// [C14] No doc line. Oracle (lead ruling): settings.rs `declared`, "a
/// declaration outside the entry grammar reads `unusable`, never absent", so
/// it still decides over the files under it.
#[test]
fn a_declaration_outside_the_grammar_reads_unusable_and_decides() {
    let meta = format!("origin=/work/api\nconfig={RECORDED}\n");
    let (_, rows) = config_tab(
        &meta,
        Some(OVERLAY),
        &[
            (OVERLAY, "[workspace]\npalette =\n"),
            (RECORDED, "[workspace]\npalette = a\n"),
        ],
    );
    assert_eq!(
        row(&rows, "palette"),
        drawn("unusable", ConfigSource::Session, false)
    );
}

/// [C15] app.md:1480-1481: "A torn config shows one honest row naming the
/// file"; an unreadable one is named with its reason.
#[test]
fn a_torn_or_unreadable_config_file_names_itself() {
    let torn = "[prompt]\ninstructions = \"\"\"\nunclosed\n".to_owned();
    let why = workspace_entries(Path::new(OVERLAY), Ok(Some(torn))).expect_err("torn");
    assert!(why.contains(OVERLAY), "{why}");
    let why = workspace_entries(Path::new(RECORDED), Err("permission denied".to_owned()))
        .expect_err("unreadable");
    assert!(
        why.contains(RECORDED) && why.contains("permission denied"),
        "{why}"
    );
}

/// [C17] With no file every key draws its default: config.md's default column
/// for `layout` :251, `quota` :254, `quota_every_secs` :255,
/// `idle_nudge_secs` :256, `done_confirmations` :257, `auto_reseat` :258,
/// `auto_reseat_grace_secs` :260, `auto_reseat_at` :261, `auto_upgrade` :275,
/// `restore` :277, `auto_trust` :279, `palette` :280, `icons` :281, `theme`
/// :282, `motion` :283, `chat` :284, and `purge_agent_history` commands.md:3004
/// (unset keeps the history: `off`). Lead ruling D3: the tab draws the raw
/// config, so `main`, `workers`, `fleet_order`, `auto_reseat_sessions` and
/// `auto_reseat_lead_at` draw their raw spellings, pinned as table data.
#[test]
fn every_key_draws_its_default_when_no_file_declares_it() {
    let (_, rows) = config_tab("", None, &[]);
    assert!(rows.iter().all(|row| row.source == ConfigSource::Default));
    let mut drawn: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.key, row.value.as_str()))
        .collect();
    drawn.sort_unstable();
    let mut expected = [
        ("main", "(unset)"),
        ("workers", "(unset)"),
        ("layout", "vertical"),
        ("auto_upgrade", "on"),
        ("auto_trust", "on"),
        ("fleet_order", "(none)"),
        ("restore", "on"),
        ("palette", "darcula"),
        ("icons", "on"),
        ("theme", "on"),
        ("motion", "on"),
        ("chat", "on"),
        ("quota", "on"),
        ("quota_every_secs", "300"),
        ("idle_nudge_secs", "300"),
        ("done_confirmations", "2"),
        ("auto_reseat", "off"),
        ("auto_reseat_sessions", "(every session)"),
        ("auto_reseat_grace_secs", "600"),
        ("auto_reseat_at", "95"),
        ("auto_reseat_lead_at", "(auto_reseat_at)"),
        ("purge_agent_history", "off"),
    ];
    expected.sort_unstable();
    assert_eq!(drawn, expected);
}

/// A meta row that gives the session one roster seat.
const SEATS: &str = "seat.main=lead\n";

/// The winner in force: source, file and text.
type Won = Option<(ConfigSource, String, String)>;

fn won(source: ConfigSource, file: &str, text: &str) -> (ConfigSource, String, String) {
    (source, file.to_owned(), text.to_owned())
}

/// The instructions tab of session `api` with `meta` and the selected `local`
/// overlay, where `files` holds the text of every file that exists.
fn instructions(meta: &str, local: Option<&str>, files: &[(&str, &str)]) -> InstructionsView {
    let plan = InstructionsPlan::of(meta.as_bytes(), local.map(Path::new));
    let texts: Vec<Option<String>> = plan
        .files()
        .iter()
        .map(|file| prompt_text(file, Ok(text_of(files, file))).expect("config text"))
        .collect();
    plan.view("api", meta.as_bytes(), &texts)
}

/// The `instructions` and the `spawn_instructions` in force.
fn in_force(view: &InstructionsView) -> (Won, Won) {
    let InstructionsView::Ready(protocol) = view else {
        panic!("ready expected: {view:?}");
    };
    let pick = |custom: &Option<CustomInstructions>| {
        custom
            .as_ref()
            .map(|custom| (custom.source, custom.file.clone(), custom.text.clone()))
    };
    (pick(&protocol.custom), pick(&protocol.spawn))
}

/// [I5, I7, I9] config.md:776 "Per-project `.ae/config` overrides the global
/// one"; app.md:1333-1335: the file is named `global` (the meta-recorded file)
/// or `session` (its local overlay), and the last file declaring the key wins.
#[test]
fn the_last_file_declaring_instructions_wins_and_is_named() {
    let meta = format!("{SEATS}config={RECORDED}\n");
    let global = "[prompt]\ninstructions = from global\n";
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, global),
            (OVERLAY, "[prompt]\ninstructions = from session\n"),
        ],
    );
    assert_eq!(
        in_force(&view).0,
        Some(won(ConfigSource::Session, OVERLAY, "from session"))
    );
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[(RECORDED, global), (OVERLAY, "[workspace]\nchat = on\n")],
    );
    assert_eq!(
        in_force(&view).0,
        Some(won(ConfigSource::Global, RECORDED, "from global"))
    );
}

/// [I7] app.md:1334-1335 "the last file declaring the key wins even when
/// empty"; config.md:823-825: an empty overlay value masks the global text.
#[test]
fn an_empty_overlay_value_masks_the_global_text() {
    let meta = format!("{SEATS}config={RECORDED}\n");
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, "[prompt]\ninstructions = from global\n"),
            (OVERLAY, "[prompt]\ninstructions = \"\"\n"),
        ],
    );
    assert_eq!(in_force(&view).0, None);
}

/// [I8] config.md:823-825: "The two keys resolve apart. Each follows the same
/// layering: the last config file that declares the key wins, even when empty,
/// so a local `spawn_instructions = ""` masks a global text."
#[test]
fn spawn_instructions_resolve_apart_with_their_own_mask() {
    let meta = format!("{SEATS}config={RECORDED}\n");
    let global = "[prompt]\ninstructions = rules\nspawn_instructions = spend\n";
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, global),
            (OVERLAY, "[prompt]\ninstructions = \"\"\n"),
        ],
    );
    assert_eq!(
        in_force(&view),
        (None, Some(won(ConfigSource::Global, RECORDED, "spend")))
    );
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, global),
            (OVERLAY, "[prompt]\nspawn_instructions = \"\"\n"),
        ],
    );
    assert_eq!(
        in_force(&view),
        (Some(won(ConfigSource::Global, RECORDED, "rules")), None)
    );
}

/// [I3, I9] app.md:1333-1334: `global` is the meta-recorded file. With no
/// `config` row no file stands in for it, and a lone overlay is `session`.
#[test]
fn only_the_recorded_config_row_is_a_global() {
    let plan = InstructionsPlan::of(SEATS.as_bytes(), Some(Path::new(OVERLAY)));
    assert_eq!(plan.files(), [PathBuf::from(OVERLAY)]);
    let view = instructions(
        SEATS,
        Some(OVERLAY),
        &[
            (OVERLAY, "[prompt]\ninstructions = local\n"),
            (CURRENT, "[prompt]\ninstructions = current\n"),
        ],
    );
    assert_eq!(
        in_force(&view).0,
        Some(won(ConfigSource::Session, OVERLAY, "local"))
    );
}

/// [I7a] Oracle (lead ruling): config.rs `text_entries`, "The frozen reader
/// drops a final line with no closing newline, except a block closer, which
/// the identity reader does see at EOF". Such a row neither declares nor
/// masks; a block closed at EOF is read.
#[test]
fn a_last_line_with_no_newline_is_not_read_but_a_block_closer_is() {
    let meta = format!("{SEATS}config={RECORDED}\n");
    let global = "[prompt]\ninstructions = from global\n";
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, global),
            (OVERLAY, "[prompt]\ninstructions = \"\""),
        ],
    );
    assert_eq!(
        in_force(&view).0,
        Some(won(ConfigSource::Global, RECORDED, "from global"))
    );
    let view = instructions(
        &meta,
        Some(OVERLAY),
        &[
            (RECORDED, global),
            (
                OVERLAY,
                "[prompt]\ninstructions = \"\"\"\nfrom session\n\"\"\"",
            ),
        ],
    );
    assert_eq!(
        in_force(&view).0,
        Some(won(ConfigSource::Session, OVERLAY, "from session"))
    );
}

/// [I6] app.md:1341-1342: an unreadable or torn config file is a named gap
/// for the whole tab.
#[test]
fn an_unreadable_or_torn_config_file_is_a_named_gap() {
    let torn = "[prompt]\ninstructions = \"\"\"\nunclosed\n".to_owned();
    let why = prompt_text(Path::new(RECORDED), Ok(Some(torn))).expect_err("torn");
    assert!(why.contains(RECORDED), "{why}");
    let why = prompt_text(Path::new(OVERLAY), Err("permission denied".to_owned()))
        .expect_err("unreadable");
    assert!(
        why.contains(OVERLAY) && why.contains("permission denied"),
        "{why}"
    );
}
