//! Driver units for the session menu: the Question builder, the key map,
//! drawn-text neutralisation, and the unseen and expired answers.

use super::*;
use crate::app::menu::{Card, Choice, Item, Menu, Modal, Question, Step};

fn card(name: &str, status: Status) -> Card {
    Card::new(name, ID, Some(ID), Some(status), None)
}

fn expectation(deadline: i64) -> crate::lifecycle::StopExpectation {
    crate::lifecycle::StopExpectation {
        session_id: "$7".to_owned(),
        uuid: ID.to_owned(),
        server_pid: "401".to_owned(),
        server_start: "1759500000".to_owned(),
        deadline,
        client: "/dev/ttysunit".to_owned(),
        client_pid: "402".to_owned(),
        server: crate::inventory::ServerId::from_typed_flags("socket", "/unit/ae.sock")
            .expect("server"),
        require_meta_agent: false,
    }
}

#[test]
fn every_modal_key_steps_as_the_keys_row_says() {
    let typed = |bytes: &[u8]| Key::Text(bytes.to_vec());
    let run = |modal: &mut Modal, script: Vec<(Key, Step)>| {
        for (key, want) in script {
            assert_eq!(modal.key(&key), want, "{key:?}");
        }
    };
    let (start, stop) = (Step::Act(Item::Start), Step::Act(Item::Stop));
    let mut menu = Modal::Menu(Menu::new(card("api", Status::Running)));
    run(
        &mut menu,
        vec![
            (typed(b"x"), Step::Kept),
            (typed(b"o"), Step::Kept),
            (typed(b"j"), Step::Kept),
            (Key::Enter, stop),
            (typed(b"k"), Step::Kept),
            (Key::Down, Step::Kept),
            (Key::Tab, Step::Kept),
            (typed(b"xs"), stop),
            (typed(b"s"), stop),
            (typed(b"m"), Step::Close),
            (typed(b"q"), Step::Close),
            (Key::Escape, Step::Close),
            (Key::Interrupt, Step::Close),
            (Key::Pasted(b"s".to_vec()), Step::Kept),
        ],
    );
    let mut stopped = Modal::Menu(Menu::new(card("api", Status::Stopped)));
    run(
        &mut stopped,
        vec![(typed(b"s"), start), (Key::Enter, start)],
    );
    let orchestrator = card(crate::orchestrator::ORCHESTRATOR_SESSION, Status::Running);
    assert!(orchestrator.items().is_empty());
    let mut lone = Modal::Menu(Menu::new(orchestrator));
    run(
        &mut lone,
        vec![
            (typed(b"s"), Step::Kept),
            (typed(b"j"), Step::Kept),
            (Key::Enter, Step::Kept),
        ],
    );
    let question = Question::new(card("api", Status::Running), expectation(0));
    let mut question = Modal::Question(question);
    run(
        &mut question,
        vec![
            (Key::Enter, Step::Close),
            (typed(b"c"), Step::Kept),
            (typed(b"s"), Step::Moved),
            (typed(b"s"), Step::Kept),
            (Key::Enter, Step::Affirm),
            (typed(b"c"), Step::Moved),
            (Key::Enter, Step::Close),
            (typed(b"s"), Step::Moved),
            (Key::Left, Step::Moved),
            (Key::Enter, Step::Close),
            (Key::Right, Step::Moved),
            (Key::Enter, Step::Affirm),
            (Key::Tab, Step::Moved),
            (Key::Enter, Step::Close),
            (typed(b"y"), Step::Affirm),
            (typed(b"n"), Step::Close),
            (typed(b"q"), Step::Close),
            (typed(b"cy"), Step::Affirm),
            (typed(b"yn"), Step::Affirm),
        ],
    );
}

#[test]
fn drawn_menu_and_question_text_is_neutralised_first() {
    let name = "api \u{1b}[2J";
    for modal in [
        Modal::Menu(Menu::new(Card::new(
            name,
            ID,
            None,
            Some(Status::Running),
            None,
        ))),
        Modal::Question(Question::new(card(name, Status::Running), expectation(0))),
    ] {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        let mut layout = crate::app::draw::Layout::default();
        layout.overlay(
            &buf,
            Rect::new(0, 2, 30, 3),
            crate::app::draw::Hit::Session(name.to_owned()),
        );
        let paint = crate::app::draw::Paint::of(None);
        crate::app::menu::paint(&modal, paint, true, &mut buf, &mut layout)
            .expect("the modal fits");
        let screen = text(&buf);
        assert!(screen.contains("api \u{FFFD}[2J"), "{screen}");
        if matches!(modal, Modal::Menu(_)) {
            assert!(screen.contains("record names no single"), "{screen}");
        }
    }
}

#[test]
fn a_question_takes_only_fresh_input_and_refuses_once_expired() {
    let root = Root::new("menu-units-question");
    let (mut app, _reader) = housed(&root);
    framed(&mut app);
    let typed = |bytes: &[u8]| vec![(Key::Text(bytes.to_vec()), Instant::now())];
    assert!(take_keys(&mut app, typed(b"m")).is_some());
    // Input made before the menu was on screen: a flash, never a notice.
    assert!(take_keys(&mut app, typed(b"j")).is_some());
    let flash = app.flash.as_ref().map(|(line, _)| line.as_str());
    assert!(
        flash.is_some_and(|line| line.contains("press again")),
        "{flash:?}"
    );
    assert!(app.notices.is_empty());
    let Some(Modal::Menu(menu)) = app.modal.take() else {
        unreachable!("an unseen j leaves the menu open")
    };
    let expired = expectation(Timestamp::now().epoch() - 1);
    let question = Question::new(menu.card, expired.clone());
    assert_eq!((question.focus, question.shown_at), (Choice::Cancel, None));
    assert_eq!(question.expect, expired);
    app.modal = Some(Modal::Question(question));
    framed(&mut app);
    let shown = Instant::now();
    app.frame_shown(shown);
    for (key, ms) in [(Key::Text(b"s".to_vec()), 1), (Key::Enter, 2)] {
        assert!(take_keys(&mut app, vec![(key, shown + Duration::from_millis(ms))]).is_some());
    }
    assert!(app.modal.is_none(), "an answered question closes");
    let (target, item) = app.notices.last().expect("the refusal");
    assert_eq!(target, "api");
    assert!(item.body.contains("the question expired"), "{}", item.body);
}
