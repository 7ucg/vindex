//! Generischer Scraper für die SerienStream-Software (aniworld.to **und** serienstream.to).
//!
//! Beide Portale laufen auf derselben Engine; die Selektoren sind identisch, nur
//! die Basis-URL unterscheidet sich. Portiert aus `pages/aniworld_to.py` und
//! `models/aniworld_to/*`.

use scraper::{Html, Selector};

use crate::error::{Error, Result};
use crate::model::{Episode, Hoster, Language, ProviderLinks};
use crate::naming::{episode_number_from_title, strip_episode_marker};

/// Liefert die Staffel-Nummern eines Titels (`0` = Filme-Tab).
pub async fn list_seasons(client: &reqwest::Client, base: &str, slug: &str) -> Result<Vec<u32>> {
    let url = format!("{base}/anime/stream/{slug}");
    let body = crate::http::get_text_retry(client, &url, None).await?;
    let seasons = {
        let doc = Html::parse_document(&body);
        let nav_sel =
            Selector::parse("div.hosterSiteDirectNav ul li a:not([data-episode-id])").unwrap();
        let mut out = std::collections::BTreeSet::new();
        for el in doc.select(&nav_sel) {
            let text = el.text().collect::<String>();
            let text = text.trim();
            if text.eq_ignore_ascii_case("Filme") {
                out.insert(0);
            } else if let Ok(n) = text.parse::<u32>() {
                out.insert(n);
            }
        }
        out.into_iter().collect::<Vec<_>>()
    };
    Ok(seasons)
}

/// Liefert die Episoden einer Staffel (`season = 0` ⇒ Filme-Tab).
pub async fn list_episodes(
    client: &reqwest::Client,
    base: &str,
    slug: &str,
    season: u32,
) -> Result<Vec<Episode>> {
    let url = if season == 0 {
        format!("{base}/anime/stream/{slug}/filme")
    } else {
        format!("{base}/anime/stream/{slug}/staffel-{season}")
    };
    let body = crate::http::get_text_retry(client, &url, None).await?;

    let episodes = {
        let doc = Html::parse_document(&body);
        // Episoden-Zeilen der Staffel; das tbody trägt die Id `season{N}` (Filme: season0).
        let row_sel = Selector::parse(&format!("tbody#season{season} tr")).unwrap();
        let title_sel = Selector::parse("td.seasonEpisodeTitle a").unwrap();
        let span_sel = Selector::parse("span").unwrap();

        let mut out = Vec::new();
        for (i, row) in doc.select(&row_sel).enumerate() {
            let Some(a) = row.select(&title_sel).next() else {
                continue;
            };
            let Some(href) = a.value().attr("href") else {
                continue;
            };
            let raw_title = a
                .select(&span_sel)
                .next()
                .map(|s| s.text().collect::<String>())
                .unwrap_or_default();
            let title = strip_episode_marker(&raw_title);
            let explicit_number = episode_number_from_title(&raw_title);
            out.push(Episode {
                url: format!("{base}{href}"),
                title,
                season,
                index_in_season: (i + 1) as u32,
                explicit_number,
            });
        }
        out
    };
    Ok(episodes)
}

/// Löst die Hoster-Links einer Episodenseite auf, gruppiert nach Sprache.
///
/// Jeder Hoster-Button ist ein `<li data-lang-key="…" data-link-target="…">` mit
/// einem `<h4>`, das den Hoster-Namen trägt. `data-link-target` zeigt auf
/// `…/redirect/N`.
pub async fn provider_links(
    client: &reqwest::Client,
    base: &str,
    episode_url: &str,
) -> Result<ProviderLinks> {
    let resp = crate::http::get_retry(client, episode_url, None).await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if crate::http::looks_like_challenge(&body, status) {
        return Err(Error::Challenge(episode_url.to_string()));
    }

    let mut links = ProviderLinks::default();
    {
        let doc = Html::parse_document(&body);
        let li_sel = Selector::parse("li[data-lang-key][data-link-target]").unwrap();
        let h4_sel = Selector::parse("h4").unwrap();
        for li in doc.select(&li_sel) {
            let Some(key) = li.value().attr("data-lang-key") else {
                continue;
            };
            let Some(lang) = Language::from_aniworld_key(key) else {
                continue;
            };
            let Some(target) = li.value().attr("data-link-target") else {
                continue;
            };
            let Some(name) = li.select(&h4_sel).next() else {
                continue;
            };
            let hoster_name = name.text().collect::<String>();
            let Some(hoster) = Hoster::from_site_name(hoster_name.trim()) else {
                continue;
            };
            links
                .by_language
                .entry(lang)
                .or_default()
                .insert(hoster, format!("{base}{target}"));
        }
    }

    if links.is_empty() {
        return Err(Error::extract(
            "SerienStream",
            "keine Hoster-Links auf der Episodenseite gefunden",
        ));
    }
    Ok(links)
}
