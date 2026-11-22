//! # vindex-core
//!
//! Kern-Library für `vindex`. Stellt die wiederverwendbare Logik bereit,
//! die CLI, API/Daemon und Desktop-App gemeinsam nutzen.
//!
//! ## Module
//! - [`model`]      — Datentypen (Sprache, Qualität, Hoster, Episode, Links).
//! - [`error`]      — Fehlertyp [`error::Error`] + [`error::Result`].
//! - [`http`]       — Client, UA-Rotation, Cloudflare-Erkennung.
//! - [`site`]       — Seiten-Scraper (aniworld.to, s.to) + Census.
//! - [`hoster`]     — Stream-Resolver je Hoster (VOE, SpeedFiles, …).
//! - [`extract`]    — HLS-Master-Parsing, Qualitäts-Tiers.
//! - [`download`]   — ffmpeg-Mux, Qualitäts-Sweep, Fallback-Kette.
//! - [`ratelimit`]  — pro-Hoster-Throttle (governor) + Backoff (backon).
//! - [`naming`]     — Datei-Templates, Episoden-Nummerierung.
//! - [`config`]     — `VINDEX_*`-Einstellungen.
//! - [`aniskip`]    — Intro/Outro-Zeiten.
//! - [`mpv`]        — mpv-Argumentbau und Wiedergabe-Start (Watch-Modus).
//! - [`watchstate`] — „keep watching": Fortschritt pro Serie merken.

pub mod aniskip;
pub mod config;
pub mod download;
pub mod error;
pub mod extract;
pub mod hoster;
pub mod http;
pub mod library;
pub mod model;
pub mod mpv;
pub mod naming;
pub mod ratelimit;
pub mod site;
pub mod watchstate;

pub use error::{Error, Result};
pub use model::{
    CatalogEntry, Classified, Episode, Hoster, Kind, Language, ProviderLinks, Quality,
};
