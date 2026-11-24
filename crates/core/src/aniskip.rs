//! AniSkip (Intro/Outro-Zeiten).
//!
//! Portiert aus `aniskip/aniskip.py`: fragt `api.aniskip.com` nach Skip-Zeiten
//! für eine MyAnimeList-ID + Episodennummer. Die MAL-ID-Auflösung läuft über die
//! Jikan-API (inoffizielle MyAnimeList-API), weil die Portal-URLs nur einen Slug
//! tragen, AniSkip aber eine MAL-ID braucht.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::http::capped_text;

const API: &str = "https://api.aniskip.com/v1/skip-times";

/// Obergrenze für eine externe API-Antwort (Jikan/AniSkip). Die JSON-Antworten sind
/// klein; die Grenze verhindert, dass eine manipulierte Antwort den Speicher flutet.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Jikan-Basis (inoffizielle MyAnimeList-API).
const JIKAN_SEARCH: &str = "https://api.jikan.moe/v4/anime";
const JIKAN_RELATIONS: &str = "https://api.jikan.moe/v4/anime";

/// AniList-GraphQL (liefert `idMal` direkt). Primäre Quelle für die MAL-ID, weil
/// Jikan in manchen Netzen nicht erreichbar ist; Jikan bleibt Fallback.
const ANILIST: &str = "https://graphql.anilist.co";

/// Jikan erlaubt grob 3 Requests/Sekunde; zwischen Folgeabrufen (Relations) wird
/// kurz pausiert, um nicht in die 429-Drossel zu laufen.
const JIKAN_DELAY_MS: u64 = 400;

/// Art eines Skip-Intervalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkipType {
    /// Opening (Intro).
    Op,
    /// Ending (Outro).
    Ed,
}

/// Ein Skip-Intervall in Sekunden.
#[derive(Debug, Clone)]
pub struct SkipInterval {
    pub start: f64,
    pub end: f64,
    pub skip_type: SkipType,
}

/// Holt die Skip-Zeiten (OP/ED) für *mal_id* + *episode*.
pub async fn skip_times(
    client: &reqwest::Client,
    mal_id: u64,
    episode: u32,
) -> Result<Vec<SkipInterval>> {
    let url = format!("{API}/{mal_id}/{episode}?types=op&types=ed");
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    let body = capped_text(resp, MAX_RESPONSE_BYTES).await?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| Error::Parse(format!("aniskip json: {e}")))?;
    if json.get("found").and_then(|v| v.as_bool()) != Some(true) {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
        for r in results {
            let interval = r.get("interval");
            let start = interval
                .and_then(|i| i.get("start_time"))
                .and_then(|v| v.as_f64());
            let end = interval
                .and_then(|i| i.get("end_time"))
                .and_then(|v| v.as_f64());
            let kind = r.get("skip_type").and_then(|v| v.as_str());
            if let (Some(start), Some(end), Some(kind)) = (start, end, kind) {
                let skip_type = match kind {
                    "op" => SkipType::Op,
                    "ed" => SkipType::Ed,
                    other => return Err(Error::Parse(format!("unbekannter skip_type: {other}"))),
                };
                out.push(SkipInterval {
                    start,
                    end,
                    skip_type,
                });
            }
        }
    }
    Ok(out)
}

// ─────────────────────────── MAL-ID-Auflösung (Jikan) ───────────────────────────
//
// Die Portal-URLs tragen nur einen Slug (z. B. `one-piece`), AniSkip braucht aber
// eine MyAnimeList-ID. Jikan (inoffizielle MAL-API) liefert sie über eine Titelsuche.
// Höhere Staffeln sind auf MAL eigene Einträge; ihnen wird ab Staffel 1 der echten
// **Sequel-Kette** gefolgt (Staffel 1 →Sequel→ Staffel 2 →Sequel→ …), was die
// tatsächliche Staffel-Reihenfolge abbildet statt sie über IDs zu raten.

/// Ein Treffer aus der Jikan-Suche bzw. den Relations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JikanEntry {
    pub mal_id: u64,
    pub kind: Option<String>,
    pub title: Option<String>,
}

/// Slug (`one-piece`) → Suchbegriff (`one piece`).
pub fn slug_to_query(slug: &str) -> String {
    slug.replace(['-', '_'], " ").trim().to_string()
}

/// Parst die Jikan-Suchantwort (`{"data":[{"mal_id":…,"type":…,"title":…}]}`).
/// Rein und netzfrei testbar.
pub fn parse_search(body: &str) -> Vec<JikanEntry> {
    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
        for item in arr {
            let Some(mal_id) = item.get("mal_id").and_then(|v| v.as_u64()) else {
                continue;
            };
            out.push(JikanEntry {
                mal_id,
                kind: item
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                title: item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }
    out
}

/// Wählt die Staffel-1-ID aus Suchtreffern: erster TV-Eintrag, sonst erster Treffer.
pub fn pick_season1(entries: &[JikanEntry]) -> Option<u64> {
    entries
        .iter()
        .find(|e| e.kind.as_deref() == Some("TV"))
        .or_else(|| entries.first())
        .map(|e| e.mal_id)
}

/// Parst die Relations-Antwort und gibt die MAL-IDs verwandter Anime-Einträge
/// zurück (nur Fortsetzungs-/Nebengeschichte-Beziehungen). Rein testbar.
/// Liest aus einer Relations-Antwort die MAL-ID des **direkten Sequels** (nächste
/// Staffel). Nur die `Sequel`-Beziehung mit `type == "anime"`; die kleinste ID
/// gewinnt, falls mehrere (deterministisch). `None`, wenn es kein Sequel gibt.
/// Rein/testbar.
pub fn parse_sequel_id(body: &str) -> Option<u64> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let groups = json.get("data")?.as_array()?;
    let mut candidates: Vec<u64> = Vec::new();
    for group in groups {
        if group.get("relation").and_then(|v| v.as_str()) != Some("Sequel") {
            continue;
        }
        if let Some(entries) = group.get("entry").and_then(|v| v.as_array()) {
            for entry in entries {
                if entry.get("type").and_then(|v| v.as_str()) == Some("anime") {
                    if let Some(id) = entry.get("mal_id").and_then(|v| v.as_u64()) {
                        candidates.push(id);
                    }
                }
            }
        }
    }
    candidates.into_iter().min()
}

/// Percent-kodiert einen Query-Wert (nur unreservierte Zeichen bleiben stehen).
fn encode_q(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Fragt die Jikan-Suche ab und gibt die Treffer zurück.
async fn search_anime(client: &reqwest::Client, query: &str) -> Result<Vec<JikanEntry>> {
    let url = format!(
        "{JIKAN_SEARCH}?q={}&type=tv&limit=5&order_by=popularity&sort=asc",
        encode_q(query)
    );
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    Ok(parse_search(&capped_text(resp, MAX_RESPONSE_BYTES).await?))
}

/// Holt die verwandten MAL-IDs für eine ID.
/// Holt die MAL-ID des direkten Sequels von *mal_id* (nächste Staffel), falls vorhanden.
async fn sequel_id(client: &reqwest::Client, mal_id: u64) -> Result<Option<u64>> {
    let url = format!("{JIKAN_RELATIONS}/{mal_id}/relations");
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    Ok(parse_sequel_id(
        &capped_text(resp, MAX_RESPONSE_BYTES).await?,
    ))
}

/// Baut die geordnete Staffel-Kette, indem ab `start` iterativ dem **Sequel**
/// gefolgt wird: `[start, sequel(start), …]`. Das spiegelt die echte Staffel-
/// Reihenfolge wider (keine ID-Sortier-Heuristik). Längen-gedeckelt, bricht bei
/// einem Zyklus ab.
async fn season_chain(client: &reqwest::Client, start: u64) -> Result<Vec<u64>> {
    const MAX_SEASONS: usize = 20;
    let mut chain = vec![start];
    let mut current = start;
    while chain.len() < MAX_SEASONS {
        tokio::time::sleep(std::time::Duration::from_millis(JIKAN_DELAY_MS)).await;
        match sequel_id(client, current).await? {
            Some(next) if !chain.contains(&next) => {
                chain.push(next);
                current = next;
            }
            _ => break,
        }
    }
    Ok(chain)
}

/// Löst die MAL-ID für `slug` + `staffel` auf. Staffel 1 nutzt direkt den
/// Suchtreffer; höhere Staffeln folgen ab Staffel 1 der **Sequel-Kette**
/// (`staffel`-ter Eintrag). `None`, wenn Jikan nichts Passendes liefert.
/// Setzt eine AniList-GraphQL-Abfrage ab und gibt das geparste JSON zurück.
async fn anilist_query(client: &reqwest::Client, body: String) -> Result<serde_json::Value> {
    let resp = client
        .post(ANILIST)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .body(body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(Error::Parse(format!("anilist http {}", resp.status())));
    }
    let text = capped_text(resp, MAX_RESPONSE_BYTES).await?;
    serde_json::from_str(&text).map_err(|e| Error::Parse(format!("anilist json: {e}")))
}

/// Wählt aus AniList-Treffern die Staffel-1-Fassung: Eintrag mit MAL-ID und dem
/// frühesten Startjahr (der Ur-Serie), bevorzugt TV. Liefert (AniList-ID, MAL-ID).
fn anilist_pick_s1(media: &[serde_json::Value]) -> Option<(i64, u64)> {
    media
        .iter()
        .filter(|m| m.get("idMal").and_then(|v| v.as_u64()).is_some())
        .min_by_key(|m| {
            let year = m
                .get("startDate")
                .and_then(|d| d.get("year"))
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let tv_rank = if m.get("format").and_then(|v| v.as_str()) == Some("TV") {
                0
            } else {
                1
            };
            (year, tv_rank)
        })
        .and_then(|m| Some((m.get("id")?.as_i64()?, m.get("idMal")?.as_u64()?)))
}

/// Folgt der `SEQUEL`-Beziehung eines AniList-Eintrags zur nächsten Staffel.
/// Deterministisch: kleinste MAL-ID unter den Sequel-Kandidaten mit MAL-ID.
async fn anilist_sequel(client: &reqwest::Client, anilist_id: i64) -> Result<Option<(i64, u64)>> {
    let body = serde_json::json!({
        "query": "query($id:Int){Media(id:$id){relations{edges{relationType node{id idMal format}}}}}",
        "variables": {"id": anilist_id}
    })
    .to_string();
    let v = anilist_query(client, body).await?;
    let edges = match v
        .get("data")
        .and_then(|d| d.get("Media"))
        .and_then(|m| m.get("relations"))
        .and_then(|r| r.get("edges"))
        .and_then(|e| e.as_array())
    {
        Some(e) => e,
        None => return Ok(None),
    };
    let best = edges
        .iter()
        .filter(|e| e.get("relationType").and_then(|v| v.as_str()) == Some("SEQUEL"))
        .filter_map(|e| e.get("node"))
        .filter(|n| n.get("idMal").and_then(|v| v.as_u64()).is_some())
        .filter_map(|n| Some((n.get("id")?.as_i64()?, n.get("idMal")?.as_u64()?)))
        .min_by_key(|&(_, mal)| mal);
    Ok(best)
}

/// MAL-ID über AniList auflösen (Suche + Staffel-Kette). `None`, wenn nichts passt.
async fn anilist_resolve(
    client: &reqwest::Client,
    query: &str,
    staffel: u32,
) -> Result<Option<u64>> {
    let body = serde_json::json!({
        "query": "query($q:String){Page(perPage:8){media(search:$q,type:ANIME,sort:SEARCH_MATCH){id idMal format startDate{year}}}}",
        "variables": {"q": query}
    })
    .to_string();
    let v = anilist_query(client, body).await?;
    let media = v
        .get("data")
        .and_then(|d| d.get("Page"))
        .and_then(|p| p.get("media"))
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let Some((mut aid, mut mal)) = anilist_pick_s1(&media) else {
        return Ok(None);
    };
    let mut season = 1u32;
    while season < staffel {
        match anilist_sequel(client, aid).await? {
            Some((nid, nmal)) => {
                aid = nid;
                mal = nmal;
                season += 1;
            }
            None => break,
        }
    }
    Ok(Some(mal))
}

pub async fn resolve_mal_id(
    client: &reqwest::Client,
    slug: &str,
    staffel: u32,
) -> Result<Option<u64>> {
    let query = slug_to_query(slug);
    if query.is_empty() {
        return Ok(None);
    }
    // AniList zuerst (erreichbar, liefert idMal direkt); Jikan nur als Fallback.
    if let Ok(Some(id)) = anilist_resolve(client, &query, staffel).await {
        return Ok(Some(id));
    }
    let entries = search_anime(client, &query).await?;
    let Some(season1) = pick_season1(&entries) else {
        return Ok(None);
    };
    if staffel <= 1 {
        return Ok(Some(season1));
    }
    let chain = season_chain(client, season1).await?;
    Ok(Some(
        chain
            .get((staffel - 1) as usize)
            .copied()
            .unwrap_or(season1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_wird_zu_suchbegriff() {
        assert_eq!(slug_to_query("one-piece"), "one piece");
        assert_eq!(slug_to_query("kaguya_sama"), "kaguya sama");
        assert_eq!(slug_to_query("dark"), "dark");
    }

    #[test]
    fn parse_search_liest_treffer() {
        let body = r#"{"data":[
            {"mal_id":21,"type":"TV","title":"One Piece"},
            {"mal_id":459,"type":"Movie","title":"One Piece Movie"}
        ]}"#;
        let got = parse_search(body);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].mal_id, 21);
        assert_eq!(got[0].kind.as_deref(), Some("TV"));
        assert_eq!(got[0].title.as_deref(), Some("One Piece"));
    }

    #[test]
    fn parse_search_toleriert_muell() {
        assert!(parse_search("nicht json").is_empty());
        assert!(parse_search(r#"{"data":[{"title":"ohne id"}]}"#).is_empty());
    }

    #[test]
    fn pick_season1_bevorzugt_tv() {
        let entries = vec![
            JikanEntry {
                mal_id: 999,
                kind: Some("Movie".into()),
                title: None,
            },
            JikanEntry {
                mal_id: 21,
                kind: Some("TV".into()),
                title: None,
            },
        ];
        assert_eq!(pick_season1(&entries), Some(21));
    }

    #[test]
    fn pick_season1_faellt_auf_ersten_zurueck() {
        let entries = vec![JikanEntry {
            mal_id: 7,
            kind: Some("OVA".into()),
            title: None,
        }];
        assert_eq!(pick_season1(&entries), Some(7));
        assert_eq!(pick_season1(&[]), None);
    }

    #[test]
    fn parse_sequel_nur_echte_sequel_beziehung() {
        // Realistische Relations-Antwort: nur „Sequel" (nächste Staffel) zählt,
        // Adaptation (Manga) und Side story (OVA) nicht.
        let body = r#"{"data":[
            {"relation":"Prequel","entry":[{"mal_id":1,"type":"anime","name":"S0"}]},
            {"relation":"Sequel","entry":[{"mal_id":100,"type":"anime","name":"S2"}]},
            {"relation":"Adaptation","entry":[{"mal_id":200,"type":"manga","name":"Manga"}]},
            {"relation":"Side story","entry":[{"mal_id":300,"type":"anime","name":"OVA"}]}
        ]}"#;
        assert_eq!(parse_sequel_id(body), Some(100));
        // Kein Sequel → None.
        let keins = r#"{"data":[{"relation":"Prequel","entry":[{"mal_id":1,"type":"anime"}]}]}"#;
        assert_eq!(parse_sequel_id(keins), None);
    }

    #[test]
    fn parse_sequel_ueberspringt_nicht_anime() {
        // Sequel, aber type==manga → kein Treffer.
        let body = r#"{"data":[{"relation":"Sequel","entry":[{"mal_id":5,"type":"manga"}]}]}"#;
        assert_eq!(parse_sequel_id(body), None);
    }

    #[test]
    fn parse_sequel_kleinste_id_deterministisch() {
        // Mehrere Sequel-Einträge (z. B. TV + Film) → kleinste (meist die TV-Staffel).
        let body = r#"{"data":[{"relation":"Sequel","entry":[
            {"mal_id":900,"type":"anime"},{"mal_id":100,"type":"anime"}]}]}"#;
        assert_eq!(parse_sequel_id(body), Some(100));
    }
}
