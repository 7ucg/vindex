//! Datei-Namensgebung und Episoden-Nummerierung.
//!
//! Portiert aus `models/common/naming.py` und `numbering.py`:
//! Jellyfin-Template `{title} ({year}) [imdbid-{imdbid}]/Season {season}/{title} S{season}E{episode}.mkv`
//! und die Logik „expliziter `[Episode N]`-Marker aus dem EN-Titel".

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Default-Template (vgl. `NAMING_TEMPLATE` / `.env.example`).
pub const DEFAULT_TEMPLATE: &str =
    "{title} ({year}) [imdbid-{imdbid}]/Season {season}/{title} S{season}E{episode}.mkv";

/// Liest eine explizite Episodennummer aus einem Titel — nur echte Marker
/// (`[Episode N]`, `[JP Episode N]`, oder die ganze Zeile `Episode N`), nie eine
/// beliebige Zahl im Titel. Nur bei genau einer eindeutigen Zahl.
pub fn episode_number_from_title(title: &str) -> Option<u32> {
    let re = Regex::new(
        r"(?i)\[\s*(?:JP\s+)?Episode\s+([0-9]+)\s*\]|^\s*(?:JP\s+)?Episode\s+([0-9]+)\s*$",
    )
    .unwrap();
    let mut numbers = std::collections::BTreeSet::new();
    for caps in re.captures_iter(title) {
        let n = caps
            .get(1)
            .or_else(|| caps.get(2))
            .and_then(|m| m.as_str().parse::<u32>().ok());
        if let Some(n) = n {
            numbers.insert(n);
        }
    }
    if numbers.len() == 1 {
        let n = *numbers.iter().next().unwrap();
        if n > 0 {
            return Some(n);
        }
    }
    None
}

/// Entfernt einen angehängten `[Episode NNN]`-/`[JP Episode NNN]`-Marker aus einem
/// Titel (die Marker stehen immer am Ende).
pub fn strip_episode_marker(title: &str) -> String {
    match title
        .find(" [Episode ")
        .or_else(|| title.find(" [JP Episode "))
    {
        Some(idx) => title[..idx].trim().to_string(),
        None => title.trim().to_string(),
    }
}

/// Rendert einen Pfad aus dem Template und den Platzhalter-Werten.
///
/// Das Template wird an `/` in Komponenten zerlegt; die Dateiendung der letzten
/// Komponente bestimmt das Containerformat (z. B. `.mkv`).
pub fn render_path(root: &Path, template: &str, values: &HashMap<&str, String>) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in template.split('/') {
        path.push(fill(part, values));
    }
    path
}

fn fill(template: &str, values: &HashMap<&str, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in values {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Baut die Standard-Platzhalter-Map.
pub fn values(
    title: &str,
    year: &str,
    imdbid: &str,
    season: u32,
    episode: u32,
    language: &str,
    resolution: &str,
) -> HashMap<&'static str, String> {
    let mut m = HashMap::new();
    m.insert("title", clean_title(title));
    m.insert("year", year.to_string());
    m.insert("imdbid", imdbid.to_string());
    m.insert("season", format!("{season:02}"));
    m.insert("episode", format!("{episode:03}"));
    m.insert("language", language.to_string());
    m.insert("resolution", resolution.to_string());
    m
}

/// Macht einen Titel dateisystem-tauglich (entfernt problematische Zeichen).
pub fn clean_title(title: &str) -> String {
    title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_single() {
        assert_eq!(
            episode_number_from_title("Title [Episode 1157]"),
            Some(1157)
        );
        assert_eq!(episode_number_from_title("[JP Episode 42]"), Some(42));
        assert_eq!(episode_number_from_title("Episode 7"), Some(7));
    }

    #[test]
    fn marker_none_or_ambiguous() {
        assert_eq!(episode_number_from_title("Just a title"), None);
        assert_eq!(episode_number_from_title("Season 2 something 2024"), None);
        // zwei verschiedene Marker ⇒ nicht eindeutig
        assert_eq!(
            episode_number_from_title("[Episode 1] and [Episode 2]"),
            None
        );
    }

    #[test]
    fn strip_marker() {
        assert_eq!(strip_episode_marker("Ep Title [Episode 001]"), "Ep Title");
        assert_eq!(strip_episode_marker("Plain"), "Plain");
    }

    #[test]
    fn render_template() {
        let v = values("My Show", "2012", "tt123", 1, 5, "German Dub", "1080p");
        let p = render_path(Path::new("/dl"), DEFAULT_TEMPLATE, &v);
        assert_eq!(
            p.to_string_lossy(),
            "/dl/My Show (2012) [imdbid-tt123]/Season 01/My Show S01E005.mkv"
        );
    }
}
