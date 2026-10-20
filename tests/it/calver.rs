//! The `CalVer` neighbours of the build under test.
//!
//! A spec that repoints the command link at "another core" must not name a
//! literal for it: a literal that equals the running version is no other core
//! at all, because `app::upgrade::seen` answers nothing on an equal link. Both
//! targets are derived from the version the binary reports.
//!
//! The domain is a released ae `CalVer`: a calendar year of 2 or more and a
//! patch below `u64::MAX`. That bound is this helper's own policy, not a limit
//! of `CalVer`; outside it the helper is unsupported and panics at once.

#![allow(
    clippy::expect_used,
    reason = "a version the product cannot carry is a broken fixture, said at once"
)]

/// The `year.month.patch` of `version`, each a plain number.
fn parts(version: &str) -> [u64; 3] {
    assert!(ae::install::is_version(version), "not a CalVer: {version}");
    let mut numbers = version
        .split('.')
        .map(|part| part.parse::<u64>().expect("a number that fits"));
    std::array::from_fn(|_| numbers.next().expect("three parts"))
}

/// A version strictly newer than `version`: the next release of its month.
pub fn after(version: &str) -> String {
    let [year, month, patch] = parts(version);
    let next = patch.checked_add(1).expect("a patch below u64::MAX");
    format!("{year}.{month}.{next}")
}

/// A version strictly older than `version`: the same release, a year earlier.
/// The year is the one part that never has to borrow, so this holds for a
/// patch of 0 or 1 and for the first month of a year alike.
pub fn before(version: &str) -> String {
    let [year, month, patch] = parts(version);
    let last = (year.checked_sub(1)).filter(|year| *year > 0);
    let last = last.expect("a calendar year of 2 or more");
    format!("{last}.{month}.{patch}")
}

/// A core newer than this build.
pub fn newer() -> String {
    after(ae::VERSION)
}

/// A core older than this build.
pub fn older() -> String {
    before(ae::VERSION)
}

#[test]
fn a_neighbour_orders_against_every_released_version() {
    use std::cmp::Ordering::{Greater, Less};
    for base in [
        "2026.10.99",
        "2026.10.100",
        "2026.10.1",
        "2026.10.0",
        "2026.1.1",
        "2026.12.99",
        "2027.1.1",
        "2099.12.9",
        "2.1.1",
        "2026.10.18446744073709551614",
        ae::VERSION,
    ] {
        let (newer, older) = (after(base), before(base));
        for neighbour in [&newer, &older] {
            assert!(ae::install::is_version(neighbour), "{base}: {neighbour}");
        }
        let judged = ae::install::compare_versions;
        assert_eq!(judged(&newer, base), Some(Greater), "{base}: {newer}");
        assert_eq!(judged(&older, base), Some(Less), "{base}: {older}");
    }
}

#[test]
fn the_neighbours_are_spelled_by_their_parts_not_by_a_literal() {
    assert_eq!(after("2026.10.99"), "2026.10.100");
    assert_eq!(after("2026.10.9"), "2026.10.10");
    assert_eq!(before("2026.10.99"), "2025.10.99");
    assert_eq!(before("2027.1.1"), "2026.1.1");
    assert_eq!(before("2.1.1"), "1.1.1");
    assert_eq!(
        after("2026.10.18446744073709551614"),
        "2026.10.18446744073709551615"
    );
    assert_ne!(newer(), ae::VERSION);
    assert_ne!(older(), ae::VERSION);
}

#[test]
#[should_panic(expected = "a patch below u64::MAX")]
fn no_newer_neighbour_exists_past_the_last_patch() {
    after("2026.10.18446744073709551615");
}

#[test]
#[should_panic(expected = "a calendar year of 2 or more")]
fn no_older_neighbour_exists_before_the_first_year() {
    before("1.1.1");
}
