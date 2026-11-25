//! Who may read and write through `ae web`: listener-lifetime credentials, held
//! in memory only. No I/O: the caller hands in the random bytes and the clock.
//!
//! The listener key `L` signs every secret it hands out, and only the HMAC tag
//! is kept, so a lookup compares tags in constant time (`ring::hmac::verify`).
//! A one-time code is good for one exchange within [`CODE_LIFE`]; the session
//! secret `S` it buys lives until [`IDLE`] passes without use, at most
//! [`SESSIONS_MAX`] at once.

use std::time::{Duration, Instant};

use ring::hmac;

/// How long a printed code may be exchanged.
pub const CODE_LIFE: Duration = Duration::from_mins(10);
/// How long a session secret lives without a request.
pub const IDLE: Duration = Duration::from_hours(12);
/// The most session secrets alive at once.
pub const SESSIONS_MAX: usize = 8;
/// The most unexchanged codes kept at once; the oldest goes first.
pub const CODES_MAX: usize = 8;

/// Why a code buys no session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Not 32 lowercase hex digits.
    Malformed,
    /// Not a live code of this listener: unknown, used or expired.
    Unknown,
}

/// One kept secret: its tag under `L`, the instant it was minted or last used,
/// and whether it is a code a refused request minted (a fresh link) rather
/// than the code printed at start. Only the tag is kept, never the secret.
struct Kept {
    tag: Vec<u8>,
    at: Instant,
    fresh: bool,
}

/// The credentials of one listener.
pub struct Auth {
    key: hmac::Key,
    codes: Vec<Kept>,
    sessions: Vec<Kept>,
}

impl Auth {
    /// The credentials of a listener whose secret is `listener_key`.
    #[must_use]
    pub fn new(listener_key: [u8; 32]) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, &listener_key),
            codes: Vec::new(),
            sessions: Vec::new(),
        }
    }

    /// A one-time code made of `random`, good until [`CODE_LIFE`] after `now`:
    /// the code printed at start. It never holds a fresh code back.
    pub fn mint_code(&mut self, random: [u8; 16], now: Instant) -> String {
        self.push_code(random, now, false)
    }

    /// A fresh one-time code made of `random` for a page that has no session,
    /// good until [`CODE_LIFE`] after `now`, or `None` while an earlier fresh
    /// code is unredeemed and unexpired. A client that keeps asking then gets
    /// one link and cannot fill the kept set to push the owner's codes out.
    /// The check and the mint are one step under the caller's lock.
    pub fn mint_fresh_code(&mut self, random: [u8; 16], now: Instant) -> Option<String> {
        self.codes.retain(|kept| live(kept, CODE_LIFE, now));
        if self.codes.iter().any(|kept| kept.fresh) {
            return None;
        }
        Some(self.push_code(random, now, true))
    }

    fn push_code(&mut self, random: [u8; 16], now: Instant, fresh: bool) -> String {
        let code = hex(&random);
        self.codes.retain(|kept| live(kept, CODE_LIFE, now));
        if self.codes.len() == CODES_MAX {
            self.codes.remove(0);
        }
        let mut kept = self.keep(&code, now);
        kept.fresh = fresh;
        self.codes.push(kept);
        code
    }

    /// Exchange `code` for a fresh session secret made of `random`. The code is
    /// spent; a ninth session drops the one used least recently.
    ///
    /// # Errors
    ///
    /// [`Refused`] when `code` is malformed or no live code of this listener.
    pub fn redeem(
        &mut self,
        code: &str,
        random: [u8; 32],
        now: Instant,
    ) -> Result<String, Refused> {
        if !is_hex(code, 32) {
            return Err(Refused::Malformed);
        }
        self.codes.retain(|kept| live(kept, CODE_LIFE, now));
        let at = self
            .codes
            .iter()
            .position(|kept| hmac::verify(&self.key, code.as_bytes(), &kept.tag).is_ok())
            .ok_or(Refused::Unknown)?;
        self.codes.remove(at);
        self.sessions.retain(|kept| live(kept, IDLE, now));
        if self.sessions.len() == SESSIONS_MAX
            && let Some(oldest) =
                (0..self.sessions.len()).min_by_key(|at| self.sessions.get(*at).map(|kept| kept.at))
        {
            self.sessions.remove(oldest);
        }
        let secret = hex(&random);
        self.sessions.push(self.keep(&secret, now));
        Ok(secret)
    }

    /// Whether `bearer` is a live session secret of this listener; a hit counts
    /// as its use at `now`.
    pub fn check(&mut self, bearer: &str, now: Instant) -> bool {
        self.sessions.retain(|kept| live(kept, IDLE, now));
        if !is_hex(bearer, 64) {
            return false;
        }
        let key = &self.key;
        match self
            .sessions
            .iter_mut()
            .find(|kept| hmac::verify(key, bearer.as_bytes(), &kept.tag).is_ok())
        {
            Some(kept) => {
                kept.at = now;
                true
            }
            None => false,
        }
    }

    fn keep(&self, secret: &str, now: Instant) -> Kept {
        Kept {
            tag: hmac::sign(&self.key, secret.as_bytes()).as_ref().to_vec(),
            at: now,
            fresh: false,
        }
    }
}

/// Whether `kept` is still within `life` of `now`.
fn live(kept: &Kept, life: Duration, now: Instant) -> bool {
    now.checked_duration_since(kept.at)
        .is_none_or(|age| age < life)
}

/// Whether `text` is exactly `len` lowercase hex digits.
pub(crate) fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Lowercase hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        for nibble in [byte >> 4, byte & 0x0f] {
            out.push(char::from(
                DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0'),
            ));
        }
    }
    out
}

/// Whether `host`, a request's `Host` value, names this listener on `port`.
#[must_use]
pub fn host_ok(host: &str, port: u16) -> bool {
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
    .iter()
    .any(|allowed| allowed == host)
}

/// Whether a POST's `origin` is the origin of the `host` it was sent to.
#[must_use]
pub fn origin_ok(origin: Option<&str>, host: &str) -> bool {
    origin.is_some_and(|origin| origin == format!("http://{host}"))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{CODE_LIFE, IDLE, Kept, live};

    #[test]
    fn a_kept_secret_is_spent_once_its_age_reaches_its_lifetime() {
        // Contract pin: "good for 10 minutes" and "12 idle hours end it" read
        // as an age under the lifetime; web_auth_spec checks 599 s and 601 s.
        let at = Instant::now();
        let kept = Kept {
            tag: Vec::new(),
            at,
            fresh: false,
        };
        for life in [CODE_LIFE, IDLE] {
            let Some(just_under) = life.checked_sub(Duration::from_millis(1)) else {
                panic!("a lifetime under one millisecond");
            };
            assert!(live(&kept, life, at + just_under), "{life:?}");
            assert!(!live(&kept, life, at + life), "{life:?}");
        }
    }
}
