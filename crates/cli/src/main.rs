//! CLI-Einstiegspunkt für `vindex`.
//!
//! Befehle: `census` (Katalog zählen), `search` (Titel suchen),
//! `download` (Episode laden), `menu` (interaktiv), `serve` (API-Daemon).

mod census;
mod download;
mod interactive;
mod library;
mod watch;

use clap::{Parser, Subcommand};

/// Modularer AniWorld-Downloader (WIP).
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Untersucht aniworld.to und zählt Serien, Filme, Staffeln und Folgen.
    Census(census::CensusArgs),
    /// Sucht Titel im aniworld.to-Katalog (Teilstring, case-insensitiv).
    Search(download::SearchArgs),
    /// Lädt eine Episode/einen Film oder per `--batch <datei>` eine ganze Episodenliste
    /// (Sprache/Hoster/Qualität mit Fallback).
    Download(download::DownloadArgs),
    /// Spielt eine Episode/einen Film direkt in mpv ab (Referer + AniSkip-Kapitel),
    /// statt sie herunterzuladen (Sprache/Hoster/Qualität wie `download`).
    Watch(watch::WatchArgs),
    /// Interaktives Menü: suchen → Staffel/Episode/Sprache wählen → laden.
    Menu,
    /// Listet die fertigen Downloads im Zielverzeichnis (Name + Größe).
    Library(library::LibraryArgs),
    /// Startet den HTTP-API-Daemon.
    Serve(ServeArgs),
}

#[derive(Parser, Debug)]
struct ServeArgs {
    /// Adresse zum Lauschen.
    #[arg(long, default_value = "127.0.0.1:8080")]
    addr: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // SIGPIPE nur für die ausgebenden Befehle zurücksetzen. Der Daemon (`serve`)
    // soll SIGPIPE weiter ignorieren, damit ein kaputter Client-Socket ihn nicht
    // beendet.
    if !matches!(cli.command, Command::Serve(_)) {
        reset_sigpipe();
    }
    match cli.command {
        Command::Census(args) => census::run(args).await,
        Command::Search(args) => download::run_search(args).await,
        Command::Download(args) => download::run_download(args).await,
        Command::Watch(args) => watch::run_watch(args).await,
        Command::Menu => interactive::run().await,
        Command::Library(args) => library::run(args).await,
        Command::Serve(args) => vindex_api::serve(&args.addr).await,
    }
}

/// Setzt SIGPIPE auf das Standardverhalten zurück. Rust ignoriert SIGPIPE sonst,
/// wodurch `println!` bei vorzeitig geschlossener Ausgabe (etwa `vindex search x
/// | head`) mit „Broken pipe" panict, statt still zu beenden.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: einmaliger Aufruf vor jeder Ausgabe; setzt nur den Signal-Handler.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}
