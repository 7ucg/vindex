//! `library`-Unterbefehl: die fertigen Downloads im Zielverzeichnis auflisten.

use clap::Parser;
use vindex_core::config::Settings;
use vindex_core::library;

#[derive(Parser, Debug)]
pub struct LibraryArgs {
    /// Zielverzeichnis (statt des konfigurierten `VINDEX_DOWNLOAD_PATH`).
    #[arg(long)]
    pub dir: Option<String>,
}

pub async fn run(args: LibraryArgs) -> anyhow::Result<()> {
    let settings = Settings::load();
    let dir = args
        .dir
        .map(std::path::PathBuf::from)
        .unwrap_or(settings.download_path);

    let entries = library::list(&dir);
    if entries.is_empty() {
        println!("Keine Downloads in {}", dir.display());
        return Ok(());
    }

    let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
    for e in &entries {
        println!("{:>10}  {}", human_size(e.size_bytes), e.name);
    }
    println!("{} Datei(en), {} gesamt", entries.len(), human_size(total));
    Ok(())
}

/// Formatiert eine Bytegröße kompakt (B, KiB, MiB, GiB).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_einheiten() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
