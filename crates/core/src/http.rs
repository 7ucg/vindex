//! HTTP-Client-Konstruktion, User-Agent-Rotation und Cloudflare-Erkennung.
//!
//! Der gestufte Anti-Bot-Ansatz (reqwest → `wreq`-TLS-Impersonate →
//! Headless-Browser-Fallback) ist in `ARCHITECTURE.md` §2.1/§5 beschrieben. Hier sind
//! Stufe 1 (reqwest + UA-Rotation) und die Challenge-Erkennung umgesetzt; die
//! Impersonate-/Browser-Stufen sind als Erweiterungspunkte markiert.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::model::USER_AGENT;

/// Pool browser-ähnlicher User-Agents (gekürzter Auszug; vgl. `browsers.jsonl`
/// der Referenz mit ~10k echten UAs). Für Produktivbetrieb erweitern.
pub const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
];

static UA_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Nächster User-Agent aus dem Pool (round-robin).
pub fn next_user_agent() -> &'static str {
    let i = UA_CURSOR.fetch_add(1, Ordering::Relaxed);
    USER_AGENTS[i % USER_AGENTS.len()]
}

static PROXY_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// True, wenn *entry* eine brauchbare Proxy-URL ist. Die Prüfung nutzt denselben
/// Parser wie der Consumer (`reqwest::Proxy::all`), damit Validator und Verbraucher
/// nicht auseinanderlaufen; zusätzlich werden gesperrte IP-Literale abgewiesen.
fn proxy_entry_ok(entry: &str) -> bool {
    // Erst normalisieren und prüfen (verlangt ein bekanntes Schema — schließt
    // schemalose Einträge wie "127.0.0.1:8080" aus, die sonst die IP-Prüfung
    // umgehen würden), dann die Grammatik des Consumers (`Proxy::all`) bestätigen.
    let Ok(url) = reqwest::Url::parse(entry) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return false;
    }
    if let Some(host) = url.host_str() {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if is_forbidden_ip(ip) {
                return false;
            }
        }
    }
    reqwest::Proxy::all(entry).is_ok()
}

/// True, wenn jede komma-getrennte Proxy-Angabe in *s* brauchbar ist (gleicher
/// Parser wie der Consumer). Leer/whitespace = gültig (bedeutet: kein Proxy).
pub fn proxy_string_ok(s: &str) -> bool {
    s.split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .all(proxy_entry_ok)
}

/// Proxy-Pool aus `VINDEX_PROXY` (komma-getrennt). Ungültige Einträge werden
/// verworfen; leer/ungesetzt → kein Proxy. Werden Einträge verworfen, wird das
/// geloggt — so sieht der Operator, dass Traffic ggf. ungeproxt läuft.
fn proxy_pool() -> &'static [String] {
    static POOL: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let raw = std::env::var("VINDEX_PROXY").unwrap_or_default();
        let total = raw.split(',').filter(|s| !s.trim().is_empty()).count();
        let pool: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && proxy_entry_ok(s))
            .collect();
        if pool.len() < total {
            tracing::warn!(
                "VINDEX_PROXY: {} von {} Einträgen verworfen (ungültig/gesperrt)",
                total - pool.len(),
                total
            );
        }
        pool
    })
}

/// True, wenn ein Proxy-Pool (`VINDEX_PROXY`) konfiguriert ist. Dient Stufe 3
/// (obscura) als Fail-closed-Signal: ihr lokaler 1.1.1.1-Proxy würde einen
/// konfigurierten Egress-Proxy umgehen, also wird Stufe 3 dann abgelehnt.
pub(crate) fn proxy_configured() -> bool {
    !proxy_pool().is_empty()
}

/// Round-Robin-Auswahl aus einem Pool (rein, testbar).
fn rotate(cursor: usize, pool: &[String]) -> Option<&str> {
    if pool.is_empty() {
        None
    } else {
        Some(pool[cursor % pool.len()].as_str())
    }
}

/// Nächster Proxy aus dem Pool (round-robin). `None`, wenn keiner konfiguriert ist.
fn next_proxy() -> Option<String> {
    let i = PROXY_CURSOR.fetch_add(1, Ordering::Relaxed);
    rotate(i, proxy_pool()).map(str::to_string)
}

/// Hängt den nächsten Proxy an den reqwest-Builder. Schlägt ein konfigurierter
/// Proxy beim Bauen fehl, wird ein Fehler zurückgegeben (fail-closed: lieber kein
/// Request als ungewollt ungeproxter Traffic). Kein Proxy → unverändert.
///
/// SSRF-Hinweis: Über einen Proxy löst der PROXY den Zielhost auf — der eigene
/// DNS-Filter (1.1.1.1 + [`is_forbidden_ip`]) greift für den finalen Connect dann
/// nicht mehr. Ein Proxy ist eine bewusste, vertrauenswürdige Operator-Konfiguration;
/// die Host-Allowlist (`site::validate_source_url`) am Eingang bleibt wirksam.
fn apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::Result<reqwest::ClientBuilder> {
    match next_proxy() {
        Some(p) => Ok(builder.proxy(reqwest::Proxy::all(&p)?)),
        None => Ok(builder),
    }
}

/// Marker, die auf eine Cloudflare-/Captcha-Interstitial-Seite hindeuten
/// (aus `browser_fetch.py` / `models/common/http.py`).
pub const CHALLENGE_MARKERS: &[&str] = &[
    "just a moment",
    "verifying you are human",
    "attention required",
    "cf-browser-verification",
    "challenge-platform",
    "_cf_chl",
    "cf_chl",
    "captcha",
    "verifizierung",
    "verification required",
];

/// Heuristik: Ist *html* (mit *status*) eine Challenge-/Captcha-Seite?
pub fn looks_like_challenge(html: &str, status: u16) -> bool {
    if matches!(status, 403 | 429 | 503) {
        return true;
    }
    let lowered = html
        .get(..html.len().min(4000))
        .unwrap_or(html)
        .to_lowercase();
    CHALLENGE_MARKERS.iter().any(|m| lowered.contains(m))
}

/// Baut den Standard-HTTP-Client (gzip, 20 s Timeout, fester Default-UA).
pub fn client() -> reqwest::Result<reqwest::Client> {
    let b = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .gzip(true)
        .timeout(Duration::from_secs(20))
        .redirect(safe_redirect_policy())
        .dns_resolver(std::sync::Arc::new(GuardedResolver));
    apply_proxy(b)?.build()
}

/// Kurzlebiger Client mit eigenem, frischem Cookie-Jar — nur für Flows, die
/// Session-/CSRF-Cookies über mehrere Requests brauchen (filmo.to: Film-GET setzt
/// die Laravel-Session, der `/n`-POST braucht sie). Bewusst **nicht** auf den
/// geteilten Langzeit-Clients, damit kein Cookie-Zustand über Flows/Hosts/Nutzer
/// hinweg geteilt wird. Der Jar ist reqwest-intern pro Domain gescopet.
pub fn cookie_client() -> reqwest::Result<reqwest::Client> {
    let b = reqwest::Client::builder()
        .user_agent(next_user_agent())
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .redirect(safe_redirect_policy())
        .cookie_store(true)
        .dns_resolver(std::sync::Arc::new(GuardedResolver));
    apply_proxy(b)?.build()
}

/// Baut einen Client mit rotierendem (browser-ähnlichem) User-Agent — für die
/// Hoster-/Episodenseiten, die auf plausible Clients achten.
pub fn browser_client() -> reqwest::Result<reqwest::Client> {
    let b = reqwest::Client::builder()
        .user_agent(next_user_agent())
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .redirect(safe_redirect_policy())
        .dns_resolver(std::sync::Arc::new(GuardedResolver));
    apply_proxy(b)?.build()
}

/// Fester Upstream-DNS. Cloudflare 1.1.1.1 umgeht DNS-Sperren des System-Resolvers,
/// über die einzelne Portale sonst nicht auflösbar sind. Überschreibbar via
/// `VINDEX_DNS` (eine oder mehrere IPs, komma-getrennt).
pub(crate) fn upstream_resolver() -> &'static hickory_resolver::TokioAsyncResolver {
    use std::sync::OnceLock;
    static R: OnceLock<hickory_resolver::TokioAsyncResolver> = OnceLock::new();
    R.get_or_init(|| {
        use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
        let group = match std::env::var("VINDEX_DNS") {
            Ok(v) => {
                // Nur öffentliche Resolver-IPs zulassen: ein privater/loopback DNS
                // könnte alle Lookups umbiegen und den Adressfilter aushebeln.
                let ips: Vec<std::net::IpAddr> = v
                    .split(',')
                    .filter_map(|s| s.trim().parse::<std::net::IpAddr>().ok())
                    .filter(|ip| !is_forbidden_ip(*ip))
                    .collect();
                if ips.is_empty() {
                    NameServerConfigGroup::cloudflare()
                } else {
                    NameServerConfigGroup::from_ips_clear(&ips, 53, true)
                }
            }
            Err(_) => NameServerConfigGroup::cloudflare(),
        };
        let config = ResolverConfig::from_parts(None, vec![], group);
        hickory_resolver::TokioAsyncResolver::tokio(config, ResolverOpts::default())
    })
}

/// DNS-Resolver über einen festen Upstream (1.1.1.1), der jede aufgelöste Adresse
/// gegen [`is_forbidden_ip`] prüft.
///
/// 1.1.1.1 umgeht DNS-basierte Portal-Sperren; der Adressfilter schließt zugleich
/// DNS-Rebinding (ein Hostname, der auf eine interne IP zeigt). Löst ein Name nur
/// auf gesperrte Adressen auf, schlägt die Verbindung fehl. IP-Literale umgehen
/// den Resolver (reqwest verbindet direkt) — die deckt die Redirect-Policy ab.
struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let lookup = upstream_resolver().lookup_ip(name.as_str()).await?;
            let allowed: Vec<std::net::SocketAddr> = lookup
                .iter()
                .filter(|ip| !is_forbidden_ip(*ip))
                .map(|ip| std::net::SocketAddr::new(ip, 0))
                .collect();
            if allowed.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    "Ziel löst nur auf gesperrte Adressen auf".into();
                return Err(err);
            }
            let iter: reqwest::dns::Addrs = Box::new(allowed.into_iter());
            Ok(iter)
        })
    }
}

/// Redirect-Policy, die jeden Hop prüft, bevor ihm gefolgt wird: nur `http`/
/// `https`, keine privaten/loopback/link-local IP-Ziele, höchstens 10 Hops.
///
/// Schließt die SSRF-Lücke, dass ein (erlaubtes) Portal per `3xx` auf ein
/// internes Ziel umleitet. Restrisiko: ein Hostname, der erst per DNS auf eine
/// private IP zeigt (DNS-Rebinding) — dafür bräuchte es einen eigenen Resolver.
fn safe_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.stop();
        }
        let url = attempt.url();
        if !matches!(url.scheme(), "http" | "https") {
            return attempt.stop();
        }
        // Kein https→http-Downgrade: sonst könnten Session-Cookies (filmo.to-Flow)
        // oder Tokens über Klartext abfließen.
        if url.scheme() == "http"
            && attempt
                .previous()
                .last()
                .is_some_and(|p| p.scheme() == "https")
        {
            return attempt.stop();
        }
        if let Some(host) = url.host_str() {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                if is_forbidden_ip(ip) {
                    return attempt.stop();
                }
            }
        }
        attempt.follow()
    })
}

/// True, wenn der Host von *url* ein IP-Literal aus einem gesperrten Bereich ist.
/// Für Hostnamen (die erst aufgelöst werden) greift stattdessen der DNS-Resolver.
pub(crate) fn host_is_forbidden_ip_literal(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => is_forbidden_ip(ip),
        Err(_) => false,
    }
}

/// True für IP-Ziele, auf die ein Redirect nicht zeigen darf (loopback, privat,
/// link-local, unspecified, CGNAT, unique-local u. a.). IPv6 wird vor der Prüfung
/// auf eingebettete IPv4 normalisiert (mapped, 6to4, NAT64), damit die Sperre
/// nicht über eine andere Schreibweise derselben Adresse umgangen wird.
pub(crate) fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0 // "aktuelles Netz" 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64.0.0/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF-Protokoll 192.0.0.0/24
                || (o[0] == 192 && o[1] == 88 && o[2] == 99) // 6to4-Relay-Anycast 192.88.99.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // Benchmark 198.18.0.0/15
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(mapped));
            }
            let seg = v6.segments();
            // 6to4 2002::/16 — eingebettete IPv4 in den Segmenten 1..2.
            if seg[0] == 0x2002 {
                let v4 = Ipv4Addr::new(
                    (seg[1] >> 8) as u8,
                    (seg[1] & 0xff) as u8,
                    (seg[2] >> 8) as u8,
                    (seg[2] & 0xff) as u8,
                );
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            // NAT64 64:ff9b::/96 — eingebettete IPv4 in den Segmenten 6..7.
            if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6].iter().all(|s| *s == 0) {
                let v4 = Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    (seg[6] & 0xff) as u8,
                    (seg[7] >> 8) as u8,
                    (seg[7] & 0xff) as u8,
                );
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        }
    }
}

/// Maximale Zusatzversuche bei transienten Fehlern.
const RETRY_ATTEMPTS: u32 = 3;

/// Entscheidet, ob ein Status einen erneuten Versuch rechtfertigt (429/5xx —
/// „dieser Host bedient uns gerade nicht", nicht „Seite fehlt").
fn status_is_transient(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// `GET` mit begrenzten Wiederholungen bei transienten Fehlern: Verbindungs-/
/// Timeout-Fehler sowie Status 429/5xx werden mit wachsendem Backoff erneut
/// versucht. Permanente Antworten (2xx, 3xx, 4xx außer 429) kommen sofort zurück.
pub async fn get_retry(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
) -> reqwest::Result<reqwest::Response> {
    let mut last_err: Option<reqwest::Error> = None;
    for attempt in 0..=RETRY_ATTEMPTS {
        if attempt > 0 {
            let backoff = std::time::Duration::from_millis(400 * (1 << (attempt - 1)));
            tokio::time::sleep(backoff).await;
        }
        let mut req = client.get(url);
        if let Some(r) = referer {
            req = req.header("Referer", r);
        }
        match req.send().await {
            Ok(resp) if status_is_transient(resp.status()) && attempt < RETRY_ATTEMPTS => continue,
            Ok(resp) => return Ok(resp),
            Err(e) => {
                let retryable = e.is_timeout() || e.is_connect() || e.is_request();
                if retryable && attempt < RETRY_ATTEMPTS {
                    last_err = Some(e);
                    continue;
                }
                return Err(e);
            }
        }
    }
    // Nur erreichbar, wenn alle Versuche transient scheiterten.
    Err(last_err.expect("retry loop endete ohne Ergebnis"))
}

/// Wie [`get_retry`], gibt aber direkt den Body-Text zurück.
pub async fn get_text_retry(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
) -> reqwest::Result<String> {
    get_retry(client, url, referer).await?.text().await
}

/// Liest den Body einer Antwort als Text, aber höchstens `cap` Bytes. Verhindert,
/// dass eine manipulierte Antwort (z. B. einer externen API) den Speicher flutet.
/// `Err` mit [`crate::error::Error::Parse`], sobald der Body `cap` überschreitet.
pub async fn capped_text(resp: reqwest::Response, cap: usize) -> crate::error::Result<String> {
    use futures::StreamExt as _;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if buf.len() + chunk.len() > cap {
            return Err(crate::error::Error::Parse(
                "Antwort überschreitet Größenlimit".into(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

// ───────────────────────────── Anti-Bot Stufe 2 ─────────────────────────────
//
// TLS-Impersonate-Fallback über `wreq` (BoringSSL-basierte JA3/JA4-Fingerprints).
// Opt-in: nur aktiv, wenn `VINDEX_IMPERSONATE` gesetzt ist (siehe `config.rs`,
// Default aus). Standard-Stufe bleibt reqwest; erst bei Cloudflare-Verdacht wird
// die Anfrage *einmalig* mit einem Chrome-Fingerprint wiederholt.
//
// SSRF/DNS: `wreq` bietet mit [`wreq::dns::Resolve`] dieselbe Resolver-Schnittstelle
// wie reqwest. Der Impersonate-Client nutzt deshalb denselben festen Upstream
// (1.1.1.1) und denselben Adressfilter ([`is_forbidden_ip`]) wie [`GuardedResolver`]
// — der Rebinding-/Interne-Ziel-Schutz greift also auch auf dem wreq-Pfad. Zusätzlich
// prüft [`get_text_impersonate`] vor dem Abruf Schema und IP-Literal der Ziel-URL.

/// Prozessweiter Schalter für den Impersonate-Fallback. Wird aus
/// [`crate::config::Settings::load`] gesetzt (ENV `VINDEX_IMPERSONATE`), damit die
/// Fetch-Schicht ohne Durchreichen von `Settings` an jeden Scraper-Aufruf auskommt.
static IMPERSONATE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Setzt den prozessweiten Impersonate-Schalter (idempotent).
pub fn set_impersonate(enabled: bool) {
    IMPERSONATE_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Liest den prozessweiten Impersonate-Schalter.
pub fn impersonate_enabled() -> bool {
    IMPERSONATE_ENABLED.load(Ordering::Relaxed)
}

/// Reine Entscheidungslogik: Soll für diese Antwort der Impersonate-Pfad genommen
/// werden? Nur wenn *flag* an ist **und** die Antwort nach einer Challenge aussieht
/// (Status 403/429/503 oder ein Interstitial-Marker im Body, vgl.
/// [`looks_like_challenge`]). Netzfrei testbar.
pub fn should_impersonate(flag: bool, status: u16, body: &str) -> bool {
    flag && looks_like_challenge(body, status)
}

/// DNS-Resolver für den wreq-Impersonate-Client. Identisch zu [`GuardedResolver`],
/// nur gegen die `wreq::dns`-Schnittstelle: fester Upstream (1.1.1.1) plus
/// Adressfilter gegen interne Ziele/DNS-Rebinding.
struct WreqGuardedResolver;

impl wreq::dns::Resolve for WreqGuardedResolver {
    fn resolve(&self, name: wreq::dns::Name) -> wreq::dns::Resolving {
        Box::pin(async move {
            let lookup = upstream_resolver().lookup_ip(name.as_str()).await?;
            let allowed: Vec<std::net::SocketAddr> = lookup
                .iter()
                .filter(|ip| !is_forbidden_ip(*ip))
                .map(|ip| std::net::SocketAddr::new(ip, 0))
                .collect();
            if allowed.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    "Ziel löst nur auf gesperrte Adressen auf".into();
                return Err(err);
            }
            let iter: wreq::dns::Addrs = Box::new(allowed.into_iter());
            Ok(iter)
        })
    }
}

/// Redirect-Policy des Impersonate-Clients — analog zu [`safe_redirect_policy`]:
/// nur http/https, keine gesperrten IP-Literale als Ziel, höchstens 10 Hops.
fn wreq_safe_redirect() -> wreq::redirect::Policy {
    wreq::redirect::Policy::custom(|attempt| {
        if attempt.previous.len() >= 10 {
            return attempt.stop();
        }
        let uri = attempt.uri.as_ref();
        match uri.scheme_str() {
            Some("http") | Some("https") => {}
            _ => return attempt.stop(),
        }
        if let Some(host) = uri.host() {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                if is_forbidden_ip(ip) {
                    return attempt.stop();
                }
            }
        }
        attempt.follow()
    })
}

/// Baut (einmalig, gecached) den wreq-Client mit Chrome-Impersonation. `Chrome149`
/// ist der neueste von `wreq-util` bereitgestellte Chrome-Preset; er setzt den
/// kompletten TLS/HTTP2/Header-Fingerprint inkl. passendem User-Agent.
fn impersonate_client() -> Result<wreq::Client, wreq::Error> {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<wreq::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c.clone());
    }
    let mut builder = wreq::Client::builder()
        .emulation(wreq_util::Emulation::Chrome149)
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .redirect(wreq_safe_redirect())
        .dns_resolver(std::sync::Arc::new(WreqGuardedResolver));
    // Proxy auch auf dem Impersonate-Pfad (fail-closed: Build-Fehler statt ungeproxt).
    if let Some(p) = next_proxy() {
        builder = builder.proxy(wreq::Proxy::all(&p)?);
    }
    let client = builder.build()?;
    // Bei Race den zuerst gesetzten Client verwenden (wreq::Client ist Arc-intern,
    // Clone ist billig).
    let _ = CLIENT.set(client.clone());
    Ok(client)
}

/// Holt *url* über den TLS-Impersonate-Client (Chrome-Fingerprint).
///
/// SSRF: wie [`crate::hoster::resolve`] werden non-http(s)-URLs und IP-Literale aus
/// gesperrten Bereichen *vor* dem Abruf abgewiesen; Hostnamen deckt der
/// [`WreqGuardedResolver`] ab. Der echte Abruf braucht Netz/ein Cloudflare-Ziel und
/// ist daher nicht netzfrei testbar — getestet wird nur die Vorab-Validierung.
pub async fn get_text_impersonate(
    url: &str,
    referer: Option<&str>,
) -> crate::error::Result<String> {
    use crate::error::Error;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(Error::Parse(
            "Impersonate: Ziel-URL muss http/https sein".to_string(),
        ));
    }
    if host_is_forbidden_ip_literal(url) {
        return Err(Error::Parse(
            "Impersonate: Ziel-URL zeigt auf eine gesperrte Adresse".to_string(),
        ));
    }
    let client = impersonate_client()
        .map_err(|e| Error::Other(anyhow::anyhow!("TLS-Impersonate-Client (wreq): {e}")))?;
    let mut req = client.get(url);
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!("TLS-Impersonate-Abruf (wreq): {e}")))?;
    resp.text()
        .await
        .map_err(|e| Error::Other(anyhow::anyhow!("TLS-Impersonate-Body (wreq): {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_entry_validierung() {
        assert!(proxy_entry_ok("http://proxy.example:8080"));
        assert!(proxy_entry_ok("socks5://1.1.1.1:1080"));
        assert!(!proxy_entry_ok("http://127.0.0.1:8080")); // gesperrtes IP-Literal
        assert!(!proxy_entry_ok("http://169.254.169.254:3128")); // Metadata-IP
        assert!(!proxy_entry_ok("127.0.0.1:8080")); // schemalos → abgewiesen
        assert!(!proxy_entry_ok("169.254.169.254:3128")); // schemalos + Metadata
        assert!(!proxy_entry_ok("ftp://proxy:21")); // Schema nicht erlaubt
        assert!(!proxy_entry_ok("kein url")); // nicht parsebar
    }

    #[test]
    fn proxy_string_validierung() {
        assert!(proxy_string_ok("")); // leer = kein Proxy = ok
        assert!(proxy_string_ok("   ")); // nur whitespace = ok
        assert!(proxy_string_ok(
            "http://a.example:8080 , socks5://b.example:1080"
        ));
        assert!(!proxy_string_ok(
            "http://a.example:8080, http://127.0.0.1:1"
        )); // ein Eintrag gesperrt
        assert!(!proxy_string_ok("kein url"));
    }

    #[test]
    fn proxy_rotation_round_robin() {
        let pool = vec!["http://a:1".to_string(), "http://b:2".to_string()];
        assert_eq!(rotate(0, &pool), Some("http://a:1"));
        assert_eq!(rotate(1, &pool), Some("http://b:2"));
        assert_eq!(rotate(2, &pool), Some("http://a:1")); // Umlauf
        assert_eq!(rotate(0, &[]), None); // leerer Pool
        let single = vec!["http://x:9".to_string()];
        assert_eq!(rotate(7, &single), Some("http://x:9"));
    }

    #[test]
    fn transient_status_classification() {
        use reqwest::StatusCode;
        assert!(status_is_transient(StatusCode::TOO_MANY_REQUESTS));
        assert!(status_is_transient(StatusCode::BAD_GATEWAY));
        assert!(status_is_transient(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!status_is_transient(StatusCode::NOT_FOUND));
        assert!(!status_is_transient(StatusCode::OK));
    }

    #[test]
    fn challenge_by_status() {
        assert!(looks_like_challenge("", 403));
        assert!(looks_like_challenge("", 429));
        assert!(!looks_like_challenge("<html>ok</html>", 200));
    }

    #[test]
    fn challenge_by_marker() {
        assert!(looks_like_challenge("<title>Just a moment...</title>", 200));
        assert!(looks_like_challenge("please complete the CAPTCHA", 200));
    }

    #[test]
    fn forbidden_ips_blocked() {
        use std::net::IpAddr;
        let f = |s: &str| is_forbidden_ip(s.parse::<IpAddr>().unwrap());
        assert!(f("127.0.0.1"));
        assert!(f("169.254.169.254")); // Cloud-Metadata
        assert!(f("10.0.0.1"));
        assert!(f("192.168.1.1"));
        assert!(f("100.64.0.1")); // CGNAT
        assert!(f("::1"));
        assert!(f("fe80::1"));
        assert!(f("fc00::1"));
        assert!(f("::ffff:127.0.0.1")); // IPv4-mapped
        assert!(f("0.0.0.0"));
        assert!(f("192.0.0.1")); // 192.0.0.0/24
        assert!(f("2002:7f00:1::")); // 6to4 auf 127.0.0.1
        assert!(f("64:ff9b::a9fe:a9fe")); // NAT64 auf 169.254.169.254
        assert!(!f("93.184.216.34")); // example.com, öffentlich
        assert!(!f("1.1.1.1"));
        assert!(!f("2002:5db8:1234::")); // 6to4 auf öffentliche 93.184.18.52
    }

    #[test]
    fn impersonate_entscheidung_nur_bei_flag_und_challenge() {
        // Flag aus: nie impersonaten, egal wie die Antwort aussieht.
        assert!(!should_impersonate(false, 403, ""));
        assert!(!should_impersonate(false, 200, "just a moment"));
        // Flag an, aber saubere 200-Antwort: kein Impersonate.
        assert!(!should_impersonate(true, 200, "<html>ok</html>"));
        // Flag an + Challenge (Status bzw. Marker): Impersonate.
        assert!(should_impersonate(true, 403, ""));
        assert!(should_impersonate(true, 429, ""));
        assert!(should_impersonate(true, 503, ""));
        assert!(should_impersonate(
            true,
            200,
            "<title>Just a moment...</title>"
        ));
    }

    #[test]
    fn impersonate_schalter_setz_und_lesbar() {
        set_impersonate(true);
        assert!(impersonate_enabled());
        set_impersonate(false);
        assert!(!impersonate_enabled());
    }

    #[tokio::test]
    async fn impersonate_weist_unerlaubte_ziele_vor_dem_abruf_ab() {
        // Netzfrei: die SSRF-Vorprüfung greift vor jedem Verbindungsversuch.
        assert!(get_text_impersonate("ftp://example.com/x", None)
            .await
            .is_err());
        assert!(get_text_impersonate("http://127.0.0.1/x", None)
            .await
            .is_err());
        assert!(get_text_impersonate("http://169.254.169.254/latest", None)
            .await
            .is_err());
    }

    #[test]
    fn ua_rotation_cycles() {
        let first = next_user_agent();
        // Nach genau einem vollen Umlauf (len Aufrufe) wieder derselbe Eintrag.
        for _ in 0..USER_AGENTS.len() - 1 {
            next_user_agent();
        }
        assert_eq!(next_user_agent(), first);
    }
}
