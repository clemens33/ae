//! The shell-word lexer that turns a composed launch command into an argv.
//!
//! Until slice Z2 a pane ran `bash -lc '<launch command>'`, so the shell did
//! this work. The pane's command is now the core itself, and the core `exec`s
//! the tool DIRECTLY — which is the whole reason `pane_current_command` still
//! reports the tool. So the one shell service the launch actually used has to
//! be here: split a command line into words, honouring quotes, escapes and
//! parameter expansion.

/// One lexed word: what the source said, and what the `exec` receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// Byte-exact source span of the word, quoting and escapes included.
    pub raw: String,
    /// The word after quote removal and expansion — what is `exec`ed.
    pub value: String,
    /// Is this a `NAME=value` assignment *as written*?
    pub assignment: bool,
}

/// Split `cmd` into words, keeping each word's source span beside its value.
///
/// # Errors
///
/// The reason, ready to print after `ae: `: an unterminated quote, a trailing
/// backslash, or a shell construct this lexer does not implement.
pub fn split_words(cmd: &str, env: &impl Fn(&str) -> Option<String>) -> Result<Vec<Word>, String> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut words: Vec<Word> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == ' ' || chars[index] == '\t' {
            index += 1;
            continue;
        }
        let (value, next) = one_word(&chars, index, env)?;
        let raw: String = chars[index..next].iter().collect();
        words.push(Word {
            assignment: crate::launch_cmd::is_assignment(&raw),
            raw,
            value,
        });
        index = next;
    }
    Ok(words)
}

/// Split `cmd` into the values alone — [`split_words`] for a caller that has no
/// question about quoting.
///
/// # Errors
///
/// As [`split_words`].
pub fn split(cmd: &str, env: &impl Fn(&str) -> Option<String>) -> Result<Vec<String>, String> {
    Ok(split_words(cmd, env)?
        .into_iter()
        .map(|word| word.value)
        .collect())
}

/// Lex one word starting at `start`, returning it and the index after it.
fn one_word(
    chars: &[char],
    start: usize,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(String, usize), String> {
    let mut word = String::new();
    let mut index = start;
    while index < chars.len() {
        match chars[index] {
            ' ' | '\t' => break,
            '\'' => {
                let (text, next) = single_quoted(chars, index + 1)?;
                word.push_str(&text);
                index = next;
            }
            '"' => {
                let (text, next) = double_quoted(chars, index + 1, env)?;
                word.push_str(&text);
                index = next;
            }
            '\\' => {
                let Some(escaped) = chars.get(index + 1) else {
                    return Err("a command line ending in a bare backslash".to_owned());
                };
                word.push(*escaped);
                index += 2;
            }
            '$' => {
                let (text, next) = expansion(chars, index, env)?;
                word.push_str(&text);
                index = next;
            }
            // A leading `~` is the one path shorthand a profile command is
            // likely to carry; anywhere else in a word it is an ordinary
            // character, exactly as a shell reads it.
            '~' if index == start
                && matches!(chars.get(index + 1), None | Some(' ' | '\t' | '/')) =>
            {
                word.push_str(&env("HOME").unwrap_or_default());
                index += 1;
            }
            ch @ ('|' | '&' | ';' | '<' | '>' | '(' | ')' | '`' | '\n') => {
                return Err(format!(
                    "'{ch}' in a launch command — ae execs the tool directly and runs no shell, so pipelines, redirection and command substitution are not available here"
                ));
            }
            // Brace expansion and grouping, which the VALIDATOR refuses as
            // `Refusal::Grouping`.
            ch @ ('{' | '}') => {
                return Err(format!(
                    "'{ch}' in a launch command — ae execs the tool directly and runs no shell, so brace expansion and grouping are not available here"
                ));
            }
            // A word-initial `#`, which the validator refuses as
            // `Refusal::Comment`.
            '#' if index == start => {
                return Err(
                    "a '#' starting a word in a launch command — ae execs the tool directly and runs no shell, so there are no comments here".to_owned(),
                );
            }
            ch => {
                word.push(ch);
                index += 1;
            }
        }
    }
    Ok((word, index))
}

/// Everything up to the closing `'`, literally.
fn single_quoted(chars: &[char], start: usize) -> Result<(String, usize), String> {
    let mut text = String::new();
    let mut index = start;
    while index < chars.len() {
        if chars[index] == '\'' {
            return Ok((text, index + 1));
        }
        text.push(chars[index]);
        index += 1;
    }
    Err("an unterminated single quote in a launch command".to_owned())
}

/// Everything up to the closing `"`, with the four escapes and expansion.
fn double_quoted(
    chars: &[char],
    start: usize,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(String, usize), String> {
    let mut text = String::new();
    let mut index = start;
    while index < chars.len() {
        match chars[index] {
            '"' => return Ok((text, index + 1)),
            '\\' => match chars.get(index + 1) {
                // A backslash-newline inside double quotes is a line
                // continuation: both characters go, exactly as
                // `crate::launch_cmd`'s lexer reads it.
                Some('\n') => index += 2,
                Some(ch @ ('"' | '\\' | '$' | '`')) => {
                    text.push(*ch);
                    index += 2;
                }
                Some(_) => {
                    text.push('\\');
                    index += 1;
                }
                None => return Err("a command line ending in a bare backslash".to_owned()),
            },
            '$' => {
                let (expanded, next) = expansion(chars, index, env)?;
                text.push_str(&expanded);
                index = next;
            }
            '`' => {
                return Err(
                    "a backquote in a launch command — ae execs the tool directly and runs no shell"
                        .to_owned(),
                );
            }
            ch => {
                text.push(ch);
                index += 1;
            }
        }
    }
    Err("an unterminated double quote in a launch command".to_owned())
}

/// Why a `${…}` span is refused — the shared vocabulary of the two lexers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamFault {
    /// An active substitution nested inside the braces: `$(`, `` ` ``, `<(` or
    /// `>(`. bash RUNS all four there (measured), so they are refused however
    /// deeply they are nested and whatever quotes surround them.
    Substitution(String),
    /// A `${` whose `}` never arrives.
    Unclosed,
    /// A parameter form neither lexer implements.
    Unsupported,
}

/// Which conditional form a `${…}` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// `${NAME}` — the value, empty when unset.
    Plain,
    /// `${NAME:-word}` / `${NAME-word}` — the operand when unset (`colon`: or
    /// set but empty).
    Default { colon: bool },
    /// `${NAME:+word}` / `${NAME+word}` — the operand when set (`colon`: and
    /// non-empty).
    Alternate { colon: bool },
}

/// One `${…}` span, scanned but not evaluated: where it ends, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// Char index of the closing `}`.
    pub close: usize,
    /// The parameter name.
    name: String,
    /// The form, and whether its test is the `:` one.
    op: Op,
    /// Char range of the operand word — empty for [`Op::Plain`].
    operand: (usize, usize),
}

/// Scan the `${…}` that starts at `dollar`, deciding whether ae implements it.
///
/// # Errors
///
/// The [`ParamFault`] met, which the caller phrases in its own terms.
///
///
/// # Panics
///
/// Never: `dollar` is only ever the index of a `$` whose next char is `{`.
pub fn scan_param(chars: &[char], dollar: usize) -> Result<Param, ParamFault> {
    let close = brace_span(chars, dollar)?;
    let body = dollar + 2;
    let mut index = body;
    if matches!(chars.get(index), Some(ch) if ch.is_ascii_alphabetic() || *ch == '_') {
        index += 1;
        while index < close
            && matches!(chars.get(index), Some(ch) if ch.is_ascii_alphanumeric() || *ch == '_')
        {
            index += 1;
        }
    }
    let name: String = chars[body..index].iter().collect();
    if name.is_empty() {
        // `${1}`, `${#x}`, `${!x}`, `${}` — positional parameters, length,
        // indirection.
        return Err(ParamFault::Unsupported);
    }
    if index == close {
        return Ok(Param {
            close,
            name,
            op: Op::Plain,
            operand: (index, index),
        });
    }
    let colon = chars.get(index) == Some(&':');
    let at = index + usize::from(colon);
    let op = match chars.get(at) {
        Some('-') => Op::Default { colon },
        Some('+') => Op::Alternate { colon },
        // `:=` and `:?` assign to or exit the shell; `#`, `%`, `/`, `^`, `,`
        // and the offset form are string operators ae does not implement.
        _ => return Err(ParamFault::Unsupported),
    };
    Ok(Param {
        close,
        name,
        op,
        operand: (at + 1, close),
    })
}

/// The index of the `}` that closes the `${` at `dollar`.
fn brace_span(chars: &[char], dollar: usize) -> Result<usize, ParamFault> {
    let n = chars.len();
    // Dollar -> '$', dollar+1 -> '{'.
    let mut index = dollar + 2;
    let mut depth = 1usize;
    while index < n {
        match chars[index] {
            '\\' => index += 1, // skip the escaped char
            '`' => return Err(ParamFault::Substitution("`".to_owned())),
            '$' if chars.get(index + 1) == Some(&'(') => {
                return Err(ParamFault::Substitution("$(".to_owned()));
            }
            ch @ ('<' | '>') if chars.get(index + 1) == Some(&'(') => {
                return Err(ParamFault::Substitution(format!("{ch}(")));
            }
            '$' if chars.get(index + 1) == Some(&'{') => {
                depth += 1;
                index += 1;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Err(ParamFault::Unclosed)
}

/// The line this lexer prints for a [`ParamFault`].
fn fault_message(fault: &ParamFault) -> String {
    match fault {
        ParamFault::Substitution(op) => format!(
            "a '{op}' substitution inside a ${{…}} in a launch command — ae execs the tool directly and runs no shell"
        ),
        ParamFault::Unclosed => "an unterminated ${…} in a launch command".to_owned(),
        ParamFault::Unsupported => {
            "a ${…} parameter form ae does not implement in a launch command".to_owned()
        }
    }
}

/// `$NAME`, `${NAME}` or a conditional form, starting at the `$` itself.
fn expansion(
    chars: &[char],
    dollar: usize,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(String, usize), String> {
    let start = dollar + 1;
    match chars.get(start) {
        Some('(') => {
            return Err(
                "a $( ) substitution in a launch command — ae execs the tool directly and runs no shell"
                    .to_owned(),
            );
        }
        Some('{') => {
            let param = scan_param(chars, dollar).map_err(|fault| fault_message(&fault))?;
            let value = env(&param.name);
            let text = match param.op {
                Op::Plain => value.unwrap_or_default(),
                Op::Default { colon } => {
                    if present(value.as_deref(), colon) {
                        value.unwrap_or_default()
                    } else {
                        operand(chars, param.operand, env)?
                    }
                }
                Op::Alternate { colon } => {
                    if present(value.as_deref(), colon) {
                        operand(chars, param.operand, env)?
                    } else {
                        String::new()
                    }
                }
            };
            return Ok((text, param.close + 1));
        }
        _ => {}
    }
    let mut index = start;
    if matches!(chars.get(index), Some(ch) if ch.is_ascii_alphabetic() || *ch == '_') {
        index += 1;
        while matches!(chars.get(index), Some(ch) if ch.is_ascii_alphanumeric() || *ch == '_') {
            index += 1;
        }
    }
    if index == start {
        // A lone `$` is an ordinary character to a shell.
        return Ok(("$".to_owned(), start));
    }
    let name: String = chars[start..index].iter().collect();
    Ok((env(&name).unwrap_or_default(), index))
}

/// Does the conditional form treat this value as set?
fn present(value: Option<&str>, colon: bool) -> bool {
    match value {
        Some(text) => !colon || !text.is_empty(),
        None => false,
    }
}

/// Expand the operand of a conditional form.
fn operand(
    chars: &[char],
    (start, end): (usize, usize),
    env: &impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut text = String::new();
    let mut index = start;
    while index < end {
        match chars[index] {
            '\\' => match chars.get(index + 1) {
                Some(ch) if index + 1 < end => {
                    text.push(*ch);
                    index += 2;
                }
                _ => return Err("a command line ending in a bare backslash".to_owned()),
            },
            '$' => {
                let (expanded, next) = expansion(chars, index, env)?;
                text.push_str(&expanded);
                index = next;
            }
            ch => {
                text.push(ch);
                index += 1;
            }
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/a".to_owned()),
            "FLAGS" => Some("--one --two".to_owned()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    fn split_ok(cmd: &str) -> Vec<String> {
        split(cmd, &env).expect("a well-formed command line")
    }

    #[test]
    fn a_plain_command_splits_on_runs_of_whitespace() {
        assert_eq!(split_ok("claude   --yolo\t-x"), ["claude", "--yolo", "-x"]);
        assert_eq!(split_ok("  "), Vec::<String>::new());
    }

    #[test]
    fn the_single_quoted_form_this_crate_emits_round_trips_byte_for_byte() {
        let payload = "a line\nwith 'quotes' and $HOME and \\ backslashes";
        let quoted = crate::launch::shell_quote(payload);
        assert_eq!(split_ok(&format!("tool {quoted}")), ["tool", payload]);
    }

    #[test]
    fn a_parameter_expands_from_the_environment_and_an_unset_one_vanishes() {
        assert_eq!(split_ok("$HOME/bin/claude"), ["/home/a/bin/claude"]);
        assert_eq!(split_ok("x ${HOME}y"), ["x", "/home/ay"]);
        assert_eq!(
            split_ok("a${NOPE}b"),
            ["ab"],
            "an unset name expands to nothing"
        );
    }

    #[test]
    fn an_expanded_value_is_one_word_not_two() {
        assert_eq!(split_ok("tool $FLAGS"), ["tool", "--one --two"]);
    }

    #[test]
    fn a_leading_tilde_is_the_home_directory_and_a_later_one_is_a_character() {
        assert_eq!(split_ok("~/bin/x a~b"), ["/home/a/bin/x", "a~b"]);
    }

    #[test]
    fn every_shell_construct_this_lexer_does_not_run_is_named_not_skipped() {
        for cmd in [
            "a | b",
            "a > f",
            "a && b",
            "a; b",
            "(a)",
            "a `b`",
            "a $(b)",
            "a \"`b`\"",
            "a 'unterminated",
            "a \"unterminated",
            "a \\",
            // Brace expansion and grouping, and a word-initial comment: the
            // VALIDATOR refuses all three, and this lexer used to accept them
            // and hand the literal bytes to an `exec` (colead Z2 BLOCKER-3).
            "claude {a,b}",
            "claude }",
            "claude # note",
            "# claude",
            // The `${…}` forms the shared grammar does not implement.
            "claude ${24}fallback",
            "claude ${X:=assign}",
            "claude ${X:?fail}",
            "claude ${#X}",
            "claude ${OUTER${INNER}}",
            "claude ${HOME",
            "claude ${X:-$(id)}",
            "claude \"${X:-`id`}\"",
        ] {
            assert!(split(cmd, &env).is_err(), "{cmd} must refuse");
        }
    }

    #[test]
    fn the_conditional_parameter_forms_the_validator_accepts_are_expanded_here() {
        // Colead Z2 BLOCKER-3: `lex_simple_command` accepted `${X:-default}`
        // and this lexer refused it, so the seat planned green and then exited
        // 1 in its own pane.
        assert_eq!(split_ok("tool ${NOPE:-default}"), ["tool", "default"]);
        assert_eq!(split_ok("tool ${NOPE-default}"), ["tool", "default"]);
        assert_eq!(split_ok("tool ${HOME:-default}"), ["tool", "/home/a"]);
        assert_eq!(split_ok("tool ${HOME-default}"), ["tool", "/home/a"]);
        // `:` counts an empty value as unset; the bare form does not.
        assert_eq!(split_ok("tool ${EMPTY:-default}"), ["tool", "default"]);
        assert_eq!(split_ok("tool ${EMPTY-default}x"), ["tool", "x"]);
        // The alternate forms, both tests.
        assert_eq!(split_ok("tool ${HOME:+set}"), ["tool", "set"]);
        assert_eq!(split_ok("tool ${EMPTY:+set}b"), ["tool", "b"]);
        assert_eq!(split_ok("tool ${EMPTY+set}"), ["tool", "set"]);
        assert_eq!(split_ok("tool ${NOPE+set}c"), ["tool", "c"]);
        // The operand expands too, and a `}` inside it is not the terminator.
        assert_eq!(split_ok("tool ${NOPE:-$HOME/x}"), ["tool", "/home/a/x"]);
        assert_eq!(split_ok("tool ${NOPE:-${HOME}}"), ["tool", "/home/a"]);
        // …and inside double quotes, where the validator also allows it.
        assert_eq!(split_ok(r#"tool "a ${NOPE:-b} c""#), ["tool", "a b c"]);
    }

    #[test]
    fn the_two_lexers_answer_the_same_question_about_every_parameter_form() {
        // The off-diagonals are the defect: a form one accepts and the other
        // refuses is either a pane that exits 1 after a green plan, or an
        // `exec` of something plan time never saw.
        for cmd in [
            "claude ${HOME}",
            "claude ${X:-default}",
            "claude ${X-default}",
            "claude ${X:+alt}",
            "claude ${X+alt}",
            "claude ${24}fallback",
            "claude ${X:=assign}",
            "claude ${X:?fail}",
            "claude ${#X}",
            "claude ${X#prefix}",
            "claude ${X/a/b}",
            "claude ${OUTER${INNER}}",
            "claude ${HOME",
            "claude ${X:-$(id)}",
            "claude ${X:-<(id)}",
            "claude {a,b}",
            "claude # note",
            "claude 'literal {a,b} # not a comment'",
            "claude --dir ${HOME}/x --y $Y",
        ] {
            assert_eq!(
                split(cmd, &env).is_ok(),
                crate::launch_cmd::lex_simple_command(cmd).is_ok(),
                "{cmd:?}: the validator and the runner disagree"
            );
        }
    }

    #[test]
    fn double_quotes_escape_only_the_four_characters_a_shell_escapes() {
        assert_eq!(split_ok(r#""a\"b\\c\$d""#), [r#"a"b\c$d"#]);
        assert_eq!(split_ok(r#""a\qb""#), [r"a\qb"]);
        // Backslash-newline inside double quotes is a line continuation.
        assert_eq!(split_ok("\"a\\\nb\""), ["ab"]);
    }
}
