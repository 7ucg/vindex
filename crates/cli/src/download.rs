//! `search`- und `download`-Unterbefehle.

use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use vindex_core::config::Settings;
use vindex_core::download::{download_url, DownloadPrefs};
use vindex_core::model::{Hoster, Language, Quality, BASE};
use vindex_core::{http, naming, site};

#[derive(Parser, Debug)]
pub struct SearchArgs {
    /// Suchbegriff (Teilstring im Titel).
    pub keyword: String,
    /// Maximale Trefferzahl.
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
}

pub async fn run_search(args: SearchArgs) -> anyhow::Result<()> {
    let client = http::client()?;
    eprintln!("Lade Katalog und filtere nach „{}\" ...", args.keyword);
    let entries = site::aniworld::fetch_catalog(&client).await?;
    let needle = args.keyword.to_lowercase();
    let mut hits = 0;
    for e in entries
        .iter()
        .filter(|e| e.title.to_lowercase().contains(&needle))
    {
        println!("{}\n    {}/anime/stream/{}", e.title, BASE, e.slug);
        hits += 1;
        if hits >= args.limit {
            break;
        }
    }
    if hits == 0 {
        println!("Keine Treffer.");
    }
    Ok(())
}

#[derive(Parser, Debug)]
pub struct DownloadArgs {
    /// Episoden- oder Film-URL (aniworld.to / s.to / filmo.to).
    pub url: String,
    /// Sprache (German Dub | English Sub | German Sub | English Dub).
    #[arg(long)]
    pub language: Option<String>,
    /// Bevorzugter Hoster (VOE, Vidmoly, Vidoza, Doodstream, Filemoon, SpeedFiles).
    #[arg(long)]
    pub provider: Option<String>,
    /// Höchste Qualität (360p..4K).
    #[arg(long)]
    pub quality: Option<String>,
    /// Zielverzeichnis (Default: Einstellung VINDEX_DOWNLOAD_PATH).
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

pub async fn run_download(args: DownloadArgs) -> anyhow::Result<()> {
    let settings = Settings::load();
    let client = http::browser_client()?;

    let language = match &args.language {
        Some(l) => Language::from_str(l).map_err(|e| anyhow::anyhow!(e))?,
        None => settings.language,
    };
    let hosters = match &args.provider {
        Some(p) => {
            let h = Hoster::from_str(p).map_err(|e| anyhow::anyhow!(e))?;
            let mut v = vec![h];
            v.extend(settings.fallback_order.iter().copied().filter(|x| *x != h));
            v
        }
        None => settings.effective_hosters(),
    };
    let max_quality = match &args.quality {
        Some(q) => Quality::from_str(q).map_err(|e| anyhow::anyhow!(e))?,
        None => settings.max_quality,
    };
    let out_dir = args.output_dir.unwrap_or(settings.download_path.clone());
    let file_name = file_name_from_url(&args.url);

    let prefs = DownloadPrefs {
        languages: vec![language],
        hosters,
        max_quality,
        min_quality: settings.min_quality,
        out_dir,
        file_name: file_name.clone(),
    };

    println!(
        "Lade „{file_name}\" ({}, Hoster-Reihenfolge: {}) ...",
        language.as_str(),
        prefs
            .hosters
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    match download_url(&client, &args.url, &prefs, None).await {
        Ok(path) => {
            println!("Fertig: {}", path.display());
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("Download fehlgeschlagen: {e}")),
    }
}

/// Leitet einen Dateinamen aus einer URL ab:
/// Episode → `Titel SxxEyy.mkv`, Filmo-Film → `Titel.mkv`.
pub fn file_name_from_url(url: &str) -> String {
    // Filmo: …/movies/<slug>
    if site::is_filmo(url) {
        if let Some(rest) = url.split("/movies/").nth(1) {
            let slug = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
            if !slug.is_empty() {
                return format!("{}.mkv", naming::clean_title(&slug.replace('-', " ")));
            }
        }
        return "movie.mkv".into();
    }
    let slug = site::slug_from_url(url).unwrap_or_else(|| "episode".into());
    let season = capture_num(url, "/staffel-").unwrap_or(1);
    let episode = capture_num(url, "/episode-").unwrap_or(1);
    let title = naming::clean_title(&slug.replace('-', " "));
    format!("{title} S{season:02}E{episode:02}.mkv")
}

fn capture_num(url: &str, marker: &str) -> Option<u32> {
    url.split(marker)
        .nth(1)?
        .split('/')
        .next()?
        .parse::<u32>()
        .ok()
}
