//! Independent #270 resource/cache/origin oracle; approved plan-r3 + lead caps.
use super::lanemd::{Cache, turn};
use super::markdown::{self, Budget, Marks, Opts, Rendered};
use std::{cell::RefCell, mem::size_of, rc::Rc};

fn texts(r: &Rendered) -> Vec<&str> {
    (0..r.rows()).map(|n| r.text(n).unwrap()).collect()
}
fn code(n: usize) -> String {
    format!("```\n{}```", "a\n".repeat(n))
}

#[test]
fn resource_boundaries_admit_exact_caps_and_reject_one_more() {
    let mut c = Cache::default();
    assert!(c.render(&"x".repeat(65536), 4096).is_some());
    let before = c.test_stats();
    assert!(c.render(&"x".repeat(65537), 4096).is_none());
    assert_eq!(
        c.test_stats(),
        before,
        "source refusal occurs before parsing"
    );
    assert_eq!(c.render(&code(2000), 20).unwrap().rows(), 2000);
    assert!(c.render(&code(2001), 20).is_none());
    assert_eq!(c.render(&code(1000), 200).unwrap().cells(), 200_000);
    assert!(
        c.render(&code(1000), 201).is_none(),
        "code padding counts against cell budget"
    );
    assert!(c.render("**ok**", 19).is_none());
    assert_eq!(texts(&c.render("**ok**", 20).unwrap()), ["ok"]);
    assert_eq!(c.render("", 20).unwrap().rows(), 0);
    assert!(
        c.render("[r]: src/a.rs\n", 20).is_none(),
        "nonblank zero-row input uses plain fallback"
    );
}

#[test]
fn turn_aggregates_parts_and_never_returns_a_partially_rendered_turn() {
    let c = RefCell::new(Cache::default());
    let half = "x".repeat(32768);
    assert!(turn(&c, &[(&half, 4096), (&half, 4096)]).is_some());
    let larger = "x".repeat(32769);
    let before = c.borrow().test_stats();
    assert!(turn(&c, &[(&half, 4096), (&larger, 4096)]).is_none());
    assert_eq!(
        c.borrow().test_stats(),
        before,
        "aggregate source precheck parses nothing"
    );
    let rows = code(1000);
    assert!(turn(&c, &[(&rows, 20), (&rows, 20)]).is_some());
    assert!(turn(&c, &[(&code(1001), 20), (&rows, 20)]).is_none());
    let cells = code(500);
    assert!(turn(&c, &[(&cells, 200), (&cells, 200)]).is_some());
    assert!(turn(&c, &[(&cells, 200), (&cells, 201)]).is_none());
    for last in [("**bad**", 19), ("[r]: src/a.rs\n", 80)] {
        assert!(
            turn(&c, &[("**good**", 80), last]).is_none(),
            "any failed part rejects the turn"
        );
    }
}

#[test]
fn cache_hits_reuse_rows_width_and_exact_text_misses_never_leak_old_words() {
    let mut c = Cache::default();
    c.test_key_with(|_| 0);
    let a = c.render("**first**", 40).unwrap();
    let first = c.test_stats();
    let again = c.render("**first**", 40).unwrap();
    assert!(
        Rc::ptr_eq(&a, &again),
        "same text+width reuses semantic rows"
    );
    assert_eq!(c.test_stats(), first, "hit does not parse or lay out");
    let b = c.render("**second**", 40).unwrap();
    assert_eq!(
        texts(&b),
        ["second"],
        "forced key collision verifies exact text"
    );
    assert_eq!(c.test_stats().2, first.2 + 1);
    let _ = c.render("**second**", 41).unwrap();
    assert_eq!(c.test_stats().3, first.3 + 2, "new width lays out new rows");
    let _ = c.render("**first**", 40).unwrap();
    assert_eq!(
        c.test_stats().2,
        first.2 + 3,
        "collision did not return another body's rows"
    );
}

#[test]
fn parsed_refusals_are_cached_without_reparsing_each_frame() {
    let mut c = Cache::default();
    let body = code(2001);
    assert!(c.render(&body, 40).is_none());
    let before = c.test_stats();
    assert!(before.2 > 0 && before.3 > 0);
    assert!(c.render(&body, 40).is_none());
    assert_eq!(c.test_stats(), before);
}

#[test]
fn cache_enforces_count_and_lru_bounds() {
    let mut c = Cache::default();
    for n in 0..1024 {
        let _ = c.render(&format!("word{n}"), 40).unwrap();
    }
    assert_eq!(c.test_stats().0, 1024);
    let _ = c.render("word0", 40).unwrap();
    let _ = c.render("overflow", 40).unwrap();
    assert_eq!(c.test_stats().0, 1024);
    let before = c.test_stats();
    let _ = c.render("word0", 40).unwrap();
    assert_eq!(
        c.test_stats(),
        before,
        "recently used entry survives eviction"
    );
    let _ = c.render("word1", 40).unwrap();
    assert_eq!(
        c.test_stats().2,
        before.2 + 1,
        "oldest untouched entry was evicted"
    );
}

#[test]
fn marks_heavy_entries_charge_metadata_and_eviction_obeys_real_lower_bound() {
    let mut c = Cache::default();
    let body = "*a* `b` ".repeat(1200);
    let r = c.render(&body, 4096).unwrap();
    let lower = body.len() + 4800 * size_of::<(usize, Marks)>();
    assert!(
        r.retained_bytes() >= 4800 * size_of::<(usize, Marks)>(),
        "four style transitions per group are owned storage"
    );
    assert!(
        c.test_stats().1 >= lower,
        "cache charge includes source + mark transitions"
    );
    for n in 0..100 {
        let _ = c.render(&format!("{body} suffix{n}"), 4096).unwrap();
    }
    let (entries, bytes, _, _) = c.test_stats();
    assert!(bytes <= 8 * 1024 * 1024);
    assert!(
        entries <= 8 * 1024 * 1024 / lower,
        "count independently proves metadata forced eviction: {entries}"
    );
}

#[test]
fn cache_charges_retained_map_storage_after_many_small_entries_are_evicted() {
    let mut c = Cache::default();
    for n in 0..1024 {
        let _ = c.render(&format!("small{n}"), 40).unwrap();
    }
    let body = "*a* `b` ".repeat(7000);
    let mut large = Vec::new();
    for n in 0..32 {
        let source = format!("{body} suffix{n:04}");
        let _ = c.render(&source, 4096).unwrap();
        large.push(source);
    }
    let (kept, charged, parses, _) = c.test_stats();
    assert!(
        kept > 0 && kept < large.len(),
        "large entries force eviction"
    );
    let mut owned = 0;
    for source in large.iter().rev().take(kept) {
        let r = c.render(source, 4096).unwrap();
        owned += source.len() + r.retained_bytes() + 2 * size_of::<usize>();
    }
    assert_eq!(
        c.test_stats().2,
        parses,
        "the surviving entries are all recent large entries"
    );
    // Independent lower bound: each reserved map slot holds at least a key,
    // an owned source pointer and an optional rendered pointer. This permits
    // either shrinking the map or charging its retained allocation.
    let slot =
        size_of::<(u64, usize)>() + size_of::<Box<str>>() + size_of::<Option<Rc<Rendered>>>();
    let lower = owned + c.test_map_capacity() * slot;
    assert!(
        charged >= lower,
        "reported {charged}, retained payload alone needs {lower}"
    );
    assert!(charged <= 8 * 1024 * 1024);
}

#[test]
fn origin_cuts_cover_tables_gaps_code_padding_and_raw_notes_without_losing_text() {
    let sources = [
        "# head\n\n```\na\n\nb\n```\n\n| a | b |\n|---|---|\n| src/x.rs | z |\n",
        "> > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > raw\n",
    ];
    for source in sources {
        let r = markdown::render_with(
            source,
            40,
            &Budget::LANE,
            Opts {
                soft_break_row: true,
            },
        );
        assert!(r.refusal().is_none());
        for n in 0..r.rows() {
            let text = r.text(n).unwrap();
            if let Some(cut) = r.cut(n) {
                let piece = &r.lines()[cut.line][cut.from..cut.from + cut.len];
                let actual = text.get(cut.gap..).unwrap_or("");
                assert_eq!(actual.trim_end_matches(' '), piece.trim_end_matches(' '));
            } else {
                assert!(
                    text.trim().is_empty(),
                    "only synthetic gap rows have no origin"
                );
            }
            let runs = r.runs(n);
            assert!(runs.windows(2).all(|p| p[0].0 <= p[1].0));
            for (at, _) in text.char_indices() {
                let expected = runs
                    .iter()
                    .rev()
                    .find(|(start, _)| *start <= at)
                    .map_or(Marks::default(), |(_, m)| *m);
                assert_eq!(
                    r.marks(n, at),
                    expected,
                    "iterator exposes exactly the existing semantic marks"
                );
            }
        }
    }
}

#[test]
fn lane_soft_break_option_does_not_change_reader_default() {
    for source in ["alpha\nbeta", "> alpha\n> beta", "- alpha\n  beta"] {
        let reader = markdown::render_with(source, 80, &Budget::READER, Opts::default());
        let lane = markdown::render_with(
            source,
            80,
            &Budget::LANE,
            Opts {
                soft_break_row: true,
            },
        );
        assert!(texts(&reader).iter().any(|t| t.contains("alpha beta")));
        let rows = texts(&lane);
        let a = rows.iter().position(|t| t.contains("alpha")).unwrap();
        let b = rows.iter().position(|t| t.contains("beta")).unwrap();
        assert!(a < b, "lane preserves report rows: {source}");
    }
}
