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
use tokio::sync::Notify;
use vindex_desktop::core;
use vindex_desktop::{
    prefs_from_settings, with_language_fallback, CancellationToken, DownloadControl,
    DownloadOverrides, PauseGate, Progress, RateLimiter,
};

/// Wie viele Downloads gleichzeitig laufen dürfen (der Rest bleibt in der Warteschlange).
const MAX_CONCURRENT: usize = 3;

/// Frontend-Sicht auf einen Job (serialisierbar, geht per Event raus).
#[derive(Clone, Serialize)]
pub struct JobView {
    pub id: u64,
    pub title: String,
    /// queued | resolving | running | paused | muxing | done | error | cancelled
    pub status: String,
    pub done: usize,
    pub total: usize,
    pub bytes: u64,
    pub error: Option<String>,
    /// Position in der Warteschlange (0 = als Nächstes), `None` wenn nicht wartend.
    pub queue_pos: Option<u32>,
}

struct Job {
    view: JobView,
    cancel: CancellationToken,
    pause: Arc<PauseGate>,
    /// Bandbreitenbegrenzung nur dieses Downloads (0 = unbegrenzt).
    rate: Arc<RateLimiter>,
    /// Öffnet der Scheduler, wenn dieser Job einen Slot bekommt.
    gate: Arc<Notify>,
    /// Original-Parameter, um den Auftrag erneut zu laden (Retry).
    url: String,
    file_name: String,
    title: String,
}

/// Scheduler-Zustand: wartende Job-IDs (Reihenfolge = Priorität, Front zuerst)
/// und die Anzahl gerade laufender Downloads.
struct Sched {
    queue: Vec<u64>,
    running: usize,
}

pub struct Downloads {
    jobs: Mutex<HashMap<u64, Job>>,
    next: AtomicU64,
    /// Eigener Prioritäts-Scheduler statt Semaphore, damit die Warteschlange
    /// umsortierbar ist (der nächste freie Slot geht an die Queue-Front).
    sched: Mutex<Sched>,
    /// Globale Bandbreitenbegrenzung über alle Downloads (0 = unbegrenzt).
    global_rate: Arc<RateLimiter>,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            jobs: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            sched: Mutex::new(Sched {
                queue: Vec::new(),
                running: 0,
            }),
            global_rate: Arc::new(RateLimiter::new(0)),
        }
    }
}

impl Downloads {
    pub fn list(&self) -> Vec<JobView> {
        // Aktuelle Warteschlangen-Positionen (Front = 0) — sched zuerst sperren und
        // wieder freigeben, bevor jobs gesperrt wird (konsistente Sperrreihenfolge).
        let positions: HashMap<u64, u32> = {
            let s = self.sched.lock().unwrap();
            s.queue
                .iter()
                .enumerate()
                .map(|(i, &id)| (id, i as u32))
                .collect()
        };
        let g = self.jobs.lock().unwrap();
        let mut v: Vec<JobView> = g
            .values()
            .map(|j| {
                let mut view = j.view.clone();
                view.queue_pos = positions.get(&j.view.id).copied();
                view
            })
            .collect();
        v.sort_by_key(|j| j.id);
        v
    }

    /// Entfernt abgeschlossene Jobs (fertig/Fehler/abgebrochen) aus der Liste.
    pub fn clear_finished(&self) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|_, j| !matches!(j.view.status.as_str(), "done" | "error" | "cancelled"));
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.jobs.lock().unwrap().get(&id) {
            j.cancel.cancel();
        }
    }

    /// Pausiert einen laufenden Download (kooperativ im Segment-Loop).
    pub fn pause(&self, id: u64) -> Option<JobView> {
        let gate = self
            .jobs
            .lock()
            .unwrap()
            .get(&id)
            .map(|j| j.pause.clone())?;
        gate.pause();
        self.update(id, |v| {
            if matches!(v.status.as_str(), "running" | "resolving" | "queued") {
                v.status = "paused".into();
            }
        })
    }

    /// Setzt einen pausierten Download fort.
    pub fn resume(&self, id: u64) -> Option<JobView> {
        let gate = self
            .jobs
            .lock()
            .unwrap()
            .get(&id)
            .map(|j| j.pause.clone())?;
        gate.resume();
        self.update(id, |v| {
            if v.status == "paused" {
                v.status = "running".into();
            }
        })
    }

    /// Globale Bandbreitenbegrenzung setzen (Bytes/s, 0 = unbegrenzt).
    pub fn set_bandwidth(&self, bytes_per_sec: u64) {
        self.global_rate.set_rate(bytes_per_sec);
    }

    /// Bandbreitenbegrenzung eines einzelnen Downloads (Bytes/s, 0 = unbegrenzt).
    pub fn set_rate(&self, id: u64, bytes_per_sec: u64) {
        if let Some(r) = self.jobs.lock().unwrap().get(&id).map(|j| j.rate.clone()) {
            r.set_rate(bytes_per_sec);
        }
    }

    /// Verschiebt einen wartenden Job in der Warteschlange (nach oben/unten).
    /// Laufende Jobs stehen nicht in der Queue und bleiben unberührt.
    pub fn move_in_queue(&self, id: u64, up: bool) {
        let mut s = self.sched.lock().unwrap();
        if let Some(pos) = s.queue.iter().position(|&x| x == id) {
            let target = if up {
                pos.checked_sub(1)
            } else if pos + 1 < s.queue.len() {
                Some(pos + 1)
            } else {
                None
            };
            if let Some(t) = target {
                s.queue.swap(pos, t);
            }
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
    overrides: DownloadOverrides,
) -> u64 {
    let id = downloads.next.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = CancellationToken::new();
    let pause = Arc::new(PauseGate::new());
    let rate = Arc::new(RateLimiter::new(0));
    let gate = Arc::new(Notify::new());
    let view = JobView {
        id,
        title: title.clone(),
        status: "queued".into(),
        done: 0,
        total: 0,
        bytes: 0,
        error: None,
        queue_pos: None,
    };
    downloads.jobs.lock().unwrap().insert(
        id,
        Job {
            view: view.clone(),
            cancel: cancel.clone(),
            pause: pause.clone(),
            rate: rate.clone(),
            gate: gate.clone(),
            url: url.clone(),
            file_name: file_name.clone(),
            title,
        },
    );
    emit(&app, &view);

    // Overrides vom Aufrufer (Desktop-Config: Sprache/Qualität/Ordner); nativ +
    // ffmpeg-frei setzt der Aufrufer ohnehin (echte Fortschritte, kein ffmpeg).
    let mut prefs = prefs_from_settings(&settings, file_name, &overrides);
    // Fallback über die übrigen Sprachen — sonst scheitert der Download, wenn genau
    // die Wunschsprache beim Titel fehlt.
    with_language_fallback(&mut prefs);

    let global_rate = downloads.global_rate.clone();

    // In die Warteschlange einreihen; der Scheduler öffnet das Gate, sobald ein
    // Slot frei ist (Queue-Front zuerst — die Reihenfolge ist umsortierbar).
    downloads.sched.lock().unwrap().queue.push(id);
    pump(&downloads);

    // Tauris Runtime, nicht tokio::spawn direkt: der Command läuft evtl. im
    // GTK-Thread ohne aktiven Tokio-Reaktor.
    tauri::async_runtime::spawn(async move {
        // Auf den eigenen Slot warten (Gate) oder Abbruch, solange noch wartend.
        let got_slot = tokio::select! {
            _ = gate.notified() => true,
            _ = cancel.cancelled() => false,
        };
        if !got_slot {
            // Abbruch in der Warteschlange: war der Job schon aus der Queue gezogen
            // (Scheduler hat ihm einen Slot zugeteilt), diesen Slot freigeben.
            let claimed = {
                let mut s = downloads.sched.lock().unwrap();
                let before = s.queue.len();
                s.queue.retain(|&x| x != id);
                before == s.queue.len()
            };
            if let Some(u) = downloads.update(id, |v| v.status = "cancelled".into()) {
                emit(&app, &u);
            }
            if claimed {
                release_slot(&downloads);
            }
            return;
        }
        if cancel.is_cancelled() {
            if let Some(u) = downloads.update(id, |v| v.status = "cancelled".into()) {
                emit(&app, &u);
            }
            release_slot(&downloads);
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
                release_slot(&downloads);
                return;
            }
        };

        let ctl = DownloadControl {
            cancel: Some(cancel.clone()),
            pause: Some(pause.clone()),
            rate: Some(rate.clone()),
            global_rate: Some(global_rate.clone()),
        };
        let result =
            core::download::download_url_ctl(&client, &url, &prefs, ctl, Some(&sink)).await;

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
        release_slot(&downloads);
    });

    id
}

/// Füllt freie Slots: zieht so lange Jobs von der Queue-Front und öffnet ihr Gate,
/// bis `MAX_CONCURRENT` erreicht ist oder die Warteschlange leer ist.
fn pump(d: &Arc<Downloads>) {
    let mut to_signal = Vec::new();
    {
        let mut s = d.sched.lock().unwrap();
        while s.running < MAX_CONCURRENT && !s.queue.is_empty() {
            let id = s.queue.remove(0);
            s.running += 1;
            to_signal.push(id);
        }
    }
    for id in to_signal {
        match d.jobs.lock().unwrap().get(&id).map(|j| j.gate.clone()) {
            Some(gate) => gate.notify_one(),
            // Job ist verschwunden — den reservierten Slot wieder freigeben.
            None => release_slot(d),
        }
    }
}

/// Gibt einen belegten Slot frei und füllt ggf. mit dem nächsten Job nach.
fn release_slot(d: &Arc<Downloads>) {
    {
        let mut s = d.sched.lock().unwrap();
        if s.running > 0 {
            s.running -= 1;
        }
    }
    pump(d);
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

/// System-Benachrichtigung bei fertigem Download. Der Titel stammt letztlich aus
/// gescraptem Inhalt → Steuerzeichen raus, auf 120 Zeichen kürzen.
fn notify_done(app: &AppHandle, title: &str) {
    use tauri_plugin_notification::NotificationExt;
    let safe: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    let _ = app
        .notification()
        .builder()
        .title("Download fertig")
        .body(&safe)
        .show();
}
