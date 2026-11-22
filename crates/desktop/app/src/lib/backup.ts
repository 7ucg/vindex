// Export/Import des lokalen Zustands (Merkliste, Fortschritt, Erscheinungsbild).
// Rein localStorage — der Cover-Cache bleibt bewusst außen vor (groß, regenerierbar).
//
// Import ist streng validiert: eingelesene URLs müssen http/https auf einem der
// bekannten Portale sein (sonst flössen sie später als beliebige Fetch-Ziele in
// den Player → SSRF). Ungültige Einträge werden verworfen.

// JSON-wertige Schlüssel (Objekte/Arrays).
const JSON_KEYS = ["vindex:watchlist", "vindex:resume", "vindex:recent"] as const;
// Rohstring-Schlüssel (so liest theme.ts sie auch).
const RAW_KEYS = ["vindex:theme", "vindex:accent", "vindex:density"] as const;

const PORTAL_HOSTS = new Set([
  "aniworld.to",
  "www.aniworld.to",
  "serienstream.to",
  "www.serienstream.to",
  "s.to",
  "www.s.to",
  "filmo.to",
  "www.filmo.to",
]);
const THEME_OK = ["dark", "light", "amoled"];
const ACCENT_OK = ["violett", "cyan", "gruen", "pink", "bernstein"];
const DENSITY_OK = ["normal", "kompakt"];
const MAX_FILE = 2 * 1024 * 1024;
const MAX_ENTRIES = 5000;

function validUrl(u: unknown): u is string {
  if (typeof u !== "string" || u.length > 2048) return false;
  try {
    const x = new URL(u);
    return (x.protocol === "http:" || x.protocol === "https:") && PORTAL_HOSTS.has(x.hostname);
  } catch {
    return false;
  }
}
function str(v: unknown, max = 400): v is string {
  return typeof v === "string" && v.length <= max;
}
function num(v: unknown): v is number {
  return typeof v === "number" && isFinite(v);
}

export function exportState(): void {
  const data: Record<string, unknown> = {};
  for (const k of JSON_KEYS) {
    try {
      const v = localStorage.getItem(k);
      if (v !== null) data[k] = JSON.parse(v);
    } catch {
      /* ungültigen Eintrag überspringen */
    }
  }
  for (const k of RAW_KEYS) {
    const v = localStorage.getItem(k);
    if (v !== null) data[k] = v;
  }
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `vindex-backup-${new Date().toISOString().slice(0, 10)}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
}

// Validiert jeden Schlüssel und gibt den zu schreibenden Wert zurück (oder null).
function sanitize(key: string, value: unknown): string | null {
  if (key === "vindex:watchlist") {
    if (!Array.isArray(value)) return null;
    const items = value
      .filter((e): e is { url: string; title: string } => {
        const o = e as Record<string, unknown>;
        return !!o && validUrl(o.url) && str(o.title);
      })
      .slice(0, MAX_ENTRIES)
      .map((e) => ({ url: e.url, title: e.title }));
    return JSON.stringify(items);
  }
  if (key === "vindex:resume") {
    if (typeof value !== "object" || value === null) return null;
    const out: Record<string, unknown> = {};
    let n = 0;
    for (const [url, r] of Object.entries(value as Record<string, unknown>)) {
      if (n >= MAX_ENTRIES || !validUrl(url)) continue;
      const o = r as Record<string, unknown>;
      if (o && str(o.title) && num(o.pos) && num(o.dur) && num(o.ts) && num(o.season) && num(o.episode)) {
        out[url] = {
          title: o.title,
          pos: o.pos,
          dur: o.dur,
          ts: o.ts,
          season: o.season,
          episode: o.episode,
        };
        n++;
      }
    }
    return JSON.stringify(out);
  }
  if (key === "vindex:recent") {
    if (!Array.isArray(value)) return null;
    return JSON.stringify(value.filter((u) => validUrl(u)).slice(0, MAX_ENTRIES));
  }
  if (key === "vindex:theme") return THEME_OK.includes(value as string) ? (value as string) : null;
  if (key === "vindex:accent") return ACCENT_OK.includes(value as string) ? (value as string) : null;
  if (key === "vindex:density")
    return DENSITY_OK.includes(value as string) ? (value as string) : null;
  return null;
}

export async function importState(file: File): Promise<boolean> {
  if (file.size > MAX_FILE) return false;
  let data: Record<string, unknown>;
  try {
    data = JSON.parse(await file.text()) as Record<string, unknown>;
  } catch {
    return false;
  }
  if (typeof data !== "object" || data === null) return false;
  let wrote = false;
  for (const k of [...JSON_KEYS, ...RAW_KEYS]) {
    if (!(k in data)) continue;
    const safe = sanitize(k, data[k]);
    if (safe === null) continue;
    try {
      localStorage.setItem(k, safe);
      wrote = true;
    } catch {
      /* egal */
    }
  }
  return wrote;
}
