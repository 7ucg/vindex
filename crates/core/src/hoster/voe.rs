//! VOE-Extraktor.
//!
//! Portiert aus `extractors/provider/voe.py` (models, maintained):
//! `<script type="application/json">`-Blöcke → encoded String → Decode-Kette
//! (ROT13 → Junk→`_` entfernen → base64 → Shift(−3) → reverse → base64 → JSON)
//! → `source`. Plus Fallbacks: `var a168c='…'`, `'hls': '…'`, bare `.m3u8`.

use regex::Regex;
use serde_json::Value;

use super::common::{b64_decode_bytes, reverse, rot13, shift_down};
use super::Resolved;
use crate::error::{Error, Result};

/// Junk-Sequenzen, die VOE in den kodierten String einstreut.
const JUNK_PARTS: &[&str] = &["@$", "^^", "~@", "%?", "*~", "!!", "#&"];

/// Dekodiert einen VOE-encoded String zu JSON.
pub fn decode_voe_string(encoded: &str) -> Result<Value> {
    let step1 = rot13(encoded);
    let mut step2 = step1;
    for part in JUNK_PARTS {
        step2 = step2.replace(part, "_");
    }
    step2 = step2.replace('_', "");

    let step3 =
        b64_decode_bytes(&step2).ok_or_else(|| Error::extract("VOE", "base64 (Stufe 3)"))?;
    let step3_str =
        String::from_utf8(step3).map_err(|_| Error::extract("VOE", "utf8 (Stufe 3)"))?;

    let step4 = shift_down(&step3_str, 3);
    let step5_input = reverse(&step4);
    let step5 =
        b64_decode_bytes(&step5_input).ok_or_else(|| Error::extract("VOE", "base64 (Stufe 5)"))?;

    serde_json::from_slice(&step5).map_err(|e| Error::extract("VOE", format!("json: {e}")))
}

/// Extrahiert die Stream-URL (`source`) aus einer VOE-Embed-Seite.
pub fn extract_source_from_html(html: &str) -> Option<String> {
    let script_re = Regex::new(
        r#"(?is)<script\b[^>]*\btype\s*=\s*["']application/json["'][^>]*>(.*?)</script>"#,
    )
    .unwrap();

    // Variante 1: JSON-Script-Blöcke (String oder Array von Strings).
    for cap in script_re.captures_iter(html) {
        let block = cap[1].trim();
        if let Ok(payload) = serde_json::from_str::<Value>(block) {
            let encoded_strings: Vec<String> = match payload {
                Value::Array(arr) => arr
                    .into_iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Value::String(s) => vec![s],
                _ => vec![],
            };
            for enc in encoded_strings {
                if let Ok(decoded) = decode_voe_string(&enc) {
                    if let Some(src) = decoded.get("source").and_then(Value::as_str) {
                        if !src.is_empty() {
                            return Some(src.to_string());
                        }
                    }
                }
            }
        }
    }

    // Variante 2: var a168c='<encoded>'
    if let Some(c) = Regex::new(r"var a168c='([^']+)'").unwrap().captures(html) {
        if let Ok(decoded) = decode_voe_string(&c[1]) {
            if let Some(src) = decoded.get("source").and_then(Value::as_str) {
                return Some(src.to_string());
            }
        }
    }

    // Variante 3: 'hls': '<url>'
    if let Some(c) = Regex::new(r"'hls':\s*'([^']+)'").unwrap().captures(html) {
        return Some(c[1].to_string());
    }

    // Letzter Ausweg: irgendeine .m3u8-URL auf der Seite.
    if let Some(m) = Regex::new(r#"https?://[^\s'"<>]+?\.m3u8[^\s'"<>]*"#)
        .unwrap()
        .find(html)
    {
        return Some(m.as_str().to_string());
    }

    None
}

/// Löst eine VOE-Embed-URL zu einem direkten Stream auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let resp = crate::http::get_retry(client, embed_url, None).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let src = extract_source_from_html(&text)
        .ok_or_else(|| Error::extract("VOE", "keine Stream-URL in der Seite"))?;
    Ok(Resolved {
        is_hls: src.contains(".m3u8"),
        url: src,
        referer: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    // Baut einen VOE-encoded String aus einem JSON, indem die Decode-Kette
    // rückwärts angewandt wird.
    fn encode_voe(json: &str) -> String {
        // step5 = b64decode(reverse(step4))  ⇒ reverse(step4) = b64encode(json)
        let a = STANDARD.encode(json.as_bytes()); // = reverse(step4)
        let step4 = reverse(&a);
        // step4 = shift_down(step3_str, 3) ⇒ step3_str = shift_up 3
        let step3_str: String = step4
            .chars()
            .map(|c| char::from_u32(c as u32 + 3).unwrap())
            .collect();
        // step3 = b64decode(step2) ⇒ step2 = b64encode(step3_str bytes)
        let step2 = STANDARD.encode(step3_str.as_bytes());
        // step2 enthält weder Junk noch '_'; step1 = step2 ⇒ encoded = rot13(step2)
        rot13(&step2)
    }

    #[test]
    fn voe_decode_roundtrip() {
        let json = r#"{"source":"https://cdn.example.com/engine/master.m3u8?t=abc"}"#;
        let encoded = encode_voe(json);
        let decoded = decode_voe_string(&encoded).expect("decode");
        assert_eq!(
            decoded.get("source").and_then(|v| v.as_str()),
            Some("https://cdn.example.com/engine/master.m3u8?t=abc")
        );
    }

    #[test]
    fn voe_extract_from_script_block() {
        let json = r#"{"source":"https://cdn.x/master.m3u8"}"#;
        let encoded = encode_voe(json);
        let html =
            format!(r#"<html><script type="application/json">["{encoded}"]</script></html>"#);
        assert_eq!(
            extract_source_from_html(&html).as_deref(),
            Some("https://cdn.x/master.m3u8")
        );
    }

    #[test]
    fn voe_hls_fallback() {
        let html = "<script>var x = {'hls': 'https://h/a.m3u8'};</script>";
        assert_eq!(
            extract_source_from_html(html).as_deref(),
            Some("https://h/a.m3u8")
        );
    }
}
