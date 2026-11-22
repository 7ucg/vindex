//! Scraper für die SerienStream-Software.
//!
//! Die Selektoren der Episoden-/Staffel-Logik (`/anime/stream/<slug>/…`) treffen
//! aktuell **aniworld.to**. [`parse_provider_links`] bedient dagegen **beide**
//! Hoster-Markups: zuerst aniworlds `li[data-lang-key][data-link-target]` mit
//! `<h4>`-Hostername und `/redirect/N`, und falls das nichts liefert, das neue
//! serienstream.to-Markup (`button[data-provider-name][data-play-url]` mit
//! `data-language-id`/`-label`, Link `/r?t=<token>`). serienstream.to leidet
//! zusätzlich unter häufigen Cloudflare-Captchas; der Parser ist nur gegen ein
//! Fixture verifiziert. s.to ist stillgelegt (→ serienstream.to). Portiert aus
//! `pages/aniworld_to.py` und `models/aniworld_to/*`.

use scraper::{Html, Selector};

use crate::error::{Error, Result};
use crate::model::{Episode, Hoster, Language, ProviderLinks};
use crate::naming::{episode_number_from_title, strip_episode_marker};

/// Liefert die Staffel-Nummern eines Titels (`0` = Filme-Tab).
pub async fn list_seasons(client: &reqwest::Client, base: &str, slug: &str) -> Result<Vec<u32>> {
    let url = format!("{base}/anime/stream/{slug}");
    let body = super::fetch_text(client, &url).await?;
    Ok(parse_season_nav(&body))
}

/// Reine HTML→Staffel-Logik von [`list_seasons`]. Liest die Staffel-Navigation
/// (`div.hosterSiteDirectNav`): numerische Einträge sind Staffeln, „Filme" ist
/// Staffel 0. Episoden-Links (`data-episode-id`) werden ausgelassen. Sortiert &
/// dedupliziert.
fn parse_season_nav(body: &str) -> Vec<u32> {
    let doc = Html::parse_document(body);
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
    out.into_iter().collect()
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
    let mut body = resp.text().await?;
    if crate::http::looks_like_challenge(&body, status) {
        // Anti-Bot Stufe 2 (opt-in): bei Cloudflare-Verdacht einmalig über den
        // TLS-Impersonate-Client (Chrome-Fingerprint) wiederholen.
        if crate::http::impersonate_enabled() {
            body = crate::http::get_text_impersonate(episode_url, None).await?;
        }
        // Anti-Bot Stufe 3 (opt-in): kommt die Seite weiter nicht durch (oder ist
        // Impersonate aus), den Headless-Browser obscura versuchen — der rendert JS
        // und löst JS-/Cloudflare-Interstitials, DNS erzwungen über 1.1.1.1.
        if crate::http::looks_like_challenge(&body, status) && crate::browser::obscura_enabled() {
            body = crate::browser::fetch_html(episode_url).await?;
        }
        if crate::http::looks_like_challenge(&body, status) {
            return Err(Error::Challenge(episode_url.to_string()));
        }
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
///
/// Bedient beide Markups: zuerst das aniworld-Markup; liefert das keine Links,
/// wird auf das neue serienstream.to-Markup zurückgefallen.
fn parse_provider_links(body: &str, base: &str) -> ProviderLinks {
    let doc = Html::parse_document(body);
    let links = parse_provider_links_aniworld(&doc, base);
    if !links.is_empty() {
        return links;
    }
    parse_provider_links_serienstream(&doc, base)
}

/// aniworld.to-Markup: `<li data-lang-key="…" data-link-target="…">` mit `<h4>`
/// als Hostername und `…/redirect/N` als Ziel.
fn parse_provider_links_aniworld(doc: &Html, base: &str) -> ProviderLinks {
    let mut links = ProviderLinks::default();
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

/// Neues serienstream.to-Markup: Jeder Hoster ist ein `<button>` mit
/// `data-provider-name` (Hostername), `data-play-url="/r?t=<token>"` (serverseitiger
/// Weiterleitungs-Link, analog aniworlds `/redirect/N`) und der Sprache direkt am
/// Button über `data-language-id`/`data-language-label`. Das `<token>` ist ein
/// Laravel-Crypt-Payload und wird **nicht** entschlüsselt, sondern unverändert in
/// die absolute URL übernommen.
fn parse_provider_links_serienstream(doc: &Html, base: &str) -> ProviderLinks {
    let mut links = ProviderLinks::default();
    let btn_sel = Selector::parse("button[data-provider-name][data-play-url]").unwrap();
    for btn in doc.select(&btn_sel) {
        let el = btn.value();
        let lang =
            language_from_serienstream(el.attr("data-language-id"), el.attr("data-language-label"));
        let Some(lang) = lang else {
            continue;
        };
        let Some(play_url) = el.attr("data-play-url") else {
            continue;
        };
        let Some(hoster) = el
            .attr("data-provider-name")
            .and_then(|n| Hoster::from_site_name(n.trim()))
        else {
            continue;
        };
        links
            .by_language
            .entry(lang)
            .or_default()
            .insert(hoster, format!("{base}{play_url}"));
    }
    links
}

/// Mappt die serienstream.to-Sprachkennung auf [`Language`].
///
/// serienstream.to führt westliche Serien, keine Anime: `data-language-id="1"`
/// („Deutsch") ist die deutsche Synchro → [`Language::GermanDub`], `="2"`
/// („Englisch") die englische Originaltonspur → [`Language::EnglishDub`] (die
/// `…Sub`-Varianten tragen anime-spezifische Anzeigenamen und passen hier nicht).
/// `data-language-id` hat Vorrang; fehlt sie, entscheidet das `data-language-label`.
fn language_from_serienstream(id: Option<&str>, label: Option<&str>) -> Option<Language> {
    match id.map(str::trim) {
        Some("1") => return Some(Language::GermanDub),
        Some("2") => return Some(Language::EnglishDub),
        _ => {}
    }
    match label.map(|l| l.trim().to_ascii_lowercase()).as_deref() {
        Some("deutsch") => Some(Language::GermanDub),
        Some("englisch") => Some(Language::EnglishDub),
        _ => None,
    }
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

    // Synthetisches serienstream.to-Markup: echte Struktur, aber frei erfundene
    // Tokens/Ids/Titel (kein echter Laravel-Crypt-Payload, kein Copyright-Inhalt).
    const SERIENSTREAM_HTML: &str = r#"
    <div id="episode-links" class="row">
        <div class="col-12"><h5 class="h6 text-muted">Deutsch</h5></div>
        <div class="col-12 col-sm-6">
            <div class="link-wrapper">
                <button type="button" class="link-box btn btn-dark"
                        data-link-id="1000001"
                        data-play-url="/r?t=SYNTH-DE-VOE-TOKEN"
                        data-auto-embed="1"
                        data-provider-name="VOE"
                        data-language-label="Deutsch"
                        data-language-id="1">
                    <span class="text-white">VOE</span>
                </button>
            </div>
        </div>
        <div class="col-12 col-sm-6">
            <div class="link-wrapper">
                <button type="button" class="link-box btn btn-dark"
                        data-link-id="1000002"
                        data-play-url="/r?t=SYNTH-DE-VIDOZA-TOKEN"
                        data-auto-embed="1"
                        data-provider-name=" Vidoza "
                        data-language-label="Deutsch"
                        data-language-id="1">
                    <span class="text-white">Vidoza</span>
                </button>
            </div>
        </div>
        <div class="col-12"><h5 class="h6 text-muted">Englisch</h5></div>
        <div class="col-12 col-sm-6">
            <div class="link-wrapper">
                <button type="button" class="link-box btn btn-dark"
                        data-link-id="1000003"
                        data-play-url="/r?t=SYNTH-EN-VOE-TOKEN"
                        data-auto-embed="1"
                        data-provider-name="VOE"
                        data-language-label="Englisch"
                        data-language-id="2">
                    <span class="text-white">VOE</span>
                </button>
            </div>
        </div>
        <div class="col-12 col-sm-6">
            <div class="link-wrapper">
                <button type="button" class="link-box btn btn-dark"
                        data-link-id="1000004"
                        data-play-url="/r?t=SYNTH-EN-UNKNOWN-TOKEN"
                        data-auto-embed="1"
                        data-provider-name="UnbekannterHoster"
                        data-language-label="Englisch"
                        data-language-id="2">
                    <span class="text-white">UnbekannterHoster</span>
                </button>
            </div>
        </div>
    </div>"#;

    #[test]
    fn serienstream_hoster_links_nach_sprache_gruppiert() {
        let links = parse_provider_links(SERIENSTREAM_HTML, BASE_T);
        assert_eq!(
            links.link(Language::GermanDub, Hoster::Voe),
            Some("https://example.test/r?t=SYNTH-DE-VOE-TOKEN")
        );
        assert_eq!(
            links.link(Language::GermanDub, Hoster::Vidoza),
            Some("https://example.test/r?t=SYNTH-DE-VIDOZA-TOKEN")
        );
        assert_eq!(
            links.link(Language::EnglishDub, Hoster::Voe),
            Some("https://example.test/r?t=SYNTH-EN-VOE-TOKEN")
        );
        // Unbekannter Hoster wird übersprungen; sonst nichts zusätzlich.
        assert!(links.link(Language::EnglishDub, Hoster::Vidoza).is_none());
        assert_eq!(
            links.hosters_for(Language::GermanDub).map(|m| m.len()),
            Some(2)
        );
        assert_eq!(
            links.hosters_for(Language::EnglishDub).map(|m| m.len()),
            Some(1)
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

    #[test]
    fn episoden_filme_tab_ist_staffel_null() {
        // Der Filme-Tab trägt keine [Episode N]-Marker; Titel bleiben wie sie sind,
        // die Reihenfolge bestimmt index_in_season.
        let html = r#"<table><tbody id="season0">
          <tr><td class="seasonEpisodeTitle"><a href="/anime/stream/x/filme/film-1"><span>Der erste Film</span></a></td></tr>
          <tr><td class="seasonEpisodeTitle"><a href="/anime/stream/x/filme/film-2"><span>Zweiter Film</span></a></td></tr>
        </tbody></table>"#;
        let eps = parse_episodes(html, BASE_T, 0);
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].season, 0);
        assert_eq!(eps[0].title, "Der erste Film");
        assert_eq!(eps[0].explicit_number, None);
        assert_eq!(eps[0].index_in_season, 1);
        assert_eq!(
            eps[1].url,
            "https://example.test/anime/stream/x/filme/film-2"
        );
        assert_eq!(eps[1].index_in_season, 2);
    }

    // Synthetische Staffel-Navigation im serienstream/aniworld-Layout.
    const NAV_HTML: &str = r#"
    <div class="hosterSiteDirectNav">
      <ul>
        <li><a href="/anime/stream/x/filme" title="Filme">Filme</a></li>
        <li><a href="/anime/stream/x/staffel-1">1</a></li>
        <li><a href="/anime/stream/x/staffel-2">2</a></li>
        <li><a href="/anime/stream/x/staffel-3">3</a></li>
        <li><a href="/anime/stream/x/staffel-1/episode-1" data-episode-id="501">1</a></li>
        <li><a href="/anime/stream/x/staffel-1/episode-2" data-episode-id="502">2</a></li>
      </ul>
    </div>"#;

    #[test]
    fn staffel_nav_filme_und_staffeln() {
        // Filme → 0, Staffeln numerisch, Episoden-Links (data-episode-id) ignoriert.
        assert_eq!(parse_season_nav(NAV_HTML), vec![0, 1, 2, 3]);
    }

    #[test]
    fn staffel_nav_sortiert_und_dedupliziert() {
        let html = r##"
        <div class="hosterSiteDirectNav"><ul>
          <li><a href="#">3</a></li>
          <li><a href="#">1</a></li>
          <li><a href="#">3</a></li>
          <li><a href="#">2</a></li>
        </ul></div>"##;
        assert_eq!(parse_season_nav(html), vec![1, 2, 3]);
    }

    #[test]
    fn staffel_nav_ohne_filme_tab() {
        let html = r##"
        <div class="hosterSiteDirectNav"><ul>
          <li><a href="#">1</a></li>
          <li><a href="#">2</a></li>
        </ul></div>"##;
        assert_eq!(parse_season_nav(html), vec![1, 2]);
    }

    #[test]
    fn staffel_nav_leer_oder_fremd_ist_leer() {
        assert!(parse_season_nav("").is_empty());
        // Nav fehlt → nichts; nicht-numerische Labels werden ignoriert.
        assert!(parse_season_nav(
            r##"<div class="hosterSiteDirectNav"><ul><li><a href="#">Staffel A</a></li></ul></div>"##
        )
        .is_empty());
    }
}
