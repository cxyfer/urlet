//! Server secret handling and the HMAC-derived identifiers built on top of it.
//!
//! Nothing that identifies a visitor is ever stored in the clear: IP addresses
//! are HMACed with the server secret before they reach the database.

use std::path::Path;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::config::ConfigError;

const SECRET_FILE: &str = "secret.key";

/// Placeholder printed instead of a secret in `Debug` output.
pub const REDACTED: &str = "[redacted]";

/// Load the HMAC secret from an explicit value or from the data directory,
/// generating (and persisting) a fresh one when neither is available.
pub fn load_or_create(data_dir: &Path, explicit: Option<&str>) -> Result<Vec<u8>, ConfigError> {
    if let Some(value) = explicit {
        let value = value.trim();
        if value.len() < 16 {
            return Err(ConfigError::WeakSecret);
        }
        return Ok(value.as_bytes().to_vec());
    }

    let path = data_dir.join(SECRET_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if trimmed.len() >= 16 {
            // An older version may have created the file world readable.
            tighten_permissions(&path);
            return Ok(trimmed.as_bytes().to_vec());
        }
    }

    let generated = generate_secret();
    write_private(&path, &generated).map_err(|source| ConfigError::Io {
        path: path.clone(),
        source,
    })?;
    tracing::info!(path = %path.display(), "generated a new server secret");
    Ok(generated.as_bytes().to_vec())
}

fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

/// Create the file with mode 0600 from the start: a chmod afterwards would
/// leave a window where the HMAC key is world readable.
#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

/// Reduce the permissions of an existing secret file to 0600 when needed.
#[cfg(unix)]
fn tighten_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.permissions().mode() & 0o077 != 0 {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        tracing::warn!(path = %path.display(), "tightened permissions on the existing server secret");
    }
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &Path) {}

/// Hex encoded SHA-256 of `input`. Used for session/API token storage.
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    to_hex(&digest)
}

/// Hex encoded HMAC-SHA256 of `input` under the server secret.
pub fn hmac_hex(secret: &[u8], input: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(input.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

/// Constant-time comparison of two hex digests.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// URL-safe random token with `bytes` of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    B64.encode(&buf)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string()
}

/// Short, unambiguous random code for invite codes.
pub fn random_code(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHABET[(rng.next_u32() as usize) % ALPHABET.len()] as char)
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn generated_secret_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("uwurls-secret-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        load_or_create(&dir, None).expect("generated");
        let mode = std::fs::metadata(dir.join(SECRET_FILE))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "secret must be 0600, got {mode:o}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_and_reloads_secret() {
        let dir = std::env::temp_dir().join(format!("uwurls-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let first = load_or_create(&dir, None).expect("generated");
        let second = load_or_create(&dir, None).expect("reloaded");
        assert_eq!(first, second);
        assert!(first.len() >= 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_weak_explicit_secret() {
        let dir = std::env::temp_dir();
        assert!(matches!(
            load_or_create(&dir, Some("short")),
            Err(ConfigError::WeakSecret)
        ));
    }

    #[test]
    fn hmac_is_stable_and_keyed() {
        let a = hmac_hex(b"secret-one", "203.0.113.7");
        let b = hmac_hex(b"secret-one", "203.0.113.7");
        let c = hmac_hex(b"secret-two", "203.0.113.7");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn tokens_are_unique_and_url_safe() {
        let a = random_token(32);
        let b = random_token(32);
        assert_ne!(a, b);
        assert!(!a.contains('+') && !a.contains('/') && !a.contains('='));
        assert_eq!(random_code(8).len(), 8);
    }
}
