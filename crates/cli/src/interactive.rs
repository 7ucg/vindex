//! `menu`-Unterbefehl: interaktiver Ablauf mit `dialoguer`.

use dialoguer::{Input, Select};
use vindex_core::config::Settings;
use vindex_core::download::{download_episode, DownloadPrefs};
use vindex_core::model::{default_fallback_order, Language};
use vindex_core::site::aniworld;
use vindex_core::{http, site};

pub async fn run() -> anyhow::Result<()> {
    let settings = Settings::load();
    let client = http::browser_client()?;

    // 1. Suchbegriff
    let keyword: String = Input::new().with_prompt("Suche (Titel)").interact_text()?;

    eprintln!("Lade Katalog ...");
    let entries = aniworld::fetch_catalog(&client).await?;
    let needle = keyword.to_lowercase();
    let hits: Vec<_> = entries
        .into_iter()
        .filter(|e| e.title.to_lowercase().contains(&needle))
        .take(40)
        .collect();
    if hits.is_empty() {
        println!("Keine Treffer.");
        return Ok(());
    }

    // 2. Titel wählen
    let titles: Vec<&str> = hits.iter().map(|e| e.title.as_str()).collect();
    let idx = Select::new()
        .with_prompt("Titel")
        .items(&titles)
        .default(0)
        .interact()?;
    let entry = &hits[idx];

    // 3. Staffel wählen
    eprintln!("Lade Staffeln ...");
    let seasons = aniworld::list_seasons(&client, &entry.slug).await?;
    if seasons.is_empty() {
        println!("Keine Staffeln gefunden.");
        return Ok(());
    }
    let season_labels: Vec<String> = seasons
        .iter()
        .map(|s| {
            if *s == 0 {
                "Filme".to_string()
            } else {
                format!("Staffel {s}")
            }
        })
        .collect();
    let s_idx = Select::new()
        .with_prompt("Staffel")
        .items(&season_labels)
        .default(0)
        .interact()?;
    let season = seasons[s_idx];

    // 4. Episode wählen
    eprintln!("Lade Episoden ...");
    let episodes = aniworld::list_episodes(&client, &entry.slug, season).await?;
    if episodes.is_empty() {
        println!("Keine Episoden gefunden.");
        return Ok(());
    }
    let ep_labels: Vec<String> = episodes
        .iter()
        .map(|e| format!("E{:02} — {}", e.index_in_season, e.title))
        .collect();
    let e_idx = Select::new()
        .with_prompt("Episode")
        .items(&ep_labels)
        .default(0)
        .interact()?;
    let episode = &episodes[e_idx];

    // 5. Sprache wählen (nur verfügbare)
    eprintln!("Lade Hoster/Sprachen ...");
    let base = site::base_for_url(&episode.url);
    let links = site::serienstream::provider_links(&client, base, &episode.url).await?;
    let mut langs: Vec<Language> = links.by_language.keys().copied().collect();
    langs.sort_by_key(|l| l.as_str());
    if langs.is_empty() {
        println!("Keine Sprachen verfügbar.");
        return Ok(());
    }
    let lang_labels: Vec<&str> = langs.iter().map(|l| l.as_str()).collect();
    let l_idx = Select::new()
        .with_prompt("Sprache")
        .items(&lang_labels)
        .default(0)
        .interact()?;
    let language = langs[l_idx];

    // 6. Download
    let file_name = crate::download::file_name_from_url(&episode.url);
    let prefs = DownloadPrefs {
        languages: vec![language],
        hosters: {
            // Nur tatsächlich verfügbare Hoster, in Default-Fallback-Reihenfolge.
            let avail = links.hosters_for(language).cloned().unwrap_or_default();
            default_fallback_order()
                .into_iter()
                .filter(|h| avail.contains_key(h))
                .collect()
        },
        max_quality: settings.max_quality,
        min_quality: settings.min_quality,
        out_dir: settings.download_path.clone(),
        file_name: file_name.clone(),
        native_hls: settings.native_hls,
        no_ffmpeg: settings.no_ffmpeg,
    };

    println!("Lade „{file_name}\" ({}) ...", language.as_str());
    let progress = crate::download::cli_progress_sink();
    match download_episode(&client, &episode.url, &prefs, None, Some(&progress)).await {
        Ok(path) => println!("Fertig: {}", path.display()),
        Err(e) => println!("Fehlgeschlagen: {e}"),
    }
    Ok(())
}
