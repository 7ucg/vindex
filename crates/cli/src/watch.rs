//! `watch`-Unterbefehl: eine Episode direkt in mpv abspielen statt herunterladen.
//!
//! Löst denselben Sprach-/Hoster-Fallback wie `download` auf, übergibt den Stream
//! aber (mit Referer und optionalen AniSkip-Kapiteln) an mpv, statt zu muxen.

use clap::Parser;
use vindex_core::aniskip::{self, skip_times};
use vindex_core::config::Settings;
use vindex_core::download::resolve_playable;
use vindex_core::http;
use vindex_core::mpv::{
    anime4k_glsl_value, build_player_args, play_with, AniSkipTimes, Anime4kMode, Player,
};
use vindex_core::watchstate::episode_from_url;

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
    /// Player: `mpv` (Standard) oder `iina` (nur macOS).
    #[arg(long)]
    pub player: Option<String>,
    /// Anime4K-Upscaling-Modus (A, B, C, A+A, B+B, C+A). Braucht die Shader im
    /// Verzeichnis `--anime4k-dir` bzw. `VINDEX_ANIME4K_DIR`.
    #[arg(long)]
    pub anime4k: Option<String>,
    /// Verzeichnis mit den Anime4K-`.glsl`-Shadern (sonst `VINDEX_ANIME4K_DIR`).
    #[arg(long)]
    pub anime4k_dir: Option<String>,
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

    // AniSkip-Kapitel: optional. Slug/Staffel/Episode aus der URL ableiten, die
    // MAL-ID über Jikan auflösen und die Skip-Zeiten von AniSkip holen. Jeder
    // Fehlschlag (keine Episode, kein Treffer, API down) führt nur zu „ohne Kapitel",
    // nie zum Abbruch der Wiedergabe.
    let aniskip: Option<AniSkipTimes> = if settings.aniskip {
        match resolve_aniskip(&client, &args.url).await {
            Ok(Some(times)) => Some(times),
            Ok(None) => {
                eprintln!("Hinweis: Keine AniSkip-Zeiten gefunden — spiele ohne Kapitel.");
                None
            }
            Err(e) => {
                eprintln!("Hinweis: AniSkip-Auflösung fehlgeschlagen ({e}) — spiele ohne Kapitel.");
                None
            }
        }
    } else {
        None
    };

    let title = title_from_url(&args.url);

    let player = match args.player.as_deref() {
        None => Player::Mpv,
        Some(name) => Player::from_name(name)
            .ok_or_else(|| anyhow::anyhow!("Unbekannter Player: {name} (nutze mpv oder iina)"))?,
    };

    // Anime4K-Shader-Kette (optional): Modus + Shader-Verzeichnis auflösen.
    let anime4k = resolve_anime4k(&args)?;

    let player_args = build_player_args(
        player,
        &stream.url,
        stream.referer.as_deref(),
        title.as_deref(),
        aniskip.as_ref(),
        anime4k.as_deref(),
    );

    println!("Starte {} ...", player.binary());
    play_with(player, &player_args)
        .await
        .map_err(|e| anyhow::anyhow!("Wiedergabe fehlgeschlagen: {e}"))?;
    Ok(())
}

/// Löst den `--glsl-shaders`-Wert für Anime4K auf, falls `--anime4k` gesetzt ist.
/// Braucht ein Shader-Verzeichnis (`--anime4k-dir` oder `VINDEX_ANIME4K_DIR`).
fn resolve_anime4k(args: &WatchArgs) -> anyhow::Result<Option<String>> {
    let Some(mode_name) = args.anime4k.as_deref() else {
        return Ok(None);
    };
    let mode = Anime4kMode::from_name(mode_name).ok_or_else(|| {
        anyhow::anyhow!("Unbekannter Anime4K-Modus: {mode_name} (A, B, C, A+A, B+B, C+A)")
    })?;
    let dir = args
        .anime4k_dir
        .clone()
        .or_else(|| std::env::var("VINDEX_ANIME4K_DIR").ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Anime4K braucht ein Shader-Verzeichnis: --anime4k-dir <pfad> oder VINDEX_ANIME4K_DIR"
            )
        })?;
    Ok(Some(anime4k_glsl_value(mode, std::path::Path::new(&dir))))
}

/// Löst die AniSkip-Kapitel für eine Episoden-URL auf: Slug/Staffel/Episode aus
/// der URL, MAL-ID über Jikan, Skip-Zeiten über AniSkip. `Ok(None)`, wenn die URL
/// keine Episode ist, Jikan keinen Treffer hat oder AniSkip nichts liefert.
async fn resolve_aniskip(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<Option<AniSkipTimes>> {
    // Filme (ohne Staffel/Episode) haben keine AniSkip-Daten.
    let Some((slug, staffel, episode)) = episode_from_url(url) else {
        return Ok(None);
    };
    let Some(mal_id) = aniskip::resolve_mal_id(client, &slug, staffel).await? else {
        return Ok(None);
    };
    let intervals = skip_times(client, mal_id, episode).await?;
    let times = AniSkipTimes::from_intervals(&intervals);
    if times.is_empty() {
        return Ok(None);
    }
    Ok(Some(times))
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
