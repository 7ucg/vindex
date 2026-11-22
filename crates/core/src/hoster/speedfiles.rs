//! SpeedFiles-Extraktor.
//!
//! Portiert aus `downloaders/SpeedFiles.py`: Die Embed-Seite versteckt die MP4-URL
//! in `var _0x5opu234 = "…";`, dekodiert über eine feste 10-Schritt-Kette.
//! Nur aniworld listet diesen Hoster. Einzel-MP4 (ffprobe-Gate im Download).

use regex::Regex;

use super::common::{b64_decode_bytes, b64_decode_str, reverse, shift_down, swapcase};
use super::Resolved;
use crate::error::{Error, Result};

/// Dekodiert den SpeedFiles-Inhalt zur direkten Video-URL.
pub fn decode(content: &str) -> Result<String> {
    let err = |s: &str| Error::extract("SpeedFiles", s.to_string());

    // 1: base64
    let c1 = b64_decode_str(content).ok_or_else(|| err("base64 (1)"))?;
    // 2: swapcase
    let c2 = swapcase(&c1);
    // 3: reverse
    let c3 = reverse(&c2);
    // 4: base64
    let c4 = b64_decode_str(&c3).ok_or_else(|| err("base64 (4)"))?;
    // 5: reverse
    let c5 = reverse(&c4);
    // 6: je zwei Hex-Ziffern → ein Zeichen
    let bytes = c5.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(err("ungerade Hex-Länge (6)"));
    }
    let mut c6 = String::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hex = std::str::from_utf8(pair).map_err(|_| err("hex utf8 (6)"))?;
        let val = u32::from_str_radix(hex, 16).map_err(|_| err("hex parse (6)"))?;
        c6.push(char::from_u32(val).ok_or_else(|| err("char (6)"))?);
    }
    // 7: jedes Zeichen um 3 nach unten schieben
    let c7 = shift_down(&c6, 3);
    // 8: swapcase
    let c8 = swapcase(&c7);
    // 9: reverse
    let c9 = reverse(&c8);
    // 10: base64 → finale URL
    let url_bytes = b64_decode_bytes(&c9).ok_or_else(|| err("base64 (10)"))?;
    String::from_utf8(url_bytes).map_err(|_| err("utf8 (10)"))
}

/// Findet den kodierten Block und dekodiert ihn.
pub fn extract_url_from_html(html: &str) -> Option<String> {
    let re = Regex::new(r#"var _0x5opu234 = "(?P<c>.*?)";"#).unwrap();
    let c = re.captures(html)?;
    decode(&c["c"]).ok()
}

/// Löst eine SpeedFiles-Embed-URL zur direkten MP4-URL auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let resp = crate::http::get_retry(client, embed_url, None).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text)
        .ok_or_else(|| Error::extract("SpeedFiles", "kein kodierter Block gefunden"))?;
    Ok(Resolved {
        is_hls: url.contains(".m3u8"),
        url,
        referer: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    // Invertiert die 10-Schritt-Kette, um einen gültigen kodierten String zu bauen.
    fn encode(url: &str) -> String {
        // 10 inv: c9 = b64encode(url)
        let c9 = STANDARD.encode(url.as_bytes());
        // 9 inv: c8 = reverse(c9)
        let c8 = reverse(&c9);
        // 8 inv: c7 = swapcase(c8)
        let c7 = swapcase(&c8);
        // 7 inv: c6 = shift_up(c7, 3)
        let c6: String = c7
            .chars()
            .map(|c| char::from_u32(c as u32 + 3).unwrap())
            .collect();
        // 6 inv: c5 = hex-Paare je Zeichen
        let c5: String = c6.chars().map(|c| format!("{:02x}", c as u32)).collect();
        // 5 inv: c4 = reverse(c5)
        let c4 = reverse(&c5);
        // 4 inv: c3 = b64encode(c4)
        let c3 = STANDARD.encode(c4.as_bytes());
        // 3 inv: c2 = reverse(c3)
        let c2 = reverse(&c3);
        // 2 inv: c1 = swapcase(c2)
        let c1 = swapcase(&c2);
        // 1 inv: content = b64encode(c1)
        STANDARD.encode(c1.as_bytes())
    }

    #[test]
    fn speedfiles_decode_roundtrip() {
        let url = "https://speedfiles.example/video/abc123.mp4";
        let content = encode(url);
        assert_eq!(decode(&content).unwrap(), url);
    }

    #[test]
    fn speedfiles_extract_from_html() {
        let url = "https://sf.example/v.mp4";
        let content = encode(url);
        let html = format!(r#"<script>var _0x5opu234 = "{content}";</script>"#);
        assert_eq!(extract_url_from_html(&html).as_deref(), Some(url));
    }

    #[test]
    fn speedfiles_ohne_block_ist_none() {
        assert_eq!(extract_url_from_html("<html>kein block</html>"), None);
    }

    #[test]
    fn speedfiles_kaputter_block_ist_none_statt_panic() {
        // Block vorhanden, Inhalt aber kein gültiger kodierter String: die Kette
        // scheitert sauber (Err → None), ohne zu paniken.
        let html = r#"<script>var _0x5opu234 = "nicht-base64-!!!";</script>"#;
        assert_eq!(extract_url_from_html(html), None);
    }

    #[test]
    fn speedfiles_decode_muell_ist_err() {
        assert!(decode("@@nicht base64@@").is_err());
    }
}
