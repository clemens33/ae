//! cliparse characterisation: frozen behaviour pins for `ae::cli::Request::parse`.
//!
//! The oracle is the baseline's observable result (73c6b309) for each argv: the
//! parsed request, the exit code argv alone decides, and the session directory
//! it names. A refactor of the parse must keep every line byte-identical. Some
//! pins record bug-shaped behaviour on purpose (an unknown `_run` option taken
//! as the directory, `_usage <dir> --hourly <extra>` naming `--hourly`,
//! `_compact-revalidate … --when <l> --keep-history <extra>` naming
//! `--keep-history`): a behaviour change there is a ruling, not a refactor.
//!
//! Knob parsers are pinned structurally against their own `Default`, so a
//! tunable's default can move without touching this file; the `_monitor` sweep
//! is the one parse that reads the clock, and only on its own branch.

use std::path::Path;
use std::time::Duration;

use ae::cli::{Request, serves};
use ae::monitor::Args as MonitorArgs;
use ae::telegram::bridge::{Knobs as TelegramKnobs, Paths};
use ae::time::Timestamp;
use ae::watchdog_daemon::Knobs;

/// One argv and its golden.
type Case = (&'static [&'static str], &'static str);

const TOP: &[Case] = &[
    (&[], "Help exit=Some(0) dir=None"),
    (&["-h"], "Help exit=Some(0) dir=None"),
    (&["--help"], "Help exit=Some(0) dir=None"),
    (&["help"], "Help exit=Some(0) dir=None"),
    (&["help", "list", "--x"], "Help exit=Some(0) dir=None"),
    (&["-h", "anything"], "Help exit=Some(0) dir=None"),
    (&["-V"], "Version exit=Some(0) dir=None"),
    (&["--version"], "Version exit=Some(0) dir=None"),
    (&["version"], "Version exit=Some(0) dir=None"),
    (&["version", "--json"], "Version exit=Some(0) dir=None"),
    (&["-x"], "UsageError(\"-x\") exit=Some(2) dir=None"),
    (
        &["--frobnicate"],
        "UsageError(\"--frobnicate\") exit=Some(2) dir=None",
    ),
    (&["-"], "UsageError(\"-\") exit=Some(2) dir=None"),
    (&["--"], "UsageError(\"--\") exit=Some(2) dir=None"),
    (
        &["--ended"],
        "UsageError(\"--ended\") exit=Some(2) dir=None",
    ),
    (
        &["my-feature"],
        "LaunchCandidate(\"my-feature\") exit=None dir=None",
    ),
    (
        &["my-feature", "--all", "extra"],
        "LaunchCandidate(\"my-feature\") exit=None dir=None",
    ),
    (&[""], "LaunchCandidate(\"\") exit=None dir=None"),
    (&["LIST"], "LaunchCandidate(\"LIST\") exit=None dir=None"),
    (&["_nope"], "LaunchCandidate(\"_nope\") exit=None dir=None"),
    (
        &["_nope", "/d"],
        "LaunchCandidate(\"_nope\") exit=None dir=None",
    ),
    (&["list-"], "LaunchCandidate(\"list-\") exit=None dir=None"),
];

const LIST: &[Case] = &[
    (
        &["list"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: false, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["ls"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: false, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--all"],
        "List(ListArgs { selection: Selection { scope: All, needs_attention: false, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["ls", "--running", "--json"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: false, active_within_secs: None }, json: true }) exit=None dir=None",
    ),
    (
        &["list", "--stopped", "--all"],
        "List(ListArgs { selection: Selection { scope: All, needs_attention: false, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--needs-attn"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: true, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--needs-me"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: true, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--needs"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: true, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--attn"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: true, active_within_secs: None }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--active"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: false, active_within_secs: Some(300) }, json: false }) exit=None dir=None",
    ),
    (
        &["ls", "--busy", "--running", "--running"],
        "List(ListArgs { selection: Selection { scope: Running, needs_attention: false, active_within_secs: Some(300) }, json: false }) exit=None dir=None",
    ),
    (
        &["list", "--json", "--bogus", "--nope"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["list", "word"],
        "UsageError(\"word\") exit=Some(2) dir=None",
    ),
    (&["list", "-a"], "UsageError(\"-a\") exit=Some(2) dir=None"),
    (&["list", "--ended"], "ListEnded exit=None dir=None"),
    (&["ls", "--ended"], "ListEnded exit=None dir=None"),
    (
        &["list", "--ended", "--json"],
        "UsageError(\"--ended cannot be combined with live filters or --json\") exit=Some(2) dir=None",
    ),
    (
        &["list", "--json", "--ended"],
        "UsageError(\"--ended cannot be combined with live filters or --json\") exit=Some(2) dir=None",
    ),
    (
        &["list", "--ended", "--ended"],
        "UsageError(\"--ended cannot be combined with live filters or --json\") exit=Some(2) dir=None",
    ),
    (
        &["list", "--bogus", "--ended"],
        "UsageError(\"--ended cannot be combined with live filters or --json\") exit=Some(2) dir=None",
    ),
    (
        &["list", "word", "--ended"],
        "UsageError(\"--ended cannot be combined with live filters or --json\") exit=Some(2) dir=None",
    ),
];

const FORWARD: &[Case] = &[
    (&["next"], "Next { tail: [] } exit=None dir=None"),
    (
        &["next", "a", "--b"],
        "Next { tail: [\"a\", \"--b\"] } exit=None dir=None",
    ),
    (
        &["jump", "--x"],
        "Next { tail: [\"--x\"] } exit=None dir=None",
    ),
    (
        &["orchestrator"],
        "Orchestrator { tail: [] } exit=None dir=None",
    ),
    (
        &["orchestrator", "--popup", "y"],
        "Orchestrator { tail: [\"--popup\", \"y\"] } exit=None dir=None",
    ),
    (
        &["brief", "s", "--all"],
        "Brief { tail: [\"s\", \"--all\"] } exit=None dir=None",
    ),
    (&["board"], "Board { tail: [] } exit=None dir=None"),
    (
        &["board", "--json"],
        "Board { tail: [\"--json\"] } exit=None dir=None",
    ),
    (
        &["chat", "s", "--follow"],
        "Console { tail: [\"s\", \"--follow\"] } exit=None dir=None",
    ),
    (
        &["console", "--all"],
        "Console { tail: [\"--all\"] } exit=None dir=None",
    ),
    (&["app"], "App { tail: [] } exit=None dir=None"),
    (&["app", "s"], "App { tail: [\"s\"] } exit=None dir=None"),
    (
        &["_launch", "a", "--b"],
        "Launch { tail: [\"a\", \"--b\"] } exit=None dir=None",
    ),
    (&["_launch"], "Launch { tail: [] } exit=None dir=None"),
    (&["doctor"], "Doctor { tail: [] } exit=None dir=None"),
    (
        &["doctor", "--refresh", "all"],
        "Doctor { tail: [\"--refresh\", \"all\"] } exit=None dir=None",
    ),
    (
        &["init", "--x"],
        "Init { tail: [\"--x\"] } exit=None dir=None",
    ),
    (
        &["rename", "old", "new"],
        "Rename { tail: [\"old\", \"new\"] } exit=None dir=None",
    ),
    (
        &["reseat", "s", "a", "--using", "p"],
        "Reseat { tail: [\"s\", \"a\", \"--using\", \"p\"] } exit=None dir=None",
    ),
    (
        &["_auto-reseat"],
        "AutoReseat { tail: [] } exit=None dir=None",
    ),
    (
        &["_auto-reseat", "/d", "slot", "key"],
        "AutoReseat { tail: [\"/d\", \"slot\", \"key\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_auto-effort", "/d", "slot", "ep"],
        "AutoEffort { tail: [\"/d\", \"slot\", \"ep\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_auto-effort"],
        "AutoEffort { tail: [] } exit=None dir=None",
    ),
    (
        &["_check-deps", "--bash-major", "5"],
        "CheckDeps { tail: [\"--bash-major\", \"5\"] } exit=None dir=None",
    ),
    (
        &["_install", "--from", "/b"],
        "Install { tail: [\"--from\", \"/b\"] } exit=None dir=None",
    ),
    (
        &["_autoupgrade"],
        "Autoupgrade { tail: [] } exit=None dir=None",
    ),
    (
        &["_autoupgrade", "x"],
        "Autoupgrade { tail: [\"x\"] } exit=None dir=None",
    ),
    (
        &["_watchdog", "start", "s"],
        "Watchdog { tail: [\"start\", \"s\"] } exit=None dir=None",
    ),
    (
        &["_telegram", "status"],
        "Telegram { tail: [\"status\"] } exit=None dir=None",
    ),
    (
        &["_launch-plan"],
        "LaunchPlan { tail: [] } exit=None dir=None",
    ),
    (
        &["_launch-plan", "--main", "lead"],
        "LaunchPlan { tail: [\"--main\", \"lead\"] } exit=None dir=None",
    ),
    (
        &["_end", "s", "-y"],
        "End { tail: [\"s\", \"-y\"] } exit=None dir=None",
    ),
    (&["_end"], "End { tail: [] } exit=None dir=None"),
    (
        &["_stop", "all"],
        "Stop { tail: [\"all\"] } exit=None dir=None",
    ),
    (
        &["_session-menu", "x"],
        "SessionMenu { tail: [\"x\"] } exit=None dir=None",
    ),
    (
        &["_reader", "--client", "c"],
        "Reader { tail: [\"--client\", \"c\"] } exit=None dir=None",
    ),
    (
        &["_console", "--jump"],
        "ConsoleToggle { tail: [\"--jump\"] } exit=None dir=None",
    ),
    (
        &["_orchestrator-jump", "x"],
        "OrchestratorJump { tail: [\"x\"] } exit=None dir=None",
    ),
    (
        &["_compact", "s", "--yes"],
        "Compact { tail: [\"s\", \"--yes\"] } exit=None dir=None",
    ),
];

const DIR_TAIL: &[Case] = &[
    (&["_say"], "MissingOperand(\"_say\") exit=Some(2) dir=None"),
    (
        &["_say", "/d"],
        "Say { dir: \"/d\", tail: [] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_say", "/d", "hello world", "--x"],
        "Say { dir: \"/d\", tail: [\"hello world\", \"--x\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_peek"],
        "MissingOperand(\"_peek\") exit=Some(2) dir=None",
    ),
    (
        &["_peek", "/d", "lead", "40"],
        "Peek { dir: \"/d\", tail: [\"lead\", \"40\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_agents"],
        "MissingOperand(\"_agents\") exit=Some(2) dir=None",
    ),
    (
        &["_agents", "/d", "--all"],
        "Agents { dir: \"/d\", tail: [\"--all\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_focus"],
        "MissingOperand(\"_focus\") exit=Some(2) dir=None",
    ),
    (
        &["_focus", "/d", "lead"],
        "Focus { dir: \"/d\", tail: [\"lead\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_state"],
        "MissingOperand(\"_state\") exit=Some(2) dir=None",
    ),
    (
        &["_state", "/d", "working", "why"],
        "State { dir: \"/d\", tail: [\"working\", \"why\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_goal"],
        "MissingOperand(\"_goal\") exit=Some(2) dir=None",
    ),
    (
        &["_goal", "/d", "--clear"],
        "Goal { dir: \"/d\", tail: [\"--clear\"] } exit=None dir=Some(\"/d\")",
    ),
    (&["_ask"], "MissingOperand(\"_ask\") exit=Some(2) dir=None"),
    (
        &["_ask", "/d", "t", "q"],
        "Ask { dir: \"/d\", tail: [\"t\", \"q\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_review"],
        "MissingOperand(\"_review\") exit=Some(2) dir=None",
    ),
    (
        &["_review", "/d", "t", "r"],
        "Review { dir: \"/d\", tail: [\"t\", \"r\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_reply"],
        "MissingOperand(\"_reply\") exit=Some(2) dir=None",
    ),
    (
        &["_reply", "/d", "--as", "x", "id", "m"],
        "Reply { dir: \"/d\", tail: [\"--as\", \"x\", \"id\", \"m\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_withdraw"],
        "MissingOperand(\"_withdraw\") exit=Some(2) dir=None",
    ),
    (
        &["_withdraw", "/d", "id", "why"],
        "Withdraw { dir: \"/d\", tail: [\"id\", \"why\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_accept"],
        "MissingOperand(\"_accept\") exit=Some(2) dir=None",
    ),
    (
        &["_accept", "/d", "id", "note"],
        "Accept { dir: \"/d\", tail: [\"id\", \"note\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_send"],
        "MissingOperand(\"_send\") exit=Some(2) dir=None",
    ),
    (
        &["_send", "/d", "t", "m"],
        "Send { dir: \"/d\", tail: [\"t\", \"m\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_relay"],
        "MissingOperand(\"_relay\") exit=Some(2) dir=None",
    ),
    (
        &["_relay", "/d", "s:a", "t"],
        "Relay { dir: \"/d\", tail: [\"s:a\", \"t\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_interrupt"],
        "MissingOperand(\"_interrupt\") exit=Some(2) dir=None",
    ),
    (
        &["_interrupt", "/d", "t"],
        "Interrupt { dir: \"/d\", tail: [\"t\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_memo"],
        "MissingOperand(\"_memo\") exit=Some(2) dir=None",
    ),
    (
        &["_memo", "/d", "add", "--topic", "t"],
        "Memo { dir: \"/d\", tail: [\"add\", \"--topic\", \"t\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_shims-render"],
        "MissingOperand(\"_shims-render\") exit=Some(2) dir=None",
    ),
    (
        &["_shims-render", "/d"],
        "ShimsRender { dir: \"/d\", tail: [] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_shims-render", "/d", "x"],
        "ShimsRender { dir: \"/d\", tail: [\"x\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_meta-init"],
        "MissingOperand(\"_meta-init\") exit=Some(2) dir=None",
    ),
    (
        &["_meta-init", "/d", "--base", "/b"],
        "MetaInit { dir: \"/d\", tail: [\"--base\", \"/b\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_spawn"],
        "MissingOperand(\"_spawn\") exit=Some(2) dir=None",
    ),
    (
        &["_spawn", "/d", "n", "--using", "p", "--", "go"],
        "Spawn { dir: \"/d\", tail: [\"n\", \"--using\", \"p\", \"--\", \"go\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_relaunch"],
        "MissingOperand(\"_relaunch\") exit=Some(2) dir=None",
    ),
    (
        &["_relaunch", "/d", "a"],
        "Relaunch { dir: \"/d\", tail: [\"a\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_retire"],
        "MissingOperand(\"_retire\") exit=Some(2) dir=None",
    ),
    (
        &["_retire", "/d", "%3"],
        "Retire { dir: \"/d\", tail: [\"%3\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_roster"],
        "MissingOperand(\"_roster\") exit=Some(2) dir=None",
    ),
    (
        &["_roster", "/d", "list"],
        "Roster { dir: \"/d\", tail: [\"list\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_manifest-render"],
        "MissingOperand(\"_manifest-render\") exit=Some(2) dir=None",
    ),
    (
        &["_manifest-render", "/d", "x"],
        "ManifestRender { dir: \"/d\", tail: [\"x\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_context"],
        "MissingOperand(\"_context\") exit=Some(2) dir=None",
    ),
    (
        &["_context", "/d", "slot"],
        "Context { dir: \"/d\", tail: [\"slot\"] } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_say", "--help"],
        "Say { dir: \"--help\", tail: [] } exit=None dir=Some(\"--help\")",
    ),
    (
        &["_say", ""],
        "Say { dir: \"\", tail: [] } exit=None dir=Some(\"\")",
    ),
];

const FIXED: &[Case] = &[
    (
        &["_quota"],
        "MissingOperand(\"_quota\") exit=Some(2) dir=None",
    ),
    (
        &["_quota", "/d"],
        "Quota { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_quota", "/d", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_usage"],
        "MissingOperand(\"_usage\") exit=Some(2) dir=None",
    ),
    (
        &["_usage", "/d"],
        "Usage { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_usage", "/d", "--hourly"],
        "UsageHourly { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_usage", "/d", "--daily"],
        "UsageError(\"--daily\") exit=Some(2) dir=None",
    ),
    (
        &["_usage", "/d", "--hourly", "x"],
        "UsageError(\"--hourly\") exit=Some(2) dir=None",
    ),
    (
        &["_usage", "--hourly"],
        "Usage { dir: \"--hourly\" } exit=None dir=Some(\"--hourly\")",
    ),
    (
        &["_capture-sid"],
        "MissingOperand(\"_capture-sid\") exit=Some(2) dir=None",
    ),
    (
        &["_capture-sid", "/d", "s"],
        "MissingOperand(\"_capture-sid\") exit=Some(2) dir=None",
    ),
    (
        &["_capture-sid", "/d", "s", "%1"],
        "CaptureSid { dir: \"/d\", slot: \"s\", pane: \"%1\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_capture-sid", "/d", "s", "%1", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_register-sid"],
        "MissingOperand(\"_register-sid\") exit=Some(2) dir=None",
    ),
    (
        &["_register-sid", "/d"],
        "MissingOperand(\"_register-sid\") exit=Some(2) dir=None",
    ),
    (
        &["_register-sid", "/d", "s"],
        "RegisterSid { dir: \"/d\", slot: \"s\", id: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_register-sid", "/d", "s", "id"],
        "RegisterSid { dir: \"/d\", slot: \"s\", id: Some(\"id\") } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_register-sid", "/d", "s", "id", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_events-tail"],
        "MissingOperand(\"_events-tail\") exit=Some(2) dir=None",
    ),
    (
        &["_events-tail", "/d"],
        "EventsTail { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_events-tail", "/d", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-preview"],
        "MissingOperand(\"_archive-preview\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-preview", "/d"],
        "ArchivePreview { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_archive-preview", "/d", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-publish"],
        "MissingOperand(\"_archive-publish\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-publish", "/d", "o", "r", "p", "w"],
        "MissingOperand(\"_archive-publish\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-publish", "/d", "o", "r", "p", "w", "t"],
        "ArchivePublish { dir: \"/d\", push_outcome: \"o\", push_ref: \"r\", preserved: \"p\", workdir: \"w\", archived_at: \"t\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_archive-publish", "/d", "o", "r", "p", "w", "t", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-from-preflight"],
        "MissingOperand(\"_archive-from-preflight\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-from-preflight", "/r"],
        "MissingOperand(\"_archive-from-preflight\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-from-preflight", "/r", "u"],
        "ArchiveFromPreflight { root: \"/r\", raw_uuid: \"u\" } exit=None dir=None",
    ),
    (
        &["_archive-from-preflight", "/r", "u", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-purge"],
        "MissingOperand(\"_archive-purge\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-purge", "/d", "a", "s"],
        "MissingOperand(\"_archive-purge\") exit=Some(2) dir=None",
    ),
    (
        &["_archive-purge", "/d", "a", "s", "-"],
        "ArchivePurge { dir: \"/d\", aid: \"a\", source_session: \"s\", parent_id: \"-\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_archive-purge", "/d", "a", "s", "-", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_end-local-teardown"],
        "MissingOperand(\"_end-local-teardown\") exit=Some(2) dir=None",
    ),
    (
        &["_end-local-teardown", "/d"],
        "EndLocalTeardown { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_end-local-teardown", "/d", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_end-nonlocal-teardown"],
        "MissingOperand(\"_end-nonlocal-teardown\") exit=Some(2) dir=None",
    ),
    (
        &["_end-nonlocal-teardown", "/d"],
        "EndNonlocalTeardown { dir: \"/d\", preserve: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_end-nonlocal-teardown", "/d", "--preserve"],
        "EndNonlocalTeardown { dir: \"/d\", preserve: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_end-nonlocal-teardown", "/d", "--preserve", "x", "y"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_end-nonlocal-teardown", "/d", "x", "--preserve"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_end-nonlocal-teardown", "--preserve"],
        "EndNonlocalTeardown { dir: \"--preserve\", preserve: false } exit=None dir=Some(\"--preserve\")",
    ),
];

const COMPACT: &[Case] = &[
    (
        &["_compact-freeze"],
        "MissingOperand(\"_compact-freeze\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-freeze", "/d"],
        "CompactFreeze { dir: \"/d\", keep_history: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-freeze", "/d", "--keep-history"],
        "CompactFreeze { dir: \"/d\", keep_history: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-freeze", "/d", "--keep-history", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-freeze", "/d", "x", "--keep-history"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate"],
        "MissingOperand(\"_compact-revalidate\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d"],
        "MissingOperand(\"_compact-revalidate\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d", "t"],
        "CompactRevalidate { dir: \"/d\", tuple: \"t\", when: \"before the handover\", keep_history: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--keep-history"],
        "CompactRevalidate { dir: \"/d\", tuple: \"t\", when: \"before the handover\", keep_history: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--when", "after"],
        "CompactRevalidate { dir: \"/d\", tuple: \"t\", when: \"after\", keep_history: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &[
            "_compact-revalidate",
            "/d",
            "t",
            "--when",
            "after",
            "--keep-history",
        ],
        "CompactRevalidate { dir: \"/d\", tuple: \"t\", when: \"after\", keep_history: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--when"],
        "MissingOperand(\"_compact-revalidate\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--when", "L", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &[
            "_compact-revalidate",
            "/d",
            "t",
            "--when",
            "L",
            "--keep-history",
            "x",
        ],
        "UsageError(\"--keep-history\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d", "t", "--keep-history", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &[
            "_compact-revalidate",
            "/d",
            "t",
            "--keep-history",
            "--when",
            "L",
        ],
        "UsageError(\"--when\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-revalidate", "/d", "t", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-archive"],
        "MissingOperand(\"_compact-archive\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-archive", "/d", "t", "at", "o", "r", "p"],
        "MissingOperand(\"_compact-archive\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-archive", "/d", "t", "at", "o", "r", "p", "w"],
        "CompactArchive { dir: \"/d\", tuple: \"t\", archived_at: \"at\", push_outcome: \"o\", push_ref: \"r\", preserved: \"p\", workdir: \"w\", keep_history: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &[
            "_compact-archive",
            "/d",
            "t",
            "at",
            "o",
            "r",
            "p",
            "w",
            "--keep-history",
        ],
        "CompactArchive { dir: \"/d\", tuple: \"t\", archived_at: \"at\", push_outcome: \"o\", push_ref: \"r\", preserved: \"p\", workdir: \"w\", keep_history: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &[
            "_compact-archive",
            "/d",
            "t",
            "at",
            "o",
            "r",
            "p",
            "w",
            "--keep-history",
            "x",
        ],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &[
            "_compact-archive",
            "/d",
            "t",
            "at",
            "o",
            "r",
            "p",
            "w",
            "x",
            "y",
        ],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-teardown"],
        "MissingOperand(\"_compact-teardown\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-teardown", "/d"],
        "MissingOperand(\"_compact-teardown\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-teardown", "/d", "t"],
        "CompactTeardown { dir: \"/d\", tuple: \"t\", keep_history: false } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-teardown", "/d", "t", "--keep-history"],
        "CompactTeardown { dir: \"/d\", tuple: \"t\", keep_history: true } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-teardown", "/d", "t", "--keep-history", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-teardown", "/d", "t", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait"],
        "MissingOperand(\"_compact-wait\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d"],
        "MissingOperand(\"_compact-wait\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref"],
        "CompactWait { dir: \"/d\", reference: \"ref\", timeout_secs: 300 } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-wait", "/d", "ref", "--timeout", "9"],
        "CompactWait { dir: \"/d\", reference: \"ref\", timeout_secs: 9 } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-wait", "/d", "ref", "--timeout", "-1"],
        "UsageError(\"-1\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref", "--timeout", "nine"],
        "UsageError(\"nine\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref", "--timeout"],
        "UsageError(\"--timeout\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref", "--timeout", "9", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-wait", "/d", "ref", "x", "9"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-cancel"],
        "MissingOperand(\"_compact-cancel\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-cancel", "/d"],
        "MissingOperand(\"_compact-cancel\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-cancel", "/d", "ref"],
        "CompactCancel { dir: \"/d\", reference: \"ref\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-cancel", "/d", "ref", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-memo-baseline"],
        "MissingOperand(\"_compact-memo-baseline\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-memo-baseline", "/d"],
        "CompactMemoBaseline { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-memo-baseline", "/d", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-find-outstanding"],
        "MissingOperand(\"_compact-find-outstanding\") exit=Some(2) dir=None",
    ),
    (
        &["_compact-find-outstanding", "/d"],
        "CompactFindOutstanding { dir: \"/d\" } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_compact-find-outstanding", "/d", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
];

const LOOPS: &[Case] = &[
    (
        &["_requests"],
        "MissingOperand(\"_requests\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d"],
        "Requests { dir: \"/d\", mode: Mine, limit: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "mine"],
        "Requests { dir: \"/d\", mode: Mine, limit: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "inbox"],
        "Requests { dir: \"/d\", mode: Inbox, limit: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "all"],
        "Requests { dir: \"/d\", mode: All, limit: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "bogus"],
        "UsageError(\"bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "all", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "--limit", "5"],
        "Requests { dir: \"/d\", mode: Mine, limit: Some(5) } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "all", "--limit", "0"],
        "Requests { dir: \"/d\", mode: All, limit: Some(0) } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "--limit", "5", "inbox"],
        "Requests { dir: \"/d\", mode: Inbox, limit: Some(5) } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_requests", "/d", "--limit"],
        "MissingOperand(\"_requests\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "--limit", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "--limit", "-1"],
        "UsageError(\"-1\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "--limit", "1", "--limit", "2"],
        "UsageError(\"--limit\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "--bogus"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "/d", "-x"],
        "UsageError(\"-x\") exit=Some(2) dir=None",
    ),
    (
        &["_requests", "--limit", "5"],
        "UsageError(\"5\") exit=Some(2) dir=None",
    ),
    (&["_run"], "MissingOperand(\"_run\") exit=Some(2) dir=None"),
    (
        &["_run", "/d"],
        "MissingOperand(\"_run\") exit=Some(2) dir=None",
    ),
    (
        &["_run", "/d", "s"],
        "Run { dir: \"/d\", slot: \"s\", print: false, command_snapshot: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "--print", "/d", "s"],
        "Run { dir: \"/d\", slot: \"s\", print: true, command_snapshot: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "/d", "--print", "s"],
        "Run { dir: \"/d\", slot: \"s\", print: true, command_snapshot: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "/d", "s", "--print", "--print"],
        "Run { dir: \"/d\", slot: \"s\", print: true, command_snapshot: None } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "--command-snapshot", "c md", "/d", "s"],
        "Run { dir: \"/d\", slot: \"s\", print: false, command_snapshot: Some(\"c md\") } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "/d", "s", "--command-snapshot", "c"],
        "Run { dir: \"/d\", slot: \"s\", print: false, command_snapshot: Some(\"c\") } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_run", "--command-snapshot"],
        "MissingOperand(\"_run\") exit=Some(2) dir=None",
    ),
    (
        &["_run", "/d", "s", "--command-snapshot"],
        "MissingOperand(\"_run\") exit=Some(2) dir=None",
    ),
    (
        &[
            "_run",
            "--command-snapshot",
            "a",
            "--command-snapshot",
            "b",
            "/d",
            "s",
        ],
        "UsageError(\"--command-snapshot\") exit=Some(2) dir=None",
    ),
    (
        &["_run", "/d", "s", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_run", "--bogus", "/d"],
        "Run { dir: \"--bogus\", slot: \"/d\", print: false, command_snapshot: None } exit=None dir=Some(\"--bogus\")",
    ),
];

const PROBE: &[Case] = &[
    (
        &["_net-probe"],
        "MissingOperand(\"_net-probe\") exit=Some(2) dir=None",
    ),
    (
        &["_net-probe", "h"],
        "NetProbe { host: \"h\", port: 443 } exit=None dir=None",
    ),
    (
        &["_net-probe", "h", "--port", "80"],
        "NetProbe { host: \"h\", port: 80 } exit=None dir=None",
    ),
    (
        &["_net-probe", "h", "--port"],
        "UsageError(\"--port\") exit=Some(2) dir=None",
    ),
    (
        &["_net-probe", "h", "--port", "99999"],
        "UsageError(\"99999\") exit=Some(2) dir=None",
    ),
    (
        &["_net-probe", "h", "--port", "80", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_net-probe", "h", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_net-probe", "--port", "80"],
        "UsageError(\"80\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor"],
        "MissingOperand(\"_monitor\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep"],
        "MissingOperand(\"_monitor\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "other"],
        "UsageError(\"other\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "other", "/d"],
        "UsageError(\"other\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--now", "5"],
        "Monitor { dir: \"/d\", args: Args { now: 5, quiet_secs: 1200, liveness_sweeps: 36, init: false, dry_run: false, notify: true, format: Text } } exit=None dir=Some(\"/d\")",
    ),
    (
        &[
            "_monitor",
            "sweep",
            "/d",
            "--now",
            "-5",
            "--quiet-secs",
            "6",
            "--liveness-sweeps",
            "7",
        ],
        "Monitor { dir: \"/d\", args: Args { now: -5, quiet_secs: 6, liveness_sweeps: 7, init: false, dry_run: false, notify: true, format: Text } } exit=None dir=Some(\"/d\")",
    ),
    (
        &[
            "_monitor",
            "sweep",
            "/d",
            "--now",
            "1",
            "--format",
            "json",
            "--init",
            "--dry-run",
            "--no-notify",
        ],
        "Monitor { dir: \"/d\", args: Args { now: 1, quiet_secs: 1200, liveness_sweeps: 36, init: true, dry_run: true, notify: false, format: Json } } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_monitor", "sweep", "/d", "--now", "1", "--format", "text"],
        "Monitor { dir: \"/d\", args: Args { now: 1, quiet_secs: 1200, liveness_sweeps: 36, init: false, dry_run: false, notify: true, format: Text } } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_monitor", "sweep", "/d", "--format", "xml"],
        "UsageError(\"xml\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--now"],
        "UsageError(\"--now\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--now", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--quiet-secs", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--liveness-sweeps", "-1"],
        "UsageError(\"-1\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--bogus"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--bogus", "1"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_monitor", "sweep", "/d", "--now", "1", "--now", "2"],
        "Monitor { dir: \"/d\", args: Args { now: 2, quiet_secs: 1200, liveness_sweeps: 36, init: false, dry_run: false, notify: true, format: Text } } exit=None dir=Some(\"/d\")",
    ),
    (
        &["_watchdog-run"],
        "MissingOperand(\"_watchdog-run\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--interval"],
        "UsageError(\"--interval\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--interval", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--bogus"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--bogus", "1"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--max-nudges", "4294967296"],
        "UsageError(\"4294967296\") exit=Some(2) dir=None",
    ),
    (
        &["_watchdog-run", "/d", "--quiet-tries", "-1"],
        "UsageError(\"-1\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run"],
        "MissingOperand(\"_telegram-run\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run", "/h", "--config"],
        "UsageError(\"--config\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run", "/h", "--limit", "x"],
        "UsageError(\"x\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run", "/h", "--long-poll", "-1"],
        "UsageError(\"-1\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run", "/h", "--bogus", "1"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
    (
        &["_telegram-run", "/h", "--once", "--bogus"],
        "UsageError(\"--bogus\") exit=Some(2) dir=None",
    ),
];

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// The golden of one argv: the parsed request, the exit code argv alone
/// decides, and the session directory it names.
fn observe(words: &[&str]) -> String {
    let request = Request::parse(&argv(words));
    format!(
        "{request:?} exit={:?} dir={:?}",
        request.exit_code(),
        request.session_dir()
    )
}

/// Every mismatch of a group, named by argv, in one failure.
fn check(group: &str, cases: &[Case]) {
    let drift: Vec<String> = cases
        .iter()
        .filter_map(|(words, want)| {
            let got = observe(words);
            (got != *want).then(|| format!("{words:?}\n   want {want}\n    got {got}"))
        })
        .collect();
    assert!(drift.is_empty(), "{group} drifted:\n{}", drift.join("\n"));
}

#[test]
fn top_level_help_version_option_and_session_name_pins() {
    check("TOP", TOP);
}

#[test]
fn list_and_ls_filters_ended_and_refusal_pins() {
    check("LIST", LIST);
}

#[test]
fn raw_tail_forwarding_pins() {
    check("FORWARD", FORWARD);
}

#[test]
fn directory_then_tail_helper_pins() {
    check("DIR_TAIL", DIR_TAIL);
}

#[test]
fn fixed_arity_helper_pins() {
    check("FIXED", FIXED);
}

#[test]
fn compact_helper_flag_and_arity_pins() {
    check("COMPACT", COMPACT);
}

#[test]
fn requests_and_run_option_loop_pins() {
    check("LOOPS", LOOPS);
}

#[test]
fn net_probe_monitor_and_daemon_refusal_pins() {
    check("PROBE", PROBE);
}

#[test]
fn watchdog_run_reads_every_knob_and_the_last_repeat_wins() {
    let base = Knobs::default();
    assert_eq!(
        Request::parse(&argv(&["_watchdog-run", "/d"])),
        Request::WatchdogRun {
            dir: "/d".into(),
            knobs: base,
            step: None
        }
    );
    let words = [
        "_watchdog-run",
        "/d",
        "--interval",
        "1",
        "--quota-every-secs",
        "2",
        "--idle-nudge-secs",
        "3",
        "--stale-secs",
        "4",
        "--max-nudges",
        "5",
        "--throttle-alert-cycles",
        "6",
        "--undelivered-max",
        "7",
        "--quiet-beat-ms",
        "8",
        "--quiet-tries",
        "9",
        "--quiet-panes-per-cycle",
        "10",
        "--sweep-secs",
        "11",
        "--sweep-retry-secs",
        "12",
        "--sweep-retry-max",
        "13",
        "--tg-supervise-secs",
        "14",
        "--interval",
        "15",
    ];
    let mut sweep = base.sweep;
    sweep.sweep_secs = 11;
    sweep.retry_secs = 12;
    sweep.retry_max = 13;
    let knobs = Knobs {
        interval_secs: 15,
        quota_every_secs: 2,
        idle_nudge_secs: 3,
        stale_secs: 4,
        max_nudges: 5,
        throttle_alert_cycles: 6,
        undelivered_max: 7,
        quiet_beat_ms: 8,
        quiet_tries: 9,
        quiet_panes_per_cycle: 10,
        sweep,
        tg_supervise_secs: 14,
        ..base
    };
    let request = Request::parse(&argv(&words));
    assert_eq!(
        request,
        Request::WatchdogRun {
            dir: "/d".into(),
            knobs,
            step: None
        }
    );
    assert_eq!(
        (request.exit_code(), request.session_dir()),
        (None, Some(Path::new("/d")))
    );
}

#[test]
fn telegram_run_derives_paths_reads_every_flag_and_once_takes_no_value() {
    let paths = Paths::under("/h/.ae");
    let knobs = TelegramKnobs::default();
    assert_eq!(
        Request::parse(&argv(&["_telegram-run", "/h/.ae"])),
        Request::TelegramRun {
            paths: paths.clone(),
            knobs
        }
    );
    let words = [
        "_telegram-run",
        "/h/.ae",
        "--once",
        "--config",
        "/c",
        "--home",
        "/H",
        "--limit",
        "5",
        "--long-poll",
        "3",
        "--outbound-interval",
        "4",
        "--limit",
        "6",
    ];
    let mut inbound = knobs.inbound;
    inbound.limit = 6;
    inbound.long_poll = Duration::from_secs(3);
    let want = Request::TelegramRun {
        paths: Paths {
            config: "/c".into(),
            home: "/H".into(),
            ..paths
        },
        knobs: TelegramKnobs {
            inbound,
            outbound_interval: Duration::from_secs(4),
            once: true,
            ..knobs
        },
    };
    let request = Request::parse(&argv(&words));
    assert_eq!(request, want);
    assert_eq!((request.exit_code(), request.session_dir()), (None, None));
}

#[test]
fn a_sweep_without_now_stamps_the_clock_read_during_its_parse() {
    let before = Timestamp::now().epoch();
    let request = Request::parse(&argv(&["_monitor", "sweep", "/d", "--quiet-secs", "6"]));
    let after = Timestamp::now().epoch();
    let Request::Monitor { dir, args } = request else {
        panic!("a sweep with a directory is the sweep: {request:?}");
    };
    assert_eq!(dir, Path::new("/d"));
    assert!(
        (before..=after).contains(&args.now),
        "{before} <= {} <= {after}",
        args.now
    );
    let want = MonitorArgs {
        now: args.now,
        quiet_secs: 6,
        ..MonitorArgs::default()
    };
    assert_eq!(args, want);
}

/// `ae::cli::serves` is the internal-namespace gate: every entry spelling is
/// served, a bare unknown word is not, and an option-shaped word is.
#[test]
fn serves_answers_for_every_spelling_the_dispatch_knows() {
    let served: Vec<&str> = [TOP, LIST, FORWARD, DIR_TAIL, FIXED, COMPACT, LOOPS, PROBE]
        .iter()
        .flat_map(|cases| cases.iter())
        .filter_map(|(words, _)| words.first().copied())
        .filter(|word| !matches!(*word, "" | "LIST" | "_nope" | "list-" | "my-feature"))
        .collect();
    for word in &served {
        assert!(serves(word), "{word} is an entry of the core");
    }
    for word in ["", "LIST", "_nope", "list-", "my-feature", "_say_", "say"] {
        assert!(!serves(word), "{word:?} is a session name, not an entry");
    }
}
