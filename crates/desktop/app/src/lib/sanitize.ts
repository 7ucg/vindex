// Datei-Namenskomponente aus einem (gescrapten) Titel. MUSS exakt der Rust-Seite
// entsprechen (sanitize_title_component in src-tauri/src/lib.rs), sonst weichen
// Download-Namen und der Abo-Abgleich voneinander ab (Endlos-Re-Download). Pure
// Funktion ohne DOM — hier zentral, damit Komponenten und Tests dieselbe Logik teilen.

// Windows-Gerätenamen, die als Dateiname verboten sind.
export const RESERVED_NAMES = new Set([
  "CON", "PRN", "AUX", "NUL",
  "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
  "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
]);

export function sanitizeFilename(s: string): string {
  const filtered = Array.from(s)
    .map((c) => (/\p{Cc}/u.test(c) ? " " : c))
    .filter((c) => !/[\\/:*?"<>|]/.test(c))
    .join("");
  // Erst auf 120 Codepoints kürzen, dann normalisieren (idempotent, FS-stabil).
  let out = Array.from(filtered).slice(0, 120).join("");
  out = out.trim().replace(/^\.+|\.+$/g, "").trim();
  if (RESERVED_NAMES.has(out.toUpperCase())) out += "_";
  if (out === "") out = "titel";
  return out;
}
