//! A watchdog refreshes the running dispatcher when its own seat is done.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "the acceptance fixture owns its watcher, journal and private tmux"
)]

use super::app_writer_spec::Rig;
use super::cli::{OwnedChild, ae};
use super::o2guard_p2_spec::World;
use super::relay_chat_spec::words;
use super::watchdog_step::Stepper;
use std::fs;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::process::Stdio;

const CHARTER_HEAD: &str = "CHARTER UPDATE (ae ";
const MARKER: &str = "You are the owner's chief of staff across the ae fleet";
const OWNER_LINE: &str = "The owner's fixture sentence exists only in this on-disk overlay.";
const CURRENT_OVERLAY: &str = include_str!("../../contrib/aeorchestrator/orchestrator.config");

mod desklive_nudge {
    use super::super::o2guard_p2_spec::{Seat, uuid_of};
    use super::{CURRENT_OVERLAY, ManuallyDrop, OwnedChild, Rig, Stdio, Stepper, World, ae, fs};
    use ae::json::{self, Value};
    use ae::time::Timestamp;
    use std::io::Write;

    const PEER: &str = "desk-peer";
    const REF: &str = "ae-20261121T084012Z-00000001";
    const TICK: &str = "desk stale";

    struct Fixture {
        rig: Rig,
        leaf: Seat,
        now: i64,
        watcher: String,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let rig = Rig::new(tag);
            super::super::orchrename_charter_spec::turn_fixture(&rig);
            let global = rig.root.join("config");
            let overlay = rig.root.join("current-orchestrator.config");
            fs::write(&global, "[workspace]\nquota = off\nauto_upgrade = off\n")
                .expect("isolate external tools");
            fs::write(&overlay, CURRENT_OVERLAY).expect("current orchestrator overlay");
            let meta = rig.tool.dir.join("meta");
            let current = fs::read_to_string(&meta).expect("fixture meta");
            let solo = current
                .lines()
                .filter(|line| !line.contains(".worker.0="))
                .collect::<Vec<_>>()
                .join("\n")
                .replace("layout=lead-pair", "layout=vertical");
            fs::write(
                meta,
                format!(
                    "{solo}\nmeta_agent=true\nmain_pane={}\nlocal_config={}\nconfig={}\n\
                     charter.main=3\ncharter_launch.main=tok-rig\nsweep_sec=900\n",
                    rig.tool.pane,
                    overlay.display(),
                    global.display()
                ),
            )
            .expect("running current charter");
            let world = ManuallyDrop::new(World {
                root: rig.root.clone(),
                sock: rig.socket.clone(),
            });
            let leaf = world.session(PEER, "worker", "mode=local\nlayout=vertical\n");
            let now = Timestamp::now().epoch();
            for dir in [&leaf.dir, &rig.tool.dir] {
                fs::write(dir.join(".launch-attempt"), (now - 1_800).to_string())
                    .expect("launch predates the scripted journal");
            }
            for name in [rig.home.as_str(), PEER] {
                for (key, val) in [
                    ("AE_SESSION", name),
                    ("AE_HOME", rig.root.to_str().expect("UTF-8 scratch path")),
                ] {
                    rig.tmux(&["set-environment", "-t", name, key, val]);
                }
            }
            let watcher = rig
                .tmux(&[
                    "new-window",
                    "-d",
                    "-t",
                    &rig.home,
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "sleep",
                    "120",
                ])
                .trim()
                .to_owned();
            rig.tmux(&["set-option", "-p", "-t", &watcher, "@ae_agent", "_watchdog"]);
            let send = rig.tool.dir.join("send");
            if !send.exists() {
                std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), send)
                    .expect("real private session send helper");
            }
            let fixture = Self {
                rig,
                leaf,
                now,
                watcher,
            };
            fixture.event(
                -1_000,
                "state",
                ",\"ref\":\"working\",\"summary\":\"fixture work\"",
            );
            fixture
        }

        fn event(&self, relative: i64, action: &str, tail: &str) {
            let ts = Timestamp::from_epoch(self.now + relative).to_string();
            let uuid = uuid_of(PEER);
            let mut journal = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.leaf.dir.join("events.jsonl"))
                .expect("private event journal");
            writeln!(
                journal,
                r#"{{"ts":"{ts}","actor":"gemini:worker","action":"{action}","actor_slot":"main","actor_session":"{PEER}","actor_session_id":"{uuid}","actor_launch_id":"tok-{PEER}"{tail}}}"#
            )
            .expect("append fixture event");
        }

        fn board(&self, relative: i64) -> String {
            let stamp = Timestamp::from_epoch(self.now + relative).to_string();
            let text = format!("ae-desk 1\nupdated {stamp}\nD1\tFixture owner's manual decision\n");
            fs::write(self.rig.tool.dir.join("desk-record"), &text).expect("manual desk record");
            text
        }

        fn start(&self, channel: &str) -> (OwnedChild, Stepper) {
            let rig = &self.rig;
            let watchdog = ae()
                .arg(ae::cli::WATCHDOG_RUN)
                .arg(&rig.tool.dir)
                .args([
                    "--interval",
                    "1",
                    "--step-channel",
                    channel,
                    "--idle-nudge-secs",
                    "0",
                    "--tg-supervise-secs",
                    "0",
                ])
                .env("HOME", &rig.root)
                .env("AE_HOME", &rig.root)
                .env("CONFIG_FILE", rig.root.join("config"))
                .env("TMUX_TMPDIR", &rig.root)
                .env("AE_TMUX_SERVER_KIND", "socket")
                .env("AE_TMUX_SERVER", &rig.socket)
                .env("TMUX", format!("{},0,0", rig.socket.display()))
                .env("TMUX_PANE", &self.watcher)
                .env_remove("AE_SENDER_OVERRIDE")
                .stdout(Stdio::null())
                .stderr(
                    fs::File::create(rig.root.join(format!("{channel}.err")))
                        .expect("watchdog diagnostics"),
                )
                .spawn()
                .expect("start the real watchdog");
            let mut stepper = Stepper::new(&rig.socket, channel);
            stepper.ready();
            (watchdog, stepper)
        }

        fn receipt(&self) -> String {
            fs::read_to_string(self.rig.root.join("received"))
                .expect("submitted private pane turns")
        }

        fn checkpoint(&self) -> Value {
            let raw = fs::read_to_string(self.rig.tool.dir.join(ae::monitor::STATE_NAME))
                .expect("watchdog checkpoint");
            json::parse(&raw).expect("checkpoint object")
        }

        fn age_overview_spacing(&self) {
            self.age_spacing("overview_last_delivered_at");
        }

        fn age_spacing(&self, stamp: &str) {
            let Value::Obj(mut fields) = self.checkpoint() else {
                panic!("watchdog checkpoint should be an object");
            };
            let (_, last) = fields
                .iter_mut()
                .find(|(key, _)| key == stamp)
                .expect("the delivery under test recorded its spacing stamp");
            *last = Value::Num(self.now - 1_000);
            fs::write(
                self.rig.tool.dir.join(ae::monitor::STATE_NAME),
                Value::Obj(fields).render(),
            )
            .expect("the prior delivery's spacing has elapsed");
        }
    }

    fn owe_reseat_continuation(fixture: &Fixture) {
        let launch = "11111111-1111-4111-8111-111111111111";
        let dir = &fixture.rig.tool.dir;
        let meta = dir.join("meta");
        let old = fs::read_to_string(&meta).expect("fixture launch");
        assert!(old.lines().any(|line| line == "launch_id.main=tok-rig"));
        fs::write(
            &meta,
            format!(
                "{}reseat_continuation.main=owed:kept:{launch}\n",
                old.replace(
                    "launch_id.main=tok-rig",
                    &format!("launch_id.main={launch}")
                )
                .replace(
                    "charter_launch.main=tok-rig",
                    &format!("charter_launch.main={launch}")
                )
            ),
        )
        .expect("published successor with one owed continuation");
    }

    #[test]
    fn desklive_nudge_uses_the_one_due_reseat_continuation_and_overview_turn() {
        let fixture = Fixture::new("deskcontinuation");
        owe_reseat_continuation(&fixture);
        let dir = &fixture.rig.tool.dir;
        let manual = fixture.board(-700);
        fixture.event(
            -30,
            "state",
            r#","ref":"blocked","summary":"fixture blocked after the board update""#,
        );

        let (_watchdog, mut steps) = fixture.start("desk-reseat");
        let shown = fixture.receipt();
        assert!(
            !shown.contains("orchestrator charter update uncertain"),
            "an owed continuation silences the charter update in this cycle: {shown:?}"
        );
        let continuation = shown.find("Restore your state first").expect(&shown);
        let peer = shown.find(PEER).expect(&shown);
        let trailer = "overview; declare done.";
        assert_eq!(shown.matches(trailer).count(), 1, "{shown:?}");
        assert!(continuation < peer, "{shown:?}");
        assert_eq!(
            fs::read_to_string(fixture.rig.root.join("enters"))
                .expect("submit receipt")
                .lines()
                .count(),
            1,
            "{shown:?}"
        );
        let folded = shown.to_ascii_lowercase();
        assert_eq!(folded.matches(TICK).count(), 1, "{shown:?}");
        assert!(
            peer < folded.find(TICK).expect("desk line")
                && folded.find(TICK) < folded.find(trailer),
            "{shown:?}"
        );
        steps.step();
        assert_eq!(fixture.receipt(), shown);
        assert_eq!(
            fs::read_to_string(dir.join("desk-record")).expect("manual desk"),
            manual
        );
    }

    #[test]
    fn desklive_nudge_leaves_a_desk_only_edge_eligible_after_a_continuation() {
        let fixture = Fixture::new("deskcontinuation-only");
        let manual = fixture.board(-700);
        fixture.event(
            -800,
            "state",
            r#","ref":"done","summary":"done before the board update""#,
        );
        let (watchdog, steps) = fixture.start("desk-reseat-baseline");
        let baseline = fixture.receipt();
        assert_eq!(baseline.matches("overview; declare done.").count(), 1);
        assert!(!baseline.to_ascii_lowercase().contains(TICK));
        drop(watchdog);
        drop(steps);
        owe_reseat_continuation(&fixture);
        let world = ManuallyDrop::new(World {
            root: fixture.rig.root.clone(),
            sock: fixture.rig.socket.clone(),
        });
        let scope = format!("@{}", fixture.rig.home);
        let ack = world.run(
            Some(&fixture.rig.tool.pane),
            &[&scope, "state", "done", "Overview acknowledged."],
            &[],
        );
        assert_eq!(ack.0, Some(0), "{ack:?}");
        fixture.age_overview_spacing();
        fixture.event(
            -90,
            "ask",
            &format!(r#","ref":"{REF}","target":"telegram:42""#),
        );
        fixture.event(-80, "cancel", &format!(r#","ref":"{REF}""#));
        let before = fixture.checkpoint();

        let (watchdog, steps) = fixture.start("desk-reseat-alone");
        let shown = fixture.receipt();
        let continuation = shown.strip_prefix(&baseline).expect("append-only receipt");
        assert_eq!(
            continuation.matches("Restore your state first").count(),
            1,
            "{continuation:?}"
        );
        assert!(
            !continuation.contains("overview; declare done."),
            "{continuation:?}"
        );
        assert!(
            !continuation.to_ascii_lowercase().contains(TICK),
            "{continuation:?}"
        );
        let after = fixture.checkpoint();
        for key in ["desk_tick_last_at", "desk_tick_cursor", "overview_hash"] {
            assert_eq!(
                after.get(key),
                before.get(key),
                "continuation preserves {key}"
            );
        }
        let dir = &fixture.rig.tool.dir;
        let meta = fs::read(dir.join("meta")).expect("continuation metadata");
        assert_eq!(
            ae::meta::reseat_continuation(&meta),
            None,
            "the continuation confirmed"
        );
        drop(watchdog);
        drop(steps);

        let (_watchdog, mut steps) = fixture.start("desk-after-reseat");
        let retried = fixture.receipt();
        let tick = retried.strip_prefix(&shown).expect("append-only receipt");
        assert_eq!(
            tick.to_ascii_lowercase().matches(TICK).count(),
            1,
            "{tick:?}"
        );
        assert!(!tick.contains("overview; declare done."), "{tick:?}");
        steps.step();
        assert_eq!(fixture.receipt(), retried);
        assert_eq!(
            fs::read_to_string(dir.join("desk-record")).expect("manual desk"),
            manual
        );
    }

    #[test]
    fn desklive_nudge_shares_one_due_overview_if_the_changed_board_is_old() {
        let fixture = Fixture::new("deskage");
        fixture.board(-70);
        fixture.event(
            -40,
            "state",
            ",\"ref\":\"blocked\",\"summary\":\"fixture needs evidence\"",
        );
        let (watchdog, steps) = fixture.start("desk-fresh");
        let fresh = fixture.receipt();
        assert!(fresh.contains("desk-peer"), "{fresh:?}");
        assert!(
            fresh.lines().any(|line| {
                let mut words = line.split_whitespace();
                words.next() == Some("worker") && words.next() == Some("blocked")
            }),
            "{fresh:?}"
        );
        assert_eq!(
            fresh.matches("overview; declare done.").count(),
            1,
            "{fresh:?}"
        );
        assert!(!fresh.to_ascii_lowercase().contains(TICK), "{fresh:?}");
        drop(watchdog);
        drop(steps);

        let manual = fixture.board(-700);
        fixture.event(
            -20,
            "state",
            ",\"ref\":\"done\",\"summary\":\"fixture evidence arrived\"",
        );
        let (watchdog, steps_within_spacing) = fixture.start("desk-spacing");
        assert_eq!(
            fixture.receipt(),
            fresh,
            "recent overview holds both messages"
        );
        drop(watchdog);
        drop(steps_within_spacing);

        fixture.age_overview_spacing();
        let (_watchdog, mut steps) = fixture.start("desk-stale");
        let shown = fixture.receipt();
        let combined = shown
            .strip_prefix(&fresh)
            .expect("pane receipt is append-only");
        let folded = combined.to_ascii_lowercase();
        assert_eq!(folded.matches(TICK).count(), 1, "{combined:?}");
        assert_eq!(
            combined.matches("overview; declare done.").count(),
            1,
            "{combined:?}"
        );
        assert!(
            folded.find(TICK) < folded.find("overview; declare done."),
            "{combined:?}"
        );
        assert_eq!(
            fs::read_to_string(fixture.rig.tool.dir.join("desk-record")).expect("manual desk"),
            manual
        );
        steps.step();
        assert_eq!(fixture.receipt(), shown, "no second turn in the next cycle");
    }

    #[test]
    fn desklive_nudge_detects_a_human_ask_opened_and_closed_before_the_next_read() {
        let fixture = Fixture::new("deskask");
        fixture.board(-700);
        fixture.event(
            -800,
            "state",
            ",\"ref\":\"done\",\"summary\":\"already finished before the board\"",
        );
        let (watchdog, mut steps) = fixture.start("desk-baseline");
        let overview = fixture.receipt();
        assert_eq!(
            overview.matches("overview; declare done.").count(),
            1,
            "{overview:?}"
        );
        assert!(
            !overview.to_ascii_lowercase().contains(TICK),
            "{overview:?}"
        );
        steps.step();
        assert_eq!(
            fixture.receipt(),
            overview,
            "an old board alone sends no turn"
        );
        drop(watchdog);
        drop(steps);
        fixture.age_overview_spacing();

        fixture.event(
            -90,
            "ask",
            &format!(r#","ref":"{REF}","target":"telegram:42""#),
        );
        fixture.event(-80, "cancel", &format!(r#","ref":"{REF}""#));
        let before = fixture.checkpoint();
        let (watchdog, mut steps) = fixture.start("desk-ask-edges");
        let shown = fixture.receipt();
        let standalone = shown
            .strip_prefix(&overview)
            .expect("pane receipt is append-only");
        assert_eq!(
            standalone.to_ascii_lowercase().matches(TICK).count(),
            1,
            "{standalone:?}"
        );
        assert!(
            !standalone.contains("overview; declare done."),
            "{standalone:?}"
        );
        let after = fixture.checkpoint();
        for key in [
            "overview_hash",
            "overview_delivered_at",
            "overview_last_delivered_at",
        ] {
            assert_eq!(
                before.get(key),
                after.get(key),
                "standalone refresh preserves {key}"
            );
        }
        steps.step();
        assert_eq!(
            fixture.receipt(),
            shown,
            "the same journal makes no second turn"
        );
        drop(watchdog);
        drop(steps);
        assert_consumed_across_restarts(&fixture, &shown);
    }

    #[test]
    fn desklive_nudge_retries_a_proven_pre_paste_refusal_on_the_standalone_path() {
        assert_retries_a_proven_pre_paste_refusal(false);
    }

    #[test]
    fn desklive_nudge_retries_a_proven_pre_paste_refusal_on_the_combined_path() {
        assert_retries_a_proven_pre_paste_refusal(true);
    }

    fn assert_retries_a_proven_pre_paste_refusal(combined: bool) {
        let fixture = Fixture::new("deskprepaste");
        let manual = fixture.board(-700);
        fixture.event(
            -800,
            "state",
            r#","ref":"done","summary":"done before the board""#,
        );
        let (watchdog, steps) = fixture.start("desk-refusal-baseline");
        let baseline = fixture.receipt();
        assert_eq!(baseline.matches("overview; declare done.").count(), 1);
        assert!(!baseline.to_ascii_lowercase().contains(TICK));
        drop(watchdog);
        drop(steps);
        fixture.age_overview_spacing();

        if combined {
            fixture.event(
                -70,
                "state",
                r#","ref":"blocked","summary":"blocked after the board""#,
            );
        } else {
            fixture.event(
                -90,
                "ask",
                &format!(r#","ref":"{REF}","target":"telegram:42""#),
            );
            fixture.event(-80, "cancel", &format!(r#","ref":"{REF}""#));
        }
        let dir = &fixture.rig.tool.dir;
        let messages = dir.join("messages");
        let saved = dir.join("messages-fixture-backup");
        fs::rename(&messages, &saved).expect("keep the prior delivery bodies");
        fs::write(&messages, "a file blocks creation of the message directory")
            .expect("make the real body store refuse before paste");
        let before = fixture.checkpoint();
        let (watchdog, steps) = fixture.start("desk-refused");
        assert_eq!(fixture.receipt(), baseline, "refusal delivered nothing");
        let diagnostics = fs::read_to_string(fixture.rig.root.join("desk-refused.err"))
            .expect("send diagnostics");
        // The standalone tick did not exist on the pre-feature base.
        if combined {
            assert!(
                diagnostics.contains("message body storage failed"),
                "{diagnostics}"
            );
        }
        let after = fixture.checkpoint();
        for key in ["desk_tick_last_at", "desk_tick_cursor", "overview_hash"] {
            assert_eq!(
                after.get(key),
                before.get(key),
                "refusal must preserve {key}"
            );
        }
        drop(watchdog);
        drop(steps);
        fs::remove_file(&messages).expect("remove fixture obstruction");
        fs::rename(&saved, &messages).expect("restore body store");

        let (_watchdog, mut steps) = fixture.start("desk-retry");
        let shown = fixture.receipt();
        let retried = shown.strip_prefix(&baseline).expect("append-only receipt");
        assert_eq!(
            retried.to_ascii_lowercase().matches(TICK).count(),
            1,
            "{retried:?}"
        );
        assert_eq!(
            retried.matches("overview; declare done.").count(),
            usize::from(combined),
            "{retried:?}"
        );
        if !combined {
            assert!(
                diagnostics.contains("message body storage failed"),
                "{diagnostics}"
            );
        }
        steps.step();
        assert_eq!(fixture.receipt(), shown, "the retry submits one turn");
        assert_eq!(
            fs::read_to_string(dir.join("desk-record")).expect("manual desk"),
            manual
        );
    }

    #[test]
    fn desklive_nudge_waits_for_a_proven_main_launch_without_spending_the_edge() {
        let fixture = Fixture::new("deskbadlaunch");
        fixture.board(-700);
        fixture.event(
            -800,
            "state",
            r#","ref":"done","summary":"done before the board""#,
        );
        let (watchdog, steps) = fixture.start("desk-launch-baseline");
        let baseline = fixture.receipt();
        assert_eq!(baseline.matches("overview; declare done.").count(), 1);
        assert!(!baseline.to_ascii_lowercase().contains(TICK));
        drop(watchdog);
        drop(steps);
        fixture.age_overview_spacing();

        fixture.event(
            -90,
            "ask",
            &format!(r#","ref":"{REF}","target":"telegram:42""#),
        );
        fixture.event(-80, "cancel", &format!(r#","ref":"{REF}""#));
        let meta = fixture.rig.tool.dir.join("meta");
        let launch = fs::read_to_string(&meta).expect("proven launch metadata");
        assert!(launch.lines().any(|line| line == "launch_id.main=tok-rig"));
        fs::write(
            &meta,
            launch.replace("launch_id.main=tok-rig", "launch_id.main="),
        )
        .expect("present but damaged launch token");
        let before = fixture.checkpoint();
        let (watchdog, steps) = fixture.start("desk-unproven-launch");
        assert_eq!(
            fixture.receipt(),
            baseline,
            "an unproven launch receives no turn"
        );
        let after = fixture.checkpoint();
        for key in ["desk_tick_last_at", "desk_tick_cursor", "overview_hash"] {
            assert_eq!(
                after.get(key),
                before.get(key),
                "unproven launch preserves {key}"
            );
        }
        drop(watchdog);
        drop(steps);
        fs::write(&meta, launch).expect("restore the proven launch token");

        let (_watchdog, mut steps) = fixture.start("desk-proven-launch");
        let shown = fixture.receipt();
        let retried = shown.strip_prefix(&baseline).expect("append-only receipt");
        assert_eq!(
            retried.to_ascii_lowercase().matches(TICK).count(),
            1,
            "{retried:?}"
        );
        assert!(!retried.contains("overview; declare done."), "{retried:?}");
        steps.step();
        assert_eq!(fixture.receipt(), shown);
    }

    fn assert_consumed_across_restarts(fixture: &Fixture, shown: &str) {
        let board = fixture.rig.tool.dir.join("desk-record");
        let manual = fs::read(&board).expect("manual board before the restarts");
        fixture.age_spacing("desk_tick_last_at");
        fixture.event(-70, "send", r#","summary":"unrelated journal append""#);
        let (watchdog, steps) = fixture.start("desk-consumed");
        assert_eq!(
            fixture.receipt(),
            shown,
            "restart, elapsed spacing and an unrelated append do not rearm the old edge"
        );
        drop(watchdog);
        drop(steps);

        let next = "ae-20261121T084012Z-00000002";
        fixture.event(
            -60,
            "ask",
            &format!(r#","ref":"{next}","target":"telegram:42""#),
        );
        fixture.event(-50, "cancel", &format!(r#","ref":"{next}""#));
        let (watchdog, steps) = fixture.start("desk-next-edge");
        let next_shown = fixture.receipt();
        let next_turn = next_shown.strip_prefix(shown).expect("append-only receipt");
        assert_eq!(next_turn.to_ascii_lowercase().matches(TICK).count(), 1);
        assert!(!next_turn.contains("overview; declare done."));
        drop(watchdog);
        drop(steps);

        fixture.event(
            -40,
            "state",
            r#","ref":"blocked","summary":"a new normal overview is due""#,
        );
        let (watchdog, steps) = fixture.start("desk-holds-overview");
        assert_eq!(
            fixture.receipt(),
            next_shown,
            "the recent standalone tick also holds the normal overview"
        );
        drop(watchdog);
        drop(steps);

        fixture.age_spacing("desk_tick_last_at");
        let (_watchdog, _steps) = fixture.start("desk-releases-overview");
        let final_receipt = fixture.receipt();
        let combined = final_receipt
            .strip_prefix(&next_shown)
            .expect("append-only receipt");
        assert_eq!(combined.matches("overview; declare done.").count(), 1);
        assert_eq!(combined.to_ascii_lowercase().matches(TICK).count(), 1);
        assert_eq!(
            fs::read(&board).expect("manual board after the restarts"),
            manual
        );
    }

    #[test]
    fn desklive_nudge_uses_the_human_target_of_a_closed_review() {
        let fixture = Fixture::new("deskclose");
        let manual = fixture.board(-700);
        let review = "review-20261121T084012Z-00000003";
        let agent_ask = "ae-20261121T084012Z-00000005";
        fixture.event(
            -810,
            "state",
            r#","ref":"blocked","summary":"first reason""#,
        );
        fixture.event(
            -800,
            "review",
            &format!(r#","ref":"{review}","target":"telegram:42""#),
        );
        let owner = &fixture.rig.home;
        let owner_meta = ae::session::read_meta(&fixture.rig.tool.dir).expect("owner meta");
        let owner_id = owner_meta.session_id().expect("owner session id");
        fixture.event(
            -790,
            "ask",
            &format!(
                r#","ref":"{agent_ask}","target":"lead","target_slot":"main","target_session":"{owner}","target_session_id":"{owner_id}","target_launch_id":"tok-rig""#
            ),
        );
        let read = || ae::session::SessionRead::open(&fixture.leaf.dir).expect("review journal");
        let before = read();
        for (id, target) in [(review, "telegram:42"), (agent_ask, "lead")] {
            assert!(
                before.pending.iter().any(|request| {
                    request.id == id && request.target.as_deref() == Some(target)
                })
            );
        }
        let (watchdog, steps) = fixture.start("desk-review-open-before-update");
        let initial = fixture.receipt();
        assert_eq!(initial.matches("overview; declare done.").count(), 1);
        assert!(!initial.to_ascii_lowercase().contains(TICK), "{initial:?}");
        drop(watchdog);
        drop(steps);
        fixture.age_overview_spacing();

        fixture.event(
            -100,
            "state",
            r#","ref":"blocked","summary":"changed reason for the same state""#,
        );
        fixture.event(
            -90,
            "cancel",
            r#","ref":"review-20261121T084012Z-00000004""#,
        );
        fixture.event(-85, "cancel", &format!(r#","ref":"{agent_ask}""#));
        let after_agent_cancel = read();
        assert!(
            after_agent_cancel
                .pending
                .iter()
                .all(|request| request.id != agent_ask)
        );
        assert!(
            after_agent_cancel
                .pending
                .iter()
                .any(|request| request.id == review)
        );
        let (watchdog, steps) = fixture.start("desk-unrelated-edges");
        let held = fixture.receipt();
        assert!(!held.to_ascii_lowercase().contains(TICK), "{held:?}");
        drop(watchdog);
        drop(steps);
        fixture.age_overview_spacing();

        // The closing record has no target. Only the older opening names it.
        fixture.event(-80, "cancel", &format!(r#","ref":"{review}""#));
        assert!(read().pending.iter().all(|request| request.id != review));
        let (_watchdog, _steps) = fixture.start("desk-review-closed-after-update");
        let after = fixture.receipt();
        let posted = after.strip_prefix(&held).expect("append-only pane receipt");
        assert_eq!(
            posted.to_ascii_lowercase().matches(TICK).count(),
            1,
            "{posted:?}"
        );
        assert_eq!(
            fs::read_to_string(fixture.rig.tool.dir.join("desk-record")).expect("manual desk"),
            manual
        );
    }
}

#[test]
fn the_released_orchestrator_prompt_carries_the_chief_of_staff_charter() {
    let config = include_str!("../../contrib/aeorchestrator/orchestrator.config");
    let charter = include_str!("../../contrib/aeorchestrator/CHARTER.md");
    let (_, prompt) = config.split_once("[prompt]\n").expect("template prompt");
    assert!(prompt.contains(MARKER));
    assert!(charter.to_ascii_lowercase().contains("chief of staff"));
}

fn prepare_old_launch(rig: &Rig, global: &Path, meta: &Path) {
    fs::write(global, "[workspace]\nquota = off\nauto_upgrade = off\n")
        .expect("no external tools in the watchdog cycle");
    let local = rig.root.join("orchestrator.config");
    let (prompt, end) = CURRENT_OVERLAY
        .rsplit_once("\n\"\"\"")
        .expect("current prompt delimiter");
    fs::write(&local, format!("{prompt}\n{OWNER_LINE}\n\"\"\"{end}"))
        .expect("a compatible overlay with the current charter marker");
    let current = fs::read_to_string(meta).expect("live fixture meta");
    let solo = current
        .lines()
        .filter(|line| !line.contains(".worker.0="))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("layout=lead-pair", "layout=vertical");
    fs::write(
        meta,
        format!(
            "{solo}\nmeta_agent=true\nmain_pane={}\nlocal_config={}\nconfig={}\n\
         charter.main=2\ncharter_launch.main=tok-older-launch\n",
            rig.tool.pane,
            local.display(),
            global.display()
        ),
    )
    .expect("the current launch has only an older launch's charter stamp");
}

fn start_watchdog(rig: &Rig, global: &Path) -> (OwnedChild, Stepper) {
    let channel = "orchexec-charter-step";
    let pane = rig.tmux(&[
        "new-window",
        "-d",
        "-t",
        &rig.home,
        "-P",
        "-F",
        "#{pane_id}",
        "sleep",
        "60",
    ]);
    let pane = pane.trim();
    rig.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "_watchdog"]);
    let watchdog = ae()
        .arg(ae::cli::WATCHDOG_RUN)
        .arg(&rig.tool.dir)
        .args([
            "--interval",
            "1",
            "--step-channel",
            channel,
            "--idle-nudge-secs",
            "0",
            "--tg-supervise-secs",
            "0",
        ])
        .env("HOME", &rig.root)
        .env("AE_HOME", &rig.root)
        .env("CONFIG_FILE", global)
        .env("TMUX_TMPDIR", &rig.root)
        .env("AE_TMUX_SERVER_KIND", "socket")
        .env("AE_TMUX_SERVER", &rig.socket)
        .env("TMUX", format!("{},0,0", rig.socket.display()))
        .env("TMUX_PANE", pane)
        .env_remove("AE_SENDER_OVERRIDE")
        .stdout(Stdio::null())
        .stderr(
            fs::File::create(rig.root.join("charter-watchdog.err")).expect("watchdog diagnostics"),
        )
        .spawn()
        .expect("start the real stepped watchdog");
    (watchdog, Stepper::new(&rig.socket, channel))
}

fn assert_charter_notice(dir: &Path, receipt: &str) {
    assert_eq!(receipt.matches(CHARTER_HEAD).count(), 1, "{receipt:?}");
    let line = receipt
        .lines()
        .find(|line| line.contains(CHARTER_HEAD))
        .expect("a visible charter heading");
    let (_, notice) = line.split_once(" LONG BODY ").expect("long charter notice");
    let (_, body) = notice
        .split_once(" B in your session dir: messages/")
        .expect("the notice names a saved body in its own session");
    let (file, _) = body
        .split_once(" — ")
        .expect("the notice delimits the path");
    assert!(!file.contains('/'), "expected one filename: {file:?}");
    assert!(
        Path::new(file).extension().is_some_and(|ext| ext == "txt"),
        "expected a saved text body: {file:?}"
    );
    let instructions = fs::read_to_string(dir.join("messages").join(file))
        .expect("the saved body named by the submitted notice");
    assert!(instructions.contains(CHARTER_HEAD));
    assert!(instructions.contains(MARKER));
    assert!(instructions.contains(OWNER_LINE));
}

#[test]
fn a_running_dispatcher_gets_one_charter_turn_for_its_current_launch_when_done() {
    let rig = Rig::new("execcharter");
    super::orchrename_charter_spec::turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_old_launch(&rig, &global, &meta);
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let scope = format!("@{}", rig.home);
    let declare = |state| {
        let result = world.run(
            Some(&rig.tool.pane),
            &[
                &scope,
                "state",
                state,
                "The fixture marks the dispatcher through its ordinary state helper.",
            ],
            &[],
        );
        assert_eq!(result.0, Some(0), "{}", words(&result));
    };
    declare("working");
    let receipt = || fs::read_to_string(rig.root.join("received")).expect("actual pane receipt");
    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    assert!(!receipt().contains(CHARTER_HEAD));
    assert!(
        fs::read_to_string(&meta)
            .expect("running meta")
            .contains("charter_launch.main=tok-older-launch\n")
    );

    declare("done");
    let draft = "A real unfinished private draft blocks delivery.";
    rig.raw(&rig.tool.pane, draft);
    rig.wait(&rig.tool.pane, "occupied composer", |screen| {
        screen.contains(draft)
    });
    stepper.step();
    assert!(!receipt().contains(CHARTER_HEAD));
    let undelivered = fs::read_to_string(&meta).expect("meta after refused delivery");
    assert!(undelivered.contains("charter_launch.main=tok-older-launch\n"));
    assert!(!undelivered.contains("charter_launch.main=tok-rig\n"));
    assert!(
        rig.screen(&rig.tool.pane).contains(draft),
        "the watchdog preserved the draft"
    );
    rig.key(&rig.tool.pane, "C-u");
    rig.wait(&rig.tool.pane, "cleared composer", |screen| {
        !screen.contains(draft)
    });
    stepper.step();
    let delivered = receipt();
    assert_charter_notice(&rig.tool.dir, &delivered);
    let current = fs::read_to_string(&meta).expect("updated live meta");
    assert!(current.lines().any(|line| line == "charter.main=3"));
    assert!(
        current
            .lines()
            .any(|line| line == "charter_launch.main=tok-rig")
    );
    assert!(!current.contains("charter_launch.main=tok-older-launch"));
    stepper.steps(2);
    assert_eq!(receipt().matches(CHARTER_HEAD).count(), 1);
}

#[test]
fn orchwake_continuation_gets_its_own_cycle_and_leaves_the_charter_owed() {
    let rig = Rig::new("orchwake-charter");
    super::orchrename_charter_spec::turn_fixture(&rig);
    let global = rig.root.join("config");
    let meta = rig.tool.dir.join("meta");
    prepare_old_launch(&rig, &global, &meta);
    let launch = "11111111-1111-4111-8111-111111111111";
    let initial = fs::read_to_string(&meta).expect("fixture meta");
    fs::write(
        &meta,
        format!(
            "{}sweep_sec=0\nreseat_continuation.main=owed:kept:{launch}\n",
            initial.replace(
                "launch_id.main=tok-rig",
                &format!("launch_id.main={launch}")
            )
        ),
    )
    .expect("fixture holds an owed continuation and an obsolete charter stamp");
    let send = rig.tool.dir.join("send");
    if !send.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &send).expect("real session send");
    }
    let world = ManuallyDrop::new(World {
        root: rig.root.clone(),
        sock: rig.socket.clone(),
    });
    let scope = format!("@{}", rig.home);
    let done = world.run(
        Some(&rig.tool.pane),
        &[&scope, "state", "done", "The seat is ready."],
        &[],
    );
    assert_eq!(done.0, Some(0), "{}", words(&done));
    let receipt = || fs::read_to_string(rig.root.join("received")).expect("actual pane receipt");

    let (_watchdog, mut stepper) = start_watchdog(&rig, &global);
    stepper.ready();
    let first = receipt();
    assert_eq!(
        first.matches("Restore your state first").count(),
        1,
        "{first:?}\n{}",
        fs::read_to_string(rig.root.join("charter-watchdog.err")).expect("watchdog diagnostics")
    );
    assert!(!first.contains(CHARTER_HEAD), "{first:?}");
    let after_first = fs::read_to_string(&meta).expect("first cycle metadata");
    assert!(after_first.contains("charter_launch.main=tok-older-launch\n"));
    assert!(!after_first.contains("reseat_continuation.main="));

    stepper.step();
    let second = receipt();
    assert_charter_notice(&rig.tool.dir, &second);
    assert_eq!(
        second.matches("Restore your state first").count(),
        1,
        "{second:?}"
    );
    assert!(
        fs::read_to_string(&meta)
            .expect("second cycle metadata")
            .contains(&format!("charter_launch.main={launch}\n"))
    );
}
