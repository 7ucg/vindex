//! Lokaler Stream-Proxy für den In-Window-Player.
//!
//! `<video>`/hls.js können keinen `Referer` setzen, viele Hoster-CDNs verlangen ihn
//! aber. Dieser Proxy läuft auf `127.0.0.1:<ephemer>`, ist Token-gesichert und
//! injiziert den Referer beim Upstream-Abruf. HLS-Playlists werden umgeschrieben,
//! sodass auch Segmente und Varianten wieder durch den Proxy (mit Referer) laufen.
//! Range-Header werden durchgereicht, damit MP4-Streams spulbar sind.

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
    http::HeaderMap,
    http::{
        header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER},
        HeaderValue, StatusCode,
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
pub async fn start() -> std::io::Result<Arc<Proxy>> {
    let token = random_token();
    let client = reqwest::Client::builder()
        .user_agent(DESKTOP_UA)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

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
    if !ct_eq(q.t.as_bytes(), st.token.as_bytes()) {
        return (StatusCode::FORBIDDEN, "bad token").into_response();
    }
    let Some(upstream) = b64d(&q.u) else {
        return (StatusCode::BAD_REQUEST, "bad upstream").into_response();
    };
    let referer = q.r.as_deref().and_then(b64d);

    let Ok(url) = reqwest::Url::parse(&upstream) else {
        return (StatusCode::BAD_REQUEST, "bad url").into_response();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return (StatusCode::BAD_REQUEST, "scheme").into_response();
    }
    if is_private_host(url.host_str()) {
        return (StatusCode::FORBIDDEN, "private host").into_response();
    }

    let mut req = st.client.get(url.clone());
    if let Some(r) = &referer {
        if let Ok(hv) = HeaderValue::from_str(r) {
            req = req.header(REFERER, hv);
        }
    }
    // Range für Spulen durchreichen.
    if let Some(range) = headers.get(RANGE) {
        req = req.header(RANGE, range.clone());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("upstream: {e}")).into_response(),
    };
    let status = resp.status();
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_hls = ct.contains("mpegurl")
        || ct.contains("vnd.apple")
        || url.path().to_ascii_lowercase().ends_with(".m3u8");

    if is_hls {
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => return (StatusCode::BAD_GATEWAY, format!("body: {e}")).into_response(),
        };
        let body = rewrite_playlist(&text, &url, referer.as_deref(), &st.base, &st.token);
        return ([(CONTENT_TYPE, "application/vnd.apple.mpegurl")], body).into_response();
    }

    // Direkte Datei: Status + relevante Header erhalten, Body durchstreamen.
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
            // Tags mit URI="..." (EXT-X-KEY, EXT-X-MEDIA, EXT-X-MAP) umschreiben.
            if let Some(rw) = rewrite_uri_attr(stripped, base, referer, proxy_base, token) {
                out.push('#');
                out.push_str(&rw);
            } else {
                out.push_str(t);
            }
            out.push('\n');
            continue;
        }
        // Segment- oder Varianten-URI.
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

/// Grobe SSRF-Bremse: offensichtlich private/lokale Hosts ablehnen. Der Proxy
/// bindet ohnehin nur auf localhost und verlangt das Token.
fn is_private_host(host: Option<&str>) -> bool {
    let Some(h) = host else {
        return true;
    };
    let h = h
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if h == "localhost" || h == "0.0.0.0" || h == "::1" || h.ends_with(".localhost") {
        return true;
    }
    if h == "metadata" || h == "metadata.google.internal" {
        return true;
    }
    if let Ok(v4) = h.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback()
            || v4.is_private()
            || v4.is_link_local()
            || v4.is_unspecified()
            || v4.octets()[0] == 0;
    }
    if let Ok(v6) = h.parse::<std::net::Ipv6Addr>() {
        let seg = v6.segments();
        return v6.is_loopback()
            || v6.is_unspecified()
            || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
            || (seg[0] & 0xffc0) == 0xfe80; // link local fe80::/10
    }
    false
}

fn random_token() -> String {
    let mut buf = [0u8; 24];
    if getrandom::getrandom(&mut buf).is_err() {
        // Fallback: Zeit-basiert (nur Notnagel; der Bind ist ohnehin localhost).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("t{nanos:x}");
    }
    URL_SAFE_NO_PAD.encode(buf)
}
