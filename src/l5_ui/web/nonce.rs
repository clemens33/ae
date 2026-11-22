//! The draft id a page takes for each message it sends: the listener's own
//! epoch second it was issued, sixteen random bytes and an HMAC under the
//! listener key over both, as 112 lowercase hex characters. Pure: the caller
//! hands in the key, the clock reading and the random bytes.
//!
//! A nonce is accepted without a claim only while its tag verifies under the
//! current key and it is at most [`LIFE`] seconds old, so a nonce of an earlier
//! listener, whose key died with it, never carries fresh.

use ring::hmac;

use super::auth::{self, hex};

/// The seconds a nonce stays acceptable, inclusive: one day.
pub const LIFE: u64 = 86_400;
/// The characters of a well-formed nonce: (8 + 16 + 32) bytes as hex.
pub const LEN: usize = 112;
/// What the tag signs before the issue time and the random bytes.
const DOMAIN: &[u8] = b"ae-web nonce 1\0";

/// Why a nonce is not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Not exactly [`LEN`] lowercase hex characters.
    Malformed,
    /// Its tag does not verify under this key.
    Forged,
    /// Issued after `now` or more than [`LIFE`] seconds before it.
    Expired,
}

/// The nonce issued at listener second `issued_at` with `random`, tagged under
/// `key`: hex of the issue time big-endian, the random bytes and the tag.
#[must_use]
pub fn mint(key: &[u8; 32], issued_at: u64, random: [u8; 16]) -> String {
    let mut bytes = Vec::with_capacity(LEN / 2);
    bytes.extend_from_slice(&issued_at.to_be_bytes());
    bytes.extend_from_slice(&random);
    let tag = hmac::sign(&signer(key), &signed(&bytes));
    bytes.extend_from_slice(tag.as_ref());
    hex(&bytes)
}

/// The issue time of `text` when it is a nonce this key minted that is still
/// current at listener second `now`. The tag is checked before the age, so a
/// changed issue time reads as forged.
///
/// # Errors
///
/// [`Refused`], for the first test `text` fails.
pub fn verify(key: &[u8; 32], text: &str, now: u64) -> Result<u64, Refused> {
    if !auth::is_hex(text, LEN) {
        return Err(Refused::Malformed);
    }
    let bytes = unhex(text).ok_or(Refused::Malformed)?;
    let (body, tag) = bytes.split_at_checked(24).ok_or(Refused::Malformed)?;
    hmac::verify(&signer(key), &signed(body), tag).map_err(|_| Refused::Forged)?;
    let mut issued = [0_u8; 8];
    issued.copy_from_slice(body.get(..8).ok_or(Refused::Malformed)?);
    let issued_at = u64::from_be_bytes(issued);
    if issued_at <= now && now - issued_at <= LIFE {
        Ok(issued_at)
    } else {
        Err(Refused::Expired)
    }
}

/// The claim file name of a nonce: the SHA-256 hex of its exact text. Only a
/// text that passed the format check reaches it.
#[must_use]
pub fn claim_name(text: &str) -> String {
    crate::install::sha256_hex(text.as_bytes())
}

fn signer(key: &[u8; 32]) -> hmac::Key {
    hmac::Key::new(hmac::HMAC_SHA256, key)
}

fn signed(body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(DOMAIN.len() + body.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(body);
    message
}

/// The bytes of lowercase hex `text`, or `None` for any other character.
fn unhex(text: &str) -> Option<Vec<u8>> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => Some((digit(*high)? << 4) | digit(*low)?),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{LEN, Refused, mint, verify};

    #[test]
    fn a_minted_nonce_round_trips_and_its_age_is_inclusive() {
        let key = [7_u8; 32];
        let nonce = mint(&key, 10, [3; 16]);
        assert_eq!(nonce.len(), LEN);
        assert_eq!(verify(&key, &nonce, 10), Ok(10));
        assert_eq!(verify(&key, &nonce, 10 + super::LIFE), Ok(10));
        assert_eq!(
            verify(&key, &nonce, 11 + super::LIFE),
            Err(Refused::Expired)
        );
        assert_eq!(verify(&key, &nonce, 9), Err(Refused::Expired));
        assert_eq!(verify(&[8; 32], &nonce, 10), Err(Refused::Forged));
    }
}
