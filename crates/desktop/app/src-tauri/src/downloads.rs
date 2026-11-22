//! Download-Manager der App: Warteschlange, Fortschritt und Abbruch.
//!
//! Jeder Job läuft in einer eigenen Task über `core::download::download_url`.
//! Der Fortschritt kommt über die `ProgressSink` des Kerns herein und geht als
//! `download:update`-Event ans Frontend. Die eigentliche Download-/Remux-Logik
//! lebt komplett im Kern — hier ist nur Orchestrierung.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use vindex_desktop::core;
use vindex_desktop::{prefs_from_settings, CancellationToken, DownloadOverrides, Progress};

/// Wie viele Downloads gleichzeitig laufen dürfen (der Rest bleibt in der Warteschlange).
const MAX_CONCURRENT: usize = 3;

/// Frontend-Sicht auf einen Job (serialisierbar, geht per Event raus).
#[derive(Clone, Serialize)]
pub struct JobView {
    pub id: u64,
    pub title: String,
    /// queued | resolving | running | muxing | done | error | cancelled
    pub status: String,
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
    pub error: Option<String>,
}

struct Job {
    view: JobView,
    cancel: CancellationToken,
    /// Original-Parameter, um den Auftrag erneut zu laden (Retry).
    url: String,
    file_name: String,
    title: String,
}

pub struct Downloads {
    jobs: Mutex<HashMap<u64, Job>>,
    next: AtomicU64,
    sem: Arc<Semaphore>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            jobs: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }
}

impl Downloads {
    pub fn list(&self) -> Vec<JobView> {
        let g = self.jobs.lock().unwrap();
        let mut v: Vec<JobView> = g.values().map(|j| j.view.clone()).collect();
        v.sort_by_key(|j| j.id);
        v
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.jobs.lock().unwrap().get(&id) {
            j.cancel.cancel();
        }
    }

    /// Original-Parameter eines Jobs (für Retry): (url, file_name, title).
    pub fn params(&self, id: u64) -> Option<(String, String, String)> {
        let g = self.jobs.lock().unwrap();
        let j = g.get(&id)?;
        Some((j.url.clone(), j.file_name.clone(), j.title.clone()))
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut JobView)) -> Option<JobView> {
        let mut g = self.jobs.lock().unwrap();
        let j = g.get_mut(&id)?;
        f(&mut j.view);
        Some(j.view.clone())
    }
}

/// Reiht einen Download ein und startet ihn. Gibt die Job-ID zurück.
pub fn start(
    downloads: Arc<Downloads>,
    app: AppHandle,
    url: String,
    file_name: String,
    title: String,
    settings: core::config::Settings,
) -> u64 {
    let id = downloads.next.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = CancellationToken::new();
    let view = JobView {
        id,
        title: title.clone(),
        status: "queued".into(),
        done: 0,
        total: 0,
        bytes: 0,
        error: None,
    };
    downloads.jobs.lock().unwrap().insert(
        id,
        Job {
            view: view.clone(),
            cancel: cancel.clone(),
            url: url.clone(),
            file_name: file_name.clone(),
            title,
        },
    );
    emit(&app, &view);

    // Desktop lädt bewusst nativ + ffmpeg-frei: echte Segment-Fortschritte und
    // der native TS→MKV-Remux (kein ffmpeg nötig; andere Codecs → roher .ts).
    let overrides = DownloadOverrides {
        native_hls: Some(true),
        no_ffmpeg: Some(true),
        ..DownloadOverrides::default()
    };
    let prefs = prefs_from_settings(&settings, file_name, &overrides);

    let sem = downloads.sem.clone();

    // Tauris Runtime, nicht tokio::spawn direkt: der Command läuft evtl. im
    // GTK-Thread ohne aktiven Tokio-Reaktor.
    tauri::async_runtime::spawn(async move {
        // Auf einen freien Slot warten — bis dahin bleibt der Job „queued".
        let _permit = sem.acquire_owned().await.ok();
        if cancel.is_cancelled() {
            if let Some(u) = downloads.update(id, |v| v.status = "cancelled".into()) {
                emit(&app, &u);
            }
            return;
        }

        // Sink: Fortschritt in die Job-Sicht schreiben und als Event rausschicken.
        let app_sink = app.clone();
        let dl_sink = downloads.clone();
        let sink: core::download::ProgressSink = Arc::new(move |p: Progress| {
            if let Some(updated) = dl_sink.update(id, |v| apply(v, &p)) {
                emit(&app_sink, &updated);
            }
        });

        let client = match core::http::browser_client() {
            Ok(c) => c,
            Err(e) => {
                finish_err(&downloads, &app, id, e.to_string());
                return;
            }
        };

        let result =
            core::download::download_url(&client, &url, &prefs, Some(cancel.clone()), Some(&sink))
                .await;

        match result {
            Ok(_path) => {
                if let Some(u) = downloads.update(id, |v| v.status = "done".into()) {
                    emit(&app, &u);
                    notify_done(&app, &u.title);
                }
            }
            Err(e) => {
                if cancel.is_cancelled() {
                    if let Some(u) = downloads.update(id, |v| v.status = "cancelled".into()) {
                        emit(&app, &u);
                    }
                } else {
                    finish_err(&downloads, &app, id, e.to_string());
                }
            }
        }
    });

    id
}

fn apply(v: &mut JobView, p: &Progress) {
    match p {
        Progress::Resolving => v.status = "resolving".into(),
        Progress::Started { total_segments } => {
            v.status = "running".into();
            if let Some(t) = total_segments {
                v.total = *t;
            }
        }
        Progress::Segment { done, total, bytes } => {
            v.status = "running".into();
            v.done = *done;
            v.total = *total;
            v.bytes = *bytes;
        }
        Progress::Muxing => v.status = "muxing".into(),
        Progress::Finished => v.status = "done".into(),
    }
}

fn finish_err(d: &Arc<Downloads>, app: &AppHandle, id: u64, msg: String) {
    if let Some(u) = d.update(id, |v| {
        v.status = "error".into();
        v.error = Some(msg.clone());
    }) {
        emit(app, &u);
    }
}

fn emit(app: &AppHandle, v: &JobView) {
    let _ = app.emit("download:update", v);
}

/// System-Benachrichtigung bei fertigem Download.
fn notify_done(app: &AppHandle, title: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("Download fertig")
        .body(title)
        .show();
}
