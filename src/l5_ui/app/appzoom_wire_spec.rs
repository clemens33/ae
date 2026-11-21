//! Canonical appzoom handover wire. This covers the public parser and writer
//! without depending on the in-memory representation of zoom.

use super::parse;

const VIEW: &str =
    "%7|2099.1.1|1760259000|web|6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11|agents|colead|31|9|2|0|1|4";
const ORCH: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";

fn canonical(text: &str) {
    let handover = parse(text).unwrap_or_else(|| panic!("refused canonical wire: {text}"));
    assert_eq!(handover.render(), text);
}

#[test]
fn v3_carries_main_zoom_with_any_panel_mode_or_none() {
    for width in ["", "56"] {
        let main = format!("v3|{VIEW}|{width}");
        canonical(&main);
        for mode in ["side", "side-main", "full"] {
            canonical(&format!("{main}|{mode},{ORCH},17,3"));
        }
    }
}

#[test]
fn v3_refuses_alternate_spellings_and_hostile_fields() {
    let main = format!("v3|{VIEW}|");
    let panel = format!("{main}|full,{ORCH},17,3");
    for good in [&main, &panel] {
        canonical(good);
        let fields: Vec<&str> = good.split('|').collect();
        for index in 0..fields.len() {
            for replacement in ["x y", "-1", "\u{1b}[2J"] {
                let mut replaced = fields.clone();
                replaced[index] = replacement;
                let bad = replaced.join("|");
                assert!(parse(&bad).is_none(), "accepted bad field {index}: {bad:?}");
            }
        }
        assert!(parse(&good.replace("|agents|", "|overview|")).is_none());
        assert!(parse(&format!("{good}|x")).is_none());
    }
    let main_id = "6f1c2b7e-0d3a-4c58-9a2e-5b1d7c9e0a11";
    for bad in [
        format!("v3|{VIEW}"),
        panel.replace(main_id, &main_id.to_ascii_uppercase()),
        panel.replace(ORCH, &ORCH.to_ascii_uppercase()),
        format!("{main}|"),
        format!("{main}|wide,{ORCH},17,3"),
        format!("{main}|full,{ORCH},17"),
        format!("{main}|full,{ORCH},17,3,0"),
        format!("{main}|full,{ORCH},017,3"),
        format!("{main}|full,{ORCH},1000001,3"),
        format!("{main}|full,{ORCH},17,65536"),
    ] {
        assert!(parse(&bad).is_none(), "accepted alternate wire: {bad:?}");
    }
}

#[test]
fn legacy_wires_keep_their_canonical_bytes() {
    canonical(&format!("v1|{VIEW}"));
    canonical(&format!("v1|{VIEW}|56"));
    for width in ["", "56"] {
        for mode in ["side", "side-main", "full"] {
            canonical(&format!("v2|{VIEW}|{width}|{mode},{ORCH},17,3"));
        }
    }
}
