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
pub fn build_mpv_args(
    stream_url: &str,
    referer: Option<&str>,
    title: Option<&str>,
    aniskip: Option<&AniSkipTimes>,
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
    // Optionen-Ende: alles danach ist die (nicht als Option gedeutete) URL.
    args.push("--".to_string());
    args.push(stream_url.to_string());
    args
}

/// Startet mpv mit den gebauten Argumenten und wartet auf das Ende der Wiedergabe.
///
/// Läuft über `tokio::process::Command` mit getrennten Argumenten (keine Shell).
/// Fehlt mpv im PATH, kommt ein sauberer Fehler statt eines Panics; ein
/// Nicht-Null-Exit wird ebenfalls als Fehler gemeldet.
pub async fn play(args: &[String]) -> Result<()> {
    let mut cmd = Command::new("mpv");
    cmd.kill_on_drop(true);
    cmd.args(args);
    let status = match cmd.status().await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Mpv(
                "mpv nicht gefunden — bitte installieren und in den PATH legen".into(),
            ));
        }
        Err(e) => return Err(Error::Io(e)),
    };
    if status.success() {
        Ok(())
    } else {
        Err(Error::Mpv(format!("mpv endete mit Status {status}")))
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
}
