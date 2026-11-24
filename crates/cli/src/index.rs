//! `index`-Unterbefehl: lokaler Katalog-Index als SQLite-Datenbank.
//!
//! Baut pro Portal (aniworld / serienstream / filmo) einen vollständigen Bestand
//! aus Titeln, Metadaten, Staffeln, Folgen und den Hoster-Links der Portalseiten
//! auf. Die Portal-Links (`/redirect/N`, `/r?t=…`, Filmo-Token) sind stabil und
//! werden beim Crawl gespeichert; die eigentlichen Embed-URLs der Hoster sind
//! flüchtiger und werden in einem getrennten Durchlauf (`resolve`) aufgelöst.
//!
//! Der Aufbau ist fortsetzbar: bereits vollständig gecrawlte Titel (frischer als
//! `--refresh-days`) werden übersprungen, ein abgebrochener Lauf macht beim
//! nächsten Start dort weiter, wo er stand.

use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rusqlite::{params, Connection};
use tokio::sync::Semaphore;
use vindex_core::http;
use vindex_core::model::{Episode, Hoster, Language, BASE, BASE_STO};
use vindex_core::site::{self, filmo, serienstream};

#[derive(Parser, Debug)]
pub struct IndexArgs {
    #[command(subcommand)]
    command: IndexCommand,
}

#[derive(Subcommand, Debug)]
enum IndexCommand {
    /// Crawlt Kataloge, Metadaten, Folgen und Portal-Links in die Datenbank.
    Build(BuildArgs),
    /// Löst gespeicherte Portal-Links zu Hoster-Embed-URLs auf.
    Resolve(ResolveArgs),
    /// Zeigt den Bestand der Datenbank.
    Stats(StatsArgs),
    /// Teilt die DB pro Portal in komprimierte Schnappschüsse (`<portal>.db.zst`).
    Export(ExportArgs),
    /// Führt komprimierte Schnappschüsse in die lokale DB zusammen.
    Import(ImportArgs),
}

#[derive(Parser, Debug, Clone)]
struct BuildArgs {
    /// Ordner mit den drei Portal-DBs (`<portal>.db`).
    #[arg(long, default_value = "index")]
    dir: String,

    /// Portal: aniworld, serienstream, filmo oder all. Bei „all" laufen die drei
    /// Portale gleichzeitig (je eigene DB-Datei).
    #[arg(long, default_value = "all")]
    portal: String,

    /// Nur die ersten N Titel pro Portal (zum Testen).
    #[arg(long)]
    limit: Option<usize>,

    /// Parallele Titel-Crawls **pro Portal**. Bei „all" ×3 insgesamt, aber je
    /// Portal auf einen eigenen Host verteilt (nicht ein Host härter gehämmert).
    #[arg(long, default_value_t = 10)]
    concurrency: usize,

    /// Wartezeit zwischen Folgen-Abrufen pro Titel in Millisekunden.
    #[arg(long, default_value_t = 150)]
    delay_ms: u64,

    /// Titel neu crawlen, deren Stand älter ist als N Tage (0 = alles neu).
    #[arg(long, default_value_t = 30)]
    refresh_days: i64,
}

#[derive(Parser, Debug)]
struct ResolveArgs {
    /// Ordner mit den drei Portal-DBs (`<portal>.db`).
    #[arg(long, default_value = "index")]
    dir: String,

    /// Portal: aniworld, serienstream, filmo oder all.
    #[arg(long, default_value = "all")]
    portal: String,

    /// Höchstens N Links auflösen (0 = alle offenen).
    #[arg(long, default_value_t = 0)]
    max: usize,

    /// Parallele Auflösungen.
    #[arg(long, default_value_t = 6)]
    concurrency: usize,

    /// Wartezeit zwischen Auflösungen pro Worker in Millisekunden.
    #[arg(long, default_value_t = 200)]
    delay_ms: u64,

    /// Auch bereits aufgelöste Links erneuern, die älter sind als N Tage.
    #[arg(long)]
    refresh_days: Option<i64>,
}

#[derive(Parser, Debug)]
struct StatsArgs {
    /// Ordner mit den drei Portal-DBs (`<portal>.db`).
    #[arg(long, default_value = "index")]
    dir: String,
}

#[derive(Parser, Debug)]
struct ExportArgs {
    /// Ordner mit den Portal-DBs; die `.db.zst` werden daneben geschrieben.
    #[arg(long, default_value = "index")]
    dir: String,

    /// Portal: aniworld, serienstream, filmo oder all.
    #[arg(long, default_value = "all")]
    portal: String,

    /// zstd-Kompressionsstufe (1–22).
    #[arg(long, default_value_t = 19)]
    level: i32,
}

#[derive(Parser, Debug)]
struct ImportArgs {
    /// Ordner mit den `<portal>.db.zst`; die `.db` werden daneben entpackt.
    #[arg(long, default_value = "index")]
    dir: String,

    /// Portal: aniworld, serienstream, filmo oder all.
    #[arg(long, default_value = "all")]
    portal: String,
}

pub async fn run(args: IndexArgs) -> anyhow::Result<()> {
    match args.command {
        IndexCommand::Build(a) => build(a).await,
        IndexCommand::Resolve(a) => resolve(a).await,
        IndexCommand::Stats(a) => stats(a),
        IndexCommand::Export(a) => export(a),
        IndexCommand::Import(a) => import(a),
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

fn open_db(path: &str) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 30000;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS title (
           id INTEGER PRIMARY KEY,
           portal TEXT NOT NULL,
           slug TEXT NOT NULL,
           url TEXT NOT NULL,
           name TEXT NOT NULL,
           kind TEXT NOT NULL DEFAULT 'unknown',
           description TEXT,
           poster TEXT,
           year TEXT,
           fsk INTEGER,
           country TEXT,
           imdb TEXT,
           genres TEXT,
           directors TEXT,
           actors TEXT,
           producers TEXT,
           crawled_at INTEGER,
           UNIQUE(portal, slug)
         );
         CREATE TABLE IF NOT EXISTS episode (
           id INTEGER PRIMARY KEY,
           title_id INTEGER NOT NULL REFERENCES title(id) ON DELETE CASCADE,
           season INTEGER NOT NULL,
           number INTEGER NOT NULL,
           url TEXT NOT NULL,
           name TEXT,
           UNIQUE(title_id, season, number)
         );
         CREATE TABLE IF NOT EXISTS link (
           id INTEGER PRIMARY KEY,
           episode_id INTEGER NOT NULL REFERENCES episode(id) ON DELETE CASCADE,
           language TEXT NOT NULL,
           hoster TEXT NOT NULL,
           portal_url TEXT NOT NULL,
           embed_url TEXT,
           resolved_at INTEGER,
           fail_count INTEGER NOT NULL DEFAULT 0,
           UNIQUE(episode_id, language, hoster)
         );
         CREATE INDEX IF NOT EXISTS idx_title_portal ON title(portal);
         CREATE INDEX IF NOT EXISTS idx_episode_title ON episode(title_id);
         CREATE INDEX IF NOT EXISTS idx_link_episode ON link(episode_id);
         CREATE INDEX IF NOT EXISTS idx_link_open ON link(resolved_at) WHERE embed_url IS NULL;",
    )?;
    // Migration für bestehende DBs: fail_count nachrüsten (Fehler „duplicate column"
    // bei neuen DBs, die die Spalte schon haben, ignorieren).
    let _ = conn.execute(
        "ALTER TABLE link ADD COLUMN fail_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    Ok(conn)
}

/// Nach so vielen vergeblichen Auflöse-Versuchen eines Links gilt sein Portal-Link
/// als abgelaufen → der Titel wird zum Neu-Crawl markiert (holt einen frischen Link).
/// Schwelle > 1, damit einzelne Cloudflare-/Netz-Aussetzer das nicht auslösen.
const RESOLVE_FAIL_THRESHOLD: i64 = 3;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn portals_from_arg(portal: &str) -> anyhow::Result<Vec<&'static str>> {
    Ok(match portal {
        "all" => vec!["aniworld", "serienstream", "filmo"],
        "aniworld" => vec!["aniworld"],
        "serienstream" => vec!["serienstream"],
        "filmo" => vec!["filmo"],
        other => anyhow::bail!("unbekanntes Portal: {other} (aniworld|serienstream|filmo|all)"),
    })
}

/// Dateipfad der Portal-DB im Index-Ordner (`<dir>/<portal>.db`).
fn portal_db_path(dir: &str, portal: &str) -> String {
    format!("{dir}/{portal}.db")
}

/// Öffnet die Portal-DB (legt den Ordner bei Bedarf an).
fn open_portal_db(dir: &str, portal: &str) -> anyhow::Result<Connection> {
    std::fs::create_dir_all(dir)?;
    open_db(&portal_db_path(dir, portal))
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Ein Link einer Folge: (Sprache, Hoster, Portal-Link).
type CrawledLink = (String, String, String);
/// Eine Folge: (Staffel, Nummer, URL, Name, Links).
type CrawledEpisode = (u32, u32, String, String, Vec<CrawledLink>);

/// Ergebnis eines Titel-Crawls, bereit zum Wegschreiben.
struct CrawledTitle {
    slug: String,
    kind: &'static str,
    meta: site::TitleMeta,
    episodes: Vec<CrawledEpisode>,
    /// Folgen, deren Hoster-Links nicht abrufbar waren.
    errors: usize,
}

async fn build(args: BuildArgs) -> anyhow::Result<()> {
    let portals = portals_from_arg(&args.portal)?;
    let client = http::client()?;
    let mp = MultiProgress::new();

    // Die Portale laufen gleichzeitig: je eigene DB-Connection (SQLite-WAL +
    // busy_timeout serialisiert die kurzen Schreibzugriffe). `Connection` ist nicht
    // `Sync`, darum werden die Portal-Treiber nicht per `spawn` (Send nötig) gestartet,
    // sondern mit `join_all` nebenläufig auf einem Task getrieben — die eigentlichen
    // Titel-Crawls laufen intern weiterhin über `tokio::spawn` auf dem Threadpool.
    // Jedes Portal ist fortsetzbar; der Fortschritt steht nach jedem Titel in der DB,
    // bereits vollständig gecrawlte Titel werden nie erneut angefasst.
    let conns = portals
        .iter()
        .map(|p| open_portal_db(&args.dir, p))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let futs = portals
        .iter()
        .zip(conns.iter())
        .map(|(portal, conn)| build_portal(conn, &client, *portal, &args, &mp));
    for result in futures::future::join_all(futs).await {
        if let Err(e) = result {
            eprintln!("Portal-Fehler: {e}");
        }
    }
    Ok(())
}

async fn build_portal(
    conn: &Connection,
    client: &reqwest::Client,
    portal: &'static str,
    args: &BuildArgs,
    mp: &MultiProgress,
) -> anyhow::Result<()> {
    eprintln!("[{portal}] Katalog laden ...");
    let entries = match portal {
        "aniworld" => vindex_core::site::aniworld::fetch_catalog(client).await?,
        "serienstream" => serienstream::fetch_catalog(client, BASE_STO).await?,
        "filmo" => fetch_filmo_catalog(client).await?,
        _ => unreachable!(),
    };
    eprintln!("[{portal}] {} Titel im Katalog", entries.len());

    // Katalog-Stand eintragen (Name/URL aktualisieren, Crawl-Stand bleibt). In einer
    // `BEGIN IMMEDIATE`-Transaktion — schnell, kurzer Schreib-Lock (parallele Portale
    // warten nur ganz kurz via busy_timeout), der `crawled_at`-Stempel bleibt unberührt.
    {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let insert = (|| -> anyhow::Result<()> {
            let mut stmt = conn.prepare(
                "INSERT INTO title (portal, slug, url, name) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(portal, slug) DO UPDATE SET name = excluded.name, url = excluded.url",
            )?;
            for e in &entries {
                stmt.execute(params![portal, e.slug, title_url(portal, &e.slug), e.title])?;
            }
            Ok(())
        })();
        match insert {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }

    // Zu crawlende Titel bestimmen (fortsetzbar).
    let cutoff = if args.refresh_days <= 0 {
        i64::MAX
    } else {
        now_unix() - args.refresh_days * 86_400
    };
    let mut pending: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, slug FROM title
             WHERE portal = ?1 AND (crawled_at IS NULL OR crawled_at < ?2)
             ORDER BY slug",
        )?;
        let rows = stmt.query_map(params![portal, cutoff], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    if let Some(limit) = args.limit {
        pending.truncate(limit);
    }
    if pending.is_empty() {
        eprintln!("[{portal}] alles aktuell, nichts zu tun");
        return Ok(());
    }
    eprintln!("[{portal}] {} Titel zu crawlen", pending.len());

    let pb = mp.add(ProgressBar::new(pending.len() as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:>13} {spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap(),
    );
    pb.set_prefix(portal.to_string());

    let semaphore = Arc::new(Semaphore::new(args.concurrency.max(1)));
    let mut futs = FuturesUnordered::new();
    for (title_id, slug) in pending {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let delay = Duration::from_millis(args.delay_ms);
        futs.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let result = match portal {
                "filmo" => crawl_filmo_title(&client, &slug, delay).await,
                _ => crawl_sst_title(&client, portal, &slug, delay).await,
            };
            (title_id, slug, result)
        }));
    }

    let (mut ok, mut partial, mut failed) = (0usize, 0usize, 0usize);
    while let Some(joined) = futs.next().await {
        let Ok((title_id, slug, result)) = joined else {
            failed += 1;
            pb.inc(1);
            continue;
        };
        pb.inc(1);
        match result {
            Ok(crawled) => {
                if let Err(e) = store_title(conn, title_id, &crawled) {
                    pb.suspend(|| eprintln!("[{portal}] {slug}: Schreiben fehlgeschlagen: {e}"));
                    failed += 1;
                } else if crawled.errors > 0 {
                    partial += 1;
                } else {
                    ok += 1;
                }
            }
            Err(e) => {
                failed += 1;
                pb.suspend(|| eprintln!("[{portal}] {slug}: {e}"));
            }
        }
        pb.set_message(format!("ok {ok} / teils {partial} / fehlge. {failed}"));
    }
    pb.finish_and_clear();
    eprintln!(
        "[{portal}] fertig: {ok} vollständig, {partial} mit Lücken (werden beim nächsten Lauf \
         nachgeholt), {failed} fehlgeschlagen"
    );
    Ok(())
}

fn title_url(portal: &str, slug: &str) -> String {
    match portal {
        "aniworld" => format!("{BASE}/anime/stream/{slug}"),
        "serienstream" => format!("{BASE_STO}/serie/{slug}"),
        _ => format!("{}/movies/{slug}", filmo::BASE),
    }
}

fn base_for_portal(portal: &str) -> &'static str {
    match portal {
        "aniworld" => BASE,
        "serienstream" => BASE_STO,
        _ => filmo::BASE,
    }
}

/// Crawlt einen aniworld-/serienstream-Titel: Metadaten, Staffeln, Folgen, Links.
async fn crawl_sst_title(
    client: &reqwest::Client,
    portal: &str,
    slug: &str,
    delay: Duration,
) -> anyhow::Result<CrawledTitle> {
    let base = base_for_portal(portal);
    let url = title_url(portal, slug);
    let meta = site::title_meta(client, &url).await.unwrap_or_default();
    let seasons = serienstream::list_seasons(client, base, slug).await?;
    anyhow::ensure!(!seasons.is_empty(), "keine Staffeln gefunden");

    let kind = if seasons == [0] { "movie" } else { "series" };
    let mut episodes = Vec::new();
    let mut errors = 0usize;
    for season in seasons {
        let eps: Vec<Episode> = serienstream::list_episodes(client, base, slug, season).await?;
        for ep in eps {
            tokio::time::sleep(delay).await;
            let number = ep.explicit_number.unwrap_or(ep.index_in_season);
            let mut links = Vec::new();
            match serienstream::provider_links(client, base, &ep.url).await {
                Ok(pl) => {
                    for (lang, hosters) in &pl.by_language {
                        for (hoster, link) in hosters {
                            let abs = if link.starts_with('/') {
                                format!("{base}{link}")
                            } else {
                                link.clone()
                            };
                            links.push((
                                lang.ui_label().to_string(),
                                hoster.as_str().to_string(),
                                abs,
                            ));
                        }
                    }
                }
                Err(_) => errors += 1,
            }
            episodes.push((ep.season, number, ep.url.clone(), ep.title.clone(), links));
        }
    }
    Ok(CrawledTitle {
        slug: slug.to_string(),
        kind,
        meta,
        episodes,
        errors,
    })
}

/// Crawlt einen Filmo-Film: Seite, Metadaten und die Hoster-Token je Sprache.
async fn crawl_filmo_title(
    client: &reqwest::Client,
    slug: &str,
    delay: Duration,
) -> anyhow::Result<CrawledTitle> {
    let url = title_url("filmo", slug);
    let movie = filmo::movie(client, &url).await?;
    tokio::time::sleep(delay).await;
    let mut meta = site::title_meta(client, &url).await.unwrap_or_default();
    if meta.year.is_none() {
        meta.year = movie.year.map(|y| y.to_string());
    }

    let mut links = Vec::new();
    for (lang, hosters) in &movie.sources {
        for (hoster, token) in hosters {
            links.push((
                lang.ui_label().to_string(),
                hoster.as_str().to_string(),
                token.clone(),
            ));
        }
    }
    Ok(CrawledTitle {
        slug: slug.to_string(),
        kind: "movie",
        meta,
        episodes: vec![(0, 1, url, movie.title, links)],
        errors: 0,
    })
}

/// Schreibt einen gecrawlten Titel in einer Transaktion weg.
fn store_title(conn: &Connection, title_id: i64, crawled: &CrawledTitle) -> anyhow::Result<()> {
    // `BEGIN IMMEDIATE` (nicht DEFERRED): holt den Schreib-Lock sofort, damit bei
    // parallelen Portalen `busy_timeout` sauber wartet, statt „database is locked"
    // (eine DEFERRED-Transaktion, die mitten drin von Lese- auf Schreib-Lock hochstuft,
    // bekommt sofort BUSY ohne Warteschleife).
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = store_title_inner(conn, title_id, crawled);
    if result.is_ok() {
        conn.execute_batch("COMMIT")?;
    } else {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

fn store_title_inner(
    conn: &Connection,
    title_id: i64,
    crawled: &CrawledTitle,
) -> anyhow::Result<()> {
    let m = &crawled.meta;
    let json = |v: &Vec<String>| serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
    // Titel mit Lücken (Folgen ohne abrufbare Links) bleiben ohne Crawl-Stempel,
    // damit der nächste Lauf sie erneut versucht — die Daten stehen trotzdem drin.
    let crawled_at: Option<i64> = (crawled.errors == 0).then(now_unix);
    conn.execute(
        "UPDATE title SET kind = ?2, description = ?3, poster = ?4, year = ?5, fsk = ?6,
         country = ?7, imdb = ?8, genres = ?9, directors = ?10, actors = ?11, producers = ?12,
         crawled_at = COALESCE(?13, crawled_at) WHERE id = ?1",
        params![
            title_id,
            crawled.kind,
            m.description,
            m.poster,
            m.year,
            m.fsk,
            m.country,
            m.imdb,
            json(&m.genres),
            json(&m.directors),
            json(&m.actors),
            json(&m.producers),
            crawled_at,
        ],
    )?;

    let mut ep_stmt = conn.prepare_cached(
        "INSERT INTO episode (title_id, season, number, url, name) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(title_id, season, number) DO UPDATE SET url = excluded.url,
         name = excluded.name
         RETURNING id",
    )?;
    // Bei unverändertem Portal-Link bleibt die aufgelöste Embed-URL stehen; ändert
    // sich der Link, wird die Auflösung verworfen und beim nächsten `resolve` neu
    // geholt.
    let mut link_stmt = conn.prepare_cached(
        "INSERT INTO link (episode_id, language, hoster, portal_url) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(episode_id, language, hoster) DO UPDATE SET
           embed_url = CASE WHEN link.portal_url = excluded.portal_url
                            THEN link.embed_url ELSE NULL END,
           resolved_at = CASE WHEN link.portal_url = excluded.portal_url
                              THEN link.resolved_at ELSE NULL END,
           fail_count = CASE WHEN link.portal_url = excluded.portal_url
                             THEN link.fail_count ELSE 0 END,
           portal_url = excluded.portal_url",
    )?;
    for (season, number, url, name, links) in &crawled.episodes {
        let episode_id: i64 =
            ep_stmt.query_row(params![title_id, season, number, url, name], |r| r.get(0))?;
        for (lang, hoster, portal_url) in links {
            link_stmt.execute(params![episode_id, lang, hoster, portal_url])?;
        }
    }
    let _ = crawled.slug;
    Ok(())
}

/// Lädt den Filmo-Katalog über alle Seiten (42 Filme je Seite), blockweise parallel.
async fn fetch_filmo_catalog(
    client: &reqwest::Client,
) -> anyhow::Result<Vec<vindex_core::model::CatalogEntry>> {
    let mut all = Vec::new();
    let mut page: u32 = 1;
    loop {
        let batch: Vec<_> = (page..page + 8)
            .map(|p| filmo::fetch_catalog(client, p))
            .collect();
        let results = futures::future::join_all(batch).await;
        let mut done = false;
        for r in results {
            let entries = r?;
            if entries.is_empty() {
                done = true;
                break;
            }
            all.extend(entries);
        }
        if done || page > 1000 {
            break;
        }
        page += 8;
    }
    // Seiten können sich beim Crawlen verschieben — Duplikate entfernen.
    let mut seen = std::collections::HashSet::new();
    all.retain(|e| seen.insert(e.slug.clone()));
    Ok(all)
}

// ---------------------------------------------------------------------------
// Resolve
// ---------------------------------------------------------------------------

struct PendingLink {
    id: i64,
    /// Nur zur Dokumentation mitgeladen; gefiltert wird in der Query per `title.portal`.
    #[allow(dead_code)]
    portal: String,
    language: String,
    hoster: String,
    portal_url: String,
    episode_url: String,
}

async fn resolve(args: ResolveArgs) -> anyhow::Result<()> {
    let portals = portals_from_arg(&args.portal)?;
    let client = http::client()?;
    let mp = MultiProgress::new();

    // Die Portale lösen gleichzeitig auf (3 Hosts parallel, jeder mit eigener
    // Concurrency — nicht ein Host härter belastet). Je Portal eine eigene
    // DB-Connection; die Schreibzugriffe sind einzelne UPDATEs und serialisieren
    // über busy_timeout. Fortsetzbar: nur offene Links (embed_url IS NULL).
    let conns = portals
        .iter()
        .map(|p| open_portal_db(&args.dir, p))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let futs = portals
        .iter()
        .zip(conns.iter())
        .map(|(portal, conn)| resolve_portal(conn, &client, portal, &args, &mp));
    for result in futures::future::join_all(futs).await {
        if let Err(e) = result {
            eprintln!("Resolve-Fehler: {e}");
        }
    }
    Ok(())
}

async fn resolve_portal(
    conn: &Connection,
    client: &reqwest::Client,
    portal: &str,
    args: &ResolveArgs,
    mp: &MultiProgress,
) -> anyhow::Result<()> {
    let refresh_cutoff = args.refresh_days.map(|d| now_unix() - d * 86_400);
    let pending: Vec<PendingLink> = {
        let mut stmt = conn.prepare(
            "SELECT link.id, title.portal, link.language, link.hoster, link.portal_url,
                    episode.url
             FROM link
             JOIN episode ON episode.id = link.episode_id
             JOIN title ON title.id = episode.title_id
             WHERE title.portal = ?1 AND (link.embed_url IS NULL OR link.resolved_at < ?2)
             ORDER BY episode.url",
        )?;
        let rows = stmt.query_map(params![portal, refresh_cutoff.unwrap_or(0)], |r| {
            Ok(PendingLink {
                id: r.get(0)?,
                portal: r.get(1)?,
                language: r.get(2)?,
                hoster: r.get(3)?,
                portal_url: r.get(4)?,
                episode_url: r.get(5)?,
            })
        })?;
        let mut v: Vec<PendingLink> = rows.collect::<Result<_, _>>()?;
        if args.max > 0 {
            v.truncate(args.max);
        }
        v
    };
    if pending.is_empty() {
        eprintln!("[{portal}] nichts aufzulösen");
        return Ok(());
    }
    eprintln!("[{portal}] {} Links aufzulösen", pending.len());

    let pb = mp.add(ProgressBar::new(pending.len() as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:>13} {spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap(),
    );
    pb.set_prefix(portal.to_string());

    // Filmo-Links laufen pro Film gebündelt (eine Seite liefert CSRF + frische
    // Token für alle Hoster); die Portal-Redirects laufen einzeln.
    let mut filmo_by_movie: std::collections::HashMap<String, Vec<PendingLink>> =
        std::collections::HashMap::new();
    // serienstream ebenfalls pro Folge gebündelt: die gespeicherten /r?t=-Tokens laufen
    // ab (410), daher wird die Episodenseite beim Auflösen frisch geladen.
    let mut sto_by_episode: std::collections::HashMap<String, Vec<PendingLink>> =
        std::collections::HashMap::new();
    let mut direct = Vec::new();
    for p in pending {
        match portal {
            "filmo" => filmo_by_movie
                .entry(p.episode_url.clone())
                .or_default()
                .push(p),
            "serienstream" => sto_by_episode
                .entry(p.episode_url.clone())
                .or_default()
                .push(p),
            _ => direct.push(p),
        }
    }

    let semaphore = Arc::new(Semaphore::new(args.concurrency.max(1)));
    let mut futs = FuturesUnordered::new();
    for p in direct {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let delay = Duration::from_millis(args.delay_ms);
        futs.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            tokio::time::sleep(delay).await;
            let result = follow_to_embed(&client, &p.portal_url, &p.episode_url).await;
            vec![(p.id, result)]
        }));
    }
    for (movie_url, links) in filmo_by_movie {
        let semaphore = semaphore.clone();
        let delay = Duration::from_millis(args.delay_ms);
        futs.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            tokio::time::sleep(delay).await;
            // filmo.to bindet die Laravel-Session ans CSRF-Token: Film-GET und
            // `/n`-POST brauchen einen eigenen, kurzlebigen Cookie-Client.
            let fclient = match http::cookie_client() {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("Cookie-Client: {e}");
                    return links
                        .into_iter()
                        .map(|l| (l.id, Err(anyhow::anyhow!(msg.clone()))))
                        .collect();
                }
            };
            resolve_filmo_movie(&fclient, &movie_url, links).await
        }));
    }
    for (episode_url, links) in sto_by_episode {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let delay = Duration::from_millis(args.delay_ms);
        futs.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            tokio::time::sleep(delay).await;
            resolve_serienstream_episode(&client, &episode_url, links).await
        }));
    }

    let (mut ok, mut failed) = (0usize, 0usize);
    while let Some(joined) = futs.next().await {
        let Ok(results) = joined else {
            failed += 1;
            pb.inc(1);
            continue;
        };
        for (link_id, result) in results {
            pb.inc(1);
            match result {
                Ok(embed) => {
                    conn.execute(
                        "UPDATE link SET embed_url = ?2, resolved_at = ?3, fail_count = 0 \
                         WHERE id = ?1",
                        params![link_id, embed, now_unix()],
                    )?;
                    ok += 1;
                }
                Err(e) => {
                    failed += 1;
                    // Fehlversuch zählen; ab der Schwelle gilt der Portal-Link als
                    // abgelaufen → Titel zum Neu-Crawl markieren (frischer Link folgt
                    // beim nächsten `build`, der dann auch das Embed neu auflösen lässt).
                    conn.execute(
                        "UPDATE link SET fail_count = fail_count + 1, resolved_at = ?2 \
                         WHERE id = ?1",
                        params![link_id, now_unix()],
                    )?;
                    let fc: i64 = conn
                        .query_row(
                            "SELECT fail_count FROM link WHERE id = ?1",
                            [link_id],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);
                    // Genau an der Schwelle **einmal** auslösen (nicht `>=`): danach
                    // steigt fail_count darüber und matcht nicht mehr, ein unveränderter
                    // (dauerhaft toter) Link kann so nicht jeden Lauf einen Titel-Neu-
                    // Crawl erzwingen. Erst ein geänderter portal_url setzt fail_count
                    // beim nächsten `build` auf 0 und gibt einem frischen Token erneut
                    // 3 faire Versuche (Verstärkung ausgeschlossen).
                    if fc == RESOLVE_FAIL_THRESHOLD {
                        conn.execute(
                            "UPDATE title SET crawled_at = NULL WHERE id = (
                                 SELECT episode.title_id FROM episode
                                 JOIN link ON link.episode_id = episode.id
                                 WHERE link.id = ?1)",
                            [link_id],
                        )?;
                    }
                    pb.suspend(|| eprintln!("[{portal}] Link {link_id}: {e}"));
                }
            }
            pb.set_message(format!("ok {ok} / fehlge. {failed}"));
        }
    }
    pb.finish_and_clear();
    eprintln!("[{portal}] aufgelöst: {ok}, fehlgeschlagen: {failed}");
    Ok(())
}

/// Prüft die finale URL eines Redirects: http(s) und nicht mehr auf einem Portal
/// (sonst Cloudflare-Challenge statt echtem Hoster-Embed).
fn check_embed_url(final_url: String) -> anyhow::Result<String> {
    // Strikt parsen (Fehler statt leerem Host bei Parse-Fehler) und userinfo ablehnen.
    let u = reqwest::Url::parse(&final_url)
        .map_err(|_| anyhow::anyhow!("Redirect-Ziel nicht parsebar"))?;
    anyhow::ensure!(
        matches!(u.scheme(), "http" | "https"),
        "Redirect-Ziel ist kein http(s)"
    );
    anyhow::ensure!(
        u.username().is_empty() && u.password().is_none(),
        "Redirect-Ziel mit Zugangsdaten"
    );
    // Schluss-Punkt (FQDN) trimmen, sonst umginge „serienstream.to." den Vergleich.
    let host = u
        .host_str()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    // Portal-Hosts ablehnen (dann blieb der Redirect auf der Challenge-Seite hängen).
    // Label-basiert: jedes Label „aniworld/serienstream/filmo" (auch Mirror-TLDs,
    // Subdomains) plus das kurze s.to. Über-Ablehnung ist hier die sichere Richtung.
    let labels: Vec<&str> = host.split('.').collect();
    let on_portal = labels
        .iter()
        .any(|l| matches!(*l, "aniworld" | "serienstream" | "filmo"))
        || host == "s.to"
        || host.ends_with(".s.to");
    anyhow::ensure!(
        !on_portal,
        "Redirect blieb auf dem Portal (vermutlich Challenge)"
    );
    Ok(final_url)
}

/// Folgt einem Portal-Redirect (`/redirect/N`, `/r?t=…`) zur Hoster-Embed-URL.
///
/// serienstream steht hinter Cloudflare — dort wird direkt über den TLS-Impersonate-
/// Client aufgelöst (wie beim `build`), sonst würde der Redirect auf der Challenge-Seite
/// hängen bleiben. Für die anderen Portale genügt der normale Client; bleibt er
/// ausnahmsweise auf dem Portal (Challenge), gibt es einen Impersonate-Nachschlag.
async fn follow_to_embed(
    client: &reqwest::Client,
    portal_url: &str,
    referer: &str,
) -> anyhow::Result<String> {
    // SSRF-Schutz: das Ziel stammt aus gescraptem Inhalt. http(s) erzwingen und
    // gesperrte IP-Literale abweisen (Hostnamen deckt der DNS-Guard-Resolver ab).
    anyhow::ensure!(
        (portal_url.starts_with("http://") || portal_url.starts_with("https://"))
            && !http::host_is_forbidden_ip_literal(portal_url),
        "unerlaubtes Ziel"
    );

    // Rate-Limit-sanft: bei 429/503 (Hoster „Too many requests") oder einem Sende-Fehler
    // (Verbindungsabbruch unter Last) kurz warten und erneut versuchen. Das Warten hält
    // den Semaphore-Platz → drosselt sich bei einer Fehlerwelle von selbst.
    const MAX_RETRIES: u32 = 4;
    const BACKOFF: Duration = Duration::from_secs(10);
    let mut attempt = 0u32;

    let final_url = loop {
        match client
            .get(portal_url)
            .header("Referer", referer)
            .send()
            .await
        {
            Ok(resp) => {
                let s = resp.status().as_u16();
                if (s == 429 || s == 503) && attempt < MAX_RETRIES {
                    attempt += 1;
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
                anyhow::ensure!(
                    s != 429 && s != 503,
                    "Rate-Limit ({s}) nach {MAX_RETRIES} Versuchen"
                );
                break resp.url().to_string();
            }
            Err(e) => {
                if attempt < MAX_RETRIES {
                    attempt += 1;
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
                return Err(e.into());
            }
        }
    };

    match check_embed_url(final_url) {
        Ok(u) => Ok(u),
        Err(e) => {
            // Blieb auf dem Portal (evtl. Cloudflare-Challenge) → ein Nachschlag über
            // den TLS-Impersonate-Client. (Abgelaufene Tokens liefern dagegen 410 — die
            // fängt für serienstream der Frisch-Holen-Pfad ab, s. resolve_serienstream_episode.)
            match http::final_url_impersonate(portal_url, Some(referer)).await {
                Ok(imp) => check_embed_url(imp).map_err(|_| e),
                Err(_) => Err(e),
            }
        }
    }
}

/// Löst die offenen Links einer serienstream-Folge auf, indem die Episodenseite
/// **frisch** geladen wird: die gespeicherten `/r?t=`-Tokens (Laravel-Crypt) laufen
/// nach Stunden ab (→ HTTP 410), die frischen funktionieren. Wie bei filmo.
async fn resolve_serienstream_episode(
    _client: &reqwest::Client,
    episode_url: &str,
    links: Vec<PendingLink>,
) -> Vec<(i64, anyhow::Result<String>)> {
    // Eigener Cookie-Client pro Folge: der Seitenabruf setzt die Cloudflare-Clearance-
    // Cookies, die der anschließende `/r?t=`-Follow braucht (sonst Challenge).
    let client = match http::cookie_client() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Cookie-Client: {e}");
            return links
                .into_iter()
                .map(|l| (l.id, Err(anyhow::anyhow!(msg.clone()))))
                .collect();
        }
    };
    let pl = match serienstream::provider_links(&client, BASE_STO, episode_url).await {
        Ok(pl) => pl,
        Err(e) => {
            let msg = format!("Episodenseite nicht ladbar: {e}");
            return links
                .into_iter()
                .map(|l| (l.id, Err(anyhow::anyhow!(msg.clone()))))
                .collect();
        }
    };
    // Frische Play-URLs nach (Sprache-ui_label, Hoster-Name) indexieren — gleiche
    // Schlüssel, mit denen `build` die Links gespeichert hat.
    let mut fresh: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    for (lang, hosters) in &pl.by_language {
        for (hoster, link) in hosters {
            let abs = if link.starts_with('/') {
                format!("{BASE_STO}{link}")
            } else {
                link.clone()
            };
            fresh.insert(
                (lang.ui_label().to_string(), hoster.as_str().to_string()),
                abs,
            );
        }
    }
    let mut out = Vec::new();
    for l in links {
        match fresh.get(&(l.language.clone(), l.hoster.clone())) {
            Some(fresh_url) => {
                let r = follow_to_embed(&client, fresh_url, episode_url).await;
                out.push((l.id, r));
            }
            None => out.push((
                l.id,
                Err(anyhow::anyhow!("Hoster/Sprache nicht mehr angeboten")),
            )),
        }
    }
    out
}

/// Löst alle offenen Links eines Filmo-Films mit frischem CSRF + Token auf.
async fn resolve_filmo_movie(
    client: &reqwest::Client,
    movie_url: &str,
    links: Vec<PendingLink>,
) -> Vec<(i64, anyhow::Result<String>)> {
    let movie = match filmo::movie(client, movie_url).await {
        Ok(m) => m,
        Err(e) => {
            let msg = format!("Filmseite nicht ladbar: {e}");
            return links
                .into_iter()
                .map(|l| (l.id, Err(anyhow::anyhow!(msg.clone()))))
                .collect();
        }
    };
    let Some(csrf) = movie.csrf.clone() else {
        return links
            .into_iter()
            .map(|l| {
                (
                    l.id,
                    Err(anyhow::anyhow!("kein CSRF-Token auf der Filmseite")),
                )
            })
            .collect();
    };

    let mut out = Vec::new();
    for l in links {
        let lang = l.language.parse::<Language>().ok();
        let hoster = l.hoster.parse::<Hoster>().ok();
        let token = lang
            .zip(hoster)
            .and_then(|(lang, hoster)| movie.sources.get(&lang).and_then(|h| h.get(&hoster)))
            .cloned();
        let result = match token {
            Some(token) => filmo::resolve_embed(client, movie_url, &csrf, &token)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            None => Err(anyhow::anyhow!(
                "Hoster auf der Filmseite nicht mehr gelistet"
            )),
        };
        out.push((l.id, result));
    }
    out
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

fn stats(args: StatsArgs) -> anyhow::Result<()> {
    println!(
        "{:<14} {:>7} {:>9} {:>8} {:>8} {:>10}",
        "Portal", "Titel", "gecrawlt", "Folgen", "Links", "aufgelöst"
    );
    for portal in ["aniworld", "serienstream", "filmo"] {
        let path = portal_db_path(&args.dir, portal);
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        let conn = open_db(&path)?;
        let row = conn.query_row(
            "SELECT COUNT(DISTINCT title.id),
                    COUNT(DISTINCT CASE WHEN title.crawled_at IS NOT NULL THEN title.id END),
                    COUNT(DISTINCT episode.id),
                    COUNT(DISTINCT link.id),
                    COUNT(DISTINCT CASE WHEN link.embed_url IS NOT NULL THEN link.id END)
             FROM title
             LEFT JOIN episode ON episode.title_id = title.id
             LEFT JOIN link ON link.episode_id = episode.id",
            [],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )?;
        let (titles, crawled, episodes, links, resolved) = row;
        println!("{portal:<14} {titles:>7} {crawled:>9} {episodes:>8} {links:>8} {resolved:>10}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Export / Import (komprimierte Portal-Schnappschüsse zum Teilen)
// ---------------------------------------------------------------------------

/// Komprimiert eine Datei mit zstd (Streaming, kein Voll-Einlesen).
fn zstd_compress(src: &str, dst: &str, level: i32) -> anyhow::Result<()> {
    use std::io::Write;
    let input = std::io::BufReader::new(std::fs::File::open(src)?);
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    zstd::stream::copy_encode(input, &mut out, level)?;
    out.flush()?;
    Ok(())
}

/// Entpackt eine zstd-Datei (großes Fenster erlaubt, falls mit `--long` gepackt).
fn zstd_decompress(src: &str, dst: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let input = std::io::BufReader::new(std::fs::File::open(src)?);
    let mut dec = zstd::stream::Decoder::new(input)?;
    dec.window_log_max(31)?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    std::io::copy(&mut dec, &mut out)?;
    out.flush()?;
    Ok(())
}

fn export(args: ExportArgs) -> anyhow::Result<()> {
    let portals = portals_from_arg(&args.portal)?;
    for portal in portals {
        let db = portal_db_path(&args.dir, portal);
        if !std::path::Path::new(&db).exists() {
            eprintln!("[{portal}] keine DB ({db}) — übersprungen");
            continue;
        }
        // WAL in die Hauptdatei schreiben, damit der Schnappschuss vollständig ist.
        {
            let conn = open_db(&db)?;
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        let zst = format!("{db}.zst");
        zstd_compress(&db, &zst, args.level)?;
        let sz = std::fs::metadata(&zst)?.len();
        eprintln!("[{portal}] {zst} ({:.1} MB)", sz as f64 / 1_048_576.0);
    }
    Ok(())
}

fn import(args: ImportArgs) -> anyhow::Result<()> {
    let portals = portals_from_arg(&args.portal)?;
    std::fs::create_dir_all(&args.dir)?;
    for portal in portals {
        let db = portal_db_path(&args.dir, portal);
        let zst = format!("{db}.zst");
        if !std::path::Path::new(&zst).exists() {
            eprintln!("[{portal}] kein Schnappschuss ({zst}) — übersprungen");
            continue;
        }
        zstd_decompress(&zst, &db)?;
        // Alte WAL-/SHM-Sidecars neben der frisch entpackten DB würden sie verfälschen.
        let _ = std::fs::remove_file(format!("{db}-wal"));
        let _ = std::fs::remove_file(format!("{db}-shm"));
        let sz = std::fs::metadata(&db)?.len();
        eprintln!("[{portal}] {db} ({:.1} MB)", sz as f64 / 1_048_576.0);
    }
    Ok(())
}
