//! The shell-word lexer that turns a composed launch command into an argv.
//!
//! Until slice Z2 a pane ran `bash -lc '<launch command>'`, so the shell did
//! this work. The pane's command is now the core itself, and the core `exec`s
//! the tool DIRECTLY — which is the whole reason `pane_current_command` still
//! reports the tool. So the one shell service the launch actually used has to
//! be here: split a command line into words, honouring quotes, escapes and
//! parameter expansion.
//!
//! # The grammar, and what is deliberately NOT in it
//!
//! Quoting matches [`crate::launch_cmd`]'s lexer exactly, so the module that
//! CLASSIFIES a profile command and the one that RUNS it never disagree about
//! where a word ends: single quotes are wholly literal, double quotes escape
//! only `"`, `\`, `$`, `` ` `` and a newline, and a bare backslash escapes one
//! character.
//!
//! `$NAME` and `${NAME}` expand from the process environment because operators
//! really do write `$HOME` in a profile command — [`crate::launch_cmd`]'s own
//! module doc says so. An unset name expands to nothing, exactly as the
//! `bash -lc` this replaces did.
//!
//! Everything else a shell would do is REFUSED, never silently skipped:
//! command substitution, control operators, redirection and subshells all end
//! the parse with the character named. A profile command that needs a pipeline
//! is a profile command that needs a wrapper script, and refusing loudly beats
//! `exec`ing a tool with `|` as an argument. Two shell behaviours are dropped
//! rather than refused, both stated here because they are silent: an expanded
//! value is NOT field-split into more words, and no word is glob-expanded.

/// Split `cmd` into words.
///
/// # Errors
///
/// The reason, ready to print after `ae: `: an unterminated quote, a trailing
/// backslash, or a shell construct this lexer does not implement.
pub fn split(cmd: &str, env: &impl Fn(&str) -> Option<String>) -> Result<Vec<String>, String> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == ' ' || chars[index] == '\t' {
            index += 1;
            continue;
        }
        let (word, next) = one_word(&chars, index, env)?;
        words.push(word);
        index = next;
    }
    Ok(words)
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
                let (text, next) = expansion(chars, index + 1, env)?;
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
                let (expanded, next) = expansion(chars, index + 1, env)?;
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

/// `$NAME` or `${NAME}` starting after the `$`.
fn expansion(
    chars: &[char],
    start: usize,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(String, usize), String> {
    let braced = chars.get(start) == Some(&'{');
    if chars.get(start) == Some(&'(') {
        return Err(
            "a $( ) substitution in a launch command — ae execs the tool directly and runs no shell"
                .to_owned(),
        );
    }
    let mut index = start + usize::from(braced);
    let name_start = index;
    if matches!(chars.get(index), Some(ch) if ch.is_ascii_alphabetic() || *ch == '_') {
        index += 1;
        while matches!(chars.get(index), Some(ch) if ch.is_ascii_alphanumeric() || *ch == '_') {
            index += 1;
        }
    }
    let name: String = chars[name_start..index].iter().collect();
    if name.is_empty() {
        // A lone `$` is an ordinary character to a shell; anything else after a
        // `${` is a parameter form this lexer does not implement.
        if braced {
            return Err(
                "a ${…} parameter form ae does not implement in a launch command".to_owned(),
            );
        }
        return Ok(("$".to_owned(), start));
    }
    if braced {
        if chars.get(index) != Some(&'}') {
            return Err(
                "a ${…} parameter form ae does not implement in a launch command".to_owned(),
            );
        }
        index += 1;
    }
    Ok((env(&name).unwrap_or_default(), index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/a".to_owned()),
            "FLAGS" => Some("--one --two".to_owned()),
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
            "a ${x:-y}",
        ] {
            assert!(split(cmd, &env).is_err(), "{cmd} must refuse");
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
