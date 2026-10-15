//! Password hashing (Argon2id) and password policy.

use std::sync::OnceLock;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::{AppError, Result};

/// Minimum accepted password length.
pub const MIN_LEN: usize = 8;
/// Maximum accepted password length (bounds Argon2 work per request).
pub const MAX_LEN: usize = 128;

/// OWASP recommended Argon2id parameters for interactive logins.
const MEMORY_KIB: u32 = 19_456;
const ITERATIONS: u32 = 2;
const PARALLELISM: u32 = 1;

fn hasher() -> Argon2<'static> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, None)
        .expect("hard coded Argon2 parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Validate the password policy. Returns a user facing error.
pub fn validate(password: &str) -> Result<()> {
    if password.chars().count() < MIN_LEN {
        return Err(AppError::validation(
            "password",
            format!("password must be at least {MIN_LEN} characters"),
        ));
    }
    if password.chars().count() > MAX_LEN {
        return Err(AppError::validation(
            "password",
            format!("password must be at most {MAX_LEN} characters"),
        ));
    }
    if password.trim().is_empty() {
        return Err(AppError::validation("password", "password must not be blank"));
    }
    Ok(())
}

/// Hash a password into a PHC string.
pub fn hash(password: &str) -> Result<String> {
    validate(password)?;
    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::internal(format!("password hashing failed: {err}")))
}

/// Verify a password against a stored PHC string. Never panics, never errors.
pub fn verify(password: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// Burn the same amount of time as a real verification, so that the response
/// time of "unknown account" and "wrong password" are indistinguishable.
pub fn dummy_verify(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let hash = DUMMY.get_or_init(|| {
        hash("uwurls-dummy-password-for-timing-equalisation").unwrap_or_default()
    });
    let _ = verify(password, hash);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies() {
        let hash = hash("correct horse battery").expect("hash");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify("correct horse battery", &hash));
        assert!(!verify("wrong password", &hash));
    }

    #[test]
    fn rejects_short_passwords() {
        assert!(validate("short").is_err());
        assert!(validate("longenough").is_ok());
        assert!(validate(&"a".repeat(MAX_LEN + 1)).is_err());
    }

    #[test]
    fn verification_never_panics_on_garbage() {
        assert!(!verify("whatever", "not-a-phc-string"));
        assert!(!verify("whatever", ""));
        dummy_verify("whatever");
    }

    #[test]
    fn hashes_are_salted() {
        let a = hash("same-password-twice").expect("hash");
        let b = hash("same-password-twice").expect("hash");
        assert_ne!(a, b);
    }
}
