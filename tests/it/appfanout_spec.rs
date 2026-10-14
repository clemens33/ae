//! Frozen acceptance for slice appfanout (tmux fan-out cut).
//!
//! Oracle: `.local/brief-appfanout.md` R1–R4. These tests derive from that
//! requirement, never from current code. Phase tags: `phase_a_*` must go GREEN
//! at commit A while `phase_b_*` stay RED; `phase_r4_*` are verdict-equality
//! regression guards and may be GREEN on base.
//!
//! Counting oracle: a PATH `tmux` shim appends tab-separated argv to a log and
//! execs the real tmux. Drive `ae list` / `ae list --json` (`current_world` →
//! `world_read`). No product code.

#![allow(
    clippy::disallowed_methods,
    reason = "acceptance fixtures create and inspect their own isolated session state"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use ae::json::{self, Value};

use super::cli::{OwnedScratch, Runner, ae};
use super::phase2::{run_tmux, tmux_present};

const CONFIG: &str = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n\
    [workspace]\nmain = lead\nlayout = vertical\nchat = off\n";

/// Running ae session whose name is a prefix of [`FANOUT`].
const FAN: &str = "fan";
/// Running ae session; carries the planted branch.
const FANOUT: &str = "fanout";
/// Durable record whose tmux session was killed.
const PARKED: &str = "parked";
/// Live tmux session on the same server with no `AE_SESSION` marker.
const FOREIGN: &str = "foreign";
/// Durable record on a socket that is not a server.
const GHOST: &str = "ghost";
/// Branch planted on [`FANOUT`] via `@ae_branch_name`.
const BRANCH: &str = "featurex";

struct Fleet {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    socket: PathBuf,
    shim_bin: PathBuf,
    log: PathBuf,
    sysctl_log: PathBuf,
    real_path: String,
}

impl Fleet {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("fanout", tag);
        assert!(
            tmux_present(&scratch),
            "setup: tmux is not runnable here, so the fan-out pins cannot be proven"
        );
        let home = scratch.join("state");
        let project = scratch.join("project");
        let socket = scratch.join("sock");
        scratch.add_tmux_server(socket.clone());
        assert!(fs::create_dir_all(&home).is_ok(), "setup: state root");
        assert!(fs::create_dir_all(&project).is_ok(), "setup: project");
        assert!(
            fs::write(home.join("config"), CONFIG).is_ok(),
            "setup: idle configuration"
        );
        let real_path = std::env::var("PATH").unwrap_or_default();
        let shim_bin = scratch.join("shim-bin");
        assert!(fs::create_dir_all(&shim_bin).is_ok(), "setup: shim dir");
        let log = scratch.join("tmux.log");
        let sysctl_log = scratch.join("sysctl.log");
        write_tmux_shim(&shim_bin.join("tmux"));
        write_sysctl_shim(&shim_bin.join("sysctl"));
        Self {
            scratch,
            home,
            project,
            socket,
            shim_bin,
            log,
            sysctl_log,
            real_path,
        }
    }

    /// N=2 running (`fan`, `fanout`), M=1 stopped (`parked`), 1 non-ae (`foreign`).
    fn with_running_prefix_stopped_foreign(tag: &str) -> Self {
        let fleet = Self::new(tag);
        let project = fleet.project.display().to_string();
        assert!(
            fleet
                .tmux(&["new-session", "-d", "-s", FOREIGN, "-c", &project])
                .0,
            "setup: non-ae session starts the private server"
        );
        fleet.launch(FAN);
        fleet.launch(FANOUT);
        fleet.launch(PARKED);
        assert!(
            fleet.tmux(&["kill-session", "-t", &format!("={PARKED}")]).0,
            "setup: parked session is killed, record kept"
        );
        let (set, set_out) = fleet.tmux(&[
            "set-option",
            "-t",
            &format!("={FANOUT}:"),
            "@ae_branch_name",
            BRANCH,
        ]);
        assert!(
            set,
            "setup: planted branch on fanout ({set_out}); stderr={}",
            fs::read_to_string(fleet.scratch.join("stderr")).unwrap_or_default()
        );
        fleet
    }

    fn command(&self, args: &[&str]) -> Runner {
        let mut command = ae();
        command
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", self.scratch.path())
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.home.join("config"))
            .env("TMUX_TMPDIR", self.scratch.path())
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("AE_NO_AUTOSTART", "1")
            .env("AE_SHIM_LOG", &self.log)
            .env("AE_SYSCTL_LOG", &self.sysctl_log)
            .env("AE_SHIM_REAL_PATH", &self.real_path)
            .env(
                "PATH",
                format!("{}:{}", self.shim_bin.display(), self.real_path),
            )
            .current_dir(&self.project)
            .args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .output()
            .unwrap_or_else(|why| panic!("setup: real ae entry: {why}"))
    }

    fn launch(&self, name: &str) {
        success(&self.run(&["--local", name, "--no-attach"]), name);
    }

    fn tmux(&self, words: &[&str]) -> (bool, String) {
        let mut argv = vec!["-S".to_owned(), self.socket.display().to_string()];
        argv.extend(words.iter().map(|word| (*word).to_owned()));
        run_tmux(&argv, &self.scratch)
    }

    fn clear_logs(&self) {
        let _ = fs::write(&self.log, "");
        let _ = fs::write(&self.sysctl_log, "");
    }

    fn list_all_json(&self) -> Output {
        self.clear_logs();
        let out = self.run(&["list", "--all", "--json"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "setup: ae list --all --json must run\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn tmux_log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_else(|why| panic!("setup: tmux log: {why}"))
    }

    fn sysctl_log(&self) -> String {
        fs::read_to_string(&self.sysctl_log)
            .unwrap_or_else(|why| panic!("setup: sysctl log: {why}"))
    }

    fn session_id(&self, name: &str) -> String {
        let (ok, id) = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &format!("={name}"),
            "-F",
            "#{session_id}",
        ]);
        let id = id.lines().next().unwrap_or("").trim().to_owned();
        assert!(
            ok && id.starts_with('$'),
            "setup: session id of {name}: {id}; stderr={}",
            fs::read_to_string(self.scratch.join("stderr")).unwrap_or_default()
        );
        id
    }
}

fn write_executable(path: &Path, body: &str) {
    assert!(
        fs::write(path, body).is_ok(),
        "setup: write {}",
        path.display()
    );
    assert!(
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).is_ok(),
        "setup: chmod {}",
        path.display()
    );
}

/// PATH tmux shim: log argv, then exec real tmux.
///
/// `AE_SHIM_DROP_SESSION` identity contract (keep this so the driver does): a
/// dropped row matches when one whole pipe field, after trim, equals
/// `session_name` or `$id`. A substring of another field is not a match.
/// The drop awk trims a copy of each field and never assigns `$i`, so a kept
/// row is printed as the raw `$0` (byte-identical to tmux). Assigning `$i`
/// would rebuild the record with default OFS space and strip the pipes.
fn write_tmux_shim(path: &Path) {
    write_executable(
        path,
        "#!/bin/sh\n\
         {\n\
         first=1\n\
         for w in \"$@\"; do\n\
         \tif [ \"$first\" = 1 ]; then printf '%s' \"$w\"; first=0\n\
         \telse printf '\\t%s' \"$w\"\n\
         \tfi\n\
         done\n\
         printf '\\n'\n\
         } >> \"$AE_SHIM_LOG\"\n\
         saw_panes=0\n\
         saw_a=0\n\
         for w in \"$@\"; do\n\
         \t[ \"$w\" = \"list-panes\" ] && saw_panes=1\n\
         \t[ \"$w\" = \"-a\" ] && saw_a=1\n\
         done\n\
         if [ \"$saw_panes\" = 1 ] && [ \"$saw_a\" = 1 ]; then\n\
         \tif [ \"${AE_SHIM_FAIL_PANES_A:-}\" = 1 ]; then exit 1; fi\n\
         \tif [ -n \"${AE_SHIM_DROP_SESSION:-}\" ]; then\n\
         \t\tout=$(PATH=\"$AE_SHIM_REAL_PATH\" tmux \"$@\") || exit $?\n\
         \t\tprintf '%s\\n' \"$out\" | awk -v drop=\"$AE_SHIM_DROP_SESSION\" -F '|' '{\n\
         \t\t\tkeep=1\n\
         \t\t\tn=split(drop, ids, \",\")\n\
         \t\t\tfor (i=1; i<=NF; i++) {\n\
         \t\t\t\tf=$i\n\
         \t\t\t\tgsub(/^ +| +$/, \"\", f)\n\
         \t\t\t\tfor (j=1; j<=n; j++) if (f == ids[j]) keep=0\n\
         \t\t\t}\n\
         \t\t\tif (keep) print\n\
         \t\t}'\n\
         \t\texit 0\n\
         \tfi\n\
         fi\n\
         PATH=\"$AE_SHIM_REAL_PATH\" exec tmux \"$@\"\n",
    );
}

fn write_sysctl_shim(path: &Path) {
    write_executable(
        path,
        "#!/bin/sh\n\
         printf '%s\\n' \"$*\" >> \"$AE_SYSCTL_LOG\"\n\
         if [ \"${AE_SHIM_SYSCTL_FAIL:-}\" = 1 ]; then exit 1; fi\n\
         PATH=\"$AE_SHIM_REAL_PATH\" exec sysctl \"$@\"\n",
    );
}

fn success(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup: {what}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn parse_json(out: &Output) -> Value {
    let text = String::from_utf8_lossy(&out.stdout);
    json::parse(text.trim_end()).unwrap_or_else(|why| {
        panic!(
            "setup: one complete list json: {why:?}\nstdout={text}\nstderr={}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn sessions(document: &Value) -> &[Value] {
    match document.get("sessions") {
        Some(Value::Arr(entries)) => entries,
        other => panic!("setup: sessions must be an array, got {other:?}"),
    }
}

fn session_named<'a>(document: &'a Value, name: &str) -> Option<&'a Value> {
    sessions(document)
        .iter()
        .find(|entry| entry.get_str("name") == Some(name))
}

fn status_of(document: &Value, name: &str) -> String {
    session_named(document, name)
        .and_then(|entry| entry.get_str("status"))
        .unwrap_or_else(|| panic!("no session named {name} in the listing"))
        .to_owned()
}

fn agents_of<'a>(document: &'a Value, name: &str) -> &'a [Value] {
    match session_named(document, name).and_then(|entry| entry.get("agents")) {
        Some(Value::Arr(agents)) => agents,
        _ => panic!("session {name} has no agents array"),
    }
}

/// Running + no pane listing: the meta roster is still present, `alive` is JSON null.
fn panes_not_listed(agents: &[Value]) -> bool {
    !agents.is_empty()
        && agents
            .iter()
            .all(|agent| agent.get("alive") == Some(&Value::Null))
}

/// At least one roster seat whose pane was listed alive.
fn panes_listed_alive(agents: &[Value]) -> bool {
    agents
        .iter()
        .any(|agent| agent.get("alive") == Some(&Value::Bool(true)))
}

fn calls(log: &str, subcommand: &str) -> Vec<Vec<String>> {
    log.lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split('\t').map(ToOwned::to_owned).collect::<Vec<_>>())
        .filter(|argv| argv.iter().any(|word| word == subcommand))
        .collect()
}

fn server_key(argv: &[String]) -> String {
    let mut words = argv.iter();
    while let Some(word) = words.next() {
        if word == "-S"
            && let Some(path) = words.next()
        {
            return format!("-S {path}");
        }
        if word == "-L"
            && let Some(name) = words.next()
        {
            return format!("-L {name}");
        }
    }
    "ambient".to_owned()
}

fn counts_by_server(log: &str, subcommand: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for argv in calls(log, subcommand) {
        *counts.entry(server_key(&argv)).or_insert(0) += 1;
    }
    counts
}

fn socket_count(log: &str, subcommand: &str, socket: &Path) -> usize {
    let key = format!("-S {}", socket.display());
    counts_by_server(log, subcommand)
        .get(&key)
        .copied()
        .unwrap_or(0)
}

/// `-t` targets of `show-environment` on `socket`, in log order.
fn show_environment_targets(log: &str, socket: &Path) -> Vec<String> {
    let key = format!("-S {}", socket.display());
    calls(log, "show-environment")
        .into_iter()
        .filter(|argv| server_key(argv) == key)
        .filter_map(|argv| {
            let mut words = argv.iter();
            while let Some(word) = words.next() {
                if word == "-t" {
                    return words.next().cloned();
                }
            }
            None
        })
        .collect()
}

fn show_options_branch(log: &str) -> usize {
    calls(log, "show-options")
        .iter()
        .filter(|argv| argv.iter().any(|word| word == "@ae_branch_name"))
        .count()
}

fn list_panes_all(log: &str) -> usize {
    calls(log, "list-panes")
        .iter()
        .filter(|argv| argv.iter().any(|word| word == "-a"))
        .count()
}

fn list_panes_targeted(log: &str) -> usize {
    calls(log, "list-panes")
        .iter()
        .filter(|argv| !argv.iter().any(|word| word == "-a"))
        .count()
}

fn sysctl_lines(log: &str) -> usize {
    log.lines().filter(|line| !line.is_empty()).count()
}

fn inventory_complete(document: &Value) -> bool {
    matches!(document.get("inventory_complete"), Some(Value::Bool(true)))
}

// ---------------------------------------------------------------------------
// Phase A (R1 + R2). GREEN at commit A; B pins stay RED.
// ---------------------------------------------------------------------------

#[test]
fn phase_a_one_list_sessions_per_entitled_server() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("a-ls");
    let _out = fleet.list_all_json();
    let log = fleet.tmux_log();
    let by_server = counts_by_server(&log, "list-sessions");
    assert!(
        !by_server.is_empty(),
        "setup: world_read must ask list-sessions"
    );
    for (server, n) in &by_server {
        assert_eq!(
            *n, 1,
            "R1: list-sessions for {server} was {n}, want 1 per entitled server per world_read\n{log}"
        );
    }
    assert_eq!(
        socket_count(&log, "list-sessions", &fleet.socket),
        1,
        "R1: the fixture socket is entitled and must be enumerated once\n{log}"
    );
}

/// Phase A: one enumerate reads show-environment once per session that
/// server lists, including foreign (ownership proof it is not ae). A
/// `=name` target is never read twice.
#[test]
fn phase_a_show_environment_at_most_the_running_sessions() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("a-env");
    let _out = fleet.list_all_json();
    let log = fleet.tmux_log();
    let targets = show_environment_targets(&log, &fleet.socket);
    assert_eq!(
        targets.len(),
        3,
        "R1: on the fixture socket, show-environment equals the sessions that server lists (foreign, fan, fanout); was {}\n{log}",
        targets.len()
    );
    let mut seen = BTreeSet::new();
    for target in &targets {
        assert!(
            seen.insert(target.clone()),
            "R1: no =name target is read twice; {target} appeared more than once in {targets:?}\n{log}"
        );
    }
}

/// Cross-process guard only: each `ae list` is a fresh process, so this
/// cannot see an in-process cache across two `world_read` calls. In-process
/// no-cache is navigator review of the app loader path.
#[test]
fn phase_a_a_second_list_asks_tmux_again() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("a-again");
    let _first_out = fleet.list_all_json();
    let first = fleet.tmux_log();
    let first_n = socket_count(&first, "list-sessions", &fleet.socket);
    assert_eq!(
        first_n, 1,
        "R1: first ae list list-sessions on the fixture socket was {first_n}, want 1\n{first}"
    );
    let _second_out = fleet.list_all_json();
    let second = fleet.tmux_log();
    let second_n = socket_count(&second, "list-sessions", &fleet.socket);
    assert_eq!(
        second_n, 1,
        "R1: a later ae list must ask tmux again (nothing cached across reads); list-sessions was {second_n}, want 1\n{second}"
    );
}

#[test]
fn phase_a_failed_enumeration_stays_failed() {
    let fleet = Fleet::new("a-fail");
    let dead = fleet.scratch.join("missing.sock");
    let dir = fleet.home.join("sessions").join(GHOST);
    assert!(fs::create_dir_all(&dir).is_ok(), "setup: ghost dir");
    assert!(
        fs::write(
            dir.join("meta"),
            format!(
                "mode=local\nmeta_version={}\nsession={GHOST}\nseat.main=lead\nprofile.main=idle\ntmux_server_kind=socket\ntmux_server={}\n",
                ae::migrate::CURRENT,
                dead.display()
            )
        )
        .is_ok(),
        "setup: ghost meta"
    );
    let out = fleet.list_all_json();
    let document = parse_json(&out);
    let log = fleet.tmux_log();
    let n = socket_count(&log, "list-sessions", &dead);
    assert_eq!(
        n, 1,
        "R1: a failed enumeration is not retried inside one read; list-sessions on the dead socket was {n}, want 1\n{log}"
    );
    assert_eq!(
        status_of(&document, GHOST),
        "unknown",
        "R1: a failed enumeration leaves its candidates Unknown"
    );
    assert!(
        !inventory_complete(&document),
        "R1: the failed server stays in incomplete"
    );
}

#[test]
fn phase_a_boot_time_override_skips_sysctl() {
    // R2 in-process memo is outside this frozen spec (waiver: unreachable
    // from tests/it without a new door; `std::env::set_var` is unsafe and
    // the crate forbids unsafe_code). Driver writes a unit test beside
    // src/doors.rs (proven value reused, None not stored, override read
    // every call); navigator reviews that test against R2. This pin is the
    // reachable half: AE_TEST_BOOT_TIME is read every call and never
    // memoized into a sysctl skip.
    let fleet = Fleet::new("a-boot-ov");
    fleet.clear_logs();
    let out = fleet
        .command(&["list", "--all", "--json"])
        .env("AE_TEST_BOOT_TIME", "1")
        .output()
        .unwrap_or_else(|why| panic!("setup: ae list: {why}"));
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup: list with AE_TEST_BOOT_TIME\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let n = sysctl_lines(&fleet.sysctl_log());
    assert_eq!(
        n, 0,
        "R2: AE_TEST_BOOT_TIME is read every call and never memoized, so sysctl is not consulted; ran {n}"
    );
}

// ---------------------------------------------------------------------------
// Phase B (R3). Stay RED at commit A.
// ---------------------------------------------------------------------------

#[test]
fn phase_b_one_list_panes_a_per_server_with_running_sessions() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("b-panes");
    let _out = fleet.list_all_json();
    let log = fleet.tmux_log();
    assert_eq!(
        list_panes_all(&log),
        1,
        "R3: pane facts come from ONE list-panes -a per server with running sessions; -a count was {}\n{log}",
        list_panes_all(&log)
    );
    assert_eq!(
        list_panes_targeted(&log),
        0,
        "R3: no per-session list-panes; targeted count was {}\n{log}",
        list_panes_targeted(&log)
    );
}

#[test]
fn phase_b_no_show_options_for_the_branch() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("b-branch");
    let _out = fleet.list_all_json();
    let log = fleet.tmux_log();
    let n = show_options_branch(&log);
    assert_eq!(
        n, 0,
        "R3: the branch comes from a format already being run, so show-options @ae_branch_name was {n}, want 0\n{log}"
    );
}

#[test]
fn phase_b_failed_list_panes_a_reads_none_for_every_running_session() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("b-fail-a");
    fleet.clear_logs();
    let out = fleet
        .command(&["list", "--all", "--json"])
        .env("AE_SHIM_FAIL_PANES_A", "1")
        .output()
        .unwrap_or_else(|why| panic!("setup: ae list: {why}"));
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup: list still succeeds when list-panes -a fails\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let document = parse_json(&out);
    for name in [FAN, FANOUT] {
        let agents = agents_of(&document, name);
        assert!(
            panes_not_listed(agents),
            "R3: a failed list-panes -a reads None for every running session; {name} agents were {agents:?}"
        );
    }
}

/// Zero rows for one running session read as not listed. The PATH shim drops
/// rows whose identity is one whole pipe field after trim (`session_name` or
/// `$id`); keep that contract so a format cell that merely contains the name
/// as a substring is not dropped.
#[test]
fn phase_b_zero_pane_rows_read_as_not_listed() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("b-zero");
    let id = fleet.session_id(FANOUT);
    fleet.clear_logs();
    let drop = format!("{FANOUT},{id}");
    let out = fleet
        .command(&["list", "--all", "--json"])
        .env("AE_SHIM_DROP_SESSION", &drop)
        .output()
        .unwrap_or_else(|why| panic!("setup: ae list: {why}"));
    assert_eq!(
        out.status.code(),
        Some(0),
        "setup: list still succeeds when one session has zero pane rows\nstderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let document = parse_json(&out);
    let fanout = agents_of(&document, FANOUT);
    assert!(
        panes_not_listed(fanout),
        "R3: a running session with ZERO rows reads as not listed (None), same as a failed per-session list-panes; fanout agents were {fanout:?}"
    );
    let fan = agents_of(&document, FAN);
    assert!(
        panes_listed_alive(fan),
        "R3: zero rows for fanout must not wipe fan; fan agents were {fan:?}"
    );
}

// ---------------------------------------------------------------------------
// R4 — verdicts from the fixture, never from code. May be GREEN on base.
// ---------------------------------------------------------------------------

#[test]
fn phase_r4_verdicts_and_branch_match_the_fixture() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("r4-verdict");
    let json_out = fleet.list_all_json();
    let document = parse_json(&json_out);
    assert_eq!(status_of(&document, FAN), "running");
    assert_eq!(status_of(&document, FANOUT), "running");
    assert_eq!(status_of(&document, PARKED), "stopped");
    assert!(
        inventory_complete(&document),
        "the live fixture server answered, so the listing is complete"
    );
    let fanout = session_named(&document, FANOUT).expect("fanout is listed");
    assert_eq!(
        fanout.get_str("branch"),
        Some(BRANCH),
        "planted @ae_branch_name is the branch shown"
    );
    let text = fleet.run(&["list", "--all"]);
    success(&text, "ae list --all");
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains(&format!("git:{BRANCH}")),
        "human list shows the planted branch: {stdout}"
    );
}

#[test]
fn phase_r4_prefix_name_is_not_cross_attributed() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("r4-prefix");
    let document = parse_json(&fleet.list_all_json());
    assert_eq!(status_of(&document, FAN), "running");
    assert_eq!(status_of(&document, FANOUT), "running");
    let names: Vec<&str> = sessions(&document)
        .iter()
        .filter_map(|entry| entry.get_str("name"))
        .collect();
    assert_eq!(
        names.iter().filter(|name| **name == FAN).count(),
        1,
        "fan is one row, not merged into fanout: {names:?}"
    );
    assert_eq!(
        names.iter().filter(|name| **name == FANOUT).count(),
        1,
        "fanout is one row, not merged into fan: {names:?}"
    );
    assert!(
        !agents_of(&document, FAN).is_empty(),
        "fan keeps its own pane facts"
    );
    assert!(
        !agents_of(&document, FANOUT).is_empty(),
        "fanout keeps its own pane facts"
    );
}

#[test]
fn phase_r4_non_ae_session_is_absent() {
    let fleet = Fleet::with_running_prefix_stopped_foreign("r4-foreign");
    let document = parse_json(&fleet.list_all_json());
    assert!(
        session_named(&document, FOREIGN).is_none(),
        "a tmux session without AE_SESSION is not an ae listing row"
    );
}
