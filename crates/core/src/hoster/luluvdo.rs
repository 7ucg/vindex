//! Luluvdo-Extraktor.
//!
//! Muster wie Filemoon (Legacy-HTML-Pfad): Die Embed-Seite `/e/<code>` enthält
//! „packed JavaScript" (`eval(function(p,a,c,k,e,d){…})`). Nach dem Entpacken
//! via [`common::unpack_js`] steht die Quelle als `sources:[{file:"…m3u8"}]`
//! bzw. `file:"…"` im Klartext. `is_hls` ergibt sich aus der Endung,
//! `Referer` = Embed-Origin.
//!
//! Die moderne **AES-GCM-API-Variante** (Player-Endpunkt mit verschlüsselter
//! Antwort) ist hier bewusst NICHT umgesetzt — nur der packed-JS/HTML-Pfad.

use regex::Regex;

use super::common::unpack_js;
use super::Resolved;
use crate::error::{Error, Result};

/// Extrahiert eine Stream-URL aus bereits entpacktem/klartextigem Script-Text.
fn extract_url_from_text(text: &str) -> Option<String> {
    // 1. Direkte m3u8-URL.
    if let Some(c) = Regex::new(r#"['"](https?://[^'"]+\.m3u8[^'"]*)['"]"#)
        .unwrap()
        .captures(text)
    {
        return Some(c[1].to_string());
    }
    // 2. sources:[{file:"…"}]
    if let Some(c) = Regex::new(r#"(?s)sources\s*:\s*\[\s*\{[^}]*file:\s*['"]([^'"]+)['"]"#)
        .unwrap()
        .captures(text)
    {
        return Some(c[1].to_string());
    }
    // 3. file:"…" (nur Video-Endungen akzeptieren).
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

/// Extrahiert die Stream-URL aus Luluvdo-HTML (packed JS entpacken, dann Muster).
///
/// Reine Funktion (ohne Netz) — direkt testbar.
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
    // Fallback: Muster direkt im (nicht gepackten) HTML.
    extract_url_from_text(html)
}

/// `scheme://host` einer URL, als Referer-Wert.
fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(format!("{}://{}", parsed.scheme(), host))
}

/// Löst eine Luluvdo-Embed-URL auf (packed-JS/HTML-Pfad).
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let referer = origin_of(embed_url);
    let resp = crate::http::get_retry(client, embed_url, referer.as_deref()).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text).ok_or_else(|| {
        Error::extract(
            "Luluvdo",
            "keine Stream-URL (packed-JS/HTML-Pfad; AES-GCM-API nicht implementiert)",
        )
    })?;
    Ok(Resolved {
        is_hls: url.contains(".m3u8"),
        url,
        referer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetisches packed-JS-Fixture (frei erfunden). base36, keywords per '|'.
    // Nach dem Entpacken steht eine direkte m3u8-URL im Klartext.
    #[test]
    fn luluvdo_m3u8_aus_packed_js() {
        // Tokens: 1→https, 2→cdn, 3→lulu, 4→test, 5→master. Die Ziffern-Tokens
        // der URL (.m3u8-Pfadsegmente) bleiben als Zahlen stehen — nur die
        // Platzhalter 1..5 werden ersetzt.
        let packed = r#"eval(function(p,a,c,k,e,d){return p}('var s="1://2.3.4/v/5.m3u8";',36,6,'file|https|cdn|lulu|test|master'.split('|'),0,{}))"#;
        assert_eq!(
            extract_url_from_html(packed).as_deref(),
            Some("https://cdn.lulu.test/v/master.m3u8")
        );
    }

    #[test]
    fn luluvdo_sources_block_klartext() {
        let html =
            r#"<script>sources: [{file:"https://cdn.lulu.test/v.mp4", label:"HD"}]</script>"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn.lulu.test/v.mp4")
        );
        // mp4 ⇒ kein HLS
        assert!(!extract_url_from_html(html).unwrap().contains(".m3u8"));
    }

    #[test]
    fn luluvdo_ohne_treffer() {
        assert!(extract_url_from_html("<html>leer</html>").is_none());
    }
}
