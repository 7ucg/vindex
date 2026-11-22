//! LoadX-Extraktor (XHR-getSources-Muster).
//!
//! **Gegen Live nicht verifiziert.** Es liegt kein Live-Sample vor; die hier
//! implementierte Struktur folgt der *dokumentierten* Mechanik vergleichbarer
//! Hoster und ist bewusst defensiv gehalten.
//!
//! Mechanik: Aus der Embed-URL (`…/e/<id>` bzw. `…/embed/<id>`) wird die Id
//! gezogen und ein Player-Endpunkt per XHR abgefragt
//! (`…/player/index.php?data=<id>&do=getVideo`, Fallback `…/api/source/<id>`),
//! jeweils mit Header `X-Requested-With: XMLHttpRequest`. Die JSON-Antwort
//! enthält die Stream-URL unter `securedLink`/`videoSource`/`file`/`m3u8`.
//! `is_hls` folgt aus der Endung. KEIN AES.

use regex::Regex;
use serde_json::Value;

use super::Resolved;
use crate::error::{Error, Result};

/// Zieht die Video-Id aus einer LoadX-Embed-URL.
///
/// Akzeptiert `/e/<id>`, `/embed/<id>`, `/d/<id>`, `/v/<id>` sowie den
/// Query-Parameter `?v=<id>`/`?id=<id>`. Reine Funktion.
pub fn extract_code(embed_url: &str) -> Option<String> {
    // Query-Parameter zuerst (eindeutig).
    if let Some(c) = Regex::new(r"[?&](?:v|id|data)=([A-Za-z0-9_-]+)")
        .unwrap()
        .captures(embed_url)
    {
        return Some(c[1].to_string());
    }
    // Pfad-Segment nach /e/ /embed/ /d/ /v/.
    if let Some(c) = Regex::new(r"/(?:e|embed|d|v)/([A-Za-z0-9_-]+)")
        .unwrap()
        .captures(embed_url)
    {
        return Some(c[1].to_string());
    }
    None
}

/// Liest die Stream-URL aus einer (defensiv geparsten) JSON-Antwort.
///
/// Zuerst bekannte Schlüssel in Prioritätsreihenfolge, dann ein generischer
/// Scan nach der ersten plausiblen Medien-URL (m3u8 bevorzugt). Reine Funktion.
pub fn extract_url_from_json(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;

    // 1. Bekannte Schlüssel (rekursiv), geordnet nach Spezifität.
    for key in [
        "securedLink",
        "videoSource",
        "file",
        "source",
        "src",
        "m3u8",
        "url",
        "link",
    ] {
        if let Some(s) = find_key_string(&v, key) {
            if s.starts_with("http") {
                return Some(s);
            }
        }
    }

    // 2. Generischer Scan: erste Medien-URL, m3u8 bevorzugt, sonst mp4.
    let mut mp4_fallback: Option<String> = None;
    scan_media_url(&v, &mut mp4_fallback).or(mp4_fallback)
}

/// Sucht rekursiv den ersten String-Wert zum angegebenen Schlüssel.
fn find_key_string(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get(key) {
                return Some(s.clone());
            }
            for val in map.values() {
                if let Some(s) = find_key_string(val, key) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(|item| find_key_string(item, key)),
        _ => None,
    }
}

/// Scannt rekursiv nach einer Medien-URL; m3u8 sofort, mp4 als Fallback.
fn scan_media_url(v: &Value, mp4_fallback: &mut Option<String>) -> Option<String> {
    match v {
        Value::String(s) => {
            if s.starts_with("http") {
                if s.contains(".m3u8") {
                    return Some(s.clone());
                }
                if s.contains(".mp4") && mp4_fallback.is_none() {
                    *mp4_fallback = Some(s.clone());
                }
            }
            None
        }
        Value::Array(arr) => arr
            .iter()
            .find_map(|item| scan_media_url(item, mp4_fallback)),
        Value::Object(map) => map
            .values()
            .find_map(|val| scan_media_url(val, mp4_fallback)),
        _ => None,
    }
}

/// `scheme://host` einer URL, als Origin/Referer-Wert.
fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(format!("{}://{}", parsed.scheme(), host))
}

/// Löst eine LoadX-Embed-URL auf. **Gegen Live nicht verifiziert.**
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let origin =
        origin_of(embed_url).ok_or_else(|| Error::extract("LoadX", "ungültige Embed-URL"))?;
    let code =
        extract_code(embed_url).ok_or_else(|| Error::extract("LoadX", "keine Video-Id in URL"))?;

    // Endpunkt-Kandidaten (dokumentierte Varianten; nicht live verifiziert).
    let endpoints = [
        format!("{origin}/player/index.php?data={code}&do=getVideo"),
        format!("{origin}/api/source/{code}"),
    ];

    let mut last_reason = String::from("keine Antwort");
    for endpoint in endpoints {
        let resp = client
            .post(&endpoint)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", embed_url)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if crate::http::looks_like_challenge(&body, status) {
            return Err(Error::Challenge(embed_url.to_string()));
        }
        match extract_url_from_json(&body) {
            Some(url) => {
                return Ok(Resolved {
                    is_hls: url.contains(".m3u8"),
                    url,
                    referer: Some(origin),
                });
            }
            None => last_reason = format!("keine Stream-URL in Antwort ({endpoint})"),
        }
    }
    Err(Error::extract("LoadX", last_reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadx_id_aus_pfad() {
        assert_eq!(
            extract_code("https://loadx.test/e/abc123XYZ").as_deref(),
            Some("abc123XYZ")
        );
        assert_eq!(
            extract_code("https://loadx.test/embed/Z-9_a").as_deref(),
            Some("Z-9_a")
        );
    }

    #[test]
    fn loadx_id_aus_query() {
        assert_eq!(
            extract_code("https://loadx.test/player?v=qwe987").as_deref(),
            Some("qwe987")
        );
    }

    // Synthetische JSON-Antworten (frei erfunden, kein echter Inhalt).
    #[test]
    fn loadx_json_securedlink() {
        let body =
            r#"{"status":200,"securedLink":"https://cdn.loadx.test/hls/master.m3u8","title":"x"}"#;
        let url = extract_url_from_json(body).unwrap();
        assert_eq!(url, "https://cdn.loadx.test/hls/master.m3u8");
        assert!(url.contains(".m3u8"));
    }

    #[test]
    fn loadx_json_sources_array() {
        let body = r#"{"sources":[{"file":"https://cdn.loadx.test/v.mp4","label":"720p"}]}"#;
        assert_eq!(
            extract_url_from_json(body).as_deref(),
            Some("https://cdn.loadx.test/v.mp4")
        );
    }

    #[test]
    fn loadx_json_generischer_scan() {
        // Kein bekannter Schlüssel — generischer Scan muss m3u8 bevorzugen.
        let body = r#"{"meta":{"poster":"https://img.test/p.jpg"},"playlist":"https://cdn.loadx.test/stream/index.m3u8"}"#;
        assert_eq!(
            extract_url_from_json(body).as_deref(),
            Some("https://cdn.loadx.test/stream/index.m3u8")
        );
    }

    #[test]
    fn loadx_json_leer() {
        assert!(extract_url_from_json(r#"{"error":"not found"}"#).is_none());
        assert!(extract_url_from_json("kein json").is_none());
    }
}
