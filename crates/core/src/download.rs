//! Download-Engine: ffmpeg-Mux, Qualitäts-Sweep und Fallback-Kette.
//!
//! Portiert aus `downloaders/common.py` (`run_ffmpeg`, `ffprobe_height`) und der
//! Worker-Logik (Sprache → Hoster → Qualität). Streaming/Resume über ffmpegs
//! Reconnect-Optionen; Abbruch über einen `CancellationToken`.

use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::extract::parse_hls_master;
use crate::hoster::{self, Resolved};
use crate::model::{Hoster, Language, Quality};
use crate::site;

/// Obergrenze für eine geladene HLS-Playlist. Master-/Media-Playlists sind klein;
/// die Grenze verhindert, dass eine manipulierte Antwort den Speicher flutet.
const MAX_PLAYLIST_BYTES: usize = 512 * 1024;

/// Lädt einen Request und gibt den Body als Text zurück, aber höchstens `cap`
/// Bytes. Wiederverwendet den konfigurierten Client (Timeout/UA) des Aufrufers.
async fn fetch_text_capped(req: reqwest::RequestBuilder, cap: usize) -> Result<String> {
    let resp = req.send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if buf.len() + chunk.len() > cap {
            return Err(Error::extract("HLS", "Playlist überschreitet Größenlimit"));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Download-Präferenzen für eine Episode.
#[derive(Debug, Clone)]
pub struct DownloadPrefs {
    /// Sprach-Reihenfolge (erste verfügbare gewinnt).
    pub languages: Vec<Language>,
    /// Hoster-Fallback-Reihenfolge.
    pub hosters: Vec<Hoster>,
    /// Höchste akzeptierte Qualität.
    pub max_quality: Quality,
    /// Niedrigste akzeptierte Qualität.
    pub min_quality: Quality,
    /// Zielverzeichnis.
    pub out_dir: PathBuf,
    /// Dateiname inkl. Endung (z. B. `Show S01E01.mkv`).
    pub file_name: String,
    /// HLS-Segmente selbst holen (robust gegen 429) statt ffmpeg die Playlist
    /// ziehen zu lassen. ffmpeg remuxt weiterhin die zusammengesetzte Datei.
    pub native_hls: bool,
}

impl DownloadPrefs {
    /// Tiers von max bis min, absteigend.
    fn quality_sweep(&self) -> Vec<Quality> {
        Quality::TIERS
            .iter()
            .map(|(q, _)| *q)
            .filter(|q| *q <= self.max_quality && *q >= self.min_quality)
            .collect()
    }
}

/// Lädt eine Episode herunter: probiert Sprachen, Hoster und Qualitäten in der
/// konfigurierten Reihenfolge und muxt den ersten erfolgreichen Stream.
pub async fn download_episode(
    client: &reqwest::Client,
    episode_url: &str,
    prefs: &DownloadPrefs,
    cancel: Option<CancellationToken>,
) -> Result<PathBuf> {
    // SSRF-Schutz: nur unterstützte Portale als Einstiegs-URL zulassen.
    site::validate_source_url(episode_url).map_err(Error::Parse)?;
    let base = site::base_for_url(episode_url);
    let links = site::serienstream::provider_links(client, base, episode_url).await?;
    let out_path = prefs.out_dir.join(&prefs.file_name);

    for &lang in &prefs.languages {
        let Some(hmap) = links.hosters_for(lang) else {
            continue;
        };
        for &hoster in &prefs.hosters {
            let Some(embed) = hmap.get(&hoster) else {
                continue;
            };
            if is_cancelled(&cancel) {
                return Err(Error::Ffmpeg("abgebrochen".into()));
            }
            let resolved = match hoster::resolve(client, hoster, embed).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("Hoster {} fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            };

            if try_download_resolved(client, &resolved, prefs, &out_path, &cancel).await? {
                tracing::info!(
                    "Download OK: {} [{}/{}]",
                    prefs.file_name,
                    lang.as_str(),
                    hoster.as_str()
                );
                return Ok(out_path);
            }
        }
    }
    Err(Error::NoStream)
}

/// Lädt einen Filmo-Film herunter (eigener CSRF-/Redirect-Resolver).
pub async fn download_movie(
    client: &reqwest::Client,
    movie_url: &str,
    prefs: &DownloadPrefs,
    cancel: Option<CancellationToken>,
) -> Result<PathBuf> {
    site::validate_source_url(movie_url).map_err(Error::Parse)?;
    let movie = site::filmo::movie(client, movie_url).await?;
    let csrf = movie
        .csrf
        .as_deref()
        .ok_or_else(|| Error::extract("Filmo", "CSRF-Token fehlt"))?;
    let out_path = prefs.out_dir.join(&prefs.file_name);

    for &lang in &prefs.languages {
        let Some(hmap) = movie.hosters_for(lang) else {
            continue;
        };
        for &hoster in &prefs.hosters {
            let Some(token) = hmap.get(&hoster) else {
                continue;
            };
            if is_cancelled(&cancel) {
                return Err(Error::Ffmpeg("abgebrochen".into()));
            }
            let embed = match site::filmo::resolve_embed(client, movie_url, csrf, token).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("Filmo-Redirect ({}) fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            };
            let resolved = match hoster::resolve(client, hoster, &embed).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!("Hoster {} fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            };
            if try_download_resolved(client, &resolved, prefs, &out_path, &cancel).await? {
                tracing::info!(
                    "Download OK: {} [{}/{}]",
                    prefs.file_name,
                    lang.as_str(),
                    hoster.as_str()
                );
                return Ok(out_path);
            }
        }
    }
    Err(Error::NoStream)
}

/// Routet auf den richtigen Download-Pfad anhand der URL (Filmo vs. SerienStream).
pub async fn download_url(
    client: &reqwest::Client,
    url: &str,
    prefs: &DownloadPrefs,
    cancel: Option<CancellationToken>,
) -> Result<PathBuf> {
    // Von Hand getippte URLs vorab korrigieren (Schema, Host-Tippfehler, Müll).
    let corrected = site::autocorrect::autocorrect(url).map(|c| c.url);
    let url = corrected.as_deref().unwrap_or(url);
    if site::is_filmo(url) {
        download_movie(client, url, prefs, cancel).await
    } else {
        download_episode(client, url, prefs, cancel).await
    }
}

/// Muxt einen aufgelösten Stream — bei HLS mit Tier-Sweep, sonst direkt.
async fn try_download_resolved(
    client: &reqwest::Client,
    resolved: &Resolved,
    prefs: &DownloadPrefs,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
) -> Result<bool> {
    if resolved.is_hls {
        // Master-Playlist über den konfigurierten Client (Timeout/UA) laden und die
        // passende Tier-Variante wählen; Antwortgröße ist begrenzt.
        let mut req = client.get(&resolved.url);
        if let Some(r) = &resolved.referer {
            req = req.header("Referer", r);
        }
        let text = fetch_text_capped(req, MAX_PLAYLIST_BYTES).await?;
        let variants = parse_hls_master(&text, &resolved.url);

        let referer = resolved.referer.as_deref();
        if variants.is_empty() {
            // Einzel-Quality-Media-Playlist: nativ holen oder muxen.
            return Ok(deliver_hls(
                client,
                &resolved.url,
                referer,
                prefs.native_hls,
                out_path,
                cancel,
            )
            .await);
        }
        for q in prefs.quality_sweep() {
            if let Some(url) = variants.get(&q) {
                if deliver_hls(client, url, referer, prefs.native_hls, out_path, cancel).await {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    } else {
        // Einzel-MP4: Höhe per ffprobe prüfen und unterhalb der Mindestqualität
        // überspringen, damit nicht versehentlich 360p statt des Minimums landet.
        // Ohne ffprobe (None) wird best effort gemuxt.
        if is_cancelled(cancel) {
            return Err(Error::Ffmpeg("abgebrochen".into()));
        }
        if let Some(h) = ffprobe_height(&resolved.url, resolved.referer.as_deref()).await {
            if Quality::from_height(h) < prefs.min_quality {
                tracing::debug!(
                    "MP4 {}p unter Mindestqualität {:?}, überspringe",
                    h,
                    prefs.min_quality
                );
                return Ok(false);
            }
        }
        mux(resolved, out_path, cancel).await.map(|_| true)
    }
}

fn is_cancelled(cancel: &Option<CancellationToken>) -> bool {
    cancel.as_ref().map(|c| c.is_cancelled()).unwrap_or(false)
}

/// Liefert eine HLS-Variante aus: bei `native` erst der eigene Segment-Download,
/// sonst oder bei Fehlschlag der ffmpeg-Mux. Gibt Erfolg als `bool` zurück.
async fn deliver_hls(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
    native: bool,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
) -> bool {
    if native {
        match download_hls_native(client, url, referer, out_path, cancel).await {
            Ok(true) => return true,
            Ok(false) => {} // verschlüsselt/leer → ffmpeg
            Err(e) => tracing::debug!("nativer HLS-Download fehlgeschlagen: {e}"),
        }
    }
    let variant = Resolved {
        url: url.to_string(),
        referer: referer.map(str::to_string),
        is_hls: true,
    };
    mux(&variant, out_path, cancel).await.is_ok()
}

/// Obergrenze einer HLS-Media-Playlist (viele kurze Segment-Zeilen, aber begrenzt).
const MAX_MEDIA_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;

/// True, wenn die Media-Playlist verschlüsselte Segmente nutzt. Jede `#EXT-X-KEY`-
/// Zeile wird geparst; verschlüsselt, sobald eine `METHOD` ungleich `NONE` ist oder
/// sich nicht lesen lässt (fail-closed — solche Streams überlässt vindex ffmpeg).
fn hls_is_encrypted(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        let Some(attrs) = line.strip_prefix("#EXT-X-KEY:") else {
            continue;
        };
        match key_method(attrs) {
            Some(m) if m.eq_ignore_ascii_case("NONE") => continue,
            _ => return true,
        }
    }
    false
}

/// Liest den Wert des `METHOD`-Attributs aus einer `EXT-X-KEY`-Attributliste.
/// Kommata innerhalb von Anführungszeichen (z. B. in `URI="…"`) trennen nicht.
fn key_method(attrs: &str) -> Option<String> {
    let mut in_quotes = false;
    let mut start = 0usize;
    let mut parts: Vec<&str> = Vec::new();
    for (i, c) in attrs.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(&attrs[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&attrs[start..]);
    for p in parts {
        if let Some((k, v)) = p.split_once('=') {
            if k.trim().eq_ignore_ascii_case("METHOD") {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Segment-URLs einer Media-Playlist, absolut aufgelöst gegen *base_url*.
fn media_segment_urls(text: &str, base_url: &str) -> Vec<String> {
    let base = reqwest::Url::parse(base_url).ok();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|seg| {
            if seg.starts_with("http://") || seg.starts_with("https://") {
                Some(seg.to_string())
            } else {
                base.as_ref()?.join(seg).ok().map(|u| u.to_string())
            }
        })
        .collect()
}

/// Lädt eine HLS-Media-Playlist segmentweise selbst, hängt die Segmente zu einer
/// `.part.ts` zusammen und remuxt sie mit ffmpeg nach *out_path*.
///
/// `Ok(true)`: erfolgreich. `Ok(false)`: nicht zuständig (verschlüsselt oder keine
/// Segmente) — der Aufrufer soll auf den normalen ffmpeg-Mux zurückfallen.
async fn download_hls_native(
    client: &reqwest::Client,
    media_url: &str,
    referer: Option<&str>,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
) -> Result<bool> {
    if !is_http_url(media_url) {
        return Ok(false);
    }
    let text = {
        let mut req = client.get(media_url);
        if let Some(r) = referer {
            req = req.header("Referer", r);
        }
        fetch_text_capped(req, MAX_MEDIA_PLAYLIST_BYTES).await?
    };
    if hls_is_encrypted(&text) {
        return Ok(false);
    }
    let segments = media_segment_urls(&text, media_url);
    if segments.is_empty() {
        return Ok(false);
    }

    if let Some(dir) = out_path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let part = out_path.with_extension("part.ts");

    let result = fetch_segments_to(client, &segments, referer, &part, cancel).await;
    if let Err(e) = result {
        tokio::fs::remove_file(&part).await.ok();
        return Err(e);
    }

    // Zusammengesetzten TS-Stream nach out_path remuxen; die lokale Datei ist
    // vertrauenswürdig, daher file-Protokoll erlaubt.
    let remux = remux_local(&part, out_path, cancel).await;
    tokio::fs::remove_file(&part).await.ok();
    remux.map(|_| true)
}

/// Höchstzahl Segmente, die aus einer Playlist gelesen werden.
const MAX_SEGMENTS: usize = 50_000;
/// Obergrenze pro Segment.
const MAX_SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
/// Obergrenze über alle Segmente zusammen.
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Holt alle Segmente nacheinander (mit Retry, streamend) und schreibt sie
/// fortlaufend in *part*. Jedes Segment wird vor dem Abruf validiert (nur
/// http/https, keine gesperrten IP-Literale), muss mit dem MPEG-TS-Sync-Byte
/// `0x47` beginnen und unterliegt Pro-Segment- und Gesamt-Größengrenzen.
async fn fetch_segments_to(
    client: &reqwest::Client,
    segments: &[String],
    referer: Option<&str>,
    part: &Path,
    cancel: &Option<CancellationToken>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    if segments.len() > MAX_SEGMENTS {
        return Err(Error::extract("HLS", "zu viele Segmente"));
    }

    let mut file = tokio::fs::File::create(part).await?;
    let mut total: u64 = 0;

    for seg in segments {
        if is_cancelled(cancel) {
            return Err(Error::Ffmpeg("abgebrochen".into()));
        }
        // Jedes Segment unterliegt denselben Ziel-Regeln wie jeder andere Fetch.
        if !is_http_url(seg) || crate::http::host_is_forbidden_ip_literal(seg) {
            return Err(Error::extract("HLS", "Segment-URL nicht erlaubt"));
        }

        let resp = crate::http::get_retry(client, seg, referer)
            .await?
            .error_for_status()?;
        let mut stream = resp.bytes_stream();
        let mut seg_bytes: u64 = 0;
        let mut first = true;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if first {
                if chunk.first() != Some(&0x47) {
                    return Err(Error::extract(
                        "HLS",
                        "Segment ist kein MPEG-TS (nativer Pfad nicht anwendbar)",
                    ));
                }
                first = false;
            }
            seg_bytes += chunk.len() as u64;
            total += chunk.len() as u64;
            if seg_bytes > MAX_SEGMENT_BYTES || total > MAX_TOTAL_BYTES {
                return Err(Error::extract(
                    "HLS",
                    "Segment-Download überschreitet Größenlimit",
                ));
            }
            file.write_all(&chunk).await?;
        }
    }
    file.flush().await?;
    Ok(())
}

/// Remuxt eine lokale Mediendatei mit ffmpeg (`-c copy`) nach *out_path*.
async fn remux_local(
    src: &Path,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
) -> Result<()> {
    if is_cancelled(cancel) {
        return Err(Error::Ffmpeg("abgebrochen".into()));
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.kill_on_drop(true);
    // Format und Protokoll pinnen: die Datei besteht aus zusammengesetzten
    // Hoster-Segmenten. Ohne `-f mpegts` könnte ffmpeg sie als HLS-/concat-Playlist
    // deuten und darüber lokale Dateien lesen; `-protocol_whitelist file` verbietet
    // jedes Nachladen.
    cmd.args([
        "-nostdin",
        "-y",
        "-protocol_whitelist",
        "file",
        "-f",
        "mpegts",
        "-i",
    ]);
    cmd.arg(src);
    cmd.args(["-c", "copy"]);
    cmd.arg(out_path);
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());
    let status = match cmd.status().await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Ffmpeg(
                "ffmpeg nicht gefunden — bitte installieren und in den PATH legen".into(),
            ));
        }
        Err(e) => return Err(Error::Io(e)),
    };
    if status.success() {
        Ok(())
    } else {
        tokio::fs::remove_file(out_path).await.ok();
        Err(Error::Ffmpeg(
            "Remux der Segmentdatei fehlgeschlagen".into(),
        ))
    }
}

/// Muxt *stream* mit ffmpeg nach *out_path* (Reconnect, `-c copy`), mit Retries.
pub async fn mux(
    stream: &Resolved,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
) -> Result<()> {
    const ATTEMPTS: u32 = 3;

    // Nur http/https als ffmpeg-Eingabe zulassen. Ein manipulierter Hoster darf
    // ffmpeg nicht auf file:, pipe: oder andere Protokolle lenken.
    if !is_http_url(&stream.url) {
        return Err(Error::Ffmpeg(format!(
            "nicht unterstütztes Stream-Schema (nur http/https): {}",
            stream.url
        )));
    }

    if let Some(dir) = out_path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }

    for attempt in 1..=ATTEMPTS {
        if is_cancelled(cancel) {
            return Err(Error::Ffmpeg("abgebrochen".into()));
        }

        let mut cmd = Command::new("ffmpeg");
        cmd.kill_on_drop(true);
        if let Some(referer) = &stream.referer {
            cmd.arg("-headers")
                .arg(format!("Referer: {}\r\n", sanitize_header_value(referer)));
        }
        // Eingabe ist oben auf http/https begrenzt; die Protokoll-Allowlist hält
        // ffmpeg beim Nachladen von HLS-Segmenten auf denselben Protokollen.
        cmd.args([
            "-protocol_whitelist",
            "https,tls,tcp,crypto,http",
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_on_network_error",
            "1",
            "-reconnect_delay_max",
            "5",
        ]);
        cmd.args(["-i", &stream.url, "-c", "copy", "-nostdin", "-y"]);
        cmd.arg(out_path);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let status = match cmd.status().await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::Ffmpeg(
                    "ffmpeg nicht gefunden — bitte installieren und in den PATH legen".into(),
                ));
            }
            Err(e) => return Err(Error::Io(e)),
        };

        if status.success() {
            return Ok(());
        }
        // Teildatei entfernen und ggf. erneut versuchen.
        tokio::fs::remove_file(out_path).await.ok();
        if attempt < ATTEMPTS && !is_cancelled(cancel) {
            tokio::time::sleep(std::time::Duration::from_secs(2 * attempt as u64)).await;
        }
    }
    Err(Error::Ffmpeg(format!(
        "Mux nach {ATTEMPTS} Versuchen fehlgeschlagen: {}",
        out_path.display()
    )))
}

/// True für `http(s)`-URLs. Alles andere darf nicht an ffmpeg/ffprobe gehen.
fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Entfernt CR/LF, damit ein Referer keine zusätzlichen ffmpeg-Header einschleust.
fn sanitize_header_value(v: &str) -> String {
    v.chars().filter(|c| *c != '\r' && *c != '\n').collect()
}

/// Ermittelt die Video-Höhe eines Streams via ffprobe (nur Header). `None`, wenn
/// ffprobe fehlt, das Schema nicht http/https ist, die Sonde abläuft oder nichts
/// gefunden wird. Die Eingabe ist auf http/https + Protokoll-Allowlist begrenzt
/// und der Prozess mit Timeout gedeckelt (wird bei Drop gekillt).
pub async fn ffprobe_height(url: &str, referer: Option<&str>) -> Option<u32> {
    if !is_http_url(url) {
        return None;
    }
    let mut cmd = Command::new("ffprobe");
    cmd.kill_on_drop(true);
    cmd.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "https,tls,tcp,crypto,http",
    ]);
    if let Some(r) = referer {
        cmd.arg("-headers")
            .arg(format!("Referer: {}\r\n", sanitize_header_value(r)));
    }
    cmd.args([
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=height",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        "-i",
        url,
    ]);
    let out = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|l| l.trim().parse::<u32>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_respects_bounds() {
        let prefs = DownloadPrefs {
            languages: vec![],
            hosters: vec![],
            max_quality: Quality::P1080,
            min_quality: Quality::P480,
            out_dir: PathBuf::from("/tmp"),
            file_name: "x.mkv".into(),
            native_hls: false,
        };
        assert_eq!(
            prefs.quality_sweep(),
            vec![Quality::P1080, Quality::P720, Quality::P480]
        );
    }

    #[test]
    fn detects_encrypted_playlist() {
        let enc = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\nseg0.ts\n";
        assert!(hls_is_encrypted(enc));
        assert!(!hls_is_encrypted(
            "#EXTM3U\nseg0.ts\n#EXT-X-KEY:METHOD=NONE\n"
        ));
        assert!(!hls_is_encrypted("#EXTM3U\nseg0.ts\n"));
        // Bypass-Versuch: METHOD=NONE nur als Substring in einer Quoted-URI.
        assert!(hls_is_encrypted(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://x/k?a=METHOD=NONE\"\nseg0.ts\n"
        ));
        // Unparsebare KEY-Zeile ohne METHOD → fail-closed (verschlüsselt).
        assert!(hls_is_encrypted("#EXT-X-KEY:URI=\"k\"\nseg0.ts\n"));
    }

    #[test]
    fn resolves_segment_urls() {
        let text = "#EXTM3U\n#EXTINF:6,\nseg0.ts\n#EXTINF:6,\nhttps://cdn/seg1.ts\n";
        let segs = media_segment_urls(text, "https://host/path/index.m3u8");
        assert_eq!(
            segs,
            vec![
                "https://host/path/seg0.ts".to_string(),
                "https://cdn/seg1.ts".to_string()
            ]
        );
    }
}
