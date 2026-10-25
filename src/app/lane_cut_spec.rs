use super::{Paint, body_rows, closing, link, parts};
use crate::console::lane::{Item, Kind};

fn assert_cut(token: &str, rest: &str) {
    let body = format!("{} {token}{rest}", "x".repeat(79 - token.len()));
    let item = |kind, body| Item {
        micros: 1,
        kind,
        body,
        record: None,
    };
    let items = [
        item(
            Kind::Asked {
                to: "lead".to_owned(),
                id: "q".to_owned(),
                uncertain: false,
                gap: None,
                via: None,
            },
            body,
        ),
        item(Kind::Closed { id: "q".to_owned() }, String::new()),
    ];
    let closed = closing(&items, 1).expect("closed ask");
    assert!(closed.cut, "fixture clips inside the original path token");
    let displayed = parts(&items[1], Some(&closed));
    assert!(displayed[0].1.ends_with(&format!("{token}…")));
    let base = link::Base {
        dir: Some("/w".to_owned()),
        home: None,
    };
    assert!(
        super::file_refs(&displayed[0].1, &base, true).is_empty(),
        "a clipped token must never open the scanner's shorter file"
    );
    let rows = body_rows(
        &items[1],
        Some(&closed),
        (160, &base),
        Paint::of(None),
        "- ",
    );
    assert!(
        rows.iter().all(|(_, _, files)| files.is_empty()),
        "cut cells keep Turn behavior"
    );
    assert!(
        rows.iter().any(|(_, links, _)| !links.is_empty()),
        "legacy OSC8 residual remains"
    );
}

#[test]
fn a_dot_trimmed_from_a_clipped_path_never_opens_another_file() {
    assert_cut("docs/a.md.", "bak");
}

#[test]
fn a_colon_trimmed_from_a_clipped_path_never_loses_its_line() {
    assert_cut("docs/b.md:", "12");
}
