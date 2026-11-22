// Wiedergabe-Fortschritt pro Folge, im WebView-localStorage (persistiert im
// Tauri-Profil). Rein lokal, pro Gerät — fürs „Weiterschauen" und das Resümieren
// an der zuletzt gesehenen Stelle. Alle Zugriffe sind try/catch-gekapselt.

const MAP_KEY = "vindex:resume";
const RECENT_KEY = "vindex:recent";
const RECENT_MAX = 24;

export interface Resume {
  title: string;
  pos: number;
  dur: number;
  ts: number;
  season: number;
  episode: number;
}

function readMap(): Record<string, Resume> {
  try {
    return JSON.parse(localStorage.getItem(MAP_KEY) || "{}") as Record<string, Resume>;
  } catch {
    return {};
  }
}
function writeMap(m: Record<string, Resume>): void {
  try {
    localStorage.setItem(MAP_KEY, JSON.stringify(m));
  } catch {
    /* privater Modus / blockiert — egal, Fortschritt ist nur Komfort */
  }
}
function readRecent(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) || "[]") as string[];
  } catch {
    return [];
  }
}
function writeRecent(l: string[]): void {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(l.slice(0, RECENT_MAX)));
  } catch {
    /* s. o. */
  }
}

export function saveProgress(url: string, r: Resume): void {
  const m = readMap();
  m[url] = r;
  writeMap(m);
  const rec = readRecent().filter((u) => u !== url);
  rec.unshift(url);
  writeRecent(rec);
}

export function getProgress(url: string): Resume | null {
  return readMap()[url] ?? null;
}

export function clearProgress(url: string): void {
  const m = readMap();
  delete m[url];
  writeMap(m);
  writeRecent(readRecent().filter((u) => u !== url));
}

/** Zuletzt angesehene Folgen, neueste zuerst — fürs „Weiterschauen" auf Start. */
export function recentProgress(): { url: string; r: Resume }[] {
  const m = readMap();
  return readRecent()
    .map((url) => ({ url, r: m[url] }))
    .filter((x) => x.r);
}
