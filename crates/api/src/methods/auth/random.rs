//! Random keys and tokens from the OS random source.

/// The OS random source failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the OS random source failed")]
pub struct RandomUnavailable;

/// The characters of Last.fm's tokens, as seen in `fixtures/auth.getToken`.
const TOKEN_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_";

/// `bytes` random bytes as lowercase hex, such as 16 bytes for a 32-character API key.
pub fn random_hex(bytes: usize) -> Result<String, RandomUnavailable> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|_| RandomUnavailable)?;
    Ok(buf
        .iter()
        .fold(String::with_capacity(bytes * 2), |mut out, b| {
            out.push(hex_digit(b >> 4));
            out.push(hex_digit(b & 0x0f));
            out
        }))
}

/// A 32-character token in Last.fm's alphabet, without modulo bias.
pub fn random_token() -> Result<String, RandomUnavailable> {
    let symbols = TOKEN_ALPHABET.len();
    // The largest multiple of the alphabet size below 256; bytes from there on are redrawn.
    let limit = 256 - 256 % symbols;
    let mut token = String::with_capacity(32);
    let mut buf = [0u8; 64];
    while token.len() < 32 {
        getrandom::fill(&mut buf).map_err(|_| RandomUnavailable)?;
        let picked = buf
            .iter()
            .map(|b| usize::from(*b))
            .filter(|b| *b < limit)
            .filter_map(|b| TOKEN_ALPHABET.get(b % symbols))
            .map(|b| char::from(*b));
        token.extend(picked.take(32 - token.len()));
    }
    Ok(token)
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_has_the_asked_length_and_alphabet() {
        let key = random_hex(16).unwrap();
        assert_eq!(key.len(), 32);
        assert!(
            key.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(key, random_hex(16).unwrap());
    }

    #[test]
    fn tokens_look_like_last_fm_tokens() {
        let token = random_token().unwrap();
        assert_eq!(token.len(), 32);
        assert!(token.bytes().all(|b| TOKEN_ALPHABET.contains(&b)));
    }
}
