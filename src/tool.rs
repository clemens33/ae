//! The six agent-tool adapters: one capability row per supported harness.
//!
//! Callers classify a profile once, then query its row. Strategy enums keep the
//! mechanics in their owning modules without making those modules classify
//! tools again.

/// Which harness a command launches — the six ae models, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
    /// Gemini CLI.
    Gemini,
    /// Antigravity CLI (`agy`).
    Agy,
    /// Grok Build.
    Grok,
    /// `OpenCode`.
    OpenCode,
    /// Anything else, or a command with no classifiable binary.
    Unknown,
}

/// Which session-flag grammar a command uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionFlags {
    /// Long `--session-id`, `--resume`, and `--continue` flags.
    Common,
    /// `--conversation`, `--continue`, and the `-c` alias.
    Conversation,
    /// Common flags plus the `-s`, `-r`, and `-c` aliases.
    ShortAliases,
}

/// How an ae-generated id reaches a fresh launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdStyle {
    /// Strip this grammar, then append the flag and id.
    Flag {
        flag: &'static str,
        grammar: SessionFlags,
    },
    /// The harness creates its own id after launch.
    None,
}

/// How workspace context reaches the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextChannel {
    /// Append context through this system-prompt flag.
    SystemPromptFlag(&'static str),
    /// Set Codex-compatible developer instructions and the registration task.
    DeveloperInstructions,
    /// Send context as a user turn, optionally through a flag and with a
    /// tool-store launch marker prefix.
    UserTurn {
        flag: Option<&'static str>,
        marker: Option<&'static str>,
    },
    /// Point the harness at generated instruction/config files.
    ConfigFile,
    /// No context channel is known.
    None,
}

/// How the final create/resume command carries environment changes and an
/// optional inline first turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandForm {
    /// Remove Claude nesting variables and disable prompt suggestions.
    SanitizedEnvironment,
    /// Preserve the command and append a non-empty first turn.
    InlinePrompt,
    /// Preserve the command and never append an inline turn.
    NoInlinePrompt,
}

/// The tool-created first user turn needed before any assigned work starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialTurn {
    /// Ask Codex to register its session id once, then wait.
    RegisterSessionId,
    /// No tool-created first turn.
    None,
}

/// Static launch behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LaunchSpec {
    /// The grammar removed before ae appends fresh or resume state.
    pub(crate) session_flags: SessionFlags,
    /// How a fresh launch receives an ae-generated id.
    pub(crate) id: IdStyle,
    /// How workspace context reaches the harness.
    pub(crate) context: ContextChannel,
    /// Environment and inline-prompt command composition.
    pub(crate) command: CommandForm,
    /// A first user turn needed before assigned work starts.
    pub(crate) initial_turn: InitialTurn,
}

impl LaunchSpec {
    /// Whether this harness accepts ae's conversation id on a fresh launch.
    pub(crate) const fn takes_session_id(self) -> bool {
        matches!(self.id, IdStyle::Flag { .. })
    }
}

/// How exact and fallback resume commands are composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeForm {
    /// Append an exact flag/id pair, or the fallback flags.
    Flags {
        exact: &'static str,
        fallback: &'static str,
    },
    /// Strip the harness's session grammar, then append exact or fallback
    /// flags.
    StrippedFlags {
        grammar: SessionFlags,
        exact: &'static str,
        fallback: &'static str,
    },
    /// Strip common flags, append an exact subcommand/id pair, and use the
    /// stripped command itself as fallback.
    Subcommand(&'static str),
    /// Preserve the command for both forms.
    None,
}

/// Evidence the tool's own store must provide before exact resume is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreProbe {
    /// Claude's project-scoped transcript path.
    ProjectTranscript,
    /// Codex's dated rollout directories.
    DatedRollouts,
    /// Agy's flat conversation database directory.
    ConversationDatabase,
    /// The recorded id is the available evidence; no local store probe exists.
    RecordedId,
}

/// Static resume behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResumeSpec {
    /// How exact and fallback command lines are composed.
    pub(crate) form: ResumeForm,
    /// What evidence permits the exact form.
    pub(crate) probe: StoreProbe,
}

/// Everything ae needs to know about one agent harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolAdapter {
    /// The classifier value that selects this row.
    pub(crate) kind: ToolKind,
    /// Binary classification, metadata, and live process-name spelling.
    pub(crate) name: &'static str,
    /// Human-facing name, or the full profile command for an unknown tool.
    pub(crate) label: Option<&'static str>,
    /// Fresh-launch and initial-turn behaviour.
    pub(crate) launch: LaunchSpec,
    /// Exact/fallback resume behaviour and its store evidence.
    pub(crate) resume: ResumeSpec,
}

const CLAUDE: ToolAdapter = ToolAdapter {
    kind: ToolKind::Claude,
    name: "claude",
    label: Some("claude code"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::Flag {
            flag: "--session-id",
            grammar: SessionFlags::Common,
        },
        context: ContextChannel::SystemPromptFlag("--append-system-prompt"),
        command: CommandForm::SanitizedEnvironment,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::Flags {
            exact: "--resume",
            fallback: "--continue",
        },
        probe: StoreProbe::ProjectTranscript,
    },
};

const CODEX: ToolAdapter = ToolAdapter {
    kind: ToolKind::Codex,
    name: "codex",
    label: Some("codex"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::DeveloperInstructions,
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::RegisterSessionId,
    },
    resume: ResumeSpec {
        form: ResumeForm::Subcommand("resume"),
        probe: StoreProbe::DatedRollouts,
    },
};

const GEMINI: ToolAdapter = ToolAdapter {
    kind: ToolKind::Gemini,
    name: "gemini",
    label: Some("gemini cli"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::UserTurn {
            flag: Some("-i"),
            marker: Some("GEMINI"),
        },
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::Flags {
            exact: "--resume",
            fallback: "--resume latest",
        },
        probe: StoreProbe::RecordedId,
    },
};

const AGY: ToolAdapter = ToolAdapter {
    kind: ToolKind::Agy,
    name: "agy",
    label: Some("antigravity cli"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Conversation,
        id: IdStyle::None,
        context: ContextChannel::UserTurn {
            flag: Some("-i"),
            marker: Some("AGY"),
        },
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::StrippedFlags {
            grammar: SessionFlags::Conversation,
            exact: "--conversation",
            fallback: "--continue",
        },
        probe: StoreProbe::ConversationDatabase,
    },
};

const GROK: ToolAdapter = ToolAdapter {
    kind: ToolKind::Grok,
    name: "grok",
    label: Some("grok build"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::Flag {
            flag: "--session-id",
            grammar: SessionFlags::ShortAliases,
        },
        context: ContextChannel::UserTurn {
            flag: None,
            marker: None,
        },
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::StrippedFlags {
            grammar: SessionFlags::ShortAliases,
            exact: "--resume",
            fallback: "--continue",
        },
        probe: StoreProbe::RecordedId,
    },
};

const OPENCODE: ToolAdapter = ToolAdapter {
    kind: ToolKind::OpenCode,
    name: "opencode",
    label: Some("opencode"),
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::ConfigFile,
        command: CommandForm::NoInlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::Flags {
            exact: "--session",
            fallback: "--continue",
        },
        probe: StoreProbe::RecordedId,
    },
};

const UNKNOWN: ToolAdapter = ToolAdapter {
    kind: ToolKind::Unknown,
    name: "unknown",
    label: None,
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::None,
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::None,
        probe: StoreProbe::RecordedId,
    },
};

const KNOWN: [&ToolAdapter; 6] = [&CLAUDE, &CODEX, &GEMINI, &AGY, &GROK, &OPENCODE];

impl ToolKind {
    /// Classify one bare binary name.
    #[must_use]
    pub fn from_binary_name(name: &str) -> Self {
        KNOWN
            .iter()
            .find(|adapter| adapter.name == name)
            .map_or(Self::Unknown, |adapter| adapter.kind)
    }

    /// Classify a whole profile command, failing toward unknown.
    #[must_use]
    pub fn from_cmd(cmd: &str) -> Self {
        crate::launch_cmd::split_binary(cmd).map_or(Self::Unknown, |split| {
            Self::from_binary_name(split.binary_name())
        })
    }

    /// Canonical diagnostic and metadata spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.adapter().name
    }

    /// The capabilities of this harness.
    #[must_use]
    pub(crate) const fn adapter(self) -> &'static ToolAdapter {
        match self {
            Self::Claude => &CLAUDE,
            Self::Codex => &CODEX,
            Self::Gemini => &GEMINI,
            Self::Agy => &AGY,
            Self::Grok => &GROK,
            Self::OpenCode => &OPENCODE,
            Self::Unknown => &UNKNOWN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_supported_binary_resolves_to_its_complete_adapter_row() {
        for adapter in KNOWN {
            assert_eq!(ToolKind::from_binary_name(adapter.name), adapter.kind);
            assert_eq!(adapter.kind.adapter(), adapter);
            assert!(adapter.label.is_some());
        }
        assert_eq!(
            ToolKind::from_binary_name("opencode.exe"),
            ToolKind::Unknown
        );
        assert_eq!(ToolKind::from_binary_name("other"), ToolKind::Unknown);
        assert_eq!(ToolKind::Unknown.adapter(), &UNKNOWN);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "six complete adapter rows are one readable contract matrix"
    )]
    fn capability_rows_pin_the_public_tool_contract() {
        let rows = KNOWN.map(|adapter| {
            (
                adapter.kind,
                adapter.name,
                adapter.label,
                adapter.launch.session_flags,
                adapter.launch.id,
                adapter.launch.context,
                adapter.launch.command,
                adapter.launch.initial_turn,
                adapter.resume.form,
                adapter.resume.probe,
            )
        });
        assert_eq!(
            rows,
            [
                (
                    ToolKind::Claude,
                    "claude",
                    Some("claude code"),
                    SessionFlags::Common,
                    IdStyle::Flag {
                        flag: "--session-id",
                        grammar: SessionFlags::Common,
                    },
                    ContextChannel::SystemPromptFlag("--append-system-prompt"),
                    CommandForm::SanitizedEnvironment,
                    InitialTurn::None,
                    ResumeForm::Flags {
                        exact: "--resume",
                        fallback: "--continue",
                    },
                    StoreProbe::ProjectTranscript,
                ),
                (
                    ToolKind::Codex,
                    "codex",
                    Some("codex"),
                    SessionFlags::Common,
                    IdStyle::None,
                    ContextChannel::DeveloperInstructions,
                    CommandForm::InlinePrompt,
                    InitialTurn::RegisterSessionId,
                    ResumeForm::Subcommand("resume"),
                    StoreProbe::DatedRollouts,
                ),
                (
                    ToolKind::Gemini,
                    "gemini",
                    Some("gemini cli"),
                    SessionFlags::Common,
                    IdStyle::None,
                    ContextChannel::UserTurn {
                        flag: Some("-i"),
                        marker: Some("GEMINI"),
                    },
                    CommandForm::InlinePrompt,
                    InitialTurn::None,
                    ResumeForm::Flags {
                        exact: "--resume",
                        fallback: "--resume latest",
                    },
                    StoreProbe::RecordedId,
                ),
                (
                    ToolKind::Agy,
                    "agy",
                    Some("antigravity cli"),
                    SessionFlags::Conversation,
                    IdStyle::None,
                    ContextChannel::UserTurn {
                        flag: Some("-i"),
                        marker: Some("AGY"),
                    },
                    CommandForm::InlinePrompt,
                    InitialTurn::None,
                    ResumeForm::StrippedFlags {
                        grammar: SessionFlags::Conversation,
                        exact: "--conversation",
                        fallback: "--continue",
                    },
                    StoreProbe::ConversationDatabase,
                ),
                (
                    ToolKind::Grok,
                    "grok",
                    Some("grok build"),
                    SessionFlags::Common,
                    IdStyle::Flag {
                        flag: "--session-id",
                        grammar: SessionFlags::ShortAliases,
                    },
                    ContextChannel::UserTurn {
                        flag: None,
                        marker: None,
                    },
                    CommandForm::InlinePrompt,
                    InitialTurn::None,
                    ResumeForm::StrippedFlags {
                        grammar: SessionFlags::ShortAliases,
                        exact: "--resume",
                        fallback: "--continue",
                    },
                    StoreProbe::RecordedId,
                ),
                (
                    ToolKind::OpenCode,
                    "opencode",
                    Some("opencode"),
                    SessionFlags::Common,
                    IdStyle::None,
                    ContextChannel::ConfigFile,
                    CommandForm::NoInlinePrompt,
                    InitialTurn::None,
                    ResumeForm::Flags {
                        exact: "--session",
                        fallback: "--continue",
                    },
                    StoreProbe::RecordedId,
                ),
            ]
        );
    }
}
