//! Independent first-sight forecast acceptance: brief + lead R1-R7 are the oracle.
//! Exercise the real cycle reader and resolved commands, with no decision mocks.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixture reads and removes only its own scratch directory"
)]

use super::{Cycle, Knobs, QuotaCarry, SendHelper};
use crate::inventory::ServerId;
use crate::meta::{RecordedClient, RecordedConfigHome, RecordedConfigHomeBase, RosterEntry};

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae-pairguard-{tag}-{}", std::process::id()));
        std::fs::create_dir(&dir).expect("new private scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove private scratch");
    }
}

fn forecast(
    tag: &str,
    peer_tool: &str,
    first_tool: &str,
    good_tool: &str,
    paired: bool,
    switch: &str,
) -> Option<String> {
    let scratch = Scratch::new(tag);
    let config = scratch.0.join("config");
    let good_home = if good_tool == "claude" {
        format!(
            "CLAUDE_CONFIG_DIR={} ",
            scratch.0.join("good-home").display()
        )
    } else {
        String::new()
    };
    let text = format!(
        "[profiles]\nsource = CLAUDE_CONFIG_DIR={} claude --pg source\npeer = {peer_tool} --pg peer\nfirst = {first_tool} --pg first\ngood = {good_home}{good_tool} --pg good\n[workspace]\nauto_reseat = {switch}\n[auto_reseat]\nsource = first, good\n",
        scratch.0.join("source-home").display(),
    );
    std::fs::write(&config, &text).expect("private config");
    let name = if paired { "slice-nav" } else { "solo" };
    std::fs::write(
        scratch.0.join("meta"),
        format!(
            "session=demo\nconfig={}\nseat.spawned.0={name}\nprofile.spawned.0=source\nagent_bin.spawned.0=claude\nseat.spawned.1=slice\nprofile.spawned.1=peer\nagent_bin.spawned.1={peer_tool}\n",
            config.display()
        ),
    ).expect("private meta");
    let entry = |slot: &str, name: &str, profile: &str, binary: &str| RosterEntry {
        slot: slot.to_owned(),
        name: name.to_owned(),
        profile: Some(profile.to_owned()),
        client: RecordedClient::Missing,
        harness_session: None,
        config_home: RecordedConfigHome::Missing,
        config_home_base: RecordedConfigHomeBase::Missing,
        binary: Some(binary.to_owned()),
        work_dir: crate::meta::RecordedWorkDir::Missing,
    };
    let helper = SendHelper::for_session(&scratch.0);
    let server = ServerId::Ambient;
    let cycle = Cycle {
        knobs: Knobs::default(),
        meta_dir: &scratch.0,
        helper: &helper,
        server: &server,
        session: "demo",
        goal: None,
        roster: vec![
            entry("spawned.0", name, "source", "claude"),
            entry("spawned.1", "slice", "peer", peer_tool),
        ],
        local_config: None,
        lead_pair: false,
        fleet_order: crate::theme::FleetOrder::EMPTY,
        auto: crate::autoreseat::settings_in(&config, &text),
        auto_trust: crate::config::AutoTrust::Off,
        meta_agent: false,
        launch_ids: Vec::new(),
        started: None,
        settled: None,
    };
    cycle.auto_deadline(&QuotaCarry::default(), &[], ("spawned.0", name), 1_000)
}

#[test]
fn first_sight_forecasts_a_hold_when_all_destinations_collapse() {
    assert_eq!(
        forecast("hold", "codex", "codex", "codex", true, "on"),
        Some("ae cannot move slice-nav: no usable candidate: first (partner's provider), good (partner's provider)".to_owned())
    );
}

#[test]
fn first_sight_forecasts_the_diverse_destination() {
    assert_eq!(
        forecast("diverse", "codex", "codex", "claude", true, "on"),
        Some("ae will move slice-nav to good in 10m".to_owned())
    );
}

#[test]
fn first_sight_does_not_claim_diversity_from_a_mutable_partner() {
    assert_eq!(
        forecast("unknown", "opencode", "claude", "claude", true, "on"),
        Some("ae cannot move slice-nav: no usable candidate: first (provider unproven), good (provider unproven)".to_owned())
    );
}

#[test]
fn unpaired_and_disabled_forecasts_keep_the_base_contract() {
    assert_eq!(
        forecast("solo", "claude", "claude", "claude", false, "on"),
        Some("ae will move solo to first in 10m".to_owned())
    );
    assert_eq!(
        forecast("disabled", "codex", "codex", "claude", true, "off"),
        None
    );
}

#[test]
fn an_already_collapsed_pair_still_prefers_a_diverse_destination() {
    assert_eq!(
        forecast("prefer-diverse", "claude", "claude", "codex", true, "on"),
        Some("ae will move slice-nav to good in 10m".to_owned())
    );
}
