//! Reversible, authenticated protection for invite codes.
//!
//! Invite codes are bearer capabilities, so the database keeps a SHA-256
//! [`code_hash`](crate::db::invites::Invite::code_hash) for lookup and never
//! needs the plaintext there. That hash is one way, though, so it cannot power
//! a "show the code again" view.
//!
//! `code_sealed` therefore holds a second, encrypted copy. The key is derived
//! from `URLET_SECRET`, and the copy is authenticated with ChaCha20-Poly1305:
//!
//!   * a database dump or backup on its own does not reveal the codes;
//!   * a corrupted or truncated value is rejected instead of being displayed;
//!   * the row's `code_hash` is authenticated data, so a sealed value copied to
//!     another invite row cannot be decrypted there;
//!   * changing the secret deliberately makes old copies unreadable. Those
//!     rows still redeem normally when someone knows the code; the UI falls
//!     back to the stored prefix and asks the issuer to create a new code.
//!
//! The plaintext never leaves this module except as the return value of
//! [`open`], and the sealed value is not logged.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Version tag of the stored format. A future format can be added without
/// guessing how to parse existing rows.
const VERSION: &str = "v2.";
/// Binds the ciphertext to this use and to one invite row. `seal` and `open`
/// both append the row's `code_hash`, so moving a sealed value to another row
/// makes authentication fail.
const AAD_PREFIX: &[u8] = b"urlet.invite-code.v2:";
/// ChaCha20-Poly1305 nonce length.
const NONCE_LEN: usize = 12;
/// Poly1305 tag length.
const TAG_LEN: usize = 16;

fn aad(code_hash: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(AAD_PREFIX.len() + code_hash.len());
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(code_hash.as_bytes());
    aad
}

fn cipher(secret: &[u8]) -> ChaCha20Poly1305 {
    // Domain separation keeps this key from being reused by another feature
    // that also derives something from the server secret.
    let mut hasher = Sha256::new();
    hasher.update(b"urlet invite code key v2");
    hasher.update(secret);
    let key = hasher.finalize();
    ChaCha20Poly1305::new_from_slice(&key).expect("SHA-256 always yields a 32-byte key")
}

/// Encrypt `code` for the invite row identified by `code_hash`. A fresh random
/// nonce is used every time, so the same code sealed twice produces different
/// values.
pub fn seal(secret: &[u8], code_hash: &str, code: &str) -> String {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let aad = aad(code_hash);
    let ciphertext = cipher(secret)
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: code.as_bytes(),
                aad: &aad,
            },
        )
        .expect("an invite code fits in one AEAD message");

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    format!("{VERSION}{}", URL_SAFE_NO_PAD.encode(blob))
}

/// Decrypt a value produced by [`seal`] for the row identified by `code_hash`.
/// Returns `None` for a missing/unknown version, malformed base64, a truncated
/// blob, a wrong secret, a tampered value or another row's ciphertext.
pub fn open(secret: &[u8], code_hash: &str, sealed: &str) -> Option<String> {
    let encoded = sealed.strip_prefix(VERSION)?;
    let blob = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    if blob.len() < NONCE_LEN + TAG_LEN {
        return None;
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let aad = aad(code_hash);
    let plaintext = cipher(secret)
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-secret-test-secret-test-secret";
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_sealed_code_round_trips() {
        let sealed = seal(SECRET, HASH, "K7M2QPX4RT");
        assert!(sealed.starts_with(VERSION));
        assert_eq!(open(SECRET, HASH, &sealed).as_deref(), Some("K7M2QPX4RT"));
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let first = seal(SECRET, HASH, "K7M2QPX4RT");
        let second = seal(SECRET, HASH, "K7M2QPX4RT");
        assert_ne!(first, second);
        assert_eq!(open(SECRET, HASH, &first).as_deref(), Some("K7M2QPX4RT"));
        assert_eq!(open(SECRET, HASH, &second).as_deref(), Some("K7M2QPX4RT"));
    }

    #[test]
    fn a_different_secret_cannot_open_the_copy() {
        let sealed = seal(SECRET, HASH, "K7M2QPX4RT");
        assert_eq!(open(b"another-secret-another-secret", HASH, &sealed), None);
    }

    #[test]
    fn a_sealed_value_is_bound_to_its_row() {
        let sealed = seal(SECRET, HASH, "K7M2QPX4RT");
        assert_eq!(open(SECRET, OTHER_HASH, &sealed), None);
    }

    #[test]
    fn tampering_is_rejected() {
        let sealed = seal(SECRET, HASH, "K7M2QPX4RT");
        let (prefix, encoded) = sealed.split_at(VERSION.len());
        let mut blob = URL_SAFE_NO_PAD.decode(encoded).expect("blob");
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        let tampered = format!("{prefix}{}", URL_SAFE_NO_PAD.encode(blob));
        assert_eq!(open(SECRET, HASH, &tampered), None);
    }

    #[test]
    fn malformed_values_are_rejected_without_panicking() {
        assert_eq!(open(SECRET, HASH, ""), None);
        assert_eq!(open(SECRET, HASH, "not-a-sealed-code"), None);
        assert_eq!(open(SECRET, HASH, "v1.AAAA"), None, "unknown version");
        assert_eq!(
            open(SECRET, HASH, "v2.AAAA"),
            None,
            "shorter than nonce + tag"
        );
        assert_eq!(open(SECRET, HASH, "v2.!!!!"), None, "invalid base64");
    }
}
