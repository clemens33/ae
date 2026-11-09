//! The `tool_stats_doc` fuzz seeds are canonical documents, and the codec's
//! refusals hold on them: a seed that stopped parsing would seed nothing.
#![allow(clippy::expect_used, reason = "test assertions")]

use super::*;

fn seed(bytes: &[u8]) -> &[u8] {
    bytes.split_first().map_or(bytes, |(_, rest)| rest)
}

const CONV: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/conv");
const VIEW: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/view");
const ASSOC: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/assoc");
const INTENT: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/intent");
const SOURCES: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/sources");
const COVERAGE: &[u8] = include_bytes!("../../fuzz/seeds/tool_stats_doc/coverage");

#[test]
fn every_seed_parses_and_renders_its_own_bytes() {
    let conv = parse_conv(seed(CONV)).expect("conversation seed");
    assert_eq!(render_conv(&conv), seed(CONV));
    assert_eq!(conv.summary.rows.len(), 1, "the derived counters are there");
    let view = parse_view(seed(VIEW)).expect("view seed");
    assert_eq!(render_view(&view), seed(VIEW));
    let assoc = parse_assoc(seed(ASSOC)).expect("association seed");
    assert_eq!(render_assoc(&assoc), seed(ASSOC));
    let intent = parse_intent(seed(INTENT)).expect("intent seed");
    assert_eq!(render_intent(&intent), seed(INTENT));
    let sources = parse_sources(seed(SOURCES)).expect("association cache seed");
    assert_eq!(render_sources(&sources), seed(SOURCES));
    assert_eq!(sources.len(), 2, "a journal and a meta record");
    let coverage = parse_coverage(seed(COVERAGE)).expect("coverage seed");
    assert_eq!(render_coverage(&coverage), seed(COVERAGE));
    assert_eq!(coverage.unsupported.len(), 3, "one record per conversation");
}

#[test]
fn a_non_canonical_or_cut_or_mistyped_document_is_damage() {
    let text = std::str::from_utf8(seed(CONV)).expect("UTF-8 seed");
    for bad in [
        text.replace("\"read\":true,", "\"read\":true, "),
        text.replace("{\"end\":4}\n", "{\"end\":3}\n"),
        text.replace("{\"end\":4}\n", ""),
        text.replace("\"tool\":\"Read\"", "\"tool\":\"a b\""),
        text.replace("\"Read\",\"mcp\"", "\"Read\",\"extra\":1,\"mcp\""),
    ] {
        assert!(matches!(
            parse_conv(bad.as_bytes()),
            Err(DocError::Damaged(_))
        ));
    }
    assert_eq!(
        parse_view(seed(CONV)),
        Err(DocError::Damaged("head")),
        "a document of another kind"
    );
    let over = vec![b' '; usize::try_from(INTENT_CAP).expect("cap") + 1];
    assert_eq!(parse_intent(&over), Err(DocError::Oversized));
}

/// A conversation document of nothing but `lines` records, one per
/// `(role, malformed)`: `None` is the parent.
fn lines_only(lines: &[(Option<&str>, u64)]) -> String {
    let mut out = vec![
        "{\"tools_conv\":1,\"adapter\":\"claude\",\"id\":\"c1\",\"read\":true,\"refused\":null,\"fallback\":false,\"rebuilt\":false}".to_owned(),
    ];
    out.extend(lines.iter().map(|(role, malformed)| {
        let role = role.map_or_else(|| "null".to_owned(), |name| format!("\"{name}\""));
        format!(
            "{{\"lines\":[{role},\"00000000000000000000000000000000\"],\"malformed\":{malformed},\"over_cap\":0}}"
        )
    }));
    out.push(format!("{{\"end\":{}}}", lines.len() + 1));
    out.push(String::new());
    out.join("\n")
}

/// Review r1 R3, r2: counts are kept exactly; a document whose counts a
/// summary or a combine could not add up in `u64` is damage, never a panic,
/// a wrap or a truncation.
#[test]
fn a_count_past_the_bound_is_damage_not_an_overflow() {
    let max = u64::MAX;
    let big = 1_u64 << 40;
    for (lines, want) in [
        // The review's counterexample first: parent 2^40, child 1.
        (vec![(None, big), (Some("child"), 1)], Some(big + 1)),
        (vec![(None, max), (Some("child"), 1)], None),
        (vec![(None, max - 1), (Some("child"), 1)], None),
        (vec![(None, max - 1)], Some(max - 1)),
    ] {
        let text = lines_only(&lines);
        match (parse_conv(text.as_bytes()), want) {
            (Err(DocError::Damaged(_)), None) => {}
            (Ok(conv), Some(count)) => assert_eq!(
                conv.summary.gaps.get(&GapKind::Malformed),
                Some(&count),
                "{text}"
            ),
            (other, _) => panic!("{text}: {other:?}"),
        }
    }
    // The view seed adds up 1 + 1 (cols), 1 (a name) and 1 (its
    // rebuilt_from_source gap) besides these.
    let text = std::str::from_utf8(seed(VIEW)).expect("UTF-8 seed");
    let at = |cols: u64| text.replace("\"cols\":[1,0,0,1]", &format!("\"cols\":[{cols},0,0,0]"));
    assert!(
        parse_view(at(max - 2).as_bytes()).is_ok(),
        "a total of exactly u64::MAX"
    );
    for bad in [
        at(max - 1),
        text.replace("\"orphans\":0", &format!("\"orphans\":{max}")),
        text.replace("\"over_cap\":0", &format!("\"over_cap\":{max}")),
    ] {
        assert!(
            matches!(parse_view(bad.as_bytes()), Err(DocError::Damaged(_))),
            "{bad}"
        );
    }
}

fn codex_conv(tool: &str) -> String {
    std::str::from_utf8(seed(CONV))
        .expect("UTF-8 seed")
        .replace("\"adapter\":\"claude\"", "\"adapter\":\"codex\"")
        .replace("\"model\":\"model-a\"", "\"model\":null")
        .replace("\"tool\":\"Read\"", &format!("\"tool\":\"{tool}\""))
}

/// Review r1 R4: a persisted identity obeys the adapter grammar of its
/// position: one valid component, a Codex tool two joined by one `/`; a
/// cell or name entry likewise; anything else is damage.
#[test]
fn a_persisted_identity_obeys_the_grammar_of_its_position() {
    let conv = std::str::from_utf8(seed(CONV)).expect("UTF-8 seed");
    let view = std::str::from_utf8(seed(VIEW)).expect("UTF-8 seed");
    let long = "a".repeat(64);
    let longer = "a".repeat(65);
    assert!(
        parse_conv(codex_conv(&format!("{long}/{long}")).as_bytes()).is_ok(),
        "a 64+64 Codex composite stays"
    );
    let damaged = [
        conv.replace(
            "\"tool\":\"Read\"",
            "\"tool\":\"/fixture/PRIVATE_SENTINEL\"",
        ),
        conv.replace("\"tool\":\"Read\"", "\"tool\":\"ns/Read\""),
        conv.replace("\"tool\":\"Read\"", "\"tool\":\".hidden\""),
        conv.replace("\"tool\":\"Read\"", &format!("\"tool\":\"{longer}\"")),
        conv.replace("\"tool\":\"Read\"", "\"tool\":[\"server\"]"),
        conv.replace("\"model\":\"model-a\"", "\"model\":\"a/b\""),
        codex_conv(&format!("{longer}/{long}")),
        codex_conv("a/b/c"),
        codex_conv("/a"),
        codex_conv("a/.b"),
        view.replace("\"model\":\"model-a\"", "\"model\":\"/x/y\""),
        view.replace("\"model\":\"model-a\"", "\"model\":[\"direct\"]"),
        view.replace("\"client\":\"work\"", "\"client\":\"no such label\""),
        view.replace("\"client\":\"work\"", "\"client\":[\"none_set\"]"),
        view.replace(
            "\"name\":\"tool\",\"value\":\"Read\"",
            "\"name\":\"tool\",\"value\":\"/x\"",
        ),
        view.replace("[\"tool\",\"Read\"]", "[\"tool\",\"ns/Read\"]"),
    ];
    for bad in &damaged[..10] {
        assert!(
            matches!(parse_conv(bad.as_bytes()), Err(DocError::Damaged(_))),
            "{bad}"
        );
    }
    for bad in &damaged[10..] {
        assert!(
            matches!(parse_view(bad.as_bytes()), Err(DocError::Damaged(_))),
            "{bad}"
        );
    }
}
