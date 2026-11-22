//! Gemeinsame Datentypen.

use std::collections::HashMap;
use std::str::FromStr;

use serde::Serialize;

/// Basis-URL des Portals aniworld.to.
pub const BASE: &str = "https://aniworld.to";
/// Basis-URL des Schwester-Portals serienstream.to (SerienStream).
pub const BASE_STO: &str = "https://serienstream.to";

/// Default-User-Agent des census-Unterbefehls.
pub const USER_AGENT: &str = "vindex/0.1";

// ---------------------------------------------------------------------------
// Katalog / Klassifizierung (Census)
// ---------------------------------------------------------------------------

/// Ein Eintrag aus dem Gesamtkatalog (`/animes-alphabet`).
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub title: String,
    pub slug: String,
}

/// Grobe Einordnung eines Titels.
#[derive(Debug, Clone, Serialize)]
pub enum Kind {
    Movie,
    Series,
    Unknown,
}

/// Ergebnis der Detail-Klassifizierung eines Titels.
#[derive(Debug, Clone, Serialize)]
pub struct Classified {
    pub title: String,
    pub slug: String,
    pub seasons: usize,
    pub episodes_season1: usize,
    /// Anzahl Filme im „Filme"-Tab der Detailseite (0 wenn kein Tab vorhanden).
    pub films: usize,
    /// Alle Folgen über alle Staffeln (nur mit Episoden-Zählung gefüllt, sonst 0).
    pub total_episodes: usize,
    pub kind: Kind,
}

// ---------------------------------------------------------------------------
// Sprache
// ---------------------------------------------------------------------------

/// Sprach-/Untertitel-Variante, wie die Portale sie anbieten.
///
/// aniworld.to kodiert Varianten als `data-lang-key` auf den Hoster-`<li>`:
/// `1` = German Dub, `2` = Japanese · English Sub, `3` = Japanese · German Sub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Language {
    GermanDub,
    EnglishDub,
    GermanSub,
    EnglishSub,
}

impl Language {
    /// aniworld-/serienstream.to-interner `data-lang-key`, falls dort angeboten.
    pub fn aniworld_key(self) -> Option<&'static str> {
        match self {
            Language::GermanDub => Some("1"),
            Language::EnglishSub => Some("2"),
            Language::GermanSub => Some("3"),
            // English Dub existiert auf serienstream.to, hat aber keinen festen aniworld-Key.
            Language::EnglishDub => None,
        }
    }

    /// Variante aus einem `data-lang-key` (aniworld/serienstream.to).
    pub fn from_aniworld_key(key: &str) -> Option<Language> {
        match key.trim() {
            "1" => Some(Language::GermanDub),
            "2" => Some(Language::EnglishSub),
            "3" => Some(Language::GermanSub),
            _ => None,
        }
    }

    /// Anzeigename.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::GermanDub => "German Dub",
            Language::EnglishDub => "English Dub",
            Language::GermanSub => "Japanese · German Sub",
            Language::EnglishSub => "Japanese · English Sub",
        }
    }
}

impl FromStr for Language {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "german dub" | "german-dub" | "ger-dub" | "gerdub" => Ok(Language::GermanDub),
            "english dub" | "english-dub" | "eng-dub" | "engdub" => Ok(Language::EnglishDub),
            "german sub" | "german-sub" | "ger-sub" | "gersub" => Ok(Language::GermanSub),
            "english sub" | "english-sub" | "eng-sub" | "engsub" => Ok(Language::EnglishSub),
            other => Err(format!("unbekannte Sprache: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Qualität
// ---------------------------------------------------------------------------

/// Qualitäts-Tier (Auflösungsklasse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Quality {
    P360,
    P480,
    P720,
    P1080,
    K4,
}

impl Quality {
    /// Tiers hoch → niedrig mit Mindesthöhe (aus `downloaders/common.py`).
    pub const TIERS: [(Quality, u32); 5] = [
        (Quality::K4, 1440),
        (Quality::P1080, 900),
        (Quality::P720, 600),
        (Quality::P480, 420),
        (Quality::P360, 0),
    ];

    /// Ordnet eine Pixel-Höhe einem Tier zu (erster passender Schwellwert).
    pub fn from_height(height: u32) -> Quality {
        for (tier, min_h) in Quality::TIERS {
            if height >= min_h {
                return tier;
            }
        }
        Quality::P360
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Quality::P360 => "360p",
            Quality::P480 => "480p",
            Quality::P720 => "720p",
            Quality::P1080 => "1080p",
            Quality::K4 => "4K",
        }
    }
}

impl FromStr for Quality {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "360p" | "360" => Ok(Quality::P360),
            "480p" | "480" => Ok(Quality::P480),
            "720p" | "720" => Ok(Quality::P720),
            "1080p" | "1080" => Ok(Quality::P1080),
            "4k" | "2160p" | "2160" => Ok(Quality::K4),
            other => Err(format!("unbekannte Qualität: {other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Hoster
// ---------------------------------------------------------------------------

/// Unterstützte Stream-Hoster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Hoster {
    Voe,
    Vidoza,
    Doodstream,
    Filemoon,
    Vidmoly,
    SpeedFiles,
    /// Im Scope, aber ohne Referenz-Implementierung (eigenes Reverse Engineering).
    Streamtape,
    /// Im Scope, aber ohne Referenz-Implementierung.
    Luluvdo,
    /// Im Scope, aber ohne Referenz-Implementierung.
    LoadX,
}

impl Hoster {
    /// Anzeigename, wie er auf der Portal-Seite im `<h4>` steht.
    pub fn as_str(self) -> &'static str {
        match self {
            Hoster::Voe => "VOE",
            Hoster::Vidoza => "Vidoza",
            Hoster::Doodstream => "Doodstream",
            Hoster::Filemoon => "Filemoon",
            Hoster::Vidmoly => "Vidmoly",
            Hoster::SpeedFiles => "SpeedFiles",
            Hoster::Streamtape => "Streamtape",
            Hoster::Luluvdo => "Luluvdo",
            Hoster::LoadX => "LoadX",
        }
    }

    /// Hoster aus dem Portal-Anzeigenamen.
    pub fn from_site_name(name: &str) -> Option<Hoster> {
        match name.trim().to_ascii_lowercase().as_str() {
            "voe" => Some(Hoster::Voe),
            "vidoza" => Some(Hoster::Vidoza),
            "doodstream" => Some(Hoster::Doodstream),
            "filemoon" => Some(Hoster::Filemoon),
            "vidmoly" => Some(Hoster::Vidmoly),
            "speedfiles" => Some(Hoster::SpeedFiles),
            "streamtape" => Some(Hoster::Streamtape),
            "luluvdo" => Some(Hoster::Luluvdo),
            "loadx" => Some(Hoster::LoadX),
            _ => None,
        }
    }

    /// Ob ein Resolver existiert (im Gegensatz zu reinen Scope-Platzhaltern).
    pub fn is_implemented(self) -> bool {
        matches!(
            self,
            Hoster::Voe
                | Hoster::Vidoza
                | Hoster::Doodstream
                | Hoster::Filemoon
                | Hoster::Vidmoly
                | Hoster::SpeedFiles
        )
    }

    /// Hoster aus einem Anzeigenamen **oder** einem Host-/Label-String
    /// (z. B. `voe.sx`, `videzz.net`) — tolerant für Portale, die statt des
    /// Provider-Namens den Host angeben.
    pub fn from_host_or_name(s: &str) -> Option<Hoster> {
        let low = s.trim().to_ascii_lowercase();
        if let Some(h) = Hoster::from_site_name(&low) {
            return Some(h);
        }
        const HOST_HINTS: &[(&str, Hoster)] = &[
            ("voe", Hoster::Voe),
            ("vidoza", Hoster::Vidoza),
            ("videzz", Hoster::Vidoza),
            ("dood", Hoster::Doodstream),
            ("filemoon", Hoster::Filemoon),
            ("vidmoly", Hoster::Vidmoly),
            ("speedfile", Hoster::SpeedFiles),
            ("streamtape", Hoster::Streamtape),
            ("lulu", Hoster::Luluvdo),
            ("loadx", Hoster::LoadX),
        ];
        HOST_HINTS
            .iter()
            .find(|(k, _)| low.contains(k))
            .map(|(_, h)| *h)
    }
}

impl FromStr for Hoster {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Hoster::from_site_name(s).ok_or_else(|| format!("unbekannter Hoster: {s}"))
    }
}

/// Default-Fallback-Reihenfolge (aus `.env.example`: VOE,Vidmoly,Vidoza,Doodstream;
/// hier um Filemoon/SpeedFiles ergänzt).
pub fn default_fallback_order() -> Vec<Hoster> {
    vec![
        Hoster::Voe,
        Hoster::Vidmoly,
        Hoster::Vidoza,
        Hoster::Doodstream,
        Hoster::Filemoon,
        Hoster::SpeedFiles,
    ]
}

// ---------------------------------------------------------------------------
// Serie / Staffel / Episode
// ---------------------------------------------------------------------------

/// Eine Episode (oder ein Film-Eintrag).
#[derive(Debug, Clone)]
pub struct Episode {
    /// Absolute Episoden-URL.
    pub url: String,
    /// Episoden-Titel (ohne `[Episode NNN]`-Marker).
    pub title: String,
    /// Staffel-Nummer (0 = Filme-Tab).
    pub season: u32,
    /// Index innerhalb der Staffel (1-basiert).
    pub index_in_season: u32,
    /// Explizite Episodennummer aus dem EN-Titel, falls vorhanden.
    pub explicit_number: Option<u32>,
}

/// Hoster-Links einer Episode, gruppiert nach Sprache.
#[derive(Debug, Clone, Default)]
pub struct ProviderLinks {
    pub by_language: HashMap<Language, HashMap<Hoster, String>>,
}

impl ProviderLinks {
    pub fn is_empty(&self) -> bool {
        self.by_language.is_empty()
    }

    /// Verfügbare Hoster für eine Sprache.
    pub fn hosters_for(&self, lang: Language) -> Option<&HashMap<Hoster, String>> {
        self.by_language.get(&lang)
    }

    /// Redirect-/Embed-URL für Sprache + Hoster.
    pub fn link(&self, lang: Language, hoster: Hoster) -> Option<&str> {
        self.by_language
            .get(&lang)
            .and_then(|m| m.get(&hoster))
            .map(String::as_str)
    }
}
