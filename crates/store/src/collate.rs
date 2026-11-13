//! The order Last.fm gives entities with equal play counts.
//!
//! Observed in the 2026-11-12 fixtures: ties in top lists, the library and weekly charts are
//! sorted by name, ignoring case (`beabadoobee`, `Beach House`, `Billy Idol`), with Latin before
//! Greek, Hangul before kana, and kana before Han. That is a Unicode collation, as a database
//! with a `unicode_ci` collation sorts. This is an approximation of it: fold case, drop accents,
//! then compare script by script and by code point within a script.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// A key that sorts names the way Last.fm sorts ties.
#[must_use]
pub fn collation_key(name: &str) -> Vec<u32> {
    // Case folding and NFKD leave ASCII as lowercase ASCII, and no ASCII character is
    // ignorable or a combining mark; skipping them is most of the cost for most names.
    if name.is_ascii() {
        return name
            .bytes()
            .map(|byte| weight(char::from(byte.to_ascii_lowercase())))
            .collect();
    }
    caseless::default_case_fold_str(name)
        .nfkd()
        .filter(|c| !is_combining_mark(*c) && !is_ignorable(*c))
        .flat_map(base_letters)
        .map(weight)
        .collect()
}

/// Zero-width and other format characters, which collation skips (the joiner in emoji).
fn is_ignorable(c: char) -> bool {
    matches!(u32::from(c), 0x200B..=0x200F | 0x2060..=0x206F | 0xFEFF)
}

/// Letters with a stroke or ligatures, which have no decomposition but sort with their base
/// letters (`Rønin` before `rylend`, `Łona` among the `L`s).
fn base_letters(c: char) -> impl Iterator<Item = char> {
    let (first, second) = match c {
        'ø' => ('o', None),
        'ł' => ('l', None),
        'đ' | 'ð' => ('d', None),
        'ħ' => ('h', None),
        'ŧ' => ('t', None),
        'ı' => ('i', None),
        'æ' => ('a', Some('e')),
        'œ' => ('o', Some('e')),
        other => (other, None),
    };
    std::iter::once(first).chain(second)
}

/// The tie key of an entity: its artist's name first for albums and tracks, then its own name.
pub(crate) fn tie_key(artist: Option<&str>, name: &str) -> (Vec<u32>, Vec<u32>) {
    (
        artist.map(collation_key).unwrap_or_default(),
        collation_key(name),
    )
}

/// ASCII punctuation, then ASCII symbols, each in the order the Unicode collation table gives.
const ASCII_PUNCTUATION: &[u8] = b" _-,;:!?.'\"()[]{}@*/\\&#%";
const ASCII_SYMBOLS: &[u8] = b"`^+<=>|~";

/// Groups in collation order: punctuation, symbols (emoji among them), currency, digits, then
/// the scripts.
const PUNCTUATION: u32 = 0;
const OTHER_PUNCTUATION: u32 = 1;
const SYMBOL: u32 = 2;
const CURRENCY: u32 = 3;
const DIGIT: u32 = 4;
const LATIN: u32 = 5;
const GREEK: u32 = 6;
const CYRILLIC: u32 = 7;
const OTHER_SCRIPT: u32 = 8;
const HANGUL: u32 = 9;
const KANA: u32 = 10;
const HAN: u32 = 11;
/// Characters newer than Last.fm's collation table, which sort after everything.
const UNLISTED: u32 = 12;

/// The group in the high bits, then a position within the group.
fn weight(c: char) -> u32 {
    // The common case first: neither ASCII list holds a letter or a digit.
    match c {
        'a'..='z' => return (LATIN << 21) | (u32::from(c) << 1),
        '0'..='9' => return (DIGIT << 21) | u32::from(c),
        _ => {}
    }
    if let Some(letter) = small_capital(c) {
        return weight(letter) | 1;
    }
    // Typographic quotes sort with the ASCII ones.
    let code = match u32::from(c) {
        0x2018..=0x201B | 0x2039 | 0x203A => u32::from(b'\''),
        0x201C..=0x201F | 0xAB | 0xBB => u32::from(b'"'),
        other => other,
    };
    let ascii = |set: &[u8]| set.iter().position(|b| u32::from(*b) == code);
    if let Some(at) = ascii(ASCII_PUNCTUATION) {
        return (PUNCTUATION << 21) | u32::try_from(at).unwrap_or(0);
    }
    if let Some(at) = ascii(ASCII_SYMBOLS) {
        return (SYMBOL << 21) | u32::try_from(at).unwrap_or(0);
    }
    let group = match code {
        0x24 | 0xA2..=0xA5 | 0x20A0..=0x20CF | 0xFFE0 | 0xFFE1 | 0xFFE5 | 0xFFE6 => CURRENCY,
        // General and CJK punctuation, including the katakana middle dot NFKD makes of `･`.
        0xA1
        | 0xA7
        | 0xB6
        | 0xB7
        | 0xBF
        | 0x2010..=0x205E
        | 0x3000..=0x303F
        | 0x30A0
        | 0x30FB
        | 0xFE10..=0xFE6F => OTHER_PUNCTUATION,
        0x30..=0x39 => DIGIT,
        0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => LATIN,
        0x370..=0x3FF => GREEK,
        0x400..=0x52F => CYRILLIC,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => HANGUL,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => KANA,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x2_0000..=0x3_FFFF => HAN,
        _ if is_unlisted(code) => UNLISTED,
        _ if c.is_numeric() => DIGIT,
        _ if c.is_alphabetic() => OTHER_SCRIPT,
        // Other symbols, emoji among them, follow the ASCII ones.
        _ => SYMBOL,
    };
    let position = match group {
        SYMBOL => code + 0x80,
        LATIN => code << 1,
        KANA => kana_sound(code),
        _ => code,
    };
    (group << 21) | position
}

/// Kana sort by sound: katakana with the hiragana of the same sound, and small kana right
/// after their full-size letter (`ア`, `カ`, `グ`, `チ`, `や` on ploxty's library). Voicing marks are combining
/// characters after NFKD and already dropped.
fn kana_sound(code: u32) -> u32 {
    let hiragana = match code {
        0x30A1..=0x30F6 => code - 0x60,
        other => other,
    };
    let full = match hiragana {
        0x3041 | 0x3043 | 0x3045 | 0x3047 | 0x3049 | 0x3063 | 0x3083 | 0x3085 | 0x3087
        | 0x308E => Some(hiragana + 1),
        0x3095 => Some(0x304B),
        0x3096 => Some(0x3051),
        _ => None,
    };
    full.map_or(hiragana << 1, |full| (full << 1) | 1)
}

/// Emoji added in Unicode 11 or later. Observed: `🥵` (Unicode 11) sorts after every name,
/// Han included, where `🤯` (Unicode 10) sorts among the other emoji.
fn is_unlisted(code: u32) -> bool {
    let listed = matches!(
        code,
        0x1_F910..=0x1_F93E
            | 0x1_F940..=0x1_F94C
            | 0x1_F950..=0x1_F96B
            | 0x1_F980..=0x1_F997
            | 0x1_F9C0
            | 0x1_F9D0..=0x1_F9E6
    );
    let newer_block = matches!(
        code,
        0x1_F900..=0x1_FAFF | 0x1_F6D5..=0x1_F6DF | 0x1_F6F9..=0x1_F6FF | 0x1_F7E0..=0x1_F7FF
    );
    newer_block && !listed
}

/// Small capitals sort right after their letter (`ʟᴏꜱᴛ` after every other `l` word).
fn small_capital(c: char) -> Option<char> {
    let letter = match c {
        'ᴀ' => 'a',
        'ʙ' => 'b',
        'ᴄ' => 'c',
        'ᴅ' => 'd',
        'ᴇ' => 'e',
        'ꜰ' => 'f',
        'ɢ' => 'g',
        'ʜ' => 'h',
        'ɪ' => 'i',
        'ᴊ' => 'j',
        'ᴋ' => 'k',
        'ʟ' => 'l',
        'ᴍ' => 'm',
        'ɴ' => 'n',
        'ᴏ' => 'o',
        'ᴘ' => 'p',
        'ʀ' => 'r',
        'ꜱ' => 's',
        'ᴛ' => 't',
        'ᴜ' => 'u',
        'ᴠ' => 'v',
        'ᴡ' => 'w',
        'ʏ' => 'y',
        'ᴢ' => 'z',
        _ => return None,
    };
    Some(letter)
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
            "„Raus mit die Viecher!“",
            "[Kyle Davis]",
            "[unknown]",
            "『ユイカ』",
            "･ ･－･ ･－",
            "･－･･ ･ －",
            "☠️😨Das ist",
            "🔇 Juice WRLD",
            "$NOT",
            "¥$",
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
            "Łona",
            "ʟᴏꜱᴛ",
            "Mac",
            "Mike",
            "Rønin",
            "rylend",
            "Z. Smith",
            "z13",
            "Zzz. & Trippie Redd",
            "Zzz. mini vlog",
            "ZZZJuice999",
            "Σtella",
            "데이먼스 이어 Damons year",
            "アイデンティティ／IDENTITY (Cover)",
            "あたらよ",
            "カミサマ・ネコサマ",
            "グラサンねこ",
            "コーコーヤ",
            "チュパカぶる？",
            "やくしまるえつこ Yakushimaru Etsuko",
            "不優雅（biuya）",
            "久石譲",
            "🥵😂REAKTION",
        ];
        let mut shuffled = want;
        shuffled.reverse();
        assert_eq!(sorted(&shuffled), want);
    }
}
