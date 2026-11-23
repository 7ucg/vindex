// Persistenter Download-Verlauf (localStorage). Reines Log: was wann geladen wurde —
// unabhängig davon, ob die Datei später noch in der Bibliothek liegt.
import { isIncognito } from "./private";

export interface HistItem {
  title: string;
  bytes: number;
  ts: number;
  ok: boolean;
}

const KEY = "vindex:history";
const MAX = 200;

function readRaw(): HistItem[] {
  try {
    const v = localStorage.getItem(KEY);
    if (!v) return [];
    const arr = JSON.parse(v);
    return Array.isArray(arr) ? (arr as HistItem[]) : [];
  } catch {
    return [];
  }
}

export function getHistory(): HistItem[] {
  return readRaw();
}

export function recordHistory(item: HistItem): HistItem[] {
  if (isIncognito()) return readRaw(); // Inkognito: nicht festhalten
  const list = [item, ...readRaw()].slice(0, MAX);
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* egal */
  }
  return list;
}

export function clearHistory(): void {
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* egal */
  }
}
