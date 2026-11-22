//! HTTP-API / Daemon für vindex (axum).
//!
//! Funktionsfähige Teilmenge der Referenz-API (`docs/http-api.md`): Suche,
//! Provider-Metadaten, Download-Queue. Die `/api/*`-Routen liegen hinter einer
//! API-Key-Prüfung (siehe [`auth`]). SSO, Library, Auto-Sync, OpenAPI
//! (`utoipa`) und ein `tower_governor`-Rate-Limit folgen gemäß
//! `ARCHITECTURE.md` §9 (Schritt 11).

mod auth;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use vindex_core as core;

/// Obergrenze gleichzeitig wartender/laufender Jobs, damit ein Client die Queue
/// nicht unbegrenzt füllen kann.
const MAX_INFLIGHT_JOBS: usize = 100;

/// Obergrenze der gesamten Job-Liste (inkl. fertiger). Darüber werden die
/// abgeschlossenen Einträge verworfen, damit die Liste nicht unbegrenzt wächst.
const MAX_JOBS_RETAINED: usize = 500;

/// Status eines Queue-Eintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }
}

/// Ein Download-Auftrag.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub url: String,
    pub status: JobStatus,
    pub detail: String,
}

/// Geteilter Server-Zustand.
#[derive(Clone)]
pub struct AppState {
    client: reqwest::Client,
    jobs: Arc<Mutex<Vec<Job>>>,
    next_id: Arc<AtomicU64>,
    auth: auth::AuthConfig,
}

impl AppState {
    fn new() -> Self {
        let settings = core::config::Settings::load();
        AppState {
            client: core::http::browser_client().expect("http client"),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: auth::AuthConfig::from_settings(&settings),
        }
    }
}

/// Baut den Router mit frisch geladenem Zustand.
pub fn router() -> Router {
    router_with(AppState::new())
}

/// Baut den Router um einen gegebenen Zustand. `/health` bleibt offen, die
/// `/api/*`-Routen liegen hinter der API-Key-Middleware.
fn router_with(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/search", get(search))
        .route("/api/providers", get(providers))
        .route("/api/download", post(download))
        .route("/api/queue", get(queue))
        .layer(axum::middleware::from_fn_with_state(
            state.auth.clone(),
            auth::require_api_key,
        ));

    Router::new()
        .route("/health", get(health))
        .merge(api)
        .with_state(state)
}

/// Startet den Daemon auf *addr* (z. B. `127.0.0.1:8080`).
///
/// Ohne konfigurierte API-Keys wird nur ein localhost-Bind zugelassen; ein
/// öffentlicher Bind ohne Auth bricht mit Fehler ab.
pub async fn serve(addr: &str) -> anyhow::Result<()> {
    let state = AppState::new();
    let open = state.auth.is_open();

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;

    if open && !local.ip().is_loopback() {
        anyhow::bail!(
            "Daemon würde öffentlich auf {local} lauschen, aber VINDEX_API_KEYS ist \
             nicht gesetzt. Entweder an 127.0.0.1 binden oder API-Keys konfigurieren."
        );
    }
    if open {
        tracing::warn!("Kein API-Key gesetzt — /api ist ungeschützt (nur localhost).");
    }

    tracing::info!("vindex API lauscht auf http://{local}");
    axum::serve(listener, router_with(state)).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct SearchParams {
    keyword: String,
}

async fn search(
    State(state): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Json<serde_json::Value> {
    let needle = p.keyword.to_lowercase();
    match core::site::aniworld::fetch_catalog(&state.client).await {
        Ok(entries) => {
            let results: Vec<_> = entries
                .into_iter()
                .filter(|e| e.title.to_lowercase().contains(&needle))
                .take(50)
                .map(|e| {
                    json!({
                        "title": e.title,
                        "url": format!("{}/anime/stream/{}", core::model::BASE, e.slug),
                    })
                })
                .collect();
            Json(json!({ "results": results }))
        }
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

#[derive(Deserialize)]
struct ProvidersParams {
    url: String,
}

async fn providers(
    State(state): State<AppState>,
    Query(p): Query<ProvidersParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(e) = core::site::validate_source_url(&p.url) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e })));
    }
    let base = core::site::base_for_url(&p.url);
    match core::site::serienstream::provider_links(&state.client, base, &p.url).await {
        Ok(links) => {
            let mut out = serde_json::Map::new();
            for (lang, hosters) in links.by_language {
                let m: serde_json::Map<String, serde_json::Value> = hosters
                    .into_iter()
                    .map(|(h, u)| (h.as_str().to_string(), json!(u)))
                    .collect();
                out.insert(lang.as_str().to_string(), serde_json::Value::Object(m));
            }
            (StatusCode::OK, Json(json!({ "providers": out })))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(Deserialize)]
struct DownloadReq {
    url: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    quality: Option<String>,
}

async fn download(
    State(state): State<AppState>,
    Json(req): Json<DownloadReq>,
) -> (StatusCode, Json<serde_json::Value>) {
    use std::str::FromStr;

    // SSRF-Schutz: nur unterstützte Portale als Quell-URL zulassen.
    if let Err(e) = core::site::validate_source_url(&req.url) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": e })));
    }

    let settings = core::config::Settings::load();
    let language = req
        .language
        .and_then(|l| core::model::Language::from_str(&l).ok())
        .unwrap_or(settings.language);
    let hosters = match req
        .provider
        .and_then(|p| core::model::Hoster::from_str(&p).ok())
    {
        Some(h) => {
            let mut v = vec![h];
            v.extend(settings.fallback_order.iter().copied().filter(|x| *x != h));
            v
        }
        None => settings.effective_hosters(),
    };
    let max_quality = req
        .quality
        .and_then(|q| core::model::Quality::from_str(&q).ok())
        .unwrap_or(settings.max_quality);

    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut jobs = state.jobs.lock().await;
        let inflight = jobs
            .iter()
            .filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
            .count();
        if inflight >= MAX_INFLIGHT_JOBS {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "Queue voll, bitte später erneut versuchen" })),
            );
        }
        // Fertige Jobs beschneiden, sobald die Liste zu lang wird. Aktive Jobs
        // sind durch MAX_INFLIGHT_JOBS begrenzt, bleiben also erhalten.
        if jobs.len() >= MAX_JOBS_RETAINED {
            jobs.retain(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
        }
        jobs.push(Job {
            id,
            url: req.url.clone(),
            status: JobStatus::Queued,
            detail: String::new(),
        });
    }

    // Hintergrund-Worker.
    let worker_state = state.clone();
    let url = req.url.clone();
    tokio::spawn(async move {
        set_status(&worker_state, id, JobStatus::Running, "").await;
        let file_name = crate::file_name_from_url(&url);
        let prefs = core::download::DownloadPrefs {
            languages: vec![language],
            hosters,
            max_quality,
            min_quality: settings.min_quality,
            out_dir: settings.download_path.clone(),
            file_name,
        };
        match core::download::download_url(&worker_state.client, &url, &prefs, None).await {
            Ok(path) => {
                set_status(
                    &worker_state,
                    id,
                    JobStatus::Completed,
                    &path.to_string_lossy(),
                )
                .await
            }
            Err(e) => {
                tracing::warn!("Job {id} fehlgeschlagen: {e}");
                set_status(&worker_state, id, JobStatus::Failed, &e.to_string()).await
            }
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "queue_id": id, "status": "queued" })),
    )
}

async fn set_status(state: &AppState, id: u64, status: JobStatus, detail: &str) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
        job.status = status;
        if !detail.is_empty() {
            job.detail = detail.to_string();
        }
    }
}

async fn queue(State(state): State<AppState>) -> Json<serde_json::Value> {
    let jobs = state.jobs.lock().await;
    let items: Vec<_> = jobs
        .iter()
        .map(|j| {
            json!({
                "id": j.id,
                "url": j.url,
                "status": j.status.as_str(),
                "detail": public_detail(j),
            })
        })
        .collect();
    Json(json!({ "items": items }))
}

/// Nach außen sichtbares Detail eines Jobs. Für fertige Downloads nur der
/// Dateiname statt des vollen lokalen Pfads; fehlgeschlagene Jobs geben eine
/// knappe Meldung ohne internen Fehlertext/Pfad zurück.
fn public_detail(job: &Job) -> String {
    match job.status {
        JobStatus::Completed => std::path::Path::new(&job.detail)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        JobStatus::Failed => "Download fehlgeschlagen".to_string(),
        JobStatus::Queued | JobStatus::Running => String::new(),
    }
}

/// Leitet einen Dateinamen aus einer URL ab:
/// Episode → `Titel SxxEyy.mkv`, Filmo-Film → `Titel.mkv`.
pub fn file_name_from_url(url: &str) -> String {
    if core::site::is_filmo(url) {
        if let Some(rest) = url.split("/movies/").nth(1) {
            let slug = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
            if !slug.is_empty() {
                return format!("{}.mkv", core::naming::clean_title(&slug.replace('-', " ")));
            }
        }
        return "movie.mkv".into();
    }
    let slug = core::site::slug_from_url(url).unwrap_or_else(|| "episode".into());
    let season = capture_num(url, "/staffel-").unwrap_or(1);
    let episode = capture_num(url, "/episode-").unwrap_or(1);
    let title = core::naming::clean_title(&slug.replace('-', " "));
    format!("{title} S{season:02}E{episode:02}.mkv")
}

fn capture_num(url: &str, marker: &str) -> Option<u32> {
    url.split(marker)
        .nth(1)?
        .split('/')
        .next()?
        .parse::<u32>()
        .ok()
}
