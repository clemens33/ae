//! The writer-lease holder record: one line naming the holding app's place.

use crate::session_launch::name::is_session_name;

/// The most bytes a record may be: `1 ` + a 128-byte session + ` ` + a 5-digit
/// window index + LF is 137, so any record the grammar allows fits.
pub const HOLDER_CAP: u64 = 160;

/// The record an `ae app` leaves in the writer lease it holds, so a refused
/// writer can say where the holder sits: `1 <session> <window index>` and LF.
/// Hostile persisted input, parsed strictly and read bounded; both fields are
/// grammar-bound, so safe to display. The holder's own tmux observation when
/// it took the lease, not a liveness proof. `None` when `session` or `window`
/// break the grammar.
#[must_use]
pub fn encode(session: &str, window: &str) -> Option<Vec<u8>> {
    (is_session_name(session) && is_window_index(window))
        .then(|| format!("1 {session} {window}\n").into_bytes())
}

/// The session and window index a record names; any other bytes are `None`.
#[must_use]
pub fn parse(body: &[u8]) -> Option<(String, String)> {
    if body.len() as u64 > HOLDER_CAP {
        return None;
    }
    let line = std::str::from_utf8(body).ok()?.strip_suffix('\n')?;
    let (session, window) = line.strip_prefix("1 ")?.split_once(' ')?;
    (is_session_name(session) && is_window_index(window))
        .then(|| (session.to_owned(), window.to_owned()))
}

/// One to five ASCII digits.
fn is_window_index(window: &str) -> bool {
    (1..=5).contains(&window.len()) && window.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{HOLDER_CAP, encode, parse};
    #[test]
    fn roundtrip_and_cap_arithmetic() {
        assert_eq!(HOLDER_CAP, 160);
        let rec = encode("alpha", "2");
        assert_eq!(rec, Some(b"1 alpha 2\n".to_vec()), "encodes");
        let held = parse(b"1 alpha 2\n");
        assert_eq!(held, Some(("alpha".into(), "2".into())), "parses");
        let long = "a".repeat(128);
        let rec = encode(&long, "12345").expect("128-char session encodes");
        assert_eq!(rec.len(), 137, "2+128+1+5+1 < 160");
        assert_eq!(parse(&rec).map(|h| h.0), Some(long), "roundtrips");
        let zero = encode("alpha", "0");
        assert_eq!(zero, Some(b"1 alpha 0\n".to_vec()), "length rule");
    }
    #[test]
    fn strict_grammar_rejects_everything_else() {
        let bad = [
            "",
            "1 alpha 2",
            "1 alpha 2\n\n",
            "2 alpha 2\n",
            "1 alpha 2 3\n",
            "1 bad name 2\n",
            "1 a 123456\n",
            "1 a 2x\n",
            "1 a \n",
            "1 a 2\r\n",
        ];
        for b in &bad {
            assert_eq!(parse(b.as_bytes()), None, "{b:?}");
        }
        let overlong = format!("1 {} 2\n", "a".repeat(129));
        assert_eq!(parse(overlong.as_bytes()), None, "129-char");
        assert_eq!(parse(&[0xff, 0xfe]), None, "utf-8");
        assert_eq!(parse(&[b'x'; 161]), None, "over cap");
        let pairs = [
            ("bad name", "2"),
            ("a", "123456"),
            ("a", ""),
            ("a", "2 "),
            ("", "2"),
        ];
        for (s, i) in pairs {
            assert_eq!(encode(s, i), None, "{s:?}/{i:?}");
        }
    }
}
