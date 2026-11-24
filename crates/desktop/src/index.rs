//! Offline-Lesezugriff auf die entpackten Portal-DBs (`<dir>/<portal>.db`).
//!
//! Browse-Only: Suche, Katalog, Discover, Staffeln, Folgen, Metadaten, Cover und
//! verfügbare Sprachen kommen aus den lokalen DBs statt von den Portalen. Der
//! Abspiel-/Embed-Pfad bleibt bewusst live — die Portal-Links (vor allem
//! serienstreams `/r?t=`-Tokens) laufen ab und müssen frisch geholt werden.
//!
//! Jede Funktion liefert `None`/leer, wenn der Titel nicht im Index steht; die
//! Facade fällt dann auf den bisherigen Live-Pfad zurück.

use std::collections::HashSet;
use std::str::FromStr;

use rusqlite::{Connection, OpenFlags};
use vindex_core as core;

use core::model::{Episode, Language};
use core::site::TitleMeta;

use crate::SearchHit;

const PORTALS: [&str; 3] = ["aniworld", "serienstream", "filmo"];

/// Entpackt mitgelieferte `<portal>.db.zst` aus `src_dir` nach `dest_dir` — nur,
/// wenn die Ziel-DB fehlt oder der Schnappschuss sich geändert hat (Stempel =
/// Byte-Länge der `.zst`). So entpackt die App beim Start einmalig in ihren
/// beschreibbaren Datenordner; spätere Starts überspringen unveränderte Portale.
pub fn ensure_unpacked(src_dir: &str, dest_dir: &str) -> std::io::Result<()> {
    use std::path::Path;
    std::fs::create_dir_all(dest_dir)?;
    for portal in PORTALS {
        let zst = format!("{src_dir}/{portal}.db.zst");
        if !Path::new(&zst).exists() {
            continue;
        }
        let db = format!("{dest_dir}/{portal}.db");
        let stamp = format!("{dest_dir}/{portal}.stamp");
        let src_len = std::fs::metadata(&zst)?.len();
        let up_to_date = Path::new(&db).exists()
            && std::fs::read_to_string(&stamp)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                == Some(src_len);
        if up_to_date {
            continue;
        }
        decompress(&zst, &db)?;
        // Alte WAL-/SHM-Sidecars einer vorherigen DB verfälschen die neue — weg damit.
        let _ = std::fs::remove_file(format!("{db}-wal"));
        let _ = std::fs::remove_file(format!("{db}-shm"));
        let _ = std::fs::write(&stamp, src_len.to_string());
    }
    Ok(())
}

/// zstd-Streaming-Dekompression (großes Fenster erlaubt, falls mit `--long` gepackt).
fn decompress(src: &str, dst: &str) -> std::io::Result<()> {
    use std::io::Write;
    let input = std::io::BufReader::new(std::fs::File::open(src)?);
    let mut dec = zstd::stream::Decoder::new(input)?;
    dec.window_log_max(31)?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(dst)?);
    std::io::copy(&mut dec, &mut out)?;
    out.flush()?;
    Ok(())
}

/// Index-Ordner (überschreibbar via `VINDEX_INDEX_DIR`, Default „index").
fn index_dir() -> String {
    std::env::var("VINDEX_INDEX_DIR").unwrap_or_else(|_| "index".to_string())
}

/// Portal-Kürzel aus einer Titel-/Episoden-URL.
fn portal_of_url(url: &str) -> &'static str {
    if core::site::is_filmo(url) {
        "filmo"
    } else if core::site::base_for_url(url) == core::model::BASE_STO {
        "serienstream"
    } else {
        "aniworld"
    }
}

/// Öffnet die Portal-DB lesend (mit Schreibrecht, aber ohne je zu schreiben —
/// so klappt der Zugriff auch auf eine WAL-DB, die der Crawler gerade nutzt).
/// `None`, wenn die Datei fehlt oder nicht öffenbar ist.
fn open(portal: &str) -> Option<Connection> {
    let path = format!("{}/{}.db", index_dir(), portal);
    if !std::path::Path::new(&path).exists() {
        return None;
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE).ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Some(conn)
}

fn json_list(row: &rusqlite::Row, idx: usize) -> Vec<String> {
    row.get::<_, Option<String>>(idx)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Volltext-ähnliche Titelsuche über alle drei Portale.
pub fn search(keyword: &str) -> Vec<SearchHit> {
    let pat = format!("%{}%", keyword.trim().to_lowercase());
    let mut out = Vec::new();
    for portal in PORTALS {
        let Some(conn) = open(portal) else { continue };
        let mut stmt = match conn.prepare(
            "SELECT name, url FROM title WHERE lower(name) LIKE ?1 ORDER BY name LIMIT 500",
        ) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rows = match stmt.query_map([&pat], |r| {
            Ok(SearchHit {
                title: r.get(0)?,
                url: r.get(1)?,
            })
        }) {
            Ok(r) => r,
            Err(_) => continue,
        };
        out.extend(rows.filter_map(|r| r.ok()));
    }
    out
}

/// Kompletter Katalog eines Portals. `None` = Portal nicht im Index (→ Live).
pub fn catalog(portal: &str) -> Option<Vec<SearchHit>> {
    let conn = open(portal)?;
    let mut stmt = conn
        .prepare("SELECT name, url FROM title ORDER BY name")
        .ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SearchHit {
                title: r.get(0)?,
                url: r.get(1)?,
            })
        })
        .ok()?;
    Some(rows.filter_map(|r| r.ok()).collect())
}

/// Discover nach Genre und/oder Jahr. Genre-Slug (`science-fiction`) wird gegen
/// die als JSON-Text gespeicherten Anzeige-Genres gematcht; Jahr gegen die
/// (ggf. als Spanne gespeicherte) Jahres-Textspalte. `None` = Portal nicht im Index.
pub fn discover(portal: &str, genre: Option<&str>, year: Option<u32>) -> Option<Vec<SearchHit>> {
    let conn = open(portal)?;
    let mut sql = String::from("SELECT name, url FROM title WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(g) = genre {
        sql.push_str(" AND lower(genres) LIKE ?");
        args.push(Box::new(format!(
            "%{}%",
            g.replace('-', " ").to_lowercase()
        )));
    }
    if let Some(y) = year {
        sql.push_str(" AND year LIKE ?");
        args.push(Box::new(format!("%{y}%")));
    }
    sql.push_str(" ORDER BY name");
    let mut stmt = conn.prepare(&sql).ok()?;
    let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            Ok(SearchHit {
                title: r.get(0)?,
                url: r.get(1)?,
            })
        })
        .ok()?;
    Some(rows.filter_map(|r| r.ok()).collect())
}

/// Staffelnummern eines Titels. `None` = nicht im Index.
pub fn seasons(url: &str) -> Option<Vec<u32>> {
    let slug = core::site::slug_from_url(url)?;
    let conn = open(portal_of_url(url))?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT episode.season FROM episode
             JOIN title ON title.id = episode.title_id
             WHERE title.slug = ?1 ORDER BY episode.season",
        )
        .ok()?;
    let rows = stmt.query_map([&slug], |r| r.get::<_, i64>(0)).ok()?;
    let v: Vec<u32> = rows.filter_map(|r| r.ok()).map(|s| s as u32).collect();
    (!v.is_empty()).then_some(v)
}

/// Episoden einer Staffel. `None` = nicht im Index.
pub fn episodes(url: &str, season: u32) -> Option<Vec<Episode>> {
    let slug = core::site::slug_from_url(url)?;
    let conn = open(portal_of_url(url))?;
    let mut stmt = conn
        .prepare(
            "SELECT episode.url, COALESCE(episode.name, ''), episode.season, episode.number
             FROM episode JOIN title ON title.id = episode.title_id
             WHERE title.slug = ?1 AND episode.season = ?2 ORDER BY episode.number",
        )
        .ok()?;
    let rows = stmt
        .query_map(rusqlite::params![slug, season as i64], |r| {
            Ok(Episode {
                url: r.get(0)?,
                title: r.get(1)?,
                season: r.get::<_, i64>(2)? as u32,
                index_in_season: r.get::<_, i64>(3)? as u32,
                explicit_number: None,
            })
        })
        .ok()?;
    let v: Vec<Episode> = rows.filter_map(|r| r.ok()).collect();
    (!v.is_empty()).then_some(v)
}

/// Verfügbare Sprachen eines Titels (aus den gespeicherten Hoster-Links). In
/// fester Reihenfolge, nur tatsächlich angebotene Varianten. `None` = nicht im Index.
pub fn languages(url: &str) -> Option<Vec<Language>> {
    let slug = core::site::slug_from_url(url)?;
    let conn = open(portal_of_url(url))?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT link.language FROM link
             JOIN episode ON episode.id = link.episode_id
             JOIN title ON title.id = episode.title_id
             WHERE title.slug = ?1",
        )
        .ok()?;
    let rows = stmt.query_map([&slug], |r| r.get::<_, String>(0)).ok()?;
    let present: HashSet<Language> = rows
        .filter_map(|r| r.ok())
        .filter_map(|s| Language::from_str(&s).ok())
        .collect();
    if present.is_empty() {
        return None;
    }
    Some(
        Language::ALL
            .into_iter()
            .filter(|l| present.contains(l))
            .collect(),
    )
}

/// Reiche Metadaten eines Titels. `None` = nicht im Index.
pub fn title_meta(url: &str) -> Option<TitleMeta> {
    let slug = core::site::slug_from_url(url)?;
    let conn = open(portal_of_url(url))?;
    conn.query_row(
        "SELECT poster, description, year, fsk, country, imdb, genres, directors, actors, producers
         FROM title WHERE slug = ?1",
        [&slug],
        |r| {
            Ok(TitleMeta {
                poster: r.get(0)?,
                description: r.get(1)?,
                year: r.get(2)?,
                fsk: r.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                country: r.get(4)?,
                imdb: r.get(5)?,
                genres: json_list(r, 6),
                directors: json_list(r, 7),
                actors: json_list(r, 8),
                producers: json_list(r, 9),
            })
        },
    )
    .ok()
}

/// Nur das Poster. Äußeres `None` = Titel nicht im Index (→ Live); inneres
/// `None` = Titel bekannt, aber ohne Poster.
pub fn cover(url: &str) -> Option<Option<String>> {
    let slug = core::site::slug_from_url(url)?;
    let conn = open(portal_of_url(url))?;
    conn.query_row("SELECT poster FROM title WHERE slug = ?1", [&slug], |r| {
        r.get::<_, Option<String>>(0)
    })
    .ok()
}
