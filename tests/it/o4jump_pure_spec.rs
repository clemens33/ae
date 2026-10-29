//! Frozen oracle: O4 lead-ruling-r1; records are inert data, app memory is local.
use ae::app::{
    fleet::{Counts, Fleet, Line2, Row},
    model::{self, Act, Key, Model},
};
use ae::{console::input, jump::Record, theme::Mark};
const A: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const O: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const W: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const LITERAL: &str = "1|/dev/ttys003|123|$4|@6|%7|0123456789abcdef";

#[test]
fn literal_record_fields_are_preserved_and_hostile_fields_are_rejected() {
    let r = Record::parse(LITERAL).expect("literal record");
    assert_eq!(
        (&*r.client_name, &*r.client_pid, &*r.session_id),
        ("/dev/ttys003", "123", "$4")
    );
    assert_eq!(
        (&*r.window_id, &*r.pane_id, &*r.nonce),
        ("@6", "%7", "0123456789abcdef")
    );
    assert_eq!(r.render(), LITERAL);
    for (index, bad) in [
        (0, "2"),
        (1, ""),
        (1, "a;b"),
        (1, "#{client_name}"),
        (1, "a\nb"),
        (1, "é"),
        (2, "0"),
        (2, "0123"),
        (2, "12345678901"),
        (2, "-1"),
        (3, "4"),
        (3, "$x"),
        (4, "%6"),
        (4, "@"),
        (5, "@7"),
        (5, "%7;kill"),
        (6, "ABCDEF0123456789"),
        (6, "0123456789abcde"),
        (6, "0123456789abcdef0"),
    ] {
        let mut fields: Vec<_> = LITERAL.split('|').collect();
        fields[index] = bad;
        assert!(
            Record::parse(&fields.join("|")).is_none(),
            "reject {index}: {bad:?}"
        );
    }
    for bad in [
        format!("{LITERAL}|extra"),
        format!("{LITERAL}\n"),
        "1|a|1|$0|@0|%0".to_owned(),
        LITERAL.replace("/dev/ttys003", &"a".repeat(256)),
    ] {
        assert!(
            Record::parse(&bad).is_none(),
            "reject hostile record {bad:?}"
        );
    }
}

fn fleet() -> Fleet {
    Fleet {
        rows: ["orchestrator", "api", "web"]
            .iter()
            .enumerate()
            .map(|(n, name)| Row {
                name: (*name).into(),
                index: n + 1,
                mark: Mark::Idle,
                needy: false,
                unanswered: None,
                counts: Counts::Unknown,
                line2: Line2::NoGoal,
                home: *name == "api",
            })
            .collect(),
        home: Some("api".into()),
    }
}
fn live() -> Vec<(String, String)> {
    [("orchestrator", O), ("api", A), ("web", W)]
        .map(|(n, id)| (n.into(), id.into()))
        .to_vec()
}
fn notice(a: Act) {
    assert!(
        matches!(a, Act::Notice(s) if !s.is_empty()),
        "visible refusal required"
    );
}

#[test]
fn browse_o_has_a_distinct_key_and_app_instances_keep_independent_return_selections() {
    assert_eq!(
        model::browse_keys(&input::Key::Text(b"O".to_vec())),
        vec![Key::Orchestrator]
    );
    assert_eq!(
        model::browse_keys(&input::Key::Text(b"o".to_vec())),
        vec![Key::Open]
    );
    let f = fleet();
    let (mut a, mut b) = (Model::new(&f), Model::new(&f));
    let _ = b.key(Key::Digit(3), &f, false);
    assert_eq!(
        a.toggle_orchestrator(&f, &live()),
        Act::Select("orchestrator".into())
    );
    assert_eq!(b.selected(), Some("web"));
    assert_eq!(
        b.toggle_orchestrator(&f, &live()),
        Act::Select("orchestrator".into())
    );
    assert_eq!(
        a.toggle_orchestrator(&f, &live()),
        Act::Select("api".into())
    );
    assert_eq!(b.selected(), Some("orchestrator"));
    assert_eq!(
        b.toggle_orchestrator(&f, &live()),
        Act::Select("web".into())
    );
}

#[test]
fn missing_unpublished_or_stopped_orchestrator_preserves_selection_and_return_memory() {
    let f = fleet();
    let mut m = Model::new(&f);
    let unavailable = vec![("api".into(), A.into())];
    let before = m.clone();
    notice(m.toggle_orchestrator(&f, &unavailable));
    assert_eq!(
        m, before,
        "retained row alone is not an available orchestrator"
    );
    let _ = m.toggle_orchestrator(&f, &live());
    let before = m.clone();
    notice(m.toggle_orchestrator(&f, &unavailable));
    assert_eq!(m, before);
    assert_eq!(
        m.toggle_orchestrator(&f, &live()),
        Act::Select("api".into())
    );
}

#[test]
fn a_same_name_successor_cannot_take_the_prior_selection_and_refusal_keeps_memory() {
    let f = fleet();
    let mut m = Model::new(&f);
    let _ = m.toggle_orchestrator(&f, &live());
    let before = m.clone();
    for id in [W, "", "not-a-uuid"] {
        let mut changed = live();
        changed[1].1 = id.into();
        notice(m.toggle_orchestrator(&f, &changed));
        assert_eq!(m, before);
    }
    let without_api = vec![("orchestrator".into(), O.into())];
    notice(m.toggle_orchestrator(&f, &without_api));
    assert_eq!(m, before);
    assert_eq!(
        m.toggle_orchestrator(&f, &live()),
        Act::Select("api".into())
    );
}
