//! Frozen acceptance: ending one session may not delete another's conversation.
//! Real `_end`, private tmux socket, state root and harness stores; no purge mocks.

use super::*;

const SHARED: &str = "0199c0de-1234-4890-abcd-ef0123456789";
const UNIQUE: &str = "0199c0de-5678-4890-abcd-ef0123456789";
const OTHER: &str = "purge-neighbor";

struct PurgeRig {
    rig: Rig,
    // Declared after rig: its server/state drop before the containing fake HOME.
    _home: Scratch,
}

impl PurgeRig {
    fn new(tag: &str) -> Self {
        let home = Scratch::new(OwnedScratch::root("purgeguard", tag).keep());
        let root = home.home.join(".ae");
        let name = format!("pg-{tag}");
        let dir = root.join("sessions").join(&name);
        std::fs::create_dir_all(&dir).expect("isolated session");
        let config = root.join("config");
        std::fs::write(
            &config,
            "[profiles]\nfake = \"sh\"\n[roster]\nfake = fake\n[workspace]\nmain = fake\nlayout = vertical\n",
        )
        .expect("isolated config");
        let mut rig = Rig {
            home: root,
            sock: home.home.join("s"),
            name,
            dir,
            pane: String::new(),
        };
        assert!(
            rig.tmux(&[
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                &rig.name,
                "sh"
            ])
            .0,
            "private server starts"
        );
        let (_, panes) = rig.tmux(&["list-panes", "-t", &rig.name, "-F", "#{pane_id}"]);
        panes
            .lines()
            .next()
            .expect("main pane")
            .clone_into(&mut rig.pane);
        std::fs::write(
            rig.dir.join("meta"),
            format!(
                "schema=2\nsession={}\nsession_id={UUID}\nsession_id_origin=session\n\
                 mode=local\nwork_dir={}\norigin={}\nlayout=vertical\nconfig={}\n\
                 main_pane={}\ntmux_server_kind=socket\ntmux_server={}\n\
                 seat.main=lead\nprofile.main=fake\nagent_bin.main=sh\n",
                rig.name,
                home.home.display(),
                home.home.display(),
                config.display(),
                rig.pane,
                rig.sock.display(),
            ),
        )
        .expect("ended session meta");
        Self { rig, _home: home }
    }

    fn conversation(&self, tool: &str, id: &str) -> Vec<PathBuf> {
        let home = self.rig.home.parent().expect("fake home");
        let (config, files) = match tool {
            "claude" => {
                let config = home.join("claude-account");
                let files = vec![config.join("projects/work").join(format!("{id}.jsonl"))];
                (config, files)
            }
            "codex" => {
                let config = home.join("codex-account");
                let files = vec![
                    config
                        .join("sessions/2026/10/16")
                        .join(format!("rollout-2026-10-16T10-00-00-{id}.jsonl")),
                ];
                (config, files)
            }
            "agy" => {
                let config = home.join(".gemini");
                let files = ["db", "db-wal", "db-shm"]
                    .map(|suffix| {
                        config
                            .join("antigravity-cli/conversations")
                            .join(format!("{id}.{suffix}"))
                    })
                    .to_vec();
                (config, files)
            }
            _ => panic!("unsupported fixture tool"),
        };
        for file in &files {
            std::fs::create_dir_all(file.parent().expect("store parent")).expect("store");
            std::fs::write(file, "independent conversation sentinel\n").expect("conversation");
        }
        let canonical = std::fs::canonicalize(config).expect("canonical config");
        for (key, value) in [
            ("agent_bin.main", tool),
            (
                "config_home.main",
                canonical.to_str().expect("fixture path"),
            ),
            ("harness_session.main", id),
        ] {
            ae::meta::rewrite(&self.rig.dir, key, Some(value)).expect("fixture row");
        }
        files
    }

    fn other_meta(&self, content: &str) -> PathBuf {
        let dir = self.rig.home.join("sessions").join(OTHER);
        std::fs::create_dir_all(&dir).expect("another session");
        let path = dir.join("meta");
        std::fs::write(&path, content).expect("other meta");
        path
    }

    fn end(&self) -> (String, String) {
        let (code, out, err) = self
            .rig
            .run(&["_end", "-f", "--purge-history", &self.rig.name]);
        assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
        assert!(!exists(&self.rig.dir), "end must still remove its session");
        (out, err)
    }
}

fn assert_kept(files: &[PathBuf], err: &str) {
    for file in files {
        assert_eq!(
            std::fs::read_to_string(file).ok().as_deref(),
            Some("independent conversation sentinel\n"),
            "shared conversation must survive: {}\n{err}",
            file.display()
        );
    }
    assert!(
        err.contains("left in place") && err.contains("main"),
        "{err}"
    );
    assert!(err.contains(OTHER), "note must name other session: {err}");
    assert!(
        !err.contains("/sessions/"),
        "ownership note names the session, not its path: {err}"
    );
}

#[test]
fn purgeguard_another_sessions_current_claim_preserves_every_tool() {
    for tool in ["claude", "codex", "agy"] {
        let fixture = PurgeRig::new(&format!("current-{tool}"));
        let files = fixture.conversation(tool, SHARED);
        let path = fixture.other_meta(&format!(
            "schema=2\nsession={OTHER}\nseat.spawned.7=neighbor\n\
             agent_bin.spawned.7={tool}\nharness_session.spawned.7={SHARED}\n"
        ));
        let before = std::fs::read(&path).expect("other meta before");
        let (_, err) = fixture.end();
        assert_kept(&files, &err);
        assert_eq!(std::fs::read(path).expect("other meta after"), before);
    }
}

#[test]
fn purgeguard_another_sessions_tagged_and_legacy_priors_preserve_every_tool() {
    for tool in ["claude", "codex", "agy"] {
        for tagged in [false, true] {
            let fixture = PurgeRig::new(&format!("prior-{tool}-{tagged}"));
            let files = fixture.conversation(tool, SHARED);
            let shared = if tagged {
                format!("{tool}:{SHARED}")
            } else {
                SHARED.to_owned()
            };
            // Shared claim is not first, and the current id differs.
            fixture.other_meta(&format!(
                "schema=2\nsession={OTHER}\nseat.worker.3=neighbor\n\
                 agent_bin.worker.3={tool}\nharness_session.worker.3={UNIQUE}\n\
                 harness_session_prior.worker.3={tool}:{UNIQUE},{shared}\n"
            ));
            let (_, err) = fixture.end();
            assert_kept(&files, &err);
        }
    }
}

#[test]
fn purgeguard_unique_current_and_prior_are_still_removed_for_every_tool() {
    for tool in ["claude", "codex", "agy"] {
        let fixture = PurgeRig::new(&format!("unique-{tool}"));
        let mut files = fixture.conversation(tool, SHARED);
        files.extend(fixture.conversation(tool, UNIQUE));
        ae::meta::rewrite(
            &fixture.rig.dir,
            "harness_session_prior.main",
            Some(&format!("{tool}:{SHARED}")),
        )
        .expect("own prior");
        fixture.other_meta(&format!(
            "schema=2\nsession={OTHER}\nseat.main=neighbor\nagent_bin.main={tool}\n\
             harness_session.main=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\n"
        ));
        let (out, err) = fixture.end();
        for file in files {
            assert!(
                !exists(&file),
                "unique conversation was not removed: {out} {err}"
            );
        }
        assert!(!err.contains("left in place"), "{err}");
    }
}

#[test]
fn purgeguard_unreadable_missing_and_damaged_other_metas_fail_closed() {
    for damage in ["malformed", "duplicate", "nonutf8", "directory", "missing"] {
        let fixture = PurgeRig::new(damage);
        let files = fixture.conversation("claude", SHARED);
        let path = fixture.other_meta("schema=2\nsession=purge-neighbor\nseat.main=neighbor\n");
        match damage {
            "malformed" => std::fs::write(&path, "broken meta without equals\n").expect("damage"),
            "duplicate" => std::fs::write(
                &path,
                "schema=2\nseat.main=neighbor\nharness_session.main=pending\n\
                 harness_session.main=unknown\n",
            )
            .expect("duplicate claim"),
            "nonutf8" => std::fs::write(&path, [0xff, 0xfe]).expect("non UTF-8 meta"),
            "directory" => {
                std::fs::remove_file(&path).expect("remove meta");
                std::fs::create_dir(&path).expect("unreadable meta path");
            }
            "missing" => std::fs::remove_file(&path).expect("missing meta"),
            _ => unreachable!(),
        }
        let (_, err) = fixture.end();
        assert_kept(&files, &err);
        assert!(
            [
                "cannot",
                "unreadable",
                "damaged",
                "malformed",
                "missing",
                "invalid"
            ]
            .iter()
            .any(|word| err.contains(word)),
            "note must explain why ownership is unknown: {err}"
        );
    }
}

#[test]
fn purgeguard_two_slots_in_the_ended_session_do_not_block_removal() {
    for tool in ["claude", "codex", "agy"] {
        let fixture = PurgeRig::new(&format!("own-slots-{tool}"));
        let files = fixture.conversation(tool, SHARED);
        let raw = std::fs::read_to_string(fixture.rig.dir.join("meta")).expect("own meta");
        let config = ae::meta::Meta::parse(&raw);
        let ae::meta::RecordedConfigHome::Path(home) = &config.roster()[0].config_home else {
            panic!("recorded fixture home");
        };
        std::fs::write(
            fixture.rig.dir.join("meta"),
            format!(
                "{raw}seat.worker.1=pair\nagent_bin.worker.1={tool}\nconfig_home.worker.1={}\n\
                 harness_session.worker.1={SHARED}\nharness_session_prior.worker.1={tool}:{SHARED}\n",
                home.display()
            ),
        )
        .expect("second own slot");
        let (out, err) = fixture.end();
        for file in files {
            assert!(!exists(&file), "own duplicate must be purged: {out} {err}");
        }
        assert!(!err.contains("left in place"), "{err}");
    }
}
