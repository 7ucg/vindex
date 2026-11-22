//! Tauri-Backend der vindex-Desktop-App.
//!
//! Dünne Command-Schicht über die `vindex-desktop`-Facade: jedes `#[tauri::command]`
//! mappt auf die Facade bzw. `vindex_core` und reicht serialisierbare DTOs ans
//! Frontend. **Keine** eigene Netz-/Scraping-/Download-Logik hier — die lebt im Kern.

use serde::Serialize;
use tauri::{Manager, State};
use vindex_desktop::{core, Backend};

/// Gehaltener Zustand: die Facade (ein HTTP-Client) für die Lebensdauer der App.
struct AppState {
    backend: Backend,
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
    // Settings früh laden (setzt u. a. den obscura-Pfad in core::browser).
    let _ = core::config::Settings::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let backend = Backend::new().map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("Backend-Init fehlgeschlagen: {e}"))
            })?;
            app.manage(AppState { backend });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![app_info, search, app_settings])
        .run(tauri::generate_context!())
        .expect("Fehler beim Start der vindex-Desktop-App");
}
