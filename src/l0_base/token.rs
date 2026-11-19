//! The attachment token, pure: the wire text a composer sends for an image or
//! a file, the label it shows in its place, and the parsers between them.
//!
//! Wire grammar, no escapes:
//! `[Image #N: "<path>"]` or `[File: "<path>"]`, N in 1..=999999 without a
//! leading zero, `<path>` absolute, UTF-8, at most [`PATH_MAX`] bytes, with no
//! `"` and no control character. The first `"` after the opening one ends the
//! path, so every other character is fine in it. Anything that does not match
//! exactly is plain text, never a token.

/// Attachments one draft holds at once.
pub const SLOTS: usize = 32;
/// The largest image number a token carries.
pub const MAX_N: u32 = 999_999;
/// The longest path a token carries.
pub const PATH_MAX: usize = 4096;
/// The byte an external input can never bring into a draft: a genuine token
/// is this byte and then `0x80 + slot` there.
pub const RESERVED: u8 = 0xFF;
/// The longest file name a label shows.
const NAME_MAX: usize = 40;
/// The longest paste run read as a list of paths.
pub const PASTE_MAX: usize = 16 * 1024;
/// The most paths one paste becomes tokens for.
const PASTE_PATHS: usize = 16;
const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "tiff"];
const FILE_EXTENSIONS: [&str; 5] = ["pdf", "docx", "xlsx", "pptx", "csv"];

/// What a token stands for. An image carries its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image(u32),
    File,
}

/// Whether `path` may sit inside a token.
#[must_use]
pub fn fits(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= PATH_MAX
        && !path.chars().any(|ch| ch == '"' || ch.is_control())
}

/// The wire text of a token, or `None` when its path or number does not fit.
#[must_use]
pub fn wire(kind: Kind, path: &str) -> Option<String> {
    if !fits(path) {
        return None;
    }
    match kind {
        Kind::Image(n) if (1..=MAX_N).contains(&n) => Some(format!("[Image #{n}: \"{path}\"]")),
        Kind::Image(_) => None,
        Kind::File => Some(format!("[File: \"{path}\"]")),
    }
}

/// A run of text, or one token read out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Token { kind: Kind, path: &'a str },
}

/// `text` as plain runs and tokens, in order: the pieces put back together
/// (a token as its wire text) are `text` again, byte for byte.
#[must_use]
pub fn parse(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let (mut from, mut at) = (0, 0);
    while let Some(open) = text[at..].find('[').map(|offset| at + offset) {
        let Some((kind, path, len)) = token_at(&text[open..]) else {
            at = open + 1;
            continue;
        };
        if from < open {
            pieces.push(Piece::Text(&text[from..open]));
        }
        pieces.push(Piece::Token { kind, path });
        (from, at) = (open + len, open + len);
    }
    if from < text.len() {
        pieces.push(Piece::Text(&text[from..]));
    }
    pieces
}

/// The token `text` begins with: its kind, path and length in bytes.
fn token_at(text: &str) -> Option<(Kind, &str, usize)> {
    let rest = text.strip_prefix('[')?;
    let (kind, rest) = if let Some(number) = rest.strip_prefix("Image #") {
        let digits = number.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 6 || number.starts_with('0') {
            return None;
        }
        let n = number[..digits].parse().ok()?;
        (Kind::Image(n), number[digits..].strip_prefix(": \"")?)
    } else {
        (Kind::File, rest.strip_prefix("File: \"")?)
    };
    let end = rest.find('"')?;
    let path = &rest[..end];
    let after = rest[end + 1..].strip_prefix(']')?;
    fits(path).then(|| (kind, path, text.len() - after.len()))
}

/// What a token shows in a draft and in the history: `[Image #N]`, or
/// `[File: <name>]` with the path's last component made inert.
#[must_use]
pub fn label(kind: Kind, path: &str) -> String {
    match kind {
        Kind::Image(n) => format!("[Image #{n}]"),
        Kind::File => {
            let base = path.rsplit('/').next().filter(|name| !name.is_empty());
            let name: String = base
                .unwrap_or("file")
                .chars()
                .map(|ch| match ch {
                    ch if ch.is_control() => '\u{fffd}',
                    '[' | ']' => '_',
                    ch => ch,
                })
                .take(NAME_MAX)
                .collect();
            format!("[File: {name}]")
        }
    }
}

/// `text` with every token shown as its label: the history's form.
#[must_use]
pub fn collapse(text: &str) -> String {
    parse(text)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(run) => run.to_owned(),
            Piece::Token { kind, path } => label(kind, path),
        })
        .collect()
}

/// The paths a whole paste is made of, with the kind each becomes, when the
/// paste is nothing else: one or more absolute paths split at unquoted
/// whitespace (`\ ` and one level of `'` or `"` undone), each with an
/// extension from a short list. Images come back as `Image(0)`: the caller
/// numbers them. Never reads a file.
#[must_use]
pub fn classify_paste(raw: &[u8]) -> Option<Vec<(Kind, String)>> {
    if raw.len() > PASTE_MAX {
        return None;
    }
    let paths = split_paths(std::str::from_utf8(raw).ok()?.trim())?;
    if paths.is_empty() || paths.len() > PASTE_PATHS {
        return None;
    }
    paths
        .into_iter()
        .map(|path| {
            let name = path.rsplit('/').next()?;
            let (stem, extension) = name.rsplit_once('.')?;
            let extension = extension.to_lowercase();
            if stem.is_empty() {
                return None;
            }
            let kind = if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                Kind::Image(0)
            } else if FILE_EXTENSIONS.contains(&extension.as_str()) {
                Kind::File
            } else {
                return None;
            };
            fits(&path).then_some((kind, path))
        })
        .collect()
}

/// `text` split at unquoted, unescaped whitespace; `None` for an open quote,
/// a trailing backslash or a word no path can be. A word is cut at
/// [`PATH_MAX`] bytes, so a long paste is refused after a bounded read.
fn split_paths(text: &str) -> Option<Vec<String>> {
    let (mut paths, mut current, mut open) = (Vec::new(), String::new(), false);
    let mut quote = None;
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if current.len() > PATH_MAX {
            return None;
        }
        match (quote, ch) {
            (Some(mark), ch) if ch == mark => quote = None,
            (Some(_), ch) => current.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                open = true;
            }
            (None, '\\') => {
                current.push(chars.next()?);
                open = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if std::mem::take(&mut open) {
                    paths.push(std::mem::take(&mut current));
                }
            }
            (None, ch) => {
                current.push(ch);
                open = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if open {
        paths.push(current);
    }
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_exactly_its_grammar_and_everything_else_is_text() {
        let good = "a [Image #7: \"/p/a b.png\"][File: \"/p/x.pdf\"] z";
        let pieces = parse(good);
        assert_eq!(
            pieces,
            [
                Piece::Text("a "),
                Piece::Token {
                    kind: Kind::Image(7),
                    path: "/p/a b.png"
                },
                Piece::Token {
                    kind: Kind::File,
                    path: "/p/x.pdf"
                },
                Piece::Text(" z"),
            ]
        );
        for bad in [
            "[Image #0: \"/p/a.png\"]",
            "[Image #01: \"/p/a.png\"]",
            "[Image #1000000: \"/p/a.png\"]",
            "[Image #1: \"p/a.png\"]",
            "[Image #1: \"/p/a\u{7}.png\"]",
            "[Image #1: \"/p/a.png\"",
            "[Image #1]",
            "[File: /p/a.pdf]",
        ] {
            assert_eq!(parse(bad), [Piece::Text(bad)], "{bad}");
        }
    }

    #[test]
    fn wire_text_parses_back_to_the_token_that_made_it() {
        for (kind, path) in [
            (Kind::Image(1), "/a.png"),
            (Kind::Image(MAX_N), "/x y/z].png"),
            (Kind::File, "/a/b c.pdf"),
        ] {
            let text = wire(kind, path).unwrap();
            assert_eq!(parse(&text), [Piece::Token { kind, path }]);
        }
        assert!(wire(Kind::Image(0), "/a.png").is_none());
        assert!(wire(Kind::Image(MAX_N + 1), "/a.png").is_none());
        assert!(wire(Kind::File, "/a\"b.pdf").is_none());
        assert!(wire(Kind::File, "rel.pdf").is_none());
    }

    #[test]
    fn labels_are_inert_and_collapse_shows_only_them() {
        assert_eq!(label(Kind::Image(3), "/a.png"), "[Image #3]");
        assert_eq!(
            label(Kind::File, "/a/[x]\u{7}.pdf"),
            "[File: _x_\u{fffd}.pdf]"
        );
        assert_eq!(label(Kind::File, "/a/"), "[File: file]");
        let long = label(Kind::File, &format!("/{}", "n".repeat(90)));
        assert_eq!(long.len(), "[File: ]".len() + NAME_MAX);
        assert_eq!(
            collapse("see [Image #2: \"/t/i.png\"] and [File: \"/t/r.pdf\"]!"),
            "see [Image #2] and [File: r.pdf]!"
        );
        assert_eq!(collapse("typed [Image #2] stays"), "typed [Image #2] stays");
    }

    #[test]
    fn a_paste_is_paths_only_when_every_word_is_one() {
        let two = classify_paste(b"/t/a.PNG\n'/t/r one.pdf'\n").unwrap();
        assert_eq!(
            two,
            [
                (Kind::Image(0), "/t/a.PNG".to_owned()),
                (Kind::File, "/t/r one.pdf".to_owned())
            ]
        );
        let escaped = classify_paste(br"/t/r\ one.csv /t/b.jpeg").unwrap();
        assert_eq!(escaped[0].1, "/t/r one.csv");
        for none in [
            &b"explain /t/a.png please"[..],
            b"/t/not.allowed",
            b"/t/noext",
            b"/t/.png",
            b"rel/a.png",
            b"'/t/a.png",
            b"/t/a.png\\",
            b"",
            b"   \n",
            b"\xff/t/a.png",
            b"\"/t/q\"x.png\"",
        ] {
            assert!(classify_paste(none).is_none(), "{none:?}");
        }
    }
}
