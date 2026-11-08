//! Direct-helper portion of P0-218. Install beside `seatproof_spec.rs` and use
//! wiring.patch. These tests use the existing real-server World fixture.
//! B0 RED means forbidden input or an audit mismatch, never a setup failure.
use super::*;

fn expected(w: &World, launch: &str) -> Runner {
    let mut command = w.command("send", true, false);
    command
        .env("_AE_EXPECT_SLOT", SLOT)
        .env("_AE_EXPECT_LAUNCH", launch);
    command
}

fn delivered(w: &World, output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "control failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait("accepted body at the real agent", || {
        read(&w.out).contains(BODY)
    });
    assert_eq!(w.count("send"), 1, "one delivered audit: {}", w.events());
    assert_eq!(w.count("refused"), 0);
}

fn refused_before_input(w: &World, output: &Output, before: &str, word: &str) {
    // Check the behavioral oracle first: B0 ignores the expectation env and
    // delivers. A missing future API is never the expected failure condition.
    w.untouched(before);
    w.refuse(output, word);
}

// The fixture bodies and addresses contain no escaped quotes. Preserve the
// complete quoted value so missing fields compare as missing, independently
// of timestamps and randomly generated event IDs.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let tail = line.split_once(&format!("\"{key}\":"))?.1;
    let quoted = tail.strip_prefix('"')?;
    let end = quoted.find('"')?;
    Some(&quoted[..end])
}

fn delivered_fields(w: &World) -> Vec<Option<String>> {
    let events = w.events();
    let event = events
        .lines()
        .rev()
        .find(|line| field(line, "action") == Some("send"))
        .expect("delivered audit");
    assert_eq!(field(event, "target"), Some("kid"));
    [
        "actor",
        "action",
        "target",
        "target_slot",
        "target_session",
        "target_session_id",
        "target_launch",
        "kind",
        "summary",
    ]
    .map(|key| field(event, key).map(str::to_owned))
    .to_vec()
}

#[test]
fn r1_expected_launch_refuses_same_name_successor_before_admission() {
    let w = World::new("r218-same-name");
    let mut command = expected(&w, "tok-kid");
    w.change("replace", SLOT);
    let before = w.screen();
    let output = command.output().expect("real helper");
    refused_before_input(&w, &output, &before, "replaced");
}

#[test]
fn r3_expected_slot_refuses_same_name_in_another_slot() {
    let w = World::new("r218-other-slot");
    let mut command = expected(&w, "tok-kid");
    construct_other_slot(&w);
    let before = w.screen();
    let output = command.output().expect("real helper");
    refused_before_input(&w, &output, &before, "replaced");
}

fn construct_other_slot(w: &World) {
    // World::change only removes meta rows; it never ends the pane.
    w.change("remove", SLOT);
    let other = "spawned.1";
    for (key, value) in [
        ("seat", "kid"),
        ("profile", "fake"),
        ("agent_bin", "claude"),
        ("harness_session", "pending"),
        ("launch_id", "tok-other"),
        ("pane", w.pane.as_str()),
    ] {
        w.row(&format!("{key}.{other}"), Some(value));
    }
    w.stamp("@ae_slot", Some(other));
}

#[test]
fn r3_no_expectation_control_delivers_to_the_constructed_new_slot() {
    let w = World::new("r218-other-slot-control");
    construct_other_slot(&w);
    let output = w
        .command("send", true, false)
        .output()
        .expect("real helper");
    delivered(&w, &output);
}

#[test]
fn r4_matching_expectation_keeps_lock_wait_reproof() {
    let w = World::new("r218-lock-wait");
    let held = w.lock();
    let before = w.screen();
    let child = expected(&w, "tok-kid").spawn().expect("real helper");
    wait("SETUP: admitted body before lock-wait replacement", || {
        w.bodies() > 0
    });
    w.change("replace", SLOT);
    drop(held);
    let output = finish(child);
    refused_before_input(&w, &output, &before, "replaced");
    assert!(w.events().contains("tok-kid"), "old launch witness lost");
}

#[test]
fn r5_matching_expectation_preserves_delivery_and_audit_fields() {
    let w = World::new("r218-audit-control");
    let output = w
        .command("send", true, false)
        .output()
        .expect("ordinary helper");
    delivered(&w, &output);
    let ordinary_fields = delivered_fields(&w);
    let output = expected(&w, "tok-kid").output().expect("expected helper");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait("second accepted body in the same World", || {
        read(&w.out).matches(BODY).count() == 2
    });
    assert_eq!(w.count("send"), 2);
    assert_eq!(w.count("refused"), 0);
    assert_eq!(ordinary_fields, delivered_fields(&w));
}

#[test]
fn r6_positive_legacy_absence_delivers_but_a_minted_launch_refuses() {
    for replace in [false, true] {
        let w = World::new(if replace {
            "r218-legacy-replaced"
        } else {
            "r218-legacy-control"
        });
        w.change("legacy", SLOT);
        let mut command = w.command("send", true, false);
        command
            .env("_AE_EXPECT_SLOT", SLOT)
            .env("_AE_EXPECT_LEGACY", "1");
        if replace {
            w.row(&format!("launch_id.{SLOT}"), Some("tok-minted"));
        }
        let before = w.screen();
        let output = command.output().expect("legacy helper");
        if replace {
            refused_before_input(&w, &output, &before, "replaced");
        } else {
            delivered(&w, &output);
        }
    }
}

#[test]
fn r7_incomplete_or_conflicting_expectation_is_unprovable() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("slot-only", &[("_AE_EXPECT_SLOT", SLOT)]),
        ("launch-only", &[("_AE_EXPECT_LAUNCH", "tok-kid")]),
        ("legacy-only", &[("_AE_EXPECT_LEGACY", "1")]),
        (
            "conflicting",
            &[
                ("_AE_EXPECT_SLOT", SLOT),
                ("_AE_EXPECT_LAUNCH", "tok-kid"),
                ("_AE_EXPECT_LEGACY", "1"),
            ],
        ),
        (
            "legacy-value",
            &[("_AE_EXPECT_SLOT", SLOT), ("_AE_EXPECT_LEGACY", "2")],
        ),
        (
            // send_env drops the empty value. This is the slot-only arm
            // reached through that door behavior, not a new wire shape.
            "empty-token",
            &[("_AE_EXPECT_SLOT", SLOT), ("_AE_EXPECT_LAUNCH", "")],
        ),
    ];
    for (name, vars) in cases {
        let w = World::new(&format!("r218-malformed-{name}"));
        let before = w.screen();
        let mut command = w.command("send", true, false);
        for (key, value) in *vars {
            command.env(key, value);
        }
        let output = command.output().expect("real malformed-env helper");
        refused_before_input(&w, &output, &before, "could not be proven");
    }
}

#[test]
fn r6_token_is_an_exact_value_even_if_it_looks_like_a_legacy_sentinel() {
    for (index, token) in ["1", "legacy", "Absent"].into_iter().enumerate() {
        let w = World::new(&format!("r218-literal-token-{index}"));
        w.row(&format!("launch_id.{SLOT}"), Some(token));
        let output = expected(&w, token).output().expect("literal token helper");
        delivered(&w, &output);
    }
}
