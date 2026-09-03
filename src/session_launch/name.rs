//! Session names: the grammar, and the name a nameless launch derives.
//!
//! Ported from `ae`'s `_validate_session_name` (ae:10020) and
//! `default_session_name` (ae:9946-ish) — the two halves of one rule. The
//! grammar is an ALLOWLIST because a session name becomes a tmux session, a
//! directory under `~/.ae/sessions`, part of `.lifecycle.<name>.lock`, a
//! neighbour in tmux format strings, and the target of the launch rollback's
//! recursive delete.
//!
//! The derivation hashes the RAW working directory with MD5 — before any
//! sanitising, deliberately: the body is lossy by construction (`A ~"B` and
//! `A#==B` both reduce to `A-B`), so the hash is the only thing that keeps two
//! such directories apart. MD5 is implemented here rather than shelled out to,
//! because `md5`/`md5sum` differ across the two platforms ae runs on and a
//! derived name must be the same on both. It is a NAMING digest, never a
//! security one.

/// The session-name grammar, echoed verbatim in the refusal.
pub(crate) const SESSION_NAME_GRAMMAR: &str = "^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$";

/// Whether `name` is a legal session name.
pub(crate) fn is_session_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if name.len() > 128 {
        return false;
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The name a launch with no name derives from `cwd`, with `home` stripped from
/// its head — the frozen `default_session_name`, whose hash length defaults to
/// 6 and widens to 12 in the collision advice.
pub(crate) fn default_session_name(cwd: &str, home: &str, hash_len: usize) -> String {
    let digest = md5_hex(cwd.as_bytes());
    let hash: String = digest.chars().take(hash_len).collect();
    let mut body = cwd.to_owned();
    if !home.is_empty() {
        let prefix = format!("{}/", home.trim_end_matches('/'));
        if let Some(rest) = body.strip_prefix(&prefix) {
            body = rest.to_owned();
        }
    }
    let mut out = String::with_capacity(body.len());
    for ch in body.chars() {
        out.push(match ch {
            '/' | '.' => {
                if ch == '/' {
                    '-'
                } else {
                    '_'
                }
            }
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            _ => '-',
        });
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let mut body = out.trim_matches('-').to_owned();
    // "ae-" + body + "-" + hash must fit the 128-character bound.
    let max_body = 128usize.saturating_sub(4 + hash.len());
    if body.len() > max_body {
        body.truncate(max_body);
    }
    let body = body.trim_end_matches('-');
    if body.is_empty() {
        format!("ae-{hash}")
    } else {
        format!("ae-{body}-{hash}")
    }
}

/// Whether `candidate` is a DIRECT child of `parent`, by pure string structure
/// — the frozen `_path_is_direct_child`, the belt to entry validation's braces
/// before anything is deleted.
pub(crate) fn is_direct_child(parent: &str, candidate: &str) -> bool {
    if parent.is_empty() || candidate.is_empty() {
        return false;
    }
    let Some((head, base)) = candidate.rsplit_once('/') else {
        return false;
    };
    head == parent && !base.is_empty() && base != "." && base != ".."
}

// ---- MD5 ------------------------------------------------------------------

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// `floor(abs(sin(i + 1)) * 2^32)`, the constants RFC 1321 tabulates.
const K: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The lowercase hex MD5 of `input` — RFC 1321, little-endian throughout.
#[allow(
    clippy::many_single_char_names,
    reason = "a, b, c, d and i are RFC 1321's own names for the working state — renaming them makes the port unreadable against the specification"
)]
pub(crate) fn md5_hex(input: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in message.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            m[index] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut out = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            // Infallible into a String; the result is consumed to satisfy
            // `-D warnings` without an unwrap.
            let _ = write!(out, "{byte:02x}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_published_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        // 64 bytes: the padding path that needs a whole extra block.
        assert_eq!(md5_hex(&[b'a'; 64]), "014842d480b571495a4a0363793f7367");
    }

    #[test]
    fn the_derived_name_is_the_frozen_shape() {
        let name = default_session_name("/home/me/projects/my.app", "/home/me", 6);
        assert!(name.starts_with("ae-projects-my_app-"), "{name}");
        assert_eq!(name.len(), "ae-projects-my_app-".len() + 6);
        assert!(is_session_name(&name));
    }

    #[test]
    fn two_directories_that_reduce_alike_still_differ() {
        let one = default_session_name("/w/A ~\"B", "", 6);
        let other = default_session_name("/w/A#==B", "", 6);
        assert_ne!(one, other, "the hash is what keeps a lossy body apart");
    }

    #[test]
    fn a_bodyless_directory_is_named_by_its_hash_alone() {
        assert_eq!(
            default_session_name("/", "", 6),
            format!("ae-{}", &md5_hex(b"/")[..6])
        );
    }

    #[test]
    fn the_grammar_refuses_what_it_says_it_refuses() {
        assert!(is_session_name("a"));
        assert!(is_session_name("ae-x_1-9"));
        assert!(!is_session_name(""));
        assert!(!is_session_name("-lead"));
        assert!(!is_session_name(".dotproject"));
        assert!(!is_session_name("a/b"));
        assert!(!is_session_name(&"a".repeat(129)));
        assert!(is_session_name(&"a".repeat(128)));
    }

    #[test]
    fn a_direct_child_is_one_segment_below_its_parent() {
        assert!(is_direct_child("/s", "/s/name"));
        assert!(!is_direct_child("/s", "/s/a/b"));
        assert!(!is_direct_child("/s", "/s/.."));
        assert!(!is_direct_child("/s", "/other/name"));
    }
}
