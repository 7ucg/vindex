//! Doodstream-Extraktor.
//!
//! Portiert aus `extractors/provider/doodstream.py`: Embed-Seite liefert eine
//! `…/pass_md5/…`-URL und einen `token=…`; `GET pass_md5` ergibt die Basis-URL,
//! der direkte Link ist `base + random10 + ?token=…&expiry=epoch`.
//! Benötigt `Referer: https://dood.li/`. Einzel-MP4.

use regex::Regex;

use super::common::{epoch_secs, pseudo_random_alnum};
use super::Resolved;
use crate::error::{Error, Result};

const BASE_URL: &str = "https://dood.li";

fn pass_md5_url(html: &str) -> Option<String> {
    let re = Regex::new(r"\$\.get\('([^']*/pass_md5/[^']*)'").unwrap();
    let path = re.captures(html)?[1].to_string();
    if path.starts_with("http") {
        Some(path)
    } else {
        Some(format!(
            "{BASE_URL}{}",
            if path.starts_with('/') {
                path.clone()
            } else {
                format!("/{path}")
            }
        ))
    }
}

fn token(html: &str) -> Option<String> {
    Regex::new(r"token=([a-zA-Z0-9]+)")
        .unwrap()
        .captures(html)
        .map(|c| c[1].to_string())
}

/// Löst eine Doodstream-Embed-URL zum direkten Link auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let referer = format!("{BASE_URL}/");
    let resp = client
        .get(embed_url)
        .header("Referer", &referer)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let html = resp.text().await?;
    if crate::http::looks_like_challenge(&html, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }

    let pass_url =
        pass_md5_url(&html).ok_or_else(|| Error::extract("Doodstream", "pass_md5-URL fehlt"))?;
    let tok = token(&html).ok_or_else(|| Error::extract("Doodstream", "token fehlt"))?;

    let base = client
        .get(&pass_url)
        .header("Referer", &referer)
        .send()
        .await?
        .text()
        .await?;
    let base = base.trim();
    if base.is_empty() {
        return Err(Error::extract("Doodstream", "leere Basis-URL"));
    }

    let url = format!(
        "{base}{}?token={tok}&expiry={}",
        pseudo_random_alnum(10),
        epoch_secs()
    );
    Ok(Resolved {
        is_hls: false,
        url,
        referer: Some(referer),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_pass_md5_and_token() {
        let html = r#"<script>$.get('/pass_md5/abc/def', function(){}); var x = "...token=XYZ123&foo";</script>"#;
        assert_eq!(
            pass_md5_url(html).as_deref(),
            Some("https://dood.li/pass_md5/abc/def")
        );
        assert_eq!(token(html).as_deref(), Some("XYZ123"));
    }
}
