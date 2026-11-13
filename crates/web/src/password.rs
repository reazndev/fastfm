//! Passwords: argon2id hashing off the async runtime, and the rules for names and passwords.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;

use crate::error::WebError;

/// Shortest password accepted.
pub(crate) const MIN_PASSWORD: usize = 10;
/// Longest password accepted; hashing cost grows with length.
pub(crate) const MAX_PASSWORD: usize = 256;
/// Hashes that may run at once, so a burst of logins cannot take every core.
const PARALLEL_HASHES: usize = 4;

/// Hashes and checks passwords on blocking threads.
#[derive(Debug)]
pub(crate) struct Passwords {
    permits: Semaphore,
    /// Checked against when the name is unknown, so a miss costs as long as a wrong password.
    dummy: String,
}

impl Passwords {
    /// Prepares the hasher. Blocking: it hashes one random password.
    pub(crate) fn new() -> Result<Self, WebError> {
        let filler = lfm_api::methods::auth::random_hex(16)?;
        Ok(Self {
            permits: Semaphore::new(PARALLEL_HASHES),
            dummy: hash_blocking(&filler)?,
        })
    }

    /// The PHC string of `password`.
    pub(crate) async fn hash(&self, password: &str) -> Result<String, WebError> {
        let _permit = self.permits.acquire().await.map_err(internal)?;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || hash_blocking(&password))
            .await
            .map_err(internal)?
    }

    /// Whether `password` matches `hash`. With no hash, checks the dummy and returns false.
    pub(crate) async fn verify(&self, hash: Option<&str>, password: &str) -> bool {
        let Ok(_permit) = self.permits.acquire().await else {
            return false;
        };
        let known = hash.is_some();
        let hash = hash.unwrap_or(&self.dummy).to_owned();
        let password = password.to_owned();
        let matched = tokio::task::spawn_blocking(move || verify_blocking(&hash, &password))
            .await
            .unwrap_or(false);
        known && matched
    }
}

/// Usernames: 2 to 32 ASCII letters, digits, `_` or `-`, starting with a letter.
pub(crate) fn check_username(name: &str) -> Result<(), WebError> {
    let starts = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let chars = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if starts && chars && (2..=32).contains(&name.len()) {
        Ok(())
    } else {
        Err(WebError::invalid(
            "invalid_username",
            "Use 2 to 32 letters, digits, _ or -, starting with a letter.",
        ))
    }
}

/// Passwords: 10 to 256 characters.
pub(crate) fn check_password(password: &str) -> Result<(), WebError> {
    if (MIN_PASSWORD..=MAX_PASSWORD).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(WebError::invalid(
            "weak_password",
            "Use a password of 10 to 256 characters.",
        ))
    }
}

/// OWASP's argon2id baseline: 19 MiB, two passes, one lane.
fn hasher() -> Result<Argon2<'static>, WebError> {
    let params = Params::new(19_456, 2, 1, None).map_err(internal)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn hash_blocking(password: &str) -> Result<String, WebError> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(internal)?;
    let salt = SaltString::encode_b64(&salt).map_err(internal)?;
    Ok(hasher()?
        .hash_password(password.as_bytes(), &salt)
        .map_err(internal)?
        .to_string())
}

fn verify_blocking(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    hasher().is_ok_and(|h| h.verify_password(password.as_bytes(), &parsed).is_ok())
}

fn internal(error: impl std::fmt::Display) -> WebError {
    WebError::internal(&error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_verify_and_differ() {
        let passwords = Passwords::new().unwrap();
        let hash = passwords.hash("correct horse").await.unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(passwords.verify(Some(&hash), "correct horse").await);
        assert!(!passwords.verify(Some(&hash), "wrong horse!").await);
        assert!(!passwords.verify(None, "correct horse").await);
        assert_ne!(hash, passwords.hash("correct horse").await.unwrap());
    }

    #[test]
    fn rules() {
        assert!(check_username("alice_1").is_ok());
        assert!(check_username("a").is_err());
        assert!(check_username("1alice").is_err());
        assert!(check_username("al ice").is_err());
        assert!(check_password("short").is_err());
        assert!(check_password("long enough").is_ok());
    }
}
