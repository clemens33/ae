//! Queued reads and batched writes: verdict cycle batches, wake channels, the ownership read, the motion exchange and the session look read.

use super::{
    FleetListingRow, HOME_VARIABLE, LookOptions, MotionPane, OWNERSHIP_VARIABLE, OptionScope,
    OptionWrite, ServerId, SessionIdentity, fleet_listing_words, interpret_display_value,
    interpret_environment_value, interpret_fleet_listing, interpret_look, interpret_marker,
    interpret_motion_panes, interpret_session_option, look_words, motion_panes_words,
    option_target, server_args, session_option_words, session_target, set_option_words,
    unset_option_words,
};

#[cfg(doc)]
use super::{
    environment_value_args, fleet_listing_args, look_args, marker_args, session_option_args,
    set_options_args,
};

/// One operation a verdict cycle publishes: an option value, or its removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CycleOp {
    /// `set-option` of one value.
    Set(OptionWrite),
    /// `set-option -u` of one user option.
    Unset {
        scope: OptionScope,
        target: String,
        name: String,
    },
}

/// What one verdict cycle publishes, in the order it decided it, sent as ONE
/// tmux invocation by `transport::publish_cycle`.
///
/// `session` is the `$<n>` id the wake hook belongs to and `channel` the
/// wake channel to install it for; `None` installs no hook.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CycleBatch {
    pub(crate) session: String,
    pub(crate) channel: Option<String>,
    pub(crate) ops: Vec<CycleOp>,
}

/// The hook index the watchdog's wake owns. Index 0 is the launch focus
/// hook's, and an indexed set leaves every other index alone.
pub(crate) const WAKE_HOOK: &str = "client-session-changed[9]";

/// The hook index the launch focus hook owns. An UNINDEXED set or unset would
/// replace or clear the whole array, the wake's [`WAKE_HOOK`] with it.
pub(crate) const FOCUS_HOOK: &str = "client-session-changed[0]";

/// The wake channel of one session incarnation: `ae-wake-<n>-<created>`.
/// Channels live in the server, so a new server, or a new session under the
/// same name, starts on a channel nothing has signalled. `None` when the
/// identity is not the `$<digits>` id and digit epoch tmux prints.
#[must_use]
pub(crate) fn wake_channel(identity: &SessionIdentity) -> Option<String> {
    let number = identity.id.strip_prefix('$')?;
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    (digits(number) && digits(&identity.created))
        .then(|| format!("ae-wake-{number}-{}", identity.created))
}

/// The line prefix of the waiter's attachment read.
const WAIT_READ_STEM: &str = "ae-wd:a|";

/// The ONE invocation a detached interval holds: the session's attachment
/// read, then the wait on `channel`. Both are items of one queued command, so
/// tmux runs them in one pass with no other client in between: an attach is
/// either before the read, which then counts it, or after the registration,
/// which its hook then signals.
#[must_use]
pub(crate) fn waiter_args(server: &ServerId, session: &str, channel: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend([
        "display-message".to_owned(),
        "-p".to_owned(),
        "-t".to_owned(),
        option_target(OptionScope::Session, session),
        format!("{WAIT_READ_STEM}#{{session_attached}}"),
        ";".to_owned(),
        "wait-for".to_owned(),
        channel.to_owned(),
    ]);
    args
}

/// A plain wait on `channel`, with no attachment read: the stepped
/// watchdog's wait for its next step.
#[must_use]
pub(crate) fn channel_waiter_args(server: &ServerId, channel: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["wait-for", "--", channel].map(str::to_owned));
    args
}

/// The signal that releases a waiter on `channel`. A killed `wait-for` client
/// leaves its queued item, and with it a server descriptor, until the channel's
/// next signal, so a waiter ae is done with is released, never killed.
#[must_use]
pub(crate) fn wake_release_args(server: &ServerId, channel: &str) -> Vec<String> {
    let mut args = server_args(server);
    args.extend(["wait-for", "-S", "--", channel].map(str::to_owned));
    args
}

/// What a waiter's output says so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaitRead {
    /// The attachment line has not arrived in full.
    Pending,
    /// The session's attached-client count.
    Attached(u64),
    /// The first line is not the attachment read.
    Malformed,
}

/// Read a waiter's output: its first LF-terminated line must be exactly the
/// stem and a decimal count.
#[must_use]
pub(crate) fn interpret_wait_read(stdout: &str) -> WaitRead {
    let Some((line, _)) = stdout.split_once('\n') else {
        return WaitRead::Pending;
    };
    line.strip_prefix(WAIT_READ_STEM)
        .filter(|count| !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|count| count.parse().ok())
        .map_or(WaitRead::Malformed, WaitRead::Attached)
}

/// The line the cycle batch prints right after it installed the wake hook.
/// tmux drops the rest of a `;` group after an error, so the line is printed
/// only when the installation ran.
pub(crate) const WAKE_HOOK_MARKER: &str = "ae-wd!w";

/// Whether `channel` can be named inside the hook's command string as one
/// bare word: tmux parses that string again when the hook fires.
#[must_use]
pub(crate) fn is_wake_channel(channel: &str) -> bool {
    !channel.is_empty()
        && channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// The most command bytes one queued invocation carries. tmux 3.4 refuses a
/// command whose words, each with its terminating NUL, exceed `MAX_IMSGSIZE`
/// (16384) less its 4-byte header: "command too long" (client.c:375). Later
/// releases accept more, so the 3.4 floor bounds every server ae runs on.
pub(crate) const COMMAND_BYTES: usize = 16_000;

/// One queued invocation of a cycle batch: its argv, and the operations it
/// carries as a range of the batch's `ops`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CycleChunk {
    pub(crate) args: Vec<String>,
    pub(crate) ops: std::ops::Range<usize>,
}

/// One verdict cycle as queued tmux invocations: the wake hook and its marker
/// first when `channel` is given, then every operation in order, in as FEW
/// invocations as [`COMMAND_BYTES`] allows — one for any ordinary cycle. A
/// single command over the bound travels alone, and tmux refuses it exactly as
/// it refused that write before the cycle was batched.
///
/// `channel` must satisfy [`is_wake_channel`]; the caller passes `None`
/// otherwise.
#[must_use]
pub(crate) fn cycle_batch_chunks(
    server: &ServerId,
    session: &str,
    channel: Option<&str>,
    ops: &[CycleOp],
) -> Vec<CycleChunk> {
    let mut chunks: Vec<(Vec<Vec<String>>, usize, std::ops::Range<usize>)> = Vec::new();
    let mut first: Vec<Vec<String>> = Vec::new();
    if let Some(channel) = channel {
        first.push(vec![
            "set-hook".to_owned(),
            "-t".to_owned(),
            option_target(OptionScope::Session, session),
            WAKE_HOOK.to_owned(),
            format!("wait-for -S {channel}"),
        ]);
        first.push(vec![
            "display-message".to_owned(),
            "-p".to_owned(),
            "-t".to_owned(),
            option_target(OptionScope::Session, session),
            WAKE_HOOK_MARKER.to_owned(),
        ]);
    }
    let first_bytes = first
        .iter()
        .map(|words| command_bytes(words))
        .sum::<usize>()
        + 2 * first.len().saturating_sub(1);
    chunks.push((first, first_bytes, 0..0));
    for (index, op) in ops.iter().enumerate() {
        let words = match op {
            CycleOp::Set(write) => set_option_words(write),
            CycleOp::Unset {
                scope,
                target,
                name,
            } => unset_option_words(*scope, target, name),
        };
        let bytes = command_bytes(&words);
        let fits = chunks.last().is_some_and(|(commands, used, _)| {
            commands.is_empty() || used + 2 + bytes <= COMMAND_BYTES
        });
        if !fits {
            chunks.push((Vec::new(), 0, index..index));
        }
        if let Some((commands, used, range)) = chunks.last_mut() {
            // `;` is a word of its own: two bytes with its NUL.
            *used += bytes + if commands.is_empty() { 0 } else { 2 };
            commands.push(words);
            range.end = index + 1;
        }
    }
    chunks
        .into_iter()
        .filter(|(commands, _, _)| !commands.is_empty())
        .map(|(commands, _, ops)| {
            let mut args = server_args(server);
            for (index, command) in commands.into_iter().enumerate() {
                if index > 0 {
                    args.push(";".to_owned());
                }
                args.extend(command);
            }
            CycleChunk { args, ops }
        })
        .collect()
}

/// What tmux counts of one command: every word with its terminating NUL.
fn command_bytes(words: &[String]) -> usize {
    words.iter().map(|word| word.len() + 1).sum()
}

/// The line prefix of the ownership read's socket answer.
const OWNERSHIP_SOCKET_STEM: &str = "ae-wd:k|";

/// The section name of the ownership read's socket answer.
const OWNERSHIP_SOCKET_SECTION: &str = "k";

/// The completion marker after one section of a queued read (the ownership
/// read, the motion exchange, the session look read). The `nonce` is 64 random
/// bits fresh per read, so a value set before the read prints a line that
/// passes for a marker only by guessing them.
fn read_marker(nonce: &str, section: &str) -> String {
    format!("ae-wd!{nonce}!{section}")
}

/// One queued invocation of [`ownership_read_args`]: its argv, and how many of
/// the requested peers, from the first, it asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnershipQuery {
    pub(crate) args: Vec<String>,
    pub(crate) peers: usize,
}

/// ONE queued invocation reading the ownership pair of as many of `peers` as
/// [`COMMAND_BYTES`] allows (at least one), led by this server's socket path
/// when `socket` is set.
///
/// Each value is today's exact read — `display-message -p '#{socket_path}'`,
/// [`marker_args`], [`environment_value_args`] for [`HOME_VARIABLE`] — followed
/// by its completion marker. tmux stops a `;` list at the first failure, so a
/// vanished peer or an unset variable ends the invocation at that peer; the
/// caller resumes with the next one.
#[must_use]
pub(crate) fn ownership_read_args(
    server: &ServerId,
    nonce: &str,
    socket: bool,
    peers: &[String],
) -> OwnershipQuery {
    let marker = |section: &str| {
        vec![
            "display-message".to_owned(),
            "-p".to_owned(),
            read_marker(nonce, section),
        ]
    };
    let mut commands: Vec<Vec<String>> = Vec::new();
    if socket {
        commands.push(vec![
            "display-message".to_owned(),
            "-p".to_owned(),
            format!("{OWNERSHIP_SOCKET_STEM}#{{socket_path}}"),
        ]);
        commands.push(marker(OWNERSHIP_SOCKET_SECTION));
    }
    let bytes = |commands: &[Vec<String>]| {
        commands
            .iter()
            .map(|words| command_bytes(words) + 2)
            .sum::<usize>()
    };
    let mut asked = 0;
    for (index, peer) in peers.iter().enumerate() {
        let read = |variable: &str| {
            vec![
                "show-environment".to_owned(),
                "-t".to_owned(),
                session_target(peer),
                variable.to_owned(),
            ]
        };
        let group = [
            read(OWNERSHIP_VARIABLE),
            marker(&format!("{index}m")),
            read(HOME_VARIABLE),
            marker(&format!("{index}h")),
        ];
        if asked > 0 && bytes(&commands) + bytes(&group) > COMMAND_BYTES {
            break;
        }
        commands.extend(group);
        asked += 1;
    }
    let mut args = server_args(server);
    for (index, command) in commands.into_iter().enumerate() {
        if index > 0 {
            args.push(";".to_owned());
        }
        args.extend(command);
    }
    OwnershipQuery { args, peers: asked }
}

/// What one [`ownership_read_args`] invocation proved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OwnershipAnswer {
    /// The socket path, when it was asked and its section completed.
    pub(crate) socket: Option<String>,
    /// One entry per DECIDED peer, from the first asked: `(marker, home)` when
    /// both its sections completed with today's one valid line each, else
    /// `None`. The peer whose read stopped the invocation is decided as
    /// `None`; the peers after it are not decided and are asked again.
    pub(crate) peers: Vec<Option<(String, String)>>,
}

/// Interpret one [`ownership_read_args`] invocation that asked `peers` peers.
///
/// A section counts only once its marker arrived, and only with exactly one
/// line: a value that spans lines is refused, as today's readers refuse it.
/// When the socket was asked and its section did not complete, nothing after
/// it ran and no peer is decided.
///
/// Lines split at LF ONLY. `str::lines` would also drop a CR before the LF,
/// and tmux prints a value's trailing CR raw, so a value today's reader keeps
/// as `/home\r` would reach the same reader here as `/home`: the framing must
/// hand every section's line over byte for byte.
#[must_use]
pub(crate) fn interpret_ownership_read(
    stdout: &str,
    nonce: &str,
    socket: bool,
    peers: usize,
) -> OwnershipAnswer {
    let stem = read_marker(nonce, "");
    let mut completed: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for line in stdout.strip_suffix('\n').unwrap_or(stdout).split('\n') {
        if let Some(section) = line.strip_prefix(stem.as_str()) {
            completed.push((section, std::mem::take(&mut lines)));
        } else {
            lines.push(line);
        }
    }
    let mut sections = completed.into_iter();
    let mut next = |expected: &str| {
        sections
            .next()
            .filter(|(section, _)| *section == expected)
            .map(|(_, lines)| lines)
    };
    let one = |lines: Vec<&str>| match lines.as_slice() {
        [line] => Some(format!("{line}\n")),
        _ => None,
    };
    let mut answer = OwnershipAnswer::default();
    if socket {
        let Some(lines) = next(OWNERSHIP_SOCKET_SECTION) else {
            return answer;
        };
        answer.socket = one(lines)
            .and_then(|line| line.strip_prefix(OWNERSHIP_SOCKET_STEM).map(str::to_owned))
            .and_then(|line| interpret_display_value(true, &line));
    }
    for index in 0..peers {
        let Some(marker) = next(&format!("{index}m")) else {
            answer.peers.push(None);
            return answer;
        };
        let Some(home) = next(&format!("{index}h")) else {
            answer.peers.push(None);
            return answer;
        };
        let marker = one(marker)
            .and_then(|line| interpret_marker(true, &line))
            .filter(|value| !value.is_empty());
        let home =
            one(home).and_then(|line| interpret_environment_value(true, &line, HOME_VARIABLE));
        answer.peers.push(marker.zip(home));
    }
    answer
}

/// The commands of one queued read: each read of `sections`, in order, followed
/// by its completion marker ([`read_marker`]). tmux stops a `;` list at the
/// first failure, so a marker on stdout proves that its read ran to completion,
/// and nothing after a failed read runs at all.
fn queued_read_commands(nonce: &str, sections: Vec<(String, Vec<String>)>) -> Vec<Vec<String>> {
    let mut commands = Vec::with_capacity(2 * sections.len());
    for (section, read) in sections {
        commands.push(read);
        commands.push(vec![
            "display-message".to_owned(),
            "-p".to_owned(),
            read_marker(nonce, &section),
        ]);
    }
    commands
}

/// `commands` behind `server`'s words, as one queued invocation.
fn queued_args(server: &ServerId, commands: Vec<Vec<String>>) -> Vec<String> {
    let mut args = server_args(server);
    for (index, command) in commands.into_iter().enumerate() {
        if index > 0 {
            args.push(";".to_owned());
        }
        args.extend(command);
    }
    args
}

/// What tmux counts of a queued list: every command, and a `;` word (two bytes
/// with its NUL) between each two.
fn queued_bytes(commands: &[Vec<String>]) -> usize {
    commands
        .iter()
        .map(|words| command_bytes(words))
        .sum::<usize>()
        + 2 * commands.len().saturating_sub(1)
}

/// The stdout of one queued read, cut into the sections `sections` names, in
/// the order they were asked.
///
/// Each completed section is EXACTLY the bytes its command printed, as the
/// same command run on its own prints them, so today's interpreter reads it
/// unchanged. The first section whose marker is missing did not complete, and
/// it and every section after it are `None`.
///
/// Lines split at LF ONLY and keep every other byte, CR included. A line counts
/// as a marker only when it IS the expected marker, nonce and all: a value that
/// prints a marker-like line without the nonce stays a line of its section. A
/// line that carries the nonce and is not the expected marker is framing this
/// read never printed, and it ends the read there.
pub(super) fn queued_sections(
    stdout: &str,
    nonce: &str,
    sections: &[String],
) -> Vec<Option<String>> {
    let stem = read_marker(nonce, "");
    let mut found: Vec<Option<String>> = Vec::with_capacity(sections.len());
    let mut lines = stdout.strip_suffix('\n').unwrap_or(stdout).split('\n');
    for section in sections {
        let marker = read_marker(nonce, section);
        let mut bytes = String::new();
        let mut complete = false;
        for line in lines.by_ref() {
            if line == marker {
                complete = true;
                break;
            }
            if line.starts_with(&stem) {
                break;
            }
            bytes.push_str(line);
            bytes.push('\n');
        }
        if !complete {
            break;
        }
        found.push(Some(bytes));
    }
    found.resize(sections.len(), None);
    found
}

/// The motion exchange's section holding [`motion_panes_words`]'s listing.
const MOTION_PANES_SECTION: &str = "p";

/// The motion exchange's section holding [`fleet_listing_args`]'s listing.
const MOTION_FLEET_SECTION: &str = "s";

/// The two sections of a motion exchange, in the order it reads them.
fn motion_sections() -> Vec<String> {
    vec![
        MOTION_PANES_SECTION.to_owned(),
        MOTION_FLEET_SECTION.to_owned(),
    ]
}

/// One queued motion exchange: its argv, and whether the writes ride in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MotionQuery {
    pub(crate) args: Vec<String>,
    /// `true` when every write rides behind the reads (always, with none). It
    /// is `false` only when the writes would carry the invocation past
    /// [`COMMAND_BYTES`]; the caller then publishes them as their own
    /// invocation, and only after the reads completed.
    pub(crate) writes: bool,
}

/// ONE queued invocation of the motion ticker: [`motion_panes_words`]'s read of
/// `session` and [`fleet_listing_args`]'s read, each followed by its completion
/// marker, then every write of `writes` as [`set_options_args`] spells it.
///
/// Both reads are today's commands verbatim. A read that fails — the session is
/// gone or renamed — ends tmux's list before any write, so a frame computed
/// from an older snapshot never lands past a missing read. The writes stay
/// behind only when they would pass [`COMMAND_BYTES`]: tmux 3.4 would refuse
/// the reads with them.
#[must_use]
pub(crate) fn motion_exchange_args(
    server: &ServerId,
    session: &str,
    nonce: &str,
    writes: &[OptionWrite],
) -> MotionQuery {
    let reads = vec![motion_panes_words(session), fleet_listing_words()];
    let mut commands =
        queued_read_commands(nonce, motion_sections().into_iter().zip(reads).collect());
    let read_commands = commands.len();
    commands.extend(writes.iter().map(set_option_words));
    let fits = commands.len() == read_commands || queued_bytes(&commands) <= COMMAND_BYTES;
    if !fits {
        commands.truncate(read_commands);
    }
    MotionQuery {
        args: queued_args(server, commands),
        writes: fits,
    }
}

/// The pane and fleet listings one [`motion_exchange_args`] invocation read,
/// each by today's interpreter over exactly the bytes its command printed — or
/// `None` unless both sections completed and both interpreters accept them.
#[must_use]
pub(crate) fn interpret_motion_exchange(
    stdout: &str,
    nonce: &str,
) -> Option<(Vec<MotionPane>, Vec<FleetListingRow>)> {
    let mut sections = queued_sections(stdout, nonce, &motion_sections()).into_iter();
    let panes = interpret_motion_panes(true, &sections.next().flatten()?)?;
    let fleet = interpret_fleet_listing(true, &sections.next().flatten()?)?;
    Some((panes, fleet))
}

/// The session look read's section holding [`look_args`]'s answer.
const LOOK_SECTION: &str = "l";

/// The sections of a session look read of `options` options: the look, then
/// one per option by its index.
fn session_look_sections(options: usize) -> Vec<String> {
    let mut sections = vec![LOOK_SECTION.to_owned()];
    sections.extend((0..options).map(|index| index.to_string()));
    sections
}

/// ONE queued invocation reading `session`'s look ([`look_args`]) and then each
/// session option of `options` ([`session_option_args`]), each followed by its
/// completion marker. Every read is today's command verbatim.
#[must_use]
pub(crate) fn session_look_read_args(
    server: &ServerId,
    session: &str,
    nonce: &str,
    options: &[&str],
) -> Vec<String> {
    let mut reads = vec![look_words(session)];
    reads.extend(
        options
            .iter()
            .map(|name| session_option_words(session, name)),
    );
    let sections = session_look_sections(options.len())
        .into_iter()
        .zip(reads)
        .collect();
    queued_args(server, queued_read_commands(nonce, sections))
}

/// What one [`session_look_read_args`] invocation read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SessionLookRead {
    /// The look, as [`interpret_look`] reads it, or `None` when its read did
    /// not complete.
    pub(crate) look: Option<LookOptions>,
    /// One entry per option asked, in order, as [`interpret_session_option`]
    /// reads it: `None` for an unset option and for a read that did not
    /// complete, exactly as a separate read gave `None` for both.
    pub(crate) options: Vec<Option<String>>,
}

/// Interpret one [`session_look_read_args`] invocation that asked `options`
/// options.
#[must_use]
pub(crate) fn interpret_session_look_read(
    stdout: &str,
    nonce: &str,
    options: usize,
) -> SessionLookRead {
    let mut sections = queued_sections(stdout, nonce, &session_look_sections(options)).into_iter();
    let look = sections
        .next()
        .flatten()
        .and_then(|section| interpret_look(true, &section));
    let options = sections
        .map(|section| section.and_then(|section| interpret_session_option(true, &section)))
        .collect();
    SessionLookRead { look, options }
}
