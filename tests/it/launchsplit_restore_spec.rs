//! Restore races at baseline 719c5d23: scan output is frozen before the competing effect.
use super::*;

const AFTER_SCAN: &str = r#"
if ((grep { $_ eq 'list-sessions' } @ARGV) && !-e $ENV{AE_LSP_ONCE}) {
    my $countfile = "$ENV{AE_LSP_ONCE}.count";
    my $count = 0;
    if (open(my $old, '<', $countfile)) { $count = <$old> || 0; close($old); }
    $count++;
    open(my $next, '>', $countfile) or die "count: $!";
    print $next $count; close($next) or die "close count: $!";
    my $target = $ENV{AE_LSP_FAULT} eq 'guard' ? 2 : 1;
    if ($count == $target) {
    open(my $once, '>', $ENV{AE_LSP_ONCE}) or die "once: $!";
    print $once "applied\n"; close($once) or die "close once: $!";
    open(my $listed, '-|', $ENV{AE_LAUNCHX_REAL_TMUX}, @ARGV) or die "snapshot: $!";
    print while <$listed>;
    close($listed) or die "snapshot failed: $?";
    my $fault = $ENV{AE_LSP_FAULT};
    if ($fault eq 'gone') {
        rename($ENV{AE_LSP_META}, "$ENV{AE_LSP_META}.saved") or die "withdraw meta: $!";
    } elsif ($fault eq 'guard') {
        open(my $stamp, '>', $ENV{AE_LSP_STAMP}) or die "stamp: $!";
        print $stamp "1\n"; close($stamp) or die "close stamp: $!";
    } elsif ($fault eq 'running') {
        system($ENV{AE_LAUNCHX_REAL_TMUX}, '-u', '-S', $ENV{AE_LSP_SOCKET},
            'new-session', '-d', '-s', 'held', '-c', $ENV{AE_LSP_PROJECT}) == 0
            or die "concurrent session create: $?";
    } else { die "unknown fault: $fault"; }
    exit 0;
    }
}
"#;

#[test]
fn launchsplit_restore_rechecks_record_guard_and_running_name_after_scan() {
    for (fault, reason) in [
        ("gone", "saved record gone before the lifecycle lock"),
        ("guard", "launched again since the scan"),
        ("running", "already running"),
    ] {
        let fleet = Fleet::new("lsp-restore-race");
        fleet.missing("held", 600, Some(60));
        let dir = fleet.dir("held");
        let meta = dir.join("meta");
        let before = std::fs::read(&meta).expect("saved metadata");
        let journal = dir.join("events.jsonl");
        let journal_before = std::fs::read(&journal).expect("saved journal");
        let stamp = dir.join(".launch-attempt");
        let stamp_before = std::fs::read(&stamp).expect("saved launch attempt");
        let mut command = fleet.command(&["--no-attach"]);
        let log = crate::session_launch::launchx_trace_command(&mut command, &fleet.scratch);
        let wrapper = fleet.scratch.join("trace-bin/tmux");
        let original = std::fs::read_to_string(&wrapper).expect("existing tmux wrapper");
        let marker = "exec {$ENV{AE_LAUNCHX_REAL_TMUX}}";
        assert_eq!(original.matches(marker).count(), 1, "one insertion point");
        std::fs::write(
            &wrapper,
            original.replace(marker, &format!("{AFTER_SCAN}\n{marker}")),
        )
        .expect("controlled competing effect after real scan");
        let once = fleet.scratch.join("after-scan");
        command
            .env("AE_LSP_ONCE", &once)
            .env("AE_LSP_FAULT", fault)
            .env("AE_LSP_META", &meta)
            .env("AE_LSP_STAMP", &stamp)
            .env("AE_LSP_SOCKET", &fleet.socket)
            .env("AE_LSP_PROJECT", &fleet.project);
        let result = command.output().expect("guarded restore");
        success(&result);
        assert_eq!(std::fs::read(&once).expect("effect applied"), b"applied\n");
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            format!("ae: skipped held: {reason}\n"),
            "fault={fault}"
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(
            stdout.contains("ae: restoring held ("),
            "scan never selected restore: {stdout}"
        );
        assert!(
            !stdout.contains("ae: restored held ("),
            "skip counted as restored: {stdout}"
        );
        let trace = crate::session_launch::launchx_trace_text(&log, &fleet.scratch, false);
        assert!(
            !trace.contains("\tnew-session\t"),
            "product rebuilt after skip: {trace}"
        );
        assert!(
            !trace.contains("\tkill-session\t"),
            "product killed a concurrent launch: {trace}"
        );
        unchanged(&journal, &journal_before);
        if fault == "gone" {
            assert!(!meta.exists(), "restore republished withdrawn record");
            unchanged(&dir.join("meta.saved"), &before);
        } else {
            unchanged(&meta, &before);
        }
        if fault != "guard" {
            unchanged(&stamp, &stamp_before);
        }
        assert_eq!(
            fleet.names(),
            if fault == "running" {
                vec!["held", "keeper"]
            } else {
                vec!["keeper"]
            }
        );
    }
}
