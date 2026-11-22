//! AniSkip (Intro/Outro-Zeiten).
//!
//! Portiert aus `aniskip/aniskip.py`: fragt `api.aniskip.com` nach Skip-Zeiten
//! für eine MyAnimeList-ID + Episodennummer. Die MAL-ID-Auflösung (via Jikan)
//! ist als Erweiterungspunkt markiert.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const API: &str = "https://api.aniskip.com/v1/skip-times";

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
    let body = resp.text().await?;
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
