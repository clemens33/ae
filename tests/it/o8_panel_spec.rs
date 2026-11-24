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
         [workspace]\nmain = lead\nlayout = vertical\nchat = off\nquota = off\nrestore = off\n",
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

/// `key` reaches an open modal only when a frame showing it was written before the app read
/// the key, and the app stamps that frame once its write returns, a moment after the screen
/// already shows the modal. A key sent in that gap is refused with the hint row's "press
/// again", which is the contract, so the key is pressed again on that refusal and on nothing
/// else: a key that closed the modal is never sent twice.
fn press(rig: &Rig, pane: &str, key: &str, why: &str, met: impl Fn(&str) -> bool) {
    let last = std::cell::Cell::new(Instant::now());
    send(rig, pane, key);
    rig.wait(pane, why, |screen| {
        if met(screen) {
            return true;
        }
        if screen.contains("was not on screen yet - press again")
            && last.get().elapsed() > Duration::from_millis(250)
        {
            send(rig, pane, key);
            last.set(Instant::now());
        }
        false
    });
}

fn open(rig: &Rig, tag: &str, gate_dir: Option<&Path>) -> (String, PathBuf) {
    open_with(rig, tag, gate_dir, &[])
}

/// [`open`] with the app's own operands: a second app in one home is `app --new`.
fn open_with(rig: &Rig, tag: &str, gate_dir: Option<&Path>, args: &[&str]) -> (String, PathBuf) {
    let rc = rig.scratch.join(format!("rc-{tag}"));
    let line = command(args, &rc, true);
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
    let (second, rc2) = open_with(&rig, "second", Some(&gate), &["app", "--new"]);
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
    press(
        &rig,
        &pane,
        "q",
        "the ordinary namesake menu closes before O",
        |s| lands(s, "orchestrator") && !s.contains("a  Activity"),
    );
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
    press(
        &rig,
        &pane,
        "q",
        "one q closes only the ordinary menu",
        |s| lands(s, "legacy") && !s.contains("a  Activity"),
    );
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

/// An older seat overlay can print an identity-section note before launch
/// refuses. Outcome 1 requires the refusal itself in the panel.
#[test]
fn a_legacy_overlay_note_never_masks_the_actual_launch_refusal() {
    let rig = configured("o8warning");
    let config_path = rig.home.join("config");
    let good = std::fs::read_to_string(&config_path).expect("owned config");
    std::fs::write(&config_path, good.replace("orchestrator = idle\n", ""))
        .expect("valid config without the seat binding");
    std::fs::write(
        rig.home.join("orchestrator.config"),
        "[workspace]\nmain = orchestrator\nworkers = \"\"\nlayout = vertical\n\
         orchestrator = true\n[profiles]\nold = \"claude\"\n[roster]\norchestrator = old\n",
    )
    .expect("an older canonical overlay with ignored identity sections");
    let (pane, rc) = open(&rig, "warning", None);
    send(&rig, &pane, "OF");
    let failed = rig.wait(
        &pane,
        "the refusal survives an earlier launch note",
        |screen| {
            // Oracle: the refusal in session_launch/preflight.rs:722, after the
            // informational identity-section note in adopt_recorded_config.
            panel_text(screen).contains("no profile for the seat")
        },
    );
    no_stopped_panel(&failed);
    assert!(!panel_text(&failed).contains("ignoring"), "{failed}");
    assert!(!rig.has_session("orchestrator"));
    assert_eq!(starts(&rig, "orchestrator"), 0);
    quit(&rig, &pane, &rc);
}

/// The app has a real ordinary session on its private host server, while its
/// canonical launch server has never existed. This keeps the app landing real
/// without allowing its fixture host to prove the launch server alive.
fn cold_configured(tag: &str) -> Rig {
    let mut rig = Rig::split(tag);
    let fake = rig.scratch.join("fake-claude");
    let trace = rig.scratch.join("agent-starts");
    // Every fake is launched by one of this Rig's two private tmux servers.
    // Inherited TMUX keeps this read-only query on the agent's own server.
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ntmux display-message -p -t \"$TMUX_PANE\" '#{{session_name}}' >> {}\nexec cat >/dev/null\n",
            quote(&trace.display().to_string())
        ),
    )
    .expect("harmless external agent reads its own inherited tmux server");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
        .expect("executable fixture agent");
    std::fs::write(
        rig.home.join("config"),
        format!(
            "[clients]\nclaude = {}\n[profiles]\nidle = \"claude\"\n\
             [roster]\nlead = idle\norchestrator = idle\n\
             [workspace]\nmain = lead\nlayout = vertical\nchat = off\nquota = off\nrestore = off\n",
            fake.display()
        ),
    )
    .expect("isolated global identity");
    let cold = rig.socket.clone();
    rig.socket.clone_from(&rig.panes);
    rig.launch("one");
    bounded(
        "the ordinary agent runs only on the private host server",
        || starts(&rig, "one") == 1,
    );
    rig.socket = cold;
    assert!(
        !rig.socket.exists(),
        "setup: the canonical launch socket has never existed"
    );
    rig
}

fn strict_launch_absence(rig: &Rig) -> ae::tmux::StopProbe {
    let server = ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(rig.socket.clone()));
    ae::transport::verify_session_absent(&server, "orchestrator")
}

/// Outcome 1 and the cold-Start ruling: O creates a missing canonical server.
/// The strict stop/end proof remains Unknown even though Start may proceed.
#[test]
fn a_cold_launch_server_is_created_when_the_panel_starts_the_seat() {
    let rig = cold_configured("o8cold");
    assert_eq!(
        strict_launch_absence(&rig),
        ae::tmux::StopProbe::Unknown,
        "a missing socket is still Unknown to the strict destructive proof"
    );
    let (pane, rc) = open(&rig, "cold", None);
    assert!(
        !rig.socket.exists(),
        "the ordinary app landing does not create the canonical launch server"
    );
    assert!(!rig.dir("orchestrator").exists());
    send(&rig, &pane, "O");
    let screen = live(&rig, &pane);
    bounded(
        "the cold Start launches the dedicated external agent",
        || starts(&rig, "orchestrator") == 1,
    );
    assert!(rig.socket.exists(), "O creates the canonical launch socket");
    let saved = meta(&rig, "orchestrator");
    assert_eq!(value(&saved, "meta_agent"), "true");
    assert_eq!(
        value(&saved, "local_config"),
        rig.home
            .join("orchestrator.config")
            .to_str()
            .expect("fixture path")
    );
    no_stopped_panel(&screen);
    quit(&rig, &pane, &rc);
}

/// An existing path which cannot answer as a tmux socket proves no absence.
/// A self-referential symlink forces a read failure without relying on
/// permission bits that a privileged test user could bypass.
#[test]
fn an_existing_unreadable_launch_socket_stays_unknown_and_starts_nothing() {
    let rig = cold_configured("o8unreadable");
    std::os::unix::fs::symlink(&rig.socket, &rig.socket).expect("an existing unusable socket path");
    assert!(
        std::fs::symlink_metadata(&rig.socket)
            .expect("the unreadable path exists")
            .file_type()
            .is_symlink(),
        "setup: the launch path exists even though following it fails"
    );
    assert_eq!(
        strict_launch_absence(&rig),
        ae::tmux::StopProbe::Unknown,
        "an existing unreadable path is never proof that the server is absent"
    );
    let (pane, rc) = open(&rig, "unreadable", None);
    send(&rig, &pane, "O");
    let screen = rig.wait(
        &pane,
        "an unreadable launch server gives a refusal rather than starting a seat",
        |screen| {
            let shown = panel_text(screen);
            // The reason names the server whose absence cannot be proved.
            shown.contains("orchestrator")
                && shown.contains("server")
                && !shown.contains("Enter writes")
        },
    );
    no_stopped_panel(&screen);
    assert!(
        !rig.dir("orchestrator").exists(),
        "a refusal creates no seat"
    );
    assert_eq!(starts(&rig, "orchestrator"), 0, "no external agent starts");
    assert_eq!(
        std::fs::read_link(&rig.socket).expect("the unusable path survives"),
        rig.socket,
        "Start never replaces an existing unreadable socket path"
    );
    quit(&rig, &pane, &rc);
}

const COLD_RESUME_ID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const COLD_LAST_LIVE: i64 = 1_700_000_000;
const COLD_BOOT: i64 = 1_700_000_100;

/// A legacy seat record after a reboot: no socket and no newer launch stamp,
/// watchdog or pane marker. Its identity and roster remain real persisted
/// inputs to discovery and launch. Only the optional liveness fact varies.
fn cold_seat_record(rig: &Rig, last_live: Option<i64>) {
    let dir = rig.dir("orchestrator");
    std::fs::create_dir_all(&dir).expect("the private legacy seat directory");
    let overlay = rig.home.join("orchestrator.config");
    std::fs::write(
        &overlay,
        "[workspace]\nmain = orchestrator\nworkers = \"\"\nlayout = vertical\norchestrator = true\n",
    )
    .expect("the canonical seat overlay");
    let stamp = last_live.map_or_else(String::new, |epoch| format!("started={epoch}\n"));
    std::fs::write(
        dir.join("meta"),
        format!(
            "schema=2\nmode=local\nsession=orchestrator\nsession_id={COLD_RESUME_ID}\n\
             origin={}\nwork_dir={}\nconfig={}\nlocal_config={}\n\
             layout=vertical\nseat.main=orchestrator\nprofile.main=idle\n\
             main_pane=%1\nmeta_agent=true\ntmux_server_kind=socket\ntmux_server={}\n{stamp}",
            rig.project.display(),
            rig.project.display(),
            rig.home.join("config").display(),
            overlay.display(),
            rig.socket.display(),
        ),
    )
    .expect("the legacy seat identity and optional liveness fact");
    assert!(
        !rig.socket.exists(),
        "the recorded server socket is missing"
    );
    assert_eq!(
        ae::inventory::last_live(&dir),
        last_live.map_or(ae::tmux::Evidence::Silent, ae::tmux::Evidence::At),
        "setup: no unmodelled newer liveness source masks this fixture"
    );
}

/// The existing checkout-only boot-time door reaches the app and its waited
/// child. The ordinary session remains live on the separate host server.
fn open_after_boot(rig: &Rig, tag: &str) -> (String, PathBuf) {
    let rc = rig.scratch.join(format!("rc-{tag}"));
    let line = format!("AE_TEST_BOOT_TIME={COLD_BOOT} {}", command(&[], &rc, true));
    let pane = rig.pane("host", &line);
    assert!(
        rig.pane_tmux(&["resize-window", "-t", &pane, "-x", "160", "-y", "45"])
            .0,
        "the boot-time app fixture uses an explicit 160x45 terminal"
    );
    rig.wait(
        &pane,
        "the ordinary app loads before the boot-time O",
        |screen| lands(screen, "one"),
    );
    (pane, rc)
}

/// The lead Resume ruling uses the launch owner's boot-time absence proof.
/// It must cross discovery and the locked Resume with the same captured UUID.
#[test]
fn a_missing_seat_socket_with_pre_boot_liveness_resumes_the_same_seat() {
    let rig = cold_configured("o8bootresume");
    cold_seat_record(&rig, Some(COLD_LAST_LIVE));
    assert_eq!(
        strict_launch_absence(&rig),
        ae::tmux::StopProbe::Unknown,
        "the boot proof never widens strict destructive absence"
    );
    let (pane, rc) = open_after_boot(&rig, "boot-resume");
    assert!(!rig.socket.exists(), "opening the app resumes nothing");
    send(&rig, &pane, "O");
    let screen = live(&rig, &pane);
    bounded("the pre-boot seat resumes its external agent once", || {
        starts(&rig, "orchestrator") == 1
    });
    assert_eq!(
        value(&meta(&rig, "orchestrator"), "session_id"),
        COLD_RESUME_ID,
        "a boot-proved Resume keeps the captured seat incarnation"
    );
    no_stopped_panel(&screen);
    quit(&rig, &pane, &rc);
}

/// A missing socket without liveness evidence stays Unknown for Resume.
/// An old seat record may lack every newer launch fact, so absence of those
/// facts must not become permission to create a second incarnation.
#[test]
fn a_missing_seat_socket_without_liveness_evidence_resumes_nothing() {
    let rig = cold_configured("o8bootunknown");
    cold_seat_record(&rig, None);
    assert_eq!(strict_launch_absence(&rig), ae::tmux::StopProbe::Unknown);
    let (pane, rc) = open_after_boot(&rig, "boot-unknown");
    let before = meta(&rig, "orchestrator");
    send(&rig, &pane, "O");
    let screen = rig.wait(
        &pane,
        "a missing socket without evidence refuses Resume with a server reason",
        |screen| {
            let shown = panel_text(screen);
            shown.contains("orchestrator")
                && shown.contains("server")
                && !shown.contains("Enter writes")
        },
    );
    no_stopped_panel(&screen);
    assert!(!rig.socket.exists(), "an unproved Resume creates no server");
    assert_eq!(starts(&rig, "orchestrator"), 0, "no external agent resumes");
    assert_eq!(
        meta(&rig, "orchestrator"),
        before,
        "the refused Resume keeps the existing seat record"
    );
    assert!(
        !rig.dir("orchestrator")
            .join(ae::store::LAUNCH_ATTEMPT)
            .exists(),
        "a refusal records no new launch attempt"
    );
    quit(&rig, &pane, &rc);
}

/// A launch claims its directory before publishing its metadata. Both app
/// paths must distinguish that real interval from a stopped or damaged seat.
mod start_publication {
    use super::*;

    /// Releasing a fixture gate also runs during unwinding. No process is
    /// selected or killed by name; the existing `Rig` owns its private server.
    struct Gate(PathBuf);

    impl Gate {
        fn release(&self) {
            std::fs::remove_file(&self.0).expect("release the owned gate");
        }
    }

    impl Drop for Gate {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Only the caller's external tmux receives this wrapper. Every real
    /// command executes unchanged after the selected create is released.
    struct Launch {
        gate: Gate,
        trace: PathBuf,
        wrap: PathBuf,
    }

    impl Launch {
        fn new(rig: &Rig, name: &str) -> Self {
            let path = std::env::var_os("PATH").expect("the fixture PATH");
            let real = std::env::split_paths(&path)
                .map(|dir| dir.join("tmux"))
                .find(|candidate| candidate.is_file())
                .expect("the real external tmux");
            let wrap = rig.scratch.join(format!("wrap-{name}"));
            std::fs::create_dir(&wrap).expect("new private wrapper directory");
            let gate = Gate(wrap.join("hold"));
            let trace = wrap.join("trace");
            std::fs::write(&gate.0, "").expect("hold the selected tmux create");
            let wrapper = wrap.join("tmux");
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\ncase \" $* \" in\n*\" new-session \"*\" -s {name} \"*)\n\
                     printf 'held create\\n' >> {trace}\n\
                     polls=0\nwhile [ -e {hold} ]; do\n\
                     polls=$((polls + 1))\n[ \"$polls\" -lt 1200 ] || exit 124\n\
                     sleep 0.05\ndone\n;;\nesac\nexec {real} \"$@\"\n",
                    trace = quote(&trace.display().to_string()),
                    hold = quote(&gate.0.display().to_string()),
                    real = quote(&real.display().to_string()),
                ),
            )
            .expect("an external dependency wrapper, not a fake launch");
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
                .expect("executable private wrapper");
            Self { gate, trace, wrap }
        }

        fn command(&self, line: &str) -> String {
            format!(
                "PATH={}:\"$PATH\" {line}",
                quote(&self.wrap.display().to_string())
            )
        }

        fn arrived(&self, rig: &Rig, name: &str) {
            bounded("the real launch reached the selected tmux create", || {
                std::fs::read_to_string(&self.trace)
                    .unwrap_or_default()
                    .contains("held create")
            });
            let dir = rig.dir(name);
            assert!(dir.is_dir(), "the product created the session directory");
            assert!(
                std::fs::symlink_metadata(dir.join("meta"))
                    .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound),
                "the real launch has not published any meta node"
            );
            assert!(
                matches!(
                    ae::store::open(&dir).launch_attempt(),
                    ae::tmux::Evidence::At(_)
                ),
                "the product stamped its claimed launch before this hold"
            );
        }
    }

    fn world_gate(rig: &Rig, gates: &Path) -> Gate {
        let gate = Gate(gates.join("@world"));
        std::fs::write(&gate.0, "").expect("hold the next real fleet read");
        bounded("the real reader reached its world gate", || {
            std::fs::read_to_string(gates.join("trace"))
                .unwrap_or_default()
                .lines()
                .any(|line| line == "held @world")
        });
        // The initial fleet completed before this hold. No synthetic Fleet,
        // proof, seat answer, session directory or metadata is supplied.
        assert!(rig.has_session("one"));
        gate
    }

    struct Pair {
        launch: Launch,
        world: Gate,
        first: String,
        rc_first: PathBuf,
        second: String,
        rc_second: PathBuf,
    }

    impl Pair {
        fn new(rig: &Rig) -> Self {
            let gates = rig.scratch.join("read-gates");
            std::fs::create_dir(&gates).expect("private read gates");
            let (second, rc_second) = open(rig, "observer", Some(&gates));
            let world = world_gate(rig, &gates);
            let launch = Launch::new(rig, "orchestrator");
            let rc_first = rig.scratch.join("rc-winner");
            // The observer above is the home's one app. The launching app is a
            // second one beside it, so it starts with `--new`.
            let first = rig.pane(
                "host",
                &launch.command(&command(&["app", "--new"], &rc_first, true)),
            );
            rig.wait(&first, "the launching app initially lists one", |s| {
                lands(s, "one")
            });
            send(rig, &first, "O");
            launch.arrived(rig, "orchestrator");
            Self {
                launch,
                world,
                first,
                rc_first,
                second,
                rc_second,
            }
        }

        fn open_observer(&self, rig: &Rig) {
            send(rig, &self.second, "O");
            rig.wait(
                &self.second,
                "O opens the observer before its reader resumes",
                |s| panel_border(s).is_some() && panel_text(s).contains("read-only"),
            );
            self.world.release();
        }
    }

    /// Observe a real drawn column for two seconds after its positive anchor.
    /// This catches the persistent bad verdict, not every transient terminal byte.
    fn observe(rig: &Rig, pane: &str, why: &str, valid: impl Fn(&str) -> bool) {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let screen = rig.screen(pane);
            assert!(valid(&panel_text(&screen)), "{why}; frame:\n{screen}");
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn an_opening_during_publication_binds_the_winner_without_reopen() {
        let rig = configured("o8-publish-pair");
        let pair = Pair::new(&rig);
        pair.open_observer(&rig);
        observe(
            &rig,
            &pair.second,
            "an unpublished live attempt is not a refusal",
            |s| !s.contains("orchestrator not started") && !s.contains("orchestrator stopped"),
        );
        pair.launch.gate.release();
        no_stopped_panel(&live(&rig, &pair.first));
        no_stopped_panel(&live(&rig, &pair.second));
        assert_eq!(
            starts(&rig, "orchestrator"),
            1,
            "both openings share the winner"
        );
        quit(&rig, &pair.first, &pair.rc_first);
        quit(&rig, &pair.second, &pair.rc_second);
    }

    /// The live wait must observe an expired attempt too. Age the persisted
    /// evidence while the real launch remains held, without changing a clock
    /// or waiting the full budget. No metadata is ever published in this leg.
    #[test]
    fn a_waiting_opening_rechecks_an_attempt_that_never_publishes() {
        let rig = configured("o8-publish-expire");
        let pair = Pair::new(&rig);
        pair.open_observer(&rig);
        observe(
            &rig,
            &pair.second,
            "the current attempt initially waits",
            |s| !s.contains("orchestrator not started"),
        );
        ae::store::open(&rig.dir("orchestrator"))
            .stamp_launch_attempt(1)
            .expect("advance only the persisted attempt evidence past any launch budget");
        let expired = rig.wait(
            &pair.second,
            "an unpublished attempt does not wait forever",
            |s| {
                let shown = panel_text(s);
                shown.contains("orchestrator not started") && shown.contains("metadata")
            },
        );
        assert!(panel_text(&expired).contains("read-only"), "{expired}");
        assert!(!rig.dir("orchestrator").join("meta").exists());
        assert!(!rig.has_session("orchestrator"));
        assert_eq!(starts(&rig, "orchestrator"), 0);
        pair.launch.gate.release();
        live(&rig, &pair.first);
        quit(&rig, &pair.first, &pair.rc_first);
        quit(&rig, &pair.second, &pair.rc_second);
    }

    /// GREEN at C3: an absent publication is bounded by its actual evidence.
    /// An ancient positive epoch advances that evidence without a long sleep.
    /// Future or missing stamps and an existing unreadable meta node are errors.
    #[test]
    fn only_a_current_unpublished_claim_may_wait_for_publication() {
        for kind in ["expired", "future", "unclaimed", "published-link"] {
            let rig = configured(&format!("o8-publish-{kind}"));
            let pair = Pair::new(&rig);
            let dir = rig.dir("orchestrator");
            match kind {
                "expired" => ae::store::open(&dir)
                    .stamp_launch_attempt(1)
                    .expect("old attempt"),
                "future" => ae::store::open(&dir)
                    .stamp_launch_attempt(i64::MAX)
                    .expect("future attempt"),
                "unclaimed" => {
                    std::fs::remove_file(dir.join(ae::store::LAUNCH_ATTEMPT)).expect("no claim");
                }
                "published-link" => {
                    std::os::unix::fs::symlink(dir.join("missing-target"), dir.join("meta"))
                        .expect("an existing unreadable published node");
                }
                _ => unreachable!("the fixture enumerates these four evidence states"),
            }
            pair.open_observer(&rig);
            let screen = rig.wait(
                &pair.second,
                "a non-current or published error refuses without an endless wait",
                |s| {
                    let shown = panel_text(s);
                    shown.contains("orchestrator not started") && shown.contains("metadata")
                },
            );
            assert!(panel_text(&screen).contains("read-only"), "{screen}");
            assert!(!rig.has_session("orchestrator"));
            assert_eq!(starts(&rig, "orchestrator"), 0);
            if kind == "published-link" {
                std::fs::remove_file(dir.join("meta")).expect("remove only the fixture link");
            }
            pair.launch.gate.release();
            live(&rig, &pair.first);
            quit(&rig, &pair.first, &pair.rc_first);
            quit(&rig, &pair.second, &pair.rc_second);
        }
    }

    #[test]
    fn another_launch_and_a_published_census_error_never_declare_the_bound_seat_stopped() {
        let rig = configured("o8-publish-bound");
        launch_seat(&rig);
        let id = value(&meta(&rig, "orchestrator"), "session_id").to_owned();
        let gates = rig.scratch.join("read-gates");
        std::fs::create_dir(&gates).expect("private read gates");
        let (pane, rc) = open(&rig, "bound", Some(&gates));
        send(&rig, &pane, "O");
        live(&rig, &pane);
        let world = world_gate(&rig, &gates);
        // This row cannot appear until a fleet read includes the held launch.
        rig.launch("landmark");
        let launch = Launch::new(&rig, "two");
        let rc_two = rig.scratch.join("rc-two");
        let line = format!(
            "cd {} && {}",
            quote(&rig.project.display().to_string()),
            launch.command(&command(&["--local", "two", "--no-attach"], &rc_two, true))
        );
        let _launcher = rig.pane("host", &line);
        launch.arrived(&rig, "two");
        world.release();
        rig.wait(
            &pane,
            "a fresh fleet from this window paints its positive landmark",
            |s| lands(s, "landmark"),
        );
        observe(
            &rig,
            &pane,
            "another launch never declares the bound seat stopped",
            |s| !s.contains("orchestrator stopped") && !s.contains("orchestrator not started"),
        );
        launch.gate.release();
        assert_eq!(wait_for_rc(&rc_two), "0", "the ordinary launch completes");
        live(&rig, &pane);
        let path = rig.dir("two").join("meta");
        let saved = std::fs::read(&path).expect("the real published ordinary metadata");
        std::fs::write(&path, b"meta_agent=true\nmeta_agent=false\n")
            .expect("damage a published claim");
        let gap = rig.wait(&pane, "a published census error remains an error", |s| {
            let shown = panel_text(s);
            shown.contains("unproven") && shown.contains("meta_agent")
        });
        assert!(!panel_text(&gap).contains("orchestrator stopped"), "{gap}");
        assert!(
            !panel_text(&gap).contains("orchestrator not started"),
            "{gap}"
        );
        std::fs::write(path, saved).expect("restore only the owned metadata");
        no_stopped_panel(&live(&rig, &pane));
        assert_eq!(value(&meta(&rig, "orchestrator"), "session_id"), id);
        assert_eq!(starts(&rig, "orchestrator"), 1);
        quit(&rig, &pane, &rc);
    }
}
