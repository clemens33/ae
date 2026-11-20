//! Future-symbol checks only; installed after base-compatible RED capture.
//! Oracle: lead L1-L12. No expected result comes from another decision function.
use super::*;

fn parsed(global: &str, lead: &str) -> Settings {
    settings_in(
        Path::new("acceptance-config"),
        &format!("[workspace]\nauto_reseat = all\nauto_reseat_at = {global}\n{lead}\n"),
    )
}

#[test]
fn leadat_compile_views_preserve_value_provenance_and_raw_settings() {
    for (global, lead, expected) in [
        ("95", "96", Some(96)),
        ("80", "80", Some(80)),
        ("off", "off", None),
        ("off", "50", Some(50)),
        ("95", "100", Some(100)),
        ("80", "off", None),
    ] {
        let raw = parsed(global, &format!("auto_reseat_lead_at = {lead}"));
        let original = raw.clone();
        for slot in ["main", "worker.0"] {
            let view = raw.seat_view(true, slot);
            assert!(matches!(view, std::borrow::Cow::Owned(_)));
            assert_eq!(view.headroom_at, expected);
            assert_eq!(view.headroom_key(), "auto_reseat_lead_at");
            let repeated = view.seat_view(true, slot);
            assert!(matches!(repeated, std::borrow::Cow::Borrowed(_)));
            assert_eq!(repeated.headroom_at, expected);
            assert_eq!(repeated.headroom_key(), "auto_reseat_lead_at");
        }
        assert_eq!(raw, original, "view cannot mutate global parse");
        for (layout, slot) in [
            (false, "main"),
            (false, "worker.0"),
            (true, "worker.1"),
            (true, "spawned.0"),
        ] {
            let view = raw.seat_view(layout, slot);
            assert!(matches!(view, std::borrow::Cow::Borrowed(_)));
            assert_eq!(view.headroom_at, raw.headroom_at);
            assert_eq!(view.headroom_key(), "auto_reseat_at");
        }
    }
}

#[test]
fn leadat_compile_absence_and_bad_values_inherit_even_global_off() {
    for global in ["80", "off", "95"] {
        let raw = parsed(global, "");
        let view = raw.seat_view(true, "main");
        assert!(matches!(view, std::borrow::Cow::Borrowed(_)));
        assert_eq!(view.headroom_at, raw.headroom_at);
        assert_eq!(view.headroom_key(), "auto_reseat_at");
        assert!(raw.notes.is_empty());
        for bad in ["49", "101", "+96", "96.0", "", "wrong", "OFF"] {
            let row = if bad.is_empty() { "\"\"" } else { bad };
            let raw = parsed(global, &format!("auto_reseat_lead_at = {row}"));
            let view = raw.seat_view(true, "main");
            assert_eq!(view.headroom_at, raw.headroom_at);
            assert_eq!(view.headroom_key(), "auto_reseat_at");
            assert_eq!(raw.notes.len(), 1);
            assert_eq!(
                raw.notes[0],
                format!(
                    "auto_reseat_lead_at = {bad:?} is not off or a whole percent from 50 to 100; auto_reseat_at used"
                )
            );
        }
        let raw = parsed(
            global,
            "auto_reseat_lead_at = 96\nauto_reseat_lead_at = off",
        );
        assert_eq!(
            raw.notes,
            ["auto_reseat_lead_at is set 2 times; auto_reseat_at used"]
        );
        assert_eq!(raw.seat_view(true, "worker.0").headroom_at, raw.headroom_at);
        for (row, why) in [
            ("auto_reseat_lead_at", "auto_reseat_lead_at is malformed"),
            (
                "auto_reseat_lead_at =",
                "auto_reseat_lead_at has an invalid value",
            ),
        ] {
            let raw = parsed(global, row);
            assert_eq!(raw.seat_view(true, "main").headroom_at, raw.headroom_at);
            assert_eq!(raw.seat_view(true, "main").headroom_key(), "auto_reseat_at");
            assert_eq!(raw.notes, [format!("{why}; auto_reseat_at used")]);
        }
    }
    let raw = settings_in(
        Path::new("acceptance-config"),
        "[workspace]\nauto_reseat = all\nauto_reseat_lead_at = bad\n",
    );
    assert_eq!(raw.seat_view(true, "main").headroom_at, Some(95));
    assert_eq!(raw.seat_view(true, "main").headroom_key(), "auto_reseat_at");
}

#[test]
fn leadat_compile_shared_percent_preserves_unreadable_errors() {
    let key = "auto_reseat_lead_at";
    assert_eq!(
        parse_percent(key, Err("unreadable".to_owned()), Ok(1)),
        Err("unreadable".to_owned())
    );
    assert_eq!(
        parse_percent(key, Ok(Some("96".to_owned())), Err("unlisted".to_owned())),
        Err("unlisted".to_owned())
    );
    assert_eq!(
        parse_percent(key, Ok(Some(" 096 ".to_owned())), Ok(1)),
        Ok(Some(96))
    );
}

#[test]
fn leadat_compile_old_notes_and_switch_off_are_unchanged() {
    let raw = parsed("bad", "auto_reseat_lead_at = bad");
    assert_eq!(raw.notes.len(), 2);
    assert!(
        raw.notes.iter().any(|s| s
            == "auto_reseat_at = \"bad\" is not off or a whole percent from 50 to 100; 95 used")
    );
    assert!(
        raw.notes
            .iter()
            .any(|s| s.ends_with("; auto_reseat_at used"))
    );
    let off = settings_in(
        Path::new("acceptance-config"),
        "[workspace]\nauto_reseat = off\nauto_reseat_at = bad\nauto_reseat_lead_at = bad\n",
    );
    assert!(off.notes.is_empty());
    assert_eq!(off.seat_view(true, "main").headroom_at, None);
    assert_eq!(off.seat_view(true, "main").headroom_key(), "auto_reseat_at");
}

#[test]
fn leadat_compile_partner_uses_same_lead_pair_classifier() {
    let meta = crate::meta::Meta::parse("schema=2\nseat.main=lead\nseat.worker.0=colead\n");
    let roster = meta.roster();
    for (layout, slot, expected) in [
        (true, "main", true),
        (true, "worker.0", true),
        (true, "worker.1", false),
        (true, "spawned.0", false),
        (false, "main", false),
        (false, "worker.0", false),
    ] {
        assert_eq!(lead_seat(layout, slot), expected);
        assert_eq!(
            matches!(partner(roster, layout, slot), Link::Partner(_)),
            expected
        );
    }
}
