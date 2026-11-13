//! #288 mutants slot 2 survivors of the tool pages (driver tests, not frozen
//! spec): the row cells in their page's column order, the progress and
//! recounting lines, and the column drop at the identity's minimum. Expected
//! values are the column heads of plan-r3 §8 and hand counts.

use super::*;
use crate::meta::RecordedClient;
use crate::tool_stats::{
    Adapter, Ask, Caps, Conversation, Event, GapKind, HarnessGroup, Key, Load, McpCols, McpOutcome,
    Membership, Page as Kind, Resource, Role, Scope, ScopeClass, SkillCols, Stream, StreamRole,
    ToolCols, fold,
};

fn page(room: u16) -> Page {
    Page {
        rows: Vec::new(),
        text: Style::default(),
        dim: Style::default(),
        room,
    }
}

fn lines(page: &Page) -> Vec<String> {
    page.rows.iter().map(|(line, _)| line.clone()).collect()
}

fn tool(name: &str) -> Item {
    Item::Tool(Ident::Name(name.to_owned()))
}

/// A complete one-row view of `kind` whose row is `item` with `cols`.
fn view(kind: Kind, item: Item, cols: Cols) -> View {
    View {
        ask: Ask {
            page: kind,
            scope: Scope::All,
            filter: Filter::All,
        },
        state: State::Numbers,
        rows: vec![Row {
            item: item.clone(),
            cols,
        }],
        more: None,
        items: 1,
        totals: cols,
        harnesses: vec![HarnessGroup {
            harness: Adapter::Claude,
            items: 1,
            totals: cols,
            rows: vec![Row { item, cols }],
            more: None,
        }],
        orphans: 0,
        detail: Vec::new(),
        detail_more: None,
        range: None,
        read: 1,
        listed: 1,
        recounting: 0,
        gaps: Vec::new(),
        filters: vec![Filter::All],
    }
}

/// The words of the drawn row that starts with `label`, indented or not.
fn row_words(page: &Page, label: &str) -> Vec<String> {
    let line = lines(page)
        .into_iter()
        .find(|line| line.trim_start().starts_with(label))
        .unwrap_or_else(|| panic!("a row for {label}: {:?}", lines(page)));
    line.split_whitespace().map(str::to_owned).collect()
}

/// `tool_body.rs:102`, 103, 138, 228: a Skills row draws its proven scope,
/// then its six numbers in the head order; an unproven one says
/// `unresolved`; a Tools or MCP row draws no scope cell.
#[test]
fn each_page_draws_its_row_cells_in_head_order() {
    let skill = Item::Skill {
        name: Ident::Name("fixture-skill".to_owned()),
        scope: Some((ScopeClass::User, Resource([0xab; 6]))),
    };
    let cols = Cols::Skill(SkillCols {
        attempts: 7,
        loaded: 5,
        not_loaded: 4,
        unknown: 3,
        unresolved: 2,
        reads: 9,
    });
    let mut skills = page(200);
    skills.numbers(&view(Kind::Skills, skill, cols), StatsPage::Skills, false);
    assert_eq!(
        row_words(&skills, "fixture-skill"),
        [
            "fixture-skill",
            "user",
            "abababababab",
            "7",
            "5",
            "4",
            "3",
            "2",
            "9"
        ]
    );
    let unproven = Item::Skill {
        name: Ident::Name("bare-skill".to_owned()),
        scope: None,
    };
    let mut bare = page(200);
    bare.numbers(
        &view(Kind::Skills, unproven, cols),
        StatsPage::Skills,
        false,
    );
    assert_eq!(
        row_words(&bare, "bare-skill"),
        ["bare-skill", "unresolved", "7", "5", "4", "3", "2", "9"]
    );
    let mcp = Item::Mcp {
        server: Ident::Name("server".to_owned()),
        tool: Ident::Name("search".to_owned()),
    };
    let cols = Cols::Mcp(McpCols {
        attempts: 8,
        returned: 6,
        success: 5,
        error: 1,
        unresolved: 2,
    });
    let mut servers = page(200);
    servers.numbers(&view(Kind::Mcp, mcp, cols), StatsPage::Mcp, false);
    assert_eq!(
        row_words(&servers, "server / search"),
        ["server", "/", "search", "8", "6", "5", "1", "2"]
    );
    let cols = Cols::Tool(ToolCols {
        attempts: 6,
        returned: 4,
        error: 1,
        unresolved: 2,
    });
    let mut tools = page(200);
    tools.numbers(
        &view(Kind::Tools, tool("Read"), cols),
        StatsPage::Tools,
        false,
    );
    assert_eq!(row_words(&tools, "Read"), ["Read", "6", "4", "1", "2"]);
}

/// `tool_body.rs:208`, 214: the recounting count is said only when there is
/// one, after a partial count or alone.
#[test]
fn recounting_is_said_only_when_a_conversation_is_recounted() {
    let said = |read: u64, listed: u64, recounting: u64| {
        let mut shown = view(Kind::Tools, tool("Read"), Cols::Tool(ToolCols::default()));
        (shown.read, shown.listed, shown.recounting) = (read, listed, recounting);
        let mut drawn = page(200);
        drawn.numbers(&shown, StatsPage::Tools, false);
        lines(&drawn)
            .into_iter()
            .filter(|line| line.contains("recounting") || line.starts_with("Partial"))
            .collect::<Vec<_>>()
    };
    assert_eq!(said(1, 2, 0), ["Partial: 1 of 2 conversations read"]);
    assert_eq!(
        said(1, 2, 3),
        ["Partial: 1 of 2 conversations read · recounting 3 changed"]
    );
    assert_eq!(said(2, 2, 0), Vec::<String>::new());
    assert_eq!(said(2, 2, 3), ["recounting 3 changed"]);
}

/// `tool_body.rs:280`: trailing columns drop only while the identity would
/// keep fewer than [`IDENT_MIN`] cells, and the first column never drops.
#[test]
fn columns_drop_only_below_the_identity_minimum_and_never_the_first() {
    let heads = ["Attempts", "Error"];
    let body = vec![("Read".to_owned(), vec!["1".to_owned(), "2".to_owned()])];
    // Each column spends its width and the gap: 8 + 2 and 5 + 2.
    let spent = 10 + 7;
    let shown = |room: u16| {
        let mut drawn = page(room);
        drawn.table("Tool", &heads, &body);
        let head = lines(&drawn).swap_remove(0);
        heads.iter().filter(|name| head.contains(**name)).count()
    };
    assert_eq!(
        shown(spent + IDENT_MIN),
        2,
        "exactly the minimum keeps both"
    );
    assert_eq!(shown(spent + IDENT_MIN - 1), 1, "one cell less drops one");
    assert_eq!(shown(10), 1, "the first column stays however narrow");
}

// #338: the tool pages group their table by harness. Every view below is the
// product's own `fold` of hand-built conversations, drawn through `rows`, so
// the expected lines are hand counts of the events, never read off the code.
// Skills exist for Claude only; Tools and MCP carry both harnesses.

fn key(id: u8) -> Key {
    Key([id; 16])
}

/// A native tool call; `mcp` is Claude's (server, tool) split of an `mcp__` name.
fn seen_call(id: u8, tool: &str, mcp: Option<(&str, &str)>, turn: Option<u8>) -> Event {
    let name = |word: &str| Ident::Name(word.to_owned());
    Event::Attempt {
        id: key(id),
        tool: name(tool),
        mcp: mcp.map(|(server, tool)| (name(server), name(tool))),
        model: None,
        turn: turn.map(key),
        message: None,
    }
}

fn seen_result(id: u8, error: bool) -> Event {
    Event::Result {
        id: key(id),
        error,
        load: None,
    }
}

/// One conversation; `client` is a proven live label, `None` proves nothing.
fn convo(adapter: Adapter, id: &str, client: Option<&str>, events: Vec<Event>) -> Conversation {
    Conversation {
        adapter,
        id: id.to_owned(),
        memberships: client
            .map(|label| Membership {
                session: Some("session-1".to_owned()),
                slot: "worker".to_owned(),
                role: Role::Live,
                client: RecordedClient::Label(label.to_owned()),
            })
            .into_iter()
            .collect(),
        evidence_complete: client.is_some(),
        reseated: false,
        streams: vec![Stream {
            role: StreamRole::Parent,
            copy: 0,
            events,
            malformed: 0,
            over_cap: 0,
        }],
        gaps: Vec::new(),
        read: true,
    }
}

/// Claude: Read x3 (answered ok, answered error, never answered), Bash x1 (answered ok).
fn claude_tools(client: Option<&str>) -> Conversation {
    convo(
        Adapter::Claude,
        "claude-1",
        client,
        vec![
            seen_call(1, "Read", None, None),
            seen_call(2, "Read", None, None),
            seen_call(3, "Read", None, None),
            seen_call(4, "Bash", None, None),
            seen_result(1, false),
            seen_result(2, true),
            seen_result(4, false),
        ],
    )
}

/// Codex: Read x2 (both answered; the adapter never reports a native tool error), shell x1
/// (never answered), one turn context.
fn codex_tools(client: Option<&str>) -> Conversation {
    let turn = Some(100);
    convo(
        Adapter::Codex,
        "codex-1",
        client,
        vec![
            Event::Context {
                turn: key(100),
                model: Ident::Name("gpt-x".to_owned()),
                effort: Ident::Name("high".to_owned()),
            },
            seen_call(11, "Read", None, turn),
            seen_call(12, "Read", None, turn),
            seen_call(13, "shell", None, turn),
            seen_result(11, false),
            seen_result(12, false),
        ],
    )
}

/// The Statistics page of `kind` for `conversations`, as the rows drawn at `width`.
fn render(
    conversations: &[Conversation],
    kind: Kind,
    filter: Filter,
    caps: &Caps,
    width: u16,
) -> Vec<String> {
    render_styled(conversations, kind, filter, caps, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

/// [`render`] with each row's style.
fn render_styled(
    conversations: &[Conversation],
    kind: Kind,
    filter: Filter,
    caps: &Caps,
    width: u16,
) -> Vec<(String, Style)> {
    let ask = Ask {
        page: kind,
        scope: Scope::All,
        filter: filter.clone(),
    };
    let page = match kind {
        Kind::Skills => StatsPage::Skills,
        Kind::Mcp => StatsPage::Mcp,
        Kind::Tools => StatsPage::Tools,
    };
    let body = StatsBody {
        page,
        filter,
        tools: Some(Box::new(fold(conversations, &ask, caps))),
        ..StatsBody::default()
    };
    super::rows(&body, Paint::of(None), width)
}

/// The main table's rows: after its head line, up to and including `Total`.
fn table(lines: &[String], head: &str) -> Vec<String> {
    let from = lines
        .iter()
        .position(|line| line.starts_with(head) && line.contains("Attempts"))
        .unwrap_or_else(|| panic!("a table headed {head}: {lines:?}"));
    let to = lines
        .iter()
        .skip(from)
        .position(|line| line.starts_with("Total ("))
        .map_or_else(|| panic!("a Total row: {lines:?}"), |at| from + at);
    lines[from + 1..=to].to_vec()
}

fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// The last `count` words of `line` as numbers.
fn tail_numbers(line: &str, count: usize) -> Vec<u64> {
    let all = words(line);
    all[all.len() - count..]
        .iter()
        .map(|word| {
            word.parse()
                .unwrap_or_else(|_| panic!("a number in {line:?}"))
        })
        .collect()
}

const WIDE: u16 = 200;

/// Each row's words, for a comparison that ignores the table's padding.
fn word_rows(rows: &[String]) -> Vec<Vec<&str>> {
    rows.iter().map(|row| words(row)).collect()
}

/// Headings start at the left edge; what is counted under one is indented.
fn indents(rows: &[String]) -> Vec<bool> {
    rows.iter().map(|row| row.starts_with("  ")).collect()
}

/// #338: Tools rows sit under their harness, each heading carries that
/// harness's own totals, and the heading totals add up to the Total row.
/// Read is in both harnesses and counted in each; Total counts it once.
#[test]
fn tools_page_groups_rows_by_harness_with_totals_per_harness() {
    let conversations = [claude_tools(Some("work")), codex_tools(None)];
    let lines = render(&conversations, Kind::Tools, Filter::All, &Caps::SLICE, WIDE);
    let rows = table(&lines, "Tool");
    assert_eq!(
        word_rows(&rows),
        [
            vec!["claude", "(2", "items)", "4", "3", "1", "1"],
            vec!["Read", "3", "2", "1", "1"],
            vec!["Bash", "1", "1", "0", "0"],
            vec!["codex", "(2", "items)", "3", "2", "0", "1"],
            vec!["Read", "2", "2", "0", "0"],
            vec!["shell", "1", "0", "0", "1"],
            vec!["Total", "(3", "items)", "7", "5", "1", "2"],
        ]
    );
    assert_eq!(
        indents(&rows),
        [false, true, true, false, true, true, false],
        "headings at the left edge, their rows indented"
    );
    let heading_sum = |at: usize| -> u64 {
        [0, 3]
            .iter()
            .map(|heading| tail_numbers(&rows[*heading], 4)[at])
            .sum()
    };
    for (at, total) in tail_numbers(&rows[6], 4).into_iter().enumerate() {
        assert_eq!(
            heading_sum(at),
            total,
            "column {at}: the harnesses add up to Total"
        );
    }
}

/// #338: MCP calls of one server and tool in both harnesses stay two rows,
/// one per harness; the Returned, Success, Error and Unresolved columns stay
/// separate, and Claude never counts a result as a success.
#[test]
fn mcp_page_groups_one_server_tool_under_each_harness_it_was_called_in() {
    let claude = convo(
        Adapter::Claude,
        "claude-mcp",
        Some("work"),
        vec![
            seen_call(1, "mcp__docs__search", Some(("docs", "search")), None),
            seen_call(2, "mcp__docs__search", Some(("docs", "search")), None),
            seen_call(3, "mcp__docs__search", Some(("docs", "search")), None),
            seen_result(1, false),
            seen_result(2, true),
        ],
    );
    let mcp = |id: u8, server: &str, tool: &str, outcome: McpOutcome| Event::Mcp {
        id: key(id),
        server: Ident::Name(server.to_owned()),
        tool: Ident::Name(tool.to_owned()),
        outcome,
        turn: None,
    };
    let codex = convo(
        Adapter::Codex,
        "codex-mcp",
        None,
        vec![
            mcp(11, "docs", "search", McpOutcome::Success),
            mcp(12, "docs", "search", McpOutcome::Success),
            mcp(13, "docs", "search", McpOutcome::Error),
            mcp(14, "other", "fetch", McpOutcome::Returned),
        ],
    );
    let lines = render(&[claude, codex], Kind::Mcp, Filter::All, &Caps::SLICE, WIDE);
    assert_eq!(
        word_rows(&table(&lines, "Server / tool")),
        [
            vec!["claude", "(1", "items)", "3", "2", "0", "1", "1"],
            vec!["docs", "/", "search", "3", "2", "0", "1", "1"],
            vec!["codex", "(2", "items)", "4", "4", "2", "1", "0"],
            vec!["docs", "/", "search", "3", "3", "2", "1", "0"],
            vec!["other", "/", "fetch", "1", "1", "0", "0", "0"],
            vec!["Total", "(2", "items)", "7", "6", "2", "2", "1"],
        ]
    );
}

/// #338: Skills exist for Claude only. Its heading totals every evidence
/// column of its rows separately, a reads-only skill included, and Codex,
/// which has conversations but no skill evidence, gets no heading.
#[test]
fn skills_page_groups_claude_evidence_and_draws_no_codex_group() {
    let skill = |id: u8, name: &str| Event::Skill {
        id: key(id),
        skill: Ident::Name(name.to_owned()),
    };
    let claude = convo(
        Adapter::Claude,
        "claude-skills",
        Some("work"),
        vec![
            seen_call(1, "Skill", None, None),
            skill(1, "code-review"),
            Event::Result {
                id: key(1),
                error: false,
                load: Some(Load::Loaded),
            },
            seen_call(2, "Skill", None, None),
            skill(2, "code-review"),
            Event::Result {
                id: key(2),
                error: true,
                load: Some(Load::NotLoaded),
            },
            seen_call(3, "Skill", None, None),
            skill(3, "diagnose"),
            seen_call(4, "Read", None, None),
            Event::Read {
                id: key(4),
                direct: Some((
                    Ident::Name("tdd".to_owned()),
                    ScopeClass::User,
                    Resource([7; 6]),
                )),
                ancestors: Vec::new(),
                over: false,
                skills: true,
            },
            seen_result(4, false),
        ],
    );
    let lines = render(
        &[claude, codex_tools(None)],
        Kind::Skills,
        Filter::All,
        &Caps::SLICE,
        WIDE,
    );
    let rows = table(&lines, "Skill");
    assert_eq!(
        word_rows(&rows),
        [
            vec!["claude", "(3", "items)", "3", "1", "1", "0", "1", "1"],
            vec!["code-review", "unresolved", "2", "1", "1", "0", "0", "0"],
            vec!["diagnose", "unresolved", "1", "0", "0", "0", "1", "0"],
            vec!["tdd", "user", "070707070707", "0", "0", "0", "0", "0", "1"],
            vec!["Total", "(3", "items)", "3", "1", "1", "0", "1", "1"],
        ]
    );
    assert!(
        !rows.iter().any(|row| row.starts_with("codex")),
        "no Codex skill evidence, so no Codex group: {rows:?}"
    );
}

/// #338: a harness with conversations but nothing counted on the page gets
/// no zero group, and an unsupported harness stays its coverage line.
#[test]
fn a_harness_without_counts_gets_no_group_and_an_unsupported_one_stays_a_gap_line() {
    let mut codex = codex_tools(None);
    codex.gaps.push(GapKind::Unsupported("muse".to_owned()));
    let lines = render(&[codex], Kind::Skills, Filter::All, &Caps::SLICE, WIDE);
    assert_eq!(
        word_rows(&table(&lines, "Skill")),
        [vec!["Total", "(0", "items)", "0", "0", "0", "0", "0", "0"]]
    );
    assert!(
        lines.contains(&"unsupported harness (muse: 1 conversations not read)".to_owned()),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("muse")),
        "{lines:?}"
    );
}

/// #338: the grouping is by harness, which the store proves, not by client:
/// a lost client proof, a client filter and a harness filter all keep one
/// group per harness that still has rows.
#[test]
fn grouping_survives_an_unknown_client_and_follows_the_filters() {
    let attempts_of = |rows: &[String]| -> Vec<Vec<u64>> {
        rows.iter()
            .filter(|row| !row.starts_with("  "))
            .map(|row| tail_numbers(row, 4))
            .collect()
    };
    let headings = |rows: &[String]| -> Vec<String> {
        rows.iter()
            .filter(|row| !row.starts_with("  "))
            .map(|row| words(row)[0].to_owned())
            .collect()
    };
    let show = |conversations: &[Conversation], filter: Filter| {
        table(
            &render(conversations, Kind::Tools, filter, &Caps::SLICE, WIDE),
            "Tool",
        )
    };
    // Neither conversation proves its client: both groups stay.
    let lost = show(&[claude_tools(None), codex_tools(None)], Filter::All);
    assert_eq!(headings(&lost), ["claude", "codex", "Total"]);
    assert_eq!(
        attempts_of(&lost),
        [vec![4, 3, 1, 1], vec![3, 2, 0, 1], vec![7, 5, 1, 2]]
    );
    // One proven and one unknown client: each client filter keeps its harness.
    let mixed = [claude_tools(Some("work")), codex_tools(None)];
    let work = show(&mixed, Filter::Client(Cell::Value("work".to_owned())));
    assert_eq!(headings(&work), ["claude", "Total"]);
    assert_eq!(attempts_of(&work), [vec![4, 3, 1, 1], vec![4, 3, 1, 1]]);
    let unknown = show(&mixed, Filter::Client(Cell::Unknown));
    assert_eq!(headings(&unknown), ["codex", "Total"]);
    assert_eq!(attempts_of(&unknown), [vec![3, 2, 0, 1], vec![3, 2, 0, 1]]);
    let codex = show(&mixed, Filter::Harness(Adapter::Codex));
    assert_eq!(headings(&codex), ["codex", "Total"]);
    assert_eq!(attempts_of(&codex), [vec![3, 2, 0, 1], vec![3, 2, 0, 1]]);
}

/// #338: the row cap applies per harness and each group says what its own
/// cut left out right under its rows; its heading still totals every row.
#[test]
fn each_group_says_what_its_own_cut_left_out() {
    let calls = |adapter: Adapter, id: &str, names: &[(&str, u8)]| {
        let mut at = 0_u8;
        let mut events = Vec::new();
        for (name, count) in names {
            for _ in 0..*count {
                at += 1;
                events.push(seen_call(at, name, None, None));
            }
        }
        convo(adapter, id, None, events)
    };
    let conversations = [
        calls(
            Adapter::Claude,
            "claude-cut",
            &[("aaa", 3), ("bbb", 2), ("ccc", 1)],
        ),
        calls(Adapter::Codex, "codex-cut", &[("ddd", 1)]),
    ];
    let caps = Caps {
        rows: 2,
        ..Caps::SLICE
    };
    let lines = render(&conversations, Kind::Tools, Filter::All, &caps, WIDE);
    assert_eq!(
        word_rows(&table(&lines, "Tool")),
        [
            vec!["claude", "(3", "items)", "6", "0", "0", "6"],
            vec!["aaa", "3", "0", "0", "3"],
            vec!["bbb", "2", "0", "0", "2"],
            vec!["+1", "more", "items,", "1", "attempts", "not", "shown"],
            vec!["codex", "(1", "items)", "1", "0", "0", "1"],
            vec!["ddd", "1", "0", "0", "1"],
            vec!["Total", "(4", "items)", "7", "0", "0", "7"],
        ]
    );
}

/// #338: at a narrow width the table drops trailing columns, never the
/// harness headings, and the leading columns still add up.
#[test]
fn narrow_width_keeps_the_harness_headings_and_their_leading_columns() {
    let conversations = [claude_tools(Some("work")), codex_tools(None)];
    let lines = render(&conversations, Kind::Tools, Filter::All, &Caps::SLICE, 36);
    let rows = table(&lines, "Tool");
    let kept: Vec<(bool, Vec<u64>)> = rows
        .iter()
        .filter(|row| !row.starts_with("  "))
        .map(|row| (row.starts_with("claude"), tail_numbers(row, 2)))
        .collect();
    assert_eq!(
        kept,
        [(true, vec![4, 3]), (false, vec![3, 2]), (false, vec![7, 5])]
    );
    assert!(rows[3].starts_with("codex"), "{rows:?}");
}

/// #338 guard, green before and after: Detail keeps its joint harness,
/// client, model and effort rows, ranked by attempts, with the grouped table above.
#[test]
fn detail_keeps_the_joint_rows_beside_the_groups() {
    let conversations = [claude_tools(Some("work")), codex_tools(None)];
    let lines = render(&conversations, Kind::Tools, Filter::All, &Caps::SLICE, WIDE);
    let from = lines
        .iter()
        .position(|line| line == "Detail")
        .unwrap_or_else(|| panic!("a Detail section: {lines:?}"));
    let detail: Vec<&String> = lines[from + 2..]
        .iter()
        .take_while(|line| !line.is_empty())
        .collect();
    assert_eq!(
        detail.iter().map(|row| words(row)).collect::<Vec<_>>(),
        [
            vec!["Read", "claude", "work", "unknown", "unknown", "3"],
            vec!["Read", "codex", "unknown", "gpt-x", "high", "2"],
            vec!["Bash", "claude", "work", "unknown", "unknown", "1"],
            vec!["shell", "codex", "unknown", "gpt-x", "high", "1"],
        ]
    );
}

/// #338: a group's heading is bold, so the sections read at a glance; the
/// rows under it, its cut note and the Total are not. Colour is off here, so
/// the weight is the only mark.
#[test]
fn group_headings_are_bold_and_what_stands_under_them_is_not() {
    let calls = |adapter: Adapter, id: &str, first: u8, names: &[&str]| {
        let events = names
            .iter()
            .zip(first..)
            .map(|(name, at)| seen_call(at, name, None, None))
            .collect();
        convo(adapter, id, None, events)
    };
    let conversations = [
        calls(Adapter::Claude, "claude-bold", 1, &["aaa", "aaa", "bbb"]),
        calls(Adapter::Codex, "codex-bold", 11, &["ccc", "ddd"]),
    ];
    let caps = Caps {
        rows: 1,
        ..Caps::SLICE
    };
    let styled = render_styled(&conversations, Kind::Tools, Filter::All, &caps, WIDE);
    let head = styled
        .iter()
        .position(|(line, _)| line.starts_with("Tool") && line.contains("Attempts"))
        .unwrap_or_else(|| panic!("a table head: {styled:?}"));
    let rows: Vec<(&str, bool)> = styled[head + 1..]
        .iter()
        .take_while(|(line, _)| !line.is_empty())
        .map(|(line, style)| (line.as_str(), style.add_modifier.contains(Modifier::BOLD)))
        .collect();
    let drawn: Vec<(Vec<&str>, bool)> = rows
        .iter()
        .map(|(line, bold)| (words(line)[..2].to_vec(), *bold))
        .collect();
    assert_eq!(
        drawn,
        [
            (vec!["claude", "(2"], true),
            (vec!["aaa", "2"], false),
            (vec!["+1", "more"], false),
            (vec!["codex", "(2"], true),
            (vec!["ccc", "1"], false),
            (vec!["+1", "more"], false),
            (vec!["Total", "(4"], false),
        ]
    );
}
