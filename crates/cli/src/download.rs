//! `search`- und `download`-Unterbefehle.

use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use vindex_core::config::Settings;
use vindex_core::download::{download_url, DownloadPrefs};
use vindex_core::model::{Hoster, Language, Quality, BASE};
use vindex_core::{http, naming, site, watchstate};

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
    /// Entfällt, wenn `--batch` genutzt wird.
    pub url: Option<String>,
    /// Textdatei mit einer URL pro Zeile; lädt alle Episoden nacheinander.
    /// Leere Zeilen und mit `#` beginnende Zeilen werden übersprungen.
    #[arg(long, value_name = "DATEI")]
    pub batch: Option<PathBuf>,
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
    /// „keep watching": nächste Folge einer Serie laden. Erwartet eine Serien-
    /// (oder Episoden-)URL; geladen wird die Folge nach der zuletzt gemerkten,
    /// oder Staffel 1 / Episode 1, wenn für die Serie noch nichts gemerkt ist.
    /// Schließt sich mit direkter URL und `--batch` aus.
    #[arg(long = "continue", value_name = "SERIEN-URL")]
    pub continue_url: Option<String>,
}

/// Aus Optionen und Einstellungen aufgelöste Download-Vorgaben. Der Dateiname
/// hängt an der jeweiligen URL und wird erst pro Download ergänzt.
pub(crate) struct ResolvedPrefs {
    pub(crate) language: Language,
    hosters: Vec<Hoster>,
    max_quality: Quality,
    min_quality: Quality,
    out_dir: PathBuf,
    native_hls: bool,
}

impl ResolvedPrefs {
    /// Baut die vollständigen `DownloadPrefs` für eine konkrete URL.
    pub(crate) fn for_url(&self, url: &str) -> DownloadPrefs {
        DownloadPrefs {
            languages: vec![self.language],
            hosters: self.hosters.clone(),
            max_quality: self.max_quality,
            min_quality: self.min_quality,
            out_dir: self.out_dir.clone(),
            file_name: file_name_from_url(url),
            native_hls: self.native_hls,
        }
    }
}

/// Löst Sprache/Hoster/Qualität/Ziel einmalig aus Optionen und Einstellungen auf.
fn resolve_prefs(args: &DownloadArgs, settings: &Settings) -> anyhow::Result<ResolvedPrefs> {
    resolve_prefs_from(
        args.language.as_deref(),
        args.provider.as_deref(),
        args.quality.as_deref(),
        args.output_dir.clone(),
        settings,
    )
}

/// Gemeinsame Auflösung von Sprache/Hoster/Qualität/Ziel aus einzelnen Optionen —
/// von `download` **und** `watch` genutzt, damit beide dieselbe Fallback-Logik teilen.
pub(crate) fn resolve_prefs_from(
    language: Option<&str>,
    provider: Option<&str>,
    quality: Option<&str>,
    output_dir: Option<PathBuf>,
    settings: &Settings,
) -> anyhow::Result<ResolvedPrefs> {
    let language = match language {
        Some(l) => Language::from_str(l).map_err(|e| anyhow::anyhow!(e))?,
        None => settings.language,
    };
    let hosters = match provider {
        Some(p) => {
            let h = Hoster::from_str(p).map_err(|e| anyhow::anyhow!(e))?;
            let mut v = vec![h];
            v.extend(settings.fallback_order.iter().copied().filter(|x| *x != h));
            v
        }
        None => settings.effective_hosters(),
    };
    let max_quality = match quality {
        Some(q) => Quality::from_str(q).map_err(|e| anyhow::anyhow!(e))?,
        None => settings.max_quality,
    };
    let out_dir = output_dir.unwrap_or_else(|| settings.download_path.clone());
    Ok(ResolvedPrefs {
        language,
        hosters,
        max_quality,
        min_quality: settings.min_quality,
        out_dir,
        native_hls: settings.native_hls,
    })
}

pub async fn run_download(args: DownloadArgs) -> anyhow::Result<()> {
    let settings = Settings::load();
    let client = http::browser_client()?;
    let resolved = resolve_prefs(&args, &settings)?;

    // Genau eine Quelle: direkte URL, --batch ODER --continue.
    let modes =
        args.url.is_some() as u8 + args.batch.is_some() as u8 + args.continue_url.is_some() as u8;
    if modes > 1 {
        anyhow::bail!("Bitte genau eine Quelle angeben: URL, --batch ODER --continue.");
    }

    if let Some(url) = &args.url {
        let path = download_one(&client, &resolved, url).await?;
        println!("Fertig: {}", path.display());
        Ok(())
    } else if let Some(file) = &args.batch {
        run_batch(&client, &resolved, file, settings.download_delay_secs).await
    } else if let Some(series) = &args.continue_url {
        run_continue(&client, &resolved, series).await
    } else {
        anyhow::bail!("Keine URL angegeben. Bitte eine URL, --batch <datei> oder --continue <serien-url> nutzen.")
    }
}

/// „keep watching": lädt die Folge nach der zuletzt gemerkten (oder Staffel 1 /
/// Episode 1, wenn nichts gemerkt). Die nächste URL wird aus Slug + (Staffel,
/// Episode + 1) gebaut. Existiert die Folge nicht, meldet der Download das sauber
/// als Fehler (kein Panic).
async fn run_continue(
    client: &reqwest::Client,
    resolved: &ResolvedPrefs,
    series_url: &str,
) -> anyhow::Result<()> {
    let slug = site::slug_from_url(series_url)
        .ok_or_else(|| anyhow::anyhow!("Keine Serie in der URL erkannt: {series_url}"))?;
    let (staffel, episode) = watchstate::load(&watchstate::default_path())
        .next_after(&slug)
        .unwrap_or((1, 1));
    let url = watchstate::episode_url(series_url, staffel, episode)
        .ok_or_else(|| anyhow::anyhow!("Konnte keine Episoden-URL für „{slug}\" bilden."))?;
    println!("Weiter mit Staffel {staffel}, Episode {episode}: {url}");
    let path = download_one(client, resolved, &url)
        .await
        .map_err(|e| anyhow::anyhow!("{e} (existiert Staffel {staffel}, Episode {episode}?)"))?;
    println!("Fertig: {}", path.display());
    Ok(())
}

/// Lädt eine einzelne URL und meldet den Fortschritt. Fehler werden als
/// `anyhow`-Fehler zurückgegeben (kein Panic).
async fn download_one(
    client: &reqwest::Client,
    resolved: &ResolvedPrefs,
    url: &str,
) -> anyhow::Result<PathBuf> {
    let prefs = resolved.for_url(url);
    println!(
        "Lade „{}\" ({}, Hoster-Reihenfolge: {}) ...",
        prefs.file_name,
        resolved.language.as_str(),
        prefs
            .hosters
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let path = download_url(client, url, &prefs, None)
        .await
        .map_err(|e| anyhow::anyhow!("Download fehlgeschlagen: {e}"))?;
    record_watch_progress(url);
    Ok(path)
}

/// Aktualisiert nach erfolgreichem Episoden-Download den Watch-State (Slug +
/// Staffel + Episode aus der URL). Bewusst hier in der CLI statt in `core::download`:
/// der Store ist eine nutzerseitige Datei (Pfad in der Nutzer-Config), und der
/// `--continue`-Workflow lebt ohnehin in der CLI — so bleibt die Download-Engine
/// eine reine Library ohne Seiteneffekt auf eine Config-Datei.
///
/// Best-Effort: Film-URLs (kein Serien-Slug/Episode) und Schreibfehler werden still
/// übergangen, damit ein geglückter Download nicht an der Zustandsdatei scheitert.
fn record_watch_progress(url: &str) {
    if let Some((slug, staffel, episode)) = watchstate::episode_from_url(url) {
        let path = watchstate::default_path();
        let mut state = watchstate::load(&path);
        state.record(&slug, staffel, episode);
        if let Err(e) = watchstate::save(&path, &state) {
            eprintln!("Hinweis: Watch-State konnte nicht gespeichert werden: {e}");
        }
    }
}

/// Lädt alle URLs aus einer Episodendatei nacheinander. Ein Fehler bei einer
/// Episode bricht den Batch nicht ab; am Ende folgt eine Zusammenfassung.
async fn run_batch(
    client: &reqwest::Client,
    resolved: &ResolvedPrefs,
    file: &std::path::Path,
    delay_secs: u64,
) -> anyhow::Result<()> {
    let contents = std::fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("Batch-Datei {} nicht lesbar: {e}", file.display()))?;
    let urls = parse_batch_list(&contents);
    if urls.is_empty() {
        anyhow::bail!(
            "Batch-Datei {} enthält keine URLs (nur Leerzeilen/Kommentare?).",
            file.display()
        );
    }

    let total = urls.len();
    let mut failures: Vec<String> = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        println!("[{}/{}] {url}", i + 1, total);
        match download_one(client, resolved, url).await {
            Ok(path) => println!("Fertig: {}", path.display()),
            Err(e) => {
                eprintln!("Fehlgeschlagen: {url} — {e}");
                failures.push(url.clone());
            }
        }
        // Konfiguriertes Delay zwischen den Downloads einhalten (nicht nach dem letzten).
        if i + 1 < total && delay_secs > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        }
    }

    let ok = total - failures.len();
    println!(
        "\nZusammenfassung: {ok} erfolgreich, {} fehlgeschlagen.",
        failures.len()
    );
    if !failures.is_empty() {
        println!("Fehlgeschlagene URLs:");
        for url in &failures {
            println!("  {url}");
        }
    }
    Ok(())
}

/// Liest eine Episodenliste: eine URL pro Zeile. Leerzeilen und mit `#`
/// beginnende Kommentarzeilen werden übersprungen, Whitespace wird getrimmt.
pub fn parse_batch_list(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dateiname_episode_aus_aniworld_url() {
        assert_eq!(
            file_name_from_url("https://aniworld.to/anime/stream/one-piece/staffel-2/episode-15"),
            "one piece S02E15.mkv"
        );
    }

    #[test]
    fn dateiname_ohne_staffel_und_episode_nutzt_s01e01() {
        assert_eq!(
            file_name_from_url("https://aniworld.to/anime/stream/naruto"),
            "naruto S01E01.mkv"
        );
    }

    #[test]
    fn dateiname_serienstream_url() {
        assert_eq!(
            file_name_from_url("https://serienstream.to/serie/stream/dark/staffel-1/episode-3"),
            "dark S01E03.mkv"
        );
    }

    #[test]
    fn dateiname_filmo_film() {
        assert_eq!(
            file_name_from_url("https://filmo.to/movies/the-matrix"),
            "the matrix.mkv"
        );
    }

    #[test]
    fn dateiname_filmo_film_mit_query_und_pfad() {
        assert_eq!(
            file_name_from_url("https://filmo.to/movies/the-matrix?x=1"),
            "the matrix.mkv"
        );
        assert_eq!(
            file_name_from_url("https://filmo.to/movies/the-matrix/watch"),
            "the matrix.mkv"
        );
    }

    #[test]
    fn dateiname_filmo_ohne_slug_faellt_auf_movie_zurueck() {
        assert_eq!(file_name_from_url("https://filmo.to/movies/"), "movie.mkv");
        assert_eq!(file_name_from_url("https://filmo.to/"), "movie.mkv");
    }

    #[test]
    fn dateiname_unbekannte_url_faellt_auf_episode_zurueck() {
        assert_eq!(
            file_name_from_url("https://aniworld.to/"),
            "episode S01E01.mkv"
        );
    }

    #[test]
    fn capture_num_liest_zahl_zwischen_markern() {
        assert_eq!(
            capture_num("https://x/staffel-3/episode-12", "/staffel-"),
            Some(3)
        );
        assert_eq!(
            capture_num("https://x/staffel-3/episode-12", "/episode-"),
            Some(12)
        );
    }

    #[test]
    fn capture_num_marker_fehlt() {
        assert_eq!(capture_num("https://x/anime/stream/foo", "/staffel-"), None);
    }

    #[test]
    fn capture_num_keine_zahl() {
        assert_eq!(
            capture_num("https://x/staffel-abc/episode-1", "/staffel-"),
            None
        );
        assert_eq!(
            capture_num("https://x/staffel-/episode-1", "/staffel-"),
            None
        );
    }

    #[test]
    fn capture_num_ueberlauf_ist_none() {
        assert_eq!(
            capture_num("https://x/staffel-99999999999", "/staffel-"),
            None
        );
    }

    #[test]
    fn batch_liste_einfache_urls() {
        let input = "https://a.to/e1\nhttps://a.to/e2\n";
        assert_eq!(
            parse_batch_list(input),
            vec!["https://a.to/e1", "https://a.to/e2"]
        );
    }

    #[test]
    fn batch_liste_ueberspringt_leerzeilen() {
        let input = "https://a.to/e1\n\n\nhttps://a.to/e2\n";
        assert_eq!(
            parse_batch_list(input),
            vec!["https://a.to/e1", "https://a.to/e2"]
        );
    }

    #[test]
    fn batch_liste_ueberspringt_kommentare() {
        let input = "# Kommentar\nhttps://a.to/e1\n# noch einer\nhttps://a.to/e2\n";
        assert_eq!(
            parse_batch_list(input),
            vec!["https://a.to/e1", "https://a.to/e2"]
        );
    }

    #[test]
    fn batch_liste_trimmt_whitespace() {
        let input = "   https://a.to/e1   \n\t https://a.to/e2\t\n";
        assert_eq!(
            parse_batch_list(input),
            vec!["https://a.to/e1", "https://a.to/e2"]
        );
    }

    #[test]
    fn batch_liste_gemischt() {
        let input = "\
# Staffel 1
https://a.to/s1e1

   https://a.to/s1e2
#   https://a.to/ausgeblendet
\thttps://a.to/s1e3
";
        assert_eq!(
            parse_batch_list(input),
            vec![
                "https://a.to/s1e1",
                "https://a.to/s1e2",
                "https://a.to/s1e3",
            ]
        );
    }

    #[test]
    fn batch_liste_leer_ergibt_leere_liste() {
        assert!(parse_batch_list("").is_empty());
        assert!(parse_batch_list("\n\n# nur Kommentar\n   \n").is_empty());
    }
}
