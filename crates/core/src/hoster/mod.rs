//! Hoster-Resolver: Embed-/Redirect-URL → direkter Stream.
//!
//! Faithfully portiert aus den Referenzen (`downloaders/*.py`,
//! `extractors/provider/*.py`). Streamtape/Luluvdo/LoadX wurden ohne Vorlage per
//! eigenem Reverse Engineering ergänzt; LoadX ist gegen Live nicht verifiziert
//! (siehe `loadx.rs` und `ARCHITECTURE.md` §1).

pub mod common;
pub mod doodstream;
pub mod filemoon;
pub mod loadx;
pub mod luluvdo;
pub mod speedfiles;
pub mod streamtape;
pub mod vidmoly;
pub mod vidoza;
pub mod voe;

use crate::error::{Error, Result};
use crate::model::Hoster;

/// Ein aufgelöster Stream, bereit für die Download-Engine.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Direkte Stream-URL (HLS-Master oder MP4).
    pub url: String,
    /// Optionaler `Referer`-Header, den der Hoster für den Abruf verlangt.
    pub referer: Option<String>,
    /// Ob es sich um eine HLS-Playlist (`true`) oder eine direkte Datei handelt.
    pub is_hls: bool,
}

/// Löst eine Embed-/Redirect-URL für *hoster* auf.
///
/// Die übergebene URL darf der Portal-Redirect (`/redirect/N`) sein; der Client
/// folgt dem 302 zur eigentlichen Hoster-Embed-Seite.
pub async fn resolve(
    client: &reqwest::Client,
    hoster: Hoster,
    embed_url: &str,
) -> Result<Resolved> {
    // SSRF-Schutz auch für direkt übergebene Embed-URLs (z. B. über die
    // Desktop-Facade): nur http/https, kein IP-Literal aus einem gesperrten
    // Bereich. Interne Ziele über Hostnamen fängt zusätzlich der DNS-Filter ab.
    if !(embed_url.starts_with("http://") || embed_url.starts_with("https://")) {
        return Err(Error::extract(
            hoster.as_str(),
            "Embed-URL muss http/https sein",
        ));
    }
    if crate::http::host_is_forbidden_ip_literal(embed_url) {
        return Err(Error::extract(
            hoster.as_str(),
            "Embed-URL zeigt auf eine gesperrte Adresse",
        ));
    }
    let resolved = match hoster {
        Hoster::Voe => voe::resolve(client, embed_url).await,
        Hoster::Vidoza => vidoza::resolve(client, embed_url).await,
        Hoster::Doodstream => doodstream::resolve(client, embed_url).await,
        Hoster::Vidmoly => vidmoly::resolve(client, embed_url).await,
        Hoster::Filemoon => filemoon::resolve(client, embed_url).await,
        Hoster::SpeedFiles => speedfiles::resolve(client, embed_url).await,
        Hoster::Streamtape => streamtape::resolve(client, embed_url).await,
        Hoster::Luluvdo => luluvdo::resolve(client, embed_url).await,
        Hoster::LoadX => loadx::resolve(client, embed_url).await,
    }?;

    // Die aufgelöste Stream-URL stammt aus gescraptem Hoster-HTML — ebenfalls gegen
    // Nicht-http/https und gesperrte IP-Literale prüfen, bevor sie weitergegeben wird.
    if !(resolved.url.starts_with("http://") || resolved.url.starts_with("https://")) {
        return Err(Error::extract(
            hoster.as_str(),
            "aufgelöste Stream-URL muss http/https sein",
        ));
    }
    if crate::http::host_is_forbidden_ip_literal(&resolved.url) {
        return Err(Error::extract(
            hoster.as_str(),
            "aufgelöste Stream-URL zeigt auf eine gesperrte Adresse",
        ));
    }
    Ok(resolved)
}
