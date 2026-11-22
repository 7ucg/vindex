//! Filemoon-Extraktor.
//!
//! Portiert aus `extractors/provider/filemoon.py`. Zwei Pfade:
//! 1. **Byse-API** (`/api/videos/{code}`): moderner Weg. Die `playback`-Daten sind
//!    AES-256-GCM-verschlüsselt; der 32-Byte-Key wird aus `key_parts` zusammengesetzt
//!    (versionsabhängige Hälften), das entschlüsselte JSON enthält die Quellen.
//! 2. **Legacy-HTML**: „packed JavaScript" entpacken, dann `sources:[{file:…}]` /
//!    `.m3u8` / `file:…`. Fallback, wenn die API nichts liefert.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use regex::Regex;

use super::common::{b64url_decode, unpack_js};
use super::Resolved;
use crate::error::{Error, Result};

fn extract_url_from_text(text: &str) -> Option<String> {
    if let Some(c) = Regex::new(r#"['"](https?://[^'"]+\.m3u8[^'"]*)['"]"#)
        .unwrap()
        .captures(text)
    {
        return Some(c[1].to_string());
    }
    if let Some(c) = Regex::new(r#"(?s)sources\s*:\s*\[\s*\{[^}]*file:\s*['"]([^'"]+)['"]"#)
        .unwrap()
        .captures(text)
    {
        return Some(c[1].to_string());
    }
    if let Some(c) = Regex::new(r#"file\s*:\s*['"](https?://[^'"]+)['"]"#)
        .unwrap()
        .captures(text)
    {
        let url = c[1].to_string();
        let lower = url.to_lowercase();
        if lower.contains(".m3u8") || lower.contains(".mp4") {
            return Some(url);
        }
    }
    None
}

/// Extrahiert die Stream-URL aus Filemoon-HTML (packed JS oder direkte Patterns).
pub fn extract_url_from_html(html: &str) -> Option<String> {
    let packed_re = Regex::new(
        r"(?s)eval\(function\(p,a,c,k,e,d\)\{.*?\}\('(?P<p>[^']+)',\s*(?P<a>\d+),\s*(?P<c>\d+),\s*'(?P<k>[^']+)'\.split\('\|'\)",
    )
    .unwrap();
    if let Some(m) = packed_re.captures(html) {
        let packed = &m["p"];
        let radix: u32 = m["a"].parse().unwrap_or(10);
        let keywords: Vec<&str> = m["k"].split('|').collect();
        let unpacked = unpack_js(packed, radix, &keywords);
        if let Some(url) = extract_url_from_text(&unpacked) {
            return Some(url);
        }
    }
    extract_url_from_text(html)
}

// ───────────────────────────── Byse-API (AES-256-GCM) ─────────────────────────────

/// Obergrenze für die Byse-API-Antwort. Die JSON-Antwort (Keys + verschlüsselte
/// Quellenliste) ist klein; die Grenze hält eine manipulierte Antwort vom Speicher fern.
const MAX_BYSE_RESPONSE_BYTES: usize = 1024 * 1024;

/// Obergrenze für einen einzelnen AES-GCM-Ciphertext (base64url-dekodiert), bevor
/// entschlüsselt wird — die Quellenliste ist wenige KB groß.
const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

/// Datei-Code aus einer Filemoon/Byse-URL (`…/d/<code>` oder `…/e/<code>`).
pub fn extract_file_code(url: &str) -> Option<String> {
    Regex::new(r"/[de]/([a-zA-Z0-9]+)")
        .unwrap()
        .captures(url)
        .map(|c| c[1].to_string())
}

/// Baut die 32-Byte-Kandidaten-Keys aus `key_parts` + `version`. Der von der Version
/// ausgewählte Paar-Key kommt zuerst, danach beide Reihenfolgen der zwei 16-Byte-
/// Hälften. Nur 32-Byte-Keys bleiben übrig, Duplikate fallen raus.
fn playback_keys(key_parts: &[String], version: Option<i64>) -> Vec<[u8; 32]> {
    let parts: Vec<Vec<u8>> = key_parts
        .iter()
        .filter(|p| !p.is_empty())
        .filter_map(|p| b64url_decode(p))
        .collect();
    let mut keys: Vec<Vec<u8>> = Vec::new();
    let version = version.unwrap_or(0);
    if version > 0 && (version as usize) <= parts.len() {
        let v = version as usize;
        let mut k = parts[v - 1].clone();
        k.extend_from_slice(&parts[parts.len() - v]);
        keys.push(k);
    }
    let halves: Vec<&Vec<u8>> = parts.iter().filter(|p| p.len() == 16).collect();
    if halves.len() == 2 {
        let mut a = halves[0].clone();
        a.extend_from_slice(halves[1]);
        keys.push(a);
        let mut b = halves[1].clone();
        b.extend_from_slice(halves[0]);
        keys.push(b);
    }
    // Dedupe (Reihenfolge erhalten) und nur 32-Byte-Keys in feste Arrays überführen.
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut out = Vec::new();
    for k in keys {
        if k.len() == 32 && !seen.contains(&k) {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&k);
            out.push(arr);
            seen.push(k);
        }
    }
    out
}

/// Entschlüsselt ein AES-256-GCM-Feldpaar (iv/payload, base64url). `None` bei
/// fehlenden Feldern, falschem Tag oder — bewusst — nicht-96-Bit-IV (Standard-GCM).
fn decrypt_payload(
    playback: &serde_json::Value,
    key: &[u8; 32],
    iv_prop: &str,
    payload_prop: &str,
) -> Option<serde_json::Value> {
    let iv = b64url_decode(playback.get(iv_prop)?.as_str()?)?;
    let ciphertext = b64url_decode(playback.get(payload_prop)?.as_str()?)?;
    // aes-gcm nutzt einen 96-Bit-Nonce; andere IV-Längen wären GHASH-abgeleitet und
    // werden hier nicht unterstützt (reale Byse-Antworten liefern 12-Byte-IVs).
    if iv.len() != 12 || ciphertext.len() <= 16 || ciphertext.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&iv), ciphertext.as_ref())
        .ok()?;
    serde_json::from_slice(&plaintext).ok()
}

/// Entschlüsselt die `playback`-Daten der Byse-API. Probiert alle Key-Kandidaten
/// gegen beide Feldpaare (iv/payload, iv2/payload2).
pub fn decrypt_playback(playback: &serde_json::Value) -> Option<serde_json::Value> {
    let key_parts: Vec<String> = playback
        .get("key_parts")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if key_parts.is_empty() {
        return None;
    }
    let version = playback.get("version").and_then(|v| match v {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    });
    for key in playback_keys(&key_parts, version) {
        for (iv_prop, payload_prop) in [("iv", "payload"), ("iv2", "payload2")] {
            if let Some(result) = decrypt_payload(playback, &key, iv_prop, payload_prop) {
                return Some(result);
            }
        }
    }
    None
}

/// Wählt die beste Quell-URL aus dem entschlüsselten JSON (`sources[]` nach Höhe,
/// sonst `source`/`file`).
pub fn best_source_url(data: &serde_json::Value) -> Option<String> {
    if let Some(sources) = data.get("sources").and_then(|v| v.as_array()) {
        let mut best: Option<(&str, i64)> = None;
        for s in sources {
            let Some(url) = s.get("url").and_then(|v| v.as_str()) else {
                continue;
            };
            let height = s.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
            if best.is_none_or(|(_, h)| height >= h) {
                best = Some((url, height));
            }
        }
        if let Some((url, _)) = best {
            return Some(url.to_string());
        }
    }
    for prop in ["source", "file"] {
        if let Some(s) = data.get(prop).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Versucht den Byse-API-Pfad: `/api/videos/{code}` holen, `playback` entschlüsseln,
/// beste URL ziehen. `Ok(None)`, wenn die API nichts Brauchbares liefert.
async fn try_byse_api(
    client: &reqwest::Client,
    embed_url: &str,
    file_code: &str,
    referer: Option<&str>,
) -> Result<Option<String>> {
    let base = origin_of(embed_url);
    let Some(base) = base else { return Ok(None) };
    let api_url = format!("{base}/api/videos/{file_code}");
    let resp = crate::http::get_retry(client, &api_url, referer).await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let body = crate::http::capped_text(resp, MAX_BYSE_RESPONSE_BYTES).await?;
    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let Some(playback) = json.get("playback") else {
        return Ok(None);
    };
    Ok(decrypt_playback(playback)
        .as_ref()
        .and_then(best_source_url))
}

/// Löst eine Filemoon-Embed-URL auf: erst Byse-API (AES-GCM), dann Legacy-HTML.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    // Filemoon prüft beim Segment-Abruf den Referer; die Embed-Origin reicht.
    let referer = origin_of(embed_url);

    // 1. Moderner Byse-API-Pfad.
    if let Some(code) = extract_file_code(embed_url) {
        if let Some(url) = try_byse_api(client, embed_url, &code, referer.as_deref()).await? {
            return Ok(Resolved {
                is_hls: url.contains(".m3u8"),
                url,
                referer,
            });
        }
    }

    // 2. Legacy-HTML-Pfad (packed JS).
    let resp = crate::http::get_retry(client, embed_url, referer.as_deref()).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text)
        .ok_or_else(|| Error::extract("Filemoon", "keine Stream-URL (Byse-API + Legacy)"))?;
    Ok(Resolved {
        is_hls: url.contains(".m3u8"),
        url,
        referer,
    })
}

/// `scheme://host` einer URL, als Referer-Wert.
fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(format!("{}://{}", parsed.scheme(), host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filemoon_direct_m3u8() {
        let html = r#"<script>var s = "https://cdn.filemoon/x/master.m3u8";</script>"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn.filemoon/x/master.m3u8")
        );
    }

    #[test]
    fn filemoon_sources_block() {
        let html = r#"sources: [{file:"https://cdn.fm/v.mp4", label:"HD"}]"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn.fm/v.mp4")
        );
    }

    #[test]
    fn file_code_aus_url() {
        assert_eq!(
            extract_file_code("https://bysezejataos.com/d/56q7gpy3qyo6").as_deref(),
            Some("56q7gpy3qyo6")
        );
        assert_eq!(
            extract_file_code("https://filemoon.sx/e/abc123XYZ").as_deref(),
            Some("abc123XYZ")
        );
        assert_eq!(extract_file_code("https://filemoon.sx/about"), None);
    }

    #[test]
    fn best_source_waehlt_hoechste_hoehe() {
        let data = json!({"sources":[
            {"url":"https://x/low.m3u8","height":480},
            {"url":"https://x/hd.m3u8","height":1080},
            {"url":"https://x/mid.m3u8","height":720}
        ]});
        assert_eq!(best_source_url(&data).as_deref(), Some("https://x/hd.m3u8"));
    }

    #[test]
    fn best_source_faellt_auf_source_und_file() {
        assert_eq!(
            best_source_url(&json!({"source":"https://x/a.m3u8"})).as_deref(),
            Some("https://x/a.m3u8")
        );
        assert_eq!(
            best_source_url(&json!({"file":"https://x/b.mp4"})).as_deref(),
            Some("https://x/b.mp4")
        );
        assert_eq!(best_source_url(&json!({"other":1})), None);
    }

    /// Byse-Playback mit bekanntem Key verschlüsseln und zurück-entschlüsseln.
    #[test]
    fn byse_playback_roundtrip() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

        let h0: [u8; 16] = *b"0123456789abcdef";
        let h1: [u8; 16] = *b"FEDCBA9876543210";
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(&h0);
        key[16..].copy_from_slice(&h1);

        let iv: [u8; 12] = *b"nonce-12byte";
        let plain = json!({"sources":[
            {"url":"https://cdn/master.m3u8","height":1080},
            {"url":"https://cdn/sd.m3u8","height":360}
        ]})
        .to_string();

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let ct = cipher
            .encrypt(Nonce::from_slice(&iv), plain.as_bytes())
            .unwrap();

        // version=1, parts=[h0,h1] → parts[0]+parts[1] = h0+h1 = key.
        let playback = json!({
            "version": 1,
            "key_parts": [URL_SAFE_NO_PAD.encode(h0), URL_SAFE_NO_PAD.encode(h1)],
            "iv": URL_SAFE_NO_PAD.encode(iv),
            "payload": URL_SAFE_NO_PAD.encode(&ct),
        });

        let decrypted = decrypt_playback(&playback).expect("entschlüsselt");
        assert_eq!(
            best_source_url(&decrypted).as_deref(),
            Some("https://cdn/master.m3u8")
        );
    }

    #[test]
    fn byse_falscher_key_gibt_none() {
        let playback = json!({
            "version": 1,
            "key_parts": ["AAAA", "BBBB"],
            "iv": "YWJjZGVmZ2hpams",
            "payload": "ZGVhZGJlZWZkZWFkYmVlZmRlYWRiZWVm",
        });
        assert!(decrypt_playback(&playback).is_none());
    }

    #[test]
    fn playback_keys_nur_32_byte_und_dedupe() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let h0 = URL_SAFE_NO_PAD.encode([1u8; 16]);
        let h1 = URL_SAFE_NO_PAD.encode([2u8; 16]);
        // version=1 → h0+h1; danach halves → h0+h1 (dupe) und h1+h0.
        let keys = playback_keys(&[h0, h1], Some(1));
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().all(|k| k.len() == 32));
        assert_ne!(keys[0], keys[1]);
    }
}

