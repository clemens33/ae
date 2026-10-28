//! The per-role pick `ae init` renders: a harness, one of its models and an
//! effort that model takes, chosen separately and rendered as one profile.
//!
//! The table is a dated snapshot, not a live probe (init runs no child): a
//! model added later needs one row here, or a hand-written profile.

/// The two standing seats a pick is made for; `Debug` is the prompt label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Lead,
    Colead,
}

impl Role {
    /// The harness and effort a role takes when nothing names one.
    pub(super) const fn defaults(self) -> (&'static str, &'static str) {
        match self {
            Self::Lead => ("claude", "high"),
            Self::Colead => ("codex", "xhigh"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Model {
    pub(super) id: &'static str,
    /// Stem of the generated profile name; unique across the whole table.
    code: &'static str,
    pub(super) efforts: &'static [&'static str],
}

impl Model {
    /// The role's effort when this model takes it, else the model's first.
    fn effort_for(&self, role: Role) -> &'static str {
        let word = role.defaults().1;
        if self.efforts.contains(&word) {
            word
        } else {
            self.efforts.first().copied().unwrap_or(word)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Harness {
    pub(super) name: &'static str,
    pub(super) provider: &'static str,
    /// Launch words up to the model flag, the model flag, the effort flag up
    /// to its value (a profile command is ONE simple command).
    launch: [&'static str; 3],
    /// The first row is the default model.
    pub(super) models: &'static [Model],
}

// Harness vocabulary: `claude --help` (Claude Code 2.1.312, 2026-10-28) lists
// `--effort <level>` as low, medium, high, xhigh, max. That is the harness's
// vocabulary, not per-model server eligibility; ids are the ones
// `src/usage/prices.rs` prices, and `fable` is the alias `fablex` pins.
const CLAUDE: &[&str] = &["low", "medium", "high", "xhigh", "max"];
// codex 0.165.0 `~/.codex/models_cache.json` (fetched 2026-10-28),
// `supported_reasoning_levels` of each listed row.
const CODEX: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
// The same cache lists gpt-6-luna without `ultra`.
const CODEX_LUNA: &[&str] = &["low", "medium", "high", "xhigh", "max"];

const fn row(id: &'static str, code: &'static str, efforts: &'static [&'static str]) -> Model {
    Model { id, code, efforts }
}

/// Only these harnesses have a measured table; the rest keep a catalog profile.
pub(super) static HARNESSES: [Harness; 2] = [
    Harness {
        name: "claude",
        provider: "Anthropic",
        launch: [
            "claude --permission-mode bypassPermissions",
            "--model",
            "--effort ",
        ],
        models: &[
            row("claude-opus-5-5", "opus55", CLAUDE),
            row("claude-sonnet-5-5", "sonnet55", CLAUDE),
            row("claude-haiku-5-5", "haiku55", CLAUDE),
            row("fable", "fable", CLAUDE),
        ],
    },
    Harness {
        name: "codex",
        provider: "OpenAI",
        launch: ["codex --yolo", "-m", "-c model_reasoning_effort="],
        models: &[
            row("gpt-6.1-sol", "sol61", CODEX),
            row("gpt-6.1-terra", "terra61", CODEX),
            row("gpt-6-astra", "astra6", CODEX),
            row("gpt-6-sol", "sol6", CODEX),
            row("gpt-6-luna", "luna6", CODEX_LUNA),
        ],
    },
];

pub(super) fn harness(name: &str) -> Option<&'static Harness> {
    HARNESSES.iter().find(|harness| harness.name == name)
}

/// One validated choice: it can only be built from table rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pick {
    pub(super) harness: &'static Harness,
    pub(super) model: &'static Model,
    pub(super) effort: &'static str,
}

impl Pick {
    /// Resolve a role's pick from optional parts. A missing harness is
    /// inferred from a model id (unique in the table), else `fallback`; a
    /// missing model is the harness's default; a missing effort is the
    /// role's default. The error is the offending value.
    pub(super) fn resolve(
        role: Role,
        fallback: &str,
        harness_arg: Option<&str>,
        model_arg: Option<&str>,
        effort_arg: Option<&str>,
    ) -> Result<Self, String> {
        let inferred = model_arg.and_then(|id| {
            HARNESSES
                .iter()
                .find(|h| h.models.iter().any(|m| m.id == id))
        });
        let wanted = harness_arg.unwrap_or_else(|| inferred.map_or(fallback, |h| h.name));
        let harness = harness(wanted).ok_or_else(|| wanted.to_owned())?;
        let model = match model_arg {
            Some(id) => harness.models.iter().find(|m| m.id == id),
            None => harness.models.first(),
        }
        .ok_or_else(|| model_arg.unwrap_or(wanted).to_owned())?;
        let effort = match effort_arg {
            Some(word) => model.efforts.iter().copied().find(|e| *e == word),
            None => Some(model.effort_for(role)),
        }
        .ok_or_else(|| effort_arg.unwrap_or_default().to_owned())?;
        Ok(Self {
            harness,
            model,
            effort,
        })
    }

    /// The generated profile name: model stem, a dash, the effort. The dash
    /// keeps it apart from every catalog name.
    pub(super) fn name(&self) -> String {
        format!("{}-{}", self.model.code, self.effort)
    }

    /// The profile's launch command, in the shape the live seat profiles use.
    pub(super) fn command(&self) -> String {
        let [launch, model_flag, effort_flag] = self.harness.launch;
        format!(
            "{launch} {model_flag} {} {effort_flag}{}",
            self.model.id, self.effort
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effort::Level;
    use crate::entry::{DEFAULT_CONFIG, PROFILE_CATALOG};
    use crate::tool::{EffortSpec, ToolKind};

    fn picks() -> Vec<Pick> {
        let mut all = Vec::new();
        for harness in &HARNESSES {
            for model in harness.models {
                for effort in model.efforts {
                    all.push(Pick {
                        harness,
                        model,
                        effort,
                    });
                }
            }
        }
        all
    }

    #[test]
    fn generated_names_are_unique_and_disjoint_from_every_catalog_and_default_profile() {
        let picks = picks();
        let mut names: Vec<String> = picks.iter().map(Pick::name).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "two table rows render one name");
        for name in &names {
            assert!(crate::config::is_agent_name(name), "{name}");
            assert!(name.contains('-'), "{name}");
            assert!(PROFILE_CATALOG.iter().all(|p| p.name != name), "{name}");
            assert!(
                DEFAULT_CONFIG
                    .lines()
                    .all(|line| !line.starts_with(&format!("{name} = "))),
                "{name} shadows a seeded profile"
            );
        }
        let mut ids: Vec<&str> = HARNESSES
            .iter()
            .flat_map(|h| h.models.iter().map(|m| m.id))
            .collect();
        let models = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), models, "a model id names two harnesses");
    }

    #[test]
    fn every_row_effort_is_in_the_tools_vocabulary_and_roles_have_defaults() {
        for harness in &HARNESSES {
            let kind = ToolKind::from_binary_name(harness.name);
            let vocabulary = match kind.adapter().effort {
                EffortSpec::Flag { vocabulary, .. } | EffortSpec::ConfigPair { vocabulary, .. } => {
                    vocabulary
                }
                EffortSpec::Unsupported => panic!("{} has no effort spelling", harness.name),
            };
            for model in harness.models {
                for effort in model.efforts {
                    assert!(
                        vocabulary.iter().any(|level| level.word() == *effort),
                        "{} {effort}",
                        model.id
                    );
                }
                for role in [Role::Lead, Role::Colead] {
                    assert!(model.efforts.contains(&role.defaults().1), "{}", model.id);
                }
            }
        }
    }

    #[test]
    fn every_generated_command_round_trips_its_model_and_effort() {
        for pick in picks() {
            let kind = ToolKind::from_binary_name(pick.harness.name);
            let command = pick.command();
            assert_eq!(
                crate::launch_cmd::model_flag_value(&command, kind).as_deref(),
                Some(pick.model.id),
                "{command}"
            );
            assert_eq!(
                crate::launch_cmd::effort_flag(&command, kind)
                    .ok()
                    .map(Level::word),
                Some(pick.effort),
                "{command}"
            );
            assert!(crate::launch_cmd::lex_simple_command(&command).is_ok());
        }
    }

    #[test]
    fn resolve_infers_defaults_and_refuses_pairs_the_table_does_not_hold() {
        let ok = |h, m, e| Pick::resolve(Role::Lead, "claude", h, m, e);
        let opus = ok(None, None, None).expect("defaults");
        assert_eq!(opus.name(), "opus55-high");
        let sol = ok(None, Some("gpt-6.1-sol"), None).expect("inferred harness");
        assert_eq!(sol.harness.name, "codex");
        assert_eq!(sol.effort, "high");
        assert_eq!(
            Pick::resolve(Role::Colead, "codex", None, None, None).map(|p| p.name()),
            Ok("sol61-xhigh".to_owned())
        );
        assert_eq!(
            ok(Some("claude"), Some("gpt-6.1-sol"), None),
            Err("gpt-6.1-sol".into())
        );
        assert_eq!(ok(Some("grok"), None, None), Err("grok".into()));
        assert_eq!(ok(Some("claude"), None, Some("ultra")), Err("ultra".into()));
        assert_eq!(
            ok(None, Some("gpt-6-luna"), Some("ultra")),
            Err("ultra".into())
        );
        assert!(ok(None, Some("gpt-6-luna"), Some("max")).is_ok());
        assert_eq!(ok(None, Some("nope"), None), Err("nope".into()));
    }
}
