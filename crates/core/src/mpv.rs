//! mpv-Wiedergabe: Argument-Bau und Prozess-Start.
//!
//! Spielt einen aufgelösten Stream ([`crate::hoster::Resolved`]) direkt in mpv ab,
//! statt ihn herunterzuladen. Der Referer wird als HTTP-Header mitgegeben, AniSkip-
//! Zeiten werden als überspringbare Kapitel eingeblendet.
//!
//! Der Aufruf läuft über `tokio::process::Command` mit getrennten Argumenten (keine
//! Shell, kein `sh -c`) — die Stream-URL ist ein eigenes Argument und wird nie als
//! Shell-Text interpretiert.

use tokio::process::Command;

use crate::aniskip::{SkipInterval, SkipType};
use crate::error::{Error, Result};

/// Ein Skip-Intervall (Sekunden) für ein mpv-Kapitel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkipRange {
    pub start: f64,
    pub end: f64,
}

/// AniSkip-Zeiten einer Episode, aufbereitet für mpv-Kapitel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AniSkipTimes {
    /// Intro (Opening), falls vorhanden.
    pub intro: Option<SkipRange>,
    /// Outro (Ending), falls vorhanden.
    pub outro: Option<SkipRange>,
}

impl AniSkipTimes {
    /// Baut die Zeiten aus den AniSkip-Intervallen (OP → Intro, ED → Outro).
    /// Mehrfache Intervalle desselben Typs: das erste gewinnt.
    pub fn from_intervals(intervals: &[SkipInterval]) -> Self {
        let mut times = AniSkipTimes::default();
        for iv in intervals {
            let range = SkipRange {
                start: iv.start,
                end: iv.end,
            };
            match iv.skip_type {
                SkipType::Op if times.intro.is_none() => times.intro = Some(range),
                SkipType::Ed if times.outro.is_none() => times.outro = Some(range),
                _ => {}
            }
        }
        times
    }

    /// True, wenn weder Intro noch Outro gesetzt ist (keine Kapitel nötig).
    pub fn is_empty(&self) -> bool {
        self.intro.is_none() && self.outro.is_none()
    }

    /// Rendert die Zeiten als ffmetadata-Kapitelliste (mpv `--chapters-file`-Format).
    /// Zeitbasis 1/1000 (Millisekunden). Jedes `[CHAPTER]` setzt eine Kapitelgrenze,
    /// zwischen denen in mpv mit den Kapitel-Tasten gesprungen werden kann.
    pub fn to_ffmetadata(&self) -> String {
        let mut out = String::from(";FFMETADATA1\n");
        if let Some(r) = self.intro {
            out.push_str(&chapter_block(r.start, r.end, "Intro"));
        }
        if let Some(r) = self.outro {
            out.push_str(&chapter_block(r.start, r.end, "Outro"));
        }
        out
    }
}

/// Ein `[CHAPTER]`-Block in ffmetadata (Millisekunden, nicht-negativ).
fn chapter_block(start: f64, end: f64, title: &str) -> String {
    let start_ms = (start.max(0.0) * 1000.0).round() as u64;
    let end_ms = (end.max(0.0) * 1000.0).round() as u64;
    format!("[CHAPTER]\nTIMEBASE=1/1000\nSTART={start_ms}\nEND={end_ms}\ntitle={title}\n")
}

/// Entfernt CR/LF sowie das Komma (mpvs `--http-header-fields` trennt die Liste an
/// Kommata) und Anführungszeichen, damit ein Wert keine zusätzlichen Header
/// einschleusen kann.
fn sanitize(v: &str) -> String {
    v.chars()
        .filter(|c| !matches!(c, '\r' | '\n' | ',' | '"'))
        .collect()
}

/// Baut die mpv-Argumentliste. Reine Funktion ohne Seiteneffekt — testbar ohne
/// mpv/Netz.
///
/// Reihenfolge: optionaler Referer-Header, optionaler Medientitel, optionale
/// AniSkip-Kapitel, dann `--` und die Stream-URL als **letztes** Argument. Das `--`
/// trennt Optionen von der URL, damit eine mit `-` beginnende URL nicht als Option
/// gedeutet wird. Die URL kommt als eigenes Argument — ohne Shell interpretiert
/// mpv keine Metazeichen.
///
/// AniSkip-Kapitel werden über mpvs `memory://`-Protokoll inline an
/// `--chapters-file` gehängt (kein Temp-File nötig): die ffmetadata-Kapitelliste
/// steht direkt im Argument.
/// Externer Player für den Watch-Modus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Player {
    /// mpv (Standard, alle Plattformen).
    #[default]
    Mpv,
    /// IINA (nur macOS); über `iina-cli`, mpv-Optionen als `--mpv-…`.
    Iina,
}

impl Player {
    /// Das zu startende Binary.
    pub fn binary(&self) -> &'static str {
        match self {
            Player::Mpv => "mpv",
            Player::Iina => "iina-cli",
        }
    }

    /// Parst einen Player-Namen (`mpv`, `iina`/`iina-cli`), case-insensitiv.
    pub fn from_name(name: &str) -> Option<Player> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mpv" => Some(Player::Mpv),
            "iina" | "iina-cli" => Some(Player::Iina),
            _ => None,
        }
    }
}

/// Anime4K-Upscaling-Modus (offizielle GLSL-Shader-Ketten, Anime4K v4.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anime4kMode {
    /// A — Restore + Upscale (für sehr degradiertes Material).
    A,
    /// B — weicheres Restore.
    B,
    /// C — Denoise + Upscale.
    C,
    /// A+A — stärkeres Restore.
    Aa,
    /// B+B — weiches Restore, zweistufig.
    Bb,
    /// C+A — Denoise plus Restore.
    Ca,
}

impl Anime4kMode {
    /// Parst einen Modusnamen (`A`, `B`, `C`, `A+A`/`AA`, `B+B`/`BB`, `C+A`/`CA`).
    pub fn from_name(name: &str) -> Option<Anime4kMode> {
        match name.trim().to_ascii_uppercase().replace('+', "").as_str() {
            "A" => Some(Anime4kMode::A),
            "B" => Some(Anime4kMode::B),
            "C" => Some(Anime4kMode::C),
            "AA" => Some(Anime4kMode::Aa),
            "BB" => Some(Anime4kMode::Bb),
            "CA" => Some(Anime4kMode::Ca),
            _ => None,
        }
    }

    /// Die Shader-Dateinamen der Kette (in Anwendungsreihenfolge).
    pub fn shaders(&self) -> &'static [&'static str] {
        match self {
            Anime4kMode::A => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_VL.glsl",
                "Anime4K_Upscale_CNN_x2_VL.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
            Anime4kMode::B => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_Soft_VL.glsl",
                "Anime4K_Upscale_CNN_x2_VL.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
            Anime4kMode::C => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Upscale_Denoise_CNN_x2_VL.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
            Anime4kMode::Aa => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_VL.glsl",
                "Anime4K_Upscale_CNN_x2_VL.glsl",
                "Anime4K_Restore_CNN_M.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
            Anime4kMode::Bb => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Restore_CNN_Soft_VL.glsl",
                "Anime4K_Upscale_CNN_x2_VL.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Restore_CNN_Soft_M.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
            Anime4kMode::Ca => &[
                "Anime4K_Clamp_Highlights.glsl",
                "Anime4K_Upscale_Denoise_CNN_x2_VL.glsl",
                "Anime4K_AutoDownscalePre_x2.glsl",
                "Anime4K_AutoDownscalePre_x4.glsl",
                "Anime4K_Restore_CNN_M.glsl",
                "Anime4K_Upscale_CNN_x2_M.glsl",
            ],
        }
    }
}

/// Baut den Wert für mpvs `--glsl-shaders` aus Modus + Shader-Verzeichnis: die
/// Shader-Pfade, getrennt durch `:` (mpv-Listensyntax). Ohne Flag-Präfix.
pub fn anime4k_glsl_value(mode: Anime4kMode, shader_dir: &std::path::Path) -> String {
    mode.shaders()
        .iter()
        .map(|s| shader_dir.join(s).to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

/// Baut die mpv-Options-Tokens (Härtung, Referer, Titel, AniSkip-Kapitel, optional
/// Anime4K-Shader) — ohne `--`-Trenner und ohne URL. Gemeinsame Basis für mpv/IINA.
/// `anime4k` ist der fertige `--glsl-shaders`-Wert (siehe [`anime4k_glsl_value`]).
fn mpv_option_tokens(
    referer: Option<&str>,
    title: Option<&str>,
    aniskip: Option<&AniSkipTimes>,
    anime4k: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Härtung: keine Nutzer-Config, kein ytdl-Hook, keine Lua-Scripts — so kann eine
    // (aus gescraptem HTML stammende) URL weder Scripts noch den ytdl-Pfad erreichen.
    args.push("--no-config".to_string());
    args.push("--ytdl=no".to_string());
    args.push("--load-scripts=no".to_string());

    if let Some(r) = referer {
        // `-append`, damit kein Komma die Header-Liste aufsplittet.
        args.push(format!(
            "--http-header-fields-append=Referer: {}",
            sanitize(r)
        ));
    }
    if let Some(t) = title {
        args.push(format!("--force-media-title={}", sanitize(t)));
    }
    if let Some(a) = aniskip {
        if !a.is_empty() {
            // ffmetadata inline über memory:// — mpv liest die Kapitel direkt aus dem
            // Argument, ohne dass eine Datei geschrieben werden muss.
            args.push(format!("--chapters-file=memory://{}", a.to_ffmetadata()));
        }
    }
    if let Some(shaders) = anime4k {
        args.push(format!("--glsl-shaders={}", sanitize(shaders)));
    }
    args
}

pub fn build_mpv_args(
    stream_url: &str,
    referer: Option<&str>,
    title: Option<&str>,
    aniskip: Option<&AniSkipTimes>,
) -> Vec<String> {
    build_mpv_args_with(stream_url, referer, title, aniskip, None)
}

/// Wie [`build_mpv_args`], aber mit optionaler Anime4K-Shader-Kette (`--glsl-shaders`).
pub fn build_mpv_args_with(
    stream_url: &str,
    referer: Option<&str>,
    title: Option<&str>,
    aniskip: Option<&AniSkipTimes>,
    anime4k: Option<&str>,
) -> Vec<String> {
    let mut args = mpv_option_tokens(referer, title, aniskip, anime4k);
    // Optionen-Ende: alles danach ist die (nicht als Option gedeutete) URL.
    args.push("--".to_string());
    args.push(stream_url.to_string());
    args
}

/// Baut die Argumente für den gewählten Player. Für mpv wie [`build_mpv_args`];
/// für IINA werden dieselben Optionen als `--mpv-<opt>` an `iina-cli` übergeben und
/// die URL als letztes Argument angehängt (kein `--`-Trenner, iina-cli kennt ihn nicht).
pub fn build_player_args(
    player: Player,
    stream_url: &str,
    referer: Option<&str>,
    title: Option<&str>,
    aniskip: Option<&AniSkipTimes>,
    anime4k: Option<&str>,
) -> Vec<String> {
    match player {
        Player::Mpv => build_mpv_args_with(stream_url, referer, title, aniskip, anime4k),
        Player::Iina => {
            let mut args: Vec<String> = mpv_option_tokens(referer, title, aniskip, anime4k)
                .into_iter()
                .map(|opt| format!("--mpv-{}", opt.trim_start_matches("--")))
                .collect();
            args.push(stream_url.to_string());
            args
        }
    }
}

/// Startet mpv mit den gebauten Argumenten und wartet auf das Ende der Wiedergabe.
///
/// Läuft über `tokio::process::Command` mit getrennten Argumenten (keine Shell).
/// Fehlt mpv im PATH, kommt ein sauberer Fehler statt eines Panics; ein
/// Nicht-Null-Exit wird ebenfalls als Fehler gemeldet.
pub async fn play(args: &[String]) -> Result<()> {
    play_with(Player::Mpv, args).await
}

/// Wie [`play`], aber für einen frei gewählten [`Player`] (mpv oder IINA).
pub async fn play_with(player: Player, args: &[String]) -> Result<()> {
    run_player(player.binary(), args).await
}

/// Startet ein Player-Binary mit getrennten Argumenten (keine Shell) und wartet.
async fn run_player(bin: &str, args: &[String]) -> Result<()> {
    let mut cmd = Command::new(bin);
    cmd.kill_on_drop(true);
    cmd.args(args);
    let status = match cmd.status().await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Mpv(format!(
                "{bin} nicht gefunden — bitte installieren und in den PATH legen"
            )));
        }
        Err(e) => return Err(Error::Io(e)),
    };
    if status.success() {
        Ok(())
    } else {
        Err(Error::Mpv(format!("{bin} endete mit Status {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baut_args_mit_referer_titel_und_url() {
        let args = build_mpv_args(
            "https://cdn.example/stream.m3u8",
            Some("https://hoster.example/embed"),
            Some("Dark S01E03"),
            None,
        );
        // Referer-Header exakt wie erwartet.
        assert!(args.contains(
            &"--http-header-fields-append=Referer: https://hoster.example/embed".to_string()
        ));
        // Medientitel gesetzt.
        assert!(args.contains(&"--force-media-title=Dark S01E03".to_string()));
        // URL ist das letzte Argument und ein eigenes Argument.
        assert_eq!(args.last().unwrap(), "https://cdn.example/stream.m3u8");
        // Optionen-Ende steht direkt vor der URL.
        assert_eq!(args[args.len() - 2], "--");
    }

    #[test]
    fn ohne_referer_kein_header_arg() {
        let args = build_mpv_args("https://cdn.example/v.mp4", None, Some("Titel"), None);
        assert!(!args
            .iter()
            .any(|a| a.starts_with("--http-header-fields-append=")));
        assert!(args.contains(&"--force-media-title=Titel".to_string()));
        assert_eq!(args.last().unwrap(), "https://cdn.example/v.mp4");
    }

    #[test]
    fn ohne_titel_kein_titel_arg() {
        let args = build_mpv_args("https://cdn.example/v.mp4", Some("https://ref"), None, None);
        assert!(!args.iter().any(|a| a.starts_with("--force-media-title=")));
        assert!(args.contains(&"--http-header-fields-append=Referer: https://ref".to_string()));
    }

    #[test]
    fn ohne_referer_und_titel_nur_trenner_und_url() {
        let args = build_mpv_args("https://cdn.example/v.mp4", None, None, None);
        // Härtungsflags, dann Trenner und URL als letztes Argument.
        assert_eq!(
            args,
            vec![
                "--no-config".to_string(),
                "--ytdl=no".to_string(),
                "--load-scripts=no".to_string(),
                "--".to_string(),
                "https://cdn.example/v.mp4".to_string()
            ]
        );
    }

    #[test]
    fn referer_mit_komma_wird_entschaerft() {
        // Ein Komma im Referer darf die Header-Liste nicht aufsplitten.
        let args = build_mpv_args("https://x/v", Some("https://r/a,b"), None, None);
        let header = args
            .iter()
            .find(|a| a.starts_with("--http-header-fields-append="))
            .unwrap();
        assert!(!header.contains(','));
        assert_eq!(header, "--http-header-fields-append=Referer: https://r/ab");
    }

    #[test]
    fn mit_aniskip_landet_als_chapters_file() {
        let times = AniSkipTimes {
            intro: Some(SkipRange {
                start: 0.0,
                end: 85.0,
            }),
            outro: Some(SkipRange {
                start: 1320.0,
                end: 1400.0,
            }),
        };
        let args = build_mpv_args("https://cdn.example/s.m3u8", None, None, Some(&times));
        let chap = args
            .iter()
            .find(|a| a.starts_with("--chapters-file=memory://"))
            .expect("chapters-file-Argument fehlt");
        // Zeiten als Millisekunden im ffmetadata-Block.
        assert!(chap.contains("START=0"));
        assert!(chap.contains("END=85000"));
        assert!(chap.contains("START=1320000"));
        assert!(chap.contains("END=1400000"));
        assert!(chap.contains("title=Intro"));
        assert!(chap.contains("title=Outro"));
        // URL bleibt letztes, eigenständiges Argument.
        assert_eq!(args.last().unwrap(), "https://cdn.example/s.m3u8");
    }

    #[test]
    fn leere_aniskip_erzeugt_kein_chapters_arg() {
        let times = AniSkipTimes::default();
        assert!(times.is_empty());
        let args = build_mpv_args("https://cdn.example/s.m3u8", None, None, Some(&times));
        assert!(!args.iter().any(|a| a.starts_with("--chapters-file=")));
    }

    #[test]
    fn aniskip_aus_intervallen() {
        let intervals = vec![
            SkipInterval {
                start: 10.0,
                end: 100.0,
                skip_type: SkipType::Op,
            },
            SkipInterval {
                start: 1300.0,
                end: 1380.0,
                skip_type: SkipType::Ed,
            },
        ];
        let times = AniSkipTimes::from_intervals(&intervals);
        assert_eq!(
            times.intro,
            Some(SkipRange {
                start: 10.0,
                end: 100.0
            })
        );
        assert_eq!(
            times.outro,
            Some(SkipRange {
                start: 1300.0,
                end: 1380.0
            })
        );
    }

    #[test]
    fn referer_mit_crlf_wird_entschaerft() {
        // Ein eingeschleustes CR/LF darf keine zweite Header-Zeile ergeben.
        let args = build_mpv_args(
            "https://cdn.example/s.m3u8",
            Some("https://ref\r\nX-Evil: 1"),
            None,
            None,
        );
        let header = args
            .iter()
            .find(|a| a.starts_with("--http-header-fields-append="))
            .unwrap();
        assert!(!header.contains('\r'));
        assert!(!header.contains('\n'));
        assert_eq!(
            header,
            "--http-header-fields-append=Referer: https://refX-Evil: 1"
        );
    }

    #[test]
    fn url_mit_sonderzeichen_bleibt_ein_argument() {
        // Shell-Metazeichen landen unverändert in genau einem Argument.
        let url = "https://cdn.example/s.m3u8?a=1&b=2;rm -rf /|$(x)";
        let args = build_mpv_args(url, None, None, None);
        assert_eq!(args.last().unwrap(), url);
        // Es gibt genau ein URL-Argument (keine Aufspaltung).
        assert_eq!(args.iter().filter(|a| a.as_str() == url).count(), 1);
    }

    #[test]
    fn player_from_name_parst_varianten() {
        assert_eq!(Player::from_name("mpv"), Some(Player::Mpv));
        assert_eq!(Player::from_name("IINA"), Some(Player::Iina));
        assert_eq!(Player::from_name("iina-cli"), Some(Player::Iina));
        assert_eq!(Player::from_name(" MPV "), Some(Player::Mpv));
        assert_eq!(Player::from_name("vlc"), None);
        assert_eq!(Player::Mpv.binary(), "mpv");
        assert_eq!(Player::Iina.binary(), "iina-cli");
    }

    #[test]
    fn iina_args_praefixen_mpv_optionen() {
        let url = "https://cdn.example/s.m3u8";
        let args = build_player_args(
            Player::Iina,
            url,
            Some("https://ref"),
            Some("Titel"),
            None,
            None,
        );
        // Jede mpv-Option trägt das --mpv-Präfix …
        assert!(args.contains(&"--mpv-no-config".to_string()));
        assert!(args.contains(&"--mpv-ytdl=no".to_string()));
        assert!(args.contains(&"--mpv-load-scripts=no".to_string()));
        assert!(args.contains(&"--mpv-force-media-title=Titel".to_string()));
        assert!(args
            .iter()
            .any(|a| a == "--mpv-http-header-fields-append=Referer: https://ref"));
        // … kein nacktes --option bleibt übrig, und die URL ist das letzte Argument.
        assert!(!args.iter().any(|a| a.starts_with("--no-")));
        assert_eq!(args.last().unwrap(), url);
        // iina-cli kennt keinen `--`-Trenner.
        assert!(!args.iter().any(|a| a == "--"));
    }

    #[test]
    fn anime4k_modus_und_glsl_wert() {
        use std::path::Path;
        assert_eq!(Anime4kMode::from_name("a"), Some(Anime4kMode::A));
        assert_eq!(Anime4kMode::from_name("C+A"), Some(Anime4kMode::Ca));
        assert_eq!(Anime4kMode::from_name("bb"), Some(Anime4kMode::Bb));
        assert_eq!(Anime4kMode::from_name("x"), None);

        let value = anime4k_glsl_value(Anime4kMode::A, Path::new("/shaders"));
        // Pfade mit `:` getrennt, jeder Shader unter dem Verzeichnis.
        assert!(value.starts_with("/shaders/Anime4K_Clamp_Highlights.glsl:"));
        assert!(value.contains("/shaders/Anime4K_Upscale_CNN_x2_M.glsl"));
        assert_eq!(value.split(':').count(), Anime4kMode::A.shaders().len());
    }

    #[test]
    fn anime4k_landet_als_glsl_shaders_option() {
        use std::path::Path;
        let value = anime4k_glsl_value(Anime4kMode::C, Path::new("/s"));
        let args = build_player_args(
            Player::Mpv,
            "https://x/v.m3u8",
            None,
            None,
            None,
            Some(&value),
        );
        assert!(args.iter().any(|a| a == &format!("--glsl-shaders={value}")));
        // IINA bekommt es mit --mpv-Präfix.
        let iina = build_player_args(
            Player::Iina,
            "https://x/v.m3u8",
            None,
            None,
            None,
            Some(&value),
        );
        assert!(iina
            .iter()
            .any(|a| a == &format!("--mpv-glsl-shaders={value}")));
    }

    #[test]
    fn mpv_und_iina_tragen_dieselben_optionen() {
        let url = "https://cdn.example/s.m3u8";
        let mpv = build_player_args(Player::Mpv, url, Some("https://r"), Some("T"), None, None);
        let iina = build_player_args(Player::Iina, url, Some("https://r"), Some("T"), None, None);
        // Gleiche Zahl Optionen (mpv hat zusätzlich den `--`-Trenner).
        assert_eq!(mpv.len(), iina.len() + 1);
        assert_eq!(mpv.last().unwrap(), url);
        assert_eq!(iina.last().unwrap(), url);
    }
}
