//! Identity overlay characterisation on f2b775d6, before extraction.
//!
//! Oracles: config.rs reader, `key_claim` and `entry_value` doc comments; contract.md
//! Config; getting-started/config.md prompt blocks. Undocumented edge acceptance,
//! refusal precedence and exact diagnostics below are labelled behaviour pins.
//! No helper API or intermediate state after a refusal is part of this spec.

use super::*;

fn source() -> PathBuf {
    PathBuf::from("<config>")
}

fn duplicate(section: &str, key: &str, line: usize) -> ConfigError {
    ConfigError::DuplicateKey {
        file: source(),
        section: section.to_owned(),
        key: key.to_owned(),
        line,
    }
}

fn roster_key(key: &str, line: usize) -> ConfigError {
    ConfigError::RosterKey {
        file: source(),
        key: key.to_owned(),
        line,
    }
}

fn client_key(key: &str, line: usize) -> ConfigError {
    ConfigError::ClientKey {
        file: source(),
        key: key.to_owned(),
        line,
    }
}

fn client_entry(client: &str, line: usize, reason: &str) -> ConfigError {
    ConfigError::ClientEntry {
        file: source(),
        client: client.to_owned(),
        line,
        reason: reason.to_owned(),
    }
}

// Behaviour pin: refusal priority follows input order, including invalid values
// on the first claim and duplicate claims whose second value is itself invalid.
#[test]
fn first_refusal_follows_line_order_and_raw_claim_precedence() {
    let relative = "offending word './x': executable must be a basename or absolute path";
    let reserved = "offending word 'env': the label is reserved prefix syntax";
    for (text, expected) in [
        (
            "[roster]\nno:colons =\n[agents]\n",
            roster_key("no:colons", 2),
        ),
        (
            "[agents]\n[profiles]\na = x\na = y\n",
            ConfigError::LegacyAgents {
                file: source(),
                line: 1,
            },
        ),
        (
            "[profiles]\na = x\na = y\n[agents]\n",
            duplicate("profiles", "a", 3),
        ),
        ("[clients]\ncc =\ncc = ./x\n", duplicate("clients", "cc", 3)),
        (
            "[clients]\ncc = ./x\ncc = codex\n",
            client_entry("cc", 2, relative),
        ),
        (
            "[clients]\nenv =\nenv = ./x\n",
            client_entry("env", 2, reserved),
        ),
        ("[clients]\n_bad = ./x\n", client_key("_bad", 2)),
        (
            "[profiles]\nx = \"\"\"\n[agents]\n",
            ConfigError::InstructionBlock {
                file: source(),
                key: "profiles.x".to_owned(),
                line: 2,
            },
        ),
    ] {
        assert_eq!(parse_identity(text).unwrap_err(), expected, "{text:?}");
    }
}

// Behaviour pin: headers are case-sensitive and must occupy the trimmed line.
#[test]
fn legacy_and_malformed_headers_keep_the_existing_section_rules() {
    for (text, line) in [("[agents]\n", 1), ("\n[profiles]\na = x\n [agents] \n", 4)] {
        assert_eq!(
            parse_identity(text).unwrap_err(),
            ConfigError::LegacyAgents {
                file: source(),
                line
            }
        );
    }
    for text in [
        "[Agents]\nno:colons = x\nno:colons = y\n",
        "[profiles]\n[agents] # comment\n",
        "[profiles]\n[]\n[roster1]\n[roster] # comment\n",
    ] {
        assert_eq!(
            parse_identity(text).unwrap(),
            IdentityConfig::default(),
            "{text:?}"
        );
    }
    let cfg = parse_identity("[profiles]\n[roster] # comment\n1a = ignored\na = kept\n").unwrap();
    assert_eq!(cfg.profiles, [("a".to_owned(), "kept".to_owned())]);
    assert!(cfg.roster.is_empty());
}

// Reader contract: only clients/profiles/roster and the two workspace seats land.
#[test]
fn ignored_sections_do_not_validate_or_claim_identity_keys() {
    for header in [
        "",
        "[unknown]\n",
        "[auto_reseat]\n",
        "[prices]\n",
        "[prompt]\n",
        "[Profiles]\n",
    ] {
        let text = format!("{header}no:colons = x\nno:colons = y\na = one\na = two\n");
        assert_eq!(
            parse_identity(&text).unwrap(),
            IdentityConfig::default(),
            "{text:?}"
        );
    }
    let cfg =
        parse_identity("[profiles]\np = one\n[unknown]\np = two\n[profiles]\nq = three\n").unwrap();
    assert_eq!(
        cfg.profiles,
        [
            ("p".to_owned(), "one".to_owned()),
            ("q".to_owned(), "three".to_owned())
        ]
    );
}

// key_claim contract + behaviour pins for empty and comment-suffixed raw keys.
#[test]
fn raw_key_claims_are_checked_before_empty_or_invalid_values() {
    for (text, expected) in [
        ("[roster]\na # =\n", roster_key("a #", 2)),
        ("[roster]\n= x\n", roster_key("", 2)),
        ("[clients]\n= ./x\n", client_key("", 2)),
        (
            "[roster]\nno:colons = # nothing\n",
            roster_key("no:colons", 2),
        ),
        (
            "[clients]\nno:colons = # nothing\n",
            client_key("no:colons", 2),
        ),
    ] {
        assert_eq!(parse_identity(text).unwrap_err(), expected, "{text:?}");
    }
    let cfg = parse_identity(
        "[roster]\n# no:colons = old\nlead\nlead = p\n[clients]\n# env = ./x\ncx = codex\n",
    )
    .unwrap();
    assert_eq!(cfg.roster, [("lead".to_owned(), "p".to_owned())]);
    assert_eq!(cfg.clients.len(), 1);
}

// Behaviour pin: ignored workspace keys and malformed config keys never claim.
#[test]
fn nonidentity_workspace_and_invalid_config_keys_never_collide() {
    let cfg = parse_identity("[profiles]\n1a = x\n1a = y\nno:colons = x\nno:colons = y\n= x\n= y\n_good = kept\n[workspace]\nlayout = x\nlayout = y\nmainly = x\nmainly = y\nno:colons = x\nno:colons = y\nmain = lead\nworkers = co\n").unwrap();
    assert_eq!(cfg.profiles, [("_good".to_owned(), "kept".to_owned())]);
    assert_eq!(cfg.main.as_deref(), Some("lead"));
    assert_eq!(cfg.workers.as_deref(), Some("co"));
    assert!(cfg.clients.is_empty() && cfg.roster.is_empty());
    assert_eq!(
        parse_identity("[workspace]\nworkers =\nworkers = co\n").unwrap_err(),
        duplicate("workspace", "workers", 3)
    );
}

// Agent-name contract: digit-leading allowed, maximum length 64, ASCII only.
#[test]
fn roster_and_client_labels_share_the_agent_name_boundary() {
    let longest = "n".repeat(64);
    let too_long = "n".repeat(65);
    for (section, value) in [("roster", "p"), ("clients", "codex")] {
        let text = format!("[{section}]\n2nd = {value}\n{longest} = {value}\n");
        let cfg = parse_identity(&text).unwrap();
        let labels: Vec<_> = if section == "roster" {
            cfg.roster.iter().map(|(key, _)| key.as_str()).collect()
        } else {
            cfg.clients.iter().map(|(key, _)| key.as_str()).collect()
        };
        assert_eq!(labels, ["2nd", longest.as_str()]);
        for key in [too_long.as_str(), "_bad", "ä", "a.b"] {
            let text = format!("[{section}]\n{key} =\n");
            let expected = if section == "roster" {
                roster_key(key, 2)
            } else {
                client_key(key, 2)
            };
            assert_eq!(parse_identity(&text).unwrap_err(), expected, "{text:?}");
        }
    }
}

// Reader duplicate contract: claims are per section and per file, even empty.
#[test]
fn duplicate_claims_survive_section_reentry_and_missing_values() {
    for (section, key, value) in [
        ("profiles", "a", "one"),
        ("roster", "lead", "p"),
        ("clients", "cx", "codex"),
        ("workspace", "main", "lead"),
        ("workspace", "workers", "co"),
    ] {
        for (first, second) in [("", value), (value, "# empty"), ("\"\"", value)] {
            // A quoted-empty client is refused at its first value, unlike an
            // absent value; its distinct refusal is pinned separately below.
            if section == "clients" && first == "\"\"" {
                continue;
            }
            let text = format!(
                "[{section}]\n{key} = {first}\n[unknown]\n{key} = ignored\n[{section}]\n{key} = {second}\n"
            );
            assert_eq!(
                parse_identity(&text).unwrap_err(),
                duplicate(section, key, 6),
                "{text:?}"
            );
        }
    }
    let cfg = parse_identity(
        "[profiles]\na = cmd\n[roster]\na = a\n[workspace]\nmain = a\nworkers = a\n",
    )
    .unwrap();
    assert_eq!(cfg.profiles, [("a".to_owned(), "cmd".to_owned())]);
    assert_eq!(cfg.roster, [("a".to_owned(), "a".to_owned())]);
}

// entry_value contract: whole quoted values preserve inner bytes; unquoted
// values strip comments. Non-whole quotes are a base behaviour pin.
#[test]
fn quoted_empty_and_comment_values_retain_their_exact_bytes() {
    for (rhs, expected) in [
        ("\"\"", Some("")),
        ("\"  cmd # literal  \"", Some("  cmd # literal  ")),
        ("cmd # note", Some("cmd")),
        ("# empty", None),
        ("", None),
        ("\"cmd\" # note", Some("\"cmd\"")),
    ] {
        for (section, key) in [
            ("profiles", "p"),
            ("roster", "lead"),
            ("workspace", "main"),
            ("workspace", "workers"),
        ] {
            let cfg = parse_identity(&format!(" [{section}] \n {key} = {rhs} \n")).unwrap();
            let actual = match section {
                "profiles" => cfg.profile(key),
                "roster" => cfg.roster_profile(key),
                _ if key == "main" => cfg.main.as_deref(),
                _ => cfg.workers.as_deref(),
            };
            assert_eq!(actual, expected, "[{section}] {key} = {rhs}");
        }
    }
}

// Prompt block contract: both supported bodies are raw text, never identity.
#[test]
fn both_prompt_block_kinds_hide_identity_text_and_keep_line_numbers() {
    for key in ["instructions", "spawn_instructions"] {
        let text = format!(
            "[prompt]\n{key} = \"\"\"\n[agents]\n[roster]\nno:colons = bad\n\"\"\"\n[profiles]\np = kept\n"
        );
        let cfg = parse_identity(&text).unwrap();
        assert_eq!(cfg.profiles, [("p".to_owned(), "kept".to_owned())]);
        assert!(cfg.roster.is_empty());
        let text = format!("{text}[roster]\nno:colons = bad\n");
        assert_eq!(
            parse_identity(&text).unwrap_err(),
            roster_key("no:colons", 10)
        );
    }
}

// config_lines contract: iterator refusals propagate even outside identity.
#[test]
fn instruction_block_refusals_propagate_with_original_provenance() {
    for (text, expected) in [
        (
            "[profiles]\nx = \"\"\"\n",
            ConfigError::InstructionBlock {
                file: source(),
                key: "profiles.x".to_owned(),
                line: 2,
            },
        ),
        (
            "[unknown]\nx = \"\"\"\n",
            ConfigError::InstructionBlock {
                file: source(),
                key: "unknown.x".to_owned(),
                line: 2,
            },
        ),
        (
            "x = \"\"\"\n",
            ConfigError::InstructionBlock {
                file: source(),
                key: "x".to_owned(),
                line: 1,
            },
        ),
        (
            "[prompt]\ninstructions = \"\"\"\n[agents]\n",
            ConfigError::UnterminatedInstructionBlock {
                file: source(),
                line: 2,
            },
        ),
        (
            "[prompt]\nspawn_instructions = \"\"\"\n",
            ConfigError::UnterminatedInstructionBlock {
                file: source(),
                line: 2,
            },
        ),
    ] {
        assert_eq!(parse_identity(text).unwrap_err(), expected, "{text:?}");
    }
    let path = Path::new("selected-global-config");
    let mut cfg = IdentityConfig::default();
    assert_eq!(
        overlay_identity_text(path, "[agents]\n", &mut cfg).unwrap_err(),
        ConfigError::LegacyAgents {
            file: path.to_owned(),
            line: 1
        }
    );
}

// read_identity overlay contract: local replaces in place and appends new keys.
// Drive its exact per-file step without introducing a filesystem scratch owner.
#[test]
fn overlays_replace_in_place_append_new_keys_and_keep_unmentioned_values() {
    let mut cfg = IdentityConfig::default();
    overlay_identity_text(Path::new("global"), "[clients]\na = claude\nb = codex\n[profiles]\na = old-a\nb = old-b\n[roster]\na = old-a\nb = old-b\n[workspace]\nmain = old-main\nworkers = old-workers\n", &mut cfg).unwrap();
    overlay_identity_text(Path::new("local"), "[clients]\nb = claude manual_resets=2\nc = codex\n[profiles]\nb = new-b\nc = new-c\n[roster]\nb = new-b\nc = new-c\n[workspace]\nmain = new-main\nworkers = new-workers\n", &mut cfg).unwrap();
    let rows = [
        ("a".to_owned(), "old-a".to_owned()),
        ("b".to_owned(), "new-b".to_owned()),
        ("c".to_owned(), "new-c".to_owned()),
    ];
    assert_eq!(cfg.profiles, rows);
    assert_eq!(cfg.roster, rows);
    assert_eq!(cfg.main.as_deref(), Some("new-main"));
    assert_eq!(cfg.workers.as_deref(), Some("new-workers"));
    assert_eq!(
        cfg.clients
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    for (key, executable, resets) in [
        ("a", "claude", None),
        ("b", "claude", Some(2)),
        ("c", "codex", None),
    ] {
        let client = cfg.client(key).unwrap();
        assert_eq!(client.executable, executable);
        assert_eq!(client.manual_resets, resets);
        assert_eq!(client.config_home, None);
        assert_eq!(client.manual_resets_note, None);
    }
}

// entry_value + overlay contract: absent value does not erase, quoted empty does.
#[test]
fn empty_overlay_claims_preserve_values_but_still_count_for_duplicates() {
    let global = "[clients]\ncx = codex\n[profiles]\np = cmd\n[roster]\nlead = p\n[workspace]\nmain = lead\nworkers = co\n";
    let original = parse_identity(global).unwrap();
    let mut cfg = original.clone();
    overlay_identity_text(Path::new("local"), "[clients]\ncx = # empty\n[profiles]\np =\n[roster]\nlead = # empty\n[workspace]\nmain =\nworkers = # empty\n", &mut cfg).unwrap();
    assert_eq!(cfg, original);
    overlay_identity_text(
        Path::new("next"),
        "[profiles]\np = \"\"\n[roster]\nlead = \"\"\n[workspace]\nmain = \"\"\nworkers = \"\"\n",
        &mut cfg,
    )
    .unwrap();
    assert_eq!(cfg.profile("p"), Some(""));
    assert_eq!(cfg.roster_profile("lead"), Some(""));
    assert_eq!(cfg.main.as_deref(), Some(""));
    assert_eq!(cfg.workers.as_deref(), Some(""));
    assert_eq!(cfg.clients, original.clients);
    assert!(matches!(
        parse_identity("[clients]\ncx = \"\"\n"),
        Err(ConfigError::ClientEntry { line: 2, .. })
    ));
}

// Behaviour pins: full diagnostics, independently literal, not formatted by the
// expected ConfigError (which would only compare Display with itself).
#[test]
fn refusal_diagnostics_are_byte_exact() {
    for (text, expected) in [
        (
            "[agents]\n",
            "Error: <config>:1: [agents] is not a v2 section — move each alias to [profiles] and bind agent names to profiles in [roster].",
        ),
        (
            "[roster]\n= x\n",
            "Error: <config>:2: invalid agent name '' in [roster]. Names must match ^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$.",
        ),
        (
            "[clients]\n_bad =\n",
            "Error: <config>:2: invalid client label '_bad' in [clients]. Labels must match ^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$.",
        ),
        (
            "[profiles]\np =\np = x\n",
            "Error: <config>:3: duplicate key 'p' in [profiles] — a v2 config names each key once per file.",
        ),
        (
            "[clients]\nenv =\n",
            "Error: <config>:2: invalid [clients] entry 'env': offending word 'env': the label is reserved prefix syntax",
        ),
        (
            "[clients]\ncx = ./x\n",
            "Error: <config>:2: invalid [clients] entry 'cx': offending word './x': executable must be a basename or absolute path",
        ),
        (
            "[profiles]\np = \"\"\"\n",
            "Error: <config>:2: 'profiles.p' cannot carry a \"\"\" block — prompt.instructions and prompt.spawn_instructions are the only keys that accept one.",
        ),
        (
            "[prompt]\ninstructions = \"\"\"\n",
            "Error: <config>:2: unterminated \"\"\" block — close it with a line that is exactly \"\"\".",
        ),
    ] {
        assert_eq!(
            parse_identity(text).unwrap_err().to_string(),
            expected,
            "{text:?}"
        );
    }
}
