//! #250 O2 phase 3 frozen acceptance (r2, lead ruling d14a29ec): no agent OUTSIDE an
//! orchestrator-role session initiates into it by any spelling, ingress, flag, gap or
//! forgery; same-session, ordinary and owner routes still land; the one entry is ONE
//! reply bound to the role's opening. Refusals name [`ROLE`], a no-tmux one also [`OWNER`],
//! a changed replier incarnation [`BINDING`]. Per-cell wrong implementations: receipt.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories; the capability boundary is \
              about what PRODUCT code may reach"
)]

use std::path::Path;
use std::time::Duration;

use ae::cli::{ACCEPT, ASK, GOAL, INTERRUPT, REPLY, REVIEW, SEND, SPAWN, WITHDRAW};

use super::o2guard_p2_spec::{Seat, World, ledger, opening, received, uuid_of};

const ROLE: &str = "orchestrator role";
const OWNER: [&str; 3] = ["ae chat", "ae app", "Telegram"];
const BINDING: &str = "seat binding";
const XS: &str = "--cross-session";
const SENDER: &str = "AE_SENDER_OVERRIDE";
const ACTION: &str = "_AE_EVENT_ACTION";
/// The environment the watchdog's one exec site hands its `send` child.
const FORGE: [(&str, &str); 2] = [(SENDER, "watchdog"), (ACTION, "sweep-nudge")];

/// `orc` is the role (not named `orchestrator`); `orchestrator` is ORDINARY.
fn fleet(tag: &str) -> (World, Seat, Seat, Seat) {
    let world = World::new(tag);
    let orc = world.session("orc", "boss", "meta_agent=true\n");
    let wrk = world.session("wrk", "wkr", "");
    let ord = world.session("orchestrator", "ody", "");
    (world, orc, wrk, ord)
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn tmux(world: &World, words: &str, tail: &[&str]) -> String {
    let (ok, out) = world.tmux(words, tail);
    assert!(ok, "setup: tmux {words} {tail:?}");
    out.trim().to_owned()
}

/// A second pane in `seat`'s tmux session, stamped `slot`/`agent`.
fn pane_in(world: &World, seat: &Seat, slot: &str, agent: &str) -> String {
    let split = "split-window -d -P -F #{pane_id} -t";
    let pane = tmux(world, split, &[&seat.pane, "sleep 600"]);
    if !agent.is_empty() {
        world.stamp(&pane, slot, agent);
    }
    pane
}

fn meta_of(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("meta")).expect("meta")
}

fn add_meta(dir: &Path, row: &str) {
    assert!(std::fs::write(dir.join("meta"), meta_of(dir) + row).is_ok());
}

fn swap_meta(dir: &Path, from: &str, to: &str) {
    let meta = meta_of(dir);
    assert!(meta.contains(from), "setup: meta holds {from}");
    assert!(std::fs::write(dir.join("meta"), meta.replacen(from, to, 1)).is_ok());
}

type Out = (Option<i32>, String, String);
type Envs<'a> = &'a [(&'a str, &'a str)];

/// A refusal that names `phrase`, or the reason it is not one.
fn refusal(label: &str, out: &Out, phrase: &str) -> Option<String> {
    (out.0 == Some(0) || !out.2.contains(phrase))
        .then(|| format!("{label}: rc {:?} {}", out.0, out.2.trim()))
}

fn count(text: &str, needle: &str) -> usize {
    text.matches(needle).count()
}

/// Under-lock refusal audits in `dir`'s ledger (A2: `refused` or `delivery-abandoned`).
fn audits(dir: &Path) -> usize {
    let text = ledger(dir);
    count(&text, "\"action\":\"refused\"") + count(&text, "delivery-abandoned")
}

/// `pane`'s per-target send-lock, the one every delivery takes.
fn lock_path(world: &World, pane: &str) -> std::path::PathBuf {
    let locks = world.root.join("sessions/.locks");
    assert!(std::fs::create_dir_all(&locks).is_ok());
    locks.join(format!("send-lock-{}", pane.replace('%', "_")))
}

fn hold_lock(world: &World, pane: &str) -> std::fs::File {
    ae::store::lock(&lock_path(world, pane), Duration::ZERO).expect("lock")
}

/// Poll `until` every 10 ms for at most `secs`.
fn wait_for(secs: u64, until: impl Fn() -> bool) -> bool {
    let tick = |_| until() || (std::thread::sleep(Duration::from_millis(10)), false).1;
    (0..secs * 100).any(tick)
}

/// Stored delivery bodies under every session directory of `root`.
fn bodies(root: &Path) -> usize {
    let sessions = ["sessions", "forged/sessions"].map(|dir| root.join(dir));
    let read = |dir: &std::path::PathBuf| std::fs::read_dir(dir).ok();
    let dirs = sessions.iter().filter_map(read);
    let messages = dirs.flatten().flatten().map(|e| e.path().join("messages"));
    messages
        .filter_map(|dir| read(&dir))
        .map(Iterator::count)
        .sum()
}

/// Run the PHYSICAL helper link `<seat dir>/<name>` (a symlink to the core,
/// which dispatches on its basename and directory) inside `pane`.
fn link_run(world: &World, seat: &Seat, pane: &str, name: &str, tail: &[&str]) -> Out {
    let link = seat.dir.join(name);
    let _ = std::fs::remove_file(&link);
    assert!(std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), &link).is_ok());
    let mut command = super::cli::helper(&link);
    command.env("AE_HOME", &world.root).env_remove(SENDER);
    let tmux = format!("{},0,0", world.sock.display());
    command.env("TMUX", tmux).env("TMUX_PANE", pane);
    let out = command.args(tail).output().expect("helper runs");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (out.status.code(), text(&out.stdout), text(&out.stderr))
}

fn set_uuid(world: &World, pane: &str, name: &str) {
    let uuid = uuid_of(name);
    tmux(world, "set-option -t", &[pane, "@ae_session_uuid", &uuid]);
}

// G1-G4, G8 negative, R1, R3, C4, C5: every spelling (`s:a`, `@s:a`, `%pane`, the role's
// helper dir, the physical LINK, `ae @scope`, a forged role-less helper root) x both flag
// states x every verb, then the caller gaps (NoSession + OWNER and unstamped for every verb;
// forged watchdog env BEFORE the forged uuid option, the same name on a 2nd socket, a
// partial or malformed pair, damaged role meta). Each refusal is pre-admission: no body, no
// opening anywhere, one `refused` audit in an existing caller ledger (none under the forged
// root). Split in two for the 60 s rule.
#[test]
fn no_outside_caller_sends_asks_or_reviews_into_a_role_session_by_any_route() {
    matrix("mx-msg", 0..3, false);
}

#[test]
fn no_outside_caller_interrupts_a_role_session_by_any_route_or_caller_gap() {
    matrix("mx-int", 3..5, true);
}

fn matrix(tag: &str, verbs: std::ops::Range<usize>, gaps: bool) {
    let (world, orc, wrk, _ord) = fleet(tag);
    let damaged = world.session("orx", "bx", "meta_agent=true\nmeta_agent=true\n");
    let forged = world.root.join("forged/sessions/orc");
    assert!(std::fs::create_dir_all(&forged).is_ok());
    let meta = meta_of(&orc.dir).replace("meta_agent=true\n", "");
    assert!(std::fs::write(forged.join("meta"), meta).is_ok());
    let alien = World::new(&format!("{tag}-alien"));
    let renamed = alien.session("orc", "boss", "");
    set_uuid(&alien, &renamed.pane, "orc");
    let unstamped = world.session("nob", "x", "");
    let un = unstamped.pane.as_str();
    tmux(&world, "set-option -p -u -t", &[un, "@ae_agent"]);
    let (wd, od, fd) = (s(&wrk.dir), s(&orc.dir), s(&forged));
    let mut wrong = Vec::new();
    let mut gap = Vec::new();
    let mut audited = |label: String, audit: bool, run: &dyn Fn() -> Out| {
        let before = count(&ledger(&wrk.dir), "\"action\":\"refused\"");
        wrong.extend(refusal(&label, &run(), ROLE));
        let n = count(&ledger(&wrk.dir), "\"action\":\"refused\"") - before;
        wrong.extend((audit && n != 1).then(|| format!("{label}: {n} refused audits, not 1")));
    };
    let all = [(SEND, "hi"), (ASK, "q"), (REVIEW, "r")];
    let all = [&all[..], &[(INTERRUPT, "stop"), (INTERRUPT, "")]].concat();
    let w = Some(wrk.pane.as_str());
    for (verb, body) in all[verbs].iter().copied() {
        let text: Vec<&str> = (!body.is_empty()).then_some(body).into_iter().collect();
        let orc_pane = orc.pane.as_str();
        let scopes = [(&wd, "orc:boss"), (&wd, "@orc:boss"), (&wd, orc_pane)];
        let scopes = [&scopes[..], &[(&od, "boss"), (&fd, "boss")]].concat();
        for ((dir, target), flag) in scopes.iter().flat_map(|sc| [(sc, true), (sc, false)]) {
            let mut argv = vec![verb, dir.as_str()];
            argv.extend(flag.then_some(XS));
            argv.extend(std::iter::once(*target).chain(text.iter().copied()));
            let audit = *dir != &fd;
            audited(format!("{argv:?}"), audit, &|| world.run(w, &argv, &[]));
            if *dir == &od {
                let short = [&["@orc", verb.trim_start_matches('_')][..], &argv[2..]].concat();
                audited(format!("{short:?}"), true, &|| world.run(w, &short, &[]));
            }
        }
        if verb != REVIEW && !body.is_empty() {
            let name = verb.trim_start_matches('_');
            let tail = [&[XS, "orc:boss"][..], &text].concat();
            let link = || link_run(&world, &wrk, &wrk.pane, name, &tail);
            audited(format!("link {name}"), true, &link);
        }
        for (label, pane) in [("no TMUX", None), ("unstamped pane", Some(un))] {
            let argv = [&[verb, od.as_str(), XS, "boss"][..], &text].concat();
            let out = world.run(pane, &argv, &[]);
            let label = format!("{label} {verb}");
            gap.extend(refusal(&label, &out, ROLE));
            let owner = OWNER.iter().filter(|_| pane.is_none());
            gap.extend(owner.filter_map(|word| refusal(&label, &out, word)));
        }
    }
    wrong.append(&mut gap);
    let alien_tmux = format!("{},0,0", alien.sock.display());
    let r = Some(renamed.pane.as_str());
    let at = [("TMUX", alien_tmux.as_str())];
    let (bad, part) = ([("TMUX", "nonsense")], [("TMUX_PANE", "")]);
    let cells: [(&str, Option<&str>, &str, &str, Envs); 6] = [
        ("forged watchdog env", w, &od, "boss", &FORGE),
        ("forged uuid option", w, &od, "boss", &[]),
        ("renamed on 2nd socket", r, &od, "boss", &at),
        ("partial pair", w, &wd, "orc:boss", &part),
        ("malformed TMUX", w, &wd, "orc:boss", &bad),
        ("damaged role meta", w, &wd, "orx:bx", &[]),
    ];
    let gap_cells = cells.into_iter().enumerate().take(if gaps { 6 } else { 0 });
    for (n, (label, pane, dir, target, envs)) in gap_cells {
        if n == 1 {
            set_uuid(&world, &wrk.pane, "orc");
        }
        let out = world.run(pane, &[SEND, dir, XS, target, "hi"], envs);
        wrong.extend(refusal(label, &out, ROLE));
    }
    for (name, seat) in [("orc", &orc), ("orx", &damaged)] {
        let got = received(seat);
        wrong.extend((!got.is_empty()).then(|| format!("{name} pane received {got:?}")));
    }
    let fw = world.root.join("forged/sessions/wrk");
    for dir in [&orc.dir, &damaged.dir, &wrk.dir, &forged, &fw] {
        let text = ledger(dir);
        let opened = count(&text, "\"action\":\"ask\"") + count(&text, "\"action\":\"review\"");
        let shown = dir.display();
        wrong.extend((opened > 0).then(|| format!("{shown} holds {opened} openings")));
    }
    let stored = bodies(&world.root);
    wrong.extend((stored > 0).then(|| format!("{stored} bodies stored before admission")));
    assert!(wrong.is_empty(), "{} wrong: {wrong:#?}", wrong.len());
}

// G1b (B2, I1, I2, C4-C6, L4): `goal` set and clear, `ae compact` and `spawn` into the role
// refuse from the wrk pane, NoSession (+OWNER) and an unstamped pane, with the seeded goal
// intact after EACH op and no record, window, file or seat left; the role's own goal, compact
// and spawn land.
#[test]
fn goal_compact_and_spawn_into_a_role_session_refuse_from_outside_it() {
    let (world, orc, wrk, _ord) = fleet("g1b");
    let config = world.root.join("config");
    assert!(std::fs::write(&config, "[profiles]\nx = \"gemini\"\n").is_ok());
    let (root, cfg) = (world.root.display(), config.display());
    let rows = format!("schema=2\nmeta_version=4\nwork_dir={root}\nconfig={cfg}\ngoal=seed\n");
    add_meta(&orc.dir, &rows);
    let goal_is = |want: &str| {
        let meta = meta_of(&orc.dir);
        count(&meta, "\ngoal=") == 1 && meta.contains(&format!("\ngoal={want}\n"))
    };
    let nob = world.session("nob", "x", "");
    tmux(&world, "set-option -p -u -t", &[&nob.pane, "@ae_agent"]);
    let files = || {
        let entries = std::fs::read_dir(&orc.dir).expect("dir").flatten();
        let names = entries.map(|e| e.file_name().to_string_lossy().into_owned());
        let keep = |n: &String| !n.starts_with('.') && n != "events.jsonl" && n != "meta";
        let mut names: Vec<String> = names.filter(keep).collect();
        names.sort();
        names
    };
    let windows = || tmux(&world, "list-windows -F #{window_id} -t", &[&orc.pane]);
    let before = (files(), windows());
    let od = s(&orc.dir);
    let mut wrong = Vec::new();
    let callers = [("wrk", Some(wrk.pane.as_str())), ("no TMUX", None)];
    let callers = [&callers[..], &[("unstamped", Some(nob.pane.as_str()))]].concat();
    for (n, (who, pane)) in callers.into_iter().enumerate() {
        let mole = format!("mole{n}");
        let cells: [(&str, &[&str]); 4] = [
            ("goal", &[GOAL, &od, "outside goal"]),
            ("goal clear", &[GOAL, &od, "--clear"]),
            ("compact", &["compact", "orc"]),
            ("spawn", &[SPAWN, &od, &mole, "--using", "x"]),
        ];
        for (label, argv) in cells {
            let label = format!("{who} {label}");
            let out = world.run(pane, argv, &[]);
            wrong.extend(refusal(&label, &out, ROLE));
            let owner = OWNER.iter().filter(|_| pane.is_none());
            wrong.extend(owner.filter_map(|word| refusal(&label, &out, word)));
            wrong.extend((!goal_is("seed")).then(|| format!("{label}: the goal changed")));
        }
    }
    let (journal, meta) = (ledger(&orc.dir), meta_of(&orc.dir));
    for (bad, what) in [
        (journal.contains("\"action\":\"goal\""), "goal recorded"),
        (journal.contains("seat-compact-run"), "outside compact ran"),
        (meta.contains("seat.spawned"), "outside spawn seated"),
        ((files(), windows()) != before, "spawn left a window/file"),
        (!received(&orc).is_empty(), "role pane received"),
    ] {
        wrong.extend(bad.then(|| what.to_owned()));
    }
    let own = |argv: &[&str]| world.run(Some(&orc.pane), argv, &[]);
    let goal = own(&[GOAL, &od, "own goal"]);
    let compact = own(&["compact", "orc"]);
    let spawn = own(&[SPAWN, &od, "kin", "--using", "x"]);
    let seated = |l: &str| l.starts_with("seat.spawned.") && l.ends_with("=kin");
    let kin = meta_of(&orc.dir).lines().any(seated);
    let ran = ledger(&orc.dir).contains("seat-compact-run");
    let landed = [goal.0 == Some(0) && goal_is("own goal"), ran, kin];
    let cells = [("goal", goal), ("compact", compact), ("spawn", spawn)];
    for ((label, out), landed) in cells.into_iter().zip(landed) {
        let why = || format!("own {label}: rc {:?} {}", out.0, out.2.trim());
        wrong.extend((out.2.contains(ROLE) || !landed).then(why));
    }
    assert!(wrong.is_empty(), "{} wrong: {wrong:#?}", wrong.len());
}

// G5 lock-wait (A1, A2): a send and a bare interrupt wait on ordinary `orchestrator:ody`'s
// held send-lock; it becomes a role; release. Catches admission judged only before the lock.
// No causal claim (residual "lock-wait overlap by timing"); a recovery body is allowed, but
// no opening, one audit each and no key.
#[test]
fn a_target_that_becomes_a_role_while_a_delivery_waits_on_its_lock_gets_no_key() {
    let (world, _orc, wrk, ord) = fleet("lock");
    let (wd, ody) = (s(&wrk.dir), "orchestrator:ody");
    let (send, bare) = ([SEND, &wd, XS, ody, "late"], [INTERRUPT, &wd, XS, ody]);
    let cells: [&[&str]; 2] = [&send, &bare];
    let held = hold_lock(&world, &ord.pane);
    let outs: [Out; 2] = std::thread::scope(|scope| {
        let (w, pane) = (&world, wrk.pane.as_str());
        let runs = cells.map(|argv| scope.spawn(move || w.run(Some(pane), argv, &[])));
        std::thread::sleep(Duration::from_secs(1));
        add_meta(&ord.dir, "meta_agent=true\n");
        drop(held);
        runs.map(|run| run.join().expect("run"))
    });
    let mut wrong: Vec<String> = (0..2)
        .filter_map(|n| refusal(&format!("cell {n}"), &outs[n], ROLE))
        .collect();
    let (n, opened) = (
        audits(&wrk.dir),
        count(&ledger(&wrk.dir), "\"action\":\"ask\""),
    );
    wrong.extend((n != 2 || opened > 0).then(|| format!("{n} audits, {opened} asks")));
    let got = received(&ord);
    wrong.extend((!got.is_empty()).then(|| format!("keys reached a role: {got:?}")));
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A view of a delivery-suite rig's own server and scratch; the rig owns both,
/// so this view never drops.
fn view(rig: &super::deliver::Rig) -> std::mem::ManuallyDrop<World> {
    let scratch = rig.dir.parent().and_then(Path::parent).expect("scratch");
    let sock = scratch.join("sock");
    std::mem::ManuallyDrop::new(World {
        root: scratch.to_owned(),
        sock,
    })
}

// G5 wait-internal (C1, A1, A2). `role` and `escape` are the CAUSAL cells: the target becomes
// a role once the flush Enter is RECEIVED (before the paste), or once the message interrupt's
// Escape is RECEIVED (500 ms settle, before the paste). `token` (launch_id change, plan r4:49):
// the send-lock seen held, 150 ms more, no key yet (before `flush_staged_chip`'s Enter; timing,
// not causal). Each refuses with no further key, no body, and exactly one caller audit.
// Catches binding checked once at the lock, a missing recheck before the flush Enter, after
// it, or before the paste that follows an Escape, and a recheck that refuses unaudited.
#[test]
fn a_target_that_changes_inside_the_delivery_wait_gets_no_further_key() {
    let rig = super::deliver::Rig::new("o2chip", "muse", 0);
    let chip = view(&rig);
    let r = chip.root.display();
    add_meta(&rig.dir, &format!("session_id={}\n", uuid_of("dlo2chip")));
    set_uuid(&chip, &rig.pane, "dlo2chip");
    let caller = chip.session("cal", "cal", "");
    let (cd, lock) = (s(&caller.dir), lock_path(&chip, &rig.pane));
    let (enters, got) = (chip.root.join("enters"), chip.root.join("received"));
    let read = |path: &Path| std::fs::read_to_string(path).unwrap_or_default();
    let mut wrong = Vec::new();
    let tok = ("launch_id.main=tok-rig\n", "launch_id.main=tok-new\n");
    let role = ("seat.main=tui\n", "seat.main=tui\nmeta_agent=true\n");
    for (label, (from, to), phrase, keys) in [("token", tok, "", 0), ("role", role, ROLE, 1)] {
        assert!(std::fs::write(&enters, "").is_ok());
        let tui = format!("exec perl {r}/faketui.pl {r}/received {r}/enters 400 muse 0 staged");
        tmux(&chip, "respawn-pane -k -t", &[&rig.pane, &tui]);
        std::thread::sleep(Duration::from_millis(800));
        let argv = [SEND, &cd, XS, "dlo2chip:tui", label];
        let before = audits(&caller.dir);
        let out = std::thread::scope(|scope| {
            let run = scope.spawn(|| chip.run(Some(&caller.pane), &argv, &[]));
            let inside = if keys == 0 {
                let held = || ae::store::lock(&lock, Duration::ZERO).is_err();
                let seen = wait_for(10, held);
                std::thread::sleep(Duration::from_millis(150));
                seen && read(&enters).is_empty()
            } else {
                wait_for(10, || !read(&enters).is_empty()) && !read(&got).contains(label)
            };
            wrong.extend((!inside).then(|| format!("{label}: setup: not inside the wait")));
            swap_meta(&rig.dir, from, to);
            run.join().expect("run")
        });
        wrong.extend(refusal(label, &out, phrase));
        let (n, body) = (count(&read(&enters), "enter"), read(&got).contains(label));
        wrong.extend((n != keys || body).then(|| format!("{label}: {n} Enters, body {body}")));
        let n = audits(&caller.dir) - before;
        wrong.extend((n != 1).then(|| format!("{label}: {n} caller audits, not 1")));
    }
    wrong.extend(escape_cell());
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// The message-interrupt cell, on a framed claude seat that records the wire.
fn escape_cell() -> Vec<String> {
    let rig = super::deliver::Rig::new("o2esc", "claude", 0);
    let esc = view(&rig);
    let r = esc.root.display();
    let idle = include_str!("../fixtures/harness-state/claude-idle-167x40.txt");
    for (file, text) in [("control", idle), ("control.onescape", idle)] {
        assert!(std::fs::write(esc.root.join(file), text).is_ok());
    }
    let tui = format!("exec perl {r}/faketui.pl {r}/received {r}/enters 400 claude 0 frame");
    let tui = format!("{tui} {r}/control");
    tmux(&esc, "respawn-pane -k -t", &[&rig.pane, &tui]);
    add_meta(&rig.dir, &format!("session_id={}\n", uuid_of("dlo2esc")));
    set_uuid(&esc, &rig.pane, "dlo2esc");
    let caller = esc.session("cal", "cal", "");
    let cd = s(&caller.dir);
    let argv = [INTERRUPT, &cd, XS, "dlo2esc:tui", "late"];
    let moved = esc.root.join("control.transitions");
    let mut wrong = Vec::new();
    let before = audits(&caller.dir);
    let out = std::thread::scope(|scope| {
        let run = scope.spawn(|| esc.run(Some(&caller.pane), &argv, &[]));
        let seen = || std::fs::read_to_string(&moved).is_ok_and(|t| t.contains("onescape"));
        let inside = wait_for(20, seen);
        wrong.extend((!inside).then(|| "escape: setup: no Escape arrived".to_owned()));
        add_meta(&rig.dir, "meta_agent=true\n");
        run.join().expect("run")
    });
    wrong.extend(refusal("escape", &out, ROLE));
    let n = audits(&caller.dir) - before;
    wrong.extend((n != 1).then(|| format!("escape: {n} caller audits, not 1")));
    assert!(std::fs::write(esc.root.join("control.drain"), "drain").is_ok());
    tmux(&esc, "wait-for", &[&format!("{r}/control.ack")]);
    let wire = std::fs::read(esc.root.join("received.wire")).unwrap_or_default();
    wrong.extend((wire != b"\x1b").then(|| format!("escape: wire {wire:?}")));
    wrong
}

/// Rewrite `from` to `to` in request `id`'s opening in each ledger; a missing fault is a
/// COLLECTED wrong entry, so base runs on and a candidate must apply it (C-N, lead N1).
fn edit(wrong: &mut Vec<String>, dirs: &[&Path], id: &str, from: &str, to: &str) {
    for dir in dirs {
        let text = ledger(dir);
        let line = opening(&text, "ask", id);
        let edited = text.replace(&line, &line.replacen(from, to, 1));
        if !line.contains(from) || std::fs::write(dir.join("events.jsonl"), edited).is_err() {
            wrong.push(format!("edit {id}: {} lacks {from}", dir.display()));
        }
    }
}

// G6/G7 (R2, I4, I7, N1, N2, C2, C3, L2): a role-opened request takes ONE reply, from the
// incarnation it was sent to. Refused: `again`; `race` (exactly one of two lands, one durable
// `reply` record, a third refused; overlap by timing); `div` (mirror differs); `legacy` (no
// keys, and an absent token is never written as one); `emptya`/`emptyt`/`torna`/`tornt` (an
// empty or missing key at either end); `cancel`; `slot` (a REGISTERED other seat); `disp`
// (`--as`); `foreign` (another session's pane through wrk's helper dir); `change` and its
// `accept` (BINDING); `asker` (the role reseated after asking: BINDING). Land: `first`,
// `relaunch` (new pane, same token), `review`. Distinct tokens per end catch a swap.
#[test]
#[allow(clippy::too_many_lines, reason = "one table of reply cells")]
fn a_role_opened_ask_takes_one_reply_from_the_incarnation_it_was_sent_to() {
    let (world, orc, wrk, ord) = fleet("reply");
    let (od, wd, wp) = (s(&orc.dir), s(&wrk.dir), wrk.pane.as_str());
    let role = |argv: &[&str]| world.run(Some(&orc.pane), argv, &[]);
    let open = |verb: &str, q: &str| {
        let out = role(&[verb, &od, XS, "wrk:wkr", q]);
        assert_eq!(out.0, Some(0), "setup: the role opens: {}", out.2);
        out.1.trim().to_owned()
    };
    let reply = |pane: &str, tail: &[&str]| {
        let argv = [&[REPLY, &wd][..], tail].concat();
        world.run(Some(pane), &argv, &[])
    };
    let both = vec![orc.dir.as_path(), wrk.dir.as_path()];
    swap_meta(&orc.dir, "launch_id.main=tok-orc\n", "");
    let legacy = open(ASK, "q-legacy");
    add_meta(&orc.dir, "launch_id.main=tok-orc\n");
    let mut wrong = Vec::new();
    if opening(&ledger(&orc.dir), "ask", &legacy).contains("actor_launch_id") {
        wrong.push("an absent token was written as a key".to_owned());
    }
    let names = "first race div emptya emptyt torna tornt cancel slot disp foreign relaunch change";
    let all = names.split(' ').chain(["accept", "asker"]);
    let mut id: std::collections::BTreeMap<&str, String> = all
        .map(|name| (name, open(ASK, &format!("q-{name}"))))
        .collect();
    id.insert("legacy", legacy);
    id.insert("review", open(REVIEW, "q-review"));
    let landed = reply(wp, &[&id["first"], "r-first"]);
    assert_eq!(landed.0, Some(0), "setup: bound seat answers: {}", landed.2);
    let (a, t) = ("\"actor_launch_id\":", "\"target_launch_id\":");
    let (orc_tok, wrk_tok) = (format!("{a}\"tok-orc\""), format!("{t}\"tok-wrk\""));
    let div = ("\"q-div\"".to_owned(), "\"q-forged\"".to_owned());
    let faults = [
        ("div", vec![wrk.dir.as_path()], div.0, div.1),
        ("emptya", both.clone(), orc_tok.clone(), format!("{a}\"\"")),
        ("emptyt", both.clone(), wrk_tok.clone(), format!("{t}\"\"")),
        ("torna", both.clone(), format!(",{orc_tok}"), String::new()),
        ("tornt", both, format!(",{wrk_tok}"), String::new()),
    ];
    for (name, dirs, from, to) in &faults {
        edit(&mut wrong, dirs, &id[name], from, to);
    }
    let gone = role(&[WITHDRAW, &od, &id["cancel"], "moot"]);
    assert_eq!(gone.0, Some(0), "setup: the role withdraws: {}", gone.2);
    let sp = pane_in(&world, &wrk, "spawned.0", "sp");
    add_meta(&wrk.dir, "seat.spawned.0=sp\nagent_bin.spawned.0=gemini\n");
    add_meta(&wrk.dir, "launch_id.spawned.0=tok-sp\n");
    let refused = "again div legacy emptya emptyt torna tornt";
    let cells = refused.split(' ').map(|c| (c, wp, Some(ROLE)));
    let others = [("cancel", wp), ("slot", &*sp), ("foreign", &*ord.pane)];
    for (label, pane, phrase) in cells.chain(others.map(|(c, p)| (c, p, None))) {
        let req = id[if label == "again" { "first" } else { label }].as_str();
        let out = reply(pane, &[req, &format!("r-{label}")]);
        wrong.extend(match phrase {
            Some(phrase) => refusal(label, &out, phrase),
            None => (out.0 == Some(0)).then(|| format!("{label}: landed")),
        });
    }
    let disp = reply(wp, &["--as", "ody", &id["disp"], "r-disp"]);
    wrong.extend((disp.0 == Some(0)).then(|| "disp: landed".to_owned()));
    let rv = reply(wp, &[&id["review"], "r-review"]);
    wrong.extend((rv.0 != Some(0)).then(|| format!("review: {}", rv.2.trim())));
    let held = hold_lock(&world, &orc.pane);
    let race = id["race"].as_str();
    let outs: [Out; 2] = std::thread::scope(|scope| {
        let runs = ["r-race-a", "r-race-b"].map(|b| scope.spawn(move || reply(wp, &[race, b])));
        std::thread::sleep(Duration::from_secs(1));
        drop(held);
        runs.map(|run| run.join().expect("run"))
    });
    let won = outs.iter().filter(|out| out.0 == Some(0)).count();
    wrong.extend((won != 1).then(|| format!("race: {won} replies succeeded, not 1")));
    let closed = |l: &&str| l.contains("\"action\":\"reply\"") && l.contains(race);
    let closing = ledger(&wrk.dir).lines().filter(closed).count();
    wrong.extend((closing != 1).then(|| format!("race: {closing} reply records, not 1")));
    wrong.extend(refusal("race third", &reply(wp, &[race, "r-race-c"]), ROLE));
    let fresh = pane_in(&world, &wrk, "main", "wkr");
    tmux(&world, "kill-pane -t", &[wp]);
    let back = reply(&fresh, &[&id["relaunch"], "r-relaunch"]);
    wrong.extend((back.0 != Some(0)).then(|| format!("relaunch: {}", back.2.trim())));
    swap_meta(&wrk.dir, "=tok-wrk\n", "=tok-new\n");
    let change = reply(&fresh, &[&id["change"], "r-change"]);
    wrong.extend(refusal("change", &change, BINDING));
    let take = world.run(Some(&fresh), &[ACCEPT, &wd, &id["accept"], "mine"], &[]);
    wrong.extend((take.0 == Some(0)).then(|| "accept by a changed seat: landed".to_owned()));
    swap_meta(&wrk.dir, "=tok-new\n", "=tok-wrk\n");
    swap_meta(&orc.dir, "=tok-orc\n", "=tok-orc2\n");
    let asker = reply(&fresh, &[&id["asker"], "r-asker"]);
    wrong.extend(refusal("asker", &asker, BINDING));
    let seen = received(&orc);
    let sent = names.replace("race", "race-") + " again legacy asker review";
    for body in sent.split(' ') {
        let n = count(&seen, &format!("r-{body}"));
        let want = usize::from(["first", "race-", "relaunch", "review"].contains(&body));
        wrong.extend((n != want).then(|| format!("r-{body} reached the role {n}x, not {want}x")));
    }
    assert!(wrong.is_empty(), "{} wrong: {wrong:#?}", wrong.len());
}

// G8/G9 positives (C6, L3, waiver 68ada956): the role's monitor pane for each daemon action
// (plan r4:43; the live daemon is a named residual), a seat inside the role (ask, review,
// interrupt), an ordinary session NAMED `orchestrator`, and the owner carry: the real
// `SharedChat` submit -> `submit_external` -> `Carry::run` -> `tracked::run`, the carry
// `ae app` and `ae chat` share, from a caller outside the role session; nothing injected.
#[test]
fn same_session_ordinary_and_owner_routes_still_land() {
    let (world, orc, wrk, ord) = fleet("keep");
    let (od, wd) = (s(&orc.dir), s(&wrk.dir));
    let monitor = pane_in(&world, &orc, "", "");
    let m = monitor.as_str();
    tmux(&world, "set-option -p -t", &[m, "@ae_agent", "_watchdog"]);
    let kid = pane_in(&world, &orc, "spawned.0", "kid");
    let rows = "seat.spawned.0=kid\nagent_bin.spawned.0=gemini\nlaunch_id.spawned.0=t\n";
    add_meta(&orc.dir, rows);
    let mut wrong = Vec::new();
    let mut run = |label: &str, pane: &str, argv: &[&str], envs: Envs| {
        let out = world.run(Some(pane), argv, envs);
        wrong.extend((out.0 != Some(0)).then(|| format!("{label}: {}", out.2.trim())));
    };
    let actions = "sweep-nudge nudge done-challenge wait-challenge quota-advisory quota-checkpoint";
    for action in actions.split(' ') {
        let body = format!("k-{action}");
        run(
            action,
            m,
            &[SEND, &od, "boss", &body],
            &[(SENDER, "watchdog"), (ACTION, action)],
        );
    }
    for (verb, body) in [(ASK, "k-ask"), (REVIEW, "k-review"), (INTERRUPT, "k-int")] {
        run(verb, &kid, &[verb, &od, "boss", body], &[]);
    }
    let named = [SEND, &wd, XS, "orchestrator:ody", "k-named"];
    run("named", &wrk.pane, &named, &[]);
    let chat = ae::telegram::bridge::SharedChat {
        root: world.root.clone(),
        echoes: ae::telegram::Echoes::default(),
    };
    let target = ae::telegram::routing::Target {
        session: "orc".to_owned(),
        dir: orc.dir.clone(),
        session_id: uuid_of("orc"),
        pair: vec!["boss".to_owned()],
        seat: Some("boss".to_owned()),
    };
    let (from_id, date) = ("42".to_owned(), 1_792_141_200);
    let origin = ae::telegram::inbound::Origin {
        update_id: 7,
        date,
        from_id,
    };
    let sent = ae::telegram::inbound::Submit::submit(&chat, &target, "k-owner", &origin);
    if !matches!(sent, ae::telegram::inbound::Submitted::Sent { .. }) {
        wrong.push(format!("owner: {sent:?}"));
    }
    let (role, named) = (received(&orc), received(&ord));
    let mut bodies: Vec<String> = actions.split(' ').map(|a| format!("k-{a}")).collect();
    bodies.extend("k-ask k-review k-int k-owner".split(' ').map(str::to_owned));
    for body in &bodies {
        wrong.extend((!role.contains(body.as_str())).then(|| format!("{body} never arrived")));
    }
    wrong.extend((!named.contains("k-named")).then(|| "k-named never arrived".to_owned()));
    assert!(wrong.is_empty(), "{wrong:?}");
}
