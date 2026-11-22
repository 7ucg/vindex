//! Desktop-Backend für vindex.
//!
//! Enthält die **plattformunabhängige Facade**, die eine `tauri`-App über
//! `#[tauri::command]`-Wrapper (bzw. `egui`/`dioxus`) aufrufen würde. Das
//! eigentliche Fenster-/Webview-Frontend (`tauri`) ist bewusst noch nicht Teil
//! des Workspaces, weil es System-Bibliotheken (WebKitGTK u. a.) zum Linken
//! braucht; die Facade hier ist jedoch voll funktionsfähig und headless testbar
//! (siehe `ARCHITECTURE.md` §9, Schritt 12).

pub use vindex_core as core;

use core::error::Result;
use core::model::ProviderLinks;
use core::{http, site};

/// Ein Suchtreffer fürs Frontend.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
}

/// Facade-Objekt, das ein GUI-Frontend hält (ein HTTP-Client).
pub struct Backend {
    client: reqwest::Client,
}

impl Backend {
    pub fn new() -> Result<Self> {
        Ok(Backend {
            client: http::browser_client()?,
        })
    }

    /// Titel-Suche (Teilstring) über den aniworld.to-Katalog.
    pub async fn search(&self, keyword: &str) -> Result<Vec<SearchHit>> {
        let needle = keyword.to_lowercase();
        let entries = site::aniworld::fetch_catalog(&self.client).await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.title.to_lowercase().contains(&needle))
            .take(50)
            .map(|e| SearchHit {
                url: format!("{}/anime/stream/{}", core::model::BASE, e.slug),
                title: e.title,
            })
            .collect())
    }

    /// Hoster-Links einer Episode (für die Download-Auswahl im Frontend).
    pub async fn providers(&self, episode_url: &str) -> Result<ProviderLinks> {
        let base = site::base_for_url(episode_url);
        site::serienstream::provider_links(&self.client, base, episode_url).await
    }
}
