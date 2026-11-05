//! #296 follow-up 1. Oracle: final owner R1b/R2, not the current resolver.
//! Real launch + admitted spawn establish identity before each stopped resume.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent acceptance fixtures own their files, sockets and fake tools"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use super::cli::{OwnedScratch, Runner, ae, helper};
use super::phase2::{run_tmux, tmux_present};

// Only the external harness is substituted. Its process name, terminal input
// and exec/account receipts are real; no ae resolver or lifecycle is mocked.
const HARNESS: &str = r#"use strict;
use warnings;
use Cwd qw(abs_path);
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw'); binmode(STDOUT, ':raw'); $| = 1;
open(my $log, '>>', "__RECEIPT__") or die;
my $selected = $ENV{CLAUDE_CONFIG_DIR} // "$ENV{HOME}/.claude";
my $account = abs_path($selected) // $selected;
print $log "EXEC=$^X ACCOUNT=$account RAW_CONFIG_DIR=", ($ENV{CLAUDE_CONFIG_DIR} // "<unset>"), " ARGV=", join(" ", @ARGV), "\n";
close($log);
print "\e[?2004h";
my $border = "\xe2\x94\x80" x 400;
sub draw {
    my ($content) = @_; $content =~ s/[\r\n]/ /g;
    print "\e[H\e[2J\xe2\x9c\xbb Fake \xc2\xb7 done (0s)\r\n$border\r\n\e[1m\xe2\x9d\xaf\e[0m\xc2\xa0$content\r\n$border\r\n  fake-model  ~/x\r\n";
}
draw("");
my $buf = ""; my $pasting = 0; my $ch;
while (1) {
    my $ready = ''; vec($ready, fileno(STDIN), 1) = 1;
    next unless select($ready, undef, undef, 0.05) > 0;
    last unless sysread(STDIN, $ch, 1); $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $pasting = 1; next; }
    if ($buf =~ s/\e\[201~\z//) { $pasting = 0; draw($buf); next; }
    if ($pasting && $ch eq "\r") { chop($buf); $buf .= "\n"; next; }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) { $buf = ""; }
    draw($buf);
}
"#;

#[derive(Clone, Copy, Debug)]
pub(super) enum Shape {
    // Two fixed bare seats avoid fixed-seat snapshot/honor synthesis.
    Flagless,
    // A one-row roster resumed with an explicit seat flag takes the snapshot route.
    ExplicitSolo,
}

pub(super) struct Rig {
    pub(super) scratch: OwnedScratch,
    pub(super) config: PathBuf,
    pub(super) project: PathBuf,
    pub(super) receipt: PathBuf,
    pub(super) home: PathBuf,
    sock: PathBuf,
    shape: Shape,
}

impl Rig {
    pub(super) fn new(tag: &str, shape: Shape) -> Self {
        let mut scratch = OwnedScratch::root("cf", tag);
        assert!(
            tmux_present(&scratch),
            "real tmux is required for this acceptance spec"
        );
        let project = scratch.join("project");
        for path in [
            project.join(".ae"),
            scratch.join("store-a"),
            scratch.join("store-b"),
        ] {
            std::fs::create_dir_all(path).expect("private directories");
        }
        let receipt = scratch.join("execs");
        std::fs::write(
            scratch.join("harness.pl"),
            HARNESS.replace("__RECEIPT__", &receipt.display().to_string()),
        )
        .expect("external harness fixture");
        for side in ["a", "b", "c"] {
            let bin = scratch.join(side);
            std::fs::create_dir_all(&bin).expect("private executable directory");
            for tool in ["claude", "codex"] {
                std::fs::copy("/usr/bin/perl", bin.join(tool))
                    .expect("tool-named interpreter for real ps identity");
            }
        }
        let sock = scratch.join("sock");
        scratch.add_tmux_server(sock.clone());
        let rig = Self {
            home: scratch.join("state"),
            config: scratch.join("config"),
            scratch,
            project,
            receipt,
            sock,
            shape,
        };
        rig.write_config(&rig.identity("b", "store-b", "claude"), "", false);
        rig
    }

    pub(super) fn binary(&self, side: &str, tool: &str) -> PathBuf {
        self.scratch.join(side).join(tool)
    }

    pub(super) fn identity(&self, side: &str, store: &str, tool: &str) -> String {
        format!(
            "[clients]\ndefault = {a} config_home={sa}\ncarry = {b} config_home={sb}\n\n[profiles]\np = \"default {script} --model chosen\"\n",
            a = self.binary("a", "claude").display(),
            b = self.binary(side, tool).display(),
            sa = self.scratch.join("store-a").display(),
            sb = self.scratch.join(store).display(),
            script = self.scratch.join("harness.pl").display(),
        )
    }

    pub(super) fn write_config(&self, identity: &str, extra: &str, auto: bool) {
        let workers = match self.shape {
            Shape::Flagless => "mate",
            Shape::ExplicitSolo => "",
        };
        std::fs::write(&self.config, format!(
            "{identity}\n[profiles]\nidle = \"sleep 600\"\n[client_pairs]\np = carry\n[roster]\nlead = idle\nmate = idle\n[workspace]\nmain = lead\nworkers = {workers}\nlayout = vertical\nwatchdog = false\nchat = off\ntheme = off\nauto_reseat = {}\n{extra}",
            if auto { "on" } else { "off" },
        )).expect("global config");
    }

    pub(super) fn command(&self) -> Runner {
        let mut cmd = ae();
        cmd.env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", &self.config)
            .env("TMUX_TMPDIR", &self.scratch)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("AE_SENDER_OVERRIDE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .current_dir(&self.project);
        cmd
    }

    fn launch_command(&self, resume: bool, marker: bool) -> Runner {
        let mut cmd = self.command();
        cmd.arg(ae::cli::LAUNCH)
            .arg("--home")
            .arg(&self.home)
            .arg("--cwd")
            .arg(&self.project)
            .arg("--global")
            .arg(&self.config)
            .args(["--server-kind", "socket", "--server"])
            .arg(&self.sock)
            .arg("--no-attach");
        if marker {
            cmd.arg("--test-pre-lock-marker");
        }
        cmd.args(["--", "--local", "cfsession"]);
        if resume && matches!(self.shape, Shape::ExplicitSolo) {
            cmd.args(["--lead", "idle"]);
        }
        cmd
    }

    pub(super) fn launch(&self) {
        success(
            &self
                .launch_command(false, false)
                .output()
                .expect("real launch"),
        );
    }

    pub(super) fn resume(&self) -> Output {
        self.launch_command(true, false)
            .output()
            .expect("real resume")
    }

    pub(super) fn dir(&self) -> PathBuf {
        self.home.join("sessions/cfsession")
    }

    pub(super) fn spawn(&self, name: &str, using: &str) -> Output {
        helper(&self.dir().join("spawn"))
            .env("HOME", &self.scratch)
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("AE_SENDER_OVERRIDE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .args([name, "--using", using, "--", "synthetic acceptance task"])
            .output()
            .expect("real spawn helper")
    }

    pub(super) fn reseat(&self, using: &str) -> Output {
        self.command()
            .args(["reseat", "cfsession", "helper", "--using", using])
            .output()
            .expect("real human reseat")
    }

    pub(super) fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|s| (*s).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    pub(super) fn pane(&self, slot: &str) -> Option<String> {
        let (ok, text) = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            "=cfsession",
            "-F",
            "#{pane_id} #{@ae_slot}",
        ]);
        if !ok {
            return None;
        }
        text.lines().find_map(|line| {
            let (pane, found) = line.split_once(' ')?;
            (found == slot).then(|| pane.to_owned())
        })
    }

    pub(super) fn meta(&self) -> String {
        std::fs::read_to_string(self.dir().join("meta")).expect("recorded meta")
    }

    pub(super) fn row(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}=");
        self.meta()
            .lines()
            .find_map(|line| line.strip_prefix(&prefix).map(ToOwned::to_owned))
    }

    pub(super) fn set_row(&self, key: &str, value: Option<&str>) {
        let prefix = format!("{key}=");
        let mut text = self
            .meta()
            .lines()
            .filter(|line| !line.starts_with(&prefix))
            .fold(String::new(), |mut text, line| {
                writeln!(text, "{line}").expect("string");
                text
            });
        if let Some(value) = value {
            writeln!(text, "{key}={value}").expect("string");
        }
        std::fs::write(self.dir().join("meta"), text).expect("private recorded-row mutation");
    }

    pub(super) fn execs(&self) -> String {
        std::fs::read_to_string(&self.receipt).unwrap_or_default()
    }

    pub(super) fn provider_history(&self, store: &str) {
        let sid = self
            .row("harness_session.spawned.0")
            .expect("actual provider session id");
        let key: String = self
            .project
            .to_string_lossy()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        let dir = self.scratch.join(store).join("projects").join(key);
        std::fs::create_dir_all(&dir).expect("external provider transcript directory");
        std::fs::write(
            dir.join(format!("{sid}.jsonl")),
            b"synthetic user\nsynthetic assistant\n",
        )
        .expect("external provider transcript carried by real reseat");
    }

    pub(super) fn wait_exec(&self, side: &str, store: &str) {
        let binary = format!("EXEC={} ", self.binary(side, "claude").display());
        let account = format!(
            "ACCOUNT={} ",
            std::fs::canonicalize(self.scratch.join(store))
                .expect("account")
                .display()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = self.execs();
            if text
                .lines()
                .any(|line| line.contains(&binary) && line.contains(&account))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "missing external receipt {binary}{account}: {text}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn stopped(tag: &str, shape: Shape, labeled: bool) -> Self {
        let rig = Self::new(tag, shape);
        rig.launch();
        success(&rig.spawn("helper", if labeled { "p@carry" } else { "p" }));
        rig.wait_exec(
            if labeled { "b" } else { "a" },
            if labeled { "store-b" } else { "store-a" },
        );
        assert_eq!(
            rig.row("client.spawned.0").as_deref(),
            labeled.then_some("carry")
        );
        let initial = rig
            .row("config_home.spawned.0")
            .expect("spawn recorded account");
        assert_eq!(
            Path::new(&initial),
            std::fs::canonicalize(
                rig.scratch
                    .join(if labeled { "store-b" } else { "store-a" })
            )
            .expect("account")
        );
        // Kill only the fixture session. Retain meta/helpers exactly as resume sees them.
        assert!(
            rig.tmux(&["kill-session", "-t", "=cfsession"]).0,
            "stop private session"
        );
        std::fs::write(&rig.receipt, "").expect("fresh resume receipt");
        rig
    }

    fn stopped_implicit(tag: &str, shape: Shape) -> Self {
        let rig = Self::new(tag, shape);
        std::fs::create_dir(rig.scratch.join(".claude")).expect("implicit provider store");
        let identity = rig
            .identity("b", "store-b", "claude")
            .lines()
            .map(|line| {
                if line.starts_with("carry = ") {
                    format!("carry = {}", rig.binary("b", "claude").display())
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        rig.write_config(&identity, "", false);
        rig.launch();
        success(&rig.spawn("helper", "p@carry"));
        rig.wait_exec("b", ".claude");
        assert_eq!(
            rig.row("config_home.spawned.0"),
            Some(format!(
                "implicit:{}",
                std::fs::canonicalize(rig.scratch.join(".claude"))
                    .expect("implicit account")
                    .display()
            ))
        );
        assert_eq!(
            rig.row("config_home_base.spawned.0"),
            Some(
                std::fs::canonicalize(&rig.scratch)
                    .expect("implicit base")
                    .display()
                    .to_string()
            )
        );
        assert!(rig.tmux(&["kill-session", "-t", "=cfsession"]).0);
        std::fs::write(&rig.receipt, "").expect("fresh resume receipt");
        rig
    }

    fn refuse(&self, diagnostic: &str) {
        let before = state_bytes(&self.dir());
        let out = self
            .launch_command(true, true)
            .output()
            .expect("real preflight refusal probe");
        assert!(
            !out.status.success(),
            "must refuse before rebuilding: {out:?}; execs={}",
            self.execs()
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(diagnostic), "expected {diagnostic}: {out:?}");
        assert!(
            !self
                .home
                .join(ae::session_launch::TEST_PRE_LOCK_MARKER)
                .exists(),
            "refusal must precede the real lifecycle-lock boundary: {out:?}"
        );
        assert_eq!(
            state_bytes(&self.dir()),
            before,
            "preflight refusal preserves state bytes"
        );
        assert!(
            !self.tmux(&["has-session", "-t", "=cfsession"]).0,
            "refusal starts no pane"
        );
        assert!(
            self.execs().is_empty(),
            "refusal executes no tool: {}",
            self.execs()
        );
    }

    fn no_start(&self) {
        let identity = [
            "seat.spawned.0",
            "profile.spawned.0",
            "client.spawned.0",
            "config_home.spawned.0",
            "config_home_base.spawned.0",
            "agent_bin.spawned.0",
        ]
        .map(|key| self.row(key));
        success(&self.resume());
        assert!(
            self.pane("spawned.0").is_none(),
            "unconfigured row has no pane"
        );
        assert!(
            self.execs().is_empty(),
            "unconfigured row has no tool receipt: {}",
            self.execs()
        );
        assert_eq!(
            [
                "seat.spawned.0",
                "profile.spawned.0",
                "client.spawned.0",
                "config_home.spawned.0",
                "config_home_base.spawned.0",
                "agent_bin.spawned.0"
            ]
            .map(|key| self.row(key)),
            identity,
            "unconfigured identity stays whole"
        );
    }
}

pub(super) fn success(out: &Output) {
    assert!(out.status.success(), "{out:?}");
}

// Snapshot state itself: helper symlinks are link bytes, not dozens of copies
// of the immutable core binary outside the session directory.
fn state_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(path)
            .expect("state directory")
            .map(|entry| entry.expect("state entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type().expect("state type");
            if kind.is_dir() {
                out.push((path.clone(), Vec::new()));
                visit(&path, out);
            } else if kind.is_symlink() {
                use std::os::unix::ffi::OsStrExt as _;
                out.push((
                    path.clone(),
                    std::fs::read_link(&path)
                        .expect("helper link")
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                ));
            } else {
                out.push((path.clone(), std::fs::read(&path).expect("state bytes")));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, &mut out);
    out
}

macro_rules! both_routes {
    ($flagless:ident, $explicit:ident, $body:ident) => {
        #[test]
        fn $flagless() {
            $body(Shape::Flagless);
        }
        #[test]
        fn $explicit() {
            $body(Shape::ExplicitSolo);
        }
    };
}

fn changed_adapter(shape: Shape) {
    let rig = Rig::stopped("adapter", shape, true);
    rig.write_config(&rig.identity("b", "store-b", "codex"), "", false);
    rig.refuse("adapter");
}
both_routes!(
    flagless_changed_adapter_refuses,
    explicit_changed_adapter_refuses,
    changed_adapter
);

fn changed_store(shape: Shape) {
    let rig = Rig::stopped("store", shape, true);
    rig.write_config(&rig.identity("b", "store-a", "claude"), "", false);
    rig.refuse("store");
}
both_routes!(
    flagless_changed_store_refuses,
    explicit_changed_store_refuses,
    changed_store
);

fn both_adapters_change(shape: Shape) {
    let rig = Rig::stopped("both-tool", shape, true);
    let identity = rig.identity("b", "store-b", "codex").replace(
        &rig.binary("a", "claude").display().to_string(),
        &rig.binary("a", "codex").display().to_string(),
    );
    rig.write_config(&identity, "", false);
    rig.refuse("recorded");
}
both_routes!(
    flagless_recorded_adapter_binds,
    explicit_recorded_adapter_binds,
    both_adapters_change
);

fn updated_global_binary(shape: Shape) {
    let rig = Rig::stopped("update", shape, true);
    let carried = [
        rig.row("client.spawned.0"),
        rig.row("config_home.spawned.0"),
        rig.row("config_home_base.spawned.0"),
    ];
    rig.write_config(&rig.identity("c", "store-b", "claude"), "", false);
    // Admission policy may be retired: only the original spawn needs approval.
    let config = std::fs::read_to_string(&rig.config)
        .expect("config")
        .replace("p = carry\n", "");
    std::fs::write(&rig.config, config).expect("retired admission");
    success(&rig.resume());
    rig.wait_exec("c", "store-b");
    assert!(
        !rig.execs()
            .contains(&format!("EXEC={} ", rig.binary("b", "claude").display())),
        "no persisted binary pin"
    );
    assert_eq!(
        [
            rig.row("client.spawned.0"),
            rig.row("config_home.spawned.0"),
            rig.row("config_home_base.spawned.0")
        ],
        carried
    );
}
both_routes!(
    flagless_global_binary_update_and_retired_pair,
    explicit_global_binary_update_and_retired_pair,
    updated_global_binary
);

fn ignored_local_binary(shape: Shape) {
    let rig = Rig::stopped("shadow", shape, true);
    std::fs::write(
        rig.project.join(".ae/config"),
        format!(
            "[clients]\ncarry = {} config_home={}\n",
            rig.binary("c", "claude").display(),
            rig.scratch.join("store-b").display()
        ),
    )
    .expect("local executable shadow");
    success(&rig.resume());
    rig.wait_exec("b", "store-b");
    assert!(
        !rig.execs()
            .contains(&format!("EXEC={} ", rig.binary("c", "claude").display())),
        "local executable must not run: {}",
        rig.execs()
    );
}
both_routes!(
    flagless_ignores_local_binary,
    explicit_ignores_local_binary,
    ignored_local_binary
);

fn missing_profile(shape: Shape) {
    let rig = Rig::stopped("no-profile", shape, true);
    let identity = rig.identity("b", "store-b", "claude");
    std::fs::write(rig.project.join(".ae/config"), &identity).expect("local-only profile");
    rig.write_config(
        identity.split("[profiles]").next().expect("client section"),
        "",
        false,
    );
    rig.no_start();
}
both_routes!(
    flagless_global_profile_absent_is_kept,
    explicit_global_profile_absent_is_kept,
    missing_profile
);

fn missing_client(shape: Shape) {
    let rig = Rig::stopped("no-client", shape, true);
    let identity = rig.identity("b", "store-b", "claude");
    std::fs::write(rig.project.join(".ae/config"), &identity).expect("local-only client");
    let identity = identity
        .lines()
        .filter(|line| !line.starts_with("carry = "))
        .fold(String::new(), |mut text, line| {
            writeln!(text, "{line}").expect("string");
            text
        });
    rig.write_config(&identity, "", false);
    rig.refuse("restore");
}
both_routes!(
    flagless_global_client_absent_refuses,
    explicit_global_client_absent_refuses,
    missing_client
);

fn both_absent(shape: Shape) {
    let rig = Rig::stopped("both-absent", shape, true);
    std::fs::write(
        rig.project.join(".ae/config"),
        rig.identity("b", "store-b", "claude"),
    )
    .expect("local-only pair");
    rig.write_config("", "", false);
    rig.refuse("restore");
}
both_routes!(
    flagless_missing_client_precedes_missing_profile,
    explicit_missing_client_precedes_missing_profile,
    both_absent
);

fn unknown_recorded_configured(shape: Shape) {
    let rig = Rig::stopped("unknown", shape, true);
    rig.set_row("config_home.spawned.0", Some("unknown"));
    rig.set_row("config_home_base.spawned.0", None);
    rig.refuse("store");
}
both_routes!(
    flagless_configured_recorded_unknown_refuses,
    explicit_configured_recorded_unknown_refuses,
    unknown_recorded_configured
);

fn unknown_recorded_unconfigured(shape: Shape) {
    let rig = Rig::stopped("unknown-off", shape, true);
    rig.set_row("config_home.spawned.0", Some("unknown"));
    rig.set_row("config_home_base.spawned.0", None);
    let identity = rig.identity("b", "store-b", "claude");
    rig.write_config(
        identity.split("[profiles]").next().expect("clients"),
        "",
        false,
    );
    rig.no_start();
}
both_routes!(
    flagless_unconfigured_unknown_is_kept,
    explicit_unconfigured_unknown_is_kept,
    unknown_recorded_unconfigured
);

fn current_unknown_mercy(shape: Shape) {
    let rig = Rig::stopped_implicit("mercy", shape);
    let carried = [
        rig.row("client.spawned.0"),
        rig.row("config_home.spawned.0"),
        rig.row("config_home_base.spawned.0"),
    ];
    let identity = rig.identity("b", "store-b", "claude");
    let identity = identity
        .lines()
        .map(|line| {
            if line.starts_with("carry = ") {
                format!("carry = {}", rig.binary("b", "claude").display())
            } else if line.starts_with("default = ") {
                format!("default = {}", rig.binary("a", "claude").display())
            } else {
                line.replace(
                    "p = \"default ",
                    "p = \"CLAUDE_CONFIG_DIR=rel/store default ",
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    rig.write_config(&identity, "", false);
    success(&rig.resume());
    rig.wait_exec("b", ".claude");
    assert_eq!(
        [
            rig.row("client.spawned.0"),
            rig.row("config_home.spawned.0"),
            rig.row("config_home_base.spawned.0")
        ],
        carried,
        "mercy preserves the carried identity"
    );
}
both_routes!(
    flagless_current_unknown_keeps_retained_store,
    explicit_current_unknown_keeps_retained_store,
    current_unknown_mercy
);

fn unavailable_authority(shape: Shape) {
    for (tag, bad) in [
        ("no-config", None),
        (
            "bad-config",
            Some("[clients]\ncarry = claude\ncarry = claude\n"),
        ),
        ("dir-config", Some("DIRECTORY")),
    ] {
        let rig = Rig::stopped(tag, shape, true);
        std::fs::remove_file(&rig.config).expect("remove recorded authority");
        match bad {
            Some("DIRECTORY") => std::fs::create_dir(&rig.config).expect("unreadable authority"),
            Some(body) => std::fs::write(&rig.config, body).expect("unparseable authority"),
            None => (),
        }
        rig.refuse("config");
    }
}
both_routes!(
    flagless_unavailable_global_authority_refuses,
    explicit_unavailable_global_authority_refuses,
    unavailable_authority
);

fn invalid_identity(shape: Shape) {
    let rig = Rig::stopped("invalid", shape, true);
    rig.set_row("client.spawned.0", Some(""));
    rig.refuse("client");
}
both_routes!(
    flagless_invalid_client_still_refuses,
    explicit_invalid_client_still_refuses,
    invalid_identity
);

fn non_simple(shape: Shape) {
    let rig = Rig::stopped("non-simple", shape, true);
    let identity = rig
        .identity("b", "store-b", "claude")
        .replace("--model chosen\"", "--model chosen ; second\"");
    rig.write_config(&identity, "", false);
    rig.refuse("profile");
}
both_routes!(
    flagless_non_simple_pair_refuses_preflight,
    explicit_non_simple_pair_refuses_preflight,
    non_simple
);

fn unchanged_bare(shape: Shape) {
    let rig = Rig::stopped("bare", shape, false);
    success(&rig.resume());
    rig.wait_exec("a", "store-a");
    assert_eq!(rig.row("client.spawned.0"), None);
}
both_routes!(
    flagless_no_client_control,
    explicit_no_client_control,
    unchanged_bare
);

// A real lifecycle barrier, not a synthetic resolver callback. Each mutation
// occurs AFTER the preflight marker and BEFORE the lock is released. Refusal
// may have migrated state: only preflight tests claim byte-for-byte neutrality.
fn identity_race(shape: Shape) {
    for (tag, key, replacement, initially_labeled, unconfigured) in [
        (
            "race-change",
            "client.spawned.0",
            Some("default"),
            true,
            false,
        ),
        ("race-remove", "client.spawned.0", None, true, false),
        ("race-add", "client.spawned.0", Some("carry"), false, false),
        (
            "race-profile",
            "profile.spawned.0",
            Some("idle"),
            true,
            false,
        ),
        ("race-name", "seat.spawned.0", Some("renamed"), true, false),
        (
            "race-tool",
            "agent_bin.spawned.0",
            Some("codex"),
            true,
            false,
        ),
        (
            "race-store",
            "config_home.spawned.0",
            Some("unknown"),
            true,
            false,
        ),
        ("race-off", "client.spawned.0", Some("default"), true, true),
        (
            "race-base",
            "config_home_base.spawned.0",
            Some("/different-base"),
            true,
            false,
        ),
    ] {
        let rig = if key == "config_home_base.spawned.0" {
            Rig::stopped_implicit(tag, shape)
        } else {
            Rig::stopped(tag, shape, initially_labeled)
        };
        if unconfigured {
            let identity = rig.identity("b", "store-b", "claude");
            rig.write_config(
                identity.split("[profiles]").next().expect("clients"),
                "",
                false,
            );
        }
        let held = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(rig.home.join("sessions/.lifecycle.cfsession.lock"))
            .expect("real lifecycle lock");
        held.try_lock().expect("hold lifecycle lock");
        let child = rig
            .launch_command(true, true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("resume racing a recorded identity update");
        let marker = rig.home.join(ae::session_launch::TEST_PRE_LOCK_MARKER);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.is_file() {
            assert!(
                Instant::now() < deadline,
                "real resume did not reach preflight barrier"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        rig.set_row(key, replacement);
        drop(held);
        let out = child.wait_with_output().expect("raced resume completes");
        assert!(
            !out.status.success(),
            "stale judgment must refuse for {key}: {out:?}"
        );
        assert!(
            rig.execs().is_empty(),
            "late refusal must execute no tool: {}",
            rig.execs()
        );
        assert!(
            !rig.tmux(&["has-session", "-t", "=cfsession"]).0,
            "late refusal rolls back created panes"
        );
    }
}
both_routes!(
    flagless_identity_changes_while_waiting_refuse,
    explicit_identity_changes_while_waiting_refuse,
    identity_race
);
