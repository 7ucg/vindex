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
        .build()
}

/// Baut einen Client mit rotierendem (browser-ähnlichem) User-Agent — für die
/// Hoster-/Episodenseiten, die auf plausible Clients achten.
pub fn browser_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(next_user_agent())
        .gzip(true)
        .timeout(Duration::from_secs(30))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ua_rotation_cycles() {
        let first = next_user_agent();
        // Nach genau einem vollen Umlauf (len Aufrufe) wieder derselbe Eintrag.
        for _ in 0..USER_AGENTS.len() - 1 {
            next_user_agent();
        }
        assert_eq!(next_user_agent(), first);
    }
}
