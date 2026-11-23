//! Desktop-Backend für vindex.
//!
//! Enthält die **plattformunabhängige Facade**, die eine `tauri`-App über
//! `#[tauri::command]`-Wrapper (bzw. `egui`/`dioxus`) aufrufen würde. Das
//! eigentliche Fenster-/Webview-Frontend (`tauri`) ist bewusst noch nicht Teil
//! des Workspaces, weil es System-Bibliotheken (WebKitGTK u. a.) zum Linken
//! braucht; die Facade hier ist jedoch voll funktionsfähig und headless testbar
//! (siehe `ARCHITECTURE.md` §9, Schritt 12).
//!
//! Die Facade bleibt bewusst dünn: sie mappt nur sauber auf `vindex_core` und
//! enthält keine eigene Netz-/Download-Logik. Die reinen Mapper (`prefs_from_settings`,
//! `validate_url`, `hoster_by_name`) sind ohne Netz getestet.

pub use vindex_core as core;

use std::path::PathBuf;

use core::config::Settings;
use core::download::DownloadPrefs;
pub use core::download::{DownloadControl, PauseGate, Progress, ProgressSink, RateLimiter};
use core::error::Result;
use core::hoster::Resolved;
use core::model::{Hoster, Language, ProviderLinks, Quality};
use core::{http, site};

pub use tokio_util::sync::CancellationToken;

/// Ein Suchtreffer fürs Frontend.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
}

/// Ein AniSkip-Intervall (Intro/Outro) in Sekunden fürs Frontend.
#[derive(Debug, Clone)]
pub struct Skip {
    pub start: f64,
    pub end: f64,
    /// "op" (Intro) oder "ed" (Outro).
    pub kind: String,
}

/// Optionale Overrides, die beim Bau eines [`DownloadPrefs`] aus [`Settings`]
/// Vorrang vor den gespeicherten Einstellungen haben. Ein `None`-Feld erbt den
/// jeweiligen Wert aus den `Settings`.
#[derive(Debug, Clone, Default)]
pub struct DownloadOverrides {
    /// Zielsprache (statt `settings.language`).
    pub language: Option<Language>,
    /// Hoster-Reihenfolge (statt `settings.effective_hosters()`).
    pub hosters: Option<Vec<Hoster>>,
    /// Höchste akzeptierte Qualität.
    pub max_quality: Option<Quality>,
    /// Niedrigste akzeptierte Qualität.
    pub min_quality: Option<Quality>,
    /// Zielverzeichnis.
    pub out_dir: Option<PathBuf>,
    /// HLS-Segmente selbst holen.
    pub native_hls: Option<bool>,
    /// Ohne ffmpeg muxen (nativer TS→MKV-Remux, sonst roher `.ts`).
    pub no_ffmpeg: Option<bool>,
}

/// Bildet [`Settings`] (plus optionale [`DownloadOverrides`]) auf [`DownloadPrefs`]
/// ab. Reiner Mapper ohne Netz — Overrides gewinnen, sonst gelten die Settings.
pub fn prefs_from_settings(
    settings: &Settings,
    file_name: impl Into<String>,
    overrides: &DownloadOverrides,
) -> DownloadPrefs {
    let language = overrides.language.unwrap_or(settings.language);
    DownloadPrefs {
        languages: vec![language],
        hosters: overrides
            .hosters
            .clone()
            .unwrap_or_else(|| settings.effective_hosters()),
        max_quality: overrides.max_quality.unwrap_or(settings.max_quality),
        min_quality: overrides.min_quality.unwrap_or(settings.min_quality),
        out_dir: overrides
            .out_dir
            .clone()
            .unwrap_or_else(|| settings.download_path.clone()),
        file_name: file_name.into(),
        native_hls: overrides.native_hls.unwrap_or(settings.native_hls),
        no_ffmpeg: overrides.no_ffmpeg.unwrap_or(settings.no_ffmpeg),
    }
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

    /// Katalog eines Portals als Titel-Liste fürs Browsen (Grid).
    ///
    /// aniworld liefert den alphabetischen Katalog direkt. serienstream und filmo
    /// haben portalseitig (noch) keinen durchblätterbaren Katalog im Kern — hier
    /// kommt ein `Err`, das das Frontend als „noch nicht angebunden" zeigt.
    pub async fn catalog(&self, portal: &str) -> Result<Vec<SearchHit>> {
        match portal {
            "aniworld" => {
                let entries = site::aniworld::fetch_catalog(&self.client)
                    .await
                    .map_err(|e| core::error::Error::Parse(e.to_string()))?;
                Ok(entries
                    .into_iter()
                    .map(|e| SearchHit {
                        url: format!("{}/anime/stream/{}", core::model::BASE, e.slug),
                        title: e.title,
                    })
                    .collect())
            }
            "serienstream" => {
                let entries =
                    site::serienstream::fetch_catalog(&self.client, core::model::BASE_STO).await?;
                Ok(entries
                    .into_iter()
                    .map(|e| SearchHit {
                        url: format!("{}/serie/{}", core::model::BASE_STO, e.slug),
                        title: e.title,
                    })
                    .collect())
            }
            "filmo" => {
                let entries = site::filmo::fetch_catalog(&self.client, 1).await?;
                Ok(entries
                    .into_iter()
                    .map(|e| SearchHit {
                        url: format!("{}/movies/{}", site::filmo::BASE, e.slug),
                        title: e.title,
                    })
                    .collect())
            }
            other => Err(core::error::Error::Parse(format!(
                "unbekanntes Portal: {other}"
            ))),
        }
    }

    /// Staffeln eines Serien-/Anime-Titels (0 = Filme-Tab). aniworld und
    /// serienstream nutzen dieselbe Staffel-Navigation; filmo hat keine Staffeln.
    pub async fn seasons(&self, url: &str) -> Result<Vec<u32>> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        if site::is_filmo(url) {
            return Err(core::error::Error::Parse(
                "filmo ist ein Film, keine Serie mit Staffeln".into(),
            ));
        }
        let base = site::base_for_url(url);
        let slug = site::slug_from_url(url)
            .ok_or_else(|| core::error::Error::Parse("keine Titel-Slug in der URL".into()))?;
        site::serienstream::list_seasons(&self.client, base, &slug).await
    }

    /// Episoden einer Staffel (lazy pro Staffel geladen).
    pub async fn episodes(&self, url: &str, season: u32) -> Result<Vec<core::model::Episode>> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        if site::is_filmo(url) {
            return Err(core::error::Error::Parse(
                "filmo ist ein Film, keine Serie mit Episoden".into(),
            ));
        }
        let base = site::base_for_url(url);
        let slug = site::slug_from_url(url)
            .ok_or_else(|| core::error::Error::Parse("keine Titel-Slug in der URL".into()))?;
        site::serienstream::list_episodes(&self.client, base, &slug, season).await
    }

    /// Poster und Kurzbeschreibung eines Titels (Open-Graph-Meta der Detailseite).
    pub async fn title_meta(&self, url: &str) -> Result<(Option<String>, Option<String>)> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        site::og_meta(&self.client, url).await
    }

    /// Nur das Poster-Cover eines Titels (für das Grid, lazy pro Karte).
    pub async fn cover(&self, url: &str) -> Result<Option<String>> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        Ok(site::og_meta(&self.client, url).await?.0)
    }

    /// AniSkip-Intervalle (Intro/Outro) für eine Episode. Braucht die MAL-ID
    /// (über Jikan aufgelöst) und die Episodennummer der Staffel. Leere Liste,
    /// wenn nichts gefunden wird (oder Jikan nicht erreichbar ist).
    pub async fn aniskip(&self, url: &str, season: u32, episode: u32) -> Result<Vec<Skip>> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        let Some(slug) = site::slug_from_url(url) else {
            return Ok(vec![]);
        };
        let Some(mal_id) = core::aniskip::resolve_mal_id(&self.client, &slug, season).await? else {
            return Ok(vec![]);
        };
        let intervals = core::aniskip::skip_times(&self.client, mal_id, episode).await?;
        Ok(intervals
            .into_iter()
            .map(|i| Skip {
                start: i.start,
                end: i.end,
                kind: match i.skip_type {
                    core::aniskip::SkipType::Op => "op",
                    core::aniskip::SkipType::Ed => "ed",
                }
                .into(),
            })
            .collect())
    }

    /// Hoster-Links einer Episode (für die Download-Auswahl im Frontend).
    pub async fn providers(&self, episode_url: &str) -> Result<ProviderLinks> {
        // SSRF-Schutz: nur die unterstützten Portale als Quell-URL zulassen.
        site::validate_source_url(episode_url).map_err(core::error::Error::Parse)?;
        let base = site::base_for_url(episode_url);
        site::serienstream::provider_links(&self.client, base, episode_url).await
    }

    /// Lädt die effektiven Laufzeit-Einstellungen (`VINDEX_*`-ENV + `.env`).
    /// Darüber sind Download-Pfad, Sprache sowie effektive Hoster/Qualität
    /// (`Settings::effective_hosters`) fürs Frontend zugänglich. Die Daemon-
    /// API-Keys werden entfernt — ein Frontend braucht sie nicht und soll sie
    /// nicht zu sehen bekommen.
    pub fn settings(&self) -> Settings {
        let mut s = Settings::load();
        s.api_keys.clear();
        s
    }

    /// Löst einen Hoster + Embed-URL zur direkten Stream-URL auf
    /// (Durchreichung auf `core::hoster::resolve`).
    pub async fn resolve_stream(&self, hoster: Hoster, embed_url: &str) -> Result<Resolved> {
        core::hoster::resolve(&self.client, hoster, embed_url).await
    }

    /// Löst eine Episoden-/Film-URL zum ersten spielbaren Stream auf — exakt der
    /// erprobte Pfad des Downloaders/`watch` (`resolve_playable`): inklusive
    /// Autocorrect und filmo-Film-Flow. Ergebnis geht durch den Referer-Proxy
    /// des Frontends an den Player.
    pub async fn resolve_playback(&self, url: &str, prefs: &DownloadPrefs) -> Result<Resolved> {
        site::validate_source_url(url).map_err(core::error::Error::Parse)?;
        core::download::resolve_playable(&self.client, url, prefs).await
    }

    /// Lädt eine Episode/einen Film anhand fertiger [`DownloadPrefs`] herunter
    /// (Durchreichung auf `core::download::download_url`). Abbruch optional über
    /// einen [`CancellationToken`].
    pub async fn download(
        &self,
        url: &str,
        prefs: &DownloadPrefs,
        cancel: Option<CancellationToken>,
    ) -> Result<PathBuf> {
        core::download::download_url(&self.client, url, prefs, cancel, None).await
    }

    /// Wie [`Backend::download`], meldet den Fortschritt aber an eine Senke — das
    /// GUI-Frontend hängt hier seinen Progress-Kanal ein (z. B. einen
    /// `std::sync::mpsc::Sender`, in den Closure verpackt).
    pub async fn download_with_progress(
        &self,
        url: &str,
        prefs: &DownloadPrefs,
        cancel: Option<CancellationToken>,
        progress: Option<&ProgressSink>,
    ) -> Result<PathBuf> {
        core::download::download_url(&self.client, url, prefs, cancel, progress).await
    }

    /// Wie [`Backend::download_with_progress`], aber mit voller Live-Steuerung
    /// (Abbruch, Pause, Bandbreite) über ein [`DownloadControl`].
    pub async fn download_controlled(
        &self,
        url: &str,
        prefs: &DownloadPrefs,
        ctl: DownloadControl,
        progress: Option<&ProgressSink>,
    ) -> Result<PathBuf> {
        core::download::download_url_ctl(&self.client, url, prefs, ctl, progress).await
    }

    /// Bequemlichkeits-Variante: baut aus [`Settings`] + [`DownloadOverrides`]
    /// die [`DownloadPrefs`] und startet den Download.
    pub async fn download_with_settings(
        &self,
        url: &str,
        file_name: impl Into<String>,
        settings: &Settings,
        overrides: &DownloadOverrides,
        cancel: Option<CancellationToken>,
    ) -> Result<PathBuf> {
        let prefs = prefs_from_settings(settings, file_name, overrides);
        self.download(url, &prefs, cancel).await
    }

    /// Prüft, ob eine Einstiegs-URL ein unterstütztes Portal ist (SSRF-Schutz).
    /// Durchreichung auf `core::site::validate_source_url`.
    pub fn validate_url(url: &str) -> std::result::Result<(), String> {
        site::validate_source_url(url)
    }

    /// Löst einen Hoster-Namen (z. B. `"voe"`) zum [`Hoster`]-Enum auf.
    /// Durchreichung auf `Hoster::from_str`.
    pub fn hoster_by_name(name: &str) -> Option<Hoster> {
        name.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ohne Overrides müssen die `DownloadPrefs` exakt den `Settings` folgen.
    #[test]
    fn prefs_erben_aus_settings() {
        let settings = Settings::default();
        let prefs =
            prefs_from_settings(&settings, "Show S01E01.mkv", &DownloadOverrides::default());

        assert_eq!(prefs.languages, vec![settings.language]);
        assert_eq!(prefs.hosters, settings.effective_hosters());
        assert_eq!(prefs.max_quality, settings.max_quality);
        assert_eq!(prefs.min_quality, settings.min_quality);
        assert_eq!(prefs.out_dir, settings.download_path);
        assert_eq!(prefs.file_name, "Show S01E01.mkv");
        assert_eq!(prefs.native_hls, settings.native_hls);
    }

    /// Gesetzte Overrides müssen die Settings-Werte überschreiben.
    #[test]
    fn overrides_gewinnen_vor_settings() {
        let settings = Settings::default();
        let overrides = DownloadOverrides {
            language: Some(Language::EnglishDub),
            hosters: Some(vec![Hoster::Vidoza]),
            max_quality: Some(Quality::P720),
            min_quality: Some(Quality::P480),
            out_dir: Some(PathBuf::from("/tmp/vindex-test")),
            native_hls: Some(!settings.native_hls),
            no_ffmpeg: Some(!settings.no_ffmpeg),
        };
        let prefs = prefs_from_settings(&settings, "x.mkv", &overrides);

        assert_eq!(prefs.languages, vec![Language::EnglishDub]);
        assert_eq!(prefs.hosters, vec![Hoster::Vidoza]);
        assert_eq!(prefs.max_quality, Quality::P720);
        assert_eq!(prefs.min_quality, Quality::P480);
        assert_eq!(prefs.out_dir, PathBuf::from("/tmp/vindex-test"));
        assert_eq!(prefs.native_hls, !settings.native_hls);
        assert_eq!(prefs.no_ffmpeg, !settings.no_ffmpeg);
    }

    /// URL-Validierung über die Facade: unterstützte Hosts ja, fremde nein.
    #[test]
    fn validate_url_filtert_fremde_hosts() {
        assert!(
            Backend::validate_url("https://aniworld.to/anime/stream/x/staffel-1/episode-1").is_ok()
        );
        assert!(Backend::validate_url("https://example.com/evil").is_err());
        assert!(Backend::validate_url("http://aniworld.to/x").is_err());
    }

    /// Hoster-Namensauflösung über die Facade.
    #[test]
    fn hoster_namensaufloesung() {
        assert_eq!(Backend::hoster_by_name("voe"), Some(Hoster::Voe));
        assert_eq!(Backend::hoster_by_name("gibtsnicht"), None);
    }
}
