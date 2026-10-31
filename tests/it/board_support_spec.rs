// Frozen by r3a-nav against the public board surface, before BoardSpec exists.
// Install as tests/it/board_support_spec.rs; append include!("board_support_spec.rs");
// to tests/it/board.rs. Existing fixture constructors are setup, never the oracle.

fn r3_observe(root: &Path, home: Option<&Path>, replies: board::Replies) -> board::Observation {
    let sessions = [SessionInput {
        name: "matrix".to_owned(),
        path: root.join("sessions/matrix"),
    }];
    board::observe(
        &Inputs {
            home,
            sessions: &sessions,
            replies,
        },
        None,
    )
}

fn r3_reasons(observed: &board::Observation) -> Vec<&str> {
    observed
        .coverage
        .iter()
        .map(|item| item.reason.as_str())
        .collect()
}

fn r3_roster(bin: &str, id: Option<&str>, config: &str) -> String {
    format!(
        "seat.main=lead\nagent_bin.main={bin}\n{}{config}",
        id.map_or(String::new(), |id| format!("harness_session.main={id}\n"))
    )
}

#[test]
fn r3_support_and_id_refusals_precede_store_work() {
    // Mutation caught: supported/unsupported row drift, ID validation after
    // locator/supplemental work, or unsupported falling through to Claude.
    let root = rig("r3-id-matrix");
    let poison = root.join("not-a-directory");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(&poison, "poison root").expect("poison");
    let upper = CLAUDE_ID.to_ascii_uppercase();
    for bin in [
        "claude",
        "codex",
        "grok",
        "muse",
        "agy",
        "opencode",
        "gemini",
        "unrecognized",
    ] {
        for id in [
            None,
            Some("../escape"),
            Some(upper.as_str()),
            Some("--continue"),
        ] {
            plant_session(
                &root,
                "matrix",
                &r3_roster(bin, id, "config_home.main=unknown\n"),
            );
            let observed = r3_observe(&root, Some(&poison), board::Replies::All);
            assert!(
                observed.rows.is_empty() && observed.seeds.is_empty(),
                "{bin} {id:?}"
            );
            let expected = match bin {
                "gemini" => "gemini: out of scope",
                "unrecognized" => "unknown tool: out of scope",
                _ => "invalid or missing conversation id",
            };
            assert_eq!(r3_reasons(&observed), [expected], "{bin} {id:?}");
        }
    }
    for bin in [
        "claude",
        "codex",
        "grok",
        "muse",
        "agy",
        "gemini",
        "unrecognized",
    ] {
        plant_session(&root, "matrix", &r3_roster(bin, Some(CLAUDE_ID), ""));
        let observed = r3_observe(&root, None, board::Replies::All);
        let expected = match bin {
            "gemini" => "gemini: out of scope",
            "unrecognized" => "unknown tool: out of scope",
            _ => "legacy config home unavailable",
        };
        assert_eq!(r3_reasons(&observed), [expected], "{bin}, no HOME");
        assert!(observed.rows.is_empty() && observed.seeds.is_empty());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one config-provenance matrix keeps all planted decoys and independently expected outcomes visible together"
)]
fn r3_recorded_and_legacy_config_home_matrix() {
    use std::os::unix::fs::PermissionsExt as _;

    // Mutation caught: using HOME instead of recorded store, treating implicit
    // base as store, accepting absent/unknown/inconsistent provenance, or
    // moving ID validation after recorded-home resolution.
    for bin in ["claude", "codex"] {
        let root = rig(&format!("r3-home-{bin}"));
        let explicit = root.join("explicit");
        let base = root.join("implicit-base");
        let implicit = base.join(if bin == "claude" { ".claude" } else { ".codex" });
        let legacy = root.join(if bin == "claude" { ".claude" } else { ".codex" });
        let id = if bin == "claude" { CLAUDE_ID } else { CODEX_ID };
        for (store, body) in [
            (&explicit, "explicit source"),
            (&implicit, "implicit source"),
            (&legacy, "legacy source"),
        ] {
            if bin == "claude" {
                plant_transcript(store, "work", id, &[user("2026-09-16T09:00:00Z", body)]);
            } else {
                plant_rollout(store, &[codex_user("2026-09-16T09:00:00Z", body)]);
            }
        }
        let configs = [
            (
                format!("config_home.main={}\n", explicit.display()),
                Some("explicit source"),
            ),
            (
                format!(
                    "config_home.main=implicit:{}\nconfig_home_base.main={}\n",
                    implicit.display(),
                    base.display()
                ),
                Some("implicit source"),
            ),
            (String::new(), Some("legacy source")),
            ("config_home.main=absent\n".to_owned(), None),
            ("config_home.main=unknown\n".to_owned(), None),
            ("config_home.main=relative\n".to_owned(), None),
            (
                format!("config_home.main=implicit:{}\n", implicit.display()),
                None,
            ),
            (
                format!(
                    "config_home.main={}\nconfig_home_base.main={}\n",
                    explicit.display(),
                    base.display()
                ),
                None,
            ),
        ];
        for (config, body) in configs {
            plant_session(&root, "matrix", &r3_roster(bin, Some(id), &config));
            let observed = r3_observe(&root, Some(&root), board::Replies::Off);
            if let Some(body) = body {
                assert!(
                    observed.coverage.is_empty(),
                    "{bin} {config}: {:?}",
                    r3_reasons(&observed)
                );
                assert_eq!(observed.rows.len(), 1, "{bin} {config}");
                assert_eq!(observed.rows[0].body, body, "{bin} {config}");
                assert_eq!(observed.rows[0].role, board::Role::Human);
                assert_eq!(observed.seeds.len(), 1, "current file source seeds");
            } else {
                assert!(observed.rows.is_empty() && observed.seeds.is_empty());
                assert_eq!(
                    r3_reasons(&observed),
                    ["recorded config home is absent, unknown, invalid, or inconsistent"],
                    "{bin} {config}"
                );
            }
        }
        // A typed but absent store must not fall back to the populated legacy store.
        let absent = root.join("missing-store");
        plant_session(
            &root,
            "matrix",
            &r3_roster(
                bin,
                Some(id),
                &format!("config_home.main={}\n", absent.display()),
            ),
        );
        let observed = r3_observe(&root, Some(&root), board::Replies::Off);
        assert!(observed.rows.is_empty() && observed.seeds.is_empty());
        assert_eq!(
            r3_reasons(&observed),
            [if bin == "claude" {
                "Claude store unreadable"
            } else {
                "rollout not found"
            }]
        );
        // Refuse an unreadable selected stream without reading a legacy decoy.
        let path = if bin == "claude" {
            explicit.join(format!("projects/work/{id}.jsonl"))
        } else {
            explicit.join(format!(
                "sessions/2026/09/08/rollout-2026-09-08T09-00-00-{id}.jsonl"
            ))
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .expect("unreadable stream");
        plant_session(
            &root,
            "matrix",
            &r3_roster(
                bin,
                Some(id),
                &format!("config_home.main={}\n", explicit.display()),
            ),
        );
        let observed = r3_observe(&root, Some(&root), board::Replies::Off);
        assert!(observed.rows.is_empty() && observed.seeds.is_empty());
        assert_eq!(r3_reasons(&observed), ["transcript unreadable"], "{bin}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("restore stream");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn r3_fixed_home_sources_preserve_config_indifference_and_filtering() {
    // Mutation caught: generalized config-root policy changing current fixed
    // stores, usage support accidentally deciding board support, lost parser
    // selection, assistant gate drift, or missing file follow seed.
    for (bin, id, human, reply) in [
        ("grok", GROK_ID, "grok human", "grok reply"),
        ("muse", MUSE_ID, "muse human", "muse reply"),
        ("agy", AGY_ID, "agy human", "agy reply"),
    ] {
        let root = rig(&format!("r3-fixed-{bin}"));
        match bin {
            "grok" => plant_grok(
                &root,
                "work",
                id,
                &[
                    grok_user(human),
                    grok_agent(reply),
                    grok_bare("turn_completed"),
                ],
            ),
            "muse" => plant_muse(
                &root,
                "2026/09/16",
                id,
                &[muse_user(human), muse_comm(reply)],
            ),
            _ => {
                let dir = root.join(".gemini/antigravity-cli");
                std::fs::create_dir_all(&dir).expect("agy dir");
                std::fs::write(
                    dir.join("history.jsonl"),
                    agy_record(Some(id), human, 1_789_549_200_500) + "\n",
                )
                .expect("history");
                plant_agy_transcript(&root, id, "transcript_full.jsonl", &[agy_tr(1, reply)]);
            }
        }
        let decoy = root.join("unrelated-config");
        for config in [
            String::new(),
            "config_home.main=absent\n".to_owned(),
            "config_home.main=unknown\n".to_owned(),
            format!("config_home.main={}\n", decoy.display()),
            format!(
                "config_home.main=implicit:{}\nconfig_home_base.main={}\n",
                decoy.display(),
                root.display()
            ),
        ] {
            plant_session(&root, "matrix", &r3_roster(bin, Some(id), &config));
            for replies in [board::Replies::Off, board::Replies::All] {
                let observed = r3_observe(&root, Some(&root), replies);
                assert!(
                    observed.coverage.is_empty(),
                    "{bin} {config}: {:?}",
                    r3_reasons(&observed)
                );
                let humans: Vec<_> = observed
                    .rows
                    .iter()
                    .filter(|row| row.role == board::Role::Human)
                    .map(|row| row.body.as_str())
                    .collect();
                let assistants: Vec<_> = observed
                    .rows
                    .iter()
                    .filter(|row| row.role == board::Role::Assistant)
                    .map(|row| row.body.as_str())
                    .collect();
                assert_eq!(humans, [human], "{bin} {config}");
                assert_eq!(
                    assistants,
                    if replies == board::Replies::All {
                        vec![reply]
                    } else {
                        vec![]
                    },
                    "{bin} {config}"
                );
                assert_eq!(observed.seeds.len(), 1, "{bin}: current stream seeds");
                assert!(observed.rows.iter().all(|row| row.generation == 0));
            }
            let absent = r3_observe(&root, None, board::Replies::All);
            assert_eq!(
                r3_reasons(&absent),
                ["legacy config home unavailable"],
                "{bin} {config}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn r3_agy_id_and_primary_locate_precede_supplemental_rows() {
    // Mutation caught: supplemental read before conversation validation or
    // before primary locate; supplemental rows from an unlocated generation.
    let root = rig("r3-agy-order");
    plant_agy_transcript(
        &root,
        AGY_ID,
        "transcript_full.jsonl",
        &[agy_tr(1, "must stay hidden")],
    );
    plant_session(&root, "matrix", &r3_roster("agy", Some(AGY_ID), ""));
    let absent = r3_observe(&root, Some(&root), board::Replies::All);
    assert!(absent.rows.is_empty() && absent.seeds.is_empty());
    assert_eq!(r3_reasons(&absent), ["transcript not found"]);
    let history = root.join(".gemini/antigravity-cli/history.jsonl");
    std::fs::write(
        &history,
        agy_record(Some(AGY_ID), "primary row", 1_789_549_200_500) + "\n",
    )
    .expect("history");
    plant_session(&root, "matrix", &r3_roster("agy", Some("../escape"), ""));
    let invalid = r3_observe(&root, Some(&root), board::Replies::All);
    assert!(invalid.rows.is_empty() && invalid.seeds.is_empty());
    assert_eq!(r3_reasons(&invalid), ["invalid or missing conversation id"]);
    let _ = std::fs::remove_dir_all(&root);
}
