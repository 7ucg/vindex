//! Bibliothek: die fertigen Downloads im Zielverzeichnis auflisten.
//!
//! Gemeinsame, netzfreie Scan-Logik für CLI (`vindex library`) und API
//! (`GET /api/library`). Gibt nur Dateinamen und Größen zurück (keine Pfade),
//! scannt nicht rekursiv und zählt nur Medien-Endungen.

use std::path::Path;

/// Ein Bibliothekseintrag (eine Datei im Zielverzeichnis).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Dateiname (ohne Pfad).
    pub name: String,
    /// Dateigröße in Bytes.
    pub size_bytes: u64,
}

/// Höchstzahl Einträge, die ein Scan zurückgibt.
pub const MAX_ENTRIES: usize = 10_000;

/// Höchstzahl Verzeichniseinträge, die überhaupt betrachtet werden (unabhängig
/// davon, wie viele davon Medien sind) — begrenzt die Arbeit bei riesigen Ordnern.
const MAX_SCANNED: usize = 200_000;

/// Medien-Dateiendungen, die als Bibliothekseintrag zählen.
const EXTENSIONS: &[&str] = &["mkv", "mp4", "ts", "webm", "mov"];

/// Scannt *dir* (nicht rekursiv) nach Mediendateien und gibt Name + Größe zurück,
/// alphabetisch sortiert. Gibt nur Dateinamen zurück (keine Pfade), listet nur
/// echte Dateien direkt im Verzeichnis — **Symlinks werden nicht gefolgt** und
/// übersprungen, ebenso Unterverzeichnisse und Nicht-Medien. Ein nicht lesbares
/// Verzeichnis ergibt eine leere Liste. Diese Funktion blockiert (Datei-I/O); in
/// einem async-Kontext über `spawn_blocking` aufrufen.
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut scanned = 0usize;
    for entry in read.flatten() {
        scanned += 1;
        if scanned >= MAX_SCANNED || out.len() >= MAX_ENTRIES {
            break;
        }
        // file_type() aus dem Verzeichniseintrag folgt dem Symlink NICHT: nur echte
        // reguläre Dateien zählen, kein Symlink kann aus dem Verzeichnis herauszeigen.
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let is_media = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .is_some_and(|e| EXTENSIONS.contains(&e.as_str()));
        if !is_media {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(Entry { name, size_bytes });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listet_nur_medien_sortiert() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("vindex-corelib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.mkv"), b"xx").unwrap();
        std::fs::write(dir.join("a.mp4"), b"yyyy").unwrap();
        std::fs::write(dir.join("notiz.txt"), b"kein medium").unwrap();
        std::fs::create_dir_all(dir.join("unter")).unwrap();

        let items = list(&dir);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "a.mp4");
        assert_eq!(items[0].size_bytes, 4);
        assert_eq!(items[1].name, "b.mkv");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fehlendes_verzeichnis_ist_leer() {
        assert!(list(Path::new("/nicht/vorhanden/xyz")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_werden_nicht_gefolgt() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("vindex-corelib-sym-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Echte Datei bleibt, ein Symlink mit Medien-Endung wird übersprungen.
        std::fs::write(dir.join("echt.mkv"), b"zz").unwrap();
        std::os::unix::fs::symlink("/etc/hostname", dir.join("link.mkv")).ok();

        let items = list(&dir);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "echt.mkv");

        std::fs::remove_dir_all(&dir).ok();
    }
}
