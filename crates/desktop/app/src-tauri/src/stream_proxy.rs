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

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

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

/// Griff auf den laufenden Proxy: nur Basis-URL und Token, um Player-URLs zu bauen.
pub struct Proxy {
    base: String,
    token: String,
}

struct ProxyState {
    token: String,
    client: reqwest::Client,
    base: String,
}

impl Proxy {
    /// Baut die Player-URL für einen aufgelösten Stream (Master-Playlist oder Datei).
    pub fn stream_url(&self, upstream: &str, referer: Option<&str>) -> String {
        proxy_url(&self.base, &self.token, upstream, referer)
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

    let state = Arc::new(ProxyState {
        token: token.clone(),
        client,
        base: base.clone(),
    });
    let router = Router::new().route("/s", get(serve)).with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok(Arc::new(Proxy { base, token }))
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

/// DNS-Resolver, der private/lokale IPs schon beim Auflösen entfernt. Bleibt
/// nichts übrig, schlägt die Auflösung fehl — die Verbindung kommt nie zustande.
struct GuardResolver;

impl Resolve for GuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<SocketAddr> = resolved.filter(|a| !is_private_ip(a.ip())).collect();
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
