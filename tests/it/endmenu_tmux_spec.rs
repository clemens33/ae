//! #248: exercise the real tmux menu, passing input through a nested client.

use super::*;

const CLICKED: &str = "end-clicked";
const VIEWED: &str = "end-viewed";
const VIEWER: &str = "end-viewer";
const MARKER: &str = "the End menu must keep this history";

struct EndMenu {
    cleanup: Cleanup,
    root: PathBuf,
    client: String,
    dir: PathBuf,
    archive: PathBuf,
}

impl EndMenu {
    fn new() -> Self {
        let scratch = scratch("endmenu");
        let socket = scratch.join("s");
        let cleanup = Cleanup { socket, scratch };
        let root = cleanup.scratch.join("state");
        let project = cleanup.scratch.join("project");
        let config = cleanup.scratch.join("config");
        write_state_fixture_config(&project, &config);
        let text = fixture_text(&config);
        assert!(fs::write(&config, format!("{text}purge_agent_history = true\n")).is_ok());
        for session in [CLICKED, VIEWED] {
            launch_ae_session(
                &cleanup.socket,
                &cleanup.scratch,
                &root,
                &project,
                &config,
                session,
            );
        }
        let id = listing_id(&cleanup.socket, &cleanup.scratch, CLICKED);
        point_session_range_at(&cleanup.socket, &cleanup.scratch, VIEWED, &id);
        let client = nested_client(&cleanup.socket, &cleanup.scratch, VIEWED, VIEWER);
        let dir = root.join("sessions").join(CLICKED);
        let text = fixture_text(&dir.join("meta"));
        let meta = ae::meta::Meta::parse(&text);
        let uuid = meta
            .session_id()
            .unwrap_or_else(|| panic!("the clicked session has no UUID"));
        let archive = root.join("archive").join(uuid);
        declare_state(&dir, "lead", "working", MARKER);
        Self {
            cleanup,
            root,
            client,
            dir,
            archive,
        }
    }

    fn tmux(&self, words: &[&str]) -> String {
        let (ok, out) = tmux(&self.cleanup.socket, &self.cleanup.scratch, words);
        assert!(ok, "tmux {words:?}: {out}");
        out
    }

    fn capture(&self) -> String {
        self.tmux(&["capture-pane", "-p", "-t", VIEWER])
    }

    fn open(&self) -> String {
        open_context_menu(
            &self.cleanup.socket,
            &self.cleanup.scratch,
            VIEWER,
            &self.client,
        )
    }

    fn key(&self, key: &str) {
        self.tmux(&["send-keys", "-t", VIEWER, key]);
    }

    fn question(&self) -> String {
        let root = self.open();
        assert!(root.contains("End session"), "End is missing:\n{root}");
        self.key("e");
        wait_for(
            "End confirmation on the invoking client",
            || self.capture(),
            |seen| seen.contains("Cancel") && seen.contains("End 'end-clicked' now"),
        )
    }

    fn live(&self, session: &str) -> bool {
        self.tmux(&["list-sessions", "-F", "#{session_name}"])
            .lines()
            .any(|line| line == session)
    }
}

fn absent(path: &Path) -> bool {
    ae::store::source_presence(path) == ae::store::SourcePresence::Absent
}

#[test]
fn endmenu_tmux_groups_end_beside_stop_below_the_safe_rows() {
    let rig = EndMenu::new();
    let menu = rig.open();
    let flip = row_of(&menu, "Flip lead/colead panes");
    let stop = row_of(&menu, "Stop session");
    let end = row_of(&menu, "End session");
    assert!(flip + 1 < stop && stop + 1 == end, "{menu}");
    assert!(!menu.contains("Purge"), "{menu}");
}

#[test]
fn endmenu_tmux_initial_enter_cancels_and_explicit_end_retires_the_clicked_session() {
    let rig = EndMenu::new();
    let claude_home = rig.cleanup.scratch.join(".claude");
    let conversations = claude_home.join("projects/work");
    let id = "0199c0de-bbbb-4890-abcd-ef0123456789";
    let file = conversations.join(format!("{id}.jsonl"));
    assert!(fs::create_dir_all(&conversations).is_ok());
    assert!(fs::write(&file, MARKER).is_ok());
    for (key, value) in [
        ("agent_bin.main", "claude"),
        ("config_home.main", &claude_home.display().to_string()),
        ("harness_session.main", id),
    ] {
        assert!(ae::meta::rewrite(&rig.dir, key, Some(value)).is_ok());
    }
    let question = rig.question();
    let question_words = question.to_lowercase();
    assert!(
        question_words.contains("removed from the session list")
            && question_words.contains("history")
            && question_words.contains("archive")
            && !question_words.contains("deleted"),
        "the question must name the session's retirement and preservation:\n{question}"
    );
    let before = (meta_bytes(&rig.dir), events_of(&rig.dir));
    rig.key("Enter");
    wait_for(
        "initial Enter selects Cancel",
        || rig.capture(),
        |seen| !seen.contains("End 'end-clicked' now"),
    );
    assert!(rig.live(CLICKED));
    assert!(absent(&rig.archive));
    assert_eq!((meta_bytes(&rig.dir), events_of(&rig.dir)), before);

    // As in the Stop fixture, a new mouse gesture just after closing a menu
    // can be dropped by a real nested client even after its redraw.
    std::thread::sleep(Duration::from_secs(2));
    rig.question();
    // With no initial focus, Down would select Cancel and this would not end.
    rig.key("Down");
    rig.key("Enter");
    wait_for(
        "the End owner archives and unregisters the clicked session",
        || events_of(&rig.archive),
        |events| events.contains(MARKER) && events.contains("end-request") && absent(&rig.dir),
    );
    assert!(!rig.live(CLICKED));
    assert!(rig.live(VIEWED));
    assert!(state_kept(&rig.root.join("sessions").join(VIEWED)));
    assert_eq!(fixture_text(&file), MARKER);
}

#[test]
fn endmenu_tmux_refuses_if_the_recorded_server_changes_while_confirmation_is_open() {
    let rig = EndMenu::new();
    let foreign_socket = rig.cleanup.scratch.join("foreign");
    assert!(
        tmux(
            &foreign_socket,
            &rig.cleanup.scratch,
            &[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                CLICKED,
                "sleep 60"
            ],
        )
        .0
    );
    rig.question();
    assert!(ae::meta::rewrite(&rig.dir, "tmux_server", foreign_socket.to_str()).is_ok());
    let before = (meta_bytes(&rig.dir), events_of(&rig.dir));

    rig.key("E");
    wait_for(
        "a client-visible refusal names the changed server",
        || rig.capture(),
        |seen| seen.contains(CLICKED) && seen.contains("different tmux server"),
    );
    assert!(rig.live(CLICKED));
    assert!(
        tmux(
            &foreign_socket,
            &rig.cleanup.scratch,
            &["has-session", "-t", CLICKED]
        )
        .0
    );
    assert_eq!((meta_bytes(&rig.dir), events_of(&rig.dir)), before);
    assert!(absent(&rig.archive));
}

#[test]
fn endmenu_tmux_refuses_when_ending_now_would_commit_and_push() {
    let rig = EndMenu::new();
    rig.question();
    assert!(ae::meta::rewrite(&rig.dir, "mode", Some("git")).is_ok());
    let before = (meta_bytes(&rig.dir), events_of(&rig.dir));

    rig.key("E");
    wait_for(
        "a client-visible refusal names the changed consequence",
        || rig.capture(),
        |seen| seen.contains(CLICKED) && seen.contains("changed"),
    );
    assert!(rig.live(CLICKED));
    assert_eq!((meta_bytes(&rig.dir), events_of(&rig.dir)), before);
    assert!(absent(&rig.archive));
}
