//! o8 frozen acceptance: the human opens a live dedicated orchestrator,
//! never an ordinary namesake or a stopped session, and resizes its panel.
//! Child of `entry_app_spec`; only external agents are harmless fixture scripts.
//! Oracles: brief-o8 outcomes 1-3 and lead's identity/renamed-role rulings.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private state, fake agents and explicit sockets"
)]

use super::*;
use std::os::unix::fs::PermissionsExt;

fn configured(tag: &str) -> Rig {
    let rig = Rig::new(tag);
    let fake = rig.scratch.join("fake-claude");
    let trace = rig.scratch.join("agent-starts");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ntmux -S {} display-message -p -t \"$TMUX_PANE\" '#{{session_name}}' >> {}\nexec cat >/dev/null\n",
            quote(&rig.socket.display().to_string()),
            quote(&trace.display().to_string())
        ),
    )
    .expect("harmless external agent");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
        .expect("executable fixture agent");
    std::fs::write(
        rig.home.join("config"),
        format!(
            "[clients]\nclaude = {}\n[profiles]\nidle = \"claude\"\n\
         [roster]\nlead = idle\norchestrator = idle\n\
         [workspace]\nmain = lead\nlayout = vertical\nchat = off\nquota = off\n",
            fake.display()
        ),
    )
    .expect("isolated global identity");
    rig.launch("one");
    bounded("the fake ordinary agent started", || {
        starts(&rig, "one") == 1
    });
    rig
}

fn starts(rig: &Rig, name: &str) -> usize {
    std::fs::read_to_string(rig.scratch.join("agent-starts"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == name)
        .count()
}

fn bounded(why: &str, met: impl Fn() -> bool) {
    let until = Instant::now() + WAIT;
    loop {
        if met() {
            return;
        }
        assert!(Instant::now() < until, "{why}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn send(rig: &Rig, pane: &str, text: &str) {
    let (ok, out) = rig.pane_tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    assert!(ok, "private app input: {out}");
}

fn open(rig: &Rig, tag: &str, gate_dir: Option<&Path>) -> (String, PathBuf) {
    let rc = rig.scratch.join(format!("rc-{tag}"));
    let line = command(&[], &rc, true);
    let line = gate_dir.map_or(line.clone(), |dir| {
        format!(
            "AE_TEST_APP_READ_GATE={} {line}",
            quote(&dir.display().to_string())
        )
    });
    let pane = rig.pane("host", &line);
    assert!(
        rig.pane_tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"])
            .0,
        "all geometry oracles use an explicit 160x45 terminal"
    );
    rig.wait(&pane, "the ordinary app loads before O", |screen| {
        lands(screen, "one")
    });
    (pane, rc)
}

fn meta(rig: &Rig, name: &str) -> String {
    std::fs::read_to_string(rig.dir(name).join("meta")).expect("owned record")
}

fn value<'a>(text: &'a str, key: &str) -> &'a str {
    text.lines()
        .find_map(|line| {
            let (found, value) = line.split_once('=')?;
            (found == key).then_some(value)
        })
        .expect("required fixture metadata field")
}

fn launch_seat(rig: &Rig) {
    let ran = rig.run(&["orchestrator", "--no-attach"], "seat");
    assert_eq!(
        ran.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let saved = meta(rig, "orchestrator");
    assert_eq!(value(&saved, "meta_agent"), "true");
    assert_eq!(
        value(&saved, "local_config"),
        rig.home
            .join("orchestrator.config")
            .to_str()
            .expect("fixture path")
    );
    bounded("the dedicated fake agent started", || {
        starts(rig, "orchestrator") == 1
    });
}

fn bars(screen: &str) -> Vec<usize> {
    screen
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .enumerate()
        .filter_map(|(x, c)| (c == '│').then_some(x))
        .collect()
}

fn panel_border(screen: &str) -> Option<usize> {
    let edges = bars(screen);
    (edges.len() == 2).then(|| edges[1])
}

fn panel_text(screen: &str) -> String {
    let from = panel_border(screen).map_or(0, |x| x + 1);
    screen
        .lines()
        .map(|line| line.chars().skip(from).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn no_stopped_panel(screen: &str) {
    let panel = panel_text(screen);
    let header = panel.lines().next().unwrap_or_default();
    assert!(
        !header.to_ascii_lowercase().contains("stopped"),
        "a stopped header is not a live panel: {header}"
    );
    for forbidden in [
        "stopped >7d",
        "Not running: no seat facts.",
        "no orchestrator running",
    ] {
        assert!(
            !panel.contains(forbidden),
            "a status column is not a stopped session: {panel}"
        );
    }
}

fn live(rig: &Rig, pane: &str) -> String {
    rig.wait(
        pane,
        "O starts or resumes and binds the live seat",
        |screen| {
            let shown = panel_text(screen);
            (shown.contains("F full") || shown.contains("F side"))
                && shown.contains("orchestrator")
                && shown.contains("Enter writes")
                && !shown.contains("starting")
                && !shown.contains("Starting")
                && !shown.contains("unproven")
                && !shown.contains("unavailable")
                && !shown.contains("no orchestrator running")
                && rig.has_session("orchestrator")
                && rig
                    .tmux(&[
                        "show-option",
                        "-qv",
                        "-t",
                        "=orchestrator:",
                        "@ae_session_uuid",
                    ])
                    .1
                    .trim()
                    == value(&meta(rig, "orchestrator"), "session_id")
        },
    )
}

fn quit(rig: &Rig, pane: &str, rc: &Path) {
    send(rig, pane, "O");
    assert_eq!(rig.quit(pane, rc), "0");
}

/// Replaces `o7app_spec.rs:865`'s superseded never-starts oracle.
/// The ordinary app itself creates nothing; explicit O starts and pins a seat.
#[test]
fn absent_seat_is_started_by_opening_the_panel() {
    let rig = configured("o8absent");
    let (pane, rc) = open(&rig, "absent", None);
    assert!(!rig.has_session("orchestrator"));
    assert_eq!(rig.recorded(), ["one"]);
    let initial = rig.screen(&pane);
    let summary = initial
        .lines()
        .next()
        .expect("ordinary summary")
        .split('│')
        .next()
        .expect("sidebar")
        .to_owned();
    send(&rig, &pane, "O");
    let screen = live(&rig, &pane);
    no_stopped_panel(&screen);
    let sidebar = screen
        .lines()
        .map(|line| line.split('│').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !sidebar.contains("orchestrator"),
        "a live role has no ordinary sidebar row: {sidebar}"
    );
    assert_eq!(
        screen
            .lines()
            .next()
            .expect("live summary")
            .split('│')
            .next()
            .expect("sidebar"),
        summary,
        "starting the role never changes ordinary session counts"
    );
    let saved = meta(&rig, "orchestrator");
    assert_eq!(value(&saved, "meta_agent"), "true");
    assert_eq!(
        value(&saved, "local_config"),
        rig.home
            .join("orchestrator.config")
            .to_str()
            .expect("fixture path")
    );
    bounded("exactly one fake orchestrator started", || {
        starts(&rig, "orchestrator") == 1
    });
    quit(&rig, &pane, &rc);
}

#[test]
fn stopped_seat_resumes_its_identity_and_is_absent_from_stopped_rows_and_counts() {
    let rig = configured("o8resume");
    launch_seat(&rig);
    let id = value(&meta(&rig, "orchestrator"), "session_id").to_owned();
    let sentinel = rig.dir("orchestrator").join("retained-o8");
    std::fs::write(&sentinel, "kept").expect("retained seat file");
    rig.stop("orchestrator");
    rig.launch("spare");
    rig.stop("spare");
    let (pane, rc) = open(&rig, "resume", None);
    let before = rig.wait(&pane, "only the ordinary stopped row is counted", |s| {
        s.contains("Stopped (1)")
    });
    assert!(!before.contains("Stopped (2)"), "{before}");
    send(&rig, &pane, "O");
    let screen = live(&rig, &pane);
    no_stopped_panel(&screen);
    assert_eq!(value(&meta(&rig, "orchestrator"), "session_id"), id);
    assert_eq!(
        std::fs::read_to_string(sentinel).expect("retained file"),
        "kept"
    );
    send(&rig, &pane, "O");
    send(&rig, &pane, "z");
    let rows = rig.wait(&pane, "ordinary stopped row expands", |s| lands(s, "spare"));
    let sidebar: String = rows
        .lines()
        .map(|line| line.split('│').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!sidebar.contains("orchestrator"), "{sidebar}");
    assert!(sidebar.contains("Stopped (1)"), "{sidebar}");
    assert_eq!(rig.quit(&pane, &rc), "0");
}

#[test]
fn a_launch_failure_shows_the_actual_reason_and_an_explicit_retry_can_recover() {
    let rig = configured("o8failure");
    let config_path = rig.home.join("config");
    let good = std::fs::read_to_string(&config_path).expect("owned config");
    std::fs::write(&config_path, good.replace("orchestrator = idle\n", ""))
        .expect("valid config without the seat binding");
    let (pane, rc) = open(&rig, "failure", None);
    send(&rig, &pane, "OF");
    let failed = rig.wait(&pane, "the launch reason reaches the panel", |screen| {
        // Oracle: the existing refusal in session_launch/preflight.rs:722.
        panel_text(screen).contains("no profile for the seat")
    });
    no_stopped_panel(&failed);
    assert_eq!(
        panel_text(&failed)
            .matches("no profile for the seat")
            .count(),
        1,
        "{failed}"
    );
    assert!(!rig.has_session("orchestrator"));
    std::fs::write(&config_path, good).expect("restore the binding");
    send(&rig, &pane, "OO");
    no_stopped_panel(&live(&rig, &pane));
    quit(&rig, &pane, &rc);
}

#[test]
fn a_seat_that_dies_while_open_stays_stopped_until_an_explicit_reopen() {
    let rig = configured("o8death");
    launch_seat(&rig);
    let id = value(&meta(&rig, "orchestrator"), "session_id").to_owned();
    let (pane, rc) = open(&rig, "death", None);
    send(&rig, &pane, "O");
    live(&rig, &pane);
    rig.stop("orchestrator");
    let stopped = rig.wait(
        &pane,
        "a lost seat becomes one status line with an explicit recovery action",
        |s| panel_text(s).contains("orchestrator stopped; O starts it"),
    );
    let header = panel_text(&stopped)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(!header.to_ascii_lowercase().contains("stopped"), "{header}");
    assert_eq!(
        panel_text(&stopped)
            .matches("orchestrator stopped; O starts it")
            .count(),
        1
    );
    // Observe two wall-clock seconds after the loss has reached the UI.
    // A deliberate stop must not trigger an autonomous start.
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            !rig.has_session("orchestrator"),
            "a lost seat never auto-restarts"
        );
        assert_eq!(starts(&rig, "orchestrator"), 1);
        if Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    send(&rig, &pane, "OO");
    no_stopped_panel(&live(&rig, &pane));
    assert_eq!(value(&meta(&rig, "orchestrator"), "session_id"), id);
    quit(&rig, &pane, &rc);
}

#[test]
fn two_apps_opening_together_bind_one_seat() {
    let rig = configured("o8pair");
    let gate = rig.scratch.join("gates");
    std::fs::create_dir(&gate).expect("decoder gate directory");
    let (first, rc1) = open(&rig, "first", Some(&gate));
    let (second, rc2) = open(&rig, "second", Some(&gate));
    std::fs::write(gate.join("@app-keys"), "").expect("hold both decoders");
    send(&rig, &first, "O");
    send(&rig, &second, "O");
    bounded("both apps reached their decoder gate", || {
        std::fs::read_to_string(gate.join("trace"))
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == "held @app-keys")
            .count()
            >= 2
    });
    std::fs::remove_file(gate.join("@app-keys")).expect("release both opens");
    no_stopped_panel(&live(&rig, &first));
    no_stopped_panel(&live(&rig, &second));
    bounded("the singleton fake orchestrator started", || {
        starts(&rig, "orchestrator") >= 1
    });
    assert_eq!(
        starts(&rig, "orchestrator"),
        1,
        "one launch, not two agents"
    );
    quit(&rig, &first, &rc1);
    quit(&rig, &second, &rc2);
}

/// A valid legacy ordinary record is made from a real ordinary launch.
/// The current CLI reserves the name, so only the historical name is amended.
fn legacy_namesake(rig: &Rig) -> String {
    rig.launch("legacy");
    rig.stop("legacy");
    let old = rig.dir("legacy");
    let new = rig.dir("orchestrator");
    std::fs::rename(&old, &new).expect("historical ordinary namesake");
    let saved = std::fs::read_to_string(new.join("meta")).expect("ordinary metadata");
    assert!(!saved.contains("meta_agent=true"));
    assert_eq!(value(&saved, "session"), "legacy");
    let saved = saved.replace("session=legacy\n", "session=orchestrator\n");
    std::fs::write(new.join("meta"), &saved).expect("legacy name");
    saved
}

#[test]
fn an_ordinary_namesake_stays_listed_has_an_ordinary_menu_and_is_never_started() {
    let rig = configured("o8collision");
    let saved = legacy_namesake(&rig);
    let (pane, rc) = open(&rig, "collision", None);
    rig.wait(&pane, "ordinary namesake counts as stopped", |s| {
        s.contains("Stopped (1)")
    });
    send(&rig, &pane, "z");
    let listed = rig.wait(&pane, "ordinary namesake is listed", |s| {
        lands(s, "orchestrator")
    });
    assert!(listed.contains("orchestrator"));
    send(&rig, &pane, "2m");
    rig.wait(&pane, "namesake has ordinary session actions", |s| {
        s.contains("Activity") && s.contains("Memos")
    });
    send(&rig, &pane, "q");
    rig.wait(&pane, "the ordinary namesake menu closes before O", |s| {
        lands(s, "orchestrator") && !s.contains("a  Activity")
    });
    send(&rig, &pane, "O");
    let refused = rig.wait(&pane, "the panel names the collision reason", |s| {
        let p = panel_text(s);
        p.contains("orchestrator")
            && (p.contains("without the orchestrator role") || p.contains("collision"))
    });
    no_stopped_panel(&refused);
    assert_eq!(meta(&rig, "orchestrator"), saved);
    assert!(!rig.has_session("orchestrator"));
    assert_eq!(starts(&rig, "orchestrator"), 0);
    quit(&rig, &pane, &rc);
}

#[test]
fn a_renamed_role_is_hidden_and_named_with_a_way_to_reach_it_without_starting_any_seat() {
    let rig = configured("o8renamed");
    launch_seat(&rig);
    let renamed = rig.run(&["rename", "orchestrator", "router"], "rename");
    assert_eq!(
        renamed.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&renamed.stderr)
    );
    let saved = meta(&rig, "router");
    assert_eq!(value(&saved, "meta_agent"), "true");
    rig.stop("router");
    rig.launch("spare");
    rig.stop("spare");
    let (pane, rc) = open(&rig, "renamed", None);
    let before = rig.wait(&pane, "only the ordinary stopped row is counted", |s| {
        s.contains("Stopped (1)")
    });
    assert!(!before.contains("Stopped (2)"), "{before}");
    let sidebar: String = before
        .lines()
        .map(|line| line.split('│').next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !sidebar.contains("router"),
        "the role is no ordinary sidebar row: {before}"
    );
    send(&rig, &pane, "OF");
    let refused = rig.wait(
        &pane,
        "renamed role has a specific reachability line",
        |s| {
            let p = panel_text(s);
            p.contains("router") && (p.contains("ae router") || p.contains("ae app router"))
        },
    );
    no_stopped_panel(&refused);
    assert_eq!(meta(&rig, "router"), saved);
    assert!(!rig.has_session("router") && !rig.has_session("orchestrator"));
    assert_eq!(starts(&rig, "orchestrator"), 1);
    quit(&rig, &pane, &rc);
}

fn report(rig: &Rig, pane: &str, code: u8, x: usize, y: usize, end: char) {
    send(rig, pane, &format!("\x1b[<{code};{};{}{end}", x + 1, y + 1));
}

fn drag(rig: &Rig, pane: &str, from: usize, to: usize) {
    report(rig, pane, 0, from, 1, 'M');
    report(rig, pane, 32, to, 1, 'M');
    report(rig, pane, 0, to, 1, 'm');
}

/// Both visible chat columns keep at least 40 columns while a panel edge is dragged.
#[test]
fn the_drawn_panel_border_drags_clamps_and_keeps_its_width_across_visibility_and_size_changes() {
    let rig = configured("o8drag");
    launch_seat(&rig);
    let (pane, rc) = open(&rig, "drag", None);
    send(&rig, &pane, "O");
    let before = live(&rig, &pane);
    let start = panel_border(&before).expect("side panel border");
    let sidebar = bars(&before)[0];
    let target = start + 8;
    drag(&rig, &pane, start, target);
    rig.wait(&pane, "the drawn border follows a real mouse drag", |s| {
        panel_border(s) == Some(target)
    });
    for keys in ["OO", "FF"] {
        send(&rig, &pane, keys);
        rig.wait(
            &pane,
            "panel width survives a mode or visibility round trip",
            |s| panel_border(s) == Some(target),
        );
    }
    assert!(
        rig.pane_tmux(&["resize-window", "-t", &pane, "-x", "100", "-y", "45"])
            .0
    );
    rig.wait(&pane, "narrow fallback shows the focused panel", |s| {
        s.contains("side needs") && s.contains("orchestrator")
    });
    assert!(
        rig.pane_tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"])
            .0
    );
    rig.wait(&pane, "widening restores the dragged width", |s| {
        panel_border(s) == Some(target)
    });
    drag(&rig, &pane, target, 0);
    let left = rig.wait(
        &pane,
        "left drag clamps while keeping both columns usable",
        |s| panel_border(s).is_some_and(|x| x < target && x >= sidebar + 1 + 40 && 160 - x >= 40),
    );
    let left = panel_border(&left).expect("clamped side panel");
    drag(&rig, &pane, left, 159);
    let right = rig.wait(
        &pane,
        "right drag clamps while keeping both columns usable",
        |s| panel_border(s).is_some_and(|x| x > left && x >= sidebar + 1 + 40 && 160 - x >= 40),
    );
    assert_eq!(
        bars(&right)[0],
        sidebar,
        "panel dragging never moves the sidebar"
    );
    quit(&rig, &pane, &rc);
}

/// GUARD: these controls must be GREEN on the base before the RED cases count.
#[test]
fn guard_real_fixture_panel_borders_menu_keys_and_role_rename_are_reachable() {
    let rig = configured("o8guard");
    launch_seat(&rig);
    rig.launch("legacy");
    rig.stop("legacy");
    let (pane, rc) = open(&rig, "guard", None);
    send(&rig, &pane, "O");
    let shown = live(&rig, &pane);
    let edges = bars(&shown);
    assert_eq!(
        edges.len(),
        2,
        "line 1 exposes the sidebar and panel borders: {shown}"
    );
    assert!(edges[0] < edges[1]);
    send(&rig, &pane, "Oz");
    rig.wait(
        &pane,
        "the ordinary second row is painted before its menu",
        |s| lands(s, "legacy"),
    );
    send(&rig, &pane, "2m");
    rig.wait(&pane, "2m opens an ordinary second-row menu", |s| {
        s.contains("Activity") && s.contains("Memos")
    });
    send(&rig, &pane, "q");
    rig.wait(&pane, "one q closes only the ordinary menu", |s| {
        lands(s, "legacy") && !s.contains("a  Activity")
    });
    assert!(!rc.exists(), "one menu q never quits the app");
    assert_eq!(rig.quit(&pane, &rc), "0");
    let renamed = rig.run(&["rename", "orchestrator", "router"], "guard-rename");
    assert_eq!(
        renamed.code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&renamed.stderr)
    );
    assert_eq!(value(&meta(&rig, "router"), "meta_agent"), "true");
}
