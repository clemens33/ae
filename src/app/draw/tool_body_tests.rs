//! #288 mutants slot 2 survivors of the tool pages (driver tests, not frozen
//! spec): the row cells in their page's column order, the progress and
//! recounting lines, and the column drop at the identity's minimum. Expected
//! values are the column heads of plan-r3 §8 and hand counts.

use super::*;
use crate::tool_stats::{
    Ask, McpCols, Page as Kind, Resource, Scope, ScopeClass, SkillCols, ToolCols,
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
        rows: vec![Row { item, cols }],
        more: None,
        items: 1,
        totals: cols,
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

/// The words of the drawn row that starts with `label`.
fn row_words(page: &Page, label: &str) -> Vec<String> {
    let line = lines(page)
        .into_iter()
        .find(|line| line.starts_with(label))
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
