//! HTTP-API / Daemon für vindex (axum).
//!
//! Funktionsfähige Teilmenge der Referenz-API (siehe README): Suche,
//! Provider-Metadaten, Download-Queue. Die `/api/*`-Routen liegen hinter einer
//! API-Key-Prüfung (siehe [`auth`]) und einem Request-Rate-Limit pro IP
//! (Sliding-Window, siehe [`RateLimiter`]). Eine OpenAPI-3-Spezifikation (via
//! [`utoipa`]) wird offen unter `/openapi.json` ausgeliefert. SSO, Library und
//! Auto-Sync folgen gemäß `ARCHITECTURE.md` §9 (Schritt 11).

mod auth;

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use utoipa::{IntoParams, Modify, OpenApi, ToSchema};
use vindex_core as core;

/// Obergrenze gleichzeitig wartender/laufender Jobs, damit ein Client die Queue
/// nicht unbegrenzt füllen kann.
const MAX_INFLIGHT_JOBS: usize = 100;

/// Obergrenze der gesamten Job-Liste (inkl. fertiger). Darüber werden die
/// abgeschlossenen Einträge verworfen, damit die Liste nicht unbegrenzt wächst.
const MAX_JOBS_RETAINED: usize = 500;

/// Standard-Rate-Limit: erlaubte Requests pro IP und Zeitfenster. Über
/// `VINDEX_RATE_LIMIT` überschreibbar.
const DEFAULT_RATE_LIMIT: u32 = 60;

/// Zeitfenster des Rate-Limits (Sliding-Window).
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

/// Liest das konfigurierte Rate-Limit aus `VINDEX_RATE_LIMIT`; ungültige oder
/// fehlende Werte fallen auf [`DEFAULT_RATE_LIMIT`] zurück.
fn rate_limit_from_env() -> u32 {
    std::env::var("VINDEX_RATE_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_RATE_LIMIT)
}

/// Sliding-Window-Rate-Limit pro Client-IP. Hält je IP die Zeitstempel der
/// Zugriffe im Fenster vor; ältere werden bei jedem Zugriff verworfen.
pub struct RateLimiter {
    max: u32,
    window: Duration,
    state: StdMutex<RateState>,
}

/// Interner Zustand des Limiters; `last_sweep` drosselt den O(n)-Aufräumlauf.
#[derive(Default)]
struct RateState {
    map: HashMap<IpAddr, VecDeque<Instant>>,
    last_sweep: Option<Instant>,
}

/// Obergrenze getrackter Schlüssel, damit die Map nicht unbegrenzt wächst.
const MAX_TRACKED_KEYS: usize = 100_000;

/// Normalisiert den Rate-Limit-Schlüssel: IPv4 unverändert, IPv6 auf das /64-Präfix
/// (die unteren 64 Bit genullt), damit ein einzelner /64-Block nicht durch
/// Adressrotation am Limit vorbeikommt.
fn normalize_rate_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            // IPv4-mapped (::ffff:a.b.c.d) zuerst auf die IPv4-Form bringen, sonst
            // würde die /64-Maskierung die eingebettete IPv4 nullen und alle
            // gemappten Adressen in einen Bucket kippen (Lockout-DoS).
            if let Some(v4) = v6.to_ipv4_mapped() {
                return IpAddr::V4(v4);
            }
            let mut seg = v6.segments();
            for s in seg.iter_mut().skip(4) {
                *s = 0;
            }
            IpAddr::V6(std::net::Ipv6Addr::from(seg))
        }
    }
}

impl RateLimiter {
    /// Baut ein Limit mit *max* Requests je *window* und IP.
    pub(crate) fn new(max: u32, window: Duration) -> Self {
        RateLimiter {
            max,
            window,
            state: StdMutex::new(RateState::default()),
        }
    }

    /// Registriert einen Zugriff von *ip*. `Ok(())` lässt ihn durch;
    /// `Err(retry_after)` meldet das überschrittene Limit samt Sekunden bis zum
    /// nächsten freien Slot (mindestens 1).
    fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        let key = normalize_rate_key(ip);
        let mut guard = self.state.lock().expect("Rate-Limit-Mutex nicht vergiftet");
        let st = &mut *guard;
        // Speicher begrenzen, sobald ein neuer Bucket nötig wäre und die Map voll ist.
        if !st.map.contains_key(&key) && st.map.len() >= MAX_TRACKED_KEYS {
            // Veraltete Einträge wegfegen, aber höchstens einmal pro Fenster, damit
            // der O(n)-Scan nicht bei jedem Request unter dem Lock läuft.
            let due = st
                .last_sweep
                .is_none_or(|t| now.duration_since(t) >= self.window);
            if due {
                st.map.retain(|_, dq| {
                    dq.back()
                        .is_some_and(|&t| now.duration_since(t) < self.window)
                });
                st.last_sweep = Some(now);
            }
            // Hartes Limit: notfalls den am längsten inaktiven Bucket verdrängen.
            if st.map.len() >= MAX_TRACKED_KEYS {
                // Nur einen Bucket verdrängen, dessen letzter Treffer schon außerhalb
                // des Fensters liegt — einen lebenden Bucket zu löschen, würde per
                // Key-Churn das Limit aushebeln. Gibt es keinen, wird der neue Request
                // abgelehnt (fail closed).
                let victim = st
                    .map
                    .iter()
                    .min_by_key(|(_, dq)| dq.back().copied())
                    .filter(|(_, dq)| {
                        dq.back()
                            .is_none_or(|&t| now.duration_since(t) >= self.window)
                    })
                    .map(|(k, _)| *k);
                match victim {
                    Some(v) => {
                        st.map.remove(&v);
                    }
                    None => return Err(self.window.as_secs().max(1)),
                }
            }
        }
        let dq = st.map.entry(key).or_default();
        // Zeitstempel außerhalb des Fensters verwerfen.
        while let Some(&front) = dq.front() {
            if now.duration_since(front) >= self.window {
                dq.pop_front();
            } else {
                break;
            }
        }
        if dq.len() as u32 >= self.max {
            let oldest = *dq.front().expect("bei vollem Fenster nicht leer");
            let retry = self
                .window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            return Err(retry);
        }
        dq.push_back(now);
        Ok(())
    }
}

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
    rate: Arc<RateLimiter>,
}

impl AppState {
    fn new() -> Self {
        let settings = core::config::Settings::load();
        AppState {
            client: core::http::browser_client().expect("http client"),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: auth::AuthConfig::from_settings(&settings),
            rate: Arc::new(RateLimiter::new(rate_limit_from_env(), RATE_LIMIT_WINDOW)),
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
        // Innen: API-Key-Prüfung. Außen: Rate-Limit (greift vor der Auth, damit
        // auch unauthentifizierte Fluten gedrosselt werden). `/health` bleibt
        // von beidem ausgenommen.
        .layer(axum::middleware::from_fn_with_state(
            state.auth.clone(),
            auth::require_api_key,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.rate.clone(),
            rate_limit,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi_json))
        .merge(api)
        .with_state(state)
}

/// Middleware für `/api/*`: drosselt Requests pro Client-IP. Bei Überschreitung
/// `429` mit Header `Retry-After`.
async fn rate_limit(State(rate): State<Arc<RateLimiter>>, req: Request, next: Next) -> Response {
    let ip = client_ip(&req);
    match rate.check(ip) {
        Ok(()) => next.run(req).await,
        Err(retry_after) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry_after.to_string())],
            Json(json!({ "error": "Rate-Limit erreicht, bitte später erneut versuchen" })),
        )
            .into_response(),
    }
}

/// Client-IP aus der Verbindung (`ConnectInfo`). Fehlt sie (z. B. im Test über
/// `oneshot`), wird localhost angenommen.
fn client_ip(req: &Request) -> IpAddr {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Liefert die OpenAPI-3-Spezifikation als JSON. Offen (kein API-Key), wie
/// `/health`, damit Clients/Tools das Schema ohne Anmeldung lesen können.
#[utoipa::path(
    get,
    path = "/openapi.json",
    tag = "meta",
    responses(
        (status = 200, description = "OpenAPI-3-Spezifikation dieser API", content_type = "application/json")
    )
)]
async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
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
    // `ConnectInfo` liefert dem Rate-Limit die echte Client-IP.
    axum::serve(
        listener,
        router_with(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Liveness-Check. Offen (kein API-Key), antwortet immer mit `ok`.
#[utoipa::path(
    get,
    path = "/health",
    tag = "meta",
    responses(
        (status = 200, description = "Dienst ist erreichbar", body = String, content_type = "text/plain")
    )
)]
async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct SearchParams {
    /// Suchbegriff; filtert den Katalog (Teilstring, case-insensitiv).
    keyword: String,
}

/// Durchsucht den Portal-Katalog nach *keyword* (max. 50 Treffer).
#[utoipa::path(
    get,
    path = "/api/search",
    tag = "vindex",
    params(SearchParams),
    responses(
        (status = 200, description = "Trefferliste", body = SearchResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 429, description = "Rate-Limit überschritten", body = ErrorResponse),
        (status = 502, description = "Katalog konnte nicht abgerufen werden", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
async fn search(
    State(state): State<AppState>,
    Query(p): Query<SearchParams>,
) -> (StatusCode, Json<serde_json::Value>) {
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
            (StatusCode::OK, Json(json!({ "results": results })))
        }
        Err(e) => {
            // Rohen Upstream-Fehler nicht an den Client geben; nur serverseitig loggen.
            tracing::warn!("Katalog-Abruf fehlgeschlagen: {e}");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "Katalog konnte nicht abgerufen werden" })),
            )
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct ProvidersParams {
    /// Quell-URL einer Episode/Folge auf einem unterstützten Portal.
    url: String,
}

/// Listet die verfügbaren Hoster-Links je Sprache zu einer Quell-URL auf.
#[utoipa::path(
    get,
    path = "/api/providers",
    tag = "vindex",
    params(ProvidersParams),
    responses(
        (status = 200, description = "Hoster-Links je Sprache", body = ProvidersResponse),
        (status = 400, description = "Fehlende/ungültige oder nicht erlaubte Quell-URL", body = ErrorResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 429, description = "Rate-Limit überschritten", body = ErrorResponse),
        (status = 502, description = "Provider-Abruf fehlgeschlagen", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
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

#[derive(Deserialize, ToSchema)]
struct DownloadReq {
    /// Quell-URL einer Episode/Folge/Films auf einem unterstützten Portal.
    #[schema(example = "https://aniworld.to/anime/stream/x/staffel-1/episode-1")]
    url: String,
    /// Gewünschte Sprache (z. B. `German Dub`). Fällt auf die Einstellung zurück.
    #[serde(default)]
    language: Option<String>,
    /// Bevorzugter Hoster (z. B. `VOE`). Fällt auf die Fallback-Kette zurück.
    #[serde(default)]
    provider: Option<String>,
    /// Maximale Qualität (z. B. `1080p`). Fällt auf die Einstellung zurück.
    #[serde(default)]
    quality: Option<String>,
}

/// Reiht einen Download in die Queue ein und startet ihn im Hintergrund.
#[utoipa::path(
    post,
    path = "/api/download",
    tag = "vindex",
    request_body = DownloadReq,
    responses(
        (status = 202, description = "Auftrag angenommen und eingereiht", body = DownloadAccepted),
        (status = 400, description = "Fehlende/ungültige oder nicht erlaubte Quell-URL", body = ErrorResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 429, description = "Queue voll oder Rate-Limit überschritten", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
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
            native_hls: settings.native_hls,
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

/// Gibt die aktuelle Download-Queue mit Status je Eintrag zurück.
#[utoipa::path(
    get,
    path = "/api/queue",
    tag = "vindex",
    responses(
        (status = 200, description = "Aktuelle Queue", body = QueueResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 429, description = "Rate-Limit überschritten", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
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

// ---------------------------------------------------------------------------
// OpenAPI-Spezifikation
//
// Die folgenden Typen beschreiben ausschließlich die Antwortkörper für die
// generierte Spezifikation. Die Handler liefern weiterhin dynamisches JSON
// (`serde_json::Value`); diese Structs ändern das Laufzeitverhalten nicht.
// ---------------------------------------------------------------------------

/// Einheitlicher Fehlerkörper der API.
#[derive(ToSchema)]
#[allow(dead_code)]
struct ErrorResponse {
    /// Menschenlesbare Fehlermeldung.
    error: String,
}

/// Ein einzelner Suchtreffer.
#[derive(ToSchema)]
#[allow(dead_code)]
struct SearchResult {
    /// Titel des Treffers.
    title: String,
    /// Direkte Quell-URL zum Stream.
    url: String,
}

/// Antwort von `GET /api/search`.
#[derive(ToSchema)]
#[allow(dead_code)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

/// Antwort von `GET /api/providers`: Hoster-Links, nach Sprache und dann nach
/// Hoster-Name gruppiert (`{ "German Dub": { "VOE": "https://…" } }`).
#[derive(ToSchema)]
#[allow(dead_code)]
struct ProvidersResponse {
    #[schema(additional_properties = true)]
    providers: serde_json::Value,
}

/// Antwort von `POST /api/download` (202).
#[derive(ToSchema)]
#[allow(dead_code)]
struct DownloadAccepted {
    /// Kennung des eingereihten Auftrags.
    queue_id: u64,
    /// Status direkt nach dem Einreihen (`queued`).
    status: String,
}

/// Ein Eintrag der Download-Queue.
#[derive(ToSchema)]
#[allow(dead_code)]
struct QueueItem {
    /// Kennung des Auftrags.
    id: u64,
    /// Quell-URL des Downloads.
    url: String,
    /// Aktueller Status: `queued`, `running`, `completed` oder `failed`.
    status: String,
    /// Nach außen sichtbares Detail (Dateiname bzw. knappe Fehlermeldung).
    detail: String,
}

/// Antwort von `GET /api/queue`.
#[derive(ToSchema)]
#[allow(dead_code)]
struct QueueResponse {
    items: Vec<QueueItem>,
}

/// Ergänzt das API-Key-Sicherheitsschema (`X-API-Key`-Header) in der Spez.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_key",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-API-Key"))),
        );
    }
}

/// Gebündelte OpenAPI-3-Spezifikation der HTTP-API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "vindex HTTP-API",
        description = "Suche, Provider-Metadaten und Download-Queue von vindex.",
        license(name = "MIT")
    ),
    paths(health, openapi_json, search, providers, download, queue),
    components(schemas(
        DownloadReq,
        DownloadAccepted,
        SearchResponse,
        SearchResult,
        ProvidersResponse,
        QueueResponse,
        QueueItem,
        ErrorResponse
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "vindex", description = "Geschützte /api/*-Routen (API-Key nötig, sofern konfiguriert)."),
        (name = "meta", description = "Offene Routen ohne API-Key.")
    )
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state(keys: &[&str]) -> AppState {
        state_mit_limit(keys, DEFAULT_RATE_LIMIT)
    }

    /// Wie [`state`], aber mit explizitem Rate-Limit für die Drossel-Tests.
    fn state_mit_limit(keys: &[&str], max: u32) -> AppState {
        AppState {
            client: reqwest::Client::new(),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: auth::AuthConfig::with_keys(keys),
            rate: Arc::new(RateLimiter::new(max, RATE_LIMIT_WINDOW)),
        }
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, String) {
        let resp = app.oneshot(req).await.expect("Router antwortet immer");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap_or_default();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn get_req(uri: &str, key: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri(uri);
        if let Some(k) = key {
            b = b.header("x-api-key", k);
        }
        b.body(Body::empty()).expect("Request baubar")
    }

    fn post_json(uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("Request baubar")
    }

    #[tokio::test]
    async fn health_ist_offen_und_liefert_ok() {
        let (status, body) = send(router_with(state(&["geheim"])), get_req("/health", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn openapi_json_ist_offen_ohne_key() {
        // /openapi.json liegt wie /health vor der API-Key-Middleware: trotz
        // konfiguriertem Key und ohne Key im Request darf es NICHT 401 sein.
        let (status, _) = send(
            router_with(state(&["geheim"])),
            get_req("/openapi.json", None),
        )
        .await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn openapi_json_liefert_gueltige_spez() {
        let (status, body) = send(
            router_with(state(&["geheim"])),
            get_req("/openapi.json", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let spec: serde_json::Value = serde_json::from_str(&body).expect("Body ist JSON");

        // OpenAPI-Version vorhanden und 3.x.
        let version = spec["openapi"].as_str().expect("openapi-Feld");
        assert!(version.starts_with("3."), "unerwartete Version: {version}");

        // Die vier /api/*-Pfade sind beschrieben.
        let paths = &spec["paths"];
        for p in [
            "/api/search",
            "/api/providers",
            "/api/download",
            "/api/queue",
        ] {
            assert!(paths.get(p).is_some(), "Pfad fehlt in der Spez: {p}");
        }
    }

    #[tokio::test]
    async fn queue_ohne_key_ist_401() {
        let (status, _) = send(router_with(state(&["geheim"])), get_req("/api/queue", None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn queue_mit_falschem_key_ist_401() {
        let (status, _) = send(
            router_with(state(&["geheim"])),
            get_req("/api/queue", Some("falsch")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn queue_mit_gueltigem_key_ist_200_und_leer() {
        let (status, body) = send(
            router_with(state(&["geheim"])),
            get_req("/api/queue", Some("geheim")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"items":[]}"#);
    }

    #[tokio::test]
    async fn queue_mit_bearer_key_ist_200() {
        let req = Request::builder()
            .uri("/api/queue")
            .header("authorization", "Bearer geheim")
            .body(Body::empty())
            .expect("Request baubar");
        let (status, _) = send(router_with(state(&["geheim"])), req).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn queue_ohne_konfigurierte_keys_ist_offen() {
        let (status, _) = send(router_with(state(&[])), get_req("/api/queue", None)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn download_mit_fremdem_host_ist_400() {
        let (status, _) = send(
            router_with(state(&[])),
            post_json("/api/download", r#"{"url":"http://evil.example/x"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn download_mit_http_auf_erlaubtem_host_ist_400() {
        let (status, _) = send(
            router_with(state(&[])),
            post_json(
                "/api/download",
                r#"{"url":"http://aniworld.to/anime/stream/x/staffel-1/episode-1"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn download_mit_gueltiger_url_wird_eingereiht_202() {
        // Der Handler validiert die URL (netzfrei) und reiht den Job ein; der
        // echte Abruf passiert erst im Hintergrund-Worker (darf scheitern). Hier
        // zählt nur der sofort zurückgegebene Statuscode 202.
        let (status, body) = send(
            router_with(state(&[])),
            post_json(
                "/api/download",
                r#"{"url":"https://aniworld.to/anime/stream/x/staffel-1/episode-1"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.contains("\"queue_id\""), "{body}");
    }

    #[tokio::test]
    async fn download_mit_kaputtem_json_ist_400() {
        let (status, _) = send(
            router_with(state(&[])),
            post_json("/api/download", "{kaputt"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn download_ohne_url_feld_ist_client_fehler() {
        let (status, _) = send(router_with(state(&[])), post_json("/api/download", "{}")).await;
        assert!(status.is_client_error(), "{status}");
    }

    #[tokio::test]
    async fn download_ohne_key_ist_401_vor_validierung() {
        let (status, _) = send(
            router_with(state(&["geheim"])),
            post_json("/api/download", r#"{"url":"http://evil.example"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn providers_mit_fremdem_host_ist_400() {
        let (status, _) = send(
            router_with(state(&[])),
            get_req("/api/providers?url=http://evil.example", None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn providers_ohne_url_parameter_ist_400() {
        let (status, _) = send(router_with(state(&[])), get_req("/api/providers", None)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unbekannte_route_ist_404() {
        let (status, _) = send(router_with(state(&[])), get_req("/nirgendwo", None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rate_limit_blockt_nach_n_requests_mit_retry_after() {
        // Limit 3: die ersten drei /api/queue-Requests kommen durch (offen, 200),
        // der vierte wird mit 429 + Retry-After gedrosselt.
        let app = router_with(state_mit_limit(&[], 3));
        for _ in 0..3 {
            let resp = app
                .clone()
                .oneshot(get_req("/api/queue", None))
                .await
                .expect("Router antwortet");
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let resp = app
            .clone()
            .oneshot(get_req("/api/queue", None))
            .await
            .expect("Router antwortet");
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry = resp
            .headers()
            .get(header::RETRY_AFTER)
            .expect("Retry-After gesetzt")
            .to_str()
            .expect("Retry-After ist ASCII");
        assert!(
            retry.parse::<u64>().is_ok_and(|s| s >= 1),
            "Retry-After-Sekunden: {retry}"
        );
    }

    #[tokio::test]
    async fn rate_limit_gilt_nicht_fuer_health() {
        // /health liegt außerhalb der Middleware und bleibt auch über dem Limit offen.
        let app = router_with(state_mit_limit(&["geheim"], 2));
        for _ in 0..5 {
            let resp = app
                .clone()
                .oneshot(get_req("/health", None))
                .await
                .expect("Router antwortet");
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[test]
    fn rate_limiter_laesst_bis_zum_limit_durch() {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let rl = RateLimiter::new(2, RATE_LIMIT_WINDOW);
        assert!(rl.check(ip).is_ok());
        assert!(rl.check(ip).is_ok());
        let retry = rl.check(ip).expect_err("drittes über Limit");
        assert!(retry >= 1);
    }

    #[test]
    fn rate_limiter_trennt_nach_ip() {
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let rl = RateLimiter::new(1, RATE_LIMIT_WINDOW);
        assert!(rl.check(a).is_ok());
        assert!(rl.check(a).is_err(), "zweite von IP a blockt");
        assert!(rl.check(b).is_ok(), "IP b hat eigenes Kontingent");
    }

    #[test]
    fn rate_limiter_gibt_slot_nach_fenster_frei() {
        // Winziges Fenster: nach kurzer Pause ist der Slot wieder frei.
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let rl = RateLimiter::new(1, Duration::from_millis(20));
        assert!(rl.check(ip).is_ok());
        assert!(rl.check(ip).is_err());
        std::thread::sleep(Duration::from_millis(30));
        assert!(rl.check(ip).is_ok(), "nach Fensterablauf wieder frei");
    }

    #[test]
    fn public_detail_versteckt_pfade_und_fehlertexte() {
        let job = |status, detail: &str| Job {
            id: 1,
            url: String::new(),
            status,
            detail: detail.to_string(),
        };
        assert_eq!(
            public_detail(&job(JobStatus::Completed, "/home/x/out/Film.mkv")),
            "Film.mkv"
        );
        assert_eq!(
            public_detail(&job(JobStatus::Failed, "interner Fehler /etc/x")),
            "Download fehlgeschlagen"
        );
        assert_eq!(public_detail(&job(JobStatus::Running, "x")), "");
        assert_eq!(public_detail(&job(JobStatus::Queued, "x")), "");
    }

    #[test]
    fn dateiname_aus_url_episode_und_film() {
        assert_eq!(
            file_name_from_url("https://aniworld.to/anime/stream/one-piece/staffel-2/episode-15"),
            "one piece S02E15.mkv"
        );
        assert_eq!(
            file_name_from_url("https://filmo.to/movies/the-matrix"),
            "the matrix.mkv"
        );
        assert_eq!(file_name_from_url("https://filmo.to/"), "movie.mkv");
    }

    #[test]
    fn capture_num_parst_und_lehnt_ab() {
        assert_eq!(
            capture_num("https://x/staffel-3/episode-1", "/staffel-"),
            Some(3)
        );
        assert_eq!(capture_num("https://x/staffel-a", "/staffel-"), None);
        assert_eq!(capture_num("https://x/", "/staffel-"), None);
    }
}
