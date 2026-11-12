//! `api_sig`: MD5 of the sorted `name` + `value` pairs followed by the shared secret.

use md5::{Digest, Md5};

use crate::params::Params;

/// Parameters that never take part in the signature.
const UNSIGNED: [&str; 3] = ["format", "callback", "api_sig"];

/// The expected `api_sig` for these parameters, as 32 lowercase hex digits.
#[must_use]
pub fn api_signature(params: &Params, secret: &str) -> String {
    let mut hasher = Md5::new();
    for (name, value) in params.effective() {
        if !UNSIGNED.contains(&name) {
            hasher.update(name.as_bytes());
            hasher.update(value.as_bytes());
        }
    }
    hasher.update(secret.as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(32), |mut out, byte| {
            out.push(hex_digit(byte >> 4));
            out.push(hex_digit(byte & 0x0f));
            out
        })
}

/// Whether `params` carry an `api_sig` that matches the secret. Hex case is ignored.
#[must_use]
pub fn verify_signature(params: &Params, secret: &str) -> bool {
    let Some(given) = params.optional("api_sig") else {
        return false;
    };
    let expected = api_signature(params, secret);
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a.to_ascii_lowercase() ^ b))
            == 0
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}
