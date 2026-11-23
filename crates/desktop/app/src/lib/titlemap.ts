// Merkt sich pro Titel (nach dem bereinigten Dateinamen-Schlüssel) die Portal-URL,
// damit die Bibliothek das Cover zeigen kann — der Dateiname allein kennt die URL nicht.
// Wird beim Öffnen eines Titels und beim Download gefüllt. Rein lokal.
const KEY = "vindex:titleurls";
const MAX = 2000;

function norm(key: string): string {
  return key.trim().toLowerCase();
}
function read(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "{}");
    return v && typeof v === "object" ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}

/** `key` ist der bereinigte Titel (wie im Dateinamen), `url` die Portal-Detail-URL. */
export function rememberTitleUrl(key: string, url: string): void {
  if (!key.trim() || !url) return;
  const m = read();
  m[norm(key)] = url;
  // Grob deckeln, damit die Map nicht unbegrenzt wächst.
  const keys = Object.keys(m);
  if (keys.length > MAX) delete m[keys[0]];
  try {
    localStorage.setItem(KEY, JSON.stringify(m));
  } catch {
    /* egal */
  }
}

export function getTitleUrl(key: string): string | null {
  return read()[norm(key)] ?? null;
}
