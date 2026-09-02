//! Argv parsing: the one place that turns a command line into an intent.
//!
//! Deliberately hand-rolled. A CLI argument parser is a dependency the skeleton
//! does not need, and #80's rule for the error crate — "no dependency exists
//! until a real error does" — applies here too. When the real command surface
//! arrives (P1+), revisit with a measured need.

use std::path::PathBuf;

use crate::filters::{ListArgs, UnknownFlag};
use crate::requests::Mode;

/// The subcommand that carries the `requests` helper surface.
///
/// **The leading underscore is the whole argument for this spelling** (lead
/// ruling on D1, 2026-08-24). `_validate_session_name` forbids a leading `_`, so
/// an underscore-prefixed subcommand can never shadow a legal session name and
/// SC-022's "a top-level bare word is a launch candidate" loses nothing to it.
/// A `requests` spelling would have taken a real name out of the launch grammar;
/// a `--session` flag would have invented an option no row names. `ae` already
/// spells its internal surfaces this way — `_register-sid`, `_recover-pending`.
pub const REQUESTS: &str = "_requests";

/// The subcommand that carries the `events-tail` helper surface. Same
/// underscore reasoning as [`REQUESTS`].
pub const EVENTS_TAIL: &str = "_events-tail";

/// `ae archive preview`'s successor: `_archive-preview <session-dir>`. Unlike
/// the helper surfaces this is a TOP-LEVEL command's core entry — `ae` resolves
/// and path-checks the session name, then shims here with the resolved dir. The
/// underscore keeps it off the human-typed surface and out of session-name
/// space (`_validate_session_name` forbids a leading `_`).
pub const ARCHIVE_PREVIEW: &str = "_archive-preview";

/// `ae end`'s archive publisher: `_archive-publish <session-dir> <push-outcome>
/// <push-ref> <preserved> <workdir> <archived-at>`. The frozen `_end_archive_step`
/// shims here in place of `_ar_publish`; the five operands after the dir are the
/// operation facts Bash owns (the core derives the commit facts itself). Like
/// [`ARCHIVE_PREVIEW`] it is underscored — a core entry, never human-typed.
pub const ARCHIVE_PUBLISH: &str = "_archive-publish";

/// `--from`'s read-only preflight: `_archive-from-preflight <archive-root>
/// <raw-uuid>`. Bash routes the frozen `_ar_from_preflight` here before any launch
/// side effect; the core proves the archive and prints `aid\thandover\tpending`.
/// Underscored — a core entry, never human-typed.
pub const ARCHIVE_FROM_PREFLIGHT: &str = "_archive-from-preflight";

/// `--purge-history`'s archive deletion: `_archive-purge <session-dir> <aid>
/// <source-session> <parent-id>`. Bash routes the frozen `_ar_purge_archive`
/// here; the core proves ownership, then deletes under the shared claim and
/// makes the deletion durable before it reports success. Underscored — a core
/// entry, never human-typed.
pub const ARCHIVE_PURGE: &str = "_archive-purge";

/// Local-mode canonical session-state removal: `_end-local-teardown
/// <session-dir>`. Bash routes here from `cleanup_session` for a `mode=local`
/// session whose name is grammar-valid; the core renames the dir to a sibling
/// tombstone and durably removes it. The session dir is `_ae_core_try`'s injected
/// meta directory. Underscored — a core entry, never human-typed.
pub const END_LOCAL_TEARDOWN: &str = "_end-local-teardown";

/// Nonlocal canonical + workdir teardown: `_end-nonlocal-teardown <session-dir>
/// [--preserve]`. Bash routes here from `cleanup_session` for a `mode=full`
/// (copy) or `mode=git` (worktree) session; the core removes the managed workdir
/// (copy tombstone, or a sealed `git worktree remove`) and then the canonical
/// state. `--preserve` keeps the workdir byte-for-byte and removes canonical state
/// only. Underscored — a core entry, never human-typed.
pub const END_NONLOCAL_TEARDOWN: &str = "_end-nonlocal-teardown";

/// compact's freeze/resolve step: `_compact-freeze <session-dir> [--keep-history]`.
/// The pinned core resolves the session's frozen tuple (identity, mode, origin,
/// config-derived roster and purge policy, archive path) BEFORE anything is messaged
/// or archived, and emits it. `--keep-history` overrides a config that opts into
/// purging agent history. Pure read-only. Underscored — a core entry, never
/// human-typed.
pub const COMPACT_FREEZE: &str = "_compact-freeze";

/// compact's authorization gate: `_compact-revalidate <dir> <tuple> [--when <label>]
/// [--keep-history]`. Proves the live session is still the one the freeze authorized, at both
/// the after-confirmation and after-handover-wait crossings. Pure read-only. Underscored — a
/// core entry, never typed.
pub const COMPACT_REVALIDATE: &str = "_compact-revalidate";

/// The gate label when the driver passes no `--when` — the single-gate wording the entry had
/// before the two crossings were distinguished, kept so a caller that omits the flag is not
/// silently mislabelled.
pub const DEFAULT_REVALIDATE_WHEN: &str = "before the handover";

/// compact's destructive stage 1: `_compact-archive <dir> <tuple> <archived-at>
/// <push-outcome> <push-ref> <preserved> <workdir> [--keep-history]`. Revalidates the
/// frozen authorization, proves the session stopped on its recorded server, makes the
/// archive durable (publishing, or reusing an equivalent existing one), and prints the
/// recovery command — before any teardown. Underscored — a core entry, never typed.
pub const COMPACT_ARCHIVE: &str = "_compact-archive";

/// compact's destructive stage 2: `_compact-teardown <dir> <tuple> [--keep-history]`.
/// Re-proves the authorization and the stop, requires the durable archive, removes the
/// live session, and prints the exec plan. Underscored — a core entry, never typed.
pub const COMPACT_TEARDOWN: &str = "_compact-teardown";

/// compact's semantic-handover wait: `_compact-wait <dir> <ref> [--timeout <secs>]`.
/// Bounded poll for BOTH the reply and a new handover memo; on timeout it leaves the
/// request as found (only `--digest-only` withdraws). Underscored — a core entry, never
/// typed.
pub const COMPACT_WAIT: &str = "_compact-wait";

/// compact's `--digest-only` withdrawal: `_compact-cancel <dir> <ref>`. Emits the
/// Rust-owned slotless cancel event that withdraws an outstanding compact handover request.
/// Underscored — a core entry, never typed.
pub const COMPACT_CANCEL: &str = "_compact-cancel";

/// compact's pre-delivery memo boundary: `_compact-memo-baseline <dir>`. Prints the current
/// `memo.tsv` byte size — the offset a handover memo must be appended past. Underscored — a
/// core entry, never typed.
pub const COMPACT_MEMO_BASELINE: &str = "_compact-memo-baseline";

/// compact's retry-reuse lookup: `_compact-find-outstanding <dir>`. Prints the ref of a
/// still-pending compact handover request, so a re-run reuses it instead of delivering a
/// duplicate. Underscored — a core entry, never typed.
pub const COMPACT_FIND_OUTSTANDING: &str = "_compact-find-outstanding";

/// The `state` helper's surface — `_state <meta-dir> [<value> [reason…]]`.
/// Underscored like [`REQUESTS`]: launched by the generated helper, not typed.
pub const STATE: &str = "_state";

/// The `goal` helper's surface — `_goal <meta-dir> [<text…>|--clear|--help]`.
pub const GOAL: &str = "_goal";

/// The `memo` helper's surface —
/// `_memo <meta-dir> [add [--topic <t>] <text…>|read [--topic <t>]|tail [n]]`.
pub const MEMO: &str = "_memo";

/// The `ask` helper's surface — `_ask <meta-dir> <target> <question…>`.
pub const ASK: &str = "_ask";

/// The `review` helper's surface — `_review <meta-dir> <target> <request…>`.
pub const REVIEW: &str = "_review";

/// The `reply` helper's surface — `_reply <meta-dir> [--as <agent>] <request-id> <message…>`.
pub const REPLY: &str = "_reply";

/// The `send` helper's surface — `_send <meta-dir> <target> <message…>`.
pub const SEND: &str = "_send";

/// The watchdog daemon's surface — `_watchdog-run <meta-dir> [knob flags]`.
///
/// Underscored like every other core entry, and never human-typed: the bash
/// `watchdog` helper's `_run` execs this. EVERY tunable arrives as a FLAG
/// because this crate's clippy policy disallows reading the environment — bash
/// reads `AE_WATCHDOG_*` (with its `AE_LOOP_*` fallback) and passes what it
/// read, so the names are spelled in exactly one place and it is the frozen
/// script.
pub const WATCHDOG_RUN: &str = "_watchdog-run";

/// The telegram bridge daemon's surface —
/// `_telegram-run <ae-home> [--config <p>] [--home <p>] [knob flags]`.
///
/// Underscored like every other core entry, and never human-typed: the bash
/// `telegram` glue execs this. The PATHS are arguments for the same reason the
/// watchdog's knobs are — this crate's clippy policy disallows reading the
/// environment, so `AE_HOME` is spelled in exactly one place and it is the
/// frozen script.
pub const TELEGRAM_RUN: &str = "_telegram-run";

/// The identity v2 launch resolver: `_launch-plan [--global <f>] [--local <f>]
/// [--main <name>] [--workers <a,b>]`. The core reads `[profiles]`/`[roster]`/
/// `[workspace]`, validates the whole workspace, and prints the seats a launch
/// creates. Underscored — a core entry, never human-typed.
pub const LAUNCH_PLAN: &str = "_launch-plan";

/// The identity v2 first-meta publisher: `_meta-init <dir> --base <file>
/// [--replace]`. Bash stages the base facts in a file and pipes the seat records
/// on stdin; the core concatenates the roster block and publishes ONE document.
/// Underscored — a core entry, never human-typed.
pub const META_INIT: &str = "_meta-init";

/// The identity v2 roster surface: `_roster <dir> <add-seat|remove-seat|
/// set-harness-session|migrate|list> …`. Every write happens under the meta
/// lock. Underscored — a core entry, never human-typed.
pub const ROSTER: &str = "_roster";

/// The musl DNS/NSS instrument — `_net-probe <host> [--port <n>]`.
///
/// Underscored like every other core entry, and for the same reason: it is run
/// by a CI step (and, later, a `doctor` check), never typed at a session. Its
/// authority is the pre-P5 coexistence design
/// (`docs/migration/coexistence.md`, item 4) rather than a semantic-contract
/// row — there is no row because there is no bash predecessor: nothing in the
/// frozen script ever resolved a hostname. The behavior is [`crate::netprobe`].
pub const NET_PROBE: &str = "_net-probe";

/// What an argv asks the binary to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Print the version line.
    Version,
    /// Print usage.
    Help,
    /// `list` (or its `ls` spelling) with the filters its flags selected.
    List(ListArgs),
    /// `_requests <meta-dir> [mine|inbox|all]` — the `requests` helper surface.
    Requests {
        /// The session meta directory the frozen helper derives from `$0`.
        dir: PathBuf,
        /// SC-212c's mode, defaulting to `mine`.
        mode: Mode,
    },
    /// `_events-tail <meta-dir>` — the `events-tail` helper surface.
    /// `_state <meta-dir> [<value> [reason…]]` — the `state` helper. The tail
    /// is validated by [`crate::state::parse`], not here: the usage text is the
    /// helper's own and belongs beside the rule it states.
    State {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_goal <meta-dir> [<text…>|--clear|--help]` — validated by
    /// [`crate::goal::parse`].
    Goal {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_memo <meta-dir> [add …|read [--topic <t>]|tail [n]]` — validated by
    /// [`crate::memo::parse`].
    Memo {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_ask <meta-dir> <target> <question…>` — validated by
    /// [`crate::tracked::parse`].
    Ask {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_review <meta-dir> <target> <request…>` — validated by
    /// [`crate::tracked::parse`].
    Review {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_reply <meta-dir> [--as <agent>] <request-id> <message…>` — validated
    /// by [`crate::reply::parse`].
    Reply {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_send <meta-dir> <target> <message…>` — validated by
    /// [`crate::send::parse`].
    Send {
        /// The session meta directory.
        dir: std::path::PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    EventsTail {
        /// The session meta directory the frozen helper derives from `$0`.
        dir: PathBuf,
    },
    /// `_archive-preview <session-dir>` — the read-only archive preview tracer.
    ArchivePreview {
        /// The session directory `ae` resolved and path-checked before shimming.
        dir: PathBuf,
    },
    /// `_archive-publish <session-dir> <push-outcome> <push-ref> <preserved>
    /// <workdir> <archived-at>` — publishes the archive under its atomic claim.
    ArchivePublish {
        /// The session directory `ae` resolved and path-checked before shimming.
        dir: PathBuf,
        /// The git push outcome Bash recorded (`not-managed` when unmanaged).
        push_outcome: String,
        /// The pushed ref, or `-`.
        push_ref: String,
        /// The preserved work dir, or `-`.
        preserved: String,
        /// The work dir whose HEAD/range the core derives, or `-`.
        workdir: String,
        /// The archive instant Bash captured (`date -u`), validated by the core.
        archived_at: String,
    },
    /// `_archive-from-preflight <archive-root> <raw-uuid>` — the read-only
    /// `--from` preflight; prints `aid\thandover\tpending` or a named refusal.
    ArchiveFromPreflight {
        /// The archive root `ae` derived (`$AE_HOME/archive`).
        root: PathBuf,
        /// The `--from` argument, canonicalised and validated by the core.
        raw_uuid: String,
    },
    /// `_archive-purge <session-dir> <aid> <source-session> <parent-id>` —
    /// deletes this session's archive under the shared claim, durably.
    ArchivePurge {
        /// The session directory `ae` resolved and path-checked before shimming.
        dir: PathBuf,
        /// The canonical archive id to purge.
        aid: String,
        /// The exact session name that must own the archive (never a wildcard).
        source_session: String,
        /// The parent archive id this session launched from, or `-`.
        parent_id: String,
    },
    /// `_end-local-teardown <session-dir>` — removes the canonical session state
    /// of a local-mode session via the rename-to-tombstone commit boundary.
    EndLocalTeardown {
        /// The session directory (`_ae_core_try`'s injected meta dir). The core
        /// derives the name and sessions root from it and validates both.
        dir: PathBuf,
    },
    /// `_watchdog-run <meta-dir> [knob flags]` — run the watchdog daemon for a
    /// session until its session or its state goes away.
    WatchdogRun {
        /// The session's meta directory.
        dir: PathBuf,
        /// Every tunable, defaulted to the frozen values a flagless call keeps.
        knobs: crate::watchdog_daemon::Knobs,
    },
    /// `_telegram-run <ae-home> [--config <p>] [--home <p>] [knob flags]` — run
    /// the Telegram bridge: inbound long poll plus outbound `say` forwarding.
    TelegramRun {
        /// Where the bridge reads its config, its secrets and its sessions.
        paths: crate::telegram::bridge::Paths,
        /// Every tunable, defaulted to the values a flagless call keeps.
        knobs: crate::telegram::bridge::Knobs,
    },
    /// `_launch-plan [--global <f>] [--local <f>] [--main <name>]
    /// [--workers <a,b>]` — validated by [`crate::identity::launch_plan`],
    /// which owns the flag grammar and its usage text.
    LaunchPlan {
        /// Everything after the subcommand, as typed.
        tail: Vec<String>,
    },
    /// `_meta-init <dir> --base <file> [--replace]` — validated by
    /// [`crate::identity::meta_init`]. The seat records arrive on stdin.
    MetaInit {
        /// The session meta directory.
        dir: PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_roster <dir> <subcommand> …` — validated by
    /// [`crate::identity::roster`].
    Roster {
        /// The session meta directory.
        dir: PathBuf,
        /// Everything after it, as typed.
        tail: Vec<String>,
    },
    /// `_net-probe <host> [--port <n>]` — resolve a name and report what the
    /// resolver did.
    NetProbe {
        /// The host to resolve. A NAME is the whole point: an IP literal is
        /// answered without a resolver ever being asked.
        host: String,
        /// The port the resolved authority carries, defaulted to
        /// [`crate::netprobe::DEFAULT_PORT`].
        port: u16,
    },
    /// `_compact-freeze <session-dir> [--keep-history]` — resolves and emits the
    /// frozen compact tuple. Pure read-only.
    CompactFreeze {
        /// The session directory the frozen helper derives from `$0`.
        dir: PathBuf,
        /// `--keep-history`: override a config that opts into purging agent history.
        keep_history: bool,
    },
    /// `_compact-revalidate <dir> <tuple> [--when <label>] [--keep-history]` — the
    /// authorization gate, crossed after confirmation AND after the handover wait.
    CompactRevalidate {
        /// The session directory.
        dir: PathBuf,
        /// The frozen tuple from `_compact-freeze`.
        tuple: String,
        /// The driver's label for WHICH gate this is (e.g. "after confirmation", "after the
        /// handover wait"), so a refusal names the crossing. Defaults to "before the handover"
        /// when the flag is absent, preserving the single-gate wording for any legacy caller.
        when: String,
        /// `--keep-history`, carried from the original invocation.
        keep_history: bool,
    },
    /// `_compact-archive <dir> <tuple> <archived-at> <push-outcome> <push-ref>
    /// <preserved> <workdir> [--keep-history]` — the first destructive stage: durable
    /// archive + recovery print, after proving the session stopped.
    CompactArchive {
        /// The session directory.
        dir: PathBuf,
        /// The frozen tuple (`0x1f`-separated) from `_compact-freeze` — the authorization.
        tuple: String,
        /// The ISO-8601 UTC instant bash supplies (std cannot format one).
        archived_at: String,
        /// Git push outcome, `-` for a local compact.
        push_outcome: String,
        /// Git push ref, `-` for a local compact.
        push_ref: String,
        /// Preserved marker, `-` for a local compact.
        preserved: String,
        /// The recorded workdir, for the archive's git facts.
        workdir: String,
        /// `--keep-history`: carried from the original invocation so revalidation's
        /// purge-flip check agrees with what was authorized.
        keep_history: bool,
    },
    /// `_compact-teardown <dir> <tuple> [--keep-history]` — the second destructive
    /// stage: remove the live session and print the exec plan, once the archive is durable.
    CompactTeardown {
        /// The session directory.
        dir: PathBuf,
        /// The frozen tuple — re-validated before teardown.
        tuple: String,
        /// `--keep-history`, as [`Request::CompactArchive`].
        keep_history: bool,
    },
    /// `_compact-wait <dir> <ref> [--timeout <secs>]` — the bounded semantic-handover wait.
    CompactWait {
        /// The session directory.
        dir: PathBuf,
        /// The tracked handover request id to wait on.
        reference: String,
        /// The bound in seconds; [`crate::compact::DEFAULT_HANDOVER_SECS`] when `--timeout`
        /// is unset.
        timeout_secs: u64,
    },
    /// `_compact-cancel <dir> <ref>` — withdraw an outstanding compact handover request.
    CompactCancel {
        /// The session directory.
        dir: PathBuf,
        /// The tracked handover request id to withdraw.
        reference: String,
    },
    /// `_compact-memo-baseline <dir>` — print the current `memo.tsv` byte size.
    CompactMemoBaseline {
        /// The session directory.
        dir: PathBuf,
    },
    /// `_compact-find-outstanding <dir>` — print a still-pending compact handover ref.
    CompactFindOutstanding {
        /// The session directory.
        dir: PathBuf,
    },
    /// `_end-nonlocal-teardown <session-dir> [--preserve]` — removes the managed
    /// workdir (copy/worktree) and then the canonical state of a nonlocal session.
    EndNonlocalTeardown {
        /// The session directory (`_ae_core_try`'s injected meta dir). The core
        /// derives the name and both roots from the configured state root and
        /// validates the dir is that exact managed child.
        dir: PathBuf,
        /// `--preserve`: keep the workdir byte-for-byte (a no-origin session whose
        /// committed work lives only there), removing canonical state only.
        preserve: bool,
    },
    /// A usage error about a MISSING operand rather than an offending token.
    ///
    /// The successor spelling's own error class: the frozen helpers read their
    /// meta directory from `$0`, so "you gave me no directory" is a state they
    /// cannot be in and no row rules. It exits `2` like every other usage error,
    /// and it is a separate variant because [`Request::UsageError`] promises to
    /// carry the offending token verbatim — and an absence is not a token.
    MissingOperand(&'static str),
    /// **SC-022** — a token that is a usage error: an unknown top-level OPTION,
    /// or an unknown token in a `list`/`ls` tail. Carries the token verbatim.
    UsageError(String),
    /// A top-level NON-option token: a session name under the start grammar.
    ///
    /// **SC-022 rules this is NEVER an unknown-subcommand error.** It is a
    /// launch candidate, and launching is not this slice's work — so the
    /// variant exists to keep the misclassification impossible rather than to
    /// implement anything. There is deliberately no "unknown command" phrase in
    /// this crate for such a token to fall into.
    LaunchCandidate(String),
}

impl Request {
    /// Classify `args` — argv WITHOUT the program name.
    ///
    /// An empty argv is [`Request::Help`]: a multiplexer that prints nothing
    /// when invoked bare is a multiplexer nobody can discover.
    ///
    /// # `list` / `ls`
    ///
    /// The flag grammar is **not** repeated here — it is
    /// [`ListArgs::parse`](crate::filters::ListArgs::parse), which owns
    /// SC-017a–f, SC-017i and SC-521a/b. This function only decides that the
    /// word `list` (or `ls`) hands the REST of argv to that parser, so there is
    /// exactly one place where a flag means something.
    ///
    /// **SC-021** — `ls` is an alias of `list`, so the two are one command with
    /// two spellings rather than two commands that happen to agree. The row's
    /// authority is the S1 surface INVENTORY, not commands.md, where `ls`
    /// appears nowhere: the row records that documentation gap as its own
    /// finding.
    ///
    /// # SC-022 — the two kinds of unrecognised token
    ///
    /// A token `list` does not know is a [`Request::UsageError`]. So is an
    /// unknown top-level OPTION — a `-`/`--` token the dispatcher does not
    /// define. A top-level token that is NOT option-shaped is neither: it is a
    /// session name under the start grammar, and becomes a
    /// [`Request::LaunchCandidate`]. The row rules that direction explicitly, so
    /// the shape of this function is the shape of the row.
    ///
    /// ```
    /// use ae::cli::Request;
    /// use ae::filters::{ListArgs, Scope};
    ///
    /// assert_eq!(Request::parse(&[]), Request::Help);
    /// assert_eq!(Request::parse(&["-V".to_owned()]), Request::Version);
    ///
    /// let Request::List(args) = Request::parse(&["ls".to_owned(), "--all".to_owned()]) else {
    ///     panic!("ls is the list command");
    /// };
    /// assert_eq!(args.selection.scope, Scope::All);
    /// assert_eq!(Request::parse(&["list".to_owned()]), Request::List(ListArgs::default()));
    ///
    /// // SC-022: option-shaped is a usage error; a bare word is a session name.
    /// assert_eq!(
    ///     Request::parse(&["--frobnicate".to_owned()]),
    ///     Request::UsageError("--frobnicate".to_owned())
    /// );
    /// assert_eq!(
    ///     Request::parse(&["my-feature".to_owned()]),
    ///     Request::LaunchCandidate("my-feature".to_owned())
    /// );
    /// ```
    ///
    /// # The internal helper surfaces: `_requests`, `_events-tail`, `_state`, `_goal`, `_memo`
    ///
    /// ```
    /// use ae::cli::Request;
    /// use ae::requests::Mode;
    ///
    /// let argv = ["_requests".to_owned(), "/s/tg1".to_owned(), "all".to_owned()];
    /// assert_eq!(
    ///     Request::parse(&argv),
    ///     Request::Requests { dir: "/s/tg1".into(), mode: Mode::All }
    /// );
    /// // The mode defaults, exactly as the helper's `${1:-mine}` does.
    /// let argv = ["_requests".to_owned(), "/s/tg1".to_owned()];
    /// assert_eq!(
    ///     Request::parse(&argv),
    ///     Request::Requests { dir: "/s/tg1".into(), mode: Mode::Mine }
    /// );
    /// ```
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "the argv parse table: one match arm per subcommand, kept as one readable dispatch rather than fragmented into sub-parsers"
    )]
    pub fn parse(args: &[String]) -> Self {
        match args.first().map(String::as_str) {
            None | Some("-h" | "--help" | "help") => Self::Help,
            Some("-V" | "--version" | "version") => Self::Version,
            Some("list" | "ls") => match ListArgs::parse(&args[1..]) {
                Ok(parsed) => Self::List(parsed),
                Err(UnknownFlag(token)) => Self::UsageError(token),
            },
            Some(REQUESTS) => Self::parse_requests(&args[1..]),
            Some(STATE) => match &args[1..] {
                [] => Self::MissingOperand(STATE),
                [dir, rest @ ..] => Self::State {
                    dir: dir.into(),
                    tail: rest.to_vec(),
                },
            },
            Some(GOAL) => match &args[1..] {
                [] => Self::MissingOperand(GOAL),
                [dir, rest @ ..] => Self::Goal {
                    dir: dir.into(),
                    tail: rest.to_vec(),
                },
            },
            Some(ASK) => match &args[1..] {
                [] => Self::MissingOperand(ASK),
                [dir, rest @ ..] => Self::Ask {
                    dir: PathBuf::from(dir),
                    tail: rest.to_vec(),
                },
            },
            Some(REVIEW) => match &args[1..] {
                [] => Self::MissingOperand(REVIEW),
                [dir, rest @ ..] => Self::Review {
                    dir: PathBuf::from(dir),
                    tail: rest.to_vec(),
                },
            },
            Some(REPLY) => match &args[1..] {
                [] => Self::MissingOperand(REPLY),
                [dir, rest @ ..] => Self::Reply {
                    dir: PathBuf::from(dir),
                    tail: rest.to_vec(),
                },
            },
            Some(SEND) => match &args[1..] {
                [] => Self::MissingOperand(SEND),
                [dir, rest @ ..] => Self::Send {
                    dir: PathBuf::from(dir),
                    tail: rest.to_vec(),
                },
            },
            Some(MEMO) => match &args[1..] {
                [] => Self::MissingOperand(MEMO),
                [dir, rest @ ..] => Self::Memo {
                    dir: dir.into(),
                    tail: rest.to_vec(),
                },
            },
            Some(EVENTS_TAIL) => match &args[1..] {
                [] => Self::MissingOperand(EVENTS_TAIL),
                [dir] => Self::EventsTail { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
            },
            Some(ARCHIVE_PUBLISH) => match &args[1..] {
                [dir, push_outcome, push_ref, preserved, workdir, archived_at] => {
                    Self::ArchivePublish {
                        dir: dir.into(),
                        push_outcome: push_outcome.clone(),
                        push_ref: push_ref.clone(),
                        preserved: preserved.clone(),
                        workdir: workdir.clone(),
                        archived_at: archived_at.clone(),
                    }
                }
                [_, _, _, _, _, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(ARCHIVE_PUBLISH),
            },
            Some(ARCHIVE_FROM_PREFLIGHT) => match &args[1..] {
                [root, raw_uuid] => Self::ArchiveFromPreflight {
                    root: root.into(),
                    raw_uuid: raw_uuid.clone(),
                },
                [_, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(ARCHIVE_FROM_PREFLIGHT),
            },
            Some(ARCHIVE_PURGE) => Self::parse_purge(&args[1..]),
            Some(END_LOCAL_TEARDOWN) => match &args[1..] {
                [dir] => Self::EndLocalTeardown { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(END_LOCAL_TEARDOWN),
            },
            Some(END_NONLOCAL_TEARDOWN) => match &args[1..] {
                [dir] => Self::EndNonlocalTeardown {
                    dir: dir.into(),
                    preserve: false,
                },
                [dir, flag] if flag == "--preserve" => Self::EndNonlocalTeardown {
                    dir: dir.into(),
                    preserve: true,
                },
                // `<dir> --preserve <extra> ...`: --preserve is valid here, so the
                // first UNEXPECTED token is the one after it — name that, not --preserve.
                [_, flag, extra, ..] if flag == "--preserve" => Self::UsageError(extra.clone()),
                [_, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(END_NONLOCAL_TEARDOWN),
            },
            Some(WATCHDOG_RUN) => match &args[1..] {
                [] => Self::MissingOperand(WATCHDOG_RUN),
                [dir, flags @ ..] => match watchdog_knobs(flags) {
                    Ok(knobs) => Self::WatchdogRun {
                        dir: dir.into(),
                        knobs,
                    },
                    Err(word) => Self::UsageError(word),
                },
            },
            Some(TELEGRAM_RUN) => match &args[1..] {
                [] => Self::MissingOperand(TELEGRAM_RUN),
                [ae_home, flags @ ..] => match telegram_options(ae_home, flags) {
                    Ok((paths, knobs)) => Self::TelegramRun { paths, knobs },
                    Err(word) => Self::UsageError(word),
                },
            },
            Some(NET_PROBE) => match &args[1..] {
                [] => Self::MissingOperand(NET_PROBE),
                [host, flags @ ..] => match net_probe_port(flags) {
                    Ok(port) => Self::NetProbe {
                        host: host.clone(),
                        port,
                    },
                    Err(word) => Self::UsageError(word),
                },
            },
            Some(COMPACT_FREEZE) => match &args[1..] {
                [dir] => Self::CompactFreeze {
                    dir: dir.into(),
                    keep_history: false,
                },
                [dir, flag] if flag == "--keep-history" => Self::CompactFreeze {
                    dir: dir.into(),
                    keep_history: true,
                },
                // `<dir> --keep-history <extra> ...`: --keep-history is valid here, so
                // the first unexpected token is the one after it — name that.
                [_, flag, extra, ..] if flag == "--keep-history" => Self::UsageError(extra.clone()),
                [_, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_FREEZE),
            },
            Some(COMPACT_REVALIDATE) => match &args[1..] {
                [dir, tuple] => Self::CompactRevalidate {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    when: DEFAULT_REVALIDATE_WHEN.to_owned(),
                    keep_history: false,
                },
                [dir, tuple, flag] if flag == "--keep-history" => Self::CompactRevalidate {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    when: DEFAULT_REVALIDATE_WHEN.to_owned(),
                    keep_history: true,
                },
                [dir, tuple, flag, label] if flag == "--when" => Self::CompactRevalidate {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    when: label.clone(),
                    keep_history: false,
                },
                [dir, tuple, flag, label, keep] if flag == "--when" && keep == "--keep-history" => {
                    Self::CompactRevalidate {
                        dir: dir.into(),
                        tuple: tuple.clone(),
                        when: label.clone(),
                        keep_history: true,
                    }
                }
                // `--when` with no label is a usage error naming the flag itself.
                [_, _, flag] if flag == "--when" => Self::MissingOperand(COMPACT_REVALIDATE),
                [_, _, flag, _, extra, ..] if flag == "--when" => Self::UsageError(extra.clone()),
                [_, _, flag, extra, ..] if flag == "--keep-history" => {
                    Self::UsageError(extra.clone())
                }
                [_, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_REVALIDATE),
            },
            Some(COMPACT_ARCHIVE) => match &args[1..] {
                [
                    dir,
                    tuple,
                    archived_at,
                    push_outcome,
                    push_ref,
                    preserved,
                    workdir,
                ] => Self::CompactArchive {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    archived_at: archived_at.clone(),
                    push_outcome: push_outcome.clone(),
                    push_ref: push_ref.clone(),
                    preserved: preserved.clone(),
                    workdir: workdir.clone(),
                    keep_history: false,
                },
                [
                    dir,
                    tuple,
                    archived_at,
                    push_outcome,
                    push_ref,
                    preserved,
                    workdir,
                    flag,
                ] if flag == "--keep-history" => Self::CompactArchive {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    archived_at: archived_at.clone(),
                    push_outcome: push_outcome.clone(),
                    push_ref: push_ref.clone(),
                    preserved: preserved.clone(),
                    workdir: workdir.clone(),
                    keep_history: true,
                },
                [_, _, _, _, _, _, _, flag, extra, ..] if flag == "--keep-history" => {
                    Self::UsageError(extra.clone())
                }
                [_, _, _, _, _, _, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_ARCHIVE),
            },
            Some(COMPACT_TEARDOWN) => match &args[1..] {
                [dir, tuple] => Self::CompactTeardown {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    keep_history: false,
                },
                [dir, tuple, flag] if flag == "--keep-history" => Self::CompactTeardown {
                    dir: dir.into(),
                    tuple: tuple.clone(),
                    keep_history: true,
                },
                [_, _, flag, extra, ..] if flag == "--keep-history" => {
                    Self::UsageError(extra.clone())
                }
                [_, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_TEARDOWN),
            },
            Some(COMPACT_WAIT) => match &args[1..] {
                [dir, reference] => Self::CompactWait {
                    dir: dir.into(),
                    reference: reference.clone(),
                    timeout_secs: crate::compact::DEFAULT_HANDOVER_SECS,
                },
                [dir, reference, flag, secs] if flag == "--timeout" => match secs.parse::<u64>() {
                    Ok(n) => Self::CompactWait {
                        dir: dir.into(),
                        reference: reference.clone(),
                        timeout_secs: n,
                    },
                    Err(_) => Self::UsageError(secs.clone()),
                },
                [_, _, flag, _, extra, ..] if flag == "--timeout" => {
                    Self::UsageError(extra.clone())
                }
                [_, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_WAIT),
            },
            Some(COMPACT_CANCEL) => match &args[1..] {
                [dir, reference] => Self::CompactCancel {
                    dir: dir.into(),
                    reference: reference.clone(),
                },
                [_, _, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_CANCEL),
            },
            Some(COMPACT_MEMO_BASELINE) => match &args[1..] {
                [dir] => Self::CompactMemoBaseline { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_MEMO_BASELINE),
            },
            Some(COMPACT_FIND_OUTSTANDING) => match &args[1..] {
                [dir] => Self::CompactFindOutstanding { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
                _ => Self::MissingOperand(COMPACT_FIND_OUTSTANDING),
            },
            // Every flag is optional: a launch with no config files still
            // resolves (to a MainMissing violation), and the entry says so
            // itself rather than being told by argv that it asked wrong.
            Some(LAUNCH_PLAN) => Self::LaunchPlan {
                tail: args[1..].to_vec(),
            },
            Some(META_INIT) => match &args[1..] {
                [] => Self::MissingOperand(META_INIT),
                [dir, tail @ ..] => Self::MetaInit {
                    dir: dir.into(),
                    tail: tail.to_vec(),
                },
            },
            Some(ROSTER) => match &args[1..] {
                [] => Self::MissingOperand(ROSTER),
                [dir, tail @ ..] => Self::Roster {
                    dir: dir.into(),
                    tail: tail.to_vec(),
                },
            },
            Some(ARCHIVE_PREVIEW) => match &args[1..] {
                [] => Self::MissingOperand(ARCHIVE_PREVIEW),
                [dir] => Self::ArchivePreview { dir: dir.into() },
                [_, extra, ..] => Self::UsageError(extra.clone()),
            },
            // SC-022, in the order the row states it: option-shaped first,
            // because everything left over is a name and not an error.
            Some(other) if other.starts_with('-') => Self::UsageError(other.to_owned()),
            Some(other) => Self::LaunchCandidate(other.to_owned()),
        }
    }

    /// `_archive-purge <session-dir> <aid> <source-session> <parent-id>`.
    fn parse_purge(tail: &[String]) -> Self {
        match tail {
            [dir, aid, source_session, parent_id] => Self::ArchivePurge {
                dir: dir.into(),
                aid: aid.clone(),
                source_session: source_session.clone(),
                parent_id: parent_id.clone(),
            },
            [_, _, _, _, extra, ..] => Self::UsageError(extra.clone()),
            _ => Self::MissingOperand(ARCHIVE_PURGE),
        }
    }

    /// `_requests <meta-dir> [mine|inbox|all]`.
    ///
    /// A mode token that is not one of the three is a usage error at `2`, not
    /// the frozen helper's `1` (lead ruling on D2): no corpus row exercises this
    /// path, and where the corpus pins nothing and the crate's exit-code
    /// contract speaks, the contract wins — `1` for both conflates "you asked
    /// wrong" with "it went wrong", which is the distinction `2` exists for.
    /// The IDENTITY refusal keeps its pinned `1`; that one is
    /// [`crate::requests::EXIT_NO_IDENTITY`] and is not decided from argv.
    fn parse_requests(tail: &[String]) -> Self {
        let (dir, token) = match tail {
            [] => return Self::MissingOperand(REQUESTS),
            [dir] => (dir, None),
            [dir, mode] => (dir, Some(mode.as_str())),
            [_, _, extra, ..] => return Self::UsageError(extra.clone()),
        };
        match Mode::parse(token) {
            Some(mode) => Self::Requests {
                dir: dir.into(),
                mode,
            },
            // `token` is necessarily `Some` on this arm: `Mode::parse(None)` is
            // the documented `mine` default and cannot fail. The fallback is
            // written rather than unwrapped so the impossible case stays
            // harmless instead of becoming a panic in a read surface.
            None => Self::UsageError(token.unwrap_or_default().to_owned()),
        }
    }

    /// The exit code **argv alone** decides, where argv decides one.
    ///
    /// `None` is the honest answer for a request whose outcome depends on
    /// something the command line does not contain: a `list` needs a session
    /// source, a launch candidate needs a launcher. Returning a number for
    /// those would publish an implementation's current behavior as though it
    /// were the contract, which is exactly the mistake this type refuses to
    /// make available.
    ///
    /// SC-022 fixes the one error code: `2` for a usage error, kept distinct
    /// from `1` so a caller can tell "you asked wrong" from "it went wrong".
    /// [`Request::MissingOperand`] is the same class and takes the same code.
    ///
    /// The helper surfaces answer `None` for the same reason `list` does:
    /// `_requests` may still refuse for want of an identity (a pinned `1`), and
    /// `_events-tail` has no completion at all — argv decides neither.
    #[must_use]
    pub fn exit_code(&self) -> Option<u8> {
        match self {
            Self::Version | Self::Help => Some(0),
            Self::UsageError(_) | Self::MissingOperand(_) => Some(2),
            Self::List(_)
            | Self::LaunchCandidate(_)
            | Self::Requests { .. }
            | Self::State { .. }
            | Self::Goal { .. }
            | Self::Memo { .. }
            | Self::Ask { .. }
            | Self::Review { .. }
            | Self::Reply { .. }
            | Self::Send { .. }
            | Self::EventsTail { .. }
            | Self::ArchivePreview { .. }
            | Self::ArchivePublish { .. }
            | Self::ArchiveFromPreflight { .. }
            | Self::ArchivePurge { .. }
            | Self::EndLocalTeardown { .. }
            | Self::EndNonlocalTeardown { .. }
            | Self::WatchdogRun { .. }
            | Self::TelegramRun { .. }
            | Self::NetProbe { .. }
            | Self::CompactFreeze { .. }
            | Self::CompactRevalidate { .. }
            | Self::CompactArchive { .. }
            | Self::CompactTeardown { .. }
            | Self::CompactWait { .. }
            | Self::CompactCancel { .. }
            | Self::CompactMemoBaseline { .. }
            | Self::CompactFindOutstanding { .. }
            | Self::LaunchPlan { .. }
            | Self::MetaInit { .. }
            | Self::Roster { .. } => None,
        }
    }
}

/// Read the watchdog's knob flags — every one `--flag <number>`, in any order.
///
/// An unrecognised flag, a missing value and an unparseable number are all the
/// same answer: the offending word, which the caller turns into SC-022's usage
/// exit. Silently defaulting a knob bash meant to set would make the daemon run
/// a cadence nobody chose, and a watchdog is not a place to guess.
fn watchdog_knobs(flags: &[String]) -> std::result::Result<crate::watchdog_daemon::Knobs, String> {
    let mut knobs = crate::watchdog_daemon::Knobs::default();
    let mut rest = flags;
    while let [flag, tail @ ..] = rest {
        let Some((value, after)) = tail.split_first() else {
            return Err(flag.clone());
        };
        let number = |text: &str| text.parse::<u64>().map_err(|_| text.to_owned());
        match flag.as_str() {
            "--interval" => knobs.interval_secs = number(value)?,
            "--stale-secs" => knobs.stale_secs = number(value)?,
            "--max-nudges" => knobs.max_nudges = count(value)?,
            "--throttle-alert-cycles" => knobs.throttle_alert_cycles = count(value)?,
            "--undelivered-max" => knobs.undelivered_max = count(value)?,
            "--quiet-beat-ms" => knobs.quiet_beat_ms = number(value)?,
            "--quiet-tries" => knobs.quiet_tries = size(value)?,
            "--quiet-panes-per-cycle" => knobs.quiet_panes_per_cycle = size(value)?,
            // The orchestrator cadence (ae:16435-16448). `--sweep-secs 0` is a
            // MEANINGFUL value, not an omission: SC-1405b makes it "no sweep
            // branch at all", so bash passing a configured zero must reach the
            // daemon as a zero rather than fall back to the frozen 300.
            "--sweep-secs" => knobs.sweep.sweep_secs = number(value)?,
            "--sweep-retry-secs" => knobs.sweep.retry_secs = number(value)?,
            "--sweep-retry-max" => knobs.sweep.retry_max = count(value)?,
            _ => return Err(flag.clone()),
        }
        rest = after;
    }
    Ok(knobs)
}

/// Read `_telegram-run`'s paths and knobs.
///
/// The two PATH flags default to the conventional layout under `<ae-home>`
/// rather than being required: bash passes them when it has them, and a human
/// running the daemon by hand should not have to restate what `AE_HOME` already
/// implies. Everything else follows the watchdog's rule — an unrecognised flag,
/// a missing value and an unparseable number are all the offending word, never
/// a silent default for a knob the caller meant to set.
fn telegram_options(
    ae_home: &str,
    flags: &[String],
) -> std::result::Result<
    (
        crate::telegram::bridge::Paths,
        crate::telegram::bridge::Knobs,
    ),
    String,
> {
    let mut paths = crate::telegram::bridge::Paths::under(ae_home);
    let mut knobs = crate::telegram::bridge::Knobs::default();
    let mut rest = flags;
    while let [flag, tail @ ..] = rest {
        if flag == "--once" {
            knobs.once = true;
            rest = tail;
            continue;
        }
        let Some((value, after)) = tail.split_first() else {
            return Err(flag.clone());
        };
        let seconds = |text: &str| {
            text.parse::<u64>()
                .map(std::time::Duration::from_secs)
                .map_err(|_| text.to_owned())
        };
        match flag.as_str() {
            "--config" => paths.config = value.into(),
            "--home" => paths.home = value.into(),
            "--limit" => knobs.inbound.limit = count(value)?,
            "--long-poll" => knobs.inbound.long_poll = seconds(value)?,
            "--outbound-interval" => knobs.outbound_interval = seconds(value)?,
            _ => return Err(flag.clone()),
        }
        rest = after;
    }
    Ok((paths, knobs))
}

/// Read `_net-probe`'s only flag: `--port <n>`.
///
/// The watchdog's rule again — an unrecognised flag, a missing value and a
/// number that is not a port are all the offending word, never a silent
/// default. A caller who typed `--port` meant a port, and resolving 443 because
/// their value did not parse would report an answer about an authority nobody
/// asked for.
fn net_probe_port(flags: &[String]) -> std::result::Result<u16, String> {
    match flags {
        [] => Ok(crate::netprobe::DEFAULT_PORT),
        [flag, value] if flag == "--port" => value.parse::<u16>().map_err(|_| value.clone()),
        // `--port <n> <extra> …`: --port is valid here, so the first UNEXPECTED
        // token is the one after its value — name that, not --port.
        [flag, _value, extra, ..] if flag == "--port" => Err(extra.clone()),
        [flag, ..] => Err(flag.clone()),
    }
}

/// A `u32` knob, or the offending word.
fn count(text: &str) -> std::result::Result<u32, String> {
    text.parse::<u32>().map_err(|_| text.to_owned())
}

/// A `usize` knob, or the offending word.
fn size(text: &str) -> std::result::Result<usize, String> {
    text.parse::<usize>().map_err(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ARCHIVE_PREVIEW, ASK, COMPACT_ARCHIVE, COMPACT_CANCEL, COMPACT_FIND_OUTSTANDING,
        COMPACT_FREEZE, COMPACT_MEMO_BASELINE, COMPACT_REVALIDATE, COMPACT_TEARDOWN, COMPACT_WAIT,
        DEFAULT_REVALIDATE_WHEN, END_NONLOCAL_TEARDOWN, EVENTS_TAIL, GOAL, MEMO, NET_PROBE, REPLY,
        REQUESTS, REVIEW, Request, SEND, STATE, TELEGRAM_RUN, WATCHDOG_RUN,
    };
    use crate::filters::{ListArgs, Scope};
    use crate::requests::Mode;

    /// Every flag the rows name, as one list — used to prove DELEGATION rather
    /// than to re-test the grammar, which is [`crate::filters`]'s job.
    const EVERY_DOCUMENTED_FLAG: [&str; 10] = [
        "--running",
        "--all",
        "--stopped",
        "--needs-attn",
        "--needs-me",
        "--needs",
        "--attn",
        "--active",
        "--busy",
        "--json",
    ];

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn every_version_spelling_parses() {
        for arg in ["-V", "--version", "version"] {
            assert_eq!(Request::parse(&[arg.to_owned()]), Request::Version, "{arg}");
        }
    }

    #[test]
    fn every_help_spelling_parses() {
        for arg in ["-h", "--help", "help"] {
            assert_eq!(Request::parse(&[arg.to_owned()]), Request::Help, "{arg}");
        }
    }

    #[test]
    fn bare_argv_is_help() {
        assert_eq!(Request::parse(&[]), Request::Help);
    }

    #[test]
    fn sc_022_an_unknown_option_is_carried_verbatim() {
        // One token, alone: what argv does with tokens AFTER a recognised one
        // is explicitly unruled by SC-022, so nothing here may depend on it.
        assert_eq!(
            Request::parse(&argv(&["--frobnicate"])),
            Request::UsageError("--frobnicate".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&["-x"])),
            Request::UsageError("-x".to_owned())
        );
    }

    #[test]
    fn sc_022_a_top_level_bare_word_is_a_session_name_not_an_error() {
        // The colead veto, as a test: there is no unknown-subcommand phrase for
        // such a token to fall into. It is a launch candidate under the start
        // grammar, and launching is out of this slice — but MISCLASSIFYING it
        // would put a phrase into the contract that the row forbids.
        for word in ["my-feature", "frobnicate", "list-ish", "9lives"] {
            assert_eq!(
                Request::parse(&argv(&[word])),
                Request::LaunchCandidate(word.to_owned()),
                "{word}"
            );
        }
    }

    #[test]
    fn sc_022_option_shape_is_what_separates_the_two() {
        // The discriminator is the leading `-`, and nothing else.
        assert!(matches!(
            Request::parse(&argv(&["-"])),
            Request::UsageError(_)
        ));
        assert!(matches!(
            Request::parse(&argv(&["-frobnicate"])),
            Request::UsageError(_)
        ));
        assert!(matches!(
            Request::parse(&argv(&["frobnicate"])),
            Request::LaunchCandidate(_)
        ));
    }

    #[test]
    fn sc_022_argv_decides_the_usage_code_and_declines_to_decide_the_others() {
        assert_eq!(Request::Version.exit_code(), Some(0));
        assert_eq!(Request::Help.exit_code(), Some(0));
        assert_eq!(Request::UsageError("x".to_owned()).exit_code(), Some(2));
        // Neither of these is decidable from the command line alone, and a
        // number here would publish today's scaffold as tomorrow's contract.
        assert_eq!(Request::List(ListArgs::default()).exit_code(), None);
        assert_eq!(Request::LaunchCandidate("s".to_owned()).exit_code(), None);
    }

    #[test]
    fn sc_017a_bare_list_is_the_default_listing() {
        assert_eq!(
            Request::parse(&argv(&["list"])),
            Request::List(ListArgs::default())
        );
    }

    #[test]
    fn ls_is_the_same_command_as_list_for_every_argv_tail() {
        // SC-021 makes `ls` an alias of `list` — one command, two spellings.
        // Tails, not just the bare word: an alias that dispatches the same but
        // parses its arguments differently is not an alias.
        let tails: [&[&str]; 5] = [
            &[],
            &["--all"],
            &["--stopped", "--json"],
            &["--needs-attn", "--busy"],
            &["--frobnicate"],
        ];
        for tail in tails {
            let mut as_list = vec!["list"];
            as_list.extend_from_slice(tail);
            let mut as_ls = vec!["ls"];
            as_ls.extend_from_slice(tail);
            assert_eq!(
                Request::parse(&argv(&as_list)),
                Request::parse(&argv(&as_ls)),
                "{tail:?}"
            );
        }
    }

    #[test]
    fn every_documented_flag_reaches_the_one_parser_that_owns_it() {
        // DELEGATION, not the grammar: what each flag MEANS is pinned in
        // `filters`. What is pinned here is that `list` hands argv to that
        // parser untouched — a second grammar in this module is exactly the
        // drift this asserts against.
        for flag in EVERY_DOCUMENTED_FLAG {
            let expected = ListArgs::parse(&[flag]).expect("a documented flag");
            assert_eq!(
                Request::parse(&argv(&["list", flag])),
                Request::List(expected),
                "{flag}"
            );
        }
    }

    #[test]
    fn sc_521_the_whole_tail_is_parsed_not_just_the_first_flag() {
        // Only the first argument decides the COMMAND; inside `list` every
        // flag counts, and SC-521b's last-distinct-selector rule needs all of
        // them to have been seen.
        let Request::List(args) = Request::parse(&argv(&["list", "--all", "--stopped", "--json"]))
        else {
            panic!("a list request");
        };
        assert_eq!(args.selection.scope, Scope::Stopped);
        assert!(args.json);
    }

    #[test]
    fn sc_022_an_unknown_flag_in_a_list_tail_is_a_usage_error() {
        let request = Request::parse(&argv(&["list", "--all", "--frobnicate"]));
        assert_eq!(request, Request::UsageError("--frobnicate".to_owned()));
        assert_eq!(request.exit_code(), Some(2));
    }

    #[test]
    fn sc_022_a_bare_word_in_a_list_tail_is_a_usage_error_unlike_at_top_level() {
        // The row draws the line by POSITION, not by shape: the same token is a
        // session name at top level and a usage error inside a list tail.
        let in_tail = Request::parse(&argv(&["list", "my-feature"]));
        assert_eq!(in_tail, Request::UsageError("my-feature".to_owned()));
        assert_eq!(in_tail.exit_code(), Some(2));
        assert_eq!(
            Request::parse(&argv(&["my-feature"])),
            Request::LaunchCandidate("my-feature".to_owned()),
            "the same word at top level is not an error at all"
        );
    }

    #[test]
    fn the_helper_spellings_all_begin_with_an_underscore() {
        // Not decoration: this is the property that keeps SC-022 whole.
        // `_validate_session_name` forbids a leading underscore, so no legal
        // session name can reach these arms — a `requests`/`events-tail`
        // spelling would have taken two real names out of the launch grammar.
        for spelling in [
            REQUESTS,
            EVENTS_TAIL,
            ARCHIVE_PREVIEW,
            STATE,
            GOAL,
            MEMO,
            ASK,
            REVIEW,
            REPLY,
            SEND,
        ] {
            assert!(spelling.starts_with('_'), "{spelling}");
            assert!(
                !spelling.starts_with('-'),
                "{spelling}: not option-shaped either, or SC-022's first arm would take it"
            );
        }
    }

    #[test]
    fn requests_takes_a_directory_and_an_optional_mode() {
        assert_eq!(
            Request::parse(&argv(&[REQUESTS, "/s/tg1"])),
            Request::Requests {
                dir: "/s/tg1".into(),
                mode: Mode::Mine
            },
            "the absent mode is `mine`, as the helper's ${{1:-mine}} is"
        );
        for (token, mode) in [
            ("mine", Mode::Mine),
            ("inbox", Mode::Inbox),
            ("all", Mode::All),
        ] {
            assert_eq!(
                Request::parse(&argv(&[REQUESTS, "/s/tg1", token])),
                Request::Requests {
                    dir: "/s/tg1".into(),
                    mode
                },
                "{token}"
            );
        }
    }

    #[test]
    fn events_tail_takes_a_directory_and_nothing_else() {
        assert_eq!(
            Request::parse(&argv(&[EVENTS_TAIL, "/s/tg1"])),
            Request::EventsTail {
                dir: "/s/tg1".into()
            }
        );
    }

    #[test]
    fn archive_preview_takes_a_directory_and_nothing_else() {
        assert_eq!(
            Request::parse(&argv(&[ARCHIVE_PREVIEW, "/s/tg1"])),
            Request::ArchivePreview {
                dir: "/s/tg1".into()
            }
        );
        assert_eq!(
            Request::parse(&argv(&[ARCHIVE_PREVIEW, "/s/tg1", "extra"])),
            Request::UsageError("extra".to_owned())
        );
    }

    #[test]
    fn a_helper_surface_with_no_directory_is_a_missing_operand_at_two() {
        for spelling in [
            REQUESTS,
            EVENTS_TAIL,
            ARCHIVE_PREVIEW,
            STATE,
            GOAL,
            MEMO,
            ASK,
            REVIEW,
            REPLY,
            SEND,
        ] {
            let request = Request::parse(&argv(&[spelling]));
            assert_eq!(request, Request::MissingOperand(spelling));
            assert_eq!(request.exit_code(), Some(2), "{spelling}");
        }
    }

    #[test]
    fn state_takes_a_directory_and_keeps_the_rest_for_the_state_module() {
        // argv is not validated here: the usage text is the helper's own and
        // lives beside the rule in `crate::state`. The parser only splits.
        assert_eq!(
            Request::parse(&argv(&[STATE, "/s/tg1", "blocked", "on", "x"])),
            Request::State {
                dir: "/s/tg1".into(),
                tail: argv(&["blocked", "on", "x"]),
            }
        );
        assert_eq!(
            Request::parse(&argv(&[STATE, "/s/tg1"])).exit_code(),
            None,
            "argv alone does not decide a declaration's outcome"
        );
    }

    #[test]
    fn a_bad_mode_token_is_the_usage_code_and_not_the_frozen_one() {
        // D2's split: the pinned `1` belongs to the identity refusal, which argv
        // cannot see. An unrecognised mode is a usage error like any other.
        let request = Request::parse(&argv(&[REQUESTS, "/s/tg1", "bogus"]));
        assert_eq!(request, Request::UsageError("bogus".to_owned()));
        assert_eq!(request.exit_code(), Some(2));
        assert_ne!(request.exit_code(), Some(1));
    }

    #[test]
    fn a_token_past_the_helper_grammar_is_a_usage_error_carrying_that_token() {
        assert_eq!(
            Request::parse(&argv(&[REQUESTS, "/s/tg1", "all", "extra"])),
            Request::UsageError("extra".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[EVENTS_TAIL, "/s/tg1", "extra"])),
            Request::UsageError("extra".to_owned())
        );
    }

    #[test]
    fn a_directory_is_taken_verbatim_however_it_is_spelled() {
        // The runner hands over whatever `<AE_HOME>/sessions/<name>` expanded
        // to. Nothing here normalises it: rewriting an operator's path is a
        // decision no row makes, and a mode-shaped directory name is still a
        // directory because position decides, not shape.
        for path in ["/s/tg1", "relative/tg1", "all", "--json", ""] {
            assert_eq!(
                Request::parse(&argv(&[REQUESTS, path, "all"])),
                Request::Requests {
                    dir: path.into(),
                    mode: Mode::All
                },
                "{path}"
            );
        }
    }

    #[test]
    fn a_helper_surface_declines_to_decide_its_exit_code_from_argv() {
        for words in [
            vec![REQUESTS, "/s/tg1", "all"],
            vec![EVENTS_TAIL, "/s/tg1"],
            vec![ARCHIVE_PREVIEW, "/s/tg1"],
        ] {
            assert_eq!(
                Request::parse(&argv(&words)).exit_code(),
                None,
                "{words:?}: an identity refusal and a follow are not argv facts"
            );
        }
    }

    #[test]
    fn a_list_that_parsed_is_never_a_usage_error() {
        for tail in [vec!["list"], vec!["ls", "--all", "--json"]] {
            assert_ne!(
                Request::parse(&argv(&tail)).exit_code(),
                Some(2),
                "{tail:?}"
            );
        }
    }

    #[test]
    fn nonlocal_teardown_parses_dir_with_optional_preserve() {
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN, "/s/tg1"])),
            Request::EndNonlocalTeardown {
                dir: "/s/tg1".into(),
                preserve: false,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN, "/s/tg1", "--preserve"])),
            Request::EndNonlocalTeardown {
                dir: "/s/tg1".into(),
                preserve: true,
            }
        );
    }

    #[test]
    fn nonlocal_teardown_usage_error_names_the_first_unexpected_token() {
        // --preserve is a VALID token here, so for `<dir> --preserve <extra>` the
        // offending token the UsageError must carry is <extra>, never the valid
        // --preserve. This is the parser's offending-token convention: name the
        // first token past the grammar, verbatim.
        assert_eq!(
            Request::parse(&argv(&[
                END_NONLOCAL_TEARDOWN,
                "/s/tg1",
                "--preserve",
                "extra"
            ])),
            Request::UsageError("extra".to_owned())
        );
        // Without --preserve, the first token after the operand is itself the
        // offender and is named verbatim (an unknown flag included).
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN, "/s/tg1", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN, "/s/tg1", "bogus", "extra"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN, "/s/tg1", "--frob"])),
            Request::UsageError("--frob".to_owned())
        );
    }

    #[test]
    fn compact_revalidate_parses_dir_tuple_and_optional_keep_history() {
        // No --when: the default single-gate label, so a legacy caller is not mislabelled.
        assert_eq!(
            Request::parse(&argv(&[COMPACT_REVALIDATE, "/s/d", "t\u{1f}u"])),
            Request::CompactRevalidate {
                dir: "/s/d".into(),
                tuple: "t\u{1f}u".to_owned(),
                when: DEFAULT_REVALIDATE_WHEN.to_owned(),
                keep_history: false,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_REVALIDATE, "/s/d", "t", "--keep-history"])),
            Request::CompactRevalidate {
                dir: "/s/d".into(),
                tuple: "t".to_owned(),
                when: DEFAULT_REVALIDATE_WHEN.to_owned(),
                keep_history: true,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_REVALIDATE, "/s/d", "t", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_REVALIDATE])),
            Request::MissingOperand(COMPACT_REVALIDATE)
        );
    }

    #[test]
    fn compact_revalidate_carries_the_gate_label_and_keep_history_together() {
        // The label is a whole argv element, spaces and all — the two crossings are named
        // "after confirmation" and "after the handover wait", and a refusal must print which.
        assert_eq!(
            Request::parse(&argv(&[
                COMPACT_REVALIDATE,
                "/s/d",
                "t",
                "--when",
                "after the handover wait",
            ])),
            Request::CompactRevalidate {
                dir: "/s/d".into(),
                tuple: "t".to_owned(),
                when: "after the handover wait".to_owned(),
                keep_history: false,
            }
        );
        // --when composes with --keep-history, in that order.
        assert_eq!(
            Request::parse(&argv(&[
                COMPACT_REVALIDATE,
                "/s/d",
                "t",
                "--when",
                "after confirmation",
                "--keep-history",
            ])),
            Request::CompactRevalidate {
                dir: "/s/d".into(),
                tuple: "t".to_owned(),
                when: "after confirmation".to_owned(),
                keep_history: true,
            }
        );
        // --when with no label names the flag as the missing operand, not a silent default.
        assert_eq!(
            Request::parse(&argv(&[COMPACT_REVALIDATE, "/s/d", "t", "--when"])),
            Request::MissingOperand(COMPACT_REVALIDATE)
        );
        // A stray token after a complete --when form is the usage error it names.
        assert_eq!(
            Request::parse(&argv(&[
                COMPACT_REVALIDATE,
                "/s/d",
                "t",
                "--when",
                "x",
                "bogus"
            ])),
            Request::UsageError("bogus".to_owned())
        );
    }

    #[test]
    fn compact_teardown_parses_dir_tuple_and_optional_keep_history() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_TEARDOWN, "/s/d", "t"])),
            Request::CompactTeardown {
                dir: "/s/d".into(),
                tuple: "t".to_owned(),
                keep_history: false,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_TEARDOWN, "/s/d", "t", "--keep-history"])),
            Request::CompactTeardown {
                dir: "/s/d".into(),
                tuple: "t".to_owned(),
                keep_history: true,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_TEARDOWN, "/s/d", "t", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
    }

    #[test]
    fn compact_wait_parses_dir_ref_and_optional_timeout() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_WAIT, "/s/d", "r1"])),
            Request::CompactWait {
                dir: "/s/d".into(),
                reference: "r1".to_owned(),
                timeout_secs: super::super::compact::DEFAULT_HANDOVER_SECS,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_WAIT, "/s/d", "r1", "--timeout", "45"])),
            Request::CompactWait {
                dir: "/s/d".into(),
                reference: "r1".to_owned(),
                timeout_secs: 45,
            }
        );
        // A non-numeric timeout is a usage error, not a silent default.
        assert_eq!(
            Request::parse(&argv(&[COMPACT_WAIT, "/s/d", "r1", "--timeout", "soon"])),
            Request::UsageError("soon".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_WAIT, "/s/d", "r1", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_WAIT])),
            Request::MissingOperand(COMPACT_WAIT)
        );
    }

    #[test]
    fn compact_cancel_parses_dir_and_ref() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_CANCEL, "/s/d", "r1"])),
            Request::CompactCancel {
                dir: "/s/d".into(),
                reference: "r1".to_owned(),
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_CANCEL, "/s/d", "r1", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_CANCEL])),
            Request::MissingOperand(COMPACT_CANCEL)
        );
    }

    #[test]
    fn compact_memo_baseline_and_find_outstanding_take_only_a_dir() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_MEMO_BASELINE, "/s/d"])),
            Request::CompactMemoBaseline { dir: "/s/d".into() }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_MEMO_BASELINE, "/s/d", "x"])),
            Request::UsageError("x".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_MEMO_BASELINE])),
            Request::MissingOperand(COMPACT_MEMO_BASELINE)
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FIND_OUTSTANDING, "/s/d"])),
            Request::CompactFindOutstanding { dir: "/s/d".into() }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FIND_OUTSTANDING])),
            Request::MissingOperand(COMPACT_FIND_OUTSTANDING)
        );
    }

    #[test]
    fn compact_archive_parses_seven_positionals_and_optional_keep_history() {
        let seven = [
            COMPACT_ARCHIVE,
            "/s/d",
            "tup",
            "2026-01-01T00:00:00Z",
            "-",
            "-",
            "-",
            "-",
        ];
        assert_eq!(
            Request::parse(&argv(&seven)),
            Request::CompactArchive {
                dir: "/s/d".into(),
                tuple: "tup".to_owned(),
                archived_at: "2026-01-01T00:00:00Z".to_owned(),
                push_outcome: "-".to_owned(),
                push_ref: "-".to_owned(),
                preserved: "-".to_owned(),
                workdir: "-".to_owned(),
                keep_history: false,
            }
        );
        let with_flag = [seven.as_slice(), &["--keep-history"]].concat();
        assert!(matches!(
            Request::parse(&argv(&with_flag)),
            Request::CompactArchive {
                keep_history: true,
                ..
            }
        ));
        // Too few positionals → missing operand; a ninth token is the offender.
        assert_eq!(
            Request::parse(&argv(&[COMPACT_ARCHIVE, "/s/d", "tup"])),
            Request::MissingOperand(COMPACT_ARCHIVE)
        );
        let extra = [seven.as_slice(), &["extra"]].concat();
        assert_eq!(
            Request::parse(&argv(&extra)),
            Request::UsageError("extra".to_owned())
        );
    }

    #[test]
    fn net_probe_takes_a_host_and_defaults_its_port() {
        assert_eq!(
            Request::parse(&argv(&[NET_PROBE, "api.telegram.org"])),
            Request::NetProbe {
                host: "api.telegram.org".to_owned(),
                port: crate::netprobe::DEFAULT_PORT,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[NET_PROBE, "example.test", "--port", "80"])),
            Request::NetProbe {
                host: "example.test".to_owned(),
                port: 80,
            }
        );
    }

    #[test]
    fn net_probe_never_guesses_a_port_it_was_asked_for() {
        // Every one of these names the offending WORD and exits 2 — a knob the
        // caller typed is never silently replaced by the default.
        for (argv_words, offender) in [
            (vec![NET_PROBE, "h", "--port"], "--port"),
            (vec![NET_PROBE, "h", "--port", "https"], "https"),
            (vec![NET_PROBE, "h", "--port", "70000"], "70000"),
            (vec![NET_PROBE, "h", "--port", "80", "extra"], "extra"),
            (vec![NET_PROBE, "h", "--nope"], "--nope"),
        ] {
            let request = Request::parse(&argv(&argv_words));
            assert_eq!(
                request,
                Request::UsageError(offender.to_owned()),
                "{argv_words:?}"
            );
            assert_eq!(request.exit_code(), Some(2));
        }
    }

    #[test]
    fn net_probe_without_a_host_is_a_missing_operand() {
        assert_eq!(
            Request::parse(&argv(&[NET_PROBE])),
            Request::MissingOperand(NET_PROBE)
        );
        assert_eq!(
            Request::parse(&argv(&[NET_PROBE])).exit_code(),
            Some(2),
            "no host is a usage error"
        );
    }

    #[test]
    fn a_lookup_is_not_decided_by_argv() {
        // `None` is the honest answer: whether this exits 0 or 1 depends on
        // what a resolver says, which the command line does not contain.
        assert_eq!(
            Request::parse(&argv(&[NET_PROBE, "localhost"])).exit_code(),
            None
        );
    }

    #[test]
    fn nonlocal_teardown_bare_is_a_missing_operand() {
        assert_eq!(
            Request::parse(&argv(&[END_NONLOCAL_TEARDOWN])).exit_code(),
            Some(2),
            "no session dir is a usage error"
        );
    }

    #[test]
    fn telegram_run_derives_both_paths_from_ae_home_when_they_are_not_given() {
        // The conventional layout is not restated by the caller: bash knows
        // AE_HOME, and everything else follows from it.
        let Request::TelegramRun { paths, knobs } =
            Request::parse(&argv(&[TELEGRAM_RUN, "/home/me/.ae"]))
        else {
            panic!("_telegram-run must parse with only an ae home");
        };
        assert_eq!(paths.ae_home, std::path::PathBuf::from("/home/me/.ae"));
        assert_eq!(
            paths.config,
            std::path::PathBuf::from("/home/me/.ae/config")
        );
        assert_eq!(paths.home, std::path::PathBuf::from("/home/me"));
        assert_eq!(knobs, crate::telegram::bridge::Knobs::default());
    }

    #[test]
    fn telegram_run_reads_every_flag_in_any_order() {
        let Request::TelegramRun { paths, knobs } = Request::parse(&argv(&[
            TELEGRAM_RUN,
            "/ae",
            "--long-poll",
            "7",
            "--once",
            "--home",
            "/elsewhere",
            "--limit",
            "3",
            "--outbound-interval",
            "9",
            "--config",
            "/etc/ae.conf",
        ])) else {
            panic!("every flag must parse in any order");
        };
        assert_eq!(paths.config, std::path::PathBuf::from("/etc/ae.conf"));
        assert_eq!(paths.home, std::path::PathBuf::from("/elsewhere"));
        assert_eq!(knobs.inbound.limit, 3);
        assert_eq!(knobs.inbound.long_poll, std::time::Duration::from_secs(7));
        assert_eq!(knobs.outbound_interval, std::time::Duration::from_secs(9));
        assert!(knobs.once);
    }

    #[test]
    fn a_telegram_knob_the_daemon_cannot_read_is_a_usage_error_not_a_default() {
        // Same rule as the watchdog's: silently defaulting a knob the caller
        // meant to set would run a cadence nobody chose.
        for bad in [
            vec![TELEGRAM_RUN, "/ae", "--nope", "1"],
            vec![TELEGRAM_RUN, "/ae", "--limit", "twelve"],
            vec![TELEGRAM_RUN, "/ae", "--long-poll"],
        ] {
            assert!(
                matches!(Request::parse(&argv(&bad)), Request::UsageError(_)),
                "{bad:?} was accepted"
            );
        }
        assert_eq!(
            Request::parse(&argv(&[TELEGRAM_RUN])),
            Request::MissingOperand(TELEGRAM_RUN)
        );
    }

    #[test]
    fn watchdog_run_defaults_every_knob_a_flagless_call_omits() {
        // The frozen cadence is what a bash side that reads no env passes: this
        // entry must run it rather than invent one.
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo"])),
            Request::WatchdogRun {
                dir: "/s/demo".into(),
                knobs: crate::watchdog_daemon::Knobs::default(),
            }
        );
    }

    #[test]
    fn watchdog_run_reads_every_knob_in_any_order() {
        let Request::WatchdogRun { dir, knobs } = Request::parse(&argv(&[
            WATCHDOG_RUN,
            "/s/demo",
            "--quiet-tries",
            "9",
            "--interval",
            "30",
            "--stale-secs",
            "120",
            "--max-nudges",
            "1",
            "--throttle-alert-cycles",
            "7",
            "--undelivered-max",
            "4",
            "--quiet-beat-ms",
            "250",
            "--quiet-panes-per-cycle",
            "3",
            "--sweep-retry-max",
            "2",
            "--sweep-secs",
            "600",
            "--sweep-retry-secs",
            "45",
        ])) else {
            panic!("the knob flags did not parse");
        };
        assert_eq!(dir, std::path::PathBuf::from("/s/demo"));
        assert_eq!(knobs.interval_secs, 30);
        assert_eq!(knobs.stale_secs, 120);
        assert_eq!(knobs.max_nudges, 1);
        assert_eq!(knobs.throttle_alert_cycles, 7);
        assert_eq!(knobs.undelivered_max, 4);
        assert_eq!(knobs.quiet_beat_ms, 250);
        assert_eq!(knobs.quiet_tries, 9);
        assert_eq!(knobs.quiet_panes_per_cycle, 3);
        assert_eq!(knobs.sweep.sweep_secs, 600);
        assert_eq!(knobs.sweep.retry_secs, 45);
        assert_eq!(knobs.sweep.retry_max, 2);
    }

    #[test]
    fn a_configured_zero_sweep_reaches_the_daemon_as_a_zero() {
        // SC-1405b is a VALUE, not an omission: `0` means "no sweep branch",
        // and defaulting it to the frozen 300 would start prompting a session
        // whose operator turned the cadence off. The control is the pair —
        // omitting the flag is what yields 300.
        let Request::WatchdogRun { knobs, .. } =
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo", "--sweep-secs", "0"]))
        else {
            panic!("the knob flags did not parse");
        };
        assert_eq!(knobs.sweep.sweep_secs, 0);
        assert!(!knobs.sweep.enabled());
        let Request::WatchdogRun { knobs, .. } = Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo"]))
        else {
            panic!("the flagless call did not parse");
        };
        assert_eq!(knobs.sweep.sweep_secs, 300);
        assert!(knobs.sweep.enabled());
    }

    #[test]
    fn a_knob_the_daemon_cannot_read_is_a_usage_error_not_a_default() {
        // Silently defaulting a knob bash meant to set would run a cadence
        // nobody chose — and a watchdog is not a place to guess.
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo", "--interval", "soon"])),
            Request::UsageError("soon".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo", "--interval"])),
            Request::UsageError("--interval".to_owned()),
            "a flag with no value names the flag"
        );
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo", "--telegram", "on"])),
            Request::UsageError("--telegram".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN])),
            Request::MissingOperand(WATCHDOG_RUN)
        );
        // SC-022: a usage error is 2, kept distinct from "it went wrong".
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo", "--interval", "soon"])).exit_code(),
            Some(2)
        );
        // The run itself answers None: argv does not decide how it ends.
        assert_eq!(
            Request::parse(&argv(&[WATCHDOG_RUN, "/s/demo"])).exit_code(),
            None
        );
    }

    #[test]
    fn compact_freeze_parses_dir_with_optional_keep_history() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FREEZE, "/s/tg1"])),
            Request::CompactFreeze {
                dir: "/s/tg1".into(),
                keep_history: false,
            }
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FREEZE, "/s/tg1", "--keep-history"])),
            Request::CompactFreeze {
                dir: "/s/tg1".into(),
                keep_history: true,
            }
        );
    }

    #[test]
    fn compact_freeze_usage_error_names_the_first_unexpected_token() {
        // --keep-history is valid, so for `<dir> --keep-history <extra>` the offending
        // token is <extra>, not the valid flag.
        assert_eq!(
            Request::parse(&argv(&[
                COMPACT_FREEZE,
                "/s/tg1",
                "--keep-history",
                "extra"
            ])),
            Request::UsageError("extra".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FREEZE, "/s/tg1", "bogus"])),
            Request::UsageError("bogus".to_owned())
        );
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FREEZE, "/s/tg1", "--frob"])),
            Request::UsageError("--frob".to_owned())
        );
    }

    #[test]
    fn compact_freeze_bare_is_a_missing_operand() {
        assert_eq!(
            Request::parse(&argv(&[COMPACT_FREEZE])).exit_code(),
            Some(2),
            "no session dir is a usage error"
        );
    }
}
