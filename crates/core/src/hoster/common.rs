//! Gemeinsame Helfer für Hoster-Extraktoren.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use regex::Regex;

/// Standard-Base64-Decode → UTF-8-String.
pub fn b64_decode_str(s: &str) -> Option<String> {
    let bytes = STANDARD.decode(s.trim()).ok()?;
    String::from_utf8(bytes).ok()
}

/// Standard-Base64-Decode → Rohbytes.
pub fn b64_decode_bytes(s: &str) -> Option<Vec<u8>> {
    STANDARD.decode(s.trim()).ok()
}

/// base64url-Decode (mit/ohne Padding), wie `decode_base64url` der Referenz.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

/// ROT13 für Buchstaben (VOE).
pub fn rot13(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            _ => c,
        })
        .collect()
}

/// Verschiebt jedes Zeichen um *n* Codepoints nach unten (VOE/SpeedFiles).
pub fn shift_down(s: &str, n: u32) -> String {
    s.chars()
        .filter_map(|c| char::from_u32((c as u32).wrapping_sub(n)))
        .collect()
}

/// Vertauscht Groß-/Kleinschreibung (SpeedFiles).
pub fn swapcase(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect()
}

/// Kehrt eine Zeichenkette um.
pub fn reverse(s: &str) -> String {
    s.chars().rev().collect()
}

/// Dean-Edwards' „packed JavaScript" entpacken (Filemoon, legacy).
pub fn unpack_js(packed: &str, radix: u32, keywords: &[&str]) -> String {
    let word_re = Regex::new(r"\b(\w+)\b").unwrap();
    word_re
        .replace_all(packed, |caps: &regex::Captures| {
            let token = &caps[1];
            match decode_base_n(token, radix) {
                Some(i) if (i as usize) < keywords.len() && !keywords[i as usize].is_empty() => {
                    keywords[i as usize].to_string()
                }
                _ => token.to_string(),
            }
        })
        .into_owned()
}

fn decode_base_n(token: &str, radix: u32) -> Option<i64> {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut result: i64 = 0;
    for ch in token.bytes() {
        let digit = ALPHABET.iter().position(|&a| a == ch)? as u32;
        if digit >= radix {
            return None;
        }
        result = result * radix as i64 + digit as i64;
    }
    Some(result)
}

/// Pseudo-zufälliger alphanumerischer String (Doodstream-Direktlink).
///
/// Ohne `rand`-Abhängigkeit: aus Systemzeit-Nanos per xorshift.
pub fn pseudo_random_alnum(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            CHARS[(state % CHARS.len() as u64) as usize] as char
        })
        .collect()
}

/// Aktueller Unix-Zeitstempel in Sekunden.
pub fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot13_is_self_inverse() {
        assert_eq!(rot13(&rot13("Hello, VOE!")), "Hello, VOE!");
        assert_eq!(rot13("abc"), "nop");
    }

    #[test]
    fn swapcase_roundtrip() {
        assert_eq!(swapcase("AbC123"), "aBc123");
        assert_eq!(swapcase(&swapcase("MixedCase")), "MixedCase");
    }

    #[test]
    fn unpack_simple() {
        // p = "0 1", keywords = [var, x]; base 10 → "var x"
        let out = unpack_js("0 1", 10, &["var", "x"]);
        assert_eq!(out, "var x");
    }
}
