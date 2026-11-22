//! Scraper für `aniworld.to`.
//!
//! Die hier verwendeten Selektoren (`/animes-alphabet`, `div.hosterSiteDirectNav`,
//! `.seasonEpisodeTitle`, der `Filme`-Tab) sind durch das census-Unterbefehl real
//! verifiziert und dienen als Fundament für die spätere Hoster-Auflösung
//! (siehe `ARCHITECTURE.md` §4.1).
//!
//! **Idiom-Hinweis:** `scraper::Html` ist nicht `Send`. Alle Daten aus dem
//! `Html`-Dokument extrahieren und `doc` vor dem nächsten `.await` fallenlassen.

use scraper::{Html, Selector};

use crate::error::Result;
use crate::model::{CatalogEntry, Classified, Episode, Kind, ProviderLinks, BASE};

// ---------------------------------------------------------------------------
// Download-orientierte Wrapper um den generischen SerienStream-Scraper.
// ---------------------------------------------------------------------------

/// Staffel-Nummern eines Titels (`0` = Filme-Tab).
pub async fn list_seasons(client: &reqwest::Client, slug: &str) -> Result<Vec<u32>> {
    super::serienstream::list_seasons(client, BASE, slug).await
}

/// Episoden einer Staffel.
pub async fn list_episodes(
    client: &reqwest::Client,
    slug: &str,
    season: u32,
) -> Result<Vec<Episode>> {
    super::serienstream::list_episodes(client, BASE, slug, season).await
}

/// Hoster-Links einer Episodenseite, gruppiert nach Sprache.
pub async fn provider_links(client: &reqwest::Client, episode_url: &str) -> Result<ProviderLinks> {
    super::serienstream::provider_links(client, BASE, episode_url).await
}

/// Lädt den Gesamtkatalog von `/animes-alphabet`.
///
/// `/animes-alphabet` ist der vollständigste Katalog (Superset von `/animes`,
/// das nach Genre gruppiert ist und einzelne Titel ohne Genre-Zuordnung auslässt).
pub async fn fetch_catalog(client: &reqwest::Client) -> anyhow::Result<Vec<CatalogEntry>> {
    let url = format!("{BASE}/animes-alphabet");
    let body = crate::http::get_text_retry(client, &url, None).await?;
    let doc = Html::parse_document(&body);
    let sel = Selector::parse(r#"a[href^="/anime/stream/"]"#).unwrap();

    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::new();
    for el in doc.select(&sel) {
        let href = el.value().attr("href").unwrap_or("");
        // Nur Basis-Links der Form /anime/stream/<slug>, keine Staffel/Episoden-Unterlinks.
        let slug = href.trim_start_matches("/anime/stream/");
        if slug.is_empty() || slug.contains('/') {
            continue;
        }
        if !seen.insert(slug.to_string()) {
            continue;
        }
        let title = el.text().collect::<String>().trim().to_string();
        if title.is_empty() {
            continue;
        }
        entries.push(CatalogEntry {
            title,
            slug: slug.to_string(),
        });
    }
    Ok(entries)
}

/// Klassifiziert einen Titel über seine Detailseite.
///
/// Zählt Staffeln, Episoden der Staffel 1 und einen etwaigen „Filme"-Tab. Mit
/// `count_episodes` werden zusätzlich alle Folgen über alle Staffeln abgerufen
/// (deutlich mehr Requests: ein Abruf pro Staffel).
pub async fn classify(
    client: &reqwest::Client,
    entry: &CatalogEntry,
    count_episodes: bool,
) -> anyhow::Result<Classified> {
    let url = format!("{BASE}/anime/stream/{}", entry.slug);
    let body = crate::http::get_text_retry(client, &url, None).await?;

    // Alles aus dem (nicht-Send) Html-Dokument hier extrahieren und doc danach
    // fallenlassen, bevor der nächste await kommt.
    let (mut season_indices, episodes_season1, has_filme_tab) = {
        let doc = Html::parse_document(&body);

        let season_sel = Selector::parse(&format!(
            r#"a[href^="/anime/stream/{}/staffel-"]"#,
            entry.slug
        ))
        .unwrap();
        let mut season_nums = std::collections::BTreeSet::new();
        for el in doc.select(&season_sel) {
            if let Some(href) = el.value().attr("href") {
                // Nur reine Staffel-Links zählen, keine Episoden-Unterlinks
                // (.../staffel-1/episode-3).
                if href.contains("/episode-") {
                    continue;
                }
                // Staffel-Nummer aus .../staffel-N extrahieren.
                if let Some(n) = href
                    .rsplit("/staffel-")
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    season_nums.insert(n);
                }
            }
        }

        let episode_sel = Selector::parse(".seasonEpisodeTitle").unwrap();
        let episodes_season1 = doc.select(&episode_sel).count();

        // „Filme"-Tab erkennen (Serien wie Detektiv Conan haben zusätzlich einen
        // Film-Bereich).
        let filme_sel =
            Selector::parse(&format!(r#"a[href="/anime/stream/{}/filme"]"#, entry.slug)).unwrap();
        let has_filme_tab = doc.select(&filme_sel).next().is_some();

        (
            season_nums.into_iter().collect::<Vec<_>>(),
            episodes_season1,
            has_filme_tab,
        )
    };

    if season_indices.is_empty() {
        season_indices.push(1); // Basis-Seite zeigt immer mind. Staffel 1.
    }
    let seasons = season_indices.len();

    let films = if has_filme_tab {
        count_episodes_on(client, &format!("{BASE}/anime/stream/{}/filme", entry.slug))
            .await
            .unwrap_or(0)
    } else {
        0
    };

    // Alle Folgen über alle Staffeln zählen (Staffel 1 kennen wir bereits).
    let total_episodes = if count_episodes {
        let mut total = episodes_season1;
        for &n in &season_indices {
            if n == 1 {
                continue; // Staffel 1 bereits von der Hauptseite.
            }
            let url = format!("{BASE}/anime/stream/{}/staffel-{}", entry.slug, n);
            total += count_episodes_on(client, &url).await.unwrap_or(0);
        }
        total
    } else {
        0
    };

    // Reiner Film-Titel: nur eine Staffel mit einer Episode und kein separater Film-Tab.
    let standalone_movie = seasons <= 1 && episodes_season1 <= 1 && !has_filme_tab;

    let kind = if standalone_movie {
        Kind::Movie
    } else {
        Kind::Series
    };

    // Ein Standalone-Film zählt als 1 Film; sonst die Filme aus dem Filme-Tab.
    let film_total = if standalone_movie { 1 } else { films };

    Ok(Classified {
        title: entry.title.clone(),
        slug: entry.slug.clone(),
        seasons,
        episodes_season1,
        films: film_total,
        total_episodes,
        kind,
    })
}

/// Lädt eine Staffel-/Filme-Seite und zählt die dort gelisteten Episoden/Filme.
pub async fn count_episodes_on(client: &reqwest::Client, url: &str) -> anyhow::Result<usize> {
    let body = crate::http::get_text_retry(client, url, None).await?;
    let doc = Html::parse_document(&body);
    let sel = Selector::parse(".seasonEpisodeTitle").unwrap();
    Ok(doc.select(&sel).count())
}
