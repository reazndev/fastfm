//! Encryption of Last.fm secrets at rest, with a key file kept in the data directory.
//!
//! XChaCha20-Poly1305: its 24-byte nonces can be drawn at random for every message without a
//! counter. A sealed value is a version byte, the nonce, and the ciphertext with its tag. The
//! caller's context string is bound in as associated data, so a value copied to another account
//! or column fails to open.

use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

/// The key file's name in the data directory.
pub const KEY_FILE: &str = "secrets.key";

const VERSION: u8 = 1;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// What can go wrong with the key file or a sealed value.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// The key file could not be read or created.
    #[error("key file {path}: {source}")]
    KeyFile {
        /// The file.
        path: PathBuf,
        /// What failed.
        source: std::io::Error,
    },
    /// The key file does not hold exactly 32 bytes.
    #[error("key file {0} is not a 32-byte key")]
    BadKeyFile(PathBuf),
    /// The OS random source failed.
    #[error("the OS random source failed")]
    Random,
    /// A sealed value is damaged, belongs elsewhere, or was sealed with another key.
    #[error("a stored secret cannot be decrypted")]
    Corrupt,
}

/// Seals and opens secrets with the instance key.
pub struct Vault {
    cipher: XChaCha20Poly1305,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Vault(REDACTED)")
    }
}

impl Vault {
    /// Reads `secrets.key` from `dir`, creating it with a new random key and mode 0600 on first
    /// start. Blocking; call it before serving or from a blocking task.
    pub fn open_or_create(dir: &Path) -> Result<Self, VaultError> {
        let path = dir.join(KEY_FILE);
        let failed = |source| VaultError::KeyFile {
            path: path.clone(),
            source,
        };
        match std::fs::File::open(&path) {
            Ok(mut file) => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).map_err(failed)?;
                let key: [u8; KEY_LEN] = bytes
                    .try_into()
                    .map_err(|_| VaultError::BadKeyFile(path.clone()))?;
                Ok(Self::from_key(key))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let key = random_bytes::<KEY_LEN>()?;
                create_private(&path)
                    .and_then(|mut file| file.write_all(&key).and_then(|()| file.sync_all()))
                    .map_err(failed)?;
                tracing::info!(path = %path.display(), "created the secrets key file");
                Ok(Self::from_key(key))
            }
            Err(error) => Err(failed(error)),
        }
    }

    /// A vault with this key.
    #[must_use]
    pub fn from_key(key: [u8; KEY_LEN]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(&key.into()),
        }
    }

    /// Encrypts `plain`, bound to `context` (such as `account:3:lastfm_sk`).
    pub fn seal(&self, context: &str, plain: &str) -> Result<Vec<u8>, VaultError> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let payload = Payload {
            msg: plain.as_bytes(),
            aad: context.as_bytes(),
        };
        let sealed = self
            .cipher
            .encrypt(&XNonce::from(nonce), payload)
            .map_err(|_| VaultError::Corrupt)?;
        Ok([&[VERSION][..], &nonce, &sealed].concat())
    }

    /// Decrypts a value sealed with the same `context`.
    pub fn open(&self, context: &str, sealed: &[u8]) -> Result<String, VaultError> {
        let (&version, rest) = sealed.split_first().ok_or(VaultError::Corrupt)?;
        if version != VERSION || rest.len() < NONCE_LEN {
            return Err(VaultError::Corrupt);
        }
        let (nonce, body) = rest.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| VaultError::Corrupt)?;
        let payload = Payload {
            msg: body,
            aad: context.as_bytes(),
        };
        let plain = self
            .cipher
            .decrypt(&XNonce::from(nonce), payload)
            .map_err(|_| VaultError::Corrupt)?;
        String::from_utf8(plain).map_err(|_| VaultError::Corrupt)
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| VaultError::Random)?;
    Ok(bytes)
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_context_binding() {
        let vault = Vault::from_key([7; KEY_LEN]);
        let sealed = vault.seal("account:1:sk", "s3cret").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"s3cret"));
        assert_eq!(vault.open("account:1:sk", &sealed).unwrap(), "s3cret");
        assert!(vault.open("account:2:sk", &sealed).is_err());
        assert_ne!(sealed, vault.seal("account:1:sk", "s3cret").unwrap());
        let other = Vault::from_key([8; KEY_LEN]);
        assert!(other.open("account:1:sk", &sealed).is_err());
        assert!(vault.open("account:1:sk", &sealed[..10]).is_err());
        assert_eq!(format!("{vault:?}"), "Vault(REDACTED)");
    }

    #[test]
    fn key_file_is_created_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = Vault::open_or_create(dir.path()).unwrap();
        let sealed = first.seal("c", "value").unwrap();
        let second = Vault::open_or_create(dir.path()).unwrap();
        assert_eq!(second.open("c", &sealed).unwrap(), "value");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(dir.path().join(KEY_FILE), b"short").unwrap();
        assert!(matches!(
            Vault::open_or_create(dir.path()),
            Err(VaultError::BadKeyFile(_))
        ));
    }
}
