//! Frozen acceptance for #296 part A: `spawn --using profile@client`.
//! Oracle: issue #296 (Decisions + Acceptance), plan r2 (.local/presetspawn/plan.md
//! secs 2-4, 12) and lead rulings ae-20261104T135756Z-83b478f9. Authored by the
//! navigator (Meta) before implementation; the driver installs byte-identical.
//!
//! Contract summary: an `@` spawn resolves profile AND client from the session's
//! recorded GLOBAL config only (a project overlay never approves or redefines a
//! pair), is admitted only when the pair is on the global `[client_pairs]`
//! list (policy before existence), passes the launch same-known-adapter gate
//! and the first-start unknown-store refusal, and persists `profile.<slot>`
//! bare plus `client.<slot>` in the seat's own publish. Every refusal exits 1
//! and leaves meta, events and panes untouched. Bare spawns are unchanged.
//! Resume rows live beside the launch rig in `session_launch.rs`
//! (`presetspawn_resumed_spawned_seat_*`); the superseded R5 row is in `spawn.rs`.
//!
//! On base 1a446bda (measured): `presetspawn_roster_list_*` is green; the
//! bare-spawn and bare-cover CONTROLS inside the red tests pass; every
//! admitted-pair row is red (spawn refuses `@` outright today), and BOTH
//! resume rows are red — the snapshot arm runs the recorded seat on the
//! default account, flagless included whenever a snapshot exists (a solo
//! roster already builds one).

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]
#![allow(
    clippy::expect_used,
    reason = "frozen acceptance fixtures must parse and run or fail loudly"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::cli::ae;
use super::phase2::{run_tmux, tmux_present};
use super::support::{SETUP_WAIT, first_pane};

/// A claude-shaped fake agent (the spawn suite's fake), signing each exec
/// with its side and the account variable it actually ran under, so a test
/// proves WHICH binary ran on WHICH account, never just what meta claims.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my $out = "__RECEIVED__";
system("stty raw -echo 2>/dev/null");
binmode(STDIN, ':raw');
binmode(STDOUT, ':raw');
$| = 1;
print "\e[?2004h";
my $border = "\xe2\x94\x80" x 400;
my $ornament = "\xe2\x9d\xaf";
my $nbsp = "\xc2\xa0";
open(my $log, '>>', "__LAUNCHED__") or die; print $log join(" ", @ARGV), "\n"; print $log "CLAUDE_CONFIG_DIR=", ($ENV{CLAUDE_CONFIG_DIR} // "<unset>"), " side=__SIDE__\n"; close($log);
sub draw {
    my ($content) = @_;
    $content =~ s/[\r\n]/ /g;
    print "\e[H\e[2J";
    print "fake claude transcript\r\n";
    print "\e[1m$ornament\e[0m$nbsp$content\r\n";
    print "$border\r\n";
    print "  fake-model  ~/x\r\n";
}
draw("");
my $buf = "";
my $pasting = 0;
my $ch;
while (1) {
    my $ready = '';
    vec($ready, fileno(STDIN), 1) = 1;
    next unless select($ready, undef, undef, 0.05) > 0;
    last unless sysread(STDIN, $ch, 1);
    $buf .= $ch;
    if ($buf =~ s/\e\[200~\z//) { $pasting = 1; next; }
    if ($buf =~ s/\e\[201~\z//) { $pasting = 0; draw($buf); next; }
    if ($pasting && $ch eq "\r") { chop($buf); $buf .= "\n"; next; }
    if (!$pasting && ($ch eq "\r" || $ch eq "\n")) {
        $buf =~ s/[\r\n]\z//;
        open(my $fh, '>>', $out) or die;
        binmode($fh);
        print $fh $buf;
        close($fh);
        $buf = "";
        draw("");
        next;
    }
    draw($buf);
}
"#;

/// The approved pairs the default rig config carries. `codex`, `zzz` and
/// `ghostc` are listed under `fake` on purpose: only the adapter, unknown and
/// existence gates may fire for them, never the pair gate.
const PAIRS: &str = "fake = cc, cc-mic, codex, zzz, ghostc\n\
                     relp = plain\nconfp = cc-mic\nbad = cc-mic\nghostp = cc-mic\n";

/// One isolated server, one live session, a v2 meta, and a recorded GLOBAL
/// config with two same-harness clients on two accounts (side a / side b),
/// a codex client, an unknown-binary client, and the approved-pair list.
struct Rig {
    scratch: PathBuf,
    sock: PathBuf,
    dir: PathBuf,
    session: String,
    main_pane: String,
    config: PathBuf,
    launched: PathBuf,
    store_a: PathBuf,
    store_b: PathBuf,
}

/// Two claude binaries, one per account, signing their side; plus a
/// codex-named and an unknown-named executable for the adapter gates
/// (neither ever runs — the gates refuse first — they only classify).
fn install_fakes(scratch: &Path, launched: &Path, received: &Path) {
    use std::os::unix::fs::PermissionsExt;
    for side in ["a", "b"] {
        let bindir = scratch.join(format!("bin-{side}"));
        assert!(std::fs::create_dir_all(&bindir).is_ok(), "a bin dir");
        let body = FAKE_CLAUDE
            .replace("__RECEIVED__", &received.display().to_string())
            .replace("__LAUNCHED__", &launched.display().to_string())
            .replace("__SIDE__", side);
        let bin = bindir.join("claude");
        assert!(std::fs::write(&bin, body).is_ok(), "the fake {side}");
        assert!(
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable fake {side}"
        );
    }
    let misc = scratch.join("bin");
    assert!(std::fs::create_dir_all(&misc).is_ok(), "a misc bin dir");
    for name in ["codex", "zzztool"] {
        let bin = misc.join(name);
        assert!(
            std::fs::copy(scratch.join("bin-a/claude"), &bin).is_ok(),
            "a {name} fake"
        );
        assert!(
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable {name}"
        );
    }
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = super::cli::OwnedScratch::root("ps", tag).keep();
        let session = format!("ps{tag}");
        let dir = scratch.join("sessions").join(&session);
        assert!(std::fs::create_dir_all(&dir).is_ok(), "a session dir");
        let launched = scratch.join("launched");
        let received = scratch.join("received");
        assert!(std::fs::write(&received, "").is_ok(), "the receipt file");
        let store_a = scratch.join("store-a");
        let store_b = scratch.join("store-b");
        for store in [&store_a, &store_b] {
            assert!(std::fs::create_dir_all(store).is_ok(), "an account store");
        }
        install_fakes(&scratch, &launched, &received);
        let config = scratch.join("config");
        let sock = scratch.join("sock");
        let rig = Self {
            scratch: scratch.clone(),
            sock,
            dir,
            session: session.clone(),
            main_pane: String::new(),
            config,
            launched,
            store_a,
            store_b,
        };
        rig.write_config(PAIRS);
        assert!(
            rig.tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-x",
                "400",
                "-y",
                "40",
                "-n",
                "main",
                "-s",
                &session,
                "sh",
            ])
            .0,
            "the session starts"
        );
        let main_pane = first_pane(
            |args| rig.tmux(args),
            &["-s", "-t", &session],
            Instant::now() + SETUP_WAIT,
        );
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &main_pane, "@ae_agent", "lead"])
                .0
        );
        assert!(
            rig.tmux(&["set-option", "-p", "-t", &main_pane, "@ae_slot", "main"])
                .0
        );
        assert!(
            std::fs::write(
                rig.dir.join("meta"),
                format!(
                    "session={session}\nwork_dir={}\norigin={}\nmode=local\nlayout=vertical\nconfig={}\nmain_pane={main_pane}\ntmux_server_kind=socket\ntmux_server={}\nschema=2\nseat.main=lead\nprofile.main=fake\nagent_bin.main=claude\n",
                    scratch.display(),
                    scratch.display(),
                    rig.config.display(),
                    rig.sock.display(),
                ),
            )
            .is_ok(),
            "a v2 meta"
        );
        let mut rig = rig;
        rig.main_pane = main_pane;
        rig
    }

    /// The whole global config with one `[client_pairs]` body swapped in.
    fn write_config(&self, pairs: &str) {
        let a = self.scratch.join("bin-a/claude").display().to_string();
        let b = self.scratch.join("bin-b/claude").display().to_string();
        let body = format!(
            "[clients]\n\
             cc = {a} config_home={sa}\n\
             cc-mic = {b} config_home={sb}\n\
             cc-other = {b} config_home={sb}\n\
             claude = {a}\n\
             plain = {b}\n\
             codex = {misc}/codex\n\
             zzz = {misc}/zzztool\n\n\
             [profiles]\n\
             fake = \"cc --model fake\"\n\
             solx = \"codex -m sol\"\n\
             relp = \"CLAUDE_CONFIG_DIR=rel/store claude --model rel\"\n\
             confp = \"CLAUDE_CONFIG_DIR={sa} claude --model conf\"\n\
             bad = \"/usr/bin/touch {marker}; /usr/bin/tail -f /dev/null\"\n\n\
             [client_pairs]\n{pairs}\n\
             [workspace]\nmain = lead\nlayout = vertical\n",
            sa = self.store_a.display(),
            sb = self.store_b.display(),
            misc = self.scratch.join("bin").display(),
            marker = self.scratch.join("marker").display(),
        );
        assert!(
            std::fs::write(&self.config, body).is_ok(),
            "a global config"
        );
    }

    /// A project overlay at the recorded origin (`<origin>/.ae/config`).
    fn write_local(&self, body: &str) {
        let local_dir = self.scratch.join(".ae");
        assert!(
            std::fs::create_dir_all(&local_dir).is_ok(),
            "an overlay dir"
        );
        assert!(
            std::fs::write(local_dir.join("config"), body).is_ok(),
            "a project overlay"
        );
    }

    fn tmux(&self, tail: &[&str]) -> (bool, String) {
        let mut args = ae::tmux::server_args(&ae::inventory::ServerId::Selected(
            ae::meta::Selector::Socket(self.sock.clone()),
        ));
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        run_tmux(&args, &self.scratch)
    }

    /// Run one core subcommand as the lead's pane.
    fn run(&self, sub: &str, tail: &[&str]) -> (Option<i32>, String, String) {
        let out = ae()
            .env("TMUX", format!("{},0,0", self.sock.display()))
            .env("TMUX_PANE", &self.main_pane)
            .env_remove("AE_SENDER_OVERRIDE")
            .arg(sub)
            .arg(&self.dir)
            .args(tail)
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn spawn(&self, name: &str, using: &str) -> (Option<i32>, String, String) {
        self.run(
            ae::cli::SPAWN,
            &[name, "--using", using, "--", "do the task"],
        )
    }

    fn meta(&self) -> String {
        std::fs::read_to_string(self.dir.join("meta")).unwrap_or_default()
    }

    fn events(&self) -> String {
        std::fs::read_to_string(self.dir.join("events.jsonl")).unwrap_or_default()
    }

    fn panes(&self) -> Vec<(String, String, String)> {
        let (_, listed) = self.tmux(&[
            "list-panes",
            "-s",
            "-t",
            &self.session,
            "-F",
            "#{pane_id}|#{@ae_slot}|#{@ae_agent}",
        ]);
        listed
            .lines()
            .map(|line| {
                let mut fields = line.splitn(3, '|');
                (
                    fields.next().unwrap_or_default().to_owned(),
                    fields.next().unwrap_or_default().to_owned(),
                    fields.next().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    /// The exec log, waiting for at least `execs` signed execs.
    fn launched_wait(&self, execs: usize) -> String {
        for _ in 0..400 {
            let seen = std::fs::read_to_string(&self.launched).unwrap_or_default();
            if seen.matches("side=").count() >= execs {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        std::fs::read_to_string(&self.launched).unwrap_or_default()
    }

    /// The canonical store a path resolves to (`_run` canonicalizes).
    fn canon(path: &Path) -> String {
        std::fs::canonicalize(path)
            .expect("a canonical store")
            .display()
            .to_string()
    }

    /// The client a seat's worker-birth certificate proved, if it proved
    /// one (`None` for no certificate or an unproved client — no pin here
    /// needs the distinction).
    fn birth_client(&self, name: &str) -> Option<String> {
        self.events().lines().find_map(|line| {
            let event = ae::events::Event::parse_line(line).ok()?;
            let ae::birth::Record::Birth(birth) = ae::birth::decode(&event)?.ok()? else {
                return None;
            };
            if birth.name.as_deref() == Some(name) {
                birth.client.clone()
            } else {
                None
            }
        })
    }
}

fn skip() -> bool {
    !tmux_present(&super::cli::OwnedScratch::root("ps", "probe").keep())
}

/// The meta row KEYS one slot carries, values aside — for shape comparisons
/// across two seats whose launch ids and floors differ.
fn slot_keys(meta: &str, slot: &str) -> Vec<String> {
    let suffix = format!(".{slot}");
    let mut keys: Vec<String> = meta
        .lines()
        .filter_map(|line| line.split_once('=').map(|(key, _)| key))
        .filter(|key| key.ends_with(&suffix))
        .map(|key| key.strip_suffix(&suffix).unwrap_or(key).to_owned())
        .collect();
    keys.sort();
    keys
}

/// #296 core: a listed pair spawns on the OVERRIDE account and persists it —
/// exec evidence (side + account variable), meta rows, the recorded store,
/// the workspace roster cell, `_roster list`, and the birth certificate all
/// read the recorded client, and no `@` spelling reaches meta.
#[test]
fn presetspawn_approved_pair_runs_override_account_and_persists() {
    if skip() {
        return;
    }
    let rig = Rig::new("pair");
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let meta = rig.meta();
    assert!(meta.contains("seat.spawned.0=helper\n"), "{meta}");
    assert!(
        meta.contains("profile.spawned.0=fake\n"),
        "bare profile: {meta}"
    );
    assert!(meta.contains("client.spawned.0=cc-mic\n"), "{meta}");
    assert!(meta.contains("agent_bin.spawned.0=claude\n"), "{meta}");
    assert!(!meta.contains('@'), "no @ reaches meta: {meta}");
    let store_b = Rig::canon(&rig.store_b);
    // `_run` records the store it execed against, before the exec; wait for
    // the exec, then both the row and the sign line are settled.
    let launched = rig.launched_wait(1);
    assert!(
        launched.contains(&format!("CLAUDE_CONFIG_DIR={store_b} side=b")),
        "the pane ran the override binary on the override store: {launched}"
    );
    let meta = rig.meta();
    assert!(
        meta.contains(&format!("config_home.spawned.0={store_b}\n")),
        "the recorded store is the override account: {meta}"
    );
    let manifest = std::fs::read_to_string(rig.dir.join("workspace.md")).unwrap_or_default();
    assert!(
        manifest.contains("| helper | fake@cc-mic |"),
        "the roster cell reads profile@client: {manifest}"
    );
    assert_eq!(
        rig.birth_client("helper"),
        Some("cc-mic".to_owned()),
        "the birth certificate proves the override client"
    );
    // `_roster list` resolves the recorded label into the seat's command.
    let global = rig.config.display().to_string();
    let (code, stdout, stderr) = rig.run(ae::cli::ROSTER, &["list", "--global", &global]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let helper = stdout
        .lines()
        .find(|line| line.contains("helper"))
        .unwrap_or_default();
    // The list prints the CONFIGURED spelling (it never canonicalizes);
    // the canonical spelling is pinned on the `config_home` row above.
    assert!(
        helper.contains(&rig.scratch.join("bin-b/claude").display().to_string())
            && helper.contains(&rig.store_b.display().to_string()),
        "roster list resolves through cc-mic: {helper}"
    );
}

/// Bare and pair seats differ by exactly the client row: a bare spawn keeps
/// today's shape byte-for-byte in row keys, records no client anywhere, and
/// runs the profile's own account. Also pins `--using=<v>` == `--using <v>`.
#[test]
fn presetspawn_bare_and_pair_seats_differ_only_by_client_row() {
    if skip() {
        return;
    }
    let rig = Rig::new("shapes");
    let (code, stdout, stderr) = rig.spawn("bareone", "fake");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let (code, stdout, stderr) = rig.run(
        ae::cli::SPAWN,
        &["pairone", "--using=fake@cc-mic", "--", "do the task"],
    );
    assert_eq!(code, Some(0), "--using= admits the pair: {stdout}{stderr}");
    let launched = rig.launched_wait(2);
    let store_a = Rig::canon(&rig.store_a);
    let store_b = Rig::canon(&rig.store_b);
    assert!(
        launched.contains(&format!("CLAUDE_CONFIG_DIR={store_a} side=a"))
            && launched.contains(&format!("CLAUDE_CONFIG_DIR={store_b} side=b")),
        "bare ran its own account, the pair the override: {launched}"
    );
    let meta = rig.meta();
    let bare = slot_keys(&meta, "spawned.0");
    let pair = slot_keys(&meta, "spawned.1");
    assert!(!bare.iter().any(|key| key == "client"), "bare: {bare:?}");
    assert_eq!(
        pair.iter().filter(|key| *key != "client").count(),
        bare.len(),
        "the pair seat adds exactly one row kind: bare {bare:?} pair {pair:?}"
    );
    assert!(
        pair.iter().filter(|key| *key != "client").eq(bare.iter()),
        "same rows, same order: bare {bare:?} pair {pair:?}"
    );
    assert_eq!(
        rig.birth_client("bareone"),
        Some("cc".to_owned()),
        "a bare birth proves the profile's own client"
    );
}

/// Every `@` refusal: exit 1, its own text, and NOTHING took effect — meta,
/// events and panes byte-identical after each. Order pinned: policy
/// (approved pair) before existence; existence before the adapter gate.
#[test]
fn presetspawn_refusals_take_no_seat() {
    if skip() {
        return;
    }
    let rig = Rig::new("refuse");
    let meta0 = rig.meta();
    let events0 = rig.events();
    let panes0 = rig.panes();
    for (using, expected, absent) in [
        // Malformed `@` shapes.
        ("fake@", "is not <profile> or <profile>@<client>", ""),
        ("@cc-mic", "is not <profile> or <profile>@<client>", ""),
        ("fake@cc@mic", "is not <profile> or <profile>@<client>", ""),
        // Unlisted pair (client exists): the refusal teaches discovery.
        ("fake@cc-other", "is not an approved pair", ""),
        ("fake@cc-other", "Approved clients for 'fake':", ""),
        ("fake@cc-other", "Nothing was spawned.", ""),
        // Policy before existence: an unlisted AND undefined client still
        // reads as not approved, never as an existence answer.
        (
            "fake@nosuch",
            "is not an approved pair",
            "not defined in [clients]",
        ),
        // Listed pair, undefined profile / client: existence answers.
        ("ghostp@cc-mic", "not defined in [profiles]", ""),
        ("fake@ghostc", "not defined in [clients]", ""),
        // Listed cross-harness pair: ONLY the adapter gate can fire.
        ("fake@codex", "never the harness adapter", ""),
        ("fake@codex", "fake@codex", ""),
        // Listed unknown-binary client: unknown on one side refuses.
        ("fake@zzz", "unrecognized binary", ""),
        // First start on an unverifiable store (relative assignment).
        ("relp@plain", "unknown conversation store", ""),
        // Profile prefix and client both set the account variable.
        (
            "confp@cc-mic",
            "both profile and client 'cc-mic' set CLAUDE_CONFIG_DIR",
            "",
        ),
        // A listed pair whose profile is not one simple command.
        ("bad@cc-mic", "one simple command", ""),
    ] {
        let (code, stdout, stderr) = rig.spawn("helper", using);
        assert_eq!(code, Some(1), "{using}: {stdout}{stderr}");
        assert!(stderr.contains(expected), "{using}: {stderr}");
        assert!(
            absent.is_empty() || !stderr.contains(absent),
            "{using} must not answer existence before policy: {stderr}"
        );
        assert_eq!(rig.meta(), meta0, "{using} wrote meta");
        assert_eq!(rig.events(), events0, "{using} wrote events");
        assert_eq!(rig.panes(), panes0, "{using} created a pane");
    }
    assert!(
        !rig.scratch.join("marker").exists(),
        "the refused two-command profile never ran"
    );
}

/// A `[client_pairs]` row ae cannot read in full approves nothing: duplicate
/// profile key, an unreadable label, or an empty list item each VOID the row
/// (never partial approval, never last-wins), and the refusal says so.
#[test]
fn presetspawn_void_pair_rows_refuse() {
    if skip() {
        return;
    }
    let rig = Rig::new("voidrows");
    let meta0 = rig.meta();
    for pairs in [
        "fake = cc-mic\nfake = cc-mic\n",
        "fake = cc-mic, cc mic\n",
        "fake = cc-mic,\n",
        "fake =\n",
    ] {
        rig.write_config(pairs);
        let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
        assert_eq!(code, Some(1), "{pairs:?}: {stdout}{stderr}");
        assert!(
            stderr.contains("is not an approved pair") && stderr.contains("unusable"),
            "{pairs:?}: {stderr}"
        );
        assert_eq!(rig.meta(), meta0, "{pairs:?} wrote meta");
    }
    // Control: the same pair under a clean row spawns.
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(1), "still void before rewrite: {stdout}{stderr}");
    rig.write_config(PAIRS);
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
}

/// The BLOCKER pin: approval binds to the GLOBAL definitions of both names.
/// A project overlay can neither approve a pair nor redefine an approved
/// client/profile into its own binary or account; a profile defined only in
/// the overlay is unknown to an `@` spawn (it still spawns bare, unchanged).
#[test]
fn presetspawn_project_overlay_never_approves_or_redefines() {
    if skip() {
        return;
    }
    let rig = Rig::new("overlay");
    // Overlay-only approval approves nothing.
    rig.write_config("");
    rig.write_local("[client_pairs]\nfake = cc-mic\n");
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("is not an approved pair"), "{stderr}");
    assert!(!rig.meta().contains("spawned."), "{}", rig.meta());
    // Global approval back; the overlay now REDEFINES the approved client
    // (side a binary, store a) and the approved profile (side a word). The
    // spawn must still run the GLOBAL pair: side b on store b.
    rig.write_config(PAIRS);
    let a = rig.scratch.join("bin-a/claude").display().to_string();
    rig.write_local(&format!(
        "[clients]\ncc-mic = {a} config_home={}\n\n[profiles]\nfake = \"{a} --model hijacked\"\nlocalp = \"cc --model local\"\n",
        rig.store_a.display()
    ));
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let launched = rig.launched_wait(1);
    let store_b = Rig::canon(&rig.store_b);
    assert!(
        launched.contains(&format!("CLAUDE_CONFIG_DIR={store_b} side=b"))
            && !launched.contains("hijacked"),
        "the global pair ran, never the overlay redefinition: {launched}"
    );
    // An overlay-only profile is unknown to `@` even with a global pair row…
    rig.write_config("localp = cc-mic\n");
    let (code, stdout, stderr) = rig.spawn("localone", "localp@cc-mic");
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("not defined in [profiles]"), "{stderr}");
    // …and still spawns bare through the layered path, as today.
    let (code, stdout, stderr) = rig.spawn("localone", "localp");
    assert_eq!(
        code,
        Some(0),
        "bare overlay spawn unchanged: {stdout}{stderr}"
    );
}

/// Retiring a route = removing its pair line: new spawns refuse, and the
/// already-recorded seat (rows, account) is untouched — approval gates new
/// admissions, never recorded seats. Same-label pairs need a row like any
/// other (lead ruling 4): listed `fake@cc` records the row, unlisted refuses.
#[test]
fn presetspawn_retired_pair_refuses_and_recorded_seat_survives() {
    if skip() {
        return;
    }
    let rig = Rig::new("retired");
    let (code, stdout, stderr) = rig.spawn("first", "fake@cc");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        rig.meta().contains("client.spawned.0=cc\n"),
        "a spelled same-default label is recorded: {}",
        rig.meta()
    );
    rig.launched_wait(1);
    let settled = rig.meta();
    // Retire every route: the pair section goes away entirely.
    rig.write_config("");
    let (code, stdout, stderr) = rig.spawn("second", "fake@cc-mic");
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("is not an approved pair"), "{stderr}");
    let (code, stdout, stderr) = rig.spawn("third", "fake@cc");
    assert_eq!(
        code,
        Some(1),
        "same-label needs its row too: {stdout}{stderr}"
    );
    assert!(stderr.contains("is not an approved pair"), "{stderr}");
    assert_eq!(rig.meta(), settled, "the recorded seat is untouched");
}

/// Lead ruling 3: the auto-reseat cover warning judges the profile's own
/// account, so an override spawn skips it — while the same profile bare
/// still warns in the same config.
#[test]
fn presetspawn_cover_warning_skipped_on_override() {
    if skip() {
        return;
    }
    let rig = Rig::new("cover");
    // auto_reseat on, no declared exit for `fake`, one other-tool profile so
    // a suggestion exists: bare `fake` warns (behavior on base today).
    let a = rig.scratch.join("bin-a/claude").display().to_string();
    let b = rig.scratch.join("bin-b/claude").display().to_string();
    let body = format!(
        "[clients]\ncc = {a} config_home={sa}\ncc-mic = {b} config_home={sb}\n\n\
         [profiles]\nfake = \"cc --model fake\"\ngrk = \"grok --model grk\"\n\n\
         [client_pairs]\nfake = cc-mic\n\n\
         [workspace]\nmain = lead\nlayout = vertical\nauto_reseat = on\n",
        sa = rig.store_a.display(),
        sb = rig.store_b.display(),
    );
    assert!(std::fs::write(&rig.config, body).is_ok(), "a cover config");
    let (code, stdout, stderr) = rig.spawn("bareone", "fake");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        stderr.contains("has no auto-reseat exit"),
        "the bare control warns in this config: {stderr}"
    );
    let (code, stdout, stderr) = rig.spawn("pairone", "fake@cc-mic");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        !stderr.contains("has no auto-reseat exit"),
        "no wrong-account warning on an override spawn: {stderr}"
    );
}

/// Retire removes a pair seat whole — the client row goes with every other
/// `.<slot>` row (the same owner every rollback path uses).
#[test]
fn presetspawn_retire_removes_client_row() {
    if skip() {
        return;
    }
    let rig = Rig::new("retire");
    let (code, stdout, stderr) = rig.spawn("helper", "fake@cc-mic");
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    rig.launched_wait(1);
    assert!(rig.meta().contains("client.spawned.0=cc-mic\n"));
    let (code, stdout, stderr) = rig.run(ae::cli::RETIRE, &["helper"]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        !rig.meta().contains("spawned.0"),
        "no row of the retired seat survives: {}",
        rig.meta()
    );
}

/// GREEN on base: the recorded-client readers already honor a
/// `client.spawned.N` row for any slot. Hand-written here exactly as a
/// correct spawn writes it; `_roster list` resolves the override command.
#[test]
fn presetspawn_roster_list_honors_recorded_client_on_spawned_slot() {
    if skip() {
        return;
    }
    let rig = Rig::new("rosterread");
    let mut meta = rig.meta();
    meta.push_str(
        "seat.spawned.0=helper\nprofile.spawned.0=fake\nclient.spawned.0=cc-mic\nagent_bin.spawned.0=claude\n",
    );
    assert!(std::fs::write(rig.dir.join("meta"), meta).is_ok(), "a meta");
    let global = rig.config.display().to_string();
    let (code, stdout, stderr) = rig.run(ae::cli::ROSTER, &["list", "--global", &global]);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    let helper = stdout
        .lines()
        .find(|line| line.contains("helper"))
        .unwrap_or_default();
    assert!(
        helper.contains(&rig.scratch.join("bin-b/claude").display().to_string()),
        "the listed command is the override's: {helper}"
    );
}
