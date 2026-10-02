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
    stdin_off: bool,
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
        Self {
            scratch,
            bin,
            stdin_off: false,
        }
    }

    fn run(&self, tty: bool) -> String {
        self.run_mode(tty, false, false)
    }

    fn run_input(&self) -> String {
        self.run_mode(true, true, false)
    }

    fn run_ask(&self) -> String {
        self.run_mode(true, true, true)
    }

    fn type_after_prompt(
        &self,
        child: &mut super::cli::OwnedChild,
    ) -> std::thread::JoinHandle<std::process::ChildStdin> {
        let mut stdin = child.stdin.take().expect("PTY input pipe");
        let record = self.scratch.path().join("record");
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                if fs::read_to_string(&record)
                    .unwrap_or_default()
                    .contains("to lead>")
                {
                    std::io::Write::write_all(&mut stdin, b"fixture-typed-ask\n")
                        .expect("type after ownership prompt");
                    // The join packet keeps the pipe open until bounded() ends.
                    return stdin;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "prompt before typed ask"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    }

    fn run_mode(&self, tty: bool, input: bool, asking: bool) -> String {
        let root = self.scratch.path();
        let mut runner = if tty {
            super::cli::helper_by_name("script")
        } else {
            super::cli::ae()
        };
        if tty {
            let command = if input {
                format!(
                    "exec /usr/bin/perl {} {} chat one --follow --input",
                    quote(&root.join("supervise.pl").to_string_lossy()),
                    quote(env!("CARGO_BIN_EXE_ae"))
                )
            } else if self.stdin_off {
                format!(
                    "exec {} chat one --input </dev/null",
                    quote(env!("CARGO_BIN_EXE_ae"))
                )
            } else {
                format!("exec {} chat one", quote(env!("CARGO_BIN_EXE_ae")))
            };
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
            .env("LOOK_RECORD", root.join("record"))
            .env("LOOK_SOCK", root.join("no-server/tmux.sock"))
            .env("LOOK_ASK", if asking { "on" } else { "off" })
            .env("AE_SEND_DEFER_SEC", "0")
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
        if input {
            runner.env("TMUX_PANE", "%99");
        }
        let mut child = runner.spawn().expect("chat fixture starts");
        let typer = asking.then(|| self.type_after_prompt(&mut child));
        let output = super::cli::bounded(child, Duration::from_secs(20))
            .expect("chat fixture exits within 20 s");
        if let Some(typer) = typer {
            let _stdin = typer.join().expect("readiness-gated typer finishes");
        }
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
    assert_eq!(
        lead,
        rgb(ae::theme::Palette::DARCULA.working),
        "main seat wears lead hue"
    );
    assert_eq!(
        colead,
        rgb(ae::theme::Palette::DARCULA.stale),
        "other pair seat wears colead hue"
    );
    assert_eq!(
        human,
        rgb(ae::theme::Palette::DARCULA.title),
        "human wears title hue"
    );
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

fn styled_printed(palette: ae::theme::Palette) -> Printed {
    Printed::styled(ae::console::view::Style::resolve(
        true,
        Some(ae::theme::Look {
            palette,
            icons: true,
            drawn: true,
            motion: false,
        }),
        Some("+0200"),
        "lead",
    ))
}

#[test]
fn chat_look_print_status_tones_follow_canonical_outcomes_and_system_words() {
    let palette = ae::theme::Palette::NEUTRAL;
    let style = ae::console::view::Style::resolve(
        true,
        Some(ae::theme::Look {
            palette,
            icons: true,
            drawn: true,
            motion: false,
        }),
        None,
        "lead",
    );
    let unknown = ae::console::input::outcome_line(
        &ae::console::submit::Outcome::Unknown("unknown-id".into(), "fixture".into()),
        "lead",
    );
    for (line, expected) in [
        (unknown.as_str(), palette.waiting_agent),
        ("refused: fixture", palette.dead),
        ("closed close-id", palette.dim),
        ("accepting input", palette.dim),
    ] {
        assert_eq!(colour_of(&style.status(line), line), rgb(expected));
    }
}

#[test]
fn chat_look_status_colours_follow_kind_and_attach_after_the_question() {
    use ae::console::submit::Outcome;
    let palette = ae::theme::Palette::NEUTRAL;
    for (outcome, prefix, expected) in [
        (
            Outcome::Sent("status-id".into(), None),
            "sent status-id",
            palette.done,
        ),
        (
            Outcome::Uncertain("status-id".into()),
            "uncertain status-id",
            palette.waiting_agent,
        ),
        (
            Outcome::NotDelivered("status-id".into()),
            "not delivered status-id",
            palette.dead,
        ),
    ] {
        let mut printed = styled_printed(palette);
        printed.outcome(
            "status-id",
            ae::console::input::outcome_line(&outcome, "lead"),
        );
        let lane = Lane {
            items: vec![item(
                Kind::Asked {
                    to: "lead".into(),
                    id: "status-id".into(),
                    uncertain: false,
                },
                "question-before-outcome",
            )],
            coverage: vec![],
        };
        let text = printed.step(&lane, 0, true);
        assert_eq!(
            colour_of(&text, prefix),
            rgb(expected),
            "outcome kind determines tone"
        );
        assert_eq!(
            bar_colour_before(&text, prefix),
            rgb(palette.title),
            "pending outcome keeps human message bar"
        );
        let plain = strip_sgr(&text);
        assert!(
            plain.find("you → lead").expect("ask header")
                < plain.find("question-before-outcome").expect("question")
        );
        assert!(
            plain.find("question-before-outcome").expect("question")
                < plain.find(prefix).expect("status")
        );
        assert_eq!(
            printed.step(&lane, 0, true),
            "",
            "outcome and row shown once"
        );
    }
}

fn bar_colour_before(text: &str, marker: &str) -> [u8; 3] {
    let at = text.find(marker).expect("body marker");
    let row = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let bar = text[row..at].find('▌').expect("message row has bar") + row;
    sgr_state_at(text, bar).0.expect("message bar coloured")
}

#[test]
fn chat_look_pending_flush_keeps_submission_order_and_status_tones() {
    let palette = ae::theme::Palette::WARM;
    let mut printed = styled_printed(palette);
    printed.outcome("z", "sent z".into());
    printed.outcome("a", "uncertain a: check lead pane (prefix H)".into());
    printed.outcome("n", "not delivered n".into());
    let text = printed.flush_outcomes();
    assert_eq!(colour_of(&text, "sent z"), rgb(palette.done));
    assert_eq!(colour_of(&text, "uncertain a"), rgb(palette.waiting_agent));
    assert_eq!(colour_of(&text, "not delivered n"), rgb(palette.dead));
    let plain = strip_sgr(&text);
    assert!(plain.find("sent z") < plain.find("uncertain a"));
    assert!(plain.find("uncertain a") < plain.find("not delivered n"));
    printed.outcome("z", "sent z twice".into());
    assert_eq!(
        printed.flush_outcomes(),
        "",
        "flush never repeats a submission"
    );
}

#[test]
fn chat_look_coverage_closure_and_rebase_use_system_colour_and_safe_text() {
    for palette in [
        ae::theme::Palette::DARCULA,
        ae::theme::Palette::NEUTRAL,
        ae::theme::Palette::WARM,
    ] {
        let mut printed = styled_printed(palette);
        let card = Lane {
            items: vec![item(
                Kind::Card {
                    seat: "lead".into(),
                },
                "card-body",
            )],
            coverage: vec!["gap \x1b[31m hostile".into()],
        };
        let first = printed.step(&card, 0, true);
        assert_eq!(colour_of(&first, "coverage incomplete:"), rgb(palette.dim));
        assert!(
            first.contains("gap �[31m hostile"),
            "coverage safe before style"
        );
        assert_eq!(
            colour_of(&first, "DECISION lead"),
            rgb(palette.working),
            "card keeps lead speaker colour"
        );
        let empty = Lane {
            items: vec![],
            coverage: vec![],
        };
        assert_eq!(
            printed.step(&empty, 0, false),
            "",
            "unread pass closes nothing"
        );
        let closed = printed.step(&empty, 0, true);
        assert_eq!(colour_of(&closed, "-- closed:"), rgb(palette.dim));
        assert_eq!(printed.step(&empty, 0, true), "", "closure once");
        let rewrite = printed.rebase(2, 1);
        assert_eq!(
            colour_of(&rewrite, "-- journal rewritten"),
            rgb(palette.dim)
        );
    }
}

#[test]
fn chat_look_resolved_plain_paths_preserve_default_bytes() {
    let lane = Lane {
        items: vec![item(Kind::Said { who: "lead".into() }, "body")],
        coverage: vec!["gap".into()],
    };
    let plain = Printed::default().step(&lane, 0, true);
    for (tty, drawn) in [(false, true), (true, false)] {
        let look = ae::theme::Look {
            palette: ae::theme::Palette::WARM,
            icons: true,
            drawn,
            motion: false,
        };
        let style = ae::console::view::Style::resolve(tty, Some(look), Some("+0200"), "lead");
        let mut printed = Printed::styled(style);
        assert_eq!(
            printed.step(&lane, 0, true),
            plain,
            "plain path stays UTC and byte-identical"
        );
        printed.outcome("plain-id", "sent plain-id".into());
        assert_eq!(printed.flush_outcomes(), "sent plain-id\n");
        assert_eq!(printed.rebase(2, 1), Printed::default().rebase(2, 1));
    }
}

#[test]
fn chat_look_cli_prompt_and_refusal_use_human_and_dead_palette_colours() {
    let rig = Rig::new("prompt-refusal", "a", true, true, "+0200");
    rig.prepare_input();
    let text = rig.run_input();
    let text = composer_controls_removed(&text);
    assert!(
        text.contains("to lead>"),
        "composer ready before colour assertion"
    );
    assert!(
        text.contains("refused: the kept draft is not a regular file"),
        "refusal path ready"
    );
    let palette = ae::theme::Palette::NEUTRAL;
    assert_eq!(
        colour_of(&text, "to lead>"),
        rgb(palette.title),
        "prompt uses human colour"
    );
    assert_eq!(
        colour_of(&text, "refused:"),
        rgb(palette.dead),
        "actual Print refusal uses red"
    );
    assert_eq!(
        colour_of(&text, "chat: one"),
        rgb(palette.dim),
        "session header uses system colour"
    );
    let _ = strip_sgr(&text);
}

#[test]
fn chat_look_cli_input_off_notice_uses_the_system_colour() {
    let mut rig = Rig::new("input-off", "a", true, true, "+0200");
    rig.stdin_off = true;
    let text = rig.run(true);
    assert!(
        text.contains("input off: stdin is not a terminal"),
        "stdin-off path reached before colour assertion"
    );
    assert_eq!(
        colour_of(&text, "input off:"),
        rgb(ae::theme::Palette::NEUTRAL.dim)
    );
    let _ = strip_sgr(&text);
}

fn composer_controls_removed(text: &str) -> String {
    let mut text = text.to_owned();
    // Only the commander's documented terminal controls may bypass the SGR
    // oracle. This fixture draws one short row; vertical movement is rejected
    // loudly. Message-only tests still reject every non-SGR escape.
    for control in [
        "\x1b[?2004h",
        "\x1b[?2004l",
        "\x1b[?7l",
        "\x1b[?7h",
        "\x1b[K",
        "\x1b[J",
        "\x1b7",
        "\x1b8",
    ] {
        text = text.replace(control, "");
    }
    text.replace('\r', "")
}

#[test]
fn chat_look_cli_typed_ask_keeps_styled_lane_and_red_outcome() {
    let rig = Rig::new("typed-ask", "a", true, true, "+0200");
    rig.prepare_follow(true);
    let text = composer_controls_removed(&rig.run_ask());
    let journal = fs::read_to_string(rig.scratch.path().join("sessions/one/events.jsonl"))
        .expect("typed ask journal");
    assert!(
        journal.contains("delivery-abandoned") && journal.contains("fixture-typed-ask"),
        "real submission recorded before colour assertion"
    );
    assert!(
        text.contains("fixture-typed-ask"),
        "typed ask body reached output"
    );
    assert!(
        text.contains("not delivered "),
        "bounded delivery produced an outcome"
    );
    // The header tag ends with "not delivered"; the following space selects
    // the canonical outcome line, where the request id follows those words.
    let palette = ae::theme::Palette::NEUTRAL;
    assert_eq!(
        colour_of(&text, "not delivered "),
        rgb(palette.dead),
        "typed ask's Lane retains ae SGR"
    );
    assert_eq!(
        bar_colour_before(&text, "not delivered "),
        rgb(palette.title)
    );
    assert!(
        !text.contains("�[38;2;"),
        "ae SGR is never neutralised twice"
    );
    let _ = strip_sgr(&text);
}

fn pane_look_tmux(root: &std::path::Path, tail: &[&str]) -> String {
    let mut args = vec!["-S".to_owned(), root.join("sock").display().to_string()];
    args.extend(tail.iter().map(|arg| (*arg).to_owned()));
    let (ok, out) = super::phase2::run_tmux(&args, root);
    assert!(ok, "private tmux {tail:?}: {out}");
    out
}

fn open_look_composer(tool: &super::deliver::Rig, drawn: bool, body: &str) -> String {
    const UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
    let root = tool.dir.parent().expect("sessions").parent().expect("root");
    let name = tool
        .dir
        .file_name()
        .expect("session")
        .to_str()
        .expect("UTF-8");
    let tmux = |tail: &[&str]| pane_look_tmux(root, tail);
    fs::write(tool.dir.join("meta"), format!(
        "session={name}\nmode=local\nsession_id={UUID}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nseat.worker.0=colead\nagent_bin.worker.0=codex\n",
        root.join("sock").display()
    )).expect("lead pair meta");
    fs::write(root.join("config"), "").expect("private config");
    fs::write(tool.dir.join("console.draft"), body).expect("kept literal draft");
    tmux(&["set-option", "-t", name, "@ae_session_uuid", UUID]);
    tmux(&["set-option", "-t", name, "@ae_main_pane", &tool.pane]);
    tmux(&["set-option", "-p", "-t", &tool.pane, "@ae_agent", "lead"]);
    tmux(&[
        "set-option",
        "-t",
        name,
        "@ae_look",
        if drawn { "on" } else { "off" },
    ]);
    tmux(&["set-option", "-t", name, "@ae_palette", "darcula"]);
    tmux(&["set-option", "-t", name, "@ae_icons", "on"]);
    tmux(&["set-option", "-t", name, "@ae_motion", "off"]);
    tmux(&["set-option", "-t", name, "default-size", "12x60"]);
    let output = super::cli::ae()
        .env("AE_HOME", root)
        .env("CONFIG_FILE", root.join("config"))
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", root.join("sock"))
        .env("TMUX", format!("{},1,0", root.join("sock").display()))
        .env("TMUX_PANE", &tool.pane)
        .arg("_console")
        .output()
        .expect("console opens in private terminal");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let panes = tmux(&[
        "list-panes",
        "-s",
        "-t",
        name,
        "-F",
        "#{pane_id}|#{@ae_console}",
    ]);
    let pane = panes
        .lines()
        .find_map(|row| {
            let (pane, stamp) = row.split_once('|')?;
            (stamp == UUID).then(|| pane.to_owned())
        })
        .expect("stamped console pane");
    assert_eq!(
        tmux(&[
            "display-message",
            "-p",
            "-t",
            &pane,
            "#{pane_width}x#{pane_height}"
        ])
        .trim(),
        "12x60",
        "narrow fixture"
    );
    pane
}

fn narrow_composer_snapshot(drawn: bool) -> (Vec<String>, (usize, usize)) {
    const BODY: &str = "QZXQZXQZXQZXQZXQZ";
    // Equal-length session names keep the headline's physical wrapping equal.
    let tool = super::deliver::Rig::new(if drawn { "lookdrawn" } else { "lookplain" }, "codex", 0);
    let root = tool.dir.parent().expect("sessions").parent().expect("root");
    let tmux = |tail: &[&str]| pane_look_tmux(root, tail);
    let pane = open_look_composer(&tool, drawn, BODY);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let screen = loop {
        let screen = tmux(&["capture-pane", "-p", "-t", &pane]);
        let literal: String = screen
            .chars()
            .filter(|ch| matches!(ch, 'Q' | 'Z' | 'X'))
            .collect();
        if screen.contains("to lead>") && literal == BODY {
            break screen;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "literal draft readiness: {screen}"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    let raw = tmux(&["capture-pane", "-e", "-p", "-t", &pane]);
    if drawn {
        assert_eq!(
            colour_of(&raw, "to lead>"),
            rgb(ae::theme::Palette::DARCULA.title),
            "real prompt painted"
        );
    } else {
        assert!(!raw.contains('\x1b'), "plain comparison has no styling");
    }
    let lines: Vec<_> = screen.lines().collect();
    let first = lines
        .iter()
        .rposition(|row| row.starts_with("to lead>"))
        .expect("composer anchor");
    let rows = lines[first..]
        .iter()
        .map(|row| row.trim_end().to_owned())
        .take_while(|row| !row.is_empty())
        .collect();
    let cursor = tmux(&[
        "display-message",
        "-p",
        "-t",
        &pane,
        "#{cursor_x},#{cursor_y}",
    ]);
    let (x, y) = cursor.trim().split_once(',').expect("cursor coordinates");
    (
        rows,
        (
            x.parse().expect("cursor column"),
            y.parse().expect("cursor row"),
        ),
    )
}

#[test]
fn chat_look_dressed_narrow_pane_keeps_plain_composer_rows_and_cursor() {
    let (plain_rows, plain_cursor) = narrow_composer_snapshot(false);
    let expected = [
        "to lead> QZX",
        "         QZX",
        "         QZX",
        "         QZX",
        "         QZX",
        "         QZ",
    ];
    assert_eq!(
        plain_rows, expected,
        "plain narrow draft uses terminal cells"
    );
    assert_eq!(plain_cursor.0, 11, "nine-cell indent plus final two cells");
    let (drawn_rows, drawn_cursor) = narrow_composer_snapshot(true);
    assert_eq!(
        drawn_rows, plain_rows,
        "SGR cannot add composer cells or rows"
    );
    assert_eq!(
        drawn_cursor, plain_cursor,
        "SGR cannot move physical cursor"
    );
}

impl Rig {
    fn prepare_input(&self) {
        self.prepare_follow(false);
    }

    fn prepare_follow(&self, asking: bool) {
        let root = self.scratch.path();
        if !asking {
            fs::create_dir(root.join("sessions/one/console.draft")).expect("refused kept draft");
        }
        let meta = root.join("sessions/one/meta");
        fs::write(
            &meta,
            format!(
                "{}tmux_server_kind=socket\ntmux_server={}\n",
                fs::read_to_string(&meta).expect("fixture meta"),
                root.join("no-server/tmux.sock").display()
            ),
        )
        .expect("record fixture server");
        fs::write(self.bin.join("tmux"), r#"#!/usr/bin/perl
use strict;
use warnings;
open my $log, '>>', $ENV{LOOK_CALLS} or die $!;
print $log join(' ', @ARGV), "\n";
my $format = $ARGV[-1] // '';
if ((grep { $_ eq 'list-panes' } @ARGV) && $format =~ /ae_console/) { print "%99 | \@9 |  |  | 0199c0de-aaaa-4890-abcd-ef0123456789 | 0 | 0 | 0 | \n"; }
elsif ((grep { $_ eq 'list-panes' } @ARGV) && $format =~ /ae_agent/) { print "%5 | lead\n"; }
elsif (grep { $_ eq 'show-options' } @ARGV) { print "0199c0de-aaaa-4890-abcd-ef0123456789\n"; }
elsif ($format =~ /ae_slot/) { print "main | one | lead | 0199c0de-aaaa-4890-abcd-ef0123456789 | $ENV{LOOK_SOCK}\n"; }
elsif ($format =~ /t\/f\//) { print $ENV{LOOK_ZONE}, "\n"; }
elsif ($format =~ /ae_palette|ae_theme|ae_icons/) { print $ENV{LOOK_ANSWER}, "\n"; }
elsif ($format =~ /pane_width/) { print "80 | 24 | 0\n"; }
elsif (grep { $_ eq 'capture-pane' } @ARGV) { exit 0; }
else { exit 1; }
"#).expect("owned-input tmux fixture");
        fs::write(
            root.join("supervise.pl"),
            r#"use strict;
use warnings;
use POSIX qw(WNOHANG);
use Time::HiRes qw(time sleep);
my $pid = fork();
defined $pid or die "fork: $!";
if (!$pid) { exec @ARGV; die "exec: $!"; }
my $ready = 0;
my $alive = 1;
my $deadline = time() + 8;
while (time() < $deadline) {
    if (open my $record, '<', $ENV{LOOK_RECORD}) {
        local $/;
        my $bytes = <$record> // '';
        my $marker = $ENV{LOOK_ASK} eq 'on' ? 'not delivered ' : 'refused: the kept draft';
        if (index($bytes, 'to lead>') >= 0 && index($bytes, $marker) >= 0) { $ready = 1; last; }
    }
    if (waitpid($pid, WNOHANG) == $pid) { $alive = 0; last; }
    sleep 0.02;
}
if ($alive) { kill 'TERM', $pid; waitpid($pid, 0); }
exit($ready ? 0 : 2);
"#,
        )
        .expect("bounded follow supervisor");
    }
}
