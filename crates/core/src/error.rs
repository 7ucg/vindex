//! Fehlertypen der Kern-Library.

use thiserror::Error;

/// Zentraler Fehlertyp.
#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP-Fehler: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Cloudflare-/Captcha-Challenge erkannt für {0}")]
    Challenge(String),

    #[error("Hoster-Extraktion fehlgeschlagen ({hoster}): {reason}")]
    Extract { hoster: String, reason: String },

    #[error("keine passende Sprache/Hoster/Qualität verfügbar")]
    NoStream,

    #[error("ffmpeg/ffprobe fehlgeschlagen: {0}")]
    Ffmpeg(String),

    #[error("Parsing-Fehler: {0}")]
    Parse(String),

    #[error("IO-Fehler: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Bequemer Result-Alias.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Baut einen Extraktionsfehler für *hoster* mit *reason*.
    pub fn extract(hoster: impl Into<String>, reason: impl Into<String>) -> Self {
        Error::Extract {
            hoster: hoster.into(),
            reason: reason.into(),
        }
    }
}
