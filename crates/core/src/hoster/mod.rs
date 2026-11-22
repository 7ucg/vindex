//! Hoster-Resolver: Embed-/Redirect-URL → direkter Stream.
//!
//! Faithfully portiert aus den Referenzen (`downloaders/*.py`,
//! `extractors/provider/*.py`). Streamtape/Luluvdo/LoadX haben keine Vorlage und
//! melden daher `NoStream`/Extract-Fehler (siehe `ARCHITECTURE.md` §1).

pub mod common;
pub mod doodstream;
pub mod filemoon;
pub mod speedfiles;
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
    match hoster {
        Hoster::Voe => voe::resolve(client, embed_url).await,
        Hoster::Vidoza => vidoza::resolve(client, embed_url).await,
        Hoster::Doodstream => doodstream::resolve(client, embed_url).await,
        Hoster::Vidmoly => vidmoly::resolve(client, embed_url).await,
        Hoster::Filemoon => filemoon::resolve(client, embed_url).await,
        Hoster::SpeedFiles => speedfiles::resolve(client, embed_url).await,
        Hoster::Streamtape | Hoster::Luluvdo | Hoster::LoadX => Err(Error::extract(
            hoster.as_str(),
            "kein Resolver — Scope-Vorgabe ohne Referenz (eigenes Reverse Engineering nötig)",
        )),
    }
}
