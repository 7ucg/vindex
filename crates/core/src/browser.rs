//! Anti-Bot Stufe 3: Headless-Browser (obscura) hinter einem 1.1.1.1-Resolver-Proxy.
//!
//! obscura (h4ckf0r0day/obscura) rendert JavaScript und kommt dadurch an Seiten,
//! an denen die reqwest- (Stufe 1) und wreq-Impersonate-Pfade (Stufe 2) scheitern.
//! obscura nutzt aber System-DNS und hat keinen DNS-Flag — die Portale brauchen
//! jedoch 1.1.1.1. Deshalb startet vindex für den Aufruf einen lokalen HTTP-CONNECT-
//! Proxy (nur `127.0.0.1`, ephemerer Port, lebt nur für diesen einen Aufruf), der
//! jeden `CONNECT` über den gemeinsamen 1.1.1.1-Resolver auflöst und dabei denselben
//! SSRF-Filter ([`crate::http::is_forbidden_ip`]) anwendet wie der Rest von vindex.
//! obscura wird mit `--proxy http://127.0.0.1:<port>` gestartet.
//!
//! Opt-in über `VINDEX_OBSCURA` (Pfad zum obscura-Binary). Ohne das ist Stufe 3 aus.

use std::net::IpAddr;
use std::sync::OnceLock;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

use crate::error::{Error, Result};

/// Prozessweiter Pfad zum obscura-Binary (aus `VINDEX_OBSCURA`). `None`/leer ⇒ aus.
static OBSCURA_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Setzt den obscura-Pfad (idempotent; aus [`crate::config::Settings::load`]).
pub fn set_obscura_path(path: Option<String>) {
    let _ = OBSCURA_PATH.set(path.filter(|p| !p.trim().is_empty()));
}

fn obscura_path() -> Option<&'static str> {
    OBSCURA_PATH.get().and_then(|o| o.as_deref())
}

/// True, wenn Stufe 3 aktiv ist (obscura-Pfad gesetzt).
pub fn obscura_enabled() -> bool {
    obscura_path().is_some()
}

/// Obergrenze für die von obscura gelieferte Seite.
const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
/// obscura-interner Navigations-Timeout (Sekunden).
const FETCH_TIMEOUT_SECS: &str = "45";

/// True, wenn *url* eine absolute http(s)-URL ist.
fn is_http_url(url: &str) -> bool {
    reqwest::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Holt die gerenderte Seite über obscura. DNS läuft über den lokalen 1.1.1.1-
/// Resolver-Proxy. `Err`, wenn Stufe 3 aus ist, die URL kein http(s) ist oder
/// obscura scheitert.
pub async fn fetch_html(url: &str) -> Result<String> {
    let bin = obscura_path().ok_or_else(|| Error::extract("obscura", "Stufe 3 nicht aktiv"))?;
    if !is_http_url(url) {
        return Err(Error::extract("obscura", "URL ist nicht http/https"));
    }

    // Lokalen 1.1.1.1-CONNECT-Proxy starten; lebt nur für diesen Aufruf.
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(Error::Io)?;
    let addr = listener.local_addr().map_err(Error::Io)?;
    let proxy = tokio::spawn(proxy_loop(listener));

    let proxy_arg = format!("http://{addr}");
    let result = Command::new(bin)
        .arg("--proxy")
        .arg(&proxy_arg)
        .arg("fetch")
        .arg(url)
        .arg("--dump")
        .arg("html")
        .arg("--quiet")
        .arg("--stealth")
        .arg("--timeout")
        .arg(FETCH_TIMEOUT_SECS)
        .kill_on_drop(true)
        .output()
        .await;

    proxy.abort();
    let out = result.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::extract("obscura", "Binary nicht gefunden (VINDEX_OBSCURA prüfen)")
        }
        _ => Error::Io(e),
    })?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::extract(
            "obscura",
            format!("Aufruf fehlgeschlagen: {}", err.trim()),
        ));
    }

    let mut bytes = out.stdout;
    bytes.truncate(MAX_HTML_BYTES);
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Nimmt Verbindungen des CONNECT-Proxys an, bis die Task abgebrochen wird.
async fn proxy_loop(listener: TcpListener) {
    while let Ok((sock, _)) = listener.accept().await {
        tokio::spawn(handle_connect(sock));
    }
}

/// Behandelt eine einzelne `CONNECT host:port`-Anfrage: Host über 1.1.1.1 auflösen
/// (SSRF-gefiltert), verbinden, dann bidirektional durchreichen (TLS bleibt
/// Ende-zu-Ende zwischen obscura und Ziel).
async fn handle_connect(mut client: TcpStream) {
    // CONNECT-Kopf bis zum Leerzeilen-Ende lesen (begrenzt).
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        match client.read(&mut tmp).await {
            Ok(0) => return,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 8192 {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let head = String::from_utf8_lossy(&buf);
    let Some((host, port)) = parse_connect_target(&head) else {
        let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };

    let Some(ip) = resolve_safe(&host).await else {
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        return;
    };

    let mut upstream = match TcpStream::connect((ip, port)).await {
        Ok(s) => s,
        Err(_) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return;
        }
    };

    if client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

/// Parst die `CONNECT host:port HTTP/1.1`-Startzeile zu `(host, port)`.
fn parse_connect_target(head: &str) -> Option<(String, u16)> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "CONNECT" {
        return None;
    }
    let hostport = parts.next()?;
    match hostport.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((hostport.to_string(), 443)),
    }
}

/// Löst *host* über 1.1.1.1 auf und gibt die erste **öffentliche** IP zurück
/// (gesperrte/interne IPs werden gefiltert — derselbe SSRF-Schutz wie sonst).
async fn resolve_safe(host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return (!crate::http::is_forbidden_ip(ip)).then_some(ip);
    }
    let resolver = crate::http::upstream_resolver();
    let lookup = resolver.lookup_ip(host).await.ok()?;
    lookup.iter().find(|ip| !crate::http::is_forbidden_ip(*ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_target_parsing() {
        assert_eq!(
            parse_connect_target("CONNECT serienstream.to:443 HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(("serienstream.to".to_string(), 443))
        );
        assert_eq!(
            parse_connect_target("CONNECT example.com HTTP/1.1\r\n\r\n"),
            Some(("example.com".to_string(), 443))
        );
        assert_eq!(
            parse_connect_target("CONNECT host:8080 HTTP/1.1\r\n"),
            Some(("host".to_string(), 8080))
        );
        // Kein CONNECT → None.
        assert_eq!(parse_connect_target("GET / HTTP/1.1\r\n"), None);
        assert_eq!(parse_connect_target(""), None);
    }

    #[test]
    fn is_http_url_guard() {
        assert!(is_http_url(
            "https://serienstream.to/serie/x/staffel-1/episode-1"
        ));
        assert!(is_http_url("http://aniworld.to/"));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("ftp://x"));
        assert!(!is_http_url("nonsense"));
    }

    #[tokio::test]
    #[ignore = "braucht obscura-Binary + Netz; VINDEX_OBSCURA=<pfad> cargo test -p vindex-core browser_e2e -- --ignored --nocapture"]
    async fn browser_e2e_durch_rust_proxy() {
        let bin = std::env::var("VINDEX_OBSCURA").expect("VINDEX_OBSCURA setzen");
        set_obscura_path(Some(bin));
        // example.com (direkt erreichbar) über den 1.1.1.1-Rust-Proxy.
        let html = fetch_html("https://example.com")
            .await
            .expect("example.com");
        assert!(html.contains("Example Domain"), "example.com HTML erwartet");
        // serienstream.to (nur über 1.1.1.1 erreichbar, hinter Cloudflare).
        let s = fetch_html("https://serienstream.to/")
            .await
            .expect("serienstream.to");
        assert!(
            s.to_ascii_lowercase().contains("serienstream"),
            "serienstream-Inhalt erwartet, nicht Challenge"
        );
    }

    #[test]
    fn obscura_aus_wenn_leer() {
        // set ist idempotent (OnceLock); prüfe nur die Pfad-Filter-Logik indirekt:
        // eine leere Angabe darf Stufe 3 nicht aktivieren.
        assert!(Some(String::new())
            .filter(|p| !p.trim().is_empty())
            .is_none());
        assert!(Some("  ".to_string())
            .filter(|p| !p.trim().is_empty())
            .is_none());
        assert_eq!(
            Some("/usr/bin/obscura".to_string()).filter(|p| !p.trim().is_empty()),
            Some("/usr/bin/obscura".to_string())
        );
    }
}
