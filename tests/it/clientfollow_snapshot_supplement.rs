//! Independent #296 regression supplement, proposed against 243d2d4c.
//! R1b keeps unlabeled/fixed routes unchanged; ordinary spawned _run reads
//! the profile fresh, while a labeled spawned seat carries its judged command.
//! Only the external tmux executable is wrapped; every request is forwarded to
//! real tmux, with its stdin unchanged, on the fixture's private socket.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "independent fixture owns all files, executables and private sockets"
)]

use std::os::unix::fs::PermissionsExt as _;

use super::clientfollow_resume_spec::{Rig, Shape, success};

// Change the fixture config synchronously when the already-composed spawned
// _run line is staged. This is after restore chose the command and before
// that pane can read config. The command bytes are never changed by the wrapper.
const TMUX: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
use Fcntl qw(O_WRONLY O_CREAT O_EXCL);
if (grep { $_ eq 'load-buffer' } @ARGV) {
    binmode(STDIN);
    my $body = do { local $/; <STDIN> };
    defined($body) or die "read tmux stdin: $!";
    if (index($body, ' _run ') >= 0 && $body =~ / 'spawned[.]0'\z/) {
        sysopen(my $mark, $ENV{CF_SNAPSHOT_MARK}, O_WRONLY|O_CREAT|O_EXCL, 0600)
            or die "one spawned staging boundary: $!";
        open(my $next, '<', $ENV{CF_SNAPSHOT_NEXT}) or die "replacement config: $!";
        my $config = do { local $/; <$next> };
        close($next) or die "close replacement config: $!";
        open(my $out, '>', $ENV{CF_SNAPSHOT_CONFIG}) or die "config: $!";
        print $out $config or die "write config: $!";
        close($out) or die "close config: $!";
        open(my $payload, '>', $ENV{CF_SNAPSHOT_PAYLOAD}) or die "payload: $!";
        print $payload $body or die "write payload: $!";
        close($payload) or die "close payload: $!";
        print $mark "config replaced before real tmux load-buffer\n";
        close($mark) or die "close marker: $!";
    }
    my $replay = "$ENV{CF_SNAPSHOT_MARK}.stdin.$$";
    sysopen(my $copy, $replay, O_WRONLY|O_CREAT|O_EXCL, 0600) or die "stdin spool: $!";
    binmode($copy);
    print $copy $body or die "write stdin spool: $!";
    close($copy) or die "close stdin spool: $!";
    open(STDIN, '<', $replay) or die "reopen stdin spool: $!";
    binmode(STDIN);
    unlink($replay) or die "unlink stdin spool: $!";
}
exec {$ENV{CF_SNAPSHOT_REAL_TMUX}} $ENV{CF_SNAPSHOT_REAL_TMUX}, @ARGV;
die "exec real tmux: $!";
"#;

#[allow(
    clippy::too_many_lines,
    reason = "one real resume and its external configuration/exec receipts"
)]
fn exec_boundary_update(labeled: bool) {
    let rig = Rig::new(
        if labeled { "snap-held" } else { "snap-fresh" },
        Shape::Flagless,
    );
    rig.launch();
    success(&rig.spawn("helper", if labeled { "p@carry" } else { "p" }));
    let (old_side, store) = if labeled {
        ("b", "store-b")
    } else {
        ("a", "store-a")
    };
    rig.wait_exec(old_side, store);
    assert_eq!(
        rig.row("client.spawned.0").as_deref(),
        labeled.then_some("carry"),
        "establish the actual recorded route"
    );
    assert!(
        rig.tmux(&["kill-session", "-t", "=cfsession"]).0,
        "stop only the private fixture session"
    );
    std::fs::write(&rig.receipt, "").expect("resume-only external receipts");

    let old_config = std::fs::read_to_string(&rig.config).expect("global config");
    let new_config = old_config.replace(
        &rig.binary(old_side, "claude").display().to_string(),
        &rig.binary("c", "claude").display().to_string(),
    );
    assert_ne!(
        new_config, old_config,
        "same-tool/same-store executable update"
    );
    let next = rig.scratch.join("next-config");
    std::fs::write(&next, &new_config).expect("future external config edit");
    let path = std::env::var_os("PATH").expect("test PATH");
    let real_tmux = std::env::split_paths(&path)
        .map(|dir| dir.join("tmux"))
        .find(|file| file.is_file())
        .expect("real tmux executable");
    let bin = rig.scratch.join("tmux-wrapper");
    std::fs::create_dir(&bin).expect("private wrapper directory");
    let wrapper = bin.join("tmux");
    std::fs::write(&wrapper, TMUX).expect("forwarding wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
        .expect("executable wrapper");
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&path)))
        .expect("private wrapper PATH");
    let marker = rig.scratch.join("boundary-reached");
    let payload = rig.scratch.join("spawned-run-payload");
    let socket = rig.row("tmux_server").expect("recorded private socket");
    let mut command = rig.command();
    command
        .env("PATH", path)
        .env("CF_SNAPSHOT_REAL_TMUX", real_tmux)
        .env("CF_SNAPSHOT_CONFIG", &rig.config)
        .env("CF_SNAPSHOT_NEXT", &next)
        .env("CF_SNAPSHOT_MARK", &marker)
        .env("CF_SNAPSHOT_PAYLOAD", &payload)
        .arg(ae::cli::LAUNCH)
        .arg("--home")
        .arg(&rig.home)
        .arg("--cwd")
        .arg(&rig.project)
        .arg("--global")
        .arg(&rig.config)
        .args(["--server-kind", "socket", "--server"])
        .arg(socket)
        .args(["--no-attach", "--", "--local", "cfsession"]);
    success(&command.output().expect("real flagless resume"));
    assert!(marker.is_file(), "actual spawned staging boundary reached");
    assert_eq!(
        std::fs::read_to_string(&rig.config).expect("edited config"),
        new_config,
        "config update completed before the spawned command was pasted"
    );
    assert!(rig.pane("spawned.0").is_some(), "restored real pane");
    // This external exec/account receipt is the behavioral oracle. The bare
    // route must read new C; the labeled control must still execute judged B.
    let expected_side = if labeled { "b" } else { "c" };
    rig.wait_exec(expected_side, store);
    let receipt = rig.execs();
    let unexpected_side = if labeled { "c" } else { "a" };
    assert!(
        !receipt.contains(&format!(
            "EXEC={} ",
            rig.binary(unexpected_side, "claude").display()
        )),
        "the opposing executable must not run: {receipt}"
    );
    assert_eq!(
        rig.row("client.spawned.0").as_deref(),
        labeled.then_some("carry"),
        "label absence/presence survives the completed resume"
    );
    let payload = std::fs::read_to_string(payload).expect("unaltered tmux stdin receipt");
    assert_eq!(
        payload.contains("--command-snapshot"),
        labeled,
        "the actual launch transport must preserve the fresh/held distinction: {payload}"
    );
}

#[test]
fn flagless_bare_spawn_reads_profile_at_exec() {
    exec_boundary_update(false);
}

#[test]
fn flagless_labeled_spawn_keeps_judged_command_at_exec() {
    exec_boundary_update(true);
}
