//! Independent chat-look acceptance oracle: the brief's group-chat look,
//! inert record bytes, and unchanged plain output. Real terminal tests below
//! drive the binary, so a styling helper nobody calls cannot satisfy them.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns isolated journal and terminal fixtures"
)]

use ae::console::lane::{Item, Kind, Lane};
use ae::console::view::Printed;

const T: i64 = 1_791_002_820_000_000; // 2026-10-03 04:47:00 UTC.

fn item(kind: Kind, body: &str) -> Item {
    Item {
        micros: T,
        kind,
        body: body.to_owned(),
        record: None,
    }
}

#[test]
fn chat_look_plain_renderer_keeps_frozen_bytes_and_pending_order() {
    let mut printed = Printed::default();
    printed.outcome("id-one", "sent id-one".to_owned());
    let lane = Lane {
        items: vec![
            item(
                Kind::Said {
                    who: "lead".to_owned(),
                },
                "hello\n\tcode\n  indent",
            ),
            item(
                Kind::Asked {
                    to: "lead".to_owned(),
                    id: "id-one".to_owned(),
                    uncertain: false,
                },
                "ship?",
            ),
        ],
        coverage: vec!["fixture gap".to_owned()],
    };
    assert_eq!(
        printed.step(&lane, 0, true),
        include_str!("../fixtures/console/chat-look-plain.txt")
    );
    assert_eq!(printed.step(&lane, 0, true), "");
}

/// Track only SGR state, independently of production styling. A raw control
/// sequence from a record fails here rather than disappearing in a strip.
fn sgr_state_at(text: &str, at: usize) -> (Option<[u8; 3]>, bool) {
    let (mut rgb, mut bold) = (None, false);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < at {
        if bytes[i] != 0x1b {
            i += 1;
            continue;
        }
        assert_eq!(bytes.get(i + 1), Some(&b'['), "non-SGR escape: {text:?}");
        let end = text[i + 2..].find('m').expect("terminated SGR") + i + 2;
        let values: Vec<u16> = text[i + 2..end]
            .split(';')
            .map(|value| value.parse().expect("numeric SGR"))
            .collect();
        let mut n = 0;
        while n < values.len() {
            match values[n] {
                0 => {
                    rgb = None;
                    bold = false;
                }
                1 => bold = true,
                2 | 22 | 39 => {}
                38 => {
                    assert_eq!(values.get(n + 1), Some(&2), "24-bit colour required");
                    let channel = |offset| {
                        u8::try_from(*values.get(n + offset).expect("RGB channel"))
                            .expect("byte channel")
                    };
                    rgb = Some([channel(2), channel(3), channel(4)]);
                    n += 4;
                }
                other => panic!("unexpected SGR {other}: {text:?}"),
            }
            n += 1;
        }
        i = end + 1;
    }
    (rgb, bold)
}

fn colour_of(text: &str, needle: &str) -> [u8; 3] {
    let at = text.find(needle).expect("fixture marker printed");
    sgr_state_at(text, at).0.expect("marker coloured")
}

fn strip_sgr(text: &str) -> String {
    let _ = sgr_state_at(text, text.len()); // Validate every escape first.
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for byte in chars.by_ref() {
                if byte == 'm' {
                    break;
                }
            }
        } else {
            plain.push(ch);
        }
    }
    plain
}

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

struct Rig {
    scratch: super::cli::OwnedScratch,
    bin: PathBuf,
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl Rig {
    fn new(tag: &str, palette: &str, icons: bool, drawn: bool, zone: &str) -> Self {
        let scratch = super::cli::OwnedScratch::root("chat-look", tag);
        let root = scratch.path();
        let store = root.join("claude");
        let roster = format!(
            "session_id=0199c0de-aaaa-4890-abcd-ef0123456789\nlayout=lead-pair\n{}{}",
            super::board::claude_roster(
                "main",
                "lead",
                "0199c0de-1234-4890-abcd-ef0123456789",
                &store
            ),
            super::board::claude_roster(
                "worker.0",
                "colead",
                "0199c0de-1234-4890-abcd-ef0123456790",
                &store
            ),
        );
        let dir = super::board::plant_session(root, "one", &roster);
        for id in [
            "0199c0de-1234-4890-abcd-ef0123456789",
            "0199c0de-1234-4890-abcd-ef0123456790",
        ] {
            super::board::plant_transcript(&store, "work", id, &[]);
        }
        fs::write(
            dir.join("events.jsonl"),
            include_str!("../fixtures/console/chat-look-events.jsonl"),
        )
        .expect("fixture journal");
        fs::write(root.join("config"), "").expect("private config");
        let bin = root.join("bin");
        fs::create_dir(&bin).expect("private PATH");
        // Generated Perl fixture: no production process door or shell file.
        fs::write(
            bin.join("tmux"),
            r#"#!/usr/bin/perl
use strict;
use warnings;
open my $log, '>>', $ENV{LOOK_CALLS} or die $!;
print $log join(' ', @ARGV), "\n";
my $format = $ARGV[-1] // '';
if ($format =~ /t\/f\//) { print $ENV{LOOK_ZONE}, "\n"; }
elsif ($format =~ /ae_palette|ae_theme|ae_icons/) { print $ENV{LOOK_ANSWER}, "\n"; }
else { exit 1; }
"#,
        )
        .expect("fake tmux");
        fs::set_permissions(bin.join("tmux"), fs::Permissions::from_mode(0o755))
            .expect("fake executable");
        fs::write(
            root.join("look"),
            format!(
                "{} | {palette} | {} | off",
                if icons { "on" } else { "off" },
                if drawn { "on" } else { "off" }
            ),
        )
        .expect("look answer");
        fs::write(root.join("zone"), zone).expect("zone answer");
        Self { scratch, bin }
    }

    fn run(&self, tty: bool) -> String {
        let root = self.scratch.path();
        let mut runner = if tty {
            super::cli::helper_by_name("script")
        } else {
            super::cli::ae()
        };
        if tty {
            let command = format!("exec {} chat one", quote(env!("CARGO_BIN_EXE_ae")));
            if cfg!(target_os = "macos") {
                runner
                    .args(["-F", "-q"])
                    .arg(root.join("record"))
                    .args(["sh", "-c", &command]);
            } else {
                runner
                    .args(["--flush", "-q", "-e", "-c", &command])
                    .arg(root.join("record"));
            }
        } else {
            runner.args(["chat", "one"]);
        }
        let inherited = std::env::var("PATH").expect("PATH");
        runner
            .env("PATH", format!("{}:{inherited}", self.bin.display()))
            .env("HOME", root)
            .env("AE_HOME", root)
            .env("CONFIG_FILE", root.join("config"))
            .env("TMUX_TMPDIR", root)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", root.join("no-server/tmux.sock"))
            .env("LOOK_CALLS", root.join("calls"))
            .env(
                "LOOK_ANSWER",
                fs::read_to_string(root.join("look")).expect("look"),
            )
            .env(
                "LOOK_ZONE",
                fs::read_to_string(root.join("zone")).expect("zone"),
            )
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .stdin(if tty { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = runner.spawn().expect("chat fixture starts");
        let output = super::cli::bounded(child, Duration::from_secs(20))
            .expect("one-shot exits within 20 s");
        assert!(
            output.status.success(),
            "chat exit: {:?}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let raw = String::from_utf8(output.stdout).expect("chat UTF-8");
        let text = if tty { raw.replace("\r\n", "\n") } else { raw };
        // Readiness comes before the intended colour assertion.
        assert!(
            text.contains("lead-one")
                && text.contains("colead-one")
                && text.contains("human-question"),
            "fixture rows ready: {text:?}"
        );
        text
    }
}

#[test]
fn chat_look_cli_tty_uses_fixed_speaker_colours_bars_dim_time_and_bold_you() {
    let rig = Rig::new("speakers", "darcula", true, true, "+0200");
    let text = rig.run(true);
    let plain = strip_sgr(&text);
    assert!(
        plain.contains('▌'),
        "terminal messages need left bars: {text:?}"
    );
    assert_eq!(
        colour_of(&text, "01:30:00"),
        rgb(ae::theme::Palette::DARCULA.dim),
        "configured darcula dim timestamp"
    );
    let lead = colour_of(&text, "said lead");
    let colead = colour_of(&text, "said colead");
    let human = colour_of(&text, "you → lead");
    assert_ne!(lead, colead, "speakers remain distinct");
    assert_ne!(human, lead, "human remains distinct");
    assert_ne!(human, colead, "human remains distinct");
    assert_eq!(
        colour_of(&text, "said lead"),
        sgr_state_at(
            &text,
            text.match_indices("said lead")
                .nth(1)
                .expect("second lead header")
                .0
        )
        .0
        .expect("second header coloured"),
        "one fixed lead colour"
    );
    let at = text.find("you → lead").expect("human header");
    assert!(sgr_state_at(&text, at).1, "you header bold");
    assert_ne!(
        colour_of(&text, "01:30:00"),
        lead,
        "timestamps dim independently of speaker"
    );
    for marker in ["lead-one", "lead-two", "colead-one", "human-question"] {
        let row = plain
            .lines()
            .find(|row| row.contains(marker))
            .expect("body row");
        assert!(
            row.trim_start().starts_with('▌'),
            "bar on every message line: {row:?}"
        );
    }
}

#[test]
fn chat_look_cli_pipe_keeps_frozen_bytes_and_queries_no_tmux() {
    let rig = Rig::new("pipe", "b", true, true, "+0200");
    let text = rig.run(false);
    assert_eq!(
        text,
        include_str!("../fixtures/console/chat-look-cli-plain.txt")
    );
    let calls = fs::read_to_string(rig.scratch.path().join("calls")).unwrap_or_default();
    assert!(
        !calls.lines().any(|line| line.contains("t/f/")
            || line.contains("ae_palette")
            || line.contains("ae_look")),
        "pipe performs no look or zone query: {calls}"
    );
}

#[test]
fn chat_look_cli_theme_off_keeps_plain_bytes_even_on_tty() {
    let rig = Rig::new("off", "darcula", true, false, "+0200");
    let text = rig.run(true);
    assert_eq!(
        text,
        include_str!("../fixtures/console/chat-look-cli-plain.txt")
    );
    assert!(!text.contains('\x1b'));
}

#[test]
fn chat_look_cli_icons_off_uses_ascii_message_bars() {
    let rig = Rig::new("ascii", "a", false, true, "+0200");
    let text = rig.run(true);
    let plain = strip_sgr(&text);
    assert_eq!(
        colour_of(&text, "01:30:00"),
        rgb(ae::theme::Palette::NEUTRAL.dim),
        "configured neutral dim timestamp"
    );
    assert!(!plain.contains('▌'), "icons off avoids glyph bar");
    let row = plain
        .lines()
        .find(|row| row.contains("lead-one"))
        .expect("lead body");
    assert!(row.trim_start().starts_with('|'), "ASCII left bar: {row:?}");
    assert!(text.contains("\x1b["), "icons off still coloured");
}

#[test]
fn chat_look_cli_neutralises_record_escapes_before_adding_sgr() {
    let rig = Rig::new("inert", "darcula", true, true, "bogus");
    let text = rig.run(true);
    assert!(
        text.contains("\x1b["),
        "tty path is styled, not trivially plain"
    );
    let hostile = "safe �[31mBAD�]52;c;AAAA� tail";
    assert!(
        text.contains(hostile),
        "neutralised body remains contiguous without SGR inside: {text:?}"
    );
    let _ = strip_sgr(&text); // Every remaining escape must be ae SGR.
    assert!(!text.contains('\u{9b}'), "C1 CSI inert too");
}

#[test]
fn chat_look_cli_local_zone_shifts_day_and_is_asked_once() {
    let rig = Rig::new("zone", "darcula", true, true, "+0200");
    let text = rig.run(true);
    let plain = strip_sgr(&text);
    assert!(
        plain.contains("# 2026-10-03 +0200"),
        "local date rolls at midnight: {plain}"
    );
    assert!(plain.contains("01:30:00"), "viewer clock shifted: {plain}");
    let calls = fs::read_to_string(rig.scratch.path().join("calls")).expect("tmux query proof");
    assert_eq!(
        calls.lines().filter(|line| line.contains("t/f/")).count(),
        1,
        "zone read once: {calls}"
    );
}

#[test]
fn chat_look_cli_invalid_zone_keeps_utc_and_never_prints_answer() {
    let rig = Rig::new("bad-zone", "darcula", true, true, "zone-answer\x1b[35m");
    let text = rig.run(true);
    let plain = strip_sgr(&text);
    assert!(
        text.contains("\x1b["),
        "bogus zone still uses styled tty path"
    );
    assert!(
        plain.contains("# 2026-10-02 UTC") && plain.contains("23:30:00"),
        "bogus zone -> UTC: {plain}"
    );
    assert!(
        !plain.contains("zone-answer"),
        "zone answer is not printable content"
    );
}

fn rgb(hex: &str) -> [u8; 3] {
    let channel = |start| u8::from_str_radix(&hex[start..start + 2], 16).expect("palette RGB");
    [channel(1), channel(3), channel(5)]
}
