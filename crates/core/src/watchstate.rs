//! Watch-State („keep watching"): merkt sich pro Serien-Slug die zuletzt
//! geladene Folge und berechnet daraus die nächste.
//!
//! Der Store ist eine kleine JSON-Datei (`{ "<slug>": {"staffel":N,"episode":M} }`).
//! Pfad über `VINDEX_WATCH_STATE`, Default `~/.config/vindex/watch.json`.
//!
//! Die Logik (`parse`, [`WatchState::record`], [`WatchState::next_after`],
//! [`episode_from_url`], [`episode_url`]) ist rein und testbar. Der Dateizugriff
//! ([`load`]/[`save`]) ist bewusst getrennt und fehlertolerant: eine fehlende oder
//! kaputte Datei ergibt einen leeren State, niemals ein Panic.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::BASE_STO;

/// Zuletzt geladene Folge einer Serie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub staffel: u32,
    pub episode: u32,
}

/// Fortschritt pro Serien-Slug. Serialisiert als flaches JSON-Objekt
/// `{ "<slug>": {"staffel":N,"episode":M} }` (BTreeMap → stabile Reihenfolge).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WatchState {
    entries: BTreeMap<String, Progress>,
}

/// Parst den JSON-Store. Leerer oder kaputter Inhalt ergibt einen leeren State
/// (fehlertolerant, kein Panic).
pub fn parse(json: &str) -> WatchState {
    serde_json::from_str(json).unwrap_or_default()
}

impl WatchState {
    /// Zuletzt gemerkte Folge einer Serie, falls vorhanden.
    pub fn get(&self, slug: &str) -> Option<Progress> {
        self.entries.get(slug).copied()
    }

    /// Anzahl gemerkter Serien.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True, wenn noch nichts gemerkt ist.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merkt die zuletzt geladene Folge einer Serie (überschreibt einen
    /// vorhandenen Eintrag für denselben Slug).
    pub fn record(&mut self, slug: &str, staffel: u32, episode: u32) {
        self.entries
            .insert(slug.to_string(), Progress { staffel, episode });
    }

    /// Nächste zu ladende Folge nach der zuletzt gemerkten: `(staffel, episode + 1)`.
    /// `None`, wenn für den Slug noch nichts gemerkt ist — der Aufrufer startet dann
    /// bei Staffel 1 / Episode 1.
    pub fn next_after(&self, slug: &str) -> Option<(u32, u32)> {
        self.get(slug)
            .map(|p| (p.staffel, p.episode.saturating_add(1)))
    }

    /// Serialisiert den Store als eingerücktes JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Zerlegt eine Episoden-URL in `(Slug, Staffel, Episode)`.
///
/// Erwartet die Form `…/<anime|serie>/stream/<slug>/staffel-<N>/episode-<M>`.
/// `None`, wenn kein Slug oder keine Staffel/Episode erkennbar ist (z. B. bei
/// Film-URLs).
pub fn episode_from_url(url: &str) -> Option<(String, u32, u32)> {
    let slug = crate::site::slug_from_url(url)?;
    let staffel = capture_num(url, "/staffel-")?;
    let episode = capture_num(url, "/episode-")?;
    Some((slug, staffel, episode))
}

/// Baut die Episoden-URL für `(staffel, episode)` derselben Serie wie `series_url`.
///
/// Übernimmt Basis-URL und Pfad-Marker aus der Eingabe: serienstream.to/s.to nutzt
/// `/serie/stream/`, aniworld.to `/anime/stream/`. `None`, wenn kein Slug erkannt wird.
pub fn episode_url(series_url: &str, staffel: u32, episode: u32) -> Option<String> {
    let slug = crate::site::slug_from_url(series_url)?;
    let base = crate::site::base_for_url(series_url);
    let marker = if base == BASE_STO {
        "serie/stream"
    } else {
        "anime/stream"
    };
    Some(format!(
        "{base}/{marker}/{slug}/staffel-{staffel}/episode-{episode}"
    ))
}

/// Liest die Zahl zwischen `marker` und dem nächsten `/` (bzw. Ende) aus einer URL.
fn capture_num(url: &str, marker: &str) -> Option<u32> {
    url.split(marker)
        .nth(1)?
        .split('/')
        .next()?
        .parse::<u32>()
        .ok()
}

/// Standardpfad des Stores: `$VINDEX_WATCH_STATE`, sonst
/// `$XDG_CONFIG_HOME/vindex/watch.json`, sonst `~/.config/vindex/watch.json`.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VINDEX_WATCH_STATE").filter(|v| !v.is_empty()) {
        return PathBuf::from(p);
    }
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    config_home.join("vindex").join("watch.json")
}

/// Lädt den Store aus einer Datei. Fehlende, unlesbare oder kaputte Datei ergibt
/// einen leeren State (kein Fehler, kein Panic).
pub fn load(path: &Path) -> WatchState {
    match std::fs::read_to_string(path) {
        Ok(s) => parse(&s),
        Err(_) => WatchState::default(),
    }
}

/// Speichert den Store. Legt bei Bedarf das übergeordnete Verzeichnis an.
pub fn save(path: &Path, state: &WatchState) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
            // Die Historie ist privat — Verzeichnis auf 0700, Datei auf 0600.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    // Atomar und ohne Symlink-Falle: in eine frisch & exklusiv angelegte Temp-Datei
    // schreiben (`create_new`/O_EXCL folgt keinem vorhandenen Symlink), Rechte am
    // Handle erzwingen, syncen, dann umbenennen. Temp-Name ist nicht vorhersagbar.
    let data = state.to_json();
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("watch");
    for attempt in 0..8u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(attempt as u128);
        let name = format!("{base}.{}.{nanos}.tmp", std::process::id());
        let tmp = match dir {
            Some(d) => d.join(&name),
            None => PathBuf::from(&name),
        };
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = match opts.open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        if let Err(e) = f.write_all(data.as_bytes()).and_then(|_| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        drop(f);
        return match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        };
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "Temp-Datei für Watch-State konnte nicht angelegt werden",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_leer_oder_kaputt_ergibt_leeren_state() {
        assert!(parse("").is_empty());
        assert!(parse("{}").is_empty());
        assert!(parse("kein json").is_empty());
        assert!(parse("[1,2,3]").is_empty());
        // Teilweise kaputt: komplett verworfen, kein Panic.
        assert!(parse("{\"dark\":{\"staffel\":\"x\"}}").is_empty());
    }

    #[test]
    fn parse_liest_gueltigen_store() {
        let s = parse("{\"one-piece\":{\"staffel\":2,\"episode\":15}}");
        assert_eq!(s.len(), 1);
        assert_eq!(
            s.get("one-piece"),
            Some(Progress {
                staffel: 2,
                episode: 15
            })
        );
    }

    #[test]
    fn record_merkt_und_ueberschreibt() {
        let mut s = WatchState::default();
        s.record("dark", 1, 3);
        assert_eq!(
            s.get("dark"),
            Some(Progress {
                staffel: 1,
                episode: 3
            })
        );
        // Neuer Fortschritt überschreibt den alten.
        s.record("dark", 1, 4);
        assert_eq!(
            s.get("dark"),
            Some(Progress {
                staffel: 1,
                episode: 4
            })
        );
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn next_after_rechnet_episode_plus_eins() {
        let mut s = WatchState::default();
        s.record("dark", 2, 7);
        assert_eq!(s.next_after("dark"), Some((2, 8)));
    }

    #[test]
    fn next_after_ohne_eintrag_ist_none() {
        let s = WatchState::default();
        // Nichts gemerkt → None; der Aufrufer startet dann bei Staffel 1 / Episode 1.
        assert_eq!(s.next_after("unbekannt"), None);
    }

    #[test]
    fn roundtrip_json() {
        let mut s = WatchState::default();
        s.record("dark", 1, 3);
        s.record("one-piece", 2, 15);
        let json = s.to_json();
        assert_eq!(parse(&json), s);
    }

    #[test]
    fn episode_from_url_aniworld() {
        assert_eq!(
            episode_from_url("https://aniworld.to/anime/stream/one-piece/staffel-2/episode-15"),
            Some(("one-piece".to_string(), 2, 15))
        );
    }

    #[test]
    fn episode_from_url_serienstream() {
        assert_eq!(
            episode_from_url("https://serienstream.to/serie/stream/dark/staffel-1/episode-3"),
            Some(("dark".to_string(), 1, 3))
        );
    }

    #[test]
    fn episode_from_url_ohne_episode_ist_none() {
        // Serien-Übersicht ohne Staffel/Episode → keine Folge ableitbar.
        assert_eq!(
            episode_from_url("https://aniworld.to/anime/stream/naruto"),
            None
        );
        // Film-URL → kein Serien-Slug.
        assert_eq!(episode_from_url("https://filmo.to/movies/the-matrix"), None);
    }

    #[test]
    fn episode_url_baut_naechste_folge() {
        assert_eq!(
            episode_url("https://aniworld.to/anime/stream/naruto", 1, 1).as_deref(),
            Some("https://aniworld.to/anime/stream/naruto/staffel-1/episode-1")
        );
        // Marker/Basis richten sich nach der Serien-URL (serienstream → /serie/stream/).
        assert_eq!(
            episode_url(
                "https://serienstream.to/serie/stream/dark/staffel-1/episode-3",
                1,
                4
            )
            .as_deref(),
            Some("https://serienstream.to/serie/stream/dark/staffel-1/episode-4")
        );
    }

    #[test]
    fn episode_url_ohne_slug_ist_none() {
        assert_eq!(episode_url("https://aniworld.to/", 1, 1), None);
    }

    #[test]
    fn load_fehlende_datei_ergibt_leeren_state() {
        let p = Path::new("/nicht/vorhanden/watch.json");
        assert!(load(p).is_empty());
    }

    #[test]
    fn save_und_load_roundtrip() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("vindex-watchstate-test-{}", std::process::id()));
        let path = dir.join("watch.json");
        let mut s = WatchState::default();
        s.record("dark", 1, 3);
        save(&path, &s).expect("speichern");
        assert_eq!(load(&path), s);
        // Aufräumen.
        std::fs::remove_dir_all(&dir).ok();
    }
}
