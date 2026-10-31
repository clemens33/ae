//! The seven agent-tool adapters: one capability row per supported harness.
//!
//! Callers classify a profile once, then query its row. Strategy enums keep the
//! mechanics in their owning modules without making those modules classify
//! tools again.

use crate::effort::Level;

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
    /// Strip the harness's session grammar, then append an exact flag/id
    /// pair. The fallback is not a flag at all: every fallback is a fresh
    /// start composed by `launch::inject_session_id`, so this form owns the
    /// exact half alone.
    StrippedExact {
        grammar: SessionFlags,
        exact: &'static str,
    },
    /// Strip common flags, append an exact subcommand/id pair, and use the
    /// stripped fresh start as fallback. `after` names the flags whose
    /// `flag value` pairs ride BEHIND the id: the harness drops them written
    /// before its subcommand (codex's `-c`, measured 0.159.2), so an empty
    /// list keeps the whole command where it was.
    Subcommand {
        grammar: SessionFlags,
        command: &'static str,
        after: &'static [&'static str],
    },
    /// Append an exact flag/id pair. The fallback is a fresh start composed
    /// by `launch::inject_session_id`: an unproven seat starts fresh rather
    /// than resuming a conversation ae cannot prove is its own.
    ExactOnly { exact: &'static str },
    /// Preserve the command for the exact form; the fallback is the stripped
    /// fresh start, like every other tool's.
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

/// Whether one conversation of this harness can be MOVED between two accounts
/// of the same tool by copying files, and what that file set is.
///
/// The question `ae reseat` asks when the profile it moves a seat to runs the
/// SAME binary under a different config home: the conversation can travel only
/// if the tool keeps it as a portable set of ordinary files in that home.
///
/// Every row but claude's is [`CarrySpec::NotPortable`], and that is a
/// statement about EVIDENCE rather than about the tools: claude's portability
/// was measured (2026-09-19, claude 2.1.278 — a transcript copied into another
/// config home is found by `--resume <uuid>`, and the resume appends). codex
/// and muse look plausibly portable on paper and are UNMEASURED; agy, gemini
/// and opencode keep databases or stores ae has not characterized at all. An
/// unmeasured tool takes the seed path, which is what it did before this
/// existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CarrySpec {
    /// No measured portable file set: a move to another account starts a fresh
    /// conversation and the successor is handed the seed pack.
    NotPortable,
    /// Claude's store: one transcript at `projects/<cwd key>/<uuid>.jsonl`,
    /// with uuid-keyed sidecars beside it and at the store root.
    ProjectTranscript,
}

/// Static resume behaviour for one harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResumeSpec {
    /// How exact and fallback command lines are composed.
    pub(crate) form: ResumeForm,
    /// What evidence permits the exact form.
    pub(crate) probe: StoreProbe,
    /// Whether an exact resume of the SAME conversation takes another model or
    /// effort, so `ae reseat` within one account keeps it.
    pub(crate) keep: KeepSpec,
}

/// Which launch words an exact resume may change and still continue the SAME
/// conversation — measured per harness (2026-10-21: claude 2.1.306 `--resume`
/// with `--model`/`--effort`, codex 0.163.0 `resume` with `-m` and
/// `-c model_reasoning_effort=`, opencode 1.19.3 `-s` with `-m`; a codeword
/// from turn 1 recalled on the other model). Every other row is unmeasured, and
/// an unmeasured move seeds exactly as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeepSpec {
    /// No measured resume across a model change.
    Unmeasured,
    /// The model flag spellings, and whether the effort setting may differ too.
    Measured {
        model_flags: &'static [&'static str],
        effort: bool,
    },
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
    /// The grok CLI's shared debug log, read only for its one billing record.
    GrokLog,
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
    /// A subscription plan whose usage ae may ask the vendor for, only once
    /// the operator opts in, for a profile routed through that plan.
    pub(crate) plan: Option<PlanRoute>,
}

/// How a profile names a subscription plan's model route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanRoute {
    /// The flags that carry the model.
    pub(crate) model_flags: &'static [&'static str],
    /// The model prefix that routes through the plan; a model must follow it.
    pub(crate) prefix: &'static str,
}

impl PlanRoute {
    /// Whether `command` names exactly one model, and that model is routed
    /// through this plan.
    pub(crate) fn routes(&self, command: &str) -> bool {
        crate::launch_cmd::sole_flag_value(command, self.model_flags).is_some_and(|model| {
            model
                .strip_prefix(self.prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
    }
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

/// Which reader turns one line stream into board rows. Each is a format, and
/// the reader itself stays with the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoardParser {
    /// Claude Code transcript records.
    Claude,
    /// Codex rollout records.
    Codex,
    /// The agy history file.
    Agy,
    /// Grok `updates.jsonl` records.
    Grok,
    /// Muse `session.jsonl` records.
    Muse,
}

/// A conversation the board reads as a line stream through its one door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BoardStream {
    /// The format of the stream.
    pub(crate) parser: BoardParser,
}

/// How the board reads this harness's conversation, or why it does not.
///
/// Values only: a row names no board code, so this module never depends on the
/// board, and usage support decides nothing here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoardSpec {
    /// A file the board streams and can follow.
    Stream(BoardStream),
    /// A snapshot the harness's own CLI exports. No file, nothing to follow.
    Export,
    /// The board does not read this harness; `reason` is the coverage row.
    Unsupported {
        /// The coverage row's text.
        reason: &'static str,
    },
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

/// How a profile command spells this harness's reasoning effort — data only;
/// the one grammar reading it is [`crate::launch_cmd::effort_flag`]. Only a
/// spelling MEASURED for its harness is listed; every other row is
/// `Unsupported`, and ae derives no effort twin there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffortSpec {
    /// ae reads and rewrites no effort for this harness.
    Unsupported,
    /// A dedicated flag, split (`--effort high`) or joined (`--effort=high`).
    Flag {
        /// The flag spellings.
        names: &'static [&'static str],
        /// The levels the flag takes.
        vocabulary: &'static [Level],
        /// The variable that overrides the flag: a command whose prefix sets
        /// or unsets it proves no level.
        env: Option<&'static str>,
    },
    /// A config override, `-c key=level` or, long spellings only,
    /// `--config=key=level`.
    ConfigPair {
        /// The override flag spellings.
        flags: &'static [&'static str],
        /// The config key.
        key: &'static str,
        /// The levels the key takes.
        vocabulary: &'static [Level],
    },
}

impl EffortSpec {
    /// Whether ae reads this harness's effort at all.
    pub(crate) const fn supported(self) -> bool {
        !matches!(self, Self::Unsupported)
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

/// How a harness draws a prompt only the HUMAN may answer — data only; the
/// ONE detector that reads it is [`crate::watchdog::human_prompt_class`].
/// MEASURED per tool from a frozen frame: one version's UI text, the same
/// inherited drift hazard as the composer markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptSpec {
    /// Which row asks the question.
    pub(crate) question: Question,
    /// A row the modal draws ABOVE its question, directly under a rule of `─`
    /// as wide as every row of the window. The modal's own anchor where its
    /// question, options and hint are prose a transcript could carry; `None`
    /// asks for none.
    pub(crate) title: Option<&'static str>,
    /// The glyph a SELECTED option row starts with.
    pub(crate) selected: char,
    /// Literals, one of which the KEY-HINT row carries.
    pub(crate) keys: &'static [&'static str],
    /// How far up from the last non-blank row the whole shape must sit, in
    /// ROWS: from the title's rule when there is a title, else the question.
    pub(crate) window: usize,
    /// The folder-trust block this prompt can be, MEASURED whole; the only
    /// prompt `auto_trust` may answer. `None` answers nothing.
    pub(crate) trust: Option<TrustSpec>,
}

/// A folder-trust modal exactly as one version draws it. Wrapped rows are
/// joined with one space, so the block reads the same at every width that
/// keeps its folder on one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrustSpec {
    /// The row above the ONE folder row, which sits directly above the question.
    pub(crate) heading: &'static str,
    /// Every paragraph from the question to the options, in order, the
    /// question's own first.
    pub(crate) paragraphs: &'static [&'static str],
    /// The option rows in drawn order, selection glyph stripped.
    pub(crate) options: &'static [&'static str],
    /// The option that trusts the folder.
    pub(crate) yes: &'static str,
    /// The key-hint row, trimmed.
    pub(crate) hint: &'static str,
    /// The most non-blank rows the modal draws below its hint.
    pub(crate) trailer: usize,
}

/// An information panel a harness draws in place of its composer until a key
/// closes it — data only, read by [`crate::watchdog::human_prompt_class`]. It
/// is NAMED by its title and hint alone; `auto_trust` may dismiss it only when
/// its body is one listed here WHOLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelSpec {
    /// The title row, trimmed, of a panel that counts nothing.
    pub(crate) title: &'static str,
    /// The title row of a counted panel: these two around `<i> of <n>`.
    pub(crate) counted: (&'static str, &'static str),
    /// The key-hint row, trimmed: the last non-blank row.
    pub(crate) hint: &'static str,
    /// How far up from the hint, in rows, the title must sit, hint included.
    pub(crate) window: usize,
    /// The bodies that may be dismissed, wrapped rows joined; each `{}` is a
    /// slot holding one token with no space or backtick.
    pub(crate) bodies: &'static [&'static str],
    /// The longest a slot may be, in bytes.
    pub(crate) slot_bytes: usize,
}

/// How a human-only prompt's question row is recognised, once trimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Question {
    /// A row that ENDS with this text: the whole row is the question.
    EndsWith(&'static str),
    /// A row that STARTS with this text: the question runs on past it, so it
    /// is reported through its first `?`.
    StartsWith(&'static str),
}

/// Everything ae needs to know about one agent harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolAdapter {
    /// The classifier value that selects this row.
    pub(crate) kind: ToolKind,
    /// Binary classification, metadata, and live process-name spelling.
    pub(crate) name: &'static str,
    /// Live process-name family beyond the exact binary: when `Some`, a live
    /// basename of `prefix` plus a version starting with an ASCII digit also
    /// belongs to this row. Only muse declares one (its launcher `muse`
    /// execs a versioned `muse-bin-<version>`); every other row is `None`
    /// and matches its exact binary only.
    pub(crate) process_family_prefix: Option<&'static str>,
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
    /// Whether one conversation can be carried to another account of this same
    /// tool by copying files.
    pub(crate) carry: CarrySpec,
    /// Post-launch conversation-id capture behaviour.
    pub(crate) capture: CaptureSpec,
    /// Input observation and first-turn delivery behaviour.
    pub(crate) input: InputSpec,
    /// The prompt only the human may answer, where one is measured.
    pub(crate) prompt: Option<PromptSpec>,
    /// The information panel that can stand where the composer was, where one
    /// is measured.
    pub(crate) panel: Option<PanelSpec>,
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
    /// How a profile command spells the reasoning effort, if ae reads it.
    pub(crate) effort: EffortSpec,
    /// Local quota discovery behaviour.
    pub(crate) quota: QuotaSpec,
    /// Local usage discovery behaviour.
    pub(crate) usage: UsageSpec,
    /// How the board reads this harness.
    pub(crate) board: BoardSpec,
}

const CLAUDE: ToolAdapter = ToolAdapter {
    kind: ToolKind::Claude,
    name: "claude",
    process_family_prefix: None,
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
        // A gone conversation starts fresh, never `--continue` into
        // another seat's newest.
        form: ResumeForm::ExactOnly { exact: "--resume" },
        probe: StoreProbe::ProjectTranscript,
        keep: KeepSpec::Measured {
            model_flags: &["--model"],
            effort: true,
        },
    },
    carry: CarrySpec::ProjectTranscript,
    capture: CaptureSpec::None,
    input: InputSpec {
        model: InputModel::BorderDelimited,
        composed: Composed::NONE,
        wait_for_process: true,
        paste_initial_on_resume: false,
    },
    // MEASURED on claude 2.1.281 (2026-09-24, private probe panes, 80x24 and
    // 200x50, bare and bypassPermissions alike) and on 2.1.306 (a 39x23 lead
    // pane): the folder-trust modal in a never-trusted directory
    // (`tests/fixtures/claude-trust/`). A full-width rule, "Accessing
    // workspace:", the path, then a question Ink wraps with its `?` mid-row,
    // and the selection `❯` on "No, exit".
    //
    // The window must hold the RULE, because the title is found under it:
    // rule to hint is 16 rows at 200 columns, 17 at 80, 21 at 39 (all three
    // measured); a greedy wrap to `columns - 1`, which reproduces those three
    // frames exactly, gives 19 at 60, 20 at 45-50, 21 at 39-44, 22 at 34-38,
    // 23 at 33 and 25 at 30-32. 25 is the smallest window that still holds the
    // whole modal at 30 columns; the old 20 cropped the rule below 45.
    // The trust block is answered only from 34 columns up, where the 33-cell
    // hint row still draws on ONE row. At 30-33 columns the prompt is NAMED
    // and never answered; so is a folder path longer than the row (Ink wraps
    // it onto two), while that one extra row still fits the 25-row window.
    // Below 30 columns the modal outgrows the window and is not promised.
    // A pane shorter than the modal scrolls its rule off the visible screen, so
    // the screen capture can miss it where the scrollback capture does not.
    prompt: Some(PromptSpec {
        question: Question::StartsWith("Quick safety check:"),
        title: Some("Accessing workspace:"),
        selected: '❯',
        keys: &["to confirm · "],
        window: 25,
        trust: Some(TrustSpec {
            heading: "Accessing workspace:",
            paragraphs: &[
                "Quick safety check: Is this a project you created or one you trust? (Like your own \
                 code, a well-known open source project, or work from your team). If not, take a \
                 moment to review what's in this folder first.",
                "Claude Code'll be able to read, edit, and execute files here.",
                "Security guide",
            ],
            options: &["No, exit", "Yes, I trust this folder"],
            yes: "Yes, I trust this folder",
            hint: "Enter to confirm · Esc to cancel",
            trailer: 0,
        }),
    }),
    panel: None,
    identity: IdentitySpec::BorderComposer,
    model: ModelSpec::ReportOnly(&["--model"]),
    pin_match: PinMatch::FamilyVersion,
    effort: EffortSpec::Flag {
        names: &["--effort"],
        vocabulary: &[
            Level::Low,
            Level::Medium,
            Level::High,
            Level::Xhigh,
            Level::Max,
        ],
        env: Some("CLAUDE_CODE_EFFORT_LEVEL"),
    },
    quota: QuotaSpec {
        source: QuotaSource::ClaudeCache,
        config_home_env: Some("CLAUDE_CONFIG_DIR"),
        default_home: Some(".claude"),
        unsupported_hint: None,
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::ClaudeTranscripts,
    },
    board: BoardSpec::Stream(BoardStream {
        parser: BoardParser::Claude,
    }),
    compact: CompactSpec::Guided {
        command: "/compact",
    },
};

const CODEX: ToolAdapter = ToolAdapter {
    kind: ToolKind::Codex,
    name: "codex",
    process_family_prefix: None,
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
            after: &["-c", "--config"],
        },
        probe: StoreProbe::DatedRollouts,
        keep: KeepSpec::Measured {
            model_flags: &["-m", "--model"],
            effort: true,
        },
    },
    carry: CarrySpec::NotPortable,
    capture: CaptureSpec::HandshakeRolloutOrTui,
    input: InputSpec {
        model: InputModel::StyleDelimited,
        composed: Composed::NONE,
        wait_for_process: true,
        paste_initial_on_resume: true,
    },
    prompt: None,
    // MEASURED on codex 0.163.0 (F2, or a click on its footer warning):
    // `tests/fixtures/codex-warning/`.
    panel: Some(PanelSpec {
        title: "Warnings",
        counted: ("Warnings · ", " · Startup"),
        hint: "k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll",
        window: 12,
        bodies: &[
            "Running without the shared background server: command-line configuration overrides (-c, --enable, --disable, or --search) requires embedded mode.",
            "This session was recorded with model `{}` but is resuming with `{}`. Consider switching back to `{}` as it may affect Codex performance.",
            "No warnings",
        ],
        slot_bytes: 256,
    }),
    identity: IdentitySpec::StyleFooter,
    model: ModelSpec::Replayable(&["-m", "--model"]),
    pin_match: PinMatch::Exact,
    effort: EffortSpec::ConfigPair {
        flags: &["-c", "--config"],
        key: "model_reasoning_effort",
        vocabulary: &Level::ALL,
    },
    quota: QuotaSpec {
        source: QuotaSource::CodexRollouts,
        config_home_env: Some("CODEX_HOME"),
        default_home: Some(".codex"),
        unsupported_hint: None,
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::CodexRollout,
    },
    board: BoardSpec::Stream(BoardStream {
        parser: BoardParser::Codex,
    }),
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const GEMINI: ToolAdapter = ToolAdapter {
    kind: ToolKind::Gemini,
    name: "gemini",
    process_family_prefix: None,
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
        // A pending seat starts fresh, never `--resume latest` into
        // another seat's newest.
        form: ResumeForm::ExactOnly { exact: "--resume" },
        probe: StoreProbe::RecordedId,
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
    capture: CaptureSpec::ChatHistory,
    input: InputSpec {
        model: InputModel::Unmodelled,
        composed: Composed::NONE,
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    prompt: None,
    panel: None,
    identity: IdentitySpec::Unmodelled,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".gemini"),
        unsupported_hint: Some("no verified local quota source"),
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Unsupported {
        reason: "gemini: out of scope",
    },
    compact: CompactSpec::Bare {
        command: "/compress",
    },
};

const AGY: ToolAdapter = ToolAdapter {
    kind: ToolKind::Agy,
    name: "agy",
    process_family_prefix: None,
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
        // A gone conversation starts fresh, never `--continue` into
        // another seat's newest.
        form: ResumeForm::StrippedExact {
            grammar: SessionFlags::Conversation,
            exact: "--conversation",
        },
        probe: StoreProbe::ConversationDatabase,
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
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
    // MEASURED on agy 1.2.6: `tests/fixtures/agy-composer/agy-trust-modal-frame.txt`.
    prompt: Some(PromptSpec {
        question: Question::EndsWith("?"),
        title: None,
        selected: '>',
        keys: &["enter Confirm", "↑/↓ Navigate"],
        window: 15,
        trust: Some(TrustSpec {
            heading: "Accessing workspace:",
            paragraphs: &[
                "Do you trust the contents of this project?",
                "Antigravity CLI requires permission to read, edit, and execute files here.",
            ],
            options: &["Yes, I trust this folder", "No, exit"],
            yes: "Yes, I trust this folder",
            hint: "↑/↓ Navigate · enter Confirm",
            trailer: 1,
        }),
    }),
    panel: None,
    identity: IdentitySpec::TrailingLabel,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".gemini/antigravity-cli"),
        unsupported_hint: Some("run agy -p \"/quota\""),
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Stream(BoardStream {
        parser: BoardParser::Agy,
    }),
    compact: CompactSpec::Unsupported {
        reason: "agy exposes no compaction command",
    },
};

const GROK: ToolAdapter = ToolAdapter {
    kind: ToolKind::Grok,
    name: "grok",
    process_family_prefix: None,
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
        // A pending seat starts fresh, never `--continue` into
        // another seat's newest.
        form: ResumeForm::StrippedExact {
            grammar: SessionFlags::ShortAliases,
            exact: "--resume",
        },
        probe: StoreProbe::RecordedId,
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
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
    prompt: None,
    panel: None,
    identity: IdentitySpec::BorderText,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::GrokLog,
        config_home_env: None,
        default_home: Some(".grok"),
        unsupported_hint: Some("run /usage in grok"),
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Stream(BoardStream {
        parser: BoardParser::Grok,
    }),
    compact: CompactSpec::Guided {
        command: "/compact",
    },
};

const MUSE: ToolAdapter = ToolAdapter {
    kind: ToolKind::Muse,
    name: "muse",
    process_family_prefix: Some("muse-bin-"),
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
            // Measured 2026-09-30: muse honors root options on either side.
            after: &[],
        },
        // A token-proven directory basename is the session id. Muse logs do
        // not put that id in their `session.jsonl` file name for a store probe.
        probe: StoreProbe::RecordedId,
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
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
    prompt: None,
    panel: None,
    identity: IdentitySpec::RuleFooter,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".config/muse"),
        unsupported_hint: Some("no verified local quota source"),
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Stream(BoardStream {
        parser: BoardParser::Muse,
    }),
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const OPENCODE: ToolAdapter = ToolAdapter {
    kind: ToolKind::OpenCode,
    name: "opencode",
    process_family_prefix: None,
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
        // #56: opencode's list is project-scoped by caller cwd, so `--continue`
        // is never provably the seat's own — an unproven seat starts fresh.
        form: ResumeForm::ExactOnly { exact: "--session" },
        probe: StoreProbe::RecordedId,
        // MEASURED on opencode 2.2.1 (2026-10-29, `.local/oc2/route3-results.txt`):
        // a configured model does not move a resumed session off the model it
        // stored, so a model change never keeps the conversation.
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
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
    prompt: None,
    panel: None,
    identity: IdentitySpec::RailStatus,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: Some(".local/share/opencode"),
        unsupported_hint: Some("local stats are cost history, not quota"),
        plan: Some(PlanRoute {
            model_flags: &["--model", "-m"],
            prefix: "opencode-go/",
        }),
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Export,
    compact: CompactSpec::Bare {
        command: "/compact",
    },
};

const UNKNOWN: ToolAdapter = ToolAdapter {
    kind: ToolKind::Unknown,
    name: "unknown",
    process_family_prefix: None,
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
        keep: KeepSpec::Unmeasured,
    },
    carry: CarrySpec::NotPortable,
    capture: CaptureSpec::None,
    input: InputSpec {
        model: InputModel::Unmodelled,
        composed: Composed::NONE,
        wait_for_process: false,
        paste_initial_on_resume: false,
    },
    prompt: None,
    panel: None,
    identity: IdentitySpec::Unmodelled,
    model: ModelSpec::Unobserved,
    pin_match: PinMatch::Exact,
    effort: EffortSpec::Unsupported,
    quota: QuotaSpec {
        source: QuotaSource::Unsupported,
        config_home_env: None,
        default_home: None,
        unsupported_hint: Some("no local quota adapter"),
        plan: None,
    },
    usage: UsageSpec {
        source: UsageSource::Unsupported,
    },
    board: BoardSpec::Unsupported {
        reason: "unknown tool: out of scope",
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

/// Every measured info panel a known adapter row declares.
pub(crate) fn panels() -> impl Iterator<Item = PanelSpec> {
    KNOWN.iter().filter_map(|adapter| adapter.panel)
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

impl ToolAdapter {
    /// Whether a stripped live basename belongs to this row: its exact
    /// binary, or its declared versioned family — the prefix plus a version
    /// whose first byte is an ASCII digit, which is what excludes
    /// `muse-binx` while surviving a truncated comm. THE ONE family rule,
    /// read by [`ToolKind::from_known_binary_name`] and by
    /// [`crate::procs::name_matches`]; a row declaring no prefix is
    /// byte-identical to a bare `==`.
    pub(crate) fn process_name_matches(&self, base: &str) -> bool {
        if self.name == base {
            return true;
        }
        self.process_family_prefix.is_some_and(|prefix| {
            base.strip_prefix(prefix)
                .is_some_and(|rest| rest.as_bytes().first().is_some_and(u8::is_ascii_digit))
        })
    }
}

impl ToolKind {
    /// Classify one known bare binary name, preserving absence as `None`.
    #[must_use]
    pub(crate) fn from_known_binary_name(name: &str) -> Option<Self> {
        KNOWN
            .iter()
            .find(|adapter| adapter.process_name_matches(name))
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

    /// Whether this tool's quota is asked of the tool itself, not read from a file.
    #[must_use]
    pub const fn quota_is_asked(self) -> bool {
        matches!(self, Self::Agy)
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

    /// Whether this harness persists multi-line ae turns inside a paste
    /// wrapper: only Claude's store shape is measured (#171). Display-only —
    /// the board's central filter consults it before its wrapper recognizer.
    #[must_use]
    pub(crate) const fn board_has_paste_wrapper(self) -> bool {
        matches!(self, Self::Claude)
    }

    /// Whether `row` is this harness's row for a usage-limit wait the human
    /// cancelled: only claude 2.1.296's is measured.
    #[must_use]
    pub(crate) fn limit_wait_cancelled(self, row: &str) -> bool {
        matches!(self, Self::Claude)
            && row == "⏺ Automatic continue cancelled · /rate-limit-options to re-arm"
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
    fn only_muse_declares_a_versioned_live_process_family() {
        // MEASURED 2026-09-28 (#206): the launcher `muse` execs
        // `muse-bin-<version>`, so the versioned basename classifies muse.
        assert_eq!(
            ToolKind::from_known_binary_name("muse-bin-1.4.0-R4302.1"),
            Some(ToolKind::Muse)
        );
        for near in ["musecode", "muse-binx", "muse-bin-", "muse-binary"] {
            assert_eq!(ToolKind::from_known_binary_name(near), None, "{near:?}");
        }
        assert_eq!(
            ToolKind::Muse.adapter().process_family_prefix,
            Some("muse-bin-")
        );
        for tool in [
            ToolKind::Claude,
            ToolKind::Codex,
            ToolKind::Gemini,
            ToolKind::Agy,
            ToolKind::Grok,
            ToolKind::OpenCode,
            ToolKind::Unknown,
        ] {
            assert_eq!(tool.adapter().process_family_prefix, None, "{tool:?}");
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
                    process_family_prefix: None,
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
                        // A gone conversation starts fresh, never
                        // `--continue` into another seat's newest.
                        form: ResumeForm::ExactOnly { exact: "--resume" },
                        probe: StoreProbe::ProjectTranscript,
                        keep: KeepSpec::Measured {
                            model_flags: &["--model"],
                            effort: true
                        },
                    },
                    carry: CarrySpec::ProjectTranscript,
                    capture: CaptureSpec::None,
                    input: InputSpec {
                        model: InputModel::BorderDelimited,
                        composed: Composed::NONE,
                        wait_for_process: true,
                        paste_initial_on_resume: false,
                    },
                    prompt: Some(PromptSpec {
                        question: Question::StartsWith("Quick safety check:"),
                        title: Some("Accessing workspace:"),
                        selected: '❯',
                        keys: &["to confirm · "],
                        window: 25,
                        trust: Some(TrustSpec {
                            heading: "Accessing workspace:",
                            paragraphs: &[
                                "Quick safety check: Is this a project you created or one you \
                                 trust? (Like your own code, a well-known open source project, \
                                 or work from your team). If not, take a moment to review \
                                 what's in this folder first.",
                                "Claude Code'll be able to read, edit, and execute files here.",
                                "Security guide",
                            ],
                            options: &["No, exit", "Yes, I trust this folder"],
                            yes: "Yes, I trust this folder",
                            hint: "Enter to confirm · Esc to cancel",
                            trailer: 0,
                        }),
                    }),
                    panel: None,
                    identity: IdentitySpec::BorderComposer,
                    model: ModelSpec::ReportOnly(&["--model"]),
                    pin_match: PinMatch::FamilyVersion,
                    effort: EffortSpec::Flag {
                        names: &["--effort"],
                        vocabulary: &[
                            Level::Low,
                            Level::Medium,
                            Level::High,
                            Level::Xhigh,
                            Level::Max
                        ],
                        env: Some("CLAUDE_CODE_EFFORT_LEVEL"),
                    },
                    quota: QuotaSpec {
                        source: QuotaSource::ClaudeCache,
                        config_home_env: Some("CLAUDE_CONFIG_DIR"),
                        default_home: Some(".claude"),
                        unsupported_hint: None,
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::ClaudeTranscripts,
                    },
                    board: BoardSpec::Stream(BoardStream {
                        parser: BoardParser::Claude,
                    }),
                    compact: CompactSpec::Guided {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Codex,
                    name: "codex",
                    process_family_prefix: None,
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
                            after: &["-c", "--config"],
                        },
                        probe: StoreProbe::DatedRollouts,
                        keep: KeepSpec::Measured {
                            model_flags: &["-m", "--model"],
                            effort: true
                        },
                    },
                    carry: CarrySpec::NotPortable,
                    capture: CaptureSpec::HandshakeRolloutOrTui,
                    input: InputSpec {
                        model: InputModel::StyleDelimited,
                        composed: Composed::NONE,
                        wait_for_process: true,
                        paste_initial_on_resume: true,
                    },
                    prompt: None,
                    panel: Some(PanelSpec {
                        title: "Warnings",
                        counted: ("Warnings · ", " · Startup"),
                        hint: "k keep & next · esc dismiss & close · ⌃o copy · ←/→ warning · ↓ scroll",
                        window: 12,
                        bodies: &[
                            "Running without the shared background server: command-line configuration overrides (-c, --enable, --disable, or --search) requires embedded mode.",
                            "This session was recorded with model `{}` but is resuming with `{}`. Consider switching back to `{}` as it may affect Codex performance.",
                            "No warnings",
                        ],
                        slot_bytes: 256,
                    }),
                    identity: IdentitySpec::StyleFooter,
                    model: ModelSpec::Replayable(&["-m", "--model"]),
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::ConfigPair {
                        flags: &["-c", "--config"],
                        key: "model_reasoning_effort",
                        vocabulary: &Level::ALL,
                    },
                    quota: QuotaSpec {
                        source: QuotaSource::CodexRollouts,
                        config_home_env: Some("CODEX_HOME"),
                        default_home: Some(".codex"),
                        unsupported_hint: None,
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::CodexRollout,
                    },
                    board: BoardSpec::Stream(BoardStream {
                        parser: BoardParser::Codex,
                    }),
                    compact: CompactSpec::Bare {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Gemini,
                    name: "gemini",
                    process_family_prefix: None,
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
                        // A pending seat starts fresh, never
                        // `--resume latest` into another seat's newest.
                        form: ResumeForm::ExactOnly { exact: "--resume" },
                        probe: StoreProbe::RecordedId,
                        keep: KeepSpec::Unmeasured,
                    },
                    carry: CarrySpec::NotPortable,
                    capture: CaptureSpec::ChatHistory,
                    input: InputSpec {
                        model: InputModel::Unmodelled,
                        composed: Composed::NONE,
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    prompt: None,
                    panel: None,
                    identity: IdentitySpec::Unmodelled,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::Unsupported,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".gemini"),
                        unsupported_hint: Some("no verified local quota source"),
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    board: BoardSpec::Unsupported {
                        reason: "gemini: out of scope",
                    },
                    compact: CompactSpec::Bare {
                        command: "/compress"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Agy,
                    name: "agy",
                    process_family_prefix: None,
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
                        form: ResumeForm::StrippedExact {
                            grammar: SessionFlags::Conversation,
                            exact: "--conversation",
                        },
                        probe: StoreProbe::ConversationDatabase,
                        keep: KeepSpec::Unmeasured,
                    },
                    carry: CarrySpec::NotPortable,
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
                    prompt: Some(PromptSpec {
                        question: Question::EndsWith("?"),
                        title: None,
                        selected: '>',
                        keys: &["enter Confirm", "↑/↓ Navigate"],
                        window: 15,
                        trust: Some(TrustSpec {
                            heading: "Accessing workspace:",
                            paragraphs: &[
                                "Do you trust the contents of this project?",
                                "Antigravity CLI requires permission to read, edit, and \
                                 execute files here.",
                            ],
                            options: &["Yes, I trust this folder", "No, exit"],
                            yes: "Yes, I trust this folder",
                            hint: "↑/↓ Navigate · enter Confirm",
                            trailer: 1,
                        }),
                    }),
                    panel: None,
                    identity: IdentitySpec::TrailingLabel,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::Unsupported,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".gemini/antigravity-cli"),
                        unsupported_hint: Some("run agy -p \"/quota\""),
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    board: BoardSpec::Stream(BoardStream {
                        parser: BoardParser::Agy,
                    }),
                    compact: CompactSpec::Unsupported {
                        reason: "agy exposes no compaction command",
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Grok,
                    name: "grok",
                    process_family_prefix: None,
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
                        form: ResumeForm::StrippedExact {
                            grammar: SessionFlags::ShortAliases,
                            exact: "--resume",
                        },
                        probe: StoreProbe::RecordedId,
                        keep: KeepSpec::Unmeasured,
                    },
                    carry: CarrySpec::NotPortable,
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
                    prompt: None,
                    panel: None,
                    identity: IdentitySpec::BorderText,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::Unsupported,
                    quota: QuotaSpec {
                        source: QuotaSource::GrokLog,
                        config_home_env: None,
                        default_home: Some(".grok"),
                        unsupported_hint: Some("run /usage in grok"),
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    board: BoardSpec::Stream(BoardStream {
                        parser: BoardParser::Grok,
                    }),
                    compact: CompactSpec::Guided {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::Muse,
                    name: "muse",
                    process_family_prefix: Some("muse-bin-"),
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
                            after: &[],
                        },
                        probe: StoreProbe::RecordedId,
                        keep: KeepSpec::Unmeasured,
                    },
                    carry: CarrySpec::NotPortable,
                    capture: CaptureSpec::MuseDatedSessions,
                    input: InputSpec {
                        model: InputModel::BorderDelimited,
                        composed: Composed::NONE,
                        wait_for_process: false,
                        paste_initial_on_resume: false,
                    },
                    prompt: None,
                    panel: None,
                    identity: IdentitySpec::RuleFooter,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::Unsupported,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".config/muse"),
                        unsupported_hint: Some("no verified local quota source"),
                        plan: None,
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    board: BoardSpec::Stream(BoardStream {
                        parser: BoardParser::Muse,
                    }),
                    compact: CompactSpec::Bare {
                        command: "/compact"
                    },
                },
                ToolAdapter {
                    kind: ToolKind::OpenCode,
                    name: "opencode",
                    process_family_prefix: None,
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
                        form: ResumeForm::ExactOnly { exact: "--session" },
                        probe: StoreProbe::RecordedId,
                        keep: KeepSpec::Unmeasured,
                    },
                    carry: CarrySpec::NotPortable,
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
                    prompt: None,
                    panel: None,
                    identity: IdentitySpec::RailStatus,
                    model: ModelSpec::Unobserved,
                    pin_match: PinMatch::Exact,
                    effort: EffortSpec::Unsupported,
                    quota: QuotaSpec {
                        source: QuotaSource::Unsupported,
                        config_home_env: None,
                        default_home: Some(".local/share/opencode"),
                        unsupported_hint: Some("local stats are cost history, not quota"),
                        plan: Some(PlanRoute {
                            model_flags: &["--model", "-m"],
                            prefix: "opencode-go/",
                        }),
                    },
                    usage: UsageSpec {
                        source: UsageSource::Unsupported,
                    },
                    board: BoardSpec::Export,
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
                process_family_prefix: None,
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
                    keep: KeepSpec::Unmeasured,
                },
                carry: CarrySpec::NotPortable,
                capture: CaptureSpec::None,
                input: InputSpec {
                    model: InputModel::Unmodelled,
                    composed: Composed::NONE,
                    wait_for_process: false,
                    paste_initial_on_resume: false,
                },
                prompt: None,
                panel: None,
                identity: IdentitySpec::Unmodelled,
                model: ModelSpec::Unobserved,
                pin_match: PinMatch::Exact,
                effort: EffortSpec::Unsupported,
                quota: QuotaSpec {
                    source: QuotaSource::Unsupported,
                    config_home_env: None,
                    default_home: None,
                    unsupported_hint: Some("no local quota adapter"),
                    plan: None,
                },
                usage: UsageSpec {
                    source: UsageSource::Unsupported,
                },
                board: BoardSpec::Unsupported {
                    reason: "unknown tool: out of scope",
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
