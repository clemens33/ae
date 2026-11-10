//! #288 A1-A7 acceptance tests. Requirements: issue #288, plan-r3, approved
//! Q1b/Q2b, D10 resume-label exception, and `READ_PATH_MAX` ruling.
//! Expected counts are hand counts. Every call enters the real native adapter.
#![allow(
    clippy::expect_used,
    reason = "independent frozen acceptance assertions"
)]

use super::*;
use crate::tool_stats_fixture as f;
use crate::usage::{claude_calls, codex_calls};
use std::path::PathBuf;

fn parsed(adapter: Adapter, text: &str, role: StreamRole, copy: u32) -> Stream {
    let config = [PathBuf::from("/fixture/config")];
    let work = [PathBuf::from("/fixture/work")];
    let roots = claude_calls::Roots {
        config_homes: &config,
        work_dirs: &work,
    };
    let mut stream = Stream {
        role,
        copy,
        events: Vec::new(),
        malformed: 0,
        over_cap: 0,
    };
    for line in text.lines() {
        let status = match adapter {
            Adapter::Claude => claude_calls::line(line.as_bytes(), &roots, &mut stream.events),
            Adapter::Codex => codex_calls::line(line.as_bytes(), &mut stream.events),
        };
        if status == Parsed::Malformed {
            stream.malformed += 1;
        }
    }
    stream
}

fn member(session: &str, role: Role, label: Option<&str>) -> Membership {
    Membership {
        session: Some(session.into()),
        slot: "main".into(),
        role,
        client: label.map_or(RecordedClient::Missing, |label| {
            RecordedClient::Label(label.into())
        }),
    }
}

fn conversation(adapter: Adapter, text: &str) -> Conversation {
    Conversation {
        adapter,
        id: f::C.into(),
        memberships: vec![member(f::S, Role::Live, Some("work"))],
        evidence_complete: true,
        reseated: false,
        streams: vec![parsed(adapter, text, StreamRole::Parent, 0)],
        gaps: Vec::new(),
        read: true,
    }
}

fn ask(page: Page) -> Ask {
    Ask {
        page,
        scope: Scope::All,
        filter: Filter::All,
    }
}

fn view(conversations: &[Conversation], page: Page) -> View {
    fold(conversations, &ask(page), &Caps::SLICE)
}

fn tools(view: &View) -> ToolCols {
    match view.totals {
        Cols::Tool(cols) => cols,
        other => panic!("Tools columns, got {other:?}"),
    }
}

fn skills(view: &View) -> SkillCols {
    match view.totals {
        Cols::Skill(cols) => cols,
        other => panic!("Skills columns, got {other:?}"),
    }
}

fn mcp(view: &View) -> McpCols {
    match view.totals {
        Cols::Mcp(cols) => cols,
        other => panic!("MCP columns, got {other:?}"),
    }
}

fn has_gap(view: &View, kind: &GapKind) -> bool {
    view.gaps
        .iter()
        .any(|gap| gap.kind == *kind && gap.count > 0)
}

fn check_privacy(value: &impl std::fmt::Debug) {
    let text = format!("{value:?}");
    for sentinel in [
        "NEVER_RETAIN",
        "/fixture/",
        "private.invalid",
        "secret-token",
        "s-ok",
        "exec-conflict",
        "scope-user",
    ] {
        assert!(
            !text.contains(sentinel),
            "retained private field {sentinel}: {text}"
        );
    }
}

#[test]
fn a1_claude_evidence_classes_have_independent_hand_counts() {
    let conversation = conversation(Adapter::Claude, f::CLASSES);
    check_privacy(&conversation.streams);
    let inputs = [conversation];
    let skill_view = view(&inputs, Page::Skills);
    assert_eq!(
        skills(&skill_view),
        SkillCols {
            attempts: 7,
            loaded: 1,
            not_loaded: 2,
            unknown: 3,
            unresolved: 1,
            reads: 0,
        }
    );
    assert_eq!(
        skill_view.items, 1,
        "catalog, prose, summary, attribution and meta are not attempts"
    );
    let tool_view = view(&inputs, Page::Tools);
    assert_eq!(
        tools(&tool_view),
        ToolCols {
            attempts: 11,
            returned: 9,
            error: 2,
            unresolved: 2
        }
    );
    assert_eq!(tool_view.orphans, 0);
    assert_eq!(mcp(&view(&inputs, Page::Mcp)), McpCols::default());
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        check_privacy(&view(&inputs, page));
    }
}

#[test]
fn a1_codex_completed_mcp_outcomes_do_not_expand_top_level_tool_counts() {
    let inputs = [conversation(Adapter::Codex, f::CODEX)];
    let tool_view = view(&inputs, Page::Tools);
    assert_eq!(
        tools(&tool_view),
        ToolCols {
            attempts: 3,
            returned: 2,
            error: 0,
            unresolved: 1
        }
    );
    assert_eq!(tool_view.orphans, 1);
    assert_eq!(skills(&view(&inputs, Page::Skills)), SkillCols::default());
    let mcp_view = view(&inputs, Page::Mcp);
    assert_eq!(
        mcp(&mcp_view),
        McpCols {
            attempts: 5,
            returned: 5,
            success: 1,
            error: 2,
            unresolved: 0
        }
    );
    assert_eq!(mcp_view.items, 2, "a/search and b/search remain separate");
    assert!(has_gap(&mcp_view, &GapKind::Conflict));
    check_privacy(&inputs[0].streams);
    check_privacy(&tool_view);
    check_privacy(&mcp_view);
}

#[test]
fn a2_normalization_accepts_only_the_ratified_display_word_grammar() {
    let kinds = [
        Kind::Skill,
        Kind::Server,
        Kind::Tool,
        Kind::Model,
        Kind::Effort,
    ];
    for kind in kinds {
        for good in ["x", "plugin:x", "A_z.2:+-", &"a".repeat(64)] {
            assert_eq!(
                ident(kind, good),
                Ident::Name(good.into()),
                "{kind:?}: {good:?}"
            );
        }
        for bad in [
            "",
            ".hidden",
            "a/b",
            "a\\b",
            "a@b",
            "a b",
            "a\nb",
            "a\u{1b}b",
            "https://private.invalid",
            "ä",
            &"a".repeat(65),
        ] {
            let value = ident(kind, bad);
            assert_eq!(value, Ident::Unresolved(kind), "{kind:?}: {bad:?}");
            if !bad.is_empty() {
                assert!(!format!("{value:?}").contains(bad));
            }
        }
    }
}

#[test]
fn a2_claude_mcp_names_have_a_pinned_namespace_grammar_and_wrapper_boundary() {
    let mut text = String::new();
    let names = [
        "mcp__a__search".to_owned(),
        "mcp__b__search".into(),
        "mcp__a__b__c".into(),
        "mcp__x".into(),
        "mcp__a/b__t".into(),
        format!("mcp__{}__t", "a".repeat(65)),
        // This is a JSON escape, not a raw JSON control byte (which would
        // make the record invalid). The native parser decodes U+0001.
        "mcp__a\\u0001__t".into(),
    ];
    for (index, name) in names.iter().enumerate() {
        let id = format!("call-{index}");
        text.push_str(&f::call(&id, &id, name, "model-a", "{}"));
        text.push_str(&f::result(&id, false, None));
    }
    text.push_str(&f::call(
        "wrapper",
        "wrapper",
        "Bash",
        "model-a",
        r#"{"command":"mcp__hidden__search; cat /fixture/config/skills/x/SKILL.md"}"#,
    ));
    let inputs = [conversation(Adapter::Claude, &text)];
    assert_eq!(tools(&view(&inputs, Page::Tools)).attempts, 8);
    let actual = view(&inputs, Page::Mcp);
    assert_eq!(
        mcp(&actual),
        McpCols {
            attempts: 7,
            returned: 7,
            success: 0,
            error: 0,
            unresolved: 0
        }
    );
    for server in ["a", "b"] {
        assert!(actual.rows.iter().any(|row| row.item
            == Item::Mcp {
                server: Ident::Name(server.into()),
                tool: Ident::Name("search".into()),
            }));
    }
    assert!(!format!("{actual:?}").contains("hidden"));
    assert!(!format!("{actual:?}").contains("a/b"));
    let unresolved: u64 = actual
        .rows
        .iter()
        .filter(|row| {
            matches!(
                &row.item,
                Item::Mcp {
                    server: Ident::Unresolved(_),
                    ..
                } | Item::Mcp {
                    tool: Ident::Unresolved(_),
                    ..
                }
            )
        })
        .map(|row| row.cols.attempts())
        .sum();
    assert_eq!(
        unresolved, 5,
        "every ambiguous or unsafe namespace stays unresolved"
    );
}

#[test]
fn q1_q2_same_named_resources_stay_separate_and_only_proved_reads_count() {
    let inputs = [conversation(Adapter::Claude, f::RESOURCES)];
    let actual = view(&inputs, Page::Skills);
    assert_eq!(
        skills(&actual),
        SkillCols {
            attempts: 5,
            loaded: 5,
            reads: 4,
            ..SkillCols::default()
        }
    );
    let loaded_rows: Vec<_> = actual
        .rows
        .iter()
        .filter(|row| row.cols.attempts() > 0)
        .collect();
    assert_eq!(
        loaded_rows.len(),
        5,
        "same display name cannot merge distinct directories"
    );
    let resources: std::collections::BTreeSet<_> = loaded_rows
        .iter()
        .map(|row| match &row.item {
            Item::Skill {
                scope: Some((class, resource)),
                ..
            } => (*class, *resource),
            other => panic!("base directory proves resource identity, got {other:?}"),
        })
        .collect();
    assert_eq!(resources.len(), 5);
    assert_eq!(
        resources
            .iter()
            .filter(|(class, _)| *class == ScopeClass::Other)
            .count(),
        2
    );
    for (_, resource) in resources {
        assert_eq!(resource.hex().len(), 12);
    }
    for (name, class) in [
        ("same", ScopeClass::User),
        ("same", ScopeClass::Project),
        ("plugin:same", ScopeClass::Plugin),
    ] {
        assert!(loaded_rows.iter().any(|row| matches!(&row.item, Item::Skill { name: Ident::Name(actual), scope: Some((scope, _)) } if actual == name && *scope == class)));
    }
    assert_eq!(
        tools(&view(&inputs, Page::Tools)),
        ToolCols {
            attempts: 18,
            returned: 18,
            error: 1,
            unresolved: 0
        }
    );
    check_privacy(&inputs[0].streams);
    check_privacy(&actual);
}

#[test]
fn q2_each_listed_negative_is_opposed_to_a_successful_resource_read() {
    let paths = [
        ("/fixture/config/skills/direct/SKILL.md", false, 1),
        ("/fixture/work/.claude/skills/direct/SKILL.md", false, 1),
        (
            "/fixture/config/plugins/.claude-plugin/plugin.json",
            false,
            0,
        ),
        ("/fixture/config/plugins/cache/registry.json", false, 0),
        ("/fixture/config/skills/plain.txt", false, 0),
        ("skills/direct/SKILL.md", false, 0),
        ("/fixture/config/skills/direct/SKILL.md", true, 0),
        ("/fixture/config/skills/../direct/SKILL.md", false, 0),
        (
            "/fixture/config/plugins/cache/pkg/skills/direct/SKILL.md",
            false,
            0,
        ),
        (
            "/fixture/config/skills/direct/references/guide.md",
            false,
            0,
        ),
    ];
    for (path, error, expected) in paths {
        let text = f::call(
            "read",
            "read",
            "Read",
            "model-a",
            &format!(r#"{{"file_path":"{path}"}}"#),
        ) + &f::result("read", error, None);
        let actual = view(&[conversation(Adapter::Claude, &text)], Page::Skills);
        assert_eq!(skills(&actual).reads, expected, "{path:?}, error={error}");
        assert_eq!(
            skills(&actual).attempts,
            0,
            "inferred reads never become explicit invocations"
        );
        if expected == 1 {
            let class = if path.starts_with("/fixture/work/") {
                ScopeClass::Project
            } else {
                ScopeClass::User
            };
            assert!(actual.rows.iter().any(|row| matches!(&row.item, Item::Skill { scope: Some((scope, _)), .. } if *scope == class)), "successful direct read retains its actual scope");
        }
    }
    let bash = f::call(
        "bash",
        "bash",
        "Bash",
        "model-a",
        r#"{"command":"cat /fixture/config/skills/direct/SKILL.md"}"#,
    ) + &f::result("bash", false, None);
    assert_eq!(
        skills(&view(&[conversation(Adapter::Claude, &bash)], Page::Skills)).reads,
        0
    );
}

#[test]
fn q2_loaded_skill_proof_is_required_and_is_conversation_local() {
    for (error, load, result_present, expected) in [
        (false, Some(true), true, 1),
        (false, Some(false), true, 0),
        (true, Some(true), true, 0),
        (false, None, true, 0),
        (false, Some(true), false, 0),
    ] {
        let mut text = f::call("load", "load", "Skill", "model-a", r#"{"skill":"custom"}"#)
            + &f::base("load", "/fixture/custom/custom");
        if result_present {
            text.push_str(&f::result("load", error, load));
        }
        let read = f::call(
            "read",
            "read",
            "Read",
            "model-a",
            r#"{"file_path":"/fixture/custom/custom/reference.md"}"#,
        ) + &f::result("read", false, None);
        let mut parent = conversation(Adapter::Claude, &text);
        parent.streams.push(parsed(
            Adapter::Claude,
            &read,
            StreamRole::Child("child".into()),
            0,
        ));
        assert_eq!(
            skills(&view(&[parent.clone()], Page::Skills)).reads,
            expected
        );
        let mut unrelated = conversation(Adapter::Claude, &read);
        unrelated.id = f::D.into();
        parent.streams.pop();
        assert_eq!(
            skills(&view(&[parent, unrelated], Page::Skills)).reads,
            0,
            "no cross-conversation proof"
        );
    }
}

#[test]
fn q1_only_a_linked_first_text_line_proves_the_skill_resource() {
    let prefix = f::call("load", "load", "Skill", "model-a", r#"{"skill":"custom"}"#)
        + &f::result("load", false, Some(true));
    for (injection, proved) in [
        (f::base("load", "/fixture/custom/custom"), true),
        (f::base("other", "/fixture/custom/custom"), false),
        (f::base("load", "relative/custom"), false),
        (f::base("load", "/fixture/custom/../custom"), false),
        (
            f::base("load", "/fixture/custom/custom").replace(
                "Base directory for this skill: ",
                "heading\\nBase directory for this skill: ",
            ),
            false,
        ),
        (
            f::base("load", "/fixture/custom/custom").replace("Base directory", "base directory"),
            false,
        ),
    ] {
        let actual = view(
            &[conversation(
                Adapter::Claude,
                &(prefix.clone() + &injection),
            )],
            Page::Skills,
        );
        assert_eq!(skills(&actual).attempts, 1);
        assert_eq!(actual.rows.len(), 1);
        assert!(
            matches!(&actual.rows[0].item, Item::Skill { scope, .. } if scope.is_some() == proved),
            "{injection}"
        );
    }
}

#[test]
fn q2_deep_resources_are_observed_and_4097_byte_paths_have_a_visible_gap() {
    let directory = "/fixture/custom/skill";
    let deep = format!(
        "{directory}/{}/guide.md",
        std::iter::repeat_n("deep", 24)
            .collect::<Vec<_>>()
            .join("/")
    );
    let load = f::call("load", "load", "Skill", "model-a", r#"{"skill":"custom"}"#)
        + &f::base("load", directory)
        + &f::result("load", false, Some(true));
    let deep_read = f::call(
        "read",
        "read",
        "Read",
        "model-a",
        &format!(r#"{{"file_path":"{deep}"}}"#),
    ) + &f::result("read", false, None);
    assert_eq!(
        skills(&view(
            &[conversation(Adapter::Claude, &(load + &deep_read))],
            Page::Skills
        ))
        .reads,
        1
    );
    let over = format!("/{}", "a".repeat(4096));
    assert_eq!(over.len(), 4097);
    let text = f::call(
        "long",
        "long",
        "Read",
        "model-a",
        &format!(r#"{{"file_path":"{over}"}}"#),
    ) + &f::result("long", false, None);
    let actual = view(&[conversation(Adapter::Claude, &text)], Page::Skills);
    assert_eq!(skills(&actual).reads, 0);
    assert!(has_gap(&actual, &GapKind::ReadPathOverLimit));
}

#[test]
fn a3_codex_results_keep_the_attempts_turn_after_a_model_switch() {
    let text = f::context("a", "model-a", "high")
        + &f::codex_call("call-a", Some("a"), "read")
        + &f::context("b", "model-b", "low")
        + &f::codex_result("call-a")
        + &f::codex_call("call-b", Some("b"), "read")
        + &f::codex_result("call-b");
    let actual = view(&[conversation(Adapter::Codex, &text)], Page::Tools);
    assert_eq!(
        tools(&actual),
        ToolCols {
            attempts: 2,
            returned: 2,
            ..ToolCols::default()
        }
    );
    assert_eq!(actual.detail.len(), 2);
    for (model, effort) in [("model-a", "high"), ("model-b", "low")] {
        assert!(
            actual
                .detail
                .iter()
                .any(|row| row.model == Cell::Value(model.into())
                    && row.effort == Cell::Value(effort.into())
                    && row.attempts == 1)
        );
    }
    let mut selected = ask(Page::Tools);
    selected.filter = Filter::Model(Cell::Value("model-a".into()));
    let filtered = fold(
        &[conversation(Adapter::Codex, &text)],
        &selected,
        &Caps::SLICE,
    );
    assert_eq!(
        tools(&filtered),
        ToolCols {
            attempts: 1,
            returned: 1,
            ..ToolCols::default()
        }
    );
}

#[test]
fn a2_plugin_prefix_is_identity_even_when_the_resource_is_unresolved() {
    let text = f::call("plain", "plain", "Skill", "model-a", r#"{"skill":"x"}"#)
        + &f::call(
            "plugin",
            "plugin",
            "Skill",
            "model-a",
            r#"{"skill":"plugin:x"}"#,
        );
    let actual = view(&[conversation(Adapter::Claude, &text)], Page::Skills);
    assert_eq!(skills(&actual).attempts, 2);
    assert_eq!(actual.rows.len(), 2);
    for name in ["x", "plugin:x"] {
        assert!(actual.rows.iter().any(|row| row.item
            == Item::Skill {
                name: Ident::Name(name.into()),
                scope: None
            }));
    }
}

#[test]
fn a3_no_turn_no_context_and_conflicting_context_do_not_borrow_another_turn() {
    for text in [
        f::context("other", "model-b", "high") + &f::codex_call("call", None, "read"),
        f::context("other", "model-b", "high") + &f::codex_call("call", Some("missing"), "read"),
        f::context("a", "model-a", "high")
            + &f::context("a", "model-b", "low")
            + &f::codex_call("call", Some("a"), "read"),
    ] {
        let actual = view(&[conversation(Adapter::Codex, &text)], Page::Tools);
        assert_eq!(tools(&actual).attempts, 1);
        assert_eq!(actual.detail.len(), 1);
        assert_eq!(actual.detail[0].model, Cell::Unknown);
        assert_eq!(actual.detail[0].effort, Cell::Unknown);
    }
    let missing_effort =
        "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"a\",\"model\":\"model-a\"}}\n"
            .to_owned()
            + &f::codex_call("call", Some("a"), "read");
    let actual = view(
        &[conversation(Adapter::Codex, &missing_effort)],
        Page::Tools,
    );
    assert_eq!(actual.detail.len(), 1);
    assert_eq!(actual.detail[0].effort, Cell::Unknown);
}

#[test]
fn a3_claude_effort_is_unknown_and_child_model_has_its_own_evidence() {
    let parent = f::call("parent", "parent", "Read", "model-a", "{}");
    let child = f::call("child", "child", "Read", "model-b", "{}");
    let mut input = conversation(Adapter::Claude, &parent);
    input.streams.push(parsed(
        Adapter::Claude,
        &child,
        StreamRole::Child("child".into()),
        0,
    ));
    let actual = view(&[input], Page::Tools);
    assert_eq!(tools(&actual).attempts, 2);
    assert_eq!(actual.detail.len(), 2);
    assert!(actual.detail.iter().all(|row| row.effort == Cell::Unknown));
    assert!(
        actual
            .detail
            .iter()
            .any(|row| row.model == Cell::Value("model-b".into()))
    );

    let mut unknown = conversation(Adapter::Claude, &parent);
    let child = child.replace("\"model\":\"model-b\",", "");
    unknown.streams.push(parsed(
        Adapter::Claude,
        &child,
        StreamRole::Child("child".into()),
        0,
    ));
    let actual = view(&[unknown], Page::Tools);
    assert!(actual.detail.iter().any(|row| row.model == Cell::Unknown));
}

#[test]
fn a3_claude_late_results_keep_the_model_at_the_attempt_with_unknown_effort() {
    let text = f::call("old", "old", "Read", "model-a", "{}")
        + &f::call("new", "new", "Read", "model-b", "{}")
        + &f::result("old", false, None);
    let selected = Ask {
        filter: Filter::Model(Cell::Value("model-a".into())),
        ..ask(Page::Tools)
    };
    let actual = fold(
        &[conversation(Adapter::Claude, &text)],
        &selected,
        &Caps::SLICE,
    );
    assert_eq!(
        tools(&actual),
        ToolCols {
            attempts: 1,
            returned: 1,
            ..ToolCols::default()
        }
    );
    assert_eq!(actual.detail[0].model, Cell::Value("model-a".into()));
    assert_eq!(actual.detail[0].effort, Cell::Unknown);
}

#[test]
fn a3_client_proof_requires_complete_consistent_current_membership() {
    let base = conversation(Adapter::Claude, "");
    assert_eq!(client(&base), Cell::Value("work".into()));
    let mut same = base.clone();
    same.memberships
        .push(member(f::T, Role::Live, Some("work")));
    assert_eq!(client(&same), Cell::Value("work".into()));
    let mut variants = Vec::new();
    for role in [Role::Prior, Role::Retired, Role::Archived] {
        let mut input = base.clone();
        input.memberships.push(member(f::T, role, Some("work")));
        variants.push(input);
    }
    for recorded in [
        RecordedClient::Missing,
        RecordedClient::Invalid,
        RecordedClient::Label("other".into()),
    ] {
        let mut input = base.clone();
        let mut membership = member(f::T, Role::Live, None);
        membership.client = recorded;
        input.memberships.push(membership);
        variants.push(input);
    }
    let mut incomplete = base.clone();
    incomplete.evidence_complete = false;
    variants.push(incomplete);
    let mut reseated = base.clone();
    reseated.reseated = true;
    variants.push(reseated);
    let mut no_membership = base;
    no_membership.memberships.clear();
    variants.push(no_membership);
    for input in variants {
        assert_eq!(client(&input), Cell::Unknown, "{input:?}");
    }
}

#[test]
fn a4_repeated_results_and_retries_count_by_native_id_with_orphans_separate() {
    let one = f::call("one", "one", "Read", "model-a", "{}");
    let returned = f::result("one", false, None);
    let mut text = returned.clone() + &one + &one + &returned + &returned;
    text.push_str(&f::call("retry", "retry", "Read", "model-a", "{}"));
    text.push_str(&f::call("retry-two", "retry-two", "Read", "model-a", "{}"));
    text.push_str(&f::result("orphan", false, None));
    let actual = view(&[conversation(Adapter::Claude, &text)], Page::Tools);
    assert_eq!(
        tools(&actual),
        ToolCols {
            attempts: 3,
            returned: 1,
            error: 0,
            unresolved: 2
        }
    );
    assert_eq!(
        actual.orphans, 1,
        "result before attempt is matched after full read"
    );
    let conflict = text + &f::result("one", true, None);
    let actual = view(&[conversation(Adapter::Claude, &conflict)], Page::Tools);
    assert_eq!(
        tools(&actual),
        ToolCols {
            attempts: 3,
            returned: 1,
            error: 0,
            unresolved: 2
        }
    );
    assert!(has_gap(&actual, &GapKind::Conflict));
}

#[test]
fn a4_conflicting_skill_results_remove_successful_load_and_resource_read_proof() {
    let text = f::call("load", "load", "Skill", "model-a", r#"{"skill":"custom"}"#)
        + &f::base("load", "/fixture/custom/custom")
        + &f::result("load", false, Some(true))
        + &f::result("load", false, Some(false))
        + &f::call(
            "read",
            "read",
            "Read",
            "model-a",
            r#"{"file_path":"/fixture/custom/custom/guide.md"}"#,
        )
        + &f::result("read", false, None);
    let actual = view(&[conversation(Adapter::Claude, &text)], Page::Skills);
    assert_eq!(
        skills(&actual),
        SkillCols {
            attempts: 1,
            unknown: 1,
            ..SkillCols::default()
        }
    );
    assert!(has_gap(&actual, &GapKind::Conflict));
}

#[test]
fn a4_copies_alias_only_the_same_stream_while_distinct_origins_keep_counts() {
    let text =
        f::call("shared", "message", "Read", "model-a", "{}") + &f::result("shared", false, None);
    let base = conversation(Adapter::Claude, &text);
    let mut copy = base.clone();
    copy.streams
        .push(parsed(Adapter::Claude, &text, StreamRole::Parent, 1));
    assert_eq!(tools(&view(&[copy.clone()], Page::Tools)).attempts, 1);
    let other_text = text.replace("model-a", "model-b");
    copy.streams[1] = parsed(Adapter::Claude, &other_text, StreamRole::Parent, 1);
    let conflicted = view(&[copy], Page::Tools);
    assert_eq!(tools(&conflicted).attempts, 1);
    assert!(has_gap(&conflicted, &GapKind::Conflict));
    assert_eq!(conflicted.detail.len(), 1);
    assert!(
        conflicted
            .detail
            .iter()
            .all(|row| row.model == Cell::Unknown)
    );
    let mut unrelated = base.clone();
    unrelated.id = f::D.into();
    let actual = view(&[base.clone(), unrelated], Page::Tools);
    assert_eq!(tools(&actual).attempts, 2);
    assert!(has_gap(&actual, &GapKind::SharedId));
    let mut children = conversation(Adapter::Claude, "");
    for child in ["one", "two"] {
        children.streams.push(parsed(
            Adapter::Claude,
            &text,
            StreamRole::Child(child.into()),
            0,
        ));
    }
    assert_eq!(tools(&view(&[children], Page::Tools)).attempts, 2);
    let mut replay = base;
    replay.streams.push(parsed(
        Adapter::Claude,
        &text,
        StreamRole::Child("replay".into()),
        0,
    ));
    assert_eq!(
        tools(&view(&[replay], Page::Tools)).attempts,
        1,
        "parent message-id proves replay"
    );
}

#[test]
fn a4_input_order_and_duplicate_feeding_leave_the_view_stable() {
    let first = conversation(Adapter::Claude, f::CLASSES);
    let mut second = conversation(Adapter::Codex, f::CODEX);
    second.id = f::D.into();
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let expected = view(&[first.clone(), second.clone()], page);
        assert_eq!(view(&[second.clone(), first.clone()], page), expected);
        assert_eq!(
            view(&[first.clone(), second.clone(), first.clone()], page),
            expected
        );
        let mut permuted = first.clone();
        permuted.streams[0].events.reverse();
        assert_eq!(view(&[permuted, second.clone()], page), expected);
    }
}

#[test]
fn a5_filters_precede_row_caps_and_totals_cover_every_item() {
    let mut text = String::new();
    for index in 0..257 {
        let id = format!("call-{index:03}");
        let name = format!("tool-{index:03}");
        let model = if index == 256 {
            "tail-model"
        } else {
            "head-model"
        };
        text.push_str(&f::call(&id, &id, &name, model, "{}"));
    }
    let inputs = [conversation(Adapter::Claude, &text)];
    let actual = view(&inputs, Page::Tools);
    assert_eq!(actual.rows.len(), 256);
    assert_eq!(actual.items, 257);
    assert_eq!(tools(&actual).attempts, 257);
    assert_eq!(
        actual.more,
        Some(More {
            items: 1,
            attempts: 1
        })
    );
    assert!(
        !actual
            .rows
            .iter()
            .any(|row| row.item == Item::Tool(Ident::Name("tool-256".into())))
    );
    let mut selected = ask(Page::Tools);
    selected.filter = Filter::Model(Cell::Value("tail-model".into()));
    let filtered = fold(&inputs, &selected, &Caps::SLICE);
    assert_eq!(tools(&filtered).attempts, 1);
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(
        filtered.rows[0].item,
        Item::Tool(Ident::Name("tool-256".into()))
    );
    assert_eq!(filtered.more, None);
}

#[test]
fn a5_conversation_observation_refusal_and_zero_byte_ledger_report_loss() {
    // Undated Bash calls each yield exactly one Attempt observation. Avoid
    // coupling this cap boundary to any adapter's timestamp metadata policy.
    let native = |id: &str| {
        f::call(id, id, "Bash", "model-a", "{}")
            .replace("\"timestamp\":\"2026-11-01T00:00:01Z\",", "")
    };
    let mut a = conversation(Adapter::Claude, &native("a"));
    let mut b = a.clone();
    a.id = f::C.into();
    b.id = f::D.into();
    let caps = Caps {
        per_conversation: 1,
        ..Caps::SLICE
    };
    a.streams.push(parsed(
        Adapter::Claude,
        &native("two"),
        StreamRole::Child("two".into()),
        0,
    ));
    assert_eq!(
        tools(&fold(&[b.clone()], &ask(Page::Tools), &caps)).attempts,
        1,
        "one native Attempt fits the cap"
    );
    let oversized = fold(&[a.clone()], &ask(Page::Tools), &caps);
    assert_eq!(
        tools(&oversized).attempts,
        0,
        "two complete calls are refused whole"
    );
    assert!(has_gap(&oversized, &GapKind::Ledger));
    let expected = fold(&[a.clone(), b.clone()], &ask(Page::Tools), &caps);
    let reversed = fold(&[b, a], &ask(Page::Tools), &caps);
    assert_eq!(expected, reversed);
    assert!(has_gap(&expected, &GapKind::Ledger));
    let nonempty = conversation(
        Adapter::Claude,
        &f::call("one", "one", "Read", "model-a", "{}"),
    );
    assert_eq!(
        tools(&view(std::slice::from_ref(&nonempty), Page::Tools)).attempts,
        1,
        "positive control"
    );
    let refused = fold(
        std::slice::from_ref(&nonempty),
        &ask(Page::Tools),
        &Caps {
            per_conversation: 0,
            ..Caps::SLICE
        },
    );
    assert_eq!(tools(&refused).attempts, 0);
    assert!(refused.partial());
    let no_bytes = fold(
        &[nonempty],
        &ask(Page::Tools),
        &Caps {
            ledger: 0,
            ..Caps::SLICE
        },
    );
    assert_eq!(tools(&no_bytes).attempts, 0);
    assert!(has_gap(&no_bytes, &GapKind::Ledger));
}

#[test]
fn a1_a5_malformed_native_lines_are_atomic_and_visible_without_echoing_input() {
    let config = [PathBuf::from("/fixture/config")];
    let work = [PathBuf::from("/fixture/work")];
    let roots = claude_calls::Roots {
        config_homes: &config,
        work_dirs: &work,
    };
    for adapter in [Adapter::Claude, Adapter::Codex] {
        for (line, expected) in [
            ("", Parsed::Ignored),
            ("   ", Parsed::Ignored),
            ("{}", Parsed::Ignored),
            ("[]", Parsed::Malformed),
            ("null", Parsed::Malformed),
            ("{\"type\":", Parsed::Malformed),
        ] {
            let mut events = Vec::new();
            let actual = match adapter {
                Adapter::Claude => claude_calls::line(line.as_bytes(), &roots, &mut events),
                Adapter::Codex => codex_calls::line(line.as_bytes(), &mut events),
            };
            assert_eq!(actual, expected, "{adapter:?}: {line:?}");
            assert!(
                events.is_empty(),
                "ignored or malformed records retain no partial event"
            );
        }
    }
    // A valid first call must not escape when the second part of the JSON
    // record is syntactically malformed. This also rejects a leaked Stamp.
    let line = f::call("private-id", "private-message", "Read", "model-a", "{}");
    let broken = format!(
        "{},\"secret\":\"{}\"",
        line.trim_end().trim_end_matches('}'),
        f::PRIVATE
    );
    let mut events = Vec::new();
    assert_eq!(
        claude_calls::line(broken.as_bytes(), &roots, &mut events),
        Parsed::Malformed
    );
    assert!(events.is_empty());
    let input = conversation(Adapter::Claude, &(line + &broken));
    let actual = view(&[input], Page::Tools);
    assert_eq!(tools(&actual).attempts, 1);
    assert!(has_gap(&actual, &GapKind::Malformed));
    check_privacy(&actual);
    assert!(!actual.gaps.is_empty());
    for gap in &actual.gaps {
        check_privacy(&gap.line());
    }
}

#[test]
fn a1_codex_invalid_terminal_shapes_are_malformed_and_namespaces_are_normalized() {
    for (status, error) in [("completed", "\"false\""), ("failed", "null")] {
        let text = format!(
            r#"{{"type":"event_msg","timestamp":"2026-11-01T00:00:00Z","payload":{{"type":"item_completed","item":{{"type":"McpToolCall","id":"private","server":"a","tool":"search","status":"{status}","result":{{"isError":{error}}}}}}}}}"#
        );
        let mut events = Vec::new();
        assert_eq!(
            codex_calls::line(text.as_bytes(), &mut events),
            Parsed::Malformed
        );
        assert!(events.is_empty(), "all-or-nothing line, including Stamp");
    }
    for (namespace, expected) in [
        ("functions", Ident::Name("functions/read".into())),
        ("bad/ns", Ident::Unresolved(Kind::Tool)),
    ] {
        let text = f::codex_call("namespaced", None, "read").replace(
            "\"name\":\"read\"",
            &format!("\"namespace\":\"{namespace}\",\"name\":\"read\""),
        );
        let actual = view(&[conversation(Adapter::Codex, &text)], Page::Tools);
        assert_eq!(tools(&actual).attempts, 1);
        assert_eq!(actual.rows[0].item, Item::Tool(expected));
    }
    // Lead ruling ae-20261108T105503Z-80a5a234: each component is normalized
    // before exactly one display separator joins the two valid components.
    let invalid_name = f::codex_call("bad-name", None, "bad/name").replace(
        "\"name\":\"bad/name\"",
        "\"namespace\":\"functions\",\"name\":\"bad/name\"",
    );
    let actual = view(&[conversation(Adapter::Codex, &invalid_name)], Page::Tools);
    assert_eq!(tools(&actual).attempts, 1);
    assert_eq!(
        actual.rows[0].item,
        Item::Tool(Ident::Unresolved(Kind::Tool))
    );
}

#[test]
fn a5_range_includes_native_noncall_records_and_child_records() {
    let text = "{\"type\":\"user\",\"timestamp\":\"2026-11-01T00:00:00Z\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"no calls here\"}]}}\n".to_owned()
        + &f::call("middle", "middle", "Read", "model-a", "{}");
    let mut input = conversation(Adapter::Claude, &text);
    input.streams.push(parsed(Adapter::Claude, "{\"type\":\"assistant\",\"timestamp\":\"2026-11-02T00:00:00Z\",\"message\":{\"id\":\"child-text\",\"content\":[{\"type\":\"text\",\"text\":\"still no calls\"}]}}\n", StreamRole::Child("later".into()), 0));
    let actual = view(&[input], Page::Tools);
    assert_eq!(tools(&actual).attempts, 1);
    // UTC epoch constants independently calculated from the Gregorian date.
    assert_eq!(
        actual.range,
        Some((1_793_491_200_000_000, 1_793_577_600_000_000))
    );
}

#[test]
fn a2_composite_limits_apply_to_each_part_before_the_display_join() {
    for (ns_len, name_len, valid) in [(64, 64, true), (65, 64, false), (64, 65, false)] {
        let namespace = "n".repeat(ns_len);
        let name = "t".repeat(name_len);
        let text = f::codex_call("composite-boundary", None, &name).replace(
            &format!("\"name\":\"{name}\""),
            &format!("\"namespace\":\"{namespace}\",\"name\":\"{name}\""),
        );
        let actual = view(&[conversation(Adapter::Codex, &text)], Page::Tools);
        assert_eq!(tools(&actual).attempts, 1);
        let expected = if valid {
            Ident::Name(format!("{namespace}/{name}"))
        } else {
            Ident::Unresolved(Kind::Tool)
        };
        assert_eq!(
            actual.rows[0].item,
            Item::Tool(expected),
            "component lengths {ns_len}/{name_len}"
        );
    }
}

#[test]
fn a5_detail_cap_preserves_full_totals_and_filtering_uses_all_observations() {
    let mut text = String::new();
    for index in 0..4097 {
        let model = format!("model-{index:04}");
        let id = format!("detail-{index}");
        text.push_str(&f::call(&id, &id, "Read", &model, "{}"));
    }
    let inputs = [conversation(Adapter::Claude, &text)];
    let actual = view(&inputs, Page::Tools);
    assert_eq!(tools(&actual).attempts, 4097);
    assert_eq!(actual.rows.len(), 1);
    assert_eq!(actual.detail.len(), 4096);
    assert_eq!(
        actual.detail_more,
        Some(More {
            items: 1,
            attempts: 1
        })
    );
    let selected = Ask {
        filter: Filter::Model(Cell::Value("model-4096".into())),
        ..ask(Page::Tools)
    };
    let filtered = fold(&inputs, &selected, &Caps::SLICE);
    assert_eq!(tools(&filtered).attempts, 1);
    assert_eq!(filtered.detail.len(), 1);
    assert_eq!(filtered.detail_more, None);
}

#[test]
fn a5_coverage_separates_empty_supported_missing_and_unsupported() {
    let empty = view(&[conversation(Adapter::Claude, "")], Page::Tools);
    assert_eq!(empty.state, State::Numbers);
    assert!(!empty.partial());
    assert_eq!(tools(&empty), ToolCols::default());
    assert_eq!((empty.read, empty.listed), (1, 1));
    let mut missing = conversation(Adapter::Claude, "");
    missing.read = false;
    missing.gaps.push(GapKind::Unlocated);
    let missing = view(&[missing], Page::Tools);
    assert!(missing.partial());
    assert!(has_gap(&missing, &GapKind::Unlocated));
    let none = view(&[], Page::Tools);
    assert_eq!(none.state, State::NoSupported);
    let no_id = fold(
        &[],
        &Ask {
            scope: Scope::This(None),
            ..ask(Page::Tools)
        },
        &Caps::SLICE,
    );
    assert_eq!(no_id.state, State::NoSessionId);
}

#[test]
fn a5_joint_details_and_filter_cycle_preserve_each_dimension() {
    let a = conversation(Adapter::Claude, &f::call("a", "a", "Read", "model-a", "{}"));
    let mut b = conversation(
        Adapter::Codex,
        &(f::context("b", "model-b", "high") + &f::codex_call("b", Some("b"), "read")),
    );
    b.id = f::D.into();
    b.memberships = vec![member(f::T, Role::Live, Some("personal"))];
    let actual = view(&[a.clone(), b.clone()], Page::Tools);
    assert!(actual.filters.contains(&Filter::Effort(Cell::Unknown)));
    // The plan leaves Unknown's position relative to named values open.
    let named_filters: Vec<_> = actual
        .filters
        .iter()
        .filter(|filter| **filter != Filter::Effort(Cell::Unknown))
        .cloned()
        .collect();
    assert_eq!(
        named_filters,
        vec![
            Filter::All,
            Filter::Harness(Adapter::Claude),
            Filter::Harness(Adapter::Codex),
            Filter::Client(Cell::Value("personal".into())),
            Filter::Client(Cell::Value("work".into())),
            Filter::Model(Cell::Value("model-a".into())),
            Filter::Model(Cell::Value("model-b".into())),
            Filter::Effort(Cell::Value("high".into())),
        ]
    );
    for filter in actual.filters.iter().skip(1) {
        let selected = Ask {
            filter: filter.clone(),
            ..ask(Page::Tools)
        };
        let filtered = fold(&[a.clone(), b.clone()], &selected, &Caps::SLICE);
        assert_eq!(tools(&filtered).attempts, 1, "joint dimension {filter:?}");
    }
    let selected = Ask {
        scope: Scope::This(Some(f::T.into())),
        ..ask(Page::Tools)
    };
    let scoped = fold(&[a, b], &selected, &Caps::SLICE);
    assert_eq!(tools(&scoped).attempts, 1);
    assert_eq!(scoped.detail[0].harness, Adapter::Codex);
    assert_eq!(scoped.detail[0].client, Cell::Value("personal".into()));
    assert_eq!(scoped.detail[0].model, Cell::Value("model-b".into()));
    assert_eq!(scoped.detail[0].effort, Cell::Value("high".into()));
}

#[test]
fn a7_nonidentity_fields_cannot_reach_events_views_or_diagnostics() {
    let text = format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-01T00:00:00Z\",\"cwd\":\"/fixture/{p}\",\"attributionSkill\":\"{p}\",\"message\":{{\"id\":\"native-{p}\",\"model\":\"model-a\",\"content\":[{{\"type\":\"text\",\"text\":\"{p}\"}},{{\"type\":\"tool_use\",\"id\":\"native-call-{p}\",\"name\":\"Skill\",\"input\":{{\"skill\":\"safe\",\"args\":\"{p}\",\"credential\":\"secret-token\",\"endpoint\":\"https://private.invalid\"}}}}]}}}}\n",
        p = f::PRIVATE,
    ) + &f::result(&format!("native-call-{}", f::PRIVATE), false, Some(true));
    let input = conversation(Adapter::Claude, &text);
    check_privacy(&input.streams);
    for page in [Page::Skills, Page::Mcp, Page::Tools] {
        let actual = view(std::slice::from_ref(&input), page);
        check_privacy(&actual);
        for gap in actual.gaps {
            check_privacy(&gap.line());
        }
    }
}
