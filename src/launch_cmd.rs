//! The launch-command lexer: which binary a profile's command line runs.
//!
//! A profile's value is an OPERATOR-AUTHORED shell command line (env
//! assignments, quoted arguments, `$HOME` expansions are all in use). ae never
//! executes it outside a pane's own shell — see the identity plan's command
//! execution contract — but it must CLASSIFY it: the binary name becomes
//! `agent_bin.<slot>`, and the tool kind selects the harness channel the
//! rendered context rides on. The rules:
//!
//! - words are split on spaces and tabs, honouring single quotes (everything
//!   literal), double quotes (`\` escapes only `"`, `\`, `$`, `` ` `` and a
//!   newline), and bare backslashes;
//! - the prefix words `env`, `-i` (no argument), `-u <name>` (one argument) and
//!   `VAR=value` are skipped; the first other word is the binary;
//! - an unterminated quote or a trailing bare backslash is MALFORMED, and so is
//!   a line with nothing but prefix words: the answer is "no binary", never a
//!   guess. A wrong classification splices a mis-shaped command into a live
//!   pane; an unknown one costs a degraded launch. Fail toward unknown.
//!
//! Validation is scoped to what is consumed — the prefix and the binary word.
//! The tail is not lexed: `codex --yolo # note\` is a valid line (the backslash
//! sits in a comment) and ae makes no claim about it.

/// The command line split at its binary word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// Everything before the binary word — the env prefix, verbatim.
    pub prefix: String,
    /// The binary word as written (quotes and escapes included).
    pub binary_raw: String,
    /// The binary word with its quoting resolved.
    pub binary: String,
    /// Everything after the binary word, verbatim.
    pub rest: String,
}

impl Split {
    /// The bare binary name: path stripped.
    #[must_use]
    pub fn binary_name(&self) -> &str {
        self.binary.rsplit('/').next().unwrap_or(&self.binary)
    }
}

use std::path::{Path, PathBuf};

use crate::config::ResolvedCommand;
use crate::tool::ToolKind;

/// Split `cmd` at its binary word, or `None` when it is malformed or carries
/// nothing but prefix words.
#[must_use]
pub fn split_binary(cmd: &str) -> Option<Split> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut skip = false;
    loop {
        while i < n && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= n {
            return None;
        }
        let wstart = i;
        let mut word = String::new();
        let mut quote: Option<char> = None;
        while i < n {
            let ch = chars[i];
            match quote {
                Some('\'') => {
                    if ch == '\'' {
                        quote = None;
                    } else {
                        word.push(ch);
                    }
                }
                Some(_) => {
                    // Double quotes.
                    if ch == '\\' {
                        let next = *chars.get(i + 1)?;
                        match next {
                            '"' | '\\' | '$' | '`' => {
                                word.push(next);
                                i += 1;
                            }
                            '\n' => i += 1,
                            _ => word.push(ch),
                        }
                    } else if ch == '"' {
                        quote = None;
                    } else {
                        word.push(ch);
                    }
                }
                None => {
                    if ch == '\\' {
                        i += 1;
                        word.push(*chars.get(i)?);
                    } else if ch == '\'' || ch == '"' {
                        quote = Some(ch);
                    } else if ch == ' ' || ch == '\t' {
                        break;
                    } else {
                        word.push(ch);
                    }
                }
            }
            i += 1;
        }
        // An unterminated quote is malformed too.
        if quote.is_some() {
            return None;
        }
        if skip {
            skip = false;
            continue;
        }
        match word.as_str() {
            "env" | "-i" => {}
            "-u" => skip = true,
            w if w.contains('=') => {}
            _ => {
                return Some(Split {
                    prefix: chars[..wstart].iter().collect(),
                    binary_raw: chars[wstart..i].iter().collect(),
                    binary: word,
                    rest: chars[i..].iter().collect(),
                });
            }
        }
    }
}

/// Why a launch command is not ONE SIMPLE COMMAND — refused at plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A quote that never closes.
    UnterminatedQuote,
    /// A bare `\` at the end of the line.
    TrailingBackslash,
    /// `;`, `&`, `|` or a newline outside quotes — a second command, or a
    /// background/pipeline operator.
    ControlOperator(String),
    /// `#` starting a word outside quotes — everything after it (the fixed
    /// suffix included) would be a comment.
    Comment,
    /// `<` or `>` outside quotes — a redirection or process substitution.
    Redirection(String),
    /// `` ` `` or `$(` outside single quotes — command substitution stays
    /// active inside double quotes, so only the single-quoted or escaped form
    /// is plain bytes.
    Substitution(String),
    /// `(`, `)`, or a `{`/`}` not introduced by `$` outside quotes —
    /// grouping or brace expansion, either of which detaches the suffix.
    Grouping(String),
    /// A `${…}` parameter form ae does not implement — refused HERE because
    /// `_run` cannot expand it either, and a form only the validator accepted
    /// was a seat that planned green and then exited 1 in its own pane (colead
    /// Z2 BLOCKER-3).
    Parameter,
    /// Assignments only, or nothing: there is no command word.
    NoCommand,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnterminatedQuote => write!(f, "an unterminated quote"),
            Self::TrailingBackslash => write!(f, "a trailing backslash"),
            Self::ControlOperator(op) => {
                write!(f, "the shell control operator '{op}' outside quotes")
            }
            Self::Comment => write!(f, "a comment ('#' starting a word)"),
            Self::Redirection(op) => write!(f, "the redirection '{op}' outside quotes"),
            Self::Substitution(op) => {
                write!(f, "command substitution ('{op}') outside quotes")
            }
            Self::Grouping(op) => write!(f, "grouping or brace expansion ('{op}') outside quotes"),
            Self::Parameter => write!(f, "a ${{…}} parameter form ae does not implement"),
            Self::NoCommand => write!(f, "no command word (assignments only, or empty)"),
        }
    }
}

/// One simple command: leading assignments, then one argv vector — with the
/// RAW spans preserved so the validated fact is transported, never reparsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// The leading `NAME=value` words, as written.
    pub assignments: Vec<String>,
    /// The command words, as written (quoting preserved) — `words[0]` is the
    /// first command-line word; the harness is [`SimpleCommand::binary`].
    pub words: Vec<String>,
    /// The RAW leading-assignment span, byte-exact from the source (internal
    /// whitespace kept), empty when there are no assignments.
    pub assign_span: String,
    /// The RAW command-vector span, byte-exact from the source.
    pub argv_span: String,
    /// The resolved binary the pane will actually run (quoting resolved, path
    /// stripped, a contextual `env` prefix peeled) — the harness word.
    pub binary: String,
    /// Quote-resolved argv words, parallel to [`Self::words`].
    word_values: Vec<String>,
    /// Index of the executable inside `words` / `word_values`.
    binary_index: usize,
    /// Byte range of the executable word in the original command.
    binary_span: (usize, usize),
}

impl SimpleCommand {
    /// Which harness this command launches.
    #[must_use]
    pub fn tool(&self) -> ToolKind {
        ToolKind::from_binary_name(&self.binary)
    }
}

/// The executable word selected by the same prefix grammar the executor uses.
pub(crate) struct LaunchBinary<'a> {
    /// Index inside [`SimpleCommand::words`].
    pub(crate) index: usize,
    /// Full quote-resolved word; paths are not stripped.
    pub(crate) word: &'a str,
    /// Byte range in the original command.
    pub(crate) span: (usize, usize),
}

/// Locate the executable after leading assignments and one contextual `env`
/// prefix. This is the shared classifier/rewriter rule.
#[must_use]
pub(crate) fn launch_binary(command: &SimpleCommand) -> LaunchBinary<'_> {
    LaunchBinary {
        index: command.binary_index,
        word: &command.word_values[command.binary_index],
        span: command.binary_span,
    }
}

/// Whether the executable prefix assigns or `env -u` unsets `variable`.
#[must_use]
pub(crate) fn prefix_mentions(command: &SimpleCommand, variable: &str) -> bool {
    if command.assignments.iter().any(|assignment| {
        assignment
            .split_once('=')
            .is_some_and(|(name, _)| name == variable)
    }) {
        return true;
    }
    let binary = launch_binary(command);
    if command.word_values.first().is_none_or(|word| word != "env") {
        return false;
    }
    let prefix = &command.word_values[1..binary.index];
    let mut index = 0;
    while index < prefix.len() {
        if prefix[index] == "-u" {
            if prefix.get(index + 1).is_some_and(|name| name == variable) {
                return true;
            }
            index += 2;
            continue;
        }
        if prefix[index]
            .split_once('=')
            .is_some_and(|(name, _)| name == variable)
        {
            return true;
        }
        index += 1;
    }
    false
}

/// The config home a resolved launch command will give its tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A verified absolute config-home path.
    Path(PathBuf),
    /// Neither the account variable nor an effective `HOME` exists.
    Absent,
    /// The command names a value ae cannot safely use as a config home.
    Unknown(String),
}

/// The effective store plus how the command selected it.
///
/// Claude distinguishes an unset account variable from one explicitly set to
/// its default directory, so callers that persist identity must keep this bit.
pub(crate) struct ConfigHomeResolution {
    pub(crate) home: Resolved,
    pub(crate) base: Resolved,
    pub(crate) explicit: bool,
}

impl Resolved {
    /// Stable spelling for a persisted `config_home.<slot>` row.
    #[must_use]
    pub fn record_value(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Absent => "absent".to_owned(),
            Self::Unknown(_) => "unknown".to_owned(),
        }
    }

    /// Short human spelling used when current config differs from a seat's
    /// retained conversation store.
    #[must_use]
    pub fn shown(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Absent => "absent".to_owned(),
            Self::Unknown(reason) => format!("unknown ({reason})"),
        }
    }
}

/// Resolve the config home the direct executor will expose to `tool`.
///
/// The injected lookup is the pane's inherited environment. Profile-prefix
/// assignments are layered over it with the same ordering as `_run`: clear,
/// unsets, then assignments. Parameter expansion happens once while words are
/// split; the resulting values are never expanded again.
#[must_use]
pub fn config_home(
    cmd: &ResolvedCommand,
    tool: ToolKind,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Resolved {
    config_home_resolution(cmd, tool, lookup).home
}

/// Resolve the store and preserve whether the tool-specific variable selected
/// it. `_run` records that mode so a resume cannot switch Claude state files.
pub(crate) fn config_home_resolution(
    cmd: &ResolvedCommand,
    tool: ToolKind,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> ConfigHomeResolution {
    let adapter = tool.adapter();
    let (Some(variable), Some(default)) = (adapter.config_home_env, adapter.config_home_default)
    else {
        return ConfigHomeResolution {
            home: Resolved::Absent,
            base: Resolved::Absent,
            explicit: false,
        };
    };
    let environment = match command_environment(cmd, lookup) {
        Ok(environment) => environment,
        Err(why) => {
            return ConfigHomeResolution {
                home: Resolved::Unknown(why.clone()),
                base: Resolved::Unknown(why),
                explicit: false,
            };
        }
    };
    let base = effective_home(&environment, lookup);
    if let Some(value) = environment.value(variable, lookup) {
        return ConfigHomeResolution {
            home: absolute_value(variable, &value),
            base,
            explicit: true,
        };
    }
    let home = default_home(&base, default);
    ConfigHomeResolution {
        home,
        base,
        explicit: false,
    }
}

struct CommandEnvironment {
    clear: bool,
    unset: Vec<String>,
    assignments: Vec<(String, String)>,
}

impl CommandEnvironment {
    fn value(&self, name: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        if let Some((_, value)) = self
            .assignments
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == name)
        {
            return Some(value.clone());
        }
        if self.clear || self.unset.iter().any(|candidate| candidate == name) {
            None
        } else {
            lookup(name)
        }
    }
}

fn command_environment(
    cmd: &ResolvedCommand,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<CommandEnvironment, String> {
    let words = crate::words::split_words(cmd.as_str(), &|name| lookup(name))?;
    let mut index = 0;
    let mut clear = false;
    let mut unset: Vec<String> = Vec::new();
    let mut assignments: Vec<(String, String)> = Vec::new();
    while words.get(index).is_some_and(|word| word.assignment) {
        if let Some((name, value)) = words[index].value.split_once('=') {
            assignments.push((name.to_owned(), value.to_owned()));
        }
        index += 1;
    }
    if words.get(index).is_some_and(|word| word.value == "env") {
        index += 1;
        while let Some(word) = words.get(index) {
            match word.value.as_str() {
                "-i" => {
                    clear = true;
                    index += 1;
                }
                "-u" => {
                    let Some(name) = words.get(index + 1) else {
                        return Err("env -u has no variable name".to_owned());
                    };
                    unset.push(name.value.clone());
                    index += 2;
                }
                value if is_assignment(value) => {
                    if let Some((name, value)) = value.split_once('=') {
                        assignments.push((name.to_owned(), value.to_owned()));
                    }
                    index += 1;
                }
                _ => break,
            }
        }
    }
    Ok(CommandEnvironment {
        clear,
        unset,
        assignments,
    })
}

fn effective_home(
    environment: &CommandEnvironment,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Resolved {
    let home = environment.value("HOME", lookup);
    let Some(home) = home else {
        return Resolved::Absent;
    };
    absolute_value("HOME", &home)
}

fn default_home(base: &Resolved, default: &str) -> Resolved {
    match base {
        Resolved::Path(home) => Resolved::Path(home.join(default)),
        other => other.clone(),
    }
}

fn absolute_value(variable: &str, value: &str) -> Resolved {
    if value.is_empty() {
        return Resolved::Unknown(format!("{variable} is empty or unresolved"));
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Resolved::Unknown(format!("{variable} is not an absolute path: {value}"));
    }
    Resolved::Path(path.to_path_buf())
}

/// A word the lexer built: the raw source span and its quote-resolved value.
struct Word {
    /// Byte-exact raw source of the word.
    raw: String,
    /// Char index where this word starts in the source.
    start: usize,
    /// Char index one past this word's last char.
    end: usize,
    /// The word with quotes and escapes resolved.
    unquoted: String,
    /// A leading `NAME=value` assignment (decided on the RAW word, so a quoted
    /// `'A=1'` is a command word, as in bash).
    assignment: bool,
}

/// Enforce the one-simple-command grammar over the WHOLE line: optional leading
/// `NAME=value` assignments, then exactly one argv vector of words.
///
/// # Errors
///
/// The first [`Refusal`] met, left to right.
pub fn lex_simple_command(cmd: &str) -> Result<SimpleCommand, Refusal> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut lexed: Vec<Word> = Vec::new();
    loop {
        while i < n && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let wstart = i;
        let (unquoted, end) = scan_word(&chars, wstart)?;
        i = end;
        let raw: String = chars[wstart..i].iter().collect();
        let assignment = lexed.iter().all(|w| w.assignment) && is_assignment(&raw);
        lexed.push(Word {
            raw,
            start: wstart,
            end: i,
            unquoted,
            assignment,
        });
    }
    let argv_start = lexed
        .iter()
        .position(|w| !w.assignment)
        .unwrap_or(lexed.len());
    if argv_start >= lexed.len() {
        return Err(Refusal::NoCommand);
    }
    let assignments: Vec<String> = lexed[..argv_start].iter().map(|w| w.raw.clone()).collect();
    let words: Vec<String> = lexed[argv_start..].iter().map(|w| w.raw.clone()).collect();
    // Byte-exact spans: slice the ORIGINAL source from the first word's start to
    // the last word's end, so tabs and runs of spaces between words survive.
    let span = |slice: &[Word]| -> String {
        match (slice.first(), slice.last()) {
            (Some(first), Some(last)) => chars[first.start..last.end].iter().collect(),
            _ => String::new(),
        }
    };
    let assign_span = span(&lexed[..argv_start]);
    let argv_span = span(&lexed[argv_start..]);
    let binary_index = launch_binary_index(&lexed[argv_start..]).ok_or(Refusal::NoCommand)?;
    let binary_word = &lexed[argv_start + binary_index];
    let binary = binary_word
        .unquoted
        .rsplit('/')
        .next()
        .unwrap_or(&binary_word.unquoted)
        .to_owned();
    let byte_at = |char_index: usize| {
        cmd.char_indices()
            .nth(char_index)
            .map_or(cmd.len(), |(byte, _)| byte)
    };
    Ok(SimpleCommand {
        assignments,
        words,
        assign_span,
        argv_span,
        binary,
        word_values: lexed[argv_start..]
            .iter()
            .map(|word| word.unquoted.clone())
            .collect(),
        binary_index,
        binary_span: (byte_at(binary_word.start), byte_at(binary_word.end)),
    })
}

/// Scan ONE word from `chars` starting at `start` (which is not whitespace),
/// returning its quote-resolved value and the index one past its last char.
fn scan_word(chars: &[char], start: usize) -> Result<(String, usize), Refusal> {
    let n = chars.len();
    let mut i = start;
    let mut unquoted = String::new();
    let mut quote: Option<char> = None;
    let mut at_word_start = true;
    while i < n {
        let ch = chars[i];
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    unquoted.push(ch);
                }
            }
            Some(_) => {
                // Double quotes keep substitution ACTIVE (measured:
                // "$(printf X)" runs), so it is refused here too; only a
                // single-quoted or escaped form is plain bytes.
                if ch == '\\' {
                    let Some(&next) = chars.get(i + 1) else {
                        return Err(Refusal::TrailingBackslash);
                    };
                    // In "…" bash keeps `\` literal except before " \ $ ` .
                    match next {
                        '"' | '\\' | '$' | '`' => unquoted.push(next),
                        _ => {
                            unquoted.push('\\');
                            unquoted.push(next);
                        }
                    }
                    i += 1;
                } else if ch == '"' {
                    quote = None;
                } else if ch == '`' {
                    return Err(Refusal::Substitution("`".to_owned()));
                } else if ch == '$' && chars.get(i + 1) == Some(&'(') {
                    return Err(Refusal::Substitution("$(".to_owned()));
                } else if ch == '$' && chars.get(i + 1) == Some(&'{') {
                    i = scan_param_expansion(chars, i)?;
                    // Scan lands on the closing `}`; push the whole span raw.
                    unquoted.push('$');
                } else {
                    unquoted.push(ch);
                }
            }
            None => match ch {
                '\\' => {
                    let Some(&next) = chars.get(i + 1) else {
                        return Err(Refusal::TrailingBackslash);
                    };
                    unquoted.push(next);
                    i += 1;
                }
                '\'' | '"' => quote = Some(ch),
                ' ' | '\t' => break,
                ';' | '&' | '|' | '\n' => {
                    return Err(Refusal::ControlOperator(ch.to_string()));
                }
                '#' if at_word_start => return Err(Refusal::Comment),
                '<' | '>' => return Err(Refusal::Redirection(ch.to_string())),
                '`' => return Err(Refusal::Substitution("`".to_owned())),
                '$' if chars.get(i + 1) == Some(&'(') => {
                    return Err(Refusal::Substitution("$(".to_owned()));
                }
                '$' if chars.get(i + 1) == Some(&'{') => {
                    // Parameter expansion `${…}`: scan to its matching close,
                    // refusing an active `$(`/backtick nested inside it.
                    i = scan_param_expansion(chars, i)?;
                    unquoted.push('$');
                }
                '(' | ')' | '{' | '}' => return Err(Refusal::Grouping(ch.to_string())),
                _ => unquoted.push(ch),
            },
        }
        at_word_start = false;
        i += 1;
    }
    if quote.is_some() {
        return Err(Refusal::UnterminatedQuote);
    }
    Ok((unquoted, i))
}

/// Scan a `${…}` parameter expansion starting at the `$`, returning the index
/// of its closing `}`.
///
/// The grammar is [`crate::words::scan_param`]'s and this is only its refusal
/// vocabulary: the module that RUNS a launch command and the one that VALIDATES
/// it must accept the same spans, or a profile plans green and exits 1 in its
/// own pane.
fn scan_param_expansion(chars: &[char], dollar: usize) -> Result<usize, Refusal> {
    use crate::words::ParamFault;

    crate::words::scan_param(chars, dollar)
        .map(|param| param.close)
        .map_err(|fault| match fault {
            ParamFault::Substitution(op) => Refusal::Substitution(op),
            ParamFault::Unclosed => Refusal::Grouping("${".to_owned()),
            ParamFault::Unsupported => Refusal::Parameter,
        })
}

/// The harness word an argv actually runs: a contextual `env` prefix (its `-i`,
/// `-u NAME` options and `VAR=value` words) is peeled, then the next word is
/// the binary, quoting resolved and path stripped.
fn launch_binary_index(argv: &[Word]) -> Option<usize> {
    let unq: Vec<&str> = argv.iter().map(|w| w.unquoted.as_str()).collect();
    let mut i = 0;
    if unq.first() == Some(&"env") {
        i = 1;
        while i < unq.len() {
            match unq[i] {
                "-i" => i += 1,
                "-u" => i += 2,
                w if is_assignment(w) => i += 1,
                _ => break,
            }
        }
    }
    let word = unq.get(i).copied().unwrap_or("");
    if word.is_empty() || word.rsplit('/').next().unwrap_or(word).is_empty() {
        None
    } else {
        Some(i)
    }
}

/// `NAME=…` with a shell variable name before the `=` (the raw word, so a
/// quoted `'A=1'` is a command word, as in bash).
pub(crate) fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Refusal, Resolved, Split, ToolKind, config_home, config_home_resolution,
        lex_simple_command, split_binary,
    };

    fn bin(cmd: &str) -> Option<String> {
        split_binary(cmd).map(|s| s.binary_name().to_owned())
    }

    #[test]
    fn the_live_profile_shapes_classify_as_the_frozen_bash_does() {
        // Every shape in the operator's live config (2026-09-02), plus the
        // README's identity pattern.
        let cases = [
            (
                "claude --permission-mode bypassPermissions --model opus[1m]",
                "claude",
                ToolKind::Claude,
            ),
            (
                "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh",
                "codex",
                ToolKind::Codex,
            ),
            (
                "CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --permission-mode bypassPermissions --model fable --effort xhigh",
                "claude",
                ToolKind::Claude,
            ),
            (
                "grok --always-approve -m grok-4.6 --effort high",
                "grok",
                ToolKind::Grok,
            ),
            (
                "opencode -m google/gemini-3-pro-preview",
                "opencode",
                ToolKind::OpenCode,
            ),
            (
                "gemini --yolo -m gemini-2.5-pro",
                "gemini",
                ToolKind::Gemini,
            ),
            // The operator's live agy profile (2026-09-04).
            ("agy --dangerously-skip-permissions", "agy", ToolKind::Agy),
            // The env launcher and its two option shapes.
            (
                "env OPENCODE_CONFIG=/x/y.json opencode",
                "opencode",
                ToolKind::OpenCode,
            ),
            ("env -u FOO claude", "claude", ToolKind::Claude),
            (
                "env -i A=1 B=2 /opt/homebrew/bin/codex -a never",
                "codex",
                ToolKind::Codex,
            ),
            // Quoting around the binary word resolves; the path is stripped.
            (
                "'/Applications/Some Dir/claude' --x",
                "claude",
                ToolKind::Claude,
            ),
            ("\"$HOME/bin/grok\" --effort high", "grok", ToolKind::Grok),
            ("X='a b' claude", "claude", ToolKind::Claude),
            // The tail is not lexed: a backslash inside a comment is fine.
            ("codex --yolo # note\\", "codex", ToolKind::Codex),
            ("mytool --flag", "mytool", ToolKind::Unknown),
        ];
        for (cmd, want_bin, want_kind) in cases {
            assert_eq!(bin(cmd).as_deref(), Some(want_bin), "{cmd:?}");
            assert_eq!(ToolKind::from_cmd(cmd), want_kind, "{cmd:?}");
        }
    }

    #[test]
    fn malformed_or_prefix_only_lines_have_no_binary() {
        // Fail toward unknown: an unterminated quote, a trailing bare
        // backslash, and nothing but prefix words.
        for cmd in [
            "'claude --x",
            "\"claude",
            "claude\\",
            "A=1\\",
            "",
            "   ",
            "env",
            "env -i A=1",
            "env -u",
            "A=1 B=2",
            "\"$HOME/bin\\",
        ] {
            assert_eq!(split_binary(cmd), None, "{cmd:?}");
            assert_eq!(ToolKind::from_cmd(cmd), ToolKind::Unknown, "{cmd:?}");
        }
    }

    #[test]
    fn the_split_keeps_prefix_and_tail_verbatim() {
        let split = split_binary("A=1  env -u B\t'/p q/claude' --model x  ").unwrap();
        assert_eq!(
            split,
            Split {
                prefix: "A=1  env -u B\t".to_owned(),
                binary_raw: "'/p q/claude'".to_owned(),
                binary: "/p q/claude".to_owned(),
                rest: " --model x  ".to_owned(),
            }
        );
        assert_eq!(split.binary_name(), "claude");
        // Double-quote escapes: only the four bytes and a newline are special.
        let split = split_binary("\"a\\$b\\\"c\\\\d\\x\" tail").unwrap();
        assert_eq!(split.binary, "a$b\"c\\d\\x");
        assert_eq!(split.rest, " tail");
        // `-u` consumes exactly one word, whatever it looks like.
        assert_eq!(bin("env -u claude codex"), Some("codex".to_owned()));
        // `-i` consumes none.
        assert_eq!(bin("env -i claude"), Some("claude".to_owned()));
        // Characters are characters, not bytes.
        assert_eq!(bin("Ä=ö  cödex --x"), Some("cödex".to_owned()));
    }

    #[test]
    fn tool_kind_spellings_are_the_frozen_ones() {
        for (kind, text) in [
            (ToolKind::Claude, "claude"),
            (ToolKind::Codex, "codex"),
            (ToolKind::Gemini, "gemini"),
            (ToolKind::Agy, "agy"),
            (ToolKind::Grok, "grok"),
            (ToolKind::OpenCode, "opencode"),
            (ToolKind::Unknown, "unknown"),
        ] {
            assert_eq!(kind.as_str(), text);
            assert_eq!(
                ToolKind::from_binary_name(text),
                if text == "unknown" {
                    ToolKind::Unknown
                } else {
                    kind
                }
            );
        }
        // `pane_current_command` reports `opencode.exe` for the bun launcher —
        // that is a PANE fact, not a command-line one; the lexer sees the
        // command as written.
        assert_eq!(
            ToolKind::from_binary_name("opencode.exe"),
            ToolKind::Unknown
        );
    }

    #[test]
    fn the_live_profile_shapes_are_one_simple_command() {
        let cases: [(&str, &[&str], usize, &str, ToolKind); 8] = [
            (
                "claude --permission-mode bypassPermissions --model opus[1m]",
                &[],
                5,
                "claude",
                ToolKind::Claude,
            ),
            (
                "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh",
                &[],
                6,
                "codex",
                ToolKind::Codex,
            ),
            (
                "CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --permission-mode bypassPermissions --model fable --effort xhigh",
                &["CLAUDE_CONFIG_DIR=$HOME/.claude-mic"],
                7,
                "claude",
                ToolKind::Claude,
            ),
            (
                "grok --always-approve -m grok-4.6 --effort high",
                &[],
                6,
                "grok",
                ToolKind::Grok,
            ),
            (
                "opencode -m google/gemini-3-pro-preview",
                &[],
                3,
                "opencode",
                ToolKind::OpenCode,
            ),
            (
                "A=1 B='x y' env -u C \"${HOME}/bin/claude\" --model ~/m",
                &["A=1", "B='x y'"],
                6,
                "claude",
                ToolKind::Claude,
            ),
            // Quoted forms of every refused character are plain bytes.
            (
                r#"claude --x '# ; | > && $(x) `y` ( ) { }' "a;b|c" \; \#"#,
                &[],
                6,
                "claude",
                ToolKind::Claude,
            ),
            ("codex -c 'k=v'", &[], 3, "codex", ToolKind::Codex),
        ];
        for (cmd, assignments, words, binary, tool) in cases {
            let sc = lex_simple_command(cmd).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
            assert_eq!(sc.assignments, assignments, "{cmd:?}");
            assert_eq!(sc.words.len(), words, "{cmd:?}: {:?}", sc.words);
            assert_eq!(sc.binary, binary, "binary of {cmd:?}");
            assert_eq!(sc.tool(), tool, "tool of {cmd:?}");
            // The spans recompose the source: assign + one space (if any) + argv.
            let recomposed = if sc.assign_span.is_empty() {
                sc.argv_span.clone()
            } else {
                format!("{} {}", sc.assign_span, sc.argv_span)
            };
            assert!(
                cmd.contains(&sc.argv_span),
                "argv_span is a source slice: {cmd:?}"
            );
            let _ = recomposed;
        }
    }

    #[test]
    fn a_contextual_env_prefix_with_nothing_after_it_is_no_command() {
        // Colead round-2 IMPORTANT-2: the peel must not yield an empty binary.
        for cmd in [
            "env",
            "env -i",
            "env -u FOO",
            "env A=1",
            "env -i A=1 -u B",
            "''",
            "env ''",
        ] {
            assert_eq!(lex_simple_command(cmd), Err(Refusal::NoCommand), "{cmd:?}");
        }
        // Controls: a prefix followed by a word is that word.
        assert_eq!(lex_simple_command("env -i x").unwrap().binary, "x");
        assert_eq!(lex_simple_command("env A=1 ./y").unwrap().binary, "y");
    }

    #[test]
    fn the_launch_binary_comes_from_the_validated_parse_not_the_frozen_heuristic() {
        // Colead P2 IMPORTANT-2: split_binary skips ANY `=`-word, so it labels
        // these as the wrong tool; the validated parse takes the real command word.
        for (cmd, binary, tool) in [
            ("--flag=foo claude", "--flag=foo", ToolKind::Unknown),
            ("A=1 -i claude", "-i", ToolKind::Unknown),
            (
                "env OPENCODE_CONFIG=/x/y.json opencode",
                "opencode",
                ToolKind::OpenCode,
            ),
            ("env -u FOO -i claude", "claude", ToolKind::Claude),
            (
                "env A=1 B=2 /opt/bin/codex -a never",
                "codex",
                ToolKind::Codex,
            ),
            ("'/Apps/My Dir/claude' --x", "claude", ToolKind::Claude),
        ] {
            let sc = lex_simple_command(cmd).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
            assert_eq!(sc.binary, binary, "binary of {cmd:?}");
            assert_eq!(sc.tool(), tool, "tool of {cmd:?}");
        }
    }

    #[test]
    fn byte_exact_spans_survive_tabs_and_runs_of_spaces() {
        // Colead P2 IMPORTANT-1: the spans are the exact source, not rejoined.
        let sc = lex_simple_command("A=1	B=2   claude   --model	x").unwrap();
        assert_eq!(sc.assign_span, "A=1	B=2");
        assert_eq!(sc.argv_span, "claude   --model	x");
        let sc = lex_simple_command("claude --x").unwrap();
        assert_eq!(sc.assign_span, "");
        assert_eq!(sc.argv_span, "claude --x");
        // A quoted assignment is a command word, so the argv span starts there.
        let sc = lex_simple_command("'A=1' claude").unwrap();
        assert_eq!(sc.assign_span, "");
        assert_eq!(sc.argv_span, "'A=1' claude");
    }

    #[test]
    fn the_parameter_grammar_is_the_one_the_runner_can_expand() {
        // `${VAR}` is parameter expansion, not grouping, and `$VAR` is fine.
        assert!(lex_simple_command("claude --dir ${HOME}/x --y $Y").is_ok());
        // The four conditional forms `crate::words` DOES expand stay accepted…
        for cmd in [
            "claude ${X:-default}",
            "claude ${X-default}",
            "claude ${X:+alternate}",
            "claude ${X+alternate}",
        ] {
            assert!(lex_simple_command(cmd).is_ok(), "{cmd:?}");
        }
        // …and colead Z2 BLOCKER-3, the validator half: a form the runner
        // cannot expand is refused HERE, at plan time, where the operator can
        // still fix the profile — never accepted here and refused there.
        for cmd in [
            "claude ${OUTER${INNER}}",
            "claude ${24}fallback",
            "claude ${X:=assign}",
            "claude ${X:?fail}",
            "claude ${#X}",
            "claude ${X#prefix}",
            "claude ${X/a/b}",
        ] {
            assert_eq!(lex_simple_command(cmd), Err(Refusal::Parameter), "{cmd:?}");
        }
        // An unclosed `${` is still grouping, not a parameter form.
        assert_eq!(
            lex_simple_command("claude ${HOME"),
            Err(Refusal::Grouping("${".to_owned()))
        );
    }

    #[test]
    fn anything_but_one_simple_command_is_refused_with_its_reason() {
        let cases = [
            ("claude --x # note", Refusal::Comment),
            (
                "claude ; rm -rf x",
                Refusal::ControlOperator(";".to_owned()),
            ),
            ("claude & ", Refusal::ControlOperator("&".to_owned())),
            ("claude && codex", Refusal::ControlOperator("&".to_owned())),
            ("claude || codex", Refusal::ControlOperator("|".to_owned())),
            ("claude | tee log", Refusal::ControlOperator("|".to_owned())),
            ("claude\ncodex", Refusal::ControlOperator("\n".to_owned())),
            ("claude > out", Refusal::Redirection(">".to_owned())),
            ("claude 2>err", Refusal::Redirection(">".to_owned())),
            ("claude >> out", Refusal::Redirection(">".to_owned())),
            ("claude < in", Refusal::Redirection("<".to_owned())),
            ("claude <(x)", Refusal::Redirection("<".to_owned())),
            ("claude $(which x)", Refusal::Substitution("$(".to_owned())),
            ("claude `which x`", Refusal::Substitution("`".to_owned())),
            // Active inside double quotes too (measured).
            (
                "claude \"$(printf X)\"",
                Refusal::Substitution("$(".to_owned()),
            ),
            ("claude \"a `x` b\"", Refusal::Substitution("`".to_owned())),
            ("(claude)", Refusal::Grouping("(".to_owned())),
            ("{ claude; }", Refusal::Grouping("{".to_owned())),
            (
                "claude --model opus{1,2}",
                Refusal::Grouping("{".to_owned()),
            ),
            ("'claude", Refusal::UnterminatedQuote),
            ("claude \"x", Refusal::UnterminatedQuote),
            ("claude\\", Refusal::TrailingBackslash),
            ("claude \"a\\", Refusal::TrailingBackslash),
            ("", Refusal::NoCommand),
            ("   ", Refusal::NoCommand),
            ("A=1 B=2", Refusal::NoCommand),
        ];
        for (cmd, want) in cases {
            assert_eq!(lex_simple_command(cmd), Err(want), "{cmd:?}");
        }
        // An active substitution nested in ${…} is refused
        // (bash runs it — measured X=HIT reaches env), quoted or not.
        assert_eq!(
            lex_simple_command("X=${X:-$(printf HIT)} claude"),
            Err(Refusal::Substitution("$(".to_owned()))
        );
        assert_eq!(
            lex_simple_command("claude ${X:-`id`}"),
            Err(Refusal::Substitution("`".to_owned()))
        );
        assert_eq!(
            lex_simple_command("claude \"${X:-$(id)}\""),
            Err(Refusal::Substitution("$(".to_owned()))
        );
        // Colead round-2 BLOCKER-1: process substitution nested in ${…} runs
        // too (measured: `${X:-<(printf HIT)}` then `cat "$X"` prints HIT).
        assert_eq!(
            lex_simple_command("X=${X:-<(printf HIT)} claude"),
            Err(Refusal::Substitution("<(".to_owned()))
        );
        assert_eq!(
            lex_simple_command("claude ${X:->(consumer)}"),
            Err(Refusal::Substitution(">(".to_owned()))
        );
        assert_eq!(
            lex_simple_command("claude \"${X:-<(id)}\""),
            Err(Refusal::Substitution("<(".to_owned()))
        );
        // Escaped and single-quoted forms are plain bytes — controls.
        assert!(lex_simple_command(r"claude ${X:-\<(x)}").is_ok());
        assert!(lex_simple_command("claude '${X:-<(x)}'").is_ok());
        assert!(
            lex_simple_command("claude ${X:-a<b}").is_ok(),
            "a bare < is a byte here"
        );
        // A word that merely CONTAINS `#` is not a comment.
        assert!(lex_simple_command("claude --tag a#b").is_ok());
        // Single-quoted or escaped substitution syntax is plain bytes; the
        // double-quoted form is not (see the refusals above).
        assert!(lex_simple_command("claude '$(x)' '`y`' \\$\\(z\\) \\`w\\`").is_ok());
        // An escaped `$` before a bare `(` is still a bare `(` — invalid shell,
        // refused as grouping (bash: syntax error near unexpected token `(`).
        assert_eq!(
            lex_simple_command("claude \\$(z)"),
            Err(Refusal::Grouping("(".to_owned()))
        );
        assert!(lex_simple_command("claude \"${HOME}/x\" \"\\$(no)\"").is_ok());
        // A quoted assignment is a command word, as in bash.
        assert_eq!(
            lex_simple_command("'A=1' claude").unwrap().words,
            ["'A=1'", "claude"]
        );
    }

    #[test]
    fn config_home_obeys_the_executor_environment_order() {
        let resolve = |command: &str, tool: ToolKind, inherited: &[(&str, &str)]| {
            let command = crate::config::IdentityConfig::resolved_snapshot(command);
            config_home(&command, tool, &|name| {
                inherited
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        let home = [("HOME", "/pane"), ("CODEX_HOME", "/inherited/codex")];
        assert_eq!(
            resolve("codex", ToolKind::Codex, &home),
            Resolved::Path(PathBuf::from("/inherited/codex"))
        );
        assert_eq!(
            resolve("HOME=/other codex", ToolKind::Codex, &home),
            Resolved::Path(PathBuf::from("/inherited/codex")),
            "the inherited account variable still outranks HOME"
        );
        assert_eq!(
            resolve("HOME=/other codex", ToolKind::Codex, &[("HOME", "/pane")]),
            Resolved::Path(PathBuf::from("/other/.codex")),
            "a prefix HOME selects the default store when no account variable exists"
        );
        assert_eq!(
            resolve(
                "env -u CODEX_HOME HOME=/other codex",
                ToolKind::Codex,
                &home
            ),
            Resolved::Path(PathBuf::from("/other/.codex"))
        );
        assert_eq!(
            resolve("env -i CODEX_HOME=/explicit codex", ToolKind::Codex, &home),
            Resolved::Path(PathBuf::from("/explicit"))
        );
        assert_eq!(
            resolve("env -i codex", ToolKind::Codex, &home),
            Resolved::Absent
        );
        assert_eq!(
            resolve("CODEX_HOME=relative codex", ToolKind::Codex, &home),
            Resolved::Unknown("CODEX_HOME is not an absolute path: relative".to_owned())
        );
        assert_eq!(
            resolve("CODEX_HOME=$MISSING codex", ToolKind::Codex, &home),
            Resolved::Unknown("CODEX_HOME is empty or unresolved".to_owned())
        );
        assert_eq!(
            resolve(
                "CLAUDE_CONFIG_DIR=${ACCOUNT} claude",
                ToolKind::Claude,
                &[("HOME", "/pane"), ("ACCOUNT", "/accounts/claude")]
            ),
            Resolved::Path(PathBuf::from("/accounts/claude"))
        );

        let command = crate::config::IdentityConfig::resolved_snapshot("HOME=/other codex");
        let implicit = config_home_resolution(&command, ToolKind::Codex, &|name| {
            (name == "HOME").then(|| "/pane".to_owned())
        });
        assert_eq!(
            implicit.home,
            Resolved::Path(PathBuf::from("/other/.codex"))
        );
        assert_eq!(implicit.base, Resolved::Path(PathBuf::from("/other")));
        assert!(!implicit.explicit);

        let command = crate::config::IdentityConfig::resolved_snapshot(
            "HOME=/other CODEX_HOME=/account codex",
        );
        let explicit = config_home_resolution(&command, ToolKind::Codex, &|_| None);
        assert_eq!(explicit.home, Resolved::Path(PathBuf::from("/account")));
        assert_eq!(explicit.base, Resolved::Path(PathBuf::from("/other")));
        assert!(explicit.explicit);
    }
}
