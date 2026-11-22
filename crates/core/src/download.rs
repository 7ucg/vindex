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
/// Fortschrittsmeldung eines laufenden Downloads. Gedacht für die CLI-Anzeige und
/// das Desktop-GUI; die Kern-Logik bleibt davon unberührt, wenn keine Senke gesetzt ist.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Der Stream wird aufgelöst (Sprach-/Hoster-Kette).
    Resolving,
    /// Der Segment-Download beginnt; `total_segments` ist bekannt, wenn die
    /// Media-Playlist vorliegt (nativer HLS-Pfad).
    Started { total_segments: Option<usize> },
    /// Ein Segment ist fertig: `done` von `total`, plus bisher geschriebene Bytes.
    Segment {
        done: usize,
        total: usize,
        bytes: u64,
    },
    /// Das Remuxen/Muxen (ffmpeg) läuft.
    Muxing,
    /// Der Download ist abgeschlossen.
    Finished,
}

/// Threadsichere, klonbare Senke für [`Progress`]-Meldungen.
pub type ProgressSink = std::sync::Arc<dyn Fn(Progress) + Send + Sync>;

/// Schickt ein Ereignis an die Senke, falls eine gesetzt ist.
fn report(progress: Option<&ProgressSink>, event: Progress) {
    if let Some(sink) = progress {
        sink(event);
    }
}

pub async fn download_episode(
    client: &reqwest::Client,
    episode_url: &str,
    prefs: &DownloadPrefs,
    cancel: Option<CancellationToken>,
    progress: Option<&ProgressSink>,
) -> Result<PathBuf> {
    // SSRF-Schutz: nur unterstützte Portale als Einstiegs-URL zulassen.
    site::validate_source_url(episode_url).map_err(Error::Parse)?;
    report(progress, Progress::Resolving);
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

            if try_download_resolved(client, &resolved, prefs, &out_path, &cancel, progress).await?
            {
                tracing::info!(
                    "Download OK: {} [{}/{}]",
                    prefs.file_name,
                    lang.as_str(),
                    hoster.as_str()
                );
                report(progress, Progress::Finished);
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
    progress: Option<&ProgressSink>,
) -> Result<PathBuf> {
    site::validate_source_url(movie_url).map_err(Error::Parse)?;
    report(progress, Progress::Resolving);
    // filmo.to bindet eine Laravel-Session ans CSRF-Token: Film-GET und `/n`-POST
    // teilen sich einen eigenen, kurzlebigen Cookie-Client (nicht den geteilten).
    let fclient = crate::http::cookie_client()?;
    let movie = site::filmo::movie(&fclient, movie_url).await?;
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
            let embed = match site::filmo::resolve_embed(&fclient, movie_url, csrf, token).await {
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
            if try_download_resolved(client, &resolved, prefs, &out_path, &cancel, progress).await?
            {
                tracing::info!(
                    "Download OK: {} [{}/{}]",
                    prefs.file_name,
                    lang.as_str(),
                    hoster.as_str()
                );
                report(progress, Progress::Finished);
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
    progress: Option<&ProgressSink>,
) -> Result<PathBuf> {
    // Von Hand getippte URLs vorab korrigieren (Schema, Host-Tippfehler, Müll).
    let corrected = site::autocorrect::autocorrect(url).map(|c| c.url);
    let url = corrected.as_deref().unwrap_or(url);
    if site::is_filmo(url) {
        download_movie(client, url, prefs, cancel, progress).await
    } else {
        download_episode(client, url, prefs, cancel, progress).await
    }
}

/// Löst den ersten abspielbaren Stream zu *url* auf — **ohne** Download/Mux.
///
/// Durchläuft dieselbe Fallback-Kette Sprache → Hoster wie [`download_url`]
/// (inklusive Filmo-Routing und URL-Autokorrektur) und gibt den ersten
/// erfolgreich aufgelösten [`Resolved`] (Stream-URL + Referer) zurück. Gedacht für
/// den Watch-Modus, der den Stream an mpv übergibt statt ihn zu muxen.
///
/// Die Qualitätsauswahl bleibt dem Player überlassen (bei HLS wählt mpv die
/// Variante selbst); `prefs` steuert hier nur Sprach- und Hoster-Reihenfolge.
pub async fn resolve_playable(
    client: &reqwest::Client,
    url: &str,
    prefs: &DownloadPrefs,
) -> Result<Resolved> {
    let corrected = site::autocorrect::autocorrect(url).map(|c| c.url);
    let url = corrected.as_deref().unwrap_or(url);
    let resolved = if site::is_filmo(url) {
        resolve_movie_playable(client, url, prefs).await?
    } else {
        resolve_episode_playable(client, url, prefs).await?
    };
    // Defense-in-Depth: die Stream-URL geht an einen externen Player (mpv). Nur
    // http/https zulassen (hoster::resolve prüft das bereits, hier noch einmal explizit),
    // damit kein file:/pipe:/anderes Schema den Player erreicht.
    if !is_http_url(&resolved.url) {
        return Err(Error::extract("watch", "Stream-URL ist nicht http/https"));
    }
    Ok(resolved)
}

/// Watch-Variante von [`download_episode`]: nur Auflösung, kein Mux.
async fn resolve_episode_playable(
    client: &reqwest::Client,
    episode_url: &str,
    prefs: &DownloadPrefs,
) -> Result<Resolved> {
    site::validate_source_url(episode_url).map_err(Error::Parse)?;
    let base = site::base_for_url(episode_url);
    let links = site::serienstream::provider_links(client, base, episode_url).await?;

    for &lang in &prefs.languages {
        let Some(hmap) = links.hosters_for(lang) else {
            continue;
        };
        for &hoster in &prefs.hosters {
            let Some(embed) = hmap.get(&hoster) else {
                continue;
            };
            match hoster::resolve(client, hoster, embed).await {
                Ok(r) => {
                    tracing::info!("Stream aufgelöst: [{}/{}]", lang.as_str(), hoster.as_str());
                    return Ok(r);
                }
                Err(e) => {
                    tracing::debug!("Hoster {} fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            }
        }
    }
    Err(Error::NoStream)
}

/// Watch-Variante von [`download_movie`]: nur Auflösung, kein Mux.
async fn resolve_movie_playable(
    client: &reqwest::Client,
    movie_url: &str,
    prefs: &DownloadPrefs,
) -> Result<Resolved> {
    site::validate_source_url(movie_url).map_err(Error::Parse)?;
    // Eigener Cookie-Client für den filmo.to-Session-/CSRF-Flow (siehe download_movie).
    let fclient = crate::http::cookie_client()?;
    let movie = site::filmo::movie(&fclient, movie_url).await?;
    let csrf = movie
        .csrf
        .as_deref()
        .ok_or_else(|| Error::extract("Filmo", "CSRF-Token fehlt"))?;

    for &lang in &prefs.languages {
        let Some(hmap) = movie.hosters_for(lang) else {
            continue;
        };
        for &hoster in &prefs.hosters {
            let Some(token) = hmap.get(&hoster) else {
                continue;
            };
            let embed = match site::filmo::resolve_embed(&fclient, movie_url, csrf, token).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("Filmo-Redirect ({}) fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            };
            match hoster::resolve(client, hoster, &embed).await {
                Ok(r) => {
                    tracing::info!("Stream aufgelöst: [{}/{}]", lang.as_str(), hoster.as_str());
                    return Ok(r);
                }
                Err(e) => {
                    tracing::debug!("Hoster {} fehlgeschlagen: {e}", hoster.as_str());
                    continue;
                }
            }
        }
    }
    Err(Error::NoStream)
}

/// Muxt einen aufgelösten Stream — bei HLS mit Tier-Sweep, sonst direkt.
async fn try_download_resolved(
    client: &reqwest::Client,
    resolved: &Resolved,
    prefs: &DownloadPrefs,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
    progress: Option<&ProgressSink>,
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
                progress,
            )
            .await);
        }
        for q in prefs.quality_sweep() {
            if let Some(url) = variants.get(&q) {
                if deliver_hls(
                    client,
                    url,
                    referer,
                    prefs.native_hls,
                    out_path,
                    cancel,
                    progress,
                )
                .await
                {
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
        report(progress, Progress::Muxing);
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
    progress: Option<&ProgressSink>,
) -> bool {
    if native {
        match download_hls_native(client, url, referer, out_path, cancel, progress).await {
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
    report(progress, Progress::Muxing);
    mux(&variant, out_path, cancel).await.is_ok()
}

/// Obergrenze einer HLS-Media-Playlist (viele kurze Segment-Zeilen, aber begrenzt).
const MAX_MEDIA_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;

/// Geparste `#EXT-X-KEY`-Zeile: Verschlüsselungsmethode, optionale Schlüssel-URL
/// und optionales deklariertes IV.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HlsKey {
    /// Wert des `METHOD`-Attributs (z. B. `AES-128`, `SAMPLE-AES`, `NONE`).
    method: String,
    /// `URI`-Attribut: Fundort des Schlüssels (noch nicht absolut aufgelöst).
    uri: Option<String>,
    /// `IV`-Attribut als 16 Bytes, falls deklariert und wohlgeformt (`0x…`).
    iv: Option<[u8; 16]>,
}

/// Verschlüsselungszustand einer Media-Playlist.
enum HlsEncryption {
    /// Unverschlüsselt (kein `EXT-X-KEY` oder nur `METHOD=NONE`).
    None,
    /// Nativ unterstützt: AES-128-CBC mit genau einem Schlüssel.
    Aes128(HlsKey),
    /// `SAMPLE-AES`, Schlüsselrotation oder unbekannt/unparsebar → ffmpeg-Fallback.
    Unsupported,
}

/// Zerlegt eine `EXT-X-KEY`-Attributliste in `(Schlüssel, Wert)`-Paare. Kommata
/// innerhalb von Anführungszeichen (z. B. in `URI="…"`) trennen nicht; umschließende
/// Anführungszeichen werden vom Wert entfernt.
fn parse_attr_list(attrs: &str) -> Vec<(String, String)> {
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
    parts
        .into_iter()
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect()
}

/// Parst eine `EXT-X-KEY`-Attributliste zu [`HlsKey`]. Fehlt `METHOD`, bleibt es
/// leer — der Aufrufer behandelt das fail-closed (kein nativer Pfad).
fn parse_ext_x_key(attrs: &str) -> HlsKey {
    let mut method = String::new();
    let mut uri = None;
    let mut iv = None;
    for (k, v) in parse_attr_list(attrs) {
        if k.eq_ignore_ascii_case("METHOD") {
            method = v;
        } else if k.eq_ignore_ascii_case("URI") {
            uri = Some(v);
        } else if k.eq_ignore_ascii_case("IV") {
            iv = parse_hex_iv(&v);
        }
    }
    HlsKey { method, uri, iv }
}

/// Parst einen `0x…`-IV (32 Hex-Ziffern) in 16 Bytes. `None` bei falscher Länge
/// oder ungültigen Hex-Zeichen.
fn parse_hex_iv(s: &str) -> Option<[u8; 16]> {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    // Erst auf genau 32 ASCII-Hexziffern prüfen, dann über Bytes gehen — so kann
    // kein Nicht-Char-Boundary-Slice panicken und kein `+`/`-` durchrutschen.
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        // bytes[2i]/[2i+1] sind gültig (len == 32) und ASCII-Hex (oben geprüft).
        let hi = (bytes[2 * i] as char).to_digit(16)?;
        let lo = (bytes[2 * i + 1] as char).to_digit(16)?;
        *b = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Baut das Default-IV aus der Mediensequenz-Nummer: 16 Byte Big-Endian.
fn iv_from_sequence(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

/// Klassifiziert die Verschlüsselung einer Media-Playlist. Nur ein einziger
/// AES-128-Schlüssel wird nativ unterstützt; `SAMPLE-AES`, mehrere verschiedene
/// Schlüssel (Rotation), unbekannte oder unparsebare `METHOD` → ffmpeg-Fallback
/// (fail-closed).
fn hls_encryption(text: &str) -> HlsEncryption {
    let mut aes_key: Option<HlsKey> = None;
    for line in text.lines() {
        let Some(attrs) = line.trim().strip_prefix("#EXT-X-KEY:") else {
            continue;
        };
        let key = parse_ext_x_key(attrs);
        if key.method.eq_ignore_ascii_case("NONE") {
            continue;
        }
        if !key.method.eq_ignore_ascii_case("AES-128") {
            // SAMPLE-AES, unbekannt oder METHOD fehlt → nicht nativ.
            return HlsEncryption::Unsupported;
        }
        match &aes_key {
            // Mehrere verschiedene Schlüssel (Rotation) kann der native Pfad nicht.
            Some(existing) if existing.uri != key.uri || existing.iv != key.iv => {
                return HlsEncryption::Unsupported;
            }
            Some(_) => {}
            None => aes_key = Some(key),
        }
    }
    match aes_key {
        Some(k) => HlsEncryption::Aes128(k),
        None => HlsEncryption::None,
    }
}

/// Liest `#EXT-X-MEDIA-SEQUENCE` (Sequenznummer des ersten Segments). Fehlt das
/// Tag, gilt 0.
fn parse_media_sequence(text: &str) -> u64 {
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            if let Ok(n) = v.trim().parse::<u64>() {
                return n;
            }
        }
    }
    0
}

/// True, wenn beide URLs denselben Origin haben (Schema + Host + Port). Das deckt
/// auch einen Schema-Downgrade ab (https-Playlist, http-Schlüssel → anderer Origin).
fn same_origin(a: &str, b: &str) -> bool {
    match (reqwest::Url::parse(a).ok(), reqwest::Url::parse(b).ok()) {
        (Some(ua), Some(ub)) => ua.host_str().is_some() && ua.origin() == ub.origin(),
        _ => false,
    }
}

/// Löst eine (ggf. relative) URL absolut gegen *base_url* auf. Absolute
/// http(s)-URLs bleiben unverändert.
fn resolve_url(target: &str, base_url: &str) -> Option<String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        Some(target.to_string())
    } else {
        reqwest::Url::parse(base_url)
            .ok()?
            .join(target)
            .ok()
            .map(|u| u.to_string())
    }
}

/// Segment-URLs einer Media-Playlist, absolut aufgelöst gegen *base_url*.
fn media_segment_urls(text: &str, base_url: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|seg| resolve_url(seg, base_url))
        .collect()
}

/// Entschlüsselungs-Kontext für AES-128-CBC-Segmente.
struct Aes128Ctx {
    /// 16-Byte-Schlüssel (über die `URI` geladen).
    key: [u8; 16],
    /// Deklariertes IV; fehlt es, dient die Segment-Sequenznummer als IV.
    declared_iv: Option<[u8; 16]>,
    /// Sequenznummer des ersten Segments (`#EXT-X-MEDIA-SEQUENCE`).
    media_sequence: u64,
}

impl Aes128Ctx {
    /// IV für das *index*-te Segment: entweder das deklarierte IV oder die
    /// (inkrementierte) Mediensequenz-Nummer als 16-Byte-Big-Endian.
    fn iv_for(&self, index: usize) -> [u8; 16] {
        self.declared_iv
            .unwrap_or_else(|| iv_from_sequence(self.media_sequence.saturating_add(index as u64)))
    }
}

/// Entschlüsselt einen AES-128-CBC-Puffer und entfernt das PKCS7-Padding am letzten
/// Block. Fehler bei ungültiger Länge (kein Vielfaches von 16) oder kaputtem Padding.
fn aes128_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Result<Vec<u8>> {
    use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    type Decryptor = cbc::Decryptor<aes::Aes128>;
    let dec = Decryptor::new_from_slices(key, iv)
        .map_err(|_| Error::extract("HLS", "AES-128-Schlüssel/IV ungültig"))?;
    dec.decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| Error::extract("HLS", "AES-128-Entschlüsselung fehlgeschlagen"))
}

/// Lädt den 16-Byte-AES-128-Schlüssel und baut den Entschlüsselungs-Kontext. Die
/// Schlüssel-URL wird absolut aufgelöst und derselben SSRF-Prüfung wie Segmente
/// unterworfen (nur http/https, kein gesperrtes IP-Literal). Die Antwort ist auf
/// wenige Bytes gedeckelt.
async fn load_aes128_ctx(
    client: &reqwest::Client,
    key: &HlsKey,
    text: &str,
    media_url: &str,
) -> Result<Aes128Ctx> {
    let uri = key
        .uri
        .as_deref()
        .ok_or_else(|| Error::extract("HLS", "EXT-X-KEY ohne URI"))?;
    let key_url = resolve_url(uri, media_url)
        .ok_or_else(|| Error::extract("HLS", "Schlüssel-URL unauflösbar"))?;
    if !is_http_url(&key_url) || crate::http::host_is_forbidden_ip_literal(&key_url) {
        return Err(Error::extract("HLS", "Schlüssel-URL nicht erlaubt"));
    }
    // Schlüssel nur vom selben Origin wie die Playlist laden.
    if !same_origin(&key_url, media_url) {
        return Err(Error::extract("HLS", "Schlüssel-URL auf fremdem Origin"));
    }

    // Schlüssel ohne Referer laden (kein Header-Leak, falls doch umgeleitet wird)
    // und jede Umleitung verwerfen — ein Schlüssel wird nicht per Redirect geholt.
    let resp = crate::http::get_retry(client, &key_url, None)
        .await?
        .error_for_status()?;
    if resp.url().as_str() != key_url {
        return Err(Error::extract("HLS", "Schlüssel-Abruf wurde umgeleitet"));
    }
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if buf.len() + chunk.len() > 1024 {
            return Err(Error::extract("HLS", "AES-128-Schlüssel zu groß"));
        }
        buf.extend_from_slice(&chunk);
    }
    if buf.len() != 16 {
        return Err(Error::extract(
            "HLS",
            "AES-128-Schlüssel ist nicht 16 Byte lang",
        ));
    }
    let mut kb = [0u8; 16];
    kb.copy_from_slice(&buf);
    Ok(Aes128Ctx {
        key: kb,
        declared_iv: key.iv,
        media_sequence: parse_media_sequence(text),
    })
}

/// Lädt eine HLS-Media-Playlist segmentweise selbst, hängt die Segmente zu einer
/// `.part.ts` zusammen und remuxt sie mit ffmpeg nach *out_path*. Unverschlüsselte
/// und AES-128-CBC-Playlists werden nativ bedient (Letztere werden nach dem Laden
/// entschlüsselt).
///
/// `Ok(true)`: erfolgreich (inkl. AES-128). `Ok(false)`: nicht zuständig
/// (`SAMPLE-AES`/unbekannte Verschlüsselung oder keine Segmente) — der Aufrufer
/// soll auf den normalen ffmpeg-Mux zurückfallen.
async fn download_hls_native(
    client: &reqwest::Client,
    media_url: &str,
    referer: Option<&str>,
    out_path: &Path,
    cancel: &Option<CancellationToken>,
    progress: Option<&ProgressSink>,
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
    // Verschlüsselung klassifizieren: AES-128 nativ entschlüsseln, alles andere
    // (SAMPLE-AES, Schlüsselrotation, unbekannt) ffmpeg überlassen.
    let cipher = match hls_encryption(&text) {
        HlsEncryption::None => None,
        HlsEncryption::Unsupported => return Ok(false),
        HlsEncryption::Aes128(key) => Some(load_aes128_ctx(client, &key, &text, media_url).await?),
    };
    let segments = media_segment_urls(&text, media_url);
    if segments.is_empty() {
        return Ok(false);
    }

    if let Some(dir) = out_path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let part = out_path.with_extension("part.ts");

    report(
        progress,
        Progress::Started {
            total_segments: Some(segments.len()),
        },
    );
    let result = fetch_segments_to(
        client,
        &segments,
        referer,
        cipher.as_ref(),
        &part,
        cancel,
        progress,
    )
    .await;
    if let Err(e) = result {
        tokio::fs::remove_file(&part).await.ok();
        return Err(e);
    }

    // Zusammengesetzten TS-Stream nach out_path remuxen; die lokale Datei ist
    // vertrauenswürdig, daher file-Protokoll erlaubt.
    report(progress, Progress::Muxing);
    let remux = remux_local(&part, out_path, cancel).await;
    tokio::fs::remove_file(&part).await.ok();
    remux.map(|_| true)
}

/// Höchstzahl Segmente, die aus einer Playlist gelesen werden.
const MAX_SEGMENTS: usize = 50_000;
/// Obergrenze pro Segment.
const MAX_SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
/// Engere Grenze für verschlüsselte Segmente (der Krypto-Pfad puffert komplett).
const MAX_ENCRYPTED_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
/// Obergrenze über alle Segmente zusammen.
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Holt alle Segmente nacheinander (mit Retry, streamend) und schreibt sie
/// fortlaufend in *part*. Jedes Segment wird vor dem Abruf validiert (nur
/// http/https, keine gesperrten IP-Literale), muss nach einer etwaigen AES-128-
/// Entschlüsselung mit dem MPEG-TS-Sync-Byte `0x47` beginnen und unterliegt
/// Pro-Segment- und Gesamt-Größengrenzen.
///
/// Ist *cipher* gesetzt, wird jedes (verschlüsselte) Segment vollständig gepuffert,
/// mit AES-128-CBC entschlüsselt (segmentweises IV) und erst der Klartext geprüft
/// und geschrieben. Ohne *cipher* läuft der bisherige streamende Pfad.
async fn fetch_segments_to(
    client: &reqwest::Client,
    segments: &[String],
    referer: Option<&str>,
    cipher: Option<&Aes128Ctx>,
    part: &Path,
    cancel: &Option<CancellationToken>,
    progress: Option<&ProgressSink>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    if segments.len() > MAX_SEGMENTS {
        return Err(Error::extract("HLS", "zu viele Segmente"));
    }

    let mut file = tokio::fs::File::create(part).await?;
    let mut total: u64 = 0;
    let total_segments = segments.len();

    for (index, seg) in segments.iter().enumerate() {
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

        if let Some(ctx) = cipher {
            // Verschlüsseltes Segment vollständig puffern — der Sync-Byte-Check und
            // das Schreiben greifen erst auf dem entschlüsselten Klartext.
            let mut enc: Vec<u8> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                seg_bytes += chunk.len() as u64;
                total += chunk.len() as u64;
                // Der Krypto-Pfad puffert das ganze Segment im Speicher → engerer
                // Cap als beim streamenden Klartext-Pfad.
                if seg_bytes > MAX_ENCRYPTED_SEGMENT_BYTES || total > MAX_TOTAL_BYTES {
                    return Err(Error::extract(
                        "HLS",
                        "verschlüsseltes Segment überschreitet Größenlimit",
                    ));
                }
                enc.extend_from_slice(&chunk);
            }
            let plain = aes128_cbc_decrypt(&ctx.key, &ctx.iv_for(index), &enc)?;
            if plain.first() != Some(&0x47) {
                return Err(Error::extract(
                    "HLS",
                    "Segment ist kein MPEG-TS (nativer Pfad nicht anwendbar)",
                ));
            }
            file.write_all(&plain).await?;
        } else {
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
        report(
            progress,
            Progress::Segment {
                done: index + 1,
                total: total_segments,
                bytes: total,
            },
        );
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
    fn progress_senke_erhaelt_events() {
        use std::sync::{Arc, Mutex};
        let log: Arc<Mutex<Vec<Progress>>> = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let sink: ProgressSink = Arc::new(move |e| log2.lock().unwrap().push(e));
        report(Some(&sink), Progress::Resolving);
        report(
            Some(&sink),
            Progress::Started {
                total_segments: Some(3),
            },
        );
        report(
            Some(&sink),
            Progress::Segment {
                done: 1,
                total: 3,
                bytes: 100,
            },
        );
        report(Some(&sink), Progress::Finished);
        let got = log.lock().unwrap().clone();
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], Progress::Resolving);
        assert_eq!(
            got[2],
            Progress::Segment {
                done: 1,
                total: 3,
                bytes: 100
            }
        );
        assert_eq!(got[3], Progress::Finished);
    }

    #[test]
    fn progress_none_ist_noop() {
        // Ohne Senke darf report nichts tun und nicht paniken.
        report(None, Progress::Resolving);
        report(None, Progress::Finished);
    }

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

    /// Klassifiziert eine Playlist und bricht ab, wenn das Ergebnis kein AES-128 ist.
    fn aes128_key_of(text: &str) -> HlsKey {
        match hls_encryption(text) {
            HlsEncryption::Aes128(k) => k,
            _ => panic!("erwartete AES-128-Klassifikation"),
        }
    }

    #[test]
    fn klassifiziert_verschluesselung() {
        // Unverschlüsselt: kein KEY bzw. nur METHOD=NONE.
        assert!(matches!(
            hls_encryption("#EXTM3U\nseg0.ts\n"),
            HlsEncryption::None
        ));
        assert!(matches!(
            hls_encryption("#EXTM3U\nseg0.ts\n#EXT-X-KEY:METHOD=NONE\n"),
            HlsEncryption::None
        ));
        // AES-128 → nativ.
        assert!(matches!(
            hls_encryption("#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\nseg0.ts\n"),
            HlsEncryption::Aes128(_)
        ));
        // Bypass-Versuch: METHOD=NONE nur als Substring in einer Quoted-URI.
        assert!(matches!(
            hls_encryption(
                "#EXT-X-KEY:METHOD=AES-128,URI=\"https://x/k?a=METHOD=NONE\"\nseg0.ts\n"
            ),
            HlsEncryption::Aes128(_)
        ));
        // Unparsebare KEY-Zeile ohne METHOD → fail-closed (nicht nativ).
        assert!(matches!(
            hls_encryption("#EXT-X-KEY:URI=\"k\"\nseg0.ts\n"),
            HlsEncryption::Unsupported
        ));
        // Mehrere verschiedene Schlüssel (Rotation) → nicht nativ.
        assert!(matches!(
            hls_encryption(
                "#EXT-X-KEY:METHOD=AES-128,URI=\"k1\"\nseg0.ts\n#EXT-X-KEY:METHOD=AES-128,URI=\"k2\"\nseg1.ts\n"
            ),
            HlsEncryption::Unsupported
        ));
    }

    #[test]
    fn parst_ext_x_key_mit_iv() {
        let key = aes128_key_of(
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://cdn/key.bin\",IV=0x000102030405060708090A0B0C0D0E0F\nseg0.ts\n",
        );
        assert_eq!(key.method, "AES-128");
        assert_eq!(key.uri.as_deref(), Some("https://cdn/key.bin"));
        assert_eq!(
            key.iv,
            Some([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
        );
    }

    #[test]
    fn parst_ext_x_key_ohne_iv() {
        let key = aes128_key_of("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\nseg0.ts\n");
        assert_eq!(key.method, "AES-128");
        assert_eq!(key.uri.as_deref(), Some("key.bin"));
        assert_eq!(key.iv, None);
    }

    #[test]
    fn sample_aes_bleibt_ffmpeg() {
        assert!(matches!(
            hls_encryption("#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\nseg0.ts\n"),
            HlsEncryption::Unsupported
        ));
    }

    #[test]
    fn iv_aus_sequenznummer() {
        assert_eq!(iv_from_sequence(0), [0u8; 16]);
        // Sequenz 1 → letzte 8 Byte Big-Endian = …00 01.
        let mut erwartet = [0u8; 16];
        erwartet[15] = 1;
        assert_eq!(iv_from_sequence(1), erwartet);
        // Höherwertige Bytes landen im Big-Endian-Teil ab Byte 8.
        let iv = iv_from_sequence(0x0102_0304_0506_0708);
        assert_eq!(&iv[8..], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&iv[..8], &[0u8; 8]);
    }

    #[test]
    fn media_sequence_default_und_wert() {
        assert_eq!(parse_media_sequence("#EXTM3U\nseg0.ts\n"), 0);
        assert_eq!(
            parse_media_sequence("#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:42\nseg0.ts\n"),
            42
        );
    }

    #[test]
    fn aes128_cbc_roundtrip() {
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        type Encryptor = cbc::Encryptor<aes::Aes128>;

        let key = [0x11u8; 16];
        let iv = [0x22u8; 16];
        let klartext = b"Genau 16 Byte!!!"; // exakt ein Block

        let enc = Encryptor::new_from_slices(&key, &iv).unwrap();
        let geheim = enc.encrypt_padded_vec_mut::<Pkcs7>(klartext);
        // PKCS7 fügt bei voller Blockgröße einen weiteren Padding-Block an.
        assert_eq!(geheim.len(), 32);

        let zurueck = aes128_cbc_decrypt(&key, &iv, &geheim).unwrap();
        assert_eq!(zurueck, klartext);

        // Falscher Schlüssel → Padding stimmt (fast sicher) nicht → Fehler.
        assert!(aes128_cbc_decrypt(&[0x33u8; 16], &iv, &geheim).is_err());
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
