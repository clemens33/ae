//! A verdict through the real authenticated send route, pane and topic fold.

use super::{Rig, TestClock, Web, confirmed, openings, role_rig, text};
use ae::json::Value;

const OLD: &str = "2026-10-01T08:00:00Z";
const NEW: &str = "2026-10-02T08:00:00Z";
const KEPT: &str = "2026-10-03T08:00:00Z";
const LINE: &str = "#tmux38 keep";

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    let Some(Value::Arr(items)) = value.get(key) else {
        panic!("missing array {key} in {}", value.render());
    };
    items
}

fn state(web: &Web) -> Value {
    let reply = web.get("/api/state");
    assert_eq!(reply.status, 200);
    reply.value
}

fn revisit(state: &Value) -> Vec<&str> {
    array(state.get("desk").expect("desk"), "revisit")
        .iter()
        .map(|entry| text(entry, "slug"))
        .collect()
}

fn card(web: &Web, slug: &str) -> Value {
    let reply = web.get(&format!("/api/topic/{slug}"));
    assert_eq!(reply.status, 200);
    reply.value
}

fn append(rig: &Rig, record: &str) {
    ae::store::open(&rig.dir)
        .append_memo(record.as_bytes())
        .expect("append fixture writer's topic memo");
}

fn seed(rig: &Rig) {
    append(
        rig,
        &format!(
            "{OLD}\ttui\ttopic-tmux38\ttitle: Tmux next | state: quiet | next: Read the notes\n\
             {NEW}\ttui\ttopic-later\ttitle: Later | state: quiet\n"
        ),
    );
}

#[test]
fn keep_carries_once_and_waits_for_the_orchestrators_memo_and_reply() {
    let rig = role_rig("wkeep");
    seed(&rig);
    let clock = TestClock::new();
    let web = Web::live(&rig, &clock, 0);
    let route = web.route(&rig);
    assert_eq!(revisit(&state(&web)), ["tmux38", "later"]);
    let original = card(&web, "tmux38");
    let line = text(original.get("verdict").expect("quiet verdict"), "keep");
    assert_eq!(line, LINE);
    let draft = web.draft(&route, line);
    let receipt = web.send(&draft);
    let id = confirmed(&receipt, &rig);
    assert_eq!(web.submit(&draft).value, receipt.value);
    assert_eq!(rig.enter_count(), 1);
    assert_eq!(openings(&rig), [id.as_str()]);
    let expected = format!("REQUEST {id} from human:chat: {LINE}\n");
    assert_eq!(rig.submitted().matches(&expected).count(), 1);

    clock.advance(1);
    let waiting = card(&web, "tmux38");
    assert_eq!(waiting.get_str("group"), Some("needs_you"));
    assert_eq!(waiting.get_str("reviewed"), Some(OLD));
    assert_eq!(waiting.get("verdict"), Some(&Value::Null));
    let waits = array(&waiting, "waits");
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].get_str("kind"), Some("ask"));
    assert_eq!(revisit(&state(&web)), ["later"]);

    append(&rig, &format!("{KEPT}\ttui\ttopic-tmux38\treview: keep\n"));
    clock.advance(1);
    let reviewed = card(&web, "tmux38");
    assert_eq!(reviewed.get_str("reviewed"), Some(KEPT));
    assert_eq!(reviewed.get_str("group"), Some("needs_you"));
    assert_eq!(reviewed.get("fields"), original.get("fields"));
    assert_eq!(revisit(&state(&web)), ["later"]);

    let events = rig.events();
    let opening = events
        .lines()
        .find(|event| {
            let value = ae::json::parse(event).expect("opening journal event");
            value.get_str("action") == Some("ask") && value.get_str("ref") == Some(id.as_str())
        })
        .expect("the real send wrote an ask opening");
    rig.answer(opening);
    clock.advance(1);
    let settled = card(&web, "tmux38");
    assert_eq!(settled.get_str("group"), Some("quiet"));
    assert_eq!(settled.get_str("reviewed"), Some(KEPT));
    assert_eq!(settled.get("fields"), original.get("fields"));
    assert!(array(&settled, "waits").is_empty());
    assert_eq!(
        array(&settled, "history")[0].get_str("text"),
        Some("review: keep")
    );
    assert_eq!(revisit(&state(&web)), ["later", "tmux38"]);
}
