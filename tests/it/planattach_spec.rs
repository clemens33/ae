//! planattach characterisation: frozen pins for `parse_plan_with_attach` (#300).
//!
//! Oracle: base b6218268's observable result for every argv here, under BOTH
//! values of `allow_attach` (public launch grammar, then the orchestrator
//! seat grammar). A refactor of the parse must keep every row. Results render
//! compactly: `ok` plus each non-default `Plan` field (the destructure below is
//! exhaustive, so a new field fails to compile), `err "<line>"` for a refusal,
//! and `unknown "<word>"` for the one context-free unknown-flag line, pinned in
//! full by `the_unknown_flag_line_is_context_free`.
//!
//! Some rows pin bug-shaped behaviour on purpose; changing one is a ruling, not
//! a refactor: `--solo --seat colead=x` parses (only `--colead` sugar trips the
//! parse-time solo refusal), `use` and the mode flags repeat with the last one
//! winning, `use ""` parses, split values swallow flag-shaped words, and the
//! seat grammar's unknown-flag line advertises flags it refuses.

use super::{Plan, parse_plan, parse_plan_with_attach};

/// One argv, its public result (`allow_attach` false), its seat result (true).
type Case = (&'static [&'static str], &'static str, &'static str);

const UNKNOWN_HEAD: &str = "Error: unknown flag '";
const UNKNOWN_TAIL: &str = "'. Use --worktree, --copy, --local, --dir <path>, --no-attach, --tmux, --solo, --from <archive-uuid>, --seat <agent>=<profile>, --lead <profile>, or --colead <profile>.";

fn render(result: Result<Plan, String>) -> String {
    let plan = match result {
        Ok(plan) => plan,
        Err(err) => {
            let word = err
                .strip_prefix(UNKNOWN_HEAD)
                .and_then(|tail| tail.strip_suffix(UNKNOWN_TAIL));
            return match word {
                Some(word) => format!("unknown {word:?}"),
                None => format!("err {err:?}"),
            };
        }
    };
    let Plan {
        mode,
        name,
        main,
        from,
        workers,
        seat_profiles,
        dir,
        attach,
        tmux,
    } = plan;
    let mut out = vec!["ok".to_owned()];
    out.extend(mode.map(|value| format!("mode={value:?}")));
    out.extend(name.map(|value| format!("name={value:?}")));
    out.extend(main.map(|value| format!("main={value:?}")));
    out.extend(from.map(|value| format!("from={value:?}")));
    out.extend(workers.map(|value| format!("workers={value:?}")));
    if !seat_profiles.is_empty() {
        out.push(format!("seats={seat_profiles:?}"));
    }
    out.extend(dir.map(|value| format!("dir={value:?}")));
    out.extend(attach.map(|value| format!("attach={value}")));
    if tmux {
        out.push("tmux".to_owned());
    }
    out.join(" ")
}

fn words(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|word| (*word).to_owned()).collect()
}

fn check(table: &[Case]) {
    let mut drift = Vec::new();
    for &(argv, public, seat) in table {
        for (allow_attach, want) in [(false, public), (true, seat)] {
            let got = render(parse_plan_with_attach(&words(argv), allow_attach));
            if got != want {
                drift.push(format!(
                    "{argv:?} allow_attach={allow_attach}\n  want {want}\n  got  {got}"
                ));
            }
        }
    }
    assert!(drift.is_empty(), "{}", drift.join("\n"));
}

/// Mode, attach, tmux and solo toggles, their contexts and repeats, and the
/// words that are session-name candidates.
const FLAGS: &[Case] = &[
    (&[], "ok", "ok"),
    (&["demo"], "ok name=\"demo\"", "ok name=\"demo\""),
    (&["--worktree"], "ok mode=Git", "ok mode=Git"),
    (&["--copy"], "ok mode=Full", "ok mode=Full"),
    (&["--local"], "ok mode=Local", "ok mode=Local"),
    (
        &["--worktree", "--local", "--copy"],
        "ok mode=Full",
        "ok mode=Full",
    ),
    (
        &["--copy", "demo", "--worktree"],
        "ok mode=Git name=\"demo\"",
        "ok mode=Git name=\"demo\"",
    ),
    (&["--attach"], "unknown \"--attach\"", "ok attach=true"),
    (&["--no-attach"], "ok attach=false", "ok attach=false"),
    (
        &["--attach", "--no-attach"],
        "unknown \"--attach\"",
        "ok attach=false",
    ),
    (
        &["--no-attach", "--attach"],
        "unknown \"--attach\"",
        "ok attach=true",
    ),
    (
        &["demo", "--tmux", "--tmux"],
        "ok name=\"demo\" tmux",
        "unknown \"--tmux\"",
    ),
    (&["--tmux"], "ok tmux", "unknown \"--tmux\""),
    (&["--solo"], "ok workers=\"\"", "unknown \"--solo\""),
    (
        &["--solo", "--solo", "demo"],
        "ok name=\"demo\" workers=\"\"",
        "unknown \"--solo\"",
    ),
    (&["-x"], "ok name=\"-x\"", "ok name=\"-x\""),
    (&["-"], "ok name=\"-\"", "ok name=\"-\""),
    (&["-h"], "ok name=\"-h\"", "ok name=\"-h\""),
    (&[""], "ok name=\"\"", "ok name=\"\""),
    (&["démo"], "ok name=\"démo\"", "ok name=\"démo\""),
    (&["@"], "ok name=\"@\"", "ok name=\"@\""),
    (
        &["demo", "--no-attach", "--tmux", "--solo", "--local"],
        "ok mode=Local name=\"demo\" workers=\"\" attach=false tmux",
        "unknown \"--tmux\"",
    ),
];

/// `--dir`, `--from` and `use`: split and inline values, missing and empty
/// values, flag-shaped values, duplicates and their precedence.
const VALUES: &[Case] = &[
    (&["--dir", "/repo"], "ok dir=\"/repo\"", "ok dir=\"/repo\""),
    (&["--dir=/repo"], "ok dir=\"/repo\"", "ok dir=\"/repo\""),
    (&["--dir=/a=b"], "ok dir=\"/a=b\"", "ok dir=\"/a=b\""),
    (&["--dir==x"], "ok dir=\"=x\"", "ok dir=\"=x\""),
    (
        &["--dir"],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (
        &["--dir="],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (
        &["--dir", ""],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (
        &["demo", "--dir"],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (
        &["--dir", "--tmux"],
        "ok dir=\"--tmux\"",
        "ok dir=\"--tmux\"",
    ),
    (
        &["--dir", "use", "demo"],
        "ok name=\"demo\" dir=\"use\"",
        "ok name=\"demo\" dir=\"use\"",
    ),
    (&["--dir", "-x"], "ok dir=\"-x\"", "ok dir=\"-x\""),
    (
        &["--dir=/one", "--dir", "/two"],
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
    ),
    (
        &["--dir", "/one", "--dir=/two"],
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
    ),
    (
        &["--dir", "/one", "--dir"],
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
    ),
    (
        &["--dir", "/one", "--dir="],
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
        "err \"Error: --dir may be given only once — a launch has one origin.\"",
    ),
    (
        &["--dir=", "--dir=/two"],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (&["--from", "u1"], "ok from=\"u1\"", "ok from=\"u1\""),
    (&["--from=u1"], "ok from=\"u1\"", "ok from=\"u1\""),
    (&["--from=u=1"], "ok from=\"u=1\"", "ok from=\"u=1\""),
    (
        &["--from"],
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
    ),
    (
        &["--from="],
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
    ),
    (
        &["--from", ""],
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
    ),
    (
        &["--from", "--solo"],
        "ok from=\"--solo\"",
        "ok from=\"--solo\"",
    ),
    (
        &["--from", "u1", "--from", "u2"],
        "err \"Error: --from may be given only once — a session inherits from exactly one archive.\"",
        "err \"Error: --from may be given only once — a session inherits from exactly one archive.\"",
    ),
    (
        &["--from=u1", "--from"],
        "err \"Error: --from may be given only once — a session inherits from exactly one archive.\"",
        "err \"Error: --from may be given only once — a session inherits from exactly one archive.\"",
    ),
    (
        &["--from=", "--from=u2"],
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
        "err \"Error: --from requires an archive UUID (list them: ls ~/.ae/archive).\"",
    ),
    (
        &["use", "colead"],
        "ok main=\"colead\"",
        "ok main=\"colead\"",
    ),
    (
        &["demo", "use", "colead"],
        "ok name=\"demo\" main=\"colead\"",
        "ok name=\"demo\" main=\"colead\"",
    ),
    (
        &["use", "colead", "demo"],
        "ok name=\"demo\" main=\"colead\"",
        "ok name=\"demo\" main=\"colead\"",
    ),
    (
        &["use"],
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
    ),
    (
        &["demo", "use"],
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
    ),
    (&["use", "--dir"], "ok main=\"--dir\"", "ok main=\"--dir\""),
    (&["use", ""], "ok main=\"\"", "ok main=\"\""),
    (&["use", "a", "use", "b"], "ok main=\"b\"", "ok main=\"b\""),
    (&["use", "use"], "ok main=\"use\"", "ok main=\"use\""),
    (
        &["use", "use", "use"],
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
        "err \"Error: 'use' requires an agent NAME bound in [roster] (e.g. ae myproject use colead)\"",
    ),
    (
        &["--dir", "/r", "--from", "u1", "use", "x", "demo"],
        "ok name=\"demo\" main=\"x\" from=\"u1\" dir=\"/r\"",
        "ok name=\"demo\" main=\"x\" from=\"u1\" dir=\"/r\"",
    ),
];

/// `--lead`, `--colead` and `--seat`: value forms, validation order,
/// duplicate agents across spellings and command-line order.
const SEATS: &[Case] = &[
    (
        &["--lead", "solx"],
        "ok seats=[(\"lead\", \"solx\")]",
        "unknown \"--lead\"",
    ),
    (
        &["--lead=solx"],
        "ok seats=[(\"lead\", \"solx\")]",
        "unknown \"--lead=solx\"",
    ),
    (
        &["--lead=a=b"],
        "ok seats=[(\"lead\", \"a=b\")]",
        "unknown \"--lead=a=b\"",
    ),
    (
        &["--lead"],
        "err \"Error: --lead requires a profile.\"",
        "unknown \"--lead\"",
    ),
    (
        &["--lead="],
        "err \"Error: --lead requires a profile.\"",
        "unknown \"--lead=\"",
    ),
    (
        &["--lead", ""],
        "err \"Error: --lead requires a profile.\"",
        "unknown \"--lead\"",
    ),
    (
        &["--lead", "--solo"],
        "ok seats=[(\"lead\", \"--solo\")]",
        "unknown \"--lead\"",
    ),
    (
        &["--colead", "astrax"],
        "ok seats=[(\"colead\", \"astrax\")]",
        "unknown \"--colead\"",
    ),
    (
        &["--colead=astrax"],
        "ok seats=[(\"colead\", \"astrax\")]",
        "unknown \"--colead=astrax\"",
    ),
    (
        &["--colead"],
        "err \"Error: --colead requires a profile.\"",
        "unknown \"--colead\"",
    ),
    (
        &["--colead="],
        "err \"Error: --colead requires a profile.\"",
        "unknown \"--colead=\"",
    ),
    (
        &["--colead", "--tmux"],
        "ok seats=[(\"colead\", \"--tmux\")]",
        "unknown \"--colead\"",
    ),
    (
        &["--seat", "builder=terrax"],
        "ok seats=[(\"builder\", \"terrax\")]",
        "unknown \"--seat\"",
    ),
    (
        &["--seat=builder=terrax"],
        "ok seats=[(\"builder\", \"terrax\")]",
        "unknown \"--seat=builder=terrax\"",
    ),
    (
        &["--seat", "a=b=c"],
        "ok seats=[(\"a\", \"b=c\")]",
        "unknown \"--seat\"",
    ),
    (
        &["--seat", "--x=y"],
        "ok seats=[(\"--x\", \"y\")]",
        "unknown \"--seat\"",
    ),
    (
        &["--seat"],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat="],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat=\"",
    ),
    (
        &["--seat", "lead"],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat=lead"],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat=lead\"",
    ),
    (
        &["--seat", "=profile"],
        "err \"Error: --seat requires non-empty <agent>=<profile>.\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat", "lead="],
        "err \"Error: --seat requires non-empty <agent>=<profile>.\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat=="],
        "err \"Error: --seat requires non-empty <agent>=<profile>.\"",
        "unknown \"--seat==\"",
    ),
    (
        &["--seat", ""],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat", "lead=one", "--seat=lead=two"],
        "err \"Error: profile override for agent 'lead' may be given only once.\"",
        "unknown \"--seat\"",
    ),
    (
        &["--lead", "one", "--seat", "lead=two"],
        "err \"Error: profile override for agent 'lead' may be given only once.\"",
        "unknown \"--lead\"",
    ),
    (
        &["--seat", "lead=one", "--lead", "two"],
        "err \"Error: profile override for agent 'lead' may be given only once.\"",
        "unknown \"--seat\"",
    ),
    (
        &["--seat", "colead=a", "--colead", "b"],
        "err \"Error: profile override for agent 'colead' may be given only once.\"",
        "unknown \"--seat\"",
    ),
    (
        &["--colead", "a", "--colead=b"],
        "err \"Error: profile override for agent 'colead' may be given only once.\"",
        "unknown \"--colead\"",
    ),
    (
        &["--lead", "x", "--lead", ""],
        "err \"Error: --lead requires a profile.\"",
        "unknown \"--lead\"",
    ),
    (
        &["--lead", "x", "--lead="],
        "err \"Error: --lead requires a profile.\"",
        "unknown \"--lead\"",
    ),
    (
        &["--seat", "x=1", "--seat", "x"],
        "err \"Error: --seat requires <agent>=<profile> (e.g. --seat lead=solx).\"",
        "unknown \"--seat\"",
    ),
    (
        &[
            "named",
            "--seat",
            "builder=terrax",
            "--lead",
            "solx",
            "--colead=astrax",
        ],
        "ok name=\"named\" seats=[(\"builder\", \"terrax\"), (\"lead\", \"solx\"), (\"colead\", \"astrax\")]",
        "unknown \"--seat\"",
    ),
    (
        &["--seat", "b=1", "--seat", "a=2", "--lead", "l"],
        "ok seats=[(\"b\", \"1\"), (\"a\", \"2\"), (\"lead\", \"l\")]",
        "unknown \"--seat\"",
    ),
];

/// Unknown flags, a second positional and its helper hint, the final solo
/// check, and which refusal wins when several apply.
const ORDER: &[Case] = &[
    (&["--"], "unknown \"--\"", "unknown \"--\""),
    (&["--", "demo"], "unknown \"--\"", "unknown \"--\""),
    (&["--dirx"], "unknown \"--dirx\"", "unknown \"--dirx\""),
    (&["--fromx"], "unknown \"--fromx\"", "unknown \"--fromx\""),
    (&["--leadx"], "unknown \"--leadx\"", "unknown \"--leadx\""),
    (
        &["--coleadx"],
        "unknown \"--coleadx\"",
        "unknown \"--coleadx\"",
    ),
    (&["--seatx"], "unknown \"--seatx\"", "unknown \"--seatx\""),
    (&["--Tmux"], "unknown \"--Tmux\"", "unknown \"--Tmux\""),
    (
        &["--no-attachx"],
        "unknown \"--no-attachx\"",
        "unknown \"--no-attachx\"",
    ),
    (&["--help"], "unknown \"--help\"", "unknown \"--help\""),
    (
        &["demo", "msg"],
        "err \"Error: a launch takes exactly one session name, not two ('demo' and 'msg').\"",
        "err \"Error: a launch takes exactly one session name, not two ('demo' and 'msg').\"",
    ),
    (
        &["demo", "msg", "--bogus"],
        "err \"Error: a launch takes exactly one session name, not two ('demo' and 'msg').\"",
        "err \"Error: a launch takes exactly one session name, not two ('demo' and 'msg').\"",
    ),
    (
        &["--bogus", "demo", "msg"],
        "unknown \"--bogus\"",
        "unknown \"--bogus\"",
    ),
    (
        &["aedevx", "send", "colead", "msg"],
        "err \"Error: a launch takes exactly one session name, not two ('aedevx' and 'send'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
        "err \"Error: a launch takes exactly one session name, not two ('aedevx' and 'send'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
    ),
    (
        &["send", "demo"],
        "err \"Error: a launch takes exactly one session name, not two ('send' and 'demo'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
        "err \"Error: a launch takes exactly one session name, not two ('send' and 'demo'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
    ),
    (
        &["send", "ask"],
        "err \"Error: a launch takes exactly one session name, not two ('send' and 'ask'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
        "err \"Error: a launch takes exactly one session name, not two ('send' and 'ask'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
    ),
    (
        &["@", "send", "x", "y"],
        "err \"Error: a launch takes exactly one session name, not two ('@' and 'send'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
        "err \"Error: a launch takes exactly one session name, not two ('@' and 'send'). 'send' is a session helper — run it by its full path (<state-root>/sessions/<session>/send …) or as 'ae @<session> send …'.\"",
    ),
    (
        &["demo", "--dir"],
        "err \"Error: --dir requires a path.\"",
        "err \"Error: --dir requires a path.\"",
    ),
    (
        &["--dir", "--from"],
        "ok dir=\"--from\"",
        "ok dir=\"--from\"",
    ),
    (
        &["--solo", "--colead", "astrax"],
        "err \"Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.\"",
        "unknown \"--solo\"",
    ),
    (
        &["--colead", "astrax", "--solo"],
        "err \"Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.\"",
        "unknown \"--colead\"",
    ),
    (
        &["--colead=astrax", "--solo", "demo"],
        "err \"Error: '--solo' starts the lead alone; drop --colead/--seat <worker>.\"",
        "unknown \"--colead=astrax\"",
    ),
    (
        &["--solo", "--colead", "astrax", "a", "b"],
        "err \"Error: a launch takes exactly one session name, not two ('a' and 'b').\"",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--colead="],
        "err \"Error: --colead requires a profile.\"",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--colead", "x", "--bogus"],
        "unknown \"--bogus\"",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--seat", "colead=x"],
        "ok workers=\"\" seats=[(\"colead\", \"x\")]",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--seat", "builder=x"],
        "ok workers=\"\" seats=[(\"builder\", \"x\")]",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--lead", "solx", "named"],
        "ok name=\"named\" workers=\"\" seats=[(\"lead\", \"solx\")]",
        "unknown \"--solo\"",
    ),
    (
        &["--solo", "--colead", "x", "--colead", "y"],
        "err \"Error: profile override for agent 'colead' may be given only once.\"",
        "unknown \"--solo\"",
    ),
    (
        &["--lead", "x", "--attach"],
        "unknown \"--attach\"",
        "unknown \"--lead\"",
    ),
    (
        &["--tmux", "--lead", "x"],
        "ok seats=[(\"lead\", \"x\")] tmux",
        "unknown \"--tmux\"",
    ),
    (
        &["--solo", "--attach", "--colead", "x"],
        "unknown \"--attach\"",
        "unknown \"--solo\"",
    ),
];

#[test]
fn flags() {
    check(FLAGS);
}

#[test]
fn values() {
    check(VALUES);
}

#[test]
fn seats() {
    check(SEATS);
}

#[test]
fn order() {
    check(ORDER);
}

#[test]
fn the_unknown_flag_line_is_context_free() {
    let line = "Error: unknown flag '--x=y'. Use --worktree, --copy, --local, --dir <path>, --no-attach, --tmux, --solo, --from <archive-uuid>, --seat <agent>=<profile>, --lead <profile>, or --colead <profile>.";
    for allow_attach in [false, true] {
        assert_eq!(
            parse_plan_with_attach(&words(&["--x=y"]), allow_attach),
            Err(line.to_owned()),
            "allow_attach={allow_attach}"
        );
    }
}

#[test]
fn the_public_parse_is_the_attach_off_grammar() {
    for &(argv, _, _) in &[FLAGS, VALUES, SEATS, ORDER].concat() {
        assert_eq!(
            parse_plan(&words(argv)),
            parse_plan_with_attach(&words(argv), false),
            "{argv:?}"
        );
    }
}
