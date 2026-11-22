// Portalübergreifende Suche: lädt die drei Kataloge einmal und hält sie im Speicher.
// Rein Frontend — die Katalog-Daten kommen aus denselben Tauri-Commands wie die Tabs.
import { catalog } from "./api";
import type { Portal } from "./store";

export interface PaletteHit {
  portal: Portal;
  title: string;
  url: string;
}

const PORTALS: Portal[] = ["aniworld", "serienstream", "filmo"];
let cache: PaletteHit[] | null = null;
let inflight: Promise<PaletteHit[]> | null = null;

/** Lädt (einmalig) alle Kataloge und gibt die vereinte, flache Liste zurück. */
export function loadAll(): Promise<PaletteHit[]> {
  if (cache) return Promise.resolve(cache);
  if (inflight) return inflight;
  inflight = (async () => {
    const parts = await Promise.all(
      PORTALS.map(async (portal) => {
        const hits = await catalog(portal).catch(() => []);
        return hits.map((h) => ({ portal, title: h.title, url: h.url }));
      }),
    );
    cache = parts.flat();
    inflight = null;
    return cache;
  })();
  return inflight;
}

/** Fuzzy-arme Filterung: alle Suchwörter müssen im Titel vorkommen. Nach Position gewichtet. */
export function filterHits(all: PaletteHit[], query: string, limit = 40): PaletteHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const words = q.split(/\s+/);
  const scored: { h: PaletteHit; score: number }[] = [];
  for (const h of all) {
    const t = h.title.toLowerCase();
    if (!words.every((w) => t.includes(w))) continue;
    // Früher Treffer des ersten Wortes = besser; exakter Präfix am besten.
    const pos = t.indexOf(words[0]);
    const score = (t.startsWith(q) ? -1000 : 0) + pos;
    scored.push({ h, score });
    if (scored.length > 400) break; // genug Kandidaten zum Sortieren
  }
  scored.sort((a, b) => a.score - b.score || a.h.title.localeCompare(b.h.title));
  return scored.slice(0, limit).map((s) => s.h);
}
