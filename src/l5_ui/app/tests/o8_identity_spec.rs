//! O8 identity pins: an ordinary listed namesake is an ordinary home and
//! keeps its normal view across a restart. The fleet has no proved holder.

use super::*;
use crate::app::handover;

const NAME: &str = crate::orchestrator::ORCHESTRATOR_SESSION;

fn ordinary(home: bool) -> App {
    let mut app = app(None);
    two(&mut app);
    app.fleet.rows.push(row(NAME, 3, home));
    app.fleet.home = home.then(|| NAME.to_owned());
    app.ids.insert(NAME.to_owned(), ID.to_owned());
    app.world.sessions.push(entry(NAME, Status::Running, None));
    app.model = Model::new(&app.fleet);
    app
}

#[test]
fn an_ordinary_orchestrator_namesake_can_be_the_home_selection() {
    let app = ordinary(true);
    assert_eq!(
        app.model.selected(),
        Some(NAME),
        "a listed orchestrator namesake remains the ordinary home"
    );
}

#[test]
fn an_ordinary_orchestrator_namesake_keeps_its_ordinary_handover() {
    let mut app = ordinary(false);
    assert_eq!(app.model.selected(), Some("web"), "the initial control");
    let at = Timestamp::now().epoch();
    let text = format!(
        "v1|%1|{}|{at}|{NAME}|{ID}|agents|lead|31|18|2|9|1|4",
        crate::VERSION
    );
    let hand = handover::parse(&text).expect("the existing canonical handover");
    app.answer(Answer::Handover {
        view: hand.view.clone(),
        at,
    });
    app.settle_keep(true);
    assert_eq!(
        app.model.selected(),
        Some(NAME),
        "the listed namesake receives its ordinary handover"
    );
    assert_eq!(
        app.model.view(ID),
        Some(hand.view),
        "the listed namesake keeps every ordinary handover preference"
    );
    assert!(
        !app.panel_focused(),
        "an ordinary handover opens no seat panel"
    );
}
