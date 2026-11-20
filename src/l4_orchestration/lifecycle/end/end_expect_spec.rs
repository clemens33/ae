//! S6b: the consequence shown by the question travels to the detached owner.
//! This observes real argv generation and parsing, not a duplicated encoder.

use super::{Action, ConfirmedPlan, EndExpectation, FrozenTmux, Plan, parse};

#[test]
fn the_question_push_consequence_survives_argv_and_parse_in_both_directions() {
    for pushes in [false, true] {
        let expect = EndExpectation {
            uuid: "0199c0de-aaaa-4890-abcd-ef0123456789".to_owned(),
            server: super::server_from_words("socket", "/owned/ae.sock").unwrap(),
            identity: None,
            deadline: 1_800_000_000,
            confirmed: ConfirmedPlan {
                name: "alpha".to_owned(),
                plan: Plan {
                    action: Action::Keep,
                    detail: "/owned/archive/id".to_owned(),
                    purge: false,
                },
                from_default: true,
                tmux: FrozenTmux::Absent,
            },
            assume_stopped: false,
            pushes,
        };
        assert_eq!(
            expect.lines().iter().any(|line| line.contains("pushes")),
            pushes,
            "the observed wire consequence must be the one shown"
        );
        let argv = expect.argv("/owned/ae".to_owned());
        let words = argv.as_args();
        let carried = words
            .iter()
            .filter(|word| word.starts_with("--expect-pushes="))
            .collect::<Vec<_>>();
        assert_eq!(carried.len(), 1, "the consequence must travel exactly once");
        assert_eq!(
            carried[0].as_str(),
            if pushes {
                "--expect-pushes=on"
            } else {
                "--expect-pushes=off"
            }
        );
        let parsed = parse(&words[2..]).expect("the owner's argv parses");
        assert_eq!(parsed.expect.expect("guarded owner").pushes, pushes);
    }
}
