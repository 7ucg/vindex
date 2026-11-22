//! HTTP-API / Daemon für vindex (axum).
//!
//! Funktionsfähige Teilmenge der Referenz-API (siehe README): Suche,
//! Provider-Metadaten, Download-Queue. Die `/api/*`-Routen liegen hinter einer
//! API-Key-Prüfung (siehe [`auth`]) und einem Request-Rate-Limit pro IP
//! (Sliding-Window, siehe [`RateLimiter`]). Eine OpenAPI-3-Spezifikation (via
//! [`utoipa`]) wird offen unter `/openapi.json` ausgeliefert. SSO, Library und
//! Auto-Sync folgen gemäß `ARCHITECTURE.md` §9 (Schritt 11).

mod auth;

pub use auth::generate_key;

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path, Query, Request, State};
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
///
/// Best-Effort-Schutz im Prozessspeicher (Defense-in-Depth), kein harter
/// DoS-Schutz: bei voller Tabelle fallen neue Keys in einen gemeinsamen
/// Overflow-Bucket. Für einen öffentlichen Betrieb gehört davor ein Edge-/
/// Reverse-Proxy-Rate-Limit (siehe TODO.md).
pub struct RateLimiter {
    max: u32,
    window: Duration,
    state: StdMutex<RateState>,
}

/// Interner Zustand des Limiters; `last_sweep` drosselt den O(n)-Aufräumlauf,
/// `overflow` ist der gemeinsame Bucket für neue Keys bei voller Tabelle.
#[derive(Default)]
struct RateState {
    map: HashMap<IpAddr, VecDeque<Instant>>,
    last_sweep: Option<Instant>,
    overflow: VecDeque<Instant>,
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
        // Welcher Bucket? Normalerweise der eigene Key-Bucket. Ist die Tabelle voll
        // und lässt sich kein veralteter Bucket freigeben, teilen sich neue Keys einen
        // gemeinsamen Overflow-Bucket. So bleibt der Speicher begrenzt, ohne lebende
        // Buckets zu löschen (Key-Churn-Bypass) oder neue Keys ganz abzuweisen (Lockout).
        let use_overflow = if st.map.contains_key(&key) || st.map.len() < MAX_TRACKED_KEYS {
            false
        } else {
            // Höchstens einmal pro Fenster aufräumen (kein O(n)-Scan je Request).
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
            if st.map.len() < MAX_TRACKED_KEYS {
                false
            } else {
                // Einen bereits veralteten Bucket freigeben; gibt es keinen, Overflow.
                let stale = st
                    .map
                    .iter()
                    .find(|(_, dq)| {
                        dq.back()
                            .is_none_or(|&t| now.duration_since(t) >= self.window)
                    })
                    .map(|(k, _)| *k);
                match stale {
                    Some(v) => {
                        st.map.remove(&v);
                        false
                    }
                    None => true,
                }
            }
        };

        let dq = if use_overflow {
            &mut st.overflow
        } else {
            st.map.entry(key).or_default()
        };
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
    /// Hot-reloadbar: die `/api/keys`-Routen tauschen den Stand nach jedem
    /// Anlegen/Löschen aus, die Auth-Middleware liest stets den aktuellen.
    auth: Arc<RwLock<auth::AuthConfig>>,
    rate: Arc<RateLimiter>,
}

impl AppState {
    fn new() -> Self {
        let settings = core::config::Settings::load();
        AppState {
            client: core::http::browser_client().expect("http client"),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: Arc::new(RwLock::new(auth::AuthConfig::from_settings(&settings))),
            rate: Arc::new(RateLimiter::new(rate_limit_from_env(), RATE_LIMIT_WINDOW)),
        }
    }

    /// True, wenn keine Keys konfiguriert sind (offene API). Liest den aktuellen
    /// Auth-Stand unter dem Lesesperre-Guard.
    fn auth_is_open(&self) -> bool {
        self.auth
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_open()
    }

    /// Lädt den Key-Store aus der Datei neu und ersetzt den In-Memory-Stand.
    /// Wird nach jedem erfolgreichen Anlegen/Löschen über `/api/keys` aufgerufen,
    /// damit die Änderung sofort wirkt.
    fn reload_auth(&self) {
        let fresh = self
            .auth
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .reloaded();
        let mut guard = self.auth.write().unwrap_or_else(|e| e.into_inner());
        *guard = fresh;
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
        // Admin-geschützte Key-Verwaltung (Scope wird in der Auth-Middleware auf
        // Admin gesetzt). Löschen per Pfad (`/api/keys/{name}`) oder Query (`?name=`).
        .route(
            "/api/keys",
            get(list_keys).post(create_key).delete(delete_key_query),
        )
        .route("/api/keys/{name}", axum::routing::delete(delete_key_path))
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
async fn openapi_json() -> impl IntoResponse {
    // Spec einmal serialisieren und zwischenspeichern, damit jeder (offene,
    // ungedrosselte) Aufruf nur den fertigen String ausliefert statt sie neu
    // aufzubauen.
    static SPEC: std::sync::LazyLock<String> =
        std::sync::LazyLock::new(|| serde_json::to_string(&ApiDoc::openapi()).unwrap_or_default());
    ([(header::CONTENT_TYPE, "application/json")], SPEC.as_str())
}

/// Startet den Daemon auf *addr* (z. B. `127.0.0.1:8080`).
///
/// Ohne konfigurierte API-Keys wird nur ein localhost-Bind zugelassen; ein
/// öffentlicher Bind ohne Auth bricht mit Fehler ab.
pub async fn serve(addr: &str) -> anyhow::Result<()> {
    let state = AppState::new();
    let open = state.auth_is_open();

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

/// Reiht einen Job ein, wenn der Queue-Cap es zulässt. `false` (→ `429`), wenn die
/// Zahl aktiver Jobs (`Queued`/`Running`) den Cap erreicht. Fertige Jobs werden
/// beschnitten, sobald die Liste zu lang wird; aktive bleiben erhalten. Rein testbar.
fn try_admit_job(jobs: &mut Vec<Job>, id: u64, url: String) -> bool {
    let inflight = jobs
        .iter()
        .filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
        .count();
    if inflight >= MAX_INFLIGHT_JOBS {
        return false;
    }
    if jobs.len() >= MAX_JOBS_RETAINED {
        jobs.retain(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
    }
    jobs.push(Job {
        id,
        url,
        status: JobStatus::Queued,
        detail: String::new(),
    });
    true
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
        (status = 403, description = "API-Key ohne Write-Scope", body = ErrorResponse),
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
        if !try_admit_job(&mut jobs, id, req.url.clone()) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({ "error": "Queue voll, bitte später erneut versuchen" })),
            );
        }
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

// ---------------------------------------------------------------------------
// Key-Verwaltung (/api/keys) — Admin-Scope, wirkt auf den persistenten Store.
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
struct CreateKeyReq {
    /// Name/Label des neuen Keys (zur Wiedererkennung im Store).
    #[schema(example = "ci-runner")]
    name: String,
    /// Scope des Keys: `read`, `write` oder `admin`.
    scope: auth::Scope,
}

/// Legt einen neuen API-Key an. Der Klartext wird **einmalig** zurückgegeben und
/// nie gespeichert; im Store landet nur der sha256-Hash. Erfordert Admin-Scope.
#[utoipa::path(
    post,
    path = "/api/keys",
    tag = "keys",
    request_body = CreateKeyReq,
    responses(
        (status = 201, description = "Key angelegt; Klartext einmalig im Body", body = CreateKeyResponse),
        (status = 400, description = "Leerer/ungültiger Name oder Scope", body = ErrorResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 403, description = "API-Key ohne Admin-Scope", body = ErrorResponse),
        (status = 500, description = "Store konnte nicht geschrieben werden", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
async fn create_key(
    State(state): State<AppState>,
    Json(req): Json<CreateKeyReq>,
) -> (StatusCode, Json<serde_json::Value>) {
    let name = req.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Name darf nicht leer sein" })),
        );
    }
    match auth::add_key(name, req.scope) {
        Ok(plain) => {
            // In-Memory-Store neu laden, damit der frische Key sofort gilt.
            state.reload_auth();
            (
                StatusCode::CREATED,
                Json(json!({ "key": plain, "name": name, "scope": req.scope.as_str() })),
            )
        }
        Err(e) => key_mgmt_error_response(e),
    }
}

/// Listet die Keys des persistenten Stores als `[{ name, scope }]` — niemals
/// Hashes oder Klartext. Erfordert Admin-Scope.
#[utoipa::path(
    get,
    path = "/api/keys",
    tag = "keys",
    responses(
        (status = 200, description = "Liste der Store-Keys (ohne Hash/Klartext)", body = [KeyInfo]),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 403, description = "API-Key ohne Admin-Scope", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
async fn list_keys(State(state): State<AppState>) -> Json<serde_json::Value> {
    let keys = state
        .auth
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .stored_keys();
    let items: Vec<_> = keys
        .into_iter()
        .map(|(name, scope)| json!({ "name": name, "scope": scope.as_str() }))
        .collect();
    Json(serde_json::Value::Array(items))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct DeleteKeyParams {
    /// Name des zu löschenden Keys (Alternative zum Pfad `/api/keys/{name}`).
    name: String,
}

/// Löscht Keys per Query (`DELETE /api/keys?name=...`). Erfordert Admin-Scope.
#[utoipa::path(
    delete,
    path = "/api/keys",
    tag = "keys",
    params(DeleteKeyParams),
    responses(
        (status = 200, description = "Key(s) entfernt", body = DeleteKeyResponse),
        (status = 400, description = "Fehlender Name", body = ErrorResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 403, description = "API-Key ohne Admin-Scope", body = ErrorResponse),
        (status = 404, description = "Kein Store-Key mit diesem Namen", body = ErrorResponse),
        (status = 500, description = "Store konnte nicht geschrieben werden", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
async fn delete_key_query(
    State(state): State<AppState>,
    Query(p): Query<DeleteKeyParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    delete_key_inner(&state, &p.name)
}

/// Löscht Keys per Pfad (`DELETE /api/keys/{name}`). Erfordert Admin-Scope.
#[utoipa::path(
    delete,
    path = "/api/keys/{name}",
    tag = "keys",
    params(("name" = String, Path, description = "Name des zu löschenden Keys")),
    responses(
        (status = 200, description = "Key(s) entfernt", body = DeleteKeyResponse),
        (status = 401, description = "Fehlender/ungültiger API-Key", body = ErrorResponse),
        (status = 403, description = "API-Key ohne Admin-Scope", body = ErrorResponse),
        (status = 404, description = "Kein Store-Key mit diesem Namen", body = ErrorResponse),
        (status = 500, description = "Store konnte nicht geschrieben werden", body = ErrorResponse)
    ),
    security(("api_key" = []))
)]
async fn delete_key_path(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    delete_key_inner(&state, &name)
}

/// Gemeinsame Löschlogik für Pfad- und Query-Variante.
fn delete_key_inner(state: &AppState, name: &str) -> (StatusCode, Json<serde_json::Value>) {
    let name = name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Name darf nicht leer sein" })),
        );
    }
    match auth::remove_keys(name) {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "kein Key mit diesem Namen im Store \
                          (Env-Bootstrap-Keys sind nicht löschbar)"
            })),
        ),
        Ok(removed) => {
            // In-Memory-Store neu laden, damit der gelöschte Key sofort ungültig ist.
            state.reload_auth();
            (
                StatusCode::OK,
                Json(json!({ "removed": removed, "name": name })),
            )
        }
        Err(e) => key_mgmt_error_response(e),
    }
}

/// Bildet einen [`auth::KeyMgmtError`] auf eine HTTP-Antwort ab. Interne Pfade/
/// Details bleiben draußen; nur eine knappe Meldung geht an den Client.
fn key_mgmt_error_response(e: auth::KeyMgmtError) -> (StatusCode, Json<serde_json::Value>) {
    use auth::KeyMgmtError as E;
    // Client-Fehler mit knapper, ungefährlicher Meldung; interne Fehler (IO/Pfad/
    // kaputter Store) bleiben bei 500 ohne Details.
    match e {
        E::InvalidName => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        ),
        E::Duplicate => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "ein Key mit diesem Namen existiert bereits" })),
        ),
        E::Full => (
            StatusCode::INSUFFICIENT_STORAGE,
            Json(json!({ "error": "Key-Store ist voll" })),
        ),
        E::NoPath | E::Corrupt | E::Rng | E::Io(_) => {
            tracing::warn!("Key-Verwaltung fehlgeschlagen: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Key-Store konnte nicht aktualisiert werden" })),
            )
        }
    }
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

/// Antwort von `POST /api/keys` (201): der Klartext wird **einmalig** geliefert.
#[derive(ToSchema)]
#[allow(dead_code)]
struct CreateKeyResponse {
    /// Klartext-Key — nur hier, nie gespeichert.
    key: String,
    /// Name/Label des Keys.
    name: String,
    /// Scope: `read`, `write` oder `admin`.
    scope: String,
}

/// Ein Listeneintrag von `GET /api/keys` (ohne Hash/Klartext).
#[derive(ToSchema)]
#[allow(dead_code)]
struct KeyInfo {
    /// Name/Label des Keys.
    name: String,
    /// Scope: `read`, `write` oder `admin`.
    scope: String,
}

/// Antwort beim Löschen (`DELETE /api/keys`).
#[derive(ToSchema)]
#[allow(dead_code)]
struct DeleteKeyResponse {
    /// Zahl der entfernten Einträge.
    removed: usize,
    /// Betroffener Name.
    name: String,
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
    paths(
        health,
        openapi_json,
        search,
        providers,
        download,
        queue,
        create_key,
        list_keys,
        delete_key_query,
        delete_key_path
    ),
    components(schemas(
        DownloadReq,
        DownloadAccepted,
        SearchResponse,
        SearchResult,
        ProvidersResponse,
        QueueResponse,
        QueueItem,
        CreateKeyReq,
        CreateKeyResponse,
        KeyInfo,
        DeleteKeyResponse,
        ErrorResponse
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "vindex", description = "Geschützte /api/*-Routen (API-Key nötig, sofern konfiguriert)."),
        (name = "keys", description = "Admin-geschützte Key-Verwaltung (/api/keys)."),
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

    fn job(id: u64, status: JobStatus) -> Job {
        Job {
            id,
            url: "https://aniworld.to/anime/stream/x/staffel-1/episode-1".into(),
            status,
            detail: String::new(),
        }
    }

    #[test]
    fn queue_cap_lehnt_bei_vollen_aktiven_jobs_ab() {
        let mut jobs: Vec<Job> = (0..MAX_INFLIGHT_JOBS as u64)
            .map(|i| job(i, JobStatus::Queued))
            .collect();
        // Voll → der nächste Job wird abgelehnt (→ 429).
        assert!(!try_admit_job(
            &mut jobs,
            999,
            "https://aniworld.to/x".into()
        ));
        assert_eq!(jobs.len(), MAX_INFLIGHT_JOBS);
        // Wird einer fertig, ist wieder Platz.
        jobs[0].status = JobStatus::Completed;
        assert!(try_admit_job(
            &mut jobs,
            1000,
            "https://aniworld.to/y".into()
        ));
        assert!(jobs.iter().any(|j| j.id == 1000));
    }

    #[test]
    fn laufende_jobs_zaehlen_zum_cap() {
        let mut jobs: Vec<Job> = (0..MAX_INFLIGHT_JOBS as u64)
            .map(|i| job(i, JobStatus::Running))
            .collect();
        assert!(!try_admit_job(
            &mut jobs,
            999,
            "https://aniworld.to/x".into()
        ));
    }

    #[test]
    fn fertige_jobs_werden_beschnitten() {
        // Volle Liste aus fertigen Jobs (inflight = 0): Admission beschneidet sie.
        let mut jobs: Vec<Job> = (0..MAX_JOBS_RETAINED as u64)
            .map(|i| job(i, JobStatus::Completed))
            .collect();
        assert!(try_admit_job(
            &mut jobs,
            10_000,
            "https://aniworld.to/z".into()
        ));
        // Nur der neue (aktive) Job bleibt übrig.
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, 10_000);
    }

    fn state(keys: &[&str]) -> AppState {
        state_mit_limit(keys, DEFAULT_RATE_LIMIT)
    }

    /// Wie [`state`], aber mit explizitem Rate-Limit für die Drossel-Tests.
    fn state_mit_limit(keys: &[&str], max: u32) -> AppState {
        AppState {
            client: reqwest::Client::new(),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: Arc::new(RwLock::new(auth::AuthConfig::with_keys(keys))),
            rate: Arc::new(RateLimiter::new(max, RATE_LIMIT_WINDOW)),
        }
    }

    /// Wie [`state`], aber mit Keys samt explizitem Scope (Store-artig).
    fn state_scoped(keys: &[(&str, auth::Scope)]) -> AppState {
        AppState {
            client: reqwest::Client::new(),
            jobs: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            auth: Arc::new(RwLock::new(auth::AuthConfig::with_scoped(keys))),
            rate: Arc::new(RateLimiter::new(DEFAULT_RATE_LIMIT, RATE_LIMIT_WINDOW)),
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
    async fn read_key_darf_queue_aber_nicht_download_403() {
        // Read-Key: GET /api/queue → 200, aber POST /api/download → 403.
        let app = router_with(state_scoped(&[("leser", auth::Scope::Read)]));
        let (status, _) = send(app.clone(), get_req("/api/queue", Some("leser"))).await;
        assert_eq!(status, StatusCode::OK);

        let mut req = post_json(
            "/api/download",
            r#"{"url":"https://aniworld.to/anime/stream/x/staffel-1/episode-1"}"#,
        );
        req.headers_mut()
            .insert("x-api-key", "leser".parse().unwrap());
        let (status, _) = send(app, req).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn write_key_darf_download_202() {
        let app = router_with(state_scoped(&[("schreiber", auth::Scope::Write)]));
        let mut req = post_json(
            "/api/download",
            r#"{"url":"https://aniworld.to/anime/stream/x/staffel-1/episode-1"}"#,
        );
        req.headers_mut()
            .insert("x-api-key", "schreiber".parse().unwrap());
        let (status, body) = send(app, req).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.contains("\"queue_id\""), "{body}");
    }

    #[tokio::test]
    async fn admin_key_deckt_read_und_write() {
        let app = router_with(state_scoped(&[("chef", auth::Scope::Admin)]));
        let (status, _) = send(app.clone(), get_req("/api/queue", Some("chef"))).await;
        assert_eq!(status, StatusCode::OK);

        let mut req = post_json(
            "/api/download",
            r#"{"url":"https://aniworld.to/anime/stream/x/staffel-1/episode-1"}"#,
        );
        req.headers_mut()
            .insert("x-api-key", "chef".parse().unwrap());
        let (status, _) = send(app, req).await;
        assert_eq!(status, StatusCode::ACCEPTED);
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

    // -----------------------------------------------------------------------
    // Key-Verwaltung (/api/keys)
    // -----------------------------------------------------------------------

    fn delete_req(uri: &str, key: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("DELETE").uri(uri);
        if let Some(k) = key {
            b = b.header("x-api-key", k);
        }
        b.body(Body::empty()).expect("Request baubar")
    }

    fn post_keys(body: &str, key: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/keys")
            .header("content-type", "application/json")
            .header("x-api-key", key)
            .body(Body::from(body.to_string()))
            .expect("Request baubar")
    }

    #[tokio::test]
    async fn keys_ohne_key_ist_401() {
        let (status, _) = send(
            router_with(state(&["admintoken"])),
            get_req("/api/keys", None),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn keys_mit_nicht_admin_key_ist_403() {
        // Ein Write-Key (kein Admin) darf auf keiner /api/keys-Route etwas tun → 403.
        let app = router_with(state_scoped(&[("schreiber", auth::Scope::Write)]));

        let (status, _) = send(app.clone(), get_req("/api/keys", Some("schreiber"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "GET");

        let (status, _) = send(
            app.clone(),
            post_keys(r#"{"name":"x","scope":"read"}"#, "schreiber"),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "POST");

        let (status, _) = send(app, delete_req("/api/keys/x", Some("schreiber"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "DELETE");
    }

    #[tokio::test]
    async fn keys_admin_crud_flow_wirkt_sofort() {
        // Eigenes Temp-Verzeichnis als Store (kein echtes Home anfassen). Der
        // Admin-Key kommt als Env-Bootstrap, damit er einen Store-Reload übersteht.
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "vindex-keymgmt-test-{}-{}",
            std::process::id(),
            AtomicU64::new(0).fetch_add(1, Ordering::Relaxed)
        ));
        let store = dir.join("api_keys.json");
        std::env::set_var("VINDEX_API_KEYS_FILE", &store);

        let app = router_with(state(&["admintoken"]));

        // POST legt an → 201 mit Klartext im Body, ohne Hash.
        let (status, body) = send(
            app.clone(),
            post_keys(r#"{"name":"ci-runner","scope":"write"}"#, "admintoken"),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        let plain = created["key"].as_str().expect("Klartext-Key im Body");
        assert_eq!(plain.len(), 64, "Klartext ist 32-Byte-Hex");
        assert_eq!(created["name"], "ci-runner");
        assert_eq!(created["scope"], "write");

        // GET listet den Namen, aber niemals Hash oder Klartext.
        let (status, body) = send(app.clone(), get_req("/api/keys", Some("admintoken"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ci-runner"), "Name fehlt: {body}");
        assert!(!body.contains("hash"), "Hash-Feld geleakt: {body}");
        assert!(!body.contains(plain), "Klartext geleakt: {body}");

        // Der frisch angelegte Key wirkt sofort (Hot-Reload): GET /api/queue klappt.
        let (status, _) = send(app.clone(), get_req("/api/queue", Some(plain))).await;
        assert_eq!(status, StatusCode::OK, "neuer Key sofort gültig");

        // DELETE entfernt den Eintrag.
        let (status, _) = send(
            app.clone(),
            delete_req("/api/keys/ci-runner", Some("admintoken")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Danach ist er aus der Liste verschwunden …
        let (status, body) = send(app.clone(), get_req("/api/keys", Some("admintoken"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.contains("ci-runner"), "noch gelistet: {body}");

        // … und sofort ungültig (Hot-Reload wirkt auch beim Löschen) → 401.
        let (status, _) = send(app.clone(), get_req("/api/queue", Some(plain))).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "gelöschter Key abgewiesen"
        );

        // Erneutes Löschen ohne Treffer → 404.
        let (status, _) = send(app, delete_req("/api/keys/ci-runner", Some("admintoken"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        std::env::remove_var("VINDEX_API_KEYS_FILE");
        std::fs::remove_dir_all(&dir).ok();
    }
}
