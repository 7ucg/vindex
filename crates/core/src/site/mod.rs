//! Seiten-Scraper.
//!
//! [`aniworld`] enthält das Census-Fundament; [`serienstream`] den generischen
//! Download-Scraper für aniworld.to **und** serienstream.to. [`aniworld`]/[`sto`] sind dünne
//! Wrapper, die die Basis-URL festlegen.

pub mod aniworld;
pub mod autocorrect;
pub mod filmo;
pub mod serienstream;
pub mod sto;

use crate::model::{BASE, BASE_STO};

/// Spiegel-Host für Transport-Ausfälle. s.to und serienstream.to liefern denselben
/// Katalog; fällt einer aus, wird der andere versucht.
fn mirror_host(host: &str) -> Option<&'static str> {
    match host {
        "s.to" | "www.s.to" => Some("serienstream.to"),
        "serienstream.to" | "www.serienstream.to" => Some("s.to"),
        _ => None,
    }
}

/// Ersetzt den Host einer URL durch ihren Spiegel, falls es einen gibt.
fn mirror_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let alt = mirror_host(&host)?;
    let mut m = parsed;
    m.set_host(Some(alt)).ok()?;
    Some(m.to_string())
}

/// `GET` mit Retry und Spiegel-Fallback: scheitert der primäre Host transient
/// (Transportfehler oder Down-Status), wird einmal gegen den Spiegel versucht.
pub async fn fetch(client: &reqwest::Client, url: &str) -> crate::error::Result<reqwest::Response> {
    let resp = crate::http::get_retry(client, url, None).await;
    let needs_mirror = match &resp {
        Err(_) => true,
        Ok(r) => matches!(r.status().as_u16(), 403 | 429 | 500 | 502 | 503 | 504),
    };
    if needs_mirror {
        if let Some(alt) = mirror_url(url) {
            return Ok(crate::http::get_retry(client, &alt, None).await?);
        }
    }
    Ok(resp?)
}

/// Wie [`fetch`], gibt aber direkt den Body-Text zurück.
///
/// Anti-Bot Stufe 2 (opt-in, `VINDEX_IMPERSONATE`): Sieht die Antwort nach einer
/// Cloudflare-Challenge aus, wird die Seite **einmalig** über den TLS-Impersonate-
/// Client (Chrome-Fingerprint) nachgeholt. Ist das Flag aus, Verhalten wie bisher.
pub async fn fetch_text(client: &reqwest::Client, url: &str) -> crate::error::Result<String> {
    let resp = fetch(client, url).await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if crate::http::should_impersonate(crate::http::impersonate_enabled(), status, &body) {
        return crate::http::get_text_impersonate(url, None).await;
    }
    Ok(body)
}

/// Liest Open-Graph-Meta (`og:image`, `og:description`) einer Portalseite — für
/// Poster und Kurzbeschreibung in der Titel-Detailansicht. Fehlt etwas, `None`.
pub async fn og_meta(
    client: &reqwest::Client,
    url: &str,
) -> crate::error::Result<(Option<String>, Option<String>)> {
    let body = fetch_text(client, url).await?;
    Ok(parse_og(&body))
}

/// Reine HTML→Open-Graph-Logik von [`og_meta`] (ohne Netz, testbar).
fn parse_og(body: &str) -> (Option<String>, Option<String>) {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(body);
    let pick = |prop: &str| -> Option<String> {
        let sel = Selector::parse(&format!(r#"meta[property="{prop}"]"#)).ok()?;
        doc.select(&sel)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    (pick("og:image"), pick("og:description"))
}

/// Normalisierter Host einer URL (ohne Trailing-Dot, lowercase). `None`, wenn die
/// URL nicht parst oder keinen Host hat.
fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok().and_then(|u| {
        u.host_str()
            .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
    })
}

/// Wählt die passende Basis-URL anhand des geparsten Hosts einer Episoden-/Serien-URL.
///
/// Routing über den echten Host, nicht über Substring-Suche im Rohstring — sonst
/// würde z. B. `aniworld.to/x?q=serienstream.to` falsch einsortiert.
pub fn base_for_url(url: &str) -> &'static str {
    match host_of(url).as_deref() {
        Some("s.to" | "www.s.to" | "serienstream.to" | "www.serienstream.to") => BASE_STO,
        _ => BASE,
    }
}

/// True, wenn die URL auf filmo.to zeigt (eigener Resolver-Pfad).
pub fn is_filmo(url: &str) -> bool {
    matches!(host_of(url).as_deref(), Some("filmo.to" | "www.filmo.to"))
}

/// Erlaubte Quell-Hosts. Alles andere wird abgewiesen, damit die serverseitig
/// abgerufene URL nicht auf interne/beliebige Ziele zeigen kann (SSRF-Schutz).
pub const ALLOWED_HOSTS: &[&str] = &[
    "aniworld.to",
    "www.aniworld.to",
    "s.to",
    "www.s.to",
    "serienstream.to",
    "www.serienstream.to",
    "filmo.to",
    "www.filmo.to",
];

/// Validiert eine vom Aufrufer gelieferte Quell-URL vor jedem serverseitigen
/// Abruf: nur `https` und nur ein Host aus [`ALLOWED_HOSTS`].
///
/// Schützt gegen SSRF, wenn die URL aus einer nicht vertrauenswürdigen Quelle
/// stammt (API-Request). Die anschließende Hoster-Auflösung folgt bewusst
/// Redirects zu den Hoster-Domains — diese Ziele stammen aus der geparsten Seite,
/// nicht aus Nutzereingaben.
pub fn validate_source_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("ungültige URL: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("nur https erlaubt".to_string());
    }
    let host = parsed
        .host_str()
        .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
        .ok_or_else(|| "URL ohne Host".to_string())?;
    if !ALLOWED_HOSTS.contains(&host.as_str()) {
        return Err(format!("Host nicht erlaubt: {host}"));
    }
    Ok(())
}

/// Extrahiert den Serien-Slug aus einer aniworld-/serienstream.to-URL
/// (`…/anime/stream/<slug>/…` bzw. `…/serie/stream/<slug>/…`).
pub fn slug_from_url(url: &str) -> Option<String> {
    // Reihenfolge wichtig: die spezifischeren `…/stream/`-Marker zuerst, damit ein
    // `/serie/stream/<slug>` nicht fälschlich den Slug „stream" ergibt. Das echte
    // serienstream.to nutzt `/serie/<slug>` (ohne `/stream/`), aniworld `/anime/stream/`.
    for marker in ["/anime/stream/", "/serie/stream/", "/serie/"] {
        if let Some(rest) = url.split(marker).nth(1) {
            let slug = rest.split('/').next().unwrap_or("").trim();
            if !slug.is_empty() {
                return Some(slug.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn og_meta_liest_image_und_description() {
        let html = r#"<html><head>
            <meta property="og:title" content="X">
            <meta property="og:image" content="https://cdn.example/poster.jpg">
            <meta property="og:description" content="  Eine Beschreibung.  ">
            </head><body></body></html>"#;
        let (img, desc) = parse_og(html);
        assert_eq!(img.as_deref(), Some("https://cdn.example/poster.jpg"));
        assert_eq!(desc.as_deref(), Some("Eine Beschreibung."));
    }

    #[test]
    fn og_meta_fehlend_ist_none() {
        let (img, desc) = parse_og("<html><head></head><body>x</body></html>");
        assert!(img.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn slug_aus_allen_url_formen() {
        // aniworld: /anime/stream/<slug>
        assert_eq!(
            slug_from_url("https://aniworld.to/anime/stream/one-piece/staffel-1/episode-1")
                .as_deref(),
            Some("one-piece")
        );
        // serienstream REAL: /serie/<slug> (ohne /stream/)
        assert_eq!(
            slug_from_url("https://serienstream.to/serie/black-clover/staffel-2/episode-8")
                .as_deref(),
            Some("black-clover")
        );
        // altes /serie/stream/<slug> ergibt weiter den echten Slug (nicht „stream")
        assert_eq!(
            slug_from_url("https://serienstream.to/serie/stream/dark/staffel-1").as_deref(),
            Some("dark")
        );
        // Serien-Übersicht ohne Pfad-Tiefe
        assert_eq!(
            slug_from_url("https://serienstream.to/serie/arrow").as_deref(),
            Some("arrow")
        );
        assert_eq!(slug_from_url("https://serienstream.to/"), None);
    }

    #[test]
    fn allows_supported_hosts() {
        assert!(
            validate_source_url("https://aniworld.to/anime/stream/x/staffel-1/episode-1").is_ok()
        );
        assert!(validate_source_url("https://serienstream.to/serie/stream/y").is_ok());
        assert!(validate_source_url("https://s.to/serie/stream/y").is_ok());
    }

    #[test]
    fn rejects_ssrf_targets() {
        assert!(validate_source_url("http://aniworld.to/x").is_err()); // kein https
        assert!(validate_source_url("https://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_source_url("https://localhost/admin").is_err());
        assert!(validate_source_url("https://evil.example/aniworld.to").is_err());
        assert!(validate_source_url("https://aniworld.to.evil.example/x").is_err());
        assert!(validate_source_url("file:///etc/passwd").is_err());
        assert!(validate_source_url("not a url").is_err());
    }

    #[test]
    fn routing_uses_host_not_substring() {
        assert_eq!(
            base_for_url("https://serienstream.to/serie/stream/x"),
            BASE_STO
        );
        assert_eq!(base_for_url("https://s.to/serie/stream/x"), BASE_STO);
        assert_eq!(base_for_url("https://aniworld.to/anime/stream/x"), BASE);
        // Query/Pfad mit fremdem Portalnamen darf die Basis nicht umlenken.
        assert_eq!(
            base_for_url("https://aniworld.to/anime/stream/x?ref=serienstream.to"),
            BASE
        );
        assert!(is_filmo("https://filmo.to/movies/x"));
        assert!(!is_filmo("https://aniworld.to/anime/stream/x?q=filmo.to"));
    }
}
