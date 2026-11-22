//! Streamtape-Extraktor.
//!
//! Muster wie Doodstream (kein AES, kein packed JS): Die Embed-Seite setzt die
//! Download-URL per JavaScript zusammen. Ein `<div id="robotlink">` (manche
//! Varianten: `id="ideoolink"`) erhält per `innerHTML` die erste Hälfte des
//! Pfads; der direkt folgende String-Literal liefert — ab `substring(n)` — die
//! zweite Hälfte. Beide Hälften ergeben zusammen `//streamtape…/get_video?…`;
//! mit vorangestelltem `https:` und angehängtem `&stream=1` entsteht der
//! direkte MP4-Stream. `Referer` = Embed-Origin, `is_hls=false`.

use regex::Regex;

use super::Resolved;
use crate::error::{Error, Result};

/// Setzt die Streamtape-Download-URL aus den beiden JS-Hälften zusammen.
///
/// Reine Funktion (ohne Netz) — Kern der Deobfuskation, direkt testbar.
pub fn build_stream_url(html: &str) -> Option<String> {
    // innerHTML = 'HÄLFTE1' + ('HÄLFTE2').substring(N)
    // Die Klammern um die zweite Hälfte sind optional; Anführungszeichen beide
    // Varianten (' oder ").
    let re = Regex::new(
        r#"(?s)innerHTML\s*=\s*['"](?P<p1>[^'"]*)['"]\s*\+\s*\(?\s*['"](?P<p2>[^'"]*)['"]\s*\)?\s*\.substring\(\s*(?P<n>\d+)\s*\)"#,
    )
    .unwrap();
    let caps = re.captures(html)?;
    let p1 = &caps["p1"];
    let p2 = &caps["p2"];
    let n: usize = caps["n"].parse().ok()?;

    // Zweite Hälfte ab Zeichen-Index n (nicht Byte-Index) anhängen.
    let tail: String = p2.chars().skip(n).collect();
    let path = format!("{p1}{tail}");
    if path.is_empty() {
        return None;
    }

    // `https:` voranstellen (Pfad beginnt i. d. R. mit `//host/…`).
    let mut url = if path.starts_with("http") {
        path
    } else if let Some(rest) = path.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = path.strip_prefix('/') {
        format!("https://{rest}")
    } else {
        format!("https://{path}")
    };

    // `&stream=1` erzwingt den direkten MP4-Stream (statt Weiterleitung).
    if !url.contains("stream=1") {
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str("stream=1");
    }
    Some(url)
}

/// `scheme://host` einer URL, als Referer-Wert.
fn origin_of(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(format!("{}://{}", parsed.scheme(), host))
}

/// Löst eine Streamtape-Embed-URL zum direkten MP4-Link auf.
pub async fn resolve(client: &reqwest::Client, embed_url: &str) -> Result<Resolved> {
    let referer = origin_of(embed_url);
    let resp = crate::http::get_retry(client, embed_url, referer.as_deref()).await?;
    let status = resp.status().as_u16();
    let html = resp.text().await?;
    if crate::http::looks_like_challenge(&html, status) {
        return Err(Error::Challenge(embed_url.to_string()));
    }

    let url = build_stream_url(&html)
        .ok_or_else(|| Error::extract("Streamtape", "robotlink-URL nicht zusammensetzbar"))?;
    Ok(Resolved {
        is_hls: false,
        url,
        referer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetisches Fixture (frei erfunden, kein echter Inhalt).
    #[test]
    fn streamtape_url_zusammenbau() {
        let html = r#"
            <div id="robotlink" style="display:none;">//dummylink.test/x</div>
            <script>
            document.getElementById('robotlink').innerHTML = '//streamtape.test/get_video?id=ABC&expires=123&ip=1&token=' + ('xxToKEN99').substring(2);
            </script>
        "#;
        // p1 = "//streamtape.test/get_video?id=ABC&expires=123&ip=1&token="
        // p2 ab Index 2 = "ToKEN99"
        assert_eq!(
            build_stream_url(html).as_deref(),
            Some(
                "https://streamtape.test/get_video?id=ABC&expires=123&ip=1&token=ToKEN99&stream=1"
            )
        );
    }

    #[test]
    fn streamtape_ideoolink_variante_mit_klammern() {
        let html = r#"
            <script>
            document.getElementById("ideoolink").innerHTML = "//cdn.streamtape.test/get_video?id=Z&token=aa"+("bbTAIL").substring(2);
            </script>
        "#;
        assert_eq!(
            build_stream_url(html).as_deref(),
            Some("https://cdn.streamtape.test/get_video?id=Z&token=aaTAIL&stream=1")
        );
    }

    #[test]
    fn streamtape_ohne_treffer() {
        assert!(build_stream_url("<html>nichts hier</html>").is_none());
    }
}
