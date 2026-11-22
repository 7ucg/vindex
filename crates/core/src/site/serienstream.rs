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
    let body = super::fetch_text(client, &url).await?;
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
    let body = super::fetch_text(client, &url).await?;

    Ok(parse_episodes(&body, base, season))
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
    let resp = super::fetch(client, episode_url).await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if crate::http::looks_like_challenge(&body, status) {
        return Err(Error::Challenge(episode_url.to_string()));
    }

    let links = parse_provider_links(&body, base);

    if links.is_empty() {
        return Err(Error::extract(
            "SerienStream",
            "keine Hoster-Links auf der Episodenseite gefunden",
        ));
    }
    Ok(links)
}

/// Reine HTML→Episoden-Logik von [`list_episodes`].
fn parse_episodes(body: &str, base: &str, season: u32) -> Vec<Episode> {
    let doc = Html::parse_document(body);
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
}

/// Reine HTML→Hoster-Links-Logik von [`provider_links`].
fn parse_provider_links(body: &str, base: &str) -> ProviderLinks {
    let mut links = ProviderLinks::default();
    let doc = Html::parse_document(body);
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
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_T: &str = "https://example.test";

    #[test]
    fn hoster_links_nach_sprache_gruppiert() {
        let html = r#"<ul>
            <li data-lang-key="1" data-link-target="/redirect/11"><h4>VOE</h4></li>
            <li data-lang-key="1" data-link-target="/redirect/12"><h4> Vidoza </h4></li>
            <li data-lang-key="3" data-link-target="/redirect/13"><h4>Filemoon</h4></li>
        </ul>"#;
        let links = parse_provider_links(html, BASE_T);
        assert_eq!(
            links.link(Language::GermanDub, Hoster::Voe),
            Some("https://example.test/redirect/11")
        );
        assert_eq!(
            links.link(Language::GermanDub, Hoster::Vidoza),
            Some("https://example.test/redirect/12")
        );
        assert_eq!(
            links.link(Language::GermanSub, Hoster::Filemoon),
            Some("https://example.test/redirect/13")
        );
    }

    #[test]
    fn hoster_links_ueberspringt_unbekannte_eintraege() {
        let html = r#"<ul>
            <li data-lang-key="9" data-link-target="/redirect/1"><h4>VOE</h4></li>
            <li data-lang-key="1" data-link-target="/redirect/2"><h4>Unbekannt</h4></li>
            <li data-lang-key="1" data-link-target="/redirect/3"></li>
            <li data-lang-key="1"><h4>VOE</h4></li>
            <li data-link-target="/redirect/4"><h4>VOE</h4></li>
        </ul>"#;
        assert!(parse_provider_links(html, BASE_T).is_empty());
    }

    #[test]
    fn hoster_links_leeres_html_ist_leer() {
        assert!(parse_provider_links("", BASE_T).is_empty());
    }

    #[test]
    fn hoster_links_letzter_eintrag_pro_hoster_gewinnt() {
        let html = r#"
            <li data-lang-key="2" data-link-target="/redirect/1"><h4>VOE</h4></li>
            <li data-lang-key="2" data-link-target="/redirect/2"><h4>VOE</h4></li>"#;
        let links = parse_provider_links(html, BASE_T);
        assert_eq!(
            links.link(Language::EnglishSub, Hoster::Voe),
            Some("https://example.test/redirect/2")
        );
    }

    fn episoden_html() -> &'static str {
        r#"<table>
        <tbody id="season1">
          <tr><td class="seasonEpisodeTitle"><a href="/anime/stream/x/staffel-1/episode-1"><strong>Ankunft</strong> <span>Arrival [Episode 1]</span></a></td></tr>
          <tr><td>ohne Titelzelle</td></tr>
          <tr><td class="seasonEpisodeTitle"><a href="/anime/stream/x/staffel-1/episode-3"><span>Plain Title</span></a></td></tr>
        </tbody>
        <tbody id="season2">
          <tr><td class="seasonEpisodeTitle"><a href="/anime/stream/x/staffel-2/episode-1"><span>Other</span></a></td></tr>
        </tbody></table>"#
    }

    #[test]
    fn episoden_nur_der_gewuenschten_staffel() {
        let eps = parse_episodes(episoden_html(), BASE_T, 2);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].season, 2);
        assert_eq!(eps[0].title, "Other");
        assert_eq!(
            eps[0].url,
            "https://example.test/anime/stream/x/staffel-2/episode-1"
        );
    }

    #[test]
    fn episoden_titel_marker_und_nummer() {
        let eps = parse_episodes(episoden_html(), BASE_T, 1);
        assert_eq!(eps[0].title, "Arrival");
        assert_eq!(eps[0].explicit_number, Some(1));
        assert_eq!(eps[0].index_in_season, 1);
    }

    #[test]
    fn episoden_index_zaehlt_zeilen_ohne_titelzelle_mit() {
        let eps = parse_episodes(episoden_html(), BASE_T, 1);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[1].index_in_season, 3);
        assert_eq!(eps[1].explicit_number, None);
    }

    #[test]
    fn episoden_unbekannte_staffel_ist_leer() {
        assert!(parse_episodes(episoden_html(), BASE_T, 7).is_empty());
    }
}
