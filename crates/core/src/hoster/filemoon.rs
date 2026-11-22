//! Filemoon-Extraktor.
//!
//! Portiert aus `extractors/provider/filemoon.py` — hier der **Legacy-HTML-Pfad**:
//! „packed JavaScript" entpacken, dann `sources:[{file:…}]` / `.m3u8` / `file:…`.
//!
//! Der moderne **Byse-API-Pfad** (`/api/videos/{code}`, AES-256-GCM) ist als
//! Erweiterungspunkt markiert — er braucht AES-GCM + die Key-Halbierungslogik und
//! wird gemäß `ARCHITECTURE.md` §8 (Hoster-Wechsel/Versionierung) nachgezogen.

use regex::Regex;

use super::common::unpack_js;
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

/// Löst eine Filemoon-Embed-URL auf (Legacy-HTML-Pfad).
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let resp = client.get(embed_url).send().await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text).ok_or_else(|| {
        Error::extract(
            "Filemoon",
            "keine Stream-URL (Legacy-Pfad; Byse-API noch nicht implementiert)",
        )
    })?;
    Ok(Resolved {
        is_hls: url.contains(".m3u8"),
        url,
        referer: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
