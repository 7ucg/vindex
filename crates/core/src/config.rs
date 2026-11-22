//! Konfiguration (`VINDEX_*`-ENV + `.env`).
//!
//! Eine Teilmenge der Referenz-Optionen (`.env.example`). `.env` wird via
//! `dotenvy` geladen, Werte danach aus der Umgebung gelesen. Für geschichtete
//! Datei-Config (TOML/JSON + ENV) steht `config` als Abhängigkeit bereit.

use std::path::PathBuf;
use std::str::FromStr;

use crate::model::{default_fallback_order, Hoster, Language, Quality};

/// Laufzeit-Einstellungen.
#[derive(Debug, Clone)]
pub struct Settings {
    pub download_path: PathBuf,
    pub language: Language,
    pub provider: Hoster,
    pub fallback_order: Vec<Hoster>,
    pub max_quality: Quality,
    pub min_quality: Quality,
    pub simultaneous_downloads: usize,
    pub download_delay_secs: u64,
    pub retries: u32,
    pub aniskip: bool,
    pub keep_watching: bool,
    pub naming_template: String,
    /// Segmente von HLS-Streams selbst holen (robust gegen 429), statt ffmpeg die
    /// Playlist ziehen zu lassen (`VINDEX_NATIVE_HLS`). ffmpeg remuxt weiterhin.
    pub native_hls: bool,
    /// API-Keys für den Daemon (`VINDEX_API_KEYS`, komma-getrennt). Leer = keine
    /// Auth; der Daemon startet dann nur auf localhost.
    pub api_keys: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            download_path: dirs_download(),
            language: Language::GermanDub,
            provider: Hoster::Voe,
            fallback_order: default_fallback_order(),
            max_quality: Quality::P1080,
            min_quality: Quality::P360,
            simultaneous_downloads: 3,
            download_delay_secs: 2,
            retries: 1,
            aniskip: false,
            keep_watching: false,
            naming_template: crate::naming::DEFAULT_TEMPLATE.to_string(),
            native_hls: false,
            api_keys: Vec::new(),
        }
    }
}

impl Settings {
    /// Lädt `.env` (falls vorhanden) und überschreibt Defaults aus `VINDEX_*`.
    pub fn load() -> Self {
        let _ = dotenvy::dotenv();
        let mut s = Settings::default();

        if let Some(v) = env("VINDEX_DOWNLOAD_PATH") {
            s.download_path = PathBuf::from(v);
        }
        if let Some(v) = env("VINDEX_LANGUAGE").and_then(|v| Language::from_str(&v).ok()) {
            s.language = v;
        }
        if let Some(v) = env("VINDEX_PROVIDER").and_then(|v| Hoster::from_str(&v).ok()) {
            s.provider = v;
        }
        if let Some(v) = env("VINDEX_PROVIDER_FALLBACK_ORDER") {
            let order: Vec<Hoster> = v
                .split(',')
                .filter_map(|p| Hoster::from_str(p.trim()).ok())
                .collect();
            if !order.is_empty() {
                s.fallback_order = order;
            }
        }
        if let Some(v) = env("VINDEX_MAX_QUALITY").and_then(|v| Quality::from_str(&v).ok()) {
            s.max_quality = v;
        }
        if let Some(v) = env("VINDEX_MIN_QUALITY").and_then(|v| Quality::from_str(&v).ok()) {
            s.min_quality = v;
        }
        if let Some(v) = env_parse::<usize>("VINDEX_SIMULTANEOUS_DOWNLOADS") {
            s.simultaneous_downloads = v.clamp(1, 20);
        }
        if let Some(v) = env_parse::<u64>("VINDEX_DOWNLOAD_DELAY") {
            s.download_delay_secs = v.min(300);
        }
        if let Some(v) = env_parse::<u32>("VINDEX_RETRIES") {
            s.retries = v.min(5);
        }
        s.aniskip = env_bool("VINDEX_ANISKIP", s.aniskip);
        s.keep_watching = env_bool("VINDEX_KEEP_WATCHING", s.keep_watching);
        s.native_hls = env_bool("VINDEX_NATIVE_HLS", s.native_hls);
        if let Some(v) = env("VINDEX_NAMING_TEMPLATE") {
            s.naming_template = v;
        }
        if let Some(v) = env("VINDEX_API_KEYS") {
            s.api_keys = v
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();
        }
        s
    }

    /// Effektive Hoster-Reihenfolge: gewählter Provider zuerst, dann Fallbacks.
    pub fn effective_hosters(&self) -> Vec<Hoster> {
        let mut out = vec![self.provider];
        for h in &self.fallback_order {
            if *h != self.provider {
                out.push(*h);
            }
        }
        out
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn env_parse<T: FromStr>(key: &str) -> Option<T> {
    env(key).and_then(|v| v.parse::<T>().ok())
}

fn env_bool(key: &str, default: bool) -> bool {
    match env(key).as_deref() {
        Some("1") | Some("true") | Some("on") => true,
        Some("0") | Some("false") | Some("off") => false,
        _ => default,
    }
}

fn dirs_download() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}
