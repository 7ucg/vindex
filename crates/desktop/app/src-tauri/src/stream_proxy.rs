//! Lokaler Stream-Proxy für den In-Window-Player.
//!
//! `<video>`/hls.js können keinen `Referer` setzen, viele Hoster-CDNs verlangen ihn
//! aber. Dieser Proxy läuft auf `127.0.0.1:<ephemer>`, ist Token-gesichert und
//! injiziert den Referer beim Upstream-Abruf. HLS-Playlists werden umgeschrieben,
//! sodass auch Segmente und Varianten wieder durch den Proxy (mit Referer) laufen.
//! Range-Header werden durchgereicht, damit MP4-Streams spulbar sind.
//!
//! SSRF-Härtung: Redirects werden manuell verfolgt und **jeder** Hop neu geprüft
//! (Schema + DNS-Auflösung + alle aufgelösten IPs gegen private/lokale Bereiche);
//! der Bind ist localhost-only, jede Anfrage braucht das (CSPRNG-)Token, und der
//! `Host`-Header muss auf localhost zeigen.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{
        header::{
            ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HOST, LOCATION, RANGE,
            REFERER,
        },
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

const DESKTOP_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/124.0 Safari/537.36";

const MAX_REDIRECTS: u8 = 6;

/// Freigegebene lokale Dateien (vom Backend vorab validiert) — nur per opaker ID
/// erreichbar, nie per Pfad in der URL (kein Path-Traversal übers Frontend).
type LocalStore = Arc<Mutex<HashMap<u64, PathBuf>>>;

/// Griff auf den laufenden Proxy: Basis-URL, Token und die Freigabe lokaler Dateien.
pub struct Proxy {
    base: String,
    token: String,
    locals: LocalStore,
    next_local: AtomicU64,
}

struct ProxyState {
    token: String,
    client: reqwest::Client,
    base: String,
    locals: LocalStore,
}

impl Proxy {
    /// Baut die Player-URL für einen aufgelösten Stream (Master-Playlist oder Datei).
    pub fn stream_url(&self, upstream: &str, referer: Option<&str>) -> String {
        proxy_url(&self.base, &self.token, upstream, referer)
    }

    /// Prefix zum Bauen von Proxy-URLs im Frontend: `<base>/s?t=<token>&u=`.
    /// Das Frontend hängt die base64url-kodierte Upstream-URL an (für Bild-Lazy-Load).
    pub fn image_prefix(&self) -> String {
        format!("{}/s?t={}&u=", self.base, self.token)
    }

    /// Gibt eine (bereits validierte) lokale Mediendatei frei und liefert die
    /// Proxy-URL, über die der In-Window-Player sie per HTTP+Range abspielt.
    pub fn local_url(&self, path: PathBuf) -> String {
        let id = self.next_local.fetch_add(1, Ordering::Relaxed) + 1;
        self.locals.lock().unwrap().insert(id, path);
        format!("{}/local?t={}&id={}", self.base, self.token, id)
    }
}

/// Startet den Proxy auf einem freien lokalen Port. Lebt für die Programmdauer.
/// Fail-closed: ohne CSPRNG-Token startet der Proxy nicht.
pub async fn start() -> std::io::Result<Arc<Proxy>> {
    let token = random_token()?;
    // Redirects werden bewusst manuell verfolgt (Re-Validierung pro Hop). Der
    // Guard-Resolver verwirft private/lokale IPs schon beim Auflösen — so nutzen
    // Prüfung und Verbindung dieselbe Auflösung (kein DNS-Rebinding-Fenster).
    let client = reqwest::Client::builder()
        .user_agent(DESKTOP_UA)
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(GuardResolver))
        .build()
        .map_err(std::io::Error::other)?;

    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let locals: LocalStore = Arc::new(Mutex::new(HashMap::new()));
    let state = Arc::new(ProxyState {
        token: token.clone(),
        client,
        base: base.clone(),
        locals: locals.clone(),
    });
    // Der WebView holt die Streams cross-origin (tauri://localhost → 127.0.0.1).
    // Ohne CORS blockt der Browser die Antwort (hls.js: manifestLoadError). Der
    // Zugriff ist ohnehin durch das Token + localhost-Bind geschützt.
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .expose_headers(tower_http::cors::Any);
    let router = Router::new()
        .route("/s", get(serve))
        .route("/local", get(serve_local))
        .with_state(state)
        .layer(cors);

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok(Arc::new(Proxy {
        base,
        token,
        locals,
        next_local: AtomicU64::new(0),
    }))
}

#[derive(Deserialize)]
struct LocalQ {
    t: String,
    id: u64,
}

/// Liefert eine freigegebene lokale Mediendatei per HTTP mit Range-Unterstützung
/// (fürs Spulen). Nur per Token + vorab registrierte ID — kein Pfad aus der URL.
async fn serve_local(
    State(st): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Query(q): Query<LocalQ>,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    if !host_is_local(&headers) || !ct_eq(q.t.as_bytes(), st.token.as_bytes()) {
        return (StatusCode::FORBIDDEN, "bad request").into_response();
    }
    let Some(path) = st.locals.lock().unwrap().get(&q.id).cloned() else {
        return (StatusCode::NOT_FOUND, "unbekannte id").into_response();
    };

    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "nicht lesbar").into_response(),
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "stat").into_response(),
    };
    let mime = mime_for(&path);

    // Range parsen (bytes=start-[end]); sonst die ganze Datei.
    let range = headers
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, len));

    let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
    if len == 0 || start > end || end >= len {
        return (StatusCode::RANGE_NOT_SATISFIABLE, "range").into_response();
    }
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "seek").into_response();
    }
    let take = file.take(end - start + 1);
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(take));

    let partial = range.is_some();
    let mut builder = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(CONTENT_TYPE, mime)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_LENGTH, (end - start + 1).to_string());
    if partial {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    builder
        .body(body)
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "body").into_response())
}

/// `bytes=start-end` (end optional) → (start, end) inkl.; `None` bei Unfug.
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    let end: u64 = if b.trim().is_empty() {
        len.saturating_sub(1)
    } else {
        b.trim().parse().ok()?
    };
    Some((start, end.min(len.saturating_sub(1))))
}

fn mime_for(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mkv" => "video/x-matroska",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

#[derive(Deserialize)]
struct Q {
    t: String,
    u: String,
    r: Option<String>,
}

async fn serve(
    State(st): State<Arc<ProxyState>>,
    headers: HeaderMap,
    Query(q): Query<Q>,
) -> Response {
    // Nur localhost: der Host-Header muss auf den lokalen Proxy zeigen (gegen
    // DNS-Rebinding einer fremden Seite auf 127.0.0.1:<port>).
    if !host_is_local(&headers) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    if !ct_eq(q.t.as_bytes(), st.token.as_bytes()) {
        return (StatusCode::FORBIDDEN, "bad token").into_response();
    }
    let Some(upstream) = b64d(&q.u) else {
        return (StatusCode::BAD_REQUEST, "bad upstream").into_response();
    };
    let referer = q.r.as_deref().and_then(b64d);

    let Ok(start_url) = reqwest::Url::parse(&upstream) else {
        return (StatusCode::BAD_REQUEST, "bad url").into_response();
    };

    let range = headers.get(RANGE).cloned();
    match fetch_validated(&st.client, start_url, referer.as_deref(), range.as_ref()).await {
        Ok((final_url, resp)) => deliver(resp, &final_url, referer.as_deref(), &st).await,
        Err((code, msg)) => (code, msg).into_response(),
    }
}

/// Holt die Upstream-Antwort und verfolgt Redirects manuell, wobei jeder Hop neu
/// gegen private/lokale Ziele geprüft wird. Gibt die finale URL (für die
/// HLS-Basis) und die nicht-weiterleitende Antwort zurück.
async fn fetch_validated(
    client: &reqwest::Client,
    start_url: reqwest::Url,
    referer: Option<&str>,
    range: Option<&HeaderValue>,
) -> Result<(reqwest::Url, reqwest::Response), (StatusCode, &'static str)> {
    let mut current = start_url;
    for _ in 0..=MAX_REDIRECTS {
        if let Err(e) = ensure_public(&current) {
            tracing::warn!(
                host = current.host_str().unwrap_or("?"),
                reason = e.1,
                "Proxy weist Ziel ab"
            );
            return Err(e);
        }

        let mut req = client.get(current.clone());
        if let Some(r) = referer {
            if let Ok(hv) = HeaderValue::from_str(r) {
                req = req.header(REFERER, hv);
            }
        }
        if let Some(rg) = range {
            req = req.header(RANGE, rg.clone());
        }

        let resp = req.send().await.map_err(|e| {
            tracing::warn!(
                host = current.host_str().unwrap_or("?"),
                "Proxy-Upstream-Fehler: {e}"
            );
            (StatusCode::BAD_GATEWAY, "upstream")
        })?;

        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or((StatusCode::BAD_GATEWAY, "redirect ohne location"))?;
            current = current
                .join(loc)
                .map_err(|_| (StatusCode::BAD_GATEWAY, "bad redirect"))?;
            continue;
        }
        return Ok((current, resp));
    }
    Err((StatusCode::BAD_GATEWAY, "zu viele redirects"))
}

/// Liefert die (nicht-weiterleitende) Antwort aus: HLS-Playlists umgeschrieben,
/// alles andere mit erhaltenen Headern durchgestreamt.
async fn deliver(
    resp: reqwest::Response,
    final_url: &reqwest::Url,
    referer: Option<&str>,
    st: &ProxyState,
) -> Response {
    let status = resp.status();
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_hls = ct.contains("mpegurl")
        || ct.contains("vnd.apple")
        || final_url.path().to_ascii_lowercase().ends_with(".m3u8");

    tracing::debug!(
        status = status.as_u16(),
        content_type = %ct,
        is_hls,
        host = final_url.host_str().unwrap_or("?"),
        "Proxy liefert aus"
    );

    if is_hls {
        let text = match resp.text().await {
            Ok(t) => t,
            Err(_) => return (StatusCode::BAD_GATEWAY, "body").into_response(),
        };
        let body = rewrite_playlist(&text, final_url, referer, &st.base, &st.token);
        return ([(CONTENT_TYPE, "application/vnd.apple.mpegurl")], body).into_response();
    }

    let mut builder = Response::builder().status(status);
    for name in [CONTENT_TYPE, CONTENT_LENGTH, CONTENT_RANGE, ACCEPT_RANGES] {
        if let Some(v) = resp.headers().get(&name) {
            builder = builder.header(name, v.clone());
        }
    }
    match builder.body(Body::from_stream(resp.bytes_stream())) {
        Ok(r) => r,
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "body").into_response(),
    }
}

/// Prüft Schema, Hostname-Allowlist und IP-Literale. Für Hostnamen setzt der
/// Guard-Resolver des Clients die IP-Prüfung beim Verbinden durch (TOCTOU-frei).
fn ensure_public(url: &reqwest::Url) -> Result<(), (StatusCode, &'static str)> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err((StatusCode::BAD_REQUEST, "scheme"));
    }
    let host = url.host_str().ok_or((StatusCode::BAD_REQUEST, "no host"))?;
    if is_blocked_hostname(host) {
        return Err((StatusCode::FORBIDDEN, "blocked host"));
    }
    // IP-Literale ruft der Resolver nicht auf — hier direkt abweisen.
    let bare = host.trim_matches(|c| c == '[' || c == ']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err((StatusCode::FORBIDDEN, "private ip"));
        }
    }
    Ok(())
}

/// Fester Upstream-DNS über Cloudflare 1.1.1.1 (nie System-DNS) — umgeht
/// DNS-Sperren und folgt Barons „1.1.1.1 überall"-Regel.
fn resolver_1111() -> &'static hickory_resolver::TokioAsyncResolver {
    use hickory_resolver::config::{ResolverConfig, ResolverOpts};
    use std::sync::OnceLock;
    static R: OnceLock<hickory_resolver::TokioAsyncResolver> = OnceLock::new();
    R.get_or_init(|| {
        hickory_resolver::TokioAsyncResolver::tokio(
            ResolverConfig::cloudflare(),
            ResolverOpts::default(),
        )
    })
}

/// DNS-Resolver über 1.1.1.1, der private/lokale IPs schon beim Auflösen entfernt.
/// Bleibt nichts übrig, schlägt die Auflösung fehl — die Verbindung kommt nie zustande.
struct GuardResolver;

impl Resolve for GuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let lookup = resolver_1111().lookup_ip(name.as_str()).await?;
            let allowed: Vec<SocketAddr> = lookup
                .iter()
                .filter(|ip| !is_private_ip(*ip))
                .map(|ip| SocketAddr::new(ip, 0))
                .collect();
            if allowed.is_empty() {
                let e: Box<dyn std::error::Error + Send + Sync> =
                    "kein erlaubtes Ziel (privat/lokal blockiert)".into();
                return Err(e);
            }
            let out: Addrs = Box::new(allowed.into_iter());
            Ok(out)
        })
    }
}

fn is_blocked_hostname(host: &str) -> bool {
    let h = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    h == "localhost"
        || h.ends_with(".localhost")
        || h == "metadata"
        || h == "metadata.google.internal"
}

/// Private/lokale/reservierte IP? Kanonisiert zuerst (IPv4-mapped-v6 → v4).
fn is_private_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || v4.octets()[0] == 0
                || is_cgnat(v4)
                || is_benchmarking(v4)
                || is_ietf_protocol(v4)
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link local fe80::/10
                || seg[0] == 0x2002 // 6to4 2002::/16
                || (seg[0] == 0x2001 && seg[1] == 0x0000) // Teredo 2001::/32
                || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64 64:ff9b::/96
        }
    }
}

fn is_cgnat(v4: Ipv4Addr) -> bool {
    // 100.64.0.0/10
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}
fn is_benchmarking(v4: Ipv4Addr) -> bool {
    // 198.18.0.0/15
    let o = v4.octets();
    o[0] == 198 && (o[1] & 0xfe) == 18
}
fn is_ietf_protocol(v4: Ipv4Addr) -> bool {
    // 192.0.0.0/24 (IETF protocol assignments)
    let o = v4.octets();
    o[0] == 192 && o[1] == 0 && o[2] == 0
}

fn host_is_local(headers: &HeaderMap) -> bool {
    let Some(h) = headers.get(HOST).and_then(|v| v.to_str().ok()) else {
        // Kein Host-Header: HTTP/1.0-Client gibt es im WebView nicht — ablehnen.
        return false;
    };
    let host = h.split(':').next().unwrap_or("");
    host == "127.0.0.1" || host == "localhost" || host == "[::1]" || host == "::1"
}

/// Schreibt eine HLS-Playlist so um, dass alle Kind-URLs wieder durch den Proxy laufen.
fn rewrite_playlist(
    text: &str,
    base: &reqwest::Url,
    referer: Option<&str>,
    proxy_base: &str,
    token: &str,
) -> String {
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            out.push('\n');
            continue;
        }
        if let Some(stripped) = t.strip_prefix('#') {
            if let Some(rw) = rewrite_uri_attr(stripped, base, referer, proxy_base, token) {
                out.push('#');
                out.push_str(&rw);
            } else {
                out.push_str(t);
            }
            out.push('\n');
            continue;
        }
        match base.join(t) {
            Ok(abs) => out.push_str(&proxy_url(proxy_base, token, abs.as_str(), referer)),
            Err(_) => out.push_str(t),
        }
        out.push('\n');
    }
    out
}

/// Ersetzt das `URI="…"` in einem EXT-X-Tag durch eine proxied absolute URL.
fn rewrite_uri_attr(
    tag: &str,
    base: &reqwest::Url,
    referer: Option<&str>,
    proxy_base: &str,
    token: &str,
) -> Option<String> {
    let key = "URI=\"";
    let start = tag.find(key)? + key.len();
    let end = tag[start..].find('"')? + start;
    let inner = &tag[start..end];
    let abs = base.join(inner).ok()?;
    let proxied = proxy_url(proxy_base, token, abs.as_str(), referer);
    Some(format!("{}{}{}", &tag[..start], proxied, &tag[end..]))
}

fn proxy_url(proxy_base: &str, token: &str, upstream: &str, referer: Option<&str>) -> String {
    let mut s = format!(
        "{}/s?t={}&u={}",
        proxy_base,
        token,
        URL_SAFE_NO_PAD.encode(upstream.as_bytes())
    );
    if let Some(r) = referer {
        s.push_str("&r=");
        s.push_str(&URL_SAFE_NO_PAD.encode(r.as_bytes()));
    }
    s
}

fn b64d(s: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(s).ok()?;
    String::from_utf8(bytes).ok()
}

/// Konstant-zeit Vergleich, damit das Token nicht über Timing erraten wird.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// CSPRNG-Token. Fail-closed: schlägt getrandom fehl, startet der Proxy nicht.
fn random_token() -> std::io::Result<String> {
    let mut buf = [0u8; 24];
    getrandom::getrandom(&mut buf).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}
