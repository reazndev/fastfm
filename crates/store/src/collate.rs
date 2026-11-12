//! The order Last.fm gives entities with equal play counts.
//!
//! Observed in the 2026-11-12 fixtures: ties in top lists, the library and weekly charts are
//! sorted by name, ignoring case (`beabadoobee`, `Beach House`, `Billy Idol`), with Latin before
//! Greek, Hangul before kana, and kana before Han. That is a Unicode collation, as a database
//! with a `unicode_ci` collation sorts. This is an approximation of it: fold case, drop accents,
//! then compare script by script and by code point within a script.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::types::Entity;

/// A key that sorts names the way Last.fm sorts ties.
#[must_use]
pub fn collation_key(name: &str) -> Vec<u32> {
    caseless::default_case_fold_str(name)
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .map(weight)
        .collect()
}

/// The tie key of an entity: its artist's name first for albums and tracks, then its own name.
pub(crate) fn entity_key(entity: &Entity) -> (Vec<u32>, Vec<u32>) {
    let artist = entity
        .artist
        .as_ref()
        .map(|artist| collation_key(&artist.name))
        .unwrap_or_default();
    (artist, collation_key(&entity.name))
}

/// ASCII punctuation and symbols in the order the Unicode collation table gives them.
const ASCII_SYMBOLS: &[u8] = b" _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// The script group in the high bits, then a position within the group.
fn weight(c: char) -> u32 {
    let code = u32::from(c);
    if let Some(at) = ASCII_SYMBOLS.iter().position(|b| u32::from(*b) == code) {
        return u32::try_from(at).unwrap_or(0);
    }
    let group = match code {
        0x30..=0x39 => 1,
        // CJK punctuation, including the katakana middle dot that NFKD makes of `･`.
        0x3000..=0x303F | 0x30A0 | 0x30FB => 0,
        0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => 2,
        0x370..=0x3FF => 3,
        0x400..=0x52F => 4,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => 6,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => 7,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x2_0000..=0x3_FFFF => 8,
        _ if c.is_numeric() => 1,
        _ if c.is_alphabetic() => 5,
        // Other symbols follow the ASCII ones.
        _ => return 0x100 + code,
    };
    (group << 21) | code
}

#[cfg(test)]
mod tests {
    use super::collation_key;

    fn sorted(names: &[&'static str]) -> Vec<&'static str> {
        let mut out = names.to_vec();
        out.sort_by_key(|name| collation_key(name));
        out
    }

    #[test]
    fn sorts_like_the_fixtures() {
        let want = [
            "･ ･－･ ･－",
            "･－･･ ･ －",
            "20 three",
            "a-ha",
            "A$AP Rocky",
            "An April March",
            "ARVERNE",
            "beabadoobee",
            "Beach House",
            "Björk",
            "blue diner.",
            "Blue Imagined",
            "Z. Smith",
            "z13",
            "Zzz. & Trippie Redd",
            "Zzz. mini vlog",
            "ZZZJuice999",
            "Σtella",
            "데이먼스 이어 Damons year",
            "あたらよ",
            "コーコーヤ",
            "不優雅（biuya）",
            "久石譲",
        ];
        let mut shuffled = want;
        shuffled.reverse();
        assert_eq!(sorted(&shuffled), want);
    }
}
