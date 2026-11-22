//! Rate-Limiting (pro Hoster) und Retry mit Backoff.
//!
//! `governor` (GCRA) als Client-Throttle pro Hoster-Domain, `backon` für
//! exponentiellen Backoff mit Jitter — siehe `ARCHITECTURE.md` §2.3/§5.

use std::num::NonZeroU32;

use backon::{ExponentialBuilder, Retryable};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};

use crate::model::Hoster;

/// Pro-Hoster-Throttle: begrenzt die Anfragen je Hoster-Domain.
pub struct HosterLimiter {
    inner: RateLimiter<Hoster, DefaultKeyedStateStore<Hoster>, DefaultClock>,
}

impl HosterLimiter {
    /// Baut einen Limiter mit *per_second* erlaubten Anfragen je Hoster.
    pub fn new(per_second: u32) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(per_second.max(1)).unwrap());
        Self {
            inner: RateLimiter::keyed(quota),
        }
    }

    /// Wartet, bis für *hoster* wieder ein Request erlaubt ist.
    pub async fn acquire(&self, hoster: Hoster) {
        self.inner.until_key_ready(&hoster).await;
    }
}

impl Default for HosterLimiter {
    fn default() -> Self {
        // Höflichkeits-Default: 2 Anfragen/Sekunde je Hoster.
        Self::new(2)
    }
}

/// Führt eine fehlbare async-Operation mit exponentiellem Backoff + Jitter aus.
pub async fn with_backoff<T, E, F, Fut>(op: F) -> std::result::Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, E>>,
{
    op.retry(ExponentialBuilder::default().with_jitter()).await
}
