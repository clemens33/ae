//! #250 O2 phase 2, frozen acceptance: every ask/review stamps BOTH seat
//! incarnations (`actor_launch_id`, `target_launch_id`), and the event reader
//! knows the two keys. Black box: the real binary, a private tmux socket, the
//! journal bytes. [`World`] is shared with `super::o2guard_spec`.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};

/// One private tmux server and one scratch state root (`AE_HOME`) holding
/// `sessions/<name>/meta` for every session started on it.
pub(crate) struct World {
    pub(crate) root: PathBuf,
    pub(crate) sock: PathBuf,
}

/// A started seat: its pane, its session directory, and the file its raw-mode
/// `cat` writes every byte the pane RECEIVES into.
pub(crate) struct Seat {
    pub(crate) pane: String,
    pub(crate) dir: PathBuf,
    pub(crate) receipt: PathBuf,
}

/// The canonical session UUID a world gives session `name`.
pub(crate) fn uuid_of(name: &str) -> String {
    let n = name
        .bytes()
        .fold(7u32, |a, b| a.wrapping_mul(31) ^ u32::from(b));
    format!("{n:08x}-1111-4111-8111-111111111111")
}

impl World {
    pub(crate) fn new(tag: &str) -> Self {
        let root = super::cli::OwnedScratch::root("o2g", tag).keep();
        assert!(std::fs::create_dir_all(root.join("sessions")).is_ok());
        let sock = root.join("sock");
        Self { root, sock }
    }

    /// tmux on this world's socket: `words` split on spaces, then `tail` whole.
    pub(crate) fn tmux(&self, words: &str, tail: &[&str]) -> (bool, String) {
        let server =
            ae::inventory::ServerId::Selected(ae::meta::Selector::Socket(self.sock.clone()));
        let mut args = ae::tmux::server_args(&server);
        args.extend(
            words
                .split(' ')
                .chain(tail.iter().copied())
                .map(str::to_owned),
        );
        super::phase2::run_tmux(&args, &self.root)
    }

    /// Start session `name` whose `main` seat is stamped `agent`, an unmodelled
    /// (gemini) tool, `extra` appended to its meta.
    pub(crate) fn session(&self, name: &str, agent: &str, extra: &str) -> Seat {
        let dir = self.root.join("sessions").join(name);
        let receipt = dir.join("receipt");
        assert!(std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&receipt, "").is_ok());
        let shell = format!(
            "printf '> '; stty raw -echo; exec cat -u > {}",
            receipt.display()
        );
        let new = format!("-f /dev/null new-session -d -x 200 -y 40 -P -F #{{pane_id}} -s {name}");
        let (ok, pane) = self.tmux(&new, &[&shell]);
        let pane = pane.trim().to_owned();
        assert!(ok && pane.starts_with('%'), "session {name} starts");
        self.stamp(&pane, "main", agent);
        let uuid = uuid_of(name);
        assert!(
            self.tmux("set-option -t", &[&pane, "@ae_session_uuid", &uuid])
                .0
        );
        let sock = self.sock.display();
        let meta = format!(
            "session={name}\nsession_id={uuid}\ntmux_server_kind=socket\ntmux_server={sock}\n\
             seat.main={agent}\nagent_bin.main=gemini\nlaunch_id.main=tok-{name}\n{extra}"
        );
        assert!(std::fs::write(dir.join("meta"), meta).is_ok());
        Seat { pane, dir, receipt }
    }

    pub(crate) fn stamp(&self, pane: &str, slot: &str, agent: &str) {
        assert!(self.tmux("set-option -p -t", &[pane, "@ae_slot", slot]).0);
        assert!(self.tmux("set-option -p -t", &[pane, "@ae_agent", agent]).0);
    }

    /// Run the real binary as a process inside `pane` (none: no tmux context):
    /// exit code, stdout, stderr.
    pub(crate) fn run(
        &self,
        pane: Option<&str>,
        argv: &[&str],
        envs: &[(&str, &str)],
    ) -> (Option<i32>, String, String) {
        let mut command = super::cli::ae();
        command
            .env("AE_HOME", &self.root)
            .env_remove("AE_SENDER_OVERRIDE");
        if let Some(pane) = pane {
            command
                .env("TMUX", format!("{},0,0", self.sock.display()))
                .env("TMUX_PANE", pane);
        }
        let out = command
            .envs(envs.iter().copied())
            .args(argv)
            .output()
            .expect("ae runs");
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        (out.status.code(), text(&out.stdout), text(&out.stderr))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = self.tmux("kill-server", &[]);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Everything the seat's pane RECEIVED, after a short settle for late bytes.
pub(crate) fn received(seat: &Seat) -> String {
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::read_to_string(&seat.receipt).unwrap_or_default()
}

pub(crate) fn ledger(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("events.jsonl")).unwrap_or_default()
}

/// The ledger line of `action` naming request `id`.
pub(crate) fn opening(ledger: &str, action: &str, id: &str) -> String {
    let (act, refer) = (
        format!("\"action\":\"{action}\""),
        format!("\"ref\":\"{id}\""),
    );
    let mut lines = ledger
        .lines()
        .filter(|line| line.contains(&act) && line.contains(&refer));
    lines.next().unwrap_or_default().to_owned()
}

// Wrong implementations caught: stamping only role-opened requests (every
// target here is ordinary); swapping the two ends; reading the target's token
// out of the CALLER's meta; stamping the caller's ledger but not the mirror;
// and stamping a slotless actor (AE_SENDER_OVERRIDE, no pane identity) with a
// GUESSED incarnation such as the helper session's main token.
#[test]
fn every_ask_and_review_stamps_both_seat_incarnations_in_both_ledgers() {
    let world = World::new("stamp");
    let (wrk, odd) = (
        world.session("wrk", "wkr", ""),
        world.session("odd", "ody", ""),
    );
    let dir = wrk.dir.to_string_lossy().into_owned();
    let mut wrong = Vec::new();
    for verb in [ae::cli::ASK, ae::cli::REVIEW] {
        let argv = [verb, &dir, "--cross-session", "odd:ody", "q"];
        let (code, id, err) = world.run(Some(&wrk.pane), &argv, &[]);
        assert_eq!(code, Some(0), "setup: {verb} is delivered today: {err}");
        for (side, dir) in [("caller", &wrk.dir), ("mirror", &odd.dir)] {
            let line = opening(&ledger(dir), verb.trim_start_matches('_'), id.trim());
            assert!(!line.is_empty(), "setup: {verb} opens in the {side} ledger");
            for stamp in [
                "\"actor_launch_id\":\"tok-wrk\"",
                "\"target_launch_id\":\"tok-odd\"",
            ] {
                if !line.contains(stamp) {
                    wrong.push(format!("{verb} {side} lacks {stamp}"));
                }
            }
        }
    }
    let envs = [("AE_SENDER_OVERRIDE", "bridge:1")];
    let (code, id, err) = world.run(Some(&wrk.pane), &[ae::cli::ASK, &dir, "wkr", "q"], &envs);
    assert_eq!(
        code,
        Some(0),
        "setup: a slotless ask is delivered today: {err}"
    );
    let line = opening(&ledger(&wrk.dir), "ask", id.trim());
    assert!(
        !line.is_empty() && !line.contains("actor_launch_id"),
        "a slotless actor has no seat: {line}"
    );
    if !line.contains("\"target_launch_id\":\"tok-wrk\"") {
        wrong.push("slotless ask lacks its target's stamp".to_owned());
    }
    assert!(
        received(&odd).contains(" q"),
        "setup: both requests reached the target pane"
    );
    assert!(wrong.is_empty(), "{wrong:?}");
}

const BASE: &str = r#"{"ts":"2026-10-24T18:00:00Z","actor":"wkr","action":"ask","ref":"ask-1""#;

// Wrong implementations caught: the two keys left out of the known-key set (a
// duplicate is then inert and a forged second value can win), a wrong type
// read as absent, an over-strict reader that throws a whole legacy or
// empty-valued record away, and a strict reader that DISCARDS the value or
// collapses an empty one to absent (the binding then compares nothing). Every
// case is judged, then reported together.
#[test]
fn the_reader_knows_both_launch_keys_strictly_and_keeps_legacy_records() {
    let parse = |line: String| ae::events::Event::parse_line(&line).ok();
    let mut wrong = Vec::new();
    let legacy = parse(format!("{BASE}}}"));
    if legacy.is_none() {
        wrong.push("legacy record refused".to_owned());
    }
    for key in ["actor_launch_id", "target_launch_id"] {
        let with = |value: &str| parse(format!("{BASE},\"{key}\":{value}}}"));
        let cases = [
            ("\"\"", true, "empty"),
            ("\"a\",\"{k}\":\"b\"", false, "duplicate"),
            ("5", false, "non-string"),
        ];
        for (value, keeps, what) in cases {
            if with(&value.replace("{k}", key)).is_some() != keeps {
                wrong.push(format!("{what} {key} judged wrong"));
            }
        }
        let (a, b, empty) = (with("\"tok-a\""), with("\"tok-b\""), with("\"\""));
        if a.is_none() || a == b || a == legacy {
            wrong.push(format!("{key} value not retained"));
        }
        if empty.is_none() || empty == legacy {
            wrong.push(format!("{key} empty read as absent"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}
