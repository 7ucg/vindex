//! Scraper + Resolver für `filmo.to` (Filme).
//!
//! Filmo unterscheidet sich von der SerienStream-Software: Die Filmseite listet
//! pro Sprache Hoster-„Chips" mit einem `data-p`-Token. Die Auflösung läuft über
//! einen CSRF-geschützten POST auf `/n` (→ Playback-Token) und einen anschließenden
//! Redirect auf die eigentliche Hoster-Embed-URL, die dann der normale Hoster-
//! Resolver übernimmt. Portiert aus `models/filmo_to/episode.py`.

use std::collections::HashMap;

use scraper::{Html, Selector};
use serde_json::json;

use crate::error::{Error, Result};
use crate::model::{Hoster, Language};

/// Basis-URL von Filmo.
pub const BASE: &str = "https://filmo.to";

/// Ein Filmo-Film mit Metadaten und Quellen (Sprache → Hoster → `data-p`-Token).
#[derive(Debug, Clone)]
pub struct FilmoMovie {
    pub url: String,
    pub title: String,
    pub year: Option<u32>,
    pub csrf: Option<String>,
    pub sources: HashMap<Language, HashMap<Hoster, String>>,
}

impl FilmoMovie {
    /// Verfügbare Hoster für eine Sprache.
    pub fn hosters_for(&self, lang: Language) -> Option<&HashMap<Hoster, String>> {
        self.sources.get(&lang)
    }
}

fn language_from_label(label: &str) -> Option<Language> {
    match label.trim().to_ascii_lowercase().as_str() {
        "deutsch" | "german" => Some(Language::GermanDub),
        "english" | "englisch" => Some(Language::EnglishDub),
        _ => None,
    }
}

/// Lädt und parst eine Filmo-Filmseite.
pub async fn movie(client: &reqwest::Client, url: &str) -> Result<FilmoMovie> {
    let resp = client
        .get(url)
        .header("Accept-Encoding", "gzip, deflate")
        .send()
        .await?;
    let status = resp.status().as_u16();
    let html = resp.text().await?;
    if crate::http::looks_like_challenge(&html, status) {
        return Err(Error::Challenge(url.to_string()));
    }

    let mut title = String::new();
    let mut csrf = None;
    let mut sources: HashMap<Language, HashMap<Hoster, String>> = HashMap::new();

    {
        let doc = Html::parse_document(&html);

        if let Some(h1) = doc.select(&Selector::parse("h1").unwrap()).next() {
            title = h1.text().collect::<String>().trim().to_string();
        }
        // CSRF-Token aus <meta name="csrf-token" content="…">.
        for meta in doc.select(&Selector::parse("meta[name='csrf-token']").unwrap()) {
            if let Some(c) = meta.value().attr("content") {
                csrf = Some(c.to_string());
            }
        }

        // Sprach-Label und Hoster-Chips in Dokumentreihenfolge durchlaufen:
        // ein kombinierter Selektor liefert Treffer in Dokumentreihenfolge, so
        // dass jeder Chip der zuletzt gesehenen Sprache zugeordnet wird
        // (spiegelt den streaming HTMLParser der Referenz).
        let combined = Selector::parse(".provider-row__lang, [data-provider-chip]").unwrap();
        let mut current_lang: Option<Language> = None;
        for el in doc.select(&combined) {
            if el.value().attr("data-provider-chip").is_some()
                || el
                    .value()
                    .attr("class")
                    .map(|c| c.split_whitespace().any(|x| x == "provider-chip"))
                    .unwrap_or(false)
            {
                // Hoster-Chip.
                let Some(lang) = current_lang else { continue };
                let label = el
                    .value()
                    .attr("aria-label")
                    .or_else(|| el.value().attr("title"))
                    .unwrap_or_default();
                let token = el.value().attr("data-p").unwrap_or_default();
                if token.is_empty() {
                    continue;
                }
                if let Some(hoster) = Hoster::from_host_or_name(label) {
                    sources
                        .entry(lang)
                        .or_default()
                        .entry(hoster)
                        .or_insert_with(|| token.to_string());
                }
            } else {
                // Sprach-Label.
                let text = el.text().collect::<String>();
                current_lang = language_from_label(&text);
            }
        }
    }

    if title.is_empty() {
        return Err(Error::extract("Filmo", "Titel (h1) nicht gefunden"));
    }
    let year = extract_year(&html);

    Ok(FilmoMovie {
        url: url.to_string(),
        title,
        year,
        csrf,
        sources,
    })
}

fn extract_year(html: &str) -> Option<u32> {
    // Jahr aus dem Release-Feld — Deutsch („Erscheinungsdatum") oder Englisch
    // („Release date"); grob das erste 4-stellige Jahr danach.
    let re = regex::Regex::new(r"(?is)(?:Erscheinungsdatum|Release date).*?(\d{4})").unwrap();
    re.captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .filter(|y| (1900..=2100).contains(y))
}

/// Löst ein `data-p`-Token zur Hoster-Embed-URL auf (CSRF-POST `/n` → Redirect).
pub async fn resolve_embed(
    client: &reqwest::Client,
    movie_url: &str,
    csrf: &str,
    token: &str,
) -> Result<String> {
    let endpoint = format!("{BASE}/n");
    let body = serde_json::to_string(&json!({ "p": token }))
        .map_err(|e| Error::extract("Filmo", format!("json: {e}")))?;

    let resp = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .header("X-CSRF-TOKEN", csrf)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Referer", movie_url)
        .header("Accept", "application/json")
        .body(body)
        .send()
        .await?;
    let text = resp.text().await?;
    let payload: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::extract("Filmo", format!("Antwort kein JSON: {e}")))?;
    let playback = payload
        .get("x")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::extract("Filmo", "kein Playback-Token (x) erhalten"))?;

    let redirect_url = format!("{endpoint}/{}", percent_encode(playback));

    // Dem Redirect zur Hoster-Embed-URL folgen.
    let final_url = client
        .get(&redirect_url)
        .header("Referer", movie_url)
        .send()
        .await?
        .url()
        .to_string();

    // Muss zu einem Hoster führen, nicht auf Filmo selbst zurück.
    if final_url.contains("filmo.to") {
        return Err(Error::extract(
            "Filmo",
            "Redirect führte nicht zu einem Stream-Hoster",
        ));
    }
    Ok(final_url)
}

/// Percent-Encoding für ein Pfadsegment (unreserved bleibt, Rest wird kodiert).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_mapping() {
        assert_eq!(language_from_label("Deutsch"), Some(Language::GermanDub));
        assert_eq!(language_from_label("German"), Some(Language::GermanDub));
        assert_eq!(language_from_label("English"), Some(Language::EnglishDub));
        assert_eq!(language_from_label("Français"), None);
    }

    #[test]
    fn year_from_release_field() {
        let html = r#"<h3>Release date</h3><dd>12 March 2021</dd>"#;
        assert_eq!(extract_year(html), Some(2021));
    }

    #[test]
    fn percent_encoding() {
        assert_eq!(percent_encode("abcXYZ09-_.~"), "abcXYZ09-_.~");
        assert_eq!(percent_encode("a/b c"), "a%2Fb%20c");
    }
}

#[cfg(test)]
mod year_de {
    #[test]
    fn deutsch_und_grenzen() {
        assert_eq!(super::extract_year("<p>Erscheinungsdatum: 2012</p>"), Some(2012));
        assert_eq!(super::extract_year("Release date 2021"), Some(2021));
        assert_eq!(super::extract_year("kein jahr"), None);
    }
}
