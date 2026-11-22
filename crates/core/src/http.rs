//! HTTP-Client-Konstruktion, User-Agent-Rotation und Cloudflare-Erkennung.
//!
//! Der gestufte Anti-Bot-Ansatz (reqwest → `wreq`-TLS-Impersonate →
//! Headless-Browser-Fallback) ist in `ARCHITECTURE.md` §2.1/§5 beschrieben. Hier sind
//! Stufe 1 (reqwest + UA-Rotation) und die Challenge-Erkennung umgesetzt; die
//! Impersonate-/Browser-Stufen sind als Erweiterungspunkte markiert.

use std::sync::atomic::{AtomicUsize, Ordering};
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
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .gzip(true)
        .timeout(Duration::from_secs(20))
        .redirect(safe_redirect_policy())
        .dns_resolver(std::sync::Arc::new(GuardedResolver))
        .build()
}

/// Baut einen Client mit rotierendem (browser-ähnlichem) User-Agent — für die
/// Hoster-/Episodenseiten, die auf plausible Clients achten.
pub fn browser_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(next_user_agent())
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .redirect(safe_redirect_policy())
        .dns_resolver(std::sync::Arc::new(GuardedResolver))
        .build()
}

/// DNS-Resolver, der jede aufgelöste Adresse gegen [`is_forbidden_ip`] prüft.
///
/// Schließt die Lücke, die die Redirect-Policy offenlässt: ein Hostname, der erst
/// per DNS auf eine private/interne IP zeigt (DNS-Rebinding). Löst nur auf
/// gesperrte Adressen auf, schlägt die Verbindung fehl. IP-Literale umgehen den
/// Resolver (reqwest verbindet direkt) — die deckt die Redirect-Policy ab.
struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<std::net::SocketAddr> =
                resolved.filter(|sa| !is_forbidden_ip(sa.ip())).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ua_rotation_cycles() {
        let first = next_user_agent();
        // Nach genau einem vollen Umlauf (len Aufrufe) wieder derselbe Eintrag.
        for _ in 0..USER_AGENTS.len() - 1 {
            next_user_agent();
        }
        assert_eq!(next_user_agent(), first);
    }
}
