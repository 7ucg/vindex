//! Stream-Extraktion: HLS-Master-Playlist-Parsing und Qualitäts-Logik.
//!
//! Portiert aus `downloaders/common.py` (Quality-Tiers, Master-Parsing mit
//! Bibliothek + Regex-Fallback, `tier_blocks`).

use std::collections::BTreeMap;

use regex::Regex;

use crate::model::Quality;

/// Parst eine HLS-Master-Playlist zu `{Quality: Variant-URL}`.
///
/// Nimmt pro Tier die Variante mit der höchsten Bandbreite; Varianten ohne
/// Auflösung werden übersprungen (nicht in ein Tier einordenbar). Leeres Ergebnis
/// bei einer Single-Quality-Media-Playlist (keine `#EXT-X-STREAM-INF`).
pub fn parse_hls_master(text: &str, base_url: &str) -> BTreeMap<Quality, String> {
    let variants = parse_with_lib(text, base_url);
    if !variants.is_empty() {
        return variants;
    }
    parse_with_regex(text, base_url)
}

fn parse_with_lib(text: &str, base_url: &str) -> BTreeMap<Quality, String> {
    let mut best: BTreeMap<Quality, (u64, String)> = BTreeMap::new();
    if let Ok(m3u8_rs::Playlist::MasterPlaylist(master)) =
        m3u8_rs::parse_playlist_res(text.as_bytes())
    {
        for v in master.variants {
            let Some(res) = v.resolution else { continue };
            let tier = Quality::from_height(res.height as u32);
            let bw = v.bandwidth;
            let url = join_url(base_url, &v.uri);
            match best.get(&tier) {
                Some((prev_bw, _)) if *prev_bw >= bw => {}
                _ => {
                    best.insert(tier, (bw, url));
                }
            }
        }
    }
    best.into_iter().map(|(t, (_, u))| (t, u)).collect()
}

fn parse_with_regex(text: &str, base_url: &str) -> BTreeMap<Quality, String> {
    let res_re = Regex::new(r"RESOLUTION=\d+x(\d+)").unwrap();
    let bw_re = Regex::new(r"BANDWIDTH=(\d+)").unwrap();
    let mut best: BTreeMap<Quality, (u64, String)> = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("#EXT-X-STREAM-INF") {
            continue;
        }
        let Some(h) = res_re
            .captures(line)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
        else {
            continue;
        };
        let bw = bw_re
            .captures(line)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u64>().ok())
            .unwrap_or(0);
        // URI = nächste nicht-leere, nicht-Kommentar-Zeile.
        let Some(uri) = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty() && !l.starts_with('#'))
        else {
            continue;
        };
        let tier = Quality::from_height(h);
        let url = join_url(base_url, uri);
        match best.get(&tier) {
            Some((prev_bw, _)) if *prev_bw >= bw => {}
            _ => {
                best.insert(tier, (bw, url));
            }
        }
    }
    best.into_iter().map(|(t, (_, u))| (t, u)).collect()
}

/// Minimale URL-Auflösung relativ zu *base* (ohne externe URL-Lib).
fn join_url(base: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    if let Some(rest) = uri.strip_prefix("//") {
        let scheme = base.split(':').next().unwrap_or("https");
        return format!("{scheme}://{rest}");
    }
    if uri.starts_with('/') {
        // Scheme + Host aus base.
        if let Some(idx) = base.find("://") {
            let after = &base[idx + 3..];
            let host_end = after.find('/').map(|p| idx + 3 + p).unwrap_or(base.len());
            return format!("{}{}", &base[..host_end], uri);
        }
        return format!("{base}{uri}");
    }
    // Relativ zum Verzeichnis von base.
    let dir_end = base.rfind('/').map(|p| p + 1).unwrap_or(base.len());
    format!("{}{}", &base[..dir_end], uri)
}

/// True, wenn eine gemessene *height* für die geforderte *quality* übersprungen
/// werden soll (Single-Stream-Hoster rufen das und melden dann Fehlschlag, damit
/// der Worker den nächsten Hoster/Tier probiert).
pub fn tier_blocks(height: Option<u32>, requested: Option<Quality>) -> bool {
    match (requested, height) {
        (Some(q), Some(h)) => Quality::from_height(h) != q,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_buckets() {
        assert_eq!(Quality::from_height(2160), Quality::K4);
        assert_eq!(Quality::from_height(1088), Quality::P1080);
        assert_eq!(Quality::from_height(1080), Quality::P1080);
        assert_eq!(Quality::from_height(720), Quality::P720);
        assert_eq!(Quality::from_height(540), Quality::P480);
        assert_eq!(Quality::from_height(360), Quality::P360);
        assert_eq!(Quality::from_height(144), Quality::P360);
    }

    #[test]
    fn master_regex_parse_picks_highest_bandwidth() {
        let master = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n\
            360/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080\n\
            1080/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n\
            1080-hi/index.m3u8\n";
        let got = parse_hls_master(master, "https://cdn.example.com/stream/master.m3u8");
        assert_eq!(
            got.get(&Quality::P1080).map(String::as_str),
            Some("https://cdn.example.com/stream/1080-hi/index.m3u8")
        );
        assert_eq!(
            got.get(&Quality::P360).map(String::as_str),
            Some("https://cdn.example.com/stream/360/index.m3u8")
        );
    }

    #[test]
    fn media_playlist_yields_empty() {
        let media = "#EXTM3U\n#EXTINF:6.0,\nseg0.ts\n#EXTINF:6.0,\nseg1.ts\n";
        assert!(parse_hls_master(media, "https://x/y.m3u8").is_empty());
    }

    #[test]
    fn join_handles_variants() {
        assert_eq!(
            join_url("https://h/a/b.m3u8", "c.m3u8"),
            "https://h/a/c.m3u8"
        );
        assert_eq!(
            join_url("https://h/a/b.m3u8", "/x.m3u8"),
            "https://h/x.m3u8"
        );
        assert_eq!(join_url("https://h/a/b.m3u8", "//cdn/x"), "https://cdn/x");
        assert_eq!(join_url("https://h/a", "https://o/z"), "https://o/z");
    }

    #[test]
    fn tier_gate() {
        assert!(tier_blocks(Some(720), Some(Quality::P1080)));
        assert!(!tier_blocks(Some(1080), Some(Quality::P1080)));
        assert!(!tier_blocks(None, Some(Quality::P1080)));
        assert!(!tier_blocks(Some(720), None));
    }
}
