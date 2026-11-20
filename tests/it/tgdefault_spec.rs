//! Frozen tgdefault acceptance: a plain Telegram message with no `/use` selection is asked in the
//! orchestrator seat's chat. Oracle: owner request 2026-11-19 (verbatim in
//! `.local/brief-tgdefault.md`), lead rulings 1-5 of that brief, the lead's 2026-11-19 rulings
//! (the seat is `settings_menu::role_holder`, never a bare `meta_agent=true`; a reply the bridge
//! cannot route refuses and never becomes a default ask, including a reply to a message without
//! text; the unavailable-default notice is confirmed by Telegram before the offset checkpoint) and
//! the reviewer's test-oracle notes. The bridge-level census and wiring pins live beside the
//! bridge: `src/l3_services/telegram/bridge.rs` tests and `tests/it/telegram.rs`.
#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "fixtures build and inspect real scratch files; the boundary is about PRODUCT code"
    )]

    use super::super::tests::{policy, temp};
    use super::super::{
        Accepted, Chat, Cycle, CycleFailure, Inbox, Knobs, Origin, Refusal, Submit, Submitted,
        Update, load_offset,
    };
    use crate::json::{self, Value};
    use crate::telegram::routing::{
        self, Chosen, Inbound, Route, RunningSession, Seat, Sticky, Target, World, decide,
    };
    use crate::telegram::tests::{CHAT, Fake, Reply};
    use std::path::PathBuf;
    use std::sync::Mutex;

    const NOW: i64 = 1_800_000_000;
    const ASK: &str = "status please";

    // ─── fixtures ────────────────────────────────────────────────────────

    fn session(name: &str, pair: &[&str]) -> RunningSession {
        RunningSession {
            name: name.to_owned(),
            dir: PathBuf::from("/sessions").join(name),
            session_id: format!("{name}-0123456789abcdef"),
            meta_agent: false,
            main: pair.first().map(|seat| format!("claude:{seat}")),
            agents: pair.iter().map(|seat| format!("claude:{seat}")).collect(),
            pair: pair.iter().map(|seat| (*seat).to_owned()).collect(),
            last_active: Some(NOW),
        }
    }

    fn pair_session(name: &str) -> RunningSession {
        session(name, &["lead", "colead"])
    }

    fn target(name: &str, seat: Option<&str>) -> Target {
        Target {
            session: name.to_owned(),
            dir: PathBuf::from("/sessions").join(name),
            session_id: format!("{name}-0123456789abcdef"),
            pair: vec!["lead".to_owned(), "colead".to_owned()],
            seat: seat.map(ToOwned::to_owned),
        }
    }

    /// The census proves `name` at its canonical directory, as `session` lays it out.
    fn held(name: &str) -> Seat {
        Seat::Held {
            name: name.to_owned(),
            dir: PathBuf::from("/sessions").join(name),
        }
    }

    /// A live record of the same name under a worktree: another session, not the seat.
    fn nested(name: &str) -> RunningSession {
        RunningSession {
            dir: PathBuf::from("/worktrees/wt/.ae").join(name),
            session_id: format!("nested-{name}-fedcba9876543210"),
            ..pair_session(name)
        }
    }

    fn plain(text: &str) -> Inbound<'_> {
        Inbound {
            text,
            reply_to: None,
        }
    }

    fn replying<'a>(text: &'a str, quoted: &'a str) -> Inbound<'a> {
        Inbound {
            text,
            reply_to: Some(quoted),
        }
    }

    fn at(text: &str, sessions: &[RunningSession], sticky: &Sticky, seat: &Seat) -> Route {
        decide(plain(text), sessions, sticky, seat, NOW)
    }

    fn selected(session: &str) -> Sticky {
        Sticky::Set {
            session: session.to_owned(),
            agent: routing::ANY_SEAT.to_owned(),
        }
    }

    // ─── the rung itself ─────────────────────────────────────────────────

    #[test]
    fn with_no_selection_plain_text_asks_the_proven_seat_as_the_chosen_default() {
        let world = vec![pair_session("work"), pair_session("orchestrator")];
        assert_eq!(
            at(ASK, &world, &Sticky::Unset, &held("orchestrator")),
            Route::Chat {
                target: target("orchestrator", None),
                text: ASK.to_owned(),
                chosen: Chosen::Default,
            }
        );
        // The text is trimmed first, as for every other rung.
        assert_eq!(
            at(
                "  status please \n",
                &world,
                &Sticky::Unset,
                &held("orchestrator")
            ),
            Route::Chat {
                target: target("orchestrator", None),
                text: ASK.to_owned(),
                chosen: Chosen::Default,
            }
        );
    }

    #[test]
    fn the_seat_is_what_the_census_proves_never_a_name_or_a_meta_bit() {
        // The seat was renamed to `boss`; an ordinary project session is now called
        // `orchestrator`. A state the census can produce holds no other role claim, so these
        // decoys carry none; a bare or a duplicate claim is an unavailable seat, below.
        let world = vec![
            pair_session("orchestrator"),
            pair_session("ws"),
            pair_session("boss"),
        ];
        let Route::Chat { target, chosen, .. } = at(ASK, &world, &Sticky::Unset, &held("boss"))
        else {
            panic!("a proven renamed seat must take the default ask");
        };
        assert_eq!((target.session.as_str(), chosen), ("boss", Chosen::Default));
        // And the census, not the name, decides in the other direction too.
        let Route::Chat { target, .. } = at(ASK, &world, &Sticky::Unset, &held("orchestrator"))
        else {
            panic!("the census named this session");
        };
        assert_eq!(target.session, "orchestrator");
        assert_eq!(target.session_id, "orchestrator-0123456789abcdef");
    }

    #[test]
    fn the_seat_is_bound_to_its_canonical_directory_not_to_its_name() {
        // `role_holder` proves a name under `<AE_HOME>/sessions` only, while the running
        // listing also holds worktree-nested records. A nested namesake is another session.
        let canonical = pair_session("boss");
        let namesake = nested("boss");
        // Both run: the canonical directory is the seat, and the namesake is no ambiguity.
        for world in [
            vec![namesake.clone(), canonical.clone()],
            vec![canonical, namesake],
        ] {
            let Route::Chat { target, chosen, .. } = at(ASK, &world, &Sticky::Unset, &held("boss"))
            else {
                panic!("the canonical seat runs and takes the default ask");
            };
            assert_eq!(chosen, Chosen::Default);
            assert_eq!(target.dir, PathBuf::from("/sessions/boss"));
            assert_eq!(target.session_id, "boss-0123456789abcdef");
        }
        // Only the namesake runs: the seat is stopped, and the default refuses (see the table
        // below) rather than asking another session's chat.
        let Route::Refused { notice, .. } =
            at(ASK, &[nested("boss")], &Sticky::Unset, &held("boss"))
        else {
            panic!("a nested namesake must not stand in for a stopped canonical seat");
        };
        assert!(notice.contains("is not running"), "{notice}");
    }

    // ─── precedence: every other rung still wins ─────────────────────────

    #[test]
    fn a_selection_an_at_prefix_and_a_reply_each_beat_the_default() {
        let world = vec![pair_session("orchestrator"), pair_session("work")];
        let seat = held("orchestrator");
        // 1. the /use selection (rung 4) — plain text goes there, not to the seat.
        assert_eq!(
            at(ASK, &world, &selected("work"), &seat),
            Route::Chat {
                target: target("work", None),
                text: ASK.to_owned(),
                chosen: Chosen::Sticky,
            }
        );
        // 2. `@session` (rung 3), with no selection and with one.
        for sticky in [Sticky::Unset, selected("orchestrator")] {
            assert_eq!(
                at("@work status please", &world, &sticky, &seat),
                Route::Chat {
                    target: target("work", None),
                    text: ASK.to_owned(),
                    chosen: Chosen::Explicit,
                }
            );
        }
        // 3. a swipe-reply to a forwarded row (rung 2).
        for sticky in [Sticky::Unset, selected("orchestrator")] {
            assert_eq!(
                decide(
                    replying("status please", "[work] colead\nready?"),
                    &world,
                    &sticky,
                    &seat,
                    NOW,
                ),
                Route::Chat {
                    target: target("work", Some("colead")),
                    text: ASK.to_owned(),
                    chosen: Chosen::Reply,
                }
            );
        }
    }

    #[test]
    fn a_reply_beats_an_at_prefix_and_a_selection_and_keeps_the_whole_text() {
        // Three distinct targets: the replied-to row's session, the `@` prefix and the selection.
        let world = vec![
            pair_session("orchestrator"),
            pair_session("work"),
            pair_session("other"),
            pair_session("third"),
        ];
        let typed = "@other do it now";
        for sticky in [Sticky::Unset, selected("third")] {
            assert_eq!(
                decide(
                    replying(typed, "[work] colead\nready?"),
                    &world,
                    &sticky,
                    &held("orchestrator"),
                    NOW,
                ),
                Route::Chat {
                    target: target("work", Some("colead")),
                    text: typed.to_owned(),
                    chosen: Chosen::Reply,
                },
                "the reply wins, and the `@other` stays in the text it asks"
            );
        }
    }

    #[test]
    fn every_slash_command_beats_the_default_even_sent_as_a_reply() {
        let world = vec![pair_session("orchestrator"), pair_session("work")];
        let seat = held("orchestrator");
        for command in [
            "/help",
            "/list",
            "/quota",
            "/use",
            "/use work",
            "/use clear",
            "/session work ask lead hello",
            "/frobnicate",
        ] {
            for quoted in [None, Some("[work] lead\nsaid"), Some("")] {
                let route = decide(
                    Inbound {
                        text: command,
                        reply_to: quoted,
                    },
                    &world,
                    &Sticky::Unset,
                    &seat,
                    NOW,
                );
                assert!(
                    !matches!(route, Route::Refused { .. })
                        && !matches!(
                            route,
                            Route::Chat {
                                chosen: Chosen::Default | Chosen::Reply,
                                ..
                            }
                        ),
                    "{command} as {quoted:?} must stay a command: {route:?}"
                );
            }
        }
        assert_eq!(
            at("/use clear", &world, &Sticky::Unset, &seat),
            Route::Unuse
        );
        assert_eq!(at("/quota", &world, &Sticky::Unset, &seat), Route::Quota);
    }

    #[test]
    fn a_corrupt_selection_never_falls_through_to_the_default() {
        let world = vec![pair_session("orchestrator")];
        let Route::Answer(text) = at(ASK, &world, &Sticky::Corrupt, &held("orchestrator")) else {
            panic!("an unreadable selection must refuse, not ask the seat");
        };
        assert!(text.contains("nothing was sent"), "{text}");
        assert!(
            text.contains("/use clear"),
            "the way back to the default is named: {text}"
        );
    }

    // ─── the refusal: cause, both alternatives, nothing sent ─────────────

    #[test]
    fn an_unusable_default_refuses_with_its_cause_and_both_alternatives_and_asks_nothing() {
        let mut role_bit_only = pair_session("ws");
        role_bit_only.meta_agent = true;
        let mut decoy = pair_session("orchestrator");
        decoy.meta_agent = true;
        let no_pair = session("boss", &[]);
        let cases: Vec<(&str, Seat, Vec<RunningSession>, Vec<&str>)> = vec![
            (
                "no seat recorded",
                Seat::Vacant,
                vec![pair_session("work")],
                vec!["no orchestrator seat is recorded"],
            ),
            (
                "a workspace with only the role bit",
                Seat::Unproven(
                    "session 'ws' claims the orchestrator role without the seat overlay".to_owned(),
                ),
                vec![role_bit_only.clone()],
                vec![
                    "cannot be proven",
                    "session 'ws' claims the orchestrator role without the seat overlay",
                ],
            ),
            (
                "duplicate claims",
                Seat::Unproven("multiple sessions claim the orchestrator role".to_owned()),
                vec![role_bit_only.clone(), decoy.clone()],
                vec![
                    "cannot be proven",
                    "multiple sessions claim the orchestrator role",
                ],
            ),
            (
                "a proven seat that is stopped, an unrelated workspace running",
                held("boss"),
                vec![role_bit_only, decoy, pair_session("work")],
                vec!["boss", "is not running"],
            ),
            (
                "a proven seat with no lead pair",
                held("boss"),
                vec![no_pair],
                vec!["boss", "no lead pair"],
            ),
            (
                "a stopped canonical seat while a live nested record has its name",
                held("boss"),
                vec![nested("boss")],
                vec!["boss", "is not running"],
            ),
        ];
        for (label, seat, world, fragments) in cases {
            let route = at(ASK, &world, &Sticky::Unset, &seat);
            let Route::Refused { notice, text } = route else {
                panic!("{label}: the unusable default must refuse, not ask or answer: {route:?}");
            };
            assert_eq!(text, ASK, "{label}: the message is handed on untouched");
            let lower = notice.to_lowercase();
            for fragment in &fragments {
                assert!(
                    lower.contains(fragment),
                    "{label}: {fragment:?} in {notice:?}"
                );
            }
            for required in ["nothing was sent", "@session", "/use"] {
                assert!(
                    notice.contains(required),
                    "{label}: {required:?} in {notice:?}"
                );
            }
            assert!(!notice.contains('\n'), "{label}: one line: {notice:?}");
        }
    }

    #[test]
    fn a_reply_that_names_no_session_is_refused_and_never_asked_of_the_default() {
        let world = vec![pair_session("orchestrator"), pair_session("work")];
        // The replied-to message carried no text (a photo, a sticker, a voice note):
        // reply presence survives, so this is a reply the bridge cannot route.
        for sticky in [Sticky::Unset, selected("work")] {
            let route = decide(
                replying("yes, go ahead", ""),
                &world,
                &sticky,
                &held("orchestrator"),
                NOW,
            );
            let Route::Answer(text) = route else {
                panic!("a reply to nothing routable must answer, not ask anyone: {route:?}");
            };
            assert!(text.contains("reply"), "{text}");
            assert!(
                text.contains("@session"),
                "the way forward is named: {text}"
            );
        }
    }

    #[test]
    fn help_and_use_say_where_unselected_plain_text_goes() {
        let world = vec![pair_session("orchestrator")];
        let seat = held("orchestrator");
        let Route::Answer(help) = at("/help", &world, &Sticky::Unset, &seat) else {
            panic!("/help answers");
        };
        assert!(help.contains("orchestrator"), "{help}");
        let Route::Answer(none) = at("/use", &world, &Sticky::Unset, &seat) else {
            panic!("/use answers");
        };
        assert!(none.contains("orchestrator"), "{none}");
        let Route::Answer(current) = at("/use", &world, &selected("work"), &seat) else {
            panic!("/use answers");
        };
        assert!(current.contains("Selected: work"), "{current}");
    }

    // ─── Message::parse keeps reply presence (raw Bot API JSON) ──────────

    fn reply_to_of(message_extra: &str) -> Option<String> {
        let raw = format!(
            r#"{{"update_id":1,"message":{{"text":"hello","date":{NOW},"from":{{"id":42}},
               "chat":{{"id":{CHAT},"type":"private"}}{message_extra}}}}}"#
        );
        let value = json::parse(&raw).unwrap();
        Update::parse(&value).unwrap().message.unwrap().reply_to
    }

    #[test]
    fn a_reply_survives_parsing_with_text_without_text_and_as_an_external_reply() {
        assert_eq!(reply_to_of(""), None, "no reply at all");
        assert_eq!(
            reply_to_of(r#","reply_to_message":null"#),
            None,
            "an explicit null is no reply"
        );
        assert_eq!(
            reply_to_of(r#","reply_to_message":{"message_id":3,"text":"[work] lead\nhi"}"#),
            Some("[work] lead\nhi".to_owned()),
            "the quoted text still reaches routing"
        );
        for textless in [
            r#","reply_to_message":{"message_id":3,"photo":[{"file_id":"x"}]}"#,
            r#","reply_to_message":{"message_id":3,"sticker":{"file_id":"x"}}"#,
            r#","reply_to_message":{"message_id":3,"voice":{"file_id":"x"}}"#,
            r#","reply_to_message":{}"#,
            r#","external_reply":{"origin":{"type":"hidden_user"},"text":"from elsewhere"}"#,
            r#","external_reply":{"origin":{"type":"hidden_user"},"photo":[{"file_id":"x"}]}"#,
        ] {
            assert_eq!(
                reply_to_of(textless),
                Some(String::new()),
                "{textless}: a reply that carries no routable text is still a reply"
            );
        }
    }

    // ─── the whole cycle: the fake Telegram, the real Inbox ──────────────

    struct Fleet {
        sessions: Vec<RunningSession>,
        seat: Seat,
    }

    impl World for Fleet {
        fn running(&self) -> Vec<RunningSession> {
            self.sessions.clone()
        }

        fn seat(&self) -> Seat {
            self.seat.clone()
        }
    }

    fn fleet(sessions: Vec<RunningSession>, seat: Seat) -> Fleet {
        Fleet { sessions, seat }
    }

    /// Records every submit and every word said, and answers a CONFIRMED word as told.
    struct Probe {
        /// Chat directory, seat named, text and request id of each submit.
        submits: Mutex<Vec<(PathBuf, String, String, String)>>,
        /// `(confirmed, text)` of each word said.
        said: Mutex<Vec<(bool, String)>>,
        confirm: Mutex<Accepted>,
        /// Accept only the give-up hand-back, and refuse any other confirmed word.
        only_the_hand_back: bool,
        refuse: Option<Refusal>,
    }

    impl Probe {
        fn new(confirm: Accepted) -> Self {
            Self {
                submits: Mutex::default(),
                said: Mutex::default(),
                confirm: Mutex::new(confirm),
                only_the_hand_back: false,
                refuse: None,
            }
        }

        fn submits(&self) -> Vec<(PathBuf, String, String, String)> {
            self.submits.lock().unwrap().clone()
        }

        fn confirmed(&self) -> Vec<String> {
            let said = self.said.lock().unwrap();
            said.iter()
                .filter(|(confirmed, _)| *confirmed)
                .map(|(_, text)| text.clone())
                .collect()
        }

        fn unconfirmed(&self) -> Vec<String> {
            let said = self.said.lock().unwrap();
            said.iter()
                .filter(|(confirmed, _)| !*confirmed)
                .map(|(_, text)| text.clone())
                .collect()
        }

        fn will_accept(&self, confirm: Accepted) {
            *self.confirm.lock().unwrap() = confirm;
        }
    }

    impl Submit for Probe {
        fn submit(&self, target: &Target, text: &str, origin: &Origin) -> Submitted {
            let id = origin.request_id();
            self.submits.lock().unwrap().push((
                target.dir.clone(),
                target.seat.clone().unwrap_or_default(),
                text.to_owned(),
                id.clone(),
            ));
            match self.refuse {
                Some(refusal) => Submitted::No(refusal),
                None => Submitted::Sent {
                    id,
                    seat: target.seat.clone().unwrap_or_else(|| "lead".to_owned()),
                    note: None,
                },
            }
        }
    }

    impl Chat for Probe {
        fn say(&self, text: &str) {
            self.said.lock().unwrap().push((false, text.to_owned()));
        }

        fn say_confirmed(&self, text: &str) -> Accepted {
            self.said.lock().unwrap().push((true, text.to_owned()));
            if self.only_the_hand_back && !text.starts_with("Could not deliver") {
                return Accepted::No;
            }
            *self.confirm.lock().unwrap()
        }
    }

    fn result(updates: &[String]) -> String {
        format!(r#"{{"ok":true,"result":[{}]}}"#, updates.join(","))
    }

    /// One update as Telegram frames it; `extra` is more JSON members of `message`.
    fn update(id: i64, text: &str, extra: &str) -> String {
        format!(
            r#"{{"update_id":{id},"message":{{"text":{},"date":{NOW},"from":{{"id":42}},
               "chat":{{"id":{CHAT},"type":"private"}}{extra}}}}}"#,
            Value::str(text).render()
        )
    }

    /// A state directory with NO selection (the shared fixture seeds one).
    fn unselected(tag: &str) -> PathBuf {
        let dir = temp(&format!("tgdefault-{tag}"));
        std::fs::remove_file(dir.join(routing::TARGET_FILE)).unwrap();
        dir
    }

    fn select(dir: &std::path::Path, line: &str) {
        std::fs::write(dir.join(routing::TARGET_FILE), line).unwrap();
    }

    fn poll(inbox: &mut Inbox, api: &crate::telegram::Api, fleet: &Fleet, probe: &Probe) -> Cycle {
        inbox.poll(api, &policy(), fleet, probe, probe, NOW)
    }

    fn orchestrator_fleet() -> Fleet {
        fleet(
            vec![pair_session("orchestrator"), pair_session("work")],
            held("orchestrator"),
        )
    }

    #[test]
    fn an_unselected_plain_message_asks_the_orchestrator_exactly_as_a_selected_one_would() {
        // Same Submit path, same Origin (so the same request id and the same admission), same
        // acknowledgement — the five-open-asks cap and the reply threading are that path's.
        let ask = |selection: Option<&str>| {
            let dir = unselected("same-path");
            if let Some(line) = selection {
                select(&dir, line);
            }
            let fake = Fake::one(Reply::json(200, &result(&[update(7, ASK, "")])));
            let probe = Probe::new(Accepted::Yes);
            let mut inbox = Inbox::new(&dir, Knobs::default());
            let cycle = poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
            assert!(cycle.failure.is_none(), "{cycle:?}");
            assert_eq!(cycle.routed, 1, "{cycle:?}");
            let offset = load_offset(&inbox.offset_path()).unwrap();
            std::fs::remove_dir_all(&dir).ok();
            (
                probe.submits(),
                probe.unconfirmed(),
                probe.confirmed(),
                offset,
            )
        };
        let by_default = ask(None);
        let by_selection = ask(Some("orchestrator\t*\n"));
        assert_eq!(by_default.0.len(), 1, "exactly one ask: {by_default:?}");
        assert_eq!(by_default.0[0].0, PathBuf::from("/sessions/orchestrator"));
        assert_eq!(
            by_default.0[0].1, "",
            "the chat's own router picks the seat"
        );
        assert_eq!(by_default.0[0].2, ASK);
        assert_eq!(
            by_default, by_selection,
            "the default is a selection nobody typed"
        );
        assert_eq!(
            by_default.3, 7,
            "the offset is durable once the ask is routed"
        );
        assert!(
            by_default
                .1
                .iter()
                .any(|said| said.contains("asked lead in orchestrator")),
            "{by_default:?}"
        );
        assert!(
            by_default.2.is_empty(),
            "an ask needs no confirmed word: {by_default:?}"
        );
    }

    #[test]
    fn a_default_ask_the_chat_will_not_take_is_owed_like_a_selected_one() {
        let owed = |selection: Option<&str>| {
            let dir = unselected("owed");
            if let Some(line) = selection {
                select(&dir, line);
            }
            let fake = Fake::one(Reply::json(200, &result(&[update(7, ASK, "")])));
            let mut probe = Probe::new(Accepted::Yes);
            probe.refuse = Some(Refusal::Transient);
            let mut inbox = Inbox::new(&dir, Knobs::default());
            let cycle = poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
            let offset = load_offset(&inbox.offset_path()).unwrap();
            std::fs::remove_dir_all(&dir).ok();
            (cycle.failure, offset, probe.submits().len())
        };
        let by_default = owed(None);
        assert_eq!(
            by_default.0,
            Some(CycleFailure::Undelivered(
                "the chat of orchestrator".to_owned()
            ))
        );
        assert_eq!(
            (by_default.1, by_default.2),
            (0, 1),
            "held and attempted once"
        );
        assert_eq!(by_default, owed(Some("orchestrator\t*\n")));
    }

    #[test]
    fn an_unavailable_default_is_told_confirmed_and_holds_the_offset_until_telegram_accepts() {
        let dir = unselected("notice");
        let update = update(7, ASK, "");
        let fake = Fake::start(vec![
            Reply::json(200, &result(std::slice::from_ref(&update))),
            Reply::json(200, &result(std::slice::from_ref(&update))),
        ]);
        let api = fake.api();
        let nowhere = fleet(vec![pair_session("work")], Seat::Vacant);
        let probe = Probe::new(Accepted::No);
        let mut inbox = Inbox::new(&dir, Knobs::default());

        // Telegram refuses the notice: nobody was told, so the update stays owed.
        let held_back = poll(&mut inbox, &api, &nowhere, &probe);
        assert!(
            matches!(held_back.failure, Some(CycleFailure::Undelivered(_))),
            "{held_back:?}"
        );
        assert_eq!(
            load_offset(&inbox.offset_path()).unwrap(),
            0,
            "the offset moved"
        );
        assert!(
            !held_back.retry_after.is_zero(),
            "a hold asks for a backoff"
        );
        assert_eq!(probe.confirmed().len(), 1, "the notice is sent CONFIRMED");
        assert!(
            probe.unconfirmed().is_empty(),
            "never best-effort: {:?}",
            probe.unconfirmed()
        );

        // Telegram accepts the retry: told, so the update is checkpointed.
        probe.will_accept(Accepted::Yes);
        let told = poll(&mut inbox, &api, &nowhere, &probe);
        assert!(told.failure.is_none(), "{told:?}");
        assert_eq!(told.routed, 1, "{told:?}");
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 7);
        assert!(
            probe.submits().is_empty(),
            "zero submit calls, in both polls"
        );
        let notices = probe.confirmed();
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert_eq!(notices[0], notices[1], "the same notice both times");
        let lower = notices[1].to_lowercase();
        assert!(
            lower.contains("no orchestrator seat is recorded"),
            "{lower}"
        );
        assert!(lower.contains("nothing was sent"), "{lower}");
        assert!(lower.contains("@session"), "{lower}");
        assert!(lower.contains("/use"), "{lower}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_held_default_is_decided_again_when_the_seat_is_back() {
        let dir = unselected("recovers");
        let update = update(7, ASK, "");
        let fake = Fake::start(vec![
            Reply::json(200, &result(std::slice::from_ref(&update))),
            Reply::json(200, &result(std::slice::from_ref(&update))),
        ]);
        let api = fake.api();
        let probe = Probe::new(Accepted::No);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let stopped = fleet(vec![pair_session("work")], held("orchestrator"));
        poll(&mut inbox, &api, &stopped, &probe);
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 0);
        assert!(probe.submits().is_empty());
        assert!(
            probe.confirmed()[0]
                .to_lowercase()
                .contains("is not running"),
            "{:?}",
            probe.confirmed()
        );

        // The seat is running when Telegram redelivers the same update.
        let back = poll(&mut inbox, &api, &orchestrator_fleet(), &probe);
        assert!(back.failure.is_none(), "{back:?}");
        let submits = probe.submits();
        assert_eq!(submits.len(), 1, "{submits:?}");
        assert_eq!(submits[0].0, PathBuf::from("/sessions/orchestrator"));
        assert_eq!(submits[0].2, ASK);
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 7);
        assert_eq!(
            probe.confirmed().len(),
            1,
            "no second notice once it was asked"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unheard_notice_reaches_the_give_up_and_the_message_comes_back() {
        // The platform refuses the refusal notice itself, but takes the give-up's hand-back.
        let bound = Knobs::default().retry_max as usize;
        let dir = unselected("give-up");
        let update = update(7, ASK, "");
        let fake = Fake::start(
            (0..bound)
                .map(|_| Reply::json(200, &result(std::slice::from_ref(&update))))
                .collect(),
        );
        let api = fake.api();
        let nowhere = fleet(vec![pair_session("work")], Seat::Vacant);
        let mut probe = Probe::new(Accepted::Yes);
        probe.only_the_hand_back = true;
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycles: Vec<Cycle> = (0..bound)
            .map(|_| poll(&mut inbox, &api, &nowhere, &probe))
            .collect();
        for early in &cycles[..bound - 1] {
            assert!(
                matches!(early.failure, Some(CycleFailure::Undelivered(_))),
                "{early:?}"
            );
        }
        let last = cycles.last().unwrap();
        assert!(
            last.failure.is_none(),
            "the hand-back ends the hold: {last:?}"
        );
        assert_eq!(last.undelivered, 1, "{last:?}");
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 7);
        assert!(probe.submits().is_empty());
        let handed_back = probe.confirmed().pop().unwrap();
        assert!(
            handed_back.contains(ASK),
            "the message comes back: {handed_back}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_swipe_reply_to_a_message_without_text_is_refused_through_the_whole_cycle() {
        for selection in [None, Some("work\t*\n")] {
            let dir = unselected("textless-reply");
            if let Some(line) = selection {
                select(&dir, line);
            }
            // What Telegram sends for a swipe-reply to a photo.
            let reply = r#","reply_to_message":{"message_id":3,"photo":[{"file_id":"x"}]}"#;
            let fake = Fake::one(Reply::json(
                200,
                &result(&[update(9, "yes, go ahead", reply)]),
            ));
            let probe = Probe::new(Accepted::Yes);
            let mut inbox = Inbox::new(&dir, Knobs::default());
            let cycle = poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
            assert!(cycle.failure.is_none(), "{selection:?}: {cycle:?}");
            assert_eq!(
                cycle.routed, 1,
                "{selection:?}: answered is routed: {cycle:?}"
            );
            assert!(
                probe.submits().is_empty(),
                "{selection:?}: a reply nobody can route asks no chat: {:?}",
                probe.submits()
            );
            let said: Vec<String> = probe.unconfirmed();
            assert!(
                said.iter()
                    .any(|line| line.contains("which session that was a reply to")),
                "{selection:?}: {said:?}"
            );
            assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 9);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn use_clear_says_plain_messages_ask_the_orchestrator_and_the_next_one_does() {
        let dir = unselected("clear");
        select(&dir, "work\t*\n");
        // `/use clear`, then a plain message, in the SAME poll.
        let fake = Fake::one(Reply::json(
            200,
            &result(&[update(4, "/use clear", ""), update(5, ASK, "")]),
        ));
        let probe = Probe::new(Accepted::Yes);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        let cycle = poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
        assert!(cycle.failure.is_none(), "{cycle:?}");
        assert_eq!(cycle.routed, 2, "{cycle:?}");
        assert!(
            !dir.join(routing::TARGET_FILE).is_file(),
            "the selection is gone"
        );
        let said = probe.unconfirmed();
        assert!(
            said[0].contains("orchestrator") && !said[0].contains("not sent"),
            "the acknowledgement says where plain messages go now: {said:?}"
        );
        let submits = probe.submits();
        assert_eq!(submits.len(), 1, "{submits:?}");
        assert_eq!(
            submits[0].0,
            PathBuf::from("/sessions/orchestrator"),
            "the actual next ask goes to the orchestrator, not the cleared selection"
        );
        assert_eq!((submits[0].1.as_str(), submits[0].2.as_str()), ("", ASK));
        assert_eq!(load_offset(&inbox.offset_path()).unwrap(), 5);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plain_text_from_an_unauthorized_sender_chat_or_chat_type_reaches_no_default() {
        // The default is available and nothing is selected, so only the trust predicate stands
        // between this text and the orchestrator: vary each of its terms on its own.
        let raw = |id: i64, sender: Option<&str>, chat_id: &str, chat_type: &str| {
            let from = sender.map_or_else(String::new, |sender| {
                format!(r#""from":{{"id":{sender}}},"#)
            });
            format!(
                r#"{{"update_id":{id},"message":{{"text":"{ASK}","date":{NOW},{from}
                   "chat":{{"id":{chat_id},"type":"{chat_type}"}}}}}}"#
            )
        };
        let rejected = [
            (
                "a sender outside the allow-list",
                raw(11, Some("43"), CHAT, "private"),
            ),
            ("a message with no sender", raw(12, None, CHAT, "private")),
            (
                "another chat",
                raw(13, Some("42"), "-1009999999999", "private"),
            ),
            (
                "a group of the configured id",
                raw(14, Some("42"), CHAT, "group"),
            ),
            (
                "a supergroup of the configured id",
                raw(15, Some("42"), CHAT, "supergroup"),
            ),
        ];
        for (label, update) in rejected {
            let dir = unselected("ingress");
            let id = json::parse(&update)
                .ok()
                .and_then(|value| match value.get("update_id") {
                    Some(Value::Num(id)) => Some(*id),
                    _ => None,
                });
            let fake = Fake::one(Reply::json(200, &result(&[update])));
            let probe = Probe::new(Accepted::Yes);
            let mut inbox = Inbox::new(&dir, Knobs::default());
            let cycle = poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
            assert!(cycle.failure.is_none(), "{label}: {cycle:?}");
            assert_eq!((cycle.routed, cycle.dropped), (0, 1), "{label}: {cycle:?}");
            assert!(probe.submits().is_empty(), "{label}: asked a chat");
            assert!(
                probe.confirmed().is_empty() && probe.unconfirmed().is_empty(),
                "{label}: an untrusted message earns no response at all"
            );
            assert_eq!(
                Some(load_offset(&inbox.offset_path()).unwrap()),
                id,
                "{label}: a policy drop is still checkpointed"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
        // The same shape with every term admitted IS asked, so the drops above are the trust
        // predicate's and not a fixture mistake.
        let dir = unselected("ingress-control");
        let admitted = raw(16, Some("42"), CHAT, "private");
        let fake = Fake::one(Reply::json(200, &result(&[admitted])));
        let probe = Probe::new(Accepted::Yes);
        let mut inbox = Inbox::new(&dir, Knobs::default());
        poll(&mut inbox, &fake.api(), &orchestrator_fleet(), &probe);
        assert_eq!(probe.submits().len(), 1, "the admitted control is asked");
        std::fs::remove_dir_all(&dir).ok();
    }
}
