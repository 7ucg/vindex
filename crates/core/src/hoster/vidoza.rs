//! Vidoza-Extraktor.
//!
//! Portiert aus `extractors/provider/vidoza.py`: `src:"…"` im Seiten-JS
//! (wenn `sourcesCode:` vorhanden). Einzel-MP4.

use regex::Regex;

use super::Resolved;
use crate::error::{Error, Result};

/// Extrahiert die direkte MP4-URL aus einer Vidoza-Embed-Seite.
pub fn extract_url_from_html(html: &str) -> Option<String> {
    if !html.contains("sourcesCode:") {
        return None;
    }
    let re = Regex::new(r#"src:\s*"([^"]+)""#).unwrap();
    re.captures(html).map(|c| c[1].to_string())
}

/// Löst eine Vidoza-Embed-URL auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let resp = crate::http::get_retry(client, embed_url, None).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text)
        .ok_or_else(|| Error::extract("Vidoza", "keine src-URL gefunden"))?;
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
    fn vidoza_extract() {
        let html = r#"<script>sourcesCode: [{src: "https://cdn.vidoza/v.mp4", type:"video/mp4"}]</script>"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn.vidoza/v.mp4")
        );
    }

    #[test]
    fn vidoza_no_sources() {
        assert!(extract_url_from_html("<html>nope</html>").is_none());
    }
}
