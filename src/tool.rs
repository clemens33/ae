//! The seven agent-tool adapters: one capability row per supported harness.
//!
//! Callers classify a profile once, then query its row. Strategy enums keep the
//! mechanics in their owning modules without making those modules classify
//! tools again.

/// Which harness a command launches — the seven ae models, or none.
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
    /// Muse Code.
    Muse,
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
    /// Send context as a user turn, optionally through a flag.
    UserTurn { flag: Option<&'static str> },
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
    Subcommand {
        grammar: SessionFlags,
        command: &'static str,
    },
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

/// How a harness-created conversation id is found after launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureSpec {
    /// Verify the handshake/rollout by token; cwd/TUI are legacy no-token fallbacks.
    HandshakeRolloutOrTui,
    /// Scan project chat history.
    ChatHistory,
    /// Scan conversation databases or the CLI log.
    ConversationDatabaseOrLog,
    /// Ask the harness for its session list.
    SessionList,
    /// Scan Muse's dated session directories for the launch token.
    MuseDatedSessions,
    /// No post-launch capture is needed.
    None,
}

impl CaptureSpec {
    /// Whether the launch needs capture metadata and a detached capture.
    pub(crate) const fn is_needed(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Which observable input-box grammar a harness draws.
///
/// Public delivery probes accept this behaviour directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModel {
    /// A prompt bounded by a structural bottom border.
    BorderDelimited,
    /// A styled prompt bounded by the last blank row before its footer.
    StyleDelimited,
    /// No input-box grammar is modelled.
    Unmodelled,
}

impl InputModel {
    /// Whether ae can prove this input box idle or occupied.
    #[must_use]
    pub const fn is_modelled(self) -> bool {
        !matches!(self, Self::Unmodelled)
    }
}

/// Which drawn composer geometry an unmodelled readiness check looks for.
///
/// The CHOICE lives here, beside the markers it pairs with; the GEOMETRY
/// lives in [`crate::deliver::region`], the one dispatcher. A modelled tool
/// never reads this: its composer answers through [`InputModel`] instead.
///
/// Pub because [`Composed`] exposes it, and the public `wait_input_ready`
/// probe takes that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerAnchor {
    /// A `┃`-rail box closed by a `╹▀` edge (opencode's measured shape).
    HeavyRail,
    /// A `│`-rail `╭`/`╰╯` box (grok's measured shape).
    RoundedBox,
    /// A `>` prompt row fenced by two full-width `─` rules (agy's shape).
    RuledPrompt,
}

/// The whole unmodelled composed signal: the markers AND the geometry that
/// owns them. One value so the two can never be mismatched at a call site.
///
/// Pub because the public `wait_input_ready` probe takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Composed {
    /// Which drawn structure the markers must sit inside.
    pub anchor: ComposerAnchor,
    /// Literals that prove the UI has COMPOSED, tested INSIDE the anchor. For
    /// [`ComposerAnchor::RoundedBox`] and [`ComposerAnchor::RuledPrompt`] the
    /// composer must also hold no draft: the box, the fence and the marker
    /// outlive a human's unsent text, so the marker alone would paste into it.
    /// For [`ComposerAnchor::HeavyRail`] the composer must hold no draft
    /// either, but NO marker presence is required: a seat with history draws
    /// the same box with an empty interior and no placeholder, so the markers
    /// here only name the placeholder PREFIX an empty interior may carry.
    pub markers: &'static [&'static str],
    /// The modal-dialog CHROME that refuses readiness: a header row carrying
    /// the dismiss word with the body row below it, whatever the dialog's
    /// title — a dialog open anywhere above the box refuses even when the box
    /// itself is drawn and empty.
    pub dialog: DialogSig,
}

/// One tool's modal-dialog refusal signal: the chrome whose open state
/// refuses readiness although the composer is drawn and empty. Titles are
/// deliberately NOT matched: the dialog family is open (pickers exist
/// unmeasured), so a title list would be one drift point per dialog.
///
/// Pub because [`Composed`] exposes it, and the public `wait_input_ready`
/// probe takes that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogSig {
    /// The dismiss affordance a dialog header row carries.
    pub dismiss: &'static str,
    /// The body row that must follow a matched header within [`DialogSig::gap`]
    /// rows, so a transcript mention of the dismiss word alone never refuses.
    pub body: &'static str,
    /// How many rows below the header the body may sit.
    pub gap: usize,
}

impl DialogSig {
    /// No measured dialog chrome: nothing is ever refused on its account.
    pub const NONE: Self = Self {
        dismiss: "",
        body: "",
        gap: 0,
    };

    /// Whether this carries no dialog signal.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.dismiss.is_empty() || self.body.is_empty()
    }
}

impl Composed {
    /// No usable composed signal: readiness REFUSES visibly rather than
    /// pasting into a frame ae cannot read. The anchor is unread when the
    /// markers are empty.
    pub const NONE: Self = Self {
        anchor: ComposerAnchor::HeavyRail,
        markers: &[],
        dialog: DialogSig::NONE,
    };

    /// Whether this carries no usable composed signal.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.markers.is_empty()
    }
}

/// Input-readiness and first-turn behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputSpec {
    /// The grammar used to observe the input box.
    pub(crate) model: InputModel,
    /// The whole unmodelled composed signal: the markers AND the drawn
    /// geometry that owns them ([`Composed`]). Each marker is tested INSIDE
    /// the bottom-most composer structure
    /// ([`crate::deliver::region::composed_ui`]), so a stray occurrence in a
    /// transcript or a modal never counts. [`Composed::NONE`] is a tool with
    /// no usable composed signal: its readiness REFUSES visibly rather than
    /// pasting into a frame ae cannot read. Read only by the unmodelled
    /// readiness arm; a modelled composer answers through [`InputModel`]
    /// instead.
    pub(crate) composed: Composed,
    /// Whether launch waits for the harness process to replace the pane shell.
    pub(crate) wait_for_process: bool,
    /// Whether a resumed seat receives its initial turn through a paste.
    pub(crate) paste_initial_on_resume: bool,
}

/// Which local vendor state, if any, can provide quota observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotaSource {
    /// Claude Code's cached usage snapshot.
    ClaudeCache,
    /// Codex rollout response records.
    CodexRollouts,
    /// No verified local quota source exists.
    Unsupported,
}

/// Static quota discovery behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuotaSpec {
    /// Client-owned source shape.
    pub(crate) source: QuotaSource,
    /// Environment assignment that relocates the client's config home.
    pub(crate) config_home_env: Option<&'static str>,
    /// Client config home relative to the operator home.
    pub(crate) default_home: Option<&'static str>,
    /// Operator action or reason shown when local quota is unsupported.
    pub(crate) unsupported_hint: Option<&'static str>,
}

/// Which local transcript shape, if any, can provide usage observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsageSource {
    /// Claude Code parent and subagent transcripts.
    ClaudeTranscripts,
    /// Codex cumulative rollout events.
    CodexRollout,
    /// No usage adapter exists yet.
    Unsupported,
}

/// Static usage discovery behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageSpec {
    /// Client-owned transcript shape.
    pub(crate) source: UsageSource,
}

/// How ae treats a harness's live model alongside its model flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelSpec {
    /// ae cannot read this harness's live model: no flag spelling is known and
    /// its seats are reported drift-unknown.
    Unobserved,
    /// ae reads the flag and reports the observed model, but never injects it:
    /// the scraped text is a display label, not a value drawn from the flag's
    /// own vocabulary.
    ReportOnly(&'static [&'static str]),
    /// The scraped text is drawn from the flag value's own vocabulary (a
    /// harness id), so ae may replay an observed model into the flag on resume.
    Replayable(&'static [&'static str]),
}

impl ModelSpec {
    /// The model flag spellings ae may READ in a profile command.
    pub(crate) const fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Unobserved => &[],
            Self::ReportOnly(flags) | Self::Replayable(flags) => flags,
        }
    }

    /// Whether ae observes this harness's live model at all.
    pub(crate) const fn observes(self) -> bool {
        !matches!(self, Self::Unobserved)
    }

    /// Whether an observation may be replayed into the flag on resume.
    pub(crate) const fn replays(self) -> bool {
        matches!(self, Self::Replayable(_))
    }
}

/// How one drawn model name is judged against the profile's pin.
///
/// Data on the adapter row, next to [`ModelSpec`]: the drift decision matches
/// this value, never the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMatch {
    /// Byte equality: the drawn name satisfies the pin only when it is the
    /// same string (every tool whose frame draws the flag's own vocabulary).
    Exact,
    /// Family/version equivalence (claude): the frame draws a display label
    /// (`Fable 5.1`) while the flag pins a harness id (`fable`), so the
    /// decision compares tokenised families and versions instead.
    FamilyVersion,
}

/// Which pane grammar proves a live model-and-effort identity for one harness.
///
/// Data only — the identity reader matches this enum, never the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentitySpec {
    /// The identity row inside a border-delimited composer (Claude's `🧠` row).
    BorderComposer,
    /// The `model effort · path` footer under a style-delimited composer.
    StyleFooter,
    /// The `model · effort · path · mode` footer under the composer's closing
    /// rule.
    RuleFooter,
    /// The `mode · model · effort` status row above a heavy-rail bottom edge.
    RailStatus,
    /// The `model (effort)` text inside a rounded composer's bottom border.
    BorderText,
    /// The trailing `model · effort` label on the last drawn row.
    TrailingLabel,
    /// No grammar is modelled: a pane proves no identity.
    Unmodelled,
}

/// In-place compaction capability: which `/compact`-shaped command ae may
/// drive in this harness, if any (R11). Data only — the call site matches
/// this enum, never the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactSpec {
    /// `<command> <instructions>`: the harness takes guidance with the
    /// command (claude, grok).
    Guided {
        /// The compaction command.
        command: &'static str,
    },
    /// `<command>` alone; the checkpoint carried the guidance (codex, gemini,
    /// muse, opencode).
    Bare {
        /// The compaction command.
        command: &'static str,
    },
    /// No drivable command (agy, unknown).
    Unsupported {
        /// Why no command can be driven.
        reason: &'static str,
    },
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
    /// The client token a roster row draws beside an observed model, at most
    /// four terminal cells.
    ///
    /// A NAME, not an abbreviation rule: an agent row has room for the model
    /// or for the client spelled out, not both, and the human reading it
    /// already knows their fleet. Short enough that the client never has to
    /// drop out of a narrow row — the model clips instead. Unknown is `-`,
    /// which says ae could not classify the binary rather than naming a tool
    /// it did not see.
    pub(crate) client: &'static str,
    /// Prefix shared by launch-marker writers and tool-store readers.
    pub(crate) launch_marker: Option<&'static str>,
    /// Environment variable that relocates this harness's account/config home.
    pub(crate) config_home_env: Option<&'static str>,
    /// Default config-home directory below `HOME`, for a verified account
    /// variable.
    pub(crate) config_home_default: Option<&'static str>,
    /// Refusal when a client names that default directory explicitly.
    pub(crate) config_home_default_refusal: &'static str,
    /// Fresh-launch and initial-turn behaviour.
    pub(crate) launch: LaunchSpec,
    /// Exact/fallback resume behaviour and its store evidence.
    pub(crate) resume: ResumeSpec,
    /// Post-launch conversation-id capture behaviour.
    pub(crate) capture: CaptureSpec,
    /// Input observation and first-turn delivery behaviour.
    pub(crate) input: InputSpec,
    /// Which pane grammar proves this harness's live identity, if any.
    pub(crate) identity: IdentitySpec,
    /// In-place compaction capability (R11).
    pub(crate) compact: CompactSpec,
    /// How ae reads this harness's live model and whether an observation may
    /// be replayed into its model flag on resume.
    ///
    /// The listed spellings are the ones whose resume flag ORDER was measured
    /// (2026-09-13) — claude `--model` and codex `-m`/`--model`. That
    /// measurement proves ae can FIND and REWRITE the flag; it never proves
    /// the text scraped from a live pane is a LEGAL VALUE for it, so replay is
    /// its own capability ([`ModelSpec`]). An unobserved tool is never told a
    /// model by ae and is reported drift-unknown.
    pub(crate) model: ModelSpec,
    /// How a drawn model name is judged against the profile's pin.
    pub(crate) pin_match: PinMatch,
    /// Local quota discovery behaviour.
    pub(crate) quota: QuotaSpec,
    /// Local usage discovery behaviour.
    pub(crate) usage: UsageSpec,
}

const CLAUDE: ToolAdapter = ToolAdapter {
    kind: ToolKind::Claude,
    name: "claude",
    label: Some("claude code"),
    client: "cc",
    launch_marker: None,
    config_home_env: Some("CLAUDE_CONFIG_DIR"),
    config_home_default: Some(".claude"),
    config_home_default_refusal: "config_home equals Claude's default directory under HOME; this switches the Claude state file; use the default client",
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
    capture: CaptureSpec::None,
    input: InputSpec {
        model: InputModel::BorderDelimited,
        composed: Composed::NONE,
        wait_for_process: true,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::BorderComposer,
    model: ModelSpec::ReportOnly(&["--model"]),
    pin_match: PinMatch::FamilyVersion,
    quota: QuotaSpec {
        source: QuotaSource::ClaudeCache,
        config_home_env: Some("CLAUDE_CONFIG_DIR"),
        default_home: Some(".claude"),
        unsupported_hint: None,
    },
    usage: UsageSpec {
        source: UsageSource::ClaudeTranscripts,
    },
    compact: CompactSpec::Guided {
        command: "/compact",
    },
};

const CODEX: ToolAdapter = ToolAdapter {
    kind: ToolKind::Codex,
    name: "codex",
    label: Some("codex"),
    client: "cx",
    launch_marker: Some("CODEX"),
    config_home_env: Some("CODEX_HOME"),
    config_home_default: Some(".codex"),
    config_home_default_refusal: "config_home equals codex's default directory under HOME; use the default client",
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::DeveloperInstructions,
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::RegisterSessionId,
    },
    resume: ResumeSpec {
        form: ResumeForm::Subcommand {
            grammar: SessionFlags::Common,
            command: "resume",
        },
        probe: StoreProbe::DatedRollouts,
    },
    capture: CaptureSpec::HandshakeRolloutOrTui,
    input: InputSpec {
        model: InputModel::StyleDelimited,
        composed: Composed::NONE,
        wait_for_process: true,
        paste_initial_on_resume: true,
    },
    identity: IdentitySpec::StyleFooter,
    model: ModelSpec::Replayable(&["-m", "--model"]),
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::CodexRollouts,
        config_home_env: Some("CODEX_HOME"),
        default_home: Some(".codex"),
        unsupported_hint: None,
    },
    usage: UsageSpec {
        source: UsageSource::CodexRollout,
    },
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const GEMINI: ToolAdapter = ToolAdapter {
    kind: ToolKind::Gemini,
    name: "gemini",
    label: Some("gemini cli"),
    client: "gem",
    launch_marker: Some("GEMINI"),
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        context: ContextChannel::UserTurn { flag: Some("-i") },
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
    capture: CaptureSpec::ChatHistory,
    input: InputSpec {
        model: InputModel::Unmodelled,
        composed: Composed::NONE,
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::Unmodelled,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".gemini"),
        unsupported_hint: Some("no verified local quota source"),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Bare {
        command: "/compress",
    },
};

const AGY: ToolAdapter = ToolAdapter {
    kind: ToolKind::Agy,
    name: "agy",
    label: Some("antigravity cli"),
    client: "agy",
    launch_marker: Some("AGY"),
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
    launch: LaunchSpec {
        session_flags: SessionFlags::Conversation,
        id: IdStyle::None,
        context: ContextChannel::UserTurn { flag: Some("-i") },
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
    capture: CaptureSpec::ConversationDatabaseOrLog,
    input: InputSpec {
        model: InputModel::Unmodelled,
        // MEASURED on agy 1.2.6 (2026-09-18, private probe panes, 80x24 and
        // 200x50): the boot frame is BLANK for ~0.5 s, then a "Signing in..."
        // spinner until ~+3 s, and the composer settles at ~+5 s (the account
        // row gains its plan suffix late). The structural anchor is the
        // rule-fenced `>` prompt row plus the footer row beneath it, owned by
        // `region::composed_ui`; this literal is the affordance that must sit
        // in those rows. The model label (`Gemini 3.8 Flash · high`) is NOT a
        // marker: the folder-trust modal carries it too, with no composer.
        // The prompt row must ALSO be empty: a measured single-line draft
        // (2026-09-18) replaces the footer hint with the model label alone,
        // and a wrapped draft adds continuation rows that break the
        // rule/prompt/rule scan. One version's UI text, same inherited drift
        // hazard as opencode's.
        composed: Composed {
            anchor: ComposerAnchor::RuledPrompt,
            markers: &["? for shortcuts"],
            dialog: DialogSig::NONE,
        },
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::TrailingLabel,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".gemini/antigravity-cli"),
        unsupported_hint: Some("run agy -p \"/quota\""),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Unsupported {
        reason: "agy exposes no compaction command",
    },
};

const GROK: ToolAdapter = ToolAdapter {
    kind: ToolKind::Grok,
    name: "grok",
    label: Some("grok build"),
    client: "grok",
    launch_marker: None,
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::Flag {
            flag: "--session-id",
            grammar: SessionFlags::ShortAliases,
        },
        // A positional turn preserves the harness's own system prompt.
        context: ContextChannel::UserTurn { flag: None },
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
    capture: CaptureSpec::None,
    input: InputSpec {
        model: InputModel::Unmodelled,
        // MEASURED on grok 1.0.34 (2026-09-18, private probe panes, 80x24 and
        // 200x50): the boot frame is BLANK until ~+1 s and the composer is
        // drawn and stable from ~+2 s. The structural anchor is the rounded
        // `╭`/`│`/`╰╯` box, owned by `region::composed_ui`; this literal is
        // the input affordance that must sit on one of its rail rows. The
        // rail row carrying it must hold nothing else and no other rail row
        // may carry text: a measured draft (2026-09-18) keeps `❯` drawn and
        // fills the input row or adds continuation rail rows, so the marker
        // alone would paste into the human's text. The bottom-edge footer
        // (`Weekly limit left: ...`, model, flags) is NOT a marker: quota
        // text is account state, not layout. The "Help improve Grok" banner
        // coexists WITH the composer; no composer-less banner state was
        // observed. One version's UI text, same inherited drift hazard as
        // opencode's.
        composed: Composed {
            anchor: ComposerAnchor::RoundedBox,
            markers: &["❯"],
            dialog: DialogSig::NONE,
        },
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::BorderText,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".grok"),
        unsupported_hint: Some("run /usage in grok"),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Guided {
        command: "/compact",
    },
};

const MUSE: ToolAdapter = ToolAdapter {
    kind: ToolKind::Muse,
    name: "muse",
    label: Some("muse code"),
    client: "muse",
    launch_marker: Some("MUSE"),
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
    launch: LaunchSpec {
        session_flags: SessionFlags::Common,
        id: IdStyle::None,
        // A positional turn preserves the harness's own system prompt.
        context: ContextChannel::UserTurn { flag: None },
        command: CommandForm::InlinePrompt,
        initial_turn: InitialTurn::None,
    },
    resume: ResumeSpec {
        form: ResumeForm::Subcommand {
            grammar: SessionFlags::Common,
            command: "resume",
        },
        // A token-proven directory basename is the session id. Muse logs do
        // not put that id in their `session.jsonl` file name for a store probe.
        probe: StoreProbe::RecordedId,
    },
    capture: CaptureSpec::MuseDatedSessions,
    input: InputSpec {
        // Modelled as a border-delimited composer: the live prompt is the
        // amber `❯` row, the staged content is the bold
        // `[Pasted Content N chars]` token, and the box is bounded below by a
        // full-width rule. Proven against real captures — stuck, occupied,
        // accepted, idle — in `tests/fixtures/muse-composer/`.
        model: InputModel::BorderDelimited,
        composed: Composed::NONE,
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::RuleFooter,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".config/muse"),
        unsupported_hint: Some("no verified local quota source"),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const OPENCODE: ToolAdapter = ToolAdapter {
    kind: ToolKind::OpenCode,
    name: "opencode",
    label: Some("opencode"),
    client: "oc",
    launch_marker: None,
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
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
    capture: CaptureSpec::SessionList,
    input: InputSpec {
        model: InputModel::Unmodelled,
        // MEASURED on opencode 1.18.31 (2026-09-15, ae-dev panes, 80x24;
        // 2026-09-19, private probes, 60x15 to 200x50): the boot frame is
        // BLANK for the first ~2.7 s — stable but not composed — while the
        // composer appears at ~+3.0 s. The structural anchor is the box's own
        // `┃` rails and `╹▀` bottom edge, owned by `region::composed_ui`;
        // this literal is the placeholder PREFIX an empty interior may carry
        // (the suggestion after it rotates per launch, and a seat with
        // history draws no placeholder at all). The row above the edge must
        // also parse as the status grammar — position names it, the grammar
        // proves it. Any dialog of the chrome below (measured: the Commands
        // palette and the Sessions list) steals keystrokes while the box
        // stays drawn and empty — a paste would land in the dialog's Search
        // field — so an open one refuses. The permission prompt is the NAMED
        // residual: unmeasured (a `true` turn ran unprompted under default
        // config), and chrome keying would not cover it (no Search field).
        // Tolerable because composed_ui gates only launch-shaped pastes —
        // launch, spawn, brief_retry, and the launch recheck — all
        // pre-first-turn, while send/ask/review never consult it. Widening:
        // brief_retry fires up to 30 min post-spawn, so a seat that has
        // since taken a turn reads composed where main refused it for the
        // unrelated missing placeholder. All UI text of ONE observed
        // version, an inherited version-drift hazard: a renamed composer
        // REFUSES visibly.
        composed: Composed {
            anchor: ComposerAnchor::HeavyRail,
            markers: &["Ask anything…"],
            dialog: DialogSig {
                dismiss: "esc",
                body: "Search",
                gap: 3,
            },
        },
        wait_for_process: true,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::RailStatus,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".local/share/opencode"),
        unsupported_hint: Some("local stats are cost history, not quota"),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const UNKNOWN: ToolAdapter = ToolAdapter {
    kind: ToolKind::Unknown,
    name: "unknown",
    label: None,
    client: "-",
    launch_marker: None,
    config_home_env: None,
    config_home_default: None,
    config_home_default_refusal: "",
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
    capture: CaptureSpec::None,
    input: InputSpec {
        model: InputModel::Unmodelled,
        composed: Composed::NONE,
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    identity: IdentitySpec::Unmodelled,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: None,
        unsupported_hint: Some("no local quota adapter"),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    compact: CompactSpec::Unsupported {
        reason: "unknown tool",
    },
};

const KNOWN: [&ToolAdapter; 7] = [&CLAUDE, &CODEX, &GEMINI, &AGY, &GROK, &MUSE, &OPENCODE];

/// Every explicit config-home environment variable declared by a known adapter.
///
/// Consumers that must isolate a whole process environment use this instead of
/// carrying a second list beside the adapter rows.
pub fn config_home_envs() -> impl Iterator<Item = &'static str> {
    KNOWN.iter().filter_map(|adapter| adapter.config_home_env)
}

/// Whether `token` is a client token some adapter row declares.
///
/// The picker's roster fact lives in a tmux option anyone with the server can
/// set, so its client field is validated against THIS table rather than by
/// shape. `-` is part of the vocabulary because an unclassifiable binary is a
/// thing ae can honestly report; `UNKNOWN` is not in [`KNOWN`], so it is
/// named beside it.
#[must_use]
pub fn is_client_token(token: &str) -> bool {
    KNOWN.iter().any(|adapter| adapter.client == token) || token == UNKNOWN.client
}

impl ToolKind {
    /// Classify one known bare binary name, preserving absence as `None`.
    #[must_use]
    pub(crate) fn from_known_binary_name(name: &str) -> Option<Self> {
        KNOWN
            .iter()
            .find(|adapter| adapter.name == name)
            .map(|adapter| adapter.kind)
    }

    /// Classify one bare binary name.
    #[must_use]
    pub fn from_binary_name(name: &str) -> Self {
        Self::from_known_binary_name(name).unwrap_or(Self::Unknown)
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

    /// The adapter-owned grammar for observing this harness's current input frame.
    #[must_use]
    pub(crate) const fn input_model(self) -> InputModel {
        self.adapter().input.model
    }

    /// The client token a roster row draws beside this harness's model.
    #[must_use]
    pub const fn client_token(self) -> &'static str {
        self.adapter().client
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
            Self::Muse => &MUSE,
            Self::OpenCode => &OPENCODE,
            Self::Unknown => &UNKNOWN,
        }
    }

    /// Whether an explicit config home for this tool can be newly broken by a
    /// working-directory change: only the cwd-keyed transcript probe reads
    /// the candidate cwd, so only it can newly fall back after a move. Every
    /// other probe (dated rollouts, record-only ids, HOME-anchored stores)
    /// is provably unaffected by where the work directory spells.
    #[must_use]
    pub(crate) const fn explicit_home_is_cwd_keyed(self) -> bool {
        matches!(self.adapter().resume.probe, StoreProbe::ProjectTranscript)
    }

    /// Whether this tool classified to a known harness (vs the unknown
    /// fallback a bare name falls toward).
    #[must_use]
    pub(crate) const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Whether this tool's seats need generated `opencode.<slot>.md/.json`
    /// context files: only the config-file channel does. The rename's asset
    /// check derives required pairs from the seat tool, never from whatever
    /// files happen to be present.
    #[must_use]
    pub(crate) const fn needs_generated_context(self) -> bool {
        matches!(self.adapter().launch.context, ContextChannel::ConfigFile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_adapter_row_carries_a_unique_client_token() {
        let mut seen: Vec<&str> = Vec::new();
        for adapter in KNOWN {
            let token = adapter.kind.client_token();
            assert_eq!(token, adapter.client, "{}: one owner", adapter.name);
            assert!(
                !token.is_empty() && token.len() <= 4,
                "{}: {token:?} must be 1..=4 bytes — the picker's client column",
                adapter.name
            );
            assert!(
                token.bytes().all(|byte| byte.is_ascii_lowercase()),
                "{}: {token:?} must be plain lowercase ASCII — it rides the \
                 roster fact, whose byte set forbids the rest",
                adapter.name
            );
            assert!(
                !seen.contains(&token),
                "{}: {token:?} already names another client",
                adapter.name
            );
            seen.push(token);
            assert!(is_client_token(token), "{}: {token:?}", adapter.name);
        }
        // Unknown is deliberately outside KNOWN and outside that spelling: it
        // says ae could not classify the binary, never that it saw a tool.
        assert_eq!(ToolKind::Unknown.client_token(), "-");
        assert!(is_client_token("-"));
        for absent in ["", "claude", "CC", "cc ", "oc!"] {
            assert!(!is_client_token(absent), "{absent:?}");
        }
    }

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
    fn config_home_envs_are_derived_from_known_adapter_rows() {
        let envs = config_home_envs().collect::<Vec<_>>();
        for expected in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            assert!(envs.contains(&expected), "missing {expected}: {envs:?}");
        }
        assert_eq!(
            envs,
            KNOWN
                .iter()
                .filter_map(|adapter| adapter.config_home_env)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_model_capability_has_three_states_and_claude_is_report_only() {
        // Observe nothing, observe-and-report, observe-and-replay. Claude's
        // footer yields a DISPLAY label, so its observation is reported while
        // its flag is still READ (for the pin and the report) — never rewritten.
        assert_eq!(
            ToolKind::Claude.adapter().model,
            ModelSpec::ReportOnly(&["--model"])
        );
        assert!(ToolKind::Claude.adapter().model.observes());
        assert!(!ToolKind::Claude.adapter().model.replays());
        assert_eq!(ToolKind::Claude.adapter().model.flags(), &["--model"]);

        assert_eq!(
            ToolKind::Codex.adapter().model,
            ModelSpec::Replayable(&["-m", "--model"])
        );
        assert!(ToolKind::Codex.adapter().model.replays());

        for tool in [
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::Muse,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(tool.adapter().model, ModelSpec::Unobserved, "{tool:?}");
            assert!(!tool.adapter().model.observes(), "{tool:?}");
            assert!(!tool.adapter().model.replays(), "{tool:?}");
            assert!(tool.adapter().model.flags().is_empty(), "{tool:?}");
        }
    }

    #[test]
    fn only_claude_judges_a_pin_by_family_version() {
        // Claude's frame draws a display label (`Fable 5.1`) while its flag
        // pins a harness id (`fable`); every other tool draws the flag's own
        // vocabulary, so byte equality is the rule there.
        assert_eq!(
            ToolKind::Claude.adapter().pin_match,
            PinMatch::FamilyVersion
        );
        for tool in [
            ToolKind::Codex,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::Muse,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(tool.adapter().pin_match, PinMatch::Exact, "{tool:?}");
        }
    }

    #[test]
    fn only_the_cwd_keyed_probe_can_break_on_a_directory_move() {
        // The rename's explicit-home preflight shares this owner: adding a
        // cwd-keyed probe to another tool must flip its row here.
        assert!(ToolKind::Claude.explicit_home_is_cwd_keyed());
        for tool in [
            ToolKind::Codex,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::Muse,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert!(!tool.explicit_home_is_cwd_keyed(), "{tool:?}");
        }
        for tool in [
            ToolKind::Claude,
            ToolKind::Codex,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::Muse,
            ToolKind::OpenCode,
        ] {
            assert!(tool.is_known(), "{tool:?}");
        }
        assert!(!ToolKind::Unknown.is_known());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "seven complete adapter rows are one readable contract matrix"
    )]
    fn capability_rows_pin_the_public_tool_contract() {
        assert_eq!(
            KNOWN.map(|adapter| *adapter),
            [
                ToolAdapter {
                    kind: ToolKind::Claude,
                    name: "claude",
                    label: Some("claude code"),
                    client: "cc",
                    launch_marker: None,
                    config_home_env: Some("CLAUDE_CONFIG_DIR"),
                    config_home_default: Some(".claude"),
                    config_home_default_refusal: "config_home equals Claude's default directory under HOME; this switches the Claude state file; use the default client",
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
                    capture: CaptureSpec::None,
                    input: InputSpec {
                        model: InputModel::BorderDelimited,
                        composed: Composed::NONE,
                        wait_for_process: true,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::BorderComposer,
                    model: ModelSpec::ReportOnly(&["--model"]),
                    pin_match: PinMatch::FamilyVersion,
                    quota: QuotaSpec {
                        source: QuotaSource::ClaudeCache,
                        config_home_env: Some("CLAUDE_CONFIG_DIR"),
                        default_home: Some(".claude"),
                        unsupported_hint: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::ClaudeTranscripts,
                    },
                    compact: CompactSpec::Guided {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Codex,
                    name: "codex",
                    label: Some("codex"),
                    client: "cx",
                    launch_marker: Some("CODEX"),
                    config_home_env: Some("CODEX_HOME"),
                    config_home_default: Some(".codex"),
                    config_home_default_refusal: "config_home equals codex's default directory under HOME; use the default client",
                    launch: LaunchSpec {
                        session_flags: SessionFlags::Common,
                        id: IdStyle::None,
                        context: ContextChannel::DeveloperInstructions,
                        command: CommandForm::InlinePrompt,
                        initial_turn: InitialTurn::RegisterSessionId,
                    },
                    resume: ResumeSpec {
                        form: ResumeForm::Subcommand {
                            grammar: SessionFlags::Common,
                            command: "resume",
                        },
                        probe: StoreProbe::DatedRollouts,
                    },
                    capture: CaptureSpec::HandshakeRolloutOrTui,
                    input: InputSpec {
                        model: InputModel::StyleDelimited,
                        composed: Composed::NONE,
                        wait_for_process: true,
                        paste_initial_on_resume: true,
                    },
                    identity: IdentitySpec::StyleFooter,
                    model: ModelSpec::Replayable(&["-m", "--model"]),
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::CodexRollouts,
                        config_home_env: Some("CODEX_HOME"),
                        default_home: Some(".codex"),
                        unsupported_hint: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::CodexRollout,
                    },
                    compact: CompactSpec::Bare {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Gemini,
                    name: "gemini",
                    label: Some("gemini cli"),
                    client: "gem",
                    launch_marker: Some("GEMINI"),
                    config_home_env: None,
                    config_home_default: None,
                    config_home_default_refusal: "",
                    launch: LaunchSpec {
                        session_flags: SessionFlags::Common,
                        id: IdStyle::None,
                        context: ContextChannel::UserTurn { flag: Some("-i") },
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
                    capture: CaptureSpec::ChatHistory,
                    input: InputSpec {
                        model: InputModel::Unmodelled,
                        composed: Composed::NONE,
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::Unmodelled,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".gemini"),
                        unsupported_hint: Some("no verified local quota source"),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    compact: CompactSpec::Bare {
                        command: "/compress"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Agy,
                    name: "agy",
                    label: Some("antigravity cli"),
                    client: "agy",
                    launch_marker: Some("AGY"),
                    config_home_env: None,
                    config_home_default: None,
                    config_home_default_refusal: "",
                    launch: LaunchSpec {
                        session_flags: SessionFlags::Conversation,
                        id: IdStyle::None,
                        context: ContextChannel::UserTurn { flag: Some("-i") },
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
                    capture: CaptureSpec::ConversationDatabaseOrLog,
                    input: InputSpec {
                        model: InputModel::Unmodelled,
                        composed: Composed {
                            anchor: ComposerAnchor::RuledPrompt,
                            markers: &["? for shortcuts"],
                            dialog: DialogSig::NONE,
                        },
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::TrailingLabel,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".gemini/antigravity-cli"),
                        unsupported_hint: Some("run agy -p \"/quota\""),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    compact: CompactSpec::Unsupported {
                        reason: "agy exposes no compaction command",
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Grok,
                    name: "grok",
                    label: Some("grok build"),
                    client: "grok",
                    launch_marker: None,
                    config_home_env: None,
                    config_home_default: None,
                    config_home_default_refusal: "",
                    launch: LaunchSpec {
                        session_flags: SessionFlags::Common,
                        id: IdStyle::Flag {
                            flag: "--session-id",
                            grammar: SessionFlags::ShortAliases,
                        },
                        context: ContextChannel::UserTurn { flag: None },
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
                    capture: CaptureSpec::None,
                    input: InputSpec {
                        model: InputModel::Unmodelled,
                        composed: Composed {
                            anchor: ComposerAnchor::RoundedBox,
                            markers: &["❯"],
                            dialog: DialogSig::NONE,
                        },
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::BorderText,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".grok"),
                        unsupported_hint: Some("run /usage in grok"),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    compact: CompactSpec::Guided {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Muse,
                    name: "muse",
                    label: Some("muse code"),
                    client: "muse",
                    launch_marker: Some("MUSE"),
                    config_home_env: None,
                    config_home_default: None,
                    config_home_default_refusal: "",
                    launch: LaunchSpec {
                        session_flags: SessionFlags::Common,
                        id: IdStyle::None,
                        context: ContextChannel::UserTurn { flag: None },
                        command: CommandForm::InlinePrompt,
                        initial_turn: InitialTurn::None,
                    },
                    resume: ResumeSpec {
                        form: ResumeForm::Subcommand {
                            grammar: SessionFlags::Common,
                            command: "resume",
                        },
                        probe: StoreProbe::RecordedId,
                    },
                    capture: CaptureSpec::MuseDatedSessions,
                    input: InputSpec {
                        model: InputModel::BorderDelimited,
                        composed: Composed::NONE,
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::RuleFooter,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".config/muse"),
                        unsupported_hint: Some("no verified local quota source"),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    compact: CompactSpec::Bare {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::OpenCode,
                    name: "opencode",
                    label: Some("opencode"),
                    client: "oc",
                    launch_marker: None,
                    config_home_env: None,
                    config_home_default: None,
                    config_home_default_refusal: "",
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
                    capture: CaptureSpec::SessionList,
                    input: InputSpec {
                        model: InputModel::Unmodelled,
                        composed: Composed {
                            anchor: ComposerAnchor::HeavyRail,
                            markers: &["Ask anything…"],
                            dialog: DialogSig {
                                dismiss: "esc",
                                body: "Search",
                                gap: 3,
                            },
                        },
                        wait_for_process: true,
                        paste_initial_on_resume: false,
                    },
                    identity: IdentitySpec::RailStatus,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".local/share/opencode"),
                        unsupported_hint: Some("local stats are cost history, not quota"),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    compact: CompactSpec::Bare {
                        command: "/compact"
                    },
                },
            ]
        );
    }

    #[test]
    fn unknown_adapter_is_inert() {
        assert_eq!(
            ToolKind::Unknown.adapter(),
            &ToolAdapter {
                kind: ToolKind::Unknown,
                name: "unknown",
                label: None,
                client: "-",
                launch_marker: None,
                config_home_env: None,
                config_home_default: None,
                config_home_default_refusal: "",
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
                capture: CaptureSpec::None,
                input: InputSpec {
                    model: InputModel::Unmodelled,
                    composed: Composed::NONE,
                    wait_for_process: false,
                    paste_initial_on_resume: false,
                },
                identity: IdentitySpec::Unmodelled,
                model: ModelSpec::Unobserved,
                pin_match: PinMatch::Exact,
                quota: QuotaSpec {
                    source: QuotaSource::Unsupported,
                    config_home_env: None,
                    default_home: None,
                    unsupported_hint: Some("no local quota adapter"),
                },
                usage: UsageSpec {
                    source: UsageSource::Unsupported,
                },
                compact: CompactSpec::Unsupported {
                    reason: "unknown tool",
                },
            }
        );
    }

    #[test]
    fn every_adapter_row_carries_its_compaction_command() {
        // R11 pin: the §2 measured table, one row per adapter. The call site
        // matches `CompactSpec`, never `ToolKind`.
        const GUIDED_COMPACT: CompactSpec = CompactSpec::Guided {
            command: "/compact",
        };
        const BARE_COMPACT: CompactSpec = CompactSpec::Bare {
            command: "/compact",
        };
        const BARE_COMPRESS: CompactSpec = CompactSpec::Bare {
            command: "/compress",
        };
        for (kind, expected) in [
            (ToolKind::Claude, GUIDED_COMPACT),
            (ToolKind::Codex, BARE_COMPACT),
            (ToolKind::Gemini, BARE_COMPRESS),
            (ToolKind::Grok, GUIDED_COMPACT),
            (ToolKind::Muse, BARE_COMPACT),
            (ToolKind::OpenCode, BARE_COMPACT),
        ] {
            assert_eq!(kind.adapter().compact, expected, "{kind:?}");
        }
        for kind in [ToolKind::Agy, ToolKind::Unknown] {
            let spec = kind.adapter().compact;
            assert!(
                matches!(spec, CompactSpec::Unsupported { reason } if !reason.is_empty()),
                "{kind:?}"
            );
        }
    }
}
