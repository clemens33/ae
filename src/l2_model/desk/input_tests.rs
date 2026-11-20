//! The input spelling of a board is what `desk show` prints and `desk set` reads.

use super::{Delta, Id, Item, Section, change_ids, parse_input, render_input, summary};

fn id(section: Section, number: u8) -> Id {
    Id { section, number }
}

fn item(section: Section, number: u8, text: &str, links: &[&str]) -> Item {
    Item {
        id: id(section, number),
        text: text.to_owned(),
        links: links.iter().map(|link| (*link).to_owned()).collect(),
    }
}

#[test]
fn a_board_shows_one_heading_per_used_section_and_parses_back_to_itself() {
    let items = vec![
        item(
            Section::Decide,
            1,
            "Pick the plan",
            &["https://example.test/a"],
        ),
        item(Section::Decide, 7, "Pick the date", &[]),
        item(
            Section::Waiting,
            2,
            "Review is out",
            &[
                "https://example.test/b",
                "/abs/dir/note.md",
                "http://c.test",
            ],
        ),
        item(Section::Links, 3, "Runbook", &["/abs/dir/runbook.md"]),
    ];
    let shown = render_input(&items);
    assert_eq!(
        shown.lines().collect::<Vec<_>>(),
        [
            "DECIDE",
            "D1 | Pick the plan | https://example.test/a",
            "D7 | Pick the date",
            "WAITING ON OTHERS",
            "W2 | Review is out | https://example.test/b | /abs/dir/note.md | http://c.test",
            "LINKS",
            "L3 | Runbook | /abs/dir/runbook.md",
        ]
    );
    assert!(shown.ends_with('\n'));
    assert_eq!(
        parse_input(shown.as_bytes()).expect("it parses back"),
        items
    );
}

#[test]
fn no_items_show_one_bare_heading_that_parses_back_to_an_empty_board() {
    let shown = render_input(&[]);
    assert_eq!(shown, "DECIDE\n");
    assert_eq!(parse_input(shown.as_bytes()), Ok(vec![]));
}

#[test]
fn the_change_ids_name_added_removed_and_changed_in_that_order() {
    let delta = Delta {
        added: vec![id(Section::Decide, 3)],
        removed: vec![id(Section::Decide, 1)],
        changed: vec![id(Section::Running, 2)],
    };
    assert_eq!(change_ids(&delta), "+D3 -D1 ~R2");
    assert_eq!(change_ids(&Delta::default()), "");
    assert_eq!(summary(&delta, 12), "+D3 -D1 ~R2 (12 items)");
    assert_eq!(summary(&delta, 1), "+D3 -D1 ~R2 (1 item)");
    assert_eq!(summary(&Delta::default(), 4), "(4 items)");
}
