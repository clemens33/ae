//! Frozen apptype acceptance: brief R1–R7, P1–P4 and accepted critique.
//! Oracles are literal fixture bytes, captured terminal cells and the public
//! help. Reuses the private Settings rig; no new process or environment door.

use crate::app_mode::writing;
use std::fs;
use std::time::{Duration, Instant};

use super::{FRAME, Rig, WAIT};

const FINISH: Duration = Duration::from_secs(40);

fn open(tag: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    let (pane, _) = rig.direct(300, 60, &rig.root.join("config"));
    rig.ready(&pane);
    (rig, pane)
}

fn journal(rig: &Rig) -> Vec<u8> {
    fs::read(rig.tool.dir.join("events.jsonl")).expect("private journal")
}

fn alive(rig: &Rig, pane: &str) {
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", pane, "#{alternate_on}"])
            .trim(),
        "1",
        "typing leaves the app on its alternate screen"
    );
}

fn selected_stopped(rig: &Rig, pane: &str) {
    rig.stopped("zzzstopped");
    rig.wait(pane, WAIT, "GUARD stopped card loaded", |s| {
        s.contains("zzzstopped")
    });
    rig.keys(pane, "2");
    rig.wait(pane, WAIT, "GUARD stopped selection", |s| {
        s.contains("read-only") && s.contains("zzzstopped is stopped")
    });
}

/// Plan §5(c), accepted I1. Independent table: priority prefix, displayed in
/// navigation order. No implementation hint helper is called.
pub(super) fn expected_browse(width: u16, writable: bool) -> String {
    let priority = [
        "? help",
        "1-9 session",
        "Enter write",
        "qq quit",
        "! next need",
        "Tab overview / agents",
        "j/k move",
        "PgUp/PgDn scroll",
        "Esc home",
        "s settings",
    ];
    let order = [1, 6, 4, 5, 7, 2, 8, 9, 0, 3];
    let mut chosen = Vec::new();
    let mut used = 8;
    for (index, item) in priority.iter().enumerate() {
        if index == 2 && !writable {
            continue;
        }
        let next = used + 3 + item.len();
        if next > usize::from(width) {
            break;
        }
        chosen.push(index);
        used = next;
    }
    let words = order
        .iter()
        .filter(|index| chosen.contains(index))
        .map(|index| priority[*index])
        .collect::<Vec<_>>();
    format!("  browse   {}", words.join("   "))
}

#[test]
fn browse_paste_enters_write_with_literal_controls_and_no_action() {
    let (rig, pane) = open("typepaste");
    let before = journal(&rig);
    let seat = rig.screen(&rig.tool.pane);
    let payload = "/close\nq s 9 !\nPASTE-END";
    rig.literal(&pane, &format!("\x1b[200~{payload}\x03\x1b[201~"));
    let shown = rig.wait(&pane, FRAME, "R1 paste becomes a draft", |s| {
        writing(s) && s.contains("PASTE-END")
    });
    assert!(shown.contains("/close") && shown.contains("q s 9 !"));
    alive(&rig, &pane);
    assert_eq!(journal(&rig), before, "paste never executes an action");
    assert_eq!(
        rig.screen(&rig.tool.pane),
        seat,
        "no paste key reaches a seat"
    );
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "R1 Esc retains the pasted draft", |s| {
        s.contains("draft kept") && s.contains("PASTE-END")
    });
}

#[test]
fn a_split_paste_preserves_every_fragment_and_the_submitted_literal_bytes() {
    let (rig, pane) = open("typesplit");
    rig.literal(&pane, "\x1b[200~PASTE-FIRST");
    rig.wait(&pane, FRAME, "R1 first fragment enters writing", |s| {
        s.contains("PASTE-FIRST") && writing(s)
    });
    rig.literal(&pane, "\nPASTE-MIDDLE");
    rig.wait(&pane, FRAME, "B1 second read keeps the paste origin", |s| {
        s.contains("PASTE-MIDDLE")
    });
    rig.literal(&pane, "\nPASTE-LAST\x1b[201~");
    rig.wait(&pane, FRAME, "B1 third read preserves the tail", |s| {
        s.contains("PASTE-FIRST") && s.contains("PASTE-MIDDLE") && s.contains("PASTE-LAST")
    });
    assert!(journal(&rig).is_empty(), "no action before explicit Enter");
    rig.keys(&pane, "Enter");
    let path = rig.tool.dir.join("console.draft");
    let until = Instant::now() + FRAME;
    while !path.is_file() {
        assert!(
            Instant::now() < until,
            "GUARD submit persisted literal draft"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        fs::read(path).expect("submitted bytes"),
        b"PASTE-FIRST\nPASTE-MIDDLE\nPASTE-LAST",
        "B1 display and submit preserve the whole paste"
    );
}

#[test]
fn read_only_paste_names_the_refusal_and_never_writes() {
    let (rig, pane) = open("typero");
    selected_stopped(&rig, &pane);
    let home = journal(&rig);
    let stopped = rig.root.join("sessions/zzzstopped");
    rig.literal(&pane, "\x1b[200~q s PASTE-REFUSED\x1b[201~");
    let shown = rig.wait(&pane, FRAME, "R1 one visible paste refusal", |s| {
        s.contains("paste not taken") && s.contains("stopped")
    });
    assert_eq!(shown.matches("paste not taken").count(), 1);
    assert!(!shown.contains("PASTE-REFUSED") && !writing(&shown));
    assert_eq!(journal(&rig), home);
    assert!(!stopped.join("console.draft").exists());
    alive(&rig, &pane);
}

#[test]
fn paste_refused_by_a_live_writer_lease_has_one_visible_reason() {
    let (rig, pane) = open("typepastebusy");
    let witness = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(rig.tool.dir.join(".console-writer.lock"))
        .expect("fixture writer lease");
    witness
        .lock()
        .expect("GUARD another app owns the writer lease");
    rig.literal(&pane, "\x1b[200~BUSY-PASTE\x1b[201~");
    let shown = rig.wait(&pane, FRAME, "R1 writer-lease refusal is visible", |s| {
        s.contains("paste not taken") && s.contains("writing to")
    });
    assert_eq!(shown.matches("paste not taken").count(), 1);
    assert!(!writing(&shown));
    assert!(journal(&rig).is_empty());
    alive(&rig, &pane);
}

#[test]
fn stopped_hint_offers_the_resume_command_without_prefix_h() {
    let (rig, pane) = open("typehint");
    selected_stopped(&rig, &pane);
    let shown = rig.wait(
        &pane,
        FRAME,
        "R4(a) stopped hint names true recovery",
        |s| s.contains("ae zzzstopped resumes it"),
    );
    assert!(
        !shown.contains("prefix h"),
        "stopped hint must not offer the host chat"
    );
}

#[test]
fn q_arms_any_other_key_disarms_and_only_a_second_q_quits() {
    let (rig, pane) = open("typequit");
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 q visibly arms", |s| {
        s.contains("q again to quit")
    });
    alive(&rig, &pane);
    rig.literal(&pane, "x");
    rig.wait(&pane, FRAME, "R2 unbound x disarms", |s| {
        !s.contains("q again to quit")
    });
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 q after another key only arms", |s| {
        s.contains("q again to quit")
    });
    alive(&rig, &pane);
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 second q ends the app", |s| {
        s.contains("ae app closed")
    });
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &pane, "#{alternate_on}"])
            .trim(),
        "0"
    );
}

#[test]
fn q_arm_expires_and_a_late_q_only_arms_again() {
    let (rig, pane) = open("typeexpire");
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 first q arm", |s| {
        s.contains("q again to quit")
    });
    rig.wait(
        &pane,
        Duration::from_secs(4),
        "R2 short arm expires on idle",
        |s| !s.contains("q again to quit"),
    );
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 late q re-arms", |s| {
        s.contains("q again to quit")
    });
    alive(&rig, &pane);
}

#[test]
fn quick_in_one_read_keeps_the_app_up_and_the_old_key_origin_guard() {
    let (rig, pane) = open("typequick");
    let before = journal(&rig);
    let seat = rig.screen(&rig.tool.pane);
    rig.literal(&pane, "quick");
    let shown = rig.wait(&pane, FRAME, "R2 quick keeps the app and B4 guard", |s| {
        writing(s) && s.contains("dropped the keys typed before writing started")
    });
    assert!(!shown.contains("q again to quit"));
    assert_eq!(journal(&rig), before);
    assert_eq!(
        rig.screen(&rig.tool.pane),
        seat,
        "R2 no letters go to a seat"
    );
    alive(&rig, &pane);
}

#[test]
fn quick_key_by_key_keeps_ck_as_the_draft() {
    let (rig, pane) = open("typetyped");
    rig.literal(&pane, "q");
    rig.wait(&pane, FRAME, "R2 q arm", |s| s.contains("q again to quit"));
    rig.literal(&pane, "u");
    rig.wait(&pane, FRAME, "R2 u disarm", |s| {
        !s.contains("q again to quit")
    });
    rig.literal(&pane, "i");
    rig.wait(
        &pane,
        FRAME,
        "GUARD i lease acquired before remaining keys",
        writing,
    );
    rig.literal(&pane, "c");
    rig.literal(&pane, "k");
    rig.wait(
        &pane,
        FRAME,
        "R2 quick typed across reads has ck draft",
        |s| {
            s.lines()
                .any(|line| line.contains("› lead") && line.trim_end().ends_with("ck"))
        },
    );
    assert!(journal(&rig).is_empty());
    alive(&rig, &pane);
}

fn word_on_a_stopped_selection(word: &str) {
    let (rig, pane) = open(&format!("typero{word}"));
    selected_stopped(&rig, &pane);
    let before = journal(&rig);
    let seat = rig.screen(&rig.tool.pane);
    rig.literal(&pane, word);
    // The existing k/Up in quick moves from stopped to the preceding home.
    // Pin both the expected selection and its exact Agents body.
    let (selected, agents) = if word == "quick" {
        (rig.name.as_str(), "No seat facts: on another tmux server.")
    } else {
        ("zzzstopped", "Not running: no seat facts.")
    };
    // Tab is a later visible effect, proving the word was processed.
    rig.keys(&pane, "Tab");
    rig.wait(&pane, FRAME, "R2 word processed before Agents", |s| {
        s.lines().nth(1).is_some_and(|row| row.contains(selected)) && s.contains(agents)
    });
    alive(&rig, &pane);
    assert_eq!(journal(&rig), before);
    assert_eq!(rig.screen(&rig.tool.pane), seat);
    assert!(!rig.root.join("sessions/zzzstopped/console.draft").exists());
}

#[test]
fn quick_on_a_stopped_selection_keeps_app_and_seats_untouched() {
    word_on_a_stopped_selection("quick");
}

#[test]
fn hello_on_a_stopped_selection_keeps_app_and_seats_untouched() {
    word_on_a_stopped_selection("hello");
}

#[test]
fn nonempty_draft_interrupt_arms_and_an_edit_disarms_it() {
    let (rig, pane) = open("typectrlc");
    rig.keys(&pane, "Enter");
    rig.wait(&pane, FRAME, "GUARD writing before draft", writing);
    rig.literal(&pane, "DRAFT-KEPT");
    rig.wait(&pane, FRAME, "GUARD draft drawn before interrupt", |s| {
        s.contains("DRAFT-KEPT")
    });
    rig.keys(&pane, "C-c");
    rig.wait(
        &pane,
        FRAME,
        "P1 first interrupt protects nonempty draft",
        |s| s.contains("^C again to quit") && s.contains("DRAFT-KEPT"),
    );
    alive(&rig, &pane);
    rig.literal(&pane, "-EDIT");
    rig.wait(&pane, FRAME, "P1 edit disarms and preserves draft", |s| {
        s.contains("DRAFT-KEPT-EDIT") && !s.contains("^C again to quit")
    });
    rig.keys(&pane, "C-c");
    rig.wait(&pane, FRAME, "P1 fresh interrupt only arms", |s| {
        s.contains("^C again to quit")
    });
    rig.keys(&pane, "C-c");
    rig.wait(&pane, FRAME, "P1 second interrupt quits", |s| {
        s.contains("ae app closed")
    });
}

#[test]
fn tmux_exit_restores_normal_screen_and_prints_one_plain_restart_line() {
    let (rig, pane) = open("typeexit");
    rig.keys(&pane, "C-c");
    let shown = rig.wait(&pane, FRAME, "R3 tmux restart hint", |s| {
        s.contains("ae app closed")
    });
    assert_eq!(shown.matches("ae app closed").count(), 1);
    assert!(shown.contains("run: ae app (in a chat window, prefix h reopens it)"));
    assert_eq!(
        rig.tmux(&["display-message", "-p", "-t", &pane, "#{alternate_on}"])
            .trim(),
        "0"
    );
    assert!(
        !rig.styled(&pane).contains('\x1b'),
        "R3 line is plain after terminal restore"
    );
}

#[test]
fn outside_tmux_exit_hint_contains_no_tmux_shortcut() {
    let rig = Rig::new("typeoutside");
    let command = format!(
        "env -u TMUX -u TMUX_PANE -u CLAUDE_CONFIG_DIR -u CODEX_HOME HOME={} AE_HOME={} CONFIG_FILE={} AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER={} {} app {}; sleep 30",
        super::quote(&rig.root.to_string_lossy()),
        super::quote(&rig.root.to_string_lossy()),
        super::quote(&rig.root.join("config").to_string_lossy()),
        super::quote(&rig.socket.to_string_lossy()),
        super::quote(env!("CARGO_BIN_EXE_ae")),
        super::quote(&rig.name),
    );
    let pane = rig
        .tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            &rig.name,
            &command,
        ])
        .trim()
        .to_owned();
    rig.ready(&pane);
    rig.keys(&pane, "C-c");
    let shown = rig.wait(&pane, FRAME, "R3 outside restart hint", |s| {
        s.contains("ae app closed")
    });
    assert_eq!(shown.matches("ae app closed").count(), 1);
    assert!(shown.contains("ae app closed - run: ae app"));
    assert!(!shown.contains("prefix") && !shown.contains("tmux") && !shown.contains("chat window"));
}

#[test]
fn keys_row_keeps_help_at_floor_and_all_browse_keys_at_full_width() {
    for width in [40, 60, 89, 160, 300] {
        let look = ae::theme::Look::read("off", "", "off", "off");
        let buf = super::closed_frame(width, 28, &look, true);
        let row = super::row(&buf, 27);
        assert!(
            row.starts_with(&expected_browse(width, true)),
            "R4(c) priority {width}: {row}"
        );
        assert!(row.contains("? help"), "R4(c) help fits at the floor");
        assert!(!row.contains("   q quit"));
    }
}

fn collect_help(rig: &Rig, pane: &str) -> String {
    let mut text = String::new();
    let mut previous = String::new();
    for _ in 0..80 {
        let shown = rig.screen(pane);
        text.push_str(&shown);
        if shown == previous {
            return text;
        }
        previous = shown;
        rig.keys(pane, "PageDown");
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("help never reached a scroll bound");
}

#[test]
fn question_opens_one_read_only_keys_overlay_with_every_existing_key() {
    let (rig, pane) = open("typehelp");
    let before = journal(&rig);
    rig.literal(&pane, "?");
    rig.wait(&pane, FRAME, "R4(d) question opens Keys overlay", |s| {
        super::is_settings(s) && s.lines().nth(1).is_some_and(|row| row.contains("Keys"))
    });
    let text = collect_help(&rig, &pane);
    for key in [
        "Browse",
        "Write",
        "Settings",
        "Held",
        "1-9",
        "!",
        "j/k",
        "Tab",
        "qq",
        "oo",
        "n/p",
        "PgUp",
        "PgDn",
        "Esc",
        "Enter",
        "s",
        "?",
        "^C",
        "Left",
        "Right",
        "Home",
        "End",
        "^A",
        "^E",
        "^U",
        "Backspace",
        "Delete",
        "@",
        "/close",
        "/open",
        "paste",
        "1-5",
    ] {
        assert!(text.contains(key), "R4(d) help omits {key}: {text}");
    }
    rig.literal(&pane, "q s QUICK-OVERLAY");
    assert_eq!(journal(&rig), before, "help does not submit an ask");
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "R4(d) Esc closes help", |s| {
        !super::is_settings(s)
    });
}

#[test]
fn narrow_help_is_reachable_scrollable_and_closes_without_a_draft() {
    let (rig, pane) = open("typehelpfloor");
    rig.tmux(&["resize-window", "-t", &pane, "-x", "40", "-y", "8"]);
    rig.wait(&pane, FRAME, "GUARD minimum frame", |s| {
        s.contains("now 40x8")
    });
    rig.literal(&pane, "?");
    rig.wait(&pane, FRAME, "R4(d) Keys reachable at 40x8", |s| {
        super::is_settings(s) && s.lines().nth(1).is_some_and(|row| row.contains("Keys"))
    });
    let text = collect_help(&rig, &pane);
    for section in ["Browse", "Write", "Settings"] {
        assert!(text.contains(section), "minimum help can reach {section}");
    }
    rig.keys(&pane, "Escape");
    rig.wait(&pane, FRAME, "R4(d) minimum help closes", |s| {
        !super::is_settings(s)
    });
    assert!(journal(&rig).is_empty());
}

#[test]
fn ask_in_flight_names_target_then_outcome_is_visible_exactly_once() {
    let (rig, pane) = open("typeflight");
    rig.keys(&pane, "Enter");
    rig.wait(&pane, FRAME, "GUARD acquired writer before ask", |s| {
        writing(s)
    });
    rig.literal(&pane, "FLIGHT-BODY");
    rig.wait(&pane, FRAME, "GUARD whole body drawn", |s| {
        s.contains("FLIGHT-BODY")
    });
    rig.keys(&pane, "Enter");
    let shown = rig.wait(
        &pane,
        FRAME,
        "R4(f) in-flight line before blocking delivery",
        |s| s.contains("sending to lead"),
    );
    assert_eq!(shown.matches("sending to lead").count(), 1);
    let done = rig.wait(&pane, FINISH, "R4(f) eventual delivery outcome", |s| {
        s.contains("not delivered") && !s.contains("sending to lead")
    });
    assert_eq!(
        done.matches("not delivered").count(),
        1,
        "R4(f) one outcome before refresh"
    );
    // The periodic settled read must not later add the same outcome again.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        rig.screen(&pane).matches("not delivered").count(),
        1,
        "R4(f) one outcome after refresh"
    );
}

#[test]
fn refused_delivery_outcome_is_not_duplicated_by_a_notice_or_refresh() {
    let (rig, pane) = open("typeoutcome");
    rig.keys(&pane, "Enter");
    rig.wait(&pane, FRAME, "GUARD writer before refused delivery", |s| {
        writing(s)
    });
    rig.literal(&pane, "OUTCOME-BODY");
    rig.wait(&pane, FRAME, "GUARD complete outcome body drawn", |s| {
        s.contains("OUTCOME-BODY")
    });
    rig.keys(&pane, "Enter");
    let done = rig.wait(&pane, FINISH, "R4(f) refused outcome read", |s| {
        s.contains("not delivered") && !s.contains("sending to lead")
    });
    assert_eq!(
        done.matches("not delivered").count(),
        1,
        "R4(f) journal and app notice must not duplicate the outcome"
    );
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(rig.screen(&pane).matches("not delivered").count(), 1);
}

#[test]
fn kept_draft_banner_names_target_open_keys_without_host_prefix_h() {
    let rig = Rig::new("typekept");
    rig.stopped("zzzforeign");
    let foreign = rig.root.join("sessions/zzzforeign");
    let meta = fs::read_to_string(foreign.join("meta")).expect("foreign fixture meta");
    let id = meta
        .lines()
        .find_map(|line| line.strip_prefix("session_id="))
        .expect("foreign id");
    rig.tmux(&["new-session", "-d", "-s", "zzzforeign", "sleep 600"]);
    rig.tmux(&["set-option", "-t", "zzzforeign", "@ae_session_uuid", id]);
    fs::write(foreign.join("events.jsonl"), "").expect("foreign journal");
    fs::write(foreign.join("console.draft"), "KEPT-FOREIGN-DRAFT").expect("fixture draft");
    let (pane, _) = rig.direct(300, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.wait(&pane, WAIT, "GUARD running foreign card", |s| {
        s.contains("zzzforeign")
    });
    rig.keys(&pane, "2");
    rig.wait(&pane, WAIT, "GUARD foreign input selection", |s| {
        s.contains("to zzzforeign (not home)") && s.contains("Enter writes")
    });
    rig.keys(&pane, "Enter");
    let shown = rig.wait(&pane, FRAME, "GUARD existing draft restored", |s| {
        s.contains("Kept line, maybe already sent") && s.contains("KEPT-FOREIGN-DRAFT")
    });
    assert!(
        !shown.contains("prefix H"),
        "R4(b) host pair shortcut is false for foreign selection"
    );
    assert!(
        shown.contains("Agents") && shown.contains("oo"),
        "R4(b) true target inspection keys: {shown}"
    );
}

#[test]
fn cli_help_names_the_selected_sessions_lead_pair() {
    let out = super::super::cli::ae()
        .arg("help")
        .output()
        .expect("ae help");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains("selected session's lead pair"),
        "R4(e) help is stale: {text}"
    );
    assert!(!text.contains("only the home session's lead pair"));
}
