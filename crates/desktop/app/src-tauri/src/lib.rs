//! Tauri-Backend der vindex-Desktop-App.
//!
//! Dünne Command-Schicht über die `vindex-desktop`-Facade: jedes `#[tauri::command]`
//! mappt auf die Facade bzw. `vindex_core` und reicht serialisierbare DTOs ans
//! Frontend. **Keine** eigene Netz-/Scraping-/Download-Logik hier — die lebt im Kern.

mod config;
mod downloads;
mod logbuf;
mod stream_proxy;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use vindex_desktop::{core, Backend};

/// Gehaltener Zustand: die Facade (HTTP-Client), der lokale Referer-Proxy, der
/// Download-Manager und die bearbeitbaren Desktop-Einstellungen.
struct AppState {
    backend: Backend,
    proxy: Arc<stream_proxy::Proxy>,
    downloads: Arc<downloads::Downloads>,
    config: Mutex<config::DesktopConfig>,
    logs: logbuf::LogBuf,
}

impl AppState {
    /// Effektiver Download-Ordner: Desktop-Config-Override, sonst Kern-Default.
    fn download_dir(&self) -> PathBuf {
        let cfg = self.config.lock().unwrap();
        match cfg.download_path.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => PathBuf::from(p),
            None => self.backend.settings().download_path,
        }
    }

    /// Download-Overrides aus der Config (Sprache/Qualität/Ordner) + nativ/ffmpeg-frei.
    fn overrides(&self) -> vindex_desktop::DownloadOverrides {
        use std::str::FromStr;
        let cfg = self.config.lock().unwrap();
        // Egress-/DNS-Invariante: ist ein Proxy oder obscura aktiv, MUSS der native
        // HLS-Pfad genutzt werden. Der ffmpeg-Pfad zöge die Playlist direkt — am
        // 1.1.1.1-Resolver und am Egress-Proxy vorbei. Die Schalter dürfen das nicht
        // aushebeln. Der Gate liest den **effektiven** Zustand über thread-sichere
        // Accessors (`proxy_configured()` = OnceLock-Pool, `obscura_enabled()` = RwLock)
        // plus die Config-Absicht — **nie** direkt aus dem (nicht thread-sicheren) Env.
        let locked = egress_locked(&cfg);
        let native_hls = locked || cfg.native_hls.unwrap_or(true);
        let no_ffmpeg = locked || cfg.no_ffmpeg.unwrap_or(true);
        vindex_desktop::DownloadOverrides {
            language: cfg
                .language
                .as_deref()
                .and_then(|l| core::model::Language::from_str(l).ok()),
            max_quality: cfg
                .max_quality
                .as_deref()
                .and_then(|q| core::model::Quality::from_str(q).ok()),
            min_quality: cfg
                .min_quality
                .as_deref()
                .and_then(|q| core::model::Quality::from_str(q).ok()),
            out_dir: cfg
                .download_path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
            native_hls: Some(native_hls),
            no_ffmpeg: Some(no_ffmpeg),
            hosters: None,
        }
    }
}

/// Ist Egress-Schutz gewollt oder aktiv? Effektiver Zustand (Proxy-Pool/obscura)
/// ODER Config-Absicht (`cfg.proxy`/`cfg.obscura`, greift erst nach Neustart). Gemeinsame
/// Basis für die Download-Overrides und das Update-Gate — nie fail-open bei gesetzter Absicht.
fn egress_locked(cfg: &config::DesktopConfig) -> bool {
    let has = |o: &Option<String>| o.as_deref().map(str::trim).is_some_and(|s| !s.is_empty());
    core::http::proxy_configured()
        || core::browser::obscura_enabled()
        || has(&cfg.proxy)
        || has(&cfg.obscura)
}

/// Ein Suchtreffer fürs Frontend (serialisierbare Spiegelung von `SearchHit`).
#[derive(Serialize)]
struct SearchHitDto {
    title: String,
    url: String,
    /// Herkunft der Liste: "db" (lokaler Index) oder "live" (Portal gescrapt).
    source: &'static str,
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
    let (hits, from_index) = state
        .backend
        .search(&keyword)
        .await
        .map_err(|e| e.to_string())?;
    let source = if from_index { "db" } else { "live" };
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            title: h.title,
            url: h.url,
            source,
        })
        .collect())
}

#[tauri::command]
async fn catalog(portal: String, state: State<'_, AppState>) -> Result<Vec<SearchHitDto>, String> {
    let (hits, from_index) = state
        .backend
        .catalog(&portal)
        .await
        .map_err(|e| e.to_string())?;
    let source = if from_index { "db" } else { "live" };
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            title: h.title,
            url: h.url,
            source,
        })
        .collect())
}

/// Discover: nach Genre oder Jahr gefilterte Titel-Liste (ohne Filter = Katalog).
#[tauri::command]
async fn discover(
    portal: String,
    genre: Option<String>,
    year: Option<u32>,
    language: Option<String>,
    sort: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchHitDto>, String> {
    let (hits, from_index) = state
        .backend
        .discover(
            &portal,
            genre.as_deref(),
            year,
            language.as_deref(),
            sort.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    let source = if from_index { "db" } else { "live" };
    Ok(hits
        .into_iter()
        .map(|h| SearchHitDto {
            title: h.title,
            url: h.url,
            source,
        })
        .collect())
}

/// Portal-URL eines Titels per Name — für Bibliotheks-Cover, wenn keine URL
/// gemerkt ist. `null`, wenn nichts im Index passt.
#[tauri::command]
fn title_url_by_name(name: String) -> Option<String> {
    vindex_desktop::index_url_by_name(&name)
}

/// Herkunft eines einzelnen Titels: "db" (im Index) oder "live".
#[tauri::command]
fn data_source(url: String) -> &'static str {
    if vindex_desktop::index_has_title(&url) {
        "db"
    } else {
        "live"
    }
}

/// Ein Portal im Index: Name und Titelzahl (`null` = DB nicht vorhanden).
#[derive(Serialize)]
struct IndexPortalDto {
    portal: &'static str,
    titles: Option<i64>,
}

/// Wo die Index-DBs liegen und was drinsteht — für die Einstellungen.
#[derive(Serialize)]
struct IndexInfoDto {
    dir: String,
    portals: Vec<IndexPortalDto>,
}

#[tauri::command]
fn index_info() -> IndexInfoDto {
    let portals = ["aniworld", "serienstream", "filmo"]
        .iter()
        .map(|&p| IndexPortalDto {
            portal: p,
            titles: vindex_desktop::index_portal_titles(p),
        })
        .collect();
    IndexInfoDto {
        dir: vindex_desktop::index_dir(),
        portals,
    }
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

/// Fenster dauerhaft im Vordergrund halten (Mini-Player / „immer oben").
#[tauri::command]
fn set_always_on_top(on: bool, window: tauri::WebviewWindow) -> Result<(), String> {
    window.set_always_on_top(on).map_err(|e| e.to_string())
}

/// Fortschrittsbalken auf dem Fenster-/Taskleisten-Icon (0–100), `None` = aus.
/// Best-effort — abhängig vom Desktop (Unity/Windows/macOS ja, GNOME oft nicht).
#[tauri::command]
fn set_taskbar_progress(window: tauri::WebviewWindow, progress: Option<u32>) {
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    let state = match progress {
        Some(p) => ProgressBarState {
            status: Some(ProgressBarStatus::Normal),
            progress: Some(p.min(100) as u64),
        },
        None => ProgressBarState {
            status: Some(ProgressBarStatus::None),
            progress: None,
        },
    };
    let _ = window.set_progress_bar(state);
}

/// Verschiebt das Fenster in die untere rechte Ecke (für den Mini-Player).
#[tauri::command]
fn snap_bottom_right(window: tauri::WebviewWindow) {
    use tauri_plugin_positioner::{Position, WindowExt};
    let _ = window.move_window(Position::BottomRight);
}

/// System-Locale (z. B. „de-DE") für künftige i18n; `None`, wenn nicht ermittelbar.
#[tauri::command]
fn os_locale() -> Option<String> {
    tauri_plugin_os::locale()
}

/// Verfügbare Update-Info fürs Frontend.
#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    current: String,
    notes: Option<String>,
}

/// Prüft die GitHub-Releases auf ein neueres, signiertes Build. `None` = aktuell.
/// Bei gesetztem/aktivem Proxy/obscura wird die Prüfung abgelehnt (fail-closed, Privatsphäre).
#[tauri::command]
async fn check_update(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    // Guard vor jedem await auswerten und den Lock sofort wieder freigeben.
    if egress_locked(&state.config.lock().unwrap()) {
        return Err("Update-Prüfung unter Proxy/obscura deaktiviert (Privatsphäre)".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await.map_err(|e| e.to_string())? {
        Some(u) => Ok(Some(UpdateInfo {
            version: u.version.clone(),
            current: u.current_version.clone(),
            notes: u.body.clone(),
        })),
        None => Ok(None),
    }
}

/// Lädt das zuvor geprüfte Update (muss der bestätigten Version entsprechen),
/// installiert es (Signatur wird geprüft) und startet neu.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    version: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if egress_locked(&state.config.lock().unwrap()) {
        return Err("Update unter Proxy/obscura deaktiviert (Privatsphäre)".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("kein Update verfügbar".into());
    };
    // Consent-Abgleich: nur die Version installieren, die der Nutzer bestätigt hat
    // (schützt vor einem Release, das zwischen Prüfen und Installieren erscheint).
    if update.version != version {
        return Err(format!(
            "Version hat sich geändert ({} statt {}) — abgebrochen",
            update.version, version
        ));
    }
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

/// Letzte Log-Zeilen fürs Diagnose-Panel (redigiert durch die Logging-Disziplin).
#[tauri::command]
fn recent_logs(state: State<'_, AppState>) -> Vec<String> {
    logbuf::snapshot(&state.logs)
}

/// Eine für einen Titel verfügbare Sprache (fürs Frontend).
#[derive(Serialize)]
struct LangOption {
    /// `FromStr`-kompatibler Wert, der an resolve_playback/start_download geht.
    value: String,
    label: String,
    sub_only: bool,
}

/// Verfügbare Sprachen eines Titels (Film- oder Episoden-URL) — inkl. „nur Untertitel".
#[tauri::command]
async fn available_languages(
    url: String,
    state: State<'_, AppState>,
) -> Result<Vec<LangOption>, String> {
    let langs = state
        .backend
        .available_languages(&url)
        .await
        .map_err(|e| e.to_string())?;
    Ok(langs
        .into_iter()
        .map(|l| LangOption {
            value: l.ui_label().to_string(),
            label: l.ui_label().to_string(),
            sub_only: l.is_subtitle_only(),
        })
        .collect())
}

#[tauri::command]
async fn resolve_playback(
    url: String,
    lang: Option<String>,
    state: State<'_, AppState>,
) -> Result<PlaybackDto, String> {
    use std::str::FromStr;
    // Sprache/Qualität wie beim Downloader (Settings + Desktop-Config-Overrides).
    let s = state.backend.settings();
    let mut ov = state.overrides();
    // Optionale Sprach-Vorwahl aus dem Frontend gewinnt für diese Wiedergabe.
    if let Some(l) = lang
        .as_deref()
        .filter(|l| !l.is_empty())
        .and_then(|l| core::model::Language::from_str(l).ok())
    {
        ov.language = Some(l);
    }
    let mut prefs = vindex_desktop::prefs_from_settings(&s, "stream", &ov);
    // Fallback über die übrigen Sprachen, damit nicht nichts kommt, wenn genau die
    // Wunschsprache fehlt (Wunsch bleibt zuerst).
    vindex_desktop::with_language_fallback(&mut prefs);

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
    year: Option<String>,
    fsk: Option<u32>,
    country: Option<String>,
    imdb: Option<String>,
    genres: Vec<String>,
    directors: Vec<String>,
    actors: Vec<String>,
    producers: Vec<String>,
}

#[tauri::command]
async fn title_meta(url: String, state: State<'_, AppState>) -> Result<TitleMetaDto, String> {
    let m = state
        .backend
        .title_meta(&url)
        .await
        .map_err(|e| e.to_string())?;
    // Poster durch den lokalen Proxy laden (SSRF-geprüft; die CSP erlaubt Bilder
    // nur von 127.0.0.1, nicht von beliebigen https-Hosts). IMDB bleibt eine externe
    // Link-URL (wird extern geöffnet, nicht ins WebView geladen).
    let poster = m.poster.map(|u| state.proxy.stream_url(&u, None));
    Ok(TitleMetaDto {
        poster,
        description: m.description,
        year: m.year,
        fsk: m.fsk,
        country: m.country,
        imdb: m.imdb,
        genres: m.genres,
        directors: m.directors,
        actors: m.actors,
        producers: m.producers,
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
    lang: Option<String>,
    hoster: Option<String>,
    max_quality: Option<String>,
    state: State<'_, AppState>,
) -> u64 {
    use std::str::FromStr;
    let settings = state.backend.settings();
    let mut overrides = state.overrides();
    // Optionale Sprach-Vorwahl aus dem Detail gilt auch für den Download.
    if let Some(l) = lang
        .as_deref()
        .filter(|l| !l.is_empty())
        .and_then(|l| core::model::Language::from_str(l).ok())
    {
        overrides.language = Some(l);
    }
    // Optionale Hoster-Vorwahl: der gewählte Hoster zuerst, dann die Default-Reihenfolge
    // als Fallback (so bleibt der Download robust, wenn der Wunsch-Hoster fehlt).
    if let Some(h) = hoster
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(vindex_desktop::Backend::hoster_by_name)
    {
        let mut order = vec![h];
        for d in settings.effective_hosters() {
            if d != h {
                order.push(d);
            }
        }
        overrides.hosters = Some(order);
    }
    // Optionale Qualitäts-Obergrenze pro Download.
    if let Some(q) = max_quality
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|q| core::model::Quality::from_str(q).ok())
    {
        overrides.max_quality = Some(q);
    }
    downloads::start(
        state.downloads.clone(),
        app,
        url,
        file_name,
        title,
        settings,
        overrides,
    )
}

#[tauri::command]
fn cancel_download(id: u64, state: State<'_, AppState>) {
    state.downloads.cancel(id);
}

/// Entfernt alle abgeschlossenen Jobs (fertig/Fehler/abgebrochen) aus der Liste.
#[tauri::command]
fn clear_finished_downloads(state: State<'_, AppState>) {
    state.downloads.clear_finished();
}

#[tauri::command]
fn pause_download(app: tauri::AppHandle, id: u64, state: State<'_, AppState>) {
    if let Some(v) = state.downloads.pause(id) {
        let _ = app.emit("download:update", v);
    }
}

#[tauri::command]
fn resume_download(app: tauri::AppHandle, id: u64, state: State<'_, AppState>) {
    if let Some(v) = state.downloads.resume(id) {
        let _ = app.emit("download:update", v);
    }
}

/// Bandbreitenbegrenzung eines einzelnen Downloads (KiB/s; 0 = unbegrenzt).
#[tauri::command]
fn set_download_rate(id: u64, kib: u64, state: State<'_, AppState>) {
    state.downloads.set_rate(id, kib.saturating_mul(1024));
}

/// Verschiebt einen wartenden Download in der Warteschlange (hoch/runter).
#[tauri::command]
fn move_download(id: u64, up: bool, state: State<'_, AppState>) {
    state.downloads.move_in_queue(id, up);
}

/// Globale Bandbreitenbegrenzung setzen (KiB/s; 0 = unbegrenzt) und persistieren.
#[tauri::command]
fn set_bandwidth_limit(kib: u64, state: State<'_, AppState>) -> Result<(), String> {
    state.downloads.set_bandwidth(kib.saturating_mul(1024));
    let mut cfg = state.config.lock().unwrap();
    cfg.bandwidth_kib = if kib == 0 { None } else { Some(kib) };
    config::save(&cfg)
}

/// Mindestabstand zwischen Download-Starts setzen (ms; 0 = aus) und persistieren.
#[tauri::command]
fn set_download_delay(ms: u64, state: State<'_, AppState>) -> Result<(), String> {
    state.downloads.set_start_delay(ms);
    let mut cfg = state.config.lock().unwrap();
    cfg.download_delay_ms = if ms == 0 { None } else { Some(ms) };
    config::save(&cfg)
}

/// Netzwerk-Status fürs Frontend. DNS ist fest 1.1.1.1 (nicht konfigurierbar).
#[derive(Serialize)]
struct NetworkStatusDto {
    dns: String,
    impersonate: bool,
    obscura: bool,
    obscura_path: Option<String>,
    proxy: Option<String>,
}

#[tauri::command]
fn network_status(state: State<'_, AppState>) -> NetworkStatusDto {
    let cfg = state.config.lock().unwrap();
    NetworkStatusDto {
        dns: "1.1.1.1".into(),
        impersonate: core::http::impersonate_enabled(),
        obscura: core::browser::obscura_enabled(),
        obscura_path: cfg.obscura.clone(),
        proxy: cfg.proxy.clone(),
    }
}

/// Anti-Bot Stufe 2 (TLS-Impersonate) live umschalten + persistieren.
#[tauri::command]
fn set_impersonate(on: bool, state: State<'_, AppState>) -> Result<(), String> {
    core::http::set_impersonate(on);
    std::env::set_var("VINDEX_IMPERSONATE", if on { "1" } else { "0" });
    let mut cfg = state.config.lock().unwrap();
    cfg.impersonate = Some(on);
    config::save(&cfg)
}

/// Anti-Bot Stufe 3 (obscura-Pfad) live setzen/löschen + persistieren.
/// Mit gesetztem Egress-Proxy wird Stufe 3 abgelehnt (sie würde den Proxy umgehen —
/// genau das lehnt auch der Kern beim Abruf ab; so stimmt die Zusage mit der Durchsetzung).
#[tauri::command]
fn set_obscura(path: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let p = path.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut cfg = state.config.lock().unwrap();
    if p.is_some()
        && cfg
            .proxy
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
    {
        return Err("Mit gesetztem Egress-Proxy ist Stufe 3 (obscura) nicht möglich.".into());
    }
    core::browser::set_obscura_path(p.clone());
    std::env::set_var("VINDEX_OBSCURA", p.as_deref().unwrap_or(""));
    cfg.obscura = p;
    config::save(&cfg)
}

/// Egress-Proxy setzen/löschen. Der HTTP-Client ist bereits gebaut → wirkt nach Neustart.
/// Ein gesetzter Proxy schaltet Stufe 3 (obscura) ab, damit sie nicht umgangen wird.
#[tauri::command]
fn set_proxy(url: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let p = url.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(ref value) = p {
        if !core::http::proxy_string_ok(value) {
            return Err("Ungültige Proxy-URL (erwartet z. B. http://host:port).".into());
        }
    }
    std::env::set_var("VINDEX_PROXY", p.as_deref().unwrap_or(""));
    let mut cfg = state.config.lock().unwrap();
    cfg.proxy = p.clone();
    // Proxy und obscura schließen sich aus: beim Setzen eines Proxys Stufe 3 abschalten.
    if p.is_some() && cfg.obscura.is_some() {
        core::browser::set_obscura_path(None);
        std::env::set_var("VINDEX_OBSCURA", "");
        cfg.obscura = None;
    }
    config::save(&cfg)
}

/// Spiegelt die Netzwerk-Felder der Desktop-Config in die ENV, damit der Kern
/// (`Settings::load`) sie konsistent anwendet. Nur gesetzte Felder; DNS bleibt 1.1.1.1.
fn apply_network_env(cfg: &config::DesktopConfig) {
    if let Some(on) = cfg.impersonate {
        std::env::set_var("VINDEX_IMPERSONATE", if on { "1" } else { "0" });
    }
    if let Some(p) = cfg
        .obscura
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        std::env::set_var("VINDEX_OBSCURA", p);
    }
    if let Some(p) = cfg
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        std::env::set_var("VINDEX_PROXY", p);
    }
}

/// Lädt einen (fehlgeschlagenen/abgebrochenen) Auftrag mit denselben Parametern erneut.
#[tauri::command]
fn retry_download(app: tauri::AppHandle, id: u64, state: State<'_, AppState>) -> Option<u64> {
    let (url, file_name, title) = state.downloads.params(id)?;
    let settings = state.backend.settings();
    let overrides = state.overrides();
    Some(downloads::start(
        state.downloads.clone(),
        app,
        url,
        file_name,
        title,
        settings,
        overrides,
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

/// Ergebnis eines Ordner-Imports fürs Frontend.
#[derive(Serialize)]
struct ImportResult {
    imported: usize,
    skipped: usize,
}

/// Importiert Mediendateien aus einem (vom Nutzer gewählten) Ordner in die
/// Bibliothek: kopiert jede reguläre Mediendatei in den Download-Ordner, sofern
/// dort noch nicht vorhanden. Nicht-destruktiv; Symlinks und Unterordner werden
/// übersprungen, der Zielname ist immer nur der Basisname (kein Traversal).
#[tauri::command]
fn import_from_folder(path: String, state: State<'_, AppState>) -> Result<ImportResult, String> {
    let src = std::fs::canonicalize(std::path::PathBuf::from(&path)).map_err(|e| e.to_string())?;
    if !src.is_dir() {
        return Err("kein Ordner".into());
    }
    let dir = state.download_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dir_canon = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    if src == dir_canon {
        return Err("Quelle ist bereits der Download-Ordner".into());
    }
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for entry in std::fs::read_dir(&src).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_media(name) {
            continue;
        }
        let dst = dir.join(name);
        // Ziel exklusiv anlegen: `create_new` schlägt fehl, wenn der Pfad existiert
        // (auch als Symlink) — kein TOCTOU und kein Schreiben durch einen
        // untergeschobenen Symlink. Die Quelle `p` ist oben als reguläre Datei geprüft.
        let mut dstf = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dst)
        {
            Ok(f) => f,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let mut srcf = match std::fs::File::open(&p) {
            Ok(f) => f,
            Err(_) => {
                let _ = std::fs::remove_file(&dst);
                skipped += 1;
                continue;
            }
        };
        if std::io::copy(&mut srcf, &mut dstf).is_ok() {
            imported += 1;
        } else {
            drop(dstf);
            let _ = std::fs::remove_file(&dst);
            skipped += 1;
        }
    }
    Ok(ImportResult { imported, skipped })
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
    let dir = state.download_dir();
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
    let dir = state.download_dir();
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    let canon = safe_media_path(&dir, &canon_dir, &name)
        .ok_or_else(|| "ungültige oder unbekannte Mediendatei".to_string())?;
    std::fs::remove_file(&canon).map_err(|e| e.to_string())
}

/// Benennt eine Mediendatei um. Quelle wird wie `delete` validiert; der Zielname
/// muss ein reiner Dateiname mit Medien-Endung sein (keine Trenner/Traversal) und
/// darf nicht existieren — so bleibt die Datei im Download-Ordner und auffindbar.
#[tauri::command]
fn rename_library_item(
    name: String,
    new_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let dir = state.download_dir();
    let canon_dir = std::fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    let src = safe_media_path(&dir, &canon_dir, &name)
        .ok_or_else(|| "ungültige oder unbekannte Mediendatei".to_string())?;
    let new_name = new_name.trim();
    if new_name.is_empty()
        || new_name.contains('/')
        || new_name.contains('\\')
        || new_name.contains("..")
        || new_name.chars().any(|c| c.is_control())
        || !is_media(new_name)
    {
        return Err("ungültiger Zielname (Medien-Endung behalten)".into());
    }
    let dst = canon_dir.join(new_name);
    if dst.parent() != Some(canon_dir.as_path()) {
        return Err("ungültiger Zielpfad".into());
    }
    if dst.exists() {
        return Err("Zieldatei existiert bereits".into());
    }
    std::fs::rename(&src, &dst).map_err(|e| e.to_string())
}

/// Validiert `name` wie `library`/`delete` und gibt den kanonisierten Pfad zurück
/// — gemeinsame Basis für Öffnen und „im Ordner zeigen" (kein Frontend-Opener-Grant).
fn library_path(name: &str, state: &AppState) -> Result<std::path::PathBuf, String> {
    let dir = state.download_dir();
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

/// Öffnet einen externen Link im System-Browser. Nur `https` und eine kleine
/// Allowlist (IMDB) — die URL stammt aus gescraptem Inhalt.
#[tauri::command]
fn open_external(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if url.chars().any(|c| c.is_control()) {
        return Err("ungültige URL".into());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "ungültige URL".to_string())?;
    if parsed.scheme() != "https" {
        return Err("nur https erlaubt".into());
    }
    // Keine eingebetteten Zugangsdaten (user:pass@host-Trick).
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL mit Zugangsdaten nicht erlaubt".into());
    }
    let host = parsed.host_str().unwrap_or("");
    if host != "imdb.com" && !host.ends_with(".imdb.com") {
        return Err("Host nicht erlaubt".into());
    }
    if !parsed.path().starts_with("/title/") {
        return Err("Pfad nicht erlaubt".into());
    }
    // Die normalisierte URL öffnen, nicht das Rohinput (Parser-Differential vermeiden).
    app.opener()
        .open_url(parsed.to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Öffnet den nativen Ordner-Dialog und gibt den gewählten Pfad zurück (oder `None`).
#[tauri::command]
async fn pick_download_folder(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    rx.await
        .ok()
        .flatten()
        .and_then(|fp| fp.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Plattform-Info fürs Frontend (zur Anpassung der Oberfläche).
#[derive(Serialize)]
struct PlatformInfo {
    os: String,   // "linux" | "macos" | "windows" | …
    arch: String, // "x86_64" | "aarch64" | …
}

#[tauri::command]
fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// Sucht ein mitgeliefertes Binary (`ffmpeg`/`ffprobe`/`obscura`) neben der App bzw.
/// im `binaries/`-Ordner — in Dev relativ zum Crate, im Bundle neben der Exe/Resources.
/// Gibt den ersten existierenden absoluten Pfad zurück.
fn bundled_tool(name: &str) -> Option<String> {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_ref().and_then(|e| e.parent());
    let fname = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = exe_dir {
        // Neben der installierten Exe bzw. in den Bundle-Resources (nicht
        // fremd-beschreibbar) — der einzige Suchpfad im Release.
        cands.push(d.join(&fname));
        cands.push(d.join("binaries").join(&fname));
        cands.push(d.join("resources").join("binaries").join(&fname));
    }
    // Compile-Zeit-Quellpfad NUR im Debug-Build (sonst Binary-Planting-Risiko: der
    // Pfad der Build-Maschine könnte auf dem Zielsystem fremd beschreibbar sein).
    #[cfg(debug_assertions)]
    cands.push(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(&fname),
    );
    cands
        .into_iter()
        .find(|p| safe_tool_path(p))
        .map(|p| p.to_string_lossy().into_owned())
}

/// Ein Kandidat ist nur sicher, wenn er eine **reguläre, nicht-symbolische** Datei ist
/// und (auf Unix) weder die Datei noch ihr Verzeichnis group-/world-schreibbar sind —
/// sonst könnte ein anderer Nutzer ein Binary unterschieben (Code-Execution).
fn safe_tool_path(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // 0o022 = group- oder world-writable.
        if meta.mode() & 0o022 != 0 {
            return false;
        }
        if let Some(dir) = path.parent() {
            match std::fs::metadata(dir) {
                Ok(dm) if dm.mode() & 0o022 == 0 => {}
                _ => return false,
            }
        }
    }
    true
}

/// Setzt `VINDEX_FFMPEG`/`VINDEX_FFPROBE`/`VINDEX_OBSCURA` auf mitgelieferte Binaries,
/// falls vorhanden und nicht bereits per ENV vorgegeben. So nutzt der Kern automatisch
/// die gebündelten Programme (Dev wie Bundle); fehlen sie, bleibt der PATH-Fallback.
fn wire_bundled_binaries() {
    for (tool, var) in [
        ("ffmpeg", "VINDEX_FFMPEG"),
        ("ffprobe", "VINDEX_FFPROBE"),
        ("obscura", "VINDEX_OBSCURA"),
    ] {
        let already = std::env::var(var).ok().filter(|s| !s.trim().is_empty());
        if already.is_none() {
            if let Some(path) = bundled_tool(tool) {
                tracing::info!("gebündeltes {tool} erkannt");
                std::env::set_var(var, path);
            }
        }
    }
}

/// Findet den Ordner mit den mitgelieferten `<portal>.db.zst`. Im Bundle die
/// Resources (`resource_dir/index`), im Debug zusätzlich das Repo-`index/`.
fn bundled_zst_dir(app: &tauri::App) -> Option<std::path::PathBuf> {
    let mut cands: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        cands.push(res.join("index"));
    }
    // Repo-`index/` nur im Debug-Build (Pfad der Build-Maschine).
    #[cfg(debug_assertions)]
    cands.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../index"));
    cands.into_iter().find(|p| {
        ["aniworld", "serienstream", "filmo"]
            .iter()
            .any(|portal| p.join(format!("{portal}.db.zst")).is_file())
    })
}

/// Entpackt die mitgelieferten Index-Schnappschüsse in den App-Datenordner und
/// setzt `VINDEX_INDEX_DIR` darauf, damit die Facade offline aus dem Index liest.
/// Eine per ENV gesetzte Dir gewinnt (z. B. Entwicklerwunsch) und wird respektiert.
fn setup_index(app: &tauri::App) {
    if std::env::var("VINDEX_INDEX_DIR")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return;
    }
    let Some(src) = bundled_zst_dir(app) else {
        tracing::info!("keine Index-Schnappschüsse gefunden — Facade bleibt live");
        return;
    };
    let Ok(base) = app.path().app_data_dir() else {
        tracing::warn!("app_data_dir nicht verfügbar — Index übersprungen");
        return;
    };
    let dest = base.join("index");
    let (src_s, dest_s) = (src.to_string_lossy(), dest.to_string_lossy().into_owned());
    match vindex_desktop::ensure_index_unpacked(&src_s, &dest_s) {
        Ok(()) => {
            std::env::set_var("VINDEX_INDEX_DIR", &dest_s);
            tracing::info!("Index bereit in {dest_s}");
        }
        Err(e) => tracing::warn!("Index entpacken fehlgeschlagen: {e}"),
    }
}

/// Ist der Autostart (Login) aktiviert?
#[tauri::command]
fn autostart_enabled(app: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Autostart ein-/ausschalten.
#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let m = app.autolaunch();
    if enabled {
        m.enable().map_err(|e| e.to_string())
    } else {
        m.disable().map_err(|e| e.to_string())
    }
}

/// Bearbeitbare Desktop-Einstellungen lesen.
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> config::DesktopConfig {
    state.config.lock().unwrap().clone()
}

/// Bearbeitbare Desktop-Einstellungen speichern (persistent) und übernehmen.
#[tauri::command]
fn set_config(cfg: config::DesktopConfig, state: State<'_, AppState>) -> Result<(), String> {
    config::save(&cfg)?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
fn app_settings(state: State<'_, AppState>) -> SettingsDto {
    let s = state.backend.settings();
    SettingsDto {
        download_path: state.download_dir().display().to_string(),
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

/// Tray-Icon mit Menü (Öffnen/Beenden) — Downloads laufen im Hintergrund weiter.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };
    let show = MenuItem::with_id(app, "show", "Öffnen", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("vindex")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
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
    // Ringpuffer für die letzten Log-Zeilen (Diagnose-Panel); teet nach stderr.
    let logs = logbuf::new_buffer();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(logbuf::BufferMaker::new(logs.clone()))
        .try_init();
    let logs_for_state = logs.clone();

    // Mitgelieferte Binaries (ffmpeg/ffprobe/obscura) erkennen und als ENV setzen —
    // vor dem Laden der Kern-Settings, damit u. a. obscura (Stufe 3) greift.
    wire_bundled_binaries();
    // Desktop-Netzwerk-Einstellungen VOR dem Laden der Kern-Settings (und vor dem
    // Client-Bau) in die ENV spiegeln, damit der Kern sie konsistent anwendet.
    apply_network_env(&config::load());
    // Settings früh laden (setzt u. a. Impersonate/obscura aus den ENV).
    let _ = core::config::Settings::load();

    tauri::Builder::default()
        // Single-Instance MUSS zuerst kommen: ein zweiter Start fokussiert das
        // vorhandene Fenster und reicht seine Argumente (ggf. vindex://-URL) weiter.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            let _ = app.emit("deep-link", args);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_os::init())
        .setup(move |app| {
            build_tray(app.handle())?;

            // Deep-Links (vindex://…): im Dev die Schemes registrieren und eingehende
            // URLs ans Frontend weiterreichen (Semantik der URLs legt das Frontend fest).
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let _ = app.deep_link().register_all();
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    let urls: Vec<String> = event.urls().iter().map(|u| u.to_string()).collect();
                    let _ = handle.emit("deep-link", urls);
                });
            }

            // Mitgelieferte Index-Schnappschüsse in den App-Datenordner entpacken
            // und VINDEX_INDEX_DIR setzen, bevor das Backend das erste Mal liest.
            setup_index(app);

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
            let cfg = config::load();
            // Gespeicherte Bandbreitenbegrenzung auf den Laufzeit-Limiter anwenden.
            if let Some(kib) = cfg.bandwidth_kib {
                downloads.set_bandwidth(kib.saturating_mul(1024));
            }
            if let Some(ms) = cfg.download_delay_ms {
                downloads.set_start_delay(ms);
            }
            app.manage(AppState {
                backend,
                proxy,
                downloads,
                config: Mutex::new(cfg),
                logs: logs_for_state.clone(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            check_update,
            install_update,
            snap_bottom_right,
            os_locale,
            search,
            catalog,
            discover,
            title_url_by_name,
            data_source,
            index_info,
            seasons,
            episodes,
            resolve_playback,
            set_always_on_top,
            set_taskbar_progress,
            recent_logs,
            available_languages,
            title_meta,
            cover,
            proxy_prefix,
            aniskip,
            start_download,
            cancel_download,
            clear_finished_downloads,
            pause_download,
            resume_download,
            set_bandwidth_limit,
            set_download_delay,
            set_download_rate,
            move_download,
            network_status,
            set_impersonate,
            set_obscura,
            set_proxy,
            open_external,
            pick_download_folder,
            platform_info,
            retry_download,
            list_downloads,
            library,
            delete_library_item,
            rename_library_item,
            import_from_folder,
            open_library_item,
            local_media_url,
            reveal_library_item,
            autostart_enabled,
            set_autostart,
            get_config,
            set_config,
            app_settings
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Start der vindex-Desktop-App");
}
