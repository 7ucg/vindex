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

        if variants.is_empty() {
            // Einzel-Quality-Media-Playlist → direkt muxen.
            return mux(resolved, out_path, cancel).await.map(|_| true);
        }
        for q in prefs.quality_sweep() {
            if let Some(url) = variants.get(&q) {
                let variant = Resolved {
                    url: url.clone(),
                    referer: resolved.referer.clone(),
                    is_hls: true,
                };
                if mux(&variant, out_path, cancel).await.is_ok() {
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
        };
        assert_eq!(
            prefs.quality_sweep(),
            vec![Quality::P1080, Quality::P720, Quality::P480]
        );
    }
}
