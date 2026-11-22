//! `watch`-Unterbefehl: eine Episode direkt in mpv abspielen statt herunterladen.
//!
//! Löst denselben Sprach-/Hoster-Fallback wie `download` auf, übergibt den Stream
//! aber (mit Referer und optionalen AniSkip-Kapiteln) an mpv, statt zu muxen.

use clap::Parser;
use vindex_core::config::Settings;
use vindex_core::download::resolve_playable;
use vindex_core::http;
use vindex_core::mpv::{build_mpv_args, play, AniSkipTimes};

use crate::download::{file_name_from_url, resolve_prefs_from};

#[derive(Parser, Debug)]
pub struct WatchArgs {
    /// Episoden- oder Film-URL (aniworld.to / s.to / filmo.to).
    pub url: String,
    /// Sprache (German Dub | English Sub | German Sub | English Dub).
    #[arg(long)]
    pub language: Option<String>,
    /// Bevorzugter Hoster (VOE, Vidmoly, Vidoza, Doodstream, Filemoon, SpeedFiles).
    #[arg(long)]
    pub provider: Option<String>,
    /// Höchste Qualität (360p..4K). Bei HLS wählt mpv die Variante selbst.
    #[arg(long)]
    pub quality: Option<String>,
}

pub async fn run_watch(args: WatchArgs) -> anyhow::Result<()> {
    let settings = Settings::load();
    let client = http::browser_client()?;
    let resolved = resolve_prefs_from(
        args.language.as_deref(),
        args.provider.as_deref(),
        args.quality.as_deref(),
        None,
        &settings,
    )?;
    let prefs = resolved.for_url(&args.url);

    println!(
        "Löse Stream auf ({}, Hoster-Reihenfolge: {}) ...",
        resolved.language.as_str(),
        prefs
            .hosters
            .iter()
            .map(|h| h.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let stream = resolve_playable(&client, &args.url, &prefs)
        .await
        .map_err(|e| anyhow::anyhow!("Konnte keinen abspielbaren Stream finden: {e}"))?;

    // AniSkip-Kapitel: optional. Die eigentliche Zeiten-Abfrage braucht eine
    // MyAnimeList-ID (noch nicht aufgelöst, siehe TODO „AniSkip-Wiring"); sobald sie
    // vorliegt, liefert `aniskip::skip_times` → `AniSkipTimes::from_intervals` die
    // überspringbaren Kapitel, die `build_mpv_args` einhängt.
    let aniskip: Option<AniSkipTimes> = None;
    if settings.aniskip && aniskip.is_none() {
        eprintln!(
            "Hinweis: AniSkip aktiviert, aber die MAL-ID-Auflösung fehlt noch — spiele ohne Kapitel."
        );
    }

    let title = title_from_url(&args.url);
    let mpv_args = build_mpv_args(
        &stream.url,
        stream.referer.as_deref(),
        title.as_deref(),
        aniskip.as_ref(),
    );

    println!("Starte mpv ...");
    play(&mpv_args)
        .await
        .map_err(|e| anyhow::anyhow!("Wiedergabe fehlgeschlagen: {e}"))?;
    Ok(())
}

/// Leitet einen Medientitel aus der URL ab (wie der Dateiname, aber ohne `.mkv`).
fn title_from_url(url: &str) -> Option<String> {
    let name = file_name_from_url(url);
    let title = name.strip_suffix(".mkv").unwrap_or(&name).trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titel_aus_episoden_url() {
        assert_eq!(
            title_from_url("https://aniworld.to/anime/stream/dark/staffel-1/episode-3"),
            Some("dark S01E03".to_string())
        );
    }

    #[test]
    fn titel_aus_film_url() {
        assert_eq!(
            title_from_url("https://filmo.to/movies/the-matrix"),
            Some("the matrix".to_string())
        );
    }
}
