//! Persistente Desktop-Einstellungen (vom Nutzer in der UI bearbeitbar).
//!
//! Überschreiben die `VINDEX_*`-Defaults aus dem Kern. Liegen als JSON unter
//! `$XDG_CONFIG_HOME/vindex/desktop.json` (bzw. `$HOME/.config/…`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    /// Zielordner für Downloads (sonst der Kern-Default).
    pub download_path: Option<String>,
    /// Sprache (z. B. „German Dub").
    pub language: Option<String>,
    /// Höchste Qualität (z. B. „1080p").
    pub max_quality: Option<String>,
    /// Niedrigste Qualität.
    pub min_quality: Option<String>,
    /// Globale Bandbreitenbegrenzung in KiB/s (0 oder None = unbegrenzt).
    #[serde(default)]
    pub bandwidth_kib: Option<u64>,
    /// Mindestabstand zwischen Download-Starts in ms (0 oder None = aus).
    #[serde(default)]
    pub download_delay_ms: Option<u64>,
    /// Anti-Bot Stufe 2 (TLS-Impersonate) einschalten. None = Kern-Default.
    #[serde(default)]
    pub impersonate: Option<bool>,
    /// Anti-Bot Stufe 3: Pfad zum obscura-Binary. None/leer = aus.
    #[serde(default)]
    pub obscura: Option<String>,
    /// Egress-Proxy (`VINDEX_PROXY`-Syntax). Wirkt erst nach Neustart. None = keiner.
    #[serde(default)]
    pub proxy: Option<String>,
    /// HLS-Segmente selbst laden (robust gegen 429). None = Standard (an).
    #[serde(default)]
    pub native_hls: Option<bool>,
    /// Ohne ffmpeg muxen (nativer TS→MKV-Remux). None = Standard (an).
    #[serde(default)]
    pub no_ffmpeg: Option<bool>,
    /// Externer Player statt des eingebauten WebView-Players (`"mpv"`).
    /// None/leer = eingebauter Player (Standard).
    #[serde(default)]
    pub external_player: Option<String>,
    /// Anime4K-Modus (Name wie `A`, `B`, `C`, `A+A`, …) für den externen Player.
    /// None = aus.
    #[serde(default)]
    pub anime4k: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("vindex").join("desktop.json"))
}

pub fn load() -> DesktopConfig {
    let Some(p) = config_path() else {
        return DesktopConfig::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &DesktopConfig) -> Result<(), String> {
    let p = config_path().ok_or("kein Konfig-Pfad (HOME fehlt)")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(p, s).map_err(|e| e.to_string())
}
