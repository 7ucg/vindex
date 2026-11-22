//! Vidmoly-Extraktor.
//!
//! Portiert aus `extractors/provider/vidmoly.py`: `file:"…m3u8…"` im Seiten-JS.
//! Benötigt `Referer: https://vidmoly.biz`. Liefert HLS.

use regex::Regex;

use super::Resolved;
use crate::error::{Error, Result};

const REFERER: &str = "https://vidmoly.biz";

/// Extrahiert die HLS-URL aus zusammengefügtem Script-Inhalt.
pub fn extract_url_from_html(html: &str) -> Option<String> {
    let script_re = Regex::new(r"(?is)<script[^>]*>(.*?)</script>").unwrap();
    let scripts: String = script_re
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let file_re = Regex::new(r#"file\s*:\s*['"]([^'"]+?\.m3u8[^'"]*)['"]"#).unwrap();
    file_re.captures(&scripts).map(|c| c[1].to_string())
}

/// Löst eine Vidmoly-Embed-URL auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let resp = crate::http::get_retry(client, embed_url, Some(REFERER)).await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    if crate::http::looks_like_challenge(&text, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }
    let url = extract_url_from_html(&text)
        .ok_or_else(|| Error::extract("Vidmoly", "keine m3u8-URL gefunden"))?;
    Ok(Resolved {
        is_hls: true,
        url,
        referer: Some(REFERER.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vidmoly_extract() {
        let html = r#"<script> sources: [{file:"https://cdn.vidmoly/hls/master.m3u8"}] </script>"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn.vidmoly/hls/master.m3u8")
        );
    }

    #[test]
    fn vidmoly_findet_m3u8_im_zweiten_script() {
        // Script-Inhalte werden zusammengefügt; der Treffer darf in einem späteren Block stehen.
        let html =
            r#"<script>var a=1;</script><script>file: 'https://cdn/v/master.m3u8?x=1'</script>"#;
        assert_eq!(
            extract_url_from_html(html).as_deref(),
            Some("https://cdn/v/master.m3u8?x=1")
        );
    }

    #[test]
    fn vidmoly_nicht_m3u8_file_wird_ignoriert() {
        // Nur .m3u8 zählt; ein mp4-file: ist kein Treffer.
        let html = r#"<script>file:"https://cdn/v.mp4"</script>"#;
        assert!(extract_url_from_html(html).is_none());
    }

    #[test]
    fn vidmoly_ohne_script_ist_none() {
        assert!(extract_url_from_html("<html>nichts</html>").is_none());
    }
}
