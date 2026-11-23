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

    /// Kurzer, `FromStr`-kompatibler Name für UI/Config (round-trips über `from_str`).
    pub fn ui_label(self) -> &'static str {
        match self {
            Language::GermanDub => "German Dub",
            Language::EnglishDub => "English Dub",
            Language::GermanSub => "German Sub",
            Language::EnglishSub => "English Sub",
        }
    }

    /// True, wenn die Variante nur Untertitel bietet (kein synchronisierter Ton).
    pub fn is_subtitle_only(self) -> bool {
        matches!(self, Language::GermanSub | Language::EnglishSub)
    }

    /// Feste Reihenfolge aller Varianten (für Fallback/Anzeige).
    pub const ALL: [Language; 4] = [
        Language::GermanDub,
        Language::GermanSub,
        Language::EnglishSub,
        Language::EnglishDub,
    ];
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
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprache_aniworld_key_roundtrip() {
        for key in ["1", "2", "3"] {
            let lang = Language::from_aniworld_key(key);
            assert_eq!(lang.and_then(Language::aniworld_key), Some(key));
        }
    }

    #[test]
    fn sprache_aniworld_key_zuordnung() {
        assert_eq!(Language::from_aniworld_key("1"), Some(Language::GermanDub));
        assert_eq!(Language::from_aniworld_key("2"), Some(Language::EnglishSub));
        assert_eq!(Language::from_aniworld_key("3"), Some(Language::GermanSub));
    }

    #[test]
    fn sprache_aniworld_key_trimmt_whitespace() {
        assert_eq!(
            Language::from_aniworld_key(" 1\n"),
            Some(Language::GermanDub)
        );
    }

    #[test]
    fn sprache_aniworld_key_unbekannt() {
        assert_eq!(Language::from_aniworld_key(""), None);
        assert_eq!(Language::from_aniworld_key("0"), None);
        assert_eq!(Language::from_aniworld_key("4"), None);
        assert_eq!(Language::from_aniworld_key("abc"), None);
    }

    #[test]
    fn sprache_englisch_dub_hat_keinen_aniworld_key() {
        assert_eq!(Language::EnglishDub.aniworld_key(), None);
    }

    #[test]
    fn sprache_from_str_aliase_und_case() {
        assert_eq!("German Dub".parse::<Language>(), Ok(Language::GermanDub));
        assert_eq!("ger-dub".parse::<Language>(), Ok(Language::GermanDub));
        assert_eq!("  ENG-DUB ".parse::<Language>(), Ok(Language::EnglishDub));
        assert_eq!("gersub".parse::<Language>(), Ok(Language::GermanSub));
        assert_eq!("english-sub".parse::<Language>(), Ok(Language::EnglishSub));
    }

    #[test]
    fn sprache_from_str_unbekannt_ist_fehler() {
        assert_eq!(
            "klingonisch".parse::<Language>(),
            Err("unbekannte Sprache: klingonisch".to_string())
        );
    }

    #[test]
    fn sprache_as_str_anzeigenamen() {
        assert_eq!(Language::GermanDub.as_str(), "German Dub");
        assert_eq!(Language::EnglishDub.as_str(), "English Dub");
        assert_eq!(Language::GermanSub.as_str(), "Japanese · German Sub");
        assert_eq!(Language::EnglishSub.as_str(), "Japanese · English Sub");
    }

    #[test]
    fn qualitaet_from_height_grenzwerte() {
        assert_eq!(Quality::from_height(0), Quality::P360);
        assert_eq!(Quality::from_height(419), Quality::P360);
        assert_eq!(Quality::from_height(420), Quality::P480);
        assert_eq!(Quality::from_height(599), Quality::P480);
        assert_eq!(Quality::from_height(600), Quality::P720);
        assert_eq!(Quality::from_height(899), Quality::P720);
        assert_eq!(Quality::from_height(900), Quality::P1080);
        assert_eq!(Quality::from_height(1439), Quality::P1080);
        assert_eq!(Quality::from_height(1440), Quality::K4);
        assert_eq!(Quality::from_height(u32::MAX), Quality::K4);
    }

    #[test]
    fn qualitaet_typische_aufloesungen() {
        assert_eq!(Quality::from_height(360), Quality::P360);
        assert_eq!(Quality::from_height(480), Quality::P480);
        assert_eq!(Quality::from_height(720), Quality::P720);
        assert_eq!(Quality::from_height(1080), Quality::P1080);
        assert_eq!(Quality::from_height(2160), Quality::K4);
    }

    #[test]
    fn qualitaet_ordnung_aufsteigend() {
        assert!(Quality::P360 < Quality::P480);
        assert!(Quality::P480 < Quality::P720);
        assert!(Quality::P720 < Quality::P1080);
        assert!(Quality::P1080 < Quality::K4);
    }

    #[test]
    fn qualitaet_as_str_und_from_str_roundtrip() {
        for q in [
            Quality::P360,
            Quality::P480,
            Quality::P720,
            Quality::P1080,
            Quality::K4,
        ] {
            assert_eq!(q.as_str().parse::<Quality>(), Ok(q));
        }
    }

    #[test]
    fn qualitaet_from_str_aliase() {
        assert_eq!("720".parse::<Quality>(), Ok(Quality::P720));
        assert_eq!(" 4K ".parse::<Quality>(), Ok(Quality::K4));
        assert_eq!("2160p".parse::<Quality>(), Ok(Quality::K4));
        assert_eq!("2160".parse::<Quality>(), Ok(Quality::K4));
    }

    #[test]
    fn qualitaet_from_str_unbekannt_ist_fehler() {
        assert_eq!(
            "144p".parse::<Quality>(),
            Err("unbekannte Qualität: 144p".to_string())
        );
    }

    const ALLE_HOSTER: [Hoster; 6] = [
        Hoster::Voe,
        Hoster::Vidoza,
        Hoster::Doodstream,
        Hoster::Filemoon,
        Hoster::Vidmoly,
        Hoster::SpeedFiles,
    ];

    #[test]
    fn hoster_as_str_from_site_name_roundtrip() {
        for h in ALLE_HOSTER {
            assert_eq!(Hoster::from_site_name(h.as_str()), Some(h));
        }
    }

    #[test]
    fn hoster_from_site_name_ignoriert_case_und_whitespace() {
        assert_eq!(Hoster::from_site_name("  vOe "), Some(Hoster::Voe));
        assert_eq!(
            Hoster::from_site_name("SPEEDFILES"),
            Some(Hoster::SpeedFiles)
        );
    }

    #[test]
    fn hoster_from_site_name_unbekannt() {
        assert_eq!(Hoster::from_site_name(""), None);
        assert_eq!(Hoster::from_site_name("voe.sx"), None);
        assert_eq!(Hoster::from_site_name("irgendwas"), None);
    }

    #[test]
    fn hoster_from_str_fehler_nennt_eingabe() {
        assert_eq!("VOE".parse::<Hoster>(), Ok(Hoster::Voe));
        assert_eq!(
            "nope".parse::<Hoster>(),
            Err("unbekannter Hoster: nope".to_string())
        );
    }

    #[test]
    fn hoster_from_host_or_name_anzeigename_hat_vorrang() {
        assert_eq!(
            Hoster::from_host_or_name("Filemoon"),
            Some(Hoster::Filemoon)
        );
    }

    #[test]
    fn hoster_from_host_or_name_hosts() {
        assert_eq!(Hoster::from_host_or_name("voe.sx"), Some(Hoster::Voe));
        assert_eq!(
            Hoster::from_host_or_name("videzz.net"),
            Some(Hoster::Vidoza)
        );
        assert_eq!(
            Hoster::from_host_or_name("dood.wf"),
            Some(Hoster::Doodstream)
        );
        assert_eq!(
            Hoster::from_host_or_name("speedfiles.net"),
            Some(Hoster::SpeedFiles)
        );
    }

    #[test]
    fn hoster_from_host_or_name_unbekannt() {
        assert_eq!(Hoster::from_host_or_name(""), None);
        assert_eq!(Hoster::from_host_or_name("example.com"), None);
    }

    #[test]
    fn fallback_reihenfolge_default() {
        let order = default_fallback_order();
        assert_eq!(order.first(), Some(&Hoster::Voe));
        assert_eq!(order.len(), 6);
    }

    #[test]
    fn fallback_reihenfolge_ohne_duplikate() {
        let order = default_fallback_order();
        let unique: std::collections::HashSet<_> = order.iter().collect();
        assert_eq!(unique.len(), order.len());
    }

    #[test]
    fn provider_links_leer_und_abfrage() {
        let mut links = ProviderLinks::default();
        assert!(links.is_empty());
        assert!(links.hosters_for(Language::GermanDub).is_none());
        assert_eq!(links.link(Language::GermanDub, Hoster::Voe), None);

        links
            .by_language
            .entry(Language::GermanDub)
            .or_default()
            .insert(Hoster::Voe, "https://x/redirect/1".to_string());
        assert!(!links.is_empty());
        assert_eq!(
            links.link(Language::GermanDub, Hoster::Voe),
            Some("https://x/redirect/1")
        );
        assert_eq!(links.link(Language::GermanDub, Hoster::Vidoza), None);
        assert_eq!(links.link(Language::GermanSub, Hoster::Voe), None);
    }
}
