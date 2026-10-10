//! Frozen appinstr acceptance: brief-appinstr R1-R6 and lead plan rulings,
//! amended by apppolish R7-R10 (generic rules once, then one line per seat).
//! Oracle: fixture bytes, the template files, and sentences of the leadership
//! texts; the custom-grammar test keeps `render::context_document`.
//! Real private terminal + background reads; no new tab API or process door.

use super::{Rig, WAIT, fs, has_colour_sgr, is_settings, reversed_text};
use std::path::{Path, PathBuf};

fn configured(rig: &Rig, custom: &str) -> PathBuf {
    let file = rig.root.join("recorded-config");
    fs::write(
        &file,
        format!("[prompt]\ninstructions = \"\"\"\n{custom}\n\"\"\"\n"),
    )
    .expect("recorded instructions");
    rig.meta(&format!(
        "work_dir=/recorded/home\nconfig={}\n",
        file.display()
    ));
    file
}

fn open(rig: &Rig, width: u16, height: u16) -> String {
    let (pane, _) = rig.direct(width, height, &rig.root.join("config"));
    rig.ready(&pane);
    rig.settings(&pane);
    rig.tab(&pane, "4", "Instructions");
    rig.wait(
        &pane,
        WAIT,
        "Instructions finishes its background read",
        |s| !s.contains("loading") && body(s).iter().any(|line| !line.is_empty()),
    );
    pane
}

fn body(screen: &str) -> Vec<String> {
    let rows: Vec<_> = screen.lines().collect();
    rows.iter()
        .skip(3)
        .take(rows.len().saturating_sub(4))
        .map(|row| row.trim().to_owned())
        .collect()
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// Pages overlap only at the clamped last page. Join actual rows, removing
/// the longest exact overlap, until an independently supplied oracle is seen.
/// Residual: identical nonblank boundary rows can be mistaken for overlap.
fn collect_until(rig: &Rig, pane: &str, wanted: &str) -> String {
    collect_matching(rig, pane, |text| compact(text).contains(&compact(wanted)))
}

fn collect_matching(rig: &Rig, pane: &str, met: impl Fn(&str) -> bool) -> String {
    let mut screen = rig.wait(
        pane,
        WAIT,
        "Instructions ready before page traversal",
        |s| !s.contains("loading"),
    );
    let mut rows = body(&screen);
    for _ in 0..200 {
        let joined = rows.join("\n");
        if met(&joined) {
            return joined;
        }
        let before = body(&screen);
        rig.keys(pane, "PageDown");
        screen = rig.wait(pane, WAIT, "page advances towards owner output", |s| {
            body(s) != before
        });
        let next = body(&screen);
        let overlap = (0..=rows.len().min(next.len()))
            .rev()
            .find(|count| rows[rows.len() - count..] == next[..*count])
            .expect("zero overlap always exists");
        rows.extend(next.into_iter().skip(overlap));
    }
    panic!("Instructions did not expose the complete render owner output");
}

fn owner(dir: &Path, name: &str, work_dir: &str, slot: &str, files: &[PathBuf]) -> String {
    ae::render::context_document(dir, name, work_dir, slot, files)
}

/// The CORE as the tab shows it: the template file with the seat facts spelled
/// as visible markers.
fn core() -> String {
    include_str!("../../src/render/core.txt")
        .replace("${meta_dir}", "<helpers>")
        .replace("${session}", "<session>")
        .replace("${_owner_line}", "<owner line>")
}

const WORKER: &str = include_str!("../../src/render/worker.txt");
/// Sentences of the leadership texts, which live in render.rs, not a template.
const LEADERSHIP: [&str; 4] = [
    "STATE REASONS: The human decides from this text ALONE",
    "LEADERSHIP PEER: you are one of two EQUAL leads",
    "CHAT TURNS: A turn whose FIRST line",
    "Your owner: the human.",
];
const WORKER_OWNER: &str =
    "if it names no agent of this session (no brief, unverified, or gone), the main seat.";
const LAST_SEAT: &str = "colead · worker.0 · lead · every seat + lead pair";

/// R7: every generic text is on the tab, under its audience, exactly once.
fn generic_rules_once(text: &str) {
    let shown = compact(text);
    for rule in [core().as_str(), WORKER, WORKER_OWNER]
        .into_iter()
        .chain(LEADERSHIP)
    {
        assert!(
            shown.contains(&compact(rule)),
            "R7 generic text missing: {rule}"
        );
    }
    for heading in [
        "Rules for every seat",
        "Rules for workers",
        "Rules for lead pair",
    ] {
        assert_eq!(text.matches(heading).count(), 1, "R7 once: {heading}");
    }
    assert_eq!(text.matches("Helpers live in").count(), 1, "R7 CORE once");
}

fn source_bytes(files: &[PathBuf]) -> Vec<Vec<u8>> {
    files
        .iter()
        .map(|file| fs::read(file).expect("source bytes"))
        .collect()
}

#[test]
fn instructions_fourth_title_is_clickable_at_width_40_and_held_read_never_blocks_keys() {
    let rig = Rig::new("instrkeys");
    configured(&rig, "PRIVATE-INSTRUCTION-RECEIPT");
    let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
    rig.ready(&pane);
    rig.tmux(&["resize-window", "-t", &pane, "-x", "40", "-y", "8"]);
    rig.wait(&pane, WAIT, "minimum frame before Settings", |s| {
        s.lines()
            .next()
            .is_some_and(|line| line.contains("now 40x8"))
    });
    let hold = rig.hold();
    let opened = rig.settings(&pane);
    rig.held();
    let line = opened.lines().nth(1).expect("tab row");
    let column = line
        .find("Instructions")
        .expect("R1 fourth tab at width 40");
    rig.click(&pane, column + "Instructions".len() - 1, 1);
    rig.wait(
        &pane,
        WAIT,
        "clicked cold Instructions remains responsive",
        |s| {
            s.contains("loading")
                && reversed_text(rig.styled(&pane).lines().nth(1).unwrap_or_default())
                    .contains("Instructions")
        },
    );
    rig.tab(&pane, "4", "Instructions");
    assert!(hold.path.exists(), "real settings read still held");
    assert!(!rig.screen(&pane).contains("PRIVATE-INSTRUCTION-RECEIPT"));
    rig.keys(&pane, "Escape");
    rig.wait(&pane, WAIT, "Esc works with instruction read held", |s| {
        !is_settings(s)
    });
    drop(hold);
    rig.settings(&pane);
    rig.tab(&pane, "4", "Instructions");
    rig.wait(&pane, WAIT, "released Instructions header rendered", |s| {
        !s.contains("loading") && body(s).iter().any(|row| row.contains("session:"))
    });
}

#[test]
fn instructions_selected_stopped_session_uses_its_local_source_and_every_roster_owner() {
    let rig = Rig::new("instrselected");
    let home_config = configured(&rig, "HOME-ONLY-INSTRUCTIONS");
    let name = "instruction-stopped";
    rig.stopped(name);
    let dir = rig.root.join("sessions").join(name);
    let global = rig.root.join("selected-global");
    let local = rig.root.join("selected-local");
    fs::write(
        &global,
        "[prompt]\ninstructions = GLOBAL-ONLY-INSTRUCTIONS\n",
    )
    .expect("selected global");
    fs::write(
        &local,
        "[prompt]\ninstructions = SELECTED-LOCAL-INSTRUCTIONS\n",
    )
    .expect("selected local");
    let meta = fs::read_to_string(dir.join("meta"))
        .expect("stopped meta")
        .replace("work_dir=/recorded/home", "work_dir=/recorded/selected")
        .replace(
            &home_config.display().to_string(),
            &global.display().to_string(),
        );
    fs::write(
        dir.join("meta"),
        format!(
            "{meta}local_config={}\nseat.spawned.0=builder\n",
            local.display()
        ),
    )
    .expect("selected three-seat records");
    let files = [global, local.clone()];
    let originals = source_bytes(&files);
    let (pane, _) = rig.direct(160, 40, &rig.root.join("config"));
    let fleet = rig.wait(&pane, WAIT, "stopped session visible", |s| s.contains(name));
    let (row, line) = fleet
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(name))
        .expect("stopped session row");
    rig.click(&pane, line.find(name).expect("name cell"), row);
    rig.wait(&pane, WAIT, "selected stopped session facts", |s| {
        s.lines().nth(1).is_some_and(|line| line.contains(name))
    });
    rig.settings(&pane);
    rig.tab(&pane, "4", "Instructions");
    let first = rig.wait(
        &pane,
        WAIT,
        "selected custom instructions and source",
        |s| s.contains("SELECTED-LOCAL-INSTRUCTIONS") && s.contains("session"),
    );
    assert!(
        first.contains(name) && first.contains("selected-local"),
        "R2 selected source: {first}"
    );
    assert!(
        !first.contains("HOME-ONLY-INSTRUCTIONS") && !first.contains("GLOBAL-ONLY-INSTRUCTIONS")
    );
    let text = collect_until(
        &rig,
        &pane,
        "builder · spawned.0 · worker · every seat + workers",
    );
    generic_rules_once(&text);
    assert!(
        !text.contains("/recorded/selected"),
        "R8 no concrete work dir"
    );
    for (name, slot, role, receives) in [
        ("lead", "main", "lead", "every seat + lead pair"),
        ("colead", "worker.0", "lead", "every seat + lead pair"),
        ("builder", "spawned.0", "worker", "every seat + workers"),
    ] {
        let line = format!("{name} · {slot} · {role} · {receives}");
        assert!(
            text.lines().any(|row| row.trim() == line),
            "R8 missing seat line {line}"
        );
    }
    let head = first.to_ascii_lowercase();
    assert!(
        head.contains("render") && head.contains("now") && head.contains("launch"),
        "R3 render-now residual: {first}"
    );
    rig.literal(&pane, "THIS-MUST-NOT-EDIT-INSTRUCTIONS");
    rig.keys(&pane, "Enter");
    rig.keys(&pane, "Escape");
    rig.wait(&pane, WAIT, "readonly Instructions closes", |s| {
        !is_settings(s)
    });
    assert_eq!(originals, source_bytes(&files));
}

#[test]
fn instructions_global_source_wraps_long_text_and_wheel_and_pages_scroll_its_body() {
    let rig = Rig::new("instrwrap");
    let token = format!("LONG-START-{}-LONG-END", "q".repeat(180));
    configured(&rig, &format!("CUSTOM-BEGIN\n{token}\nCUSTOM-END"));
    let pane = open(&rig, 80, 18);
    let first = rig.screen(&pane);
    assert!(
        first.contains("global") && first.contains("recorded-config"),
        "R2 global source: {first}"
    );
    let top = first.lines().nth(3).expect("body top").to_owned();
    for (down, up) in [("PageDown", "PageUp"), ("Down", "Up")] {
        rig.keys(&pane, down);
        rig.wait(&pane, WAIT, "Instructions scrolls down", |s| {
            s.lines().nth(3) != Some(top.as_str())
        });
        rig.keys(&pane, up);
        rig.wait(&pane, WAIT, "Instructions returns to top", |s| {
            s.lines().nth(3) == Some(top.as_str())
        });
    }
    rig.literal(&pane, "\x1b[<65;2;5M");
    rig.wait(&pane, WAIT, "wheel scrolls Instructions", |s| {
        s.lines().nth(3) != Some(top.as_str())
    });
    rig.literal(&pane, "\x1b[<64;2;5M");
    rig.wait(&pane, WAIT, "wheel returns Instructions", |s| {
        s.lines().nth(3) == Some(top.as_str())
    });
    let text = collect_until(&rig, &pane, LAST_SEAT);
    assert!(
        compact(&text).contains(&token),
        "long unbroken instruction token was clipped"
    );
}

#[test]
fn instructions_empty_local_masks_global_and_reports_none() {
    let rig = Rig::new("instrempty");
    let global = configured(&rig, "SUPPRESSED-GLOBAL-INSTRUCTION");
    let local = rig.root.join("empty-local");
    fs::write(&local, "[prompt]\ninstructions = \"\"\n").expect("empty final declaration");
    rig.meta(&format!(
        "work_dir=/recorded/home\nconfig={}\nlocal_config={}\n",
        global.display(),
        local.display()
    ));
    let pane = open(&rig, 160, 40);
    let first = rig.screen(&pane);
    assert!(
        body(&first)
            .iter()
            .any(|line| line.to_ascii_lowercase().contains("none")),
        "R2 no custom instructions line: {first}"
    );
    let text = collect_until(&rig, &pane, LAST_SEAT);
    assert!(!text.contains("SUPPRESSED-GLOBAL-INSTRUCTION"));
}

#[test]
fn instructions_hostile_config_text_is_visible_inert_and_cannot_colour_cells() {
    let rig = Rig::new("instrhostile");
    configured(&rig, "SAFE-HEAD\x1b[31m\x07\0SAFE-TAIL");
    let pane = open(&rig, 220, 60);
    let screen = rig.wait(
        &pane,
        WAIT,
        "R4 hostile instruction text neutralised",
        |s| s.contains("SAFE-HEAD�[31m��SAFE-TAIL"),
    );
    assert!(
        !screen.chars().any(char::is_control)
            || screen
                .chars()
                .filter(|ch| ch.is_control())
                .all(|ch| ch == '\n')
    );
    assert!(
        !has_colour_sgr(&rig.styled(&pane)),
        "hostile config injected colour"
    );
}

#[test]
fn instructions_keeps_render_owner_duplicate_and_final_newline_grammar() {
    let rig = Rig::new("instrgrammar");
    let global = configured(&rig, "discarded");
    fs::write(
        &global,
        "[prompt]\ninstructions = OLD-VALUE\ninstructions = WINNING-VALUE\ninstructions = UNTERMINATED-LAST-LINE",
    )
    .expect("render grammar fixture");
    let expected = owner(
        &rig.tool.dir,
        &rig.name,
        "/recorded/home",
        "worker.0",
        &[global],
    );
    assert!(expected.contains("WINNING-VALUE"));
    assert!(!expected.contains("OLD-VALUE") && !expected.contains("UNTERMINATED-LAST-LINE"));
    let pane = open(&rig, 160, 40);
    let first = rig.screen(&pane);
    assert!(
        first.contains("WINNING-VALUE"),
        "custom instruction diverges from render owner: {first}"
    );
    assert!(!first.contains("OLD-VALUE") && !first.contains("UNTERMINATED-LAST-LINE"));
    collect_until(&rig, &pane, LAST_SEAT);
}

#[test]
fn instructions_missing_meta_unreadable_config_and_unknown_slot_name_the_gap() {
    for kind in ["meta", "config", "slot"] {
        let rig = Rig::new(&format!("instrgap{kind}"));
        configured(&rig, "KNOWN-INSTRUCTIONS");
        let broken = rig.root.join("unreadable-instructions-config");
        if kind == "config" {
            fs::create_dir(&broken).expect("unreadable config fixture is a directory");
            rig.meta(&format!(
                "work_dir=/recorded/home\nconfig={}\n",
                broken.display()
            ));
        } else if kind == "slot" {
            let bytes = fs::read_to_string(rig.tool.dir.join("meta")).expect("fixture meta");
            fs::write(
                rig.tool.dir.join("meta"),
                format!("{bytes}seat.unknown=mystery\n"),
            )
            .expect("unknown roster slot");
        }
        let (pane, _) = rig.direct(220, 60, &rig.root.join("config"));
        rig.ready(&pane);
        let hold = rig.hold();
        rig.settings(&pane);
        rig.held();
        rig.tab(&pane, "4", "Instructions");
        if kind == "meta" {
            fs::remove_file(rig.tool.dir.join("meta")).expect("remove records while read held");
        }
        drop(hold);
        if kind == "slot" {
            let text = collect_matching(&rig, &pane, |text| {
                text.contains("mystery")
                    && text.lines().any(|line| {
                        line.contains("unknown")
                            && (line.contains("gap")
                                || line.contains("cannot")
                                || line.contains("unusable"))
                    })
            });
            assert!(
                text.lines().any(|line| line.contains("unknown")
                    && (line.contains("gap")
                        || line.contains("cannot")
                        || line.contains("unusable"))),
                "R5 unknown slot named gap: {text}"
            );
            assert!(text.contains("mystery"));
        } else {
            rig.wait(&pane, WAIT, "R5 named session/config gap", |s| {
                !s.contains("loading")
                    && if kind == "meta" {
                        s.contains(&rig.name)
                    } else {
                        s.contains("unreadable-instructions-config")
                    }
            });
            let screen = rig.screen(&pane).to_ascii_lowercase();
            assert!(
                screen.contains("gap")
                    || screen.contains("unreadable")
                    || screen.contains("could not")
                    || screen.contains("no records"),
                "R5 honest named gap: {screen}"
            );
            assert!(
                !screen.contains("custom instructions") || !screen.contains("none"),
                "gap guessed no custom instructions"
            );
        }
    }
}

#[test]
fn instructions_docs_name_selected_sources_and_render_now_residual() {
    let doc = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/app.md"))
        .expect("app documentation");
    let section = doc
        .split("## Settings")
        .nth(1)
        .expect("Settings section")
        .split("\n## ")
        .next()
        .expect("Settings content");
    for word in [
        "Instructions",
        "selected",
        "global",
        "session",
        "launch",
        "render",
    ] {
        assert!(section.contains(word), "R6 Settings docs omit {word}");
    }
}
