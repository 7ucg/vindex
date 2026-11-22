//! Tauri-Backend der vindex-Desktop-App.
//!
//! Dünne Command-Schicht über die `vindex-desktop`-Facade: jedes `#[tauri::command]`
//! mappt auf die Facade bzw. `vindex_core` und reicht serialisierbare DTOs ans
//! Frontend. **Keine** eigene Netz-/Scraping-/Download-Logik hier — die lebt im Kern.

mod downloads;
mod stream_proxy;

use std::sync::Arc;

use serde::Serialize;
use tauri::{Manager, State};
use vindex_desktop::{core, Backend};

/// Gehaltener Zustand: die Facade (HTTP-Client), der lokale Referer-Proxy und
/// der Download-Manager.
struct AppState {
    backend: Backend,
    proxy: Arc<stream_proxy::Proxy>,
    downloads: Arc<downloads::Downloads>,
}

/// Ein Suchtreffer fürs Frontend (serialisierbare Spiegelung von `SearchHit`).
#[derive(Serialize)]
struct SearchHitDto {
    title: String,
    url: String,
}

/// Laufzeit-Einstellungen, soweit das Frontend sie zeigt/braucht.
#[derive(Serialize)]
struct SettingsDto {
    download_path: String,
    language: String,
    max_quality: String,
    min_quality: String,
    hosters: Vec<String>,
    native_hls: bool,
    no_ffmpeg: bool,
}

/// Eine Episode fürs Frontend (serialisierbare Spiegelung von `core::model::Episode`).
#[derive(Serialize)]
struct EpisodeDto {
    url: String,
    title: String,
    season: u32,
    index_in_season: u32,
}

/// Ein abspielbereiter Stream: die proxied URL fürs `<video>`/hls.js + HLS-Flag.
#[derive(Serialize)]
struct PlaybackDto {
    url: String,
    is_hls: bool,
}

/// Ein heruntergeladener Titel in der Bibliothek.
#[derive(Serialize)]
struct LibraryEntryDto {
    name: String,
    size_bytes: u64,
    modified: u64,
    /// Absoluter Pfad (fürs lokale Abspielen über das Asset-Protokoll).
    path: String,
}

/// Ein AniSkip-Intervall fürs Frontend.
#[derive(Serialize)]
struct SkipDto {
    start: f64,
    end: f64,
    kind: String,
}

/// App-Metadaten fürs „Über"-Panel und den Health-Check beim Start.
#[derive(Serialize)]
struct AppInfo {
    name: &'static str,
    version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "vindex",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
async fn search(keyword: String, state: State<'_, AppState>) -> Result<Vec<SearchHitDto>, String> {
    let hits = state
        .backend
        .search(&keyword)
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            title: h.title,
            url: h.url,
        })
        .collect())
}

#[tauri::command]
async fn catalog(portal: String, state: State<'_, AppState>) -> Result<Vec<SearchHitDto>, String> {
    let hits = state
        .backend
        .catalog(&portal)
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            title: h.title,
            url: h.url,
        })
        .collect())
}

#[tauri::command]
async fn seasons(url: String, state: State<'_, AppState>) -> Result<Vec<u32>, String> {
    state.backend.seasons(&url).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn episodes(
    url: String,
    season: u32,
    state: State<'_, AppState>,
) -> Result<Vec<EpisodeDto>, String> {
    let eps = state
        .backend
        .episodes(&url, season)
        .await
        .map_err(|e| e.to_string())?;
    Ok(eps
        .into_iter()
        .map(|e| EpisodeDto {
            url: e.url,
            title: e.title,
            season: e.season,
            index_in_season: e.index_in_season,
        })
        .collect())
}

#[tauri::command]
async fn resolve_playback(url: String, state: State<'_, AppState>) -> Result<PlaybackDto, String> {
    // Sprache/Hoster/Qualität wie beim Downloader (aus den Settings).
    let s = state.backend.settings();
    let prefs = vindex_desktop::prefs_from_settings(
        &s,
        "stream",
        &vindex_desktop::DownloadOverrides::default(),
    );

    let resolved = state
        .backend
        .resolve_playback(&url, &prefs)
        .await
        .map_err(|e| {
            tracing::warn!("resolve_playback fehlgeschlagen: {e}");
            e.to_string()
        })?;

    // Nur Host loggen, nie die volle URL (enthält signierte Token/Query).
    let host = reqwest::Url::parse(&resolved.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| "?".into());
    tracing::debug!(
        is_hls = resolved.is_hls,
        has_referer = resolved.referer.is_some(),
        host = %host,
        "Stream aufgelöst"
    );

    let proxied = state
        .proxy
        .stream_url(&resolved.url, resolved.referer.as_deref());
    Ok(PlaybackDto {
        url: proxied,
        is_hls: resolved.is_hls,
    })
}

#[derive(Serialize)]
struct TitleMetaDto {
    poster: Option<String>,
    description: Option<String>,
}

#[tauri::command]
async fn title_meta(url: String, state: State<'_, AppState>) -> Result<TitleMetaDto, String> {
    let (poster, description) = state
        .backend
        .title_meta(&url)
        .await
        .map_err(|e| e.to_string())?;
    // Poster durch den lokalen Proxy laden (SSRF-geprüft; die CSP erlaubt Bilder
    // nur von 127.0.0.1, nicht von beliebigen https-Hosts).
    let poster = poster.map(|u| state.proxy.stream_url(&u, None));
    Ok(TitleMetaDto {
        poster,
        description,
    })
}

#[tauri::command]
async fn cover(url: String, state: State<'_, AppState>) -> Result<Option<String>, String> {
    state.backend.cover(&url).await.map_err(|e| e.to_string())
}

/// Prefix, mit dem das Frontend Bild-Proxy-URLs baut: `<base>/s?t=<token>&u=`.
#[tauri::command]
fn proxy_prefix(state: State<'_, AppState>) -> String {
    state.proxy.image_prefix()
}

#[tauri::command]
async fn aniskip(
    url: String,
    season: u32,
    episode: u32,
    state: State<'_, AppState>,
) -> Result<Vec<SkipDto>, String> {
    let skips = state
        .backend
        .aniskip(&url, season, episode)
        .await
        .map_err(|e| e.to_string())?;
    Ok(skips
        .into_iter()
        .map(|s| SkipDto {
            start: s.start,
            end: s.end,
            kind: s.kind,
        })
        .collect())
}

#[tauri::command]
fn start_download(
    app: tauri::AppHandle,
    url: String,
    file_name: String,
    title: String,
    state: State<'_, AppState>,
) -> u64 {
    let settings = state.backend.settings();
    downloads::start(
        state.downloads.clone(),
        app,
        url,
        file_name,
        title,
        settings,
    )
}

#[tauri::command]
fn cancel_download(id: u64, state: State<'_, AppState>) {
    state.downloads.cancel(id);
}

/// Lädt einen (fehlgeschlagenen/abgebrochenen) Auftrag mit denselben Parametern erneut.
#[tauri::command]
fn retry_download(app: tauri::AppHandle, id: u64, state: State<'_, AppState>) -> Option<u64> {
    let (url, file_name, title) = state.downloads.params(id)?;
    let settings = state.backend.settings();
    Some(downloads::start(
        state.downloads.clone(),
        app,
        url,
        file_name,
        title,
        settings,
    ))
}

#[tauri::command]
fn list_downloads(state: State<'_, AppState>) -> Vec<downloads::JobView> {
    state.downloads.list()
}

/// Bekannte Medien-Endungen — nur solche Dateien zeigt/spielt/löscht die Bibliothek.
fn is_media(name: &str) -> bool {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "mkv" | "mp4" | "ts" | "webm" | "m4v" | "mov" | "avi" | "m4a" | "mp3" | "aac"
    )
}

/// Validiert einen Bibliotheks-Dateinamen und gibt den **kanonisierten** Pfad
/// zurück: reiner Mediendateiname (keine Trenner/`..`), reguläre Datei (kein
/// Symlink/Verzeichnis), und der kanonisierte Elternpfad ist exakt `canon_dir`.
fn safe_media_path(
    dir: &std::path::Path,
    canon_dir: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    if !is_media(name) {
        return None;
    }
    let path = dir.join(name);
    if !std::fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    let canon = std::fs::canonicalize(&path).ok()?;
    if canon.parent() != Some(canon_dir) {
        return None;
    }
    Some(canon)
}

#[tauri::command]
fn library(app: tauri::AppHandle, state: State<'_, AppState>) -> Vec<LibraryEntryDto> {
    let dir = state.backend.settings().download_path;
    let Ok(canon_dir) = std::fs::canonicalize(&dir) else {
        return Vec::new();
    };
    let scope = app.asset_protocol_scope();
    core::library::list(&dir)
        .into_iter()
        .filter_map(|e| {
            // Nur die validierte, kanonisierte Datei freigeben (kein Symlink-Ausbruch).
            let canon = safe_media_path(&dir, &canon_dir, &e.name)?;
            let _ = scope.allow_file(&canon);
            Some(LibraryEntryDto {
                name: e.name,
                size_bytes: e.size_bytes,
                modified: e.modified,
                path: canon.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

/// Löscht eine Mediendatei aus der Bibliothek — gleiche Validierung wie `library`.
#[tauri::command]
fn delete_library_item(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.backend.settings().download_path;
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    let canon = safe_media_path(&dir, &canon_dir, &name)
        .ok_or_else(|| "ungültige oder unbekannte Mediendatei".to_string())?;
    std::fs::remove_file(&canon).map_err(|e| e.to_string())
}

/// Validiert `name` wie `library`/`delete` und gibt den kanonisierten Pfad zurück
/// — gemeinsame Basis für Öffnen und „im Ordner zeigen" (kein Frontend-Opener-Grant).
fn library_path(name: &str, state: &AppState) -> Result<std::path::PathBuf, String> {
    let dir = state.backend.settings().download_path;
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    safe_media_path(&dir, &canon_dir, name)
        .ok_or_else(|| "ungültige oder unbekannte Mediendatei".to_string())
}

/// Spielt eine Bibliotheksdatei ab (nach Validierung). Bevorzugt mpv — spielt
/// MKV/TS zuverlässig und ist der weit bessere Player als der System-Default;
/// ist mpv nicht da, Fallback auf den System-Opener.
#[tauri::command]
fn open_library_item(
    app: tauri::AppHandle,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let canon = library_path(&name, &state)?;
    // mpv loslösen (eigenes Fenster, blockiert die App nicht).
    let mpv = std::process::Command::new("mpv")
        .arg("--force-window=yes")
        .arg(&canon)
        .spawn();
    if mpv.is_ok() {
        return Ok(());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canon.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Proxy-URL, über die der In-Window-Player eine lokale Datei per HTTP+Range
/// spielt (nach Validierung). GStreamer im WebView spielt so MKV/TS.
#[tauri::command]
fn local_media_url(name: String, state: State<'_, AppState>) -> Result<String, String> {
    let canon = library_path(&name, &state)?;
    Ok(state.proxy.local_url(canon))
}

/// Zeigt eine Bibliotheksdatei im Dateimanager (vom Backend, nach Validierung).
#[tauri::command]
fn reveal_library_item(
    app: tauri::AppHandle,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let canon = library_path(&name, &state)?;
    app.opener()
        .reveal_item_in_dir(canon)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn app_settings(state: State<'_, AppState>) -> SettingsDto {
    let s = state.backend.settings();
    SettingsDto {
        download_path: s.download_path.display().to_string(),
        language: format!("{:?}", s.language),
        max_quality: format!("{:?}", s.max_quality),
        min_quality: format!("{:?}", s.min_quality),
        hosters: s
            .effective_hosters()
            .iter()
            .map(|h| format!("{:?}", h))
            .collect(),
        native_hls: s.native_hls,
        no_ffmpeg: s.no_ffmpeg,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logging nach stderr (sichtbar im `tauri dev`-Terminal). Level über
    // VINDEX_LOG bzw. RUST_LOG, Standard: info fürs App- und vindex-Modul.
    let filter = tracing_subscriber::EnvFilter::try_from_env("VINDEX_LOG")
        .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "info,vindex_desktop_app_lib=debug,vindex_core=debug",
            )
        });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();

    // Settings früh laden (setzt u. a. den obscura-Pfad in core::browser).
    let _ = core::config::Settings::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let backend = Backend::new().map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("Backend-Init fehlgeschlagen: {e}"))
            })?;
            let proxy = tauri::async_runtime::block_on(stream_proxy::start()).map_err(|e| {
                Box::<dyn std::error::Error>::from(format!(
                    "Stream-Proxy-Start fehlgeschlagen: {e}"
                ))
            })?;
            // Lokale Wiedergabe wird pro Mediendatei freigegeben (siehe `library`),
            // nicht pauschal für den ganzen Ordner.
            let downloads = Arc::new(downloads::Downloads::default());
            app.manage(AppState {
                backend,
                proxy,
                downloads,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            search,
            catalog,
            seasons,
            episodes,
            resolve_playback,
            title_meta,
            cover,
            proxy_prefix,
            aniskip,
            start_download,
            cancel_download,
            retry_download,
            list_downloads,
            library,
            delete_library_item,
            open_library_item,
            local_media_url,
            reveal_library_item,
            app_settings
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Start der vindex-Desktop-App");
}
