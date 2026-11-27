//! ae's dictation sidecar as tmux sees it: the identity that tells it apart
//! from every human client.
//!
//! The sidecar is one control client that attaches with `TERM` set to
//! [`SIDECAR_TERM`], so every client format can tell it apart. An ordinary
//! control client with the same flags still counts as a human viewer: only the
//! control mode and the reserved terminal name together mark the sidecar.
//!
//! Everything here is pure: the identity formats, the proved pane, the
//! command lines, the reply grammar and the hook table.

use std::collections::VecDeque;

use crate::inventory::ServerId;

/// The sidecar's client format, as a literal for `concat!`.
macro_rules! sidecar_client {
    () => {
        "#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}}"
    };
}

/// The human-client format, as a literal for `concat!`.
macro_rules! human_client {
    () => {
        concat!("#{?", $crate::tmux::control::sidecar_client!(), ",0,1}")
    };
}

/// The human count of the session in context, as a literal for `concat!`.
/// `L:` loops over every client with the outer session kept, and `n;` counts
/// the one byte each human client of that session prints. The conjunction
/// stays in one literal, so the 3.4 floor guard reads its two operands; the
/// predicate inside is `human_client!`'s text, which a unit test pins.
macro_rules! human_attached {
    () => {
        "#{n;L:#{?#{&&:#{==:#{session_name},#{client_session}},#{?#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}},0,1}},1,}}"
    };
}

pub(crate) use {human_attached, sidecar_client};

/// The terminal name the sidecar announces, and no human client should.
pub const SIDECAR_TERM: &str = "ae-dictation";

/// Client format: `1` for ae's dictation sidecar, `0` for every other client.
pub const SIDECAR_CLIENT: &str = sidecar_client!();

/// Client format: `1` for every client that is not the sidecar. A filter
/// (`list-clients -f`) and a hook guard (`if-shell -F`) both take it.
pub const HUMAN_CLIENT: &str = human_client!();

/// Session format: how many clients that are not the sidecar are attached to
/// the session in context. It replaces `#{session_attached}`, which counts the
/// sidecar too.
pub const HUMAN_ATTACHED: &str = human_attached!();

/// `format` for a client that is not the sidecar, and an empty line for the
/// sidecar: a client-targeted read (`display-message -c`) then finds nothing.
/// `format` must hold no comma outside its own `#{…}`.
#[must_use]
pub fn for_humans(format: &str) -> String {
    format!("#{{?{SIDECAR_CLIENT},,{format}}}")
}

/// The pane a sidecar may read and key, as the start worker proved it: only
/// canonical ids and a live process id ever reach a command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proven {
    session_id: String,
    pane_id: String,
    pane_pid: u32,
}

/// Whether `id` is `sigil` and a decimal number with no sign and no leading
/// zero, in ASCII digits, inside tmux's 32-bit id domain: a command line
/// built from it stays one short line.
fn canonical_id(id: &str, sigil: char) -> bool {
    id.strip_prefix(sigil).is_some_and(|digits| {
        !digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'))
            && digits.parse::<u32>().is_ok()
    })
}

impl Proven {
    /// `None` unless `session_id` is `$<digits>`, `pane_id` is `%<digits>`
    /// and `pane_pid` is not 0.
    #[must_use]
    pub fn new(session_id: &str, pane_id: &str, pid: u32) -> Option<Self> {
        (canonical_id(session_id, '$') && canonical_id(pane_id, '%') && pid > 0).then(|| Self {
            session_id: session_id.to_owned(),
            pane_id: pane_id.to_owned(),
            pane_pid: pid,
        })
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }

    #[must_use]
    pub const fn pane_pid(&self) -> u32 {
        self.pane_pid
    }

    /// Whether `facts` show this pane, in this session, with this process.
    #[must_use]
    pub fn matches(&self, facts: &PaneFacts) -> bool {
        facts.session_id == self.session_id
            && facts.pane_id == self.pane_id
            && facts.pane_pid == self.pane_pid
    }

    /// The pane id's digits. A format names the pane by them: tmux expands a
    /// literal `%2` inside a format as a time.
    fn pane_number(&self) -> &str {
        self.pane_id.get(1..).unwrap_or_default()
    }
}

/// One request the sidecar writes on its stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidecarCommand {
    /// The proved pane's facts, then its styled screen.
    Frame,
    /// One Space key into the proved pane, if it is still the proved one.
    Space,
}

/// The sidecar's argv after the server's: one control client on the proved
/// session that never sizes, never renders and never copies its environment.
#[must_use]
pub fn sidecar_attach_args(server: &ServerId, proven: &Proven) -> Vec<String> {
    let mut args = super::server_args(server);
    args.extend(
        [
            "-C",
            "attach-session",
            "-E",
            "-f",
            "no-output,ignore-size",
            "-t",
        ]
        .map(ToOwned::to_owned),
    );
    args.push(format!("={}", proven.session_id));
    args
}

/// The facts format: one row, printed only for the pane whose id's digits
/// are `<n>` while it belongs to the session it is read in.
macro_rules! facts_format {
    () => {
        concat!(
            "#{W:#{P:#{?#{==:#{s|^.||:pane_id},<n>},",
            "#{session_id} #{pane_id} #{pane_pid} #{pane_height} #{cursor_x} #{cursor_y},}}}"
        )
    };
}

/// The Frame's facts, then its screen: the facts are printed only for the
/// proved pane while it belongs to the proved session.
const FRAME_LINE: &str = concat!(
    "display-message -p -t '=<sid>:' '",
    facts_format!(),
    "' ; capture-pane -e -p -S 0 -t <pane>\n"
);

/// ONE tmux client's argv that prints the facts of `pane_id` while it is a
/// pane of `session_id`, as the Frame's first block does: the process a
/// sidecar may key, read before the sidecar exists. `None` for an id that is
/// not canonical.
#[must_use]
pub fn pane_proof_args(server: &ServerId, session_id: &str, pane_id: &str) -> Option<Vec<String>> {
    if !(canonical_id(session_id, '$') && canonical_id(pane_id, '%')) {
        return None;
    }
    let mut args = vec!["-u".to_owned()];
    args.extend(super::server_args(server));
    args.extend(["display-message", "-p", "-t"].map(ToOwned::to_owned));
    args.push(format!("={session_id}:"));
    args.push(facts_format!().replace("<n>", pane_id.get(1..)?));
    Some(args)
}

/// The pane [`pane_proof_args`] proved: exactly one facts row that names
/// `session_id` and `pane_id`, with a live process id.
#[must_use]
pub fn interpret_pane_proof(
    succeeded: bool,
    stdout: &str,
    session_id: &str,
    pane_id: &str,
) -> Option<Proven> {
    let row = stdout.strip_suffix('\n')?;
    if !succeeded || row.contains('\n') {
        return None;
    }
    let facts = PaneFacts::parse(row)?;
    if facts.session_id != session_id || facts.pane_id != pane_id {
        return None;
    }
    Proven::new(session_id, pane_id, facts.pane_pid)
}

/// The Space: tmux checks the pane's id, process and liveness inside the
/// proved session and sends the key in the same command.
const SPACE_LINE: &str = concat!(
    "if-shell -F -t '=<sid>:' ",
    "'#{W:#{P:#{?#{&&:#{==:#{s|^.||:pane_id},<n>},#{&&:#{==:#{pane_pid},<pid>},#{==:#{pane_dead},0}}},1,}}}' ",
    "'send-keys -t <pane> Space' 'display-message -p ae-pane-changed'\n"
);

/// The one stdin line of `command`, ending in a newline. The session target
/// is single-quoted, so tmux's parser never reads `$7` as a variable.
#[must_use]
pub fn command_line(proven: &Proven, command: SidecarCommand) -> String {
    let template = match command {
        SidecarCommand::Frame => FRAME_LINE,
        SidecarCommand::Space => SPACE_LINE,
    };
    template
        .replace("<sid>", &proven.session_id)
        .replace("<n>", proven.pane_number())
        .replace("<pid>", &proven.pane_pid.to_string())
        .replace("<pane>", &proven.pane_id)
}

/// What the Frame's first block reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFacts {
    pub session_id: String,
    pub pane_id: String,
    pub pane_pid: u32,
    /// The screen's rows: the capture block holds exactly this many.
    pub height: u16,
    /// The cursor as `(x, y)`.
    pub cursor: (u16, u16),
}

impl PaneFacts {
    /// The six fields of the Frame's facts line, or `None`.
    fn parse(line: &str) -> Option<Self> {
        let fields: Vec<&str> = line.split(' ').collect();
        let [session_id, pane_id, pid, height, x, y] = fields.as_slice() else {
            return None;
        };
        let number = |field: &str| {
            (!field.is_empty() && field.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| field.parse::<u32>().ok())
                .flatten()
        };
        let small = |field: &str| number(field).and_then(|value| u16::try_from(value).ok());
        let facts = Self {
            session_id: (*session_id).to_owned(),
            pane_id: (*pane_id).to_owned(),
            pane_pid: number(pid)?,
            height: small(height).filter(|rows| *rows > 0)?,
            cursor: (small(x)?, small(y)?),
        };
        (canonical_id(session_id, '$') && canonical_id(pane_id, '%')).then_some(facts)
    }
}

/// What the next reply block must be. The writer queues these before it
/// writes the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The attach command's own block, flags 0. [`ControlParser::new`]
    /// queues it.
    Attach,
    /// A block with no body.
    Empty,
    /// The Frame's facts line.
    Facts,
    /// The Frame's screen, exactly as many rows as the facts said.
    Capture,
}

impl Expect {
    /// The blocks `command` answers with, in order.
    #[must_use]
    pub const fn for_command(command: SidecarCommand) -> [Self; 2] {
        match command {
            SidecarCommand::Frame => [Self::Facts, Self::Capture],
            SidecarCommand::Space => [Self::Empty, Self::Empty],
        }
    }

    /// The `%begin` flags of this block: 1 for the client's own commands.
    const fn flags(self) -> &'static str {
        match self {
            Self::Attach => "0",
            Self::Empty | Self::Facts | Self::Capture => "1",
        }
    }
}

/// One fact of the control client's stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlEvent {
    /// The attach command's block ended `%end` with no body.
    Attached,
    /// An expected Empty block ended `%end` with no body.
    Empty,
    /// An expected Facts block's one line.
    Facts(PaneFacts),
    /// An expected Capture block: its rows, joined by newlines.
    Capture(String),
    /// A block ended `%error`, carried a payload where none belongs, or its
    /// facts were unreadable. The text is kept.
    Refused(String),
    /// tmux said `%exit`.
    Exit,
    /// The stream broke the grammar.
    Malformed,
    /// A line passed [`ControlParser::MAX_LINE`] or a body passed
    /// [`ControlParser::MAX_BODY`].
    Overflow,
}

impl ControlEvent {
    /// Whether the parser yields nothing after this event, and the sidecar
    /// may send nothing more.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Refused(_) | Self::Exit | Self::Malformed | Self::Overflow
        )
    }
}

/// One open reply block.
#[derive(Debug)]
struct Block {
    /// `<time> <number> <flags>`, which the closing line must repeat.
    header: String,
    expect: Expect,
    rows: Vec<String>,
    bytes: usize,
}

#[derive(Debug)]
enum State {
    Between,
    Open(Block),
    Ended,
}

/// The control client's stdout as events. The stream is hostile: a Capture
/// block's rows are pane text, so they are counted, never decoded, and every
/// line and body is bounded.
#[derive(Debug)]
pub struct ControlParser {
    line: Vec<u8>,
    queue: VecDeque<Expect>,
    state: State,
    /// The rows the last Facts promised its Capture.
    height: Option<u16>,
}

impl Default for ControlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlParser {
    pub const MAX_LINE: usize = 64 * 1024;
    pub const MAX_BODY: usize = 1024 * 1024;

    /// A parser that expects the attach block first.
    #[must_use]
    pub fn new() -> Self {
        Self {
            line: Vec::new(),
            queue: VecDeque::from([Expect::Attach]),
            state: State::Between,
            height: None,
        }
    }

    /// Queue the next expected block. A block with nothing queued is
    /// Malformed.
    pub fn expect(&mut self, next: Expect) {
        self.queue.push_back(next);
    }

    /// Parse a chunk of the client's stdout. Events come in stream order, and
    /// none follows a Refused, Exit, Malformed or Overflow.
    pub fn feed(&mut self, mut bytes: &[u8]) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        while !bytes.is_empty() && !matches!(self.state, State::Ended) {
            let (head, rest, complete) = match bytes.iter().position(|byte| *byte == b'\n') {
                Some(at) => (&bytes[..at], &bytes[at + 1..], true),
                None => (bytes, &[][..], false),
            };
            bytes = rest;
            self.line.extend_from_slice(head);
            let event = if self.line.len() > Self::MAX_LINE {
                Some(ControlEvent::Overflow)
            } else if complete {
                match String::from_utf8(std::mem::take(&mut self.line)) {
                    Ok(line) => self.line_event(line),
                    Err(_) => Some(ControlEvent::Malformed),
                }
            } else {
                None
            };
            if let Some(event) = event {
                if event.is_terminal() {
                    self.state = State::Ended;
                }
                events.push(event);
            }
        }
        events
    }

    /// The end of the stream: a partial line, an open block or a queued
    /// expectation is Malformed.
    pub fn finish(&mut self) -> Vec<ControlEvent> {
        let open =
            !self.line.is_empty() || matches!(self.state, State::Open(_)) || !self.queue.is_empty();
        let ended = matches!(self.state, State::Ended);
        self.state = State::Ended;
        if open && !ended {
            vec![ControlEvent::Malformed]
        } else {
            Vec::new()
        }
    }

    fn line_event(&mut self, line: String) -> Option<ControlEvent> {
        match std::mem::replace(&mut self.state, State::Ended) {
            State::Ended => None,
            State::Between => self.between(&line),
            State::Open(block) => self.inside(block, line),
        }
    }

    /// A line outside any block: a `%begin`, `%exit`, or a notification.
    fn between(&mut self, line: &str) -> Option<ControlEvent> {
        if let Some(header) = line.strip_prefix("%begin ") {
            let Some(expect) = self.queue.pop_front() else {
                return Some(ControlEvent::Malformed);
            };
            let fields: Vec<&str> = header.split(' ').collect();
            let well_formed = fields.len() == 3
                && fields
                    .iter()
                    .all(|field| !field.is_empty() && field.bytes().all(|b| b.is_ascii_digit()));
            if !well_formed || fields.get(2) != Some(&expect.flags()) {
                return Some(ControlEvent::Malformed);
            }
            if expect == Expect::Capture {
                if self.height.is_none() {
                    return Some(ControlEvent::Malformed);
                }
            } else {
                self.height = None;
            }
            self.state = State::Open(Block {
                header: header.to_owned(),
                expect,
                rows: Vec::new(),
                bytes: 0,
            });
            return None;
        }
        match line.split(' ').next().unwrap_or_default() {
            "%exit" => Some(ControlEvent::Exit),
            "%begin" | "%end" | "%error" => Some(ControlEvent::Malformed),
            word if word.starts_with('%') => {
                self.state = State::Between;
                None
            }
            _ => Some(ControlEvent::Malformed),
        }
    }

    /// A line inside `block`.
    fn inside(&mut self, mut block: Block, line: String) -> Option<ControlEvent> {
        let end = format!("%end {}", block.header);
        if block.expect == Expect::Capture {
            let height = usize::from(self.height.unwrap_or_default());
            if block.rows.len() < height {
                return self.keep(block, line);
            }
            if line != end {
                return Some(ControlEvent::Malformed);
            }
            self.height = None;
            self.state = State::Between;
            return Some(ControlEvent::Capture(block.rows.join("\n")));
        }
        if line == end {
            return Some(self.closed(&block));
        }
        if line == format!("%error {}", block.header) {
            return Some(ControlEvent::Refused(block.rows.join("\n")));
        }
        if matches!(line.split(' ').next(), Some("%begin" | "%end" | "%error")) {
            return Some(ControlEvent::Malformed);
        }
        block.bytes = block.bytes.saturating_add(line.len() + 1);
        if block.bytes > Self::MAX_BODY {
            return Some(ControlEvent::Overflow);
        }
        block.rows.push(line);
        self.state = State::Open(block);
        None
    }

    /// Add one counted screen row to a Capture block.
    fn keep(&mut self, mut block: Block, line: String) -> Option<ControlEvent> {
        block.bytes = block.bytes.saturating_add(line.len() + 1);
        if block.bytes > Self::MAX_BODY {
            return Some(ControlEvent::Overflow);
        }
        block.rows.push(line);
        self.state = State::Open(block);
        None
    }

    /// The event of a non-Capture block that ended `%end`.
    fn closed(&mut self, block: &Block) -> ControlEvent {
        let event = match block.expect {
            Expect::Attach | Expect::Empty if !block.rows.is_empty() => {
                ControlEvent::Refused(block.rows.join("\n"))
            }
            Expect::Attach => ControlEvent::Attached,
            Expect::Empty => ControlEvent::Empty,
            Expect::Facts => match block.rows.as_slice() {
                [row] => match PaneFacts::parse(row) {
                    Some(facts) => {
                        self.height = Some(facts.height);
                        ControlEvent::Facts(facts)
                    }
                    None => ControlEvent::Refused(row.clone()),
                },
                _ => ControlEvent::Malformed,
            },
            Expect::Capture => ControlEvent::Malformed,
        };
        if !event.is_terminal() {
            self.state = State::Between;
        }
        event
    }
}

/// Why the sidecar may not attach with the session's hooks as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookRefusal {
    /// A hook read failed or printed a row the table does not parse.
    Unread,
    /// A global hook is set on a watched name.
    Global,
    /// A session hook on a watched name runs without the human-client guard.
    Unguarded { index: u16 },
}

/// The hooks an attach or a detach of the sidecar can fire. Of them only
/// `client-detached` fires with no client context, so no guard can tell the
/// sidecar apart there: it must be unset.
pub const WATCHED_HOOKS: [&str; 3] = [
    "client-attached",
    "client-session-changed",
    "client-detached",
];

/// Whether `name`'s hooks let the sidecar attach: `session` is what
/// `show-hooks -t =<sid>: <name>` printed, `global` what `show-hooks -g
/// <name>` printed. Every session row must run its whole command inside the
/// human-client guard; no global hook may be set.
///
/// # Errors
///
/// The [`HookRefusal`] of the first row that fails, then of the global read.
pub fn hooks_eligible(name: &str, session: &str, global: &str) -> Result<(), HookRefusal> {
    if !session.is_empty() && !session.ends_with('\n') {
        return Err(HookRefusal::Unread);
    }
    for row in session.lines() {
        let index = row
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('['))
            .and_then(|rest| rest.split_once("] "))
            .filter(|(digits, _)| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
            .and_then(|(digits, command)| Some((digits.parse::<u16>().ok()?, command)));
        let Some((index, command)) = index else {
            return Err(HookRefusal::Unread);
        };
        if name == "client-detached" || !guarded(command) {
            return Err(HookRefusal::Unguarded { index });
        }
    }
    if global == format!("{name}\n") {
        Ok(())
    } else {
        Err(HookRefusal::Global)
    }
}

/// The marker line after each hook read of [`sidecar_hook_args`]: the hook's
/// place in [`WATCHED_HOOKS`], then `s` for the session read or `g` for the
/// global one.
fn hook_marker(index: usize, scope: char) -> String {
    format!("ae-hooks!{index}{scope}")
}

/// ONE tmux client's argv that reads every watched hook of the proved
/// session, then globally, each read followed by its marker line.
#[must_use]
pub fn sidecar_hook_args(server: &ServerId, proven: &Proven) -> Vec<String> {
    let mut args = vec!["-u".to_owned()];
    args.extend(super::server_args(server));
    let target = format!("={}:", proven.session_id);
    for (index, name) in WATCHED_HOOKS.iter().enumerate() {
        if index > 0 {
            args.push(";".to_owned());
        }
        args.extend(["show-hooks", "-t", &target, name, ";"].map(ToOwned::to_owned));
        args.extend(
            ["display-message", "-p", &hook_marker(index, 's'), ";"].map(ToOwned::to_owned),
        );
        args.extend(["show-hooks", "-g", name, ";"].map(ToOwned::to_owned));
        args.extend(["display-message", "-p", &hook_marker(index, 'g')].map(ToOwned::to_owned));
    }
    args
}

/// Split what [`sidecar_hook_args`] printed into each watched hook's session
/// and global output, `None` unless every marker came in order and nothing
/// followed the last.
#[must_use]
pub fn interpret_sidecar_hooks(stdout: &str) -> Option<Vec<(&'static str, String, String)>> {
    /// The rows before `marker`, each with its newline, and `None` when the
    /// marker never came.
    fn section<'a>(rows: &mut impl Iterator<Item = &'a str>, marker: &str) -> Option<String> {
        let mut text = String::new();
        for row in rows {
            if row == marker {
                return Some(text);
            }
            text.push_str(row);
            text.push('\n');
        }
        None
    }
    let mut rows = stdout.strip_suffix('\n')?.split('\n');
    let mut read = Vec::new();
    for (index, name) in WATCHED_HOOKS.iter().enumerate() {
        let session = section(&mut rows, &hook_marker(index, 's'))?;
        let global = section(&mut rows, &hook_marker(index, 'g'))?;
        read.push((*name, session, global));
    }
    rows.next().is_none().then_some(read)
}

/// Whether `command`, as `show-hooks` prints it, is the human-client guard
/// around one brace group that closes exactly at the end of the row.
fn guarded(command: &str) -> bool {
    let opening = format!("if-shell -F \"{HUMAN_CLIENT}\" {{ ");
    let Some(body) = command.strip_prefix(&opening) else {
        return false;
    };
    let mut depth = 1_usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, ch) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('"'), '\\') => escaped = true,
            (Some(open), _) if ch == open => quote = None,
            (None, '"' | '\'') => quote = Some(ch),
            (None, '{') => depth += 1,
            (None, '}') => {
                depth -= 1;
                if depth == 0 {
                    return at + ch.len_utf8() == body.len();
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
#[path = "../../../tests/fixtures/harness-state/d54-control-parser.rs"]
mod parser;

#[cfg(test)]
mod tests {
    /// The oracle is d54-nav's measurement on a private tmux 3.7b server
    /// (sidecar-format-evidence.md in the D54 evidence): one marked client
    /// counted 0 and two unmarked control clients counted 2.
    #[test]
    fn the_identity_formats_are_the_measured_ones() {
        assert_eq!(
            super::SIDECAR_CLIENT,
            "#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}}"
        );
        assert_eq!(
            super::HUMAN_CLIENT,
            "#{?#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}},0,1}"
        );
        assert_eq!(
            super::HUMAN_ATTACHED,
            "#{n;L:#{?#{&&:#{==:#{session_name},#{client_session}},#{?#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}},0,1}},1,}}"
        );
        assert!(super::SIDECAR_CLIENT.contains(&format!(",{}}}", super::SIDECAR_TERM)));
    }

    #[test]
    fn a_client_targeted_read_prints_nothing_for_the_sidecar() {
        assert_eq!(
            super::for_humans("#{session_id} | #{pane_id}"),
            "#{?#{&&:#{client_control_mode},#{==:#{client_termname},ae-dictation}},,#{session_id} | #{pane_id}}"
        );
    }

    /// The human count spells its conjunction in one literal for the 3.4
    /// floor guard, so its client predicate is written out: it must stay the
    /// human-client format, inside the session test, and count one per client.
    #[test]
    fn the_human_count_is_the_human_client_format_inside_the_session_test() {
        let (outer, inner) = super::HUMAN_ATTACHED
            .split_once(human_client!())
            .unwrap_or_else(|| panic!("the human count lost the human-client format"));
        // Spelled in pieces: the floor guard reads a conjunction's opener in
        // the source, and this pin is not one.
        let session_test = [
            "#{n;L:#{?#{",
            "&&",
            ":#{==:#{session_name},#{client_session}},",
        ];
        assert_eq!(outer, session_test.concat());
        assert_eq!(inner, "},1,}}");
    }

    /// The facts row the measured Frame prints (d54-nav, tmux 3.8 and the 3.4
    /// floor), read once and alone: the same format, the same session target.
    #[test]
    fn the_pane_proof_is_the_frames_facts_read_once() {
        use crate::inventory::ServerId;
        use crate::meta::Selector;

        const MEASURED: &str = "#{W:#{P:#{?#{==:#{s|^.||:pane_id},7},#{session_id} #{pane_id} #{pane_pid} #{pane_height} #{cursor_x} #{cursor_y},}}}";
        let server = ServerId::Selected(Selector::Socket("/private/ae-sock".into()));
        let mut expected = vec!["-u".to_owned()];
        expected.extend(super::super::server_args(&server));
        expected.extend(["display-message", "-p", "-t", "=$3:", MEASURED].map(ToOwned::to_owned));
        assert_eq!(super::pane_proof_args(&server, "$3", "%7"), Some(expected));
        let proven = super::Proven::new("$3", "%7", 4242).expect("canonical ids");
        let frame = super::command_line(&proven, super::SidecarCommand::Frame);
        assert!(frame.contains(&format!("'{MEASURED}'")), "{frame}");
        for (session, pane) in [
            ("3", "%7"),
            ("$3", "7"),
            ("$03", "%7"),
            ("$3", "%4294967296"),
        ] {
            assert_eq!(super::pane_proof_args(&server, session, pane), None);
        }
    }

    #[test]
    fn only_one_row_of_the_proved_pane_with_a_live_process_proves_it() {
        let proof = |ok, stdout: &str| super::interpret_pane_proof(ok, stdout, "$3", "%7");
        let proven = proof(true, "$3 %7 4242 41 2 39\n").expect("the one facts row");
        assert_eq!(
            (proven.session_id(), proven.pane_id(), proven.pane_pid()),
            ("$3", "%7", 4242)
        );
        for (ok, stdout) in [
            (false, "$3 %7 4242 41 2 39\n"),
            (true, ""),
            (true, "\n"),
            (true, "$3 %7 4242 41 2 39"),
            (true, "$3 %7 4242 41 2 39\n$3 %7 4242 41 2 39\n"),
            (true, "$4 %7 4242 41 2 39\n"),
            (true, "$3 %8 4242 41 2 39\n"),
            (true, "$3 %7 0 41 2 39\n"),
            (true, "$3 %7 4242 0 2 39\n"),
        ] {
            assert_eq!(proof(ok, stdout), None, "{ok} {stdout:?}");
        }
    }
}
