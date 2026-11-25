//! T1 acceptance at the snapshot boundary.

use std::sync::Arc;

use ae::json::{self, Value};
use ae::topic::{Card, Group};
use ae::web::{
    chat::{Row, TAGS_WINDOW},
    state::{Read, render},
};

fn array(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Arr(values)) => values,
        other => panic!("expected an array, got {other:?}"),
    }
}

#[test]
fn snapshot_carries_only_distinct_known_tags_in_text_order() {
    assert_eq!(TAGS_WINDOW, 512, "the row tag scan budget in bytes");
    let mut read = Read::default();
    read.desk.groups = vec![
        (Group::NeedsYou, vec![Card::bare("smoke")]),
        (Group::Quiet, vec![Card::bare("calm")]),
        (Group::Closed, vec![Card::bare("done")]),
        (Group::Unreadable, vec![Card::bare("broken")]),
        (Group::Moving, vec![Card::bare("reserved")]),
    ];
    let last = format!("{} #calm", "x".repeat(506));
    assert_eq!(last.len(), 512, "#calm ends at the byte boundary");
    let cases = [
        (
            "#calm #smoke. #smoke #unknown #done #broken #reserved".to_owned(),
            &["calm", "smoke", "done", "broken"][..],
        ),
        (
            "a#smoke (#calm) #done_more #smoke-test #unknown".to_owned(),
            &[][..],
        ),
        ("ordinary words".to_owned(), &[][..]),
        (format!("{last} #smoke"), &["calm"][..]),
        (format!("{last}. #smoke"), &["calm"][..]),
        (format!("{last}-later #smoke"), &[][..]),
        (format!("{last}_later #smoke"), &[][..]),
        (format!("{} #smoke", "x".repeat(511)), &[][..]),
        (format!("#smoke {}é #calm", "x".repeat(504)), &["smoke"][..]),
    ];
    for (body, _) in &cases {
        read.chat.rows.push(Row {
            body: body.clone(),
            ..Row::default()
        });
    }
    let snapshot = render(Arc::new(read), "web-t1", 0, &[7; 32]);
    let document = std::str::from_utf8(&snapshot.body).expect("UTF-8 snapshot");
    let state = json::parse(document).expect("state JSON");
    let rows = array(state.get("chat").and_then(|chat| chat.get("rows")));
    assert_eq!(rows.len(), cases.len());
    for (row, (body, expected)) in rows.iter().zip(cases) {
        assert_eq!(row.get_str("body"), Some(body.as_str()));
        let actual: Vec<_> = array(row.get("tags"))
            .iter()
            .map(|tag| tag.as_str().expect("tag string"))
            .collect();
        assert_eq!(actual, expected, "row {body:?}");
    }
}
