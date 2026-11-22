//! `census`-Unterbefehl: Katalog zählen.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use vindex_core::http;
use vindex_core::model::Classified;
use vindex_core::site::aniworld;
use vindex_core::Kind;

#[derive(Parser, Debug)]
pub struct CensusArgs {
    /// Nur die ersten N Katalog-Einträge klassifizieren (zum Testen).
    #[arg(long)]
    limit: Option<usize>,

    /// Anzahl paralleler Requests für die Detailseiten.
    #[arg(long, default_value_t = 8)]
    concurrency: usize,

    /// Nur den Gesamtkatalog zählen, keine Detailseiten abrufen.
    #[arg(long)]
    catalog_only: bool,

    /// Zusätzlich alle Staffeln durchsuchen und alle Folgen zählen.
    #[arg(long)]
    episodes: bool,

    /// CSV-Datei mit dem Klassifizierungs-Ergebnis je Titel schreiben.
    #[arg(long)]
    output: Option<String>,

    /// Wartezeit zwischen Requests pro Worker in Millisekunden.
    #[arg(long, default_value_t = 150)]
    delay_ms: u64,
}

pub async fn run(args: CensusArgs) -> anyhow::Result<()> {
    let client = http::client()?;

    eprintln!("Lade Gesamtkatalog von aniworld.to/animes-alphabet ...");
    let mut entries = aniworld::fetch_catalog(&client).await?;
    println!("Katalog-Einträge gefunden: {}", entries.len());

    if let Some(limit) = args.limit {
        entries.truncate(limit);
        eprintln!("(--limit aktiv, beschränkt auf {} Einträge)", entries.len());
    }

    if args.catalog_only {
        println!("\n--catalog-only gesetzt: keine Detailklassifizierung durchgeführt.");
        println!(
            "Gesamtzahl Titel (Serien + Filme zusammen): {}",
            entries.len()
        );
        return Ok(());
    }

    let total = entries.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap(),
    );

    let semaphore = Arc::new(Semaphore::new(args.concurrency.max(1)));
    let movies = Arc::new(AtomicUsize::new(0));
    let series = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(AtomicUsize::new(0));

    let mut futs = FuturesUnordered::new();
    for entry in entries {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let movies = movies.clone();
        let series = series.clone();
        let errors = errors.clone();
        let pb = pb.clone();
        let delay_ms = args.delay_ms;
        let count_episodes = args.episodes;

        futs.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let result = aniworld::classify(&client, &entry, count_episodes).await;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            pb.inc(1);
            match &result {
                Ok(c) => match c.kind {
                    Kind::Movie => {
                        movies.fetch_add(1, Ordering::Relaxed);
                    }
                    Kind::Series => {
                        series.fetch_add(1, Ordering::Relaxed);
                    }
                    Kind::Unknown => {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                },
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            result.unwrap_or(Classified {
                title: entry.title.clone(),
                slug: entry.slug.clone(),
                seasons: 0,
                episodes_season1: 0,
                films: 0,
                total_episodes: 0,
                kind: Kind::Unknown,
            })
        }));
    }

    let mut results = Vec::with_capacity(total);
    while let Some(joined) = futs.next().await {
        if let Ok(classified) = joined {
            results.push(classified);
        }
    }
    pb.finish_and_clear();

    let movies = movies.load(Ordering::Relaxed);
    let series = series.load(Ordering::Relaxed);
    let errors = errors.load(Ordering::Relaxed);

    let films_total: usize = results.iter().map(|r| r.films).sum();
    let standalone_films = movies;
    let embedded_films = films_total.saturating_sub(standalone_films);
    let series_with_films = results
        .iter()
        .filter(|r| r.films > 0 && r.seasons >= 1 && !matches!(r.kind, Kind::Movie))
        .count();
    let total_seasons: usize = results.iter().map(|r| r.seasons).sum();

    println!("\n=== AniWorld.to Bestand ===");
    println!("Katalog-Titel gesamt:        {total}");
    println!("  davon Serien:              {series}");
    println!("  davon eigenständige Filme: {movies}");
    println!();
    println!("Filme gesamt:                {films_total}");
    println!("  eigenständige Film-Titel: {standalone_films}");
    println!("  Filme in Serien-Tabs:      {embedded_films}  (aus {series_with_films} Serien)");
    println!();
    println!("Staffeln gesamt (alle Titel): {total_seasons}");
    if args.episodes {
        let total_episodes: usize = results.iter().map(|r| r.total_episodes).sum();
        println!("Folgen gesamt (alle Staffeln): {total_episodes}");
    }
    if errors > 0 {
        println!("\n  nicht klassifizierbar:     {errors} (Netzwerkfehler o.ä., siehe --output)");
    }

    if let Some(path) = args.output {
        write_csv(&path, &results)?;
        println!("\nDetailliertes Ergebnis geschrieben nach: {path}");
    }

    Ok(())
}

fn write_csv(path: &str, results: &[Classified]) -> anyhow::Result<()> {
    let mut out = String::from("title,slug,seasons,episodes_season1,total_episodes,films,kind\n");
    for r in results {
        let kind = match r.kind {
            Kind::Movie => "movie",
            Kind::Series => "series",
            Kind::Unknown => "unknown",
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_escape(&r.title),
            csv_escape(&r.slug),
            r.seasons,
            r.episodes_season1,
            r.total_episodes,
            r.films,
            kind
        ));
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::csv_escape;

    #[test]
    fn csv_escape_einfaches_feld_unveraendert() {
        assert_eq!(csv_escape("One Piece"), "One Piece");
    }

    #[test]
    fn csv_escape_leerer_string() {
        assert_eq!(csv_escape(""), "");
    }

    #[test]
    fn csv_escape_komma_wird_gequotet() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
    }

    #[test]
    fn csv_escape_anfuehrungszeichen_werden_verdoppelt() {
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn csv_escape_komma_und_quote_kombiniert() {
        assert_eq!(csv_escape("a,\"b\""), "\"a,\"\"b\"\"\"");
    }
}
